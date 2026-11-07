//! `gitpass hook` with `gitpassd` off (DL-015/016/017), driven through real `git commit` / `git push`
//! with the hook scripts the integration installs: emergency mode lets local commits through, pushes
//! stay blocked, CI bypass needs `GITPASS_ALLOW_CI=1`, and every verdict reaches `recent.json` and
//! the audit log. Isolated: temporary `HOME` (Application Support), `GIT_CONFIG_GLOBAL`, no daemon.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use gitpass_core::audit::{self, AuditKind};
use gitpass_core::recent::{self, Verdict};
use gitpass_core::{emergency, integration, Account, AppConfig, AuthMode, RepoBinding};
use uuid::Uuid;

const GITPASS: &str = env!("CARGO_BIN_EXE_gitpass");

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    app_support: PathBuf,
    global: PathBuf,
    repo: PathBuf,
    bare: PathBuf,
    binding_id: Uuid,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new(emergency_on: bool) -> Self {
        let root = std::env::temp_dir().join(format!("gitpass-hook-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("home");
        let app_support = home.join("Library/Application Support/com.gitpass.GitPass");
        let f = Fixture {
            global: root.join("global.gitconfig"),
            repo: root.join("app"),
            bare: root.join("remote.git"),
            binding_id: Uuid::new_v4(),
            home,
            app_support,
            root,
        };
        fs::create_dir_all(&f.app_support).unwrap();
        fs::write(&f.global, "").unwrap();

        // Hook scripts exactly as `integration::apply` writes them, pointing at this build.
        let hooks = f.root.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        for (name, body) in integration::hook_scripts(Path::new(GITPASS), &f.app_support.join("config.toml"), None) {
            fs::write(hooks.join(&name), body).unwrap();
            fs::set_permissions(hooks.join(&name), fs::Permissions::from_mode(0o755)).unwrap();
        }
        f.git_ok(&f.root, &["config", "--global", "core.hooksPath", &hooks.display().to_string()]);
        f.git_ok(&f.root, &["config", "--global", "user.name", "Octo"]);
        f.git_ok(&f.root, &["config", "--global", "user.email", "octo@example.com"]);

        fs::create_dir_all(&f.repo).unwrap();
        f.git_ok(&f.root, &["init", "-q", "--bare", &f.bare.display().to_string()]);
        f.git_ok(&f.repo, &["init", "-q", "-b", "main"]);
        f.git_ok(&f.repo, &["remote", "add", "origin", "https://github.com/octo/app.git"]);
        // Pushes really go to a local bare repository; GitPass checks the `origin` URL.
        f.git_ok(&f.repo, &["config", "remote.origin.pushurl", &f.bare.display().to_string()]);

        let account_id = Uuid::new_v4();
        let mut cfg = AppConfig {
            accounts: vec![Account { id: account_id, github_login: Some("octo".into()), auth: AuthMode::Https, ..Default::default() }],
            repos: vec![RepoBinding {
                id: f.binding_id,
                account_id,
                root_path: f.repo.clone(),
                github_owner: Some("octo".into()),
                github_repo: Some("app".into()),
            }],
            ..Default::default()
        };
        if emergency_on {
            emergency::start(&mut cfg, 15, "serviço fora do ar", "cli", chrono_now()).unwrap();
        }
        cfg.save_to_path(&f.app_support.join("config.toml")).unwrap();
        f
    }

    fn cmd(&self, program: &str, dir: &Path) -> Command {
        let mut c = Command::new(program);
        c.current_dir(dir)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GITPASS_TEST_IPC_CONNECT_WAIT_MS", "50")
            .stdin(Stdio::null());
        for k in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "CI",
            "GITHUB_ACTIONS",
            "GITPASS_ALLOW_CI",
            "GITPASS_ALLOW_LOCAL_COMMIT",
            "GITPASS_ALLOW_PUSH_WITHOUT_DAEMON",
            "GITPASS_QUIET_HOOK",
        ] {
            c.env_remove(k);
        }
        c
    }

    fn git(&self, dir: &Path, args: &[&str]) -> Output {
        self.cmd("git", dir).args(args).output().unwrap()
    }

    fn git_ok(&self, dir: &Path, args: &[&str]) {
        let o = self.git(dir, args);
        assert!(o.status.success(), "git {args:?}: {}", err(&o));
    }

    fn commit(&self, file: &str) -> Output {
        fs::write(self.repo.join(file), file).unwrap();
        self.git_ok(&self.repo, &["add", file]);
        self.git(&self.repo, &["commit", "-q", "-m", file])
    }

    fn commits(&self) -> usize {
        let o = self.git(&self.repo, &["rev-list", "--count", "HEAD"]);
        String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0)
    }

    fn audit(&self) -> Vec<audit::AuditEntry> {
        audit::read_in(&self.app_support, 50)
    }
}

