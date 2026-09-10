use std::collections::{HashMap, HashSet};

use gpui_kit::component::input::{CompletionProvider, Rope, RopeExt};
use gpui_kit::*;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit, InsertTextFormat, Range,
};
use mongodb::bson::{Bson, Document};

use crate::app::search::ranked_match_score;
use crate::state::{AppCommands, AppState, SchemaField, SessionKey};
use crate::views::forge::parser::{PositionKind, ScopeKind, parse_context};

use super::fast_filter::{document_id_input, is_date_field};
use crate::views::forge::logic::{cursor_from_template, label_from_template};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryInputKind {
    Filter,
    Sort,
    Projection,
}

#[derive(Clone, Copy)]
struct Operator {
    label: &'static str,
    snippet: &'static str,
    detail: &'static str,
}

#[derive(Clone, Copy)]
struct ValueLiteral {
    label: &'static str,
    snippet: &'static str,
    detail: &'static str,
}

#[derive(Clone, Debug, Default)]
struct QueryEditorContext {
    raw_text: String,
    position_kind: PositionKind,
    scope_kind: ScopeKind,
    token: String,
    replace_range: Range,
    in_string_or_comment: bool,
    session_key: Option<SessionKey>,
    value: Option<super::query_values::ValueContext>,
    field: Option<super::query_values::FieldContext>,
}

#[derive(Clone, Debug)]
struct FieldCandidate {
    path: String,
    depth: usize,
    presence: u64,
    sampled_count: u64,
    type_label: String,
}

const BSON_CONSTRUCTORS: &[Operator] = &[
    Operator { label: "ObjectId", snippet: "ObjectId(\"$1\")$0", detail: "MongoDB ObjectId" },
    Operator { label: "ISODate", snippet: "ISODate(\"$1\")$0", detail: "ISO 8601 date" },
    Operator { label: "Date", snippet: "Date(\"$1\")$0", detail: "Date alias" },
    Operator { label: "NumberLong", snippet: "NumberLong($1)$0", detail: "64-bit integer" },
    Operator { label: "NumberInt", snippet: "NumberInt($1)$0", detail: "32-bit integer" },
    Operator {
        label: "NumberDecimal",
        snippet: "NumberDecimal(\"$1\")$0",
        detail: "128-bit decimal",
    },
    Operator { label: "NumberDouble", snippet: "NumberDouble($1)$0", detail: "64-bit float" },
    Operator { label: "UUID", snippet: "UUID(\"$1\")$0", detail: "UUID value" },
    Operator { label: "Timestamp", snippet: "Timestamp($1, $2)$0", detail: "BSON timestamp" },
];

const FILTER_OPERATORS: &[Operator] = &[
    Operator { label: "$eq", snippet: "$eq: $1$0", detail: "Equals" },
    Operator { label: "$ne", snippet: "$ne: $1$0", detail: "Not equal" },
    Operator { label: "$gt", snippet: "$gt: $1$0", detail: "Greater than" },
    Operator { label: "$gte", snippet: "$gte: $1$0", detail: "Greater than or equal" },
    Operator { label: "$lt", snippet: "$lt: $1$0", detail: "Less than" },
    Operator { label: "$lte", snippet: "$lte: $1$0", detail: "Less than or equal" },
    Operator { label: "$in", snippet: "$in: [$1]$0", detail: "Matches any value in array" },
    Operator { label: "$nin", snippet: "$nin: [$1]$0", detail: "Matches values not in array" },
    Operator { label: "$exists", snippet: "$exists: true$0", detail: "Field exists" },
    Operator { label: "$type", snippet: "$type: \"$1\"$0", detail: "BSON type" },
    Operator { label: "$regex", snippet: "$regex: /$1/$0", detail: "Regular expression" },
    Operator { label: "$not", snippet: "$not: {$1}$0", detail: "Logical NOT" },
    Operator { label: "$and", snippet: "$and: [{$1}]$0", detail: "Logical AND" },
    Operator { label: "$or", snippet: "$or: [{$1}]$0", detail: "Logical OR" },
    Operator { label: "$nor", snippet: "$nor: [{$1}]$0", detail: "Logical NOR" },
    Operator { label: "$elemMatch", snippet: "$elemMatch: {$1}$0", detail: "Array element match" },
    Operator { label: "$all", snippet: "$all: [$1]$0", detail: "All elements match" },
    Operator { label: "$size", snippet: "$size: $1$0", detail: "Array size" },
];

const FILTER_VALUE_LITERALS: &[ValueLiteral] = &[
    ValueLiteral { label: "true", snippet: "true", detail: "Boolean true" },
    ValueLiteral { label: "false", snippet: "false", detail: "Boolean false" },
    ValueLiteral { label: "null", snippet: "null", detail: "Null value" },
    ValueLiteral { label: "[]", snippet: "[$1]$0", detail: "Array literal" },
    ValueLiteral { label: "{}", snippet: "{$1}$0", detail: "Document literal" },
];

