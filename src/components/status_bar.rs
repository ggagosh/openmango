use gpui::*;
use gpui::prelude::FluentBuilder as _;

use crate::state::{StatusLevel, StatusMessage};
use crate::theme::{borders, colors, sizing, spacing};

#[derive(IntoElement)]
pub struct StatusBar {
    is_connected: bool,
    connection_name: Option<String>,
    status_message: Option<StatusMessage>,
    read_only: bool,
}

impl StatusBar {
    pub fn new(
        is_connected: bool,
        connection_name: Option<String>,
        status_message: Option<StatusMessage>,
        read_only: bool,
    ) -> Self {
        Self { is_connected, connection_name, status_message, read_only }
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (status_color, status_text) = if self.is_connected {
            (
                colors::status_success(),
                self.connection_name.as_deref().unwrap_or("Connected").to_string(),
            )
        } else {
            (colors::text_muted(), "Not connected".to_string())
        };

        let status_right = match self.status_message {
            Some(message) => match message.level {
                StatusLevel::Info => {
                    div().text_xs().text_color(colors::text_secondary()).child(message.text)
                }
                StatusLevel::Error => {
                    div().text_xs().text_color(colors::text_muted()).child("v0.1.0")
                }
            },
            None => div().text_xs().text_color(colors::text_muted()).child("v0.1.0"),
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .h(sizing::status_bar_height())
            .px(spacing::md())
            .bg(colors::bg_header())
            .border_t_1()
            .border_color(colors::border())
            .child(
                // Left side: connection status
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .w(sizing::status_dot())
                            .h(sizing::status_dot())
                            .rounded_full()
                            .bg(status_color),
                    )
                    .child(div().text_xs().text_color(colors::text_primary()).child(status_text))
                    .when(self.read_only && self.is_connected, |s: Div| {
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
                    }),
            )
            .child(status_right)
    }
}
