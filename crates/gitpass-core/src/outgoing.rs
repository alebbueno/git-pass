//! Commits a bound repository would send with `git push`, the checks run before sending (author, large
//! files, possible secrets, branch behind GitHub, …) and the operations around it: push, pull with rebase
//! and rewriting the author of unpushed commits.
//!
//! Network operations take the account token, which reaches `git` only through the environment
//! ([`crate::git_auth`]). Everything here works with temporary repos and a `file://` remote.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::Serialize;

use crate::git_auth::{apply_git_env, for_each_line, git_program, parse_progress_line, redact, spawn_error};

/// Most commits listed in [`OutgoingState::commits`].
pub const MAX_COMMITS: usize = 200;
const MIB: u64 = 1024 * 1024;
const SECRET_SCAN_MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_LARGE_FILE_CHECKS: usize = 20;
const MAX_SECRET_CHECKS: usize = 50;
const MAX_COMMIT_FILES: usize = 5000;
const NOREPLY_DOMAIN: &str = "users.noreply.github.com";
const SUMMARY_MAX_LINES: usize = 40;

// --- JSON types (gitpass-desktop/src/lib/api-repos.ts) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckCode {
    AuthorMismatch,
    CommitterMismatch,
    LargeFile,
    FileTooLarge,
    SecretSuspected,
    BehindRemote,
    NoPushPermission,
    AccountDisconnected,
    DetachedHead,
    NoRemote,
    DirtyWorktree,
    MergeCommit,
    FetchFailed,
    NothingToPush,
    MergeConflict,
    TargetDirty,
    TargetBehindRemote,
    AlreadyInTarget,
    SourcePushed,
}

impl CheckCode {
    pub const ALL: [CheckCode; 19] = [
        CheckCode::AuthorMismatch,
        CheckCode::CommitterMismatch,
        CheckCode::LargeFile,
        CheckCode::FileTooLarge,
        CheckCode::SecretSuspected,
        CheckCode::BehindRemote,
        CheckCode::NoPushPermission,
        CheckCode::AccountDisconnected,
        CheckCode::DetachedHead,
        CheckCode::NoRemote,
        CheckCode::DirtyWorktree,
        CheckCode::MergeCommit,
        CheckCode::FetchFailed,
        CheckCode::NothingToPush,
        CheckCode::MergeConflict,
        CheckCode::TargetDirty,
        CheckCode::TargetBehindRemote,
        CheckCode::AlreadyInTarget,
        CheckCode::SourcePushed,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushCheck {
    pub code: CheckCode,
    pub severity: CheckSeverity,
    pub message: String,
    pub sha: Option<String>,
    pub path: Option<String>,
}

impl PushCheck {
    pub(crate) fn new(code: CheckCode, severity: CheckSeverity, message: impl Into<String>) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            sha: None,
            path: None,
        }
    }

    pub(crate) fn sha(mut self, sha: Option<&str>) -> Self {
        self.sha = sha.map(String::from);
        self
    }

    pub(crate) fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }
}

/// `RepoRow.sync`: local only (no network), as of the last fetch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SyncSummary {
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
    pub dirty: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DirtyCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

impl DirtyCounts {
    pub fn any(&self) -> bool {
        self.staged + self.unstaged + self.untracked > 0
    }

    /// No staged or unstaged changes (untracked files do not get in the way of a rebase).
    pub fn tracked_clean(&self) -> bool {
        self.staged == 0 && self.unstaged == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutgoingCommit {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub body: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub committer_name: String,
    pub committer_email: String,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub is_merge: bool,
    pub checks: Vec<PushCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutgoingState {
    pub binding_id: String,
    pub branch: Option<String>,
    pub remote_name: Option<String>,
    pub upstream: Option<String>,
    pub remote_branch_url: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub fetched: bool,
    pub dirty: DirtyCounts,
    pub commits: Vec<OutgoingCommit>,
    pub truncated: bool,
    pub checks: Vec<PushCheck>,
    pub head_sha: Option<String>,
    pub can_push: bool,
    pub has_warnings: bool,
    pub can_fix_authors: bool,
    pub can_pull: bool,
    /// Conta conectada cujos emails batem com o autor dos commits (quando a conta vinculada não bate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_push_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_push_account_login: Option<String>,
    /// Conta recomendada para enviar neste remoto (dona / push confirmado).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_push_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_push_account_login: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub insertions: Option<usize>,
    pub deletions: Option<usize>,
    pub binary: bool,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushResult {
    pub pushed: usize,
    pub branch: String,
    pub created_upstream: bool,
    pub remote_branch_url: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushPhase {
    Validating,
    Connecting,
    Counting,
    Compressing,
    Writing,
    Resolving,
    Done,
}

impl PushPhase {
    pub const ALL: [PushPhase; 7] = [
        PushPhase::Validating,
        PushPhase::Connecting,
        PushPhase::Counting,
        PushPhase::Compressing,
        PushPhase::Writing,
        PushPhase::Resolving,
        PushPhase::Done,
    ];

    fn from_git(phase: &str) -> Option<Self> {
        match phase {
            "counting" => Some(Self::Counting),
            "compressing" => Some(Self::Compressing),
            "writing" => Some(Self::Writing),
            "resolving" => Some(Self::Resolving),
            _ => None,
        }
    }
}

/// `repos-push-progress` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushProgress {
    pub binding_id: String,
    pub phase: PushPhase,
    pub percent: Option<u8>,
}

// --- Identity of the bound account ---

/// Emails that count as "the account" in commit author/committer checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityRules {
    pub login: Option<String>,
    /// GitHub user id, when known: then only `ID+login@users.noreply.github.com` is accepted.
    pub user_id: Option<u64>,
    /// Configured commit email and verified emails of the account.
    pub emails: Vec<String>,
}

impl IdentityRules {
    pub fn is_empty(&self) -> bool {
        self.login.is_none() && self.emails.is_empty()
    }

    /// Case-insensitive: one of `emails`, or the account's noreply address (`login@…` or `ID+login@…`).
    pub fn accepts(&self, email: &str) -> bool {
        let email = email.trim();
        if self.emails.iter().any(|e| e.trim().eq_ignore_ascii_case(email)) {
            return true;
        }
        let Some(login) = self.login.as_deref() else {
            return false;
        };
        let Some((local, domain)) = email.rsplit_once('@') else {
            return false;
        };
        if !domain.eq_ignore_ascii_case(NOREPLY_DOMAIN) {
            return false;
        }
        if local.eq_ignore_ascii_case(login) {
            return true;
        }
        match local.split_once('+') {
            // GitHub usa o id numérico real do usuário no noreply; pode diferir do email salvo na conta
            // (troca de email primário, login renomeado, etc.). O sufixo `@login` é o sinal confiável.
            Some((id, l)) if l.eq_ignore_ascii_case(login) && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => true,
            _ => false,
        }
    }
}

/// Name and email written by [`rewrite_authors`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

// --- Git runner ---

/// A work tree plus extra environment for every `git` run (tests isolate the user's Git config with it).
#[derive(Debug, Clone)]
pub struct GitRepo {
    root: PathBuf,
    env: Vec<(String, String)>,
}

/// Commits and objects that a push of `tip` (`HEAD` or `refs/heads/<branch>`) would send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Range {
    pub(crate) commits: Vec<String>,
    pub(crate) objects: Vec<String>,
}

impl Range {
    pub(crate) fn new(tip: &str, tracking: Option<&str>, remote: Option<&str>) -> Self {
        let not_remote = match remote {
            Some(r) => format!("--remotes={r}"),
            None => "--remotes".to_string(),
        };
        let objects = vec![tip.to_string(), "--not".to_string(), not_remote];
        let commits = match tracking {
            Some(t) => vec![format!("{t}..{tip}")],
            None => objects.clone(),
        };
        Self { commits, objects }
    }
}

/// Where `HEAD` goes on a push of `branch` to `remote`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTarget {
    pub remote: String,
    pub branch: String,
    /// Branch on GitHub: the configured upstream's when it is on `remote`, else the local name.
    pub remote_branch: String,
    /// `refs/remotes/<remote>/<remote_branch>` when it exists (the branch is on GitHub, as of the last fetch).
    pub tracking: Option<String>,
    /// `branch.<name>.remote`/`merge` are set (to any remote).
    pub upstream_configured: bool,
}

/// Failure of a network operation (fetch, push, pull), already in user-facing words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitNetError {
    pub message: String,
    /// GitHub rejected the token: refreshing it and trying once more may help.
    pub auth: bool,
}

impl GitNetError {
    fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            auth: false,
        }
    }
}

impl std::fmt::Display for GitNetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What a successful push did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOutput {
    pub remote_branch: String,
    /// The branch did not exist on the remote before (the upstream is configured now when it was missing).
    pub created_upstream: bool,
    /// Short output without credentials, for "Ver detalhes".
    pub summary: String,
}

