//! Index operations for MongoDB collections.

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::Client;
use mongodb::IndexModel;
use mongodb::bson::{Document, doc};

use crate::connection::ConnectionManager;
use crate::error::{Error, Result};

pub(crate) fn index_model_to_create_document(index: &IndexModel) -> Result<Document> {
    let mut document = mongodb::bson::to_document(index)
        .map_err(|error| Error::Parse(format!("Invalid index metadata: {error}")))?;
    // Returned by listIndexes for clustered indexes, but createIndexes rejects it.
    document.remove("clustered");
    Ok(document)
}

pub(crate) fn canonical_index_document(index: &IndexModel) -> Result<Document> {
    let mut document = index_model_to_create_document(index)?;
    document.remove("v");
    document.remove("ns");
    Ok(document)
}

fn validate_index_document(index: &Document) -> Result<()> {
    let keys = index
        .get_document("key")
        .map_err(|_| Error::Parse("Index specification requires a key document".to_string()))?;
    if keys.is_empty() {
        return Err(Error::Parse("Index key document cannot be empty".to_string()));
    }
    if let Ok(name) = index.get_str("name")
        && name.trim().is_empty()
    {
        return Err(Error::Parse("Index name cannot be empty".to_string()));
    }
    for field in ["partialFilterExpression", "collation", "wildcardProjection", "weights"] {
        if index.contains_key(field) && index.get_document(field).is_err() {
            return Err(Error::Parse(format!("Index option {field} must be a document")));
        }
    }
    Ok(())
}

pub async fn list_indexes_async(
    client: &Client,
    database: &str,
    collection: &str,
    max_time: Duration,
) -> Result<Vec<IndexModel>> {
    let coll = client.database(database).collection::<Document>(collection);
    Ok(coll.list_indexes().max_time(max_time).await?.try_collect().await?)
}

