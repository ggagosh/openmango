# Relations — implementation plan

Planned 2026-09-20 from `OpenMango-Relations-Feature.pdf`, cross-checked against `src/`,
gpui-kit 0.6.2 (`refs/gpui-kit`), the Compass feedback thread, and competing clients.

**Decisions so far:** one build, not three releases. ObjectId references only. The relation graph
is written from scratch here (no external reference implementation).

## 1. Verdict on the feature doc

The direction is right. Three things change:

1. **The tab model cannot do what §3 describes.** `TabKey::Collection(SessionKey)` with
   `SessionKey = (connection, database, collection)` means one session per collection, so a jump to
   `users` would overwrite an open `users` tab. Fixed by the browser model in §4, which lands first.
2. **The §7 data structure is premature.** Arenas, `u32` ids, interning, hot/cold split and
   `arc-swap` are benchmarked at 464k paths; the doc itself says a typical database loads in a
   millisecond. Keep the **on-disk format**, use plain structs in memory (§5).
3. **Plain-hover peek is wrong for a dense grid and for production.** It runs queries without
   intent (§6).

The rest holds. The statistics are correct (rule of three: containment ≥ 1 − 3/k at 95%) and the
`$sample` random-cursor conditions match MongoDB's documented behaviour.

## 2. What users actually asked for

Compass idea CUSTOMER-I-6695, "Allow following a referenced ObjectID to the associated document":
created 2022-07-28, still *Submitted*, **7 votes, 3 comments**. Long-standing, not loud. The
comments are the useful part:

| User said | Requirement |
| --- | --- |
| "as easy as clicking a link on a web page" | Link affordance; one gesture; **browser mental model** |
| "finding the collection the ObjectID is in (which luckily I know, but if I didn't it would be even harder)" | Resolve the target for the user — value-based probe, zero setup |
| "right-click should open a context menu with a list of the DB's collections … opens another tab … with the `{_id: ObjectId("")}` query filled in and submitted" | Manual target pick; new-tab variant; real, visible filter |
| "double clicking the ObjectId switches to editing … most of the time I'm trying to select the text" | Do not steal click / double-click; copying an id stays trivial |

Competitors: **Mongon** — every ObjectId is a link, opens a new tab, "detects which collection the
ID belongs to" (click-time probing, no model). **NoSQLBooster** — "Follow Reference" in the context
menu, Shift+F7. **Compass** — no click-through, but now ships ER diagrams with inferred
relationships and export. Nobody ships visible evidence, referenced-by, or a shared model.

## 3. Cross-check: what already exists

| Doc needs | In the codebase | Gap |
| --- | --- | --- |
| Observed profile | `build_schema_analysis` (`state/commands/schema.rs`), `SchemaAnalysis` types | Reuse, do not write a second profiler. Paths are `items.[*].x` not `items[].x`; arrays-in-arrays not recursed; fixed 1000-doc sample; raw values not retained |
| `$sample` + `maxTimeMS` | `sample_for_schema_async` | None |
| Indexes, collStats | `list_indexes_async`, `collection_stats_async` | None |
| Fetch by `_id` | `find_document_by_id` | New op: bounded multi-collection `_id` probe |
| Real, editable filter on the target | `AppState::set_filter(key, raw, doc)`; fast filter parses bare ObjectIds | None once §4 lands |
| Context-menu entry point | `tree/tree_menus.rs`: "Filter by this value" / "Exclude this value" | Add items beside them |
| Model file | `ConfigManager` JSON files | Add `relations.json` |
| Environment awareness | `SavedConnection.environment`, `.protected`, `.read_only` | Gate unindexed / background reads |
| Agent tools | `src/ai/tools/*.rs` (one file per tool), `src/mcp/` | Two more files |
| DBRef | Nothing | Detect `{ $ref, $id, $db? }` sub-documents |
| Back / forward | Nothing; `cmd-[`, `cmd-]`, `cmd-b`, `f12` unbound | §4 |
| Document detail panel | Does not exist | References tab instead (§6) |
| Peek and trail widgets | gpui-kit `Popover`, `HoverCard`, `Breadcrumb`, `DescriptionList`, `Skeleton`, `Tag`, `Kbd` | None |
| Diagram view | gpui-kit has `chart` / `plot` only | Custom canvas + layout crate |
| Cmd-hover link state | gpui `on_modifiers_changed` | None |

**Cmd+click conflict.** Tree: Cmd+click toggles multi-select only on document root rows, so value
rows are free. Table: `render_tr` toggles selection anywhere in the row — the reference *text* in
`render_cell` must handle Cmd+mouse-down and stop propagation (`render_cell` needs a callback
parameter; it has no listener context today).

## 4. Tabs become browser tabs (lands first, own PR)

