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
pub mod updater;
mod workspace;

pub(crate) use aggregation::{
    PipelineAnalysis, PipelineStage, PipelineState, StageDocCounts, StageStatsMode,
    default_stage_body,
};
pub(crate) use connection::write_conn_secrets;
pub(crate) use database_sessions::DatabaseSessionStore;
pub(crate) use sessions::SessionStore;
pub use types::{
    ActiveTab, BsonOutputFormat, CardinalityBand, CollectionOverview, CollectionProgress,
    CollectionStats, CollectionSubview, CollectionTransferStatus, CompressionMode, CopiedTreeItem,
    DatabaseKey, DatabaseSessionData, DatabaseSessionState, DatabaseStats,
    DatabaseTransferProgress, Encoding, ExplainBottleneck, ExplainCostBand, ExplainDiff,
    ExplainNode, ExplainOpenMode, ExplainPanelTab, ExplainRejectedPlan, ExplainRun, ExplainScope,
    ExplainSeverity, ExplainStageDelta, ExplainState, ExplainSummary, ExplainViewMode,
    ExtendedJsonMode, ForgeTabKey, ForgeTabState, InsertMode, SchemaAnalysis, SchemaCardinality,
    SchemaField, SchemaFieldType, SessionData, SessionDocument, SessionKey, SessionState,
    SessionViewState, TabKey, TransferFormat, TransferMode, TransferScope, TransferTabKey,
    TransferTabState, View,
};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, atomic::AtomicU64};
use std::time::Instant;

use gpui::{Context, EventEmitter};

use crate::ai::AiChatState;
use crate::connection::ConnectionManager;
use crate::models::connection::SavedConnection;
use crate::state::StatusMessage;
use crate::state::editor_sessions::EditorSessionStore;
use crate::state::events::AppEvent;
use crate::state::settings::{AppSettings, migrate_islands_tab_style_to_islands};
use crate::state::{ConfigManager, WorkspaceState};

use updater::UpdateStatus;

use types::*;

/// Cached schema fields with TTL.
pub(crate) struct ForgeSchemaCache {
    pub fields: Vec<String>,
    pub cached_at: Instant,
}

const FORGE_SCHEMA_TTL_SECS: u64 = 300; // 5 minutes

/// Cached schema analysis for a sibling collection.
pub(crate) struct CollectionMetaCache {
    pub schema: SchemaAnalysis,
    pub fetched_at: Instant,
}

const COLLECTION_META_TTL_SECS: u64 = 600; // 10 minutes

/// Global application state
pub struct AppState {
    // Persisted state
    pub connections: Vec<SavedConnection>,
    pub settings: AppSettings,

    /// Vibrancy state from startup (window creation). Runtime changes require restart.
    pub startup_vibrancy: bool,

    // Connection manager (injected for testability)
    connection_manager: Arc<ConnectionManager>,

    // Organized sub-states
    conn: ConnectionState,
    tabs: TabState,
    sessions: SessionStore,
    db_sessions: DatabaseSessionStore,
    transfer_tabs: HashMap<uuid::Uuid, TransferTabState>,
    forge_tabs: HashMap<uuid::Uuid, ForgeTabState>,
    forge_schema: HashMap<SessionKey, ForgeSchemaCache>,
    forge_schema_inflight: HashSet<SessionKey>,
    collection_meta: HashMap<SessionKey, CollectionMetaCache>,
    collection_meta_inflight: HashSet<SessionKey>,
    pub ai_chat: AiChatState,

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
    pub(crate) changelog_pending: bool,
    aggregation_workspace_save_gen: Arc<AtomicU64>,

    // Auto-update
    pub update_status: UpdateStatus,

