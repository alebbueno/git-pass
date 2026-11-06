//! Shared types and paths for GitPass (daemon, CLI, Tauri).

pub mod bindings;
pub mod clone;
pub mod github_user;
pub mod integration;
mod config;
pub mod github_api;
pub mod github_repos;
mod hooks;
pub mod git;
pub mod oauth_github;
pub mod remote;
mod secrets;
mod validate;

pub use git::{git_log_lines, git_remote_url, git_work_tree_root};

pub use config::{
    config_file_path, Account, AppConfig, AuthMode, ConfigError, EmergencyState, RepoBinding,
    UiState, CONFIG_FILE_NAME,
};
pub use hooks::{
    hook_script, hooks_dir, hooks_installed_ok, write_hook_scripts, HookPhase, HOOKS_DIR_NAME,
};
pub use validate::{
    ci_bypass_from_env, is_validated_phase, policy_when_daemon_unreachable,
    validate_repo_for_hook, validate_repo_for_hook_daemon, validate_repo_for_hook_with_options,
    HookContext, ValidateFail, ValidateOptions,
};
pub use remote::{normalize_owner, parse_github_remote};
pub use secrets::{
    github_token_delete, github_token_load, github_token_save, GithubTokenBundle, SecretError,
    KEYCHAIN_SERVICE,
};
pub use oauth_github::{
    embedded_oauth_client_id, github_oauth_device_login, github_oauth_device_poll_once,
    github_oauth_device_request_code, github_oauth_env_credentials, github_oauth_login,
    github_oauth_refresh, load_oauth_env_file_if_needed, resolve_oauth_client_id,
    DeviceCodeStart, DevicePollOutcome, DEFAULT_GITHUB_SCOPES, DEFAULT_OAUTH_CALLBACK_PORT, OauthError,
};
pub use github_api::{token_can_push, token_can_push_for_account, GithubApiError};
pub use github_repos::{list_user_repos, GithubRepoSummary};

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Application support directory name (reverse-DNS).
pub const APP_SUPPORT_DIR: &str = "com.gitpass.GitPass";

/// Returns `~/Library/Application Support/com.gitpass.GitPass` on macOS.
pub fn app_support_dir() -> Result<PathBuf, std::io::Error> {
    let base = dirs::data_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve user data directory",
        )
    })?;
    Ok(base.join(APP_SUPPORT_DIR))
}

/// Unix socket path for IPC (`gitpassd` listens here).
pub fn ipc_socket_path() -> Result<PathBuf, std::io::Error> {
    let dir = app_support_dir()?;
    Ok(dir.join("gitpassd.sock"))
}

/// Ensure parent dirs for config/socket exist.
pub fn ensure_app_support_exists() -> Result<PathBuf, std::io::Error> {
    let dir = app_support_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

// --- IPC protocol (line-delimited JSON, one message per line) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcRequest {
    Ping,
    /// Validate a Git hook invocation (`gitpass hook run` delegates here when the daemon is up).
    ValidateHook {
        phase: String,
        /// Current working directory when the hook ran (usually the repo root or subdir).
        cwd: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcResponse {
    Pong { gitpassd_version: String },
    ValidateResult {
        ok: bool,
        #[serde(default)]
        unconfirmed: bool,
        /// Error code when `ok` is false (e.g. `GP-OWNER`).
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        /// Human-readable message when `ok` is false.
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        /// Warning when `ok` is true but API could not confirm (ambiguous case).
        #[serde(skip_serializing_if = "Option::is_none")]
        warning: Option<String>,
    },
    Error { code: String, message: String },
}

/// Encode a single IPC message as one line (newline-terminated).
pub fn encode_line(msg: &impl Serialize) -> Result<String, serde_json::Error> {
    let s = serde_json::to_string(msg)?;
    Ok(format!("{s}\n"))
}

/// Decode one line (strip `\n`).
pub fn decode_request_line(line: &str) -> Result<IpcRequest, serde_json::Error> {
    serde_json::from_str(line.trim())
}

pub fn decode_response_line(line: &str) -> Result<IpcResponse, serde_json::Error> {
    serde_json::from_str(line.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ping() {
        let req = IpcRequest::Ping;
        let line = encode_line(&req).unwrap();
        let back: IpcRequest = serde_json::from_str(line.trim()).unwrap();
        assert!(matches!(back, IpcRequest::Ping));
    }
}
