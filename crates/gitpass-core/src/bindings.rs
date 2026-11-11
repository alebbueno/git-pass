//! Creating repo bindings and applying the account's Git identity to a clone.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use uuid::Uuid;

use crate::remote::{BitbucketRepoId, GitHubRepoId};
use crate::{git_remote_url, normalize_owner, parse_bitbucket_remote, parse_github_remote, Account, AppConfig, RepoBinding};

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepoIdentity {
    pub name: Option<String>,
    pub email: Option<String>,
}

fn git_config_get(root: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--local", "--get", "--", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

fn git_config_set(root: &Path, key: &str, value: &str) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--local", "--", key, value])
        .output()
        .map_err(|e| format!("Não foi possível rodar o Git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Local (`--local`) `user.name` / `user.email` of the repo; global values are ignored on purpose.
pub fn read_repo_identity(root: &Path) -> RepoIdentity {
    RepoIdentity {
        name: git_config_get(root, "user.name"),
        email: git_config_get(root, "user.email"),
    }
}

/// Writes the account's name/email into the repo's local Git config. Fields the account lacks are left untouched.
/// Writes explicit local identity values (used by undo).
pub fn set_repo_identity(root: &Path, id: &RepoIdentity) -> Result<RepoIdentity, String> {
    match &id.name {
        Some(n) => git_config_set(root, "user.name", n)?,
        None => {
            git_config_unset(root, "user.name")?;
        }
    }
    match &id.email {
        Some(e) => git_config_set(root, "user.email", e)?,
        None => git_config_unset(root, "user.email")?,
    }
    Ok(read_repo_identity(root))
}

pub fn apply_repo_identity(root: &Path, account: &Account) -> Result<RepoIdentity, String> {
    let name = account.name.clone().or_else(|| account.github_login.clone());
    if let Some(n) = &name {
        git_config_set(root, "user.name", n)?;
    }
    if let Some(e) = &account.email {
        git_config_set(root, "user.email", e)?;
    }
    Ok(read_repo_identity(root))
}

/// Validates `root` (GitHub `origin`, not already bound), adds the binding, applies the identity and saves the config.
pub fn create_binding(cfg: &mut AppConfig, account_id: Uuid, root: &Path) -> Result<RepoBinding, String> {
    let account = cfg
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .cloned()
        .ok_or_else(|| "Conta desconhecida.".to_string())?;

    let canon: PathBuf = root.canonicalize().map_err(|e| format!("Caminho inválido: {e}"))?;

    let remotes = read_remotes(&canon);
    let picked = pick_remote(&remotes);
    let (owner, repo) = match &picked {
        RemotePick::GitHub { id, .. } => (id.owner.clone(), id.repo.clone()),
        RemotePick::Bitbucket { id, .. } => (id.workspace.clone(), id.slug.clone()),
        RemotePick::Other { name, url } => {
            return Err(format!(
                "O remoto `{name}` não aponta para github.com nem bitbucket.org (`{url}`)."
            ));
        }
        RemotePick::None => {
            return Err("O repositório não tem remoto. Adicione o remoto `origin` ou escolha outro clone.".to_string());
        }
    };

    if let Some(acc) = cfg.accounts.iter().find(|a| a.id == account_id) {
        use crate::config::AccountHost;
        let remote_host = match &picked {
            RemotePick::GitHub { .. } => AccountHost::Github,
            RemotePick::Bitbucket { .. } => AccountHost::Bitbucket,
            _ => AccountHost::Github,
        };
        if acc.host != remote_host {
            return Err(format!(
                "Este repositório é do {}, mas a conta escolhida é do {}.",
                remote_host.as_str(),
                acc.host.as_str()
            ));
        }
    }

    if cfg.repos.iter().any(|r| r.root_path == canon) {
        return Err("Esta pasta já está vinculada.".to_string());
    }

    let binding = RepoBinding {
        id: Uuid::new_v4(),
        account_id,
        root_path: canon.clone(),
        github_owner: Some(owner),
        github_repo: Some(repo),
    };
    cfg.repos.push(binding.clone());
    cfg.save().map_err(|e| e.to_string())?;

    apply_repo_identity(&canon, &account)?;
    Ok(binding)
}

// --- Identity status ---

/// What `apply_repo_identity` writes for `account` (same fallback: profile name, else login).
pub fn expected_identity(account: &Account) -> RepoIdentity {
    RepoIdentity {
        name: account.name.clone().or_else(|| account.github_login.clone()),
        email: account.email.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityState {
    /// Every field the account defines matches the repo's local config.
    Correct,
    /// At least one local value is set but differs from (or lacks) the account's value.
    Different,
    /// Neither `user.name` nor `user.email` is set locally (commits fall back to the global identity).
    NotSet,
}

/// Compares local `user.name`/`user.email` with the account's. Emails compare case-insensitively.
pub fn identity_state(local: &RepoIdentity, expected: &RepoIdentity) -> IdentityState {
    if local.name.is_none() && local.email.is_none() {
        return IdentityState::NotSet;
    }
    let name_ok = expected
        .name
        .as_deref()
        .is_none_or(|e| local.name.as_deref() == Some(e));
    let email_ok = expected.email.as_deref().is_none_or(|e| {
        local
            .email
            .as_deref()
            .is_some_and(|l| l.eq_ignore_ascii_case(e))
    });
    if name_ok && email_ok {
        IdentityState::Correct
    } else {
        IdentityState::Different
    }
}

fn git_config_unset(root: &Path, key: &str) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--local", "--unset-all", "--", key])
        .output()
        .map_err(|e| format!("Não foi possível rodar o Git: {e}"))?;
    // Exit code 5: key was not set.
    if out.status.success() || out.status.code() == Some(5) {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Removes local `user.name`/`user.email` only where they still hold the account's values,
/// so identities the user set by hand survive.
pub fn clear_repo_identity(root: &Path, account: &Account) -> Result<RepoIdentity, String> {
    let local = read_repo_identity(root);
    let expected = expected_identity(account);
    if local.name.is_some() && local.name == expected.name {
        git_config_unset(root, "user.name")?;
    }
    if let (Some(l), Some(e)) = (&local.email, &expected.email) {
        if l.eq_ignore_ascii_case(e) {
            git_config_unset(root, "user.email")?;
        }
    }
    Ok(read_repo_identity(root))
}

// --- Binding mutations ---

/// Removes the binding (and UI references to it) from `cfg` without saving.
pub fn remove_binding_in(cfg: &mut AppConfig, binding_id: Uuid) -> Option<RepoBinding> {
    let idx = cfg.repos.iter().position(|r| r.id == binding_id)?;
    let removed = cfg.repos.remove(idx);
    if cfg.ui.pinned_repo_binding_id == Some(binding_id) {
        cfg.ui.pinned_repo_binding_id = None;
    }
    if cfg.ui.last_active_repo_binding_id == Some(binding_id) {
        cfg.ui.last_active_repo_binding_id = None;
    }
    Some(removed)
}

/// Unbinds and saves. With `reset_identity`, first clears the identity GitPass applied (if the folder still exists).
pub fn remove_binding(cfg: &mut AppConfig, binding_id: Uuid, reset_identity: bool) -> Result<RepoBinding, String> {
    let binding = cfg
        .repos
        .iter()
        .find(|r| r.id == binding_id)
        .cloned()
        .ok_or_else(|| "Vínculo desconhecido. Atualize a lista e tente de novo.".to_string())?;
    if reset_identity && binding.root_path.is_dir() {
        if let Some(acc) = cfg.accounts.iter().find(|a| a.id == binding.account_id) {
            clear_repo_identity(&binding.root_path, acc)
                .map_err(|e| format!("Não foi possível restaurar a identidade do Git: {e}"))?;
        }
    }
    remove_binding_in(cfg, binding_id);
    cfg.save().map_err(|e| e.to_string())?;
    Ok(binding)
}

fn binding_and_account(cfg: &AppConfig, binding_id: Uuid) -> Result<(RepoBinding, Account), String> {
    let binding = cfg
        .repos
        .iter()
        .find(|r| r.id == binding_id)
        .cloned()
        .ok_or_else(|| "Vínculo desconhecido. Atualize a lista e tente de novo.".to_string())?;
    let account = cfg
        .accounts
        .iter()
        .find(|a| a.id == binding.account_id)
        .cloned()
        .ok_or_else(|| "A conta deste vínculo não existe mais. Troque o vínculo para outra conta.".to_string())?;
    Ok((binding, account))
}

fn ensure_repo_dir(binding: &RepoBinding) -> Result<(), String> {
    if binding.root_path.is_dir() {
        Ok(())
    } else {
        Err(format!(
            "A pasta {} não existe mais. Desvincule-a ou clone de novo.",
            binding.root_path.display()
        ))
    }
}

/// Points the binding at another account, saves, and reapplies the identity.
pub fn change_binding_account(cfg: &mut AppConfig, binding_id: Uuid, account_id: Uuid) -> Result<RepoIdentity, String> {
    let account = cfg
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .cloned()
        .ok_or_else(|| "Conta desconhecida.".to_string())?;
    let binding = cfg
        .repos
        .iter_mut()
        .find(|r| r.id == binding_id)
        .ok_or_else(|| "Vínculo desconhecido. Atualize a lista e tente de novo.".to_string())?;
    binding.account_id = account_id;
    let binding = binding.clone();
    cfg.save().map_err(|e| e.to_string())?;
    ensure_repo_dir(&binding)?;
    apply_repo_identity(&binding.root_path, &account)
}

/// Reapplies the bound account's `user.name`/`user.email`.
pub fn fix_binding_identity(cfg: &AppConfig, binding_id: Uuid) -> Result<RepoIdentity, String> {
    let (binding, account) = binding_and_account(cfg, binding_id)?;
    ensure_repo_dir(&binding)?;
    apply_repo_identity(&binding.root_path, &account)
}

// --- Account suggestion ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionReason {
    /// The account's login equals the `origin` owner.
    OwnerLogin,
    /// Other repos of the same owner are already bound to this account.
    SameOwnerBinding,
    /// It is the only account.
    OnlyAccount,
    /// The GitHub API confirmed push access for this account only.
    PushAccess,
    /// User rule: folder prefix (and optional owner).
    FolderRule,
    /// User rule: GitHub organization / owner.
    OrgRule,
    /// User rule: folder prefix and owner together.
    FolderOrgRule,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AccountSuggestion {
    pub account_id: Uuid,
    pub reason: SuggestionReason,
    /// When the pick comes from [`crate::account_rules`], a short UI label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_label: Option<String>,
}

/// Suggestion without network: login match, then the account most used for that owner, then the only account.
pub fn suggest_account_offline(cfg: &AppConfig, owner: &str) -> Option<AccountSuggestion> {
    let owner = normalize_owner(owner);
    if let Some(a) = cfg.accounts.iter().find(|a| {
        a.github_login
            .as_deref()
            .is_some_and(|l| normalize_owner(l) == owner)
    }) {
        return Some(AccountSuggestion {
            account_id: a.id,
            reason: SuggestionReason::OwnerLogin,
            rule_label: None,
        });
    }

    let mut counts: HashMap<Uuid, usize> = HashMap::new();
    for r in &cfg.repos {
        let same_owner = r
            .github_owner
            .as_deref()
            .is_some_and(|o| normalize_owner(o) == owner);
        if same_owner && cfg.accounts.iter().any(|a| a.id == r.account_id) {
            *counts.entry(r.account_id).or_default() += 1;
        }
    }
    // Ties resolve to the account listed first in the config.
    let best = cfg
        .accounts
        .iter()
        .filter_map(|a| counts.get(&a.id).map(|n| (a.id, *n)))
        .fold(None::<(Uuid, usize)>, |acc, (id, n)| match acc {
            Some((_, m)) if m >= n => acc,
            _ => Some((id, n)),
        });
    if let Some((account_id, _)) = best {
        return Some(AccountSuggestion {
            account_id,
            reason: SuggestionReason::SameOwnerBinding,
            rule_label: None,
        });
    }

    if cfg.accounts.len() == 1 {
        return Some(AccountSuggestion {
            account_id: cfg.accounts[0].id,
            reason: SuggestionReason::OnlyAccount,
            rule_label: None,
        });
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountDecision {
    /// Unambiguous: bind with this account.
    Bind(AccountSuggestion),
    /// The user must confirm; `suggested` is preselected in the UI.
    Ask { suggested: Option<Uuid> },
}

/// Combines the offline suggestion with per-account push checks (`None` = could not verify).
pub fn decide_account(offline: Option<AccountSuggestion>, push: &[(Uuid, Option<bool>)]) -> AccountDecision {
    if let Some(s) = offline.as_ref().filter(|s| {
        matches!(
            s.reason,
            SuggestionReason::OwnerLogin
                | SuggestionReason::FolderRule
                | SuggestionReason::OrgRule
                | SuggestionReason::FolderOrgRule
        )
    }) {
        return AccountDecision::Bind(s.clone());
    }
    let pushers: Vec<Uuid> = push
        .iter()
        .filter(|(_, p)| *p == Some(true))
        .map(|(id, _)| *id)
        .collect();
    if pushers.len() == 1 {
        return AccountDecision::Bind(AccountSuggestion {
            account_id: pushers[0],
            reason: SuggestionReason::PushAccess,
            rule_label: None,
        });
    }
    if let Some(s) = offline.as_ref() {
        if pushers.contains(&s.account_id) {
            return AccountDecision::Bind(s.clone());
        }
    }
    let suggested = offline
        .as_ref()
        .map(|s| s.account_id)
        .filter(|id| push.iter().all(|(a, p)| a != id || *p != Some(false)))
        .or_else(|| pushers.first().copied());
    AccountDecision::Ask { suggested }
}

/// Who may be bound to this repo when the user switches accounts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BindingSwitchOption {
    pub account_id: Uuid,
    /// `None` when the check was not run (offline / Bitbucket).
    pub can_push: Option<bool>,
    pub owner_match: bool,
    pub recommended: bool,
    pub allowed: bool,
}

/// Accounts allowed for this binding: owner/login match, confirmed push, or the recommended pick.
pub fn binding_switch_options(
    cfg: &AppConfig,
    binding: &RepoBinding,
    push_checks: &[(Uuid, Option<bool>)],
) -> Vec<BindingSwitchOption> {
    use crate::remote::normalize_owner;

    let owner = binding.github_owner.as_deref();
    let norm = owner.map(normalize_owner);
    let offline = owner
        .and_then(|o| crate::account_rules::suggest_account_for_repo(cfg, &binding.root_path, o))
        .or_else(|| owner.and_then(|o| suggest_account_offline(cfg, o)));
    let recommended = offline.as_ref().map(|s| s.account_id).or_else(|| {
        let pushers: Vec<Uuid> = push_checks
            .iter()
            .filter(|(_, p)| *p == Some(true))
            .map(|(id, _)| *id)
            .collect();
        (pushers.len() == 1).then_some(pushers[0])
    });

    cfg.accounts
        .iter()
        .map(|a| {
            let owner_match = match (&norm, a.github_login.as_deref()) {
                (Some(n), Some(l)) => normalize_owner(l) == *n,
                _ => false,
            };
            let can_push = push_checks
                .iter()
                .find(|(id, _)| *id == a.id)
                .map(|(_, p)| *p)
                .unwrap_or(None);
            let is_rec = recommended == Some(a.id);
            let mut allowed = can_push == Some(true) || owner_match || is_rec;
            if can_push == Some(false) && !owner_match && !is_rec {
                allowed = false;
            }
            BindingSwitchOption {
                account_id: a.id,
                can_push,
                owner_match,
                recommended: is_rec,
                allowed,
            }
        })
        .collect()
}

pub fn account_allowed_for_binding(
    cfg: &AppConfig,
    binding: &RepoBinding,
    account_id: Uuid,
    push_checks: &[(Uuid, Option<bool>)],
) -> bool {
    binding_switch_options(cfg, binding, push_checks)
        .into_iter()
        .find(|o| o.account_id == account_id)
        .is_some_and(|o| o.allowed)
}

// --- Repo inspection ---

/// The repo's git directory (`.git`, or the target of a `.git` file in worktrees/submodules).
pub fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    let meta = std::fs::symlink_metadata(&dot).ok()?;
    if meta.is_dir() {
        return Some(dot);
    }
    let raw = std::fs::read_to_string(&dot).ok()?;
    let target = raw.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))?.trim();
    let p = PathBuf::from(target);
    Some(if p.is_absolute() { p } else { root.join(p) })
}

/// `(name, first url)` of every `[remote "<name>"]` in a git config file's text, in file order.
pub fn remotes_from_git_config(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            let header = line.trim_start_matches('[').split(']').next().unwrap_or("").trim();
            let mut parts = header.splitn(2, char::is_whitespace);
            let section = parts.next().unwrap_or("");
            let sub = parts.next().unwrap_or("").trim().trim_matches('"');
            current = (section.eq_ignore_ascii_case("remote") && !sub.is_empty()).then(|| sub.to_string());
            continue;
        }
        let Some(name) = &current else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("url") {
            let v = value.trim().trim_matches('"').trim();
            if !v.is_empty() && !out.iter().any(|(n, _)| n == name) {
                out.push((name.clone(), v.to_string()));
            }
        }
    }
    out
}

/// First `url` of `[remote "origin"]` in a git config file's text.
pub fn origin_url_from_git_config(text: &str) -> Option<String> {
    remotes_from_git_config(text)
        .into_iter()
        .find(|(n, _)| n == "origin")
        .map(|(_, u)| u)
}

/// Remotes of the repo, from `.git/config` when possible (no process spawn), else via `git config`
/// (worktrees and submodules keep their config in another git dir).
pub fn read_remotes(root: &Path) -> Vec<(String, String)> {
    let dot = root.join(".git");
    if dot.is_dir() {
        return std::fs::read_to_string(dot.join("config"))
            .map(|t| remotes_from_git_config(&t))
            .unwrap_or_default();
    }
    if std::fs::symlink_metadata(&dot).is_err() {
        return Vec::new();
    }
    let Some(listing) = git_stdout(root, &["config", "--local", "--get-regexp", r"^remote\..*\.url$"]) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for line in listing.lines() {
        let Some((key, url)) = line.split_once(' ') else {
            continue;
        };
        let Some(name) = key.strip_prefix("remote.").and_then(|k| k.strip_suffix(".url")) else {
            continue;
        };
        let url = url.trim();
        if !name.is_empty() && !url.is_empty() && !out.iter().any(|(n, _)| n == name) {
            out.push((name.to_string(), url.to_string()));
        }
    }
    out
}

/// The remote that identifies a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemotePick {
    GitHub { name: String, id: GitHubRepoId },
    Bitbucket { name: String, id: BitbucketRepoId },
    /// Unsupported host; `url` has any embedded credentials removed.
    Other { name: String, url: String },
    None,
}

fn pick_known_remote(name: &str, url: &str) -> Option<RemotePick> {
    if let Some(id) = parse_github_remote(url) {
        return Some(RemotePick::GitHub {
            name: name.to_string(),
            id,
        });
    }
    if let Some(id) = parse_bitbucket_remote(url) {
        return Some(RemotePick::Bitbucket {
            name: name.to_string(),
            id,
        });
    }
    None
}

/// `origin` when it exists (GitHub/Bitbucket or not), else the first known host, else the first remote.
pub fn pick_remote(remotes: &[(String, String)]) -> RemotePick {
    let other = |(name, url): &(String, String)| RemotePick::Other {
        name: name.clone(),
        url: redact_remote_url(url),
    };
    if let Some(origin) = remotes.iter().find(|(n, _)| n == "origin") {
        return pick_known_remote(&origin.0, &origin.1).unwrap_or_else(|| other(origin));
    }
    if let Some((name, url)) = remotes
        .iter()
        .find(|(n, u)| pick_known_remote(n, u).is_some())
    {
        return pick_known_remote(name, url).unwrap();
    }
    remotes.first().map(other).unwrap_or(RemotePick::None)
}

/// `origin` URL, read from the config file when possible (no process spawn), else via `git remote get-url`.
pub fn read_origin_url(root: &Path) -> Option<String> {
    let dot = root.join(".git");
    if dot.is_dir() {
        let text = std::fs::read_to_string(dot.join("config")).ok()?;
        return origin_url_from_git_config(&text);
    }
    git_remote_url(root, "origin").ok()
}

/// `https://github.com/<owner>/<repo>` — never contains credentials, whatever the raw remote holds.
pub fn github_https_url(owner: &str, repo: &str) -> String {
    format!("https://github.com/{owner}/{repo}")
}

/// True when an HTTP(S) remote URL embeds a password or token (e.g. `https://user:ghp_…@github.com/…`).
pub fn remote_url_has_credentials(raw: &str) -> bool {
    let Ok(u) = url::Url::parse(raw.trim()) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    if u.password().is_some() {
        return true;
    }
    let user = u.username();
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "x-access-token", "x-oauth-basic"]
        .iter()
        .any(|p| user.starts_with(p))
}

