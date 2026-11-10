//! API keys for LLM providers (Keychain).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::KEYCHAIN_SERVICE;

static KEY_CACHE: std::sync::LazyLock<Mutex<HashMap<ProviderId, Option<ApiKey>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static KEYCHAIN_IO: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Openai,
    Anthropic,
    Google,
}

impl ProviderId {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(Self::Openai),
            "anthropic" => Some(Self::Anthropic),
            "google" => Some(Self::Google),
            _ => None,
        }
    }
}

pub fn keychain_account_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Openai => "ai.api.openai",
        ProviderId::Anthropic => "ai.api.anthropic",
        ProviderId::Google => "ai.api.google",
    }
}

/// Opaque API key — never log or serialize to the frontend.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(raw: String) -> Self {
        Self(raw.trim().to_string())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn format_warning(&self) -> Option<&'static str> {
        let s = self.0.as_str();
        if s.starts_with("sk-ant-") {
            Some("Formato típico de chave Anthropic.")
        } else if s.starts_with("AIza") {
            Some("Formato típico de chave Google.")
        } else if s.starts_with("sk-") {
            Some("Formato típico de chave OpenAI.")
        } else {
            None
        }
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl std::fmt::Display for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

fn marker_path(provider: ProviderId) -> Result<PathBuf, std::io::Error> {
    let slug = match provider {
        ProviderId::Openai => "openai",
        ProviderId::Anthropic => "anthropic",
        ProviderId::Google => "google",
    };
    Ok(crate::app_support_dir()?.join("ai-keys").join(format!("{slug}.present")))
}

fn set_marker(provider: ProviderId) {
    if let Ok(path) = marker_path(provider) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, b"1");
    }
}

fn clear_marker(provider: ProviderId) {
    if let Ok(path) = marker_path(provider) {
        let _ = std::fs::remove_file(path);
    }
}

fn entry(provider: ProviderId) -> Result<keyring::Entry, SecretError> {
    Ok(keyring::Entry::new(KEYCHAIN_SERVICE, keychain_account_name(provider))?)
}

pub fn ai_key_exists(provider: ProviderId) -> bool {
    if let Ok(cache) = KEY_CACHE.lock() {
        if let Some(hit) = cache.get(&provider) {
            return hit.is_some();
        }
    }
    marker_path(provider).is_ok_and(|p| p.exists())
}

pub fn ai_key_save(provider: ProviderId, key: &ApiKey) -> Result<(), SecretError> {
    let e = entry(provider)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    e.set_password(key.expose_secret())?;
    set_marker(provider);
    if let Ok(mut cache) = KEY_CACHE.lock() {
        cache.insert(provider, Some(key.clone()));
    }
    Ok(())
}

pub fn ai_key_load(provider: ProviderId) -> Result<Option<ApiKey>, SecretError> {
    if let Ok(cache) = KEY_CACHE.lock() {
        if let Some(hit) = cache.get(&provider) {
            return Ok(hit.clone());
        }
    }
    let e = entry(provider)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    let loaded = match e.get_password() {
        Ok(s) => {
            let k = ApiKey::new(s);
            if k.expose_secret().is_empty() {
                None
            } else {
                Some(k)
            }
        }
        Err(keyring::Error::NoEntry) => None,
        Err(e) => return Err(SecretError::Keyring(e)),
    };
    if loaded.is_some() {
        set_marker(provider);
    } else {
        clear_marker(provider);
    }
    if let Ok(mut cache) = KEY_CACHE.lock() {
        cache.insert(provider, loaded.clone());
    }
    Ok(loaded)
}

pub fn ai_key_delete(provider: ProviderId) -> Result<(), SecretError> {
    let e = entry(provider)?;
    let _guard = KEYCHAIN_IO.lock().unwrap_or_else(|e| e.into_inner());
    match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {
            clear_marker(provider);
            if let Ok(mut cache) = KEY_CACHE.lock() {
                cache.remove(&provider);
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
    fn api_key_debug_redacted() {
        let k = ApiKey::new("sk-secret".into());
        assert!(!format!("{k:?}").contains("sk-secret"));
    }
}
