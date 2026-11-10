//! Account rules CRUD (Configurações › Regras de conta).

use gitpass_core::{remove_rule, upsert_rule, AccountRule, AccountRuleInput, AppConfig};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cmd_repos::{load_config, notify};
use tauri::AppHandle;

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Tarefa interrompida: {e}"))?
}

#[derive(Serialize)]
pub struct AccountRuleRow {
    id: String,
    account_id: String,
    path_prefix: Option<String>,
    github_owner: Option<String>,
    label: Option<String>,
}

impl From<&AccountRule> for AccountRuleRow {
    fn from(r: &AccountRule) -> Self {
        Self {
            id: r.id.to_string(),
            account_id: r.account_id.to_string(),
            path_prefix: r.path_prefix.as_ref().map(|p| p.display().to_string()),
            github_owner: r.github_owner.clone(),
            label: r.label.clone(),
        }
    }
}

fn rows(cfg: &AppConfig) -> Vec<AccountRuleRow> {
    cfg.account_rules.iter().map(AccountRuleRow::from).collect()
}

#[tauri::command]
pub async fn account_rules_list() -> Result<Vec<AccountRuleRow>, String> {
    blocking(|| Ok(rows(&load_config()?))).await
}

#[derive(Deserialize)]
pub struct AccountRuleUpsert {
    id: Option<String>,
    account_id: String,
    path_prefix: Option<String>,
    github_owner: Option<String>,
    label: Option<String>,
}

#[tauri::command]
pub async fn account_rules_upsert(app: AppHandle, rule: AccountRuleUpsert) -> Result<Vec<AccountRuleRow>, String> {
    blocking(move || {
        let mut cfg = load_config()?;
        let id = rule
            .id
            .as_deref()
            .map(|s| Uuid::parse_str(s).map_err(|_| "Regra inválida.".to_string()))
            .transpose()?;
        let account_id = Uuid::parse_str(&rule.account_id).map_err(|_| "Conta inválida.".to_string())?;
        upsert_rule(
            &mut cfg,
            AccountRuleInput {
                id,
                account_id,
                path_prefix: rule.path_prefix,
                github_owner: rule.github_owner,
                label: rule.label,
            },
        )?;
        notify(&app);
        Ok(rows(&cfg))
    })
    .await
}

#[tauri::command]
pub async fn account_rules_remove(app: AppHandle, rule_id: String) -> Result<Vec<AccountRuleRow>, String> {
    blocking(move || {
        let mut cfg = load_config()?;
        let id = Uuid::parse_str(&rule_id).map_err(|_| "Regra inválida.".to_string())?;
        remove_rule(&mut cfg, id)?;
        notify(&app);
        Ok(rows(&cfg))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_tests::{expect, keys};

    #[test]
    fn row_keys_match_typescript() {
        let r = AccountRuleRow {
            id: String::new(),
            account_id: String::new(),
            path_prefix: None,
            github_owner: Some("acme".into()),
            label: Some("Clientes".into()),
        };
        assert_eq!(
            keys(&r),
            expect(&["account_id", "github_owner", "id", "label", "path_prefix"])
        );
    }
}
