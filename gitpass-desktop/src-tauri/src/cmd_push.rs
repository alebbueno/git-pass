//! Tauri commands for this area. Registered in main.rs `generate_handler!`.
//!
//! Commands: `repos_outgoing`, `repos_commit_files`, `repos_push`, `repos_fix_authors`, `repos_pull`.
//! The Git work lives in `gitpass_core::outgoing`; here: binding lookup, the account's token
//! (refreshed like `cmd_cloud`), GitHub checks with a short cache, events and one operation per repo.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gitpass_core::bindings::{pick_remote, read_remotes, RemotePick};
use gitpass_core::branches;
use gitpass_core::github_api::fetch_repo_permissions;
use gitpass_core::github_user::{fetch_emails, fetch_user};
use gitpass_core::outgoing::{
    self, github_branch_url, outgoing_state, push_refusal, CommitFile, FetchOutcome, FileLimits, GitNetError,
    GitRepo, Identity, IdentityRules, OutgoingContext, OutgoingState, Permission, PushPhase, PushProgress,
    PushResult,
};
use gitpass_core::{Account, AppConfig, GithubApiError};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::cmd_cloud::{api_error_message, current_token, load_bundle, refresh_bundle};

pub(crate) const PUSH_PROGRESS_EVENT: &str = "repos-push-progress";
pub(crate) const OUTGOING_CHANGED_EVENT: &str = "repos-outgoing-changed";
const PERMISSION_TTL: Duration = Duration::from_secs(5 * 60);
const PERSON_TTL: Duration = Duration::from_secs(10 * 60);
const NOREPLY_DOMAIN: &str = "users.noreply.github.com";

pub(crate) async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Algo deu errado no app: {e}"))?
}

pub(crate) fn parse_binding(raw: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw).map_err(|_| "Identificador de repositório inválido.".to_string())
}

// --- One push, pull, author fix, checkout or merge per repository at a time ---

fn busy() -> &'static Mutex<HashSet<Uuid>> {
    static SET: OnceLock<Mutex<HashSet<Uuid>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Holds the repository for one operation; released when dropped.
pub(crate) struct BusyGuard(Uuid);

impl BusyGuard {
    pub(crate) fn acquire(id: Uuid) -> Result<Self, String> {
        let mut set = busy().lock().map_err(|_| "Algo deu errado no app. Tente de novo.".to_string())?;
        if !set.insert(id) {
            return Err("Já há um envio ou atualização em andamento neste repositório. Aguarde terminar.".into());
        }
        Ok(Self(id))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = busy().lock() {
            set.remove(&self.0);
        }
    }
}

// --- GitHub lookups (short in-memory cache) ---

struct Cached<T> {
    at: Instant,
    value: T,
}

impl<T: Clone> Cached<T> {
    fn fresh(&self, ttl: Duration) -> Option<T> {
        (self.at.elapsed() < ttl).then(|| self.value.clone())
    }
}

type PermissionKey = (Uuid, String);

fn permissions() -> &'static Mutex<HashMap<PermissionKey, Cached<bool>>> {
    static MAP: OnceLock<Mutex<HashMap<PermissionKey, Cached<bool>>>> = OnceLock::new();
    MAP.get_or_init(Default::default)
}

fn permission_key(account: Uuid, owner: &str, repo: &str) -> PermissionKey {
    (account, format!("{}/{}", owner.to_lowercase(), repo.to_lowercase()))
}

fn cached_permission(account: Uuid, owner: &str, repo: &str) -> Option<bool> {
    permissions().lock().ok()?.get(&permission_key(account, owner, repo))?.fresh(PERMISSION_TTL)
}

fn remember_permission(account: Uuid, owner: &str, repo: &str, can_push: bool) {
    if let Ok(mut m) = permissions().lock() {
        m.insert(permission_key(account, owner, repo), Cached { at: Instant::now(), value: can_push });
    }
}

/// GitHub user id and verified emails of an account.
#[derive(Clone)]
pub(crate) struct Person {
    login: String,
    user_id: u64,
    emails: Vec<String>,
}

fn people() -> &'static Mutex<HashMap<Uuid, Cached<Person>>> {
    static MAP: OnceLock<Mutex<HashMap<Uuid, Cached<Person>>>> = OnceLock::new();
    MAP.get_or_init(Default::default)
}

fn cached_person(account: Uuid) -> Option<Person> {
    people().lock().ok()?.get(&account)?.fresh(PERSON_TTL)
}

