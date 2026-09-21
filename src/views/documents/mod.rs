//! Document/Collection view component.

mod actions;
mod ask_ai;
mod explain;
#[cfg(test)]
mod explain_escape_tests;
mod fast_filter;
pub(crate) use fast_filter::compile_filter_input;
mod header;
mod json_view;
mod node_meta;
mod pagination;
mod query;
mod query_completion;
mod query_editor;
mod query_format;
mod query_values;
pub mod reference;
mod schema_filter;
mod schema_filter_completion;
mod state;
mod view;
mod view_model;
mod workflow;

pub mod dialogs;
pub mod export;
pub mod table;
pub mod tree;
pub mod views;

pub use state::CollectionView;

use gpui_kit::{App, AppContext as _, Entity, Window};
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
                let aggregation = &session.data.aggregation;
                let target = aggregation.preview_target();
                aggregation_write_impact(&aggregation.stages, target, &session_key.database)
                    .map(|impact| (impact, aggregation.stages.clone(), target))
            })
        }
    };

    let Some(((operator, target), confirmed_stages, confirmed_target)) = write_confirmation else {
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
                confirmed_target,
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
        state.set_status_message(Some(StatusMessage::info("Counting documents…")));
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

    cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
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
                let recovery = " This cannot be undone.";
                let message = format!(
                    "Delete every {scope_label} document matching this filter from {database}.{collection}? {count} document{} currently match.{recovery}\n\nFilter: {filter_text}",
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

/// Saves the aggregation screen's pipeline as a view. A pipeline that was opened from a view's
/// definition updates that view, after saying what that means for its readers; any other
/// pipeline is given a name first.
pub(crate) fn request_save_view(
    state: Entity<AppState>,
    session_key: SessionKey,
    window: &mut Window,
    cx: &mut App,
) {
    let Some((stages, editing_view)) = state.read(cx).session(&session_key).map(|session| {
        let aggregation = &session.data.aggregation;
        let editing = aggregation.editing_view.as_ref().map(|editing| editing.name.clone());
        (aggregation.stages.clone(), editing)
    }) else {
        return;
    };
    let pipeline = match crate::state::view_pipeline(&stages) {
        Ok(pipeline) => pipeline,
        Err(message) => {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(message)));
                cx.notify();
            });
            return;
        }
    };
    let SessionKey { connection_id, database, collection: view_on, .. } = session_key.clone();
    let Some(view) = editing_view else {
        crate::app::dialogs::open_new_view_dialog(
            state,
            connection_id,
            database,
            crate::app::dialogs::NewView::Pipeline { view_on, pipeline },
            window,
            cx,
        );
        return;
    };

    let namespace = format!("{database}.{view}");
    let confirmation = crate::components::WriteConfirmation {
        title: format!("Update view \"{namespace}\"?"),
        message: "Everything that reads this view gets the new pipeline from now on. Its \
                  collation stays as it is."
            .to_string(),
        confirm_label: "Update view".into(),
        destructive: false,
    };
    let state_for_write = state.clone();
    crate::components::request_connection_write(
        state,
        crate::components::WriteRequest::new(
            connection_id,
            namespace,
            "Update a view",
            Some(confirmation),
        ),
        window,
        cx,
        move |_window, cx| {
            // What the button shows follows this: busy while the write runs, then "up to date"
            // against the pipeline that was sent, or back to "changed" if the server refused.
            let mark = |state: &Entity<AppState>,
                        key: &SessionKey,
                        saved: Option<Vec<mongodb::bson::Document>>,
                        updating: bool,
                        cx: &mut App| {
                state.update(cx, |state, cx| {
                    let editing = state
                        .session_mut(key)
                        .and_then(|session| session.data.aggregation.editing_view.as_mut());
                    if let Some(editing) = editing {
                        editing.updating = updating;
                        if let Some(saved) = saved {
                            editing.saved = saved;
                        }
                    }
                    cx.notify();
                });
            };
            mark(&state_for_write, &session_key, None, true, cx);
            let state_when_done = state_for_write.clone();
            let sent = pipeline.clone();
            AppCommands::save_view(
                state_for_write,
                crate::state::ViewSave {
                    connection_id,
                    database,
                    name: view,
                    source: crate::state::ViewSource::Pipeline {
                        view_on,
                        pipeline,
                        collation: None,
                    },
                    replace: true,
                },
                cx,
                // A failure is already recorded by the command; there is no dialog to keep open.
                move |result, cx| {
                    mark(&state_when_done, &session_key, result.is_ok().then_some(sent), false, cx);
                },
            );
        },
    );
}

