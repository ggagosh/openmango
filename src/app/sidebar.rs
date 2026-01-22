use std::collections::{HashMap, HashSet};

use gpui::prelude::{FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _};
use gpui::*;
use gpui_component::input::{Input, InputState};
use gpui_component::menu::ContextMenuExt;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::{Icon, IconName, Sizable as _};
use uuid::Uuid;

use crate::components::{ConnectionDialog, ConnectionManager, TreeNodeId, open_confirm_dialog};
use crate::keyboard::{
    CloseSidebarSearch, CopyConnectionUri, CopySelectionName, DeleteSelection,
    DisconnectConnection, EditConnection, FindInSidebar, OpenSelection, OpenSelectionPreview,
    RenameCollection,
};
use crate::state::{AppCommands, AppEvent, AppState};
use crate::theme::{borders, colors, sizing, spacing};

use super::dialogs::open_rename_collection_dialog;
use super::menus::{build_collection_menu, build_connection_menu, build_database_menu};
use super::search::{SidebarSearchResult, search_results};
use super::sidebar_model::SidebarModel;

// =============================================================================
// Sidebar Component
// =============================================================================

pub(crate) struct Sidebar {
    state: Entity<AppState>,
    model: SidebarModel,
    search_state: Entity<InputState>,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Sidebar {
    pub(crate) fn new(
        state: Entity<AppState>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (connections, active) = {
            let state_ref = state.read(cx);
            (state_ref.connections_snapshot(), state_ref.active_connections_snapshot())
        };
        let model = SidebarModel::new(connections, active);
        let search_state =
            cx.new(|cx| InputState::new(_window, cx).placeholder("Search databases"));

        let mut subscriptions = vec![];

        // Subscribe to AppState events for targeted tree updates (Phase 5.5)
        subscriptions.push(cx.subscribe(&state, move |this, _, event, cx| match event {
            AppEvent::ConnectionAdded
            | AppEvent::ConnectionUpdated
            | AppEvent::ConnectionRemoved
            | AppEvent::DatabasesLoaded(_) => {
                this.refresh_tree(cx);
            }
            AppEvent::CollectionsLoaded(_) => {
                this.model.loading_databases.clear();
                this.refresh_tree(cx);
            }
            AppEvent::CollectionsFailed(_) => {
                this.model.loading_databases.clear();
                cx.notify();
            }
            AppEvent::Connecting(connection_id) => {
                this.model.connecting_connection = Some(*connection_id);
                cx.notify();
            }
            AppEvent::Connected(connection_id) => {
                if this.model.connecting_connection == Some(*connection_id) {
                    this.model.connecting_connection = None;
                }
                this.model.loading_databases.clear();
                if this.state.read(cx).workspace_restore_pending
                    && this.state.read(cx).workspace.last_connection_id == Some(*connection_id)
                {
                    this.state.update(cx, |state, cx| {
                        state.restore_workspace_after_connect(cx);
                    });
                    this.restore_workspace_expansion(cx);
                }
                this.refresh_tree(cx);
            }
            AppEvent::Disconnected(connection_id) => {
                if this.model.connecting_connection == Some(*connection_id) {
                    this.model.connecting_connection = None;
                }
                this.model.loading_databases.clear();
                this.model.selected_tree_id = None;
                this.refresh_tree(cx);
            }
            AppEvent::ConnectionFailed(_) => {
                this.model.connecting_connection = None;
                this.model.loading_databases.clear();
                this.model.selected_tree_id = None;
                cx.notify();
            }
            AppEvent::DocumentsLoaded { .. }
            | AppEvent::DocumentInserted
            | AppEvent::DocumentInsertFailed { .. }
            | AppEvent::DocumentsInserted { .. }
            | AppEvent::DocumentsInsertFailed { .. }
            | AppEvent::DocumentSaved { .. }
            | AppEvent::DocumentSaveFailed { .. }
            | AppEvent::DocumentDeleted { .. }
            | AppEvent::DocumentDeleteFailed { .. }
            | AppEvent::DocumentsDeleted { .. }
            | AppEvent::DocumentsDeleteFailed { .. }
            | AppEvent::IndexesLoaded { .. }
            | AppEvent::IndexesLoadFailed { .. }
            | AppEvent::IndexDropped { .. }
            | AppEvent::IndexDropFailed { .. }
            | AppEvent::IndexCreated { .. }
            | AppEvent::IndexCreateFailed { .. }
            | AppEvent::DocumentsUpdated { .. }
            | AppEvent::DocumentsUpdateFailed { .. } => {}
            AppEvent::ViewChanged => {
                this.sync_selection_from_state(cx);
            }
        }));

        subscriptions.push(cx.observe_window_bounds(_window, |this, window, cx| {
            this.state.update(cx, |state, _cx| {
                state.set_workspace_window_bounds(window.window_bounds());
            });
        }));

        subscriptions.push(cx.observe(&search_state, |_, _, cx| cx.notify()));

        subscriptions.push(cx.on_window_closed({
            let state = state.clone();
            move |cx| {
                state.update(cx, |state, cx| {
                    state.workspace_restore_pending = false;
                    state.update_workspace_from_state();
                    cx.notify();
                });
            }
        }));

        subscriptions.push(cx.on_app_quit(|this, cx| {
            let state = this.state.clone();
            state.update(cx, |state, cx| {
                state.workspace_restore_pending = false;
                state.update_workspace_from_state();
                cx.notify();
            });
            async {}
        }));

        let sidebar = Self {
            state,
            model,
            search_state,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::default(),
            _subscriptions: subscriptions,
        };

        if let Some(connection_id) = sidebar.state.read(cx).workspace_autoconnect_id() {
            AppCommands::connect(sidebar.state.clone(), connection_id, cx);
        }

        sidebar
    }

    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        let (connections, active, selected_connection, selected_db, selected_col) = {
            let state_ref = self.state.read(cx);
            (
                state_ref.connections_snapshot(),
                state_ref.active_connections_snapshot(),
                state_ref.selected_connection_id(),
                state_ref.selected_database_name(),
                state_ref.selected_collection_name(),
            )
        };

        if self.model.selected_tree_id.is_none() {
            self.model.ensure_selection_from_state(selected_connection, selected_db, selected_col);
        }

        if let Some(ix) = self.model.refresh_entries(&connections, &active) {
            self.scroll_handle.scroll_to_item(ix, gpui::ScrollStrategy::Center);
        }
        cx.notify();
    }

