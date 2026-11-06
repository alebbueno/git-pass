//! `gitpass doctor` — environment checks (no daemon required for most checks).

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

use gitpass_core::{
    app_support_dir, config_file_path, hooks_dir, hooks_installed_ok, ipc_socket_path, AppConfig,
    HOOKS_DIR_NAME,
};

#[derive(Default)]
pub struct DoctorReport {
    pub lines: Vec<String>,
    pub errors: usize,
    pub warnings: usize,
}

impl DoctorReport {
    fn ok(&mut self, msg: impl Into<String>) {
        self.lines.push(format!("ok: {}", msg.into()));
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("warn: {}", msg.into()));
    }

    fn err(&mut self, msg: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("error: {}", msg.into()));
    }

    pub fn print_and_status(&self) -> io::Result<i32> {
        let mut out = io::stdout().lock();
        for line in &self.lines {
            writeln!(out, "{line}")?;
        }
        let code = if self.errors > 0 {
            1
        } else if self.warnings > 0 {
            2
        } else {
            0
        };
        Ok(code)
    }
}

/// Path to this `gitpass` executable (used in generated hook scripts).
pub fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gitpass"))
}

pub fn run() -> io::Result<i32> {
    let gitpass_bin = current_exe();
    let mut r = DoctorReport::default();

    // macOS version (sw_vers)
    if cfg!(target_os = "macos") {
        match Command::new("/usr/bin/sw_vers").arg("-productVersion").output() {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout);
                let ver = s.lines().next().unwrap_or("").trim();
                if ver.is_empty() {
                    r.warn("could not read macOS version (sw_vers)");
                } else {
                    r.ok(format!("macOS version {ver}"));
                }
            }
            _ => r.warn("could not run sw_vers — not macOS?"),
        }
    } else {
        r.warn("not running on macOS — GitPass v1 targets macOS 14+");
    }

    // Git on PATH
    match Command::new("git").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            let line = s.lines().next().unwrap_or("git").trim();
            r.ok(line.to_string());
        }
        _ => r.err("git not found on PATH (install Xcode CLT or Git)"),
    }

    // Application Support
    match app_support_dir() {
        Ok(p) => {
            if p.is_dir() {
                r.ok(format!(
                    "application support {}",
                    p.display()
                ));
            } else {
                r.warn(format!(
                    "application support missing (will be created): {}",
                    p.display()
                ));
            }
        }
        Err(e) => r.err(format!("application support path: {e}")),
    }

    // config.toml
    match config_file_path() {
        Ok(p) => match AppConfig::load_or_default() {
            Ok(cfg) => {
                if p.exists() {
                    r.ok(format!(
                        "{} ({} accounts, {} repos)",
                        p.display(),
                        cfg.accounts.len(),
                        cfg.repos.len()
                    ));
                } else {
                    r.warn(format!("{} — not created yet (run gitpassd once)", p.display()));
                }
            }
            Err(e) => r.err(format!("{} invalid: {e}", p.display())),
        },
        Err(e) => r.err(format!("config path: {e}")),
    }

    // hooks directory + files
    match hooks_installed_ok(&gitpass_bin) {
        Ok(true) => {
            if let Ok(h) = hooks_dir() {
                r.ok(format!("hooks installed and up to date under {}", h.display()));
            }
        }
        Ok(false) => {
            if let Ok(h) = hooks_dir() {
                if h.is_dir() {
                    r.warn(format!(
                        "hooks under {} — missing, not executable, or stale (run: gitpass hooks install)",
                        h.display()
                    ));
                } else {
                    r.warn("hooks not installed yet (run: gitpass hooks install)");
                }
            }
        }
        Err(e) => r.err(format!("hooks check: {e}")),
    }

    // credential helper for github.com (HTTPS)
    match git_global_config("credential.https://github.com.helper") {
        Ok(Some(v)) => {
            let expect = "git-credential-gitpass";
            if v.contains(expect) {
                r.ok(format!("git global credential.https://github.com.helper = {v}"));
            } else {
                r.warn(format!(
                    "credential.https://github.com.helper = {v} — expected !.../git-credential-gitpass (run: gitpass config install-credential-helper)"
                ));
            }
        }
        Ok(None) => r.warn(
            "HTTPS: credential helper for github.com not set (run: gitpass config install-credential-helper)",
        ),
        Err(e) => r.err(format!("git config credential helper: {e}")),
    }

    // core.hooksPath
    match git_global_config("core.hooksPath") {
        Ok(Some(v)) => {
            let expect_suffix = format!("{}/{}", gitpass_core::APP_SUPPORT_DIR, HOOKS_DIR_NAME);
            if v.contains(&expect_suffix) || v.ends_with(HOOKS_DIR_NAME) {
                r.ok(format!("git global core.hooksPath = {v}"));
            } else {
                r.warn(format!(
                    "core.hooksPath is set to {v} — expected .../{}/{} (run: gitpass hooks install)",
                    gitpass_core::APP_SUPPORT_DIR,
                    HOOKS_DIR_NAME
                ));
            }
        }
        Ok(None) => r.warn("git global core.hooksPath not set (run: gitpass hooks install)"),
        Err(e) => r.err(format!("git config: {e}")),
    }

    // gitpassd socket
    match ipc_socket_path() {
        Ok(p) => {
            if p.exists() {
                r.ok(format!("gitpassd socket present {}", p.display()));
            } else {
                r.warn(format!(
                    "gitpassd socket missing {} — start gitpassd (Login Item later)",
                    p.display()
                ));
            }
        }
        Err(e) => r.err(format!("socket path: {e}")),
    }

    r.ok(format!(
        "this gitpass binary (for hooks): {}",
        gitpass_bin.display()
    ));

    r.print_and_status()
}

pub fn git_global_config(key: &str) -> Result<Option<String>, String> {
    let out = Command::new("git")
        .args(["config", "--global", "--get", key])
        .output()
        .map_err(|e| format!("{e}"))?;
    if !out.status.success() {
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let v = s.trim();
    if v.is_empty() {
        return Ok(None);
    }
    Ok(Some(v.to_string()))
}

pub fn set_git_global_config(key: &str, value: &str) -> Result<(), String> {
    let st = Command::new("git")
        .args(["config", "--global", key, value])
        .status()
        .map_err(|e| format!("{e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("git config --global {key} failed (status {st})"))
    }
}

/// Backup existing `core.hooksPath` to `core.hooksPath.gitpass-backup` if set and different.
pub fn backup_hooks_path_if_needed(new_path: &str) -> Result<(), String> {
    match git_global_config("core.hooksPath")? {
        Some(cur) if cur != new_path => {
            set_git_global_config("core.hooksPath.gitpass-backup", &cur)?;
            Ok(())
        }
        _ => Ok(()),
    }
}
