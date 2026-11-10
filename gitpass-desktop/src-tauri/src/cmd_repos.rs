//! Tauri commands for this area. Registered in main.rs `generate_handler!`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gitpass_core::bindings::{
    change_binding_account, create_binding, decide_account, default_discover_roots, discover_unbound_repos,
    expected_identity, fix_binding_identity, github_https_url, identity_state, pick_remote, read_head,
    read_origin_url, read_remotes, read_repo_identity, remote_url_has_credentials, remove_binding,
    AccountDecision, IdentityState, RemotePick, SuggestionReason, DISCOVER_MAX_DEPTH,
};
use gitpass_core::github_api::{token_can_push_for_account, GithubApiError};
use gitpass_core::identity_diagnostic::{binding_identity_diagnostic, CommitDiagnostic, IdentityDiagnostic, PushDiagnostic};
use gitpass_core::repo_map::{
    map_projects, resolve_map_root, scan_projects, MappedProject, MAP_BUDGET, MAP_MAX_DEPTH, MAP_MAX_PROJECTS,
};
use gitpass_core::config_history::{record_change_account, record_fix_identity, ConfigHistoryStatus};
use gitpass_core::outgoing::{GitRepo, SyncSummary};
use gitpass_core::{git_work_tree_root, parse_github_remote, AppConfig, GithubRepoSummary, RepoBinding};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::cmd_cloud::list_account_repos;

const CONFIG_CHANGED_EVENT: &str = "gitpass-config-changed";
pub(crate) const MAP_PROGRESS_EVENT: &str = "repos-map-progress";
const MAP_PROGRESS_EVERY: Duration = Duration::from_millis(150);
const DISCOVER_BUDGET: Duration = Duration::from_millis(2500);
const MAX_DISCOVER_ROOTS: usize = 32;
const MAX_BIND_MANY: usize = 500;

#[derive(Serialize)]
pub struct IdentityRow {
    /// `None` when the folder no longer exists.
    state: Option<IdentityState>,
    local_name: Option<String>,
    local_email: Option<String>,
    expected_name: Option<String>,
    expected_email: Option<String>,
}

#[derive(Serialize)]
pub struct RepoRow {
    id: String,
    account_id: String,
    account_login: Option<String>,
    account_label: Option<String>,
    /// The bound account was removed from the config.
    account_missing: bool,
    root_path: String,
    github_owner: Option<String>,
    github_repo: Option<String>,
    /// Normalized `https://github.com/owner/repo` (never the raw remote, which may hold credentials).
    remote_url: Option<String>,
    /// The raw `origin` embeds a password/token and should be cleaned.
    remote_has_credentials: bool,
    exists: bool,
    branch: Option<String>,
    detached_at: Option<String>,
    identity: IdentityRow,
    /// Local ahead/behind and pending changes (no network); `None` when the folder is missing.
    sync: Option<SyncSummary>,
}

#[derive(Serialize)]
pub struct CandidateRow {
    account_id: String,
    /// `None`: could not verify (no token, offline, API error).
    can_push: Option<bool>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BindOutcome {
    /// The user closed the folder picker; the UI shows nothing.
    Cancelled,
    Bound {
        repo: RepoRow,
        /// Why this account was chosen automatically (`None` when the UI passed one).
        reason: Option<SuggestionReason>,
    },
    /// Ambiguous: the UI confirms the account and then calls `repos_bind_path`.
    NeedsAccount {
        root_path: String,
        github_owner: String,
        github_repo: String,
        suggested_account_id: Option<String>,
        candidates: Vec<CandidateRow>,
    },
}

#[derive(Deserialize)]
pub struct BindRequest {
    path: String,
    account_id: String,
}

#[derive(Serialize)]
pub struct BindItemResult {
    path: String,
    repo: Option<RepoRow>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct DiscoveredRow {
    root_path: String,
    github_owner: String,
    github_repo: String,
    remote_url: String,
    suggested_account_id: Option<String>,
    suggestion_reason: Option<SuggestionReason>,
    suggestion_label: Option<String>,
}

#[derive(Serialize)]
pub struct DiscoverResult {
    repos: Vec<DiscoveredRow>,
    /// Existing folders that were scanned.
    roots: Vec<String>,
    /// The time budget ran out before every folder was visited.
    truncated: bool,
    elapsed_ms: u64,
}

/// `repos-map-progress` payload. `phase`: `scanning`, `github` or `matching`.
#[derive(Clone, Serialize)]
pub(crate) struct MapProgress {
    pub(crate) phase: &'static str,
    /// Projects found so far.
    pub(crate) found: usize,
}

#[derive(Serialize)]
pub(crate) struct MapAccountRow {
    pub(crate) account_id: String,
    /// The account's repo list was fetched.
    pub(crate) ok: bool,
    pub(crate) error: Option<String>,
    pub(crate) repo_count: usize,
}

#[derive(Serialize)]
pub(crate) struct MapResult {
    /// The mapped folder, canonical.
    pub(crate) root: String,
    pub(crate) items: Vec<MappedProject>,
    pub(crate) accounts: Vec<MapAccountRow>,
    /// Not every folder was visited (time ran out or `max_projects` was reached).
    pub(crate) truncated: bool,
    pub(crate) limit_reached: bool,
    pub(crate) max_projects: usize,
    pub(crate) elapsed_ms: u64,
}

pub(crate) async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Algo deu errado no app: {e}"))?
}

pub(crate) fn load_config() -> Result<AppConfig, String> {
    AppConfig::load_or_default().map_err(|e| format!("Não foi possível ler as configurações do GitPass: {e}"))
}

/// Repos with local commits not yet on the remote (no network; scans each bound folder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingPushSummary {
    pub repos: usize,
    pub commits: usize,
}

