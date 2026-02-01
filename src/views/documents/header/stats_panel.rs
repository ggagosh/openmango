//! Stats row rendering for collection header.

use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::spinner::Spinner;

use crate::components::Button;
use crate::helpers::{format_bytes, format_number};
use crate::state::{AppCommands, AppState, CollectionStats, SessionKey};
use crate::theme::{colors, spacing};

/// Render the stats row with collection statistics.
pub fn render_stats_row(
    stats: Option<CollectionStats>,
    stats_loading: bool,
    stats_error: Option<String>,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
) -> AnyElement {
    let mut row = div()
        .flex()
        .items_center()
        .gap(spacing::lg())
        .px(spacing::md())
        .py(spacing::sm())
        .bg(colors::bg_header())
        .border_t_1()
        .border_color(colors::border());

    if stats_loading {
        row = row
            .child(Spinner::new().small())
            .child(div().text_sm().text_color(colors::text_muted()).child("Loading stats..."));
        return row.into_any_element();
    }

    if let Some(error) = stats_error {
        row = row.child(div().text_sm().text_color(colors::text_error()).child(error)).child(
            Button::new("retry-stats")
                .ghost()
                .compact()
                .label("Retry")
                .disabled(session_key.is_none())
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        AppCommands::load_collection_stats(state.clone(), session_key, cx);
                    }
                }),
        );
        return row.into_any_element();
    }

    let Some(stats) = stats else {
        row =
            row.child(div().text_sm().text_color(colors::text_muted()).child("No stats available"));
        return row.into_any_element();
    };

    row = row
        .child(stat_cell("Documents", format_number(stats.document_count)))
        .child(stat_cell("Avg size", format_bytes(stats.avg_obj_size)))
        .child(stat_cell("Data size", format_bytes(stats.data_size)))
        .child(stat_cell("Storage", format_bytes(stats.storage_size)))
        .child(stat_cell("Index size", format_bytes(stats.total_index_size)))
        .child(stat_cell("Indexes", format_number(stats.index_count)))
        .child(stat_cell(
            "Capped",
            if stats.capped { "Yes".to_string() } else { "No".to_string() },
        ));

    if let Some(max_size) = stats.max_size {
        row = row.child(stat_cell("Max size", format_bytes(max_size)));
    }

    row.into_any_element()
}

/// Render a single stat cell with label and value.
pub fn stat_cell(label: &str, value: String) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_xs().text_color(colors::text_muted()).child(label.to_string()))
        .child(div().text_sm().text_color(colors::text_primary()).child(value))
        .into_any_element()
}
