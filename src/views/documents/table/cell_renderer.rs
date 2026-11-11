use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use std::borrow::Cow;
use std::ops::Range;

use mongodb::bson::Bson;

use crate::bson::{bson_value_details, bson_value_preview, has_value_details, is_relaxed_key};
use crate::theme::{colors, spacing};

const CELL_PREVIEW_MAX_LEN: usize = 80;
/// Lines in a nested value's hover card, the cut line included.
const NESTED_TOOLTIP_MAX_LINES: usize = 16;
/// A document or array this wide or narrower stays on one line in the hover card.
const NESTED_TOOLTIP_INLINE_WIDTH: usize = 60;
const SCALAR_PREVIEW_MAX_LEN: usize = 120;

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

/// The hover card for a date or binary value: every other way to read it, labels in one column
/// and readings in the next so they line up. Built when the card opens, never per frame.
pub fn value_details_tooltip(value: &Bson, window: &mut Window, cx: &mut App) -> AnyView {
    let rows = bson_value_details(value);
    gpui_kit::component::tooltip::Tooltip::element(move |_window, cx| {
        let label_color = cx.theme().muted_foreground;
        div()
            .flex()
            .gap(spacing::md())
            .text_xs()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_color(label_color)
                    .children(rows.iter().map(|(label, _)| div().child(*label))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .children(rows.iter().map(|(_, text)| div().child(text.clone()))),
            )
    })
    .build(window, cx)
}

/// The text a cell draws, which is also what `TableDelegate::cell_label` reads to a screen reader.
pub fn cell_text(value: &Bson) -> String {
    bson_value_preview(value, CELL_PREVIEW_MAX_LEN)
}

