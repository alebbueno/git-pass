//! Tauri commands for this area. Registered in main.rs `generate_handler!`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gitpass_core::bindings::{
    change_binding_account, create_binding, decide_account, default_discover_roots, discover_unbound_repos,
    expected_identity, fix_binding_identity, github_https_url, identity_state, read_head, read_origin_url,
    read_repo_identity, remote_url_has_credentials, remove_binding, suggest_account_offline, AccountDecision,
    IdentityState, SuggestionReason, DISCOVER_MAX_DEPTH,
};
use gitpass_core::github_api::{token_can_push_for_account, GithubApiError};
use gitpass_core::{git_work_tree_root, parse_github_remote, AppConfig, RepoBinding};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const CONFIG_CHANGED_EVENT: &str = "gitpass-config-changed";
const DISCOVER_BUDGET: Duration = Duration::from_millis(2500);

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

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Erro interno: {e}"))?
}

fn load_config() -> Result<AppConfig, String> {
    AppConfig::load_or_default().map_err(|e| format!("Não foi possível ler a configuração do GitPass: {e}"))
}

fn parse_id(raw: &str, what: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw).map_err(|_| format!("Identificador de {what} inválido."))
}

fn notify(app: &AppHandle) {
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
    }
}

fn row_for(cfg: &AppConfig, binding_id: Uuid) -> Result<RepoRow, String> {
    cfg.repos
        .iter()
        .find(|r| r.id == binding_id)
        .map(|b| build_row(cfg, b))
        .ok_or_else(|| "Vínculo desconhecido. Atualiza a lista e tenta novamente.".to_string())
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
        "A pasta escolhida não é um repositório Git. Escolhe a pasta de um clone (com .git) ou clona o repositório no separador Nuvem.".to_string()
    })?;
    let root = root.canonicalize().unwrap_or(root);
    let url = read_origin_url(&root).ok_or_else(|| {
        "Este repositório não tem o remote `origin`. Adiciona-o (por exemplo no GitHub Desktop) e tenta novamente.".to_string()
    })?;
    let gh = parse_github_remote(&url).ok_or_else(|| {
        "O `origin` deste repositório não aponta para github.com. O GitPass só gere repositórios do GitHub.".to_string()
    })?;
    Ok(RepoTarget {
        root,
        owner: gh.owner,
        repo: gh.repo,
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
        "Esta pasta já está vinculada{who}. Para usar outra conta, escolhe «Mudar conta» no detalhe do repositório."
    ))
}

/// `create_binding` saves before applying the identity; if only the identity fails, the binding exists anyway.
fn bind(app: &AppHandle, cfg: &mut AppConfig, account: Uuid, root: &Path) -> Result<RepoBinding, String> {
    match create_binding(cfg, account, root) {
        Ok(b) => Ok(b),
        Err(e) if cfg.repos.iter().any(|r| r.root_path == root) => {
            notify(app);
            Err(format!(
                "O repositório foi vinculado, mas não foi possível aplicar a identidade Git ({e}). Use «Corrigir» no repositório."
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
    let offline = suggest_account_offline(cfg, &target.owner);
    if let Some(s) = offline.filter(|s| s.reason == SuggestionReason::OwnerLogin) {
        return (AccountDecision::Bind(s), Vec::new());
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
fn osascript(script: &str) -> Result<Result<Option<PathBuf>, String>, String> {
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
    Err("O seletor de pastas só está disponível no macOS.".to_string())
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

fn existing_dir(path: &str) -> Result<PathBuf, String> {
    let p = expand_home(path);
    match p.canonicalize() {
        Ok(c) if c.is_dir() => Ok(c),
        _ => Err(format!("A pasta {} não existe.", p.display())),
    }
}

fn open_with(args: &[&str], path: &Path) -> Result<(), String> {
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
        Err("Disponível apenas no macOS.".to_string())
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
            return Err("Liga primeiro uma conta GitHub no separador Contas.".to_string());
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
            return Err("Conta desconhecida. Atualiza a lista e tenta novamente.".to_string());
        }
        if !cfg.repos.iter().any(|r| r.id == id) {
            return Err("Vínculo desconhecido. Atualiza a lista e tenta novamente.".to_string());
        }
        let res = change_binding_account(&mut cfg, id, account);
        // Past the checks above, the switch is saved even if the identity could not be written.
        notify(&app);
        res.map_err(|e| format!("A conta foi alterada, mas a identidade Git não foi aplicada: {e}"))?;
        row_for(&cfg, id)
    })
    .await
}

#[tauri::command]
pub async fn repos_fix_identity(binding_id: String) -> Result<RepoRow, String> {
    blocking(move || {
        let id = parse_id(&binding_id, "vínculo")?;
        let cfg = load_config()?;
        fix_binding_identity(&cfg, id).map_err(|e| format!("Não foi possível corrigir a identidade: {e}"))?;
        row_for(&cfg, id)
    })
    .await
}

/// Finds unbound GitHub clones in common folders (or `roots`), within a ~2.5 s budget.
#[tauri::command]
pub async fn repos_discover(roots: Option<Vec<String>>) -> Result<DiscoverResult, String> {
    blocking(move || {
        let started = Instant::now();
        let cfg = load_config()?;
        let candidates: Vec<PathBuf> = match roots {
            Some(r) => r.iter().map(|s| expand_home(s)).collect(),
            None => {
                let home = dirs_home().ok_or_else(|| "Não foi possível determinar a pasta pessoal.".to_string())?;
                default_discover_roots(&home)
            }
        };
        let scanned: Vec<PathBuf> = candidates.into_iter().filter(|p| p.is_dir()).collect();
        let (found, truncated) =
            discover_unbound_repos(&cfg, &scanned, DISCOVER_MAX_DEPTH, Some(started + DISCOVER_BUDGET));
        let repos = found
            .into_iter()
            .map(|d| DiscoveredRow {
                remote_url: github_https_url(&d.github_owner, &d.github_repo),
                root_path: d.root_path.display().to_string(),
                github_owner: d.github_owner,
                github_repo: d.github_repo,
                suggested_account_id: d.suggestion.map(|s| s.account_id.to_string()),
                suggestion_reason: d.suggestion.map(|s| s.reason),
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

#[tauri::command]
pub fn repos_open_in_finder(path: String) -> Result<(), String> {
    open_with(&[], &existing_dir(&path)?)
}

#[tauri::command]
pub fn repos_open_in_terminal(path: String) -> Result<(), String> {
    open_with(&["-a", "Terminal"], &existing_dir(&path)?)
}
