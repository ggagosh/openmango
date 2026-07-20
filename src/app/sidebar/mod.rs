use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::input::InputState;
use uuid::Uuid;

use crate::components::{
    ConnectionManager, open_confirm_dialog, request_disconnect_connection,
    request_preview_collection, request_remove_connection, request_unsaved_action,
};
use crate::keyboard::FocusContent;
use crate::models::{ActiveConnection, SavedConnection, TreeNodeId};
use crate::state::{
    AppCommands, AppEvent, AppState, CopiedTreeItem, DatabaseKey, StatusMessage, TransferMode,
    TransferScope,
};

use super::dialogs::open_rename_collection_dialog;
use super::search::{SidebarSearchCandidate, SidebarSearchResult, search_results};
use super::sidebar_model::{SidebarModel, TYPEAHEAD_RESET_DELAY};

mod keys;
mod view;

// =============================================================================
// Sidebar Component
// =============================================================================

const SIDEBAR_DEFAULT_WIDTH: Pixels = px(260.0);
const SIDEBAR_MIN_WIDTH: Pixels = px(180.0);
const SIDEBAR_MAX_WIDTH: Pixels = px(500.0);
const KEYBOARD_PREVIEW_DELAY: Duration = Duration::from_millis(140);

/// Memoizes sidebar fuzzy-search results so the full candidate scan + fuzzy
/// match doesn't re-run on every incidental sidebar re-render. Invalidated when
/// the query changes or the (Rc-identity) connection/database/collection source
/// changes.
struct SidebarSearchCache {
    query: String,
    connections: Rc<Vec<SavedConnection>>,
    active: Rc<HashMap<Uuid, ActiveConnection>>,
    results: Vec<SidebarSearchResult>,
}

pub(crate) struct Sidebar {
    state: Entity<AppState>,
    model: SidebarModel,
    search_state: Entity<InputState>,
    pub(crate) focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    width: Pixels,
    collapsed: bool,
    sticky_connection_index: Option<usize>,
    typeahead_clear_task: Option<Task<()>>,
    typeahead_generation: u64,
    keyboard_preview_task: Option<Task<()>>,
    keyboard_preview_generation: u64,
    last_tree_click: Option<(TreeNodeId, Instant)>,
    // Rc-cached: only refreshed on structural changes, cheap to capture in render closures.
    cached_connections: Rc<Vec<SavedConnection>>,
    cached_active: Rc<HashMap<Uuid, ActiveConnection>>,
    search_cache: Option<SidebarSearchCache>,
    _subscriptions: Vec<Subscription>,
}

