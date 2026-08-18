use chrono::{DateTime, Utc};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::bson::truncate_for_preview;
use crate::components::{Button, WriteRequest, request_connection_write};
use crate::operations::{OperationChangePreview, OperationKind, OperationStatus, OperationSummary};
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{fonts, spacing};

pub(crate) struct HistoryViewState {
    pub operations: Vec<OperationSummary>,
    pub loading: bool,
    pub total: u64,
    pub next_offset: Option<u32>,
    pub error: Option<String>,
}

pub(crate) fn render_history_view(
    state: Entity<AppState>,
    session_key: Option<SessionKey>,
    history: HistoryViewState,
    cx: &App,
) -> AnyElement {
    let HistoryViewState { operations, loading, total, next_offset, error } = history;
    let Some(session_key) = session_key else {
        return empty_state(
            IconName::Undo2,
            "No collection selected",
            "Select a collection to view its operation history.",
            cx,
        )
        .into_any_element();
    };
    let enabled = state.read(cx).connection_reversible_history(session_key.connection_id);

    if let Some(error) = error {
        return empty_state(IconName::TriangleAlert, "History unavailable", &error, cx)
            .into_any_element();
    }
    if operations.is_empty() {
        return empty_state(
            IconName::Undo2,
            if loading {
                "Loading history…"
            } else if enabled {
                "No tracked changes yet"
            } else {
                "History is disabled"
            },
            if enabled {
                "Document inserts, replacements, and deletions saved from now on will appear here."
            } else {
                "Enable Reversible history in this connection's settings to track future document changes."
            },
            cx,
        )
        .into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(cx.theme().background)
        .child(history_header(total, enabled, cx))
        .child(column_header(cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scrollbar()
                .children(
                    operations
                        .into_iter()
                        .map(|operation| operation_row(state.clone(), operation, cx)),
                )
                .children(next_offset.map(|_| {
                    let state = state.clone();
                    let session_key = session_key.clone();
                    div().flex().justify_center().p(spacing::lg()).child(
                        Button::new("load-more-collection-history")
                            .ghost()
                            .compact()
                            .label(if loading { "Loading…" } else { "Load more" })
                            .disabled(loading)
                            .on_click(move |_, _, cx| {
                                AppCommands::load_more_collection_history(
                                    state.clone(),
                                    session_key.clone(),
                                    cx,
                                );
                            }),
                    )
                })),
        )
        .into_any_element()
}

fn history_header(total: u64, enabled: bool, cx: &App) -> Div {
    let recording_color = if enabled { cx.theme().success } else { cx.theme().muted_foreground };
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_shrink_0()
        .w_full()
        .h(px(58.0))
        .gap(spacing::lg())
        .px(spacing::lg())
        .bg(cx.theme().tab_bar)
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w(px(0.0))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Document history"))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Review document edits and restore an earlier value. Recovery data is encrypted on this Mac.",
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(spacing::md())
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(operation_count(total)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .text_xs()
                        .text_color(if enabled { cx.theme().foreground } else { recording_color })
                        .child(div().size(px(6.0)).rounded_full().bg(recording_color))
                        .child(if enabled { "Recording" } else { "Recording off" }),
                ),
        )
}

fn column_header(cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .w_full()
        .h(px(26.0))
        .px(spacing::lg())
        .bg(cx.theme().background)
        .border_b_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(div().flex_1().min_w(px(0.0)).child("Change"))
        .child(div().w(px(112.0)).child("When"))
        .child(div().w(px(136.0)).child("Status"))
        .child(div().w(px(88.0)).text_align(TextAlign::Right).child("Action"))
}

fn operation_row(state: Entity<AppState>, operation: OperationSummary, cx: &App) -> Div {
    let color = status_color(operation.status, cx);
    let operation_id = operation.id;
    let connection_id = operation.connection_id;
    let database = operation.database.clone();
    let collection = operation.collection.clone();
    let target = format!("{database}.{collection}");
    let document_id = operation
        .preview
        .as_ref()
        .map(|preview| preview.document_id.as_str())
        .unwrap_or("unknown document");
    let title = match operation.kind {
        OperationKind::InsertDocument => format!("Inserted document {document_id}"),
        OperationKind::ReplaceDocument => format!("Updated document {document_id}"),
        OperationKind::DeleteDocument => format!("Deleted document {document_id}"),
        OperationKind::RevertDocument => format!("Reverted document {document_id}"),
    };
    let preview = change_preview(&operation);
    let icon = match operation.kind {
        OperationKind::InsertDocument => IconName::Plus,
        OperationKind::ReplaceDocument => IconName::Replace,
        OperationKind::DeleteDocument => IconName::Delete,
        OperationKind::RevertDocument => IconName::Undo2,
    };

    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .w_full()
        .gap(spacing::md())
        .px(spacing::lg())
        .py(spacing::md())
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .hover(|row| row.bg(cx.theme().list_hover))
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::md())
                .min_w(px(0.0))
                .flex_1()
                .child(
                    div()
                        .size(px(30.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .rounded(px(7.0))
                        .bg(cx.theme().secondary.opacity(0.45))
                        .child(Icon::new(icon).small().text_color(cx.theme().secondary_foreground)),
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
                                .flex()
                                .items_baseline()
                                .gap(spacing::sm())
                                .min_w(px(0.0))
                                .child(
                                    div()
                                        .min_w(px(0.0))
                                        .truncate()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .font_family(fonts::mono())
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("#{}", short_id(operation.id))),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.0))
                                .font_family(fonts::mono())
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(preview),
                        )
                        .children(
                            operation
                                .recovery_status
                                .clone()
                                .map(|status| div().text_xs().text_color(color).child(status)),
                        ),
                ),
        )
        .child(
            div()
                .w(px(112.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(relative_time(operation.created_at))
                .child(operation.origin.label()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .w(px(136.0))
                .flex_shrink_0()
                .child(status_badge(operation.status.label(), color)),
        )
        .child(div().w(px(88.0)).flex().justify_end().flex_shrink_0().when(
            operation.can_revert(),
            |actions| {
                actions.child(
                    Button::new(("restore-collection-operation", operation_id.as_u128() as u64))
                        .ghost()
                        .compact()
                        .label("Revert")
                        .on_click(move |_, window, cx| {
                            let state_for_write = state.clone();
                            let database = database.clone();
                            let collection = collection.clone();
                            request_connection_write(
                                state.clone(),
                                WriteRequest::new(
                                    connection_id,
                                    target.clone(),
                                    "Revert this document change",
                                    None,
                                ),
                                window,
                                cx,
                                move |_window, cx| {
                                    AppCommands::revert_operation(
                                        state_for_write.clone(),
                                        operation_id,
                                        connection_id,
                                        database.clone(),
                                        collection.clone(),
                                        cx,
                                    );
                                },
                            );
                        }),
                )
            },
        ))
}

fn change_preview(operation: &OperationSummary) -> String {
    let Some(preview) = &operation.preview else {
        return "Preview unavailable".into();
    };
    if preview.total_changes == 0 {
        return "No field-level changes".into();
    }
    let mut text = preview.changes.iter().map(format_change).collect::<Vec<_>>().join("  ·  ");
    let remaining = preview.total_changes.saturating_sub(preview.changes.len());
    if remaining > 0 {
        text.push_str(&format!("  ·  +{remaining} more"));
    }
    truncate_for_preview(&text, 160)
}

fn format_change(change: &OperationChangePreview) -> String {
    let field = truncate_for_preview(&change.field, 32);
    match (&change.before, &change.after) {
        (Some(before), Some(after)) => format!("{field}: {before} → {after}"),
        (None, Some(after)) => format!("{field}: added {after}"),
        (Some(before), None) => format!("{field}: removed {before}"),
        (None, None) => field,
    }
}

fn status_color(status: OperationStatus, cx: &App) -> Hsla {
    match status {
        OperationStatus::Prepared | OperationStatus::Running => cx.theme().primary,
        OperationStatus::Completed => cx.theme().success,
        OperationStatus::Failed => cx.theme().muted_foreground,
        OperationStatus::Conflict | OperationStatus::RecoveryRequired => cx.theme().danger,
        OperationStatus::Uncertain => cx.theme().warning,
    }
}

fn status_badge(label: &str, color: Hsla) -> Div {
    div()
        .px(spacing::sm())
        .py(px(2.0))
        .rounded(px(5.0))
        .bg(color.opacity(0.11))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(label.to_string())
}

fn empty_state(icon: IconName, title: &str, description: &str, cx: &App) -> Div {
    div().size_full().flex().items_center().justify_center().bg(cx.theme().background).child(
        div()
            .max_w(px(520.0))
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .gap(spacing::sm())
            .px(px(24.0))
            .child(
                div()
                    .size(px(34.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .bg(cx.theme().secondary.opacity(0.45))
                    .child(Icon::new(icon).small().text_color(cx.theme().muted_foreground)),
            )
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title.to_string()))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(description.to_string()),
            ),
    )
}

fn operation_count(total: u64) -> String {
    if total == 1 { "1 operation".into() } else { format!("{total} operations") }
}

fn relative_time(time: DateTime<Utc>) -> String {
    let seconds = (Utc::now() - time).num_seconds().max(0);
    if seconds < 60 {
        "just now".into()
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else {
        time.format("%b %-d, %H:%M").to_string()
    }
}

fn short_id(id: uuid::Uuid) -> String {
    id.to_string()[..8].to_string()
}
