//! Transfer view for import, export, and copy operations.

mod helpers;
mod options;
mod progress_panel;
mod query_modal;
mod select_states;
mod simple;

pub use query_modal::QueryEditField;

use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::component::select::{SearchableVec, SelectState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _};
use gpui_kit::*;
use uuid::Uuid;

use crate::components::{WriteConfirmation, open_confirm_dialog, request_connection_write};
use crate::keyboard::{CancelTransfer, CloseTransferQueryModal, RunTransfer, SaveTransferQuery};
use crate::state::{
    AppCommands, AppState, InsertMode, StatusMessage, TargetWriteMode, TransferMode, TransferScope,
    TransferTabState, coerce_transfer_format, resolved_export_destination,
    transfer_write_connection, validate_transfer,
};
use crate::theme::{islands, sizing, spacing};

use select_states::ConnectionItem;

pub struct TransferView {
    state: Entity<AppState>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _select_subscriptions: Vec<Subscription>,
    options_expanded: bool,

    // Select states for searchable dropdowns (lazily initialized on first render)
    source_conn_state: Option<Entity<SelectState<SearchableVec<ConnectionItem>>>>,
    source_db_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    source_coll_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    dest_conn_state: Option<Entity<SelectState<SearchableVec<ConnectionItem>>>>,
    dest_db_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    dest_coll_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    dest_db_input_state: Option<Entity<InputState>>,
    dest_coll_input_state: Option<Entity<InputState>>,

    // Exclude collections multi-select state
    exclude_coll_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,

    // Input state for export path (lazily initialized on first render)
    export_path_input_state: Option<Entity<InputState>>,

    // Track previous items to avoid resetting search state on every render
    prev_connections: Vec<(Uuid, crate::components::ConnectionIdentity)>,
    prev_db_names: Vec<String>,
    prev_coll_names: Vec<String>,
    prev_dest_db_names: Vec<String>,
    prev_dest_coll_names: Vec<String>,

    // JSON editor modal state
    query_edit_modal: Option<QueryEditField>, // Which field is being edited (None = closed)
    query_edit_input: Option<Entity<EditorState>>, // Textarea content for modal
    query_edit_transfer_id: Option<Uuid>,
    query_edit_previous_focus: Option<FocusHandle>,
}