const DATE_SHORTCUT_LITERALS: &[ValueLiteral] = &[
    ValueLiteral { label: "today", snippet: "today", detail: "UTC day range" },
    ValueLiteral { label: "yesterday", snippet: "yesterday", detail: "Previous UTC day" },
    ValueLiteral { label: "tomorrow", snippet: "tomorrow", detail: "Next UTC day" },
    ValueLiteral { label: "now", snippet: "now", detail: "Current instant" },
    ValueLiteral { label: "thisweek", snippet: "thisweek", detail: "Current UTC week" },
    ValueLiteral { label: "lastweek", snippet: "lastweek", detail: "Previous UTC week" },
    ValueLiteral { label: "thismonth", snippet: "thismonth", detail: "Current UTC month" },
    ValueLiteral { label: "lastmonth", snippet: "lastmonth", detail: "Previous UTC month" },
    ValueLiteral { label: "thisyear", snippet: "thisyear", detail: "Current UTC year" },
    ValueLiteral { label: "lastyear", snippet: "lastyear", detail: "Previous UTC year" },
    ValueLiteral { label: "last24h", snippet: "last24h", detail: "Rolling 24 hours" },
    ValueLiteral { label: "last7d", snippet: "last7d", detail: "Rolling 7 days" },
    ValueLiteral { label: "last30d", snippet: "last30d", detail: "Rolling 30 days" },
    ValueLiteral { label: "next7d", snippet: "next7d", detail: "Next 7 days" },
    ValueLiteral { label: "wtd", snippet: "wtd", detail: "Week to date" },
    ValueLiteral { label: "mtd", snippet: "mtd", detail: "Month to date" },
    ValueLiteral { label: "ytd", snippet: "ytd", detail: "Year to date" },
    ValueLiteral { label: "YYYY-MM-DD", snippet: "2026-05-23", detail: "Specific UTC day" },
    ValueLiteral { label: "YYYY-MM", snippet: "2026-05", detail: "Specific UTC month" },
    ValueLiteral { label: "YYYYQ1", snippet: "2026Q1", detail: "Specific UTC quarter" },
];

const SORT_VALUE_LITERALS: &[ValueLiteral] = &[
    ValueLiteral { label: "1", snippet: "1", detail: "Ascending sort" },
    ValueLiteral { label: "-1", snippet: "-1", detail: "Descending sort" },
];

const PROJECTION_VALUE_LITERALS: &[ValueLiteral] = &[
    ValueLiteral { label: "1", snippet: "1", detail: "Include field" },
    ValueLiteral { label: "0", snippet: "0", detail: "Exclude field" },
    ValueLiteral { label: "true", snippet: "true", detail: "Include field" },
    ValueLiteral { label: "false", snippet: "false", detail: "Exclude field" },
];

#[derive(Clone)]
pub struct QueryCompletionProvider {
    state: Entity<AppState>,
    kind: QueryInputKind,
}

impl QueryCompletionProvider {
    pub fn new(state: Entity<AppState>, kind: QueryInputKind) -> Self {
        Self { state, kind }
    }

    pub(crate) fn items(&self, rope: &Rope, offset: usize, cx: &mut App) -> Vec<CompletionItem> {
        let ctx = self.context(rope, offset, cx);
        match self.kind {
            QueryInputKind::Filter => self.filter_items(&ctx, cx),
            QueryInputKind::Sort | QueryInputKind::Projection => {
                self.sort_or_projection_items(&ctx, cx)
            }
        }
    }

    fn current_session_key(&self, cx: &App) -> Option<SessionKey> {
        self.state.read(cx).current_session_key()
    }

    fn context(&self, rope: &Rope, offset: usize, cx: &mut App) -> QueryEditorContext {
        let text = rope.to_string();
        let fast_filter = self.kind == QueryInputKind::Filter && is_fast_filter_text(&text);
        let (token_start, mut token) =
            if fast_filter { fast_filter_token(&text, offset) } else { query_token(&text, offset) };
        let mut replace_range = Range {
            start: rope.offset_to_position(token_start),
            end: rope.offset_to_position(offset.min(rope.len())),
        };

        let Some(session_key) = self.current_session_key(cx) else {
            return QueryEditorContext {
                raw_text: text,
                token,
                replace_range,
                ..Default::default()
            };
        };

        let (wrapped, wrapped_cursor) =
            wrap_query_input(self.kind, &session_key.collection, &text, offset.min(text.len()));
        let parsed = parse_context(&wrapped, wrapped_cursor);
        let field = (self.kind == QueryInputKind::Filter && !fast_filter)
            .then(|| super::query_values::field_context(&text, offset))
            .flatten();
        if let Some(field) = &field {
            token = field.prefix.clone();
            replace_range = Range {
                start: rope.offset_to_position(field.range.start),
                end: rope.offset_to_position(field.range.end),
            };
        }

        QueryEditorContext {
            value: (self.kind == QueryInputKind::Filter && !fast_filter)
                .then(|| super::query_values::value_context(&text, offset))
                .flatten(),
            raw_text: text,
            position_kind: if field.is_some() { PositionKind::Key } else { parsed.position_kind },
            scope_kind: parsed.scope_kind,
            token,
            replace_range,
            in_string_or_comment: parsed.in_comment && field.is_none(),
            session_key: Some(session_key),
            field,
        }
    }

