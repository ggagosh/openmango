use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::components::{Button, WriteConfirmation, WriteRequest, request_connection_write};
use crate::history::{BatchStatus, BatchSummary, HistoryGap};
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{fonts, spacing};

pub(crate) struct HistoryViewState {
    pub batches: Vec<BatchSummary>,
    pub gaps: Vec<HistoryGap>,
    pub details: HashMap<uuid::Uuid, crate::history::BatchDetails>,
    pub detail_loading: HashSet<uuid::Uuid>,
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
    let HistoryViewState {
        batches,
        gaps,
        details,
        detail_loading,
        loading,
        total,
        next_offset,
        error,
    } = history;
    let Some(session_key) = session_key else {
        return empty_state(
            IconName::Undo2,
            "No collection selected",
            "Select a collection to view its History batches.",
            cx,
        )
        .into_any_element();
    };
    let enabled = state.read(cx).connection_history_enabled(session_key.connection_id);
    if !enabled {
        return empty_state(
            IconName::Undo2,
            "History is disabled",
            "Enable History in Settings after the server passes eligibility checks.",
            cx,
        )
        .into_any_element();
    }
    if let Some(error) = error {
        return empty_state(IconName::TriangleAlert, "History unavailable", &error, cx)
            .into_any_element();
    }
    if batches.is_empty() && gaps.is_empty() {
        return empty_state(
            IconName::Undo2,
            if loading { "Loading History…" } else { "No observed change sets yet" },
            "History records supported changes observed by OpenMango on this device. It may include writes from other clients and can contain gaps. It is not a backup or audit log.",
            cx,
        )
        .into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .bg(cx.theme().background)
        .child(history_header(total, gaps.len(), state.clone(), session_key.clone(), cx))
        .child(column_header(cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .overflow_y_scrollbar()
                .children(gaps.into_iter().map(|gap| gap_row(gap, cx)))
                .children(batches.into_iter().map(|batch| {
                    let batch_id = batch.id;
                    batch_row(
                        state.clone(),
                        session_key.clone(),
                        batch,
                        details.get(&batch_id).cloned(),
                        detail_loading.contains(&batch_id),
                        cx,
                    )
                }))
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

fn history_header(
    total: u64,
    gaps: usize,
    state: Entity<AppState>,
    session_key: SessionKey,
    cx: &App,
) -> Div {
    let state_for_refresh = state.clone();
    let session_for_refresh = session_key.clone();
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .w_full()
        .h(px(40.0))
        .gap(spacing::sm())
        .px(spacing::lg())
        .bg(cx.theme().tab_bar)
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(format!("History · {total} change set{}", if total == 1 { "" } else { "s" })),
        )
        .when(gaps > 0, |element| {
            element.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child("Coverage incomplete"),
            )
        })
        .child(
            Button::new("refresh-collection-history")
                .ghost()
                .compact()
                .label("Refresh")
                .on_click(move |_, _, cx| {
                    AppCommands::load_collection_history(
                        state_for_refresh.clone(),
                        session_for_refresh.clone(),
                        cx,
                    );
                }),
        )
        .child(
            Button::new("clear-collection-history")
                .ghost()
                .compact()
                .label("Clear collection")
                .on_click(move |_, window, cx| {
                    let state_for_clear = state.clone();
                    let session_for_clear = session_key.clone();
                    request_connection_write(
                        state.clone(),
                        WriteRequest::new(
                            session_key.connection_id,
                            format!("{}.{} local History", session_key.database, session_key.collection),
                            "Delete local encrypted History data",
                            Some(WriteConfirmation {
                                title: "Clear collection History".into(),
                                message: "Delete all non-active History batches and collection-scoped gaps for this collection. This cannot be undone.".into(),
                                confirm_label: "Clear History".into(),
                                destructive: true,
                            }),
                        ),
                        window,
                        cx,
                        move |_, cx| {
                            AppCommands::clear_collection_history(
                                state_for_clear.clone(),
                                session_for_clear.clone(),
                                cx,
                            );
                        },
                    );
                }),
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
        .border_b_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(div().flex_1().child("Observed change set"))
        .child(div().w(px(130.0)).child("Grouping"))
        .child(div().w(px(112.0)).child("When"))
        .child(div().w(px(208.0)).text_align(TextAlign::Right).child("Actions"))
}

fn gap_row(gap: HistoryGap, cx: &App) -> Div {
    let (title, description) = gap_copy(&gap.kind, &gap.reason);
    div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .px(spacing::lg())
        .py(spacing::sm())
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .bg(cx.theme().warning.opacity(0.08))
        .child(Icon::new(IconName::TriangleAlert).small().text_color(cx.theme().warning))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().warning)
                        .child(title),
                )
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(description)),
        )
        .child(
            div()
                .w(px(112.0))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(relative_time(gap.created_at)),
        )
}

