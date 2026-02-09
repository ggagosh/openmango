use gpui::*;
use gpui_component::ActiveTheme as _;

use crate::state::app_state::updater::UpdateStatus;
use crate::state::{AppCommands, AppState, StatusLevel, StatusMessage};

pub(crate) fn render_status_right(
    status_message: Option<StatusMessage>,
    update_status: UpdateStatus,
    state: Entity<AppState>,
    cx: &App,
) -> AnyElement {
    match &update_status {
        UpdateStatus::Available { version, .. } => div()
            .id("update-available")
            .cursor_pointer()
            .text_xs()
            .text_color(cx.theme().primary)
            .child(format!("v{version} available \u{2193}"))
            .on_click(move |_, _window, cx| {
                AppCommands::download_update(state.clone(), cx);
            })
            .into_any_element(),
        UpdateStatus::Downloading { progress_pct, .. } => div()
            .text_xs()
            .text_color(cx.theme().secondary_foreground)
            .child(format!("Updating\u{2026} {progress_pct}%"))
            .into_any_element(),
        UpdateStatus::ReadyToInstall { .. } => div()
            .id("update-restart")
            .cursor_pointer()
            .text_xs()
            .text_color(cx.theme().primary)
            .child("Restart to update \u{21BB}")
            .on_click(move |_, _window, cx| {
                AppCommands::install_update(state.clone(), cx);
            })
            .into_any_element(),
        _ => {
            // Idle, Checking, Failed — show status message or version
            match status_message {
                Some(message) => match message.level {
                    StatusLevel::Info => div()
                        .text_xs()
                        .text_color(cx.theme().secondary_foreground)
                        .child(message.text)
                        .into_any_element(),
                    StatusLevel::Error => div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .into_any_element(),
                },
                None => div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("v{}", env!("CARGO_PKG_VERSION")))
                    .into_any_element(),
            }
        }
    }
}
