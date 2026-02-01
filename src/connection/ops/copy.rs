//! Collection and database copy operations.

use mongodb::Client;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::connection::types::CopyOptions;
use crate::error::{Error, Result};

impl ConnectionManager {
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

        // List collections first (blocking)
        let collections = self.runtime.block_on(async {
            let src_db = src_client.database(&src_database);
            src_db.list_collection_names().await
        })?;

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

            // Call copy_collection directly on self (no nested block_on issue since
            // copy_collection_with_options runs its own block_on sequentially)
            let count = self.copy_collection(
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
    }
}
