use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::TryStreamExt as _;
use mongodb::Client;
use mongodb::bson::{Binary, Bson, Document, doc, spec::BinarySubtype};
use mongodb::results::CollectionType;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::model::DocumentTarget;
use crate::connection::ConnectionManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum BackendError {
    #[error("target is unavailable")]
    Unavailable,
    #[error("document changed on the server")]
    Conflict,
    #[error("document mutation failed")]
    Failed,
    #[error("target is not supported by collection snapshots")]
    Unsupported,
    #[error("snapshot recovery requires manual attention")]
    RecoveryRequired,
}

pub(crate) trait MutationBackend: Send + Sync {
    fn current_document(&self, target: &DocumentTarget) -> Result<Option<Document>, BackendError>;

    fn replace_document_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
        replacement: &Document,
    ) -> Result<(), BackendError>;

    fn delete_document_if_current(
        &self,
        _target: &DocumentTarget,
        _expected: &Document,
    ) -> Result<(), BackendError> {
        Err(BackendError::Failed)
    }

    fn insert_document_if_absent(
        &self,
        _target: &DocumentTarget,
        _document: &Document,
    ) -> Result<(), BackendError> {
        Err(BackendError::Failed)
    }

    fn current_index(&self, _target: &DocumentTarget) -> Result<Option<Document>, BackendError> {
        Err(BackendError::Failed)
    }

    fn create_index_if_absent(
        &self,
        _target: &DocumentTarget,
        _definition: &Document,
    ) -> Result<(), BackendError> {
        Err(BackendError::Failed)
    }

    fn drop_index_if_current(
        &self,
        _target: &DocumentTarget,
        _expected: &Document,
    ) -> Result<(), BackendError> {
        Err(BackendError::Failed)
    }

    fn capture_collection_snapshot(
        &self,
        _target: &DocumentTarget,
        _path: &Path,
    ) -> Result<Document, BackendError> {
        Err(BackendError::Unsupported)
    }

    fn verify_collection_snapshot(
        &self,
        _target: &DocumentTarget,
        _path: &Path,
        _expected: &Document,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }

    fn current_collection(
        &self,
        _target: &DocumentTarget,
    ) -> Result<Option<Document>, BackendError> {
        Err(BackendError::Unsupported)
    }

    fn drop_collection_if_current(
        &self,
        _target: &DocumentTarget,
        _expected: &Document,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }

    fn restore_collection_if_absent(
        &self,
        _target: &DocumentTarget,
        _archive: &Path,
        _expected: &Document,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }

    fn reconcile_collection(
        &self,
        target: &DocumentTarget,
        _before: Option<&Document>,
        _after: Option<&Document>,
    ) -> Result<Option<Document>, BackendError> {
        self.current_collection(target)
    }
}

pub(crate) struct MongoMutationBackend {
    manager: Arc<ConnectionManager>,
    clients: Mutex<HashMap<Uuid, Client>>,
    tool_uris: Mutex<HashMap<Uuid, String>>,
}

impl MongoMutationBackend {
    pub(crate) fn new(manager: Arc<ConnectionManager>) -> Self {
        Self { manager, clients: Mutex::new(HashMap::new()), tool_uris: Mutex::new(HashMap::new()) }
    }

    pub(crate) fn register_client(&self, connection_id: Uuid, client: Client) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.insert(connection_id, client);
        }
    }

    pub(crate) fn register_snapshot_connection(
        &self,
        connection_id: Uuid,
        client: Client,
        tool_uri: String,
    ) {
        self.register_client(connection_id, client);
        if let Ok(mut tool_uris) = self.tool_uris.lock() {
            tool_uris.insert(connection_id, tool_uri);
        }
    }

    fn client(&self, connection_id: Uuid) -> Result<Client, BackendError> {
        self.clients
            .lock()
            .map_err(|_| BackendError::Unavailable)?
            .get(&connection_id)
            .cloned()
            .ok_or(BackendError::Unavailable)
    }

    fn tool_uri(&self, connection_id: Uuid) -> Result<String, BackendError> {
        self.tool_uris
            .lock()
            .map_err(|_| BackendError::Unavailable)?
            .get(&connection_id)
            .cloned()
            .ok_or(BackendError::Unavailable)
    }
}