The Compass commenter's phrase is the design: "like clicking a link on a web page". Everyone
already knows that model — follow a link in the same tab, Back returns exactly where you were,
modifier-click opens a new tab, and no *other* tab is ever disturbed. Anything that rewrites a tab
the user is not looking at breaks the expectation that tabs are their working contexts.

- `SessionKey` gains `instance: u32` from a counter on `AppState`. A session is one view of a
  collection; several can exist per collection.
- Each collection tab owns `history: Vec<SessionKey>` plus a cursor.
  **Cmd+click navigates in place**: a new session for the target becomes the tab's current key and
  the previous session **stays alive** in history. Back therefore restores page, scroll,
  selection, expansion and filter for free — nothing is rebuilt or re-queried.
- Cap history at 20 per tab; sessions that fall off go through the existing `cleanup_session`.
  New jump truncates forward entries. Closing the tab cleans up its whole history.
- **Cmd+Shift+click** opens the target in a new tab.
- Current tab has unsaved edits → the jump opens a new tab instead. A dirty session is never
  buried in history where closing the tab would silently drop it.
- Sidebar click keeps today's behaviour (focus the tab whose current collection matches); a new
  "Open in new tab" gives a second view — the same context-menu item Compass has had since
  1.31, so it is familiar rather than invented. It has value on its own: comparing two filters
  of one collection is impossible today.

Blast radius, measured: `SessionKey` appears 473 times in 83 files, but only **42 construction
sites in 17 files** need a decision; the rest pass keys through unchanged. `WorkspaceTab` already
persists per tab, so the saved format already allows duplicates. Two caches are per *collection*,
not per view, and must be re-keyed by namespace: `collection_meta` and `forge_schema`
(`app_state/mod.rs`). `matches_collection`, `close_tabs_for_collection` and
`rename_collection_keys` must match every instance of a namespace.

## 5. Model and resolution

### Scope: ObjectId only

- Links: ObjectId values at any path except the root `_id`, and DBRef sub-documents whose `$id` is
  an ObjectId.
- Inference candidates: paths whose sampled values are ≥ 90% ObjectId. Targets: `_id` only.
- Dropped for now: unique-index targets, integer / string / UUID keys, ids-as-object-keys,
  compound keys. The file format still writes `target.path`, so none of this is a migration later.
- Known first follow-up: 24-hex **strings** that point at ObjectId `_id`s. Common in real schemas;
  out of this build.

ObjectId-only removes most ambiguity: ObjectIds are near-globally unique, so a probe hit is
near-proof. Ambiguity survives only in the rare shared-`_id` case (`users` / `user_profiles`),
and then there are at most a handful of *single* documents to choose between — a popover, not
a tab. The References view answers a different question: what points **at** this document, which
is many documents from many collections.

### In memory: plain structs

```rust
pub struct Relation {
    pub source: FieldRef,           // db, collection, path ("items[].productId")
    pub target: FieldRef,           // path is "_id" for now
    pub kind: RelationKind,         // Reference | Polymorphic | Embedded
    pub cardinality: Cardinality,
    pub origin: Origin,             // Inferred < Probe < CodeImport < DbRef < User
    pub status: Status,             // Candidate | Accepted | Rejected
    pub confidence: f32,
    pub evidence: Option<Evidence>, // sample size, hits, sampled_at
}
// ponytail: Vec<Relation> + HashMap indexes (outgoing by path, incoming by collection, outgoing by
// collection). Arena ids / interning / arc-swap only if a 2k-collection deployment shows up in a
// profile. join_path is a BFS over <100 nodes; no petgraph.
```

API as in the doc: `outgoing(path, min_conf)`, `referenced_by(coll, min_conf)`, `join_path(a, b)`,
`mongo_path()` (dot path + unwind count), `to_model()`. `upsert` is idempotent on
(source, target, kind) and never lets a lower origin overwrite a higher one. No `arc-swap`:
inference hands its result to the entity on the main thread, as schema analysis does now.

`Probe` is an origin the doc lacks: a mapping learned from a successful click. It is how the graph
fills itself in through ordinary use.

### On disk

`relations.json` via `ConfigManager`: versioned, string keys (`"shop.orders:items[].productId"`),
sorted for clean diffs. Export / import of the same format for sharing in git.

**Keyed by database name, not by connection.** A relation such as "`orders.userId` → `users`" is a
fact about the application's schema, and dev, staging and prod of one app share that schema. Learn
it on local dev, and it is already there when connecting to production — without running inference
on production. If two unrelated apps happen to share a database name, the wrong entry heals
itself: every jump is confirmed by a probe, a miss falls through to the search, and the new hit
replaces the stale mapping.

### Click-time resolve

1. DBRef → target is explicit.
2. Relation exists → `find_one({_id: v})` on the target: existence check and peek payload in one
   query. Miss → broken-reference state.
