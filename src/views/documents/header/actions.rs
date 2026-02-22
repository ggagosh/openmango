//! Action buttons rendering for collection header.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::{Disableable as _, Icon, IconName, Sizable as _, Size};
use mongodb::bson::Document;

use crate::bson::DocumentKey;
use crate::components::{Button, open_confirm_dialog};
use crate::keyboard::RunAggregation;
use crate::state::{AppCommands, AppState, SessionKey, TransferMode, TransferScope};
use crate::theme::{borders, spacing};
use crate::views::documents::CollectionView;
use crate::views::documents::dialogs::bulk_update::BulkUpdateDialog;

/// Render action buttons for the Documents subview.
#[allow(clippy::too_many_arguments)]
pub fn render_documents_actions(
    view: Entity<CollectionView>,
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    selected_doc: Option<DocumentKey>,
    selected_count: usize,
    any_selected_dirty: bool,
    is_loading: bool,
    filter_active: bool,
    cx: &mut Context<CollectionView>,
) -> Div {
    let state_for_refresh = state.clone();
    let state_for_apply = state.clone();
    let state_for_dialog = state.clone();
    let state_for_insert = state.clone();
    let state_for_delete = state.clone();
    let state_for_transfer = state.clone();

    let delete_variant = ButtonCustomVariant::new(cx)
        .color(cx.theme().danger)
        .foreground(cx.theme().danger_foreground)
        .border(cx.theme().danger)
        .hover(cx.theme().danger_hover)
        .active(cx.theme().danger_hover)
        .shadow(false);

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            Button::new("insert-document")
                .compact()
                .label("Insert")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state_for_insert = state_for_insert.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        CollectionView::open_insert_document_json_editor(
                            state_for_insert.clone(),
                            session_key,
                            window,
                            cx,
                        );
                    }
                }),
        )
        .child(
            Button::new("edit-json")
                .compact()
                .label("Edit JSON")
                .disabled(selected_doc.is_none() || session_key.is_none() || selected_count > 1)
                .on_click({
                    let selected_doc = selected_doc.clone();
                    let session_key = session_key.clone();
                    let view = view.clone();
                    let state_for_dialog = state_for_dialog.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(doc_key) = selected_doc.clone() else {
                            return;
                        };
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        CollectionView::open_document_json_editor(
                            view.clone(),
                            state_for_dialog.clone(),
                            session_key,
                            doc_key,
                            window,
                            cx,
                        );
                    }
                }),
        )
        .child(
            Button::new("discard")
                .compact()
                .label("Discard")
                .disabled(!any_selected_dirty)
                .on_click({
                    let view = view.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        view.update(cx, |this, cx| {
                            let Some(session_key) = this.view_model.current_session() else {
                                return;
                            };
                            let dirty_selected: Vec<_> = {
                                let state_ref = this.state.read(cx);
                                let Some(session) = state_ref.session(&session_key) else {
                                    return;
                                };
                                session
                                    .view
                                    .selected_docs
                                    .iter()
                                    .filter(|dk| session.view.dirty.contains(*dk))
                                    .cloned()
                                    .collect()
                            };
                            this.state.update(cx, |state, cx| {
                                for doc_key in &dirty_selected {
                                    state.clear_draft(&session_key, doc_key);
                                }
                                cx.notify();
                            });
                            this.view_model.clear_inline_edit();
                            this.view_model.rebuild_tree(&this.state, cx);
                            this.view_model.sync_dirty_state(&this.state, cx);
                            cx.notify();
                        });
                    }
                }),
        )
        .child(
            Button::new("apply")
                .primary()
                .compact()
                .label("Apply")
                .disabled(!any_selected_dirty)
                .on_click({
                    let state_for_apply = state_for_apply.clone();
                    let view = view.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        view.update(cx, |this, cx| {
                            this.view_model.commit_inline_edit(&this.state, cx);
                        });
                        let Some(session_key) = state_for_apply.read(cx).current_session_key()
                        else {
                            return;
                        };
                        let dirty_docs: Vec<_> = {
                            let state_ref = state_for_apply.read(cx);
                            let Some(session) = state_ref.session(&session_key) else {
                                return;
                            };
                            session
                                .view
                                .selected_docs
                                .iter()
                                .filter(|dk| session.view.dirty.contains(*dk))
                                .cloned()
                                .collect()
                        };
                        for doc_key in dirty_docs {
                            let doc =
                                state_for_apply.read(cx).session_draft(&session_key, &doc_key);
                            if let Some(doc) = doc {
                                AppCommands::save_document(
                                    state_for_apply.clone(),
                                    session_key.clone(),
                                    doc_key,
                                    doc,
                                    cx,
                                );
                            }
                        }
                    }
                }),
        )
        .child(
            Button::new("bulk-update")
                .compact()
                .label("Update")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let selected_doc = selected_doc.clone();
                    let state_for_dialog = state_for_dialog.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        BulkUpdateDialog::open(
                            state_for_dialog.clone(),
                            session_key,
                            selected_doc.clone(),
                            window,
                            cx,
                        );
                    }
                }),
        )
        .child(render_delete_menu(
            state_for_delete.clone(),
            session_key.clone(),
            selected_count,
            filter_active,
            delete_variant,
        ))
        .child(
            Button::new("export-collection")
                .compact()
                .label("Export")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state_for_transfer = state_for_transfer.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state_for_transfer.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                session_key.connection_id,
                                session_key.database.clone(),
                                Some(session_key.collection.clone()),
                                TransferScope::Collection,
                                TransferMode::Export,
                                cx,
                            );
                        });
                    }
                }),
        )
        .child(
            Button::new("import-collection")
                .compact()
                .label("Import")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state_for_transfer = state_for_transfer.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state_for_transfer.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                session_key.connection_id,
                                session_key.database.clone(),
                                Some(session_key.collection.clone()),
                                TransferScope::Collection,
                                TransferMode::Import,
                                cx,
                            );
                        });
                    }
                }),
        )
        .child(
            Button::new("copy-collection")
                .compact()
                .label("Copy")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    let state_for_transfer = state_for_transfer.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        state_for_transfer.update(cx, |state, cx| {
                            state.open_transfer_tab_with_prefill(
                                session_key.connection_id,
                                session_key.database.clone(),
                                Some(session_key.collection.clone()),
                                TransferScope::Collection,
                                TransferMode::Copy,
                                cx,
                            );
                        });
                    }
                }),
        )
        .child(
            Button::new("refresh")
                .ghost()
                .icon(Icon::new(IconName::Redo).xsmall())
                .disabled(is_loading)
                .on_click({
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(session_key) = state_for_refresh.read(cx).current_session_key()
                        {
                            AppCommands::load_documents_for_session(
                                state_for_refresh.clone(),
                                session_key,
                                cx,
                            );
                        }
                    }
                }),
        )
}

