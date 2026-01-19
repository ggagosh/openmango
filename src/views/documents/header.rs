//! Header bar rendering for collection view.

use gpui::*;
use gpui_component::button::{Button as MenuButton, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::spinner::Spinner;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{Disableable as _, Icon, IconName, Sizable as _, Size, WindowExt};

use crate::bson::{DocumentKey, document_to_relaxed_extjson_string, parse_document_from_json};
use mongodb::bson::Document;
use crate::components::{Button, open_confirm_dialog};
use crate::helpers::{format_bytes, format_number};
use crate::state::{
    AppCommands, AppState, CollectionStats, CollectionSubview, SessionKey, StatusMessage,
};
use crate::theme::{borders, colors, spacing};

use super::index_create::IndexCreateDialog;
use super::CollectionView;

impl CollectionView {
    pub(super) fn apply_filter(
        state: Entity<AppState>,
        session_key: SessionKey,
        filter_state: Entity<InputState>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let raw = filter_state.read(cx).value().to_string();
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed == "{}" {
            state.update(cx, |state, cx| {
                state.clear_filter(&session_key);
                state.status_message = Some(StatusMessage::info("Filter cleared"));
                cx.notify();
            });
            AppCommands::load_documents_for_session(state.clone(), session_key, cx);
            return;
        }

        match parse_document_from_json(trimmed) {
            Ok(filter) => {
                state.update(cx, |state, cx| {
                    state.set_filter(&session_key, trimmed.to_string(), Some(filter));
                    state.status_message = Some(StatusMessage::info("Filter applied"));
                    cx.notify();
                });
                AppCommands::load_documents_for_session(state.clone(), session_key, cx);
            }
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.status_message =
                        Some(StatusMessage::error(format!("Invalid filter JSON: {err}")));
                    cx.notify();
                });
            }
        }
    }

    pub(super) fn apply_query_options(
        state: Entity<AppState>,
        session_key: SessionKey,
        sort_state: Entity<InputState>,
        projection_state: Entity<InputState>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let sort_raw = sort_state.read(cx).value().to_string();
        let projection_raw = projection_state.read(cx).value().to_string();

        let (sort_raw_store, sort_doc) = match parse_optional_doc(&sort_raw) {
            Ok(result) => result,
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.status_message = Some(StatusMessage::error(format!(
                        "Invalid sort JSON: {err}"
                    )));
                    cx.notify();
                });
                return;
            }
        };

        let (projection_raw_store, projection_doc) = match parse_optional_doc(&projection_raw) {
            Ok(result) => result,
            Err(err) => {
                state.update(cx, |state, cx| {
                    state.status_message = Some(StatusMessage::error(format!(
                        "Invalid projection JSON: {err}"
                    )));
                    cx.notify();
                });
                return;
            }
        };

        let message = if sort_doc.is_none() && projection_doc.is_none() {
            "Sort/projection cleared"
        } else {
            "Sort/projection applied"
        };

        state.update(cx, |state, cx| {
            state.set_sort_projection(
                &session_key,
                sort_raw_store,
                sort_doc,
                projection_raw_store,
                projection_doc,
            );
            state.status_message = Some(StatusMessage::info(message));
            cx.notify();
        });
        AppCommands::load_documents_for_session(state.clone(), session_key, cx);
    }

    pub(super) fn open_document_json_editor(
        view: Entity<CollectionView>,
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        let doc = {
            let state_ref = state.read(cx);
            state_ref
                .session(&session_key)
                .and_then(|session| session.view.drafts.get(&doc_key).cloned())
                .or_else(|| state_ref.document_for_key(&session_key, &doc_key))
        };
        let Some(doc) = doc else {
            return;
        };

        let json_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .line_number(true)
                .searchable(true)
                .soft_wrap(true)
        });

        let json_text = document_to_relaxed_extjson_string(&doc);
        json_state.update(cx, |state, cx| {
            state.set_value(json_text, window, cx);
        });

        let session_key_for_update = session_key.clone();
        let doc_key_for_update = doc_key.clone();
        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog
                .title("Document JSON Editor")
                .min_w(px(720.0))
                .child(
                    div().flex().flex_col().gap(spacing::sm()).p(spacing::md()).child(
                        Input::new(&json_state)
                            .font_family(crate::theme::fonts::mono())
                            .h(px(420.0))
                            .w_full(),
                    ),
                )
                .footer({
                    let json_state = json_state.clone();
                    let view = view.clone();
                    let session_key = session_key_for_update.clone();
                    let doc_key = doc_key_for_update.clone();
                    move |_ok_fn, _cancel_fn, _window, _cx| {
                        let doc_key = doc_key.clone();
                        let session_key = session_key.clone();
                        vec![
                            Button::new("format-json")
                                .label("Format JSON")
                                .on_click({
                                    let json_state = json_state.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        let raw = json_state.read(cx).value().to_string();
                                        if let Ok(parsed) =
                                            serde_json::from_str::<serde_json::Value>(&raw)
                                            && let Ok(formatted) =
                                                serde_json::to_string_pretty(&parsed)
                                        {
                                            json_state.update(cx, |state, cx| {
                                                state.set_value(formatted, window, cx);
                                            });
                                        }
                                    }
                                })
                                .into_any_element(),
                            Button::new("update-json")
                                .primary()
                                .label("Update Draft")
                                .on_click({
                                    let json_state = json_state.clone();
                                    let view = view.clone();
                                    let session_key = session_key.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        let raw = json_state.read(cx).value().to_string();
                                        match parse_document_from_json(&raw) {
                                            Ok(doc) => {
                                                view.update(cx, |this, cx| {
                                                    this.state.update(cx, |state, cx| {
                                                        state.set_draft(
                                                            &session_key,
                                                            doc_key.clone(),
                                                            doc,
                                                        );
                                                        cx.notify();
                                                    });
                                                    this.view_model.clear_inline_edit();
                                                    this.view_model.rebuild_tree(&this.state, cx);
                                                    this.view_model
                                                        .sync_dirty_state(&this.state, cx);
                                                    cx.notify();
                                                });
                                                window.close_dialog(cx);
                                            }
                                            Err(err) => {
                                                log::warn!("Failed to parse JSON: {err}");
                                            }
                                        }
                                    }
                                })
                                .into_any_element(),
                            Button::new("cancel-json")
                                .label("Cancel")
                                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    window.close_dialog(cx);
                                })
                                .into_any_element(),
                        ]
                    }
                })
        });
    }

    pub(super) fn open_insert_document_json_editor(
        state: Entity<AppState>,
        session_key: SessionKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        let json_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .line_number(true)
                .searchable(true)
                .soft_wrap(true)
        });

        json_state.update(cx, |state, cx| {
            state.set_value("{}".to_string(), window, cx);
        });

        window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog
                .title("Insert Document")
                .min_w(px(720.0))
                .child(
                    div().flex().flex_col().gap(spacing::sm()).p(spacing::md()).child(
                        Input::new(&json_state)
                            .font_family(crate::theme::fonts::mono())
                            .h(px(420.0))
                            .w_full(),
                    ),
                )
                .footer({
                    let json_state = json_state.clone();
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_ok_fn, _cancel_fn, _window, _cx| {
                        vec![
                            Button::new("format-insert-json")
                                .label("Format JSON")
                                .on_click({
                                    let json_state = json_state.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        let raw = json_state.read(cx).value().to_string();
                                        if let Ok(parsed) =
                                            serde_json::from_str::<serde_json::Value>(&raw)
                                            && let Ok(formatted) =
                                                serde_json::to_string_pretty(&parsed)
                                        {
                                            json_state.update(cx, |state, cx| {
                                                state.set_value(formatted, window, cx);
                                            });
                                        }
                                    }
                                })
                                .into_any_element(),
                            Button::new("cancel-insert")
                                .label("Cancel")
                                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    window.close_dialog(cx);
                                })
                                .into_any_element(),
                            Button::new("confirm-insert")
                                .primary()
                                .label("Insert")
                                .on_click({
                                    let json_state = json_state.clone();
                                    let state = state.clone();
                                    let session_key = session_key.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        let raw = json_state.read(cx).value().to_string();
                                        match parse_document_from_json(&raw) {
                                            Ok(doc) => {
                                                AppCommands::insert_document(
                                                    state.clone(),
                                                    session_key.clone(),
                                                    doc,
                                                    cx,
                                                );
                                                window.close_dialog(cx);
                                            }
                                            Err(err) => {
                                                state.update(cx, |state, cx| {
                                                    state.status_message = Some(
                                                        StatusMessage::error(format!(
                                                            "Invalid JSON: {err}"
                                                        )),
                                                    );
                                                    cx.notify();
                                                });
                                            }
                                        }
                                    }
                                })
                                .into_any_element(),
                        ]
                    }
                })
        });
    }

    /// Render the header bar with collection title and action buttons.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_header(
        &self,
        collection_name: &str,
        db_name: &str,
        total: u64,
        session_key: Option<SessionKey>,
        selected_doc: Option<DocumentKey>,
        dirty_selected: bool,
        is_loading: bool,
        filter_state: Option<Entity<InputState>>,
        filter_active: bool,
        sort_state: Option<Entity<InputState>>,
        projection_state: Option<Entity<InputState>>,
        sort_active: bool,
        projection_active: bool,
        query_options_open: bool,
        active_subview: CollectionSubview,
        stats_loading: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        let state_for_refresh = self.state.clone();
        let state_for_apply = self.state.clone();
        let state_for_dialog = self.state.clone();
        let state_for_insert = self.state.clone();
        let state_for_filter = self.state.clone();
        let state_for_clear = self.state.clone();
        let state_for_delete = self.state.clone();
        let state_for_query = self.state.clone();
        let state_for_clear_query = self.state.clone();
        let state_for_toggle_options = self.state.clone();
        let state_for_subview = self.state.clone();
        let state_for_stats_refresh = self.state.clone();
        let state_for_indexes_refresh = self.state.clone();

        let options_label = if sort_active || projection_active {
            "Options •"
        } else {
            "Options"
        };
        let options_icon = Icon::new(if query_options_open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .xsmall();

        let is_documents = active_subview == CollectionSubview::Documents;
        let is_indexes = active_subview == CollectionSubview::Indexes;
        let is_stats = active_subview == CollectionSubview::Stats;

        let mut action_row = div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .child(
                div()
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child(db_name.to_string()),
            );

        if is_documents {
            action_row = action_row
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
                        .disabled(selected_doc.is_none() || session_key.is_none())
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
                        .disabled(!dirty_selected)
                        .on_click({
                            let selected_doc = selected_doc.clone();
                            let view = view.clone();
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                if let Some(doc_key) = selected_doc.clone() {
                                    view.update(cx, |this, cx| {
                                        if let Some(session_key) = this.view_model.current_session() {
                                            this.state.update(cx, |state, cx| {
                                                state.clear_draft(&session_key, &doc_key);
                                                cx.notify();
                                            });
                                        }
                                        this.view_model.clear_inline_edit();
                                        this.view_model.rebuild_tree(&this.state, cx);
                                        this.view_model.sync_dirty_state(&this.state, cx);
                                        cx.notify();
                                    });
                                }
                            }
                        }),
                )
                .child(
                    Button::new("apply")
                        .primary()
                        .compact()
                        .label("Apply")
                        .disabled(!dirty_selected)
                        .on_click({
                            let selected_doc = selected_doc.clone();
                            let state_for_apply = state_for_apply.clone();
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                let Some(doc_key) = selected_doc.clone() else {
                                    return;
                                };
                                let doc = {
                                    let state_ref = state_for_apply.read(cx);
                                    let session_key = state_ref.current_session_key();
                                    session_key
                                        .as_ref()
                                        .and_then(|session_key| state_ref.session(session_key))
                                        .and_then(|session| {
                                            session.view.drafts.get(&doc_key).cloned()
                                        })
                                };
                                if let Some(doc) = doc
                                    && let Some(session_key) =
                                        state_for_apply.read(cx).current_session_key()
                                {
                                    AppCommands::save_document(
                                        state_for_apply.clone(),
                                        session_key,
                                        doc_key.clone(),
                                        doc,
                                        cx,
                                    );
                                }
                            }
                        }),
                )
                .child({
                    let session_key = session_key.clone();
                    let selected_doc = selected_doc.clone();
                    let state_for_delete = state_for_delete.clone();
                    let delete_variant = ButtonCustomVariant::new(cx)
                        .color(colors::bg_button_danger().into())
                        .foreground(colors::text_button_danger().into())
                        .border(colors::bg_button_danger().into())
                        .hover(colors::bg_button_danger_hover().into())
                        .active(colors::bg_button_danger_hover().into())
                        .shadow(false);

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
                            let selected_doc = selected_doc.clone();
                            let state_for_delete = state_for_delete.clone();
                            move |menu: PopupMenu, _window, _cx| {
                                menu.item(
                                    PopupMenuItem::new("Delete selected")
                                        .disabled(selected_doc.is_none())
                                        .on_click({
                                            let selected_doc = selected_doc.clone();
                                            let session_key = session_key.clone();
                                            let state_for_delete = state_for_delete.clone();
                                            move |_, window, cx| {
                                                let Some(doc_key) = selected_doc.clone() else {
                                                    return;
                                                };
                                                let Some(session_key) = session_key.clone() else {
                                                    return;
                                                };
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
                                                        let doc_key = doc_key.clone();
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
                                                        .session(&session_key)
                                                        .and_then(|session| session.data.filter.clone())
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
                                            "Delete all documents in this collection? This cannot be undone.".to_string(),
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
                })
                .child(
                    Button::new("refresh")
                        .ghost()
                        .icon(Icon::new(IconName::Redo).xsmall())
                        .disabled(is_loading)
                        .on_click({
                            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                if let Some(session_key) =
                                    state_for_refresh.read(cx).current_session_key()
                                {
                                    AppCommands::load_documents_for_session(
                                        state_for_refresh.clone(),
                                        session_key,
                                        cx,
                                    );
                                }
                            }
                        }),
                );
        } else if is_indexes {
            action_row = action_row
                .child(
                    Button::new("create-index")
                        .compact()
                        .label("Create index")
                        .disabled(session_key.is_none())
                        .on_click({
                            let session_key = session_key.clone();
                            let state_for_dialog = state_for_dialog.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(session_key) = session_key.clone() else {
                                    return;
                                };
                                IndexCreateDialog::open(
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
                                    state_for_indexes_refresh.clone(),
                                    session_key,
                                    true,
                                    cx,
                                );
                            }
                        }),
                );
        } else if is_stats {
            action_row = action_row.child(
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
                            AppCommands::load_collection_stats(
                                state_for_stats_refresh.clone(),
                                session_key,
                                cx,
                            );
                        }
                    }),
            );
        }

        let subview_tabs = TabBar::new("collection-subview-tabs")
            .underline()
            .xsmall()
            .selected_index(active_subview.to_index())
            .on_click({
                let session_key = session_key.clone();
                move |index, _window, cx| {
                    let Some(session_key) = session_key.clone() else {
                        return;
                    };
                    let next = CollectionSubview::from_index(*index);
                    let should_load = state_for_subview.update(cx, |state, cx| {
                        let should_load = state.set_collection_subview(&session_key, next);
                        cx.notify();
                        should_load
                    });
                    if next == CollectionSubview::Indexes {
                        AppCommands::load_collection_indexes(
                            state_for_subview.clone(),
                            session_key,
                            false,
                            cx,
                        );
                    } else if should_load {
                        AppCommands::load_collection_stats(
                            state_for_subview.clone(),
                            session_key,
                            cx,
                        );
                    }
                }
            })
            .children(vec![
                Tab::new().label("Documents"),
                Tab::new().label("Indexes"),
                Tab::new().label("Stats"),
            ]);

        let mut root = div()
            .flex()
            .flex_col()
            .px(spacing::lg())
            .py(spacing::md())
            .gap(spacing::sm())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                window.blur();
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Icon::new(IconName::Folder)
                                    .small()
                                    .text_color(colors::accent_green()),
                            )
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(colors::text_primary())
                                    .font_family(crate::theme::fonts::heading())
                                    .child(collection_name.to_string()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors::text_muted())
                                    .child(format!("({} docs)", format_number(total))),
                            ),
                    )
                    .child(action_row),
            )
            .child(div().pl(spacing::xs()).child(subview_tabs));

        if is_documents {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(if let Some(filter_state) = filter_state.clone() {
                        div()
                            .flex_1()
                            .min_w(px(240.0))
                            .on_mouse_down(MouseButton::Left, |_, _window, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                Input::new(&filter_state)
                                    .font_family(crate::theme::fonts::mono())
                                    .w_full()
                                    .disabled(session_key.is_none()),
                            )
                            .into_any_element()
                    } else {
                        div().flex_1().into_any_element()
                    })
                    .child(
                        Button::new("apply-filter")
                            .compact()
                            .label("Filter")
                            .disabled(session_key.is_none())
                            .on_click({
                                let session_key = session_key.clone();
                                let filter_state = filter_state.clone();
                                let state_for_filter = state_for_filter.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let Some(session_key) = session_key.clone() else {
                                        return;
                                    };
                                    let Some(filter_state) = filter_state.clone() else {
                                        return;
                                    };
                                    CollectionView::apply_filter(
                                        state_for_filter.clone(),
                                        session_key,
                                        filter_state,
                                        window,
                                        cx,
                                    );
                                }
                            }),
                    )
                    .child(
                        Button::new("clear-filter")
                            .compact()
                            .label("Clear")
                            .disabled(session_key.is_none() || !filter_active)
                            .on_click({
                                let session_key = session_key.clone();
                                let filter_state = filter_state.clone();
                                let state_for_clear = state_for_clear.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let Some(session_key) = session_key.clone() else {
                                        return;
                                    };
                                    let Some(filter_state) = filter_state.clone() else {
                                        return;
                                    };
                                    filter_state.update(cx, |state, cx| {
                                        state.set_value("{}".to_string(), window, cx);
                                    });
                                    CollectionView::apply_filter(
                                        state_for_clear.clone(),
                                        session_key,
                                        filter_state,
                                        window,
                                        cx,
                                    );
                                }
                            }),
                    )
                    .child(
                        Button::new("toggle-options")
                            .ghost()
                            .compact()
                            .label(options_label)
                            .icon(options_icon)
                            .icon_right()
                            .disabled(session_key.is_none())
                            .on_click({
                                let session_key = session_key.clone();
                                let state_for_toggle = state_for_toggle_options.clone();
                                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    let Some(session_key) = session_key.clone() else {
                                        return;
                                    };
                                    state_for_toggle.update(cx, |state, cx| {
                                        let session = state.ensure_session(session_key.clone());
                                        session.view.query_options_open =
                                            !session.view.query_options_open;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            );

            if query_options_open {
                root = root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::sm())
                        .px(spacing::md())
                        .py(spacing::sm())
                        .bg(colors::bg_sidebar())
                        .border_1()
                        .border_color(colors::border_subtle())
                        .rounded(borders::radius_sm())
                        .child(render_query_option_row(
                            "Sort",
                            sort_state.clone(),
                            session_key.is_none(),
                        ))
                        .child(render_query_option_row(
                            "Project",
                            projection_state.clone(),
                            session_key.is_none(),
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap(spacing::sm())
                                .child(
                                    Button::new("apply-query")
                                        .compact()
                                        .label("Apply")
                                        .disabled(session_key.is_none())
                                        .on_click({
                                            let session_key = session_key.clone();
                                            let sort_state = sort_state.clone();
                                            let projection_state = projection_state.clone();
                                            let state_for_query = state_for_query.clone();
                                            move |_: &ClickEvent,
                                                  window: &mut Window,
                                                  cx: &mut App| {
                                                let Some(session_key) = session_key.clone()
                                                else {
                                                    return;
                                                };
                                                let Some(sort_state) = sort_state.clone() else {
                                                    return;
                                                };
                                                let Some(projection_state) =
                                                    projection_state.clone()
                                                else {
                                                    return;
                                                };
                                                CollectionView::apply_query_options(
                                                    state_for_query.clone(),
                                                    session_key,
                                                    sort_state,
                                                    projection_state,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                )
                                .child(
                                    Button::new("clear-query")
                                        .compact()
                                        .label("Clear")
                                        .disabled(
                                            session_key.is_none()
                                                || (!sort_active && !projection_active),
                                        )
                                        .on_click({
                                            let session_key = session_key.clone();
                                            let sort_state = sort_state.clone();
                                            let projection_state = projection_state.clone();
                                            let state_for_clear_query =
                                                state_for_clear_query.clone();
                                            move |_: &ClickEvent,
                                                  window: &mut Window,
                                                  cx: &mut App| {
                                                let Some(session_key) = session_key.clone()
                                                else {
                                                    return;
                                                };
                                                let Some(sort_state) = sort_state.clone() else {
                                                    return;
                                                };
                                                let Some(projection_state) =
                                                    projection_state.clone()
                                                else {
                                                    return;
                                                };
                                                sort_state.update(cx, |state, cx| {
                                                    state.set_value("{}".to_string(), window, cx);
                                                });
                                                projection_state.update(cx, |state, cx| {
                                                    state.set_value("{}".to_string(), window, cx);
                                                });
                                                CollectionView::apply_query_options(
                                                    state_for_clear_query.clone(),
                                                    session_key,
                                                    sort_state,
                                                    projection_state,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                ),
                        ),
                );
            }
        }

        root
    }

    pub(super) fn render_stats_row(
        stats: Option<CollectionStats>,
        stats_loading: bool,
        stats_error: Option<String>,
        session_key: Option<SessionKey>,
        state: Entity<AppState>,
    ) -> AnyElement {
        let mut row = div()
            .flex()
            .items_center()
            .gap(spacing::lg())
            .px(spacing::md())
            .py(spacing::sm())
            .bg(colors::bg_header())
            .border_t_1()
            .border_color(colors::border());

        if stats_loading {
            row = row
                .child(Spinner::new().small())
                .child(div().text_sm().text_color(colors::text_muted()).child("Loading stats..."));
            return row.into_any_element();
        }

        if let Some(error) = stats_error {
            row = row.child(div().text_sm().text_color(colors::text_error()).child(error)).child(
                Button::new("retry-stats")
                    .ghost()
                    .compact()
                    .label("Retry")
                    .disabled(session_key.is_none())
                    .on_click({
                        let state = state.clone();
                        let session_key = session_key.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            AppCommands::load_collection_stats(state.clone(), session_key, cx);
                        }
                    }),
            );
            return row.into_any_element();
        }

        let Some(stats) = stats else {
            row = row.child(
                div().text_sm().text_color(colors::text_muted()).child("No stats available"),
            );
            return row.into_any_element();
        };

        row = row
            .child(stat_cell("Documents", format_number(stats.document_count)))
            .child(stat_cell("Avg size", format_bytes(stats.avg_obj_size)))
            .child(stat_cell("Data size", format_bytes(stats.data_size)))
            .child(stat_cell("Storage", format_bytes(stats.storage_size)))
            .child(stat_cell("Index size", format_bytes(stats.total_index_size)))
            .child(stat_cell("Indexes", format_number(stats.index_count)))
            .child(stat_cell(
                "Capped",
                if stats.capped { "Yes".to_string() } else { "No".to_string() },
            ));

        if let Some(max_size) = stats.max_size {
            row = row.child(stat_cell("Max size", format_bytes(max_size)));
        }

        row.into_any_element()
    }
}

fn stat_cell(label: &str, value: String) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(div().text_sm().text_color(colors::text_primary()).child(value))
        .into_any_element()
}

fn render_query_option_row(
    label: &str,
    state: Option<Entity<InputState>>,
    disabled: bool,
) -> AnyElement {
    let Some(state) = state else {
        return div().into_any_element();
    };

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            div()
                .w(px(72.0))
                .text_sm()
                .text_color(colors::text_muted())
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .on_mouse_down(MouseButton::Left, |_, _window, cx| {
                    cx.stop_propagation();
                })
                .child(
                    Input::new(&state)
                        .font_family(crate::theme::fonts::mono())
                        .w_full()
                        .disabled(disabled),
                ),
        )
        .into_any_element()
}

fn parse_optional_doc(
    raw: &str,
) -> Result<(String, Option<mongodb::bson::Document>), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return Ok((String::new(), None));
    }

    match parse_document_from_json(trimmed) {
        Ok(doc) => Ok((trimmed.to_string(), Some(doc))),
        Err(err) => Err(err.to_string()),
    }
}
