use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::state::{AppEvent, AppState, StatusMessage};

use super::AppCommands;

impl AppCommands {
    /// Create a collection.
    pub fn create_collection(
        state: Entity<AppState>,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
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
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { manager.create_collection(&client, &database, &collection) }
        });

        cx.spawn({
            let state = state.clone();
            let database = database.clone();
            let collection = collection.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let Some(conn_id) = connection_id else {
                                return;
                            };
                            let (databases, collections) = {
                                let Some(conn) = state.active_connection_mut(conn_id) else {
                                    return;
                                };
                                if !conn.databases.contains(&database) {
                                    conn.databases.push(database.clone());
                                    conn.databases.sort();
                                }
                                let entry = conn.collections.entry(database.clone()).or_default();
                                if !entry.contains(&collection) {
                                    entry.push(collection.clone());
                                    entry.sort();
                                }
                                (conn.databases.clone(), entry.clone())
                            };

                            state.set_status_message(Some(StatusMessage::info(format!(
                                "Created collection {database}.{collection}"
                            ))));
                            if state.selected_connection_is(conn_id) {
                                cx.emit(AppEvent::DatabasesLoaded(databases));
                                cx.emit(AppEvent::CollectionsLoaded(collections));
                            }
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to create collection: {}", e);
                        state.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Create collection failed: {e}"
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Rename a collection.
    pub fn rename_collection(
        state: Entity<AppState>,
        database: String,
        from: String,
        to: String,
        cx: &mut App,
    ) {
        let connection_id = state.read(cx).selected_connection_id();
        if !Self::ensure_writable(&state, connection_id, cx) {
            return;
        }
        if from == to {
            return;
        }

        let Some(conn_id) = connection_id else {
            return;
        };
        let Some(client) = Self::active_client(&state, conn_id, cx) else {
            return;
        };
        let connection_id = conn_id;
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let from = from.clone();
            let to = to.clone();
            async move { manager.rename_collection(&client, &database, &from, &to) }
        });

        cx.spawn({
            let state = state.clone();
            let database = database.clone();
            let from = from.clone();
            let to = to.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let selection_changed = state.selected_connection_id()
                                == Some(connection_id)
                                && state
                                    .selected_database()
                                    .is_some_and(|selected| selected == database.as_str())
                                && state
                                    .selected_collection()
                                    .is_some_and(|selected| selected == from.as_str());

                            let collections = {
                                let Some(conn) = state.active_connection_mut(connection_id) else {
                                    return;
                                };

                                if let Some(entry) = conn.collections.get_mut(&database)
                                    && let Some(pos) = entry.iter().position(|name| name == &from)
                                {
                                    entry[pos] = to.clone();
                                    entry.sort();
                                }

                                conn.collections.get(&database).cloned().unwrap_or_default()
                            };

                            state.rename_collection_keys(connection_id, &database, &from, &to);

                            if selection_changed {
                                state.set_selected_collection_name(Some(to.clone()));
                                cx.emit(AppEvent::ViewChanged);
                            }

                            if state.selected_connection_is(connection_id) {
                                let event = AppEvent::CollectionsLoaded(collections);
                                state.update_status_from_event(&event);
                                cx.emit(event);
                            }
                            state.set_status_message(Some(StatusMessage::info(format!(
                                "Renamed collection {database}.{from} → {to}"
                            ))));
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to rename collection: {}", e);
                        state.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Rename collection failed: {e}"
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Drop a collection.
    pub fn drop_collection(
        state: Entity<AppState>,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
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
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move { manager.drop_collection(&client, &database, &collection) }
        });

        cx.spawn({
            let state = state.clone();
            let database = database.clone();
            let collection = collection.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let Some(conn_id) = connection_id else {
                                return;
                            };
                            if let Some(conn) = state.active_connection_mut(conn_id)
                                && let Some(entry) = conn.collections.get_mut(&database)
                            {
                                entry.retain(|name| name != &collection);
                            }
                            state.close_tabs_for_collection(conn_id, &database, &collection, cx);
                            state.set_status_message(Some(StatusMessage::info(format!(
                                "Dropped collection {database}.{collection}"
                            ))));
                            if state.selected_connection_is(conn_id) {
                                let collections = state
                                    .active_connection_by_id(conn_id)
                                    .and_then(|conn| conn.collections.get(&database))
                                    .cloned()
                                    .unwrap_or_default();
                                cx.emit(AppEvent::CollectionsLoaded(collections));
                            }
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to drop collection: {}", e);
                        state.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Drop collection failed: {e}"
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Load collections for a database.
    pub fn load_collections(
        state: Entity<AppState>,
        connection_id: Uuid,
        database: String,
        cx: &mut App,
    ) {
        // Get active client
        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            return;
        };
        let manager = state.read(cx).connection_manager();

        // Run blocking MongoDB operation in background thread
        let task = cx.background_spawn({
            let database = database.clone();
            async move { manager.list_collections(&client, &database) }
        });

        // Handle result on main thread
        cx.spawn({
            let state = state.clone();
            let database = database.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Vec<String>, crate::error::Error> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok(collections) => {
                        state.update(cx, |state, cx| {
                            if let Some(conn) = state.active_connection_mut(connection_id) {
                                conn.collections.insert(database.clone(), collections.clone());
                            }
                            if state.selected_connection_is(connection_id) {
                                let event = AppEvent::CollectionsLoaded(collections.clone());
                                state.update_status_from_event(&event);
                                cx.emit(event);
                            }
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        let error = e.to_string();
                        log::error!("Failed to load collections: {}", error);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::CollectionsFailed(error);
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
