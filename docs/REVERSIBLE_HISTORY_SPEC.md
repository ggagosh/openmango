# History Specification

## Status and scope

History is an optional, passive MongoDB change-stream recorder. It is disabled by default and never participates in the write path. A failed, unavailable, or disabled recorder must never block a MongoDB write.

History records supported changes observed by OpenMango on this device. It may include writes from OpenMango UI, MCP, Forge, shells, drivers, jobs, or other applications and can contain gaps. It is not a backup, an audit/compliance log, or a guaranteed complete journal.

The first version records only document **update**, **replace**, and **delete** events. Inserts, indexes, views, time-series collections, collection/database DDL, and other event types are not recovery items.

Database backup, sync/replacement, and database-operation revert are separate Arcula workflows. They keep verified backups, recovery interlocks, progress, cancellation, Agent Activity, and native approval. History does not replace or weaken them.

## Eligibility and setup

A connection is eligible only when all mandatory requirements can be proven:

- MongoDB 6.0 or newer;
- replica-set or sharded topology (standalone is unsupported);
- WiredTiger storage;
- change streams available to the authenticated user;
- covered regular collections are readable and have `changeStreamPreAndPostImages` enabled.

Views and time-series collections are explicitly uncovered. The UI disables History and reports the exact failed requirement when version, topology, storage, `find`, or `changeStream` checks fail.

Enabling is an explicit two-step operation:

1. **Inspect eligibility** reads server/topology metadata and collection options.
2. **Enable pre/post images** offers an explicit `collMod` setup action for uncovered regular collections. Missing `collMod` privilege is reported; OpenMango never silently disables pre/post images later because another application may depend on them.

When a regular collection appears while recording is enabled, the deployment-level supervisor retries the already-authorized setup. Failure creates a visible coverage gap. No standalone or pre-6.0 fallback exists.

## Recorder boundary

`src/history/` is the deep module boundary. UI and command callers use a small service API for lifecycle, eligibility/setup, paginated batches/details, restore, gaps, retention, usage, and clear operations. Callers do not manage cursors, tokens, encryption, batching, or restore recipes.

The recorder runs MongoDB and SQLite work off the GPUI thread. One connection supervisor owns database-level watchers; there is never one watcher per open tab.

Each watcher requests:

- `fullDocumentBeforeChange: "whenAvailable"`;
- `fullDocument: "whenAvailable"`.

`updateLookup` is never used as a recovery image because it can observe a later concurrent version. Servers that support it use `$changeStreamSplitLargeEvent`; unsupported oversized-event failures become gaps.

## Durable local model

The clean SQLite schema contains:

- `history_batches`: displayed change sets and aggregate restore state;
- `history_items`: AES-256-GCM encrypted document key and exact before/after images;
- `history_cursors`: AES-256-GCM encrypted resume state per connection/database;
- `history_gaps`: durable discontinuities and coverage/storage failures.

The installation key is generated once and kept in macOS Keychain. Connection URIs and credentials are never stored in History.

An event item and its new cursor are committed in one SQLite transaction. A unique resume-token hash deduplicates replay. On reconnect/restart the watcher resumes from the encrypted token. Missing, invalid, or expired tokens create a durable visible gap before recording restarts at the current point. Missing exact images likewise advance the cursor only after recording a gap; no false recovery item is created.

If encryption or persistence fails, that recorder stops and attempts to persist/surface a gap. The originating database write remains unaffected.

## Change-set grouping

History never presents one top-level row per document.

- **Transaction (exact):** events sharing `lsid + txnNumber`.
- **Attributed (best effort):** an OpenMango-controlled command has a local trace ID, command time window, namespace/family, and returned affected count. OpenMango attaches `{ openmango_trace_id: ... }` as the MongoDB command `comment`, but comments are absent from change events, so correlation is explicitly not guaranteed.
- **Observed change set:** adjacent compatible events in the same namespace/family. This is a local burst, not a claim about one original MongoDB command.

Observed batches use a 1-second idle boundary, 30-second maximum duration, and 100,000-item continuation ceiling. A 10,000-document `updateMany` normally appears as one or a few batch rows while preserving all exact encrypted per-document images internally. Items are decrypted only for detail pagination or restore.

The Collection History UI shows grouping quality, namespace, family, item/revertible/conflict counts, time range, encrypted size, status, document-key samples on demand, and prominent gaps. It does not claim complete coverage while any gap or uncovered collection exists.

## Conflict-safe restore

Restore requires native write/Production confirmation and runs in bounded background chunks with durable progress.

- Update/replace: restore `before` only if the current document exactly equals recorded `after`.
- Delete: insert `before` only if that `_id` remains absent.
- Existing `before` state is treated as already restored; any other current state is a conflict.

Restore never force-overwrites. It records restored, skipped, conflicted, and failed counts. Cancellation and restart retain honest partial state; interrupted applying items become failed/partial rather than being reported as successful. Restore writes are ordinary MongoDB writes and therefore produce ordinary change-stream events.

Because observed batches can be heuristic, confirmation shows namespace, grouping quality, time range, item count, and the conflict-safe rule before execution.

## Retention and clearing

Every connection has persisted maximum age and encrypted-byte limits. Defaults are 30 days and 1 GiB. Retention purges oldest batches first and never deletes a batch with active restore work. Current encrypted bytes, batch count, and item count are displayed.

Supported clear scopes are one batch, one collection, one connection, and all History. Active restores are excluded. Storage-limit failures become visible gaps rather than blocking database writes.

## Non-goals

History does not provide insert undo, DDL/index recovery, collection snapshots, encrypted archive artifacts, pre-write recipes, verified backup guarantees, universal origin attribution, profiler-based tracing, or trace fields in user documents.
