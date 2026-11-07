//! Repositories recently validated by the Git hooks (`recent.json` in Application Support, not
//! sensitive): the menu bar falls back to the newest one (DL-004/DL-027). Written by `gitpass hook`
//! after each verdict, so it also covers commits checked while `gitpassd` was off.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{self, AuditKind};
use crate::config::AppConfig;
use crate::redact::short_message;
use crate::RepoBinding;

pub const RECENT_FILE_NAME: &str = "recent.json";
pub const RECENT_MAX: usize = 10;
const MESSAGE_MAX_CHARS: usize = 300;

/// Outcome of the last GitPass validation in a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    Blocked,
    Unconfirmed,
}

impl Verdict {
    pub const ALL: [Verdict; 3] = [Verdict::Ok, Verdict::Blocked, Verdict::Unconfirmed];

    /// Menu bar text.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Ok => "OK",
            Verdict::Blocked => "Bloqueado",
            Verdict::Unconfirmed => "Não confirmado",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub root_path: PathBuf,
    #[serde(default)]
    pub binding_id: Option<Uuid>,
    #[serde(default)]
    pub account_id: Option<Uuid>,
    /// `owner/repo` on GitHub.
    #[serde(default)]
    pub full_name: Option<String>,
    pub verdict: Verdict,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    /// RFC 3339.
    pub at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: Vec<RecentEntry>,
}

pub fn recent_file(app_support: &Path) -> PathBuf {
    app_support.join(RECENT_FILE_NAME)
}

/// Entries, newest first (empty when the file is missing or unreadable).
pub fn load_in(app_support: &Path) -> Vec<RecentEntry> {
    fs::read_to_string(recent_file(app_support))
        .ok()
        .and_then(|raw| serde_json::from_str::<RecentFile>(&raw).ok())
        .map(|f| f.entries)
        .unwrap_or_default()
}

pub fn load() -> Vec<RecentEntry> {
    crate::app_support_dir().map(|d| load_in(&d)).unwrap_or_default()
}

/// Puts `entry` first (replacing the previous one for the same folder) and keeps [`RECENT_MAX`].
pub fn record_in(app_support: &Path, mut entry: RecentEntry) -> io::Result<()> {
    entry.message = entry.message.map(|m| short_message(&m, MESSAGE_MAX_CHARS)).filter(|m| !m.is_empty());
    let mut entries = load_in(app_support);
    entries.retain(|e| e.root_path != entry.root_path);
    entries.insert(0, entry);
    entries.truncate(RECENT_MAX);
    let raw = serde_json::to_vec_pretty(&RecentFile { version: 1, entries }).map_err(io::Error::other)?;
    crate::integration::write_atomic(&recent_file(app_support), &raw, 0o600)
}

/// Binding whose root is `root` (paths compared as stored, then canonicalized).
pub fn binding_for_root<'a>(cfg: &'a AppConfig, root: &Path) -> Option<&'a RepoBinding> {
    cfg.repos.iter().find(|r| r.root_path == root).or_else(|| {
        let canon = root.canonicalize().ok()?;
        cfg.repos
            .iter()
            .find(|r| r.root_path.canonicalize().unwrap_or_else(|_| r.root_path.clone()) == canon)
    })
}

pub fn full_name(b: &RepoBinding) -> Option<String> {
    match (&b.github_owner, &b.github_repo) {
        (Some(o), Some(r)) => Some(format!("{o}/{}", r.trim_end_matches(".git"))),
        _ => None,
    }
}

/// What the audit log shows as the repository.
pub fn repo_label(full_name: Option<&str>, root: &Path) -> String {
    match full_name {
        Some(n) => n.to_string(),
        None => crate::redact::redact_home(&root.display().to_string(), dirs::home_dir().as_deref()),
    }
}

/// Why a hook let a commit through without `gitpassd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyVia {
    /// Toggle in the app / menu bar / CLI (`ui.emergency`).
    Toggle,
    /// Environment variable set for this command.
    Env,
}

/// One hook verdict, as `gitpass hook` saw it.
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub verdict: Verdict,
    pub code: Option<String>,
    pub message: Option<String>,
    /// Set when the commit passed only because emergency mode allowed it.
    pub emergency: Option<EmergencyVia>,
}

/// Records a hook verdict for the repository at `root`: `recent.json` and, for blocked,
/// unconfirmed or emergency verdicts, the audit log.
pub fn note_hook_outcome_in(app_support: &Path, cfg: Option<&AppConfig>, root: &Path, o: &HookOutcome) -> io::Result<()> {
    let binding = cfg.and_then(|c| binding_for_root(c, root));
    let name = binding.and_then(full_name);
    let repo = repo_label(name.as_deref(), root);
    let message = o.message.as_deref().map(clean_hook_message);
    match (o.emergency, o.verdict) {
        (_, Verdict::Blocked) => {
            let _ = audit::append_in(app_support, AuditKind::Blocked, Some(&repo), o.code.as_deref(), message.as_deref().unwrap_or("Bloqueado."));
        }
        (Some(via), _) => {
            let how = match via {
                EmergencyVia::Toggle => "pelo modo emergência",
                EmergencyVia::Env => "pelo modo emergência (variável de ambiente)",
            };
            let mut text = format!("Commit local liberado {how}: o serviço do GitPass não respondeu.");
            if o.verdict == Verdict::Unconfirmed {
                text.push_str(" A conta não pôde ser confirmada.");
            }
            let _ = audit::append_in(app_support, AuditKind::CommitAllowedEmergency, Some(&repo), o.code.as_deref(), &text);
        }
        (None, Verdict::Unconfirmed) => {
            let _ = audit::append_in(
                app_support,
                AuditKind::Unconfirmed,
                Some(&repo),
                o.code.as_deref(),
                message.as_deref().unwrap_or("Permitido sem confirmar a conta no GitHub."),
            );
        }
        (None, Verdict::Ok) => {}
    }
    record_in(
        app_support,
        RecentEntry {
            root_path: binding.map(|b| b.root_path.clone()).unwrap_or_else(|| root.to_path_buf()),
            binding_id: binding.map(|b| b.id),
            account_id: binding.map(|b| b.account_id),
            full_name: name,
            verdict: o.verdict,
            code: o.code.clone(),
            message,
            at: audit::now_rfc3339(),
        },
    )
}

