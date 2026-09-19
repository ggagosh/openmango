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
    #[error("{provider} does not serve model {model}")]
    UnknownModel { provider: String, model: String },
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
            Self::UnknownModel { .. } => AiErrorKind::Config,
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
                format!("No API key for {provider} yet. Add one in Settings > AI.")
            }
            Self::InvalidConfig { field, message } => {
                format!("Invalid AI setting '{field}': {message}")
            }
            Self::UnsupportedProvider(provider) => {
                format!("Provider '{provider}' is not supported yet.")
            }
            Self::Unauthorized { provider } => format!(
                "{provider} did not accept the API key. Check it in Settings > AI — a key that \
                 was revoked or belongs to another account fails this way."
            ),
            Self::UnknownModel { provider, model } => format!(
                "{provider} has no model called \"{model}\". Pick another one from the model \
                 list, or Refresh it — a model can be retired, or need access your key does not \
                 have yet."
            ),
            Self::RateLimited { provider } => format!(
                "{provider} is rate limiting this key. It was already retried a few times; wait \
                 a moment, or switch to another model while it clears."
            ),
            Self::Timeout(_) => {
                "The provider did not answer in time. Ask again, or pick the Fast model for a \
                 question that does not need the big one."
                    .to_string()
            }
            Self::Network(message) => format!(
                "Could not reach the provider: {message}. Check the network, and any proxy or \
                 VPN between this machine and it."
            ),
            Self::Cancelled => "Stopped.".to_string(),
            Self::Provider(message) => format!(
                "The provider refused the request: {message}. Trying again often clears it; if \
                 it does not, another model will."
            ),
            Self::Parse(message) => format!(
                "The provider sent something this app could not read: {message}. Try again, or \
                 another model."
            ),
            Self::Runtime(message) => format!("Something went wrong inside OpenMango: {message}"),
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
        assert_eq!(error.user_message(), "Stopped.");
    }
}