pub(crate) fn valid_remote_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub(crate) fn valid_branch_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub fn is_hex_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl GitRepo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            env: Vec::new(),
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn command(&self, token: Option<&str>) -> Command {
        let mut c = Command::new(git_program());
        c.arg("-C").arg(&self.root).args(["-c", "core.quotepath=false"]);
        apply_git_env(&mut c, token);
        for (k, v) in &self.env {
            c.env(k, v);
        }
        c.stdin(Stdio::null());
        c
    }

    pub(crate) fn output(&self, args: &[&str]) -> Result<Output, String> {
        self.command(None).args(args).output().map_err(spawn_error)
    }

    /// The same repository with another work tree (a linked `git worktree`), keeping the extra environment.
    pub fn at(&self, root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            env: self.env.clone(),
        }
    }

    /// Trimmed stdout when `git` succeeds and prints something.
    pub(crate) fn value(&self, args: &[&str]) -> Option<String> {
        let out = self.output(args).ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !s.is_empty()).then_some(s)
    }

    pub(crate) fn args_with<'a>(head: &[&'a str], range: &'a [String]) -> Vec<&'a str> {
        let mut v: Vec<&str> = head.to_vec();
        v.extend(range.iter().map(String::as_str));
        v.push("--");
        v
    }

    pub(crate) fn count(&self, head: &[&str], range: &[String]) -> Option<usize> {
        let mut h = vec!["rev-list", "--count"];
        h.extend_from_slice(head);
        self.value(&Self::args_with(&h, range))?.parse().ok()
    }

    pub fn branch(&self) -> Option<String> {
        self.value(&["symbolic-ref", "-q", "--short", "HEAD"])
    }

    pub fn head_sha(&self) -> Option<String> {
        self.value(&["rev-parse", "-q", "--verify", "HEAD^{commit}"])
    }

    pub fn dirty_counts(&self) -> Result<DirtyCounts, String> {
        let out = self.output(&["--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=normal"])?;
        if !out.status.success() {
            return Err(format!(
                "Não foi possível ler o estado do repositório ({}).",
                last_line(&String::from_utf8_lossy(&out.stderr))
            ));
        }
        Ok(parse_status_z(&String::from_utf8_lossy(&out.stdout)))
    }

    /// Rebase, merge, cherry-pick or revert left in progress.
    pub fn operation_in_progress(&self) -> bool {
        let names = ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"];
        let mut args = vec!["rev-parse"];
        for n in names {
            args.extend(["--git-path", n]);
        }
        self.value(&args)
            .is_some_and(|out| out.lines().any(|p| self.root.join(p.trim()).exists()))
    }

    /// `RepoRow.sync`: `rev-list --left-right --count @{u}...HEAD`, or (no upstream) commits in no remote.
    /// `None` when the repository cannot be read.
    pub fn sync_summary(&self) -> Option<SyncSummary> {
        let dirty = self.dirty_counts().ok()?.any();
        if self.head_sha().is_none() {
            return Some(SyncSummary {
                dirty,
                ..Default::default()
            });
        }
        if let Some((behind, ahead)) = self
            .value(&["rev-list", "--left-right", "--count", "@{upstream}...HEAD", "--"])
            .and_then(|s| parse_left_right(&s))
        {
            return Some(SyncSummary {
                ahead,
                behind,
                has_upstream: true,
                dirty,
            });
        }
        let ahead = self.count(&[], &["HEAD".into(), "--not".into(), "--remotes".into()])?;
        Some(SyncSummary {
            ahead,
            behind: 0,
            has_upstream: false,
            dirty,
        })
    }

    pub fn push_target(&self, remote: &str, branch: &str) -> PushTarget {
        let cfg_remote = self.value(&["config", "--get", &format!("branch.{branch}.remote")]);
        let cfg_merge = self.value(&["config", "--get", &format!("branch.{branch}.merge")]);
        let upstream_configured = cfg_remote.is_some() && cfg_merge.is_some();
        let remote_branch = match (&cfg_remote, &cfg_merge) {
            (Some(r), Some(m)) if r == remote => m.strip_prefix("refs/heads/").unwrap_or(m).to_string(),
            _ => branch.to_string(),
        };
        let tracking_ref = format!("refs/remotes/{remote}/{remote_branch}");
        let tracking = self
            .value(&["rev-parse", "-q", "--verify", &format!("{tracking_ref}^{{commit}}")])
            .map(|_| tracking_ref);
        PushTarget {
            remote: remote.to_string(),
            branch: branch.to_string(),
            remote_branch,
            tracking,
            upstream_configured,
        }
    }

    pub(crate) fn commits(&self, range: &[String], limit: usize) -> Result<Vec<OutgoingCommit>, String> {
        let max = format!("--max-count={limit}");
        let args = Self::args_with(&["log", "--no-color", "--no-ext-diff", "--numstat", LOG_FORMAT, &max], range);
        let out = self.output(&args)?;
        if !out.status.success() {
            return Err(format!(
                "Não foi possível ler os commits ({}).",
                last_line(&String::from_utf8_lossy(&out.stderr))
            ));
        }
        Ok(parse_log(&String::from_utf8_lossy(&out.stdout)))
    }

    /// `(sha, author email, committer email)` of every commit in `range`.
    pub(crate) fn identities(&self, range: &[String]) -> Vec<(String, String, String)> {
        let args = Self::args_with(&["log", "--format=%H%x1f%ae%x1f%ce"], range);
        let Some(text) = self.value(&args) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| {
                let mut it = l.splitn(3, '\x1f');
                Some((it.next()?.to_string(), it.next()?.to_string(), it.next()?.to_string()))
            })
            .collect()
    }

    /// Blobs over `min_bytes` among the objects of `revs` (`rev-list --objects` piped into `cat-file --batch-check`).
    fn large_blobs(&self, revs: &[String], min_bytes: u64) -> Result<Vec<(String, String, u64)>, String> {
        let mut list = self.command(None);
        list.args(Self::args_with(&["rev-list", "--objects"], revs))
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut list = list.spawn().map_err(spawn_error)?;
        let listed = list.stdout.take().ok_or("Não foi possível ler a saída do Git.")?;
        let mut check = self.command(None);
        check
            .args(["cat-file", "--batch-check=%(objecttype) %(objectsize) %(objectname) %(rest)"])
            .stdin(Stdio::from(listed))
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut check = check.spawn().map_err(spawn_error)?;
        let reader = BufReader::new(check.stdout.take().ok_or("Não foi possível ler a saída do Git.")?);
        let mut found = Vec::new();
        for line in reader.lines().map_while(Result::ok) {
            let mut it = line.splitn(4, ' ');
            let (Some("blob"), Some(size), Some(sha)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let Ok(size) = size.parse::<u64>() else {
                continue;
            };
            if size > min_bytes {
                found.push((sha.to_string(), it.next().unwrap_or("").to_string(), size));
            }
        }
        let _ = list.wait();
        let _ = check.wait();
        found.sort_by(|a, b| b.2.cmp(&a.2));
        Ok(found)
    }

    /// Newest commit of `range` that adds `blob`.
    fn commit_adding(&self, blob: &str, range: &[String]) -> Option<String> {
        let find = format!("--find-object={blob}");
        self.value(&Self::args_with(&["log", "-1", "--format=%H", &find], range))
    }

    fn scan_secrets(&self, range: &[String]) -> Result<Vec<SecretHit>, String> {
        let mut cmd = self.command(None);
        cmd.args(Self::args_with(
            &["log", "-p", "--no-color", "--no-ext-diff", "--no-textconv", "--no-renames", "--unified=0", "--format=%x1e%H"],
            range,
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(spawn_error)?;
        let mut reader = BufReader::new(child.stdout.take().ok_or("Não foi possível ler a saída do Git.")?);
        let mut scanner = SecretScanner::default();
        let mut line = Vec::new();
        let mut total = 0usize;
        loop {
            line.clear();
            let n = match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            total += n;
            if total > SECRET_SCAN_MAX_BYTES || scanner.hits.len() >= MAX_SECRET_CHECKS {
                break;
            }
            scanner.feed(String::from_utf8_lossy(&line).trim_end_matches(['\n', '\r']));
        }
        let _ = child.kill();
        let _ = child.wait();
        scanner.finish();
        Ok(scanner.hits)
    }

    /// Full commit id of `sha` (7–64 hex characters) when it names a commit of this repository.
    pub fn resolve_commit(&self, sha: &str) -> Result<String, String> {
        if !is_hex_sha(sha) {
            return Err("Identificador de commit inválido.".to_string());
        }
        self.value(&["rev-parse", "-q", "--verify", &format!("{sha}^{{commit}}")])
            .ok_or_else(|| "Este commit não existe neste repositório.".to_string())
    }

    /// Files changed by `sha` (against its first parent), with line counts and blob sizes.
    pub fn commit_files(&self, sha: &str) -> Result<Vec<CommitFile>, String> {
        let full = self.resolve_commit(sha)?;
        let parent = self.value(&["rev-parse", "-q", "--verify", &format!("{full}^1")]);
        let trees: Vec<&str> = match &parent {
            Some(p) => vec![p.as_str(), full.as_str()],
            None => vec!["--root", full.as_str()],
        };
        let run = |kind: &str| -> Result<String, String> {
            let mut args = vec!["diff-tree", "-r", "-z", "-M", "--no-commit-id", "--no-ext-diff", "--no-textconv", kind];
            args.extend(&trees);
            let out = self.output(&args)?;
            if !out.status.success() {
                return Err(format!(
                    "Não foi possível ler os arquivos do commit ({}).",
                    last_line(&String::from_utf8_lossy(&out.stderr))
                ));
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let raw = parse_raw_z(&run("--raw")?);
        let stats = parse_numstat_z(&run("--numstat")?);
        let blobs: Vec<String> = raw
            .iter()
            .filter(|r| r.status != FileStatus::Deleted && r.new_mode != "160000")
            .map(|r| r.new_sha.clone())
            .collect();
        let sizes = self.blob_sizes(&blobs);
        Ok(raw
            .into_iter()
            .take(MAX_COMMIT_FILES)
            .map(|r| {
                let (ins, del) = stats.iter().find(|(p, _)| *p == r.path).map(|(_, s)| *s).unwrap_or((None, None));
                let has_size = r.status != FileStatus::Deleted && r.new_mode != "160000";
                CommitFile {
                    binary: ins.is_none() && del.is_none() && stats.iter().any(|(p, _)| *p == r.path),
                    size_bytes: if has_size { sizes.iter().find(|(s, _)| *s == r.new_sha).map(|(_, n)| *n) } else { None },
                    insertions: ins,
                    deletions: del,
                    status: r.status,
                    old_path: r.old_path,
                    path: r.path,
                }
            })
            .collect())
    }

    fn blob_sizes(&self, shas: &[String]) -> Vec<(String, u64)> {
        if shas.is_empty() {
            return Vec::new();
        }
        let mut cmd = self.command(None);
        cmd.args(["cat-file", "--batch-check=%(objectname) %(objectsize)"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(mut child) = cmd.spawn() else {
            return Vec::new();
        };
        let input = shas.join("\n") + "\n";
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take();
        let text = std::thread::scope(|s| {
            s.spawn(move || {
                if let Some(i) = stdin.as_mut() {
                    let _ = i.write_all(input.as_bytes());
                }
                drop(stdin);
            });
            let mut t = String::new();
            if let Some(o) = stdout.as_mut() {
                let _ = o.read_to_string(&mut t);
            }
            t
        });
        let _ = child.wait();
        text.lines()
            .filter_map(|l| {
                let (sha, size) = l.split_once(' ')?;
                Some((sha.to_string(), size.trim().parse().ok()?))
            })
            .collect()
    }

    pub(crate) fn rev_list(&self, range: &[String]) -> Vec<String> {
        self.value(&Self::args_with(&["rev-list"], range))
            .map(|s| s.lines().map(String::from).collect())
            .unwrap_or_default()
    }
}

const LOG_FORMAT: &str = "--format=%x1e%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%ce%x1f%s%x1f%b%x1f";

pub(crate) fn parse_left_right(s: &str) -> Option<(usize, usize)> {
    let mut it = s.split_whitespace();
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Counts from `git status --porcelain=v1 -z`.
fn parse_status_z(text: &str) -> DirtyCounts {
    let mut c = DirtyCounts::default();
    let mut entries = text.split('\0');
    while let Some(e) = entries.next() {
        let b = e.as_bytes();
        if b.len() < 3 {
            continue;
        }
        match (b[0], b[1]) {
            (b'?', _) => c.untracked += 1,
            (b'!', _) => {}
            (x, y) => {
                if x != b' ' {
                    c.staged += 1;
                }
                if y != b' ' {
                    c.unstaged += 1;
                }
                if x == b'R' || x == b'C' {
                    entries.next();
                }
            }
        }
    }
    c
}

fn parse_log(text: &str) -> Vec<OutgoingCommit> {
    text.split('\x1e')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.splitn(11, '\x1f').collect();
            if f.len() < 11 {
                return None;
            }
            let (mut files, mut ins, mut del) = (0, 0, 0);
            for line in f[10].lines() {
                let mut it = line.splitn(3, '\t');
                let (Some(a), Some(d), Some(_)) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                files += 1;
                ins += a.parse::<usize>().unwrap_or(0);
                del += d.parse::<usize>().unwrap_or(0);
            }
            Some(OutgoingCommit {
                sha: f[0].trim().to_string(),
                short_sha: f[1].to_string(),
                is_merge: f[2].split_whitespace().count() > 1,
                author_name: f[3].to_string(),
                author_email: f[4].to_string(),
                authored_at: f[5].to_string(),
                committer_name: f[6].to_string(),
                committer_email: f[7].to_string(),
                subject: f[8].to_string(),
                body: f[9].trim_end().to_string(),
                files_changed: files,
                insertions: ins,
                deletions: del,
                checks: Vec::new(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawEntry {
    status: FileStatus,
    new_mode: String,
    new_sha: String,
    path: String,
    old_path: Option<String>,
}

/// `git diff-tree --raw -z`: `:old_mode new_mode old_sha new_sha STATUS\0path\0` (two paths for R/C).
fn parse_raw_z(text: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut it = text.split('\0');
    while let Some(meta) = it.next() {
        let Some(meta) = meta.strip_prefix(':') else {
            continue;
        };
        let f: Vec<&str> = meta.split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let letter = f[4].chars().next().unwrap_or('M');
        let status = match letter {
            'A' => FileStatus::Added,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            'C' => FileStatus::Copied,
            'T' => FileStatus::TypeChanged,
            _ => FileStatus::Modified,
        };
        let (path, old_path) = if matches!(letter, 'R' | 'C') {
            let old = it.next().unwrap_or("").to_string();
            (it.next().unwrap_or("").to_string(), Some(old))
        } else {
            (it.next().unwrap_or("").to_string(), None)
        };
        out.push(RawEntry {
            status,
            new_mode: f[1].to_string(),
            new_sha: f[3].to_string(),
            path,
            old_path,
        });
    }
    out
}

pub(crate) type LineStats = (Option<usize>, Option<usize>);

/// `git diff-tree --numstat -z`: `ins\tdel\tpath\0`, or `ins\tdel\t\0old\0new\0` for renames. `-` = binary.
pub(crate) fn parse_numstat_z(text: &str) -> Vec<(String, LineStats)> {
    let mut out = Vec::new();
    let mut it = text.split('\0');
    while let Some(entry) = it.next() {
        let mut f = entry.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            let _old = it.next();
            it.next().unwrap_or("").to_string()
        } else {
            path.to_string()
        };
        out.push((path, (a.parse().ok(), d.parse().ok())));
    }
    out
}

// --- Secret detection ---

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecretHit {
    sha: Option<String>,
    path: String,
    /// With article, e.g. "um token do GitHub".
    kind: &'static str,
    /// The file name itself is sensitive (not a line in it).
    file: bool,
}

/// Reads `git log -p --format=%x1e%H` line by line: added lines with secrets and new sensitive files.
#[derive(Default)]
struct SecretScanner {
    sha: Option<String>,
    path: String,
    in_header: bool,
    new_file: bool,
    seen: HashSet<(String, &'static str)>,
    hits: Vec<SecretHit>,
}

impl SecretScanner {
    fn record(&mut self, kind: &'static str, file: bool) {
        if self.path.is_empty() || !self.seen.insert((self.path.clone(), kind)) {
            return;
        }
        self.hits.push(SecretHit {
            sha: self.sha.clone(),
            path: self.path.clone(),
            kind,
            file,
        });
    }

    fn end_header(&mut self) {
        if self.new_file {
            self.new_file = false;
            if let Some(kind) = sensitive_file_kind(&self.path) {
                self.record(kind, true);
            }
        }
        self.in_header = false;
    }

    fn feed(&mut self, line: &str) {
        if let Some(sha) = line.strip_prefix('\x1e') {
            self.end_header();
            self.sha = Some(sha.trim().to_string());
            return;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            self.end_header();
            self.in_header = true;
            self.path = diff_git_path(rest).unwrap_or_default();
            return;
        }
        if self.in_header {
            if line.starts_with("new file mode") {
                self.new_file = true;
            } else if let Some(p) = line.strip_prefix("+++ ") {
                if let Some(p) = unquote_path(p).strip_prefix("b/") {
                    self.path = p.to_string();
                }
            } else if line.starts_with("@@") || line.starts_with("Binary files") {
                self.end_header();
            }
            return;
        }
        if let Some(added) = line.strip_prefix('+') {
            if !added_line_is_secret_fixture(&self.path, added) {
                if let Some(kind) = secret_kind(added) {
                    self.record(kind, false);
                }
            }
        }
    }

    fn finish(&mut self) {
        self.end_header();
    }
}

/// Path of `a/<p> b/<p>` (same path on both sides, as for new files).
fn diff_git_path(rest: &str) -> Option<String> {
    let rest = rest.trim();
    if rest.len() < 5 || (rest.len() - 5) % 2 != 0 {
        return None;
    }
    let l = (rest.len() - 5) / 2;
    let (a, b) = (rest.get(2..2 + l)?, rest.get(l + 5..)?);
    (rest.starts_with("a/") && rest.get(l + 2..l + 5) == Some(" b/") && a == b).then(|| a.to_string())
}

fn unquote_path(p: &str) -> String {
    let p = p.trim();
    match p.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) => inner.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => p.to_string(),
    }
}

/// Length of the run of `ok` characters right after each `prefix` not glued to a preceding letter/digit.
fn token_runs<'a>(line: &'a str, prefix: &'a str, ok: impl Fn(char) -> bool + 'a) -> impl Iterator<Item = usize> + 'a {
    let mut from = 0;
    std::iter::from_fn(move || loop {
        let i = line.get(from..)?.find(prefix)?;
        let start = from + i;
        from = start + prefix.len();
        let glued = line[..start].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric());
        if !glued {
            return Some(line[from..].chars().take_while(|c| ok(*c)).count());
        }
    })
}

/// Added diff line that only exercises secret detection/redaction in tests or holds placeholders.
fn added_line_is_secret_fixture(path: &str, added: &str) -> bool {
    let path = path.replace('\\', "/");
    if path.contains("/tests/") || path.ends_with("_test.rs") {
        return true;
    }
    let t = added.trim();
    if t.starts_with("//") {
        return true;
    }
    if t.contains("#[test]") || t.contains("#[cfg(test)]") {
        return true;
    }
    if t.contains("assert") {
        return true;
    }
    if t.contains("redact_secrets")
        || t.contains("secret_kind")
        || t.contains("sanitize_output")
        || t.contains("SecretScanner")
        || t.contains("scan_secrets")
        || t.contains("sensitive_file_kind")
        || t.contains("token_runs")
    {
        return true;
    }
    if t.contains("[REDIGIDO]") || t.contains("REDACTED_KEY") || t.contains("REDACTED") {
        return true;
    }
    // Regex / pattern definitions (e.g. redact.rs), not real key material.
    if t.contains("-----BEGIN") && t.contains("PRIVATE KEY") && (t.contains("r\"") || t.contains("r#") || t.contains("(?s)")) {
        return true;
    }
    if path.ends_with("logs_export.rs")
        && (t.contains("test_fixtures") || t.contains("archive_is_redacted") || t.contains("assert!"))
    {
        return true;
    }
    if path.ends_with("redact.rs") && t.contains("redact_secrets") && t.contains("assert") {
        return true;
    }
    if path.ends_with("outgoing.rs") && t.contains("secret_kind") && t.contains("assert") {
        return true;
    }
    if t.contains("OPENSSH PRIVATE KEY")
        && (t.contains("b3BlbnNza") || t.contains("MIIEow") || t.contains("AAAA") || t.contains("format!"))
    {
        return true;
    }
    t.contains("ghp_") && t.contains(".repeat(")
}

/// Kind of secret on an added line (with article), or `None`. Only clear token formats count.
fn secret_kind(line: &str) -> Option<&'static str> {
    let alnum = |c: char| c.is_ascii_alphanumeric();
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if token_runs(line, prefix, alnum).any(|n| n >= 36) {
            return Some("um token do GitHub");
        }
    }
    if token_runs(line, "github_pat_", |c| alnum(c) || c == '_').any(|n| n >= 22) {
        return Some("um token do GitHub");
    }
    if token_runs(line, "AKIA", |c| c.is_ascii_uppercase() || c.is_ascii_digit()).any(|n| n == 16) {
        return Some("uma chave de acesso da AWS");
    }
    if let Some(i) = line.find("-----BEGIN ") {
        if line[i..].contains("PRIVATE KEY-----") {
            return Some("uma chave privada");
        }
    }
    for prefix in ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"] {
        if token_runs(line, prefix, |c| alnum(c) || c == '-').any(|n| n >= 10) {
            return Some("um token do Slack");
        }
    }
    if token_runs(line, "sk_live_", alnum).any(|n| n >= 16) {
        return Some("uma chave secreta do Stripe");
    }
    None
}

/// Files that usually hold secrets: `.env` (not `.env.example`), SSH private keys, `*.pem`.
fn sensitive_file_kind(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let template = [".example", ".sample", ".template", ".dist"].iter().any(|s| name.ends_with(s));
    if name == ".env" || (name.starts_with(".env.") && !template) {
        return Some("variáveis de ambiente");
    }
    if ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"].contains(&name.as_str()) {
        return Some("chave SSH privada");
    }
    if name.ends_with(".pem") {
        return Some("certificado ou chave");
    }
    None
}

// --- Outgoing state ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    Allowed,
    Denied,
    /// The API could not answer; the text says why (full sentence).
    Unknown(String),
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    NotRequested,
    Fetched,
    /// Full sentence from [`fetch`].
    Failed(String),
}

/// Size thresholds of GitHub (100 MiB rejected, 50 MiB warned); tests lower them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileLimits {
    pub warn_bytes: u64,
    pub max_bytes: u64,
}