pub(crate) fn pending_push_summary(cfg: &AppConfig) -> PendingPushSummary {
    let (repos, commits) = cfg.repos.iter().fold((0usize, 0usize), |(r, c), b| {
        if !b.root_path.is_dir() {
            return (r, c);
        }
        let Some(s) = GitRepo::new(&b.root_path).sync_summary() else {
            return (r, c);
        };
        if s.ahead == 0 {
            (r, c)
        } else {
            (r + 1, c + s.ahead)
        }
    });
    PendingPushSummary { repos, commits }
}

fn parse_id(raw: &str, what: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw).map_err(|_| format!("Identificador de {what} inválido."))
}

pub(crate) fn notify(app: &AppHandle) {
    let _ = app.emit(CONFIG_CHANGED_EVENT, ());
}

fn build_row(cfg: &AppConfig, b: &RepoBinding) -> RepoRow {
    let account = cfg.accounts.iter().find(|a| a.id == b.account_id);
    let expected = account.map(expected_identity).unwrap_or_default();
    let exists = b.root_path.is_dir();

    let (identity, head, origin) = if exists {
        let local = read_repo_identity(&b.root_path);
        let state = identity_state(&local, &expected);
        (Some((local, state)), read_head(&b.root_path), read_origin_url(&b.root_path))
    } else {
        (None, Default::default(), None)
    };
    let sync = if exists { GitRepo::new(&b.root_path).sync_summary() } else { None };

    let parsed = origin.as_deref().and_then(parse_github_remote);
    let (owner, repo) = match parsed {
        Some(gh) => (Some(gh.owner), Some(gh.repo)),
        None => (b.github_owner.clone(), b.github_repo.clone()),
    };
    let remote_url = match (&owner, &repo) {
        (Some(o), Some(r)) => Some(github_https_url(o, r)),
        _ => None,
    };

    let (local, state) = match identity {
        Some((l, s)) => (l, Some(s)),
        None => (Default::default(), None),
    };

    RepoRow {
        id: b.id.to_string(),
        account_id: b.account_id.to_string(),
        account_login: account.and_then(|a| a.github_login.clone()),
        account_label: account.and_then(|a| a.label.clone()),
        account_missing: account.is_none(),
        root_path: b.root_path.display().to_string(),
        github_owner: owner,
        github_repo: repo,
        remote_url,
        remote_has_credentials: origin.as_deref().is_some_and(remote_url_has_credentials),
        exists,
        branch: head.branch,
        detached_at: head.detached_at,
        identity: IdentityRow {
            state,
            local_name: local.name,
            local_email: local.email,
            expected_name: expected.name,
            expected_email: expected.email,
        },
        sync,
    }
}

pub(crate) fn row_for(cfg: &AppConfig, binding_id: Uuid) -> Result<RepoRow, String> {
    cfg.repos
        .iter()
        .find(|r| r.id == binding_id)
        .map(|b| build_row(cfg, b))
        .ok_or_else(|| "Vínculo desconhecido. Atualize a lista e tente de novo.".to_string())
}

struct RepoTarget {
    root: PathBuf,
    owner: String,
    repo: String,
}

/// Resolves a picked folder (root or any subfolder) to its work tree and GitHub `origin`.
fn inspect_folder(path: &Path) -> Result<RepoTarget, String> {
    if !path.is_dir() {
        return Err(format!("A pasta {} não existe.", path.display()));
    }
    let root = git_work_tree_root(Some(path)).map_err(|_| {
        "A pasta escolhida não é um repositório Git. Escolha a pasta de um clone (com .git) ou clone o repositório na aba Nuvem.".to_string()
    })?;
    let root = root.canonicalize().unwrap_or(root);
    let (owner, repo) = match pick_remote(&read_remotes(&root)) {
        RemotePick::GitHub { id, .. } => (id.owner, id.repo),
        RemotePick::Bitbucket { id, .. } => (id.workspace, id.slug),
        RemotePick::Other { name, .. } => {
            return Err(format!(
                "O remoto `{name}` não aponta para github.com nem bitbucket.org. O GitPass suporta esses hosts."
            ));
        }
        RemotePick::None => {
            return Err(
                "Este repositório não tem o remoto `origin`. Adicione um (por exemplo, no GitHub Desktop) e tente de novo."
                    .to_string(),
            );
        }
    };
    Ok(RepoTarget {
        root,
        owner,
        repo,
    })
}

