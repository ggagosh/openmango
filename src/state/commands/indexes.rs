use std::sync::Arc;

use gpui::{App, AppContext as _, Entity};
use mongodb::{IndexModel, bson::Document};

use crate::operations::{MongoMutationBackend, OperationEngine, OperationError};
use crate::state::{AppEvent, AppState, SessionKey};

use super::AppCommands;

struct IndexHistory {
    engine: Arc<OperationEngine>,
    backend: Arc<MongoMutationBackend>,
    connection_name: String,
}

fn index_history(
    state: &AppState,
    connection_id: uuid::Uuid,
) -> Result<Option<IndexHistory>, OperationError> {
    let Some(engine) = crate::operations::tracked_engine(
        state.connection_reversible_history(connection_id),
        state.operation_engine(),
    )?
    else {
        return Ok(None);
    };
    Ok(Some(IndexHistory {
        engine,
        backend: state.operation_backend(),
        connection_name: state
            .connection_name(connection_id)
            .unwrap_or_else(|| "Connection".to_string()),
    }))
}

fn index_target(
    session_key: &SessionKey,
    connection_name: &str,
    index_name: &str,
) -> crate::operations::DocumentTarget {
    crate::operations::DocumentTarget {
        connection_id: session_key.connection_id,
        connection_name: connection_name.to_string(),
        database: session_key.database.clone(),
        collection: session_key.collection.clone(),
        id: index_name.into(),
    }
}

