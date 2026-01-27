# Aggregation Editor — Complete Review

Branch: `aggr`
Commits: `a2d5a3f` (init aggr editor), `6443c51` (more aggr features)
Date: 2026-01-27

---

## Executive Summary

The aggregation editor is a feature-rich three-panel builder with modern UX (drag-and-drop, keyboard shortcuts, stage statistics). However, it has significant gaps compared to the mature document editor and potential scaling risks.

| Metric | Value |
|--------|-------|
| Aggregation code | ~2,300 lines across 4 files |
| Document editor | ~3,300 lines (tree, dialogs, actions) |

---

## What The Last Two Commits Delivered

The aggregation subview is now a solid three-panel builder:

| Component | File | Lines | Features |
|-----------|------|-------|----------|
| Stage List | `stage_list.rs` | ~1500 | Drag reorder, enable/disable, selection, context menu, delete confirmation, insert points, keyboard nav |
| Stage Editor | `stage_editor.rs` | ~240 | Operator dropdown, JSON editor, format/clear actions, Cmd+Enter to run |
| Results View | `results_view.rs` | ~400 | Read-only tree, stage preview, counts/timing, limit input, pagination |
| Operator Picker | `stage_list.rs` | ~200 | Search, grouped operators, keyboard handling |

### Implemented Features
- Dedicated Aggregation subview and state model per session
- Stage list with enable/disable, selection, drag reorder, insert points, context menu, delete confirmation
- Operator picker dialog with search, keyboard handling, and grouped operators
- Stage editor with operator dropdown, JSON editor, format/clear actions, Cmd/Ctrl+Enter to run
- Results panel with stage preview labeling, stage counts/timing, limit input, pagination, read-only tree
- Pipeline import from JSON array
- Per-stage doc counts and stage timing computed during runs
- Workspace persistence of pipeline stages

### Key Entry Points
- `src/views/documents/views/aggregation/mod.rs`
- `src/views/documents/views/aggregation/stage_list.rs`
- `src/views/documents/views/aggregation/stage_editor.rs`
- `src/views/documents/views/aggregation/results_view.rs`

---

## Feature Comparison Matrix

### Legend
- ✅ Full support
- ⚠️ Partial/buggy
- ❌ Not implemented

| Feature | Document Editor | Index Editor | Aggregation |
|---------|-----------------|--------------|-------------|
| **Editing** |
| Inline editing | ✅ | ❌ | ❌ |
| Dialog editing | ✅ (Property) | ✅ | ⚠️ (operator picker) |
| Type conversion | ✅ (10+ types) | ❌ | ❌ |
| Field validation | ✅ | ✅ | ❌ |
| JSON validation | ✅ | ✅ | ⚠️ (basic parse) |
| **Keyboard** |
| Navigation | ✅ | ❌ | ✅ |
| Edit shortcuts | ✅ (F2, Alt+) | ❌ | ✅ (Cmd+D, Cmd+Shift+E) |
| Execute | ❌ | ❌ | ✅ (Cmd+Enter) |
| Format/Clear | ❌ | ❌ | ✅ (Cmd+Shift+F/K) |
| Find | ✅ (Cmd+F) | ❌ | ❌ |
| Copy | ✅ (Cmd+C) | ❌ | ❌ |
| **Drag & Drop** |
| Reorder | ❌ | ❌ | ✅ |
| Visual feedback | ❌ | ❌ | ✅ |
| **Context Menus** |
| Node-level menu | ✅ | ❌ | ⚠️ (stage only) |
| Copy actions | ✅ | ❌ | ❌ |
| Bulk operations | ✅ | ❌ | ❌ |
| **State** |
| Dirty tracking | ✅ (blue dot) | ❌ | ❌ |
| Request-id gating | ✅ | ❌ | ❌ |
| **Search** |
| Search UI | ✅ | ❌ | ❌ |
| Highlight matches | ✅ | ❌ | ❌ |
| **Safety** |
| Read-only check | ✅ | ✅ | ❌ |
| Confirmation dialogs | ✅ | ✅ | ⚠️ (delete only) |
| **Auto-Features** |
| Auto-indent | ❌ | ❌ | ✅ |
| Auto-suggestions | ❌ | ✅ (fields) | ❌ |
| Auto-save | ❌ | ❌ | ⚠️ (per keystroke) |

---

## Gaps Compared To Other Editors

### P0 - Critical (High-Impact)

#### 1. No "Find in results" for aggregation results
| | |
|-|-|
| **Impact** | Users can't search aggregation output |
| **Location** | `src/views/documents/actions.rs:27`, `actions.rs:39` |
| **Evidence** | Handler explicitly exits unless subview is Documents |
| **Fix** | Extend find handler to support Aggregation subview |