/// Exchanges the stored refresh token for a new access token.
fn refreshed_token(account: Uuid) -> Option<String> {
    let bundle = load_bundle(account).ok()?;
    refresh_bundle(account, &bundle).ok().map(|b| b.access_token)
}

/// What the app knows about the account's access while preparing one operation.
pub(crate) struct Access {
    account: Option<Uuid>,
    token: Option<String>,
    /// GitHub accepted (or has not rejected) the token.
    connected: bool,
}

impl Access {
    pub(crate) fn load(account: Option<&Account>) -> Self {
        let id = account.map(|a| a.id);
        let token = id.and_then(|id| current_token(id).ok());
        Self { account: id, connected: token.is_some(), token }
    }

    /// GitHub API call; on 401 refreshes the token once, and marks the account disconnected if still rejected.
    fn api<T>(&mut self, f: impl Fn(&str) -> Result<T, GithubApiError>) -> Option<Result<T, GithubApiError>> {
        let (Some(id), Some(token)) = (self.account, self.token.clone()) else {
            return None;
        };
        let mut r = f(&token);
        if matches!(r, Err(GithubApiError::Http { status: 401, .. })) {
            if let Some(t) = refreshed_token(id) {
                r = f(&t);
                self.token = Some(t);
            }
        }
        if matches!(r, Err(GithubApiError::Http { status: 401, .. })) {
            self.connected = false;
        }
        Some(r)
    }

    /// Git network call with the token; on an access error refreshes the token and tries once more.
    pub(crate) fn git<T>(&mut self, f: impl Fn(Option<&str>) -> Result<T, GitNetError>) -> Result<T, GitNetError> {
        let mut r = f(self.token.as_deref());
        if matches!(&r, Err(e) if e.auth) {
            if let Some(t) = self.account.and_then(refreshed_token) {
                self.token = Some(t);
                r = f(self.token.as_deref());
            }
        }
        if matches!(&r, Err(e) if e.auth) {
            self.connected = false;
        }
        r
    }

    fn permission(&mut self, owner: &str, repo: &str, online: bool) -> Permission {
        let Some(id) = self.account else {
            return Permission::NotChecked;
        };
        if let Some(can) = cached_permission(id, owner, repo) {
            return if can { Permission::Allowed } else { Permission::Denied };
        }
        if !online || !self.connected {
            return Permission::NotChecked;
        }
        match self.api(|t| fetch_repo_permissions(t, owner, repo)) {
            None => Permission::NotChecked,
            Some(Ok(p)) => {
                let can = p.admin || p.maintain || p.push;
                remember_permission(id, owner, repo, can);
                if can {
                    Permission::Allowed
                } else {
                    Permission::Denied
                }
            }
            Some(Err(GithubApiError::Http { status: 401, .. })) => Permission::NotChecked,
            Some(Err(GithubApiError::Http { status: 403, body })) if body.to_lowercase().contains("rate limit") => {
                Permission::Unknown(api_error_message(GithubApiError::Http { status: 403, body }))
            }
            // Private repos the token cannot see answer 404.
            Some(Err(GithubApiError::Http { status: 403 | 404, .. })) => {
                remember_permission(id, owner, repo, false);
                Permission::Denied
            }
            Some(Err(e)) => Permission::Unknown(api_error_message(e)),
        }
    }

    pub(crate) fn person(&mut self, online: bool) -> Option<Person> {
        let id = self.account?;
        if let Some(p) = cached_person(id) {
            return Some(p);
        }
        if !online || !self.connected {
            return None;
        }
        let user = self.api(fetch_user)?.ok()?;
        // Needs the `user:email` scope; without it only the configured email counts.
        let emails = self
            .api(fetch_emails)
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.verified)
            .map(|e| e.email)
            .collect();
        let person = Person { login: user.login, user_id: user.id, emails };
        if let Ok(mut m) = people().lock() {
            m.insert(id, Cached { at: Instant::now(), value: person.clone() });
        }
        Some(person)
    }
}

/// `ID` of `ID+login@users.noreply.github.com`.
fn noreply_user_id(email: &str) -> Option<u64> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    if !domain.eq_ignore_ascii_case(NOREPLY_DOMAIN) {
        return None;
    }
    local.split_once('+')?.0.parse().ok()
}

