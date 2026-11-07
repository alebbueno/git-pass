//! Removes secrets (GitHub tokens, `Bearer` headers, URL credentials, private keys) and the home
//! folder from text that leaves the app: audit entries, `recent.json` messages and exported logs.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

pub const REDACTED: &str = "[REDIGIDO]";
pub const REDACTED_KEY: &str = "[CHAVE PRIVADA REDIGIDA]";

struct Rule {
    re: Regex,
    replacement: &'static str,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let rule = |re: &str, replacement: &'static str| Rule {
        re: Regex::new(re).expect("valid redaction regex"),
        replacement,
    };
    vec![
        // Unterminated blocks (truncated logs) are redacted up to the end.
        rule(
            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)",
            REDACTED_KEY,
        ),
        rule(r"(?i)\b([a-z][a-z0-9+.-]*://)[^/@\s:]+:[^/@\s]+@", "${1}[REDIGIDO]@"),
        rule(r"(?i)(x-access-token:)[^@\s]*@", "${1}[REDIGIDO]@"),
        rule(r"\bgithub_pat_[A-Za-z0-9_]+", REDACTED),
        rule(r"\bgh[opsur]_[A-Za-z0-9_]+", REDACTED),
        rule(r"(?i)\b(bearer\s+)[A-Za-z0-9._~+/=-]+", "${1}[REDIGIDO]"),
        rule(r"(?i)\b(authorization:\s*token\s+)[A-Za-z0-9._~+/=-]+", "${1}[REDIGIDO]"),
        // `git credential` protocol lines.
        rule(r"(?m)^(\s*password=).*$", "${1}[REDIGIDO]"),
    ]
});

/// `text` with tokens, credentials and private keys replaced by [`REDACTED`] / [`REDACTED_KEY`].
pub fn redact_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for r in RULES.iter() {
        if r.re.is_match(&out) {
            out = r.re.replace_all(&out, r.replacement).into_owned();
        }
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// `text` with the home folder written as `~` (`/Users/ana/x` → `~/x`; `/Users/anabel` is kept).
pub fn redact_home(text: &str, home: Option<&Path>) -> String {
    let Some(home) = home.map(|h| h.to_string_lossy().trim_end_matches('/').to_string()) else {
        return text.to_string();
    };
    if home.len() < 2 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (i, _) in text.match_indices(home.as_str()) {
        if i < last {
            continue;
        }
        let end = i + home.len();
        let before_ok = text[..i].chars().next_back().is_none_or(|c| !is_name_char(c) && c != '/');
        let after_ok = text[end..].chars().next().is_none_or(|c| !is_name_char(c));
        if before_ok && after_ok {
            out.push_str(&text[last..i]);
            out.push('~');
            last = end;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Secrets and home folder removed: what exported files contain.
pub fn redact_for_export(text: &str, home: Option<&Path>) -> String {
    redact_home(&redact_secrets(text), home)
}

/// Short single-line message for the audit log and `recent.json`: redacted and at most `max` chars.
pub fn short_message(text: &str, max: usize) -> String {
    let one_line = redact_secrets(text).split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let mut s: String = one_line.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_tokens_are_redacted() {
        let samples = [
            format!("ghp_{}", "1".repeat(36)),
            "gho_short".to_string(),
            "ghs_server_to_server_token".to_string(),
            "ghu_user_token_xyz".to_string(),
            "ghr_refresh_token".to_string(),
            crate::test_fixtures::github_pat_sample(),
        ];
        for token in samples {
            let s = redact_secrets(&format!("token={token} ok"));
            assert!(!s.contains(&token), "{s}");
            assert!(s.contains(REDACTED), "{s}");
        }
        assert_eq!(redact_secrets("the ghost_town is fine"), "the ghost_town is fine");
    }

    #[test]
    fn headers_and_url_credentials_are_redacted() {
        assert_eq!(redact_secrets("Authorization: Bearer abc.DEF-123"), "Authorization: Bearer [REDIGIDO]");
        assert_eq!(redact_secrets("authorization: token deadbeef42"), "authorization: token [REDIGIDO]");
        assert_eq!(
            redact_secrets("https://x-access-token:s3cr3t@github.com/o/r.git"),
            "https://[REDIGIDO]@github.com/o/r.git"
        );
        assert_eq!(redact_secrets("url x-access-token:s3cr3t@github.com"), "url x-access-token:[REDIGIDO]@github.com");
        assert_eq!(
            redact_secrets("clone https://octo:hunter2@gitlab.com/a/b"),
            "clone https://[REDIGIDO]@gitlab.com/a/b"
        );
        assert_eq!(redact_secrets("protocol=https\npassword=abc\nhost=x"), "protocol=https\npassword=[REDIGIDO]\nhost=x");
        assert_eq!(redact_secrets("git@github.com:a/b.git"), "git@github.com:a/b.git");
    }

    #[test]
    fn private_keys_are_redacted_even_when_truncated() {
        let key = crate::test_fixtures::openssh_private_key_block();
        assert_eq!(redact_secrets(&key), format!("a\n{REDACTED_KEY}\nz"));
        let cut = format!("x -----BEGIN RSA PRIVATE KEY-----\n{}", "MIIEow");
        assert_eq!(redact_secrets(&cut), format!("x {REDACTED_KEY}"));
        let public = "ssh-ed25519 AAAAC3Nza user@host";
        assert_eq!(redact_secrets(public), public);
    }

    #[test]
    fn home_becomes_tilde_only_on_whole_path_components() {
        let home = Path::new("/Users/ana");
        assert_eq!(redact_home("/Users/ana/proj and /Users/ana", Some(home)), "~/proj and ~");
        assert_eq!(redact_home("/Users/anabel/x", Some(home)), "/Users/anabel/x");
        assert_eq!(redact_home("\"/Users/ana/.ssh\"", Some(home)), "\"~/.ssh\"");
        assert_eq!(redact_home("/tmp/Users/ana/x", Some(home)), "/tmp/Users/ana/x");
        assert_eq!(redact_home("x", None), "x");
    }

    #[test]
    fn short_messages_are_single_line_and_bounded() {
        assert_eq!(short_message("a\n  b  gho_x", 100), "a b [REDIGIDO]");
        let long = "é".repeat(50);
        let s = short_message(&long, 10);
        assert_eq!(s.chars().count(), 10);
        assert!(s.ends_with('…'));
    }
}
