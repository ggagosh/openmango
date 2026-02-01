use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read as _, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use flate2::Compression;
use flate2::write::GzEncoder;

use mongodb::Client;
use mongodb::IndexModel;
use mongodb::bson::doc;
use mongodb::bson::{Bson, Document};
use mongodb::results::{CollectionSpecification, UpdateResult};
use std::time::Duration;
use tokio::runtime::Runtime;

use crate::error::{Error, Result};
use crate::models::SavedConnection;

#[derive(Debug)]
pub enum AggregatePipelineError {
    Mongo(crate::error::Error),
    Aborted,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub enum JsonTransferFormat {
    JsonArray,
    #[default]
    JsonLines,
}

/// Extended JSON output mode
#[derive(Clone, Copy, Debug, Default)]
pub enum ExtendedJsonMode {
    #[default]
    Relaxed,
    Canonical,
}

/// Insert mode for imports
#[derive(Clone, Copy, Debug, Default)]
pub enum InsertMode {
    #[default]
    Insert,
    Upsert,
    Replace,
}

/// Options for JSON export
#[derive(Clone, Debug)]
pub struct JsonExportOptions {
    pub format: JsonTransferFormat,
    pub json_mode: ExtendedJsonMode,
    pub pretty_print: bool,
    pub gzip: bool,
}

impl Default for JsonExportOptions {
    fn default() -> Self {
        Self {
            format: JsonTransferFormat::JsonLines,
            json_mode: ExtendedJsonMode::Relaxed,
            pretty_print: false,
            gzip: false,
        }
    }
}

/// Text encoding for file imports
#[derive(Clone, Copy, Debug, Default)]
pub enum FileEncoding {
    #[default]
    Utf8,
    Latin1,
}

/// Progress callback type for reporting operation progress.
pub type ProgressCallback = Arc<dyn Fn(u64) + Send + Sync>;

/// Cancellation token for aborting long-running operations.
#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self { cancelled: Arc::new(AtomicBool::new(false)) }
    }

    #[allow(dead_code)]
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Options for JSON import
#[derive(Clone, Default)]
pub struct JsonImportOptions {
    pub format: JsonTransferFormat,
    pub insert_mode: InsertMode,
    pub stop_on_error: bool,
    pub batch_size: usize,
    pub encoding: FileEncoding,
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
    pub encoding: FileEncoding,
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

/// BSON output format for mongodump
#[derive(Clone, Copy, Debug, Default)]
pub enum BsonOutputFormat {
    #[default]
    Folder,
    Archive,
}

/// Query options for collection-level exports
#[derive(Clone, Debug, Default)]
pub struct ExportQueryOptions {
    pub filter: Option<Document>,
    pub projection: Option<Document>,
    pub sort: Option<Document>,
}

/// Options for copy operations
#[derive(Clone, Default)]
pub struct CopyOptions {
    pub batch_size: usize,
    pub copy_indexes: bool,
    pub progress: Option<ProgressCallback>,
    pub cancellation: Option<CancellationToken>,
}

impl CopyOptions {
    pub fn new(batch_size: usize, copy_indexes: bool) -> Self {
        Self { batch_size, copy_indexes, progress: None, cancellation: None }
    }
}

impl std::fmt::Debug for CopyOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopyOptions")
            .field("batch_size", &self.batch_size)
            .field("copy_indexes", &self.copy_indexes)
            .field("progress", &self.progress.is_some())
            .field("cancellation", &self.cancellation.is_some())
            .finish()
    }
}

impl From<crate::error::Error> for AggregatePipelineError {
    fn from(value: crate::error::Error) -> Self {
        Self::Mongo(value)
    }
}

pub struct FindDocumentsOptions {
    pub filter: Option<mongodb::bson::Document>,
    pub sort: Option<mongodb::bson::Document>,
    pub projection: Option<mongodb::bson::Document>,
    pub skip: u64,
    pub limit: i64,
}

/// Global singleton connection manager
static CONNECTION_MANAGER: LazyLock<ConnectionManager> = LazyLock::new(ConnectionManager::new);

/// Get the global connection manager instance
pub fn get_connection_manager() -> &'static ConnectionManager {
    &CONNECTION_MANAGER
}

