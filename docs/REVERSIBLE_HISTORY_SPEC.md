# OpenMango Reversible Operation History — Design Direction

> **Status:** Manual single-document insert, replacement, and delete slices implemented
> **Scope:** General OpenMango feature used by users, built-in AI, and MCP clients

## Implemented document slices

The initial implementation is deliberately narrower than the full direction below:

- manual single-document insertion, existing-document replacement, and deletion are tracked when the connection's default-off switch is enabled;
- recovery envelopes contain exact BSON before/after images and `_id` values, encrypted with AES-256-GCM using a random Keychain-held installation key;
- bundled SQLite stores the authoritative operation projection, append-only lifecycle events, and encrypted item payloads through one serialized worker;
- conditional insertion, replacement, and deletion, linked conflict-safe restore, paginated collection History with document and field previews, and startup/connection reconciliation are implemented;
- bulk/metadata/snapshot recipes, retention/purge controls, and agent/MCP routing remain future slices.

A missing Keychain key never replaces the key for an existing history database. Existing recovery data is preserved and tracked writes remain failed closed until the key problem is resolved.

## Decision

OpenMango should provide **application-owned reversible history** for mutations executed through OpenMango. It must not claim to capture writes made by other applications, shells, drivers, or server-side jobs.

The feature is broader than AI:

- users enable reversible history independently for each connection;
- manual, built-in AI, and MCP writes use the same mutation seam;
- every operation records its origin;
- revert creates a new operation instead of deleting or rewriting history;
- agents may execute only operations for which OpenMango can create a verified recovery recipe;
- read-only queries and aggregations remain in Query History, not Operation History.

Studio 3T is a useful product reference, but its public Collection History is narrower: it locally records selected document updates and deletes, groups documents by operation, previews before/after values, calculates restore conflicts, and supports age/size purging. It does not document general rollback for inserts, indexes, collections, databases, scripts, or write aggregations.

## Architecture

### Domain terms

- **Operation** — one durable OpenMango mutation attempt. It has an origin, target, lifecycle, outcomes, and optional recovery recipe.
- **Action proposal** — an immutable request awaiting approval. It is not an operation and does not imply execution.
- **Recovery recipe** — encrypted material and rules required to reverse an operation safely.
- **Revert operation** — a new operation that applies another operation's recovery recipe. The original record remains unchanged.
- **Conflict** — current MongoDB state does not match the state the recovery recipe expects.
- **History** — a queryable projection of operations, not a second execution system.

### One deep module

Create `src/operations/` as the only mutation seam used by manual UI, built-in AI, and MCP clients:

```text
Manual UI ─┐
Built-in AI ├─ MutationRequest + Origin ─> OperationEngine ─> MongoDB
MCP client ─┘                                  │
Action approval ───────────────────────────────┘
                                               │
                                      DurableOperationStore
                                               │
                                      encrypted recovery data
```

The small external interface is:

```text
execute(context, mutation) -> OperationId
revert(context, operation_id) -> OperationId
get(operation_id) -> OperationDetails
list(query) -> Page<OperationSummary>
reconcile() -> ReconciliationReport
```

Callers never capture before-images, persist records, issue inverse writes, or select a recovery strategy themselves. That behavior stays behind the module interface.

Internally:

- `model.rs` — pure operation, origin, status, event, target, conflict, and recipe types;
- `engine.rs` — prepare/apply/record/revert/reconcile state machine;
- `planner.rs` — classifies requests and builds bounded recovery recipes;
- `mongodb.rs` — real MongoDB adapter for conditional document transitions and metadata checks;
- `store.rs` — SQLite metadata, events, item outcomes, retention, and schema migrations;
- `crypto.rs` — authenticated encryption using a random per-install key stored in macOS Keychain;
- `snapshot.rs` — adapter over the existing verified archive backup/restore implementation.

`src/actions/` remains approval policy and proposal handling only. On native approval it calls `OperationEngine::execute`. `src/sync/` remains the snapshot execution implementation but no longer owns a separate notion of operation history.

### Durable store

