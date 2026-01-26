//! Aggregation pipeline execution commands.

use gpui::{App, AppContext as _, Entity};

use crate::bson::parse_document_from_json;
use crate::connection::get_connection_manager;
use crate::state::app_state::PipelineStage;
use crate::state::{AppCommands, AppEvent, AppState, SessionKey};
use mongodb::bson::Document;

impl AppCommands {
    pub fn run_aggregation(
        state: Entity<AppState>,
        session_key: SessionKey,
        preview: bool,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let (database, collection, stages, selected_stage, result_limit) = {
            let state_ref = state.read(cx);
            let (stages, selected_stage, result_limit) = state_ref
                .session(&session_key)
                .map(|session| {
                    (
                        session.data.aggregation.stages.clone(),
                        session.data.aggregation.selected_stage,
                        session.data.aggregation.result_limit,
                    )
                })
                .unwrap_or((Vec::new(), None, 50));
            (
                session_key.database.clone(),
                session_key.collection.clone(),
                stages,
                selected_stage,
                result_limit,
            )
        };

        let pipeline = match build_pipeline(&stages, selected_stage) {
            Ok(pipeline) if !pipeline.is_empty() => pipeline,
            Ok(_) => {
                state.update(cx, |state, cx| {
                    if let Some(session) = state.session_mut(&session_key) {
                        session.data.aggregation.error = Some("Pipeline is empty".to_string());
                        session.data.aggregation.results = None;
                    }
                    cx.notify();
                });
                let event = AppEvent::AggregationFailed {
                    session: session_key,
                    error: "Pipeline is empty".to_string(),
                };
                state.update(cx, |state, cx| {
                    state.update_status_from_event(&event);
                    cx.emit(event);
                    cx.notify();
                });
                return;
            }
            Err(err) => {
                state.update(cx, |state, cx| {
                    if let Some(session) = state.session_mut(&session_key) {
                        session.data.aggregation.error = Some(format!("Invalid stage JSON: {err}"));
                        session.data.aggregation.results = None;
                    }
                    cx.notify();
                });
                let event = AppEvent::AggregationFailed {
                    session: session_key,
                    error: format!("Invalid stage JSON: {err}"),
                };
                state.update(cx, |state, cx| {
                    state.update_status_from_event(&event);
                    cx.emit(event);
                    cx.notify();
                });
                return;
            }
        };

        state.update(cx, |state, cx| {
            if let Some(session) = state.session_mut(&session_key) {
                session.data.aggregation.loading = true;
                session.data.aggregation.error = None;
            }
            cx.notify();
        });

        let limited = result_limit > 0;
        let limit = if limited { Some(result_limit) } else { None };

        let task = cx.background_spawn({
            let database_for_task = database.clone();
            let collection_for_task = collection.clone();
            let pipeline_for_task = pipeline.clone();
            async move {
                let manager = get_connection_manager();
                manager.aggregate_pipeline(
                    &client,
                    &database_for_task,
                    &collection_for_task,
                    pipeline_for_task,
                    limit,
                )
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Vec<mongodb::bson::Document>, crate::error::Error> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok(documents) => {
                        let count = documents.len();
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.aggregation.results = Some(documents);
                                session.data.aggregation.loading = false;
                                session.data.aggregation.error = None;
                                let target_index =
                                    session.data.aggregation.selected_stage.or_else(|| {
                                        session.data.aggregation.stages.len().checked_sub(1)
                                    });
                                if let Some(index) = target_index
                                    && index < session.data.aggregation.stage_doc_counts.len()
                                {
                                    session.data.aggregation.stage_doc_counts[index] =
                                        Some(count as u64);
                                }
                            }
                            let event = AppEvent::AggregationCompleted {
                                session: session_key.clone(),
                                count,
                                preview,
                                limited,
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.aggregation.loading = false;
                                session.data.aggregation.error =
                                    Some(format!("Aggregation failed: {error}"));
                            }
                            let event = AppEvent::AggregationFailed {
                                session: session_key.clone(),
                                error: format!("{error}"),
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }
}

fn build_pipeline(
    stages: &[PipelineStage],
    selected_stage: Option<usize>,
) -> Result<Vec<Document>, String> {
    let mut pipeline = Vec::new();
    for (idx, stage) in stages.iter().enumerate() {
        if let Some(selected) = selected_stage
            && idx > selected
        {
            break;
        }
        if !stage.enabled {
            continue;
        }
        let operator = stage.operator.trim();
        if operator.is_empty() {
            return Err(format!("Stage {} has no operator", idx + 1));
        }
        let body = stage.body.trim();
        let body_doc = if body.is_empty() || body == "{}" {
            Document::new()
        } else {
            parse_document_from_json(body)
                .map_err(|err| format!("Stage {} ({operator}): {err}", idx + 1))?
        };
        let mut stage_doc = Document::new();
        stage_doc.insert(operator, body_doc);
        pipeline.push(stage_doc);
    }
    Ok(pipeline)
}
