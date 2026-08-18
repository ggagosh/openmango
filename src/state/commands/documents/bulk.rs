use std::{sync::Arc, time::Duration};

use gpui::{App, AppContext as _, Entity};
use mongodb::{
    Client,
    bson::{Document, oid::ObjectId},
};

use crate::connection::{CancellationToken, ConnectionManager, FindDocumentsOptions};
use crate::operations::{
    DocumentTarget, MAX_REVERSIBLE_BULK_BYTES, MAX_REVERSIBLE_BULK_DOCUMENTS, MongoMutationBackend,
    Mutation, OperationContext, OperationEngine, OperationError,
};
use crate::state::AppCommands;
use crate::state::{AppEvent, AppState, SessionKey};

struct BulkHistory {
    engine: Arc<OperationEngine>,
    backend: Arc<MongoMutationBackend>,
    connection_name: String,
}

fn bulk_history(
    state: &AppState,
    connection_id: uuid::Uuid,
) -> Result<Option<BulkHistory>, OperationError> {
    let Some(engine) = crate::operations::tracked_engine(
        state.connection_reversible_history(connection_id),
        state.operation_engine(),
    )?
    else {
        return Ok(None);
    };
    Ok(Some(BulkHistory {
        engine,
        backend: state.operation_backend(),
        connection_name: state
            .connection_name(connection_id)
            .unwrap_or_else(|| "Connection".to_string()),
    }))
}

fn bounded_documents(
    manager: &ConnectionManager,
    client: &Client,
    session_key: &SessionKey,
    filter: Document,
    cancellation: CancellationToken,
) -> Result<Vec<Document>, crate::error::Error> {
    let (documents, total) = manager.find_documents(
        client,
        &session_key.database,
        &session_key.collection,
        FindDocumentsOptions {
            filter: Some(filter),
            sort: Some(mongodb::bson::doc! { "_id": 1 }),
            projection: None,
            skip: 0,
            limit: (MAX_REVERSIBLE_BULK_DOCUMENTS + 1) as i64,
            max_time: Duration::from_secs(30),
            cancellation,
        },
    )?;
    if exceeds_bulk_limit(total, documents.len()) {
        return Err(crate::error::Error::Parse(format!(
            "Reversible bulk writes are limited to {MAX_REVERSIBLE_BULK_DOCUMENTS} documents. Narrow the filter and try again."
        )));
    }
    Ok(documents)
}

fn target(
    session_key: &SessionKey,
    connection_name: &str,
    id: mongodb::bson::Bson,
) -> DocumentTarget {
    DocumentTarget {
        connection_id: session_key.connection_id,
        connection_name: connection_name.to_string(),
        database: session_key.database.clone(),
        collection: session_key.collection.clone(),
        id,
    }
}

fn ensure_recovery_size<'a>(
    documents: impl IntoIterator<Item = &'a Document>,
    max_bytes: usize,
) -> Result<(), crate::error::Error> {
    let mut bytes = 0usize;
    for document in documents {
        let encoded = mongodb::bson::to_vec(document).map_err(|error| {
            crate::error::Error::Parse(format!("Could not encode recovery data: {error}"))
        })?;
        bytes = bytes
            .checked_add(encoded.len())
            .ok_or_else(|| crate::error::Error::Parse("Recovery data is too large.".to_string()))?;
        if bytes > max_bytes {
            return Err(crate::error::Error::Parse(format!(
                "Reversible bulk recovery data is limited to {} MiB. Narrow the write and try again.",
                MAX_REVERSIBLE_BULK_BYTES / (1024 * 1024)
            )));
        }
    }
    Ok(())
}

fn exceeds_bulk_limit(total: u64, fetched: usize) -> bool {
    total > MAX_REVERSIBLE_BULK_DOCUMENTS as u64 || fetched > MAX_REVERSIBLE_BULK_DOCUMENTS
}

fn assign_missing_ids(documents: &mut [Document]) {
    for document in documents {
        if !document.contains_key("_id") {
            document.insert("_id", ObjectId::new());
        }
    }
}

fn plan_replacements(
    documents: Vec<Document>,
    replacement: &Document,
) -> Result<Vec<(mongodb::bson::Bson, Document, Document)>, crate::error::Error> {
    documents
        .into_iter()
        .map(|before| {
            let id = before.get("_id").cloned().ok_or_else(|| {
                crate::error::Error::Parse(
                    "Matched document is missing _id; no replacements were attempted.".to_string(),
                )
            })?;
            let mut after = replacement.clone();
            after.insert("_id", id.clone());
            Ok((id, before, after))
        })
        .collect()
}

fn batch_error(
    action: &str,
    completed: usize,
    total: usize,
    error: OperationError,
) -> crate::error::Error {
    crate::error::Error::Parse(format!(
        "Reversible {action} stopped after {completed} of {total} documents: {}",
        error.user_message()
    ))
}

