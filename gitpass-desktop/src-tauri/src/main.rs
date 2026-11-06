// Prevents additional console window on Windows in release; keep macOS behavior default.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gitpass_core::{
    config_file_path, decode_response_line, encode_line, embedded_oauth_client_id, git_log_lines,
    git_remote_url, github_oauth_device_poll_once, github_oauth_device_request_code,
    github_token_load, github_token_save, list_user_repos, load_oauth_env_file_if_needed,
    normalize_owner, parse_github_remote, resolve_oauth_client_id, AppConfig, AuthMode,
    DeviceCodeStart, DevicePollOutcome, GithubRepoSummary, IpcRequest, IpcResponse, KEYCHAIN_SERVICE,
};
use gitpass_core::DEFAULT_GITHUB_SCOPES;
use gitpass_core::{Account, RepoBinding};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

const TRAY_ID: &str = "gitpass-tray";

fn tray_icon_from_build() -> Result<Image<'static>, String> {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/trayTemplate.png"));
    Image::from_bytes(bytes).map_err(|e| e.to_string())
}

fn shorten_path(p: &Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        if p.starts_with(&home) {
            if let Ok(rest) = p.strip_prefix(&home) {
                return format!("~{}", rest.display());
            }
        }
    }
    p.display().to_string()
}

fn tray_tooltip(cfg: &AppConfig) -> String {
    let binding: Option<&RepoBinding> = if let Some(rid) = cfg
        .ui
        .pinned_repo_binding_id
        .or(cfg.ui.last_active_repo_binding_id)
    {
        cfg.repos.iter().find(|r| r.id == rid)
    } else if cfg.repos.len() == 1 {
        cfg.repos.first()
    } else {
        None
    };

    let Some(b) = binding else {
        return "GitPass — add accounts & repo bindings".to_string();
    };

    let acc = cfg.accounts.iter().find(|a| a.id == b.account_id);
    let login = acc
        .and_then(|a| a.github_login.as_deref())
        .unwrap_or("?");
    let path = shorten_path(&b.root_path);
    let gh = match (&b.github_owner, &b.github_repo) {
        (Some(o), Some(r)) => format!(" — {o}/{r}"),
        _ => String::new(),
    };

    format!("GitPass — @{login}{gh}\n{path}")
}

fn refresh_tray_tooltip(app: &AppHandle) {
    let tip = match AppConfig::load_or_default() {
        Ok(c) => tray_tooltip(&c),
        Err(_) => "GitPass".to_string(),
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tip.as_str()));
    }
}

fn initial_tooltip() -> String {
    match AppConfig::load_or_default() {
        Ok(c) => tray_tooltip(&c),
        Err(_) => "GitPass".to_string(),
    }
}

fn find_account_by_login<'a>(cfg: &'a AppConfig, login: &str) -> Option<&'a Account> {
    let n = normalize_owner(login);
    cfg.accounts.iter().find(|a| {
        a.github_login
            .as_ref()
            .map(|l| normalize_owner(l) == n)
            .unwrap_or(false)
    })
}

// --- Tauri commands ---

#[derive(serde::Serialize)]
struct AccountRow {
    id: String,
    login: Option<String>,
    label: Option<String>,
    auth: String,
    has_token: bool,
}

#[derive(serde::Serialize)]
struct OauthConfigStatus {
    ok: bool,
    message: String,
    oauth_env_path: String,
    client_id_configured: bool,
}

#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum DevicePollUi {
    Pending,
    SlowDown {
        interval_secs: u64,
    },
    Done {
        message: String,
    },
}

#[derive(serde::Serialize)]
struct BindingRow {
    id: String,
    account_id: String,
    account_login: Option<String>,
    root_path: String,
    github_owner: Option<String>,
    github_repo: Option<String>,
}

