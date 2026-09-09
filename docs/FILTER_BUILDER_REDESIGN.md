# Filter Builder redesign proposal

Status: proposed direction, based on the supplied September 9 screenshot, current source at `50bd518`, synthetic model probes, and official MongoDB/GPUI documentation. No desktop interaction audit was performed.

## Implemented consistency pass

The existing side-panel UI now uses native Small buttons/inputs/operator menus, pressed All/Any button groups, wrapping toolbars, borderless disclosure groups, and consistent separate value rows. Group headers show a condition count and keep their All/Any control visible when collapsed; indentation and spacing show nesting without guide lines or enclosing boxes. Collapsing preserves the query and input entities, and moves focus out of a field that becomes hidden. The title and footer are compact, guidance/error text wraps, and the empty state has an Add condition action. Type labels follow UI scaling rather than a fixed tiny font.

Adding a condition focuses its field. The builder has its own focus scope, Cmd/Ctrl+Enter is routed to the builder, Escape no longer clears input text, and closing restores query focus. Find/Edit MQL validate the draft before serialization; Find first commits pending list values. Programmatic field synchronization preserves the existing type, value edits update validation immediately, and source replacements preserve undo history.

Headless tests render the actual panel with an isolated temporary configuration and cover native control heights, nested layouts at narrow widths/UI scaling, add-condition focus, invalid execution, and independent mouse/keyboard group selection. The wider layout and advanced BSON/operator coverage below remain proposed work; this pass does not implement that model redesign.

## Outcome

Build and revise complex collection filters confidently, with the query and results still visible. A developer must be able to understand group scope, choose exact BSON values, move between visual conditions and MQL, and run the same query they see.

The coverage contract is **no lost conditions and no silent value conversions**. Common predicates get guided controls; specialist or unfamiliar predicates remain editable as MQL clauses in their original scope. An unsupported clause must not disable the rest of the builder or require starting over.

This covers collection filter documents. Sort and projection stay in the existing query options; aggregation pipelines stay in their existing workspace.

## What is wrong today

### Visible in the supplied screenshot