Use bundled SQLite through `rusqlite`, rather than directory-scanned JSON, Turso Cloud, libSQL, or the newer Turso embedded engine. Operation history is expected to contain many operations and potentially many item outcomes; it needs indexed pagination, atomic state transitions, retention queries, and migrations—not concurrent-write throughput.

Why not Turso here:

- Turso's strongest published performance advantage comes from experimental MVCC under contended concurrent writers. OpenMango should deliberately serialize local journal writes through one worker, so that advantage does not apply; Turso's own benchmark reports SQLite faster for its single-thread/no-compute case.
- The Turso engine is still approaching full SQLite compatibility, with partial SQL/PRAGMA support and documented behavior differences. The operation journal is recovery-critical and gains nothing from adopting a younger reimplementation.
- Turso's embedded encryption is currently listed as experimental. OpenMango needs application-controlled authenticated encryption for document payloads regardless, so database-engine encryption does not replace the payload design.
- Turso Cloud or embedded-replica sync would create a second remote copy of sensitive history and a network dependency. Reversible history is intentionally local-only.
- `rusqlite` can bundle a known SQLite release and exposes SQLite's backup, hooks, and BLOB support if later required.

Keep the SQL conventional and the store module private so replacing the adapter remains possible if measured local journal contention ever becomes material.

Suggested tables:

```text
operations          current operation projection and parent/revert links
operation_events    append-only lifecycle events
operation_items     per-document/per-metadata outcome and encrypted payload
operation_artifacts snapshot manifests and external encrypted file references
actions             immutable approval proposals and decisions
schema_migrations   local format version
```

The current operation row is the fast query model; `operation_events` is the durable diagnostic trail. This is not full event sourcing: the current row is authoritative, while events explain how it reached that state.

Keep document bodies and identifiers inside authenticated-encrypted item payloads. Store only bounded display metadata, opaque connection identity, status, timestamps, counts, recipe type, hashes, and artifact sizes in queryable columns. Large snapshots remain chunked files referenced by the database.

Use one SQLite transaction for every local transition, including native approval changing an action and creating its operation. Use a single background store worker, WAL mode, `synchronous=FULL`, a bounded busy timeout, foreign keys, startup integrity/version checks, and explicit checkpointing during clean shutdown. All database work runs off the GPUI thread.

### Internal test seam

The engine has one internal `MutationBackend` interface with two adapters:

- production MongoDB adapter;
- deterministic in-memory adapter for state-machine and fault tests.

The store is tested using real temporary SQLite databases rather than a mock. Time, operation IDs, and injected crash points are controlled in tests without appearing in the public interface.

### Views, not duplicate systems

- **Collection History** queries operations for the active connection/database/collection regardless of origin.
- **Agent Activity** queries pending action proposals plus operations whose origin is built-in AI or MCP.
- Database backup/sync/revert cards and document edit rows are projections of the same store.

## One history, several recovery recipes

There is no safe universal inverse for every MongoDB command. OpenMango should project one durable operation store through collection History while selecting one of three internal recovery recipes.

### 1. Document transitions

Use for bounded inserts, replacements, updates, and deletes.

```text
DocumentTransition {
    namespace identity,
    document _id,
    before: Document | Absent,
    after:  Document | Absent,
    before hash,
    after hash
}
```

The same model covers CRUD:

- insert: `Absent -> Document`;
- update/replace: `Document A -> Document B`;
- delete: `Document -> Absent`.

Revert flips the transition, but only when the current document still matches the recorded `after` state. A mismatch is a conflict, never an automatic overwrite.

### 2. Metadata specifications

Use for index and compatible collection metadata operations.

- create index: store the exact resulting name, keys, and options; revert drops it only if its current definition still matches;
- drop index: store the exact index definition; revert recreates it, while reporting uniqueness or data-drift failures;
- prefer hide/unhide over drop where that satisfies the user's intent;
- rename collection: store namespace identities and reverse only if the same collection still occupies the expected target.

### 3. Verified namespace snapshots

Use where per-document inversion is unsafe or impractical.

