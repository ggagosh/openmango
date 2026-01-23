# Next Feature Analysis: Most Useful Addition

## Recommendation: Aggregation Pipeline Editor

The **Aggregation Pipeline Editor** (features.md Section 5, Priority P1) is the most valuable unimplemented feature for OpenMango.

## Rationale

### 1. Core MongoDB Workflow
Aggregation pipelines are how users perform reporting, data transformation, and complex queries. Without a pipeline editor, users must drop to `mongosh` for anything beyond basic `find()` operations. This represents the single largest gap in daily usability.

### 2. Competitive Necessity
The pipeline editor is the defining feature of MongoDB GUI tools (Studio 3T, Compass). It's what justifies using a desktop client over the shell. No serious MongoDB GUI ships without it.

### 3. Existing Infrastructure Ready
The codebase already provides the building blocks:
- JSON filter input and validation (documents view)
- Session-per-tab state management (SessionState/SessionData)
- Document tree rendering + JSON editor
- Async command pattern (`state/commands.rs`)
- MongoDB connection layer (`connection/mongo.rs`) - needs `aggregate()` method added

### 4. High Complexity Made Accessible
Pipelines with `$lookup`, `$unwind`, `$group`, `$facet`, etc. are notoriously hard to debug in text form. A stage-by-stage preview showing intermediate results at each step is enormously valuable for both learning and productivity.

## Implementation Outline

1. **Backend**: Add `aggregate()` to `connection/mongo.rs`
2. **State**: New `PipelineSession` or extend `SessionData` with pipeline stages
3. **UI**: Pipeline stage list (add/remove/reorder stages)
4. **UI**: Per-stage JSON editor with autocomplete for `$` operators
5. **UI**: Stage preview panel showing intermediate results
6. **Command**: `RunPipeline` command that executes and streams results
7. **Event**: `PipelineResult` / `PipelineError` events for status bar

## Runner-Up: Import/Export (JSON/CSV)

Import/Export is the second most impactful gap. Users frequently need to move data in/out of collections, and there's currently no path for this. However, the aggregation editor has broader daily-use impact since it's used in every working session, whereas import/export is periodic.

## Current State Summary

All P0 (MVP) features are complete:
- Connections, CRUD, pagination, filtering, sorting, projections
- Bulk operations, index management, session restoration
- Tabs, context menus, status bar, error handling

The app is fully functional for basic MongoDB browsing and editing. The aggregation pipeline editor is the natural next step to elevate it from a data browser to a complete development tool.