fn ensure_not_bound(cfg: &AppConfig, root: &Path) -> Result<(), String> {
    let Some(b) = cfg
        .repos
        .iter()
        .find(|r| r.root_path.canonicalize().unwrap_or_else(|_| r.root_path.clone()) == root)
    else {
        return Ok(());
    };
    let who = cfg
        .accounts
        .iter()
        .find(|a| a.id == b.account_id)
        .and_then(|a| a.github_login.as_deref())
        .map(|l| format!(" à conta @{l}"))
        .unwrap_or_default();
    Err(format!(
        "Esta pasta já está vinculada{who}. Para usar outra conta, escolha «Mudar conta» nos detalhes do repositório."
    ))
}

/// `create_binding` saves before applying the identity; if only the identity fails, the binding exists anyway.
fn bind(app: &AppHandle, cfg: &mut AppConfig, account: Uuid, root: &Path) -> Result<RepoBinding, String> {
    match create_binding(cfg, account, root) {
        Ok(b) => Ok(b),
        Err(e) if cfg.repos.iter().any(|r| r.root_path == root) => {
            notify(app);
            Err(format!(
                "O repositório foi vinculado, mas não foi possível aplicar a identidade do Git ({e}). Use «Corrigir» no repositório."
            ))
        }
        Err(e) => Err(e),
    }
}

fn push_check(account_id: Uuid, owner: &str, repo: &str) -> Option<bool> {
    match token_can_push_for_account(account_id, owner, repo) {
        Ok(p) => Some(p),
        // Private repos the token cannot see answer 404.
        Err(GithubApiError::Http { status: 403 | 404, .. }) => Some(false),
        Err(_) => None,
    }
}

/// Offline suggestion first; only queries the API (in parallel) when the owner is not one of the logins.
fn choose_account(cfg: &AppConfig, target: &RepoTarget) -> (AccountDecision, Vec<(Uuid, Option<bool>)>) {
    let offline = gitpass_core::account_rules::suggest_account_for_repo(cfg, &target.root, &target.owner);
    if let Some(s) = offline.as_ref().filter(|s| {
        matches!(
            s.reason,
            SuggestionReason::OwnerLogin
                | SuggestionReason::FolderRule
                | SuggestionReason::OrgRule
                | SuggestionReason::FolderOrgRule
        )
    }) {
        return (AccountDecision::Bind(s.clone()), Vec::new());
    }
    let checks: Vec<(Uuid, Option<bool>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = cfg
            .accounts
            .iter()
            .map(|a| {
                let id = a.id;
                (id, scope.spawn(move || push_check(id, &target.owner, &target.repo)))
            })
            .collect();
        handles
            .into_iter()
            .map(|(id, h)| (id, h.join().ok().flatten()))
            .collect()
    });
    (decide_account(offline, &checks), checks)
}

