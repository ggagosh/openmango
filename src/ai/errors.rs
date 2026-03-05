use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiErrorKind {
    Disabled,
    FeatureDisabled,
    Config,
    Auth,
    RateLimit,
    Timeout,
    Network,
    Cancelled,
    Provider,
    Parse,
    Runtime,
}

#[derive(Debug, Error)]
pub enum AiError {
    #[error("AI is disabled in settings")]
    Disabled,
    #[error("AI assistant feature is disabled for this build")]
    FeatureDisabled,
    #[error("Missing API key for provider {provider}")]
    MissingApiKey { provider: String },
    #[error("Invalid AI setting '{field}': {message}")]
    InvalidConfig { field: String, message: String },
    #[error("Unsupported provider: {0}")]
    UnsupportedProvider(String),
    #[error("{provider} authentication failed")]
    Unauthorized { provider: String },
    #[error("{provider} rate limit reached")]
    RateLimited { provider: String },
    #[error("Provider timeout: {0}")]
    Timeout(String),
    #[error("Provider request failed: {0}")]
    Network(String),
    #[error("AI request cancelled")]
    Cancelled,
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Failed to parse provider response: {0}")]
    Parse(String),
    #[error("Runtime error: {0}")]
    Runtime(String),
}

impl AiError {
    pub fn kind(&self) -> AiErrorKind {
        match self {
            Self::Disabled => AiErrorKind::Disabled,
            Self::FeatureDisabled => AiErrorKind::FeatureDisabled,
            Self::MissingApiKey { .. } | Self::InvalidConfig { .. } => AiErrorKind::Config,
            Self::UnsupportedProvider(_) => AiErrorKind::Config,
            Self::Unauthorized { .. } => AiErrorKind::Auth,
            Self::RateLimited { .. } => AiErrorKind::RateLimit,
            Self::Timeout(_) => AiErrorKind::Timeout,
            Self::Network(_) => AiErrorKind::Network,
            Self::Cancelled => AiErrorKind::Cancelled,
            Self::Provider(_) => AiErrorKind::Provider,
            Self::Parse(_) => AiErrorKind::Parse,
            Self::Runtime(_) => AiErrorKind::Runtime,
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::Disabled => "AI assistant is disabled in Settings > AI.".to_string(),
            Self::FeatureDisabled => {
                "AI assistant is disabled by feature policy for this build.".to_string()
            }
            Self::MissingApiKey { provider } => {
                format!("Missing API key for {provider}. Add it in Settings > AI.")
            }
            Self::InvalidConfig { field, message } => {
                format!("Invalid AI setting '{field}': {message}")
            }
            Self::UnsupportedProvider(provider) => {
                format!("Provider '{provider}' is not supported yet.")
            }
            Self::Unauthorized { provider } => {
                format!("{provider} rejected credentials. Check your API key.")
            }
            Self::RateLimited { provider } => {
                format!("{provider} rate limit reached. Retry shortly.")
            }
            Self::Timeout(_) => "AI request timed out. Retry or choose a faster model.".to_string(),
            Self::Network(_) => "Network error while calling AI provider.".to_string(),
            Self::Cancelled => "AI request was cancelled.".to_string(),
            Self::Provider(message) => format!("AI provider error: {message}"),
            Self::Parse(_) => "AI provider returned an invalid response.".to_string(),
            Self::Runtime(message) => format!("AI runtime failure: {message}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AiError, AiErrorKind};

    #[test]
    fn cancelled_error_maps_to_cancelled_kind_and_message() {
        let error = AiError::Cancelled;
        assert_eq!(error.kind(), AiErrorKind::Cancelled);
        assert_eq!(error.user_message(), "AI request was cancelled.");
    }
}
