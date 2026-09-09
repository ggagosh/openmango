# Forge editor experience

Reviewed 2026-09-08 against baseline commit `c9899f0` and published GPUI Kit 0.6.0.

## Implemented in this change

- Tab accepts the selected item in a GPUI List popup; with no popup, it dispatches the native indentation action. Up/Down navigate suggestions and Escape dismisses them. Shift+Tab retains native outdent behavior.
- Ctrl+Space and typing use the same request flow. Delayed replies are checked against the tab, request generation, source text, focus, and cursor before display. The native Editor still owns text editing, highlighting, selection, and undo; Forge owns the completion session.
- All completion candidates use OpenMango's existing case-insensitive fuzzy matcher. Exact and prefix matches rank before weaker matches, with stable kind/label tie breaks. No separate prefix filter drops field or runtime candidates before ranking. Examples include `db.getcol` and `db.gc` matching `getCollection`.
- Each open Forge tab retains its own editor, selection, scroll, folding state, and undo history. Closing a tab releases its buffer. Loading a saved query remains undoable.
- Running a selection no longer changes the editor's synchronization snapshot and causes a subsequent buffer reset.
- Completion templates produce plain text without unsupported `$1`/`$0` markers. Completion replacement covers the remaining identifier when invoked in the middle of a token. This does not add snippet placeholder navigation.
- Accepting a template by Tab, Enter, or mouse now uses one synchronous editor update: validate the displayed snapshot, replace its range through the native atomic edit API, then place the caret at the first argument. Post-change inference and the toolkit completion menu's deferred insertion are no longer involved. Existing call arguments and operator values are preserved when completing a name in place.
- The selected candidate is retained by label as matching results change. Scrolling uses the native nearest-item behavior. A stale candidate cannot mutate a newer buffer; caret navigation, dismissal, and tab changes invalidate pending results.
- Enter after an opening delimiter adds one indentation level. Between matching `{}`, `[]`, or `()`, it also puts the closer on its own line. String/comment contexts retain the native Enter behavior.
- Word movement, Shift-word selection, and word deletion use consistent code boundaries, including member-access dots. macOS uses Option for word operations and keeps Command+Backspace as native line deletion; other platforms use Ctrl for word operations. Plain Backspace and Shift+Backspace retain native character deletion.
- Native paste, cut, undo, and redo dismiss completion state before editing. Pairing responds to ordinary input rather than silently altering pasted or restored text.
- The editor uses its native styled surface, folding, and a wider completion popup. The parent click handler no longer bounces focus away from the editor before refocusing it.
- Ordinary typing skips the extra JavaScript parse formerly used to check pairing on every change; unchanged buffers are not copied again on every Forge render.

Validation: compilation, Clippy, formatting, and diff whitespace checks. The app has not been launched. Editor interaction checks remain manual; automated output checks are listed below. The intermittent `db.getCollection("")co` report was not reproduced; the asynchronous insertion/caret-inference path that could permit ordering problems has been removed. Latency has not been benchmarked.

## Results and console update — 2026-09-09

- Console appends or replaces only the changed text, preserving selection and viewport. Full resets are limited to clearing or trimming old history. Long lines scroll horizontally so follow calculations use the native text engine's exact line geometry.
- New output follows the bottom. Scrolling up or selecting text pauses following; scrolling back to the bottom or using **Follow output** resumes it. Resize and pending-layout changes do not incorrectly pause following. Background output does not take keyboard focus.
- Incoming results do not override a user-selected output tab. Runs initially show Console; structured results may be selected after completion if the user has not chosen another view. Result tabs retain their own scroll and expansion state.
- Printed document snapshots are grouped by run and label, with independent row identities even when `_id` repeats. Structured documents are converted once and shared with the renderer, avoiding full document copies on every repaint.
- Console uses the shell's printed text directly. Final evaluation values remain after print lines even when UI callbacks arrive in a different order. The sidecar distinguishes an undefined return from a real null, while explicit `print(undefined)` still displays `undefined`.
- Output events are processed in batches. Existing retention limits remain 50 runs and 5,000 output lines; trimming and dropped stream events are reported instead of silently implying a complete transcript. Timestamps use local time.

Checks performed: 10 Rust output/follow/ordering checks, one shared UTF-8 edit-range check, three Bun print-format checks, sidecar bundle validation, rebuilt sidecar, and a compiled-sidecar ping. Final lint and formatting checks are completed before handoff. No desktop application was launched.

Manual checks: run enough output to scroll; scroll upward and select text while more output arrives; resume with Follow output; switch between result tabs and verify retained position; print repeated documents and inspect the grouped result; verify that real null values remain visible without extra nulls after print calls. Restart the dev app to use the rebuilt sidecar.

## Editor scope

