//! Bitbucket Cloud — pull requests (listar, criar, detalhar, merge).

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::bitbucket_api::{authed_get, authed_post_json, BitbucketApiError};
use crate::bitbucket_repos::list_bitbucket_repos;
use crate::github_pr::{
    CreatePullRequestInput, MergePullRequestResult, PullRequestCreated, PullRequestDetail, RepoPullRequestRow,
};
use crate::pr_inbox::PrInboxItem;

fn bb_repo(workspace: &str, slug: &str) -> String {
    format!(
        "https://api.bitbucket.org/2.0/repositories/{}/{}",
        workspace.trim(),
        slug.trim()
    )
}

#[derive(Debug, Deserialize)]
struct BbPrListPage {
    #[serde(default)]
    values: Vec<BbPrRow>,
}

#[derive(Debug, Default, Deserialize)]
struct BbBranchRef {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct BbRepoSnippet {
    #[serde(default)]
    full_name: String,
}

#[derive(Debug, Default, Deserialize)]
struct BbPrEndpoint {
    #[serde(default)]
    branch: BbBranchRef,
    #[serde(default)]
    repository: BbRepoSnippet,
}

#[derive(Debug, Default, Deserialize)]
struct BbReviewer {
    #[serde(default)]
    nickname: String,
}

#[derive(Debug, Deserialize)]
struct BbLinkHref {
    href: String,
}

#[derive(Debug, Default, Deserialize)]
struct BbLinks {
    #[serde(default)]
    html: Option<BbLinkHref>,
}

#[derive(Debug, Deserialize)]
struct BbAuthor {
    #[serde(default)]
    nickname: String,
}

#[derive(Debug, Deserialize)]
struct BbPrRow {
    id: u64,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    source: BbPrEndpoint,
    #[serde(default)]
    destination: BbPrEndpoint,
    #[serde(default)]
    author: Option<BbAuthor>,
    #[serde(default)]
    updated_on: String,
    links: BbLinks,
    #[serde(default)]
    reviewers: Vec<BbReviewer>,
}

#[derive(Debug, Deserialize)]
struct BbMergeBody {
    #[serde(default)]
    state: String,
}

pub fn list_bitbucket_pull_requests(
    username: &str,
    token: &str,
    via_oauth: bool,
    workspace: &str,
    slug: &str,
) -> Result<Vec<RepoPullRequestRow>, BitbucketApiError> {
    let url = format!("{}/pullrequests?state=OPEN&pagelen=30", bb_repo(workspace, slug));
    let raw = authed_get(&url, username, token, via_oauth)?;
    let page: BbPrListPage = serde_json::from_str(&raw)?;
    Ok(page.values.into_iter().map(map_row).collect())
}

fn repo_full_name_of(pr: &BbPrRow) -> String {
    let from_dest = pr.destination.repository.full_name.trim();
    if !from_dest.is_empty() {
        return from_dest.to_string();
    }
    let from_src = pr.source.repository.full_name.trim();
    if !from_src.is_empty() {
        return from_src.to_string();
    }
    String::new()
}

fn pr_to_inbox(pr: &BbPrRow, repo_full_name: &str, kind: &str) -> PrInboxItem {
    let html_url = pr
        .links
        .html
        .as_ref()
        .map(|h| h.href.clone())
        .unwrap_or_default();
    PrInboxItem {
        number: pr.id,
        title: pr.title.clone(),
        html_url,
        repo_full_name: repo_full_name.to_string(),
        updated_at: pr.updated_on.clone(),
        kind: kind.to_string(),
        state: pr.state.to_lowercase(),
    }
}

const INBOX_TTL: Duration = Duration::from_secs(45);
const REVIEW_REPO_SCAN_CAP: usize = 22;

static INBOX_CACHE: LazyLock<Mutex<HashMap<String, (Instant, Vec<PrInboxItem>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn inbox_cache_key(user: &str, bound: &[String]) -> String {
    let mut parts: Vec<&str> = bound.iter().map(|s| s.as_str()).collect();
    parts.sort_unstable();
    format!("{user}|{}", parts.join(","))
}

fn encode_bb_query(q: &str) -> String {
    q.replace(' ', "%20").replace('"', "%22")
}

fn review_prs_in_repo(
    username: &str,
    token: &str,
    via_oauth: bool,
    user: &str,
    repo_full_name: &str,
) -> Result<Vec<PrInboxItem>, BitbucketApiError> {
    let q = format!(r#"reviewers.nickname="{user}" AND state="OPEN""#);
    let url = format!(
        "https://api.bitbucket.org/2.0/repositories/{repo_full_name}/pullrequests?q={}&pagelen=12",
        encode_bb_query(&q)
    );
    let raw = authed_get(&url, username, token, via_oauth)?;
    let page: BbPrListPage = serde_json::from_str(&raw)?;
    Ok(page
        .values
        .iter()
        .filter(|pr| {
            let author = pr.author.as_ref().map(|a| a.nickname.as_str()).unwrap_or("");
            !author.eq_ignore_ascii_case(user)
        })
        .map(|pr| pr_to_inbox(pr, repo_full_name, "review_requested"))
        .collect())
}

fn fetch_bitbucket_pr_inbox_uncached(
    username: &str,
    token: &str,
    via_oauth: bool,
    bound_repos: &[String],
) -> Result<Vec<PrInboxItem>, BitbucketApiError> {
    let user = username.trim();
    let mut out: Vec<PrInboxItem> = Vec::new();
    let authored_url = format!(
        "https://api.bitbucket.org/2.0/workspaces/{user}/pullrequests/{user}?state=OPEN&pagelen=25"
    );
    if let Ok(raw) = authed_get(&authored_url, username, token, via_oauth) {
        if let Ok(page) = serde_json::from_str::<BbPrListPage>(&raw) {
            for pr in &page.values {
                let repo = repo_full_name_of(pr);
                if repo.is_empty() {
                    continue;
                }
                out.push(pr_to_inbox(pr, &repo, "authored"));
            }
        }
    }
    let all_repos = list_bitbucket_repos(username, token, user, via_oauth)?;
    let mut scan_order: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for b in bound_repos {
        let name = b.trim();
        if name.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        scan_order.push(name.to_string());
    }
    for r in all_repos {
        if scan_order.len() >= REVIEW_REPO_SCAN_CAP {
            break;
        }
        if seen.insert(r.full_name.clone()) {
            scan_order.push(r.full_name);
        }
    }
    for repo in scan_order {
        if let Ok(mut rows) = review_prs_in_repo(username, token, via_oauth, user, &repo) {
            out.append(&mut rows);
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out.dedup_by(|a, b| a.html_url == b.html_url && a.number == b.number);
    Ok(out)
}

/// Inbox com cache (~45s). `bound_repos` são vinculados no GitPass (varridos primeiro).
pub fn fetch_bitbucket_pr_inbox(
    username: &str,
    token: &str,
    via_oauth: bool,
    bound_repos: &[String],
    force_refresh: bool,
) -> Result<Vec<PrInboxItem>, BitbucketApiError> {
    let user = username.trim();
    let key = inbox_cache_key(user, bound_repos);
    if !force_refresh {
        if let Ok(guard) = INBOX_CACHE.lock() {
            if let Some((at, items)) = guard.get(&key) {
                if at.elapsed() < INBOX_TTL {
                    return Ok(items.clone());
                }
            }
        }
    }
    let items = fetch_bitbucket_pr_inbox_uncached(username, token, via_oauth, bound_repos)?;
    if let Ok(mut guard) = INBOX_CACHE.lock() {
        guard.insert(key, (Instant::now(), items.clone()));
    }
    Ok(items)
}

pub fn approve_bitbucket_pull_request(
    username: &str,
    token: &str,
    via_oauth: bool,
    workspace: &str,
    slug: &str,
    id: u64,
) -> Result<(), BitbucketApiError> {
    let url = format!("{}/pullrequests/{id}/approve", bb_repo(workspace, slug));
    let _ = crate::bitbucket_api::authed_post_empty(&url, username, token, via_oauth)?;
    Ok(())
}

pub fn fetch_bitbucket_pull_request_diff(
    username: &str,
    token: &str,
    via_oauth: bool,
    workspace: &str,
    slug: &str,
    id: u64,
) -> Result<String, BitbucketApiError> {
    let url = format!("{}/pullrequests/{id}/diff", bb_repo(workspace, slug));
    let raw = authed_get(&url, username, token, via_oauth)?;
    if raw.trim_start().starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(s) = v.get("diff").and_then(|d| d.as_str()) {
                return Ok(s.to_string());
            }
        }
    }
    Ok(raw)
}

fn map_row(pr: BbPrRow) -> RepoPullRequestRow {
    let html_url = pr
        .links
        .html
        .map(|h| h.href)
        .unwrap_or_default();
    RepoPullRequestRow {
        number: pr.id,
        title: pr.title,
        html_url,
        state: pr.state.to_lowercase(),
        draft: pr.draft,
        head_ref: pr.source.branch.name,
        base_ref: pr.destination.branch.name,
        author_login: pr.author.as_ref().map(|a| a.nickname.clone()).unwrap_or_default(),
        updated_at: pr.updated_on,
    }
}

pub fn create_bitbucket_pull_request(
    username: &str,
    token: &str,
    via_oauth: bool,
    workspace: &str,
    slug: &str,
    input: CreatePullRequestInput,
) -> Result<PullRequestCreated, BitbucketApiError> {
    let url = format!("{}/pullrequests", bb_repo(workspace, slug));
    let body = serde_json::json!({
        "title": input.title.trim(),
        "source": { "branch": { "name": input.head.trim() } },
        "destination": { "branch": { "name": input.base.trim() } },
        "description": input.body.as_deref().unwrap_or("").trim(),
        "close_source_branch": false,
    });
    let raw = authed_post_json(&url, username, token, via_oauth, body)?;
    let pr: BbPrRow = serde_json::from_str(&raw)?;
    Ok(PullRequestCreated {
        number: pr.id,
        title: pr.title,
        html_url: pr.links.html.map(|h| h.href).unwrap_or_default(),
        state: pr.state.to_lowercase(),
        draft: pr.draft,
        head_ref: pr.source.branch.name,
        base_ref: pr.destination.branch.name,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BitbucketPullRequestDetail {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub head_ref: String,
    pub base_ref: String,
    pub author_login: String,
    pub can_merge: bool,
    pub can_review: bool,
    pub repo_full_name: String,
}

pub fn fetch_bitbucket_pull_request_detail(
    username: &str,
    token: &str,
    via_oauth: bool,
    workspace: &str,
    slug: &str,
    id: u64,
) -> Result<BitbucketPullRequestDetail, BitbucketApiError> {
    let url = format!("{}/pullrequests/{}", bb_repo(workspace, slug), id);
    let raw = authed_get(&url, username, token, via_oauth)?;
    let pr: BbPrRow = serde_json::from_str(&raw)?;
    let open = pr.state.eq_ignore_ascii_case("OPEN");
    let author = pr.author.as_ref().map(|a| a.nickname.as_str()).unwrap_or("");
    let is_reviewer = pr
        .reviewers
        .iter()
        .any(|r| r.nickname.eq_ignore_ascii_case(username));
    let can_review = open && is_reviewer && !author.eq_ignore_ascii_case(username);
    Ok(BitbucketPullRequestDetail {
        number: pr.id,
        title: pr.title,
        body: pr.description.filter(|s| !s.trim().is_empty()),
        state: pr.state.to_lowercase(),
        html_url: pr.links.html.map(|h| h.href).unwrap_or_default(),
        head_ref: pr.source.branch.name,
        base_ref: pr.destination.branch.name,
        author_login: pr.author.as_ref().map(|a| a.nickname.clone()).unwrap_or_default(),
        can_merge: open,
        can_review,
        repo_full_name: format!("{workspace}/{slug}"),
    })
}

pub fn merge_bitbucket_pull_request(
    username: &str,
    token: &str,
    via_oauth: bool,
    workspace: &str,
    slug: &str,
    id: u64,
) -> Result<MergePullRequestResult, BitbucketApiError> {
    let url = format!("{}/pullrequests/{}/merge", bb_repo(workspace, slug), id);
    let raw = authed_post_json(&url, username, token, via_oauth, serde_json::json!({}))?;
    let _: BbMergeBody = serde_json::from_str(&raw).unwrap_or(BbMergeBody {
        state: "MERGED".into(),
    });
    Ok(MergePullRequestResult {
        merged: true,
        message: "Pull request mesclado.".into(),
        sha: None,
    })
}

/// Converte detalhe Bitbucket para o painel unificado (campos GitHub ausentes ficam vazios).
impl From<BitbucketPullRequestDetail> for PullRequestDetail {
    fn from(b: BitbucketPullRequestDetail) -> Self {
        PullRequestDetail {
            number: b.number,
            title: b.title,
            body: b.body,
            state: b.state.clone(),
            html_url: b.html_url,
            draft: false,
            merged: b.state == "merged",
            mergeable: Some(b.can_merge),
            head_ref: b.head_ref,
            base_ref: b.base_ref,
            author_login: b.author_login,
            additions: 0,
            deletions: 0,
            changed_files: 0,
            viewer_login: String::new(),
            permissions: crate::github_api::RepoPermissions {
                admin: false,
                maintain: false,
                push: b.can_merge,
                triage: false,
                pull: true,
            },
            reviews: vec![],
            can_merge: b.can_merge,
            can_review: b.can_review,
            repo_full_name: b.repo_full_name,
            head_sha: String::new(),
            checks: vec![],
            comments: vec![],
            requested_reviewers: vec![],
            can_request_reviewers: false,
            auto_merge_enabled: false,
            can_enable_auto_merge: false,
        }
    }
}
