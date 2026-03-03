use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;

use crate::theme::spacing;

const MAX_CELL_CHARS: usize = 40;
const MAX_COLUMNS: usize = 8;
const MIN_COL_WIDTH: f32 = 80.0;

/// Render a JSON array of objects as a div-based table.
pub fn render_datatable(json: &str, cx: &App) -> Option<AnyElement> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let array = value.as_array()?;
    if array.is_empty() {
        return None;
    }

    // Extract column names from union of all object keys, preserving insertion order.
    let mut columns: Vec<String> = Vec::new();
    for item in array {
        if let Some(obj) = item.as_object() {
            for key in obj.keys() {
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    if columns.is_empty() {
        return None;
    }

    let hidden_cols = columns.len().saturating_sub(MAX_COLUMNS);
    let columns: Vec<String> = columns.into_iter().take(MAX_COLUMNS).collect();

    let border = cx.theme().border;
    let fg = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let head_bg = cx.theme().table_head;
    let stripe_bg = cx.theme().table_active;

    // Header row
    let header =
        div().flex().flex_shrink_0().bg(head_bg).border_b_1().border_color(border).children(
            columns.iter().map(|col| {
                div()
                    .min_w(px(MIN_COL_WIDTH))
                    .flex_1()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .overflow_x_hidden()
                    .text_ellipsis()
                    .child(col.clone())
            }),
        );

    // Data rows (capped at 50)
    let rows: Vec<AnyElement> = array
        .iter()
        .take(50)
        .enumerate()
        .map(|(i, item)| {
            let obj = item.as_object();
            let row = div()
                .flex()
                .flex_shrink_0()
                .when(i % 2 == 1, |d: Div| d.bg(stripe_bg))
                .border_b_1()
                .border_color(border)
                .children(columns.iter().map(|col| {
                    let cell_text =
                        obj.and_then(|o| o.get(col)).map(format_value).unwrap_or_default();
                    div()
                        .min_w(px(MIN_COL_WIDTH))
                        .flex_1()
                        .px(spacing::sm())
                        .py(spacing::xs())
                        .text_xs()
                        .text_color(muted)
                        .overflow_x_hidden()
                        .text_ellipsis()
                        .child(cell_text)
                }));
            row.into_any_element()
        })
        .collect();

    let footer_note = if array.len() > 50 || hidden_cols > 0 {
        let mut parts = Vec::new();
        if array.len() > 50 {
            parts.push(format!("{} more rows", array.len() - 50));
        }
        if hidden_cols > 0 {
            parts.push(format!("{hidden_cols} more columns"));
        }
        Some(
            div()
                .px(spacing::sm())
                .py(spacing::xs())
                .text_xs()
                .text_color(muted)
                .child(format!("... and {}", parts.join(", "))),
        )
    } else {
        None
    };

    Some(
        div()
            .w_full()
            .border_1()
            .border_color(border)
            .rounded(px(4.0))
            .overflow_x_hidden()
            .my(spacing::sm())
            .child(header)
            .children(rows)
            .children(footer_note)
            .into_any_element(),
    )
}

fn format_value(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(a) => format!("[{} items]", a.len()),
        serde_json::Value::Object(o) => format!("{{{} fields}}", o.len()),
    };
    truncate(&s, MAX_CELL_CHARS)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{truncated}…")
    }
}
