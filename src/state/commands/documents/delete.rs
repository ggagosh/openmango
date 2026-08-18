use gpui::{App, AppContext as _, Entity};

use crate::bson::DocumentKey;
use crate::state::{AppCommands, AppEvent, AppState, SessionKey};

impl AppCommands {
    /// Delete a document by _id in MongoDB.
    pub fn delete_document(
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let (
            database,
            collection,
            original,
            original_id,
            history_enabled,
            operation_engine,
            operation_backend,
            connection_name,
        ) = {
            let state_ref = state.read(cx);
            let Some(original) = state_ref.document_for_key(&session_key, &doc_key) else {
                return;
            };
            let Some(id) = original.get("_id") else {
                return;
            };

            (
                session_key.database.clone(),
                session_key.collection.clone(),
                original.clone(),
                id.clone(),
                state_ref.connection_reversible_history(session_key.connection_id),
                state_ref.operation_engine(),
                state_ref.operation_backend(),
                state_ref
                    .connection_name(session_key.connection_id)
                    .unwrap_or_else(|| "Connection".to_string()),
            )
        };
        let history_engine =
            match crate::operations::tracked_engine(history_enabled, operation_engine) {
                Ok(engine) => engine,
                Err(error) => {
                    state.update(cx, |state, cx| {
                        let event = AppEvent::DocumentDeleteFailed {
                            session: session_key.clone(),
                            error: error.user_message().to_string(),
                        };
                        state.update_status_from_event(&event);
                        cx.emit(event);
                        cx.notify();
                    });
                    return;
                }
            };
        let tracked_delete = history_enabled;
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                if let Some(engine) = history_engine {
                    operation_backend.register_client(session_key.connection_id, client);
                    engine
                        .execute(
                            crate::operations::OperationContext::user(),
                            crate::operations::Mutation::DeleteDocument {
                                target: crate::operations::DocumentTarget {
                                    connection_id: session_key.connection_id,
                                    connection_name,
                                    database,
                                    collection,
                                    id: original_id,
                                },
                                editor_precondition: Some(original),
                            },
                        )
                        .map(|_| ())
                        .map_err(|error| {
                            crate::error::Error::Parse(error.user_message().to_string())
                        })
                } else {
                    manager.delete_document(&client, &database, &collection, &original_id)
                }
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            let doc_key = doc_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                if let Some(index) =
                                    session.data.index_by_key.get(&doc_key).copied()
                                {
                                    session.data.items.remove(index);
                                    session.data.index_by_key = session
                                        .data
                                        .items
                                        .iter()
                                        .enumerate()
                                        .map(|(idx, item)| (item.key.clone(), idx))
                                        .collect();
                                    session.data.total = session.data.total.saturating_sub(1);
                                }
                                session.view.drafts.remove(&doc_key);
                                session.view.dirty.remove(&doc_key);
                                session.view.selected_docs.remove(&doc_key);
                                if session.view.selected_doc.as_ref() == Some(&doc_key) {
                                    session.view.selected_doc = None;
                                    session.view.selected_node_id = None;
                                }
                                session.generation = session.generation.wrapping_add(1);
                            }

                            let event = AppEvent::DocumentDeleted {
                                session: session_key.clone(),
                                document: doc_key.clone(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        if tracked_delete {
                            AppCommands::collection_history_changed(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        log::error!("Failed to delete document");
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentDeleteFailed {
                                session: session_key.clone(),
                                error: error.to_string(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    // Bulk delete moved to documents/bulk.rs.
}