    // Shared detached JSON editor sessions
    editor_sessions: EditorSessionStore,
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
        let mut settings = config.load_settings().unwrap_or_else(|e| {
            log::warn!("Failed to load settings: {}", e);
            AppSettings::default()
        });
        if migrate_islands_tab_style_to_islands(&mut settings)
            && let Err(e) = config.save_settings(&settings)
        {
            log::warn!("Failed to persist Islands tab style migration: {e}");
        }
        let workspace = config.load_workspace().unwrap_or_else(|e| {
            log::warn!("Failed to load workspace: {}", e);
            WorkspaceState::default()
        });
        let workspace_restore_pending = workspace.last_connection_id.is_some();
        let aggregation_workspace_save_gen = Arc::new(AtomicU64::new(0));

        let startup_vibrancy = crate::theme::effective_vibrancy(
            settings.appearance.theme,
            settings.appearance.vibrancy,
        );

        Self {
            connections,
            settings,
            startup_vibrancy,
            connection_manager,
            conn: ConnectionState::default(),
            tabs: TabState::default(),
            sessions: SessionStore::new(),
            db_sessions: DatabaseSessionStore::new(),
            transfer_tabs: HashMap::new(),
            forge_tabs: HashMap::new(),
            forge_schema: HashMap::new(),
            forge_schema_inflight: std::collections::HashSet::new(),
            collection_meta: HashMap::new(),
            collection_meta_inflight: HashSet::new(),
            ai_chat: AiChatState::default(),
            current_view: View::Welcome,
            status_message: None,
            copied_tree_item: None,
            config,
            workspace,
            workspace_restore_pending,
            changelog_pending: false,
            aggregation_workspace_save_gen,
            update_status: UpdateStatus::Idle,
            editor_sessions: EditorSessionStore::default(),
        }
    }

    /// Get the connection manager
    pub fn connection_manager(&self) -> Arc<ConnectionManager> {
        self.connection_manager.clone()
    }

    pub fn status_message(&self) -> Option<StatusMessage> {
        self.status_message.clone()
    }

    pub fn editor_sessions(&self) -> EditorSessionStore {
        self.editor_sessions.clone()
    }

    pub fn set_status_message(&mut self, message: Option<StatusMessage>) {
        self.status_message = message;
    }

    pub fn clear_status_message(&mut self) {
        self.status_message = None;
    }

    pub fn ai_assistant_available(&self) -> bool {
        self.settings.ai.assistant_available()
    }

    /// Save settings to disk
    pub fn save_settings(&self) {
        if let Err(e) = self.config.save_settings(&self.settings) {
            log::error!("Failed to save settings: {}", e);
        }
    }

    pub(crate) fn collection_meta(&self, key: &SessionKey) -> Option<&CollectionMetaCache> {
        self.collection_meta.get(key)
    }

    pub(crate) fn collection_meta_stale(&self, key: &SessionKey) -> bool {
        match self.collection_meta.get(key) {
            Some(cache) => cache.fetched_at.elapsed().as_secs() > COLLECTION_META_TTL_SECS,
            None => true,
        }
    }

    pub(crate) fn set_collection_meta(&mut self, key: SessionKey, schema: SchemaAnalysis) {
        self.collection_meta
            .insert(key, CollectionMetaCache { schema, fetched_at: Instant::now() });
    }

    pub(crate) fn mark_collection_meta_inflight(&mut self, key: &SessionKey) -> bool {
        self.collection_meta_inflight.insert(key.clone())
    }

    pub(crate) fn is_collection_meta_inflight(&self, key: &SessionKey) -> bool {
        self.collection_meta_inflight.contains(key)
    }

    pub(crate) fn clear_collection_meta_inflight(&mut self, key: &SessionKey) {
        self.collection_meta_inflight.remove(key);
    }

    pub(crate) fn evict_collection_meta_for_connection(&mut self, connection_id: uuid::Uuid) {
        self.collection_meta.retain(|k, _| k.connection_id != connection_id);
        self.collection_meta_inflight.retain(|k| k.connection_id != connection_id);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// Enable reactive UI updates via event subscription
impl EventEmitter<AppEvent> for AppState {}