#[tauri::command]
fn get_config_path() -> Result<String, String> {
    config_file_path()
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn oauth_config_status() -> OauthConfigStatus {
    let oauth_env_path = match gitpass_core::app_support_dir() {
        Ok(d) => d.join("oauth.env").display().to_string(),
        Err(_) => "(app support dir unknown)".to_string(),
    };
    let _ = load_oauth_env_file_if_needed();
    match resolve_oauth_client_id() {
        Ok(id) => {
            let embed = embedded_oauth_client_id().is_some();
            OauthConfigStatus {
                ok: true,
                client_id_configured: true,
                message: if embed {
                    format!("GitHub OAuth Device flow ready (client_id embutido no build).")
                } else {
                    format!("GitHub OAuth client_id: {id}… (env ou oauth.env)")
                },
                oauth_env_path,
            }
        }
        Err(e) => OauthConfigStatus {
            ok: false,
            client_id_configured: false,
            message: format!(
                "{e}\n\nPara builds plug-and-play: define GITPASS_GITHUB_OAUTH_CLIENT_ID ao compilar e ativa **Device flow** na OAuth App do GitHub (Developer Settings)."
            ),
            oauth_env_path,
        },
    }
}

#[tauri::command]
fn list_accounts() -> Result<Vec<AccountRow>, String> {
    let cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for a in &cfg.accounts {
        let has_token = github_token_load(a.id)
            .map_err(|e| e.to_string())?
            .is_some();
        out.push(AccountRow {
            id: a.id.to_string(),
            login: a.github_login.clone(),
            label: a.label.clone(),
            auth: format!("{:?}", a.auth).to_lowercase(),
            has_token,
        });
    }
    Ok(out)
}

#[tauri::command]
fn list_bindings() -> Result<Vec<BindingRow>, String> {
    let cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in &cfg.repos {
        let acc_login = cfg
            .accounts
            .iter()
            .find(|a| a.id == r.account_id)
            .and_then(|a| a.github_login.clone());
        out.push(BindingRow {
            id: r.id.to_string(),
            account_id: r.account_id.to_string(),
            account_login: acc_login,
            root_path: r.root_path.display().to_string(),
            github_owner: r.github_owner.clone(),
            github_repo: r.github_repo.clone(),
        });
    }
    Ok(out)
}

#[tauri::command]
fn repo_commit_log(root_path: String, limit: Option<usize>) -> Result<Vec<String>, String> {
    let root = PathBuf::from(root_path);
    git_log_lines(&root, limit.unwrap_or(30))
}

#[tauri::command]
fn github_cloud_repos_for_account(account_id: String) -> Result<Vec<GithubRepoSummary>, String> {
    let id = Uuid::parse_str(&account_id).map_err(|_| "invalid account id".to_string())?;
    let cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
    if !cfg.accounts.iter().any(|a| a.id == id) {
        return Err("unknown account".to_string());
    }
    let bundle = github_token_load(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no GitHub token — use Connect GitHub first".to_string())?;
    list_user_repos(&bundle.access_token).map_err(|e| e.to_string())
}

#[tauri::command]
fn github_device_oauth_start() -> Result<DeviceCodeStart, String> {
    load_oauth_env_file_if_needed().map_err(|e| e.to_string())?;
    let client_id = resolve_oauth_client_id().map_err(|e| e.to_string())?;
    let scopes =
        std::env::var("GITPASS_OAUTH_SCOPES").unwrap_or_else(|_| DEFAULT_GITHUB_SCOPES.to_string());

    let start = github_oauth_device_request_code(&client_id, &scopes).map_err(|e| e.to_string())?;
    opener::open(&start.verification_uri).map_err(|e| format!("open browser: {e}"))?;
    Ok(start)
}

#[tauri::command]
fn github_device_oauth_poll(
    app: AppHandle,
    client_id: String,
    device_code: String,
) -> Result<DevicePollUi, String> {
    match github_oauth_device_poll_once(&client_id, &device_code).map_err(|e| e.to_string())? {
        DevicePollOutcome::Pending => Ok(DevicePollUi::Pending),
        DevicePollOutcome::SlowDown { interval_secs } => Ok(DevicePollUi::SlowDown { interval_secs }),
        DevicePollOutcome::Success { bundle, login } => {
            let login_remote = login.clone();
            let login_norm = normalize_owner(&login_remote);

            let mut cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
            if find_account_by_login(&cfg, &login_norm).is_some() {
                return Err(format!(
                    "A conta `{login_remote}` já existe. Remove-a no config ou usa o CLI."
                ));
            }

            let id = Uuid::new_v4();
            cfg.accounts.push(Account {
                id,
                github_login: Some(login_remote.clone()),
                label: None,
                auth: AuthMode::Https,
            });
            github_token_save(id, &bundle).map_err(|e| e.to_string())?;
            cfg.save().map_err(|e| e.to_string())?;

            let _ = app.emit("gitpass-config-changed", ());
            refresh_tray_tooltip(&app);
            Ok(DevicePollUi::Done {
                message: format!("Conta @{login_remote} ligada; token no Keychain."),
            })
        }
    }
}

#[tauri::command]
fn bind_repo_dialog(account_id: String, app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let out = Command::new("osascript")
            .arg("-e")
            .arg(
                r#"tell application "System Events"
  activate
  set f to choose folder with prompt "Choose a local git repository folder"
  POSIX path of f
end tell"#,
            )
            .output()
            .map_err(|e| format!("could not open folder picker: {e}"))?;
        if !out.status.success() {
            return Err("folder selection cancelled".to_string());
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() {
            return Err("no folder selected".to_string());
        }
        let root = PathBuf::from(&path);
        return bind_repo_at_path(account_id, root, app);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (account_id, app);
        Err("folder picker is only wired on macOS for now".to_string())
    }
}

fn bind_repo_at_path(account_id: String, root: PathBuf, app: AppHandle) -> Result<(), String> {
    let acc_id = Uuid::parse_str(&account_id).map_err(|_| "invalid account id".to_string())?;

    let mut cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
    if !cfg.accounts.iter().any(|a| a.id == acc_id) {
        return Err("unknown account".to_string());
    }

    let canon = root
        .canonicalize()
        .map_err(|e| format!("invalid path: {e}"))?;

    let url = git_remote_url(&canon, "origin")
        .map_err(|e| format!("{e}. Add remote `origin` or pick another clone."))?;
    let gh = parse_github_remote(&url).ok_or_else(|| {
        format!("origin is not a github.com URL (got `{url}`). Use HTTPS or git@github.com:owner/repo")
    })?;
    let owner = gh.owner;
    let repo = gh.repo;

    if cfg.repos.iter().any(|r| r.root_path == canon) {
        return Err("this folder is already bound".to_string());
    }

    let id = Uuid::new_v4();
    cfg.repos.push(RepoBinding {
        id,
        account_id: acc_id,
        root_path: canon,
        github_owner: Some(owner.clone()),
        github_repo: Some(repo.clone()),
    });
    cfg.save().map_err(|e| e.to_string())?;

    let _ = app.emit("gitpass-config-changed", ());
    refresh_tray_tooltip(&app);
    Ok(())
}

#[cfg(unix)]
#[tauri::command]
fn ipc_ping_gitpassd() -> Result<String, String> {
    use std::os::unix::net::UnixStream;

    let path = gitpass_core::ipc_socket_path().map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err(format!(
            "gitpassd não está a correr (sem socket em {}). Só os hooks Git precisam do daemon; Connect OAuth e painel funcionam sem ele. Para arrancar: cargo run -p gitpassd --bin gitpassd",
            path.display()
        ));
    }

    let mut stream = UnixStream::connect(&path).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));

    let req = encode_line(&IpcRequest::Ping).map_err(|e| e.to_string())?;
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if line.is_empty() {
        return Err("empty response from gitpassd".to_string());
    }

    match decode_response_line(&line).map_err(|e| e.to_string())? {
        IpcResponse::Pong {
            gitpassd_version,
        } => Ok(format!("Connected — gitpassd {}", gitpassd_version)),
        IpcResponse::Error { code, message } => Err(format!("{code}: {message}")),
        _ => Err("unexpected response from gitpassd".to_string()),
    }
}

