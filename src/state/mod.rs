// Application state management
#![allow(unused_imports)]

pub mod app_state;
pub mod commands;
pub mod compare;
pub mod compare_sync;
pub mod config;
pub mod editor_sessions;
pub mod events;
mod query_library;
pub mod relations;
pub mod settings;
pub mod status;
pub mod transfer_rules;
pub mod workspace;

pub use crate::ai::{AiProvider, AiSettings};
pub use app_state::{
    ActiveTab, AppState, BsonOutputFormat, CardinalityBand, CollectionKey, CollectionOverview,
    CollectionStats, CollectionSubview, CompressionMode, CopiedTreeItem, DatabaseKey,
    DatabaseSessionData, DatabaseSessionState, DatabaseStats, DocumentViewMode, Encoding,
    ErrorAction, ErrorEntry, ExplainBottleneck, ExplainCostBand, ExplainDiff, ExplainNode,
    ExplainOpenMode, ExplainPanelTab, ExplainRejectedPlan, ExplainRun, ExplainScope,
    ExplainSeverity, ExplainStageDelta, ExplainState, ExplainSummary, ExplainViewMode,
    ExtendedJsonMode, ForgeTabKey, ForgeTabState, InsertMode, KeybindingCapture, NavHistory,
    ReferencesTabKey, SchemaAnalysis, SchemaCardinality, SchemaField, SchemaFieldType, SessionData,
    SessionDocument, SessionKey, SessionState, SessionViewState, TabKey, TargetWriteMode,
    TaskNotice, TransferFormat, TransferMode, TransferScope, TransferTabKey, TransferTabState,
    UnsavedChange, UnsavedInventory, UnsavedScope, View,
};
pub use commands::AppCommands;
pub(crate) use commands::view_pipeline;
pub use commands::{ViewSave, ViewSource};
pub use config::ConfigManager;
pub use editor_sessions::{
    EditorSession, EditorSessionId, EditorSessionStore, EditorSessionTarget,
};
pub use events::AppEvent;
pub use query_library::{
    DocumentQuery, QueryContent, QueryDefinition, QueryHistoryEntry, QueryImportReport, QueryKind,
    QueryLibrary, QueryLibraryPersistenceError, SavedQuery, SavedQueryInput, SavedQueryScope,
};
pub use relations::resolve::{Plan as ResolutionPlan, Reference};
pub use relations::{
    Cardinality, Evidence, FieldRef, JoinStep, Origin, Relation, RelationGraph, RelationKind,
    RelationModel, Status as RelationStatus, Upsert,
};
pub use settings::{
    AppSettings, AppTheme, AppearanceSettings, DATABASE_SCOPE_FILENAME_TEMPLATE,
    DEFAULT_FILENAME_TEMPLATE, FILENAME_PLACEHOLDERS, IslandsAppearanceSettings,
    IslandsCornerSoftness, IslandsTabStyle, KeybindingSettings, McpClientGrant, McpClientKind,
    McpSettings, TransferSettings, expand_filename_template,
};
pub use status::{StatusLevel, StatusMessage};
pub use transfer_rules::{
    TransferValidation, available_transfer_formats, coerce_transfer_format,
    parse_export_query_document, resolved_export_destination, transfer_write_connection,
    validate_transfer,
};
pub use workspace::{WindowMode, WindowState, WorkspaceState, WorkspaceTab, WorkspaceTabKind};