    fn field_candidates(&self, session_key: &SessionKey, cx: &mut App) -> Vec<FieldCandidate> {
        let mut fields: HashMap<String, FieldCandidate> = HashMap::new();
        let should_fetch = {
            let state_ref = self.state.read(cx);

            if let Some(session) = state_ref.session(session_key)
                && let Some(schema) = session.data.schema.as_ref()
            {
                collect_schema_candidates(&schema.fields, &mut fields);
            }

            if let Some(cache) = state_ref.collection_meta(session_key) {
                collect_schema_candidates(&cache.schema.fields, &mut fields);
            }

            if let Some(session_data) = state_ref.session_data(session_key) {
                for item in session_data.items.iter().take(100) {
                    collect_document_path_counts(&item.doc, "", 0, &mut fields);
                }
            }

            state_ref.collection_meta_stale(session_key)
                && !state_ref.is_collection_meta_inflight(session_key)
        };

        if should_fetch {
            AppCommands::fetch_single_collection_meta(self.state.clone(), session_key.clone(), cx);
        }

        let mut ordered: Vec<FieldCandidate> = fields.into_values().collect();
        ordered.sort_unstable_by(compare_field_candidates);
        ordered
    }

    fn filter_items(&self, ctx: &QueryEditorContext, cx: &mut App) -> Vec<CompletionItem> {
        if let Some(id) = document_id_input(&ctx.raw_text) {
            let range = Range {
                start: lsp_types::Position::new(0, 0),
                end: Rope::from(ctx.raw_text.as_str()).offset_to_position(ctx.raw_text.len()),
            };
            let query = crate::bson::format_relaxed_json_compact(
                &Bson::Document(mongodb::bson::doc! { "_id": id.clone() }).into_relaxed_extjson(),
            );
            let mut items = vec![completion_item(
                query.clone(),
                CompletionItemKind::VALUE,
                format!("{} · exact _id match", crate::bson::bson_type_label(&id)),
                query,
                false,
                range,
            )];
            if matches!(id, Bson::ObjectId(_)) && ctx.raw_text.trim().len() == 24 {
                let query = format!(
                    "{{ _id: {} }}",
                    serde_json::to_string(ctx.raw_text.trim()).unwrap_or_default()
                );
                items.push(completion_item(
                    query.clone(),
                    CompletionItemKind::VALUE,
                    "String · exact _id match",
                    query,
                    false,
                    range,
                ));
            }
            return items;
        }
        if let Some(value) = &ctx.value {
            return self.value_items(ctx, value, cx);
        }
        if ctx.in_string_or_comment {
            return Vec::new();
        }

        if is_fast_filter_text(&ctx.raw_text) {
            return self.fast_filter_items(ctx, cx);
        }

        let mut items = Vec::new();

        if matches!(
            ctx.position_kind,
            PositionKind::Key | PositionKind::Unknown | PositionKind::MemberAccess
        ) && !ctx.token.starts_with('$')
            && let Some(session_key) = ctx.session_key.as_ref()
        {
            let parent =
                ctx.field.as_ref().map(|field| field.parent_path.as_str()).unwrap_or_default();
            let fields = self
                .field_candidates(session_key, cx)
                .into_iter()
                .filter_map(|mut field| {
                    if !parent.is_empty() {
                        field.path = field.path.strip_prefix(&format!("{parent}."))?.to_string();
                    }
                    Some(field)
                })
                .collect();
            for field in filter_field_candidates(fields, &ctx.token) {
                let key = if ctx.field.as_ref().is_some_and(|field| field.quoted) {
                    serde_json::to_string(&field.path).unwrap_or_default()
                } else {
                    format_query_key(&field.path)
                };
                let text = if ctx.field.as_ref().is_some_and(|field| field.has_colon) {
                    key
                } else {
                    format!("{key}: ")
                };
                items.push(completion_item(
                    field.path,
                    CompletionItemKind::FIELD,
                    field.type_label,
                    text,
                    false,
                    ctx.replace_range,
                ));
            }
        }

        if ctx.position_kind == PositionKind::OperatorKey
            || (ctx.position_kind == PositionKind::Key && ctx.token.starts_with('$'))
            || matches!(
                ctx.scope_kind,
                ScopeKind::FindFilter | ScopeKind::MatchFilter | ScopeKind::OperatorValue
            ) && ctx.token.starts_with('$')
        {
            let token_lower = ctx.token.to_ascii_lowercase();
            for op in FILTER_OPERATORS {
                if !token_lower.is_empty()
                    && !op.label.to_ascii_lowercase().starts_with(&token_lower)
                {
                    continue;
                }
                let replace_key = ctx.field.as_ref().is_some_and(|field| field.has_colon);
                items.push(completion_item(
                    op.label,
                    CompletionItemKind::OPERATOR,
                    op.detail,
                    if replace_key {
                        if ctx.field.as_ref().is_some_and(|field| field.quoted) {
                            serde_json::to_string(op.label).unwrap_or_default()
                        } else {
                            op.label.to_string()
                        }
                    } else {
                        op.snippet.to_string()
                    },
                    !replace_key,
                    ctx.replace_range,
                ));
            }
        }

        if matches!(ctx.position_kind, PositionKind::Value | PositionKind::ArrayElement)
            || matches!(ctx.scope_kind, ScopeKind::OperatorValue)
        {
            let token_lower = ctx.token.to_ascii_lowercase();
            for constructor in BSON_CONSTRUCTORS {
                if !token_lower.is_empty()
                    && !constructor.label.to_ascii_lowercase().starts_with(&token_lower)
                {
                    continue;
                }
                items.push(completion_item(
                    constructor.label,
                    CompletionItemKind::CONSTRUCTOR,
                    constructor.detail,
                    constructor.snippet,
                    true,
                    ctx.replace_range,
                ));
            }

            for literal in FILTER_VALUE_LITERALS {
                if !token_lower.is_empty() && !literal.label.starts_with(&ctx.token) {
                    continue;
                }
                items.push(completion_item(
                    literal.label,
                    CompletionItemKind::VALUE,
                    literal.detail,
                    literal.snippet,
                    literal.snippet.contains('$'),
                    ctx.replace_range,
                ));
            }
        }

        rank_and_dedupe(items)
    }

