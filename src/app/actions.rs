use gpui::*;
use uuid::Uuid;

use crate::components::ConnectionDialog;
use crate::components::action_bar::ActionExecution;
use crate::state::settings::AppTheme;
use crate::state::{ActiveTab, AppCommands, AppState, CollectionSubview, View};
use crate::views::CollectionView;

use super::AppRoot;
use super::dialogs::{open_create_collection_dialog, open_create_database_dialog};

impl AppRoot {
    pub(super) fn install_global_shortcuts(cx: &mut Context<Self>) -> Subscription {
        let weak_view = cx.entity().downgrade();
        cx.intercept_keystrokes(move |event, window, cx| {
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

                if cmd_or_ctrl && !alt && !shift && key == "k" {
                    this.action_bar.update(
                        cx,
                        |bar: &mut crate::components::action_bar::ActionBar, cx| {
                            bar.toggle(window, cx);
                        },
                    );
                    cx.stop_propagation();
                    return;
                }

                if cmd_or_ctrl && !alt && !shift && key == "," {
                    this.state.update(cx, |state, cx| {
                        state.open_settings_tab(cx);
                    });
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
                                .and_then(|key| this.state.read(cx).session_subview(&key))
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
                                    CollectionSubview::Stats | CollectionSubview::Aggregation => {}
                                }
                            }
                        }
                        View::Database => {
                            let database = this.state.read(cx).selected_database_name();
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
                        View::Transfer | View::Forge | View::Settings | View::Changelog => {}
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
                    return;
                }

                // Cmd+1-9: switch to tab by index
                if cmd_or_ctrl
                    && !alt
                    && !shift
                    && key.len() == 1
                    && let Some(digit) = key.chars().next().and_then(|c| c.to_digit(10))
                    && (1..=9).contains(&digit)
                {
                    let tab_index = (digit - 1) as usize;
                    this.state.update(cx, |state, cx| {
                        state.select_tab(tab_index, cx);
                    });
                    cx.stop_propagation();
                }
            });
        })
    }

    pub(super) fn handle_new_connection(&mut self, window: &mut Window, cx: &mut App) {
        ConnectionDialog::open(self.state.clone(), window, cx);
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

    pub(super) fn handle_close_tab(&mut self, cx: &mut App) {
        self.state.update(cx, |state, cx| match state.active_tab() {
            ActiveTab::Preview => state.close_preview_tab(cx),
            ActiveTab::Index(index) => state.close_tab(index, cx),
            ActiveTab::None => {}
        });
    }

    pub(super) fn execute_action(
        state: &Entity<AppState>,
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
                AppCommands::disconnect(state.clone(), conn_id, cx);
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
                let vibrancy = state.read(cx).startup_vibrancy;
                crate::theme::apply_theme(theme, vibrancy, window, cx);
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
            return;
        }

        // Commands and views
        match id {
            "cmd:new-connection" => {
                ConnectionDialog::open(state.clone(), window, cx);
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
            "cmd:refresh" => {
                let state_ref = state.read(cx);
                if let Some(conn_id) = state_ref.selected_connection_id()
                    && state_ref.is_connected(conn_id)
                {
                    AppCommands::refresh_databases(state.clone(), conn_id, cx);
                }
            }
            "cmd:disconnect" => {
                // Handled as two-step in ActionBar (switches to Disconnect mode)
            }
            "cmd:settings" => {
                state.update(cx, |state, cx| {
                    state.open_settings_tab(cx);
                });
            }
            "cmd:whats-new" => {
                crate::changelog::open_changelog_tab(state.clone(), cx);
            }
            "view:documents" => {
                if let Some(key) = state.read(cx).current_session_key() {
                    state.update(cx, |state, _cx| {
                        state.set_collection_subview(&key, CollectionSubview::Documents);
                    });
                }
            }
            "view:indexes" => {
                if let Some(key) = state.read(cx).current_session_key() {
                    state.update(cx, |state, _cx| {
                        state.set_collection_subview(&key, CollectionSubview::Indexes);
                    });
                }
            }
            "view:stats" => {
                if let Some(key) = state.read(cx).current_session_key() {
                    state.update(cx, |state, _cx| {
                        state.set_collection_subview(&key, CollectionSubview::Stats);
                    });
                }
            }
            "view:aggregation" => {
                if let Some(key) = state.read(cx).current_session_key() {
                    state.update(cx, |state, _cx| {
                        state.set_collection_subview(&key, CollectionSubview::Aggregation);
                    });
                }
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

    pub(super) fn handle_refresh(&mut self, cx: &mut App) {
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
                }
            }
            View::Database => {
                let Some(database_key) = database_key else {
                    return;
                };
                AppCommands::load_database_overview(self.state.clone(), database_key, true, cx);
            }
            View::Transfer | View::Forge | View::Settings | View::Changelog => {}
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