    pub(crate) fn expand_connection_and_refresh(
        &mut self,
        connection_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.model.expanded_nodes.insert(TreeNodeId::connection(connection_id));
        self.persist_expanded_nodes(cx);
        self.refresh_tree(cx);
    }

    pub(crate) fn mark_database_loading(&mut self, node_id: TreeNodeId, cx: &mut Context<Self>) {
        self.model.loading_databases.insert(node_id);
        cx.notify();
    }

    fn sync_selection_from_state(&mut self, cx: &mut Context<Self>) {
        let (connection_id, selected_db, selected_col) = {
            let state_ref = self.state.read(cx);
            (
                state_ref.selected_connection_id(),
                state_ref.selected_database_name(),
                state_ref.selected_collection_name(),
            )
        };
        if connection_id.is_none() {
            return;
        }

        self.model.ensure_selection_from_state(connection_id, selected_db, selected_col);
        self.persist_expanded_nodes(cx);
        self.refresh_tree(cx);
    }

    fn persist_expanded_nodes(&mut self, cx: &mut Context<Self>) {
        let mut nodes: Vec<String> =
            self.model.expanded_nodes.iter().map(|id| id.to_tree_id()).collect();
        nodes.sort();
        self.state.update(cx, |state, _cx| {
            state.set_workspace_expanded_nodes(nodes);
        });
    }

    fn restore_workspace_expansion(&mut self, cx: &mut Context<Self>) {
        let (connection_id, selected_db, expanded) = {
            let state_ref = self.state.read(cx);
            let Some(connection_id) =
                state_ref.workspace.last_connection_id.or(state_ref.selected_connection_id())
            else {
                return;
            };
            let Some(_active) = state_ref.active_connection_by_id(connection_id) else {
                return;
            };
            let mut expanded: HashSet<TreeNodeId> = state_ref
                .workspace
                .expanded_nodes
                .iter()
                .filter_map(|id| TreeNodeId::from_tree_id(id))
                .filter(|node| node.connection_id() == connection_id)
                .collect();

            let selected_db = state_ref.selected_database_name();
            if let Some(db) = selected_db.as_ref() {
                expanded.insert(TreeNodeId::connection(connection_id));
                expanded.insert(TreeNodeId::database(connection_id, db));
            }

            (connection_id, selected_db, expanded)
        };

        self.model.expanded_nodes = expanded;
        if selected_db.is_some() {
            self.model.expanded_nodes.insert(TreeNodeId::connection(connection_id));
        }
        self.model.selected_tree_id = None;
        self.refresh_tree(cx);
        self.load_expanded_databases(cx);
    }

    fn load_expanded_databases(&mut self, cx: &mut Context<Self>) {
        for node in self.model.expanded_nodes.iter() {
            let TreeNodeId::Database { connection, database } = node else {
                continue;
            };
            let collections = {
                let state_ref = self.state.read(cx);
                let Some(conn) = state_ref.active_connection_by_id(*connection) else {
                    continue;
                };
                conn.collections.clone()
            };
            if collections.contains_key(database) || self.model.loading_databases.contains(node) {
                continue;
            }
            self.model.loading_databases.insert(node.clone());
            AppCommands::load_collections(self.state.clone(), *connection, database.clone(), cx);
        }
    }

    fn open_add_dialog(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        ConnectionDialog::open(state, window, cx);
    }

