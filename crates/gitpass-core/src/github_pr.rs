//! GitHub Pull Requests — criar, detalhar, revisar e fazer merge.

use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::clone::split_full_name;
use crate::github_api::{fetch_repo_permissions, sanitize_error_body, RepoPermissions, GithubApiError};
use crate::github_user::fetch_user;

const GH_ACCEPT: &str = "application/vnd.github+json";
const GH_VERSION: &str = "2022-11-28";

fn client() -> Result<Client, GithubApiError> {
    Ok(Client::builder()
        .user_agent("GitPass")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()?)
}

fn validate_owner_repo(owner: &str, repo: &str) -> Result<(), GithubApiError> {
    split_full_name(&format!("{owner}/{repo}")).map_err(|_| GithubApiError::Http {
        status: 400,
        body: "Nome de repositório inválido.".into(),
    })?;
    Ok(())
}

fn request(token: &str, method: reqwest::Method, url: &str, body: Option<serde_json::Value>) -> Result<String, GithubApiError> {
    let client = client()?;
    let mut req = client
        .request(method, url)
        .header("Accept", GH_ACCEPT)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-GitHub-Api-Version", GH_VERSION);
    if let Some(json) = body {
        req = req.json(&json);
    }
    let resp = req.send()?;
    let status = resp.status().as_u16();
    let text = resp.text()?;
    if !(200..300).contains(&status) {
        return Err(GithubApiError::Http {
            status,
            body: sanitize_error_body(&text),
        });
    }
    Ok(text)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePullRequestInput {
    pub title: String,
    pub head: String,
    pub base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestCreated {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub state: String,
    pub draft: bool,
    pub head_ref: String,
    pub base_ref: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewRow {
    pub id: u64,
    pub user_login: String,
    pub state: String,
    pub body: Option<String>,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub draft: bool,
    pub merged: bool,
    pub mergeable: Option<bool>,
    pub head_ref: String,
    pub base_ref: String,
    pub author_login: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub viewer_login: String,
    pub permissions: RepoPermissions,
    pub reviews: Vec<PrReviewRow>,
    pub can_merge: bool,
    pub can_review: bool,
    pub repo_full_name: String,
    pub head_sha: String,
    pub checks: Vec<PrCheckRow>,
    pub comments: Vec<PrReviewComment>,
    pub requested_reviewers: Vec<String>,
    pub can_request_reviewers: bool,
    pub auto_merge_enabled: bool,
    pub can_enable_auto_merge: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommentSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCheckRow {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewComment {
    pub id: u64,
    pub user_login: String,
    pub body: String,
    pub path: String,
    pub line: Option<u64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePullRequestResult {
    pub merged: bool,
    pub message: String,
    pub sha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PrApiUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct PrApiRef {
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    sha: String,
}

#[derive(Debug, Deserialize)]
struct PrApiResponse {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: String,
    draft: bool,
    merged: bool,
    mergeable: Option<bool>,
    head: PrApiRef,
    base: PrApiRef,
    user: PrApiUser,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    #[serde(default)]
    changed_files: u64,
    #[serde(default)]
    requested_reviewers: Vec<PrApiUser>,
    #[serde(default)]
    node_id: String,
    auto_merge: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ReviewApiRow {
    id: u64,
    user: PrApiUser,
    state: String,
    body: Option<String>,
    submitted_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergeApiResponse {
    merged: bool,
    message: String,
    sha: Option<String>,
}

pub fn create_pull_request(
    token: &str,
    owner: &str,
    repo: &str,
    input: CreatePullRequestInput,
) -> Result<PullRequestCreated, GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls");
    let body = serde_json::json!({
        "title": input.title.trim(),
        "head": input.head.trim(),
        "base": input.base.trim(),
        "body": input.body.as_deref().unwrap_or("").trim(),
        "draft": input.draft,
    });
    let raw = request(token, reqwest::Method::POST, &url, Some(body))?;
    let pr: PrApiResponse = serde_json::from_str(&raw)?;
    Ok(PullRequestCreated {
        number: pr.number,
        title: pr.title,
        html_url: pr.html_url,
        state: pr.state,
        draft: pr.draft,
        head_ref: pr.head.r#ref,
        base_ref: pr.base.r#ref,
    })
}

pub fn fetch_pull_request_detail(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<PullRequestDetail, GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let viewer = fetch_user(token)?.login;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let pr: PrApiResponse = serde_json::from_str(&raw)?;
    let head_sha = pr.head.sha.clone();
    let reviews = list_reviews(token, owner, repo, number)?;
    let comments = list_review_comments(token, owner, repo, number)?;
    let checks = if head_sha.is_empty() {
        Vec::new()
    } else {
        fetch_commit_checks(token, owner, repo, &head_sha)?
    };
    let permissions = fetch_repo_permissions(token, owner, repo)?;
    let open = pr.state == "open";
    let mergeable = pr.mergeable.unwrap_or(false);
    let checks_ok = !checks.iter().any(|c| {
        c.status == "completed"
            && matches!(
                c.conclusion.as_deref(),
                Some("failure") | Some("timed_out") | Some("action_required")
            )
    });
    let can_merge_perm = permissions.admin || permissions.maintain;
    let can_merge = open && mergeable && checks_ok && can_merge_perm;
    let can_enable_auto_merge = open && mergeable && checks_ok && can_merge_perm;
    let is_author = pr.user.login.eq_ignore_ascii_case(&viewer);
    let can_review = open && !is_author && (permissions.admin || permissions.maintain || permissions.push || permissions.triage);
    let can_request_reviewers = open && (is_author || permissions.admin || permissions.maintain || permissions.push);
    let requested_reviewers: Vec<String> = pr.requested_reviewers.into_iter().map(|u| u.login).collect();
    Ok(PullRequestDetail {
        number: pr.number,
        title: pr.title,
        body: pr.body.filter(|s| !s.trim().is_empty()),
        state: pr.state,
        html_url: pr.html_url,
        draft: pr.draft,
        merged: pr.merged,
        mergeable: pr.mergeable,
        head_ref: pr.head.r#ref,
        base_ref: pr.base.r#ref,
        author_login: pr.user.login,
        additions: pr.additions,
        deletions: pr.deletions,
        changed_files: pr.changed_files,
        viewer_login: viewer,
        permissions,
        reviews,
        can_merge,
        can_review,
        repo_full_name: format!("{owner}/{repo}"),
        head_sha,
        checks,
        comments,
        requested_reviewers,
        can_request_reviewers,
        auto_merge_enabled: pr.auto_merge.is_some(),
        can_enable_auto_merge,
    })
}

#[derive(Debug, Deserialize)]
struct CheckRunsEnvelope {
    #[serde(default)]
    check_runs: Vec<CheckRunApiRow>,
}

#[derive(Debug, Deserialize)]
struct CheckRunApiRow {
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CombinedStatus {
  state: String,
  #[serde(default)]
  statuses: Vec<LegacyStatusRow>,
}

#[derive(Debug, Deserialize)]
struct LegacyStatusRow {
  context: String,
  state: String,
  target_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReviewCommentApiRow {
    id: u64,
    user: PrApiUser,
    body: String,
    path: String,
    line: Option<u64>,
    created_at: String,
}

fn fetch_commit_checks(
    token: &str,
    owner: &str,
    repo: &str,
    sha: &str,
) -> Result<Vec<PrCheckRow>, GithubApiError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/commits/{sha}/check-runs?per_page=100");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let parsed: CheckRunsEnvelope = serde_json::from_str(&raw)?;
    if !parsed.check_runs.is_empty() {
        return Ok(parsed
            .check_runs
            .into_iter()
            .map(|c| PrCheckRow {
                name: c.name,
                status: c.status,
                conclusion: c.conclusion,
                html_url: c.html_url,
            })
            .collect());
    }
    let url = format!("https://api.github.com/repos/{owner}/{repo}/commits/{sha}/status");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let st: CombinedStatus = serde_json::from_str(&raw)?;
    if st.statuses.is_empty() && st.state != "pending" {
        return Ok(vec![PrCheckRow {
            name: "Status combinado".into(),
            status: "completed".into(),
            conclusion: Some(st.state),
            html_url: None,
        }]);
    }
    Ok(st.statuses
        .into_iter()
        .map(|s| PrCheckRow {
            name: s.context,
            status: "completed".into(),
            conclusion: Some(s.state),
            html_url: s.target_url,
        })
        .collect())
}

fn list_review_comments(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Vec<PrReviewComment>, GithubApiError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments?per_page=100");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let rows: Vec<ReviewCommentApiRow> = serde_json::from_str(&raw)?;
    Ok(rows
        .into_iter()
        .map(|c| PrReviewComment {
            id: c.id,
            user_login: c.user.login,
            body: c.body,
            path: c.path,
            line: c.line,
            created_at: c.created_at,
        })
        .collect())
}

/// Primeira linha adicionada no patch (lado RIGHT) para ancorar comentário de revisão.
pub fn anchor_line_in_patch(patch: &str) -> Option<u32> {
    let mut right_line = 0u32;
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            right_line = parse_hunk_start(line).unwrap_or(0);
            continue;
        }
        if !in_hunk {
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            return Some(right_line);
        }
        if line.starts_with(' ') {
            right_line = right_line.saturating_add(1);
        }
    }
    None
}

fn parse_hunk_start(header: &str) -> Option<u32> {
    let plus = header.split('+').nth(1)?;
    let num = plus.split_whitespace().next()?;
    let base = num.split(',').next()?;
    base.parse().ok()
}

/// A linha existe no lado RIGHT do patch (contexto ou adição)?
pub fn line_exists_on_right_side(patch: &str, target: u32) -> bool {
    if target == 0 {
        return false;
    }
    let mut right_line = 0u32;
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            right_line = parse_hunk_start(line).unwrap_or(0);
            continue;
        }
        if !in_hunk || line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('-') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('+') {
            if right_line == target {
                return true;
            }
            right_line = right_line.saturating_add(1);
        }
    }
    false
}

pub fn request_pull_request_reviewers(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    reviewers: &[String],
) -> Result<(), GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let logins: Vec<String> = reviewers
        .iter()
        .map(|s| s.trim().trim_start_matches('@').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if logins.is_empty() {
        return Err(GithubApiError::Http {
            status: 400,
            body: "Informe ao menos um usuário (@login).".into(),
        });
    }
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/requested_reviewers");
    let body = serde_json::json!({ "reviewers": logins });
    let _ = request(token, reqwest::Method::POST, &url, Some(body))?;
    Ok(())
}

/// A linha existe no lado LEFT (remoções e contexto antigo)?
pub fn line_exists_on_left_side(patch: &str, target: u32) -> bool {
    if target == 0 {
        return false;
    }
    let mut left_line = 0u32;
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            left_line = parse_hunk_start_left(line).unwrap_or(0);
            continue;
        }
        if !in_hunk || line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('-') {
            if left_line == target {
                return true;
            }
            left_line = left_line.saturating_add(1);
        }
    }
    false
}

fn parse_hunk_start_left(header: &str) -> Option<u32> {
    let minus = header.split('-').nth(1)?;
    let num = minus.split_whitespace().next()?;
    let base = num.split(',').next()?;
    base.parse().ok()
}

pub fn enable_pull_request_auto_merge(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    method: MergeMethod,
) -> Result<(), GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let pr: PrApiResponse = serde_json::from_str(&raw)?;
    let node_id = pr.node_id.trim();
    if node_id.is_empty() {
        return Err(GithubApiError::Http {
            status: 400,
            body: "GitHub não enviou o ID do PR para auto-merge.".into(),
        });
    }
    let merge_method = match method {
        MergeMethod::Merge => "MERGE",
        MergeMethod::Squash => "SQUASH",
        MergeMethod::Rebase => "REBASE",
    };
    let query = r#"mutation($id: ID!, $method: PullRequestMergeMethod!) {
  enablePullRequestAutoMerge(input: {pullRequestId: $id, mergeMethod: $method}) {
    pullRequest { number }
  }
}"#;
  graphql_request(
        token,
        query,
        serde_json::json!({ "id": node_id, "method": merge_method }),
    )
}

pub fn disable_pull_request_auto_merge(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<(), GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let pr: PrApiResponse = serde_json::from_str(&raw)?;
    let node_id = pr.node_id.trim();
    if node_id.is_empty() {
        return Err(GithubApiError::Http {
            status: 400,
            body: "GitHub não enviou o ID do PR.".into(),
        });
    }
    let query = r#"mutation($id: ID!) {
  disablePullRequestAutoMerge(input: {pullRequestId: $id}) {
    pullRequest { number }
  }
}"#;
    graphql_request(token, query, serde_json::json!({ "id": node_id }))
}

