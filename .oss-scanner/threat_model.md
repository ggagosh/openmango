# Threat model

## What this project does and where untrusted input enters
OpenMango is a desktop MongoDB client (Rust, GPUI). The user is trusted; their machine is not
shared. Untrusted input enters from:
- **MongoDB servers**: documents, schemas, stats, explain plans and errors returned by any server
  the user connects to, including a malicious one. Rendering, editing, exporting and comparing that
  data must not crash the app, run code or touch files the user did not pick.
- **Connection files and URIs** the user imports or pastes (`src/helpers/connection_io.rs`,
  `src/components/connection_manager/import.rs`).
- **Imported files**: JSON, CSV and BSON dumps (`src/connection/ops/import.rs`, `csv_utils.rs`,
  `bson_tools.rs`, which drives the bundled `mongodump`/`mongorestore`).
- **The local MCP server** (`src/mcp/`): an HTTP listener on 127.0.0.1 for AI agents, guarded by a
  bearer token and per-connection sharing and write policy. Other local processes and web pages
  (DNS rebinding, cross-origin requests) are untrusted.
- **AI tool calls** (`src/ai/`): model output, which may be steered by prompt injection hidden in
  database content, is untrusted. Writes are classified in `src/ai/safety.rs` and must be confirmed.
- **SSH servers** reached by the tunnel (`src/connection/tunnel.rs`); with strict host-key checking
  on, a changed key must be refused.
- **The updater** (`src/state/commands/updater/`): release manifests and downloads are untrusted
  until their minisign signature verifies.

## Components that matter most / least
- Most: the MCP server and its auth/policy, the AI write path, the updater, SSH host-key
  verification, and secret handling. Connection secrets, AI API keys and MCP tokens live in the OS
  keychain (`src/helpers/keystore.rs`); history, AI memory and task runs are SQLCipher databases
  keyed from it. Secrets must not leak into config files, logs, telemetry, AI prompts or MCP
  responses.
- Less: UI layout and theming. `refs/`, `dist/`, `target/` and `tools/forge-sidecar/node_modules/`
  are out of scope.

## How to exercise it
- `cargo test --lib -- --test-threads=1` runs the unit tests (already built in the image).
- `tests/*_tests.rs` are integration suites that need MongoDB via Testcontainers (Docker), which the
  scan environment does not have.

## How you rate severity
- Critical: code execution, or reading/writing the user's files or credentials, triggered by a
  remote MongoDB server, an imported file, a web page, or an unauthenticated local process.
- High: bypassing the MCP token or write policy, an AI write that skips confirmation, an update
  accepted without a valid signature, an SSH connection accepted on a changed host key, or secrets
  leaking to disk, logs or the network.
- Medium: crashes or hangs caused by malicious server data or imported files.
- Low: issues that need the user to paste and run something obviously hostile.

## Anything to leave alone
- The "before connect" command (`src/connection/before_connect.rs`) runs a shell command the user
  typed, by design. Report it only if something other than the user can set or change it.
- Dependencies (GPUI, the MongoDB driver, etc.); report only bugs in this repository's code.
