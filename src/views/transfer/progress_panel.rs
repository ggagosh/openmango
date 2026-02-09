//! Progress panel for database-scope transfer operations.

use gpui::*;
use gpui_component::progress::Progress;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use uuid::Uuid;

use crate::connection::tools_available;
use crate::state::app_state::{
    CollectionProgress, CollectionTransferStatus, DatabaseTransferProgress,
};
use crate::state::{AppState, TransferFormat, TransferMode, TransferTabState};
use crate::theme::{borders, colors, spacing};

/// Render the database progress panel with per-collection progress rows.
pub(super) fn render_progress_panel(
    db_progress: &DatabaseTransferProgress,
    state: Entity<AppState>,
    transfer_id: Uuid,
    cx: &App,
) -> impl IntoElement {
    let completed = db_progress.completed_count();
    let total = db_progress.collections.len();

    // Collapsible header - use a button for click handling
    let expanded = db_progress.panel_expanded;
    let header = {
        let state = state.clone();
        div()
            .id("progress-panel-header")
            .flex()
            .items_center()
            .gap(spacing::sm())
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                state.update(cx, |state, cx| {
                    if let Some(tab) = state.transfer_tab_mut(transfer_id)
                        && let Some(ref mut db_progress) = tab.runtime.database_progress
                    {
                        db_progress.panel_expanded = !db_progress.panel_expanded;
                    }
                    cx.notify();
                });
            })
            .child(
                Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().secondary_foreground)
                    .child("Progress Details"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("({}/{})", completed, total)),
            )
    };

    // Content - per-collection progress rows (scrollable with max height)
    let content = if expanded {
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .mt(spacing::sm())
            .max_h(px(300.0)) // Limit height for scrolling
            .overflow_y_scrollbar()
            .children(
                db_progress.collections.iter().map(|coll| render_collection_progress_row(coll, cx)),
            )
            .into_any_element()
    } else {
        div().into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .p(spacing::md())
        .bg(cx.theme().sidebar)
        .border_1()
        .border_color(cx.theme().sidebar_border)
        .rounded(borders::radius_sm())
        .child(header)
        .child(content)
}

/// Render a single collection progress row.
fn render_collection_progress_row(coll: &CollectionProgress, cx: &App) -> impl IntoElement {
    // Status indicator (icon + color)
    let (status_icon, status_color) = match &coll.status {
        CollectionTransferStatus::Pending => (IconName::ChevronRight, cx.theme().muted_foreground),
        CollectionTransferStatus::InProgress => {
            (IconName::ArrowRight, crate::theme::colors::syntax_string(cx))
        }
        CollectionTransferStatus::Completed => (IconName::Check, cx.theme().success),
        CollectionTransferStatus::Failed(_) => (IconName::Close, cx.theme().danger),
        CollectionTransferStatus::Cancelled => (IconName::Close, cx.theme().warning),
    };

    // Progress percentage
    let percentage = coll.percentage().unwrap_or(0.0);

    // Progress text
    let progress_text = match (coll.documents_processed, coll.documents_total) {
        (processed, Some(total)) => format!("{} / {} ({:.0}%)", processed, total, percentage),
        (processed, None) => format!("{} docs", processed),
    };

    // Error message (if failed)
    let error_row: AnyElement = if let CollectionTransferStatus::Failed(err) = &coll.status {
        div()
            .ml(px(20.0)) // Align with collection name
            .text_xs()
            .text_color(cx.theme().danger)
            .overflow_hidden()
            .text_ellipsis()
            .child(err.clone())
            .into_any_element()
    } else {
        div().into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .min_w_0() // Allow shrinking for flex children
                // Status icon
                .child(Icon::new(status_icon).xsmall().text_color(status_color))
                // Collection name (flexible with min/max width)
                .child(
                    div()
                        .min_w(px(120.0))
                        .max_w(px(240.0))
                        .flex_shrink_0()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(coll.name.clone()),
                )
                // Progress bar (flexible)
                .child(div().flex_1().min_w(px(100.0)).child(Progress::new().value(percentage)))
                // Progress text (fixed width, accommodates large numbers)
                .child(
                    div()
                        .w(px(180.0))
                        .flex_shrink_0()
                        .text_xs()
                        .text_right()
                        .text_color(cx.theme().muted_foreground)
                        .child(progress_text),
                ),
        )
        // Error message row (if present)
        .child(error_row)
}

/// Render format warnings (CSV type loss, BSON tool requirements).
pub(super) fn render_warnings(transfer_state: &TransferTabState, cx: &App) -> AnyElement {
    let mut warnings = Vec::new();

    // Only show format warnings for Export/Import modes (not Copy)
    if matches!(transfer_state.config.mode, TransferMode::Export | TransferMode::Import) {
        // CSV warning
        if matches!(transfer_state.config.format, TransferFormat::Csv) {
            warnings.push("CSV export will lose BSON type fidelity (dates, ObjectIds, etc.)");
        }

        // BSON warning - only show if tools are NOT available
        if matches!(transfer_state.config.format, TransferFormat::Bson) && !tools_available() {
            warnings.push("BSON format requires mongodump/mongorestore. Run: just download-tools");
        }
    }

    if warnings.is_empty() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .mb(spacing::md())
        .children(warnings.into_iter().map(|warning| {
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .px(spacing::md())
                .py(spacing::sm())
                .bg(colors::bg_warning(cx))
                .border_1()
                .border_color(colors::border_warning(cx))
                .rounded(borders::radius_sm())
                .child(Icon::new(IconName::Info).xsmall().text_color(cx.theme().warning))
                .child(div().text_sm().text_color(cx.theme().warning).child(warning))
        }))
        .into_any_element()
}
