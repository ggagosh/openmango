//! Status message updates derived from events.

use crate::state::StatusMessage;
use crate::state::events::AppEvent;

use super::AppState;

impl AppState {
    pub(crate) fn update_status_from_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::Connecting(_) => {
                self.status_message = Some(StatusMessage::info("Connecting..."));
            }
            AppEvent::Connected(_) => {
                self.status_message = Some(StatusMessage::info("Connected"));
            }
            AppEvent::Disconnected(_) => {
                self.status_message = Some(StatusMessage::info("Disconnected"));
            }
            AppEvent::ConnectionFailed(error) => {
                self.status_message =
                    Some(StatusMessage::error(format!("Connection failed: {error}")));
            }
            AppEvent::ConnectionUpdated => {
                self.status_message = Some(StatusMessage::info("Connection updated"));
            }
            AppEvent::ConnectionRemoved => {
                self.status_message = Some(StatusMessage::info("Connection removed"));
            }
            AppEvent::DatabasesLoaded(databases) => {
                self.status_message =
                    Some(StatusMessage::info(format!("Loaded {} databases", databases.len())));
            }
            AppEvent::CollectionsLoaded(collections) => {
                self.status_message =
                    Some(StatusMessage::info(format!("Loaded {} collections", collections.len())));
            }
            AppEvent::CollectionsFailed(error) => {
                self.status_message =
                    Some(StatusMessage::error(format!("Collections failed: {error}")));
            }
            AppEvent::DocumentsLoaded { total, .. } => {
                self.status_message =
                    Some(StatusMessage::info(format!("Loaded {total} documents")));
            }
            AppEvent::DocumentInserted => {
                self.status_message = Some(StatusMessage::info("Document inserted"));
            }
            AppEvent::DocumentInsertFailed { error, .. } => {
                self.status_message =
                    Some(StatusMessage::error(format!("Insert failed: {error}")));
            }
            AppEvent::DocumentSaveFailed { error, .. } => {
                self.status_message = Some(StatusMessage::error(format!("Save failed: {error}")));
            }
            AppEvent::DocumentDeleted { .. } => {
                self.status_message = Some(StatusMessage::info("Document deleted"));
            }
            AppEvent::DocumentDeleteFailed { error, .. } => {
                self.status_message = Some(StatusMessage::error(format!("Delete failed: {error}")));
            }
            AppEvent::IndexesLoaded { count, .. } => {
                self.status_message =
                    Some(StatusMessage::info(format!("Loaded {count} indexes")));
            }
            AppEvent::IndexesLoadFailed { error, .. } => {
                self.status_message =
                    Some(StatusMessage::error(format!("Indexes failed: {error}")));
            }
            AppEvent::IndexDropped { name, .. } => {
                self.status_message = Some(StatusMessage::info(format!("Index {name} dropped")));
            }
            AppEvent::IndexDropFailed { error, .. } => {
                self.status_message =
                    Some(StatusMessage::error(format!("Drop index failed: {error}")));
            }
            AppEvent::IndexCreated { name, .. } => {
                if let Some(name) = name {
                    self.status_message =
                        Some(StatusMessage::info(format!("Index {name} created")));
                } else {
                    self.status_message = Some(StatusMessage::info("Index created"));
                }
            }
            AppEvent::IndexCreateFailed { error, .. } => {
                self.status_message =
                    Some(StatusMessage::error(format!("Create index failed: {error}")));
            }
            _ => {}
        }
    }
}