#### 2. No copy/context menu actions in aggregation results
| | |
|-|-|
| **Impact** | Can't copy values from aggregation results |
| **Location** | `src/views/documents/tree/tree_row.rs:206` |
| **Evidence** | Read-only tree row doesn't attach any context menu or copy actions |
| **Fix** | Add context menu to `render_readonly_tree_row` |

#### 3. Aggregation runs can race and overwrite newer results
| | |
|-|-|
| **Impact** | Slow runs can complete out of order and overwrite newer results |
| **Location** | `src/state/commands/aggregation.rs:134` |
| **Evidence** | Document loads use `request_id` check (`documents/query.rs:56`, `:92`). Aggregation has no equivalent guard |
| **Fix** | Add `aggregation_request_id` to session state |

#### 4. Read-only safety and destructive-stage safety not enforced
| | |
|-|-|
| **Impact** | `$out`/`$merge` can execute on read-only connections |
| **Location** | `src/state/commands/aggregation.rs:15`, `operators.rs:12` |
| **Evidence** | Write commands check `ensure_writable` (`commands/mod.rs:12`). Aggregation doesn't check this |
| **Fix** | Block `$out`/`$merge` or check `ensure_writable()` before execution |

#### 5. `$out`/`$merge` pipelines will often fail in preview mode
| | |
|-|-|
| **Impact** | Pipelines with write stages fail |
| **Location** | `src/state/commands/aggregation.rs:326`, `src/connection/mongo.rs:364` |
| **Evidence** | Runner appends `$count` and connection layer appends `$limit`. Both violate MongoDB requirement that `$out`/`$merge` be last |
| **Fix** | Detect write stages and skip appending, or run without limit/count |

---

### P1 - High (Medium Gaps)

#### 6. Errors can go stale after edits
| | |
|-|-|
| **Impact** | Confusing UX - old errors remain visible |
| **Location** | `src/state/app_state/sessions/model.rs:423` |
| **Evidence** | `set_pipeline_stage_body` resets counts/results but leaves `error` untouched |
| **Fix** | Clear `error` in mutation functions |

#### 7. Aggregation ignores any existing filter/sort/projection context
| | |
|-|-|
| **Impact** | Aggregation runs independently of active filter |
| **Location** | `src/state/commands/aggregation.rs:25` |
| **Evidence** | Command only uses aggregation stages and pagination |
| **Fix** | Option to prepend `$match` from active filter |

#### 8. Operator coverage is limited and there is no free-form operator input
| | |
|-|-|
| **Impact** | Advanced users need free-form entry |
| **Location** | `src/views/documents/views/aggregation/operators.rs:6` |
| **Evidence** | Only 23 operators supported, dropdown-only selection |
| **Missing** | `$changeStream`, `$collStats`, `$currentOp`, `$indexStats`, `$listSessions`, `$planCacheStats`, `$redact`, `$search`, `$setWindowFields`, etc. |
| **Fix** | Add "Custom" option for arbitrary operator names |

---

### P2 - Medium (Smaller Gaps)

#### 9. Limit validation is permissive
| | |
|-|-|
| **Impact** | Negative limits accepted, silently fall back to 50 |
| **Location** | `src/views/documents/views/aggregation/mod.rs:150`, `aggregation.rs:106` |
| **Fix** | Validate and show error for invalid limits |

#### 10. Aggregation results not integrated with selection/copy action system
| | |
|-|-|
| **Impact** | Aggregation results don't support document-level actions |
| **Location** | `src/views/documents/views/aggregation/results_view.rs` |
| **Evidence** | Documents copy actions operate on session-backed documents and node meta, which aggregation results don't populate |
| **Fix** | Use shared tree infrastructure with document editor |

---

## Refactoring & Scaling Risks

### Performance Issues

#### 1. Workspace persistence triggered on every keystroke
| | |
|-|-|
| **Impact** | I/O on every character typed in stage editor |
| **Location** | `src/state/app_state/sessions/model.rs:423`, `workspace.rs:31` |
| **Flow** | `InputEvent::Change` → `set_pipeline_stage_body` → `update_workspace_session_view` → disk write |
| **Fix** | Debounce workspace saves (300-500ms) or save on blur/run only |

#### 2. Stage stats are expensive: O(n) database calls plus repeated parsing
| | |
|-|-|
| **Impact** | n+1 count queries per aggregation run, slow for many stages |
| **Location** | `src/state/commands/aggregation.rs:273`, `:292` |
| **Evidence** | Loop runs count query per stage, rebuilds and reparses pipeline for every stage |
| **Fix** | Use `$facet` to get all counts in one query, or make stats optional |

#### 3. Stage list clones full stage bodies on render
| | |
|-|-|
| **Impact** | Large JSON bodies cloned for every render, memory pressure |
| **Location** | `src/views/documents/views/aggregation/stage_list.rs:362` |
| **Evidence** | Stage list only needs operator/enabled/counts but clones entire stage vector |
| **Fix** | Clone only metadata (operator, enabled, counts) for list display |