impl Default for FileLimits {
    fn default() -> Self {
        Self {
            warn_bytes: 50 * MIB,
            max_bytes: 100 * MIB,
        }
    }
}

/// What the app knows about the binding besides the repository itself.
#[derive(Debug, Clone)]
pub struct OutgoingContext<'a> {
    pub binding_id: &'a str,
    /// Local branch to check (`None`: the current one). Another branch is checked without its work tree:
    /// no pending changes, no author fix and no pull.
    pub branch: Option<&'a str>,
    /// Remote of the binding (`None`: no GitHub remote).
    pub remote: Option<&'a str>,
    /// `(owner, repo)` on GitHub, for links and messages.
    pub github: Option<(&'a str, &'a str)>,
    pub identity: &'a IdentityRules,
    pub account_connected: bool,
    pub permission: Permission,
    pub fetch: FetchOutcome,
    pub limits: FileLimits,
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / MIB as f64)
}

/// `https://github.com/<owner>/<repo>/tree/<branch>` (no credentials, unusual characters escaped).
pub fn github_branch_url(owner: &str, repo: &str, branch: &str) -> String {
    let mut b = String::new();
    for c in branch.chars() {
        if c.is_ascii_alphanumeric() || "-._~/".contains(c) {
            b.push(c);
        } else {
            let mut buf = [0u8; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                b.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    format!("https://github.com/{owner}/{repo}/tree/{b}")
}

/// Commits that a push would send and every check; never touches the network.
pub fn outgoing_state(repo: &GitRepo, ctx: &OutgoingContext) -> Result<OutgoingState, String> {
    use CheckCode as C;
    use CheckSeverity::{Error, Info, Warning};

    if !repo.root().is_dir() {
        return Err(format!(
            "A pasta {} não existe mais. Desvincule-a ou clone de novo.",
            repo.root().display()
        ));
    }
    let current = repo.branch().filter(|b| valid_branch_name(b));
    let other = match ctx.branch {
        Some(b) if current.as_deref() != Some(b) => {
            if !valid_branch_name(b) {
                return Err("Nome de branch inválido.".into());
            }
            Some(b)
        }
        _ => None,
    };
    let (branch, head_sha, tip, dirty) = match other {
        Some(b) => {
            let tip = format!("refs/heads/{b}");
            let sha = repo
                .value(&["rev-parse", "-q", "--verify", &format!("{tip}^{{commit}}")])
                .ok_or_else(|| format!("O branch {b} não existe nesta pasta. Atualize a lista de branches."))?;
            (Some(b.to_string()), Some(sha), tip, DirtyCounts::default())
        }
        None => (current, repo.head_sha(), "HEAD".to_string(), repo.dirty_counts()?),
    };
    let remote = ctx.remote.filter(|r| valid_remote_name(r));
    let account = match ctx.identity.login.as_deref() {
        Some(l) => format!("a conta @{l}"),
        None => "a conta vinculada".to_string(),
    };
    let account_cap = match ctx.identity.login.as_deref() {
        Some(l) => format!("A conta @{l}"),
        None => "A conta vinculada".to_string(),
    };

    let mut checks: Vec<PushCheck> = Vec::new();
    if !ctx.account_connected {
        checks.push(PushCheck::new(
            C::AccountDisconnected,
            Error,
            format!("{account_cap} não está conectada ao GitHub. Conecte a conta de novo em «Contas»."),
        ));
    }
    if let FetchOutcome::Failed(why) = &ctx.fetch {
        checks.push(PushCheck::new(
            C::FetchFailed,
            Warning,
            format!("Não foi possível atualizar com o GitHub agora. {why} As informações podem estar desatualizadas."),
        ));
    }
    match &ctx.permission {
        Permission::Denied => {
            let repo_name = ctx.github.map(|(o, r)| format!(" em {o}/{r}")).unwrap_or_default();
            checks.push(PushCheck::new(
                C::NoPushPermission,
                Error,
                format!("{account_cap} não tem permissão para enviar commits{repo_name}. Peça acesso ou troque a conta do repositório."),
            ));
        }
        Permission::Unknown(why) => checks.push(PushCheck::new(
            C::FetchFailed,
            Warning,
            format!("Não foi possível confirmar no GitHub se {account} pode enviar para este repositório. {why}"),
        )),
        Permission::Allowed | Permission::NotChecked => {}
    }
    if remote.is_none() {
        checks.push(PushCheck::new(
            C::NoRemote,
            Error,
            "Este repositório não tem um remoto do GitHub. Adicione o remoto `origin` e vincule o repositório de novo.",
        ));
    }
    if head_sha.is_some() && branch.is_none() {
        checks.push(PushCheck::new(
            C::DetachedHead,
            Error,
            "Você não está em nenhum branch agora (HEAD solto). Volte para um branch antes de enviar.",
        ));
    }
    if dirty.any() {
        let n = dirty.staged.max(dirty.unstaged) + dirty.untracked;
        checks.push(PushCheck::new(
            C::DirtyWorktree,
            Info,
            format!(
                "Há alterações que ainda não viraram commit ({}). Elas não vão junto no envio.",
                plural(n.max(1), "arquivo", "arquivos")
            ),
        ));
    }

    let target = match (remote, branch.as_deref()) {
        (Some(r), Some(b)) => Some(repo.push_target(r, b)),
        _ => None,
    };
    let tracking = target.as_ref().and_then(|t| t.tracking.clone());
    let range = Range::new(&tip, tracking.as_deref(), remote);
    let (ahead, behind) = match (&head_sha, &tracking) {
        (None, _) => (0, 0),
        (Some(_), Some(t)) => repo
            .value(&["rev-list", "--left-right", "--count", &format!("{t}...{tip}"), "--"])
            .and_then(|s| parse_left_right(&s))
            .map(|(behind, ahead)| (ahead, behind))
            .unwrap_or((0, 0)),
        (Some(_), None) => (repo.count(&[], &range.commits).unwrap_or(0), 0),
    };

    let mut commits = if ahead > 0 {
        repo.commits(&range.commits, MAX_COMMITS + 1)?
    } else {
        Vec::new()
    };
    let truncated = ahead > MAX_COMMITS || commits.len() > MAX_COMMITS;
    commits.truncate(MAX_COMMITS);

    if head_sha.is_none() || (ahead == 0 && remote.is_some() && branch.is_some()) {
        checks.push(PushCheck::new(C::NothingToPush, Info, "Não há commits para enviar."));
    }
    if behind > 0 {
        let n = plural(behind, "commit", "commits");
        checks.push(if ahead > 0 {
            PushCheck::new(
                C::BehindRemote,
                Error,
                format!("O GitHub tem {n} que você ainda não tem. Traga essas mudanças antes de enviar, ou o GitHub vai recusar o envio."),
            )
        } else {
            PushCheck::new(C::BehindRemote, Info, format!("O GitHub tem {n} que você ainda não tem."))
        });
    }

    let mut commit_checks: Vec<PushCheck> = Vec::new();
    for c in commits.iter().filter(|c| c.is_merge) {
        commit_checks.push(PushCheck::new(C::MergeCommit, Info, "Este é um commit de merge.").sha(Some(c.sha.as_str())));
    }
    if !ctx.identity.is_empty() {
        let people = if truncated {
            repo.identities(&range.commits)
        } else {
            commits
                .iter()
                .map(|c| (c.sha.clone(), c.author_email.clone(), c.committer_email.clone()))
                .collect()
        };
        for (sha, author, committer) in &people {
            if !ctx.identity.accepts(author) {
                commit_checks.push(
                    PushCheck::new(
                        C::AuthorMismatch,
                        Error,
                        format!("O autor deste commit usa o email {author}, que não é d{account}. Corrija o autor antes de enviar."),
                    )
                    .sha(Some(sha.as_str())),
                );
            }
            if !ctx.identity.accepts(committer) {
                commit_checks.push(
                    PushCheck::new(
                        C::CommitterMismatch,
                        Warning,
                        format!("Este commit foi gravado com o email {committer}, que não é d{account}."),
                    )
                    .sha(Some(sha.as_str())),
                );
            }
        }
    }
    if ahead > 0 {
        let big = repo.large_blobs(&range.objects, ctx.limits.warn_bytes).unwrap_or_default();
        for (blob, path, size) in big.iter().take(MAX_LARGE_FILE_CHECKS) {
            let sha = repo.commit_adding(blob, &range.commits);
            let shown = if path.is_empty() { blob.as_str() } else { path.as_str() };
            let check = if *size > ctx.limits.max_bytes {
                PushCheck::new(
                    C::FileTooLarge,
                    Error,
                    format!(
                        "O arquivo {shown} tem {}. O GitHub não aceita arquivos com mais de {}. Tire o arquivo dos commits antes de enviar.",
                        mb(*size),
                        mb(ctx.limits.max_bytes)
                    ),
                )
            } else {
                PushCheck::new(
                    C::LargeFile,
                    Warning,
                    format!(
                        "O arquivo {shown} tem {}. O GitHub aceita, mas recomenda arquivos com menos de {}.",
                        mb(*size),
                        mb(ctx.limits.warn_bytes)
                    ),
                )
            };
            commit_checks.push(check.sha(sha.as_deref()).path(shown));
        }
        for hit in repo.scan_secrets(&range.commits).unwrap_or_default() {
            let message = if hit.file {
                format!(
                    "O arquivo {} foi adicionado e costuma guardar senhas ou chaves ({}). Confira se ele deve ir para o GitHub.",
                    hit.path, hit.kind
                )
            } else {
                format!("Possível segredo em {}: parece {}. Confira antes de enviar.", hit.path, hit.kind)
            };
            commit_checks.push(PushCheck::new(C::SecretSuspected, Warning, message).sha(hit.sha.as_deref()).path(&hit.path));
        }
    }
    for c in commits.iter_mut() {
        c.checks = commit_checks.iter().filter(|k| k.sha.as_deref() == Some(c.sha.as_str())).cloned().collect();
    }
    checks.extend(commit_checks);

    let has_error = checks.iter().any(|c| c.severity == Error);
    let has_warnings = checks.iter().any(|c| c.severity == Warning);
    let busy = repo.operation_in_progress();
    let merges = ahead > 0 && repo.count(&["--min-parents=2"], &range.commits).unwrap_or(1) > 0;
    let on_repo_branch = other.is_none() && branch.is_some() && remote.is_some() && !busy;
    let on_branch = on_repo_branch && dirty.tracked_clean();

    Ok(OutgoingState {
        binding_id: ctx.binding_id.to_string(),
        remote_name: remote.map(String::from),
        upstream: tracking.as_deref().map(|t| t.trim_start_matches("refs/remotes/").to_string()),
        remote_branch_url: match (ctx.github, &target) {
            (Some((o, r)), Some(t)) if t.tracking.is_some() => Some(github_branch_url(o, r, &t.remote_branch)),
            _ => None,
        },
        branch,
        ahead,
        behind,
        fetched: ctx.fetch == FetchOutcome::Fetched,
        dirty,
        commits,
        truncated,
        checks,
        head_sha,
        can_push: !has_error,
        has_warnings,
        can_fix_authors: on_repo_branch && ahead > 0 && !merges,
        can_pull: on_branch && tracking.is_some() && behind > 0,
        suggested_push_account_id: None,
        suggested_push_account_login: None,
        preferred_push_account_id: None,
        preferred_push_account_login: None,
    })
}

/// Why a push must not start (`None`: it may). `expected_head` is what the user reviewed.
pub fn push_refusal(state: &OutgoingState, expected_head: &str, acknowledge_warnings: bool) -> Option<String> {
    let expected = expected_head.trim();
    if expected.is_empty() || state.head_sha.as_deref() != Some(expected) {
        return Some("Os commits mudaram desde a última verificação. Confira a lista de novo antes de enviar.".into());
    }
    if let Some(e) = state.checks.iter().find(|c| c.severity == CheckSeverity::Error) {
        return Some(format!("O envio foi bloqueado. {}", e.message));
    }
    if state.has_warnings && !acknowledge_warnings {
        return Some("Há avisos sobre estes commits. Confira os avisos e confirme o envio.".into());
    }
    if state.ahead == 0 && state.upstream.is_some() {
        return Some("Não há commits para enviar.".into());
    }
    None
}

// --- Network operations ---

pub(crate) fn last_line(text: &str) -> &str {
    text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("erro desconhecido")
}

pub(crate) fn has(lower: &str, any: &[&str]) -> bool {
    any.iter().any(|p| lower.contains(p))
}

/// Removes `user:password@` from every URL in `line`.
fn strip_url_userinfo(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        out.push_str(authority.rfind('@').map(|at| &authority[at + 1..]).unwrap_or(authority));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Git output safe to show: without the token and without credentials embedded in URLs.
pub fn sanitize_output(text: &str, token: Option<&str>) -> String {
    let text = redact(text, token.unwrap_or(""));
    text.lines().map(strip_url_userinfo).collect::<Vec<_>>().join("\n")
}

/// Connection and access problems shared by fetch, pull and push.
fn net_error(lower: &str) -> Option<GitNetError> {
    if has(
        lower,
        &["authentication failed", "invalid username or password", "could not read username", "bad credentials", "returned error: 401"],
    ) {
        return Some(GitNetError {
            message: "O GitHub recusou o acesso da conta. Conecte a conta de novo em «Contas» e tente de novo.".into(),
            auth: true,
        });
    }
    if has(lower, &["repository not found", "returned error: 404"]) {
        return Some(GitNetError::other(
            "O repositório não foi encontrado no GitHub, ou a conta não tem acesso a ele.",
        ));
    }
    if has(lower, &["low speed limit", "operation too slow"]) {
        return Some(GitNetError::other("A conexão com o GitHub está lenta demais. Tente de novo daqui a pouco."));
    }
    if has(
        lower,
        &["could not resolve host", "failed to connect", "connection timed out", "network is unreachable", "operation timed out", "connection reset", "connection refused"],
    ) {
        return Some(GitNetError::other("Sem conexão com o GitHub. Veja se você está online e tente de novo."));
    }
    None
}

/// Turns the (sanitized) output of a failed `git push` into a pt-BR message.
pub fn friendly_push_error(text: &str) -> GitNetError {
    let lower = text.to_lowercase();
    if let Some(msg) = text.lines().find_map(|l| l.split_once("GitPass [").and_then(|(_, r)| r.split_once("] "))).map(|(_, m)| m) {
        return GitNetError::other(format!("A verificação do GitPass bloqueou o envio: {}", msg.trim()));
    }
    if has(&lower, &["push cannot contain secrets", "secret scanning", "push protection", "gh009"]) {
        return GitNetError::other(
            "O GitHub bloqueou o envio porque encontrou um segredo (senha, chave ou token) nos commits. Tire o segredo dos commits e tente de novo.",
        );
    }
    if has(&lower, &["gh001", "large files detected", "exceeds github's file size limit"]) {
        return GitNetError::other(
            "O GitHub recusou o envio porque há um arquivo maior que 100 MB. Tire o arquivo dos commits e tente de novo.",
        );
    }
    if has(&lower, &["gh006", "protected branch"]) {
        return GitNetError::other(
            "Este branch é protegido no GitHub e não aceita envios diretos. Envie para outro branch e abra um pull request.",
        );
    }
    if has(&lower, &["gh013", "repository rule violations"]) {
        return GitNetError::other(
            "As regras deste repositório no GitHub não permitem este envio. Veja as regras do repositório ou envie para outro branch.",
        );
    }
    if has(&lower, &["without `workflow` scope", "without 'workflow' scope", "workflow scope"]) {
        return GitNetError::other(
            "O GitHub não deixa o GitPass alterar arquivos em .github/workflows com a permissão atual. Reconecte a conta em «Contas» e tente de novo.",
        );
    }
    if has(&lower, &["non-fast-forward", "[rejected]", "fetch first", "stale info", "tip of your current branch is behind"]) {
        return GitNetError::other(
            "O GitHub recusou o envio porque tem commits que você ainda não tem. Traga as mudanças do GitHub e tente de novo.",
        );
    }
    if has(&lower, &["permission to", "write access to repository not granted", "returned error: 403"]) {
        return GitNetError::other(
            "A conta vinculada não tem permissão para enviar commits para este repositório. Peça acesso ou troque a conta.",
        );
    }
    if has(&lower, &["pre-push hook", "hook declined"]) {
        return GitNetError::other(format!("Uma verificação do Git bloqueou o envio: {}", last_line(text)));
    }
    net_error(&lower).unwrap_or_else(|| GitNetError::other(format!("Não foi possível enviar: {}", last_line(text))))
}

pub(crate) fn friendly_fetch_error(text: &str) -> GitNetError {
    net_error(&text.to_lowercase())
        .unwrap_or_else(|| GitNetError::other(format!("O Git respondeu: {}.", last_line(text).trim_end_matches('.'))))
}

/// `git fetch <remote>` with `token` (`None` in tests with `file://` remotes).
pub fn fetch(repo: &GitRepo, remote: &str, token: Option<&str>) -> Result<(), GitNetError> {
    if !valid_remote_name(remote) {
        return Err(GitNetError::other("Nome de remoto inválido."));
    }
    let out = repo
        .command(token)
        .args(["fetch", "--quiet", "--recurse-submodules=no", remote])
        .output()
        .map_err(|e| GitNetError::other(spawn_error(e)))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(friendly_fetch_error(&sanitize_output(&String::from_utf8_lossy(&out.stderr), token)))
    }
}

fn porcelain_summary(stdout: &str, stderr: &str) -> String {
    let mut lines: Vec<String> = stdout
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && *l != "Done")
        .map(String::from)
        .collect();
    lines.extend(stderr.lines().map(str::trim).filter(|l| l.starts_with("remote:")).map(String::from));
    lines.truncate(SUMMARY_MAX_LINES);
    lines.join("\n")
}

/// `git push --porcelain --progress <remote> HEAD:refs/heads/<branch on GitHub>`. Configures the upstream
/// afterwards when the branch had none. `progress` receives counting/compressing/writing/resolving samples.
pub fn push(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
    token: Option<&str>,
    progress: impl FnMut(PushPhase, Option<u8>),
) -> Result<PushOutput, GitNetError> {
    push_ref(repo, remote, branch, true, token, progress)
}

/// Like [`push`], for a local branch that is not checked out here: `git push <remote>
/// refs/heads/<branch>:refs/heads/<branch on GitHub>`, then `git branch --set-upstream-to` when it had none.
pub fn push_branch(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
    token: Option<&str>,
    progress: impl FnMut(PushPhase, Option<u8>),
) -> Result<PushOutput, GitNetError> {
    push_ref(repo, remote, branch, false, token, progress)
}

fn push_ref(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
    from_head: bool,
    token: Option<&str>,
    mut progress: impl FnMut(PushPhase, Option<u8>),
) -> Result<PushOutput, GitNetError> {
    if !valid_remote_name(remote) || !valid_branch_name(branch) {
        return Err(GitNetError::other("Nome de remoto ou de branch inválido."));
    }
    let target = repo.push_target(remote, branch);
    let source = if from_head { "HEAD".to_string() } else { format!("refs/heads/{branch}") };
    let refspec = format!("{source}:refs/heads/{}", target.remote_branch);
    let mut cmd = repo.command(token);
    cmd.args(["push", "--porcelain", "--progress", remote, &refspec])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| GitNetError::other(spawn_error(e)))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (out_text, err_lines) = std::thread::scope(|s| {
        let reader = s.spawn(move || {
            let mut t = String::new();
            if let Some(mut o) = stdout {
                let _ = o.read_to_string(&mut t);
            }
            t
        });
        let mut lines: Vec<String> = Vec::new();
        let mut last: Option<(PushPhase, u8)> = None;
        if let Some(err) = stderr {
            for_each_line(err, |line| {
                if let Some(p) = parse_progress_line(line) {
                    if let Some(phase) = PushPhase::from_git(p.phase) {
                        if last != Some((phase, p.percent)) {
                            last = Some((phase, p.percent));
                            progress(phase, Some(p.percent));
                        }
                    }
                } else if !line.trim().is_empty() {
                    lines.push(line.trim().to_string());
                    if lines.len() > 80 {
                        lines.remove(0);
                    }
                }
            });
        }
        (reader.join().unwrap_or_default(), lines)
    });
    let status = child
        .wait()
        .map_err(|e| GitNetError::other(format!("Algo deu errado ao esperar o Git terminar: {e}")))?;
    let stdout = sanitize_output(&out_text, token);
    let stderr = sanitize_output(&err_lines.join("\n"), token);
    let rejected = stdout.lines().any(|l| l.starts_with('!'));
    if !status.success() || rejected {
        return Err(friendly_push_error(&format!("{stderr}\n{stdout}")));
    }
    if !target.upstream_configured {
        let upstream = format!("--set-upstream-to={remote}/{}", target.remote_branch);
        let set = !from_head
            && repo
                .output(&["branch", "--quiet", &upstream, branch])
                .is_ok_and(|o| o.status.success());
        if !set {
            let _ = repo.output(&["config", &format!("branch.{branch}.remote"), remote]);
            let _ = repo.output(&["config", &format!("branch.{branch}.merge"), &format!("refs/heads/{}", target.remote_branch)]);
        }
    }
    Ok(PushOutput {
        created_upstream: target.tracking.is_none(),
        remote_branch: target.remote_branch,
        summary: porcelain_summary(&stdout, &stderr),
    })
}

/// Puts `branch` back at `old` after a failed rebase (only called when the tracked tree was clean).
fn restore(repo: &GitRepo, branch: &str, old: &str) {
    if repo.operation_in_progress() {
        let _ = repo.output(&["rebase", "--abort"]);
    }
    if repo.branch().as_deref() != Some(branch) {
        let _ = repo.output(&["checkout", "-q", branch, "--"]);
    }
    if repo.head_sha().as_deref() != Some(old) && repo.branch().as_deref() == Some(branch) {
        let _ = repo.output(&["reset", "--hard", "-q", old]);
    }
}

fn ensure_clean_branch(repo: &GitRepo, branch: &str) -> Result<String, String> {
    if repo.branch().as_deref() != Some(branch) {
        return Err("O branch mudou desde a última verificação. Atualize e tente de novo.".into());
    }
    if repo.operation_in_progress() {
        return Err("Há um rebase ou merge em andamento neste repositório. Termine ou cancele no terminal e tente de novo.".into());
    }
    if !repo.dirty_counts()?.tracked_clean() {
        return Err("Há alterações que ainda não viraram commit. Faça um commit ou guarde as alterações (stash) e tente de novo.".into());
    }
    repo.head_sha().ok_or_else(|| "Este repositório ainda não tem commits.".to_string())
}

/// `git pull --rebase <remote> <branch on GitHub>`; on conflict or any failure, aborts and restores `HEAD`.
pub fn pull_rebase(repo: &GitRepo, remote: &str, branch: &str, token: Option<&str>) -> Result<(), GitNetError> {
    if !valid_remote_name(remote) || !valid_branch_name(branch) {
        return Err(GitNetError::other("Nome de remoto ou de branch inválido."));
    }
    let old = ensure_clean_branch(repo, branch).map_err(GitNetError::other)?;
    let target = repo.push_target(remote, branch);
    let out = repo
        .command(token)
        .env("GIT_EDITOR", "true")
        .args(["pull", "--rebase", "--no-autostash", "--recurse-submodules=no", remote, &target.remote_branch])
        .output()
        .map_err(|e| GitNetError::other(spawn_error(e)))?;
    if out.status.success() {
        return Ok(());
    }
    let text = sanitize_output(
        &format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout)),
        token,
    );
    let conflict = repo.operation_in_progress() || has(&text.to_lowercase(), &["conflict", "could not apply"]);
    restore(repo, branch, &old);
    if conflict {
        return Err(GitNetError::other(
            "Não foi possível trazer os commits do GitHub: as mudanças de lá conflitam com as suas. Nada foi alterado. Resolva o conflito pelo terminal ou pelo seu editor.",
        ));
    }
    let mut e = friendly_fetch_error(&text);
    if !e.auth && net_error(&text.to_lowercase()).is_none() {
        e.message = format!("Não foi possível trazer os commits do GitHub. Nada foi alterado. {}", e.message);
    }
    Err(e)
}

