//! Cloning GitHub repositories with an OAuth token.
//!
//! The token is handed to `git` **only through the environment**
//! (`GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_0`/`GIT_CONFIG_VALUE_0` → `http.extraheader`), so it never shows up in
//! `ps`, in the stored `origin` URL, in `.git/config` or in logs.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// One progress sample parsed from `git clone --progress` (stderr).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CloneProgress {
    /// `counting`, `compressing`, `receiving`, `resolving` or `checkout`.
    pub phase: &'static str,
    /// 0..=100.
    pub percent: u8,
}

/// Parses a single progress line such as `Receiving objects:  45% (450/1000), 1.2 MiB | 2.0 MiB/s`
/// (an optional `remote: ` prefix is accepted). Returns `None` for lines without a percentage.
pub fn parse_progress_line(line: &str) -> Option<CloneProgress> {
    let line = line.trim();
    let line = line.strip_prefix("remote:").unwrap_or(line).trim_start();
    let (label, rest) = line.split_once(':')?;
    let phase = match label.trim() {
        "Counting objects" | "Enumerating objects" => "counting",
        "Compressing objects" => "compressing",
        "Receiving objects" => "receiving",
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
    Some(CloneProgress {
        phase,
        percent: percent.min(100) as u8,
    })
}

/// Splits `owner/repo`, rejecting anything that could escape the destination folder.
pub fn split_full_name(full_name: &str) -> Result<(String, String), String> {
    fn valid(seg: &str) -> bool {
        !seg.is_empty()
            && seg != "."
            && seg != ".."
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    }
    let (owner, repo) = full_name
        .split_once('/')
        .ok_or_else(|| format!("Nome de repositório inválido: `{full_name}` (esperado dono/repositório)."))?;
    if !valid(owner) || !valid(repo) {
        return Err(format!("Nome de repositório inválido: `{full_name}`."));
    }
    Ok((owner.to_string(), repo.to_string()))
}

/// Default clone location: `~/GitHub/<owner>/<repo>`.
pub fn default_clone_dir(owner: &str, repo: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Não foi possível determinar a pasta pessoal.".to_string())?;
    Ok(home.join("GitHub").join(owner).join(repo))
}

/// System locations a clone must never be written into (checked after resolving symlinks).
const FORBIDDEN_CLONE_ROOTS: &[&str] = &[
    "/System",
    "/Library",
    "/bin",
    "/sbin",
    "/usr",
    "/etc",
    "/var",
    "/private/etc",
    "/private/var",
    "/dev",
    "/Applications",
];

/// Validates a clone parent folder that came from the WebView and returns it with symlinks resolved.
///
/// Requires an absolute path without `.`/`..` components; the deepest existing ancestor is
/// canonicalized, so a symlink cannot redirect the clone. Refuses system folders, `~/Library`
/// (LaunchAgents, Application Support, …) and hidden folders directly under the home (`~/.ssh`, …).
pub fn validate_clone_parent(parent: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    use std::path::Component;
    let invalid = || "O destino tem de ser um caminho absoluto, sem «..».".to_string();
    if !parent.is_absolute() || parent.to_string_lossy().contains('\0') {
        return Err(invalid());
    }
    if parent.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(invalid());
    }

    let mut existing = parent.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let resolved = loop {
        match existing.canonicalize() {
            Ok(c) => break c,
            Err(_) => {
                let Some(name) = existing.file_name() else {
                    return Err(invalid());
                };
                rest.push(name.to_os_string());
                if !existing.pop() {
                    return Err(invalid());
                }
            }
        }
    };
    if !resolved.is_dir() {
        return Err(format!("`{}` não é uma pasta.", resolved.display()));
    }
    let full = rest.iter().rev().fold(resolved, |p, n| p.join(n));

    let forbidden = || {
        format!(
            "Não é possível clonar para `{}`. Escolhe uma pasta tua, por exemplo ~/GitHub.",
            parent.display()
        )
    };
    if full == Path::new("/") || FORBIDDEN_CLONE_ROOTS.iter().any(|r| starts_with_ci(&full, Path::new(r))) {
        return Err(forbidden());
    }
    if let Some(home) = home {
        let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
        if starts_with_ci(&full, &home) {
            let first = full
                .components()
                .nth(home.components().count())
                .map(|c| c.as_os_str().to_string_lossy().into_owned());
            if first.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("Library") || f.starts_with('.')) {
                return Err(forbidden());
            }
        }
    }
    Ok(full)
}