impl MutationBackend for MongoMutationBackend {
    fn current_document(&self, target: &DocumentTarget) -> Result<Option<Document>, BackendError> {
        let client = self.client(target.connection_id)?;
        self.manager
            .find_document_by_id(&client, &target.database, &target.collection, &target.id)
            .map_err(|_| BackendError::Unavailable)
    }

    fn replace_document_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
        replacement: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        match self.manager.replace_document_if_current_matches(
            &client,
            &target.database,
            &target.collection,
            &target.id,
            expected,
            replacement.clone(),
        ) {
            Ok(true) => Ok(()),
            Ok(false) => Err(BackendError::Conflict),
            Err(_) => Err(BackendError::Failed),
        }
    }

    fn delete_document_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        match self.manager.delete_document_if_current_matches(
            &client,
            &target.database,
            &target.collection,
            &target.id,
            expected,
        ) {
            Ok(true) => Ok(()),
            Ok(false) => Err(BackendError::Conflict),
            Err(_) => Err(BackendError::Failed),
        }
    }

    fn insert_document_if_absent(
        &self,
        target: &DocumentTarget,
        document: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        match self.manager.insert_document_if_absent_matches(
            &client,
            &target.database,
            &target.collection,
            document.clone(),
        ) {
            Ok(true) => Ok(()),
            Ok(false) => Err(BackendError::Conflict),
            Err(_) => Err(BackendError::Failed),
        }
    }

    fn current_index(&self, target: &DocumentTarget) -> Result<Option<Document>, BackendError> {
        let client = self.client(target.connection_id)?;
        let name = target.id.as_str().ok_or(BackendError::Failed)?;
        self.manager
            .find_index_document(&client, &target.database, &target.collection, name)
            .map_err(|_| BackendError::Unavailable)
    }

    fn create_index_if_absent(
        &self,
        target: &DocumentTarget,
        definition: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        match self.manager.create_index_if_absent_matches(
            &client,
            &target.database,
            &target.collection,
            definition.clone(),
        ) {
            Ok(true) => Ok(()),
            Ok(false) => Err(BackendError::Conflict),
            Err(_) => Err(BackendError::Failed),
        }
    }

    fn drop_index_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        let name = target.id.as_str().ok_or(BackendError::Failed)?;
        match self.manager.drop_index_if_current_matches(
            &client,
            &target.database,
            &target.collection,
            name,
            expected,
        ) {
            Ok(true) => Ok(()),
            Ok(false) => Err(BackendError::Conflict),
            Err(_) => Err(BackendError::Failed),
        }
    }

    fn capture_collection_snapshot(
        &self,
        target: &DocumentTarget,
        path: &Path,
    ) -> Result<Document, BackendError> {
        let client = self.client(target.connection_id)?;
        let tool_uri = self.tool_uri(target.connection_id)?;
        let before =
            collection_manifest(&self.manager, &client, target)?.ok_or(BackendError::Conflict)?;
        self.manager
            .export_collection_archive(&tool_uri, &target.database, &target.collection, path)
            .map_err(|_| BackendError::Failed)?;
        self.manager
            .verify_collection_archive(&tool_uri, &target.database, &target.collection, path)
            .map_err(|_| BackendError::Failed)?;
        match collection_manifest(&self.manager, &client, target) {
            Ok(Some(current)) if current == before => Ok(before),
            Ok(_) | Err(BackendError::Unsupported | BackendError::Conflict) => {
                Err(BackendError::Conflict)
            }
            Err(error) => Err(error),
        }
    }

    fn verify_collection_snapshot(
        &self,
        target: &DocumentTarget,
        path: &Path,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        let tool_uri = self.tool_uri(target.connection_id)?;
        let staging = snapshot_collection_name("verify", target)?;
        let staging_target = DocumentTarget { collection: staging.clone(), ..target.clone() };
        if manager_collection_exists(&self.manager, &client, &staging_target)? {
            return Err(BackendError::Conflict);
        }
        if self
            .manager
            .restore_collection_archive_as(
                &tool_uri,
                &target.database,
                &target.collection,
                &target.database,
                &staging,
                path,
            )
            .is_err()
        {
            return Err(if manager_collection_exists(&self.manager, &client, &staging_target)? {
                BackendError::RecoveryRequired
            } else {
                BackendError::Failed
            });
        }
        match collection_manifest(&self.manager, &client, &staging_target) {
            Ok(Some(current)) if current == *expected => cleanup_snapshot_collection_if_matches(
                &self.manager,
                &client,
                &staging_target,
                expected,
            ),
            _ => Err(BackendError::RecoveryRequired),
        }
    }

    fn current_collection(
        &self,
        target: &DocumentTarget,
    ) -> Result<Option<Document>, BackendError> {
        let client = self.client(target.connection_id)?;
        collection_manifest(&self.manager, &client, target)
    }

    fn drop_collection_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        let quarantine = snapshot_collection_name("drop", target)?;
        let quarantine_target = DocumentTarget { collection: quarantine.clone(), ..target.clone() };
        match collection_manifest(&self.manager, &client, &quarantine_target) {
            Ok(Some(_)) | Err(BackendError::Unsupported | BackendError::Conflict) => {
                return Err(BackendError::Conflict);
            }
            Err(error) => return Err(error),
            Ok(None) => {}
        }
        if collection_manifest(&self.manager, &client, target)?.as_ref() != Some(expected) {
            return Err(BackendError::Conflict);
        }
        if self
            .manager
            .rename_collection(&client, &target.database, &target.collection, &quarantine)
            .is_err()
        {
            return match collection_manifest(&self.manager, &client, target) {
                Ok(Some(current)) if current != *expected => Err(BackendError::Conflict),
                Err(BackendError::Unsupported) => Err(BackendError::Conflict),
                _ => Err(BackendError::Failed),
            };
        }
        if collection_manifest(&self.manager, &client, &quarantine_target)?.as_ref()
            != Some(expected)
        {
            return rollback_quarantined_drop(&self.manager, &client, target, &quarantine_target);
        }
        finish_quarantined_drop(&self.manager, &client, target, &quarantine_target, expected)
    }

    fn restore_collection_if_absent(
        &self,
        target: &DocumentTarget,
        archive: &Path,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let client = self.client(target.connection_id)?;
        let tool_uri = self.tool_uri(target.connection_id)?;
        match collection_manifest(&self.manager, &client, target) {
            Ok(None) => {}
            Ok(Some(_)) | Err(BackendError::Unsupported | BackendError::Conflict) => {
                return Err(BackendError::Conflict);
            }
            Err(error) => return Err(error),
        }
        let staging = snapshot_collection_name("restore", target)?;
        let staging_target = DocumentTarget { collection: staging.clone(), ..target.clone() };
        match collection_manifest(&self.manager, &client, &staging_target) {
            Ok(Some(_)) | Err(BackendError::Unsupported | BackendError::Conflict) => {
                return Err(BackendError::Conflict);
            }
            Err(error) => return Err(error),
            Ok(None) => {
                if self
                    .manager
                    .restore_collection_archive_as(
                        &tool_uri,
                        &target.database,
                        &target.collection,
                        &target.database,
                        &staging,
                        archive,
                    )
                    .is_err()
                {
                    return Err(
                        if manager_collection_exists(&self.manager, &client, &staging_target)? {
                            BackendError::RecoveryRequired
                        } else {
                            BackendError::Failed
                        },
                    );
                }
            }
        }
        let staged = collection_manifest(&self.manager, &client, &staging_target);
        if !matches!(staged, Ok(Some(ref current)) if current == expected) {
            return Err(BackendError::RecoveryRequired);
        }
        match collection_manifest(&self.manager, &client, target) {
            Ok(None) => {}
            Ok(Some(_)) | Err(BackendError::Unsupported | BackendError::Conflict) => {
                cleanup_snapshot_collection_if_matches(
                    &self.manager,
                    &client,
                    &staging_target,
                    expected,
                )?;
                return Err(BackendError::Conflict);
            }
            Err(error) => {
                cleanup_snapshot_collection_if_matches(
                    &self.manager,
                    &client,
                    &staging_target,
                    expected,
                )?;
                return Err(error);
            }
        }
        if self
            .manager
            .rename_collection(&client, &target.database, &staging, &target.collection)
            .is_err()
        {
            let conflict = !matches!(collection_manifest(&self.manager, &client, target), Ok(None));
            cleanup_snapshot_collection_if_matches(
                &self.manager,
                &client,
                &staging_target,
                expected,
            )?;
            return Err(if conflict { BackendError::Conflict } else { BackendError::Failed });
        }
        if collection_manifest(&self.manager, &client, target)?.as_ref() != Some(expected) {
            return Err(BackendError::Failed);
        }
        Ok(())
    }

    fn reconcile_collection(
        &self,
        target: &DocumentTarget,
        before: Option<&Document>,
        after: Option<&Document>,
    ) -> Result<Option<Document>, BackendError> {
        let client = self.client(target.connection_id)?;
        let verification = snapshot_collection_name("verify", target)?;
        let verification_target = DocumentTarget { collection: verification, ..target.clone() };
        if let Some(expected) = before {
            reconcile_snapshot_cleanup(&self.manager, &client, &verification_target, expected)?;
        }
        if manager_collection_exists(&self.manager, &client, &verification_target)? {
            let expected = before.ok_or(BackendError::RecoveryRequired)?;
            cleanup_snapshot_collection_if_matches(
                &self.manager,
                &client,
                &verification_target,
                expected,
            )?;
        }
        match (before, after) {
            (Some(expected), None) => {
                let quarantine = snapshot_collection_name("drop", target)?;
                let quarantine_target = DocumentTarget { collection: quarantine, ..target.clone() };
                reconcile_snapshot_cleanup(&self.manager, &client, &quarantine_target, expected)?;
                match collection_manifest(&self.manager, &client, &quarantine_target) {
                    Ok(Some(current)) if current == *expected => {
                        cleanup_snapshot_collection_if_matches(
                            &self.manager,
                            &client,
                            &quarantine_target,
                            expected,
                        )?;
                    }
                    Ok(Some(current)) => return Ok(Some(current)),
                    Err(BackendError::Unsupported | BackendError::Conflict) => {
                        return Ok(Some(doc! { "_openmango_recovery_conflict": true }));
                    }
                    Err(error) => return Err(error),
                    Ok(None) => {}
                }
            }
            (None, Some(expected)) => {
                let staging = snapshot_collection_name("restore", target)?;
                let staging_target = DocumentTarget { collection: staging, ..target.clone() };
                reconcile_snapshot_cleanup(&self.manager, &client, &staging_target, expected)?;
                match collection_manifest(&self.manager, &client, &staging_target) {
                    Ok(Some(current)) if current == *expected => {
                        if collection_manifest(&self.manager, &client, target)?.is_none() {
                            if self
                                .manager
                                .rename_collection(
                                    &client,
                                    &target.database,
                                    &staging_target.collection,
                                    &target.collection,
                                )
                                .is_err()
                            {
                                cleanup_snapshot_collection_if_matches(
                                    &self.manager,
                                    &client,
                                    &staging_target,
                                    expected,
                                )?;
                            }
                        } else {
                            cleanup_snapshot_collection_if_matches(
                                &self.manager,
                                &client,
                                &staging_target,
                                expected,
                            )?;
                        }
                    }
                    Ok(Some(_)) | Err(BackendError::Unsupported | BackendError::Conflict) => {
                        return Err(BackendError::RecoveryRequired);
                    }
                    Err(error) => return Err(error),
                    Ok(None) => {}
                }
            }
            _ => {}
        }
        collection_manifest(&self.manager, &client, target)
    }
}

