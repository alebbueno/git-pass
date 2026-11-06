//! `git credential fill/approve/reject` driving the real `git-credential-gitpass`, configured as the
//! GitPass app does it. Everything is isolated: `HOME` (Application Support, `config.toml`),
//! `GIT_CONFIG_GLOBAL`, no system config, and tokens read from a temporary directory (debug builds).

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use gitpass_core::integration::{self, CredentialBackup, IntegrationBackup};
use gitpass_core::{Account, AppConfig, AuthMode, GithubTokenBundle, RepoBinding};
use uuid::Uuid;

const HELPER: &str = env!("CARGO_BIN_EXE_git-credential-gitpass");
const TOKEN: &str = "gho_test_token_not_real";

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    app_support: PathBuf,
    global: PathBuf,
    tokens: PathBuf,
    helpers: PathBuf,
    bound: PathBuf,
    stale: PathBuf,
    unbound: PathBuf,
    octo_id: Uuid,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_exec(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Logs the operation and its stdin; answers `get` unless `<name>-silent` exists next to it.
fn fake_helper(name: &str) -> String {
    format!(
        "#!/bin/sh\nd=$(dirname \"$0\")\necho \"op=$1\" >> \"$d/{name}.log\"\ncat >> \"$d/{name}.log\"\n\
         [ \"$1\" = get ] || exit 0\n[ -f \"$d/{name}-silent\" ] && exit 0\n\
         echo username={name}-user\necho password={name}-pass\n"
    )
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("gitpass-cred-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("home");
        let app_support = home.join("Library/Application Support/com.gitpass.GitPass");
        let f = Fixture {
            global: root.join("global.gitconfig"),
            tokens: root.join("tokens"),
            helpers: root.join("fake helpers"),
            bound: root.join("bound"),
            stale: root.join("stale"),
            unbound: root.join("unbound"),
            octo_id: Uuid::new_v4(),
            home,
            app_support,
            root,
        };
        fs::create_dir_all(&f.app_support).unwrap();
        fs::create_dir_all(&f.tokens).unwrap();

        let previous = f.helpers.join("previous");
        let generic = f.helpers.join("generic");
        write_exec(&previous, &fake_helper("previous"));
        write_exec(&generic, &fake_helper("generic"));

        // Global config written by `integration::apply`, plus a generic helper (and GitPass itself,
        // which must never be delegated to).
        let gitpass_value = integration::credential_helper_value(Path::new(HELPER));
        f.git_ok(&f.root, &["config", "--global", "--add", "credential.helper", &integration::credential_helper_value(&generic)]);
        f.git_ok(&f.root, &["config", "--global", "--add", "credential.helper", &gitpass_value]);
        f.git_ok(&f.root, &["config", "--global", "--add", integration::CREDENTIAL_HELPER_KEY, ""]);
        f.git_ok(&f.root, &["config", "--global", "--add", integration::CREDENTIAL_HELPER_KEY, &gitpass_value]);
        f.git_ok(&f.root, &["config", "--global", integration::CREDENTIAL_USE_HTTP_PATH_KEY, "true"]);

        // What the integration backed up: the github.com helpers it replaced.
        let backup = IntegrationBackup {
            version: 1,
            credential: Some(CredentialBackup {
                github_helpers: vec![String::new(), integration::credential_helper_value(&previous)],
                use_http_path: None,
            }),
            ..Default::default()
        };
        fs::write(f.app_support.join(integration::BACKUP_FILE_NAME), serde_json::to_vec(&backup).unwrap()).unwrap();

        for (dir, origin) in [
            (&f.bound, "https://github.com/octo/bound.git"),
            (&f.stale, "https://github.com/stale/old.git"),
            (&f.unbound, "https://github.com/someone/else.git"),
        ] {
            fs::create_dir_all(dir).unwrap();
            f.git_ok(dir, &["init", "-q"]);
            f.git_ok(dir, &["remote", "add", "origin", origin]);
        }

        let stale_id = Uuid::new_v4();
        let account = |id, login: &str| Account {
            id,
            github_login: Some(login.into()),
            auth: AuthMode::Https,
            ..Default::default()
        };
        let binding = |account_id, root: &Path, owner: &str, repo: &str| RepoBinding {
            id: Uuid::new_v4(),
            account_id,
            root_path: root.to_path_buf(),
            github_owner: Some(owner.into()),
            github_repo: Some(repo.into()),
        };
        let cfg = AppConfig {
            accounts: vec![account(f.octo_id, "octo"), account(stale_id, "stale")],
            repos: vec![
                binding(f.octo_id, &f.bound, "octo", "bound"),
                binding(stale_id, &f.stale, "stale", "old"),
            ],
            ..Default::default()
        };
        cfg.save_to_path(&f.app_support.join("config.toml")).unwrap();

        // Only `octo` has a token; `stale` lost its access.
        let bundle = GithubTokenBundle {
            access_token: TOKEN.into(),
            token_type: Some("bearer".into()),
            scope: Some("repo".into()),
            expires_at_epoch: None,
            refresh_token: None,
            oauth_via_device: true,
            oauth_client_id: None,
        };
        fs::write(f.tokens.join(format!("{}.json", f.octo_id)), serde_json::to_vec(&bundle).unwrap()).unwrap();
        f
    }

    fn cmd(&self, program: &str, dir: &Path) -> Command {
        let mut c = Command::new(program);
        c.current_dir(dir)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GITPASS_TEST_TOKENS_DIR", &self.tokens)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .env_remove("GITPASS_CREDENTIAL_DELEGATE");
        c
    }

    fn git_ok(&self, dir: &Path, args: &[&str]) {
        let o = self.cmd("git", dir).args(args).stdin(Stdio::null()).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    fn run(&self, mut c: Command, input: &str) -> Output {
        let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn credential(&self, dir: &Path, op: &str, path: &str, extra: &str) -> Output {
        let mut c = self.cmd("git", dir);
        c.args(["credential", op]);
        self.run(c, &format!("protocol=https\nhost=github.com\npath={path}\n{extra}\n"))
    }

    fn log(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.helpers.join(format!("{name}.log"))).ok()
    }

    fn silence(&self, name: &str) {
        fs::write(self.helpers.join(format!("{name}-silent")), "").unwrap();
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn bound_repo_gets_its_account_token() {
    let f = Fixture::new();
    let o = f.credential(&f.bound, "fill", "octo/bound.git", "");
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("username=octo\n"), "{}", out(&o));
    assert!(out(&o).contains(&format!("password={TOKEN}\n")));
    assert!(!err(&o).contains(TOKEN));
    assert_eq!(f.log("previous"), None);
    assert_eq!(f.log("generic"), None);
}

#[test]
fn clone_outside_a_repo_matches_the_url_path() {
    let f = Fixture::new();
    let o = f.credential(&f.root, "fill", "octo/bound.git", "");
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains(&format!("password={TOKEN}\n")));
    assert_eq!(f.log("previous"), None);
}

#[test]
fn bound_repo_without_token_quits_without_falling_back() {
    let f = Fixture::new();
    let o = f.credential(&f.stale, "fill", "stale/old.git", "");
    assert!(!o.status.success());
    assert!(!out(&o).contains("password="), "{}", out(&o));
    assert!(
        err(&o).contains("GitPass: a conta @stale precisa ser conectada de novo. Abra o app GitPass."),
        "{}",
        err(&o)
    );
    assert_eq!(f.log("previous"), None);
    assert_eq!(f.log("generic"), None);
}

#[test]
fn token_rejected_by_github_quits_until_reconnected() {
    let f = Fixture::new();
    integration::mark_token_rejected(&f.app_support, f.octo_id, TOKEN).unwrap();
    let o = f.credential(&f.bound, "fill", "octo/bound.git", "");
    assert!(!o.status.success());
    assert!(err(&o).contains("a conta @octo precisa ser conectada de novo"), "{}", err(&o));
    assert_eq!(f.log("previous"), None);

    // Reconnecting stores a new token, which no longer matches the rejected one.
    let mut bundle: GithubTokenBundle =
        serde_json::from_slice(&fs::read(f.tokens.join(format!("{}.json", f.octo_id))).unwrap()).unwrap();
    bundle.access_token = "gho_new_token_not_real".into();
    fs::write(f.tokens.join(format!("{}.json", f.octo_id)), serde_json::to_vec(&bundle).unwrap()).unwrap();
    let o = f.credential(&f.bound, "fill", "octo/bound.git", "");
    assert!(out(&o).contains("password=gho_new_token_not_real\n"), "{}", err(&o));
}

#[test]
fn unbound_repo_delegates_to_previous_github_helper_without_path() {
    let f = Fixture::new();
    let o = f.credential(&f.unbound, "fill", "someone/else.git", "");
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("username=previous-user\n"), "{}", out(&o));
    assert!(out(&o).contains("password=previous-pass\n"));
    let log = f.log("previous").unwrap();
    assert!(log.contains("op=get\n") && log.contains("host=github.com\n"), "{log}");
    assert!(!log.contains("path="), "{log}");
    assert!(!log.contains("capability[]"), "{log}");
    assert_eq!(f.log("generic"), None);
}

#[test]
fn unbound_repo_falls_through_to_generic_helper() {
    let f = Fixture::new();
    f.silence("previous");
    let o = f.credential(&f.unbound, "fill", "someone/else.git", "");
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("password=generic-pass\n"), "{}", out(&o));
    assert!(f.log("previous").unwrap().contains("op=get"));
    assert!(f.log("generic").unwrap().contains("op=get"));
}

