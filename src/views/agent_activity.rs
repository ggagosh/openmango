use chrono::{DateTime, Utc};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::InputState;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::model::{
    ActionRequest, ActionStatus, OperationRecord, OperationStatus, ProposedAction,
};
use crate::components::{Button, FormField, cancel_button, open_confirm_dialog};
use crate::state::{AppCommands, AppState};
use crate::theme::{islands, sizing, spacing};

pub struct AgentActivityView {
    state: Entity<AppState>,
    _subscription: Subscription,
    _refresh: Task<()>,
}

impl AgentActivityView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_view, _state, cx| cx.notify());
        let refresh = cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                if view.update(cx, |_view, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self { state, _subscription: subscription, _refresh: refresh }
    }
}

impl Render for AgentActivityView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let broker = self.state.read(cx).action_broker();
        let actions = broker.list_all().unwrap_or_default();
        let operations = broker.store().list_operations().unwrap_or_default();
        let pending = actions
            .iter()
            .filter(|action| action.status == ActionStatus::PendingApproval)
            .cloned()
            .collect::<Vec<_>>();
        let reviewed = actions
            .into_iter()
            .filter(|action| {
                action.status != ActionStatus::PendingApproval && action.operation_id.is_none()
            })
            .take(20)
            .collect::<Vec<_>>();
        let pending_count = pending.len();
        let has_reviewed = !reviewed.is_empty();
        let running = operations
            .iter()
            .filter(|operation| {
                matches!(
                    operation.status,
                    OperationStatus::Queued
                        | OperationStatus::Running
                        | OperationStatus::CancelRequested
                )
            })
            .count();
        let recovery_required = operations
            .iter()
            .filter(|operation| operation.status == OperationStatus::RecoveryRequired)
            .count();

        let pending_list = if pending.is_empty() {
            empty_state(
                IconName::Check,
                "No approvals waiting",
                "New database-scale requests from agent clients will appear here.",
                cx,
            )
            .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .children(
                    pending
                        .into_iter()
                        .map(|action| action_card(self.state.clone(), action, window, cx)),
                )
                .into_any_element()
        };

        let operation_list = if operations.is_empty() {
            empty_state(
                IconName::Inbox,
                "No agent operations yet",
                "Approved backups, syncs, and reverts will be recorded here.",
                cx,
            )
            .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(px(8.0))
                .overflow_hidden()
                .bg(cx.theme().background)
                .children(
                    operations
                        .into_iter()
                        .take(50)
                        .map(|operation| operation_row(self.state.clone(), operation, cx)),
                )
                .into_any_element()
        };

        let reviewed_list = div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(8.0))
            .overflow_hidden()
            .bg(cx.theme().background)
            .children(reviewed.into_iter().map(|action| reviewed_action_row(action, cx)));

        let appearance = self.state.read(cx).settings.appearance.clone();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(islands::content_bg(&appearance, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .min_h(sizing::header_height())
                    .px(spacing::lg())
                    .py(spacing::sm())
                    .border_b_1()
                    .border_color(islands::panel_border(&appearance, cx))
                    .bg(islands::tool_bg(&appearance, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .child(
                                Icon::new(IconName::Bot)
                                    .small()
                                    .text_color(cx.theme().secondary_foreground),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.0))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(cx.theme().foreground)
                                            .child("Agent Activity"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(activity_summary(running, recovery_required)),
                                    ),
                            ),
                    )
                    .child(if pending_count == 0 {
                        status_badge("All clear", cx.theme().success, cx)
                    } else {
                        status_badge(&format!("{pending_count} waiting"), cx.theme().warning, cx)
                    }),
            )
            .child(
                div().flex_1().min_h_0().overflow_y_scrollbar().child(
                    div()
                        .w_full()
                        .max_w(px(960.0))
                        .mx_auto()
                        .p(spacing::lg())
                        .pb(px(32.0))
                        .flex()
                        .flex_col()
                        .gap(px(24.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(spacing::sm())
                                .child(section_header(
                                    "Needs your review",
                                    "OpenMango revalidates every request at approval time.",
                                    cx,
                                ))
                                .child(pending_list),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(spacing::sm())
                                .child(section_header(
                                    "Operations",
                                    "Durable execution, cancellation, and recovery history.",
                                    cx,
                                ))
                                .child(operation_list),
                        )
                        .when(has_reviewed, |content| {
                            content.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(spacing::sm())
                                    .child(section_header(
                                        "Reviewed requests",
                                        "Requests that ended without creating an operation.",
                                        cx,
                                    ))
                                    .child(reviewed_list),
                            )
                        }),
                ),
            )
            .into_any_element()
    }
}

