use gpui::{App, AppContext as _, Entity};

use crate::connection::get_connection_manager;
use crate::state::{
    AppEvent, AppState, CollectionOverview, DatabaseKey, DatabaseStats, StatusMessage, View,
};

use super::AppCommands;

impl AppCommands {
    /// Create a database by creating an initial collection.
    pub fn create_database(
        state: Entity<AppState>,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
        let connection_id = state.read(cx).selected_connection_id();
        if !Self::ensure_writable(&state, connection_id, cx) {
            return;
        }
        Self::create_collection(state, database, collection, cx);
    }

    /// Drop a database.
    pub fn drop_database(state: Entity<AppState>, database: String, cx: &mut App) {
        let connection_id = state.read(cx).selected_connection_id();
        if !Self::ensure_writable(&state, connection_id, cx) {
            return;
        }
        let Some(conn_id) = connection_id else {
            return;
        };
        let Some(client) = Self::active_client(&state, conn_id, cx) else {
            return;
        };

        let task = cx.background_spawn({
            let database = database.clone();
            async move {
                let manager = get_connection_manager();
                manager.drop_database(&client, &database)
            }
        });

        cx.spawn({
            let state = state.clone();
            let database = database.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let Some(conn_id) = connection_id else {
                                return;
                            };
                            if let Some(conn) = state.active_connection_mut(conn_id) {
                                conn.databases.retain(|db| db != &database);
                                conn.collections.remove(&database);
                            }
                            state.close_tabs_for_database(conn_id, &database, cx);
                            if state.selected_connection_is(conn_id)
                                && state.selected_database() == Some(database.as_str())
                            {
                                state.set_selected_database_name(None);
                                state.set_selected_collection_name(None);
                                state.current_view = View::Databases;
                                cx.emit(AppEvent::ViewChanged);
                            }
                            state.set_status_message(Some(StatusMessage::info(format!(
                                "Dropped database {database}"
                            ))));
                            if state.selected_connection_is(conn_id) {
                                let databases = state
                                    .active_connection_by_id(conn_id)
                                    .map(|conn| conn.databases.clone())
                                    .unwrap_or_default();
                                cx.emit(AppEvent::DatabasesLoaded(databases));
                            }
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to drop database: {}", e);
                        state.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Drop database failed: {e}"
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Load database stats + collection overviews for a database tab.
    pub fn load_database_overview(
        state: Entity<AppState>,
        database_key: DatabaseKey,
        force: bool,
        cx: &mut App,
    ) {
        let Some(client) = Self::active_client(&state, database_key.connection_id, cx) else {
            return;
        };
        let database = database_key.database.clone();

        let should_load = {
            let state = state.read(cx);
            if force {
                true
            } else if let Some(session) = state.database_session(&database_key) {
                !(session.data.stats_loading
                    || session.data.collections_loading
                    || (session.data.stats.is_some()
                        && session.data.stats_error.is_none()
                        && session.data.collections_error.is_none()))
            } else {
                true
            }
        };

        if !should_load {
            return;
        }

        state.update(cx, |state, cx| {
            let session = state.ensure_database_session(database_key.clone());
            session.data.stats_loading = true;
            session.data.stats_error = None;
            session.data.collections_loading = true;
            session.data.collections_error = None;
            cx.notify();
        });

        let task = cx.background_spawn({
            let database = database.clone();
            async move {
                let manager = get_connection_manager();
                let stats_result = manager
                    .database_stats(&client, &database)
                    .map(|doc| DatabaseStats::from_document(&doc));
                let collections_result =
                    manager.list_collection_specs(&client, &database).map(|specs| {
                        let names = specs.iter().map(|spec| spec.name.clone()).collect::<Vec<_>>();
                        let collections = specs
                            .into_iter()
                            .map(CollectionOverview::from_spec)
                            .collect::<Vec<_>>();
                        (names, collections)
                    });

                (stats_result, collections_result)
            }
        });

        cx.spawn({
            let state = state.clone();
            let database = database.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let (stats_result, collections_result) = task.await;

                let _ = cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        let mut status_message = None;
                        {
                            let session = state.ensure_database_session(database_key.clone());
                            session.data.stats_loading = false;
                            session.data.collections_loading = false;

                            match stats_result {
                                Ok(stats) => {
                                    session.data.stats = Some(stats);
                                    session.data.stats_error = None;
                                }
                                Err(err) => {
                                    session.data.stats_error = Some(err.to_string());
                                    status_message = Some(format!("Database stats failed: {err}"));
                                }
                            }

                            match collections_result {
                                Ok((names, collections)) => {
                                    session.data.collections = collections;
                                    session.data.collections_error = None;
                                    if let Some(conn) =
                                        state.active_connection_mut(database_key.connection_id)
                                    {
                                        conn.collections.insert(database.clone(), names);
                                    }
                                }
                                Err(err) => {
                                    session.data.collections_error = Some(err.to_string());
                                    status_message =
                                        Some(format!("Database collections failed: {err}"));
                                }
                            }
                        }

                        if let Some(message) = status_message {
                            state.set_status_message(Some(StatusMessage::error(message)));
                        }

                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }
}
