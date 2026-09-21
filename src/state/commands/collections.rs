use gpui_kit::{App, AppContext as _, Entity};
use mongodb::bson::Document;
use uuid::Uuid;

use crate::connection::manager::ViewDefinition;
use crate::error::ErrorReport;
use crate::models::CollectionDetail;
use crate::state::{AppEvent, AppState, CollectionSubview, StatusMessage};

use super::AppCommands;

/// Where a new view's definition comes from.
pub enum ViewSource {
    /// A pipeline over a collection, as built on the aggregation screen.
    Pipeline { view_on: String, pipeline: Vec<Document>, collation: Option<Document> },
    /// Another view in the same database, as the server defines it right now.
    CopyOf(String),
}

/// One request to create a view, or with `replace` to redefine the view of that name.
pub struct ViewSave {
    pub connection_id: Uuid,
    pub database: String,
    pub name: String,
    pub source: ViewSource,
    pub replace: bool,
}

impl AppCommands {
    /// Create a collection.
    /// `on_done` gets the outcome, so the dialog that asked can close or show why it failed.
    pub fn create_collection(
        state: Entity<AppState>,
        database: String,
        collection: String,
        cx: &mut App,
        on_done: impl FnOnce(Result<(), ErrorReport>, &mut App) + 'static,
    ) {
        let connection_id = state.read(cx).selected_connection_id();
        let Some(conn_id) =
            connection_id.filter(|_| Self::ensure_writable(&state, connection_id, cx))
        else {
            on_done(Err(not_writable("Couldn't create the collection")), cx);
            return;
        };
        Self::create_collection_authorized(state, conn_id, database, collection, cx, on_done);
    }

