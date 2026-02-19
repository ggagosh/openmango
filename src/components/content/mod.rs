use gpui::*;

use crate::state::{ActiveTab, AppEvent, AppState, StatusLevel, View};
use crate::views::{
    ChangelogView, CollectionView, DatabaseView, ForgeView, JsonEditorView, SettingsView,
    TransferView,
};

mod empty;
mod shell;
mod tabs;

use empty::render_empty_state;
use shell::render_shell;
use tabs::{TabsHost, render_tabs_host};

/// Content area component that shows collection view or welcome screen
pub struct ContentArea {
    state: Entity<AppState>,
    tabs_scroll_handle: ScrollHandle,
    last_seen_open_tab_count: usize,
    last_seen_active_tab: ActiveTab,
    pending_scroll_to_end_frames: u8,
    collection_view: Option<Entity<CollectionView>>,
    database_view: Option<Entity<DatabaseView>>,
    json_editor_view: Option<Entity<JsonEditorView>>,
    transfer_view: Option<Entity<TransferView>>,
    forge_view: Option<Entity<ForgeView>>,
    settings_view: Option<Entity<SettingsView>>,
    changelog_view: Option<Entity<ChangelogView>>,
    _subscriptions: Vec<Subscription>,
}

impl ContentArea {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![];

        subscriptions.push(cx.observe(&state, |_, _, cx| cx.notify()));

        // Subscribe to view-change events to lazily create collection view
        subscriptions.push(cx.subscribe(&state, |this, state, event, cx| match event {
            AppEvent::ViewChanged | AppEvent::Connected(_) => {
                let (
                    should_create_collection,
                    should_create_database,
                    should_create_json_editor,
                    should_create_transfer,
                    should_create_forge,
                    should_create_settings,
                    should_create_changelog,
                ) = {
                    let state_ref = state.read(cx);
                    (
                        state_ref.selected_collection().is_some(),
                        matches!(state_ref.current_view, View::Database)
                            && state_ref.selected_database().is_some(),
                        matches!(state_ref.current_view, View::JsonEditor),
                        matches!(state_ref.current_view, View::Transfer),
                        matches!(state_ref.current_view, View::Forge),
                        matches!(state_ref.current_view, View::Settings),
                        matches!(state_ref.current_view, View::Changelog),
                    )
                };

                if should_create_collection && this.collection_view.is_none() {
                    this.collection_view =
                        Some(cx.new(|cx| CollectionView::new(state.clone(), cx)));
                }
                if should_create_database && this.database_view.is_none() {
                    this.database_view = Some(cx.new(|cx| DatabaseView::new(state.clone(), cx)));
                }
                if should_create_json_editor && this.json_editor_view.is_none() {
                    this.json_editor_view =
                        Some(cx.new(|cx| JsonEditorView::new(state.clone(), cx)));
                }
                if should_create_transfer && this.transfer_view.is_none() {
                    this.transfer_view = Some(cx.new(|cx| TransferView::new(state.clone(), cx)));
                }
                if should_create_forge && this.forge_view.is_none() {
                    this.forge_view = Some(cx.new(|cx| ForgeView::new(state.clone(), cx)));
                }
                if should_create_settings && this.settings_view.is_none() {
                    this.settings_view = Some(cx.new(|cx| SettingsView::new(state.clone(), cx)));
                }
                if should_create_changelog && this.changelog_view.is_none() {
                    this.changelog_view = Some(cx.new(|cx| ChangelogView::new(state.clone(), cx)));
                }

                cx.notify();
            }
            _ => {}
        }));