impl AppCommands {
    /// Load indexes for a collection session.
    pub fn load_collection_indexes(
        state: Entity<AppState>,
        session_key: SessionKey,
        force: bool,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let manager = state.read(cx).connection_manager();

        let should_load = state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            if session.data.indexes_loading {
                return false;
            }
            if !force && session.data.indexes.is_some() && session.data.indexes_error.is_none() {
                return false;
            }
            session.data.indexes_loading = true;
            session.data.indexes_error = None;
            cx.notify();
            true
        });

        if !should_load {
            return;
        }

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { manager.list_indexes(&client, &database, &collection) }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Vec<IndexModel>, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(indexes) => {
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.indexes = Some(indexes.clone());
                                session.data.indexes_loading = false;
                                session.data.indexes_error = None;
                            }
                            let event = AppEvent::IndexesLoaded { count: indexes.len() };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to load indexes: {}", e);
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.indexes_loading = false;
                                session.data.indexes_error = Some(e.to_string());
                            }
                            let event = AppEvent::IndexesLoadFailed { error: e.to_string() };
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

    /// Drop an index by name for a collection session.
    pub fn drop_collection_index(
        state: Entity<AppState>,
        session_key: SessionKey,
        index_name: String,
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
            (index_history(state, session_key.connection_id), state.connection_manager())
        };
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                state.update(cx, |state, cx| {
                    let event =
                        AppEvent::IndexDropFailed { error: error.user_message().to_string() };
                    state.update_status_from_event(&event);
                    cx.emit(event);
                    cx.notify();
                });
                return;
            }
        };
        let tracked = history.is_some();
        let session_for_task = session_key.clone();
        let name_for_task = index_name.clone();
        let task = cx.background_spawn(async move {
            let Some(history) = history else {
                return manager.drop_index(
                    &client,
                    &session_for_task.database,
                    &session_for_task.collection,
                    &name_for_task,
                );
            };
            history.backend.register_client(session_for_task.connection_id, client);
            history
                .engine
                .execute(
                    crate::operations::OperationContext::user(),
                    crate::operations::Mutation::DropIndex {
                        target: index_target(
                            &session_for_task,
                            &history.connection_name,
                            &name_for_task,
                        ),
                    },
                )
                .map(|_| ())
                .map_err(|error| crate::error::Error::Parse(error.user_message().to_string()))
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            let index_name = index_name.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let event = AppEvent::IndexDropped { name: index_name.clone() };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        AppCommands::load_collection_indexes(
                            state.clone(),
                            session_key.clone(),
                            true,
                            cx,
                        );
                        if tracked {
                            AppCommands::collection_history_changed(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to drop index: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::IndexDropFailed { error: e.to_string() };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        if tracked {
                            AppCommands::load_collection_indexes(
                                state.clone(),
                                session_key.clone(),
                                true,
                                cx,
                            );
                            AppCommands::collection_history_changed(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                    }
                });
            }
        })
        .detach();
    }

    /// Create an index for a collection session.
    pub fn create_collection_index(
        state: Entity<AppState>,
        session_key: SessionKey,
        index_doc: Document,
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
            (index_history(state, session_key.connection_id), state.connection_manager())
        };
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                state.update(cx, |state, cx| {
                    let event = AppEvent::IndexCreateFailed {
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
        let index_name = index_doc.get_str("name").ok().map(|value| value.to_string());
        if history.is_some() && index_name.is_none() {
            state.update(cx, |state, cx| {
                let event = AppEvent::IndexCreateFailed {
                    session: session_key.clone(),
                    error: "Reversible index creation requires an explicit name.".to_string(),
                };
                state.update_status_from_event(&event);
                cx.emit(event);
                cx.notify();
            });
            return;
        }
        let tracked = history.is_some();
        let session_for_task = session_key.clone();
        let name_for_task = index_name.clone();
        let task = cx.background_spawn(async move {
            let Some(history) = history else {
                return manager.create_index(
                    &client,
                    &session_for_task.database,
                    &session_for_task.collection,
                    index_doc,
                );
            };
            history.backend.register_client(session_for_task.connection_id, client);
            let name = name_for_task.expect("validated above");
            history
                .engine
                .execute(
                    crate::operations::OperationContext::user(),
                    crate::operations::Mutation::CreateIndex {
                        target: index_target(&session_for_task, &history.connection_name, &name),
                        definition: index_doc,
                    },
                )
                .map(|_| ())
                .map_err(|error| crate::error::Error::Parse(error.user_message().to_string()))
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            let index_name = index_name.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let event = AppEvent::IndexCreated {
                                session: session_key.clone(),
                                name: index_name.clone(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        AppCommands::load_collection_indexes(
                            state.clone(),
                            session_key.clone(),
                            true,
                            cx,
                        );
                        if tracked {
                            AppCommands::collection_history_changed(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to create index: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::IndexCreateFailed {
                                session: session_key.clone(),
                                error: e.to_string(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        if tracked {
                            AppCommands::load_collection_indexes(
                                state.clone(),
                                session_key.clone(),
                                true,
                                cx,
                            );
                            AppCommands::collection_history_changed(
                                state.clone(),
                                session_key.clone(),
                                cx,
                            );
                        }
                    }
                });
            }
        })
        .detach();
    }

    /// Replace an index by dropping the old name and creating a new one.
    pub fn replace_collection_index(
        state: Entity<AppState>,
        session_key: SessionKey,
        old_name: String,
        index_doc: Document,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            return;
        }
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        if state.read(cx).connection_reversible_history(session_key.connection_id) {
            state.update(cx, |state, cx| {
                let event = AppEvent::IndexCreateFailed {
                    session: session_key.clone(),
                    error: "Index replacement is not reversible yet. Drop and recreate the index, or disable Reversible history for this write."
                        .to_string(),
                };
                state.update_status_from_event(&event);
                cx.emit(event);
                cx.notify();
            });
            return;
        }
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let manager = state.read(cx).connection_manager();

        let new_name = index_doc.get_str("name").ok().map(|value| value.to_string());
        let task =
            cx.background_spawn({
                let database = database.clone();
                let collection = collection.clone();
                let old_name = old_name.clone();
                let index_doc = index_doc.clone();
                async move {
                    manager.replace_index(&client, &database, &collection, &old_name, index_doc)
                }
            });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            let new_name = new_name.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let event = AppEvent::IndexCreated {
                                session: session_key.clone(),
                                name: new_name.clone(),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                        AppCommands::load_collection_indexes(
                            state.clone(),
                            session_key.clone(),
                            true,
                            cx,
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to replace index: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::IndexCreateFailed {
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
