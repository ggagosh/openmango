use gpui::*;
use gpui_component::button::Button as MenuButton;
use gpui_component::input::{Input, InputState, Position, RopeExt};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::select::Select;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, Size};
use uuid::Uuid;

use crate::components::file_picker::{
    FilePickerMode, filters_for_format, open_file_dialog_async, open_folder_dialog_async,
    unexpanded_export_filename_bson_for_scope, unexpanded_export_filename_for_scope,
};
use crate::components::{Button, ConnectionIdentity};
use crate::state::{
    CompressionMode, TransferFormat, TransferMode, TransferScope, TransferTabState,
    available_transfer_formats, coerce_transfer_format, validate_transfer,
};
use crate::theme::{borders, islands, spacing};

use super::helpers::{option_field, option_section, render_query_field_row};
use super::progress_panel::{render_progress_status, render_warnings};
use super::{QueryEditField, TransferView, cancel_active_transfer, options, run_active_transfer};

fn property_row(label: &str, control: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .min_h(px(44.0))
        .px(spacing::md())
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            div()
                .w(px(140.0))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w(px(0.0)).child(control))
}

fn insert_filename_token(
    input_state: &Entity<InputState>,
    token: &str,
    window: &mut Window,
    cx: &mut App,
) {
    input_state.update(cx, |input, cx| {
        let cursor = input.cursor();
        let text = input.value().to_string();
        let mut value = String::with_capacity(text.len() + token.len());
        value.push_str(&text[..cursor]);
        value.push_str(token);
        value.push_str(&text[cursor..]);
        input.set_value(value, window, cx);
        let position = input.text().offset_to_position(cursor + token.len());
        input.set_cursor_position(Position::new(position.line, position.character), window, cx);
    });
}

fn namespace(database: &str, collection: &str, scope: TransferScope) -> String {
    if matches!(scope, TransferScope::Collection) && !collection.is_empty() {
        format!("{database}.{collection}")
    } else if database.is_empty() {
        "Not selected".to_string()
    } else {
        database.to_string()
    }
}

impl TransferView {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_simple_transfer(
        &self,
        transfer_id: Uuid,
        key: u64,
        transfer_state: &TransferTabState,
        source_identity: Option<&ConnectionIdentity>,
        destination_identity: Option<&ConnectionIdentity>,
        view: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.state.clone();
        let appearance = self.state.read(cx).settings.appearance.clone();
        let mode = transfer_state.config.mode;
        let scope = transfer_state.config.scope;
        let source_name = source_identity
            .map(ConnectionIdentity::display_name)
            .unwrap_or_else(|| "No connection".to_string());
        let source_namespace = namespace(
            &transfer_state.config.source_database,
            &transfer_state.config.source_collection,
            scope,
        );
        let destination_name = destination_identity
            .map(ConnectionIdentity::display_name)
            .unwrap_or_else(|| "No connection".to_string());
        let destination_namespace = namespace(
            &transfer_state.config.destination_database,
            &transfer_state.config.destination_collection,
            scope,
        );

        let edit_button = |id: &'static str, view: Entity<Self>| {
            Button::new((id, key)).ghost().compact().label("Edit...").on_click(move |_, _, cx| {
                view.update(cx, |view, cx| {
                    view.options_expanded = true;
                    cx.notify();
                });
            })
        };

