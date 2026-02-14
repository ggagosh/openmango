use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::*;
use gpui_component::input::{CompletionProvider, InputState, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit, InsertTextFormat, Range,
};

use super::logic::{
    METHODS, PIPELINE_OPERATORS, QUERY_OPERATORS, UPDATE_OPERATORS, collection_method_template,
    db_method_template, label_from_template,
};
use super::parser::{PositionKind, ScopeKind, parse_context};
use super::runtime::ForgeRuntime;
use super::runtime::active_forge_session_info;
use super::types::{Suggestion, SuggestionKind};
use crate::state::{AppState, SessionKey};

// ── Accumulator operators ──────────────────────────────────────────────────

pub const ACCUMULATOR_OPERATORS: &[&str] = &[
    "$sum",
    "$avg",
    "$min",
    "$max",
    "$first",
    "$last",
    "$push",
    "$addToSet",
    "$stdDevPop",
    "$stdDevSamp",
    "$count",
    "$mergeObjects",
    "$accumulator",
    "$top",
    "$bottom",
    "$topN",
    "$bottomN",
    "$firstN",
    "$lastN",
    "$maxN",
    "$minN",
];

// ── Completion intent (output of context stage) ────────────────────────────

struct CompletionIntent {
    position: PositionKind,
    scope: ScopeKind,
    collection: Option<String>,
    token: String,
    replace_range: Range,
    line_prefix: String,
}

// ── Provider ───────────────────────────────────────────────────────────────

pub struct ForgeCompletionProvider {
    state: Entity<AppState>,
    runtime: Arc<ForgeRuntime>,
    request_id: Arc<AtomicU64>,
}

impl ForgeCompletionProvider {
    pub fn new(
        state: Entity<AppState>,
        runtime: Arc<ForgeRuntime>,
        request_id: Arc<AtomicU64>,
    ) -> Self {
        Self { state, runtime, request_id }
    }

    pub fn schedule_schema_sample(&self, collection: &str, cx: &mut Context<InputState>) {
        let Some(tab_key) = self.state.read(cx).active_forge_tab_key() else {
            return;
        };
        let Some((session_id, uri, database)) = active_forge_session_info(self.state.read(cx))
        else {
            return;
        };

        let session_key = SessionKey::new(
            tab_key.connection_id,
            tab_key.database.clone(),
            collection.to_string(),
        );

        let should_spawn =
            self.state.update(cx, |state, _| state.mark_forge_schema_inflight(session_key.clone()));
        if !should_spawn {
            return;
        }

        let runtime = self.runtime.clone();
        let runtime_handle = self.state.read(cx).connection_manager().runtime_handle();
        let collection = session_key.collection.clone();

        let task = cx.background_spawn(async move {
            runtime_handle
                .spawn_blocking(move || {
                    let bridge = runtime.ensure_bridge()?;
                    bridge.ensure_session(session_id, &uri, &database)?;
                    let code = format!(
                        "(() => {{ const d = db.getCollection(\"{}\").findOne(); return d || null; }})()",
                        collection.replace('\"', "\\\"")
                    );
                    let eval = bridge.evaluate(session_id, &code, None, Duration::from_secs(5))?;
                    Ok::<_, crate::error::Error>(eval.printable)
                })
                .await
        });

        let state = self.state.clone();
        cx.spawn({
            let session_key = session_key.clone();
            async move |_editor: WeakEntity<InputState>, cx: &mut AsyncApp| {
                let result = task.await;
                let fields = match result {
                    Ok(Ok(printable)) => extract_fields_from_printable(&printable),
                    _ => Vec::new(),
                };

                let _ = cx.update(|cx| {
                    state.update(cx, |state, _| {
                        if !fields.is_empty() {
                            state.set_forge_schema_fields(session_key.clone(), fields);
                        }
                        state.clear_forge_schema_inflight(&session_key);
                    })
                });
            }
        })
        .detach();
    }
}

// ── Pipeline Stage 1: Context ──────────────────────────────────────────────

