use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mongodb::bson::Bson;

use crate::bson::bson_value_preview;
use crate::theme::colors;

const CELL_PREVIEW_MAX_LEN: usize = 80;
const NESTED_TOOLTIP_PREVIEW_MAX_LEN: usize = 500;
const NESTED_TOOLTIP_MAX_DEPTH: usize = 6;

pub fn value_color(value: &Bson, cx: &App) -> Hsla {
    match value {
        Bson::String(_) | Bson::Symbol(_) => colors::syntax_string(cx),
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => {
            colors::syntax_number(cx)
        }
        Bson::Boolean(_) => colors::syntax_boolean(cx),
        Bson::Null | Bson::Undefined => colors::syntax_null(cx),
        Bson::ObjectId(_) => colors::syntax_object_id(cx),
        Bson::DateTime(_) | Bson::Timestamp(_) => colors::syntax_date(cx),
        Bson::RegularExpression(_) | Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => {
            colors::syntax_comment(cx)
        }
        Bson::Document(_) | Bson::Array(_) | Bson::Binary(_) => cx.theme().muted_foreground,
        _ => cx.theme().foreground,
    }
}

pub fn render_cell(value: &Bson, row_ix: usize, col_ix: usize, cx: &App) -> AnyElement {
    let text = bson_value_preview(value, CELL_PREVIEW_MAX_LEN);
    let color = value_color(value, cx);
    let is_nested = matches!(value, Bson::Document(_) | Bson::Array(_));

    if is_nested {
        // Build the (bounded) nested preview lazily on hover instead of every
        // frame — the string is only ever shown inside the tooltip. Index-based
        // id avoids a per-cell `format!` allocation each render.
        let value = value.clone();
        div()
            .id(("doc-cell", row_ix * 64 + col_ix))
            .text_xs()
            .text_color(color)
            .cursor_pointer()
            .tooltip(move |_window, cx| {
                let preview = format_nested_preview(&value);
                cx.new(|_cx| gpui_kit::component::tooltip::Tooltip::new(preview)).into()
            })
            .child(text)
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(color)
            .whitespace_nowrap()
            .text_ellipsis()
            .overflow_x_hidden()
            .child(text)
            .into_any_element()
    }
}

fn format_nested_preview(value: &Bson) -> String {
    let mut preview = String::new();
    append_bson_preview(value, &mut preview, 0);
    preview
}

fn append_bson_preview(value: &Bson, out: &mut String, depth: usize) {
    if preview_full(out) {
        return;
    }
    if depth >= NESTED_TOOLTIP_MAX_DEPTH {
        push_limited(out, &bson_value_preview(value, remaining_chars(out).min(120)));
        return;
    }

    match value {
        Bson::Document(doc) => {
            push_limited(out, "{");
            for (idx, (key, child)) in doc.iter().enumerate() {
                if preview_full(out) {
                    break;
                }
                if idx > 0 {
                    push_limited(out, ", ");
                }
                push_limited(out, "\"");
                push_limited(out, &escape_preview_string(key));
                push_limited(out, "\": ");
                append_bson_preview(child, out, depth + 1);
            }
            push_limited(out, "}");
        }
        Bson::Array(values) => {
            push_limited(out, "[");
            for (idx, child) in values.iter().enumerate() {
                if preview_full(out) {
                    break;
                }
                if idx > 0 {
                    push_limited(out, ", ");
                }
                append_bson_preview(child, out, depth + 1);
            }
            push_limited(out, "]");
        }
        Bson::String(value) | Bson::Symbol(value) => {
            push_limited(out, "\"");
            push_limited(out, &escape_preview_string(value));
            push_limited(out, "\"");
        }
        _ => {
            push_limited(out, &bson_value_preview(value, remaining_chars(out).min(120)));
        }
    }
}

fn push_limited(out: &mut String, chunk: &str) -> bool {
    let remaining = remaining_chars(out);
    if remaining == 0 {
        return false;
    }

    let chunk_len = chunk.chars().count();
    if chunk_len <= remaining {
        out.push_str(chunk);
        return true;
    }

    if remaining <= 3 {
        for _ in 0..remaining {
            out.push('.');
        }
    } else {
        for ch in chunk.chars().take(remaining - 3) {
            out.push(ch);
        }
        out.push_str("...");
    }
    false
}

fn remaining_chars(out: &str) -> usize {
    NESTED_TOOLTIP_PREVIEW_MAX_LEN.saturating_sub(out.chars().count())
}

fn preview_full(out: &str) -> bool {
    remaining_chars(out) == 0
}

fn escape_preview_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push('?'),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc};

    #[test]
    fn nested_tooltip_preview_includes_nested_content() {
        let value = Bson::Document(doc! {
            "name": "alice",
            "profile": {
                "age": 42,
                "city": "Tbilisi",
            },
            "tags": ["admin", "beta"],
        });

        let preview = super::format_nested_preview(&value);

        assert!(preview.contains("\"name\": \"alice\""));
        assert!(preview.contains("\"profile\""));
        assert!(preview.contains("\"age\": 42"));
        assert!(preview.contains("\"tags\""));
        assert_ne!(preview, crate::bson::bson_value_preview(&value, super::CELL_PREVIEW_MAX_LEN));
    }

    #[test]
    fn nested_tooltip_preview_is_bounded() {
        let value = Bson::Document(doc! {
            "long": "x".repeat(1_000),
            "nested": {
                "long": "y".repeat(1_000),
            },
        });

        let preview = super::format_nested_preview(&value);

        assert!(preview.chars().count() <= super::NESTED_TOOLTIP_PREVIEW_MAX_LEN);
    }
}