Improve the experience of editing MongoDB scripts in the existing Forge, using Studio 3T IntelliShell as the interaction reference. Keep the existing execution commands, connection scope, shell runtime, results, query library, drafts, and write confirmations. This is an editor integration plan, not a new console or a replacement execution model.

Keep `gpui_kit::component::input::Editor`. Forge already uses it. GPUI Shell would let JavaScript describe native UI, while the same Rust editor still owns text, selection, and cursor behavior. It does not add MongoDB completion intelligence or repair editor state management. Its JavaScript runs on GPUI's main thread, so there is no basis for promising improved typing latency from that migration. See [engine research](FORGE_ENGINE_RESEARCH.md) and the supplied [engine guide](https://gpui-kit.com/shell/engine/).

The useful IntelliShell reference is the continuity of editing: start from a collection query, write normal JavaScript, receive relevant completions, run existing commands, and return to the same editing position. Its official documentation describes automatic and explicitly invoked completions across collection names, fields, operators, shell methods, and JavaScript helpers. We should use that interaction standard without copying its shortcut assignments or adding unrelated features. [IntelliShell](https://studio3t.com/knowledge-base/articles/mongo-shell-intellishell/), [shortcuts](https://studio3t.com/knowledge-base/articles/hotkeys/)

## Findings at the baseline

These are source findings from before the implementation above. No application was launched and no typing benchmark or GUI reproduction was performed during this research.

| Finding | Evidence | Consequence |
| --- | --- | --- |
| All Forge tabs share one editor entity. Switching calls `set_value`. | [editor.rs](../src/views/forge/editor.rs), [state.rs](../src/views/forge/state.rs). Published Base 0.6.0 `input/base/state.rs::set_value` resets selection, scroll, LSP state, and undo history. | Returning to a script loses the editor's working state even though its text persists. |
| Completion payloads contain unsupported snippet markers. | [completion.rs](../src/views/forge/completion.rs) supplies `$1`/`$0` and `InsertTextFormat::SNIPPET`. Published Base 0.6.0 `input/editor/lsp/overlay.rs::insert_completion` inserts `new_text` verbatim. | Accepting a template can insert literal markers; declaring the LSP format does not implement snippets. |
| Completion is heavily restricted by context. | `candidate_stage` rejects top-level, value, and array-element positions. [parser.rs](../src/views/forge/parser.rs) treats strings and comments alike for suppression. | Ordinary JavaScript and quoted collection/field positions cannot receive suitable suggestions through this path. Those positions need different policies, rather than indiscriminately showing field names everywhere. |
| Runtime completion is only a narrow fallback. | [completion.rs](../src/views/forge/completion.rs) calls the bridge only for database/collection member access when local candidates are empty; it sends the current line prefix. | Available shell suggestions cannot enrich a nonempty local list; multiline context and broader JavaScript contexts are lost before the runtime is consulted. |
| Typing performs repeated full-document work. | `try_auto_pair`, `context_stage`, and `handle_editor_change` copy text; each `parse_context` constructs a parser and parses without a previous tree. | There is avoidable work on the typing path. Its actual latency must be measured before assigning a performance claim. |
| Pairing is a second edit after the original input. | [auto_pair.rs](../src/helpers/auto_pair.rs) diffs complete strings, then inserts/removes text and moves the cursor from `InputEvent::Change`. | Undo grouping, paste, overtype, Unicode, and selection wrapping need end-to-end verification; helper tests alone do not establish correct editor behavior. |
| Schema completion warms from one document's top-level keys. | `schedule_schema_sample`, `extract_fields_from_printable`, and `build_field_suggestions` in [completion.rs](../src/views/forge/completion.rs). | Sparse documents and nested fields give incomplete suggestions. A completed schema fetch does not explicitly refresh the current completion menu. |

## What the native component actually provides

The [styled Editor guide](https://gpui-kit.com/component/editor/) covers line numbers, folding, whitespace display, search, decorations, theme monospace fonts, and normal editing controls. Forge should use these capabilities through the styled editor and its existing provider interfaces. The [Base guide](https://gpui-kit.com/base/primitives/editor/) describes the lower-level extension points; using the styled component does not preclude application-owned completion providers.

Published 0.6.0 exposes completion, hover, definition, code-action, semantic-token, and decoration interfaces. These are integration points, not a bundled JavaScript language service. Forge currently wires a completion provider. A JavaScript execution engine evaluates code; a language service analyzes incomplete code. Installing the former does not provide the latter.

There are material version limits:

- The live guide advertises multiple cursors and rectangular selection. Current upstream implements them, but the installed/published 0.6.0 source still has a single selection. Do not promise these as an existing configuration option.
- Neither the published completion acceptance code nor the upstream revision inspected in [engine research](FORGE_ENGINE_RESEARCH.md) interprets snippet placeholders. Moving to that Git revision would not solve this mismatch. The implemented Forge adapter handles initial argument placement; full placeholder traversal remains unsupported.
- The installed completion popup handles Enter, Escape, and arrow navigation; Tab acceptance and retaining the selected candidate across refreshes need toolkit-level work or a supported acceptance hook. Do not replace the whole editor to address these small, specific gaps.

Use released upstream support where available; any unreleased capability needs a deliberate dependency decision. Preserve the removal of vendored toolkit code. The research does not recommend a dependency change.

## Direction and remaining work

### 1. Preserve the editing session

Retain one native `EditorState` per Forge tab ID within the existing Forge view. Keep each editor's undo stack, selection, scroll, and folding state alive while its tab is open. Its change subscription must capture its own tab ID, so a deferred edit cannot save into whichever tab happens to be active later. Release the entity and subscription when the tab closes.

Load a buffer once. A tab switch selects the retained entity instead of replacing its contents. Apply explicit external content changes, such as loading a query, through an undoable edit operation, preserving the existing action's intended cursor placement. Published `replace_all` preserves history but still resets selection and scroll, so that placement needs deliberate handling too.

Move ongoing synchronization and focus transitions to the relevant tab/change events. Rendering should consume the current editor state. Preserve focus when results update, when the app regains focus, and when an overlay closes.

### 2. Make accepted completions correct

Use a native GPUI List with the existing MongoDB provider. [completion_menu.rs](../src/views/forge/completion_menu.rs) owns the displayed snapshot, selected candidate, and synchronous commit. [completion.rs](../src/views/forge/completion.rs) owns request cancellation and linguistic matching. Every accepted item must produce valid intended text, replace the correct token, and be one undoable action. The native Editor remains the text engine.

Until a supported snippet acceptance path exists, emit correct plain-text edits rather than unsupported placeholders. This is a compatibility repair, not full snippet support. Argument placement and Tab-through-placeholders remain explicit toolkit requirements. Never strip arbitrary dollar-prefixed text after insertion: MongoDB operators and field references are real source text. Snippet metadata must be explicit, and literal MongoDB dollars must be escaped if a real snippet parser is introduced.

Separate completion contexts: database member, collection method, query key, operator key, quoted collection name, quoted field path, and JavaScript expression. Preserve strict behavior in comments and ordinary string values. Use cached candidates immediately; refresh relevant metadata asynchronously. Bind requests to tab identity, buffer revision, cursor position, and connection/database context, extending the existing request counter rather than adding a second competing completion system.

Reuse the existing runtime completion endpoint where it can help, pass the relevant source prefix rather than only the current line, and verify its behavior with unfinished expressions before expanding its use. Completion must never execute the user's unfinished script. A full JavaScript language service is a separate decision only if the agreed examples exceed what this integration can correctly supply.

Use the existing schema cache and safe metadata reads to improve nested-field coverage without blocking typing. Refresh an open menu only when its request context still matches. Keep candidate ordering stable and show useful method/field detail using the component's existing completion presentation.

### 3. Remove friction from typing and navigation

Reuse a parser/tree per retained buffer and update it incrementally. Avoid parsing again solely to inspect a newly inserted bracket. Keep only necessary text snapshots on each edit; retain the existing debounced persistence and snapshot at its actual persistence boundary. Measure the typing path before and after these changes.

Verify native indentation, newline behavior, search/replace, word/line movement, selection, and undo before introducing custom handlers. Keep any pairing behavior the toolkit does not supply in a narrow editor integration with correct edit grouping. Do not blindly delete the current pairing helper: the installed editor was not found to provide an equivalent implementation.

Use the styled editor's font, gutter, line-height, search, and scrollbar treatment coherently with OpenMango's theme. Reduce unnecessary padding around the code surface. Keep existing run/cancel commands discoverable without changing their meanings or bindings. Completion should dismiss before Escape is allowed to act on the surrounding Forge surface; accepting a completion must not run a query.

## Acceptance examples

These are future verification scenarios, not tests executed in this research.

- Type in tab A, scroll, select text, switch to B, return to A, then undo: each tab retains its own position and edit history.
- Accept a method/operator completion: no literal `$1` or `$0`, no duplicate punctuation, no unrelated token replaced, and one undo restores the original edit.
- Accept `getCollection` or `getSiblingDB` with Tab, Enter, and the mouse: the caret is between the quotes. Accept `find`: the caret is between the braces. Invoke completion while editing an existing method name: its current arguments remain intact.
- Type `db.getcol`, accept with Tab, then immediately type `co`: expect `db.getCollection("co")`. Repeat without waiting between acceptance and typing, and with Enter/mouse acceptance. Exercise rapid type/backspace, cursor movement, and delayed responses separately.
- Verify `getcol`, `GETCOL`, and `gc` match the method in database-member context. Move the suggestion selection, refine the query, and verify that the selected candidate stays selected when it still matches.
- At `find({|})`, press Enter: the result is `find({\n  |\n})`. Repeat with arrays, nested indentation, and spaces inside the pair. Brackets occurring inside strings or comments must not cause structural formatting.
- At `db.getCollection|`, Option+Backspace on macOS removes `getCollection` and keeps `db.`; Option+Left and Option+Shift+Left use the same boundary. Command+Backspace removes to the line start and Shift+Backspace removes a character. Verify the Ctrl equivalents on non-macOS platforms.
- Open completion, then paste text resembling a completion or use undo/redo: neither operation should jump into an argument or synthesize another bracket pair.
- Try `db.us`, `db.users.fi`, `db.getCollection("us")`, filter keys, a quoted dotted field, and a multiline aggregate. Suggestions fit the actual position; ordinary comments and string contents stay quiet.
- Type quickly, move the cursor, switch databases/tabs, and receive a delayed completion: stale results do not change the current menu or buffer.
- Add braces/quotes, overtype a closer, wrap a selection, paste a large block, and undo/redo. Verify Unicode and IME input as well as ASCII.
- Navigate the completion menu and search panel under the keyboard. Enter/Escape affect the active editor overlay; the existing Run shortcuts still run exactly their documented scope.
- Type and scroll while a query produces output, with a warm cache, cold cache, slow server, and disconnected server. Input never waits for metadata.
- Exercise short queries and approximately 50 KB/250 KB scripts. Record foreground edit time, completion latency, and frame stalls on the same build/profile and machine. Performance budgets are acceptance targets to choose from those measurements, not claims already demonstrated.

The implemented scope covers retained per-tab editing, native completion acceptance with first-argument placement, structural newline indentation, code-word editing, and the focus/pairing fixes listed above. Broader completion context coverage, incremental parsing, richer schema suggestions, and toolkit-level snippet/multiple-cursor support remain separate work described here; they are not claimed as implemented.

## Sidecar build and performance — September 9, 2026

The compiled sidecar now uses Bun's ESM bytecode with depth 1, minification, preserved function names, and embedded source maps. CI, nightly, and release workflows pin Bun 1.4.2; build/check scripts enforce the existing lockfile. The package build command delegates to the architecture-aware build script.

Measured on this Apple Silicon Mac using Bun 1.4.2 and a disposable MongoDB 7.0 container. These are medians of seven fresh processes per variant, alternating order after one warm-up each. The filesystem cache was warm; startup measures process spawn through its first `ping` reply, not GUI startup or first-download launch checks. Connection and subsequent timings exclude process startup. Binary sizes use decimal MB.

| Measurement | Previous build | Optimized build |
| --- | ---: | ---: |
| Startup to `ping` | 215.3 ms | 58.5 ms |
| Create a MongoDB shell session | 54.0 ms | 41.8 ms |
| First `db.runCommand({ping:1})` | 13.9 ms | 13.5 ms |
| Runtime `db.getCol` completion | 1.5 ms | 1.5 ms |
| Receive 1,000 `printjson` events and evaluation reply | 23.7 ms | 23.6 ms |
| Executable size | 83.33 MB | 96.23 MB |

Startup improved by about 73%, with a 12.9 MB size increase. Separate nine-run comparisons found minification with source maps alone took about 196 ms / 83.81 MB; unrestricted bytecode took about 46 ms / 114.50 MB. Depth 1 keeps most of the startup improvement at a lower size cost. Completion and print throughput were essentially unchanged, so no speculative runtime replacement or IPC rewrite was added. These measurements do not establish remote database latency, other architectures, or GPUI rendering throughput.

The previous global 30-second timer closed providers and deleted shell sessions even while an evaluation was running. Sessions now survive inactivity and are disposed asynchronously when their Forge tab closes, or through the existing restart/cancel actions. This avoids periodic reconnection and loss of shell variables.

Validation: optimized executable built successfully; three formatting tests and two compiled-sidecar tests passed. The latter exercise RPC errors, runtime completion, BSON output, `null` versus `undefined`, a query lasting over 30 seconds, another idle session retaining variables during that query, and explicit session disposal. The output work separately passed 10 Rust output checks and one Unicode edit check; final all-target Clippy and formatting checks passed. GUI behavior and remote CI were not run.

To repeat the compiled-sidecar checks, build with `just build-sidecar`, then run `bun test ./src/format.test.ts ./src/sidecar.test.ts` from `tools/forge-sidecar`, with `FORGE_SIDECAR_BINARY` set to the absolute executable path and `FORGE_TEST_MONGODB_URI` set to a disposable MongoDB instance. Without those variables, the corresponding integration checks are skipped.

Build options were checked against Bun's primary documentation: [bytecode caching and depth](https://bun.com/docs/bundler/bytecode), [standalone executables](https://bun.com/docs/bundler/executables), and [minification](https://bun.com/docs/bundler/minifier).
