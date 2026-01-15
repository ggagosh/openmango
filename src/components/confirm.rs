use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;

use crate::components::Button;
use crate::theme::{colors, spacing};

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

    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
        dialog
            .title(title.clone())
            .min_w(px(420.0))
            .child(
                div().flex().flex_col().gap(spacing::sm()).p(spacing::md()).child(
                    div().text_sm().text_color(colors::text_secondary()).child(message.clone()),
                ),
            )
            .footer({
                let confirm_label = confirm_label.clone();
                let on_confirm = on_confirm.clone();
                move |_ok_fn, _cancel_fn, _window, _cx| {
                    let on_confirm = on_confirm.clone();
                    vec![
                        Button::new("cancel-confirm")
                            .label("Cancel")
                            .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        if destructive {
                            Button::new("confirm-action")
                                .danger()
                                .label(confirm_label.clone())
                                .on_click({
                                    let on_confirm = on_confirm.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        if let Some(on_confirm) = on_confirm.borrow_mut().take() {
                                            on_confirm(window, cx);
                                        }
                                        window.close_dialog(cx);
                                    }
                                })
                                .into_any_element()
                        } else {
                            Button::new("confirm-action")
                                .primary()
                                .label(confirm_label.clone())
                                .on_click({
                                    let on_confirm = on_confirm.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        if let Some(on_confirm) = on_confirm.borrow_mut().take() {
                                            on_confirm(window, cx);
                                        }
                                        window.close_dialog(cx);
                                    }
                                })
                                .into_any_element()
                        },
                    ]
                }
            })
    });
}
