//! Validation of a hook against **`config.toml`** and **`git remote`**.
//! Optional GitHub API (token) for **fork/collaborator** ambiguity when `origin` owner ≠ account login.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::github_api::{token_can_push, token_can_push_for_account};
use crate::secrets::github_token_load;
use crate::git::{git_remote_url, git_work_tree_root};
use crate::remote::{normalize_owner, parse_github_remote, GitHubRepoId};
use crate::{Account, RepoBinding};

#[derive(Debug, Clone)]
pub struct HookContext {
    pub phase: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ValidateFail {
    Ok,
    /// Allowed but GitHub API could not confirm collaborator access (warning text for stderr).
    OkUnconfirmed { warning: String },
    Err { code: &'static str, message: String },
}

/// When validating inside **`gitpassd`**, an OAuth token may disambiguate org/upstream repos.
pub struct ValidateOptions<'a> {
    /// When **not** using daemon token path, pass the access token string (e.g. offline hook).
    pub github_token: Option<&'a str>,
    /// When true (`gitpassd`): load token from Keychain for **binding.account_id** and support **401 → refresh**.
    pub use_keychain_token: bool,
    /// If true (daemon): when API fails on an ambiguous case, allow with **OkUnconfirmed** (design DL-014).
    pub allow_ambiguous_when_api_unreachable: bool,
}

impl Default for ValidateOptions<'_> {
    fn default() -> Self {
        Self {
            github_token: None,
            use_keychain_token: false,
            allow_ambiguous_when_api_unreachable: false,
        }
    }
}

// --- env: CI bypass (handled in `gitpass hook` before IPC) ---

pub fn ci_bypass_from_env() -> bool {
    let ci = std::env::var("CI").unwrap_or_default();
    let ga = std::env::var("GITHUB_ACTIONS").unwrap_or_default();
    let looks_ci = ci == "true" || ci == "1" || ga == "true";
    let allow = std::env::var("GITPASS_ALLOW_CI").unwrap_or_default();
    looks_ci && (allow == "1" || allow.eq_ignore_ascii_case("true"))
}

// --- policy when the daemon does not answer (used by `gitpass hook` after IPC failure) ---

fn is_push_phase(phase: &str) -> bool {
    phase == "pre-push"
}

/// Hook phases where we enforce rules (excludes `post-commit`, too late to block).
pub fn is_validated_phase(phase: &str) -> bool {
    matches!(
        phase,
        "pre-commit"
            | "pre-merge-commit"
            | "commit-msg"
            | "prepare-commit-msg"
            | "pre-rebase"
            | "pre-push"
    )
}

fn emergency_allows_local_commit(cfg: &AppConfig) -> bool {
    let raw = match &cfg.ui.emergency.allow_local_commit_until {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        let now = Utc::now();
        return now < dt.with_timezone(&Utc);
    }
    false
}

/// Enforces **daemon must validate** policy when IPC is unavailable or the socket is missing.
pub fn policy_when_daemon_unreachable(phase: &str, cfg: &AppConfig) -> ValidateFail {
    if !is_validated_phase(phase) {
        return ValidateFail::Ok;
    }

    if is_push_phase(phase) {
        let allow = std::env::var("GITPASS_ALLOW_PUSH_WITHOUT_DAEMON").unwrap_or_default();
        if allow == "1" || allow.eq_ignore_ascii_case("true") {
            return ValidateFail::Ok;
        }
        return ValidateFail::Err {
            code: "GP-DAEMON-OFF",
            message: format!(
                "cannot reach gitpassd for hook `{phase}`.\n\
                 Push blocked by policy. Start gitpassd or set GITPASS_ALLOW_PUSH_WITHOUT_DAEMON=1 for development only."
            ),
        };
    }

    let dev = std::env::var("GITPASS_ALLOW_LOCAL_COMMIT").unwrap_or_default();
    if dev == "1" || dev.eq_ignore_ascii_case("true") {
        return ValidateFail::Ok;
    }
    if emergency_allows_local_commit(cfg) {
        return ValidateFail::Ok;
    }

    ValidateFail::Err {
        code: "GP-DAEMON-OFF",
        message: format!(
            "cannot reach gitpassd for hook `{phase}`.\n\
             Commit blocked by policy. Start gitpassd, use emergency mode in config, or set GITPASS_ALLOW_LOCAL_COMMIT=1 (development only)."
        ),
    }
}

