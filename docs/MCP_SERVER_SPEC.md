# OpenMango MCP and Agent Actions — Developer Specification

> **Version:** 1.0
> **Date:** August 2026
> **Status:** Ready for implementation
> **Protocol baseline:** MCP `2026-07-28`
> **Repository:** https://github.com/ggagosh/openmango

---

## Table of contents

1. [Decision](#1-decision)
2. [Goals and non-goals](#2-goals-and-non-goals)
3. [Domain model](#3-domain-model)
4. [Connection authority and policy](#4-connection-authority-and-policy)
5. [Architecture](#5-architecture)
6. [MCP transport and client access](#6-mcp-transport-and-client-access)
7. [MCP tool surface](#7-mcp-tool-surface)
8. [Data contracts and untrusted content](#8-data-contracts-and-untrusted-content)
9. [Proposed actions and approval](#9-proposed-actions-and-approval)
10. [Database backup, sync, and revert](#10-database-backup-sync-and-revert)
11. [Persistence, audit, and redaction](#11-persistence-audit-and-redaction)
12. [Limits, cancellation, and concurrency](#12-limits-cancellation-and-concurrency)
13. [User interface](#13-user-interface)
14. [Implementation modules](#14-implementation-modules)
15. [Delivery phases](#15-delivery-phases)
16. [Testing and acceptance](#16-testing-and-acceptance)
17. [Future work](#17-future-work)
18. [Reviewed primary sources](#18-reviewed-primary-sources)

---

## 1. Decision

OpenMango will ship an **embedded, disabled-by-default MCP server** and a shared **Action Broker** for agent-originated mutations.

The MCP server:

- runs only while OpenMango is running;
- uses MCP Streamable HTTP on a literal loopback address;
- exposes only connections explicitly shared by the user;
- uses opaque connection UUIDs, never connection strings;
- reuses OpenMango-owned MongoDB clients, credentials, SSH tunnels, and proxy transports;
- executes bounded read operations immediately when policy permits;
- represents every database or filesystem mutation as an immutable proposed action;
- never accepts approval through MCP;
- lets the user approve, reject, cancel, inspect, and revert actions in OpenMango's native UI.

Arcula's safe database-sync behavior will be merged into OpenMango as domain logic, not embedded as a CLI or independent connection manager. OpenMango will keep Arcula's plan hashing, backup-before-mutation, operation records, recovery, and revert concepts while reusing OpenMango's existing secure BSON tool runner, connections, keychain integration, cancellation, and progress UI.

The first implementation assumes OpenMango remains open. A headless or remote server is a separate product and security architecture.

### 1.1 Core invariant

> MCP can request work, but only OpenMango policy and the local user can grant authority.

Tool annotations, prompts, acknowledgement booleans, model compliance, MCP elicitation, and possession of the local HTTP token are not database authorization.

---

## 2. Goals and non-goals

### 2.1 Goals

1. Let agents discover and use user-approved OpenMango connections without receiving credentials.
2. Respect user-owned connection sharing, environment, protected, and read-only flags.
3. Provide bounded MongoDB metadata and read tools with structured Extended JSON output.
4. Give agents a safe way to propose database backups, environment syncs, and reverts.
5. Give users one native interface for reviewing agent activity and approving exact actions.
6. Reuse the same Action Broker for MCP clients and OpenMango's built-in AI assistant.
7. Preserve durable action and operation history sufficient for review and recovery.
8. Fail closed when identity, policy, approval, cancellation, or recovery state is uncertain.

### 2.2 Non-goals for the initial release

- A server that works while OpenMango is closed.
- Remote network exposure, hosted MCP, OAuth, TLS termination, or multi-tenancy.
- Independent MCP credentials, MongoDB pools, tunnels, or connection configuration.
- Raw connection strings, keychain identifiers, certificate data, SSH/proxy configuration, or effective tool URIs.
- Agent control over sharing, environment, protected, or read-only flags.
- Agent-supplied connection credentials.
- Arbitrary `runCommand`, mongosh/Forge JavaScript, shell execution, process execution, or filesystem paths.
- Direct CRUD/index write tools in the first release.
- Transfers unrelated to database backup/sync/revert.
- Query history, unsaved editors, clipboard data, support logs, or AI provider keys.
- Resources and prompts until target-client testing demonstrates value.
- MCP Tasks as a required capability; Tasks remain experimental.

---

## 3. Domain model

Use these terms consistently in code, UI, tests, and documentation.

### Connection

An OpenMango-saved MongoDB connection identified by a UUID. OpenMango owns its credentials and runtime transport.

### Agent sharing

A user-owned boolean granting agents visibility of a connection. Sharing does not grant write authority and defaults to disabled.

### Protected connection

A connection requiring the strongest mutation safeguards. Production is always effectively protected; users may also protect non-production connections.

### Effective policy

The capabilities derived from current application state, client access, agent sharing, connection flags, requested operation, and MongoDB RBAC.

### Proposed action

An immutable, durable request for a future side effect. Creation performs validation and preview only; it does not mutate MongoDB or write backup data.

### Approval

A one-shot local-user decision bound to one exact proposed-action hash and current preconditions. Approval is not reusable authorization.

### Operation

A durable execution record created after approval. It tracks phases, progress, cancellation, result, recovery, and any retained backup.

### Backup

An app-managed MongoDB dump plus a verification manifest. MCP never chooses its filesystem path.

### Database sync

A controlled source-database dump followed by target replacement, protected by a verified target backup and recoverable operation record.

### Revert

A new proposed action that restores the target from a backup belonging to a completed or interrupted operation. Revert is itself destructive and requires approval.

---

## 4. Connection authority and policy

### 4.1 Persisted connection fields

Reuse existing `SavedConnection` fields:

- `id`
- `name`
- `environment`
- `read_only`
- connection and transport configuration

Add only:

```rust
pub agent_shared: bool, // serde default false
pub protected: bool,    // serde default false; Production is protected regardless
```

Do not port Arcula's second connection registry or granular policy booleans. The fixed policy below is smaller, easier to explain, and cannot be weakened accidentally.

### 4.2 User-owned controls

Only the user may change:

- agent sharing;
- environment;
- protected status;
- read-only status;
- connection credentials or transport configuration.

No MCP tool or built-in agent tool may change these fields. Connection creation/editing may later be initiated by an agent only by opening a user-owned form; credentials still never cross MCP.

Changing a connection to Production or protected automatically disables `agent_shared`. The user must explicitly share it again after seeing the warning. Any policy, credential, endpoint, transport, or identity change makes pending actions involving that connection stale.

### 4.3 Capability matrix

| Current connection state | Agent capability |
| --- | --- |
| Not shared | Invisible and unusable. |
| Shared, disconnected | Listed as disconnected; may be tested or connected. |
| Shared, connected, read-only | Metadata/reads; database backup source; sync source. Never a mutation target. |
| Shared, connected, writable, non-protected | Reads and proposed target mutations. Every mutation still requires UI approval. |
| Shared, connected, protected or Production | Reads and proposed target mutations with high-risk UI treatment. Verified backup is mandatory. |

A Production connection is not automatically read-only. `read_only` is the hard write prohibition; Production/protected is a safety floor around writes that are otherwise allowed.

### 4.4 Effective policy formula

Every request evaluates:

```text
mcp_enabled
∩ client_grant_active
∩ connection_agent_shared
∩ connection_exists
∩ required_runtime_state
∩ tool_class_allowed
∩ connection_not_read_only_for_target_mutations
∩ protected/production_safety_floor
∩ no_target_recovery_interlock
∩ MongoDB_RBAC
```

Evaluation occurs:

1. when advertising dynamic tool availability;
2. when the tool is invoked;
3. when the user opens an approval;
4. immediately before the first side effect;
5. before each recovery or revert side effect.

A cached MCP session never retains connection authority.

### 4.5 Layered read-only enforcement

Read-only is enforced through all applicable layers:

1. omit mutation proposal tools when no shared writable target exists;
2. reject a read-only target during proposal validation;
3. reject again in the Action Broker at approval and execution;
4. structurally reject aggregation write stages from read tools;
5. rely on least-privilege MongoDB users as the final boundary.

MCP `readOnlyHint` and `destructiveHint` must be accurate but remain UX hints only.

---

## 5. Architecture

### 5.1 Process shape

```text
MCP client
    │ Streamable HTTP: 127.0.0.1:<persisted-port>/mcp
    │ Authorization: Bearer <client grant token>
    ▼
HTTP security middleware
    │ body/Host/Origin/auth/rate/concurrency checks
    ▼
rmcp handler
    │ typed and bounded tool arguments
    ▼
MCP ↔ GPUI bridge
    │ re-resolve client grant, connection, identity, policy, runtime state
    ▼
Read executor OR Action Broker
    │
    ├── bounded async MongoDB reads
    │
    └── durable proposal → native approval → Sync Executor
                                      │
                                      ▼
                       ConnectionManager / connection::ops
                       active MongoDB clients and tunnels
```

### 5.2 Ownership

- `AppState` owns saved connections, active connections, settings, query library, and action UI state.
- `ConnectionManager` owns the Tokio runtime, MongoDB runtime resources, and SSH tunnel lifecycle.
- `McpController` owns listener configuration, cancellation, and loaded client grants.
- `PolicyEvaluator` derives capabilities from live state.
- `ActionBroker` owns proposal state transitions and approval checks.
- `SyncExecutor` owns backup/sync/revert orchestration, not policy decisions.
- `AgentStore` atomically persists proposed actions and operations.

### 5.3 GPUI bridge

An MCP task sends a bounded request to GPUI only to resolve live application state. GPUI returns a sanitized runtime snapshot containing only what execution needs, such as:

- cloned `mongodb::Client`;
- `Arc<ConnectionManager>` or its runtime handle;
- opaque connection UUID;
- sanitized identity fingerprint;
- derived policy and policy version;
- an operation lease when required.

Database work never runs on the GPUI thread.

### 5.4 Tokio rule

Current `ConnectionManager` methods commonly call `Runtime::block_on`. Calling those wrappers from an MCP Tokio task can panic due to nested runtime execution.

Before MCP read tools are added:

- extract reusable async read functions under `connection::ops`;
- let MCP await those functions directly;
- keep thin synchronous wrappers only for existing GPUI background callers that require them;
- never call a `block_on` wrapper from an MCP or Sync Executor Tokio task.

The MCP listener should run on the existing `ConnectionManager` runtime unless the spike proves isolation is necessary. Do not add another runtime by default.

### 5.5 Connection leases

A running operation acquires leases for its source and target connections. While leased:

- disconnect, removal, credential edits, and transport edits are blocked or require cancelling the operation first;
- tunnels remain alive;
- read-only/environment/protected/sharing changes invalidate execution before its next side effect;
- application shutdown requests cooperative cancellation and recovery before tunnel teardown.

---

## 6. MCP transport and client access

### 6.1 SDK

Use the official Rust SDK:

```toml
rmcp = { version = "=3.1.2", default-features = false, features = [
  "server",
  "macros",
  "transport-streamable-http-server",
] }
```

Add Axum/Tower dependencies only where the server composition requires direct use. Commit `Cargo.lock`. Upgrade `rmcp` deliberately with protocol and target-client regression tests; never depend on the SDK `main` branch.

### 6.2 Listener lifecycle

1. MCP starts disabled.
2. On first enable, bind `127.0.0.1:0`, persist the assigned non-secret port, then reuse it on later launches.
3. If the persisted port is unavailable, do not silently expose another endpoint. Bind a new loopback port, persist it, invalidate displayed client configuration, and notify the user.
4. Start only after connection credential hydration and client-grant loading succeed.
5. Show a persistent “MCP enabled” indicator and active-call count.
6. Stop accepting requests before cancelling operations or destroying active connections/tunnels.
7. Rotate/revoke grants when MCP is disabled only if the user chooses “Disable and revoke”; ordinary disable preserves grants.

IPv6 `::1` is excluded until separately tested. Never bind `0.0.0.0` or a hostname.

### 6.3 HTTP checks

Every request must:

- target `/mcp`;
- have `Host` exactly matching `127.0.0.1:<port>`;
- have no `Origin` header in v1;
- pass `Authorization: Bearer <token>`;
- use an allowed method and content type;
- fit the request-byte ceiling;
- pass rate and concurrency checks.

CORS is disabled. If a required target client supplies `Origin`, add only an exact, user-visible allow-list after Phase 0 validation; never reflect arbitrary origins.

Compare bearer tokens in constant time. Never place tokens in URLs, query strings, logs, error text, settings JSON, or copied support bundles.

### 6.4 Client grants

A user creates an MCP client grant in Settings. A grant contains:

- opaque grant UUID;
- user-visible label;
- random 256-bit token stored in the platform keychain;
- created, last-used, and revoked timestamps;
- optional observed MCP `clientInfo` for display only.

Possession of a token authenticates one local grant. Client-provided `clientInfo` is not trusted identity. Users can revoke one grant or reset all access without changing database credentials.

The copied client configuration uses `127.0.0.1`, the current port, `/mcp`, and the bearer header. Raw tokens are displayed only during explicit copy/reveal actions.

The first supported real client is **Pi with `pi-mcp-adapter`**, after MCP Inspector. A development configuration uses Pi's proxy mode so tool definitions remain lazy:

```json
{
  "mcpServers": {
    "openmango": {
      "url": "http://127.0.0.1:<port>/mcp",
      "auth": "bearer",
      "bearerTokenEnv": "OPENMANGO_MCP_TOKEN",
      "protocolVersion": "2026-07-28",
      "lifecycle": "lazy",
      "requestTimeoutMs": 35000,
      "directTools": false
    }
  }
}
```

The token is supplied through the environment or another adapter-supported secret command and is never committed to MCP configuration. After reloading Pi, validation uses the adapter's `connect`, `search`, `describe`, and `tool` calls. Pi's elicitation and tool-approval support may be tested for compatibility, but neither is accepted as OpenMango action approval.

Settings also exports client-native configurations for the most important authenticated Streamable HTTP clients:

- **Claude Code** uses `headersHelper` to read the grant directly from macOS Keychain;
- **Codex / ChatGPT** uses `bearer_token_env_var` in `config.toml`;
- **Cursor** uses an `Authorization` header sourced from `OPENMANGO_MCP_TOKEN`;
- **VS Code / Copilot** uses a password input variable that VS Code stores securely after prompting.

Each exported profile receives its own revocable grant. Profiles never embed the token. Clients that cannot read macOS Keychain directly require the user to invoke the explicit **Copy token** action and supply it through the client's documented secret mechanism.

### 6.5 Remote mode

A future remotely reachable server must replace—not extend—the local-token design with TLS, OAuth 2.1, protected-resource metadata, audience/resource validation, rate limiting, tenancy, and a non-desktop approval model. Remote writes are not inherited from local mode.

---

## 7. MCP tool surface

All tool names are stable and prefixed with `openmango_`. Connections remain arguments; never encode connection names in tool names. Tools are grouped by user journey and may be omitted as a group when no current policy can authorize them. Invocation still re-checks policy.

### 7.1 Connection tools

| Tool | Behavior |
| --- | --- |
| `openmango_list_connections` | List explicitly shared connections with opaque ID, name, environment, protected/read-only/connected state, and a redacted endpoint label. Never return URI, username, secret ID, SSH/proxy details, or certificate paths. |
| `openmango_test_connection` | Test one shared saved connection with bounded timeout. It may create a temporary transport but performs no database write. |
| `openmango_connect_connection` | Connect one shared saved connection through `ConnectionManager`. Return current status; never return effective URI. |
| `openmango_get_connection_status` | Return connected/connecting/error state and safe identity metadata. |

Disconnect is omitted initially because it can destroy user workspace/runtime state.

### 7.2 Read tools

Every read tool requires an explicit `connection_id`. Database/collection tools also require explicit namespace arguments and never use the active tab.

| Tool | Behavior |
| --- | --- |
| `openmango_list_databases` | Bounded list of databases for one shared active connection. |
| `openmango_list_collections` | Bounded collection names/specifications for one database. |
| `openmango_inspect_collection` | Bounded safe stats, index definitions, and sampled schema summary for one collection. Sampling is clearly identified. |
| `openmango_find_documents` | Strict Extended JSON filter/projection/sort; 20 default and 100 maximum documents; pagination/truncation metadata. |
| `openmango_count_documents` | Exact filtered count under `maxTimeMS`; no estimated count when a filter is supplied. |
| `openmango_aggregate` | Read-only pipeline with structural validation, stage/result/time limits, and no command passthrough. |
| `openmango_explain_query` | `queryPlanner` explain for a typed find or aggregation request; bounded output. |

Read tools reject server-side JavaScript constructs such as `$where`, `$function`, and `$accumulator`. Aggregation recursively rejects `$out`, `$merge`, `$changeStream`, and administrative stages. Validation is structural BSON/JSON traversal, not regex matching. Database RBAC remains the final enforcement boundary.

### 7.3 Saved-query tools

| Tool | Behavior |
| --- | --- |
| `openmango_search_saved_queries` | Search user-curated saved queries visible to an explicit shared connection. Return global entries and entries scoped to that connection only. |
| `openmango_get_saved_query` | Return one visible saved definition. Never execute it. Forge content is inert untrusted text. |

Never expose query history. A global saved query must not leak the connection UUID with which it was originally created.

### 7.4 Action and operation tools

| Tool | Behavior |
| --- | --- |
| `openmango_propose_database_backup` | Validate and persist a pending backup action using an app-managed destination. No backup starts. |
| `openmango_propose_database_sync` | Validate source/target/database replacement, gather preview data, hash the exact request, and persist it pending approval. No dump or mutation starts. |
| `openmango_propose_operation_revert` | Create a pending revert action referencing an eligible operation and retained verified backup. Never accepts an arbitrary path. |
| `openmango_get_action` | Return one action's current status, preview, staleness, approval, and operation ID when present. |
| `openmango_list_actions` | Return a bounded page of actions visible to the authenticated client grant. |
| `openmango_get_operation` | Return durable phase/progress/result/recovery status. |
| `openmango_cancel_operation` | Request cooperative cancellation of an operation created by the same client grant. Cancellation remains in audit history and may trigger rollback. |

Proposal tools automatically surface the pending action in OpenMango's Agent Activity UI. **There is no MCP approve or execute tool.** The user's “Approve & Run” action creates and starts the operation.

Identical pending proposals from the same client are deduplicated by canonical hash and return the existing action ID.

### 7.5 Tools explicitly excluded

- combined `manage_database(action=...)` or arbitrary operation unions;
- `runCommand`;
- raw shell, filesystem, mongosh, or Forge execution;
- arbitrary backup/import paths;
- direct insert/update/delete/index writes in the initial release;
- connection credential import/export;
- users, roles, profiling, server configuration, shutdown, and transactions;
- documents as freely readable MCP Resources.

---

## 8. Data contracts and untrusted content

### 8.1 Inputs

- Use typed Rust argument structures and generated JSON Schema.
- BSON-bearing fields accept JSON values, not shell-syntax strings.
- Support MongoDB Extended JSON deliberately and reject unsupported forms with stable errors.
- Reject unknown fields where practical.
- Validate names, lengths, nesting depth, arrays, pipeline stage count, numeric ranges, and mutually exclusive options before reaching MongoDB.

### 8.2 Outputs

Every tool declares `outputSchema` and returns `structuredContent`. Text content exists only for clients requiring compatibility.

Read responses include:

```json
{
  "data_classification": "untrusted_database_content",
  "applied_limits": {
    "documents": 20,
    "bytes": 262144,
    "max_time_ms": 30000
  },
  "truncated": false,
  "has_more": false,
  "next_offset": null,
  "data": {}
}
```

Never silently truncate. If a single BSON value cannot fit, return a bounded error with safe size metadata rather than malformed or partial JSON.

### 8.3 Extended JSON

Use a documented Extended JSON mode consistently. Canonical Extended JSON is preferred for MCP because it preserves BSON types. If relaxed values are also provided, label the mode in the response.

### 8.4 Prompt-injection boundary

Treat all database-derived values as untrusted, including:

- document fields and values;
- database, collection, and index names;
- saved-query names, descriptions, tags, and content;
- MongoDB and external-tool errors.

Structured content separates trusted envelope metadata from untrusted values. Compatibility text wraps untrusted data in per-response randomized delimiters and explicitly says that embedded instructions are data. This is defense in depth, not an injection guarantee.

No database-returned content can authorize, approve, or silently parameterize a mutation. The approval UI renders normalized action values as plain data, not Markdown or executable links.

---

## 9. Proposed actions and approval

### 9.1 Proposed-action record

```text
ProposedAction {
  version,
  id,
  kind: database_backup | database_sync | operation_revert,
  origin: { kind: mcp | built_in_ai | user, client_grant_id?, session_id? },
  source_connection_id?,
  target_connection_id,
  source_database?,
  target_database,
  normalized_parameters,
  content_hash,
  source_identity_hash?,
  target_identity_hash,
  policy_snapshot,
  policy_version,
  target_state_fingerprint?,
  preview,
  prerequisites,
  created_at,
  expires_at,
  status,
  decision?,
  operation_id?
}
```

The exact persisted model uses typed variants rather than a free-form action map.

### 9.2 Canonical hashing

Compute SHA-256 over canonical, versioned data with recursively sorted object keys. The hash includes:

- action kind and normalized arguments;
- origin client grant;
- source/target UUIDs;
- sanitized connection identity hashes;
- environment, protected, read-only, and sharing policy snapshot;
- source/target databases;
- target replacement semantics;
- backup/recovery requirements;
- target-state fingerprint when available.

Status, timestamps, preview prose, and progress do not alter the approved payload hash.

### 9.3 Status model

Proposed actions use:

```text
pending_approval | rejected | expired | stale | accepted
```

An accepted action points to one operation. It cannot be approved twice or edited. Changed inputs require a new action.

Operations use:

```text
queued | running | cancel_requested | completed | failed | cancelled |
interrupted | recovery_required
```

### 9.4 Approval contract

Approval occurs only in OpenMango's native UI. MCP elicitation may notify a capable client but never authorizes execution.

The UI displays:

- requesting client and session;
- exact source and target connection identities;
- environment, protected, read-only, and sharing state;
- exact databases and operation semantics;
- estimated scope and bytes, clearly marked as estimates;
- target-state change warnings;
- backup and rollback behavior;
- action hash suffix and expiry;
- consequences of cancellation and failure.

For protected or Production targets, the user must type the target database name before “Approve & Run” enables. Non-protected actions require an explicit button click.

Approval is one-shot, expires with the proposal, and binds the exact hash. Missing UI, unsupported elicitation, timeout, cancellation, app shutdown, stale identity, changed policy, changed target fingerprint, or unavailable recovery prerequisites means no execution.

### 9.5 Execution adjacency and races

On “Approve & Run” the broker:

1. reloads the durable action;
2. verifies the hash;
3. re-resolves both connections and effective policy;
4. refreshes target preconditions;
5. acquires connection and target mutation leases;
6. creates the operation record atomically;
7. marks the action accepted;
8. starts execution.

Any mismatch marks the action stale and asks the agent to create a new proposal. Approval never turns into a reusable Production authorization.

### 9.6 Built-in AI

OpenMango's built-in AI assistant must ultimately use the same Action Broker for writes. Existing direct tool-confirmation flows may remain during earlier phases but must not be reused by MCP or Arcula workflows. The final migration removes parallel safety semantics.

---

## 10. Database backup, sync, and revert

### 10.1 Merge strategy for Arcula

Port the relevant behavior from Arcula revision `ae575caa946b7088769cf94193da637e5d8c8753` into OpenMango modules. Do not add Arcula as a path/git dependency and do not copy its CLI layer.

Keep:

- normalized, hash-bound plans;
- protected-target safety floors;
- durable operation records;
- backup-before-mutation;
- backup verification;
- automatic recovery after failed restore;
- explicit revert workflow.

Replace:

- Arcula connection metadata with `SavedConnection` UUIDs and OpenMango policy;
- Arcula keyring vault with OpenMango keychain-managed credentials;
- environment-name lookup with explicit connection UUIDs;
- sudo/polkit approval with native GPUI approval;
- Arcula process execution with `src/connection/ops/bson_tools.rs`;
- console progress/output with operation events and GPUI rendering;
- command-line URIs with just-in-time effective active-connection configuration.

Do not port `clap`, `inquire`, `colored`, `indicatif`, `.env` loading, direct URI arguments, or CLI command modules. If substantial MIT-licensed source is copied rather than independently adapted, retain its copyright/license notice in the repository's third-party notices.

### 10.2 Initial sync semantics

The initial sync operation supports one deliberately narrow mode:

> `replace`: replace one target database with a dump of one source database.

`mode` is an explicit enum in the contract and defaults to `replace`; `replace` is the only accepted v1 value. It means a real database replacement: after a verified target backup and validated source dump, OpenMango drops the target database, restores the source dump under the target database name, and verifies the namespace inventory. It must not rely on `mongorestore --drop` alone because that can leave target-only collections.

The tool does not expose `create_backup`, `drop_collections`, or `clear_collections` booleans. Backup, cleanup, and verification are server-owned safety policy. It also does not expose clear, merge, arbitrary namespace mappings, collection subsets, or filesystem destinations. Source and target connection/database pairs must differ.

Every agent-originated sync requires a verified target backup, including Development. This makes failure and cancellation behavior consistent. If the target database does not exist, persist a verified absence marker; reverting that operation removes the newly created target database after approval.

A later release may add `clear_and_import`, merge, or collection-level modes as separate explicit actions only after their recovery and verification semantics are defined.

### 10.3 Sync preflight

Before proposal:

- both connections are shared and active;
- source is allowed even if read-only;
- target is writable;
- no recovery interlock exists for the target;
- database names are valid;
- MongoDB Database Tools are available;
- source and target are reachable;
- rough source/target size and collection metadata are collected when available;
- app-managed backup storage is available;
- estimated free space is checked; known insufficiency blocks the proposal and unknown capacity is a warning.

Before execution, repeat all authority and availability checks.

### 10.4 Sync phases

```text
preparing
→ dumping_source
→ validating_source_dump
→ checking_target_precondition
→ backing_up_target
→ verifying_target_backup
→ replacing_target
→ verifying_target
→ completed
```

If failure or cancellation occurs after target mutation begins:

```text
restoring_target_backup
→ verifying_recovery
→ failed | cancelled
```

If recovery fails, mark `recovery_required`, retain all artifacts, block further agent mutations to that target database, and present manual recovery guidance.

### 10.5 Secure process execution

Reuse and deepen OpenMango's BSON runner:

- locate bundled or installed `mongodump`/`mongorestore` reproducibly;
- place credentials in owner-only temporary config files, never command arguments;
- use active transport effective URIs only in memory/temporary files;
- pipe and bound stderr/stdout;
- sanitize all external-tool output before logs, records, or UI;
- parse progress without depending on it for correctness;
- kill and wait for child processes on cancellation;
- stage dumps before promotion;
- use single-file MongoDB archives for managed backups/source dumps so case-distinct namespaces remain representable on case-insensitive filesystems;
- clean temporary source dumps after terminal success/recovery;
- retain backups according to retention policy.

### 10.6 Backup verification

Directory existence alone is not verification. A backup manifest records:

- backup ID and format version;
- safe source connection identity hash;
- database name;
- start/completion timestamps;
- MongoDB Database Tools version;
- successful process status;
- preflight collection/view inventory;
- archive file inventory, count, and bytes;
- exact, case-sensitive namespace coverage validated with a non-mutating `mongorestore --dryRun`;
- whether the database was legitimately absent/empty;
- verification result and warnings.

Verification requires successful `mongodump` completion and dump artifacts consistent with preflight inventory. Empty databases are valid only when preflight and manifest explicitly agree.

### 10.7 Target verification

After restore:

- require successful `mongorestore` completion;
- reconnect/list target collections;
- compare expected namespace inventory;
- record count/index differences as warnings or failures according to what the tools can guarantee;
- never claim transactional or globally consistent sync.

MongoDB may change externally during dump/restore. The UI states that the source dump reflects live data during the dump and is not a cross-collection transaction unless a future implementation provides stronger MongoDB-native snapshot semantics.

### 10.8 Revert

Revert accepts only a completed/interrupted operation ID with a retained verified backup or absence marker. It creates a new proposal and operation; it never reuses the original approval.

Before reverting, OpenMango first creates and verifies a safety backup of the target's current state (or an absence marker), making the revert operation itself revertible. For the original operation's normal backup, revert replaces the target from that backup. For an original absence marker, revert drops the newly created target database. Both paths re-check current identity/policy, require native approval, and acquire the target mutation lease.

### 10.9 Crash and shutdown recovery

- OpenMango prompts before quitting while an operation is running.
- Normal quit requests cancellation, waits for child termination, and performs required rollback before tunnel shutdown.
- Forced quit warns that target recovery may be required.
- On startup, records left `running` become `interrupted`; OpenMango never automatically retries.
- If target mutation may have begun, set the target recovery interlock and offer a reviewed revert/reconcile action.

---

## 11. Persistence, audit, and redaction

### 11.1 Storage layout

Use app-owned data storage, separate from ordinary settings:

```text
agent/
  audit.jsonl
  actions/<action-id>.json
  operations/<operation-id>.json
  backups/<backup-id>/manifest.json
  backups/<backup-id>/...
```

Writes are atomic. Directories and sensitive files use owner-only permissions where supported. Action and operation IDs are unguessable UUIDs.

### 11.2 Durable exact records versus audit

Exact normalized action arguments must be persisted so the UI can review and execute what was approved. They live only in protected action records.

The general application log and metadata audit never contain full action payloads, filters, pipelines, document bodies, credentials, URIs, tunnel details, or raw external-tool output.

### 11.3 Audit allow-list

An audit event may contain only:

- timestamp and correlation ID;
- MCP grant UUID and observed client label;
- session ID;
- tool name and operation class;
- action/operation IDs;
- opaque connection UUIDs and safe display names;
- environment/protected/read-only state;
- policy version and decision class;
- duration, document count, result bytes, phase, and public error code;
- approval/rejection actor and timestamp;
- cancellation and recovery status.

Remote telemetry export is opt-in and non-blocking. Document content is never telemetry.

### 11.4 Central redaction

All protocol errors, logs, audit summaries, operation errors, support bundles, and telemetry pass through one redaction module. Tests cover:

- URI usernames/passwords and query credentials;
- bearer tokens and keychain identifiers;
- SSH/proxy credentials and endpoints;
- certificate/key paths and content;
- temporary config paths where sensitive;
- nested MongoDB driver errors;
- `mongodump`/`mongorestore` command output.

Client-facing failures use stable public codes and safe remediation text. Detailed local diagnostic chains remain redacted.

### 11.5 Retention

Defaults:

- rejected/expired actions: 30 days;
- operation metadata: 180 days;
- successful backups: 30 days;
- failed/interrupted/recovery-required artifacts: never auto-delete until resolved or explicitly removed by the user.

Retention runs locally and records deletion metadata. The UI warns before deleting the last usable revert backup. MCP cannot delete audit or backup data in v1.

---

## 12. Limits, cancellation, and concurrency

Initial hard ceilings:

| Limit | Value |
| --- | ---: |
| HTTP request body | 64 KiB |
| Structured tool result | 256 KiB |
| JSON/BSON nesting depth | 64 |
| Metadata items per response | 500 |
| Find default / maximum documents | 20 / 100 |
| Find maximum skip | 10,000 |
| Aggregation stages | 20 |
| Aggregation returned documents | 100 |
| Schema sample | 100 documents |
| Read `maxTimeMS` / wall timeout | 30,000 ms / 35 s |
| Connection test timeout | 15 s |
| Requests per client grant | 10 requests/second |
| Global concurrent MCP calls | 4 |
| Concurrent calls per client grant | 2 |
| Concurrent mutation per target database | 1 |
| Action/list page size | 50 default, 100 maximum |
| Operation polling | 2 requests/second/client |
| Pending-action expiry | 1 hour |

There is no unbounded request queue. Excess calls receive a retryable busy error.

Limits are server ceilings independent of model arguments. User settings may lower them but cannot raise them in v1. Phase 0/1 measurement may lower a number before release; any change must update this specification and tests.

### 12.1 Cancellation

Transport cancellation propagates through the bridge to MongoDB reads and cursor collection. On cancellation, a non-durable read sends no late result.

Durable operation cancellation:

- transitions the operation to `cancel_requested`;
- terminates/waits for external processes;
- stops before target mutation when possible;
- performs rollback if target mutation may have started;
- retains its audit and terminal result;
- tolerates cancellation/completion races idempotently.

Cancellation does not guarantee MongoDB reversed a command already committed. Reconciliation and recovery state must represent uncertain outcomes explicitly.

---

## 13. User interface

### 13.1 Settings: MCP & Agents

Add a Settings subtab containing:

- MCP enable/disable;
- endpoint and health status;
- current port;
- create/copy/revoke client grants;
- reset all access;
- active client/session list;
- per-connection sharing controls;
- safe connection identity, environment, protected, read-only, and connected state;
- explicit warning/reconfirmation when sharing Production/protected connections;
- backup root and retention summary;
- links to Agent Activity and diagnostics.

Environment, protected, and read-only remain connection settings, not MCP settings.

### 13.2 Persistent indicator

While MCP is enabled, show a persistent status indicator with:

- enabled state;
- active call count;
- pending-approval count;
- running-operation count;
- error/recovery badge.

### 13.3 Agent Activity

Add a first-class view with sections or filters for:

- Pending approval
- Running
- Needs recovery
- Completed
- Rejected/expired/cancelled/failed

Each card shows client, action, safe identities, risk, phase, timestamps, backup/revert state, and redacted errors.

### 13.4 Approval view

The approval surface must be native GPUI, focused, keyboard-accessible, and screen-reader-labelled. It provides:

- Reject
- Approve & Run
- Cancel operation, once running
- Propose Revert, when eligible

No pre-checked approval, generic “Continue” without consequences, or approval from notification-only UI. Protected/Production approval requires typing the target database name.

### 13.5 Progress and recovery

Sync operations show phase-level progress rather than fabricated percentages when MongoDB tools do not provide reliable totals. Recovery UI prominently distinguishes:

- original operation failure;
- rollback in progress;
- rollback completed;
- manual recovery required.

---

## 14. Implementation modules

Use three focused modules and keep their interfaces small:

```text
src/mcp/
  mod.rs             McpController public interface
  server.rs          rmcp/Axum transport and middleware
  bridge.rs          bounded GPUI state resolution
  tools/
    mod.rs
    connections.rs
    reads.rs
    saved_queries.rs
    actions.rs

src/actions/
  mod.rs             ActionBroker public interface
  model.rs           typed actions, operations, states
  policy.rs          effective policy evaluator
  store.rs           atomic durable persistence/retention
  audit.rs           metadata allow-list and correlation
  redaction.rs       shared secret/error redaction

src/sync/
  mod.rs             SyncExecutor public interface
  plan.rs            normalized backup/sync/revert plans and hashes
  backup.rs          manifests and verification
  executor.rs        phase orchestration and recovery

src/state/app_state/mcp.rs
src/state/app_state/actions.rs
src/views/settings/mcp.rs
src/views/agent_activity.rs
```

Expected interfaces:

```text
McpController: enable, disable, status, revoke_grant, shutdown
PolicyEvaluator: capabilities, authorize_read, authorize_proposal, revalidate
ActionBroker: propose, get/list, approve_and_start_from_ui, reject_from_ui, cancel
SyncExecutor: execute(operation, cancellation, progress_sink)
```

`ActionBroker` is the only module that converts an approved proposal into an operation. `SyncExecutor` cannot approve work. MCP tools cannot construct approval state.

Prefer existing helpers and operations over new abstractions:

- `ConnectionManager` and `connection::ops` for MongoDB work;
- `src/connection/ops/bson_tools.rs` for secure external-tool execution;
- `ConnectionWriteIdentity` as the starting identity snapshot;
- `ConfigManager` atomic-write patterns;
- platform `KeyStore` for MCP grants;
- existing query-library visibility/search behavior;
- existing GPUI connection identity badges and confirmation styling.

---

## 15. Delivery phases

### Phase 0 — transport and client spike

Implement only:

- MCP initialization and capability negotiation;
- `tools/list`;
- `openmango_list_connections`;
- `openmango_list_databases`.

Prove:

- `rmcp 3.1.2` integration;
- authenticated literal-loopback Streamable HTTP;
- Host/Origin rejection and request limit;
- Pi's installed `pi-mcp-adapter` can configure the bearer token, connect, discover, describe, and call tools;
- MCP Inspector compatibility;
- GPUI bridge and tunnel reuse;
- listener/app shutdown cancellation;
- no nested Tokio `block_on`.

If required clients cannot configure authenticated Streamable HTTP, stop and decide whether a tiny stdio discovery/authentication shim is justified. Do not build the complete server first.

### Phase 1 — connection policy and bounded reads

- add `agent_shared` and `protected` migration-safe fields;
- add per-client grants and Settings UI;
- implement `PolicyEvaluator`, central redaction, audit envelope, and semaphore;
- extract async read operations;
- add connection, read, and saved-query tools;
- add structured schemas, canonical Extended JSON, limits, and untrusted-output treatment.

### Phase 2 — durable Action Broker and UI, no execution

- add typed action/operation records and atomic store;
- add canonical hashing, expiry, deduplication, policy/identity snapshots, and target fingerprints;
- add Agent Activity and approval/rejection UI;
- add proposal/get/list tools;
- verify that approval cannot be produced through MCP;
- do not run backup/sync/revert yet.

### Phase 3 — backup execution

Implement the lowest-risk durable workflow first:

- app-managed backup paths;
- connection leases;
- secure BSON runner integration;
- manifests and real verification;
- operation progress/cancellation;
- crash/shutdown state handling;
- retention.

Release-gate backup behavior before target mutation is introduced.

### Phase 4 — database sync and automatic recovery

- source staging dump and validation;
- target precondition check;
- verified target backup;
- target replacement;
- target verification;
- automatic rollback on failure/cancellation;
- recovery interlock and recovery UI;
- Testcontainers fault injection.

### Phase 5 — revert and MCP completion

- approved revert proposals;
- absence-marker revert;
- action/operation polling and cancellation tools;
- full target-client manual testing;
- operation/audit retention behavior.

### Phase 6 — unify built-in AI writes

Route built-in AI writes through the same Action Broker. Only after this is stable consider explicit proposal tools for insert, update, delete, and create-index. Keep each operation separate; never expose arbitrary commands or Forge execution.

---

## 16. Testing and acceptance

### 16.1 Unit tests

- policy matrix and Production/protected safety floor;
- sharing reset on Production/protected changes;
- action canonicalization/hash stability;
- stale identity/policy/target preconditions;
- one-shot approval and expiry;
- action deduplication/idempotent transitions;
- read-only target rejection;
- recursive aggregation write/JavaScript-stage rejection;
- Extended JSON conversion and size/depth limits;
- result truncation/pagination metadata;
- central redaction matrix;
- backup manifest verification, including empty/absent databases;
- audit allow-list excludes content.

### 16.2 Protocol and HTTP tests

- initialize/version/capability negotiation;
- tools list/call/error/shutdown;
- invalid/missing/revoked token;
- invalid Host and any Origin;
- CORS absent;
- oversized body and wrong content type;
- per-client/global concurrency and rate limits;
- cancellation races and no late responses;
- unsupported capabilities fail closed;
- Inspector smoke test.

### 16.3 MongoDB integration tests

Use Testcontainers and existing suites for:

- metadata/find/count/aggregate/schema/index/stat/explain behavior;
- read-only MongoDB RBAC denial;
- source and target through direct, SSH, and SOCKS5 transports where applicable;
- source dump, target backup, replacement, and verification;
- import failure followed by successful recovery;
- import and rollback failure causing `recovery_required`;
- cancellation in every sync phase;
- absent and empty target databases;
- external target changes causing stale approval;
- app shutdown with active work.

Extend existing test files when natural; add focused MCP/action/sync integration binaries rather than duplicating all fixtures.

### 16.4 Malicious-content tests

Seed database and saved-query content containing:

- fake system/tool instructions;
- delimiter-like strings;
- huge nested values;
- URIs and credentials;
- terminal/control characters;
- Markdown links and HTML;
- BSON edge types.

Verify structured output, plain-data UI rendering, limits, and redaction. Do not claim these tests prove prompt-injection immunity.

### 16.5 Manual acceptance

Before release:

- test MCP Inspector;
- test Pi through the installed `pi-mcp-adapter` as the first supported real client;
- test every additional declared target MCP client with custom headers;
- verify native approval keyboard and accessibility behavior;
- verify Production sharing warning and typed confirmation;
- inspect copied client configuration for secret leakage;
- force quit/crash during each sync phase and validate restart recovery;
- run `just fmt-check && just lint && just check && just test`.

### 16.6 Release blockers

The feature cannot ship if any of these are true:

- MCP can reconnect independently from copied URIs;
- a mutation can execute without native UI approval;
- unsupported elicitation is treated as approval;
- read-only or unshared connections can become targets;
- secrets appear in protocol output, logs, audit, or support bundles;
- target mutation can begin without a verified backup;
- cancellation can leave an uncertain target without recovery state;
- connection/tunnel teardown can race active target mutation;
- the selected target client cannot authenticate to the endpoint.

---

## 17. Future work

Only after the initial design is proven:

- collection-level sync as a separate explicit action;
- merge/append sync with defined conflict and recovery semantics;
- CRUD/index proposal tools through `ActionBroker`;
- MCP Resources for stable schema/index/saved-query context;
- user-selected prompts;
- MCP Tasks mapped onto existing `Operation.id` after bilateral capability negotiation;
- optional per-tool client scopes;
- a tiny stdio discovery shim if required by supported clients;
- headless/remote architecture with OAuth/TLS and a replacement approval model.

MCP Tasks never replace OpenMango's action/operation storage. Task expiry must not delete audit or recovery records.

---

## 18. Reviewed primary sources

Implementation decisions were checked against these primary sources at the listed revisions.

### MCP specification

- [MCP specification `2026-07-28`](https://modelcontextprotocol.io/specification/2026-07-28)
- [Transports and HTTP security](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)
- [Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)
- [Security best practices](https://modelcontextprotocol.io/specification/2026-07-28/basic/security_best_practices)
- [Tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools)
- [Cancellation](https://modelcontextprotocol.io/specification/2026-07-28/basic/utilities/cancellation)
- [Experimental Tasks](https://modelcontextprotocol.io/specification/draft/basic/utilities/tasks)

### Official Rust SDK

- [`rmcp` release `3.1.2`](https://github.com/modelcontextprotocol/rust-sdk/tree/02c62aef2e331e5cf79c06c744eb1eb052cc8ebd)

### Official MongoDB MCP Server

Reviewed commit: [`994c9b56f61924afd41ec366ee622b08180f90df`](https://github.com/mongodb-js/mongodb-mcp-server/tree/994c9b56f61924afd41ec366ee622b08180f90df) (`2.1.0`).

- [README: transports, read-only, confirmations, and limits](https://github.com/mongodb-js/mongodb-mcp-server/blob/994c9b56f61924afd41ec366ee622b08180f90df/README.md)
- [`src/tools/tool.ts`: operation policy, confirmation, cancellation, telemetry, redaction, and untrusted output](https://github.com/mongodb-js/mongodb-mcp-server/blob/994c9b56f61924afd41ec366ee622b08180f90df/src/tools/tool.ts)
- [`src/common/connectionManager.ts`: standalone server connection ownership](https://github.com/mongodb-js/mongodb-mcp-server/blob/994c9b56f61924afd41ec366ee622b08180f90df/src/common/connectionManager.ts)
- [Malicious/untrusted data tests](https://github.com/mongodb-js/mongodb-mcp-server/blob/994c9b56f61924afd41ec366ee622b08180f90df/tests/accuracy/untrustedData.test.ts)

OpenMango intentionally does **not** copy MongoDB MCP Server's behavior that may continue confirmation-required tools when client elicitation is unavailable. OpenMango always fails closed.

### Google MCP Toolbox for Databases

Reviewed commit: [`6648ad813c17bf12c1a22c37f9c944b31314314b`](https://github.com/googleapis/mcp-toolbox/tree/6648ad813c17bf12c1a22c37f9c944b31314314b).

- [Tool style guide: small toolsets, safety, explicit writes, and pagination](https://github.com/googleapis/mcp-toolbox/blob/6648ad813c17bf12c1a22c37f9c944b31314314b/docs/en/reference/style-guide.md)
- [MCP client transports and Inspector](https://github.com/googleapis/mcp-toolbox/blob/6648ad813c17bf12c1a22c37f9c944b31314314b/docs/en/documentation/connect-to/mcp-client/_index.md)
- [Authentication and authorization](https://github.com/googleapis/mcp-toolbox/blob/6648ad813c17bf12c1a22c37f9c944b31314314b/docs/en/documentation/configuration/authentication/_index.md)
- [Telemetry](https://github.com/googleapis/mcp-toolbox/blob/6648ad813c17bf12c1a22c37f9c944b31314314b/docs/en/documentation/monitoring/telemetry/index.md)

### Arcula

Reviewed local/repository revision: [`ae575caa946b7088769cf94193da637e5d8c8753`](https://github.com/ggagosh/arcula/tree/ae575caa946b7088769cf94193da637e5d8c8753) (`2.0.3`).

- [Connection safety policy](https://github.com/ggagosh/arcula/blob/ae575caa946b7088769cf94193da637e5d8c8753/src/connections.rs)
- [Hash-bound sync plans](https://github.com/ggagosh/arcula/blob/ae575caa946b7088769cf94193da637e5d8c8753/src/plans.rs)
- [Approval records](https://github.com/ggagosh/arcula/blob/ae575caa946b7088769cf94193da637e5d8c8753/src/approvals.rs)
- [Operation and revert records](https://github.com/ggagosh/arcula/blob/ae575caa946b7088769cf94193da637e5d8c8753/src/operations.rs)
- [Backup/sync/recovery workflow](https://github.com/ggagosh/arcula/blob/ae575caa946b7088769cf94193da637e5d8c8753/src/core/sync.rs)

The Arcula source is guidance for behavior, not a second runtime or authority model. OpenMango's connection store, keychain integration, native UI, and BSON process runner remain authoritative.