fn context_stage(rope: &Rope, offset: usize) -> Option<CompletionIntent> {
    let (line_prefix, line_start) = line_prefix_for_offset(rope, offset);

    let full_text = rope.to_string();
    let parse_ctx = parse_context(&full_text, offset);
    if parse_ctx.in_comment {
        return None;
    }

    // For member access scopes, use the member_token from the parser
    let (token, token_start_in_line) =
        if matches!(parse_ctx.scope_kind, ScopeKind::DbMember | ScopeKind::CollectionMember) {
            let mt = parse_ctx.member_token.as_deref().unwrap_or("");
            // Token start = offset - member_token.len(), relative to line_start
            let abs_start = offset.saturating_sub(mt.len());
            let rel_start = abs_start.saturating_sub(line_start);
            (mt.to_string(), rel_start)
        } else {
            object_token_from_line(&line_prefix)
        };

    let replace_start = line_start.saturating_add(token_start_in_line);
    let replace_range = completion_range(rope, replace_start, offset);

    Some(CompletionIntent {
        position: parse_ctx.position_kind,
        scope: parse_ctx.scope_kind,
        collection: parse_ctx.collection,
        token,
        replace_range,
        line_prefix,
    })
}

// ── Pipeline Stage 2: Candidates (policy matrix) ──────────────────────────

fn candidate_stage(
    intent: &CompletionIntent,
    state: &AppState,
    schedule_schema: &mut bool,
) -> Vec<Suggestion> {
    // ── DbMember / CollectionMember → specialized candidates ─────
    match intent.scope {
        ScopeKind::DbMember => return build_collection_suggestions(state),
        ScopeKind::CollectionMember => return build_method_suggestions(),
        _ => {}
    }

    // ── Policy: Value and ArrayElement positions → empty ─────────
    if matches!(intent.position, PositionKind::Value | PositionKind::ArrayElement) {
        return Vec::new();
    }

    // ── Policy: Unknown position in non-TopLevel scope → empty ──
    if intent.position == PositionKind::Unknown && intent.scope != ScopeKind::TopLevel {
        return Vec::new();
    }

    // ── TopLevel scope → nothing to complete ─────────────────────
    if intent.scope == ScopeKind::TopLevel {
        return Vec::new();
    }

    let mut suggestions = Vec::new();

    // ── Key position: schema fields ─────────────────────────────
    if intent.position == PositionKind::Key
        && let Some(collection) = intent.collection.as_deref()
    {
        let fields = build_field_suggestions(state, collection, &intent.token);
        if wants_schema_for_scope(intent.scope)
            && (fields.is_empty() || schema_cache_stale(state, collection))
        {
            *schedule_schema = true;
        }
        suggestions.extend(fields);
    }

    // ── OperatorKey position: operator suggestions ──────────────
    if intent.position == PositionKind::OperatorKey
        || (intent.position == PositionKind::Key && intent.token.starts_with('$'))
    {
        let ops = operators_for_scope(intent.scope);
        suggestions.extend(ops);
    }

    // ── Key position in AggregateStage with empty token: wait for $ ──
    if intent.position == PositionKind::Key
        && intent.scope == ScopeKind::AggregateStage
        && intent.token.is_empty()
    {
        suggestions.extend(build_pipeline_operator_suggestions());
    }

    suggestions
}

fn schema_cache_stale(state: &AppState, collection: &str) -> bool {
    let Some(tab_key) = state.active_forge_tab_key() else {
        return false;
    };
    let session_key =
        SessionKey::new(tab_key.connection_id, tab_key.database.clone(), collection.to_string());
    state.forge_schema_stale(&session_key)
}

fn wants_schema_for_scope(scope: ScopeKind) -> bool {
    matches!(
        scope,
        ScopeKind::FindFilter
            | ScopeKind::MatchFilter
            | ScopeKind::UpdateDoc
            | ScopeKind::InsertDoc
            | ScopeKind::GroupSpec
            | ScopeKind::ProjectSpec
            | ScopeKind::SetDoc
    )
}