pub(crate) fn identity_rules(account: Option<&Account>, person: Option<Person>) -> IdentityRules {
    let Some(a) = account else {
        return IdentityRules::default();
    };
    let mut rules = IdentityRules {
        login: a.github_login.clone(),
        user_id: a.email.as_deref().and_then(noreply_user_id),
        emails: a.email.iter().cloned().collect(),
    };
    if let Some(p) = person {
        if rules.login.is_none() {
            rules.login = Some(p.login);
        }
        rules.user_id = Some(p.user_id);
        rules.emails.extend(p.emails);
    }
    rules
}

// --- Preparing an operation ---

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fetch {
    /// Local data only (no network).
    No,
    /// `git fetch` and GitHub checks first.
    Yes,
    /// The caller just talked to GitHub (pull).
    Done,
}

/// A bound repository, its remote and the account's access, plus the current state.
struct Prepared {
    binding_id: String,
    repo: GitRepo,
    remote: Option<String>,
    github: Option<(String, String)>,
    account: Option<Account>,
    access: Access,
    state: OutgoingState,
}

pub(crate) struct Bound {
    pub(crate) id: Uuid,
    pub(crate) repo: GitRepo,
    pub(crate) remote: Option<String>,
    github: Option<(String, String)>,
    pub(crate) account: Option<Account>,
}

pub(crate) fn bound(binding_id: &str) -> Result<Bound, String> {
    let id = parse_binding(binding_id)?;
    let cfg = AppConfig::load_or_default()
        .map_err(|e| format!("Não foi possível ler as configurações do GitPass: {e}"))?;
    let b = cfg
        .repos
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| "Este repositório não está vinculado ao GitPass. Atualize a lista e tente de novo.".to_string())?;
    if !b.root_path.is_dir() {
        return Err(format!(
            "A pasta {} não existe mais. Desvincule o repositório ou clone de novo.",
            b.root_path.display()
        ));
    }
    let (remote, github) = match pick_remote(&read_remotes(&b.root_path)) {
        RemotePick::GitHub { name, id } => (Some(name), Some((id.owner, id.repo))),
        _ => (None, None),
    };
    Ok(Bound {
        id,
        repo: GitRepo::new(&b.root_path),
        remote,
        github,
        account: cfg.accounts.iter().find(|a| a.id == b.account_id).cloned(),
    })
}

fn prepare(binding_id: &str, fetch: Fetch, branch: Option<&str>) -> Result<Prepared, String> {
    let b = bound(binding_id)?;
    if let Some(name) = branch {
        branches::check_branch_name(&b.repo, name)?;
    }
    let mut access = Access::load(b.account.as_ref());
    let online = fetch == Fetch::Yes;

    let fetched = match (fetch, b.remote.as_deref()) {
        (Fetch::No, _) | (_, None) => FetchOutcome::NotRequested,
        (Fetch::Done, Some(_)) => FetchOutcome::Fetched,
        (Fetch::Yes, Some(remote)) => match access.git(|t| outgoing::fetch(&b.repo, remote, t)) {
            Ok(()) => FetchOutcome::Fetched,
            Err(e) => FetchOutcome::Failed(e.message),
        },
    };
    let permission = match &b.github {
        Some((owner, repo)) => access.permission(owner, repo, online),
        None => Permission::NotChecked,
    };
    let person = access.person(online);
    let identity = identity_rules(b.account.as_ref(), person);

    let binding_id = b.id.to_string();
    let state = outgoing_state(
        &b.repo,
        &OutgoingContext {
            binding_id: &binding_id,
            branch,
            remote: b.remote.as_deref(),
            github: b.github.as_ref().map(|(o, r)| (o.as_str(), r.as_str())),
            identity: &identity,
            account_connected: b.account.is_some() && access.connected,
            permission,
            fetch: fetched,
            limits: FileLimits::default(),
        },
    )?;
    Ok(Prepared {
        binding_id,
        repo: b.repo,
        remote: b.remote,
        github: b.github,
        account: b.account,
        access,
        state,
    })
}

pub(crate) fn notify_changed(app: &AppHandle, binding_id: &str) {
    let _ = app.emit(OUTGOING_CHANGED_EVENT, binding_id.to_string());
}

fn emit_progress(app: &AppHandle, binding_id: &str, phase: PushPhase, percent: Option<u8>) {
    let _ = app.emit(
        PUSH_PROGRESS_EVENT,
        PushProgress { binding_id: binding_id.to_string(), phase, percent },
    );
}

