//! Git credential helper for `https://github.com`, installed by the GitPass app after a reset
//! (`credential.https://github.com.helper = ""`), so it is the only helper Git asks for github.com.
//!
//! - Repository bound in GitPass with a usable token → that account's login and token (refreshed
//!   first when it expired or GitHub rejected it).
//! - Bound, but no usable token → `quit=1`: Git neither prompts nor falls back to another account.
//! - Not bound → the request goes to the helpers GitPass replaced for github.com, then to the generic
//!   `credential.helper` list (e.g. `osxkeychain`), as Git would have done without GitPass.
//!
//! Binding lookup: the work tree of the current directory against `[[repos]].root_path`, else the
//! credential `path` (`owner/repo`, sent because of `useHttpPath`) against `github_owner`/`github_repo`.
//! Tokens only ever go to stdout (to Git) or to a delegated helper's stdin.

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use gitpass_core::github_user::{check_token, TokenCheck};
use gitpass_core::integration::{self, IntegrationEnv};
use gitpass_core::{
    app_support_dir, embedded_oauth_client_id, git_work_tree_root, github_oauth_env_credentials,
    github_oauth_refresh, normalize_owner, resolve_oauth_client_id, Account, AppConfig,
    GithubTokenBundle, RepoBinding,
};
use uuid::Uuid;

/// Set for delegated helpers: a helper chain that loops back into GitPass stops there.
const DELEGATE_ENV: &str = "GITPASS_CREDENTIAL_DELEGATE";
/// Keys a delegated helper may hand back to Git.
const RESPONSE_KEYS: &[&str] = &["username", "password", "password_expiry_utc", "oauth_refresh_token"];
/// Input keys never forwarded: without capabilities, helpers answer with a plain username/password.
const DROPPED_INPUT_KEYS: &[&str] = &["capability[]", "state[]"];
/// Refresh this long before `expires_at` so the token does not expire mid-push.
const EXPIRY_MARGIN_SECS: i64 = 60;

type Fields = Vec<(String, String)>;

fn main() {
    let op = std::env::args().nth(1).unwrap_or_default();
    let input = read_fields(io::stdin().lock());
    if std::env::var_os(DELEGATE_ENV).is_some() {
        return;
    }
    match (op.as_str(), resolve(&input)) {
        ("get", Target::Bound(acc)) => get_bound(acc),
        ("get", Target::Unbound) => get_delegated(&input),
        ("erase", Target::Bound(acc)) => erase_bound(acc, &input),
        ("store" | "erase", Target::Unbound) => {
            for d in delegates() {
                run_delegate(&d, &op, &input);
            }
        }
        _ => {}
    }
}

// --- Protocol ---

fn read_fields(r: impl Read) -> Fields {
    let mut out = Vec::new();
    for line in io::BufReader::new(r).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((k.to_string(), v.to_string()));
        }
    }
    out
}

fn field<'a>(f: &'a Fields, key: &str) -> Option<&'a str> {
    f.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn is_truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn quit() {
    println!("quit=1");
}

// --- Binding lookup ---

enum Target {
    /// `Err(id)`: the binding points at an account that no longer exists.
    Bound(Result<Account, Uuid>),
    Unbound,
}

fn resolve(input: &Fields) -> Target {
    let github_https = field(input, "protocol") == Some("https")
        && field(input, "host").is_some_and(|h| h.eq_ignore_ascii_case("github.com"));
    if !github_https {
        return Target::Unbound;
    }
    let cfg = match AppConfig::load_or_default() {
        Ok(c) => c,
        Err(_) => {
            eprintln!("GitPass: não foi possível ler as configurações do GitPass; usando as credenciais anteriores. Abra o app GitPass.");
            return Target::Unbound;
        }
    };
    if cfg.repos.is_empty() {
        return Target::Unbound;
    }
    let binding = binding_for_cwd(&cfg).or_else(|| field(input, "path").and_then(|p| binding_for_url_path(&cfg, p)));
    match binding {
        Some(b) => Target::Bound(
            cfg.accounts
                .iter()
                .find(|a| a.id == b.account_id)
                .cloned()
                .ok_or(b.account_id),
        ),
        None => Target::Unbound,
    }
}

