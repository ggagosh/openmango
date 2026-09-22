# Compare UX revision

The latest screenshots showed two separate problems: editing controls crowded the main header,
and virtualized document rows did not align with their column headings. The earlier dropdown
outer-sizing repair is retained.

Applied `better-ui`, `better-layout`, `emil-design-eng`, `better-typography`, and
`better-accessibility`: separate setup from inspection, use clear grouping and native controls,
keep repeated keyboard interactions immediate, preserve full values behind truncated previews,
and retain useful hit areas in narrow panes.

| Severity | Location | Before | After | Why |
| --- | --- | --- | --- | --- |
| HIGH | `src/views/compare/setup.rs:554` | Match tokens, custom inputs, suggestions, options and actions shared one crowded toolbar | A native Comparison settings popover groups matching keys, filter and ignored fields; the main bar shows a compact summary and Compare action | Settings no longer push inspection out of view |
| HIGH | `src/views/compare/detail.rs:268` | Virtual rows shrank to their text; a value measured 8 px wide, starting hundreds of pixels before its heading | Header and every row share a full-width layout with a bounded field column and equal Left/Right value columns | Values, empty cells and nested fields stay under the correct headings |
| MEDIUM | `src/views/compare/detail.rs:440` | Highlighted backgrounds formed ragged text-width patches and absent sides looked ambiguous | Cell-specific highlights, explicit missing-value dashes, and a “No matching document” column state | Presence and differences are clear without relying on color |
| MEDIUM | `src/views/compare/results.rs:284` | One-sided results reserved an empty subtitle line | Compact single-line result rows with inline summaries where useful | Consistent row alignment and more visible results |
| MEDIUM | `src/state/compare.rs:55` | Adding the first custom key silently retained default `_id` | First custom key replaces default `_id`; subsequent keys are explicitly compound | Copies with different MongoDB IDs are not accidentally matched on both fields |

Existing compound key configurations are preserved. Settings explains that every key field must
match and offers **Match without _id**. The indexed-key selector also clears a stale suggestion
when it no longer represents the configured key.

Keyboard checks use the real rendered controls: indexed-key selection, Escape from the nested
selector, typing/Enter to add a custom key without closing settings, and Done committing pending
field text before closing. Enter is handled only around the editable fields, so it does not
intercept the nested selector's keyboard confirmation.

## Validation

- `cargo test --lib compare:: -- --test-threads=1`: **21 passed**.
- Includes six Compare UI/state cases, with actual dropdown hitboxes and settings containment
  at 430/900/1200/1750 px.
- Opening settings leaves the main header and results in place.
- Inspector cell bounds match header bounds at 430/700/1000/1750 px, including long values,
  an absent side, and a six-level nested document.
- Deep disclosure buttons remain inside their field cells with at least 24 px width.
- Formatting and `cargo clippy --lib --bin openmango --tests -- -D warnings` passed;
  the mechanical layout scan reported no findings.

The inspector regression was confirmed failing before the repair: at 1750 px its first value
cell started at x=329.5 and was only 8 px wide, while its header started at x=780.5.

**Source-review finding resolved:** indentation is limited to 16% of the field column. Deep
paths remain available in tooltips; nesting beyond four levels shares indentation.

**Not verified:** a fresh live-app screenshot, screen-reader behavior and 200% text scaling.
Desktop automation still cannot initialize because its runtime rejects the Codex configuration
(`features`: map where a boolean was expected). No configuration was changed.

**Approve for the checked layout and interaction cases.** This does not claim a live visual pass.

# Compare redesign (2026-09-22)

The first live screenshots showed a screen of same-weight muted text: no hierarchy, `Left`/`Right`
named eight times with nothing to tell them apart, a wall of dashes for the absent side, disclosure
buttons stretched to their column and centred, raw ISO timestamps, zeroed segment counts before the
first run, and a sync bar whose targets looked like plain text.

