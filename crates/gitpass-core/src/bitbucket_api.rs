//! Bitbucket Cloud REST (API token Bearer ou Basic com email Atlassian).

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use reqwest::blocking::Client;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BitbucketApiError {
    #[error("http {status}: {body}")]
    Http { status: u16, body: String },
    #[error("request: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

fn basic_header(username: &str, token: &str) -> String {
    format!("Basic {}", B64.encode(format!("{}:{}", username.trim(), token.trim())))
}

fn bearer_header(token: &str) -> String {
    format!("Bearer {}", token.trim())
}

fn client() -> Client {
    Client::builder()
        .user_agent("GitPass")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| Client::new())
}

#[derive(Debug, Clone)]
pub struct BitbucketUser {
    pub username: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserRow {
    username: String,
    #[serde(default)]
    display_name: Option<String>,
}

fn parse_user(body: &str) -> Result<BitbucketUser, BitbucketApiError> {
    let row: UserRow = serde_json::from_str(body)?;
    Ok(BitbucketUser {
        username: row.username,
        display_name: row.display_name,
    })
}

fn user_get(auth_header: &str) -> Result<BitbucketUser, BitbucketApiError> {
    let res = client()
        .get("https://api.bitbucket.org/2.0/user")
        .header("Authorization", auth_header)
        .send()?;
    let status = res.status();
    let status_code = status.as_u16();
    let body = res.text().unwrap_or_default();
    if status_code == 401 || status_code == 403 {
        return Err(BitbucketApiError::Http {
            status: status_code,
            body: "Credenciais Bitbucket recusadas.".into(),
        });
    }
    if !status.is_success() {
        return Err(BitbucketApiError::Http {
            status: status_code,
            body,
        });
    }
    parse_user(&body)
}

/// API token Atlassian: autenticação recomendada (`Authorization: Bearer`).
pub fn validate_api_token(token: &str) -> Result<BitbucketUser, BitbucketApiError> {
    user_get(&bearer_header(token))
}

/// Access token OAuth (Bearer).
pub fn validate_oauth_token(token: &str) -> Result<BitbucketUser, BitbucketApiError> {
    validate_api_token(token)
}

/// Basic com **email Atlassian** + API token (fallback quando Bearer falha).
pub fn validate_credentials_email(email: &str, token: &str) -> Result<BitbucketUser, BitbucketApiError> {
    user_get(&basic_header(email, token))
}

/// @deprecated Preferir [`validate_api_token`]. Mantido para testes / app passwords legadas.
pub fn validate_credentials(username: &str, token: &str) -> Result<BitbucketUser, BitbucketApiError> {
    user_get(&basic_header(username, token))
}

/// Conectar conta: Bearer primeiro; se falhar e houver email, tenta Basic com email.
pub fn validate_for_connect(atlassian_email: Option<&str>, token: &str) -> Result<BitbucketUser, BitbucketApiError> {
    match validate_api_token(token) {
        Ok(u) => Ok(u),
        Err(bearer_err) => {
            if let Some(email) = atlassian_email.map(str::trim).filter(|e| !e.is_empty() && e.contains('@')) {
                validate_credentials_email(email, token).or(Err(bearer_err))
            } else {
                Err(bearer_err)
            }
        }
    }
}

pub(crate) fn authed_post_empty(
    url: &str,
    username: &str,
    token: &str,
    via_oauth: bool,
) -> Result<String, BitbucketApiError> {
    let auth = bearer_header(token);
    let _ = via_oauth;
    let res = client()
        .post(url)
        .header("Authorization", auth)
        .header("Accept", "application/json")
        .send()?;
    let status_code = res.status().as_u16();
    let body = res.text().unwrap_or_default();
    if !(200..300).contains(&status_code) {
        return Err(BitbucketApiError::Http {
            status: status_code,
            body,
        });
    }
    Ok(body)
}

pub(crate) fn authed_post_json(
    url: &str,
    _username: &str,
    token: &str,
    via_oauth: bool,
    json: serde_json::Value,
) -> Result<String, BitbucketApiError> {
    let auth = bearer_header(token);
    let _ = via_oauth;
    let res = client()
        .post(url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .json(&json)
        .send()?;
    let status_code = res.status().as_u16();
    let body = res.text().unwrap_or_default();
    if !(200..300).contains(&status_code) {
        return Err(BitbucketApiError::Http {
            status: status_code,
            body,
        });
    }
    Ok(body)
}

pub(crate) fn authed_put_json(
    url: &str,
    _username: &str,
    token: &str,
    via_oauth: bool,
    json: serde_json::Value,
) -> Result<String, BitbucketApiError> {
    let auth = bearer_header(token);
    let _ = via_oauth;
    let res = client()
        .put(url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .json(&json)
        .send()?;
    let status_code = res.status().as_u16();
    let body = res.text().unwrap_or_default();
    if !(200..300).contains(&status_code) {
        return Err(BitbucketApiError::Http {
            status: status_code,
            body,
        });
    }
    Ok(body)
}

pub(crate) fn authed_get(
    url: &str,
    _username: &str,
    token: &str,
    via_oauth: bool,
) -> Result<String, BitbucketApiError> {
    let auth = bearer_header(token);
    let _ = via_oauth;
    let res = client().get(url).header("Authorization", auth).send()?;
    let status = res.status();
    let status_code = status.as_u16();
    let body = res.text().unwrap_or_default();
    if !status.is_success() {
        return Err(BitbucketApiError::Http {
            status: status_code,
            body,
        });
    }
    Ok(body)
}