impl AppCommands {
    /// Insert multiple documents into a collection.
    pub fn insert_documents(
        state: Entity<AppState>,
        session_key: SessionKey,
        mut documents: Vec<Document>,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let count = documents.len();
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let (history, manager) = {
            let state = state.read(cx);
            (bulk_history(state, session_key.connection_id), state.connection_manager())
        };
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                state.update(cx, |state, cx| {
                    let event = AppEvent::DocumentsInsertFailed {
                        count,
                        error: error.user_message().to_string(),
                    };
                    state.update_status_from_event(&event);
                    cx.emit(event);
                    cx.notify();
                });
                return;
            }
        };
        if history.is_some() && count > MAX_REVERSIBLE_BULK_DOCUMENTS {
            state.update(cx, |state, cx| {
                let event = AppEvent::DocumentsInsertFailed {
                    count,
                    error: format!(
                        "Reversible bulk writes are limited to {MAX_REVERSIBLE_BULK_DOCUMENTS} documents."
                    ),
                };
                state.update_status_from_event(&event);
                cx.emit(event);
                cx.notify();
            });
            return;
        }
        if history.is_some() {
            assign_missing_ids(&mut documents);
        }
        let tracked = history.is_some();
        let session_for_task = session_key.clone();
        let task = cx.background_spawn(async move {
            let Some(history) = history else {
                return manager.insert_documents(
                    &client,
                    &session_for_task.database,
                    &session_for_task.collection,
                    documents,
                );
            };
            ensure_recovery_size(documents.iter(), MAX_REVERSIBLE_BULK_BYTES)?;
            history.backend.register_client(session_for_task.connection_id, client);
            let total = documents.len();
            for (completed, document) in documents.into_iter().enumerate() {
                let Some(id) = document.get("_id").cloned() else {
                    return Err(crate::error::Error::Parse(
                        "Could not assign an _id for reversible insert.".to_string(),
                    ));
                };
                history
                    .engine
                    .execute(
                        OperationContext::user(),
                        Mutation::InsertDocument {
                            target: target(&session_for_task, &history.connection_name, id),
                            document,
                        },
                    )
                    .map_err(|error| batch_error("insert", completed, total, error))?;
            }
            Ok(total)
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<usize, crate::error::Error> = task.await;
                let succeeded = result.is_ok();
                let _ = cx.update(|cx| match result {
                    Ok(inserted) => {
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentsInserted { count: inserted };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
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
                let _ = cx.update(|cx| {
                    if succeeded || tracked {
                        AppCommands::load_documents_for_session(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                    }
                    if tracked {
                        AppCommands::collection_history_changed(state.clone(), session_key, cx);
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
        let (history, manager) = {
            let state = state.read(cx);
            (bulk_history(state, session_key.connection_id), state.connection_manager())
        };
        match history {
            Err(error) => {
                state.update(cx, |state, cx| {
                    let event = AppEvent::DocumentsUpdateFailed {
                        session: session_key.clone(),
                        error: error.user_message().to_string(),
                    };
                    state.update_status_from_event(&event);
                    cx.emit(event);
                    cx.notify();
                });
                return;
            }
            Ok(Some(_)) => {
                state.update(cx, |state, cx| {
                    let event = AppEvent::DocumentsUpdateFailed {
                        session: session_key.clone(),
                        error: "Bulk operator updates are not reversible yet. Use Replace mode or disable reversible history for this write.".to_string(),
                    };
                    state.update_status_from_event(&event);
                    cx.emit(event);
                    cx.notify();
                });
                return;
            }
            Ok(None) => {}
        }
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let task = cx.background_spawn(async move {
            manager.update_many(&client, &database, &collection, filter, update)
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

    /// Replace each document matching a filter while preserving its original `_id`.
    pub fn replace_documents_by_filter(
        state: Entity<AppState>,
        session_key: SessionKey,
        filter: Document,
        replacement: Document,
        cancellation: crate::connection::types::CancellationToken,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let (history, manager) = {
            let state = state.read(cx);
            (bulk_history(state, session_key.connection_id), state.connection_manager())
        };
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                state.update(cx, |state, cx| {
                    let event = AppEvent::DocumentsUpdateFailed {
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
        let tracked = history.is_some();
        let session_for_task = session_key.clone();
        let task = cx.background_spawn(async move {
            let Some(history) = history else {
                return manager.replace_documents_by_filter(
                    &client,
                    &session_for_task.database,
                    &session_for_task.collection,
                    filter,
                    replacement,
                    cancellation,
                );
            };
            let documents = bounded_documents(
                &manager,
                &client,
                &session_for_task,
                filter,
                cancellation.clone(),
            )?;
            let replacements = plan_replacements(documents, &replacement)?;
            ensure_recovery_size(
                replacements.iter().flat_map(|(_, before, after)| [before, after]),
                MAX_REVERSIBLE_BULK_BYTES,
            )?;
            history.backend.register_client(session_for_task.connection_id, client);
            let total = replacements.len();
            let modified_count =
                replacements.iter().filter(|(_, before, after)| before != after).count() as u64;
            for (completed, (id, before, after)) in replacements.into_iter().enumerate() {
                if cancellation.is_cancelled() {
                    return Err(crate::error::Error::Parse(format!(
                        "Reversible replacement cancelled after {completed} of {total} documents"
                    )));
                }
                history
                    .engine
                    .execute(
                        OperationContext::user(),
                        Mutation::ReplaceDocument {
                            target: target(&session_for_task, &history.connection_name, id),
                            replacement: after,
                            editor_precondition: Some(before),
                        },
                    )
                    .map_err(|error| batch_error("replacement", completed, total, error))?;
            }
            Ok(crate::connection::types::BulkReplaceResult {
                matched_count: total as u64,
                modified_count,
            })
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<
                    crate::connection::types::BulkReplaceResult,
                    crate::error::Error,
                > = task.await;
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
                    }
                    Err(error) => {
                        state.update(cx, |state, cx| {
                            let event = AppEvent::DocumentsUpdateFailed {
                                session: session_key.clone(),
                                error: error.to_string(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                });
                let _ = cx.update(|cx| {
                    AppCommands::load_documents_for_session(state.clone(), session_key.clone(), cx);
                    if tracked {
                        AppCommands::collection_history_changed(state.clone(), session_key, cx);
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
        let (history, manager) = {
            let state = state.read(cx);
            (bulk_history(state, session_key.connection_id), state.connection_manager())
        };
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                state.update(cx, |state, cx| {
                    let event = AppEvent::DocumentsDeleteFailed {
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
        let tracked = history.is_some();
        let session_for_task = session_key.clone();
        let task = cx.background_spawn(async move {
            let Some(history) = history else {
                return manager.delete_documents(
                    &client,
                    &session_for_task.database,
                    &session_for_task.collection,
                    filter,
                );
            };
            let documents = bounded_documents(
                &manager,
                &client,
                &session_for_task,
                filter,
                CancellationToken::new(),
            )?;
            if documents.iter().any(|document| !document.contains_key("_id")) {
                return Err(crate::error::Error::Parse(
                    "Matched document is missing _id; no deletes were attempted.".to_string(),
                ));
            }
            ensure_recovery_size(documents.iter(), MAX_REVERSIBLE_BULK_BYTES)?;
            history.backend.register_client(session_for_task.connection_id, client);
            let total = documents.len();
            for (completed, document) in documents.into_iter().enumerate() {
                let id = document.get("_id").cloned().expect("validated above");
                history
                    .engine
                    .execute(
                        OperationContext::user(),
                        Mutation::DeleteDocument {
                            target: target(&session_for_task, &history.connection_name, id),
                            editor_precondition: Some(document),
                        },
                    )
                    .map_err(|error| batch_error("delete", completed, total, error))?;
            }
            Ok(total as u64)
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<u64, crate::error::Error> = task.await;
                let succeeded = result.is_ok();
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
                let _ = cx.update(|cx| {
                    if succeeded || tracked {
                        AppCommands::load_documents_for_session(
                            state.clone(),
                            session_key.clone(),
                            cx,
                        );
                    }
                    if tracked {
                        AppCommands::collection_history_changed(state.clone(), session_key, cx);
                    }
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc};

    use super::{
        MAX_REVERSIBLE_BULK_BYTES, MAX_REVERSIBLE_BULK_DOCUMENTS, assign_missing_ids,
        ensure_recovery_size, exceeds_bulk_limit, plan_replacements,
    };

    #[test]
    fn tracked_bulk_inserts_assign_missing_ids_without_replacing_existing_ids() {
        let mut documents = vec![doc! { "name": "new" }, doc! { "_id": 7, "name": "kept" }];

        assign_missing_ids(&mut documents);

        assert!(matches!(documents[0].get("_id"), Some(Bson::ObjectId(_))));
        assert_eq!(documents[1].get_i32("_id").unwrap(), 7);
        assert_eq!(MAX_REVERSIBLE_BULK_DOCUMENTS, 100);
        assert!(!exceeds_bulk_limit(100, 100));
        assert!(exceeds_bulk_limit(100, 101));
        assert!(exceeds_bulk_limit(101, 100));
        assert!(ensure_recovery_size(documents.iter(), MAX_REVERSIBLE_BULK_BYTES).is_ok());
        assert!(ensure_recovery_size(documents.iter(), 0).is_err());
    }

    #[test]
    fn tracked_bulk_replacements_preserve_each_id_and_fail_before_missing_ids() {
        let planned = plan_replacements(
            vec![doc! { "_id": 1, "old": true }, doc! { "_id": 2, "old": true }],
            &doc! { "new": true },
        )
        .unwrap();

        assert_eq!(planned[0].2, doc! { "new": true, "_id": 1 });
        assert_eq!(planned[1].2, doc! { "new": true, "_id": 2 });
        assert!(plan_replacements(vec![doc! { "old": true }], &doc! { "new": true }).is_err());
    }
}