- collection/database drop;
- database replacement and revert;
- large imports or destructive transfers;
- aggregation `$out`;
- aggregation `$merge` unless a future bounded planner can prove and capture every affected document.

Reuse OpenMango's verified archive backup and restore implementation. Capture collection options, validators, and index definitions in addition to documents. Atlas Search, triggers, users/roles, encryption metadata, and sharding configuration require separate support and must not be advertised as recoverable until verified.

## Operation classification

| Operation | History strategy | Safe revert condition |
| --- | --- | --- |
| Read query / normal aggregation | Query History only | No mutation |
| Insert one/many | Document transitions | Delete only if current document equals recorded post-image |
| Update/replace | Document transitions | Restore before-image only if current document equals recorded post-image |
| Delete one/many | Document transitions | Reinsert only if `_id` is absent |
| Bounded bulk mutation | Per-document transitions | Revert confirmed successes in reverse order; conflicts stay unresolved |
| Create index | Metadata specification | Drop only if current index equals recorded specification |
| Drop index | Metadata specification | Recreate; may fail after data drift |
| Create collection | Metadata specification | Drop only if the same collection remains empty/unchanged |
| Rename collection | Metadata specification | Reverse only when namespace identity still matches |
| Drop collection/database | Verified snapshot | Restore into staging, verify, then cut over |
| `$out` | Verified target snapshot | Restore the previous destination collection |
| `$merge` | Verified target snapshot initially | Restore the complete target; never assume the merge was all-or-nothing |
| Unknown/admin command | Unsupported | No agent execution; manual execution must say it is not reversible |

## Durable execution protocol

MongoDB and a local desktop journal cannot participate in one atomic transaction. OpenMango therefore needs a write-ahead protocol:

1. **Plan** — resolve exact targets, capture before-state and expected after-state, classify the recovery recipe, and enforce size limits.
2. **Prepare** — encrypt and durably persist the recovery payload, then atomically persist operation state `prepared`.
3. **Apply** — execute only if the server's current state still matches the captured precondition.
4. **Record** — persist per-item outcomes and transition to `completed`, `partial`, `failed`, or `uncertain`.
5. **Reconcile** — after restart, compare current state with before/after hashes for every `prepared`, `running`, or `uncertain` operation.
6. **Revert** — create a linked operation with reversed transitions or a restore plan; never mutate the original record.

Recommended operation states:

```text
prepared -> running -> completed
                    -> partial
                    -> failed
                    -> uncertain
completed/partial -> reverting -> reverted
                              -> conflicts
                              -> recovery_required
```

The existing durable action/operation store, atomic file replacement, recovery interlock, target lease, and archive verification should be reused. Approval remains separate: manual tracked edits create operations directly; Arcula-class proposals still require native approval before creating an operation.

## Concurrency rules

MongoDB guarantees single-document atomicity, not whole-operation atomicity for `updateMany` or other multi-document writes. MongoDB recommends including the expected current value in the write filter to prevent lost updates.

OpenMango should therefore:

- never revert by `_id` alone;
- use expected-current-state conditions when applying and reverting;
- treat zero matched documents as conflicts;
- offer `Skip`, `Inspect`, and explicit `Force overwrite` for users;
- never allow agents to force through a conflict;
- use transactions for bounded multi-document work when the deployment supports them, but not depend on transactions because standalone MongoDB does not support them and `$out`/`$merge` cannot run inside them.

`findOneAndUpdate` and `findOneAndDelete` can return affected documents atomically, but they do not atomically persist OpenMango's local journal. They are useful execution primitives, not a complete history design.

## Why change streams and the oplog are not the foundation

Change streams are optional verification and conflict-detection inputs, not the source of truth for history:

- they require a replica set or sharded cluster;
- document pre/post images require MongoDB 6.0+ and per-collection enablement;
- images can expire or disappear with their oplog event;
- `updateLookup` can return a later document version;
- enabling pre/post images adds storage and processing overhead;
- time-series collections do not support normal document change streams.

OpenMango should not alter customer collections to enable pre/post images merely to implement local history. The oplog is rolling replication history and does not provide guaranteed full before-images or indefinite retention.

## Write aggregations

