use chrono::Utc;
use gpui::{App, AppContext as _, Entity};
use uuid::Uuid;

use crate::connection::get_connection_manager;
use crate::models::ActiveConnection;
use crate::state::{AppEvent, AppState, StatusMessage, View};

use super::AppCommands;

impl AppCommands {
    /// Connect to a saved connection by ID.
    pub fn connect(state: Entity<AppState>, connection_id: Uuid, cx: &mut App) {
        // Find the connection config
        let saved = {
            let state = state.read(cx);
            state.connections.iter().find(|c| c.id == connection_id).cloned()
        };

        let Some(saved) = saved else {
            state.update(cx, |_, cx| {
                cx.emit(AppEvent::ConnectionFailed("Connection not found".to_string()));
            });
            return;
        };

        // Emit connecting event
        state.update(cx, |state, cx| {
            let event = AppEvent::Connecting(connection_id);
            state.update_status_from_event(&event);
            cx.emit(event);
        });

        // Run blocking MongoDB operations in background thread
        let task = cx.background_spawn({
            let saved = saved.clone();
            async move {
                let manager = get_connection_manager();

                // Connect (blocking, runs in Tokio runtime internally)
                let client = manager.connect(&saved)?;

                // List databases (blocking, runs in Tokio runtime internally)
                let databases = manager.list_databases(&client)?;

                Ok::<_, crate::error::Error>((client, databases))
            }
        });

        // Handle result on main thread
        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<(mongodb::Client, Vec<String>), crate::error::Error> =
                    task.await;

                let _ = cx.update(|cx| match result {
                    Ok((client, databases)) => {
                        state.update(cx, |state, cx| {
                            let mut saved = saved.clone();
                            saved.last_connected = Some(Utc::now());
                            state.conn.active.insert(
                                connection_id,
                                ActiveConnection {
                                    config: saved.clone(),
                                    client,
                                    databases: databases.clone(),
                                    collections: std::collections::HashMap::new(),
                                },
                            );
                            state.update_connection(saved, cx);
                            state.select_connection(Some(connection_id), cx);
                            state.update_workspace_from_state();
                            let connected = AppEvent::Connected(connection_id);
                            state.update_status_from_event(&connected);
                            cx.emit(connected);

                            let loaded = AppEvent::DatabasesLoaded(databases.clone());
                            state.update_status_from_event(&loaded);
                            cx.emit(loaded);
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to connect: {}", e);
                        state.update(cx, |state, cx| {
                            let event = AppEvent::ConnectionFailed(e.to_string());
                            state.update_status_from_event(&event);
                            cx.emit(event);
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Disconnect a connection and reset its runtime state.
    pub fn disconnect(state: Entity<AppState>, connection_id: Uuid, cx: &mut App) {
        if !state.read(cx).conn.active.contains_key(&connection_id) {
            return;
        }

        state.update(cx, |state, cx| {
            state.conn.active.remove(&connection_id);
            state.reset_connection_runtime_state(connection_id, cx);
            if state.conn.selected_connection == Some(connection_id) {
                state.current_view = View::Welcome;
            }
            state.update_workspace_from_state();
            let event = AppEvent::Disconnected(connection_id);
            state.update_status_from_event(&event);
            cx.emit(event);
            cx.emit(AppEvent::ViewChanged);
            cx.notify();
        });
    }

    /// Refresh databases for the selected connection.
    pub fn refresh_databases(state: Entity<AppState>, connection_id: Uuid, cx: &mut App) {
        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            return;
        };

        let task = cx.background_spawn(async move {
            let manager = get_connection_manager();
            manager.list_databases(&client)
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Vec<String>, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(databases) => {
                        state.update(cx, |state, cx| {
                            let removed = {
                                let Some(conn) = state.conn.active.get_mut(&connection_id) else {
                                    return;
                                };

                                let removed: Vec<String> = conn
                                    .databases
                                    .iter()
                                    .filter(|db| !databases.contains(db))
                                    .cloned()
                                    .collect();
                                conn.databases = databases.clone();
                                conn.databases.sort();
                                conn.collections.retain(|db, _| conn.databases.contains(db));
                                removed
                            };

                            for db in &removed {
                                state.close_tabs_for_database(connection_id, db, cx);
                            }

                            if state.conn.selected_connection == Some(connection_id)
                                && state
                                    .conn
                                    .selected_database
                                    .as_ref()
                                    .is_some_and(|selected| !databases.contains(selected))
                            {
                                state.conn.selected_database = None;
                                state.conn.selected_collection = None;
                                state.current_view = View::Databases;
                                cx.emit(AppEvent::ViewChanged);
                            }

                            let event = AppEvent::DatabasesLoaded(databases.clone());
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to refresh databases: {}", e);
                        state.update(cx, |state, cx| {
                            state.status_message = Some(StatusMessage::error(format!(
                                "Refresh databases failed: {e}"
                            )));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }
}