impl ConnectionManager {
    /// List indexes for a collection (runs in Tokio runtime)
    pub fn list_indexes(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
    ) -> Result<Vec<IndexModel>> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(list_indexes_async(
            &client,
            &database,
            &collection,
            Duration::from_secs(30),
        ))
    }

    /// Create an index for a collection (runs in Tokio runtime)
    pub fn create_index(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        index: Document,
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

    pub fn find_index_document(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        name: &str,
    ) -> Result<Option<Document>> {
        self.list_indexes(client, database, collection)?
            .iter()
            .find(|index| {
                index.options.as_ref().and_then(|options| options.name.as_deref()) == Some(name)
            })
            .map(canonical_index_document)
            .transpose()
    }

    pub fn create_index_if_absent_matches(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        index: Document,
    ) -> Result<bool> {
        let name = index
            .get_str("name")
            .map_err(|_| Error::Parse("Tracked index requires a name".to_string()))?
            .to_string();
        if self.find_index_document(client, database, collection, &name)?.is_some() {
            return Ok(false);
        }
        match self.create_index(client, database, collection, index) {
            Ok(()) => Ok(true),
            Err(error)
                if self.find_index_document(client, database, collection, &name)?.is_some() =>
            {
                Err(Error::Parse(format!(
                    "Index creation returned an error, but {name} now exists; the outcome is uncertain: {error}"
                )))
            }
            Err(error) => Err(error),
        }
    }

    pub fn drop_index_if_current_matches(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        name: &str,
        expected: &Document,
    ) -> Result<bool> {
        if self.find_index_document(client, database, collection, name)?.as_ref() != Some(expected)
        {
            return Ok(false);
        }
        // ponytail: MongoDB has no conditional dropIndexes primitive. A concurrent external
        // drop/recreate after this check can be dropped and cannot be distinguished afterward.
        self.drop_index(client, database, collection, name)?;
        Ok(true)
    }

    /// Create multiple indexes in a single command (runs in Tokio runtime)
    pub fn create_indexes(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        indexes: Vec<Document>,
    ) -> Result<()> {
        let client = client.clone();
        let database = database.to_string();
        let collection = collection.to_string();

        self.runtime.block_on(async {
            let db = client.database(&database);
            db.run_command(doc! { "createIndexes": collection, "indexes": indexes }).await?;
            Ok(())
        })
    }

    /// Safely replace an index, retaining or restoring a working index on failure.
    pub fn replace_index(
        &self,
        client: &Client,
        database: &str,
        collection: &str,
        old_name: &str,
        replacement: Document,
    ) -> Result<()> {
        validate_index_document(&replacement)?;
        let new_name = replacement
            .get_str("name")
            .map_err(|_| Error::Parse("Replacement index requires a name".to_string()))?
            .to_string();

        let indexes = self.list_indexes(client, database, collection)?;
        let old_model = indexes
            .iter()
            .find(|index| {
                index.options.as_ref().and_then(|options| options.name.as_deref()) == Some(old_name)
            })
            .ok_or_else(|| Error::Parse(format!("Index {old_name} no longer exists")))?;
        let old_document = index_model_to_create_document(old_model)?;
        let mut comparable_old = old_document.clone();
        let mut comparable_replacement = replacement.clone();
        comparable_old.remove("v");
        comparable_replacement.remove("v");
        if comparable_old == comparable_replacement {
            return Ok(());
        }

        if new_name != old_name {
            self.create_index(client, database, collection, replacement)?;
            if let Err(error) = self.drop_index(client, database, collection, old_name) {
                let cleanup = self.drop_index(client, database, collection, &new_name);
                return Err(Error::Parse(match cleanup {
                    Ok(()) => format!(
                        "Created replacement index but could not drop {old_name}; the replacement was removed: {error}"
                    ),
                    Err(cleanup_error) => format!(
                        "Created replacement index but could not drop {old_name}, and cleanup of {new_name} also failed: {error}; cleanup: {cleanup_error}"
                    ),
                }));
            }
            return Ok(());
        }

        let temporary_name = format!("__openmango_validate_{}", uuid::Uuid::new_v4().simple());
        let mut temporary = replacement.clone();
        temporary.insert("name", temporary_name.clone());

        // Validate keys, options, and unique constraints while the original remains available.
        // If MongoDB disallows the parallel build, fail closed and retain the original.
        self.create_index(client, database, collection, temporary)?;
        if let Err(error) = self.drop_index(client, database, collection, old_name) {
            let _ = self.drop_index(client, database, collection, &temporary_name);
            return Err(error);
        }
        if let Err(error) = self.drop_index(client, database, collection, &temporary_name) {
            let rollback = self.create_index(client, database, collection, old_document.clone());
            return Err(Error::Parse(match rollback {
                Ok(()) => format!(
                    "Replacement validation succeeded but temporary cleanup failed; {old_name} was restored: {error}"
                ),
                Err(rollback_error) => format!(
                    "Replacement validation succeeded but temporary cleanup failed, and restoring {old_name} also failed: {error}; restore: {rollback_error}"
                ),
            }));
        }

        if let Err(error) = self.create_index(client, database, collection, replacement) {
            let rollback = self.create_index(client, database, collection, old_document);
            return Err(Error::Parse(match rollback {
                Ok(()) => format!(
                    "Replacement index creation failed; the original {old_name} index was restored: {error}"
                ),
                Err(rollback_error) => format!(
                    "Replacement index creation failed and restoring {old_name} also failed: {error}; restore: {rollback_error}"
                ),
            }));
        }
        Ok(())
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
            let coll = client.database(&database).collection::<Document>(&collection);
            coll.drop_index(name).await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_index_document_preserves_supported_metadata() {
        let source = doc! {
            "key": { "$**": 1 },
            "name": "searchable",
            "unique": false,
            "sparse": true,
            "hidden": true,
            "partialFilterExpression": { "active": true },
            "collation": { "locale": "en", "strength": 2 },
            "wildcardProjection": { "secret": 0 },
            "weights": { "title": 5 },
            "default_language": "english",
            "language_override": "language",
            "storageEngine": { "wiredTiger": { "configString": "block_compressor=zstd" } },
        };
        let model: IndexModel = mongodb::bson::from_document(source.clone()).unwrap();

        let copied = index_model_to_create_document(&model).unwrap();

        for field in [
            "partialFilterExpression",
            "collation",
            "hidden",
            "wildcardProjection",
            "weights",
            "default_language",
            "language_override",
            "storageEngine",
        ] {
            assert_eq!(copied.get(field), source.get(field), "lost option {field}");
        }
    }

    #[test]
    fn replacement_validation_rejects_invalid_shapes_before_server_work() {
        assert!(validate_index_document(&doc! { "name": "missing_key" }).is_err());
        assert!(validate_index_document(&doc! { "key": {}, "name": "empty" }).is_err());
        assert!(
            validate_index_document(
                &doc! { "key": { "value": 1 }, "partialFilterExpression": "invalid" }
            )
            .is_err()
        );
    }
}
