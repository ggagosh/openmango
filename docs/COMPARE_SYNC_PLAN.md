# Compare and sync collections — implementation plan

Planned 2026-09-21. Cross-checked against `src/` (file references below), gpui-kit 0.6.2
(`gpui-component`, `gpui-base`), the MongoDB manual, and the compare tools in Studio 3T, Navicat,
Mingo, Redgate SQL Data Compare, dbForge, DataGrip, Beyond Compare and DBeaver. Design rules come
from the `better-ui`, `better-layout` and `emil-design-eng` skills.

**What ships:** a Compare tab. Pick two collections (same or different connections), choose what
identifies a document (`_id` by default), press Compare, see what is only on one side and what
differs, inspect any document field by field, then write the chosen differences into one side,
with an undo.

**Decisions** (confirmed by the owner on 2026-09-21):

1. **Documents are matched by a chosen key, `_id` by default.** One or more fields. It is part of
   the first build, in the engine, the screen and sync. There is one code path: `_id` is simply the
   key `["_id"]`.
2. Field order and number type (`1` vs `1.0`) are **minor** differences: counted and listable, but
   kept out of "Different" and out of sync unless the user opts in. No options for them.
3. Sync writes whole documents, one target per run, target chosen explicitly (no default).
   Delete is off by default.
4. At most 250,000 differences are stored per run. Counts stay exact past the cap.
5. Undo is per tab and in-session, **for now**. Before-images go to an encrypted temp file whose
   key lives only in memory. Undo across restarts is under Later.
6. Time-series collections are refused as a side in v1 (no index to stream by).
7. **Sync and undo require MongoDB 8.0+ on the write target** (owner decision, 2026-09-22).
   Older servers remain supported for comparison and as read-only sync sources. There is no
   legacy write fallback: native unordered bulk writes preserve batching and individual results.

## Implementation status — 2026-09-22

Slice 1's read-only engine is implemented and validated: 12 engine unit tests and 8 MongoDB
integration tests passed. The million-document benchmark passed twice, with exact expected
counts. Indexed scans used IXSCAN; the unindexed case used SORT/COLLSCAN. See
[measured results and reproduction steps](COMPARE_BENCHMARKS.md).

Slice 2's read-only Compare tab is implemented: collection-menu and command-palette entry points,
connection/database/collection selectors, match-key and ignore tokens, filter, index suggestions,
progress/cancel, virtualized difference segments, exact key lookup, and document detail with
expandable branches, unchanged groups, BSON type notes, copy/open actions, and stale-result notices.
Each tab retains its own results; only configuration persists, including when offline.
Slice 3 is implemented: explicit target selection, category and row selection, review through the
existing write gate, native unordered bulk writes, per-row outcomes, cancellation between batches,
encrypted session-only restore files, and guarded bulk undo. Sync-specific validation is recorded
in [the benchmark report](COMPARE_BENCHMARKS.md).

After screenshot feedback, matching, filtering and ignored-field controls now live in a native
**Comparison settings** popover. The main header retains a compact rule summary and Compare
action. This supersedes the inline token/Options arrangement in §3.1 below. The inspector uses
aligned Field/Left/Right columns, cell highlights, and explicit missing values; result rows are
single-line. See [the UX revision and rendered-control checks](COMPARE_UI_REVIEW.md).

The Compare GPUI layout test covers 430, 900, and 1200 px widths, tab switching and populated
results. New row tints are included in the theme contrast test. Live native interaction and
million-document UI responsiveness have not been measured. Cargo reports the existing
future-compatibility notice for dependency `block 0.1.6`.

Read-only milestone tests: 683 library tests passed (1 ignored), 8 comparison integration tests passed
(manual benchmark ignored), and 9 settings/workspace integration tests passed. The benchmark
was run explicitly twice. No application launch was performed.
Formatting and Clippy (`--lib --bin openmango --tests -- -D warnings`) passed at that milestone.

After sync and undo: 693 library tests passed (1 ignored), plus 8 sync, 8 comparison, and 9
settings/workspace integration tests. Formatting and Clippy with `--all-targets -- -D warnings`
passed. A separate 100,000-replacement sync/undo benchmark passed on MongoDB 8.2.3; timings and
reproduction steps are in [the validation report](COMPARE_BENCHMARKS.md). Native checkbox and
mixed-selection interactions are covered by the GPUI harness; no live application launch was
performed.

Implementation refinements to the design below:

- Raw comparison returns `Result<Verdict>` so parsing failures stop the scan. The detail function
  uses the same walker and carries explicit value/order/number-type notes.
- Int64/Double comparison checks the exclusive `2^63` upper bound before casting. The formula
  in §4.4 alone incorrectly equates `i64::MAX` with the double `2^63` because casts saturate.
