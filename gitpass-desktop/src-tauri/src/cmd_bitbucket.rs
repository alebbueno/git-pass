//! Bitbucket Cloud: API token, OAuth (navegador) e listagem de repositórios.

use gitpass_core::bitbucket_api::{validate_for_connect, BitbucketApiError};
use gitpass_core::oauth_bitbucket::{
    bitbucket_oauth_login, resolve_bitbucket_oauth_client, DEFAULT_BITBUCKET_OAUTH_CALLBACK_PORT,
    DEFAULT_BITBUCKET_OAUTH_SCOPES,
};
use gitpass_core::oauth_github::OauthError;
use gitpass_core::{
    bitbucket_token_save, bitbucket_token_exists, bitbucket_token_load, list_bitbucket_repos,
    Account, AccountHost, AuthMode, BitbucketRepoSummary, BitbucketTokenBundle, ConfigError,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::cmd_accounts::{account_index, account_row, load_cfg, AccountInfo};

const CONFIG_CHANGED_EVENT: &str = "gitpass-config-changed";

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Algo deu errado no app: {e}"))?
}

fn config_err(e: ConfigError) -> String {
    format!("Não foi possível ler ou salvar as configurações do GitPass ({e}).")
}

fn bb_err(e: BitbucketApiError) -> String {
    match e {
        BitbucketApiError::Http { status: 401 | 403, .. } => {
            "O Bitbucket recusou o token. Use um API token da Atlassian com escopos Bitbucket (inclua read:user:bitbucket). Cole só o token ou o email Atlassian + token.".into()
        }
        BitbucketApiError::Http { status, body } => format!("O Bitbucket respondeu com erro HTTP {status}: {body}"),
        BitbucketApiError::Reqwest(_) => {
            "Não foi possível conectar ao Bitbucket. Veja se você está online e tente de novo.".into()
        }
        BitbucketApiError::Json(_) => "Resposta inesperada do Bitbucket. Tente de novo.".into(),
    }
}

fn oauth_err(e: OauthError) -> String {
    e.to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BitbucketConnectResult {
    pub account: AccountInfo,
    pub message: String,
}

fn save_bitbucket_account(
    login: String,
    display_name: Option<String>,
    label: Option<String>,
    bundle: BitbucketTokenBundle,
) -> Result<BitbucketConnectResult, String> {
    let mut cfg = load_cfg()?;
    let existing = cfg
        .accounts
        .iter()
        .find(|a| a.host == AccountHost::Bitbucket && a.github_login.as_deref() == Some(login.as_str()))
        .map(|a| a.id);
    let reconnected = existing.is_some();
    let id = existing.unwrap_or_else(Uuid::new_v4);

    bitbucket_token_save(id, &bundle).map_err(|e| format!("Não foi possível salvar o token no Keychain: {e}"))?;

    let display_name = display_name.filter(|n| !n.trim().is_empty());
    if reconnected {
        let idx = account_index(&cfg, id)?;
        cfg.accounts[idx].github_login = Some(login.clone());
        if let Some(n) = display_name {
            cfg.accounts[idx].name = Some(n);
        }
        if let Some(l) = label.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            cfg.accounts[idx].label = Some(l.to_string());
        }
    } else {
        cfg.accounts.push(Account {
            id,
            host: AccountHost::Bitbucket,
            github_login: Some(login.clone()),
            label: label
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            name: display_name,
            auth: AuthMode::Https,
            ..Default::default()
        });
    }
    cfg.save().map_err(config_err)?;
    let idx = account_index(&cfg, id)?;
    let a = &cfg.accounts[idx];
    let has = bitbucket_token_exists(id) || bitbucket_token_load(id).ok().flatten().is_some();
    let info = account_row(&cfg, a, None, has, Some(true), None);
    Ok(BitbucketConnectResult {
        account: info,
        message: if reconnected {
            format!("Conta Bitbucket @{login} reconectada.")
        } else {
            format!("Conta Bitbucket @{login} conectada.")
        },
    })
}

#[tauri::command]
pub async fn accounts_bitbucket_connect(
    app: AppHandle,
    username: String,
    app_password: String,
    label: Option<String>,
) -> Result<BitbucketConnectResult, String> {
    let res = blocking(move || {
        let email = username.trim();
        let email_opt = if email.is_empty() { None } else { Some(email) };
        let user = validate_for_connect(email_opt, &app_password).map_err(bb_err)?;
        let login = user.username.clone();
        let bundle = BitbucketTokenBundle {
            app_password: app_password.trim().to_string(),
            via_oauth: false,
            refresh_token: None,
            expires_at_epoch: None,
        };
        save_bitbucket_account(login, user.display_name, label, bundle)
    })
    .await?;
    let _ = app.emit(CONFIG_CHANGED_EVENT, ());
    Ok(res)
}

#[tauri::command]
pub async fn accounts_bitbucket_oauth_connect(app: AppHandle) -> Result<BitbucketConnectResult, String> {
    let res = blocking(move || {
        let (client_id, client_secret) = resolve_bitbucket_oauth_client()?;
        let (bundle, login) = bitbucket_oauth_login(
            &client_id,
            &client_secret,
            DEFAULT_BITBUCKET_OAUTH_CALLBACK_PORT,
            DEFAULT_BITBUCKET_OAUTH_SCOPES,
        )
        .map_err(oauth_err)?;
        save_bitbucket_account(login, None, None, bundle)
    })
    .await?;
    let _ = app.emit(CONFIG_CHANGED_EVENT, ());
    Ok(res)
}

#[tauri::command]
pub async fn bitbucket_repos(account_id: String) -> Result<Vec<BitbucketRepoSummary>, String> {
    blocking(move || {
        let id = Uuid::parse_str(account_id.trim()).map_err(|_| "Conta inválida.".to_string())?;
        let cfg = load_cfg()?;
        let a = cfg
            .accounts
            .iter()
            .find(|x| x.id == id)
            .ok_or_else(|| "Conta desconhecida.".to_string())?;
        if a.host != AccountHost::Bitbucket {
            return Err("Esta conta não é do Bitbucket.".into());
        }
        let login = a.github_login.clone().ok_or_else(|| "Conta sem usuário Bitbucket.".to_string())?;
        let bundle = bitbucket_token_load(id)
            .map_err(|e| format!("Keychain: {e}"))?
            .ok_or_else(|| "Nenhum token salvo. Reconecte a conta.".to_string())?;
        list_bitbucket_repos(&login, &bundle.app_password, &login, bundle.via_oauth).map_err(bb_err)
    })
    .await
}
