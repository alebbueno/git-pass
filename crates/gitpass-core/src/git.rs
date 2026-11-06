//! Minimal Git invocation helpers (no libgit2).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Return the absolute work tree root (`git rev-parse --show-toplevel`).
pub fn git_work_tree_root(cwd: Option<&Path>) -> Result<PathBuf, String> {
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--show-toplevel"]);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd.output().map_err(|e| format!("Não foi possível rodar o Git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Esta pasta não é um repositório Git ({})", err.trim()));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next().ok_or_else(|| "O Git não informou a pasta do repositório.".to_string())?;
    Ok(PathBuf::from(line.trim()))
}

/// `git remote get-url <name>` (typically `origin`).
pub fn git_remote_url(cwd: &Path, remote: &str) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["remote", "get-url", remote])
        .output()
        .map_err(|e| format!("Não foi possível rodar o Git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("O repositório não tem o remoto `{remote}` ({})", err.trim()));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.trim().to_string())
}

/// Recent commits: one line each — `hash — subject — author — date`.
pub fn git_log_lines(root: &Path, limit: usize) -> Result<Vec<String>, String> {
    let limit = limit.max(1).min(500);
    let out = Command::new("git")
        .current_dir(root)
        .args([
            "log",
            "-n",
            &limit.to_string(),
            "--date=short",
            "--pretty=format:%h — %s — %an — %ad",
        ])
        .output()
        .map_err(|e| format!("Não foi possível rodar o Git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("does not have any commits yet") {
            return Ok(Vec::new());
        }
        return Err(format!("Não foi possível ler o histórico do Git ({})", err.trim()));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<String> = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    Ok(lines)
}