/// Render the delete dropdown menu with options.
fn render_delete_menu(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    selected_count: usize,
    filter_active: bool,
    delete_variant: ButtonCustomVariant,
) -> impl IntoElement {
    let delete_selected_label = if selected_count > 1 {
        format!("Delete {} documents", selected_count)
    } else {
        "Delete selected".to_string()
    };
    MenuButton::new("delete-menu")
        .compact()
        .label("Delete")
        .dropdown_caret(true)
        .custom(delete_variant)
        .rounded(borders::radius_sm())
        .with_size(Size::XSmall)
        .disabled(session_key.is_none())
        .dropdown_menu_with_anchor(Corner::BottomLeft, {
            let session_key = session_key.clone();
            let state_for_delete = state.clone();
            move |menu: PopupMenu, _window, cx| {
                let selected_docs: Vec<_> = {
                    let state_ref = state_for_delete.read(cx);
                    session_key
                        .as_ref()
                        .and_then(|sk| state_ref.session(sk))
                        .map(|session| session.view.selected_docs.iter().cloned().collect())
                        .unwrap_or_default()
                };
                let count = selected_docs.len();
                menu.item(
                    PopupMenuItem::new(delete_selected_label.clone())
                        .disabled(count == 0)
                        .on_click({
                            let session_key = session_key.clone();
                            let state_for_delete = state_for_delete.clone();
                            move |_, window, cx| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                if count == 1 {
                                    let doc_key = selected_docs[0].clone();
                                    let message = format!(
                                        "Delete document {}? This cannot be undone.",
                                        doc_key
                                    );
                                    open_confirm_dialog(
                                        window,
                                        cx,
                                        "Delete document",
                                        message,
                                        "Delete",
                                        true,
                                        {
                                            let state_for_delete = state_for_delete.clone();
                                            let session_key = session_key.clone();
                                            move |_window, cx| {
                                                AppCommands::delete_document(
                                                    state_for_delete.clone(),
                                                    session_key.clone(),
                                                    doc_key.clone(),
                                                    cx,
                                                );
                                            }
                                        },
                                    );
                                } else {
                                    let ids: Vec<mongodb::bson::Bson> = {
                                        let state_ref = state_for_delete.read(cx);
                                        selected_docs
                                            .iter()
                                            .filter_map(|dk| {
                                                state_ref
                                                    .document_for_key(&session_key, dk)
                                                    .and_then(|d| d.get("_id").cloned())
                                            })
                                            .collect()
                                    };
                                    if ids.is_empty() {
                                        return;
                                    }
                                    let filter =
                                        mongodb::bson::doc! { "_id": { "$in": ids } };
                                    let message = format!(
                                        "Delete {} documents? This cannot be undone.",
                                        count
                                    );
                                    open_confirm_dialog(
                                        window,
                                        cx,
                                        "Delete documents",
                                        message,
                                        "Delete",
                                        true,
                                        {
                                            let state_for_delete = state_for_delete.clone();
                                            let session_key = session_key.clone();
                                            move |_window, cx| {
                                                AppCommands::delete_documents_by_filter(
                                                    state_for_delete.clone(),
                                                    session_key.clone(),
                                                    filter.clone(),
                                                    cx,
                                                );
                                            }
                                        },
                                    );
                                }
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new("Delete filtered")
                        .disabled(!filter_active)
                        .on_click({
                            let session_key = session_key.clone();
                            let state_for_delete = state_for_delete.clone();
                            move |_, window, cx| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let filter = {
                                    let state_ref = state_for_delete.read(cx);
                                    state_ref
                                        .session_filter(&session_key)
                                        .unwrap_or_default()
                                };
                                if filter.is_empty() {
                                    return;
                                }
                                open_confirm_dialog(
                                    window,
                                    cx,
                                    "Delete filtered documents",
                                    "Delete all documents matching the current filter? This cannot be undone.".to_string(),
                                    "Delete",
                                    true,
                                    {
                                        let state_for_delete = state_for_delete.clone();
                                        let session_key = session_key.clone();
                                        move |_window, cx| {
                                            AppCommands::delete_documents_by_filter(
                                                state_for_delete.clone(),
                                                session_key.clone(),
                                                filter.clone(),
                                                cx,
                                            );
                                        }
                                    },
                                );
                            }
                        }),
                )
                .item(PopupMenuItem::new("Delete all").on_click({
                    let session_key = session_key.clone();
                    let state_for_delete = state_for_delete.clone();
                    move |_, window, cx| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        open_confirm_dialog(
                            window,
                            cx,
                            "Delete all documents",
                            "Delete all documents in this collection? This cannot be undone."
                                .to_string(),
                            "Delete",
                            true,
                            {
                                let state_for_delete = state_for_delete.clone();
                                let session_key = session_key.clone();
                                move |_window, cx| {
                                    AppCommands::delete_documents_by_filter(
                                        state_for_delete.clone(),
                                        session_key.clone(),
                                        Document::new(),
                                        cx,
                                    );
                                }
                            },
                        );
                    }
                }))
            }
        })
}

