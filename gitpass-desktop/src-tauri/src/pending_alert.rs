//! Aviso nativo quando sobe a quantidade de commits ou repositórios com push pendente.

use crate::cmd_repos::PendingPushSummary;

/// Só notifica após o primeiro refresh (baseline) e quando a contagem **aumenta**.
pub fn should_notify(prev: Option<&PendingPushSummary>, next: &PendingPushSummary) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    if next.repos == 0 {
        return false;
    }
    next.repos > prev.repos || next.commits > prev.commits
}

pub fn notification_body(pending: &PendingPushSummary) -> String {
    if pending.repos == 1 {
        if pending.commits == 1 {
            "1 commit para enviar em 1 repositório.".to_string()
        } else {
            format!(
                "{} commits para enviar em 1 repositório.",
                pending.commits.to_string()
            )
        }
    } else if pending.commits == 1 {
        format!("1 commit para enviar em {} repositórios.", pending.repos)
    } else {
        format!(
            "{} commits para enviar em {} repositórios.",
            pending.commits, pending.repos
        )
    }
}

#[cfg(target_os = "macos")]
pub fn notify(pending: &PendingPushSummary) {
    use notify_rust::Notification;
    let body = notification_body(pending);
    let _ = Notification::new()
        .appname("GitPass")
        .summary("Commits aguardando envio")
        .body(&body)
        .show();
}

#[cfg(not(target_os = "macos"))]
pub fn notify(_pending: &PendingPushSummary) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_only_when_counts_rise() {
        let zero = PendingPushSummary { repos: 0, commits: 0 };
        let one = PendingPushSummary { repos: 1, commits: 2 };
        let two = PendingPushSummary { repos: 2, commits: 2 };
        let more = PendingPushSummary { repos: 2, commits: 5 };

        assert!(!should_notify(None, &one));
        assert!(!should_notify(Some(&zero), &zero));
        assert!(should_notify(Some(&zero), &one));
        assert!(should_notify(Some(&one), &two));
        assert!(should_notify(Some(&two), &more));
        assert!(!should_notify(Some(&more), &PendingPushSummary { repos: 1, commits: 3 }));
    }

    #[test]
    fn body_in_portuguese() {
        assert_eq!(
            notification_body(&PendingPushSummary { repos: 1, commits: 1 }),
            "1 commit para enviar em 1 repositório."
        );
        assert_eq!(
            notification_body(&PendingPushSummary { repos: 2, commits: 5 }),
            "5 commits para enviar em 2 repositórios."
        );
    }
}
