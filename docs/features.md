# OpenMango Features & Roadmap

Snapshot date: 2026-02-21
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
- Transfer workflows (import/export/copy JSON/CSV/BSON, progress)
- Aggregation pipeline editor (stage flow, preview, results)
- Forge query shell with completion/schema sampling
- Tabbed workspace restore + keyboard-heavy navigation

## Missing / Needs Implementation

### Query & Performance

- [ ] P0: Explain plan UI (winning plan, scanned docs, stage costs)
- [ ] P0: Index hinting and "why query is slow" diagnostics
- [ ] P1: Query history (per tab/session) with restore
- [ ] P1: Saved query snippets/templates

### Schema & Data Quality

- [ ] P0: Schema explorer (field cardinality, type drift, outliers)
- [ ] P1: Validation rule editor (JSON Schema / validator)
- [ ] P2: Data profiling reports (null %, distinct count, min/max)

### Operations & Automation

- [ ] P1: Task presets for transfer operations
- [ ] P1: Scheduler for recurring import/export/copy
- [ ] P2: Compare & sync between collections/query results
- [ ] P2: Dry-run mode with impact summary before write

### Connectivity & Security

- [ ] P0: Connection import/export (redacted + encrypted options)
- [ ] P1: SSH tunneling and proxy-aware connection flow
- [ ] P1: Secrets integration (Keychain-backed storage policy)
- [ ] P2: Field-level masking workflows for export/share

### Observability

- [ ] P1: Live server health panel (ops/sec, connections, network)
- [ ] P1: Per-operation timeline/log for long-running jobs
- [ ] P2: Change stream viewer (watch collection changes)

### UX / Workflow

- [ ] P1: Split view (side-by-side tabs/collections)
- [ ] P1: Tab pinning/grouping and better large-workspace ergonomics
- [ ] P2: Keymap customization and command palette expansion
