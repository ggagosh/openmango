# MongoDB Context-Aware Completion System

## Goal
Build a native Rust completion system for JSON editors that understands MongoDB aggregation pipelines. Provides intelligent autocompletion based on:
- Current stage operator ($match, $group, etc.)
- Cursor position in JSON (key vs value context)
- Collection schema (field names from sample)
- Previous stages in pipeline (field transformations)

---

## Research Summary

### gpui-component CompletionProvider API
```rust
pub trait CompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        window: &mut Window,
        cx: &mut Context<InputState>,
    )
        -> Task<Result<CompletionResponse>>;
    fn is_completion_trigger(
        &self,
        offset: usize,
        new_text: &str,
        cx: &mut Context<InputState>,
    ) -> bool;
}
```

### Existing Codebase
- **Operators defined**: `src/views/documents/views/aggregation/operators.rs` - 23 operators in 7 groups
- **Stage model**: `PipelineStage { operator: String, body: String, enabled: bool }`
- **BSON parsing**: `src/bson/parser.rs` - JSON↔BSON conversion utilities
- **Session state**: Can access `PipelineState` with all stages and results

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      AggregationCompletionProvider          │
│  (implements CompletionProvider)                            │
├─────────────────────────────────────────────────────────────┤
│  Context:                                                   │
│  - current_operator: String ($match, $group, etc.)          │
│  - collection_fields: Vec<FieldInfo>                        │
│  - pipeline_context: PipelineFieldTracker                   │
│  - shared_state: Rc<RefCell<CompletionState>>               │
├─────────────────────────────────────────────────────────────┤
│                          │                                  │
│    ┌────────────────────┼────────────────────┐              │
│    ▼                    ▼                    ▼              │
│ JsonCursorContext   OperatorKnowledge   FieldSuggester      │
│ (parse position)    (operator schemas)  (schema + pipeline) │
└─────────────────────────────────────────────────────────────┘
```

### Components

1. **JsonCursorContext** - Determines where cursor is in JSON
   - At object key position (after `{` or `,`)
   - At value position (after `:`)
   - Inside string (key vs value; field path `"$fieldName"`)
   - At array element position
   - Handles scalar root bodies (number/string) for stages like `$limit`, `$skip`, `$count`
   - Offset handling is byte-based (matches gpui)

2. **OperatorKnowledge** - Static MongoDB operator definitions
   - Per-stage operator valid keys ($match keys, $group keys, etc.)
   - Query operators ($eq, $gt, $in, $regex, etc.)
   - Accumulator operators ($sum, $avg, $push, etc.)
   - Expression operators ($concat, $add, $cond, etc.)

3. **FieldSuggester** - Dynamic field suggestions
   - From collection sample (fetched on editor focus)
   - From pipeline tracking (field additions/renames/projections)

---

## MongoDB Operator Knowledge Base

### Supported Stage Operators
All stages from `OPERATOR_GROUPS` are in scope:
```
$match, $project, $addFields, $set, $unset, $replaceRoot, $replaceWith,
$group, $bucket, $bucketAuto, $lookup, $unwind, $sort, $limit, $skip,
$out, $merge, $count, $facet, $sample, $unionWith, $redact, $graphLookup
```

### Stage Operators (what goes after `:` in stage body)

| Stage | Expected Structure |
|-------|-------------------|
| `$match` | Query object: `{ field: { $op: value } }` or `{ field: value }` |
| `$project` | Projection: `{ field: 1/0/expr }` |
| `$group` | Group: `{ _id: expr, field: { $acc: expr } }` |
| `$sort` | Sort: `{ field: 1/-1 }` |
| `$lookup` | Join: `{ from, localField, foreignField, as }` |
| `$unwind` | Unwind: `"$arrayField"` or `{ path, ... }` |
| `$addFields` | Add: `{ newField: expr }` |
| `$set` | Alias of `$addFields` |
| `$unset` | `"field"` or `["field1", "field2"]` |
| `$replaceRoot` | `{ newRoot: expr }` |
| `$replaceWith` | `expr` |
| `$limit` | Number |
| `$skip` | Number |
| `$count` | String field name |
| `$facet` | Object of sub-pipelines |
| `$sample` | `{ size: N }` |
| `$unionWith` | `"collection"` or `{ coll, pipeline }` |
| `$redact` | Expression returning `$$KEEP` / `$$PRUNE` / `$$DESCEND` |
| `$graphLookup` | `{ from, startWith, connectFromField, connectToField, as, ... }` |
| `$bucket` | `{ groupBy, boundaries, default?, output? }` |
| `$bucketAuto` | `{ groupBy, buckets, output? }` |
| `$out` | `"collection"` or `{ db?, coll? }` |
| `$merge` | `{ into, on?, whenMatched?, whenNotMatched? }` |

For advanced stages (facet/graphLookup/bucket/etc.), v1 completion will provide
baseline key suggestions and field refs, with richer schemas added incrementally.

### Query Operators (for $match)
```
Comparison: $eq, $ne, $gt, $gte, $lt, $lte, $in, $nin
Logical: $and, $or, $not, $nor
Element: $exists, $type
Array: $all, $elemMatch, $size
Evaluation: $regex, $expr, $mod, $text
```

### Accumulator Operators (for $group)
```
$sum, $avg, $min, $max, $first, $last, $push, $addToSet, $stdDevPop, $stdDevSamp
```

### Expression Operators (for $project, $addFields, $group expressions)
```
Arithmetic: $add, $subtract, $multiply, $divide, $mod, $abs, $ceil, $floor
String: $concat, $substr, $toLower, $toUpper, $trim, $split
Date: $year, $month, $dayOfMonth, $hour, $minute, $second, $dateToString
Conditional: $cond, $ifNull, $switch
Array: $arrayElemAt, $concatArrays, $filter, $map, $reduce, $size
Type: $toString, $toInt, $toDouble, $toDate, $toBool
```

---

## Implementation Phases

### Phase 1: Basic Completion Infrastructure

**1.1 Create module structure**
```
src/completions/
├── mod.rs              # Module exports
├── provider.rs         # AggregationCompletionProvider
├── cursor.rs           # JsonCursorContext parser
└── operators.rs        # MongoDB operator knowledge
```

**1.2 `src/completions/cursor.rs`** - JSON position detection (using Chumsky)

```rust
use chumsky::prelude::*;

pub enum JsonContext {
    ObjectKey { depth: usize, path: Vec<String> },
    ObjectValue { depth: usize, path: Vec<String>, key: String },
    ArrayElement { depth: usize, path: Vec<String> },
    StringLiteral { role: StringRole, in_field_ref: bool },
    Unknown,
}

pub enum StringRole {
    Key,
    Value,
    Unknown,
}

/// Partial JSON AST with position spans
#[derive(Debug, Clone)]
pub enum JsonNode {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Spanned<JsonNode>>),
    Object(Vec<(Spanned<String>, Option<Spanned<JsonNode>>)>),  // Value optional for incomplete
    Invalid,
}

