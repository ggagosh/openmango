//! Command helpers for async operations + event emission.

use chrono::Utc;
use gpui::{App, AppContext as _, Entity};
use mongodb::{
    IndexModel,
    bson::{Document, doc},
};
use uuid::Uuid;

use crate::bson::DocumentKey;
use crate::connection::{FindDocumentsOptions, get_connection_manager};
use crate::models::ActiveConnection;
use crate::state::View;
use crate::state::{
    AppEvent, AppState, CollectionOverview, CollectionStats, DatabaseKey, DatabaseStats,
    SessionDocument, SessionKey, StatusMessage,
};

pub struct AppCommands;

impl AppCommands {
    fn ensure_writable(state: &Entity<AppState>, cx: &mut App) -> bool {
        let read_only =
            state.read(cx).conn.active.as_ref().map(|conn| conn.config.read_only).unwrap_or(false);
        if read_only {
            state.update(cx, |state, cx| {
                state.status_message =
                    Some(StatusMessage::error("Read-only connection: writes are disabled."));
                cx.notify();
            });
        }
        !read_only
    }

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
                            state.conn.active = Some(ActiveConnection {
                                config: saved.clone(),
                                client,
                                databases: databases.clone(),
                                collections: std::collections::HashMap::new(),
                            });
                            state.update_connection(saved, cx);
                            state.reset_runtime_state();
                            state.current_view = View::Databases;
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

    /// Disconnect the active connection and reset runtime state.
    pub fn disconnect(state: Entity<AppState>, cx: &mut App) {
        let connection_id = state.read(cx).conn.active.as_ref().map(|conn| conn.config.id);

        let Some(connection_id) = connection_id else {
            return;
        };

        state.update(cx, |state, cx| {
            state.conn.active = None;
            state.reset_runtime_state();
            state.current_view = View::Welcome;
            state.update_workspace_from_state();
            let event = AppEvent::Disconnected(connection_id);
            state.update_status_from_event(&event);
            cx.emit(event);
            cx.emit(AppEvent::ViewChanged);
            cx.notify();
        });
    }

