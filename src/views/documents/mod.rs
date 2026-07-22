//! Document/Collection view component.

mod actions;
#[allow(dead_code)]
pub(crate) mod ai_completion;
mod explain;
mod fast_filter;
pub(crate) use fast_filter::compile_filter_input;
mod header;
mod node_meta;
mod pagination;
mod query;
mod query_completion;
mod schema_filter;
mod schema_filter_completion;
mod state;
mod types;
mod view;
mod view_model;

pub mod dialogs;
pub mod export;
pub mod table;
pub mod tree;
pub mod views;

pub use state::CollectionView;

use gpui::{App, AppContext as _, Entity, Window};
use mongodb::bson::Bson;

use crate::state::app_state::PipelineStage;
use crate::state::{AppCommands, AppState, SessionKey, StatusMessage};

pub(crate) fn request_run_aggregation(
    state: Entity<AppState>,
    session_key: SessionKey,
    preview: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let write_confirmation = {
        let state_ref = state.read(cx);
        if state_ref.connection_read_only(session_key.connection_id) {
            None
        } else {
            state_ref.session(&session_key).and_then(|session| {
                aggregation_write_impact(
                    &session.data.aggregation.stages,
                    session.data.aggregation.selected_stage,
                    &session_key.database,
                )
                .map(|impact| {
                    (
                        impact,
                        session.data.aggregation.stages.clone(),
                        session.data.aggregation.selected_stage,
                    )
                })
            })
        }
    };

    let Some(((operator, target), confirmed_stages, confirmed_selected_stage)) = write_confirmation
    else {
        AppCommands::run_aggregation(state, session_key, preview, cx);
        return;
    };

    let semantics = if operator == "$out" {
        format!("$out will atomically replace the target collection {target} with pipeline output.")
    } else {
        format!("$merge will write pipeline output into {target} using the configured merge rules.")
    };
    let state_for_write = state.clone();
    crate::components::request_connection_write(
        state,
        crate::components::WriteRequest::new(
            session_key.connection_id,
            target,
            format!("Run an aggregation {operator} write stage"),
            Some(crate::components::WriteConfirmation {
                title: "Run aggregation write stage".into(),
                message: format!("{semantics}\n\nRun this write operation?"),
                confirm_label: "Run write stage".into(),
                destructive: true,
            }),
        ),
        window,
        cx,
        move |_window, cx| {
            AppCommands::run_aggregation_confirmed(
                state_for_write,
                session_key,
                preview,
                confirmed_stages,
                confirmed_selected_stage,
                cx,
            );
        },
    );
}

pub(crate) fn request_delete_confirmation(
    state: Entity<AppState>,
    session_key: SessionKey,
    filter: mongodb::bson::Document,
    scope_label: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    let (client, manager) = {
        let state_ref = state.read(cx);
        let Some(client) = state_ref.active_connection_client(session_key.connection_id) else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error("Connection is not active.")));
                cx.notify();
            });
            return;
        };
        (client, state_ref.connection_manager())
    };

    state.update(cx, |state, cx| {
        state.set_status_message(Some(StatusMessage::info("Counting documents...")));
        cx.notify();
    });
    let database = session_key.database.clone();
    let collection = session_key.collection.clone();
    let task = cx.background_spawn({
        let filter = filter.clone();
        let database = database.clone();
        let collection = collection.clone();
        async move { manager.count_documents(&client, &database, &collection, filter) }
    });
    let window_handle = window.window_handle();

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let result: Result<u64, crate::error::Error> = task.await;
        let _ = cx.update_window(window_handle, |_root, window, cx| match result {
            Ok(0) => {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(StatusMessage::info(
                        "No documents match the selected delete scope.",
                    )));
                    cx.notify();
                });
            }
            Ok(count) => {
                state.update(cx, |state, cx| {
                    state.set_status_message(None);
                    cx.notify();
                });
                let filter_text = crate::bson::document_to_shell_string(&filter);
                let message = format!(
                    "Delete every {scope_label} document matching this filter from {database}.{collection}? {count} document{} currently match. This cannot be undone.\n\nFilter: {filter_text}",
                    if count == 1 { "" } else { "s" }
                );
                let state_for_write = state.clone();
                crate::components::request_connection_write(
                    state.clone(),
                    crate::components::WriteRequest::new(
                        session_key.connection_id,
                        session_key.namespace(),
                        format!("Delete {count} documents"),
                        Some(crate::components::WriteConfirmation {
                        title: "Delete documents".into(),
                        message,
                        confirm_label: "Delete".into(),
                        destructive: true,
                    }),
                    ),
                    window,
                    cx,
                    move |_window, cx| {
                        AppCommands::delete_documents_by_filter(
                            state_for_write,
                            session_key,
                            filter,
                            cx,
                        );
                    },
                );
            }
            Err(error) => {
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(StatusMessage::error(format!(
                        "Failed to count documents: {error}"
                    ))));
                    cx.notify();
                });
            }
        });
    })
    .detach();
}

fn aggregation_write_impact(
    stages: &[PipelineStage],
    selected_stage: Option<usize>,
    default_database: &str,
) -> Option<(String, String)> {
    let target_index = selected_stage.or_else(|| stages.len().checked_sub(1))?;
    stages.iter().take(target_index + 1).find_map(|stage| {
        if !stage.enabled {
            return None;
        }
        let operator = stage.operator.trim();
        if !matches!(operator, "$out" | "$merge") {
            return None;
        }
        let parsed = crate::bson::parse_bson_from_relaxed_json(stage.body.trim()).ok();
        let target =
            aggregation_target(operator, parsed.as_ref(), default_database, stage.body.trim());
        Some((operator.to_string(), target))
    })
}

fn aggregation_target(
    operator: &str,
    body: Option<&Bson>,
    default_database: &str,
    fallback: &str,
) -> String {
    let target = if operator == "$merge" {
        match body {
            Some(Bson::Document(options)) => options.get("into"),
            value => value,
        }
    } else {
        body
    };

    match target {
        Some(Bson::String(collection)) => format!("{default_database}.{collection}"),
        Some(Bson::Document(namespace)) => {
            let database = namespace.get_str("db").unwrap_or(default_database);
            let collection = namespace.get_str("coll").unwrap_or("<unknown>");
            format!("{database}.{collection}")
        }
        _ if fallback.is_empty() => format!("{default_database}.<unknown>"),
        _ => fallback.to_string(),
    }
}

#[cfg(test)]
mod write_impact_tests {
    use super::*;

    fn stage(operator: &str, body: &str) -> PipelineStage {
        PipelineStage { operator: operator.to_string(), body: body.to_string(), enabled: true }
    }

    #[test]
    fn write_impact_resolves_targets_with_selected_stage_boundary() {
        let stages =
            vec![stage("$match", "{}"), stage("$out", r#"{"db":"archive","coll":"orders"}"#)];

        assert!(aggregation_write_impact(&stages, Some(0), "app").is_none());
        assert_eq!(
            aggregation_write_impact(&stages, None, "app"),
            Some(("$out".to_string(), "archive.orders".to_string()))
        );
    }

    #[test]
    fn disabled_write_stage_does_not_require_confirmation() {
        let mut output = stage("$merge", r#"{"into":"orders"}"#);
        output.enabled = false;
        assert!(aggregation_write_impact(&[output], None, "app").is_none());
    }
}
