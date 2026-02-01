//! Aggregation pipeline execution commands.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use gpui::{App, AppContext as _, Entity};

use crate::bson::parse_bson_from_relaxed_json;
use crate::connection::{AggregatePipelineError, ConnectionManager, get_connection_manager};
use crate::state::app_state::{PipelineStage, StageDocCounts, StageStatsMode};
use crate::state::{AppCommands, AppEvent, AppState, SessionKey, StatusMessage};
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

        let (
            database,
            collection,
            stages,
            selected_stage,
            result_limit,
            results_page,
            stage_stats_mode,
            run_generation,
            abort_handle,
        ) = {
            let state_ref = state.read(cx);
            let (
                stages,
                selected_stage,
                result_limit,
                results_page,
                stage_stats_mode,
                run_generation,
                abort_handle,
            ) = state_ref
                .session(&session_key)
                .map(|session| {
                    (
                        session.data.aggregation.stages.clone(),
                        session.data.aggregation.selected_stage,
                        session.data.aggregation.result_limit,
                        session.data.aggregation.results_page,
                        session.data.aggregation.stage_stats_mode,
                        session.data.aggregation.run_generation.clone(),
                        session.data.aggregation.abort_handle.clone(),
                    )
                })
                .unwrap_or((
                    Vec::new(),
                    None,
                    50,
                    0,
                    StageStatsMode::default(),
                    Arc::new(AtomicU64::new(0)),
                    Arc::new(std::sync::Mutex::new(None)),
                ));
            (
                session_key.database.clone(),
                session_key.collection.clone(),
                stages,
                selected_stage,
                result_limit,
                results_page,
                stage_stats_mode,
                run_generation,
                abort_handle,
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

        let has_write_stage = pipeline_has_write_stage(&stages, selected_stage);
        if has_write_stage && !Self::ensure_writable(&state, Some(session_key.connection_id), cx) {
            state.update(cx, |state, cx| {
                if let Some(session) = state.session_mut(&session_key) {
                    session.data.aggregation.error = Some(
                        "Write stages ($out/$merge) require a writable connection.".to_string(),
                    );
                    session.data.aggregation.results = None;
                    session.data.aggregation.last_run_time_ms = None;
                    session.data.aggregation.loading = false;
                }
                cx.notify();
            });
            let event = AppEvent::AggregationFailed {
                session: session_key,
                error: "Write stages require a writable connection.".to_string(),
            };
            state.update(cx, |state, cx| {
                state.update_status_from_event(&event);
                cx.emit(event);
                cx.notify();
            });
            return;
        }

        let (request_id, run_generation_value) = state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            if let Ok(mut handle) = session.data.aggregation.abort_handle.lock()
                && let Some(handle) = handle.take()
            {
                handle.abort();
            }
            session.data.aggregation.loading = true;
            session.data.aggregation.error = None;
            session.data.aggregation.request_id += 1;
            let request_id = session.data.aggregation.request_id;
            let run_generation_value =
                session.data.aggregation.run_generation.fetch_add(1, Ordering::SeqCst) + 1;
            cx.notify();
            (request_id, run_generation_value)
        });

        let per_page = if result_limit > 0 { result_limit } else { 50 };
        let limited = per_page > 0 && !has_write_stage;
        let skip = if has_write_stage { 0 } else { results_page.saturating_mul(per_page as u64) };
        let skip_i64 = skip.min(i64::MAX as u64) as i64;

        let task = cx.background_spawn({
            let database_for_task = database.clone();
            let collection_for_task = collection.clone();
            let stages_for_task = stages.clone();
            let run_generation_for_task = run_generation.clone();
            let stage_stats_mode_for_task = stage_stats_mode;
            let abort_handle_for_task = abort_handle.clone();
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
                    has_write_stage,
                    stage_stats_mode: stage_stats_mode_for_task,
                    run_generation: run_generation_for_task,
                    abort_handle: abort_handle_for_task,
                    run_generation_value,
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
                        let applied = state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return false;
                            };
                            if session.data.aggregation.request_id != request_id {
                                return false;
                            }
                            session.data.aggregation.results = Some(run.documents);
                            session.data.aggregation.loading = false;
                            session.data.aggregation.error = None;
                            session.data.aggregation.stage_doc_counts = run.stage_stats;
                            session.data.aggregation.last_run_time_ms = Some(run.run_time_ms);
                            let event = AppEvent::AggregationCompleted {
                                session: session_key.clone(),
                                count,
                                preview,
                                limited,
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                            true
                        });
                        if applied && has_write_stage {
                            state.update(cx, |state, cx| {
                                state.set_status_message(Some(StatusMessage::info(
                                    "Write stage detected: stage stats and preview limit are disabled.",
                                )));
                                cx.notify();
                            });
                        }
                    }
                    Err(AggregationRunError::Cancelled) => {}
                    Err(error) => {
                        let error_message = error.to_string();
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            if session.data.aggregation.request_id != request_id {
                                return;
                            }
                            session.data.aggregation.loading = false;
                            session.data.aggregation.error = Some(error_message.clone());
                            session.data.aggregation.last_run_time_ms = None;
                            let event = AppEvent::AggregationFailed {
                                session: session_key.clone(),
                                error: error_message,
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

fn pipeline_has_write_stage(stages: &[PipelineStage], selected_stage: Option<usize>) -> bool {
    let target_index = selected_stage.or_else(|| stages.len().checked_sub(1));
    let Some(target_index) = target_index else {
        return false;
    };
    (0..=target_index).any(|idx| {
        stages.get(idx).is_some_and(|stage| {
            if !stage.enabled {
                return false;
            }
            matches!(stage.operator.trim(), "$out" | "$merge")
        })
    })
}

fn build_stage_doc(stage: &PipelineStage, idx: usize) -> Result<Document, AggregationRunError> {
    let operator = stage.operator.trim();
    if operator.is_empty() {
        return Err(AggregationRunError::Pipeline(format!("Stage {} has no operator", idx + 1)));
    }
    let body = stage.body.trim();
    let body_bson = if body.is_empty() || body == "{}" {
        Bson::Document(Document::new())
    } else {
        parse_bson_from_relaxed_json(body).map_err(|err| {
            AggregationRunError::Pipeline(format!("Stage {} ({operator}): {err}", idx + 1))
        })?
    };
    let mut stage_doc = Document::new();
    stage_doc.insert(operator, body_bson);
    Ok(stage_doc)
}

fn parse_pipeline_slice(
    stages: &[PipelineStage],
    target_index: usize,
    run_generation: &Arc<AtomicU64>,
    run_generation_value: u64,
) -> Result<Vec<Option<Document>>, AggregationRunError> {
    let mut parsed = Vec::with_capacity(target_index.saturating_add(1));
    for idx in 0..=target_index {
        if is_cancelled(run_generation, run_generation_value) {
            return Err(AggregationRunError::Cancelled);
        }
        let Some(stage) = stages.get(idx) else {
            break;
        };
        if !stage.enabled {
            parsed.push(None);
            continue;
        }
        parsed.push(Some(build_stage_doc(stage, idx)?));
    }
    Ok(parsed)
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
    Cancelled,
}

impl std::fmt::Display for AggregationRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipeline(message) => write!(f, "{message}"),
            Self::Mongo(error) => write!(f, "Aggregation failed: {error}"),
            Self::Cancelled => write!(f, "Aggregation cancelled"),
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
    has_write_stage: bool,
    stage_stats_mode: StageStatsMode,
    run_generation: Arc<AtomicU64>,
    abort_handle: Arc<std::sync::Mutex<Option<futures::future::AbortHandle>>>,
    run_generation_value: u64,
    pagination: AggregationPagination,
}

fn run_pipeline_with_stage_stats(
    ctx: &AggregationRunContext<'_>,
    params: AggregationRunParams,
) -> Result<AggregationRunResult, AggregationRunError> {
    let AggregationRunParams {
        stages,
        target_index,
        has_write_stage,
        stage_stats_mode,
        run_generation,
        abort_handle,
        run_generation_value,
        pagination,
    } = params;
    let AggregationPagination { per_page, skip } = pagination;

    if is_cancelled(&run_generation, run_generation_value) {
        return Err(AggregationRunError::Cancelled);
    }

    let mut stage_stats = vec![StageDocCounts::default(); stages.len()];

    let parsed_slice =
        parse_pipeline_slice(&stages, target_index, &run_generation, run_generation_value)?;
    let stage_counts_enabled = stage_stats_mode.counts_enabled() && !has_write_stage;
    let stage_timings_enabled = stage_stats_mode.timings_enabled() && stage_counts_enabled;
    let mut prev_output = if stage_counts_enabled {
        Some(
            run_count(
                ctx,
                Vec::new(),
                stage_timings_enabled,
                &run_generation,
                run_generation_value,
                &abort_handle,
            )?
            .0,
        )
    } else {
        None
    };
    let mut running_pipeline: Vec<Document> = Vec::new();

    for idx in 0..=target_index {
        if is_cancelled(&run_generation, run_generation_value) {
            return Err(AggregationRunError::Cancelled);
        }
        let Some(stage) = stages.get(idx) else {
            break;
        };
        if let Some(counts) = stage_stats.get_mut(idx) {
            counts.input = prev_output;
        }

        if !stage.enabled {
            if let Some(counts) = stage_stats.get_mut(idx) {
                counts.output = prev_output;
                counts.time_ms = if stage_counts_enabled { Some(0) } else { None };
            }
            continue;
        }

        let Some(stage_doc) = parsed_slice.get(idx).and_then(|doc| doc.clone()) else {
            continue;
        };
        running_pipeline.push(stage_doc);

        if stage_counts_enabled {
            let (count, elapsed_ms) = run_count(
                ctx,
                running_pipeline.clone(),
                stage_timings_enabled,
                &run_generation,
                run_generation_value,
                &abort_handle,
            )?;
            if let Some(counts) = stage_stats.get_mut(idx) {
                counts.output = Some(count);
                counts.time_ms = elapsed_ms;
            }
            prev_output = Some(count);
        }
    }

    let mut results_pipeline = running_pipeline;
    if skip > 0 && !has_write_stage {
        results_pipeline.push(doc! { "$skip": skip });
    }
    if is_cancelled(&run_generation, run_generation_value) {
        return Err(AggregationRunError::Cancelled);
    }
    let abort_registration = register_abort_handle(&abort_handle);
    let start = Instant::now();
    let documents = match ctx.manager.aggregate_pipeline_abortable(
        ctx.client,
        ctx.database,
        ctx.collection,
        results_pipeline,
        if has_write_stage { None } else { Some(per_page) },
        !has_write_stage,
        abort_registration,
    ) {
        Ok(documents) => documents,
        Err(AggregatePipelineError::Aborted) => return Err(AggregationRunError::Cancelled),
        Err(AggregatePipelineError::Mongo(error)) => return Err(AggregationRunError::Mongo(error)),
    };
    let run_time_ms = start.elapsed().as_millis() as u64;

    Ok(AggregationRunResult { documents, stage_stats, run_time_ms })
}

fn run_count(
    ctx: &AggregationRunContext<'_>,
    mut pipeline: Vec<Document>,
    include_timing: bool,
    run_generation: &Arc<AtomicU64>,
    run_generation_value: u64,
    abort_handle: &Arc<std::sync::Mutex<Option<futures::future::AbortHandle>>>,
) -> Result<(u64, Option<u64>), AggregationRunError> {
    if is_cancelled(run_generation, run_generation_value) {
        return Err(AggregationRunError::Cancelled);
    }
    pipeline.push(doc! { "$count": "__openmango_count" });
    let start = include_timing.then(Instant::now);
    let abort_registration = register_abort_handle(abort_handle);
    let docs = match ctx.manager.aggregate_pipeline_abortable(
        ctx.client,
        ctx.database,
        ctx.collection,
        pipeline,
        None,
        false,
        abort_registration,
    ) {
        Ok(docs) => docs,
        Err(AggregatePipelineError::Aborted) => return Err(AggregationRunError::Cancelled),
        Err(AggregatePipelineError::Mongo(error)) => return Err(AggregationRunError::Mongo(error)),
    };
    if is_cancelled(run_generation, run_generation_value) {
        return Err(AggregationRunError::Cancelled);
    }
    let elapsed_ms = start.map(|start| start.elapsed().as_millis() as u64);
    let count = docs.first().map(count_from_doc).unwrap_or(0);
    Ok((count, elapsed_ms))
}

fn is_cancelled(run_generation: &Arc<AtomicU64>, run_generation_value: u64) -> bool {
    run_generation.load(Ordering::SeqCst) != run_generation_value
}

fn register_abort_handle(
    abort_handle: &Arc<std::sync::Mutex<Option<futures::future::AbortHandle>>>,
) -> futures::future::AbortRegistration {
    let (handle, registration) = futures::future::AbortHandle::new_pair();
    if let Ok(mut current) = abort_handle.lock() {
        if let Some(previous) = current.take() {
            previous.abort();
        }
        *current = Some(handle);
    }
    registration
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

#[cfg(test)]
mod tests {
    use super::{build_stage_doc, is_cancelled, pipeline_has_write_stage};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    use mongodb::bson::doc;

    use crate::state::app_state::PipelineStage;

    #[test]
    fn pipeline_has_write_stage_respects_selection_and_enabled() {
        let stages = vec![
            PipelineStage { operator: "$match".to_string(), body: "{}".to_string(), enabled: true },
            PipelineStage { operator: "$out".to_string(), body: "{}".to_string(), enabled: true },
            PipelineStage { operator: "$merge".to_string(), body: "{}".to_string(), enabled: true },
        ];

        assert!(pipeline_has_write_stage(&stages, None));
        assert!(!pipeline_has_write_stage(&stages, Some(0)));

        let mut disabled = stages.clone();
        disabled[1].enabled = false;
        disabled[2].enabled = false;
        assert!(!pipeline_has_write_stage(&disabled, None));
    }

    #[test]
    fn build_stage_doc_parses_and_handles_empty_body() {
        let parsed = build_stage_doc(
            &PipelineStage {
                operator: "$match".to_string(),
                body: r#"{ "status": "active" }"#.to_string(),
                enabled: true,
            },
            0,
        )
        .expect("stage should parse");
        assert_eq!(parsed, doc! { "$match": { "status": "active" } });

        let empty = build_stage_doc(
            &PipelineStage { operator: "$match".to_string(), body: "".to_string(), enabled: true },
            1,
        )
        .expect("empty body should become {}");
        assert_eq!(empty, doc! { "$match": {} });
    }

    #[test]
    fn build_stage_doc_errors_on_empty_operator() {
        let err = build_stage_doc(
            &PipelineStage { operator: "   ".to_string(), body: "{}".to_string(), enabled: true },
            2,
        )
        .expect_err("empty operator should error");
        let message = err.to_string();
        assert!(message.contains("Stage 3 has no operator"));
    }

    #[test]
    fn is_cancelled_detects_generation_changes() {
        let generation = Arc::new(AtomicU64::new(0));
        assert!(!is_cancelled(&generation, 0));
        generation.fetch_add(1, Ordering::SeqCst);
        assert!(is_cancelled(&generation, 0));
        assert!(!is_cancelled(&generation, 1));
    }
}