/// Whether unpushed commits need author/committer aligned with `rules`.
pub fn unpushed_identity_mismatch(repo: &GitRepo, remote: &str, branch: &str, rules: &IdentityRules) -> Result<bool, String> {
    if rules.is_empty() {
        return Ok(false);
    }
    if !valid_remote_name(remote) || !valid_branch_name(branch) {
        return Ok(false);
    }
    let target = repo.push_target(remote, branch);
    let range = Range::new("HEAD", target.tracking.as_deref(), Some(remote));
    if range.commits.is_empty() {
        return Ok(false);
    }
    let people = repo.identities(&range.commits);
    Ok(people
        .iter()
        .any(|(_, author, committer)| !rules.accepts(author) || !rules.accepts(committer)))
}

/// Rewrites author and committer of the unpushed commits of `branch` (linear, no merges) as `who`.
/// Hooks are off for this rebase only. On any failure the branch is put back where it was.
pub fn rewrite_authors(repo: &GitRepo, remote: &str, branch: &str, who: &Identity) -> Result<(), String> {
    rewrite_authors_inner(repo, remote, branch, who, false)
}

/// Like [`rewrite_authors`], but usa `git rebase --autostash` para guardar alterações sem commit temporariamente.
pub fn rewrite_authors_autostash(repo: &GitRepo, remote: &str, branch: &str, who: &Identity) -> Result<(), String> {
    rewrite_authors_inner(repo, remote, branch, who, true)
}

