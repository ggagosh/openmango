# Changelog

All notable changes to OpenMango will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Compare collections from the collection menu or command palette: choose match fields, filter both sides, inspect missing and changed documents, distinguish number-type and field-order differences, and find duplicate keys. Comparisons can be cancelled, results remain readable when a connection closes, and setup is restored across restarts
- Selective collection sync with explicit target selection, row/category checkboxes, write review, native bulk operations, stale-document guards, cancellation between batches, and encrypted session undo. Sync and undo require MongoDB 8.0+ on the write target; older servers remain supported for comparison and as read-only sources. Undo expires when the tab closes or a new comparison starts
- Compare 2 Documents in the document menu: select two documents in a collection and see them field by field in the same view the collection comparison uses, with unsaved edits compared as shown
- The comparison's count of documents skipped for lacking the match key opens those documents on either side, filtered exactly as the scan skipped them
- Compare's connection pickers list every saved connection. Picking a closed one opens it without leaving the tab, shows progress and any error under the picker, and a restored comparison offers Connect for connections that are closed
- Ask AI in the documents filter: the sparkle turns the filter bar into a bar you describe the filter to, Find becomes Generate, and what you typed comes back as the filter in the same box, written from the collection's own field names, types and — for fields that hold a handful of values — examples of those values. A description that asks for an order or for particular fields fills Sort and Projection too and opens the options row to show them. Nothing runs until you press Find, Escape gives back the filter you had, and undo takes it back after that. Cmd/Ctrl+I switches the bar either way without reaching for the mouse
- Connection switcher on the sidebar's Connections header and on Cmd/Ctrl+Shift+K, listing open connections first and saved ones by most recent use
- Recent connections on the welcome screen, one click each, with progress shown on the one being opened
- Multiple cursors in Forge and the query editors: Alt-click adds a cursor and Shift-Alt-drag selects a column
- Backspace between an empty bracket or quote pair removes both, and single quotes close automatically in code editors
- The command palette also opens with Cmd/Ctrl+Shift+P, lists recently used commands first, finds commands by related words such as "dump" for Export Data, and narrows the search to databases and collections when it starts with `#` or to connections with `@`
- Mango Dark and Mango Light themes in the openmango.app colors, listed first in each group, with every text color meeting WCAG AA contrast on the surfaces it appears on
- Match system appearance in Settings and in the command palette's theme list switches between Mango Dark and Mango Light with the system's dark or light mode; choosing a theme turns it off
- AI models come from a models.dev catalogue: Fast, Balanced and Powerful presets per provider, a searchable picker that shows each model's context size and price, and a Refresh that fetches the latest list; a snapshot ships with the app so the picker is right offline
- OpenRouter as an AI provider, offering its whole searchable catalogue of tool-calling models instead of presets
- The assistant remembers a conversation between runs, and can search earlier conversations when you refer to work you did before. Conversations are kept in an encrypted database with a key from the system keychain, and nothing else on disk holds them: not the workspace file, not the log. Tool results, which hold your data, are never written down at all. Conversations are deleted after 30 days, and Settings can turn the memory off or delete everything it has kept
- Every answer shows what it cost in tokens and in money at the model's list price, and can be copied; an answer that failed can be tried again. The chat header keeps the running total for the whole conversation
- Tool calls in the chat carry an icon for the tool that ran, and a collapsed group shows which tools it used
- New chat and a list of conversations in the chat header, the open one marked: starting over keeps what came before, and any of the last 20 conversations can be reopened where it left off. Each one is named by the model from its first exchange, says when it was, how many questions were asked and what it spent, and can be deleted from the list
- Clear chat has a shortcut of its own while the chat has focus, and the chat's buttons show the keys that trigger them
- Follow a reference: Cmd/Ctrl+click an ObjectId, or press Cmd/Ctrl+B or F12 on its row, and the tab moves to the document it points at, with a real filter you can edit. Where it points is confirmed against the server before anything moves, remembered for next time, and an id that leads nowhere says so instead of opening an empty collection. The arrow beside an id, or Space, shows the document in place without leaving; a DBRef goes where it names
- Tabs go back and forward like a browser's: Cmd/Ctrl+[ returns to the view you followed a reference from, with its page, scroll, selection and filter as you left them and without asking the server again, and a trail under the collection name shows the way you came. Cmd/Ctrl+Shift+click follows into a new tab, and a tab with unsaved edits is never navigated away from
- Open in new tab in the sidebar's collection menu and on Cmd/Ctrl+Shift+Enter, so one collection can be open twice with two different filters
- Find references: Cmd/Ctrl+click a document's `_id`, or press Shift+F12, to see every document that points at it, grouped by the field that does. Each group can be opened as an ordinary filter, a row opens that document, and on a production or protected connection a field with no index waits for you to run it
- Infer relations, in a database's tab and in a collection's menu: reads a sample of each collection one at a time, finds the fields that hold ids, and confirms where they point with index-only lookups, including references a collection makes to itself. It reports what it read, what it could not read and which fields matched nothing, can be stopped between collections, and what it learns is kept per database name, so it carries from a local copy to production
- Relations tab, from Open canvas in a database's tab: every collection as a card listing the fields that reference something, laid out in layers with each line routed between the cards. Point at a field to light its one line and the collection at the other end, or at a collection to light everything joined to it; click holds it, Escape lets go, and double-click opens the collection. Drag or scroll to pan, pinch or Cmd/Ctrl+scroll to zoom, and copy the whole graph as Mermaid or DBML
- A chip beside the collection name says whether its database has been read for relations, and either starts that or opens the canvas on this collection, counting what is new since you last looked
- Adding an aggregation stage offers Join a related collection, which writes the `$lookup` from a known relation and the `$unwind` when the join arrives at one document; From relation on a `$lookup` stage fills in its four fields
- Ids in aggregation results can be followed with Cmd/Ctrl+click, in the tree and the table, and an array of ids has Open all in its menu
- The assistant and MCP clients can ask what references what (`get_relations`) and how two collections join (`join_path`), and get the `$lookup` stages back. Over MCP the graph is given only for a database the shared connection has
- Dates can be shown in your local time: Show dates in, under Settings, a chip in the status bar that says `UTC` or your offset, and a command in the palette all switch it. A local date always carries its offset, such as `2024-01-31T13:30:00+04:00`, and everything copied or exported stays UTC whatever the setting
- A UUID shows as a UUID and other binary as its size, with the subtype in the type column, instead of a dump of bytes. A legacy UUID shows as stored, and says so
- Hovering a date or a binary value lists its other forms: UTC, local, how long ago and the epoch for a date; subtype, size and Base64 for binary, and the Java and C# byte orders for a legacy UUID
- Copy value as, on a date or binary field: `ISODate("…")`, UTC, local or epoch milliseconds for a date, and `UUID("…")`, the plain string, Base64 or Hex for a UUID
- Plain JSON under Copy as, where ObjectIds, dates and UUIDs are ordinary strings, for pasting somewhere that doesn't speak Extended JSON
- AWS IAM sign-in works: `MONGODB-AWS` was refused by the driver before. The authentication mechanism is picked from a list (Automatic, SCRAM, X.509, LDAP, AWS IAM) that explains what each one expects, and a mechanism typed into the URI that isn't on the list is kept
- The Indexes tab shows how often the server has used each index and since when, and marks one that nothing has used as Unused. The column is left out where the server won't report usage
- Views are told apart from collections: an eye icon in the sidebar, and in the tab a VIEW tag, a link to the collection it reads, and a READ-ONLY tag that says why. A time-series collection gets an icon of its own
- Save as view on the aggregation screen makes a view from the pipeline, offering a name read off what the pipeline does, such as `orders_open_by_country`, in the source collection's naming style, and taking an optional collation
- Edit view definition, in the sidebar menu and the view's header, opens the view's pipeline in the aggregation builder as the server holds it now. The builder says the view is up to date until a stage really changes, then offers Update view, which keeps the view's collation
- Duplicate view and Drop view in the sidebar menu; dropping says that the documents in the source collection are untouched
- Show system collections under Settings lists `system.*` collections in the sidebar, muted and last. They are hidden by default now that creating a view adds `system.views`; a collection that merely has "system" in its name is never hidden
- Escape cancels a drag, and the pipeline stage list, the filter builder and the tab bar scroll when a drag nears their edge

