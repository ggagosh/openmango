# Native components for the Filter Builder

Research date: 2026-09-09. Status: design input; no product changes or dependency upgrades.

Reference: `refs/gpui-kit` at [`36b51819deb52c947a79f8de29e0e9175eda7464`](https://github.com/longbridge/gpui-kit/tree/36b51819deb52c947a79f8de29e0e9175eda7464).
The older `refs/gpui-component` reference was not modified or used as the compatibility baseline.
Shipping baseline: registry `gpui-kit`, `gpui-base`, and `gpui-component` **0.6.0**, as resolved in [Cargo.lock](../Cargo.lock).
Published source was inspected alongside the clone; matching version numbers in upstream manifests do not establish matching behavior.

## Decision

Compose the builder from existing native components while retaining an application-owned condition tree and BSON values.
No complete filter/query-builder component was found in the inspected Base/Component catalogue and source.
Base provides interaction/state primitives; Component provides styled controls on top. Neither supplies MongoDB condition semantics.
Use Component by default, dropping to Base only where the builder needs a different composition, not merely different colors.
[Base primitives](https://gpui-kit.com/base/), [Component catalogue](https://gpui-kit.com/component/)

The current builder already uses native inputs, an editor, calendar, switches, menus, and retained per-condition input entities.
Reuse those foundations and stable node identities; the opportunity is stronger control choice and a coherent editing flow.
[Current panel](../src/components/filter_builder/panel.rs), [current tree model](../src/components/filter_builder/types.rs)

## Recommended control mapping

| Builder need | Native choice available in 0.6.0 | Application responsibility |
| --- | --- | --- |
| Discover/select a field | Searchable `Combobox` with field-path identity | Unknown/custom paths, schema hints, recently used fields |
| Choose an operator or explicit BSON type | `Select` | Valid operator/type combinations and readable labels |
| Select several known values | `ComboboxState::multiple(true)` | Preserve typed values independently from display labels |
| Enter arbitrary or mixed-type list values | Retained `InputState` entries with removable rows/tags | Per-item type, exact draft, parsing, ordering, validation |
| Enter scalar values | `Input`, with type-specific labels/help | Exact BSON parsing and distinction between empty, missing, and null |
| Pick a calendar day | `DatePicker` / `Calendar` plus explicit timestamp input | Time, timezone, precision, and range-boundary semantics |
| Nested conditions | Native row/group composition; optional `Tree` overview | Group logic, reorder, indent/outdent, node undo, cycle prevention |
| Builder and results share space | `v_resizable` / `ResizablePanelGroup` | Minimum usable heights and persistence of chosen size |
| Generated query | Readonly native `Editor` | BSON serialization, preview revisions, and copy command |

## Field discovery: Combobox, Select, and List

Published constructors are `ComboboxState::new(delegate, Vec<IndexPath>, window, cx)` and `Combobox::new(&state)`.
Configure `.searchable(true)` on the state; `.multiple(true)` enables multiple committed values.
`SearchableVec`, grouped items, and custom `SearchableListItem` values avoid reducing an item to its displayed label.
The trigger can be customized through `render_trigger`; this suits a path plus compact type hint.
[Published Combobox](https://docs.rs/crate/gpui-component/0.6.0/source/src/combobox.rs), [Combobox guide](https://gpui-kit.com/component/combobox/)

Prefer `SelectState::new(delegate, Option<IndexPath>, window, cx)` for a small closed vocabulary such as operators.
Search is also supported by Select; Combobox earns its place when multi-selection or a custom trigger is needed.
Neither control automatically means arbitrary text creation: expose an explicit custom-field/value path if the catalogue has no match.
[Published Select](https://docs.rs/crate/gpui-component/0.6.0/source/src/select.rs), [Select guide](https://gpui-kit.com/component/select/)

Commit by stable field/value identity, not a filtered row index.
In published 0.6.0, `set_selected_values(&values, window, cx)` clears the search query before resolving values.
`set_selected_indices` addresses the currently visible rows instead; this distinction already exists in the published release.
Programmatic setters should not be mistaken for a user confirmation event.
[Current source, same value-selection mechanism](https://github.com/longbridge/gpui-kit/blob/36b51819deb52c947a79f8de29e0e9175eda7464/crates/component/src/combobox.rs#L319)

Use a custom native `ListState` only for a genuinely different suggestion interaction.
Its public `set_selected_index` does not scroll; `scroll_to_item(ix, ScrollStrategy::Nearest, window, cx)` keeps the selected row visible.
The built-in arrow handlers are crate-private, so an app-owned list also owns keyboard dispatch and confirmation.
Combobox already packages that work for ordinary field selection.
[Published List](https://docs.rs/crate/gpui-component/0.6.0/source/src/list/list.rs)

## Typed values and dates

Use text-backed draft state for exact numbers, ObjectIds, regex patterns, and timestamps.
Keep the explicit type separate from both the string draft and the committed BSON value.
Schema samples can suggest a type but should not silently overwrite a user's choice or imported BSON type.
An empty string is a value; absence of a value editor for an operator is a different state.

Do not route arbitrary BSON numeric values through `NumberInput` stepping.
Its published `NumberStep::Fixed(f64)`, `ByValue` callback, parsing, and numeric min/max path use `f64`.
That is unsuitable as the preservation path for exact Int64 or Decimal128 values.
Use it only where the domain explicitly permits its numeric representation, such as a bounded count setting.
[Published numeric engine](https://docs.rs/crate/gpui-base/0.6.0/source/src/number_input.rs)

`DatePickerState::new(window, cx)` provides a single date; `DatePickerState::range(window, cx)` provides a day range.
`date()` returns `Date`, whose variants contain `chrono::NaiveDate`; `date_format(...)` changes presentation.
The Calendar/DatePicker API does not represent time of day, timezone, or BSON millisecond precision.
A calendar should assist an explicit timestamp editor, not silently truncate an existing timestamp to a day.
If offering “on this day,” the domain must define timezone and interval boundaries explicitly.
[Published DatePicker](https://docs.rs/crate/gpui-component/0.6.0/source/src/time/date_picker.rs), [published Date type](https://docs.rs/crate/gpui-base/0.6.0/source/src/calendar.rs)

For `$in`/`$nin`, multi-select works well for known categorical values; it is not a generic BSON-array editor.
Mixed values need per-item types and drafts, with clear remove/edit actions and an accessible name for each item.
Display tags can summarize committed values; invalid drafts should remain editable rather than disappear into tags.

## Nested groups and keyboard use

Published `TreeState::new(cx)`, `TreeItem::new(id, label)`, and `Tree::new(&state)` provide hierarchy, expansion, selection, and virtualization.
Its native bindings include Up/Down and Left/Right for tree navigation/expansion.
No tree-specific reorder or drag/drop implementation was found in the inspected published Tree modules.
Reordering still needs application commands and GPUI drag/drop wiring with domain validation.
[Published Base Tree](https://docs.rs/crate/gpui-base/0.6.0/source/src/tree.rs), [Tree guide](https://gpui-kit.com/component/tree/)

A filter row contains multiple independent controls, so an editable nested form need not become a single Tree widget.
Use group headers with a clearly named conjunction control and explicit Add condition / Add group commands.
Provide move/indent/outdent commands as a keyboard alternative to dragging; preserve focus and node identity when moving.
Do not let tree-navigation shortcuts consume arrows while an input, select, or calendar is editing.
Give the builder a real focus handle and scoped action context; Enter inside its editors/menus must not invoke collection save or inline-edit actions.
Use native focus/action routing rather than relying solely on a root `on_key_down` listener to stop parent shortcuts.

## Integrated panel, popovers, and focus

Published resizable helpers take one identifier: `h_resizable(id)`, `v_resizable(id)`, and `resizable_panel()`.
Panels support `.size(...)` and `.size_range(min..max)`; groups support `.with_state(&Entity<ResizableState>)`.
`.on_resize(...)` receives the state entity, window, and app; the application owns persistence.
These support a bounded builder below the query bar without opening a separate modal for every edit.
[Published resizable helpers](https://docs.rs/crate/gpui-base/0.6.0/source/src/resizable/mod.rs), [panel API](https://docs.rs/crate/gpui-base/0.6.0/source/src/resizable/panel.rs)

Popover owns opening/dismissal and focus lifecycle. Opening focuses the tracked handle or its own handle.
Closing restores previous focus conditionally, when focus still belongs to that popover.
Use `.track_focus(...)` for the intended first field; do not stack a second bespoke focus manager around it.
Use a local draft/commit boundary for a multi-control value popover, so Escape can discard that edit without reverting the whole filter.
Nested menu Escape should close the innermost surface; test actual nested interaction before claiming it works.
[Published popover state](https://docs.rs/crate/gpui-base/0.6.0/source/src/popover.rs), [Popover guide](https://gpui-kit.com/component/popover/)

## Validation, preview, and undo

In 0.6.0, use `v_form()` / `h_form()` or `Form::vertical()` / `Form::horizontal()` with `field()`.
Fields provide labels, descriptions, required markers, and composition; these are not a validation engine.
Display each domain error adjacent to its field using a description/custom element, with explicit accessible labels on inputs.
Keep malformed or incomplete drafts visible and disable Apply with an actionable explanation; never silently substitute `{}`.
[Published Form](https://docs.rs/crate/gpui-component/0.6.0/source/src/form/form.rs), [Field](https://docs.rs/crate/gpui-component/0.6.0/source/src/form/field.rs)

Generate the preview from the typed model; keep preview generation separate from execution.
Retain its `EditorState` and update only when serialized text changes, preserving viewport and selection where practical.
Native text undo covers text edits, not add/remove/move/type/group operations in an application-owned tree.
Define those operations as builder commands with their own reversible state changes.
For an editable source view, `set_value` resets selection, scroll, and undo; `replace_all` records replacement but still resets selection/scroll.
Neither is a reason to rewrite the source editor on every render or every incomplete visual-field keystroke.
[Published input mutation behavior](https://docs.rs/crate/gpui-base/0.6.0/source/src/input/base/state.rs)

## Documentation compatibility traps

- For the current All/Any control, published `ButtonGroup` can style native pressed buttons, but its group-level callback does not receive keyboard activation. Keep callbacks on individual buttons; the app's headless test covers both keyboard activation and independent nested-group mouse selection.
- Current Form documentation uses public `Form::new()`, `.label_layout(...)`, and `.footer(...)`; these are not available in published 0.6.0. Use the constructors above and an ordinary native footer sibling.
- Current Select adds `.id(...)`; the inspected published `Select` lacks that builder method. Do not copy that example without checking the pin.
- Resizable documentation contains examples with extra state/window/context arguments; the published helpers take only `id`, with state attached using `.with_state(...)`.
- Combobox multi-select and selection-by-value are already published; they do not require the newer clone or a dependency upgrade.
- The existing native Editor is enough for preview/ordinary source editing. Live editor documentation advertises features that are ahead of 0.6.0; multiple cursors and snippet expansion should not be assumed from that page.

[Current Form source](https://github.com/longbridge/gpui-kit/blob/36b51819deb52c947a79f8de29e0e9175eda7464/crates/component/src/form/form.rs#L29), [current Select source](https://github.com/longbridge/gpui-kit/blob/36b51819deb52c947a79f8de29e0e9175eda7464/crates/component/src/select.rs#L614), [current resizable guide](https://github.com/longbridge/gpui-kit/blob/36b51819deb52c947a79f8de29e0e9175eda7464/website/component/resizable.md), [earlier editor compatibility findings](FORGE_ENGINE_RESEARCH.md)

Validation here was source/document inspection and targeted public-API comparison, not rendered UI testing.
No packages were installed, no product code was changed, and no app or test process was launched.