3. None → rank same-database collections by name (`userId`, `user_id`, `user`, `ownerIds[]` →
   strip the id suffix, singular / plural match), probe with covered `find({_id: v}, {_id: 1})`,
   concurrency 8, `maxTimeMS` 2000. Above ~200 collections probe the top 50, offer "Search all".
   One hit → navigate and remember. Several → the peek popover in choose mode, one row per
   collection holding the id, "Remember for `orders.userId`" checked. None → "No document with
   this `_id` in any collection of `shop`" + pick a collection.

Value-first probing resolves polymorphic references (`refPath`) for free. Click-time probes use
the connection's own read preference; `secondaryPreferred` is a per-operation setting for
background inference only.

## 6. UI / UX

Lens: PRODUCT.md, better-ui, and the emil-design-eng frequency rule — this happens tens to hundreds
of times a day, so almost nothing here animates.

### Gestures

| Input | Result |
| --- | --- |
| Click / double-click | Unchanged: select / edit |
| Cmd+click on a reference | Navigate in this tab |
| Cmd+click on a document's `_id` | Find what references it — the same gesture, asked inward |
| Cmd+Shift+click | Navigate in a new tab |
| Cmd held | References get underline + pointer cursor |
| Row hover | Small arrow after the value; click = peek |
| `cmd-b` (also `f12`) | Navigate from the selected row |
| `space` on a selected reference row | Peek |
| `cmd-[` / `cmd-]` | Back / forward in this tab |
| Context menu | "Go to referenced document", "Find in collection ▸", "Open all" (arrays), "Find references to this document" (root rows) |

### Peek

- **Triggers: arrow click, `space`, or Cmd+hover. Never plain hover** — accidental cards, and
  unintended reads on production. Cmd+hover is VS Code's definition preview. `HoverCard` (with its
  open delay) for Cmd+hover only; `Popover` otherwise.
- Header: target namespace + connection / environment chip. Body: first ~8 top-level scalar fields
  as a `DescriptionList` in the existing BSON colors. Footer: `Open` (Enter), `Open in new tab`
  (Cmd+Enter), with `Kbd` hints.
- `Skeleton` rows only after ~150 ms, so a 20 ms local probe never flashes a loader.
- Esc closes and restores focus to the originating row.

### Motion

| Element | Decision | Why |
| --- | --- | --- |
| Hover arrow, Cmd-underline | Instant | Constant in a grid; state cue, not decoration |
| Peek by pointer | gpui-kit `MotionTokens` popover enter, origin at the trigger, scale never below ~0.97 | Match the kit; popovers grow from their trigger |
| Peek by keyboard | None | Never animate keyboard-initiated actions |
| Peek exit | Faster than enter, or instant | The system responding should be snappy |
| Navigate / back / forward | Instant swap, no slide | Ornamental motion is a PRODUCT.md anti-reference |
| "N relations found" badge | Opacity-only fade, once | Rare; movement would compete with data |
| Reduced motion | Opacity only | PRODUCT.md |

### Details that compound

- A jump that returns exactly one document **expands it** in tree view.
- Link affordance is underline + arrow, never color alone. The arrow points right on a
  reference and left on an `_id`, because the jump each one offers goes the opposite way. Broken reference is icon + sentence,
  neutral tone, two actions ("Search other collections", "Edit relation"). "Unindexed" on a
  References group is an icon **and** a label.
- Peek radius is concentric (inner = popover radius − padding) from existing tokens; elevation from
  the kit's popover shadow, no extra border.
- Breadcrumb renders the tab's history only when depth > 1, truncates in the middle
  (`users › … › products`), every crumb is a button, and it takes no space when hidden.

### References view

Zed's answer to "what points at this?" is not a list of counts but a multibuffer: one tab holding
the actual matches from every file. The same shape fits here and replaces the earlier popover
idea: seeing the twelve orders beats seeing "`orders.userId` · 12" and clicking through.

- **Its own tab kind**, `TabKey::References`, with state in a map keyed by id — the pattern
  Transfer and Forge tabs already use. Title: "References to `users` / 64f…". It opens as a new
  tab and never touches the tab it came from. No in-tab history: it is a result, and re-runnable.
