use gpui::{App, AppContext as _, Entity};
use mongodb::bson::Document;

use crate::connection::get_connection_manager;
use crate::state::AppCommands;
use crate::state::{AppEvent, AppState, SessionKey};

impl AppCommands {
    /// Insert multiple documents into a collection.
    pub fn insert_documents(
        state: Entity<AppState>,
        session_key: SessionKey,
        documents: Vec<Document>,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let count = documents.len();
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.insert_documents(&client, &database, &collection, documents)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<usize, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(inserted) => {
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentsInserted { count: inserted };
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
                        log::error!("Failed to insert documents: {}", e);
                        state.update(cx, |state, cx| {
                            let event =
                                AppEvent::DocumentsInsertFailed { count, error: e.to_string() };
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

    /// Update multiple documents by filter.
    pub fn update_documents_by_filter(
        state: Entity<AppState>,
        session_key: SessionKey,
        filter: Document,
        update: Document,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            let update = update.clone();
            async move {
                let manager = get_connection_manager();
                manager.update_many(&client, &database, &collection, filter, update)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<mongodb::results::UpdateResult, crate::error::Error> =
                    task.await;
                let _ = cx.update(|cx| match result {
                    Ok(result) => {
                        state.update(cx, |state, cx| {
                            state.clear_all_drafts(&session_key);
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
                        log::error!("Failed to update documents: {}", e);
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

    /// Delete multiple documents by filter.
    pub fn delete_documents_by_filter(
        state: Entity<AppState>,
        session_key: SessionKey,
        filter: Document,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.delete_documents(&client, &database, &collection, filter)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<u64, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(deleted) => {
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentsDeleted {
                                session: session_key.clone(),
                                deleted,
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
                        log::error!("Failed to delete documents: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentsDeleteFailed {
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
}