/// Git runs helpers from the repository it is working on (also with `git -C`), so the current
/// directory is authoritative; `PWD` may still be the caller's shell directory.
fn binding_for_cwd(cfg: &AppConfig) -> Option<&RepoBinding> {
    let cwd = std::env::current_dir()
        .ok()
        .or_else(|| std::env::var_os("GIT_WORK_TREE").map(PathBuf::from))?;
    let root = git_work_tree_root(Some(&cwd)).ok()?;
    let root = root.canonicalize().unwrap_or(root);
    cfg.repos
        .iter()
        .find(|r| r.root_path.canonicalize().unwrap_or_else(|_| r.root_path.clone()) == root)
}

fn parse_owner_repo(path: &str) -> Option<(String, String)> {
    let path = path.trim().trim_matches('/');
    let (owner, repo) = path.split_once('/')?;
    let repo = repo.split('/').next().unwrap_or("").trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((normalize_owner(owner), repo.to_string()))
}

fn binding_for_url_path<'a>(cfg: &'a AppConfig, path: &str) -> Option<&'a RepoBinding> {
    let (owner, repo) = parse_owner_repo(path)?;
    cfg.repos.iter().find(|r| {
        r.github_owner.as_deref().is_some_and(|o| normalize_owner(o) == owner)
            && r.github_repo
                .as_deref()
                .is_some_and(|n| n.trim_end_matches(".git").eq_ignore_ascii_case(&repo))
    })
}

// --- Bound repositories ---

fn account_handle(acc: &Account) -> String {
    match acc.github_login.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(login) => format!("@{login}"),
        None => format!("«{}»", acc.label.as_deref().unwrap_or("sem nome")),
    }
}

fn needs_reconnect(acc: &Account) {
    eprintln!(
        "GitPass: a conta {} precisa ser conectada de novo. Abra o app GitPass.",
        account_handle(acc)
    );
    quit();
}

fn get_bound(acc: Result<Account, Uuid>) {
    let acc = match acc {
        Ok(a) => a,
        Err(_) => {
            eprintln!("GitPass: este repositório está vinculado a uma conta que não existe mais. Escolha outra conta no app GitPass (Repositórios).");
            return quit();
        }
    };
    let Some(login) = acc.github_login.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return needs_reconnect(&acc);
    };
    let bundle = match tokens::load(acc.id) {
        Ok(Some(b)) => b,
        Ok(None) => return needs_reconnect(&acc),
        Err(()) => {
            eprintln!(
                "GitPass: não foi possível ler o acesso da conta {} nas Chaves do macOS. Abra o app GitPass e, se o macOS pedir, permita o acesso.",
                account_handle(&acc)
            );
            return quit();
        }
    };
    match usable_token(acc.id, bundle) {
        Some(b) => {
            let mut out = io::stdout().lock();
            let _ = writeln!(out, "username={login}");
            let _ = writeln!(out, "password={}", b.access_token);
        }
        None => needs_reconnect(&acc),
    }
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The stored token, or a refreshed one when it expired or GitHub rejected it.
fn usable_token(account_id: Uuid, bundle: GithubTokenBundle) -> Option<GithubTokenBundle> {
    if bundle.access_token.trim().is_empty() {
        return None;
    }
    let app_support = app_support_dir().ok();
    let rejected = app_support
        .as_deref()
        .is_some_and(|d| integration::is_token_rejected(d, account_id, &bundle.access_token));
    let expired = bundle
        .expires_at_epoch
        .is_some_and(|t| t <= now_epoch() + EXPIRY_MARGIN_SECS);
    if !rejected && !expired {
        return Some(bundle);
    }
    let fresh = refresh(&bundle)?;
    let _ = tokens::save(account_id, &fresh);
    if let Some(d) = app_support.as_deref() {
        integration::clear_token_rejected(d, account_id);
    }
    Some(fresh)
}

fn refresh(bundle: &GithubTokenBundle) -> Option<GithubTokenBundle> {
    let refresh_token = bundle.refresh_token.as_deref().filter(|s| !s.is_empty())?;
    if bundle.oauth_via_device {
        let client_id = bundle
            .oauth_client_id
            .clone()
            .or_else(|| embedded_oauth_client_id().map(String::from))
            .or_else(|| resolve_oauth_client_id().ok())?;
        github_oauth_refresh(&client_id, None, refresh_token, true).ok()
    } else {
        let (id, secret) = github_oauth_env_credentials().ok()?;
        github_oauth_refresh(&id, Some(&secret), refresh_token, false).ok()
    }
}

