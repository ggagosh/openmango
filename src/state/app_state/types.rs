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
    Schema,
}

impl CollectionSubview {
    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Indexes,
            2 => Self::Stats,
            3 => Self::Aggregation,
            4 => Self::Schema,
            _ => Self::Documents,
        }
    }

    pub fn to_index(self) -> usize {
        match self {
            Self::Documents => 0,
            Self::Indexes => 1,
            Self::Stats => 2,
            Self::Aggregation => 3,
            Self::Schema => 4,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExplainScope {
    #[default]
    Find,
    Aggregation,
}

impl ExplainScope {
    pub fn label(self) -> &'static str {
        match self {
            ExplainScope::Find => "Find",
            ExplainScope::Aggregation => "Aggregation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExplainViewMode {
    #[default]
    Tree,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExplainPanelTab {
    #[default]
    Inspector,
    RejectedPlans,
    Diff,
}

impl ExplainPanelTab {
    pub fn label(self) -> &'static str {
        match self {
            ExplainPanelTab::Inspector => "Inspector",
            ExplainPanelTab::RejectedPlans => "Rejected Plans",
            ExplainPanelTab::Diff => "Diff",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExplainOpenMode {
    #[default]
    Closed,
    Modal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExplainSeverity {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

impl ExplainSeverity {
    pub fn label(self) -> &'static str {
        match self {
            ExplainSeverity::Low => "Low",
            ExplainSeverity::Medium => "Medium",
            ExplainSeverity::High => "High",
            ExplainSeverity::Critical => "Critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExplainCostBand {
    #[default]
    Low,
    Medium,
    High,
    VeryHigh,
}

impl ExplainCostBand {
    pub fn label(self) -> &'static str {
        match self {
            ExplainCostBand::Low => "Low",
            ExplainCostBand::Medium => "Medium",
            ExplainCostBand::High => "High",
            ExplainCostBand::VeryHigh => "Very High",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExplainNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub depth: usize,
    pub n_returned: Option<u64>,
    pub docs_examined: Option<u64>,
    pub keys_examined: Option<u64>,
    pub time_ms: Option<u64>,
    pub index_name: Option<String>,
    pub is_multi_key: Option<bool>,
    pub is_covered: Option<bool>,
    pub extra_metrics: Vec<(String, String)>,
    pub cost_band: ExplainCostBand,
    pub severity: ExplainSeverity,
}

#[derive(Debug, Clone, Default)]
pub struct ExplainSummary {
    pub n_returned: Option<u64>,
    pub docs_examined: Option<u64>,
    pub keys_examined: Option<u64>,
    pub execution_time_ms: Option<u64>,
    pub has_sort_stage: bool,
    pub has_collscan: bool,
    pub covered_indexes: Vec<String>,
    pub is_covered_query: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ExplainRejectedPlan {
    pub plan_id: String,
    pub root_stage: String,
    pub reason_hint: String,
    pub nodes: Vec<ExplainNode>,
    pub docs_examined: Option<u64>,
    pub keys_examined: Option<u64>,
    pub execution_time_ms: Option<u64>,
    pub index_names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExplainBottleneck {
    pub rank: usize,
    pub node_id: String,
    pub stage: String,
    pub impact_score: u64,
    pub docs_examined: Option<u64>,
    pub keys_examined: Option<u64>,
    pub execution_time_ms: Option<u64>,
    pub recommendation: String,
}

#[derive(Debug, Clone, Default)]
pub struct ExplainStageDelta {
    pub stage: String,
    pub node_id: String,
    pub docs_examined_delta: Option<i64>,
    pub keys_examined_delta: Option<i64>,
    pub execution_time_delta_ms: Option<i64>,
    pub impact_score_delta: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ExplainDiff {
    pub from_run_id: String,
    pub to_run_id: String,
    pub from_generated_at_unix_ms: u64,
    pub to_generated_at_unix_ms: u64,
    pub plan_shape_changed: bool,
    pub n_returned_delta: Option<i64>,
    pub docs_examined_delta: Option<i64>,
    pub keys_examined_delta: Option<i64>,
    pub execution_time_delta_ms: Option<i64>,
    pub stage_deltas: Vec<ExplainStageDelta>,
}

#[derive(Debug, Clone, Default)]
pub struct ExplainRun {
    pub id: String,
    pub generated_at_unix_ms: u64,
    pub signature: Option<u64>,
    pub scope: ExplainScope,
    pub raw_json: String,
    pub nodes: Vec<ExplainNode>,
    pub summary: ExplainSummary,
    pub rejected_plans: Vec<ExplainRejectedPlan>,
    pub bottlenecks: Vec<ExplainBottleneck>,
}

#[derive(Debug, Clone)]
pub struct ExplainState {
    pub loading: bool,
    pub error: Option<String>,
    pub open_mode: ExplainOpenMode,
    pub stale: bool,
    pub scope: ExplainScope,
    pub view_mode: ExplainViewMode,
    pub panel_tab: ExplainPanelTab,
    pub raw_json: Option<String>,
    pub nodes: Vec<ExplainNode>,
    pub selected_node_id: Option<String>,
    pub summary: Option<ExplainSummary>,
    pub rejected_plans: Vec<ExplainRejectedPlan>,
    pub bottlenecks: Vec<ExplainBottleneck>,
    pub history: Vec<ExplainRun>,
    pub current_run_id: Option<String>,
    pub compare_run_id: Option<String>,
    pub diff: Option<ExplainDiff>,
    pub signature: Option<u64>,
    pub generated_at_unix_ms: Option<u64>,
}

impl ExplainState {
    pub fn mark_stale(&mut self) {
        if self.raw_json.is_some() {
            self.stale = true;
        }
        self.signature = None;
    }

    pub fn push_run_with_limit(&mut self, run: ExplainRun, max_history: usize) {
        self.history.push(run);
        if max_history > 0 && self.history.len() > max_history {
            let overflow = self.history.len() - max_history;
            self.history.drain(0..overflow);
        }
        self.current_run_id = self.history.last().map(|item| item.id.clone());
        self.compare_run_id = if self.history.len() > 1 {
            self.history.get(self.history.len() - 2).map(|item| item.id.clone())
        } else {
            None
        };
        self.sync_from_selected_runs();
    }

    pub fn set_current_run(&mut self, run_id: Option<String>) {
        if let Some(run_id) = run_id {
            if self.history.iter().any(|run| run.id == run_id) {
                self.current_run_id = Some(run_id);
            }
        } else {
            self.current_run_id = None;
        }
        self.sync_from_selected_runs();
    }

    pub fn set_compare_run(&mut self, run_id: Option<String>) {
        if let Some(run_id) = run_id {
            if self.history.iter().any(|run| run.id == run_id) {
                self.compare_run_id = Some(run_id);
            }
        } else {
            self.compare_run_id = None;
        }
        self.sync_from_selected_runs();
    }

    pub fn cycle_current_run(&mut self, step: isize) {
        let len = self.history.len() as isize;
        if len == 0 {
            return;
        }
        let has_compare = self.compare_run_id.is_some();
        let current = self.current_run_index().unwrap_or((len - 1) as usize) as isize;
        let next = (current + step).clamp(0, len - 1) as usize;
        self.current_run_id = self.history.get(next).map(|run| run.id.clone());
        if has_compare {
            self.compare_with_previous_run();
        } else {
            self.sync_from_selected_runs();
        }
    }

    pub fn current_run_index(&self) -> Option<usize> {
        self.current_run_id
            .as_ref()
            .and_then(|run_id| self.history.iter().position(|run| run.id == *run_id))
    }

    pub fn compare_with_previous_run(&mut self) {
        let Some(current_idx) = self.current_run_index() else {
            return;
        };
        if current_idx == 0 {
            self.compare_run_id = None;
        } else {
            self.compare_run_id = self.history.get(current_idx - 1).map(|run| run.id.clone());
        }
        self.sync_from_selected_runs();
    }

    pub fn clear_compare_run(&mut self) {
        self.compare_run_id = None;
        if self.panel_tab == ExplainPanelTab::Diff {
            self.panel_tab = ExplainPanelTab::Inspector;
        }
        self.sync_from_selected_runs();
    }

    pub fn has_history_to_clear(&self) -> bool {
        self.history.len() > 1
    }

    pub fn clear_previous_runs_keep_current(&mut self) {
        if self.history.is_empty() {
            self.current_run_id = None;
            self.compare_run_id = None;
            self.diff = None;
            if self.panel_tab == ExplainPanelTab::Diff {
                self.panel_tab = ExplainPanelTab::Inspector;
            }
            return;
        }

        let run_to_keep = self
            .current_run_id
            .as_ref()
            .and_then(|run_id| self.history.iter().find(|run| run.id == *run_id))
            .cloned()
            .or_else(|| self.history.last().cloned());

        let Some(run_to_keep) = run_to_keep else {
            self.current_run_id = None;
            self.compare_run_id = None;
            self.diff = None;
            if self.panel_tab == ExplainPanelTab::Diff {
                self.panel_tab = ExplainPanelTab::Inspector;
            }
            return;
        };

        self.history.clear();
        self.current_run_id = Some(run_to_keep.id.clone());
        self.history.push(run_to_keep);
        self.compare_run_id = None;
        self.diff = None;
        if self.panel_tab == ExplainPanelTab::Diff {
            self.panel_tab = ExplainPanelTab::Inspector;
        }
        self.sync_from_selected_runs();
    }

    pub fn sync_from_selected_runs(&mut self) {
        if self.history.is_empty() {
            self.current_run_id = None;
            self.compare_run_id = None;
            self.diff = None;
            return;
        }

        let current_index = self
            .current_run_id
            .as_ref()
            .and_then(|run_id| self.history.iter().position(|run| run.id == *run_id))
            .unwrap_or_else(|| self.history.len() - 1);
        let current = &self.history[current_index];
        self.current_run_id = Some(current.id.clone());

        self.scope = current.scope;
        self.raw_json = Some(current.raw_json.clone());
        self.nodes = current.nodes.clone();
        self.summary = Some(current.summary.clone());
        self.rejected_plans = current.rejected_plans.clone();
        self.bottlenecks = current.bottlenecks.clone();
        self.signature = current.signature;
        self.generated_at_unix_ms = Some(current.generated_at_unix_ms);

        if !self
            .selected_node_id
            .as_ref()
            .is_some_and(|id| self.nodes.iter().any(|node| node.id == *id))
        {
            self.selected_node_id = self.nodes.first().map(|node| node.id.clone());
        }

        let compare_index = self.compare_run_id.as_ref().and_then(|run_id| {
            self.history
                .iter()
                .position(|run| run.id == *run_id)
                .filter(|index| *index != current_index)
        });

        if let Some(compare_index) = compare_index {
            let compare = &self.history[compare_index];
            self.compare_run_id = Some(compare.id.clone());
            self.diff = Some(build_explain_diff(compare, current));
        } else {
            self.compare_run_id = None;
            self.diff = None;
        }
    }
}

impl Default for ExplainState {
    fn default() -> Self {
        Self {
            loading: false,
            error: None,
            open_mode: ExplainOpenMode::Closed,
            stale: false,
            scope: ExplainScope::Find,
            view_mode: ExplainViewMode::Tree,
            panel_tab: ExplainPanelTab::Inspector,
            raw_json: None,
            nodes: Vec::new(),
            selected_node_id: None,
            summary: None,
            rejected_plans: Vec::new(),
            bottlenecks: Vec::new(),
            history: Vec::new(),
            current_run_id: None,
            compare_run_id: None,
            diff: None,
            signature: None,
            generated_at_unix_ms: None,
        }
    }
}

fn build_explain_diff(from: &ExplainRun, to: &ExplainRun) -> ExplainDiff {
    let mut stage_deltas = build_stage_deltas(&from.nodes, &to.nodes);
    stage_deltas
        .sort_by(|left, right| right.impact_score_delta.abs().cmp(&left.impact_score_delta.abs()));
    stage_deltas.truncate(16);

    ExplainDiff {
        from_run_id: from.id.clone(),
        to_run_id: to.id.clone(),
        from_generated_at_unix_ms: from.generated_at_unix_ms,
        to_generated_at_unix_ms: to.generated_at_unix_ms,
        plan_shape_changed: normalized_stage_chain(&from.nodes)
            != normalized_stage_chain(&to.nodes),
        n_returned_delta: delta_u64(to.summary.n_returned, from.summary.n_returned),
        docs_examined_delta: delta_u64(to.summary.docs_examined, from.summary.docs_examined),
        keys_examined_delta: delta_u64(to.summary.keys_examined, from.summary.keys_examined),
        execution_time_delta_ms: delta_u64(
            to.summary.execution_time_ms,
            from.summary.execution_time_ms,
        ),
        stage_deltas,
    }
}

fn build_stage_deltas(
    from_nodes: &[ExplainNode],
    to_nodes: &[ExplainNode],
) -> Vec<ExplainStageDelta> {
    let mut by_key: HashMap<String, (Option<&ExplainNode>, Option<&ExplainNode>)> = HashMap::new();
    for node in from_nodes {
        by_key.insert(stage_delta_key(node), (Some(node), None));
    }
    for node in to_nodes {
        let key = stage_delta_key(node);
        if let Some((_, to)) = by_key.get_mut(&key) {
            *to = Some(node);
        } else {
            by_key.insert(key, (None, Some(node)));
        }
    }

    by_key
        .into_values()
        .filter_map(|(from, to)| {
            let to = to?;
            let from_score = from.map(explain_node_impact_score).unwrap_or(0);
            let to_score = explain_node_impact_score(to);
            Some(ExplainStageDelta {
                stage: to.label.clone(),
                node_id: to.id.clone(),
                docs_examined_delta: delta_u64(
                    to.docs_examined,
                    from.and_then(|node| node.docs_examined),
                ),
                keys_examined_delta: delta_u64(
                    to.keys_examined,
                    from.and_then(|node| node.keys_examined),
                ),
                execution_time_delta_ms: delta_u64(to.time_ms, from.and_then(|node| node.time_ms)),
                impact_score_delta: to_score as i64 - from_score as i64,
            })
        })
        .collect()
}

fn stage_delta_key(node: &ExplainNode) -> String {
    format!("{}:{}", node.id, node.label.to_ascii_uppercase())
}

fn normalized_stage_chain(nodes: &[ExplainNode]) -> Vec<String> {
    nodes.iter().map(|node| node.label.trim_start_matches('$').to_ascii_uppercase()).collect()
}

fn explain_node_impact_score(node: &ExplainNode) -> u64 {
    node.docs_examined.unwrap_or(0)
        + node.keys_examined.unwrap_or(0)
        + node.time_ms.unwrap_or(0) * 120
        + node.n_returned.unwrap_or(0)
}

fn delta_u64(next: Option<u64>, prev: Option<u64>) -> Option<i64> {
    match (next, prev) {
        (Some(next), Some(prev)) => Some(next as i64 - prev as i64),
        (Some(next), None) => Some(next as i64),
        (None, Some(prev)) => Some(-(prev as i64)),
        (None, None) => None,
    }
}

// ============================================================================
// Schema Analysis Types
// ============================================================================

#[derive(Debug, Clone)]
pub struct SchemaField {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub types: Vec<SchemaFieldType>,
    pub presence: u64,
    pub null_count: u64,
    pub is_polymorphic: bool,
    pub children: Vec<SchemaField>,
}

#[derive(Debug, Clone)]
pub struct SchemaFieldType {
    pub bson_type: String,
    pub count: u64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardinalityBand {
    Low,
    Medium,
    High,
}

impl CardinalityBand {
    pub fn label(self) -> &'static str {
        match self {
            CardinalityBand::Low => "Low",
            CardinalityBand::Medium => "Medium",
            CardinalityBand::High => "High",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SchemaCardinality {
    pub distinct_estimate: u64,
    pub band: CardinalityBand,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SchemaAnalysis {
    pub fields: Vec<SchemaField>,
    pub total_fields: usize,
    pub total_types: usize,
    pub max_depth: usize,
    pub sampled: u64,
    pub total_documents: u64,
    pub polymorphic_count: usize,
    pub sparse_count: usize,
    pub complete_count: usize,
    /// (display_value, bson_type) tuples for BSON-aware coloring.
    pub sample_values: HashMap<String, Vec<(String, String)>>,
    pub cardinality: HashMap<String, SchemaCardinality>,
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
    pub explain: ExplainState,
    pub schema: Option<SchemaAnalysis>,
    pub schema_loading: bool,
    pub schema_error: Option<String>,
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
            explain: ExplainState::default(),
            schema: None,
            schema_loading: false,
            schema_error: None,
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
    pub schema_selected_field: Option<String>,
    pub schema_expanded_fields: HashSet<String>,
    pub schema_filter: String,
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
    pub explain: ExplainState,
    pub schema: Option<SchemaAnalysis>,
    pub schema_loading: bool,
    pub schema_error: Option<String>,
    pub schema_selected_field: Option<String>,
    pub schema_expanded_fields: HashSet<String>,
    pub schema_filter: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn explain_node(
        id: &str,
        label: &str,
        docs: Option<u64>,
        keys: Option<u64>,
        time_ms: Option<u64>,
    ) -> ExplainNode {
        ExplainNode {
            id: id.to_string(),
            parent_id: None,
            label: label.to_string(),
            depth: 0,
            n_returned: Some(10),
            docs_examined: docs,
            keys_examined: keys,
            time_ms,
            index_name: Some("status_1".to_string()),
            is_multi_key: Some(false),
            is_covered: Some(true),
            extra_metrics: Vec::new(),
            cost_band: ExplainCostBand::Medium,
            severity: ExplainSeverity::Medium,
        }
    }

    #[test]
    fn explain_state_tracks_history_and_builds_diff() {
        let mut state = ExplainState::default();
        let run1 = ExplainRun {
            id: "run-1".to_string(),
            generated_at_unix_ms: 1,
            signature: Some(1),
            scope: ExplainScope::Find,
            raw_json: "{}".to_string(),
            nodes: vec![explain_node("1", "IXSCAN", Some(120), Some(240), Some(5))],
            summary: ExplainSummary {
                n_returned: Some(10),
                docs_examined: Some(120),
                keys_examined: Some(240),
                execution_time_ms: Some(5),
                has_sort_stage: false,
                has_collscan: false,
                covered_indexes: vec!["status_1".to_string()],
                is_covered_query: true,
            },
            rejected_plans: Vec::new(),
            bottlenecks: Vec::new(),
        };
        state.push_run_with_limit(run1, 20);
        assert_eq!(state.history.len(), 1);
        assert!(state.diff.is_none());

        let run2 = ExplainRun {
            id: "run-2".to_string(),
            generated_at_unix_ms: 2,
            signature: Some(2),
            scope: ExplainScope::Find,
            raw_json: "{}".to_string(),
            nodes: vec![explain_node("1", "IXSCAN", Some(200), Some(500), Some(12))],
            summary: ExplainSummary {
                n_returned: Some(10),
                docs_examined: Some(200),
                keys_examined: Some(500),
                execution_time_ms: Some(12),
                has_sort_stage: false,
                has_collscan: false,
                covered_indexes: vec!["status_1".to_string()],
                is_covered_query: true,
            },
            rejected_plans: Vec::new(),
            bottlenecks: Vec::new(),
        };
        state.push_run_with_limit(run2, 20);
        assert_eq!(state.history.len(), 2);
        assert!(state.diff.is_some());
        let diff = state.diff.as_ref().expect("diff expected");
        assert_eq!(diff.execution_time_delta_ms, Some(7));
        assert_eq!(diff.docs_examined_delta, Some(80));

        state.cycle_current_run(-1);
        assert_eq!(state.current_run_id.as_deref(), Some("run-1"));
        assert!(state.has_history_to_clear());

        state.clear_previous_runs_keep_current();
        assert_eq!(state.history.len(), 1);
        assert_eq!(state.current_run_id.as_deref(), Some("run-1"));
        assert!(state.compare_run_id.is_none());
        assert!(state.diff.is_none());
        assert!(!state.has_history_to_clear());
    }
}
