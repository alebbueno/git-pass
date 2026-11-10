//! Structured commit log + authorship hints for a bound repo.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use uuid::Uuid;

use crate::bindings::expected_identity;
use crate::outgoing::{GitRepo, OutgoingCommit};
use crate::{Account, AppConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitOwnership {
    SelfAuthored,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitReach {
    OnUpstream,
    BeyondUpstream,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitReviewRow {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub ownership: CommitOwnership,
    pub identity_mismatch: bool,
    pub reach: CommitReach,
}

pub fn accepted_emails_for_account(acc: &Account) -> HashSet<String> {
    let mut s = HashSet::new();
    if let Some(e) = &acc.email {
        s.insert(e.to_ascii_lowercase());
    }
    if let Some(login) = &acc.github_login {
        s.insert(format!("{login}@users.noreply.github.com").to_ascii_lowercase());
    }
    s
}

pub fn binding_commit_review(
    cfg: &AppConfig,
    binding_id: Uuid,
    limit: usize,
) -> Result<Vec<CommitReviewRow>, String> {
    let binding = cfg
        .repos
        .iter()
        .find(|b| b.id == binding_id)
        .ok_or_else(|| "Vínculo desconhecido.".to_string())?;
    let acc = cfg
        .accounts
        .iter()
        .find(|a| a.id == binding.account_id)
        .ok_or_else(|| "Conta do vínculo não existe.".to_string())?;
    let root = &binding.root_path;
    if !root.is_dir() {
        return Err("A pasta do repositório não existe.".into());
    }
    let repo = GitRepo::new(root);
    let limit = limit.clamp(1, 200);
    let commits: Vec<OutgoingCommit> = repo.commits(&[], limit)?;
    let accepted = accepted_emails_for_account(acc);
    let expected = expected_identity(acc);
    let beyond = beyond_upstream_shas(root);
    Ok(commits
        .into_iter()
        .map(|c| {
            let email_l = c.author_email.to_ascii_lowercase();
            let ownership = if accepted.contains(&email_l) {
                CommitOwnership::SelfAuthored
            } else {
                CommitOwnership::Other
            };
            let identity_mismatch = ownership == CommitOwnership::SelfAuthored
                && expected
                    .email
                    .as_ref()
                    .is_some_and(|e| !e.eq_ignore_ascii_case(&c.author_email));
            let reach = match &beyond {
                Some(set) if set.contains(&c.sha) => CommitReach::BeyondUpstream,
                Some(_) => CommitReach::OnUpstream,
                None => CommitReach::Unknown,
            };
            CommitReviewRow {
                sha: c.sha,
                short_sha: c.short_sha,
                subject: c.subject,
                author_name: c.author_name,
                author_email: c.author_email,
                authored_at: c.authored_at,
                ownership,
                identity_mismatch,
                reach,
            }
        })
        .collect())
}

fn beyond_upstream_shas(root: &Path) -> Option<HashSet<String>> {
    let repo = GitRepo::new(root);
    if repo
        .value(&["rev-parse", "-q", "--verify", "@{upstream}^{commit}"])
        .is_none()
    {
        return None;
    }
    let list = repo.value(&["rev-list", "@{upstream}..HEAD"])?;
    Some(list.lines().map(String::from).collect())
}