fn snapshot_collection_name(
    purpose: &str,
    target: &DocumentTarget,
) -> Result<String, BackendError> {
    let operation_id = target
        .id
        .as_str()
        .ok_or(BackendError::Failed)?
        .parse::<Uuid>()
        .map_err(|_| BackendError::Failed)?;
    Ok(format!("_openmango_{purpose}_{}", operation_id.simple()))
}

fn finish_quarantined_drop(
    manager: &ConnectionManager,
    client: &Client,
    target: &DocumentTarget,
    quarantine: &DocumentTarget,
    expected: &Document,
) -> Result<(), BackendError> {
    if let Err(error) =
        cleanup_snapshot_collection_if_matches(manager, client, quarantine, expected)
    {
        return if error == BackendError::Failed {
            rollback_quarantined_drop(manager, client, target, quarantine)
        } else {
            Err(error)
        };
    }
    match collection_manifest(manager, client, target) {
        Ok(None) => Ok(()),
        Ok(Some(_)) | Err(BackendError::Unsupported | BackendError::Conflict) => {
            Err(BackendError::Failed)
        }
        Err(error) => Err(error),
    }
}

fn rollback_quarantined_drop(
    manager: &ConnectionManager,
    client: &Client,
    target: &DocumentTarget,
    quarantine: &DocumentTarget,
) -> Result<(), BackendError> {
    if !matches!(collection_manifest(manager, client, target), Ok(None)) {
        return Err(BackendError::Failed);
    }
    manager
        .rename_collection(client, &target.database, &quarantine.collection, &target.collection)
        .map_err(|_| BackendError::Failed)?;
    Err(BackendError::Conflict)
}

