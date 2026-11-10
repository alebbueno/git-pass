//! `api-system.ts`: current repository (menu bar), emergency mode, audit log, diagnostics and log
//! export. Also keeps the tray and the frontend up to date: a light watcher re-reads `recent.json`,
//! the config and the app in focus (every 2 s while the window is open, else every 15 s, and right
//! when an emergency runs out) and emits the change events.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use gitpass_core::audit::{self, AuditEntry};
use gitpass_core::current_repo::{self, CurrentRepoState, FocusedRepo};
use gitpass_core::diagnostics::{self, Diagnostics, DiagnosticsInput};
use gitpass_core::emergency::{self, EmergencyView};
use gitpass_core::integration::{self, IntegrationEnv};
use gitpass_core::{logs_export, recent, AppConfig};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::cmd_repos::{self, PendingPushSummary};
use crate::{dock, focus, pending_alert, tray};

pub const CURRENT_REPO_CHANGED_EVENT: &str = "gitpass-current-repo-changed";
pub const EMERGENCY_CHANGED_EVENT: &str = "gitpass-emergency-changed";

const ACCESSIBILITY_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const POLL_VISIBLE: Duration = Duration::from_secs(2);
const POLL_HIDDEN: Duration = Duration::from_secs(15);
const AUDIT_MAX_LIMIT: usize = 1000;

#[derive(Default)]
struct Watch {
    /// Repository of the last app in focus (kept while GitPass itself is in front).
    focus: Option<FocusedRepo>,
    repo: Option<CurrentRepoState>,
    emergency: Option<EmergencyView>,
    pending: Option<PendingPushSummary>,
}

static WATCH: Mutex<Watch> = Mutex::new(Watch {
    focus: None,
    repo: None,
    emergency: None,
    pending: None,
});
/// One refresh at a time (watcher, tray events and commands).
static REFRESH: Mutex<()> = Mutex::new(());

fn watch() -> std::sync::MutexGuard<'static, Watch> {
    WATCH.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> String {
    audit::now_rfc3339()
}

fn load_config() -> Result<AppConfig, String> {
    AppConfig::load_or_default().map_err(|e| format!("Não foi possível ler as configurações do GitPass ({e})."))
}

fn app_support() -> Result<PathBuf, String> {
    gitpass_core::app_support_dir().map_err(|e| format!("Não foi possível localizar a pasta de dados do GitPass ({e})."))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Algo deu errado no GitPass: {e}"))?
}

/// Updates the remembered focus from the frontmost app. Returns whether Accessibility is granted.
fn observe_focus(cfg: &AppConfig) -> bool {
    if !focus::accessibility_granted() {
        watch().focus = None;
        return false;
    }
    let Some(w) = focus::focused_window() else { return true };
    if w.pid == std::process::id() as i32 {
        return true;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let root = current_repo::binding_for_focus(cfg, w.document.as_deref(), w.title.as_deref(), home.as_deref())
        .map(|b| b.root_path.clone());
    let mut g = watch();
    g.focus = match root {
        Some(r) if g.focus.as_ref().is_some_and(|f| f.root_path == r) => g.focus.take(),
        Some(r) => Some(FocusedRepo { root_path: r, at: now() }),
        None => None,
    };
    true
}

fn current_state_with(cfg: &AppConfig) -> CurrentRepoState {
    // `at` of a pin without a date (older configs): stable, so the watcher does not see a change every tick.
    static STARTED: OnceLock<String> = OnceLock::new();
    let granted = observe_focus(cfg);
    let focus = watch().focus.clone();
    current_repo::resolve(cfg, &recent::load(), focus.as_ref(), granted, STARTED.get_or_init(now))
}

/// Clears an emergency that ran out (audited as `emergency_expired`) and returns the state.
fn emergency_checked() -> Result<EmergencyView, String> {
    emergency::expire_now()?;
    emergency::view_now()
}

/// Re-reads everything, emits the events whose payload changed and updates the tray.
/// `force` redraws the tray even when nothing changed (e.g. an account was renamed).
pub fn refresh(app: &AppHandle, force: bool) {
    let _one = REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(em) = emergency_checked() else { return };
    let Ok(cfg) = load_config() else { return };
    let state = current_state_with(&cfg);
    let pending = cmd_repos::pending_push_summary(&cfg);
    let prev_pending = watch().pending.clone();
    let (repo_changed, em_changed, pending_changed) = {
        let mut g = watch();
        let r = g.repo.as_ref() != Some(&state);
        let e = g.emergency.as_ref() != Some(&em);
        let p = g.pending.as_ref() != Some(&pending);
        g.repo = Some(state.clone());
        g.emergency = Some(em.clone());
        g.pending = Some(pending.clone());
        (r, e, p)
    };
    if repo_changed {
        let _ = app.emit(CURRENT_REPO_CHANGED_EVENT, &state);
    }
    if em_changed {
        let _ = app.emit(EMERGENCY_CHANGED_EVENT, &em);
    }
    if force || repo_changed || em_changed || pending_changed {
        tray::update(app, &cfg, &state, &em, &pending);
        dock::update(app, &pending);
        if pending_changed && pending_alert::should_notify(prev_pending.as_ref(), &pending) {
            pending_alert::notify(&pending);
        }
    }
}

/// [`refresh`] off the calling thread (tray and window events run on the main thread).
pub fn refresh_soon(app: &AppHandle, force: bool) {
    let app = app.clone();
    std::thread::spawn(move || refresh(&app, force));
}

fn next_wait(app: &AppHandle) -> Duration {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    let wait = if visible { POLL_VISIBLE } else { POLL_HIDDEN };
    let until = watch().emergency.as_ref().and_then(|e| e.until.clone());
    let left = until
        .and_then(|u| DateTime::parse_from_rfc3339(&u).ok())
        .and_then(|u| (u.with_timezone(&Utc) - Utc::now()).to_std().ok())
        .map(|d| d + Duration::from_millis(300));
    left.map_or(wait, |l| l.min(wait))
}

pub fn spawn_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        refresh(&app, true);
        loop {
            std::thread::sleep(next_wait(&app));
            refresh(&app, false);
        }
    });
}