/// Hook messages start with `GitPass: aviso:` for the terminal; the app shows them without it.
fn clean_hook_message(m: &str) -> String {
    let m = m.trim();
    let m = m.strip_prefix("GitPass:").map(str::trim_start).unwrap_or(m);
    let m = m.strip_prefix("aviso:").map(str::trim_start).unwrap_or(m);
    let mut c = m.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Account;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dir() -> Dir {
        let d = std::env::temp_dir().join(format!("gitpass-recent-{}", Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        Dir(d)
    }

    fn entry(root: &str, verdict: Verdict) -> RecentEntry {
        RecentEntry {
            root_path: root.into(),
            binding_id: None,
            account_id: None,
            full_name: None,
            verdict,
            code: None,
            message: None,
            at: audit::now_rfc3339(),
        }
    }

    #[test]
    fn newest_first_one_per_folder_at_most_ten() {
        let d = dir();
        for i in 0..12 {
            record_in(&d.0, entry(&format!("/r/{i}"), Verdict::Ok)).unwrap();
        }
        record_in(&d.0, entry("/r/5", Verdict::Blocked)).unwrap();
        let got = load_in(&d.0);
        assert_eq!(got.len(), RECENT_MAX);
        assert_eq!(got[0].root_path, PathBuf::from("/r/5"));
        assert_eq!(got[0].verdict, Verdict::Blocked);
        assert_eq!(got.iter().filter(|e| e.root_path == Path::new("/r/5")).count(), 1);
        assert_eq!(got[1].root_path, PathBuf::from("/r/11"));
        assert!(!d.0.join(".recent.json.tmp").exists());
    }

    #[test]
    fn hook_outcomes_fill_recent_and_audit_without_secrets() {
        let d = dir();
        let account_id = Uuid::new_v4();
        let binding_id = Uuid::new_v4();
        let cfg = AppConfig {
            accounts: vec![Account { id: account_id, github_login: Some("octo".into()), ..Default::default() }],
            repos: vec![RepoBinding {
                id: binding_id,
                account_id,
                root_path: "/repos/app".into(),
                github_owner: Some("octo".into()),
                github_repo: Some("app".into()),
            }],
            ..Default::default()
        };
        let root = Path::new("/repos/app");
        let blocked = HookOutcome {
            verdict: Verdict::Blocked,
            code: Some("GP-OWNER".into()),
            message: Some("origin https://u:gho_abc@github.com/x/y mudou".into()),
            emergency: None,
        };
        note_hook_outcome_in(&d.0, Some(&cfg), root, &blocked).unwrap();
        let r = &load_in(&d.0)[0];
        assert_eq!((r.binding_id, r.account_id), (Some(binding_id), Some(account_id)));
        assert_eq!(r.full_name.as_deref(), Some("octo/app"));
        assert_eq!(r.code.as_deref(), Some("GP-OWNER"));
        assert!(!r.message.as_deref().unwrap().contains("gho_abc"));
        let a = &audit::read_in(&d.0, 5)[0];
        assert_eq!((a.kind, a.repo.as_deref(), a.code.as_deref()), (AuditKind::Blocked, Some("octo/app"), Some("GP-OWNER")));

        let warned = HookOutcome {
            verdict: Verdict::Unconfirmed,
            code: None,
            message: Some("GitPass: aviso: o GitHub demorou".into()),
            emergency: None,
        };
        note_hook_outcome_in(&d.0, Some(&cfg), root, &warned).unwrap();
        assert_eq!(load_in(&d.0)[0].message.as_deref(), Some("O GitHub demorou"));
        assert_eq!(audit::read_in(&d.0, 5)[0].kind, AuditKind::Unconfirmed);

        let emergency = HookOutcome { verdict: Verdict::Ok, code: None, message: None, emergency: Some(EmergencyVia::Toggle) };
        note_hook_outcome_in(&d.0, Some(&cfg), root, &emergency).unwrap();
        let a = &audit::read_in(&d.0, 5)[0];
        assert_eq!(a.kind, AuditKind::CommitAllowedEmergency);
        assert!(a.message.contains("modo emergência"), "{}", a.message);

        let ok = HookOutcome { verdict: Verdict::Ok, code: None, message: None, emergency: None };
        note_hook_outcome_in(&d.0, Some(&cfg), root, &ok).unwrap();
        assert_eq!(audit::read_in(&d.0, 10).len(), 3);
        assert_eq!(load_in(&d.0).len(), 1);
        assert_eq!(load_in(&d.0)[0].verdict, Verdict::Ok);
    }

    #[test]
    fn verdicts_serialize_like_the_contract() {
        let v: Vec<_> = Verdict::ALL.iter().map(|v| serde_json::to_value(v).unwrap()).collect();
        assert_eq!(v, ["ok", "blocked", "unconfirmed"]);
        assert_eq!(Verdict::Unconfirmed.label(), "Não confirmado");
    }
}
