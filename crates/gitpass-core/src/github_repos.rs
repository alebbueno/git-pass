//! List repositories visible to the authenticated user (GitHub REST).

use reqwest::blocking::Client;
use serde::Deserialize;

use crate::GithubApiError;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
}

/// All repos from `GET /user/repos` (paginated, affiliation=owner,collaborator).
pub fn list_user_repos(token: &str) -> Result<Vec<GithubRepoSummary>, GithubApiError> {
    let client = Client::builder()
        .user_agent("gitpass-desktop")
        .build()?;

    let mut all = Vec::new();
    let mut page: u32 = 1;
    loop {
        let url = format!(
            "https://api.github.com/user/repos?per_page=100&page={}&sort=pushed&affiliation=owner,collaborator,organization_member",
            page
        );
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

        let page_rows: Vec<RepoRow> = serde_json::from_str(&body)?;
        if page_rows.is_empty() {
            break;
        }
        for r in page_rows {
            all.push(GithubRepoSummary {
                full_name: r.full_name,
                name: r.name,
                private: r.private,
                html_url: r.html_url,
                pushed_at: r.pushed_at,
                default_branch: r.default_branch,
            });
        }
        page += 1;
        if page > 50 {
            break; // safety cap (~5000 repos)
        }
    }

    Ok(all)
}
