//! "Current repository" of the menu bar (DL-004/DL-027): pinned > app in focus > most recent.
//! Types match `CurrentRepo` / `CurrentRepoState` in `api-system.ts`.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::AppConfig;
use crate::recent::{binding_for_root, full_name, RecentEntry, Verdict};
use crate::RepoBinding;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentRepoSource {
    Pinned,
    Focus,
    Recent,
}

impl CurrentRepoSource {
    pub const ALL: [CurrentRepoSource; 3] = [CurrentRepoSource::Pinned, CurrentRepoSource::Focus, CurrentRepoSource::Recent];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CurrentRepo {
    pub binding_id: Option<String>,
    pub root_path: String,
    pub full_name: Option<String>,
    pub account_id: Option<String>,
    pub source: CurrentRepoSource,
    pub at: String,
    pub verdict: Option<Verdict>,
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CurrentRepoState {
    pub current: Option<CurrentRepo>,
    pub pinned_binding_id: Option<String>,
    pub recent: Vec<CurrentRepo>,
    pub accessibility_granted: bool,
}

/// Repository of the app in focus, and since when it has been in focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedRepo {
    pub root_path: PathBuf,
    pub at: String,
}

fn last_verdict<'a>(recent: &'a [RecentEntry], root: &Path) -> Option<&'a RecentEntry> {
    recent.iter().find(|e| e.root_path == root)
}

fn from_binding(b: &RepoBinding, source: CurrentRepoSource, at: String, last: Option<&RecentEntry>) -> CurrentRepo {
    CurrentRepo {
        binding_id: Some(b.id.to_string()),
        root_path: b.root_path.display().to_string(),
        full_name: full_name(b).or_else(|| last.and_then(|e| e.full_name.clone())),
        account_id: Some(b.account_id.to_string()),
        source,
        at,
        verdict: last.map(|e| e.verdict),
        code: last.and_then(|e| e.code.clone()),
        message: last.and_then(|e| e.message.clone()),
    }
}

fn from_recent(cfg: &AppConfig, e: &RecentEntry) -> CurrentRepo {
    match binding_for_root(cfg, &e.root_path) {
        Some(b) => from_binding(b, CurrentRepoSource::Recent, e.at.clone(), Some(e)),
        None => CurrentRepo {
            binding_id: None,
            root_path: e.root_path.display().to_string(),
            full_name: e.full_name.clone(),
            account_id: e.account_id.map(|a| a.to_string()),
            source: CurrentRepoSource::Recent,
            at: e.at.clone(),
            verdict: Some(e.verdict),
            code: e.code.clone(),
            message: e.message.clone(),
        },
    }
}

/// Builds the state. A pin to a binding that no longer exists is ignored.
pub fn resolve(
    cfg: &AppConfig,
    recent: &[RecentEntry],
    focus: Option<&FocusedRepo>,
    accessibility_granted: bool,
    now: &str,
) -> CurrentRepoState {
    let pinned = cfg
        .ui
        .pinned_repo_binding_id
        .and_then(|id| cfg.repos.iter().find(|r| r.id == id));
    let recent_rows: Vec<CurrentRepo> = recent.iter().map(|e| from_recent(cfg, e)).collect();

    let current = if let Some(b) = pinned {
        let last = last_verdict(recent, &b.root_path);
        let at = cfg.ui.pinned_at.clone().or_else(|| last.map(|e| e.at.clone())).unwrap_or_else(|| now.to_string());
        Some(from_binding(b, CurrentRepoSource::Pinned, at, last))
    } else if let Some(f) = focus.filter(|_| accessibility_granted) {
        let last = last_verdict(recent, &f.root_path);
        Some(match binding_for_root(cfg, &f.root_path) {
            Some(b) => from_binding(b, CurrentRepoSource::Focus, f.at.clone(), last),
            None => CurrentRepo {
                binding_id: None,
                root_path: f.root_path.display().to_string(),
                full_name: None,
                account_id: None,
                source: CurrentRepoSource::Focus,
                at: f.at.clone(),
                verdict: None,
                code: None,
                message: None,
            },
        })
    } else {
        recent_rows.first().cloned()
    };

    CurrentRepoState {
        current,
        pinned_binding_id: pinned.map(|b| b.id.to_string()),
        recent: recent_rows,
        accessibility_granted,
    }
}

