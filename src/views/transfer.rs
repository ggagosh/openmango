//! Transfer view for import, export, and copy operations.

use gpui::*;
use gpui_component::button::Button as MenuButton;
use gpui_component::checkbox::Checkbox;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, Sizable as _, Size};
use uuid::Uuid;

use crate::components::Button;
use crate::components::file_picker::{
    FilePickerMode, default_export_filename, filters_for_format, open_file_dialog,
};
use crate::state::{
    AppCommands, AppState, BsonOutputFormat, ExtendedJsonMode, InsertMode, TransferFormat,
    TransferMode, TransferScope, TransferTabState,
};
use crate::theme::{borders, colors, sizing, spacing};

// Custom SelectItem for connections (stores UUID + display name)
#[derive(Clone, Debug)]
struct ConnectionItem {
    id: Uuid,
    name: SharedString,
}

impl SelectItem for ConnectionItem {
    type Value = Uuid;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(&query.to_lowercase())
    }
}

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

    // Track previous items to avoid resetting search state on every render
    prev_conn_ids: Vec<Uuid>,
    prev_db_names: Vec<String>,
    prev_coll_names: Vec<String>,
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
            prev_conn_ids: Vec::new(),
            prev_db_names: Vec::new(),
            prev_coll_names: Vec::new(),
        }
    }

    /// Initialize select states on first render (when window is available)
    fn ensure_select_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.source_conn_state.is_some() {
            return; // Already initialized
        }

        let state = self.state.clone();

        // Create select states
        let source_conn_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let source_db_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let source_coll_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let dest_conn_state = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });

        // Subscribe to select events
        let state_clone = state.clone();
        let sub1 = cx.subscribe_in(
            &source_conn_state,
            window,
            move |view, _select_state, event, window, cx| {
                if let SelectEvent::Confirm(Some(conn_id)) = event {
                    let tab_id = state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.source_connection_id = Some(*conn_id);
                            tab.source_database.clear();
                            tab.source_collection.clear();
                            cx.notify();
                            return Some(tab_id);
                        }
                        None
                    });
                    // Clear dependent selects
                    if let Some(ref db_state) = view.source_db_state {
                        db_state.update(cx, |s, cx| {
                            s.set_selected_index(None, window, cx);
                        });
                    }
                    if let Some(ref coll_state) = view.source_coll_state {
                        coll_state.update(cx, |s, cx| {
                            s.set_selected_index(None, window, cx);
                        });
                    }
                    if tab_id.is_some() {
                        cx.notify();
                    }
                }
            },
        );

        let state_clone = state.clone();
        let sub2 = cx.subscribe_in(
            &source_db_state,
            window,
            move |view,
                  _select_state,
                  event: &SelectEvent<SearchableVec<SharedString>>,
                  window,
                  cx| {
                if let SelectEvent::Confirm(Some(db_name)) = event {
                    let db_str = db_name.to_string();
                    state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.source_database = db_str.clone();
                            tab.source_collection.clear();
                            cx.notify();
                        }
                    });
                    // Clear collection select
                    if let Some(ref coll_state) = view.source_coll_state {
                        coll_state.update(cx, |s, cx| {
                            s.set_selected_index(None, window, cx);
                        });
                    }
                    cx.notify();
                }
            },
        );

        let state_clone = state.clone();
        let sub3 = cx.subscribe_in(
            &source_coll_state,
            window,
            move |_view,
                  _select_state,
                  event: &SelectEvent<SearchableVec<SharedString>>,
                  _window,
                  cx| {
                if let SelectEvent::Confirm(Some(coll_name)) = event {
                    let coll_str = coll_name.to_string();
                    let tab_id = state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.source_collection = coll_str.clone();
                            cx.notify();
                            return Some(tab_id);
                        }
                        None
                    });
                    if let Some(tab_id) = tab_id {
                        AppCommands::load_transfer_preview(state_clone.clone(), tab_id, cx);
                    }
                }
            },
        );

        let state_clone = state.clone();
        let sub4 = cx.subscribe_in(
            &dest_conn_state,
            window,
            move |_view, _select_state, event, _window, cx| {
                if let SelectEvent::Confirm(Some(conn_id)) = event {
                    state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.destination_connection_id = Some(*conn_id);
                            cx.notify();
                        }
                    });
                }
            },
        );

        self._select_subscriptions = vec![sub1, sub2, sub3, sub4];
        self.source_conn_state = Some(source_conn_state);
        self.source_db_state = Some(source_db_state);
        self.source_coll_state = Some(source_coll_state);
        self.dest_conn_state = Some(dest_conn_state);
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

        // Update select items only when data changes (to preserve search state)
        let conn_ids: Vec<Uuid> = connections.iter().map(|(id, _)| *id).collect();
        let db_names: Vec<String> = databases.clone();
        let coll_names: Vec<String> = collections.clone();

        // Only update connection items if they changed
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
            self.prev_conn_ids = conn_ids;
        }

        // Only update database items if they changed
        if db_names != self.prev_db_names {
            let db_items: Vec<SharedString> =
                databases.iter().map(|s| SharedString::from(s.clone())).collect();
            if let Some(ref source_db_state) = self.source_db_state {
                source_db_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(db_items), window, cx);
                });
            }
            self.prev_db_names = db_names;
        }

        // Only update collection items if they changed
        if coll_names != self.prev_coll_names {
            let coll_items: Vec<SharedString> =
                collections.iter().map(|s| SharedString::from(s.clone())).collect();
            if let Some(ref source_coll_state) = self.source_coll_state {
                source_coll_state.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(coll_items), window, cx);
                });
            }
            self.prev_coll_names = coll_names;
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

        // Run button
        let can_run = can_execute_transfer(&transfer_state);
        let run_button = {
            let state = state.clone();
            Button::new("transfer-run")
                .primary()
                .compact()
                .label(transfer_state.mode.label())
                .disabled(!can_run || transfer_state.is_running)
                .on_click(move |_, _, cx| {
                    AppCommands::execute_transfer(state.clone(), transfer_id, cx);
                })
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
                    .child(run_button),
            );

        // Source panel
        let source_panel = self.render_source_panel(&transfer_state);

        // Destination panel
        let destination_panel = self.render_destination_panel(&transfer_state);

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

        let side_panel = render_side_panel(&transfer_state, &source_conn_name, &dest_conn_name);

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
                .child(error.clone())
                .into_any_element()
        } else {
            div().into_any_element()
        };

        div()
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
                    .gap(spacing::md())
                    .p(spacing::md())
                    .overflow_y_scrollbar()
                    .child(
                        // Main content area with panels
                        div()
                            .flex()
                            .gap(spacing::md())
                            .items_start()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(spacing::md())
                                    .flex_1()
                                    .child(mode_tabs)
                                    .child(
                                        div()
                                            .flex()
                                            .gap(spacing::md())
                                            .items_start()
                                            .child(source_panel)
                                            .child(destination_panel),
                                    )
                                    .child(warnings)
                                    .child(error_display)
                                    .child(options_panel),
                            )
                            .child(side_panel),
                    ),
            )
            .into_any_element()
    }
}