- Readers use explicit sessions and `refreshSessions` every five minutes, including while
  blocked by backpressure. Reading another buffered chunk does not necessarily issue `getMore`.
  [MongoDB documents session refresh for idle cursors](https://www.mongodb.com/docs/manual/reference/parameters/#mongodb-parameter-param.cursorTimeoutMillis).
- Chunks stop at 1,024 documents or 4 MiB (one larger document may exceed that); each side has
  four queued chunks. Stored rows also have a 64 MiB serialized-key/id-plus-row budget, since
  large BSON keys make a row-count-only memory estimate unreliable. This is a storage estimate,
  not a measured peak RSS guarantee. Counts continue after either cap.
- Scans, skipped counts, and dotted-array checks all use simple collation. A view with a
  non-simple collation is refused because MongoDB does not permit overriding its collation.
- Dotted-key array traversal is checked before eligibility filtering, so arrays that would be
  filtered out cannot silently escape the guard. Decimal128 and unsupported legacy/code/regex
  key types fail closed. Decimal128 document values still compare by their exact representation.

Focused engine validation commands:

```sh
cargo test --lib bson::compare::tests
cargo test --lib connection::ops::compare::tests
cargo test --test compare_tests -- --test-threads=1
```

The ignored benchmark requires explicitly seeding a disposable MongoDB server with
`mongosh <local-test-uri> scripts/compare-bench-seed.js`, then setting `OPENMANGO_COMPARE_BENCH_URI` and
running `cargo test --test compare_tests compare_million_document_benchmark -- --ignored --nocapture`.
Peak RSS and sampled growth were measured separately from compilation with `/usr/bin/time -l`
and `ps` on macOS. Long-idle session refresh and sharded deployment behavior remain unverified.

---

## 1. What the evidence says

Every paid tool has this; Compass closed its ticket (COMPASS-3536). A G2 reviewer names compare as
the reason they bought Studio 3T. The table turns what the tools do and what their users complain
about into requirements.

| Evidence | Requirement here |
| --- | --- |
| Studio 3T idea, 3 votes, completed as "Data Compare 2.0": "Object ID … will always differ for collections, so I am unable to extract results as desired… Ability to compare two different collections by specifying User defined Unique ID field" ([board](https://3t-io.uservoice.com/forums/265122-share-your-ideas-with-us)). DBeaver blocks its wizard until keys are chosen; Beyond Compare lets any columns be the key | **Match by** is a first-class control in the header, not a setting. Default `_id` |
| Studio 3T advises match fields that "1.) have unique values and 2.) whose collections have an index", and puts non-unique keys in a **Multiple Matches** tab ([KB](https://studio3t.com/knowledge-base/articles/data-compare-and-sync/)) | Suggest unique-index fields first; warn when no index covers the key; a **Multiple matches** bucket, never an error |
| Every tool uses the same four buckets: source-only, target-only, different, identical ([Navicat manual](https://www.navicat.com/manual/pdf_manual/en/navicat_17/win_manual/navicat_en.pdf), [Mingo](https://mingo.io/docs/tools/compare-sync)) | Same four, plus **Minor** and **Multiple matches** |
| Redgate: "identical values will not be stored on disk nor appear in the comparison results" ([options](https://documentation.red-gate.com/sdc/setting-up-the-comparison/setting-project-options)) | Identical documents are counted, never stored. This is the main memory rule |
| Redgate has a troubleshooting page for "[differences in two identical databases](https://documentation.red-gate.com/sdc/troubleshooting/common-issues/sql-data-compare-showing-differences-in-two-identical-databases)"; Studio 3T has a "treat number types as equal" option | Type-only and order-only differences must not flood the result. Beyond Compare's important/unimportant split is the model: the **Minor** bucket |
| Studio 3T idea, 3 votes: the overview counts ignore the user's ignore-list | One comparator produces both the counts and the rows. Never two code paths |
| Navicat turns Insert, Update **and Delete** on by default | Delete is off by default here |
| Navicat is one direction per run; Studio 3T copies per document and per field in both directions | One target per run, whole documents. Per-field copy is listed under Later |
| Navicat, Redgate and DBeaver preview before applying; Redgate offers a backup step | A review dialog with exact counts, and an automatic restore file with Undo |
| DBeaver: "Limit compared rows", "Limit different rows". DataGrip compares 500 rows by default | State limits in the UI. A filter narrows the read; the stored-difference cap is visible when hit |
| Studio 3T ideas: compare two documents (5 votes), ignore array order (2), export all diffs (1) | Later (§11). The diff renderer built here makes "compare two documents" nearly free |

Not verified by the survey: how Studio 3T behaves at scale (limits, memory, cancel). Only its
1,000-document read batch is documented.

---

## 2. The user flow

The common case is "same namespace, other server" (staging against production). It must take four
actions:

1. Right-click `orders` in the sidebar → **Compare with…** (or palette: "Compare collections").
   A Compare tab opens with the left side filled in.
2. Pick the other connection on the right. Database and collection prefill to the same names when
   they exist there. **Match by** already says `_id`; change it only when the two sides were
   seeded separately and their `_id`s differ (then pick `sku`, `email`, or `tenantId` + `sku`).
3. Press **Compare** (`cmd-enter`). Nothing reads until this press: a compare reads both collections
   in full, so it needs intent. (Same rule as `docs/RELATIONS_PLAN.md` §1.3.)
4. Rows stream in while the scan runs. Click any row to see its field diff, even mid-scan.

Sync is three more: choose the target, adjust the checkboxes, **Review and sync…** → confirm.

---

## 3. Screen design

One tab, four regions, top to bottom. Regions are separated by space (16px between regions, 8px
inside one); the only lines are the split divider and the top border of the sync bar, because both
mark real structure. All leading edges share one inset.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│ LEFT                                     ⇄   RIGHT                                     │
│ [● Staging ▾] [shop ▾] [orders ▾]            [● Production ▾] [shop ▾] [orders ▾]      │
│ ~1.2M documents · 1.4 GB                     ~1.2M documents · 1.4 GB                  │
│                                                                                        │
│ Match by [ sku × ] [+]     ▸ Options  No filter · ignoring updatedAt   [ Compare  ⌘↵ ] │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ [All 168] [+ Only in left 120] [− Only in right 3] [≠ Different 45] [≈ Minor 212]      │
│ [2× Multiple matches 4]                                                                │
│ 9,832 identical · 37 without sku skipped · compared 14:32 UTC · 2.1 s  [ Find sku…   ] │
│                                                                                        │
│ ┌ differences ───────────────────────┐ │ ┌ sku "A-1001" ── copy · open left · open right ┐
│ │☑ ≠ "A-1001"    3 fields: status, … │ │ │ field        staging            production    │
│ │☑ ≠ "A-1002"    1 field: total      │ │ │ status       "open"             "closed"      │
│ │☑ + "A-1044"                        │ │ │ total        Int32 120          Double 120.5  │
│ │☐ − "B-2210"                        │ │ │ items.2.qty  2                  3             │
│ │ …                                  │ │ │ ▸ 14 unchanged fields                         │
│ └────────────────────────────────────┘ │ └───────────────────────────────────────────────┘
├────────────────────────────────────────────────────────────────────────────────────────┤
│ Write changes to  ( ) ● Staging · shop.orders   ( ) ● Production · shop.orders         │
│ ☑ Insert 120 missing  ☑ Replace 45 different  ☐ Replace 212 minor  ☐ Delete 3 extra    │
│                                                                  [ Review and sync… ] │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Setup header

- **Pickers.** Two groups of three cascading `Select`s, exactly the Transfer tab's
  (`src/views/transfer/select_states.rs:15-68`). Promote `ConnectionItem` to `pub(crate)` rather than
  copying it: its `render` draws the connection identity badge, so production looks like production
  inside the picker. Only open connections are listed (today's constraint,
  `src/views/transfer/mod.rs:299-306`); the connection select's empty state says so.
- **Prefill.** When the right connection changes and the left database/collection exist on it,
  select them. Never overwrite a choice the user made by hand.
- **Size line.** Under each side: `~1.2M documents · 1.4 GB` from `estimated_document_count` and
  the existing `collStats` loader (`src/connection/ops/stats.rs`). When the two sides together
  exceed 1 GB, one muted line under the header: "This reads both collections in full (about
  2.8 GB). A filter compares part of them." and the word "filter" opens Options.
- **Swap** (`⇄`) exchanges the sides. Plain ghost `Button`; no kit component exists.
- **Match by.** Visible in the header, never inside Options: it decides what the whole result
  means.
  - A token input of field paths, the same component as Ignore fields. It starts with the token
    `_id`. One or more fields; removing the last token puts `_id` back.
  - Suggestions, best first: key patterns of **unique indexes** that exist on both sides (a
    compound unique index is one suggestion that fills all its fields), labelled "unique index";
    then unique on one side; then field names from the cached schema the query completion already
    uses (`src/views/documents/query_completion.rs`). Indexes come from `list_indexes_async`
    (`src/connection/ops/indexes.rs:48`), loaded for both sides once both are chosen. Until then the
    suggestions are schema fields only.
  - One advisory line under the control when no index covers the key on a side (§4.2): "No index
    starts with `sku` in Production · shop.orders, so the server sorts the collection first. That
    is slow on a large collection." It is a hint, not a block.
  - Nothing is said about uniqueness up front. If the key turns out not to be unique, the
    Multiple matches bucket shows it (§3.3), with a notice when it is clearly the wrong key (§3.8).
  - When the key is not `_id`, `_id` is left out of the comparison automatically (it differs on
    every pair by construction). The Options summary says so: "ignoring _id (matched by sku)".
- **Options** is a `Collapsible`, closed by default, with the same hand-made disclosure trigger as
  `src/components/connection_manager/tabs.rs:879`. Closed, it still says what is set ("No filter ·
  ignoring updatedAt, __v"): hidden content needs a visible cue. Inside:
  - **Filter**: one query input, applied to both sides. Parse with `parse_bson_from_relaxed_json`
    like the aggregation stages do. An unparseable filter disables Compare and says why inline.
  - **Ignore fields**: token input of dotted paths; reuse the chip look of `render_token_chip`
    (`src/components/filter_builder/panel.rs:2784`).
  - Nothing else. Number type and field order are handled by the Minor bucket (§4.4), so they need
    no switches. Good defaults over options.
- **Compare button**: primary, shows its shortcut with `Kbd`. While scanning it becomes **Cancel**
  (`src/components/busy_button.rs` pattern) and `escape` also cancels.
- **Disabled states**, each with its reason inline next to the button, never a dead button alone:
  same namespace on both sides; a side not chosen; a time-series side; filter does not parse.
- **Changing anything after a run** leaves the results on screen (the summary strip says what was
  compared, including the key) and shows "Settings changed. Compare again to update." beside the
  button.
- **Narrow windows.** The two picker groups sit in a wrapping flex row; below the width where both
  fit, Right drops under Left with the swap button between them. Inside a group the three selects
  wrap too, and so does the Match by / Options / Compare row. Use the same recipe as the fixed
  collection header (`src/views/documents/header/mod.rs:319`): `flex_wrap`, `min_w(0)` on the
  shrinking child, `max_w_full` on the actions.

### 3.2 Progress

One row between header and results, only while scanning or applying:
`Progress` bar + `412,000 of ~2.4M documents · 38,000/s` + nothing else. With a filter there is no
cheap total, so use `Progress::loading(true)` and show the counts only. The rate makes a long scan
feel alive; do not show an ETA (it would jump around). The whole app is monospaced, so the digits
do not jitter as they tick.

When a side has no covering index, the server sorts before it returns anything. Until that side's
first document arrives, the row says so per side: "Production is sorting on the server · Staging
120,000 read". Otherwise a long silence looks like a hang.

### 3.3 Summary strip

- A single-select segmented row: **All · Only in left · Only in right · Different · Minor**, plus
  **Multiple matches** when the key is not `_id`. Each has a glyph, label and live count. Build it
  from `ButtonGroup` (already used at `connection_manager/tabs.rs:86`) with the count in the label;
  `Tag` cannot be clicked and `Badge` is an overlay. The row wraps.
- Single-select on purpose. Each segment maps to one prebuilt index vector (§4.5), so switching is
  O(1) at any size. Multi-toggle would need merged indexes for every combination and buys little.
- **All** means the three real buckets. Minor and Multiple matches are separate: neither is a plain
  difference.
- Glyphs carry the meaning without colour: `+` only in left, `−` only in right, `≠` different,
  `≈` minor, `2×` multiple matches. Colour is a second channel: success, danger, warning and muted
  tints. Add `bg_added`, `bg_removed`, `bg_changed` beside `bg_dirty` in `src/theme.rs:169` (same
  recipe: theme colour at α 0.1) and extend the contrast test at `src/theme.rs:351` to cover them.
- The line under the segments is plain text, not controls: "9,832 identical" (identical documents
  are not stored, so there is nothing to list), then "37 without sku skipped" when documents lack
  the key (tooltip: "left 30, right 7. A document without the key cannot be matched."), then the
  time and duration. The timestamp follows the UTC/local date setting.
- **Find**: placeholder names the key ("Find _id…", "Find sku…"). Parse the text as ObjectId, UUID,
  number or string, scan the stored rows for a key (or any part of a compound key) equal to it,
  scroll to the hit and select it. A linear scan of 250,000 rows is milliseconds. No hit: say "Not
  among the differences", which is itself useful information.

### 3.4 Difference list (left of the split)

- Raw `uniform_list`, the pattern in `src/views/results/mod.rs:77`. It is the only list here that
  allocates nothing per row. The kit's `List` builds a size entry per row and scans linearly on
  selection (`gpui-component-0.6.2/src/list/cache.rs:91-101, 193-224`); `VirtualList` needs every
  row size up front.
- Row: checkbox · glyph · the **key value** via `bson_value_preview` (UUIDs and dates already
  render properly; a compound key joins its values with ` · `) · muted summary `3 fields: status,
  total, +1`. A Multiple matches row has no checkbox and its summary is `2 in left · 1 in right`.
  One fixed height; reuse the results view's.
- The list is append-only during a scan, so indexes and the selection stay stable while rows arrive.
- Hoist theme colours out of the row closure as `src/app/sidebar/view.rs:529-536` does.
- **Keyboard:** `up`/`down` move, `space` toggles the row's checkbox, `cmd-a` checks the whole
  segment, `enter` moves focus to the detail pane, `cmd-f` focuses Find. Shift-click range and
  cmd-click toggle: copy `aggregation_table_delegate.rs:232-285`, which already solves it.
- Checkboxes appear only once a sync target is chosen (§5.1). Before that the list is for reading.
- Own key context on the view (`"Compare"`, `"Compare CompareRunning"`), like
  `src/views/transfer/mod.rs:666-672`.

### 3.5 Document diff (right of the split)

- Split with `h_resizable`, same shape and limits as the Schema tab
  (`src/views/documents/views/schema_view.rs:255`): list left, detail right.
- The detail is fetched on selection: two point reads, each by that side's own `_id` (stored on the
  row, §4.5), run together on the connection runtime. Rules that keep arrow-key browsing smooth:
  - debounce 80 ms while a key is held, and tag each request with a generation so a late reply for
    an old row is dropped;
  - keep the previous diff on screen until the new one arrives, and show skeleton rows only if the
    fetch passes 150 ms (no flash for fast servers);
  - cache the last 64 pairs in a map, cleared when full or on a new compare.
- Header: the key and its value, **Copy**, **Open in left**, **Open in right**. These open the
  collection in a new tab filtered to that document, with the opener reference-following already
  uses: `open_collection_in_new_tab(database, collection, raw, Some(filter), cx)`
  (`src/state/commands/relations.rs:275`).
- Body: a three-column tree on `uniform_list`: field, left value, right value. Follow
  `src/views/documents/tree/lazy_tree.rs` (flattened `VisibleRow`s), not the kit `Tree`, whose
  items hold a single label.
  - Only changed fields by default. Ancestors of a nested change are open.
  - Unchanged siblings fold into one row, `▸ 14 unchanged fields`, which expands in place. That is
    the visible cue for hidden content.
  - Changed rows get `bg_changed`; a field present on one side only gets `bg_added` or `bg_removed`
    with the other cell empty. Values keep their syntax colour (`bson_tree_value_color`,
    `document_tree.rs:201`).
  - When only the type differs, show the type on both sides (`Int32 120` / `Double 120`). This is
    where a Minor difference explains itself.
  - With a key other than `_id`, the `_id` row is always shown first, muted and untinted: it
    differs by design and is information, not a change.
  - Long values truncate; the existing value hover card shows them in full.
  - Arrays are compared by position (§4.4).
- One-sided documents show the document in its column and a muted "Not in production · shop.orders"
  in the other.
- A **Multiple matches** row shows no diff. It lists the documents that share the key on each side
  (a find by the key values, limit 20 per side, `_id` and a one-line preview each), with "Open in
  left / right" opening the collection filtered by the key so the user can fix the duplicates.
- If a fetched document no longer matches the hash recorded by the scan, a single line above the
  diff says "Changed since the comparison" and the diff shows the current state.

### 3.6 Sync bar

Bottom of the tab, in normal flow (not floating), top border, visible once a run has finished or
was cancelled with at least one difference. It wraps at narrow widths and its button never clips.

- **Target first, no default.** `Write changes to` + two options that name the real thing with its
  identity badge: `● Staging · shop.orders` / `● Production · shop.orders`. Writing the wrong way
  round is the catastrophic mistake in every sync tool; naming the target, and forcing a choice,
  removes it. Arrows and "left → right" are avoided on purpose. A read-only connection or a view is
  a disabled option whose tooltip gives the existing reason
  (`session_read_only_reason`, `src/state/app_state/connection.rs:182`).
- **Four checkboxes are the selection.** `Insert 120 missing`, `Replace 45 different`,
  `Replace 212 minor`, `Delete 3 extra`, worded from the target's point of view. Defaults once a
  target is chosen: Insert on, Replace different on, Replace minor off, Delete off. Unchecking rows
  in the list turns the matching box indeterminate and its label into `Replace 43 of 45`. One
  concept, not a list selection plus a separate set of toggles.
- Multiple matches are never part of a sync. When there are any, a muted note at the end of the
  bar says "4 keys with multiple matches are left alone."
- The kit's styled `Checkbox` is two-state. The tri-state lives one layer down
  (`gpui_base::Checkbox`, `CheckboxState::Indeterminate`, `gpui-base-0.6.2/src/checkbox.rs:17`).
  Add a small styled wrapper in `src/components/`; it is the only new generic component.
- Switching the target resets row exceptions (their meaning flips) and re-applies the defaults.
- **Review and sync…** is disabled with a reason until a target is chosen and something is checked.

### 3.7 Review, apply, undo

- **One dialog.** Pass a `WriteConfirmation` into `request_connection_write`
  (`src/components/confirm.rs:78`) so the production gate and the review are the same dialog. Follow
  the wording rule in `src/components/node_commands.rs:41` (the title is the decision and names its
  object; the body says only what the title cannot):
  - title: `Write 165 changes to "Production · shop.orders"?`
  - body: `Insert 120` / `Replace 45` / `Delete 0`, then "Documents that changed since the
    comparison are skipped. You can undo this from the Compare tab until you close it." With a key
    other than `_id`, one more line: "Replaced documents keep their _id in Production."
  - confirm label `Write 165 changes`; `destructive: true` when anything is replaced or deleted.
    Cancel keeps default focus (existing behaviour, `confirm.rs:304`).
- Between confirmation and execution, re-check that the plan is unchanged, as the Transfer tab does
  (`src/views/transfer/mod.rs:207-221`).
- **While applying**, the sync bar becomes progress + Cancel (cancel stops after the current batch).
  Rows gain an outcome glyph: `✓` written, `↻` skipped (changed since the comparison), `!` failed
  with the server's message in a tooltip.
- **After:** a success `Notification`, and the bar shows `Wrote 163 · skipped 2 · failed 0`,
  **Undo** and **Compare again**. Undo goes through the same gate and dialog
  (`Undo 163 changes in "Production · shop.orders"?`).

### 3.8 States

| State | What shows |
| --- | --- |
| Before the first run | Centered hint in the results area: "Pick two collections and press Compare", with the `Kbd` hint |
| Scanning | Progress row; segments count up; rows stream in; everything is inspectable |
| A side is sorting on the server | The per-side line in §3.2 until its first document arrives |
| No differences | "No differences. 9,832 documents are identical." plus "212 minor differences" as a link to that segment when there are any |
| The key is clearly not unique | Once 1,000 keys have multiple matches, a notice above the list: "`status` does not identify a document: 1,000 values appear more than once. Pick a field that is unique, such as one with a unique index." The scan keeps running; the user decides |
| Cancelled | Results so far stay, labelled "Cancelled at 412,000 documents"; sync is allowed on what was found |
| Cap reached | Banner: "Showing the first 250,000 differences. Counts are exact. Sync these, then compare again for the rest." |
| The key cannot be ordered | Scan stops with the reason (§4.3): a Decimal128 key, or a key path that passes through an array |
| Scan failed | Inline error card with the message and Retry; also `report_compare_error`, a twin of `report_transfer_error` (`src/state/app_state/errors.rs:119`) so a background failure notifies |
| Connection closed | Results stay readable; Compare, detail fetches and sync are disabled with the reason |

### 3.9 Motion

Following the frequency rule: row selection, segment switching, diff swapping, fold expanding and
every keyboard action are instant, with no transition. The kit's own motion stays as shipped:
`Progress` value easing, the dialog's 250 ms, toast entry, and the Options `Collapsible` reveal
(matching the connection manager). Nothing new is animated, and every state change has a static
cue (glyph, label or count), so motion is never the only signal. The kit honours reduced motion.

---

## 4. Engine (the performance part)

The cost of a compare is reading two collections. Everything below exists to read each document
once, touch identical documents as little as possible, and keep memory flat.

### 4.1 Read path: a streaming merge join on the key

- Both sides: `find(filter).sort(key)` on `Collection<RawDocumentBuf>`, where `key` is the chosen
  fields ascending. With an index that covers the key (always true for `_id`) the server walks the
  index, does no in-memory sort, and the client never holds a collection.
- The installed driver exposes the raw cursor API (`Cursor::advance` and `Cursor::current ->
  &RawDocument`, `mongodb-3.5.1/src/cursor.rs:189, 222`). Nothing in the app uses raw BSON yet.
  Raw documents are what make the fast path in §4.4 possible.
- Leave `batch_size` unset: after the first batch the server sends up to 16 MB per `getMore`,
  which is the fewest round trips.
- Shape: two reader tasks and one joiner, all on the connection manager's Tokio runtime
  (`runtime.spawn`, idiom A in `src/state/commands/documents/query.rs:199-275`; AGENTS.md forbids
  driver calls on gpui's executor). Each reader sends `Vec<RawDocumentBuf>` chunks of 1,024
  documents into a bounded `tokio::sync::mpsc` channel (capacity 4 chunks). The joiner pops from
  both and advances whichever side has the smaller key. Bounded channels give backpressure, so
  memory is a few chunks per side regardless of collection size.
- **Keepalive.** With skewed key ranges, or while the other side sorts, one side can wait a long
  time. The server kills an idle cursor after 10 minutes, and an idle session after 30 even with
  `noCursorTimeout`
  ([manual](https://www.mongodb.com/docs/manual/reference/method/cursor.noCursorTimeout/)); each
  `getMore` resets both. So a reader that has been blocked on `send` for 5 minutes pulls one more
  chunk into a local overflow queue. Worst case is one extra chunk per 5 minutes of waiting.
- No `max_time`: like export and copy (`src/connection/ops/copy.rs`), a compare is exempt from the
  interactive query timeout.
- Cancellation: the existing `CancellationToken` (`src/connection/types.rs:143`), checked per chunk.

### 4.2 Keys other than `_id`

`_id` always exists, is never an array, is unique and is indexed, so for `["_id"]` none of the
rules below add anything to the query. They apply to every other key.

- **Documents without the key are skipped and counted.** A sort cannot tell `null` from a missing
  field ("The comparison treats a non-existent field as if it were null",
  [manual](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)), and a
  document without the key cannot be matched anyway. AND `{field: {$exists: true}}` into the find
  for every key field. An explicit `null` is a value like any other.
- **Array values are skipped and counted too.** The server sorts an array by its smallest element,
  which the client cannot reproduce. AND `{field: {$not: {$type: "array"}}}` into the find.
- The "skipped" number in the summary comes from one `count_documents` per side, run beside the
  scan: the user filter AND (`$or` of "field missing" and "field is an array" for each key field).
- **Index cover and sort order.** Matching does not depend on the order of the key fields, so sort
  in the order an index provides:

  ```rust
  /// The order to sort the key fields in, and which sides have an index that covers it.
  pub fn sort_plan(fields: &[String], left: &[IndexModel], right: &[IndexModel]) -> SortPlan
  ```

  An index covers the key when its first `fields.len()` fields are exactly the key's field set, all
  ascending or all descending, and it is not partial, hidden or sparse (being conservative only
  costs an unnecessary hint). Prefer an order both sides cover, then the larger side's, then the
  order typed. A pure function with a table test.
- A side that is not covered gets `allow_disk_use(true)`, the advisory line in §3.1, and the
  "sorting on the server" state in §3.2.
- **Multiple matches.** The joiner groups consecutive documents with equal keys on each side. One
  on each side → compare them. Otherwise the key goes to the Multiple matches bucket as **one row**
  carrying the two counts. A group is never buffered: on the second document with the same key the
  joiner drops the first and only counts, so a key like `status` with 300,000 documents per value
  still costs one document of memory per side.

### 4.3 Ordering keys on the client

A merge join needs the client to order keys exactly as the server sorted them. Put this in
`src/bson/compare.rs`:

```rust
/// MongoDB's sort order for one key value. `None` means "cannot order safely".
pub fn cmp_key_value(a: RawBsonRef<'_>, b: RawBsonRef<'_>) -> Option<Ordering>
/// Lexicographic over the key fields, in sort order.
pub fn cmp_keys(a: &[RawBsonRef<'_>], b: &[RawBsonRef<'_>]) -> Option<Ordering>
```

Rules, from the [comparison order page](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/):
type brackets MinKey < Null < numbers < String/Symbol < Object < Array < BinData < ObjectId <
Boolean < Date < Timestamp < Regex < MaxKey; numbers compare by value across Int32, Int64 and
Double; strings compare as bytes (Rust's `str` ordering is bytewise); BinData by length, then
subtype, then bytes; embedded documents pair by pair (type, then key, then value), shorter first.
Return `None` for Decimal128 against another numeric type and for regex or code values.

A key path may be dotted (`customer.email`). Extract it from the raw document without parsing the
rest. If the path passes through an array in any document, stop the scan with "`customer.email`
passes through an array in some documents, so it cannot be used to match": that is a multikey
sort, and `$type` on the final value does not catch it.

Three guards make this safe rather than hopeful:

1. **Monotonicity check.** For every document, assert its key is not less than the previous key
   from the same side. If `cmp_keys` ever disagrees with the server's order, or returns `None`,
   stop with a clear message instead of producing a wrong diff. Cost: one comparison per document.
2. **Collation.** A collection's default collation changes the sort order of strings
   ([createCollection](https://www.mongodb.com/docs/manual/reference/method/db.createCollection/)).
   The sidebar's `CollectionDetail` does not carry collation, so at scan start run one
   `listCollections` per side filtered by name and read `options.collation`. If a side has one, add
   `collation: {locale: "simple"}` and `allow_disk_use(true)` to that side's find, and tell the
   user the scan will be slower: its indexes were built with the collection's collation, so the
   server sorts instead of walking one. Collections without a default collation take the normal
   path untouched.
3. **Integration test against a real server** (§9): insert keys of every supported type, read them
   back sorted by the server, assert `cmp_keys` agrees on every neighbouring pair.

### 4.4 Comparing two documents

```rust
pub enum Verdict { Same, Minor(MinorFlags), Different { changed: u16, first_paths: Box<str> } }
// MinorFlags: FIELD_ORDER | NUMBER_TYPE

pub fn compare_raw(left: &RawDocument, right: &RawDocument, ignore: &IgnoreSet) -> Verdict
```

`IgnoreSet` is the user's ignore list, plus `_id` whenever the key is not `_id`.

1. **Fast path:** `left.as_bytes() == right.as_bytes()` → `Same`. One `memcmp`, no parsing, no
   allocation. In a staging copy of production matched by `_id` this is nearly every document.
   With another key the `_id`s differ, so every pair takes the walk below; it is allocation-free,
   and the per-element byte comparison keeps it cheap.
2. **Walk** both raw documents in lockstep without building a `Document`. While keys line up,
   compare each element's bytes; skip ignored paths; recurse into sub-documents and arrays.
3. If the keys do not line up, compare that level by key (small map built on the spot) and set
   `FIELD_ORDER`.
4. Two numbers of different types that are equal by value set `NUMBER_TYPE`. Compare Int64 and
   Double exactly (`(f as i64) as f64 == f && f as i64 == i`), never through a lossy cast.
   Decimal128 equals only an identical Decimal128. `// ponytail:` note that in the code.
5. Arrays compare by position. Order-insensitive arrays are under Later.
6. The verdict is `Different` if any value differs, else `Minor` if any flag is set, else `Same`
   (same content with ignored fields differing is `Same`).
7. `Different` carries the changed-leaf count (saturating `u16`) and the first three paths joined,
   for the row summary.

The ignore list is applied here, on the client, not as a server projection. That keeps the stored
hashes equal to the hash of the full document, which the stale guard in §5.2 depends on. A
projection pushdown is an optimisation for later, if someone ignores a huge field.

The detail pane uses a separate, simpler function on parsed documents, since it handles one pair:

```rust
pub struct FieldChange { pub path: Vec<PathSegment>, pub left: Option<Bson>, pub right: Option<Bson> }
pub fn field_changes(left: &Document, right: &Document, ignore: &IgnoreSet) -> Vec<FieldChange>
```

Both functions must agree. A property-style unit test asserts that `field_changes` is empty exactly
when `compare_raw` says `Same`, and non-empty with only type/order notes exactly when it says
`Minor`. This is the Studio 3T "overview ignores my ignore list" bug, prevented by a test.

### 4.5 What is stored

```rust
pub enum DiffKind { OnlyLeft, OnlyRight, Different, Minor, MultipleMatches }
pub struct DiffRow {
    pub key: Bson,                // the key value; a Document of the key fields when compound
    pub left_id: Option<Bson>,    // that side's `_id`; None when absent, and when the key is `_id`
    pub right_id: Option<Bson>,
    pub kind: DiffKind,
    pub changed: u16,             // MultipleMatches reuses `changed`/`paths` for nothing
    pub paths: Box<str>,
    pub left_hash: u64,           // 0 when the side is absent
    pub right_hash: u64,
    pub left_count: u32,          // MultipleMatches only
    pub right_count: u32,
}
```

- One accessor hides the `_id` shortcut: `row.id_on(side, key_is_id) -> Option<&Bson>`.
- Identical documents increment a counter and are dropped.
- Hashes: `std::hash::DefaultHasher` over the raw bytes, only for documents that differ. They live
  in memory for one run, so the hasher's cross-version instability does not matter.
- `rows: Vec<DiffRow>` for the three real kinds, `minor` and `multiple` apart, plus one `Vec<u32>`
  of row indexes per real kind. Every segment of the summary strip is a direct index lookup.
- Cap: 250,000 stored rows in total. Past it, keep counting and set `truncated`.
  `// ponytail: Vec<DiffRow> with a 250k cap; an estimate of 40 MB worst case matched by _id and
  about 90 MB with another key; upgrade path is an id arena or spilling to the sqlite store`.

### 4.6 Reaching the UI

- The joiner sends `CompareMessage::Progress { counts, new_rows, left_started, right_started }` at
  most every 100 ms (check the clock every 1,024 documents), then `Done { counts, skipped,
  truncated, elapsed }` or `Failed`.
- A `cx.spawn` loop drains the `futures::channel::mpsc` receiver, appends rows to the tab state and
  calls `cx.notify()` once per message, the shape of
  `src/state/commands/transfer/copy.rs:450-540` but throttled by time, not by message count.
- Ten UI updates a second at most, whatever the document rate. No locks: the UI thread owns the
  rows.

### 4.7 Considered and rejected

| Idea | Why not |
| --- | --- |
| Server-side hashing with `$toHashedIndexKey` so only hashes cross the network | Unsafe. "Hashed indexes truncate floating-point numbers to 64-bit integers before hashing … the same hash to store the values 2.3, 2.2, and 2.9" ([hashed indexes](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-hashed/)). A price change would be reported as identical |
| `dbHash` as an "are they identical?" shortcut | "obtains a shared (S) lock on the database, which prevents writes until the command completes" ([dbHash](https://www.mongodb.com/docs/manual/reference/command/dbHash/)). Not something to run against production; also unsupported on Atlas M0/Flex |
| Hash map of one side, then probe with the other (needs no sort, so no index) | Memory grows with collection size. The merge join is O(1); an uncovered key pays a server-side sort instead |
| Adding `_id` to the sort as a tiebreaker for equal keys | A `{sku: 1}` index cannot provide `{sku: 1, _id: 1}`, so it would force a server sort on every custom key. Order inside a group of equal keys does not matter: the group goes to Multiple matches as a whole |
| Paging by key ranges (`$gt` last key) instead of long cursors | `$gt` is type-bracketed, so a resume point loses values of other types. The keepalive is simpler |
| `$hash` / `$hexHash` (mentioned on the `$toHashedIndexKey` page as general-purpose hashing) | Not verified: server version, and whether they accept whole documents. Revisit under Later; it is the one route to a large WAN speed-up |

---

## 5. Sync

### 5.1 Plan

```rust
pub struct CategorySelection { pub all: bool, pub exceptions: HashSet<u32> } // row indexes
pub struct SyncPlan { pub target: Side, pub inserts: Vec<u32>, pub replaces: Vec<u32>, pub deletes: Vec<u32> }
```

`all` plus `exceptions` keeps "everything checked" and "one row unchecked out of 200,000" equally
cheap, and yields the tri-state directly: no exceptions and `all` → checked; no exceptions and not
`all` → unchecked; otherwise indeterminate.

Three operations, the same for every key:

| Operation | Rows | What is written |
| --- | --- | --- |
| **insert** | only in the source | The source document, with the source's `_id`, as a plain insert. **Never an upsert:** with a key other than `_id`, the target may hold an unrelated document that happens to use that `_id`, and an upsert would silently replace it. An insert fails on it instead, and the row reports "a different document in the target already uses this _id" |
| **replace** | different, and minor when checked | The source document's content under the **target's** `_id` (`_id` is immutable, and with `_id` as the key the two are the same value). No upsert |
| **delete** | only in the target | By the target's `_id` |

Multiple matches are never planned.

### 5.2 Apply, in batches of 1,000 rows

1. Read the batch's documents from **both** sides on the primary, raw, **without the compare
   filter**, by match key (`$in` for one field, literal `$eq` filters in `$or` for compound keys).
   This detects new duplicates as well as stale documents and changed identities. Match replies
   using the same BSON ordering as comparison. Split batches when a side exceeds 32 MiB of retained
   documents or the query exceeds 12 MiB.
2. **Stale guard.** Hash each reply and compare it with the row's `left_hash` / `right_hash`; a
   side that was absent must still be absent. Any mismatch → skip the row with outcome "changed
   since the comparison". Reading without the filter also covers the filtered-compare trap: a
   document that exists in the source but fell outside the filter is found here, so it is never
   deleted from the target as "extra", and one that exists in the target outside the filter is
   never inserted twice.
3. Append the target-side before-images to the restore file (§5.4).
4. Write with `Client::bulk_write(models).ordered(false).verbose_results()` on MongoDB 8.0+.
   The driver handles wire-message splitting. Inserts are plain inserts, never upserts;
   replacements retain the target `_id`. Replacement and deletion filters include the expected
   target document through `$expr` / `$literal`, using the existing editor's conditional-write
   pattern. This also detects target changes between the read and the write under MongoDB equality
   semantics. There is no per-document compatibility path for older servers.
5. Map individual insert/update/delete results and write-error indexes back to rows. A matched
   count of zero is skipped, not counted as written. Partial successes retain their undo records.
   Missing acknowledgements are marked uncertain and stop further batches; guarded undo remains
   available for those possible writes. Write concern must be acknowledged (`w != 0`).

The [Rust driver's bulk-write API requires MongoDB 8.0+](https://www.mongodb.com/docs/drivers/rust/current/crud/bulk/).

No transactions: they need a replica set and have size and time limits. Unordered batches with
per-row outcomes, a stale guard and an undo are the safety model.

### 5.3 Write gate

- Target connection: `request_connection_write` with `WriteRequest::new(connection_id, target_ns,
  "Sync differences", Some(confirmation))` (§3.7). Read-only connections are blocked by the gate;
  production authorisation is consumed once by `ensure_writable` in the command
  (`src/state/commands/mod.rs:12`), as copy does.
- Views: refuse before spending an authorisation, via `ensure_collection_writable`
  (`src/state/commands/mod.rs:47`). The target option is already disabled in the UI; this is the
  backstop.
- The source side is only read.

### 5.4 Restore file and undo

- One file per sync under `config_dir/compare-undo/`, created through `ConfigManager`
  (`src/state/config.rs`). Frames are length-prefixed and encrypted with AES-256-GCM (`aes_gcm` is
  already a dependency, see `src/history/crypto.rs`) under a random key generated for the tab and
  held only in memory; the nonce is the frame counter. Production documents never sit on disk in
  the clear, and the file is unreadable once the app exits, which matches an in-session undo.
  `HistoryCipher` itself is tied to History items, so do not bend it; use `aes_gcm` directly.
- A frame lists `{op, target_id, before: Option<RawDocumentBuf>, after_hash}` per written row.
  The after-image fingerprint is SHA-256. Frames are authenticated, sequence-checked, flushed and
  synced to disk before a batch can write. In-memory frame status distinguishes prepared,
  possibly written, confirmed, and inactive records.
- **Undo** builds the inverse plan and runs the same apply engine: an insert becomes a delete of
  that `_id`, a replace becomes a replace with the before-image, a delete becomes an insert of the
  before-image. Its stale guard compares the target's current hash with `after_hash`, so anything
  edited since the sync is skipped and reported, not clobbered.
- The file is deleted when the tab closes or a new compare starts, and the directory is swept at
  startup. Locked files belonging to live app instances are preserved. An in-flight batch keeps
  its file alive until it stops. Undo reads encrypted frames and current documents in byte-bounded
  batches and uses the same native bulk writer. Disk, not memory, because before-images can be
  hundreds of megabytes.
- Undo removes inserted documents before reverting replacements and restoring deletions, so a
  delete/insert pair that reused an `_id` can restore the original without relying on unordered
  bulk execution order.
- History, where a connection has it enabled, records these writes as well. The two are
  independent.

---

## 6. Components

| Need | Use | Precedent |
| --- | --- | --- |
| Pickers | `Select`, `SelectState`, `SearchableVec`, `ConnectionItem` | `src/views/transfer/select_states.rs:15-68` |
| Match by, Ignore fields | token input with suggestions, chips | `filter_builder/panel.rs:2784`; field names from `documents/query_completion.rs` |
| Options | `Collapsible`, `Input` | `connection_manager/tabs.rs:879` |
| Progress, cancel | `Progress`, `Spinner`, busy button | `views/transfer/progress_panel.rs:271`, `components/busy_button.rs` |
| Segments | `ButtonGroup`, count in the label | `connection_manager/tabs.rs:86` |
| Long list | `uniform_list` | `views/results/mod.rs:77` |
| Multi-select | hand-rolled range and toggle | `documents/table/aggregation_table_delegate.rs:232-285` |
| Split | `h_resizable`, `resizable_panel` | `documents/views/schema_view.rs:255` |
| Diff tree | flattened rows on `uniform_list` | `documents/tree/lazy_tree.rs`, `lazy_row.rs` |
| Confirm | `request_connection_write` + `WriteConfirmation` | `components/confirm.rs:78` |
| Toasts, tooltips, keys | `Notification`, `Tooltip`, `Kbd` | `app/root.rs:690`, `components/action_bar/mod.rs:389` |

**Custom work, all of it:** the tri-state checkbox wrapper; the diff tree rows; the list's
multi-select; the three tint helpers. The kit has no diff viewer of any kind. gpui-kit 0.6.3–0.6.6
are patch releases with nothing relevant, so no upgrade is needed.

---

## 7. Fitting into the app

Model the tab on **Forge**: keyed by tab id, a per-tab state map, persisted with one payload field.
The full touch list is long; the sites are, in order of the compiler errors they produce:

- `src/state/app_state/types.rs:20, 170` — `View::Compare`, `TabKey::Compare(CompareTabKey)`,
  `CompareTabKey { id, connection_id }`, `CompareTabState`.
- `src/state/app_state/mod.rs:127` — `compare_tabs: HashMap<Uuid, CompareTabState>`;
  new `src/state/app_state/compare.rs` copied from `transfer.rs` (33 lines).
- `src/state/app_state/tabs/model.rs` — `apply_tab_selection` (:95), `open_compare_tab(prefill)`
  beside `push_transfer_tab` (:564), `close_tab` (:980), `tab_kind_label` (:1335).
- Persistence: `src/state/workspace.rs:12, 46` (`WorkspaceTabKind::Compare`, payload
  `compare: Option<CompareConfig>`), `tabs/persistence.rs` (:17, :93, ~:342, :425),
  `app_state/workspace.rs:62`. Persist the **config only** (sides, match key, filter, ignore
  list). Results are not persisted; a restored tab is ready to run again.
- `src/state/app_state/connection.rs:373` — the tab belongs to **both** of its connections, so
  closing either one disables it (§3.8) rather than closing it.
- Rendering: `src/components/content/mod.rs` (`ContentArea` field, `ensure_views`, flags, focus) and
  `content/tabs.rs` (:159, :248, :623). Note the comment at `content/mod.rs:273` about collapsing
  the per-view fields once a tenth view arrives; count them before adding.
- Entry points: `src/app/menus.rs:382` (`build_collection_menu`: "Compare with…" next to "Copy
  data…", :604), `src/components/action_bar/providers.rs:70, 162`, `src/app/actions.rs:148`
  (`"cmd:compare"`), `src/keyboard.rs:11, 153` (actions `OpenCompare`, `RunCompare`,
  `CancelCompare`; bind `cmd-enter`/`ctrl-enter` and `escape` in the `Compare` contexts; give
  `OpenCompare` no global shortcut, the palette covers it).
- Events and errors: `src/state/events.rs:11`, `src/state/app_state/status.rs:208`,
  `src/state/app_state/errors.rs:119`.

**New files**

```
src/bson/compare.rs                      cmp_key_value, cmp_keys, compare_raw, field_changes
src/connection/ops/compare.rs            sort_plan, merge-join scan, collation probe, apply engine
src/state/app_state/compare.rs           accessors
src/state/commands/compare/{mod,scan,sync}.rs
src/views/compare/{mod,setup,match_key,summary,diff_list,doc_diff,sync_bar,layout_tests}.rs
src/components/tri_checkbox.rs
tests/compare_tests.rs
```

---

## 8. Build order

Three pull requests. Each leaves the app shippable. The match key is in all three from the start:
the engine takes a key, the screen has the control, sync follows the `_id` rules in §5.1.

**Slice 1 — engine, no UI.** `src/bson/compare.rs` and the keyed scan in
`src/connection/ops/compare.rs` (key filters, `sort_plan`, multiple matches, skipped count), with
unit and integration tests, and the measurement in §9. Nothing user-visible. Do this first: if the
numbers disappoint, the design changes before any UI exists.

**Slice 2 — the Compare tab, read-only.** Tab plumbing, setup header with Match by, progress and
cancel, summary strip, list, document diff, the Multiple matches detail, states, entry points,
layout tests. Zero write risk. This is a complete feature on its own ("Compare"), and the changelog
can say so.

**Slice 3 — sync.** Tri-state checkbox, selection model, sync bar, plan, apply engine with the
stale guard, restore file, undo, the review dialog.

Each slice also updates `docs/features.md` (:45 is this feature) and the README's "Copy data
between environments safely" bullet (`README.md:73`), and gets its own changelog commit. That
bullet's "sync a whole database" is the agent-started database replace, a different feature; word
the new text so the two are not confused.

---

## 9. Tests and measurements

**Unit** (`cargo test --lib`; the AGENTS.md `--bin` command runs nothing):

- `cmp_key_value` / `cmp_keys`: a table across every type bracket, numbers across
  Int32/Int64/Double, strings with non-ASCII bytes, BinData by length then subtype,
  embedded-document values, compound keys; `None` for Decimal128 against Int32.
- Key extraction: dotted paths; a path through an array is reported, not guessed.
- `sort_plan`: an index with the fields in another order reorders the sort; a partial, hidden or
  sparse index does not count; "both sides" beats "one side" beats "as typed".
- `compare_raw`: byte-identical → Same; reordered keys → Minor(FIELD_ORDER); `1` against `1.0` →
  Minor(NUMBER_TYPE); `2^53 + 1` as Int64 against the nearest Double → Different; NaN against NaN →
  Same; an ignored path that differs → Same; `_id` ignored when the key is not `_id`; nested and
  array changes produce the right paths.
- The agreement test between `compare_raw` and `field_changes` (§4.4).
- Grouping: equal keys on either side produce one Multiple matches row with the right counts, and
  the joiner holds no more than one document per side while counting a large group.
- Selection tri-state and counts; `SyncPlan` from rows + selection + target; the inverse plan.
- Restore-file frame round trip, and a decrypt failure with the wrong key.

**Integration** (`tests/compare_tests.rs`, Testcontainers, `--test-threads=1` like the others):

- Keys of every supported type: server sort order equals `cmp_keys` order on every neighbouring
  pair.
- Two collections with a known set of differences, matched by `_id`: exact counts and kinds.
- The same data with different `_id`s on each side, matched by `sku`: the same counts; then by a
  compound key whose index lists the fields in the other order.
- Documents without the key, and with an array in it, are skipped and counted; a duplicate key
  lands in Multiple matches and nowhere else.
- A collection created with a non-simple default collation and string keys compares correctly.
- Cancel mid-scan returns promptly and leaves partial results.
- Sync into the right side, compare again: zero differences. Then undo, compare again: the original
  differences are back. Run once by `_id` and once by `sku`; by `sku`, replaced documents keep the
  target's `_id`.
- Insert collision: a target document that already uses the source's `_id` under another key makes
  that row fail, and the unrelated document is untouched.
- Stale guard: change a document between compare and sync; it is skipped and untouched.
- Filtered compare: a document outside the filter in the source is never deleted from the target.
- A view as a side compares; as a target it is refused without spending an authorisation.

**Layout** (copy `bounds_at` from `src/views/documents/header/mod.rs:420`): at 430, 900 and 1200 px
the two picker groups never overlap, the Match by / Options / Compare row stays inside the header,
and the sync bar's button stays inside the tab.

**Measurements, to record in the slice 1 pull request** (targets to check, not promises): seed two
local collections of 1,000,000 documents of about 1 KB with 1% differing (`scripts/compare-bench-seed.js`).
Record documents per second, peak resident memory, and whether memory stays flat as identical
documents stream past, for three runs: matched by `_id`; matched by an indexed `sku` with different
`_id`s (every pair takes the walk, so this shows its real cost); matched by an unindexed field
(shows the server sort). Then repeat with the UI in slice 2 and confirm the window stays responsive
while scanning. If throughput is dominated by the two reader tasks' copies, measure the single-task
variant that holds both cursors and compares `cursor.current()` borrows with no channel.

---

## 10. Risks and things not verified

- **Driver raw path.** `Cursor::advance`/`current` exist in 3.5.1, but nothing in the app uses
  them yet; confirm `Collection<RawDocumentBuf>` with `find().sort()` behaves as expected in the
  first hour of slice 1.
- **Uncovered keys on large collections.** A server-side sort of millions of documents is slow and
  uses disk on the server. The hint, the per-side "sorting" state and Cancel are the mitigations;
  there is no way to make it fast without an index, and the UI should not pretend otherwise.
- **Whether the planner walks the index with the key filters attached** (`$exists`, `$not $type`).
  Expected, not verified: check one `explain` in slice 1 and drop or reshape the predicates if they
  cost the index.
- **Raw `update` command.** Reply parsing (`writeErrors`, `n`) is hand-written, and the reply does
  not say *which* statement matched nothing. A document deleted in the instant between the stale
  guard and the write shows up only as a batch-level "not found" count. Cover the parsing in the
  integration tests, including a deliberate duplicate-key failure.
- **Long scans.** The keepalive rests on the manual's statement that each `getMore` refreshes the
  session. Test with an artificially stalled side.
- **Scans are not snapshots.** Documents written during a scan may or may not be seen. The stale
  guard protects writes; the "compared at" time tells the user what they are looking at.
- **Sharded clusters** sort through mongos and should just work; untested.
- Not verified by research: Studio 3T's behaviour at scale; whether `dbHash` works through mongos;
  the server version and input types of `$hash`/`$hexHash`.

## 11. Later

Shipped since: comparing two documents picked in a collection view, links from the skipped
count that open those documents, connecting a saved connection from the pickers, and whole-database
compare with index differences and sync by collection (`docs/COMPARE_DATABASE_PLAN.md`), and
Ignore array order, which counts reordered arrays as minor, and single-field copy in the document
diff: a guarded replace of the target with one path taken from the source, sharing the sync's undo.
Read-only MCP compare tools also shipped, running as MCP tasks for clients that support them.

- Undo that survives a restart (decision 5 is "for now"): needs a persisted key, so it belongs with
  the keychain-backed History key, and a list of past syncs to undo from.
- Export the difference list as CSV (1 vote).
- Saved comparisons, then scheduling, once tasks exist (`docs/features.md:43-44`).
- Compare validators (indexes are compared in database scope).
- A server-side hashing fast path for slow links, if `$hash` proves safe.
