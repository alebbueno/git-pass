// Prevents additional console window on Windows in release; keep macOS behavior default.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cmd_accounts;
mod cmd_branches;
mod cmd_cloud;
mod cmd_integration;
mod cmd_push;
mod cmd_repos;
mod cmd_system;
#[cfg(test)]
mod contract_tests;
mod focus;
mod tray;

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use gitpass_core::{
    config_file_path, decode_response_line, encode_line, git_log_lines,
    load_oauth_env_file_if_needed, resolve_oauth_client_id, AppConfig, IpcRequest, IpcResponse,
    KEYCHAIN_SERVICE,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Listener, Manager};

// --- Tauri commands ---

#[derive(serde::Serialize)]
struct OauthConfigStatus {
    ok: bool,
    message: String,
    oauth_env_path: String,
    client_id_configured: bool,
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
        Err(_) => "(pasta de dados desconhecida)".to_string(),
    };
    let _ = load_oauth_env_file_if_needed();
    match resolve_oauth_client_id() {
        Ok(_) => OauthConfigStatus {
            ok: true,
            client_id_configured: true,
            message: "Conexão com o GitHub pronta.".to_string(),
            oauth_env_path,
        },
        Err(_) => OauthConfigStatus {
            ok: false,
            client_id_configured: false,
            message: "A conexão com o GitHub não está disponível nesta versão do app.".to_string(),
            oauth_env_path,
        },
    }
}

#[tauri::command]
fn list_bindings() -> Result<Vec<BindingRow>, String> {
    let cfg = AppConfig::load_or_default().map_err(|e| format!("Não foi possível ler as configurações do GitPass ({e})."))?;
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
async fn repo_commit_log(root_path: String, limit: Option<usize>) -> Result<Vec<String>, String> {
    // `git log` can be slow on big repos; keep it off the main thread (sync commands run there).
    tauri::async_runtime::spawn_blocking(move || {
        let root = cmd_repos::bound_repo_root(&root_path)?;
        git_log_lines(&root, limit.unwrap_or(30))
    })
    .await
    .map_err(|e| format!("Algo deu errado no app: {e}"))?
}

#[cfg(unix)]
#[tauri::command]
fn ipc_ping_gitpassd() -> Result<String, String> {
    use std::os::unix::net::UnixStream;

    let path = gitpass_core::ipc_socket_path().map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err(format!(
            "O serviço em segundo plano não está rodando (sem socket em {}). Só a integração com o Git precisa dele; conectar contas e usar o painel funcionam sem ele. Abra as Configurações e escolha «Reparar».",
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
        return Err("O serviço em segundo plano não respondeu.".to_string());
    }

    match decode_response_line(&line).map_err(|e| e.to_string())? {
        IpcResponse::Pong {
            gitpassd_version,
        } => Ok(format!("Conectado — serviço em segundo plano {}", gitpassd_version)),
        IpcResponse::Error { code, message } => Err(format!("{code}: {message}")),
        _ => Err("O serviço em segundo plano enviou uma resposta que não entendemos.".to_string()),
    }
}

#[cfg(not(unix))]
#[tauri::command]
fn ipc_ping_gitpassd() -> Result<String, String> {
    Err("A verificação do serviço em segundo plano só funciona no macOS e no Linux.".to_string())
}

#[tauri::command]
fn open_keychain_docs() -> String {
    format!("Os acessos às contas ficam salvos nas Chaves do macOS (Keychain), no serviço `{KEYCHAIN_SERVICE}`, conta github.oauth.<id>. Eles nunca são gravados no arquivo de configurações nem em endereços web.")
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
        Err("Abrir a pasta só funciona no macOS.".to_string())
    }
}

const EXTERNAL_URL_MAX_LEN: usize = 2048;

/// Only web addresses with a host and without embedded credentials reach the system browser.
fn checked_external_url(raw: &str) -> Result<tauri::Url, String> {
    let refused = || "Só é possível abrir endereços web (http ou https).".to_string();
    if raw.len() > EXTERNAL_URL_MAX_LEN {
        return Err(refused());
    }
    let url = tauri::Url::parse(raw.trim()).map_err(|_| refused())?;
    let has_host = url.host_str().is_some_and(|h| !h.is_empty());
    let has_credentials = !url.username().is_empty() || url.password().is_some();
    if !matches!(url.scheme(), "http" | "https") || !has_host || has_credentials {
        return Err(refused());
    }
    Ok(url)
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let url = checked_external_url(&url)?;
    opener::open_browser(url.as_str())
        .map_err(|e| format!("Não foi possível abrir o navegador ({e}). Copie o endereço e abra manualmente."))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_config_path,
            oauth_config_status,
            list_bindings,
            repo_commit_log,
            ipc_ping_gitpassd,
            open_keychain_docs,
            open_application_support,
            open_external_url,
            cmd_accounts::accounts_list,
            cmd_accounts::accounts_device_start,
            cmd_accounts::accounts_device_poll,
            cmd_accounts::accounts_refresh_profile,
            cmd_accounts::accounts_set_label,
            cmd_accounts::accounts_emails,
            cmd_accounts::accounts_set_email,
            cmd_accounts::accounts_remove,
            cmd_accounts::accounts_check,
            cmd_repos::repos_list,
            cmd_repos::repos_bind_dialog,
            cmd_repos::repos_bind_path,
            cmd_repos::repos_bind_many,
            cmd_repos::repos_unbind,
            cmd_repos::repos_change_account,
            cmd_repos::repos_fix_identity,
            cmd_repos::repos_discover,
            cmd_repos::repos_pick_folder,
            cmd_repos::repos_map_folder,
            cmd_repos::repos_open_in_finder,
            cmd_repos::repos_open_in_terminal,
            cmd_push::repos_outgoing,
            cmd_push::repos_commit_files,
            cmd_push::repos_push,
            cmd_push::repos_fix_authors,
            cmd_push::repos_pull,
            cmd_branches::repos_branches,
            cmd_branches::repos_checkout,
            cmd_branches::repos_merge_preview,
            cmd_branches::repos_merge_apply,
            cmd_cloud::cloud_repos,
            cmd_cloud::cloud_clone,
            cmd_cloud::cloud_choose_folder,
            cmd_integration::integration_status,
            cmd_integration::integration_apply,
            cmd_integration::integration_undo,
            cmd_integration::integration_daemon_restart,
            cmd_integration::integration_daemon_stop,
            cmd_system::system_current_repo,
            cmd_system::system_pin_repo,
            cmd_system::system_emergency_state,
            cmd_system::system_emergency_start,
            cmd_system::system_emergency_stop,
            cmd_system::system_audit,
            cmd_system::system_diagnostics,
            cmd_system::system_export_logs,
            cmd_system::system_open_accessibility_settings,
        ])
        .setup(|app| {
            cmd_integration::spawn_autostart(app.handle().clone());
            let handle = app.handle().clone();
            app.listen("gitpass-config-changed", move |_| cmd_system::refresh_soon(&handle, true));
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }

            let icon = tray::icon().map_err(|e| format!("tray icon: {e}"))?;
            // Placeholder until the watcher's first refresh fills in the current repository.
            let show = MenuItem::with_id(app, tray::MENU_SHOW, "Abrir GitPass", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, tray::MENU_QUIT, "Sair do GitPass", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            let _tray = TrayIconBuilder::with_id(tray::TRAY_ID)
                .icon(icon)
                .menu(&menu)
                .tooltip("GitPass")
                .icon_as_template(true)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    tray::MENU_SHOW => {
                        tray::show_main(app);
                        cmd_system::refresh_soon(app, false);
                    }
                    tray::MENU_HIDE => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                    tray::MENU_QUIT => app.exit(0),
                    id => cmd_system::on_tray_menu(app, id),
                })
                .on_tray_icon_event(|tray, event| {
                    let app = tray.app_handle();
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            if let Some(w) = app.get_webview_window("main") {
                                if w.is_visible().unwrap_or(false) {
                                    let _ = w.hide();
                                } else {
                                    tray::show_main(app);
                                }
                            }
                            cmd_system::refresh_soon(app, false);
                        }
                        // Hovering shows the tooltip: bring it up to date (the app in focus may have changed).
                        TrayIconEvent::Enter { .. } => cmd_system::refresh_soon(app, false),
                        _ => {}
                    }
                })
                .build(app)?;

            cmd_system::spawn_watcher(app.handle().clone());
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

