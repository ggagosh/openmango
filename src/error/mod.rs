use thiserror::Error;

mod report;

pub use report::{ErrorKind, ErrorReport, sentence};

/// Application-wide error type.
///
/// `Display` is what people read, so it carries no internal prefixes; [`ErrorReport`] keeps the
/// technical parts.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{}", ErrorReport::from_mongo("", .0).display_text())]
    Mongo(#[from] mongodb::error::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid CSV: {0}")]
    Csv(#[from] csv::Error),

    #[error("SSH: {0}")]
    Ssh(#[from] ssh2::Error),

    /// Input that couldn't be used, or a plain message with no better variant.
    #[error("{0}")]
    Parse(String),

    /// The data changed underneath the operation, e.g. a document edited elsewhere.
    #[error("{0}")]
    Conflict(String),

    /// The user stopped the operation.
    #[error("{0}")]
    Cancelled(String),

    #[error("{0}")]
    Timeout(String),

    #[error("{0}")]
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

    /// Whether the user stopped this, including after some documents were processed.
    pub fn is_cancelled(&self) -> bool {
        match self {
            Self::Cancelled(_) => true,
            Self::PartialTransfer { source, .. } => source.is_cancelled(),
            _ => false,
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
