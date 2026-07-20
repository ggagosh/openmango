use thiserror::Error;

/// Application-wide error type
#[derive(Debug, Error)]
pub enum Error {
    #[error("MongoDB error: {0}")]
    Mongo(#[from] mongodb::error::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    #[error("SSH error: {0}")]
    Ssh(#[from] ssh2::Error),

    #[error("Parse error: {0}")]
    #[allow(dead_code)]
    Parse(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Transfer failed after {processed} document(s): {source}")]
    PartialTransfer { processed: u64, source: Box<Error> },

    #[error(
        "Operation continued after {failure_count} failure(s); {processed} document(s) succeeded:\n- {}",
        details.join("\n- ")
    )]
    ContinuedOperation { processed: u64, failure_count: usize, details: Vec<String> },
}

impl Error {
    pub fn with_processed(self, processed: u64) -> Self {
        match self {
            Self::PartialTransfer { processed: inner, source } => {
                Self::PartialTransfer { processed: processed + inner, source }
            }
            Self::ContinuedOperation { processed: inner, failure_count, details } => {
                Self::ContinuedOperation { processed: processed + inner, failure_count, details }
            }
            source if processed > 0 => {
                Self::PartialTransfer { processed, source: Box::new(source) }
            }
            source => source,
        }
    }

    pub fn with_total_processed(self, processed: u64) -> Self {
        match self {
            Self::PartialTransfer { source, .. } => Self::PartialTransfer { processed, source },
            Self::ContinuedOperation { failure_count, details, .. } => {
                Self::ContinuedOperation { processed, failure_count, details }
            }
            source => Self::PartialTransfer { processed, source: Box::new(source) },
        }
    }

    pub fn processed_count(&self) -> u64 {
        match self {
            Self::PartialTransfer { processed, .. }
            | Self::ContinuedOperation { processed, .. } => *processed,
            _ => 0,
        }
    }

    pub fn failure_count(&self) -> usize {
        match self {
            Self::ContinuedOperation { failure_count, .. } => *failure_count,
            _ => 1,
        }
    }

    pub fn continued(processed: u64, failures: Vec<Error>) -> Self {
        let failure_count = failures.iter().map(Error::failure_count).sum();
        let details = failures.into_iter().map(|error| error.to_string()).collect();
        Self::ContinuedOperation { processed, failure_count, details }
    }
}

/// Convenience Result type using our Error
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continued_operation_preserves_nested_failure_counts_and_details() {
        let nested = Error::continued(
            2,
            vec![Error::Parse("first batch".to_string()), Error::Parse("second batch".to_string())],
        );
        let combined = Error::continued(5, vec![nested, Error::Parse("third batch".to_string())]);

        assert_eq!(combined.processed_count(), 5);
        assert_eq!(combined.failure_count(), 3);
        let message = combined.to_string();
        assert!(message.contains("3 failure(s)"));
        assert!(message.contains("first batch"));
        assert!(message.contains("third batch"));
    }
}