/// Component-wise, ASCII case-insensitive `starts_with` (APFS is case-insensitive by default).
fn starts_with_ci(path: &Path, prefix: &Path) -> bool {
    let mut p = path.components();
    prefix.components().all(|c| {
        p.next()
            .is_some_and(|x| x.as_os_str().to_string_lossy().eq_ignore_ascii_case(&c.as_os_str().to_string_lossy()))
    })
}

/// Value of the `Authorization` header used for HTTPS access to github.com with an OAuth token.
fn auth_header_value(token: &str) -> String {
    format!("AUTHORIZATION: basic {}", B64.encode(format!("x-access-token:{token}")))
}

fn is_github_https(url: &str) -> bool {
    url.starts_with("https://github.com/") && !url.contains('@') && !url.contains(char::is_whitespace)
}

/// Turns raw `git clone` stderr into an actionable pt-PT message.
pub fn friendly_clone_error(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    let last = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("erro desconhecido");
    if lower.contains("repository not found") || (lower.contains("not found") && lower.contains("fatal")) {
        "Repositório não encontrado, ou esta conta não tem acesso. Confirma as permissões da conta GitHub.".to_string()
    } else if lower.contains("authentication failed")
        || lower.contains("invalid username or password")
        || lower.contains("could not read username")
        || lower.contains("bad credentials")
        || lower.contains("requested url returned error: 401")
        || lower.contains("requested url returned error: 403")
    {
        "A autenticação no GitHub falhou. Volta a ligar a conta em «Contas» e tenta de novo.".to_string()
    } else if lower.contains("could not resolve host")
        || lower.contains("failed to connect")
        || lower.contains("connection timed out")
        || lower.contains("network is unreachable")
    {
        "Sem ligação ao GitHub. Verifica a tua ligação à internet e tenta de novo.".to_string()
    } else if lower.contains("no space left") {
        "Sem espaço em disco para clonar este repositório.".to_string()
    } else {
        format!("Falha ao clonar: {last}")
    }
}

fn redact(text: &str, token: &str) -> String {
    let mut out = text.to_string();
    for secret in [token.to_string(), B64.encode(format!("x-access-token:{token}"))] {
        if !secret.is_empty() {
            out = out.replace(&secret, "***");
        }
    }
    out
}

