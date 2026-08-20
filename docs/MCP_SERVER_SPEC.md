# Embedded MCP Server Specification

## Purpose

OpenMango embeds a disabled-by-default, loopback-only MCP server. It exposes bounded typed MongoDB reads, explicitly authorized typed document writes, and proposal/status tools for Arcula database workflows. Credentials, connection URIs, local History payloads, clipboard contents, unsaved editors, and provider secrets are never exposed.

## Authority model

Connection authority has independent persisted switches:

- `agent_shared` (**Share with agents**, default `false`) grants visibility and read access.
- `agent_writable` (**Allow agent writes**, default `false`) grants direct typed document writes and conflict-safe History restores and is meaningful only while shared.
- `read_only` is the global hard prohibition and always overrides `agent_writable`.

Removing sharing also removes write authority. Sensitive endpoint/transport/identity changes and becoming protected/Production clear both grants. Enabling direct write authority on a protected or Production connection requires a clear native warning that authenticated MCP clients can write without per-operation approval. Protected/Production is not itself a prohibition after that explicit grant.

History is independent and never required for MCP writes.

Policy is checked when live GPUI state resolves each request. Shared-but-not-writable and read-only targets fail before MongoDB receives a mutation.

## Direct typed document tools

The document tools execute immediately after authentication and authorization:

| Tool | Semantics |
| --- | --- |
| `openmango_insert_documents` | Insert 1–100 typed Extended JSON documents. |
| `openmango_update_documents` | Update one or many using a normal MongoDB update document or supported update pipeline. |
| `openmango_replace_document` | Replace at most one matching document. |
| `openmango_delete_documents` | Delete one or many matching documents. |

They do not create Action Broker proposals, native per-operation approvals, pre-read checkpoints, or pending History transitions. Structured responses include MongoDB-faithful inserted IDs and matched/modified/deleted counts.

An empty-filter update-many or delete-many requires explicit `allow_all: true`. One-document variants keep normal MongoDB empty-filter semantics. Typed request objects reject unknown fields. Recursive input validation rejects server-side JavaScript operators including `$where`, `$function`, and `$accumulator`. Namespace validation, typed Extended JSON, request/depth/stage/document limits, timeouts, output bounds, authentication, concurrency/rate limits, and audit logging apply to writes as they do to reads.

Every direct command carries a generated MongoDB `comment` document:

```json
{ "openmango_trace_id": "<uuid>" }
```

The same trace is registered with passive History for best-effort attributed grouping when History happens to be enabled. Comments do not appear in change-stream events, so attribution is never described as exact. No trace field is written to user documents and the profiler is not enabled.

The direct-write authorization seam is category-neutral: document mutations and History restores reuse the same `shared ∩ agent_writable ∩ !read_only` policy. Operations that need MongoDB also require a connected client.

## History tools

Agents can inspect bounded History metadata without receiving decrypted document keys or before/after payloads:

- `openmango_list_history_batches`
- `openmango_get_history_batch`

An explicitly write-authorized agent can start, resume, and cancel the same conflict-safe restore engine used by the native History UI:

- `openmango_restore_history_batch`
- `openmango_cancel_history_restore`

Restore requests identify an immutable batch and verify that it belongs to the authorized connection. Restores never force-overwrite a document: exact-after comparisons classify changed documents as conflicts, independent documents restore concurrently, same-document changes remain ordered, and durable progress remains resumable. `openmango_get_history_batch` exposes status and aggregate restored/skipped/conflict/failed counts for polling. Cancellation is cooperative.

History restoration is direct rather than Action Broker-gated because it is bounded, document-level, conflict-safe, and covered by the same explicit agent write grant as larger update/delete operations. Arcula database reverts remain native-approved because they replace database state from verified backup artifacts and enforce recovery interlocks.

## Arcula workflows remain approval-gated

These tools create immutable pending Action Broker proposals and never execute or approve work:

- `openmango_propose_database_backup`
- `openmango_propose_database_sync`
- `openmango_propose_operation_revert`

Only OpenMango's native Agent Activity UI can approve. MCP has no approve tool. The existing workflow retains target identity fingerprints, policy revalidation, verified backups, free-space/tool preflight, recovery interlocks, protected/Production confirmation, durable progress, cancellation, interruption recovery, and database-sync activity.

Action/operation query and cancellation tools remain because Arcula clients need them:

- `openmango_get_action`
- `openmango_list_actions`
- `openmango_get_operation`
- `openmango_cancel_operation`

Cancellation is cooperative; it is not approval. Database operation revert still requires a new native-approved proposal and a matching retained verified backup.

Removed document proposal names have no compatibility aliases:

- `openmango_propose_insert_documents`
- `openmango_propose_replace_documents`
- `openmango_propose_delete_documents`

## Read tools

Read tools remain bounded and require only explicit sharing plus a connected client:

- `openmango_list_connections`
- `openmango_list_databases`
- `openmango_list_collections`
- `openmango_count_documents`
- `openmango_find_documents`
- `openmango_inspect_collection`
- `openmango_aggregate`
- `openmango_explain_query`

Database-derived values are untrusted content. Read aggregation rejects write stages and JavaScript recursively. Responses use canonical Extended JSON where BSON fidelity matters and never include transport or secret material.

## Transport, authentication, and auditing

- Bind only IPv4 loopback.
- Use opaque per-grant bearer tokens stored through the existing secret pattern.
- Reject hostile browser origins and unauthenticated requests before dispatch.
- Enforce request-body, global concurrency, per-grant concurrency, and per-second rate limits.
- Bound MongoDB execution with server/client timeouts and response-size limits.
- Audit request metadata, grant ID, MCP method/tool, operation class, outcome, latency, and public error code.
- Never write bearer tokens, request bodies, BSON documents, connection strings, or credentials to the audit log.

Connection list responses expose `writable` separately from `read_only`, allowing clients to distinguish visibility from mutation authority without learning credentials.

## History distinction

Passive History observes eligible MongoDB change streams from all clients. It is optional, local, encrypted, retained, and can contain explicit gaps. It does not authorize, gate, pre-read, approve, or guarantee recovery for an MCP write. MCP exposes only bounded batch metadata and aggregate restore progress; encrypted document payloads stay local.

Arcula backup/sync/revert is different: it is a native-approved database workflow with verified backup artifacts and recovery interlocks. Neither subsystem is a substitute for the other.

## Acceptance invariants

- New connections are shared=false and agent_writable=false.
- Shared without agent_writable cannot mutate.
- read_only cannot mutate even with agent_writable=true.
- Direct document writes succeed without History when explicitly authorized.
- Shared agents can inspect bounded History metadata but never decrypted document payloads.
- History restore and cancellation require agent_writable and remain blocked by read_only.
- History restore verifies batch ownership and uses the existing conflict-safe, resumable engine.
- update-many is not limited by the removed 100-document History checkpoint ceiling.
- Empty-filter many mutations require `allow_all`.
- Arcula proposals remain pending until native approval; MCP cannot approve them.
- Old document proposal tools are absent from tool discovery.