`$out` and `$merge` must be treated differently from read aggregations.

- `$out` builds a temporary collection and atomically renames it over the destination, but MongoDB does not retain the previous destination. OpenMango must snapshot the destination first.
- `$merge` can insert, replace, merge, keep, discard, fail, or run an update pipeline. It can leave earlier writes applied after a later failure, cannot run in a transaction, and can repeatedly update documents when writing into its own source collection. Initial support therefore requires a verified destination snapshot.

Normal read-only aggregation runs stay outside Operation History.

## Storage and privacy

History duplicates potentially sensitive production data. Store metadata and payload separately:

```text
operation.json      metadata, status, counts, hashes, origin, no document bodies
payload.enc         encrypted BSON transitions or recovery manifest
backup/             verified encrypted or access-controlled snapshot artifacts
```

Requirements:

- generate a per-install history key and keep it in macOS Keychain;
- encrypt recovery payloads with authenticated encryption;
- use owner-only directories/files;
- never put document bodies, filters, pipelines, or secrets in audit logs or telemetry;
- expose disk usage, retention age, and byte quota;
- support immediate purge and per-connection disablement;
- never purge unresolved, reverting, or `recovery_required` operations automatically;
- warn that purging payloads removes revert capability while retaining minimal audit metadata.

## Product behavior

Connection settings:

- per-connection **Reversible history** switch, default off;
- future retention period and storage quota;
- future disk usage and **Clear history**.

Collection History:

- a History subview beside Documents, Indexes, Stats, Aggregation, and Schema;
- a collection-scoped timeline for User, built-in AI, and MCP origins;
- operation kind, affected count, status, and expiry;
- before/after preview for document transitions;
- conflict calculation before revert;
- `Revert`, `Inspect conflicts`, and `Remove recovery data` actions;
- database backup/sync/revert operations appear in the same timeline.

Policy:

- manual users may explicitly continue with a non-reversible operation after a warning;
- built-in AI and MCP clients cannot bypass recoverability;
- unbounded or unsupported agent writes remain rejected.

## Delivery order

1. General operation model, encrypted payload store, connection setting, retention, and collection History UI.
2. Manual single-document insert/update/delete through document transitions.
3. Conflict calculation and revert as a new operation.
4. Bounded multi-document edits with per-document checkpoints.
5. Route built-in AI document writes through the same seam.
6. Expose bounded MCP document mutation tools.
7. Add index metadata recipes.
8. Integrate transfers, collection/database operations, `$out`, and `$merge` through verified snapshots.

The first implementation slice was **manual single-document update**. Manual single-document insert and delete now use the same transition model with absent before/after states. Insert revert deletes only an unchanged post-image; delete restore inserts only while the `_id` remains absent. Together they exercise before/after capture, encryption, durable preparation, conditional application, conflict detection, restart reconciliation, History UI, and restore without requiring bulk or snapshot complexity.

## Primary sources

- [Studio 3T: Insert, Update, and Restore MongoDB Documents](https://studio3t.com/knowledge-base/articles/mongodb-documents-beginners-guide/)
- [MongoDB: Atomicity and Transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
- [MongoDB: Transactions](https://www.mongodb.com/docs/manual/core/transactions/)
- [MongoDB: Change Streams](https://www.mongodb.com/docs/manual/changestreams/)
- [MongoDB: `changeStreamOptions`](https://www.mongodb.com/docs/manual/reference/cluster-parameters/changestreamoptions/)
- [MongoDB: `findOneAndUpdate`](https://www.mongodb.com/docs/manual/reference/method/db.collection.findoneandupdate/)
- [MongoDB: `findOneAndDelete`](https://www.mongodb.com/docs/manual/reference/method/db.collection.findoneanddelete/)
- [MongoDB: `$out`](https://www.mongodb.com/docs/manual/reference/operator/aggregation/out/)
- [MongoDB: `$merge`](https://www.mongodb.com/docs/manual/reference/operator/aggregation/merge/)
- [MongoDB: Replica Set Oplog](https://www.mongodb.com/docs/manual/core/replica-set-oplog/)
