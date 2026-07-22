use gpui::*;

use crate::state::{AppEvent, AppState, StatusLevel, View};
use crate::views::{
    AiView, ChangelogView, CollectionView, DatabaseView, ForgeView, SettingsView, TransferView,
};

mod empty;
mod shell;
mod tabs;

use empty::render_empty_state;
use shell::render_shell;
use tabs::{OpenTabsBar, TabsHost, render_tabs_host};

/// Content area component that shows collection view or welcome screen
pub struct ContentArea {
    state: Entity<AppState>,
    tabs_bar: Entity<OpenTabsBar>,
    collection_view: Option<Entity<CollectionView>>,
    database_view: Option<Entity<DatabaseView>>,
    ai_view: Option<Entity<AiView>>,
    transfer_view: Option<Entity<TransferView>>,
    forge_view: Option<Entity<ForgeView>>,
    settings_view: Option<Entity<SettingsView>>,
    changelog_view: Option<Entity<ChangelogView>>,
    last_inputs: ContentAreaInputs,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, PartialEq, Eq)]
struct ContentAreaInputs {
    has_collection: bool,
    has_connection: bool,
    selected_db: Option<String>,
    has_tabs: bool,
    current_view: View,
    error_text: Option<String>,
}

impl ContentAreaInputs {
    fn from_state(state: &AppState) -> Self {
        Self {
            has_collection: state.selected_collection().is_some(),
            has_connection: state.has_active_connections(),
            selected_db: state.selected_database_name(),
            has_tabs: !state.open_tabs().is_empty() || state.preview_tab().is_some(),
            current_view: state.current_view,
            error_text: state.status_message().and_then(|message| {
                if matches!(message.level, StatusLevel::Error) { Some(message.text) } else { None }
            }),
        }
    }
}

impl ContentArea {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![];
        let last_inputs = ContentAreaInputs::from_state(state.read(cx));

        subscriptions.push(cx.observe(&state, |this, state, cx| {
            let next_inputs = ContentAreaInputs::from_state(state.read(cx));
            if this.last_inputs != next_inputs {
                this.last_inputs = next_inputs;
                cx.notify();
            }
        }));

        // Subscribe to view-change events to lazily create collection view
        subscriptions.push(cx.subscribe(&state, |this, state, event, cx| match event {
            AppEvent::ViewChanged | AppEvent::Connected(_) => {
                let (
                    should_create_collection,
                    should_create_database,
                    should_create_ai,
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
                        false,
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
                if should_create_ai && this.ai_view.is_none() {
                    this.ai_view = Some(cx.new(|cx| AiView::new(state.clone(), cx)));
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

                this.last_inputs = ContentAreaInputs::from_state(state.read(cx));
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
        let ai_view = None;
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
        let tabs_bar = cx.new(|cx| OpenTabsBar::new(state.clone(), cx));

        Self {
            state,
            tabs_bar,
            collection_view,
            database_view,
            ai_view,
            transfer_view,
            forge_view,
            settings_view,
            changelog_view,
            last_inputs,
            _subscriptions: subscriptions,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_views(
        &mut self,
        should_collection: bool,
        should_database: bool,
        should_ai: bool,
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
        if should_ai && self.ai_view.is_none() {
            self.ai_view = Some(cx.new(|cx| AiView::new(self.state.clone(), cx)));
        } else if !should_ai {
            self.ai_view = None;
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

    pub(crate) fn focus_current_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let should_focus_collection = {
            let state_ref = self.state.read(cx);
            matches!(state_ref.current_view, View::Documents)
                && state_ref.selected_collection().is_some()
        };

        if should_focus_collection {
            if self.collection_view.is_none() {
                self.collection_view =
                    Some(cx.new(|cx| CollectionView::new(self.state.clone(), cx)));
            }
            if let Some(view) = self.collection_view.clone() {
                view.update(cx, |view, _cx| view.focus_documents(window));
                return true;
            }
        }

        if matches!(self.state.read(cx).current_view, View::Transfer) {
            if self.transfer_view.is_none() {
                self.transfer_view = Some(cx.new(|cx| TransferView::new(self.state.clone(), cx)));
            }
            if let Some(view) = self.transfer_view.clone() {
                view.update(cx, |view, cx| view.focus(window, cx));
                return true;
            }
        }

        if matches!(self.state.read(cx).current_view, View::Forge) {
            if self.forge_view.is_none() {
                self.forge_view = Some(cx.new(|cx| ForgeView::new(self.state.clone(), cx)));
            }
            if let Some(view) = self.forge_view.clone() {
                view.update(cx, |view, cx| view.focus(window, cx));
                return true;
            }
        }

        false
    }
}

impl Render for ContentArea {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let inputs = {
            let state_ref = self.state.read(cx);
            ContentAreaInputs::from_state(state_ref)
        };
        self.last_inputs = inputs.clone();
        let ContentAreaInputs {
            has_collection,
            has_connection,
            selected_db,
            has_tabs,
            current_view,
            error_text,
        } = inputs;

        let should_collection_view = matches!(current_view, View::Documents);
        let should_database_view = matches!(current_view, View::Database) && selected_db.is_some();
        let should_ai_view = false;
        let should_transfer_view = matches!(current_view, View::Transfer);
        let should_forge_view = matches!(current_view, View::Forge);
        let should_settings_view = matches!(current_view, View::Settings);
        let should_changelog_view = matches!(current_view, View::Changelog);

        if has_tabs {
            self.ensure_views(
                should_collection_view,
                should_database_view,
                should_ai_view,
                should_transfer_view,
                should_forge_view,
                should_settings_view,
                should_changelog_view,
                cx,
            );
            let host = TabsHost {
                state: self.state.clone(),
                tabs_bar: self.tabs_bar.clone(),
                current_view,
                has_collection,
                collection_view: self.collection_view.as_ref(),
                database_view: self.database_view.as_ref(),
                transfer_view: self.transfer_view.as_ref(),
                forge_view: self.forge_view.as_ref(),
                settings_view: self.settings_view.as_ref(),
                changelog_view: self.changelog_view.as_ref(),
            };
            let content = render_tabs_host(host, cx);
            return render_shell(error_text, self.state.clone(), content, false, cx);
        }

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