/// Where the aggregation screen's pipeline stands against the view it is an edit of, or `None`
/// when it isn't one. Compared as parsed pipelines, so reformatting a stage or adding a
/// disabled one is not a change; a stage that doesn't parse is, since it can't be the saved one.
pub(crate) fn view_edit_status(
    state: &AppState,
    session_key: &SessionKey,
) -> Option<(String, crate::state::app_state::ViewEditStatus)> {
    let aggregation = &state.session(session_key)?.data.aggregation;
    let editing = aggregation.editing_view.as_ref()?;
    Some((editing.name.clone(), edit_status(&aggregation.stages, editing)))
}

fn edit_status(
    stages: &[PipelineStage],
    editing: &crate::state::app_state::EditingView,
) -> crate::state::app_state::ViewEditStatus {
    use crate::state::app_state::ViewEditStatus;
    if editing.updating {
        ViewEditStatus::Updating
    } else if crate::state::view_pipeline(stages).is_ok_and(|now| now == editing.saved) {
        ViewEditStatus::UpToDate
    } else {
        ViewEditStatus::Changed
    }
}

#[cfg(test)]
mod view_edit_tests {
    use mongodb::bson::doc;

    use super::edit_status;
    use crate::state::app_state::{
        EditingView, PipelineStage, ViewEditStatus, stages_from_pipeline,
    };

    /// Opened the way `edit_view_definition` opens it: the saved form is what the builder
    /// makes of the server's pipeline, which here holds `$limit` as a double.
    fn opened() -> (Vec<PipelineStage>, EditingView) {
        let server = [doc! { "$match": { "status": "open" } }, doc! { "$limit": 5.0 }];
        let stages = stages_from_pipeline(&server);
        let saved = crate::state::view_pipeline(&stages).unwrap();
        (stages, EditingView { name: "open_orders".into(), saved, updating: false })
    }

    #[test]
    fn only_a_real_change_asks_for_an_update() {
        let (mut stages, mut editing) = opened();
        assert_eq!(edit_status(&stages, &editing), ViewEditStatus::UpToDate);

        // Reformatting and a switched-off stage leave the definition as it is.
        stages[0].body = "{\n  status:   \"open\"\n}".into();
        stages.push(PipelineStage::with("$sort".to_string(), "{ n: 1 }".to_string(), false));
        assert_eq!(edit_status(&stages, &editing), ViewEditStatus::UpToDate);

        stages[0].body = "{ status: \"done\" }".into();
        assert_eq!(edit_status(&stages, &editing), ViewEditStatus::Changed);
        stages[0].body = "{ status: ".into();
        assert_eq!(edit_status(&stages, &editing), ViewEditStatus::Changed);

        editing.updating = true;
        assert_eq!(edit_status(&stages, &editing), ViewEditStatus::Updating);
    }
}

/// The first `$out`/`$merge` stage a run through `target` would execute, as (operator, namespace).
pub(crate) fn aggregation_write_impact(
    stages: &[PipelineStage],
    target: Option<usize>,
    default_database: &str,
) -> Option<(String, String)> {
    let target_index = target?;
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
        PipelineStage::with(operator.to_string(), body.to_string(), true)
    }

    #[test]
    fn write_impact_resolves_targets_with_selected_stage_boundary() {
        let stages =
            vec![stage("$match", "{}"), stage("$out", r#"{"db":"archive","coll":"orders"}"#)];

        assert!(aggregation_write_impact(&stages, Some(0), "app").is_none());
        assert!(aggregation_write_impact(&stages, None, "app").is_none());
        assert_eq!(
            aggregation_write_impact(&stages, Some(1), "app"),
            Some(("$out".to_string(), "archive.orders".to_string()))
        );
    }

    #[test]
    fn disabled_write_stage_does_not_require_confirmation() {
        let mut output = stage("$merge", r#"{"into":"orders"}"#);
        output.enabled = false;
        assert!(aggregation_write_impact(&[output], Some(0), "app").is_none());
    }
}