---

### Architecture Issues

#### 4. No request-id gating or cancellation for long-running aggregation runs
| | |
|-|-|
| **Impact** | Slow runs complete out of order; can't abort long runs |
| **Location** | `src/state/commands/aggregation.rs:134` vs `documents/query.rs:92` |
| **Fix** | Add abort channel, check between stages |

#### 5. Connection layer appends `$limit` unconditionally
| | |
|-|-|
| **Impact** | Changes pipeline semantics, breaks `$out`/`$merge` |
| **Location** | `src/connection/mongo.rs:364` |
| **Fix** | Pass flag to skip `$limit` for aggregations with write stages |

#### 6. Document tree logic duplicated in reduced read-only form
| | |
|-|-|
| **Impact** | Maintenance burden, inconsistent behavior |
| **Location** | `src/views/documents/views/aggregation/results_view.rs:332`, `tree_row.rs:206` |
| **Evidence** | Uses `render_readonly_tree_row` which is a stripped-down copy |
| **Fix** | Parameterize main tree component with `readonly: bool` |

#### 7. State reset logic is scattered and inconsistent
| | |
|-|-|
| **Impact** | Inconsistent field clearing leads to stale state |
| **Location** | Various functions in `src/state/app_state/sessions/model.rs:423` and nearby |
| **Evidence** | Different mutations clear different subsets of fields |
| **Fix** | Centralize reset logic in single function |

#### 8. No aggregation-specific tests
| | |
|-|-|
| **Impact** | Regressions in core logic |
| **Missing** | `build_pipeline` construction, stage-counting behavior, drag/drop index math, operator validation, pipeline import parsing |
| **Fix** | Add unit tests for core logic |

---

## Summary Tables

### Missing Features from Document Editor

| Feature | Priority | Effort | Notes |
|---------|----------|--------|-------|
| Find in results (Cmd+F) | P0 | Medium | Extend find handler |
| Copy actions (Cmd+C, context menu) | P0 | Medium | Add to read-only tree |
| Request-id gating | P0 | Low | Add counter to state |
| Clear errors on edit | P1 | Low | Clear in mutations |
| Dirty state tracking | P2 | Medium | Add visual indicator |
| Inline editing of simple values | P3 | High | Significant work |

### Safety Features

| Feature | Priority | Effort | Notes |
|---------|----------|--------|-------|
| Read-only connection check | P0 | Low | Call `ensure_writable()` |
| Block $out/$merge in preview | P0 | Low | Detect and skip append |
| Scope control for write stages | P1 | Medium | Add confirmation |

### Performance Fixes

| Fix | Priority | Effort | Notes |
|-----|----------|--------|-------|
| Debounce workspace saves | P0 | Low | 300ms debounce |
| Optimize stage stats | P1 | Medium | $facet or optional |
| Clone only metadata for list | P2 | Low | Struct change |
| Add request cancellation | P2 | Medium | Abort channel |

---

## Suggested Fix Order

### Phase 1: Critical Safety & Correctness
1. Stop disk writes on every keystroke (debounce or persist on blur/run)
2. Add request-id gating for aggregation runs
3. Handle `$out`/`$merge` safely (block in preview mode, guard read-only, avoid appending stages after them)
4. Clear errors when stage is edited

### Phase 2: Feature Parity
5. Add find/search to aggregation results (restore copy/search parity by reusing more of the documents tree model)
6. Add copy/context menu to aggregation results
7. Add "Custom" operator option for free-form entry
8. Validate limit input (reject negative, show errors)

### Phase 3: Optimization
9. Make stage counts optional or cheaper (use `$facet`, avoid reparsing per stage)
10. Clone only stage metadata for list rendering
11. Reuse document tree component for results (parameterize with readonly flag)
12. Add cancellation for long-running aggregations

### Phase 4: Polish
13. Add dirty state tracking with visual indicator
14. Option to prepend filter as `$match`
15. Add more operators to picker
16. Add unit tests for core logic

---

## Files Changed in These Commits

| File | Lines Changed | Purpose |
|------|---------------|---------|
| `views/aggregation/stage_list.rs` | +1495 | Stage list UI, drag/drop, operator picker |
| `views/aggregation/results_view.rs` | +403 | Results tree, pagination |
| `views/aggregation/stage_editor.rs` | +239 | JSON editor for stage body |
| `views/aggregation/mod.rs` | +353 | Main aggregation view layout |
| `views/aggregation/operators.rs` | +26 | Operator definitions |
| `state/commands/aggregation.rs` | +347 | Aggregation execution logic |
| `state/app_state/sessions/model.rs` | +251 | Pipeline state management |
| `views/documents/actions.rs` | +328 | Keyboard shortcuts |
| `views/documents/state.rs` | +200 | View state for aggregation |
| `views/documents/tree/tree_row.rs` | +114 | Read-only tree row |