type Spanned<T> = (T, SimpleSpan);

/// Parse JSON with error recovery, returns partial AST even for incomplete input
fn json_parser<'a>() -> impl Parser<'a, &'a str, Spanned<JsonNode>, extra::Err<Rich<'a, char>>> {
    recursive(|value| {
        let string = just('"')
            // NOTE: simplified; implement proper JSON escapes
            .ignore_then(none_of('"').repeated().collect::<String>())
            .then_ignore(just('"').or_not())  // Optional closing quote for incomplete
            .map_with(|s, e| (s, e.span()));

        let number = text::int(10)
            .then(just('.').then(text::digits(10)).or_not())
            .to_slice()
            .from_str::<f64>()
            .map(JsonNode::Number)
            .or_not()
            .map(|num| num.unwrap_or(JsonNode::Invalid));

        let array = value.clone()
            .separated_by(just(',').padded())
            .collect()
            .delimited_by(just('['), just(']').or_not())
            .map(JsonNode::Array)
            .recover_with(via_parser(nested_delimiters('[', ']', [], |_| JsonNode::Invalid)));

        let member = string.clone()
            .then_ignore(just(':').padded().or_not())
            .then(value.clone().or_not());

        let object = member
            .separated_by(just(',').padded())
            .collect()
            .delimited_by(just('{'), just('}').or_not())
            .map(JsonNode::Object)
            .recover_with(via_parser(nested_delimiters('{', '}', [], |_| JsonNode::Invalid)));

        choice((
            just("null").to(JsonNode::Null),
            just("true").to(JsonNode::Bool(true)),
            just("false").to(JsonNode::Bool(false)),
            number,
            string.map(|(s, _)| JsonNode::String(s)),
            array,
            object,
        ))
        .map_with(|node, e| (node, e.span()))
        .padded()
    })
}