### Changed
- Filters are written with spaces inside their braces, `{ _id: ObjectId("…") }`, everywhere one is shown, copied or saved
- Stop ends a tool call that has already started instead of waiting for it to finish, and the rows it interrupted say so rather than spinning
- When a request fails, the chat says what to do about it: which key to check, which model to pick, or that it is a rate limit that will clear
- New installs match the system appearance with the Mango themes; a theme you already picked stays as it is
- The sidebar lists only open connections, shows a spinner in place of the icon while one connects, keeps the connection color on the icon, and reveals row actions on hover or selection
- Document values are plain text everywhere with one set of rules: numbers, `true`/`false`, ObjectId hex, dates such as `2024-01-31` or RFC 3339 timestamps, `null`, and mongosh forms like `ISODate("…")` or `NumberLong(42)`, replacing the switches and number steppers in inline tree editing, the edit value dialog, and the filter builder
- Enter opens the selected document, expanding it in the tree or opening it as JSON from the table, like double-click
- Workspace tabs from a colored connection share an underline in that color
- The edit value dialog submits with Cmd/Ctrl+Enter, and its Array type accepts mongosh syntax like Document does
- The Indexes tab shows keys as field and direction pairs and properties such as Unique, TTL, or Partial as tags, without disabled actions on the built-in `_id_` index
- The Create Index and Edit Index dialogs label every field, name key types, explain unavailable options where they apply, submit with Cmd/Ctrl+Enter, and describe how replacing an index works before you confirm
- The command palette shows shortcuts as keycaps, scrolls its whole list with a scrollbar, follows the mouse with one highlight, checks the current theme, names the open submenu with a back button (Backspace also goes back), and clears the search on the first Escape
- The assistant works a question through step by step instead of being told to stop after a few tool calls, keeps what its tools found across follow-up questions, and retries a request the provider rate-limited
- Save and Discard for unsaved document edits sit in the collection header with their keyboard shortcuts shown, and act on every unsaved document in the tab rather than only the selected ones
- After a query the status bar says how many documents were loaded, out of how many matched, and how long it took
- The Find button shows a spinner in place of its icon instead of pushing the row aside, and busy buttons keep their size
- A long run of tool calls shows only its last few while it works, with the rest one click away, instead of pushing the answer off the screen
- The status bar and the chat are built on gpui-kit's own components, so the chat scrolls, follows new messages and renders markdown the way the rest of the app does
- Headings in an answer are bigger than the text they introduce, field names in a sentence carry the same blue the document tree gives them, code blocks have room around them, and the answer no longer changes size the moment it finishes streaming
- The existing JSON copy format is named Extended JSON, now that Plain JSON sits beside it
- What you drag is the thing you grabbed, lifted off in place and held where you took hold of it: a key or value in its own color, a stage or condition by its grip, a tab as a tab. The hand closes while you drag, and the blue chips beside the pointer are gone
- In the tree, a drag starts on the key or value text rather than anywhere in its column
- Inserting, editing, deleting and bulk-updating documents in a view, and changing its indexes, are refused in the app with the name of the collection to change instead, rather than by a server error. Import is off for a view, and Rename is not offered
- A view's Indexes tab says it uses its source's indexes and links to them, and its Stats row says a view stores nothing of its own

