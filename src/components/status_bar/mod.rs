use gpui::*;
use gpui_component::ActiveTheme as _;

use crate::state::app_state::updater::UpdateStatus;
use crate::state::{AppState, StatusMessage};
use crate::theme::{sizing, spacing};

mod left;
mod right;

use left::render_status_left;
use right::render_status_right;

type ToggleSidebarHandler = Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>;

#[derive(IntoElement)]
pub struct StatusBar {
    is_connected: bool,
    connection_name: Option<String>,
    status_message: Option<StatusMessage>,
    read_only: bool,
    update_status: UpdateStatus,
    state: Entity<AppState>,
    sidebar_collapsed: bool,
    on_toggle_sidebar: ToggleSidebarHandler,
}

impl StatusBar {
    pub fn new(
        is_connected: bool,
        connection_name: Option<String>,
        status_message: Option<StatusMessage>,
        read_only: bool,
        update_status: UpdateStatus,
        state: Entity<AppState>,
    ) -> Self {
        Self {
            is_connected,
            connection_name,
            status_message,
            read_only,
            update_status,
            state,
            sidebar_collapsed: false,
            on_toggle_sidebar: None,
        }
    }

    pub fn sidebar_collapsed(mut self, collapsed: bool) -> Self {
        self.sidebar_collapsed = collapsed;
        self
    }

    pub fn on_toggle_sidebar(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle_sidebar = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .h(sizing::status_bar_height())
            .px(spacing::md())
            .bg(cx.theme().tab_bar)
            .border_t_1()
            .border_color(cx.theme().border)
            .child(render_status_left(
                self.is_connected,
                self.connection_name,
                self.read_only,
                self.sidebar_collapsed,
                self.on_toggle_sidebar,
                cx,
            ))
            .child(render_status_right(self.status_message, self.update_status, self.state, cx))
    }
}
