use gpui::{App, AppContext as _, Entity};
use mongodb::bson::Document;

use crate::bson::DocumentKey;
use crate::state::{AppEvent, AppState, SessionKey};

use crate::state::AppCommands;

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
        let (database, collection, original_id) = {
            let state = state.read(cx);
            let Some(original) = state.document_for_key(&session_key, &doc_key) else {
                return;
            };
            let Some(id) = original.get("_id") else {
                return;
            };

            (session_key.database.clone(), session_key.collection.clone(), id.clone())
        };
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { manager.delete_document(&client, &database, &collection, &original_id) }
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
                    }
                    Err(e) => {
                        log::error!("Failed to delete document: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentDeleteFailed {
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

    // Bulk delete moved to documents/bulk.rs.
}