/// Manages MongoDB client connections with caching
pub struct ConnectionManager {
    /// Tokio runtime for MongoDB async operations
    runtime: Runtime,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("Failed to create Tokio runtime");
        Self { runtime }
    }

    /// Get a handle to the Tokio runtime for spawning parallel tasks
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// Connect to MongoDB using the saved connection config (runs in Tokio runtime)
    pub fn connect(&self, config: &SavedConnection) -> Result<Client> {
        let uri = config.uri.clone();
        self.runtime.block_on(async {
            let client = Client::with_uri_str(&uri).await?;

            // Ping to verify connection
            client.database("admin").run_command(doc! { "ping": 1 }).await?;

            Ok(client)
        })
    }

    /// Test connectivity with a timeout (runs in Tokio runtime)
    pub fn test_connection(&self, config: &SavedConnection, timeout: Duration) -> Result<()> {
        let uri = config.uri.clone();
        self.runtime.block_on(async {
            let fut = async {
                let client = Client::with_uri_str(&uri).await?;
                client.database("admin").run_command(doc! { "ping": 1 }).await?;
                Ok::<(), mongodb::error::Error>(())
            };

            match tokio::time::timeout(timeout, fut).await {
                Ok(result) => result.map_err(Error::from),
                Err(_) => Err(Error::Timeout("Connection timed out".to_string())),
            }
        })
    }

    /// List databases for a connected client (runs in Tokio runtime)
    pub fn list_databases(&self, client: &Client) -> Result<Vec<String>> {
        let client = client.clone();
        self.runtime.block_on(async {
            let mut databases = client.list_database_names().await?;
            databases.sort_unstable_by_key(|name| name.to_lowercase());
            Ok(databases)
        })
    }

    /// List collections in a database (runs in Tokio runtime)
    pub fn list_collections(&self, client: &Client, database: &str) -> Result<Vec<String>> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let mut collections = db.list_collection_names().await?;
            collections.sort_unstable_by_key(|name| name.to_lowercase());
            Ok(collections)
        })
    }

    /// List collection specs in a database (runs in Tokio runtime)
    pub fn list_collection_specs(
        &self,
        client: &Client,
        database: &str,
    ) -> Result<Vec<CollectionSpecification>> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let cursor = db.list_collections().await?;
            let mut specs: Vec<CollectionSpecification> = cursor.try_collect().await?;
            specs.sort_unstable_by_key(|spec| spec.name.to_lowercase());
            Ok(specs)
        })
    }

    /// Create a collection in a database (runs in Tokio runtime)
    pub fn create_collection(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            db.create_collection(&collection).await?;
            Ok(())
        })
    }

    /// Drop a collection in a database (runs in Tokio runtime)
    pub fn drop_collection(&self, client: &Client, database: &str, collection: &str) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            coll.drop().await?;
            Ok(())
        })
    }

    /// Rename a collection in a database (runs in Tokio runtime)
    pub fn rename_collection(
        &self,
        client: &Client,
        database: &str,
        from: &str,
        to: &str,
    ) -> Result<()> {
        let client = client.clone();
        let from = format!("{database}.{from}");
        let to = format!("{database}.{to}");
        self.runtime.block_on(async {
            let admin = client.database("admin");
            admin
                .run_command(doc! { "renameCollection": from, "to": to, "dropTarget": false })
                .await?;
            Ok(())
        })
    }

    /// Drop a database (runs in Tokio runtime)
    pub fn drop_database(&self, client: &Client, database: &str) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            db.drop().await?;
            Ok(())
        })
    }

    /// Fetch collection stats (runs in Tokio runtime)
    pub fn collection_stats(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<mongodb::bson::Document> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let stats = db.run_command(doc! { "collStats": collection }).await?;
            Ok(stats)
        })
    }

    /// Fetch database stats (runs in Tokio runtime)
    pub fn database_stats(
        &self,
        client: &Client,
        database: &str,
    ) -> Result<mongodb::bson::Document> {
        let client = client.clone();
        let database = database.to_string();
        self.runtime.block_on(async {
            let db = client.database(&database);
            let stats = db.run_command(doc! { "dbStats": 1 }).await?;
            Ok(stats)
        })
    }

    /// Find documents in a collection with pagination (runs in Tokio runtime)
    pub fn find_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        opts: FindDocumentsOptions,
    ) -> Result<(Vec<mongodb::bson::Document>, u64)> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let filter = opts.filter.unwrap_or_default();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);

            // Get total count (with filter)
            let total = coll.count_documents(filter.clone()).await?;

            // Fetch documents with pagination
            let mut options = mongodb::options::FindOptions::default();
            options.skip = Some(opts.skip);
            options.limit = Some(opts.limit);
            options.sort = opts.sort;
            options.projection = opts.projection;

            let cursor = coll.find(filter).with_options(options).await?;
            let documents: Vec<mongodb::bson::Document> = cursor.try_collect().await?;

            Ok((documents, total))
        })
    }

    /// Insert a document into a collection (runs in Tokio runtime)
    pub fn insert_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        document: mongodb::bson::Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            coll.insert_one(document).await?;
            Ok(())
        })
    }

    /// Insert multiple documents into a collection (runs in Tokio runtime)
    pub fn insert_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        documents: Vec<mongodb::bson::Document>,
    ) -> Result<usize> {
        let count = documents.len();
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            coll.insert_many(documents).await?;
            Ok(count)
        })
    }

    /// Delete multiple documents by filter (runs in Tokio runtime)
    pub fn delete_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: mongodb::bson::Document,
    ) -> Result<u64> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let result = coll.delete_many(filter).await?;
            Ok(result.deleted_count)
        })
    }

    /// List indexes for a collection (runs in Tokio runtime)
    pub fn list_indexes(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<Vec<IndexModel>> {
        use futures::TryStreamExt;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let cursor = coll.list_indexes().await?;
            let indexes: Vec<IndexModel> = cursor.try_collect().await?;
            Ok(indexes)
        })
    }

    /// Sample documents from a collection (runs in Tokio runtime)
    pub fn sample_documents(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        size: i64,
    ) -> Result<Vec<mongodb::bson::Document>> {
        use futures::TryStreamExt;

        if size <= 0 {
            return Ok(Vec::new());
        }

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let pipeline = vec![doc! { "$sample": { "size": size } }];
            let cursor = coll.aggregate(pipeline).await?;
            let docs: Vec<mongodb::bson::Document> = cursor.try_collect().await?;
            Ok(docs)
        })
    }

    /// Get estimated document count for a collection (fast, uses metadata).
    /// This is much faster than count_documents() as it uses collection statistics.
    pub fn estimated_document_count(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<u64> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let count = coll.estimated_document_count().await?;
            Ok(count)
        })
    }

    /// List all collection names in a database (runs in Tokio runtime).
    pub fn list_collection_names(&self, client: &Client, database: &str) -> Result<Vec<String>> {
        let client = client.clone();
        let database = database.to_string();

        self.runtime.block_on(async {
            let db = client.database(&database);
            let names = db.list_collection_names().await?;
            Ok(names)
        })
    }

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

    /// Import a collection from JSON/JSONL (runs in Tokio runtime).
    #[allow(dead_code)]
    pub fn import_collection_json(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        format: JsonTransferFormat,
        path: &Path,
        batch_size: usize,
    ) -> Result<u64> {
        self.import_collection_json_with_options(
            client,
            database,
            collection,
            path,
            JsonImportOptions { format, batch_size, ..Default::default() },
        )
    }

    /// Import a collection from JSON/JSONL with full options (runs in Tokio runtime).
    /// Uses streaming for JSONL format to minimize memory usage on large files.
    pub fn import_collection_json_with_options(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        options: JsonImportOptions,
    ) -> Result<u64> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);
            let mut processed = 0u64;

            match options.format {
                JsonTransferFormat::JsonLines => {
                    // Stream JSONL line-by-line to minimize memory usage
                    let file = File::open(&path)?;
                    let reader: Box<dyn BufRead + Send> = match options.encoding {
                        FileEncoding::Utf8 => Box::new(BufReader::new(file)),
                        FileEncoding::Latin1 => {
                            // For Latin-1, we need to decode first (read entire file)
                            // This is unavoidable for non-UTF-8 encodings
                            let bytes = std::fs::read(&path)?;
                            let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
                            Box::new(std::io::Cursor::new(decoded.into_owned().into_bytes()))
                        }
                    };

                    let mut batch: Vec<Document> = Vec::with_capacity(options.batch_size);

                    for line_result in reader.lines() {
                        // Check cancellation
                        if options.cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                            return Err(Error::Parse("Import cancelled".to_string()));
                        }

                        let line = line_result?;
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        let doc =
                            crate::bson::parse_document_from_json(trimmed).map_err(Error::Parse)?;
                        batch.push(doc);

                        // Insert batch when full
                        if batch.len() >= options.batch_size {
                            let result = import_batch_by_mode(
                                &coll,
                                &batch,
                                options.insert_mode,
                                options.stop_on_error,
                            )
                            .await;

                            match result {
                                Ok(count) => {
                                    processed += count;
                                    if let Some(ref progress) = options.progress {
                                        progress(processed);
                                    }
                                }
                                Err(e) if options.stop_on_error => return Err(e),
                                Err(_) => {}
                            }
                            batch.clear();
                        }
                    }

                    // Flush remaining documents
                    if !batch.is_empty() {
                        let result = import_batch_by_mode(
                            &coll,
                            &batch,
                            options.insert_mode,
                            options.stop_on_error,
                        )
                        .await;

                        match result {
                            Ok(count) => {
                                processed += count;
                                if let Some(ref progress) = options.progress {
                                    progress(processed);
                                }
                            }
                            Err(e) if options.stop_on_error => return Err(e),
                            Err(_) => {}
                        }
                    }
                }
                JsonTransferFormat::JsonArray => {
                    // JSON arrays require parsing the entire structure
                    // Use streaming JSON parser for large arrays
                    let file = File::open(&path)?;
                    let content = match options.encoding {
                        FileEncoding::Utf8 => {
                            let mut reader = BufReader::new(file);
                            let mut content = String::new();
                            reader.read_to_string(&mut content)?;
                            content
                        }
                        FileEncoding::Latin1 => {
                            let bytes = std::fs::read(&path)?;
                            let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
                            decoded.into_owned()
                        }
                    };

                    // Parse all documents from JSON array
                    let docs =
                        crate::bson::parse_documents_from_json(&content).map_err(Error::Parse)?;

                    // Process in batches
                    for batch in docs.chunks(options.batch_size) {
                        // Check cancellation
                        if options.cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                            return Err(Error::Parse("Import cancelled".to_string()));
                        }

                        let result = import_batch_by_mode(
                            &coll,
                            batch,
                            options.insert_mode,
                            options.stop_on_error,
                        )
                        .await;

                        match result {
                            Ok(count) => {
                                processed += count;
                                if let Some(ref progress) = options.progress {
                                    progress(processed);
                                }
                            }
                            Err(e) if options.stop_on_error => return Err(e),
                            Err(_) => {}
                        }
                    }
                }
            }

            Ok(processed)
        })
    }

    /// Import a collection from CSV (runs in Tokio runtime).
    /// Uses streaming to process CSV records in batches without loading entire file.
    pub fn import_collection_csv(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        path: &Path,
        options: CsvImportOptions,
    ) -> Result<u64> {
        use crate::connection::csv_utils::unflatten_row;
        use std::collections::HashMap;

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);

            // Create CSV reader with streaming
            let file = File::open(&path)?;
            let reader: Box<dyn std::io::Read + Send> = match options.encoding {
                FileEncoding::Utf8 => Box::new(BufReader::new(file)),
                FileEncoding::Latin1 => {
                    // For Latin-1, decode the entire file first
                    let bytes = std::fs::read(&path)?;
                    let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
                    Box::new(std::io::Cursor::new(decoded.into_owned().into_bytes()))
                }
            };

            let mut csv_reader = csv::Reader::from_reader(reader);
            let headers: Vec<String> =
                csv_reader.headers()?.iter().map(|h| h.to_string()).collect();

            let mut batch: Vec<Document> = Vec::with_capacity(options.batch_size);
            let mut processed = 0u64;

            for result in csv_reader.records() {
                // Check cancellation
                if options.cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err(Error::Parse("Import cancelled".to_string()));
                }

                let record = result?;
                let mut row: HashMap<String, String> = HashMap::new();
                for (i, value) in record.iter().enumerate() {
                    if let Some(header) = headers.get(i) {
                        row.insert(header.clone(), value.to_string());
                    }
                }
                batch.push(unflatten_row(&row));

                // Insert batch when full
                if batch.len() >= options.batch_size {
                    let result = import_batch_by_mode(
                        &coll,
                        &batch,
                        options.insert_mode,
                        options.stop_on_error,
                    )
                    .await;

                    match result {
                        Ok(count) => {
                            processed += count;
                            if let Some(ref progress) = options.progress {
                                progress(processed);
                            }
                        }
                        Err(e) if options.stop_on_error => return Err(e),
                        Err(_) => {}
                    }
                    batch.clear();
                }
            }

            // Flush remaining documents
            if !batch.is_empty() {
                let result =
                    import_batch_by_mode(&coll, &batch, options.insert_mode, options.stop_on_error)
                        .await;

                match result {
                    Ok(count) => {
                        processed += count;
                        if let Some(ref progress) = options.progress {
                            progress(processed);
                        }
                    }
                    Err(e) if options.stop_on_error => return Err(e),
                    Err(_) => {}
                }
            }

            Ok(processed)
        })
    }

    /// Copy a collection from one connection/database to another (runs in Tokio runtime).
    /// Supports cancellation and progress callbacks.
    #[allow(clippy::too_many_arguments)]
    pub fn copy_collection(
        &self,
        src_client: &Client,
        src_database: &str,
        src_collection: &str,
        dest_client: &Client,
        dest_database: &str,
        dest_collection: &str,
        batch_size: usize,
        copy_indexes: bool,
    ) -> Result<u64> {
        self.copy_collection_with_options(
            src_client,
            src_database,
            src_collection,
            dest_client,
            dest_database,
            dest_collection,
            CopyOptions::new(batch_size, copy_indexes),
        )
    }

    /// Copy a collection with full options including progress and cancellation.
    #[allow(clippy::too_many_arguments)]
    pub fn copy_collection_with_options(
        &self,
        src_client: &Client,
        src_database: &str,
        src_collection: &str,
        dest_client: &Client,
        dest_database: &str,
        dest_collection: &str,
        options: CopyOptions,
    ) -> Result<u64> {
        use futures::TryStreamExt;
        use mongodb::options::InsertManyOptions;

        let src_client = src_client.clone();
        let dest_client = dest_client.clone();
        let src_database = src_database.to_string();
        let src_collection = src_collection.to_string();
        let dest_database = dest_database.to_string();
        let dest_collection = dest_collection.to_string();
        let batch_size = options.batch_size;
        let progress = options.progress.clone();
        let cancellation = options.cancellation.clone();

        let copied = self.runtime.block_on(async {
            let src_coll =
                src_client.database(&src_database).collection::<Document>(&src_collection);
            let dest_coll =
                dest_client.database(&dest_database).collection::<Document>(&dest_collection);

            let mut cursor = src_coll.find(doc! {}).await?;
            let mut batch: Vec<Document> = Vec::with_capacity(batch_size);
            let mut copied = 0u64;

            let insert_options = InsertManyOptions::builder().ordered(false).build();

            while let Some(doc) = cursor.try_next().await? {
                // Check cancellation
                if cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err(Error::Parse("Copy cancelled".to_string()));
                }

                batch.push(doc);
                if batch.len() >= batch_size {
                    let docs = std::mem::take(&mut batch);
                    copied += docs.len() as u64;
                    dest_coll.insert_many(docs).with_options(insert_options.clone()).await?;

                    // Report progress
                    if let Some(ref progress_fn) = progress {
                        progress_fn(copied);
                    }
                }
            }

            // Flush remaining
            if !batch.is_empty() {
                copied += batch.len() as u64;
                dest_coll.insert_many(batch).with_options(insert_options).await?;

                // Report final progress
                if let Some(ref progress_fn) = progress {
                    progress_fn(copied);
                }
            }

            Ok::<u64, Error>(copied)
        })?;

        // Copy indexes if requested (after documents are copied)
        if options.copy_indexes {
            let indexes = self.list_indexes(&src_client, &src_database, &src_collection)?;
            for index in indexes {
                // Skip _id_ index (auto-created)
                let name = index
                    .options
                    .as_ref()
                    .and_then(|opts| opts.name.as_ref())
                    .map(|n| n.as_str())
                    .unwrap_or("");
                if name == "_id_" {
                    continue;
                }

                // Build index doc from IndexModel
                let mut index_doc = doc! { "key": index.keys.clone() };
                if let Some(opts) = &index.options {
                    if let Some(n) = &opts.name {
                        index_doc.insert("name", n.clone());
                    }
                    if let Some(u) = opts.unique {
                        index_doc.insert("unique", u);
                    }
                    if let Some(s) = opts.sparse {
                        index_doc.insert("sparse", s);
                    }
                    if let Some(exp) = opts.expire_after {
                        index_doc.insert("expireAfterSeconds", exp.as_secs() as i64);
                    }
                    if let Some(bg) = opts.background {
                        index_doc.insert("background", bg);
                    }
                }

                // Non-fatal: log warning if index creation fails
                if let Err(e) =
                    self.create_index(&dest_client, &dest_database, &dest_collection, index_doc)
                {
                    log::warn!("Failed to copy index '{}': {}", name, e);
                }
            }
        }

        Ok(copied)
    }

    /// Copy all collections from one database to another (runs in Tokio runtime).
    /// Uses HashSet for O(1) excluded collection lookup.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub fn copy_database(
        &self,
        src_client: &Client,
        src_database: &str,
        dest_client: &Client,
        dest_database: &str,
        batch_size: usize,
        copy_indexes: bool,
        exclude_collections: &[String],
    ) -> Result<u64> {
        use std::collections::HashSet;

        let src_client = src_client.clone();
        let dest_client = dest_client.clone();
        let src_database = src_database.to_string();
        let dest_database = dest_database.to_string();
        // Use HashSet for O(1) lookup instead of Vec::contains O(n)
        let exclude_set: HashSet<String> = exclude_collections.iter().cloned().collect();

        self.runtime.block_on(async move {
            let src_db = src_client.database(&src_database);
            let collections = src_db.list_collection_names().await?;

            let mut total_copied = 0u64;

            for collection in collections {
                // Skip system collections
                if collection.starts_with("system.") {
                    continue;
                }

                // Skip excluded collections (O(1) lookup)
                if exclude_set.contains(&collection) {
                    continue;
                }

                let count = get_connection_manager().copy_collection(
                    &src_client,
                    &src_database,
                    &collection,
                    &dest_client,
                    &dest_database,
                    &collection,
                    batch_size,
                    copy_indexes,
                )?;
                total_copied += count;
            }

            Ok(total_copied)
        })
    }

    /// Export a database to BSON format using mongodump (runs synchronously).
    pub fn export_database_bson(
        &self,
        connection_string: &str,
        database: &str,
        output_format: BsonOutputFormat,
        path: &Path,
        gzip: bool,
        exclude_collections: &[String],
    ) -> Result<()> {
        let mongodump = mongodump_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongodump not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut cmd = Command::new(&mongodump);
        cmd.arg(format!("--uri={}", connection_string)).arg("--db").arg(database);

        if gzip {
            cmd.arg("--gzip");
        }

        // Add exclude collection flags
        for collection in exclude_collections {
            cmd.arg("--excludeCollection").arg(collection);
        }

        match output_format {
            BsonOutputFormat::Folder => {
                cmd.arg("--out").arg(path);
            }
            BsonOutputFormat::Archive => {
                let archive_path = if path.extension().map(|e| e == "archive").unwrap_or(false) {
                    path.to_path_buf()
                } else {
                    path.with_extension("archive")
                };
                cmd.arg("--archive").arg(&archive_path);
            }
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Parse(format!("mongodump failed: {}", stderr)));
        }

        Ok(())
    }

    /// Import a database from BSON format using mongorestore (runs synchronously).
    pub fn import_database_bson(
        &self,
        connection_string: &str,
        database: &str,
        path: &Path,
        drop_before: bool,
    ) -> Result<()> {
        let mongorestore = mongorestore_path().ok_or_else(|| {
            Error::ToolNotFound(
                "mongorestore not found. Run 'just download-tools' or install MongoDB Database Tools."
                    .into(),
            )
        })?;

        let mut cmd = Command::new(&mongorestore);
        cmd.arg(format!("--uri={}", connection_string)).arg("--db").arg(database);

        if drop_before {
            cmd.arg("--drop");
        }

        // Detect if path is archive or folder
        if path.extension().map(|e| e == "archive").unwrap_or(false) {
            cmd.arg("--archive").arg(path);
        } else {
            // mongodump creates a subfolder with the database name
            let db_path = path.join(database);
            if db_path.exists() {
                cmd.arg("--dir").arg(&db_path);
            } else {
                cmd.arg("--dir").arg(path);
            }
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Parse(format!("mongorestore failed: {}", stderr)));
        }

        Ok(())
    }

    /// Run an aggregation pipeline for a collection with abort support (runs in Tokio runtime)
    #[allow(clippy::too_many_arguments)]
    pub fn aggregate_pipeline_abortable(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        mut pipeline: Vec<mongodb::bson::Document>,
        limit: Option<i64>,
        append_limit: bool,
        abort_registration: futures::future::AbortRegistration,
    ) -> std::result::Result<Vec<mongodb::bson::Document>, AggregatePipelineError> {
        use futures::TryStreamExt;
        use futures::future::Abortable;

        if append_limit
            && let Some(limit) = limit
            && limit > 0
        {
            pipeline.push(doc! { "$limit": limit });
        }

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let fut = async move {
                let cursor = coll.aggregate(pipeline).await?;
                let docs: Vec<mongodb::bson::Document> = cursor.try_collect().await?;
                Ok::<_, crate::error::Error>(docs)
            };
            match Abortable::new(fut, abort_registration).await {
                Ok(result) => result.map_err(AggregatePipelineError::from),
                Err(_aborted) => Err(AggregatePipelineError::Aborted),
            }
        })
    }

    /// Create an index for a collection (runs in Tokio runtime)
    pub fn create_index(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        index: mongodb::bson::Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let db = client.database(&database);
            db.run_command(doc! { "createIndexes": collection, "indexes": [index] }).await?;
            Ok(())
        })
    }

    /// Drop an index by name in a collection (runs in Tokio runtime)
    pub fn drop_index(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        name: &str,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let name = name.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            coll.drop_index(name).await?;
            Ok(())
        })
    }

    /// Update a single document (runs in Tokio runtime)
    pub fn update_one(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: mongodb::bson::Document,
        update: mongodb::bson::Document,
    ) -> Result<UpdateResult> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let result = coll.update_one(filter, update).await?;
            Ok(result)
        })
    }

    /// Update multiple documents (runs in Tokio runtime)
    pub fn update_many(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        filter: mongodb::bson::Document,
        update: mongodb::bson::Document,
    ) -> Result<UpdateResult> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);
            let result = coll.update_many(filter, update).await?;
            Ok(result)
        })
    }

    /// Replace a document by _id in a collection (runs in Tokio runtime)
    pub fn replace_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
        replacement: mongodb::bson::Document,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);

            coll.replace_one(doc! { "_id": id }, replacement).await?;
            Ok(())
        })
    }

    /// Delete a document by _id in a collection (runs in Tokio runtime)
    pub fn delete_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        id: &mongodb::bson::Bson,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let id = id.clone();

        self.runtime.block_on(async {
            let coll =
                client.database(&database).collection::<mongodb::bson::Document>(&collection);

            coll.delete_one(doc! { "_id": id }).await?;
            Ok(())
        })
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

