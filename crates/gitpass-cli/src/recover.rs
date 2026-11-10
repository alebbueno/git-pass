//! Recover `config.toml` accounts from Keychain OAuth tokens (`tokens/*.present`).

use std::fs;
use std::path::{Path, PathBuf};

use gitpass_core::github_user::{fetch_profile, GithubProfile};
use gitpass_core::recent::{self, RecentEntry};
use gitpass_core::bindings::{read_repo_identity, RepoIdentity};
use gitpass_core::{
    app_support_dir, config_file_path, git_remote_url, github_token_exists, github_token_load,
    parse_github_remote, Account, AppConfig, AuthMode, RepoBinding,
};
use uuid::Uuid;

#[derive(Debug)]
pub struct RecoverReport {
    pub accounts: usize,
    pub repos_from_recent: usize,
    pub repos_from_scan: usize,
    pub skipped_tokens: usize,
    pub backup: PathBuf,
    pub config_path: PathBuf,
}

pub fn run_recover(scan_roots: &[PathBuf], dry_run: bool) -> Result<RecoverReport, String> {
    let config_path = config_file_path().map_err(|e| e.to_string())?;
    let mut cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
    let kept_rules = std::mem::take(&mut cfg.account_rules);
    let kept_ui = std::mem::take(&mut cfg.ui);

    let backup = backup_config(&config_path)?;
    let mut accounts = Vec::new();
    let mut skipped_tokens = 0usize;

    for id in token_marker_ids()? {
        if !github_token_exists(id) {
            skipped_tokens += 1;
            eprintln!("skip {id}: sem marcador/token");
            continue;
        }
        let Some(bundle) = github_token_load(id).map_err(|e| e.to_string())? else {
            skipped_tokens += 1;
            eprintln!("skip {id}: token no Keychain ilegível ou ausente");
            continue;
        };
        let profile = fetch_profile(&bundle.access_token).map_err(api_err)?;
        let acc = account_from_profile(id, &profile);
        println!(
            "ok: @{} ({}) — {}",
            acc.github_login.as_deref().unwrap_or("?"),
            id,
            acc.email.as_deref().unwrap_or("(sem e-mail)")
        );
        accounts.push(acc);
    }

    if accounts.is_empty() {
        return Err(
            "Nenhuma conta recuperada. Conecte de novo em Contas ou verifique o Keychain.".into(),
        );
    }

    accounts.sort_by(|a, b| {
        a.github_login
            .as_deref()
            .unwrap_or("")
            .cmp(b.github_login.as_deref().unwrap_or(""))
    });

    let mut repos = Vec::new();
    let mut repos_from_recent = 0usize;
    for e in recent_entries()? {
        if let Some(b) = binding_from_recent(&accounts, &e) {
            if !repos.iter().any(|r: &RepoBinding| r.root_path == b.root_path) {
                repos.push(b);
                repos_from_recent += 1;
            }
        }
    }

    let mut repos_from_scan = 0usize;
    if !scan_roots.is_empty() {
        for root in scan_roots {
            for b in scan_repos_by_identity(root, &accounts)? {
                if !repos.iter().any(|r| r.root_path == b.root_path) {
                    repos.push(b);
                    repos_from_scan += 1;
                }
            }
        }
    }

    cfg.accounts = accounts;
    cfg.account_rules = kept_rules;
    cfg.ui = kept_ui;
    cfg.repos = repos;

    if dry_run {
        println!(
            "(dry-run) restauraria {} conta(s), {} repo(s) (recent: {}, scan: {})",
            cfg.accounts.len(),
            cfg.repos.len(),
            repos_from_recent,
            repos_from_scan
        );
        return Ok(RecoverReport {
            accounts: cfg.accounts.len(),
            repos_from_recent,
            repos_from_scan,
            skipped_tokens,
            backup,
            config_path,
        });
    }

    cfg.save().map_err(|e| e.to_string())?;
    println!(
        "gravado: {} conta(s), {} vínculo(s) — {}",
        cfg.accounts.len(),
        cfg.repos.len(),
        config_path.display()
    );
    println!("backup: {}", backup.display());

    Ok(RecoverReport {
        accounts: cfg.accounts.len(),
        repos_from_recent,
        repos_from_scan,
        skipped_tokens,
        backup,
        config_path,
    })
}

