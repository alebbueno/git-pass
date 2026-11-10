//! Non-secret AI preferences (`ai.toml`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{app_support_dir, ensure_app_support_exists};

pub const AI_CONFIG_FILE_NAME: &str = "ai.toml";

#[derive(Debug, Error)]
pub enum AiConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML serialization: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("TOML deserialization: {0}")]
    TomlDe(#[from] toml::de::Error),
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AiProviderSetting {
    #[serde(rename = "")]
    None,
    Openai,
    Anthropic,
    Google,
}

impl Default for AiProviderSetting {
    fn default() -> Self {
        Self::None
    }
}

impl AiProviderSetting {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiLimits {
    #[serde(default = "default_calls_per_hour")]
    pub calls_per_hour: u32,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    #[serde(default = "default_max_context_chars")]
    pub max_context_chars: u32,
}

fn default_calls_per_hour() -> u32 {
    20
}
fn default_max_output_tokens() -> u32 {
    1500
}
fn default_max_context_chars() -> u32 {
    48_000
}

impl Default for AiLimits {
    fn default() -> Self {
        Self {
            calls_per_hour: default_calls_per_hour(),
            max_output_tokens: default_max_output_tokens(),
            max_context_chars: default_max_context_chars(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPrivacy {
    #[serde(default = "default_true")]
    pub pseudonymize_authors: bool,
    #[serde(default)]
    pub include_repo_name: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AiPrivacy {
    fn default() -> Self {
        Self {
            pseudonymize_authors: true,
            include_repo_name: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: AiProviderSetting,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_locale")]
    pub response_locale: String,
    #[serde(default)]
    pub consent_version: u32,
    #[serde(default)]
    pub diff_consent_version: u32,
    #[serde(default)]
    pub limits: AiLimits,
    #[serde(default)]
    pub privacy: AiPrivacy,
}

fn default_locale() -> String {
    "pt-BR".into()
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: false,
            provider: AiProviderSetting::None,
            model: String::new(),
            response_locale: default_locale(),
            consent_version: 0,
            diff_consent_version: 0,
            limits: AiLimits::default(),
            privacy: AiPrivacy::default(),
        }
    }
}

pub fn ai_config_path() -> Result<PathBuf, std::io::Error> {
    Ok(app_support_dir()?.join(AI_CONFIG_FILE_NAME))
}

impl AiConfig {
    pub fn load_or_default() -> Result<Self, AiConfigError> {
        let path = ai_config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self) -> Result<(), AiConfigError> {
        let dir = ensure_app_support_exists()?;
        let path = dir.join(AI_CONFIG_FILE_NAME);
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), AiConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let pretty = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, pretty)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ai_toml() {
        let mut cfg = AiConfig::default();
        cfg.enabled = true;
        cfg.provider = AiProviderSetting::Openai;
        cfg.consent_version = 1;
        let dir = std::env::temp_dir().join(format!("gitpass-ai-cfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai.toml");
        cfg.save_to_path(&path).unwrap();
        let back = {
            let raw = std::fs::read_to_string(&path).unwrap();
            toml::from_str(&raw).unwrap()
        };
        assert_eq!(back.enabled, true);
        assert_eq!(back.consent_version, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