        let source_summary = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::md())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!("{source_name} / {source_namespace}")),
            )
            .child(edit_button("edit-source", view.clone()));
        let destination_summary = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::md())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!("{destination_name} / {destination_namespace}")),
            )
            .child(edit_button("edit-target", view.clone()));

        let scope_button = {
            let state = state.clone();
            MenuButton::new(("simple-scope", key))
                .compact()
                .label(scope.label())
                .dropdown_caret(true)
                .rounded(borders::radius_sm())
                .with_size(Size::Small)
                .dropdown_menu_with_anchor(Corner::TopLeft, move |menu, _window, _cx| {
                    let collection_state = state.clone();
                    let database_state = state.clone();
                    menu.item(PopupMenuItem::new("Collection").on_click(move |_, _, cx| {
                        collection_state.update(cx, |state, cx| {
                            if let Some(id) = state.active_transfer_tab_id()
                                && let Some(tab) = state.transfer_tab_mut(id)
                            {
                                tab.config.scope = TransferScope::Collection;
                                tab.config.format = coerce_transfer_format(
                                    tab.config.mode,
                                    tab.config.scope,
                                    tab.config.format,
                                );
                                cx.notify();
                            }
                        });
                    }))
                    .item(PopupMenuItem::new("Database").on_click(
                        move |_, _, cx| {
                            database_state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.config.scope = TransferScope::Database;
                                    tab.config.format = coerce_transfer_format(
                                        tab.config.mode,
                                        tab.config.scope,
                                        tab.config.format,
                                    );
                                    cx.notify();
                                }
                            });
                        },
                    ))
                })
        };

        let format_button = if matches!(mode, TransferMode::Export | TransferMode::Import) {
            let state = state.clone();
            let formats = available_transfer_formats(mode, scope);
            Some(
                MenuButton::new(("simple-format", key))
                    .compact()
                    .label(transfer_state.config.format.label())
                    .dropdown_caret(true)
                    .rounded(borders::radius_sm())
                    .with_size(Size::Small)
                    .dropdown_menu_with_anchor(Corner::TopLeft, move |mut menu, _window, _cx| {
                        for format in formats.clone() {
                            let state = state.clone();
                            menu = menu.item(PopupMenuItem::new(format.label()).on_click(
                                move |_, _, cx| {
                                    state.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.config.format = format;
                                            if matches!(tab.config.mode, TransferMode::Export) {
                                                tab.config.file_path.clear();
                                            }
                                            cx.notify();
                                        }
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
        } else {
            None
        };

        let mut primary_rows = Vec::new();
        match mode {
            TransferMode::Export => {
                primary_rows.push(property_row("Source", source_summary, cx).into_any_element());
                primary_rows.push(property_row("Scope", scope_button, cx).into_any_element());
                if let Some(format_button) = format_button {
                    primary_rows.push(property_row("Format", format_button, cx).into_any_element());
                }
                primary_rows.push(
                    property_row(
                        "Save to",
                        self.render_export_destination_control(transfer_state, window, cx),
                        cx,
                    )
                    .into_any_element(),
                );
            }
            TransferMode::Import => {
                primary_rows.push(
                    property_row(
                        "Import file",
                        self.render_import_file_control(transfer_state, cx),
                        cx,
                    )
                    .into_any_element(),
                );
                primary_rows.push(property_row("Target", source_summary, cx).into_any_element());
                primary_rows.push(property_row("Scope", scope_button, cx).into_any_element());
                if let Some(format_button) = format_button {
                    primary_rows.push(property_row("Format", format_button, cx).into_any_element());
                }
            }
            TransferMode::Copy => {
                primary_rows.push(property_row("Source", source_summary, cx).into_any_element());
                primary_rows
                    .push(property_row("Target", destination_summary, cx).into_any_element());
                primary_rows.push(property_row("Scope", scope_button, cx).into_any_element());
            }
        }

        let advanced_header = {
            let view = view.clone();
            let expanded = self.options_expanded;
            div()
                .id(("simple-advanced", key))
                .flex()
                .items_center()
                .min_h(px(40.0))
                .px(spacing::md())
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    view.update(cx, |view, cx| {
                        view.options_expanded = !view.options_expanded;
                        cx.notify();
                    });
                })
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .ml(spacing::xs())
                        .text_sm()
                        .text_color(cx.theme().secondary_foreground)
                        .child("Advanced options"),
                )
        };

        let advanced_content = if self.options_expanded {
            self.render_simple_advanced(key, transfer_state, view.clone(), window, cx)
        } else {
            div().into_any_element()
        };

        let validation = validate_transfer(transfer_state);
        let can_run = validation.can_run();
        let cancellation_pending = transfer_state.runtime.cancellation_pending();
        let action_button = if cancellation_pending {
            Button::new(("simple-cancelling", key))
                .ghost()
                .compact()
                .label("Cancelling...")
                .disabled(true)
                .into_any_element()
        } else if transfer_state.runtime.is_running {
            let state = state.clone();
            Button::new(("simple-cancel", key))
                .ghost()
                .compact()
                .label("Cancel")
                .on_click(move |_, _, cx| cancel_active_transfer(state.clone(), cx))
                .into_any_element()
        } else {
            let state = state.clone();
            Button::new(("simple-run", key))
                .primary()
                .compact()
                .label(mode.label())
                .disabled(!can_run)
                .on_click(move |_, window, cx| run_active_transfer(state.clone(), window, cx))
                .into_any_element()
        };
        let idle_hint = validation.blocking_errors.first().cloned().unwrap_or_else(|| match mode {
            TransferMode::Export => "Ready to export".to_string(),
            TransferMode::Import => "Ready to import".to_string(),
            TransferMode::Copy => "Ready to copy".to_string(),
        });
        let show_validation =
            !can_run && !transfer_state.runtime.is_running && !cancellation_pending;
        let status = if show_validation {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(idle_hint)
                .into_any_element()
        } else if transfer_state.runtime.has_started
            || transfer_state.runtime.error_message.is_some()
        {
            render_progress_status(transfer_state, state.clone(), transfer_id, cx)
        } else {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(idle_hint)
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .max_w(px(900.0))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .overflow_hidden()
                            .bg(islands::card_bg(&appearance, cx))
                            .border_1()
                            .border_color(islands::panel_border(&appearance, cx))
                            .rounded(islands::radius_sm(&appearance))
                            .children(primary_rows)
                            .child(advanced_header)
                            .child(advanced_content),
                    )
                    .child(div().mt(spacing::md()).child(render_warnings(transfer_state, cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::lg())
                    .pt(spacing::md())
                    .border_t_1()
                    .border_color(islands::panel_border(&appearance, cx))
                    .child(div().flex_1().min_w(px(0.0)).child(status))
                    .child(action_button),
            )
            .into_any_element()
    }

    fn render_export_destination_control(
        &self,
        transfer_state: &TransferTabState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(input_state) = self.export_path_input_state.as_ref() else {
            return div().child("Loading...").into_any_element();
        };
        let current = input_state.read(cx).value().to_string();
        if current != transfer_state.config.file_path {
            input_state.update(cx, |input, cx| {
                input.set_value(transfer_state.config.file_path.clone(), window, cx);
            });
        }

        let state = self.state.clone();
        let settings = self.state.read(cx).settings.clone();
        let format = transfer_state.config.format;
        let output = transfer_state.options.bson_output;
        let scope = transfer_state.config.scope;
        let browse = Button::new("simple-export-browse")
            .compact()
            .icon(IconName::Folder)
            .label("Choose...")
            .on_click(move |_, _, cx| {
                let state = state.clone();
                let settings = settings.clone();
                cx.spawn(async move |cx| {
                    if let Some(folder) = open_folder_dialog_async().await {
                        cx.update(|cx| {
                            let filename = if matches!(format, TransferFormat::Bson) {
                                unexpanded_export_filename_bson_for_scope(&settings, output, scope)
                            } else {
                                unexpanded_export_filename_for_scope(&settings, format, scope)
                            };
                            let path = folder.join(filename).display().to_string();
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.config.file_path = path;
                                    cx.notify();
                                }
                            });
                        })
                        .ok();
                    }
                })
                .detach();
            });

        div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .child(Input::new(input_state).small().flex_1())
            .child(browse)
            .into_any_element()
    }

    fn render_filename_token_control(
        &self,
        transfer_state: &TransferTabState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(input_state) = self.export_path_input_state.as_ref() else {
            return div().into_any_element();
        };
        let input_state = input_state.clone();
        let settings = self.state.read(cx).settings.clone();
        let format = transfer_state.config.format;
        let output = transfer_state.options.bson_output;
        let scope = transfer_state.config.scope;

        MenuButton::new("simple-filename-tokens")
            .compact()
            .label("Insert...")
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(Size::Small)
            .dropdown_menu_with_anchor(Corner::TopLeft, move |menu, _window, _cx| {
                let datetime_input = input_state.clone();
                let date_input = input_state.clone();
                let time_input = input_state.clone();
                let reset_input = input_state.clone();
                let settings = settings.clone();
                menu.item(PopupMenuItem::new("${datetime} — Date and time").on_click(
                    move |_, window, cx| {
                        insert_filename_token(&datetime_input, "${datetime}", window, cx);
                    },
                ))
                .item(PopupMenuItem::new("${date} — Date").on_click(move |_, window, cx| {
                    insert_filename_token(&date_input, "${date}", window, cx);
                }))
                .item(PopupMenuItem::new("${time} — Time").on_click(move |_, window, cx| {
                    insert_filename_token(&time_input, "${time}", window, cx);
                }))
                .separator()
                .item(PopupMenuItem::new("Reset to default").on_click(
                    move |_, window, cx| {
                        let current = reset_input.read(cx).value().to_string();
                        let filename = if matches!(format, TransferFormat::Bson) {
                            unexpanded_export_filename_bson_for_scope(&settings, output, scope)
                        } else {
                            unexpanded_export_filename_for_scope(&settings, format, scope)
                        };
                        let value = std::path::Path::new(&current)
                            .parent()
                            .filter(|path| !path.as_os_str().is_empty())
                            .map(|path| path.join(&filename).display().to_string())
                            .unwrap_or(filename);
                        reset_input.update(cx, |input, cx| {
                            input.set_value(value, window, cx);
                        });
                    },
                ))
            })
            .into_any_element()
    }

    fn render_import_file_control(
        &self,
        transfer_state: &TransferTabState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let file_name = if transfer_state.config.file_path.is_empty() {
            "No file selected".to_string()
        } else {
            std::path::Path::new(&transfer_state.config.file_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&transfer_state.config.file_path)
                .to_string()
        };
        let state = self.state.clone();
        let format = transfer_state.config.format;
        let browse = Button::new("simple-import-browse")
            .compact()
            .icon(IconName::Folder)
            .label("Choose...")
            .on_click(move |_, _, cx| {
                let state = state.clone();
                cx.spawn(async move |cx| {
                    if let Some(path) = open_file_dialog_async(
                        FilePickerMode::Open,
                        filters_for_format(format),
                        None,
                    )
                    .await
                    {
                        cx.update(|cx| {
                            state.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    if tab.options.detect_format
                                        && let Some(extension) =
                                            path.extension().and_then(|value| value.to_str())
                                    {
                                        tab.config.format = match extension {
                                            "jsonl" | "ndjson" => TransferFormat::JsonLines,
                                            "json" => TransferFormat::JsonArray,
                                            "csv" => TransferFormat::Csv,
                                            "archive" | "bson" => TransferFormat::Bson,
                                            _ => tab.config.format,
                                        };
                                    }
                                    tab.config.file_path = path.display().to_string();
                                    cx.notify();
                                }
                            });
                        })
                        .ok();
                    }
                })
                .detach();
            });

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .child(
                div()
                    .text_sm()
                    .text_color(if transfer_state.config.file_path.is_empty() {
                        cx.theme().muted_foreground
                    } else {
                        cx.theme().foreground
                    })
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(file_name),
            )
            .child(browse)
            .into_any_element()
    }

    fn render_simple_advanced(
        &self,
        key: u64,
        transfer_state: &TransferTabState,
        view: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.state.clone();
        let mut content =
            div().border_t_1().border_color(cx.theme().sidebar_border).flex().flex_col();

        let source_label = if matches!(transfer_state.config.mode, TransferMode::Import) {
            "Target"
        } else {
            "Source"
        };
        content = content.child(self.render_source_editor(source_label, transfer_state, cx));
        if matches!(transfer_state.config.mode, TransferMode::Copy) {
            content = content.child(self.render_copy_target_editor(transfer_state, window, cx));
        }

        let mut sections = Vec::new();
        match transfer_state.config.mode {
            TransferMode::Export => {
                let filename_tokens = self.render_filename_token_control(transfer_state, cx);
                let compression = {
                    let state = state.clone();
                    MenuButton::new(("simple-compression", key))
                        .compact()
                        .label(transfer_state.options.compression.label())
                        .dropdown_caret(true)
                        .rounded(borders::radius_sm())
                        .with_size(Size::Small)
                        .dropdown_menu_with_anchor(Corner::TopLeft, move |menu, _window, _cx| {
                            let none_state = state.clone();
                            let gzip_state = state.clone();
                            menu.item(PopupMenuItem::new("None").on_click(move |_, _, cx| {
                                none_state.update(cx, |state, cx| {
                                    if let Some(id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(id)
                                    {
                                        tab.options.compression = CompressionMode::None;
                                        cx.notify();
                                    }
                                });
                            }))
                            .item(
                                PopupMenuItem::new("Gzip").on_click(move |_, _, cx| {
                                    gzip_state.update(cx, |state, cx| {
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.options.compression = CompressionMode::Gzip;
                                            cx.notify();
                                        }
                                    });
                                }),
                            )
                        })
                };
                sections.push(
                    option_section(
                        "Output",
                        vec![
                            option_field("Compression", compression.into_any_element(), cx),
                            option_field("Filename token", filename_tokens, cx),
                        ],
                        cx,
                    )
                    .into_any_element(),
                );
                if matches!(transfer_state.config.scope, TransferScope::Collection) {
                    sections.push(
                        option_section(
                            "Query",
                            vec![
                                render_query_field_row(
                                    "Filter",
                                    QueryEditField::Filter,
                                    &transfer_state.options.export_filter,
                                    view.clone(),
                                    state.clone(),
                                    cx,
                                )
                                .into_any_element(),
                                render_query_field_row(
                                    "Projection",
                                    QueryEditField::Projection,
                                    &transfer_state.options.export_projection,
                                    view.clone(),
                                    state.clone(),
                                    cx,
                                )
                                .into_any_element(),
                                render_query_field_row(
                                    "Sort",
                                    QueryEditField::Sort,
                                    &transfer_state.options.export_sort,
                                    view,
                                    state.clone(),
                                    cx,
                                )
                                .into_any_element(),
                            ],
                            cx,
                        )
                        .into_any_element(),
                    );
                }
                options::render_export_options(
                    &mut sections,
                    state,
                    key,
                    transfer_state,
                    self.exclude_coll_state.as_ref(),
                    cx,
                );
            }
            TransferMode::Import => {
                options::render_import_options(&mut sections, state, key, transfer_state, cx);
            }
            TransferMode::Copy => {
                options::render_copy_options(
                    &mut sections,
                    state,
                    key,
                    transfer_state,
                    self.exclude_coll_state.as_ref(),
                    cx,
                );
            }
        }

        content.child(div().flex().flex_col().children(sections)).into_any_element()
    }

    fn render_source_editor(
        &self,
        title: &str,
        transfer_state: &TransferTabState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(connection), Some(database)) =
            (self.source_conn_state.as_ref(), self.source_db_state.as_ref())
        else {
            return div().into_any_element();
        };
        let mut rows = vec![
            property_row(
                &format!("{title} connection"),
                Select::new(connection).small().w_full(),
                cx,
            )
            .into_any_element(),
            property_row(&format!("{title} database"), Select::new(database).small().w_full(), cx)
                .into_any_element(),
        ];
        if matches!(transfer_state.config.scope, TransferScope::Collection)
            && let Some(collection) = self.source_coll_state.as_ref()
        {
            rows.push(
                property_row(
                    &format!("{title} collection"),
                    Select::new(collection).small().w_full(),
                    cx,
                )
                .into_any_element(),
            );
        }
        div().children(rows).into_any_element()
    }

    fn render_copy_target_editor(
        &self,
        transfer_state: &TransferTabState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(connection), Some(database)) =
            (self.dest_conn_state.as_ref(), self.dest_db_input_state.as_ref())
        else {
            return div().into_any_element();
        };
        if database.read(cx).value().as_ref() != transfer_state.config.destination_database {
            database.update(cx, |input, cx| {
                input.set_value(transfer_state.config.destination_database.clone(), window, cx);
            });
        }
        let mut rows = vec![
            property_row("Target connection", Select::new(connection).small().w_full(), cx)
                .into_any_element(),
            property_row("Target database", Input::new(database).small().w_full(), cx)
                .into_any_element(),
        ];
        if matches!(transfer_state.config.scope, TransferScope::Collection)
            && let Some(collection) = self.dest_coll_input_state.as_ref()
        {
            if collection.read(cx).value().as_ref() != transfer_state.config.destination_collection
            {
                collection.update(cx, |input, cx| {
                    input.set_value(
                        transfer_state.config.destination_collection.clone(),
                        window,
                        cx,
                    );
                });
            }
            rows.push(
                property_row("Target collection", Input::new(collection).small().w_full(), cx)
                    .into_any_element(),
            );
        }
        div().children(rows).into_any_element()
    }
}
