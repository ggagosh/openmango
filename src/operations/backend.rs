use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mongodb::Client;
use mongodb::bson::Document;
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
}

pub(crate) struct MongoMutationBackend {
    manager: Arc<ConnectionManager>,
    clients: Mutex<HashMap<Uuid, Client>>,
}

impl MongoMutationBackend {
    pub(crate) fn new(manager: Arc<ConnectionManager>) -> Self {
        Self { manager, clients: Mutex::new(HashMap::new()) }
    }

    pub(crate) fn register_client(&self, connection_id: Uuid, client: Client) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.insert(connection_id, client);
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
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct InMemoryMutationBackend {
    documents: Mutex<HashMap<Vec<u8>, Document>>,
    indexes: Mutex<HashMap<Vec<u8>, Document>>,
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
