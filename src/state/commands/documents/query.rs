use gpui_kit::{App, AppContext as _, Entity};
use mongodb::bson::{Bson, Document, doc};

use crate::bson::{DocumentKey, format_relaxed_json_compact};
use crate::connection::FindDocumentsOptions;
use crate::connection::ops::documents::find_documents_page_async;
use crate::state::{
    AppEvent, AppState, DocumentQuery, QueryContent, QueryDefinition, SessionData, SessionDocument,
    SessionKey, StatusMessage,
};

use crate::state::AppCommands;

fn begin_document_query(
    data: &mut SessionData,
    request_id: u64,
    cancellation: crate::connection::types::CancellationToken,
) {
    if let Some(previous) = data.query_cancellation.take() {
        previous.cancel();
    }
    data.is_loading = true;
    data.query_error = None;
    data.query_cancellation = Some(cancellation);
    data.request_id = request_id;
}

fn record_document_query_success(
    data: &mut SessionData,
    request_id: u64,
    documents: Vec<Document>,
    total: u64,
) -> bool {
    if data.request_id != request_id {
        return false;
    }
    let items: Vec<SessionDocument> = documents
        .into_iter()
        .enumerate()
        .map(|(index, document)| SessionDocument {
            key: DocumentKey::from_document(&document, index),
            doc: document,
        })
        .collect();
    data.index_by_key =
        items.iter().enumerate().map(|(index, item)| (item.key.clone(), index)).collect();
    data.items = items;
    data.total = total;
    data.loaded = true;
    data.is_loading = false;
    data.query_error = None;
    data.query_cancellation = None;
    true
}

fn record_document_query_failure(data: &mut SessionData, request_id: u64, details: String) -> bool {
    if data.request_id != request_id {
        return false;
    }
    data.is_loading = false;
    data.query_error = Some(details);
    data.query_cancellation = None;
    true
}

