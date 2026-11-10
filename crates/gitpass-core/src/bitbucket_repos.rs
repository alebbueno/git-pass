//! List Bitbucket Cloud repositories for the authenticated user.

use serde::Deserialize;

use crate::bitbucket_api::{authed_get, BitbucketApiError};

pub const MAX_BITBUCKET_REPOS: usize = 500;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BitbucketRepoSummary {
    pub full_name: String,
    pub name: String,
    pub workspace: String,
    pub slug: String,
    pub html_url: String,
    pub clone_url: String,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    #[serde(default)]
    values: Vec<T>,
    #[serde(default)]
    next: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RepoRow {
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    is_private: bool,
    #[serde(default)]
    description: Option<String>,
    links: RepoLinks,
    workspace: WorkspaceRow,
}

#[derive(Debug, Default, Deserialize)]
struct WorkspaceRow {
    #[serde(default)]
    slug: String,
}

#[derive(Debug, Default, Deserialize)]
struct RepoLinks {
    #[serde(default)]
    html: Option<LinkHref>,
    #[serde(default)]
    clone: Vec<CloneLink>,
}

#[derive(Debug, Default, Deserialize)]
struct LinkHref {
    href: String,
}

#[derive(Debug, Default, Deserialize)]
struct CloneLink {
    name: String,
    href: String,
}

impl From<RepoRow> for BitbucketRepoSummary {
    fn from(r: RepoRow) -> Self {
        let workspace = r.workspace.slug;
        let full_name = format!("{workspace}/{}", r.slug);
        let clone_url = r
            .links
            .clone
            .iter()
            .find(|c| c.name == "https")
            .map(|c| c.href.clone())
            .unwrap_or_else(|| format!("https://bitbucket.org/{workspace}/{}.git", r.slug));
        let html_url = r
            .links
            .html
            .map(|h| h.href)
            .unwrap_or_else(|| format!("https://bitbucket.org/{workspace}/{}", r.slug));
        Self {
            full_name,
            name: r.name,
            workspace,
            slug: r.slug,
            html_url,
            clone_url,
            is_private: r.is_private,
            description: r.description.filter(|d| !d.trim().is_empty()),
        }
    }
}

/// Repositories in `workspace` visible to the user (typically their username workspace).
pub fn list_bitbucket_repos(
    username: &str,
    app_password: &str,
    workspace: &str,
    via_oauth: bool,
) -> Result<Vec<BitbucketRepoSummary>, BitbucketApiError> {
    let mut out: Vec<BitbucketRepoSummary> = Vec::new();
    let mut url = Some(format!(
        "https://api.bitbucket.org/2.0/repositories/{workspace}?pagelen=100"
    ));
    while let Some(u) = url {
        if out.len() >= MAX_BITBUCKET_REPOS {
            break;
        }
        let body = authed_get(&u, username, app_password, via_oauth)?;
        let page: Page<RepoRow> = serde_json::from_str(&body)?;
        for row in page.values {
            out.push(row.into());
            if out.len() >= MAX_BITBUCKET_REPOS {
                break;
            }
        }
        url = page.next;
    }
    Ok(out)
}
