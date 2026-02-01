//! Progress panel for database-scope transfer operations.

use gpui::*;
use gpui_component::progress::Progress;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{Icon, IconName, Sizable as _};
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
                        && let Some(ref mut db_progress) = tab.database_progress
                    {
                        db_progress.panel_expanded = !db_progress.panel_expanded;
                    }
                    cx.notify();
                });
            })
            .child(
                Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(colors::text_muted()),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::text_secondary())
                    .child("Progress Details"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(colors::text_muted())
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
            .children(db_progress.collections.iter().map(render_collection_progress_row))
            .into_any_element()
    } else {
        div().into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .p(spacing::md())
        .bg(colors::bg_sidebar())
        .border_1()
        .border_color(colors::border_subtle())
        .rounded(borders::radius_sm())
        .child(header)
        .child(content)
}

/// Render a single collection progress row.
fn render_collection_progress_row(coll: &CollectionProgress) -> impl IntoElement {
    // Status indicator (icon + color)
    let (status_icon, status_color) = match &coll.status {
        CollectionTransferStatus::Pending => (IconName::ChevronRight, colors::text_muted()),
        CollectionTransferStatus::InProgress => (IconName::ArrowRight, colors::syntax_string()),
        CollectionTransferStatus::Completed => (IconName::Check, colors::status_success()),
        CollectionTransferStatus::Failed(_) => (IconName::Close, colors::status_error()),
        CollectionTransferStatus::Cancelled => (IconName::Close, colors::status_warning()),
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
            .text_color(colors::status_error())
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
                        .text_color(colors::text_primary())
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
                        .text_color(colors::text_muted())
                        .child(progress_text),
                ),
        )
        // Error message row (if present)
        .child(error_row)
}

/// Render format warnings (CSV type loss, BSON tool requirements).
pub(super) fn render_warnings(transfer_state: &TransferTabState) -> impl IntoElement {
    let mut warnings = Vec::new();

    // Only show format warnings for Export/Import modes (not Copy)
    if matches!(transfer_state.mode, TransferMode::Export | TransferMode::Import) {
        // CSV warning
        if matches!(transfer_state.format, TransferFormat::Csv) {
            warnings.push("CSV export will lose BSON type fidelity (dates, ObjectIds, etc.)");
        }

        // BSON warning - only show if tools are NOT available
        if matches!(transfer_state.format, TransferFormat::Bson) && !tools_available() {
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
                .bg(hsla(0.12, 0.9, 0.5, 0.1))
                .border_1()
                .border_color(hsla(0.12, 0.9, 0.5, 0.3))
                .rounded(borders::radius_sm())
                .child(Icon::new(IconName::Info).xsmall().text_color(hsla(0.12, 0.9, 0.5, 1.0)))
                .child(div().text_sm().text_color(hsla(0.12, 0.9, 0.5, 1.0)).child(warning))
        }))
        .into_any_element()
}
