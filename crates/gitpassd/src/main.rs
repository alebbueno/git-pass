//! GitPass daemon: Unix socket IPC (macOS v1). Windows builds ship a stub until W2.

#[cfg(unix)]
mod unix;

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    unix::run().await
}

#[cfg(not(unix))]
fn main() {
    eprintln!(
        "gitpassd ainda não está disponível no Windows (veja docs/plans/windows-port.md — fase W2)."
    );
    std::process::exit(1);
}
