//! Embeds the GitHub OAuth App `client_id` at compile time so the app works with no setup.
//! Source order: `GITPASS_GITHUB_OAUTH_CLIENT_ID` env, then `github-oauth-client-id.txt` next to this file.
//! Safe to publish — a Device Flow client_id is public by design (no secret involved).

fn main() {
    println!("cargo:rerun-if-env-changed=GITPASS_GITHUB_OAUTH_CLIENT_ID");
    println!("cargo:rerun-if-changed=github-oauth-client-id.txt");

    let from_env = std::env::var("GITPASS_GITHUB_OAUTH_CLIENT_ID").ok();
    let from_file = std::fs::read_to_string("github-oauth-client-id.txt").ok();
    let id = from_env
        .into_iter()
        .chain(from_file)
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty() && !s.starts_with('#'));

    if let Some(id) = id {
        println!("cargo:rustc-env=GITPASS_EMBEDDED_OAUTH_CLIENT_ID={id}");
    }
}