    fn value_items(
        &self,
        ctx: &QueryEditorContext,
        value: &super::query_values::ValueContext,
        cx: &App,
    ) -> Vec<CompletionItem> {
        let samples = ctx
            .session_key
            .as_ref()
            .and_then(|key| self.state.read(cx).session_data(key))
            .map(|data| {
                super::query_values::sampled_values(
                    data.items.iter().map(|item| &item.doc),
                    &value.field,
                )
            })
            .unwrap_or_default();
        value_completion_items(&ctx.raw_text, value, &samples)
    }

    fn fast_filter_items(&self, ctx: &QueryEditorContext, cx: &mut App) -> Vec<CompletionItem> {
        let Some(session_key) = ctx.session_key.as_ref() else {
            return Vec::new();
        };

        let token = ctx.token.trim();
        let field_token = token.strip_prefix('!').unwrap_or(token);
        if let Some((field, operator, value_prefix)) = split_fast_value_token(field_token) {
            if field.trim().is_empty() {
                return Vec::new();
            }
            let prefix = if token.starts_with('!') { "!" } else { "" };
            return fast_filter_value_items(
                ctx,
                &format!("{prefix}{field}{operator}"),
                field,
                value_prefix,
            );
        }

        let mut items = Vec::new();
        for field in filter_field_candidates(self.field_candidates(session_key, cx), field_token) {
            let text = if token.starts_with('!') {
                format!("!{}", field.path)
            } else if ctx.raw_text.trim() == token {
                format!("{{ {}:  }}", format_query_key(&field.path))
            } else {
                format!("{}:", field.path)
            };
            let cursor = if text.ends_with("  }") { text.len() - 2 } else { text.len() };
            let mut item = completion_item(
                field.path,
                CompletionItemKind::FIELD,
                field.type_label,
                text,
                false,
                ctx.replace_range,
            );
            item.data = Some(serde_json::json!({ "cursor_offset": cursor }));
            items.push(item);
        }

        rank_and_dedupe(items)
    }

