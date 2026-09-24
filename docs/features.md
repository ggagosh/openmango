# OpenMango Features & Roadmap

Snapshot date: 2026-07-22
Audience: power users and small engineering teams

## Priority Legend

- P0: Must-have for everyday reliability and speed
- P1: Core v1 capabilities expected by advanced users
- P2: Productivity and quality-of-life multipliers
- P3: Strategic/long-tail

## Already Strong

- Connection manager (add/edit/test/connect/disconnect)
- Database/collection CRUD + stats
- Document browse/edit (inline + detached JSON editor)
- Sort/projection/pagination/filter
- Bulk document ops
- Index create/list/drop
- Transfer workflows (import JSON/CSV/BSON; export JSON/CSV/BSON/Excel; copy with progress)
- Aggregation pipeline editor (stage flow, preview, results)
- Forge query shell with completion/schema sampling
- Tabbed workspace restore + keyboard-heavy navigation

## Missing / Needs Implementation

### Query & Performance

- [x] P0: Explain plan UI (winning plan, scanned docs, stage costs)
- [ ] P0: Index hinting and "why query is slow" diagnostics
- [x] P1: Query history across Documents, Aggregation, and Forge with restore
- [x] P1: Saved query snippets with metadata, tags, global scope, and portable import/export

### Schema & Data Quality

- [x] P0: Schema explorer (field cardinality, type drift, outliers)
- [ ] P1: Validation rule editor (JSON Schema / validator)
- [ ] P2: Data profiling reports (null %, distinct count, min/max)

### Operations & Automation

- [x] P1: Task presets for transfer and compare operations, with Run now and an encrypted run history ([plan](TASKS_PLAN.md))
- [x] P1: Scheduler for recurring import/export/copy and compare/sync tasks, also while OpenMango is closed on macOS, Windows and Linux ([plan](TASKS_PLAN.md))
- [x] P2: Read-only collection comparison with custom match keys, filters, BSON differences, and document inspection
- [x] P2: Selective sync and guarded session undo from comparison results; MongoDB 8.0+ write targets (see [plan](COMPARE_SYNC_PLAN.md) and [benchmarks](COMPARE_BENCHMARKS.md))
- [x] P2: Compare two documents picked in a collection view
- [x] P2: Compare two databases collection by collection, with index differences, copying of one-sided collections, and sync by collection (Add missing, Add and update, Mirror) with one undo ([plan](COMPARE_DATABASE_PLAN.md))
- [x] P2: Ignore array order and single-field copy with undo in the compare document diff
- [x] P2: Read-only MCP compare tools, run as MCP tasks for clients that support them
- [x] P2: Sync two databases by collection: add missing, add and update, or mirror, with one undo for the run
- [ ] P2: Dry-run mode with impact summary before write

### Connectivity & Security

- [x] P0: Connection import/export (redacted + encrypted options)
- [x] P1: SSH tunneling and proxy-aware connection flow
- [x] P1: Secrets integration (versioned Keychain-backed credential bundles)
- [x] P1: Explicit connection environments with optional Production write confirmation
- [ ] P2: Field-level masking workflows for export/share

### Transfer format notes

- Excel export is available for collection results and multi-sheet reports.
- Excel exports flatten nested fields into columns and inspect the complete result set before writing.
- Unsupported Excel row/string sizes and fields discovered after schema inspection are reported as errors; data is not silently truncated.
- BSON import/export requires MongoDB Database Tools and reuses the active SSH or SOCKS5 transport.

### Observability

- [ ] P1: Live server health panel (ops/sec, connections, network)
- [ ] P1: Per-operation timeline/log for long-running jobs
- [ ] P2: Change stream viewer (watch collection changes)

### UX / Workflow

- [ ] P1: Split view (side-by-side tabs/collections)
- [ ] P1: Tab pinning/grouping and better large-workspace ergonomics
- [x] P2: Keymap customization and command palette expansion
