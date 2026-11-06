//! Contas GitHub: listagem, Device Flow (ligar / voltar a ligar), perfil, email de commit, rótulo, remoção e estado do token.

use gitpass_core::bindings::apply_repo_identity;
use gitpass_core::github_user::{
    check_token, fetch_profile, is_noreply_email, missing_required_scopes, parse_scopes,
    selectable_emails, GithubProfile, TokenCheck,
};
use gitpass_core::{
    github_oauth_device_poll_once, github_oauth_device_request_code, github_oauth_refresh,
    github_token_delete, github_token_load, github_token_save, normalize_owner,
    resolve_oauth_client_id, Account, AppConfig, AuthMode, ConfigError, DeviceCodeStart,
    DevicePollOutcome, GithubApiError, GithubTokenBundle, OauthError, SecretError,
    DEFAULT_GITHUB_SCOPES,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const CONFIG_CHANGED_EVENT: &str = "gitpass-config-changed";
const OAUTH_UNAVAILABLE: &str = "A ligação ao GitHub não está disponível nesta versão da app.";
/// accounts.ts matches the substring «contactar o GitHub» to keep polling through transient network errors.
const NETWORK_ERROR: &str =
    "Não foi possível contactar o GitHub. Verifique a ligação à Internet e tente novamente.";
const LABEL_MAX_CHARS: usize = 64;

#[derive(Debug, Serialize)]
pub struct AccountInfo {
    id: String,
    login: Option<String>,
    label: Option<String>,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
    auth: String,
    has_token: bool,
    /// `None` until [`accounts_check`] runs (the list never calls the network).
    token_ok: Option<bool>,
    scopes: Vec<String>,
    missing_scopes: Vec<String>,
    repos_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DevicePollResult {
    Pending,
    SlowDown {
        interval_secs: u64,
    },
    Done {
        message: String,
        account_id: String,
        login: String,
        reconnected: bool,
        /// Set when the token was saved but the profile (name/email) could not be fetched.
        warning: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenState {
    Ok,
    Revoked,
    Missing,
    Offline,
    Error,
}

#[derive(Debug, Serialize)]
pub struct AccountCheck {
    account_id: String,
    state: TokenState,
    scopes: Vec<String>,
    missing_scopes: Vec<String>,
    needs_reconnect: bool,
    message: String,
}

#[derive(Debug, Serialize)]
pub struct EmailOption {
    email: String,
    primary: bool,
    noreply: bool,
    selected: bool,
}

#[derive(Debug, Serialize)]
pub struct AccountEmails {
    emails: Vec<EmailOption>,
    current: Option<String>,
    /// The token lacks `user:email`: only the public/noreply addresses are listed.
    needs_reconnect: bool,
}

#[derive(Debug, Serialize)]
pub struct IdentityUpdate {
    account: AccountInfo,
    repos_updated: usize,
    /// Paths of bound repos where `git config user.*` could not be written.
    repos_failed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RemoveResult {
    removed_bindings: usize,
}

// --- helpers ---

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Erro interno da app: {e}"))?
}

fn notify(app: &AppHandle) {
    let _ = app.emit(CONFIG_CHANGED_EVENT, ());
}

fn config_err(e: ConfigError) -> String {
    format!("Não foi possível ler ou guardar a configuração do GitPass ({e}).")
}

fn keychain_err(e: SecretError) -> String {
    format!(
        "Não foi possível aceder ao Keychain ({e}). Confirme que o GitPass tem acesso ao Keychain e tente novamente."
    )
}

fn api_err(e: GithubApiError) -> String {
    match e {
        GithubApiError::Http { status: 401, .. } => {
            "O GitHub recusou o acesso desta conta (token revogado ou expirado). Use «Voltar a ligar» para autorizar de novo.".into()
        }
        GithubApiError::Http { status: 403, body } if body.to_ascii_lowercase().contains("rate limit") => {
            "O GitHub limitou temporariamente os pedidos. Tente novamente daqui a alguns minutos.".into()
        }
        GithubApiError::Http { status, .. } => {
            format!("O GitHub respondeu com um erro (HTTP {status}). Tente novamente mais tarde.")
        }
        GithubApiError::Reqwest(_) => NETWORK_ERROR.into(),
        GithubApiError::Json(_) => "Resposta inesperada do GitHub. Tente novamente mais tarde.".into(),
        GithubApiError::Keychain(m) => format!("Não foi possível aceder ao Keychain ({m})."),
    }
}

fn oauth_err(e: OauthError) -> String {
    match e {
        OauthError::Reqwest(_) | OauthError::Io(_) => NETWORK_ERROR.into(),
        OauthError::Msg(m) => {
            let code = m.split(':').next().unwrap_or("").trim();
            match code {
                "expired_token" => "O código expirou. Peça um novo código para tentar outra vez.".into(),
                "access_denied" => {
                    "A autorização foi recusada no GitHub. Pode tentar novamente quando quiser.".into()
                }
                "incorrect_device_code" | "unsupported_grant_type" => {
                    "Este código já não é válido. Peça um novo código.".into()
                }
                "device_flow_disabled" | "incorrect_client_credentials" | "unauthorized_client" => {
                    OAUTH_UNAVAILABLE.into()
                }
                _ if m.starts_with("device code:") => OAUTH_UNAVAILABLE.into(),
                _ => "O GitHub não concluiu a autorização. Tente novamente.".into(),
            }
        }
        OauthError::Json(_) => "Resposta inesperada do GitHub. Tente novamente mais tarde.".into(),
        OauthError::Secret(e) => keychain_err(e),
        OauthError::Url(_) => "O GitHub não concluiu a autorização. Tente novamente.".into(),
    }
}

fn parse_id(account_id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(account_id).map_err(|_| "Identificador de conta inválido.".to_string())
}

fn load_cfg() -> Result<AppConfig, String> {
    AppConfig::load_or_default().map_err(config_err)
}

fn account_index(cfg: &AppConfig, id: Uuid) -> Result<usize, String> {
    cfg.accounts
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| "Esta conta já não existe. Atualize a lista de contas.".to_string())
}

fn load_bundle(id: Uuid) -> Result<GithubTokenBundle, String> {
    github_token_load(id).map_err(keychain_err)?.ok_or_else(|| {
        "O token desta conta não está no Keychain. Use «Voltar a ligar» para autorizar de novo."
            .to_string()
    })
}

fn stored_scopes(bundle: Option<&GithubTokenBundle>) -> Vec<String> {
    bundle
        .and_then(|b| b.scope.as_deref())
        .map(parse_scopes)
        .unwrap_or_default()
}

fn account_row(
    cfg: &AppConfig,
    a: &Account,
    bundle: Option<&GithubTokenBundle>,
    token_ok: Option<bool>,
    live_scopes: Option<Vec<String>>,
) -> AccountInfo {
    let scopes = live_scopes.unwrap_or_else(|| stored_scopes(bundle));
    let missing_scopes = if bundle.is_some() && !scopes.is_empty() {
        missing_required_scopes(&scopes)
    } else {
        Vec::new()
    };
    AccountInfo {
        id: a.id.to_string(),
        login: a.github_login.clone(),
        label: a.label.clone(),
        name: a.name.clone(),
        email: a.email.clone(),
        avatar_url: a.avatar_url.clone(),
        auth: format!("{:?}", a.auth).to_lowercase(),
        has_token: bundle.is_some(),
        token_ok,
        scopes,
        missing_scopes,
        repos_count: cfg.repos.iter().filter(|r| r.account_id == a.id).count(),
    }
}

/// Exchanges the refresh token (device-flow tokens only) and stores the new bundle.
fn try_refresh(account_id: Uuid, bundle: &GithubTokenBundle) -> Option<GithubTokenBundle> {
    if !bundle.oauth_via_device {
        return None;
    }
    let refresh = bundle.refresh_token.as_deref()?;
    let client_id = bundle
        .oauth_client_id
        .clone()
        .or_else(|| resolve_oauth_client_id().ok())?;
    let fresh = github_oauth_refresh(&client_id, None, refresh, true).ok()?;
    github_token_save(account_id, &fresh).ok()?;
    Some(fresh)
}

/// Fetches the profile, retrying once with a refreshed token on 401.
fn profile_with_refresh(
    account_id: Uuid,
    bundle: GithubTokenBundle,
) -> Result<(GithubProfile, GithubTokenBundle), String> {
    match fetch_profile(&bundle.access_token) {
        Ok(p) => Ok((p, bundle)),
        Err(GithubApiError::Http { status: 401, .. }) => {
            let fresh = try_refresh(account_id, &bundle).ok_or_else(|| {
                api_err(GithubApiError::Http {
                    status: 401,
                    body: String::new(),
                })
            })?;
            let p = fetch_profile(&fresh.access_token).map_err(api_err)?;
            Ok((p, fresh))
        }
        Err(e) => Err(api_err(e)),
    }
}

/// Copies name/avatar/login from GitHub; keeps the chosen email while it is still selectable.
/// Returns true when the Git identity (name or email) changed.
fn apply_profile(acc: &mut Account, p: &GithubProfile) -> bool {
    let before = (acc.name.clone(), acc.email.clone(), acc.github_login.clone());
    acc.github_login = Some(p.user.login.clone());
    acc.name = p.user.name.clone().filter(|n| !n.trim().is_empty());
    acc.avatar_url = p.user.avatar_url.clone();
    let keep_email = match (&acc.email, &p.emails) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(cur), Some(_)) => selectable_emails(&p.user, p.emails.as_deref())
            .iter()
            .any(|o| o.email.eq_ignore_ascii_case(cur)),
    };
    if !keep_email {
        acc.email = Some(p.commit_email.clone());
    }
    before != (acc.name.clone(), acc.email.clone(), acc.github_login.clone())
}

/// Writes the account's `user.name` / `user.email` into every bound repo that still exists.
fn reapply_identity(cfg: &AppConfig, account_id: Uuid) -> (usize, Vec<String>) {
    let Some(acc) = cfg.accounts.iter().find(|a| a.id == account_id) else {
        return (0, Vec::new());
    };
    let mut ok = 0;
    let mut failed = Vec::new();
    for r in cfg.repos.iter().filter(|r| r.account_id == account_id) {
        if !r.root_path.exists() {
            continue;
        }
        match apply_repo_identity(&r.root_path, acc) {
            Ok(_) => ok += 1,
            Err(_) => failed.push(r.root_path.display().to_string()),
        }
    }
    (ok, failed)
}

/// Keeps the stored scope list in sync with what GitHub reports (the list view reads it without network).
fn sync_bundle_scopes(account_id: Uuid, mut bundle: GithubTokenBundle, scopes: &[String]) {
    if scopes.is_empty() || stored_scopes(Some(&bundle)) == scopes {
        return;
    }
    bundle.scope = Some(scopes.join(","));
    let _ = github_token_save(account_id, &bundle);
}

fn find_existing(cfg: &AppConfig, login: &str, github_id: Option<u64>) -> Option<Uuid> {
    let n = normalize_owner(login);
    if let Some(a) = cfg.accounts.iter().find(|a| {
        a.github_login
            .as_deref()
            .map(|l| normalize_owner(l) == n)
            .unwrap_or(false)
    }) {
        return Some(a.id);
    }
    // Login renamed on GitHub: the noreply address still carries the numeric user id.
    let prefix = format!("{}+", github_id?);
    cfg.accounts
        .iter()
        .find(|a| {
            a.email
                .as_deref()
                .map(|e| is_noreply_email(e) && e.starts_with(&prefix))
                .unwrap_or(false)
        })
        .map(|a| a.id)
}

// --- commands ---

#[tauri::command]
pub async fn accounts_list() -> Result<Vec<AccountInfo>, String> {
    blocking(|| {
        let cfg = load_cfg()?;
        Ok(cfg
            .accounts
            .iter()
            .map(|a| {
                let bundle = github_token_load(a.id).ok().flatten();
                account_row(&cfg, a, bundle.as_ref(), None, None)
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn accounts_device_start() -> Result<DeviceCodeStart, String> {
    blocking(|| {
        let client_id = resolve_oauth_client_id().map_err(|_| OAUTH_UNAVAILABLE.to_string())?;
        let scopes = std::env::var("GITPASS_OAUTH_SCOPES")
            .unwrap_or_else(|_| DEFAULT_GITHUB_SCOPES.to_string());
        let start = github_oauth_device_request_code(&client_id, &scopes).map_err(oauth_err)?;
        // The modal also offers an «Abrir GitHub» button, so a failure here is not fatal.
        let _ = opener::open(&start.verification_uri);
        Ok(start)
    })
    .await
}

#[tauri::command]
pub async fn accounts_device_poll(
    app: AppHandle,
    client_id: String,
    device_code: String,
) -> Result<DevicePollResult, String> {
    let res = blocking(move || {
        match github_oauth_device_poll_once(&client_id, &device_code).map_err(oauth_err)? {
            DevicePollOutcome::Pending => Ok(DevicePollResult::Pending),
            DevicePollOutcome::SlowDown { interval_secs } => {
                Ok(DevicePollResult::SlowDown { interval_secs })
            }
            DevicePollOutcome::Success { bundle, login } => finish_login(bundle, login),
        }
    })
    .await?;
    if matches!(res, DevicePollResult::Done { .. }) {
        notify(&app);
    }
    Ok(res)
}

fn finish_login(mut bundle: GithubTokenBundle, login: String) -> Result<DevicePollResult, String> {
    let (profile, warning) = match fetch_profile(&bundle.access_token) {
        Ok(p) => (Some(p), None),
        Err(_) => (
            None,
            Some("Não foi possível obter o nome e o email do perfil. Use «Atualizar perfil» mais tarde.".to_string()),
        ),
    };
    let login = profile.as_ref().map(|p| p.user.login.clone()).unwrap_or(login);
    if let Some(p) = &profile {
        if bundle.scope.as_deref().map(str::is_empty).unwrap_or(true) && !p.scopes.is_empty() {
            bundle.scope = Some(p.scopes.join(","));
        }
    }

    let mut cfg = load_cfg()?;
    let existing = find_existing(&cfg, &login, profile.as_ref().map(|p| p.user.id));
    let reconnected = existing.is_some();
    let id = existing.unwrap_or_else(Uuid::new_v4);

    github_token_save(id, &bundle).map_err(keychain_err)?;
    if !reconnected {
        cfg.accounts.push(Account {
            id,
            github_login: Some(login.clone()),
            auth: AuthMode::Https,
            ..Default::default()
        });
    }
    let idx = account_index(&cfg, id)?;
    let identity_changed = match &profile {
        Some(p) => apply_profile(&mut cfg.accounts[idx], p),
        None => {
            cfg.accounts[idx].github_login = Some(login.clone());
            false
        }
    };
    if let Err(e) = cfg.save() {
        if !reconnected {
            let _ = github_token_delete(id);
        }
        return Err(config_err(e));
    }
    if reconnected && identity_changed {
        reapply_identity(&cfg, id);
    }

    let message = if reconnected {
        format!("Conta @{login} ligada de novo. Token e perfil atualizados.")
    } else {
        format!("Conta @{login} ligada. O token ficou guardado no Keychain.")
    };
    Ok(DevicePollResult::Done {
        message,
        account_id: id.to_string(),
        login,
        reconnected,
        warning,
    })
}

#[tauri::command]
pub async fn accounts_refresh_profile(app: AppHandle, account_id: String) -> Result<AccountInfo, String> {
    let row = blocking(move || {
        let id = parse_id(&account_id)?;
        let mut cfg = load_cfg()?;
        let idx = account_index(&cfg, id)?;
        let (profile, bundle) = profile_with_refresh(id, load_bundle(id)?)?;
        let changed = apply_profile(&mut cfg.accounts[idx], &profile);
        cfg.save().map_err(config_err)?;
        if changed {
            reapply_identity(&cfg, id);
        }
        sync_bundle_scopes(id, bundle.clone(), &profile.scopes);
        let scopes = (!profile.scopes.is_empty()).then(|| profile.scopes.clone());
        Ok(account_row(&cfg, &cfg.accounts[idx], Some(&bundle), Some(true), scopes))
    })
    .await?;
    notify(&app);
    Ok(row)
}

#[tauri::command]
pub async fn accounts_set_label(
    app: AppHandle,
    account_id: String,
    label: String,
) -> Result<AccountInfo, String> {
    let row = blocking(move || {
        let id = parse_id(&account_id)?;
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        if label.chars().count() > LABEL_MAX_CHARS {
            return Err(format!("O rótulo pode ter no máximo {LABEL_MAX_CHARS} caracteres."));
        }
        let mut cfg = load_cfg()?;
        let idx = account_index(&cfg, id)?;
        cfg.accounts[idx].label = (!label.is_empty()).then_some(label);
        cfg.save().map_err(config_err)?;
        let bundle = github_token_load(id).ok().flatten();
        Ok(account_row(&cfg, &cfg.accounts[idx], bundle.as_ref(), None, None))
    })
    .await?;
    notify(&app);
    Ok(row)
}

#[tauri::command]
pub async fn accounts_emails(account_id: String) -> Result<AccountEmails, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let cfg = load_cfg()?;
        let current = cfg.accounts[account_index(&cfg, id)?].email.clone();
        let (profile, _) = profile_with_refresh(id, load_bundle(id)?)?;
        let emails = selectable_emails(&profile.user, profile.emails.as_deref())
            .into_iter()
            .map(|e| EmailOption {
                selected: current
                    .as_deref()
                    .map(|c| c.eq_ignore_ascii_case(&e.email))
                    .unwrap_or(false),
                noreply: is_noreply_email(&e.email),
                primary: e.primary,
                email: e.email,
            })
            .collect();
        Ok(AccountEmails {
            emails,
            current,
            needs_reconnect: profile.emails.is_none(),
        })
    })
    .await
}

#[tauri::command]
pub async fn accounts_set_email(
    app: AppHandle,
    account_id: String,
    email: String,
) -> Result<IdentityUpdate, String> {
    let res = blocking(move || {
        let id = parse_id(&account_id)?;
        let wanted = email.trim().to_string();
        let mut cfg = load_cfg()?;
        let idx = account_index(&cfg, id)?;
        let (profile, bundle) = profile_with_refresh(id, load_bundle(id)?)?;
        let chosen = selectable_emails(&profile.user, profile.emails.as_deref())
            .into_iter()
            .find(|o| o.email.eq_ignore_ascii_case(&wanted))
            .ok_or_else(|| {
                "Este email não está verificado nesta conta GitHub. Escolha um dos emails da lista."
                    .to_string()
            })?;
        cfg.accounts[idx].email = Some(chosen.email);
        cfg.save().map_err(config_err)?;
        let (repos_updated, repos_failed) = reapply_identity(&cfg, id);
        Ok(IdentityUpdate {
            account: account_row(&cfg, &cfg.accounts[idx], Some(&bundle), Some(true), None),
            repos_updated,
            repos_failed,
        })
    })
    .await?;
    notify(&app);
    Ok(res)
}

#[tauri::command]
pub async fn accounts_remove(app: AppHandle, account_id: String) -> Result<RemoveResult, String> {
    let res = blocking(move || {
        let id = parse_id(&account_id)?;
        let mut cfg = load_cfg()?;
        account_index(&cfg, id)?;
        github_token_delete(id).map_err(keychain_err)?;

        let removed: Vec<Uuid> = cfg
            .repos
            .iter()
            .filter(|r| r.account_id == id)
            .map(|r| r.id)
            .collect();
        cfg.repos.retain(|r| r.account_id != id);
        cfg.accounts.retain(|a| a.id != id);
        let ui = &mut cfg.ui;
        if ui.pinned_repo_binding_id.is_some_and(|b| removed.contains(&b)) {
            ui.pinned_repo_binding_id = None;
        }
        if ui.last_active_repo_binding_id.is_some_and(|b| removed.contains(&b)) {
            ui.last_active_repo_binding_id = None;
        }
        cfg.save().map_err(config_err)?;
        Ok(RemoveResult {
            removed_bindings: removed.len(),
        })
    })
    .await?;
    notify(&app);
    Ok(res)
}

#[tauri::command]
pub async fn accounts_check(account_id: String) -> Result<AccountCheck, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let cfg = load_cfg()?;
        account_index(&cfg, id)?;
        let done = |state, scopes: Vec<String>, needs_reconnect, message: &str| AccountCheck {
            account_id: id.to_string(),
            state,
            missing_scopes: if scopes.is_empty() { Vec::new() } else { missing_required_scopes(&scopes) },
            scopes,
            needs_reconnect,
            message: message.to_string(),
        };

        let Some(mut bundle) = github_token_load(id).map_err(keychain_err)? else {
            return Ok(done(
                TokenState::Missing,
                Vec::new(),
                true,
                "O token desta conta não está no Keychain. Volte a ligar a conta.",
            ));
        };

        let mut result = check_token(&bundle.access_token);
        if matches!(result, Ok(TokenCheck::Revoked)) {
            if let Some(fresh) = try_refresh(id, &bundle) {
                result = check_token(&fresh.access_token);
                bundle = fresh;
            }
        }

        match result {
            Ok(TokenCheck::Valid { scopes }) => {
                let scopes = if scopes.is_empty() { stored_scopes(Some(&bundle)) } else { scopes };
                sync_bundle_scopes(id, bundle, &scopes);
                let missing = missing_required_scopes(&scopes);
                if missing.is_empty() || scopes.is_empty() {
                    Ok(done(TokenState::Ok, scopes, false, "Token válido."))
                } else {
                    Ok(done(
                        TokenState::Ok,
                        scopes,
                        true,
                        "Faltam permissões no GitHub. Volte a ligar a conta para dar permissões.",
                    ))
                }
            }
            Ok(TokenCheck::Revoked) => Ok(done(
                TokenState::Revoked,
                Vec::new(),
                true,
                "O acesso desta conta foi revogado no GitHub. Volte a ligar a conta.",
            )),
            Err(GithubApiError::Reqwest(_)) => Ok(done(
                TokenState::Offline,
                stored_scopes(Some(&bundle)),
                false,
                "Sem ligação ao GitHub: não foi possível verificar o token.",
            )),
            Err(e) => Ok(done(TokenState::Error, stored_scopes(Some(&bundle)), false, &api_err(e))),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_tests::{expect, keys};

    fn info() -> AccountInfo {
        AccountInfo {
            id: String::new(),
            login: None,
            label: None,
            name: None,
            email: None,
            avatar_url: None,
            auth: String::new(),
            has_token: false,
            token_ok: None,
            scopes: vec![],
            missing_scopes: vec![],
            repos_count: 0,
        }
    }

    /// api-accounts.ts: AccountInfo, AccountDevicePoll, AccountCheck, EmailOption, AccountEmails, IdentityUpdate.
    #[test]
    fn response_keys_match_typescript() {
        assert_eq!(
            keys(&info()),
            expect(&[
                "id", "login", "label", "name", "email", "avatar_url", "auth", "has_token", "token_ok", "scopes",
                "missing_scopes", "repos_count"
            ])
        );
        assert_eq!(
            keys(&AccountCheck {
                account_id: String::new(),
                state: TokenState::Ok,
                scopes: vec![],
                missing_scopes: vec![],
                needs_reconnect: false,
                message: String::new(),
            }),
            expect(&["account_id", "state", "scopes", "missing_scopes", "needs_reconnect", "message"])
        );
        assert_eq!(
            keys(&EmailOption { email: String::new(), primary: false, noreply: false, selected: false }),
            expect(&["email", "primary", "noreply", "selected"])
        );
        assert_eq!(
            keys(&AccountEmails { emails: vec![], current: None, needs_reconnect: false }),
            expect(&["emails", "current", "needs_reconnect"])
        );
        assert_eq!(
            keys(&IdentityUpdate { account: info(), repos_updated: 0, repos_failed: vec![] }),
            expect(&["account", "repos_updated", "repos_failed"])
        );
        assert_eq!(keys(&RemoveResult { removed_bindings: 0 }), expect(&["removed_bindings"]));
    }

    #[test]
    fn device_poll_variants_match_typescript() {
        assert_eq!(keys(&DevicePollResult::Pending), expect(&["status"]));
        assert_eq!(keys(&DevicePollResult::SlowDown { interval_secs: 10 }), expect(&["status", "interval_secs"]));
        let done = DevicePollResult::Done {
            message: String::new(),
            account_id: String::new(),
            login: String::new(),
            reconnected: false,
            warning: None,
        };
        assert_eq!(keys(&done), expect(&["status", "message", "account_id", "login", "reconnected", "warning"]));
        assert_eq!(serde_json::to_value(&done).unwrap()["status"], "done");
        assert_eq!(serde_json::to_value(&DevicePollResult::SlowDown { interval_secs: 1 }).unwrap()["status"], "slow_down");
    }

    #[test]
    fn token_states_match_typescript() {
        let all = [TokenState::Ok, TokenState::Revoked, TokenState::Missing, TokenState::Offline, TokenState::Error];
        let json: Vec<_> = all.iter().map(|s| serde_json::to_value(s).unwrap()).collect();
        assert_eq!(json, ["ok", "revoked", "missing", "offline", "error"]);
    }
}
