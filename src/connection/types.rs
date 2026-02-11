use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

/// Error type for aggregation pipeline operations
#[derive(Debug)]
pub enum AggregatePipelineError {
    Mongo(crate::error::Error),
    Aborted,
}

impl From<crate::error::Error> for AggregatePipelineError {
    fn from(value: crate::error::Error) -> Self {
        Self::Mongo(value)
    }
}

/// JSON format for import/export operations
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub enum JsonTransferFormat {
    JsonArray,
    #[default]
    JsonLines,
}

/// Extended JSON output mode
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtendedJsonMode {
    #[default]
    Relaxed,
    Canonical,
}

impl ExtendedJsonMode {
    pub fn label(self) -> &'static str {
        match self {
            ExtendedJsonMode::Relaxed => "Relaxed",
            ExtendedJsonMode::Canonical => "Canonical",
        }
    }
}

/// Insert mode for imports
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsertMode {
    #[default]
    Insert,
    Upsert,
    Replace,
}

impl InsertMode {
    pub fn label(self) -> &'static str {
        match self {
            InsertMode::Insert => "Insert",
            InsertMode::Upsert => "Upsert",
            InsertMode::Replace => "Replace",
        }
    }

    #[allow(dead_code)]
    pub fn description(self) -> &'static str {
        match self {
            InsertMode::Insert => "Insert new documents (fail on duplicates)",
            InsertMode::Upsert => "Update existing documents or insert new ones",
            InsertMode::Replace => "Replace existing documents or insert new ones",
        }
    }
}

/// Text encoding for file imports
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    #[default]
    Utf8,
    Latin1,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Latin1 => "Latin-1",
        }
    }
}

/// BSON output format for mongodump
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BsonOutputFormat {
    #[default]
    Folder,
    Archive,
}

impl BsonOutputFormat {
    pub fn label(self) -> &'static str {
        match self {
            BsonOutputFormat::Folder => "Folder",
            BsonOutputFormat::Archive => "Archive (.archive)",
        }
    }
}

/// Progress callback type for reporting operation progress.
pub type ProgressCallback = Arc<dyn Fn(u64) + Send + Sync>;

/// Cancellation token for aborting long-running operations.
#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self { cancelled: Arc::new(AtomicBool::new(false)) }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Options for JSON export
#[derive(Clone, Default)]
pub struct JsonExportOptions {
    pub format: JsonTransferFormat,
    pub json_mode: ExtendedJsonMode,
    pub pretty_print: bool,
    pub gzip: bool,
    pub cancellation: Option<CancellationToken>,
}

impl std::fmt::Debug for JsonExportOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonExportOptions")
            .field("format", &self.format)
            .field("json_mode", &self.json_mode)
            .field("pretty_print", &self.pretty_print)
            .field("gzip", &self.gzip)
            .field("cancellation", &self.cancellation.is_some())
            .finish()
    }
}

/// Options for JSON import
#[derive(Clone, Default)]
pub struct JsonImportOptions {
    pub format: JsonTransferFormat,
    pub insert_mode: InsertMode,
    pub stop_on_error: bool,
    pub batch_size: usize,
    pub encoding: Encoding,
    pub progress: Option<ProgressCallback>,
    pub cancellation: Option<CancellationToken>,
}

impl std::fmt::Debug for JsonImportOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonImportOptions")
            .field("format", &self.format)
            .field("insert_mode", &self.insert_mode)
            .field("stop_on_error", &self.stop_on_error)
            .field("batch_size", &self.batch_size)
            .field("encoding", &self.encoding)
            .field("progress", &self.progress.is_some())
            .field("cancellation", &self.cancellation.is_some())
            .finish()
    }
}

/// Options for CSV import
#[derive(Clone, Default)]
pub struct CsvImportOptions {
    pub insert_mode: InsertMode,
    pub stop_on_error: bool,
    pub batch_size: usize,
    pub encoding: Encoding,
    pub progress: Option<ProgressCallback>,
    pub cancellation: Option<CancellationToken>,
}

impl std::fmt::Debug for CsvImportOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsvImportOptions")
            .field("insert_mode", &self.insert_mode)
            .field("stop_on_error", &self.stop_on_error)
            .field("batch_size", &self.batch_size)
            .field("encoding", &self.encoding)
            .field("progress", &self.progress.is_some())
            .field("cancellation", &self.cancellation.is_some())
            .finish()
    }
}

/// Query options for collection-level exports
#[derive(Clone, Debug, Default)]
pub struct ExportQueryOptions {
    pub filter: Option<mongodb::bson::Document>,
    pub projection: Option<mongodb::bson::Document>,
    pub sort: Option<mongodb::bson::Document>,
}

/// Options for copy operations
#[derive(Clone, Default)]
pub struct CopyOptions {
    pub batch_size: usize,
    pub copy_indexes: bool,
    pub insert_mode: InsertMode,
    pub ordered: bool,
    pub progress: Option<ProgressCallback>,
    pub cancellation: Option<CancellationToken>,
}

impl CopyOptions {
    pub fn new(batch_size: usize, copy_indexes: bool) -> Self {
        Self { batch_size, copy_indexes, ..Default::default() }
    }
}

impl std::fmt::Debug for CopyOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopyOptions")
            .field("batch_size", &self.batch_size)
            .field("copy_indexes", &self.copy_indexes)
            .field("insert_mode", &self.insert_mode)
            .field("ordered", &self.ordered)
            .field("progress", &self.progress.is_some())
            .field("cancellation", &self.cancellation.is_some())
            .finish()
    }
}

/// Options for finding documents with pagination
pub struct FindDocumentsOptions {
    pub filter: Option<mongodb::bson::Document>,
    pub sort: Option<mongodb::bson::Document>,
    pub projection: Option<mongodb::bson::Document>,
    pub skip: u64,
    pub limit: i64,
}

/// Progress information from mongodump/mongorestore tools.
#[derive(Clone, Debug)]
pub enum BsonToolProgress {
    /// Collection export/import started
    Started { collection: String },
    /// Progress update with current/total counts
    Progress {
        collection: String,
        current: u64,
        total: u64,
        #[allow(dead_code)]
        percent: f32,
    },
    /// Collection export/import completed
    Completed { collection: String, documents: u64 },
}
