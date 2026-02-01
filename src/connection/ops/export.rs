//! Collection and database export operations (JSON, CSV).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use mongodb::Client;
use mongodb::bson::{Bson, Document, doc};

use crate::connection::ConnectionManager;
use crate::connection::types::{
    ExportQueryOptions, ExtendedJsonMode, JsonExportOptions, JsonTransferFormat,
};
use crate::error::Result;

impl ConnectionManager {
    /// Export a collection to JSON/JSONL (runs in Tokio runtime).
    #[allow(dead_code)]
    pub fn export_collection_json(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        format: JsonTransferFormat,
        path: &Path,
    ) -> Result<u64> {
        self.export_collection_json_with_options(
            client,
            database,
            collection,
            path,
            JsonExportOptions { format, ..Default::default() },
        )
    }

    /// Export a collection to JSON/JSONL with full options (runs in Tokio runtime).
    pub fn export_collection_json_with_options(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        options: JsonExportOptions,
    ) -> Result<u64> {
        self.export_collection_json_with_query(
            client,
            database,
            collection,
            path,
            options,
            ExportQueryOptions::default(),
        )
    }

    /// Export a collection to JSON/JSONL with full options and query options (runs in Tokio runtime).
    pub fn export_collection_json_with_query(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        options: JsonExportOptions,
        query: ExportQueryOptions,
    ) -> Result<u64> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);

            // Build find options with query options
            let filter = query.filter.unwrap_or_default();
            let mut find_options = mongodb::options::FindOptions::default();
            find_options.projection = query.projection;
            find_options.sort = query.sort;

            let mut cursor = coll.find(filter).with_options(find_options).await?;
            let file = File::create(&path)?;

            // Wrap writer with gzip encoder if compression is enabled
            let mut writer: Box<dyn Write> = if options.gzip {
                Box::new(BufWriter::new(GzEncoder::new(file, Compression::default())))
            } else {
                Box::new(BufWriter::new(file))
            };

            let mut count = 0u64;

            if matches!(options.format, JsonTransferFormat::JsonArray) {
                writer.write_all(b"[")?;
                if options.pretty_print {
                    writer.write_all(b"\n")?;
                }
            }

            let mut first = true;
            while let Some(doc) = cursor.try_next().await? {
                let json_value = match options.json_mode {
                    ExtendedJsonMode::Relaxed => Bson::Document(doc).into_relaxed_extjson(),
                    ExtendedJsonMode::Canonical => Bson::Document(doc).into_canonical_extjson(),
                };

                let json = if options.pretty_print {
                    serde_json::to_string_pretty(&json_value)?
                } else {
                    serde_json::to_string(&json_value)?
                };

                match options.format {
                    JsonTransferFormat::JsonLines => {
                        writer.write_all(json.as_bytes())?;
                        writer.write_all(b"\n")?;
                    }
                    JsonTransferFormat::JsonArray => {
                        if !first {
                            writer.write_all(b",")?;
                            if options.pretty_print {
                                writer.write_all(b"\n")?;
                            }
                        }
                        writer.write_all(json.as_bytes())?;
                        first = false;
                    }
                }
                count += 1;
            }

            if matches!(options.format, JsonTransferFormat::JsonArray) {
                if count > 0 && options.pretty_print {
                    writer.write_all(b"\n")?;
                }
                writer.write_all(b"]")?;
            }

            writer.flush()?;
            Ok(count)
        })
    }

    /// Export a collection to JSON/JSONL with query options and progress callback (runs in Tokio runtime).
    /// The callback is invoked every ~1000 documents with the current count.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub fn export_collection_json_with_query_and_progress<F>(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        options: JsonExportOptions,
        query: ExportQueryOptions,
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

            // Build find options with query options
            let filter = query.filter.unwrap_or_default();
            let mut find_options = mongodb::options::FindOptions::default();
            find_options.projection = query.projection;
            find_options.sort = query.sort;

            let mut cursor = coll.find(filter).with_options(find_options).await?;
            let file = File::create(&path)?;

            let mut writer: Box<dyn Write> = if options.gzip {
                Box::new(BufWriter::new(GzEncoder::new(file, Compression::default())))
            } else {
                Box::new(BufWriter::new(file))
            };

            let mut count = 0u64;
            const PROGRESS_INTERVAL: u64 = 1000;

            if matches!(options.format, JsonTransferFormat::JsonArray) {
                writer.write_all(b"[")?;
                if options.pretty_print {
                    writer.write_all(b"\n")?;
                }
            }

            let mut first = true;
            while let Some(doc) = cursor.try_next().await? {
                let json_value = match options.json_mode {
                    ExtendedJsonMode::Relaxed => Bson::Document(doc).into_relaxed_extjson(),
                    ExtendedJsonMode::Canonical => Bson::Document(doc).into_canonical_extjson(),
                };

                let json = if options.pretty_print {
                    serde_json::to_string_pretty(&json_value)?
                } else {
                    serde_json::to_string(&json_value)?
                };

                match options.format {
                    JsonTransferFormat::JsonLines => {
                        writer.write_all(json.as_bytes())?;
                        writer.write_all(b"\n")?;
                    }
                    JsonTransferFormat::JsonArray => {
                        if !first {
                            writer.write_all(b",")?;
                            if options.pretty_print {
                                writer.write_all(b"\n")?;
                            }
                        }
                        writer.write_all(json.as_bytes())?;
                        first = false;
                    }
                }
                count += 1;

                // Report progress every N documents
                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            if matches!(options.format, JsonTransferFormat::JsonArray) {
                if count > 0 && options.pretty_print {
                    writer.write_all(b"\n")?;
                }
                writer.write_all(b"]")?;
            }

            writer.flush()?;
            // Final progress report
            on_progress(count);
            Ok(count)
        })
    }

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
        )
    }

    /// Export a collection to CSV with query options (runs in Tokio runtime).
    /// Uses single-pass buffering: buffers first N docs to detect columns, then continues streaming.
    pub fn export_collection_csv_with_query(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        gzip: bool,
        query: ExportQueryOptions,
    ) -> Result<u64> {
        use crate::connection::csv_utils::{collect_columns, flatten_document};
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

            // Start single cursor for all documents
            let mut cursor = coll.find(filter).with_options(find_options).await?;

            // Buffer first N documents to detect columns
            const SAMPLE_SIZE: usize = 1000;
            let mut buffered_docs: Vec<Document> = Vec::with_capacity(SAMPLE_SIZE);

            while buffered_docs.len() < SAMPLE_SIZE {
                match cursor.try_next().await? {
                    Some(doc) => buffered_docs.push(doc),
                    None => break, // No more documents
                }
            }

            // Collect columns from buffered documents
            let columns = collect_columns(&buffered_docs);

            if columns.is_empty() {
                return Ok(0);
            }

            // Write CSV with optional gzip compression
            let file = File::create(&path)?;
            let mut csv_writer = if gzip {
                csv::Writer::from_writer(
                    Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                )
            } else {
                csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
            };

            // Write header
            csv_writer.write_record(&columns)?;

            // Write buffered documents first
            let mut count = 0u64;
            for doc in buffered_docs {
                let flat = flatten_document(&doc);
                let row: Vec<String> =
                    columns.iter().map(|col| flat.get(col).cloned().unwrap_or_default()).collect();
                csv_writer.write_record(&row)?;
                count += 1;
            }

            // Continue streaming remaining documents from same cursor
            while let Some(doc) = cursor.try_next().await? {
                let flat = flatten_document(&doc);
                let row: Vec<String> =
                    columns.iter().map(|col| flat.get(col).cloned().unwrap_or_default()).collect();
                csv_writer.write_record(&row)?;
                count += 1;
            }

            csv_writer.flush()?;
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
        on_progress: F,
    ) -> Result<u64>
    where
        F: Fn(u64) + Send + 'static,
    {
        use crate::connection::csv_utils::{collect_columns, flatten_document};
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

            // Start single cursor for all documents
            let mut cursor = coll.find(filter).with_options(find_options).await?;

            // Buffer first N documents to detect columns
            const SAMPLE_SIZE: usize = 1000;
            let mut buffered_docs: Vec<Document> = Vec::with_capacity(SAMPLE_SIZE);

            while buffered_docs.len() < SAMPLE_SIZE {
                match cursor.try_next().await? {
                    Some(doc) => buffered_docs.push(doc),
                    None => break, // No more documents
                }
            }

            // Collect columns from buffered documents
            let columns = collect_columns(&buffered_docs);

            if columns.is_empty() {
                on_progress(0);
                return Ok(0);
            }

            // Write CSV with optional gzip compression
            let file = File::create(&path)?;
            let mut csv_writer = if gzip {
                csv::Writer::from_writer(
                    Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                )
            } else {
                csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
            };

            // Write header
            csv_writer.write_record(&columns)?;

            // Write buffered documents first
            let mut count = 0u64;
            const PROGRESS_INTERVAL: u64 = 1000;

            for doc in buffered_docs {
                let flat = flatten_document(&doc);
                let row: Vec<String> =
                    columns.iter().map(|col| flat.get(col).cloned().unwrap_or_default()).collect();
                csv_writer.write_record(&row)?;
                count += 1;

                // Report progress every N documents
                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            // Continue streaming remaining documents from same cursor
            while let Some(doc) = cursor.try_next().await? {
                let flat = flatten_document(&doc);
                let row: Vec<String> =
                    columns.iter().map(|col| flat.get(col).cloned().unwrap_or_default()).collect();
                csv_writer.write_record(&row)?;
                count += 1;

                // Report progress every N documents
                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            csv_writer.flush()?;
            // Final progress report
            on_progress(count);
            Ok(count)
        })
    }

    /// Export a collection to JSON/JSONL with progress callback (runs in Tokio runtime).
    /// The callback is invoked every ~1000 documents with the current count.
    #[allow(dead_code)]
    pub fn export_collection_json_with_progress<F>(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        options: JsonExportOptions,
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

            let mut cursor = coll.find(doc! {}).await?;
            let file = File::create(&path)?;

            let mut writer: Box<dyn Write> = if options.gzip {
                Box::new(BufWriter::new(GzEncoder::new(file, Compression::default())))
            } else {
                Box::new(BufWriter::new(file))
            };

            let mut count = 0u64;
            const PROGRESS_INTERVAL: u64 = 1000;

            if matches!(options.format, JsonTransferFormat::JsonArray) {
                writer.write_all(b"[")?;
                if options.pretty_print {
                    writer.write_all(b"\n")?;
                }
            }

            let mut first = true;
            while let Some(doc) = cursor.try_next().await? {
                let json_value = match options.json_mode {
                    ExtendedJsonMode::Relaxed => Bson::Document(doc).into_relaxed_extjson(),
                    ExtendedJsonMode::Canonical => Bson::Document(doc).into_canonical_extjson(),
                };

                let json = if options.pretty_print {
                    serde_json::to_string_pretty(&json_value)?
                } else {
                    serde_json::to_string(&json_value)?
                };

                match options.format {
                    JsonTransferFormat::JsonLines => {
                        writer.write_all(json.as_bytes())?;
                        writer.write_all(b"\n")?;
                    }
                    JsonTransferFormat::JsonArray => {
                        if !first {
                            writer.write_all(b",")?;
                            if options.pretty_print {
                                writer.write_all(b"\n")?;
                            }
                        }
                        writer.write_all(json.as_bytes())?;
                        first = false;
                    }
                }
                count += 1;

                // Report progress every N documents
                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            if matches!(options.format, JsonTransferFormat::JsonArray) {
                if count > 0 && options.pretty_print {
                    writer.write_all(b"\n")?;
                }
                writer.write_all(b"]")?;
            }

            writer.flush()?;
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
        use crate::connection::csv_utils::{collect_columns, flatten_document};
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);

            // Start single cursor for all documents
            let mut cursor = coll.find(doc! {}).await?;

            // Buffer first N documents to detect columns
            const SAMPLE_SIZE: usize = 1000;
            let mut buffered_docs: Vec<Document> = Vec::with_capacity(SAMPLE_SIZE);

            while buffered_docs.len() < SAMPLE_SIZE {
                match cursor.try_next().await? {
                    Some(doc) => buffered_docs.push(doc),
                    None => break, // No more documents
                }
            }

            // Collect columns from buffered documents
            let columns = collect_columns(&buffered_docs);

            if columns.is_empty() {
                on_progress(0);
                return Ok(0);
            }

            // Write CSV
            let file = File::create(&path)?;
            let mut csv_writer = if gzip {
                csv::Writer::from_writer(
                    Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                )
            } else {
                csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
            };

            csv_writer.write_record(&columns)?;

            // Write buffered documents first
            let mut count = 0u64;
            const PROGRESS_INTERVAL: u64 = 1000;

            for doc in buffered_docs {
                let flat = flatten_document(&doc);
                let row: Vec<String> =
                    columns.iter().map(|col| flat.get(col).cloned().unwrap_or_default()).collect();
                csv_writer.write_record(&row)?;
                count += 1;

                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            // Continue streaming remaining documents from same cursor
            while let Some(doc) = cursor.try_next().await? {
                let flat = flatten_document(&doc);
                let row: Vec<String> =
                    columns.iter().map(|col| flat.get(col).cloned().unwrap_or_default()).collect();
                csv_writer.write_record(&row)?;
                count += 1;

                if count.is_multiple_of(PROGRESS_INTERVAL) {
                    on_progress(count);
                }
            }

            csv_writer.flush()?;
            on_progress(count);
            Ok(count)
        })
    }

    /// Export all collections in a database to JSON files (runs in Tokio runtime).
    /// Creates one file per collection in the specified directory.
    #[allow(dead_code, clippy::too_many_arguments)]
    pub fn export_database_json(
        &self,
        client: &Client,
        database: &str,
        directory: &Path,
        options: JsonExportOptions,
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

            // Create directory if it doesn't exist
            std::fs::create_dir_all(&directory)?;

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
                let extension = match options.format {
                    JsonTransferFormat::JsonLines => "jsonl",
                    JsonTransferFormat::JsonArray => "json",
                };
                let file_name = format!("{}_{}.{}", database, coll_name, extension);
                let file_path = directory.join(&file_name);

                // Export this collection (inlined to avoid nested block_on)
                let coll = client.database(&database).collection::<Document>(&coll_name);
                let mut cursor = coll.find(doc! {}).await?;
                let file = File::create(&file_path)?;

                let mut writer: Box<dyn Write> = if options.gzip {
                    Box::new(BufWriter::new(GzEncoder::new(file, Compression::default())))
                } else {
                    Box::new(BufWriter::new(file))
                };

                let mut count = 0u64;

                if matches!(options.format, JsonTransferFormat::JsonArray) {
                    writer.write_all(b"[")?;
                    if options.pretty_print {
                        writer.write_all(b"\n")?;
                    }
                }

                let mut first = true;
                while let Some(doc) = cursor.try_next().await? {
                    let json_value = match options.json_mode {
                        ExtendedJsonMode::Relaxed => Bson::Document(doc).into_relaxed_extjson(),
                        ExtendedJsonMode::Canonical => Bson::Document(doc).into_canonical_extjson(),
                    };

                    let json = if options.pretty_print {
                        serde_json::to_string_pretty(&json_value)?
                    } else {
                        serde_json::to_string(&json_value)?
                    };

                    match options.format {
                        JsonTransferFormat::JsonLines => {
                            writer.write_all(json.as_bytes())?;
                            writer.write_all(b"\n")?;
                        }
                        JsonTransferFormat::JsonArray => {
                            if !first {
                                writer.write_all(b",")?;
                                if options.pretty_print {
                                    writer.write_all(b"\n")?;
                                }
                            }
                            writer.write_all(json.as_bytes())?;
                            first = false;
                        }
                    }
                    count += 1;
                }

                if matches!(options.format, JsonTransferFormat::JsonArray) {
                    if count > 0 && options.pretty_print {
                        writer.write_all(b"\n")?;
                    }
                    writer.write_all(b"]")?;
                }

                writer.flush()?;
                total_count += count;
            }

            Ok(total_count)
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
        use crate::connection::csv_utils::{collect_columns, flatten_document};
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let directory = directory.to_path_buf();
        let exclude_collections = exclude_collections.to_vec();

        self.runtime.block_on(async move {
            let db = client.database(&database);
            let collections = db.list_collection_names().await?;

            // Create directory if it doesn't exist
            std::fs::create_dir_all(&directory)?;

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

                // Export this collection using single-pass buffering
                let coll = client.database(&database).collection::<Document>(&coll_name);

                // Start single cursor for all documents
                let mut cursor = coll.find(doc! {}).await?;

                // Buffer first N documents to detect columns
                const SAMPLE_SIZE: usize = 1000;
                let mut buffered_docs: Vec<Document> = Vec::with_capacity(SAMPLE_SIZE);

                while buffered_docs.len() < SAMPLE_SIZE {
                    match cursor.try_next().await? {
                        Some(doc) => buffered_docs.push(doc),
                        None => break,
                    }
                }

                // Collect columns from buffered documents
                let columns = collect_columns(&buffered_docs);

                if columns.is_empty() {
                    continue;
                }

                // Write CSV
                let file = File::create(&file_path)?;
                let mut csv_writer = if gzip {
                    csv::Writer::from_writer(
                        Box::new(GzEncoder::new(file, Compression::default())) as Box<dyn Write>
                    )
                } else {
                    csv::Writer::from_writer(Box::new(file) as Box<dyn Write>)
                };

                csv_writer.write_record(&columns)?;

                // Write buffered documents first
                let mut count = 0u64;
                for doc in buffered_docs {
                    let flat = flatten_document(&doc);
                    let row: Vec<String> = columns
                        .iter()
                        .map(|col| flat.get(col).cloned().unwrap_or_default())
                        .collect();
                    csv_writer.write_record(&row)?;
                    count += 1;
                }

                // Continue streaming remaining documents from same cursor
                while let Some(doc) = cursor.try_next().await? {
                    let flat = flatten_document(&doc);
                    let row: Vec<String> = columns
                        .iter()
                        .map(|col| flat.get(col).cloned().unwrap_or_default())
                        .collect();
                    csv_writer.write_record(&row)?;
                    count += 1;
                }

                csv_writer.flush()?;
                total_count += count;
            }

            Ok(total_count)
        })
    }
}

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
