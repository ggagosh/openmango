use gpui::*;

use crate::state::StatusMessage;
use crate::theme::{colors, sizing, spacing};

mod left;
mod right;

use left::render_status_left;
use right::render_status_right;

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
            .child(render_status_left(self.is_connected, self.connection_name, self.read_only))
            .child(render_status_right(self.status_message))
    }
}
