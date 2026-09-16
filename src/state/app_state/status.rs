//! Status message updates derived from events.

use crate::error::{ErrorKind, ErrorReport};
use crate::state::StatusMessage;
use crate::state::app_state::ErrorAction;
use crate::state::app_state::{
    CollectionProgress, CollectionTransferStatus, DatabaseTransferProgress,
};
use crate::state::events::AppEvent;

use super::AppState;

impl AppState {
    pub(crate) fn update_status_from_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::Connecting(connection_id) => {
                self.set_connection_failure(*connection_id, None);
                self.set_status_message(Some(StatusMessage::info("Connecting...")));
            }
            AppEvent::Connected(connection_id) => {
                self.set_connection_failure(*connection_id, None);
                self.set_status_message(Some(StatusMessage::info("Connected")));
            }
            AppEvent::Disconnected(_) => {
                self.set_status_message(Some(StatusMessage::info("Disconnected")));
            }
            AppEvent::ConnectionFailed { connection_id, error } => {
                let name = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == *connection_id)
                    .map(|connection| connection.name.clone());
                let title = match name {
                    Some(name) => format!("Couldn't connect to {name}"),
                    None => "Couldn't connect".to_string(),
                };
                let report = ErrorReport::from_message(title, error).kind(ErrorKind::Connection);
                self.set_connection_failure(*connection_id, Some(report.message.clone()));
                self.report_error_with_action(report, ErrorAction::Reconnect(*connection_id));
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
                self.report_error(ErrorReport::from_message("Couldn't load collections", error));
            }
            AppEvent::DocumentsLoaded { total, .. } => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Loaded {total} documents"
                ))));
            }
            AppEvent::DocumentsLoadFailed { error, .. } => {
                // The query panel shows this error.
                self.record_error(ErrorReport::from_message("Couldn't run the query", error));
            }
            AppEvent::DocumentInserted { .. } => {
                self.set_status_message(Some(StatusMessage::info("Document inserted")));
            }
            AppEvent::DocumentInsertFailed { error, editor, .. } => {
                let report = ErrorReport::from_message("Couldn't insert the document", error);
                // A JSON editor window shows its own save and insert errors.
                if editor.is_some() {
                    self.record_error(report);
                } else {
                    self.report_error(report);
                }
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
                let title = format!(
                    "Couldn't insert {count} {}",
                    if *count == 1 { "document" } else { "documents" }
                );
                self.report_error(ErrorReport::from_message(title, error));
            }
            AppEvent::DocumentSaveFailed { error, editor, .. } => {
                let report = ErrorReport::from_message("Couldn't save the document", error);
                if editor.is_some() {
                    self.record_error(report);
                } else {
                    self.report_error(report);
                }
            }
            AppEvent::DocumentDeleted { .. } => {
                self.set_status_message(Some(StatusMessage::info("Document deleted")));
            }
            AppEvent::DocumentDeleteFailed { error, .. } => {
                self.report_error(ErrorReport::from_message("Couldn't delete the document", error));
            }
            AppEvent::IndexesLoaded { count, .. } => {
                self.set_status_message(Some(StatusMessage::info(format!(
                    "Loaded {count} indexes"
                ))));
            }
            AppEvent::IndexesLoadFailed { error, .. } => {
                // The Indexes view shows this error.
                self.record_error(ErrorReport::from_message("Couldn't load indexes", error));
            }
            AppEvent::IndexDropped { name, .. } => {
                self.set_status_message(Some(StatusMessage::info(format!("Index {name} dropped"))));
            }
            AppEvent::IndexDropFailed { error, .. } => {
                self.report_error(ErrorReport::from_message("Couldn't drop the index", error));
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
                // The index dialog shows this error.
                self.record_error(ErrorReport::from_message("Couldn't create the index", error));
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
                // The update dialog shows this error.
                self.record_error(ErrorReport::from_message("Couldn't update documents", error));
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
                self.report_error(ErrorReport::from_message("Couldn't delete documents", error));
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
                // The aggregation results panel shows this error.
                self.record_error(ErrorReport::from_message("Couldn't run the pipeline", error));
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
                // The explain dialog shows this error.
                let title = match scope {
                    crate::state::ExplainScope::Find => "Couldn't explain the query",
                    crate::state::ExplainScope::Aggregation => "Couldn't explain the pipeline",
                };
                self.record_error(ErrorReport::from_message(title, error));
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
                        panel_expanded: false,
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
                            panel_expanded: false,
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
                // The Schema view shows this error.
                self.record_error(ErrorReport::from_message("Couldn't analyze the schema", error));
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
