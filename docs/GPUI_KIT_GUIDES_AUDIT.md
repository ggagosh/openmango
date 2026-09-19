# gpui-kit guides audit

Audited 2026-09-19 against the two normative gpui-kit guides:

- <https://gpui-kit.com/docs/coding-guides/>
- <https://gpui-kit.com/docs/design-guides/>

**Scope.** `src/` (325 files, ~124k lines), excluding tests and `src/theme.rs` token definitions.
**Out of scope by decision:** everything about zoom — rem vs `px()`, base font, the 869 `px()`
literals and the pixel-based `spacing::`/`sizing::` tokens. OpenMango has no zoom today. Revisit
the day a font-size or zoom setting is planned; the cheap move then is converting the ~15 token
functions in `src/theme.rs`, which flips their 939 call sites at once.

**Method.** Six parallel read-only audits, one per rule area. Every finding was read in context,
not just grepped. Items marked ✔ were re-checked by hand afterwards. Paths are relative to `src/`.
Effort: S ≈ minutes, M ≈ an hour or two, L ≈ a day or more.

**Overall.** The app follows most of both guides. Raw colors: 1 in the whole codebase. Radii: 153
calls, 100% token-based. No render function mutates state or notifies unconditionally. Stale-result
rejection is correct in document queries, aggregation, updates, deletes and every transfer path.
Status is never carried by color alone. Destructive verbs are consistent per object class. The work
below is the remainder.

---

## P0 — bugs the guides would have prevented

### A. Disk reads and heavy work inside `render` (perf)

Measure before/after with the FPS monitor (command palette → "Toggle FPS Monitor"): watch `FRAME`
and `P95`.

- [ ] ✔ `components/content/tabs.rs:167` — `OpenTabsBar::render` calls `action_broker().list_all()`
  (`read_dir` + JSON parse of every file) every frame; the bar also observes all of `AppState`
  (`:121`). Same bug as the one fixed in the sidebar. Cache the pending count on the view, refresh
  on `AgentActivityChanged`. **M**
- [ ] ✔ `views/agent_activity.rs:42-43` — `render` does `list_all()` **and** `list_operations()`
  (two directory sweeps) per frame. Load into fields on open and on the change event. **M**
- [ ] ✔ `views/agent_activity.rs:27-34` — a spawned loop calls `cx.notify()` every 500 ms forever,
  forcing those sweeps twice a second with nothing changed. Drop the timer; notify from the event.
  **S**
- [ ] ✔ `state/app_state/sessions/model.rs:112-180` (called from `views/documents/view.rs:109`,
  per frame) — `session_snapshot()` deep-clones the session: with the explain modal open that is
  up to 20 explain runs each holding the full `raw_json` plan, plus schema, indexes, aggregation
  stages, history and `selected_docs`. Put the payloads behind `Arc`, or split scalar snapshot from
  payload. **M**
- [ ] `state/app_state/sessions/model.rs:137-142` — same path serializes the filter BSON → JSON
  every frame. Store the compiled string, recompute when the filter changes. **S**
- [ ] ✔ `views/documents/header/filter_bar.rs:65-66` — re-parses the filter text every frame
  (`compile_filter_input` + `document_id_input`) only to get a `valid` bool for styling. Compute on
  `InputEvent::Change` (subscription exists at `view.rs:299`). **S**

### B. Async safety

- [ ] ✔ `state/commands/transfer/export.rs:112`, `import.rs:118`, `copy.rs:87` — `execute_*` set
  `is_running = true` without checking it first; the only guard is in the UI. A double click between
  frames can start two runs, and with drop-before-import that is a destructive double submit.
  Early-return when already running. **S**
- [ ] `state/commands/schema.rs:20-37` — no in-flight guard and no generation: an older, slower
  sample can overwrite a newer result. Copy the pattern in `indexes.rs:23-39`. **S**
- [ ] `state/commands/explain.rs:37,164` — `explain.loading` is set but never checked; on completion
  the run is marked current without comparing its signature to the session's current query, so a
  plan for the previous filter is shown as current. **S**
- [ ] `app/root.rs:380,388,394,404` — four `start_history` failure exits only `log::error!`; the
  user gets a History feature that silently never works. Surface it. **S**

### C. Focus and keyboard correctness

