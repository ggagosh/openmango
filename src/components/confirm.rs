use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;

use crate::components::Button;
use crate::theme::spacing;

#[derive(Default)]
struct ConfirmDialogState {
    focused_once: bool,
}

fn close_dialog_and_restore_focus(
    window: &mut Window,
    cx: &mut App,
    previous_focus: Option<FocusHandle>,
) {
    window.close_dialog(cx);
    if let Some(previous_focus) = previous_focus {
        window.defer(cx, move |window, _cx| window.focus(&previous_focus));
    }
}

pub fn open_confirm_dialog(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    confirm_label: impl Into<SharedString>,
    destructive: bool,
    on_confirm: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let title: SharedString = title.into();
    let message: SharedString = message.into();
    let confirm_label: SharedString = confirm_label.into();
    let on_confirm = Rc::new(RefCell::new(Some(on_confirm)));
    let previous_focus = window.focused(cx);
    let cancel_focus = cx.focus_handle().tab_index(0).tab_stop(true);
    let confirm_focus = cx.focus_handle().tab_index(1).tab_stop(true);

    window.open_dialog(cx, move |dialog: Dialog, window: &mut Window, cx: &mut App| {
        let dialog_state = window.use_keyed_state("confirm-dialog-focus", cx, |_window, _cx| {
            ConfirmDialogState::default()
        });
        let key_cancel_focus = cancel_focus.clone();
        let key_confirm_focus = confirm_focus.clone();
        let key_previous_focus = previous_focus.clone();
        let key_on_confirm = on_confirm.clone();

        let key_handler = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let key = event.keystroke.key.to_ascii_lowercase();
            if key == "escape" {
                cx.stop_propagation();
                close_dialog_and_restore_focus(window, cx, key_previous_focus.clone());
                return;
            }
            if key == "enter" || key == "return" {
                cx.stop_propagation();
                if key_confirm_focus.is_focused(window)
                    && let Some(on_confirm) = key_on_confirm.borrow_mut().take()
                {
                    on_confirm(window, cx);
                }
                close_dialog_and_restore_focus(window, cx, key_previous_focus.clone());
            }
        };

        let confirm_button = if destructive {
            Button::new("confirm-action")
                .danger()
                .label(confirm_label.clone())
                .track_focus(&confirm_focus)
                .tab_index(1)
                .on_click({
                    let on_confirm = on_confirm.clone();
                    let previous_focus = previous_focus.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        if let Some(on_confirm) = on_confirm.borrow_mut().take() {
                            on_confirm(window, cx);
                        }
                        close_dialog_and_restore_focus(window, cx, previous_focus.clone());
                    }
                })
        } else {
            Button::new("confirm-action")
                .primary()
                .label(confirm_label.clone())
                .track_focus(&confirm_focus)
                .tab_index(1)
                .on_click({
                    let on_confirm = on_confirm.clone();
                    let previous_focus = previous_focus.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        if let Some(on_confirm) = on_confirm.borrow_mut().take() {
                            on_confirm(window, cx);
                        }
                        close_dialog_and_restore_focus(window, cx, previous_focus.clone());
                    }
                })
        };

        // Default focus to cancel so Enter is a safe action.
        let should_focus_cancel = !dialog_state.read(cx).focused_once;
        if should_focus_cancel {
            dialog_state.update(cx, |state, _cx| state.focused_once = true);
            let cancel_focus = key_cancel_focus.clone();
            window.defer(cx, move |window, _cx| {
                window.focus(&cancel_focus);
            });
        }

        dialog.title(title.clone()).min_w(px(420.0)).keyboard(false).child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .p(spacing::md())
                .on_key_down(key_handler)
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().secondary_foreground)
                        .child(message.clone()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(spacing::xs())
                        .child(
                            Button::new("cancel-confirm")
                                .label("Cancel")
                                .track_focus(&key_cancel_focus)
                                .tab_index(0)
                                .on_click({
                                    let previous_focus = previous_focus.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        close_dialog_and_restore_focus(
                                            window,
                                            cx,
                                            previous_focus.clone(),
                                        );
                                    }
                                }),
                        )
                        .child(confirm_button),
                ),
        )
    });
}
