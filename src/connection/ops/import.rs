//! Collection import operations (JSON, CSV).

use std::fs::File;
use std::io::{BufRead, BufReader, Read as _};
use std::path::Path;

use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::connection::types::{
    CancellationToken, CsvImportOptions, Encoding, InsertMode, JsonImportOptions,
    JsonTransferFormat, TargetWriteMode,
};
use crate::error::{Error, Result};

impl ConnectionManager {
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
        if options.batch_size == 0 {
            return Err(Error::Parse("Import batch size must be greater than zero".to_string()));
        }

        if options.target_write_mode != TargetWriteMode::Append {
            let target_write_mode = options.target_write_mode;
            let cancellation = options.cancellation.clone();
            let mut staged_options = options;
            staged_options.target_write_mode = TargetWriteMode::Append;
            return self.with_staged_collection(
                client,
                database,
                collection,
                target_write_mode,
                cancellation.as_ref(),
                |staging_collection| {
                    self.import_collection_json_with_options(
                        client,
                        database,
                        staging_collection,
                        path,
                        staged_options,
                    )
                },
            );
        }

        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();
        let path = path.to_path_buf();

        self.runtime.block_on(async move {
            let coll = client.database(&database).collection::<Document>(&collection);
            let mut processed = 0u64;
            let mut first_error = Vec::new();

            match options.format {
                JsonTransferFormat::JsonLines => {
                    // Stream JSONL line-by-line to minimize memory usage
                    let file = File::open(&path)?;
                    let reader: Box<dyn BufRead + Send> = match options.encoding {
                        Encoding::Utf8 => Box::new(BufReader::new(file)),
                        Encoding::Latin1 => {
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
                            return Err(Error::Parse("Import cancelled".to_string())
                                .with_processed(processed));
                        }

                        let line = line_result
                            .map_err(Error::from)
                            .map_err(|error| error.with_processed(processed))?;
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        let doc = crate::bson::parse_document_from_json(trimmed)
                            .map_err(Error::Parse)
                            .map_err(|error| error.with_processed(processed))?;
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

                            record_import_batch(
                                result,
                                options.stop_on_error,
                                &mut processed,
                                &mut first_error,
                                options.progress.as_ref(),
                            )?;
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

                        record_import_batch(
                            result,
                            options.stop_on_error,
                            &mut processed,
                            &mut first_error,
                            options.progress.as_ref(),
                        )?;
                    }
                }
                JsonTransferFormat::JsonArray => {
                    // JSON arrays require parsing the entire structure
                    // Use streaming JSON parser for large arrays
                    let file = File::open(&path)?;
                    let content = match options.encoding {
                        Encoding::Utf8 => {
                            let mut reader = BufReader::new(file);
                            let mut content = String::new();
                            reader.read_to_string(&mut content)?;
                            content
                        }
                        Encoding::Latin1 => {
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
                            return Err(Error::Parse("Import cancelled".to_string())
                                .with_processed(processed));
                        }

                        let result = import_batch_by_mode(
                            &coll,
                            batch,
                            options.insert_mode,
                            options.stop_on_error,
                        )
                        .await;

                        record_import_batch(
                            result,
                            options.stop_on_error,
                            &mut processed,
                            &mut first_error,
                            options.progress.as_ref(),
                        )?;
                    }
                }
            }

            finish_import(processed, first_error)
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
        if options.batch_size == 0 {
            return Err(Error::Parse("Import batch size must be greater than zero".to_string()));
        }

        if options.target_write_mode != TargetWriteMode::Append {
            let target_write_mode = options.target_write_mode;
            let cancellation = options.cancellation.clone();
            let mut staged_options = options;
            staged_options.target_write_mode = TargetWriteMode::Append;
            return self.with_staged_collection(
                client,
                database,
                collection,
                target_write_mode,
                cancellation.as_ref(),
                |staging_collection| {
                    self.import_collection_csv(
                        client,
                        database,
                        staging_collection,
                        path,
                        staged_options,
                    )
                },
            );
        }

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
                Encoding::Utf8 => Box::new(BufReader::new(file)),
                Encoding::Latin1 => {
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
            let mut first_error = Vec::new();

            for result in csv_reader.records() {
                // Check cancellation
                if options.cancellation.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err(
                        Error::Parse("Import cancelled".to_string()).with_processed(processed)
                    );
                }

                let record =
                    result.map_err(Error::from).map_err(|error| error.with_processed(processed))?;
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

                    record_import_batch(
                        result,
                        options.stop_on_error,
                        &mut processed,
                        &mut first_error,
                        options.progress.as_ref(),
                    )?;
                    batch.clear();
                }
            }

            // Flush remaining documents
            if !batch.is_empty() {
                let result =
                    import_batch_by_mode(&coll, &batch, options.insert_mode, options.stop_on_error)
                        .await;

                record_import_batch(
                    result,
                    options.stop_on_error,
                    &mut processed,
                    &mut first_error,
                    options.progress.as_ref(),
                )?;
            }

            finish_import(processed, first_error)
        })
    }

    pub(crate) fn with_staged_collection<F>(
        &self,
        client: &Client,
        database: &str,
        target_collection: &str,
        target_write_mode: TargetWriteMode,
        cancellation: Option<&CancellationToken>,
        write_staging_collection: F,
    ) -> Result<u64>
    where
        F: FnOnce(&str) -> Result<u64>,
    {
        debug_assert_ne!(target_write_mode, TargetWriteMode::Append);

        let staging_collection = format!("__openmango_stage_{}", uuid::Uuid::new_v4().simple());
        self.create_collection(client, database, &staging_collection)?;

        let count = match write_staging_collection(&staging_collection) {
            Ok(count) => count,
            Err(error) => {
                return Err(self.cleanup_staging_error(
                    client,
                    database,
                    &staging_collection,
                    error,
                ));
            }
        };

        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(self.cleanup_staging_error(
                client,
                database,
                &staging_collection,
                Error::Parse("Transfer cancelled".to_string()),
            ));
        }

        if let Err(error) = self.promote_staged_collection(
            client,
            database,
            &staging_collection,
            target_collection,
            target_write_mode,
        ) {
            return Err(self.cleanup_staging_error(client, database, &staging_collection, error));
        }

        if target_write_mode == TargetWriteMode::Clear
            && let Err(error) =
                self.discard_staging_collection(client, database, &staging_collection)
        {
            return Err(Error::PartialTransfer { processed: count, source: Box::new(error) });
        }

        Ok(count)
    }

    fn promote_staged_collection(
        &self,
        client: &Client,
        database: &str,
        staging_collection: &str,
        target_collection: &str,
        target_write_mode: TargetWriteMode,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let staging_collection = staging_collection.to_string();
        let target_collection = target_collection.to_string();

        self.runtime.block_on(async move {
            match target_write_mode {
                TargetWriteMode::Append => unreachable!("append mode does not use staging"),
                TargetWriteMode::Clear => {
                    use futures::TryStreamExt;

                    let staging =
                        client.database(&database).collection::<Document>(&staging_collection);
                    let mut cursor = staging
                        .aggregate(vec![doc! {
                            "$out": { "db": &database, "coll": &target_collection }
                        }])
                        .await?;
                    while cursor.try_next().await?.is_some() {}
                }
                TargetWriteMode::Drop => {
                    client
                        .database("admin")
                        .run_command(doc! {
                            "renameCollection": format!("{database}.{staging_collection}"),
                            "to": format!("{database}.{target_collection}"),
                            "dropTarget": true,
                        })
                        .await?;
                }
            }
            Ok(())
        })
    }

    fn discard_staging_collection(
        &self,
        client: &Client,
        database: &str,
        staging_collection: &str,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let staging_collection = staging_collection.to_string();

        self.runtime.block_on(async move {
            let db = client.database(&database);
            if db.list_collection_names().await?.contains(&staging_collection) {
                db.collection::<Document>(&staging_collection).drop().await?;
            }
            Ok(())
        })
    }

    fn cleanup_staging_error(
        &self,
        client: &Client,
        database: &str,
        staging_collection: &str,
        error: Error,
    ) -> Error {
        match self.discard_staging_collection(client, database, staging_collection) {
            Ok(()) => error,
            Err(cleanup_error) => Error::Parse(format!(
                "{error}; also failed to remove staging collection: {cleanup_error}"
            )),
        }
    }
}

