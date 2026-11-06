//! Optional embed of `GITPASS_GITHUB_OAUTH_CLIENT_ID` at compile time for shipping builds
//! (`cargo build` with that env var set). Safe to publish — client_id is public in desktop apps.

fn main() {
    println!("cargo:rerun-if-env-changed=GITPASS_GITHUB_OAUTH_CLIENT_ID");
    if let Ok(id) = std::env::var("GITPASS_GITHUB_OAUTH_CLIENT_ID") {
        let id = id.trim();
        if !id.is_empty() {
            println!("cargo:rustc-env=GITPASS_EMBEDDED_OAUTH_CLIENT_ID={id}");
        }
    }
}
