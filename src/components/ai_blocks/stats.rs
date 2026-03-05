use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::description_list::{DescriptionItem, DescriptionList};
use serde::Deserialize;

use crate::theme::spacing;

#[derive(Deserialize)]
struct StatsData {
    title: Option<String>,
    metrics: Vec<Metric>,
}

#[derive(Deserialize)]
struct Metric {
    label: String,
    value: String,
}

pub fn render_stats(json: &str, cx: &App) -> Option<AnyElement> {
    let stats: StatsData = serde_json::from_str(json).ok()?;
    if stats.metrics.is_empty() {
        return None;
    }

    let list = stats
        .metrics
        .into_iter()
        .fold(DescriptionList::new().columns(2).bordered(true), |list, m| {
            list.child(DescriptionItem::new(m.label).value(m.value))
        });

    let mut container = div().w_full().overflow_hidden();

    if let Some(title) = stats.title {
        container = container.child(
            div()
                .mb(spacing::xs())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(title),
        );
    }

    Some(container.child(list).into_any_element())
}