- [ ] ✔ `components/filter_builder/panel.rs:2462` — `cx.focus_handle()` created fresh inside
  `render`, never tracked; Cmd+Enter focuses a handle attached to nothing, so focus is dropped.
  Store one handle on `BulkValueEditor`. **S**
- [ ] `views/documents/explain/mod.rs:162-188` — modal overlay with no focus handle, `track_focus`
  or key context: its Escape handler is off the dispatch path until clicked; no focus transfer on
  open, no restore on close, no trap. `:165-167` the scrim swallows clicks without dismissing. **M**
- [ ] `views/documents/dialogs/shared.rs:48-56` — window-global Escape intercept closes *a* dialog
  on any Escape regardless of which is topmost; the kit's Dialog already binds Escape on its own
  region. Delete the helper and its call sites (`property_dialog.rs:337`, `bulk_update.rs:131`). **S**
- [ ] `views/json_editor_detached.rs:835-859` — Cmd+W / Cmd+S handled by raw key match next to the
  identical `on_action` handlers at `:827/:831`; Cmd+C, Cmd+Shift+F, Cmd+N are raw-only, so not
  rebindable. **M**
- [ ] `views/documents/state.rs:137-306` — window-global `intercept_keystrokes` owns Escape, Enter,
  Tab, arrows and every Cmd chord for the view's lifetime with no "focus is inside this view" guard,
  so it fires while a dialog or the sidebar has focus. Move to `on_action` on the element that
  carries the key context (`view.rs:803`). Quick wins inside it: delete the Cmd+F branch
  (`:178-181`) and the Escape→close-search branch (`:212-216`), both already covered by bound
  actions. **L** (the two deletions are **S**)

### D. One command, several implementations that already disagree

- [ ] `app/actions.rs:372` vs `app/root.rs:1075-1080` — palette "AI Assistant" toggles the panel
  only; the real `ToggleAiPanel` handler also focuses the input. Dispatch the action. **S**
- [ ] `app/actions.rs:288-297` vs `views/documents/actions.rs:86-99` vs `app/actions.rs:92-108` —
  `CreateIndex` implemented three times with three different guards. And
  `components/action_bar/providers.rs:197-206` marks it available when both handlers would no-op.
  One owner method; derive `available` from its predicate. **M**
- [ ] `app/root.rs:1021-1052` vs `app/sidebar/mod.rs:736-781` — `EditConnection`,
  `DisconnectConnection`, `CopyConnectionUri`, `CopySelectionName` registered in both, reading
  different selections: Copy Name yields `db/collection` from the sidebar and the bare name from
  root; Disconnect checks `is_connected` only in the sidebar. Sidebar owns; root delegates. **M**
- [ ] `app/actions.rs:243-430` (`execute_action`) — the palette is a third implementation of ~12
  commands; some arms dispatch the Action, others copy the mutation (`cmd:create-database`,
  `cmd:create-collection`, `cmd:create-index`, `cmd:settings`, `cmd:query-library`, `cmd:ai`).
  Route every arm through `window.dispatch_action`. **M**
- [ ] `app/root.rs:1081-1093` vs `app/sidebar/mod.rs:680-700` — `OpenForge` twice. **M**

### E. Element identity on a list the user reorders

- [ ] `views/documents/views/aggregation/stage_list/stage_row.rs:251` and `:459` — the stage row and
  its **drag handle** are keyed by index; the drag source's key changes the instant the drop lands.
  `PipelineStage` has no id. Add `id: u64` (`state/app_state/aggregation.rs:12`) from a per-pipeline
  counter; also fixes `:228` (remove button tooltip). **M**

### F. Scroll regions with no scrollbar or a misplaced one

- [ ] `views/documents/views/schema_view.rs:186-215` — `uniform_list` with no `track_scroll` and no
  scrollbar: scroll position is unowned. Add a `UniformListScrollHandle`, `.track_scroll`, and
  `.vertical_scrollbar` on the wrapper (`:187`). **M**
- [ ] `views/documents/views/aggregation/results_view.rs:606-631` — list tracks a handle nothing
  draws a scrollbar for. `.vertical_scrollbar(&view.aggregation_results_scroll)`. **S**
- [ ] `views/results/mod.rs:68-88` — same, Forge result trees. **S**
- [ ] `views/databases.rs:283-293`, `:350-356`, `:456-464` — the scroll region sits inside a padded
  section, so its scrollbar floats inset from the panel edge; the header row is padded twice, so
  column labels sit 2×lg in while rows sit 1×lg in. One padding owner. **M**

