# Changelog

All notable changes to OpenMango will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Connection import/export — back up, share, or migrate your saved connections as JSON. Three modes: Redacted (passwords stripped, safe to share), Encrypted (passwords locked with a passphrase via AES-256-GCM), or Plaintext. Import auto-renames duplicates and prompts for the passphrase when opening encrypted files.
- Schema Explorer tab — analyzes your collection's structure by sampling documents, showing a searchable field tree with types, presence rates, cardinality, polymorphism detection, and an inspector panel with charts and sample values
- Automatic background updates — new versions download silently and are ready to install on restart, VS Code style. Disable in Settings > Updates.
- Periodic update re-checks every 4 hours for long-running sessions.
- Multi-document selection in document lists.
- JSON editing now opens in a dedicated editor window, so you can browse and copy data while editing.
- JSON editor productivity shortcuts: move line, duplicate line, delete line, join lines, toggle comment, and format document.
- Clear inline status messages in the JSON editor for format/save/insert actions.
- Explain for queries and aggregation pipelines — click "Explain" next to Run to see the execution plan as a visual tree or raw JSON, with stage-level stats, index usage, cost indicators, and optimization suggestions.

### Fixed
- Re-opening Edit/Insert now focuses the existing editor window instead of creating duplicates.
- `Cmd/Ctrl+W` now closes the editor window instead of the main app tab.
- Save and Insert now close the editor window after a successful operation.
- Safer document saving: detects changed/deleted server documents and unapplied inline drafts, with recovery actions (`Reload`, `Load Inline Draft`, `Create as New`).
- Query text no longer clears when switching tabs.
- Preview tabs now promote/restore more consistently, including after restart.
- Inline field-edit save flow is more reliable.
- Typing around auto-paired characters in Forge is smoother.
- Format JSON no longer mangles non-English text (Georgian, Japanese, and other multi-byte characters come through intact now).
- "Create as New" actually creates a new document instead of failing with a duplicate key error every time.
- Typing non-English characters in query inputs no longer crashes the app.
- BSON export/import no longer fails when the connection URI contains a database name (e.g. `/admin` for auth) that differs from the target database.

### Changed
- Tab switching is noticeably snappier — workspace state now saves with a debounce instead of blocking the UI on every switch
- Switching back to a previously-visited collection tab restores the document tree instantly from cache instead of rebuilding it from scratch
- Fewer unnecessary re-renders when switching tabs
- JSON editor window titles are now clearer and more descriptive.
- Clear shortcut for Forge output and aggregation stage is now `Cmd/Ctrl+Alt+K`.

## [0.1.7] - 2026-02-12

### Added
- Smart query inputs for filter, sort, and projection with autocomplete for MongoDB operators (`$gt`, `$in`, `$regex`, etc.) and field names from loaded documents
- Auto-closing brackets, braces, and quotes in query inputs and Forge editor
- JSON validation on query submit with red border and "invalid json" hint when invalid
- Shift+Enter in query inputs to insert newlines with auto-indentation between braces
- Tab key accepts autocomplete suggestions in all code inputs
- In-document search (Cmd/Ctrl+F) with case-sensitive, whole word, regex, and values-only modes
- Expand All / Collapse All buttons for document trees, aggregation results, and Forge results
- Drag-and-drop tab reordering with scroll wheel support for overflowing tabs
- Pinnable result tabs in Forge shell to keep important results across runs
- Search and Format JSON buttons in JSON editing dialogs
- Pagination for aggregation results
- Theme system with Vercel Dark and Darcula Dark themes, runtime switching
- Window vibrancy effect

### Fixed
- Collection data not loading / spinner stuck on empty collections
- SRV connection string resolution errors
- Password redaction in connection display
- Sidecar build for x86_64 release target
- Text overflow in JSON editor
- Forge shell spinner not appearing

### Changed
- Replaced "What's New" dialog with a scrollable changelog tab in the tab bar
- Switched sidecar runtime from Node.js to Bun
- Updated JSON editor font
- Preview tabs now shown in italic to distinguish from pinned tabs
- JSON dialogs now use soft-wrapped editors with line numbers
- Integration tests now share one MongoDB container per test binary instead of spawning one per test (121 → 9 containers), with UUID-namespaced databases for isolation
- Upgraded test MongoDB image from 5.0.6 (EOL) to 7.0 LTS
- Fixed MongoDB 7.0 compatibility in stats tests (`i64` field types, removed `indexDetails` option, `currentOp` admin-only enforcement)

## [0.1.6] - 2026-02-07

### Added
- Forge query shell (mongosh-compatible REPL per database)
- Transfer progress tracking for database-scope operations
- Aggregation pipeline list performance improvements

### Fixed
- Node sign display issues
- Forge shell state persistence
- Editor inline editing bugs
- Export/import edge cases

### Changed
- Major internal refactoring of editor and state management
- Custom fonts (KAPO)

## [0.1.5] - 2026-01-31

### Added
- Aggregation pipeline builder
- Import/Export/Copy transfer system (JSON, JSONL, CSV, BSON formats)
- Multi-connection support
- Bulk update operations
- Document key assignments
- Extended JSON support (Relaxed & Canonical modes)
- Action bar with common operations
- Cancel in-progress async operations
- Copy/paste for sidebar tree items

### Fixed
- Inline editing regressions
- Tab close behavior
- Expand/collapse state bugs

### Changed
- Major architecture refactor (session-per-tab model)

## [0.1.4] - 2026-01-20

### Added
- Connection manager
- Keyboard navigation for document tree
- Delete and paste operations
- Read-only mode for views

### Fixed
- Long text editing overflow

## [0.1.3] - 2026-01-18

### Added
- Error banner notifications
- Context menu actions for properties

## [0.1.2] - 2026-01-16

### Added
- Document search (Cmd+F)
- Index creation dialog
- Property-level actions (copy, add, delete)

## [0.1.1] - 2026-01-15

### Changed
- Initial improvements after first release

## [0.1.0] - 2026-01-15

### Added
- Initial release
- Connect to MongoDB and browse databases/collections
- Tree-based document viewer with expand/collapse
- Inline BSON value editing
- Pagination
- BSON syntax highlighting