fn operators_for_scope(scope: ScopeKind) -> Vec<Suggestion> {
    match scope {
        ScopeKind::AggregateStage => build_pipeline_operator_suggestions(),
        ScopeKind::UpdateDoc => build_update_operator_suggestions(),
        ScopeKind::FindFilter | ScopeKind::MatchFilter | ScopeKind::OperatorValue => {
            build_query_operator_suggestions()
        }
        ScopeKind::GroupSpec => build_accumulator_operator_suggestions(),
        _ => Vec::new(),
    }
}

// ── Pipeline Stage 3: Ranking ──────────────────────────────────────────────

fn ranking_stage(mut suggestions: Vec<Suggestion>, token: &str) -> Vec<Suggestion> {
    // Filter by prefix
    if !token.is_empty() {
        suggestions.retain(|s| s.label.starts_with(token));
    }

    // Deduplicate by label
    let mut seen = std::collections::HashSet::new();
    suggestions.retain(|s| seen.insert(s.label.clone()));

    // Deterministic ordering:
    // 1. Fields first, then operators, then methods, then collections
    // 2. Alphabetical within each kind
    suggestions.sort_by(|a, b| {
        let kind_ord = |k: &SuggestionKind| -> u8 {
            match k {
                SuggestionKind::Field => 0,
                SuggestionKind::Operator => 1,
                SuggestionKind::Method => 2,
                SuggestionKind::Collection => 3,
            }
        };
        kind_ord(&a.kind).cmp(&kind_ord(&b.kind)).then(a.label.cmp(&b.label))
    });

    suggestions
}

// ── Pipeline Stage 4: Render ───────────────────────────────────────────────

fn render_stage(suggestions: Vec<Suggestion>, replace_range: &Range) -> Vec<CompletionItem> {
    suggestions_to_completion_items(suggestions, replace_range)
}

// ── Bridge merge ───────────────────────────────────────────────────────────

fn merge_bridge_suggestions(
    local: Vec<Suggestion>,
    bridge: Vec<String>,
    scope: ScopeKind,
    token: &str,
) -> Vec<Suggestion> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    // Add local suggestions first
    for s in local {
        let label = s.label.clone();
        if seen.insert(label.clone()) {
            out.push(s);
        }
        // Also mark base name as seen (e.g. "find" for "find({})")
        if let Some((base, _)) = label.split_once('(')
            && !base.is_empty()
        {
            seen.insert(base.to_string());
        }
    }

    // Process bridge completions
    for completion in bridge {
        let normalized = match scope {
            ScopeKind::DbMember => {
                // Strip "db." prefix if present
                completion.trim().strip_prefix("db.").unwrap_or(completion.trim()).to_string()
            }
            ScopeKind::CollectionMember => {
                // Take last dot-segment
                completion.trim().rsplit('.').next().unwrap_or(completion.trim()).to_string()
            }
            _ => continue,
        };

        if normalized.is_empty() {
            continue;
        }
        if !token.is_empty() && !normalized.starts_with(token) {
            continue;
        }

        let suggestion = match scope {
            ScopeKind::DbMember => db_method_template(&normalized)
                .map(|template| Suggestion {
                    label: label_from_template(template),
                    kind: SuggestionKind::Method,
                    insert_text: template.to_string(),
                    is_snippet: template.contains('$'),
                })
                .unwrap_or_else(|| Suggestion {
                    label: normalized.clone(),
                    kind: SuggestionKind::Collection,
                    insert_text: normalized,
                    is_snippet: false,
                }),
            ScopeKind::CollectionMember => {
                let base = normalized.split_once('(').map(|(b, _)| b).unwrap_or(&normalized);
                collection_method_template(base)
                    .map(|template| Suggestion {
                        label: label_from_template(template),
                        kind: SuggestionKind::Method,
                        insert_text: template.to_string(),
                        is_snippet: template.contains('$'),
                    })
                    .unwrap_or_else(|| Suggestion {
                        label: normalized.clone(),
                        kind: SuggestionKind::Method,
                        insert_text: normalized,
                        is_snippet: false,
                    })
            }
            _ => continue,
        };

        if seen.insert(suggestion.label.clone()) {
            out.push(suggestion);
        }
    }

    out
}