Applied `better-ui`, `better-layout` and `emil-design-eng`. What changed:

| Area | Before | After |
| --- | --- | --- |
| Visual system | `+ − ≠ ≈` glyphs in one colour | One 6px dot per side or kind: Left cyan, Right magenta, Different warning, Minor muted, Multiple matches danger. The same dot names the side in the pickers, the segment filter, the list, the column headings and the detail tag |
| Setup header | Loose rows on the content background | The app's `tool_bg` header with panel border; side label rows carry the side dot; Swap is an icon button with a tooltip; notes only when they apply |
| Before first run | Zeroed segments, `0 identical`, two bare hints | One empty state with the compare icon, a title, one sentence and the shortcut |
| Segments | Ghost buttons with glyph labels | The same ButtonGroup with dot, label and a muted count; no sliding indicator (high-frequency control) |
| Status | `0 identical · 0.01s · 2026-09-22T11:20:32.177Z` | `4 identical · 0.01 s · just now`, full timestamp in a tooltip |
| Difference list | Glyph + key + summary | Search icon in Find, rounded 28px rows, kind dot, key, muted trailing detail (`4 fields`, `Left only` in All only). Sync outcomes replace the dot with a check or warning icon |
| Detail header | Key, muted kind text, two ghost buttons | Key, a tinted kind tag, Copy as an icon button, `Open in…` with a real dropdown caret; notes only when they apply |
| Column headings | 56px, three lines on one side, two on the other | One 28px row: side dot, name, namespace, and a `No document` tag on the absent side |
| Detail rows | 30px, dashes on the absent side, whole-column tint, block-stretched disclosure buttons | 26px rows like the document tree. An absent document leaves its column empty; a field the other side has shows `—`. Changed values tint both cells with `bg_changed`; a field missing on one side of a pair tints the present cell with that side's hue. Disclosures sit in a flex wrapper so they hug their label |
| Sync footer | `Change collection:` and text-looking ghost buttons | `Sync to` with two radios; operation checkboxes read `Insert 1 · Right only`; outcome line with spinner or status icon, Undo and Compare again |
| Settings | Redundant select + chips + add row | Same controls, section titles in the app's option style, chips and the add input on one row, help text that says what the default means |

Icons `arrow-left-right` and `git-compare-arrows` were added to `assets/icons` from Lucide.

## Validation

- `cargo test --lib compare:: -- --test-threads=1`: 22 passed (one alignment test caught a collapsed
  disclosure button; fixed by giving the indent wrapper a definite width).
- `just lint`, `just fmt-check`.
- Live screenshots of the empty state, a finished run with all four kinds in the list, a one-sided
  document in the detail pane and the sync target row.

Later verified live from screenshots: the detail pane for `Different` and `Minor` pairs, the
Settings popover, the sync bar with a target chosen, and the outcome line with Undo. Still not
seen live: the progress line during a long scan (the demo data finishes in milliseconds).

## Interaction fixes after the second round of screenshots

- Pickers ignored arrow keys and Enter: the header re-applied the config to every picker on each
  frame, which reset the highlighted row of an open list. It now applies only when the config or
  the item list changes. Covered by `compare_pickers_take_arrow_keys_and_enter`.
- Compare flickered the whole tab: a run wiped all results at once, and every disabled state and
  progress cue appeared for the two frames a small scan takes. Previous results now stay until the
  new run reports, and busy cues wait 150 ms (`CompareTabState::busy`). Covered by
  `a_new_run_keeps_the_previous_results_until_it_reports`.
- Swap made the header jump: the size line is always present at a fixed height, and the sides'
  metadata swaps with them instead of reloading.
- The Compare button no longer flips to "Cancel"; Cancel sits next to the progress text.
- The sync footer is present from the first run on, has two rows instead of three, names
  operations by their effect on the target (`Insert 2 missing`, `Delete 2 extra`), hides empty
  categories, and offers Cancel (also Escape) to leave sync mode.
