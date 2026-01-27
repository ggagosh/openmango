//! Aggregation pipeline execution commands.

use std::time::Instant;

use gpui::{App, AppContext as _, Entity};

use crate::bson::parse_document_from_json;
use crate::connection::get_connection_manager;
use crate::connection::mongo::ConnectionManager;
use crate::state::app_state::{PipelineStage, StageDocCounts};
use crate::state::{AppCommands, AppEvent, AppState, SessionKey};
use mongodb::bson::{Bson, Document, doc};

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

        let (database, collection, stages, selected_stage, result_limit, results_page) = {
            let state_ref = state.read(cx);
            let (stages, selected_stage, result_limit, results_page) = state_ref
                .session(&session_key)
                .map(|session| {
                    (
                        session.data.aggregation.stages.clone(),
                        session.data.aggregation.selected_stage,
                        session.data.aggregation.result_limit,
                        session.data.aggregation.results_page,
                    )
                })
                .unwrap_or((Vec::new(), None, 50, 0));
            (
                session_key.database.clone(),
                session_key.collection.clone(),
                stages,
                selected_stage,
                result_limit,
                results_page,
            )
        };

        if stages.is_empty() {
            state.update(cx, |state, cx| {
                if let Some(session) = state.session_mut(&session_key) {
                    session.data.aggregation.error = Some("Pipeline is empty".to_string());
                    session.data.aggregation.results = None;
                    session.data.aggregation.last_run_time_ms = None;
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

        let target_index = selected_stage.or_else(|| stages.len().checked_sub(1));
        let Some(target_index) = target_index else {
            return;
        };

        let pipeline = match build_pipeline(&stages, selected_stage) {
            Ok(pipeline) => pipeline,
            Err(err) => {
                state.update(cx, |state, cx| {
                    if let Some(session) = state.session_mut(&session_key) {
                        session.data.aggregation.error = Some(format!("Invalid stage JSON: {err}"));
                        session.data.aggregation.results = None;
                        session.data.aggregation.last_run_time_ms = None;
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

        let per_page = if result_limit > 0 { result_limit } else { 50 };
        let limited = per_page > 0;
        let skip = results_page.saturating_mul(per_page as u64);
        let skip_i64 = skip.min(i64::MAX as u64) as i64;

        let task = cx.background_spawn({
            let database_for_task = database.clone();
            let collection_for_task = collection.clone();
            let pipeline_for_task = pipeline.clone();
            let stages_for_task = stages.clone();
            async move {
                let manager = get_connection_manager();
                let ctx = AggregationRunContext {
                    manager,
                    client: &client,
                    database: &database_for_task,
                    collection: &collection_for_task,
                };
                let params = AggregationRunParams {
                    stages: stages_for_task,
                    target_index,
                    pipeline: pipeline_for_task,
                    pagination: AggregationPagination { per_page, skip: skip_i64 },
                };
                run_pipeline_with_stage_stats(&ctx, params)
            }
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<AggregationRunResult, AggregationRunError> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok(run) => {
                        let count = run.documents.len();
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.aggregation.results = Some(run.documents);
                                session.data.aggregation.loading = false;
                                session.data.aggregation.error = None;
                                session.data.aggregation.stage_doc_counts = run.stage_stats;
                                session.data.aggregation.last_run_time_ms = Some(run.run_time_ms);
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
                                session.data.aggregation.error = Some(error.to_string());
                                session.data.aggregation.last_run_time_ms = None;
                            }
                            let event = AppEvent::AggregationFailed {
                                session: session_key.clone(),
                                error: error.to_string(),
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

#[derive(Debug)]
struct AggregationRunResult {
    documents: Vec<Document>,
    stage_stats: Vec<StageDocCounts>,
    run_time_ms: u64,
}

#[derive(Debug)]
enum AggregationRunError {
    Pipeline(String),
    Mongo(crate::error::Error),
}

impl std::fmt::Display for AggregationRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipeline(message) => write!(f, "{message}"),
            Self::Mongo(error) => write!(f, "Aggregation failed: {error}"),
        }
    }
}

impl From<crate::error::Error> for AggregationRunError {
    fn from(value: crate::error::Error) -> Self {
        Self::Mongo(value)
    }
}

struct AggregationRunContext<'a> {
    manager: &'a ConnectionManager,
    client: &'a mongodb::Client,
    database: &'a str,
    collection: &'a str,
}

struct AggregationPagination {
    per_page: i64,
    skip: i64,
}

struct AggregationRunParams {
    stages: Vec<PipelineStage>,
    target_index: usize,
    pipeline: Vec<Document>,
    pagination: AggregationPagination,
}

fn run_pipeline_with_stage_stats(
    ctx: &AggregationRunContext<'_>,
    params: AggregationRunParams,
) -> Result<AggregationRunResult, AggregationRunError> {
    let AggregationRunParams { stages, target_index, pipeline, pagination } = params;
    let AggregationPagination { per_page, skip } = pagination;

    let mut stage_stats = vec![StageDocCounts::default(); stages.len()];

    let (base_count, _) = run_count(ctx, Vec::new())?;
    let mut prev_output = Some(base_count);

    for idx in 0..=target_index {
        let Some(stage) = stages.get(idx) else {
            break;
        };
        if let Some(counts) = stage_stats.get_mut(idx) {
            counts.input = prev_output;
        }

        if !stage.enabled {
            if let Some(counts) = stage_stats.get_mut(idx) {
                counts.output = prev_output;
                counts.time_ms = Some(0);
            }
            continue;
        }

        let pipeline_for_stage =
            build_pipeline(&stages, Some(idx)).map_err(AggregationRunError::Pipeline)?;
        let (count, elapsed_ms) = run_count(ctx, pipeline_for_stage)?;
        if let Some(counts) = stage_stats.get_mut(idx) {
            counts.output = Some(count);
            counts.time_ms = Some(elapsed_ms);
        }
        prev_output = Some(count);
    }

    let mut results_pipeline = pipeline;
    if skip > 0 {
        results_pipeline.push(doc! { "$skip": skip });
    }
    let start = Instant::now();
    let documents = ctx
        .manager
        .aggregate_pipeline(
            ctx.client,
            ctx.database,
            ctx.collection,
            results_pipeline,
            Some(per_page),
        )
        .map_err(AggregationRunError::from)?;
    let run_time_ms = start.elapsed().as_millis() as u64;

    Ok(AggregationRunResult { documents, stage_stats, run_time_ms })
}

fn run_count(
    ctx: &AggregationRunContext<'_>,
    mut pipeline: Vec<Document>,
) -> Result<(u64, u64), AggregationRunError> {
    pipeline.push(doc! { "$count": "__openmango_count" });
    let start = Instant::now();
    let docs = ctx
        .manager
        .aggregate_pipeline(ctx.client, ctx.database, ctx.collection, pipeline, None)
        .map_err(AggregationRunError::from)?;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let count = docs.first().map(count_from_doc).unwrap_or(0);
    Ok((count, elapsed_ms))
}

fn count_from_doc(doc: &Document) -> u64 {
    let Some(value) = doc.get("__openmango_count") else {
        return 0;
    };
    match value {
        Bson::Int64(v) => (*v).max(0) as u64,
        Bson::Int32(v) => (*v).max(0) as u64,
        Bson::Double(v) => (*v).max(0.0) as u64,
        _ => 0,
    }
}
