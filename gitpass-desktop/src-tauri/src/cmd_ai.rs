//! AI settings (BYOK). No repository data leaves the app in Fase 0.

use gitpass_core::{
    ai_key_delete, ai_key_exists, ai_key_load, ai_key_save, openai_recommended_model, openai_validate_key,
    AiConfig, AiError, AiProviderSetting, ApiKey, ProviderId, OPENAI_CURATED_MODELS,
};

use serde::Serialize;

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Tarefa interrompida: {e}"))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub response_locale: String,
    pub consent_version: u32,
    pub openai_key_configured: bool,
    pub anthropic_key_configured: bool,
    pub google_key_configured: bool,
    pub curated_models: Vec<String>,
    pub recommended_model: String,
    pub config_path: String,
}

fn load_cfg() -> Result<AiConfig, String> {
    AiConfig::load_or_default().map_err(|e| format!("Não foi possível ler ai.toml ({e})."))
}

fn save_cfg(cfg: &AiConfig) -> Result<(), String> {
    cfg.save().map_err(|e| format!("Não foi possível gravar ai.toml ({e})."))
}

fn map_ai_err(e: AiError) -> String {
    format!("{}", e)
}

#[tauri::command]
pub async fn ai_status() -> Result<AiStatus, String> {
    blocking(move || {
        let cfg = load_cfg()?;
        let path = gitpass_core::ai_config_path().map_err(|e| e.to_string())?;
        let model = if cfg.model.is_empty() {
            openai_recommended_model().to_string()
        } else {
            cfg.model.clone()
        };
        Ok(AiStatus {
            enabled: cfg.enabled,
            provider: cfg.provider.as_str().to_string(),
            model,
            response_locale: cfg.response_locale.clone(),
            consent_version: cfg.consent_version,
            openai_key_configured: ai_key_exists(ProviderId::Openai),
            anthropic_key_configured: ai_key_exists(ProviderId::Anthropic),
            google_key_configured: ai_key_exists(ProviderId::Google),
            curated_models: OPENAI_CURATED_MODELS.iter().map(|s| (*s).to_string()).collect(),
            recommended_model: openai_recommended_model().to_string(),
            config_path: path.display().to_string(),
        })
    })
    .await
}

#[tauri::command]
pub async fn ai_set_enabled(enabled: bool, consent_version: u32) -> Result<AiStatus, String> {
    blocking(move || {
        let mut cfg = load_cfg()?;
        if enabled {
            if consent_version < 1 {
                return Err("Aceite o aviso de privacidade antes de ativar.".into());
            }
            if !ai_key_exists(ProviderId::Openai) {
                return Err("GP-AI-002: Salve uma chave OpenAI antes de ativar.".into());
            }
            cfg.consent_version = consent_version.max(cfg.consent_version);
        }
        cfg.enabled = enabled;
        save_cfg(&cfg)?;
        drop(cfg);
        ai_status_sync()
    })
    .await
}

fn ai_status_sync() -> Result<AiStatus, String> {
    let cfg = load_cfg()?;
    let path = gitpass_core::ai_config_path().map_err(|e| e.to_string())?;
    let model = if cfg.model.is_empty() {
        openai_recommended_model().to_string()
    } else {
        cfg.model.clone()
    };
    Ok(AiStatus {
        enabled: cfg.enabled,
        provider: cfg.provider.as_str().to_string(),
        model,
        response_locale: cfg.response_locale.clone(),
        consent_version: cfg.consent_version,
        openai_key_configured: ai_key_exists(ProviderId::Openai),
        anthropic_key_configured: ai_key_exists(ProviderId::Anthropic),
        google_key_configured: ai_key_exists(ProviderId::Google),
        curated_models: OPENAI_CURATED_MODELS.iter().map(|s| (*s).to_string()).collect(),
        recommended_model: openai_recommended_model().to_string(),
        config_path: path.display().to_string(),
    })
}

#[tauri::command]
pub async fn ai_set_key(provider: String, key: String) -> Result<AiStatus, String> {
    blocking(move || {
        let pid = ProviderId::parse(&provider).ok_or_else(|| "Provedor desconhecido.".to_string())?;
        if pid != ProviderId::Openai {
            return Err("Na versão atual só OpenAI é suportado.".into());
        }
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err("Informe a chave.".into());
        }
        let api = ApiKey::new(trimmed.to_string());
        ai_key_save(pid, &api).map_err(|e| format!("Keychain: {e}"))?;
        let mut cfg = load_cfg()?;
        cfg.provider = AiProviderSetting::Openai;
        if cfg.model.is_empty() {
            cfg.model = openai_recommended_model().to_string();
        }
        save_cfg(&cfg)?;
        Ok(ai_status_sync()?)
    })
    .await
}

#[tauri::command]
pub async fn ai_test_key(provider: String) -> Result<String, String> {
    blocking(move || {
        let pid = ProviderId::parse(&provider).ok_or_else(|| "Provedor desconhecido.".to_string())?;
        if pid != ProviderId::Openai {
            return Err("Na versão atual só OpenAI é suportado.".into());
        }
        let Some(key) = ai_key_load(pid).map_err(|e| format!("Keychain: {e}"))? else {
            return Err("GP-AI-002: Nenhuma chave salva.".into());
        };
        openai_validate_key(&key).map_err(map_ai_err)?;
        Ok("Chave reconhecida pelo provedor. Saldo e limites não são verificados.".into())
    })
    .await
}

#[tauri::command]
pub async fn ai_remove_key(provider: String) -> Result<AiStatus, String> {
    blocking(move || {
        let pid = ProviderId::parse(&provider).ok_or_else(|| "Provedor desconhecido.".to_string())?;
        ai_key_delete(pid).map_err(|e| format!("Keychain: {e}"))?;
        let mut cfg = load_cfg()?;
        if cfg.provider == AiProviderSetting::Openai && pid == ProviderId::Openai {
            cfg.enabled = false;
            cfg.provider = AiProviderSetting::None;
        }
        save_cfg(&cfg)?;
        Ok(ai_status_sync()?)
    })
    .await
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPreferencesPatch {
    pub model: Option<String>,
    pub response_locale: Option<String>,
}

#[tauri::command]
pub async fn ai_set_preferences(patch: AiPreferencesPatch) -> Result<AiStatus, String> {
    blocking(move || {
        let mut cfg = load_cfg()?;
        if let Some(m) = patch.model {
            let m = m.trim();
            if m.is_empty() {
                cfg.model = openai_recommended_model().to_string();
            } else if !OPENAI_CURATED_MODELS.iter().any(|&c| c == m) {
                return Err("GP-AI-010: Modelo fora da lista curada.".into());
            } else {
                cfg.model = m.to_string();
            }
        }
        if let Some(loc) = patch.response_locale {
            let loc = loc.trim();
            if !loc.is_empty() {
                cfg.response_locale = loc.to_string();
            }
        }
        save_cfg(&cfg)?;
        Ok(ai_status_sync()?)
    })
    .await
}
