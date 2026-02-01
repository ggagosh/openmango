//! Destination panel for transfer view (Export, Import, Copy modes).

use gpui::*;
use gpui_component::button::Button as MenuButton;
use gpui_component::input::{Input, Position, RopeExt};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::select::Select;
use gpui_component::{IconName, Sizable as _, Size};

use crate::components::Button;
use crate::components::file_picker::{
    FilePickerMode, filters_for_format, open_file_dialog_async, open_folder_dialog_async,
    unexpanded_export_filename_bson_for_scope, unexpanded_export_filename_for_scope,
};
use crate::state::{
    AppSettings, AppState, BsonOutputFormat, DATABASE_SCOPE_FILENAME_TEMPLATE,
    DEFAULT_FILENAME_TEMPLATE, TransferFormat, TransferMode, TransferScope, TransferTabState,
};
use crate::theme::{borders, spacing};

use super::TransferView;
use super::helpers::{form_row, form_row_static, panel, value_box};

impl TransferView {
    /// Render the destination panel based on the transfer mode.
    pub(super) fn render_destination_panel(
        &self,
        transfer_state: &TransferTabState,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let state = self.state.clone();
        let settings = self.state.read(cx).settings.clone();

        match transfer_state.mode {
            TransferMode::Export => {
                render_export_destination(self, transfer_state, &state, &settings, window, cx)
            }
            TransferMode::Import => render_import_destination(self, transfer_state, &state, cx),
            TransferMode::Copy => render_copy_destination(self, transfer_state, cx),
        }
    }
}

/// Render export destination panel with folder picker and filename template.
fn render_export_destination(
    view: &TransferView,
    transfer_state: &TransferTabState,
    state: &Entity<AppState>,
    settings: &AppSettings,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    // All export formats use folder picker + editable input with placeholders
    let Some(ref export_path_input_state) = view.export_path_input_state else {
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
    let is_bson_folder = is_bson && matches!(transfer_state.bson_output, BsonOutputFormat::Folder);

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
                                unexpanded_export_filename_for_scope(&settings, format, scope)
                            };
                            let full_path = folder_path.join(&filename).display().to_string();

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
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |mut menu, _window, _cx| {
                // Only show user-insertable placeholders (date/time variants)
                let insertable_placeholders = [
                    ("${datetime}", "Date and time"),
                    ("${date}", "Date only"),
                    ("${time}", "Time only"),
                ];

                for (placeholder, description) in insertable_placeholders {
                    let p = placeholder.to_string();
                    let export_path_input = export_path_input.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{} - {}", placeholder, description)).on_click(
                            move |_, window, cx| {
                                // Insert placeholder at cursor position
                                export_path_input.update(cx, |input_state, cx| {
                                    let cursor = input_state.cursor();
                                    let text = input_state.value().to_string();
                                    let mut new_text = String::with_capacity(text.len() + p.len());
                                    new_text.push_str(&text[..cursor]);
                                    new_text.push_str(&p);
                                    new_text.push_str(&text[cursor..]);
                                    input_state.set_value(new_text, window, cx);
                                    // Move cursor to after inserted text
                                    let new_cursor_offset = cursor + p.len();
                                    let position =
                                        input_state.text().offset_to_position(new_cursor_offset);
                                    input_state.set_cursor_position(
                                        Position::new(position.line, position.character),
                                        window,
                                        cx,
                                    );
                                });
                            },
                        ),
                    );
                }

                // Add reset option
                let state = state.clone();
                let export_path_input = export_path_input.clone();
                menu = menu.separator().item(PopupMenuItem::new("Reset to default").on_click(
                    move |_, window, cx| {
                        // Get current folder from path (if any)
                        let current_path = export_path_input.read(cx).value().to_string();
                        let folder =
                            std::path::Path::new(&current_path).parent().map(|p| p.to_path_buf());

                        // Generate default filename based on format and scope
                        let default_filename = match scope {
                            TransferScope::Database => {
                                // Database scope: path is a directory
                                if matches!(format, TransferFormat::Bson)
                                    && matches!(bson_output, BsonOutputFormat::Archive)
                                {
                                    // BSON Archive is a file
                                    format!("{}.archive", DATABASE_SCOPE_FILENAME_TEMPLATE)
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
                                            format!("{}.archive", DEFAULT_FILENAME_TEMPLATE)
                                        }
                                        BsonOutputFormat::Folder => {
                                            DEFAULT_FILENAME_TEMPLATE.to_string()
                                        }
                                    }
                                } else {
                                    format!("{}.{}", DEFAULT_FILENAME_TEMPLATE, format.extension())
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
                                && let Some(tab) = state.transfer_tab_mut(tab_id)
                            {
                                tab.file_path = new_path;
                                cx.notify();
                            }
                        });
                    },
                ));
                menu
            })
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
        div().flex().flex_col().gap(spacing::md()).child(form_row(dest_label, file_control)),
    )
    .into_any_element()
}

/// Render import destination panel with file browser.
fn render_import_destination(
    _view: &TransferView,
    transfer_state: &TransferTabState,
    state: &Entity<AppState>,
    _cx: &mut App,
) -> AnyElement {
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
        Button::new("browse-import").compact().label("Browse...").on_click(move |_, _, cx| {
            let filters = filters_for_format(format);
            let state = state.clone();
            cx.spawn(async move |cx| {
                if let Some(path) =
                    open_file_dialog_async(FilePickerMode::Open, filters, None).await
                {
                    cx.update(|cx| {
                        state.update(cx, |state, cx| {
                            if let Some(tab_id) = state.active_transfer_tab_id()
                                && let Some(tab) = state.transfer_tab_mut(tab_id)
                            {
                                // Auto-detect format
                                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                                    tab.format = match ext {
                                        "jsonl" | "ndjson" => TransferFormat::JsonLines,
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
        })
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
            .children(show_coll.then(|| form_row_static("Target collection", target_coll))),
    )
    .into_any_element()
}

/// Render copy destination panel with connection selector.
fn render_copy_destination(
    view: &TransferView,
    transfer_state: &TransferTabState,
    _cx: &mut App,
) -> AnyElement {
    // Searchable select for destination connection
    let Some(ref dest_conn_state) = view.dest_conn_state else {
        return panel("Destination", div().child("Loading...")).into_any_element();
    };

    let conn_select =
        Select::new(dest_conn_state).small().w_full().placeholder("Select connection...");

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