pub fn analyze_cursor_position(text: &str, offset: usize) -> JsonContext;
```

Algorithm:
1. Parse text up to cursor with Chumsky (error recovery handles incomplete JSON)
2. Walk AST to find deepest node containing cursor offset
3. Determine if cursor is at key position, value position, or inside string
4. Extract path from root to cursor position
5. Offsets are byte-based (align with gpui); spans must be byte offsets
6. If parsing fails, fall back to lightweight scanning for string/field-ref context

**1.3 `src/completions/operators.rs`** - Static operator definitions
```rust
pub struct OperatorInfo {
    pub name: &'static str,
    pub kind: OperatorKind,
    pub doc: &'static str,
    pub snippet: Option<&'static str>,
}

pub enum OperatorKind {
    Stage,           // $match, $group
    Query,           // $eq, $gt
    Accumulator,     // $sum, $avg
    Expression,      // $add, $concat
}

pub fn get_stage_completions(operator: &str, context: &JsonContext) -> Vec<OperatorInfo>;
pub fn get_query_operators() -> &'static [OperatorInfo];
pub fn get_accumulators() -> &'static [OperatorInfo];
pub fn get_expressions() -> &'static [OperatorInfo];
```

**1.4 `src/completions/provider.rs`** - CompletionProvider impl
```rust
pub struct AggregationCompletionProvider {
    state: Rc<RefCell<CompletionState>>,     // Shared mutable state
}

pub struct CompletionState {
    pub operator: String,
    pub collection_fields: Vec<FieldInfo>,
    pub pipeline_tracker: PipelineFieldTracker,
}

impl CompletionProvider for AggregationCompletionProvider {
    fn is_completion_trigger(&self, offset: usize, new_text: &str, cx) -> bool {
        matches!(new_text, "\"" | ":" | "$" | "." | "{" | ",")
    }

    fn completions(&self, text: &Rope, offset: usize, trigger, window, cx)
        -> Task<Result<CompletionResponse>>
    {
        let text_str = text.to_string();
        let context = analyze_cursor_position(&text_str, offset);
        let items = self.get_completions_for_context(&context);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }
}
```

### Phase 2: Context-Aware Suggestions

**2.1 Per-operator completion logic**

```rust
impl AggregationCompletionProvider {
    fn get_completions_for_context(&self, ctx: &JsonContext) -> Vec<CompletionItem> {
        let state = self.state.borrow();
        let operator = state.operator.as_str();
        match (operator, ctx) {
            // $match at root key → field names + logical operators
            ("$match", JsonContext::ObjectKey { depth: 1, .. }) => {
                self.field_completions() + query_logical_operators()
            }

            // $match field value → query operators or literal
            ("$match", JsonContext::ObjectValue { depth: 1, key, .. }) => {
                query_comparison_operators()
            }

            // $group at root → _id required, then accumulators
            ("$group", JsonContext::ObjectKey { depth: 1, .. }) => {
                vec![completion("_id")] + self.field_completions()
            }

            // $group accumulator value → accumulator operators
            ("$group", JsonContext::ObjectValue { depth: 2, .. }) => {
                accumulator_operators()
            }

            // $project field value → 1, 0, or expression
            ("$project", JsonContext::ObjectValue { .. }) => {
                vec![completion("1"), completion("0")] + expression_operators()
            }

            // $lookup at root → required keys
            ("$lookup", JsonContext::ObjectKey { depth: 1, .. }) => {
                vec!["from", "localField", "foreignField", "as", "let", "pipeline"]
                    .into_iter().map(completion).collect()
            }

            // Key strings inside object behave like ObjectKey
            (_, JsonContext::StringLiteral { role: StringRole::Key, .. }) => {
                self.key_completions_for_operator(operator)
            }

            // Inside string starting with $ → field references
            (_, JsonContext::StringLiteral { in_field_ref: true, .. }) => {
                self.field_ref_completions()  // "$fieldName" format
            }

            _ => vec![]
        }
    }
}

// Treat key-string context like ObjectKey for the current operator
fn key_completions_for_operator(&self, operator: &str) -> Vec<CompletionItem>;
```

**2.2 Field reference detection**
- When cursor is inside `"$..."`, suggest field names prefixed with `$`
- Handle nested paths: `"$address.city"`
- Also trigger on `.` to continue nested suggestions

### Phase 3: Collection Schema Integration

**3.1 `src/completions/schema.rs`** - Schema extraction
```rust
pub struct FieldInfo {
    pub path: String,           // "address.city"
    pub bson_type: String,      // "String", "Int32", "Array"
    pub sample_value: Option<String>,
}