// Preview and tools functions

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

/// Check if mongodump/mongorestore tools are available.
pub fn tools_available() -> bool {
    mongodump_path().is_some() && mongorestore_path().is_some()
}

/// Find the path to mongodump executable.
pub fn mongodump_path() -> Option<std::path::PathBuf> {
    find_mongo_tool("mongodump")
}

/// Find the path to mongorestore executable.
pub fn mongorestore_path() -> Option<std::path::PathBuf> {
    find_mongo_tool("mongorestore")
}

fn find_mongo_tool(name: &str) -> Option<std::path::PathBuf> {
    // 1. Check app bundle (macOS)
    #[cfg(target_os = "macos")]
    {
        if let Ok(exe_path) = std::env::current_exe() {
            // In app bundle: ../Resources/bin/mongodump
            if let Some(parent) = exe_path.parent() {
                let bundle_path = parent.join("../Resources/bin").join(name);
                if bundle_path.exists() && is_executable(&bundle_path) {
                    return Some(bundle_path);
                }
            }
        }
    }

    // 2. Check resources/bin (dev mode) with architecture-specific paths
    let arch_dir = dev_tools_arch();
    let dev_path = std::path::PathBuf::from("resources/bin").join(arch_dir).join(name);
    if dev_path.exists() && is_executable(&dev_path) {
        return Some(dev_path);
    }

    // 3. Check PATH
    which::which(name).ok()
}