impl Sidebar {
    pub(crate) fn new(
        state: Entity<AppState>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (cached_connections, cached_active) = {
            let state_ref = state.read(cx);
            (
                Rc::new(state_ref.connections_snapshot()),
                Rc::new(state_ref.active_connections_snapshot()),
            )
        };
        let model = SidebarModel::new((*cached_connections).clone(), (*cached_active).clone());
        let search_state = cx.new(|cx| {
            InputState::new(_window, cx).placeholder("Search connections, databases, collections")
        });

        let mut subscriptions = vec![];

        // Subscribe to AppState events for targeted tree updates (Phase 5.5)
        subscriptions.push(cx.subscribe_in(&state, _window, move |this, _, event, window, cx| {
            match event {
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
                        let state = this.state.clone();
                        let sidebar = cx.entity();
                        window.defer(cx, move |window, cx| {
                            request_unsaved_action(
                                state.clone(),
                                crate::state::UnsavedScope::Workspace,
                                window,
                                cx,
                                move |_window, cx| {
                                    state.update(cx, |state, cx| {
                                        state.restore_workspace_after_connect(cx);
                                    });
                                    sidebar.update(cx, |sidebar, cx| {
                                        sidebar.restore_workspace_expansion(cx);
                                    });
                                },
                            );
                        });
                    }
                    this.refresh_tree(cx);
                }
                AppEvent::Disconnected(connection_id) => {
                    if this.model.connecting_connection == Some(*connection_id) {
                        this.model.connecting_connection = None;
                    }
                    this.model.loading_databases.clear();
                    this.model.clear_selection();
                    this.refresh_tree(cx);
                }
                AppEvent::ConnectionFailed(_) => {
                    this.model.connecting_connection = None;
                    this.model.loading_databases.clear();
                    this.model.clear_selection();
                    cx.notify();
                }
                AppEvent::DocumentsLoaded { .. }
                | AppEvent::DocumentsLoadFailed { .. }
                | AppEvent::DocumentInserted { .. }
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
                | AppEvent::DocumentsUpdateFailed { .. }
                | AppEvent::AggregationCompleted { .. }
                | AppEvent::AggregationFailed { .. }
                | AppEvent::ExplainStarted { .. }
                | AppEvent::ExplainCompleted { .. }
                | AppEvent::ExplainFailed { .. }
                | AppEvent::TransferPreviewLoaded { .. }
                | AppEvent::TransferStarted { .. }
                | AppEvent::TransferCompleted { .. }
                | AppEvent::TransferFailed { .. }
                | AppEvent::TransferCancelled { .. }
                | AppEvent::DatabaseTransferStarted { .. }
                | AppEvent::CollectionProgressUpdate { .. }
                | AppEvent::SchemaAnalyzed { .. }
                | AppEvent::SchemaFailed { .. }
                | AppEvent::UpdateAvailable { .. } => {}
                AppEvent::ViewChanged => {
                    this.sync_selection_from_state(cx);
                }
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

        subscriptions.push(cx.observe_keystrokes(|this, event, window, cx| {
            if event.action.is_some() {
                return;
            }
            if !this.focus_handle.contains_focused(window, cx) {
                return;
            }
            if this.focus_handle.is_focused(window) {
                return;
            }
            if this.search_state.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            if this.model.search_open || this.collapsed {
                return;
            }
            let ks = &event.keystroke;
            let key = ks.key.to_lowercase();
            let has_query = !this.model.typeahead_query.is_empty();

            if key == "enter" || key == "return" {
                this.clear_typeahead(cx);
                this.handle_open_selection(window, cx);
                return;
            }
            if key == "escape" && has_query {
                this.clear_typeahead(cx);
                return;
            }
            if (key == "backspace" || key == "delete") && (has_query || this.typeahead_is_active())
            {
                this.delete_typeahead_char(cx);
                return;
            }
            if key == "up" || key == "arrowup" {
                this.move_sidebar_selection(-1, cx);
                return;
            }
            if key == "down" || key == "arrowdown" {
                this.move_sidebar_selection(1, cx);
                return;
            }
            if key == "home" {
                this.select_sidebar_first(cx);
                return;
            }
            if key == "end" {
                this.select_sidebar_last(cx);
                return;
            }
            if key == "pageup" {
                this.move_sidebar_page(-1, cx);
                return;
            }
            if key == "pagedown" {
                this.move_sidebar_page(1, cx);
                return;
            }
            this.handle_typeahead_keystroke(ks, cx);
        }));

        let sidebar = Self {
            state,
            model,
            search_state,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::default(),
            width: SIDEBAR_DEFAULT_WIDTH,
            collapsed: false,
            sticky_connection_index: None,
            typeahead_clear_task: None,
            typeahead_generation: 0,
            keyboard_preview_task: None,
            keyboard_preview_generation: 0,
            last_tree_click: None,
            cached_connections,
            cached_active,
            search_cache: None,
            _subscriptions: subscriptions,
        };

        if let Some(connection_id) = sidebar.state.read(cx).workspace_autoconnect_id() {
            sidebar.state.update(cx, |state, cx| {
                state.connect_when_secrets_ready(connection_id, cx);
            });
        }

        sidebar
    }

    pub(crate) fn width(&self) -> Pixels {
        if self.collapsed { px(0.0) } else { self.width }
    }

    pub(crate) fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub(crate) fn set_width(&mut self, w: Pixels) {
        self.width = w.max(SIDEBAR_MIN_WIDTH).min(SIDEBAR_MAX_WIDTH);
    }

    pub(crate) fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
    }

    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        let (selected_connection, selected_db, selected_col) = {
            let state_ref = self.state.read(cx);
            self.cached_connections = Rc::new(state_ref.connections_snapshot());
            self.cached_active = Rc::new(state_ref.active_connections_snapshot());
            (
                state_ref.selected_connection_id(),
                state_ref.selected_database_name(),
                state_ref.selected_collection_name(),
            )
        };

        if self.model.selected_tree_id.is_none() {
            self.model.ensure_selection_from_state(selected_connection, selected_db, selected_col);
        }

        if let Some(ix) = self.model.refresh_entries(&self.cached_connections, &self.cached_active)
        {
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

    pub(crate) fn reload_selected_database_if_focused(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.model.search_open || !self.focus_handle.contains_focused(window, cx) {
            return false;
        }

        self.reload_selected_database(cx)
    }

    fn reload_selected_database(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(TreeNodeId::Database { connection, database }) =
            self.model.selected_tree_id.clone()
        else {
            return false;
        };
        if !self.state.read(cx).is_connected(connection) {
            return false;
        }

        let node_id = TreeNodeId::database(connection, database.clone());
        self.model.loading_databases.insert(node_id);
        cx.notify();
        AppCommands::reload_database(
            self.state.clone(),
            DatabaseKey::new(connection, database),
            cx,
        );
        true
    }

    pub(crate) fn handle_transfer_action(&mut self, mode: TransferMode, cx: &mut Context<Self>) {
        let Some(node_id) = self.model.selected_tree_id.clone() else {
            return;
        };

        match node_id {
            TreeNodeId::Database { connection, database } => {
                let state = self.state.clone();
                state.update(cx, |state, cx| {
                    state.open_transfer_tab_with_prefill(
                        connection,
                        database,
                        None,
                        TransferScope::Database,
                        mode,
                        cx,
                    );
                });
            }
            TreeNodeId::Collection { connection, database, collection } => {
                let state = self.state.clone();
                state.update(cx, |state, cx| {
                    state.open_transfer_tab_with_prefill(
                        connection,
                        database,
                        Some(collection),
                        TransferScope::Collection,
                        mode,
                        cx,
                    );
                });
            }
            _ => {}
        }
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

        if let Some(ix) =
            self.model.ensure_selection_from_state(connection_id, selected_db, selected_col)
        {
            self.scroll_handle.scroll_to_item(ix, gpui::ScrollStrategy::Center);
        }
        cx.notify();
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
        self.model.clear_selection();
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
        ConnectionManager::open_new(state, window, cx);
    }

    fn handle_open_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_typeahead(cx);
        self.cancel_keyboard_preview();
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
            window.dispatch_action(Box::new(FocusContent), cx);
        }
    }

