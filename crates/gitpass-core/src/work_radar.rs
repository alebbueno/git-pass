//! Work tree radar from `git status --porcelain=v2` (local only, no network).

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkHeadState {
    #[default]
    Branch,
    Detached,
    Empty,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkTreeStatus {
    pub head_state: WorkHeadState,
    pub branch: Option<String>,
    pub detached_at: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub has_upstream: bool,
    pub staged_files: usize,
    pub unstaged_files: usize,
    pub untracked_files: usize,
    pub conflicted_files: usize,
    pub has_pending: bool,
    pub inspected_at: String,
    pub error: Option<String>,
}

impl WorkTreeStatus {
    pub fn unreadable(msg: impl Into<String>) -> Self {
        Self {
            head_state: WorkHeadState::Unreadable,
            inspected_at: now_rfc3339(),
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

impl Default for WorkTreeStatus {
    fn default() -> Self {
        Self {
            head_state: WorkHeadState::default(),
            branch: None,
            detached_at: None,
            upstream: None,
            ahead: None,
            behind: None,
            has_upstream: false,
            staged_files: 0,
            unstaged_files: 0,
            untracked_files: 0,
            conflicted_files: 0,
            has_pending: false,
            inspected_at: String::new(),
            error: None,
        }
    }
}

pub fn inspect_work_tree(root: &Path) -> WorkTreeStatus {
    if !root.is_dir() {
        return WorkTreeStatus::unreadable("A pasta não existe.");
    }
    let out = Command::new("git")
        .current_dir(root)
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "--ahead-behind",
            "-z",
            "--untracked-files=all",
        ])
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return WorkTreeStatus::unreadable(format!("Não foi possível rodar o Git: {e}")),
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return WorkTreeStatus::unreadable(format!(
            "Não foi possível ler o estado do repositório ({}).",
            err.lines().next().unwrap_or("erro")
        ));
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    parse_porcelain_v2_z(&raw)
}

fn now_rfc3339() -> String {
    use std::time::Duration;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64;
    // Good enough for UI freshness (UTC).
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| secs.to_string())
}

fn parse_porcelain_v2_z(text: &str) -> WorkTreeStatus {
    let mut st = WorkTreeStatus {
        inspected_at: now_rfc3339(),
        ..Default::default()
    };
    let mut parts = text.split('\0');
    while let Some(line) = parts.next() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# branch.") {
            parse_branch_header(&mut st, rest);
            continue;
        }
        let kind = line.as_bytes().first().copied().unwrap_or(0);
        match kind {
            b'1' => parse_ordinary_change(&mut st, line, &mut parts),
            b'2' => {
                parse_rename(&mut st, line);
                let _ = parts.next();
            }
            b'u' => {
                st.conflicted_files += 1;
            }
            b'?' => st.untracked_files += 1,
            b'!' => {}
            _ => {}
        }
    }
    finish_pending(&mut st);
    st
}

fn parse_branch_header(st: &mut WorkTreeStatus, rest: &str) {
    if let Some(oid) = rest.strip_prefix("oid ") {
        if oid.is_empty() || oid == "(initial)" {
            st.head_state = WorkHeadState::Empty;
        }
        return;
    }
    if let Some(head) = rest.strip_prefix("head ") {
        let head = head.trim();
        if head == "(detached)" {
            st.head_state = WorkHeadState::Detached;
        } else if head == "(unborn)" {
            st.head_state = WorkHeadState::Empty;
        } else {
            st.head_state = WorkHeadState::Branch;
            st.branch = Some(head.to_string());
        }
        return;
    }
    if let Some(up) = rest.strip_prefix("upstream ") {
        st.upstream = Some(up.to_string());
        st.has_upstream = true;
        return;
    }
    if let Some(ab) = rest.strip_prefix("ab ") {
        let mut it = ab.split_whitespace();
        let plus = it.next().and_then(|s| s.strip_prefix('+')?.parse().ok());
        let minus = it.next().and_then(|s| s.strip_prefix('-')?.parse().ok());
        st.ahead = plus;
        st.behind = minus;
        st.has_upstream = true;
    }
}

fn xy_from_v2_line(line: &str) -> Option<(u8, u8)> {
    let rest = line.strip_prefix('1').or_else(|| line.strip_prefix('2'))?.strip_prefix(' ')?;
    let bytes = rest.as_bytes();
    if bytes.len() < 3 || bytes[2] != b' ' {
        return None;
    }
    Some((bytes[0], bytes[1]))
}

fn parse_ordinary_change(st: &mut WorkTreeStatus, line: &str, parts: &mut std::str::Split<'_, char>) {
    let Some((x, y)) = xy_from_v2_line(line) else {
        return;
    };
    if x == b'R' as u8 || x == b'C' as u8 {
        let _ = parts.next();
    }
    if x != b' ' {
        st.staged_files += 1;
    }
    if y != b' ' {
        st.unstaged_files += 1;
    }
}

fn parse_rename(st: &mut WorkTreeStatus, line: &str) {
    let Some((x, y)) = xy_from_v2_line(line) else {
        return;
    };
    if x != b' ' {
        st.staged_files += 1;
    }
    if y != b' ' {
        st.unstaged_files += 1;
    }
}

fn finish_pending(st: &mut WorkTreeStatus) {
    let dirty = st.staged_files + st.unstaged_files + st.untracked_files + st.conflicted_files > 0;
    let sync = st.ahead.unwrap_or(0) > 0 || st.behind.unwrap_or(0) > 0;
    st.has_pending = dirty || sync || matches!(st.head_state, WorkHeadState::Empty);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_and_dirty_counts() {
        let raw = "# branch.oid abcdef\n\0# branch.head main\n\0# branch.upstream origin/main\n\0\
# branch.ab +2 -1\n\0\
1 M. N... 100644 100644 100644 111 222 path/file.txt\n\0\
1 MM N... 100644 100644 100644 111 222 other.txt\n\0\
? new.txt\n\0";
        let st = parse_porcelain_v2_z(raw);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!((st.ahead, st.behind), (Some(2), Some(1)));
        assert!(st.staged_files >= 1);
        assert!(st.unstaged_files >= 1);
        assert!(st.untracked_files >= 1);
        assert!(st.has_pending);
    }

    #[test]
    fn empty_repo_is_pending() {
        let raw = "# branch.head (unborn)\n\0";
        let st = parse_porcelain_v2_z(raw);
        assert_eq!(st.head_state, WorkHeadState::Empty);
        assert!(st.has_pending);
    }
}
