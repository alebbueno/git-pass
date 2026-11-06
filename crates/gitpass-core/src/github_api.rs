//! Minimal GitHub REST API (disambiguation in daemon).

use reqwest::blocking::Client;
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::oauth_github::{
    embedded_oauth_client_id, github_oauth_env_credentials, github_oauth_refresh,
    resolve_oauth_client_id,
};
use crate::secrets::{github_token_load, github_token_save};
use crate::SecretError;

#[derive(Debug, Error)]
pub enum GithubApiError {
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("reqwest: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("keychain/oauth: {0}")]
    Keychain(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoPermissions {
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub maintain: bool,
    #[serde(default)]
    pub push: bool,
    #[serde(default)]
    pub triage: bool,
    #[serde(default)]
    pub pull: bool,
}

#[derive(Debug, Deserialize)]
struct RepoResponse {
    permissions: Option<RepoPermissions>,
}

/// `GET /repos/{owner}/{repo}` — returns permissions for the authenticated token.
pub fn fetch_repo_permissions(
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<RepoPermissions, GithubApiError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}");
    let client = Client::builder()
        .user_agent("gitpassd-validation")
        .build()?;

    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()?;

    let status = resp.status().as_u16();
    let body = resp.text()?;
    if !(200..300).contains(&status) {
        return Err(GithubApiError::Http { status, body });
    }

    let r: RepoResponse = serde_json::from_str(&body)?;
    Ok(r.permissions.unwrap_or_default())
}

/// True if the token has **push** or **admin** (or **maintain**) on the repository.
pub fn token_can_push(token: &str, owner: &str, repo: &str) -> Result<bool, GithubApiError> {
    let p = fetch_repo_permissions(token, owner, repo)?;
    Ok(p.admin || p.maintain || p.push)
}

fn keychain_err(e: impl std::fmt::Display) -> GithubApiError {
    GithubApiError::Keychain(e.to_string())
}

/// Loads token from Keychain for `account_id`. On **401**, tries **OAuth refresh** when `GITHUB_OAUTH_CLIENT_*` and `refresh_token` are present, then saves the new bundle.
pub fn token_can_push_for_account(
    account_id: Uuid,
    owner: &str,
    repo: &str,
) -> Result<bool, GithubApiError> {
    let mut bundle = github_token_load(account_id)
        .map_err(|e: SecretError| keychain_err(e))?
        .ok_or_else(|| keychain_err("no token in keychain"))?;

    match fetch_repo_permissions(&bundle.access_token, owner, repo) {
        Ok(p) => return Ok(p.admin || p.maintain || p.push),
        Err(GithubApiError::Http { status, .. }) if status == 401 => {
            let Some(ref refresh) = bundle.refresh_token else {
                return Err(GithubApiError::Http {
                    status: 401,
                    body: "unauthorized; no refresh_token stored — run gitpass account login".into(),
                });
            };
            let new_bundle = if bundle.oauth_via_device {
                let cid = bundle
                    .oauth_client_id
                    .clone()
                    .or_else(|| embedded_oauth_client_id().map(String::from))
                    .or_else(|| resolve_oauth_client_id().ok())
                    .ok_or_else(|| {
                        keychain_err(
                            "device-flow token refresh: missing OAuth client id (re-authenticate)",
                        )
                    })?;
                github_oauth_refresh(&cid, None, refresh, true).map_err(|e| keychain_err(e))?
            } else {
                let (id, secret) =
                    github_oauth_env_credentials().map_err(|e| keychain_err(e))?;
                github_oauth_refresh(&id, Some(secret.as_str()), refresh, false)
                    .map_err(|e| keychain_err(e))?
            };
            github_token_save(account_id, &new_bundle).map_err(|e: SecretError| keychain_err(e))?;
            bundle = new_bundle;
        }
        Err(e) => return Err(e),
    }

    let p = fetch_repo_permissions(&bundle.access_token, owner, repo)?;
    Ok(p.admin || p.maintain || p.push)
}
