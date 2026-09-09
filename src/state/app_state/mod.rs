//! Global application state.

mod aggregation;
mod connection;
mod database_sessions;
mod forge;
mod keybindings;
mod query_library;
mod selection;
mod sessions;
mod status;
mod tabs;
mod transfer;
mod types;
mod unsaved;
pub mod updater;
mod workspace;

pub(crate) use aggregation::{
    PipelineAnalysis, PipelineStage, PipelineState, StageDocCounts, StageStatsMode,
    default_stage_body,
};
pub(crate) use connection::{
    ConnectionSecrets, LEGACY_CONNECTION_SECRET_KEYS, connection_secret_bundle_key,
};
pub(crate) use database_sessions::DatabaseSessionStore;
pub use keybindings::KeybindingCapture;
pub(crate) use sessions::SessionStore;
pub use types::{
    ActiveTab, BsonOutputFormat, CardinalityBand, CollectionOverview, CollectionProgress,
    CollectionStats, CollectionSubview, CollectionTransferStatus, CompressionMode,
    ConnectionManagerRequest, CopiedTreeItem, DatabaseKey, DatabaseSessionData,
    DatabaseSessionState, DatabaseStats, DatabaseTransferProgress, DocumentViewMode, Encoding,
    ExplainBottleneck, ExplainCostBand, ExplainDiff, ExplainNode, ExplainOpenMode, ExplainPanelTab,
    ExplainRejectedPlan, ExplainRun, ExplainScope, ExplainSeverity, ExplainStageDelta,
    ExplainState, ExplainSummary, ExplainViewMode, ExtendedJsonMode, ForgeTabKey, ForgeTabState,
    InsertMode, SchemaAnalysis, SchemaCardinality, SchemaField, SchemaFieldType, SessionData,
    SessionDocument, SessionKey, SessionState, SessionViewState, TabKey, TargetWriteMode,
    TransferFormat, TransferMode, TransferScope, TransferTabKey, TransferTabState, View,
};
pub use unsaved::{UnsavedChange, UnsavedInventory, UnsavedScope};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, atomic::AtomicU64};
use std::time::Instant;

use gpui_kit::{Context, EventEmitter};
use uuid::Uuid;

