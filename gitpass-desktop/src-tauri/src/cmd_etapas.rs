//! Etapas 3–4: work radar, command helpers, config history, commit review, PR inbox.

use gitpass_core::bindings::{read_repo_identity, set_repo_identity};
use gitpass_core::commit_review::{binding_commit_review, CommitReviewRow};
use gitpass_core::config_history::{
    list_records, remove_record, undo_snapshot, ConfigHistoryOp, ConfigHistoryRecord, UndoError,
};
use gitpass_core::pr_inbox::{fetch_pr_inbox, PrInboxItem};
use gitpass_core::work_radar::{inspect_work_tree, WorkTreeStatus};
use gitpass_core::AppConfig;
use serde::Serialize;
use uuid::Uuid;

use crate::cmd_cloud::current_token;
use crate::cmd_repos::{bound_repo_root, load_config, notify};
use tauri::AppHandle;

fn parse_id(raw: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw).map_err(|_| "Identificador inválido.".to_string())
}

#[derive(Serialize)]
pub struct WorkStatusRow {
    binding_id: String,
    #[serde(flatten)]
    status: WorkTreeStatus,
}

#[tauri::command]
pub async fn repos_work_status(binding_ids: Vec<String>) -> Result<Vec<WorkStatusRow>, String> {
    crate::cmd_repos::blocking(move || {
        let cfg = load_config()?;
        let mut out = Vec::new();
        for raw in binding_ids {
            let Ok(id) = parse_id(&raw) else { continue };
            let Some(b) = cfg.repos.iter().find(|r| r.id == id) else { continue };
            let status = if b.root_path.is_dir() {
                inspect_work_tree(&b.root_path)
            } else {
                WorkTreeStatus::unreadable("A pasta não existe.")
            };
            out.push(WorkStatusRow {
                binding_id: id.to_string(),
                status,
            });
        }
        Ok(out)
    })
    .await
}

#[derive(Serialize)]
pub struct ConfigHistoryRow {
    id: String,
    at: String,
    binding_id: String,
    op: String,
    status: String,
    undoable: bool,
    message: Option<String>,
    before_account_id: Option<String>,
    after_account_id: Option<String>,
    before_name: Option<String>,
    before_email: Option<String>,
    after_name: Option<String>,
    after_email: Option<String>,
}

impl From<ConfigHistoryRecord> for ConfigHistoryRow {
    fn from(r: ConfigHistoryRecord) -> Self {
        Self {
            id: r.id.to_string(),
            at: r.at,
            binding_id: r.binding_id.to_string(),
            op: serde_json::to_value(r.op).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            status: serde_json::to_value(r.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            undoable: r.undoable,
            message: r.message,
            before_account_id: r.before_account_id.map(|u| u.to_string()),
            after_account_id: r.after_account_id.map(|u| u.to_string()),
            before_name: r.before_identity.name,
            before_email: r.before_identity.email,
            after_name: r.after_identity.name,
            after_email: r.after_identity.email,
        }
    }
}

#[tauri::command]
pub async fn repos_config_history_list() -> Result<Vec<ConfigHistoryRow>, String> {
    crate::cmd_repos::blocking(|| Ok(list_records().into_iter().map(ConfigHistoryRow::from).collect())).await
}

#[tauri::command]
pub async fn repos_config_history_undo(app: AppHandle, record_id: String) -> Result<(), String> {
    crate::cmd_repos::blocking(move || undo_record(&app, &record_id)).await
}

fn undo_record(app: &AppHandle, record_id: &str) -> Result<(), String> {
    let id = parse_id(record_id)?;
    let rec = undo_snapshot(id).map_err(|e| e.message())?;
    let mut cfg = load_config()?;
    let binding = cfg
        .repos
        .iter()
        .find(|b| b.id == rec.binding_id)
        .cloned()
        .ok_or_else(|| UndoError::BindingMissing.message())?;
    if !binding.root_path.is_dir() {
        return Err("A pasta do repositório não existe.".into());
    }
    let current = read_repo_identity(&binding.root_path);
    if current != rec.after_identity {
        return Err(UndoError::Conflict {
            current,
            expected_after: rec.after_identity.clone(),
        }
        .message());
    }
    match rec.op {
        ConfigHistoryOp::ChangeAccount => {
            let before = rec.before_account_id.ok_or_else(|| "Registro incompleto.".to_string())?;
            let Some(repo) = cfg.repos.iter_mut().find(|b| b.id == rec.binding_id) else {
                return Err(UndoError::BindingMissing.message());
            };
            repo.account_id = before;
            cfg.save().map_err(|e| e.to_string())?;
            set_repo_identity(&binding.root_path, &rec.before_identity)
                .map_err(|e| format!("Não foi possível restaurar a identidade: {e}"))?;
        }
        ConfigHistoryOp::FixIdentity => {
            set_repo_identity(&binding.root_path, &rec.before_identity)
                .map_err(|e| format!("Não foi possível restaurar a identidade: {e}"))?;
        }
    }
    let _ = remove_record(id);
    notify(app);
    Ok(())
}

#[tauri::command]
pub async fn repos_commit_review(binding_id: String, limit: Option<usize>) -> Result<Vec<CommitReviewRow>, String> {
    crate::cmd_repos::blocking(move || {
        let id = parse_id(&binding_id)?;
        let cfg = load_config()?;
        binding_commit_review(&cfg, id, limit.unwrap_or(40))
    })
    .await
}

#[tauri::command]
pub async fn cloud_pr_inbox(account_id: String) -> Result<Vec<PrInboxItem>, String> {
    crate::cmd_repos::blocking(move || {
        let id = parse_id(&account_id)?;
        let cfg = AppConfig::load_or_default().map_err(|e| e.to_string())?;
        let login = cfg
            .accounts
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| a.github_login.clone())
            .ok_or_else(|| "Conta desconhecida.".to_string())?;
        let token = current_token(id)?;
        fetch_pr_inbox(&token, &login).map_err(crate::cmd_cloud::api_error_message)
    })
    .await
}

#[tauri::command]
pub fn repos_open_in_editor(path: String, editor_app: Option<String>) -> Result<(), String> {
    let root = bound_repo_root(&path)?;
    let app = editor_app.as_deref().unwrap_or("Cursor");
    crate::cmd_repos::open_with(&["-a", app], &root)
}
