# Changelog

All notable changes to OpenMango will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Query Library for Documents, Aggregation, and Forge with successful-run history, saved queries, full-text search, restore/run/copy/update/delete actions, keyboard access, atomic local persistence, and credential-aware exclusion
- Optional connection colors that accent connections in the sidebar, connection manager, and tabs, and survive connection import/export
- Shared unsaved-change protection across tabs, detached editors, connection changes, workspace restore, app quit, theme restart, and updater relaunch
- Query failures now stay visible per tab with Retry and Copy Details actions while preserving the last successful result
- Configurable `maxTimeMS` and real cancellation for interactive document queries
- Settings now show the log location and can export a redacted support bundle with runtime diagnostics
- AI privacy controls for selected-document and automatic sample sharing, both disabled by default
- Keyboard-operable app buttons with focus rings and Enter, Return, and Space activation
- Accessibility QA documentation and focus restoration for searches and confirmation dialogs
- Table view for documents — browse collections in a spreadsheet-style grid with sortable, resizable, and pinnable columns
- Per-page selector in the pagination bar — choose between 10, 25, 50, or 100 documents per page
- Islands tab style — choose between Islands, Segmented, or Underline tab appearance in Settings
- Tab icons — every tab now shows an icon for its content type (collection, database, forge, settings, etc.)
- Icons in context menus throughout the app (document actions, connection menu, field operations)
- AI sample_values tool — the AI assistant can now inspect real field values to give better answers
- Column pinning — pin frequently-used columns to the left so they stay visible while scrolling
- Fast collection filters — type compact filters like `status:active age>30` instead of writing full MongoDB JSON
- Smart filter value conversion — ObjectId fields accept bare 24-character IDs, and date fields accept shortcuts like `today`, `last7d`, `2026-05-23`, `2026-05`, `2026Q2`, and explicit ranges like `2026-05-01..2026-05-31`
- Filter autocomplete now suggests field names, MongoDB constructors like `ISODate(...)` and `ObjectId(...)`, and date shortcuts after fast-filter operators
- Reload a database to refresh its collection list from the server without reconnecting

### Fixed
- Workspace restore no longer crashes by re-entering the sidebar while a connection event is being handled
- Workspace restore now waits for saved connection credentials to finish loading from Keychain before reconnecting
- Import and copy Clear/Drop operations now stage changes before atomic promotion, and Replace preserves failed originals while reporting partial progress
- Application read-only mode now blocks every app-owned write path, including AI and Forge, while destructive operations require frozen, revalidated confirmations
- Connection credentials now use versioned Keychain bundles; saved configuration and process arguments no longer expose URI secrets
- BSON import/export cancellation now terminates and waits for `mongodump` or `mongorestore`, and stale transfer completions are ignored
- Transfer filter, projection, and sort parsing now fails closed with field-specific errors instead of silently broadening queries
- JSON, CSV, Excel, report, aggregation, database-scope, and BSON exports now stage output atomically and preserve existing destinations on failure or cancellation
- CSV and Excel exports discover the complete schema and report late fields, row limits, string limits, and failed batches instead of silently dropping data
- Bulk Replace now performs ordered per-document replacements, preserves `_id`, supports cancellation, and reports exact partial execution
- Index replacement validates before dropping, restores the previous index on failure, and collection copy preserves supported index metadata
- Forge and BSON tools now reuse active SSH and SOCKS5 transports with their TLS and authentication options
- Query refresh now cancels actual client/server work rather than relying only on stale request IDs
- Numbered-tab, content-focus, document, and aggregation shortcuts no longer conflict; palette and menu shortcuts come from registered actions
- Palette Refresh now follows the same context-sensitive path as Cmd/Ctrl+R, AI opening focuses its input, and Forge preserves the selected collection
- Unit-test CI now runs the library tests instead of the zero-test binary target
- Search in JSON editors now wraps correctly in both directions — pressing Enter cycles forward through all matches, Shift+Enter cycles backward
- Detached editor windows now inherit the vibrancy setting from the main window instead of always appearing opaque
- Closing the main window now also closes all detached editor windows
- Cmd+W works reliably for successive tab closes — previously only the first press worked, then the shortcut stopped responding
- Table column order is now deterministic — columns sort alphabetically (_id always first) instead of depending on document key insertion order
- Table column widths no longer jump around when sorting or paginating — widths lock in on first render
- Explain modal no longer shows content scrolling behind it — backdrop opacity increased and scroll events are properly blocked
- Explain modal header is no longer semi-transparent
- Filter, sort, and projection inputs now have JSON syntax highlighting
- Sidebar typeahead now works regardless of which node type is selected (previously only worked with databases selected)
- Typeahead indicator dismisses on Enter (opens selection), Escape, and auto-clears after 1 second of inactivity
- Backspace deletes characters from the typeahead query
- Typeahead no longer opens collections during type-ahead — it only highlights; Enter opens
- Typeahead prefix match now stays on the current selection while the query still matches instead of jumping between similar names
- Pressing Backspace with the typeahead indicator active no longer triggers the delete confirmation dialog
- Preview tabs restored — single-clicking a collection opens an italic preview tab that gets replaced on the next click, matching VS Code behavior; previously every click opened a new permanent tab
- Arrow keys now work in the sidebar tree after clicking a collection (previously stopped responding due to focus loss)
- Applied fast filters now keep the text you typed instead of rewriting it into MongoDB JSON

