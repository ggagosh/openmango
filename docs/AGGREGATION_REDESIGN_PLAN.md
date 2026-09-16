# Aggregation tab redesign plan

Scope: `Collection → Aggregation` subview (`src/views/documents/views/aggregation/**`, its state in
`src/state/app_state/aggregation.rs`, `src/state/app_state/sessions/aggregation.rs`,
`src/state/commands/aggregation.rs`, header actions in `src/views/documents/header/actions.rs:1189`).

Inputs: `better-interface` review (all six domain skills), `emil-design-eng`, a runtime screenshot of the
empty state, a survey of Compass / Studio 3T / NoSQLBooster / VS Code / DataGrip / VisuaLeaf, and a
component audit of `gpui-kit 0.6.1` (source + gpui-kit.com).

## 1. What is wrong today

Current shape: three islands. Stage list (280px) and a single-stage editor share a 240px-tall top split,
results (360px) below. Run/Explain sit in the collection header, top right.

| # | Sev | Problem | Evidence |
| --- | --- | --- | --- |
| 1 | HIGH | Selecting another stage relabels the results ("Results (after Stage 2: $group)") and recomputes count/pages from that stage, but the documents are still from the previous run. | `sessions/aggregation.rs:127-139` only invalidates; `results_view.rs:39-64` labels from `selected_stage` |
| 2 | HIGH | Every keystroke in the stage editor wipes results and every stage count; the results panel flips to "Run the pipeline to see results". You lose the output you were editing against. | `sessions/aggregation.rs:142-156` → `reset_aggregation_after_edit` (`:44`); `results_view.rs:527-540` |
| 3 | HIGH | Two meanings share "dimmed": stages after the selected one fade to 0.55 (= not in preview), while a disabled stage only flips its switch. Dimmed reads as disabled. | `stage_row.rs:164,277` vs `:203` |
| 4 | HIGH | The stage list takes focus with no visible focus indicator. Backspace/Delete deletes a stage when the list has focus and edits text when the editor has it. | `stage_list/mod.rs:156-162`; `keyboard.rs` `DeleteAggregationStage` on `!Input` |
| 5 | HIGH | Before any run: "0 document(s)", "Showing 50 per page", "Page —", Prev/Next. Reads as an empty collection. | screenshot; `results_view.rs:54-56,401-419` |
| 6 | HIGH | "Timing" switch says "(slower)" but adds no query (only `Instant::now`). The per-row "· 41ms" is the cumulative `$count` of the whole prefix, not the stage's own cost. | `commands/aggregation.rs:554-586`; `results_view.rs:165`; `stage_row.rs:586-591` |
| 7 | MED | Results header crams title, count, spinner, 2 switches, limit, view toggle, copy, export into one row with 4px gaps; "Timing" and "Limit" read as one label "Timing Limit". Disabled switches render checked in accent. | screenshot; `results_view.rs:71-245` |
| 8 | MED | Empty pipeline shows four messages across three panels ("No pipeline stages yet.", "Select a stage", "Select a stage to edit", "Run the pipeline to see results"). | screenshot; `stage_list/mod.rs:209-240`, `stage_editor.rs:148,230`, `results_view.rs:537` |
| 9 | MED | Stage list lives in a 240px-tall split: ~5 rows before scrolling; editor gets the same cramped height. Split sizes are not persisted. | `aggregation/mod.rs:68-69` |
| 10 | MED | Pipeline errors are one line of danger text above results, not attached to the stage; Format errors only go to the status bar. | `results_view.rs:249-257`; `stage_editor.rs:204-212` |
| 11 | MED | Delete (button, menu, Backspace) always opens a modal "This cannot be undone." | `stage_row.rs:248-259,555-565` |
| 12 | MED | Insert-stage chips are invisible until hover (opacity 0) with a 16×16 target; "Add Stage" always appends, so there is no keyboard path to insert mid-pipeline. | `stage_row.rs:405-449,609-610`; `stage_list/mod.rs:113-119` |
| 13 | MED | Copy: Title Case ("Add Stage", "Copy Results As", "Export Results") next to sentence case ("Insert stage", "Import pipeline"); "Limit" is really page size; "document(s)"; hard-coded shortcut text in tooltips. | `results_view.rs:43,217,745,794`; `stage_row.rs:206`; `stage_list/mod.rs:59` |
| 14 | MED | Hand-rolled widgets that gpui-kit already ships (see §4). | `dialogs.rs:37-287`, `stage_row.rs:695-714`, `results_view.rs:301-499,708` |
| 15 | LOW | Drag preview says "Moving stage..."; drop target swaps in a 3px border that shifts row content. | `stage_row.rs:45,319-324` |

## 2. What the other tools taught us

Steal:
- **Selection = preview point, preview follows selection** (Studio 3T, Compass Focus Mode). Arrow through
  stages and the output follows.