fn action_card(
    state: Entity<AppState>,
    action: ProposedAction,
    _window: &mut Window,
    cx: &App,
) -> Div {
    let action_id = action.id;
    let approve_state = state.clone();
    let reject_state = state.clone();
    let preview = &action.content.preview;
    let target_database = preview.target_database.clone();
    let protected = preview.target.protected;

    div()
        .flex()
        .flex_col()
        .gap(spacing::md())
        .p(spacing::lg())
        .rounded(px(8.0))
        .border_1()
        .border_color(if protected {
            cx.theme().warning.opacity(0.45)
        } else {
            cx.theme().border
        })
        .bg(islands::card_bg(&state.read(cx).settings.appearance, cx))
        .child(
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap(spacing::md())
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(spacing::sm())
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .size(px(30.0))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .bg(cx.theme().secondary.opacity(0.55))
                                .child(
                                    Icon::new(action_icon(&action.content.request))
                                        .xsmall()
                                        .text_color(cx.theme().secondary_foreground),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .min_w(px(0.0))
                                .flex_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(cx.theme().foreground)
                                        .child(preview.summary.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{} · requested {} · expires {} · #{}",
                                            action
                                                .content
                                                .origin
                                                .client_label
                                                .as_deref()
                                                .unwrap_or("Agent"),
                                            relative_time(action.created_at),
                                            action.expires_at.format("%H:%M"),
                                            action.hash_suffix()
                                        )),
                                ),
                        ),
                )
                .child(status_badge(
                    if protected { "Protected" } else { "Approval required" },
                    cx.theme().warning,
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(6.0))
                .bg(cx.theme().secondary.opacity(0.25))
                .px(spacing::md())
                .children(preview.source.as_ref().map(|source| {
                    scope_row(
                        "Source",
                        &format!(
                            "{} / {}",
                            source.display_name,
                            preview.source_database.as_deref().unwrap_or("—")
                        ),
                        &format!(
                            "{} · {}",
                            source.environment.as_deref().unwrap_or("Unclassified"),
                            short_id(source.connection_id)
                        ),
                        cx,
                    )
                }))
                .child(scope_row(
                    "Target",
                    &format!("{} / {}", preview.target.display_name, preview.target_database),
                    &format!(
                        "{} · {} · {}",
                        preview.target.environment.as_deref().unwrap_or("Unclassified"),
                        if preview.target.read_only { "Read-only" } else { "Writable" },
                        short_id(preview.target.connection_id)
                    ),
                    cx,
                ))
                .child(scope_row(
                    "Estimated scope",
                    &format!(
                        "{} documents · {}",
                        preview.estimated_documents,
                        format_bytes(preview.estimated_bytes)
                    ),
                    "Live estimate; verified again before execution",
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .child(detail_line("Backup", &preview.backup_behavior, cx))
                .child(detail_line("Recovery", &preview.rollback_behavior, cx)),
        )
        .children((!preview.warnings.is_empty()).then(|| {
            div()
                .flex()
                .items_start()
                .gap(spacing::sm())
                .p(spacing::sm())
                .rounded(px(6.0))
                .bg(cx.theme().warning.opacity(0.08))
                .text_color(cx.theme().warning)
                .child(Icon::new(IconName::TriangleAlert).xsmall())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .text_xs()
                        .children(preview.warnings.iter().cloned().map(|warning| {
                            div().child(warning)
                        })),
                )
        }))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(spacing::md())
                .pt(spacing::xs())
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Approval applies only to this exact, hashed request."),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(
                            Button::new(("reject-agent-action", action_id.as_u128() as u64))
                                .xsmall()
                                .label("Reject")
                                .on_click(move |_, window, cx| {
                                    let state = reject_state.clone();
                                    open_confirm_dialog(
                                        window,
                                        cx,
                                        "Reject agent request",
                                        "Reject this exact request? The agent may submit a new one later.",
                                        "Reject",
                                        true,
                                        move |_window, cx| {
                                            AppCommands::reject_agent_action(
                                                state.clone(),
                                                action_id,
                                                cx,
                                            );
                                        },
                                    );
                                }),
                        )
                        .child(
                            Button::new(("approve-agent-action", action_id.as_u128() as u64))
                                .xsmall()
                                .primary()
                                .label("Approve & Run")
                                .on_click(move |_, window, cx| {
                                    if protected {
                                        open_typed_approval_dialog(
                                            approve_state.clone(),
                                            action_id,
                                            target_database.clone(),
                                            window,
                                            cx,
                                        );
                                    } else {
                                        let state = approve_state.clone();
                                        open_confirm_dialog(
                                            window,
                                            cx,
                                            "Approve and run",
                                            "OpenMango will revalidate this request, create a durable operation, and begin execution.",
                                            "Approve & Run",
                                            false,
                                            move |_window, cx| {
                                                AppCommands::approve_agent_action(
                                                    state.clone(),
                                                    action_id,
                                                    cx,
                                                );
                                            },
                                        );
                                    }
                                }),
                        ),
                ),
        )
}