    fn handle_open_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.search_open {
            let query = self.search_state.read(cx).value().to_string();
            let results = self.search_results(&query, cx);
            let selection = self.model.search_selected;
            let result = selection.and_then(|ix| results.get(ix)).or_else(|| results.first());
            if let Some(result) = result {
                self.select_search_result(result, window, cx);
            }
            return;
        }
        let Some(node_id) = self.model.selected_tree_id.clone() else {
            return;
        };

        if node_id.is_connection() {
            let connection_id = node_id.connection_id();
            self.state.update(cx, |state, cx| {
                state.select_connection(Some(connection_id), cx);
            });
            let is_connected = self.state.read(cx).is_connected(connection_id);
            let is_connecting = self.model.connecting_connection == Some(connection_id);

            if !is_connected && !is_connecting {
                self.model.expanded_nodes.insert(node_id.clone());
                self.persist_expanded_nodes(cx);
                self.refresh_tree(cx);
                AppCommands::connect(self.state.clone(), connection_id, cx);
            }
            return;
        }

        if node_id.is_database() {
            let Some(db) = node_id.database_name().map(|db| db.to_string()) else {
                return;
            };
            self.state.update(cx, |state, cx| {
                state.select_connection(Some(node_id.connection_id()), cx);
                state.select_database(db.clone(), cx);
            });
            let should_expand = !self.model.expanded_nodes.contains(&node_id);
            if should_expand {
                self.model.expanded_nodes.insert(node_id.clone());
                self.persist_expanded_nodes(cx);
                self.refresh_tree(cx);
            }
            if should_expand && !self.model.loading_databases.contains(&node_id) {
                let should_load = self
                    .state
                    .read(cx)
                    .active_connection_by_id(node_id.connection_id())
                    .is_some_and(|conn| !conn.collections.contains_key(&db));
                if should_load {
                    self.model.loading_databases.insert(node_id.clone());
                    cx.notify();
                    AppCommands::load_collections(
                        self.state.clone(),
                        node_id.connection_id(),
                        db,
                        cx,
                    );
                }
            }
            return;
        }

