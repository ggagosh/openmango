//! MongoDB connection management and operations.
//!
//! This module provides:
//! - `ConnectionManager`: Core connection management and basic operations
//! - `ops`: Database operations (documents, export, import, indexes, stats, aggregation, copy, bson_tools)
//! - `tools`: MongoDB tools (mongodump/mongorestore) path detection
//! - `types`: Shared types for all operations
//! - `csv_utils`: CSV flattening/unflattening utilities

pub mod csv_utils;
pub mod manager;
pub mod ops;
pub mod tools;
pub mod tunnel;
pub mod types;

// Re-export commonly used items at the crate level
pub use manager::ConnectionManager;
pub use ops::export::generate_export_preview;
pub use tools::tools_available;
pub use types::{
    AggregatePipelineError, BsonOutputFormat, BsonToolProgress, CopyOptions, CsvImportOptions,
    Encoding, ExportQueryOptions, ExtendedJsonMode, FindDocumentsOptions, InsertMode,
    JsonExportOptions, JsonImportOptions, JsonTransferFormat, ProgressCallback,
};
