use gpui::{App, AppContext as _, Entity};
use mongodb::bson::{Document, oid::ObjectId};

use crate::state::{AppCommands, AppEvent, AppState, EditorSessionId, SessionKey};

impl AppCommands {
    /// Insert a document into a collection.
    pub fn insert_document(
        state: Entity<AppState>,
        session_key: SessionKey,
        document: Document,
        cx: &mut App,
    ) {
        Self::insert_document_internal(state, session_key, document, None, cx);
    }

    pub fn insert_document_for_editor(
        state: Entity<AppState>,
        session_key: SessionKey,
        document: Document,
        editor: EditorSessionId,
        cx: &mut App,
    ) {
        Self::insert_document_internal(state, session_key, document, Some(editor), cx);
    }

    fn insert_document_internal(
        state: Entity<AppState>,
        session_key: SessionKey,
        mut document: Document,
        editor: Option<EditorSessionId>,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let (history_enabled, operation_engine, operation_backend, connection_name, manager) = {
            let state_ref = state.read(cx);
            (
                state_ref.connection_reversible_history(session_key.connection_id),
                state_ref.operation_engine(),
                state_ref.operation_backend(),
                state_ref
                    .connection_name(session_key.connection_id)
                    .unwrap_or_else(|| "Connection".to_string()),
                state_ref.connection_manager(),
            )
        };
        let history_engine =
            match crate::operations::tracked_engine(history_enabled, operation_engine) {
                Ok(engine) => engine,
                Err(error) => {
                    state.update(cx, |state, cx| {
                        let event = AppEvent::DocumentInsertFailed {
                            session: session_key.clone(),
                            editor,
                            error: error.user_message().to_string(),
                        };
                        state.update_status_from_event(&event);
                        cx.emit(event);
                        cx.notify();
                    });
                    return;
                }
            };
        if history_engine.is_some() && !document.contains_key("_id") {
            document.insert("_id", ObjectId::new());
        }
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let tracked_insert = history_enabled;

        let task = cx.background_spawn(async move {
            if let Some(engine) = history_engine {
                let Some(id) = document.get("_id").cloned() else {
                    return Err(crate::error::Error::Parse(
                        "Could not assign an _id for reversible insert.".to_string(),
                    ));
                };
                operation_backend.register_client(session_key.connection_id, client);
                engine
                    .execute(
                        crate::operations::OperationContext::user(),
                        crate::operations::Mutation::InsertDocument {
                            target: crate::operations::DocumentTarget {
                                connection_id: session_key.connection_id,
                                connection_name,
                                database,
                                collection,
                                id,
                            },
                            document,
                        },
                    )
                    .map(|_| ())
                    .map_err(|error| crate::error::Error::Parse(error.user_message().to_string()))
            } else {
                manager.insert_document(&client, &database, &collection, document)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let event =
                                AppEvent::DocumentInserted { session: session_key.clone(), editor };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        AppCommands::load_documents_for_session(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                        if tracked_insert {
                            AppCommands::collection_history_changed(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        log::error!("Failed to insert document");
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentInsertFailed {
                                session: session_key.clone(),
                                editor,
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

    // Bulk insert moved to documents/bulk.rs.
}