        // Check if we should create collection view initially
        let collection_view = if state.read(cx).selected_collection().is_some() {
            Some(cx.new(|cx| CollectionView::new(state.clone(), cx)))
        } else {
            None
        };
        let database_view = if matches!(state.read(cx).current_view, View::Database)
            && state.read(cx).selected_database().is_some()
        {
            Some(cx.new(|cx| DatabaseView::new(state.clone(), cx)))
        } else {
            None
        };
        let json_editor_view = if matches!(state.read(cx).current_view, View::JsonEditor) {
            Some(cx.new(|cx| JsonEditorView::new(state.clone(), cx)))
        } else {
            None
        };
        let transfer_view = if matches!(state.read(cx).current_view, View::Transfer) {
            Some(cx.new(|cx| TransferView::new(state.clone(), cx)))
        } else {
            None
        };
        let forge_view = if matches!(state.read(cx).current_view, View::Forge) {
            Some(cx.new(|cx| ForgeView::new(state.clone(), cx)))
        } else {
            None
        };
        let settings_view = if matches!(state.read(cx).current_view, View::Settings) {
            Some(cx.new(|cx| SettingsView::new(state.clone(), cx)))
        } else {
            None
        };
        let changelog_view = if matches!(state.read(cx).current_view, View::Changelog) {
            Some(cx.new(|cx| ChangelogView::new(state.clone(), cx)))
        } else {
            None
        };
        let state_ref = state.read(cx);
        let last_seen_open_tab_count = state_ref.open_tabs().len();
        let last_seen_active_tab = state_ref.active_tab();

        Self {
            state,
            tabs_scroll_handle: ScrollHandle::new(),
            last_seen_open_tab_count,
            last_seen_active_tab,
            pending_scroll_to_end_frames: 0,
            collection_view,
            database_view,
            json_editor_view,
            transfer_view,
            forge_view,
            settings_view,
            changelog_view,
            _subscriptions: subscriptions,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_views(
        &mut self,
        should_collection: bool,
        should_database: bool,
        should_json_editor: bool,
        should_transfer: bool,
        should_forge: bool,
        should_settings: bool,
        should_changelog: bool,
        cx: &mut Context<Self>,
    ) {
        if should_collection && self.collection_view.is_none() {
            self.collection_view = Some(cx.new(|cx| CollectionView::new(self.state.clone(), cx)));
        }
        if should_database && self.database_view.is_none() {
            self.database_view = Some(cx.new(|cx| DatabaseView::new(self.state.clone(), cx)));
        }
        if should_json_editor && self.json_editor_view.is_none() {
            self.json_editor_view = Some(cx.new(|cx| JsonEditorView::new(self.state.clone(), cx)));
        }
        if should_transfer && self.transfer_view.is_none() {
            self.transfer_view = Some(cx.new(|cx| TransferView::new(self.state.clone(), cx)));
        }
        if should_forge && self.forge_view.is_none() {
            self.forge_view = Some(cx.new(|cx| ForgeView::new(self.state.clone(), cx)));
        }
        if should_settings && self.settings_view.is_none() {
            self.settings_view = Some(cx.new(|cx| SettingsView::new(self.state.clone(), cx)));
        }
        if should_changelog && self.changelog_view.is_none() {
            self.changelog_view = Some(cx.new(|cx| ChangelogView::new(self.state.clone(), cx)));
        }
    }
}

impl Render for ContentArea {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (
            has_collection,
            has_connection,
            selected_db,
            tabs,
            active_tab,
            preview_tab,
            dirty_tabs,
            current_view,
            error_text,
        ) = {
            let state_ref = self.state.read(cx);
            (
                state_ref.selected_collection().is_some(),
                state_ref.has_active_connections(),
                state_ref.selected_database_name(),
                state_ref.open_tabs().to_vec(),
                state_ref.active_tab(),
                state_ref.preview_tab().cloned(),
                state_ref.dirty_tabs().clone(),
                state_ref.current_view,
                state_ref.status_message().and_then(|message| {
                    if matches!(message.level, StatusLevel::Error) {
                        Some(message.text.clone())
                    } else {
                        None
                    }
                }),
            )
        };

        let should_collection_view = matches!(current_view, View::Documents);
        let should_database_view = matches!(current_view, View::Database) && selected_db.is_some();
        let should_json_editor_view = matches!(current_view, View::JsonEditor);
        let should_transfer_view = matches!(current_view, View::Transfer);
        let should_forge_view = matches!(current_view, View::Forge);
        let should_settings_view = matches!(current_view, View::Settings);
        let should_changelog_view = matches!(current_view, View::Changelog);
        let tab_count = tabs.len();
        let active_changed = active_tab != self.last_seen_active_tab;
        let tab_count_increased = tab_count > self.last_seen_open_tab_count;
        if tab_count_increased || active_changed {
            // Multi-frame reveal is needed because content width can settle after the first render.
            self.pending_scroll_to_end_frames = self.pending_scroll_to_end_frames.max(4);
        }
        self.last_seen_open_tab_count = tab_count;
        self.last_seen_active_tab = active_tab;

