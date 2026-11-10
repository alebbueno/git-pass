//! macOS Keychain (via `keyring`) — OAuth tokens per account.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Tokens already read this process — avoids repeated Keychain prompts in the app.
static TOKEN_CACHE: std::sync::LazyLock<Mutex<HashMap<Uuid, Option<GithubTokenBundle>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static BITBUCKET_TOKEN_CACHE: std::sync::LazyLock<Mutex<HashMap<Uuid, Option<BitbucketTokenBundle>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
/// One Keychain dialog at a time (parallel `accounts_list` / cloud used to stack prompts).
static KEYCHAIN_IO: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

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

fn token_marker_path(account_id: Uuid) -> Result<PathBuf, std::io::Error> {
    Ok(crate::app_support_dir()?.join("tokens").join(format!("{account_id}.present")))
}

fn set_token_marker(account_id: Uuid) {
    if let Ok(path) = token_marker_path(account_id) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, b"1");
    }
}

fn clear_token_marker(account_id: Uuid) {
    if let Ok(path) = token_marker_path(account_id) {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether an OAuth token is stored, without reading the Keychain (marker + cache).
pub fn github_token_exists(account_id: Uuid) -> bool {
    if let Ok(cache) = TOKEN_CACHE.lock() {
        if let Some(hit) = cache.get(&account_id) {
            return hit.is_some();
        }
    }
    token_marker_path(account_id).is_ok_and(|p| p.exists())
}

pub fn github_token_save(account_id: Uuid, bundle: &GithubTokenBundle) -> Result<(), SecretError> {
    let json = serde_json::to_string(bundle)?;
    let e = entry_for_account(account_id)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    e.set_password(&json)?;
    set_token_marker(account_id);
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
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    let loaded = match e.get_password() {
        Ok(s) => {
            let b: GithubTokenBundle = serde_json::from_str(&s)?;
            Some(b)
        }
        Err(keyring::Error::NoEntry) => None,
        Err(e) => return Err(SecretError::Keyring(e)),
    };
    if loaded.is_some() {
        set_token_marker(account_id);
    } else {
        clear_token_marker(account_id);
    }
    if let Ok(mut cache) = TOKEN_CACHE.lock() {
        cache.insert(account_id, loaded.clone());
    }
    Ok(loaded)
}

/// Deletes the token item. A missing item is not an error; any other failure is, so a removed
/// account never silently leaves its token behind in the Keychain.
/// API token ou access token OAuth do Bitbucket Cloud (campo legado `app_password` no JSON).
#[derive(Clone, Serialize, Deserialize)]
pub struct BitbucketTokenBundle {
    #[serde(alias = "api_token")]
    pub app_password: String,
    #[serde(default)]
    pub via_oauth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_epoch: Option<i64>,
}

impl std::fmt::Debug for BitbucketTokenBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitbucketTokenBundle")
            .field("app_password", &"<redacted>")
            .field("via_oauth", &self.via_oauth)
            .finish_non_exhaustive()
    }
}

pub fn keychain_bitbucket_account_name(account_id: Uuid) -> String {
    format!("bitbucket.app_password.{account_id}")
}

fn bitbucket_entry(account_id: Uuid) -> Result<keyring::Entry, SecretError> {
    Ok(keyring::Entry::new(
        KEYCHAIN_SERVICE,
        &keychain_bitbucket_account_name(account_id),
    )?)
}

fn bitbucket_marker_path(account_id: Uuid) -> Result<PathBuf, std::io::Error> {
    Ok(crate::app_support_dir()?.join("tokens").join(format!("{account_id}.bitbucket")))
}

fn set_bitbucket_marker(account_id: Uuid) {
    if let Ok(path) = bitbucket_marker_path(account_id) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, b"1");
    }
}

fn clear_bitbucket_marker(account_id: Uuid) {
    if let Ok(path) = bitbucket_marker_path(account_id) {
        let _ = std::fs::remove_file(path);
    }
}

pub fn bitbucket_token_exists(account_id: Uuid) -> bool {
    if let Ok(cache) = BITBUCKET_TOKEN_CACHE.lock() {
        if let Some(hit) = cache.get(&account_id) {
            return hit.is_some();
        }
    }
    bitbucket_marker_path(account_id).is_ok_and(|p| p.exists())
}

pub fn bitbucket_token_save(account_id: Uuid, bundle: &BitbucketTokenBundle) -> Result<(), SecretError> {
    let json = serde_json::to_string(bundle)?;
    let e = bitbucket_entry(account_id)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    e.set_password(&json)?;
    set_bitbucket_marker(account_id);
    if let Ok(mut cache) = BITBUCKET_TOKEN_CACHE.lock() {
        cache.insert(account_id, Some(bundle.clone()));
    }
    Ok(())
}

pub fn bitbucket_token_load(account_id: Uuid) -> Result<Option<BitbucketTokenBundle>, SecretError> {
    if let Ok(cache) = BITBUCKET_TOKEN_CACHE.lock() {
        if let Some(hit) = cache.get(&account_id) {
            return Ok(hit.clone());
        }
    }
    let e = bitbucket_entry(account_id)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    let loaded = match e.get_password() {
        Ok(s) => {
            let b: BitbucketTokenBundle = serde_json::from_str(&s)?;
            Some(b)
        }
        Err(keyring::Error::NoEntry) => None,
        Err(e) => return Err(SecretError::Keyring(e)),
    };
    if loaded.is_some() {
        set_bitbucket_marker(account_id);
    } else {
        clear_bitbucket_marker(account_id);
    }
    if let Ok(mut cache) = BITBUCKET_TOKEN_CACHE.lock() {
        cache.insert(account_id, loaded.clone());
    }
    Ok(loaded)
}

pub fn bitbucket_token_delete(account_id: Uuid) -> Result<(), SecretError> {
    let e = bitbucket_entry(account_id)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {
            clear_bitbucket_marker(account_id);
            if let Ok(mut cache) = BITBUCKET_TOKEN_CACHE.lock() {
                cache.remove(&account_id);
            }
            Ok(())
        }
        Err(e) => Err(SecretError::Keyring(e)),
    }
}

pub fn github_token_delete(account_id: Uuid) -> Result<(), SecretError> {
    let e = entry_for_account(account_id)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {
            clear_token_marker(account_id);
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
