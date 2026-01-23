use gpui::{App, AppContext as _, Entity};
use mongodb::{IndexModel, bson::Document};

use crate::connection::get_connection_manager;
use crate::state::{AppEvent, AppState, SessionKey};

use super::AppCommands;

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
            async move {
                let manager = get_connection_manager();
                manager.list_indexes(&client, &database, &collection)
            }
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
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            let index_name = index_name.clone();
            async move {
                let manager = get_connection_manager();
                manager.drop_index(&client, &database, &collection, &index_name)
            }
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
                    }
                    Err(e) => {
                        log::error!("Failed to drop index: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::IndexDropFailed { error: e.to_string() };
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
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let index_name = index_doc.get_str("name").ok().map(|value| value.to_string());
        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            let index_doc = index_doc.clone();
            async move {
                let manager = get_connection_manager();
                manager.create_index(&client, &database, &collection, index_doc)
            }
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
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();

        let new_name = index_doc.get_str("name").ok().map(|value| value.to_string());
        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            let old_name = old_name.clone();
            let new_name = new_name.clone();
            let index_doc = index_doc.clone();
            async move {
                let manager = get_connection_manager();
                if new_name.as_deref() == Some(old_name.as_str()) {
                    manager.drop_index(&client, &database, &collection, &old_name)?;
                    manager.create_index(&client, &database, &collection, index_doc)?;
                } else {
                    manager.create_index(&client, &database, &collection, index_doc)?;
                    manager.drop_index(&client, &database, &collection, &old_name)?;
                }
                Ok::<(), crate::error::Error>(())
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