fn rewrite_authors_inner(repo: &GitRepo, remote: &str, branch: &str, who: &Identity, autostash: bool) -> Result<(), String> {
    if !valid_remote_name(remote) || !valid_branch_name(branch) {
        return Err("Nome de remoto ou de branch inválido.".into());
    }
    if who.name.trim().is_empty() || who.email.trim().is_empty() {
        return Err("A conta não tem nome e email de commit. Escolha um email em «Contas» e tente de novo.".into());
    }
    let old = ensure_branch_for_author_rewrite(repo, branch, autostash)?;
    let target = repo.push_target(remote, branch);
    let range = Range::new("HEAD", target.tracking.as_deref(), Some(remote));
    let shas = repo.rev_list(&range.commits);
    let Some(oldest) = shas.last() else {
        return Err("Não há commits para corrigir.".into());
    };
    if repo.count(&["--min-parents=2"], &range.commits).unwrap_or(1) > 0 {
        return Err("Há commits de merge entre os que ainda não foram enviados. Não dá para corrigir o autor automaticamente.".into());
    }
    if shas.len() == 1 {
        return rewrite_single_commit_author(repo, branch, who, autostash, &old);
    }
    let git = git_program();
    let git_q = crate::integration::sh_quote(&git.to_string_lossy());
    let name_q = crate::integration::sh_quote(who.name.trim());
    let email_q = crate::integration::sh_quote(who.email.trim());
    let exec = format!(
        "{git_q} -c core.hooksPath=/dev/null -c user.name={name_q} -c user.email={email_q} commit --amend --no-edit --reset-author --allow-empty --no-verify --quiet"
    );
    let base = repo.value(&["rev-parse", "-q", "--verify", &format!("{oldest}^1")]);
    let mut cmd = repo.command(None);
    cmd.args(["-c", "core.hooksPath=/dev/null"])
        .args(["-c", &format!("user.name={}", who.name.trim())])
        .args(["-c", &format!("user.email={}", who.email.trim())])
        .args(["-c", "rebase.autoStash=false", "-c", "rebase.updateRefs=false", "-c", "rebase.autoSquash=false"])
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true");
    for var in ["GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_AUTHOR_DATE", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL", "GIT_COMMITTER_DATE"] {
        cmd.env_remove(var);
    }
    cmd.args(["rebase", "--quiet"]);
    if autostash {
        cmd.arg("--autostash");
    }
    cmd.args(["--exec", &exec]);
    match &base {
        Some(b) => cmd.arg(b),
        None => cmd.arg("--root"),
    };
    let out = cmd.output().map_err(spawn_error)?;
    if out.status.success() && repo.branch().as_deref() == Some(branch) && !repo.operation_in_progress() {
        return Ok(());
    }
    restore(repo, branch, &old);
    Err(author_rewrite_error(&out))
}