fn manager_collection_exists(
    manager: &ConnectionManager,
    client: &Client,
    target: &DocumentTarget,
) -> Result<bool, BackendError> {
    Ok(manager
        .list_collections(client, &target.database)
        .map_err(|_| BackendError::Unavailable)?
        .iter()
        .any(|name| name == &target.collection))
}

fn cleanup_snapshot_collection_if_matches(
    manager: &ConnectionManager,
    client: &Client,
    target: &DocumentTarget,
    expected: &Document,
) -> Result<(), BackendError> {
    match collection_manifest(manager, client, target) {
        Ok(None) => return Ok(()),
        Ok(Some(current)) if current == *expected => {}
        Ok(Some(_)) | Err(BackendError::Unsupported | BackendError::Conflict) => {
            return Err(BackendError::RecoveryRequired);
        }
        Err(error) => return Err(error),
    }
    let cleanup_target =
        DocumentTarget { collection: format!("{}_cleanup", target.collection), ..target.clone() };
    if manager_collection_exists(manager, client, &cleanup_target)? {
        return Err(BackendError::RecoveryRequired);
    }
    manager
        .rename_collection(client, &target.database, &target.collection, &cleanup_target.collection)
        .map_err(|_| BackendError::RecoveryRequired)?;
    if collection_manifest(manager, client, &cleanup_target)?.as_ref() != Some(expected) {
        let _ = rollback_snapshot_cleanup(manager, client, target, &cleanup_target);
        return Err(BackendError::RecoveryRequired);
    }
    let _ = manager.drop_collection(client, &target.database, &cleanup_target.collection);
    if manager_collection_exists(manager, client, &cleanup_target)? {
        let _ = rollback_snapshot_cleanup(manager, client, target, &cleanup_target);
        return Err(BackendError::RecoveryRequired);
    }
    Ok(())
}