/// Get the architecture-specific directory name for dev mode tools
fn dev_tools_arch() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        "unknown"
    }
}

/// Check if a path is executable
fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

// Import mode helper functions

/// Helper to dispatch batch import by mode.
async fn import_batch_by_mode(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    mode: InsertMode,
    ordered: bool,
) -> Result<u64> {
    match mode {
        InsertMode::Insert => import_batch_insert(coll, batch, ordered).await,
        InsertMode::Upsert => import_batch_upsert(coll, batch, ordered).await,
        InsertMode::Replace => import_batch_replace(coll, batch, ordered).await,
    }
}

async fn import_batch_insert(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
    use mongodb::options::InsertManyOptions;

    if batch.is_empty() {
        return Ok(0);
    }

    let options = InsertManyOptions::builder().ordered(ordered).build();
    coll.insert_many(batch.to_vec()).with_options(options).await?;
    Ok(batch.len() as u64)
}

/// Upsert documents using concurrent update operations.
/// Groups documents by whether they have _id for efficient processing.
async fn import_batch_upsert(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
    use mongodb::bson::doc;
    use mongodb::options::{InsertManyOptions, UpdateOptions};

    if batch.is_empty() {
        return Ok(0);
    }

    // Separate documents with _id (upsert) from those without (insert)
    let mut with_id: Vec<&Document> = Vec::new();
    let mut without_id: Vec<Document> = Vec::new();

    for doc in batch {
        if doc.get("_id").is_some() {
            with_id.push(doc);
        } else {
            without_id.push(doc.clone());
        }
    }

    let mut count = 0u64;
    let update_options = UpdateOptions::builder().upsert(true).build();

    // Process documents with _id using update_one with upsert
    // Note: MongoDB doesn't have bulk_write on Collection, only on Client (8.0+)
    // We optimize by not creating options for every document
    for doc in with_id {
        let id = doc.get("_id").unwrap();
        let filter = doc! { "_id": id.clone() };
        let mut update_doc = doc.clone();
        update_doc.remove("_id");

        let result = coll
            .update_one(filter, doc! { "$set": update_doc })
            .with_options(update_options.clone())
            .await;

        match result {
            Ok(_) => count += 1,
            Err(e) if ordered => return Err(e.into()),
            Err(_) => {} // Continue on error in unordered mode
        }
    }

    // Insert documents without _id
    if !without_id.is_empty() {
        let insert_options = InsertManyOptions::builder().ordered(ordered).build();
        match coll.insert_many(without_id.clone()).with_options(insert_options).await {
            Ok(_) => count += without_id.len() as u64,
            Err(e) if ordered => return Err(e.into()),
            Err(_) => {}
        }
    }

    Ok(count)
}