fn batch_row(
    state: Entity<AppState>,
    session_key: SessionKey,
    batch: BatchSummary,
    details: Option<crate::history::BatchDetails>,
    detail_loading: bool,
    cx: &App,
) -> Div {
    let batch_id = batch.id;
    let connection_id = batch.connection_id;
    let database = batch.database.clone();
    let collection = batch.collection.clone();
    let target = format!("{database}.{collection}");
    let pending_restore_count = batch.pending_restore_count();
    let resuming = batch.status == BatchStatus::PartiallyRestored;
    let (restore_title, restore_description, restore_label) =
        restore_copy(batch.family, pending_restore_count, resuming);
    let status_color = match batch.status {
        BatchStatus::Restoring => cx.theme().primary,
        BatchStatus::Restored => cx.theme().success,
        BatchStatus::PartiallyRestored | BatchStatus::Failed => cx.theme().warning,
        BatchStatus::Open | BatchStatus::Closed => cx.theme().muted_foreground,
    };
    let details_loaded = details.is_some();
    let detail_panel = details.map(|details| {
        let shown = details.items.len();
        div()
            .px(spacing::lg())
            .pt(spacing::sm())
            .pb(spacing::md())
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().tab_bar)
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Document samples")
                    .child(format!("Showing {shown} of {}", batch.item_count)),
            )
            .children(details.items.into_iter().map(|item| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::sm())
                    .font_family(fonts::mono())
                    .text_xs()
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_color(cx.theme().secondary_foreground)
                            .child(compact_document_key(&item.document_key)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(cx.theme().muted_foreground)
                            .child(item.outcome.to_string()),
                    )
            }))
    });
    let row = div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .px(spacing::lg())
        .py(spacing::md())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(3.0))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!(
                    "{} · {} item{}",
                    batch.family.label(),
                    batch.item_count,
                    if batch.item_count == 1 { "" } else { "s" }
                )))
                .child(
                    div()
                        .text_xs()
                        .text_color(status_color)
                        .child(batch_summary(&batch)),
                ),
        )
        .child(
            div()
                .w(px(130.0))
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(batch.grouping.label()),
        )
        .child(
            div()
                .w(px(112.0))
                .flex()
                .flex_col()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(relative_time(batch.last_wall_time))
                .child(format_time_range(batch.first_wall_time, batch.last_wall_time)),
        )
        .child(
            div()
                .w(px(208.0))
                .flex()
                .items_center()
                .justify_end()
                .gap(spacing::xs())
                .child(
                    Button::new(("history-batch-details", batch_id.as_u128() as u64))
                        .ghost()
                        .compact()
                        .label(if detail_loading {
                            "Loading…"
                        } else if details_loaded {
                            "Hide"
                        } else {
                            "Details"
                        })
                        .disabled(detail_loading)
                        .on_click({
                            let state = state.clone();
                            let session_key = session_key.clone();
                            move |_, _, cx| {
                                AppCommands::toggle_history_batch_details(
                                    state.clone(),
                                    session_key.clone(),
                                    batch_id,
                                    cx,
                                );
                            }
                        }),
                )
                .when(batch.status != BatchStatus::Restoring, |actions| {
                    let state = state.clone();
                    let session_key = session_key.clone();
                    actions.child(
                        Button::new(("delete-history-batch", batch_id.as_u128() as u64))
                            .ghost()
                            .compact()
                            .label("Delete")
                            .on_click(move |_, window, cx| {
                                let state_for_delete = state.clone();
                                let session_for_delete = session_key.clone();
                                request_connection_write(
                                    state.clone(),
                                    WriteRequest::new(
                                        connection_id,
                                        "local encrypted History batch",
                                        "Delete one local History batch",
                                        Some(WriteConfirmation {
                                            title: "Delete History batch".into(),
                                            message: "Delete this encrypted recovery batch permanently. This cannot be undone.".into(),
                                            confirm_label: "Delete batch".into(),
                                            destructive: true,
                                        }),
                                    ),
                                    window,
                                    cx,
                                    move |_, cx| {
                                        AppCommands::delete_history_batch(
                                            state_for_delete.clone(),
                                            session_for_delete.clone(),
                                            batch_id,
                                            cx,
                                        );
                                    },
                                );
                            }),
                    )
                })
                .when(batch.status == BatchStatus::Restoring, |actions| {
                    let state = state.clone();
                    actions.child(
                        Button::new(("cancel-history-restore", batch_id.as_u128() as u64))
                            .ghost()
                            .compact()
                            .label("Cancel")
                            .on_click(move |_, _, cx| {
                                AppCommands::cancel_history_restore(state.clone(), batch_id, cx);
                            }),
                    )
                })
                .when(batch.can_restore(), |actions| {
                    actions.child(
                        Button::new(("restore-history-batch", batch_id.as_u128() as u64))
                            .ghost()
                            .compact()
                            .label(if resuming { "Resume" } else { "Restore" })
                            .on_click(move |_, window, cx| {
                                let state_for_write = state.clone();
                                let database = database.clone();
                                let collection = collection.clone();
                                request_connection_write(
                                    state.clone(),
                                    WriteRequest::new(
                                        connection_id,
                                        target.clone(),
                                        "Restore this observed History change set",
                                        Some(WriteConfirmation {
                                            title: restore_title.clone(),
                                            message: restore_description.clone(),
                                            confirm_label: restore_label.clone(),
                                            destructive: false,
                                        }),
                                    ),
                                    window,
                                    cx,
                                    move |_window, cx| {
                                        AppCommands::revert_operation(
                                            state_for_write.clone(),
                                            batch_id,
                                            connection_id,
                                            database.clone(),
                                            collection.clone(),
                                            cx,
                                        );
                                    },
                                );
                            }),
                    )
                }),
        );

    div()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .hover(|container| container.bg(cx.theme().list_hover))
        .child(row)
        .children(detail_panel)
}