fn backup_config(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Ok(path.with_extension("toml.bak-missing"));
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_extension(format!("toml.bak-{stamp}"));
    fs::copy(path, &backup).map_err(|e| e.to_string())?;
    Ok(backup)
}

fn token_marker_ids() -> Result<Vec<Uuid>, String> {
    let dir = app_support_dir()
        .map_err(|e| e.to_string())?
        .join("tokens");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for ent in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let ent = ent.map_err(|e| e.to_string())?;
        let name = ent.file_name();
        let Some(s) = name.to_str() else { continue };
        let Some(uuid_s) = s.strip_suffix(".present") else { continue };
        if let Ok(id) = Uuid::parse_str(uuid_s) {
            ids.push(id);
        }
    }
    ids.sort_by_key(|u| u.to_string());
    Ok(ids)
}

fn account_from_profile(id: Uuid, p: &GithubProfile) -> Account {
    Account {
        id,
        github_login: Some(p.user.login.clone()),
        label: None,
        auth: AuthMode::Https,
        name: p
            .user
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| Some(p.user.login.clone())),
        email: Some(p.commit_email.clone()),
        avatar_url: p.user.avatar_url.clone(),
        ..Default::default()
    }
}

fn recent_entries() -> Result<Vec<RecentEntry>, String> {
    let dir = app_support_dir().map_err(|e| e.to_string())?;
    Ok(recent::load_in(&dir))
}

fn binding_from_recent(accounts: &[Account], e: &RecentEntry) -> Option<RepoBinding> {
    if !e.root_path.exists() {
        return None;
    }
    let account_id = e.account_id?;
    if !accounts.iter().any(|a| a.id == account_id) {
        return None;
    }
    let (owner, repo) = split_full_name(e.full_name.as_deref()?)?;
    Some(RepoBinding {
        id: e.binding_id.unwrap_or_else(Uuid::new_v4),
        account_id,
        root_path: e.root_path.clone(),
        github_owner: Some(owner),
        github_repo: Some(repo),
    })
}

fn split_full_name(full: &str) -> Option<(String, String)> {
    let (o, r) = full.split_once('/')?;
    if o.is_empty() || r.is_empty() {
        return None;
    }
    Some((o.to_string(), r.to_string()))
}

fn scan_repos_by_identity(scan_root: &Path, accounts: &[Account]) -> Result<Vec<RepoBinding>, String> {
    if !scan_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    walk_git_roots(scan_root, 0, 4, &mut |root| {
        let RepoIdentity { email, .. } = read_repo_identity(root);
        let Some(email) = email.filter(|e| !e.is_empty()) else {
            return;
        };
        let Some(acc) = accounts
            .iter()
            .find(|a| a.email.as_ref().is_some_and(|e| e.eq_ignore_ascii_case(&email)))
        else {
            return;
        };
        let Some((owner, repo)) = github_from_origin(root) else {
            return;
        };
        out.push(RepoBinding {
            id: Uuid::new_v4(),
            account_id: acc.id,
            root_path: root.to_path_buf(),
            github_owner: Some(owner),
            github_repo: Some(repo),
        });
    });
    Ok(out)
}

fn walk_git_roots(dir: &Path, depth: usize, max_depth: usize, f: &mut dyn FnMut(&Path)) {
    if depth > max_depth {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let path = ent.path();
        let ft = ent.file_type().ok();
        if ft.as_ref().is_some_and(|t| t.is_dir()) {
            if path.file_name().is_some_and(|n| n == ".git") {
                if let Some(root) = path.parent() {
                    f(root);
                }
                continue;
            }
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            walk_git_roots(&path, depth + 1, max_depth, f);
        }
    }
}

fn github_from_origin(root: &Path) -> Option<(String, String)> {
    let url = git_remote_url(root, "origin").ok()?;
    let id = parse_github_remote(&url)?;
    Some((id.owner, id.repo))
}

fn api_err(e: gitpass_core::github_api::GithubApiError) -> String {
    format!("GitHub API: {e}")
}