fn rollback_snapshot_cleanup(
    manager: &ConnectionManager,
    client: &Client,
    target: &DocumentTarget,
    cleanup_target: &DocumentTarget,
) -> Result<(), BackendError> {
    if manager_collection_exists(manager, client, target)? {
        return Err(BackendError::RecoveryRequired);
    }
    manager
        .rename_collection(client, &target.database, &cleanup_target.collection, &target.collection)
        .map_err(|_| BackendError::RecoveryRequired)
}

fn reconcile_snapshot_cleanup(
    manager: &ConnectionManager,
    client: &Client,
    target: &DocumentTarget,
    expected: &Document,
) -> Result<(), BackendError> {
    let cleanup_target =
        DocumentTarget { collection: format!("{}_cleanup", target.collection), ..target.clone() };
    if !manager_collection_exists(manager, client, &cleanup_target)? {
        return Ok(());
    }
    if manager_collection_exists(manager, client, target)?
        || collection_manifest(manager, client, &cleanup_target)?.as_ref() != Some(expected)
    {
        return Err(BackendError::RecoveryRequired);
    }
    let _ = manager.drop_collection(client, &cleanup_target.database, &cleanup_target.collection);
    if manager_collection_exists(manager, client, &cleanup_target)? {
        Err(BackendError::RecoveryRequired)
    } else {
        Ok(())
    }
}

