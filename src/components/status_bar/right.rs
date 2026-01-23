use gpui::*;

use crate::state::{StatusLevel, StatusMessage};
use crate::theme::colors;

pub(crate) fn render_status_right(status_message: Option<StatusMessage>) -> AnyElement {
    match status_message {
        Some(message) => match message.level {
            StatusLevel::Info => {
                div().text_xs().text_color(colors::text_secondary()).child(message.text)
            }
            StatusLevel::Error => div()
                .text_xs()
                .text_color(colors::text_muted())
                .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
        },
        None => div()
            .text_xs()
            .text_color(colors::text_muted())
            .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
    }
    .into_any_element()
}
