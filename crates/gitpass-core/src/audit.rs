//! Local audit log (JSON Lines, no secrets) under `Application Support/logs`: emergency mode,
//! commits let through by it, blocked / unconfirmed validations and CI bypasses (DL-016, DL-017).
//! Rotated at [`AUDIT_MAX_BYTES`], keeping [`AUDIT_FILES`] files.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::integration::LOGS_DIR_NAME;
use crate::redact::short_message;

pub const AUDIT_FILE_STEM: &str = "audit";
pub const AUDIT_MAX_BYTES: u64 = 1024 * 1024;
/// Current file plus rotated ones (`audit.jsonl`, `audit.1.jsonl`, …).
pub const AUDIT_FILES: usize = 5;
const MESSAGE_MAX_CHARS: usize = 400;
/// One `git commit` runs several validated hooks; the same verdict is logged once.
const DEDUP_SECONDS: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    EmergencyOn,
    EmergencyOff,
    EmergencyExpired,
    CommitAllowedEmergency,
    Blocked,
    Unconfirmed,
    CiBypass,
}

impl AuditKind {
    pub const ALL: [AuditKind; 7] = [
        AuditKind::EmergencyOn,
        AuditKind::EmergencyOff,
        AuditKind::EmergencyExpired,
        AuditKind::CommitAllowedEmergency,
        AuditKind::Blocked,
        AuditKind::Unconfirmed,
        AuditKind::CiBypass,
    ];

