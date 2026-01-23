use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::theme::{borders, colors, sizing, spacing};

pub(crate) fn render_status_left(
    is_connected: bool,
    connection_name: Option<String>,
    read_only: bool,
) -> AnyElement {
    let (status_color, status_text) = if is_connected {
        (colors::status_success(), connection_name.as_deref().unwrap_or("Connected").to_string())
    } else {
        (colors::text_muted(), "Not connected".to_string())
    };

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            div().w(sizing::status_dot()).h(sizing::status_dot()).rounded_full().bg(status_color),
        )
        .child(div().text_xs().text_color(colors::text_primary()).child(status_text))
        .when(read_only && is_connected, |s: Div| {
            s.child(
                div()
                    .px(spacing::xs())
                    .py(px(1.0))
                    .rounded(borders::radius_sm())
                    .bg(colors::status_warning())
                    .text_xs()
                    .text_color(colors::bg_header())
                    .child("READ-ONLY"),
            )
        })
        .into_any_element()
}
