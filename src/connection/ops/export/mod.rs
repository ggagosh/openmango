//! Collection and database export operations (JSON, CSV).
//!
//! This module provides export functionality for MongoDB collections and databases:
//! - JSON/JSONL export with various options (pretty print, gzip, extended JSON modes)
//! - CSV export with automatic column detection
//! - Database-wide export (all collections)
//! - Progress callbacks for large exports

mod csv;
mod json;

use mongodb::Client;
use mongodb::bson::Bson;

use crate::connection::ConnectionManager;
use crate::connection::types::ExtendedJsonMode;
use crate::error::Result;

/// Generate a preview of documents for export.
pub fn generate_export_preview(
    manager: &ConnectionManager,
    client: &Client,
    database: &str,
    collection: &str,
    json_mode: ExtendedJsonMode,
    pretty_print: bool,
    limit: usize,
) -> Result<Vec<String>> {
    let docs = manager.sample_documents(client, database, collection, limit as i64)?;

    let previews: Vec<String> = docs
        .into_iter()
        .map(|doc| {
            let json_value = match json_mode {
                ExtendedJsonMode::Relaxed => Bson::Document(doc).into_relaxed_extjson(),
                ExtendedJsonMode::Canonical => Bson::Document(doc).into_canonical_extjson(),
            };

            if pretty_print {
                serde_json::to_string_pretty(&json_value).unwrap_or_default()
            } else {
                serde_json::to_string(&json_value).unwrap_or_default()
            }
        })
        .collect();

    Ok(previews)
}
