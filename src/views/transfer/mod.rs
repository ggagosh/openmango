//! Transfer view for import, export, and copy operations.

mod destination;
mod helpers;
mod options;
mod progress_panel;
mod query_modal;
mod select_states;
mod source_panel;
mod summary_panel;

pub use query_modal::QueryEditField;

use gpui::*;
use gpui_component::button::Button as MenuButton;
use gpui_component::input::InputState;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::select::{SearchableVec, SelectState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, IndexPath, Sizable as _, Size};
use uuid::Uuid;

use crate::components::Button;
use crate::state::{
    AppCommands, AppState, CompressionMode, TransferFormat, TransferMode, TransferScope,
    TransferTabState,
};
use crate::theme::{borders, colors, sizing, spacing};

use helpers::{option_field, option_field_static, option_section};
use progress_panel::{render_progress_panel, render_warnings};
use select_states::ConnectionItem;
use summary_panel::{can_execute_transfer, render_summary_panel};

pub struct TransferView {
    state: Entity<AppState>,
    _subscriptions: Vec<Subscription>,
    _select_subscriptions: Vec<Subscription>,
    options_expanded: bool,

    // Select states for searchable dropdowns (lazily initialized on first render)
    source_conn_state: Option<Entity<SelectState<SearchableVec<ConnectionItem>>>>,
    source_db_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    source_coll_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    dest_conn_state: Option<Entity<SelectState<SearchableVec<ConnectionItem>>>>,

    // Exclude collections multi-select state
    exclude_coll_state: Option<Entity<SelectState<SearchableVec<SharedString>>>>,

    // Input state for export path (lazily initialized on first render)
    export_path_input_state: Option<Entity<InputState>>,

    // Track previous items to avoid resetting search state on every render
    prev_conn_ids: Vec<Uuid>,
    prev_db_names: Vec<String>,
    prev_coll_names: Vec<String>,

    // JSON editor modal state
    query_edit_modal: Option<QueryEditField>, // Which field is being edited (None = closed)
    query_edit_input: Option<Entity<InputState>>, // Textarea content for modal
}

impl TransferView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];

        Self {
            state,
            _subscriptions: subscriptions,
            _select_subscriptions: Vec::new(),
            options_expanded: true,
            source_conn_state: None,
            source_db_state: None,
            source_coll_state: None,
            dest_conn_state: None,
            exclude_coll_state: None,
            export_path_input_state: None,
            prev_conn_ids: Vec::new(),
            prev_db_names: Vec::new(),
            prev_coll_names: Vec::new(),
            query_edit_modal: None,
            query_edit_input: None,
        }
    }
}