        let has_tabs = !tabs.is_empty() || preview_tab.is_some();
        if has_tabs {
            self.ensure_views(
                should_collection_view,
                should_database_view,
                should_json_editor_view,
                should_transfer_view,
                should_forge_view,
                should_settings_view,
                should_changelog_view,
                cx,
            );
            let scroll_to_end_once = self.pending_scroll_to_end_frames > 0;
            if self.pending_scroll_to_end_frames > 0 {
                self.pending_scroll_to_end_frames -= 1;
                if self.pending_scroll_to_end_frames > 0 {
                    cx.notify();
                }
            }
            let host = TabsHost {
                state: self.state.clone(),
                tabs_scroll_handle: &self.tabs_scroll_handle,
                scroll_to_end_once,
                tabs: &tabs,
                active_tab,
                preview_tab,
                dirty_tabs: &dirty_tabs,
                current_view,
                has_collection,
                collection_view: self.collection_view.as_ref(),
                database_view: self.database_view.as_ref(),
                json_editor_view: self.json_editor_view.as_ref(),
                transfer_view: self.transfer_view.as_ref(),
                forge_view: self.forge_view.as_ref(),
                settings_view: self.settings_view.as_ref(),
                changelog_view: self.changelog_view.as_ref(),
            };
            let content = render_tabs_host(host, cx);
            return render_shell(error_text, self.state.clone(), content, false, cx);
        }
        self.pending_scroll_to_end_frames = 0;

        if matches!(current_view, View::Settings) {
            self.ensure_views(false, false, false, false, false, should_settings_view, false, cx);
            if let Some(view) = &self.settings_view {
                return render_shell(error_text, self.state.clone(), view.clone(), false, cx);
            }
        }

        if matches!(current_view, View::Changelog) {
            self.ensure_views(false, false, false, false, false, false, should_changelog_view, cx);
            if let Some(view) = &self.changelog_view {
                return render_shell(error_text, self.state.clone(), view.clone(), false, cx);
            }
        }

        if matches!(current_view, View::JsonEditor) {
            self.ensure_views(
                false,
                false,
                should_json_editor_view,
                false,
                false,
                false,
                false,
                cx,
            );
            if let Some(view) = &self.json_editor_view {
                return render_shell(error_text, self.state.clone(), view.clone(), false, cx);
            }
        }

        if matches!(current_view, View::Database) {
            self.ensure_views(false, should_database_view, false, false, false, false, false, cx);
            if let Some(view) = &self.database_view {
                return render_shell(error_text, self.state.clone(), view.clone(), false, cx);
            }
        }

        if matches!(current_view, View::Transfer) {
            self.ensure_views(false, false, false, should_transfer_view, false, false, false, cx);
            if let Some(view) = &self.transfer_view {
                return render_shell(error_text, self.state.clone(), view.clone(), false, cx);
            }
        }

        if matches!(current_view, View::Forge) {
            self.ensure_views(false, false, false, false, should_forge_view, false, false, cx);
            if let Some(view) = &self.forge_view {
                return render_shell(error_text, self.state.clone(), view.clone(), false, cx);
            }
        }

        if has_collection {
            self.ensure_views(should_collection_view, false, false, false, false, false, false, cx);
            if let Some(view) = &self.collection_view {
                return render_shell(error_text, self.state.clone(), view.clone(), false, cx);
            }
        }

        let hint = if !has_connection {
            "Add a connection to get started".to_string()
        } else if selected_db.is_none() {
            "Select a database in the sidebar".to_string()
        } else {
            "Select a collection to view documents".to_string()
        };

        let empty = render_empty_state(hint, cx);
        render_shell(error_text, self.state.clone(), empty, true, cx)
    }
}
