# Documents workflow research

Reviewed 2026-09-10 against current primary MongoDB and Apple documentation.
Scope: Collection → Documents, tree/JSON views, edits, clipboard actions, pagination, menus, toolbar, and subtabs.
The recommendations below are OpenMango design implications, not assertions that Compass implements every proposed behavior.
Preserve the completed Filter Builder and existing native GPUI Kit visual system; no product code was changed for this research.

## Core direction

Treat Documents as one task surface with several representations of the same result set and staged document drafts.
Keep collection-level subtabs separate from the Tree/JSON representation switch inside Documents.
The toolbar should expose query execution/refresh, insertion, representation, and clearly scoped document actions.
Put selected-document commands near the selection; page navigation and result counts belong with the result set.
Changing representation must not execute a write, rerun a query unnecessarily, or discard a draft.

Compass offers List, JSON, and Table representations: JSON displays Extended JSON for BSON types, while List expands nested objects/arrays.
Its clipboard actions distinguish a whole document from a selected field/value pair.
That is a useful precedent for consistent scope across OpenMango's existing representations.
[Compass: View Documents](https://www.mongodb.com/docs/compass/documents/view/)

## Make every action's scope explicit

| Command | Target | Required behavior |
| --- | --- | --- |
| Copy field name / path | Selected field | Preserve the actual key/path; distinguish a literal key containing `.` from a nested path |
| Copy value | Selected BSON value | Serialize its complete typed value, not an ellipsized preview |
| Copy field & value | Selected field | Produce a one-field Extended JSON object with the original key |
| Copy document | Selected document | Produce one complete document; never silently copy the page |
| Copy selected documents | Explicit selected IDs | Produce a document array and show the count |
| Copy current page | Loaded page | Name that bounded scope; do not call it “all documents” |
| Export all matching documents | Current executed query | Separate operation using the filter, not just the loaded page |
| Delete field / element / document | The named object | These are different mutations and need different labels |

When a query projects fields away, distinguish the displayed result from the full stored document.
Whole-document editing/replacement needs a full source document and stable identity; do not reconstruct it from a projected or truncated row.
If identity is unavailable, do not guess the target. Keep `_id`/document identity separate from field selection and display formatting.
These are implications of MongoDB's full-replacement semantics, not cosmetic preferences.
[MongoDB: findOneAndReplace](https://www.mongodb.com/docs/manual/reference/method/db.collection.findOneAndReplace/)

## Preserve BSON through text and clipboard

Use **Canonical Extended JSON v2** for the authoritative copy/edit round trip; offer Relaxed JSON only as an explicitly readable representation.
MongoDB documents that Canonical generally preserves BSON type information, whereas Relaxed can lose it.
Canonical wrappers distinguish Int32, Int64, Double, Decimal128, Date, Timestamp, ObjectId, binary, and regex values.
Do not flatten those values through an ordinary JSON-number or plain-string conversion before copying or saving.
Examples: Int64 `{"$numberLong":"9007199254740993"}`, Decimal128 `{"$numberDecimal":"1.10"}`, and Date `{"$date":{"$numberLong":"0"}}`.
The JSON display can be friendlier than the stored editing representation, but display strings are not the data model.
[MongoDB: Extended JSON v2](https://www.mongodb.com/docs/manual/reference/mongodb-extended-json/)

Preserve empty strings, significant string whitespace, null, empty arrays/objects, numeric types, regex options, and binary subtypes.
Missing field is distinct from a field whose value is null. A type change must be explicit rather than inferred from the edited text alone.
Copy from BSON-backed state or an explicitly parsed draft; never copy a shortened tree-cell label.

Canonical Extended JSON is not a universal lossless envelope: MongoDB documents ambiguity when user field names collide with `$`-prefixed type wrappers.
Treat such documents as a known format boundary; do not silently reinterpret an ordinary object as a BSON scalar wrapper.
Literal dotted field names also require special handling: ordinary dot notation means traversal, not an escaped field name.
[MongoDB: dollar and period field-name considerations](https://www.mongodb.com/docs/manual/core/dot-dollar-considerations/)

## Paste and insert

Native Paste inside a focused text editor should remain text editing; it must not unexpectedly insert database documents.
An explicit **Insert document…** or **Paste documents…** action opens a staged editor and validates before writing.
For multi-document paste, show the parsed document count and use **Insert N documents**; retain failures with their document index and draft.
Reject unsupported top-level values with a specific error instead of accepting a scalar as a document.
Do not change `_id` during ordinary copy. A separate Duplicate document command may intentionally prepare a new draft with a new/omitted ID.

Current Compass documents three insertion modes: Shell Syntax, Field-by-Field, and JSON.
Shell Syntax allows constructors such as `ObjectId()`; JSON expects Extended JSON; both text modes accept arrays of documents.
Field-by-Field supports one document with explicit value types. If `_id` is omitted, Compass generates one.
Do not call arbitrary shell syntax “JSON,” and do not evaluate pasted source merely to make an Extended JSON parser accept it.
[Compass: Insert Documents](https://www.mongodb.com/docs/compass/documents/insert/)

## Staged edits and save semantics

Maintain an original BSON snapshot, the editable draft, and its dirty state for each document being edited.
Field editing should expose the field's type and distinguish rename, replace value, remove field, and remove array element.
Show changed fields and offer a local revert; provide explicit Save changes and Discard for the document.
Invalid JSON remains visible and recoverable. Switching to Tree must not replace an unparseable draft with server data or `{}`.
On write failure, retain the draft and selection, show the error beside the document, and allow correction/retry.
On success, clear only the draft revision that was actually saved; later keystrokes must remain dirty.

Compass List/Table editing updates only changed fields using `findOneAndUpdate`; its JSON editing uses `findOneAndReplace`.
It highlights changes, supports field-level reversion, and has explicit Update/Cancel actions.
It also reports detected external changes rather than silently overwriting them.
OpenMango should likewise distinguish a patch from whole-document replacement; a full JSON replacement from incomplete source is unsafe.
[Compass: Modify Single Document](https://www.mongodb.com/docs/compass/documents/modify/)

MongoDB single-document writes are atomic, but `_id`-only writes can still overwrite another client's edits.
Use an expected original value/version in the write predicate where appropriate; a separate read-then-write check alone leaves a race.
A failed conditional match should produce a conflict/missing-document state and keep the local draft, not silently upsert or claim success.
Whole-document replacement needs a guard appropriate to the whole snapshot, not just one unchanged field.
[MongoDB: Atomicity and Transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)

Removing an array element is not equivalent to removing an object field.
MongoDB documents that positional `$unset` leaves a null array element rather than shrinking the array.
Keep typed path segments and implement the intended array mutation; a displayed dotted path is insufficient for every BSON key shape.
[MongoDB: $unset](https://www.mongodb.com/docs/manual/reference/operator/update/unset/)

## Pagination, refresh, and pending edits

Next/Previous, page-size changes, query execution, refresh, collection changes, and subtab changes need one consistent pending-edit policy.
Either retain drafts by document identity across navigation or ask Save / Discard / Cancel before replacing their source data; do not mix policies between Tree and JSON.
If Save is asynchronous, perform navigation only after acknowledged success. Failure or Cancel leaves the current page and draft intact.
Recheck dirty state at the destructive transition, not only before an asynchronous prompt/save started.
An old page request must not overwrite a newer query/page or reset the current selection; bind responses to the executed query and page generation.
Display the actual visible range and distinguish loaded count, matching count, and collection count. Avoid claiming an exact total before it is known.

For offset pagination, MongoDB recommends a sort containing a unique value, commonly `_id`, for consistent ordering among equal sort values.
`skip()` becomes slower at large offsets; indexed range pagination can avoid scanning skipped records, but is not a universal replacement for arbitrary sort/page-number behavior.
Stable ordering does not make independently fetched pages a frozen database snapshot while writes continue.
[MongoDB: cursor.skip](https://www.mongodb.com/docs/manual/reference/method/cursor.skip/)

Bulk actions must be named separately from page actions.
Compass bulk update uses the Query Bar's filter and previews the result; an empty filter targets the collection.
For OpenMango, **Update all matching…** must preserve that query scope and show it before committing, never inherit only the current selection/page by accident.
[Compass: Modify Multiple Documents](https://www.mongodb.com/docs/compass/documents/modify-multiple/)

## Context menus, toolbar, and keyboard ownership

Right-click should target the clicked document/field; visible selection and command target must agree.
For an existing multi-selection, clearly state the selected count when an action affects the whole selection.
Keep field menus short: edit/revert, copy variants, then applicable remove commands. Keep whole-document and query-wide commands out of a field menu unless explicitly labeled.
Expose essential edit/copy/insert commands through ordinary controls or the menu bar as well; the context menu is an accelerator, not the only discovery path.
Apple recommends relevant, compact context menus, consistent availability, and main-interface equivalents; a scope title is useful when it clarifies a multi-item action.
[Apple: Context menus](https://developer.apple.com/design/human-interface-guidelines/context-menus/)

While a native Input/Editor is focused, Copy/Paste/Select All/Undo belong to that editor.
Document-level save/delete/page shortcuts must not intercept text edits, popup confirmation, or IME composition.
Focus should return to the edited field/document after Save or Discard; changing view mode should retain document identity and useful expansion/scroll state.
Preserve the existing filter controls and native Kit components; this task needs clearer command/state ownership, not a second visual language.

## Focused acceptance checks

- Round-trip representative BSON types through copy → paste → edit → save; include large Int64, Decimal128, regex flags, binary subtype, dates, and mixed arrays.
- Verify selected field, document, selected documents, current page, and all-matching actions never cross scopes.
- Edit a projected document without losing omitted fields; reject whole-document replacement when full source/identity is unavailable.
- Preserve malformed drafts and server-error drafts across retry, view-mode changes, and cancelled pagination.
- Simulate a concurrent server edit and a late save/page response; keep newer local work and report conflicts accurately.
- Exercise nested object keys containing periods, empty strings/null/missing fields, and array removal semantics.
- Verify Copy/Paste and Enter/Escape with field editors, JSON editor, menus, and dirty prompts focused.

This is source-backed design research, not a claim about current OpenMango behavior or completed runtime verification.
No builds, tests, application launches, or product edits were performed.