impl TransferView {
    fn render_source_panel(&self, transfer_state: &TransferTabState) -> impl IntoElement {
        let show_collection = matches!(transfer_state.scope, TransferScope::Collection);

        // Searchable select components (states are initialized by ensure_select_states)
        let Some(ref source_conn_state) = self.source_conn_state else {
            return panel("Source", div().child("Loading...")).flex_1();
        };
        let Some(ref source_db_state) = self.source_db_state else {
            return panel("Source", div().child("Loading...")).flex_1();
        };

        let conn_select =
            Select::new(source_conn_state).small().w_full().placeholder("Select connection...");

        let db_select =
            Select::new(source_db_state).small().w_full().placeholder("Select database...");

        let coll_select = if show_collection {
            self.source_coll_state.as_ref().map(|coll_state| {
                Select::new(coll_state).small().w_full().placeholder("Select collection...")
            })
        } else {
            None
        };

        panel(
            "Source",
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .child(form_field("Connection", conn_select))
                .child(form_field("Database", db_select))
                .children(coll_select.map(|s| form_field("Collection", s))),
        )
        .flex_1()
    }

    fn render_destination_panel(&self, transfer_state: &TransferTabState) -> impl IntoElement {
        let state = self.state.clone();

        match transfer_state.mode {
            TransferMode::Export => {
                let file_path = if transfer_state.file_path.is_empty() {
                    "No file selected".to_string()
                } else {
                    // Show just filename with ellipsis for long paths
                    let path = std::path::Path::new(&transfer_state.file_path);
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| transfer_state.file_path.clone())
                };

                let browse_button = {
                    let state = state.clone();
                    let format = transfer_state.format;
                    let db = transfer_state.source_database.clone();
                    let coll = transfer_state.source_collection.clone();
                    Button::new("browse-export").compact().label("Browse...").on_click(
                        move |_, _, cx| {
                            let filters = filters_for_format(format);
                            let default_name = default_export_filename(&db, &coll, format);
                            if let Some(path) =
                                open_file_dialog(FilePickerMode::Save, filters, Some(&default_name))
                            {
                                state.update(cx, |state, cx| {
                                    if let Some(tab_id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(tab_id)
                                    {
                                        tab.file_path = path.display().to_string();
                                        cx.notify();
                                    }
                                });
                            }
                        },
                    )
                };

                let file_control = div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        value_box(file_path, transfer_state.file_path.is_empty())
                            .flex_1()
                            .overflow_x_hidden()
                            .text_ellipsis(),
                    )
                    .child(browse_button);

                panel(
                    "Destination",
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::md())
                        .child(form_field("File", file_control)),
                )
                .flex_1()
                .into_any_element()
            }
            TransferMode::Import => {
                let file_path = if transfer_state.file_path.is_empty() {
                    "No file selected".to_string()
                } else {
                    let path = std::path::Path::new(&transfer_state.file_path);
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| transfer_state.file_path.clone())
                };

