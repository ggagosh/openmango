//! Global application state.

mod connection;
mod database_sessions;
mod selection;
mod sessions;
mod status;
mod tabs;
mod types;
mod workspace;

pub use database_sessions::DatabaseSessionStore;
pub use sessions::SessionStore;
pub use types::{
    ActiveTab, CollectionOverview, CollectionStats, CollectionSubview, ConnectionState,
    DatabaseKey, DatabaseSessionData, DatabaseSessionState, DatabaseStats, SessionData,
    SessionDocument, SessionKey, SessionState, SessionViewState, TabKey, TabState, View,
};

use gpui::EventEmitter;

use crate::models::connection::SavedConnection;
use crate::state::StatusMessage;
use crate::state::events::AppEvent;
use crate::state::{ConfigManager, WorkspaceState};

use types::*;

/// Global application state
pub struct AppState {
    // Persisted state
    pub connections: Vec<SavedConnection>,

    // Organized sub-states
    pub conn: ConnectionState,
    pub tabs: TabState,
    pub sessions: SessionStore,
    pub db_sessions: DatabaseSessionStore,

    // View state
    pub current_view: View,
    pub status_message: Option<StatusMessage>,

    // Config manager for persistence
    pub(crate) config: ConfigManager,

    // Workspace persistence
    pub workspace: WorkspaceState,
    pub(crate) workspace_restore_pending: bool,
}

impl AppState {
    /// Create new AppState, loading persisted data from disk
    pub fn new() -> Self {
        let config = ConfigManager::default();

        // Load saved connections
        let connections = config.load_connections().unwrap_or_else(|e| {
            log::warn!("Failed to load connections: {}", e);
            Vec::new()
        });
        let workspace = config.load_workspace().unwrap_or_else(|e| {
            log::warn!("Failed to load workspace: {}", e);
            WorkspaceState::default()
        });
        let workspace_restore_pending = workspace.last_connection_id.is_some();

        Self {
            connections,
            conn: ConnectionState::default(),
            tabs: TabState::default(),
            sessions: SessionStore::new(),
            db_sessions: DatabaseSessionStore::new(),
            current_view: View::Welcome,
            status_message: None,
            config,
            workspace,
            workspace_restore_pending,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// Enable reactive UI updates via event subscription
impl EventEmitter<AppEvent> for AppState {}
