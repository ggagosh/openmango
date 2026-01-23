# Aggregation Pipeline Builder — UX Specification

## Entry Point

New subview tab in the collection header:

```
Documents | Indexes | Stats | Aggregate
```

Same pattern as existing subviews. No new routing or tab types needed.

---

## Layout: Three-Panel Split

```
┌─────────────────────────────────────────────────────────┐
│  [+ Add Stage]            [Auto ○] [▶ Run] [📊 Analyze] │
├────────────────────┬────────────────────────────────────┤
│                    │                                    │
│  STAGE LIST        │  STAGE EDITOR                     │
│  (left panel)      │  (right panel)                    │
│                    │                                    │
│  ☑ 1. $match    ←  │  [$match ▼]                       │
│     3,201 docs     │  ┌────────────────────────────┐   │
│                    │  │ 1 │ {                      │   │
│  ☑ 2. $lookup     │  │ 2 │   "status": "active",  │   │
│     3,201 docs     │  │ 3 │   "age": { "$gt": 21 } │   │
│                    │  │ 4 │ }                      │   │
│  ☐ 3. $sort       │  └────────────────────────────┘   │
│     (disabled)     │                                    │
│                    │                                    │
│  ☑ 4. $group      │                                    │
│     48 docs        │                                    │
│                    │                                    │
├────────────────────┴────────────────────────────────────┤
│  RESULTS (after Stage 1: $match)        20 documents   │
│  Key            │ Value            │ Type               │
│  ▶ Document 0   │                  │ Object             │
│    status       │ "active"         │ String             │
│    age          │ 34               │ Int32              │
│  ▶ Document 1   │                  │ Object             │
├─────────────────────────────────────────────────────────┤
│  48 docs total (23ms)                  [< Prev] [Next >] │
└─────────────────────────────────────────────────────────┘
```

### Panel Responsibilities

| Panel | Purpose |
|-------|---------|
| **Stage List** (left) | All stages at a glance. Checkboxes, doc counts, selection, reorder. |
| **Stage Editor** (right) | Code editor for the selected stage. Operator dropdown at top. |
| **Results** (bottom) | Document tree showing output at the selected stage. |

### Why Split Over Cards

- Scales to many stages without scrolling through large editors
- Editor area is larger (complex `$lookup`/`$facet` stages need space)
- Stage list gives a pipeline overview at all times
- Easy to extend later (add input/output diff, templates panel, code export)

---

## Stage List (Left Panel)

Each stage row shows:

```
☑ 1. $match  [IXSCAN 2ms]     ← after Analyze
   12,450 → 3,201 docs
```

### Elements

- **Checkbox** — enable/disable stage (disabled stages are skipped on Run)
- **Number** — stage position (1-indexed)
- **Operator name** — `$match`, `$group`, etc.
- **Analysis badge** — appears after clicking Analyze (see Analysis section)
- **Doc count** — `in → out` format, updates after Run

### Interactions

- **Click** a stage → selects it, shows its JSON in editor, shows its output in results
- **Arrow keys** (↑↓) — navigate between stages when list is focused
- **Drag** — reorder stages (drag handle on left edge)
- **Right-click** — context menu: Delete, Duplicate, Disable, Move Up, Move Down

### Adding Stages

- **"+ Add Stage"** button at top → appends new stage, opens operator dropdown
- **Insert between** — hover between stages reveals a `+` insert point

### Empty State

When no stages exist:

```
┌─────────────────────────────────────────┐
│                                         │
│   No pipeline stages yet.               │
│   [+ Add your first stage]              │
│                                         │
│   Common starting points:               │
│   [$match]  [$group]  [$project]        │
│                                         │
└─────────────────────────────────────────┘
```

Quick-start buttons for the most common operators.

---

## Stage Editor (Right Panel)

Shows the JSON body of the currently selected stage.

### Header

```
[$match ▼]                              [Format] [Clear]
```

- **Operator dropdown** — grouped by category (see below)
- **Format** — prettify JSON
- **Clear** — reset to `{}`

### Editor

Uses existing `InputState` code editor with:
- Line numbers
- JSON syntax highlighting
- Soft wrap
- Monospace font

### Operator Dropdown (grouped)

```
── Filter ──────────
  $match
── Transform ───────
  $project
  $addFields / $set
  $unset
  $replaceRoot / $replaceWith
── Group ───────────
  $group
  $bucket
  $bucketAuto
── Join ────────────
  $lookup
  $unwind
── Sort & Limit ────
  $sort
  $limit
  $skip
── Output ──────────
  $out
  $merge
── Other ───────────
  $count
  $facet
  $sample
  $unionWith
  $redact
  $graphLookup
```

---

## Results Panel (Bottom)

Reuses existing document tree component (same as documents_view.rs).

### Header

```
Results (after Stage 2: $group)          48 documents (23ms)
```

Shows which stage is being previewed, doc count, and execution time.

### Stage Preview Behavior

- **Clicking a stage** in the list → results show output *up to and including* that stage
- Pipeline is truncated at the selected stage, disabled stages skipped
- Stages *after* the selected one are visually dimmed in the list
- If no stage selected → shows final pipeline output

### Pagination

Same skip/limit pagination as the documents view:

