# Compare two databases — plan

Planned 2026-09-22 on top of the collection compare (`docs/COMPARE_SYNC_PLAN.md`, PRs #38 and #39).
Cross-checked against `src/` (file references below), gpui-kit 0.6.2 and the MongoDB manual.
Design rules come from the `ui-skills`, `better-ui` and `emil-design-eng` skills, applied through
the app's own components and the compare tab's existing visual language.

**What ships:** the Compare tab gets a second scope. Pick two databases, press Compare, and see
every collection paired by name: which exist on one side only, which differ and by how many
documents, and which are identical. Identical collections are counted and hidden. Opening a
collection hands it to the collection comparison that already exists, with sync and undo.

**Decisions** (confirmed by the owner on 2026-09-23, as recommended):

1. **A scope switch in the Compare tab, not a new kind of tab.** "Collections | Databases" at the
   start of the setup header. Same pickers, same shortcut, same results layout.
2. **One press does both passes.** Pairing and sizes appear within a second. The content scan
   follows at once, one collection at a time, smallest first. Any collection can be skipped and
   the run cancelled. Alternative: stop after the listing and wait for a second press.
3. **Documents are matched by `_id` in every collection.** Ignore fields apply to all of them.
   There is no filter in this scope. A collection that needs another key is opened in its own tab,
   and the overview flags collections where no `_id` matches at all.
4. **Identical collections are hidden by default,** counted in the status line and one click away.
5. **Opening a collection opens its own Compare tab and runs it.** The database tab keeps its
   results. After a sync there, **Recheck** reruns that one collection in the database tab.
6. **Views and time-series collections are listed but not compared,** with the reason. `system.*`
   collections are not listed.
7. **No database-level sync.** A collection that exists on one side only hands off to Transfer,
   prefilled, to copy it across.
8. **Counts only.** The database run stores no documents or difference rows.

## Implementation status — 2026-09-23

**PR 1, scope and pairing: built.** The scope switch, "Compare with…" on a database and "Compare
databases…" in the palette, pass one (`src/connection/ops/compare_database.rs`), the collection
list and detail (`src/views/compare/database.rs`), and Open comparison. Until pass two lands, the
segments are **All · Left only · Right only · In both · Not compared**, and All lists every
collection. PR 2 replaces In both with the verdict segments of §3.3.

---

## 1. What the evidence says

| Evidence | Requirement here |
| --- | --- |
| Studio 3T pairs every same-named collection when one database is dropped on another, and lets the user remove pairs or pair differently named ones ([how-to](https://studio3t.com/knowledge-base/articles/compare-mongodb-collections/)) | Pair by name automatically. Removing pairs is **Skip collections**; pairing different names is under Later |
| Studio 3T: "When the run is completed, a tab opens for each pair of collections" (same page) | One overview, identical collections hidden. A tab opens only for the collection the user picks |
| `dbHash` "obtains a shared (S) lock on the database, which prevents writes until the command completes", and is "not supported in M0 and Flex clusters" ([dbHash](https://www.mongodb.com/docs/manual/reference/command/dbHash/)) | No `dbHash`, not even as a shortcut for identical collections. The overview must be safe on production |
| Without a predicate, counts "return results based on the collection's metadata": wrong for orphaned documents on sharded clusters and after an unclean shutdown ([count](https://www.mongodb.com/docs/manual/reference/command/count/), [estimatedDocumentCount](https://www.mongodb.com/docs/manual/reference/method/db.collection.estimatedDocumentCount/)) | Pass one shows estimates with a `~`. Only the scan's own counts are exact, and only they decide "identical" |
| `$hash` accepts a UTF-8 string or BinData and errors on other types ([$hash](https://www.mongodb.com/docs/manual/reference/operator/aggregation/hash/)). `$function` is deprecated from 8.0 and can be disabled ([$function](https://www.mongodb.com/docs/manual/reference/operator/aggregation/function/)) | No documented server-side way to hash whole documents. Content is compared on the client, as in the collection scope |
| `listCollections` returns each name with its type, `collection`, `view` or `timeseries`. `authorizedCollections` with `nameOnly` lists only what a restricted user may read ([listCollections](https://www.mongodb.com/docs/manual/reference/command/listCollections/)) | Kinds come from the listing. The collection comparison already needs the full listing (`metadata` in `src/connection/ops/compare.rs`), so there is no partial fallback: a refused listing is shown as the run's error |

---

## 2. The user flow

The common case is still "same database, other server". It takes three actions and a press:

1. Right-click `shop` in the sidebar → **Compare with…** (or palette: "Compare databases…"). A
   Compare tab opens in database scope with the left side filled in.
2. Pick the other connection on the right. The database prefills to the same name when it exists
   there, and connects in place if it is closed (PR #39).
3. Read the line under each side: "41 collections · ~3.2M documents · 2.4 GB". That is what the
   press will read.
4. Press **Compare** (`cmd-enter`).

The list fills with every collection at once. Rows then settle one by one as the scan reaches
them, smallest first, so the first verdicts arrive in seconds. Collections that turn out identical
leave the default view. The user can open any row while the scan continues.

To fix a collection: select it, press `enter` (or **Open comparison**). Its own tab opens and
runs. Sync there as today, come back, press **Recheck**.

---

## 3. Screen design

Same regions as the collection scope: setup header, summary strip, then a split with the list on
the left and detail on the right. There is no sync bar in this scope.

How the design rules apply:

| Rule | Where it shows here |
| --- | --- |
| Good defaults beat options (`emil-design-eng`) | `_id`, identical hidden, smallest first, one press. Nothing to configure for the common case |
| Use the project's primitives first (`ui-skills`) | `ButtonGroup`, `uniform_list`, `ErrorCallout`, the token editor and the diff's columns |
| Empty states give one next action (`ui-skills`) | Before the first run: one sentence and the `cmd-enter` hint |
| Errors sit where the action happened (`ui-skills`) | A failed collection says so in its row; Retry is in its detail; Skip is beside the name it skips |
| Structural loading, not spinners (`ui-skills`) | The list appears whole at once with estimates and "Waiting", then settles in place |
| Tabular numbers, truncation in dense UI (`ui-skills`) | Right-aligned counts in a monospaced app; names truncate with a tooltip |
| One accent per view (`ui-skills`) | Compare is the only primary button; Open comparison is outlined |
| Frequent actions do not animate (`emil-design-eng`) | Row updates, selection, segments and keys are instant; busy states wait 150 ms |
| Every state change has a static cue (`better-ui`) | Each kind has a glyph, a word and a colour; motion is never the only signal |

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│ [Collections│Databases]                                                                 │
│ ● LEFT                                         ● RIGHT                                  │
│ [● Staging ▾] [shop ▾]                         [● Production ▾] [shop ▾]                │
│ 41 collections · ~3.2M documents · 2.4 GB      39 collections · ~3.3M documents · 2.6 GB│
│                                                                                         │
│ [⚙ Settings]  Match by _id · ignoring updatedAt · skipping sessions   [⇄] [Compare] ⌘↵  │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ [All 9] [● Left only 2] [● Right only 0] [● Different 6] [● Minor 1] [Identical 29]     │
│ [Not compared 3]                     29 identical · 3 not compared · 1m 12s · just now  │
│ ┌──────────────────────────────────────────────┐│┌────────────────────────────────────────┐
│ │ [🔍 Find collection…                       ] │││ orders            Different  [Open ↵] │
│ │ Collection      ● Left     ● Right  Result   │││            ● Left · Staging  ● Right · │
│ │ ● audit_log     120,442          —  Left only│││ Documents  1,204,113         1,204,160 │
│ │ ● orders      1,204,113  1,204,160  45 diff… │││ Size       1.4 GB            1.4 GB    │
│ │ ● products        8,120      8,120  3 differ.│││ Indexes    6                 5         │
│ │ ● users          40,221     40,229  8 right …│││                                        │
│ │ ○ events       ~9.1M      ~9.1M     Waiting  │││ ≠ 45 different  − 47 right only        │
│ │                                              │││ ≈ 212 minor · 1,203,856 identical      │
│ └──────────────────────────────────────────────┘│└────────────────────────────────────────┘
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Setup header

- **Scope switch.** A two-button `ButtonGroup`, the control the segments already use. It keeps
  both connections and databases when switching. In database scope the collection pickers and the
  filter disappear; nothing else moves.
- **The line under each side** reads `dbStats` once per side: collections, estimated documents,
  data size. Estimates carry a `~`. It fills the slot the collection size uses today, so the header
  keeps its height. Connection progress and errors from PR #39 keep priority there.
- **Settings** keeps Ignore fields, which apply to every collection, and adds **Skip collections**,
  a token list built with the same token editor. The Match by editor shows `_id` and explains why
  it is fixed: "Every collection is matched by _id. Open one to match it by another field."
- **The summary line** names what the run will do: "Match by _id · ignoring updatedAt · skipping
  sessions".
- **Disabled reasons** reuse `compare_disabled_reason` with a scope branch: "Choose a connection
  and database on the left", "Choose two different databases".

### 3.2 Progress

While scanning, the status text is replaced in place, as it is today, and the 2 px line runs along
the strip's bottom edge. Its fraction is documents read over the estimated documents of the
collections still to scan.

`Comparing orders · 12 of 38 collections · 1.2M read · 38,000/s  [Skip orders] [Cancel]`

- **Skip** is next to the name it skips, so its effect is obvious. It marks that collection
  Skipped and moves on.
- **No ETA**, for the same reason as the collection scope: it would jump around.
- The per-side "sorting on the server" note carries over for collections without an `_id` index,
  such as views, which are not compared anyway.

### 3.3 Summary strip

Segments count **collections**, not documents:

**All · Left only · Right only · Different · Minor · Identical · Not compared**

- **All** is the three real buckets, as in the collection scope. It is the default, which is what
  hides identical collections.
- **Identical** is a segment here, unlike the collection scope, because collections are few enough
  to list and seeing which ones matched is useful.
- **Not compared** holds views, time-series, skipped, failed and cancelled collections, so nothing
  disappears silently. Failed ones also show in red in their row.
- Every segment is always present, even at zero, so the strip never shifts while counts arrive.
- The status line after a run: "29 identical · 3 not compared · 1m 12s · just now".

### 3.4 Collection list (left of the split)

- `uniform_list`, 28 px rows, the same row shell as the difference list.
- **A heading row** with the side dots names the columns: Collection · ● Left · ● Right · Result.
  The legend is always on screen, as in the document diff.
- **Row:** marker · name · left count · right count · result.
  - Marker: the kind dot (the colour of Left only, Right only, Different, Minor, as today). A
    hollow dot for waiting and not compared. Glyphs carry the meaning without colour for
    Left only (`+`), Right only (`−`), Different (`≠`) and Minor (`≈`).
  - Counts are right-aligned so the digits line up. Before a collection is scanned they are the
    estimates with `~`. After, they are the exact documents read, and the `~` goes away.
  - `—` for a side where the collection does not exist.
  - Result is short and muted: "45 different", "45 different · 47 right only", "Left only",
    "Identical", "212 minor", "Waiting", "38%", "View", "Time-series", "Skipped", "Cancelled",
    and "Failed" in the danger colour.
- **Order is alphabetical and never changes.** The scan order, smallest first, is invisible;
  rows update in place, so the selection and scroll hold still.
- **Find** jumps to the first collection whose name contains the text, like Find key does for
  keys. The placeholder reads "Find collection…".

### 3.5 Collection detail (right of the split)

Built from the document diff's columns (`comparison_row`, `field_column`) so the two scopes look
alike and the side columns line up with the heading.

- **Header:** name, result tag, **Open comparison** with its `enter` hint. **Recheck** appears
  once the collection has a result. For a collection on one side only, **Copy to Right…** (or
  Left) opens Transfer prefilled instead of Open comparison.
- **Facts**, one row each with a value per side: Documents, Size, Indexes.
- **Breakdown** after a scan, with glyphs: different, left only, right only, minor, identical,
  and the time it took.
- **Notes** only when they apply:
  - No `_id` matched although both sides have documents: "No document matched by _id. They were
    probably inserted separately. Open the comparison and match by another field."
  - Estimates only: "Counts are estimates until this collection is compared."
  - The failure message with Retry, as an `ErrorCallout`, next to the action that failed.
- **Nothing selected:** "Select a collection" and "Its counts and actions appear here."

### 3.6 States

| State | What shows |
| --- | --- |
| Before the first run | The empty hint, retitled: "Compare two databases". "Collections are paired by name. Identical ones are hidden." Press `cmd-enter` to start. One next action |
| Listing | Under a second; rows appear together with estimates and "Waiting" |
| Scanning | Progress text and line; rows settle one by one; everything is selectable |
| Nothing differs | "No differences. 38 collections are identical." with a link to the Identical segment |
| A collection fails | Its row says Failed; the scan continues; the detail shows the error and Retry |
| Cancelled | Finished rows keep their results; the rest say Cancelled; Compare starts again |
| Connection closed | Results stay readable; Compare, Open comparison and Recheck say why they are disabled |
| Setup changed | The existing note: "These results compared Staging · shop with Production · shop" |

### 3.7 Keyboard and accessibility

- `up` and `down` move through rows, `cmd-f` focuses Find, `cmd-enter` compares and `escape`
  cancels a run, with the bindings the collection scope already has (`src/keyboard.rs:220-228`).
  `enter`, which focuses the document diff in the collection scope, opens the selected collection
  here: the view handles the same action by scope.
- Each row's accessible label reads the whole line: "orders, 45 different, 47 right only".
- Colour is never the only signal: every kind has a glyph and a word.

### 3.8 Motion

Rows settle many times a second during a scan, and selection and segment switching are frequent.
By the frequency rule none of them animate. The progress line and the kit's dialog and popover
motion stay as they are. Busy states wait 150 ms before showing, as in the collection scope, so a
fast run does not flicker. Nothing new is animated, and every state has a static cue.

---

## 4. Engine

### 4.1 Pass one: pairing and sizes

Per side, in parallel, on the connection runtime:

- `listCollections` gives names and kinds. `system.*` is dropped (`is_system_collection`).
  Kinds come from `CollectionDetail::from_spec` (`src/models/connection.rs:336`).
- `dbStats` gives the header line (`src/connection/ops/stats.rs:47`).
- For every name present as a collection: `estimatedDocumentCount` and the storage size from
  `collStats`, at most 8 at a time, each bounded by the interactive query timeout. Both are
  optional: restricted accounts still get the pairing.

Pairs are the union of names. Display order is alphabetical. Scan order is by the larger estimate,
smallest first, unknown sizes last.

### 4.2 Pass two: contents

For each pair of real collections, in scan order, one at a time:

```
compare_collections_async(left, right,
    CompareOptions { fields: ["_id"], ignore, row_limit: 0, filter: {} },
    pair_token, sender)
```

This is the collection comparison unchanged. `row_limit: 0` keeps counts exact and stores no rows
(`src/connection/ops/compare.rs:119`). Progress messages are tagged with the pair's index. One
comparator produces every number, so the overview can never disagree with the tab that opens.

The result is the pair's `CompareSummary`. Its verdict: Different when any document is different
or on one side only, Minor when only minor differences exist, otherwise Identical.

### 4.3 Cancel and skip

`CancellationToken` is a flag (`src/connection/types.rs:143`). The run holds one token and each
pair gets a fresh one. Cancel sets both; Skip sets only the pair's. The loop checks the run token
between pairs.

### 4.4 What is stored

Per pair: names, kinds, estimates, sizes, index counts, the summary, and an error. A few hundred
bytes each, so thousands of collections cost nothing. Opening a collection reads it again in its
own tab, which is also what makes that view current.

### 4.5 Considered and rejected

| Idea | Why not |
| --- | --- |
| `dbHash` for identical collections | Blocks writes on the whole database while it runs and is refused on Atlas M0 and Flex. It would also ignore the ignore fields, so it could disagree with the scan |
| Metadata counts as the verdict | Estimates drift after unclean shutdowns and include orphans on sharded clusters. Equal counts also say nothing about contents |
| Hashing documents on the server | `$hash` takes only strings and BinData; `$function` is deprecated and often disabled. Revisit if MongoDB adds a document hash |
| Comparing several collections at once | Doubles the load on the server for a gain only on many tiny collections. One at a time is predictable; see Later |
| Keeping difference rows for every collection | Memory would grow with the database. Opening a collection rereads it, which also makes that view current |
| A match key for the whole database | Field names differ between collections. The key belongs to one collection, in its own tab |
| One tab per collection | The Studio 3T behaviour this feature exists to avoid |

---

## 5. Fitting into the app

- **Config** (`src/state/compare.rs`): `scope: CompareScope` and `skip: Vec<String>`, both
  `#[serde(default)]`, so saved tabs load unchanged. `CompareEndpoint::complete` takes the scope.
- **Tab state:** `pairs: Vec<PairState>` and per-segment index vectors, next to the collection
  scope's rows. A new run clears them the same way `begin` clears rows today.
- **Ops:** `src/connection/ops/compare_database.rs` for pass one and the pair loop.
- **Commands:** `src/state/commands/compare_database.rs`: run, skip, recheck, open collection.
- **View:** `src/views/compare/database.rs` for the list and the detail. `mod.rs` picks the
  scope's list and detail; setup and summary take a scope branch.
- **Entry points:** "Compare with…" in the database menu (`src/app/menus.rs:128`), and "Compare
  databases…" in the palette beside "Compare collections…"
  (`src/components/action_bar/providers.rs:178`).
- **Open comparison** calls `open_compare_tab` with both endpoints, copies the ignore fields and
  runs it. **Copy to…** calls `open_transfer_tab_with_prefill` as the collection menu does.

---

## 6. Build order

Three PRs on the existing stack, each shippable:

1. **Scope and pairing.** Scope switch, entry points, pass one, the list with estimates and
   Left only / Right only, the detail with facts, Open comparison. Useful on its own: which
   collections exist where, and how big.
2. **Contents.** Pass two, progress, Skip, Cancel, results and segments, Identical hidden,
   Recheck, Skip collections.
3. **Details.** Index differences in the detail, Copy to… for one-sided collections, and the
   "no `_id` matched" note.

---

## 7. Tests

- **Unit:** pairing of names and kinds, `system.*` dropped, scan order, verdict mapping, the
  reducer for pair messages, config round-trip with and without the new fields.
- **View:** segment counts, alphabetical order holding still while results arrive, `enter`
  opening a collection tab, detail columns aligned with the heading, the empty and failed states.
- **Docker:** two databases with an identical, a different, a one-sided, a view and a
  time-series collection, a collection the user cannot read, and a document-less pair. Assert
  every verdict, then cancel mid-run, skip, and recheck.

---

## 8. Risks and things not verified

- **Load.** Pass two reads every document of every compared collection on both sides, like
  running each collection comparison in turn. Mitigations: the size is shown before the press,
  one collection at a time, smallest first, Skip, Cancel, and Skip collections for the regulars.
- **Time on slow links.** Local reads reached about 600,000 documents a second
  (`docs/COMPARE_BENCHMARKS.md`); a remote link can be a hundred times slower. The rate is shown;
  there is no ETA.
- **Restricted accounts.** `dbStats` and `collStats` may be refused; both are optional. A refused
  listing fails the run with the server's message. Not tested against a real restricted user yet.
- **Sharded clusters.** Not tested, as in the collection scope. Estimates there include orphans.
- **Thousands of collections.** The list is virtual and pass one caps concurrency at 8, but a
  database with 10,000 collections has not been tried.
- **The research agent stalled,** so the evidence above comes from the MongoDB manual and one
  Studio 3T page only. Navicat and dbForge were not checked.

---

## 9. Later

- Pair collections with different names, as Studio 3T allows.
- Run small collections concurrently if one at a time proves slow on many tiny collections.
- Validators in the detail, beside indexes.
- Saved database comparisons and scheduling, once tasks exist.
