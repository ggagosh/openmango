//! Mode-specific options panel rendering.

use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::button::Button as MenuButton;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::select::{SearchableVec, Select, SelectState};

use crate::state::{
    AppState, BsonOutputFormat, Encoding, ExtendedJsonMode, InsertMode, TransferFormat,
    TransferScope, TransferTabState,
};
use crate::theme::{borders, colors, spacing};

use super::helpers::{checkbox_field, option_field, option_field_static, option_section};

/// Render export-specific options sections.
pub(super) fn render_export_options(
    sections: &mut Vec<AnyElement>,
    state: Entity<AppState>,
    key: u64,
    transfer_state: &TransferTabState,
    exclude_coll_state: Option<&Entity<SelectState<SearchableVec<SharedString>>>>,
) {
    // Format-specific options
    match transfer_state.config.format {
        TransferFormat::Bson => {
            let bson_output_dropdown = {
                let state = state.clone();
                MenuButton::new(("bson-output", key))
                    .compact()
                    .label(transfer_state.options.bson_output.label())
                    .dropdown_caret(true)
                    .rounded(borders::radius_sm())
                    .with_size(gpui_component::Size::XSmall)
                    .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
                        let s1 = state.clone();
                        let s2 = state.clone();
                        menu.item(PopupMenuItem::new("Folder").on_click(move |_, _, cx| {
                            s1.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.options.bson_output = BsonOutputFormat::Folder;
                                    cx.notify();
                                }
                            });
                        }))
                        .item(
                            PopupMenuItem::new("Archive (.archive)").on_click(move |_, _, cx| {
                                s2.update(cx, |state, cx| {
                                    if let Some(id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(id)
                                    {
                                        tab.options.bson_output = BsonOutputFormat::Archive;
                                        cx.notify();
                                    }
                                });
                            }),
                        )
                    })
            };

            sections.push(
                option_section(
                    "BSON Options",
                    vec![option_field("Output", bson_output_dropdown.into_any_element())],
                )
                .into_any_element(),
            );
        }
        TransferFormat::Csv => {
            // CSV export - no options
        }
        _ => {
            // JSON Options - Extended JSON dropdown + Pretty print only
            let json_mode_dropdown = {
                let state = state.clone();
                MenuButton::new(("json-mode", key))
                    .compact()
                    .label(transfer_state.options.json_mode.label())
                    .dropdown_caret(true)
                    .rounded(borders::radius_sm())
                    .with_size(gpui_component::Size::XSmall)
                    .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
                        let s1 = state.clone();
                        let s2 = state.clone();
                        menu.item(PopupMenuItem::new("Relaxed").on_click(move |_, _, cx| {
                            s1.update(cx, |state, cx| {
                                if let Some(id) = state.active_transfer_tab_id()
                                    && let Some(tab) = state.transfer_tab_mut(id)
                                {
                                    tab.options.json_mode = ExtendedJsonMode::Relaxed;
                                    cx.notify();
                                }
                            });
                        }))
                        .item(PopupMenuItem::new("Canonical").on_click(
                            move |_, _, cx| {
                                s2.update(cx, |state, cx| {
                                    if let Some(id) = state.active_transfer_tab_id()
                                        && let Some(tab) = state.transfer_tab_mut(id)
                                    {
                                        tab.options.json_mode = ExtendedJsonMode::Canonical;
                                        cx.notify();
                                    }
                                });
                            },
                        ))
                    })
            };

            let pretty_checkbox = {
                let state = state.clone();
                let checked = transfer_state.options.pretty_print;
                checkbox_field(("pretty-print", key), checked, move |cx| {
                    state.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            tab.options.pretty_print = !checked;
                            cx.notify();
                        }
                    });
                })
            };

            sections.push(
                option_section(
                    "JSON Options",
                    vec![
                        option_field("Extended JSON", json_mode_dropdown.into_any_element()),
                        option_field("Pretty print", pretty_checkbox.into_any_element()),
                    ],
                )
                .into_any_element(),
            );
        }
    }

    // Database scope options (only for BSON format - indexes can't be stored in JSON/CSV)
    if matches!(transfer_state.config.scope, TransferScope::Database)
        && matches!(transfer_state.config.format, TransferFormat::Bson)
    {
        let include_indexes_checkbox = {
            let state = state.clone();
            let checked = transfer_state.options.include_indexes;
            checkbox_field(("include-indexes-export", key), checked, move |cx| {
                state.update(cx, |state, cx| {
                    if let Some(id) = state.active_transfer_tab_id()
                        && let Some(tab) = state.transfer_tab_mut(id)
                    {
                        tab.options.include_indexes = !checked;
                        cx.notify();
                    }
                });
            })
        };

        sections.push(
            option_section(
                "Database",
                vec![option_field("Include indexes", include_indexes_checkbox.into_any_element())],
            )
            .into_any_element(),
        );

        // Collection Filter section
        let exclude_select = if let Some(exclude_state) = exclude_coll_state {
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
                transfer_state.options.exclude_collections.iter().enumerate().map(|(idx, coll)| {
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
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.options
                                                .exclude_collections
                                                .retain(|c| c != &coll_name);
                                            cx.notify();
                                        }
                                    });
                                })
                                .child("×"),
                        )
                }),
            )
        };

        sections.push(
            option_section(
                "Collection Filter",
                vec![option_field("Exclude", exclude_select), excluded_tags.into_any_element()],
            )
            .into_any_element(),
        );
    }
}

