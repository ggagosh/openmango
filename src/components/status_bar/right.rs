use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::app_state::updater::UpdateStatus;
use crate::state::{AppState, StatusLevel, StatusMessage};

pub(crate) fn render_status_right(
    status_message: Option<StatusMessage>,
    update_status: UpdateStatus,
    ai_available: bool,
    ai_panel_open: bool,
    state: Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let state_for_ai = state.clone();
    let inner = match &update_status {
        UpdateStatus::Idle | UpdateStatus::UpToDate { .. } | UpdateStatus::Unavailable(_) => {
            let label = status_message
                .filter(|message| matches!(message.level, StatusLevel::Info))
                .map(|message| message.text)
                .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
            div().text_xs().text_color(cx.theme().muted_foreground).child(label).into_any_element()
        }
        _ => crate::components::Button::new("software-update-status")
            .ghost()
            .xsmall()
            .label(crate::components::updater::status_label(&update_status))
            .tooltip("View software update details")
            .when(matches!(update_status, UpdateStatus::Failed { .. }), |button| {
                button.text_color(cx.theme().danger)
            })
            .on_click(move |_, window, cx| {
                crate::components::updater::open_updates(state.clone(), window, cx)
            })
            .into_any_element(),
    };

    let ai_icon_color =
        if ai_panel_open { cx.theme().primary } else { cx.theme().muted_foreground };

    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(inner)
        .when(ai_available, |this: Div| {
            this.child(
                div()
                    .id("ai-toggle")
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .tooltip(|window, cx| {
                        Tooltip::new("Toggle AI Assistant (⌘L)").build(window, cx)
                    })
                    .child(Icon::new(IconName::Bot).with_size(px(14.0)).text_color(ai_icon_color))
                    .on_click(move |_, _window, cx| {
                        state_for_ai.update(cx, |state, cx| {
                            state.toggle_ai_panel(cx);
                        });
                    }),
            )
        })
        .into_any_element()
}
