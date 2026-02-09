use gpui::*;
use gpui_component::ActiveTheme as _;

use crate::theme::spacing;

pub(crate) fn render_empty_state(hint: String, cx: &App) -> AnyElement {
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
                        .text_color(cx.theme().primary)
                        .font_family(crate::theme::fonts::heading())
                        .child("OpenMango"),
                )
                .child(
                    div()
                        .text_base()
                        .text_color(cx.theme().secondary_foreground)
                        .child("MongoDB GUI Client"),
                )
                .child(
                    div()
                        .mt(spacing::lg())
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                ),
        )
        .into_any_element()
}