                let browse_button = {
                    let state = state.clone();
                    let format = transfer_state.format;
                    Button::new("browse-import").compact().label("Browse...").on_click(
                        move |_, _, cx| {
                            let filters = filters_for_format(format);
                            if let Some(path) =
                                open_file_dialog(FilePickerMode::Open, filters, None)
                            {
                                state.update(cx, |state, cx| {
                                    if let Some(tab_id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(tab_id)
                                    {
                                        // Auto-detect format
                                        if let Some(ext) = path.extension().and_then(|e| e.to_str())
                                        {
                                            tab.format = match ext {
                                                "jsonl" | "ndjson" => TransferFormat::JsonLines,
                                                "json" => TransferFormat::JsonArray,
                                                "csv" => TransferFormat::Csv,
                                                _ => tab.format,
                                            };
                                        }
                                        tab.file_path = path.display().to_string();
                                        cx.notify();
                                    }
                                });
                            }
                        },
                    )
                };

                let target_db = if transfer_state.destination_database.is_empty() {
                    &transfer_state.source_database
                } else {
                    &transfer_state.destination_database
                };

                let target_coll = if transfer_state.destination_collection.is_empty() {
                    &transfer_state.source_collection
                } else {
                    &transfer_state.destination_collection
                };

                let show_coll = matches!(transfer_state.scope, TransferScope::Collection);

                let file_control = div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        value_box(file_path, transfer_state.file_path.is_empty())
                            .flex_1()
                            .overflow_x_hidden()
                            .text_ellipsis(),
                    )
                    .child(browse_button);