    fn handle_open_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_keyboard_preview();
        let Some(node_id) = self.model.selected_tree_id.clone() else {
            return;
        };
        if node_id.is_collection()
            && let (Some(db), Some(col)) = (
                node_id.database_name().map(|db| db.to_string()),
                node_id.collection_name().map(|col| col.to_string()),
            )
        {
            request_preview_collection(
                self.state.clone(),
                node_id.connection_id(),
                db,
                col,
                window,
                cx,
            );
        }
    }

    fn handle_edit_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Connection(connection_id)) = self.model.selected_tree_id.clone()
        else {
            return;
        };
        ConnectionManager::open_selected(self.state.clone(), connection_id, window, cx);
    }

    fn handle_disconnect_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(TreeNodeId::Connection(connection_id)) = self.model.selected_tree_id.clone()
        else {
            return;
        };
        let is_active = self.state.read(cx).is_connected(connection_id);
        if is_active {
            request_disconnect_connection(self.state.clone(), connection_id, window, cx);
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

    pub(super) fn register_tree_click(&mut self, node_id: &TreeNodeId) -> bool {
        let now = Instant::now();
        let is_double = self.last_tree_click.as_ref().is_some_and(|(last_id, last_at)| {
            last_id == node_id && now.duration_since(*last_at) <= Duration::from_millis(350)
        });
        self.last_tree_click = Some((node_id.clone(), now));
        is_double
    }

    fn handle_copy_tree_item(&mut self, cx: &mut Context<Self>) {
        let Some(node_id) = self.model.selected_tree_id.clone() else {
            return;
        };

        match &node_id {
            TreeNodeId::Connection(connection_id) => {
                // For connections, just copy the name to OS clipboard (no internal clipboard)
                if let Some(name) = self.state.read(cx).connection_name(*connection_id) {
                    cx.write_to_clipboard(ClipboardItem::new_string(name));
                }
            }
            TreeNodeId::Database { connection, database } => {
                // Copy name to OS clipboard
                cx.write_to_clipboard(ClipboardItem::new_string(database.clone()));
                // Set internal clipboard
                self.state.update(cx, |state, cx| {
                    state.copied_tree_item = Some(CopiedTreeItem::Database {
                        connection_id: *connection,
                        database: database.clone(),
                    });
                    state.set_status_message(Some(StatusMessage::info(format!(
                        "Copied database: {}",
                        database
                    ))));
                    cx.notify();
                });
            }
            TreeNodeId::Collection { connection, database, collection } => {
                // Copy name to OS clipboard (as db/collection format)
                cx.write_to_clipboard(ClipboardItem::new_string(format!(
                    "{}/{}",
                    database, collection
                )));
                // Set internal clipboard
                self.state.update(cx, |state, cx| {
                    state.copied_tree_item = Some(CopiedTreeItem::Collection {
                        connection_id: *connection,
                        database: database.clone(),
                        collection: collection.clone(),
                    });
                    state.set_status_message(Some(StatusMessage::info(format!(
                        "Copied collection: {}.{}",
                        database, collection
                    ))));
                    cx.notify();
                });
            }
        }
    }

    fn handle_paste_tree_item(&mut self, cx: &mut Context<Self>) {
        let copied = self.state.read(cx).copied_tree_item.clone();
        let Some(item) = copied else {
            self.state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error("Nothing to paste")));
                cx.notify();
            });
            return;
        };

        // Verify the source connection still exists
        let source_connection_id = match &item {
            CopiedTreeItem::Database { connection_id, .. } => *connection_id,
            CopiedTreeItem::Collection { connection_id, .. } => *connection_id,
        };

        if self.state.read(cx).connection_by_id(source_connection_id).is_none() {
            self.state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Source connection no longer exists",
                )));
                state.copied_tree_item = None;
                cx.notify();
            });
            return;
        }

        // Get destination from current sidebar selection
        let (dest_connection_id, dest_database) = match &self.model.selected_tree_id {
            Some(TreeNodeId::Connection(conn_id)) => (Some(*conn_id), None),
            Some(TreeNodeId::Database { connection, database }) => {
                (Some(*connection), Some(database.clone()))
            }
            Some(TreeNodeId::Collection { connection, database, .. }) => {
                (Some(*connection), Some(database.clone()))
            }
            None => (None, None),
        };

        self.state.update(cx, |state, cx| match item {
            CopiedTreeItem::Database { connection_id, database } => {
                state.open_transfer_tab_for_paste(
                    connection_id,
                    database,
                    None,
                    dest_connection_id,
                    dest_database,
                    TransferScope::Database,
                    cx,
                );
            }
            CopiedTreeItem::Collection { connection_id, database, collection } => {
                state.open_transfer_tab_for_paste(
                    connection_id,
                    database,
                    Some(collection),
                    dest_connection_id,
                    dest_database,
                    TransferScope::Collection,
                    cx,
                );
            }
        });
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
                    move |window, cx| {
                        request_remove_connection(state.clone(), connection_id, window, cx);
                    }
                });
            }
            TreeNodeId::Database { connection, database } => {
                let message = format!("Drop database \"{database}\"? This cannot be undone.");
                open_confirm_dialog(window, cx, "Drop database", message, "Drop", true, {
                    let state = self.state.clone();
                    let database = database.clone();
                    move |_window, cx| {
                        AppCommands::drop_database(state.clone(), connection, database.clone(), cx);
                    }
                });
            }
            TreeNodeId::Collection { connection, database, collection } => {
                let message =
                    format!("Drop collection \"{database}.{collection}\"? This cannot be undone.");
                open_confirm_dialog(window, cx, "Drop collection", message, "Drop", true, {
                    let state = self.state.clone();
                    let database = database.clone();
                    let collection = collection.clone();
                    move |_window, cx| {
                        AppCommands::drop_collection(
                            state.clone(),
                            connection,
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

    fn handle_typeahead_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.model.search_open {
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return false;
        }
        let key = event.keystroke.key.to_lowercase();
        let key_char = event.keystroke.key_char.as_deref();
        if !self.model.handle_typeahead_key(&key, key_char) {
            return false;
        }
        self.finish_typeahead_key(&key, cx);
        cx.notify();
        true
    }

    fn handle_typeahead_keystroke(
        &mut self,
        keystroke: &Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.model.search_open || self.collapsed {
            return false;
        }
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return false;
        }
        let key = keystroke.key.to_lowercase();
        let key_char = keystroke.key_char.as_deref();
        if !self.model.handle_typeahead_key(&key, key_char) {
            return false;
        }
        self.finish_typeahead_key(&key, cx);
        cx.notify();
        true
    }

    fn clear_typeahead(&mut self, cx: &mut Context<Self>) {
        self.model.typeahead_query.clear();
        self.model.typeahead_last = None;
        self.typeahead_generation = self.typeahead_generation.wrapping_add(1);
        self.typeahead_clear_task = None;
        cx.notify();
    }

    fn typeahead_is_active(&self) -> bool {
        !self.model.typeahead_query.is_empty() || self.typeahead_clear_task.is_some()
    }

    fn should_ignore_delete_action(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.model.search_open || self.typeahead_is_active() || window.has_active_dialog(cx)
    }

    fn delete_typeahead_char(&mut self, cx: &mut Context<Self>) {
        if !self.model.typeahead_query.is_empty() {
            self.model.typeahead_query.pop();
            self.model.typeahead_last = Some(Instant::now());
            if !self.model.typeahead_query.is_empty() {
                self.select_typeahead_match(cx);
            }
        }
        self.schedule_typeahead_clear(cx);
        cx.notify();
    }

    fn finish_typeahead_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.model.typeahead_query.is_empty() {
            self.select_typeahead_match(cx);
            self.schedule_typeahead_clear(cx);
            return;
        }

        if key == "backspace" || key == "delete" {
            self.schedule_typeahead_clear(cx);
        } else {
            self.model.typeahead_last = None;
            self.typeahead_generation = self.typeahead_generation.wrapping_add(1);
            self.typeahead_clear_task = None;
        }
    }

    fn schedule_typeahead_clear(&mut self, cx: &mut Context<Self>) {
        self.typeahead_generation = self.typeahead_generation.wrapping_add(1);
        let generation = self.typeahead_generation;
        self.typeahead_clear_task = Some(cx.spawn(async move |entity, cx| {
            cx.background_executor().timer(TYPEAHEAD_RESET_DELAY).await;
            entity
                .update(cx, |this, cx| {
                    if this.typeahead_generation != generation {
                        return;
                    }
                    this.model.typeahead_query.clear();
                    this.model.typeahead_last = None;
                    this.typeahead_clear_task = None;
                    cx.notify();
                })
                .ok();
        }));
    }

    fn select_typeahead_match(&mut self, cx: &mut Context<Self>) {
        if let Some((ix, _node_id)) = self.model.select_typeahead_match() {
            self.scroll_handle.scroll_to_item(ix, gpui::ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn cancel_keyboard_preview(&mut self) {
        self.keyboard_preview_generation = self.keyboard_preview_generation.wrapping_add(1);
        self.keyboard_preview_task = None;
    }

    fn schedule_keyboard_preview(&mut self, node_id: TreeNodeId, cx: &mut Context<Self>) {
        self.keyboard_preview_generation = self.keyboard_preview_generation.wrapping_add(1);
        let generation = self.keyboard_preview_generation;
        let state = self.state.clone();

        self.keyboard_preview_task = Some(cx.spawn(async move |entity, cx| {
            cx.background_executor().timer(KEYBOARD_PREVIEW_DELAY).await;
            entity
                .update(cx, move |this, cx| {
                    if this.keyboard_preview_generation != generation
                        || this.model.selected_tree_id.as_ref() != Some(&node_id)
                    {
                        return;
                    }

                    this.keyboard_preview_task = None;
                    state.update(cx, |state, cx| match node_id {
                        TreeNodeId::Connection(connection_id) => {
                            state.select_connection(Some(connection_id), cx);
                        }
                        TreeNodeId::Database { connection, database } => {
                            state.select_connection(Some(connection), cx);
                            state.select_database(database, cx);
                        }
                        TreeNodeId::Collection { connection, database, collection } => {
                            if let Some(preview) = state.preview_tab().cloned()
                                && !state
                                    .unsaved_inventory(&crate::state::UnsavedScope::Preview(
                                        preview,
                                    ))
                                    .is_empty()
                            {
                                return;
                            }
                            state.select_connection(Some(connection), cx);
                            state.preview_collection(database, collection, cx);
                        }
                    });
                })
                .ok();
        }));
    }

    fn move_sidebar_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some((next, node_id)) = self.model.move_sidebar_selection(delta) else {
            return;
        };
        self.apply_sidebar_selection(next, node_id, true, cx);
    }

    fn move_sidebar_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some((next, node_id)) = self.model.move_sidebar_page(delta, 10) else {
            return;
        };
        self.apply_sidebar_selection(next, node_id, true, cx);
    }

    fn select_sidebar_first(&mut self, cx: &mut Context<Self>) {
        let Some((next, node_id)) = self.model.select_first() else {
            return;
        };
        self.apply_sidebar_selection(next, node_id, true, cx);
    }

    fn select_sidebar_last(&mut self, cx: &mut Context<Self>) {
        let Some((next, node_id)) = self.model.select_last() else {
            return;
        };
        self.apply_sidebar_selection(next, node_id, true, cx);
    }

    fn select_sidebar_node(
        &mut self,
        node_id: TreeNodeId,
        preview: bool,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let next = self.model.select_node(node_id.clone())?;
        self.apply_sidebar_selection(next, node_id, preview, cx);
        Some(next)
    }

    fn apply_sidebar_selection(
        &mut self,
        next: usize,
        node_id: TreeNodeId,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        self.scroll_handle.scroll_to_item(next, gpui::ScrollStrategy::Center);
        cx.notify();
        if preview {
            self.schedule_keyboard_preview(node_id, cx);
        }
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
        self.cancel_keyboard_preview();
        self.model.clear_selection();
        self.model.expanded_nodes.insert(TreeNodeId::connection(result.connection_id));
        if let Some(database) = result.database.as_ref() {
            self.model
                .expanded_nodes
                .insert(TreeNodeId::database(result.connection_id, database.clone()));
        }
        if result.node_id.is_connection() {
            self.model.expanded_nodes.insert(result.node_id.clone());
        }
        self.persist_expanded_nodes(cx);
        self.refresh_tree(cx);
        self.select_sidebar_node(result.node_id.clone(), false, cx);

        let opened_collection = result.collection.is_some();
        match (result.database.clone(), result.collection.clone()) {
            (Some(database), Some(collection)) => {
                self.state.update(cx, |state, cx| {
                    state.select_connection(Some(result.connection_id), cx);
                    state.select_collection(database, collection, cx);
                });
            }
            (Some(database), None) => {
                let database_node = TreeNodeId::database(result.connection_id, database.clone());
                let should_load = self
                    .state
                    .read(cx)
                    .active_connection_by_id(result.connection_id)
                    .is_some_and(|conn| !conn.collections.contains_key(&database));
                if should_load && !self.model.loading_databases.contains(&database_node) {
                    self.model.loading_databases.insert(database_node.clone());
                    AppCommands::load_collections(
                        self.state.clone(),
                        result.connection_id,
                        database.clone(),
                        cx,
                    );
                }
                self.state.update(cx, |state, cx| {
                    state.select_connection(Some(result.connection_id), cx);
                    state.select_database(database, cx);
                });
            }
            (None, None) => {
                self.state.update(cx, |state, cx| {
                    state.select_connection(Some(result.connection_id), cx);
                });
            }
            (None, Some(_)) => {}
        }
        self.close_search(window, cx);
        if opened_collection {
            window.dispatch_action(Box::new(FocusContent), cx);
        }
    }

    fn search_results(&mut self, query: &str, _cx: &mut Context<Self>) -> Vec<SidebarSearchResult> {
        // Skip the candidate scan + fuzzy match entirely when neither the query
        // nor the (Rc-identity) source changed since the last call.
        if let Some(cache) = &self.search_cache
            && cache.query == query
            && Rc::ptr_eq(&cache.connections, &self.cached_connections)
            && Rc::ptr_eq(&cache.active, &self.cached_active)
        {
            return cache.results.clone();
        }

        let mut candidates = Vec::new();
        for connection in self.cached_connections.iter() {
            let Some(active) = self.cached_active.get(&connection.id) else {
                continue;
            };
            candidates
                .push(SidebarSearchCandidate::connection(connection.id, connection.name.clone()));
            for database in &active.databases {
                candidates.push(SidebarSearchCandidate::database(
                    connection.id,
                    connection.name.clone(),
                    database.clone(),
                ));
                if let Some(collections) = active.collections.get(database) {
                    for collection in collections {
                        candidates.push(SidebarSearchCandidate::collection(
                            connection.id,
                            connection.name.clone(),
                            database.clone(),
                            collection.clone(),
                        ));
                    }
                }
            }
        }

        let results = search_results(query, candidates);
        self.search_cache = Some(SidebarSearchCache {
            query: query.to_string(),
            connections: self.cached_connections.clone(),
            active: self.cached_active.clone(),
            results: results.clone(),
        });
        results
    }
}
