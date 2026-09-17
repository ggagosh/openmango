use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::status_bar::StatusBar as KitStatusBar;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::Button;
use crate::keyboard::ToggleAiPanel;
use crate::state::app_state::updater::UpdateStatus;
use crate::state::{AppState, StatusLevel, StatusMessage};
use crate::theme::{borders, islands, sizing, spacing};

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
    ai_available: bool,
    ai_panel_open: bool,
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
            ai_available: false,
            ai_panel_open: false,
            on_toggle_sidebar: None,
        }
    }

    pub fn ai_state(mut self, available: bool, panel_open: bool) -> Self {
        self.ai_available = available;
        self.ai_panel_open = panel_open;
        self
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let appearance = self.state.read(cx).settings.appearance.clone();
        let (status_color, status_text) = if self.is_connected {
            (cx.theme().success, self.connection_name.unwrap_or_else(|| "Connected".to_string()))
        } else {
            (cx.theme().muted_foreground, "Not connected".to_string())
        };
        let (error_count, unseen_errors) = {
            let state = self.state.read(cx);
            (state.error_count(), state.unseen_error_count())
        };
        let ai_tooltip = match crate::keyboard::shortcut_label(window, &ToggleAiPanel) {
            Some(shortcut) => format!("Toggle AI Assistant ({shortcut})"),
            None => "Toggle AI Assistant".to_string(),
        };

        // The kit bar supplies the regions and the text style; the overrides keep the island.
        KitStatusBar::new()
            .flex_shrink_0()
            .h(sizing::status_bar_height())
            .py_0()
            .px(spacing::md())
            .mx(spacing::xs())
            .mb(spacing::xs())
            .rounded(islands::radius_sm(&appearance))
            .border_1()
            .border_color(islands::panel_border(&appearance, cx))
            .bg(islands::tool_bg(&appearance, cx))
            .left(
                Button::new("toggle-sidebar-btn")
                    .ghost()
                    .xsmall()
                    .icon(
                        Icon::new(if self.sidebar_collapsed {
                            IconName::PanelLeftOpen
                        } else {
                            IconName::PanelLeftClose
                        })
                        .xsmall(),
                    )
                    .tooltip(if self.sidebar_collapsed { "Show sidebar" } else { "Hide sidebar" })
                    .when_some(self.on_toggle_sidebar, |button, handler| {
                        button.on_click(move |_, window, cx| handler(window, cx))
                    }),
            )
            .left(div().flex_shrink_0().size(sizing::status_dot()).rounded_full().bg(status_color))
            .left(div().text_color(cx.theme().foreground).child(status_text))
            .when(self.read_only && self.is_connected, |bar| {
                bar.left(
                    div()
                        .px(spacing::xs())
                        .py(px(1.0))
                        .rounded(borders::radius_sm())
                        .bg(cx.theme().warning)
                        .text_color(cx.theme().tab_bar)
                        .child("READ-ONLY"),
                )
            })
            .when(error_count > 0, |bar| {
                let state = self.state.clone();
                bar.right(
                    Button::new("status-error-history")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::CircleX))
                        .label(if error_count == 1 {
                            "1 error".to_string()
                        } else {
                            format!("{error_count} errors")
                        })
                        .tooltip("Errors this session")
                        .text_color(if unseen_errors > 0 {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .on_click(move |_, window, cx| {
                            crate::components::error_history::open_error_history(
                                state.clone(),
                                window,
                                cx,
                            )
                        }),
                )
            })
            .map(|bar| match &self.update_status {
                UpdateStatus::Idle
                | UpdateStatus::UpToDate { .. }
                | UpdateStatus::Unavailable(_) => bar.right(SharedString::from(
                    self.status_message
                        .filter(|message| matches!(message.level, StatusLevel::Info))
                        .map(|message| message.text)
                        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION"))),
                )),
                status => {
                    let state = self.state.clone();
                    bar.right(
                        Button::new("software-update-status")
                            .ghost()
                            .xsmall()
                            .label(crate::components::updater::status_label(status))
                            .tooltip("View software update details")
                            .when(matches!(status, UpdateStatus::Failed { .. }), |button| {
                                button.text_color(cx.theme().danger)
                            })
                            .on_click(move |_, window, cx| {
                                crate::components::updater::open_updates(state.clone(), window, cx)
                            }),
                    )
                }
            })
            .when(self.ai_available, |bar| {
                let state = self.state.clone();
                bar.right(
                    Button::new("ai-toggle")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Bot).text_color(if self.ai_panel_open {
                            cx.theme().primary
                        } else {
                            cx.theme().muted_foreground
                        }))
                        .tooltip(ai_tooltip)
                        .on_click(move |_, _, cx| {
                            state.update(cx, |state, cx| state.toggle_ai_panel(cx));
                        }),
                )
            })
    }
}
