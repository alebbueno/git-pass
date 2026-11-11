//! Pull requests Bitbucket Cloud.

use gitpass_core::bitbucket_pr::{
    approve_bitbucket_pull_request, create_bitbucket_pull_request, fetch_bitbucket_pull_request_detail,
    fetch_bitbucket_pull_request_diff, list_bitbucket_pull_requests, merge_bitbucket_pull_request,
};
use gitpass_core::github_pr::{CreatePullRequestInput, MergePullRequestResult, PullRequestCreated, PullRequestDetail, RepoPullRequestRow};
use gitpass_core::{
    bitbucket_token_load, AccountHost, AppConfig,
};
use uuid::Uuid;

use crate::cmd_bitbucket::{bb_err, bb_err_pr_write};
use crate::cmd_repos::blocking;

fn parse_id(raw: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw.trim()).map_err(|_| "Conta inválida.".to_string())
}

fn bb_auth(id: Uuid) -> Result<(String, String, bool), String> {
    let cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
    let a = cfg
        .accounts
        .iter()
        .find(|x| x.id == id)
        .ok_or_else(|| "Conta desconhecida.".to_string())?;
    if a.host != AccountHost::Bitbucket {
        return Err("Esta conta não é do Bitbucket.".into());
    }
    let login = a
        .github_login
        .clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "Conta sem usuário Bitbucket.".to_string())?;
    let bundle = bitbucket_token_load(id)
        .map_err(|e| format!("Keychain: {e}"))?
        .ok_or_else(|| "Reconecte a conta Bitbucket.".to_string())?;
    Ok((login, bundle.app_password, bundle.via_oauth))
}

#[tauri::command]
pub async fn bitbucket_pr_list_repo(
    account_id: String,
    workspace: String,
    repo_slug: String,
) -> Result<Vec<RepoPullRequestRow>, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let (login, token, oauth) = bb_auth(id)?;
        list_bitbucket_pull_requests(&login, &token, oauth, workspace.trim(), repo_slug.trim()).map_err(bb_err)
    })
    .await
}

#[tauri::command]
pub async fn bitbucket_pr_detail(
    account_id: String,
    workspace: String,
    repo_slug: String,
    number: u64,
) -> Result<PullRequestDetail, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let (login, token, oauth) = bb_auth(id)?;
        let detail = fetch_bitbucket_pull_request_detail(&login, &token, oauth, workspace.trim(), repo_slug.trim(), number)
            .map_err(bb_err)?;
        Ok(detail.into())
    })
    .await
}

#[tauri::command]
pub async fn bitbucket_pr_create(
    account_id: String,
    workspace: String,
    repo_slug: String,
    title: String,
    head: String,
    base: String,
    body: Option<String>,
    draft: Option<bool>,
) -> Result<PullRequestCreated, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let (login, token, oauth) = bb_auth(id)?;
        let input = CreatePullRequestInput {
            title,
            head,
            base,
            body,
            draft: draft.unwrap_or(false),
        };
        create_bitbucket_pull_request(&login, &token, oauth, workspace.trim(), repo_slug.trim(), input)
            .map_err(bb_err_pr_write)
    })
    .await
}

#[tauri::command]
pub async fn bitbucket_pr_approve(
    account_id: String,
    workspace: String,
    repo_slug: String,
    number: u64,
) -> Result<(), String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let (login, token, oauth) = bb_auth(id)?;
        approve_bitbucket_pull_request(&login, &token, oauth, workspace.trim(), repo_slug.trim(), number)
            .map_err(bb_err_pr_write)
    })
    .await
}

#[tauri::command]
pub async fn bitbucket_pr_diff(
    account_id: String,
    workspace: String,
    repo_slug: String,
    number: u64,
) -> Result<String, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let (login, token, oauth) = bb_auth(id)?;
        fetch_bitbucket_pull_request_diff(&login, &token, oauth, workspace.trim(), repo_slug.trim(), number)
            .map_err(bb_err)
    })
    .await
}

#[tauri::command]
pub async fn bitbucket_pr_merge(
    account_id: String,
    workspace: String,
    repo_slug: String,
    number: u64,
) -> Result<MergePullRequestResult, String> {
    blocking(move || {
        let id = parse_id(&account_id)?;
        let (login, token, oauth) = bb_auth(id)?;
        merge_bitbucket_pull_request(&login, &token, oauth, workspace.trim(), repo_slug.trim(), number)
            .map_err(bb_err_pr_write)
    })
    .await
}
