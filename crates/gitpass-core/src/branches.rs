//! Branches of a bound repository and "levar commits para outro branch": a merge of a whole branch or a
//! cherry-pick of chosen commits, with a preview that changes nothing and an apply that either finishes or
//! leaves everything as it was.
//!
//! When the target branch is not checked out in any folder, the result is built with `git merge-tree
//! --write-tree` + `git commit-tree` and the branch moves with a single `git update-ref <new> <old>`, so no
//! folder is touched. When it is checked out (here or in another `git worktree`), `git merge`/`git
//! cherry-pick` run in that folder and are aborted on any failure.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::{Deserialize, Serialize};

use crate::git_auth::spawn_error;
use crate::outgoing::{
    friendly_fetch_error, has, last_line, parse_numstat_z, plural, sanitize_output, valid_branch_name,
    valid_remote_name, CheckCode, CheckSeverity, GitNetError, GitRepo, Identity, IdentityRules, OutgoingCommit,
    PushCheck, MAX_COMMITS,
};

/// Most SHAs accepted in a cherry-pick selection.
pub const MAX_SELECTED: usize = 1000;

/// [`merge_apply`] needs new commits but got no identity for the committer.
pub const NO_IDENTITY: &str = "A conta não tem nome e email de commit. Escolha um email em «Contas» e tente de novo.";

const IDENTITY_VARS: [&str; 6] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_AUTHOR_DATE",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_COMMITTER_DATE",
];

// --- JSON types (gitpass-desktop/src/lib/api-repos.ts) ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub current: bool,
    pub is_default: bool,
    pub upstream: Option<String>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub worktree_path: Option<String>,
    pub head_sha: String,
    pub last_commit_at: String,
    pub last_commit_subject: String,
    pub remote_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMode {
    Merge,
    CherryPick,
}

impl MergeMode {
    pub const ALL: [MergeMode; 2] = [MergeMode::Merge, MergeMode::CherryPick];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    FastForward,
    NewCommits,
    UpToDate,
}

impl MergeStrategy {
    pub const ALL: [MergeStrategy; 3] = [MergeStrategy::FastForward, MergeStrategy::NewCommits, MergeStrategy::UpToDate];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergePreview {
    pub binding_id: String,
    pub source: String,
    pub target: String,
    pub mode: MergeMode,
    pub commits: Vec<OutgoingCommit>,
    pub selected: Vec<String>,
    pub strategy: MergeStrategy,
    pub conflicts: Vec<String>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub checks: Vec<PushCheck>,
    pub can_apply: bool,
    pub source_sha: String,
    pub target_sha: String,
    pub applies_in_worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergeResult {
    pub target: String,
    pub new_target_sha: String,
    pub strategy: MergeStrategy,
    pub created: usize,
    pub ahead_after: usize,
}

/// What the app knows about the binding besides the repository itself.
#[derive(Debug, Clone)]
pub struct MergeContext<'a> {
    pub binding_id: &'a str,
    /// Remote of the binding (`None`: no GitHub remote).
    pub remote: Option<&'a str>,
    pub identity: &'a IdentityRules,
}

/// Bring `source` into `target`. `commits` only counts for `cherry_pick` (empty = every commit).
#[derive(Debug, Clone, Copy)]
pub struct MergeRequest<'a> {
    pub source: &'a str,
    pub target: &'a str,
    pub mode: MergeMode,
    pub commits: &'a [String],
}

// --- Names, refs and worktrees ---

/// `git check-ref-format --branch`, also refusing what Git would expand (`@{-1}`) or read as an option.
pub fn check_branch_name(repo: &GitRepo, name: &str) -> Result<(), String> {
    let invalid = || "Nome de branch inválido.".to_string();
    if name.len() > 255 || name.contains("@{") || !valid_branch_name(name) {
        return Err(invalid());
    }
    match repo.value(&["check-ref-format", "--branch", name]) {
        Some(out) if out == name => Ok(()),
        _ => Err(invalid()),
    }
}

fn branch_sha(repo: &GitRepo, name: &str) -> Option<String> {
    repo.value(&["rev-parse", "-q", "--verify", &format!("refs/heads/{name}^{{commit}}")])
}

fn missing_branch(name: &str) -> String {
    format!("O branch {name} não existe nesta pasta. Atualize a lista de branches e tente de novo.")
}

fn is_ancestor(repo: &GitRepo, older: &str, newer: &str) -> bool {
    repo.output(&["merge-base", "--is-ancestor", older, newer]).is_ok_and(|o| o.status.success())
}

fn git_path_exists(repo: &GitRepo, name: &str) -> bool {
    repo.value(&["rev-parse", "--git-path", name]).is_some_and(|p| repo.root().join(p.trim()).exists())
}

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// One entry of `git worktree list --porcelain -z` (bare and prunable entries are left out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
}