### G. Wrong or missing semantic component

- [ ] `components/updater.rs:243` — "Release notes" is a `Button` that only opens a URL; the guide
  reserves that for `Link`. Only `open_url` in the repo. **S**
- [ ] `components/connection_manager/export_dialog.rs:115` — row `on_click` and the inner `Checkbox`
  (`:135`) both toggle the same field. Delete the row handler; let the checkbox own the label. **S**
- [ ] `views/forge/mod.rs:277,306` — result-tab pin/close are hand-built `div`s: no tooltip, no name,
  not keyboard reachable. Ghost icon `Button` + tooltip. **S**
- [ ] `views/databases.rs:398` — collection row is a clickable `div` with no selected state, focus or
  keyboard. `ListItem` + selected. **M**
- [ ] Six icon-only buttons with no name or tooltip: `views/documents/header/actions.rs:1050`,
  `:1080`, `:1200` (three refreshes, same icon), `components/filter_builder/panel.rs:2694`,
  `views/transfer/helpers.rs:148`, `views/transfer/query_modal.rs:207`. **S**

### H. Theme

- [ ] ✔ `components/connection_manager/export_dialog.rs:246` — `hsla(0,0,0.25,1)` marks the active
  button; invisible on the light themes. The only raw color in the app. `cx.theme().secondary` or
  `.primary()`. **S**

### I. Interface copy (Must-level)

- [ ] 20 labels use `...` where the guide requires `…`, and both ship side by side:
  `app/menus.rs:35,46,205,242,263,284,305,487,509`,
  `views/documents/tree/tree_menus.rs:145,171,194,217,240,263,283`,
  `views/transfer/simple.rs:109,425,477,540`. **S**
- [ ] One command, three names — Export is "Export Data" / "Export Data..." / "Export entire
  collection…", same for Import and Copy: `components/action_bar/providers.rs:230,240,250`,
  `app/menus.rs:242,263,284`, `views/documents/header/actions.rs:873,896,919` (these last also lack
  `.action(...)`, so their shortcuts never show). Pick one name per command. **M**
- [ ] Four confirm dialogs whose body re-asks the title: `app/menus.rs:57-60`, `:314,324`,
  `:518,530`, `views/documents/workflow.rs:84-85`. Put the object in the title
  (`Drop database "x"?`), leave only the consequence in the body. **S**
- [ ] `views/settings.rs:1858,1860` — title and confirm label are both "Clear all History". **S**
- [ ] `components/confirm.rs:115-117` — default production-write dialog: title "Confirm Production
  write", confirm label "Continue". Name the operation and its verb. **M**
- [ ] `views/transfer/mod.rs:250,252` — "Confirm destructive transfer" names no object; "Run
  Transfer" is the only Title Case confirm label. **S**
- [ ] `components/unsaved_guard.rs:82-84` — destructive button "Cancel operations & quit" sits next
  to the dialog's own "Cancel". Use "Quit anyway". **S**

---

## P1 — consistency work, mostly mechanical

### Index-keyed ids on lists that shift (~28 sites, almost all S)

The stable id is already in scope at nearly every site.

- [ ] `views/results/tree.rs:52,30` → `row.node_id` / `toggle_node_id`
- [ ] `views/documents/tree/lazy_row.rs:146,114` → `node_id` / `toggle_node_id`
- [ ] `views/documents/tree/tree_row.rs:139,88,356,442` → `item_id` / `toggle_item_id` (`:356` and
  `:442` are drag sources)
- [ ] `views/forge/mod.rs:278,307` → `page_id` (the click handler already distrusts the index)
- [ ] `components/query_library.rs:730,837,864,889,915,935,959,977` → `item.id` **M**
- [ ] `components/content/tabs.rs:282,264` → add `TabKey::element_key()` **M**
- [ ] `views/documents/table/document_table_delegate.rs:188,179`,
  `aggregation_table_delegate.rs:176,167` → column key (the pin button re-keys under the cursor as
  it is clicked)
- [ ] `views/documents/views/indexes_view.rs:174,156,115` → index name
- [ ] `views/transfer/options.rs:251,237,641,627` → collection name (the two blocks are verbatim
  duplicates; dedupe)