fn record_import_batch(
    result: Result<u64>,
    stop_on_error: bool,
    processed: &mut u64,
    failures: &mut Vec<Error>,
    progress: Option<&crate::connection::types::ProgressCallback>,
) -> Result<()> {
    match result {
        Ok(count) => {
            *processed += count;
            if let Some(progress) = progress {
                progress(*processed);
            }
            Ok(())
        }
        Err(error) if stop_on_error => Err(error.with_processed(*processed)),
        Err(error) => {
            *processed += error.processed_count();
            if let Some(progress) = progress {
                progress(*processed);
            }
            log::warn!("Import batch error (continuing): {error}");
            failures.push(error);
            Ok(())
        }
    }
}

fn finish_import(processed: u64, failures: Vec<Error>) -> Result<u64> {
    if failures.is_empty() { Ok(processed) } else { Err(Error::continued(processed, failures)) }
}

// Import mode helper functions

/// Helper to dispatch batch import by mode.
pub(crate) async fn import_batch_by_mode(
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

pub(crate) async fn import_batch_insert(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
    use mongodb::options::InsertManyOptions;

    if batch.is_empty() {
        return Ok(0);
    }

    let options = InsertManyOptions::builder().ordered(ordered).build();
    match coll.insert_many(batch.to_vec()).with_options(options).await {
        Ok(_) => Ok(batch.len() as u64),
        Err(error) => {
            let processed = insert_many_processed_count(&error, batch.len(), ordered);
            Err(Error::from(error).with_processed(processed))
        }
    }
}

fn insert_many_processed_count(
    error: &mongodb::error::Error,
    batch_size: usize,
    ordered: bool,
) -> u64 {
    let mongodb::error::ErrorKind::InsertMany(details) = &*error.kind else {
        return 0;
    };
    if details.write_concern_error.is_some() {
        return 0;
    }

    let Some(write_errors) = details.write_errors.as_ref() else {
        return 0;
    };
    if ordered {
        write_errors.iter().map(|error| error.index).min().unwrap_or(0) as u64
    } else {
        batch_size.saturating_sub(write_errors.len()) as u64
    }
}

/// Upsert documents using update_one with $set.
/// Groups documents by whether they have _id for efficient processing.
/// Unordered mode runs up to 50 concurrent operations for throughput.
pub(crate) async fn import_batch_upsert(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
    use futures::StreamExt;
    use mongodb::options::UpdateOptions;

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
    let mut failures = Vec::new();
    let update_options = UpdateOptions::builder().upsert(true).build();

    if ordered {
        // Ordered: process sequentially, stop on first error
        for doc in with_id {
            let id = doc.get("_id").unwrap();
            let filter = doc! { "_id": id.clone() };
            let mut update_doc = doc.clone();
            update_doc.remove("_id");

            match coll
                .update_one(filter, doc! { "$set": update_doc })
                .with_options(update_options.clone())
                .await
            {
                Ok(_) => count += 1,
                Err(error) => return Err(Error::from(error).with_processed(count)),
            }
        }
    } else {
        // Unordered: run concurrently for throughput (50 in-flight at a time)
        let results: Vec<_> = futures::stream::iter(with_id.into_iter().map(|doc| {
            let coll = coll.clone();
            let opts = update_options.clone();
            async move {
                let id = doc.get("_id").unwrap();
                let filter = doc! { "_id": id.clone() };
                let mut update_doc = doc.clone();
                update_doc.remove("_id");
                coll.update_one(filter, doc! { "$set": update_doc }).with_options(opts).await
            }
        }))
        .buffer_unordered(50)
        .collect()
        .await;

        for result in results {
            match result {
                Ok(_) => count += 1,
                Err(error) => failures.push(Error::from(error)),
            }
        }
    }

    // Insert documents without _id
    if !without_id.is_empty() {
        match import_batch_insert(coll, &without_id, ordered).await {
            Ok(inserted) => count += inserted,
            Err(error) if ordered => return Err(error.with_processed(count)),
            Err(error) => {
                count += error.processed_count();
                failures.push(error);
            }
        }
    }

    if failures.is_empty() { Ok(count) } else { Err(Error::continued(count, failures)) }
}

/// Replace documents atomically one document at a time.
pub(crate) async fn import_batch_replace(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
    use mongodb::options::ReplaceOptions;

    let mut processed = 0u64;
    let mut failures = Vec::new();
    let replace_options = ReplaceOptions::builder().upsert(true).build();

    // ponytail: sequential replacements favor recoverability; batch writes can return when
    // MongoDB < 8.0 support is dropped and the driver exposes mixed replace/insert bulk writes.
    for document in batch {
        let result = if let Some(id) = document.get("_id") {
            coll.replace_one(doc! { "_id": id.clone() }, document.clone())
                .with_options(replace_options.clone())
                .await
                .map(|_| ())
        } else {
            coll.insert_one(document.clone()).await.map(|_| ())
        };

        match result {
            Ok(()) => processed += 1,
            Err(error) if ordered => return Err(Error::from(error).with_processed(processed)),
            Err(error) => failures.push(Error::from(error)),
        }
    }

    if failures.is_empty() { Ok(processed) } else { Err(Error::continued(processed, failures)) }
}