- The fixed side panel gives the empty draft a large blank canvas while its actual controls compete for one narrow toolbar row. The onboarding sentence is truncated.
- `$and`, `$or`, Rule, Group, and a drop target expose the query structure before providing an obvious first editable condition.
- Clear and Reset do not explain whether they affect the draft or the applied query. Run is separated from the work by most of the panel height.
- The fixed 480 px overlay also covers collection content rather than allocating space alongside it. This placement is confirmed in [the collection view](../src/views/documents/view.rs#L1032).

### Confirmed in synthetic model round-trip probes

These probes used `FilterTree::from_document`, `validation_error`, and `to_document` with synthetic BSON. They did not query a database or launch the app.

| Input | Current model result | Required behavior |
| --- | --- | --- |
| `{ name: "" }` | Accepted by import; then reports missing value and emits `{}` | An empty string is a valid explicit value |
| `{ name: "  mango  " }` | Emits `{ name: "mango" }` without an error | Preserve significant whitespace |
| `{ kind: { $in: ["7", 7] } }` | Emits two string values | Preserve the BSON type of every list item |
| Decimal128 `1.2300` | Reports invalid number and emits `{}` | Preserve exact decimal value/type |
| BSON regex `^mango` with option `i` | Emits the literal string `"^mango"` | Preserve pattern, regex type, and options |
| `{ age: { $gte: 18, $lt: 65 } }` | Rejects the import | Support multiple predicates on one field |
| `$elemMatch`, `$nor` | Rejects the import | Represent array-element scopes and logical negation |
| Literal embedded-document equality | Rejects the import | Preserve literal object equality, including member order |

The conversion problems follow from [string-based value storage and parsing](../src/components/filter_builder/types.rs#L499), [first-item list type inference](../src/components/filter_builder/types.rs#L1261), and [unsupported multi-operator handling](../src/components/filter_builder/types.rs#L1088). Existing tests explicitly assert that ranges and regex options are unsupported at [types.rs](../src/components/filter_builder/types.rs#L1500).

There is also a validation gap in the execution path: the button uses `can_run`, but [the shared apply method](../src/components/filter_builder/panel.rs#L683) checks only unsupported state. The keyboard handler calls it directly. Invalid conditions return `None`, and [tree serialization](../src/components/filter_builder/types.rs#L863) filters those conditions out. Validation must be enforced by the execution method so every entry path either runs the complete filter or reports its errors.

### Other source-backed workflow gaps

- Type inference uses field names or sampled data, but the visible type badge has no editing action. Mixed schemas need an explicit value-type control. See [field changes](../src/components/filter_builder/panel.rs#L241) and [type badge](../src/components/filter_builder/panel.rs#L2903).
- Opening the builder parses the saved raw text as a document directly; scalar-ID and shorthand inputs accepted by the main query bar are not compiled through the same entry path. See [builder initialization](../src/views/documents/view.rs#L1037).
- Builder-to-MQL and Run use `set_value`, which clears the editor's undo history. The main query editor already has atomic, undoable edits that can be reused.
- The [document-level key interceptor](../src/views/documents/state.rs#L130) exempts filter/sort/projection inputs, but not builder controls, before entering document-edit/save handling. Give the builder explicit focus/key ownership and test Enter with a selected or dirty document; this interaction risk has not been exercised in the desktop app.
- List entry splits on every comma/newline, without respecting quoted strings. A value such as `"New York, NY"` must stay one value. See [token splitting](../src/components/filter_builder/panel.rs#L2632).
- Suggestions run a separate 500-document sample and retain the first observed type. Reuse the existing collection metadata/loaded-document sources and expose mixed/unknown types rather than coercing values from one sample.

## Proposed interaction

### Panel and first action

The proposed default is a **resizable panel beneath the query bar, above results**. It starts compact and grows with the user's work, with a bounded scroll region for larger filters. The user's preference between this and a resizable side panel is still open.

Use the existing native resizable pattern from Forge/aggregation. Keep the active collection context and main query controls visible. Give the builder one compact toolbar and a stable Run action; do not create another full-height empty workspace.

On an empty draft, show one ready-to-edit condition with its field picker focused. An untouched placeholder row is not a predicate. Add condition is the primary local action. Add group is available next to it; drag-and-drop remains an optional shortcut.

### Condition rows

Use a consistent row: **Field → Operator → Value**, with an editable BSON type attached to the value and secondary row actions in a menu. Long paths, values, or narrow bounds may use a second row without truncating the input itself.

The field picker is searchable, supports dotted paths, and accepts fields absent from sampled metadata. Type/sample suggestions help entry but do not restrict legal values. The value editor follows the chosen operator and value type.

Use familiar labels with MongoDB syntax available as secondary information: **equals** (`$eq`), **is at least** (`$gte`), **is one of** (`$in`). Operator search must find both the friendly label and the MongoDB token.

### Groups and arrays

Every group starts with **Match all / Match any / Match none** and a clearly scoped Add action. Indentation and a labelled disclosure header show nesting; avoid guide lines, enclosing cards, and colored borders at every level. All/Any are implemented; None remains part of the advanced model proposal.

Groups can collapse to a useful summary, duplicate, move, and be removed with Undo. Moves need keyboard/menu equivalents to dragging. Empty groups remain draft errors rather than silently changing the predicate.

Array conditions need explicit scopes: matching values anywhere in an array is different from matching multiple conditions on **the same array element**. The latter gets an element group, with child fields relative to the selected array. This distinction is required by [MongoDB's `$elemMatch` semantics](https://www.mongodb.com/docs/manual/reference/operator/query/elemMatch/).

For example, users must be able to express: status is active, either tier is pro or spend is at least 1000, and one item has both SKU `A` and quantity at least 2. The element group must not match the SKU in one item and quantity in another.

### Draft, MQL, validation, and execution

- Keep one shared query draft. Visual edits and MQL edits are two representations of that draft, with one validation/execution path.
- Show generated MQL on demand, with Copy and a route to editing it. Preview generation is local; editing a rule does not trigger a database query.
- Preserve unrecognized predicates as editable MQL clauses at their original tree location. Display the operator and a useful summary so the clause remains visible in the logic.
- Invalid text or rows remain editable. Show the error at its source, focus the first error on Run, and never display or execute a silently reduced filter as though it were complete.
- Distinguish draft changes from the applied filter and from currently displayed results. Keep previous results visible while the replacement query runs.
- Rename Clear/Reset to **Clear conditions** and **Revert to applied**. Closing and reopening retains the per-collection draft. Clear and structural edits are undoable.
- Use the existing Cmd/Ctrl+Enter execution convention. Within controls, Enter accepts suggestions or commits a value; Tab navigates. Escape dismisses the innermost popup first and does not erase field/value text.

Studio 3T's [documented visual builder](https://studio3t.com/knowledge-base/articles/visual-query-builder/) supports friendly match modes, editable value types, array-element conditions, and visible shell syntax. Those are useful workflow references; the implementation should use OpenMango's existing native components and query infrastructure.

## Coverage required before calling the builder complete

| Area | Cases to cover |
| --- | --- |
| Boolean logic | Implicit/explicit AND, OR, NOR, nested combinations, field-level NOT, repeated field predicates, group movement and empty-group validation |
| Comparisons | Equality/inequality, open/closed range endpoints, one-sided ranges, several operators on the same field |
| Strings | Empty and whitespace-only strings, leading/trailing spaces, Unicode, escapes, long/multiline values, contains/prefix/suffix, regex patterns and flags |
| Presence and types | Missing, present, null, null-or-missing, empty string, empty array, empty object, BSON type checks; scalar versus array-element type semantics |
| BSON values | String, Boolean, Int32, Int64, Double, Decimal128, ObjectId, Date, Timestamp, null, arrays, documents, regex, UUID/Binary and uncommon BSON types through typed MQL values |
| Dates | Exact timestamps and milliseconds, explicit timezone, date-only range intent, inclusive/exclusive endpoints; relative dates must show when they are resolved |
| Arrays | Any/all listed values, excluded values, exact array equality/order, size, empty/nonempty, array of objects, same-element matching, nested arrays, heterogeneous values |
| Objects and paths | Literal document equality/order, nested-field conditions, array indexes, long paths, fields containing literal dots or dollar signs with the required explicit MQL expression |
| Specialist predicates | `$expr`, field-to-field comparisons, `$jsonSchema`, text, geospatial, bitwise and other server-supported operators as editable clauses; local/server errors for invalid operator placement or unavailable capabilities |
| Lifecycle | Open an existing query, edit in either representation, undo/redo, switch collections, restore history, duplicate/move groups, failed loads, empty collections, no schema sample, repeated execution |

The [MongoDB predicate index](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/) provides the operator-family inventory. The builder must preserve valid predicates outside its guided subset instead of presenting that subset as the entire query language.

Null/presence labels need explicit semantics: equality with null can include missing fields, and a query `$type` check can inspect array elements. Test these separately from exact field-type checks; MongoDB documents the differences in [null/missing queries](https://www.mongodb.com/docs/manual/tutorial/query-for-null-fields/) and [expression `$type`](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/type/).

## Implementation order and acceptance

1. **Make the model trustworthy.** Preserve typed BSON values and unsupported clauses; represent multiple field predicates, NOR/NOT and element scopes; make compilation return a complete document or errors. Route every Run entry through the same validation.
2. **Build the native editing flow.** Replace the fixed overlay and cramped toolbar, add searchable fields/operators and editable types, keep draft/preview/undo synchronized, and preserve query-bar behavior.
3. **Verify advanced behavior and usability.** Turn the probe cases into permanent regression tests; add MongoDB fixture tests for query meaning and headless GPUI tests for interaction.

Acceptance requires semantic round-trip checks (not only string comparisons), exact value/type preservation, and execution tests proving that invalid rows cannot broaden a filter. Array fixtures must distinguish different matching elements from one element matching all predicates. Include scalar/array/null/missing fixtures and large numeric values.

Headless UI coverage should include adding and focusing a row, searching and accepting fields/operators, editing types, nested groups, menu/keyboard moves, popup Escape, one-step Undo, MQL switching, invalid Run by button and shortcut, narrow bounds, UI zoom, long values, and many conditions. Final desktop verification remains with the user.

The GPUI control/API choices and current-versus-pinned source distinctions are recorded separately in [GPUI Filter Builder research](GPUI_FILTER_BUILDER_RESEARCH.md).
