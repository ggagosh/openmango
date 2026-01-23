//! Status message updates derived from events.

use crate::state::StatusMessage;
use crate::state::events::AppEvent;

use super::AppState;

impl AppState {
    pub(crate) fn update_status_from_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::Connecting(_) => {
                self.set_status_message(Some(StatusMessage::info("Connecting...")));
            }
            AppEvent::Connected(_) => {
                self.set_status_message(Some(StatusMessage::info("Connected")));
            }
            AppEvent::Disconnected(_) => {
                self.set_status_message(Some(StatusMessage::info("Disconnected")));
            }
            AppEvent::ConnectionFailed(error) => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Connection failed: {error}"
                ))));
            }
            AppEvent::ConnectionUpdated => {
                self.set_status_message(Some(StatusMessage::info("Connection updated")));
            }
            AppEvent::ConnectionRemoved => {
                self.set_status_message(Some(StatusMessage::info("Connection removed")));
            }
            AppEvent::DatabasesLoaded(databases) => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Loaded {} databases",
                    databases.len()
                ))));
            }
            AppEvent::CollectionsLoaded(collections) => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Loaded {} collections",
                    collections.len()
                ))));
            }
            AppEvent::CollectionsFailed(error) => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Collections failed: {error}"
                ))));
            }
            AppEvent::DocumentsLoaded { total, .. } => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Loaded {total} documents"
                ))));
            }
            AppEvent::DocumentInserted => {
                self.set_status_message(Some(StatusMessage::info("Document inserted")));
            }
            AppEvent::DocumentInsertFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Insert failed: {error}"
                ))));
            }
            AppEvent::DocumentsInserted { count } => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Inserted {} document(s)",
                    count
                ))));
            }
            AppEvent::DocumentsInsertFailed { count, error } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Failed to insert {} document(s): {}",
                    count, error
                ))));
            }
            AppEvent::DocumentSaveFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Save failed: {error}"
                ))));
            }
            AppEvent::DocumentDeleted { .. } => {
                self.set_status_message(Some(StatusMessage::info("Document deleted")));
            }
            AppEvent::DocumentDeleteFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Delete failed: {error}"
                ))));
            }
            AppEvent::IndexesLoaded { count, .. } => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Loaded {count} indexes"
                ))));
            }
            AppEvent::IndexesLoadFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Indexes failed: {error}"
                ))));
            }
            AppEvent::IndexDropped { name, .. } => {
                self.set_status_message(Some(StatusMessage::info(format!("Index {name} dropped"))));
            }
            AppEvent::IndexDropFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Drop index failed: {error}"
                ))));
            }
            AppEvent::IndexCreated { name, .. } => {
                if let Some(name) = name {
                    self.set_status_message(Some(StatusMessage::info(format!(
                        "Index {name} created"
                    ))));
                } else {
                    self.set_status_message(Some(StatusMessage::info("Index created")));
                }
            }
            AppEvent::IndexCreateFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Create index failed: {error}"
                ))));
            }
            AppEvent::DocumentsUpdated { matched, modified, .. } => {
                let message = if *matched == 0 {
                    "No documents matched the update.".to_string()
                } else if *modified == 0 {
                    format!("Matched {matched} documents; no changes applied.")
                } else {
                    format!("Updated {modified} of {matched} documents")
                };
                self.set_status_message(Some(StatusMessage::info(message)));
            }
            AppEvent::DocumentsUpdateFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Update failed: {error}"
                ))));
            }
            AppEvent::DocumentsDeleted { session, deleted } => {
                let _ = session;
                if *deleted == 0 {
                    self.set_status_message(Some(StatusMessage::info(
                        "No documents matched the delete.".to_string(),
                    )));
                } else {
                    self.set_status_message(Some(StatusMessage::info(format!(
                        "Deleted {deleted} document(s)"
                    ))));
                }
            }
            AppEvent::DocumentsDeleteFailed { session, error } => {
                let _ = session;
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Delete failed: {error}"
                ))));
            }
            _ => {}
        }
    }
}