fn collection_manifest(
    manager: &ConnectionManager,
    client: &Client,
    target: &DocumentTarget,
) -> Result<Option<Document>, BackendError> {
    let Some(specification) = manager
        .list_collection_specs(client, &target.database)
        .map_err(|_| BackendError::Unavailable)?
        .into_iter()
        .find(|specification| specification.name == target.collection)
    else {
        return Ok(None);
    };
    if !matches!(specification.collection_type, CollectionType::Collection)
        || specification.options.timeseries.is_some()
    {
        return Err(BackendError::Unsupported);
    }
    let stats = manager
        .runtime_handle()
        .block_on(async {
            client
                .database(&target.database)
                .run_command(doc! { "collStats": &target.collection })
                .await
        })
        .map_err(|_| BackendError::Unavailable)?;
    if stats.get_bool("sharded").unwrap_or(false) {
        return Err(BackendError::Unsupported);
    }

    let options =
        mongodb::bson::to_document(&specification.options).map_err(|_| BackendError::Failed)?;
    let mut indexes = manager
        .list_indexes(client, &target.database, &target.collection)
        .map_err(|_| BackendError::Unavailable)?
        .iter()
        .map(crate::connection::ops::indexes::canonical_index_document)
        .collect::<crate::error::Result<Vec<_>>>()
        .map_err(|_| BackendError::Failed)?;
    indexes.sort_by(|left, right| {
        left.get_str("name").unwrap_or_default().cmp(right.get_str("name").unwrap_or_default())
    });

    let (document_count, document_hash) = manager
        .runtime_handle()
        .block_on(async {
            let collection =
                client.database(&target.database).collection::<Document>(&target.collection);
            let mut cursor = collection.find(doc! {}).sort(doc! { "_id": 1 }).await?;
            let mut count = 0i64;
            let mut digest = Sha256::new();
            while let Some(document) = cursor.try_next().await? {
                let encoded = mongodb::bson::to_vec(&document)?;
                digest.update((encoded.len() as u64).to_be_bytes());
                digest.update(encoded);
                count = count.checked_add(1).ok_or_else(|| {
                    mongodb::error::Error::custom("collection document count overflow")
                })?;
            }
            Ok::<_, mongodb::error::Error>((count, digest.finalize().to_vec()))
        })
        .map_err(|_| BackendError::Unavailable)?;

    Ok(Some(doc! {
        "format_version": 1,
        "options": options,
        "indexes": indexes.into_iter().map(Bson::Document).collect::<Vec<_>>(),
        "document_count": document_count,
        "documents_sha256": Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: document_hash,
        }),
    }))
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct InMemoryMutationBackend {
    documents: Mutex<HashMap<Vec<u8>, Document>>,
    indexes: Mutex<HashMap<Vec<u8>, Document>>,
    collections: Mutex<HashMap<Vec<u8>, Document>>,
}

#[cfg(test)]
impl InMemoryMutationBackend {
    pub(crate) fn set_document(&self, target: &DocumentTarget, document: Document) {
        self.documents.lock().unwrap().insert(target_key(target), document);
    }

    pub(crate) fn document(&self, target: &DocumentTarget) -> Option<Document> {
        self.documents.lock().unwrap().get(&target_key(target)).cloned()
    }

    pub(crate) fn set_index(&self, target: &DocumentTarget, definition: Document) {
        self.indexes.lock().unwrap().insert(target_key(target), definition);
    }

    pub(crate) fn index(&self, target: &DocumentTarget) -> Option<Document> {
        self.indexes.lock().unwrap().get(&target_key(target)).cloned()
    }

    pub(crate) fn set_collection(&self, target: &DocumentTarget, manifest: Document) {
        self.collections.lock().unwrap().insert(collection_key(target), manifest);
    }

    pub(crate) fn collection(&self, target: &DocumentTarget) -> Option<Document> {
        self.collections.lock().unwrap().get(&collection_key(target)).cloned()
    }
}

#[cfg(test)]
impl MutationBackend for InMemoryMutationBackend {
    fn current_document(&self, target: &DocumentTarget) -> Result<Option<Document>, BackendError> {
        Ok(self.document(target))
    }

