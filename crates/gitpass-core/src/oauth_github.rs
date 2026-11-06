//! GitHub OAuth — web flow (PKCE + localhost) and **device flow** (desktop-friendly).

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::Rng;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tiny_http::{Header, Response, Server};
use url::Url;

use crate::secrets::{GithubTokenBundle, SecretError};

pub const DEFAULT_GITHUB_SCOPES: &str = "read:user read:org repo";

/// Default localhost port — your GitHub OAuth app "Authorization callback URL" must be `http://127.0.0.1:{port}/callback`.
pub const DEFAULT_OAUTH_CALLBACK_PORT: u16 = 8765;

/// Optional `oauth.env` next to `config.toml` (`~/Library/Application Support/com.gitpass.GitPass/`):
/// lines `KEY=value`, `#` comments. Lets the desktop `.app` load credentials without Terminal env.
pub fn load_oauth_env_file_if_needed() -> Result<(), String> {
    if has_oauth_env_vars() {
        return Ok(());
    }
    let dir = crate::app_support_dir().map_err(|e| e.to_string())?;
    let path = dir.join("oauth.env");
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        if k == "GITHUB_OAUTH_CLIENT_ID" && !v.is_empty() {
            std::env::set_var("GITHUB_OAUTH_CLIENT_ID", v);
        }
        if k == "GITHUB_OAUTH_CLIENT_SECRET" && !v.is_empty() {
            std::env::set_var("GITHUB_OAUTH_CLIENT_SECRET", v);
        }
    }
    Ok(())
}

/// Compile-time OAuth client ID (set with `GITPASS_GITHUB_OAUTH_CLIENT_ID=... cargo build`).
pub fn embedded_oauth_client_id() -> Option<&'static str> {
    option_env!("GITPASS_EMBEDDED_OAUTH_CLIENT_ID")
}

