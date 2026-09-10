# Document query bar

The collection query bar serves Tree and Table views with the same persisted draft and applied MongoDB filter. Its primary entry path produces standard MongoDB filter documents; existing shorthand filters remain supported.

## Interaction

- A visible **Find** action runs the current filter. **Add condition**, **Options**, **History**, and **Explain** use the existing query tools. Reset clears the filter and reloads documents.
- Enter and Find/Apply format valid filter, sort, and projection documents before running them: `{backend:ObjectId("…")}` becomes `{ backend: ObjectId("…") }`. Formatting changes whitespace only, keeps line breaks with two-space indentation, preserves the caret/selection, and is undoable in one step. Values, constructors, quoted keys, field order, and comments retain their original text. Scalar IDs and shorthand retain their entry form. Typing and Shift+Enter do not trigger formatting; Enter still accepts an open completion first.
- The editor grows from one to four lines, including a trailing empty line. The expand control provides ten lines. Shift+Enter inserts an indented newline without running a query.
- Filter, sort, and projection use GPUI Kit's themed Editor surface with the folding gutter disabled. The main actions share a height derived from the same font metrics and native editor padding; secondary toolbar buttons use the toolkit's Small size. Focus changes do not reposition the caret.
- Query inputs disable file-editor overscroll and surrounding-line margins. A collapsed one-line query cannot scroll into blank rows and hide its caret.
- The header preserves child input focus when left clicks bubble up from filter, sort, or projection. It must not blur the window after an editor receives focus.
- Tab or Enter accepts an open completion. Enter otherwise runs the query. Ctrl+Space requests suggestions. Escape dismisses completion/date popups and preserves the draft.
- Field suggestions show types, support fuzzy matching and dotted paths, and preserve an existing field value when editing its key. Quoted keys and values use complete replacement ranges.
- Accepting a field offers values from loaded documents. Sampling is bounded to 100 documents, 1,000 visited values, and 20 distinct short scalar values; suggestions identify these as loaded values, not an exhaustive database enum. Numeric/date fields also receive appropriate comparison operators.
- ObjectId constructors place the caret inside the quotes. Constructor and operator placeholders are removed before insertion; literal dollar-prefixed values remain intact. Date constructors retain the existing calendar picker, with undoable insertion.
- A bare 24-digit hex ObjectId, an explicit ObjectId/UUID constructor, or a quoted string ID becomes an exact `_id` filter. The bar displays the interpreted type. A pasted bare ObjectId also offers a string-ID alternative.
- Incomplete typing receives neutral feedback. Enter on an invalid filter shows the parser error. Valid draft changes are explicitly marked as not applied.
- Existing results remain visible during replacement queries and failures. Loading, failure, no matches, and an empty collection have distinct messages.
- Drafts retain line breaks across collection tabs. History restores synchronize even if focus has returned to the query input; replacement within the same collection remains undoable.

## Implementation

- [Query bar](../src/views/documents/header/filter_bar.rs): native layout and query feedback.
- [Query editor](../src/views/documents/query_editor.rs): coalesced completion requests and keyboard routing.
- [Completion provider](../src/views/documents/query_completion.rs) and [value context](../src/views/documents/query_values.rs): field/type/value suggestions and exact source ranges.
- [Shared completion menu](../src/views/editor_completion.rs): native GPUI List, generation/source/scope guards, and atomic text/caret edits. Observing editor paint without a state or anchor change does not request another menu render.

## Manual UI checks

1. In Tree, type a field prefix and accept with Tab. Choose a suggested value, then press Enter to run. Repeat in Table and verify the same query and results.
2. Paste an ObjectId. Check the visible type, select either the ObjectId or string-ID suggestion, and run. Reset afterwards.
3. Edit a quoted key in the middle of an existing query. Accept completion and verify its value is preserved. Undo restores the original query.
4. Use Shift+Enter inside `{}` and after a final line. Verify indentation, automatic height, and no query execution. Expand, collapse, change collection tabs, and return.
5. Open History and restore a query. Verify filter, sort, and projection update, including when their input regains focus. Test undo and Escape without losing a draft.
6. Run a slow query, edit the draft while it runs, and verify prior results stay visible with accurate feedback. Exercise a query error and a successful query with no matches separately.
7. Submit a valid query with inconsistent spacing using Enter, then Find. Verify consistent formatting, no extra query from an autocomplete acceptance or Shift+Enter, and one-step undo. Repeat with sort/projection and a multiline query containing a comment or `NumberLong` value.

Automated Rust checks cover typed ID filters, completion text/caret metadata, quoted and nested replacement ranges, sampled value traversal, draft equivalence, empty-state distinctions, and the existing query/Forge behavior. The app is not launched automatically; final desktop visual verification remains with the user.

Headless GPUI tests run inside the toolkit Root and the production header container. They cover left/right-click focus retention, control alignment, font-size changes, collapse transitions, focus across the field's hit area, painted caret visibility, Unicode, horizontal/vertical scrolling, Shift-click and drag selection. Run them with `cargo test --lib views::documents::header::filter_bar::tests -- --test-threads=1`.

The published `gpui-pre` 0.3.4 dependency has a last-glyph rounding issue in `closest_index_for_x`: clicking just after the final glyph's leading edge jumps to the line end. A query-only compatibility guard uses the editor's public IME text bounds and corrects only that native result after normal mouse handling. No toolkit sources are vendored or modified. Remove the guard after the upstream behavior is fixed. Completion anchoring also accounts for the SDK's already-scrolled caret X coordinate.

Component references: [Editor](https://gpui-kit.com/component/editor), [Button](https://gpui-kit.com/component/button), and [native design guidance](https://gpui-kit.com/docs/design-guides/).
