//! Running `git` against github.com with an account's OAuth token (clone, fetch, push, pull).
//!
//! The token is handed to `git` **only through the environment** (`GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/
//! `GIT_CONFIG_VALUE_n` → `http.https://github.com/.extraheader`), so it never shows up in `ps`, in a remote
//! URL, in `.git/config` or in logs. Every configured credential helper is reset for that one command, and
//! SSH GitHub remotes are rewritten to HTTPS so the bound account is used whatever key the SSH agent holds.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// Git executable (Homebrew/CLT aware, see [`crate::integration::find_git`]), else `git` from `PATH`.
pub fn git_program() -> PathBuf {
    crate::integration::find_git().unwrap_or_else(|| PathBuf::from("git"))
}

/// Value of the `Authorization` header used for HTTPS access to github.com with an OAuth token.
pub fn auth_header_value(token: &str) -> String {
    format!("AUTHORIZATION: basic {}", B64.encode(format!("x-access-token:{token}")))
}

/// Config entries injected through `GIT_CONFIG_*` for a token-authenticated command.
pub fn token_config(token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("http.https://github.com/.extraheader", auth_header_value(token)),
        // Empty helper resets every configured helper (global and `credential.<url>.helper`): if the
        // token is rejected, Git must not fall back to another account's credentials, nor erase them.
        ("credential.helper", String::new()),
        ("url.https://github.com/.insteadOf", "git@github.com:".to_string()),
        ("url.https://github.com/.insteadOf", "ssh://git@github.com/".to_string()),
    ]
}

/// Environment shared by every `git` the app runs: English output (parsed), no prompts, no inherited
/// repository overrides. With `token`, also the [`token_config`] entries.
pub fn apply_git_env(cmd: &mut Command, token: Option<&str>) {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        // A stalled transfer (under 1 KB/s for 60 s) fails instead of hanging the app.
        .env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
        .env("GIT_HTTP_LOW_SPEED_TIME", "60")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_CONFIG_PARAMETERS");
    if let Some(token) = token {
        let entries = token_config(token);
        cmd.env("GIT_CONFIG_COUNT", entries.len().to_string());
        for (i, (key, value)) in entries.into_iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), key).env(format!("GIT_CONFIG_VALUE_{i}"), value);
        }
    }
}

/// Replaces the token (raw and inside the basic-auth header) with `***`.
pub fn redact(text: &str, token: &str) -> String {
    let mut out = text.to_string();
    if token.is_empty() {
        return out;
    }
    for secret in [token.to_string(), B64.encode(format!("x-access-token:{token}"))] {
        out = out.replace(&secret, "***");
    }
    out
}

/// Message when `git` cannot be started.
pub fn spawn_error(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "O Git não está instalado. Instale as Xcode Command Line Tools (`xcode-select --install`).".to_string()
    } else {
        format!("Não foi possível rodar o Git: {e}")
    }
}

/// Reads `stream` until EOF, calling `on_line` for every line; `\r` also ends a line (progress updates).
pub fn for_each_line(mut stream: impl Read, mut on_line: impl FnMut(&str)) {
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let mut flush = |pending: &mut Vec<u8>| {
        if !pending.is_empty() {
            on_line(&String::from_utf8_lossy(pending));
            pending.clear();
        }
    };
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                flush(&mut pending);
            } else {
                pending.push(b);
            }
        }
    }
    flush(&mut pending);
}

/// One progress sample parsed from `git clone/fetch/push --progress` (stderr).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GitProgress {
    /// `counting`, `compressing`, `receiving`, `writing`, `resolving` or `checkout`.
    pub phase: &'static str,
    /// 0..=100.
    pub percent: u8,
}

/// Parses a single progress line such as `Receiving objects:  45% (450/1000), 1.2 MiB | 2.0 MiB/s`
/// (an optional `remote: ` prefix is accepted). Returns `None` for lines without a percentage.
pub fn parse_progress_line(line: &str) -> Option<GitProgress> {
    let line = line.trim();
    let line = line.strip_prefix("remote:").unwrap_or(line).trim_start();
    let (label, rest) = line.split_once(':')?;
    let phase = match label.trim() {
        "Counting objects" | "Enumerating objects" => "counting",
        "Compressing objects" => "compressing",
        "Receiving objects" => "receiving",
        "Writing objects" => "writing",
        "Resolving deltas" => "resolving",
        "Updating files" | "Checking out files" => "checkout",
        _ => return None,
    };
    let rest = rest.trim_start();
    let digits_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 || !rest[digits_end..].starts_with('%') {
        return None;
    }
    let percent: u32 = rest[..digits_end].parse().ok()?;
    Some(GitProgress {
        phase,
        percent: percent.min(100) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_header_is_basic_x_access_token() {
        let h = auth_header_value("abc");
        assert_eq!(h, format!("AUTHORIZATION: basic {}", B64.encode("x-access-token:abc")));
    }

    #[test]
    fn redact_hides_token() {
        let t = "gho_secret";
        let text = format!("oops {t} and {}", B64.encode(format!("x-access-token:{t}")));
        let r = redact(&text, t);
        assert!(!r.contains(t) && !r.contains(&B64.encode(format!("x-access-token:{t}"))));
        assert_eq!(redact("nothing", ""), "nothing");
    }

    /// The token reaches `git` as config (no network needed to check it) and never as an argument.
    #[test]
    fn token_is_visible_to_git_only_through_the_environment() {
        let mut cmd = Command::new(git_program());
        cmd.args(["config", "--get-all", "http.https://github.com/.extraheader"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        apply_git_env(&mut cmd, Some("gho_test"));
        assert!(cmd.get_args().all(|a| !a.to_string_lossy().contains("gho_test")));
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), auth_header_value("gho_test"));

        // SSH remotes are rewritten so the token (not the SSH agent's key) authenticates.
        let mut cmd = Command::new(git_program());
        cmd.args(["ls-remote", "--get-url", "git@github.com:o/r.git"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        apply_git_env(&mut cmd, Some("gho_test"));
        let out = cmd.output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "https://github.com/o/r.git");
    }

    #[test]
    fn without_token_no_config_is_injected() {
        let mut cmd = Command::new("true");
        apply_git_env(&mut cmd, None);
        assert!(cmd.get_envs().all(|(k, _)| k != "GIT_CONFIG_COUNT"));
    }

    #[test]
    fn lines_split_on_cr_and_lf() {
        let mut got = Vec::new();
        for_each_line(&b"a: 1%\rb\n\nc"[..], |l| got.push(l.to_string()));
        assert_eq!(got, ["a: 1%", "b", "c"]);
    }

    fn p(phase: &'static str, percent: u8) -> Option<GitProgress> {
        Some(GitProgress { phase, percent })
    }

    #[test]
    fn parses_push_progress() {
        assert_eq!(parse_progress_line("Writing objects:  50% (1/2), 200 bytes | 200.00 KiB/s"), p("writing", 50));
        assert_eq!(parse_progress_line("remote: Resolving deltas: 100% (3/3), done."), p("resolving", 100));
        assert_eq!(parse_progress_line("Enumerating objects: 5, done."), None);
    }
}