/// Replace documents using concurrent replace operations.
async fn import_batch_replace(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
    use mongodb::bson::doc;
    use mongodb::options::{InsertManyOptions, ReplaceOptions};

    if batch.is_empty() {
        return Ok(0);
    }

    // Separate documents with _id (replace) from those without (insert)
    let mut with_id: Vec<&Document> = Vec::new();
    let mut without_id: Vec<Document> = Vec::new();

    for doc in batch {
        if doc.get("_id").is_some() {
            with_id.push(doc);
        } else {
            without_id.push(doc.clone());
        }
    }

    let mut count = 0u64;
    let replace_options = ReplaceOptions::builder().upsert(true).build();

    // Process documents with _id using replace_one with upsert
    for doc in with_id {
        let id = doc.get("_id").unwrap();
        let filter = doc! { "_id": id.clone() };

        let result =
            coll.replace_one(filter, doc.clone()).with_options(replace_options.clone()).await;

        match result {
            Ok(_) => count += 1,
            Err(e) if ordered => return Err(e.into()),
            Err(_) => {} // Continue on error in unordered mode
        }
    }

    // Insert documents without _id
    if !without_id.is_empty() {
        let insert_options = InsertManyOptions::builder().ordered(ordered).build();
        match coll.insert_many(without_id.clone()).with_options(insert_options).await {
            Ok(_) => count += without_id.len() as u64,
            Err(e) if ordered => return Err(e.into()),
            Err(_) => {}
        }
    }

    Ok(count)
}