// --- core checks ---

fn expect_binding<'a>(root: &Path, cfg: &'a AppConfig) -> Result<&'a RepoBinding, ValidateFail> {
    let root = root
        .canonicalize()
        .map_err(|e| ValidateFail::Err {
            code: "GP-REPO",
            message: format!("cannot canonicalize repo root: {e}"),
        })?;
    cfg.repos
        .iter()
        .find(|r| {
            let bound = r
                .root_path
                .canonicalize()
                .unwrap_or_else(|_| r.root_path.clone());
            bound == root
        })
        .ok_or_else(|| {
            let msg = format!(
                "no GitPass binding for this repo (root {}).\n\
                 Add a [[repos]] entry in {} or run the app to bind an account.",
                root.display(),
                crate::config_file_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "config.toml".into())
            );
            ValidateFail::Err {
                code: "GP-BINDING",
                message: msg,
            }
        })
}

fn account_by_id(cfg: &AppConfig, id: Uuid) -> Option<&Account> {
    cfg.accounts.iter().find(|a| a.id == id)
}

/// Strict consistency: binding metadata vs `origin` URL (not about whether the user may push).
fn binding_remote_consistency(binding: &RepoBinding, remote: &GitHubRepoId) -> Result<(), String> {
    if let Some(ref bo) = binding.github_owner {
        let bo_n = normalize_owner(bo);
        if bo_n != remote.owner {
            return Err(format!(
                "binding says owner `{bo}` but `origin` is `{}/{}`",
                remote.owner, remote.repo
            ));
        }
    }
    if let Some(ref br) = binding.github_repo {
        let br_norm = br.trim_end_matches(".git");
        if !br_norm.eq_ignore_ascii_case(remote.repo.as_str()) {
            return Err(format!(
                "binding says repo `{br}` but `origin` points to `{}`",
                remote.repo
            ));
        }
    }
    Ok(())
}

/// Direct owner match: login equals `origin` owner.
fn is_direct_owner(login: &str, remote: &GitHubRepoId) -> bool {
    normalize_owner(login) == remote.owner
}

/// Cases where local `login == origin.owner` rule does not hold, but the user might still be allowed (collaborator, etc.).
fn needs_api_disambiguation(login: &str, remote: &GitHubRepoId, binding: &RepoBinding) -> bool {
    if is_direct_owner(login, remote) {
        return false;
    }
    if binding_remote_consistency(binding, remote).is_err() {
        return false;
    }
    // If binding lists an owner matching `origin`, but account login differs — typical org/upstream clone.
    if let Some(ref bo) = binding.github_owner {
        if normalize_owner(bo) == remote.owner {
            return true;
        }
    }
    // No stored owner — still could be collaborator on origin.
    binding.github_owner.is_none()
}

/// Validates **repository, binding, account, and `origin`**. Hook / offline path: **no token**, no API.
pub fn validate_repo_for_hook(ctx: &HookContext, cfg: &AppConfig) -> ValidateFail {
    validate_repo_for_hook_with_options(ctx, cfg, ValidateOptions::default())
}

/// Like [`validate_repo_for_hook_with_options`] but loads the OAuth token from **Keychain** for the bound account (`gitpassd` only).
pub fn validate_repo_for_hook_daemon(
    ctx: &HookContext,
    cfg: &AppConfig,
    allow_ambiguous_when_api_unreachable: bool,
) -> ValidateFail {
    validate_repo_for_hook_with_options(
        ctx,
        cfg,
        ValidateOptions {
            github_token: None,
            use_keychain_token: true,
            allow_ambiguous_when_api_unreachable,
        },
    )
}