fn expand_tilde(p: &str, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(rest) = p.strip_prefix("~/") {
        return home.map(|h| h.join(rest));
    }
    if p == "~" {
        return home.map(Path::to_path_buf);
    }
    p.starts_with('/').then(|| PathBuf::from(p))
}

/// Deepest bound repository containing `path`.
fn binding_containing<'a>(cfg: &'a AppConfig, path: &Path) -> Option<&'a RepoBinding> {
    cfg.repos
        .iter()
        .filter(|r| path.starts_with(&r.root_path))
        .max_by_key(|r| r.root_path.components().count())
}

/// File path of a window's `AXDocument` (`file:///…` URL or a plain path).
pub fn document_path(document: &str) -> Option<PathBuf> {
    let d = document.trim();
    if let Some(rest) = d.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let decoded = url::Url::parse(&format!("file://{rest}")).ok()?.to_file_path().ok()?;
        return Some(decoded);
    }
    d.starts_with('/').then(|| PathBuf::from(d))
}

const TITLE_SEPARATORS: [&str; 4] = [" — ", " – ", " - ", " | "];

/// Bound repository shown by the app in focus: the window's document (Terminal and iTerm set it to
/// the shell's folder, VS Code / Cursor to the open file), else a path or a repository folder name
/// in the window title (`main.rs — my-app — Cursor`, `~/src/my-app (zsh)`). Only bound
/// repositories are matched; nothing on disk is read.
pub fn binding_for_focus<'a>(
    cfg: &'a AppConfig,
    document: Option<&str>,
    title: Option<&str>,
    home: Option<&Path>,
) -> Option<&'a RepoBinding> {
    if let Some(b) = document.and_then(document_path).and_then(|p| binding_containing(cfg, &p)) {
        return Some(b);
    }
    let title = title?.trim();
    if title.is_empty() {
        return None;
    }
    let by_path = title
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '"' | '\'' | ':'))
        .filter_map(|w| expand_tilde(w.trim_end_matches(['/', ',']), home))
        .find_map(|p| binding_containing(cfg, &p));
    if by_path.is_some() {
        return by_path;
    }
    let mut parts = vec![title.to_string()];
    for sep in TITLE_SEPARATORS {
        parts = parts.iter().flat_map(|p| p.split(sep).map(|s| s.trim().to_string())).collect();
    }
    for part in parts.iter().filter(|p| !p.is_empty()) {
        let part = part.trim_start_matches(['●', '•', '*']).trim();
        let matches: Vec<&RepoBinding> = cfg
            .repos
            .iter()
            .filter(|r| r.root_path.file_name().is_some_and(|n| n.to_string_lossy() == part))
            .collect();
        // Two bound repositories with the same folder name: the title does not say which.
        if matches.len() == 1 {
            return Some(matches[0]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn binding(root: &str, owner: &str, repo: &str) -> RepoBinding {
        RepoBinding {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            root_path: root.into(),
            github_owner: Some(owner.into()),
            github_repo: Some(repo.into()),
        }
    }

    fn cfg() -> AppConfig {
        AppConfig {
            repos: vec![
                binding("/Users/ana/src/app", "octo", "app"),
                binding("/Users/ana/src/app/vendor/lib", "octo", "lib"),
                binding("/Users/ana/work/site", "acme", "site"),
                binding("/Users/ana/a/dup", "x", "dup"),
                binding("/Users/ana/b/dup", "y", "dup"),
            ],
            ..Default::default()
        }
    }

    fn entry(root: &str, verdict: Verdict, at: &str) -> RecentEntry {
        RecentEntry {
            root_path: root.into(),
            binding_id: None,
            account_id: None,
            full_name: None,
            verdict,
            code: (verdict == Verdict::Blocked).then(|| "GP-OWNER".into()),
            message: None,
            at: at.into(),
        }
    }

    #[test]
    fn pin_beats_focus_which_beats_recent() {
        let mut c = cfg();
        let recent = vec![
            entry("/Users/ana/work/site", Verdict::Blocked, "2026-11-06T10:00:00Z"),
            entry("/Users/ana/src/app", Verdict::Ok, "2026-11-06T09:00:00Z"),
        ];
        let focus = FocusedRepo { root_path: "/Users/ana/src/app".into(), at: "2026-11-06T11:00:00Z".into() };

        let s = resolve(&c, &recent, None, true, "now");
        let cur = s.current.unwrap();
        assert_eq!((cur.source, cur.full_name.as_deref()), (CurrentRepoSource::Recent, Some("acme/site")));
        assert_eq!((cur.verdict, cur.code.as_deref()), (Some(Verdict::Blocked), Some("GP-OWNER")));
        assert_eq!(cur.binding_id, Some(c.repos[2].id.to_string()));
        assert_eq!(s.recent.len(), 2);

        let s = resolve(&c, &recent, Some(&focus), true, "now");
        let cur = s.current.unwrap();
        assert_eq!((cur.source, cur.full_name.as_deref()), (CurrentRepoSource::Focus, Some("octo/app")));
        assert_eq!((cur.verdict, cur.at.as_str()), (Some(Verdict::Ok), "2026-11-06T11:00:00Z"));

        // Without the Accessibility permission focus is not used.
        assert_eq!(resolve(&c, &recent, Some(&focus), false, "now").current.unwrap().source, CurrentRepoSource::Recent);

        c.ui.pinned_repo_binding_id = Some(c.repos[2].id);
        c.ui.pinned_at = Some("2026-11-01T00:00:00Z".into());
        let s = resolve(&c, &recent, Some(&focus), true, "now");
        let cur = s.current.unwrap();
        assert_eq!((cur.source, cur.at.as_str()), (CurrentRepoSource::Pinned, "2026-11-01T00:00:00Z"));
        assert_eq!(s.pinned_binding_id, Some(c.repos[2].id.to_string()));

        c.ui.pinned_repo_binding_id = Some(Uuid::new_v4());
        let s = resolve(&c, &recent, None, false, "now");
        assert_eq!((s.pinned_binding_id, s.current.unwrap().source), (None, CurrentRepoSource::Recent));
        assert_eq!(resolve(&AppConfig::default(), &[], None, false, "now").current, None);
    }

    #[test]
    fn state_serializes_like_the_contract() {
        let c = cfg();
        let s = resolve(&c, &[entry("/gone", Verdict::Unconfirmed, "t")], None, false, "now");
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["current"]["source"], "recent");
        assert_eq!(v["current"]["verdict"], "unconfirmed");
        assert_eq!(v["current"]["binding_id"], serde_json::Value::Null);
        assert_eq!(v["accessibility_granted"], false);
    }

    #[test]
    fn focus_is_matched_by_document_path_or_title() {
        let c = cfg();
        let home = Some(Path::new("/Users/ana"));
        let name = |b: Option<&RepoBinding>| b.and_then(full_name);
        assert_eq!(name(binding_for_focus(&c, Some("file:///Users/ana/src/app/"), None, home)), Some("octo/app".into()));
        assert_eq!(
            name(binding_for_focus(&c, Some("file:///Users/ana/src/app/vendor/lib/x%20y.rs"), None, home)),
            Some("octo/lib".into())
        );
        assert_eq!(name(binding_for_focus(&c, Some("/Users/ana/work/site/index.html"), None, home)), Some("acme/site".into()));
        assert!(binding_for_focus(&c, Some("file:///Users/ana/elsewhere/"), None, home).is_none());
        // Cursor / VS Code titles.
        assert_eq!(name(binding_for_focus(&c, None, Some("main.rs — site — Cursor"), home)), Some("acme/site".into()));
        assert_eq!(name(binding_for_focus(&c, None, Some("● lib.rs - app - Visual Studio Code"), home)), Some("octo/app".into()));
        // Terminal titles with the folder.
        assert_eq!(name(binding_for_focus(&c, None, Some("ana@mac: ~/src/app/vendor/lib"), home)), Some("octo/lib".into()));
        assert_eq!(name(binding_for_focus(&c, None, Some("~/work/site (zsh)"), home)), Some("acme/site".into()));
        // Ambiguous or unrelated names.
        assert!(binding_for_focus(&c, None, Some("notes — dup — Cursor"), home).is_none());
        assert!(binding_for_focus(&c, None, Some("Mail"), home).is_none());
        assert!(binding_for_focus(&c, None, Some(""), home).is_none());
    }
}