fn parse_worktrees(text: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    let mut cur: Option<Worktree> = None;
    let mut skip = false;
    let mut flush = |cur: &mut Option<Worktree>, skip: &mut bool| {
        if let Some(w) = cur.take() {
            if !*skip {
                out.push(w);
            }
        }
        *skip = false;
    };
    for field in text.split('\0') {
        if field.is_empty() {
            flush(&mut cur, &mut skip);
        } else if let Some(p) = field.strip_prefix("worktree ") {
            flush(&mut cur, &mut skip);
            cur = Some(Worktree { path: PathBuf::from(p), branch: None });
        } else if let Some(w) = cur.as_mut() {
            if let Some(b) = field.strip_prefix("branch ") {
                w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if field == "bare" || field.starts_with("prunable") {
                skip = true;
            }
        }
    }
    flush(&mut cur, &mut skip);
    out
}

pub fn worktrees(repo: &GitRepo) -> Vec<Worktree> {
    match repo.output(&["worktree", "list", "--porcelain", "-z"]) {
        Ok(o) if o.status.success() => parse_worktrees(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// Where a branch is checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    /// In the bound folder.
    Here,
    /// In another `git worktree`.
    Elsewhere(PathBuf),
    Nowhere,
}

impl Place {
    fn repo(&self, base: &GitRepo) -> Option<GitRepo> {
        match self {
            Place::Here => Some(base.clone()),
            Place::Elsewhere(p) => Some(base.at(p)),
            Place::Nowhere => None,
        }
    }

    fn folder(&self) -> String {
        match self {
            Place::Elsewhere(p) => format!("na pasta {}", p.display()),
            _ => "nesta pasta".to_string(),
        }
    }
}

fn place_of(repo: &GitRepo, branch: &str) -> Place {
    if repo.branch().as_deref() == Some(branch) {
        return Place::Here;
    }
    let root = canonical(repo.root());
    for w in worktrees(repo) {
        if w.branch.as_deref() == Some(branch) && w.path.is_dir() {
            return if canonical(&w.path) == root { Place::Here } else { Place::Elsewhere(w.path) };
        }
    }
    Place::Nowhere
}

// --- Listing ---

const REF_FORMAT: &str = "--format=%(refname)%1f%(objectname)%1f%(upstream)%1f%(upstream:track,nobracket)%1f%(committerdate:iso-strict)%1f%(committerdate:unix)%1f%(contents:subject)";

struct RefLine {
    refname: String,
    sha: String,
    upstream: String,
    track: String,
    date: String,
    unix: i64,
    subject: String,
}

fn for_each_ref(repo: &GitRepo, prefix: &str) -> Result<Vec<RefLine>, String> {
    let out = repo.output(&["for-each-ref", REF_FORMAT, prefix])?;
    if !out.status.success() {
        return Err(format!(
            "Não foi possível ler os branches ({}).",
            last_line(&String::from_utf8_lossy(&out.stderr))
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(7, '\x1f').collect();
            (f.len() == 7).then(|| RefLine {
                refname: f[0].to_string(),
                sha: f[1].to_string(),
                upstream: f[2].to_string(),
                track: f[3].to_string(),
                date: f[4].to_string(),
                unix: f[5].parse().unwrap_or(0),
                subject: f[6].to_string(),
            })
        })
        .collect())
}

/// `%(upstream:track,nobracket)`: `ahead 2, behind 1`, `ahead 2`, `behind 1`, `` (in sync) or `gone`.
fn parse_track(track: &str) -> Option<(usize, usize)> {
    if track == "gone" {
        return None;
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in track.split(',').map(str::trim) {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().ok()?;
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().ok()?;
        }
    }
    Some((ahead, behind))
}

fn short_ref(r: &str) -> String {
    r.strip_prefix("refs/remotes/").or_else(|| r.strip_prefix("refs/heads/")).unwrap_or(r).to_string()
}

pub fn has_remote_head(repo: &GitRepo, remote: &str) -> bool {
    repo.value(&["symbolic-ref", "-q", &format!("refs/remotes/{remote}/HEAD")]).is_some()
}

/// GitHub's default branch: `refs/remotes/<remote>/HEAD`, else `main` or `master` when it exists.
pub fn default_branch(repo: &GitRepo, remote: Option<&str>) -> Option<String> {
    let remote = remote.filter(|r| valid_remote_name(r));
    if let Some(r) = remote {
        let prefix = format!("refs/remotes/{r}/");
        if let Some(b) = repo
            .value(&["symbolic-ref", "-q", &format!("{prefix}HEAD")])
            .and_then(|t| t.strip_prefix(&prefix).map(String::from))
        {
            return Some(b);
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|c| {
            branch_sha(repo, c).is_some()
                || remote.is_some_and(|r| repo.value(&["rev-parse", "-q", "--verify", &format!("refs/remotes/{r}/{c}")]).is_some())
        })
        .map(String::from)
}

/// `git remote set-head <remote> --auto` with `token` (asks GitHub which branch is the default).
pub fn set_head_auto(repo: &GitRepo, remote: &str, token: Option<&str>) -> Result<(), GitNetError> {
    if !valid_remote_name(remote) {
        return Err(GitNetError { message: "Nome de remoto inválido.".into(), auth: false });
    }
    let out = repo
        .command(token)
        .args(["remote", "set-head", remote, "--auto"])
        .output()
        .map_err(|e| GitNetError { message: spawn_error(e), auth: false })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(friendly_fetch_error(&sanitize_output(&String::from_utf8_lossy(&out.stderr), token)))
    }
}

/// Local branches plus the ones only on the bound remote (as of the last fetch). Default first, then the
/// current one, then by date of the last commit.
pub fn list_branches(repo: &GitRepo, remote: Option<&str>) -> Result<Vec<BranchInfo>, String> {
    if !repo.root().is_dir() {
        return Err(format!(
            "A pasta {} não existe mais. Desvincule-a ou clone de novo.",
            repo.root().display()
        ));
    }
    let remote = remote.filter(|r| valid_remote_name(r));
    let current = repo.branch();
    let root = canonical(repo.root());
    let open: Vec<Worktree> = worktrees(repo)
        .into_iter()
        .filter(|w| w.path.is_dir() && canonical(&w.path) != root)
        .collect();

    let mut rows: Vec<(i64, BranchInfo)> = Vec::new();
    let mut tracked: HashSet<String> = HashSet::new();
    for r in for_each_ref(repo, "refs/heads/")? {
        let Some(name) = r.refname.strip_prefix("refs/heads/") else {
            continue;
        };
        let counts = (!r.upstream.is_empty()).then(|| parse_track(&r.track)).flatten();
        tracked.insert(r.refname.clone());
        tracked.insert(r.upstream.clone());
        let is_current = current.as_deref() == Some(name);
        rows.push((
            r.unix,
            BranchInfo {
                name: name.to_string(),
                current: is_current,
                is_default: false,
                upstream: (!r.upstream.is_empty()).then(|| short_ref(&r.upstream)),
                ahead: counts.map(|c| c.0),
                behind: counts.map(|c| c.1),
                worktree_path: if is_current {
                    None
                } else {
                    open.iter()
                        .find(|w| w.branch.as_deref() == Some(name))
                        .map(|w| w.path.display().to_string())
                },
                head_sha: r.sha,
                last_commit_at: r.date,
                last_commit_subject: r.subject,
                remote_only: false,
            },
        ));
    }
    if let Some(remote) = remote {
        let prefix = format!("refs/remotes/{remote}/");
        for r in for_each_ref(repo, &prefix)? {
            let Some(name) = r.refname.strip_prefix(&prefix) else {
                continue;
            };
            if name == "HEAD" || tracked.contains(&r.refname) || tracked.contains(&format!("refs/heads/{name}")) {
                continue;
            }
            rows.push((
                r.unix,
                BranchInfo {
                    name: name.to_string(),
                    current: false,
                    is_default: false,
                    upstream: Some(format!("{remote}/{name}")),
                    ahead: None,
                    behind: None,
                    worktree_path: None,
                    head_sha: r.sha,
                    last_commit_at: r.date,
                    last_commit_subject: r.subject,
                    remote_only: true,
                },
            ));
        }
    }
    let default = default_branch(repo, remote);
    for (_, b) in rows.iter_mut() {
        b.is_default = default.as_deref() == Some(b.name.as_str());
    }
    rows.sort_by(|(ta, a), (tb, b)| {
        (!a.is_default, !a.current, std::cmp::Reverse(*ta), a.remote_only, &a.name).cmp(&(
            !b.is_default,
            !b.current,
            std::cmp::Reverse(*tb),
            b.remote_only,
            &b.name,
        ))
    });
    Ok(rows.into_iter().map(|(_, b)| b).collect())
}

// --- Checkout ---

fn friendly_checkout_error(text: &str, branch: &str) -> String {
    let lower = text.to_lowercase();
    if has(&lower, &["would be overwritten", "untracked working tree files"]) {
        return format!(
            "Alguns arquivos da pasta seriam substituídos pelos do branch {branch}. Faça um commit, guarde (stash) ou mova esses arquivos e tente de novo."
        );
    }
    if has(&lower, &["already checked out", "already used by worktree"]) {
        return format!("O branch {branch} já está aberto em outra pasta. Feche-o lá ou escolha outro branch.");
    }
    format!("Não foi possível trocar para o branch {branch} ({}).", last_line(text))
}

/// `git switch <branch>`, or `git switch -c <branch> --track <remote>/<branch>` for a branch only on GitHub.
/// Refuses with staged or unstaged changes (untracked files are fine when Git accepts them).
pub fn checkout(repo: &GitRepo, remote: Option<&str>, branch: &str) -> Result<(), String> {
    check_branch_name(repo, branch)?;
    if repo.branch().as_deref() == Some(branch) {
        return Ok(());
    }
    if repo.operation_in_progress() {
        return Err("Há um merge, rebase ou cherry-pick em andamento nesta pasta. Termine ou cancele e tente de novo.".into());
    }
    if !repo.dirty_counts()?.tracked_clean() {
        return Err(
            "Há alterações que ainda não viraram commit. Faça um commit ou guarde as alterações (stash) antes de trocar de branch."
                .into(),
        );
    }
    let out = if branch_sha(repo, branch).is_some() {
        if let Place::Elsewhere(p) = place_of(repo, branch) {
            return Err(format!(
                "O branch {branch} já está aberto em outra pasta ({}). Feche-o lá ou escolha outro branch.",
                p.display()
            ));
        }
        repo.output(&["switch", "-q", "--no-guess", branch])?
    } else {
        let remote = remote.filter(|r| valid_remote_name(r)).ok_or_else(|| missing_branch(branch))?;
        if repo
            .value(&["rev-parse", "-q", "--verify", &format!("refs/remotes/{remote}/{branch}^{{commit}}")])
            .is_none()
        {
            return Err(format!(
                "O branch {branch} não existe nesta pasta nem no GitHub (desde a última atualização). Atualize a lista e tente de novo."
            ));
        }
        repo.output(&["switch", "-q", "-c", branch, "--track", &format!("{remote}/{branch}")])?
    };
    if out.status.success() && repo.branch().as_deref() == Some(branch) {
        return Ok(());
    }
    Err(friendly_checkout_error(&String::from_utf8_lossy(&out.stderr), branch))
}

// --- Simulated merges ---

/// Result of `git merge-tree --write-tree`: the merged tree (with conflict markers when `conflicts` is not
/// empty) and the conflicted files.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeMerge {
    tree: String,
    conflicts: Vec<String>,
}

/// `<tree>\0<file>\0…\0\0<messages>`; messages are `<n>\0<path>…\0<type>\0<text>\0`.
fn parse_merge_tree_z(text: &str, conflicted: bool) -> TreeMerge {
    let mut it = text.split('\0');
    let tree = it.next().unwrap_or("").trim().to_string();
    let mut conflicts: Vec<String> = Vec::new();
    let push = |c: &mut Vec<String>, f: &str| {
        if !c.iter().any(|x| x == f) {
            c.push(f.to_string());
        }
    };
    for f in it.by_ref() {
        if f.is_empty() {
            break;
        }
        push(&mut conflicts, f);
    }
    if !conflicted {
        return TreeMerge { tree, conflicts: Vec::new() };
    }
    if conflicts.is_empty() {
        while let Some(n) = it.next().and_then(|n| n.parse::<usize>().ok()) {
            for _ in 0..n {
                if let Some(p) = it.next() {
                    push(&mut conflicts, p);
                }
            }
            let (kind, _text) = (it.next(), it.next());
            if kind.is_none() {
                break;
            }
        }
    }
    if conflicts.is_empty() {
        conflicts.push("(arquivo não identificado)".to_string());
    }
    TreeMerge { tree, conflicts }
}

fn merge_trees(repo: &GitRepo, base: Option<&str>, ours: &str, theirs: &str) -> Result<TreeMerge, String> {
    let base_arg = base.map(|b| format!("--merge-base={b}"));
    let mut args = vec!["merge-tree", "--write-tree", "--name-only", "--messages", "-z"];
    if let Some(b) = &base_arg {
        args.push(b);
    }
    args.extend([ours, theirs]);
    let out = repo.output(&args)?;
    match out.status.code() {
        Some(code @ (0 | 1)) => Ok(parse_merge_tree_z(&String::from_utf8_lossy(&out.stdout), code == 1)),
        _ => {
            let err = String::from_utf8_lossy(&out.stderr);
            if has(&err.to_lowercase(), &["usage: git merge-tree", "unknown option", "unrecognized argument", "is a tree, not a commit", "not a commit"]) {
                Err("A versão do Git instalada é antiga demais para esta função. Atualize o Git (por exemplo, pelo Homebrew) e tente de novo.".into())
            } else {
                Err(format!("Não foi possível simular a junção dos branches ({}).", last_line(&err)))
            }
        }
    }
}

/// First parent of `sha`, or the empty tree for a root commit.
fn parent_or_empty(repo: &GitRepo, sha: &str) -> Result<String, String> {
    repo.value(&["rev-parse", "-q", "--verify", &format!("{sha}^1")])
        .or_else(|| repo.value(&["hash-object", "-t", "tree", "/dev/null"]))
        .ok_or_else(|| "Não foi possível ler o commit anterior.".to_string())
}

/// Commits of `head` with an equivalent change (same patch id) already in `upstream` (`git cherry`).
fn equivalent_commits(repo: &GitRepo, upstream: &str, head: &str) -> HashSet<String> {
    repo.value(&["cherry", upstream, head])
        .map(|t| t.lines().filter_map(|l| l.strip_prefix("- ")).map(|s| s.trim().to_string()).collect())
        .unwrap_or_default()
}

fn diff_stats(repo: &GitRepo, spec: &[&str]) -> (usize, usize, usize) {
    let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--numstat", "-z", "-M"];
    args.extend_from_slice(spec);
    args.push("--");
    let Some(text) = repo
        .output(&args)
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    else {
        return (0, 0, 0);
    };
    let stats = parse_numstat_z(&text);
    let ins = stats.iter().filter_map(|(_, s)| s.0).sum();
    let del = stats.iter().filter_map(|(_, s)| s.1).sum();
    (stats.len(), ins, del)
}

/// Commits of `branch` not on the remote yet (against its upstream, else against every branch of `remote`).
pub fn ahead_of_remote(repo: &GitRepo, remote: Option<&str>, branch: &str) -> usize {
    let tip = format!("refs/heads/{branch}");
    let range: Vec<String> = match remote.filter(|r| valid_remote_name(r)) {
        Some(r) => match repo.push_target(r, branch).tracking {
            Some(t) => vec![format!("{t}..{tip}")],
            None => vec![tip, "--not".into(), format!("--remotes={r}")],
        },
        None => vec![tip, "--not".into(), "--remotes".into()],
    };
    repo.count(&[], &range).unwrap_or(0)
}

// --- Preview ---

/// The preview plus what apply needs to build the same result.
struct Plan {
    preview: MergePreview,
    /// Merge with a new commit: the merged tree.
    merge_tree: Option<String>,
    /// Cherry-pick: `(original commit, resulting tree)`, oldest first, without those already in the target.
    picks: Vec<(String, String)>,
    place: Place,
}

fn account_name(identity: &IdentityRules) -> String {
    match identity.login.as_deref() {
        Some(l) => format!("a conta @{l}"),
        None => "a conta vinculada".to_string(),
    }
}

fn plan(repo: &GitRepo, ctx: &MergeContext, req: &MergeRequest) -> Result<Plan, String> {
    use CheckCode as C;
    use CheckSeverity::{Error, Info, Warning};
    let (source, target) = (req.source, req.target);

    if !repo.root().is_dir() {
        return Err(format!(
            "A pasta {} não existe mais. Desvincule-a ou clone de novo.",
            repo.root().display()
        ));
    }
    check_branch_name(repo, source)?;
    check_branch_name(repo, target)?;
    if source == target {
        return Err("Escolha dois branches diferentes: um de origem e um de destino.".into());
    }
    if req.commits.len() > MAX_SELECTED {
        return Err(format!("Escolha no máximo {MAX_SELECTED} commits de cada vez."));
    }
    let source_sha = branch_sha(repo, source).ok_or_else(|| missing_branch(source))?;
    let target_sha = branch_sha(repo, target).ok_or_else(|| missing_branch(target))?;
    let remote = ctx.remote.filter(|r| valid_remote_name(r));
    let range = vec![format!("{target_sha}..{source_sha}")];
    let newest_first = repo.rev_list(&["--topo-order".to_string(), range[0].clone()]);
    let mut commits = repo.commits(&range, MAX_COMMITS)?;

    let mut checks: Vec<PushCheck> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    let mut merge_tree: Option<String> = None;
    let mut picks: Vec<(String, String)> = Vec::new();
    let mut selected: Vec<String> = Vec::new();
    let nothing = || {
        PushCheck::new(
            C::AlreadyInTarget,
            Info,
            format!("Os commits de {source} já estão em {target}. Não há nada para levar."),
        )
    };

    let (strategy, stats, brought) = match req.mode {
        MergeMode::Merge => {
            if newest_first.is_empty() {
                checks.push(nothing());
                (MergeStrategy::UpToDate, (0, 0, 0), Vec::new())
            } else if is_ancestor(repo, &target_sha, &source_sha) {
                (MergeStrategy::FastForward, diff_stats(repo, &[&target_sha, &source_sha]), newest_first.clone())
            } else {
                let m = merge_trees(repo, None, &target_sha, &source_sha)?;
                let stats = if m.conflicts.is_empty() {
                    let s = diff_stats(repo, &[&target_sha, &m.tree]);
                    merge_tree = Some(m.tree);
                    s
                } else {
                    conflicts = m.conflicts;
                    diff_stats(repo, &[&format!("{target_sha}...{source_sha}")])
                };
                (MergeStrategy::NewCommits, stats, newest_first.clone())
            }
        }
        MergeMode::CherryPick => {
            let in_range: HashSet<&str> = newest_first.iter().map(String::as_str).collect();
            let wanted: Option<HashSet<String>> = if req.commits.is_empty() {
                None
            } else {
                let mut set = HashSet::new();
                for c in req.commits {
                    let full = repo.resolve_commit(c.trim())?;
                    if !in_range.contains(full.as_str()) {
                        return Err(format!(
                            "O commit {} não está em {source}, ou já está em {target}. Atualize a prévia e tente de novo.",
                            &full[..7.min(full.len())]
                        ));
                    }
                    set.insert(full);
                }
                Some(set)
            };
            let chosen: Vec<&String> =
                newest_first.iter().rev().filter(|s| wanted.as_ref().is_none_or(|w| w.contains(*s))).collect();
            let merges: HashSet<String> = repo.rev_list(&["--min-parents=2".to_string(), range[0].clone()]).into_iter().collect();
            let equivalent = equivalent_commits(repo, &target_sha, &source_sha);
            let start = repo
                .value(&["rev-parse", "-q", "--verify", &format!("{target_sha}^{{tree}}")])
                .ok_or_else(|| "Não foi possível ler o branch de destino.".to_string())?;
            let mut tree = start.clone();
            let mut conflict_at: Option<String> = None;
            for c in chosen {
                if merges.contains(c) {
                    checks.push(
                        PushCheck::new(
                            C::MergeCommit,
                            Error,
                            "Este é um commit de merge. Não dá para copiar só ele: escolha «Juntar o branch inteiro» ou tire este commit da seleção.",
                        )
                        .sha(Some(c)),
                    );
                    selected.push(c.clone());
                    continue;
                }
                if equivalent.contains(c) {
                    checks.push(
                        PushCheck::new(C::AlreadyInTarget, Info, format!("Este commit já está em {target} (como cópia). Ele vai ser pulado."))
                            .sha(Some(c)),
                    );
                    continue;
                }
                if conflict_at.is_some() {
                    selected.push(c.clone());
                    continue;
                }
                let base = parent_or_empty(repo, c)?;
                let m = merge_trees(repo, Some(&base), &tree, c)?;
                if !m.conflicts.is_empty() {
                    conflicts = m.conflicts;
                    conflict_at = Some(c.clone());
                    selected.push(c.clone());
                    continue;
                }
                if m.tree == tree {
                    checks.push(
                        PushCheck::new(
                            C::AlreadyInTarget,
                            Info,
                            format!("As mudanças deste commit já estão em {target}. Ele vai ser pulado."),
                        )
                        .sha(Some(c)),
                    );
                    continue;
                }
                tree = m.tree.clone();
                selected.push(c.clone());
                picks.push((c.clone(), m.tree));
            }
            if newest_first.is_empty() {
                checks.push(nothing());
            }
            let stats = if conflicts.is_empty() {
                diff_stats(repo, &[&start, &tree])
            } else {
                commits.iter().filter(|c| selected.contains(&c.sha)).fold((0, 0, 0), |(f, i, d), c| {
                    (f + c.files_changed, i + c.insertions, d + c.deletions)
                })
            };
            let strategy = if selected.is_empty() { MergeStrategy::UpToDate } else { MergeStrategy::NewCommits };
            if let Some(r) = remote.filter(|_| !selected.is_empty()) {
                let st = repo.push_target(r, source);
                if let Some(t) = &st.tracking {
                    let pushed: HashSet<String> = repo.rev_list(&[format!("{target_sha}..{t}")]).into_iter().collect();
                    if selected.iter().any(|s| pushed.contains(s)) {
                        checks.push(PushCheck::new(
                            C::SourcePushed,
                            Info,
                            format!(
                                "Esses commits já estão no GitHub, no branch {}. Levar com cherry-pick cria cópias deles em {target}, com outro identificador.",
                                st.remote_branch
                            ),
                        ));
                    }
                }
            }
            if let Some(sha) = &conflict_at {
                checks.push(conflict_check(&conflicts).sha(Some(sha)));
            }
            (strategy, stats, selected.clone())
        }
    };
    if req.mode == MergeMode::Merge && !conflicts.is_empty() {
        checks.push(conflict_check(&conflicts));
    }

    let place = place_of(repo, target);
    if strategy != MergeStrategy::UpToDate {
        if let Some(wt) = place.repo(repo) {
            let folder = place.folder();
            if wt.operation_in_progress() || git_path_exists(&wt, "sequencer") {
                checks.push(PushCheck::new(
                    C::TargetDirty,
                    Error,
                    format!("O branch {target} está aberto {folder} e tem um merge, rebase ou cherry-pick em andamento. Termine ou cancele e tente de novo."),
                ));
            } else if !wt.dirty_counts()?.tracked_clean() {
                checks.push(PushCheck::new(
                    C::TargetDirty,
                    Error,
                    format!("O branch {target} está aberto {folder} e tem alterações que ainda não viraram commit. Faça um commit ou guarde as alterações (stash) antes de continuar."),
                ));
            }
        }
    }
    if let Some(r) = remote {
        let t = repo.push_target(r, target);
        if let Some(tracking) = &t.tracking {
            let n = repo.count(&[], &[format!("{target_sha}..{tracking}")]).unwrap_or(0);
            if n > 0 {
                checks.push(PushCheck::new(
                    C::TargetBehindRemote,
                    Warning,
                    format!(
                        "O GitHub tem {} no branch {} que ainda não estão aqui. Traga essas mudanças para {target} antes, para não ter problemas ao enviar.",
                        plural(n, "commit", "commits"),
                        t.remote_branch
                    ),
                ));
            }
        }
    }
    if !ctx.identity.is_empty() && !brought.is_empty() {
        let account = account_name(ctx.identity);
        let wanted: HashSet<&str> = brought.iter().map(String::as_str).collect();
        for (sha, author, _) in repo.identities(&range).into_iter().filter(|(s, _, _)| wanted.contains(s.as_str())) {
            if !ctx.identity.accepts(&author) {
                checks.push(
                    PushCheck::new(
                        C::AuthorMismatch,
                        Warning,
                        format!("O autor deste commit usa o email {author}, que não é d{account}."),
                    )
                    .sha(Some(&sha)),
                );
            }
        }
    }
    for c in commits.iter_mut() {
        c.checks = checks.iter().filter(|k| k.sha.as_deref() == Some(c.sha.as_str())).cloned().collect();
    }

    let has_error = checks.iter().any(|c| c.severity == Error);
    let (files_changed, insertions, deletions) = stats;
    Ok(Plan {
        preview: MergePreview {
            binding_id: ctx.binding_id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            mode: req.mode,
            commits,
            selected: if req.mode == MergeMode::CherryPick { selected } else { Vec::new() },
            strategy,
            conflicts,
            files_changed,
            insertions,
            deletions,
            can_apply: !has_error && strategy != MergeStrategy::UpToDate,
            checks,
            source_sha,
            target_sha,
            applies_in_worktree: place != Place::Nowhere,
        },
        merge_tree,
        picks,
        place,
    })
}

fn conflict_check(conflicts: &[String]) -> PushCheck {
    let check = PushCheck::new(
        CheckCode::MergeConflict,
        CheckSeverity::Error,
        format!(
            "Os dois branches mudaram as mesmas linhas em {}. Resolva no seu editor.",
            plural(conflicts.len(), "arquivo", "arquivos")
        ),
    );
    match conflicts {
        [only] => check.path(only),
        _ => check,
    }
}

/// What bringing `req.source` into `req.target` would do; changes nothing (no index, folder or ref).
pub fn merge_preview(repo: &GitRepo, ctx: &MergeContext, req: &MergeRequest) -> Result<MergePreview, String> {
    plan(repo, ctx, req).map(|p| p.preview)
}

// --- Apply ---

fn merge_message(req: &MergeRequest) -> String {
    format!("Merge branch '{}' into {}", req.source, req.target)
}

/// Message plus the `(cherry picked from commit …)` line, placed like `git cherry-pick -x` does.
fn with_cherry_line(message: &[u8], sha: &str) -> Vec<u8> {
    let mut m = message.to_vec();
    while m.last().is_some_and(|b| b.is_ascii_whitespace()) {
        m.pop();
    }
    let trailers = ends_with_trailers(&String::from_utf8_lossy(&m));
    if !m.is_empty() {
        m.push(b'\n');
        if !trailers {
            m.push(b'\n');
        }
    }
    m.extend(format!("(cherry picked from commit {sha})\n").bytes());
    m
}

/// The last paragraph (not the subject) is a block of `Key: value` trailers.
fn ends_with_trailers(message: &str) -> bool {
    let paragraphs: Vec<&str> = message.split("\n\n").filter(|p| !p.trim().is_empty()).collect();
    let [_, .., last] = paragraphs.as_slice() else {
        return false;
    };
    last.lines().filter(|l| !l.trim().is_empty()).all(|l| {
        l.starts_with("(cherry picked from commit ")
            || l.split_once(':').is_some_and(|(k, v)| {
                !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && v.starts_with(' ')
            })
    })
}

struct Original {
    author_name: String,
    author_email: String,
    /// `<unix> <tz>`, as stored in the commit.
    author_date: String,
    encoding: Option<String>,
    message: Vec<u8>,
}

/// `Name <email> 1700000000 -0300` → `(name, email, date)`.
fn parse_person(s: &str) -> Option<(String, String, String)> {
    let lt = s.find('<')?;
    let gt = lt + s[lt..].find('>')?;
    Some((s[..lt].trim().to_string(), s[lt + 1..gt].to_string(), s[gt + 1..].trim().to_string()))
}

fn read_commit(repo: &GitRepo, sha: &str) -> Result<Original, String> {
    let out = repo.output(&["cat-file", "commit", sha])?;
    if !out.status.success() {
        return Err("Não foi possível ler um dos commits. Nada foi alterado.".into());
    }
    let raw = out.stdout;
    let (head, message) = match raw.windows(2).position(|w| w == b"\n\n") {
        Some(i) => (&raw[..i], raw[i + 2..].to_vec()),
        None => (&raw[..], Vec::new()),
    };
    let head = String::from_utf8_lossy(head);
    let mut author = None;
    let mut encoding = None;
    for line in head.lines() {
        if let Some(a) = line.strip_prefix("author ") {
            author = parse_person(a);
        } else if let Some(e) = line.strip_prefix("encoding ") {
            encoding = Some(e.trim().to_string());
        }
    }
    let (author_name, author_email, author_date) =
        author.ok_or_else(|| "Não foi possível ler o autor de um dos commits. Nada foi alterado.".to_string())?;
    Ok(Original { author_name, author_email, author_date, encoding, message })
}

fn run_with_input(mut cmd: Command, input: &[u8]) -> Result<Output, String> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(spawn_error)?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    child.wait_with_output().map_err(spawn_error)
}

/// `git commit-tree`; `author = None` means the committer, now.
fn commit_tree(
    repo: &GitRepo,
    tree: &str,
    parents: &[&str],
    message: &[u8],
    author: Option<&Original>,
    committer: &Identity,
) -> Result<String, String> {
    let mut cmd = repo.command(None);
    if let Some(e) = author.and_then(|a| a.encoding.as_deref()) {
        cmd.args(["-c", &format!("i18n.commitEncoding={e}")]);
    }
    cmd.args(["commit-tree", tree]);
    for p in parents {
        cmd.args(["-p", p]);
    }
    cmd.args(["-F", "-"]);
    for var in IDENTITY_VARS {
        cmd.env_remove(var);
    }
    let (name, email) = (committer.name.trim(), committer.email.trim());
    cmd.env("GIT_COMMITTER_NAME", name).env("GIT_COMMITTER_EMAIL", email);
    match author {
        Some(a) => cmd
            .env("GIT_AUTHOR_NAME", &a.author_name)
            .env("GIT_AUTHOR_EMAIL", &a.author_email)
            .env("GIT_AUTHOR_DATE", &a.author_date),
        None => cmd.env("GIT_AUTHOR_NAME", name).env("GIT_AUTHOR_EMAIL", email),
    };
    let out = run_with_input(cmd, message)?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && crate::outgoing::is_hex_sha(&sha) {
        Ok(sha)
    } else {
        Err(format!(
            "Não foi possível criar o commit ({}). Nada foi alterado.",
            last_line(&String::from_utf8_lossy(&out.stderr))
        ))
    }
}

/// Builds the result without any folder and moves the branch with `update-ref <new> <old>` (atomic).
fn apply_detached(repo: &GitRepo, req: &MergeRequest, plan: &Plan, who: Option<&Identity>) -> Result<String, String> {
    let p = &plan.preview;
    let need_who = || who.ok_or_else(|| NO_IDENTITY.to_string());
    let new = match (p.strategy, req.mode) {
        (MergeStrategy::FastForward, _) => p.source_sha.clone(),
        (_, MergeMode::Merge) => {
            let tree = plan.merge_tree.as_deref().ok_or_else(|| "Algo deu errado ao preparar o merge. Nada foi alterado.".to_string())?;
            commit_tree(repo, tree, &[&p.target_sha, &p.source_sha], merge_message(req).as_bytes(), None, need_who()?)?
        }
        (_, MergeMode::CherryPick) => {
            let who = need_who()?;
            let mut parent = p.target_sha.clone();
            for (c, tree) in &plan.picks {
                let original = read_commit(repo, c)?;
                let message = with_cherry_line(&original.message, c);
                parent = commit_tree(repo, tree, &[&parent], &message, Some(&original), who)?;
            }
            parent
        }
    };
    let reason = match req.mode {
        MergeMode::Merge => format!("gitpass: merge {} into {}", req.source, req.target),
        MergeMode::CherryPick => format!("gitpass: cherry-pick from {} into {}", req.source, req.target),
    };
    let out = repo.output(&["update-ref", "-m", &reason, &format!("refs/heads/{}", req.target), &new, &p.target_sha])?;
    if !out.status.success() {
        return Err(format!(
            "O branch {} mudou enquanto o GitPass trabalhava. Nada foi alterado. Confira a prévia de novo.",
            req.target
        ));
    }
    Ok(new)
}

/// Puts the folder back on `branch` at `old` after a failed merge or cherry-pick (its tracked files were clean).
fn restore_worktree(wt: &GitRepo, branch: &str, old: &str) {
    if git_path_exists(wt, "MERGE_HEAD") {
        let _ = wt.output(&["merge", "--abort"]);
    }
    if git_path_exists(wt, "CHERRY_PICK_HEAD") || git_path_exists(wt, "sequencer") {
        let _ = wt.output(&["cherry-pick", "--abort"]);
        if git_path_exists(wt, "sequencer") || git_path_exists(wt, "CHERRY_PICK_HEAD") {
            let _ = wt.output(&["cherry-pick", "--quit"]);
        }
    }
    if wt.branch().as_deref() != Some(branch) {
        let _ = wt.output(&["checkout", "-q", branch, "--"]);
    }
    if wt.head_sha().as_deref() != Some(old) && wt.branch().as_deref() == Some(branch) {
        let _ = wt.output(&["reset", "--hard", "-q", old]);
    }
}

fn friendly_apply_error(text: &str) -> String {
    let lower = text.to_lowercase();
    if has(&lower, &["untracked working tree files would be overwritten", "would be overwritten by merge"]) {
        return "Há arquivos na pasta que ainda não estão no Git e seriam substituídos. Mova ou apague esses arquivos e tente de novo. Nada foi alterado.".into();
    }
    if has(&lower, &["conflict", "could not apply"]) {
        return "Apareceu um conflito ao levar os commits. Nada foi alterado. Resolva no seu editor.".into();
    }
    if has(&lower, &["hook"]) {
        return format!("Uma verificação do Git bloqueou a operação: {}. Nada foi alterado.", last_line(text));
    }
    format!("Não foi possível levar os commits ({}). Nada foi alterado.", last_line(text))
}

/// `git merge` / `git cherry-pick -x` in the folder where the target is checked out.
fn apply_in_worktree(wt: &GitRepo, req: &MergeRequest, plan: &Plan, who: Option<&Identity>) -> Result<String, String> {
    let p = &plan.preview;
    let old = p.target_sha.as_str();
    if wt.branch().as_deref() != Some(req.target) || wt.head_sha().as_deref() != Some(old) {
        return Err(format!("O branch {} mudou desde a prévia. Confira a prévia de novo.", req.target));
    }
    if wt.operation_in_progress() || git_path_exists(wt, "sequencer") {
        return Err("Há um merge, rebase ou cherry-pick em andamento na pasta do branch de destino. Termine ou cancele e tente de novo.".into());
    }
    if !wt.dirty_counts()?.tracked_clean() {
        return Err(format!(
            "O branch {} tem alterações que ainda não viraram commit. Faça um commit ou guarde as alterações (stash) e tente de novo.",
            req.target
        ));
    }
    let mut cmd = wt.command(None);
    if let Some(w) = who {
        cmd.args(["-c", &format!("user.name={}", w.name.trim())])
            .args(["-c", &format!("user.email={}", w.email.trim())]);
    } else if p.strategy == MergeStrategy::NewCommits {
        return Err(NO_IDENTITY.into());
    }
    cmd.env("GIT_EDITOR", "true");
    for var in IDENTITY_VARS {
        cmd.env_remove(var);
    }
    match (p.strategy, req.mode) {
        (MergeStrategy::FastForward, _) => cmd.args(["merge", "--ff-only", "--no-edit", "-q", &p.source_sha]),
        (_, MergeMode::Merge) => cmd.args(["merge", "--no-ff", "--no-edit", "-q", "-m", &merge_message(req), &p.source_sha]),
        (_, MergeMode::CherryPick) => cmd.args(["cherry-pick", "-x"]).args(plan.picks.iter().map(|(c, _)| c.as_str())),
    };
    let out = cmd.output().map_err(spawn_error)?;
    let finished = out.status.success()
        && wt.branch().as_deref() == Some(req.target)
        && !wt.operation_in_progress()
        && !git_path_exists(wt, "sequencer");
    if finished {
        if let Some(sha) = wt.head_sha() {
            return Ok(sha);
        }
    }
    restore_worktree(wt, req.target, old);
    Err(friendly_apply_error(&format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )))
}

