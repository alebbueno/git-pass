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

/// What the user was doing, for messages ("o push", "o commit", ...).
fn operation_label(phase: &str) -> &'static str {
    match phase {
        "pre-push" => "o push",
        "pre-rebase" => "o rebase",
        "pre-merge-commit" => "o merge",
        _ => "o commit",
    }
}

const REPAIR_HINT: &str = "Abre a app GitPass e escolhe «Reparar» em Definições.";

/// Enforces **daemon must validate** policy when IPC is unavailable or the socket is missing.
/// Development escape hatches (`GITPASS_ALLOW_*`) are deliberately not mentioned to users.
pub fn policy_when_daemon_unreachable(phase: &str, cfg: &AppConfig) -> ValidateFail {
    if !is_validated_phase(phase) {
        return ValidateFail::Ok;
    }

    let blocked = || ValidateFail::Err {
        code: "GP-DAEMON-OFF",
        message: format!(
            "O serviço do GitPass não está a responder, por isso {} foi bloqueado. {REPAIR_HINT}",
            operation_label(phase)
        ),
    };

    if is_push_phase(phase) {
        let allow = std::env::var("GITPASS_ALLOW_PUSH_WITHOUT_DAEMON").unwrap_or_default();
        if allow == "1" || allow.eq_ignore_ascii_case("true") {
            return ValidateFail::Ok;
        }
        return blocked();
    }

    let dev = std::env::var("GITPASS_ALLOW_LOCAL_COMMIT").unwrap_or_default();
    if dev == "1" || dev.eq_ignore_ascii_case("true") {
        return ValidateFail::Ok;
    }
    if emergency_allows_local_commit(cfg) {
        return ValidateFail::Ok;
    }

    blocked()
}

/// `origin` as shown to the user: credentials embedded in the URL (`https://user:token@…`) removed.
fn redact_url(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        if let Some((_, host)) = authority.rsplit_once('@') {
            return format!("{scheme}://{host}{path}");
        }
    }
    url.to_string()
}

fn api_error_detail(e: &crate::GithubApiError) -> String {
    match e {
        crate::GithubApiError::Http { status, .. } => format!("o GitHub respondeu HTTP {status}"),
        crate::GithubApiError::Reqwest(_) => "sem ligação ao GitHub".into(),
        _ => "resposta inesperada do GitHub".into(),
    }
}

// --- core checks ---

