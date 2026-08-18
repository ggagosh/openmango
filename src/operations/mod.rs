mod backend;
mod crypto;
mod engine;
mod model;
mod snapshot;
mod store;

use std::sync::Arc;

pub use engine::{OperationEngine, OperationError};
pub use model::{
    CollectionTarget, DocumentTarget, Mutation, OperationChangePreview, OperationContext,
    OperationDetails, OperationEvent, OperationId, OperationKind, OperationOrigin,
    OperationPreview, OperationQuery, OperationStatus, OperationSummary, Page,
    ReconciliationReport,
};

#[cfg(test)]
pub(crate) use backend::InMemoryMutationBackend;
pub(crate) use backend::MongoMutationBackend;

pub const MAX_REVERSIBLE_BULK_DOCUMENTS: usize = 100;
pub const MAX_REVERSIBLE_BULK_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn ensure_reversible_bulk_size<'a>(
    documents: impl IntoIterator<Item = &'a mongodb::bson::Document>,
) -> Result<(), OperationError> {
    reversible_bulk_size(documents).map(|_| ())
}

pub(crate) fn reversible_bulk_size<'a>(
    documents: impl IntoIterator<Item = &'a mongodb::bson::Document>,
) -> Result<usize, OperationError> {
    ensure_reversible_bulk_size_with_limit(documents, MAX_REVERSIBLE_BULK_BYTES)
}

fn ensure_reversible_bulk_size_with_limit<'a>(
    documents: impl IntoIterator<Item = &'a mongodb::bson::Document>,
    max_bytes: usize,
) -> Result<usize, OperationError> {
    let mut bytes = 0usize;
    for document in documents {
        bytes = bytes
            .checked_add(reversible_document_size(document)?)
            .ok_or(OperationError::RecoveryLimitExceeded)?;
        if bytes > max_bytes {
            return Err(OperationError::RecoveryLimitExceeded);
        }
    }
    Ok(bytes)
}

pub(crate) fn reversible_document_size(
    document: &mongodb::bson::Document,
) -> Result<usize, OperationError> {
    mongodb::bson::to_vec(document)
        .map(|encoded| encoded.len())
        .map_err(|_| OperationError::Internal)
}

pub(crate) fn tracked_engine(
    enabled: bool,
    engine: Option<Arc<OperationEngine>>,
) -> Result<Option<Arc<OperationEngine>>, OperationError> {
    if enabled { engine.map(Some).ok_or(OperationError::Unavailable) } else { Ok(None) }
}

#[cfg(test)]
mod tests;
