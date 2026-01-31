//! Application events for reactive UI updates

use uuid::Uuid;

use crate::bson::DocumentKey;
use crate::state::SessionKey;

/// Events emitted by AppState for UI reactivity
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AppEvent {
    // Connection lifecycle
    ConnectionAdded,
    ConnectionUpdated,
    ConnectionRemoved,

    // Connection state changes
    Connecting(Uuid),
    Connected(Uuid),
    Disconnected(Uuid),
    ConnectionFailed(String),

    // Data loaded
    DatabasesLoaded(Vec<String>),
    CollectionsLoaded(Vec<String>),
    CollectionsFailed(String),
    DocumentsLoaded { session: SessionKey, total: u64 },
    DocumentInserted,
    DocumentInsertFailed { error: String },
    DocumentsInserted { count: usize },
    DocumentsInsertFailed { count: usize, error: String },
    DocumentSaved { session: SessionKey, document: DocumentKey },
    DocumentSaveFailed { session: SessionKey, error: String },
    DocumentDeleted { session: SessionKey, document: DocumentKey },
    DocumentDeleteFailed { session: SessionKey, error: String },
    IndexesLoaded { count: usize },
    IndexesLoadFailed { error: String },
    IndexDropped { name: String },
    IndexDropFailed { error: String },
    IndexCreated { session: SessionKey, name: Option<String> },
    IndexCreateFailed { session: SessionKey, error: String },
    DocumentsUpdated { session: SessionKey, matched: u64, modified: u64 },
    DocumentsUpdateFailed { session: SessionKey, error: String },
    DocumentsDeleted { session: SessionKey, deleted: u64 },
    DocumentsDeleteFailed { session: SessionKey, error: String },
    AggregationCompleted { session: SessionKey, count: usize, preview: bool, limited: bool },
    AggregationFailed { session: SessionKey, error: String },

    // Transfer events
    TransferPreviewLoaded { transfer_id: Uuid },
    TransferStarted { transfer_id: Uuid },
    TransferCompleted { transfer_id: Uuid, count: u64 },
    TransferFailed { transfer_id: Uuid, error: String },
    TransferCancelled { transfer_id: Uuid },

    // View navigation
    ViewChanged,
}
