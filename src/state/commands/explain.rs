//! Explain command handlers for query and aggregation diagnostics.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext as _, Entity};
use mongodb::bson::{Bson, Document};

use crate::bson::parse_bson_from_relaxed_json;
use crate::connection::ops::explain::ExplainFindRequest;
use crate::state::app_state::PipelineStage;
use crate::state::{
    AppCommands, AppEvent, AppState, ExplainBottleneck, ExplainCostBand, ExplainNode,
    ExplainOpenMode, ExplainRejectedPlan, ExplainRun, ExplainScope, ExplainSeverity,
    ExplainSummary, ExplainViewMode, SessionKey,
};

const EXPLAIN_VERBOSITY: &str = "executionStats";
const EXPLAIN_HISTORY_LIMIT: usize = 20;

#[derive(Default)]
struct ParsedExplain {
    nodes: Vec<ExplainNode>,
    summary: ExplainSummary,
    rejected_plans: Vec<ExplainRejectedPlan>,
    bottlenecks: Vec<ExplainBottleneck>,
}

impl AppCommands {
    pub fn run_explain_for_session(state: Entity<AppState>, session_key: SessionKey, cx: &mut App) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let (database, collection, filter, sort, projection, signature) = {
            let state_ref = state.read(cx);
            let Some(session) = state_ref.session(&session_key) else {
                return;
            };
            (
                session_key.database.clone(),
                session_key.collection.clone(),
                session.data.filter.clone(),
                session.data.sort.clone(),
                session.data.projection.clone(),
                signature_for_find(&session_key, session),
            )
        };

        state.update(cx, |state, cx| {
            let Some(session) = state.session_mut(&session_key) else {
                return;
            };
            session.data.explain.loading = true;
            session.data.explain.error = None;
            session.data.explain.scope = ExplainScope::Find;
            session.data.explain.open_mode = ExplainOpenMode::Modal;
            session.data.explain.view_mode = ExplainViewMode::Tree;

            let event = AppEvent::ExplainStarted {
                session: session_key.clone(),
                scope: ExplainScope::Find,
            };
            state.update_status_from_event(&event);
            cx.emit(event);
            cx.notify();
        });

