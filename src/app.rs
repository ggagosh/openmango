use crate::components::Button;
use crate::components::ConnectionDialog;
use gpui::prelude::{FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _};
use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::{Icon, IconName, Sizable as _};
use std::collections::HashSet;
use uuid::Uuid;

use crate::components::{
    ConnectionManager, ContentArea, StatusBar, TreeNodeId, open_confirm_dialog,
};
use crate::keyboard::{
    CloseSidebarSearch, CloseTab, CopyConnectionUri, CopySelectionName, CreateCollection,
    CreateDatabase, CreateIndex, DeleteConnection, DeleteDatabase, DeleteSelection,
    DisconnectConnection, EditConnection, FindInSidebar, NewConnection, NextTab, OpenSelection,
    OpenSelectionPreview, PrevTab, QuitApp, RefreshView, RenameCollection,
};
use crate::models::{ActiveConnection, SavedConnection};
use crate::state::{
    ActiveTab, AppCommands, AppEvent, AppState, CollectionSubview, SessionKey, View,
};
use crate::theme::{borders, colors, sizing, spacing};
use crate::views::CollectionView;

// =============================================================================
// App Component
// =============================================================================

pub struct AppRoot {
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    content_area: Entity<ContentArea>,
    key_debug: bool,
    last_keystroke: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl AppRoot {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create the app state entity
        let state = cx.new(|_| AppState::new());

        // Create sidebar with state reference
        let sidebar = cx.new(|cx| Sidebar::new(state.clone(), window, cx));

        // Create content area with state reference
        let content_area = cx.new(|cx| ContentArea::new(state.clone(), cx));

        cx.observe(&state, |_, _, cx| cx.notify()).detach();

        let key_debug = std::env::var("OPENMANGO_DEBUG_KEYS").is_ok();
        let mut subscriptions = Vec::new();

        let weak_view = cx.entity().downgrade();
        let subscription = cx.intercept_keystrokes(move |event, window, cx| {
            let Some(view) = weak_view.upgrade() else {
                return;
            };
            view.update(cx, |this, cx| {
                if this.key_debug {
                    this.last_keystroke = Some(format_keystroke(event));
                    cx.notify();
                }

                let key = event.keystroke.key.to_ascii_lowercase();
                let modifiers = event.keystroke.modifiers;
                let cmd_or_ctrl = modifiers.secondary() || modifiers.control;
                let alt = modifiers.alt;
                let shift = modifiers.shift;

                if cmd_or_ctrl && !alt && !shift && key == "q" {
                    cx.quit();
                    cx.stop_propagation();
                    return;
                }

                if cmd_or_ctrl && !alt && !shift && key == "n" {
                    match this.state.read(cx).current_view {
                        View::Documents => {
                            let subview = this
                                .state
                                .read(cx)
                                .current_session_key()
                                .and_then(|key| {
                                    this.state
                                        .read(cx)
                                        .session(&key)
                                        .map(|session| session.view.subview)
                                })
                                .unwrap_or(CollectionSubview::Documents);
                            if let Some(session_key) = this.state.read(cx).current_session_key() {
                                match subview {
                                    CollectionSubview::Documents => {
                                        CollectionView::open_insert_document_json_editor(
                                            this.state.clone(),
                                            session_key,
                                            window,
                                            cx,
                                        );
                                        cx.stop_propagation();
                                    }
                                    CollectionSubview::Indexes => {
                                        CollectionView::open_index_create_dialog(
                                            this.state.clone(),
                                            session_key,
                                            window,
                                            cx,
                                        );
                                        cx.stop_propagation();
                                    }
                                    CollectionSubview::Stats => {}
                                }
                            }
                        }
                        View::Database => {
                            let database = this.state.read(cx).conn.selected_database.clone();
                            if let Some(database) = database {
                                open_create_collection_dialog(
                                    this.state.clone(),
                                    database,
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }
                        }
                        View::Welcome | View::Databases | View::Collections => {
                            ConnectionDialog::open(this.state.clone(), window, cx);
                            cx.stop_propagation();
                        }
                    }
                    return;
                }

                if cmd_or_ctrl && shift && !alt && key == "n" {
                    if !matches!(this.state.read(cx).current_view, View::Documents) {
                        this.handle_create_database(window, cx);
                        cx.stop_propagation();
                    }
                    return;
                }

                if cmd_or_ctrl && !alt && !shift && key == "w" {
                    this.handle_close_tab(cx);
                    cx.stop_propagation();
                    return;
                }

                if cmd_or_ctrl && !alt && !shift && key == "r" {
                    this.handle_refresh(cx);
                    cx.stop_propagation();
                }
            });
        });
        subscriptions.push(subscription);

        Self {
            state,
            sidebar,
            content_area,
            key_debug,
            last_keystroke: None,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read state for StatusBar props
        let state = self.state.read(cx);
        let active_conn = state.conn.selected_connection.and_then(|id| state.conn.active.get(&id));
        let is_connected = active_conn.is_some();
        let connection_name = active_conn.map(|c| c.config.name.clone());
        let status_message = state.status_message.clone();
        let read_only = active_conn.map(|c| c.config.read_only).unwrap_or(false);

        let documents_subview = if matches!(state.current_view, View::Documents) {
            state
                .current_session_key()
                .and_then(|key| state.session(&key))
                .map(|session| session.view.subview)
        } else {
            None
        };

        let mut key_context = String::from("Workspace");
        match state.current_view {
            View::Documents => {
                key_context.push_str(" Documents");
                match documents_subview {
                    Some(CollectionSubview::Indexes) => key_context.push_str(" Indexes"),
                    Some(CollectionSubview::Stats) => key_context.push_str(" Stats"),
                    _ => {}
                }
            }
            View::Database => key_context.push_str(" Database"),
            View::Databases => key_context.push_str(" Databases"),
            View::Collections => key_context.push_str(" Collections"),
            View::Welcome => key_context.push_str(" Welcome"),
        }

        // Render dialog layer (Context derefs to App)
        use gpui_component::Root;
        let dialog_layer = Root::render_dialog_layer(window, cx);

        let mut root = div()
            .key_context(key_context.as_str())
            .on_action(cx.listener(|this, _: &CloseTab, _window, cx| {
                this.handle_close_tab(cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, _window, cx| {
                this.state.update(cx, |state, cx| {
                    state.select_next_tab(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _window, cx| {
                this.state.update(cx, |state, cx| {
                    state.select_prev_tab(cx);
                });
            }))
            .on_action(cx.listener(|_this, _: &QuitApp, _window, cx| {
                cx.quit();
            }))
            .on_action(cx.listener(|this, _: &NewConnection, window, cx| {
                this.handle_new_connection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CreateDatabase, window, cx| {
                this.handle_create_database(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CreateCollection, window, cx| {
                this.handle_create_collection(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CreateIndex, window, cx| {
                this.handle_create_index(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RefreshView, _window, cx| {
                this.handle_refresh(cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteDatabase, window, cx| {
                let database = this.state.read(cx).conn.selected_database.clone();
                let Some(database) = database else {
                    return;
                };
                open_confirm_dialog(
                    window,
                    cx,
                    "Drop database",
                    format!("Drop database {database}? This cannot be undone."),
                    "Drop",
                    true,
                    {
                        let state = this.state.clone();
                        let database = database.clone();
                        move |_window, cx| {
                            AppCommands::drop_database(state.clone(), database.clone(), cx);
                        }
                    },
                );
            }))
            .on_action(cx.listener(|this, _: &DeleteConnection, window, cx| {
                let connection_id = this.state.read(cx).conn.selected_connection;
                let Some(connection_id) = connection_id else {
                    return;
                };
                open_confirm_dialog(
                    window,
                    cx,
                    "Remove connection",
                    "Remove this connection? This cannot be undone.".to_string(),
                    "Remove",
                    true,
                    {
                        let state = this.state.clone();
                        move |_window, cx| {
                            state.update(cx, |state, cx| {
                                state.remove_connection(connection_id, cx);
                            });
                        }
                    },
                );
            }))
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .bg(crate::theme::bg_app())
            .text_color(crate::theme::text_primary())
            .font_family(crate::theme::fonts::ui())
            .child(
                // Main content area: Sidebar + Content
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.sidebar.clone())
                    .child(div().flex().flex_1().min_w(px(0.0)).child(self.content_area.clone())),
            )
            .child(StatusBar::new(is_connected, connection_name, status_message, read_only))
            .children(dialog_layer);

        if self.key_debug {
            root =
                root.child(render_key_debug_overlay(&key_context, self.last_keystroke.as_deref()));
        }

        root
    }
}

impl AppRoot {
    fn handle_new_connection(&mut self, window: &mut Window, cx: &mut App) {
        ConnectionDialog::open(self.state.clone(), window, cx);
    }

    fn handle_create_database(&mut self, window: &mut Window, cx: &mut App) {
        let state_ref = self.state.read(cx);
        let Some(conn_id) = state_ref.conn.selected_connection else {
            return;
        };
        if !state_ref.conn.active.contains_key(&conn_id) {
            return;
        }
        open_create_database_dialog(self.state.clone(), window, cx);
    }

    fn handle_create_collection(&mut self, window: &mut Window, cx: &mut App) {
        let state_ref = self.state.read(cx);
        let Some(conn_id) = state_ref.conn.selected_connection else {
            return;
        };
        if !state_ref.conn.active.contains_key(&conn_id) {
            return;
        }
        let database = state_ref.conn.selected_database.clone();
        let Some(database) = database else {
            return;
        };
        open_create_collection_dialog(self.state.clone(), database, window, cx);
    }

    fn handle_create_index(&mut self, window: &mut Window, cx: &mut App) {
        if !matches!(self.state.read(cx).current_view, View::Documents) {
            return;
        }
        let Some(session_key) = self.state.read(cx).current_session_key() else {
            return;
        };
        let subview = self
            .state
            .read(cx)
            .session(&session_key)
            .map(|session| session.view.subview)
            .unwrap_or(CollectionSubview::Documents);
        if subview != CollectionSubview::Indexes {
            return;
        }
        CollectionView::open_index_create_dialog(self.state.clone(), session_key, window, cx);
    }

    fn handle_close_tab(&mut self, cx: &mut App) {
        self.state.update(cx, |state, cx| match state.tabs.active {
            ActiveTab::Preview => state.close_preview_tab(cx),
            ActiveTab::Index(index) => state.close_tab(index, cx),
            ActiveTab::None => {}
        });
    }

    fn handle_refresh(&mut self, cx: &mut App) {
        let (current_view, session_key, database_key, subview) = {
            let state_ref = self.state.read(cx);
            let session_key = state_ref.current_session_key();
            let subview = session_key
                .as_ref()
                .and_then(|key| state_ref.session(key))
                .map(|session| session.view.subview)
                .unwrap_or(CollectionSubview::Documents);
            (state_ref.current_view, session_key, state_ref.current_database_key(), subview)
        };

        match current_view {
            View::Documents => {
                let Some(session_key) = session_key else {
                    return;
                };
                match subview {
                    CollectionSubview::Documents => {
                        AppCommands::load_documents_for_session(
                            self.state.clone(),
                            session_key,
                            cx,
                        );
                    }
                    CollectionSubview::Indexes => {
                        AppCommands::load_collection_indexes(
                            self.state.clone(),
                            session_key,
                            true,
                            cx,
                        );
                    }
                    CollectionSubview::Stats => {
                        AppCommands::load_collection_stats(self.state.clone(), session_key, cx);
                    }
                }
            }
            View::Database => {
                let Some(database_key) = database_key else {
                    return;
                };
                AppCommands::load_database_overview(self.state.clone(), database_key, true, cx);
            }
            View::Databases | View::Collections | View::Welcome => {
                let state_ref = self.state.read(cx);
                if let Some(conn_id) = state_ref.conn.selected_connection
                    && state_ref.conn.active.contains_key(&conn_id)
                {
                    AppCommands::refresh_databases(self.state.clone(), conn_id, cx);
                }
            }
        }
    }
}

fn render_key_debug_overlay(key_context: &str, last_keystroke: Option<&str>) -> AnyElement {
    let last_keystroke = last_keystroke.unwrap_or("-");
    div()
        .absolute()
        .bottom(px(12.0))
        .right(px(12.0))
        .w(px(320.0))
        .p(spacing::sm())
        .rounded(borders::radius_sm())
        .bg(colors::bg_header())
        .border_1()
        .border_color(colors::border())
        .text_xs()
        .text_color(colors::text_primary())
        .font_family(crate::theme::fonts::mono())
        .child(div().text_sm().child("Keymap debug"))
        .child(div().text_color(colors::text_muted()).child("Key context:"))
        .child(div().child(key_context.to_string()))
        .child(div().text_color(colors::text_muted()).child("Last keystroke:"))
        .child(div().child(last_keystroke.to_string()))
        .into_any_element()
}

fn format_keystroke(event: &KeystrokeEvent) -> String {
    let modifiers = event.keystroke.modifiers;
    let mut parts = Vec::new();
    if modifiers.platform {
        parts.push("cmd");
    }
    if modifiers.control {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    let key = event.keystroke.key.to_string();
    if parts.is_empty() {
        key
    } else {
        parts.push(&key);
        parts.join("-")
    }
}

// =============================================================================
// Sidebar Component
// =============================================================================

pub struct Sidebar {
    state: Entity<AppState>,
    connecting_connection: Option<Uuid>,
    loading_databases: HashSet<TreeNodeId>,
    expanded_nodes: HashSet<TreeNodeId>,
    selected_tree_id: Option<TreeNodeId>,
    entries: Vec<SidebarEntry>,
    search_open: bool,
    search_state: Entity<InputState>,
    search_selected: Option<usize>,
    typeahead_query: String,
    typeahead_last: Option<std::time::Instant>,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
struct SidebarEntry {
    id: TreeNodeId,
    label: String,
    depth: usize,
    is_folder: bool,
    is_expanded: bool,
}

#[derive(Clone, Debug)]
struct SidebarSearchResult {
    index: usize,
    node_id: TreeNodeId,
    connection_id: Uuid,
    connection_name: String,
    database: String,
    score: usize,
}

impl Sidebar {
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (connections, active) = {
            let state_ref = state.read(cx);
            (state_ref.connections.clone(), state_ref.conn.active.clone())
        };
        let entries = Self::build_entries(&connections, &active, &HashSet::new());
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
                this.loading_databases.clear();
                this.refresh_tree(cx);
            }
            AppEvent::CollectionsFailed(_) => {
                this.loading_databases.clear();
                cx.notify();
            }
            AppEvent::Connecting(connection_id) => {
                this.connecting_connection = Some(*connection_id);
                cx.notify();
            }
            AppEvent::Connected(connection_id) => {
                if this.connecting_connection == Some(*connection_id) {
                    this.connecting_connection = None;
                }
                this.loading_databases.clear();
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
                if this.connecting_connection == Some(*connection_id) {
                    this.connecting_connection = None;
                }
                this.loading_databases.clear();
                this.selected_tree_id = None;
                this.refresh_tree(cx);
            }
            AppEvent::ConnectionFailed(_) => {
                this.connecting_connection = None;
                this.loading_databases.clear();
                this.selected_tree_id = None;
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
            connecting_connection: None,
            loading_databases: HashSet::new(),
            expanded_nodes: HashSet::new(),
            selected_tree_id: None,
            entries,
            search_open: false,
            search_state,
            search_selected: None,
            typeahead_query: String::new(),
            typeahead_last: None,
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
        let (connections, active) = {
            let state_ref = self.state.read(cx);
            (state_ref.connections.clone(), state_ref.conn.active.clone())
        };

        if self.selected_tree_id.is_none() {
            let state_ref = self.state.read(cx);
            if let Some(connection_id) = state_ref.conn.selected_connection
                && let Some(db) = state_ref.conn.selected_database.as_ref()
            {
                if let Some(col) = state_ref.conn.selected_collection.as_ref() {
                    self.selected_tree_id = Some(TreeNodeId::collection(connection_id, db, col));
                } else {
                    self.selected_tree_id = Some(TreeNodeId::database(connection_id, db));
                }
            }
        }

        self.entries = Self::build_entries(&connections, &active, &self.expanded_nodes);
        if let Some(node_id) = &self.selected_tree_id
            && let Some(ix) = self.entries.iter().position(|entry| &entry.id == node_id)
        {
            self.scroll_handle.scroll_to_item(ix, gpui::ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn sync_selection_from_state(&mut self, cx: &mut Context<Self>) {
        let (connection_id, selected_db, selected_col) = {
            let state_ref = self.state.read(cx);
            let Some(connection_id) = state_ref.conn.selected_connection else {
                return;
            };
            (
                connection_id,
                state_ref.conn.selected_database.clone(),
                state_ref.conn.selected_collection.clone(),
            )
        };

        if let Some(db) = selected_db.as_ref() {
            self.expanded_nodes.insert(TreeNodeId::connection(connection_id));
            if selected_col.is_some() {
                self.expanded_nodes.insert(TreeNodeId::database(connection_id, db));
            }
        }

        self.selected_tree_id = match (selected_db.as_ref(), selected_col.as_ref()) {
            (Some(db), Some(col)) => {
                Some(TreeNodeId::collection(connection_id, db.to_string(), col.to_string()))
            }
            (Some(db), None) => Some(TreeNodeId::database(connection_id, db.to_string())),
            _ => None,
        };

        self.persist_expanded_nodes(cx);
        self.refresh_tree(cx);
    }

    fn persist_expanded_nodes(&mut self, cx: &mut Context<Self>) {
        let mut nodes: Vec<String> = self.expanded_nodes.iter().map(|id| id.to_tree_id()).collect();
        nodes.sort();
        self.state.update(cx, |state, _cx| {
            state.set_workspace_expanded_nodes(nodes);
        });
    }

    fn restore_workspace_expansion(&mut self, cx: &mut Context<Self>) {
        let (connection_id, selected_db, expanded) = {
            let state_ref = self.state.read(cx);
            let Some(connection_id) =
                state_ref.workspace.last_connection_id.or(state_ref.conn.selected_connection)
            else {
                return;
            };
            let Some(_active) = state_ref.conn.active.get(&connection_id) else {
                return;
            };
            let mut expanded: HashSet<TreeNodeId> = state_ref
                .workspace
                .expanded_nodes
                .iter()
                .filter_map(|id| TreeNodeId::from_tree_id(id))
                .filter(|node| node.connection_id() == connection_id)
                .collect();

            let selected_db = state_ref.conn.selected_database.clone();
            if let Some(db) = selected_db.as_ref() {
                expanded.insert(TreeNodeId::connection(connection_id));
                expanded.insert(TreeNodeId::database(connection_id, db));
            }

            (connection_id, selected_db, expanded)
        };

        self.expanded_nodes = expanded;
        if selected_db.is_some() {
            self.expanded_nodes.insert(TreeNodeId::connection(connection_id));
        }
        self.selected_tree_id = None;
        self.refresh_tree(cx);
        self.load_expanded_databases(cx);
    }

    fn load_expanded_databases(&mut self, cx: &mut Context<Self>) {
        for node in self.expanded_nodes.iter() {
            let TreeNodeId::Database { connection, database } = node else {
                continue;
            };
            let collections = {
                let state_ref = self.state.read(cx);
                let Some(conn) = state_ref.conn.active.get(connection) else {
                    continue;
                };
                conn.collections.clone()
            };
            if collections.contains_key(database) || self.loading_databases.contains(node) {
                continue;
            }
            self.loading_databases.insert(node.clone());
            AppCommands::load_collections(self.state.clone(), *connection, database.clone(), cx);
        }
    }

    fn build_entries(
        connections: &[SavedConnection],
        active: &std::collections::HashMap<Uuid, ActiveConnection>,
        expanded: &HashSet<TreeNodeId>,
    ) -> Vec<SidebarEntry> {
        let mut items = Vec::new();
        for conn in connections {
            let conn_node_id = TreeNodeId::connection(conn.id);
            let conn_expanded = expanded.contains(&conn_node_id);
            let active_conn = active.get(&conn.id);
            let conn_is_folder = active_conn.is_some();
            items.push(SidebarEntry {
                id: conn_node_id,
                label: conn.name.clone(),
                depth: 0,
                is_folder: conn_is_folder,
                is_expanded: conn_expanded,
            });

            if let Some(active_conn) = active_conn
                && conn_expanded
            {
                for db_name in &active_conn.databases {
                    let db_node_id = TreeNodeId::database(conn.id, db_name);
                    let db_expanded = expanded.contains(&db_node_id);
                    items.push(SidebarEntry {
                        id: db_node_id.clone(),
                        label: db_name.clone(),
                        depth: 1,
                        is_folder: true,
                        is_expanded: db_expanded,
                    });

                    if db_expanded && let Some(collections) = active_conn.collections.get(db_name) {
                        for col_name in collections {
                            let col_node_id = TreeNodeId::collection(conn.id, db_name, col_name);
                            items.push(SidebarEntry {
                                id: col_node_id,
                                label: col_name.clone(),
                                depth: 2,
                                is_folder: false,
                                is_expanded: false,
                            });
                        }
                    }
                }
            }
        }

        items
    }

    fn open_add_dialog(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        ConnectionDialog::open(state, window, cx);
    }

    fn handle_open_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_open {
            let query = self.search_state.read(cx).value().to_string();
            let results = self.search_results(&query, cx);
            let selection = self.search_selected;
            let result = selection.and_then(|ix| results.get(ix)).or_else(|| results.first());
            if let Some(result) = result {
                self.select_search_result(result, window, cx);
            }
            return;
        }
        let Some(node_id) = self.selected_tree_id.clone() else {
            return;
        };

        if node_id.is_connection() {
            let connection_id = node_id.connection_id();
            self.state.update(cx, |state, cx| {
                state.select_connection(Some(connection_id), cx);
            });
            let is_connected = self.state.read(cx).conn.active.contains_key(&connection_id);
            let is_connecting = self.connecting_connection == Some(connection_id);

            if !is_connected && !is_connecting {
                self.expanded_nodes.insert(node_id.clone());
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
            let should_expand = !self.expanded_nodes.contains(&node_id);
            if should_expand {
                self.expanded_nodes.insert(node_id.clone());
                self.persist_expanded_nodes(cx);
                self.refresh_tree(cx);
            }
            if should_expand && !self.loading_databases.contains(&node_id) {
                let should_load = self
                    .state
                    .read(cx)
                    .conn
                    .active
                    .get(&node_id.connection_id())
                    .is_some_and(|conn| !conn.collections.contains_key(&db));
                if should_load {
                    self.loading_databases.insert(node_id.clone());
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
        let Some(node_id) = self.selected_tree_id.clone() else {
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
        let Some(TreeNodeId::Connection(connection_id)) = self.selected_tree_id.clone() else {
            return;
        };
        ConnectionManager::open_selected(self.state.clone(), connection_id, window, cx);
    }

    fn handle_disconnect_connection(&mut self, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Connection(connection_id)) = self.selected_tree_id.clone() else {
            return;
        };
        let is_active = self.state.read(cx).conn.active.contains_key(&connection_id);
        if is_active {
            AppCommands::disconnect(self.state.clone(), connection_id, cx);
        }
    }

    fn handle_copy_selection_name(&mut self, cx: &mut Context<Self>) {
        let Some(node_id) = self.selected_tree_id.clone() else {
            return;
        };
        let text = match node_id {
            TreeNodeId::Connection(connection_id) => self
                .state
                .read(cx)
                .connections
                .iter()
                .find(|conn| conn.id == connection_id)
                .map(|conn| conn.name.clone()),
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
        let Some(TreeNodeId::Connection(connection_id)) = self.selected_tree_id.clone() else {
            return;
        };
        if let Some(conn) =
            self.state.read(cx).connections.iter().find(|conn| conn.id == connection_id)
        {
            cx.write_to_clipboard(ClipboardItem::new_string(conn.uri.clone()));
        }
    }

    fn handle_rename_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Collection { database, collection, .. }) =
            self.selected_tree_id.clone()
        else {
            return;
        };
        open_rename_collection_dialog(self.state.clone(), database, collection, window, cx);
    }

    fn handle_delete_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node_id) = self.selected_tree_id.clone() else {
            return;
        };
        match node_id {
            TreeNodeId::Connection(connection_id) => {
                let name = self
                    .state
                    .read(cx)
                    .connections
                    .iter()
                    .find(|conn| conn.id == connection_id)
                    .map(|conn| conn.name.clone())
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
        self.search_open = true;
        self.typeahead_query.clear();
        self.typeahead_last = None;
        self.search_selected = Some(0);
        self.search_state.update(cx, |state, cx| {
            state.set_value(String::new(), window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            return;
        }
        self.search_open = false;
        self.search_selected = None;
        self.search_state.update(cx, |state, cx| {
            state.set_value(String::new(), window, cx);
        });
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn handle_typeahead_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.search_open {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return;
        }
        let key = event.keystroke.key.to_lowercase();
        if key == "escape" {
            if !self.typeahead_query.is_empty() {
                self.typeahead_query.clear();
                cx.notify();
            }
            return;
        }
        if key == "backspace" {
            if !self.typeahead_query.is_empty() {
                self.typeahead_query.pop();
                self.typeahead_last = Some(std::time::Instant::now());
                self.select_typeahead_match(cx);
            }
            return;
        }
        let Some(key_char) = event.keystroke.key_char.as_ref() else {
            return;
        };
        if key_char.chars().count() != 1 {
            return;
        }
        let ch = key_char.to_lowercase();
        let now = std::time::Instant::now();
        if self
            .typeahead_last
            .is_none_or(|last| now.duration_since(last) > std::time::Duration::from_millis(1000))
        {
            self.typeahead_query.clear();
        }
        self.typeahead_last = Some(now);
        self.typeahead_query.push_str(&ch);
        self.select_typeahead_match(cx);
    }

    fn select_typeahead_match(&mut self, cx: &mut Context<Self>) {
        let query = self.typeahead_query.trim();
        if query.is_empty() {
            return;
        }
        let query = query.to_lowercase();
        let entries = &self.entries;
        if entries.is_empty() {
            return;
        }
        let start = self
            .selected_tree_id
            .as_ref()
            .and_then(|id| entries.iter().position(|entry| &entry.id == id))
            .map(|ix| ix + 1)
            .unwrap_or(0);

        let mut matched_index = None;
        for offset in 0..entries.len() {
            let idx = (start + offset) % entries.len();
            let entry = &entries[idx];
            if entry.label.to_lowercase().starts_with(&query) {
                matched_index = Some(idx);
                break;
            }
        }

        if let Some(ix) = matched_index {
            let entry = &entries[ix];
            self.selected_tree_id = Some(entry.id.clone());
            self.scroll_handle.scroll_to_item(ix, gpui::ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn move_sidebar_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.entries.is_empty() {
            return;
        }
        let current_index = self
            .selected_tree_id
            .as_ref()
            .and_then(|id| self.entries.iter().position(|entry| &entry.id == id))
            .unwrap_or(0);
        let len = self.entries.len() as isize;
        let next = (current_index as isize + delta).rem_euclid(len) as usize;
        let entry = &self.entries[next];
        self.selected_tree_id = Some(entry.id.clone());
        self.scroll_handle.scroll_to_item(next, gpui::ScrollStrategy::Center);
        let state = self.state.clone();
        let node_id = entry.id.clone();
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
        if results.is_empty() {
            self.search_selected = None;
            cx.notify();
            return;
        }
        let len = results.len() as isize;
        let current = self.search_selected.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.search_selected = Some(next);
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
        self.expanded_nodes.insert(TreeNodeId::connection(connection_id));
        self.expanded_nodes.insert(result.node_id.clone());
        self.persist_expanded_nodes(cx);
        self.loading_databases.insert(result.node_id.clone());
        AppCommands::load_collections(self.state.clone(), connection_id, database.clone(), cx);
        self.selected_tree_id = Some(result.node_id.clone());
        self.scroll_handle.scroll_to_item(result.index, gpui::ScrollStrategy::Center);
        self.state.update(cx, |state, cx| {
            state.select_connection(Some(connection_id), cx);
            state.select_database(database, cx);
        });
        self.close_search(window, cx);
    }

    fn fuzzy_match_score(query: &str, text: &str) -> Option<usize> {
        if query.is_empty() {
            return None;
        }
        let mut score = 0usize;
        let mut last_index = 0usize;
        let chars: Vec<char> = text.chars().collect();
        for ch in query.chars() {
            let mut found = None;
            for (offset, tc) in chars.iter().enumerate().skip(last_index) {
                if *tc == ch {
                    found = Some(offset);
                    break;
                }
            }
            let pos = found?;
            score += pos.saturating_sub(last_index);
            last_index = pos + 1;
        }
        Some(score)
    }

    fn search_results(&self, query: &str, cx: &mut Context<Self>) -> Vec<SidebarSearchResult> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }

        let connection_names: std::collections::HashMap<Uuid, String> = self
            .state
            .read(cx)
            .connections
            .iter()
            .map(|conn| (conn.id, conn.name.clone()))
            .collect();

        let mut results = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let TreeNodeId::Database { connection, database } = &entry.id else {
                continue;
            };
            let label = entry.label.to_lowercase();
            let Some(score) = Self::fuzzy_match_score(&query, &label) else {
                continue;
            };
            let connection_name = connection_names
                .get(connection)
                .cloned()
                .unwrap_or_else(|| "Connection".to_string());
            results.push(SidebarSearchResult {
                index,
                node_id: entry.id.clone(),
                connection_id: *connection,
                connection_name,
                database: database.clone(),
                score,
            });
        }

        results.sort_by(|a, b| {
            a.score
                .cmp(&b.score)
                .then_with(|| a.database.len().cmp(&b.database.len()))
                .then_with(|| a.database.cmp(&b.database))
        });
        results
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_ref = self.state.read(cx);
        let active_connections = state_ref.conn.active.clone();
        let connecting_id = self.connecting_connection;

        let state = self.state.clone();
        let state_for_add = state.clone();
        let state_for_manager = state.clone();
        let state_for_tree = self.state.clone();
        let sidebar_entity = cx.entity();
        let scroll_handle = self.scroll_handle.clone();

        let search_query = self.search_state.read(cx).value().to_string();
        let search_results =
            if self.search_open { self.search_results(&search_query, cx) } else { Vec::new() };
        if self.search_open {
            if search_query.trim().is_empty() || search_results.is_empty() {
                self.search_selected = None;
            } else if self.search_selected.is_none_or(|ix| ix >= search_results.len()) {
                self.search_selected = Some(0);
            }
        }

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
                        if !sidebar.search_open {
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
                                    if let Some(node_id) = sidebar.selected_tree_id.clone() {
                                        if sidebar.expanded_nodes.contains(&node_id) {
                                            sidebar.expanded_nodes.remove(&node_id);
                                            sidebar.persist_expanded_nodes(cx);
                                            sidebar.refresh_tree(cx);
                                        } else if let TreeNodeId::Database { connection, database: _ } =
                                            node_id
                                        {
                                            let parent = TreeNodeId::connection(connection);
                                            sidebar.selected_tree_id = Some(parent.clone());
                                            sidebar.scroll_handle.scroll_to_item(
                                                sidebar
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
                                            sidebar.selected_tree_id = Some(parent.clone());
                                            sidebar.scroll_handle.scroll_to_item(
                                                sidebar
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
                                    if let Some(node_id) = sidebar.selected_tree_id.clone() {
                                        if node_id.is_connection() {
                                            if !sidebar.expanded_nodes.contains(&node_id) {
                                                sidebar.expanded_nodes.insert(node_id.clone());
                                                sidebar.persist_expanded_nodes(cx);
                                                sidebar.refresh_tree(cx);
                                            }
                                        } else if let TreeNodeId::Database {
                                            connection,
                                            database,
                                        } = node_id.clone()
                                        {
                                            if !sidebar.expanded_nodes.contains(&node_id) {
                                                sidebar.expanded_nodes.insert(node_id.clone());
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
                                                && !sidebar.loading_databases.contains(&node_id)
                                            {
                                                sidebar.loading_databases.insert(node_id.clone());
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
                            ),
                    ),
            )
            .child(
                if self.search_open {
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
                                                let selection = sidebar.search_selected;
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
                                            self.search_selected == Some(ix);
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
                    .child(if self.entries.is_empty() {
                        div()
                            .p(spacing::md())
                            .text_sm()
                            .text_color(colors::text_muted())
                            .child("No connections yet")
                            .into_any_element()
                    } else {
                        uniform_list("sidebar-rows", self.entries.len(), {
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
                                        let Some(entry) = sidebar.entries.get(ix) else {
                                            continue;
                                        };
                                        let node_id = entry.id.clone();
                                        let depth = entry.depth;
                                        let is_folder = entry.is_folder;
                                        let is_expanded = entry.is_expanded;
                                        let label = entry.label.clone();

                                        let is_connection = node_id.is_connection();
                                        let is_database = node_id.is_database();
                                        let is_collection = node_id.is_collection();

                                        let connection_id = node_id.connection_id();
                                        let is_connected = is_connection
                                            && active_connections.contains_key(&connection_id);
                                        let is_connecting =
                                            is_connection && connecting_id == Some(connection_id);
                                        let is_loading_db =
                                            is_database && sidebar.loading_databases.contains(&node_id);

                                        let db_name =
                                            node_id.database_name().map(|db| db.to_string());
                                        let selected_db = db_name.clone();
                                        let selected_col =
                                            node_id.collection_name().map(|col| col.to_string());

                                        let selected =
                                            sidebar.selected_tree_id.as_ref() == Some(&node_id);
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
                                                            sidebar.selected_tree_id =
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
                                                                        .expanded_nodes
                                                                        .insert(node_id.clone());
                                                                } else {
                                                                    sidebar.expanded_nodes.remove(
                                                                        &node_id,
                                                                    );
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
                                                                        .expanded_nodes
                                                                        .insert(node_id.clone());
                                                                } else {
                                                                    sidebar.expanded_nodes.remove(
                                                                        &node_id,
                                                                    );
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
                                                    .child(label),
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

fn build_connection_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    connection_id: Uuid,
    connecting_id: Option<Uuid>,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let is_connected = state.read(cx).conn.active.contains_key(&connection_id);
    let is_connecting = connecting_id == Some(connection_id);

    menu = menu
        .item(
            PopupMenuItem::new("Connect")
                .action(Box::new(OpenSelection))
                .disabled(is_connected || is_connecting)
                .on_click({
                    let state = state.clone();
                    let sidebar = sidebar.clone();
                    move |_, _window, cx| {
                        if state.read(cx).conn.active.contains_key(&connection_id) {
                            return;
                        }

                        sidebar.update(cx, |sidebar, cx| {
                            sidebar.expanded_nodes.insert(TreeNodeId::connection(connection_id));
                            sidebar.persist_expanded_nodes(cx);
                            sidebar.refresh_tree(cx);
                        });

                        AppCommands::connect(state.clone(), connection_id, cx);
                    }
                }),
        )
        .item(PopupMenuItem::new("Edit Connection...").action(Box::new(EditConnection)).on_click({
            let state = state.clone();
            move |_, window, cx| {
                ConnectionManager::open_selected(state.clone(), connection_id, window, cx);
            }
        }))
        .item(
            PopupMenuItem::new("Remove Connection...").action(Box::new(DeleteSelection)).on_click(
                {
                    let state = state.clone();
                    move |_, window, cx| {
                        let name = state
                            .read(cx)
                            .connections
                            .iter()
                            .find(|conn| conn.id == connection_id)
                            .map(|conn| conn.name.clone())
                            .unwrap_or_else(|| "this connection".to_string());
                        let message =
                            format!("Remove connection \"{name}\"? This cannot be undone.");
                        open_confirm_dialog(
                            window,
                            cx,
                            "Remove connection",
                            message,
                            "Remove",
                            true,
                            {
                                let state = state.clone();
                                move |_window, cx| {
                                    state.update(cx, |state, cx| {
                                        state.remove_connection(connection_id, cx);
                                    });
                                }
                            },
                        );
                    }
                },
            ),
        )
        .item(
            PopupMenuItem::new("Disconnect")
                .action(Box::new(DisconnectConnection))
                .disabled(!is_connected)
                .on_click({
                    let state = state.clone();
                    move |_, _window, cx| {
                        if !state.read(cx).conn.active.contains_key(&connection_id) {
                            return;
                        }
                        AppCommands::disconnect(state.clone(), connection_id, cx);
                    }
                }),
        )
        .item(
            PopupMenuItem::new("Refresh Databases")
                .action(Box::new(RefreshView))
                .disabled(!is_connected)
                .on_click({
                    let state = state.clone();
                    move |_, _window, cx| {
                        AppCommands::refresh_databases(state.clone(), connection_id, cx);
                    }
                }),
        )
        .item(
            PopupMenuItem::new("Create Database...")
                .action(Box::new(CreateDatabase))
                .disabled(!is_connected)
                .on_click({
                    let state = state.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        open_create_database_dialog(state.clone(), window, cx);
                    }
                }),
        )
        .separator()
        .item(PopupMenuItem::new("Copy Name").action(Box::new(CopySelectionName)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                if let Some(conn) =
                    state.read(cx).connections.iter().find(|conn| conn.id == connection_id)
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(conn.name.clone()));
                }
            }
        }))
        .item(PopupMenuItem::new("Copy URI").action(Box::new(CopyConnectionUri)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                if let Some(conn) =
                    state.read(cx).connections.iter().find(|conn| conn.id == connection_id)
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(conn.uri.clone()));
                }
            }
        }));

    menu
}

fn build_database_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    node_id: TreeNodeId,
    database: String,
    is_loading: bool,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let database_for_select = database.clone();
    let database_for_create = database.clone();
    let database_for_refresh = database.clone();
    let database_for_drop = database.clone();
    let database_for_copy = database;

    menu = menu
        .item(PopupMenuItem::new("Select Database").action(Box::new(OpenSelection)).on_click({
            let state = state.clone();
            let connection_id = node_id.connection_id();
            move |_, _window, cx| {
                state.update(cx, |state, cx| {
                    state.select_connection(Some(connection_id), cx);
                    state.select_database(database_for_select.clone(), cx);
                });
            }
        }))
        .item(
            PopupMenuItem::new("Create Collection...").action(Box::new(CreateCollection)).on_click(
                {
                    let state = state.clone();
                    let connection_id = node_id.connection_id();
                    let database = database_for_create.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        open_create_collection_dialog(state.clone(), database.clone(), window, cx);
                    }
                },
            ),
        )
        .item(
            PopupMenuItem::new("Refresh Collections")
                .action(Box::new(RefreshView))
                .disabled(is_loading)
                .on_click({
                    let state = state.clone();
                    let sidebar = sidebar.clone();
                    let node_id = node_id.clone();
                    let connection_id = node_id.connection_id();
                    move |_, _window, cx| {
                        sidebar.update(cx, |sidebar, cx| {
                            sidebar.loading_databases.insert(node_id.clone());
                            cx.notify();
                        });
                        AppCommands::load_collections(
                            state.clone(),
                            connection_id,
                            database_for_refresh.clone(),
                            cx,
                        );
                    }
                }),
        )
        .item(PopupMenuItem::new("Drop Database...").action(Box::new(DeleteSelection)).on_click({
            let state = state.clone();
            let connection_id = node_id.connection_id();
            let database = database_for_drop.clone();
            move |_, window, cx| {
                let message = format!("Drop database \"{database}\"? This cannot be undone.");
                open_confirm_dialog(window, cx, "Drop database", message, "Drop", true, {
                    let state = state.clone();
                    let database = database.clone();
                    move |_window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        AppCommands::drop_database(state.clone(), database.clone(), cx);
                    }
                });
            }
        }))
        .separator()
        .item(PopupMenuItem::new("Copy Name").action(Box::new(CopySelectionName)).on_click({
            move |_, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(database_for_copy.clone()));
            }
        }));

    menu
}

fn build_collection_menu(
    mut menu: PopupMenu,
    state: Entity<AppState>,
    connection_id: Uuid,
    database: String,
    collection: String,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let database_for_open = database.clone();
    let collection_for_open = collection.clone();
    let database_for_preview = database.clone();
    let collection_for_preview = collection.clone();
    let session_key = SessionKey::new(connection_id, database.clone(), collection.clone());
    let label = format!("{}/{}", database, collection);

    menu = menu
        .item(PopupMenuItem::new("Open").action(Box::new(OpenSelection)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                state.update(cx, |state, cx| {
                    state.select_connection(Some(connection_id), cx);
                    state.select_collection(
                        database_for_open.clone(),
                        collection_for_open.clone(),
                        cx,
                    );
                });
            }
        }))
        .item(PopupMenuItem::new("Open Preview").action(Box::new(OpenSelectionPreview)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                state.update(cx, |state, cx| {
                    state.select_connection(Some(connection_id), cx);
                    state.preview_collection(
                        database_for_preview.clone(),
                        collection_for_preview.clone(),
                        cx,
                    );
                });
            }
        }))
        .item(PopupMenuItem::new("Refresh Documents").action(Box::new(RefreshView)).on_click({
            let state = state.clone();
            move |_, _window, cx| {
                AppCommands::load_documents_for_session(state.clone(), session_key.clone(), cx);
            }
        }))
        .item(
            PopupMenuItem::new("Rename Collection...").action(Box::new(RenameCollection)).on_click(
                {
                    let state = state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.select_connection(Some(connection_id), cx);
                        });
                        open_rename_collection_dialog(
                            state.clone(),
                            database.clone(),
                            collection.clone(),
                            window,
                            cx,
                        );
                    }
                },
            ),
        )
        .item(PopupMenuItem::new("Drop Collection...").action(Box::new(DeleteSelection)).on_click(
            {
                let state = state.clone();
                let database = database.clone();
                let collection = collection.clone();
                move |_, window, cx| {
                    let message = format!(
                        "Drop collection \"{database}.{collection}\"? This cannot be undone."
                    );
                    open_confirm_dialog(window, cx, "Drop collection", message, "Drop", true, {
                        let state = state.clone();
                        let database = database.clone();
                        let collection = collection.clone();
                        move |_window, cx| {
                            state.update(cx, |state, cx| {
                                state.select_connection(Some(connection_id), cx);
                            });
                            AppCommands::drop_collection(
                                state.clone(),
                                database.clone(),
                                collection.clone(),
                                cx,
                            );
                        }
                    });
                }
            },
        ))
        .separator()
        .item(PopupMenuItem::new("Copy Name").action(Box::new(CopySelectionName)).on_click({
            move |_, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(label.clone()));
            }
        }));

    menu
}

fn open_create_database_dialog(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let db_state =
        cx.new(|cx| InputState::new(window, cx).placeholder("database_name").default_value(""));
    let col_state = cx.new(|cx| {
        InputState::new(window, cx).placeholder("collection_name").default_value("default")
    });

    let db_state_save = db_state.clone();
    let col_state_save = col_state.clone();
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog
            .title("Create Database")
            .min_w(px(420.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Database name"),
                            )
                            .child(Input::new(&db_state)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_primary())
                                    .child("Initial collection"),
                            )
                            .child(Input::new(&col_state)),
                    ),
            )
            .footer({
                let state = state.clone();
                let db_state = db_state_save.clone();
                let col_state = col_state_save.clone();
                move |_ok_fn, _cancel_fn, _window: &mut Window, _cx: &mut App| {
                    let state = state.clone();
                    let db_state = db_state.clone();
                    let col_state = col_state.clone();
                    vec![
                        Button::new("cancel-db")
                            .label("Cancel")
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        Button::new("create-db")
                            .primary()
                            .label("Create")
                            .on_click({
                                let state = state.clone();
                                let db_state = db_state.clone();
                                let col_state = col_state.clone();
                                move |_, window, cx| {
                                    let db = db_state.read(cx).value().to_string();
                                    let col = col_state.read(cx).value().to_string();
                                    if db.trim().is_empty() || col.trim().is_empty() {
                                        return;
                                    }
                                    AppCommands::create_database(
                                        state.clone(),
                                        db.trim().to_string(),
                                        col.trim().to_string(),
                                        cx,
                                    );
                                    window.close_dialog(cx);
                                }
                            })
                            .into_any_element(),
                    ]
                }
            })
    });
}

fn open_create_collection_dialog(
    state: Entity<AppState>,
    database: String,
    window: &mut Window,
    cx: &mut App,
) {
    let col_state =
        cx.new(|cx| InputState::new(window, cx).placeholder("collection_name").default_value(""));
    let col_state_save = col_state.clone();
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog
            .title(format!("Create Collection in {database}"))
            .min_w(px(420.0))
            .child(
                div().flex().flex_col().gap(spacing::md()).p(spacing::md()).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors::text_primary())
                                .child("Collection name"),
                        )
                        .child(Input::new(&col_state)),
                ),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let col_state = col_state_save.clone();
                move |_ok_fn, _cancel_fn, _window: &mut Window, _cx: &mut App| {
                    let state = state.clone();
                    let database = database.clone();
                    let col_state = col_state.clone();
                    vec![
                        Button::new("cancel-collection")
                            .label("Cancel")
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        Button::new("create-collection")
                            .primary()
                            .label("Create")
                            .on_click({
                                let state = state.clone();
                                let database = database.clone();
                                let col_state = col_state.clone();
                                move |_, window, cx| {
                                    let col = col_state.read(cx).value().to_string();
                                    if col.trim().is_empty() {
                                        return;
                                    }
                                    AppCommands::create_collection(
                                        state.clone(),
                                        database.clone(),
                                        col.trim().to_string(),
                                        cx,
                                    );
                                    window.close_dialog(cx);
                                }
                            })
                            .into_any_element(),
                    ]
                }
            })
    });
}

fn open_rename_collection_dialog(
    state: Entity<AppState>,
    database: String,
    collection: String,
    window: &mut Window,
    cx: &mut App,
) {
    let name_state = cx.new(|cx| {
        InputState::new(window, cx).placeholder("collection_name").default_value(collection.clone())
    });
    let name_state_save = name_state.clone();
    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog
            .title(format!("Rename Collection {database}.{collection}"))
            .min_w(px(420.0))
            .child(
                div().flex().flex_col().gap(spacing::md()).p(spacing::md()).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors::text_primary())
                                .child("New collection name"),
                        )
                        .child(Input::new(&name_state)),
                ),
            )
            .footer({
                let state = state.clone();
                let database = database.clone();
                let collection = collection.clone();
                let name_state = name_state_save.clone();
                move |_ok_fn, _cancel_fn, _window: &mut Window, _cx: &mut App| {
                    let state = state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    let name_state = name_state.clone();
                    vec![
                        Button::new("cancel-rename-collection")
                            .label("Cancel")
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        Button::new("rename-collection")
                            .primary()
                            .label("Rename")
                            .on_click({
                                let state = state.clone();
                                let database = database.clone();
                                let collection = collection.clone();
                                let name_state = name_state.clone();
                                move |_, window, cx| {
                                    let new_name = name_state.read(cx).value().to_string();
                                    let new_name = new_name.trim();
                                    if new_name.is_empty() || new_name == collection.as_str() {
                                        return;
                                    }
                                    AppCommands::rename_collection(
                                        state.clone(),
                                        database.clone(),
                                        collection.clone(),
                                        new_name.to_string(),
                                        cx,
                                    );
                                    window.close_dialog(cx);
                                }
                            })
                            .into_any_element(),
                    ]
                }
            })
    });
}
