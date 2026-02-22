//! Export connections dialog.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::checkbox::Checkbox;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement;

use crate::components::file_picker::{FileFilter, FilePickerMode, open_file_dialog_async};
use crate::components::{Button, cancel_button};
use crate::helpers::connection_io::{self, ExportMode};
use crate::helpers::extract_host_from_uri;
use crate::models::SavedConnection;
use crate::state::AppState;
use crate::state::status::StatusMessage;
use crate::theme::{borders, spacing};

struct ExportDialogState {
    app_state: Entity<AppState>,
    connections: Vec<SavedConnection>,
    selected: Vec<bool>,
    mode: ExportMode,
    passphrase_state: Entity<InputState>,
    confirm_state: Entity<InputState>,
}

impl ExportDialogState {
    fn new(
        app_state: Entity<AppState>,
        connections: Vec<SavedConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let count = connections.len();
        Self {
            app_state,
            connections,
            selected: vec![true; count],
            mode: ExportMode::Redacted,
            passphrase_state: cx
                .new(|cx| InputState::new(window, cx).placeholder("Passphrase").masked(true)),
            confirm_state: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Confirm passphrase").masked(true)
            }),
        }
    }
}

impl Render for ExportDialogState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let all_selected = self.selected.iter().all(|&s| s);
        let none_selected = self.selected.iter().all(|&s| !s);
        let count = self.connections.len();
        let mode = self.mode;

        div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .p(spacing::md())
            // Connection checklist
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().secondary_foreground)
                                    .child("Select connections to export"),
                            )
                            .child({
                                let view = view.clone();
                                Button::new("toggle-all-export")
                                    .compact()
                                    .ghost()
                                    .label(if all_selected { "Deselect All" } else { "Select All" })
                                    .on_click(move |_, _, cx| {
                                        let new_val = !all_selected;
                                        view.update(cx, |this, cx| {
                                            for i in 0..count {
                                                this.selected[i] = new_val;
                                            }
                                            cx.notify();
                                        });
                                    })
                            }),
                    )
                    .child(
                        div()
                            .max_h(px(200.0))
                            .overflow_y_scrollbar()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(borders::radius_sm())
                            .p(spacing::xs())
                            .child(div().flex().flex_col().gap(px(2.0)).children(
                                self.connections.iter().enumerate().map(|(i, conn)| {
                                    let is_checked = self.selected[i];
                                    let host = extract_host_from_uri(&conn.uri)
                                        .unwrap_or_else(|| "Unknown".to_string());
                                    let view = view.clone();
                                    div()
                                        .id(ElementId::NamedInteger("export-row".into(), i as u64))
                                        .flex()
                                        .items_center()
                                        .gap(spacing::sm())
                                        .px(spacing::xs())
                                        .py(px(2.0))
                                        .cursor_pointer()
                                        .rounded(borders::radius_sm())
                                        .hover(|s| s.bg(cx.theme().list_hover))
                                        .on_click({
                                            let view = view.clone();
                                            move |_, _, cx| {
                                                view.update(cx, |this, cx| {
                                                    this.selected[i] = !this.selected[i];
                                                    cx.notify();
                                                });
                                            }
                                        })
                                        .child(
                                            Checkbox::new(ElementId::NamedInteger(
                                                "export-check".into(),
                                                i as u64,
                                            ))
                                            .checked(is_checked)
                                            .on_click({
                                                let view = view.clone();
                                                move |_, _, cx| {
                                                    view.update(cx, |this, cx| {
                                                        this.selected[i] = !this.selected[i];
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(cx.theme().foreground)
                                                        .child(conn.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(host),
                                                ),
                                        )
                                        .into_any_element()
                                }),
                            )),
                    )
                    .when(none_selected, |s| {
                        s.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child("Select at least one connection"),
                        )
                    }),
            )
            // Mode selector
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().secondary_foreground)
                            .child("Export mode"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(spacing::xs())
                            .child(mode_button(
                                "Redacted",
                                ExportMode::Redacted,
                                mode,
                                view.clone(),
                            ))
                            .child(mode_button(
                                "Encrypted",
                                ExportMode::Encrypted,
                                mode,
                                view.clone(),
                            ))
                            .child(mode_button(
                                "Plaintext",
                                ExportMode::Plaintext,
                                mode,
                                view.clone(),
                            )),
                    )
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                        match mode {
                            ExportMode::Redacted => "Passwords replaced with *****, safe to share",
                            ExportMode::Encrypted => "Passwords encrypted with a passphrase",
                            ExportMode::Plaintext => "Full URIs with passwords included",
                        },
                    )),
            )
            // Passphrase fields (only visible when Encrypted mode)
            .when(mode == ExportMode::Encrypted, |s| {
                s.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::sm())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().secondary_foreground)
                                .child("Encryption passphrase"),
                        )
                        .child(Input::new(&self.passphrase_state).w_full())
                        .child(Input::new(&self.confirm_state).w_full()),
                )
            })
    }
}