/// The remote URL safe to show: without the user/password part when it carries credentials.
pub fn redact_remote_url(raw: &str) -> String {
    let raw = raw.trim();
    if remote_url_has_credentials(raw) {
        if let Ok(mut u) = url::Url::parse(raw) {
            let _ = u.set_password(None);
            let _ = u.set_username("");
            return u.to_string();
        }
    }
    raw.to_string()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct HeadInfo {
    /// Current branch (also for unborn branches without commits).
    pub branch: Option<String>,
    /// Short commit hash when HEAD is detached.
    pub detached_at: Option<String>,
}

fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn read_head(root: &Path) -> HeadInfo {
    if let Some(branch) = git_stdout(root, &["symbolic-ref", "--short", "-q", "HEAD"]) {
        return HeadInfo {
            branch: Some(branch),
            detached_at: None,
        };
    }
    HeadInfo {
        branch: None,
        detached_at: git_stdout(root, &["rev-parse", "--short", "HEAD"]),
    }
}

// --- Discovery of local clones ---

/// Directory names never descended into while discovering clones (hidden dirs are skipped too).
pub const DISCOVER_IGNORED_DIRS: &[&str] = &[
    "node_modules",
    "Library",
    "target",
    "vendor",
    "Pods",
    "DerivedData",
    "build",
    "dist",
    "venv",
    "__pycache__",
    "Applications",
];

pub const DISCOVER_MAX_DEPTH: usize = 3;

pub fn is_ignored_dir_name(name: &str) -> bool {
    name.starts_with('.') || DISCOVER_IGNORED_DIRS.contains(&name)
}

/// Common places where people keep clones, relative to `home`.
pub fn default_discover_roots(home: &Path) -> Vec<PathBuf> {
    ["Developer", "Projects", "GitHub", "Code", "src", "Documents/GitHub", "Desktop"]
        .iter()
        .map(|p| home.join(p))
        .collect()
}

/// Work trees (dirs containing `.git`) under `roots`, up to `max_depth` levels below each root.
/// Does not descend into repos, ignored dirs or symlinks. Returns `(paths, truncated_by_deadline)`.
pub fn find_git_work_trees(roots: &[PathBuf], max_depth: usize, deadline: Option<Instant>) -> (Vec<PathBuf>, bool) {
    let mut found = Vec::new();
    let truncated = walk_git_work_trees(roots, max_depth, deadline, |p| {
        found.push(p);
        true
    });
    found.sort();
    (found, truncated)
}

/// Breadth-first walk behind [`find_git_work_trees`]: calls `visit` for each work tree as soon as it
/// is found; `visit` returns `false` to stop. Returns `true` when stopped early (deadline or `visit`).
pub fn walk_git_work_trees(
    roots: &[PathBuf],
    max_depth: usize,
    deadline: Option<Instant>,
    mut visit: impl FnMut(PathBuf) -> bool,
) -> bool {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    for r in roots {
        if let Ok(c) = r.canonicalize() {
            if c.is_dir() && seen.insert(c.clone()) {
                queue.push_back((c, 0));
            }
        }
    }
    while let Some((dir, depth)) = queue.pop_front() {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return true;
        }
        if std::fs::symlink_metadata(dir.join(".git")).is_ok() {
            if !visit(dir) {
                return true;
            }
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| !is_ignored_dir_name(&e.file_name().to_string_lossy()))
            .map(|e| e.path())
            .collect();
        children.sort();
        for c in children {
            if seen.insert(c.clone()) {
                queue.push_back((c, depth + 1));
            }
        }
    }
    false
}

