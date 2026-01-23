use gpui::{App, AppContext as _, Entity};
use mongodb::bson::{Document, doc};

use crate::bson::DocumentKey;
use crate::connection::get_connection_manager;
use crate::state::{AppEvent, AppState, SessionKey, StatusMessage};

use crate::state::AppCommands;

impl AppCommands {
    /// Save a document by replacing it in MongoDB.
    pub fn save_document(
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        updated: Document,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let (database, collection, original_id, doc_index) = {
            let state = state.read(cx);
            let Some(index) = state.document_index(&session_key, &doc_key) else {
                return;
            };
            let Some(original) = state.document_for_key(&session_key, &doc_key) else {
                return;
            };
            let Some(id) = original.get("_id") else {
                return;
            };

            (session_key.database.clone(), session_key.collection.clone(), id.clone(), index)
        };

        let updated_for_task = updated.clone();
        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.replace_document(
                    &client,
                    &database,
                    &collection,
                    &original_id,
                    updated_for_task,
                )
            }
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                if let Some(existing) = session.data.items.get_mut(doc_index) {
                                    existing.doc = updated;
                                }
                                session.view.drafts.remove(&doc_key);
                                session.view.dirty.remove(&doc_key);
                            }
                            cx.emit(AppEvent::DocumentSaved {
                                session: session_key.clone(),
                                document: doc_key.clone(),
                            });
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to save document: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentSaveFailed {
                                session: session_key.clone(),
                                error: e.to_string(),
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

    /// Update a single document by _id.
    pub fn update_document_by_key(
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        update: Document,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let (database, collection, id) = {
            let state_ref = state.read(cx);
            let Some(doc) = state_ref
                .session(&session_key)
                .and_then(|session| session.view.drafts.get(&doc_key).cloned())
                .or_else(|| state_ref.document_for_key(&session_key, &doc_key))
            else {
                return;
            };
            let id = doc.get("_id").cloned();
            (session_key.database.clone(), session_key.collection.clone(), id)
        };

        let Some(id) = id else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Document missing _id; cannot update.",
                )));
                cx.notify();
            });
            return;
        };

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            let update = update.clone();
            async move {
                let manager = get_connection_manager();
                manager.update_one(&client, &database, &collection, doc! { "_id": id }, update)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            let doc_key = doc_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<mongodb::results::UpdateResult, crate::error::Error> =
                    task.await;
                let _ = cx.update(|cx| match result {
                    Ok(result) => {
                        state.update(cx, |state, cx| {
                            state.clear_draft(&session_key, &doc_key);
                            let event = AppEvent::DocumentsUpdated {
                                session: session_key.clone(),
                                matched: result.matched_count,
                                modified: result.modified_count,
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        AppCommands::load_documents_for_session(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to update document: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentsUpdateFailed {
                                session: session_key.clone(),
                                error: e.to_string(),
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

    // Bulk update moved to documents/bulk.rs.
}
