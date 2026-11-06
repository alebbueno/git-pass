//! On-disk configuration (non-secrets only). Stored as TOML under Application Support.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{app_support_dir, ensure_app_support_exists};

/// Filename inside Application Support.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Returns the path to `config.toml` (may not exist yet).
pub fn config_file_path() -> Result<PathBuf, std::io::Error> {
    Ok(app_support_dir()?.join(CONFIG_FILE_NAME))
}

/// Top-level persisted state for GitPass (v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Config schema version for forward-compatible migration.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub repos: Vec<RepoBinding>,
    #[serde(default)]
    pub ui: UiState,
}

fn default_schema_version() -> u32 {
    1
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            accounts: Vec::new(),
            repos: Vec::new(),
            ui: UiState::default(),
        }
    }
}

/// A GitHub identity managed by the user (OAuth tokens live in Keychain, not here).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Account {
    pub id: Uuid,
    /// GitHub login, e.g. `octocat` (filled after OAuth or manual entry).
    #[serde(default)]
    pub github_login: Option<String>,
    /// Display label in the UI (defaults to login in the app if empty).
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub auth: AuthMode,
    /// GitHub profile name, used as `user.name` in bound repos.
    #[serde(default)]
    pub name: Option<String>,
    /// Commit email applied as `user.email` in bound repos (primary or noreply).
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    Ssh,
    Https,
}

/// Binding between a local Git repo root and an account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBinding {
    pub id: Uuid,
    pub account_id: Uuid,
    /// Normalized absolute path to the repository root (directory containing `.git` or the work tree).
    pub root_path: PathBuf,
    /// Parsed from `origin` when known (for diagnostics and local checks).
    #[serde(default)]
    pub github_owner: Option<String>,
    #[serde(default)]
    pub github_repo: Option<String>,
}

/// Menu bar / UI state (non-sensitive).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default)]
    pub pinned_repo_binding_id: Option<Uuid>,
    #[serde(default)]
    pub last_active_repo_binding_id: Option<Uuid>,
    #[serde(default)]
    pub emergency: EmergencyState,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            pinned_repo_binding_id: None,
            last_active_repo_binding_id: None,
            emergency: EmergencyState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyState {
    /// If set and `now < until`, local commits may be allowed when daemon IPC fails (policy in daemon).
    #[serde(default)]
    pub allow_local_commit_until: Option<String>,
    /// `env` | `menubar` | `cli` for audit trail.
    #[serde(default)]
    pub allow_local_commit_via: Option<String>,
}

impl Default for EmergencyState {
    fn default() -> Self {
        Self {
            allow_local_commit_until: None,
            allow_local_commit_via: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML serialization: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("TOML deserialization: {0}")]
    TomlDe(#[from] toml::de::Error),
}

impl AppConfig {
    /// Load from disk, or return default if the file is missing.
    pub fn load_or_default() -> Result<Self, ConfigError> {
        let path = config_file_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        let cfg: AppConfig = toml::from_str(&raw)?;
        Ok(cfg)
    }

    /// Write atomically to `config.toml` (same directory must exist).
    pub fn save(&self) -> Result<(), ConfigError> {
        let dir = ensure_app_support_exists()?;
        let path = dir.join(CONFIG_FILE_NAME);
        self.save_to_path(&path)
    }

    /// Write to a specific path (used in tests).
    pub fn save_to_path(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let pretty = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, pretty)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_toml() {
        let mut cfg = AppConfig::default();
        cfg.accounts.push(Account {
            id: Uuid::nil(),
            github_login: Some("octocat".into()),
            label: Some("Work".into()),
            auth: AuthMode::Https,
            ..Default::default()
        });
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: AppConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.accounts.len(), 1);
        assert_eq!(back.accounts[0].github_login.as_deref(), Some("octocat"));
    }
}