/// Git sends `erase` after GitHub answered 401 to these credentials. When they are this account's
/// token and GitHub confirms it is no longer valid, refresh it or remember the rejection, so the next
/// `get` answers `quit=1` with a clear message instead of failing the same way again.
fn erase_bound(acc: Result<Account, Uuid>, input: &Fields) {
    let (Ok(acc), Some(password)) = (acc, field(input, "password")) else {
        return;
    };
    let Ok(Some(bundle)) = tokens::load(acc.id) else {
        return;
    };
    if bundle.access_token != password || !matches!(check_token(password), Ok(TokenCheck::Revoked)) {
        return;
    }
    let Ok(app_support) = app_support_dir() else {
        return;
    };
    match refresh(&bundle) {
        Some(fresh) => {
            let _ = tokens::save(acc.id, &fresh);
            integration::clear_token_rejected(&app_support, acc.id);
        }
        None => {
            let _ = integration::mark_token_rejected(&app_support, acc.id, password);
        }
    }
}

mod tokens {
    use super::*;

    /// Debug builds only (integration tests): read tokens from `<dir>/<account id>.json` instead of
    /// the Keychain. Release builds always use the Keychain.
    #[cfg(debug_assertions)]
    const TEST_TOKENS_DIR_ENV: &str = "GITPASS_TEST_TOKENS_DIR";

    #[cfg(debug_assertions)]
    fn test_file(id: Uuid) -> Option<PathBuf> {
        std::env::var_os(TEST_TOKENS_DIR_ENV).map(|d| PathBuf::from(d).join(format!("{id}.json")))
    }

    pub fn load(id: Uuid) -> Result<Option<GithubTokenBundle>, ()> {
        #[cfg(debug_assertions)]
        if let Some(path) = test_file(id) {
            return match std::fs::read_to_string(path) {
                Ok(s) => serde_json::from_str(&s).map(Some).map_err(|_| ()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(_) => Err(()),
            };
        }
        gitpass_core::github_token_load(id).map_err(|_| ())
    }

    pub fn save(id: Uuid, bundle: &GithubTokenBundle) -> Result<(), ()> {
        #[cfg(debug_assertions)]
        if let Some(path) = test_file(id) {
            let json = serde_json::to_string(bundle).map_err(|_| ())?;
            return std::fs::write(path, json).map_err(|_| ());
        }
        gitpass_core::github_token_save(id, bundle).map_err(|_| ())
    }
}

// --- Unbound repositories: delegate to the previous helpers ---

#[derive(Debug, Clone, PartialEq, Eq)]
struct Delegate {
    helper: String,
    /// Forward `path=`: only when the helper was configured with `useHttpPath`.
    with_path: bool,
}

/// Git's list semantics: an empty value clears the helpers configured before it.
fn effective_helpers(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        if v.trim().is_empty() {
            out.clear();
        } else {
            out.push(v.clone());
        }
    }
    out
}

/// Previous github.com helpers first, then the generic ones; GitPass itself and duplicates removed.
fn delegate_list(previous: &[String], previous_path: bool, generic: &[String], generic_path: bool) -> Vec<Delegate> {
    let mut out: Vec<Delegate> = Vec::new();
    let candidates = effective_helpers(previous)
        .into_iter()
        .map(|h| (h, previous_path))
        .chain(effective_helpers(generic).into_iter().map(|h| (h, generic_path)));
    for (helper, with_path) in candidates {
        if integration::is_gitpass_helper(&helper) || out.iter().any(|d| d.helper == helper) {
            continue;
        }
        out.push(Delegate { helper, with_path });
    }
    out
}

fn git_bin() -> PathBuf {
    integration::find_git().unwrap_or_else(|| PathBuf::from("git"))
}

fn git_config_all(git: &Path, key: &str) -> Vec<String> {
    let out = Command::new(git)
        .args(["config", "-z", "--get-all", key])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            let mut vals: Vec<String> = s.split('\0').map(String::from).collect();
            vals.pop();
            vals
        }
        _ => Vec::new(),
    }
}

fn delegates() -> Vec<Delegate> {
    let (previous, previous_path) = IntegrationEnv::system(None)
        .ok()
        .and_then(|env| integration::load_backup(&env).ok().flatten())
        .and_then(|b| b.credential)
        .map(|c| (c.github_helpers, c.use_http_path.as_deref().is_some_and(is_truthy)))
        .unwrap_or_default();
    let git = git_bin();
    let generic = git_config_all(&git, "credential.helper");
    let generic_path = git_config_all(&git, "credential.useHttpPath")
        .last()
        .is_some_and(|v| is_truthy(v));
    delegate_list(&previous, previous_path, &generic, generic_path)
}