/// Tray menu actions (run off the main thread).
pub fn on_tray_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    std::thread::spawn(move || {
        let result = match id.as_str() {
            tray::MENU_PIN => {
                let current = load_config().map(|c| current_state_with(&c).current);
                match current {
                    Ok(Some(r)) => pin(r.binding_id.as_deref()).map(|_| ()),
                    Ok(None) => Ok(()),
                    Err(e) => Err(e),
                }
            }
            tray::MENU_UNPIN => pin(None).map(|_| ()),
            tray::MENU_EMERGENCY_15 => emergency::start_now(15, tray::MENUBAR_EMERGENCY_REASON, "menubar").map(|_| ()),
            tray::MENU_EMERGENCY_60 => emergency::start_now(60, tray::MENUBAR_EMERGENCY_REASON, "menubar").map(|_| ()),
            tray::MENU_EMERGENCY_OFF => emergency::stop_now("menubar").map(|_| ()),
            _ => return,
        };
        if let Err(e) = result {
            eprintln!("GitPass: {e}");
        }
        refresh(&app, true);
    });
}

fn pin(binding_id: Option<&str>) -> Result<(), String> {
    let mut cfg = load_config()?;
    let id = match binding_id {
        None => None,
        Some(raw) => {
            let id = Uuid::parse_str(raw.trim()).map_err(|_| "Repositório inválido.".to_string())?;
            if !cfg.repos.iter().any(|r| r.id == id) {
                return Err("Este repositório não está mais vinculado. Atualize a lista em Repositórios.".into());
            }
            Some(id)
        }
    };
    if cfg.ui.pinned_repo_binding_id == id {
        return Ok(());
    }
    cfg.ui.pinned_repo_binding_id = id;
    cfg.ui.pinned_at = id.map(|_| now());
    cfg.save().map_err(|e| format!("Não foi possível salvar as configurações do GitPass ({e})."))
}

// --- Commands ---

#[tauri::command]
pub async fn system_current_repo() -> Result<CurrentRepoState, String> {
    blocking(|| Ok(current_state_with(&load_config()?))).await
}

#[tauri::command]
pub async fn system_pin_repo(app: AppHandle, binding_id: Option<String>) -> Result<CurrentRepoState, String> {
    blocking(move || {
        pin(binding_id.as_deref())?;
        refresh(&app, true);
        Ok(current_state_with(&load_config()?))
    })
    .await
}

#[tauri::command]
pub async fn system_emergency_state(app: AppHandle) -> Result<EmergencyView, String> {
    blocking(move || {
        let v = emergency_checked()?;
        refresh(&app, false);
        Ok(v)
    })
    .await
}