/// Render action buttons for the Indexes subview.
pub fn render_indexes_actions(state: Entity<AppState>, session_key: Option<SessionKey>) -> Div {
    let state_for_dialog = state.clone();
    let state_for_refresh = state.clone();

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            Button::new("create-index")
                .compact()
                .label("Create index")
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        CollectionView::open_index_create_dialog(
                            state_for_dialog.clone(),
                            session_key,
                            window,
                            cx,
                        );
                    }
                }),
        )
        .child(
            Button::new("refresh-indexes")
                .ghost()
                .icon(Icon::new(IconName::Redo).xsmall())
                .disabled(session_key.is_none())
                .on_click({
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        AppCommands::load_collection_indexes(
                            state_for_refresh.clone(),
                            session_key,
                            true,
                            cx,
                        );
                    }
                }),
        )
}

/// Render action buttons for the Stats subview.
pub fn render_stats_actions(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    stats_loading: bool,
) -> Div {
    let state_for_refresh = state.clone();

    div().flex().items_center().gap(spacing::sm()).child(
        Button::new("refresh-stats")
            .ghost()
            .icon(Icon::new(IconName::Redo).xsmall())
            .disabled(session_key.is_none() || stats_loading)
            .on_click({
                let session_key = session_key.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    let Some(session_key) = session_key.clone() else {
                        return;
                    };
                    AppCommands::load_collection_stats(state_for_refresh.clone(), session_key, cx);
                }
            }),
    )
}

