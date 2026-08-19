# MCP Direct Writes and Passive History Simplification

## Objective

Implement the approved MCP and History simplification in this repository on branch `feat/embedded-mcp-agent-actions`.

Do not push. Preserve Arcula behavior. Prefer deletion over compatibility layers: the current Reversible History implementation is unreleased and needs no migration support.

## Product decisions

### 1. Arcula operations remain approval-gated

Keep the current Action Broker and native approval workflow for:

- database backup;
- database sync/replacement;
- database operation revert.

Preserve verified backups, target fingerprints, recovery interlocks, progress, cancellation, Agent Activity, and native Production/protected confirmation.

MCP must never approve these operations itself.

### 2. Other MCP writes execute directly

Add a persisted per-connection `agent_writable` setting:

- default `false`;
- independent from `agent_shared`;
- `agent_shared` grants visibility/read access;
- `agent_writable` grants direct typed MCP writes;
- global `read_only` always overrides and blocks writes;
- Reversible History must not be required;
- no Action Broker proposal or native per-operation approval;
- explicit write access may be enabled for protected/Production connections;
- enabling it on protected/Production connections requires a clear one-time UI warning.

Replace MCP document proposal tools with direct tools:

- `openmango_insert_documents`
- `openmango_update_documents`
- `openmango_replace_document`
- `openmango_delete_documents`

Use MongoDB-faithful semantics:

- update supports one or many and normal update documents or supported update pipelines;
- replace affects one document;
- delete supports one or many;
- empty-filter update/delete-many requires explicit `allow_all: true`;
- reject server-side JavaScript such as `$where`, `$function`, and `$accumulator`;
- retain typed Extended JSON parsing, namespace validation, request limits, timeouts, authentication, rate limits, audit logging, and structured write-result counts;
- do not pre-read documents for History;
- attach an `openmango_trace_id` through MongoDB's `comment` option for diagnostics where supported.

Remove:

- MCP `DocumentTransitions` proposals;
- pending History checkpoints for MCP;
- `McpDocumentWriteContext`;
- `ResolveDocumentWrite`;
- `DocumentProposal`;
- `PendingTransitionGuard`;
- MCP document approval/expiry/cancellation branches;
- old `openmango_propose_{insert,replace,delete}_documents` tools and compatibility aliases.

Keep Action Broker action/operation query tools because Arcula workflows still use them.

Do not add speculative MCP mutation categories during this refactor. Establish the direct-write policy seam so future typed writes follow the same authorization model.

### 3. Replace current History completely

Delete the current pre-write recipe-based Reversible History implementation:

- document transition state machine;
- insert History;
- index History;
- collection snapshot/drop History;
- encrypted collection artifacts;
- operation preparation/reconciliation machinery;
- History routing from manual commands, bulk commands, unsaved-save paths, indexes, collections, built-in AI, and MCP;
- old operation kinds, snapshot recipes, UI rows, and tests;
- `src/operations/snapshot.rs` and other code used only by the removed design.

Remove any History requirement from built-in AI writes. Built-in AI may retain its current local confirmation UX, but History must not gate execution.

Reuse crypto/SQLite helpers only where they materially simplify the replacement. Do not preserve the old module merely because code already exists.

No legacy database/schema/enum migration is required. Use a clean new History schema.

## New History contract

History is an optional passive MongoDB change-stream recorder.

It:

- defaults off;
- never gates or blocks writes;
- records writes from every client, including OpenMango UI, MCP, Forge, and external applications;
- cannot reliably attribute every event to an origin;
- is local and encrypted;
- is not a backup, audit/compliance log, or guaranteed complete journal;
- surfaces gaps explicitly;
- initially records only document update, replace, and delete events;
- ignores inserts, indexes, collection/database DDL, and other operations for recovery.

### Eligibility

History is available only when all mandatory server requirements are met:

- MongoDB 6.0 or newer;
- replica-set or sharded topology;
- WiredTiger;
- change streams available;
- document pre/post images enabled for covered regular collections.

For standalone MongoDB, MongoDB before 6.0, unsupported topology/storage, views, or time-series collections, History is unavailable. Do not provide the old fallback.