fn graphql_request(token: &str, query: &str, variables: serde_json::Value) -> Result<(), GithubApiError> {
    let client = client()?;
    let body = serde_json::json!({ "query": query, "variables": variables });
    let resp = client
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()?;
    let status = resp.status().as_u16();
    let text = resp.text()?;
    if !(200..300).contains(&status) {
        return Err(GithubApiError::Http {
            status,
            body: sanitize_error_body(&text),
        });
    }
    let v: serde_json::Value = serde_json::from_str(&text)?;
    if let Some(errs) = v.get("errors") {
        let msg = errs
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("Erro GraphQL.");
        return Err(GithubApiError::Http {
            status: 400,
            body: msg.to_string(),
        });
    }
    Ok(())
}

pub fn comment_on_pull_request_file(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    head_sha: &str,
    path: &str,
    body: &str,
    line: Option<u32>,
    side: CommentSide,
) -> Result<(), GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let text = body.trim();
    if text.is_empty() {
        return Err(GithubApiError::Http {
            status: 400,
            body: "Escreva o comentário.".into(),
        });
    }
    if head_sha.trim().is_empty() {
        return Err(GithubApiError::Http {
            status: 400,
            body: "Commit do PR indisponível.".into(),
        });
    }
    let files = fetch_pull_request_files(token, owner, repo, number)?;
    let patch = files
        .iter()
        .find(|f| f.filename == path)
        .and_then(|f| f.patch.as_deref())
        .ok_or_else(|| GithubApiError::Http {
            status: 400,
            body: "Não há diff prévio para este arquivo — comente no GitHub.".into(),
        })?;
    let side_str = match side {
        CommentSide::Right => "RIGHT",
        CommentSide::Left => "LEFT",
    };
    let line = match (line, side) {
        (Some(n), CommentSide::Right) if n > 0 => {
            if !line_exists_on_right_side(patch, n) {
                return Err(GithubApiError::Http {
                    status: 400,
                    body: "Linha inválida neste arquivo — escolha outra no diff.".into(),
                });
            }
            n
        }
        (Some(n), CommentSide::Left) if n > 0 => {
            if !line_exists_on_left_side(patch, n) {
                return Err(GithubApiError::Http {
                    status: 400,
                    body: "Linha inválida no lado removido.".into(),
                });
            }
            n
        }
        (_, CommentSide::Right) => anchor_line_in_patch(patch).ok_or_else(|| GithubApiError::Http {
            status: 400,
            body: "Não foi possível achar uma linha no diff para o comentário.".into(),
        })?,
        (_, CommentSide::Left) => {
            return Err(GithubApiError::Http {
                status: 400,
                body: "Escolha uma linha removida no diff.".into(),
            });
        }
    };
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments");
    let json = serde_json::json!({
        "body": text,
        "commit_id": head_sha.trim(),
        "path": path.trim(),
        "line": line,
        "side": side_str,
    });
    let _ = request(token, reqwest::Method::POST, &url, Some(json))?;
    Ok(())
}