- **Opened by** "Find references to this document" on a document root row, or `shift-f12`
  (VS Code's Go to References, the natural pair to `f12`).
- **Content**: one group per incoming edge from `referenced_by(collection)`. Group header:
  `orders.userId` · count · "indexed" / "unindexed — may be slow", plus **Open as filter**, which
  opens `orders` in a collection tab with the real filter `{ userId: ObjectId(…) }`. Under it,
  the first 20 matching documents as read-only rows; Enter on a row opens that document in its
  collection, filtered by `_id`.
- **Rendering is reuse, not new code**: `AggregationTableDelegate::refresh_data(Vec<Document>)`
  and the aggregation results view already draw an arbitrary document list as tree or table
  with read-only rows (`render_lazy_readonly_row`).
- **Loading**: groups load lazily and independently with `limit` + `maxTimeMS`, so one slow
  collection never blocks the rest. An unindexed group on a production / protected connection
  does not run by itself: its body is a "Run anyway" button.
- Tabular numerals for the counts, so lazy results do not jitter the headers.

The peek popover stays for the common single-target jump: a glance should not cost a tab.

### Relations page

Table of edges: source → target, kind, cardinality, origin, status, confidence, sample size and
age. Accept / reject / edit inline. **"Infer relations"** is an explicit button; automatic
inference on collection open is an opt-in setting, because unrequested background reads on
production contradict the product's trust stance. Drift on accepted edges shows here and in the
integrity report only — never as a toast.

## 7. Build order (one build)

Ordered by dependency; each step leaves the app working.

1. ~~**Tab refactor** (§4): session instances, per-tab history, back / forward, "Open in new
   tab".~~ **Done** on `feat/relations-tab-history`. `SessionKey` gained `instance`;
   `CollectionKey` now keys the per-collection caches; `state/app_state/tabs/navigation.rs`
   holds `navigate_to_collection` / `navigate_back` / `navigate_forward` /
   `open_collection_in_new_tab` / `navigation_trail`; `cmd-[` / `cmd-]` and
   `cmd-shift-enter` are bound. Navigation history is per-run and not persisted.
2. ~~**Relation graph**~~ **Done** — `src/state/relations/mod.rs`: structs, indexes, `outgoing` / `referenced_by` /
   `join_path` / `mongo_path` / `to_model`, path helper, name heuristic, `relations.json`.
   Unit tests: nested paths and unwind counts, join finding including misses, origin precedence,
   decision persistence, JSON round-trip and sort stability.
3. **Navigation** — mostly done. `connection/ops/relations.rs` probe op; `resolve()`; links in `tree_row.rs` and
   `cell_renderer.rs`; context menu; key bindings; peek; the References tab shell
   (`TabKey::References`, reusing the aggregation results rendering), first used for ambiguous
   targets; broken-reference state; breadcrumb; single-result expand.
   `tests/relations_tests.rs` (Testcontainers): resolve → probe → remembered relation,
   shared-`_id` ambiguity, miss, DBRef. **Left:** links in aggregation results.
4. **Inference** — done. Run it for a whole database from the **Relations** section of the
   database tab (known count, progress, Stop), or for one collection from its context menu.
   `state/relations/infer.rs` profiles a byte-budgeted sample for ObjectId-shaped paths
   (arrays at any depth, ≤ 200 distinct ids each), reads DBRefs outright, pairs each field with
   the 8 best-named collections and confirms with covered `$in` probes escalating 10 → 50 → 200.
   Confidence is the share of probed ids that were found — it answers *which collection*, and
   the rule-of-three bound on containment stays in the evidence for the integrity report. A field
   keeps only its strongest target, ties broken by name.
   **Deferred, with the ceiling named in code:** the `_id` time-range prune and the `collStats`
   metadata stage it needs (name ranking already cuts the probe count); `secondaryPreferred` on
   inference reads; a job runner with pause / resume for whole-deployment sweeps; automatic
   inference on collection open.
5. **Surfaces** — References tab done: `TabKey::References`, "Find references" on a document
   (`shift-f12`), one group per incoming relation loading independently, unindexed groups held
   behind "Run anyway" on production and protected connections, "Open as filter" per group.
   **Left:** the Relations page, the "N relations found" badge, "Open all" for arrays, and the
   integrity report (orphans, drift, unindexed reference fields).
6. **Consumers on the graph** — agent / MCP tools `get_relations`, `join_path` (accepted edges as
   compact text); Mermaid / DBML export; `$lookup` generator and autocomplete.
7. **Independent consumers** — code imports (Mongoose `ref`, Prisma, `$jsonSchema`), codegen
   (TypeScript / Zod / Rust), native ER diagram view (custom canvas + a layout crate: `dagre` Rust
   port, `layout-rs` or `rust-sugiyama`, chosen when we get there). Last because nothing depends
   on them and they share no code with steps 1–6.

New crates: none until step 7's layout crate.

## Sources

- Compass idea CUSTOMER-I-6695: <https://feedback.mongodb.com/ideas/CUSTOMER-I-6695>
- Compass data modeling: <https://www.mongodb.com/docs/compass/data-modeling/relationships/>
- Mongon: <https://mongon.app>
- NoSQLBooster blog: <https://www.nosqlbooster.com/blog/>
- Layout crates: <https://crates.io/crates/dagre>, <https://crates.io/crates/layout-rs>,
  <https://crates.io/crates/rust-sugiyama>
- gpui-kit guides: <https://gpui-kit.com/docs/coding-guides/>,
  <https://gpui-kit.com/docs/design-guides/>
