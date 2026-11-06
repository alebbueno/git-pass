//! Git credential helper — supplies OAuth token for `https://github.com/...`.
//!
//! Resolution order:
//! 1. **Local repo**: `GIT_WORK_TREE` or `PWD` or current dir → `git rev-parse --show-toplevel` → match `[[repos]].root_path`.
//! 2. **Fallback**: match credential `path` (`owner/repo`) against stored `github_owner` + `github_repo`.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use gitpass_core::{git_work_tree_root, github_token_load, normalize_owner, AppConfig};
use uuid::Uuid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let op = std::env::args().nth(1).unwrap_or_default();
    match op.as_str() {
        "get" => get(),
        "store" | "erase" => Ok(()), // Keychain only
        _ => Ok(()),
    }
}

fn read_input() -> io::Result<HashMap<String, String>> {
    let stdin = io::stdin().lock();
    let mut map = HashMap::new();
    for line in BufReader::new(stdin).lines() {
        let line = line?;
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.to_string(), v.to_string());
        }
    }
    Ok(map)
}

/// Directories Git often sets when running credential helpers from a repo context.
fn effective_cwd() -> std::path::PathBuf {
    std::env::var("GIT_WORK_TREE")
        .or_else(|_| std::env::var("PWD"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
}

fn find_binding_by_local_root(cfg: &AppConfig) -> Option<Uuid> {
    let cwd = effective_cwd();
    let root = git_work_tree_root(Some(&cwd)).ok()?;
    let root = root.canonicalize().unwrap_or(root);

    cfg.repos.iter().find_map(|r| {
        let bound = r.root_path.canonicalize().unwrap_or_else(|_| r.root_path.clone());
        if bound == root {
            Some(r.account_id)
        } else {
            None
        }
    })
}

fn parse_owner_repo(path: &str) -> Option<(String, String)> {
    let path_trim = path.trim().trim_start_matches('/').trim_end_matches('/');
    let (owner, repo) = path_trim.split_once('/')?;
    let repo = repo.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((normalize_owner(owner), repo.to_string()))
}

fn find_account_id_by_url_path(cfg: &AppConfig, owner_n: &str, repo: &str) -> Option<Uuid> {
    cfg.repos.iter().find_map(|r| {
        let owner_ok = r
            .github_owner
            .as_ref()
            .map(|o| normalize_owner(o) == *owner_n)
            .unwrap_or(false);
        let repo_ok = r.github_repo.as_ref().map_or(false, |name| {
            name.trim_end_matches(".git")
                .eq_ignore_ascii_case(repo.trim_end_matches(".git"))
        });
        if owner_ok && repo_ok {
            Some(r.account_id)
        } else {
            None
        }
    })
}

fn get() -> Result<(), Box<dyn std::error::Error>> {
    let input = read_input()?;
    let protocol = input.get("protocol").map(|s| s.as_str()).unwrap_or("");
    let host = input.get("host").map(|s| s.as_str()).unwrap_or("");
    let path = input.get("path").map(|s| s.as_str()).unwrap_or("");

    if protocol != "https" || !host.eq_ignore_ascii_case("github.com") {
        return Ok(());
    }

    let cfg = AppConfig::load_or_default()?;

    let account_id = find_binding_by_local_root(&cfg).or_else(|| {
        let (owner_n, repo) = parse_owner_repo(path)?;
        find_account_id_by_url_path(&cfg, &owner_n, &repo)
    });

    let Some(aid) = account_id else {
        return Ok(());
    };

    let acc = cfg
        .accounts
        .iter()
        .find(|a| a.id == aid)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "account missing"))?;

    let login = acc
        .github_login
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no github_login"))?;

    let Some(tok) = github_token_load(aid)? else {
        return Ok(());
    };

    let mut out = io::stdout().lock();
    writeln!(out, "username={login}")?;
    writeln!(out, "password={}", tok.access_token)?;
    Ok(())
}
