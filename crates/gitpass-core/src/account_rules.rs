//! Folder / GitHub-owner rules that suggest an account for new repo bindings.

use std::path::{Component, Path, PathBuf};

use uuid::Uuid;

use crate::bindings::{AccountSuggestion, SuggestionReason};
use crate::normalize_owner;
use crate::{AccountRule, AppConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleKind {
    FolderOrg,
    Folder,
    Org,
}

fn rule_kind(rule: &AccountRule) -> Option<RuleKind> {
    let has_path = rule.path_prefix.as_ref().is_some_and(|p| !p.as_os_str().is_empty());
    let has_org = rule
        .github_owner
        .as_ref()
        .is_some_and(|o| !o.trim().is_empty());
    match (has_path, has_org) {
        (true, true) => Some(RuleKind::FolderOrg),
        (true, false) => Some(RuleKind::Folder),
        (false, true) => Some(RuleKind::Org),
        (false, false) => None,
    }
}

fn reason_for(kind: RuleKind) -> SuggestionReason {
    match kind {
        RuleKind::FolderOrg => SuggestionReason::FolderOrgRule,
        RuleKind::Folder => SuggestionReason::FolderRule,
        RuleKind::Org => SuggestionReason::OrgRule,
    }
}

/// `~/Projetos/Clientes` matches `~/Projetos/Clientes/app` but not `~/Projetos/ClientesAntigos`.
pub fn path_matches_prefix(repo: &Path, prefix: &Path) -> bool {
    let Ok(repo) = repo.canonicalize() else {
        return false;
    };
    let Ok(prefix) = prefix.canonicalize() else {
        return false;
    };
    if repo == prefix {
        return true;
    }
    // `strip_prefix` yields remaining components (e.g. `app`), not a leading `/`.
    repo.strip_prefix(&prefix)
        .ok()
        .is_some_and(|rest| !rest.as_os_str().is_empty())
}

fn path_depth(p: &Path) -> usize {
    p.components().filter(|c| matches!(c, Component::Normal(_))).count()
}

fn owner_matches(rule_owner: &str, repo_owner: &str) -> bool {
    normalize_owner(rule_owner) == normalize_owner(repo_owner)
}

fn rule_score(kind: RuleKind, path_depth: usize) -> u32 {
    let base = match kind {
        RuleKind::FolderOrg => 3000,
        RuleKind::Folder => 2000,
        RuleKind::Org => 1000,
    };
    base + path_depth as u32
}

fn display_label(rule: &AccountRule) -> String {
    if let Some(l) = rule.label.as_ref().filter(|s| !s.trim().is_empty()) {
        return l.trim().to_string();
    }
    if let Some(p) = &rule.path_prefix {
        if let Some(Component::Normal(name)) = p.components().next_back() {
            return name.to_string_lossy().into_owned();
        }
    }
    rule.github_owner.clone().unwrap_or_else(|| "regra".into())
}

/// Returns the winning rule suggestion, or `None` when no rule matches or rules conflict.
pub fn suggest_from_rules(cfg: &AppConfig, repo_root: &Path, github_owner: &str) -> Option<AccountSuggestion> {
    let mut best: Option<(u32, Uuid, SuggestionReason, String)> = None;
    for rule in &cfg.account_rules {
        let Some(kind) = rule_kind(rule) else {
            continue;
        };
        if !cfg.accounts.iter().any(|a| a.id == rule.account_id) {
            continue;
        }
        let path_ok = rule.path_prefix.as_ref().is_some_and(|p| path_matches_prefix(repo_root, p));
        let org_ok = rule
            .github_owner
            .as_ref()
            .is_some_and(|o| owner_matches(o, github_owner));
        let matches = match kind {
            RuleKind::FolderOrg => path_ok && org_ok,
            RuleKind::Folder => path_ok,
            RuleKind::Org => org_ok,
        };
        if !matches {
            continue;
        }
        let depth = rule.path_prefix.as_deref().map(path_depth).unwrap_or(0);
        let score = rule_score(kind, depth);
        let label = display_label(rule);
        match &best {
            None => best = Some((score, rule.account_id, reason_for(kind), label)),
            Some((bs, ba, _, _)) if score > *bs => best = Some((score, rule.account_id, reason_for(kind), label)),
            Some((bs, ba, _, _)) if score == *bs && *ba != rule.account_id => return None,
            Some((bs, _, _, _)) if score == *bs => {}
            Some(_) => {}
        }
    }
    best.map(|(_, account_id, reason, label)| AccountSuggestion {
        account_id,
        reason,
        rule_label: Some(label),
    })
}

/// Applies a repo path + owner to rules, then falls back to [`crate::bindings::suggest_account_offline`].
pub fn suggest_account_for_repo(cfg: &AppConfig, repo_root: &Path, owner: &str) -> Option<AccountSuggestion> {
    if let Some(s) = suggest_from_rules(cfg, repo_root, owner) {
        return Some(s);
    }
    crate::bindings::suggest_account_offline(cfg, owner)
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(p));
    }
    if p == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(p));
    }
    PathBuf::from(p)
}

