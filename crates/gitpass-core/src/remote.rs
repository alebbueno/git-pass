//! Parse GitHub `origin` URLs into owner/repo.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepoId {
    pub owner: String,
    pub repo: String,
}

/// Normalize owner segment (GitHub usernames are case-insensitive; treat lower).
pub fn normalize_owner(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

fn strip_git_suffix(name: &str) -> String {
    name.trim_end_matches(".git").to_string()
}

/// Parse SSH (`git@github.com:org/repo` or `ssh://git@github.com/org/repo`) and HTTPS URLs.
pub fn parse_github_remote(url: &str) -> Option<GitHubRepoId> {
    let u = url.trim();

    // git@github.com:owner/repo.git
    if let Some(rest) = u.strip_prefix("git@github.com:") {
        let mut parts = rest.splitn(2, '/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some(GitHubRepoId {
            owner: normalize_owner(owner),
            repo: strip_git_suffix(repo),
        });
    }

    // ssh://git@github.com/owner/repo.git
    if u.starts_with("ssh://git@github.com/") {
        let path = u.trim_start_matches("ssh://git@github.com/");
        let mut parts = path.splitn(2, '/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some(GitHubRepoId {
            owner: normalize_owner(owner),
            repo: strip_git_suffix(repo),
        });
    }

    // https://github.com/owner/repo.git
    // https://github.com/org/repo
    if let Some(idx) = u.find("github.com/") {
        let tail = &u[idx + "github.com/".len()..];
        let mut parts = tail.splitn(2, '/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim();
        // drop query/fragment if present
        let repo = repo.split(['?', '#']).next().unwrap_or(repo);
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some(GitHubRepoId {
            owner: normalize_owner(owner),
            repo: strip_git_suffix(repo),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_shorthand() {
        let id = parse_github_remote("git@github.com:OctoCat/Hello-World.git").unwrap();
        assert_eq!(id.owner, "octocat");
        assert_eq!(id.repo, "Hello-World");
    }

    #[test]
    fn parses_https() {
        let id = parse_github_remote("https://github.com/OctoCat/Hello-World").unwrap();
        assert_eq!(id.owner, "octocat");
        assert_eq!(id.repo, "Hello-World");
    }
}