impl TransferView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];

        Self {
            state,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
            _select_subscriptions: Vec::new(),
            options_expanded: false,
            source_conn_state: None,
            source_db_state: None,
            source_coll_state: None,
            dest_conn_state: None,
            dest_db_state: None,
            dest_coll_state: None,
            dest_db_input_state: None,
            dest_coll_input_state: None,
            exclude_coll_state: None,
            export_path_input_state: None,
            prev_connections: Vec::new(),
            prev_db_names: Vec::new(),
            prev_coll_names: Vec::new(),
            prev_dest_db_names: Vec::new(),
            prev_dest_coll_names: Vec::new(),
            query_edit_modal: None,
            query_edit_input: None,
            query_edit_transfer_id: None,
            query_edit_previous_focus: None,
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        let active_transfer = self.state.read(cx).active_transfer_tab_id();
        if self.query_edit_transfer_id == active_transfer
            && let Some(input) = self.query_edit_input.as_ref()
        {
            window.focus(&input.read(cx).focus_handle(cx), cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }
}

fn destructive_transfer_message(transfer_state: &TransferTabState) -> String {
    let target_db = if transfer_state.config.destination_database.is_empty() {
        &transfer_state.config.source_database
    } else {
        &transfer_state.config.destination_database
    };
    let target_collection = if transfer_state.config.destination_collection.is_empty() {
        &transfer_state.config.source_collection
    } else {
        &transfer_state.config.destination_collection
    };
    let target = if matches!(transfer_state.config.scope, TransferScope::Collection) {
        format!("{target_db}.{target_collection}")
    } else {
        target_db.to_string()
    };

    let mut effects = Vec::new();
    if transfer_state.options.target_write_mode() != TargetWriteMode::Append {
        effects.push(format!(
            "{} will run before the transfer.",
            transfer_state.options.target_write_mode().label()
        ));
    }
    match transfer_state.options.insert_mode {
        InsertMode::Insert => {}
        InsertMode::Upsert => effects
            .push("Upsert may update existing documents with matching _id values.".to_string()),
        InsertMode::Replace => effects.push(
            "Replace may fully replace existing documents with matching _id values.".to_string(),
        ),
    }
    effects.push(format!("Target: {target}. This cannot be undone."));
    effects.join(" ")
}

fn run_active_transfer(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let Some((transfer_id, transfer_state)) = ({
        let state = state.read(cx);
        state
            .active_transfer_tab_id()
            .and_then(|id| state.transfer_tab(id).cloned().map(|transfer| (id, transfer)))
    }) else {
        return;
    };
    if transfer_state.runtime.is_running {
        return;
    }

    let validation = validate_transfer(&transfer_state);
    if !validation.can_run() {
        return;
    }
    let resolved_destination = resolved_export_destination(&transfer_state);
    let overwrite_destination =
        resolved_destination.clone().filter(|destination| destination.exists());
    let requires_confirmation = validation.requires_confirmation || overwrite_destination.is_some();
    let write_connection = transfer_write_connection(&transfer_state);
    let production_confirmation = write_connection.is_some_and(|connection_id| {
        state.read(cx).connection_requires_production_write_confirmation(connection_id)
    });
    if !requires_confirmation && !production_confirmation {
        AppCommands::execute_transfer(state, transfer_id, cx);
        return;
    }

    let message = overwrite_destination.map_or_else(
        || destructive_transfer_message(&transfer_state),
        |destination| {
            format!(
                "The export destination '{}' already exists and will be replaced only after the export completes successfully.",
                destination.display()
            )
        },
    );
    let expected_config = transfer_state.config.clone();
    let expected_options = transfer_state.options.clone();
    let target_database = if transfer_state.config.destination_database.is_empty() {
        transfer_state.config.source_database.clone()
    } else {
        transfer_state.config.destination_database.clone()
    };
    let target_collection = if transfer_state.config.destination_collection.is_empty() {
        transfer_state.config.source_collection.clone()
    } else {
        transfer_state.config.destination_collection.clone()
    };
    let target = if transfer_state.config.scope == TransferScope::Collection {
        format!("{target_database}.{target_collection}")
    } else {
        target_database
    };
    let state_for_run = state.clone();
    let run = move |_window: &mut Window, cx: &mut App| {
        let unchanged = state_for_run
            .read(cx)
            .transfer_tab(transfer_id)
            .is_some_and(|tab| tab.config == expected_config && tab.options == expected_options);
        if !unchanged {
            state_for_run.update(cx, |state, cx| {
                let message = "Transfer options changed after confirmation. Review and run again.";
                if let Some(tab) = state.transfer_tab_mut(transfer_id) {
                    tab.runtime.error_message = Some(message.to_string());
                }
                state.set_status_message(Some(StatusMessage::error(message)));
                cx.notify();
            });
            return;
        }
        AppCommands::execute_confirmed_transfer(
            state_for_run.clone(),
            transfer_id,
            resolved_destination.clone(),
            cx,
        );
    };
    if let Some(connection_id) = write_connection {
        let ordinary = requires_confirmation.then(|| WriteConfirmation {
            title: "Confirm destructive transfer".to_string(),
            message,
            confirm_label: "Run Transfer".to_string(),
            destructive: true,
        });
        request_connection_write(
            state.clone(),
            crate::components::WriteRequest::new(
                connection_id,
                target,
                "Run a data transfer that writes to MongoDB",
                ordinary,
            ),
            window,
            cx,
            run,
        );
    } else {
        open_confirm_dialog(
            window,
            cx,
            "Confirm destructive transfer",
            message,
            "Run Transfer",
            true,
            run,
        );
    }
}

fn cancel_active_transfer(state: Entity<AppState>, cx: &mut App) {
    let Some(transfer_id) = state.read(cx).active_transfer_tab_id() else {
        return;
    };
    if state.read(cx).transfer_tab(transfer_id).is_some_and(|tab| tab.runtime.is_running) {
        AppCommands::cancel_transfer(state, transfer_id, cx);
    }
}

impl Render for TransferView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Ensure select states are initialized
        self.ensure_select_states(window, cx);

        // Read state once at the start
        let (
            transfer_id,
            transfer_state,
            connections,
            databases,
            collections,
            dest_databases,
            dest_collections,
        ) = {
            let state_ref = self.state.read(cx);
            let Some(id) = state_ref.active_transfer_tab_id() else {
                return div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Open a Transfer tab to configure import/export")
                    .into_any_element();
            };
            let transfer = state_ref.transfer_tab(id).cloned().unwrap_or_default();

            let active = state_ref.active_connections_snapshot();
            let connections: Vec<(Uuid, crate::components::ConnectionIdentity)> = active
                .keys()
                .filter_map(|id| {
                    state_ref.connection_by_id(*id).map(|connection| {
                        (*id, crate::components::ConnectionIdentity::from(connection))
                    })
                })
                .collect();

            let databases: Vec<String> = transfer
                .config
                .source_connection_id
                .and_then(|conn_id| active.get(&conn_id).map(|conn| conn.databases.clone()))
                .unwrap_or_default();

            let collections: Vec<String> = transfer
                .config
                .source_connection_id
                .and_then(|conn_id| {
                    if !transfer.config.source_database.is_empty() {
                        active.get(&conn_id).and_then(|conn| {
                            conn.collections.get(&transfer.config.source_database).cloned()
                        })
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            let dest_databases: Vec<String> = transfer
                .config
                .destination_connection_id
                .and_then(|conn_id| active.get(&conn_id).map(|conn| conn.databases.clone()))
                .unwrap_or_default();

            let dest_collections: Vec<String> = transfer
                .config
                .destination_connection_id
                .and_then(|conn_id| {
                    if !transfer.config.destination_database.is_empty() {
                        active.get(&conn_id).and_then(|conn| {
                            conn.collections.get(&transfer.config.destination_database).cloned()
                        })
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            (id, transfer, connections, databases, collections, dest_databases, dest_collections)
        };
        if self.query_edit_modal.is_some() && self.query_edit_transfer_id != Some(transfer_id) {
            self.query_edit_modal = None;
            self.query_edit_input = None;
            self.query_edit_transfer_id = None;
            self.query_edit_previous_focus = None;
        }

        // Update select items and sync selected indices
        let conn_ids: Vec<Uuid> = connections.iter().map(|(id, _)| *id).collect();

        // Update connection items if changed
        if connections != self.prev_connections {
            let conn_items: Vec<ConnectionItem> = connections
                .iter()
                .map(|(id, identity)| ConnectionItem {
                    id: *id,
                    name: SharedString::from(identity.display_name()),
                    identity: identity.clone(),
                })
                .collect();

            if let Some(ref source_conn_state) = self.source_conn_state {
                source_conn_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(conn_items.clone()), window, cx);
                });
            }
            if let Some(ref dest_conn_state) = self.dest_conn_state {
                dest_conn_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(conn_items), window, cx);
                });
            }
            self.prev_connections = connections.clone();
        }

        // Sync source connection selected value
        if let Some(ref source_conn_state) = self.source_conn_state {
            source_conn_state.update(cx, |s, cx| {
                let current = s.selected_value().copied();
                if current != transfer_state.config.source_connection_id {
                    let idx = transfer_state
                        .config
                        .source_connection_id
                        .and_then(|id| conn_ids.iter().position(|c| *c == id))
                        .map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Sync destination connection selected value
        if let Some(ref dest_conn_state) = self.dest_conn_state {
            dest_conn_state.update(cx, |s, cx| {
                let current = s.selected_value().copied();
                if current != transfer_state.config.destination_connection_id {
                    let idx = transfer_state
                        .config
                        .destination_connection_id
                        .and_then(|id| conn_ids.iter().position(|c| *c == id))
                        .map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Database items - only show if connection is selected
        let db_names: Vec<String> = if transfer_state.config.source_connection_id.is_some() {
            databases.clone()
        } else {
            Vec::new()
        };

        if db_names != self.prev_db_names {
            let db_items: Vec<SharedString> =
                db_names.iter().map(|s| SharedString::from(s.clone())).collect();
            if let Some(ref source_db_state) = self.source_db_state {
                source_db_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(db_items), window, cx);
                });
            }
            self.prev_db_names = db_names.clone();
        }

        // Sync database selected value
        if let Some(ref source_db_state) = self.source_db_state {
            let expected: Option<&str> = if !transfer_state.config.source_database.is_empty() {
                Some(&transfer_state.config.source_database)
            } else {
                None
            };
            source_db_state.update(cx, |s, cx| {
                let current = s.selected_value().map(|v| v.as_ref());
                if current != expected {
                    let idx = expected
                        .and_then(|v| db_names.iter().position(|d| d == v))
                        .map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Collection items - only show if connection AND database are selected
        let coll_names: Vec<String> = if transfer_state.config.source_connection_id.is_some()
            && !transfer_state.config.source_database.is_empty()
        {
            collections.clone()
        } else {
            Vec::new()
        };

        if coll_names != self.prev_coll_names {
            let coll_items: Vec<SharedString> =
                coll_names.iter().map(|s| SharedString::from(s.clone())).collect();
            if let Some(ref source_coll_state) = self.source_coll_state {
                source_coll_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(coll_items.clone()), window, cx);
                });
            }
            // Update exclude collections dropdown with same items
            if let Some(ref exclude_coll_state) = self.exclude_coll_state {
                exclude_coll_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(coll_items), window, cx);
                    // Clear selection (multi-select behavior)
                    s.set_selected_index(None, window, cx);
                });
            }
            self.prev_coll_names = coll_names.clone();
        }

        // Sync collection selected value
        if let Some(ref source_coll_state) = self.source_coll_state {
            let expected: Option<&str> = if !transfer_state.config.source_collection.is_empty() {
                Some(&transfer_state.config.source_collection)
            } else {
                None
            };
            source_coll_state.update(cx, |s, cx| {
                let current = s.selected_value().map(|v| v.as_ref());
                if current != expected {
                    let idx = expected
                        .and_then(|v| coll_names.iter().position(|c| c == v))
                        .map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Destination database items
        let dest_db_names: Vec<String> =
            if transfer_state.config.destination_connection_id.is_some() {
                dest_databases.clone()
            } else {
                Vec::new()
            };

        if dest_db_names != self.prev_dest_db_names {
            let db_items: Vec<SharedString> =
                dest_db_names.iter().map(|s| SharedString::from(s.clone())).collect();
            if let Some(ref dest_db_state) = self.dest_db_state {
                dest_db_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(db_items), window, cx);
                });
            }
            self.prev_dest_db_names = dest_db_names.clone();
        }

        // Sync destination database selected value
        if let Some(ref dest_db_state) = self.dest_db_state {
            let expected: Option<&str> = if !transfer_state.config.destination_database.is_empty() {
                Some(&transfer_state.config.destination_database)
            } else {
                None
            };
            dest_db_state.update(cx, |s, cx| {
                let current = s.selected_value().map(|v| v.as_ref());
                if current != expected {
                    let idx = expected
                        .and_then(|v| dest_db_names.iter().position(|d| d == v))
                        .map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Destination collection items
        let dest_coll_names: Vec<String> =
            if transfer_state.config.destination_connection_id.is_some()
                && !transfer_state.config.destination_database.is_empty()
            {
                dest_collections.clone()
            } else {
                Vec::new()
            };

        if dest_coll_names != self.prev_dest_coll_names {
            let coll_items: Vec<SharedString> =
                dest_coll_names.iter().map(|s| SharedString::from(s.clone())).collect();
            if let Some(ref dest_coll_state) = self.dest_coll_state {
                dest_coll_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(coll_items), window, cx);
                });
            }
            self.prev_dest_coll_names = dest_coll_names.clone();
        }

        // Sync destination collection selected value
        if let Some(ref dest_coll_state) = self.dest_coll_state {
            let expected: Option<&str> = if !transfer_state.config.destination_collection.is_empty()
            {
                Some(&transfer_state.config.destination_collection)
            } else {
                None
            };
            dest_coll_state.update(cx, |s, cx| {
                let current = s.selected_value().map(|v| v.as_ref());
                if current != expected {
                    let idx = expected
                        .and_then(|v| dest_coll_names.iter().position(|c| c == v))
                        .map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        let state = self.state.clone();
        let appearance = self.state.read(cx).settings.appearance.clone();
        let transfer_key: u64 = (transfer_id.as_u128() & 0xffff_ffff_ffff_ffff) as u64;
        let view = cx.entity();

        let mode_tabs = islands::tab_bar(TabBar::new(("transfer-mode", transfer_key)), &appearance)
            .small()
            .selected_index(transfer_state.config.mode.index())
            .on_click({
                let state = state.clone();
                let view = view.clone();
                move |index, _window, cx| {
                    let mode = TransferMode::from_index(*index);
                    view.update(cx, |view, cx| {
                        view.options_expanded = false;
                        cx.notify();
                    });
                    state.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            if tab.runtime.is_running || tab.runtime.cancellation_pending() {
                                return;
                            }
                            if tab.config.mode != mode {
                                tab.runtime.has_started = false;
                                tab.runtime.progress_count = 0;
                                tab.runtime.error_message = None;
                                tab.runtime.database_progress = None;
                            }
                            tab.config.mode = mode;
                            if matches!(mode, TransferMode::Import) {
                                tab.config.destination_database.clear();
                                tab.config.destination_collection.clear();
                            }
                            tab.config.format = coerce_transfer_format(
                                tab.config.mode,
                                tab.config.scope,
                                tab.config.format,
                            );
                            cx.notify();
                        }
                    });
                }
            })
            .children(vec![
                Tab::new().label("Export"),
                Tab::new().label("Import"),
                Tab::new().label("Copy"),
            ]);

        let subtitle = match transfer_state.config.mode {
            TransferMode::Export => "Export data to a file",
            TransferMode::Import => "Import data from a file",
            TransferMode::Copy => "Copy data between connections",
        };
        let header = div()
            .flex()
            .items_center()
            .h(sizing::header_height())
            .px(spacing::lg())
            .bg(islands::tool_bg(&appearance, cx))
            .border_b_1()
            .border_color(islands::panel_border(&appearance, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child(transfer_state.config.mode.label()),
                    )
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(subtitle)),
            );

        let source_identity = transfer_state.config.source_connection_id.and_then(|id| {
            connections
                .iter()
                .find(|(connection_id, _)| *connection_id == id)
                .map(|(_, identity)| identity)
        });
        let destination_identity = transfer_state.config.destination_connection_id.and_then(|id| {
            connections
                .iter()
                .find(|(connection_id, _)| *connection_id == id)
                .map(|(_, identity)| identity)
        });

        let transfer_key_context =
            match (self.query_edit_modal.is_some(), transfer_state.runtime.is_running) {
                (true, true) => "Transfer TransferRunning TransferQueryModal",
                (true, false) => "Transfer TransferQueryModal",
                (false, true) => "Transfer TransferRunning",
                (false, false) => "Transfer",
            };
        let modal_overlay = self.render_query_edit_modal(window, cx);

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .key_context(transfer_key_context)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &RunTransfer, window, cx| {
                run_active_transfer(this.state.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &CancelTransfer, _window, cx| {
                cancel_active_transfer(this.state.clone(), cx);
            }))
            .on_action(cx.listener(|this, _: &SaveTransferQuery, window, cx| {
                this.save_query_modal(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTransferQueryModal, window, cx| {
                this.close_query_modal(window, cx);
            }))
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .p(spacing::lg())
                    .overflow_hidden()
                    .child(div().mb(px(20.0)).child(mode_tabs))
                    .child(self.render_simple_transfer(
                        transfer_id,
                        transfer_key,
                        &transfer_state,
                        source_identity,
                        destination_identity,
                        view,
                        window,
                        cx,
                    )),
            )
            .child(modal_overlay)
            .into_any_element()
    }
}