    /// Hook verdicts repeat for every phase of one Git command.
    fn deduplicated(self) -> bool {
        matches!(
            self,
            AuditKind::CommitAllowedEmergency | AuditKind::Blocked | AuditKind::Unconfirmed | AuditKind::CiBypass
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: String,
    pub kind: AuditKind,
    /// `owner/repo` when known, else the folder with the home written as `~`.
    pub repo: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn logs_dir(app_support: &Path) -> PathBuf {
    app_support.join(LOGS_DIR_NAME)
}

/// Audit files, newest first (only the ones that may exist; callers skip missing files).
pub fn audit_files(app_support: &Path) -> Vec<PathBuf> {
    let dir = logs_dir(app_support);
    (0..AUDIT_FILES)
        .map(|i| match i {
            0 => dir.join(format!("{AUDIT_FILE_STEM}.jsonl")),
            n => dir.join(format!("{AUDIT_FILE_STEM}.{n}.jsonl")),
        })
        .collect()
}

fn rotate(files: &[PathBuf]) {
    let _ = fs::remove_file(&files[files.len() - 1]);
    for i in (0..files.len() - 1).rev() {
        let _ = fs::rename(&files[i], &files[i + 1]);
    }
}

/// Last line of `path` (reads at most the final 8 KB).
fn last_line(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(8 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    buf.lines().rev().find(|l| !l.trim().is_empty()).map(String::from)
}

fn is_duplicate(path: &Path, e: &AuditEntry) -> bool {
    let Some(prev) = last_line(path).and_then(|l| serde_json::from_str::<AuditEntry>(&l).ok()) else {
        return false;
    };
    let close = match (DateTime::parse_from_rfc3339(&prev.at), DateTime::parse_from_rfc3339(&e.at)) {
        (Ok(a), Ok(b)) => (b - a).num_seconds().abs() <= DEDUP_SECONDS,
        _ => false,
    };
    close && prev.kind == e.kind && prev.repo == e.repo && prev.code == e.code
}

/// Appends one entry to the audit log in `app_support` (message redacted and shortened).
pub fn append_in(app_support: &Path, kind: AuditKind, repo: Option<&str>, code: Option<&str>, message: &str) -> io::Result<()> {
    let entry = AuditEntry {
        at: now_rfc3339(),
        kind,
        repo: repo.map(|r| short_message(r, 200)),
        code: code.map(String::from),
        message: short_message(message, MESSAGE_MAX_CHARS),
    };
    let files = audit_files(app_support);
    let current = &files[0];
    fs::create_dir_all(current.parent().unwrap_or(app_support))?;
    if kind.deduplicated() && is_duplicate(current, &entry) {
        return Ok(());
    }
    let mut line = serde_json::to_string(&entry).map_err(io::Error::other)?;
    line.push('\n');
    if fs::metadata(current).is_ok_and(|m| m.len() + line.len() as u64 > AUDIT_MAX_BYTES) {
        rotate(&files);
    }
    let mut opts = fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    // One `write` per line: concurrent hooks appending do not interleave.
    opts.open(current)?.write_all(line.as_bytes())
}

/// [`append_in`] for the real Application Support folder. Errors are ignored: auditing never
/// blocks Git or the app.
pub fn append(kind: AuditKind, repo: Option<&str>, code: Option<&str>, message: &str) {
    if let Ok(dir) = crate::app_support_dir() {
        let _ = append_in(&dir, kind, repo, code, message);
    }
}

/// Up to `limit` entries, newest first. Unreadable lines are skipped.
pub fn read_in(app_support: &Path, limit: usize) -> Vec<AuditEntry> {
    let mut out = Vec::new();
    for file in audit_files(app_support) {
        if out.len() >= limit {
            break;
        }
        let Ok(raw) = fs::read_to_string(&file) else { continue };
        for line in raw.lines().rev() {
            if out.len() >= limit {
                break;
            }
            if let Ok(e) = serde_json::from_str::<AuditEntry>(line) {
                out.push(e);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dir() -> Dir {
        let d = std::env::temp_dir().join(format!("gitpass-audit-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        Dir(d)
    }

    #[test]
    fn entries_are_read_newest_first_and_redacted() {
        let d = dir();
        append_in(&d.0, AuditKind::EmergencyOn, None, None, "motivo com gho_secret123").unwrap();
        append_in(&d.0, AuditKind::Blocked, Some("octo/app"), Some("GP-OWNER"), "bloqueado").unwrap();
        let got = read_in(&d.0, 10);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, AuditKind::Blocked);
        assert_eq!(got[0].repo.as_deref(), Some("octo/app"));
        assert_eq!(got[1].kind, AuditKind::EmergencyOn);
        assert!(!got[1].message.contains("gho_secret123"), "{}", got[1].message);
        assert_eq!(read_in(&d.0, 1).len(), 1);
        let raw = fs::read_to_string(&audit_files(&d.0)[0]).unwrap();
        assert!(!raw.contains("gho_secret123"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&audit_files(&d.0)[0]).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
    }

    #[test]
    fn the_same_hook_verdict_is_logged_once_per_command() {
        let d = dir();
        for _ in 0..3 {
            append_in(&d.0, AuditKind::CommitAllowedEmergency, Some("octo/app"), None, "liberado").unwrap();
        }
        append_in(&d.0, AuditKind::CommitAllowedEmergency, Some("octo/other"), None, "liberado").unwrap();
        append_in(&d.0, AuditKind::EmergencyOff, None, None, "desligado").unwrap();
        append_in(&d.0, AuditKind::EmergencyOff, None, None, "desligado").unwrap();
        let kinds: Vec<_> = read_in(&d.0, 10).into_iter().map(|e| (e.kind, e.repo)).collect();
        assert_eq!(
            kinds,
            vec![
                (AuditKind::EmergencyOff, None),
                (AuditKind::EmergencyOff, None),
                (AuditKind::CommitAllowedEmergency, Some("octo/other".into())),
                (AuditKind::CommitAllowedEmergency, Some("octo/app".into())),
            ]
        );
    }

    #[test]
    fn log_rotates_and_keeps_a_bounded_number_of_files() {
        let d = dir();
        let files = audit_files(&d.0);
        fs::create_dir_all(files[0].parent().unwrap()).unwrap();
        let old = AuditEntry {
            at: "2020-01-01T00:00:00Z".into(),
            kind: AuditKind::EmergencyOn,
            repo: None,
            code: None,
            message: "x".repeat(200),
        };
        let line = format!("{}\n", serde_json::to_string(&old).unwrap());
        let filler = line.repeat((AUDIT_MAX_BYTES as usize / line.len()) + 1);
        for f in &files {
            fs::write(f, &filler).unwrap();
        }
        append_in(&d.0, AuditKind::EmergencyOff, None, None, "novo").unwrap();
        assert_eq!(fs::read_to_string(&files[0]).unwrap().lines().count(), 1);
        assert!(files.iter().all(|f| f.exists()));
        assert!(!logs_dir(&d.0).join(format!("{AUDIT_FILE_STEM}.{AUDIT_FILES}.jsonl")).exists());
        let newest = read_in(&d.0, 2);
        assert_eq!(newest[0].kind, AuditKind::EmergencyOff);
        assert_eq!(newest[1].kind, AuditKind::EmergencyOn);
    }

    #[test]
    fn kinds_serialize_as_snake_case() {
        let names: Vec<_> = AuditKind::ALL.iter().map(|k| serde_json::to_value(k).unwrap()).collect();
        assert_eq!(
            names,
            [
                "emergency_on",
                "emergency_off",
                "emergency_expired",
                "commit_allowed_emergency",
                "blocked",
                "unconfirmed",
                "ci_bypass"
            ]
        );
    }
}
