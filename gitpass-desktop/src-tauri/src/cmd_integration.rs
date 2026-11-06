//! Tauri commands for the automatic Git integration (credential helper, hooks, gitpassd).
//! Logic lives in `gitpass_core::integration`; these wrappers run it off the main thread.

use gitpass_core::integration::{self, ApplyOptions, IntegrationEnv, IntegrationReport, IntegrationStatus};
use tauri::{AppHandle, Emitter};

/// Emitted with an [`IntegrationStatus`] payload whenever the integration changes.
pub const INTEGRATION_CHANGED_EVENT: &str = "gitpass-integration-changed";

fn env() -> Result<IntegrationEnv, String> {
    let sidecar_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    IntegrationEnv::system(sidecar_dir)
        .map_err(|e| format!("Não foi possível localizar a pasta de dados do GitPass: {e}"))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Algo deu errado no GitPass: {e}"))?
}

fn notify(app: &AppHandle, status: &IntegrationStatus) {
    let _ = app.emit(INTEGRATION_CHANGED_EVENT, status);
}

#[tauri::command]
pub async fn integration_status() -> Result<IntegrationStatus, String> {
    blocking(|| Ok(integration::status(&env()?))).await
}

#[tauri::command]
pub async fn integration_apply(app: AppHandle, take_over_hooks: Option<bool>) -> Result<IntegrationReport, String> {
    let opts = ApplyOptions {
        take_over_hooks: take_over_hooks.unwrap_or(false),
    };
    let report = blocking(move || integration::apply(&env()?, opts)).await?;
    notify(&app, &report.status);
    Ok(report)
}

#[tauri::command]
pub async fn integration_undo(app: AppHandle) -> Result<IntegrationReport, String> {
    let report = blocking(|| integration::undo(&env()?)).await?;
    notify(&app, &report.status);
    Ok(report)
}

#[tauri::command]
pub async fn integration_daemon_restart(app: AppHandle) -> Result<IntegrationStatus, String> {
    let status = blocking(|| integration::daemon_restart(&env()?)).await?;
    notify(&app, &status);
    Ok(status)
}

#[tauri::command]
pub async fn integration_daemon_stop(app: AppHandle) -> Result<IntegrationStatus, String> {
    let status = blocking(|| integration::daemon_stop(&env()?)).await?;
    notify(&app, &status);
    Ok(status)
}

/// Called from `setup`: installs/repairs the integration in the background and notifies the UI.
/// Debug builds skip it unless `GITPASS_DEV_AUTO_INTEGRATION=1`, so `tauri dev` does not rewrite
/// the developer's real `~/.gitconfig` (the "Reparar" button still works).
pub fn spawn_autostart(app: AppHandle) {
    if cfg!(debug_assertions) && std::env::var("GITPASS_DEV_AUTO_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    std::thread::spawn(move || {
        let Ok(env) = env() else {
            return;
        };
        let status = match integration::autostart(&env) {
            Ok(Some(report)) => report.status,
            Ok(None) | Err(_) => integration::status(&env),
        };
        notify(&app, &status);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_tests::{expect, keys};
    use gitpass_core::integration::{CheckState, ComponentStatus};

    fn component() -> ComponentStatus {
        ComponentStatus {
            state: CheckState::Ok,
            summary: String::new(),
            current_value: None,
            previous_value: None,
        }
    }

    /// api-integration.ts: ComponentStatus, IntegrationStatus, IntegrationReport, CheckState.
    #[test]
    fn response_keys_match_typescript() {
        assert_eq!(keys(&component()), expect(&["state", "summary", "current_value", "previous_value"]));
        let status = IntegrationStatus {
            enabled: true,
            git_found: true,
            binaries: component(),
            credentials: component(),
            hooks: component(),
            daemon: component(),
            has_backup: false,
            ready: true,
        };
        assert_eq!(
            keys(&status),
            expect(&["enabled", "git_found", "binaries", "credentials", "hooks", "daemon", "has_backup", "ready"])
        );
        assert_eq!(
            keys(&IntegrationReport { status, changes: vec![], warnings: vec![] }),
            expect(&["status", "changes", "warnings"])
        );
        let states: Vec<_> = [CheckState::Ok, CheckState::Missing, CheckState::Outdated, CheckState::Conflict, CheckState::Error]
            .iter()
            .map(|s| serde_json::to_value(s).unwrap())
            .collect();
        assert_eq!(states, ["ok", "missing", "outdated", "conflict", "error"]);
    }
}
