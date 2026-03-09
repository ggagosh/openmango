use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::{Icon, IconName};

use crate::ai::blocks::ReportSheet;
use crate::components::Button;
use crate::theme::spacing;

const MAX_CELL_CHARS: usize = 40;
const MAX_COLUMNS: usize = 10;
const MIN_COL_WIDTH: f32 = 80.0;
const MAX_COL_WIDTH: f32 = 250.0;
const CHAR_WIDTH: f32 = 7.0;
const CELL_PAD: f32 = 16.0;
const MAX_TABLE_H: f32 = 280.0;

pub type DownloadHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

pub fn render_report_preview(
    title: &str,
    sheets: &[ReportSheet],
    id: ElementId,
    on_download: Option<DownloadHandler>,
    cx: &App,
) -> AnyElement {
    if sheets.is_empty() {
        return div().into_any_element();
    }

    let border = cx.theme().border.opacity(0.78);
    let fg = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;

    let mut title_bar = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .px(spacing::md())
        .py(spacing::sm())
        .border_b_1()
        .border_color(border)
        .bg(cx.theme().table_head.opacity(0.5))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(title.to_string()),
        );

    if let Some(on_dl) = on_download {
        title_bar = title_bar.child(
            Button::new(ElementId::Name(format!("{}-dl", id).into()))
                .primary()
                .compact()
                .icon(Icon::new(IconName::Download).xsmall())
                .label("Download Excel")
                .on_click(move |ev, window, cx| on_dl(ev, window, cx)),
        );
    }

    let mut container = div()
        .id(id)
        .w_full()
        .border_1()
        .border_color(border)
        .rounded(px(8.0))
        .bg(cx.theme().table.opacity(0.55))
        .overflow_hidden()
        .child(title_bar);

    for (sheet_idx, sheet) in sheets.iter().enumerate() {
        let sheet_el = render_sheet_preview(sheet, sheet_idx, sheets.len() > 1, cx);
        container = container.child(sheet_el);
    }

    let total_rows: usize = sheets.iter().map(|s| s.preview_count).sum();
    let any_has_more = sheets.iter().any(|s| s.has_more);
    let footer_text = if sheets.len() == 1 {
        if any_has_more { format!("{}+ rows", total_rows) } else { format!("{} rows", total_rows) }
    } else {
        let row_label = if any_has_more {
            format!("{}+ rows", total_rows)
        } else {
            format!("{} rows", total_rows)
        };
        format!("{row_label} across {} sheets", sheets.len())
    };

    let footer = div()
        .px(spacing::md())
        .py(spacing::xs())
        .border_t_1()
        .border_color(border)
        .child(div().text_xs().text_color(muted).child(footer_text));

    container = container.child(footer);

    container.into_any_element()
}

fn render_sheet_preview(sheet: &ReportSheet, sheet_idx: usize, show_header: bool, cx: &App) -> Div {
    let border = cx.theme().border.opacity(0.78);
    let fg = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let head_bg = cx.theme().table_head.opacity(0.86);
    let stripe_bg = cx.theme().table_active.opacity(0.55);

    let array: Vec<serde_json::Value> =
        serde_json::from_str(&sheet.preview_json).unwrap_or_default();

    let mut columns: Vec<String> = Vec::new();
    for item in &array {
        if let Some(obj) = item.as_object() {
            for key in obj.keys() {
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    let columns: Vec<String> = columns.into_iter().take(MAX_COLUMNS).collect();

    let col_widths: Vec<f32> = columns
        .iter()
        .map(|col| {
            let header_len = col.chars().count();
            let max_content_len = array
                .iter()
                .take(5)
                .filter_map(|item| item.as_object())
                .filter_map(|obj| obj.get(col))
                .map(|v| format_value(v).chars().count())
                .max()
                .unwrap_or(0);
            let chars = header_len.max(max_content_len) as f32;
            (chars * CHAR_WIDTH + CELL_PAD).clamp(MIN_COL_WIDTH, MAX_COL_WIDTH)
        })
        .collect();

    let total_width: f32 = col_widths.iter().sum();

    let mut section = div().flex().flex_col();

    if show_header {
        section = section.child(
            div()
                .px(spacing::md())
                .py(spacing::xs())
                .border_b_1()
                .border_color(border)
                .bg(cx.theme().background.opacity(0.5))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg.opacity(0.8))
                        .child(sheet.name.clone()),
                ),
        );
    }

    if columns.is_empty() {
        return section.child(
            div().px(spacing::md()).py(spacing::sm()).text_xs().text_color(muted).child("No data"),
        );
    }

    let header =
        div().flex().flex_shrink_0().bg(head_bg).border_b_1().border_color(border).children(
            columns.iter().zip(col_widths.iter()).map(|(col, &w)| {
                div()
                    .w(px(w))
                    .flex_shrink_0()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .whitespace_nowrap()
                    .overflow_x_hidden()
                    .text_ellipsis()
                    .child(col.clone())
            }),
        );

    let rows: Vec<AnyElement> = array
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let obj = item.as_object();
            div()
                .flex()
                .flex_shrink_0()
                .when(i % 2 == 1, |d: Div| d.bg(stripe_bg))
                .border_b_1()
                .border_color(border)
                .children(columns.iter().zip(col_widths.iter()).map(|(col, &w)| {
                    let cell_text =
                        obj.and_then(|o| o.get(col)).map(format_value).unwrap_or_default();
                    div()
                        .w(px(w))
                        .flex_shrink_0()
                        .px(spacing::sm())
                        .py(spacing::xs())
                        .text_xs()
                        .text_color(muted)
                        .overflow_x_hidden()
                        .text_ellipsis()
                        .child(cell_text)
                }))
                .into_any_element()
        })
        .collect();

    let table_content = div().min_w(px(total_width)).child(header).children(rows);

    let table_scroll = div()
        .id(ElementId::Name(format!("rpt-sheet-{sheet_idx}").into()))
        .w_full()
        .max_h(px(MAX_TABLE_H))
        .overflow_scroll()
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .child(table_content);

    section.child(table_scroll)
}

fn format_value(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(a) => format!("[{} items]", a.len()),
        serde_json::Value::Object(o) => {
            if let Some(v) = o.get("$oid").and_then(|v| v.as_str()) {
                return truncate(v);
            }
            if let Some(v) = o.get("$date").and_then(|v| v.as_str()) {
                return truncate(v);
            }
            if let Some(v) = o.get("$numberDecimal").and_then(|v| v.as_str()) {
                return truncate(v);
            }
            if let Some(v) = o.get("$numberLong").and_then(|v| v.as_str()) {
                return truncate(v);
            }
            format!("{{{} fields}}", o.len())
        }
    };
    truncate(&s)
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_CELL_CHARS {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(MAX_CELL_CHARS - 1).collect();
        format!("{truncated}…")
    }
}
