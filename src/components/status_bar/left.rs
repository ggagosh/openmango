use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::theme::{borders, sizing, spacing};

pub(crate) fn render_status_left(
    is_connected: bool,
    connection_name: Option<String>,
    read_only: bool,
    sidebar_collapsed: bool,
    on_toggle_sidebar: super::ToggleSidebarHandler,
    cx: &App,
) -> AnyElement {
    let (status_color, status_text) = if is_connected {
        (cx.theme().success, connection_name.as_deref().unwrap_or("Connected").to_string())
    } else {
        (cx.theme().muted_foreground, "Not connected".to_string())
    };

    let sidebar_icon =
        if sidebar_collapsed { IconName::PanelLeftOpen } else { IconName::PanelLeftClose };

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .child(
            div()
                .id("toggle-sidebar-btn")
                .flex()
                .items_center()
                .justify_center()
                .w(sizing::icon_lg())
                .h(sizing::icon_lg())
                .rounded(borders::radius_sm())
                .cursor_pointer()
                .hover(|s| s.bg(cx.theme().list_hover))
                .text_color(cx.theme().secondary_foreground)
                .child(Icon::new(sidebar_icon).xsmall())
                .when_some(on_toggle_sidebar, |el, handler| {
                    el.on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        handler(window, cx);
                    })
                }),
        )
        .child(
            div().w(sizing::status_dot()).h(sizing::status_dot()).rounded_full().bg(status_color),
        )
        .child(div().text_xs().text_color(cx.theme().foreground).child(status_text))
        .when(read_only && is_connected, |s: Div| {
            s.child(
                div()
                    .px(spacing::xs())
                    .py(px(1.0))
                    .rounded(borders::radius_sm())
                    .bg(cx.theme().warning)
                    .text_xs()
                    .text_color(cx.theme().tab_bar)
                    .child("READ-ONLY"),
            )
        })
        .into_any_element()
}