Detect eligibility using server/topology metadata. Disable the History setting and explain the exact unmet requirement.

Enabling History must:

1. inspect server version and topology;
2. inspect regular collections and pre/post-image configuration;
3. clearly explain that all-client changes are recorded;
4. offer an explicit setup action to enable `changeStreamPreAndPostImages` using `collMod`;
5. report missing `collMod`, `find`, or `changeStream` privileges cleanly;
6. never silently disable pre/post images later because another application may depend on them.

When a new regular collection appears while History is enabled, attempt the already-authorized setup and record a coverage gap if pre/post images cannot be enabled before writes occur.

References:

- <https://www.mongodb.com/docs/manual/changeStreams/>
- <https://www.mongodb.com/docs/manual/reference/command/collMod/#change-streams-with-document-pre--and-post-images>
- <https://www.mongodb.com/docs/manual/reference/change-events/update/>

## Recorder module

Create one deep `history` module with a small interface. Callers must not know about change-stream cursors, resume tokens, batching, encryption, retention, or restore recipes.

Suggested external interface:

- `start(connection)`
- `stop(connection_id)`
- `eligibility(connection) -> eligibility report`
- `list_batches(query) -> page`
- `get_batch(batch_id) -> details`
- `revert_batch(batch_id)`
- `clear(scope)`
- `reconcile/startup resume`

Run MongoDB watching, encryption, and SQLite work off the GPUI thread.

Use database- or deployment-level watchers rather than one watcher per open UI tab.

Request exact images:

- `fullDocumentBeforeChange: whenAvailable`
- `fullDocument: whenAvailable`

Never use `updateLookup` as a recovery post-image because it can return a later concurrent version.

Persist each encrypted event and its resume token atomically. Deduplicate by resume-token hash. Resume after reconnect/restart. If the token or pre-images have expired, persist a durable gap marker and restart only after surfacing that discontinuity.

Support `$changeStreamSplitLargeEvent` where the connected server version supports it. Otherwise convert oversized-event failures into visible gaps rather than silently claiming coverage.

## Batch/change-set model

The History UI must not show one top-level row per changed document.

Use:

- `history_batches` — one displayed change set;
- `history_items` — encrypted per-document recovery items;
- `history_cursors` — resume state;
- `history_gaps` — explicit discontinuities/coverage failures.

A batch contains:

- connection and namespace;
- operation family;
- grouping kind;
- optional local trace ID;
- optional transaction key;
- first/last cluster and wall time;
- item/revertible/conflict counts;
- encrypted-byte usage;
- lifecycle/revert status.

Grouping kinds:

1. `transaction`
   - exact grouping by `lsid + txnNumber`.
2. `attributed`
   - OpenMango-controlled operation with a local trace ID;
   - attach the trace as MongoDB command `comment`;
   - correlate using namespace, operation type, command start/end, and returned affected count;
   - never describe this as guaranteed because command comments are not present in change-stream events.
3. `observed`
   - rolling burst grouping for Forge, external clients, and ordinary non-transactional writes;
   - group adjacent compatible events by namespace and operation family;
   - close after a short idle period, bounded maximum duration, or high item ceiling;
   - label it clearly as an observed change set, not an original MongoDB operation.

Use conservative constants such as:

- 1-second idle close;
- 30-second maximum open duration;
- 100,000-item continuation boundary.

A 10,000-document `updateMany` should normally produce one or a few History batch rows, never 10,000 top-level rows.

Exact per-document before/after images must still be retained internally because conflict-safe restoration is impossible without them. Load items only for detail pagination or restoration.

Do not add trace fields to user documents and do not enable the MongoDB profiler.

## Conflict-safe restore

For update/replace items:

- restore `before` only when the current document exactly equals `after`.

For delete items:

- reinsert `before` only when the document remains absent.

Never force overwrite.

Batch restore:

- runs in the background;
- processes bounded chunks;
- records restored, skipped, conflicted, and failed counts;
- survives cancellation/restart with honest partial state;
- supports progress UI;
- requires native confirmation;
- produces ordinary change-stream events itself.

A batch may contain heuristic grouping, so the UI must show samples, namespace, time range, grouping quality, and item count before confirmation.