/// Remote and branch of a state that passed its checks.
fn remote_and_branch(p: &Prepared) -> Result<(String, String), String> {
    match (&p.remote, &p.state.branch) {
        (Some(r), Some(b)) => Ok((r.clone(), b.clone())),
        _ => Err("Este repositório não tem um branch e um remoto do GitHub para enviar.".into()),
    }
}

fn ensure_head(state: &OutgoingState, expected_head: &str) -> Result<(), String> {
    let expected = expected_head.trim();
    if expected.is_empty() || state.head_sha.as_deref() != Some(expected) {
        return Err("Os commits mudaram desde a última verificação. Confira a lista de novo.".into());
    }
    Ok(())
}

fn push_blocking(
    app: &AppHandle,
    binding_id: &str,
    expected_head: &str,
    acknowledge: bool,
    branch: Option<&str>,
) -> Result<PushResult, String> {
    let id = parse_binding(binding_id)?;
    let _guard = BusyGuard::acquire(id)?;
    let bid = id.to_string();
    emit_progress(app, &bid, PushPhase::Validating, None);
    let result = (|| {
        let mut p = prepare(&bid, Fetch::Yes, branch)?;
        if let Some(why) = push_refusal(&p.state, expected_head, acknowledge) {
            return Err(why);
        }
        let (remote, branch) = remote_and_branch(&p)?;
        let current = p.repo.branch().as_deref() == Some(branch.as_str());
        emit_progress(app, &bid, PushPhase::Connecting, None);
        let out = p
            .access
            .git(|t| {
                let progress = |phase, pct| emit_progress(app, &bid, phase, pct);
                if current {
                    outgoing::push(&p.repo, &remote, &branch, t, progress)
                } else {
                    outgoing::push_branch(&p.repo, &remote, &branch, t, progress)
                }
            })
            .map_err(|e| e.message)?;
        emit_progress(app, &bid, PushPhase::Done, Some(100));
        Ok(PushResult {
            pushed: p.state.ahead,
            remote_branch_url: p.github.as_ref().map(|(o, r)| github_branch_url(o, r, &out.remote_branch)),
            branch: out.remote_branch,
            created_upstream: out.created_upstream,
            summary: out.summary,
        })
    })();
    notify_changed(app, &bid);
    result
}

/// Name and email the bound account commits with.
pub(crate) fn commit_identity(account: Option<&Account>) -> Result<Identity, String> {
    let account =
        account.ok_or_else(|| "A conta deste repositório não existe mais. Troque a conta do repositório.".to_string())?;
    let email = account
        .email
        .clone()
        .filter(|e| !e.trim().is_empty())
        .ok_or_else(|| "A conta não tem um email de commit. Escolha um email em «Contas» e tente de novo.".to_string())?;
    let name = account
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| account.github_login.clone())
        .ok_or_else(|| "A conta não tem um nome de commit. Atualize a conta em «Contas» e tente de novo.".to_string())?;
    Ok(Identity { name, email })
}

fn fix_authors_blocking(app: &AppHandle, binding_id: &str, expected_head: &str) -> Result<OutgoingState, String> {
    let id = parse_binding(binding_id)?;
    let _guard = BusyGuard::acquire(id)?;
    let p = prepare(binding_id, Fetch::No, None)?;
    ensure_head(&p.state, expected_head)?;
    if !p.state.can_fix_authors {
        return Err(
            "Não dá para corrigir os autores agora: é preciso estar em um branch, sem alterações pendentes, sem commits de merge e com commits para enviar."
                .into(),
        );
    }
    let who = commit_identity(p.account.as_ref())?;
    let (remote, branch) = remote_and_branch(&p)?;
    let done = outgoing::rewrite_authors(&p.repo, &remote, &branch, &who);
    notify_changed(app, &p.binding_id);
    done?;
    Ok(prepare(binding_id, Fetch::No, None)?.state)
}

fn pull_blocking(app: &AppHandle, binding_id: &str) -> Result<OutgoingState, String> {
    let id = parse_binding(binding_id)?;
    let _guard = BusyGuard::acquire(id)?;
    let mut p = prepare(binding_id, Fetch::Yes, None)?;
    if !p.state.can_pull {
        return Err(if p.state.behind == 0 {
            "Não há commits novos no GitHub para trazer.".to_string()
        } else {
            "Não dá para trazer os commits agora: é preciso estar em um branch ligado ao GitHub e sem alterações pendentes."
                .to_string()
        });
    }
    let (remote, branch) = remote_and_branch(&p)?;
    let done = p.access.git(|t| outgoing::pull_rebase(&p.repo, &remote, &branch, t));
    notify_changed(app, &p.binding_id);
    done.map_err(|e| e.message)?;
    Ok(prepare(binding_id, Fetch::Done, None)?.state)
}

