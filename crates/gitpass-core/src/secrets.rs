//! macOS Keychain (via `keyring`) — OAuth tokens per account.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Keychain service name for GitPass items.
pub const KEYCHAIN_SERVICE: &str = "com.gitpass.GitPass";

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Stored JSON blob for a GitHub OAuth access token (refresh optional).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubTokenBundle {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    /// When the access token expires (if GitHub returned `expires_in`).
    #[serde(default)]
    pub expires_at_epoch: Option<i64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Set when the token was obtained via OAuth **device flow** (refresh without `client_secret`).
    #[serde(default)]
    pub oauth_via_device: bool,
    /// `client_id` used for this token (needed to refresh device-flow tokens).
    #[serde(default)]
    pub oauth_client_id: Option<String>,
}

fn entry_for_account(account_id: Uuid) -> Result<keyring::Entry, SecretError> {
    let user = format!("github.oauth.{account_id}");
    Ok(keyring::Entry::new(KEYCHAIN_SERVICE, &user)?)
}

pub fn github_token_save(account_id: Uuid, bundle: &GithubTokenBundle) -> Result<(), SecretError> {
    let json = serde_json::to_string(bundle)?;
    let e = entry_for_account(account_id)?;
    e.set_password(&json)?;
    Ok(())
}

pub fn github_token_load(account_id: Uuid) -> Result<Option<GithubTokenBundle>, SecretError> {
    let e = entry_for_account(account_id)?;
    match e.get_password() {
        Ok(s) => {
            let b: GithubTokenBundle = serde_json::from_str(&s)?;
            Ok(Some(b))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(SecretError::Keyring(e)),
    }
}

pub fn github_token_delete(account_id: Uuid) -> Result<(), SecretError> {
    let e = entry_for_account(account_id)?;
    let _ = e.delete_credential();
    Ok(())
}