/// Clones `clone_url` (must be `https://github.com/...`) into `dest_dir` using `token`.
///
/// `dest_dir` is the final repository folder (e.g. `~/GitHub/owner/repo`); missing parents are created. It must not
/// exist yet (an empty folder is accepted). `progress_cb` is called when the phase or percentage changes.
pub fn clone_repo(
    token: &str,
    clone_url: &str,
    dest_dir: &Path,
    mut progress_cb: impl FnMut(CloneProgress),
) -> Result<(), String> {
    if !is_github_https(clone_url) {
        return Err(format!("URL de clonagem não suportado: `{clone_url}` (só https://github.com/…)."));
    }
    if token.trim().is_empty() {
        return Err("Esta conta não tem token. Liga a conta GitHub em «Contas».".to_string());
    }

    if std::fs::symlink_metadata(dest_dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "`{}` é um atalho (symlink). Escolhe outro destino.",
            dest_dir.display()
        ));
    }
    if dest_dir.exists() {
        let empty_dir = dest_dir.is_dir()
            && std::fs::read_dir(dest_dir)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false);
        if !empty_dir {
            return Err(format!(
                "A pasta `{}` já existe. Escolhe outro destino ou remove-a primeiro.",
                dest_dir.display()
            ));
        }
    }
    if let Some(parent) = dest_dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Não foi possível criar a pasta `{}`: {e}", parent.display()))?;
    }

    let mut child = Command::new("git")
        .args(["clone", "--progress", "--", clone_url])
        .arg(dest_dir)
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
        .env("GIT_CONFIG_VALUE_0", auth_header_value(token))
        // Empty helper resets every configured helper (global and `credential.<url>.helper`): if the
        // token is rejected, Git must not fall back to another account's credentials, nor erase them.
        .env("GIT_CONFIG_KEY_1", "credential.helper")
        .env("GIT_CONFIG_VALUE_1", "")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG_PARAMETERS")
        // Progress/error parsing relies on English output.
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "O Git não está instalado. Instala as Xcode Command Line Tools (`xcode-select --install`).".to_string()
            } else {
                format!("Não foi possível executar o git: {e}")
            }
        })?;

    let mut stderr = child.stderr.take().ok_or_else(|| "Sem acesso à saída do git.".to_string())?;

    let mut other_lines: Vec<String> = Vec::new();
    let mut last: Option<CloneProgress> = None;
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];

    let mut handle_line = |bytes: &[u8], cb: &mut dyn FnMut(CloneProgress)| {
        if bytes.is_empty() {
            return;
        }
        let line = String::from_utf8_lossy(bytes);
        if let Some(p) = parse_progress_line(&line) {
            if last.as_ref() != Some(&p) {
                last = Some(p.clone());
                cb(p);
            }
        } else if !line.trim().is_empty() && !line.starts_with("Cloning into") {
            other_lines.push(line.trim().to_string());
            if other_lines.len() > 40 {
                other_lines.remove(0);
            }
        }
    };

    loop {
        let n = match stderr.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                handle_line(&pending, &mut progress_cb);
                pending.clear();
            } else {
                pending.push(b);
            }
        }
    }
    handle_line(&pending, &mut progress_cb);

    let status = child.wait().map_err(|e| format!("Falha ao aguardar o git: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        let text = redact(&other_lines.join("\n"), token);
        Err(friendly_clone_error(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(phase: &'static str, percent: u8) -> Option<CloneProgress> {
        Some(CloneProgress { phase, percent })
    }

    #[test]
    fn parses_receiving_and_resolving() {
        assert_eq!(
            parse_progress_line("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s"),
            p("receiving", 45)
        );
        assert_eq!(
            parse_progress_line("Receiving objects: 100% (1000/1000), 3.4 MiB | 2.0 MiB/s, done."),
            p("receiving", 100)
        );
        assert_eq!(parse_progress_line("Resolving deltas:   7% (7/100)"), p("resolving", 7));
    }

    #[test]
    fn parses_remote_prefixed_lines() {
        assert_eq!(
            parse_progress_line("remote: Counting objects: 100% (10/10), done."),
            p("counting", 100)
        );
        assert_eq!(
            parse_progress_line("remote: Compressing objects:  50% (5/10)"),
            p("compressing", 50)
        );
    }

    #[test]
    fn parses_checkout() {
        assert_eq!(parse_progress_line("Updating files:  12% (12/100)"), p("checkout", 12));
    }

    #[test]
    fn ignores_non_progress_lines() {
        assert_eq!(parse_progress_line("Cloning into 'repo'..."), None);
        assert_eq!(parse_progress_line("remote: Enumerating objects: 100, done."), None);
        assert_eq!(parse_progress_line("fatal: repository not found"), None);
        assert_eq!(parse_progress_line("Receiving objects: abc%"), None);
        assert_eq!(parse_progress_line(""), None);
    }

    #[test]
    fn percent_is_clamped() {
        assert_eq!(parse_progress_line("Receiving objects: 250% (1/1)"), p("receiving", 100));
    }

    #[test]
    fn full_name_validation() {
        assert_eq!(split_full_name("octo/hello-world.rs").unwrap(), ("octo".into(), "hello-world.rs".into()));
        assert!(split_full_name("octo").is_err());
        assert!(split_full_name("../etc").is_err());
        assert!(split_full_name("octo/..").is_err());
        assert!(split_full_name("octo/a b").is_err());
        assert!(split_full_name("octo/a/b").is_err());
    }

    #[test]
    fn auth_header_is_basic_x_access_token() {
        let h = auth_header_value("abc");
        assert_eq!(h, format!("AUTHORIZATION: basic {}", B64.encode("x-access-token:abc")));
    }

    #[test]
    fn only_clean_github_https_urls_are_accepted() {
        assert!(is_github_https("https://github.com/o/r.git"));
        assert!(!is_github_https("https://tok@github.com/o/r.git"));
        assert!(!is_github_https("git@github.com:o/r.git"));
        assert!(!is_github_https("https://evil.com/o/r.git"));
    }

    #[test]
    fn errors_are_friendly() {
        assert!(friendly_clone_error("remote: Repository not found.\nfatal: repository 'x' not found").contains("não encontrado"));
        assert!(friendly_clone_error("fatal: Authentication failed for 'https://github.com/o/r.git/'").contains("autenticação"));
        assert!(friendly_clone_error("fatal: unable to access: Could not resolve host: github.com").contains("ligação"));
        assert!(friendly_clone_error("something odd").starts_with("Falha ao clonar: something odd"));
    }

    #[test]
    fn redact_hides_token() {
        let t = "gho_secret";
        let text = format!("oops {t} and {}", B64.encode(format!("x-access-token:{t}")));
        let r = redact(&text, t);
        assert!(!r.contains(t) && !r.contains(&B64.encode(format!("x-access-token:{t}"))));
    }

    /// Under `/tmp`: macOS's per-user temp dir lives in `/private/var`, which clones refuse.
    fn temp_dir(tag: &str) -> PathBuf {
        let d = PathBuf::from("/tmp").join(format!("gitpass-clone-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn clone_parent_must_be_absolute_without_dot_dot() {
        assert!(validate_clone_parent(Path::new("GitHub"), None).is_err());
        assert!(validate_clone_parent(Path::new("/Users/x/../../etc"), None).is_err());
        assert!(validate_clone_parent(Path::new("/Users/x/.."), None).is_err());
    }

    #[test]
    fn clone_parent_refuses_system_library_and_hidden_home_dirs() {
        let home = temp_dir("home");
        std::fs::create_dir_all(home.join("Library/LaunchAgents")).unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join("Code")).unwrap();
        let h = Some(home.as_path());
        for bad in ["/", "/System/x", "/usr/local", "/etc", "/Library/LaunchDaemons", "/applications"] {
            assert!(validate_clone_parent(Path::new(bad), h).is_err(), "{bad}");
        }
        assert!(validate_clone_parent(&home.join("Library/LaunchAgents"), h).is_err());
        assert!(validate_clone_parent(&home.join("library/new"), h).is_err());
        assert!(validate_clone_parent(&home.join(".ssh"), h).is_err());
        assert_eq!(validate_clone_parent(&home.join("Code"), h).unwrap(), home.join("Code"));
        // Missing folders are fine (created by the clone).
        assert_eq!(validate_clone_parent(&home.join("Code/new/sub"), h).unwrap(), home.join("Code/new/sub"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn clone_parent_symlinks_are_resolved_before_checks() {
        let home = temp_dir("home-link");
        std::fs::create_dir_all(home.join("Library/LaunchAgents")).unwrap();
        std::os::unix::fs::symlink(home.join("Library/LaunchAgents"), home.join("innocent")).unwrap();
        assert!(validate_clone_parent(&home.join("innocent"), Some(&home)).is_err());
        assert!(validate_clone_parent(&home.join("innocent/deeper"), Some(&home)).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dest_is_rejected() {
        let d = temp_dir("dest-link");
        std::fs::create_dir_all(d.join("target")).unwrap();
        std::os::unix::fs::symlink(d.join("target"), d.join("repo")).unwrap();
        let err = clone_repo("tok", "https://github.com/o/r.git", &d.join("repo"), |_| {}).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn existing_non_empty_dest_is_rejected() {
        let dir = std::env::temp_dir().join(format!("gitpass-clone-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f"), "x").unwrap();
        let err = clone_repo("tok", "https://github.com/o/r.git", &dir, |_| {}).unwrap_err();
        assert!(err.contains("já existe"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