/// Public OAuth App client id: env, `oauth.env`, or compile-time embed.
pub fn resolve_oauth_client_id() -> Result<String, String> {
    load_oauth_env_file_if_needed()?;
    if let Ok(s) = std::env::var("GITHUB_OAUTH_CLIENT_ID") {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    embedded_oauth_client_id()
        .map(str::to_string)
        .ok_or_else(|| {
            "Missing GitHub OAuth client ID. Set GITHUB_OAUTH_CLIENT_ID, add it to oauth.env, or build with GITPASS_GITHUB_OAUTH_CLIENT_ID (enable Device flow on the GitHub OAuth app).".into()
        })
}

fn has_oauth_env_vars() -> bool {
    matches!(
        (
            std::env::var("GITHUB_OAUTH_CLIENT_ID"),
            std::env::var("GITHUB_OAUTH_CLIENT_SECRET")
        ),
        (Ok(ref a), Ok(ref b)) if !a.trim().is_empty() && !b.trim().is_empty()
    )
}

/// Web/PKCE flow: `client_id` + `client_secret`.
pub fn github_oauth_env_credentials() -> Result<(String, String), String> {
    load_oauth_env_file_if_needed()?;
    let id = std::env::var("GITHUB_OAUTH_CLIENT_ID")
        .map_err(|_| "missing env GITHUB_OAUTH_CLIENT_ID".to_string())?;
    let secret = std::env::var("GITHUB_OAUTH_CLIENT_SECRET")
        .map_err(|_| "missing env GITHUB_OAUTH_CLIENT_SECRET".to_string())?;
    if id.trim().is_empty() || secret.trim().is_empty() {
        return Err("GITHUB_OAUTH_CLIENT_ID / GITHUB_OAUTH_CLIENT_SECRET must be non-empty".into());
    }
    Ok((id, secret))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceCodeStart {
    pub client_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DevicePollOutcome {
    Pending,
    SlowDown { interval_secs: u64 },
    Success {
        bundle: GithubTokenBundle,
        login: String,
    },
}

/// Step 1 of device flow (no secret). Call [`github_oauth_device_poll_once`] until `Success`.
pub fn github_oauth_device_request_code(client_id: &str, scopes: &str) -> Result<DeviceCodeStart, OauthError> {
    let http = Client::builder()
        .user_agent("gitpass-oauth-device")
        .build()?;

    let mut params = HashMap::new();
    params.insert("client_id", client_id);
    params.insert("scope", scopes);

    let init: DeviceCodeResponse = http
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&params)
        .send()?
        .json()?;

    if let Some(err) = &init.error {
        let desc = init.error_description.clone().unwrap_or_default();
        return Err(OauthError::Msg(format!(
            "device code: {err}: {desc} (enable Device flow on your GitHub OAuth app settings)"
        )));
    }

    let device_code = init
        .device_code
        .ok_or_else(|| OauthError::Msg("device flow: missing device_code".into()))?;
    let user_code = init
        .user_code
        .ok_or_else(|| OauthError::Msg("device flow: missing user_code".into()))?;
    let verification_uri = init
        .verification_uri
        .unwrap_or_else(|| "https://github.com/login/device".to_string());
    let interval_secs = init.interval.unwrap_or(5).max(5);
    let expires_in_secs = init.expires_in.unwrap_or(900);

    Ok(DeviceCodeStart {
        client_id: client_id.to_string(),
        user_code,
        verification_uri,
        device_code,
        interval_secs,
        expires_in_secs,
    })
}

/// Step 2: single poll (call repeatedly from UI with `interval_secs` spacing).
pub fn github_oauth_device_poll_once(client_id: &str, device_code: &str) -> Result<DevicePollOutcome, OauthError> {
    let http = Client::builder()
        .user_agent("gitpass-oauth-device")
        .build()?;

    let mut poll = HashMap::new();
    poll.insert("client_id", client_id);
    poll.insert("device_code", device_code);
    poll.insert(
        "grant_type",
        "urn:ietf:params:oauth:grant-type:device_code",
    );

    let resp = http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&poll)
        .send()?;

    let token: AccessTokenResponse = resp.json()?;

    if let Some(err) = &token.error {
        return match err.as_str() {
            "authorization_pending" => Ok(DevicePollOutcome::Pending),
            "slow_down" => {
                let extra = token
                    .interval
                    .unwrap_or(5)
                    .try_into()
                    .unwrap_or(5u64);
                Ok(DevicePollOutcome::SlowDown {
                    interval_secs: extra.max(5),
                })
            }
            _ => {
                let desc = token.error_description.clone().unwrap_or_default();
                Err(OauthError::Msg(format!("{err}: {desc}")))
            }
        };
    }

    let Some(at) = token.access_token.clone().filter(|s| !s.is_empty()) else {
        return Ok(DevicePollOutcome::Pending);
    };

    let login = fetch_github_login(&http, &at)?;
    let expires_at_epoch = token.expires_in.map(|sec| now_epoch() + sec as i64);
    let bundle = GithubTokenBundle {
        access_token: at,
        token_type: token.token_type,
        scope: token.scope,
        expires_at_epoch,
        refresh_token: token.refresh_token,
        oauth_via_device: true,
        oauth_client_id: Some(client_id.to_string()),
    };

    Ok(DevicePollOutcome::Success { bundle, login })
}

/// CLI convenience: request, open browser, poll until success or timeout.
pub fn github_oauth_device_login(client_id: &str, scopes: &str) -> Result<(GithubTokenBundle, String), OauthError> {
    let start = github_oauth_device_request_code(client_id, scopes)?;
    eprintln!(
        "GitHub device login:\n  Open {} and enter code: {}\n",
        start.verification_uri, start.user_code
    );
    opener::open(&start.verification_uri).map_err(|e| OauthError::Msg(format!("open browser: {e}")))?;

    let deadline = std::time::Instant::now() + Duration::from_secs(start.expires_in_secs);
    let mut interval = start.interval_secs.max(5);

    while std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_secs(interval));
        match github_oauth_device_poll_once(&start.client_id, &start.device_code)? {
            DevicePollOutcome::Pending => {}
            DevicePollOutcome::SlowDown { interval_secs } => {
                interval = interval_secs.max(5);
            }
            DevicePollOutcome::Success { bundle, login } => return Ok((bundle, login)),
        }
    }
    Err(OauthError::Msg(
        "device flow: timed out waiting for authorization".into(),
    ))
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    #[serde(default)]
    device_code: Option<String>,
    #[serde(default)]
    user_code: Option<String>,
    #[serde(default)]
    verification_uri: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Error)]