pub fn extract_fields_from_documents(docs: &[Document]) -> Vec<FieldInfo>;
```

Algorithm:
- Sample first N documents (e.g., 100)
- Recursively extract all field paths
- Track most common BSON type per path
- Deduplicate and sort (deterministic ordering)

**3.2 Fetch schema on editor focus**
```rust
// In aggregation view, when stage body editor focuses:
cx.spawn(|view, mut cx| async move {
    let sample = fetch_collection_sample(&session_key).await?;
    let fields = extract_fields_from_documents(&sample);
    view.update(&mut cx, |view, cx| {
        view.completion_provider.set_collection_fields(fields);
    })
});
```

### Phase 4: Pipeline Field Tracking

**4.1 `src/completions/pipeline.rs`** - Track field transformations

```rust
use serde_json::Value;
use std::collections::BTreeSet;

pub struct PipelineFieldTracker {
    /// Fields available at current stage (after processing previous stages)
    available_fields: BTreeSet<String>,
    /// Original collection fields (baseline)
    collection_fields: BTreeSet<String>,
}

impl PipelineFieldTracker {
    pub fn new(collection_fields: Vec<String>) -> Self {
        let fields: BTreeSet<_> = collection_fields.into_iter().collect();
        Self {
            available_fields: fields.clone(),
            collection_fields: fields,
        }
    }

    /// Process stages up to (but not including) current_stage_index
    pub fn process_stages(&mut self, stages: &[PipelineStage], current_stage_index: usize) {
        // Reset to collection fields
        self.available_fields = self.collection_fields.clone();

        for stage in stages.iter().take(current_stage_index) {
            if !stage.enabled { continue; }
            self.apply_stage(stage);
        }
    }

    fn parse_body_value(&self, body: &str) -> Option<Value> {
        serde_json::from_str::<Value>(body).ok()
    }

    fn apply_stage(&mut self, stage: &PipelineStage) {
        match stage.operator.as_str() {
            "$project" => self.apply_project(&stage.body),
            "$group" => self.apply_group(&stage.body),
            "$addFields" | "$set" => self.apply_add_fields(&stage.body),
            "$unset" => self.apply_unset(&stage.body),
            "$unwind" => self.apply_unwind(&stage.body),
            "$lookup" => self.apply_lookup(&stage.body),
            "$replaceRoot" | "$replaceWith" => self.apply_replace_root(&stage.body),
            _ => {}  // $match, $sort, $limit, $skip don't change fields
        }
    }
    // Note: stage bodies can be scalar (e.g., $limit: 5, $count: "name", $unwind: "$path").
    // parse_body_value returns Value::Number/String for these; field tracking should ignore or
    // use the string path for $unwind/$replaceWith where applicable.

    fn apply_project(&mut self, body: &str) {
        // Parse body, extract included fields
        // { "name": 1, "age": 1 } → fields = ["name", "age", "_id"]
        // { "name": 1, "_id": 0 } → fields = ["name"]
        // Exclusion mode: { "password": 0 } → remove "password", keep others
    }

    fn apply_group(&mut self, body: &str) {
        // { "_id": "$category", "total": { "$sum": 1 } }
        // → fields = ["_id", "total"]
        self.available_fields.clear();
        // Parse and extract output field names
    }

    fn apply_add_fields(&mut self, body: &str) {
        // { "fullName": { "$concat": [...] } }
        // → add "fullName" to existing fields
    }

    fn apply_unset(&mut self, body: &str) {
        // "fieldName" or ["field1", "field2"]
        // → remove specified fields
    }

    fn apply_unwind(&mut self, body: &str) {
        // "$items" or { "path": "$items", "as": "item" }
        // → array field becomes scalar (type changes, not field list)
    }

    fn apply_lookup(&mut self, body: &str) {
        // { "as": "joined" } → add "joined" field
    }

    fn apply_replace_root(&mut self, body: &str) {
        // { "newRoot": "$embedded" } → fields become embedded doc fields
        // Complex: may need to track nested field structures
    }

    pub fn get_available_fields(&self) -> &BTreeSet<String> {
        &self.available_fields
    }
}
```

**4.2 Integration with provider**

```rust
impl AggregationCompletionProvider {
    pub fn update_pipeline_context(
        &mut self,
        stages: &[PipelineStage],
        current_stage_index: usize,
    ) {
        self.pipeline_tracker.process_stages(stages, current_stage_index);
    }