fn rewrite_single_commit_author(
    repo: &GitRepo,
    branch: &str,
    who: &Identity,
    autostash: bool,
    old_head: &str,
) -> Result<(), String> {
    let stashed = if autostash {
        stash_before_author_fixup(repo)?
    } else {
        false
    };
    let mut cmd = repo.command(None);
    cmd.args(["-c", "core.hooksPath=/dev/null"])
        .env("GIT_AUTHOR_NAME", who.name.trim())
        .env("GIT_AUTHOR_EMAIL", who.email.trim())
        .env("GIT_COMMITTER_NAME", who.name.trim())
        .env("GIT_COMMITTER_EMAIL", who.email.trim())
        .args(["commit", "--amend", "--reset-author", "--no-edit", "--no-verify", "--quiet"]);
    let out = cmd.output().map_err(spawn_error)?;
    if stashed {
        let pop = repo.command(None).args(["stash", "pop", "--quiet"]).output().map_err(spawn_error)?;
        if !pop.status.success() {
            restore(repo, branch, old_head);
            return Err(format!(
                "O autor foi corrigido, mas não deu para restaurar suas alterações sem commit: {}",
                last_line(&String::from_utf8_lossy(&pop.stderr))
            ));
        }
    }
    if out.status.success() && repo.branch().as_deref() == Some(branch) {
        return Ok(());
    }
    restore(repo, branch, old_head);
    Err(author_rewrite_error(&out))
}

fn stash_before_author_fixup(repo: &GitRepo) -> Result<bool, String> {
    if repo.dirty_counts()?.tracked_clean() {
        return Ok(false);
    }
    let o = repo
        .command(None)
        .args(["stash", "push", "-u", "-m", "gitpass-fix-author"])
        .output()
        .map_err(spawn_error)?;
    let msg = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    if !o.status.success() {
        return Err(author_rewrite_error(&o));
    }
    Ok(!msg.contains("No local changes to save"))
}

fn author_rewrite_error(out: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}\n{stdout}");
    let detail = last_line(&combined);
    format!("Não foi possível corrigir o autor dos commits ({detail}). Nada foi alterado.")
}

