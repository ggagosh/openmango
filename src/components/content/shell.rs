use gpui::*;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _};

use crate::components::Button;
use crate::state::AppState;
use crate::theme::{colors, spacing};

pub(crate) fn render_shell(
    error_text: Option<String>,
    state: Entity<AppState>,
    content: impl IntoElement,
    with_background: bool,
    cx: &App,
) -> AnyElement {
    let mut root = div().flex().flex_col().flex_1().h_full().min_h(px(0.0));
    if with_background {
        root = root.bg(cx.theme().background);
    }
    if let Some(text) = error_text {
        root = root.child(render_error_banner(text, state.clone(), cx));
    }
    root.child(content).into_any_element()
}

fn format_error_banner_preview(message: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 100;

    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in normalized.chars().enumerate() {
        if idx >= MAX_PREVIEW_CHARS {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn render_error_banner(message: String, state: Entity<AppState>, cx: &App) -> AnyElement {
    let preview = format_error_banner_preview(&message);
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::md())
        .w_full()
        .px(spacing::md())
        .py(spacing::sm())
        .bg(colors::bg_error(cx))
        .border_b_1()
        .border_color(cx.theme().danger)
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .flex_1()
                .min_w(px(0.0))
                .child(Icon::new(IconName::TriangleAlert).xsmall().text_color(cx.theme().danger))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .truncate()
                        .child(preview),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .flex_shrink_0()
                .child(Button::new("show-error").ghost().compact().label("Show more").on_click({
                    let message = message.clone();
                    move |_, window, cx| {
                        let message = message.clone();
                        let text_state = cx.new(|cx| {
                            InputState::new(window, cx).code_editor("text").soft_wrap(true)
                        });
                        text_state.update(cx, |state, cx| {
                            state.set_value(message.clone(), window, cx);
                        });
                        window.open_dialog(cx, move |dialog: Dialog, _window, _cx| {
                            dialog
                                .title("Error details")
                                .min_w(px(720.0))
                                .child(
                                    div().p(spacing::md()).child(
                                        Input::new(&text_state)
                                            .font_family(crate::theme::fonts::mono())
                                            .h(px(320.0))
                                            .w_full()
                                            .disabled(true),
                                    ),
                                )
                                .footer({
                                    let message = message.clone();
                                    move |_ok_fn, _cancel_fn, _window, _cx| {
                                        vec![
                                            Button::new("copy-error")
                                                .label("Copy")
                                                .on_click({
                                                    let message = message.clone();
                                                    move |_, _window, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(
                                                                message.clone(),
                                                            ),
                                                        );
                                                    }
                                                })
                                                .into_any_element(),
                                            Button::new("close-error")
                                                .label("Close")
                                                .on_click(|_, window, cx| {
                                                    window.close_dialog(cx);
                                                })
                                                .into_any_element(),
                                        ]
                                    }
                                })
                        });
                    }
                }))
                .child(
                    Button::new("dismiss-error")
                        .ghost()
                        .icon(Icon::new(IconName::Close).xsmall())
                        .on_click({
                            let state = state.clone();
                            move |_, _window, cx| {
                                state.update(cx, |state, cx| {
                                    state.clear_status_message();
                                    cx.notify();
                                });
                            }
                        }),
                ),
        )
        .into_any_element()
}
