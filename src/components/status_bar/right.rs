use gpui::*;

use crate::state::app_state::updater::UpdateStatus;
use crate::state::{AppCommands, AppState, StatusLevel, StatusMessage};
use crate::theme::colors;

pub(crate) fn render_status_right(
    status_message: Option<StatusMessage>,
    update_status: UpdateStatus,
    state: Entity<AppState>,
) -> AnyElement {
    match &update_status {
        UpdateStatus::Available { version, .. } => div()
            .id("update-available")
            .cursor_pointer()
            .text_xs()
            .text_color(colors::accent())
            .child(format!("v{version} available \u{2193}"))
            .on_click(move |_, _window, cx| {
                AppCommands::download_update(state.clone(), cx);
            })
            .into_any_element(),
        UpdateStatus::Downloading { progress_pct, .. } => div()
            .text_xs()
            .text_color(colors::text_secondary())
            .child(format!("Updating\u{2026} {progress_pct}%"))
            .into_any_element(),
        UpdateStatus::ReadyToInstall { .. } => div()
            .id("update-restart")
            .cursor_pointer()
            .text_xs()
            .text_color(colors::accent())
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
                        .text_color(colors::text_secondary())
                        .child(message.text)
                        .into_any_element(),
                    StatusLevel::Error => div()
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .into_any_element(),
                },
                None => div()
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child(format!("v{}", env!("CARGO_PKG_VERSION")))
                    .into_any_element(),
            }
        }
    }
}