// No public server info returned yet.

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{Bson, Document};
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct DbCleanup<'a> {
        manager: &'a ConnectionManager,
        client: Client,
        database: String,
    }

    impl<'a> Drop for DbCleanup<'a> {
        fn drop(&mut self) {
            let _ = self.manager.drop_database(&self.client, &self.database);
        }
    }

    fn test_uri() -> Option<String> {
        env::var("MONGO_URI").ok().filter(|value| !value.trim().is_empty())
    }

    fn unique_db_name(prefix: &str) -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let suffix = format!("{}_{}", std::process::id(), now.as_millis());
        format!("om_smoke_{prefix}_{suffix}")
    }

    #[test]
    fn smoke_core_flows() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping smoke_core_flows: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Test".to_string(), uri);

        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let databases = manager.list_databases(&client)?;
        if databases.is_empty() {
            return Ok(());
        }

        let db_name = env::var("MONGO_DB")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| databases[0].clone());
        let collections = manager.list_collections(&client, &db_name)?;
        if collections.is_empty() {
            return Ok(());
        }

        let collection = env::var("MONGO_COLLECTION")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| collections[0].clone());

        let _ = manager.find_documents(
            &client,
            &db_name,
            &collection,
            FindDocumentsOptions { filter: None, sort: None, projection: None, skip: 0, limit: 1 },
        )?;

        Ok(())
    }

    #[test]
    fn crud_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping crud_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke CRUD".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("crud");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        let doc_a = doc! { "_id": "a", "name": "first", "n": 1 };
        let doc_b = doc! { "_id": "b", "name": "second", "n": 2 };
        manager.insert_document(&client, &database, collection, doc_a)?;
        manager.insert_document(&client, &database, collection, doc_b)?;

        let (docs, total) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions { filter: None, sort: None, projection: None, skip: 0, limit: 10 },
        )?;
        if total < 2 || docs.len() < 2 {
            return Err(Error::Timeout("CRUD insert failed".to_string()));
        }

        let updated = doc! { "_id": "a", "name": "updated", "n": 10 };
        manager.replace_document(
            &client,
            &database,
            collection,
            &Bson::String("a".into()),
            updated,
        )?;

        let (docs, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: Some(doc! { "_id": "a" }),
                sort: None,
                projection: None,
                skip: 0,
                limit: 1,
            },
        )?;
        let updated_name =
            docs.first().and_then(|doc| doc.get_str("name").ok()).unwrap_or_default();
        if updated_name != "updated" {
            return Err(Error::Timeout("CRUD update failed".to_string()));
        }

        manager.delete_document(&client, &database, collection, &Bson::String("b".into()))?;

        let (_, total) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions { filter: None, sort: None, projection: None, skip: 0, limit: 10 },
        )?;
        if total != 1 {
            return Err(Error::Timeout("CRUD delete failed".to_string()));
        }

        Ok(())
    }

    #[test]
    fn query_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping query_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Query".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("query");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        let docs = vec![
            doc! { "_id": 1, "value": "b", "n": 2 },
            doc! { "_id": 2, "value": "a", "n": 1 },
            doc! { "_id": 3, "value": "c", "n": 3 },
        ];
        for doc in docs {
            manager.insert_document(&client, &database, collection, doc)?;
        }

        let (filtered, total) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: Some(doc! { "value": "a" }),
                sort: None,
                projection: None,
                skip: 0,
                limit: 10,
            },
        )?;
        if total != 1 || filtered.len() != 1 {
            return Err(Error::Timeout("Query filter failed".to_string()));
        }

        let (sorted, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: None,
                sort: Some(doc! { "n": 1 }),
                projection: None,
                skip: 0,
                limit: 3,
            },
        )?;
        let first_n = sorted.first().and_then(|doc| doc.get_i32("n").ok()).unwrap_or_default();
        if first_n != 1 {
            return Err(Error::Timeout("Query sort failed".to_string()));
        }

        let (paged, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: None,
                sort: Some(doc! { "n": 1 }),
                projection: None,
                skip: 1,
                limit: 1,
            },
        )?;
        let paged_n = paged.first().and_then(|doc| doc.get_i32("n").ok()).unwrap_or_default();
        if paged_n != 2 {
            return Err(Error::Timeout("Query pagination failed".to_string()));
        }

        let (projected, _) = manager.find_documents(
            &client,
            &database,
            collection,
            FindDocumentsOptions {
                filter: None,
                sort: None,
                projection: Some(doc! { "value": 1, "_id": 0 }),
                skip: 0,
                limit: 1,
            },
        )?;
        let projected_doc =
            projected.first().ok_or_else(|| Error::Timeout("Projection failed".to_string()))?;
        if projected_doc.get("_id").is_some() || projected_doc.get("value").is_none() {
            return Err(Error::Timeout("Query projection failed".to_string()));
        }

        Ok(())
    }

    #[test]
    fn indexes_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping indexes_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Indexes".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("indexes");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        manager.insert_document(&client, &database, collection, doc! { "_id": 1, "n": 1 })?;

        let index = IndexModel::builder().keys(doc! { "n": 1 }).build();

        manager.runtime.block_on(async {
            let coll = client.database(&database).collection::<Document>(collection);
            coll.create_index(index).await.map(|_| ())
        })?;

        let indexes = manager.list_indexes(&client, &database, collection)?;
        let created = indexes.iter().find(|model| model.keys == doc! { "n": 1 });
        let Some(created) = created else {
            return Err(Error::Timeout("Index list failed".to_string()));
        };
        let Some(name) = created.options.as_ref().and_then(|options| options.name.as_ref()) else {
            return Err(Error::Timeout("Index name missing".to_string()));
        };

        manager.drop_index(&client, &database, collection, name)?;

        let indexes = manager.list_indexes(&client, &database, collection)?;
        let still_has_index = indexes.iter().any(|model| model.keys == doc! { "n": 1 });
        if still_has_index {
            return Err(Error::Timeout("Index drop failed".to_string()));
        }

        Ok(())
    }

    #[test]
    fn stats_sanity() -> Result<()> {
        let uri = match test_uri() {
            Some(value) => value,
            None => {
                eprintln!("Skipping stats_sanity: MONGO_URI not set.");
                return Ok(());
            }
        };

        let manager = get_connection_manager();
        let connection = SavedConnection::new("Smoke Stats".to_string(), uri);
        manager.test_connection(&connection, Duration::from_secs(5))?;
        let client = manager.connect(&connection)?;

        let database = unique_db_name("stats");
        let collection = "docs";
        let _cleanup = DbCleanup { manager, client: client.clone(), database: database.clone() };

        manager.insert_document(&client, &database, collection, doc! { "_id": 1, "n": 1 })?;

        let db_stats = manager.database_stats(&client, &database)?;
        if db_stats.get("db").is_none() {
            return Err(Error::Timeout("Database stats missing".to_string()));
        }

        let coll_stats = manager.collection_stats(&client, &database, collection)?;
        if coll_stats.get("count").is_none() {
            return Err(Error::Timeout("Collection stats missing".to_string()));
        }

        Ok(())
    }
}