#[cfg(target_os = "macos")]
pub(crate) fn osascript(script: &str) -> Result<Result<Option<PathBuf>, String>, String> {
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Não foi possível abrir o seletor de pastas: {e}"))?;
    if out.status.success() {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Ok(Ok((!path.is_empty()).then(|| PathBuf::from(path))));
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if err.contains("-128") {
        return Ok(Ok(None));
    }
    Ok(Err(err))
}

/// Native folder picker. `Ok(None)` when the user cancels.
#[cfg(target_os = "macos")]
fn pick_folder(prompt: &str) -> Result<Option<PathBuf>, String> {
    let choose = format!(r#"POSIX path of (choose folder with prompt "{prompt}")"#);
    let in_front = format!("tell application \"System Events\"\nactivate\n{choose}\nend tell");
    match osascript(&in_front)? {
        Ok(p) => Ok(p),
        // Automation of System Events was denied: the plain dialog still works, it may just open behind the window.
        Err(e) if e.contains("-1743") || e.contains("-600") => osascript(&choose)?
            .map_err(|e| format!("Não foi possível abrir o seletor de pastas: {e}")),
        Err(e) => Err(format!("Não foi possível abrir o seletor de pastas: {e}")),
    }
}

#[cfg(not(target_os = "macos"))]
fn pick_folder(_prompt: &str) -> Result<Option<PathBuf>, String> {
    Err("O seletor de pastas só funciona no macOS.".to_string())
}

fn expand_home(raw: &str) -> PathBuf {
    let home = dirs_home();
    match raw.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match home {
            Some(h) => h.join(rest.trim_start_matches('/')),
            None => PathBuf::from(raw),
        },
        _ => PathBuf::from(raw),
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Root of a bound repo matching `path` (symlinks resolved). The WebView is untrusted: only folders
/// listed in the config may be opened or inspected (`open` would launch an `.app` bundle, for instance).
pub(crate) fn bound_repo_root(path: &str) -> Result<PathBuf, String> {
    let p = expand_home(path);
    let Ok(c) = p.canonicalize() else {
        return Err(format!("A pasta {} não existe.", p.display()));
    };
    let cfg = load_config()?;
    let bound = cfg
        .repos
        .iter()
        .any(|r| r.root_path.canonicalize().unwrap_or_else(|_| r.root_path.clone()) == c);
    if bound && c.is_dir() {
        Ok(c)
    } else {
        Err("Esta pasta não é um repositório vinculado no GitPass. Atualize a lista e tente de novo.".to_string())
    }
}

pub(crate) fn open_with(args: &[&str], path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let st = std::process::Command::new("open")
            .args(args)
            .arg(path)
            .status()
            .map_err(|e| format!("Não foi possível abrir: {e}"))?;
        if st.success() {
            Ok(())
        } else {
            Err("Não foi possível abrir a pasta.".to_string())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (args, path);
        Err("Só funciona no macOS.".to_string())
    }
}

// --- Commands ---

/// Bound repos with branch, identity check and normalized remote.
#[tauri::command]
pub async fn repos_list() -> Result<Vec<RepoRow>, String> {
    blocking(|| {
        let cfg = load_config()?;
        let cfg = &cfg;
        let rows = std::thread::scope(|scope| {
            let handles: Vec<_> = cfg.repos.iter().map(|b| scope.spawn(move || build_row(cfg, b))).collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        Ok(rows)
    })
    .await
}

/// Folder picker → binding. With `account_id = None`, picks the account automatically or asks the UI to confirm.
#[tauri::command]
pub async fn repos_bind_dialog(app: AppHandle, account_id: Option<String>) -> Result<BindOutcome, String> {
    blocking(move || {
        let forced = account_id.as_deref().map(|id| parse_id(id, "conta")).transpose()?;
        let Some(path) = pick_folder("Escolha a pasta do repositório Git")? else {
            return Ok(BindOutcome::Cancelled);
        };
        let target = inspect_folder(&path)?;
        let mut cfg = load_config()?;
        ensure_not_bound(&cfg, &target.root)?;
        if cfg.accounts.is_empty() {
            return Err("Primeiro, conecte uma conta do GitHub na aba Contas.".to_string());
        }

        let (account, reason) = match forced {
            Some(id) => (id, None),
            None => match choose_account(&cfg, &target) {
                (AccountDecision::Bind(s), _) => (s.account_id, Some(s.reason)),
                (AccountDecision::Ask { suggested }, checks) => {
                    return Ok(BindOutcome::NeedsAccount {
                        root_path: target.root.display().to_string(),
                        github_owner: target.owner,
                        github_repo: target.repo,
                        suggested_account_id: suggested.map(|id| id.to_string()),
                        candidates: checks
                            .into_iter()
                            .map(|(id, can_push)| CandidateRow {
                                account_id: id.to_string(),
                                can_push,
                            })
                            .collect(),
                    });
                }
            },
        };

        let b = bind(&app, &mut cfg, account, &target.root)?;
        notify(&app);
        Ok(BindOutcome::Bound {
            repo: build_row(&cfg, &b),
            reason,
        })
    })
    .await
}

/// Binds a known folder (after `needs_account` confirmation or from discovery).
#[tauri::command]
pub async fn repos_bind_path(app: AppHandle, path: String, account_id: String) -> Result<RepoRow, String> {
    blocking(move || {
        let account = parse_id(&account_id, "conta")?;
        let target = inspect_folder(&expand_home(&path))?;
        let mut cfg = load_config()?;
        ensure_not_bound(&cfg, &target.root)?;
        let b = bind(&app, &mut cfg, account, &target.root)?;
        notify(&app);
        Ok(build_row(&cfg, &b))
    })
    .await
}

/// "Vincular todos": one result per item; failures do not stop the rest.
#[tauri::command]
pub async fn repos_bind_many(app: AppHandle, items: Vec<BindRequest>) -> Result<Vec<BindItemResult>, String> {
    if items.len() > MAX_BIND_MANY {
        return Err(format!("Vincule no máximo {MAX_BIND_MANY} repositórios de cada vez."));
    }
    blocking(move || {
        let mut cfg = load_config()?;
        let mut out = Vec::with_capacity(items.len());
        let mut changed = false;
        for item in items {
            let res = (|| {
                let account = parse_id(&item.account_id, "conta")?;
                let target = inspect_folder(&expand_home(&item.path))?;
                ensure_not_bound(&cfg, &target.root)?;
                let b = bind(&app, &mut cfg, account, &target.root)?;
                Ok::<_, String>(build_row(&cfg, &b))
            })();
            match res {
                Ok(row) => {
                    changed = true;
                    out.push(BindItemResult {
                        path: item.path,
                        repo: Some(row),
                        error: None,
                    });
                }
                Err(e) => out.push(BindItemResult {
                    path: item.path,
                    repo: None,
                    error: Some(e),
                }),
            }
        }
        if changed {
            notify(&app);
        }
        Ok(out)
    })
    .await
}

/// Removes the binding; with `reset_identity`, also removes the `user.name`/`user.email` GitPass applied.
#[tauri::command]
pub async fn repos_unbind(app: AppHandle, binding_id: String, reset_identity: bool) -> Result<(), String> {
    blocking(move || {
        let id = parse_id(&binding_id, "vínculo")?;
        let mut cfg = load_config()?;
        remove_binding(&mut cfg, id, reset_identity)?;
        notify(&app);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn repos_change_account(app: AppHandle, binding_id: String, account_id: String) -> Result<RepoRow, String> {
    blocking(move || {
        let id = parse_id(&binding_id, "vínculo")?;
        let account = parse_id(&account_id, "conta")?;
        let mut cfg = load_config()?;
        if !cfg.accounts.iter().any(|a| a.id == account) {
            return Err("Conta desconhecida. Atualize a lista e tente de novo.".to_string());
        }
        let Some(binding) = cfg.repos.iter().find(|r| r.id == id).cloned() else {
            return Err("Vínculo desconhecido. Atualize a lista e tente de novo.".to_string());
        };
        let before_account = binding.account_id;
        let before_identity = read_repo_identity(&binding.root_path);
        let res = change_binding_account(&mut cfg, id, account);
        let after_identity = read_repo_identity(&binding.root_path);
        let (status, message) = match &res {
            Ok(_) => (ConfigHistoryStatus::Applied, None),
            Err(e) => (
                ConfigHistoryStatus::Partial,
                Some(format!("A conta foi trocada, mas a identidade do Git não foi aplicada: {e}")),
            ),
        };
        let _ = record_change_account(
            id,
            before_account,
            account,
            before_identity,
            after_identity,
            status,
            message.clone(),
        );
        notify(&app);
        res.map_err(|e| message.unwrap_or_else(|| format!("A conta foi trocada, mas a identidade do Git não foi aplicada: {e}")))?;
        row_for(&cfg, id)
    })
    .await
}

#[tauri::command]
pub async fn repos_fix_identity(binding_id: String) -> Result<RepoRow, String> {
    blocking(move || {
        let id = parse_id(&binding_id, "vínculo")?;
        let cfg = load_config()?;
        let root = cfg
            .repos
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.root_path.clone())
            .ok_or_else(|| "Vínculo desconhecido.".to_string())?;
        let before = read_repo_identity(&root);
        let res = fix_binding_identity(&cfg, id);
        let (status, message) = match &res {
            Ok(_) => (ConfigHistoryStatus::Applied, None),
            Err(e) => (ConfigHistoryStatus::Failed, Some(e.clone())),
        };
        let _ = record_fix_identity(id, &root, before, status, message.clone());
        res.map_err(|e| format!("Não foi possível corrigir a identidade: {e}"))?;
        row_for(&cfg, id)
    })
    .await
}

/// Read-only commit/push identity diagnostic for the repository drawer (Etapa 1).
#[tauri::command]
pub async fn repos_identity_diagnostic(
    binding_id: String,
    remote: Option<String>,
    verify_token: Option<bool>,
) -> Result<IdentityDiagnosticRow, String> {
    blocking(move || {
        let id = parse_id(&binding_id, "vínculo")?;
        let cfg = load_config()?;
        let d = binding_identity_diagnostic(&cfg, id, remote.as_deref(), verify_token.unwrap_or(false))?;
        Ok(IdentityDiagnosticRow::from(d))
    })
    .await
}

#[derive(Serialize)]
pub struct IdentityOriginRow {
    key: String,
    value: Option<String>,
    scope: Option<String>,
    origin: Option<String>,
}

#[derive(Serialize)]
pub struct CommitDiagnosticRow {
    state: String,
    summary: String,
    detail: Option<String>,
    effective_name: Option<String>,
    effective_email: Option<String>,
    expected_name: Option<String>,
    expected_email: Option<String>,
    author_context_name: Option<String>,
    author_context_email: Option<String>,
    origins: Vec<IdentityOriginRow>,
}

#[derive(Serialize)]
pub struct PushDestinationRow {
    remote_name: String,
    label: String,
    host: Option<String>,
    owner: Option<String>,
    repo: Option<String>,
    schemes: Vec<String>,
    is_origin: bool,
}

#[derive(Serialize)]
pub struct PushDiagnosticRow {
    destination_state: String,
    destination_summary: String,
    selected_remote: Option<String>,
    available_remotes: Vec<String>,
    destinations: Vec<PushDestinationRow>,
    auth_state: String,
    auth_summary: String,
    bound_account_login: Option<String>,
    token_login: Option<String>,
    token_push_permission: Option<bool>,
    token_message: Option<String>,
    evidence: Vec<String>,
}

#[derive(Serialize)]
pub struct IdentityDiagnosticRow {
    binding_id: String,
    exists: bool,
    commit: CommitDiagnosticRow,
    push: PushDiagnosticRow,
}

impl From<IdentityDiagnostic> for IdentityDiagnosticRow {
    fn from(d: IdentityDiagnostic) -> Self {
        Self {
            binding_id: d.binding_id,
            exists: d.exists,
            commit: CommitDiagnosticRow::from(d.commit),
            push: PushDiagnosticRow::from(d.push),
        }
    }
}

impl From<CommitDiagnostic> for CommitDiagnosticRow {
    fn from(c: CommitDiagnostic) -> Self {
        Self {
            state: c.state,
            summary: c.summary,
            detail: c.detail,
            effective_name: c.effective_name,
            effective_email: c.effective_email,
            expected_name: c.expected_name,
            expected_email: c.expected_email,
            author_context_name: c.author_context_name,
            author_context_email: c.author_context_email,
            origins: c
                .origins
                .into_iter()
                .map(|o| IdentityOriginRow {
                    key: o.key,
                    value: o.value,
                    scope: o.scope,
                    origin: o.origin,
                })
                .collect(),
        }
    }
}

impl From<PushDiagnostic> for PushDiagnosticRow {
    fn from(p: PushDiagnostic) -> Self {
        Self {
            destination_state: p.destination_state,
            destination_summary: p.destination_summary,
            selected_remote: p.selected_remote,
            available_remotes: p.available_remotes,
            destinations: p
                .destinations
                .into_iter()
                .map(|d| PushDestinationRow {
                    remote_name: d.remote_name,
                    label: d.label,
                    host: d.host,
                    owner: d.owner,
                    repo: d.repo,
                    schemes: d.schemes,
                    is_origin: d.is_origin,
                })
                .collect(),
            auth_state: p.auth_state,
            auth_summary: p.auth_summary,
            bound_account_login: p.bound_account_login,
            token_login: p.token_login,
            token_push_permission: p.token_push_permission,
            token_message: p.token_message,
            evidence: p.evidence,
        }
    }
}

/// Finds unbound GitHub clones in common folders (or `roots`), within a ~2.5 s budget.
#[tauri::command]
pub async fn repos_discover(roots: Option<Vec<String>>) -> Result<DiscoverResult, String> {
    blocking(move || {
        let started = Instant::now();
        let cfg = load_config()?;
        let candidates: Vec<PathBuf> = match roots {
            Some(r) if r.len() > MAX_DISCOVER_ROOTS => {
                return Err(format!("Escolha no máximo {MAX_DISCOVER_ROOTS} pastas de cada vez."));
            }
            Some(r) => r.iter().map(|s| expand_home(s)).filter(|p| p.is_absolute()).collect(),
            None => {
                let home = dirs_home().ok_or_else(|| "Não foi possível encontrar a sua pasta pessoal.".to_string())?;
                default_discover_roots(&home)
            }
        };
        let scanned: Vec<PathBuf> = candidates.into_iter().filter(|p| p.is_dir()).collect();
        let (found, truncated) =
            discover_unbound_repos(&cfg, &scanned, DISCOVER_MAX_DEPTH, Some(started + DISCOVER_BUDGET));
        let repos = found
            .into_iter()
            .map(|d| {
                let s = d.suggestion.as_ref();
                DiscoveredRow {
                    remote_url: github_https_url(&d.github_owner, &d.github_repo),
                    root_path: d.root_path.display().to_string(),
                    github_owner: d.github_owner,
                    github_repo: d.github_repo,
                    suggested_account_id: s.map(|s| s.account_id.to_string()),
                    suggestion_reason: s.map(|s| s.reason),
                    suggestion_label: s.and_then(|s| s.rule_label.clone()),
                }
            })
            .collect();
        Ok(DiscoverResult {
            repos,
            roots: scanned.iter().map(|p| p.display().to_string()).collect(),
            truncated,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    })
    .await
}

/// Folder picker only (e.g. "Procurar noutra pasta"). `None` when cancelled.
#[tauri::command]
pub async fn repos_pick_folder() -> Result<Option<String>, String> {
    blocking(|| Ok(pick_folder("Escolha uma pasta onde procurar repositórios")?.map(|p| p.display().to_string()))).await
}

static MAP_RUNNING: AtomicBool = AtomicBool::new(false);

/// Only one mapping at a time: each one walks the disk and calls GitHub for every account.
struct MapRunGuard;

impl MapRunGuard {
    fn acquire() -> Result<Self, String> {
        if MAP_RUNNING.swap(true, Ordering::SeqCst) {
            return Err("Já existe um mapeamento em andamento. Espere ele terminar.".to_string());
        }
        Ok(Self)
    }
}

impl Drop for MapRunGuard {
    fn drop(&mut self) {
        MAP_RUNNING.store(false, Ordering::SeqCst);
    }
}

fn map_folder(app: &AppHandle, raw_root: &str) -> Result<MapResult, String> {
    let started = Instant::now();
    let root = resolve_map_root(raw_root, dirs_home().as_deref())?;
    let cfg = load_config()?;
    let emit = |phase: &'static str, found: usize| {
        let _ = app.emit(MAP_PROGRESS_EVENT, MapProgress { phase, found });
    };
    emit("scanning", 0);

    let (scan, projects, fetched) = std::thread::scope(|s| {
        // Each account's list is fetched once, in parallel, while the disk is scanned.
        let fetches: Vec<_> = cfg
            .accounts
            .iter()
            .map(|a| {
                let id = a.id;
                (id, s.spawn(move || list_account_repos(id)))
            })
            .collect();

        let mut last = Instant::now();
        let scan = scan_projects(&root, MAP_MAX_DEPTH, MAP_MAX_PROJECTS, started + MAP_BUDGET, |n| {
            if last.elapsed() >= MAP_PROGRESS_EVERY {
                last = Instant::now();
                emit("scanning", n);
            }
        });
        emit("scanning", scan.projects.len());
        let projects: Vec<(PathBuf, Vec<(String, String)>)> =
            scan.projects.iter().map(|p| (p.clone(), read_remotes(p))).collect();

        emit("github", projects.len());
        let fetched: Vec<(Uuid, Result<Vec<GithubRepoSummary>, String>)> = fetches
            .into_iter()
            .map(|(id, h)| {
                let res = h
                    .join()
                    .unwrap_or_else(|_| Err("Algo deu errado ao consultar o GitHub. Tente de novo.".to_string()));
                (id, res)
            })
            .collect();
        (scan, projects, fetched)
    });

    emit("matching", projects.len());
    let accounts = fetched
        .iter()
        .map(|(id, res)| MapAccountRow {
            account_id: id.to_string(),
            ok: res.is_ok(),
            error: res.as_ref().err().cloned(),
            repo_count: res.as_ref().map(Vec::len).unwrap_or(0),
        })
        .collect();
    let lists: Vec<(Uuid, Vec<GithubRepoSummary>)> =
        fetched.into_iter().filter_map(|(id, res)| res.ok().map(|v| (id, v))).collect();
    let items = map_projects(&cfg, &lists, &projects);

    Ok(MapResult {
        root: root.display().to_string(),
        items,
        accounts,
        truncated: scan.truncated(),
        limit_reached: scan.limit_reached,
        max_projects: MAP_MAX_PROJECTS,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// "Mapear pasta": every Git project under `root` (absolute or `~/…`), classified against the config
/// and each connected account's GitHub repos. Emits `repos-map-progress` while running. Binding is
/// done afterwards with `repos_bind_many`.
#[tauri::command]
pub async fn repos_map_folder(app: AppHandle, root: String) -> Result<MapResult, String> {
    let guard = MapRunGuard::acquire()?;
    blocking(move || {
        let _guard = guard;
        map_folder(&app, &root)
    })
    .await
}

#[tauri::command]
pub fn repos_open_in_finder(path: String) -> Result<(), String> {
    open_with(&[], &bound_repo_root(&path)?)
}

#[tauri::command]
pub fn repos_open_in_terminal(path: String) -> Result<(), String> {
    open_with(&["-a", "Terminal"], &bound_repo_root(&path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_tests::{expect, keys};

    fn row() -> RepoRow {
        RepoRow {
            id: String::new(),
            account_id: String::new(),
            account_login: None,
            account_label: None,
            account_missing: false,
            root_path: String::new(),
            github_owner: None,
            github_repo: None,
            remote_url: None,
            remote_has_credentials: false,
            exists: true,
            branch: None,
            detached_at: None,
            identity: IdentityRow {
                state: Some(IdentityState::NotSet),
                local_name: None,
                local_email: None,
                expected_name: None,
                expected_email: None,
            },
            sync: Some(SyncSummary { ahead: 1, behind: 0, has_upstream: true, dirty: false }),
        }
    }

    /// api-repos.ts: RepoRow, IdentityDiagnosticRow, BindOutcome, …
    #[test]
    fn response_keys_match_typescript() {
        let r = row();
        assert_eq!(
            keys(&r),
            expect(&[
                "id", "account_id", "account_login", "account_label", "account_missing", "root_path", "github_owner",
                "github_repo", "remote_url", "remote_has_credentials", "exists", "branch", "detached_at", "identity",
                "sync"
            ])
        );
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(keys(&r.identity), expect(&["state", "local_name", "local_email", "expected_name", "expected_email"]));
        assert_eq!(v["identity"]["state"], "not_set");
        assert_eq!(keys(&v["sync"]), expect(&["ahead", "behind", "has_upstream", "dirty"]));

        assert_eq!(keys(&BindOutcome::Cancelled), expect(&["status"]));
        assert_eq!(keys(&BindOutcome::Bound { repo: row(), reason: Some(SuggestionReason::OnlyAccount) }), expect(&["status", "repo", "reason"]));
        let needs = BindOutcome::NeedsAccount {
            root_path: String::new(),
            github_owner: String::new(),
            github_repo: String::new(),
            suggested_account_id: None,
            candidates: vec![CandidateRow { account_id: String::new(), can_push: None }],
        };
        assert_eq!(
            keys(&needs),
            expect(&["status", "root_path", "github_owner", "github_repo", "suggested_account_id", "candidates"])
        );
        let n = serde_json::to_value(&needs).unwrap();
        assert_eq!(n["status"], "needs_account");
        assert_eq!(keys(&CandidateRow { account_id: String::new(), can_push: Some(true) }), expect(&["account_id", "can_push"]));

        let diag = IdentityDiagnosticRow {
            binding_id: String::new(),
            exists: true,
            commit: CommitDiagnosticRow {
                state: "correct".into(),
                summary: String::new(),
                detail: None,
                effective_name: None,
                effective_email: None,
                expected_name: None,
                expected_email: None,
                author_context_name: None,
                author_context_email: None,
                origins: vec![IdentityOriginRow {
                    key: "user.name".into(),
                    value: Some("Ana".into()),
                    scope: Some("local".into()),
                    origin: None,
                }],
            },
            push: PushDiagnosticRow {
                destination_state: "ok".into(),
                destination_summary: String::new(),
                selected_remote: Some("origin".into()),
                available_remotes: vec!["origin".into()],
                destinations: vec![PushDestinationRow {
                    remote_name: "origin".into(),
                    label: "github.com/o/r".into(),
                    host: Some("github.com".into()),
                    owner: Some("o".into()),
                    repo: Some("r".into()),
                    schemes: vec!["https".into()],
                    is_origin: true,
                }],
                auth_state: "gitpass_routed".into(),
                auth_summary: String::new(),
                bound_account_login: None,
                token_login: None,
                token_push_permission: None,
                token_message: None,
                evidence: Vec::new(),
            },
        };
        assert_eq!(
            keys(&diag),
            expect(&["binding_id", "commit", "exists", "push"])
        );
        assert_eq!(
            keys(&diag.commit),
            expect(&[
                "author_context_email", "author_context_name", "detail", "effective_email", "effective_name", "expected_email",
                "expected_name", "origins", "state", "summary"
            ])
        );
        assert_eq!(
            keys(&diag.push),
            expect(&[
                "auth_state", "auth_summary", "available_remotes", "bound_account_login", "destination_state",
                "destination_summary", "destinations", "evidence", "selected_remote", "token_login", "token_message",
                "token_push_permission"
            ])
        );

        assert_eq!(keys(&BindItemResult { path: String::new(), repo: None, error: None }), expect(&["path", "repo", "error"]));
        assert_eq!(
            keys(&DiscoveredRow {
                root_path: String::new(),
                github_owner: String::new(),
                github_repo: String::new(),
                remote_url: String::new(),
                suggested_account_id: None,
                suggestion_reason: None,
                suggestion_label: None,
            }),
            expect(&[
                "root_path", "github_owner", "github_repo", "remote_url", "suggested_account_id", "suggestion_reason",
                "suggestion_label"
            ])
        );
        assert_eq!(
            keys(&DiscoverResult { repos: vec![], roots: vec![], truncated: false, elapsed_ms: 0 }),
            expect(&["repos", "roots", "truncated", "elapsed_ms"])
        );
    }

    #[test]
    fn enums_match_typescript() {
        let id: Vec<_> = [IdentityState::Correct, IdentityState::Different, IdentityState::NotSet]
            .iter()
            .map(|s| serde_json::to_value(s).unwrap())
            .collect();
        assert_eq!(id, ["correct", "different", "not_set"]);
        let reasons: Vec<_> = [
            SuggestionReason::OwnerLogin,
            SuggestionReason::SameOwnerBinding,
            SuggestionReason::OnlyAccount,
            SuggestionReason::PushAccess,
            SuggestionReason::FolderRule,
            SuggestionReason::OrgRule,
            SuggestionReason::FolderOrgRule,
        ]
        .iter()
        .map(|s| serde_json::to_value(s).unwrap())
        .collect();
        assert_eq!(
            reasons,
            [
                "owner_login",
                "same_owner_binding",
                "only_account",
                "push_access",
                "folder_rule",
                "org_rule",
                "folder_org_rule"
            ]
        );
    }

    /// `repos_bind_many` receives `{ path, account_id }` exactly as api-repos.ts sends it.
    #[test]
    fn bind_request_accepts_frontend_payload() {
        let items: Vec<BindRequest> =
            serde_json::from_str(r#"[{"path":"/tmp/x","account_id":"abc"}]"#).expect("deserialize BindRequest");
        assert_eq!(items[0].path, "/tmp/x");
        assert_eq!(items[0].account_id, "abc");
    }
}
