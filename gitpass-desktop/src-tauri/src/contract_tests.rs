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

#[test]
fn helper_parsing_works() {
    assert_eq!(camel("take_over_hooks"), "takeOverHooks");
    assert_eq!(split_top_level("a: Option<Vec<String>>, b: (u8, u8), c"), vec!["a: Option<Vec<String>>", "b: (u8, u8)", "c"]);
}