#[test]
fn unbound_repo_with_no_answer_leaves_git_to_its_normal_flow() {
    let f = Fixture::new();
    f.silence("previous");
    f.silence("generic");
    let mut c = f.cmd(HELPER, &f.unbound);
    c.arg("get");
    let o = f.run(c, "protocol=https\nhost=github.com\npath=someone/else.git\n\n");
    assert!(o.status.success());
    assert_eq!(out(&o), "");
    assert_eq!(err(&o), "");
}

#[test]
fn store_and_erase_are_delegated_only_for_unbound_repos() {
    let f = Fixture::new();
    let creds = "username=someone\npassword=typed-by-user\n";
    assert!(f.credential(&f.unbound, "approve", "someone/else.git", creds).status.success());
    assert!(f.credential(&f.unbound, "reject", "someone/else.git", creds).status.success());
    for name in ["previous", "generic"] {
        let log = f.log(name).unwrap();
        assert!(log.contains("op=store\n") && log.contains("op=erase\n"), "{name}: {log}");
        assert!(log.contains("password=typed-by-user\n"));
    }

    fs::remove_file(f.helpers.join("previous.log")).unwrap();
    fs::remove_file(f.helpers.join("generic.log")).unwrap();
    let bound_creds = "username=octo\npassword=not-the-stored-token\n";
    assert!(f.credential(&f.bound, "approve", "octo/bound.git", bound_creds).status.success());
    assert!(f.credential(&f.bound, "reject", "octo/bound.git", bound_creds).status.success());
    assert_eq!(f.log("previous"), None);
    assert_eq!(f.log("generic"), None);
}

#[test]
fn nested_invocation_from_a_delegated_helper_does_nothing() {
    let f = Fixture::new();
    let mut c = f.cmd(HELPER, &f.bound);
    c.arg("get").env("GITPASS_CREDENTIAL_DELEGATE", "1");
    let o = f.run(c, "protocol=https\nhost=github.com\npath=octo/bound.git\n\n");
    assert_eq!(out(&o), "");
}
