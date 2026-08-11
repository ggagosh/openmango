//! Normalized progress UI for export, import, and copy operations.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::progress::Progress;
use gpui_component::spinner::Spinner;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use uuid::Uuid;

use crate::connection::tools_available;
use crate::state::app_state::{CollectionTransferStatus, DatabaseTransferProgress};
use crate::state::{AppState, TransferFormat, TransferMode, TransferScope, TransferTabState};
use crate::theme::{borders, colors, spacing};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressState {
    Idle,
    Running,
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

#[derive(Debug, PartialEq)]
struct ProgressSnapshot {
    state: ProgressState,
    title: String,
    detail: String,
    percentage: Option<f32>,
    errors: Vec<(String, String)>,
}

impl ProgressSnapshot {
    fn from_transfer(transfer: &TransferTabState) -> Self {
        let runtime = &transfer.runtime;
        if !runtime.has_started && !runtime.is_running && runtime.error_message.is_none() {
            return Self {
                state: ProgressState::Idle,
                title: String::new(),
                detail: String::new(),
                percentage: None,
                errors: Vec::new(),
            };
        }

        let errors = collection_errors(runtime.database_progress.as_ref());
        if runtime.cancellation_pending() {
            return Self {
                state: ProgressState::Running,
                title: "Cancelling transfer...".to_string(),
                detail: runtime.error_message.clone().unwrap_or_default(),
                percentage: None,
                errors,
            };
        }

        if let Some(error) = runtime.error_message.as_ref() {
            let lower_error = error.to_ascii_lowercase();
            let cancelled =
                lower_error.contains("cancel") && !lower_error.contains("could not be confirmed");
            if cancelled || runtime.is_running || errors.is_empty() {
                return Self {
                    state: if cancelled { ProgressState::Cancelled } else { ProgressState::Failed },
                    title: if cancelled {
                        "Transfer cancelled".to_string()
                    } else {
                        format!("{} failed", transfer.config.mode.label())
                    },
                    detail: error.clone(),
                    percentage: None,
                    errors,
                };
            }
        }

        if runtime.is_running {
            return running_snapshot(transfer, errors);
        }

        let failed = errors.len();
        let processed = runtime
            .database_progress
            .as_ref()
            .map(DatabaseTransferProgress::total_documents_processed)
            .filter(|count| *count > 0)
            .unwrap_or(runtime.progress_count);
        let past_tense = match transfer.config.mode {
            TransferMode::Export => "Exported",
            TransferMode::Import => "Imported",
            TransferMode::Copy => "Copied",
        };
        let title = if failed == 0 {
            format!("{past_tense} {processed} documents")
        } else {
            format!("{past_tense} {processed} documents with errors")
        };
        let detail = runtime
            .database_progress
            .as_ref()
            .map(|progress| {
                let completed = progress.completed_count();
                let total = progress.collections.len();
                if failed == 0 {
                    format!("{completed} collections completed")
                } else {
                    format!("{completed} of {total} collections completed · {failed} failed")
                }
            })
            .unwrap_or_else(|| transfer_subject(transfer));

        Self {
            state: if failed == 0 {
                ProgressState::Completed
            } else {
                ProgressState::CompletedWithErrors
            },
            title,
            detail,
            percentage: (failed == 0).then_some(100.0),
            errors,
        }
    }
}

fn running_snapshot(
    transfer: &TransferTabState,
    errors: Vec<(String, String)>,
) -> ProgressSnapshot {
    let verb = match transfer.config.mode {
        TransferMode::Export => "Exporting",
        TransferMode::Import => "Importing",
        TransferMode::Copy => "Copying",
    };
    let Some(progress) = transfer.runtime.database_progress.as_ref() else {
        return ProgressSnapshot {
            state: ProgressState::Running,
            title: format!("{verb} {}", transfer_subject(transfer)),
            detail: format!("{} documents", transfer.runtime.progress_count),
            percentage: None,
            errors,
        };
    };

    let active = progress
        .collections
        .iter()
        .find(|collection| matches!(collection.status, CollectionTransferStatus::InProgress));

    if matches!(transfer.config.format, TransferFormat::Bson)
        && !matches!(transfer.config.mode, TransferMode::Copy)
    {
        let title = active
            .map(|collection| format!("{verb} {}", collection.name))
            .unwrap_or_else(|| format!("{verb} {}", transfer_subject(transfer)));
        let unit = if matches!(transfer.config.mode, TransferMode::Import) {
            "bytes"
        } else {
            "documents"
        };
        let detail = active
            .map(|collection| match collection.documents_total {
                Some(total) => {
                    format!("{} of {total} {unit}", collection.documents_processed)
                }
                None => format!("{} {unit}", collection.documents_processed),
            })
            .unwrap_or_else(|| format!("{} collections completed", progress.completed_count()));
        return ProgressSnapshot {
            state: ProgressState::Running,
            title,
            detail,
            percentage: active.and_then(|collection| collection.percentage()),
            errors,
        };
    }

    let total = progress.collections.len();
    let finished = finished_collection_count(progress);
    let detail = active
        .map(|collection| {
            format!(
                "{finished} of {total} collections · {} · {} documents",
                collection.name, collection.documents_processed
            )
        })
        .unwrap_or_else(|| format!("{finished} of {total} collections"));
    let percentage = if total == 0 { None } else { Some((finished as f32 / total as f32) * 100.0) };

    ProgressSnapshot {
        state: ProgressState::Running,
        title: format!("{verb} {}", transfer_subject(transfer)),
        detail,
        percentage,
        errors,
    }
}

fn transfer_subject(transfer: &TransferTabState) -> String {
    let database = if transfer.config.source_database.is_empty() {
        "data"
    } else {
        transfer.config.source_database.as_str()
    };
    if matches!(transfer.config.scope, TransferScope::Collection)
        && !transfer.config.source_collection.is_empty()
    {
        format!("{database}.{}", transfer.config.source_collection)
    } else {
        database.to_string()
    }
}

fn finished_collection_count(progress: &DatabaseTransferProgress) -> usize {
    progress
        .collections
        .iter()
        .filter(|collection| {
            matches!(
                collection.status,
                CollectionTransferStatus::Completed
                    | CollectionTransferStatus::Failed(_)
                    | CollectionTransferStatus::Cancelled
            )
        })
        .count()
}

fn collection_errors(progress: Option<&DatabaseTransferProgress>) -> Vec<(String, String)> {
    progress
        .into_iter()
        .flat_map(|progress| progress.collections.iter())
        .filter_map(|collection| match &collection.status {
            CollectionTransferStatus::Failed(error) => {
                Some((collection.name.clone(), error.clone()))
            }
            _ => None,
        })
        .collect()
}

pub(super) fn render_progress_status(
    transfer: &TransferTabState,
    state: Entity<AppState>,
    transfer_id: Uuid,
    cx: &App,
) -> AnyElement {
    let snapshot = ProgressSnapshot::from_transfer(transfer);
    if snapshot.state == ProgressState::Idle {
        return div().into_any_element();
    }

    let status_icon: AnyElement = match snapshot.state {
        ProgressState::Running => Spinner::new().small().into_any_element(),
        ProgressState::Completed => {
            Icon::new(IconName::Check).xsmall().text_color(cx.theme().success).into_any_element()
        }
        ProgressState::CompletedWithErrors => {
            Icon::new(IconName::Info).xsmall().text_color(cx.theme().warning).into_any_element()
        }
        ProgressState::Failed => {
            Icon::new(IconName::Close).xsmall().text_color(cx.theme().danger).into_any_element()
        }
        ProgressState::Cancelled => {
            Icon::new(IconName::Close).xsmall().text_color(cx.theme().warning).into_any_element()
        }
        ProgressState::Idle => div().into_any_element(),
    };

    let progress_bar =
        snapshot.percentage.map(|percentage| Progress::new().value(percentage).into_any_element());
    let errors_expanded =
        transfer.runtime.database_progress.as_ref().is_some_and(|progress| progress.panel_expanded);
    let error_count = snapshot.errors.len();
    let errors = if error_count == 0 {
        div().into_any_element()
    } else {
        let toggle_state = state.clone();
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(
                div()
                    .id("transfer-errors")
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        toggle_state.update(cx, |state, cx| {
                            if let Some(tab) = state.transfer_tab_mut(transfer_id)
                                && let Some(progress) = tab.runtime.database_progress.as_mut()
                            {
                                progress.panel_expanded = !progress.panel_expanded;
                            }
                            cx.notify();
                        });
                    })
                    .child(
                        Icon::new(if errors_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(format!("Show {error_count} errors")),
                    ),
            )
            .when(errors_expanded, |this| {
                this.children(snapshot.errors.into_iter().map(|(collection, error)| {
                    div()
                        .flex()
                        .gap(spacing::sm())
                        .pl(spacing::md())
                        .text_xs()
                        .child(
                            div()
                                .w(px(160.0))
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_color(cx.theme().secondary_foreground)
                                .child(collection),
                        )
                        .child(div().text_color(cx.theme().danger).child(error))
                }))
            })
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(status_icon)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().foreground)
                        .child(snapshot.title),
                )
                .child(
                    div().text_xs().text_color(cx.theme().muted_foreground).child(snapshot.detail),
                ),
        )
        .children(progress_bar)
        .child(errors)
        .into_any_element()
}