use crate::ai::AiChatState;
use crate::connection::ConnectionManager;
use crate::models::connection::SavedConnection;
use crate::state::StatusMessage;
use crate::state::editor_sessions::EditorSessionStore;
use crate::state::events::AppEvent;
use crate::state::settings::{AppSettings, migrate_islands_tab_style_to_islands};
use crate::state::{ConfigManager, QueryLibrary, WorkspaceState};

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
    query_library: QueryLibrary,
    query_library_persistence_blocked: bool,

    /// Window/keymap state from startup. Runtime changes require restart.
    pub startup_vibrancy: bool,
    pub startup_keybindings: crate::state::KeybindingSettings,

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
    connection_manager_request: ConnectionManagerRequest,
    status_message: Option<StatusMessage>,
    keybinding_capture: Option<KeybindingCapture>,
    unsaved_guard_active: bool,
    invalid_inline_edits: HashSet<SessionKey>,
    production_write_authorizations: HashMap<Uuid, usize>,

    /// Copied tree item for paste operation (internal clipboard)
    pub copied_tree_item: Option<CopiedTreeItem>,

    // Passive all-client History recorder
    history_service: Option<Arc<crate::history::HistoryService>>,
    history_eligibility: HashMap<Uuid, crate::history::EligibilityReport>,
    history_usage: HashMap<Uuid, crate::history::Usage>,
    history_inspecting: HashSet<Uuid>,

    // Agent action persistence
    action_broker: Arc<crate::actions::ActionBroker>,
    sync_executor: Arc<crate::sync::SyncExecutor>,

    // Config manager for persistence
    pub(crate) config: ConfigManager,
    pub(crate) connections_persistence_blocked: bool,
    connection_secret_sync_pending: bool,
    connections_waiting_for_secret_sync: HashSet<Uuid>,

    // Workspace persistence
    pub workspace: WorkspaceState,
    pub(crate) workspace_restore_pending: bool,
    pub(crate) changelog_pending: bool,
    aggregation_workspace_save_gen: Arc<AtomicU64>,

    // Auto-update
    pub update_status: UpdateStatus,

    // Lightweight file export progress (Save As)
    export_progress: Option<crate::state::commands::ExportProgress>,

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
        Self::with_config(connection_manager, ConfigManager::default())
    }

    pub(crate) fn with_config(
        connection_manager: Arc<ConnectionManager>,
        config: ConfigManager,
    ) -> Self {
        // A malformed connection file must never be replaced with an empty list.
        let (connections, connection_load_error) = match config.load_connections() {
            Ok(connections) => (connections, None),
            Err(error) => {
                let message = format!(
                    "Connections could not be loaded. The original file was preserved; fix or restore it before editing connections: {error}"
                );
                log::error!("{message}");
                (Vec::new(), Some(message))
            }
        };
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
        let (query_library, query_library_load_error) = match config.load_query_library() {
            Ok(library) => (library, None),
            Err(error) => {
                let message = format!(
                    "Query Library could not be loaded. The original file was preserved: {error}"
                );
                log::error!("{message}");
                (QueryLibrary::default(), Some(message))
            }
        };
        let query_library_persistence_blocked = query_library_load_error.is_some();
        let workspace_restore_pending = workspace.last_connection_id.is_some();
        let aggregation_workspace_save_gen = Arc::new(AtomicU64::new(0));

        let startup_vibrancy = crate::theme::effective_vibrancy(
            settings.appearance.theme,
            settings.appearance.vibrancy,
        );
        let startup_keybindings = settings.keybindings.clone();
        let action_store = Arc::new(crate::actions::ActionStore::new(config.agent_data_dir()));
        if let Err(error) = action_store.reconcile_interrupted() {
            log::error!("Could not reconcile interrupted agent operations: {error}");
        }
        let action_broker = Arc::new(crate::actions::ActionBroker::new(action_store.clone()));
        let sync_executor =
            Arc::new(crate::sync::SyncExecutor::new(connection_manager.clone(), action_store));

        Self {
            connections,
            settings,
            query_library,
            query_library_persistence_blocked,
            startup_vibrancy,
            startup_keybindings,
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
            connection_manager_request: ConnectionManagerRequest::default(),
            status_message: connection_load_error
                .clone()
                .or(query_library_load_error)
                .map(StatusMessage::error),
            keybinding_capture: None,
            unsaved_guard_active: false,
            invalid_inline_edits: HashSet::new(),
            production_write_authorizations: HashMap::new(),
            copied_tree_item: None,
            history_service: None,
            history_eligibility: HashMap::new(),
            history_usage: HashMap::new(),
            history_inspecting: HashSet::new(),
            action_broker,
            sync_executor,
            config,
            connections_persistence_blocked: connection_load_error.is_some(),
            connection_secret_sync_pending: false,
            connections_waiting_for_secret_sync: HashSet::new(),
            workspace,
            workspace_restore_pending,
            changelog_pending: false,
            aggregation_workspace_save_gen,
            update_status: UpdateStatus::Idle,
            export_progress: None,
            editor_sessions: EditorSessionStore::default(),
        }
    }

    /// Get the connection manager
    pub fn connection_manager(&self) -> Arc<ConnectionManager> {
        self.connection_manager.clone()
    }

    pub fn history_service(&self) -> Option<Arc<crate::history::HistoryService>> {
        self.history_service.clone()
    }

    pub(crate) fn set_history_service(&mut self, service: Arc<crate::history::HistoryService>) {
        self.history_service = Some(service);
    }

    pub fn history_eligibility(
        &self,
        connection_id: Uuid,
    ) -> Option<&crate::history::EligibilityReport> {
        self.history_eligibility.get(&connection_id)
    }

    pub fn history_usage(&self, connection_id: Uuid) -> Option<crate::history::Usage> {
        self.history_usage.get(&connection_id).copied()
    }

    pub fn collection_history_available(
        &self,
        connection_id: Uuid,
        database: &str,
        collection: &str,
    ) -> bool {
        self.connection_history_enabled(connection_id)
            && self.history_service.is_some()
            && self
                .history_eligibility(connection_id)
                .is_some_and(|report| report.collection_available(database, collection))
    }

    pub(crate) fn refresh_history_usage(&mut self, connection_id: Uuid) {
        if let Some(service) = &self.history_service
            && let Ok(usage) = service.usage(Some(connection_id))
        {
            self.history_usage.insert(connection_id, usage);
        }
    }

    pub fn history_inspecting(&self, connection_id: Uuid) -> bool {
        self.history_inspecting.contains(&connection_id)
    }

    pub(crate) fn begin_history_inspection(&mut self, connection_id: Uuid) -> bool {
        self.history_inspecting.insert(connection_id)
    }

    pub(crate) fn finish_history_inspection(
        &mut self,
        connection_id: Uuid,
        report: crate::history::EligibilityReport,
        usage: Option<crate::history::Usage>,
    ) {
        self.history_inspecting.remove(&connection_id);
        self.history_eligibility.insert(connection_id, report);
        if let Some(usage) = usage {
            self.history_usage.insert(connection_id, usage);
        }
    }

    pub fn action_broker(&self) -> Arc<crate::actions::ActionBroker> {
        self.action_broker.clone()
    }

    pub fn sync_executor(&self) -> Arc<crate::sync::SyncExecutor> {
        self.sync_executor.clone()
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

    pub fn export_progress(&self) -> Option<&crate::state::commands::ExportProgress> {
        self.export_progress.as_ref()
    }

    pub fn export_progress_mut(&mut self) -> Option<&mut crate::state::commands::ExportProgress> {
        self.export_progress.as_mut()
    }

    pub fn set_export_progress(
        &mut self,
        progress: Option<crate::state::commands::ExportProgress>,
    ) {
        self.export_progress = progress;
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
