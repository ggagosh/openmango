use gpui::*;

use crate::components::ConnectionDialog;
use crate::state::{ActiveTab, AppCommands, CollectionSubview, View};
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
                                    CollectionSubview::Stats => {}
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
