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
        .size_full()
        .bg(cx.theme().background)
        .child(history_header(total, gaps.len(), state.clone(), session_key.clone(), cx))
        .child(column_header(cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
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
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_shrink_0()
        .w_full()
        .min_h(px(72.0))
        .gap(spacing::lg())
        .px(spacing::lg())
        .py(spacing::sm())
        .bg(cx.theme().tab_bar)
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w(px(0.0))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("History"))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("History records supported changes observed by OpenMango on this device. It may include writes from other clients and can contain gaps. It is not a backup or audit log."),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_end()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{total} change set{}", if total == 1 { "" } else { "s" }))
                .when(gaps > 0, |element| {
                    element.child(
                        div()
                            .text_color(cx.theme().warning)
                            .child(format!("{gaps} visible coverage gap{}", if gaps == 1 { "" } else { "s" })),
                    )
                })
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
        .border_b_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(div().flex_1().child("Observed change set"))
        .child(div().w(px(130.0)).child("Grouping"))
        .child(div().w(px(112.0)).child("When"))
        .child(div().w(px(88.0)).text_align(TextAlign::Right).child("Action"))
}

fn gap_row(gap: HistoryGap, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .px(spacing::lg())
        .py(spacing::md())
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
                        .child(format!("Coverage gap: {}", gap.kind)),
                )
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(gap.reason)),
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
    let sample_keys = details
        .as_ref()
        .map(|details| {
            details
                .items
                .iter()
                .take(5)
                .map(|item| crate::bson::document_to_shell_string(&item.document_key))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "Load details to review representative document keys".into());
    let conflict_rule = match batch.family {
        crate::history::OperationFamily::Delete => {
            "Deleted documents are reinserted only while their keys remain absent."
        }
        crate::history::OperationFamily::Update | crate::history::OperationFamily::Replace => {
            "Before-images replace documents only while current documents exactly equal recorded after-images."
        }
    };
    let restore_description = format!(
        "Namespace: {target}\nGrouping: {}\nTime: {}\nItems: {} ({} revertible)\nSamples: {}\n\n{} Conflicts are skipped and never overwritten.",
        batch.grouping.label(),
        format_time_range(batch.first_wall_time, batch.last_wall_time),
        batch.item_count,
        batch.revertible_count,
        sample_keys,
        conflict_rule,
    );
    let status_color = match batch.status {
        BatchStatus::Restoring => cx.theme().primary,
        BatchStatus::Restored => cx.theme().success,
        BatchStatus::PartiallyRestored | BatchStatus::Failed => cx.theme().warning,
        BatchStatus::Open | BatchStatus::Closed => cx.theme().muted_foreground,
    };
    let details_loaded = details.is_some();
    let has_more_details = details.as_ref().is_some_and(|details| details.next_offset.is_some());
    let detail_panel = details.map(|details| {
        div()
            .mt(spacing::xs())
            .pl(spacing::sm())
            .border_l_2()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(details.items.into_iter().map(|item| {
                let key = crate::bson::document_to_shell_string(&item.document_key);
                div()
                    .font_family(fonts::mono())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} · {}",
                        crate::bson::truncate_for_preview(&key, 100),
                        item.outcome
                    ))
            }))
    });
    div()
        .flex()
        .items_center()
        .gap(spacing::md())
        .px(spacing::lg())
        .py(spacing::md())
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .hover(|row| row.bg(cx.theme().list_hover))
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
                        .font_family(fonts::mono())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{}.{} · {} revertible · {} restored · {} skipped · {} conflicts · {} failed · {} encrypted bytes",
                            batch.database,
                            batch.collection,
                            batch.revertible_count,
                            batch.restored_count,
                            batch.skipped_count,
                            batch.conflict_count,
                            batch.failed_count,
                            batch.encrypted_bytes
                        )),
                )
                .child(
                    div().text_xs().text_color(status_color).child(format!("{:?}", batch.status)),
                )
                .children(detail_panel),
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
                .w(px(88.0))
                .flex()
                .flex_col()
                .items_end()
                .gap(spacing::xs())
                .child(
                    Button::new(("history-batch-details", batch_id.as_u128() as u64))
                        .ghost()
                        .compact()
                        .label(if detail_loading {
                            "Loading…"
                        } else if !details_loaded {
                            "Details"
                        } else if has_more_details {
                            "More"
                        } else {
                            "Loaded"
                        })
                        .disabled(detail_loading || (details_loaded && !has_more_details))
                        .on_click({
                            let state = state.clone();
                            let session_key = session_key.clone();
                            move |_, _, cx| {
                                AppCommands::load_history_batch_details(
                                    state.clone(),
                                    session_key.clone(),
                                    batch_id,
                                    details_loaded,
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
                            .label("Restore")
                            .disabled(!details_loaded)
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
                                            title: "Confirm History restore".into(),
                                            message: restore_description.clone(),
                                            confirm_label: "Restore without overwriting conflicts".into(),
                                            destructive: true,
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
        )
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