#[cfg(not(unix))]
#[tauri::command]
fn ipc_ping_gitpassd() -> Result<String, String> {
    Err("IPC ping is only available on Unix".to_string())
}

#[tauri::command]
fn open_keychain_docs() -> String {
    format!("OAuth tokens use Keychain service `{KEYCHAIN_SERVICE}` (account github.oauth.<uuid>).")
}

#[tauri::command]
fn open_application_support() -> Result<(), String> {
    let dir = gitpass_core::app_support_dir().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dir;
        Err("open folder is only implemented on macOS".to_string())
    }
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("URL não permitido: {url}"));
    }
    opener::open_browser(&url).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_config_path,
            oauth_config_status,
            list_accounts,
            list_bindings,
            repo_commit_log,
            github_cloud_repos_for_account,
            github_device_oauth_start,
            github_device_oauth_poll,
            bind_repo_dialog,
            ipc_ping_gitpassd,
            open_keychain_docs,
            open_application_support,
            open_external_url,
        ])
        .setup(|app| {
            let icon = tray_icon_from_build().map_err(|e| format!("tray icon: {e}"))?;

            let show = MenuItem::with_id(app, "show", "Open GitPass", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit GitPass", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &sep, &quit])?;

            let _tray = TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .menu(&menu)
                .tooltip(initial_tooltip())
                .icon_as_template(true)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                            refresh_tray_tooltip(app);
                        }
                        "hide" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.hide();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        refresh_tray_tooltip(&app);
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                    if let TrayIconEvent::Click {
                        button: MouseButton::Right,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        refresh_tray_tooltip(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running GitPass");
}
