//! Ícone do Dock (badge com quantidade de repositórios com push pendente).

use tauri::{AppHandle, Manager};

use crate::cmd_repos::PendingPushSummary;

fn badge_count(pending: &PendingPushSummary) -> Option<i64> {
    if pending.repos == 0 {
        None
    } else {
        Some(pending.repos.min(99) as i64)
    }
}

/// Atualiza o badge no Dock / taskbar (macOS: contagem no ícone do app).
pub fn update(app: &AppHandle, pending: &PendingPushSummary) {
    let count = badge_count(pending);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_badge_count(count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_caps_at_99() {
        let p = PendingPushSummary { repos: 120, commits: 200 };
        assert_eq!(badge_count(&p), Some(99));
        assert_eq!(badge_count(&PendingPushSummary { repos: 0, commits: 0 }), None);
    }
}
