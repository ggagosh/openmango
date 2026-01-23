use gpui::*;

use crate::theme::{colors, spacing};

pub(crate) fn render_empty_state(hint: String) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::lg())
                .items_center()
                .child(img("logo/openmango-logo.svg").w(px(120.0)).h(px(120.0)))
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors::accent())
                        .font_family(crate::theme::fonts::heading())
                        .child("OpenMango"),
                )
                .child(
                    div()
                        .text_base()
                        .text_color(colors::text_secondary())
                        .child("MongoDB GUI Client"),
                )
                .child(
                    div().mt(spacing::lg()).text_sm().text_color(colors::text_muted()).child(hint),
                ),
        )
        .into_any_element()
}