/// Shell command Git itself would run for `helper` (see `credential.helper` in git-config(1)).
fn helper_command(helper: &str, op: &str, git: &Path) -> Option<String> {
    let h = helper.trim();
    if h.is_empty() {
        None
    } else if let Some(cmd) = h.strip_prefix('!') {
        Some(format!("{cmd} {op}"))
    } else if h.starts_with('/') {
        Some(format!("{h} {op}"))
    } else {
        Some(format!("{} credential-{h} {op}", integration::sh_quote(&git.to_string_lossy())))
    }
}

fn forwarded_input(input: &Fields, with_path: bool) -> String {
    let mut s = String::new();
    for (k, v) in input {
        if DROPPED_INPUT_KEYS.contains(&k.as_str()) || (k == "path" && !with_path) {
            continue;
        }
        s.push_str(&format!("{k}={v}\n"));
    }
    s.push('\n');
    s
}

fn run_delegate(d: &Delegate, op: &str, input: &Fields) -> Option<Fields> {
    let cmd = helper_command(&d.helper, op, &git_bin())?;
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&cmd)
        .env(DELEGATE_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        // A helper may exit without reading its input (broken pipe); its answer still counts.
        let _ = stdin.write_all(forwarded_input(input, d.with_path).as_bytes());
    }
    let out = child.wait_with_output().ok()?;
    Some(read_fields(out.stdout.as_slice()))
}

fn get_delegated(input: &Fields) {
    for d in delegates() {
        let Some(resp) = run_delegate(&d, "get", input) else {
            continue;
        };
        if field(&resp, "password").is_some() {
            let mut out = io::stdout().lock();
            for (k, v) in resp.iter().filter(|(k, _)| RESPONSE_KEYS.contains(&k.as_str())) {
                let _ = writeln!(out, "{k}={v}");
            }
            return;
        }
        if field(&resp, "quit").is_some_and(is_truthy) {
            return quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_value_resets_the_helper_list() {
        assert_eq!(effective_helpers(&v(&["osxkeychain", "", "!gh auth git-credential"])), v(&["!gh auth git-credential"]));
        assert_eq!(effective_helpers(&v(&["osxkeychain", ""])), Vec::<String>::new());
        assert_eq!(effective_helpers(&v(&["a", "b"])), v(&["a", "b"]));
    }

    #[test]
    fn delegates_skip_gitpass_and_duplicates_and_keep_order() {
        let list = delegate_list(
            &v(&["", "!gh auth git-credential"]),
            true,
            &v(&["osxkeychain", "!'/x/bin/git-credential-gitpass'", "!gh auth git-credential"]),
            false,
        );
        assert_eq!(
            list,
            vec![
                Delegate { helper: "!gh auth git-credential".into(), with_path: true },
                Delegate { helper: "osxkeychain".into(), with_path: false },
            ]
        );
    }

    #[test]
    fn helper_commands_follow_git_rules() {
        let git = Path::new("/opt/homebrew/bin/git");
        assert_eq!(helper_command("osxkeychain", "get", git).unwrap(), "'/opt/homebrew/bin/git' credential-osxkeychain get");
        assert_eq!(
            helper_command("cache --timeout=300", "store", git).unwrap(),
            "'/opt/homebrew/bin/git' credential-cache --timeout=300 store"
        );
        assert_eq!(helper_command("!gh auth git-credential", "get", git).unwrap(), "gh auth git-credential get");
        assert_eq!(helper_command("/usr/local/bin/helper --x", "erase", git).unwrap(), "/usr/local/bin/helper --x erase");
        assert_eq!(helper_command("  ", "get", git), None);
    }

    #[test]
    fn forwarded_input_drops_path_and_capabilities() {
        let input: Fields = vec![
            ("capability[]".into(), "authtype".into()),
            ("protocol".into(), "https".into()),
            ("host".into(), "github.com".into()),
            ("path".into(), "o/r.git".into()),
        ];
        assert_eq!(forwarded_input(&input, false), "protocol=https\nhost=github.com\n\n");
        assert_eq!(forwarded_input(&input, true), "protocol=https\nhost=github.com\npath=o/r.git\n\n");
    }

    #[test]
    fn owner_repo_from_credential_path() {
        assert_eq!(parse_owner_repo("Octo/Repo.git"), Some(("octo".into(), "Repo".into())));
        assert_eq!(parse_owner_repo("/octo/repo/info/lfs"), Some(("octo".into(), "repo".into())));
        assert_eq!(parse_owner_repo("octo"), None);
    }
}
