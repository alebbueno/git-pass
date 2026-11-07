//! Menu bar icon: tooltip and menu show the current repository (name, account, last verdict),
//! pin / unpin, emergency mode and "Abrir GitPass". Only names and codes, never secrets (DL-034).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use gitpass_core::current_repo::{CurrentRepo, CurrentRepoSource, CurrentRepoState};
use gitpass_core::emergency::EmergencyView;
use gitpass_core::AppConfig;
use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};

pub const TRAY_ID: &str = "gitpass-tray";

pub const MENU_PIN: &str = "pin";
pub const MENU_UNPIN: &str = "unpin";
pub const MENU_EMERGENCY_15: &str = "emergency_15";
pub const MENU_EMERGENCY_60: &str = "emergency_60";
pub const MENU_EMERGENCY_OFF: &str = "emergency_off";
pub const MENU_SHOW: &str = "show";
pub const MENU_HIDE: &str = "hide";
pub const MENU_QUIT: &str = "quit";

/// Reason recorded in the audit log for emergencies turned on from the menu bar.
pub const MENUBAR_EMERGENCY_REASON: &str = "Ativado pela barra de menus";

pub fn icon() -> Result<Image<'static>, String> {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/trayTemplate.png"));
    Image::from_bytes(bytes).map_err(|e| e.to_string())
}

fn shorten_path(p: &Path) -> String {
    gitpass_core::redact::redact_home(&p.display().to_string(), dirs_home().as_deref())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn repo_name(r: &CurrentRepo) -> String {
    r.full_name.clone().unwrap_or_else(|| {
        Path::new(&r.root_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| r.root_path.clone())
    })
}

fn account_login(cfg: &AppConfig, r: &CurrentRepo) -> Option<String> {
    let id = r.account_id.as_deref()?;
    cfg.accounts
        .iter()
        .find(|a| a.id.to_string() == id)
        .and_then(|a| a.github_login.clone())
}

fn source_label(s: CurrentRepoSource) -> &'static str {
    match s {
        CurrentRepoSource::Pinned => "fixado",
        CurrentRepoSource::Focus => "app em foco",
        CurrentRepoSource::Recent => "último usado",
    }
}

/// `Situação: Bloqueado (GP-OWNER)`.
fn verdict_line(r: &CurrentRepo) -> String {
    match (r.verdict, &r.code) {
        (Some(v), Some(code)) if v != gitpass_core::recent::Verdict::Ok => format!("Situação: {} ({code})", v.label()),
        (Some(v), _) => format!("Situação: {}", v.label()),
        (None, _) if r.binding_id.is_none() => "Situação: não vinculado a uma conta".into(),
        (None, _) => "Situação: ainda não verificado".into(),
    }
}

fn title_line(cfg: &AppConfig, r: &CurrentRepo) -> String {
    match account_login(cfg, r) {
        Some(login) => format!("{} — @{login}", repo_name(r)),
        None => repo_name(r),
    }
}

fn local_time(rfc3339: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(rfc3339).ok().map(|d| d.with_timezone(&Local).format("%H:%M").to_string())
}

fn emergency_line(em: &EmergencyView) -> Option<String> {
    if !em.active {
        return None;
    }
    Some(match em.until.as_deref().and_then(local_time) {
        Some(t) => format!("Modo emergência ativo até {t}"),
        None => "Modo emergência ativo".into(),
    })
}

pub fn tooltip(cfg: &AppConfig, state: &CurrentRepoState, em: &EmergencyView) -> String {
    let mut lines = Vec::new();
    match &state.current {
        Some(r) => {
            lines.push(format!("GitPass — {}", title_line(cfg, r)));
            lines.push(format!("{} · {}", verdict_line(r), source_label(r.source)));
            lines.push(shorten_path(Path::new(&r.root_path)));
        }
        None if cfg.repos.is_empty() => lines.push("GitPass — conecte contas e vincule repositórios".into()),
        None => lines.push("GitPass — nenhum repositório usado recentemente".into()),
    }
    lines.extend(emergency_line(em));
    lines.join("\n")
}

fn item(app: &AppHandle, id: &str, text: &str, enabled: bool) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, enabled, None::<&str>)
}

fn info(app: &AppHandle, n: usize, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, format!("info_{n}"), text, false, None::<&str>)
}

