//! Transfer view for import, export, and copy operations.

use gpui::*;
use gpui_component::button::Button as MenuButton;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Position, RopeExt};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Icon, IconName, IndexPath, Sizable as _, Size};
use uuid::Uuid;

use crate::bson::{format_relaxed_json_compact, parse_value_from_relaxed_json};
use crate::components::Button;
use crate::components::file_picker::{
    FilePickerMode, filters_for_format, open_file_dialog_async, open_folder_dialog_async,
    unexpanded_export_filename_bson_for_scope, unexpanded_export_filename_for_scope,
};
use crate::connection::tools_available;
use crate::state::{
    AppCommands, AppState, BsonOutputFormat, CompressionMode, DATABASE_SCOPE_FILENAME_TEMPLATE,
    DEFAULT_FILENAME_TEMPLATE, Encoding, ExtendedJsonMode, InsertMode, TransferFormat,
    TransferMode, TransferScope, TransferTabState,
};
use crate::theme::{borders, colors, sizing, spacing};

/// Which query field is being edited in the modal
#[derive(Clone, Copy, PartialEq)]
enum QueryEditField {
    Filter,
    Projection,
    Sort,
}

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

    /// Open the query edit modal for the specified field
    fn open_query_modal(
        &mut self,
        field: QueryEditField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Get current value for this field
        let current_value = {
            let state_ref = self.state.read(cx);
            if let Some(id) = state_ref.active_transfer_tab_id()
                && let Some(tab) = state_ref.transfer_tab(id)
            {
                match field {
                    QueryEditField::Filter => tab.export_filter.clone(),
                    QueryEditField::Projection => tab.export_projection.clone(),
                    QueryEditField::Sort => tab.export_sort.clone(),
                }
            } else {
                String::new()
            }
        };

        // Create input state for modal textarea
        let input_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(current_value, window, cx);
            state
        });

        self.query_edit_modal = Some(field);
        self.query_edit_input = Some(input_state);
        cx.notify();
    }

    /// Save the query modal content and close
    fn save_query_modal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.query_edit_modal else {
            return;
        };
        let Some(ref input_state) = self.query_edit_input else {
            return;
        };

        let new_value = input_state.read(cx).value().to_string();

        self.state.update(cx, |state, cx| {
            if let Some(id) = state.active_transfer_tab_id()
                && let Some(tab) = state.transfer_tab_mut(id)
            {
                match field {
                    QueryEditField::Filter => tab.export_filter = new_value,
                    QueryEditField::Projection => tab.export_projection = new_value,
                    QueryEditField::Sort => tab.export_sort = new_value,
                }
                cx.notify();
            }
        });

        // Close modal
        self.query_edit_modal = None;
        self.query_edit_input = None;
        cx.notify();
    }

    /// Close the query modal without saving
    fn close_query_modal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.query_edit_modal = None;
        self.query_edit_input = None;
        cx.notify();
    }

    /// Format the JSON in the modal textarea (compact, single-line since Input doesn't support newlines)
    fn format_query_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref input_state) = self.query_edit_input else {
            return;
        };

        let current_text = input_state.read(cx).value().to_string();
        if current_text.is_empty() {
            return;
        }

        // Try to parse using relaxed JSON parser, then output as compact relaxed JSON
        // (Input component doesn't support newlines, so we use single-line format)
        if let Ok(value) = parse_value_from_relaxed_json(&current_text) {
            let formatted = format_relaxed_json_compact(&value);
            input_state.update(cx, |state, cx| {
                state.set_value(formatted, window, cx);
            });
        }
    }

    /// Clear the JSON in the modal textarea
    fn clear_query_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref input_state) = self.query_edit_input {
            input_state.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
        }
    }

    /// Render the query edit modal (returns empty if not open)
    fn render_query_edit_modal(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(field) = self.query_edit_modal else {
            return div().into_any_element();
        };

        let Some(ref input_state) = self.query_edit_input else {
            return div().into_any_element();
        };

        let title = match field {
            QueryEditField::Filter => "Edit Filter",
            QueryEditField::Projection => "Edit Projection",
            QueryEditField::Sort => "Edit Sort",
        };

        let current_text = input_state.read(cx).value().to_string();
        let is_valid =
            current_text.is_empty() || parse_value_from_relaxed_json(&current_text).is_ok();

        let view = cx.entity();
        let view_save = view.clone();
        let view_cancel = view.clone();
        let view_format = view.clone();
        let view_clear = view.clone();

        // Modal overlay
        div()
            .absolute()
            .inset_0()
            .bg(hsla(0.0, 0.0, 0.0, 0.5)) // Semi-transparent backdrop
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(500.0))
                    .max_h(px(400.0))
                    .bg(colors::bg_sidebar())
                    .rounded(borders::radius_sm())
                    .border_1()
                    .border_color(colors::border())
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(spacing::md())
                            .py(spacing::sm())
                            .border_b_1()
                            .border_color(colors::border_subtle())
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(colors::text_primary())
                                    .child(title),
                            )
                            .child(
                                Button::new("modal-close")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Close)
                                    .on_click(move |_, window, cx| {
                                        view_cancel.update(cx, |view, cx| {
                                            view.close_query_modal(window, cx);
                                        });
                                    }),
                            ),
                    )
                    // Body - textarea
                    .child(
                        div()
                            .flex_1()
                            .p(spacing::md())
                            .min_h(px(200.0))
                            .child(Input::new(input_state).h_full().w_full()),
                    )
                    // Validation status
                    .child(
                        div()
                            .px(spacing::md())
                            .pb(spacing::sm())
                            .text_sm()
                            .text_color(if is_valid {
                                hsla(0.33, 0.7, 0.5, 1.0) // Green
                            } else {
                                hsla(0.0, 0.7, 0.5, 1.0) // Red
                            })
                            .child(if is_valid { "✓ Valid JSON" } else { "✗ Invalid JSON" }),
                    )
                    // Footer buttons
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(spacing::sm())
                            .px(spacing::md())
                            .py(spacing::sm())
                            .border_t_1()
                            .border_color(colors::border_subtle())
                            .child(
                                Button::new("modal-format")
                                    .ghost()
                                    .compact()
                                    .label("Format")
                                    .on_click(move |_, window, cx| {
                                        view_format.update(cx, |view, cx| {
                                            view.format_query_modal(window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("modal-clear")
                                    .ghost()
                                    .compact()
                                    .label("Clear")
                                    .on_click(move |_, window, cx| {
                                        view_clear.update(cx, |view, cx| {
                                            view.clear_query_modal(window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("modal-cancel")
                                    .ghost()
                                    .compact()
                                    .label("Cancel")
                                    .on_click(move |_, window, cx| {
                                        view.update(cx, |view, cx| {
                                            view.close_query_modal(window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("modal-save")
                                    .primary()
                                    .compact()
                                    .label("Save")
                                    .disabled(!is_valid)
                                    .on_click(move |_, window, cx| {
                                        view_save.update(cx, |view, cx| {
                                            view.save_query_modal(window, cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .into_any_element()
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

        // Create exclude collections select state (searchable multi-select behavior)
        let exclude_coll_state = cx.new(|cx| {
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
                    let conn_id = state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.source_database = db_str.clone();
                            tab.source_collection.clear();
                            cx.notify();
                            return tab.source_connection_id;
                        }
                        None
                    });
                    // Clear collection select
                    if let Some(ref coll_state) = view.source_coll_state {
                        coll_state.update(cx, |s, cx| {
                            s.set_selected_index(None, window, cx);
                        });
                    }
                    // Load collections for the selected database
                    if let Some(conn_id) = conn_id {
                        AppCommands::load_collections(state_clone.clone(), conn_id, db_str, cx);
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

        // Subscribe to exclude collections select - append to exclude list (multi-select behavior)
        let state_clone = state.clone();
        let exclude_coll_state_clone = exclude_coll_state.clone();
        let sub6 = cx.subscribe_in(
            &exclude_coll_state,
            window,
            move |_view,
                  _select_state,
                  event: &SelectEvent<SearchableVec<SharedString>>,
                  window,
                  cx| {
                if let SelectEvent::Confirm(Some(coll_name)) = event {
                    let coll_str = coll_name.to_string();
                    state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            // Only add if not already excluded
                            if !tab.exclude_collections.contains(&coll_str) {
                                tab.exclude_collections.push(coll_str);
                            }
                            cx.notify();
                        }
                    });
                    // Clear selection after pick (multi-select behavior)
                    exclude_coll_state_clone.update(cx, |s, cx| {
                        s.set_selected_index(None, window, cx);
                    });
                }
            },
        );

        // Create export path input state
        let current_file_path = {
            let state_ref = state.read(cx);
            state_ref
                .active_transfer_tab_id()
                .and_then(|id| state_ref.transfer_tab(id).map(|tab| tab.file_path.clone()))
                .unwrap_or_default()
        };

        let export_path_input_state = cx.new(|cx| {
            let mut input_state =
                InputState::new(window, cx).placeholder("Select folder or enter path...");
            input_state.set_value(current_file_path, window, cx);
            input_state
        });

        // Subscribe to export path input changes
        let state_clone = state.clone();
        let sub5 = cx.subscribe_in(
            &export_path_input_state,
            window,
            move |_view, input_state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let new_path = input_state.read(cx).value().to_string();
                    state_clone.update(cx, |state, cx| {
                        if let Some(tab_id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(tab_id)
                        {
                            tab.file_path = new_path;
                            cx.notify();
                        }
                    });
                }
            },
        );

        self._select_subscriptions = vec![sub1, sub2, sub3, sub4, sub5, sub6];
        self.source_conn_state = Some(source_conn_state);
        self.source_db_state = Some(source_db_state);
        self.source_coll_state = Some(source_coll_state);
        self.dest_conn_state = Some(dest_conn_state);
        self.exclude_coll_state = Some(exclude_coll_state);
        self.export_path_input_state = Some(export_path_input_state);
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
                    .child(summary_panel),
            )
            .child(modal_overlay)
            .into_any_element()
    }
}

impl TransferView {
    fn render_source_panel(
        &self,
        transfer_state: &TransferTabState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_collection = matches!(transfer_state.scope, TransferScope::Collection);
        let show_query_options =
            matches!(transfer_state.mode, TransferMode::Export) && show_collection;

        // Searchable select components (states are initialized by ensure_select_states)
        let Some(ref source_conn_state) = self.source_conn_state else {
            return panel("Source", div().child("Loading...")).into_any_element();
        };
        let Some(ref source_db_state) = self.source_db_state else {
            return panel("Source", div().child("Loading...")).into_any_element();
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

        // Query options rows (Filter, Projection, Sort)
        let view = cx.entity();
        let state = self.state.clone();
        let query_options = if show_query_options {
            vec![
                render_query_field_row(
                    "Filter",
                    QueryEditField::Filter,
                    &transfer_state.export_filter,
                    view.clone(),
                    state.clone(),
                )
                .into_any_element(),
                render_query_field_row(
                    "Projection",
                    QueryEditField::Projection,
                    &transfer_state.export_projection,
                    view.clone(),
                    state.clone(),
                )
                .into_any_element(),
                render_query_field_row(
                    "Sort",
                    QueryEditField::Sort,
                    &transfer_state.export_sort,
                    view,
                    state,
                )
                .into_any_element(),
            ]
        } else {
            vec![]
        };

        panel(
            "Source",
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .child(form_row("Connection", conn_select))
                .child(form_row("Database", db_select))
                .children(coll_select.map(|s| form_row("Collection", s)))
                .children(query_options),
        )
        .into_any_element()
    }

    fn render_destination_panel(
        &self,
        transfer_state: &TransferTabState,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let state = self.state.clone();
        let settings = self.state.read(cx).settings.clone();

        match transfer_state.mode {
            TransferMode::Export => {
                // All export formats use folder picker + editable input with placeholders
                let Some(ref export_path_input_state) = self.export_path_input_state else {
                    return panel("Destination", div().child("Loading...")).into_any_element();
                };

                // Sync input state with transfer state file_path (when changed externally)
                let input_value = export_path_input_state.read(cx).value().to_string();
                if input_value != transfer_state.file_path {
                    export_path_input_state.update(cx, |input_state, cx| {
                        input_state.set_value(transfer_state.file_path.clone(), window, cx);
                    });
                }

                let is_bson = matches!(transfer_state.format, TransferFormat::Bson);
                let is_bson_folder =
                    is_bson && matches!(transfer_state.bson_output, BsonOutputFormat::Folder);

                // Determine the label based on format and output mode
                let dest_label = if is_bson_folder {
                    "Output Folder"
                } else if is_bson {
                    "Archive File"
                } else {
                    "File"
                };

                // Folder browse button - opens folder picker, then appends template filename
                let browse_button = {
                    let state = state.clone();
                    let format = transfer_state.format;
                    let bson_output = transfer_state.bson_output;
                    let scope = transfer_state.scope;
                    let settings = settings.clone();
                    Button::new("browse-export-folder").compact().icon(IconName::Folder).on_click(
                        move |_, _, cx| {
                            let state = state.clone();
                            let settings = settings.clone();
                            cx.spawn(async move |cx| {
                                if let Some(folder_path) = open_folder_dialog_async().await {
                                    cx.update(|cx| {
                                        // Generate unexpanded filename from template
                                        // Use scope-aware function for appropriate template
                                        let filename = if matches!(format, TransferFormat::Bson) {
                                            unexpanded_export_filename_bson_for_scope(
                                                &settings,
                                                bson_output,
                                                scope,
                                            )
                                        } else {
                                            unexpanded_export_filename_for_scope(
                                                &settings, format, scope,
                                            )
                                        };
                                        let full_path =
                                            folder_path.join(&filename).display().to_string();

                                        // Update tab state (InputState synced in render)
                                        state.update(cx, |state, cx| {
                                            if let Some(tab_id) = state.active_transfer_tab_id()
                                                && let Some(tab) = state.transfer_tab_mut(tab_id)
                                            {
                                                tab.file_path = full_path;
                                                cx.notify();
                                            }
                                        });
                                    })
                                    .ok();
                                }
                            })
                            .detach();
                        },
                    )
                };

                // Placeholder dropdown button - insert placeholders at cursor
                let placeholder_button = {
                    let state = state.clone();
                    let export_path_input = export_path_input_state.clone();
                    let format = transfer_state.format;
                    let bson_output = transfer_state.bson_output;
                    let scope = transfer_state.scope;
                    MenuButton::new("placeholder-dropdown")
                        .compact()
                        .label("${}")
                        .rounded(borders::radius_sm())
                        .with_size(Size::XSmall)
                        .dropdown_menu_with_anchor(
                            Corner::BottomLeft,
                            move |mut menu, _window, _cx| {
                                // Only show user-insertable placeholders (date/time variants)
                                let insertable_placeholders = [
                                    ("${datetime}", "Date and time"),
                                    ("${date}", "Date only"),
                                    ("${time}", "Time only"),
                                ];

                                for (placeholder, description) in insertable_placeholders {
                                    let p = placeholder.to_string();
                                    let export_path_input = export_path_input.clone();
                                    menu =
                                        menu.item(
                                            PopupMenuItem::new(format!(
                                                "{} - {}",
                                                placeholder, description
                                            ))
                                            .on_click(move |_, window, cx| {
                                                // Insert placeholder at cursor position
                                                export_path_input.update(cx, |input_state, cx| {
                                                    let cursor = input_state.cursor();
                                                    let text = input_state.value().to_string();
                                                    let mut new_text =
                                                        String::with_capacity(text.len() + p.len());
                                                    new_text.push_str(&text[..cursor]);
                                                    new_text.push_str(&p);
                                                    new_text.push_str(&text[cursor..]);
                                                    input_state.set_value(new_text, window, cx);
                                                    // Move cursor to after inserted text
                                                    let new_cursor_offset = cursor + p.len();
                                                    let position = input_state
                                                        .text()
                                                        .offset_to_position(new_cursor_offset);
                                                    input_state.set_cursor_position(
                                                        Position::new(
                                                            position.line,
                                                            position.character,
                                                        ),
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }),
                                        );
                                }

                                // Add reset option
                                let state = state.clone();
                                let export_path_input = export_path_input.clone();
                                menu = menu.separator().item(
                                    PopupMenuItem::new("Reset to default").on_click(
                                        move |_, window, cx| {
                                            // Get current folder from path (if any)
                                            let current_path =
                                                export_path_input.read(cx).value().to_string();
                                            let folder = std::path::Path::new(&current_path)
                                                .parent()
                                                .map(|p| p.to_path_buf());

                                            // Generate default filename based on format and scope
                                            let default_filename = match scope {
                                                TransferScope::Database => {
                                                    // Database scope: path is a directory
                                                    if matches!(format, TransferFormat::Bson)
                                                        && matches!(
                                                            bson_output,
                                                            BsonOutputFormat::Archive
                                                        )
                                                    {
                                                        // BSON Archive is a file
                                                        format!(
                                                            "{}.archive",
                                                            DATABASE_SCOPE_FILENAME_TEMPLATE
                                                        )
                                                    } else {
                                                        // JSON/CSV/BSON Folder: directory, no extension
                                                        DATABASE_SCOPE_FILENAME_TEMPLATE.to_string()
                                                    }
                                                }
                                                TransferScope::Collection => {
                                                    // Collection scope: path is a file
                                                    if matches!(format, TransferFormat::Bson) {
                                                        match bson_output {
                                                            BsonOutputFormat::Archive => {
                                                                format!(
                                                                    "{}.archive",
                                                                    DEFAULT_FILENAME_TEMPLATE
                                                                )
                                                            }
                                                            BsonOutputFormat::Folder => {
                                                                DEFAULT_FILENAME_TEMPLATE
                                                                    .to_string()
                                                            }
                                                        }
                                                    } else {
                                                        format!(
                                                            "{}.{}",
                                                            DEFAULT_FILENAME_TEMPLATE,
                                                            format.extension()
                                                        )
                                                    }
                                                }
                                            };

                                            // Combine folder + default filename
                                            let new_path = if let Some(folder) = folder {
                                                folder.join(&default_filename).display().to_string()
                                            } else {
                                                default_filename
                                            };

                                            // Update input state
                                            export_path_input.update(cx, |input_state, cx| {
                                                input_state.set_value(new_path.clone(), window, cx);
                                            });

                                            // Update tab state
                                            state.update(cx, |state, cx| {
                                                if let Some(tab_id) = state.active_transfer_tab_id()
                                                    && let Some(tab) =
                                                        state.transfer_tab_mut(tab_id)
                                                {
                                                    tab.file_path = new_path;
                                                    cx.notify();
                                                }
                                            });
                                        },
                                    ),
                                );
                                menu
                            },
                        )
                };

                // Editable input for file path
                let file_input = Input::new(export_path_input_state).small().flex_1();

                let file_control = div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(file_input)
                    .child(placeholder_button)
                    .child(browse_button);

                panel(
                    "Destination",
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::md())
                        .child(form_row(dest_label, file_control)),
                )
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
                            let state = state.clone();
                            cx.spawn(async move |cx| {
                                if let Some(path) =
                                    open_file_dialog_async(FilePickerMode::Open, filters, None)
                                        .await
                                {
                                    cx.update(|cx| {
                                        state.update(cx, |state, cx| {
                                            if let Some(tab_id) = state.active_transfer_tab_id()
                                                && let Some(tab) = state.transfer_tab_mut(tab_id)
                                            {
                                                // Auto-detect format
                                                if let Some(ext) =
                                                    path.extension().and_then(|e| e.to_str())
                                                {
                                                    tab.format = match ext {
                                                        "jsonl" | "ndjson" => {
                                                            TransferFormat::JsonLines
                                                        }
                                                        "json" => TransferFormat::JsonArray,
                                                        "csv" => TransferFormat::Csv,
                                                        "archive" | "bson" => TransferFormat::Bson,
                                                        _ => tab.format,
                                                    };
                                                }
                                                tab.file_path = path.display().to_string();
                                                cx.notify();
                                            }
                                        });
                                    })
                                    .ok();
                                }
                            })
                            .detach();
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
                        .child(form_row("File", file_control))
                        .child(form_row_static("Target database", target_db))
                        .children(
                            show_coll.then(|| form_row_static("Target collection", target_coll)),
                        ),
                )
                .into_any_element()
            }
            TransferMode::Copy => {
                // Searchable select for destination connection
                let Some(ref dest_conn_state) = self.dest_conn_state else {
                    return panel("Destination", div().child("Loading...")).into_any_element();
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
                        .child(form_row("Connection", conn_select))
                        .child(form_row_static("Database", target_db))
                        .children(show_coll.then(|| form_row_static("Collection", target_coll))),
                )
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
                    // Format-specific options
                    match transfer_state.format {
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
                        TransferFormat::Csv => {
                            // CSV export - no options (removed)
                        }
                        _ => {
                            // JSON Options - Extended JSON dropdown + Pretty print only
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
                                checkbox_field(("pretty-print", key), checked, move |cx| {
                                    state.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.pretty_print = !checked;
                                            cx.notify();
                                        }
                                    });
                                })
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
                                    ],
                                )
                                .into_any_element(),
                            );
                        }
                    }

                    // Database scope options (only for BSON format - indexes can't be stored in JSON/CSV)
                    if matches!(transfer_state.scope, TransferScope::Database)
                        && matches!(transfer_state.format, TransferFormat::Bson)
                    {
                        let include_indexes_checkbox = {
                            let state = state.clone();
                            let checked = transfer_state.include_indexes;
                            checkbox_field(("include-indexes-export", key), checked, move |cx| {
                                state.update(cx, |state, cx| {
                                    if let Some(id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(id)
                                    {
                                        tab.include_indexes = !checked;
                                        cx.notify();
                                    }
                                });
                            })
                        };

                        sections.push(
                            option_section(
                                "Database",
                                vec![option_field(
                                    "Include indexes",
                                    include_indexes_checkbox.into_any_element(),
                                )],
                            )
                            .into_any_element(),
                        );

                        // Collection Filter section
                        let exclude_select =
                            if let Some(ref exclude_state) = self.exclude_coll_state {
                                Select::new(exclude_state)
                                    .small()
                                    .w_full()
                                    .placeholder("Search collections to exclude...")
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            };

                        // Render tags for excluded collections
                        let excluded_tags = {
                            let state = state.clone();
                            div().flex().flex_wrap().gap(spacing::xs()).mt(spacing::xs()).children(
                                transfer_state.exclude_collections.iter().enumerate().map(
                                    |(idx, coll)| {
                                        let coll_name = coll.clone();
                                        let state = state.clone();

                                        div()
                                            .id(("exclude-tag", idx))
                                            .flex()
                                            .items_center()
                                            .gap(spacing::xs())
                                            .px(spacing::sm())
                                            .py_1()
                                            .rounded(borders::radius_sm())
                                            .bg(colors::bg_header())
                                            .border_1()
                                            .border_color(colors::border_subtle())
                                            .text_sm()
                                            .child(coll.clone())
                                            .child(
                                                div()
                                                    .id(("exclude-tag-remove", idx))
                                                    .cursor_pointer()
                                                    .text_color(colors::text_muted())
                                                    .hover(|s| s.text_color(colors::text_primary()))
                                                    .on_click(move |_, _, cx| {
                                                        let coll_name = coll_name.clone();
                                                        state.update(cx, |state, cx| {
                                                            if let Some(id) =
                                                                state.active_transfer_tab_id()
                                                                && let Some(tab) =
                                                                    state.transfer_tab_mut(id)
                                                            {
                                                                tab.exclude_collections
                                                                    .retain(|c| c != &coll_name);
                                                                cx.notify();
                                                            }
                                                        });
                                                    })
                                                    .child("×"),
                                            )
                                    },
                                ),
                            )
                        };

                        sections.push(
                            option_section(
                                "Collection Filter",
                                vec![
                                    option_field("Exclude", exclude_select),
                                    excluded_tags.into_any_element(),
                                ],
                            )
                            .into_any_element(),
                        );
                    }
                }
                TransferMode::Import => {
                    // Input section
                    let detect_format_checkbox = {
                        let state = state.clone();
                        let checked = transfer_state.detect_format;
                        checkbox_field(("detect-format", key), checked, move |cx| {
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.detect_format = !checked;
                                    cx.notify();
                                }
                            });
                        })
                    };

                    let encoding_dropdown = {
                        let state = state.clone();
                        MenuButton::new(("encoding", key))
                            .compact()
                            .label(transfer_state.encoding.label())
                            .dropdown_caret(true)
                            .rounded(borders::radius_sm())
                            .with_size(Size::XSmall)
                            .dropdown_menu_with_anchor(
                                Corner::BottomLeft,
                                move |menu, _window, _cx| {
                                    let s1 = state.clone();
                                    let s2 = state.clone();
                                    menu.item(PopupMenuItem::new("UTF-8").on_click(
                                        move |_, _, cx| {
                                            s1.update(cx, |state, cx| {
                                                if let Some(id) = state.active_transfer_tab_id()
                                                    && let Some(tab) = state.transfer_tab_mut(id)
                                                {
                                                    tab.encoding = Encoding::Utf8;
                                                    cx.notify();
                                                }
                                            });
                                        },
                                    ))
                                    .item(
                                        PopupMenuItem::new("Latin-1").on_click(move |_, _, cx| {
                                            s2.update(cx, |state, cx| {
                                                if let Some(id) = state.active_transfer_tab_id()
                                                    && let Some(tab) = state.transfer_tab_mut(id)
                                                {
                                                    tab.encoding = Encoding::Latin1;
                                                    cx.notify();
                                                }
                                            });
                                        }),
                                    )
                                },
                            )
                    };

                    sections.push(
                        option_section(
                            "Input",
                            vec![
                                option_field(
                                    "Detect format",
                                    detect_format_checkbox.into_any_element(),
                                ),
                                option_field("Encoding", encoding_dropdown.into_any_element()),
                            ],
                        )
                        .into_any_element(),
                    );

                    // Insert section
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
                        checkbox_field(("drop-before", key), checked, move |cx| {
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.drop_before_import = !checked;
                                    cx.notify();
                                }
                            });
                        })
                    };

                    let clear_checkbox = {
                        let state = state.clone();
                        let checked = transfer_state.clear_before_import;
                        checkbox_field(("clear-before", key), checked, move |cx| {
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.clear_before_import = !checked;
                                    cx.notify();
                                }
                            });
                        })
                    };

                    let stop_checkbox = {
                        let state = state.clone();
                        let checked = transfer_state.stop_on_error;
                        checkbox_field(("stop-on-error", key), checked, move |cx| {
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.stop_on_error = !checked;
                                    cx.notify();
                                }
                            });
                        })
                    };

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
                                option_field(
                                    "Clear before import",
                                    clear_checkbox.into_any_element(),
                                ),
                                option_field("Stop on error", stop_checkbox.into_any_element()),
                            ],
                        )
                        .into_any_element(),
                    );

                    // Database scope options (only for BSON format - indexes can't be stored in JSON/CSV)
                    if matches!(transfer_state.scope, TransferScope::Database)
                        && matches!(transfer_state.format, TransferFormat::Bson)
                    {
                        let restore_indexes_checkbox = {
                            let state = state.clone();
                            let checked = transfer_state.restore_indexes;
                            checkbox_field(("restore-indexes", key), checked, move |cx| {
                                state.update(cx, |state, cx| {
                                    if let Some(id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(id)
                                    {
                                        tab.restore_indexes = !checked;
                                        cx.notify();
                                    }
                                });
                            })
                        };

                        sections.push(
                            option_section(
                                "Database",
                                vec![option_field(
                                    "Restore indexes",
                                    restore_indexes_checkbox.into_any_element(),
                                )],
                            )
                            .into_any_element(),
                        );
                    }
                }
                TransferMode::Copy => {
                    // Copy Options - only show implemented options (copy_indexes, batch_size)
                    let copy_indexes_checkbox = {
                        let state = state.clone();
                        let checked = transfer_state.copy_indexes;
                        checkbox_field(("copy-indexes", key), checked, move |cx| {
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.copy_indexes = !checked;
                                    cx.notify();
                                }
                            });
                        })
                    };

                    sections.push(
                        option_section(
                            "Copy Options",
                            vec![
                                option_field(
                                    "Copy indexes",
                                    copy_indexes_checkbox.into_any_element(),
                                ),
                                option_field_static(
                                    "Batch size",
                                    transfer_state.batch_size.to_string(),
                                ),
                            ],
                        )
                        .into_any_element(),
                    );

                    // Collection Filter section (for Copy mode + Database scope)
                    if matches!(transfer_state.scope, TransferScope::Database) {
                        let exclude_select =
                            if let Some(ref exclude_state) = self.exclude_coll_state {
                                Select::new(exclude_state)
                                    .small()
                                    .w_full()
                                    .placeholder("Search collections to exclude...")
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            };

                        // Render tags for excluded collections
                        let excluded_tags = {
                            let state = state.clone();
                            div().flex().flex_wrap().gap(spacing::xs()).mt(spacing::xs()).children(
                                transfer_state.exclude_collections.iter().enumerate().map(
                                    |(idx, coll)| {
                                        let coll_name = coll.clone();
                                        let state = state.clone();

                                        div()
                                            .id(("exclude-tag-copy", idx))
                                            .flex()
                                            .items_center()
                                            .gap(spacing::xs())
                                            .px(spacing::sm())
                                            .py_1()
                                            .rounded(borders::radius_sm())
                                            .bg(colors::bg_header())
                                            .border_1()
                                            .border_color(colors::border_subtle())
                                            .text_sm()
                                            .child(coll.clone())
                                            .child(
                                                div()
                                                    .id(("exclude-tag-copy-remove", idx))
                                                    .cursor_pointer()
                                                    .text_color(colors::text_muted())
                                                    .hover(|s| s.text_color(colors::text_primary()))
                                                    .on_click(move |_, _, cx| {
                                                        let coll_name = coll_name.clone();
                                                        state.update(cx, |state, cx| {
                                                            if let Some(id) =
                                                                state.active_transfer_tab_id()
                                                                && let Some(tab) =
                                                                    state.transfer_tab_mut(id)
                                                            {
                                                                tab.exclude_collections
                                                                    .retain(|c| c != &coll_name);
                                                                cx.notify();
                                                            }
                                                        });
                                                    })
                                                    .child("×"),
                                            )
                                    },
                                ),
                            )
                        };

                        sections.push(
                            option_section(
                                "Collection Filter",
                                vec![
                                    option_field("Exclude", exclude_select),
                                    excluded_tags.into_any_element(),
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

fn render_summary_panel(
    transfer_state: &TransferTabState,
    _source_conn_name: &str,
    dest_conn_name: &str,
) -> impl IntoElement {
    let source_db = fallback_text(&transfer_state.source_database, "...");
    let source_coll = if matches!(transfer_state.scope, TransferScope::Collection) {
        format!(".{}", fallback_text(&transfer_state.source_collection, "..."))
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

    let source_desc = format!("{source_db}{source_coll}");

    let dest_desc = match transfer_state.mode {
        TransferMode::Export => {
            let is_bson_folder = matches!(transfer_state.format, TransferFormat::Bson)
                && matches!(transfer_state.bson_output, BsonOutputFormat::Folder);

            if transfer_state.file_path.is_empty() {
                if is_bson_folder {
                    "Choose folder...".to_string()
                } else {
                    "Choose file...".to_string()
                }
            } else {
                std::path::Path::new(&transfer_state.file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| transfer_state.file_path.clone())
            }
        }
        TransferMode::Import => {
            let mut label = fallback_text(&target_db, "...");
            if matches!(transfer_state.scope, TransferScope::Collection) {
                label.push_str(&format!(".{}", fallback_text(&target_coll, "...")));
            }
            label
        }
        TransferMode::Copy => {
            let conn = if dest_conn_name == "Select connection" { "..." } else { dest_conn_name };
            let mut label = format!("{conn}:{}", fallback_text(&target_db, "..."));
            if matches!(transfer_state.scope, TransferScope::Collection) {
                label.push_str(&format!(".{}", fallback_text(&target_coll, "...")));
            }
            label
        }
    };

    let format_label = match (transfer_state.mode, transfer_state.format) {
        (TransferMode::Copy, _) => "Live copy".to_string(),
        (_, TransferFormat::Bson) => {
            // Include BSON output type
            format!("BSON {}", transfer_state.bson_output.label())
        }
        _ => transfer_state.format.label().to_string(),
    };

    // Add compression indicator if enabled
    let format_label = match transfer_state.compression {
        CompressionMode::Gzip => format!("{format_label} (gzip)"),
        CompressionMode::None => format_label,
    };

    // Compact horizontal summary
    div()
        .flex()
        .items_center()
        .justify_between()
        .p(spacing::md())
        .bg(colors::bg_sidebar())
        .border_1()
        .border_color(colors::border())
        .rounded(borders::radius_sm())
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::lg())
                .child(summary_item("From", source_desc))
                .child(Icon::new(IconName::ArrowRight).xsmall().text_color(colors::text_muted()))
                .child(summary_item("To", dest_desc))
                .child(summary_item("Format", format_label)),
        )
}

fn render_warnings(transfer_state: &TransferTabState) -> impl IntoElement {
    let mut warnings = Vec::new();

    // Only show format warnings for Export/Import modes (not Copy)
    if matches!(transfer_state.mode, TransferMode::Export | TransferMode::Import) {
        // CSV warning
        if matches!(transfer_state.format, TransferFormat::Csv) {
            warnings.push("CSV export will lose BSON type fidelity (dates, ObjectIds, etc.)");
        }

        // BSON warning - only show if tools are NOT available
        if matches!(transfer_state.format, TransferFormat::Bson) && !tools_available() {
            warnings.push("BSON format requires mongodump/mongorestore. Run: just download-tools");
        }
    }

    if warnings.is_empty() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .mb(spacing::md())
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

/// Form row with horizontal label + control for cleaner alignment
fn form_row(label: &str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .child(
            div()
                .w(px(100.0)) // Fixed label width for alignment
                .text_sm()
                .text_color(colors::text_muted())
                .child(label.to_string()),
        )
        .child(div().flex_1().max_w(px(400.0)).child(control))
}

/// Static form row with horizontal label + value
fn form_row_static(label: &str, value: impl Into<String>) -> impl IntoElement {
    form_row(label, value_box(value, false))
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

/// Creates a checkbox field with "Enabled" label
fn checkbox_field<F>(id: impl Into<ElementId>, checked: bool, on_click: F) -> Div
where
    F: Fn(&mut App) + 'static,
{
    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(Checkbox::new(id).checked(checked).on_click(move |_, _, cx| on_click(cx)))
        .child(div().text_sm().text_color(colors::text_secondary()).child("Enabled"))
}

/// Compact summary item for horizontal summary bar
fn summary_item(label: &str, value: impl Into<String>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(
            div()
                .text_sm()
                .text_color(colors::text_secondary())
                .overflow_x_hidden()
                .text_ellipsis()
                .child(value.into()),
        )
}

/// Returns the value if non-empty, otherwise returns the fallback.
fn fallback_text(value: &str, fallback: &str) -> String {
    if value.is_empty() { fallback.to_string() } else { value.to_string() }
}

/// Render a read-only query field row with Edit and Clear buttons
fn render_query_field_row(
    label: &str,
    field: QueryEditField,
    value: &str,
    view: Entity<TransferView>,
    state: Entity<AppState>,
) -> impl IntoElement {
    // Display text: truncated JSON or "(none)"
    let display_text = if value.is_empty() {
        "(none)".to_string()
    } else {
        // Truncate if longer than ~40 chars
        if value.len() > 40 { format!("{}...", &value[..37]) } else { value.to_string() }
    };

    let is_empty = value.is_empty();

    let value_box = div()
        .flex_1()
        .px(spacing::sm())
        .py_1()
        .rounded(borders::radius_sm())
        .bg(colors::bg_app())
        .border_1()
        .border_color(colors::border())
        .text_sm()
        .text_color(if is_empty { colors::text_muted() } else { colors::text_primary() })
        .overflow_hidden()
        .text_ellipsis()
        .child(display_text);

    let edit_button = Button::new(("edit-query", field as usize)).compact().label("Edit").on_click(
        move |_, window, cx| {
            view.update(cx, |view, cx| {
                view.open_query_modal(field, window, cx);
            });
        },
    );

    // Clear button - only shown when field has a value
    let clear_button = if !is_empty {
        Some(
            Button::new(("clear-query", field as usize))
                .ghost()
                .compact()
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    state.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            match field {
                                QueryEditField::Filter => tab.export_filter.clear(),
                                QueryEditField::Projection => tab.export_projection.clear(),
                                QueryEditField::Sort => tab.export_sort.clear(),
                            }
                            cx.notify();
                        }
                    });
                }),
        )
    } else {
        None
    };

    form_row(
        label,
        div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .child(value_box)
            .child(edit_button)
            .children(clear_button),
    )
}
