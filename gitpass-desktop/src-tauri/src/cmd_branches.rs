//! Tauri commands for this area. Registered in main.rs `generate_handler!`.
//!
//! Commands: `repos_branches`, `repos_checkout`, `repos_merge_preview`, `repos_merge_apply`.
//! The Git work lives in `gitpass_core::branches`; binding lookup, the account's access, events and the
//! one-operation-per-repository lock are shared with `cmd_push`.

use gitpass_core::branches::{self, BranchInfo, MergeContext, MergeMode, MergePreview, MergeRequest, MergeResult};
use gitpass_core::outgoing::{self, IdentityRules};
use tauri::AppHandle;

use crate::cmd_push::{
    blocking, bound, commit_identity, identity_rules, notify_changed, parse_binding, Access, Bound, BusyGuard,
};
use crate::cmd_repos::{load_config, notify, row_for, RepoRow};

/// Emails that count as the bound account (GitHub's list only when already cached: no network here).
fn identity_of(b: &Bound) -> IdentityRules {
    let mut access = Access::load(b.account.as_ref());
    identity_rules(b.account.as_ref(), access.person(false))
}

/// Local branches and the ones only on GitHub. With `fetch`, updates from GitHub first (bound account).
#[tauri::command]
pub async fn repos_branches(binding_id: String, fetch: bool) -> Result<Vec<BranchInfo>, String> {
    blocking(move || {
        let b = bound(&binding_id)?;
        if let (true, Some(remote)) = (fetch, b.remote.as_deref()) {
            let mut access = Access::load(b.account.as_ref());
            // Offline, the local list still comes back; `repos_outgoing` reports the failed fetch.
            if access.git(|t| outgoing::fetch(&b.repo, remote, t)).is_ok() && !branches::has_remote_head(&b.repo, remote) {
                let _ = access.git(|t| branches::set_head_auto(&b.repo, remote, t));
            }
        }
        branches::list_branches(&b.repo, b.remote.as_deref())
    })
    .await
}

/// Switches the folder to `branch` (created from GitHub when it only exists there). Refuses pending changes.
#[tauri::command]
pub async fn repos_checkout(app: AppHandle, binding_id: String, branch: String) -> Result<RepoRow, String> {
    blocking(move || {
        let id = parse_binding(&binding_id)?;
        let _guard = BusyGuard::acquire(id)?;
        let b = bound(&binding_id)?;
        let done = branches::checkout(&b.repo, b.remote.as_deref(), &branch);
        notify(&app);
        notify_changed(&app, &b.id.to_string());
        done?;
        row_for(&load_config()?, id)
    })
    .await
}

/// What bringing `source` into `target` would do (merge or cherry-pick of `commits`); changes nothing.
#[tauri::command]
pub async fn repos_merge_preview(
    binding_id: String,
    source: String,
    target: String,
    mode: MergeMode,
    commits: Vec<String>,
) -> Result<MergePreview, String> {
    blocking(move || {
        let b = bound(&binding_id)?;
        let identity = identity_of(&b);
        let bid = b.id.to_string();
        let ctx = MergeContext { binding_id: &bid, remote: b.remote.as_deref(), identity: &identity };
        let req = MergeRequest { source: &source, target: &target, mode, commits: &commits };
        branches::merge_preview(&b.repo, &ctx, &req)
    })
    .await
}

/// Applies the preview if `source`/`target` are still at `source_sha`/`target_sha`; all or nothing.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn repos_merge_apply(
    app: AppHandle,
    binding_id: String,
    source: String,
    target: String,
    mode: MergeMode,
    commits: Vec<String>,
    source_sha: String,
    target_sha: String,
) -> Result<MergeResult, String> {
    blocking(move || {
        let id = parse_binding(&binding_id)?;
        let _guard = BusyGuard::acquire(id)?;
        let b = bound(&binding_id)?;
        let identity = identity_of(&b);
        let who = commit_identity(b.account.as_ref());
        let bid = b.id.to_string();
        let ctx = MergeContext { binding_id: &bid, remote: b.remote.as_deref(), identity: &identity };
        let req = MergeRequest { source: &source, target: &target, mode, commits: &commits };
        let res = branches::merge_apply(&b.repo, &ctx, &req, &source_sha, &target_sha, who.as_ref().ok());
        notify_changed(&app, &bid);
        match (res, who) {
            (Err(e), Err(why)) if e == branches::NO_IDENTITY => Err(why),
            (res, _) => res,
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_mode_accepts_frontend_values() {
        let modes: Vec<MergeMode> = serde_json::from_str(r#"["merge","cherry_pick"]"#).unwrap();
        assert_eq!(modes, MergeMode::ALL);
        assert!(serde_json::from_str::<MergeMode>(r#""rebase""#).is_err());
    }

    #[test]
    fn unknown_bindings_are_refused() {
        assert!(bound("not-a-uuid").err().unwrap().contains("inválido"));
    }
}
