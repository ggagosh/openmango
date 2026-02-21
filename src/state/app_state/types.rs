//! Type definitions for application state.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use crate::bson::DocumentKey;
use crate::models::connection::ActiveConnection;
use crate::state::app_state::PipelineState;
use futures::future::AbortHandle;
use mongodb::IndexModel;
use mongodb::bson::{Bson, Document};
use mongodb::results::{CollectionSpecification, CollectionType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current view in the application
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Welcome,
    Databases,
    Collections,
    Documents,
    Database,
    Transfer,
    Forge,
    Settings,
    Changelog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CollectionSubview {
    #[default]
    Documents,
    Indexes,
    Stats,
    Aggregation,
}

impl CollectionSubview {
    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Indexes,
            2 => Self::Stats,
            3 => Self::Aggregation,
            _ => Self::Documents,
        }
    }

    pub fn to_index(self) -> usize {
        match self {
            Self::Documents => 0,
            Self::Indexes => 1,
            Self::Stats => 2,
            Self::Aggregation => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub connection_id: Uuid,
    pub database: String,
    pub collection: String,
}

impl SessionKey {
    pub fn new(
        connection_id: Uuid,
        database: impl Into<String>,
        collection: impl Into<String>,
    ) -> Self {
        Self { connection_id, database: database.into(), collection: collection.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatabaseKey {
    pub connection_id: Uuid,
    pub database: String,
}

impl DatabaseKey {
    pub fn new(connection_id: Uuid, database: impl Into<String>) -> Self {
        Self { connection_id, database: database.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TabKey {
    Collection(SessionKey),
    Database(DatabaseKey),
    Transfer(TransferTabKey),
    Forge(ForgeTabKey),
    Settings,
    Changelog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferMode {
    #[default]
    Export,
    Import,
    Copy,
}

impl TransferMode {
    pub fn label(self) -> &'static str {
        match self {
            TransferMode::Export => "Export",
            TransferMode::Import => "Import",
            TransferMode::Copy => "Copy",
        }
    }

    pub fn index(self) -> usize {
        match self {
            TransferMode::Export => 0,
            TransferMode::Import => 1,
            TransferMode::Copy => 2,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Import,
            2 => Self::Copy,
            _ => Self::Export,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferScope {
    #[default]
    Collection,
    Database,
}

impl TransferScope {
    pub fn label(self) -> &'static str {
        match self {
            TransferScope::Collection => "Collection",
            TransferScope::Database => "Database",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferFormat {
    #[default]
    JsonLines,
    JsonArray,
    Csv,
    Bson,
}

impl TransferFormat {
    pub fn label(self) -> &'static str {
        match self {
            TransferFormat::JsonLines => "JSON Lines (.jsonl)",
            TransferFormat::JsonArray => "JSON array (.json)",
            TransferFormat::Csv => "CSV (.csv)",
            TransferFormat::Bson => "BSON (mongodump)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            TransferFormat::JsonLines => "jsonl",
            TransferFormat::JsonArray => "json",
            TransferFormat::Csv => "csv",
            TransferFormat::Bson => "bson",
        }
    }

    #[allow(dead_code)]
    pub fn available_for_collection(self) -> bool {
        !matches!(self, TransferFormat::Bson)
    }
}

// InsertMode, ExtendedJsonMode, BsonOutputFormat: canonical definitions in crate::connection::types
pub use crate::connection::{BsonOutputFormat, ExtendedJsonMode, InsertMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompressionMode {
    #[default]
    None,
    Gzip,
}

impl CompressionMode {
    pub fn label(self) -> &'static str {
        match self {
            CompressionMode::None => "None",
            CompressionMode::Gzip => "Gzip",
        }
    }
}

// Encoding: canonical definition in crate::connection::types
pub use crate::connection::Encoding;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferTabKey {
    pub id: Uuid,
    pub connection_id: Option<Uuid>,
}

// ============================================================================
// Forge Tab Types - Query Shell
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ForgeTabKey {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub database: String,
}

/// Default content for a Forge query shell tab.
pub const DEFAULT_FORGE_CONTENT: &str = "";

/// State for a Forge query shell tab
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ForgeTabState {
    pub content: String,
    pub is_running: bool,
    pub error: Option<String>,
    pub pending_cursor: Option<usize>,
}

impl Default for ForgeTabState {
    fn default() -> Self {
        Self {
            content: DEFAULT_FORGE_CONTENT.to_string(),
            is_running: false,
            error: None,
            pending_cursor: None,
        }
    }
}

// ============================================================================
// Transfer Tab State - Split into focused sub-structs
// ============================================================================

/// Core transfer configuration (mode, scope, source/destination)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransferConfig {
    pub mode: TransferMode,
    pub scope: TransferScope,
    pub source_connection_id: Option<Uuid>,
    pub source_database: String,
    pub source_collection: String,
    pub destination_connection_id: Option<Uuid>,
    pub destination_database: String,
    pub destination_collection: String,
    pub format: TransferFormat,
    pub file_path: String,
}

/// Mode-specific transfer options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferOptions {
    // Compression (all modes)
    pub compression: CompressionMode,

    // Database scope options (Export/Import/Copy)
    pub include_collections: Vec<String>,
    pub exclude_collections: Vec<String>,
    pub include_indexes: bool,

    // Import options
    pub insert_mode: InsertMode,
    pub drop_before_import: bool,
    pub clear_before_import: bool,
    pub stop_on_error: bool,
    pub batch_size: u32,
    pub detect_format: bool,
    pub encoding: Encoding,
    pub restore_indexes: bool,

    // JSON options
    pub json_mode: ExtendedJsonMode,
    pub pretty_print: bool,

    // BSON options
    pub bson_output: BsonOutputFormat,

    // Copy options
    pub copy_indexes: bool,
    pub copy_options: bool,
    pub overwrite_target: bool,
    pub ordered: bool,

    // Export query options (Collection scope only)
    pub export_filter: String,
    pub export_projection: String,
    pub export_sort: String,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            compression: CompressionMode::None,
            include_collections: Vec::new(),
            exclude_collections: Vec::new(),
            include_indexes: true,

            insert_mode: InsertMode::Insert,
            drop_before_import: false,
            clear_before_import: false,
            stop_on_error: true,
            batch_size: 1000,
            detect_format: true,
            encoding: Encoding::Utf8,
            restore_indexes: true,

            json_mode: ExtendedJsonMode::Relaxed,
            pretty_print: false,

            bson_output: BsonOutputFormat::Folder,

            copy_indexes: true,
            copy_options: true,
            overwrite_target: false,
            ordered: true,

            export_filter: String::new(),
            export_projection: String::new(),
            export_sort: String::new(),
        }
    }
}

/// Runtime transfer execution state (not serialized)
#[derive(Default)]
pub struct TransferRuntime {
    pub is_running: bool,
    pub progress_count: u64,
    pub error_message: Option<String>,
    pub transfer_generation: Arc<AtomicU64>,
    pub abort_handle: Arc<Mutex<Option<AbortHandle>>>,
    pub cancellation_token: Option<crate::connection::types::CancellationToken>,
    pub database_progress: Option<DatabaseTransferProgress>,
}

impl Clone for TransferRuntime {
    fn clone(&self) -> Self {
        Self {
            is_running: self.is_running,
            progress_count: self.progress_count,
            error_message: self.error_message.clone(),
            transfer_generation: Arc::new(AtomicU64::new(
                self.transfer_generation.load(std::sync::atomic::Ordering::SeqCst),
            )),
            abort_handle: Arc::new(Mutex::new(None)),
            cancellation_token: None,
            database_progress: self.database_progress.clone(),
        }
    }
}

impl std::fmt::Debug for TransferRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferRuntime")
            .field("is_running", &self.is_running)
            .field("progress_count", &self.progress_count)
            .field("error_message", &self.error_message)
            .field("database_progress", &self.database_progress)
            .finish()
    }
}

/// Preview state for transfer operations
#[derive(Debug, Clone, Default)]
pub struct TransferPreview {
    pub docs: Vec<String>,
    pub loading: bool,
    pub warnings: Vec<String>,
}

/// Complete transfer tab state - composed of focused sub-structs
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransferTabState {
    /// Core configuration (mode, scope, source/destination)
    pub config: TransferConfig,

    /// Mode-specific options
    pub options: TransferOptions,

    /// Runtime execution state (not serialized)
    #[serde(skip)]
    pub runtime: TransferRuntime,

    /// Preview state (not serialized)
    #[serde(skip)]
    pub preview: TransferPreview,
}

impl TransferTabState {
    pub fn tab_label(&self) -> String {
        let base = self.config.mode.label();
        let source = if !self.config.source_database.is_empty() {
            if self.config.scope == TransferScope::Collection
                && !self.config.source_collection.is_empty()
            {
                format!("{}/{}", self.config.source_database, self.config.source_collection)
            } else {
                self.config.source_database.clone()
            }
        } else {
            "New".to_string()
        };
        format!("{base}: {source}")
    }

    /// Create a new TransferTabState with defaults from settings.
    pub fn from_settings(settings: &crate::state::settings::AppSettings) -> Self {
        Self {
            config: TransferConfig {
                format: settings.transfer.default_export_format,
                ..Default::default()
            },
            options: TransferOptions {
                batch_size: settings.transfer.default_batch_size,
                insert_mode: settings.transfer.default_import_mode,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

// ============================================================================
// Sub-state structs for better organization
// ============================================================================

/// Connection-related state
#[derive(Default)]
pub struct ConnectionState {
    /// Currently active MongoDB connection
    pub active: HashMap<Uuid, ActiveConnection>,
    /// Currently selected connection ID
    pub selected_connection: Option<Uuid>,
    /// Currently selected database name
    pub selected_database: Option<String>,
    /// Currently selected collection name
    pub selected_collection: Option<String>,
    /// Remembered selection per connection (db, collection)
    pub selection_cache: HashMap<Uuid, (Option<String>, Option<String>)>,
}

/// Tab management state
#[derive(Default)]
pub struct TabState {
    /// Open collection tabs
    pub open: Vec<TabKey>,
    /// Index of currently active tab
    pub active: ActiveTab,
    /// Preview tab (shown before committing to full tab)
    pub preview: Option<SessionKey>,
    /// Tabs with unsaved changes
    pub dirty: HashSet<SessionKey>,
    /// Current drag-over target for open tab reordering: (tab_index, insert_after)
    pub drag_over: Option<(usize, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    None,
    Index(usize),
    Preview,
}

#[derive(Debug, Clone)]
pub struct SessionDocument {
    pub key: DocumentKey,
    pub doc: Document,
}

/// Session data loaded from MongoDB and pagination state.
pub struct SessionData {
    pub items: Vec<SessionDocument>,
    pub index_by_key: HashMap<DocumentKey, usize>,
    pub page: u64,
    pub per_page: i64,
    pub total: u64,
    pub is_loading: bool,
    pub loaded: bool,
    pub request_id: u64,
    pub filter_raw: String,
    pub filter: Option<Document>,
    pub sort_raw: String,
    pub sort: Option<Document>,
    pub projection_raw: String,
    pub projection: Option<Document>,
    pub stats: Option<CollectionStats>,
    pub stats_loading: bool,
    pub stats_error: Option<String>,
    pub indexes: Option<Vec<IndexModel>>,
    pub indexes_loading: bool,
    pub indexes_error: Option<String>,
    pub aggregation: PipelineState,
}

impl Default for SessionData {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            index_by_key: HashMap::new(),
            page: 0,
            per_page: 50,
            total: 0,
            is_loading: false,
            loaded: false,
            request_id: 0,
            filter_raw: String::new(),
            filter: None,
            sort_raw: String::new(),
            sort: None,
            projection_raw: String::new(),
            projection: None,
            stats: None,
            stats_loading: false,
            stats_error: None,
            indexes: None,
            indexes_loading: false,
            indexes_error: None,
            aggregation: PipelineState::default(),
        }
    }
}

/// Per-collection view state (selection, expansion, edits).
#[derive(Default)]
pub struct SessionViewState {
    pub selected_doc: Option<DocumentKey>,
    pub selected_node_id: Option<String>,
    pub selected_docs: HashSet<DocumentKey>,
    pub expanded_nodes: HashSet<String>,
    pub drafts: HashMap<DocumentKey, Document>,
    pub dirty: HashSet<DocumentKey>,
    pub subview: CollectionSubview,
    pub stats_open: bool,
    pub query_options_open: bool,
}

/// Per-collection session state (one per tab).
#[derive(Default)]
pub struct SessionState {
    pub data: SessionData,
    pub view: SessionViewState,
    /// Monotonically increasing counter bumped on document load, save, or delete.
    /// Used by the tree cache to detect stale entries.
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub items: Vec<SessionDocument>,
    pub total: u64,
    pub page: u64,
    pub per_page: i64,
    pub is_loading: bool,
    pub selected_doc: Option<DocumentKey>,
    pub selected_docs: HashSet<DocumentKey>,
    pub selected_count: usize,
    pub any_selected_dirty: bool,
    pub filter_raw: String,
    pub sort_raw: String,
    pub projection_raw: String,
    pub query_options_open: bool,
    pub subview: CollectionSubview,
    pub stats: Option<CollectionStats>,
    pub stats_loading: bool,
    pub stats_error: Option<String>,
    pub indexes: Option<Vec<IndexModel>>,
    pub indexes_loading: bool,
    pub indexes_error: Option<String>,
    pub aggregation: PipelineState,
}

#[derive(Debug, Clone)]
pub struct DatabaseStats {
    pub collections: u64,
    pub objects: u64,
    pub avg_obj_size: u64,
    pub data_size: u64,
    pub storage_size: u64,
    pub indexes: u64,
    pub index_size: u64,
}

impl DatabaseStats {
    pub fn from_document(doc: &Document) -> Self {
        Self {
            collections: read_u64(doc, "collections"),
            objects: read_u64(doc, "objects"),
            avg_obj_size: read_u64(doc, "avgObjSize"),
            data_size: read_u64(doc, "dataSize"),
            storage_size: read_u64(doc, "storageSize"),
            indexes: read_u64(doc, "indexes"),
            index_size: read_u64(doc, "indexSize"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CollectionOverview {
    pub name: String,
    pub collection_type: String,
    pub capped: bool,
    pub read_only: bool,
}

impl CollectionOverview {
    pub fn from_spec(spec: CollectionSpecification) -> Self {
        Self {
            name: spec.name,
            collection_type: collection_type_label(&spec.collection_type).to_string(),
            capped: spec.options.capped.unwrap_or(false),
            read_only: spec.info.read_only,
        }
    }
}

#[derive(Default)]
pub struct DatabaseSessionData {
    pub stats: Option<DatabaseStats>,
    pub stats_loading: bool,
    pub stats_error: Option<String>,
    pub collections: Vec<CollectionOverview>,
    pub collections_loading: bool,
    pub collections_error: Option<String>,
}

#[derive(Default)]
pub struct DatabaseSessionState {
    pub data: DatabaseSessionData,
}

#[derive(Debug, Clone)]
pub struct CollectionStats {
    pub document_count: u64,
    pub avg_obj_size: u64,
    pub data_size: u64,
    pub storage_size: u64,
    pub total_index_size: u64,
    pub index_count: u64,
    pub capped: bool,
    pub max_size: Option<u64>,
}

impl CollectionStats {
    pub fn from_document(doc: &Document) -> Self {
        let document_count = read_u64(doc, "count");
        let avg_obj_size = read_u64(doc, "avgObjSize");
        let data_size = read_u64(doc, "size");
        let storage_size = read_u64(doc, "storageSize");
        let total_index_size = read_u64(doc, "totalIndexSize");
        let index_count = read_u64(doc, "nindexes");
        let capped = doc.get_bool("capped").unwrap_or(false);
        let max_size = read_u64_opt(doc, "maxSize");

        Self {
            document_count,
            avg_obj_size,
            data_size,
            storage_size,
            total_index_size,
            index_count,
            capped,
            max_size,
        }
    }
}

fn collection_type_label(collection_type: &CollectionType) -> &'static str {
    match collection_type {
        CollectionType::Collection => "collection",
        CollectionType::View => "view",
        CollectionType::Timeseries => "timeseries",
        _ => "collection",
    }
}

fn read_u64(doc: &Document, key: &str) -> u64 {
    read_u64_opt(doc, key).unwrap_or(0)
}

fn read_u64_opt(doc: &Document, key: &str) -> Option<u64> {
    doc.get(key).and_then(bson_to_u64)
}

fn bson_to_u64(value: &Bson) -> Option<u64> {
    match value {
        Bson::Int32(v) => Some(*v as u64),
        Bson::Int64(v) => Some(*v as u64),
        Bson::Double(v) => {
            if *v >= 0.0 {
                Some(*v as u64)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Item copied from the sidebar tree for paste operation (internal clipboard)
#[derive(Clone, Debug)]
pub enum CopiedTreeItem {
    Database { connection_id: Uuid, database: String },
    Collection { connection_id: Uuid, database: String, collection: String },
}

// ============================================================================
// Progress tracking for database-scope transfer operations
// ============================================================================

/// Status of a single collection transfer
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CollectionTransferStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}

/// Progress tracking for a single collection
#[derive(Clone, Debug, Default)]
pub struct CollectionProgress {
    pub name: String,
    pub status: CollectionTransferStatus,
    pub documents_processed: u64,
    pub documents_total: Option<u64>, // None = unknown/estimating
}

impl CollectionProgress {
    pub fn percentage(&self) -> Option<f32> {
        self.documents_total.map(|total| {
            if total == 0 {
                100.0
            } else {
                (self.documents_processed as f32 / total as f32) * 100.0
            }
        })
    }
}

/// Progress tracking for database-scope operations
#[derive(Clone, Debug, Default)]
pub struct DatabaseTransferProgress {
    pub collections: Vec<CollectionProgress>,
    pub panel_expanded: bool,
}

#[allow(dead_code)]
impl DatabaseTransferProgress {
    pub fn total_documents_processed(&self) -> u64 {
        self.collections.iter().map(|c| c.documents_processed).sum()
    }

    pub fn total_documents_total(&self) -> Option<u64> {
        let totals: Vec<u64> = self.collections.iter().filter_map(|c| c.documents_total).collect();
        if totals.len() == self.collections.len() && !totals.is_empty() {
            Some(totals.iter().sum())
        } else {
            None
        }
    }

    pub fn overall_percentage(&self) -> Option<f32> {
        let total = self.total_documents_total()?;
        if total == 0 {
            return Some(100.0);
        }
        Some((self.total_documents_processed() as f32 / total as f32) * 100.0)
    }

    pub fn completed_count(&self) -> usize {
        self.collections
            .iter()
            .filter(|c| matches!(c.status, CollectionTransferStatus::Completed))
            .count()
    }
}