pub fn render_cell(value: &Bson, row_ix: usize, col_ix: usize, cx: &App) -> AnyElement {
    let text = cell_text(value);
    let color = value_color(value, cx);
    let is_nested = matches!(value, Bson::Document(_) | Bson::Array(_));

    if has_value_details(value) {
        // ponytail: clones the value for the card on each render, as nested cells below do. If
        // multi-megabyte binaries ever make the table stutter, capture the row and column and
        // resolve the value on hover instead.
        let value = value.clone();
        div()
            .id(("doc-cell", row_ix * 64 + col_ix))
            .text_xs()
            .text_color(color)
            .whitespace_nowrap()
            .text_ellipsis()
            .overflow_x_hidden()
            .tooltip(move |window, cx| value_details_tooltip(&value, window, cx))
            .child(text)
            .into_any_element()
    } else if is_nested {
        // Build the (bounded) nested preview lazily on hover instead of every
        // frame — the string is only ever shown inside the tooltip. Index-based
        // id avoids a per-cell `format!` allocation each render.
        let value = value.clone();
        div()
            .id(("doc-cell", row_ix * 64 + col_ix))
            .text_xs()
            .text_color(color)
            .cursor_pointer()
            .tooltip(move |window, cx| nested_value_tooltip(&value, window, cx))
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

/// The width a column's content draws at, for `TableDelegate::fit_column`: the widest of `header`
/// as the tables' `render_th` draws it and of `cells` as `render_cell` does, where a cell's flag
/// adds the link arrow drawn after its value. The table adds the cell padding itself.
pub fn fit_column_width<'a>(
    header: &str,
    cells: impl IntoIterator<Item = (&'a Bson, bool)>,
    window: &Window,
    cx: &App,
) -> Pixels {
    let mut font = window.text_style().font();
    font.family = cx.theme().font_family.clone();
    let rem = window.rem_size();
    let measure = |text: String, size: Rems| {
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window.text_system().shape_line(text.into(), size.to_pixels(rem), &[run], None).width()
    };
    // The header is text_sm, then a gap_1 and the 12px pin icon with 1px padding, then the table's
    // 0.75rem sort icon with 2px padding.
    let header = measure(header.to_string(), rems(0.875))
        + rems(0.25).to_pixels(rem)
        + px(14.0)
        + rems(0.75).to_pixels(rem)
        + px(4.0);
    // The arrow is an xsmall icon button, 1.25rem square, after the cell's gap.
    let arrow = spacing::xs() + rems(1.25).to_pixels(rem);
    cells
        .into_iter()
        .map(|(value, linked)| {
            let text = measure(cell_text(value), rems(0.75));
            if linked { text + arrow } else { text }
        })
        .fold(header, Pixels::max)
}

/// The hover card of a document or array cell, laid out as the JSON view lays it out and colored
/// as the cells are, so a long value reads as a short block instead of one line across the screen.
fn nested_value_tooltip(value: &Bson, window: &mut Window, cx: &mut App) -> AnyView {
    let (text, colors) = format_nested_preview(value, |value| value_color(value, cx));
    let text = SharedString::from(text);
    let highlights: Vec<_> = colors
        .into_iter()
        .map(|(range, color)| (range, HighlightStyle { color: Some(color), ..Default::default() }))
        .collect();
    gpui_kit::component::tooltip::Tooltip::element(move |_window, _cx| {
        div()
            .max_w(px(520.0))
            .font_family(crate::theme::fonts::mono())
            .text_xs()
            .child(StyledText::new(text.clone()).with_highlights(highlights.clone()))
    })
    .build(window, cx)
}

/// `value` one entry per line, indented, with each scalar's byte range and `color` for it. A
/// document or array that fits in `NESTED_TOOLTIP_INLINE_WIDTH` columns stays on one line, as a
/// formatter keeps short arrays. Cut after `NESTED_TOOLTIP_MAX_LINES`: the editor shows the rest.
fn format_nested_preview(
    value: &Bson,
    color: impl Fn(&Bson) -> Hsla,
) -> (String, Vec<(Range<usize>, Hsla)>) {
    let mut preview = NestedPreview { text: String::new(), colors: Vec::new(), lines: 1, color };
    preview.value(value, 0);
    (preview.text, preview.colors)
}

struct NestedPreview<F> {
    text: String,
    colors: Vec<(Range<usize>, Hsla)>,
    lines: usize,
    color: F,
}

impl<F: Fn(&Bson) -> Hsla> NestedPreview<F> {
    /// Writes `value`; false once the preview is cut.
    fn value(&mut self, value: &Bson, indent: usize) -> bool {
        let Some((open, close, entries)) = entries(value) else {
            let start = self.text.len();
            self.text.push_str(&scalar_preview(value));
            self.colors.push((start..self.text.len(), (self.color)(value)));
            return true;
        };
        let line = &self.text[self.text.rfind('\n').map_or(0, |at| at + 1)..];
        let room = NESTED_TOOLTIP_INLINE_WIDTH.saturating_sub(line.chars().count());
        let inline = compact_width(value, room).is_some();
        self.text.push(open);
        let mut empty = true;
        for (key, child) in entries {
            if !empty {
                self.text.push_str(if inline { ", " } else { "," });
            }
            empty = false;
            if !inline && !self.newline(indent + 1) {
                return false;
            }
            if let Some(key) = key {
                self.text.push_str(&key_preview(key));
                self.text.push_str(": ");
            }
            if !self.value(child, indent + 1) {
                return false;
            }
        }
        if !inline && !empty && !self.newline(indent) {
            return false;
        }
        self.text.push(close);
        true
    }

    /// Starts a line at `indent`, or ends the preview with `…` there once it has its lines.
    fn newline(&mut self, indent: usize) -> bool {
        self.text.push('\n');
        self.text.push_str(&"  ".repeat(indent));
        if self.lines + 1 == NESTED_TOOLTIP_MAX_LINES {
            self.text.push('…');
            return false;
        }
        self.lines += 1;
        true
    }
}

type Entries<'a> = Box<dyn Iterator<Item = (Option<&'a String>, &'a Bson)> + 'a>;

/// A document's or array's brackets and entries; `None` for a scalar.
fn entries(value: &Bson) -> Option<(char, char, Entries<'_>)> {
    match value {
        Bson::Document(doc) => Some(('{', '}', Box::new(doc.iter().map(|(k, v)| (Some(k), v))))),
        Bson::Array(items) => Some(('[', ']', Box::new(items.iter().map(|v| (None, v))))),
        _ => None,
    }
}

/// The width `value` takes on one line, if it is at most `budget`. Stops counting past it, so
/// a large array costs no more than a small one.
fn compact_width(value: &Bson, budget: usize) -> Option<usize> {
    let Some((_, _, entries)) = entries(value) else {
        return Some(scalar_preview(value).chars().count()).filter(|width| *width <= budget);
    };
    let mut width = 2;
    for (ix, (key, child)) in entries.enumerate() {
        width += if ix > 0 { 2 } else { 0 };
        width += key.map_or(0, |key| key_preview(key).chars().count() + 2);
        width += compact_width(child, budget.checked_sub(width)?)?;
    }
    Some(width).filter(|width| *width <= budget)
}

/// A key as the JSON view writes it: bare when it is an identifier, quoted otherwise.
fn key_preview(key: &str) -> Cow<'_, str> {
    if is_relaxed_key(key) {
        Cow::Borrowed(key)
    } else {
        Cow::Owned(format!("\"{}\"", escape_preview_string(key)))
    }
}

fn scalar_preview(value: &Bson) -> String {
    match value {
        Bson::String(text) | Bson::Symbol(text) => {
            let shown: String = text.chars().take(SCALAR_PREVIEW_MAX_LEN).collect();
            let cut = if shown.len() < text.len() { "…" } else { "" };
            format!("\"{}{cut}\"", escape_preview_string(&shown))
        }
        _ => bson_value_preview(value, SCALAR_PREVIEW_MAX_LEN),
    }
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

    fn preview(value: Bson) -> String {
        super::format_nested_preview(&value, |_| gpui_kit::Hsla::default()).0
    }

    #[test]
    fn nested_tooltip_lays_out_one_entry_per_line_and_keeps_short_ones_inline() {
        let value = Bson::Document(doc! {
            "name": "alice",
            "profile": { "age": 42, "city": "Tbilisi" },
            "tags": ["admin", "beta"],
            "a key": "line one\nline two that is long enough to need a line of its own",
        });
        assert_eq!(
            preview(value),
            "{\n  name: \"alice\",\n  profile: {age: 42, city: \"Tbilisi\"},\n  \
             tags: [\"admin\", \"beta\"],\n  \
             \"a key\": \"line one\\nline two that is long enough to need a line of its own\"\n}"
        );
        assert_eq!(preview(Bson::Array(vec![])), "[]");
        assert_eq!(preview(Bson::Array(vec![1.into(), 2.into()])), "[1, 2]");
    }

    #[test]
    fn nested_tooltip_is_cut_after_a_screenful_of_short_lines() {
        let items = (0..1_000).map(|n| Bson::String(format!("{n:>3}{}", "x".repeat(1_000))));
        let text = preview(Bson::Array(items.collect()));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), super::NESTED_TOOLTIP_MAX_LINES);
        assert_eq!(lines.last(), Some(&"  …"));
        assert!(lines.iter().all(|line| line.chars().count() <= super::SCALAR_PREVIEW_MAX_LEN + 8));
    }

    #[test]
    fn nested_tooltip_colors_each_value() {
        let value = Bson::Document(doc! { "n": 1, "s": "x" });
        let (text, colors) = super::format_nested_preview(&value, |value| match value {
            Bson::Int32(_) => gpui_kit::red(),
            _ => gpui_kit::blue(),
        });
        let colored: Vec<_> =
            colors.iter().map(|(range, color)| (&text[range.clone()], *color)).collect();
        assert_eq!(colored, [("1", gpui_kit::red()), ("\"x\"", gpui_kit::blue())]);
    }
}
