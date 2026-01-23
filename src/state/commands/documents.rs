use gpui::{App, AppContext as _, Entity};
use mongodb::bson::{Document, doc};

use crate::bson::DocumentKey;
use crate::connection::{FindDocumentsOptions, get_connection_manager};
use crate::state::{AppEvent, AppState, SessionDocument, SessionKey, StatusMessage};

use super::AppCommands;

impl AppCommands {
    /// Load documents for a collection session with pagination.
    pub fn load_documents_for_session(
        state: Entity<AppState>,
        session_key: SessionKey,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        // Get selected db/collection + session data
        let (database, collection, skip, limit, request_id, filter, sort, projection, sort_raw) = {
            let state = state.read(cx);
            let (page, per_page, request_id, filter, sort, projection, sort_raw) =
                match state.session(&session_key) {
                    Some(session) => (
                        session.data.page,
                        session.data.per_page,
                        session.data.request_id + 1,
                        session.data.filter.clone(),
                        session.data.sort.clone(),
                        session.data.projection.clone(),
                        session.data.sort_raw.clone(),
                    ),
                    None => (0, 50, 1, None, None, None, String::new()),
                };
            (
                session_key.database.clone(),
                session_key.collection.clone(),
                page * per_page as u64,
                per_page,
                request_id,
                filter,
                sort,
                projection,
                sort_raw,
            )
        };

        let effective_sort = if sort.is_none() && sort_raw.trim().is_empty() {
            Some(doc! { "$natural": 1 })
        } else {
            sort
        };

        // Mark session as loading and bump request id
        state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            session.data.is_loading = true;
            session.data.request_id = request_id;
            cx.notify();
        });

        // Run blocking MongoDB operation in background thread
        let task = cx.background_spawn({
            let database_for_task = database.clone();
            let collection_for_task = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.find_documents(
                    &client,
                    &database_for_task,
                    &collection_for_task,
                    FindDocumentsOptions { filter, sort: effective_sort, projection, skip, limit },
                )
            }
        });

        // Handle result on main thread
        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(Vec<Document>, u64), crate::error::Error> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok((documents, total)) => {
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            if session.data.request_id != request_id {
                                return;
                            }
                            let items: Vec<SessionDocument> = documents
                                .into_iter()
                                .enumerate()
                                .map(|(idx, doc)| SessionDocument {
                                    key: DocumentKey::from_document(&doc, idx),
                                    doc,
                                })
                                .collect();
                            session.data.index_by_key = items
                                .iter()
                                .enumerate()
                                .map(|(idx, item)| (item.key.clone(), idx))
                                .collect();
                            session.data.items = items;
                            session.data.total = total;
                            session.data.is_loading = false;

                            if let Some(selected) = session.view.selected_doc.clone()
                                && !session.data.index_by_key.contains_key(&selected)
                            {
                                session.view.selected_doc = None;
                                session.view.selected_node_id = None;
                            }

                            let event =
                                AppEvent::DocumentsLoaded { session: session_key.clone(), total };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key)
                                && session.data.request_id == request_id
                            {
                                session.data.is_loading = false;
                            }
                            cx.notify();
                        });
                        log::error!("Failed to load documents: {}", e);
                    }
                });
            }
        })
        .detach();
    }

    /// Insert a document into a collection.
    pub fn insert_document(
        state: Entity<AppState>,
        session_key: SessionKey,
        document: Document,
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
            let document = document.clone();
            async move {
                let manager = get_connection_manager();
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
                            let event = AppEvent::DocumentInserted;
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
                        log::error!("Failed to insert document: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentInsertFailed { error: e.to_string() };
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

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.delete_document(&client, &database, &collection, &original_id)
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
                                if session.view.selected_doc.as_ref() == Some(&doc_key) {
                                    session.view.selected_doc = None;
                                    session.view.selected_node_id = None;
                                }
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