fn expect_binding<'a>(root: &Path, cfg: &'a AppConfig) -> Result<&'a RepoBinding, ValidateFail> {
    let root = root
        .canonicalize()
        .map_err(|e| ValidateFail::Err {
            code: "GP-REPO",
            message: format!("Não foi possível abrir a pasta do repositório ({e})."),
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
        .ok_or_else(|| ValidateFail::Err {
            code: "GP-BINDING",
            message: format!(
                "Este repositório ({}) não está associado a nenhuma conta. Associa-o na app GitPass (Repositórios).",
                root.display()
            ),
        })
}

fn account_by_id(cfg: &AppConfig, id: Uuid) -> Option<&Account> {
    cfg.accounts.iter().find(|a| a.id == id)
}

/// Strict consistency: binding metadata vs `origin` URL (not about whether the user may push).
fn binding_remote_consistency(binding: &RepoBinding, remote: &GitHubRepoId) -> Result<(), String> {
    let owner_ok = binding
        .github_owner
        .as_deref()
        .map_or(true, |bo| normalize_owner(bo) == remote.owner);
    let repo_ok = binding
        .github_repo
        .as_deref()
        .map_or(true, |br| br.trim_end_matches(".git").eq_ignore_ascii_case(remote.repo.as_str()));
    if owner_ok && repo_ok {
        return Ok(());
    }
    let registered = format!(
        "{}/{}",
        binding.github_owner.as_deref().unwrap_or(remote.owner.as_str()),
        binding.github_repo.as_deref().unwrap_or(remote.repo.as_str())
    );
    Err(format!(
        "O remoto «origin» aponta para {}/{}, mas no GitPass este repositório está registado como {registered}. Associa-o de novo na app GitPass (Repositórios).",
        remote.owner, remote.repo
    ))
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
                message: format!("Não foi possível identificar o repositório Git ({e})."),
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
                message: "Este repositório está associado a uma conta que já não existe no GitPass. Escolhe outra conta na app GitPass (Repositórios).".into(),
            };
        }
    };

    let login = match &account.github_login {
        Some(l) if !l.trim().is_empty() => l.trim(),
        None | Some(_) => {
            return ValidateFail::Err {
                code: "GP-ACCOUNT-LOGIN",
                message: format!(
                    "A conta «{}» não tem utilizador do GitHub. Liga-a de novo na app GitPass (Contas).",
                    account.label.as_deref().unwrap_or("sem nome")
                ),
            };
        }
    };

    let origin_url = match git_remote_url(&root, "origin") {
        Ok(u) => u,
        Err(_) => {
            return ValidateFail::Err {
                code: "GP-ORIGIN",
                message: "Este repositório não tem o remoto «origin». Adiciona-o (o endereço do repositório no GitHub) ou associa o repositório de novo na app GitPass.".into(),
            };
        }
    };

    let remote = match parse_github_remote(&origin_url) {
        Some(r) => r,
        None => {
            return ValidateFail::Err {
                code: "GP-REMOTE",
                message: format!(
                    "O remoto «origin» ({}) não é um repositório do github.com. O GitPass só protege repositórios do GitHub; remove a associação na app GitPass (Repositórios) se este repositório não for do GitHub.",
                    redact_url(&origin_url)
                ),
            };
        }
    };

    if let Err(msg) = binding_remote_consistency(binding, &remote) {
        return ValidateFail::Err {
            code: "GP-OWNER",
            message: msg,
        };
    }

    if is_direct_owner(login, &remote) {
        return ValidateFail::Ok;
    }

    if !needs_api_disambiguation(login, &remote, binding) {
        return ValidateFail::Err {
            code: "GP-OWNER",
            message: owner_mismatch_message(&remote.owner, login),
        };
    }

    let no_token = || ValidateFail::Err {
        code: "GP-OAUTH",
        message: format!(
            "Este repositório pertence a {}, e é preciso confirmar no GitHub se a conta @{login} tem acesso, mas a conta não está ligada. Liga-a de novo na app GitPass (Contas).",
            remote.owner
        ),
    };

    // --- Ambiguous: org/collaborator / fork-style ---
    let api_result = if opts.use_keychain_token {
        if github_token_load(binding.account_id).ok().flatten().is_none() {
            return no_token();
        }
        token_can_push_for_account(binding.account_id, &remote.owner, &remote.repo)
    } else {
        let Some(tok) = opts.github_token.filter(|t| !t.trim().is_empty()) else {
            return no_token();
        };
        token_can_push(tok, &remote.owner, &remote.repo)
    };

    match api_result {
        Ok(true) => ValidateFail::Ok,
        Ok(false) => ValidateFail::Err {
            code: "GP-PERMISSION",
            message: format!(
                "A conta @{login} não tem permissão de escrita em {}/{}. Muda a conta na app GitPass (Repositórios) ou pede acesso a {}.",
                remote.owner, remote.repo, remote.owner
            ),
        },
        Err(e) => {
            let detail = if matches!(e, crate::GithubApiError::Http { status: 401, .. }) {
                format!("a conta @{login} precisa de ser ligada de novo na app GitPass (Contas)")
            } else {
                api_error_detail(&e)
            };
            if opts.allow_ambiguous_when_api_unreachable {
                ValidateFail::OkUnconfirmed {
                    warning: format!(
                        "GitPass: não foi possível confirmar no GitHub o acesso da conta @{login} a {}/{} ({detail}). {} foi permitido.",
                        remote.owner,
                        remote.repo,
                        capitalize(operation_label(phase))
                    ),
                }
            } else {
                ValidateFail::Err {
                    code: "GP-API",
                    message: format!(
                        "Não foi possível confirmar no GitHub o acesso da conta @{login} a {}/{} ({detail}). Verifica a ligação à Internet e tenta de novo.",
                        remote.owner, remote.repo
                    ),
                }
            }
        }
    }
}

fn owner_mismatch_message(owner: &str, login: &str) -> String {
    format!(
        "Este repositório pertence a {owner}, mas está associado à conta @{login}. Muda a conta na app GitPass (Repositórios)."
    )
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
