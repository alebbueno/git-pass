//! Tauri commands for this area. Registered in main.rs `generate_handler!`.
//!
//! Commands: `cloud_repos`, `cloud_clone`, `cloud_choose_folder`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use gitpass_core::clone::{clone_repo, default_clone_dir, split_full_name};
use gitpass_core::{
    embedded_oauth_client_id, github_oauth_env_credentials, github_oauth_refresh, github_token_load,
    github_token_save, list_user_repos, resolve_oauth_client_id, AppConfig, GithubApiError,
    GithubRepoSummary, GithubTokenBundle,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const PROGRESS_EVENT: &str = "gitpass-clone-progress";
const CONFIG_EVENT: &str = "gitpass-config-changed";

/// GitHub repository plus its local clone (if bound to the same account).
#[derive(Debug, Serialize)]
pub struct CloudRepo {
    #[serde(flatten)]
    pub repo: GithubRepoSummary,
    pub local_binding_id: Option<String>,
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CloneProgressPayload {
    full_name: String,
    /// `starting`, `counting`, `compressing`, `receiving`, `resolving`, `checkout` or `binding`.
    phase: String,
    percent: u8,
}

fn parse_account(account_id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(account_id).map_err(|_| "Identificador de conta inválido.".to_string())
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn load_bundle(id: Uuid) -> Result<GithubTokenBundle, String> {
    github_token_load(id)
        .map_err(|e| format!("Não foi possível ler o token do Keychain: {e}"))?
        .ok_or_else(|| "Esta conta não tem token. Liga a conta GitHub em «Contas».".to_string())
}

/// Exchanges the stored refresh token for a new bundle and saves it.
fn refresh_bundle(id: Uuid, bundle: &GithubTokenBundle) -> Result<GithubTokenBundle, String> {
    let expired_msg = "A sessão do GitHub expirou. Volta a ligar a conta em «Contas».".to_string();
    let refresh = bundle.refresh_token.as_deref().ok_or_else(|| expired_msg.clone())?;
    let new_bundle = if bundle.oauth_via_device {
        let cid = bundle
            .oauth_client_id
            .clone()
            .or_else(|| embedded_oauth_client_id().map(String::from))
            .or_else(|| resolve_oauth_client_id().ok())
            .ok_or_else(|| expired_msg.clone())?;
        github_oauth_refresh(&cid, None, refresh, true)
    } else {
        let (cid, secret) = github_oauth_env_credentials().map_err(|_| expired_msg.clone())?;
        github_oauth_refresh(&cid, Some(secret.as_str()), refresh, false)
    }
    .map_err(|_| expired_msg)?;
    github_token_save(id, &new_bundle).map_err(|e| format!("Não foi possível guardar o token no Keychain: {e}"))?;
    Ok(new_bundle)
}

/// Access token for the account, refreshed beforehand when it is about to expire.
fn current_token(id: Uuid) -> Result<String, String> {
    let bundle = load_bundle(id)?;
    let about_to_expire = bundle.expires_at_epoch.map(|e| e <= now_epoch() + 60).unwrap_or(false);
    if about_to_expire && bundle.refresh_token.is_some() {
        if let Ok(nb) = refresh_bundle(id, &bundle) {
            return Ok(nb.access_token);
        }
    }
    Ok(bundle.access_token)
}

fn api_error_message(e: GithubApiError) -> String {
    match e {
        GithubApiError::Http { status: 401, .. } => {
            "O GitHub recusou o token (sessão expirada ou revogada). Volta a ligar a conta em «Contas».".to_string()
        }
        GithubApiError::Http { status: 403, body } if body.to_lowercase().contains("rate limit") => {
            "Limite de pedidos do GitHub atingido. Tenta de novo dentro de alguns minutos.".to_string()
        }
        GithubApiError::Http { status: 403, .. } => {
            "O GitHub negou o acesso. Verifica as permissões concedidas à app GitPass (SSO/organizações).".to_string()
        }
        GithubApiError::Http { status, .. } => format!("O GitHub respondeu com erro (HTTP {status}). Tenta de novo."),
        GithubApiError::Reqwest(_) => "Sem ligação ao GitHub. Verifica a tua ligação à internet.".to_string(),
        GithubApiError::Json(_) => "Resposta inesperada do GitHub.".to_string(),
        GithubApiError::Keychain(m) => format!("Erro do Keychain: {m}"),
    }
}

fn list_repos_blocking(id: Uuid) -> Result<Vec<GithubRepoSummary>, String> {
    let bundle = load_bundle(id)?;
    let token = current_token(id)?;
    match list_user_repos(&token) {
        Ok(v) => Ok(v),
        Err(GithubApiError::Http { status: 401, .. }) => {
            // Token rejected: try once with a refreshed one.
            let nb = refresh_bundle(id, &bundle)?;
            list_user_repos(&nb.access_token).map_err(api_error_message)
        }
        Err(e) => Err(api_error_message(e)),
    }
}

/// Repositories of the account on GitHub (with permissions) and the matching local binding, if any.
#[tauri::command]
pub async fn cloud_repos(account_id: String) -> Result<Vec<CloudRepo>, String> {
    let id = parse_account(&account_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
        if !cfg.accounts.iter().any(|a| a.id == id) {
            return Err("Conta desconhecida.".to_string());
        }
        let repos = list_repos_blocking(id)?;
        Ok(repos
            .into_iter()
            .map(|repo| {
                let (owner, name) = repo.full_name.split_once('/').unwrap_or(("", repo.full_name.as_str()));
                let local = cfg.repos.iter().find(|b| {
                    b.account_id == id
                        && b.github_owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(owner))
                        && b.github_repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(name))
                });
                CloudRepo {
                    local_binding_id: local.map(|b| b.id.to_string()),
                    local_path: local.map(|b| b.root_path.display().to_string()),
                    repo,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| format!("Tarefa interrompida: {e}"))?
}

fn in_flight() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Removes the key from the in-flight set when dropped.
struct InFlightGuard(String);
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = in_flight().lock() {
            s.remove(&self.0);
        }
    }
}

fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" || p.starts_with("~/") {
        if let Some(home) = dirs_home() {
            return home.join(p.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    PathBuf::from(p)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Clones `full_name` (`owner/repo`) with the account's token, then binds the clone to the account.
///
/// `dest_parent`: folder that will contain the repo folder (`<dest_parent>/<repo>`); when absent the default
/// `~/GitHub/<owner>/<repo>` is used. Emits `gitpass-clone-progress` `{ full_name, phase, percent }` while
/// running and `gitpass-config-changed` at the end. Returns the path of the clone.
#[tauri::command]
pub async fn cloud_clone(
    app: AppHandle,
    account_id: String,
    full_name: String,
    dest_parent: Option<String>,
) -> Result<String, String> {
    let id = parse_account(&account_id)?;
    let (owner, repo) = split_full_name(&full_name)?;

    let dest: PathBuf = match dest_parent.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(parent) => {
            let parent = expand_tilde(parent);
            if !parent.is_absolute() {
                return Err("O destino tem de ser um caminho absoluto.".to_string());
            }
            parent.join(&repo)
        }
        None => default_clone_dir(&owner, &repo)?,
    };

    {
        let mut set = in_flight().lock().map_err(|_| "Estado interno inconsistente.".to_string())?;
        if !set.insert(full_name.to_lowercase()) {
            return Err(format!("`{full_name}` já está a ser clonado."));
        }
    }
    let guard = InFlightGuard(full_name.to_lowercase());

    let result = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let emit = |phase: &str, percent: u8| {
            let _ = app.emit(
                PROGRESS_EVENT,
                CloneProgressPayload {
                    full_name: full_name.clone(),
                    phase: phase.to_string(),
                    percent,
                },
            );
        };

        let mut cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
        if !cfg.accounts.iter().any(|a| a.id == id) {
            return Err("Conta desconhecida.".to_string());
        }
        if cfg.repos.iter().any(|b| {
            b.account_id == id
                && b.github_owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&owner))
                && b.github_repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(&repo))
        }) {
            return Err(format!("`{full_name}` já tem um clone local vinculado a esta conta."));
        }

        let token = current_token(id)?;
        emit("starting", 0);
        let url = format!("https://github.com/{owner}/{repo}.git");
        clone_repo(&token, &url, &dest, |p| emit(p.phase, p.percent))?;

        emit("binding", 100);
        // Reload: the config may have changed during a long clone.
        cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
        gitpass_core::bindings::create_binding(&mut cfg, id, &dest).map_err(|e| {
            format!(
                "Clonado em `{}`, mas não foi possível vincular à conta: {e}. Podes vinculá-lo em «Repositórios».",
                dest.display()
            )
        })?;
        let _ = app.emit(CONFIG_EVENT, ());
        Ok(dest.display().to_string())
    })
    .await
    .map_err(|e| format!("Tarefa interrompida: {e}"))?;

    result
}

/// Native folder picker (macOS). `Ok(None)` when the user cancels.
#[tauri::command]
pub async fn cloud_choose_folder() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(|| {
            let out = std::process::Command::new("osascript")
                .arg("-e")
                .arg(
                    r#"tell application "System Events"
  activate
  set f to choose folder with prompt "Escolhe a pasta onde clonar o repositório"
  POSIX path of f
end tell"#,
                )
                .output()
                .map_err(|e| format!("Não foi possível abrir o seletor de pastas: {e}"))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                if err.contains("-128") || err.to_lowercase().contains("cancel") {
                    return Ok(None);
                }
                return Err(format!("O seletor de pastas falhou: {}", err.trim()));
            }
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let path = if path.len() > 1 { path.trim_end_matches('/').to_string() } else { path };
            Ok((!path.is_empty()).then_some(path))
        })
        .await
        .map_err(|e| format!("Tarefa interrompida: {e}"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("O seletor de pastas só está disponível no macOS por agora.".to_string())
    }
}