// --- Commands ---

/// Local commits not yet on GitHub and their checks. With `fetch`, updates from GitHub first.
/// `branch = None` checks the current branch; a name checks that local branch without switching to it.
#[tauri::command]
pub async fn repos_outgoing(binding_id: String, fetch: bool, branch: Option<String>) -> Result<OutgoingState, String> {
    blocking(move || {
        let fetch = if fetch { Fetch::Yes } else { Fetch::No };
        prepare(&binding_id, fetch, branch.as_deref()).map(|p| p.state)
    })
    .await
}

/// Files changed by one commit of a bound repository.
#[tauri::command]
pub async fn repos_commit_files(binding_id: String, sha: String) -> Result<Vec<CommitFile>, String> {
    blocking(move || bound(&binding_id)?.repo.commit_files(&sha)).await
}

/// Validates again (with fetch) and pushes the branch (`None`: the current one) to GitHub with the bound account.
#[tauri::command]
pub async fn repos_push(
    app: AppHandle,
    binding_id: String,
    expected_head: String,
    acknowledge_warnings: bool,
    branch: Option<String>,
) -> Result<PushResult, String> {
    blocking(move || push_blocking(&app, &binding_id, &expected_head, acknowledge_warnings, branch.as_deref())).await
}

/// Rewrites author and committer of the unpushed commits with the bound account's identity.
#[tauri::command]
pub async fn repos_fix_authors(app: AppHandle, binding_id: String, expected_head: String) -> Result<OutgoingState, String> {
    blocking(move || fix_authors_blocking(&app, &binding_id, &expected_head)).await
}

/// `git pull --rebase` with the bound account; aborts and restores on conflict.
#[tauri::command]
pub async fn repos_pull(app: AppHandle, binding_id: String) -> Result<OutgoingState, String> {
    blocking(move || pull_blocking(&app, &binding_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_operation_per_repository() {
        let id = Uuid::new_v4();
        let first = BusyGuard::acquire(id).unwrap();
        assert!(BusyGuard::acquire(id).err().unwrap().contains("em andamento"));
        assert!(BusyGuard::acquire(Uuid::new_v4()).is_ok());
        drop(first);
        assert!(BusyGuard::acquire(id).is_ok());
    }

    #[test]
    fn permission_cache_is_per_account_and_repo() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(cached_permission(a, "Org", "Repo"), None);
        remember_permission(a, "Org", "Repo", true);
        assert_eq!(cached_permission(a, "org", "repo"), Some(true));
        assert_eq!(cached_permission(b, "org", "repo"), None);
        assert_eq!(cached_permission(a, "org", "other"), None);
        let stale = Cached { at: Instant::now() - PERMISSION_TTL - Duration::from_secs(1), value: true };
        assert_eq!(stale.fresh(PERMISSION_TTL), None);
    }

    fn account(email: Option<&str>) -> Account {
        Account {
            id: Uuid::new_v4(),
            github_login: Some("ana".into()),
            label: None,
            auth: Default::default(),
            name: None,
            email: email.map(String::from),
            avatar_url: None,
        }
    }

    #[test]
    fn identity_rules_combine_config_and_github() {
        let a = account(Some("123+ana@users.noreply.github.com"));
        let offline = identity_rules(Some(&a), None);
        assert_eq!(offline.user_id, Some(123));
        assert!(offline.accepts("123+ana@users.noreply.github.com"));
        assert!(!offline.accepts("999+ana@users.noreply.github.com"));

        let online = identity_rules(
            Some(&account(Some("ana@work.com"))),
            Some(Person { login: "ana".into(), user_id: 7, emails: vec!["ana@home.com".into()] }),
        );
        assert!(online.accepts("ANA@work.com") && online.accepts("ana@home.com"));
        assert!(online.accepts("7+ana@users.noreply.github.com") && online.accepts("ana@users.noreply.github.com"));
        assert!(!online.accepts("bob@work.com"));
        assert!(identity_rules(None, None).is_empty());
    }

    #[test]
    fn unknown_or_malformed_bindings_are_refused() {
        assert!(bound("not-a-uuid").err().unwrap().contains("inválido"));
    }
}
