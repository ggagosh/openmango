use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use uuid::Uuid;

use crate::actions::model::ActionStatus;
use crate::components::{
    ConnectionManager, WriteConfirmation, open_confirm_dialog, request_connection_write,
    request_disconnect_connection, request_preview_collection, request_remove_connection,
    request_unsaved_action,
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
#[cfg(test)]
mod tests;
mod view;

// =============================================================================
// Sidebar Component
// =============================================================================

const SIDEBAR_DEFAULT_WIDTH: Pixels = px(260.0);
const SIDEBAR_MIN_WIDTH: Pixels = px(180.0);
const SIDEBAR_MAX_WIDTH: Pixels = px(500.0);
const KEYBOARD_PREVIEW_DELAY: Duration = Duration::from_millis(140);
/// Every tree row is exactly this tall. The pinned rows and paging do their arithmetic with
/// it; the scroll handle only reports the viewport and content sizes, never a row's. In rems,
/// so a row grows with its label when the base font does (24px at the default 16px).
const ROW_HEIGHT: Rems = rems(1.5);
// ponytail: the results list is not virtualized, so it shows the best matches only. Swap it
// for a uniform_list if people need to page through more.
const SEARCH_RESULTS_LIMIT: usize = 50;

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
    search_scroll_handle: ScrollHandle,
    /// The view node last revealed, so a repeated `ViewChanged` for the same view does not
    /// reopen what the user has since collapsed.
    last_revealed: Option<TreeNodeId>,
    /// A reveal waiting for its row to load.
    pending_reveal: Option<TreeNodeId>,
    /// Counted when agent activity changes; the broker reads disk, which render must not.
    pending_agent_actions: usize,
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
                | AppEvent::ConnectionSaveFinished { .. }
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
                    this.model.expanded_nodes.insert(TreeNodeId::connection(*connection_id));
                    this.persist_expanded_nodes(cx);
                    this.refresh_tree(cx);
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
                AppEvent::ConnectionFailed { connection_id, .. } => {
                    if this.model.connecting_connection == Some(*connection_id) {
                        this.model.connecting_connection = None;
                    }
                    this.model.loading_databases.clear();
                    this.model.clear_selection();
                    this.refresh_tree(cx);
                }
                AppEvent::DocumentsLoaded { .. }
                | AppEvent::DocumentsLoadFailed { .. }
                | AppEvent::DocumentInserted { .. }
                | AppEvent::DocumentInsertFailed { .. }
                | AppEvent::DocumentsInserted { .. }
                | AppEvent::DocumentsInsertFailed { .. }
                | AppEvent::DocumentDraftChanged { .. }
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
                AppEvent::AgentActivityChanged => {
                    this.pending_agent_actions = Self::count_pending_agent_actions(&this.state, cx);
                    cx.notify();
                }
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
            move |cx, _| {
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
            if this.handle_nav_key(&key, ks.modifiers, cx) {
                return;
            }
            this.handle_typeahead_keystroke(ks, cx);
        }));

        let pending_agent_actions = Self::count_pending_agent_actions(&state, cx);
        let sidebar = Self {
            state,
            model,
            search_state,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::default(),
            width: SIDEBAR_DEFAULT_WIDTH,
            collapsed: false,
            search_scroll_handle: ScrollHandle::new(),
            last_revealed: None,
            pending_reveal: None,
            pending_agent_actions,
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

    fn count_pending_agent_actions(state: &Entity<AppState>, cx: &App) -> usize {
        state
            .read(cx)
            .action_broker()
            .list_all()
            .unwrap_or_default()
            .into_iter()
            .filter(|action| action.status == ActionStatus::PendingApproval)
            .count()
    }

    fn view_node(&self, cx: &App) -> Option<TreeNodeId> {
        let state = self.state.read(cx);
        SidebarModel::node_for_view(
            state.selected_connection_id(),
            state.selected_database_name(),
            state.selected_collection_name(),
        )
    }

    /// Rebuilds the rows from the cached snapshot: all an expansion change needs. It neither
    /// scrolls nor touches what is expanded, so the list stays put under the pointer.
    fn rebuild_entries(&mut self, cx: &mut Context<Self>) {
        self.model.refresh_entries(&self.cached_connections, &self.cached_active);
        cx.notify();
    }

    /// Re-reads connections and their contents from the state, then rebuilds the rows.
    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        {
            let state_ref = self.state.read(cx);
            self.cached_connections = Rc::new(state_ref.connections_snapshot());
            self.cached_active = Rc::new(state_ref.active_connections_snapshot());
        }
        if self.model.selected_tree_id.is_none() {
            self.model.selected_tree_id = self.view_node(cx);
        }
        self.rebuild_entries(cx);

        // A row revealed before its parent finished loading is selected now that it exists.
        if let Some(node_id) = self.pending_reveal.take() {
            match self.model.select_node(node_id.clone()) {
                Some(ix) => self.scroll_to_row(ix, ScrollStrategy::Center),
                None => self.pending_reveal = Some(node_id),
            }
        }
    }

    /// Opens whatever hides `node_id`, selects it and brings it into view.
    fn reveal_node(&mut self, node_id: TreeNodeId, cx: &mut Context<Self>) {
        if self.model.expand_ancestors(&node_id) {
            self.persist_expanded_nodes(cx);
            self.rebuild_entries(cx);
        }
        self.pending_reveal = None;
        match self.model.select_node(node_id.clone()) {
            Some(ix) => self.scroll_to_row(ix, ScrollStrategy::Center),
            None => {
                self.model.select_nearest(node_id.clone());
                self.pending_reveal = Some(node_id);
            }
        }
        cx.notify();
    }

    /// The ancestor rows to pin over the top of the list: the connection, then the database,
    /// of whatever has scrolled beneath them. Each level looks at the first row its
    /// predecessors leave uncovered. Read from the live scroll offset during render, so the
    /// pins never trail the scroll by a frame.
    pub(super) fn sticky_rows(&self, cx: &App) -> Vec<usize> {
        let offset = self.scroll_handle.0.borrow().base_handle.offset().y;
        let first = (-offset / Self::row_height(cx)).floor().max(0.0) as usize;
        let mut pinned = Vec::new();
        for depth in 0..2 {
            let covered = first + depth;
            match SidebarModel::ancestor_index(&self.model.entries, covered, depth) {
                Some(ix) if ix < covered => pinned.push(ix),
                _ => break,
            }
        }
        pinned
    }

    /// A row's height in pixels. `Root` sets the window's rem size from the theme's base font,
    /// so that font is the length a rem resolves against.
    fn row_height(cx: &App) -> Pixels {
        ROW_HEIGHT.to_pixels(cx.theme().font_size)
    }

    /// Scrolls a row into view, clear of the ancestor rows pinned over the top of the list.
    fn scroll_to_row(&self, ix: usize, strategy: ScrollStrategy) {
        let pinned = self.model.entries.get(ix).map_or(0, |entry| entry.depth);
        self.scroll_handle.scroll_to_item_with_offset(ix, strategy, pinned);
    }

    /// The one place a row opens or closes: the chevron, the arrow keys and Enter all land here.
    pub(super) fn set_node_expanded(
        &mut self,
        node_id: &TreeNodeId,
        expanded: bool,
        recursive: bool,
        cx: &mut Context<Self>,
    ) {
        if self.model.set_expanded(node_id, expanded, recursive) {
            self.persist_expanded_nodes(cx);
            self.rebuild_entries(cx);
        }
        if expanded {
            self.load_collections_if_needed(node_id, cx);
        }
    }

    /// Starts loading a database's collections unless they are here or already on their way.
    fn load_collections_if_needed(&mut self, node_id: &TreeNodeId, cx: &mut Context<Self>) {
        let TreeNodeId::Database { connection, database } = node_id else {
            return;
        };
        let loaded = self
            .state
            .read(cx)
            .active_connection_by_id(*connection)
            .is_none_or(|conn| conn.collections.contains_key(database));
        if loaded || !self.model.loading_databases.insert(node_id.clone()) {
            return;
        }
        cx.notify();
        AppCommands::load_collections(self.state.clone(), *connection, database.clone(), cx);
    }

    /// Selects an open connection's row, scrolls to it and moves focus to the tree.
    pub(crate) fn reveal_connection(
        &mut self,
        connection_id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.collapsed {
            self.toggle_collapsed();
        }
        self.reveal_node(TreeNodeId::connection(connection_id), cx);
        window.focus(&self.focus_handle, cx);
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

    pub(crate) fn handle_transfer_action(
        &mut self,
        mode: TransferMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                window.dispatch_action(Box::new(FocusContent), cx);
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
                window.dispatch_action(Box::new(FocusContent), cx);
            }
            _ => {}
        }
    }

    /// Reveals the row for the view the app switched to. `ViewChanged` fires for much more
    /// than that, so an unchanged view reveals nothing: what the user collapsed stays closed.
    fn sync_selection_from_state(&mut self, cx: &mut Context<Self>) {
        let Some(node_id) = self.view_node(cx) else {
            return;
        };
        if self.last_revealed.as_ref() == Some(&node_id) {
            return;
        }
        self.last_revealed = Some(node_id.clone());
        self.reveal_node(node_id, cx);
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
        let expanded: Vec<TreeNodeId> = {
            let state_ref = self.state.read(cx);
            let Some(connection_id) =
                state_ref.workspace.last_connection_id.or(state_ref.selected_connection_id())
            else {
                return;
            };
            if state_ref.active_connection_by_id(connection_id).is_none() {
                return;
            }
            state_ref
                .workspace
                .expanded_nodes
                .iter()
                .filter_map(|id| TreeNodeId::from_tree_id(id))
                .filter(|node| node.connection_id() == connection_id)
                .collect()
        };

        // Added to what is open, not swapped in: other connections keep their expansion.
        self.model.expanded_nodes.extend(expanded.iter().cloned());
        self.refresh_tree(cx);
        for node_id in &expanded {
            self.load_collections_if_needed(node_id, cx);
        }
        // Land on the restored view's row; a collection waits for its database to load.
        self.last_revealed = None;
        self.sync_selection_from_state(cx);
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
                self.set_node_expanded(&node_id, true, false, cx);
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
            self.set_node_expanded(&node_id, true, false, cx);
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

    fn handle_open_forge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_keyboard_preview();
        self.clear_typeahead(cx);
        let node_id = if self.model.search_open {
            let query = self.search_state.read(cx).value().to_string();
            let results = self.search_results(&query, cx);
            self.model
                .search_selected
                .and_then(|ix| results.get(ix))
                .or_else(|| results.first())
                .map(|result| result.node_id.clone())
        } else {
            self.model.selected_tree_id.clone()
        };
        let Some(node_id) = node_id else {
            return;
        };
        let Some(database) = node_id.database_name() else {
            return;
        };
        self.state.update(cx, |state, cx| {
            state.open_forge_tab(
                node_id.connection_id(),
                database.to_string(),
                node_id.collection_name().map(str::to_string),
                cx,
            );
        });
        if self.model.search_open {
            self.close_search(window, cx);
        }
        window.dispatch_action(Box::new(FocusContent), cx);
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
                let state = self.state.clone();
                let state_for_write = state.clone();
                request_connection_write(
                    state,
                    crate::components::WriteRequest::new(
                        connection,
                        database.clone(),
                        "Drop a database",
                        Some(WriteConfirmation {
                            title: "Drop database".into(),
                            message,
                            confirm_label: "Drop".into(),
                            destructive: true,
                        }),
                    ),
                    window,
                    cx,
                    move |_window, cx| {
                        AppCommands::drop_database(state_for_write, connection, database, cx);
                    },
                );
            }
            TreeNodeId::Collection { connection, database, collection } => {
                let message =
                    format!("Drop collection \"{database}.{collection}\"? This cannot be undone.");
                let state = self.state.clone();
                let state_for_write = state.clone();
                request_connection_write(
                    state,
                    crate::components::WriteRequest::new(
                        connection,
                        format!("{database}.{collection}"),
                        "Drop a collection",
                        Some(WriteConfirmation {
                            title: "Drop collection".into(),
                            message,
                            confirm_label: "Drop".into(),
                            destructive: true,
                        }),
                    ),
                    window,
                    cx,
                    move |_window, cx| {
                        AppCommands::drop_collection(
                            state_for_write,
                            connection,
                            database,
                            collection,
                            cx,
                        );
                    },
                );
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
        window.focus(&self.focus_handle, cx);
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
            self.scroll_to_row(ix, ScrollStrategy::Center);
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

    /// Rows that fit in the list right now, less one so a page keeps a row of context.
    fn page_size(&self, cx: &App) -> isize {
        let viewport = self.scroll_handle.0.borrow().base_handle.bounds().size.height;
        ((viewport / Self::row_height(cx)) as isize - 1).max(1)
    }

    fn move_sidebar_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.move_sidebar_selection(delta * self.page_size(cx), cx);
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
        // The user picked a row themselves, so a reveal still waiting on a load stands down.
        self.pending_reveal = None;
        // Nearest moves the list only as far as it must; centering made every step past the
        // edge jump half a screen.
        self.scroll_to_row(next, ScrollStrategy::Nearest);
        cx.notify();
        if preview {
            self.schedule_keyboard_preview(node_id, cx);
        }
    }

    fn move_search_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let query = self.search_state.read(cx).value().to_string();
        let results = self.search_results(&query, cx);
        if let Some(ix) = self.model.move_search_selection(delta, results.len()) {
            self.search_scroll_handle.scroll_to_item(ix);
        }
        cx.notify();
    }

    fn select_search_result(
        &mut self,
        result: &SidebarSearchResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_keyboard_preview();
        self.reveal_node(result.node_id.clone(), cx);
        if !result.node_id.is_collection() {
            // Picking a connection or a database opens it, so its contents are in reach.
            self.set_node_expanded(&result.node_id, true, false, cx);
        }

        let opened_collection = result.collection.is_some();
        match (result.database.clone(), result.collection.clone()) {
            (Some(database), Some(collection)) => {
                self.state.update(cx, |state, cx| {
                    state.select_connection(Some(result.connection_id), cx);
                    state.select_collection(database, collection, cx);
                });
            }
            (Some(database), None) => {
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

        let mut results = search_results(query, candidates);
        results.truncate(SEARCH_RESULTS_LIMIT);
        self.search_cache = Some(SidebarSearchCache {
            query: query.to_string(),
            connections: self.cached_connections.clone(),
            active: self.cached_active.clone(),
            results: results.clone(),
        });
        results
    }
}