### Changed
- Updates now require published SHA-256 assets, verify the downloaded archive and macOS code signature, respect the automatic-update preference, and install only after Restart to Update
- Update-check failures remain visible with Retry instead of silently returning to idle
- AI enablement now discloses the workspace metadata sent to the selected provider, and complete system prompts are no longer written to debug logs
- Transfer jobs that continue after errors retain failure counts, per-collection details, and processed-document totals
- Release workflows now publish per-archive SHA-256 checksum assets
- Filter bar redesigned — filter stays primary with parsed readback chips, while sort and projection live in the Options panel
- AI chat panel moved out of the documents view into its own dedicated space
- Close buttons on tabs now only appear on hover (except the active tab)
- Tab bar styling updated with padding and theme-aware background

### Security
- Existing files, collections, and databases remain unchanged until destructive imports, copies, and exports complete successfully
- Write confirmations include the exact connection, namespace, filter or pipeline, current count, and frozen options being approved
- Plaintext credential export is disabled; connection export is redacted or passphrase-encrypted
- Update archives and final app bundles are verified before replacing the installed application

### Performance
- Document tree (JSON view) expands and scrolls much faster on large or deeply nested documents — removed a quadratic dirty-check and the redundant full-tree clones that ran on every interaction
- Documents table is much smoother — it now re-renders only when the data or selection actually changes instead of rebuilding every visible cell every frame
- Aggregation results, schema view, and in-document search no longer redo expensive work (deep document clones, regex compilation, full schema re-walks) on every frame
- Sidebar search is much faster — results are cached and recomputed only when the query or the connection/database/collection list changes
- Per-collection caches are now freed when a tab closes, so memory no longer grows as you browse through many collections
- Copying a large multi-document selection no longer briefly freezes the UI

## [0.2.0] - 2026-03-05

### Added
- AI chat assistant with multi-provider support (OpenAI, Anthropic, Google, Ollama)
- MongoDB-aware tool calls: find, aggregate, insert, update, delete, explain, indexes, schema inference, collection stats, and more
- Rich response blocks: data tables, charts, stats, and query previews
- AI completion suggestions in the documents view
- Secure API key storage via macOS Keychain
- Token budget tracking and safety guardrails for AI operations
- Model registry with per-provider model selection
- AI provider settings UI in the settings view
- Workspace and tab persistence for AI chat sessions
- Collection metadata command for AI context enrichment

## [0.1.8] - 2026-02-25

### Added
- SSH tunnel support — connect to MongoDB through a bastion host with password or identity file auth, strict host key checking, and configurable local bind address
- SOCKS5 proxy support — route connections through a SOCKS5 proxy with optional credentials
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
- Connection manager redesigned — 8 tabs consolidated to 4 (General, TLS, Network, Advanced), with a wider near-fullscreen dialog that gives fields more breathing room
- Pool & Timeouts and Compression settings are now tucked behind collapsible sections in the Advanced tab, keeping things clean until you need them
- Connection test now shows live progress steps instead of a generic spinner
- Tab switching is noticeably snappier — workspace state now saves with a debounce instead of blocking the UI on every switch
- Switching back to a previously-visited collection tab restores the document tree instantly from cache instead of rebuilding it from scratch
- Fewer unnecessary re-renders when switching tabs
- JSON editor window titles are now clearer and more descriptive.
- Clear shortcut for Forge output and aggregation stage is now `Cmd/Ctrl+Alt+K`.
- New app icons

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
