//! Undoable binding operations (identity / account), local JSON store.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bindings::{read_repo_identity, RepoIdentity};
use crate::{app_support_dir, ensure_app_support_exists};

const FILE_NAME: &str = "config_history.json";
const MAX_RECORDS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigHistoryOp {
    ChangeAccount,
    FixIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigHistoryStatus {
    Applied,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigHistoryRecord {
    pub id: Uuid,
    pub at: String,
    pub binding_id: Uuid,
    pub op: ConfigHistoryOp,
    pub status: ConfigHistoryStatus,
    #[serde(default)]
    pub before_account_id: Option<Uuid>,
    #[serde(default)]
    pub after_account_id: Option<Uuid>,
    pub before_identity: RepoIdentity,
    pub after_identity: RepoIdentity,
    pub undoable: bool,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    records: Vec<ConfigHistoryRecord>,
}

fn path() -> Result<PathBuf, String> {
    Ok(app_support_dir().map_err(|e| e.to_string())?.join(FILE_NAME))
}

pub fn list_records() -> Vec<ConfigHistoryRecord> {
    load_file().records
}

fn load_file() -> HistoryFile {
    let Ok(p) = path() else {
        return HistoryFile::default();
    };
    fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_file(f: &HistoryFile) -> Result<(), String> {
    let dir = ensure_app_support_exists().map_err(|e| e.to_string())?;
    let p = dir.join(FILE_NAME);
    let tmp = p.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn append_record(rec: ConfigHistoryRecord) -> Result<Uuid, String> {
    let mut f = load_file();
    f.records.insert(0, rec.clone());
    f.records.truncate(MAX_RECORDS);
    save_file(&f)?;
    Ok(rec.id)
}

pub fn record_change_account(
    binding_id: Uuid,
    before_account: Uuid,
    after_account: Uuid,
    before_identity: RepoIdentity,
    after_identity: RepoIdentity,
    status: ConfigHistoryStatus,
    message: Option<String>,
) -> Result<Uuid, String> {
    append_record(ConfigHistoryRecord {
        id: Uuid::new_v4(),
        at: chrono::Utc::now().to_rfc3339(),
        binding_id,
        op: ConfigHistoryOp::ChangeAccount,
        status,
        before_account_id: Some(before_account),
        after_account_id: Some(after_account),
        before_identity,
        after_identity,
        undoable: status == ConfigHistoryStatus::Applied,
        message,
    })
}

pub fn record_fix_identity(
    binding_id: Uuid,
    root: &Path,
    before: RepoIdentity,
    status: ConfigHistoryStatus,
    message: Option<String>,
) -> Result<Uuid, String> {
    append_record(ConfigHistoryRecord {
        id: Uuid::new_v4(),
        at: chrono::Utc::now().to_rfc3339(),
        binding_id,
        op: ConfigHistoryOp::FixIdentity,
        status,
        before_account_id: None,
        after_account_id: None,
        before_identity: before,
        after_identity: read_repo_identity(root),
        undoable: status == ConfigHistoryStatus::Applied,
        message,
    })
}

// read identity after fix

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoError {
    NotFound,
    NotUndoable,
    BindingMissing,
    Conflict { current: RepoIdentity, expected_after: RepoIdentity },
}

impl UndoError {
    pub fn message(&self) -> String {
        match self {
            Self::NotFound => "Registro não encontrado.".into(),
            Self::NotUndoable => "Esta operação não pode ser desfeita.".into(),
            Self::BindingMissing => "O vínculo não existe mais.".into(),
            Self::Conflict { .. } => {
                "A identidade local mudou desde a operação. Revise manualmente antes de desfazer.".into()
            }
        }
    }
}

/// Restores account (when applicable) and Git identity from the snapshot; does not write `config.toml` alone —
/// caller must set `account_id` and save when op is `ChangeAccount`.
pub fn undo_snapshot(record_id: Uuid) -> Result<ConfigHistoryRecord, UndoError> {
    let f = load_file();
    let Some(rec) = f.records.iter().find(|r| r.id == record_id).cloned() else {
        return Err(UndoError::NotFound);
    };
    if !rec.undoable {
        return Err(UndoError::NotUndoable);
    }
    Ok(rec)
}

pub fn remove_record(id: Uuid) -> Result<(), String> {
    let mut f = load_file();
    let len = f.records.len();
    f.records.retain(|r| r.id != id);
    if f.records.len() == len {
        return Err("Registro não encontrado.".into());
    }
    save_file(&f)
}