- [ ] `views/documents/views/schema_view.rs:902` → token label
- [ ] `views/documents/dialogs/index_create/key_rows.rs:117` → `suggestion.path`
- [ ] `views/ai.rs:1433` → collection name
- [ ] `app/sidebar/view.rs:654` → `.id("chevron")`, `:728` → `.id("connection-failure")` (parent row
  already namespaces them), `:914` → `.id(&entry.id)` instead of depth

### Unneeded `stop_propagation` (24 sites, S)

- [ ] Tail of `on_action` handlers where no ancestor registers the same action:
  `views/forge/actions.rs` (18), `views/ai.rs:1553,1557,1561`,
  `views/documents/actions.rs:49,57,65`. The other ~69 calls are justified.

### Scrolling

- [ ] Raw `overflow_y_scroll()` on panel regions, so no scrollbar is ever drawn — swap for the kit's
  `overflow_y_scrollbar()`: `aggregation/stage_list/mod.rs:106-119`,
  `components/ai_blocks/datatable.rs:154`, `components/ai_blocks/report.rs:226`,
  `components/error_history.rs:95`, `views/ai.rs:1146`. **S**
- [ ] Unbounded lists built one element per item per frame — virtualize:
  `views/documents/explain/mod.rs:1115-1132` (one div per JSON line), `history_view.rs:86-124`,
  `components/query_library.rs:1345-1357`, `views/databases.rs:457-464`. **M–L**
  (`settings/keybindings.rs:526-547` is bounded; only if it feels slow.)

### Per-frame clones and work (Should)

- [ ] `views/ai.rs:1172` — clones the whole conversation per frame to read `len()`. `Rc`. **M**
- [ ] `components/query_library.rs:991` → `:312-366` — `items()` per frame, O(n²) lookup. Cache. **M**
- [ ] `views/documents/table/cell_renderer.rs:39` — deep-clones nested BSON per cell per frame for a
  tooltip closure. `Arc<Document>` + path. **M**
- [ ] `components/filter_builder/panel.rs:2451-2452` — `parsed_values` runs twice per frame. **S**
- [ ] `views/transfer/mod.rs:297`, `views/settings.rs:1418` — full connection snapshots in render. **S**
- [ ] `state/commands/updater.rs:179-213` — every download chunk notifies `AppState`, redrawing the
  window. Batch like the transfer path does. **S**

### Side effects started from `render` (Should)

- [ ] `views/ai.rs:891` — `send_pending_prompt` can start a network request from render. **S**
- [ ] `components/content/mod.rs:351,372-387` — creates entities and may open a discard dialog. **M**
- [ ] `views/forge/mod.rs:188-190` — focus mutation in render. **S**
- [ ] `views/settings/keybindings.rs:388-393` — grabs focus on first render. **S**
- [ ] `views/documents/view.rs:79-101,273-687,1005-1014` — six lazy `cx.new`/subscribe blocks. **M**

### Key contexts and navigation

- [ ] Duplicate key-context declarations: `views/databases.rs:180`,
  `views/transfer/query_modal.rs:173`; the Documents context strings are built in both
  `views/documents/view.rs:791-799` and `app/root.rs:864-885`. **S**
- [ ] `views/ai.rs:1544` — `key_context("AiPanel")` with no focus handle, so `ClearAiChat` only fires
  while the composer is focused. **S**
- [ ] `views/documents/state.rs:845-908` — document tree has arrows only; no Home/End/PageUp/PageDown
  (the sidebar has all of them). **S**
- [ ] `app/root.rs:592-609` — close-tab re-matched by hand as a workaround for stale focus. **M**
- [ ] `components/filter_builder/panel.rs:2406-2418`, `dialogs/index_create/mod.rs:151`,
  `property_dialog.rs:341` — Cmd+Enter / Escape by key-string match; add actions. **M**
- [ ] `app/sidebar/view.rs:117-126` + `app/sidebar/mod.rs:252-290` — two dispatch paths for the same
  nav keys. Verified it does **not** double-move, but one path is the rule. Converting the arrows
  to Actions would also make them rebindable. **M**

### Components and accessibility

- [ ] Custom rows that want `ListItem`: `views/documents/views/schema_view.rs:567`,
  `views/ai.rs:1071`, `:1432`, `components/filter_builder/panel.rs:1946` (the pattern is already in
  `views/editor_completion.rs:54`). **M**
