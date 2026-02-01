//! Collection import operations (JSON, CSV).

use std::fs::File;
use std::io::{BufRead, BufReader, Read as _};
use std::path::Path;

use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::connection::types::{
    CsvImportOptions, FileEncoding, InsertMode, JsonImportOptions, JsonTransferFormat,
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
                                Err(e) => {
                                    log::warn!("Import batch error (continuing): {e}");
                                }
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
                            Err(e) => {
                                log::warn!("Import batch error (continuing): {e}");
                            }
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
                            Err(e) => {
                                log::warn!("Import batch error (continuing): {e}");
                            }
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
                        Err(e) => {
                            log::warn!("Import batch error (continuing): {e}");
                        }
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
                    Err(e) => {
                        log::warn!("Import batch error (continuing): {e}");
                    }
                }
            }

            Ok(processed)
        })
    }
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
    coll.insert_many(batch.to_vec()).with_options(options).await?;
    Ok(batch.len() as u64)
}

/// Upsert documents using concurrent update operations.
/// Groups documents by whether they have _id for efficient processing.
pub(crate) async fn import_batch_upsert(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
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
pub(crate) async fn import_batch_replace(
    coll: &mongodb::Collection<Document>,
    batch: &[Document],
    ordered: bool,
) -> Result<u64> {
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