### Removed
- The Vibrancy setting: windows are always opaque, so text keeps the same contrast whatever sits behind the window, and theme changes no longer ask for a restart

### Fixed
- Cmd/Ctrl+Enter in an aggregation stage runs the pipeline without also adding a line to the stage
- Closing a preview tab releases its session; it was kept alive until the app quit
- Cmd/Ctrl+F while typing in a query editor no longer opens the document search over the results
- The filter bar keeps a line for its message whether or not it has one, so a query that finishes in milliseconds no longer flashes "Searching collection…" and shifts the documents under it
- JSON, Insert and Refresh no longer grey out for the length of a query, which made the toolbar blink on every reload while Tree and Table stayed put
- Running a query no longer rewrites the filter, sort and projection inputs or closes the options row, which made the view blink on every Find
- The @collection list in the chat answers to the arrow keys, and Enter takes the highlighted collection instead of sending the half-typed name as a message
- Switching themes now reaches the layer that draws the chat's markdown, so a dark theme no longer renders answers with light tables and washed-out text
- Installing an update on macOS opened a second copy of OpenMango instead of replacing the running one
- The Indexes tab rendered every index side by side on a single line
- Error messages on the Indexes tab and in the index, edit value, and bulk update dialogs were drawn in a color that matched the background in most themes
- Number steppers in the filter builder and the index TTL field did nothing, and both are now plain inputs
- The command palette sat off-center and overflowed small windows, took Enter and arrow keys from other windows while open, and let Tab move focus behind it
- Shortcut hints in the command palette and sidebar tooltips showed the Ctrl variant on macOS
- Double-clicking a value in the document tree to edit it shifted the text and the rows below
- The Schema tab's field filter showed its text low, clipped, and indented behind an empty gutter; it now matches the documents filter
- Replacing documents with `many` now stops at the 100 it promises, instead of rewriting every document that matched the filter
- Inserting more than 100 documents is refused rather than quietly exceeding the limit the assistant was told about
- A field's value no longer shifts by a couple of pixels when it is marked as edited or selected, and a document's key no longer moves when it gets unsaved changes
- Two calls to the same tool in one answer keep their own results
- On a read-only connection the assistant is no longer told about write tools it does not have
- Stopping an answer says it stopped, instead of reporting a tool call limit
- The chat's text box starts the caret at the edge of the box, and grows as you type
- The model picker opens on the model you are using, and Settings says when the model list could not be loaded instead of showing "Ready"
- A group of tool calls can be collapsed while the assistant is still working, and no longer blinks open and shut between calls
- Dragging a date or boolean field's key into the filter builder made a condition that compared text against the field and matched nothing
- Dragging a UUID or other binary value into the filter builder filters on the value instead of on its JSON as text
- On a narrow window the collection header's buttons were drawn over the collection name. They now move below it, and the chips beside the name wrap
- The edit value dialog prefilled binary and decimal values with debug text instead of the Extended JSON it asks for

