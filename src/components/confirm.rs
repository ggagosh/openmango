use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use uuid::Uuid;

use crate::components::{Button, ConnectionIdentity, connection_identity_badge};
use crate::models::ConnectionWriteIdentity;
use crate::state::{AppState, StatusMessage};
use crate::theme::spacing;

#[derive(Debug, Clone)]
pub struct WriteConfirmation {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub destructive: bool,
}

#[derive(Debug, Clone)]
pub struct WriteRequest {
    pub connection_id: Uuid,
    pub target: String,
    pub operation: String,
    pub confirmation: Option<WriteConfirmation>,
    authorization_uses: usize,
}

impl WriteRequest {
    pub fn new(
        connection_id: Uuid,
        target: impl Into<String>,
        operation: impl Into<String>,
        confirmation: Option<WriteConfirmation>,
    ) -> Self {
        Self {
            connection_id,
            target: target.into(),
            operation: operation.into(),
            confirmation,
            authorization_uses: 1,
        }
    }

    pub fn for_writes(mut self, uses: usize) -> Self {
        self.authorization_uses = uses.max(1);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRequestDecision {
    BlockReadOnly,
    Confirm,
    Proceed,
}

pub fn write_request_decision(
    read_only: bool,
    ordinary_confirmation: bool,
    protected_production: bool,
) -> WriteRequestDecision {
    if read_only {
        WriteRequestDecision::BlockReadOnly
    } else if ordinary_confirmation || protected_production {
        WriteRequestDecision::Confirm
    } else {
        WriteRequestDecision::Proceed
    }
}

pub fn request_connection_write(
    state: Entity<AppState>,
    request: WriteRequest,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let WriteRequest { connection_id, target, operation, confirmation, authorization_uses } =
        request;
    let Some(connection) = state.read(cx).connection_by_id(connection_id).cloned() else {
        state.update(cx, |state, cx| {
            state.set_status_message(Some(StatusMessage::error(
                "Write blocked because the connection no longer exists.",
            )));
            cx.notify();
        });
        return;
    };
    let identity = ConnectionIdentity::from(&connection);
    let snapshot = ConnectionWriteIdentity::from(&connection);
    let protected = connection.requires_production_write_confirmation();
    match write_request_decision(
        state.read(cx).connection_read_only(connection_id),
        confirmation.is_some(),
        protected,
    ) {
        WriteRequestDecision::BlockReadOnly => {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Read-only connection: writes are disabled.",
                )));
                cx.notify();
            });
        }
        WriteRequestDecision::Proceed => on_confirm(window, cx),
        WriteRequestDecision::Confirm => {
            let confirmation = confirmation.unwrap_or_else(|| WriteConfirmation {
                title: "Confirm Production write".into(),
                message: format!("{operation}."),
                confirm_label: "Continue".into(),
                destructive: true,
            });
            let confirmation = WriteConfirmation {
                message: format!(
                    "{}\n\nConnection: {}\nEnvironment: {}\nTarget: {target}",
                    confirmation.message,
                    identity.name,
                    identity.environment_label().unwrap_or("Not set")
                ),
                ..confirmation
            };
            let state_for_confirm = state.clone();
            open_confirm_dialog_boxed(
                window,
                cx,
                confirmation,
                Some(identity.clone()),
                Box::new(move |window, cx| {
                    let allowed = state_for_confirm
                        .read(cx)
                        .connection_by_id(connection_id)
                        .is_some_and(|connection| {
                            snapshot.matches(connection)
                                && !state_for_confirm.read(cx).connection_read_only(connection_id)
                        });
                    if !allowed {
                        state_for_confirm.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::error(
                                "Write blocked because the connection identity changed. Review it again.",
                            )));
                            cx.notify();
                        });
                        return;
                    }
                    if protected {
                        state_for_confirm.update(cx, |state, _cx| {
                            state.authorize_production_writes(connection_id, authorization_uses);
                        });
                    }
                    on_confirm(window, cx);
                    if protected {
                        state_for_confirm.update(cx, |state, _cx| {
                            state.revoke_production_write_authorizations(
                                connection_id,
                                authorization_uses,
                            );
                        });
                    }
                }),
            );
        }
    }
}

pub(crate) fn with_scoped_production_authorizations(
    state: &Entity<AppState>,
    grants: &[(Uuid, usize)],
    cx: &mut App,
    action: impl FnOnce(&mut App),
) {
    state.update(cx, |state, _cx| {
        for (connection_id, uses) in grants {
            state.authorize_production_writes(*connection_id, *uses);
        }
    });
    action(cx);
    state.update(cx, |state, _cx| {
        for (connection_id, uses) in grants {
            state.revoke_production_write_authorizations(*connection_id, *uses);
        }
    });
}

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

type ConfirmCallback = Box<dyn FnOnce(&mut Window, &mut App)>;

pub fn open_confirm_dialog(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<String>,
    message: impl Into<String>,
    confirm_label: impl Into<String>,
    destructive: bool,
    on_confirm: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    open_confirm_dialog_boxed(
        window,
        cx,
        WriteConfirmation {
            title: title.into(),
            message: message.into(),
            confirm_label: confirm_label.into(),
            destructive,
        },
        None,
        Box::new(on_confirm),
    );
}

fn open_confirm_dialog_boxed(
    window: &mut Window,
    cx: &mut App,
    confirmation: WriteConfirmation,
    identity: Option<ConnectionIdentity>,
    on_confirm: ConfirmCallback,
) {
    let WriteConfirmation { title, message, confirm_label, destructive } = confirmation;
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
                .when_some(identity.clone(), |content, identity| {
                    content.child(connection_identity_badge(&identity, true, cx))
                })
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