pub fn normalize_path_prefix(raw: &str) -> Result<PathBuf, String> {
    let p = expand_tilde(raw.trim());
    if !p.is_absolute() {
        return Err("O caminho da pasta precisa ser absoluto.".into());
    }
    p.canonicalize()
        .map_err(|e| format!("Não foi possível usar esta pasta ({e})."))
}

pub fn normalize_rule_owner(raw: Option<String>) -> Option<String> {
    raw.filter(|s| !s.trim().is_empty())
        .map(|s| normalize_owner(&s))
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AccountRuleInput {
    pub id: Option<Uuid>,
    pub account_id: Uuid,
    pub path_prefix: Option<String>,
    pub github_owner: Option<String>,
    pub label: Option<String>,
}

pub fn upsert_rule(cfg: &mut AppConfig, input: AccountRuleInput) -> Result<AccountRule, String> {
    if !cfg.accounts.iter().any(|a| a.id == input.account_id) {
        return Err("Conta desconhecida.".into());
    }
    let path_prefix = match input.path_prefix {
        None => None,
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(normalize_path_prefix(&s)?),
    };
    let github_owner = normalize_rule_owner(input.github_owner);
    let label = input.label.filter(|s| !s.trim().is_empty());
    let draft = AccountRule {
        id: input.id.unwrap_or_else(Uuid::new_v4),
        account_id: input.account_id,
        path_prefix,
        github_owner,
        label,
    };
    if rule_kind(&draft).is_none() {
        return Err("Informe uma pasta, uma organização no GitHub, ou os dois.".into());
    }
    if let Some(i) = cfg.account_rules.iter().position(|r| r.id == draft.id) {
        cfg.account_rules[i] = draft.clone();
    } else {
        cfg.account_rules.push(draft.clone());
    }
    cfg.save().map_err(|e| e.to_string())?;
    Ok(draft)
}

pub fn remove_rule(cfg: &mut AppConfig, rule_id: Uuid) -> Result<(), String> {
    let len = cfg.account_rules.len();
    cfg.account_rules.retain(|r| r.id != rule_id);
    if cfg.account_rules.len() == len {
        return Err("Regra desconhecida.".into());
    }
    cfg.save().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Account;
    use std::fs;
    use uuid::Uuid;

    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("gitpass-rule-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p.canonicalize().unwrap()
    }

    #[test]
    fn prefix_does_not_match_sibling_folder_name() {
        let base = temp();
        let clients = base.join("Clientes");
        let other = base.join("ClientesAntigos");
        fs::create_dir_all(clients.join("app")).unwrap();
        fs::create_dir_all(&other).unwrap();
        assert!(path_matches_prefix(&clients.join("app"), &clients));
        assert!(!path_matches_prefix(&other, &clients));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn folder_rule_wins_over_org_only() {
        let base = temp();
        let repo = base.join("work").join("client-a");
        fs::create_dir_all(&repo).unwrap();
        let acc = Uuid::new_v4();
        let mut cfg = AppConfig::default();
        cfg.accounts.push(Account {
            id: acc,
            github_login: Some("orbita".into()),
            ..Default::default()
        });
        cfg.account_rules.push(AccountRule {
            id: Uuid::new_v4(),
            account_id: acc,
            path_prefix: Some(base.join("work").canonicalize().unwrap()),
            github_owner: None,
            label: Some("Clientes".into()),
        });
        let s = suggest_from_rules(&cfg, &repo, "other-org").unwrap();
        assert_eq!(s.reason, SuggestionReason::FolderRule);
        assert_eq!(s.rule_label.as_deref(), Some("Clientes"));
        let _ = fs::remove_dir_all(&base);
    }
}