## [0.3.0] - 2026-09-14

### Added
- Windows support: per-user installers for x64 and ARM64 with Start menu integration and an uninstaller, signed in-app updates, no console windows for the app or its bundled tools, and credentials stored in Windows Credential Manager
- Linux support: AppImage builds for x86_64 and aarch64 with a desktop-entry install action, signed in-app updates, and a combined title bar matching the macOS window chrome
- Searchable native connection list, visible disconnected connections with row actions, and separate Save and Save & Connect actions
- Copy ID action for documents, including ID-only copying of a selected collapsed document
- Open a highlighted collection in Forge with Cmd/Ctrl+Shift+F, or choose Open Forge as the collection double-click action in Settings; queries start with `find({})`, ready to run ([#12](https://github.com/ggagosh/openmango/issues/12))
- Authenticated local MCP agent access with per-connection sharing and write controls, bounded read tools, direct document insert/update/replace/delete, and metadata-only History restore tools
- Native approval and Agent Activity workflows for Arcula database backups, syncs, and verified-backup reverts, including progress, cancellation, target fingerprints, and recovery interlocks
- Encrypted passive document History with change-stream capture, visible coverage gaps, retention controls, concise batch details, and conflict-safe resumable restores
- Saved-query descriptions, tags, global scope, and bounded versioned JSON import/export with atomic persistence and credential screening
- Explicit Development, Staging, and Production connection identity across the workspace, with optional fail-closed confirmation for Production writes and Forge execution
- Complete keyboard and command-palette coverage for Schema, Transfer and its query editor, Forge, document/index/aggregation workflows, tabs, and focus navigation, with a visible palette button
- Customizable keyboard shortcuts with search, context-aware conflict validation, recording, disable/reset controls, persisted overrides, and restart-safe application
- Query Library for Documents, Aggregation, and Forge with successful-run history, saved queries, full-text search, restore/run/copy/update/delete actions, keyboard access, atomic local persistence, and credential-aware exclusion
- Optional connection colors that accent connections in the sidebar, connection manager, and tabs, and survive connection import/export
- Shared unsaved-change protection across tabs, detached editors, connection changes, workspace restore, app quit, theme restart, and updater relaunch
- Query failures now stay visible per tab with Retry and Copy Details actions while preserving the last successful result
- Configurable `maxTimeMS` and real cancellation for interactive document queries
- Settings now show the log location and can export a redacted support bundle with runtime diagnostics
- AI privacy controls for selected-document and automatic sample sharing, both disabled by default
- Keyboard-operable app buttons with focus rings and Enter, Return, and Space activation
- Focus returns to where you were after closing searches and confirmation dialogs
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
- macOS updates no longer reject valid app signatures with "invalid requirement specification"
- Prevent a crash when opening New Connection or switching saved connections; retain drafts and active sessions when connection persistence fails
- Keep pasted URI options and encoded credentials in sync with the editor, and ignore connection test results after the tested settings change
- Workspace tabs now stay within the title bar, follow the active tab when overflowing, and accept shortcuts immediately after launch
- Query editors retain focus and place the caret correctly on left-click, including collapsed and scrolled inputs
- Forge completions preserve existing arguments and apply the inserted text and caret position together
- Forge console output follows new results without stealing focus, pauses while reading older output, and preserves the distinction between printed `undefined` and `null`
- Long-running Forge queries and idle shell sessions are no longer interrupted by the sidecar's former inactivity timeout
- Filter Builder shortcuts stay within the builder, invalid drafts are blocked before execution, and collapsing a group preserves its inputs and query
- Opening Forge now targets the highlighted collection, reuses matching find-all queries, and preserves existing query drafts
- Running Forge queries or selected statements with keyboard shortcuts no longer causes a nested view-update crash
- Transfer cancellation now blocks reruns and mode changes until the active operation has stopped, preventing stale completion races
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
- Smaller downloads: the app bundles plain JetBrains Mono instead of its Nerd Font build (text looks the same), and syntax highlighting includes only JavaScript and JSON, so AI answer code blocks in other languages show without colors
- Migrated the desktop UI to published GPUI Kit 0.6 components and removed the vendored toolkit patches
- Forge retains editor and result-view state across tabs and uses fuzzy completions with consistent native editing shortcuts
- Filter Builder now uses consistent native controls, collapsible borderless groups, and scoped keyboard handling with validation before execution
- Corner radii now follow one shared application scale across all built-in color themes
- Settings now use a searchable full-content tab with General, Transfer, AI Assistant, Agents & MCP, and Keybindings pages
- Connection management now uses a full-content singleton tab with explicit new-connection drafts, cancellation, draft-discard protection, and consistent New Connection entry points
- History now observes MongoDB changes passively and never pre-reads, authorizes, approves, or blocks originating writes
- Transfer now uses one compact Export, Import, and Copy workflow with progressive options and consistent aggregate progress across collection, database, JSON/CSV, and BSON operations
- Updates now require published SHA-256 assets, verify the downloaded archive and macOS code signature, respect the automatic-update preference, and install only after you choose Restart and install
- Update-check failures remain visible with Retry instead of silently returning to idle
- AI enablement now discloses the workspace metadata sent to the selected provider, and complete system prompts are no longer written to debug logs
- Transfer jobs that continue after errors retain failure counts, per-collection details, and processed-document totals
- Every download now has a published SHA-256 checksum
- Document query editors now provide field/value completion, typed ID queries, multiline drafts, and undoable formatting on submission, with sort and projection in Options
- AI chat panel moved out of the documents view into its own dedicated space
- Close buttons on tabs now only appear on hover (except the active tab)
- Tab bar styling updated with padding and theme-aware background

### Security
- Agent sharing and direct write authority default off independently; application read-only mode always wins, protected or Production access requires an explicit warning, MCP cannot approve Arcula operations, and decrypted History payloads never leave the app
- Existing files, collections, and databases remain unchanged until destructive imports, copies, and exports complete successfully
- Write confirmations include the exact connection, namespace, filter or pipeline, current count, and frozen options being approved
- Plaintext credential export is disabled; connection export is redacted or passphrase-encrypted
- Update archives and final app bundles are verified before replacing the installed application

### Performance
- Forge sidecar startup uses precompiled bytecode, and console/result updates avoid rebuilding unchanged output
- History uses one deployment-wide change stream per connection to avoid exhausting MongoDB connection pools, while large restores process independent documents concurrently and preserve same-document ordering
- Document tree (JSON view) expands and scrolls much faster on large or deeply nested documents — removed a quadratic dirty-check and the redundant full-tree clones that ran on every interaction
- Documents table is much smoother — it now re-renders only when the data or selection actually changes instead of rebuilding every visible cell every frame
- Aggregation results, schema view, and in-document search no longer redo expensive work (deep document clones, regex compilation, full schema re-walks) on every frame
- Sidebar search is much faster — results are cached and recomputed only when the query or the connection/database/collection list changes
- Per-collection caches are now freed when a tab closes, so memory no longer grows as you browse through many collections
- Copying a large multi-document selection no longer briefly freezes the UI

## [0.2.1] - 2026-03-05

### Fixed
- Release builds no longer include a debug-only Keychain override that failed the release lint check

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