```
Showing 1-20 of 48                      [< Prev] Page 1 of 3 [Next >]
```

---

## Running the Pipeline

### Manual Run

- **▶ Run** button or **Cmd+Enter** — executes the pipeline
- Shows spinner in results panel while running
- On completion: updates results + doc counts in stage list
- On error: error banner in results panel with MongoDB error message

### Auto-Preview (Option)

- **[Auto ○]** toggle in the header — off by default
- When on: re-runs pipeline (up to selected stage) after 500ms debounce on edit
- When off: only runs on explicit Run or Cmd+Enter
- Status indicator: `○` = off, `●` = on

### What Gets Executed

- Only enabled stages (checked) up to the selected stage
- Sample limit applied (configurable, default: no limit for now)
- Pipeline sent as-is — no client-side validation beyond JSON parsing

---

## Analysis

### Triggering

Click **📊 Analyze** button → runs two things in parallel:
1. `aggregate().explain("executionStats")` on the full pipeline
2. Truncated pipeline runs with `$count` appended at each stage (for doc counts)

### Inline Badges (on stage list)

After analysis completes, badges appear on each stage row:

| Badge | Color | Meaning |
|-------|-------|---------|
| `IXSCAN` | green | Stage uses an index |
| `COLLSCAN` | yellow | Full collection scan |
| `2ms` | neutral | Execution time for this stage |
| `⚠ 94MB` | red | Memory approaching 100MB limit |

```
  ☑ 1. $match  [IXSCAN · 2ms]
     12,450 → 3,201 docs

  ☑ 2. $lookup [COLLSCAN ⚠ · 45ms]
     3,201 → 3,201 docs
```

Badges persist until the pipeline is edited (then cleared until next Analyze).

### Analysis Results View

When Analyze runs, results panel switches to analysis mode:

```
├──────────────── Analysis ────────────────── [× Close] ──┤
│                                                          │
│  Stage       │ Docs In → Out │ Strategy   │ Time │ Mem  │
│  ────────────┼───────────────┼────────────┼──────┼───── │
│  1. $match   │ 12,450 → 3,201│ IXSCAN    │  2ms │  -   │
│  2. $lookup  │  3,201 → 3,201│ COLLSCAN  │ 45ms │ 12MB │
│  3. $group   │  3,201 →   48 │ In-memory │  8ms │ 2MB  │
│  4. $sort    │     48 →   48 │ In-memory │ <1ms │  -   │
│                                                          │
│  Warnings                                                │
│  ─────────────────────────────────────────────────────   │
│  • Stage 2 ($lookup): COLLSCAN on foreign collection     │
│    "orders" — consider adding index on "orders.user_id"  │
│  • Stage 3 ($group): 3,201 docs grouped in memory        │
│    — acceptable, but watch with larger datasets          │
│                                                          │
│  Total: 12,450 → 48 docs, 55ms                          │
│                                                          │
├──────────────────────────────────────────────────────────┤
```

Clicking **[× Close]** returns to normal Output mode.

### Warning Rules

Simple conditionals on the explain data:

| Condition | Warning |
|-----------|---------|
| $match with COLLSCAN | "No index — consider creating one on [field]" |
| $sort on unindexed field, >10k docs | "In-memory sort — consider index" |
| $lookup foreign collection with COLLSCAN | "Foreign COLLSCAN — add index on [foreignField]" |
| Stage output = 0 docs | "Pipeline produces no results after this stage" |
| Memory >80MB estimated | "Approaching 100MB limit — consider allowDiskUse" |
| $unwind fan-out >3x | "Large fan-out — consider filtering before this stage" |

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Cmd+Enter | Run pipeline |
| ↑ / ↓ | Navigate stages (when list focused) |
| Delete / Backspace | Delete selected stage (with confirmation) |
| Cmd+D | Duplicate selected stage |
| Cmd+Shift+↑ | Move stage up |
| Cmd+Shift+↓ | Move stage down |

---

## State Model

```
PipelineState {
    stages: Vec<PipelineStage>,
    selected_stage: Option<usize>,
    results: Option<Vec<Document>>,
    stage_doc_counts: Vec<Option<u64>>,
    analysis: Option<PipelineAnalysis>,
    auto_preview: bool,
    loading: bool,
}

PipelineStage {
    operator: String,           // "$match", "$group", etc.
    body: String,               // JSON string
    enabled: bool,              // checkbox state
}

PipelineAnalysis {
    stages: Vec<StageAnalysis>,
    warnings: Vec<AnalysisWarning>,
    total_time_ms: u64,
}

StageAnalysis {
    docs_in: u64,
    docs_out: u64,
    strategy: String,           // "IXSCAN", "COLLSCAN", "In-memory"
    index_name: Option<String>,
    time_ms: u64,
    memory_bytes: Option<u64>,
}
```

Fits into existing session-per-tab model — `PipelineState` lives alongside `SessionData`.

---

## Scope: What's Excluded (for now)

- Pipeline persistence / saved pipelines
- Export (JSON, driver code, CSV)
- Text mode (free-form full pipeline editor)
- Stage Wizard / templates (beyond empty state suggestions)
- AI-assisted stage generation
- allowDiskUse toggle
- $merge / $out safety confirmations
