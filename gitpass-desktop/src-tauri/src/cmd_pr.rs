//! GitHub Pull Requests (criar, revisar, merge).

use gitpass_core::github_pr::{
    comment_on_pull_request_file, create_pull_request, disable_pull_request_auto_merge,
    enable_pull_request_auto_merge, fetch_pull_request_detail, fetch_pull_request_files, list_repo_pull_requests,
    merge_pull_request, request_pull_request_reviewers, submit_pull_request_review, CommentSide, CreatePullRequestInput,
    MergeMethod,
    MergePullRequestResult, PrFileChange, PullRequestCreated, PullRequestDetail, RepoPullRequestRow, ReviewEvent,
};
use gitpass_core::{AccountHost, AppConfig};
use uuid::Uuid;

use crate::cmd_cloud::{api_error_message, current_token};
use crate::cmd_repos::blocking;

fn parse_id(raw: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw.trim()).map_err(|_| "Conta inválida.".to_string())
}

fn account_github_login(cfg: &AppConfig, id: Uuid) -> Result<String, String> {
    let a = cfg
        .accounts
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "Conta desconhecida.".to_string())?;
    if a.host != AccountHost::Github {
        return Err("Pull requests remotos nesta versão são só para contas GitHub.".into());
    }
    a.github_login
        .clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "Conta sem login GitHub.".to_string())
}

#[tauri::command]
pub async fn github_pr_list_repo(
    account_id: String,
    owner: String,
    repo: String,
) -> Result<Vec<RepoPullRequestRow>, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        list_repo_pull_requests(&token, owner.trim(), repo.trim()).map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_files(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
) -> Result<Vec<PrFileChange>, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        fetch_pull_request_files(&token, owner.trim(), repo.trim(), number).map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_detail(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
) -> Result<PullRequestDetail, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        fetch_pull_request_detail(&token, owner.trim(), repo.trim(), number).map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_create(
    account_id: String,
    owner: String,
    repo: String,
    title: String,
    head: String,
    base: String,
    body: Option<String>,
    draft: Option<bool>,
) -> Result<PullRequestCreated, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        let input = CreatePullRequestInput {
            title,
            head,
            base,
            body,
            draft: draft.unwrap_or(false),
        };
        create_pull_request(&token, owner.trim(), repo.trim(), input).map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_submit_review(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
    event: String,
    body: Option<String>,
) -> Result<(), String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        let ev = match event.trim() {
            "approve" => ReviewEvent::Approve,
            "request_changes" => ReviewEvent::RequestChanges,
            "comment" => ReviewEvent::Comment,
            _ => return Err("Tipo de revisão inválido.".into()),
        };
        submit_pull_request_review(&token, owner.trim(), repo.trim(), number, ev, body.as_deref())
            .map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_comment_file(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
    head_sha: String,
    path: String,
    body: String,
    line: Option<u64>,
    side: Option<String>,
) -> Result<(), String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        let line_u32 = line.and_then(|n| u32::try_from(n).ok());
        let side = match side.as_deref().unwrap_or("RIGHT").trim().to_uppercase().as_str() {
            "LEFT" => CommentSide::Left,
            _ => CommentSide::Right,
        };
        comment_on_pull_request_file(
            &token,
            owner.trim(),
            repo.trim(),
            number,
            head_sha.trim(),
            path.trim(),
            body.trim(),
            line_u32,
            side,
        )
        .map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_enable_auto_merge(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
    method: Option<String>,
) -> Result<(), String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        let m = match method.as_deref().unwrap_or("merge").trim() {
            "squash" => MergeMethod::Squash,
            "rebase" => MergeMethod::Rebase,
            _ => MergeMethod::Merge,
        };
        enable_pull_request_auto_merge(&token, owner.trim(), repo.trim(), number, m).map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_disable_auto_merge(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
) -> Result<(), String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        disable_pull_request_auto_merge(&token, owner.trim(), repo.trim(), number).map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_request_reviewers(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
    reviewers: Vec<String>,
) -> Result<(), String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let token = current_token(id)?;
        request_pull_request_reviewers(&token, owner.trim(), repo.trim(), number, &reviewers)
            .map_err(api_error_message)
    })
    .await
}

#[tauri::command]
pub async fn github_pr_merge(
    account_id: String,
    owner: String,
    repo: String,
    number: u64,
    method: Option<String>,
) -> Result<MergePullRequestResult, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let _login = {
            let cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
            account_github_login(&cfg, id)?
        };
        let token = current_token(id)?;
        let m = match method.as_deref().unwrap_or("merge").trim() {
            "squash" => MergeMethod::Squash,
            "rebase" => MergeMethod::Rebase,
            _ => MergeMethod::Merge,
        };
        merge_pull_request(&token, owner.trim(), repo.trim(), number, m).map_err(api_error_message)
    })
    .await
}