- [ ] Bare icon `div`s as remove controls: `views/transfer/options.rs:250,640`, `views/ai.rs:1362`.
  Ghost icon `Button` + tooltip. **S**
- [ ] Four tree chevrons with no tooltip (the sidebar's has one):
  `views/documents/tree/tree_row.rs:87`, `lazy_row.rs:113`, `views/results/tree.rs:29`,
  `views/documents/views/schema_view.rs:512` (also has no id). **M**
- [ ] Three disclosure headers as clickable `div`s: `views/transfer/progress_panel.rs:284`,
  `views/transfer/simple.rs:271`, `views/ai.rs:2431`. **M**
- [ ] Column pin toggles, icon-only and invisible until hover, no tooltip:
  `document_table_delegate.rs:187`, `aggregation_table_delegate.rs:175`. **S**
- [ ] `views/documents/table/cell_renderer.rs:40` — `cursor_pointer()` with no click handler. **S**
- [ ] `views/ai.rs:2498` — "Show N earlier calls" is link-styled text for an in-app command. **S**
- [ ] `views/documents/header/actions.rs:838` — ellipsis menu trigger without tooltip. **S**
- [ ] Unsaved-change dots with no tooltip: `views/documents/tree/tree_row.rs:399`,
  `components/content/tabs.rs:342`. **S**
- [ ] `views/databases.rs:406` — hover + pointer cursor even when the click will early-return. **S**
- [ ] `views/agent_activity.rs:430` — confirm dialog, flagged destructive, on a reversible action
  (rejecting an agent request). **S**
- [ ] Context-menu-only commands: "Copy Name" (`app/menus.rs:105,707`) has an action but no binding
  or palette entry; `tree_menus.rs:307` "Copy Value" declares an action with no binding (empty
  shortcut slot); `tree_menus.rs:68,337,346,372,386` Filter/Exclude by value have no action. **S–M**
- [ ] `views/documents/header/actions.rs:943-951` — AI toggle encodes state in its label instead of
  `.checked()`, and has no `.action(...)`. **S**
- [ ] "Refresh" is "Reload Database" in the sidebar menu, "Refresh" in the palette, and the three
  toolbar icons don't dispatch `RefreshView` at all. **S**

### Copy consistency (Should)

- [ ] Sentence case within a component class: 72 `PopupMenuItem` labels split Title/sentence across
  files (`app/menus.rs`, `tree_menus.rs`, `table/column_menu.rs`); all 30 palette labels are Title
  Case; 14 of 37 dialog titles are Title Case. **M**
- [ ] Palette items that open a dialog lack `…`: `providers.rs:162,170,180,190,200,230,240,250`;
  also `aggregation/stage_list/mod.rs:222`. **S**
- [ ] Placeholders: three treatments (`...`, none, `…`), one with a period. Strip. **S**
- [ ] `views/transfer/simple.rs:315,408` — "Cancelling...", "Loading..." instead of a spinner. **S**
- [ ] Error strings: ~19 of 36 lack the final period; several name no cause or next step ("Drop
  failed: not allowed", "Export failed: {}"). **M**
- [ ] Settings descriptions: 8 of 21 lack the period. **S**
- [ ] Small lexicon drift: "Retry" ×5 vs "Try again" ×1; "Show All" vs "Show all"; "Approve & Run"
  vs "Approve and run"; four phrasings of "discard changes"; "What's New" opens "Changelog". **S**
- [ ] Eight strings use `document(s)` style plurals; `tree_menus.rs:38-40` already branches. **S**
- [ ] `components/query_library.rs:1079,1091` — selection shown by mutating the label ("✓ This
  connection"). Use the button's selected state. **S**
- [ ] `providers.rs:339-340,501-502` — detail text restates the label. **S**

### Theme tokens (Should)

- [ ] One intent, many alphas. Divider "softer than border" is spelled nine ways (.50–.85) across 28
  sites → add `colors::border_subtle`. Hover wash on custom rows is `secondary.opacity(x)` at five
  values while `list_hover` exists. Warning/danger washes and borders re-spell
  `colors::bg_warning/bg_error/border_warning/border_error`, which exist and are used by exactly one
  file. **M**
- [ ] `components/content/tabs.rs:205-209` — builds tab chrome from `foreground.opacity()` behind an
  `is_dark()` branch while `tab_active` tokens exist 100 lines up. **M**
- [ ] Emoji as icons: `changelog.rs:89-95` (on tags that already carry color),
  `views/transfer/query_modal.rs:238,240`, `components/query_library.rs:1079,1091`. **S**
- [ ] `views/documents/export/mod.rs:39-41` — raw asset path where `AppIcon` variants exist. **S**
- [ ] Icon sizing: four idioms across 227 sites. Pick two. **M**
- [ ] Three spellings of transparent. **S**

---

## P2 — structural, decide first

- [ ] **Value pickers as menus.** 15 pickers (theme, export format, import mode, AI provider, write
  mode…) are `Button` + `dropdown_menu` where the kit's `Select` carries current-value and
  accessibility semantics: `views/settings.rs:506,590,1888,1922,2033,2280`,
  `views/transfer/options.rs:23,62,112,316,363,497`, `views/transfer/simple.rs:148,196,627`. **L**
- [ ] **No OS menu bar.** Commands are discoverable only via palette and context menus; Rename
  Collection and Drop Database exist only as a context-menu row plus a bare keystroke. The actions
  and shortcut metadata already exist. **L**
- [ ] **Everything observes `AppState`.** Ten blanket `cx.observe(&state, …notify)`. Measure first:
  watch the `INV` row while an AI reply streams and while an update downloads. Cheapest narrowing,
  in order: batch updater progress; move `ai_chat` into its own entity; replace blanket observes in
  `tabs.rs:121`, `databases.rs:26`, `query_library.rs:289` with subscriptions to the events they
  use. **L**
- [ ] **Product decisions, not bugs.** `theme.rs:45-48,493-510`: every UI font token is JetBrains
  Mono and `apply_design_tokens` force-sets font and radius after every theme load, so no theme file
  can change them. The guide wants a platform UI font for chrome and theme-derived radii. Only
  matters if custom themes should be able to restyle these.

---

## Verified fine — do not re-audit

- Scroll ownership: app shell, connection manager, filter builder, explain panes, indexes view,
  document tree, table view, agent activity, changelog. `min_h_0` is released by the kit's scroll
  wrapper, so sites without an explicit one are not broken. No doubled hairlines in the shell.
- Identity: nothing mints ids, scroll handles, focus handles or entities inside `render` (except the
  one orphan focus handle above). A child id is scoped by its parent's, which clears table cells
  (the kit wraps each), AI blocks (namespaced by message id), error callouts, filter-builder cards.
  Table row ids stay index-based on purpose: the kit's own selection contract is index-based.
- Keyboard: 145 actions, 239 bindings; all 20 context-menu items dispatch Actions with shortcuts
  derived once. Dialog focus save/restore is done by the kit's `Root`. ~69 of 93
  `stop_propagation` calls are justified.
- Components: of 330 `Button` chains only the six listed lack a name. Dropdown triggers stay active
  while open (the kit does it). Status is never color-only (20 sites). ~40 confirm dialogs, all but
  one guard something irreversible.
- Async: generation/revision rejection correct in `documents/query.rs`, `aggregation.rs`,
  `documents/update.rs`, `documents/delete.rs`, `operations.rs`, `updater.rs` and all transfer
  paths. The transfer commands are the strongest async code in the repo. The AI timeline is well
  built (revision-keyed cache, virtualized, 16 ms batching).
- Theme: 1 raw color in the codebase; 153 radius calls all token-based; 9 shadows, all on elevated
  surfaces; all 53 font-family calls go through `fonts::`.
- Copy: "Cancel" consistent ×16; no "OK"/"Yes"/"No" confirm labels; no "Are you sure"; tooltips all
  sentence case. Drop / Delete / Remove maps one verb to one object class.

## Needs the running app to settle

- Explain modal open vs closed: does `P95` jump? That ranks the `session_snapshot` fix.
- `FRAME` with an empty actions directory vs ~50 agent operations on disk.
- `INV` during AI streaming and during an update download.
- Does the `export_dialog.rs:115` checkbox row actually double-toggle?
- Are the custom disclosure headers and bare chevrons reachable by Tab?
- Does Escape close one dialog or two with `escape_key_subscription` in place?
- Vertical swipe over an AI result table inside a long conversation: does it hand off?
- Are the clustered alphas (.78/.80/.82) visually distinguishable, i.e. is collapsing them lossless?
