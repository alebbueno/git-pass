//! Contract tests between the Tauri commands and the TypeScript frontend (`src/**/*.ts`).
//!
//! * every `invoke("cmd", { args })` in the frontend targets a command registered in `generate_handler!`,
//!   and its argument names are the camelCase versions of the Rust parameters;
//! * the JSON keys of response structs (tests next to each struct, using [`keys`]) match the TS types.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Sorted top-level JSON keys of `v` (must serialize to an object).
pub fn keys<T: Serialize>(v: &T) -> Vec<String> {
    let json = serde_json::to_value(v).expect("serialize");
    let mut k: Vec<String> = json.as_object().expect("JSON object").keys().cloned().collect();
    k.sort();
    k
}

/// Builds the expected sorted key list from a literal.
pub fn expect(list: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

fn camel(snake: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in snake.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Splits `s` on commas that are not nested inside `<>`, `()`, `[]` or `{}`.
fn split_top_level(s: &str) -> Vec<String> {
    let (mut depth, mut cur, mut out) = (0i32, String::new(), Vec::new());
    let mut prev = ' ';
    for c in s.chars() {
        match c {
            '<' | '(' | '[' | '{' => depth += 1,
            // `=>` and `->` are not closing brackets.
            '>' if prev == '=' || prev == '-' => {}
            '>' | ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        prev = c;
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

/// Index just past the bracket that closes the one at `open` (which must be `(`, `{` or `<`).
fn matching(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let (o, c) = match b[open] {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        _ => (b'<', b'>'),
    };
    let mut depth = 0;
    let mut i = open;
    while i < b.len() {
        if b[i] == o {
            depth += 1;
        } else if b[i] == c && !(c == b'>' && i > 0 && (b[i - 1] == b'=' || b[i - 1] == b'-')) {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
        i += 1;
    }
    panic!("unbalanced brackets in: {}", &s[open..s.len().min(open + 80)]);
}

struct RustCommand {
    /// camelCase argument name → required (`false` for `Option<_>`).
    args: Vec<(String, bool)>,
}

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_commands() -> Vec<(String, RustCommand)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(src_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") || path.ends_with("contract_tests.rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("#[tauri::command]") {
            rest = &rest[i + "#[tauri::command]".len()..];
            let f = rest.find("fn ").expect("fn after #[tauri::command]");
            let after = &rest[f + 3..];
            let paren = after.find('(').unwrap();
            let name = after[..paren].trim().to_string();
            let end = matching(after, paren);
            let params = &after[paren + 1..end - 1];
            let args = split_top_level(params)
                .into_iter()
                .filter_map(|p| {
                    let (n, ty) = p.split_once(':')?;
                    let ty = ty.trim();
                    // Injected by Tauri, not sent by the frontend.
                    if ty == "AppHandle" || ty.starts_with("State<") || ty.starts_with("Window") || ty.starts_with("WebviewWindow") {
                        return None;
                    }
                    Some((camel(n.trim().trim_start_matches("mut ")), !ty.starts_with("Option<")))
                })
                .collect();
            out.push((name, RustCommand { args }));
            rest = &after[end..];
        }
    }
    out
}

fn registered_commands() -> BTreeSet<String> {
    let main = fs::read_to_string(src_dir().join("main.rs")).unwrap();
    let start = main.find("generate_handler![").expect("generate_handler!") + "generate_handler![".len();
    let body = &main[start..start + main[start..].find(']').unwrap()];
    body.split(',')
        .map(|s| s.trim().rsplit("::").next().unwrap().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

struct TsInvoke {
    file: String,
    command: String,
    /// `None` when the call passes no argument object.
    args: Option<BTreeSet<String>>,
}

fn ts_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            ts_files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("ts") {
            out.push(p);
        }
    }
}

fn ts_invokes() -> Vec<TsInvoke> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src");
    let mut files = Vec::new();
    ts_files(&root, &mut files);
    let mut out = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("invoke") {
            rest = &rest[i + "invoke".len()..];
            // Skip the import line (`import { invoke } from ...`).
            let mut s = rest;
            if s.starts_with('<') {
                s = &s[matching(s, 0)..];
            }
            let Some(s) = s.strip_prefix("(\"") else { continue };
            let end = s.find('"').unwrap();
            let command = s[..end].to_string();
            let after = s[end + 1..].trim_start();
            let args = after.strip_prefix(',').map(|a| a.trim_start()).filter(|a| a.starts_with('{')).map(|a| {
                let close = matching(a, 0);
                split_top_level(&a[1..close - 1])
                    .into_iter()
                    .map(|kv| kv.split(':').next().unwrap().trim().to_string())
                    .collect::<BTreeSet<_>>()
            });
            out.push(TsInvoke {
                file: name.clone(),
                command,
                args,
            });
        }
    }
    out
}

#[test]
fn every_tauri_command_is_registered() {
    let registered = registered_commands();
    for (name, _) in rust_commands() {
        assert!(registered.contains(&name), "`{name}` has #[tauri::command] but is not in generate_handler!");
    }
    let defined: BTreeSet<String> = rust_commands().into_iter().map(|(n, _)| n).collect();
    for name in &registered {
        assert!(defined.contains(name), "generate_handler! lists `{name}` but no such command exists");
    }
}

#[test]
fn frontend_invokes_match_rust_commands() {
    let registered = registered_commands();
    let commands = rust_commands();
    let invokes = ts_invokes();
    assert!(invokes.len() >= 25, "expected to find the frontend invoke() calls, found {}", invokes.len());
    for call in invokes {
        assert!(
            registered.contains(&call.command),
            "{}: invoke(\"{}\") targets a command that does not exist / is not registered",
            call.file,
            call.command
        );
        let (_, cmd) = commands.iter().find(|(n, _)| *n == call.command).unwrap();
        let passed = call.args.clone().unwrap_or_default();
        for (arg, required) in &cmd.args {
            if *required {
                assert!(
                    passed.contains(arg),
                    "{}: invoke(\"{}\") is missing required argument `{arg}`",
                    call.file,
                    call.command
                );
            }
        }
        for arg in &passed {
            assert!(
                cmd.args.iter().any(|(a, _)| a == arg),
                "{}: invoke(\"{}\") passes `{arg}`, which the Rust command does not take (expected: {:?})",
                call.file,
                call.command,
                cmd.args.iter().map(|(a, _)| a.as_str()).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn removed_commands_are_not_referenced() {
    const REMOVED: &[&str] = &[
        "list_accounts",
        "github_device_oauth_start",
        "github_device_oauth_poll",
        "github_cloud_repos_for_account",
        "bind_repo_dialog",
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src");
    let mut files = Vec::new();
    ts_files(&root, &mut files);
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        for r in REMOVED {
            assert!(!text.contains(r), "{} still references removed command `{r}`", path.display());
        }
    }
}

/// api-repos.ts: MapResult, MappedProject, MapCandidate, MapAccountStatus, MapProgress, MapStatus, RepoPermission.
#[test]
fn map_folder_json_matches_typescript() {
    use crate::cmd_repos::{MapAccountRow, MapProgress, MapResult, MAP_PROGRESS_EVENT};
    use gitpass_core::repo_map::{MapCandidate, MapStatus, MappedProject, Permission};
    use uuid::Uuid;

    let candidate = MapCandidate { account_id: Uuid::nil(), permission: Permission::Push };
    let item = MappedProject {
        root_path: "/tmp/x".into(),
        status: MapStatus::ReadOnly,
        remote_name: Some("origin".into()),
        remote_url: None,
        github_owner: None,
        github_repo: None,
        candidates: vec![candidate],
        suggested_account_id: Some(Uuid::nil()),
        binding_id: None,
        bound_account_id: None,
        bound_matches_suggestion: None,
        suggestion_reason: None,
        suggestion_label: None,
    };
    assert_eq!(
        keys(&item),
        expect(&[
            "root_path", "status", "remote_name", "remote_url", "github_owner", "github_repo", "candidates",
            "suggested_account_id", "binding_id", "bound_account_id", "bound_matches_suggestion", "suggestion_reason",
            "suggestion_label"
        ])
    );
    let v = serde_json::to_value(&item).unwrap();
    assert_eq!(v["status"], "read_only");
    assert_eq!(v["suggested_account_id"], Uuid::nil().to_string());
    assert_eq!(keys(&candidate), expect(&["account_id", "permission"]));
    assert_eq!(v["candidates"][0]["permission"], "push");

    let account = MapAccountRow { account_id: String::new(), ok: false, error: Some("x".into()), repo_count: 0 };
    assert_eq!(keys(&account), expect(&["account_id", "ok", "error", "repo_count"]));
    let result = MapResult {
        root: String::new(),
        items: vec![item],
        accounts: vec![account],
        truncated: false,
        limit_reached: false,
        max_projects: 1,
        elapsed_ms: 0,
    };
    assert_eq!(
        keys(&result),
        expect(&["root", "items", "accounts", "truncated", "limit_reached", "max_projects", "elapsed_ms"])
    );
    assert_eq!(keys(&MapProgress { phase: "scanning", found: 3 }), expect(&["phase", "found"]));

    let statuses: Vec<_> = [
        MapStatus::Bound,
        MapStatus::Match,
        MapStatus::ReadOnly,
        MapStatus::NotFound,
        MapStatus::OtherRemote,
        MapStatus::NoRemote,
    ]
    .iter()
    .map(|s| serde_json::to_value(s).unwrap())
    .collect();
    assert_eq!(statuses, ["bound", "match", "read_only", "not_found", "other_remote", "no_remote"]);
    let perms: Vec<_> = [Permission::Admin, Permission::Maintain, Permission::Push, Permission::Triage, Permission::Pull]
        .iter()
        .map(|p| serde_json::to_value(p).unwrap())
        .collect();
    assert_eq!(perms, ["admin", "maintain", "push", "triage", "pull"]);

    let ts = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/api-repos.ts")).unwrap();
    assert!(ts.contains(&format!("\"{MAP_PROGRESS_EVENT}\"")), "api-repos.ts must listen to `{MAP_PROGRESS_EVENT}`");
}

fn api_repos_ts() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/api-repos.ts")).unwrap()
}

/// `text` without `/* */` and `//` comments.
fn strip_ts_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        rest = &rest[i + 2..];
        rest = &rest[rest.find("*/").map(|j| j + 2).unwrap_or(rest.len())..];
    }
    out.push_str(rest);
    out.lines().map(|l| l.split(" //").next().unwrap()).collect::<Vec<_>>().join("\n")
}

/// Right-hand side of `export type <name> = ...;` (comments removed).
fn ts_type(ts: &str, name: &str) -> String {
    let ts = strip_ts_comments(ts);
    let head = format!("export type {name} =");
    let start = ts.find(&head).unwrap_or_else(|| panic!("api-repos.ts has no `{head}`")) + head.len();
    let body = ts[start..].trim_start();
    if body.starts_with('{') {
        return body[..matching(body, 0)].to_string();
    }
    body[..body.find(';').unwrap()].to_string()
}

/// `field → type` of a TS object type `{ a: T; b?: U }` (only the top level).
fn ts_fields(object: &str) -> Vec<(String, String)> {
    let inner = &object[1..object.len() - 1];
    let (mut depth, mut cur, mut parts) = (0i32, String::new(), Vec::new());
    for c in inner.chars() {
        match c {
            '{' | '(' | '[' | '<' => depth += 1,
            '}' | ')' | ']' | '>' => depth -= 1,
            _ => {}
        }
        if c == ';' && depth == 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    parts.push(cur);
    parts
        .into_iter()
        .filter_map(|p| {
            let (k, t) = p.split_once(':')?;
            Some((k.trim().trim_end_matches('?').to_string(), t.trim().to_string()))
        })
        .collect()
}

fn ts_keys(ts: &str, name: &str) -> Vec<String> {
    let mut k: Vec<String> = ts_fields(&ts_type(ts, name)).into_iter().map(|(k, _)| k).collect();
    k.sort();
    k
}

/// String literals of a TS union such as `"a" | "b"`.
fn ts_literals(ty: &str) -> Vec<String> {
    ty.split('"').skip(1).step_by(2).map(String::from).collect()
}

fn ts_field_literals(ts: &str, name: &str, field: &str) -> Vec<String> {
    let fields = ts_fields(&ts_type(ts, name));
    let (_, ty) = fields.iter().find(|(k, _)| k == field).unwrap_or_else(|| panic!("{name}.{field} not in api-repos.ts"));
    ts_literals(ty)
}

fn json_strings<T: Serialize>(values: &[T]) -> Vec<String> {
    values.iter().map(|v| serde_json::to_value(v).unwrap().as_str().unwrap().to_string()).collect()
}

/// api-repos.ts: RepoSyncSummary, CheckSeverity, PushCheck, OutgoingCommit, OutgoingState, CommitFile,
/// PushResult, PushProgress and the push events, read from the TS source itself.
#[test]
fn push_json_matches_typescript() {
    use crate::cmd_push::{OUTGOING_CHANGED_EVENT, PUSH_PROGRESS_EVENT};
    use gitpass_core::outgoing::{
        CheckCode, CheckSeverity, CommitFile, DirtyCounts, FileStatus, OutgoingCommit, OutgoingState, PushCheck,
        PushPhase, PushProgress, PushResult, SyncSummary,
    };

    let ts = api_repos_ts();
    let sync = SyncSummary { ahead: 1, behind: 2, has_upstream: true, dirty: false };
    assert_eq!(keys(&sync), ts_keys(&ts, "RepoSyncSummary"));

    let check = PushCheck {
        code: CheckCode::AuthorMismatch,
        severity: CheckSeverity::Error,
        message: String::new(),
        sha: Some("abc".into()),
        path: None,
    };
    assert_eq!(keys(&check), ts_keys(&ts, "PushCheck"));
    assert_eq!(json_strings(&CheckCode::ALL), ts_field_literals(&ts, "PushCheck", "code"));
    assert_eq!(
        json_strings(&[CheckSeverity::Error, CheckSeverity::Warning, CheckSeverity::Info]),
        ts_literals(&ts_type(&ts, "CheckSeverity"))
    );

    let commit = OutgoingCommit {
        sha: String::new(),
        short_sha: String::new(),
        subject: String::new(),
        body: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at: String::new(),
        committer_name: String::new(),
        committer_email: String::new(),
        files_changed: 0,
        insertions: 0,
        deletions: 0,
        is_merge: false,
        checks: vec![check.clone()],
    };
    assert_eq!(keys(&commit), ts_keys(&ts, "OutgoingCommit"));

    let state = OutgoingState {
        binding_id: String::new(),
        branch: None,
        remote_name: None,
        upstream: None,
        remote_branch_url: None,
        ahead: 0,
        behind: 0,
        fetched: false,
        dirty: DirtyCounts { staged: 0, unstaged: 0, untracked: 0 },
        commits: vec![commit],
        truncated: false,
        checks: vec![check],
        head_sha: None,
        can_push: false,
        has_warnings: false,
        can_fix_authors: false,
        can_pull: false,
    };
    assert_eq!(keys(&state), ts_keys(&ts, "OutgoingState"));
    let dirty_ts = ts_fields(&ts_type(&ts, "OutgoingState")).into_iter().find(|(k, _)| k == "dirty").unwrap().1;
    let mut dirty_keys: Vec<String> = ts_fields(&dirty_ts).into_iter().map(|(k, _)| k).collect();
    dirty_keys.sort();
    assert_eq!(keys(&state.dirty), dirty_keys);

    let file = CommitFile {
        path: String::new(),
        old_path: None,
        status: FileStatus::Added,
        insertions: None,
        deletions: None,
        binary: true,
        size_bytes: None,
    };
    assert_eq!(keys(&file), ts_keys(&ts, "CommitFile"));
    let statuses = [
        FileStatus::Added,
        FileStatus::Modified,
        FileStatus::Deleted,
        FileStatus::Renamed,
        FileStatus::Copied,
        FileStatus::TypeChanged,
    ];
    assert_eq!(json_strings(&statuses), ts_field_literals(&ts, "CommitFile", "status"));

    let result = PushResult {
        pushed: 1,
        branch: String::new(),
        created_upstream: false,
        remote_branch_url: None,
        summary: String::new(),
    };
    assert_eq!(keys(&result), ts_keys(&ts, "PushResult"));

    let progress = PushProgress { binding_id: String::new(), phase: PushPhase::Writing, percent: Some(3) };
    assert_eq!(keys(&progress), ts_keys(&ts, "PushProgress"));
    assert_eq!(json_strings(&PushPhase::ALL), ts_field_literals(&ts, "PushProgress", "phase"));

    for (name, event) in [("PUSH_PROGRESS_EVENT", PUSH_PROGRESS_EVENT), ("OUTGOING_CHANGED_EVENT", OUTGOING_CHANGED_EVENT)] {
        assert!(
            ts.contains(&format!("export const {name} = \"{event}\"")),
            "api-repos.ts must export {name} = \"{event}\""
        );
    }
}

/// api-repos.ts: BranchInfo, MergeMode, MergeStrategy, MergePreview, MergeResult.
#[test]
fn branches_json_matches_typescript() {
    use gitpass_core::branches::{BranchInfo, MergeMode, MergePreview, MergeResult, MergeStrategy};
    use gitpass_core::outgoing::{CheckCode, CheckSeverity, PushCheck};

    let ts = api_repos_ts();
    let branch = BranchInfo {
        name: "main".into(),
        current: true,
        is_default: true,
        upstream: Some("origin/main".into()),
        ahead: Some(0),
        behind: None,
        worktree_path: None,
        head_sha: String::new(),
        last_commit_at: String::new(),
        last_commit_subject: String::new(),
        remote_only: false,
    };
    assert_eq!(keys(&branch), ts_keys(&ts, "BranchInfo"));
    assert_eq!(json_strings(&MergeMode::ALL), ts_literals(&ts_type(&ts, "MergeMode")));
    assert_eq!(json_strings(&MergeStrategy::ALL), ts_literals(&ts_type(&ts, "MergeStrategy")));

    let preview = MergePreview {
        binding_id: String::new(),
        source: String::new(),
        target: String::new(),
        mode: MergeMode::CherryPick,
        commits: vec![],
        selected: vec![],
        strategy: MergeStrategy::NewCommits,
        conflicts: vec!["a.txt".into()],
        files_changed: 1,
        insertions: 1,
        deletions: 0,
        checks: vec![PushCheck {
            code: CheckCode::MergeConflict,
            severity: CheckSeverity::Error,
            message: String::new(),
            sha: None,
            path: None,
        }],
        can_apply: false,
        source_sha: String::new(),
        target_sha: String::new(),
        applies_in_worktree: false,
    };
    assert_eq!(keys(&preview), ts_keys(&ts, "MergePreview"));
    let v = serde_json::to_value(&preview).unwrap();
    assert_eq!((v["mode"].as_str(), v["strategy"].as_str()), (Some("cherry_pick"), Some("new_commits")));
    assert_eq!(v["checks"][0]["code"], "merge_conflict");

    let result = MergeResult {
        target: String::new(),
        new_target_sha: String::new(),
        strategy: MergeStrategy::FastForward,
        created: 0,
        ahead_after: 1,
    };
    assert_eq!(keys(&result), ts_keys(&ts, "MergeResult"));
}

fn api_system_ts() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/api-system.ts")).unwrap()
}

/// Identifiers in backticks on the first line of `text` containing `marker`.
fn backticked_after(text: &str, marker: &str) -> Vec<String> {
    let line = text.lines().find(|l| l.contains(marker)).unwrap_or_else(|| panic!("no line with {marker:?}"));
    let mut v: Vec<String> = line.split('`').skip(1).step_by(2).map(String::from).collect();
    v.sort();
    v
}

fn sorted(v: &[&str]) -> Vec<String> {
    expect(v)
}

/// api-system.ts: CurrentRepo, CurrentRepoState, EmergencyState, AuditEntry, DiagnosticItem,
/// Diagnostics, their string unions, the suggested actions / item ids and the events.
#[test]
fn system_json_matches_typescript() {
    use crate::cmd_system::{CURRENT_REPO_CHANGED_EVENT, EMERGENCY_CHANGED_EVENT};
    use gitpass_core::audit::{AuditEntry, AuditKind};
    use gitpass_core::current_repo::{CurrentRepo, CurrentRepoSource, CurrentRepoState};
    use gitpass_core::diagnostics::{DiagnosticItem, DiagnosticState, Diagnostics, ACTIONS, ITEM_IDS};
    use gitpass_core::emergency::EmergencyView;
    use gitpass_core::recent::Verdict;

    let ts = api_system_ts();
    let repo = CurrentRepo {
        binding_id: None,
        root_path: String::new(),
        full_name: None,
        account_id: None,
        source: CurrentRepoSource::Focus,
        at: String::new(),
        verdict: Some(Verdict::Blocked),
        code: None,
        message: None,
    };
    assert_eq!(keys(&repo), ts_keys(&ts, "CurrentRepo"));
    let state = CurrentRepoState {
        current: Some(repo.clone()),
        pinned_binding_id: None,
        recent: vec![repo],
        accessibility_granted: false,
    };
    assert_eq!(keys(&state), ts_keys(&ts, "CurrentRepoState"));
    assert_eq!(json_strings(&CurrentRepoSource::ALL), ts_literals(&ts_type(&ts, "CurrentRepoSource")));
    assert_eq!(json_strings(&Verdict::ALL), ts_literals(&ts_type(&ts, "ValidationVerdict")));

    let em = EmergencyView { active: false, until: None, started_at: None, reason: None };
    assert_eq!(keys(&em), ts_keys(&ts, "EmergencyState"));

    let entry = AuditEntry { at: String::new(), kind: AuditKind::CiBypass, repo: None, code: None, message: String::new() };
    assert_eq!(keys(&entry), ts_keys(&ts, "AuditEntry"));
    assert_eq!(json_strings(&AuditKind::ALL), ts_literals(&ts_type(&ts, "AuditKind")));

    let item = DiagnosticItem {
        id: String::new(),
        label: String::new(),
        state: DiagnosticState::Info,
        detail: String::new(),
        code: None,
        action: None,
    };
    assert_eq!(keys(&item), ts_keys(&ts, "DiagnosticItem"));
    assert_eq!(json_strings(&DiagnosticState::ALL), ts_literals(&ts_type(&ts, "DiagnosticState")));
    let diag = Diagnostics {
        generated_at: String::new(),
        app_version: String::new(),
        macos_version: None,
        git_version: None,
        items: vec![item],
    };
    assert_eq!(keys(&diag), ts_keys(&ts, "Diagnostics"));
    assert_eq!(sorted(&ACTIONS), backticked_after(&ts, "Ação sugerida"));
    assert_eq!(sorted(&ITEM_IDS), backticked_after(&ts, "Ex.: `daemon`"));

    for (name, event) in [
        ("CURRENT_REPO_CHANGED_EVENT", CURRENT_REPO_CHANGED_EVENT),
        ("EMERGENCY_CHANGED_EVENT", EMERGENCY_CHANGED_EVENT),
    ] {
        assert!(
            ts.contains(&format!("export const {name} = \"{event}\"")),
            "api-system.ts must export {name} = \"{event}\""
        );
    }
}

#[test]
fn ts_type_parsing_works() {
    let ts = "/** doc */\nexport type A = {\n  /** x */\n  a: string;\n  b?: { c: number; d: number };\n  e: \"x\" | \"y\";\n};\nexport type B = \"p\" | \"q\";\n";
    assert_eq!(ts_keys(ts, "A"), expect(&["a", "b", "e"]));
    assert_eq!(ts_field_literals(ts, "A", "e"), ["x", "y"]);
    assert_eq!(ts_literals(&ts_type(ts, "B")), ["p", "q"]);
}

#[test]
fn helper_parsing_works() {
    assert_eq!(camel("take_over_hooks"), "takeOverHooks");
    assert_eq!(split_top_level("a: Option<Vec<String>>, b: (u8, u8), c"), vec!["a: Option<Vec<String>>", "b: (u8, u8)", "c"]);
}