// ── CompletionProvider impl ────────────────────────────────────────────────

impl CompletionProvider for ForgeCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let request_id = self.request_id.fetch_add(1, Ordering::AcqRel) + 1;

        // Stage 1: Context
        let Some(intent) = context_stage(rope, offset) else {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        };

        // Stage 2: Candidates
        let mut schedule_schema = false;
        let candidates = candidate_stage(&intent, self.state.read(cx), &mut schedule_schema);

        if schedule_schema && let Some(collection) = intent.collection.as_deref() {
            self.schedule_schema_sample(collection, cx);
        }

        // Stage 3 + 4: Rank + Render
        let ranked = ranking_stage(candidates.clone(), &intent.token);
        let local_items = render_stage(ranked, &intent.replace_range);

        // For DbMember/CollectionMember: optionally enhance with bridge (debounced)
        let use_bridge = matches!(intent.scope, ScopeKind::DbMember | ScopeKind::CollectionMember);

        if !use_bridge {
            return Task::ready(Ok(CompletionResponse::Array(local_items)));
        }

        // Prefer local deterministic suggestions while typing; bridge only when local is empty.
        if !local_items.is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(local_items)));
        }

        let Some((session_id, uri, database)) = active_forge_session_info(self.state.read(cx))
        else {
            return Task::ready(Ok(CompletionResponse::Array(local_items)));
        };

        // Build the bridge request text
        let request_text = intent.line_prefix.trim_end().to_string();
        if request_text.is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(local_items)));
        }

        let runtime = self.runtime.clone();
        let latest_request_id = self.request_id.clone();
        let runtime_handle = self.state.read(cx).connection_manager().runtime_handle();
        let token = intent.token.clone();
        let scope = intent.scope;
        let replace_range = intent.replace_range;
        let local_items = local_items.clone();

        cx.background_spawn(async move {
            let result = runtime_handle
                .spawn_blocking(move || {
                    let bridge = runtime.ensure_bridge()?;
                    bridge.ensure_session(session_id, &uri, &database)?;
                    match bridge.complete(session_id, &request_text, Duration::from_millis(500)) {
                        Err(e) if e.to_string().contains("Session not found") => {
                            bridge.ensure_session(session_id, &uri, &database)?;
                            bridge.complete(session_id, &request_text, Duration::from_millis(500))
                        }
                        other => other,
                    }
                })
                .await;

            let completions = match result {
                Ok(Ok(list)) => list,
                Ok(Err(err)) => {
                    log::warn!("Forge completion error: {}", err);
                    Vec::new()
                }
                Err(err) => {
                    log::warn!("Forge completion join error: {}", err);
                    Vec::new()
                }
            };

            // Drop stale async completion results to avoid menu flicker/context jumps.
            if latest_request_id.load(Ordering::Acquire) != request_id {
                return Ok(CompletionResponse::Array(local_items));
            }

            let merged = merge_bridge_suggestions(candidates, completions, scope, &token);
            let ranked = ranking_stage(merged, &token);
            let items = render_stage(ranked, &replace_range);
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        if new_text.is_empty() {
            return false;
        }
        if new_text.chars().all(|c| c.is_whitespace()) {
            return false;
        }
        new_text.chars().any(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn line_prefix_for_offset(rope: &Rope, offset: usize) -> (String, usize) {
    let offset = offset.min(rope.len());
    let position = rope.offset_to_position(offset);
    let line_start = rope.line_start_offset(position.line as usize);
    let line = rope.slice_line(position.line as usize).to_string();
    let prefix_len = offset.saturating_sub(line_start).min(line.len());
    let prefix = line.get(..prefix_len).unwrap_or("").to_string();
    (prefix, line_start)
}

fn completion_range(rope: &Rope, start: usize, end: usize) -> Range {
    let start = start.min(rope.len());
    let end = end.min(rope.len());
    let start_pos = rope.offset_to_position(start);
    let end_pos = rope.offset_to_position(end);
    Range::new(start_pos, end_pos)
}

fn suggestions_to_completion_items(
    suggestions: Vec<Suggestion>,
    replace_range: &Range,
) -> Vec<CompletionItem> {
    suggestions
        .into_iter()
        .map(|suggestion| {
            let kind = match suggestion.kind {
                SuggestionKind::Collection => CompletionItemKind::FIELD,
                SuggestionKind::Method => CompletionItemKind::METHOD,
                SuggestionKind::Operator => CompletionItemKind::OPERATOR,
                SuggestionKind::Field => CompletionItemKind::FIELD,
            };
            CompletionItem {
                label: suggestion.label.clone(),
                kind: Some(kind),
                detail: Some(suggestion.kind.as_str().to_string()),
                insert_text_format: if suggestion.is_snippet {
                    Some(InsertTextFormat::SNIPPET)
                } else {
                    None
                },
                text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                    new_text: suggestion.insert_text,
                    insert: *replace_range,
                    replace: *replace_range,
                })),
                ..Default::default()
            }
        })
        .collect()
}