fn format_query_document(document: &Option<Document>) -> String {
    document
        .as_ref()
        .map(|document| {
            let value = Bson::Document(document.clone()).into_relaxed_extjson();
            format_relaxed_json_compact(&value)
        })
        .unwrap_or_default()
}

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
        let (
            database,
            collection,
            skip,
            limit,
            request_id,
            filter,
            sort,
            sort_raw,
            projection,
            max_time,
        ) = {
            let state = state.read(cx);
            let (page, per_page, request_id, filter, sort, sort_raw, projection) =
                match state.session(&session_key) {
                    Some(session) => (
                        session.data.page,
                        session.data.per_page,
                        session.data.request_id + 1,
                        session.data.filter.clone(),
                        session.data.sort.clone(),
                        session.data.sort_raw.clone(),
                        session.data.projection.clone(),
                    ),
                    None => (0, 50, 1, None, None, String::new(), None),
                };
            (
                session_key.database.clone(),
                session_key.collection.clone(),
                page * per_page as u64,
                per_page,
                request_id,
                filter,
                sort,
                sort_raw,
                projection,
                std::time::Duration::from_millis(
                    state.settings.interactive_query_timeout_ms.max(100),
                ),
            )
        };

        let query_definition = QueryDefinition {
            connection_id: session_key.connection_id,
            database: session_key.database.clone(),
            collection: Some(session_key.collection.clone()),
            content: QueryContent::Documents(Box::new(DocumentQuery {
                filter_raw: format_query_document(&filter),
                filter: filter.clone(),
                sort_raw: format_query_document(&sort),
                sort: sort.clone(),
                projection_raw: format_query_document(&projection),
                projection: projection.clone(),
            })),
        };
        let effective_sort = if sort.is_none() && sort_raw.trim().is_empty() {
            Some(doc! { "$natural": 1 })
        } else {
            sort
        };

        // Cancel actual driver/server work before replacing it with a newer request.
        let runtime = state.read(cx).connection_manager().runtime_handle();
        let cancellation = crate::connection::types::CancellationToken::new();
        state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            begin_document_query(&mut session.data, request_id, cancellation.clone());
            cx.notify();
        });

        let task = runtime.spawn({
            let database_for_task = database.clone();
            let collection_for_task = collection.clone();
            async move {
                find_documents_page_async(
                    &client,
                    &database_for_task,
                    &collection_for_task,
                    FindDocumentsOptions {
                        filter,
                        sort: effective_sort,
                        projection,
                        skip,
                        limit,
                        max_time,
                        cancellation,
                    },
                )
                .await
            }
        });

        // Handle result on main thread
        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result: Result<(Vec<Document>, u64), crate::error::Error> = match task.await {
                    Ok(result) => result,
                    Err(error) => Err(crate::error::Error::Parse(format!(
                        "Document query task failed: {error}"
                    ))),
                };

                cx.update(|cx| match result {
                    Ok((documents, total)) => {
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            if !record_document_query_success(
                                &mut session.data,
                                request_id,
                                documents,
                                total,
                            ) {
                                return;
                            }

                            session.view.selected_docs.clear();
                            session.view.selected_doc = None;
                            session.view.selected_node_id = None;

                            session.generation = session.generation.wrapping_add(1);
                            let event =
                                AppEvent::DocumentsLoaded { session: session_key.clone(), total };
                            state.update_status_from_event(&event);
                            if let Err(error) = state.record_query(query_definition.clone()) {
                                state.set_status_message(Some(StatusMessage::error(format!(
                                    "Documents loaded, but {error}"
                                ))));
                            }
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            let details = error.to_string();
                            if !record_document_query_failure(
                                &mut session.data,
                                request_id,
                                details.clone(),
                            ) {
                                return;
                            }
                            let event = AppEvent::DocumentsLoadFailed {
                                session: session_key.clone(),
                                error: details,
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        log::error!("Failed to load documents: {}", error);
                    }
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::*;

    #[test]
    fn refresh_cancels_the_previous_query_token() {
        let previous = crate::connection::types::CancellationToken::new();
        let current = crate::connection::types::CancellationToken::new();
        let mut data = SessionData::default();
        data.query_cancellation = Some(previous.clone());
        data.query_error = Some("old failure".to_string());

        begin_document_query(&mut data, 2, current.clone());

        assert!(previous.is_cancelled());
        assert!(!current.is_cancelled());
        assert_eq!(data.request_id, 2);
        assert!(data.is_loading);
        assert!(data.query_error.is_none());
    }

    #[test]
    fn current_query_failure_preserves_stale_documents() {
        let document = doc! { "_id": 1, "value": "stale but visible" };
        let mut data = SessionData::default();
        data.items = vec![SessionDocument {
            key: DocumentKey::from_document(&document, 0),
            doc: document.clone(),
        }];
        data.total = 1;
        data.loaded = true;
        data.is_loading = true;
        data.request_id = 7;
        data.query_cancellation = Some(crate::connection::types::CancellationToken::new());

        assert!(record_document_query_failure(&mut data, 7, "server rejected query".to_string(),));

        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].doc, document);
        assert_eq!(data.total, 1);
        assert!(data.loaded);
        assert!(!data.is_loading);
        assert_eq!(data.query_error.as_deref(), Some("server rejected query"));
        assert!(data.query_cancellation.is_none());
    }

    #[test]
    fn stale_query_success_cannot_replace_current_documents() {
        let current_document = doc! { "_id": 9, "value": "current" };
        let stale_document = doc! { "_id": 8, "value": "stale" };
        let current = crate::connection::types::CancellationToken::new();
        let mut data = SessionData::default();
        data.items = vec![SessionDocument {
            key: DocumentKey::from_document(&current_document, 0),
            doc: current_document.clone(),
        }];
        data.total = 1;
        data.is_loading = true;
        data.request_id = 9;
        data.query_cancellation = Some(current.clone());

        assert!(!record_document_query_success(&mut data, 8, vec![stale_document], 99));

        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].doc, current_document);
        assert_eq!(data.total, 1);
        assert!(data.is_loading);
        assert!(!current.is_cancelled());
        assert!(data.query_cancellation.is_some());
    }

    #[test]
    fn stale_query_failure_cannot_replace_current_state() {
        let current = crate::connection::types::CancellationToken::new();
        let mut data = SessionData::default();
        data.total = 3;
        data.is_loading = true;
        data.request_id = 9;
        data.query_cancellation = Some(current.clone());

        assert!(!record_document_query_failure(&mut data, 8, "stale failure".to_string(),));

        assert_eq!(data.total, 3);
        assert!(data.is_loading);
        assert!(data.query_error.is_none());
        assert!(!current.is_cancelled());
        assert!(data.query_cancellation.is_some());
    }
}
