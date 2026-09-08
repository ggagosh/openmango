mod chart;
mod datatable;
pub mod report;
mod stats;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::*;

use crate::ai::blocks::{ChartType, ChatMessage, ContentBlock, parse_content_to_blocks};
use crate::theme::spacing;

fn block_label(block_type: &str) -> &str {
    match block_type {
        "datatable" => "table",
        "barchart" => "bar chart",
        "piechart" => "pie chart",
        "linechart" => "line chart",
        "stats" => "stats",
        "report" => "report",
        _ => "block",
    }
}

/// Render a single ContentBlock to an element.
pub fn render_single_block(
    id_prefix: &str,
    index: usize,
    block: &ContentBlock,
    style: &TextViewStyle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match block {
        ContentBlock::Markdown { text } => {
            let id = ElementId::Name(format!("{id_prefix}-{index}").into());
            TextView::markdown(id, text.clone())
                .selectable(true)
                .style(style.clone())
                .into_any_element()
        }
        ContentBlock::DataTable { json } => match datatable::render_datatable(
            json,
            ElementId::Name(format!("{id_prefix}-dt-{index}").into()),
            cx,
        ) {
            Some(el) => el,
            None => render_code_fallback(id_prefix, index, "datatable", json, style, window, cx),
        },
        ContentBlock::Chart { chart_type, json } => {
            let (lang, rendered) = match chart_type {
                ChartType::Bar => ("barchart", chart::render_bar(json, cx)),
                ChartType::Pie => ("piechart", chart::render_pie(json, cx)),
                ChartType::Line => ("linechart", chart::render_line(json, cx)),
            };
            match rendered {
                Some(el) => el,
                None => render_code_fallback(id_prefix, index, lang, json, style, window, cx),
            }
        }
        ContentBlock::Stats { json } => match stats::render_stats(json, cx) {
            Some(el) => el,
            None => render_code_fallback(id_prefix, index, "stats", json, style, window, cx),
        },
        ContentBlock::Report { title, sheets } => report::render_report_preview(
            title,
            sheets,
            ElementId::Name(format!("{id_prefix}-rpt-{index}").into()),
            None,
            cx,
        ),
        ContentBlock::Pending { block_type } => div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .py(spacing::sm())
            .child(Spinner::new().xsmall())
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Generating {}...", block_label(block_type))),
            )
            .into_any_element(),
    }
}

/// Graceful degradation: render as a normal markdown code block.
fn render_code_fallback(
    id_prefix: &str,
    index: usize,
    lang: &str,
    code: &str,
    style: &TextViewStyle,
    _window: &mut Window,
    _cx: &mut App,
) -> AnyElement {
    let fallback = format!("```{lang}\n{code}\n```");
    let id = ElementId::Name(format!("{id_prefix}-{index}").into());
    TextView::markdown(id, fallback).selectable(true).style(style.clone()).into_any_element()
}

/// Render structured content blocks to elements.
pub fn render_content_blocks(
    id_prefix: &str,
    blocks: &[ContentBlock],
    style: TextViewStyle,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    blocks
        .iter()
        .enumerate()
        .map(|(i, block)| render_single_block(id_prefix, i, block, &style, window, cx))
        .collect()
}

/// Render content blocks from a ChatMessage, with fallback for old messages
/// that have no blocks (parses `content` string as legacy path).
pub fn render_content_blocks_or_fallback(
    id_prefix: &str,
    msg: &ChatMessage,
    style: TextViewStyle,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    if msg.blocks.is_empty() {
        let blocks = parse_content_to_blocks(&msg.content);
        render_content_blocks(id_prefix, &blocks, style, window, cx)
    } else {
        render_content_blocks(id_prefix, &msg.blocks, style, window, cx)
    }
}
