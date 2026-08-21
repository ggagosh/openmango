use gpui::*;
use uuid::Uuid;

use crate::components::action_bar::ActionExecution;
use crate::components::{
    ConnectionManager, ContentArea, QueryLibraryDialog, request_disconnect_connection,
    request_unsaved_action,
};
use crate::keyboard::{
    CloseTab, DiscardDocumentChanges, FocusContent, FocusSidebar, OpenForge, RefreshView,
    SaveDocument, format_keystroke,
};
use crate::state::settings::AppTheme;
use crate::state::{
    ActiveTab, AppCommands, AppState, CollectionSubview, TransferMode, TransferScope, UnsavedScope,
    View,
};
use crate::views::CollectionView;

use super::AppRoot;
use super::dialogs::{open_create_collection_dialog, open_create_database_dialog};

impl AppRoot {
    pub(super) fn install_global_shortcuts(cx: &mut Context<Self>) -> Subscription {
        let weak_view = cx.entity().downgrade();
        cx.intercept_keystrokes(move |event, _window, cx| {
            let Some(view) = weak_view.upgrade() else {
                return;
            };
            view.update(cx, |this, cx| {
                if this.key_debug {
                    this.last_keystroke = Some(format_keystroke(event));
                    cx.notify();
                }

                let recording = this.state.read(cx).keybinding_capture().is_some();
                if recording && this.state.read(cx).current_view != View::Settings {
                    this.state.update(cx, |state, cx| {
                        state.cancel_keybinding_capture();
                        cx.notify();
                    });
                    return;
                }
                if recording {
                    let cancel = event.keystroke.key.eq_ignore_ascii_case("escape");
                    let shortcut = (!cancel).then(|| format_keystroke(event));
                    this.state.update(cx, |state, cx| {
                        if let Some(shortcut) = shortcut {
                            state.capture_keybinding(shortcut);
                        } else {
                            state.cancel_keybinding_capture();
                        }
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            });
        })
    }

    pub(super) fn handle_new_connection(&mut self, window: &mut Window, cx: &mut App) {
        ConnectionManager::open_new(self.state.clone(), window, cx);
    }

    pub(super) fn handle_create_database(&mut self, window: &mut Window, cx: &mut App) {
        let state_ref = self.state.read(cx);
        let Some(conn_id) = state_ref.selected_connection_id() else {
            return;
        };
        if !state_ref.is_connected(conn_id) {
            return;
        }
        open_create_database_dialog(self.state.clone(), window, cx);
    }

    pub(super) fn handle_create_collection(&mut self, window: &mut Window, cx: &mut App) {
        let state_ref = self.state.read(cx);
        let Some(conn_id) = state_ref.selected_connection_id() else {
            return;
        };
        if !state_ref.is_connected(conn_id) {
            return;
        }
        let database = state_ref.selected_database_name();
        let Some(database) = database else {
            return;
        };
        open_create_collection_dialog(self.state.clone(), database, window, cx);
    }

    pub(super) fn handle_create_index(&mut self, window: &mut Window, cx: &mut App) {
        if !matches!(self.state.read(cx).current_view, View::Documents) {
            return;
        }
        let Some(session_key) = self.state.read(cx).current_session_key() else {
            return;
        };
        let subview = self
            .state
            .read(cx)
            .session_subview(&session_key)
            .unwrap_or(CollectionSubview::Documents);
        if subview != CollectionSubview::Indexes {
            return;
        }
        CollectionView::open_index_create_dialog(self.state.clone(), session_key, window, cx);
    }

    pub(super) fn handle_close_tab(&mut self, window: &mut Window, cx: &mut App) {
        let target = {
            let state = self.state.read(cx);
            match state.active_tab() {
                ActiveTab::Preview => state
                    .preview_tab()
                    .cloned()
                    .map(|key| (UnsavedScope::Preview(key.clone()), None, Some(key))),
                ActiveTab::Index(index) => state
                    .open_tabs()
                    .get(index)
                    .cloned()
                    .map(|tab| (UnsavedScope::Tab(tab.clone()), Some(tab), None)),
                ActiveTab::None => None,
            }
        };
        let Some((scope, tab, preview)) = target else {
            return;
        };
        let state = self.state.clone();
        request_unsaved_action(self.state.clone(), scope, window, cx, move |_window, cx| {
            state.update(cx, |state, cx| {
                if let Some(tab) = &tab {
                    if let Some(index) =
                        state.open_tabs().iter().position(|candidate| candidate == tab)
                    {
                        state.close_tab(index, cx);
                    }
                } else if let Some(preview) = &preview
                    && state.preview_tab() == Some(preview)
                {
                    state.close_preview_tab(cx);
                }
            });
        });
    }

    pub(super) fn execute_action(
        state: &Entity<AppState>,
        content_area: &Entity<ContentArea>,
        exec: ActionExecution,
        window: &mut Window,
        cx: &mut App,
    ) {
        let id = exec.action_id.as_ref();

        // Navigation: connections
        if let Some(uuid_str) = id.strip_prefix("nav:conn:") {
            if let Ok(conn_id) = Uuid::parse_str(uuid_str) {
                state.update(cx, |state, cx| {
                    state.select_connection(Some(conn_id), cx);
                });
            }
            return;
        }

        // Navigation: databases (format: "nav:db:<uuid>:<database>")
        if let Some(rest) = id.strip_prefix("nav:db:") {
            if let Some((uuid_str, database)) = rest.split_once(':')
                && let Ok(conn_id) = Uuid::parse_str(uuid_str)
            {
                state.update(cx, |state, cx| {
                    state.select_connection(Some(conn_id), cx);
                    state.select_database(database.to_string(), cx);
                });
            }
            return;
        }

        // Navigation: collections (format: "nav:col:<uuid>:<database>:<collection>")
        if let Some(rest) = id.strip_prefix("nav:col:") {
            // Parse: uuid:db:col (uuid is always 36 chars)
            if rest.len() > 37 {
                let uuid_str = &rest[..36];
                let remainder = &rest[37..]; // skip the ':'
                if let Ok(conn_id) = Uuid::parse_str(uuid_str)
                    && let Some((database, collection)) = remainder.split_once(':')
                {
                    state.update(cx, |state, cx| {
                        state.select_connection(Some(conn_id), cx);
                        state.select_collection(database.to_string(), collection.to_string(), cx);
                    });
                }
            }
            return;
        }

        // Connect actions
        if let Some(conn_str) = id.strip_prefix("connect:") {
            if let Ok(conn_id) = Uuid::parse_str(conn_str) {
                AppCommands::connect(state.clone(), conn_id, cx);
            }
            return;
        }

        // Disconnect actions
        if let Some(conn_str) = id.strip_prefix("disconnect:") {
            if let Ok(conn_id) = Uuid::parse_str(conn_str) {
                request_disconnect_connection(state.clone(), conn_id, window, cx);
            }
            return;
        }

        // Theme actions
        if let Some(theme_id) = id.strip_prefix("theme:") {
            if let Some(theme) = AppTheme::from_theme_id(theme_id) {
                state.update(cx, |state, cx| {
                    state.settings.appearance.theme = theme;
                    state.save_settings();
                    cx.notify();
                });
                let (user_vibrancy, startup_vibrancy) = {
                    let state_ref = state.read(cx);
                    (state_ref.settings.appearance.vibrancy, state_ref.startup_vibrancy)
                };
                let target_vibrancy = crate::theme::effective_vibrancy(theme, user_vibrancy);
                crate::theme::apply_theme(theme, target_vibrancy, window, cx);
                if crate::theme::requires_vibrancy_restart(startup_vibrancy, theme, user_vibrancy) {
                    crate::components::open_confirm_dialog(
                        window,
                        cx,
                        "Restart required",
                        "Switching this theme changes window vibrancy mode. Restart now to fully apply it.",
                        "Restart now",
                        false,
                        {
                            let state = state.clone();
                            move |window, cx| {
                                crate::components::request_app_quit(state.clone(), window, cx);
                            }
                        },
                    );
                }
            }
            return;
        }

        // Tab actions
        if let Some(tab_str) = id.strip_prefix("tab:") {
            if tab_str == "preview" {
                state.update(cx, |state, cx| {
                    state.select_preview_tab(cx);
                });
            } else if let Ok(index) = tab_str.parse::<usize>() {
                state.update(cx, |state, cx| {
                    state.select_tab(index, cx);
                });
            }
            content_area.update(cx, |content, cx| {
                content.focus_current_view(window, cx);
            });
            return;
        }

        // Commands and views
        match id {
            "cmd:new-connection" => {
                ConnectionManager::open_new(state.clone(), window, cx);
            }
            "cmd:create-database" => {
                let state_ref = state.read(cx);
                let Some(conn_id) = state_ref.selected_connection_id() else {
                    return;
                };
                if !state_ref.is_connected(conn_id) {
                    return;
                }
                open_create_database_dialog(state.clone(), window, cx);
            }
            "cmd:create-collection" => {
                let state_ref = state.read(cx);
                let Some(conn_id) = state_ref.selected_connection_id() else {
                    return;
                };
                if !state_ref.is_connected(conn_id) {
                    return;
                }
                let Some(database) = state_ref.selected_database_name() else {
                    return;
                };
                open_create_collection_dialog(state.clone(), database, window, cx);
            }
            "cmd:insert-document" => {
                let Some(session_key) = state.read(cx).current_session_key() else {
                    return;
                };
                state.update(cx, |state, cx| {
                    state.set_collection_subview(&session_key, CollectionSubview::Documents);
                    cx.notify();
                });
                CollectionView::open_insert_document_json_editor(
                    state.clone(),
                    session_key,
                    window,
                    cx,
                );
            }
            "cmd:create-index" => {
                let Some(session_key) = state.read(cx).current_session_key() else {
                    return;
                };
                state.update(cx, |state, cx| {
                    state.set_collection_subview(&session_key, CollectionSubview::Indexes);
                    cx.notify();
                });
                CollectionView::open_index_create_dialog(state.clone(), session_key, window, cx);
            }
            "cmd:run-aggregation" => {
                let Some(session_key) = state.read(cx).current_session_key() else {
                    return;
                };
                state.update(cx, |state, cx| {
                    state.set_collection_subview(&session_key, CollectionSubview::Aggregation);
                    cx.notify();
                });
                crate::views::documents::request_run_aggregation(
                    state.clone(),
                    session_key,
                    false,
                    window,
                    cx,
                );
            }
            "cmd:open-forge" => {
                window.dispatch_action(Box::new(OpenForge), cx);
            }
            "cmd:transfer-export" => {
                if Self::open_transfer_from_current(state, TransferMode::Export, cx) {
                    content_area.update(cx, |content, cx| {
                        content.focus_current_view(window, cx);
                    });
                }
            }
            "cmd:transfer-import" => {
                if Self::open_transfer_from_current(state, TransferMode::Import, cx) {
                    content_area.update(cx, |content, cx| {
                        content.focus_current_view(window, cx);
                    });
                }
            }
            "cmd:transfer-copy" => {
                if Self::open_transfer_from_current(state, TransferMode::Copy, cx) {
                    content_area.update(cx, |content, cx| {
                        content.focus_current_view(window, cx);
                    });
                }
            }
            "cmd:save-document" => {
                Self::dispatch_content_action(content_area, Box::new(SaveDocument), window, cx);
            }
            "cmd:discard-document" => {
                Self::dispatch_content_action(
                    content_area,
                    Box::new(DiscardDocumentChanges),
                    window,
                    cx,
                );
            }
            "cmd:close-tab" => {
                window.dispatch_action(Box::new(CloseTab), cx);
            }
            "cmd:focus-sidebar" => {
                window.dispatch_action(Box::new(FocusSidebar), cx);
            }
            "cmd:focus-content" => {
                window.dispatch_action(Box::new(FocusContent), cx);
            }
            "cmd:refresh" => {
                window.dispatch_action(Box::new(RefreshView), cx);
            }
            "cmd:disconnect" => {
                // Handled as two-step in ActionBar (switches to Disconnect mode)
            }
            "cmd:query-library" => {
                QueryLibraryDialog::open_for_current(state.clone(), window, cx);
            }
            "cmd:settings" => {
                state.update(cx, |state, cx| {
                    state.open_settings_tab(cx);
                });
            }
            "cmd:ai" => {
                state.update(cx, |state, cx| {
                    state.toggle_ai_panel(cx);
                });
            }
            "cmd:whats-new" => {
                crate::changelog::open_changelog_tab(state.clone(), cx);
            }
            "view:documents" => {
                Self::show_collection_subview(state, CollectionSubview::Documents, cx);
            }
            "view:indexes" => {
                Self::show_collection_subview(state, CollectionSubview::Indexes, cx);
            }
            "view:stats" => {
                Self::show_collection_subview(state, CollectionSubview::Stats, cx);
            }
            "view:aggregation" => {
                Self::show_collection_subview(state, CollectionSubview::Aggregation, cx);
            }
            "view:schema" => {
                Self::show_collection_subview(state, CollectionSubview::Schema, cx);
            }
            "view:history" => {
                Self::show_collection_subview(state, CollectionSubview::History, cx);
            }
            "cmd:check-updates" => {
                AppCommands::check_for_updates(state.clone(), cx);
            }
            "cmd:download-update" => {
                AppCommands::download_update(state.clone(), cx);
            }
            "cmd:install-update" => {
                AppCommands::install_update(state.clone(), cx);
            }
            _ => {} // Unknown action — no-op
        }
    }

    fn dispatch_content_action(
        content_area: &Entity<ContentArea>,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if content_area.update(cx, |content, cx| content.focus_current_view(window, cx)) {
            window.dispatch_action(action, cx);
        }
    }

    fn open_transfer_from_current(
        state: &Entity<AppState>,
        mode: TransferMode,
        cx: &mut App,
    ) -> bool {
        let Some((key, collection)) = ({
            let state = state.read(cx);
            state.current_database_key().map(|key| (key, state.selected_collection_name()))
        }) else {
            return false;
        };
        let scope =
            if collection.is_some() { TransferScope::Collection } else { TransferScope::Database };
        state.update(cx, |state, cx| {
            state.open_transfer_tab_with_prefill(
                key.connection_id,
                key.database,
                collection,
                scope,
                mode,
                cx,
            );
        });
        true
    }

    fn show_collection_subview(state: &Entity<AppState>, subview: CollectionSubview, cx: &mut App) {
        let Some(key) = state.read(cx).current_session_key() else {
            return;
        };
        let should_load = state.update(cx, |state, cx| {
            let should_load = state.set_collection_subview(&key, subview);
            cx.notify();
            should_load
        });
        match subview {
            CollectionSubview::Indexes => {
                AppCommands::load_collection_indexes(state.clone(), key, false, cx);
            }
            CollectionSubview::Stats if should_load => {
                AppCommands::load_collection_stats(state.clone(), key, cx);
            }
            CollectionSubview::Schema if should_load => {
                AppCommands::analyze_collection_schema(state.clone(), key, cx);
            }
            _ => {}
        }
    }

    pub(super) fn focus_current_content(&mut self, window: &mut Window, cx: &mut App) {
        if !self.content_area.update(cx, |content, cx| content.focus_current_view(window, cx)) {
            window.focus(&self.focus_handle);
        }
    }

    pub(super) fn handle_refresh(&mut self, window: &mut Window, cx: &mut App) {
        let reloaded_sidebar_database = self
            .sidebar
            .update(cx, |sidebar, cx| sidebar.reload_selected_database_if_focused(window, cx));
        if reloaded_sidebar_database {
            return;
        }

        let (current_view, session_key, database_key, subview) = {
            let state_ref = self.state.read(cx);
            let session_key = state_ref.current_session_key();
            let subview = session_key
                .as_ref()
                .and_then(|key| state_ref.session_subview(key))
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
                    CollectionSubview::Aggregation => {
                        AppCommands::run_aggregation(self.state.clone(), session_key, false, cx);
                    }
                    CollectionSubview::Schema => {
                        AppCommands::analyze_collection_schema(self.state.clone(), session_key, cx);
                    }
                    CollectionSubview::History => {
                        AppCommands::load_collection_history(self.state.clone(), session_key, cx);
                    }
                }
            }
            View::Database => {
                let Some(database_key) = database_key else {
                    return;
                };
                AppCommands::reload_database(self.state.clone(), database_key, cx);
            }
            View::Transfer
            | View::Forge
            | View::AgentActivity
            | View::Connections
            | View::Settings
            | View::Changelog => {}
            View::Databases | View::Collections | View::Welcome => {
                let state_ref = self.state.read(cx);
                if let Some(conn_id) = state_ref.selected_connection_id()
                    && state_ref.is_connected(conn_id)
                {
                    AppCommands::refresh_databases(self.state.clone(), conn_id, cx);
                }
            }
        }
    }
}
