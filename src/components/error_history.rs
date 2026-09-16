//! Error notifications and the session's error history.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::*;

use crate::components::{Button, ErrorCallout};
use crate::state::{AppCommands, AppState, ErrorAction, ErrorEntry};
use crate::theme::spacing;

pub(crate) struct ErrorToast;

/// One notification slot per connection for Reconnect, otherwise one per distinct error.
pub(crate) fn toast_key(entry: &ErrorEntry) -> SharedString {
    match &entry.action {
        Some(ErrorAction::Reconnect(connection_id)) => format!("reconnect-{connection_id}").into(),
        _ => format!("{}\n{}", entry.report.title, entry.report.message).into(),
    }
}

/// A notification for an error with no place of its own on screen.
/// Errors with a fix, and startup failures, stay until handled; the rest hide after a few seconds
/// and stay in history.
pub fn error_notification(entry: &ErrorEntry, state: Entity<AppState>) -> Notification {
    let report = entry.report.clone();
    let (title, message) = match report.title.is_empty() {
        true => (report.message.clone(), None),
        false => (report.title.clone(), Some(report.one_line())),
    };
    let mut notification = Notification::error(message.unwrap_or_default())
        .title(title)
        .id1::<ErrorToast>(toast_key(entry));
    let action = entry.action.clone();
    let autohide = action.is_none() && !entry.sticky;
    notification = notification.action(move |_, _, cx| {
        let label = match &action {
            Some(ErrorAction::Reconnect(_)) => "Reconnect",
            Some(ErrorAction::ReloadDocuments(_)) => "Reload",
            None => "Copy",
        };
        let (state, action, copy_text) = (state.clone(), action.clone(), report.copy_text());
        Button::new("error-toast-action").xsmall().label(label).on_click(cx.listener(
            move |notification, _, window, cx| {
                match action.clone() {
                    // An old notification must not disconnect a connection that's back up.
                    Some(ErrorAction::Reconnect(connection_id)) => {
                        if !state.read(cx).is_connected(connection_id) {
                            AppCommands::connect(state.clone(), connection_id, cx);
                        }
                    }
                    Some(ErrorAction::ReloadDocuments(key)) => {
                        let state = state.clone();
                        crate::components::request_unsaved_action(
                            state.clone(),
                            crate::state::UnsavedScope::Preview(key.clone()),
                            window,
                            cx,
                            move |_, cx| AppCommands::load_documents_for_session(state, key, cx),
                        );
                    }
                    None => cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone())),
                }
                notification.dismiss(window, cx);
            },
        ))
    });
    notification.autohide(autohide)
}

pub fn open_error_history(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.mark_errors_seen();
        cx.notify();
    });
    window.open_dialog(cx, move |dialog: Dialog, _window, cx| {
        let entries: Vec<ErrorEntry> = state.read(cx).error_entries().cloned().collect();
        let body = if entries.is_empty() {
            div()
                .py(spacing::lg())
                .text_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No errors this session.")
                .into_any_element()
        } else {
            div()
                .id("error-history-list")
                .flex()
                .flex_col()
                .gap(spacing::sm())
                .max_h(px(520.0))
                .overflow_y_scroll()
                .children(entries.into_iter().map(|entry| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(entry.at.format("%H:%M:%S").to_string()),
                        )
                        .child(
                            ErrorCallout::new(format!("error-history-{}", entry.id), entry.report)
                                .state(state.clone()),
                        )
                }))
                .into_any_element()
        };
        let has_entries = state.read(cx).error_count() > 0;
        dialog
            .title("Errors this session")
            .w(px(640.0))
            .child(div().p(spacing::md()).child(body))
            .footer(gpui_kit::component::dialog::DialogFooter::new().children(vec![
                Button::new("error-history-clear")
                    .ghost()
                    .label("Clear")
                    .disabled(!has_entries)
                    .on_click({
                        let state = state.clone();
                        move |_, _, cx| {
                            state.update(cx, |state, cx| {
                                state.clear_errors();
                                cx.notify();
                            })
                        }
                    })
                    .into_any_element(),
                Button::new("error-history-close")
                    .label("Close")
                    .on_click(|_, window, cx| window.close_dialog(cx))
                    .into_any_element(),
            ]))
    });
}