                panel(
                    "Destination",
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::md())
                        .child(form_field("File", file_control))
                        .child(form_field_static("Target database", target_db))
                        .children(
                            show_coll.then(|| form_field_static("Target collection", target_coll)),
                        ),
                )
                .flex_1()
                .into_any_element()
            }
            TransferMode::Copy => {
                // Searchable select for destination connection
                let Some(ref dest_conn_state) = self.dest_conn_state else {
                    return panel("Destination", div().child("Loading..."))
                        .flex_1()
                        .into_any_element();
                };

                let conn_select = Select::new(dest_conn_state)
                    .small()
                    .w_full()
                    .placeholder("Select connection...");

                let target_db = if transfer_state.destination_database.is_empty() {
                    &transfer_state.source_database
                } else {
                    &transfer_state.destination_database
                };

                let target_coll = if transfer_state.destination_collection.is_empty() {
                    &transfer_state.source_collection
                } else {
                    &transfer_state.destination_collection
                };

                let show_coll = matches!(transfer_state.scope, TransferScope::Collection);

                panel(
                    "Destination",
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::md())
                        .child(form_field("Connection", conn_select))
                        .child(form_field_static("Database", target_db))
                        .children(show_coll.then(|| form_field_static("Collection", target_coll))),
                )
                .flex_1()
                .into_any_element()
            }
        }
    }

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
                        option_field_static("Compression", "Off"),
                    ],
                )
                .into_any_element(),
            );

            match transfer_state.mode {
                TransferMode::Export => {
                    sections.push(
                        option_section(
                            "Query",
                            vec![
                                option_field_static("Filter", "Query bar"),
                                option_field_static("Projection", "Query bar"),
                                option_field_static("Sort", "Query bar"),
                                option_field_static("Limit", "None"),
                            ],
                        )
                        .into_any_element(),
                    );

                    match transfer_state.format {
                        TransferFormat::Csv => {
                            sections.push(
                                option_section(
                                    "CSV Options",
                                    vec![
                                        option_field_static("Delimiter", ","),
                                        option_field_static("Header row", "On"),
                                        option_field_static("Flatten fields", "On"),
                                        option_field_static("Encoding", "UTF-8"),
                                    ],
                                )
                                .into_any_element(),
                            );
                        }
                        TransferFormat::Bson => {
                            let bson_output_dropdown = {
                                let state = state.clone();
                                MenuButton::new(("bson-output", key))
                                    .compact()
                                    .label(transfer_state.bson_output.label())
                                    .dropdown_caret(true)
                                    .rounded(borders::radius_sm())
                                    .with_size(Size::XSmall)
                                    .dropdown_menu_with_anchor(
                                        Corner::BottomLeft,
                                        move |menu, _window, _cx| {
                                            let s1 = state.clone();
                                            let s2 = state.clone();
                                            menu.item(PopupMenuItem::new("Folder").on_click(
                                                move |_, _, cx| {
                                                    s1.update(cx, |state, cx| {
                                                        if let Some(id) =
                                                            state.active_transfer_tab_id()
                                                            && let Some(tab) =
                                                                state.transfer_tab_mut(id)
                                                        {
                                                            tab.bson_output =
                                                                BsonOutputFormat::Folder;
                                                            cx.notify();
                                                        }
                                                    });
                                                },
                                            ))
                                            .item(
                                                PopupMenuItem::new("Archive (.archive)").on_click(
                                                    move |_, _, cx| {
                                                        s2.update(cx, |state, cx| {
                                                            if let Some(id) =
                                                                state.active_transfer_tab_id()
                                                                && let Some(tab) =
                                                                    state.transfer_tab_mut(id)
                                                            {
                                                                tab.bson_output =
                                                                    BsonOutputFormat::Archive;
                                                                cx.notify();
                                                            }
                                                        });
                                                    },
                                                ),
                                            )
                                        },
                                    )
                            };

                            sections.push(
                                option_section(
                                    "BSON Options",
                                    vec![option_field(
                                        "Output",
                                        bson_output_dropdown.into_any_element(),
                                    )],
                                )
                                .into_any_element(),
                            );
                        }
                        _ => {
                            let json_mode_dropdown =
                                {
                                    let state = state.clone();
                                    MenuButton::new(("json-mode", key))
                                        .compact()
                                        .label(transfer_state.json_mode.label())
                                        .dropdown_caret(true)
                                        .rounded(borders::radius_sm())
                                        .with_size(Size::XSmall)
                                        .dropdown_menu_with_anchor(
                                            Corner::BottomLeft,
                                            move |menu, _window, _cx| {
                                                let s1 = state.clone();
                                                let s2 = state.clone();
                                                menu.item(PopupMenuItem::new("Relaxed").on_click(
                                                    move |_, _, cx| {
                                                        s1.update(cx, |state, cx| {
                                                            if let Some(id) =
                                                                state.active_transfer_tab_id()
                                                                && let Some(tab) =
                                                                    state.transfer_tab_mut(id)
                                                            {
                                                                tab.json_mode =
                                                                    ExtendedJsonMode::Relaxed;
                                                                cx.notify();
                                                            }
                                                        });
                                                    },
                                                ))
                                                .item(PopupMenuItem::new("Canonical").on_click(
                                                    move |_, _, cx| {
                                                        s2.update(cx, |state, cx| {
                                                            if let Some(id) =
                                                                state.active_transfer_tab_id()
                                                                && let Some(tab) =
                                                                    state.transfer_tab_mut(id)
                                                            {
                                                                tab.json_mode =
                                                                    ExtendedJsonMode::Canonical;
                                                                cx.notify();
                                                            }
                                                        });
                                                    },
                                                ))
                                            },
                                        )
                                };

                            let pretty_checkbox = {
                                let state = state.clone();
                                let checked = transfer_state.pretty_print;
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::sm())
                                    .child(
                                        Checkbox::new(("pretty-print", key))
                                            .checked(checked)
                                            .on_click(move |_, _, cx| {
                                                state.update(cx, |state, cx| {
                                                    if let Some(id) = state.active_transfer_tab_id()
                                                        && let Some(tab) =
                                                            state.transfer_tab_mut(id)
                                                    {
                                                        tab.pretty_print = !checked;
                                                        cx.notify();
                                                    }
                                                });
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(colors::text_secondary())
                                            .child("Enabled"),
                                    )
                            };

                            sections.push(
                                option_section(
                                    "JSON Options",
                                    vec![
                                        option_field(
                                            "Extended JSON",
                                            json_mode_dropdown.into_any_element(),
                                        ),
                                        option_field(
                                            "Pretty print",
                                            pretty_checkbox.into_any_element(),
                                        ),
                                        option_field_static(
                                            "Container",
                                            match transfer_state.format {
                                                TransferFormat::JsonArray => "JSON array",
                                                _ => "JSON lines",
                                            },
                                        ),
                                        option_field_static("Allow shell syntax", "On"),
                                    ],
                                )
                                .into_any_element(),
                            );
                        }
                    }

                    if matches!(transfer_state.scope, TransferScope::Database) {
                        sections.push(
                            option_section(
                                "Database",
                                vec![
                                    option_field_static("Include collections", "All"),
                                    option_field_static("Exclude collections", "None"),
                                    option_field_static("Include indexes/options", "On"),
                                ],
                            )
                            .into_any_element(),
                        );
                    } else {
                        sections.push(
                            option_section(
                                "Collection",
                                vec![option_field_static("Include indexes", "Off")],
                            )
                            .into_any_element(),
                        );
                    }
                }
                TransferMode::Import => {
                    let insert_mode_dropdown = {
                        let state = state.clone();
                        MenuButton::new(("insert-mode", key))
                            .compact()
                            .label(transfer_state.insert_mode.label())
                            .dropdown_caret(true)
                            .rounded(borders::radius_sm())
                            .with_size(Size::XSmall)
                            .dropdown_menu_with_anchor(
                                Corner::BottomLeft,
                                move |menu, _window, _cx| {
                                    let s1 = state.clone();
                                    let s2 = state.clone();
                                    let s3 = state.clone();
                                    menu.item(PopupMenuItem::new("Insert").on_click(
                                        move |_, _, cx| {
                                            s1.update(cx, |state, cx| {
                                                if let Some(id) = state.active_transfer_tab_id()
                                                    && let Some(tab) = state.transfer_tab_mut(id)
                                                {
                                                    tab.insert_mode = InsertMode::Insert;
                                                    cx.notify();
                                                }
                                            });
                                        },
                                    ))
                                    .item(PopupMenuItem::new("Upsert").on_click(move |_, _, cx| {
                                        s2.update(cx, |state, cx| {
                                            if let Some(id) = state.active_transfer_tab_id()
                                                && let Some(tab) = state.transfer_tab_mut(id)
                                            {
                                                tab.insert_mode = InsertMode::Upsert;
                                                cx.notify();
                                            }
                                        });
                                    }))
                                    .item(
                                        PopupMenuItem::new("Replace").on_click(move |_, _, cx| {
                                            s3.update(cx, |state, cx| {
                                                if let Some(id) = state.active_transfer_tab_id()
                                                    && let Some(tab) = state.transfer_tab_mut(id)
                                                {
                                                    tab.insert_mode = InsertMode::Replace;
                                                    cx.notify();
                                                }
                                            });
                                        }),
                                    )
                                },
                            )
                    };

                    let drop_checkbox = {
                        let state = state.clone();
                        let checked = transfer_state.drop_before_import;
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(Checkbox::new(("drop-before", key)).checked(checked).on_click(
                                move |_, _, cx| {
                                    state.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.drop_before_import = !checked;
                                            cx.notify();
                                        }
                                    });
                                },
                            ))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_secondary())
                                    .child("Enabled"),
                            )
                    };

                    let stop_checkbox = {
                        let state = state.clone();
                        let checked = transfer_state.stop_on_error;
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(Checkbox::new(("stop-on-error", key)).checked(checked).on_click(
                                move |_, _, cx| {
                                    state.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.stop_on_error = !checked;
                                            cx.notify();
                                        }
                                    });
                                },
                            ))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_secondary())
                                    .child("Enabled"),
                            )
                    };

                    sections.push(
                        option_section(
                            "Input",
                            vec![
                                option_field_static("Detect format", "Auto"),
                                option_field_static("Encoding", "UTF-8"),
                                option_field_static("Preview rows", "Off"),
                            ],
                        )
                        .into_any_element(),
                    );

                    sections.push(
                        option_section(
                            "Insert",
                            vec![
                                option_field(
                                    "Insert mode",
                                    insert_mode_dropdown.into_any_element(),
                                ),
                                option_field_static(
                                    "Batch size",
                                    transfer_state.batch_size.to_string(),
                                ),
                                option_field(
                                    "Drop before import",
                                    drop_checkbox.into_any_element(),
                                ),
                                option_field("Stop on error", stop_checkbox.into_any_element()),
                            ],
                        )
                        .into_any_element(),
                    );

                    match transfer_state.format {
                        TransferFormat::Csv => {
                            sections.push(
                                option_section(
                                    "CSV Options",
                                    vec![
                                        option_field_static("Delimiter", ","),
                                        option_field_static("Header row", "On"),
                                        option_field_static("Columns have types", "Off"),
                                        option_field_static("Ignore empty strings", "Off"),
                                        option_field_static("Map columns", "Off"),
                                    ],
                                )
                                .into_any_element(),
                            );
                        }
                        _ => {
                            sections.push(
                                option_section(
                                    "JSON Options",
                                    vec![
                                        option_field_static("Extended JSON", "Relaxed/Canonical"),
                                        option_field_static("Allow shell syntax", "On"),
                                    ],
                                )
                                .into_any_element(),
                            );
                        }
                    }

                    if matches!(transfer_state.scope, TransferScope::Database) {
                        sections.push(
                            option_section(
                                "Database",
                                vec![
                                    option_field_static("Namespace mapping", "Off"),
                                    option_field_static("Restore indexes/options", "On"),
                                ],
                            )
                            .into_any_element(),
                        );
                    }
                }
                TransferMode::Copy => {
                    sections.push(
                        option_section(
                            "Copy Options",
                            vec![
                                option_field_static("Copy indexes", "On"),
                                option_field_static("Copy options", "On"),
                                option_field_static("Overwrite target", "Off"),
                                option_field_static(
                                    "Batch size",
                                    transfer_state.batch_size.to_string(),
                                ),
                                option_field_static("Ordered", "On"),
                            ],
                        )
                        .into_any_element(),
                    );

                    sections.push(
                        option_section(
                            "Filters",
                            vec![
                                option_field_static("Filter", "Optional"),
                                option_field_static("Projection", "Optional"),
                                option_field_static("Pipeline", "Optional"),
                            ],
                        )
                        .into_any_element(),
                    );

                    if matches!(transfer_state.scope, TransferScope::Database) {
                        sections.push(
                            option_section(
                                "Collections",
                                vec![
                                    option_field_static("Include collections", "All"),
                                    option_field_static("Exclude collections", "None"),
                                ],
                            )
                            .into_any_element(),
                        );
                    }
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

fn can_execute_transfer(state: &TransferTabState) -> bool {
    // Must have source connection and database
    if state.source_connection_id.is_none() || state.source_database.is_empty() {
        return false;
    }

    // For collection scope, must have collection
    if matches!(state.scope, TransferScope::Collection) && state.source_collection.is_empty() {
        return false;
    }

    // For export/import, must have file path
    if matches!(state.mode, TransferMode::Export | TransferMode::Import)
        && state.file_path.is_empty()
    {
        return false;
    }

    // For copy, must have destination connection
    if matches!(state.mode, TransferMode::Copy) && state.destination_connection_id.is_none() {
        return false;
    }

    true
}

fn render_side_panel(
    transfer_state: &TransferTabState,
    source_conn_name: &str,
    dest_conn_name: &str,
) -> impl IntoElement {
    let summary_panel = render_summary_panel(transfer_state, source_conn_name, dest_conn_name);
    let secondary_panel = match transfer_state.mode {
        TransferMode::Export => render_preview_panel(transfer_state).into_any_element(),
        TransferMode::Import => render_plan_panel(
            "Import plan",
            vec![
                "Choose a source file",
                "Select target database/collection",
                "Review options",
                "Run import",
            ],
        )
        .into_any_element(),
        TransferMode::Copy => render_plan_panel(
            "Copy plan",
            vec!["Select destination connection", "Review copy options", "Run copy"],
        )
        .into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .gap(spacing::md())
        .w(px(300.0))
        .min_w(px(240.0))
        .max_w(px(360.0))
        .child(summary_panel)
        .child(secondary_panel)
}

fn render_summary_panel(
    transfer_state: &TransferTabState,
    source_conn_name: &str,
    dest_conn_name: &str,
) -> impl IntoElement {
    let source_db = fallback_text(&transfer_state.source_database, "Select database");
    let source_coll = if matches!(transfer_state.scope, TransferScope::Collection) {
        format!(" / {}", fallback_text(&transfer_state.source_collection, "Select collection"))
    } else {
        String::new()
    };

    let target_db = if transfer_state.destination_database.is_empty() {
        transfer_state.source_database.clone()
    } else {
        transfer_state.destination_database.clone()
    };

    let target_coll = if transfer_state.destination_collection.is_empty() {
        transfer_state.source_collection.clone()
    } else {
        transfer_state.destination_collection.clone()
    };

    let destination = match transfer_state.mode {
        TransferMode::Export => fallback_text(&transfer_state.file_path, "Choose file"),
        TransferMode::Import => {
            let mut label =
                format!("{source_conn_name} / {}", fallback_text(&target_db, "Select database"));
            if matches!(transfer_state.scope, TransferScope::Collection) {
                label.push_str(&format!(" / {}", fallback_text(&target_coll, "Select collection")));
            }
            label
        }
        TransferMode::Copy => {
            let mut label =
                format!("{dest_conn_name} / {}", fallback_text(&target_db, "Select database"));
            if matches!(transfer_state.scope, TransferScope::Collection) {
                label.push_str(&format!(" / {}", fallback_text(&target_coll, "Select collection")));
            }
            label
        }
    };

    panel(
        "Summary",
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .child(summary_row("Mode", transfer_state.mode.label()))
            .child(summary_row("Scope", transfer_state.scope.label()))
            .child(summary_row("Source", format!("{source_conn_name} / {source_db}{source_coll}")))
            .child(summary_row("Destination", destination))
            .child(summary_row(
                "Format",
                if matches!(transfer_state.mode, TransferMode::Copy) {
                    "Live copy".to_string()
                } else {
                    transfer_state.format.label().to_string()
                },
            ))
            .child(summary_row("Batch size", transfer_state.batch_size.to_string())),
    )
}

fn render_plan_panel(title: &str, steps: Vec<&str>) -> impl IntoElement {
    panel(
        title,
        div().flex().flex_col().gap(spacing::xs()).children(steps.into_iter().map(|step| {
            div().text_sm().text_color(colors::text_secondary()).child(format!("• {step}"))
        })),
    )
}

fn render_preview_panel(transfer_state: &TransferTabState) -> impl IntoElement {
    let content = if transfer_state.preview_loading {
        div()
            .flex()
            .items_center()
            .justify_center()
            .h_full()
            .text_sm()
            .text_color(colors::text_muted())
            .child("Loading preview...")
            .into_any_element()
    } else if transfer_state.preview_docs.is_empty() {
        div()
            .flex()
            .items_center()
            .justify_center()
            .h_full()
            .text_sm()
            .text_color(colors::text_muted())
            .child("Select a collection to preview")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .children(transfer_state.preview_docs.iter().map(|doc| {
                // Truncate long documents for preview
                let display_doc =
                    if doc.len() > 500 { format!("{}...", &doc[..500]) } else { doc.clone() };
                div()
                    .p(spacing::sm())
                    .bg(colors::bg_header())
                    .border_1()
                    .border_color(colors::border_subtle())
                    .rounded(borders::radius_sm())
                    .text_xs()
                    .font_family("monospace")
                    .text_color(colors::text_secondary())
                    .overflow_x_hidden()
                    .child(display_doc)
            }))
            .into_any_element()
    };

    panel(
        "Preview",
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .child(div().text_xs().text_color(colors::text_muted()).child(format!(
                "{} doc{}",
                transfer_state.preview_docs.len(),
                if transfer_state.preview_docs.len() == 1 { "" } else { "s" }
            )))
            .child(
                div()
                    .flex_1()
                    .min_h(px(150.0))
                    .max_h(px(320.0))
                    .overflow_y_scrollbar()
                    .child(content),
            ),
    )
}

fn render_warnings(transfer_state: &TransferTabState) -> impl IntoElement {
    let mut warnings = Vec::new();

    // CSV warning
    if matches!(transfer_state.format, TransferFormat::Csv) {
        warnings.push("CSV export will lose BSON type fidelity (dates, ObjectIds, etc.)");
    }

    // BSON warning
    if matches!(transfer_state.format, TransferFormat::Bson) {
        warnings.push("BSON format requires mongodump/mongorestore tools installed");
    }

    if warnings.is_empty() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .children(warnings.into_iter().map(|warning| {
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .px(spacing::md())
                .py(spacing::sm())
                .bg(hsla(0.12, 0.9, 0.5, 0.1))
                .border_1()
                .border_color(hsla(0.12, 0.9, 0.5, 0.3))
                .rounded(borders::radius_sm())
                .child(Icon::new(IconName::Info).xsmall().text_color(hsla(0.12, 0.9, 0.5, 1.0)))
                .child(div().text_sm().text_color(hsla(0.12, 0.9, 0.5, 1.0)).child(warning))
        }))
        .into_any_element()
}

// Helper functions for building UI

fn panel(title: &str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .p(spacing::md())
        .bg(colors::bg_header())
        .border_1()
        .border_color(colors::border_subtle())
        .rounded(borders::radius_sm())
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::text_secondary())
                .child(title.to_string()),
        )
        .child(content)
}