/// Render action buttons for the Schema subview.
pub fn render_schema_actions(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    schema_loading: bool,
) -> Div {
    let state_for_refresh = state.clone();
    let state_for_copy = state.clone();

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            MenuButton::new("copy-schema")
                .ghost()
                .compact()
                .label("Copy Schema")
                .dropdown_caret(true)
                .rounded(borders::radius_sm())
                .with_size(Size::XSmall)
                .disabled(session_key.is_none() || schema_loading)
                .dropdown_menu_with_anchor(Corner::BottomLeft, {
                    let session_key = session_key.clone();
                    let state_for_copy = state_for_copy.clone();
                    move |menu: PopupMenu, _window, _cx| {
                        menu.item(PopupMenuItem::new("JSON Schema").on_click({
                            let session_key = session_key.clone();
                            let state = state_for_copy.clone();
                            move |_, _window, cx| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let state_ref = state.read(cx);
                                let schema = state_ref
                                    .session_data(&session_key)
                                    .and_then(|d| d.schema.as_ref());
                                if let Some(schema) = schema {
                                    let json =
                                        crate::state::commands::schema_to_json_schema(schema);
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
                                }
                            }
                        }))
                        .item(PopupMenuItem::new("Compass Format").on_click({
                            let session_key = session_key.clone();
                            let state = state_for_copy.clone();
                            move |_, _window, cx| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let state_ref = state.read(cx);
                                let schema = state_ref
                                    .session_data(&session_key)
                                    .and_then(|d| d.schema.as_ref());
                                if let Some(schema) = schema {
                                    let json = crate::state::commands::schema_to_compass(schema);
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
                                }
                            }
                        }))
                        .item(PopupMenuItem::new("Summary").on_click({
                            let session_key = session_key.clone();
                            let state = state_for_copy.clone();
                            move |_, _window, cx| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                let state_ref = state.read(cx);
                                let schema = state_ref
                                    .session_data(&session_key)
                                    .and_then(|d| d.schema.as_ref());
                                if let Some(schema) = schema {
                                    let json = crate::state::commands::schema_to_summary(schema);
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
                                }
                            }
                        }))
                    }
                }),
        )
        .child(
            Button::new("refresh-schema")
                .ghost()
                .icon(Icon::new(IconName::Redo).xsmall())
                .disabled(session_key.is_none() || schema_loading)
                .on_click({
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        AppCommands::analyze_collection_schema(
                            state_for_refresh.clone(),
                            session_key,
                            cx,
                        );
                    }
                }),
        )
}

/// Render action buttons for the Aggregation subview.
pub fn render_aggregation_actions(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    aggregation_loading: bool,
    explain_loading: bool,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            Button::new("agg-run")
                .primary()
                .compact()
                .label("Run")
                .tooltip_with_action(
                    "Run aggregation",
                    &RunAggregation,
                    Some("Documents Aggregation"),
                )
                .disabled(session_key.is_none() || aggregation_loading)
                .on_click({
                    let session_key = session_key.clone();
                    let state = state.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        AppCommands::run_aggregation(state.clone(), session_key, false, cx);
                    }
                }),
        )
        .child(
            Button::new("agg-explain")
                .compact()
                .label("Explain")
                .disabled(session_key.is_none() || explain_loading)
                .on_click({
                    let session_key = session_key.clone();
                    let state = state.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        AppCommands::run_explain_for_aggregation(state.clone(), session_key, cx);
                    }
                }),
        )
}