    fn sort_or_projection_items(
        &self,
        ctx: &QueryEditorContext,
        cx: &mut App,
    ) -> Vec<CompletionItem> {
        let Some(session_key) = ctx.session_key.as_ref() else {
            return Vec::new();
        };

        let mut items = Vec::new();

        if !ctx.in_string_or_comment
            && matches!(ctx.position_kind, PositionKind::Key | PositionKind::Unknown)
        {
            for field in filter_field_candidates(self.field_candidates(session_key, cx), &ctx.token)
            {
                let key = format_query_key(&field.path);
                let default_value = match self.kind {
                    QueryInputKind::Sort => "1",
                    QueryInputKind::Projection => "1",
                    QueryInputKind::Filter => unreachable!(),
                };
                items.push(completion_item(
                    field.path,
                    CompletionItemKind::FIELD,
                    field.type_label,
                    format!("{key}: {default_value}$0"),
                    true,
                    ctx.replace_range,
                ));
            }
        }

        if !ctx.in_string_or_comment
            && matches!(ctx.position_kind, PositionKind::Value | PositionKind::Unknown)
        {
            let token_lower = ctx.token.to_ascii_lowercase();
            let literals = match self.kind {
                QueryInputKind::Sort => SORT_VALUE_LITERALS,
                QueryInputKind::Projection => PROJECTION_VALUE_LITERALS,
                QueryInputKind::Filter => &[],
            };
            for literal in literals {
                if !token_lower.is_empty() && !literal.label.starts_with(&ctx.token) {
                    continue;
                }
                items.push(completion_item(
                    literal.label,
                    CompletionItemKind::VALUE,
                    literal.detail,
                    literal.snippet,
                    literal.snippet.contains('$'),
                    ctx.replace_range,
                ));
            }
        }

        rank_and_dedupe(items)
    }
}

impl CompletionProvider for QueryCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let items = self.items(rope, offset, cx);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        if new_text.is_empty() || new_text.chars().all(char::is_whitespace) {
            return false;
        }
        new_text.chars().any(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '$' | '.' | ':' | '>' | '<' | '=' | '~' | '!')
        })
    }
}

pub fn query_input_in_string_or_comment(
    kind: QueryInputKind,
    collection: &str,
    text: &str,
    cursor: usize,
) -> bool {
    let (wrapped, wrapped_cursor) =
        wrap_query_input(kind, collection, text, cursor.min(text.len()));
    parse_context(&wrapped, wrapped_cursor).in_comment
}

pub fn is_query_input_in_string_or_comment(text: &str, cursor: usize) -> bool {
    query_input_in_string_or_comment(QueryInputKind::Filter, "__filter__", text, cursor)
}

fn wrap_query_input(
    kind: QueryInputKind,
    collection: &str,
    text: &str,
    cursor: usize,
) -> (String, usize) {
    let escaped_collection = collection.replace('\\', "\\\\").replace('"', "\\\"");

    let prefix = match kind {
        QueryInputKind::Filter => format!("db.getCollection(\"{escaped_collection}\").find("),
        QueryInputKind::Sort => {
            format!("db.getCollection(\"{escaped_collection}\").find({{}}).sort(")
        }
        QueryInputKind::Projection => {
            format!("db.getCollection(\"{escaped_collection}\").find({{}}, ")
        }
    };
    let wrapped = format!("{prefix}{text})");
    (wrapped, prefix.len() + cursor)
}

fn collect_schema_candidates(fields: &[SchemaField], out: &mut HashMap<String, FieldCandidate>) {
    for field in fields {
        let normalized = normalize_query_path(&field.path);
        if !normalized.is_empty() {
            let depth = normalized.matches('.').count();
            out.entry(normalized.clone())
                .and_modify(|candidate| {
                    candidate.presence = candidate.presence.max(field.presence);
                })
                .or_insert(FieldCandidate {
                    path: normalized,
                    depth,
                    presence: field.presence,
                    sampled_count: 0,
                    type_label: field
                        .types
                        .iter()
                        .map(|kind| kind.bson_type.as_str())
                        .collect::<Vec<_>>()
                        .join(" / "),
                });
        }
        collect_schema_candidates(&field.children, out);
    }
}

