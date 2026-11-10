//! "Mapear pasta": finds the Git projects under a folder and matches them with the repositories the
//! connected accounts can access on GitHub. Everything here is pure or local (no HTTP): the caller
//! fetches each account's repo list and passes it in, so the matching can be tested offline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::account_rules::suggest_account_for_repo;
use crate::bindings::{
    github_https_url, is_ignored_dir_name, pick_remote, walk_git_work_trees, RemotePick, SuggestionReason,
};
use crate::github_repos::GithubRepoPermissions;
use crate::{git_work_tree_root, normalize_owner, AppConfig, GithubRepoSummary};

/// Deeper than the automatic discovery: the user chose the folder, so it may hold nested groups.
pub const MAP_MAX_DEPTH: usize = 6;
pub const MAP_MAX_PROJECTS: usize = 2000;
pub const MAP_BUDGET: Duration = Duration::from_secs(20);
const MAP_ROOT_MAX_LEN: usize = 4096;

/// Validates the folder chosen in the UI: absolute (or `~`-relative), existing directory. Symlinks resolved.
pub fn resolve_map_root(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Escolha uma pasta para mapear.".to_string());
    }
    if raw.len() > MAP_ROOT_MAX_LEN || raw.contains('\0') {
        return Err("O caminho da pasta é inválido.".to_string());
    }
    let path = match raw.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home
            .ok_or_else(|| "Não foi possível encontrar sua pasta pessoal.".to_string())?
            .join(rest.trim_start_matches('/')),
        _ => PathBuf::from(raw),
    };
    if !path.is_absolute() {
        return Err("Use o caminho completo da pasta (começando com / ou ~).".to_string());
    }
    let canon = path
        .canonicalize()
        .map_err(|_| format!("A pasta {} não existe.", path.display()))?;
    if !canon.is_dir() {
        return Err(format!("{} não é uma pasta. Escolha uma pasta.", canon.display()));
    }
    Ok(canon)
}

// --- Scanning ---

#[derive(Debug, Default)]
pub struct ScanOutcome {
    pub projects: Vec<PathBuf>,
    /// The time budget ran out (or a folder stayed blocked, e.g. waiting for a macOS privacy prompt).
    pub timed_out: bool,
    /// More than `max_projects` projects exist; only the first ones are listed.
    pub limit_reached: bool,
}

impl ScanOutcome {
    pub fn truncated(&self) -> bool {
        self.timed_out || self.limit_reached
    }
}

fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !is_ignored_dir_name(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

enum ScanMsg {
    Found(PathBuf),
    Done { stopped: bool, limit_hit: bool },
}

/// Git work trees under `root` (canonical), up to `max_depth` levels, skipping ignored/hidden dirs and
/// symlinks. When `root` is itself a repo it is listed and its subfolders are still searched (e.g. a
/// home folder versioned for dotfiles). `on_found` gets the running count.
///
/// The walk runs on its own thread so a folder that blocks (macOS privacy prompt, stale network
/// volume) cannot hold the caller past `deadline`; such a walker finishes detached.
pub fn scan_projects(
    root: &Path,
    max_depth: usize,
    max_projects: usize,
    deadline: Instant,
    mut on_found: impl FnMut(usize),
) -> ScanOutcome {
    let mut out = ScanOutcome::default();
    let root_is_repo = std::fs::symlink_metadata(root.join(".git")).is_ok();
    if root_is_repo {
        out.projects.push(root.to_path_buf());
        on_found(1);
    }
    let (starts, depth) = if root_is_repo {
        (child_dirs(root), max_depth.saturating_sub(1))
    } else {
        (vec![root.to_path_buf()], max_depth)
    };
    let budget = max_projects.saturating_sub(out.projects.len());

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut sent = 0usize;
        let mut limit_hit = false;
        let stopped = walk_git_work_trees(&starts, depth, Some(deadline), |p| {
            if sent >= budget {
                limit_hit = true;
                return false;
            }
            sent += 1;
            tx.send(ScanMsg::Found(p)).is_ok()
        });
        let _ = tx.send(ScanMsg::Done { stopped, limit_hit });
    });

    // The walker stops at `deadline` itself; the grace lets its last results arrive.
    let collect_until = deadline + Duration::from_millis(300);
    loop {
        match rx.recv_timeout(collect_until.saturating_duration_since(Instant::now())) {
            Ok(ScanMsg::Found(p)) => {
                if !out.projects.contains(&p) {
                    out.projects.push(p);
                    on_found(out.projects.len());
                }
            }
            Ok(ScanMsg::Done { stopped, limit_hit }) => {
                out.limit_reached = limit_hit;
                out.timed_out = stopped && !limit_hit;
                break;
            }
            Err(_) => {
                out.timed_out = true;
                break;
            }
        }
    }

    // The user picked a folder inside a project: map that project.
    if out.projects.is_empty() && !out.timed_out {
        if let Ok(top) = git_work_tree_root(Some(root)) {
            out.projects.push(top.canonicalize().unwrap_or(top));
            on_found(1);
        }
    }
    out.projects.sort();
    out
}

