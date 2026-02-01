//! JSON/JSONL export operations for collections and databases.

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
}