fn list_reviews(token: &str, owner: &str, repo: &str, number: u64) -> Result<Vec<PrReviewRow>, GithubApiError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let rows: Vec<ReviewApiRow> = serde_json::from_str(&raw)?;
    Ok(rows
        .into_iter()
        .map(|r| PrReviewRow {
            id: r.id,
            user_login: r.user.login,
            state: r.state,
            body: r.body.filter(|s| !s.trim().is_empty()),
            submitted_at: r.submitted_at,
        })
        .collect())
}

pub fn submit_pull_request_review(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    event: ReviewEvent,
    body: Option<&str>,
) -> Result<(), GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let event_str = match event {
        ReviewEvent::Approve => "APPROVE",
        ReviewEvent::RequestChanges => "REQUEST_CHANGES",
        ReviewEvent::Comment => "COMMENT",
    };
    let needs_body = matches!(event, ReviewEvent::RequestChanges | ReviewEvent::Comment);
    let text = body.unwrap_or("").trim();
    if needs_body && text.is_empty() {
        return Err(GithubApiError::Http {
            status: 400,
            body: "Escreva um comentário para esta revisão.".into(),
        });
    }
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews");
    let mut json = serde_json::json!({ "event": event_str });
    if !text.is_empty() {
        json["body"] = serde_json::Value::String(text.to_string());
    }
    let _ = request(token, reqwest::Method::POST, &url, Some(json))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoPullRequestRow {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub state: String,
    pub draft: bool,
    pub head_ref: String,
    pub base_ref: String,
    pub author_login: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFileChange {
    pub filename: String,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListPrApiRow {
    number: u64,
    title: String,
    html_url: String,
    state: String,
    draft: bool,
    head: PrApiRef,
    base: PrApiRef,
    user: PrApiUser,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct PrFileApiRow {
    filename: String,
    status: String,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    patch: Option<String>,
}

/// PRs abertos do repositório (até 30, mais recentes primeiro).
pub fn list_repo_pull_requests(
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<Vec<RepoPullRequestRow>, GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=30"
    );
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let rows: Vec<ListPrApiRow> = serde_json::from_str(&raw)?;
    Ok(rows
        .into_iter()
        .map(|pr| RepoPullRequestRow {
            number: pr.number,
            title: pr.title,
            html_url: pr.html_url,
            state: pr.state,
            draft: pr.draft,
            head_ref: pr.head.r#ref,
            base_ref: pr.base.r#ref,
            author_login: pr.user.login,
            updated_at: pr.updated_at,
        })
        .collect())
}

/// Arquivos alterados no PR (com patch unified quando o GitHub envia).
pub fn fetch_pull_request_files(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Vec<PrFileChange>, GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/files?per_page=100");
    let raw = request(token, reqwest::Method::GET, &url, None)?;
    let rows: Vec<PrFileApiRow> = serde_json::from_str(&raw)?;
    Ok(rows
        .into_iter()
        .map(|f| PrFileChange {
            filename: f.filename,
            status: f.status,
            additions: f.additions,
            deletions: f.deletions,
            patch: f.patch.filter(|p| !p.trim().is_empty()),
        })
        .collect())
}

pub fn merge_pull_request(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    method: MergeMethod,
) -> Result<MergePullRequestResult, GithubApiError> {
    validate_owner_repo(owner, repo)?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/merge");
    let method_str = match method {
        MergeMethod::Merge => "merge",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase",
    };
    let body = serde_json::json!({ "merge_method": method_str });
    let raw = request(token, reqwest::Method::PUT, &url, Some(body))?;
    let m: MergeApiResponse = serde_json::from_str(&raw)?;
    Ok(MergePullRequestResult {
        merged: m.merged,
        message: m.message,
        sha: m.sha,
    })
}

#[cfg(test)]
mod tests {
    use super::anchor_line_in_patch;

    #[test]
    fn anchor_line_first_addition() {
        let patch = "@@ -1,3 +1,4 @@\n line\n+added\n";
        assert_eq!(anchor_line_in_patch(patch), Some(2));
    }

    #[test]
    fn anchor_line_skips_deletions() {
        let patch = "@@ -1,2 +1,2 @@\n-removed\n kept\n";
        assert_eq!(anchor_line_in_patch(patch), None);
    }

    #[test]
    fn right_side_line_lookup() {
        use super::line_exists_on_right_side;
        let patch = "@@ -1,3 +1,4 @@\n ctx\n+add\n more\n";
        assert!(line_exists_on_right_side(patch, 1));
        assert!(line_exists_on_right_side(patch, 2));
        assert!(line_exists_on_right_side(patch, 3));
        assert!(!line_exists_on_right_side(patch, 4));
    }
}
