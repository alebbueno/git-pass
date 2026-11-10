//! LLM integration (BYOK). Gated behind the `ai` Cargo feature — only the desktop app enables it.

mod config;
mod error;
mod provider;
mod secrets;

pub use config::{ai_config_path, AiConfig, AiConfigError, AiProviderSetting, AI_CONFIG_FILE_NAME};
pub use error::{AiError, AiErrorCode};
pub use provider::openai::{openai_recommended_model, openai_validate_key, OPENAI_API_BASE, OPENAI_CURATED_MODELS};
pub use secrets::{
    ai_key_delete, ai_key_exists, ai_key_load, ai_key_save, ApiKey, ProviderId, SecretError as AiSecretError,
};
