//! CSV export operations for collections and databases.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use mongodb::Client;
use mongodb::bson::Document;

use crate::connection::ConnectionManager;
use crate::connection::ops::export::AtomicExportFile;
use crate::connection::types::{CancellationToken, ExportQueryOptions};
use crate::error::Result;

fn csv_row(
    document: &Document,
    columns: &[String],
    seen_columns: &HashSet<String>,
) -> Result<Vec<String>> {
    let flat = crate::connection::csv_utils::flatten_document(document);
    if let Some(field) = flat.keys().find(|field| !seen_columns.contains(*field)) {
        return Err(crate::error::Error::Parse(format!(
            "Export source changed while discovering columns; new field '{field}' was not skipped"
        )));
    }
    Ok(columns.iter().map(|column| flat.get(column).cloned().unwrap_or_default()).collect())
}

impl ConnectionManager {
    /// Export a collection to CSV with query options (runs in Tokio runtime).
    /// Uses single-pass buffering: buffers first N docs to detect columns, then continues streaming.
    #[allow(clippy::too_many_arguments)]
    pub fn export_collection_csv_with_query(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        gzip: bool,
        query: ExportQueryOptions,
        cancellation: Option<CancellationToken>,
    ) -> Result<u64> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);

            // Build find options with query options (single query for all documents)
            let filter = query.filter.unwrap_or_default();
            let mut find_options = mongodb::options::FindOptions::default();
            find_options.projection = query.projection;
            find_options.sort = query.sort;

            // First pass discovers every column without buffering every document.
            let mut discovery_cursor =
                coll.find(filter.clone()).with_options(find_options.clone()).await?;
            let mut seen_columns = HashSet::new();
            let mut columns = Vec::new();
            while let Some(doc) = discovery_cursor.try_next().await? {
                if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }
                crate::connection::csv_utils::collect_document_columns(
                    &doc,
                    &mut seen_columns,
                    &mut columns,
                );
            }

            let output = AtomicExportFile::new(&path)?;
            let file = output.reopen()?;
            if columns.is_empty() {
                drop(file);
                if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }
                output.commit()?;
                return Ok(0);
            }

            // Second pass streams complete rows using the full column set.
            let mut cursor = coll.find(filter).with_options(find_options).await?;

            // Write CSV with optional gzip compression
            let mut csv_writer = if gzip {
                csv::Writer::from_writer(
                    Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                )
            } else {
                csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
            };

            // Write header
            csv_writer.write_record(&columns)?;

            let mut count = 0u64;
            while let Some(doc) = cursor.try_next().await? {
                // Check cancellation
                if cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }

                let row = csv_row(&doc, &columns, &seen_columns)?;
                csv_writer.write_record(&row)?;
                count += 1;
            }

            csv_writer.flush()?;
            drop(csv_writer);
            if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                return Err(crate::error::Error::Parse("Export cancelled".to_string()));
            }
            output.commit()?;
            Ok(count)
        })
    }

    /// Export a collection to CSV with query options and progress callback (runs in Tokio runtime).
    /// Uses single-pass buffering: buffers first N docs to detect columns, then continues streaming.
    /// The callback is invoked every ~1000 documents with the current count.
    #[allow(clippy::too_many_arguments)]
    pub fn export_collection_csv_with_query_and_progress<F>(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        gzip: bool,
        query: ExportQueryOptions,
        cancellation: Option<CancellationToken>,
        on_progress: F,
    ) -> Result<u64>
    where
        F: Fn(u64) + Send + 'static,
    {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);

            // Build find options with query options (single query for all documents)
            let filter = query.filter.unwrap_or_default();
            let mut find_options = mongodb::options::FindOptions::default();
            find_options.projection = query.projection;
            find_options.sort = query.sort;

            let mut discovery_cursor =
                coll.find(filter.clone()).with_options(find_options.clone()).await?;
            let mut seen_columns = HashSet::new();
            let mut columns = Vec::new();
            while let Some(doc) = discovery_cursor.try_next().await? {
                if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }
                crate::connection::csv_utils::collect_document_columns(
                    &doc,
                    &mut seen_columns,
                    &mut columns,
                );
            }

            let output = AtomicExportFile::new(&path)?;
            let file = output.reopen()?;
            if columns.is_empty() {
                drop(file);
                if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }
                output.commit()?;
                on_progress(0);
                return Ok(0);
            }
            let mut cursor = coll.find(filter).with_options(find_options).await?;

            // Write CSV with optional gzip compression
            let mut csv_writer = if gzip {
                csv::Writer::from_writer(
                    Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                )
            } else {
                csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
            };

            // Write header
            csv_writer.write_record(&columns)?;

            let mut count = 0u64;
            const PROGRESS_INTERVAL: u64 = 1000;

            while let Some(doc) = cursor.try_next().await? {
                if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                    return Err(crate::error::Error::Parse("Export cancelled".to_string()));
                }
                let row = csv_row(&doc, &columns, &seen_columns)?;
                csv_writer.write_record(&row)?;
                count += 1;

                // Report progress every N documents
                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            csv_writer.flush()?;
            drop(csv_writer);
            if cancellation.as_ref().is_some_and(|token| token.is_cancelled()) {
                return Err(crate::error::Error::Parse("Export cancelled".to_string()));
            }
            output.commit()?;
            // Final progress report
            on_progress(count);
            Ok(count)
        })
    }
}