## Retention and storage

Retention is mandatory because History observes all clients and may capture large batches.

Implement:

- configurable maximum age;
- configurable maximum encrypted bytes;
- current usage display;
- delete one batch;
- clear one collection;
- clear one connection;
- clear all;
- oldest-first purge;
- no deletion of active restore work;
- explicit handling when storage/encryption fails.

Encrypt pre/post images and sensitive document keys with AES-256-GCM using the existing Keychain-backed installation key pattern. Do not store credentials or connection URIs in History.

If encryption or local persistence fails, stop recording, write/surface a persistent gap state, and never block the originating MongoDB write.

## UI

Connection settings:

- **Share with agents**
- **Allow agent writes** — visible only when shared, default off
- **History** — default off and disabled with a reason when ineligible
- eligibility/setup/coverage state
- age/size retention controls and current usage

Collection History:

- hide when History is disabled or unavailable;
- show paginated batch rows;
- display count, operation family, time range, grouping badge, revertible/conflict totals, and samples;
- expand/paginate document items only on demand;
- display gap markers prominently;
- never claim complete coverage while a gap or uncovered collection exists.

Use wording similar to:

> History records supported changes observed by OpenMango on this device. It may include writes from other clients and can contain gaps. It is not a backup or audit log.

## Preserve unchanged

Do not regress:

- Arcula database backup/sync/revert;
- Action Broker approval for those operations;
- verified backups and recovery;
- connection authentication and secret handling;
- MCP loopback/token security;
- read-only MCP tools;
- MCP request auditing;
- manual Production confirmation outside direct MCP authority;
- existing database sync Agent Activity.

## Documentation

Rewrite:

- `docs/REVERSIBLE_HISTORY_SPEC.md`
- `docs/MCP_SERVER_SPEC.md`

Remove old promises about verified recovery data existing before every supported write.

Document:

- direct MCP write authority;
- Arcula-only approval gating;
- change-stream eligibility;
- all-client capture;
- grouping quality;
- gaps and retention;
- unsupported deployments and targets;
- distinction between History and backup/sync.

## Tests and acceptance

Add focused unit tests for:

- `agent_writable` defaults false and survives persistence;
- shared-but-not-writable denial;
- global read-only denial;
- direct MCP write success without History;
- direct update-many affecting more than 100 documents;
- `allow_all` protection;
- Arcula proposals still requiring native approval;
- topology/version eligibility;
- pre/post-image coverage checks;
- resume-token deduplication and atomic persistence;
- missing/expired resume-token gap handling;
- transaction grouping;
- attributed grouping;
- observed burst grouping;
- a simulated 10,000-event update producing bounded batch rows;
- encrypted payload authentication/tamper failure;
- retention by age and bytes;
- update/delete conflict-safe restore;
- partial batch restore outcomes;
- no forced overwrite.

Add MongoDB replica-set integration tests for:

- `updateMany` pre/post-image capture;
- delete pre-image capture;
- restart/resume;
- change-stream History observing direct MCP and Forge-equivalent writes;
- exact transaction grouping;
- restoration conflicts.

Run:

- `cargo fmt --all`
- `just fmt-check`
- `just lint`
- `just check`
- focused History/MCP tests
- full test suite with the existing `/usr/bin/ld` workaround if required
- `git diff --check`

Perform an independent security/correctness review focusing on:

- authority defaults;
- resume gaps;
- accidental overwrite;
- batching misrepresentation;
- Arcula regressions.

Commit the completed refactor with a short imperative subject. Do not push.

## Completion criteria

The work is complete only when:

- old recipe-based History code and call-site instrumentation are removed;
- old MCP document proposals are removed;
- direct MCP writes require explicit `agent_writable` and never require History;
- Arcula operations still require native approval;
- ineligible MongoDB deployments have no History;
- eligible deployments record passive encrypted update/replace/delete events;
- 10,000+ changes are represented as batches, not top-level per-document rows;
- exact versus attributed versus observed grouping is honest in the UI;
- gaps are durable and visible;
- restore never overwrites conflicts;
- retention prevents unbounded local growth;
- tests, formatting, lint, and full validation pass.
