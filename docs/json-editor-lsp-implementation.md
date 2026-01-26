# JSON Editor + LSP Sidecar (Low Footprint) — Implementation Plan

## Goal
Provide a scalable JSON editor with completions, diagnostics, and formatting by running a bundled `vscode-json-languageserver` sidecar process. Users must not install Node. The solution should be low‑footprint, shared across editors, and safe to ship.

## Scope
- Aggregation stage editor (primary)
- Existing JSON editors (filter, sort, projection, dialogs) should be able to opt in
- One LSP server process per app (not per editor)

## Non‑Goals
- Full VS Code feature parity (e.g., references, rename)
- Long‑running background indexing
- Custom JSON schema UI (at first)

---

## Architecture Overview

```
[InputState] --(CompletionProvider/Diagnostics)--> [JsonEditorAdapter]
                                      |
                                      v
                         [JsonLspClient (JSON-RPC)]
                                      |
                                      v
              [json-language-server sidecar binary]
```

Key points:
- `InputState` already supports LSP hooks (completion + diagnostics).
- We implement an LSP client in Rust and share it across all JSON editors.
- The sidecar is a single native binary built from `vscode-json-languageserver` and shipped inside the app.

---

## Packaging Strategy (No Node Dependency)

### Why this approach
- `vscode-json-languageserver` is the most up‑to‑date JSON LSP implementation.
- Users do not need Node installed.
- We ship a single, platform‑specific sidecar binary.

### Build pipeline (proposed)
1. Create a small Node entrypoint in `tools/json-lsp/` that wires:
   - `vscode-languageserver/node`
   - `vscode-json-languageservice`
   - standard LSP over stdio
2. Bundle JS with `esbuild` to a single file (`server.js`).
3. Produce a native binary with `pkg` (or `nexe`) per target:
   - `macos-arm64`, `macos-x64`, `linux-x64`, `windows-x64`
4. Store binaries in `resources/lsp/json/<platform>/json-lsp`.
5. App extracts the binary to a writable cache dir on first run and marks it executable.

Notes:
- This still embeds a Node runtime inside the server binary, but no external Node install is needed.
- We can optionally compress binaries (UPX) if size is a concern.

---

## Runtime Integration

### LSP client lifecycle
- A single `JsonLspClient` is created on app start (or on first JSON editor focus).
- It spawns the sidecar and runs JSON‑RPC over stdio.
- On crash, the client restarts with exponential backoff.

### Document model
Each editor session becomes a logical LSP document:
- URI format: `inmemory://json/<session>/<purpose>`
- Maintain a `version: i32` per document.
- Always send full‑text `didChange` for simplicity (can optimize later).

### Sync strategy
- On editor focus: `didOpen`
- On InputEvent::Change: debounced `didChange`
- On editor close: `didClose`

---

## JSON Editor Adapter

Create a reusable adapter to wire `InputState` to the LSP client.

Responsibilities:
- Provide `CompletionProvider` for `InputState.lsp.completion_provider`.
- Push diagnostics into `InputState.diagnostics_mut()`.
- Call `formatting` on demand for the Format button.

Suggested API:
```
JsonEditorAdapter::attach(input_state, doc_uri, client)
JsonEditorAdapter::set_schema(doc_uri, schema_json)
JsonEditorAdapter::format(doc_uri)
```

---

## Diagnostics Flow

1. LSP publishes diagnostics via `textDocument/publishDiagnostics`.
2. Convert LSP ranges → `gpui_component` diagnostics:
   - Use `DiagnosticSet::reset` and `extend`.
3. Notify the input state to re-render underlines/popovers.

---

## Completions Flow

1. `InputState` calls `CompletionProvider::completions`.
2. Client sends `textDocument/completion` with cursor position.
3. Return completion list to gpui for popup display.
4. Optional inline completions later (`inlineCompletion`).

---

## Formatting

- Format button triggers `textDocument/formatting`.
- Apply edits using `replace_text_in_lsp_range`.
- Update document version accordingly.

---

## Schema Strategy (Phase‑in)

Phase 1 (minimal):
- No schema, just JSON validation + generic completions.

Phase 2 (collection schema aware):
- If collection has validation rules or inferred schema, generate JSON Schema.
- Send `workspace/didChangeConfiguration` with schema association.

Phase 3 (user/connection overrides):
- Allow a user‑defined schema per collection or per editor.

---

## Low‑Footprint Considerations

- Single shared LSP server process.
- Debounced change notifications.
- Avoid file watchers (no workspace folders).
- Disable features we don’t need (hover/definitions) to reduce CPU.
- Keep JS bundle minimal (tree‑shake, exclude unused libs).

---

## Failure Modes & Fallbacks

- If sidecar fails to spawn, editor falls back to basic JSON editing:
  - Syntax highlighting only
  - No completions/diagnostics
- Surface a non‑blocking warning in the status bar.

---

## Proposed File Layout

```
src/lsp/json/
  client.rs           // JSON-RPC client, process management
  protocol.rs         // LSP message structs/helpers
  adapter.rs          // InputState integration
src/components/
  json_editor.rs      // UI wrapper for InputState + adapter
resources/lsp/json/
  macos-arm64/json-lsp
  macos-x64/json-lsp
  linux-x64/json-lsp
  windows-x64/json-lsp.exe
```

---

## Milestones (3 phases)

### Phase 1 — Minimal LSP wiring
- Spawn sidecar, initialize LSP
- `didOpen` / `didChange` / `didClose`
- Completion popup + diagnostics in aggregation editor only

### Phase 2 — Formatting + reuse
- Format button uses LSP edits
- Extract `JsonEditor` component and apply to other JSON inputs

### Phase 3 — Schema integration
- Add schema association per collection
- Schema‑aware completions and validation

---

## Open Questions
- Where to source per‑collection schema (inference vs. validation rules)?
- Should we allow schema overrides in UI or only via config?
- Desired debounce interval for `didChange` (150–300ms)?

