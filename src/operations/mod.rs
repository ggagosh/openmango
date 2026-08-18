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

pub(crate) use backend::MongoMutationBackend;

pub const MAX_REVERSIBLE_BULK_DOCUMENTS: usize = 100;
pub const MAX_REVERSIBLE_BULK_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn tracked_engine(
    enabled: bool,
    engine: Option<Arc<OperationEngine>>,
) -> Result<Option<Arc<OperationEngine>>, OperationError> {
    if enabled { engine.map(Some).ok_or(OperationError::Unavailable) } else { Ok(None) }
}

#[cfg(test)]
mod tests;