fn ensure_branch_for_author_rewrite(repo: &GitRepo, branch: &str, autostash: bool) -> Result<String, String> {
    if repo.branch().as_deref() != Some(branch) {
        return Err("O branch mudou desde a última verificação. Atualize e tente de novo.".into());
    }
    if repo.operation_in_progress() {
        return Err("Há um rebase ou merge em andamento neste repositório. Termine ou cancele no terminal e tente de novo.".into());
    }
    if !autostash && !repo.dirty_counts()?.tracked_clean() {
        return Err("Há alterações que ainda não viraram commit. Faça um commit ou guarde as alterações (stash) e tente de novo.".into());
    }
    repo.head_sha().ok_or_else(|| "Este repositório ainda não tem commits.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct Tmp(PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp() -> Tmp {
        let p = std::env::temp_dir().join(format!("gitpass-outgoing-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p.canonicalize().unwrap())
    }

    /// The user's global/system config (and their global hooks) never take part.
    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new(git_program())
            .arg("-C")
            .arg(dir)
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo(dir: &Path) -> GitRepo {
        GitRepo::new(dir)
            .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }

    struct Setup {
        _t: Tmp,
        bare: PathBuf,
        work: PathBuf,
        root: PathBuf,
    }

    fn set_identity(dir: &Path, name: &str, email: &str) {
        git(dir, &["config", "user.name", name]);
        git(dir, &["config", "user.email", email]);
    }

    fn commit(dir: &Path, file: &str, content: &str, msg: &str) -> String {
        let p = dir.join(file);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
        git(dir, &["rev-parse", "HEAD"])
    }

    /// Bare `file://` remote plus a clone with one pushed commit on `main`.
    fn setup() -> Setup {
        let t = tmp();
        let bare = t.0.join("remote.git");
        let work = t.0.join("work");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        set_identity(&work, "Ana", "ana@example.com");
        git(&work, &["remote", "add", "origin", &format!("file://{}", bare.display())]);
        commit(&work, "README.md", "hello\n", "init");
        git(&work, &["push", "-q", "-u", "origin", "main"]);
        Setup { root: t.0.clone(), _t: t, bare, work }
    }

    fn rules() -> IdentityRules {
        IdentityRules {
            login: Some("ana".into()),
            user_id: Some(42),
            emails: vec!["Ana@Example.com".into()],
        }
    }

    fn state_with(r: &GitRepo, identity: &IdentityRules, limits: FileLimits) -> OutgoingState {
        state_of(r, identity, limits, None).unwrap()
    }

    fn state_of(r: &GitRepo, identity: &IdentityRules, limits: FileLimits, branch: Option<&str>) -> Result<OutgoingState, String> {
        let ctx = OutgoingContext {
            binding_id: "b1",
            branch,
            remote: Some("origin"),
            github: Some(("ana", "proj")),
            identity,
            account_connected: true,
            permission: Permission::Allowed,
            fetch: FetchOutcome::NotRequested,
            limits,
        };
        outgoing_state(r, &ctx)
    }

    fn state(r: &GitRepo) -> OutgoingState {
        state_with(r, &rules(), FileLimits::default())
    }

    fn codes(s: &OutgoingState) -> Vec<CheckCode> {
        s.checks.iter().map(|c| c.code).collect()
    }

    #[test]
    fn identity_rules_accept_account_emails_and_noreply() {
        let r = rules();
        assert!(r.accepts("ana@example.COM"));
        assert!(r.accepts("ana@users.noreply.github.com"));
        assert!(r.accepts("42+Ana@users.noreply.github.com"));
        assert!(r.accepts("43+ana@users.noreply.github.com"));
        assert!(!r.accepts("bob@users.noreply.github.com"));
        assert!(!r.accepts("ana@example.org"));
        let no_id = IdentityRules { user_id: None, ..rules() };
        assert!(no_id.accepts("7+ana@users.noreply.github.com"));
        assert!(!no_id.accepts("x+ana@users.noreply.github.com"));
        assert!(IdentityRules::default().is_empty());
    }

    #[test]
    fn status_counts() {
        let c = parse_status_z("M  a\0 M b\0MM c\0?? d\0R  new\0old\0");
        assert_eq!(c, DirtyCounts { staged: 3, unstaged: 2, untracked: 1 });
        assert!(!c.tracked_clean());
        assert_eq!(parse_status_z(""), DirtyCounts::default());
    }

    #[test]
    fn secret_scan_skips_test_and_redaction_fixtures() {
        let fixtures = [
            (
                "crates/gitpass-core/src/redact.rs",
                r#"            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)","#,
            ),
            (
                "crates/gitpass-core/src/redact.rs",
                r#"            assert!(!s.contains(&token), "{s}");"#,
            ),
            (
                "crates/gitpass-core/src/logs_export.rs",
                r#"            crate::test_fixtures::ghp_token_36(),"#,
            ),
            (
                "crates/gitpass-core/src/outgoing.rs",
                r#"        assert_eq!(secret_kind(&crate::test_fixtures::aws_key_line()), Some("uma chave de acesso da AWS"));"#,
            ),
            (
                "crates/gitpass-core/src/outgoing.rs",
                r#"        let gh = format!("token = \"ghp_{}\"", "a1".repeat(18));"#,
            ),
        ];
        for (path, line) in fixtures {
            assert!(added_line_is_secret_fixture(path, line), "{path}: {line}");
        }
        let real = format!("export GITHUB_TOKEN=ghp_{}", "a".repeat(40));
        assert!(!added_line_is_secret_fixture("app/.env", &real));
        assert!(secret_kind(&real).is_some());
    }

    #[test]
    fn secrets_are_detected_only_for_clear_formats() {
        let gh = format!("token = \"ghp_{}\"", "a1".repeat(18));
        assert_eq!(secret_kind(&gh), Some("um token do GitHub"));
        assert_eq!(
            secret_kind(&crate::test_fixtures::github_pat_sample()),
            Some("um token do GitHub")
        );
        assert_eq!(
            secret_kind(&crate::test_fixtures::aws_key_line()),
            Some("uma chave de acesso da AWS")
        );
        assert_eq!(secret_kind("-----BEGIN OPENSSH PRIVATE KEY-----"), Some("uma chave privada"));
        assert_eq!(
            secret_kind(&crate::test_fixtures::slack_token_line()),
            Some("um token do Slack")
        );
        assert_eq!(
            secret_kind(&crate::test_fixtures::stripe_live_line()),
            Some("uma chave secreta do Stripe")
        );
        for clean in [
            "use ghp_ tokens",
            "ghp_short",
            "-----BEGIN PUBLIC KEY-----",
            "sk_test_123",
        ] {
            assert_eq!(secret_kind(clean), None, "{clean}");
        }
        assert_eq!(secret_kind(&format!("xAKIA{}", "Z".repeat(16))), None);
        assert_eq!(secret_kind(&format!("AKIA{}1", "Z".repeat(16))), None);
    }

    #[test]
    fn sensitive_file_names() {
        assert!(sensitive_file_kind(".env").is_some());
        assert!(sensitive_file_kind("app/.env.local").is_some());
        assert!(sensitive_file_kind("app/.env.example").is_none());
        assert!(sensitive_file_kind("home/.ssh/id_rsa").is_some());
        assert!(sensitive_file_kind("home/.ssh/id_rsa.pub").is_none());
        assert!(sensitive_file_kind("certs/server.PEM").is_some());
        assert!(sensitive_file_kind("src/main.rs").is_none());
    }

    #[test]
    fn diff_git_paths() {
        assert_eq!(diff_git_path("a/x y.txt b/x y.txt").as_deref(), Some("x y.txt"));
        assert_eq!(diff_git_path("a/old b/new"), None);
    }

    #[test]
    fn credentials_are_removed_from_output() {
        let text = "fatal: unable to access 'https://ana:gho_tok@github.com/a/b.git/': 403\nTo https://x-access-token:abc@github.com/a/b";
        let clean = sanitize_output(text, Some("gho_tok"));
        assert!(!clean.contains("gho_tok") && !clean.contains("abc@") && !clean.contains("ana:"), "{clean}");
        assert!(clean.contains("https://github.com/a/b.git/"), "{clean}");
    }

    #[test]
    fn push_errors_are_friendly() {
        let e = |t: &str| friendly_push_error(t).message;
        assert!(e(" ! [rejected]        HEAD -> main (fetch first)\nerror: failed to push some refs").contains("commits que você ainda não tem"));
        assert!(e("remote: error: GH013: Repository rule violations found for refs/heads/main.\nremote: - Push cannot contain secrets").contains("segredo"));
        assert!(e("remote: error: GH013: Repository rule violations found for refs/heads/main.").contains("regras"));
        assert!(e("remote: error: GH006: Protected branch update failed for refs/heads/main.").contains("protegido"));
        assert!(e("remote: error: GH001: Large files detected.").contains("100 MB"));
        assert!(e("remote: Permission to a/b.git denied to ana.\nfatal: unable to access 'https://github.com/a/b.git/': The requested URL returned error: 403").contains("permissão"));
        let auth = friendly_push_error("fatal: Authentication failed for 'https://github.com/a/b.git/'");
        assert!(auth.auth && auth.message.contains("Conecte a conta"));
        assert!(e("fatal: unable to access 'https://github.com/a/b/': Could not resolve host: github.com").contains("Sem conexão"));
        assert!(e("GitPass [GP-OWNER] Este repositório pertence a x.\nerror: failed to push some refs").contains("verificação do GitPass bloqueou o envio: Este repositório pertence a x."));
        assert!(e("something odd").ends_with("something odd"));
    }

    #[test]
    fn sync_summary_tracks_upstream_and_dirty() {
        let s = setup();
        let r = repo(&s.work);
        assert_eq!(r.sync_summary(), Some(SyncSummary { ahead: 0, behind: 0, has_upstream: true, dirty: false }));
        commit(&s.work, "a.txt", "a\n", "a");
        std::fs::write(s.work.join("untracked.txt"), "x").unwrap();
        assert_eq!(r.sync_summary(), Some(SyncSummary { ahead: 1, behind: 0, has_upstream: true, dirty: true }));

        git(&s.work, &["checkout", "-q", "-b", "feature"]);
        commit(&s.work, "b.txt", "b\n", "b");
        assert_eq!(r.sync_summary().map(|x| (x.ahead, x.has_upstream)), Some((2, false)));
        assert_eq!(repo(&s.root.join("missing")).sync_summary(), None);
    }

    #[test]
    fn outgoing_lists_commits_and_checks_authors() {
        let s = setup();
        let r = repo(&s.work);
        let st = state(&r);
        assert_eq!((st.ahead, st.behind, st.upstream.as_deref()), (0, 0, Some("origin/main")));
        assert_eq!(codes(&st), [CheckCode::NothingToPush]);
        assert_eq!(st.remote_branch_url.as_deref(), Some("https://github.com/ana/proj/tree/main"));
        assert!(st.can_push && !st.can_fix_authors && !st.can_pull);

        commit(&s.work, "src/a.rs", "fn a() {}\nfn b() {}\n", "Add a\n\nLonger body.");
        set_identity(&s.work, "Bob", "bob@work.example");
        let bad = commit(&s.work, "src/a.rs", "fn a() {}\n", "Change a");
        std::fs::write(s.work.join("README.md"), "changed\n").unwrap();
        let st = state(&r);
        assert_eq!(st.ahead, 2);
        assert_eq!(st.commits.len(), 2);
        assert_eq!(st.commits[0].sha, bad);
        assert_eq!((st.commits[0].files_changed, st.commits[0].insertions, st.commits[0].deletions), (1, 0, 1));
        assert_eq!(st.commits[1].subject, "Add a");
        assert_eq!(st.commits[1].body, "Longer body.");
        assert_eq!(st.dirty, DirtyCounts { staged: 0, unstaged: 1, untracked: 0 });
        let author = st.checks.iter().find(|c| c.code == CheckCode::AuthorMismatch).unwrap();
        assert_eq!((author.severity, author.sha.as_deref()), (CheckSeverity::Error, Some(bad.as_str())));
        assert!(author.message.contains("bob@work.example") && author.message.contains("@ana"), "{}", author.message);
        assert!(st.checks.iter().any(|c| c.code == CheckCode::CommitterMismatch && c.severity == CheckSeverity::Warning));
        assert_eq!(st.commits[0].checks.len(), 2);
        assert!(st.commits[1].checks.is_empty());
        assert!(codes(&st).contains(&CheckCode::DirtyWorktree));
        assert!(!st.can_push && st.has_warnings);
        assert!(!st.can_fix_authors, "unstaged changes");
        assert_eq!(st.head_sha.as_deref(), Some(bad.as_str()));
        git(&s.work, &["checkout", "--", "README.md"]);
        assert!(state(&r).can_fix_authors);
    }

    #[test]
    fn detached_head_and_missing_remote_are_errors() {
        let s = setup();
        git(&s.work, &["checkout", "-q", "--detach"]);
        let r = repo(&s.work);
        let st = state(&r);
        assert!(codes(&st).contains(&CheckCode::DetachedHead) && !st.can_push && st.branch.is_none());
        let ids = rules();
        let ctx = OutgoingContext {
            binding_id: "b",
            branch: None,
            remote: None,
            github: None,
            identity: &ids,
            account_connected: false,
            permission: Permission::Denied,
            fetch: FetchOutcome::Failed("Sem conexão.".into()),
            limits: FileLimits::default(),
        };
        let st = outgoing_state(&r, &ctx).unwrap();
        for code in [CheckCode::NoRemote, CheckCode::AccountDisconnected, CheckCode::NoPushPermission, CheckCode::FetchFailed] {
            assert!(codes(&st).contains(&code), "{code:?}");
        }
        assert!(st.checks.iter().find(|c| c.code == CheckCode::FetchFailed).unwrap().severity == CheckSeverity::Warning);
    }

    #[test]
    fn behind_remote_after_fetch_blocks_when_diverged_and_pull_rebases() {
        let s = setup();
        let other = s.root.join("other");
        git(&s.root, &["clone", "-q", &format!("file://{}", s.bare.display()), other.to_str().unwrap()]);
        set_identity(&other, "Ana", "ana@example.com");
        commit(&other, "remote.txt", "from github\n", "remote change");
        git(&other, &["push", "-q", "origin", "main"]);

        let r = repo(&s.work);
        assert_eq!(state(&r).behind, 0, "not fetched yet");
        fetch(&r, "origin", None).unwrap();
        let st = state(&r);
        assert_eq!((st.ahead, st.behind), (0, 1));
        let b = st.checks.iter().find(|c| c.code == CheckCode::BehindRemote).unwrap();
        assert_eq!(b.severity, CheckSeverity::Info);
        assert!(st.can_pull);

        commit(&s.work, "local.txt", "mine\n", "local change");
        let st = state(&r);
        assert_eq!((st.ahead, st.behind), (1, 1));
        assert!(st.checks.iter().any(|c| c.code == CheckCode::BehindRemote && c.severity == CheckSeverity::Error));
        assert!(!st.can_push && st.can_pull);
        let err = push(&r, "origin", "main", None, |_, _| {}).unwrap_err();
        assert!(err.message.contains("commits que você ainda não tem"), "{}", err.message);

        pull_rebase(&r, "origin", "main", None).unwrap();
        let st = state(&r);
        assert_eq!((st.ahead, st.behind), (1, 0));
        assert!(s.work.join("remote.txt").exists());
        assert!(fetch(&r, "-bad", None).is_err());
    }

    #[test]
    fn pull_conflict_is_aborted_and_head_restored() {
        let s = setup();
        let other = s.root.join("other");
        git(&s.root, &["clone", "-q", &format!("file://{}", s.bare.display()), other.to_str().unwrap()]);
        set_identity(&other, "Ana", "ana@example.com");
        commit(&other, "README.md", "theirs\n", "theirs");
        git(&other, &["push", "-q", "origin", "main"]);
        let mine = commit(&s.work, "README.md", "mine\n", "mine");
        let r = repo(&s.work);
        let err = pull_rebase(&r, "origin", "main", None).unwrap_err();
        assert!(err.message.contains("conflitam"), "{}", err.message);
        assert_eq!(r.head_sha().as_deref(), Some(mine.as_str()));
        assert_eq!(r.branch().as_deref(), Some("main"));
        assert!(!r.operation_in_progress());
        assert_eq!(std::fs::read_to_string(s.work.join("README.md")).unwrap(), "mine\n");
    }

    #[test]
    fn push_new_branch_sets_upstream_and_reports_progress() {
        let s = setup();
        git(&s.work, &["checkout", "-q", "-b", "feature/x"]);
        commit(&s.work, "f.txt", "feature\n", "feature");
        let r = repo(&s.work);
        let st = state(&r);
        assert_eq!((st.ahead, st.upstream.as_deref(), st.remote_branch_url.as_deref()), (1, None, None));
        assert_eq!(push_refusal(&st, "deadbeef", false).as_deref(), Some("Os commits mudaram desde a última verificação. Confira a lista de novo antes de enviar."));
        assert_eq!(push_refusal(&st, st.head_sha.as_deref().unwrap(), false), None);

        let mut phases = Vec::new();
        let out = push(&r, "origin", "feature/x", None, |p, pct| phases.push((p, pct))).unwrap();
        assert!(out.created_upstream && out.remote_branch == "feature/x");
        assert!(out.summary.contains("refs/heads/feature/x"), "{}", out.summary);
        assert!(phases.iter().all(|(p, _)| *p != PushPhase::Validating));
        assert_eq!(git(&s.work, &["rev-parse", "--abbrev-ref", "feature/x@{upstream}"]), "origin/feature/x");
        let st = state(&r);
        assert_eq!((st.ahead, st.upstream.as_deref()), (0, Some("origin/feature/x")));
        assert_eq!(push_refusal(&st, st.head_sha.as_deref().unwrap(), true).as_deref(), Some("Não há commits para enviar."));
        assert_eq!(git(&s.bare, &["rev-parse", "refs/heads/feature/x"]), st.head_sha.unwrap());
    }

    #[test]
    fn push_refusal_requires_ack_for_warnings_and_blocks_errors() {
        let s = setup();
        commit(&s.work, "conf/.env", "A=1\n", "env");
        let r = repo(&s.work);
        let st = state(&r);
        let head = st.head_sha.clone().unwrap();
        assert!(st.has_warnings && st.can_push);
        assert!(push_refusal(&st, &head, false).unwrap().contains("avisos"));
        assert_eq!(push_refusal(&st, &head, true), None);
        let mut blocked = st.clone();
        blocked.checks.push(PushCheck::new(CheckCode::FileTooLarge, CheckSeverity::Error, "grande"));
        assert_eq!(push_refusal(&blocked, &head, true).as_deref(), Some("O envio foi bloqueado. grande"));
    }

    #[test]
    fn secrets_and_sensitive_files_are_warned_without_the_secret() {
        let s = setup();
        let token = format!("ghp_{}", "Z9".repeat(18));
        let sha = commit(&s.work, "src/config.js", &format!("const t = '{token}';\n"), "config");
        commit(&s.work, "keys/server.pem", "not really\n", "pem");
        commit(&s.work, ".env.example", "A=\n", "example");
        let st = state(&repo(&s.work));
        let secrets: Vec<&PushCheck> = st.checks.iter().filter(|c| c.code == CheckCode::SecretSuspected).collect();
        assert_eq!(secrets.len(), 2, "{secrets:?}");
        let line = secrets.iter().find(|c| c.path.as_deref() == Some("src/config.js")).unwrap();
        assert_eq!((line.severity, line.sha.as_deref()), (CheckSeverity::Warning, Some(sha.as_str())));
        assert!(line.message.contains("token do GitHub"));
        assert!(st.checks.iter().all(|c| !c.message.contains(&token)));
        assert!(secrets.iter().any(|c| c.path.as_deref() == Some("keys/server.pem")));
    }

    #[test]
    fn large_files_are_flagged_by_size() {
        let s = setup();
        let big = commit(&s.work, "data/big.bin", &"x".repeat(3000), "big");
        commit(&s.work, "data/medium.bin", &"y".repeat(1500), "medium");
        let limits = FileLimits { warn_bytes: 1000, max_bytes: 2000 };
        let st = state_with(&repo(&s.work), &rules(), limits);
        let too = st.checks.iter().find(|c| c.code == CheckCode::FileTooLarge).unwrap();
        assert_eq!((too.path.as_deref(), too.sha.as_deref()), (Some("data/big.bin"), Some(big.as_str())));
        let warn = st.checks.iter().find(|c| c.code == CheckCode::LargeFile).unwrap();
        assert_eq!(warn.path.as_deref(), Some("data/medium.bin"));
        assert!(!st.can_push);
        // Already on the remote: not flagged again.
        git(&s.work, &["push", "-q", "origin", "main"]);
        commit(&s.work, "small.txt", "s", "small");
        let st = state_with(&repo(&s.work), &rules(), limits);
        assert!(!codes(&st).contains(&CheckCode::FileTooLarge));
    }

    #[test]
    fn merge_commits_are_listed_and_block_author_fix() {
        let s = setup();
        git(&s.work, &["checkout", "-q", "-b", "side"]);
        commit(&s.work, "side.txt", "s\n", "side");
        git(&s.work, &["checkout", "-q", "main"]);
        commit(&s.work, "main.txt", "m\n", "main");
        git(&s.work, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
        let st = state(&repo(&s.work));
        assert!(st.commits[0].is_merge);
        assert!(st.commits[0].checks.iter().any(|c| c.code == CheckCode::MergeCommit && c.severity == CheckSeverity::Info));
        assert!(!st.can_fix_authors);
        let who = Identity { name: "Ana".into(), email: "ana@example.com".into() };
        assert!(rewrite_authors(&repo(&s.work), "origin", "main", &who).unwrap_err().contains("merge"));
    }

    #[test]
    fn rewrite_authors_fixes_unpushed_commits_only() {
        let s = setup();
        let pushed = git(&s.work, &["rev-parse", "HEAD"]);
        set_identity(&s.work, "Bob", "bob@work.example");
        commit(&s.work, "a.txt", "a\n", "a");
        commit(&s.work, "b.txt", "b\n", "b");
        let r = repo(&s.work);
        assert_eq!(state(&r).checks.iter().filter(|c| c.code == CheckCode::AuthorMismatch).count(), 2);
        let who = Identity { name: "Ana".into(), email: "ana@example.com".into() };
        rewrite_authors(&r, "origin", "main", &who).unwrap();
        let st = state(&r);
        assert_eq!(st.ahead, 2);
        assert!(!codes(&st).contains(&CheckCode::AuthorMismatch) && !codes(&st).contains(&CheckCode::CommitterMismatch), "{:?}", st.checks);
        assert_eq!(st.commits[1].subject, "a");
        assert_eq!(git(&s.work, &["rev-parse", "HEAD~2"]), pushed);
        assert_eq!(git(&s.work, &["config", "user.email"]), "bob@work.example", "local config untouched");
    }

    #[test]
    fn rewrite_authors_handles_never_pushed_repos() {
        let t = tmp();
        git(&t.0, &["init", "-q", "-b", "main"]);
        set_identity(&t.0, "Bob", "bob@work.example");
        git(&t.0, &["remote", "add", "origin", "file:///nonexistent"]);
        commit(&t.0, "a.txt", "a\n", "first");
        commit(&t.0, "b.txt", "b\n", "second");
        let r = repo(&t.0);
        let who = Identity { name: "Ana".into(), email: "ana@example.com".into() };
        rewrite_authors(&r, "origin", "main", &who).unwrap();
        assert_eq!(git(&t.0, &["log", "--format=%ae %ce"]), "ana@example.com ana@example.com\nana@example.com ana@example.com");
        assert_eq!(git(&t.0, &["log", "--format=%s"]), "second\nfirst");
    }

    #[test]
    fn rewrite_authors_restores_on_failure() {
        let s = setup();
        set_identity(&s.work, "Bob", "bob@work.example");
        let head = commit(&s.work, "a.txt", "a\n", "a");
        let r = repo(&s.work);
        std::fs::write(s.work.join("a.txt"), "dirty\n").unwrap();
        let who = Identity { name: "Ana".into(), email: "ana@example.com".into() };
        assert!(rewrite_authors(&r, "origin", "main", &who).unwrap_err().contains("ainda não viraram commit"));
        git(&s.work, &["checkout", "--", "a.txt"]);
        // A hook-free failure inside the rebase: GPG signing with a program that does not exist.
        git(&s.work, &["config", "commit.gpgSign", "true"]);
        git(&s.work, &["config", "gpg.program", "/nonexistent/gpg"]);
        let err = rewrite_authors(&r, "origin", "main", &who).unwrap_err();
        assert!(err.contains("Nada foi alterado"), "{err}");
        assert_eq!(r.head_sha().as_deref(), Some(head.as_str()));
        assert_eq!(r.branch().as_deref(), Some("main"));
        assert!(!r.operation_in_progress());
    }

    #[test]
    fn commit_files_lists_statuses_sizes_and_binaries() {
        let s = setup();
        commit(&s.work, "old.txt", "one\ntwo\nthree\nfour\nfive\n", "old");
        std::fs::write(s.work.join("README.md"), "hello\nworld\n").unwrap();
        git(&s.work, &["mv", "old.txt", "renamed.txt"]);
        std::fs::write(s.work.join("logo.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
        let sha = commit(&s.work, "added.txt", "new\n", "mixed");
        git(&s.work, &["rm", "-q", "added.txt"]);
        let del = commit(&s.work, "x.txt", "x", "delete");
        let r = repo(&s.work);

        let files = r.commit_files(&sha[..10]).unwrap();
        let get = |p: &str| files.iter().find(|f| f.path == p).unwrap_or_else(|| panic!("{p} in {files:?}"));
        assert_eq!(get("README.md").status, FileStatus::Modified);
        assert_eq!((get("README.md").insertions, get("README.md").deletions), (Some(1), Some(0)));
        let ren = get("renamed.txt");
        assert_eq!((ren.status, ren.old_path.as_deref()), (FileStatus::Renamed, Some("old.txt")));
        assert_eq!(get("added.txt").status, FileStatus::Added);
        assert_eq!(get("added.txt").size_bytes, Some(4));
        let bin = get("logo.bin");
        assert!(bin.binary && bin.insertions.is_none() && bin.size_bytes == Some(6));

        let files = r.commit_files(&del).unwrap();
        let gone = files.iter().find(|f| f.path == "added.txt").unwrap();
        assert_eq!((gone.status, gone.size_bytes), (FileStatus::Deleted, None));

        let root_files = r.commit_files(&git(&s.work, &["rev-list", "--max-parents=0", "HEAD"])).unwrap();
        assert_eq!(root_files[0].status, FileStatus::Added);

        assert!(r.commit_files("xyz").unwrap_err().contains("inválido"));
        assert!(r.commit_files("HEAD~1").is_err());
        assert!(r.commit_files("0123456789abcdef").unwrap_err().contains("não existe"));
    }

    #[test]
    fn other_branch_is_checked_and_pushed_without_switching() {
        let s = setup();
        git(&s.work, &["checkout", "-q", "-b", "feature"]);
        let f = commit(&s.work, "f.txt", "f\n", "feature work");
        git(&s.work, &["checkout", "-q", "main"]);
        std::fs::write(s.work.join("README.md"), "dirty\n").unwrap();
        let r = repo(&s.work);
        let limits = FileLimits::default();
        let st = state_of(&r, &rules(), limits, Some("feature")).unwrap();
        assert_eq!(st.branch.as_deref(), Some("feature"));
        assert_eq!((st.ahead, st.upstream.as_deref(), st.head_sha.as_deref()), (1, None, Some(f.as_str())));
        assert_eq!(st.commits[0].subject, "feature work");
        assert!(!codes(&st).contains(&CheckCode::DirtyWorktree) && st.dirty == DirtyCounts::default());
        assert!(st.can_push && !st.can_fix_authors && !st.can_pull);
        assert_eq!(push_refusal(&st, &f, false), None);
        let cur = state_of(&r, &rules(), limits, Some("main")).unwrap();
        assert!(codes(&cur).contains(&CheckCode::DirtyWorktree));
        assert!(state_of(&r, &rules(), limits, Some("nope")).unwrap_err().contains("não existe"));

        let out = push_branch(&r, "origin", "feature", None, |_, _| {}).unwrap();
        assert!(out.created_upstream && out.remote_branch == "feature");
        assert_eq!(git(&s.bare, &["rev-parse", "refs/heads/feature"]), f);
        assert_eq!(git(&s.work, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]), "origin/feature");
        assert_eq!(r.branch().as_deref(), Some("main"));
        assert_eq!(std::fs::read_to_string(s.work.join("README.md")).unwrap(), "dirty\n");
        let st = state_of(&r, &rules(), limits, Some("feature")).unwrap();
        assert_eq!((st.ahead, st.upstream.as_deref()), (0, Some("origin/feature")));

        let other = s.root.join("other");
        git(&s.root, &["clone", "-q", "-b", "feature", &format!("file://{}", s.bare.display()), other.to_str().unwrap()]);
        set_identity(&other, "Ana", "ana@example.com");
        commit(&other, "g.txt", "g\n", "remote change");
        git(&other, &["push", "-q", "origin", "feature"]);
        fetch(&r, "origin", None).unwrap();
        let st = state_of(&r, &rules(), limits, Some("feature")).unwrap();
        assert_eq!((st.ahead, st.behind, st.can_pull), (0, 1, false));
    }

    #[test]
    fn branch_urls_are_escaped() {
        assert_eq!(github_branch_url("o", "r", "feat/x"), "https://github.com/o/r/tree/feat/x");
        assert_eq!(github_branch_url("o", "r", "a#b ç"), "https://github.com/o/r/tree/a%23b%20%C3%A7");
    }
}