- **Never blank the output.** Keep the last good output, mark it outdated (Compass: "Output outdated and
  no longer in sync"). Optional auto-preview ~700ms after typing stops, cancelling in-flight runs.
- **Errors live on the stage that caused them**; later stages say "blocked by stage N" with a jump link.
- **Input | Output for the selected stage** (Studio 3T) instead of a preview on every card.
- **Stages and Text mode over one source of truth**, switchable without losing work; parse per stage so
  one bad stage doesn't lock the switch.
- **Be honest about cost and sampling.** Counts are an explicit, cheap-to-understand option.
- **`$out`/`$merge` replace the preview** with "Writes to db.coll" and require confirmation (we already confirm).
- Operator picker accepts `match` without `$`.

Avoid: horizontal doc-card strips per stage (Compass), stages as tabs, one Run button per panel, helper
comments injected into stage bodies, form wizards that can only add.

## 3. Target design

```
┌ Pipeline ─────────── Counts ▾ ─┐┌ Stage 2  $group ▾ ───────────── Format  ⋯ ┐
│ ● 1  $match   34,820 → 12,004  ││ { _id: "$sensor", readings: { $sum: 1 } } │
│ ● 2  $group   12,004 → 85    ◀ ││                                           │
│ ─ ─ preview stops here ─ ─ ─ ─ ││ [Alert] Stage 2: unknown operator $sm     │
│ ○ 3  $sort    Skipped          │├ Output after stage 2 · 85 docs  Outdated ─┤
│ ● 4  $limit                    ││ [Input|Output]        [Tree|Table] Copy ▾ │
│                                ││ …results…                                  │
│ + Add stage            ⌘⇧N     ││ Page size [50]         ‹ 1 2 3 ›          │
└────────────────────────────────┘└───────────────────────────────────────────┘
```

- **Full-height pipeline rail** on the left; editor over results on the right. Sizes persisted via
  `ResizableState` (`.with_state` + `on_resize`).
- **Results are bound to the run that produced them.** Header text comes from the run's target stage,
  not the current selection. Edits and selection changes keep the documents and add an `Outdated` tag
  plus "⌘↵ to update". Selecting a stage re-runs the preview (debounced ~250ms so arrowing is free).
- **Preview cutoff is explicit**: a "Preview stops here" divider under the selected stage replaces opacity
  dimming. Disabled stages show "Skipped" text + muted operator, not just a switch.
- **Errors**: `error_stage` in `PipelineState`. Parse errors already know the index
  (`commands/aggregation.rs:352-359`); count failures know the prefix index. Editor shows
  `Alert::error` for the selected stage; the row shows an error marker with text; results show
  "Preview unavailable: error in stage N" with a "Go to stage N" button.
- **One empty state** when there are no stages: what a pipeline is, "Add stage ⌘⇧N", quick starts
  (`$match`, `$group`, `$project`), "Import pipeline", "Open library". Editor and results appear after
  the first stage. Before the first run: "Run to preview · ⌘↵", no count, no pagination.
- **Results header** = title + count/time (left); `Input|Output`, `Tree|Table` ButtonGroup, Copy/Export
  menus (right). Stats moves to the rail header as `Off / Counts` (Timing folds into Counts; label the ms
  as "to here"). Page size moves to the footer beside `Pagination`.
- **Delete is instant with Undo.** `UndoHistory<Vec<PipelineStage>>` records add/delete/move/toggle/
  duplicate/import; ⌘Z in the rail; `Notification` "Stage deleted · Undo".
- **Keyboard**: "Add stage" inserts after the selected stage; ⌘⇧N shortcut (verify free; `cmd-shift-a`
  is taken by `AddField`); visible focus ring on the rail when it owns focus; insert chip is 24px and
  always visible on the selected row.

### Motion (Emil): almost none, on purpose

This view is keyboard-driven and used hundreds of times a day.

| Before | After | Why |
| --- | --- | --- |
| Results blank to "Run the pipeline…" on edit | Keep documents; `Outdated` tag + 0.6 opacity, instant | No flash of empty; state carried by a label, not motion |
| "Running pipeline..." replaces the tree | Spinner/`ShimmerText` in header; old docs stay until new ones land, then swap instantly | Perceived speed; content stays readable |
| Stage selection | Instant background change, no transition | Keyboard action, 100s/day |
| Drop target = 3px border on row | Absolute 2px accent line, no layout change | Row content must not jump during drag |
| Insert chip opacity 0 → 1 on hover | Visible on selected row; hover fade ≤120ms opacity only | Discoverability is the purpose, not decoration |
| Confirm modal on delete | Immediate delete, Undo notification (gpui-kit default motion, respects reduced motion) | Fast where the system responds |

## 4. gpui-kit adoption

Replace (drop-in or near):

| Custom today | gpui-kit | Notes |
| --- | --- | --- |
| Operator picker dialog, manual focus map (`dialogs.rs:37-287`) | `component::command::{Command, CommandGroup, CommandItem}` in `open_dialog` | Same pattern as `components/action_bar/mod.rs:348-477`; add `keywords` ("filter", "join", "match") |
| Operator dropdown (`stage_editor.rs:54-123`) | Same `Command` popup, or `select::Select` + `SelectGroup` `.searchable(true)` | One picker, two entry points |
| `menu_item` + `registered_shortcut` (`stage_row.rs:695-714`) | `PopupMenuItem::new(..).action(Box::new(A)).on_click(..)` | Menu renders the binding itself; delete both helpers |
| Tree/Table toggle with manual bg (`results_view.rs:301-369`) | `ButtonGroup` + `Button::selected().toggled()` | As in `filter_builder/controls.rs:16` |
| `agg_separator` (`results_view.rs:708`) | `separator::Separator::vertical()` | |
| Prev/Next footer (`results_view.rs:381-499`) | `pagination::Pagination`, reuse `views/documents/pagination.rs:19-98` | Keep Prev/Next fallback when total is unknown (counts off) |
| Limit input + manual parse (`mod.rs:176-251`) | `input::NumberInput` with `min(1.)` | Keep the integer check |
| Danger text error (`results_view.rs:249`) | `alert::Alert::error(..)` | Also in import dialog |
| Stats + Timing switches (`results_view.rs:104-216`) | `tab::TabBar::segmented()` (Off / Counts) or a menu | |
| Delete confirm (`stage_row.rs:248,555`) | `gpui_kit::base::UndoHistory` + `notification::Notification` with Undo action | Requires `Root::render_notification_layer` in `app/root.rs:773` |
| "Clean" `ButtonCustomVariant` copied 3× | `Button::ghost()` | |
| Unlabelled rows/switches | `.role(Role::ListItem)`, `.aria_label`, `.aria_selected`; `Switch::accessibility_label` | GPUI now exposes AX roles/names; update `docs/accessibility.md` "Framework limitation" |

Keep custom (not in 0.6.1): drag reorder (`DragStage`, `compute_drop_target`), empty-state layout
(docs list `Empty`, 0.6.1 doesn't ship it), the lazy multi-column results tree, integer-only validation,
live-region announcements.

Considered and skipped: `Stepper` (the rail already shows the flow), `HoverCard` doc previews
(Input|Output covers it), `Accordion`/collapsible stage cards (one editor at a time is the point),
`List`/`ListDelegate` for the rail (drag stays custom anyway; revisit only for its keyboard handling).

## 5. Phases

**Phase 0: stop misleading (small, state-only).** Fixes 1, 2, 5, 6.
- `PipelineState`: add `results_stage: Option<usize>`, `stale: bool`, `error_stage: Option<usize>`.
- Split `reset_aggregation_after_edit` into "mark stale" (body/operator edit, selection) and "reset counts"
  (structural edits only). Results survive edits.
- Header/footer read `results_stage`; hide count and pagination until the first run.
- Selection change schedules a debounced preview run.
- Remove the Timing switch; relabel per-row time.
- Tests: extend `sessions/aggregation.rs` unit tests (edit keeps results + marks stale; selection marks
  stale; structural edit realigns counts) and `tests/aggregation_tests.rs` for `error_stage`.

**Phase 1: layout and gpui-kit swaps.** Fixes 7, 8, 9, 10, 13, 14.
- Full-height rail, persisted splits, single empty state, regrouped results header, footer page size.
- `Command` picker, `PopupMenuItem::action`, `ButtonGroup`, `Separator`, `Pagination`, `NumberInput`, `Alert`.
- Stage-attached errors (editor alert, row marker, results "Go to stage N").
- Sentence case, "Page size", pluralization, `tooltip_with_action` everywhere.

**Phase 2: keyboard, safety, accessibility.** Fixes 3, 4, 11, 12, 15.
- "Preview stops here" divider, "Skipped" rows.
- Rail focus ring; insert-after-selected + ⌘⇧N; 24px chip visible on selected row.
- `UndoHistory` + Undo notification; drop the delete confirm (keep `$out`/`$merge` run confirm).
- AX roles/labels on rows, switches, chips; drag preview "2 · $group"; overlay drop line.
- Update `docs/accessibility.md`.

**Phase 3: power features (from research), each shippable alone.**
- Auto-preview toggle (700ms debounce, reuses `run_generation` cancellation).
- `Input | Output` for the selected stage (input = run to `idx - 1`).
- Text mode: whole pipeline in one `Editor`, per-stage parsing, lossless switch.
- `$out`/`$merge` as last stage: replace preview with "Writes to db.coll".
- Later: field autocomplete from the previous stage's output.

## 6. Open questions

1. Auto-preview on by default (Compass) or opt-in (cheaper on big collections)?
2. Stage counts default: keep on (N extra `$count` scans per run) or off with a one-click "Count"?
3. Text mode: Phase 3, or pull it earlier?