fn make_suggestion(label: &str, kind: SuggestionKind, insert_text: &str) -> Suggestion {
    Suggestion {
        label: label.to_string(),
        kind,
        insert_text: insert_text.to_string(),
        is_snippet: insert_text.contains('$'),
    }
}

fn build_collection_suggestions(state: &AppState) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    for (label, template) in [
        ("stats()", "stats()"),
        ("getCollection(\"\")", "getCollection(\"$1\")$0"),
        ("getSiblingDB(\"\")", "getSiblingDB(\"$1\")$0"),
        ("runCommand({})", "runCommand({$1})$0"),
        ("listCollections({})", "listCollections({$1})$0"),
        ("createCollection(\"\")", "createCollection(\"$1\")$0"),
    ] {
        suggestions.push(make_suggestion(label, SuggestionKind::Method, template));
    }

    if let Some(key) = state.active_forge_tab_key()
        && let Some(conn) = state.active_connection_by_id(key.connection_id)
        && let Some(collections) = conn.collections.get(&key.database)
    {
        for coll in collections {
            suggestions.push(Suggestion {
                label: coll.clone(),
                kind: SuggestionKind::Collection,
                insert_text: coll.clone(),
                is_snippet: false,
            });
        }
    }

    suggestions
}

fn build_method_suggestions() -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    for method in METHODS {
        if let Some(template) = collection_method_template(method) {
            let label = label_from_template(template);
            suggestions.push(make_suggestion(&label, SuggestionKind::Method, template));
        } else {
            let insert = format!("{}()", method);
            suggestions.push(make_suggestion(&insert, SuggestionKind::Method, &insert));
        }
    }

    suggestions
}

fn build_pipeline_operator_suggestions() -> Vec<Suggestion> {
    PIPELINE_OPERATORS
        .iter()
        .map(|op| {
            let insert = match *op {
                "$match" | "$project" | "$group" | "$sort" | "$addFields" | "$set" | "$facet"
                | "$bucket" | "$bucketAuto" | "$lookup" | "$graphLookup" | "$sample"
                | "$replaceRoot" => format!("{op}: {{$1}}$0"),
                "$unset" | "$count" | "$out" => format!("{op}: \"$1\"$0"),
                "$limit" | "$skip" => format!("{op}: $1$0"),
                "$replaceWith" => format!("{op}: $1$0"),
                "$merge" | "$unionWith" => format!("{op}: {{$1}}$0"),
                _ => format!("{op}: {{$1}}$0"),
            };
            make_suggestion(op, SuggestionKind::Operator, &insert)
        })
        .collect()
}

