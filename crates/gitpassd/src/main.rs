//! GitPass daemon: Unix socket IPC (macOS v1).

use std::path::Path;

use gitpass_core::{
    config_file_path, decode_request_line, encode_line, ensure_app_support_exists,
    ipc_client, ipc_socket_path, validate_repo_for_hook_daemon_within, AppConfig, HookContext,
    IpcRequest, IpcResponse, ValidateFail,
};
use tokio::io::{split, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if !cfg!(target_os = "macos") {
        warn!("gitpassd is intended for macOS; continuing anyway (dev only)");
    }

    let config_path = config_file_path()?;
    let cfg = match AppConfig::load_or_default() {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "failed to load config.toml — fix or remove the file");
            std::process::exit(1);
        }
    };
    if !config_path.exists() {
        match cfg.save() {
            Ok(()) => info!(path = %config_path.display(), "created default config.toml"),
            Err(e) => {
                error!(error = %e, path = %config_path.display(), "failed to write default config.toml");
                std::process::exit(1);
            }
        }
    } else {
        info!(
            config_path = %config_path.display(),
            accounts = cfg.accounts.len(),
            repos = cfg.repos.len(),
            "loaded app config"
        );
    }

    let sock_path = ipc_socket_path()?;
    ensure_app_support_exists()?;
    ensure_socket_parent(&sock_path)?;
    if ipc_client::socket_in_use(&sock_path) {
        // Replacing a live daemon's socket would leave it serving nobody; take over once it stops.
        info!(path = %sock_path.display(), "another gitpassd is listening; waiting for it to stop");
        while ipc_client::socket_in_use(&sock_path) {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    if sock_path.exists() {
        // Remove stale socket from a prior crash.
        std::fs::remove_file(&sock_path)?;
    }

    let listener = UnixListener::bind(&sock_path)?;
    info!(path = %sock_path.display(), "listening");

    let version = env!("CARGO_PKG_VERSION").to_string();

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let v = version.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, v).await {
                        error!(error = %e, "client handler failed");
                    }
                });
            }
            Err(e) => error!(error = %e, "accept failed"),
        }
    }
}

fn ensure_socket_parent(sock_path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

async fn handle_client(
    stream: UnixStream,
    version: String,
) -> Result<(), std::io::Error> {
    let (read_half, mut write_half) = split(stream);
    let mut lines = BufReader::new(read_half).lines();
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            continue;
        }

        let response = match decode_request_line(&line) {
            Ok(IpcRequest::Ping) => IpcResponse::Pong {
                gitpassd_version: version.clone(),
            },
            Ok(IpcRequest::ValidateHook { phase, cwd }) => {
                let ctx = HookContext { phase, cwd };
                let res = tokio::task::spawn_blocking(move || {
                    let strict_api = std::env::var("GITPASS_STRICT_API")
                        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                        .unwrap_or(false);
                    let allow_ambiguous = !strict_api;
                    match AppConfig::load_or_default() {
                        Ok(cfg) => validate_repo_for_hook_daemon_within(
                            &ctx,
                            &cfg,
                            allow_ambiguous,
                            ipc_client::DAEMON_VALIDATE_DEADLINE,
                        ),
                        Err(e) => ValidateFail::Err {
                            code: "GP-CONFIG",
                            message: format!("{e}"),
                        },
                    }
                })
                .await;

                match res {
                    Ok(ValidateFail::Ok) => IpcResponse::ValidateResult {
                        ok: true,
                        unconfirmed: false,
                        code: None,
                        message: None,
                        warning: None,
                    },
                    Ok(ValidateFail::OkUnconfirmed { warning }) => IpcResponse::ValidateResult {
                        ok: true,
                        unconfirmed: true,
                        code: None,
                        message: None,
                        warning: Some(warning),
                    },
                    Ok(ValidateFail::Err { code, message }) => IpcResponse::ValidateResult {
                        ok: false,
                        unconfirmed: false,
                        code: Some(code.to_string()),
                        message: Some(message),
                        warning: None,
                    },
                    Err(e) => IpcResponse::ValidateResult {
                        ok: false,
                        unconfirmed: false,
                        code: Some("GP-IPC-TASK".into()),
                        message: Some(format!("{e}")),
                        warning: None,
                    },
                }
            }
            Err(e) => IpcResponse::Error {
                code: "GP-IPC-DECODE".into(),
                message: format!("invalid request: {e}"),
            },
        };

        let out = encode_line(&response).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("encode: {e}"),
            )
        })?;
        write_half.write_all(out.as_bytes()).await?;
        write_half.flush().await?;
    }
    Ok(())
}
