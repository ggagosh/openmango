mod backend;
mod crypto;
mod engine;
mod model;
mod store;

use std::sync::Arc;

pub use engine::{OperationEngine, OperationError};
pub use model::{
    DocumentTarget, Mutation, OperationChangePreview, OperationContext, OperationDetails,
    OperationEvent, OperationId, OperationKind, OperationOrigin, OperationPreview, OperationQuery,
    OperationStatus, OperationSummary, Page, ReconciliationReport,
};

#[cfg(test)]
pub(crate) use backend::InMemoryMutationBackend;
pub(crate) use backend::MongoMutationBackend;

pub const MAX_REVERSIBLE_BULK_DOCUMENTS: usize = 100;
pub const MAX_REVERSIBLE_BULK_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn ensure_reversible_bulk_size<'a>(
    documents: impl IntoIterator<Item = &'a mongodb::bson::Document>,
) -> Result<(), OperationError> {
    ensure_reversible_bulk_size_with_limit(documents, MAX_REVERSIBLE_BULK_BYTES)
}

fn ensure_reversible_bulk_size_with_limit<'a>(
    documents: impl IntoIterator<Item = &'a mongodb::bson::Document>,
    max_bytes: usize,
) -> Result<(), OperationError> {
    let mut bytes = 0usize;
    for document in documents {
        let encoded = mongodb::bson::to_vec(document).map_err(|_| OperationError::Internal)?;
        bytes = bytes.checked_add(encoded.len()).ok_or(OperationError::RecoveryLimitExceeded)?;
        if bytes > max_bytes {
            return Err(OperationError::RecoveryLimitExceeded);
        }
    }
    Ok(())
}

pub(crate) fn tracked_engine(
    enabled: bool,
    engine: Option<Arc<OperationEngine>>,
) -> Result<Option<Arc<OperationEngine>>, OperationError> {
    if enabled { engine.map(Some).ok_or(OperationError::Unavailable) } else { Ok(None) }
}

#[cfg(test)]
mod tests;
