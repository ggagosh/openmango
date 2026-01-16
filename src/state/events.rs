// Application events for reactive UI updates
use uuid::Uuid;

use crate::bson::DocumentKey;
use crate::state::SessionKey;

/// Events emitted by AppState for UI reactivity
#[derive(Debug, Clone)]
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

    // View navigation
    ViewChanged,
}