/// Form field with label above control (vertical layout)
fn form_field(label: &str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(control)
}

/// Static form field with label above value (vertical layout)
fn form_field_static(label: &str, value: impl Into<String>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(value_box(value, false))
}

fn value_box(value: impl Into<String>, muted: bool) -> Div {
    div()
        .px(spacing::sm())
        .py(px(6.0))
        .bg(colors::bg_sidebar())
        .border_1()
        .border_color(colors::border_subtle())
        .rounded(borders::radius_sm())
        .text_sm()
        .text_color(if muted { colors::text_muted() } else { colors::text_primary() })
        .child(value.into())
}

fn option_value_pill(value: impl Into<String>) -> AnyElement {
    div()
        .px(spacing::sm())
        .py(px(4.0))
        .bg(colors::bg_sidebar())
        .border_1()
        .border_color(colors::border_subtle())
        .rounded(borders::radius_sm())
        .text_xs()
        .text_color(colors::text_secondary())
        .child(value.into())
        .into_any_element()
}

fn option_section(title: &str, rows: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .p(spacing::sm())
        .bg(colors::bg_header())
        .border_1()
        .border_color(colors::border_subtle())
        .rounded(borders::radius_sm())
        .min_w(px(220.0))
        .flex_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::text_muted())
                .child(title.to_string()),
        )
        .child(div().flex().flex_wrap().gap(spacing::md()).children(rows))
}

fn option_field(label: &str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .min_w(px(160.0))
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(control)
        .into_any_element()
}

fn option_field_static(label: &str, value: impl Into<String>) -> AnyElement {
    option_field(label, option_value_pill(value))
}

fn summary_row(label: &str, value: impl Into<String>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(
            div()
                .flex_1()
                .text_sm()
                .text_color(colors::text_secondary())
                .overflow_x_hidden()
                .text_ellipsis()
                .child(value.into()),
        )
        .into_any_element()
}

/// Returns the value if non-empty, otherwise returns the fallback.
fn fallback_text(value: &str, fallback: &str) -> String {
    if value.is_empty() { fallback.to_string() } else { value.to_string() }
}
