use gpui::*;
use gpui_component::ActiveTheme as _;
use mongodb::bson::Bson;

use crate::bson::bson_value_preview;
use crate::theme::colors;

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

pub fn format_nested_preview(value: &Bson) -> String {
    match value {
        Bson::Document(doc) => {
            let ext = Bson::Document(doc.clone()).into_relaxed_extjson();
            crate::bson::format_relaxed_json_value(&ext)
        }
        Bson::Array(arr) => {
            let ext = Bson::Array(arr.clone()).into_relaxed_extjson();
            crate::bson::format_relaxed_json_value(&ext)
        }
        _ => bson_value_preview(value, 500),
    }
}

pub fn render_cell(value: &Bson, row_ix: usize, col_ix: usize, cx: &App) -> AnyElement {
    let text = bson_value_preview(value, 80);
    let color = value_color(value, cx);
    let is_nested = matches!(value, Bson::Document(_) | Bson::Array(_));

    if is_nested {
        let preview = format_nested_preview(value);
        div()
            .id(ElementId::Name(format!("cell-{}-{}", row_ix, col_ix).into()))
            .text_xs()
            .text_color(color)
            .cursor_pointer()
            .tooltip(move |_window, cx| {
                cx.new(|_cx| gpui_component::tooltip::Tooltip::new(preview.clone())).into()
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
