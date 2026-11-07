//! Emergency mode (DL-015/016): for a limited time, local commits pass when `gitpassd` does not
//! answer. Pushes stay blocked. Stored in `config.toml` (`ui.emergency`); every change is audited.

use std::path::Path;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::Serialize;

use crate::audit::{self, AuditKind};
use crate::config::{AppConfig, EmergencyState};

pub const MIN_MINUTES: i64 = 5;
pub const MAX_MINUTES: i64 = 240;
pub const REASON_MIN_CHARS: usize = 3;
pub const REASON_MAX_CHARS: usize = 200;

/// What the app shows (`EmergencyState` in `api-system.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmergencyView {
    pub active: bool,
    pub until: Option<String>,
    pub started_at: Option<String>,
    pub reason: Option<String>,
}

fn until_of(cfg: &AppConfig) -> Option<DateTime<Utc>> {
    let raw = cfg.ui.emergency.allow_local_commit_until.as_deref().filter(|s| !s.is_empty())?;
    DateTime::parse_from_rfc3339(raw).ok().map(|d| d.with_timezone(&Utc))
}

/// Emergency mode allows local commits at `now`.
pub fn is_active_at(cfg: &AppConfig, now: DateTime<Utc>) -> bool {
    until_of(cfg).is_some_and(|until| now < until)
}

pub fn is_active(cfg: &AppConfig) -> bool {
    is_active_at(cfg, Utc::now())
}

pub fn view_at(cfg: &AppConfig, now: DateTime<Utc>) -> EmergencyView {
    if !is_active_at(cfg, now) {
        return EmergencyView { active: false, until: None, started_at: None, reason: None };
    }
    let e = &cfg.ui.emergency;
    EmergencyView {
        active: true,
        until: e.allow_local_commit_until.clone(),
        started_at: e.started_at.clone(),
        reason: e.reason.clone(),
    }
}