    pub(super) fn create_collection_authorized(
        state: Entity<AppState>,
        conn_id: Uuid,
        database: String,
        collection: String,
        cx: &mut App,
        on_done: impl FnOnce(Result<(), ErrorReport>, &mut App) + 'static,
    ) {
        let connection_id = Some(conn_id);
        let Some(client) = Self::active_client(&state, conn_id, cx) else {
            on_done(Err(not_connected("Couldn't create the collection")), cx);
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
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                cx.update(|cx| match result {
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
                        on_done(Ok(()), cx);
                    }
                    Err(e) => {
                        log::error!("Failed to create collection: {e:?}");
                        let report = ErrorReport::from_error("Couldn't create the collection", &e);
                        state.update(cx, |state, cx| {
                            // The create dialog stays open and shows this.
                            state.record_error(report.clone());
                            cx.notify();
                        });
                        on_done(Err(report), cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Create a view, or with `replace` change an existing one. The caller has already asked
    /// for the write; `on_done` lets the dialog that asked close or show why it failed.
    pub fn save_view(
        state: Entity<AppState>,
        save: ViewSave,
        cx: &mut App,
        on_done: impl FnOnce(Result<(), ErrorReport>, &mut App) + 'static,
    ) {
        let ViewSave { connection_id, database, name, source, replace } = save;
        let title = if replace { "Couldn't update the view" } else { "Couldn't create the view" };
        if !Self::ensure_writable(&state, Some(connection_id), cx) {
            on_done(Err(not_writable(title)), cx);
            return;
        }
        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            on_done(Err(not_connected(title)), cx);
            return;
        };
        let manager = state.read(cx).connection_manager();

        let task = cx.background_spawn({
            let database = database.clone();
            let name = name.clone();
            async move {
                let definition = match source {
                    ViewSource::Pipeline { view_on, pipeline, collation } => {
                        ViewDefinition { name, view_on, pipeline, collation }
                    }
                    // Read now, not from what the sidebar last listed: a copy of a stale
                    // definition would look right and be wrong.
                    ViewSource::CopyOf(view) => ViewDefinition {
                        name,
                        ..manager.view_definition(&client, &database, &view)?.ok_or_else(|| {
                            crate::error::Error::Parse(format!("{view} is no longer a view."))
                        })?
                    },
                };
                manager.save_view(&client, &database, &definition, replace)?;
                Ok::<_, crate::error::Error>(definition)
            }
        });

        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let result = task.await;
            cx.update(|cx| match result {
                Ok(definition) => {
                    state.update(cx, |state, cx| {
                        let name = definition.name.clone();
                        let Some(conn) = state.active_connection_mut(connection_id) else {
                            return;
                        };
                        let names = conn.collections.entry(database.clone()).or_default();
                        if !names.contains(&name) {
                            names.push(name.clone());
                            names.sort_unstable_by_key(|name| name.to_lowercase());
                        }
                        let names = names.clone();
                        conn.collection_details.entry(database.clone()).or_default().insert(
                            name.clone(),
                            CollectionDetail::View {
                                view_on: definition.view_on,
                                pipeline: definition.pipeline,
                            },
                        );
                        state.set_status_message(Some(StatusMessage::info(format!(
                            "{} view {database}.{name}",
                            if replace { "Updated" } else { "Created" }
                        ))));
                        if state.selected_connection_is(connection_id) {
                            cx.emit(AppEvent::CollectionsLoaded(names));
                            // Opening a new view is the confirmation, and it puts the sidebar
                            // on the row. An update stays on the builder, to keep iterating.
                            if !replace {
                                state.select_collection(database.clone(), name, cx);
                            }
                        }
                        cx.notify();
                    });
                    on_done(Ok(()), cx);
                }
                Err(e) => {
                    log::error!("Failed to save view: {e:?}");
                    let report = ErrorReport::from_error(title, &e);
                    state.update(cx, |state, cx| {
                        state.record_error(report.clone());
                        cx.notify();
                    });
                    on_done(Err(report), cx);
                }
            });
        })
        .detach();
    }

    /// Opens a view's definition for editing: its source collection in a tab of its own, on the
    /// aggregation screen, holding the pipeline the server has right now. Never the copy the
    /// sidebar listed earlier, which someone else may have changed since.
    pub fn edit_view_definition(
        state: Entity<AppState>,
        connection_id: Uuid,
        database: String,
        view: String,
        cx: &mut App,
    ) {
        const TITLE: &str = "Couldn't open the view's definition";
        let Some(client) = Self::active_client(&state, connection_id, cx) else {
            state.update(cx, |state, cx| {
                state.record_error(not_connected(TITLE));
                cx.notify();
            });
            return;
        };
        let manager = state.read(cx).connection_manager();
        let task = cx.background_spawn({
            let database = database.clone();
            let view = view.clone();
            async move { manager.view_definition(&client, &database, &view) }
        });

        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let result = task.await;
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    let definition = match result {
                        Ok(Some(definition)) => definition,
                        Ok(None) => {
                            state.record_error(ErrorReport::new(
                                TITLE,
                                format!(
                                    "{database}.{view} is no longer a view. Refresh the database."
                                ),
                            ));
                            cx.notify();
                            return;
                        }
                        Err(e) => {
                            state.record_error(ErrorReport::from_error(TITLE, &e));
                            cx.notify();
                            return;
                        }
                    };
                    if !state.selected_connection_is(connection_id) {
                        return;
                    }
                    let Some(key) = state.open_collection_in_new_tab(
                        database.clone(),
                        definition.view_on.clone(),
                        String::new(),
                        None,
                        cx,
                    ) else {
                        return;
                    };
                    state.set_collection_subview(&key, CollectionSubview::Aggregation);
                    state.replace_pipeline_stages(
                        &key,
                        crate::state::app_state::stages_from_pipeline(&definition.pipeline),
                    );
                    if let Some(session) = state.session_mut(&key) {
                        session.data.aggregation.editing_view = Some(view.clone());
                    }
                    if let Some(conn) = state.active_connection_mut(connection_id) {
                        conn.collection_details.entry(database.clone()).or_default().insert(
                            view.clone(),
                            CollectionDetail::View {
                                view_on: definition.view_on,
                                pipeline: definition.pipeline,
                            },
                        );
                    }
                    cx.notify();
                });
            });
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
        on_done: impl FnOnce(Result<(), ErrorReport>, &mut App) + 'static,
    ) {
        let connection_id = state.read(cx).selected_connection_id();
        let Some(conn_id) =
            connection_id.filter(|_| Self::ensure_writable(&state, connection_id, cx))
        else {
            on_done(Err(not_writable("Couldn't rename the collection")), cx);
            return;
        };
        if from == to {
            on_done(Ok(()), cx);
            return;
        }
        let Some(client) = Self::active_client(&state, conn_id, cx) else {
            on_done(Err(not_connected("Couldn't rename the collection")), cx);
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
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                cx.update(|cx| match result {
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
                        on_done(Ok(()), cx);
                    }
                    Err(e) => {
                        log::error!("Failed to rename collection: {e:?}");
                        let report = ErrorReport::from_error("Couldn't rename the collection", &e);
                        state.update(cx, |state, cx| {
                            // The rename dialog stays open and shows this.
                            state.record_error(report.clone());
                            cx.notify();
                        });
                        on_done(Err(report), cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Drop a collection.
    pub fn drop_collection(
        state: Entity<AppState>,
        connection_id: Uuid,
        database: String,
        collection: String,
        cx: &mut App,
    ) {
        if !Self::ensure_writable(&state, Some(connection_id), cx) {
            return;
        }
        let Some(client) = Self::active_client(&state, connection_id, cx) else {
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
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result: Result<(), crate::error::Error> = task.await;
                cx.update(|cx| match result {
                    Ok(()) => {
                        state.update(cx, |state, cx| {
                            let mut kind = "collection";
                            if let Some(conn) = state.active_connection_mut(connection_id) {
                                if let Some(entry) = conn.collections.get_mut(&database) {
                                    entry.retain(|name| name != &collection);
                                }
                                // Or a collection later created under this name reads as a view.
                                if let Some(details) = conn.collection_details.get_mut(&database)
                                    && let Some(CollectionDetail::View { .. }) =
                                        details.remove(&collection)
                                {
                                    kind = "view";
                                }
                            }
                            state.close_tabs_for_collection(
                                connection_id,
                                &database,
                                &collection,
                                cx,
                            );
                            state.set_status_message(Some(StatusMessage::info(format!(
                                "Dropped {kind} {database}.{collection}"
                            ))));
                            if state.selected_connection_is(connection_id) {
                                let collections = state
                                    .active_connection_by_id(connection_id)
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
            async move {
                manager
                    .list_collection_specs(&client, &database)
                    .map(|specs| CollectionDetail::split_specs(&specs))
            }
        });

        // Handle result on main thread
        cx.spawn({
            let state = state.clone();
            let database = database.clone();
            async move |cx: &mut gpui_kit::AsyncApp| {
                let result = task.await;

                cx.update(|cx| match result {
                    Ok((collections, details)) => {
                        state.update(cx, |state, cx| {
                            if let Some(conn) = state.active_connection_mut(connection_id) {
                                conn.collections.insert(database.clone(), collections.clone());
                                conn.collection_details.insert(database.clone(), details);
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

fn not_writable(title: &str) -> ErrorReport {
    ErrorReport::new(title, "This connection doesn't allow writes right now.")
}

fn not_connected(title: &str) -> ErrorReport {
    ErrorReport::new(title, "The connection isn't active. Connect and try again.")
        .kind(crate::error::ErrorKind::Connection)
}
