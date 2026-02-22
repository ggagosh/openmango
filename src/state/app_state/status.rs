//! Status message updates derived from events.

use crate::state::StatusMessage;
use crate::state::app_state::{
    CollectionProgress, CollectionTransferStatus, DatabaseTransferProgress,
};
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
            AppEvent::DocumentSaved { .. } => {
                self.set_status_message(Some(StatusMessage::info("Document saved")));
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
            AppEvent::AggregationCompleted { session, count, preview, limited } => {
                let _ = session;
                let mode = if *preview { "Preview" } else { "Aggregation" };
                let mut message = format!("{mode} returned {count} result(s)");
                if *limited {
                    message.push_str(" (limited)");
                }
                self.set_status_message(Some(StatusMessage::info(message)));
            }
            AppEvent::AggregationFailed { session, error } => {
                let _ = session;
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Aggregation failed: {error}"
                ))));
            }
            AppEvent::ExplainStarted { session, scope } => {
                let _ = session;
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Running {} explain...",
                    scope.label()
                ))));
            }
            AppEvent::ExplainCompleted { session, scope } => {
                let _ = session;
                self.set_status_message(Some(StatusMessage::info(format!(
                    "{} explain completed",
                    scope.label()
                ))));
            }
            AppEvent::ExplainFailed { session, scope, error } => {
                let _ = session;
                self.set_status_message(Some(StatusMessage::error(format!(
                    "{} explain failed: {error}",
                    scope.label()
                ))));
            }
            AppEvent::DatabaseTransferStarted { transfer_id, collections } => {
                // Initialize database progress tracking
                if let Some(tab) = self.transfer_tab_mut(*transfer_id) {
                    tab.runtime.database_progress = Some(DatabaseTransferProgress {
                        collections: collections
                            .iter()
                            .map(|name| CollectionProgress {
                                name: name.clone(),
                                status: CollectionTransferStatus::Pending,
                                documents_processed: 0,
                                documents_total: None,
                            })
                            .collect(),
                        panel_expanded: true,
                    });
                }
            }
            AppEvent::CollectionProgressUpdate {
                transfer_id,
                collection_name,
                status,
                documents_processed,
                documents_total,
            } => {
                // Update collection progress (or add if not exists for BSON exports)
                if let Some(tab) = self.transfer_tab_mut(*transfer_id) {
                    // Initialize database_progress if not set (for BSON exports that start empty)
                    if tab.runtime.database_progress.is_none() {
                        tab.runtime.database_progress = Some(DatabaseTransferProgress {
                            collections: vec![],
                            panel_expanded: true,
                        });
                    }

                    if let Some(ref mut db_progress) = tab.runtime.database_progress {
                        // Find existing collection or add new one
                        if let Some(coll) =
                            db_progress.collections.iter_mut().find(|c| c.name == *collection_name)
                        {
                            coll.status = status.clone();
                            coll.documents_processed = *documents_processed;
                            coll.documents_total = *documents_total;
                        } else {
                            // Collection not in list yet - add it (happens with BSON exports)
                            db_progress.collections.push(CollectionProgress {
                                name: collection_name.clone(),
                                status: status.clone(),
                                documents_processed: *documents_processed,
                                documents_total: *documents_total,
                            });
                        }
                    }
                }
            }
            AppEvent::SchemaAnalyzed { .. } => {
                self.set_status_message(Some(StatusMessage::info("Schema analysis complete")));
            }
            AppEvent::SchemaFailed { error, .. } => {
                self.set_status_message(Some(StatusMessage::error(format!(
                    "Schema analysis failed: {error}"
                ))));
            }
            AppEvent::UpdateAvailable { version } => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Update available: v{version}"
                ))));
            }
            _ => {}
        }
    }
}
