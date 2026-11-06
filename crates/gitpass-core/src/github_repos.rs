//! List repositories visible to the authenticated user (GitHub REST).

use reqwest::blocking::Client;
use serde::Deserialize;

use crate::GithubApiError;

/// Maximum number of repositories returned by [`list_user_repos`].
pub const MAX_REPOS: usize = 1000;

const PER_PAGE: u32 = 100;
const API_HOST_PREFIX: &str = "https://api.github.com/";

/// Effective permissions of the authenticated user on a repository (`permissions` of `/user/repos`).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GithubRepoPermissions {
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

impl GithubRepoPermissions {
    /// True when the user can push (admin, maintain or push).
    pub fn can_write(&self) -> bool {
        self.admin || self.maintain || self.push
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GithubRepoSummary {
    pub full_name: String,
    pub name: String,
    #[serde(default)]
    pub private: bool,
    pub html_url: String,
    #[serde(default)]
    pub pushed_at: Option<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub permissions: GithubRepoPermissions,
    /// Login of the owner (user or organisation).
    #[serde(default)]
    pub owner_login: String,
    /// `User` or `Organization` (as returned by GitHub).
    #[serde(default)]
    pub owner_type: String,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub description: Option<String>,
    /// HTTPS clone URL (`https://github.com/owner/repo.git`).
    #[serde(default)]
    pub clone_url: String,
}

#[derive(Debug, Deserialize)]
struct OwnerRow {
    #[serde(default)]
    login: String,
    #[serde(default, rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct RepoRow {
    full_name: String,
    name: String,
    #[serde(default)]
    private: bool,
    html_url: String,
    #[serde(default)]
    pushed_at: Option<String>,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    permissions: Option<GithubRepoPermissions>,
    #[serde(default)]
    owner: Option<OwnerRow>,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    clone_url: Option<String>,
}

impl From<RepoRow> for GithubRepoSummary {
    fn from(r: RepoRow) -> Self {
        let (owner_login, owner_type) = match r.owner {
            Some(o) if !o.login.is_empty() => (o.login, o.kind),
            _ => (
                r.full_name.split('/').next().unwrap_or_default().to_string(),
                String::new(),
            ),
        };
        let clone_url = r
            .clone_url
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| format!("https://github.com/{}.git", r.full_name));
        GithubRepoSummary {
            full_name: r.full_name,
            name: r.name,
            private: r.private,
            html_url: r.html_url,
            pushed_at: r.pushed_at,
            default_branch: r.default_branch,
            permissions: r.permissions.unwrap_or_default(),
            owner_login,
            owner_type,
            fork: r.fork,
            archived: r.archived,
            description: r.description.filter(|d| !d.trim().is_empty()),
            clone_url,
        }
    }
}

/// Extracts the `rel="next"` URL from a `Link` response header.
pub fn parse_next_link(header: &str) -> Option<String> {
    for part in header.split(',') {
        let part = part.trim();
        let Some((url_part, params)) = part.split_once(';') else {
            continue;
        };
        let is_next = params
            .split(';')
            .any(|p| p.trim().replace(' ', "") == "rel=\"next\"");
        if !is_next {
            continue;
        }
        if let Some(url) = url_part.trim().strip_prefix('<').and_then(|u| u.strip_suffix('>')) {
            return Some(url.to_string());
        }
    }
    None
}

/// All repos from `GET /user/repos` (`per_page=100`, follows `Link: rel="next"`, up to [`MAX_REPOS`]).
pub fn list_user_repos(token: &str) -> Result<Vec<GithubRepoSummary>, GithubApiError> {
    let client = Client::builder().user_agent("gitpass-desktop").build()?;

    let mut all: Vec<GithubRepoSummary> = Vec::new();
    let mut next = Some(format!(
        "{API_HOST_PREFIX}user/repos?per_page={PER_PAGE}&sort=pushed&affiliation=owner,collaborator,organization_member"
    ));

    while let Some(url) = next.take() {
        let resp = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {token}"))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()?;

        let status = resp.status().as_u16();
        // Never follow a `next` link to another host: the token would leak.
        let link = resp
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_next_link)
            .filter(|u| u.starts_with(API_HOST_PREFIX));
        let body = resp.text()?;
        if !(200..300).contains(&status) {
            return Err(GithubApiError::Http {
                status,
                body: crate::github_api::sanitize_error_body(&body),
            });
        }

        let rows: Vec<RepoRow> = serde_json::from_str(&body)?;
        if rows.is_empty() {
            break;
        }
        all.extend(rows.into_iter().map(GithubRepoSummary::from));
        if all.len() >= MAX_REPOS {
            all.truncate(MAX_REPOS);
            break;
        }
        next = link;
    }

    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_link_is_found() {
        let h = r#"<https://api.github.com/user/repos?per_page=100&page=2>; rel="next", <https://api.github.com/user/repos?per_page=100&page=5>; rel="last""#;
        assert_eq!(
            parse_next_link(h).as_deref(),
            Some("https://api.github.com/user/repos?per_page=100&page=2")
        );
    }

    #[test]
    fn next_link_absent_on_last_page() {
        let h = r#"<https://api.github.com/user/repos?page=1>; rel="prev", <https://api.github.com/user/repos?page=1>; rel="first""#;
        assert_eq!(parse_next_link(h), None);
        assert_eq!(parse_next_link(""), None);
    }

    #[test]
    fn next_link_after_prev() {
        let h = r#"<https://api.github.com/x?page=1>; rel="prev", <https://api.github.com/x?page=3>; rel="next""#;
        assert_eq!(parse_next_link(h).as_deref(), Some("https://api.github.com/x?page=3"));
    }

    #[test]
    fn row_maps_all_fields() {
        let json = r#"{
            "full_name": "acme/widget", "name": "widget", "private": true,
            "html_url": "https://github.com/acme/widget",
            "pushed_at": "2026-01-01T00:00:00Z", "default_branch": "main",
            "permissions": {"admin": false, "maintain": false, "push": true, "triage": true, "pull": true},
            "owner": {"login": "acme", "type": "Organization"},
            "fork": true, "archived": true, "description": "Um widget",
            "clone_url": "https://github.com/acme/widget.git"
        }"#;
        let s: GithubRepoSummary = serde_json::from_str::<RepoRow>(json).unwrap().into();
        assert_eq!(s.owner_login, "acme");
        assert_eq!(s.owner_type, "Organization");
        assert!(s.permissions.push && s.permissions.can_write() && !s.permissions.admin);
        assert!(s.fork && s.archived && s.private);
        assert_eq!(s.description.as_deref(), Some("Um widget"));
        assert_eq!(s.clone_url, "https://github.com/acme/widget.git");
    }

    #[test]
    fn row_without_optional_fields_falls_back() {
        let json = r#"{"full_name": "me/repo", "name": "repo", "html_url": "https://github.com/me/repo", "description": "  "}"#;
        let s: GithubRepoSummary = serde_json::from_str::<RepoRow>(json).unwrap().into();
        assert_eq!(s.owner_login, "me");
        assert_eq!(s.clone_url, "https://github.com/me/repo.git");
        assert_eq!(s.permissions, GithubRepoPermissions::default());
        assert_eq!(s.description, None);
    }

    #[test]
    fn summary_deserializes_from_old_shape() {
        let json = r#"{"full_name": "me/repo", "name": "repo", "private": false, "html_url": "https://github.com/me/repo"}"#;
        let s: GithubRepoSummary = serde_json::from_str(json).unwrap();
        assert!(!s.fork && s.clone_url.is_empty());
    }
}