#[cfg(test)]
mod tests {
    use super::checked_external_url;
    use crate::contract_tests::{expect, keys};

    #[test]
    fn response_keys_match_typescript() {
        // api.ts: OauthConfigStatus, BindingRow
        assert_eq!(
            keys(&super::OauthConfigStatus {
                ok: true,
                message: String::new(),
                oauth_env_path: String::new(),
                client_id_configured: true,
            }),
            expect(&["ok", "message", "oauth_env_path", "client_id_configured"])
        );
        assert_eq!(
            keys(&super::BindingRow {
                id: String::new(),
                account_id: String::new(),
                account_login: None,
                root_path: String::new(),
                github_owner: None,
                github_repo: None,
            }),
            expect(&["id", "account_id", "account_login", "root_path", "github_owner", "github_repo"])
        );
        // api.ts: DeviceCodeStart (core struct returned by accounts_device_start)
        assert_eq!(
            keys(&gitpass_core::DeviceCodeStart {
                client_id: String::new(),
                user_code: String::new(),
                verification_uri: String::new(),
                device_code: String::new(),
                interval_secs: 5,
                expires_in_secs: 900,
            }),
            expect(&["client_id", "user_code", "verification_uri", "device_code", "interval_secs", "expires_in_secs"])
        );
    }

    #[test]
    fn external_urls_are_limited_to_plain_web_addresses() {
        for ok in ["https://github.com/o/r", "http://example.com/x?y=1#z", " https://github.com/settings/applications "] {
            assert!(checked_external_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "x-apple.systempreferences:com.apple.preference",
            "https://user:pass@github.com/",
            "https://ghp_abc@github.com/",
            "https://",
            "/relative",
            "",
        ] {
            assert!(checked_external_url(bad).is_err(), "{bad}");
        }
        assert!(checked_external_url(&format!("https://e.com/{}", "a".repeat(3000))).is_err());
    }
}
