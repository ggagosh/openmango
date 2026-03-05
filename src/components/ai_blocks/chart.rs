use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::chart::{BarChart, LineChart, PieChart};
use serde::Deserialize;

use crate::theme::spacing;

#[derive(Deserialize)]
struct ChartData {
    title: Option<String>,
    data: Vec<DataPoint>,
}

#[derive(Deserialize, Clone)]
struct DataPoint {
    label: String,
    value: f64,
}

/// Wraps a DataPoint with its ordinal index so color callbacks can use it.
#[derive(Clone)]
struct IndexedPoint {
    index: usize,
    label: String,
    value: f64,
}

fn palette(cx: &App) -> Vec<Hsla> {
    let t = cx.theme();
    vec![t.chart_1, t.chart_2, t.chart_3, t.chart_4, t.chart_5]
}

struct LegendEntry {
    color: Hsla,
    label: String,
}

fn chart_wrapper(
    title: Option<&str>,
    chart_el: AnyElement,
    legend: &[LegendEntry],
    cx: &App,
) -> AnyElement {
    let base_height: f32 = if title.is_some() { 300.0 } else { 250.0 };
    let legend_rows = if legend.is_empty() { 0 } else { legend.len().div_ceil(3) };
    let height = base_height + legend_rows as f32 * 22.0;

    let mut container = div()
        .flex()
        .flex_col()
        .w_full()
        .h(px(height))
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(4.0))
        .overflow_hidden();

    if let Some(title) = title {
        container = container.child(
            div()
                .px(spacing::sm())
                .pt(spacing::sm())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(title.to_string()),
        );
    }

    container = container.child(div().flex_1().min_h(px(0.0)).p(spacing::sm()).child(chart_el));

    if !legend.is_empty() {
        let legend_el = div()
            .flex()
            .flex_wrap()
            .gap(spacing::md())
            .px(spacing::sm())
            .pb(spacing::sm())
            .children(legend.iter().map(|entry| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded(px(2.0))
                            .bg(entry.color)
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(entry.label.clone()),
                    )
            }));
        container = container.child(legend_el);
    }

    container.into_any_element()
}

pub fn render_bar(json: &str, cx: &App) -> Option<AnyElement> {
    let chart_data: ChartData = serde_json::from_str(json).ok()?;
    if chart_data.data.is_empty() {
        return None;
    }

    // Bar charts: x-axis already shows labels, so use a single color and no legend.
    let primary = cx.theme().primary;
    let chart = BarChart::new(chart_data.data.clone())
        .x(|d: &DataPoint| d.label.clone())
        .y(|d: &DataPoint| d.value)
        .fill(move |_: &DataPoint| primary);

    Some(chart_wrapper(chart_data.title.as_deref(), chart.into_any_element(), &[], cx))
}

pub fn render_pie(json: &str, cx: &App) -> Option<AnyElement> {
    let chart_data: ChartData = serde_json::from_str(json).ok()?;
    if chart_data.data.is_empty() {
        return None;
    }

    let colors = palette(cx);

    // Build indexed data so the color callback can use ordinal position.
    let indexed: Vec<IndexedPoint> = chart_data
        .data
        .iter()
        .enumerate()
        .map(|(i, d)| IndexedPoint { index: i, label: d.label.clone(), value: d.value })
        .collect();

    let colors_for_chart = colors.clone();
    let chart = PieChart::new(indexed.clone())
        .value(|d: &IndexedPoint| d.value as f32)
        .color(move |d: &IndexedPoint| colors_for_chart[d.index % colors_for_chart.len()])
        .inner_radius(40.0)
        .outer_radius(90.0);

    let legend: Vec<LegendEntry> = indexed
        .iter()
        .map(|d| LegendEntry { color: colors[d.index % colors.len()], label: d.label.clone() })
        .collect();

    Some(chart_wrapper(chart_data.title.as_deref(), chart.into_any_element(), &legend, cx))
}

pub fn render_line(json: &str, cx: &App) -> Option<AnyElement> {
    let chart_data: ChartData = serde_json::from_str(json).ok()?;
    if chart_data.data.is_empty() {
        return None;
    }

    let chart = LineChart::new(chart_data.data.clone())
        .x(|d: &DataPoint| d.label.clone())
        .y(|d: &DataPoint| d.value)
        .stroke(cx.theme().primary)
        .dot()
        .natural();

    Some(chart_wrapper(chart_data.title.as_deref(), chart.into_any_element(), &[], cx))
}