pub fn validate_repo_for_hook_with_options(
    ctx: &HookContext,
    cfg: &AppConfig,
    opts: ValidateOptions<'_>,
) -> ValidateFail {
    let phase = ctx.phase.as_str();

    let root = match git_work_tree_root(Some(&ctx.cwd)) {
        Ok(p) => p,
        Err(e) => {
            return ValidateFail::Err {
                code: "GP-GIT",
                message: e,
            };
        }
    };

    if !is_validated_phase(phase) {
        return ValidateFail::Ok;
    }

    let binding = match expect_binding(&root, cfg) {
        Ok(b) => b,
        Err(e) => return e,
    };

    let account = match account_by_id(cfg, binding.account_id) {
        Some(a) => a,
        None => {
            return ValidateFail::Err {
                code: "GP-ACCOUNT",
                message: format!(
                    "binding references missing account_id {} in config",
                    binding.account_id
                ),
            };
        }
    };

    let login = match &account.github_login {
        Some(l) if !l.trim().is_empty() => l.trim(),
        None | Some(_) => {
            return ValidateFail::Err {
                code: "GP-ACCOUNT-LOGIN",
                message: format!(
                    "account {:?} has no github_login — set it in {}",
                    account.label.as_deref().unwrap_or("(no label)"),
                    crate::config_file_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| "config.toml".into())
                ),
            };
        }
    };

    let origin_url = match git_remote_url(&root, "origin") {
        Ok(u) => u,
        Err(e) => {
            return ValidateFail::Err {
                code: "GP-ORIGIN",
                message: e,
            };
        }
    };

    let remote = match parse_github_remote(&origin_url) {
        Some(r) => r,
        None => {
            return ValidateFail::Err {
                code: "GP-REMOTE",
                message: format!(
                    "`origin` is not a github.com URL we understand: {origin_url}\n\
                     GitPass v1 supports GitHub only."
                ),
            };
        }
    };

    if let Err(msg) = binding_remote_consistency(binding, &remote) {
        return ValidateFail::Err {
            code: "GP-OWNER",
            message: format!("{msg}\n  remote: {origin_url}\n  account: {login}"),
        };
    }

    if is_direct_owner(login, &remote) {
        return ValidateFail::Ok;
    }

    if !needs_api_disambiguation(login, &remote, binding) {
        return ValidateFail::Err {
            code: "GP-OWNER",
            message: format!(
                "account `{login}` does not match `origin` owner `{}`.\n\
                 remote: {origin_url}",
                remote.owner
            ),
        };
    }

    // --- Ambiguous: org/collaborator / fork-style ---
    let api_result = if opts.use_keychain_token {
        if github_token_load(binding.account_id).ok().flatten().is_none() {
            return ValidateFail::Err {
                code: "GP-OAUTH",
                message: format!(
                    "account `{login}` is not the `origin` owner (`{}`), and no OAuth token is stored.\n\
                     Run: gitpass account login --account {login}\n\
                     (GitHub API is required to confirm collaborator access on `{}/{}`.)",
                    remote.owner, remote.owner, remote.repo
                ),
            };
        }
        token_can_push_for_account(binding.account_id, &remote.owner, &remote.repo)
    } else {
        let Some(tok) = opts.github_token.filter(|t| !t.trim().is_empty()) else {
            return ValidateFail::Err {
                code: "GP-OAUTH",
                message: format!(
                    "account `{login}` is not the `origin` owner (`{}`), and no OAuth token is stored.\n\
                     Run: gitpass account login --account {login}\n\
                     (GitHub API is required to confirm collaborator access on `{}/{}`.)",
                    remote.owner, remote.owner, remote.repo
                ),
            };
        };
        token_can_push(tok, &remote.owner, &remote.repo)
    };

    match api_result {
        Ok(true) => ValidateFail::Ok,
        Ok(false) => ValidateFail::Err {
            code: "GP-PERMISSION",
            message: format!(
                "GitHub API: token for `{login}` does not have push access to `{}/{}`.\n\
                 remote: {origin_url}",
                remote.owner, remote.repo
            ),
        },
        Err(e) => {
            let detail = format!("{e}");
            if opts.allow_ambiguous_when_api_unreachable {
                ValidateFail::OkUnconfirmed {
                    warning: format!(
                        "GitPass: could not confirm collaborator access via GitHub API ({detail}).\n\
                         Allowing (ambiguous case) — set `GITPASS_STRICT_API=1` in the daemon environment to block when API fails."
                    ),
                }
            } else {
                ValidateFail::Err {
                    code: "GP-API",
                    message: format!(
                        "GitHub API error while checking `{}/{}`: {detail}",
                        remote.owner, remote.repo
                    ),
                }
            }
        }
    }
}
