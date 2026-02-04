//! Global application state.

mod aggregation;
mod connection;
mod database_sessions;
mod forge;
mod selection;
mod sessions;
mod status;
mod tabs;
mod transfer;
mod types;
mod workspace;

pub(crate) use aggregation::{
    PipelineAnalysis, PipelineStage, PipelineState, StageDocCounts, StageStatsMode,
    default_stage_body,
};
pub(crate) use database_sessions::DatabaseSessionStore;
pub(crate) use sessions::SessionStore;
pub use types::{
    ActiveTab, BsonOutputFormat, CollectionOverview, CollectionProgress, CollectionStats,
    CollectionSubview, CollectionTransferStatus, CompressionMode, CopiedTreeItem, DatabaseKey,
    DatabaseSessionData, DatabaseSessionState, DatabaseStats, DatabaseTransferProgress, Encoding,
    ExtendedJsonMode, ForgeTabKey, ForgeTabState, InsertMode, SessionData, SessionDocument,
    SessionKey, SessionState, SessionViewState, TabKey, TransferFormat, TransferMode,
    TransferScope, TransferTabKey, TransferTabState, View,
};

use std::collections::HashMap;
use std::sync::{Arc, atomic::AtomicU64};

use gpui::{Context, EventEmitter};

use crate::connection::ConnectionManager;
use crate::models::connection::SavedConnection;
use crate::state::StatusMessage;
use crate::state::events::AppEvent;
use crate::state::settings::AppSettings;
use crate::state::{ConfigManager, WorkspaceState};

use types::*;

/// Global application state
pub struct AppState {
    // Persisted state
    pub connections: Vec<SavedConnection>,
    pub settings: AppSettings,

    // Connection manager (injected for testability)
    connection_manager: Arc<ConnectionManager>,

    // Organized sub-states
    conn: ConnectionState,
    tabs: TabState,
    sessions: SessionStore,
    db_sessions: DatabaseSessionStore,
    transfer_tabs: HashMap<uuid::Uuid, TransferTabState>,
    forge_tabs: HashMap<uuid::Uuid, ForgeTabState>,

    // View state
    pub current_view: View,
    status_message: Option<StatusMessage>,

    /// Copied tree item for paste operation (internal clipboard)
    pub copied_tree_item: Option<CopiedTreeItem>,

    // Config manager for persistence
    pub(crate) config: ConfigManager,

    // Workspace persistence
    pub workspace: WorkspaceState,
    pub(crate) workspace_restore_pending: bool,
    aggregation_workspace_save_gen: Arc<AtomicU64>,
}

impl AppState {
    /// Create new AppState, loading persisted data from disk
    pub fn new() -> Self {
        Self::with_connection_manager(Arc::new(ConnectionManager::new()))
    }

    /// Create new AppState with a custom ConnectionManager (for testing)
    pub fn with_connection_manager(connection_manager: Arc<ConnectionManager>) -> Self {
        let config = ConfigManager::default();

        // Load saved connections
        let connections = config.load_connections().unwrap_or_else(|e| {
            log::warn!("Failed to load connections: {}", e);
            Vec::new()
        });
        let settings = config.load_settings().unwrap_or_else(|e| {
            log::warn!("Failed to load settings: {}", e);
            AppSettings::default()
        });
        let workspace = config.load_workspace().unwrap_or_else(|e| {
            log::warn!("Failed to load workspace: {}", e);
            WorkspaceState::default()
        });
        let workspace_restore_pending = workspace.last_connection_id.is_some();
        let aggregation_workspace_save_gen = Arc::new(AtomicU64::new(0));

        Self {
            connections,
            settings,
            connection_manager,
            conn: ConnectionState::default(),
            tabs: TabState::default(),
            sessions: SessionStore::new(),
            db_sessions: DatabaseSessionStore::new(),
            transfer_tabs: HashMap::new(),
            forge_tabs: HashMap::new(),
            current_view: View::Welcome,
            status_message: None,
            copied_tree_item: None,
            config,
            workspace,
            workspace_restore_pending,
            aggregation_workspace_save_gen,
        }
    }

    /// Get the connection manager
    pub fn connection_manager(&self) -> Arc<ConnectionManager> {
        self.connection_manager.clone()
    }

    pub fn status_message(&self) -> Option<StatusMessage> {
        self.status_message.clone()
    }

    pub fn set_status_message(&mut self, message: Option<StatusMessage>) {
        self.status_message = message;
    }

    pub fn clear_status_message(&mut self) {
        self.status_message = None;
    }

    /// Save settings to disk
    pub fn save_settings(&self) {
        if let Err(e) = self.config.save_settings(&self.settings) {
            log::error!("Failed to save settings: {}", e);
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