fn operation_row(state: Entity<AppState>, operation: OperationRecord, cx: &App) -> Div {
    let can_cancel = matches!(operation.status, OperationStatus::Queued | OperationStatus::Running);
    let operation_id = operation.id;
    let tone = operation_status_color(operation.status, cx);

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::md())
        .px(spacing::md())
        .py(spacing::md())
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .min_w(px(0.0))
                .flex_1()
                .child(div().size(px(8.0)).flex_shrink_0().rounded_full().bg(tone))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(cx.theme().foreground)
                                .child(format!(
                                    "{} · {}",
                                    operation_kind_label(&operation.request),
                                    operation.target_database
                                )),
                        )
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                            format!(
                                "{} · {} · {}",
                                operation.progress_label(),
                                operation.origin.client_label.as_deref().unwrap_or("Agent"),
                                relative_time(operation.updated_at)
                            ),
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(spacing::sm())
                .child(status_badge(operation_status_label(operation.status), tone, cx))
                .when(can_cancel, |actions| {
                    actions.child(
                        Button::new(("cancel-agent-operation", operation_id.as_u128() as u64))
                            .xsmall()
                            .danger()
                            .label("Cancel")
                            .on_click(move |_, _, cx| {
                                AppCommands::cancel_agent_operation(
                                    state.clone(),
                                    operation_id,
                                    cx,
                                );
                            }),
                    )
                }),
        )
}

fn reviewed_action_row(action: ProposedAction, cx: &App) -> Div {
    let tone = action_status_color(action.status, cx);
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::md())
        .px(spacing::md())
        .py(spacing::md())
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .min_w(px(0.0))
                .flex_1()
                .child(
                    Icon::new(action_icon(&action.content.request))
                        .xsmall()
                        .flex_shrink_0()
                        .text_color(tone),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child(action.content.preview.summary.clone()),
                        )
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                            format!(
                                "{} · #{}",
                                relative_time(action.created_at),
                                action.hash_suffix()
                            ),
                        )),
                ),
        )
        .child(status_badge(action_status_label(action.status), tone, cx))
}

fn open_typed_approval_dialog(
    state: Entity<AppState>,
    action_id: uuid::Uuid,
    target_database: String,
    window: &mut Window,
    cx: &mut App,
) {
    let input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(target_database.clone()).default_value("")
    });
    window.open_dialog(cx, move |dialog: Dialog, _window, cx| {
        dialog
            .title("Confirm protected database operation")
            .min_w(px(500.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .p(spacing::md())
                    .child(
                        div().text_sm().child(format!(
                            "Type {target_database} to approve this exact operation."
                        )),
                    )
                    .child(FormField::new("Target database", &input).render(cx)),
            )
            .footer({
                let state = state.clone();
                let input = input.clone();
                let target_database = target_database.clone();

                let matches = input.read(cx).value().as_ref() == target_database;
                let state = state.clone();
                let input_for_click = input.clone();
                let target_for_click = target_database.clone();
                gpui_kit::component::dialog::DialogFooter::new().children(vec![
                    cancel_button("cancel-protected-approval"),
                    Button::new("approve-protected-action")
                        .danger()
                        .label("Approve & Run")
                        .disabled(!matches)
                        .on_click(move |_, window, cx| {
                            if input_for_click.read(cx).value().as_ref() != target_for_click {
                                return;
                            }
                            window.close_dialog(cx);
                            AppCommands::approve_agent_action(state.clone(), action_id, cx);
                        })
                        .into_any_element(),
                ])
            })
    });
}

fn section_header(title: &str, description: &str, cx: &App) -> Div {
    div().flex().items_end().justify_between().gap(spacing::md()).child(
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(description.to_string()),
            ),
    )
}

fn empty_state(icon: IconName, title: &str, description: &str, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .p(spacing::lg())
        .rounded(px(8.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            div()
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.0))
                .bg(cx.theme().secondary.opacity(0.45))
                .child(Icon::new(icon).small().text_color(cx.theme().muted_foreground)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().foreground)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description.to_string()),
                ),
        )
}

fn scope_row(label: &str, value: &str, detail: &str, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .py(spacing::sm())
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.65))
        .child(
            div()
                .w(px(110.0))
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .min_w(px(0.0))
                .flex_1()
                .child(div().text_sm().text_color(cx.theme().foreground).child(value.to_string()))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail.to_string()),
                ),
        )
}