fn mode_button(
    label: &'static str,
    target: ExportMode,
    current: ExportMode,
    view: Entity<ExportDialogState>,
) -> Button {
    let is_active = current == target;
    let mut btn = Button::new(SharedString::new_static(label)).compact().label(label);
    if is_active {
        btn = btn.active_style(gpui::hsla(0.0, 0.0, 0.25, 1.0));
    } else {
        btn = btn.ghost();
    }
    btn.on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
            this.mode = target;
            cx.notify();
        });
    })
}

pub fn open_export_dialog(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let connections = state.read(cx).connections_snapshot();
    if connections.is_empty() {
        state.update(cx, |state, _cx| {
            state.set_status_message(Some(StatusMessage::info("No connections to export")));
        });
        return;
    }

    let dialog_state = cx.new(|cx| ExportDialogState::new(state.clone(), connections, window, cx));

    window.open_dialog(cx, {
        let dialog_state = dialog_state.clone();
        move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
            dialog.title("Export Connections").w(px(520.0)).child(dialog_state.clone()).footer({
                let dialog_state = dialog_state.clone();
                move |_ok, _cancel, _window, _cx| {
                    vec![cancel_button("cancel-export"), render_export_button(dialog_state.clone())]
                }
            })
        }
    });
}

fn render_export_button(dialog_state: Entity<ExportDialogState>) -> AnyElement {
    Button::new("do-export")
        .primary()
        .label("Export")
        .on_click(move |_, window, cx| {
            let ds = dialog_state.read(cx);
            let chosen: Vec<SavedConnection> = ds
                .connections
                .iter()
                .enumerate()
                .filter(|(i, _)| ds.selected[*i])
                .map(|(_, c)| c.clone())
                .collect();

            if chosen.is_empty() {
                return;
            }

            let current_mode = ds.mode;
            let app_state = ds.app_state.clone();

            let passphrase = if current_mode == ExportMode::Encrypted {
                let pw = ds.passphrase_state.read(cx).value().to_string();
                let confirm = ds.confirm_state.read(cx).value().to_string();
                if pw.is_empty() {
                    app_state.update(cx, |state, _cx| {
                        state.set_status_message(Some(StatusMessage::error(
                            "Passphrase is required for encrypted export",
                        )));
                    });
                    return;
                }
                if pw != confirm {
                    app_state.update(cx, |state, _cx| {
                        state.set_status_message(Some(StatusMessage::error(
                            "Passphrases do not match",
                        )));
                    });
                    return;
                }
                Some(pw)
            } else {
                None
            };

            let export_file =
                match connection_io::build_export(&chosen, current_mode, passphrase.as_deref()) {
                    Ok(f) => f,
                    Err(e) => {
                        app_state.update(cx, |state, _cx| {
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Export failed: {e}"
                            ))));
                        });
                        return;
                    }
                };

            let json = match serde_json::to_string_pretty(&export_file) {
                Ok(j) => j,
                Err(e) => {
                    app_state.update(cx, |state, _cx| {
                        state.set_status_message(Some(StatusMessage::error(format!(
                            "Serialization failed: {e}"
                        ))));
                    });
                    return;
                }
            };

            let count = chosen.len();
            window.close_dialog(cx);

            cx.spawn(async move |cx: &mut AsyncApp| {
                let path = open_file_dialog_async(
                    FilePickerMode::Save,
                    vec![FileFilter::connections_json(), FileFilter::all()],
                    Some("openmango-connections.json".to_string()),
                )
                .await;

                if let Some(path) = path {
                    if let Err(e) = std::fs::write(&path, &json) {
                        let _ = cx.update(|cx| {
                            app_state.update(cx, |state, _cx| {
                                state.set_status_message(Some(StatusMessage::error(format!(
                                    "Failed to write file: {e}"
                                ))));
                            });
                        });
                        return;
                    }
                    let _ = cx.update(|cx| {
                        app_state.update(cx, |state, _cx| {
                            state.set_status_message(Some(StatusMessage::info(format!(
                                "Exported {count} connection{}",
                                if count == 1 { "" } else { "s" }
                            ))));
                        });
                    });
                }
            })
            .detach();
        })
        .into_any_element()
}
