# Documents workflow

Implemented on `redesign/documents-workflow`. The existing filter builder and query controls keep their layout and behavior.

## Views and actions

- Tree, Table, and JSON have labeled controls. Tree/Table retain the selection; JSON focuses one document and supports previous/next document navigation.
- JSON uses the existing editor and shared editor sessions inside Documents. “Open in window” carries the same buffer into a detached window. Pending edits and save callbacks survive collection-tab changes.
- The toolbar names its scope: selected documents, current page, matching results, or the entire collection. Save and Discard appear when selected documents have drafts.
- Right-clicking an unselected row selects that row. Right-clicking an already selected document preserves a multi-selection; field actions target the clicked field.
- Pagination uses GPUI Kit. Up to 100 pages, it shows page numbers. Larger results use compact navigation and a First/Last page menu because Kit 0.6 eagerly creates every page in an ellipsis dropdown.

## Component ownership

The implementation uses components from the published `gpui-kit = 0.6.0` dependency.

| Surface | Kit components | Application responsibility |
| --- | --- | --- |
| Tree | Kit `base::Tree`, `TreeState`, `TreeItem`, Component `ListItem` | BSON columns, document multi-selection, field edit presentation, context actions |
| JSON | `Editor`, `EditorState`, built-in JSON Tree-sitter grammar | BSON serialization, draft ownership, save/conflict handling |
| Table | `DataTable`, `TableState`, `TableDelegate` | BSON cells, stable document identities, multi-selection, persisted column preferences |
| Controls | `Button`, `Checkbox`, `Input`, `NumberInput`, `Switch`, `TabBar`, `PopupMenu`, `Popover`, `Pagination` | Action scopes and command dispatch |

The tree's extra rounded selection background and accent stripe were removed. Browsing selections use Kit's `ListItem` styling; editing rows use only a compact input frame. Kit's base Tree supplies the same tree state, keyboard behavior, and virtualization while allowing this distinction—the styled Tree reapplies selection unconditionally. Tree items remain enabled even when a field is immutable. Row click handling consumes the whole `ListItem` so single clicks select without invoking the default expansion handler. The table column picker uses labeled Kit checkboxes and buttons.

The document editor selects the JSON grammar rather than JavaScript, so keys, strings, numbers, booleans, and punctuation use the active syntax theme. No custom highlighter or dependency is required.

References: [Kit Tree](https://gpui-kit.com/component/tree/), [Kit Editor](https://gpui-kit.com/component/editor/), [Kit DataTable](https://gpui-kit.com/component/data-table/). API usage was checked against the pinned registry source; rendering remains subject to manual verification.

## Editing and clipboard

- Inline edits and single-document field dialogs stage local changes. Save commits selected drafts with an atomic comparison against the original document; conflicts retain the draft. Bulk operations retain their explicit database-write confirmation.
- Inline editing uses compact controls with one thin border, no focus halo, and a bounded width. “Done” or Enter keeps the field change in the local draft; Cancel or Escape restores the value from before that edit and preserves other staged fields.
- Moving to another row finishes a valid edit. Invalid edits stay open, show an explanation below the tree, and disable Done. Buttons and switches retain their native keyboard actions; tabbing to Cancel and pressing Enter cancels. Focus loss alone does not commit or close the editor.
- Field dialogs support type-preserving Extended JSON for specialized BSON values. String whitespace, Int64 values, nested BSON types, literal dotted keys, and nested `_id` fields are preserved. The root `_id` and its contents remain immutable.
- “Paste value” (or the existing Cmd/Ctrl+Shift+V action with a field selected) stages a value of the current BSON type. In an editor, normal paste remains a native text operation.
- Document JSON and JSONL clipboard output use canonical Extended JSON. “Copy field and value as JSON” copies a complete object with an escaped field name. Multiple documents follow displayed order.
- “Duplicate as new document” opens a prefilled insert editor. Pasting documents confirms the destination and the use of new `_id` values before inserting.
- Read-only JSON remains selectable and copyable. Projected results open a full-document JSON editor before replacement; field edits direct the user to JSON or to clear the projection.
- Page changes and refresh use the existing Save/Discard/Cancel prompt. Invalid inline values cannot be hidden by view/subtab switches or overwritten by a new query. Repeated saves and discards are blocked while a document save is pending.

## Validation

App launches, compilation, lint, and test execution were intentionally left to the user. Formatting and source/API review are not runtime verification.

Focused regression checks were added for BSON clipboard round trips, whitespace and type preservation, immutable root IDs, staged field edits, draft baselines, editor clean-state tracking, and page bounds.

Manual checks:

1. Edit a field, switch Tree/Table, then open JSON. Confirm the same draft appears. Save, reopen, and verify the BSON type and value.
2. Enter invalid JSON or an invalid numeric field. Try Save, refresh, pagination, and subtab shortcuts. Confirm the buffer remains available and errors are visible.
3. Right-click a different row, then copy, edit, duplicate, and discard. Repeat with several selected rows and with a nested field.
4. Copy/paste strings with leading/trailing whitespace, Int64 values above JavaScript's safe integer range, Decimal128, arrays, and a field containing a dot or quote.
5. Edit in JSON, switch collection tabs, and return. Repeat while a save is pending, and after moving the editor to a separate window.
6. Use a projection, edit in JSON, and verify fields excluded by the projection remain on the server. Modify the document externally before saving and verify conflict handling.
7. Check empty results, a one-page collection, more than 100 pages, page-size changes, narrow windows, and read-only connections.
8. While editing, verify there is one input border and no row-wide selection outline. Try Cancel by mouse, Escape, and Tab-to-Cancel followed by Enter. Confirm that prior edits to other fields survive cancellation and that Done keeps changes local until document Save.

Primary-source background: [Documents workflow research](DOCUMENTS_WORKFLOW_RESEARCH.md).