    fn field_completions(&self) -> Vec<CompletionItem> {
        // Prefer pipeline-tracked fields if available
        // Fall back to collection schema fields (FieldInfo)
        let state = self.state.borrow();
        if state.pipeline_tracker.get_available_fields().is_empty() {
            state
                .collection_fields
                .iter()
                .map(|f| self.make_field_completion(&f.path))
                .collect()
        } else {
            state
                .pipeline_tracker
                .get_available_fields()
                .iter()
                .map(|f| self.make_field_completion(f))
                .collect()
        }
    }
}
```

This tracks:
- `$project { a: 1 }` → only field `a` (and `_id` unless excluded) remains
- `$group { _id: ..., total: { $sum: 1 } }` → fields become `_id`, `total`
- `$addFields { newField: ... }` → adds `newField` to existing
- `$unset "secret"` → removes `secret` field
- `$lookup { as: "orders" }` → adds `orders` array field

---

## Files to Create/Modify

| File | Action |
|------|--------|
| `src/completions/mod.rs` | CREATE |
| `src/completions/provider.rs` | CREATE |
| `src/completions/cursor.rs` | CREATE |
| `src/completions/operators.rs` | CREATE |
| `src/completions/schema.rs` | CREATE (Phase 3) |
| `src/completions/pipeline.rs` | CREATE (Phase 4) |
| `src/main.rs` | MODIFY - add `mod completions;` |
| `src/views/documents/views/aggregation/mod.rs` | MODIFY - wire provider |
| `Cargo.toml` | MODIFY - add chumsky, ensure lsp-types |

---

## Dependencies

```toml
[dependencies]
chumsky = "1.0"     # Parser combinator with error recovery
lsp-types = "..."   # Already in Cargo.lock - CompletionItem types
```

---

## Completion Triggers

| Trigger | Context | Suggestions |
|---------|---------|-------------|
| `"` | Start of string | Field names, operators |
| `$` | Inside string | Field refs (`$field`), operators |
| `.` | Inside string | Nested field paths |
| `:` | After key | Values based on key context |
| `{` | New object | Keys for current context |
| `,` | After value | Next key/element |

---

## Verification

### Phase 1-2: Basic Completions
1. **Basic completions**: Type `{` in $match body → see field names
2. **Query operators**: Type `"$` after field name → see $eq, $gt, etc.
3. **Accumulators**: In $group, type `{` after field → see $sum, $avg
4. **Field refs**: Type `"$` anywhere → see collection field names
5. **Lookup keys**: In $lookup, type `"` → see from, localField, etc.
6. **No crashes**: Invalid JSON doesn't crash completion (Chumsky recovery)

### Phase 3: Schema Integration
7. **Schema fetch**: Focus on stage editor → collection fields load
8. **Field suggestions**: Type `"` in $match → see actual collection fields
9. **Nested fields**: Type `"address.` → see `address.city`, `address.zip`

### Phase 4: Pipeline Tracking
10. **After $project**: Stage 2 after `$project { name: 1 }` → only `name`, `_id` suggested
11. **After $group**: Stage 2 after `$group { _id: "$cat", count: {...} }` → only `_id`, `count` suggested
12. **After $addFields**: New field appears in suggestions
13. **After $lookup**: `as` field name appears in suggestions

---

## Design Decisions

- **Pure Rust**: No sidecar process, all logic in Rust
- **Synchronous completions**: No async fetch during completion (pre-load schema)
- **Graceful fallback**: If context unclear, return empty (user types manually)
- **Performance**: Partial AST with Chumsky error recovery; fall back to lightweight scanning if parse fails
- **Extensibility**: Operator knowledge in static arrays, easy to extend

---

## Scope

**Implementing all phases (1-4)** in this iteration:
- Phase 1: Basic infrastructure
- Phase 2: Context-aware suggestions
- Phase 3: Collection schema integration
- Phase 4: Pipeline field tracking

---

## Future Enhancements (Post-Implementation)

1. **Schema caching**: Cache collection schemas across sessions
2. **Inline validation**: Red squiggles for invalid operators
3. **Snippets**: Insert templates like `{ $sum: "$" }` with cursor positioning
4. **Documentation hover**: Show operator docs on hover
5. **Filter/Sort editors**: Extend completion to other JSON editors (find, sort dialogs)
