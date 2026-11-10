use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiErrorCode {
    Disabled,
    NoKey,
    InvalidKey,
    Quota,
    LocalLimit,
    Network,
    Provider,
    ContextTooLarge,
    EmptyResponse,
    BadModel,
    BadFormat,
    Keychain,
    PrepareExpired,
}

impl AiErrorCode {
    pub fn gp_code(self) -> &'static str {
        match self {
            Self::Disabled => "GP-AI-001",
            Self::NoKey => "GP-AI-002",
            Self::InvalidKey => "GP-AI-003",
            Self::Quota => "GP-AI-004",
            Self::LocalLimit => "GP-AI-005",
            Self::Network => "GP-AI-006",
            Self::Provider => "GP-AI-007",
            Self::ContextTooLarge => "GP-AI-008",
            Self::EmptyResponse => "GP-AI-009",
            Self::BadModel => "GP-AI-010",
            Self::BadFormat => "GP-AI-011",
            Self::Keychain => "GP-AI-012",
            Self::PrepareExpired => "GP-AI-013",
        }
    }
}

#[derive(Debug)]
pub struct AiError {
    pub code: AiErrorCode,
    pub message: String,
}

impl AiError {
    pub fn new(code: AiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.gp_code(), self.message)
    }
}

impl std::error::Error for AiError {}
