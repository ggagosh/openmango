use gpui::*;
use gpui_component::WindowExt;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};

use crate::bson::{
    DocumentKey, document_to_shell_string, format_relaxed_json_value, parse_document_from_json,
    parse_value_from_relaxed_json,
};
use crate::components::{Button, cancel_button};
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};
use crate::theme::spacing;

use super::super::CollectionView;
use super::index_create::IndexCreateDialog;

impl CollectionView {
    pub(crate) fn open_document_json_editor(
        view: Entity<CollectionView>,
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        let doc = {
            let state_ref = state.read(cx);
            state_ref.session_draft_or_document(&session_key, &doc_key)
        };
        let Some(doc) = doc else {
            return;
        };

        let json_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("javascript")
                .line_number(true)
                .searchable(true)
                .soft_wrap(true)
        });

        let json_text = document_to_shell_string(&doc);
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
                                        if let Ok(value) = parse_value_from_relaxed_json(&raw) {
                                            let formatted = format_relaxed_json_value(&value);
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
                            cancel_button("cancel-json"),
                        ]
                    }
                })
        });
    }

    pub(crate) fn open_insert_document_json_editor(
        state: Entity<AppState>,
        session_key: SessionKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        let json_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("javascript")
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
                                        if let Ok(value) = parse_value_from_relaxed_json(&raw) {
                                            let formatted = format_relaxed_json_value(&value);
                                            json_state.update(cx, |state, cx| {
                                                state.set_value(formatted, window, cx);
                                            });
                                        }
                                    }
                                })
                                .into_any_element(),
                            cancel_button("cancel-insert"),
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
                                                    state.set_status_message(Some(
                                                        StatusMessage::error(format!(
                                                            "Invalid JSON: {err}"
                                                        )),
                                                    ));
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

    pub(crate) fn open_index_create_dialog(
        state: Entity<AppState>,
        session_key: SessionKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        IndexCreateDialog::open(state, session_key, window, cx);
    }
}