        let manager = state.read(cx).connection_manager();
        let task = cx.background_spawn(async move {
            manager.explain_find(
                &client,
                ExplainFindRequest {
                    database,
                    collection,
                    filter,
                    sort,
                    projection,
                    verbosity: EXPLAIN_VERBOSITY.to_string(),
                },
            )
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Document, crate::error::Error> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok(explain_doc) => {
                        let parsed = parse_explain_document(&explain_doc);
                        let raw_json = explain_to_pretty_json(&explain_doc);
                        let generated_at_unix_ms = now_unix_ms();
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            let explain = &mut session.data.explain;
                            explain.loading = false;
                            explain.error = None;
                            explain.scope = ExplainScope::Find;
                            explain.open_mode = ExplainOpenMode::Modal;
                            explain.view_mode = ExplainViewMode::Tree;
                            explain.stale = false;
                            let run = ExplainRun {
                                id: format!("{generated_at_unix_ms}-{signature:016x}"),
                                generated_at_unix_ms,
                                signature: Some(signature),
                                scope: ExplainScope::Find,
                                raw_json,
                                nodes: parsed.nodes,
                                summary: parsed.summary,
                                rejected_plans: parsed.rejected_plans,
                                bottlenecks: parsed.bottlenecks,
                            };
                            explain.push_run_with_limit(run, EXPLAIN_HISTORY_LIMIT);

                            let event = AppEvent::ExplainCompleted {
                                session: session_key.clone(),
                                scope: ExplainScope::Find,
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        let error_message = error.to_string();
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            let explain = &mut session.data.explain;
                            explain.loading = false;
                            explain.scope = ExplainScope::Find;
                            explain.open_mode = ExplainOpenMode::Modal;
                            explain.view_mode = ExplainViewMode::Tree;
                            explain.error = Some(error_message.clone());
                            explain.mark_stale();

                            let event = AppEvent::ExplainFailed {
                                session: session_key.clone(),
                                scope: ExplainScope::Find,
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

    pub fn run_explain_for_aggregation(
        state: Entity<AppState>,
        session_key: SessionKey,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };

        let (database, collection, pipeline, signature) = {
            let state_ref = state.read(cx);
            let Some(session) = state_ref.session(&session_key) else {
                return;
            };
            let stages = &session.data.aggregation.stages;
            let selected_stage = session.data.aggregation.selected_stage;
            let pipeline = match build_explain_pipeline(stages, selected_stage) {
                Ok(pipeline) => pipeline,
                Err(error) => {
                    state.update(cx, |state, cx| {
                        let Some(session) = state.session_mut(&session_key) else {
                            return;
                        };
                        let explain = &mut session.data.explain;
                        explain.loading = false;
                        explain.scope = ExplainScope::Aggregation;
                        explain.error = Some(error.clone());
                        explain.open_mode = ExplainOpenMode::Modal;
                        explain.view_mode = ExplainViewMode::Tree;
                        explain.mark_stale();

                        let event = AppEvent::ExplainFailed {
                            session: session_key.clone(),
                            scope: ExplainScope::Aggregation,
                            error,
                        };
                        state.update_status_from_event(&event);
                        cx.emit(event);
                        cx.notify();
                    });
                    return;
                }
            };

            (
                session_key.database.clone(),
                session_key.collection.clone(),
                pipeline,
                signature_for_aggregation(&session_key, stages, selected_stage),
            )
        };

        state.update(cx, |state, cx| {
            let Some(session) = state.session_mut(&session_key) else {
                return;
            };
            session.data.explain.loading = true;
            session.data.explain.error = None;
            session.data.explain.scope = ExplainScope::Aggregation;
            session.data.explain.open_mode = ExplainOpenMode::Modal;
            session.data.explain.view_mode = ExplainViewMode::Tree;

            let event = AppEvent::ExplainStarted {
                session: session_key.clone(),
                scope: ExplainScope::Aggregation,
            };
            state.update_status_from_event(&event);
            cx.emit(event);
            cx.notify();
        });

        let manager = state.read(cx).connection_manager();
        let task = cx.background_spawn(async move {
            manager.explain_aggregation(
                &client,
                &database,
                &collection,
                pipeline,
                EXPLAIN_VERBOSITY,
            )
        });

        cx.spawn({
            let state = state.clone();
            let session_key = session_key.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result: Result<Document, crate::error::Error> = task.await;

                let _ = cx.update(|cx| match result {
                    Ok(explain_doc) => {
                        let parsed = parse_explain_document(&explain_doc);
                        let raw_json = explain_to_pretty_json(&explain_doc);
                        let generated_at_unix_ms = now_unix_ms();
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            let explain = &mut session.data.explain;
                            explain.loading = false;
                            explain.error = None;
                            explain.scope = ExplainScope::Aggregation;
                            explain.open_mode = ExplainOpenMode::Modal;
                            explain.view_mode = ExplainViewMode::Tree;
                            explain.stale = false;
                            let run = ExplainRun {
                                id: format!("{generated_at_unix_ms}-{signature:016x}"),
                                generated_at_unix_ms,
                                signature: Some(signature),
                                scope: ExplainScope::Aggregation,
                                raw_json,
                                nodes: parsed.nodes,
                                summary: parsed.summary,
                                rejected_plans: parsed.rejected_plans,
                                bottlenecks: parsed.bottlenecks,
                            };
                            explain.push_run_with_limit(run, EXPLAIN_HISTORY_LIMIT);

                            let event = AppEvent::ExplainCompleted {
                                session: session_key.clone(),
                                scope: ExplainScope::Aggregation,
                            };
                            state.update_status_from_event(&event);
                            cx.emit(event);
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        let error_message = error.to_string();
                        state.update(cx, |state, cx| {
                            let Some(session) = state.session_mut(&session_key) else {
                                return;
                            };
                            let explain = &mut session.data.explain;
                            explain.loading = false;
                            explain.scope = ExplainScope::Aggregation;
                            explain.open_mode = ExplainOpenMode::Modal;
                            explain.view_mode = ExplainViewMode::Tree;
                            explain.error = Some(error_message.clone());
                            explain.mark_stale();

                            let event = AppEvent::ExplainFailed {
                                session: session_key.clone(),
                                scope: ExplainScope::Aggregation,
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

fn build_explain_pipeline(
    stages: &[PipelineStage],
    selected_stage: Option<usize>,
) -> Result<Vec<Document>, String> {
    let end_index = selected_stage.unwrap_or_else(|| stages.len().saturating_sub(1));
    let mut pipeline = Vec::new();

    for (idx, stage) in stages.iter().enumerate() {
        if idx > end_index {
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
        let body_bson = if body.is_empty() || body == "{}" {
            Bson::Document(Document::new())
        } else {
            parse_bson_from_relaxed_json(body)
                .map_err(|err| format!("Stage {} has invalid body JSON: {}", idx + 1, err))?
        };
        let mut stage_doc = Document::new();
        stage_doc.insert(operator.to_string(), body_bson);
        pipeline.push(stage_doc);
    }

    if pipeline.is_empty() {
        return Err("Pipeline is empty. Add or enable at least one stage.".to_string());
    }
    Ok(pipeline)
}

fn signature_for_find(session_key: &SessionKey, session: &crate::state::SessionState) -> u64 {
    let mut hasher = DefaultHasher::new();
    session_key.connection_id.hash(&mut hasher);
    session_key.database.hash(&mut hasher);
    session_key.collection.hash(&mut hasher);
    session.data.filter_raw.hash(&mut hasher);
    session.data.sort_raw.hash(&mut hasher);
    session.data.projection_raw.hash(&mut hasher);
    hasher.finish()
}

fn signature_for_aggregation(
    session_key: &SessionKey,
    stages: &[PipelineStage],
    selected_stage: Option<usize>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    session_key.connection_id.hash(&mut hasher);
    session_key.database.hash(&mut hasher);
    session_key.collection.hash(&mut hasher);
    selected_stage.hash(&mut hasher);
    for stage in stages {
        stage.operator.hash(&mut hasher);
        stage.body.hash(&mut hasher);
        stage.enabled.hash(&mut hasher);
    }
    hasher.finish()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn explain_to_pretty_json(explain_doc: &Document) -> String {
    let value = Bson::Document(explain_doc.clone()).into_relaxed_extjson();
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn parse_explain_document(explain_doc: &Document) -> ParsedExplain {
    let mut parsed = ParsedExplain::default();

    if let Ok(query_planner) = explain_doc.get_document("queryPlanner") {
        if let Ok(winning_plan) = query_planner.get_document("winningPlan") {
            append_stage_tree(
                winning_plan,
                0,
                "1".to_string(),
                None,
                &mut parsed.nodes,
                StageTreeMode::Planner,
            );
        }
        append_rejected_plans_from_query_planner(query_planner, "r", &mut parsed.rejected_plans);
    } else if let Some(exec_stage) = explain_doc
        .get_document("executionStats")
        .ok()
        .and_then(|stats| stats.get_document("executionStages").ok())
    {
        append_stage_tree(
            exec_stage,
            0,
            "1".to_string(),
            None,
            &mut parsed.nodes,
            StageTreeMode::Execution,
        );
    } else if let Ok(stages) = explain_doc.get_array("stages") {
        append_aggregation_stages(stages, &mut parsed.nodes, &mut parsed.rejected_plans);
    }

    if parsed.nodes.is_empty() {
        // Fallback: keep at least one node so the visual tree is meaningful.
        parsed.nodes.push(ExplainNode {
            id: "1".to_string(),
            parent_id: None,
            label: "Explain".to_string(),
            depth: 0,
            n_returned: None,
            docs_examined: None,
            keys_examined: None,
            time_ms: None,
            index_name: None,
            is_multi_key: None,
            is_covered: None,
            extra_metrics: Vec::new(),
            cost_band: ExplainCostBand::Low,
            severity: ExplainSeverity::Low,
        });
    }

    parsed.summary = build_summary(explain_doc, &parsed.nodes);
    parsed.bottlenecks = rank_bottlenecks(&parsed.nodes);
    parsed
}

fn append_aggregation_stages(
    stages: &[Bson],
    out: &mut Vec<ExplainNode>,
    rejected_out: &mut Vec<ExplainRejectedPlan>,
) {
    for (index, stage) in stages.iter().enumerate() {
        let Some(stage_doc) = stage.as_document() else {
            continue;
        };
        let Some((name, value)) = stage_doc.iter().next() else {
            continue;
        };

        let path = format!("{}", index + 1);
        if let Some(inner) = value.as_document() {
            let cursor_totals = inner.get_document("executionStats").ok();
            let docs_examined = read_u64(inner, "docsExamined")
                .or_else(|| read_u64(inner, "totalDocsExamined"))
                .or_else(|| cursor_totals.and_then(|stats| read_u64(stats, "totalDocsExamined")));
            let keys_examined = read_u64(inner, "keysExamined")
                .or_else(|| read_u64(inner, "totalKeysExamined"))
                .or_else(|| cursor_totals.and_then(|stats| read_u64(stats, "totalKeysExamined")));
            let n_returned =
                read_u64(inner, "nReturned").or_else(|| read_u64(stage_doc, "nReturned"));
            let time_ms = read_u64(inner, "executionTimeMillisEstimate")
                .or_else(|| read_u64(inner, "executionTimeMillis"))
                .or_else(|| read_u64(stage_doc, "executionTimeMillisEstimate"))
                .or_else(|| read_u64(stage_doc, "executionTimeMillis"))
                .or_else(|| cursor_totals.and_then(|stats| read_u64(stats, "executionTimeMillis")));

            // `$cursor` often contains the detailed plan tree.
            if name == "$cursor" {
                if let Some(winning_plan) = inner
                    .get_document("queryPlanner")
                    .ok()
                    .and_then(|planner| planner.get_document("winningPlan").ok())
                {
                    if let Ok(query_planner) = inner.get_document("queryPlanner") {
                        append_rejected_plans_from_query_planner(
                            query_planner,
                            &format!("r.{path}"),
                            rejected_out,
                        );
                    }
                    let start_index = out.len();
                    append_stage_tree(
                        winning_plan,
                        0,
                        path.clone(),
                        None,
                        out,
                        StageTreeMode::Planner,
                    );
                    if let Some(root) = out.get_mut(start_index) {
                        seed_missing_stage_metrics(
                            root,
                            n_returned,
                            docs_examined,
                            keys_examined,
                            time_ms,
                        );
                    }
                    continue;
                } else if let Some(exec_stage) = inner
                    .get_document("executionStats")
                    .ok()
                    .and_then(|stats| stats.get_document("executionStages").ok())
                {
                    append_stage_tree(
                        exec_stage,
                        0,
                        path.clone(),
                        None,
                        out,
                        StageTreeMode::Execution,
                    );
                    continue;
                }
            }

            let cost_band = cost_band(docs_examined, keys_examined, n_returned);
            let severity = severity_for_stage(name, cost_band);
            let index_name = inner.get_str("indexName").ok().map(ToString::to_string);
            let is_multi_key = read_bool(inner, "isMultiKey");
            let is_covered = read_bool(inner, "indexOnly")
                .or_else(|| infer_covered_query(name, docs_examined, keys_examined));
            out.push(ExplainNode {
                id: path.clone(),
                parent_id: None,
                label: name.clone(),
                depth: 0,
                n_returned,
                docs_examined,
                keys_examined,
                time_ms,
                index_name,
                is_multi_key,
                is_covered,
                extra_metrics: collect_extra_metrics(inner),
                cost_band,
                severity,
            });
        } else {
            let cost_band = ExplainCostBand::Low;
            out.push(ExplainNode {
                id: path,
                parent_id: None,
                label: name.clone(),
                depth: 0,
                n_returned: None,
                docs_examined: None,
                keys_examined: None,
                time_ms: None,
                index_name: None,
                is_multi_key: None,
                is_covered: None,
                extra_metrics: Vec::new(),
                cost_band,
                severity: severity_for_stage(name, cost_band),
            });
        }
    }
}

#[derive(Clone, Copy)]
enum StageTreeMode {
    Planner,
    Execution,
}

fn append_stage_tree(
    doc: &Document,
    depth: usize,
    path: String,
    parent_id: Option<String>,
    out: &mut Vec<ExplainNode>,
    mode: StageTreeMode,
) {
    if matches!(mode, StageTreeMode::Planner)
        && doc.get("stage").is_none()
        && let Ok(query_plan) = doc.get_document("queryPlan")
    {
        append_stage_tree(query_plan, depth, path, parent_id, out, mode);
        return;
    }

    let label = stage_label(doc);
    let docs_examined =
        read_u64(doc, "docsExamined").or_else(|| read_u64(doc, "totalDocsExamined"));
    let keys_examined =
        read_u64(doc, "keysExamined").or_else(|| read_u64(doc, "totalKeysExamined"));
    let n_returned = read_u64(doc, "nReturned");
    let time_ms = read_u64(doc, "executionTimeMillisEstimate")
        .or_else(|| read_u64(doc, "executionTimeMillis"));
    let index_name = doc.get_str("indexName").ok().map(ToString::to_string);
    let is_multi_key = read_bool(doc, "isMultiKey");
    let is_covered = read_bool(doc, "indexOnly")
        .or_else(|| infer_covered_query(&label, docs_examined, keys_examined));
    let cost_band = cost_band(docs_examined, keys_examined, n_returned);
    let severity = severity_for_stage(&label, cost_band);

    out.push(ExplainNode {
        id: path.clone(),
        parent_id: parent_id.clone(),
        label,
        depth,
        n_returned,
        docs_examined,
        keys_examined,
        time_ms,
        index_name,
        is_multi_key,
        is_covered,
        extra_metrics: collect_extra_metrics(doc),
        cost_band,
        severity,
    });

    let mut child_index = 1usize;
    let mut append_child = |child: &Document, out: &mut Vec<ExplainNode>| {
        let child_path = format!("{path}.{child_index}");
        child_index += 1;
        append_stage_tree(child, depth + 1, child_path, Some(path.clone()), out, mode);
    };

    if let Ok(child) = doc.get_document("inputStage") {
        append_child(child, out);
    }
    if let Ok(children) = doc.get_array("inputStages") {
        for child in children.iter().filter_map(Bson::as_document) {
            append_child(child, out);
        }
    }

    if matches!(mode, StageTreeMode::Execution) {
        if let Ok(child) = doc.get_document("outerStage") {
            append_child(child, out);
        }
        if let Ok(child) = doc.get_document("innerStage") {
            append_child(child, out);
        }
        if let Ok(child) = doc.get_document("thenStage") {
            append_child(child, out);
        }
        if let Ok(child) = doc.get_document("elseStage") {
            append_child(child, out);
        }
        if doc.get("stage").is_none()
            && let Ok(child) = doc.get_document("queryPlan")
        {
            append_child(child, out);
        }
    }

    if let Ok(shards) = doc.get_array("shards") {
        for shard in shards.iter().filter_map(Bson::as_document) {
            if let Ok(plan) = shard.get_document("winningPlan") {
                append_child(plan, out);
            } else if let Ok(plan) = shard.get_document("queryPlan") {
                append_child(plan, out);
            } else if let Ok(exec) = shard.get_document("executionStages") {
                append_child(exec, out);
            } else {
                append_child(shard, out);
            }
        }
    }
}

fn seed_missing_stage_metrics(
    node: &mut ExplainNode,
    n_returned: Option<u64>,
    docs_examined: Option<u64>,
    keys_examined: Option<u64>,
    time_ms: Option<u64>,
) {
    if node.n_returned.is_none() {
        node.n_returned = n_returned;
    }
    if node.docs_examined.is_none() {
        node.docs_examined = docs_examined;
    }
    if node.keys_examined.is_none() {
        node.keys_examined = keys_examined;
    }
    if node.time_ms.is_none() {
        node.time_ms = time_ms;
    }
}

fn append_rejected_plans_from_query_planner(
    query_planner: &Document,
    id_prefix: &str,
    out: &mut Vec<ExplainRejectedPlan>,
) {
    let Ok(rejected_plans) = query_planner.get_array("rejectedPlans") else {
        return;
    };

    for (index, rejected) in rejected_plans.iter().enumerate() {
        let Some(rejected_doc) = rejected.as_document() else {
            continue;
        };

        let mut nodes = Vec::new();
        append_stage_tree(
            rejected_doc,
            0,
            format!("{id_prefix}.{}", index + 1),
            None,
            &mut nodes,
            StageTreeMode::Planner,
        );
        if nodes.is_empty() {
            continue;
        }

        let docs_examined = nodes.iter().filter_map(|node| node.docs_examined).max();
        let keys_examined = nodes.iter().filter_map(|node| node.keys_examined).max();
        let execution_time_ms = nodes.iter().filter_map(|node| node.time_ms).max();
        let mut index_names = Vec::new();
        for index_name in nodes.iter().filter_map(|node| node.index_name.clone()) {
            if !index_names.contains(&index_name) {
                index_names.push(index_name);
            }
        }
        let root_stage = nodes
            .first()
            .map(|node| node.label.to_ascii_uppercase())
            .unwrap_or_else(|| "PLAN".to_string());
        let reason_hint = infer_rejected_plan_reason(&root_stage, docs_examined, keys_examined);

        out.push(ExplainRejectedPlan {
            plan_id: format!("{id_prefix}.{}", index + 1),
            root_stage,
            reason_hint,
            nodes,
            docs_examined,
            keys_examined,
            execution_time_ms,
            index_names,
        });
    }
}

fn infer_rejected_plan_reason(
    root_stage: &str,
    docs_examined: Option<u64>,
    keys_examined: Option<u64>,
) -> String {
    if root_stage.contains("COLLSCAN") {
        return "Rejected because it required a collection scan compared with a lower-cost alternative."
            .to_string();
    }
    if docs_examined.unwrap_or(0) > 50_000 || keys_examined.unwrap_or(0) > 50_000 {
        return "Rejected due to higher examined volume than the winning plan.".to_string();
    }
    "Planner selected another candidate with lower estimated execution cost.".to_string()
}

fn rank_bottlenecks(nodes: &[ExplainNode]) -> Vec<ExplainBottleneck> {
    let mut ranked: Vec<_> = nodes
        .iter()
        .filter(|node| node.depth <= 8)
        .map(|node| {
            let mut impact_score =
                node.docs_examined.unwrap_or(0) + node.keys_examined.unwrap_or(0);
            impact_score = impact_score.saturating_add(node.time_ms.unwrap_or(0) * 140);
            if node.label.to_ascii_uppercase().contains("COLLSCAN") {
                impact_score = impact_score.saturating_add(40_000);
            }
            if matches!(node.cost_band, ExplainCostBand::High | ExplainCostBand::VeryHigh) {
                impact_score = impact_score.saturating_add(10_000);
            }

            ExplainBottleneck {
                rank: 0,
                node_id: node.id.clone(),
                stage: node.label.clone(),
                impact_score,
                docs_examined: node.docs_examined,
                keys_examined: node.keys_examined,
                execution_time_ms: node.time_ms,
                recommendation: recommendation_for_node(node),
            }
        })
        .collect();

    ranked.sort_by(|left, right| right.impact_score.cmp(&left.impact_score));
    ranked.truncate(5);
    for (index, item) in ranked.iter_mut().enumerate() {
        item.rank = index + 1;
    }
    ranked
}

fn recommendation_for_node(node: &ExplainNode) -> String {
    let upper = node.label.to_ascii_uppercase();
    if upper.contains("COLLSCAN") {
        return "Add a selective index for this predicate path to avoid full collection scans."
            .to_string();
    }
    if upper.contains("SORT") {
        return "Align sort keys with an index prefix to avoid expensive in-memory sort work."
            .to_string();
    }
    if upper.contains("GROUP") && node.docs_examined.unwrap_or(0) > 10_000 {
        return "Push selective $match stages earlier so $group runs on fewer documents."
            .to_string();
    }
    if node.keys_examined.unwrap_or(0) > node.n_returned.unwrap_or(0).saturating_mul(200) {
        return "Index selectivity is low; consider a compound index with a more selective leading key."
            .to_string();
    }
    "Monitor this stage across runs; optimize if examined counts continue to grow.".to_string()
}

fn stage_label(doc: &Document) -> String {
    if let Ok(stage) = doc.get_str("stage") {
        return stage.to_string();
    }
    if let Ok(stage) = doc.get_str("planNodeType") {
        return stage.to_string();
    }
    if let Ok(query_plan) = doc.get_document("queryPlan")
        && let Ok(stage) = query_plan.get_str("stage")
    {
        return stage.to_string();
    }
    if let Ok(stage) = doc.get_str("strategy") {
        return stage.to_string();
    }
    "Stage".to_string()
}

fn build_summary(explain_doc: &Document, nodes: &[ExplainNode]) -> ExplainSummary {
    let mut summary = ExplainSummary::default();
    if let Ok(execution_stats) = explain_doc.get_document("executionStats") {
        summary.n_returned = read_u64(execution_stats, "nReturned");
        summary.docs_examined = read_u64(execution_stats, "totalDocsExamined");
        summary.keys_examined = read_u64(execution_stats, "totalKeysExamined");
        summary.execution_time_ms = read_u64(execution_stats, "executionTimeMillis");
    }
    if summary.execution_time_ms.is_none() {
        summary.execution_time_ms = read_u64(explain_doc, "executionTimeMillis");
    }
    if summary.n_returned.is_none() {
        summary.n_returned = nodes.iter().rev().find_map(|node| node.n_returned);
    }
    if summary.docs_examined.is_none() {
        summary.docs_examined = nodes.iter().filter_map(|node| node.docs_examined).max();
    }
    if summary.keys_examined.is_none() {
        summary.keys_examined = nodes.iter().filter_map(|node| node.keys_examined).max();
    }
    if summary.execution_time_ms.is_none() {
        summary.execution_time_ms = nodes.iter().filter_map(|node| node.time_ms).max();
    }

    summary.has_collscan =
        nodes.iter().any(|node| node.label.to_ascii_uppercase().contains("COLLSCAN"));
    summary.has_sort_stage = nodes.iter().any(|node| {
        let stage = node.label.to_ascii_uppercase();
        stage.contains("SORT") || stage.contains("$SORT")
    });
    summary.covered_indexes = nodes.iter().filter_map(|node| node.index_name.clone()).fold(
        Vec::new(),
        |mut acc, index_name| {
            if !acc.contains(&index_name) {
                acc.push(index_name);
            }
            acc
        },
    );
    summary.is_covered_query = nodes.iter().any(|node| node.is_covered.unwrap_or(false))
        || nodes.iter().any(|node| node.label.to_ascii_uppercase().contains("PROJECTION_COVERED"));
    summary
}

fn read_u64(doc: &Document, key: &str) -> Option<u64> {
    let value = doc.get(key)?;
    match value {
        Bson::Int32(v) if *v >= 0 => Some(*v as u64),
        Bson::Int64(v) if *v >= 0 => Some(*v as u64),
        Bson::Double(v) if *v >= 0.0 => Some(*v as u64),
        _ => None,
    }
}

fn read_bool(doc: &Document, key: &str) -> Option<bool> {
    match doc.get(key)? {
        Bson::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn infer_covered_query(
    stage_label: &str,
    docs_examined: Option<u64>,
    keys_examined: Option<u64>,
) -> Option<bool> {
    let upper = stage_label.to_ascii_uppercase();
    if upper.contains("PROJECTION_COVERED") {
        return Some(true);
    }
    match (docs_examined, keys_examined) {
        (Some(0), Some(keys)) if keys > 0 => Some(true),
        _ => None,
    }
}

fn collect_extra_metrics(doc: &Document) -> Vec<(String, String)> {
    const METRIC_KEYS: &[&str] = &[
        "works",
        "advanced",
        "needTime",
        "needYield",
        "saveState",
        "restoreState",
        "seeks",
        "dupsTested",
        "dupsDropped",
        "spills",
        "memUsageBytes",
    ];

    METRIC_KEYS
        .iter()
        .filter_map(|key| doc.get(*key).map(|value| ((*key).to_string(), value)))
        .map(|(key, value)| (key, bson_metric_to_string(value)))
        .collect()
}

fn bson_metric_to_string(value: &Bson) -> String {
    match value {
        Bson::Int32(v) => v.to_string(),
        Bson::Int64(v) => v.to_string(),
        Bson::Double(v) => format!("{v:.3}"),
        Bson::Boolean(v) => {
            if *v {
                "yes".to_string()
            } else {
                "no".to_string()
            }
        }
        Bson::String(v) => v.clone(),
        _ => value.to_string(),
    }
}

fn cost_band(
    docs_examined: Option<u64>,
    keys_examined: Option<u64>,
    n_returned: Option<u64>,
) -> ExplainCostBand {
    let examined = docs_examined.unwrap_or(0).max(keys_examined.unwrap_or(0));
    if examined == 0 {
        return ExplainCostBand::Low;
    }
    if let Some(returned) = n_returned
        && returned > 0
    {
        let ratio = examined / returned;
        if ratio > 1000 {
            return ExplainCostBand::VeryHigh;
        }
        if ratio > 200 {
            return ExplainCostBand::High;
        }
    }
    if examined >= 100_000 {
        ExplainCostBand::VeryHigh
    } else if examined >= 10_000 {
        ExplainCostBand::High
    } else if examined >= 1_000 {
        ExplainCostBand::Medium
    } else {
        ExplainCostBand::Low
    }
}

fn severity_for_stage(stage: &str, cost_band: ExplainCostBand) -> ExplainSeverity {
    let upper = stage.to_ascii_uppercase();
    if upper.contains("COLLSCAN") {
        return ExplainSeverity::Critical;
    }
    if upper.contains("SORT")
        && matches!(cost_band, ExplainCostBand::High | ExplainCostBand::VeryHigh)
    {
        return ExplainSeverity::High;
    }
    match cost_band {
        ExplainCostBand::Low => ExplainSeverity::Low,
        ExplainCostBand::Medium => ExplainSeverity::Medium,
        ExplainCostBand::High => ExplainSeverity::High,
        ExplainCostBand::VeryHigh => ExplainSeverity::Critical,
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::*;

    #[test]
    fn parse_find_explain_marks_collscan_and_summary_flags() {
        let explain = doc! {
            "executionStats": {
                "nReturned": 10,
                "totalDocsExamined": 2000,
                "totalKeysExamined": 0,
                "executionTimeMillis": 42,
                "executionStages": {
                    "stage": "COLLSCAN",
                    "nReturned": 10,
                    "docsExamined": 2000,
                    "executionTimeMillisEstimate": 42
                }
            }
        };

        let parsed = parse_explain_document(&explain);
        assert!(!parsed.nodes.is_empty());
        assert_eq!(parsed.nodes[0].label, "COLLSCAN");
        assert_eq!(parsed.nodes[0].severity, ExplainSeverity::Critical);
        assert!(parsed.summary.has_collscan);
        assert_eq!(parsed.summary.execution_time_ms, Some(42));
    }

    #[test]
    fn parse_aggregation_stages_array_extracts_rows() {
        let explain = doc! {
            "stages": [
                {
                    "$match": {
                        "nReturned": 5,
                        "docsExamined": 25,
                        "keysExamined": 10,
                        "executionTimeMillisEstimate": 2
                    }
                },
                {
                    "$sort": {
                        "nReturned": 5,
                        "docsExamined": 25,
                        "keysExamined": 10,
                        "executionTimeMillisEstimate": 3
                    }
                }
            ]
        };

        let parsed = parse_explain_document(&explain);
        assert_eq!(parsed.nodes.len(), 2);
        assert_eq!(parsed.nodes[0].label, "$match");
        assert_eq!(parsed.nodes[1].label, "$sort");
        assert_eq!(parsed.nodes[1].time_ms, Some(3));
    }

    #[test]
    fn parse_aggregation_cursor_prefers_planner_chain_over_execution_tree() {
        let explain = doc! {
            "stages": [
                {
                    "$cursor": {
                        "queryPlanner": {
                            "winningPlan": {
                                "isCached": false,
                                "queryPlan": {
                                    "stage": "GROUP",
                                    "inputStage": {
                                        "stage": "PROJECTION_COVERED",
                                        "inputStage": {
                                            "stage": "IXSCAN",
                                            "indexName": "status_1_definitionSnapshot.responsibility_1"
                                        }
                                    }
                                },
                                "slotBasedPlan": { "stages": "omitted" }
                            }
                        },
                        "executionStats": {
                            "nReturned": 3,
                            "totalDocsExamined": 0,
                            "totalKeysExamined": 11396,
                            "executionTimeMillis": 29,
                            "executionStages": {
                                "stage": "project",
                                "inputStage": {
                                    "stage": "group",
                                    "inputStage": {
                                        "stage": "project",
                                        "inputStage": {
                                            "stage": "unique"
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                {
                    "$sort": {
                        "nReturned": 3,
                        "executionTimeMillisEstimate": 12
                    }
                }
            ]
        };

        let parsed = parse_explain_document(&explain);
        let labels: Vec<_> = parsed.nodes.iter().map(|node| node.label.as_str()).collect();

        assert_eq!(labels, vec!["GROUP", "PROJECTION_COVERED", "IXSCAN", "$sort"]);
        assert_eq!(parsed.nodes[0].keys_examined, Some(11396));
        assert_eq!(parsed.summary.n_returned, Some(3));
        assert_eq!(parsed.summary.keys_examined, Some(11396));
    }

    #[test]
    fn parse_query_planner_collects_rejected_plans_and_bottlenecks() {
        let explain = doc! {
            "queryPlanner": {
                "winningPlan": {
                    "queryPlan": {
                        "stage": "IXSCAN",
                        "indexName": "status_1",
                        "keysExamined": 1200,
                        "nReturned": 12
                    }
                },
                "rejectedPlans": [
                    {
                        "queryPlan": {
                            "stage": "COLLSCAN",
                            "docsExamined": 56000,
                            "nReturned": 12
                        }
                    }
                ]
            },
            "executionStats": {
                "nReturned": 12,
                "totalDocsExamined": 0,
                "totalKeysExamined": 1200,
                "executionTimeMillis": 18
            }
        };

        let parsed = parse_explain_document(&explain);
        assert_eq!(parsed.rejected_plans.len(), 1);
        assert_eq!(parsed.rejected_plans[0].root_stage, "COLLSCAN");
        assert!(!parsed.bottlenecks.is_empty());
        assert_eq!(parsed.bottlenecks[0].rank, 1);
    }
}