pub enum OauthError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse url: {0}")]
    Url(#[from] url::ParseError),
    #[error("oauth: {0}")]
    Msg(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("reqwest: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("keychain: {0}")]
    Secret(#[from] SecretError),
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    /// Present on `slow_down` errors (seconds between polls).
    #[serde(default)]
    interval: Option<u64>,
}

fn random_string(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

fn pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Exchange a **refresh_token** for new tokens. Use `client_secret: None` for tokens from **device flow**
/// (GitHub documents that `client_secret` is not required in that case).
pub fn github_oauth_refresh(
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    oauth_via_device: bool,
) -> Result<GithubTokenBundle, OauthError> {
    let http = Client::builder()
        .user_agent("gitpass-oauth-refresh")
        .build()?;

    let mut params = HashMap::new();
    params.insert("client_id", client_id);
    if let Some(sec) = client_secret {
        params.insert("client_secret", sec);
    }
    params.insert("grant_type", "refresh_token");
    params.insert("refresh_token", refresh_token);

    let resp = http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&params)
        .send()?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(OauthError::Msg(format!(
            "refresh token HTTP {}: {}",
            status,
            body.chars().take(400).collect::<String>()
        )));
    }

    let token: AccessTokenResponse = resp.json()?;
    if let Some(err) = &token.error {
        let desc = token.error_description.clone().unwrap_or_default();
        return Err(OauthError::Msg(format!("{err}: {desc}")));
    }

    let Some(at) = token.access_token.clone().filter(|s| !s.is_empty()) else {
        return Err(OauthError::Msg("refresh: no access_token".into()));
    };

    let expires_at_epoch = token.expires_in.map(|sec| now_epoch() + sec as i64);

    let new_refresh = token.refresh_token.clone().filter(|s| !s.is_empty());
    Ok(GithubTokenBundle {
        access_token: at,
        token_type: token.token_type,
        scope: token.scope,
        expires_at_epoch,
        refresh_token: new_refresh.or_else(|| Some(refresh_token.to_string())),
        oauth_via_device,
        oauth_client_id: Some(client_id.to_string()),
    })
}

/// Run OAuth: opens browser, listens on localhost for `code`, exchanges token, fetches `login` from GitHub API.
pub fn github_oauth_login(
    client_id: &str,
    client_secret: &str,
    port: u16,
    scopes: &str,
) -> Result<(GithubTokenBundle, String), OauthError> {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| OauthError::Msg(format!("bind {addr}: {e}")))?;

    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = random_string(32);
    let code_verifier = random_string(64);
    let code_challenge = pkce_challenge(&code_verifier);

    let mut auth = Url::parse("https://github.com/login/oauth/authorize")?;
    auth.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", scopes)
        .append_pair("state", &state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("allow_signup", "false");

    let auth_url = auth.to_string();
    opener::open(&auth_url).map_err(|e| OauthError::Msg(format!("open browser: {e}")))?;

    eprintln!("If the browser did not open, visit:\n{auth_url}\n");

    let timeout = Duration::from_secs(10 * 60);
    let deadline = std::time::Instant::now() + timeout;
    let mut code: Option<String> = None;
    let mut got_state: Option<String> = None;

    while std::time::Instant::now() < deadline {
        let tick = Duration::from_secs(1);
        let Some(req) = server.recv_timeout(tick).ok().flatten() else {
            if std::time::Instant::now() >= deadline {
                break;
            }
            continue;
        };
        let url = req.url().to_string();
        let ok_html = r#"<html><body><p>GitPass: signed in. You can close this tab.</p></body></html>"#;
        let err_html = |msg: &str| {
            format!(
                r#"<html><body><p>GitPass error: {}</p></body></html>"#,
                html_escape(msg)
            )
        };

        let full = format!("http://127.0.0.1{url}");
        let parsed = match Url::parse(&full) {
            Ok(u) => u,
            Err(e) => {
                let resp = Response::from_string(err_html(&format!("bad url: {e}")))
                    .with_status_code(400);
                let _ = req.respond(resp);
                continue;
            }
        };

        if parsed.path() != "/callback" {
            let resp =
                Response::from_string(err_html("unexpected path")).with_status_code(404);
            let _ = req.respond(resp);
            continue;
        }
        let mut q: HashMap<String, String> = HashMap::new();
        for (k, v) in parsed.query_pairs() {
            q.insert(k.into_owned(), v.into_owned());
        }

        if let Some(ecode) = q.get("error") {
            let desc = q.get("error_description").cloned().unwrap_or_default();
            let msg = format!("{ecode} {desc}");
            let resp = Response::from_string(err_html(&msg)).with_status_code(400);
            let _ = req.respond(resp);
            return Err(OauthError::Msg(msg));
        }

        let Some(c) = q.get("code").cloned() else {
            let resp = Response::from_string(err_html("missing code")).with_status_code(400);
            let _ = req.respond(resp);
            continue;
        };
        let Some(st) = q.get("state").cloned() else {
            let resp = Response::from_string(err_html("missing state")).with_status_code(400);
            let _ = req.respond(resp);
            continue;
        };

        if st != state {
            let resp = Response::from_string(err_html("state mismatch")).with_status_code(400);
            let _ = req.respond(resp);
            return Err(OauthError::Msg("OAuth state mismatch".into()));
        }

        code = Some(c);
        got_state = Some(st);
        let resp = Response::from_string(ok_html)
            .with_header(Header::from_bytes(&b"Content-Type"[..], b"text/html; charset=utf-8").unwrap());
        let _ = req.respond(resp);
        break;
    }

    let code = code.ok_or_else(|| OauthError::Msg("timed out waiting for OAuth callback".into()))?;
    let _ = got_state;

    let http = Client::builder()
        .user_agent("gitpass-oauth")
        .build()?;

    let mut params = HashMap::new();
    params.insert("client_id", client_id);
    params.insert("client_secret", client_secret);
    params.insert("code", &code);
    params.insert("redirect_uri", &redirect_uri);
    params.insert("grant_type", "authorization_code");
    params.insert("code_verifier", &code_verifier);

    let resp = http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&params)
        .send()?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(OauthError::Msg(format!(
            "token exchange HTTP {}: {}",
            status,
            body.chars().take(400).collect::<String>()
        )));
    }

    let token: AccessTokenResponse = resp.json()?;

    if let Some(err) = &token.error {
        let desc = token
            .error_description
            .clone()
            .unwrap_or_default();
        return Err(OauthError::Msg(format!("{err}: {desc}")));
    }

    let Some(at) = token.access_token.clone().filter(|s| !s.is_empty()) else {
        return Err(OauthError::Msg(
            "GitHub returned no access_token (check OAuth app callback URL matches this CLI)".into(),
        ));
    };

    let login = fetch_github_login(&http, &at)?;

    let expires_at_epoch = token.expires_in.map(|sec| now_epoch() + sec as i64);

    let bundle = GithubTokenBundle {
        access_token: at,
        token_type: token.token_type,
        scope: token.scope,
        expires_at_epoch,
        refresh_token: token.refresh_token,
        oauth_via_device: false,
        oauth_client_id: Some(client_id.to_string()),
    };

    Ok((bundle, login))
}

fn fetch_github_login(client: &Client, token: &str) -> Result<String, OauthError> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    let u: User = client
        .get("https://api.github.com/user")
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()?
        .json()?;
    Ok(u.login)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