fn collect_document_path_counts(
    doc: &Document,
    prefix: &str,
    depth: usize,
    out: &mut HashMap<String, FieldCandidate>,
) {
    if depth > 4 {
        return;
    }

    for (key, value) in doc {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        let entry = out.entry(path.clone()).or_insert(FieldCandidate {
            path: path.clone(),
            depth: path.matches('.').count(),
            presence: 0,
            sampled_count: 0,
            type_label: crate::bson::bson_type_label(value).to_string(),
        });
        entry.sampled_count += 1;

        match value {
            Bson::Document(nested) => collect_document_path_counts(nested, &path, depth + 1, out),
            Bson::Array(items) => {
                for item in items.iter().take(8) {
                    if let Bson::Document(nested) = item {
                        collect_document_path_counts(nested, &path, depth + 1, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn normalize_query_path(path: &str) -> String {
    let normalized = path.replace(".[*].", ".").replace(".[*]", "").replace("[*].", "");
    normalized.trim_matches('.').to_string()
}

fn format_query_key(field: &str) -> String {
    if is_relaxed_key(field) {
        field.to_string()
    } else {
        serde_json::to_string(field).unwrap_or_else(|_| "\"\"".to_string())
    }
}

fn filter_field_candidates(fields: Vec<FieldCandidate>, token: &str) -> Vec<FieldCandidate> {
    let token = token.trim();
    let only_top_level = token.is_empty();

    let mut filtered: Vec<(usize, FieldCandidate)> = fields
        .into_iter()
        .filter_map(|field| {
            if only_top_level && field.depth > 0 {
                return None;
            }
            let score = if token.is_empty() { 0 } else { field_match_score(token, &field.path)? };
            Some((score, field))
        })
        .collect();

    filtered.sort_unstable_by(|(a_score, a), (b_score, b)| {
        a_score.cmp(b_score).then_with(|| compare_field_candidates(a, b))
    });
    filtered.truncate(24);
    filtered.into_iter().map(|(_, field)| field).collect()
}

fn field_match_score(token: &str, path: &str) -> Option<usize> {
    let path_score = ranked_match_score(token, path);
    let leaf_score = path
        .rsplit('.')
        .next()
        .and_then(|leaf| ranked_match_score(token, leaf))
        .map(|score| score + 8);

    match (path_score, leaf_score) {
        (Some(path_score), Some(leaf_score)) => Some(path_score.min(leaf_score)),
        (Some(score), None) | (None, Some(score)) => Some(score),
        (None, None) => None,
    }
}

fn compare_field_candidates(a: &FieldCandidate, b: &FieldCandidate) -> std::cmp::Ordering {
    field_penalty(&a.path)
        .cmp(&field_penalty(&b.path))
        .then(a.depth.cmp(&b.depth))
        .then_with(|| b.presence.cmp(&a.presence))
        .then_with(|| b.sampled_count.cmp(&a.sampled_count))
        .then_with(|| a.path.cmp(&b.path))
}

fn field_penalty(path: &str) -> u8 {
    if path.starts_with("__") {
        3
    } else if path == "_id" {
        1
    } else if path.starts_with('_') {
        2
    } else {
        0
    }
}

fn is_relaxed_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first == '$' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

fn query_token(text: &str, offset: usize) -> (usize, String) {
    let offset = offset.min(text.len());
    let bytes = text.as_bytes();
    let mut start = offset;
    while start > 0 {
        let ch = bytes[start - 1];
        if ch == b'.' || ch == b'$' || ch == b'_' || ch.is_ascii_alphanumeric() {
            start -= 1;
        } else {
            break;
        }
    }
    (start, text[start..offset].to_string())
}

fn fast_filter_token(text: &str, offset: usize) -> (usize, String) {
    let offset = offset.min(text.len());
    let bytes = text.as_bytes();
    let mut start = offset;
    while start > 0 {
        if bytes[start - 1].is_ascii_whitespace() {
            break;
        }
        start -= 1;
    }
    (start, text[start..offset].to_string())
}

fn is_fast_filter_text(text: &str) -> bool {
    !text.trim_start().starts_with('{')
}

fn split_fast_value_token(token: &str) -> Option<(&str, &str, &str)> {
    for operator in [">=", "<=", "!=", ":!", ":", "=", "~", ">", "<"] {
        if let Some(index) = token.find(operator) {
            let value_start = index + operator.len();
            return Some((&token[..index], operator, &token[value_start..]));
        }
    }
    None
}

fn fast_filter_value_items(
    ctx: &QueryEditorContext,
    token_prefix: &str,
    field: &str,
    value_prefix: &str,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let value_prefix = value_prefix.trim();

    if is_date_field(field) {
        for literal in DATE_SHORTCUT_LITERALS {
            push_fast_value_completion(&mut items, ctx, token_prefix, value_prefix, literal);
        }
    }

    for constructor in BSON_CONSTRUCTORS {
        if !matches_value_prefix(value_prefix, constructor.label) {
            continue;
        }
        items.push(completion_item(
            constructor.label,
            CompletionItemKind::CONSTRUCTOR,
            constructor.detail,
            format!("{token_prefix}{}", constructor.snippet),
            true,
            ctx.replace_range,
        ));
    }

    for literal in FILTER_VALUE_LITERALS {
        push_fast_value_completion(&mut items, ctx, token_prefix, value_prefix, literal);
    }

    rank_and_dedupe(items)
}

fn push_fast_value_completion(
    items: &mut Vec<CompletionItem>,
    ctx: &QueryEditorContext,
    token_prefix: &str,
    value_prefix: &str,
    literal: &ValueLiteral,
) {
    if !matches_value_prefix(value_prefix, literal.label) {
        return;
    }
    items.push(completion_item(
        literal.label,
        CompletionItemKind::VALUE,
        literal.detail,
        format!("{token_prefix}{}", literal.snippet),
        literal.snippet.contains('$'),
        ctx.replace_range,
    ));
}

fn value_completion_items(
    raw: &str,
    value: &super::query_values::ValueContext,
    samples: &[Bson],
) -> Vec<CompletionItem> {
    let rope = Rope::from(raw);
    let range = Range {
        start: rope.offset_to_position(value.range.start),
        end: rope.offset_to_position(value.range.end),
    };
    let mut items = Vec::new();
    for sample in samples {
        let label = crate::bson::format_relaxed_json_value(&sample.clone().into_relaxed_extjson());
        let searchable = match sample {
            Bson::String(text) => text.as_str(),
            _ => label.as_str(),
        };
        if !matches_value_prefix(&value.prefix, searchable) {
            continue;
        }
        items.push(completion_item(
            label.clone(),
            CompletionItemKind::VALUE,
            format!("{} · loaded values", crate::bson::bson_type_label(sample)),
            label,
            false,
            range,
        ));
    }
    if value.direct {
        let comparable = samples.is_empty()
            || samples.iter().any(|value| {
                matches!(
                    value,
                    Bson::Int32(_)
                        | Bson::Int64(_)
                        | Bson::Double(_)
                        | Bson::Decimal128(_)
                        | Bson::DateTime(_)
                )
            });
        for op in FILTER_OPERATORS {
            if !matches!(op.label, "$ne" | "$in" | "$exists")
                && !(comparable && matches!(op.label, "$gt" | "$gte" | "$lt" | "$lte"))
            {
                continue;
            }
            if !matches_value_prefix(&value.prefix, op.label) {
                continue;
            }
            items.push(completion_item(
                op.label,
                CompletionItemKind::OPERATOR,
                op.detail,
                format!("{{ {} }}", op.snippet),
                true,
                range,
            ));
        }
    }
    for constructor in BSON_CONSTRUCTORS {
        if matches_value_prefix(&value.prefix, constructor.label) {
            items.push(completion_item(
                constructor.label,
                CompletionItemKind::CONSTRUCTOR,
                constructor.detail,
                constructor.snippet,
                true,
                range,
            ));
        }
    }
    for literal in FILTER_VALUE_LITERALS {
        if matches_value_prefix(&value.prefix, literal.label) {
            items.push(completion_item(
                literal.label,
                CompletionItemKind::VALUE,
                literal.detail,
                literal.snippet,
                literal.snippet.contains('$'),
                range,
            ));
        }
    }
    rank_and_dedupe(items)
}

fn matches_value_prefix(prefix: &str, label: &str) -> bool {
    prefix.is_empty() || label.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase())
}

fn completion_item(
    label: impl Into<String>,
    kind: CompletionItemKind,
    detail: impl Into<String>,
    new_text: impl Into<String>,
    is_snippet: bool,
    replace_range: Range,
) -> CompletionItem {
    let template = new_text.into();
    let cursor = if is_snippet { cursor_from_template(&template) } else { None };
    let text = if is_snippet { label_from_template(&template) } else { template };
    CompletionItem {
        label: label.into(),
        kind: Some(kind),
        detail: Some(detail.into()),
        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
        data: cursor.map(|offset| serde_json::json!({ "cursor_offset": offset })),
        text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
            new_text: text,
            insert: replace_range,
            replace: replace_range,
        })),
        ..Default::default()
    }
}

fn rank_and_dedupe(mut items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.label.clone()));
    items
}

#[cfg(test)]
mod tests {
    use super::{
        FieldCandidate, QueryInputKind, compare_field_candidates, fast_filter_token, field_penalty,
        filter_field_candidates, format_query_key, matches_value_prefix, normalize_query_path,
        query_input_in_string_or_comment, query_token, split_fast_value_token, wrap_query_input,
    };

    #[test]
    fn completion_templates_insert_plain_text_and_place_the_caret_in_the_argument() {
        let item = super::completion_item(
            "ObjectId",
            lsp_types::CompletionItemKind::CONSTRUCTOR,
            "ObjectId",
            "ObjectId(\"$1\")$0",
            true,
            lsp_types::Range::default(),
        );
        assert_eq!(item.insert_text_format, Some(lsp_types::InsertTextFormat::PLAIN_TEXT));
        assert_eq!(item.data.unwrap()["cursor_offset"], 10);
        let Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) = item.text_edit else {
            panic!("text edit");
        };
        assert_eq!(edit.new_text, "ObjectId(\"\")");
        let item = super::completion_item(
            "$gt",
            lsp_types::CompletionItemKind::OPERATOR,
            "Greater than",
            "{ $gt: $1 }$0",
            true,
            lsp_types::Range::default(),
        );
        let Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) = item.text_edit else {
            panic!("text edit");
        };
        assert_eq!(edit.new_text, "{ $gt:  }");
    }

    #[test]
    fn sampled_value_completion_replaces_only_the_value_and_preserves_literal_dollars() {
        use gpui_kit::component::RopeExt;
        use mongodb::bson::Bson;
        let source = "{ status: \"ac\", enabled: true }";
        let cursor = source.find("ac").unwrap() + 2;
        let context = super::super::query_values::value_context(source, cursor).unwrap();
        let items = super::value_completion_items(
            source,
            &context,
            &[Bson::String("active".into()), Bson::String("pending".into())],
        );
        assert_eq!(items.len(), 1);
        let Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) = &items[0].text_edit
        else {
            panic!("text edit");
        };
        let rope = gpui_kit::component::input::Rope::from(source);
        let mut result = source.to_string();
        result.replace_range(
            rope.position_to_offset(&edit.replace.start)
                ..rope.position_to_offset(&edit.replace.end),
            &edit.new_text,
        );
        assert_eq!(result, "{ status: \"active\", enabled: true }");

        let source = "{ status:  }";
        let context = super::super::query_values::value_context(source, 10).unwrap();
        let items = super::value_completion_items(source, &context, &[Bson::String("$0".into())]);
        let Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) = &items[0].text_edit
        else {
            panic!("text edit");
        };
        assert_eq!(edit.new_text, "\"$0\"");
    }

    #[test]
    fn normalize_schema_array_paths_for_queries() {
        assert_eq!(normalize_query_path("items.[*].sku"), "items.sku");
        assert_eq!(normalize_query_path("tags.[*]"), "tags");
    }

    #[test]
    fn dotted_query_keys_are_quoted() {
        assert_eq!(format_query_key("profile.name"), "\"profile.name\"");
        assert_eq!(format_query_key("status"), "status");
    }

    #[test]
    fn token_scan_keeps_dot_paths() {
        let (start, token) = query_token("{ profile.na", "{ profile.na".len());
        assert_eq!(start, 2);
        assert_eq!(token, "profile.na");
    }

    #[test]
    fn fast_filter_token_keeps_negated_field_prefix() {
        let (start, token) = fast_filter_token("status:active !dele", "status:active !dele".len());
        assert_eq!(start, "status:active ".len());
        assert_eq!(token, "!dele");
    }

    #[test]
    fn fast_value_prefix_matching_is_case_insensitive() {
        assert!(matches_value_prefix("iso", "ISODate"));
        assert!(matches_value_prefix("TOD", "today"));
        assert!(!matches_value_prefix("last9", "last7d"));
    }

    #[test]
    fn fast_value_token_splits_field_operator_and_value_prefix() {
        assert_eq!(split_fast_value_token("createdAt:to"), Some(("createdAt", ":", "to")));
        assert_eq!(split_fast_value_token("createdAt>=last"), Some(("createdAt", ">=", "last")));
    }

    #[test]
    fn wrapped_filter_cursor_lands_in_find_argument() {
        let raw = "{ status: true }";
        let (wrapped, cursor) = wrap_query_input(QueryInputKind::Filter, "users", raw, raw.len());
        assert!(wrapped.contains(".find("));
        assert_eq!(wrapped.as_bytes()[cursor], b')');
    }

    #[test]
    fn string_detection_works_for_filter_inputs() {
        let raw = r#"{ name: "ali" }"#;
        let cursor = raw.find("ali").expect("cursor") + 2;
        assert!(query_input_in_string_or_comment(QueryInputKind::Filter, "users", raw, cursor));
    }

    #[test]
    fn internal_fields_rank_after_normal_fields() {
        let normal = FieldCandidate {
            path: "status".to_string(),
            depth: 0,
            presence: 10,
            sampled_count: 10,
            type_label: "String".into(),
        };
        let internal = FieldCandidate {
            path: "__v".to_string(),
            depth: 0,
            presence: 10,
            sampled_count: 10,
            type_label: "Int32".into(),
        };
        assert!(compare_field_candidates(&normal, &internal).is_lt());
        assert!(field_penalty("__v") > field_penalty("status"));
    }

    #[test]
    fn field_candidates_are_fuzzy_and_include_nested_matches() {
        let fields = vec![
            FieldCandidate {
                path: "status".to_string(),
                depth: 0,
                presence: 10,
                sampled_count: 10,
                type_label: "String".into(),
            },
            FieldCandidate {
                path: "profile.email".to_string(),
                depth: 1,
                presence: 9,
                sampled_count: 9,
                type_label: "String".into(),
            },
        ];

        let results = filter_field_candidates(fields, "emial");
        assert_eq!(results.first().map(|field| field.path.as_str()), Some("profile.email"));
    }
}