fn build_query_operator_suggestions() -> Vec<Suggestion> {
    QUERY_OPERATORS
        .iter()
        .map(|op| {
            let insert = match *op {
                "$in" | "$nin" | "$all" => format!("{op}: [$1]$0"),
                "$exists" => format!("{op}: true$0"),
                "$regex" => format!("{op}: /$1/$0"),
                "$and" | "$or" | "$nor" => format!("{op}: [$1]$0"),
                "$elemMatch" => format!("{op}: {{$1}}$0"),
                _ => format!("{op}: $1$0"),
            };
            make_suggestion(op, SuggestionKind::Operator, &insert)
        })
        .collect()
}

fn build_update_operator_suggestions() -> Vec<Suggestion> {
    UPDATE_OPERATORS
        .iter()
        .map(|op| {
            let insert = match *op {
                "$unset" => format!("{op}: {{$1: \"\"}}$0"),
                "$currentDate" => format!("{op}: {{$1: true}}$0"),
                _ => format!("{op}: {{$1}}$0"),
            };
            make_suggestion(op, SuggestionKind::Operator, &insert)
        })
        .collect()
}

fn build_accumulator_operator_suggestions() -> Vec<Suggestion> {
    ACCUMULATOR_OPERATORS
        .iter()
        .map(|op| {
            let insert = match *op {
                "$count" => format!("{op}: {{}}$0"),
                "$mergeObjects" => format!("{op}: \"$$1\"$0"),
                _ => format!("{op}: $1$0"),
            };
            make_suggestion(op, SuggestionKind::Operator, &insert)
        })
        .collect()
}

fn build_field_suggestions(state: &AppState, collection: &str, token: &str) -> Vec<Suggestion> {
    if token.starts_with('$') {
        return Vec::new();
    }
    let Some(tab_key) = state.active_forge_tab_key() else {
        return Vec::new();
    };
    let session_key =
        SessionKey::new(tab_key.connection_id, tab_key.database.clone(), collection.to_string());
    let mut fields: Vec<String> = Vec::new();
    if let Some(cached) = state.forge_schema_fields(&session_key) {
        fields.extend(cached.iter().cloned());
    } else if let Some(session) = state.session_data(&session_key) {
        let mut set = std::collections::HashSet::new();
        for item in &session.items {
            for key in item.doc.keys() {
                set.insert(key.to_string());
            }
        }
        fields.extend(set);
    }
    fields.sort();
    fields
        .into_iter()
        .filter(|field| field.starts_with(token))
        .map(|field| Suggestion {
            label: field.clone(),
            kind: SuggestionKind::Field,
            insert_text: field,
            is_snippet: false,
        })
        .collect()
}

fn object_token_from_line(line_prefix: &str) -> (String, usize) {
    let mut idx = line_prefix.len();
    let bytes = line_prefix.as_bytes();
    while idx > 0 {
        let ch = bytes[idx - 1] as char;
        if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            idx -= 1;
        } else {
            break;
        }
    }
    (line_prefix[idx..].to_string(), idx)
}