fn rfc3339(d: DateTime<Utc>) -> String {
    d.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Validated reason (trimmed, single line).
pub fn check_reason(reason: &str) -> Result<String, String> {
    let r = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = r.chars().count();
    if n < REASON_MIN_CHARS {
        return Err(format!("Escreva o motivo (pelo menos {REASON_MIN_CHARS} letras)."));
    }
    if n > REASON_MAX_CHARS {
        return Err(format!("O motivo pode ter até {REASON_MAX_CHARS} caracteres."));
    }
    Ok(r)
}

pub fn check_minutes(minutes: i64) -> Result<i64, String> {
    if (MIN_MINUTES..=MAX_MINUTES).contains(&minutes) {
        Ok(minutes)
    } else {
        Err(format!("Escolha entre {MIN_MINUTES} minutos e {} horas.", MAX_MINUTES / 60))
    }
}

/// Turns emergency mode on in `cfg` (replacing an active one). `via`: `app`, `menubar` or `cli`.
pub fn start(cfg: &mut AppConfig, minutes: i64, reason: &str, via: &str, now: DateTime<Utc>) -> Result<EmergencyView, String> {
    let minutes = check_minutes(minutes)?;
    let reason = check_reason(reason)?;
    cfg.ui.emergency = EmergencyState {
        allow_local_commit_until: Some(rfc3339(now + Duration::minutes(minutes))),
        allow_local_commit_via: Some(via.to_string()),
        started_at: Some(rfc3339(now)),
        reason: Some(reason),
    };
    Ok(view_at(cfg, now))
}

/// Clears emergency mode. Returns whether it was active.
pub fn stop(cfg: &mut AppConfig, now: DateTime<Utc>) -> bool {
    let was = is_active_at(cfg, now);
    cfg.ui.emergency = EmergencyState::default();
    was
}

/// Clears an emergency that has run out. Returns whether one did.
pub fn take_expired(cfg: &mut AppConfig, now: DateTime<Utc>) -> bool {
    if until_of(cfg).is_some_and(|until| now >= until) {
        cfg.ui.emergency = EmergencyState::default();
        return true;
    }
    false
}

fn duration_label(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn save(cfg: &AppConfig) -> Result<(), String> {
    cfg.save().map_err(|e| format!("Não foi possível salvar as configurações do GitPass ({e})."))
}

fn load() -> Result<AppConfig, String> {
    AppConfig::load_or_default().map_err(|e| format!("Não foi possível ler as configurações do GitPass ({e})."))
}

fn audit_dir() -> Option<std::path::PathBuf> {
    crate::app_support_dir().ok()
}

fn audit_in(dir: Option<&Path>, kind: AuditKind, message: &str) {
    if let Some(d) = dir {
        let _ = audit::append_in(d, kind, None, None, message);
    }
}

/// Loads `config.toml`, turns emergency mode on, saves and audits (`emergency_on`).
pub fn start_now(minutes: i64, reason: &str, via: &str) -> Result<EmergencyView, String> {
    let mut cfg = load()?;
    let now = Utc::now();
    let view = start(&mut cfg, minutes, reason, via, now)?;
    save(&cfg)?;
    audit_in(
        audit_dir().as_deref(),
        AuditKind::EmergencyOn,
        &format!(
            "Modo emergência ligado por {}: {}",
            duration_label(minutes),
            view.reason.as_deref().unwrap_or_default()
        ),
    );
    Ok(view)
}

/// Loads `config.toml`, turns emergency mode off, saves and audits (`emergency_off`, or
/// `emergency_expired` when it had already run out).
pub fn stop_now(via: &str) -> Result<EmergencyView, String> {
    let mut cfg = load()?;
    let now = Utc::now();
    let had_any = cfg.ui.emergency.allow_local_commit_until.is_some();
    let was_active = stop(&mut cfg, now);
    if had_any {
        save(&cfg)?;
        if was_active {
            let who = match via {
                "menubar" => " pela barra de menus",
                "cli" => " pelo Terminal",
                _ => "",
            };
            audit_in(audit_dir().as_deref(), AuditKind::EmergencyOff, &format!("Modo emergência desligado{who}."));
        } else {
            audit_in(audit_dir().as_deref(), AuditKind::EmergencyExpired, "O modo emergência terminou.");
        }
    }
    Ok(view_at(&cfg, now))
}

/// Clears an emergency that ran out and audits `emergency_expired`. Returns whether it did.
pub fn expire_now() -> Result<bool, String> {
    let mut cfg = load()?;
    if !take_expired(&mut cfg, Utc::now()) {
        return Ok(false);
    }
    save(&cfg)?;
    audit_in(audit_dir().as_deref(), AuditKind::EmergencyExpired, "O modo emergência terminou.");
    Ok(true)
}

pub fn view_now() -> Result<EmergencyView, String> {
    Ok(view_at(&load()?, Utc::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-06T22:00:00Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn start_validates_minutes_and_reason() {
        let mut cfg = AppConfig::default();
        assert!(start(&mut cfg, 4, "servidor fora", "app", now()).is_err());
        assert!(start(&mut cfg, 241, "servidor fora", "app", now()).is_err());
        assert!(start(&mut cfg, 15, "  x ", "app", now()).is_err());
        assert!(start(&mut cfg, 15, &"a".repeat(201), "app", now()).is_err());
        assert!(!is_active_at(&cfg, now()));

        let v = start(&mut cfg, 15, "  serviço   caiu\n", "menubar", now()).unwrap();
        assert_eq!(
            v,
            EmergencyView {
                active: true,
                until: Some("2026-11-06T22:15:00Z".into()),
                started_at: Some("2026-11-06T22:00:00Z".into()),
                reason: Some("serviço caiu".into()),
            }
        );
        assert_eq!(cfg.ui.emergency.allow_local_commit_via.as_deref(), Some("menubar"));
        assert!(start(&mut cfg, 240, "abc", "app", now()).is_ok());
    }

    #[test]
    fn expiry_is_part_of_the_state() {
        let mut cfg = AppConfig::default();
        start(&mut cfg, 5, "teste", "app", now()).unwrap();
        let later = now() + Duration::minutes(5);
        assert!(is_active_at(&cfg, later - Duration::seconds(1)));
        assert!(!is_active_at(&cfg, later));
        assert!(!view_at(&cfg, later).active);
        assert_eq!(view_at(&cfg, later).reason, None);
        assert!(!take_expired(&mut cfg, later - Duration::seconds(1)));
        assert!(take_expired(&mut cfg, later));
        assert!(cfg.ui.emergency.allow_local_commit_until.is_none());
        assert!(!take_expired(&mut cfg, later));
    }

    #[test]
    fn stop_reports_whether_it_was_active() {
        let mut cfg = AppConfig::default();
        assert!(!stop(&mut cfg, now()));
        start(&mut cfg, 60, "teste", "app", now()).unwrap();
        assert!(stop(&mut cfg, now()));
        assert!(!view_at(&cfg, now()).active);
        assert_eq!(cfg.ui.emergency.reason, None);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_label(15), "15 min");
        assert_eq!(duration_label(60), "1 h");
        assert_eq!(duration_label(90), "1 h 30 min");
    }

    #[test]
    fn old_configs_without_the_new_fields_still_load() {
        let cfg: AppConfig = toml::from_str(
            "[ui]\npinned_binding_id = \"00000000-0000-0000-0000-000000000001\"\n[ui.emergency]\nallow_local_commit_until = \"2999-01-01T00:00:00Z\"\n",
        )
        .unwrap();
        assert!(cfg.ui.pinned_repo_binding_id.is_some());
        assert!(is_active_at(&cfg, now()));
        assert_eq!(view_at(&cfg, now()).reason, None);
    }
}
