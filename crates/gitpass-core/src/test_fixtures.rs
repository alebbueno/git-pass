//! Credential-shaped strings for **tests only**, built at runtime so push scanners (GitHub) do not
//! match literals in the repository.

#[cfg(test)]
pub(crate) fn ghp_token_36() -> String {
    format!("ghp_{}", "a".repeat(36))
}

#[cfg(test)]
pub(crate) fn github_pat_sample() -> String {
    format!("github_pat_11AAAA_{}", "b".repeat(22))
}

#[cfg(test)]
pub(crate) fn aws_key_line() -> String {
    format!("aws_key=AKIA{}", "E".repeat(16))
}

#[cfg(test)]
pub(crate) fn slack_token_line() -> String {
    format!("SLACK=xoxb-{}-abc", "1".repeat(12))
}

#[cfg(test)]
pub(crate) fn stripe_live_line() -> String {
    format!("sk_live_{}", "5".repeat(24))
}

#[cfg(test)]
pub(crate) fn openssh_private_key_block() -> String {
    format!(
        "a\n-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\nz",
        "placeholder"
    )
}
