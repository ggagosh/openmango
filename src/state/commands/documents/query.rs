use gpui::{App, AppContext as _, Entity};
use mongodb::bson::{Document, doc};

use crate::bson::DocumentKey;
use crate::connection::FindDocumentsOptions;
use crate::state::{AppEvent, AppState, SessionDocument, SessionKey};

use crate::state::AppCommands;

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
        let manager = state.read(cx).connection_manager();
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
}
