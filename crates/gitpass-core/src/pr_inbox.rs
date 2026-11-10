//! Pull request inbox via GitHub search API (per account).

use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;

use crate::github_api::sanitize_error_body;
use crate::GithubApiError;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PrInboxItem {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub repo_full_name: String,
    pub updated_at: String,
    /// `review_requested` | `authored`
    pub kind: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    items: Vec<SearchItem>,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    number: u64,
    title: String,
    html_url: String,
    updated_at: String,
    state: String,
    repository_url: Option<String>,
}

pub fn fetch_pr_inbox(token: &str, login: &str) -> Result<Vec<PrInboxItem>, GithubApiError> {
    let login = login.trim();
    if login.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let q_review = format!("is:pr+is:open+review-requested:{login}");
    out.extend(search_kind(token, &q_review, "review_requested")?);
    let q_auth = format!("is:pr+is:open+author:{login}");
    out.extend(search_kind(token, &q_auth, "authored")?);
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out.dedup_by(|a, b| a.html_url == b.html_url);
    Ok(out)
}

fn search_kind(token: &str, q: &str, kind: &str) -> Result<Vec<PrInboxItem>, GithubApiError> {
    let url = format!(
        "https://api.github.com/search/issues?q={q}&sort=updated&order=desc&per_page=20"
    );
    let client = Client::builder()
        .user_agent("gitpass-desktop")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
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
        return Err(GithubApiError::Http {
            status,
            body: sanitize_error_body(&body),
        });
    }
    let parsed: SearchResponse = serde_json::from_str(&body)?;
    Ok(parsed
        .items
        .into_iter()
        .map(|i| PrInboxItem {
            number: i.number,
            title: i.title,
            html_url: i.html_url,
            repo_full_name: repo_from_api_url(i.repository_url.as_deref()),
            updated_at: i.updated_at,
            kind: kind.to_string(),
            state: i.state,
        })
        .collect())
}

fn repo_from_api_url(url: Option<&str>) -> String {
    let Some(u) = url else {
        return String::new();
    };
    u.strip_prefix("https://api.github.com/repos/")
        .unwrap_or(u)
        .to_string()
}
