//! CSV export operations for collections and databases.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use mongodb::Client;
use mongodb::bson::{Document, doc};

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
    /// Export a collection to CSV (runs in Tokio runtime).
    #[allow(dead_code)]
    pub fn export_collection_csv(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        gzip: bool,
    ) -> Result<u64> {
        self.export_collection_csv_with_query(
            client,
            database,
            collection,
            path,
            gzip,
            ExportQueryOptions::default(),
            None,
        )
    }

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
    #[allow(dead_code)]
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

    /// Export a collection to CSV with progress callback (runs in Tokio runtime).
    /// Uses single-pass buffering: buffers first N docs to detect columns, then continues streaming.
    /// The callback is invoked every ~1000 documents with the current count.
    #[allow(dead_code)]
    pub fn export_collection_csv_with_progress<F>(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        gzip: bool,
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

            let mut discovery_cursor = coll.find(doc! {}).await?;
            let mut seen_columns = HashSet::new();
            let mut columns = Vec::new();
            while let Some(doc) = discovery_cursor.try_next().await? {
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
                output.commit()?;
                on_progress(0);
                return Ok(0);
            }
            let mut cursor = coll.find(doc! {}).await?;

            // Write CSV
            let mut csv_writer = if gzip {
                csv::Writer::from_writer(
                    Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                )
            } else {
                csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
            };

            csv_writer.write_record(&columns)?;

            let mut count = 0u64;
            const PROGRESS_INTERVAL: u64 = 1000;

            while let Some(doc) = cursor.try_next().await? {
                let row = csv_row(&doc, &columns, &seen_columns)?;
                csv_writer.write_record(&row)?;
                count += 1;

                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            csv_writer.flush()?;
            drop(csv_writer);
            output.commit()?;
            on_progress(count);
            Ok(count)
        })
    }

    /// Export all collections in a database to CSV files (runs in Tokio runtime).
    /// Creates one file per collection in the specified directory.
    /// Uses single-pass buffering: buffers first N docs to detect columns, then continues streaming.
    #[allow(dead_code)]
    pub fn export_database_csv(
        &self,
        client: &Client,
        database: &str,
        directory: &Path,
        gzip: bool,
        exclude_collections: &[String],
    ) -> Result<u64> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let directory = directory.to_path_buf();
        let exclude_collections = exclude_collections.to_vec();

        self.runtime.block_on(async move {
            let db = client.database(&database);
            let collections = db.list_collection_names().await?;
            let final_directory = directory;
            let parent = final_directory
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let staging =
                tempfile::Builder::new().prefix(".openmango-database-csv-").tempdir_in(parent)?;
            let directory = staging.path().join("export");
            std::fs::create_dir(&directory)?;

            let mut total_count = 0u64;

            for coll_name in collections {
                // Skip system collections
                if coll_name.starts_with("system.") {
                    continue;
                }

                // Skip excluded collections
                if exclude_collections.contains(&coll_name) {
                    continue;
                }

                // Create file path for this collection
                let file_name = format!("{}_{}.csv", database, coll_name);
                let file_path = directory.join(&file_name);

                let coll = client.database(&database).collection::<Document>(&coll_name);
                let mut discovery_cursor = coll.find(doc! {}).await?;
                let mut seen_columns = HashSet::new();
                let mut columns = Vec::new();
                while let Some(doc) = discovery_cursor.try_next().await? {
                    crate::connection::csv_utils::collect_document_columns(
                        &doc,
                        &mut seen_columns,
                        &mut columns,
                    );
                }

                let output = AtomicExportFile::new(&file_path)?;
                let file = output.reopen()?;
                if columns.is_empty() {
                    drop(file);
                    output.commit()?;
                    continue;
                }
                let mut cursor = coll.find(doc! {}).await?;

                // Write CSV
                let mut csv_writer = if gzip {
                    csv::Writer::from_writer(
                        Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                    )
                } else {
                    csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
                };

                csv_writer.write_record(&columns)?;

                let mut count = 0u64;
                while let Some(doc) = cursor.try_next().await? {
                    let row = csv_row(&doc, &columns, &seen_columns)?;
                    csv_writer.write_record(&row)?;
                    count += 1;
                }

                csv_writer.flush()?;
                drop(csv_writer);
                output.commit()?;
                total_count += count;
            }

            crate::connection::ops::export::promote_export_directory(&directory, &final_directory)?;
            Ok(total_count)
        })
    }
}
