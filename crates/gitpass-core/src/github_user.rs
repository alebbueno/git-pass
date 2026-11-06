//! Authenticated GitHub user: profile, commit email, OAuth scopes and token health (GitHub REST).

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::GithubApiError;

const API_USER: &str = "https://api.github.com/user";
const API_USER_EMAILS: &str = "https://api.github.com/user/emails";
const NOREPLY_DOMAIN: &str = "users.noreply.github.com";

/// Scopes the app needs to push and to pick the commit email.
pub const REQUIRED_SCOPES: &[&str] = &["repo", "user:email"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubUser {
    pub id: u64,
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Public profile email (`null` when the user keeps it private).
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubEmail {
    pub email: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub verified: bool,
    /// `public`, `private` or `null` (GitHub only sets it on the primary address).
    #[serde(default)]
    pub visibility: Option<String>,
}

/// User + emails + scopes fetched together after login or on refresh.
#[derive(Debug, Clone, Serialize)]
pub struct GithubProfile {
    pub user: GithubUser,
    /// `None` when `GET /user/emails` was not allowed (token without `user:email`).
    pub emails: Option<Vec<GithubEmail>>,
    pub commit_email: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TokenCheck {
    Valid { scopes: Vec<String> },
    /// HTTP 401: the token was revoked, expired or the OAuth app was deauthorized.
    Revoked,
}

struct RawResponse {
    status: u16,
    scopes: Option<Vec<String>>,
    body: String,
}

fn client() -> Result<Client, GithubApiError> {
    Ok(Client::builder().user_agent("gitpass-desktop").build()?)
}

fn get(url: &str, token: &str) -> Result<RawResponse, GithubApiError> {
    let resp = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()?;
    let status = resp.status().as_u16();
    let scopes = resp
        .headers()
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .map(parse_scopes);
    let body = resp.text()?;
    Ok(RawResponse { status, scopes, body })
}

fn ensure_ok(r: RawResponse) -> Result<RawResponse, GithubApiError> {
    if (200..300).contains(&r.status) {
        Ok(r)
    } else {
        Err(GithubApiError::Http {
            status: r.status,
            body: crate::github_api::sanitize_error_body(&r.body),
        })
    }
}

/// Splits a scope list as sent by GitHub (`"repo, user:email"` in headers, `"repo,user:email"` in token responses).
pub fn parse_scopes(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    out
}

/// True if `granted` covers `needed`, including GitHub's implied scopes (`user` ⊃ `user:email`, `admin:org` ⊃ `read:org`, …).
pub fn has_scope(granted: &[String], needed: &str) -> bool {
    let implied_by: &[&str] = match needed {
        "user:email" | "read:user" | "user:follow" => &["user"],
        "read:org" => &["write:org", "admin:org"],
        "write:org" => &["admin:org"],
        "public_repo" | "repo:status" | "repo_deployment" | "repo:invite" => &["repo"],
        _ => &[],
    };
    granted
        .iter()
        .any(|g| g == needed || implied_by.contains(&g.as_str()))
}

/// Required scopes ([`REQUIRED_SCOPES`]) missing from `granted`.
pub fn missing_required_scopes(granted: &[String]) -> Vec<String> {
    REQUIRED_SCOPES
        .iter()
        .filter(|s| !has_scope(granted, s))
        .map(|s| s.to_string())
        .collect()
}

/// GitHub's private commit address: `ID+login@users.noreply.github.com`.
pub fn noreply_email(id: u64, login: &str) -> String {
    format!("{id}+{login}@{NOREPLY_DOMAIN}")
}

pub fn is_noreply_email(email: &str) -> bool {
    email
        .rsplit_once('@')
        .map(|(_, d)| d.eq_ignore_ascii_case(NOREPLY_DOMAIN))
        .unwrap_or(false)
}

/// The noreply address GitHub lists for the account (older accounts use `login@…`), or the `ID+login` form.
fn account_noreply(user: &GithubUser, emails: &[GithubEmail]) -> String {
    emails
        .iter()
        .find(|e| is_noreply_email(&e.email))
        .map(|e| e.email.clone())
        .unwrap_or_else(|| noreply_email(user.id, &user.login))
}

/// Commit email to use by default: the primary verified address, unless the user keeps it private
/// (then the noreply address, so pushes are not rejected by "Block command line pushes that expose my email").
/// Without the email list (`None`), falls back to the public profile email, then noreply.
pub fn choose_commit_email(user: &GithubUser, emails: Option<&[GithubEmail]>) -> String {
    let Some(emails) = emails else {
        return user
            .email
            .clone()
            .filter(|e| !e.trim().is_empty())
            .unwrap_or_else(|| noreply_email(user.id, &user.login));
    };
    let primary = emails.iter().find(|e| e.primary && e.verified);
    match primary {
        Some(p) if !is_private(p) => p.email.clone(),
        _ => account_noreply(user, emails),
    }
}

fn is_private(e: &GithubEmail) -> bool {
    e.visibility
        .as_deref()
        .map(|v| v.eq_ignore_ascii_case("private"))
        .unwrap_or(false)
}

/// Addresses the user may pick as commit email: verified ones (primary first) plus the noreply address, without duplicates.
pub fn selectable_emails(user: &GithubUser, emails: Option<&[GithubEmail]>) -> Vec<GithubEmail> {
    let list = emails.unwrap_or(&[]);
    let mut out: Vec<GithubEmail> = Vec::new();
    let mut verified: Vec<&GithubEmail> = list.iter().filter(|e| e.verified).collect();
    verified.sort_by_key(|e| (!e.primary, is_noreply_email(&e.email)));
    for e in verified {
        if !out.iter().any(|o| o.email.eq_ignore_ascii_case(&e.email)) {
            out.push(e.clone());
        }
    }
    if emails.is_none() {
        if let Some(public) = user.email.as_deref().filter(|e| !e.trim().is_empty()) {
            out.push(GithubEmail {
                email: public.to_string(),
                primary: false,
                verified: true,
                visibility: Some("public".into()),
            });
        }
    }
    let noreply = account_noreply(user, list);
    if !out.iter().any(|o| o.email.eq_ignore_ascii_case(&noreply)) {
        out.push(GithubEmail {
            email: noreply,
            primary: false,
            verified: true,
            visibility: None,
        });
    }
    out
}

/// `GET /user`.
pub fn fetch_user(token: &str) -> Result<GithubUser, GithubApiError> {
    let r = ensure_ok(get(API_USER, token)?)?;
    Ok(serde_json::from_str(&r.body)?)
}

/// `GET /user/emails` (needs `user:email` or `user`).
pub fn fetch_emails(token: &str) -> Result<Vec<GithubEmail>, GithubApiError> {
    let r = ensure_ok(get(API_USER_EMAILS, token)?)?;
    Ok(serde_json::from_str(&r.body)?)
}

/// Scopes granted to the token (`X-OAuth-Scopes` header).
pub fn token_scopes(token: &str) -> Result<Vec<String>, GithubApiError> {
    match check_token(token)? {
        TokenCheck::Valid { scopes } => Ok(scopes),
        TokenCheck::Revoked => Err(GithubApiError::Http {
            status: 401,
            body: "Bad credentials".into(),
        }),
    }
}

/// Valid (with scopes) or revoked (401). Other HTTP/network failures are errors.
pub fn check_token(token: &str) -> Result<TokenCheck, GithubApiError> {
    let r = get(API_USER, token)?;
    if r.status == 401 {
        return Ok(TokenCheck::Revoked);
    }
    let scopes = r.scopes.clone().unwrap_or_default();
    ensure_ok(r)?;
    Ok(TokenCheck::Valid { scopes })
}

/// Profile + emails + scopes. A token without `user:email` still yields a profile (email falls back to public/noreply).
pub fn fetch_profile(token: &str) -> Result<GithubProfile, GithubApiError> {
    let r = ensure_ok(get(API_USER, token)?)?;
    let scopes = r.scopes.clone().unwrap_or_default();
    let user: GithubUser = serde_json::from_str(&r.body)?;
    let emails = match fetch_emails(token) {
        Ok(v) => Some(v),
        Err(GithubApiError::Http { status, .. }) if status == 403 || status == 404 => None,
        Err(e) => return Err(e),
    };
    let commit_email = choose_commit_email(&user, emails.as_deref());
    Ok(GithubProfile {
        user,
        emails,
        commit_email,
        scopes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(email: Option<&str>) -> GithubUser {
        GithubUser {
            id: 583231,
            login: "octocat".into(),
            name: Some("The Octocat".into()),
            avatar_url: None,
            email: email.map(String::from),
        }
    }

    fn em(email: &str, primary: bool, verified: bool, visibility: Option<&str>) -> GithubEmail {
        GithubEmail {
            email: email.into(),
            primary,
            verified,
            visibility: visibility.map(String::from),
        }
    }

    #[test]
    fn primary_public_verified_is_chosen() {
        let emails = vec![
            em("other@example.com", false, true, None),
            em("octo@example.com", true, true, Some("public")),
        ];
        assert_eq!(choose_commit_email(&user(None), Some(&emails)), "octo@example.com");
    }

    #[test]
    fn private_primary_uses_listed_noreply() {
        let emails = vec![
            em("octo@example.com", true, true, Some("private")),
            em("583231+octocat@users.noreply.github.com", false, true, None),
        ];
        assert_eq!(
            choose_commit_email(&user(None), Some(&emails)),
            "583231+octocat@users.noreply.github.com"
        );
    }

    #[test]
    fn private_primary_without_listed_noreply_builds_it() {
        let emails = vec![em("octo@example.com", true, true, Some("private"))];
        assert_eq!(
            choose_commit_email(&user(None), Some(&emails)),
            "583231+octocat@users.noreply.github.com"
        );
    }

    #[test]
    fn legacy_noreply_form_is_kept() {
        let emails = vec![
            em("octo@example.com", true, true, Some("private")),
            em("octocat@users.noreply.github.com", false, true, None),
        ];
        assert_eq!(
            choose_commit_email(&user(None), Some(&emails)),
            "octocat@users.noreply.github.com"
        );
    }

    #[test]
    fn unverified_primary_falls_back_to_noreply() {
        let emails = vec![em("octo@example.com", true, false, None)];
        assert_eq!(
            choose_commit_email(&user(None), Some(&emails)),
            "583231+octocat@users.noreply.github.com"
        );
    }

    #[test]
    fn without_email_list_uses_public_then_noreply() {
        assert_eq!(choose_commit_email(&user(Some("pub@example.com")), None), "pub@example.com");
        assert_eq!(
            choose_commit_email(&user(None), None),
            "583231+octocat@users.noreply.github.com"
        );
    }

    #[test]
    fn selectable_lists_verified_primary_first_and_noreply_once() {
        let emails = vec![
            em("b@example.com", false, true, None),
            em("unverified@example.com", false, false, None),
            em("583231+octocat@users.noreply.github.com", false, true, None),
            em("a@example.com", true, true, Some("private")),
        ];
        let got: Vec<String> = selectable_emails(&user(None), Some(&emails))
            .into_iter()
            .map(|e| e.email)
            .collect();
        assert_eq!(
            got,
            vec![
                "a@example.com",
                "b@example.com",
                "583231+octocat@users.noreply.github.com"
            ]
        );
    }

    #[test]
    fn selectable_without_list_has_public_and_noreply() {
        let got: Vec<String> = selectable_emails(&user(Some("pub@example.com")), None)
            .into_iter()
            .map(|e| e.email)
            .collect();
        assert_eq!(got, vec!["pub@example.com", "583231+octocat@users.noreply.github.com"]);
    }

    #[test]
    fn scopes_parse_header_and_token_formats() {
        assert_eq!(parse_scopes("repo, user:email, workflow"), vec!["repo", "user:email", "workflow"]);
        assert_eq!(parse_scopes("repo,read:org"), vec!["repo", "read:org"]);
        assert!(parse_scopes("").is_empty());
    }

    #[test]
    fn implied_scopes_count() {
        let granted = parse_scopes("repo, user, admin:org");
        assert!(has_scope(&granted, "user:email"));
        assert!(has_scope(&granted, "read:org"));
        assert!(has_scope(&granted, "public_repo"));
        assert!(!has_scope(&granted, "workflow"));
        assert!(missing_required_scopes(&granted).is_empty());
        assert_eq!(missing_required_scopes(&parse_scopes("read:user")), vec!["repo", "user:email"]);
    }

    #[test]
    fn noreply_detection() {
        assert!(is_noreply_email("1+x@users.noreply.github.com"));
        assert!(is_noreply_email("x@Users.NoReply.GitHub.com"));
        assert!(!is_noreply_email("x@example.com"));
    }
}