fn chrono_now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn emergency_allows_local_commits_but_push_stays_blocked() {
    let f = Fixture::new(true);
    let o = f.commit("a.txt");
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("modo emergência"), "{}", err(&o));
    assert_eq!(f.commits(), 1);

    // pre-commit, prepare-commit-msg and commit-msg each ran, but one commit is one audit entry.
    let allowed: Vec<_> = f.audit().into_iter().filter(|e| e.kind == AuditKind::CommitAllowedEmergency).collect();
    assert_eq!(allowed.len(), 1, "{allowed:?}");
    assert_eq!(allowed[0].repo.as_deref(), Some("octo/app"));
    let r = &recent::load_in(&f.app_support)[0];
    assert_eq!((r.verdict, r.binding_id, r.full_name.as_deref()), (Verdict::Ok, Some(f.binding_id), Some("octo/app")));
    assert_eq!(r.root_path, f.repo);

    let o = f.git(&f.repo, &["push", "-q", "origin", "main"]);
    assert!(!o.status.success(), "push must stay blocked: {}", err(&o));
    assert!(err(&o).contains("GP-DAEMON-OFF") && err(&o).contains("o push continua bloqueado"), "{}", err(&o));
    let refs = f.git(&f.bare, &["for-each-ref"]);
    assert!(String::from_utf8_lossy(&refs.stdout).trim().is_empty(), "nothing reached the remote");

    let newest = &f.audit()[0];
    assert_eq!((newest.kind, newest.code.as_deref()), (AuditKind::Blocked, Some("GP-DAEMON-OFF")));
    let r = &recent::load_in(&f.app_support)[0];
    assert_eq!((r.verdict, r.code.as_deref()), (Verdict::Blocked, Some("GP-DAEMON-OFF")));
}

#[test]
fn without_emergency_commits_are_blocked_when_the_daemon_is_off() {
    let f = Fixture::new(false);
    let o = f.commit("a.txt");
    assert!(!o.status.success());
    assert!(err(&o).contains("GitPass [GP-DAEMON-OFF]") && err(&o).contains("o commit foi bloqueado"), "{}", err(&o));
    assert_eq!(f.commits(), 0);
    let kinds: Vec<_> = f.audit().into_iter().map(|e| (e.kind, e.code)).collect();
    assert_eq!(kinds, vec![(AuditKind::Blocked, Some("GP-DAEMON-OFF".into()))]);
    assert_eq!(recent::load_in(&f.app_support)[0].verdict, Verdict::Blocked);
}

#[test]
fn expired_emergency_no_longer_allows_commits() {
    let f = Fixture::new(false);
    let path = f.app_support.join("config.toml");
    let mut cfg: AppConfig = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    emergency::start(&mut cfg, 5, "teste", "cli", chrono_now() - chrono::Duration::minutes(6)).unwrap();
    cfg.save_to_path(&path).unwrap();
    assert!(!f.commit("a.txt").status.success());
}

#[test]
fn ci_bypass_needs_ci_and_the_opt_in_and_is_audited() {
    let f = Fixture::new(false);
    fs::write(f.repo.join("a.txt"), "a").unwrap();
    f.git_ok(&f.repo, &["add", "a.txt"]);

    let only_ci = f.cmd("git", &f.repo).env("CI", "true").args(["commit", "-q", "-m", "a"]).output().unwrap();
    assert!(!only_ci.status.success(), "CI alone is not enough");

    let o = f
        .cmd("git", &f.repo)
        .env("CI", "true")
        .env("GITPASS_ALLOW_CI", "1")
        .args(["commit", "-q", "-m", "a"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let bypass: Vec<_> = f.audit().into_iter().filter(|e| e.kind == AuditKind::CiBypass).collect();
    assert_eq!(bypass.len(), 1, "{bypass:?}");
    assert_eq!(bypass[0].repo.as_deref(), Some("octo/app"));
}

#[test]
fn emergency_command_turns_it_on_and_off_with_audit() {
    let f = Fixture::new(false);
    let run = |args: &[&str]| f.cmd(GITPASS, &f.root).args(args).output().unwrap();
    let bad = run(&["emergency", "on", "--minutes", "2", "--reason", "teste"]);
    assert!(!bad.status.success());
    assert!(err(&bad).contains("Escolha entre 5 minutos e 4 horas"), "{}", err(&bad));
    let on = run(&["emergency", "on", "--minutes", "15", "--reason", "serviço caiu"]);
    assert!(on.status.success(), "{}", err(&on));
    assert!(String::from_utf8_lossy(&on.stdout).contains("Modo emergência ativo"));
    assert!(f.commit("a.txt").status.success());
    let off = run(&["emergency", "off"]);
    assert!(off.status.success());
    assert!(!f.commit("b.txt").status.success());
    let kinds: Vec<_> = f.audit().into_iter().rev().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [AuditKind::EmergencyOn, AuditKind::CommitAllowedEmergency, AuditKind::EmergencyOff, AuditKind::Blocked]
    );
}