fn batch_summary(batch: &BatchSummary) -> String {
    let pending = batch.pending_restore_count();
    let processed = batch.revertible_count.saturating_sub(pending);
    let mut parts = match batch.status {
        BatchStatus::Open => {
            vec!["Recording".to_string(), format!("{} documents", batch.revertible_count)]
        }
        BatchStatus::Closed => {
            vec!["Ready to restore".to_string(), format!("{} documents", batch.revertible_count)]
        }
        BatchStatus::Restoring => {
            vec!["Restoring".to_string(), format!("{processed} of {}", batch.revertible_count)]
        }
        BatchStatus::PartiallyRestored if pending > 0 => {
            vec!["Paused".to_string(), format!("{pending} remaining")]
        }
        BatchStatus::PartiallyRestored => vec!["Completed with issues".to_string()],
        BatchStatus::Restored => {
            vec!["Restored".to_string(), format!("{} documents", batch.restored_count)]
        }
        BatchStatus::Failed => vec!["Restore failed".to_string(), format!("{pending} remaining")],
    };
    if batch.restored_count > 0
        && !matches!(batch.status, BatchStatus::Restoring | BatchStatus::Restored)
    {
        parts.push(format!("{} restored", batch.restored_count));
    }
    if batch.skipped_count > 0 {
        parts.push(format!("{} skipped", batch.skipped_count));
    }
    if batch.conflict_count > 0 {
        parts.push(format!("{} conflicts", batch.conflict_count));
    }
    if batch.failed_count > 0 {
        parts.push(format!("{} failed", batch.failed_count));
    }
    parts.join(" · ")
}

fn compact_document_key(key: &mongodb::bson::Document) -> String {
    let value =
        crate::bson::document_to_shell_string(key).split_whitespace().collect::<Vec<_>>().join(" ");
    crate::bson::truncate_for_preview(&value, 96)
}

fn restore_copy(
    family: crate::history::OperationFamily,
    count: u64,
    resuming: bool,
) -> (String, String, String) {
    let action = if resuming { "Resume" } else { "Restore" };
    let label = format!("{action} {count}");
    match family {
        crate::history::OperationFamily::Delete => (
            format!("{action} {count} deleted document{}?", if count == 1 { "" } else { "s" }),
            "Deleted documents are reinserted only when their original keys are still absent. Existing documents are skipped.".into(),
            label,
        ),
        crate::history::OperationFamily::Update | crate::history::OperationFamily::Replace => (
            format!("{action} {count} previous document version{}?", if count == 1 { "" } else { "s" }),
            "Documents are restored only when they still match the version captured after this change set. Later changes are skipped.".into(),
            label,
        ),
    }
}

fn gap_copy(kind: &str, reason: &str) -> (String, String) {
    match kind {
        "missing_resume_token" => (
            "Restore coverage was interrupted".into(),
            "Changes made during this gap are unavailable to History and cannot be restored."
                .into(),
        ),
        _ => ("History coverage gap".into(), reason.into()),
    }
}

fn empty_state(icon: IconName, title: &str, description: &str, cx: &App) -> Div {
    div().size_full().flex().items_center().justify_center().bg(cx.theme().background).child(
        div()
            .max_w(px(560.0))
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

fn format_time_range(first: DateTime<Utc>, last: DateTime<Utc>) -> String {
    if first == last {
        first.format("%H:%M:%S").to_string()
    } else {
        format!("{}–{}", first.format("%H:%M:%S"), last.format("%H:%M:%S"))
    }
}

#[cfg(test)]
mod tests {
    use super::{gap_copy, restore_copy};

    #[test]
    fn restore_confirmation_is_short_and_does_not_require_samples() {
        let (title, message, label) =
            restore_copy(crate::history::OperationFamily::Delete, 22, false);

        assert_eq!(title, "Restore 22 deleted documents?");
        assert_eq!(label, "Restore 22");
        assert!(message.contains("Existing documents are skipped"));
        assert!(!message.contains("Samples"));
    }

    #[test]
    fn missing_resume_token_uses_plain_language() {
        let (title, message) = gap_copy("missing_resume_token", "technical reason");

        assert_eq!(title, "Restore coverage was interrupted");
        assert!(message.contains("cannot be restored"));
        assert!(!message.contains("resume_token"));
    }
}