/// Render format, safety, and validation warnings.
pub(super) fn render_warnings(transfer_state: &TransferTabState, cx: &App) -> AnyElement {
    let validation = crate::state::validate_transfer(transfer_state);
    let mut messages: Vec<(bool, String)> =
        validation.warnings.into_iter().map(|message| (false, message)).collect();

    if matches!(transfer_state.config.mode, TransferMode::Export | TransferMode::Import)
        && matches!(transfer_state.config.format, TransferFormat::Bson)
        && !tools_available()
    {
        messages.push((
            true,
            "BSON format requires mongodump/mongorestore. Run: just download-tools.".to_string(),
        ));
    }

    if messages.is_empty() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .children(messages.into_iter().map(|(is_error, message)| {
            let (bg, border, fg, icon) = if is_error {
                (colors::bg_error(cx), colors::border_error(cx), cx.theme().danger, IconName::Close)
            } else {
                (
                    colors::bg_warning(cx),
                    colors::border_warning(cx),
                    cx.theme().warning,
                    IconName::Info,
                )
            };
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .px(spacing::md())
                .py(spacing::sm())
                .bg(bg)
                .border_1()
                .border_color(border)
                .rounded(borders::radius_sm())
                .child(Icon::new(icon).xsmall().text_color(fg))
                .child(div().text_sm().text_color(fg).child(message))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{ProgressSnapshot, ProgressState};
    use crate::state::app_state::{
        CollectionProgress, CollectionTransferStatus, DatabaseTransferProgress,
    };
    use crate::state::{TransferFormat, TransferMode, TransferScope, TransferTabState};

    #[test]
    fn collection_progress_is_indeterminate_without_a_total() {
        let mut transfer = TransferTabState::default();
        transfer.runtime.has_started = true;
        transfer.runtime.is_running = true;
        transfer.runtime.progress_count = 42;

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.state, ProgressState::Running);
        assert_eq!(snapshot.percentage, None);
        assert_eq!(snapshot.detail, "42 documents");
    }

    #[test]
    fn database_progress_uses_completed_collections() {
        let mut transfer = TransferTabState::default();
        transfer.config.scope = TransferScope::Database;
        transfer.runtime.has_started = true;
        transfer.runtime.is_running = true;
        transfer.runtime.database_progress = Some(DatabaseTransferProgress {
            collections: vec![
                CollectionProgress {
                    name: "done".into(),
                    status: CollectionTransferStatus::Completed,
                    documents_processed: 10,
                    documents_total: None,
                },
                CollectionProgress {
                    name: "active".into(),
                    status: CollectionTransferStatus::InProgress,
                    documents_processed: 5,
                    documents_total: None,
                },
            ],
            panel_expanded: false,
        });

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.percentage, Some(50.0));
        assert!(snapshot.detail.starts_with("1 of 2 collections"));
    }

    #[test]
    fn bson_progress_is_scoped_to_the_active_collection() {
        let mut transfer = TransferTabState::default();
        transfer.config.scope = TransferScope::Database;
        transfer.config.format = TransferFormat::Bson;
        transfer.runtime.has_started = true;
        transfer.runtime.is_running = true;
        transfer.runtime.database_progress = Some(DatabaseTransferProgress {
            collections: vec![CollectionProgress {
                name: "users".into(),
                status: CollectionTransferStatus::InProgress,
                documents_processed: 25,
                documents_total: Some(100),
            }],
            panel_expanded: false,
        });

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.percentage, Some(25.0));
        assert!(snapshot.title.ends_with("users"));
    }

    #[test]
    fn collection_import_uses_the_same_indeterminate_document_progress() {
        let mut transfer = TransferTabState::default();
        transfer.config.mode = TransferMode::Import;
        transfer.runtime.has_started = true;
        transfer.runtime.is_running = true;
        transfer.runtime.progress_count = 18;

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.percentage, None);
        assert!(snapshot.title.starts_with("Importing"));
        assert_eq!(snapshot.detail, "18 documents");
    }

    #[test]
    fn database_copy_uses_the_collection_denominator() {
        let mut transfer = TransferTabState::default();
        transfer.config.mode = TransferMode::Copy;
        transfer.config.scope = TransferScope::Database;
        transfer.config.format = TransferFormat::Bson;
        transfer.runtime.has_started = true;
        transfer.runtime.is_running = true;
        transfer.runtime.database_progress = Some(DatabaseTransferProgress {
            collections: vec![
                CollectionProgress {
                    name: "done".into(),
                    status: CollectionTransferStatus::Completed,
                    documents_processed: 10,
                    documents_total: None,
                },
                CollectionProgress {
                    name: "pending".into(),
                    status: CollectionTransferStatus::Pending,
                    documents_processed: 0,
                    documents_total: None,
                },
            ],
            panel_expanded: false,
        });

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.percentage, Some(50.0));
        assert!(snapshot.title.starts_with("Copying"));
    }

    #[test]
    fn pre_start_failure_is_visible() {
        let mut transfer = TransferTabState::default();
        transfer.runtime.error_message = Some("mongodump unavailable".into());

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.state, ProgressState::Failed);
        assert_eq!(snapshot.detail, "mongodump unavailable");
    }

    #[test]
    fn bson_import_progress_uses_bytes() {
        let mut transfer = TransferTabState::default();
        transfer.config.mode = TransferMode::Import;
        transfer.config.scope = TransferScope::Database;
        transfer.config.format = TransferFormat::Bson;
        transfer.runtime.has_started = true;
        transfer.runtime.is_running = true;
        transfer.runtime.database_progress = Some(DatabaseTransferProgress {
            collections: vec![CollectionProgress {
                name: "users".into(),
                status: CollectionTransferStatus::InProgress,
                documents_processed: 256,
                documents_total: Some(1024),
            }],
            panel_expanded: false,
        });

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.detail, "256 of 1024 bytes");
    }

    #[test]
    fn bson_import_completion_uses_collection_document_counts() {
        let mut transfer = TransferTabState::default();
        transfer.config.mode = TransferMode::Import;
        transfer.config.scope = TransferScope::Database;
        transfer.config.format = TransferFormat::Bson;
        transfer.runtime.has_started = true;
        transfer.runtime.database_progress = Some(DatabaseTransferProgress {
            collections: vec![CollectionProgress {
                name: "users".into(),
                status: CollectionTransferStatus::Completed,
                documents_processed: 12,
                documents_total: Some(12),
            }],
            panel_expanded: false,
        });

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.title, "Imported 12 documents");
    }

    #[test]
    fn partial_database_failure_is_completed_with_errors() {
        let mut transfer = TransferTabState::default();
        transfer.config.scope = TransferScope::Database;
        transfer.runtime.has_started = true;
        transfer.runtime.progress_count = 10;
        transfer.runtime.error_message = Some("1 collection failed".into());
        transfer.runtime.database_progress = Some(DatabaseTransferProgress {
            collections: vec![
                CollectionProgress {
                    name: "done".into(),
                    status: CollectionTransferStatus::Completed,
                    documents_processed: 10,
                    documents_total: None,
                },
                CollectionProgress {
                    name: "failed".into(),
                    status: CollectionTransferStatus::Failed("duplicate key".into()),
                    documents_processed: 0,
                    documents_total: None,
                },
            ],
            panel_expanded: false,
        });

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.state, ProgressState::CompletedWithErrors);
        assert_eq!(snapshot.errors.len(), 1);
    }

    #[test]
    fn bson_cancellation_pending_remains_running() {
        let mut transfer = TransferTabState::default();
        transfer.runtime.has_started = true;
        transfer.runtime.is_running = true;
        transfer.runtime.cancellation_requested = true;
        transfer.runtime.error_message =
            Some("Cancellation requested; waiting for the MongoDB tool to terminate".into());

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.state, ProgressState::Running);
        assert_eq!(snapshot.title, "Cancelling transfer...");
    }

    #[test]
    fn unconfirmed_bson_termination_is_a_failure() {
        let mut transfer = TransferTabState::default();
        transfer.runtime.has_started = true;
        transfer.runtime.error_message = Some(
            "BSON export cancellation requested, but mongodump termination could not be confirmed"
                .into(),
        );

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.state, ProgressState::Failed);
    }

    #[test]
    fn zero_document_transfer_still_has_a_completed_state() {
        let mut transfer = TransferTabState::default();
        transfer.runtime.has_started = true;

        let snapshot = ProgressSnapshot::from_transfer(&transfer);
        assert_eq!(snapshot.state, ProgressState::Completed);
        assert_eq!(snapshot.percentage, Some(100.0));
    }
}