// --- Matching with GitHub ---

/// Effective permission of an account on a repo, ordered from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Pull,
    Triage,
    Push,
    Maintain,
    Admin,
}

impl Permission {
    /// Highest flag set. A repo listed by `/user/repos` is at least readable, so no flag means `Pull`.
    pub fn from_github(p: &GithubRepoPermissions) -> Self {
        if p.admin {
            Self::Admin
        } else if p.maintain {
            Self::Maintain
        } else if p.push {
            Self::Push
        } else if p.triage {
            Self::Triage
        } else {
            Self::Pull
        }
    }

    pub fn can_push(self) -> bool {
        self >= Self::Push
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MapCandidate {
    pub account_id: Uuid,
    pub permission: Permission,
}

/// `owner/repo` lowercased (GitHub names are case-insensitive); ignores a trailing `.git` or path.
fn repo_key(owner: &str, repo: &str) -> String {
    let repo = repo.split('/').next().unwrap_or(repo).trim_end_matches(".git");
    format!("{}/{}", normalize_owner(owner), repo.to_lowercase())
}

struct KnownRepo {
    /// Name as GitHub spells it.
    full_name: String,
    /// In config order.
    candidates: Vec<MapCandidate>,
}

/// Repos visible to each account, by `owner/repo`.
pub struct RepoIndex(HashMap<String, KnownRepo>);

impl RepoIndex {
    /// `lists`: `(account, repos)` for every account whose list was fetched. Unknown accounts are ignored.
    pub fn new(cfg: &AppConfig, lists: &[(Uuid, Vec<GithubRepoSummary>)]) -> Self {
        let mut map: HashMap<String, KnownRepo> = HashMap::new();
        for acc in &cfg.accounts {
            let Some((_, repos)) = lists.iter().find(|(id, _)| *id == acc.id) else {
                continue;
            };
            for r in repos {
                let Some((owner, name)) = r.full_name.split_once('/') else {
                    continue;
                };
                let permission = Permission::from_github(&r.permissions);
                let known = map.entry(repo_key(owner, name)).or_insert_with(|| KnownRepo {
                    full_name: r.full_name.clone(),
                    candidates: Vec::new(),
                });
                match known.candidates.iter_mut().find(|c| c.account_id == acc.id) {
                    Some(c) => c.permission = c.permission.max(permission),
                    None => known.candidates.push(MapCandidate { account_id: acc.id, permission }),
                }
            }
        }
        Self(map)
    }

    fn get(&self, owner: &str, repo: &str) -> Option<&KnownRepo> {
        self.0.get(&repo_key(owner, repo))
    }
}

/// Highest permission wins; on a tie, the offline suggestion (owner login, then accounts already used
/// for that owner) if it is among the tied accounts, else the first one in config order.
pub fn choose_map_account(cfg: &AppConfig, owner: &str, candidates: &[MapCandidate]) -> Option<Uuid> {
    let best = candidates.iter().map(|c| c.permission).max()?;
    let tied: Vec<Uuid> = candidates
        .iter()
        .filter(|c| c.permission == best)
        .map(|c| c.account_id)
        .collect();
    if tied.len() > 1 {
        if let Some(s) = crate::bindings::suggest_account_offline(cfg, owner).filter(|s| tied.contains(&s.account_id)) {
            return Some(s.account_id);
        }
    }
    tied.first().copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MapStatus {
    /// Already bound in GitPass.
    Bound,
    /// On GitHub, and at least one account can push.
    Match,
    /// On GitHub, but every account with access can only read (pull/triage).
    ReadOnly,
    /// GitHub remote, but no connected account lists the repo (another owner, deleted, renamed, or the
    /// account's list could not be fetched).
    NotFound,
    /// The remote is not on GitHub.
    OtherRemote,
    /// Git project without remotes.
    NoRemote,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MappedProject {
    pub root_path: String,
    pub status: MapStatus,
    pub remote_name: Option<String>,
    /// GitHub: `https://github.com/owner/repo`. Other hosts: the remote with credentials removed.
    pub remote_url: Option<String>,
    /// As GitHub spells it when an account lists the repo, else as parsed from the remote.
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    /// Accounts with access, in config order.
    pub candidates: Vec<MapCandidate>,
    pub suggested_account_id: Option<Uuid>,
    pub binding_id: Option<Uuid>,
    pub bound_account_id: Option<Uuid>,
    /// Bound projects: whether the bound account has the best permission available
    /// (`None` when no account lists the repo, so it cannot be told).
    pub bound_matches_suggestion: Option<bool>,
    pub suggestion_reason: Option<SuggestionReason>,
    /// e.g. «Clientes» when a folder rule matched.
    pub suggestion_label: Option<String>,
}

/// Classifies local projects against the config and the accounts' repo lists.
pub struct MapContext<'a> {
    cfg: &'a AppConfig,
    /// Canonical root → (binding, account).
    bound: HashMap<PathBuf, (Uuid, Uuid)>,
    index: RepoIndex,
}

impl<'a> MapContext<'a> {
    pub fn new(cfg: &'a AppConfig, lists: &[(Uuid, Vec<GithubRepoSummary>)]) -> Self {
        let bound = cfg
            .repos
            .iter()
            .map(|r| {
                let root = r.root_path.canonicalize().unwrap_or_else(|_| r.root_path.clone());
                (root, (r.id, r.account_id))
            })
            .collect();
        Self {
            cfg,
            bound,
            index: RepoIndex::new(cfg, lists),
        }
    }

    /// `root` must be canonical (as returned by [`scan_projects`]); `remotes` as from `read_remotes`.
    pub fn classify(&self, root: &Path, remotes: &[(String, String)]) -> MappedProject {
        let mut p = MappedProject {
            root_path: root.display().to_string(),
            status: MapStatus::NoRemote,
            remote_name: None,
            remote_url: None,
            github_owner: None,
            github_repo: None,
            candidates: Vec::new(),
            suggested_account_id: None,
            binding_id: None,
            bound_account_id: None,
            bound_matches_suggestion: None,
            suggestion_reason: None,
            suggestion_label: None,
        };
        match pick_remote(remotes) {
            RemotePick::None => {}
            RemotePick::Other { name, url } => {
                p.status = MapStatus::OtherRemote;
                p.remote_name = Some(name);
                p.remote_url = Some(url);
            }
            RemotePick::Bitbucket { name, id } => {
                let owner = id.workspace.clone();
                let repo = id.slug.clone();
                p.remote_name = Some(name);
                p.remote_url = Some(crate::remote::bitbucket_https_url(&owner, &repo));
                p.github_owner = Some(owner.clone());
                p.github_repo = Some(repo);
                p.status = MapStatus::NotFound;
                if let Some(s) = suggest_account_for_repo(self.cfg, root, &owner) {
                    p.suggestion_reason = Some(s.reason);
                    p.suggestion_label = s.rule_label.clone();
                    p.suggested_account_id = Some(s.account_id);
                }
            }
            RemotePick::GitHub { name, id } => {
                let known = self.index.get(&id.owner, &id.repo);
                let (owner, repo) = known
                    .and_then(|k| k.full_name.split_once('/'))
                    .map(|(o, r)| (o.to_string(), r.to_string()))
                    .unwrap_or_else(|| {
                        let r = id.repo.split('/').next().unwrap_or(&id.repo).trim_end_matches(".git");
                        (id.owner.clone(), r.to_string())
                    });
                p.candidates = known.map(|k| k.candidates.clone()).unwrap_or_default();
                if let Some(s) = suggest_account_for_repo(self.cfg, root, &id.owner) {
                    p.suggestion_reason = Some(s.reason);
                    p.suggestion_label = s.rule_label.clone();
                    if p.candidates.is_empty() || p.candidates.iter().any(|c| c.account_id == s.account_id) {
                        p.suggested_account_id = Some(s.account_id);
                    }
                }
                if p.suggested_account_id.is_none() {
                    p.suggested_account_id = choose_map_account(self.cfg, &id.owner, &p.candidates);
                }
                p.status = if p.candidates.is_empty() {
                    MapStatus::NotFound
                } else if p.candidates.iter().any(|c| c.permission.can_push()) {
                    MapStatus::Match
                } else {
                    MapStatus::ReadOnly
                };
                p.remote_name = Some(name);
                p.remote_url = Some(github_https_url(&owner, &repo));
                p.github_owner = Some(owner);
                p.github_repo = Some(repo);
            }
        }

        if let Some(&(binding_id, account_id)) = self.bound.get(root) {
            p.status = MapStatus::Bound;
            p.binding_id = Some(binding_id);
            p.bound_account_id = Some(account_id);
            if let Some(best) = p.candidates.iter().map(|c| c.permission).max() {
                let ok = p
                    .candidates
                    .iter()
                    .any(|c| c.account_id == account_id && c.permission == best);
                if ok {
                    p.suggested_account_id = Some(account_id);
                }
                p.bound_matches_suggestion = Some(ok);
            }
        }
        p
    }
}

/// Classifies every `(root, remotes)` pair, sorted by path.
pub fn map_projects(
    cfg: &AppConfig,
    lists: &[(Uuid, Vec<GithubRepoSummary>)],
    projects: &[(PathBuf, Vec<(String, String)>)],
) -> Vec<MappedProject> {
    let ctx = MapContext::new(cfg, lists);
    let mut out: Vec<MappedProject> = projects.iter().map(|(root, remotes)| ctx.classify(root, remotes)).collect();
    out.sort_by(|a, b| a.root_path.cmp(&b.root_path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::read_remotes;
    use crate::{Account, RepoBinding};
    use std::process::Command;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("gitpass-map-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
    }

    fn init_repo(dir: &Path, remotes: &[(&str, &str)]) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        for (name, url) in remotes {
            git(dir, &["remote", "add", name, url]);
        }
    }

    fn account(login: &str) -> Account {
        Account {
            id: Uuid::new_v4(),
            github_login: Some(login.into()),
            ..Default::default()
        }
    }

    fn repo(full_name: &str, perm: Permission) -> GithubRepoSummary {
        let mut s = GithubRepoSummary {
            full_name: full_name.into(),
            name: full_name.split('/').nth(1).unwrap_or_default().into(),
            ..Default::default()
        };
        let p = &mut s.permissions;
        p.pull = true;
        p.triage = perm >= Permission::Triage;
        p.push = perm >= Permission::Push;
        p.maintain = perm >= Permission::Maintain;
        p.admin = perm == Permission::Admin;
        s
    }

    fn cand(account: &Account, permission: Permission) -> MapCandidate {
        MapCandidate { account_id: account.id, permission }
    }

    #[test]
    fn permission_order_and_mapping() {
        assert!(Permission::Admin > Permission::Maintain && Permission::Maintain > Permission::Push);
        assert!(Permission::Push > Permission::Triage && Permission::Triage > Permission::Pull);
        assert!(Permission::Push.can_push() && !Permission::Triage.can_push());
        assert_eq!(Permission::from_github(&repo("a/b", Permission::Maintain).permissions), Permission::Maintain);
        assert_eq!(Permission::from_github(&GithubRepoPermissions::default()), Permission::Pull);
    }

    #[test]
    fn chooses_highest_permission_then_owner_then_first() {
        let ana = account("ana");
        let bob = account("bob");
        let eve = account("eve");
        let mut cfg = AppConfig::default();
        cfg.accounts = vec![ana.clone(), bob.clone(), eve.clone()];

        let c = [cand(&ana, Permission::Push), cand(&bob, Permission::Admin)];
        assert_eq!(choose_map_account(&cfg, "ana", &c), Some(bob.id));

        let tie = [cand(&ana, Permission::Push), cand(&bob, Permission::Push), cand(&eve, Permission::Pull)];
        assert_eq!(choose_map_account(&cfg, "BOB", &tie), Some(bob.id));
        assert_eq!(choose_map_account(&cfg, "acme", &tie), Some(ana.id));
        // The owner's own account only wins among the accounts with the best permission.
        assert_eq!(choose_map_account(&cfg, "eve", &tie), Some(ana.id));

        // Tie broken by the account already used for that owner.
        cfg.repos.push(RepoBinding {
            id: Uuid::new_v4(),
            account_id: bob.id,
            root_path: PathBuf::from("/tmp/none/acme"),
            github_owner: Some("acme".into()),
            github_repo: Some("x".into()),
        });
        assert_eq!(choose_map_account(&cfg, "acme", &tie), Some(bob.id));
        assert_eq!(choose_map_account(&cfg, "acme", &[]), None);
    }

    #[test]
    fn classifies_every_kind_of_project() {
        let t = TempDir::new();
        let r = &t.0;
        init_repo(&r.join("mine"), &[("origin", "git@github.com:ana/mine.git")]);
        init_repo(&r.join("work/shared"), &[("origin", "https://github.com/Acme/Shared")]);
        init_repo(&r.join("work/docs"), &[("origin", "https://github.com/acme/docs.git")]);
        init_repo(&r.join("bound-wrong"), &[("origin", "git@github.com:ana/site.git")]);
        init_repo(&r.join("bound-ok"), &[("origin", "git@github.com:ana/mine2.git")]);
        init_repo(&r.join("bound-unknown"), &[("origin", "git@github.com:zed/old.git")]);
        init_repo(&r.join("gone"), &[("origin", "git@github.com:someone/gone.git")]);
        init_repo(&r.join("lab"), &[("origin", "https://ana:s3cr3t@gitlab.com/ana/lab.git")]);
        init_repo(&r.join("local"), &[]);
        init_repo(&r.join("fork"), &[("upstream", "git@gitlab.com:x/y.git"), ("gh", "git@github.com:ana/Fork.git")]);

        let ana = account("ana");
        let bob = account("bob");
        let offline = account("carol");
        let mut cfg = AppConfig::default();
        cfg.accounts = vec![ana.clone(), bob.clone(), offline.clone()];
        let bind = |name: &str, acc: &Account| RepoBinding {
            id: Uuid::new_v4(),
            account_id: acc.id,
            root_path: r.join(name),
            github_owner: None,
            github_repo: None,
        };
        cfg.repos = vec![bind("bound-wrong", &bob), bind("bound-ok", &ana), bind("bound-unknown", &ana)];

        // `carol` could not be fetched: she has no list.
        let lists = vec![
            (
                ana.id,
                vec![
                    repo("ana/mine", Permission::Admin),
                    repo("Acme/Shared", Permission::Push),
                    repo("acme/docs", Permission::Pull),
                    repo("ana/site", Permission::Admin),
                    repo("ana/mine2", Permission::Admin),
                    repo("ana/fork", Permission::Admin),
                ],
            ),
            (
                bob.id,
                vec![
                    repo("acme/shared", Permission::Admin),
                    repo("acme/docs", Permission::Triage),
                    repo("ana/site", Permission::Push),
                ],
            ),
        ];

        let scan = scan_projects(r, MAP_MAX_DEPTH, MAP_MAX_PROJECTS, Instant::now() + Duration::from_secs(10), |_| {});
        assert!(!scan.truncated());
        assert_eq!(scan.projects.len(), 10);
        let projects: Vec<_> = scan.projects.iter().map(|p| (p.clone(), read_remotes(p))).collect();
        let items = map_projects(&cfg, &lists, &projects);
        let by = |name: &str| items.iter().find(|i| i.root_path == r.join(name).display().to_string()).unwrap();

        let mine = by("mine");
        assert_eq!(mine.status, MapStatus::Match);
        assert_eq!(mine.suggested_account_id, Some(ana.id));
        assert_eq!(mine.remote_url.as_deref(), Some("https://github.com/ana/mine"));

        let shared = by("work/shared");
        assert_eq!(shared.status, MapStatus::Match);
        assert_eq!(shared.candidates, vec![cand(&ana, Permission::Push), cand(&bob, Permission::Admin)]);
        assert_eq!(shared.suggested_account_id, Some(bob.id));
        // GitHub's spelling of the name wins over the remote's.
        assert_eq!((shared.github_owner.as_deref(), shared.github_repo.as_deref()), (Some("Acme"), Some("Shared")));

        let docs = by("work/docs");
        assert_eq!(docs.status, MapStatus::ReadOnly);
        assert_eq!(docs.suggested_account_id, Some(bob.id));

        let wrong = by("bound-wrong");
        assert_eq!(wrong.status, MapStatus::Bound);
        assert_eq!(wrong.bound_account_id, Some(bob.id));
        assert_eq!(wrong.bound_matches_suggestion, Some(false));
        assert_eq!(wrong.suggested_account_id, Some(ana.id));
        assert!(wrong.binding_id.is_some());

        let ok = by("bound-ok");
        assert_eq!((ok.status, ok.bound_matches_suggestion), (MapStatus::Bound, Some(true)));
        assert_eq!(ok.suggested_account_id, Some(ana.id));

        let unknown = by("bound-unknown");
        assert_eq!((unknown.status, unknown.bound_matches_suggestion), (MapStatus::Bound, None));

        let gone = by("gone");
        assert_eq!(gone.status, MapStatus::NotFound);
        assert!(gone.candidates.is_empty() && gone.suggested_account_id.is_none());
        assert_eq!(gone.github_owner.as_deref(), Some("someone"));

        let lab = by("lab");
        assert_eq!(lab.status, MapStatus::OtherRemote);
        assert_eq!(lab.remote_url.as_deref(), Some("https://gitlab.com/ana/lab.git"));
        assert!(!format!("{items:?}").contains("s3cr3t"));

        let local = by("local");
        assert_eq!((local.status, local.remote_name.as_deref()), (MapStatus::NoRemote, None));

        let fork = by("fork");
        assert_eq!(fork.status, MapStatus::Match);
        assert_eq!(fork.remote_name.as_deref(), Some("gh"));
        assert_eq!(fork.github_repo.as_deref(), Some("fork"));
    }

    #[test]
    fn scan_respects_depth_ignores_symlinks_and_limit() {
        let t = TempDir::new();
        let r = &t.0;
        init_repo(&r.join("a/b/c/d/e/deep"), &[]);
        init_repo(&r.join("a/b/c/d/e/f/too-deep"), &[]);
        init_repo(&r.join("node_modules/dep"), &[]);
        init_repo(&r.join(".cache/hidden"), &[]);
        init_repo(&r.join("app"), &[]);
        init_repo(&r.join("app/nested"), &[]);
        #[cfg(unix)]
        std::os::unix::fs::symlink(r.join("app"), r.join("link")).unwrap();

        let far = Instant::now() + Duration::from_secs(10);
        let mut counts = Vec::new();
        let scan = scan_projects(r, MAP_MAX_DEPTH, 100, far, |n| counts.push(n));
        assert_eq!(scan.projects, vec![r.join("a/b/c/d/e/deep"), r.join("app")]);
        assert!(!scan.truncated());
        assert_eq!(counts.last(), Some(&2));

        let limited = scan_projects(r, MAP_MAX_DEPTH, 1, far, |_| {});
        assert_eq!(limited.projects.len(), 1);
        assert!(limited.limit_reached && !limited.timed_out);

        let late = scan_projects(r, MAP_MAX_DEPTH, 100, Instant::now(), |_| {});
        assert!(late.timed_out);
    }

    #[test]
    fn scan_of_a_repo_root_or_subfolder() {
        let t = TempDir::new();
        let r = &t.0;
        init_repo(r, &[]);
        init_repo(&r.join("projects/one"), &[]);
        std::fs::create_dir_all(r.join("projects/one/src")).unwrap();
        let far = Instant::now() + Duration::from_secs(10);

        let scan = scan_projects(r, MAP_MAX_DEPTH, 100, far, |_| {});
        assert_eq!(scan.projects, vec![r.clone(), r.join("projects/one")]);

        let inside = scan_projects(&r.join("projects/one/src"), MAP_MAX_DEPTH, 100, far, |_| {});
        assert_eq!(inside.projects, vec![r.join("projects/one")]);
    }

    #[test]
    fn validates_the_chosen_folder() {
        let t = TempDir::new();
        let home = t.0.join("home");
        std::fs::create_dir_all(home.join("Projects")).unwrap();
        std::fs::write(home.join("notes.txt"), "x").unwrap();

        assert_eq!(resolve_map_root("~/Projects", Some(&home)), Ok(home.join("Projects")));
        assert_eq!(resolve_map_root(" ~ ", Some(&home)), Ok(home.clone()));
        assert_eq!(resolve_map_root(&format!("{}/Projects/", home.display()), None), Ok(home.join("Projects")));
        assert!(resolve_map_root("", Some(&home)).unwrap_err().contains("Escolha"));
        assert!(resolve_map_root("Projects", Some(&home)).unwrap_err().contains("caminho completo"));
        assert!(resolve_map_root("~/missing", Some(&home)).unwrap_err().contains("não existe"));
        assert!(resolve_map_root("~/notes.txt", Some(&home)).unwrap_err().contains("não é uma pasta"));
        assert!(resolve_map_root("~", None).unwrap_err().contains("pasta pessoal"));
        assert!(resolve_map_root("/tmp/a\0b", None).is_err());
        assert!(resolve_map_root(&format!("/{}", "a".repeat(5000)), None).is_err());
        // `~user` is not expanded: it is a relative name.
        assert!(resolve_map_root("~ana/x", Some(&home)).is_err());
    }
}
