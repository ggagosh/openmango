mod download;
mod install;
mod release;

use std::sync::Arc;

use anyhow::Context as _;
use futures::StreamExt as _;
use gpui_kit::{App, AppContext as _, Entity};

use crate::components::request_unsaved_action;
use crate::state::app_state::updater::{UpdateChannel, UpdateStage, UpdateStatus};
use crate::state::{AppEvent, AppState, UnsavedScope};

use super::AppCommands;

fn begin(state: &Entity<AppState>, status: UpdateStatus, cx: &mut App) -> u64 {
    state.update(cx, |state, cx| {
        if let Some(task) = state.update_task.take() {
            task.abort();
        }
        state.update_request_id = state.update_request_id.wrapping_add(1);
        state.update_status = status;
        cx.notify();
        state.update_request_id
    })
}

impl AppCommands {
    pub fn automatic_updates_supported() -> bool {
        install::running_bundle().is_ok()
    }

    pub fn check_for_updates(state: Entity<AppState>, cx: &mut App) {
        Self::check_updates(state, false, cx);
    }

    fn check_updates(state: Entity<AppState>, download_after_check: bool, cx: &mut App) {
        if !state.read(cx).update_status.can_check() {
            return;
        }
        if let Err(error) = install::running_bundle() {
            state.update(cx, |state, cx| {
                state.update_status = UpdateStatus::Unavailable(error.to_string());
                cx.notify();
            });
            return;
        }
        let channel = state.read(cx).settings.update_channel;
        let request = begin(&state, UpdateStatus::Checking, cx);
        let runtime = state.read(cx).connection_manager().runtime_handle();
        let sha = std::env::var("OPENMANGO_TEST_SHA")
            .unwrap_or_else(|_| env!("OPENMANGO_GIT_SHA").to_string());
        let task = runtime.spawn(async move {
            release::check(channel, UpdateChannel::default(), env!("CARGO_PKG_VERSION"), &sha).await
        });
        state.update(cx, |state, _| state.update_task = Some(task.abort_handle()));
        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let result = task
                .await
                .context("The update check stopped unexpectedly")
                .and_then(|result| result);
            cx.update(|cx| {
                if state.read(cx).update_request_id != request {
                    return;
                }
                let mut should_download = false;
                state.update(cx, |state, cx| {
                    state.update_task = None;
                    match result {
                        Ok(Some(release)) => {
                            let label = release.label();
                            state.update_status = UpdateStatus::Available(release);
                            should_download = state.settings.auto_update || download_after_check;
                            let event = AppEvent::UpdateAvailable { version: label };
                            cx.emit(event);
                        }
                        Ok(None) => state.update_status = UpdateStatus::UpToDate { channel },
                        Err(error) => {
                            log::warn!("Update check failed: {error:#}");
                            state.update_status = UpdateStatus::Failed {
                                stage: UpdateStage::Check,
                                message: format!("{error:#}"),
                                release: None,
                                downloaded: None,
                            };
                        }
                    }
                    cx.notify();
                });
                if should_download {
                    Self::download_update(state.clone(), cx);
                }
            });
        })
        .detach();
    }

    pub fn set_update_channel(state: Entity<AppState>, channel: UpdateChannel, cx: &mut App) {
        if state.read(cx).update_status.is_busy() || state.read(cx).unsaved_guard_is_active() {
            return;
        }
        if state.read(cx).settings.update_channel == channel {
            return;
        }
        state.update(cx, |state, cx| {
            state.settings.update_channel = channel;
            state.save_settings();
            state.update_request_id = state.update_request_id.wrapping_add(1);
            state.update_status = UpdateStatus::Idle;
            cx.notify();
        });
        Self::check_for_updates(state, cx);
    }

    pub fn cancel_update(state: Entity<AppState>, cx: &mut App) {
        if matches!(state.read(cx).update_status, UpdateStatus::Installing(_)) {
            return;
        }
        let release = state.read(cx).update_status.release();
        begin(&state, release.map(UpdateStatus::Available).unwrap_or(UpdateStatus::Idle), cx);
    }

    pub fn retry_update(state: Entity<AppState>, cx: &mut App) {
        let status = state.read(cx).update_status.clone();
        match status {
            UpdateStatus::Failed { downloaded: Some(download), .. }
                if download.archive.is_file() =>
            {
                state.update(cx, |state, cx| {
                    state.update_status = UpdateStatus::ReadyToInstall(download);
                    cx.notify();
                });
                Self::install_update(state, cx);
            }
            // Refresh release metadata: a nightly may have been replaced since the failure.
            UpdateStatus::Failed { stage, .. } => {
                state.update(cx, |state, _| state.update_status = UpdateStatus::Idle);
                Self::check_updates(state, stage != UpdateStage::Check, cx);
            }
            _ => {}
        }
    }

    pub fn download_update(state: Entity<AppState>, cx: &mut App) {
        let UpdateStatus::Available(release) = state.read(cx).update_status.clone() else {
            return;
        };
        let request = begin(
            &state,
            UpdateStatus::Downloading {
                release: release.clone(),
                received: 0,
                total: release.size,
            },
            cx,
        );
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let runtime = state.read(cx).connection_manager().runtime_handle();
        let release_for_task = release.clone();
        let task = runtime.spawn(async move {
            let result = download::download(release_for_task, sender.clone()).await;
            let _ = sender.unbounded_send(download::Progress::Finished(result));
        });
        state.update(cx, |state, _| state.update_task = Some(task.abort_handle()));
        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let mut finished = false;
            // Progress and completion share one ordered stream; late progress cannot undo Ready.
            while let Some(progress) = receiver.next().await {
                if matches!(&progress, download::Progress::Finished(_)) {
                    finished = true;
                }
                cx.update(|cx| {
                    if state.read(cx).update_request_id != request {
                        return;
                    }
                    state.update(cx, |state, cx| {
                        match progress {
                            download::Progress::Downloading { received, total } => {
                                state.update_status = UpdateStatus::Downloading {
                                    release: release.clone(),
                                    received,
                                    total,
                                };
                            }
                            download::Progress::Verifying => {
                                state.update_status = UpdateStatus::Verifying(release.clone())
                            }
                            download::Progress::Finished(result) => {
                                state.update_task = None;
                                let stage =
                                    if matches!(state.update_status, UpdateStatus::Verifying(_)) {
                                        UpdateStage::Verify
                                    } else {
                                        UpdateStage::Download
                                    };
                                state.update_status = match result {
                                    Ok(download) => {
                                        UpdateStatus::ReadyToInstall(Arc::new(download))
                                    }
                                    Err(error) => {
                                        log::error!("Update {stage:?} failed: {error:#}");
                                        UpdateStatus::Failed {
                                            stage,
                                            message: format!("{error:#}"),
                                            release: Some(release.clone()),
                                            downloaded: None,
                                        }
                                    }
                                };
                            }
                        }
                        cx.notify();
                    });
                });
            }
            let result = task.await;
            if !finished {
                cx.update(|cx| {
                    if state.read(cx).update_request_id != request {
                        return;
                    }
                    state.update(cx, |state, cx| {
                        state.update_task = None;
                        state.update_status = UpdateStatus::Failed {
                            stage: UpdateStage::Download,
                            message: result
                                .err()
                                .map(|error| format!("The update download stopped: {error}"))
                                .unwrap_or_else(|| {
                                    "The update download ended without a verified file.".into()
                                }),
                            release: Some(release),
                            downloaded: None,
                        };
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    pub fn install_update(state: Entity<AppState>, cx: &mut App) {
        if state.read(cx).unsaved_guard_is_active() {
            return;
        }
        let UpdateStatus::ReadyToInstall(download) = state.read(cx).update_status.clone() else {
            return;
        };
        let request = begin(&state, UpdateStatus::Installing(download.release.clone()), cx);
        let download_for_task = download.clone();
        let task = cx.background_spawn(async move { install::prepare(&download_for_task.archive) });
        cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
            let result = task.await;
            cx.update(|cx| {
                if state.read(cx).update_request_id != request {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        // Preparation has not touched the installed bundle. Cancel leaves it intact.
                        state.update(cx, |state, cx| {
                            state.update_status = UpdateStatus::ReadyToInstall(download.clone());
                            cx.notify();
                        });
                        let Some(handle) = cx.windows().into_iter().next() else {
                            return;
                        };
                        let state_for_install = state.clone();
                        let _ = handle.update(cx, move |_, window, cx| {
                            request_unsaved_action(
                                state.clone(),
                                UnsavedScope::App,
                                window,
                                cx,
                                move |_, cx| {
                                    if state_for_install.read(cx).update_request_id != request {
                                        return;
                                    }
                                    state_for_install.update(cx, |state, _| {
                                        state.update_workspace_from_state();
                                        state.flush_workspace_now();
                                    });
                                    match install::activate_and_restart(prepared) {
                                        Ok(()) => cx.quit(),
                                        Err(error) => {
                                            log::error!("Update installation failed: {error:#}");
                                            state_for_install.update(cx, |state, cx| {
                                                state.update_status = UpdateStatus::Failed {
                                                    stage: UpdateStage::Install,
                                                    message: format!("{error:#}"),
                                                    release: Some(download.release.clone()),
                                                    downloaded: Some(download),
                                                };
                                                cx.notify();
                                            });
                                        }
                                    }
                                },
                            );
                        });
                    }
                    Err(error) => {
                        log::error!("Update preparation failed: {error:#}");
                        state.update(cx, |state, cx| {
                            state.update_status = UpdateStatus::Failed {
                                stage: UpdateStage::Install,
                                message: format!("{error:#}"),
                                release: Some(download.release.clone()),
                                downloaded: Some(download),
                            };
                            cx.notify();
                        });
                    }
                }
            });
        })
        .detach();
    }
}