#[tauri::command]
pub async fn system_emergency_start(app: AppHandle, minutes: i64, reason: String) -> Result<EmergencyView, String> {
    blocking(move || {
        let v = emergency::start_now(minutes, &reason, "app")?;
        refresh(&app, true);
        Ok(v)
    })
    .await
}

#[tauri::command]
pub async fn system_emergency_stop(app: AppHandle) -> Result<EmergencyView, String> {
    blocking(move || {
        let v = emergency::stop_now("app")?;
        refresh(&app, true);
        Ok(v)
    })
    .await
}

#[tauri::command]
pub async fn system_audit(limit: usize) -> Result<Vec<AuditEntry>, String> {
    blocking(move || Ok(audit::read_in(&app_support()?, limit.clamp(1, AUDIT_MAX_LIMIT)))).await
}

fn integration_env() -> Result<IntegrationEnv, String> {
    let sidecar_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    IntegrationEnv::system(sidecar_dir).map_err(|e| format!("Não foi possível localizar a pasta de dados do GitPass: {e}"))
}

/// The account has an access saved in the Keychain that GitHub has not rejected.
fn token_usable(app_support: &std::path::Path, id: Uuid) -> Result<bool, String> {
    match gitpass_core::github_token_load(id) {
        Ok(Some(t)) => Ok(!integration::is_token_rejected(app_support, id, &t.access_token)),
        Ok(None) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn run_diagnostics(probe_tokens: bool) -> Result<Diagnostics, String> {
    let env = integration_env()?;
    let input_base = || DiagnosticsInput {
        env: &env,
        app_version: env!("CARGO_PKG_VERSION"),
        accessibility_granted: Some(focus::accessibility_granted()),
        token_probe: None,
    };
    if probe_tokens {
        let support = env.app_support.clone();
        let probe = move |id: Uuid| token_usable(&support, id);
        Ok(diagnostics::run(&DiagnosticsInput {
            token_probe: Some(&probe),
            ..input_base()
        }))
    } else {
        Ok(diagnostics::run(&input_base()))
    }
}

#[tauri::command]
pub async fn system_diagnostics(probe_tokens: Option<bool>) -> Result<Diagnostics, String> {
    blocking(move || run_diagnostics(probe_tokens.unwrap_or(true))).await
}

/// "Salvar como" for the logs ZIP. `Ok(None)` when the user cancels.
#[cfg(target_os = "macos")]
fn save_dialog(default_name: &str) -> Result<Option<PathBuf>, String> {
    let choose = format!(
        r#"POSIX path of (choose file name with prompt "Salvar os logs do GitPass" default name "{default_name}" default location (path to desktop folder))"#
    );
    let in_front = format!("tell application \"System Events\"\nactivate\n{choose}\nend tell");
    let picked = match crate::cmd_repos::osascript(&in_front)? {
        Ok(p) => p,
        // Automation of System Events was denied: the plain dialog still works, it may just open behind the window.
        Err(e) if e.contains("-1743") || e.contains("-600") => crate::cmd_repos::osascript(&choose)?
            .map_err(|e| format!("Não foi possível abrir a janela para salvar: {e}"))?,
        Err(e) => return Err(format!("Não foi possível abrir a janela para salvar: {e}")),
    };
    Ok(picked.map(|p| {
        let is_zip = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if is_zip { p } else { p.with_extension("zip") }
    }))
}

#[cfg(not(target_os = "macos"))]
fn save_dialog(_default_name: &str) -> Result<Option<PathBuf>, String> {
    Err("Exportar os logs só funciona no macOS.".into())
}

#[tauri::command]
pub async fn system_export_logs() -> Result<Option<String>, String> {
    blocking(|| {
        let name = logs_export::default_file_name(chrono::Local::now());
        let Some(dest) = save_dialog(&name)? else { return Ok(None) };
        let diag = run_diagnostics(true)?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        logs_export::write_logs_zip(&app_support()?, home.as_deref(), &diag, &dest)?;
        Ok(Some(dest.display().to_string()))
    })
    .await
}

#[tauri::command]
pub fn system_open_accessibility_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg(ACCESSIBILITY_SETTINGS_URL)
            .status()
            .map_err(|e| format!("Não foi possível abrir os Ajustes do Sistema ({e})."))?;
        if !status.success() {
            return Err("Não foi possível abrir os Ajustes do Sistema. Abra Privacidade e Segurança › Acessibilidade.".into());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ACCESSIBILITY_SETTINGS_URL;
        Err("Os ajustes de Acessibilidade só existem no macOS.".into())
    }
}
