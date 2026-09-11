use gpui_kit::{App, AppContext as _, Entity};
use mongodb::bson::{Document, doc};

use crate::bson::{DocumentKey, parse_bson_from_relaxed_json};
use crate::connection::ops::documents::replace_document_if_current_async;
use crate::state::{AppEvent, AppState, EditorSessionId, SessionKey, StatusMessage};

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
        let baseline = state.read(cx).document_edit_baseline(&session_key, &doc_key);
        Self::save_document_internal(state, session_key, doc_key, updated, baseline, None, cx);
    }

    pub fn save_document_for_editor(
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        updated: Document,
        baseline_document: Document,
        editor: EditorSessionId,
        cx: &mut App,
    ) {
        Self::save_document_internal(
            state,
            session_key,
            doc_key,
            updated,
            Some(baseline_document),
            Some(editor),
            cx,
        );
    }

    fn save_document_internal(
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        updated: Document,
        baseline_document: Option<Document>,
        editor: Option<EditorSessionId>,
        cx: &mut App,
    ) {
        let reject = |message: &str, cx: &mut App| {
            state.update(cx, |state, cx| {
                let event = AppEvent::DocumentSaveFailed {
                    session: session_key.clone(),
                    document: doc_key.clone(),
                    editor,
                    error: message.to_string(),
                };
                state.update_status_from_event(&event);
                cx.emit(event);
                cx.notify();
            });
        };
        if state
            .read(cx)
            .session_view(&session_key)
            .is_some_and(|view| view.saving_documents.contains(&doc_key))
        {
            reject("A save is already in progress for this document.", cx);
            return;
        }
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            reject("Document could not be saved. Check connection write permissions.", cx);
            return;
        }
        let Some(baseline_document) = baseline_document else {
            reject("Original document is unavailable. Reload before saving.", cx);
            return;
        };
        if updated.get("_id") != baseline_document.get("_id") {
            reject("The document _id cannot be changed.", cx);
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            reject("Connection is no longer active.", cx);
            return;
        };
        let (database, collection, original_id, should_reload_after_save) = {
            let state_ref = state.read(cx);
            let doc_index = state_ref.document_index(&session_key, &doc_key).or_else(|| {
                state_ref.session(&session_key).and_then(|session| {
                    session.data.items.iter().position(|item| item.key == doc_key)
                })
            });
            let original_id = state_ref
                .document_for_key(&session_key, &doc_key)
                .and_then(|original| original.get("_id").cloned())
                .or_else(|| parse_bson_from_relaxed_json(doc_key.as_str()).ok());

            let Some(original_id) = original_id else {
                reject("Could not resolve original document ID for save.", cx);
                return;
            };

            (
                session_key.database.clone(),
                session_key.collection.clone(),
                original_id,
                doc_index.is_none(),
            )
        };
        let runtime = state.read(cx).connection_manager().runtime_handle();
        state.update(cx, |state, cx| {
            state.ensure_session(session_key.clone()).view.saving_documents.insert(doc_key.clone());
            state.set_status_message(Some(StatusMessage::info("Saving document…")));
            cx.notify();
        });

        let updated_for_task = updated.clone();
        let task = runtime.spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                replace_document_if_current_async(
                    &client,
                    &database,
                    &collection,
                    original_id,
                    baseline_document,
                    updated_for_task,
                )
                .await
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            let doc_key = doc_key.clone();
            let updated = updated.clone();
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result: Result<(), crate::error::Error> = match task.await {
                    Ok(result) => result,
                    Err(error) => Err(crate::error::Error::Parse(format!(
                        "Document save task failed: {error}"
                    ))),
                };
                let saved = result.is_ok();

                cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            if let Some(editor) = editor {
                                state
                                    .editor_sessions()
                                    .refresh_document_baseline(editor, updated.clone());
                            }
                            if let Some(session) = state.session_mut(&session_key) {
                                session.view.saving_documents.remove(&doc_key);
                                let draft_unchanged =
                                    session.view.drafts.get(&doc_key) == Some(&updated);
                                let index =
                                    session.data.items.iter().position(|item| item.key == doc_key);
                                if let Some(index) = index
                                    && let Some(existing) = session.data.items.get_mut(index)
                                {
                                    existing.doc = updated.clone();
                                }
                                if draft_unchanged {
                                    session.view.drafts.remove(&doc_key);
                                    session.view.draft_baselines.remove(&doc_key);
                                    session.view.dirty.remove(&doc_key);
                                } else if session.view.drafts.contains_key(&doc_key) {
                                    session.view.draft_baselines.insert(doc_key.clone(), updated);
                                }
                                session.generation = session.generation.wrapping_add(1);
                            }
                            let event = AppEvent::DocumentSaved {
                                session: session_key.clone(),
                                document: doc_key.clone(),
                                editor,
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        if should_reload_after_save {
                            AppCommands::load_documents_for_session(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to save document");
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.view.saving_documents.remove(&doc_key);
                            }
                            let event = AppEvent::DocumentSaveFailed {
                                session: session_key.clone(),
                                document: doc_key.clone(),
                                editor,
                                error: e.to_string(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                });
                if saved {
                    cx.background_executor().timer(std::time::Duration::from_millis(750)).await;
                    cx.update(|cx| {
                        AppCommands::collection_history_changed(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                    });
                }
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
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            let update = update.clone();
            async move {
                manager.update_one(&client, &database, &collection, doc! { "_id": id }, update)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            let doc_key = doc_key.clone();
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result: Result<mongodb::results::UpdateResult, crate::error::Error> =
                    task.await;
                cx.update(|cx| match result {
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