pub fn menu(app: &AppHandle, cfg: &AppConfig, state: &CurrentRepoState, em: &EmergencyView) -> tauri::Result<Menu<Wry>> {
    let mut infos = Vec::new();
    match &state.current {
        Some(r) => {
            infos.push(info(app, 0, &format!("{} ({})", title_line(cfg, r), source_label(r.source)))?);
            infos.push(info(app, 1, &verdict_line(r))?);
        }
        None => infos.push(info(app, 0, "Nenhum repositório usado recentemente")?),
    }
    let pinned = state.current.as_ref().is_some_and(|r| r.source == CurrentRepoSource::Pinned);
    let pin = if pinned {
        item(app, MENU_UNPIN, "Desafixar", true)?
    } else {
        let can_pin = state.current.as_ref().is_some_and(|r| r.binding_id.is_some());
        item(app, MENU_PIN, "Fixar este repositório", can_pin)?
    };

    let em_status = info(app, 2, &emergency_line(em).unwrap_or_else(|| "Desligado: commits e push precisam do serviço do GitPass".into()))?;
    let em_15 = item(app, MENU_EMERGENCY_15, "Ativar por 15 min", true)?;
    let em_60 = item(app, MENU_EMERGENCY_60, "Ativar por 1 h", true)?;
    let em_off = item(app, MENU_EMERGENCY_OFF, "Desativar", em.active)?;
    let em_hint = info(app, 3, "Libera só commits locais; o push continua bloqueado")?;
    let em_sep = PredefinedMenuItem::separator(app)?;
    let emergency = Submenu::with_items(
        app,
        "Modo emergência…",
        true,
        &[&em_status, &em_hint, &em_sep, &em_15, &em_60, &em_off],
    )?;

    let show = item(app, MENU_SHOW, "Abrir GitPass", true)?;
    let hide = item(app, MENU_HIDE, "Ocultar", true)?;
    let quit = item(app, MENU_QUIT, "Sair do GitPass", true)?;
    let (s1, s2, s3) = (
        PredefinedMenuItem::separator(app)?,
        PredefinedMenuItem::separator(app)?,
        PredefinedMenuItem::separator(app)?,
    );

    let mut items: Vec<&dyn IsMenuItem<Wry>> = infos.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    items.extend([&pin as &dyn IsMenuItem<Wry>, &s1, &emergency, &s2, &show, &hide, &s3, &quit]);
    Menu::with_items(app, &items)
}

/// Replaces the tray tooltip and menu.
pub fn update(app: &AppHandle, cfg: &AppConfig, state: &CurrentRepoState, em: &EmergencyView) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let _ = tray.set_tooltip(Some(tooltip(cfg, state, em)));
    match menu(app, cfg, state, em) {
        Ok(m) => {
            let _ = tray.set_menu(Some(m));
        }
        Err(e) => eprintln!("GitPass: menu da barra: {e}"),
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitpass_core::recent::Verdict;
    use uuid::Uuid;

    fn repo(source: CurrentRepoSource, verdict: Option<Verdict>, code: Option<&str>) -> CurrentRepo {
        CurrentRepo {
            binding_id: Some(Uuid::nil().to_string()),
            root_path: "/tmp/src/app".into(),
            full_name: Some("octo/app".into()),
            account_id: Some(Uuid::nil().to_string()),
            source,
            at: "2026-11-06T22:00:00Z".into(),
            verdict,
            code: code.map(String::from),
            message: Some("ghp_secret should never show".into()),
        }
    }

    fn cfg() -> AppConfig {
        AppConfig {
            accounts: vec![gitpass_core::Account { id: Uuid::nil(), github_login: Some("octo".into()), ..Default::default() }],
            ..Default::default()
        }
    }

    fn state(r: Option<CurrentRepo>) -> CurrentRepoState {
        CurrentRepoState { current: r, pinned_binding_id: None, recent: vec![], accessibility_granted: false }
    }

    const OFF: EmergencyView = EmergencyView { active: false, until: None, started_at: None, reason: None };

    #[test]
    fn tooltip_shows_repo_account_and_verdict_in_words() {
        let t = tooltip(&cfg(), &state(Some(repo(CurrentRepoSource::Pinned, Some(Verdict::Ok), None))), &OFF);
        assert!(t.starts_with("GitPass — octo/app — @octo\nSituação: OK · fixado\n"), "{t}");
        let t = tooltip(&cfg(), &state(Some(repo(CurrentRepoSource::Recent, Some(Verdict::Blocked), Some("GP-OWNER")))), &OFF);
        assert!(t.contains("Situação: Bloqueado (GP-OWNER) · último usado"), "{t}");
        let t = tooltip(&cfg(), &state(Some(repo(CurrentRepoSource::Focus, Some(Verdict::Unconfirmed), None))), &OFF);
        assert!(t.contains("Situação: Não confirmado · app em foco"), "{t}");
        assert!(!t.contains("ghp_secret"));
        assert_eq!(tooltip(&AppConfig::default(), &state(None), &OFF), "GitPass — conecte contas e vincule repositórios");
    }

    #[test]
    fn active_emergency_is_in_the_tooltip() {
        let em = EmergencyView {
            active: true,
            until: Some("2026-11-06T22:15:00Z".into()),
            started_at: None,
            reason: Some("motivo".into()),
        };
        let t = tooltip(&cfg(), &state(None), &em);
        assert!(t.contains("Modo emergência ativo até "), "{t}");
        assert!(!t.contains("motivo"));
    }
}
