# OpenMango Features

GPU-accelerated MongoDB Desktop Client (macOS)

## Priority Legend

| Level | Target | Description |
|-------|--------|-------------|
| P0 | MVP | Core functionality - app unusable without |
| P1 | v1.0 | Expected for first public release |
| P2 | v1.x | Enhanced experience |
| P3 | Future | Nice to have, power user features |

---

## Known gaps (current build)

Non-exhaustive highlights; see each section below for full status.

- No import/export flows (data or connections)
- No aggregation pipeline editor, query history, or saved queries
- Bulk ops limited (paste insert + delete + update/replace)
- No multi-window/split views

---

## 1. Connections & Sessions

- [x] P0: Add connection (URI input with validation)
- [x] P0: Test connection (verify before save)
- [x] P0: Connect / Disconnect
- [x] P0: List databases on connect
- [x] P0: Connection status indicator
- [x] P0: Connection error feedback
- [x] P1: Edit connection
- [x] P1: Remove connection
- [x] P1: Connection profiles (auth, TLS, timeouts)
- [x] P1: Read-only mode / safe mode
- [ ] P2: Favorites / Tags
- [ ] P2: Import/Export connections
- [ ] P2: Health ping / latency
- [ ] P3: Connection groups / workspaces
- [ ] P3: SSH tunnel support

## 2. Databases

- [x] P0: List collections
- [x] P0: Create database
- [x] P0: Drop database (with confirmation)
- [x] P0: Refresh databases
- [x] P0: Database stats
- [ ] P1: Rename database
- [ ] P1: Copy database
- [ ] P2: User/Role management

## 3. Collections

- [x] P0: List with document counts
- [x] P0: Create collection
- [x] P0: Drop collection (with confirmation)
- [x] P0: Rename collection
- [x] P0: Collection stats
- [x] P0: Open document browser
- [x] P0: Refresh documents
- [ ] P1: Collection stats (extended)
- [x] P1: Index management (list/drop)
- [x] P1: Index management (create)
- [ ] P1: Import (JSON/CSV)
- [ ] P1: Export (JSON/CSV/BSON)
- [ ] P2: Schema analysis / explorer
- [ ] P2: Validation rules editor

## 4. Documents

- [x] P0: Find with filter
- [x] P0: View document (JSON)
- [x] P0: Edit document (inline + JSON editor)
- [x] P0: Delete document (with confirmation)
- [x] P0: Pagination
- [x] P0: Insert document
- [x] P1: Sort
- [x] P1: Projection (field selection)
- [x] P1: Duplicate document
- [x] P1: Paste document(s) from clipboard (JSON array / NDJSON)
- [x] P1: Bulk delete (filtered/all)
- [x] P2: Bulk update/replace
- [x] P2: Client-side find in results (Cmd/Ctrl+F)

## 5. Query & Aggregation

- [x] P0: JSON filter input
- [ ] P1: Aggregation pipeline editor
- [ ] P1: Stage preview / explain plan
- [ ] P1: Query history
- [ ] P1: Saved queries
- [ ] P2: Visual query builder
- [ ] P3: AI query assistant

## 6. Workspace & UI

- [x] P0: Tabs with preview/permanent behavior
- [x] P0: Per-tab session state
- [x] P0: Restore last session (auto-connect, tabs, selection)
- [x] P0: Restore filters and expanded nodes
- [x] P0: Restore window size/position
- [x] P0: Context menus (connection/db/collection/document)
- [x] P0: Destructive action confirmations
- [x] P0: Status bar
- [x] P0: Error display (banner)
- [x] P3: Keyboard navigation
- [ ] P1: Split views / side-by-side tabs
- [ ] P2: Theming
- [ ] P2: Keymap customization
- [ ] P3: Plugin system

## 7. Admin & Security

- [ ] P1: Connection-level permissions view
- [ ] P1: Audit log / operation log
- [ ] P2: Field-level masking
- [ ] P2: Secrets vault integration (Keychain)

## 8. Automation & Collaboration

- [ ] P1: Import/Export presets
- [ ] P1: Task scheduler (recurring imports/exports)
- [ ] P2: Shareable workspaces / team settings
- [ ] P3: Data compare & sync
- [ ] P3: SQL-to-Mongo query mode