    fn replace_document_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
        replacement: &Document,
    ) -> Result<(), BackendError> {
        let mut documents = self.documents.lock().map_err(|_| BackendError::Failed)?;
        let Some(current) = documents.get(&target_key(target)) else {
            return Err(BackendError::Conflict);
        };
        if current != expected {
            return Err(BackendError::Conflict);
        }
        documents.insert(target_key(target), replacement.clone());
        Ok(())
    }

    fn delete_document_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let mut documents = self.documents.lock().map_err(|_| BackendError::Failed)?;
        let key = target_key(target);
        if documents.get(&key) != Some(expected) {
            return Err(BackendError::Conflict);
        }
        documents.remove(&key);
        Ok(())
    }

    fn insert_document_if_absent(
        &self,
        target: &DocumentTarget,
        document: &Document,
    ) -> Result<(), BackendError> {
        let mut documents = self.documents.lock().map_err(|_| BackendError::Failed)?;
        let key = target_key(target);
        if documents.contains_key(&key) {
            return Err(BackendError::Conflict);
        }
        documents.insert(key, document.clone());
        Ok(())
    }

    fn current_index(&self, target: &DocumentTarget) -> Result<Option<Document>, BackendError> {
        Ok(self.index(target))
    }

    fn create_index_if_absent(
        &self,
        target: &DocumentTarget,
        definition: &Document,
    ) -> Result<(), BackendError> {
        let mut indexes = self.indexes.lock().map_err(|_| BackendError::Failed)?;
        let key = target_key(target);
        if indexes.contains_key(&key) {
            return Err(BackendError::Conflict);
        }
        indexes.insert(key, definition.clone());
        Ok(())
    }

    fn drop_index_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let mut indexes = self.indexes.lock().map_err(|_| BackendError::Failed)?;
        let key = target_key(target);
        if indexes.get(&key) != Some(expected) {
            return Err(BackendError::Conflict);
        }
        indexes.remove(&key);
        Ok(())
    }

    fn capture_collection_snapshot(
        &self,
        target: &DocumentTarget,
        path: &Path,
    ) -> Result<Document, BackendError> {
        let manifest = self.collection(target).ok_or(BackendError::Conflict)?;
        std::fs::write(path, b"verified collection snapshot").map_err(|_| BackendError::Failed)?;
        Ok(manifest)
    }

    fn verify_collection_snapshot(
        &self,
        _target: &DocumentTarget,
        path: &Path,
        _expected: &Document,
    ) -> Result<(), BackendError> {
        if std::fs::metadata(path).map_err(|_| BackendError::Failed)?.len() == 0 {
            Err(BackendError::Failed)
        } else {
            Ok(())
        }
    }

    fn current_collection(
        &self,
        target: &DocumentTarget,
    ) -> Result<Option<Document>, BackendError> {
        Ok(self.collection(target))
    }

    fn drop_collection_if_current(
        &self,
        target: &DocumentTarget,
        expected: &Document,
    ) -> Result<(), BackendError> {
        let mut collections = self.collections.lock().map_err(|_| BackendError::Failed)?;
        let key = collection_key(target);
        if collections.get(&key) != Some(expected) {
            return Err(BackendError::Conflict);
        }
        collections.remove(&key);
        Ok(())
    }

    fn restore_collection_if_absent(
        &self,
        target: &DocumentTarget,
        archive: &Path,
        expected: &Document,
    ) -> Result<(), BackendError> {
        if std::fs::metadata(archive).map_err(|_| BackendError::Failed)?.len() == 0 {
            return Err(BackendError::Failed);
        }
        let mut collections = self.collections.lock().map_err(|_| BackendError::Failed)?;
        let key = collection_key(target);
        if collections.contains_key(&key) {
            return Err(BackendError::Conflict);
        }
        collections.insert(key, expected.clone());
        Ok(())
    }
}

#[cfg(test)]
fn target_key(target: &DocumentTarget) -> Vec<u8> {
    mongodb::bson::to_vec(&mongodb::bson::doc! {
        "connection_id": target.connection_id.to_string(),
        "database": target.database.clone(),
        "collection": target.collection.clone(),
        "id": target.id.clone(),
    })
    .unwrap()
}

#[cfg(test)]
fn collection_key(target: &DocumentTarget) -> Vec<u8> {
    mongodb::bson::to_vec(&mongodb::bson::doc! {
        "connection_id": target.connection_id.to_string(),
        "database": target.database.clone(),
        "collection": target.collection.clone(),
    })
    .unwrap()
}
