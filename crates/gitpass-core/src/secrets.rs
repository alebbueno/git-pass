//! macOS Keychain (via `keyring`) — OAuth tokens per account.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Tokens already read this process — avoids repeated Keychain prompts in the app.
static TOKEN_CACHE: std::sync::LazyLock<Mutex<HashMap<Uuid, Option<GithubTokenBundle>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

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
#[derive(Clone, Serialize, Deserialize)]
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

/// Tokens never reach `{:?}` output (logs, panics, `dbg!`).
impl std::fmt::Debug for GithubTokenBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubTokenBundle")
            .field("access_token", &"<redacted>")
            .field("token_type", &self.token_type)
            .field("scope", &self.scope)
            .field("expires_at_epoch", &self.expires_at_epoch)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("oauth_via_device", &self.oauth_via_device)
            .field("oauth_client_id", &self.oauth_client_id)
            .finish()
    }
}

/// Keychain account name of the token item for `account_id`.
pub fn keychain_account_name(account_id: Uuid) -> String {
    format!("github.oauth.{account_id}")
}

fn entry_for_account(account_id: Uuid) -> Result<keyring::Entry, SecretError> {
    Ok(keyring::Entry::new(KEYCHAIN_SERVICE, &keychain_account_name(account_id))?)
}

pub fn github_token_save(account_id: Uuid, bundle: &GithubTokenBundle) -> Result<(), SecretError> {
    let json = serde_json::to_string(bundle)?;
    let e = entry_for_account(account_id)?;
    e.set_password(&json)?;
    if let Ok(mut cache) = TOKEN_CACHE.lock() {
        cache.insert(account_id, Some(bundle.clone()));
    }
    Ok(())
}

pub fn github_token_load(account_id: Uuid) -> Result<Option<GithubTokenBundle>, SecretError> {
    if let Ok(cache) = TOKEN_CACHE.lock() {
        if let Some(hit) = cache.get(&account_id) {
            return Ok(hit.clone());
        }
    }
    let e = entry_for_account(account_id)?;
    let loaded = match e.get_password() {
        Ok(s) => {
            let b: GithubTokenBundle = serde_json::from_str(&s)?;
            Some(b)
        }
        Err(keyring::Error::NoEntry) => None,
        Err(e) => return Err(SecretError::Keyring(e)),
    };
    if let Ok(mut cache) = TOKEN_CACHE.lock() {
        cache.insert(account_id, loaded.clone());
    }
    Ok(loaded)
}

/// Deletes the token item. A missing item is not an error; any other failure is, so a removed
/// account never silently leaves its token behind in the Keychain.
pub fn github_token_delete(account_id: Uuid) -> Result<(), SecretError> {
    let e = entry_for_account(account_id)?;
    match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {
            if let Ok(mut cache) = TOKEN_CACHE.lock() {
                cache.remove(&account_id);
            }
            Ok(())
        }
        Err(e) => Err(SecretError::Keyring(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_tokens() {
        let b = GithubTokenBundle {
            access_token: "gho_secret_access".into(),
            token_type: Some("bearer".into()),
            scope: Some("repo".into()),
            expires_at_epoch: None,
            refresh_token: Some("ghr_secret_refresh".into()),
            oauth_via_device: true,
            oauth_client_id: Some("Iv1.abc".into()),
        };
        let s = format!("{b:?} {:?}", Some(b.clone()));
        assert!(!s.contains("gho_secret_access") && !s.contains("ghr_secret_refresh"), "{s}");
        assert!(s.contains("Iv1.abc") && s.contains("<redacted>"));
    }

    #[test]
    fn keychain_item_naming() {
        assert_eq!(
            keychain_account_name(Uuid::nil()),
            "github.oauth.00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(KEYCHAIN_SERVICE, crate::APP_SUPPORT_DIR);
    }
}
