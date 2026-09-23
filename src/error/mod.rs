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

    /// Whether trying again later can succeed: a dropped connection, a timeout, a primary that
    /// stepped down. A wrong password, a rejected document or a full disk won't fix themselves.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Mongo(error) => mongo_is_transient(error),
            Self::Io(error) => matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::UnexpectedEof
            ),
            // A dropped SSH tunnel is a dropped connection.
            Self::Ssh(_) | Self::Timeout(_) => true,
            Self::PartialTransfer { source, .. } => source.is_transient(),
            _ => false,
        }
    }

    pub fn continued(processed: u64, failures: Vec<Error>) -> Self {
        let failure_count = failures.iter().map(Error::failure_count).sum();
        let details = failures.into_iter().map(|error| error.to_string()).collect();
        Self::ContinuedOperation { processed, failure_count, details }
    }
}

/// Server errors the driver itself retries once: not primary, node recovering, shutting down,
/// host unreachable, network timeout and the like. Its list isn't public; this is the union of its
/// retryable read and write codes in mongodb 3.5.1 (`src/error.rs`). Recheck on upgrade.
const RETRYABLE_CODES: [i32; 13] =
    [11600, 11602, 10107, 13435, 13436, 189, 91, 7, 6, 89, 9001, 134, 262];

fn mongo_is_transient(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    if error.contains_label("RetryableWriteError")
        || error.contains_label("ResumableChangeStreamError")
    {
        return true;
    }
    match error.kind.as_ref() {
        ErrorKind::Io(_)
        | ErrorKind::ServerSelection { .. }
        | ErrorKind::ConnectionPoolCleared { .. }
        | ErrorKind::DnsResolve { .. } => true,
        ErrorKind::Command(command) => RETRYABLE_CODES.contains(&command.code),
        ErrorKind::Write(WriteFailure::WriteConcernError(concern)) => {
            RETRYABLE_CODES.contains(&concern.code)
        }
        ErrorKind::InsertMany(many) => many
            .write_concern_error
            .as_ref()
            .is_some_and(|concern| RETRYABLE_CODES.contains(&concern.code)),
        _ => false,
    }
}

/// A failure reported through a progress channel, which carries text: the text, and whether
/// trying again can help, decided while the error itself was still at hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub transient: bool,
}

impl From<&Error> for Failure {
    fn from(error: &Error) -> Self {
        Self { message: error.to_string(), transient: error.is_transient() }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::from(&error)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Convenience Result type using our Error
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_connections_are_transient_and_bad_input_is_not() {
        let reset = Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert!(reset.is_transient());
        assert!(Error::Timeout("slow".into()).is_transient());
        let partial = Error::Io(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        assert!(partial.with_processed(10).is_transient(), "after some documents too");
        assert!(!Error::Parse("bad filter".into()).is_transient());
        assert!(!Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound)).is_transient());
        let not_primary = mongodb::error::Error::custom("x");
        assert!(!Error::Mongo(not_primary).is_transient(), "an unknown driver error isn't");
        let failure = Failure::from(&reset);
        assert!(failure.transient);
        assert_eq!(failure.to_string(), reset.to_string());
    }

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
