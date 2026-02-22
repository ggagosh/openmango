//! Connection import flow.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};

use crate::components::file_picker::{FileFilter, FilePickerMode, open_file_dialog_async};
use crate::components::{Button, cancel_button};
use crate::helpers::connection_io::{self, ExportMode};
use crate::state::AppState;
use crate::state::status::StatusMessage;
use crate::theme::spacing;

pub fn open_import_flow(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let window_handle = window.window_handle();
    let state_clone = state.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let path = open_file_dialog_async(
            FilePickerMode::Open,
            vec![FileFilter::connections_json(), FileFilter::all()],
            None,
        )
        .await;

        let Some(path) = path else {
            return;
        };

        let json = match std::fs::read_to_string(&path) {
            Ok(j) => j,
            Err(e) => {
                let _ = cx.update(|cx| {
                    state_clone.update(cx, |state, _cx| {
                        state.set_status_message(Some(StatusMessage::error(format!(
                            "Failed to read file: {e}"
                        ))));
                    });
                });
                return;
            }
        };

        let file = match connection_io::parse_import(&json) {
            Ok(f) => f,
            Err(e) => {
                let _ = cx.update(|cx| {
                    state_clone.update(cx, |state, _cx| {
                        state.set_status_message(Some(StatusMessage::error(format!(
                            "Invalid export file: {e}"
                        ))));
                    });
                });
                return;
            }
        };

        if file.mode == ExportMode::Encrypted {
            let _ = cx.update_window(window_handle, |_root, window, cx| {
                open_passphrase_dialog(state_clone.clone(), file, window, cx);
            });
        } else {
            let _ = cx.update(|cx| {
                finish_import(state_clone.clone(), &file, cx);
            });
        }
    })
    .detach();
}

fn open_passphrase_dialog(
    state: Entity<AppState>,
    file: connection_io::ConnectionExportFile,
    window: &mut Window,
    cx: &mut App,
) {
    let input_state =
        cx.new(|cx| InputState::new(window, cx).placeholder("Enter passphrase").masked(true));

    let file = std::rc::Rc::new(std::cell::RefCell::new(file));

    window.open_dialog(cx, {
        let input_state = input_state.clone();
        let state = state.clone();
        let file = file.clone();
        move |dialog: Dialog, window: &mut Window, cx: &mut App| {
            input_state.update(cx, |s, cx| s.focus(window, cx));
            dialog
                .title("Enter Import Passphrase")
                .w(px(420.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::sm())
                        .p(spacing::md())
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                            "This file contains encrypted passwords. \
                                     Enter the passphrase to decrypt them.",
                        ))
                        .child(Input::new(&input_state).w_full()),
                )
                .footer({
                    let input_state = input_state.clone();
                    let state = state.clone();
                    let file = file.clone();
                    move |_ok, _cancel, _window, _cx| {
                        vec![
                            cancel_button("cancel-import-passphrase"),
                            Button::new("decrypt-import")
                                .primary()
                                .label("Decrypt & Import")
                                .on_click({
                                    let input_state = input_state.clone();
                                    let state = state.clone();
                                    let file = file.clone();
                                    move |_, window, cx| {
                                        let passphrase = input_state.read(cx).value().to_string();
                                        if passphrase.is_empty() {
                                            return;
                                        }

                                        let mut file_mut = file.borrow_mut();
                                        if let Err(e) = connection_io::decrypt_import_file(
                                            &mut file_mut,
                                            &passphrase,
                                        ) {
                                            drop(file_mut);
                                            state.update(cx, |state, _cx| {
                                                state.set_status_message(Some(
                                                    StatusMessage::error(format!(
                                                        "Decryption failed: {e}"
                                                    )),
                                                ));
                                            });
                                            window.close_dialog(cx);
                                            return;
                                        }

                                        finish_import(state.clone(), &file_mut, cx);
                                        drop(file_mut);
                                        window.close_dialog(cx);
                                    }
                                })
                                .into_any_element(),
                        ]
                    }
                })
        }
    });
}

fn finish_import(
    state: Entity<AppState>,
    file: &connection_io::ConnectionExportFile,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        let existing = state.connections_snapshot();
        let imported = connection_io::resolve_import(file, &existing);
        let count = imported.len();
        let is_redacted = file.mode == ExportMode::Redacted;

        for conn in imported {
            state.add_connection(conn, cx);
        }

        let message = if is_redacted {
            format!(
                "Imported {count} connection{} (passwords not included)",
                if count == 1 { "" } else { "s" }
            )
        } else {
            format!("Imported {count} connection{}", if count == 1 { "" } else { "s" })
        };
        state.set_status_message(Some(StatusMessage::info(message)));
    });
}