/// Render import-specific options sections.
pub(super) fn render_import_options(
    sections: &mut Vec<AnyElement>,
    state: Entity<AppState>,
    key: u64,
    transfer_state: &TransferTabState,
) {
    // Input section
    let detect_format_checkbox = {
        let state = state.clone();
        let checked = transfer_state.options.detect_format;
        checkbox_field(("detect-format", key), checked, move |cx| {
            state.update(cx, |state, cx| {
                if let Some(id) = state.active_transfer_tab_id()
                    && let Some(tab) = state.transfer_tab_mut(id)
                {
                    tab.options.detect_format = !checked;
                    cx.notify();
                }
            });
        })
    };

    let encoding_dropdown = {
        let state = state.clone();
        MenuButton::new(("encoding", key))
            .compact()
            .label(transfer_state.options.encoding.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(gpui_component::Size::XSmall)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
                let s1 = state.clone();
                let s2 = state.clone();
                menu.item(PopupMenuItem::new("UTF-8").on_click(move |_, _, cx| {
                    s1.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            tab.options.encoding = Encoding::Utf8;
                            cx.notify();
                        }
                    });
                }))
                .item(PopupMenuItem::new("Latin-1").on_click(move |_, _, cx| {
                    s2.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            tab.options.encoding = Encoding::Latin1;
                            cx.notify();
                        }
                    });
                }))
            })
    };

    sections.push(
        option_section(
            "Input",
            vec![
                option_field("Detect format", detect_format_checkbox.into_any_element()),
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
            .label(transfer_state.options.insert_mode.label())
            .dropdown_caret(true)
            .rounded(borders::radius_sm())
            .with_size(gpui_component::Size::XSmall)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _window, _cx| {
                let s1 = state.clone();
                let s2 = state.clone();
                let s3 = state.clone();
                menu.item(PopupMenuItem::new("Insert").on_click(move |_, _, cx| {
                    s1.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            tab.options.insert_mode = InsertMode::Insert;
                            cx.notify();
                        }
                    });
                }))
                .item(PopupMenuItem::new("Upsert").on_click(move |_, _, cx| {
                    s2.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            tab.options.insert_mode = InsertMode::Upsert;
                            cx.notify();
                        }
                    });
                }))
                .item(PopupMenuItem::new("Replace").on_click(move |_, _, cx| {
                    s3.update(cx, |state, cx| {
                        if let Some(id) = state.active_transfer_tab_id()
                            && let Some(tab) = state.transfer_tab_mut(id)
                        {
                            tab.options.insert_mode = InsertMode::Replace;
                            cx.notify();
                        }
                    });
                }))
            })
    };

    let drop_checkbox = {
        let state = state.clone();
        let checked = transfer_state.options.drop_before_import;
        checkbox_field(("drop-before", key), checked, move |cx| {
            state.update(cx, |state, cx| {
                if let Some(id) = state.active_transfer_tab_id()
                    && let Some(tab) = state.transfer_tab_mut(id)
                {
                    tab.options.drop_before_import = !checked;
                    cx.notify();
                }
            });
        })
    };

    let clear_checkbox = {
        let state = state.clone();
        let checked = transfer_state.options.clear_before_import;
        checkbox_field(("clear-before", key), checked, move |cx| {
            state.update(cx, |state, cx| {
                if let Some(id) = state.active_transfer_tab_id()
                    && let Some(tab) = state.transfer_tab_mut(id)
                {
                    tab.options.clear_before_import = !checked;
                    cx.notify();
                }
            });
        })
    };

    let stop_checkbox = {
        let state = state.clone();
        let checked = transfer_state.options.stop_on_error;
        checkbox_field(("stop-on-error", key), checked, move |cx| {
            state.update(cx, |state, cx| {
                if let Some(id) = state.active_transfer_tab_id()
                    && let Some(tab) = state.transfer_tab_mut(id)
                {
                    tab.options.stop_on_error = !checked;
                    cx.notify();
                }
            });
        })
    };

    sections.push(
        option_section(
            "Insert",
            vec![
                option_field("Insert mode", insert_mode_dropdown.into_any_element()),
                option_field_static("Batch size", transfer_state.options.batch_size.to_string()),
                option_field("Drop before import", drop_checkbox.into_any_element()),
                option_field("Clear before import", clear_checkbox.into_any_element()),
                option_field("Stop on error", stop_checkbox.into_any_element()),
            ],
        )
        .into_any_element(),
    );

    // Database scope options (only for BSON format - indexes can't be stored in JSON/CSV)
    if matches!(transfer_state.config.scope, TransferScope::Database)
        && matches!(transfer_state.config.format, TransferFormat::Bson)
    {
        let restore_indexes_checkbox = {
            let state = state.clone();
            let checked = transfer_state.options.restore_indexes;
            checkbox_field(("restore-indexes", key), checked, move |cx| {
                state.update(cx, |state, cx| {
                    if let Some(id) = state.active_transfer_tab_id()
                        && let Some(tab) = state.transfer_tab_mut(id)
                    {
                        tab.options.restore_indexes = !checked;
                        cx.notify();
                    }
                });
            })
        };

        sections.push(
            option_section(
                "Database",
                vec![option_field("Restore indexes", restore_indexes_checkbox.into_any_element())],
            )
            .into_any_element(),
        );
    }
}