fn detail_line(label: &str, value: &str, cx: &App) -> Div {
    div()
        .flex()
        .items_start()
        .gap(spacing::sm())
        .text_xs()
        .child(
            div()
                .w(px(70.0))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(div().text_color(cx.theme().secondary_foreground).child(value.to_string()))
}

fn status_badge(label: &str, color: Hsla, _cx: &App) -> Div {
    div()
        .px(spacing::sm())
        .py(px(2.0))
        .rounded(px(5.0))
        .bg(color.opacity(0.11))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .text_color(color)
        .child(label.to_string())
}

fn action_icon(request: &ActionRequest) -> Icon {
    match request {
        ActionRequest::DatabaseBackup { .. } => crate::assets::AppIcon::Download.into(),
        ActionRequest::DatabaseSync { .. } => IconName::Replace.into(),
        ActionRequest::OperationRevert { .. } => IconName::Undo2.into(),
    }
}

fn operation_kind_label(request: &ActionRequest) -> &'static str {
    match request {
        ActionRequest::DatabaseBackup { .. } => "Backup",
        ActionRequest::DatabaseSync { .. } => "Database sync",
        ActionRequest::OperationRevert { .. } => "Revert",
    }
}

fn action_status_label(status: ActionStatus) -> &'static str {
    match status {
        ActionStatus::PendingApproval => "Pending approval",
        ActionStatus::Rejected => "Rejected",
        ActionStatus::Expired => "Expired",
        ActionStatus::Stale => "Stale",
        ActionStatus::Accepted => "Accepted",
    }
}

fn action_status_color(status: ActionStatus, cx: &App) -> Hsla {
    match status {
        ActionStatus::PendingApproval | ActionStatus::Stale => cx.theme().warning,
        ActionStatus::Rejected => cx.theme().danger,
        ActionStatus::Expired => cx.theme().muted_foreground,
        ActionStatus::Accepted => cx.theme().success,
    }
}

fn operation_status_label(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::Queued => "Queued",
        OperationStatus::Running => "Running",
        OperationStatus::CancelRequested => "Cancelling",
        OperationStatus::Completed => "Completed",
        OperationStatus::Failed => "Failed",
        OperationStatus::Cancelled => "Cancelled",
        OperationStatus::Interrupted => "Interrupted",
        OperationStatus::RecoveryRequired => "Recovery required",
    }
}

fn operation_status_color(status: OperationStatus, cx: &App) -> Hsla {
    match status {
        OperationStatus::Queued => cx.theme().muted_foreground,
        OperationStatus::Running | OperationStatus::CancelRequested => cx.theme().primary,
        OperationStatus::Completed => cx.theme().success,
        OperationStatus::Failed | OperationStatus::RecoveryRequired => cx.theme().danger,
        OperationStatus::Cancelled | OperationStatus::Interrupted => cx.theme().warning,
    }
}

fn activity_summary(running: usize, recovery_required: usize) -> String {
    match (running, recovery_required) {
        (0, 0) => "Approvals and durable database operations".into(),
        (_, 0) => format!("{running} operation{} running", if running == 1 { "" } else { "s" }),
        (0, _) => format!(
            "{recovery_required} operation{} need recovery",
            if recovery_required == 1 { "" } else { "s" }
        ),
        _ => format!("{running} running · {recovery_required} need recovery"),
    }
}

fn relative_time(time: DateTime<Utc>) -> String {
    let seconds = (Utc::now() - time).num_seconds().max(0);
    if seconds < 60 {
        "just now".into()
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else if seconds < 604_800 {
        format!("{}d ago", seconds / 86_400)
    } else {
        time.format("%b %-d, %H:%M").to_string()
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}

fn short_id(id: uuid::Uuid) -> String {
    id.to_string()[..8].to_string()
}

trait OperationProgressLabel {
    fn progress_label(&self) -> &'static str;
}

impl OperationProgressLabel for OperationRecord {
    fn progress_label(&self) -> &'static str {
        match self.progress.phase {
            crate::actions::model::OperationPhase::Queued => "Queued",
            crate::actions::model::OperationPhase::Preparing => "Preparing",
            crate::actions::model::OperationPhase::DumpingDatabase => "Dumping database",
            crate::actions::model::OperationPhase::DumpingSource => "Dumping source",
            crate::actions::model::OperationPhase::ValidatingSourceDump => "Validating source dump",
            crate::actions::model::OperationPhase::CheckingTargetPrecondition => "Checking target",
            crate::actions::model::OperationPhase::BackingUpTarget => "Backing up target",
            crate::actions::model::OperationPhase::VerifyingBackup => "Verifying backup",
            crate::actions::model::OperationPhase::ReplacingTarget => "Replacing target",
            crate::actions::model::OperationPhase::VerifyingTarget => "Verifying target",
            crate::actions::model::OperationPhase::RestoringTargetBackup => "Restoring target",
            crate::actions::model::OperationPhase::VerifyingRecovery => "Verifying recovery",
            crate::actions::model::OperationPhase::Completed => "Completed",
        }
    }
}