    /// Refresh databases for the active connection.
    pub fn refresh_databases(state: Entity<AppState>, cx: &mut App) {
        let (client, connection_id) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), conn.config.id)
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
                                let Some(conn) = state.conn.active.as_mut() else {
                                    return;
                                };
                                if conn.config.id != connection_id {
                                    return;
                                }

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
                                state.close_tabs_for_database(db, cx);
                            }

                            if state
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

    /// Create a collection.
    pub fn create_collection(
        state: Entity<AppState>,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let client = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            conn.client.clone()
        };

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.create_collection(&client, &database, &collection)
            }
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
                            let Some(conn) = state.conn.active.as_mut() else {
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
                            state.status_message = Some(StatusMessage::info(format!(
                                "Created collection {database}.{collection}"
                            )));
                            cx.emit(AppEvent::DatabasesLoaded(conn.databases.clone()));
                            cx.emit(AppEvent::CollectionsLoaded(entry.clone()));
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to create collection: {}", e);
                        state.update(cx, |state, cx| {
                            state.status_message = Some(StatusMessage::error(format!(
                                "Create collection failed: {e}"
                            )));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Create a database by creating an initial collection.
    pub fn create_database(
        state: Entity<AppState>,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        Self::create_collection(state, database, collection, cx);
    }

    /// Rename a collection.
    pub fn rename_collection(
        state: Entity<AppState>,
        database: String,
        from: String,
        to: String,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        if from == to {
            return;
        }

        let (client, connection_id) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), conn.config.id)
        };

        let task = cx.background_spawn({
            let database = database.clone();
            let from = from.clone();
            let to = to.clone();
            async move {
                let manager = get_connection_manager();
                manager.rename_collection(&client, &database, &from, &to)
            }
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
                            let selection_changed = state
                                .conn
                                .selected_database
                                .as_ref()
                                .is_some_and(|selected| selected == &database)
                                && state
                                    .conn
                                    .selected_collection
                                    .as_ref()
                                    .is_some_and(|selected| selected == &from);

                            let collections = {
                                let Some(conn) = state.conn.active.as_mut() else {
                                    return;
                                };
                                if conn.config.id != connection_id {
                                    return;
                                }

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
                                state.conn.selected_collection = Some(to.clone());
                                cx.emit(AppEvent::ViewChanged);
                            }

                            let event = AppEvent::CollectionsLoaded(collections);
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            state.status_message = Some(StatusMessage::info(format!(
                                "Renamed collection {database}.{from} → {to}"
                            )));
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to rename collection: {}", e);
                        state.update(cx, |state, cx| {
                            state.status_message = Some(StatusMessage::error(format!(
                                "Rename collection failed: {e}"
                            )));
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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let client = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            conn.client.clone()
        };

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.drop_collection(&client, &database, &collection)
            }
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
                            if let Some(conn) = state.conn.active.as_mut()
                                && let Some(entry) = conn.collections.get_mut(&database)
                            {
                                entry.retain(|name| name != &collection);
                            }
                            state.close_tabs_for_collection(&database, &collection, cx);
                            state.status_message = Some(StatusMessage::info(format!(
                                "Dropped collection {database}.{collection}"
                            )));
                            let collections = state
                                .conn
                                .active
                                .as_ref()
                                .and_then(|conn| conn.collections.get(&database))
                                .cloned()
                                .unwrap_or_default();
                            cx.emit(AppEvent::CollectionsLoaded(collections));
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to drop collection: {}", e);
                        state.update(cx, |state, cx| {
                            state.status_message =
                                Some(StatusMessage::error(format!("Drop collection failed: {e}")));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Drop a database.
    pub fn drop_database(state: Entity<AppState>, database: String, cx: &mut App) {
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let client = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            conn.client.clone()
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
                            if let Some(conn) = state.conn.active.as_mut() {
                                conn.databases.retain(|db| db != &database);
                                conn.collections.remove(&database);
                            }
                            state.close_tabs_for_database(&database, cx);
                            if state.conn.selected_database.as_ref() == Some(&database) {
                                state.conn.selected_database = None;
                                state.conn.selected_collection = None;
                                state.current_view = View::Databases;
                                cx.emit(AppEvent::ViewChanged);
                            }
                            state.status_message =
                                Some(StatusMessage::info(format!("Dropped database {database}")));
                            let databases = state
                                .conn
                                .active
                                .as_ref()
                                .map(|conn| conn.databases.clone())
                                .unwrap_or_default();
                            cx.emit(AppEvent::DatabasesLoaded(databases));
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to drop database: {}", e);
                        state.update(cx, |state, cx| {
                            state.status_message =
                                Some(StatusMessage::error(format!("Drop database failed: {e}")));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Load collections for a database.
    pub fn load_collections(state: Entity<AppState>, database: String, cx: &mut App) {
        // Get active client
        let client = {
            let state = state.read(cx);
            match &state.conn.active {
                Some(conn) => conn.client.clone(),
                None => return,
            }
        };

        // Run blocking MongoDB operation in background thread
        let task = cx.background_spawn({
            let database = database.clone();
            async move {
                let manager = get_connection_manager();
                manager.list_collections(&client, &database)
            }
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
                            if let Some(ref mut conn) = state.conn.active {
                                conn.collections.insert(database.clone(), collections.clone());
                            }
                            let event = AppEvent::CollectionsLoaded(collections.clone());
                            state.update_status_from_event(&event);
                            cx.emit(event);
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

    /// Load database stats + collection overviews for a database tab.
    pub fn load_database_overview(
        state: Entity<AppState>,
        database_key: DatabaseKey,
        force: bool,
        cx: &mut App,
    ) {
        let (client, database) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            if conn.config.id != database_key.connection_id {
                return;
            }
            (conn.client.clone(), database_key.database.clone())
        };

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
                                    if let Some(conn) = state.conn.active.as_mut() {
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
                            state.status_message = Some(StatusMessage::error(message));
                        }

                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    /// Load documents for a collection session with pagination.
    pub fn load_documents_for_session(
        state: Entity<AppState>,
        session_key: SessionKey,
        cx: &mut App,
    ) {
        // Get active client and selected db/collection
        let (
            client,
            database,
            collection,
            skip,
            limit,
            request_id,
            filter,
            sort,
            projection,
            sort_raw,
        ) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            if conn.config.id != session_key.connection_id {
                return;
            }
            let (page, per_page, request_id, filter, sort, projection, sort_raw) =
                match state.sessions.get(&session_key) {
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
                conn.client.clone(),
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

    /// Load collection stats for a session.
    pub fn load_collection_stats(state: Entity<AppState>, session_key: SessionKey, cx: &mut App) {
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

        state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            session.data.stats_loading = true;
            session.data.stats_error = None;
            cx.notify();
        });

        let task = cx.background_spawn({
            let database = database.clone();
            let collection = collection.clone();
            async move {
                let manager = get_connection_manager();
                manager.collection_stats(&client, &database, &collection)
            }
        });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Document, crate::error::Error> = task.await;
                let _ = cx.update(|cx| match result {
                    Ok(stats_doc) => {
                        let stats = CollectionStats::from_document(&stats_doc);
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.stats = Some(stats);
                                session.data.stats_loading = false;
                            }
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to load collection stats: {}", e);
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.stats_loading = false;
                                session.data.stats_error = Some(e.to_string());
                            }
                            state.status_message =
                                Some(StatusMessage::error(format!("Stats failed: {e}")));
                            cx.notify();
                        });
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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            if conn.config.id != session_key.connection_id {
                return;
            }
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let count = documents.len();
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            if conn.config.id != session_key.connection_id {
                return;
            }
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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

    /// Load indexes for a collection session.
    pub fn load_collection_indexes(
        state: Entity<AppState>,
        session_key: SessionKey,
        force: bool,
        cx: &mut App,
    ) {
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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

    /// Save a document by replacing it in MongoDB.
    pub fn save_document(
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        updated: Document,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection, original_id, doc_index) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            let Some(index) = state.document_index(&session_key, &doc_key) else {
                return;
            };
            let Some(original) = state.document_for_key(&session_key, &doc_key) else {
                return;
            };
            let Some(id) = original.get("_id") else {
                return;
            };

            (
                conn.client.clone(),
                session_key.database.clone(),
                session_key.collection.clone(),
                id.clone(),
                index,
            )
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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection, id) = {
            let state_ref = state.read(cx);
            let Some(conn) = &state_ref.conn.active else {
                return;
            };
            let Some(doc) = state_ref
                .session(&session_key)
                .and_then(|session| session.view.drafts.get(&doc_key).cloned())
                .or_else(|| state_ref.document_for_key(&session_key, &doc_key))
            else {
                return;
            };
            let id = doc.get("_id").cloned();
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone(), id)
        };

        let Some(id) = id else {
            state.update(cx, |state, cx| {
                state.status_message =
                    Some(StatusMessage::error("Document missing _id; cannot update."));
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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection, original_id) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            let Some(original) = state.document_for_key(&session_key, &doc_key) else {
                return;
            };
            let Some(id) = original.get("_id") else {
                return;
            };

            (
                conn.client.clone(),
                session_key.database.clone(),
                session_key.collection.clone(),
                id.clone(),
            )
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
        if !Self::ensure_writable(&state, cx) {
            return;
        }
        let (client, database, collection) = {
            let state = state.read(cx);
            let Some(conn) = &state.conn.active else {
                return;
            };
            (conn.client.clone(), session_key.database.clone(), session_key.collection.clone())
        };

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
