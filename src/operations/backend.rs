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
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct InMemoryMutationBackend {
    documents: Mutex<HashMap<Vec<u8>, Document>>,
}

#[cfg(test)]
impl InMemoryMutationBackend {
    pub(crate) fn set_document(&self, target: &DocumentTarget, document: Document) {
        self.documents.lock().unwrap().insert(target_key(target), document);
    }

    pub(crate) fn document(&self, target: &DocumentTarget) -> Option<Document> {
        self.documents.lock().unwrap().get(&target_key(target)).cloned()
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