/// Previews again and applies only if nothing changed since the user saw it (`source_sha`/`target_sha`).
/// `who` (the bound account) becomes the committer of new commits; cherry-picked commits keep their author.
pub fn merge_apply(
    repo: &GitRepo,
    ctx: &MergeContext,
    req: &MergeRequest,
    source_sha: &str,
    target_sha: &str,
    who: Option<&Identity>,
) -> Result<MergeResult, String> {
    let plan = plan(repo, ctx, req)?;
    let p = &plan.preview;
    if p.source_sha != source_sha.trim() || p.target_sha != target_sha.trim() {
        return Err("Os branches mudaram desde a prévia. Confira a prévia de novo antes de continuar.".into());
    }
    if let Some(e) = p.checks.iter().find(|c| c.severity == CheckSeverity::Error) {
        return Err(format!("Não dá para continuar. {} Nada foi alterado.", e.message));
    }
    if p.strategy == MergeStrategy::UpToDate {
        return Err(format!("Não há nada para levar: os commits já estão em {}.", req.target));
    }
    let who = who.filter(|w| !w.name.trim().is_empty() && !w.email.trim().is_empty());
    let new_sha = match plan.place.repo(repo) {
        None => apply_detached(repo, req, &plan, who)?,
        Some(wt) => apply_in_worktree(&wt, req, &plan, who)?,
    };
    let created = match (p.strategy, req.mode) {
        (MergeStrategy::FastForward, _) => 0,
        (_, MergeMode::Merge) => 1,
        (_, MergeMode::CherryPick) => repo
            .count(&[], &[format!("{}..{new_sha}", p.target_sha)])
            .unwrap_or(plan.picks.len()),
    };
    Ok(MergeResult {
        target: req.target.to_string(),
        new_target_sha: new_sha,
        strategy: p.strategy,
        created,
        ahead_after: ahead_of_remote(repo, ctx.remote, req.target),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_auth::git_program;
    use crate::outgoing::fetch;
    use uuid::Uuid;

    struct Tmp(PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp() -> Tmp {
        let p = std::env::temp_dir().join(format!("gitpass-branches-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p.canonicalize().unwrap())
    }

    /// The user's global/system config (and their global hooks) never take part.
    fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
        let mut cmd = Command::new(git_program());
        cmd.arg("-C")
            .arg(dir)
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        git_env(dir, args, &[])
    }

    fn repo(dir: &Path) -> GitRepo {
        GitRepo::new(dir)
            .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }

    fn set_identity(dir: &Path, name: &str, email: &str) {
        git(dir, &["config", "user.name", name]);
        git(dir, &["config", "user.email", email]);
    }

    fn commit_env(dir: &Path, file: &str, content: &str, msg: &str, env: &[(&str, &str)]) -> String {
        let p = dir.join(file);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
        git(dir, &["add", "-A"]);
        git_env(dir, &["commit", "-q", "-m", msg], env);
        git(dir, &["rev-parse", "HEAD"])
    }

    fn commit(dir: &Path, file: &str, content: &str, msg: &str) -> String {
        commit_env(dir, file, content, msg, &[])
    }

    struct Setup {
        _t: Tmp,
        bare: PathBuf,
        work: PathBuf,
        root: PathBuf,
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

    /// A second clone, to change the remote behind the work folder's back.
    fn other_clone(s: &Setup) -> PathBuf {
        let other = s.root.join("other");
        if !other.exists() {
            git(&s.root, &["clone", "-q", &format!("file://{}", s.bare.display()), other.to_str().unwrap()]);
            set_identity(&other, "Ana", "ana@example.com");
        }
        other
    }

    fn rules() -> IdentityRules {
        IdentityRules {
            login: Some("ana".into()),
            user_id: Some(42),
            emails: vec!["ana@example.com".into()],
        }
    }

    fn who() -> Identity {
        Identity { name: "Ana Conta".into(), email: "ana@example.com".into() }
    }

    fn sha_list(shas: &[&str]) -> Vec<String> {
        shas.iter().map(|s| s.to_string()).collect()
    }

    fn preview_of(r: &GitRepo, source: &str, target: &str, mode: MergeMode, commits: &[String]) -> Result<MergePreview, String> {
        let ids = rules();
        let ctx = MergeContext { binding_id: "b1", remote: Some("origin"), identity: &ids };
        merge_preview(r, &ctx, &MergeRequest { source, target, mode, commits })
    }

    fn preview(r: &GitRepo, source: &str, target: &str, mode: MergeMode, commits: &[String]) -> MergePreview {
        preview_of(r, source, target, mode, commits).unwrap()
    }

    fn apply(r: &GitRepo, p: &MergePreview, commits: &[String]) -> Result<MergeResult, String> {
        let ids = rules();
        let ctx = MergeContext { binding_id: "b1", remote: Some("origin"), identity: &ids };
        let req = MergeRequest { source: &p.source, target: &p.target, mode: p.mode, commits };
        merge_apply(r, &ctx, &req, &p.source_sha, &p.target_sha, Some(&who()))
    }

    fn codes(p: &MergePreview) -> Vec<CheckCode> {
        p.checks.iter().map(|c| c.code).collect()
    }

    /// Status, refs and index bytes: what a preview must leave as they were.
    fn snapshot(dir: &Path) -> (String, String, Vec<u8>) {
        let index = std::fs::read(dir.join(".git/index")).unwrap();
        let status = git(dir, &["--no-optional-locks", "status", "--porcelain=v1", "--untracked-files=all"]);
        let refs = git(dir, &["for-each-ref", "--format=%(refname) %(objectname) %(upstream)"]);
        (status, refs, index)
    }

    #[test]
    fn parsers() {
        let wt = "worktree /a\0HEAD 1\0branch refs/heads/main\0\0worktree /b\0HEAD 2\0detached\0\0worktree /c\0HEAD 3\0branch refs/heads/x\0prunable gitdir file points to non-existent location\0\0";
        assert_eq!(
            parse_worktrees(wt),
            [
                Worktree { path: "/a".into(), branch: Some("main".into()) },
                Worktree { path: "/b".into(), branch: None }
            ]
        );
        assert_eq!(parse_track(""), Some((0, 0)));
        assert_eq!(parse_track("ahead 2, behind 1"), Some((2, 1)));
        assert_eq!(parse_track("behind 3"), Some((0, 3)));
        assert_eq!(parse_track("gone"), None);

        let m = parse_merge_tree_z("abc\0f.txt\0g h.txt\0f.txt\0\x001\0f.txt\0Auto-merging\0Auto-merging f.txt\n\0", true);
        assert_eq!(m, TreeMerge { tree: "abc".into(), conflicts: sha_list(&["f.txt", "g h.txt"]) });
        assert_eq!(parse_merge_tree_z("abc\0\0", false).conflicts, Vec::<String>::new());
        let msgs = parse_merge_tree_z("abc\0\x001\0x.txt\0CONFLICT (modify/delete)\0deleted in one side\0", true);
        assert_eq!(msgs.conflicts, ["x.txt"]);

        assert_eq!(
            String::from_utf8(with_cherry_line(b"Subject\n", "abc")).unwrap(),
            "Subject\n\n(cherry picked from commit abc)\n"
        );
        assert_eq!(
            String::from_utf8(with_cherry_line(b"Subject\n\nBody text.\n\nSigned-off-by: A <a@x>\n", "abc")).unwrap(),
            "Subject\n\nBody text.\n\nSigned-off-by: A <a@x>\n(cherry picked from commit abc)\n"
        );
        assert_eq!(
            String::from_utf8(with_cherry_line(b"Subject\n\nNot: a trailer block\nreally\n", "abc")).unwrap(),
            "Subject\n\nNot: a trailer block\nreally\n\n(cherry picked from commit abc)\n"
        );
        assert_eq!(parse_person("Ana Maria <a@x.com> 1700000000 -0300"), Some(("Ana Maria".into(), "a@x.com".into(), "1700000000 -0300".into())));
    }

    #[test]
    fn branch_names_are_validated_by_git() {
        let s = setup();
        let r = repo(&s.work);
        for ok in ["main", "feat/x", "fix-1.2", "ação"] {
            assert!(check_branch_name(&r, ok).is_ok(), "{ok}");
        }
        for bad in ["", "-x", "a..b", "@{-1}", "a b", "x.lock", "a~1", "HEAD@{1}", "/x", "x/"] {
            assert!(check_branch_name(&r, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn lists_branches_with_current_default_upstream_remote_only_and_worktrees() {
        let s = setup();
        let main = git(&s.work, &["rev-parse", "HEAD"]);
        git(&s.work, &["branch", "old"]);
        git(&s.work, &["branch", "wt"]);
        git(&s.work, &["checkout", "-q", "-b", "feature"]);
        commit(&s.work, "f.txt", "f\n", "feature work");
        let wt_dir = s.root.join("wt-dir");
        git(&s.work, &["worktree", "add", "-q", wt_dir.to_str().unwrap(), "wt"]);

        let other = other_clone(&s);
        commit(&other, "m.txt", "m\n", "remote main");
        git(&other, &["push", "-q", "origin", "main"]);
        git(&other, &["checkout", "-q", "-b", "remote-feat"]);
        let future = [("GIT_COMMITTER_DATE", "2030-01-01T00:00:00Z"), ("GIT_AUTHOR_DATE", "2030-01-01T00:00:00Z")];
        let rf = commit_env(&other, "r.txt", "r\n", "remote feature", &future);
        git(&other, &["push", "-q", "origin", "remote-feat"]);

        let r = repo(&s.work);
        fetch(&r, "origin", None).unwrap();
        let list = list_branches(&r, Some("origin")).unwrap();
        let names: Vec<&str> = list.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(&names[..3], &["main", "feature", "remote-feat"], "{names:?}");
        assert_eq!(names.len(), 5);
        let get = |n: &str| list.iter().find(|b| b.name == n).unwrap();

        let m = get("main");
        assert!(m.is_default && !m.current && !m.remote_only);
        assert_eq!((m.upstream.as_deref(), m.ahead, m.behind), (Some("origin/main"), Some(0), Some(1)));
        assert_eq!((m.head_sha.as_str(), m.last_commit_subject.as_str()), (main.as_str(), "init"));
        assert!(m.last_commit_at.contains('T'), "{}", m.last_commit_at);
        let f = get("feature");
        assert!(f.current && !f.is_default && f.worktree_path.is_none());
        assert_eq!((f.upstream.as_deref(), f.ahead, f.behind), (None, None, None));
        let remote_only = get("remote-feat");
        assert!(remote_only.remote_only && !remote_only.current);
        assert_eq!((remote_only.upstream.as_deref(), remote_only.head_sha.as_str()), (Some("origin/remote-feat"), rf.as_str()));
        assert!(remote_only.last_commit_at.starts_with("2030-01-01"));
        assert_eq!(get("wt").worktree_path.as_deref(), Some(wt_dir.to_str().unwrap()));
        assert_eq!(get("old").worktree_path, None);

        // The remote's own default branch wins over the `main` fallback.
        git(&s.bare, &["symbolic-ref", "HEAD", "refs/heads/remote-feat"]);
        set_head_auto(&r, "origin", None).unwrap();
        assert!(has_remote_head(&r, "origin"));
        let list = list_branches(&r, Some("origin")).unwrap();
        assert_eq!((list[0].name.as_str(), list[0].is_default), ("remote-feat", true));
        assert!(!list.iter().find(|b| b.name == "main").unwrap().is_default);
        git(&s.work, &["remote", "set-head", "origin", "-d"]);
        assert_eq!(default_branch(&r, Some("origin")).as_deref(), Some("main"));

        let local_only = list_branches(&r, None).unwrap();
        assert!(local_only.iter().all(|b| !b.remote_only));
        assert_eq!(local_only[0].name, "main");
    }

    #[test]
    fn checkout_switches_refuses_changes_and_tracks_remote_only_branches() {
        let s = setup();
        git(&s.work, &["branch", "side"]);
        let r = repo(&s.work);
        checkout(&r, Some("origin"), "side").unwrap();
        assert_eq!(r.branch().as_deref(), Some("side"));
        checkout(&r, Some("origin"), "side").unwrap();

        std::fs::write(s.work.join("README.md"), "dirty\n").unwrap();
        let err = checkout(&r, Some("origin"), "main").unwrap_err();
        assert!(err.contains("ainda não viraram commit"), "{err}");
        assert_eq!(r.branch().as_deref(), Some("side"));
        assert_eq!(std::fs::read_to_string(s.work.join("README.md")).unwrap(), "dirty\n");
        git(&s.work, &["checkout", "--", "README.md"]);

        std::fs::write(s.work.join("notes.txt"), "untracked\n").unwrap();
        checkout(&r, Some("origin"), "main").unwrap();
        assert_eq!(r.branch().as_deref(), Some("main"));
        assert!(s.work.join("notes.txt").exists());

        let other = other_clone(&s);
        git(&other, &["checkout", "-q", "-b", "from-github"]);
        let gh = commit(&other, "gh.txt", "gh\n", "from github");
        git(&other, &["push", "-q", "origin", "from-github"]);
        fetch(&r, "origin", None).unwrap();
        assert!(list_branches(&r, Some("origin")).unwrap().iter().any(|b| b.name == "from-github" && b.remote_only));
        checkout(&r, Some("origin"), "from-github").unwrap();
        assert_eq!(r.branch().as_deref(), Some("from-github"));
        assert_eq!(r.head_sha().as_deref(), Some(gh.as_str()));
        assert_eq!(git(&s.work, &["rev-parse", "--abbrev-ref", "from-github@{upstream}"]), "origin/from-github");
        assert!(!list_branches(&r, Some("origin")).unwrap().iter().any(|b| b.remote_only));

        assert!(checkout(&r, Some("origin"), "nope").unwrap_err().contains("não existe"));
        for bad in ["-f", "a..b", "@{-1}"] {
            assert!(checkout(&r, Some("origin"), bad).unwrap_err().contains("inválido"), "{bad}");
        }
        let wt_dir = s.root.join("wt-dir");
        git(&s.work, &["worktree", "add", "-q", wt_dir.to_str().unwrap(), "side"]);
        assert!(checkout(&r, Some("origin"), "side").unwrap_err().contains("aberto em outra pasta"));
        assert_eq!(r.branch().as_deref(), Some("from-github"));
    }

    #[test]
    fn merge_preview_reports_strategy_and_conflicts_without_changing_anything() {
        let s = setup();
        let w = &s.work;
        git(w, &["checkout", "-q", "-b", "ff"]);
        let ff = commit(w, "ff.txt", "ff\n", "ff");
        git(w, &["checkout", "-q", "main"]);
        git(w, &["checkout", "-q", "-b", "clash"]);
        commit(w, "README.md", "clash\n", "clash");
        git(w, &["checkout", "-q", "main"]);
        git(w, &["checkout", "-q", "-b", "feature"]);
        commit(w, "a.txt", "a\n", "a");
        let f2 = commit(w, "b.txt", "b\nb2\n", "b");
        git(w, &["checkout", "-q", "main"]);
        git(w, &["merge", "-q", "--ff-only", "ff"]);
        let main = commit(w, "README.md", "main\n", "main change");
        git(w, &["checkout", "-q", "feature"]);
        std::fs::write(w.join("a.txt"), "local edit\n").unwrap();
        std::fs::write(w.join("scratch.txt"), "x\n").unwrap();
        git(w, &["branch", "behind", &ff]);
        let r = repo(w);
        let before = snapshot(w);

        let p = preview(&r, "ff", "main", MergeMode::Merge, &[]);
        assert_eq!(p.strategy, MergeStrategy::UpToDate);
        assert!(!p.can_apply && codes(&p).contains(&CheckCode::AlreadyInTarget) && p.commits.is_empty());

        let p = preview(&r, "main", "behind", MergeMode::Merge, &[]);
        assert_eq!((p.strategy, p.commits.len(), p.files_changed), (MergeStrategy::FastForward, 1, 1));
        assert!(p.can_apply && !p.applies_in_worktree && p.conflicts.is_empty() && p.selected.is_empty());
        assert_eq!((p.source_sha.as_str(), p.target_sha.as_str()), (main.as_str(), ff.as_str()));

        let p = preview(&r, "feature", "main", MergeMode::Merge, &[]);
        assert_eq!(p.strategy, MergeStrategy::NewCommits);
        assert_eq!((p.commits.len(), p.commits[0].sha.as_str()), (2, f2.as_str()));
        assert_eq!((p.files_changed, p.insertions, p.deletions), (2, 3, 0));
        assert!(p.conflicts.is_empty() && p.can_apply && !p.applies_in_worktree, "{:?}", p.checks);

        let p = preview(&r, "clash", "main", MergeMode::Merge, &[]);
        assert_eq!((p.strategy, p.conflicts.as_slice()), (MergeStrategy::NewCommits, ["README.md".to_string()].as_slice()));
        let c = p.checks.iter().find(|c| c.code == CheckCode::MergeConflict).unwrap();
        assert_eq!(c.severity, CheckSeverity::Error);
        assert_eq!(c.message, "Os dois branches mudaram as mesmas linhas em 1 arquivo. Resolva no seu editor.");
        assert_eq!(c.path.as_deref(), Some("README.md"));
        assert!(!p.can_apply);

        // The target is the current branch, with an unstaged change.
        let p = preview(&r, "main", "feature", MergeMode::Merge, &[]);
        assert!(p.applies_in_worktree && !p.can_apply);
        assert!(p.checks.iter().any(|c| c.code == CheckCode::TargetDirty && c.severity == CheckSeverity::Error));

        assert!(preview_of(&r, "feature", "feature", MergeMode::Merge, &[]).unwrap_err().contains("diferentes"));
        assert!(preview_of(&r, "nope", "main", MergeMode::Merge, &[]).unwrap_err().contains("não existe"));
        assert!(preview_of(&r, "-x", "main", MergeMode::Merge, &[]).unwrap_err().contains("inválido"));
        assert_eq!(snapshot(w), before, "a preview must not touch status, refs or index");
        assert_eq!(std::fs::read_to_string(w.join("a.txt")).unwrap(), "local edit\n");
    }

    #[test]
    fn cherry_pick_preview_one_several_present_merge_commit_and_conflict() {
        let s = setup();
        let w = &s.work;
        git(w, &["checkout", "-q", "-b", "feature"]);
        let c1 = commit(w, "a.txt", "a\n", "c1");
        let c2 = commit(w, "b.txt", "b\n", "c2");
        let c3 = commit(w, "c.txt", "c\n", "c3");
        let c4 = commit(w, "README.md", "feature\n", "c4");
        git(w, &["checkout", "-q", "-b", "side", &c1]);
        commit(w, "side.txt", "s\n", "side");
        git(w, &["checkout", "-q", "feature"]);
        git(w, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
        let mc = git(w, &["rev-parse", "HEAD"]);
        git(w, &["checkout", "-q", "main"]);
        // Committing first gives the copy of c1 another parent: within the same second, a cherry-pick
        // onto c1's own parent would recreate c1 byte for byte and make it reachable from main.
        commit(w, "README.md", "main\n", "main change");
        git(w, &["cherry-pick", &c1]);
        git(w, &["checkout", "-q", "feature"]);
        let r = repo(w);
        let before = snapshot(w);
        let pick = MergeMode::CherryPick;

        let p = preview(&r, "feature", "main", pick, &sha_list(&[&c2]));
        assert_eq!((p.strategy, p.selected.clone(), p.files_changed), (MergeStrategy::NewCommits, sha_list(&[&c2]), 1));
        assert!(p.can_apply && p.conflicts.is_empty(), "{:?}", p.checks);
        assert_eq!(p.commits.len(), 6);
        assert!(!codes(&p).contains(&CheckCode::SourcePushed));

        let p = preview(&r, "feature", "main", pick, &sha_list(&[&c3, &c2[..10]]));
        assert_eq!(p.selected, sha_list(&[&c2, &c3]), "oldest first");
        assert_eq!(p.files_changed, 2);

        let p = preview(&r, "feature", "main", pick, &sha_list(&[&c1, &c2]));
        assert_eq!(p.selected, sha_list(&[&c2]));
        let present = p.checks.iter().find(|c| c.code == CheckCode::AlreadyInTarget).unwrap();
        assert_eq!((present.severity, present.sha.as_deref()), (CheckSeverity::Info, Some(c1.as_str())));
        assert!(p.commits.iter().find(|c| c.sha == c1).unwrap().checks.iter().any(|k| k.code == CheckCode::AlreadyInTarget));
        let p = preview(&r, "feature", "main", pick, &sha_list(&[&c1]));
        assert_eq!((p.strategy, p.can_apply), (MergeStrategy::UpToDate, false));

        let p = preview(&r, "feature", "main", pick, &sha_list(&[&mc, &c2]));
        let merge = p.checks.iter().find(|c| c.code == CheckCode::MergeCommit).unwrap();
        assert_eq!((merge.severity, merge.sha.as_deref()), (CheckSeverity::Error, Some(mc.as_str())));
        assert!(!p.can_apply);

        let p = preview(&r, "feature", "main", pick, &sha_list(&[&c2, &c4]));
        assert_eq!(p.conflicts, ["README.md"]);
        let conflict = p.checks.iter().find(|c| c.code == CheckCode::MergeConflict).unwrap();
        assert_eq!((conflict.severity, conflict.sha.as_deref()), (CheckSeverity::Error, Some(c4.as_str())));
        assert!(!p.can_apply);

        assert!(preview_of(&r, "feature", "main", pick, &sha_list(&["xyz"])).unwrap_err().contains("inválido"));
        let main = git(w, &["rev-parse", "main"]);
        assert!(preview_of(&r, "feature", "main", pick, &sha_list(&[&main])).unwrap_err().contains("não está em feature"));
        assert_eq!(snapshot(w), before);

        git(w, &["push", "-q", "origin", "feature"]);
        let p = preview(&r, "feature", "main", pick, &sha_list(&[&c2]));
        let pushed = p.checks.iter().find(|c| c.code == CheckCode::SourcePushed).unwrap();
        assert_eq!(pushed.severity, CheckSeverity::Info);
        assert!(p.can_apply);
    }

    #[test]
    fn apply_without_worktree_moves_only_the_ref() {
        let s = setup();
        let w = &s.work;
        git(w, &["checkout", "-q", "-b", "feature"]);
        commit(w, "a.txt", "a\n", "c1");
        set_identity(w, "Bob", "bob@work.example");
        let bob_date = [("GIT_AUTHOR_DATE", "2024-05-06T07:08:09-03:00")];
        let c2 = commit_env(w, "b.txt", "b\n", "c2 subject\n\nc2 body.", &bob_date);
        set_identity(w, "Ana", "ana@example.com");
        git(w, &["checkout", "-q", "main"]);
        let m1 = commit(w, "m.txt", "m\n", "m1");
        git(w, &["checkout", "-q", "feature"]);
        std::fs::write(w.join("a.txt"), "local edit\n").unwrap();
        let r = repo(w);
        let status = git(w, &["status", "--porcelain=v1"]);

        let picks = sha_list(&[&c2]);
        let p = preview(&r, "feature", "main", MergeMode::CherryPick, &picks);
        assert!(!p.applies_in_worktree && p.can_apply);
        let author = p.checks.iter().find(|c| c.code == CheckCode::AuthorMismatch).unwrap();
        assert_eq!((author.severity, author.sha.as_deref()), (CheckSeverity::Warning, Some(c2.as_str())));
        let res = apply(&r, &p, &picks).unwrap();
        assert_eq!((res.strategy, res.created, res.ahead_after), (MergeStrategy::NewCommits, 1, 2));
        assert_eq!(res.new_target_sha, git(w, &["rev-parse", "main"]));
        assert_eq!(git(w, &["rev-parse", "main^"]), m1);
        let fmt = git(w, &["log", "-1", "--format=%an|%ae|%ad|%cn|%ce", "--date=iso-strict", "main"]);
        assert_eq!(fmt, "Bob|bob@work.example|2024-05-06T07:08:09-03:00|Ana Conta|ana@example.com");
        assert_eq!(git(w, &["log", "-1", "--format=%B", "main"]), format!("c2 subject\n\nc2 body.\n\n(cherry picked from commit {c2})"));
        assert_eq!(git(w, &["show", "main:b.txt"]), "b");
        assert!(git(w, &["ls-tree", "--name-only", "main"]).lines().all(|f| f != "a.txt"));
        assert_eq!(git(w, &["status", "--porcelain=v1"]), status, "the folder is untouched");
        assert_eq!(std::fs::read_to_string(w.join("a.txt")).unwrap(), "local edit\n");
        assert_eq!(r.branch().as_deref(), Some("feature"));

        let p = preview(&r, "feature", "main", MergeMode::Merge, &[]);
        assert_eq!(p.strategy, MergeStrategy::NewCommits);
        let before = git(w, &["rev-parse", "main"]);
        let res = apply(&r, &p, &[]).unwrap();
        // Not on GitHub yet: m1, the copy of c2, c1, c2 and the merge commit.
        assert_eq!((res.created, res.ahead_after), (1, 5));
        assert_eq!(git(w, &["rev-list", "--parents", "-n1", "main"]), format!("{} {before} {c2}", res.new_target_sha));
        assert_eq!(git(w, &["log", "-1", "--format=%s|%an|%ae|%cn", "main"]), "Merge branch 'feature' into main|Ana Conta|ana@example.com|Ana Conta");
        assert_eq!(git(w, &["show", "main:a.txt"]), "a");
        assert_eq!(git(w, &["status", "--porcelain=v1"]), status);
        assert!(git(w, &["reflog", "-1", "main"]).contains("gitpass: merge feature into main"));

        let tip = git(w, &["commit-tree", "main^{tree}", "-p", "main", "-m", "ahead of main"]);
        git(w, &["branch", "ahead", &tip]);
        let p = preview(&r, "ahead", "main", MergeMode::Merge, &[]);
        assert_eq!(p.strategy, MergeStrategy::FastForward);
        let res = apply(&r, &p, &[]).unwrap();
        assert_eq!((res.created, res.new_target_sha.as_str()), (0, tip.as_str()));
        assert_eq!(git(w, &["rev-parse", "main"]), tip);
    }

    #[test]
    fn apply_refuses_when_branches_moved_and_update_is_atomic() {
        let s = setup();
        let w = &s.work;
        git(w, &["checkout", "-q", "-b", "feature"]);
        let c1 = commit(w, "a.txt", "a\n", "c1");
        let r = repo(w);
        let picks = sha_list(&[&c1]);
        let p = preview(&r, "feature", "main", MergeMode::CherryPick, &picks);
        let moved = git(w, &["commit-tree", "main^{tree}", "-p", "main", "-m", "moved"]);
        git(w, &["update-ref", "refs/heads/main", &moved]);
        let err = apply(&r, &p, &picks).unwrap_err();
        assert!(err.contains("mudaram desde a prévia"), "{err}");
        assert_eq!(git(w, &["rev-parse", "main"]), moved);

        // The branch moves between the new preview and `update-ref`: the old value check refuses it.
        let ids = rules();
        let ctx = MergeContext { binding_id: "b1", remote: Some("origin"), identity: &ids };
        let req = MergeRequest { source: "feature", target: "main", mode: MergeMode::CherryPick, commits: &picks };
        let stale = plan(&r, &ctx, &req).unwrap();
        let again = git(w, &["commit-tree", "main^{tree}", "-p", "main", "-m", "again"]);
        git(w, &["update-ref", "refs/heads/main", &again]);
        let err = apply_detached(&r, &req, &stale, Some(&who())).unwrap_err();
        assert!(err.contains("Nada foi alterado"), "{err}");
        assert_eq!(git(w, &["rev-parse", "main"]), again);
    }

    #[test]
    fn apply_in_current_branch_merges_there_or_restores_everything() {
        let s = setup();
        let w = &s.work;
        git(w, &["checkout", "-q", "-b", "feature"]);
        commit(w, "a.txt", "a\n", "c1");
        commit(w, "b.txt", "b\n", "c2");
        git(w, &["checkout", "-q", "main"]);
        let m1 = commit(w, "m.txt", "m\n", "m1");
        let r = repo(w);

        std::fs::write(w.join("README.md"), "dirty\n").unwrap();
        let p = preview(&r, "feature", "main", MergeMode::CherryPick, &[]);
        assert!(p.applies_in_worktree && !p.can_apply && codes(&p).contains(&CheckCode::TargetDirty));
        let err = apply(&r, &p, &[]).unwrap_err();
        assert!(err.contains("ainda não viraram commit"), "{err}");
        assert_eq!(r.head_sha().as_deref(), Some(m1.as_str()));
        assert_eq!(std::fs::read_to_string(w.join("README.md")).unwrap(), "dirty\n");
        git(w, &["checkout", "--", "README.md"]);

        std::fs::write(w.join("notes.txt"), "untracked\n").unwrap();
        let p = preview(&r, "feature", "main", MergeMode::CherryPick, &[]);
        assert!(p.applies_in_worktree && p.can_apply, "{:?}", p.checks);
        let res = apply(&r, &p, &[]).unwrap();
        assert_eq!((res.created, res.strategy), (2, MergeStrategy::NewCommits));
        assert_eq!(r.head_sha().as_deref(), Some(res.new_target_sha.as_str()));
        assert_eq!(git(w, &["log", "-2", "--format=%s|%an|%cn"]), "c2|Ana|Ana Conta\nc1|Ana|Ana Conta");
        assert!(git(w, &["log", "-1", "--format=%B"]).contains("(cherry picked from commit"));
        assert!(w.join("a.txt").exists() && w.join("b.txt").exists() && w.join("notes.txt").exists());
        assert_eq!(git(w, &["status", "--porcelain=v1"]), "?? notes.txt");
        std::fs::remove_file(w.join("notes.txt")).unwrap();

        // A merge blocked by a hook of the user is aborted and HEAD stays where it was.
        git(w, &["checkout", "-q", "feature"]);
        commit(w, "c.txt", "c\n", "c3");
        git(w, &["checkout", "-q", "main"]);
        let head = r.head_sha().unwrap();
        let hook = w.join(".git/hooks/pre-merge-commit");
        std::fs::write(&hook, "#!/bin/sh\necho blocked by hook >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let p = preview(&r, "feature", "main", MergeMode::Merge, &[]);
        assert!(p.can_apply, "{:?}", p.checks);
        let err = apply(&r, &p, &[]).unwrap_err();
        assert!(err.contains("Nada foi alterado"), "{err}");
        assert_eq!(r.head_sha().as_deref(), Some(head.as_str()));
        assert!(!r.operation_in_progress());
        assert_eq!(git(w, &["status", "--porcelain=v1"]), "");
        std::fs::remove_file(&hook).unwrap();

        // A cherry-pick that fails half-way (an untracked file in the way) leaves nothing applied.
        git(w, &["checkout", "-q", "feature"]);
        let d1 = commit(w, "d1.txt", "1\n", "d1");
        let d2 = commit(w, "blocker.txt", "from feature\n", "d2");
        git(w, &["checkout", "-q", "main"]);
        std::fs::write(w.join("blocker.txt"), "untracked\n").unwrap();
        let picks = sha_list(&[&d1, &d2]);
        let p = preview(&r, "feature", "main", MergeMode::CherryPick, &picks);
        assert!(p.can_apply);
        let err = apply(&r, &p, &picks).unwrap_err();
        assert!(err.contains("Nada foi alterado"), "{err}");
        assert_eq!(r.head_sha().as_deref(), Some(head.as_str()));
        assert!(!r.operation_in_progress() && !git_path_exists(&r, "sequencer"));
        assert!(!w.join("d1.txt").exists());
        assert_eq!(std::fs::read_to_string(w.join("blocker.txt")).unwrap(), "untracked\n");
        assert_eq!(git(w, &["status", "--porcelain=v1"]), "?? blocker.txt");

        // Merge, then fast-forward, in the folder.
        std::fs::remove_file(w.join("blocker.txt")).unwrap();
        git(w, &["branch", "-f", "behind", "main"]);
        let p = preview(&r, "feature", "main", MergeMode::Merge, &[]);
        let res = apply(&r, &p, &[]).unwrap();
        assert_eq!((res.strategy, res.created), (MergeStrategy::NewCommits, 1));
        assert!(w.join("blocker.txt").exists() && w.join("c.txt").exists());
        git(w, &["checkout", "-q", "behind"]);
        let p = preview(&r, "main", "behind", MergeMode::Merge, &[]);
        assert!(p.applies_in_worktree && p.strategy == MergeStrategy::FastForward);
        let res = apply(&r, &p, &[]).unwrap();
        assert_eq!((res.created, Some(res.new_target_sha)), (0, r.head_sha()));
        assert_eq!(git(w, &["status", "--porcelain=v1"]), "");
    }

    #[test]
    fn apply_runs_in_the_worktree_where_the_target_is_open() {
        let s = setup();
        let w = &s.work;
        git(w, &["checkout", "-q", "-b", "feature"]);
        commit(w, "a.txt", "a\n", "c1");
        git(w, &["branch", "release", "main"]);
        let wt_dir = s.root.join("release-dir");
        git(w, &["worktree", "add", "-q", wt_dir.to_str().unwrap(), "release"]);
        commit(&wt_dir, "r.txt", "r\n", "release work");
        let r = repo(w);
        let status = git(w, &["status", "--porcelain=v1"]);

        std::fs::write(wt_dir.join("r.txt"), "dirty\n").unwrap();
        let p = preview(&r, "feature", "release", MergeMode::Merge, &[]);
        let dirty = p.checks.iter().find(|c| c.code == CheckCode::TargetDirty).unwrap();
        assert!(dirty.message.contains(wt_dir.to_str().unwrap()), "{}", dirty.message);
        git(&wt_dir, &["checkout", "--", "r.txt"]);

        let p = preview(&r, "feature", "release", MergeMode::Merge, &[]);
        assert!(p.applies_in_worktree && p.can_apply);
        let res = apply(&r, &p, &[]).unwrap();
        assert_eq!(res.created, 1);
        assert!(wt_dir.join("a.txt").exists(), "merged in the release folder");
        assert_eq!(git(&wt_dir, &["rev-parse", "HEAD"]), res.new_target_sha);
        assert_eq!(git(&wt_dir, &["status", "--porcelain=v1"]), "");
        assert_eq!(git(w, &["status", "--porcelain=v1"]), status);
        assert_eq!(r.branch().as_deref(), Some("feature"));
    }
}