impl Render for TransferView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Ensure select states are initialized
        self.ensure_select_states(window, cx);

        // Read state once at the start
        let (transfer_id, transfer_state, connections, databases, collections) = {
            let state_ref = self.state.read(cx);
            let Some(id) = state_ref.active_transfer_tab_id() else {
                return div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(colors::text_muted())
                    .child("Open a Transfer tab to configure import/export")
                    .into_any_element();
            };
            let transfer = state_ref.transfer_tab(id).cloned().unwrap_or_default();

            let active = state_ref.active_connections_snapshot();
            let connections: Vec<(Uuid, String)> =
                active.iter().map(|(id, conn)| (*id, conn.config.name.clone())).collect();

            let databases: Vec<String> = transfer
                .source_connection_id
                .and_then(|conn_id| active.get(&conn_id).map(|conn| conn.databases.clone()))
                .unwrap_or_default();

            let collections: Vec<String> = transfer
                .source_connection_id
                .and_then(|conn_id| {
                    if !transfer.source_database.is_empty() {
                        active.get(&conn_id).and_then(|conn| {
                            conn.collections.get(&transfer.source_database).cloned()
                        })
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            (id, transfer, connections, databases, collections)
        };

        // Update select items and sync selected indices
        let conn_ids: Vec<Uuid> = connections.iter().map(|(id, _)| *id).collect();

        // Update connection items if changed
        if conn_ids != self.prev_conn_ids {
            let conn_items: Vec<ConnectionItem> = connections
                .iter()
                .map(|(id, name)| ConnectionItem {
                    id: *id,
                    name: SharedString::from(name.clone()),
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
            self.prev_conn_ids = conn_ids.clone();
        }

        // Sync source connection selected index
        if let Some(ref source_conn_state) = self.source_conn_state {
            let expected_row = transfer_state
                .source_connection_id
                .and_then(|id| conn_ids.iter().position(|c| *c == id));
            source_conn_state.update(cx, |s, cx| {
                let current_row = s.selected_index(cx).map(|ip| ip.row);
                if current_row != expected_row {
                    let idx = expected_row.map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Sync destination connection selected index
        if let Some(ref dest_conn_state) = self.dest_conn_state {
            let expected_row = transfer_state
                .destination_connection_id
                .and_then(|id| conn_ids.iter().position(|c| *c == id));
            dest_conn_state.update(cx, |s, cx| {
                let current_row = s.selected_index(cx).map(|ip| ip.row);
                if current_row != expected_row {
                    let idx = expected_row.map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Database items - only show if connection is selected
        let db_names: Vec<String> = if transfer_state.source_connection_id.is_some() {
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

        // Sync database selected index
        if let Some(ref source_db_state) = self.source_db_state {
            let expected_row = if !transfer_state.source_database.is_empty() {
                db_names.iter().position(|d| d == &transfer_state.source_database)
            } else {
                None
            };
            source_db_state.update(cx, |s, cx| {
                let current_row = s.selected_index(cx).map(|ip| ip.row);
                if current_row != expected_row {
                    let idx = expected_row.map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        // Collection items - only show if connection AND database are selected
        let coll_names: Vec<String> = if transfer_state.source_connection_id.is_some()
            && !transfer_state.source_database.is_empty()
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

        // Sync collection selected index
        if let Some(ref source_coll_state) = self.source_coll_state {
            let expected_row = if !transfer_state.source_collection.is_empty() {
                coll_names.iter().position(|c| c == &transfer_state.source_collection)
            } else {
                None
            };
            source_coll_state.update(cx, |s, cx| {
                let current_row = s.selected_index(cx).map(|ip| ip.row);
                if current_row != expected_row {
                    let idx = expected_row.map(|r| IndexPath::default().row(r));
                    s.set_selected_index(idx, window, cx);
                }
            });
        }

        let state = self.state.clone();
        let transfer_key: u64 = (transfer_id.as_u128() & 0xffff_ffff_ffff_ffff) as u64;
        let options_expanded = self.options_expanded;
        let view = cx.entity();

        // Mode tabs
        let mode_tabs = TabBar::new(("transfer-mode", transfer_key))
            .underline()
            .small()
            .selected_index(transfer_state.mode.index())
            .on_click({
                let state = state.clone();
                move |index, _window, cx| {
                    let mode = TransferMode::from_index(*index);
                    state.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            tab.mode = mode;
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

        // Scope dropdown
        let scope_button = {
            let state = state.clone();
            MenuButton::new(("transfer-scope", transfer_key))
                .compact()
                .label(transfer_state.scope.label())
                .dropdown_caret(true)
                .rounded(borders::radius_sm())
                .with_size(Size::XSmall)
                .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
                    let state = state.clone();
                    let state2 = state.clone();
                    menu.item(PopupMenuItem::new("Collection").on_click({
                        move |_, _, cx| {
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.scope = TransferScope::Collection;
                                    if matches!(tab.format, TransferFormat::Bson) {
                                        tab.format = TransferFormat::JsonLines;
                                    }
                                    cx.notify();
                                }
                            });
                        }
                    }))
                    .item(PopupMenuItem::new("Database").on_click({
                        move |_, _, cx| {
                            state2.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.scope = TransferScope::Database;
                                    cx.notify();
                                }
                            });
                        }
                    }))
                })
        };

        // Format dropdown
        let format_control =
            if matches!(transfer_state.mode, TransferMode::Export | TransferMode::Import) {
                let state = state.clone();
                let is_collection = matches!(transfer_state.scope, TransferScope::Collection);
                MenuButton::new(("transfer-format", transfer_key))
                    .compact()
                    .label(transfer_state.format.label())
                    .dropdown_caret(true)
                    .rounded(borders::radius_sm())
                    .with_size(Size::XSmall)
                    .dropdown_menu_with_anchor(Corner::BottomLeft, move |mut menu, _window, _cx| {
                        let s1 = state.clone();
                        let s2 = state.clone();
                        let s3 = state.clone();
                        let s4 = state.clone();

                        menu = menu
                            .item(PopupMenuItem::new("JSON Lines (.jsonl)").on_click(
                                move |_, _, cx| {
                                    s1.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.format = TransferFormat::JsonLines;
                                            tab.file_path.clear();
                                            cx.notify();
                                        }
                                    });
                                },
                            ))
                            .item(PopupMenuItem::new("JSON array (.json)").on_click(
                                move |_, _, cx| {
                                    s2.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.format = TransferFormat::JsonArray;
                                            tab.file_path.clear();
                                            cx.notify();
                                        }
                                    });
                                },
                            ))
                            .item(PopupMenuItem::new("CSV (.csv)").on_click(move |_, _, cx| {
                                s3.update(cx, |state, cx| {
                                    if let Some(id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(id)
                                    {
                                        tab.format = TransferFormat::Csv;
                                        tab.file_path.clear();
                                        cx.notify();
                                    }
                                });
                            }));

                        if !is_collection {
                            menu = menu.item(PopupMenuItem::new("BSON (mongodump)").on_click(
                                move |_, _, cx| {
                                    s4.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.format = TransferFormat::Bson;
                                            tab.file_path.clear();
                                            cx.notify();
                                        }
                                    });
                                },
                            ));
                        }
                        menu
                    })
                    .into_any_element()
            } else {
                div().into_any_element()
            };

        // Run or Cancel button (depending on is_running state)
        let can_run = can_execute_transfer(&transfer_state);
        let action_button = if transfer_state.is_running {
            let state = state.clone();
            Button::new("transfer-cancel")
                .ghost()
                .compact()
                .label("Cancel")
                .on_click(move |_, _, cx| {
                    AppCommands::cancel_transfer(state.clone(), transfer_id, cx);
                })
                .into_any_element()
        } else {
            let state = state.clone();
            Button::new("transfer-run")
                .primary()
                .compact()
                .label(transfer_state.mode.label())
                .disabled(!can_run)
                .on_click(move |_, _, cx| {
                    AppCommands::execute_transfer(state.clone(), transfer_id, cx);
                })
                .into_any_element()
        };

        // Header
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(sizing::header_height())
            .px(spacing::lg())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors::text_primary())
                            .child("Transfer"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors::text_muted())
                            .child("Import, export, or copy data"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(scope_button)
                    .child(format_control)
                    .child(action_button),
            );

        // Source panel
        let source_panel = self.render_source_panel(&transfer_state, cx);

        // Destination panel
        let destination_panel = self.render_destination_panel(&transfer_state, window, cx);

        let source_conn_name = transfer_state
            .source_connection_id
            .and_then(|id| {
                connections.iter().find(|(cid, _)| *cid == id).map(|(_, name)| name.clone())
            })
            .unwrap_or_else(|| "Select connection".to_string());

        let dest_conn_name = transfer_state
            .destination_connection_id
            .and_then(|id| {
                connections.iter().find(|(cid, _)| *cid == id).map(|(_, name)| name.clone())
            })
            .unwrap_or_else(|| "Select connection".to_string());

        // Summary panel (now inline)
        let summary_panel =
            render_summary_panel(&transfer_state, &source_conn_name, &dest_conn_name);

        // Progress panel for database-scope operations (only shown when running)
        let progress_panel: AnyElement =
            if let Some(ref db_progress) = transfer_state.database_progress {
                render_progress_panel(db_progress, state.clone(), transfer_id).into_any_element()
            } else {
                div().into_any_element()
            };

        // Warning banners
        let warnings = render_warnings(&transfer_state);

        // Options panel
        let options_panel =
            self.render_options_panel(transfer_key, &transfer_state, options_expanded, view);

        // Error message
        let error_display: AnyElement = if let Some(error) = &transfer_state.error_message {
            div()
                .px(spacing::md())
                .py(spacing::sm())
                .bg(hsla(0.0, 0.7, 0.5, 0.1))
                .border_1()
                .border_color(hsla(0.0, 0.7, 0.5, 0.3))
                .rounded(borders::radius_sm())
                .text_sm()
                .text_color(hsla(0.0, 0.7, 0.5, 1.0))
                .overflow_hidden()
                .max_h(px(120.0))
                .overflow_y_scrollbar()
                .child(error.clone())
                .into_any_element()
        } else {
            div().into_any_element()
        };

        // Section spacing
        let section_gap = px(20.0);

        // Modal overlay for query editing
        let modal_overlay = self.render_query_edit_modal(window, cx);

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .p(spacing::lg())
                    .overflow_y_scrollbar()
                    // Mode tabs
                    .child(div().mb(section_gap).child(mode_tabs))
                    // Source panel - full width
                    .child(div().mb(section_gap).child(source_panel))
                    // Destination panel - full width
                    .child(div().mb(section_gap).child(destination_panel))
                    // Options panel - collapsible
                    .child(div().mb(section_gap).child(options_panel))
                    // Warnings (only shown when needed)
                    .child(warnings)
                    // Error display (only shown when needed)
                    .child(error_display)
                    // Summary at bottom - review before action
                    .child(summary_panel)
                    // Progress panel for database-scope operations
                    .child(div().mt(spacing::md()).child(progress_panel)),
            )
            .child(modal_overlay)
            .into_any_element()
    }
}

impl TransferView {
    fn render_options_panel(
        &self,
        key: u64,
        transfer_state: &TransferTabState,
        expanded: bool,
        view: Entity<Self>,
    ) -> impl IntoElement {
        let state = self.state.clone();

        let header = div()
            .id(("options-header", key))
            .flex()
            .items_center()
            .gap(spacing::xs())
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                view.update(cx, |view, cx| {
                    view.options_expanded = !view.options_expanded;
                    cx.notify();
                });
            })
            .child(
                Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(colors::text_muted()),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::text_secondary())
                    .child("Options"),
            );

        let content = if expanded {
            let mut sections = Vec::new();

            // General section with compression dropdown
            let compression_dropdown = {
                let state = state.clone();
                MenuButton::new(("compression", key))
                    .compact()
                    .label(transfer_state.compression.label())
                    .dropdown_caret(true)
                    .rounded(borders::radius_sm())
                    .with_size(Size::XSmall)
                    .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
                        let s1 = state.clone();
                        let s2 = state.clone();
                        menu.item(PopupMenuItem::new("None").on_click(move |_, _, cx| {
                            s1.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.compression = CompressionMode::None;
                                    cx.notify();
                                }
                            });
                        }))
                        .item(PopupMenuItem::new("Gzip").on_click(
                            move |_, _, cx| {
                                s2.update(cx, |state, cx| {
                                    if let Some(id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(id)
                                    {
                                        tab.compression = CompressionMode::Gzip;
                                        cx.notify();
                                    }
                                });
                            },
                        ))
                    })
            };

            sections.push(
                option_section(
                    "General",
                    vec![
                        option_field_static("Scope", transfer_state.scope.label()),
                        option_field_static(
                            "Format",
                            if matches!(transfer_state.mode, TransferMode::Copy) {
                                "Live copy"
                            } else {
                                transfer_state.format.label()
                            },
                        ),
                        option_field("Compression", compression_dropdown.into_any_element()),
                    ],
                )
                .into_any_element(),
            );

            match transfer_state.mode {
                TransferMode::Export => {
                    options::render_export_options(
                        &mut sections,
                        state.clone(),
                        key,
                        transfer_state,
                        self.exclude_coll_state.as_ref(),
                    );
                }
                TransferMode::Import => {
                    options::render_import_options(
                        &mut sections,
                        state.clone(),
                        key,
                        transfer_state,
                    );
                }
                TransferMode::Copy => {
                    options::render_copy_options(
                        &mut sections,
                        state.clone(),
                        key,
                        transfer_state,
                        self.exclude_coll_state.as_ref(),
                    );
                }
            }

            div().flex().flex_wrap().items_start().gap(spacing::md()).children(sections)
        } else {
            div()
        };

        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .p(spacing::md())
            .bg(colors::bg_sidebar())
            .border_1()
            .border_color(colors::border_subtle())
            .rounded(borders::radius_sm())
            .child(header)
            .child(content)
    }
}
