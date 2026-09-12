use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::app_state::updater::{RELEASES_URL, UpdateChannel, UpdateStage, UpdateStatus};
use crate::state::{AppCommands, AppState};
use crate::theme::spacing;

use super::Button;

pub fn status_label(status: &UpdateStatus) -> String {
    match status {
        UpdateStatus::Idle => "Software Update".into(),
        UpdateStatus::Checking => "Checking for updates…".into(),
        UpdateStatus::UpToDate { channel } => {
            format!("No {} update available", channel.label().to_lowercase())
        }
        UpdateStatus::Unavailable(_) => "Manual update available".into(),
        UpdateStatus::Available(release) => format!("{} available", release.label()),
        UpdateStatus::Downloading { .. } => status
            .progress_pct()
            .map(|pct| format!("Downloading update · {pct:.0}%"))
            .unwrap_or_else(|| "Downloading update…".into()),
        UpdateStatus::Verifying(_) => "Verifying update…".into(),
        UpdateStatus::ReadyToInstall(_) => "Update verified · Restart".into(),
        UpdateStatus::Installing(_) => "Preparing installation…".into(),
        UpdateStatus::Failed { .. } => "Update failed · Details".into(),
    }
}

pub fn channel_picker(
    id: impl Into<ElementId>,
    state: Entity<AppState>,
    cx: &App,
) -> impl IntoElement {
    let channel = state.read(cx).settings.update_channel;
    let disabled =
        state.read(cx).update_status.is_busy() || state.read(cx).unsaved_guard_is_active();
    Button::new(id)
        .small()
        .label(channel.label())
        .dropdown_caret(true)
        .disabled(disabled)
        .dropdown_menu(move |mut menu, _, _| {
            for option in [UpdateChannel::Stable, UpdateChannel::Nightly] {
                let state = state.clone();
                menu = menu.item(
                    PopupMenuItem::new(option.label()).checked(option == channel).on_click(
                        move |_, _, cx| AppCommands::set_update_channel(state.clone(), option, cx),
                    ),
                );
            }
            menu
        })
}

pub fn open_updates(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let panel = cx.new(|cx| UpdatePanel::new(state, cx));
    window.open_dialog(cx, move |dialog: Dialog, _, _| {
        dialog.title("Software Update").w(px(500.0)).child(panel.clone())
    });
}

struct UpdatePanel {
    state: Entity<AppState>,
    _subscription: Subscription,
}

impl UpdatePanel {
    fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self { state, _subscription: subscription }
    }
}

impl Render for UpdatePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        let status = state.read(cx).update_status.clone();
        let channel = state.read(cx).settings.update_channel;
        let release = status.release();
        let notes_url = release
            .as_ref()
            .map(|release| release.release_url.clone())
            .unwrap_or_else(|| RELEASES_URL.into());
        let mut body = div()
            .flex()
            .flex_col()
            .gap(spacing::md())
            .min_w(px(0.0))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(format!(
                "Installed: {} v{} · {}",
                UpdateChannel::default().label(),
                env!("CARGO_PKG_VERSION"),
                env!("OPENMANGO_GIT_SHA").get(..7).unwrap_or("development"),
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("Update channel")
                    .child(channel_picker("update-dialog-channel", state.clone(), cx)),
            )
            .when(channel == UpdateChannel::Nightly, |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Nightly includes unreleased changes and may be less stable."),
                )
            });

        let title = match &status {
            UpdateStatus::Failed { stage, .. } => match stage {
                UpdateStage::Check => "Could not check for updates".into(),
                UpdateStage::Download => "Download did not finish".into(),
                UpdateStage::Verify => "Update could not be verified".into(),
                UpdateStage::Install => "Update could not be installed".into(),
            },
            _ => status_label(&status),
        };
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .when(status.is_busy(), |row| row.child(Spinner::new().small()))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title)),
        );
        if let Some(release) = &release {
            body = body.child(
                div().text_xs().text_color(cx.theme().muted_foreground).child(release.label()),
            );
        }
        match &status {
            UpdateStatus::Downloading { received, total, .. } => {
                body = body
                    .child(
                        Progress::new("update-download-progress")
                            .accessibility_label("Update download progress")
                            .loading(*total == 0)
                            .value(status.progress_pct().unwrap_or(0.)),
                    )
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
                        "{:.1} MB of {:.1} MB",
                        *received as f64 / 1_048_576.,
                        *total as f64 / 1_048_576.
                    )));
            }
            UpdateStatus::Verifying(_) => {
                body = body.child(div().text_sm().child(
                    "The download is complete. Checking its size and SHA-256 before installation.",
                ));
            }
            UpdateStatus::ReadyToInstall(_) => {
                body = body.child(div().text_sm().child("The download is verified. You can finish your work now and restart when ready."));
            }
            UpdateStatus::Installing(_) => {
                body = body.child(div().text_sm().child("Preparing and verifying the application. Your installed copy has not been replaced."));
            }
            UpdateStatus::Failed { message, .. } => {
                body = body.child(Alert::error("update-error", message.clone()));
            }
            UpdateStatus::Unavailable(message) => {
                body = body.child(div().text_sm().child(message.clone()))
            }
            _ => {}
        }
        let mut actions = div().flex().items_center().justify_end().gap(spacing::sm());
        match &status {
            UpdateStatus::Idle | UpdateStatus::UpToDate { .. } => {
                let state = state.clone();
                actions = actions.child(
                    Button::new("update-check").primary().label("Check again").on_click(
                        move |_, _, cx| AppCommands::check_for_updates(state.clone(), cx),
                    ),
                );
            }
            UpdateStatus::Available(_) => {
                let state = state.clone();
                actions = actions.child(
                    Button::new("update-download")
                        .primary()
                        .label("Download update")
                        .on_click(move |_, _, cx| AppCommands::download_update(state.clone(), cx)),
                );
            }
            UpdateStatus::ReadyToInstall(_) => {
                let state = state.clone();
                actions = actions.child(
                    Button::new("update-install")
                        .primary()
                        .label("Restart and install")
                        .on_click(move |_, _, cx| AppCommands::install_update(state.clone(), cx)),
                );
            }
            UpdateStatus::Failed { stage, downloaded, .. } => {
                let label = if *stage == UpdateStage::Install
                    && downloaded.as_ref().is_some_and(|download| download.archive.is_file())
                {
                    "Retry installation"
                } else {
                    "Check and retry"
                };
                let state = state.clone();
                actions = actions.child(
                    Button::new("update-retry")
                        .primary()
                        .label(label)
                        .on_click(move |_, _, cx| AppCommands::retry_update(state.clone(), cx)),
                );
            }
            UpdateStatus::Checking
            | UpdateStatus::Downloading { .. }
            | UpdateStatus::Verifying(_) => {
                let state = state.clone();
                actions = actions.child(
                    Button::new("update-cancel")
                        .label("Cancel")
                        .on_click(move |_, _, cx| AppCommands::cancel_update(state.clone(), cx)),
                );
            }
            _ => {}
        }
        body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(spacing::sm())
                .child(
                    Button::new("update-notes")
                        .ghost()
                        .label("Release notes")
                        .on_click(move |_, _, cx| cx.open_url(&notes_url)),
                )
                .child(actions),
        )
        .child(
            div().flex().justify_end().child(
                Button::new("close-updates")
                    .ghost()
                    .label("Close")
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ),
        )
    }
}