        if node_id.is_collection()
            && let (Some(db), Some(col)) = (
                node_id.database_name().map(|db| db.to_string()),
                node_id.collection_name().map(|col| col.to_string()),
            )
        {
            self.state.update(cx, |state, cx| {
                state.select_connection(Some(node_id.connection_id()), cx);
                state.select_collection(db, col, cx);
            });
        }
    }

    fn handle_open_preview(&mut self, cx: &mut Context<Self>) {
        let Some(node_id) = self.model.selected_tree_id.clone() else {
            return;
        };
        if node_id.is_collection()
            && let (Some(db), Some(col)) = (
                node_id.database_name().map(|db| db.to_string()),
                node_id.collection_name().map(|col| col.to_string()),
            )
        {
            self.state.update(cx, |state, cx| {
                state.select_connection(Some(node_id.connection_id()), cx);
                state.preview_collection(db, col, cx);
            });
        }
    }

    fn handle_edit_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Connection(connection_id)) = self.model.selected_tree_id.clone()
        else {
            return;
        };
        ConnectionManager::open_selected(self.state.clone(), connection_id, window, cx);
    }

    fn handle_disconnect_connection(&mut self, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Connection(connection_id)) = self.model.selected_tree_id.clone()
        else {
            return;
        };
        let is_active = self.state.read(cx).is_connected(connection_id);
        if is_active {
            AppCommands::disconnect(self.state.clone(), connection_id, cx);
        }
    }

    fn handle_copy_selection_name(&mut self, cx: &mut Context<Self>) {
        let Some(node_id) = self.model.selected_tree_id.clone() else {
            return;
        };
        let text = match node_id {
            TreeNodeId::Connection(connection_id) => {
                self.state.read(cx).connection_name(connection_id)
            }
            TreeNodeId::Database { database, .. } => Some(database),
            TreeNodeId::Collection { database, collection, .. } => {
                Some(format!("{database}/{collection}"))
            }
        };
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn handle_copy_connection_uri(&mut self, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Connection(connection_id)) = self.model.selected_tree_id.clone()
        else {
            return;
        };
        if let Some(uri) = self.state.read(cx).connection_uri(connection_id) {
            cx.write_to_clipboard(ClipboardItem::new_string(uri));
        }
    }

    fn handle_rename_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Collection { database, collection, .. }) =
            self.model.selected_tree_id.clone()
        else {
            return;
        };
        open_rename_collection_dialog(self.state.clone(), database, collection, window, cx);
    }

    fn handle_delete_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node_id) = self.model.selected_tree_id.clone() else {
            return;
        };
        match node_id {
            TreeNodeId::Connection(connection_id) => {
                let name = self
                    .state
                    .read(cx)
                    .connection_name(connection_id)
                    .unwrap_or_else(|| "this connection".to_string());
                let message = format!("Remove connection \"{name}\"? This cannot be undone.");
                open_confirm_dialog(window, cx, "Remove connection", message, "Remove", true, {
                    let state = self.state.clone();
                    move |_window, cx| {
                        state.update(cx, |state, cx| {
                            state.remove_connection(connection_id, cx);
                        });
                    }
                });
            }
            TreeNodeId::Database { database, .. } => {
                let message = format!("Drop database \"{database}\"? This cannot be undone.");
                open_confirm_dialog(window, cx, "Drop database", message, "Drop", true, {
                    let state = self.state.clone();
                    let database = database.clone();
                    move |_window, cx| {
                        AppCommands::drop_database(state.clone(), database.clone(), cx);
                    }
                });
            }
            TreeNodeId::Collection { database, collection, .. } => {
                let message =
                    format!("Drop collection \"{database}.{collection}\"? This cannot be undone.");
                open_confirm_dialog(window, cx, "Drop collection", message, "Drop", true, {
                    let state = self.state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    move |_window, cx| {
                        AppCommands::drop_collection(
                            state.clone(),
                            database.clone(),
                            collection.clone(),
                            cx,
                        );
                    }
                });
            }
        }
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model.open_search();
        self.search_state.update(cx, |state, cx| {
            state.set_value(String::new(), window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.search_open {
            return;
        }
        self.model.close_search();
        self.search_state.update(cx, |state, cx| {
            state.set_value(String::new(), window, cx);
        });
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn handle_typeahead_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.model.search_open {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return;
        }
        let key = event.keystroke.key.to_lowercase();
        let key_char = event.keystroke.key_char.as_deref();
        if !self.model.handle_typeahead_key(&key, key_char) {
            return;
        }
        self.select_typeahead_match(cx);
        if self.model.typeahead_query.is_empty() {
            cx.notify();
        }
    }

    fn select_typeahead_match(&mut self, cx: &mut Context<Self>) {
        if let Some((ix, _node_id)) = self.model.select_typeahead_match() {
            self.scroll_handle.scroll_to_item(ix, gpui::ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn move_sidebar_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some((next, node_id)) = self.model.move_sidebar_selection(delta) else {
            return;
        };
        self.scroll_handle.scroll_to_item(next, gpui::ScrollStrategy::Center);
        let state = self.state.clone();
        state.update(cx, |state, cx| match node_id {
            TreeNodeId::Connection(connection_id) => {
                state.select_connection(Some(connection_id), cx);
            }
            TreeNodeId::Database { connection, database } => {
                state.select_connection(Some(connection), cx);
                state.select_database(database, cx);
            }
            TreeNodeId::Collection { connection, database, collection } => {
                state.select_connection(Some(connection), cx);
                state.preview_collection(database, collection, cx);
            }
        });
        cx.notify();
    }

    fn move_search_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let query = self.search_state.read(cx).value().to_string();
        let results = self.search_results(&query, cx);
        self.model.move_search_selection(delta, results.len());
        cx.notify();
    }

    fn select_search_result(
        &mut self,
        result: &SidebarSearchResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let database = result.database.clone();
        let connection_id = result.connection_id;
        self.model.expanded_nodes.insert(TreeNodeId::connection(connection_id));
        self.model.expanded_nodes.insert(result.node_id.clone());
        self.persist_expanded_nodes(cx);
        self.model.loading_databases.insert(result.node_id.clone());
        AppCommands::load_collections(self.state.clone(), connection_id, database.clone(), cx);
        self.model.selected_tree_id = Some(result.node_id.clone());
        self.scroll_handle.scroll_to_item(result.index, gpui::ScrollStrategy::Center);
        self.state.update(cx, |state, cx| {
            state.select_connection(Some(connection_id), cx);
            state.select_database(database, cx);
        });
        self.close_search(window, cx);
    }

    fn search_results(&self, query: &str, cx: &mut Context<Self>) -> Vec<SidebarSearchResult> {
        let connection_names: HashMap<Uuid, String> = self
            .state
            .read(cx)
            .connections_snapshot()
            .into_iter()
            .map(|conn| (conn.id, conn.name))
            .collect();

        search_results(query, &self.model.entries, &connection_names)
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_ref = self.state.read(cx);
        let active_connections = state_ref.active_connections_snapshot();
        let connecting_id = self.model.connecting_connection;

        let state = self.state.clone();
        let state_for_add = state.clone();
        let state_for_manager = state.clone();
        let state_for_tree = self.state.clone();
        let sidebar_entity = cx.entity();
        let scroll_handle = self.scroll_handle.clone();

        let search_query = self.search_state.read(cx).value().to_string();
        let search_results = if self.model.search_open {
            self.search_results(&search_query, cx)
        } else {
            Vec::new()
        };
        self.model.update_search_selection(&search_query, search_results.len());

        div()
            .key_context("Sidebar")
            .flex()
            .flex_col()
            .w(sizing::sidebar_width())
            .min_w(sizing::sidebar_width())
            .flex_shrink_0()
            .h_full()
            .bg(colors::bg_sidebar())
            .border_r_1()
            .border_color(colors::border())
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, {
                let focus_handle = self.focus_handle.clone();
                move |_, window, _cx| {
                    window.focus(&focus_handle);
                }
            })
            .on_key_down({
                let sidebar_entity = sidebar_entity.clone();
                move |event: &KeyDownEvent, _window: &mut Window, cx: &mut App| {
                    let key = event.keystroke.key.to_lowercase();
                    sidebar_entity.update(cx, |sidebar, cx| {
                        if !sidebar.model.search_open {
                            match key.as_str() {
                                "up" | "arrowup" => {
                                    sidebar.move_sidebar_selection(-1, cx);
                                    cx.stop_propagation();
                                    return;
                                }
                                "down" | "arrowdown" => {
                                    sidebar.move_sidebar_selection(1, cx);
                                    cx.stop_propagation();
                                    return;
                                }
                                "left" | "arrowleft" => {
                                    if let Some(node_id) = sidebar.model.selected_tree_id.clone() {
                                        if sidebar.model.expanded_nodes.contains(&node_id) {
                                            sidebar.model.expanded_nodes.remove(&node_id);
                                            sidebar.persist_expanded_nodes(cx);
                                            sidebar.refresh_tree(cx);
                                        } else if let TreeNodeId::Database { connection, database: _ } =
                                            node_id
                                        {
                                            let parent = TreeNodeId::connection(connection);
                                            sidebar.model.selected_tree_id = Some(parent.clone());
                                            sidebar.scroll_handle.scroll_to_item(
                                                sidebar
                                                    .model
                                                    .entries
                                                    .iter()
                                                    .position(|entry| entry.id == parent)
                                                    .unwrap_or(0),
                                                gpui::ScrollStrategy::Center,
                                            );
                                            cx.notify();
                                        } else if let TreeNodeId::Collection {
                                            connection,
                                            database,
                                            ..
                                        } = node_id
                                        {
                                            let parent =
                                                TreeNodeId::database(connection, database.clone());
                                            sidebar.model.selected_tree_id = Some(parent.clone());
                                            sidebar.scroll_handle.scroll_to_item(
                                                sidebar
                                                    .model
                                                    .entries
                                                    .iter()
                                                    .position(|entry| entry.id == parent)
                                                    .unwrap_or(0),
                                                gpui::ScrollStrategy::Center,
                                            );
                                            cx.notify();
                                        }
                                    }
                                    cx.stop_propagation();
                                    return;
                                }
                                "right" | "arrowright" => {
                                    if let Some(node_id) = sidebar.model.selected_tree_id.clone() {
                                        if node_id.is_connection() {
                                            if !sidebar.model.expanded_nodes.contains(&node_id) {
                                                sidebar.model.expanded_nodes.insert(node_id.clone());
                                                sidebar.persist_expanded_nodes(cx);
                                                sidebar.refresh_tree(cx);
                                            }
                                        } else if let TreeNodeId::Database {
                                            connection,
                                            database,
                                        } = node_id.clone()
                                        {
                                            if !sidebar.model.expanded_nodes.contains(&node_id) {
                                                sidebar.model.expanded_nodes.insert(node_id.clone());
                                                sidebar.persist_expanded_nodes(cx);
                                                sidebar.refresh_tree(cx);
                                            }
                                            let should_load = sidebar
                                                .state
                                                .read(cx)
                                                .conn
                                                .active
                                                .get(&connection)
                                                .is_some_and(|conn| {
                                                    !conn.collections.contains_key(&database)
                                                });
                                            if should_load
                                                && !sidebar.model.loading_databases.contains(&node_id)
                                            {
                                                sidebar.model.loading_databases.insert(node_id.clone());
                                                cx.notify();
                                                AppCommands::load_collections(
                                                    sidebar.state.clone(),
                                                    connection,
                                                    database,
                                                    cx,
                                                );
                                            }
                                        }
                                    }
                                    cx.stop_propagation();
                                    return;
                                }
                                _ => {}
                            }
                        }
                        sidebar.handle_typeahead_key(event, cx);
                    });
                }
            })
            .on_action(cx.listener(|this, _: &OpenSelection, window, cx| {
                this.handle_open_selection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSelectionPreview, _window, cx| {
                this.handle_open_preview(cx);
            }))
            .on_action(cx.listener(|this, _: &EditConnection, window, cx| {
                this.handle_edit_connection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DisconnectConnection, _window, cx| {
                this.handle_disconnect_connection(cx);
            }))
            .on_action(cx.listener(|this, _: &CopySelectionName, _window, cx| {
                this.handle_copy_selection_name(cx);
            }))
            .on_action(cx.listener(|this, _: &CopyConnectionUri, _window, cx| {
                this.handle_copy_connection_uri(cx);
            }))
            .on_action(cx.listener(|this, _: &RenameCollection, window, cx| {
                this.handle_rename_collection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteSelection, window, cx| {
                this.handle_delete_selection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindInSidebar, window, cx| {
                this.open_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseSidebarSearch, window, cx| {
                this.close_search(window, cx);
            }))
            .child(
                // Header with "+" button
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(spacing::md())
                    .h(sizing::header_height())
                    .border_b_1()
                    .border_color(colors::border())
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(colors::text_secondary())
                            .child("CONNECTIONS"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                // Add button
                                div()
                                    .id("add-connection-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(sizing::icon_lg())
                                    .h(sizing::icon_lg())
                                    .rounded(crate::theme::borders::radius_sm())
                                    .cursor_pointer()
                                    .hover(|s| s.bg(colors::bg_hover()))
                                    .text_color(colors::text_primary())
                                    .child(Icon::new(IconName::Plus).xsmall())
                                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        Sidebar::open_add_dialog(state_for_add.clone(), window, cx);
                                    }),
                            )
                            .child(
                                // Manager button
                                div()
                                    .id("manage-connections-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(sizing::icon_lg())
                                    .h(sizing::icon_lg())
                                    .rounded(crate::theme::borders::radius_sm())
                                    .cursor_pointer()
                                    .hover(|s| s.bg(colors::bg_hover()))
                                    .text_color(colors::text_primary())
                                    .child(Icon::new(IconName::Settings).xsmall())
                                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        ConnectionManager::open(state_for_manager.clone(), window, cx);
                                    }),
                            )
                    ),
            )
            .child(
                if self.model.search_open {
                    let sidebar_entity = sidebar_entity.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .px(spacing::md())
                        .py(spacing::xs())
                        .border_b_1()
                        .border_color(colors::border())
                        .child(
                            div()
                                .capture_key_down({
                                    let sidebar_entity = sidebar_entity.clone();
                                    move |event: &KeyDownEvent,
                                          window: &mut Window,
                                          cx: &mut App| {
                                        let key = event.keystroke.key.to_lowercase();
                                        if key == "escape" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                sidebar.close_search(window, cx);
                                            });
                                            cx.stop_propagation();
                                            return;
                                        }
                                        if key == "down" || key == "arrowdown" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                sidebar.move_search_selection(1, cx);
                                            });
                                            cx.stop_propagation();
                                            return;
                                        }
                                        if key == "up" || key == "arrowup" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                sidebar.move_search_selection(-1, cx);
                                            });
                                            cx.stop_propagation();
                                            return;
                                        }
                                        if key == "enter" || key == "return" {
                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                let query =
                                                    sidebar.search_state.read(cx).value().to_string();
                                                let results = sidebar.search_results(&query, cx);
                                                let selection = sidebar.model.search_selected;
                                                let result = selection
                                                    .and_then(|ix| results.get(ix))
                                                    .or_else(|| results.first());
                                                if let Some(result) = result {
                                                    sidebar.select_search_result(
                                                        result,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            });
                                            cx.stop_propagation();
                                        }
                                    }
                                })
                                .child(Input::new(&self.search_state).w_full()),
                        )
                        .child({
                            if search_query.trim().is_empty() {
                                div()
                                    .text_xs()
                                    .text_color(colors::text_muted())
                                    .child("Type to search databases")
                                    .into_any_element()
                            } else if search_results.is_empty() {
                                div()
                                    .text_xs()
                                    .text_color(colors::text_muted())
                                    .child("No matches")
                                    .into_any_element()
                            } else {
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .children(search_results.iter().enumerate().map(|(ix, result)| {
                                        let result = result.clone();
                                        let database = result.database.clone();
                                        let connection_name = result.connection_name.clone();
                                        let sidebar_entity = sidebar_entity.clone();
                                        let is_selected =
                                            self.model.search_selected == Some(ix);
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .px(spacing::sm())
                                            .py(px(4.0))
                                            .rounded(borders::radius_sm())
                                            .hover(|s| s.bg(colors::list_hover()))
                                            .cursor_pointer()
                                            .id(("sidebar-search-row", result.index))
                                            .when(is_selected, |s| s.bg(colors::list_selected()))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .min_w(px(0.0))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(colors::text_primary())
                                                            .truncate()
                                                            .child(database.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(colors::text_muted())
                                                            .truncate()
                                                            .child(connection_name),
                                                    ),
                                            )
                                            .on_click(move |_: &ClickEvent,
                                                           window: &mut Window,
                                                           cx: &mut App| {
                                                let result = result.clone();
                                                sidebar_entity.update(cx, |sidebar, cx| {
                                                    sidebar.select_search_result(
                                                        &result,
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            })
                                    }))
                                    .into_any_element()
                            }
                        })
                        .into_any_element()
                } else {
                    div().into_any_element()
                },
            )
            .child(
                // Connection tree
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(if self.model.entries.is_empty() {
                        div()
                            .p(spacing::md())
                            .text_sm()
                            .text_color(colors::text_muted())
                            .child("No connections yet")
                            .into_any_element()
                    } else {
                        uniform_list("sidebar-rows", self.model.entries.len(), {
                            let state_clone = state_for_tree.clone();
                            let sidebar_entity = sidebar_entity.clone();
                            cx.processor(
                                move |sidebar,
                                      visible_range: std::ops::Range<usize>,
                                      _window,
                                      _cx| {
                                    let mut items = Vec::with_capacity(visible_range.len());
                                    let active_connections = active_connections.clone();
                                    let connecting_id = connecting_id;

                                    for ix in visible_range {
                                        let Some(entry) = sidebar.model.entries.get(ix) else {
                                            continue;
                                        };
                                        let node_id = entry.id.clone();
                                        let depth = entry.depth;
                                        let is_folder = entry.is_folder;
                                        let is_expanded = entry.is_expanded;
                                        let label = entry.label.clone();
                                        let label_for_menu = label.clone();

                                        let is_connection = node_id.is_connection();
                                        let is_database = node_id.is_database();
                                        let is_collection = node_id.is_collection();

                                        let connection_id = node_id.connection_id();
                                        let is_connected = is_connection
                                            && active_connections.contains_key(&connection_id);
                                        let is_connecting =
                                            is_connection && connecting_id == Some(connection_id);
                                        let is_loading_db =
                                            is_database && sidebar.model.loading_databases.contains(&node_id);

                                        let db_name =
                                            node_id.database_name().map(|db| db.to_string());
                                        let selected_db = db_name.clone();
                                        let selected_col =
                                            node_id.collection_name().map(|col| col.to_string());

                                        let selected =
                                            sidebar.model.selected_tree_id.as_ref() == Some(&node_id);
                                        let menu_focus = sidebar.focus_handle.clone();
                                        let row_focus = menu_focus.clone();

                                        let row = div()
                                            .id(("sidebar-row", ix))
                                            .flex()
                                            .items_center()
                                            .w_full()
                                            .gap(px(4.0))
                                            .pl(px(8.0 + 12.0 * depth as f32))
                                            .py(px(2.0))
                                            .on_click({
                                                let node_id = node_id.clone();
                                                let state_clone = state_clone.clone();
                                                let db_name = db_name.clone();
                                                let selected_db = selected_db.clone();
                                                let selected_col = selected_col.clone();
                                                let sidebar_entity = sidebar_entity.clone();
                                                move |event: &ClickEvent,
                                                      _window: &mut Window,
                                                      cx: &mut App| {
                                                    _window.focus(&row_focus);
                                                    cx.stop_propagation();

                                                    state_clone.update(cx, |state, cx| {
                                                        state.select_connection(
                                                            Some(connection_id),
                                                            cx,
                                                        );
                                                    });

                                                    let is_double_click =
                                                        sidebar_entity.update(cx, |sidebar, cx| {
                                                            sidebar.model.selected_tree_id =
                                                                Some(node_id.clone());
                                                            cx.notify();
                                                            event.click_count() >= 2
                                                        });

                                                    if is_double_click {
                                                        // Double-click on connection to connect (expand only)
                                                        if is_connection {
                                                            let should_expand = if is_connecting
                                                            {
                                                                true
                                                            } else if is_connected {
                                                                !is_expanded
                                                            } else {
                                                                true
                                                            };
                                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                                if should_expand {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .insert(node_id.clone());
                                                                } else {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .remove(&node_id);
                                                                }
                                                                sidebar.persist_expanded_nodes(cx);
                                                                sidebar.refresh_tree(cx);
                                                            });

                                                            if is_connected || is_connecting {
                                                                return;
                                                            }
                                                            AppCommands::connect(
                                                                state_clone.clone(),
                                                                node_id.connection_id(),
                                                                cx,
                                                            );
                                                        }
                                                        // Double-click on database to load collections (expand only)
                                                        else if is_database
                                                            && let Some(ref db) = db_name
                                                        {
                                                            let should_expand = !is_expanded;
                                                            sidebar_entity.update(cx, |sidebar, cx| {
                                                                if should_expand {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .insert(node_id.clone());
                                                                } else {
                                                                    sidebar
                                                                        .model
                                                                        .expanded_nodes
                                                                        .remove(&node_id);
                                                                }
                                                                sidebar.persist_expanded_nodes(cx);
                                                                sidebar.refresh_tree(cx);
                                                            });

                                                            if !should_expand || is_loading_db {
                                                                return;
                                                            }
                                                            let should_load = state_clone
                                                                .read(cx)
                                                                .conn
                                                                .active
                                                                .get(&connection_id)
                                                                .is_some_and(|conn| {
                                                                    !conn
                                                                        .collections
                                                                        .contains_key(db)
                                                                });
                                                            if should_load {
                                                                sidebar_entity.update(
                                                                    cx,
                                                                    |sidebar, cx| {
                                                                        sidebar
                                                                            .model
                                                                            .loading_databases
                                                                            .insert(node_id.clone());
                                                                        cx.notify();
                                                                    },
                                                                );
                                                                AppCommands::load_collections(
                                                                    state_clone.clone(),
                                                                    connection_id,
                                                                    db.clone(),
                                                                    cx,
                                                                );
                                                            }
                                                        }
                                                        // Double-click on collection to open
                                                        else if is_collection
                                                            && let (Some(db), Some(col)) =
                                                                (&selected_db, &selected_col)
                                                        {
                                                            state_clone.update(cx, |state, cx| {
                                                                state.select_collection(
                                                                    db.clone(),
                                                                    col.clone(),
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    }
                                                    // Single click: select database or preview collection
                                                    else if is_database
                                                        && let Some(db) = &db_name
                                                    {
                                                        state_clone.update(cx, |state, cx| {
                                                            state.select_database(db.clone(), cx);
                                                        });
                                                    } else if is_collection
                                                        && let (Some(db), Some(col)) =
                                                            (&selected_db, &selected_col)
                                                    {
                                                        state_clone.update(cx, |state, cx| {
                                                            state.preview_collection(
                                                                db.clone(),
                                                                col.clone(),
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                }
                                            })
                                            .hover(|s| s.bg(colors::list_hover()))
                                            .when(selected, |s| s.bg(colors::list_selected()))
                                            .cursor_pointer()
                                            // Chevron for expandable items
                                            .when(is_folder, |this| {
                                                this.child(
                                                    Icon::new(if is_expanded {
                                                        IconName::ChevronDown
                                                    } else {
                                                        IconName::ChevronRight
                                                    })
                                                    .size(sizing::icon_sm())
                                                    .text_color(colors::text_muted()),
                                                )
                                            })
                                            // Spacer for non-folders (align with chevron)
                                            .when(!is_folder, |this| {
                                                this.child(div().w(sizing::icon_sm()))
                                            })
                                            // Connection: status dot + server icon
                                            .when(is_connection, |this| {
                                                this.child(
                                                    div()
                                                        .w(sizing::status_dot())
                                                        .h(sizing::status_dot())
                                                        .rounded_full()
                                                        .bg(if is_connected {
                                                            colors::status_success()
                                                        } else if is_connecting {
                                                            colors::status_warning()
                                                        } else {
                                                            colors::text_muted()
                                                        }),
                                                )
                                                .child(
                                                    Icon::new(IconName::Globe)
                                                        .size(sizing::icon_md())
                                                        .text_color(if is_connected {
                                                            colors::text_primary()
                                                        } else {
                                                            colors::text_secondary()
                                                        }),
                                                )
                                            })
                                            // Database: folder icon
                                            .when(is_database, |this| {
                                                this.child(
                                                    Icon::new(IconName::Folder)
                                                        .size(sizing::icon_md())
                                                        .text_color(colors::text_secondary()),
                                                )
                                            })
                                            // Collection: file icon
                                            .when(is_collection, |this| {
                                                this.child(
                                                    Icon::new(IconName::File)
                                                        .size(sizing::icon_md())
                                                        .text_color(colors::text_muted()),
                                                )
                                            })
                                            // Label
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(if selected {
                                                        colors::text_primary()
                                                    } else {
                                                        colors::text_secondary()
                                                    })
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .child(label.clone()),
                                            )
                                            .when(is_connecting || is_loading_db, |this| {
                                                this.child(Spinner::new().xsmall())
                                            });

                                        let row = row.context_menu({
                                            let menu_node_id = node_id.clone();
                                            let state = state_clone.clone();
                                            let sidebar_entity = sidebar_entity.clone();
                                            move |menu, _window, cx| {
                                                let menu = menu.action_context(menu_focus.clone());
                                                match menu_node_id.clone() {
                                                    TreeNodeId::Connection(connection_id) => {
                                                        build_connection_menu(
                                                            menu,
                                                            state.clone(),
                                                            sidebar_entity.clone(),
                                                            connection_id,
                                                            connecting_id,
                                                            cx,
                                                        )
                                                    }
                                                    TreeNodeId::Database {
                                                        connection,
                                                        database,
                                                    } => {
                                                        let node_id = TreeNodeId::database(
                                                            connection,
                                                            database.clone(),
                                                        );
                                                        build_database_menu(
                                                            menu,
                                                            state.clone(),
                                                            sidebar_entity.clone(),
                                                            node_id,
                                                            database.clone(),
                                                            is_loading_db,
                                                            cx,
                                                        )
                                                    }
                                                    TreeNodeId::Collection {
                                                        connection,
                                                        database,
                                                        collection,
                                                    } => build_collection_menu(
                                                        menu,
                                                        state.clone(),
                                                        connection,
                                                        database.clone(),
                                                        collection.clone(),
                                                        label_for_menu.clone(),
                                                        cx,
                                                    ),
                                                }
                                            }
                                        });

                                        items.push(row);
                                    }

                                    items
                                },
                            )
                        })
                        .flex_grow()
                        .size_full()
                        .track_scroll(scroll_handle)
                        .with_sizing_behavior(ListSizingBehavior::Auto)
                        .into_any_element()
                    }),
            )
    }
}
