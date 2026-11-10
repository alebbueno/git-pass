//! Bitbucket Cloud OAuth 2.0 (authorization code + localhost callback).

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use reqwest::blocking::Client;
use serde::Deserialize;
use tiny_http::{Header, Response, Server};
use url::Url;

use crate::oauth_github::{html_escape, load_oauth_env_file_if_needed, OauthError, random_string};
use crate::secrets::BitbucketTokenBundle;

pub const DEFAULT_BITBUCKET_OAUTH_SCOPES: &str = "repository account email";

/// Register this URL on the Bitbucket OAuth consumer: `http://127.0.0.1:{port}/callback`.
pub const DEFAULT_BITBUCKET_OAUTH_CALLBACK_PORT: u16 = 8766;

pub fn resolve_bitbucket_oauth_client() -> Result<(String, String), String> {
    load_oauth_env_file_if_needed()?;
    let id = std::env::var("BITBUCKET_OAUTH_CLIENT_ID")
        .map_err(|_| "Missing BITBUCKET_OAUTH_CLIENT_ID.".to_string())?
        .trim()
        .to_string();
    let secret = std::env::var("BITBUCKET_OAUTH_CLIENT_SECRET")
        .map_err(|_| "Missing BITBUCKET_OAUTH_CLIENT_SECRET.".to_string())?
        .trim()
        .to_string();
    if id.is_empty() || secret.is_empty() {
        return Err(
            "Bitbucket OAuth precisa de client id e secret (oauth.env ou variáveis de ambiente)."
                .into(),
        );
    }
    Ok((id, secret))
}

/// Opens the browser, waits for `code` on localhost, exchanges token, loads Bitbucket username.
pub fn bitbucket_oauth_login(
    client_id: &str,
    client_secret: &str,
    port: u16,
    scopes: &str,
) -> Result<(BitbucketTokenBundle, String), OauthError> {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| OauthError::Msg(format!("bind {addr}: {e}")))?;

    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = random_string(32);

    let mut auth = Url::parse("https://bitbucket.org/site/oauth2/authorize")?;
    auth.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &state)
        .append_pair("scope", scopes);

    let auth_url = auth.to_string();
    opener::open(&auth_url).map_err(|e| OauthError::Msg(format!("open browser: {e}")))?;
    eprintln!("If the browser did not open, visit:\n{auth_url}\n");

    let timeout = Duration::from_secs(10 * 60);
    let deadline = std::time::Instant::now() + timeout;
    let mut code: Option<String> = None;

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
        let resp = Response::from_string(ok_html)
            .with_header(Header::from_bytes(&b"Content-Type"[..], b"text/html; charset=utf-8").unwrap());
        let _ = req.respond(resp);
        break;
    }

    let code = code.ok_or_else(|| OauthError::Msg("timed out waiting for Bitbucket OAuth callback".into()))?;

    let http = Client::builder().user_agent("GitPass").build()?;
    let basic = format!("Basic {}", B64.encode(format!("{client_id}:{client_secret}")));

    let resp = http
        .post("https://bitbucket.org/site/oauth2/access_token")
        .header("Authorization", basic)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
        ])
        .send()?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().unwrap_or_default();
        return Err(OauthError::Msg(format!("token exchange HTTP {status}: {body}")));
    }

    let token: AccessTokenResponse = resp.json()?;
    let Some(at) = token.access_token.clone().filter(|s| !s.is_empty()) else {
        return Err(OauthError::Msg(
            "Bitbucket returned no access_token (confira o callback URL no OAuth consumer)".into(),
        ));
    };

    let login = fetch_bitbucket_username(&http, &at)?;
    let bundle = BitbucketTokenBundle {
        app_password: at,
        via_oauth: true,
        refresh_token: token.refresh_token.filter(|s| !s.is_empty()),
        expires_at_epoch: token.expires_in.map(|sec| now_epoch() + sec as i64),
    };
    Ok((bundle, login))
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UserRow {
    username: String,
}

fn fetch_bitbucket_username(client: &Client, token: &str) -> Result<String, OauthError> {
    let row: UserRow = client
        .get("https://api.bitbucket.org/2.0/user")
        .header("Authorization", format!("Bearer {token}"))
        .send()?
        .json()?;
    Ok(row.username)
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