/// Render copy-specific options sections.
pub(super) fn render_copy_options(
    sections: &mut Vec<AnyElement>,
    state: Entity<AppState>,
    key: u64,
    transfer_state: &TransferTabState,
    exclude_coll_state: Option<&Entity<SelectState<SearchableVec<SharedString>>>>,
) {
    // Copy Options - only show implemented options (copy_indexes, batch_size)
    let copy_indexes_checkbox = {
        let state = state.clone();
        let checked = transfer_state.options.copy_indexes;
        checkbox_field(("copy-indexes", key), checked, move |cx| {
            state.update(cx, |state, cx| {
                if let Some(id) = state.active_transfer_tab_id()
                    && let Some(tab) = state.transfer_tab_mut(id)
                {
                    tab.options.copy_indexes = !checked;
                    cx.notify();
                }
            });
        })
    };

    sections.push(
        option_section(
            "Copy Options",
            vec![
                option_field("Copy indexes", copy_indexes_checkbox.into_any_element()),
                option_field_static("Batch size", transfer_state.options.batch_size.to_string()),
            ],
        )
        .into_any_element(),
    );

    // Collection Filter section (for Copy mode + Database scope)
    if matches!(transfer_state.config.scope, TransferScope::Database) {
        let exclude_select = if let Some(exclude_state) = exclude_coll_state {
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
                transfer_state.options.exclude_collections.iter().enumerate().map(|(idx, coll)| {
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
                                        if let Some(id) = state.active_transfer_tab_id()
                                            && let Some(tab) = state.transfer_tab_mut(id)
                                        {
                                            tab.options
                                                .exclude_collections
                                                .retain(|c| c != &coll_name);
                                            cx.notify();
                                        }
                                    });
                                })
                                .child("×"),
                        )
                }),
            )
        };

        sections.push(
            option_section(
                "Collection Filter",
                vec![option_field("Exclude", exclude_select), excluded_tags.into_any_element()],
            )
            .into_any_element(),
        );
    }
}