/// Like [`find_git_work_trees`], one thread per root. Returns at `deadline` even if a root is stuck
/// (e.g. macOS blocks reads of ~/Desktop until the user answers the privacy prompt); stuck threads finish detached.
pub fn find_git_work_trees_parallel(
    roots: &[PathBuf],
    max_depth: usize,
    deadline: Option<Instant>,
) -> (Vec<PathBuf>, bool) {
    let (tx, rx) = std::sync::mpsc::channel();
    for r in roots {
        let tx = tx.clone();
        let r = r.clone();
        std::thread::spawn(move || {
            let _ = tx.send(find_git_work_trees(&[r], max_depth, deadline));
        });
    }
    drop(tx);

    // Walkers stop at `deadline` themselves; the grace lets their partial results arrive.
    let collect_until = deadline.map(|d| d + std::time::Duration::from_millis(300));
    let mut found = Vec::new();
    let mut truncated = false;
    for _ in 0..roots.len() {
        let res = match collect_until {
            Some(d) => rx
                .recv_timeout(d.saturating_duration_since(Instant::now()))
                .map_err(|_| ()),
            None => rx.recv().map_err(|_| ()),
        };
        match res {
            Ok((f, t)) => {
                found.extend(f);
                truncated |= t;
            }
            Err(()) => {
                truncated = true;
                break;
            }
        }
    }
    found.sort();
    found.dedup();
    (found, truncated)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredRepo {
    pub root_path: PathBuf,
    pub github_owner: String,
    pub github_repo: String,
    pub suggestion: Option<AccountSuggestion>,
}

/// GitHub clones under `roots` that are not bound yet, each with an offline account suggestion.
pub fn discover_unbound_repos(
    cfg: &AppConfig,
    roots: &[PathBuf],
    max_depth: usize,
    deadline: Option<Instant>,
) -> (Vec<DiscoveredRepo>, bool) {
    let bound: HashSet<PathBuf> = cfg
        .repos
        .iter()
        .map(|r| r.root_path.canonicalize().unwrap_or_else(|_| r.root_path.clone()))
        .collect();
    let (trees, truncated) = find_git_work_trees_parallel(roots, max_depth, deadline);
    let repos = trees
        .into_iter()
        .filter(|p| !bound.contains(p))
        .filter_map(|p| {
            let (owner, repo) = match pick_remote(&read_remotes(&p)) {
                RemotePick::GitHub { id, .. } => (id.owner, id.repo),
                RemotePick::Bitbucket { id, .. } => (id.workspace, id.slug),
                _ => return None,
            };
            Some(DiscoveredRepo {
                suggestion: crate::account_rules::suggest_account_for_repo(cfg, &p, &owner),
                root_path: p,
                github_owner: owner,
                github_repo: repo,
            })
        })
        .collect();
    (repos, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("gitpass-bindings-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
    }

    fn init_repo(dir: &Path, origin: Option<&str>) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        if let Some(o) = origin {
            git(dir, &["remote", "add", "origin", o]);
        }
    }

    fn account(login: &str) -> Account {
        Account {
            id: Uuid::new_v4(),
            github_login: Some(login.into()),
            ..Default::default()
        }
    }

    fn binding(account_id: Uuid, owner: &str) -> RepoBinding {
        RepoBinding {
            id: Uuid::new_v4(),
            account_id,
            root_path: PathBuf::from(format!("/tmp/none/{owner}")),
            github_owner: Some(owner.into()),
            github_repo: Some("r".into()),
        }
    }

    fn ident(name: Option<&str>, email: Option<&str>) -> RepoIdentity {
        RepoIdentity {
            name: name.map(String::from),
            email: email.map(String::from),
        }
    }

    #[test]
    fn identity_state_cases() {
        let exp = ident(Some("Ana"), Some("ana@x.pt"));
        assert_eq!(identity_state(&ident(None, None), &exp), IdentityState::NotSet);
        assert_eq!(identity_state(&ident(Some("Ana"), Some("ANA@x.pt")), &exp), IdentityState::Correct);
        assert_eq!(identity_state(&ident(Some("Ana"), None), &exp), IdentityState::Different);
        assert_eq!(identity_state(&ident(Some("Bob"), Some("ana@x.pt")), &exp), IdentityState::Different);
        // Account without email: only the name is compared.
        assert_eq!(identity_state(&ident(Some("Ana"), Some("o@x")), &ident(Some("Ana"), None)), IdentityState::Correct);
    }

    #[test]
    fn expected_identity_falls_back_to_login() {
        let mut a = account("octo");
        assert_eq!(expected_identity(&a).name.as_deref(), Some("octo"));
        a.name = Some("Octo Cat".into());
        assert_eq!(expected_identity(&a).name.as_deref(), Some("Octo Cat"));
    }

    #[test]
    fn suggests_by_owner_login_then_bindings_then_only_account() {
        let work = account("ACME-dev");
        let personal = account("ana");
        let mut cfg = AppConfig::default();
        cfg.accounts = vec![work.clone(), personal.clone()];

        let s = suggest_account_offline(&cfg, "acme-dev").unwrap();
        assert_eq!((s.account_id, s.reason), (work.id, SuggestionReason::OwnerLogin));

        assert_eq!(suggest_account_offline(&cfg, "some-org"), None);

        cfg.repos = vec![binding(personal.id, "some-org"), binding(work.id, "Some-Org"), binding(work.id, "some-org")];
        let s = suggest_account_offline(&cfg, "some-org").unwrap();
        assert_eq!((s.account_id, s.reason), (work.id, SuggestionReason::SameOwnerBinding));

        cfg.repos.clear();
        cfg.accounts = vec![personal.clone()];
        let s = suggest_account_offline(&cfg, "other").unwrap();
        assert_eq!((s.account_id, s.reason), (personal.id, SuggestionReason::OnlyAccount));
    }

    #[test]
    fn decide_account_cases() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let owner = Some(AccountSuggestion {
            account_id: a,
            reason: SuggestionReason::OwnerLogin,
            rule_label: None,
        });
        assert!(matches!(decide_account(owner, &[(a, Some(false))]), AccountDecision::Bind(s) if s.account_id == a));

        let d = decide_account(None, &[(a, Some(false)), (b, Some(true))]);
        assert!(matches!(d, AccountDecision::Bind(s) if s.account_id == b && s.reason == SuggestionReason::PushAccess));

        let same = Some(AccountSuggestion {
            account_id: b,
            reason: SuggestionReason::SameOwnerBinding,
            rule_label: None,
        });
        assert!(matches!(decide_account(same, &[(a, Some(true)), (b, Some(true))]), AccountDecision::Bind(s) if s.account_id == b));

        assert_eq!(decide_account(None, &[(a, Some(true)), (b, Some(true))]), AccountDecision::Ask { suggested: Some(a) });
        assert_eq!(decide_account(None, &[(a, None), (b, None)]), AccountDecision::Ask { suggested: None });
        let only = Some(AccountSuggestion {
            account_id: a,
            reason: SuggestionReason::OnlyAccount,
            rule_label: None,
        });
        assert_eq!(
            decide_account(only.clone(), &[(a, Some(false))]),
            AccountDecision::Ask { suggested: None }
        );
        assert_eq!(decide_account(only, &[(a, None)]), AccountDecision::Ask { suggested: Some(a) });
    }

    #[test]
    fn parses_origin_from_git_config() {
        let text = r#"
[core]
	bare = false
[remote "upstream"]
	url = git@github.com:up/stream.git
[remote "origin"]
	url = https://github.com/Me/proj.git
	fetch = +refs/heads/*:refs/remotes/origin/*
"#;
        assert_eq!(origin_url_from_git_config(text).as_deref(), Some("https://github.com/Me/proj.git"));
        assert_eq!(origin_url_from_git_config("[remote \"up\"]\nurl = x"), None);
        assert_eq!(
            remotes_from_git_config(text),
            vec![
                ("upstream".to_string(), "git@github.com:up/stream.git".to_string()),
                ("origin".to_string(), "https://github.com/Me/proj.git".to_string()),
            ]
        );
    }

    fn remotes(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(n, u)| (n.to_string(), u.to_string())).collect()
    }

    #[test]
    fn pick_remote_prefers_origin_then_first_github() {
        let gh = |name: &str, owner: &str, repo: &str| RemotePick::GitHub {
            name: name.into(),
            id: GitHubRepoId { owner: owner.into(), repo: repo.into() },
        };
        assert_eq!(
            pick_remote(&remotes(&[("up", "git@github.com:up/x.git"), ("origin", "https://github.com/Me/y")])),
            gh("origin", "me", "y")
        );
        // A non-GitHub origin wins over other GitHub remotes.
        assert_eq!(
            pick_remote(&remotes(&[("origin", "https://ana:pw@gitlab.com/a/b.git"), ("gh", "git@github.com:a/b.git")])),
            RemotePick::Other { name: "origin".into(), url: "https://gitlab.com/a/b.git".into() }
        );
        assert_eq!(
            pick_remote(&remotes(&[("lab", "git@gitlab.com:a/b.git"), ("gh", "git@github.com:A/b.git")])),
            gh("gh", "a", "b")
        );
        assert_eq!(
            pick_remote(&remotes(&[("lab", "git@gitlab.com:a/b.git")])),
            RemotePick::Other { name: "lab".into(), url: "git@gitlab.com:a/b.git".into() }
        );
        assert_eq!(pick_remote(&[]), RemotePick::None);
    }

    #[test]
    fn reads_remotes_of_plain_repos_and_worktrees() {
        let t = TempDir::new();
        let main = t.0.join("main");
        init_repo(&main, None);
        git(&main, &["remote", "add", "github", "git@github.com:ana/proj.git"]);
        git(&main, &["remote", "add", "lab", "https://gitlab.com/ana/proj.git"]);
        let expected = remotes(&[("github", "git@github.com:ana/proj.git"), ("lab", "https://gitlab.com/ana/proj.git")]);
        assert_eq!(read_remotes(&main), expected);

        // The user's global hooks (e.g. GitPass's own) must not run in tests.
        let no_hooks = ["-c", "core.hooksPath=/dev/null", "-c", "user.name=t", "-c", "user.email=t@x"];
        git(&main, &[&no_hooks[..], &["commit", "-q", "--no-verify", "--allow-empty", "-m", "init"]].concat());
        let wt = t.0.join("wt");
        git(&main, &[&no_hooks[..], &["worktree", "add", "-q", wt.to_str().unwrap()]].concat());
        assert!(wt.join(".git").is_file());
        let mut got = read_remotes(&wt);
        got.sort();
        assert_eq!(got, expected);
        assert_eq!(read_remotes(&t.0), Vec::<(String, String)>::new());
    }

    #[test]
    fn detects_credentials_in_remote() {
        assert!(remote_url_has_credentials("https://ana:secret@github.com/a/b.git"));
        assert!(remote_url_has_credentials("https://ghp_abc123@github.com/a/b.git"));
        assert!(!remote_url_has_credentials("https://ana@github.com/a/b.git"));
        assert!(!remote_url_has_credentials("git@github.com:a/b.git"));
        assert_eq!(github_https_url("a", "b"), "https://github.com/a/b");
    }

    #[test]
    fn remote_credentials_are_redacted_for_display() {
        assert_eq!(redact_remote_url("https://ana:secret@gitlab.com/a/b.git"), "https://gitlab.com/a/b.git");
        assert_eq!(redact_remote_url("https://ghp_abc@example.com/a/b"), "https://example.com/a/b");
        assert_eq!(redact_remote_url("git@gitlab.com:a/b.git"), "git@gitlab.com:a/b.git");
    }

    #[test]
    fn create_binding_error_never_shows_remote_credentials() {
        let t = TempDir::new();
        init_repo(&t.0, Some("https://ana:s3cr3t@gitlab.com/a/b.git"));
        let acc = account("ana");
        let mut cfg = AppConfig::default();
        cfg.accounts.push(acc.clone());
        let err = create_binding(&mut cfg, acc.id, &t.0).unwrap_err();
        assert!(!err.contains("s3cr3t"), "{err}");
        assert!(cfg.repos.is_empty());
    }

    #[test]
    fn identity_values_that_look_like_options_are_stored_verbatim() {
        let t = TempDir::new();
        init_repo(&t.0, Some("https://github.com/ana/x"));
        let mut acc = account("ana");
        acc.name = Some("--global".into());
        acc.email = Some("--file=/tmp/x".into());
        let got = apply_repo_identity(&t.0, &acc).unwrap();
        assert_eq!(got.name.as_deref(), Some("--global"));
        assert_eq!(got.email.as_deref(), Some("--file=/tmp/x"));
    }

    #[test]
    fn ignored_dir_names() {
        for n in ["node_modules", "Library", ".git", ".cache", "target"] {
            assert!(is_ignored_dir_name(n), "{n}");
        }
        for n in ["projects", "my-app", "Code"] {
            assert!(!is_ignored_dir_name(n), "{n}");
        }
    }

    #[test]
    fn finds_work_trees_respecting_depth_and_ignores() {
        let t = TempDir::new();
        let root = t.0.join("Developer");
        init_repo(&root.join("app"), Some("git@github.com:acme/app.git"));
        init_repo(&root.join("org/team/svc"), Some("https://github.com/acme/svc"));
        init_repo(&root.join("a/b/c/too-deep"), Some("https://github.com/acme/deep"));
        init_repo(&root.join("web/node_modules/dep"), Some("https://github.com/x/dep"));
        init_repo(&root.join(".hidden/repo"), Some("https://github.com/x/hidden"));
        // Nested repo inside a repo is not reported separately.
        init_repo(&root.join("app/vendored"), Some("https://github.com/x/vendored"));
        std::fs::create_dir_all(root.join("app/.git/inner")).unwrap();

        let (found, truncated) = find_git_work_trees(&[root.clone(), t.0.join("missing")], DISCOVER_MAX_DEPTH, None);
        assert!(!truncated);
        assert_eq!(found, vec![root.join("app"), root.join("org/team/svc")]);

        // Overlapping roots are merged without duplicates.
        let deadline = Some(Instant::now() + std::time::Duration::from_secs(10));
        let (par, truncated) = find_git_work_trees_parallel(&[root.clone(), root.join("org")], DISCOVER_MAX_DEPTH, deadline);
        assert!(!truncated);
        assert_eq!(par, found);
    }

    #[test]
    fn discovery_skips_bound_and_non_github_and_suggests() {
        let t = TempDir::new();
        init_repo(&t.0.join("mine"), Some("git@github.com:Ana/mine.git"));
        init_repo(&t.0.join("bound"), Some("git@github.com:ana/bound.git"));
        init_repo(&t.0.join("gitlab"), Some("git@gitlab.com:ana/x.git"));
        init_repo(&t.0.join("no-origin"), None);

        let ana = account("ana");
        let other = account("bob");
        let mut cfg = AppConfig::default();
        cfg.accounts = vec![other, ana.clone()];
        cfg.repos.push(RepoBinding {
            root_path: t.0.join("bound"),
            ..binding(ana.id, "ana")
        });

        let (repos, _) = discover_unbound_repos(&cfg, &[t.0.clone()], 1, None);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].root_path, t.0.join("mine"));
        assert_eq!((repos[0].github_owner.as_str(), repos[0].github_repo.as_str()), ("ana", "mine"));
        assert_eq!(repos[0].suggestion.as_ref().map(|s| s.account_id), Some(ana.id));
    }

    #[test]
    fn identity_apply_clear_and_head() {
        let t = TempDir::new();
        init_repo(&t.0, Some("https://github.com/ana/x"));
        let mut acc = account("ana");
        acc.name = Some("Ana Silva".into());
        acc.email = Some("ana@example.pt".into());

        assert_eq!(identity_state(&read_repo_identity(&t.0), &expected_identity(&acc)), IdentityState::NotSet);
        apply_repo_identity(&t.0, &acc).unwrap();
        assert_eq!(identity_state(&read_repo_identity(&t.0), &expected_identity(&acc)), IdentityState::Correct);

        // A hand-set email survives clearing; the GitPass-applied name is removed.
        git(&t.0, &["config", "--local", "user.email", "custom@x.pt"]);
        let left = clear_repo_identity(&t.0, &acc).unwrap();
        assert_eq!(left.name, None);
        assert_eq!(left.email.as_deref(), Some("custom@x.pt"));

        assert_eq!(read_head(&t.0), HeadInfo { branch: Some("main".into()), detached_at: None });
        assert_eq!(resolve_git_dir(&t.0), Some(t.0.join(".git")));
        assert_eq!(read_origin_url(&t.0).as_deref(), Some("https://github.com/ana/x"));
    }

    #[test]
    fn remove_binding_in_clears_ui_refs() {
        let acc = account("ana");
        let b = binding(acc.id, "ana");
        let mut cfg = AppConfig::default();
        cfg.accounts.push(acc);
        cfg.repos.push(b.clone());
        cfg.ui.pinned_repo_binding_id = Some(b.id);
        cfg.ui.last_active_repo_binding_id = Some(b.id);
        assert_eq!(remove_binding_in(&mut cfg, b.id).map(|r| r.id), Some(b.id));
        assert!(cfg.repos.is_empty());
        assert_eq!(cfg.ui.pinned_repo_binding_id, None);
        assert_eq!(cfg.ui.last_active_repo_binding_id, None);
        assert!(remove_binding_in(&mut cfg, b.id).is_none());
    }
}