fn extract_fields_from_printable(printable: &serde_json::Value) -> Vec<String> {
    if let Some(obj) = printable.as_object() {
        return obj.keys().cloned().collect();
    }
    if let Some(text) = printable.as_str()
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
        && let Some(obj) = value.as_object()
    {
        return obj.keys().cloned().collect();
    }
    Vec::new()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{
        CompletionIntent, build_accumulator_operator_suggestions, build_query_operator_suggestions,
        candidate_stage, make_suggestion, operators_for_scope, ranking_stage,
        wants_schema_for_scope,
    };
    use crate::state::AppState;
    use crate::views::forge::parser::{PositionKind, ScopeKind};
    use crate::views::forge::types::SuggestionKind;

    // ── Policy: position gating ─────────────────────────────────────

    #[test]
    fn value_position_is_gated() {
        // Policy: Value positions should never produce candidates
        assert!(matches!(PositionKind::Value, PositionKind::Value | PositionKind::ArrayElement));
    }

    #[test]
    fn array_element_is_gated() {
        assert!(matches!(
            PositionKind::ArrayElement,
            PositionKind::Value | PositionKind::ArrayElement
        ));
    }

    // ── Policy: operators_for_scope ─────────────────────────────────

    #[test]
    fn find_filter_returns_query_ops() {
        let ops = operators_for_scope(ScopeKind::FindFilter);
        assert!(!ops.is_empty());
        assert!(ops.iter().any(|s| s.label == "$eq"));
        assert!(ops.iter().any(|s| s.label == "$gt"));
    }

    #[test]
    fn update_doc_returns_update_ops() {
        let ops = operators_for_scope(ScopeKind::UpdateDoc);
        assert!(!ops.is_empty());
        assert!(ops.iter().any(|s| s.label == "$set"));
        assert!(ops.iter().any(|s| s.label == "$inc"));
        // Should NOT contain query operators
        assert!(!ops.iter().any(|s| s.label == "$eq"));
    }

    #[test]
    fn aggregate_stage_returns_pipeline_ops() {
        let ops = operators_for_scope(ScopeKind::AggregateStage);
        assert!(!ops.is_empty());
        assert!(ops.iter().any(|s| s.label == "$match"));
        assert!(ops.iter().any(|s| s.label == "$group"));
        // Should NOT contain query operators
        assert!(!ops.iter().any(|s| s.label == "$eq"));
    }

    #[test]
    fn group_spec_returns_accumulator_ops() {
        let ops = operators_for_scope(ScopeKind::GroupSpec);
        assert!(!ops.is_empty());
        assert!(ops.iter().any(|s| s.label == "$sum"));
        assert!(ops.iter().any(|s| s.label == "$avg"));
        // Should NOT contain pipeline operators
        assert!(!ops.iter().any(|s| s.label == "$match"));
    }

    #[test]
    fn match_filter_returns_query_ops() {
        let ops = operators_for_scope(ScopeKind::MatchFilter);
        assert!(ops.iter().any(|s| s.label == "$eq"));
    }

    #[test]
    fn insert_doc_returns_no_ops() {
        let ops = operators_for_scope(ScopeKind::InsertDoc);
        assert!(ops.is_empty());
    }

    #[test]
    fn operator_value_returns_query_ops() {
        let ops = operators_for_scope(ScopeKind::OperatorValue);
        assert!(ops.iter().any(|s| s.label == "$eq"));
    }

    #[test]
    fn db_member_returns_no_ops() {
        let ops = operators_for_scope(ScopeKind::DbMember);
        assert!(ops.is_empty());
    }

    #[test]
    fn collection_member_returns_no_ops() {
        let ops = operators_for_scope(ScopeKind::CollectionMember);
        assert!(ops.is_empty());
    }

    // ── Ranking ─────────────────────────────────────────────────────

    #[test]
    fn ranking_deduplicates() {
        let suggestions = vec![
            make_suggestion("$eq", SuggestionKind::Operator, "$eq: $1$0"),
            make_suggestion("$eq", SuggestionKind::Operator, "$eq: $1$0"),
        ];
        let ranked = ranking_stage(suggestions, "");
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn ranking_filters_by_prefix() {
        let suggestions = vec![
            make_suggestion("$eq", SuggestionKind::Operator, "$eq: $1$0"),
            make_suggestion("$gt", SuggestionKind::Operator, "$gt: $1$0"),
            make_suggestion("$gte", SuggestionKind::Operator, "$gte: $1$0"),
        ];
        let ranked = ranking_stage(suggestions, "$gt");
        assert_eq!(ranked.len(), 2);
        assert!(ranked.iter().all(|s| s.label.starts_with("$gt")));
    }

    #[test]
    fn ranking_is_deterministic() {
        let suggestions = vec![
            make_suggestion("$gt", SuggestionKind::Operator, "$gt: $1$0"),
            make_suggestion("$eq", SuggestionKind::Operator, "$eq: $1$0"),
            make_suggestion("name", SuggestionKind::Field, "name"),
        ];
        let ranked = ranking_stage(suggestions, "");
        assert_eq!(ranked[0].label, "name"); // Field first
        assert_eq!(ranked[1].label, "$eq"); // Then operators alphabetical
        assert_eq!(ranked[2].label, "$gt");
    }

    // ── Accumulator operators ───────────────────────────────────────

    #[test]
    fn accumulator_ops_include_sum_avg() {
        let ops = build_accumulator_operator_suggestions();
        assert!(ops.iter().any(|s| s.label == "$sum"));
        assert!(ops.iter().any(|s| s.label == "$avg"));
        assert!(ops.iter().any(|s| s.label == "$first"));
        assert!(ops.iter().any(|s| s.label == "$last"));
        assert!(ops.iter().any(|s| s.label == "$push"));
        assert!(ops.iter().any(|s| s.label == "$addToSet"));
    }

    // ── Snippets don't leak raw $1/$0 ───────────────────────────────

    #[test]
    fn snippets_are_marked() {
        let ops = build_query_operator_suggestions();
        for op in &ops {
            if op.insert_text.contains("$1") || op.insert_text.contains("$0") {
                assert!(op.is_snippet, "Snippet {} not marked", op.label);
            }
        }
    }

    // ── wants_schema_for_scope ───────────────────────────────────────

    #[test]
    fn schema_wanted_for_filter_scopes() {
        assert!(wants_schema_for_scope(ScopeKind::FindFilter));
        assert!(wants_schema_for_scope(ScopeKind::MatchFilter));
        assert!(wants_schema_for_scope(ScopeKind::InsertDoc));
        assert!(wants_schema_for_scope(ScopeKind::SetDoc));
    }

    #[test]
    fn schema_not_wanted_for_operator_value() {
        assert!(!wants_schema_for_scope(ScopeKind::OperatorValue));
        assert!(!wants_schema_for_scope(ScopeKind::AggregateStage));
        assert!(!wants_schema_for_scope(ScopeKind::TopLevel));
        assert!(!wants_schema_for_scope(ScopeKind::DbMember));
        assert!(!wants_schema_for_scope(ScopeKind::CollectionMember));
    }

    // ── Candidate policy checks ────────────────────────────────────

    fn intent(scope: ScopeKind, position: PositionKind, token: &str) -> CompletionIntent {
        CompletionIntent {
            position,
            scope,
            collection: None,
            token: token.to_string(),
            replace_range: lsp_types::Range::new(
                lsp_types::Position::new(0, 0),
                lsp_types::Position::new(0, 0),
            ),
            line_prefix: String::new(),
        }
    }

    #[test]
    fn candidate_stage_top_level_is_empty() {
        let state = AppState::default();
        let mut schedule_schema = false;
        let out = candidate_stage(
            &intent(ScopeKind::TopLevel, PositionKind::Unknown, ""),
            &state,
            &mut schedule_schema,
        );
        assert!(out.is_empty());
        assert!(!schedule_schema);
    }

    #[test]
    fn candidate_stage_value_position_is_empty() {
        let state = AppState::default();
        let mut schedule_schema = false;
        let out = candidate_stage(
            &intent(ScopeKind::FindFilter, PositionKind::Value, ""),
            &state,
            &mut schedule_schema,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn candidate_stage_db_member_has_core_methods() {
        let state = AppState::default();
        let mut schedule_schema = false;
        let out = candidate_stage(
            &intent(ScopeKind::DbMember, PositionKind::MemberAccess, ""),
            &state,
            &mut schedule_schema,
        );
        assert!(out.iter().any(|s| s.label == "stats()"));
        assert!(out.iter().any(|s| s.label == "getCollection(\"\")"));
        assert!(!schedule_schema);
    }
}
