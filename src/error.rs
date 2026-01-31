use thiserror::Error;

/// Application-wide error type
#[derive(Debug, Error)]
pub enum Error {
    #[error("MongoDB error: {0}")]
    Mongo(#[from] mongodb::error::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Postcard(#[from] postcard::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    #[error("Parse error: {0}")]
    #[allow(dead_code)]
    Parse(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),
}

/// Convenience Result type using our Error
pub type Result<T> = std::result::Result<T, Error>;

// No custom helpers needed currently.
