# openmango Features

MongoDB TUI Manager - Complete Feature Set

---

## Navigation Hierarchy

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                 │
│   CONNECTION ──────► DATABASE ──────► COLLECTION ──────► DOCUMENT
│                                                                 │
│   Server-level      DB-level         Table-level       Row-level
│   operations        operations       operations        operations
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

```mermaid
graph LR
    C[Connection] --> D[Database]
    D --> L[Collection]
    L --> Doc[Document]

    C -.->|copy to| C2[Another Connection]
    D -.->|copy to| D2[Another Database]
    L -.->|copy to| L2[Another Collection]
```

---

## Priority Legend

| Level | Meaning | Description |
|:-----:|---------|-------------|
| **P0** | Critical | Core functionality - app unusable without it |
| **P1** | Important | Expected features - users will look for these |
| **P2** | Useful | Enhanced experience - productivity boosters |
| **P3** | Polish | Nice-to-have - power user features |

---

## 1. Per Connection

```
┌──────────────────────────────────────────────────────────────┐
│  CONNECTION                                                  │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────────────┐ │
│  │  Add    │  │  Edit   │  │ Remove  │  │ Connect/Disconnect│
│  └─────────┘  └─────────┘  └─────────┘  └─────────────────┘ │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ mongodb+srv://user:pass@cluster.mongodb.net/db          ││
│  │ [Test Connection]  [Save]                               ││
│  └─────────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────────────┘
```

| Priority | Feature | Description |
|:--------:|---------|-------------|
| P0 | Add connection | Add new MongoDB connection with string validation |
| P0 | Test connection | Verify connectivity before saving |
| P0 | Connect / Disconnect | Establish and close connections |
| P0 | List databases | Show all databases on connected server |
| P0 | Connection status | Show connected/disconnected/error state |
| P0 | Error display | Show connection errors clearly |
| P1 | Edit connection | Modify saved connection details |
| P1 | Remove connection | Delete saved connection |
| P1 | Connection profiles | Auth method, TLS settings, timeouts |
| P1 | Tagging / Favorites | Mark frequently used connections |
| P1 | Import/Export connections | Backup and share connection list |
| P2 | Health ping | Show connection latency |
| P2 | Server info | Version, topology, replica set status |
| P2 | Read-only mode | Toggle to prevent accidental writes |
| P3 | Connection groups | Organize connections into folders |
| P3 | SSH tunnel | Connect through SSH |

---

## 2. Per Database

```
┌──────────────────────────────────────────────────────────────┐
│  DATABASE: my_database                                       │
│                                                              │
│  ┌──────────────────────────────────────────────────────────┐│
│  │  Stats                                                   ││
│  │  ├─ Collections: 12                                      ││
│  │  ├─ Documents: 1.2M                                      ││
│  │  └─ Size: 450 MB                                         ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
│  Actions: [Create] [Drop] [Copy] [Refresh]                   │
└──────────────────────────────────────────────────────────────┘
```

| Priority | Feature | Description |
|:--------:|---------|-------------|
| P0 | List collections | Show all collections in database |
| P0 | Create database | Create new empty database |
| P0 | Drop database | Delete database with confirmation |
| P0 | Refresh | Reload database and collection list |
| P0 | Database stats | Collection count, document count, size |
| P1 | Copy database | Copy to another connection |
| P1 | Rename database | Copy + drop original |
| P1 | Copy options | Drop target, include indexes, select collections |
| P2 | Users / Roles | Manage database-level access |
| P2 | Read/Write concern | Configure durability settings |
| P2 | Profiler toggle | Enable/disable slow query logging |
| P3 | Schema summary | Overview of document schemas |

```mermaid
flowchart TB
    subgraph "Copy Database"
        A[Source DB] --> B{Options}
        B --> C[Drop target first?]
        B --> D[Include indexes?]
        B --> E[Select collections]
        C & D & E --> F[Target Connection]
        F --> G[Target DB Name]
    end
```

---

## 3. Per Collection

```
┌──────────────────────────────────────────────────────────────┐
│  COLLECTION: users                                           │
│                                                              │
│  ┌──────────────────────────────────────────────────────────┐│
│  │  Stats                                                   ││
│  │  ├─ Documents: 45,231                                    ││
│  │  ├─ Avg Doc Size: 1.2 KB                                 ││
│  │  ├─ Total Size: 52 MB                                    ││
│  │  └─ Indexes: 4                                           ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
│  ┌──────────────────────────────────────────────────────────┐│
│  │  Indexes                                                 ││
│  │  ├─ _id_ (unique)                                        ││
│  │  ├─ email_1 (unique)                                     ││
│  │  ├─ created_at_-1                                        ││
│  │  └─ name_text                                            ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
│  Actions: [Browse] [Create] [Drop] [Rename] [Copy] [Export]  │
└──────────────────────────────────────────────────────────────┘
```

| Priority | Feature | Description |
|:--------:|---------|-------------|
| P0 | List collections | Show collections with doc counts |
| P0 | Create collection | Create new collection |
| P0 | Drop collection | Delete with confirmation |
| P0 | Rename collection | Change collection name |
| P0 | Collection stats | Doc count, size, index count |
| P0 | Open document browser | View documents in collection |
| P1 | Copy collection | Copy to another db/connection |
| P1 | Copy with filter | Copy only matching documents |
| P1 | List indexes | Show all indexes |
| P1 | Create index | Add new index |
| P1 | Drop index | Remove index |
| P1 | Import JSON/CSV | Load documents from file |
| P1 | Export JSON/CSV | Save documents to file |
| P1 | Aggregation editor | Build aggregation pipelines |
| P2 | Schema analysis | Sample and analyze document structure |
| P2 | Validation rules | JSON schema validation |
| P2 | TTL index management | Auto-expire documents |
| P2 | Collection options | View capped, validator, etc. |
| P3 | Compare collections | Diff between two collections |
| P3 | Sync collections | Sync data across connections |
| P3 | Filtered clone | Clone subset to new collection |

```mermaid
flowchart LR
    subgraph "Collection Actions"
        A[Collection] --> B[Browse Documents]
        A --> C[Manage Indexes]
        A --> D[Import/Export]
        A --> E[Copy/Clone]
    end
```

---

## 4. Per Document

```
┌──────────────────────────────────────────────────────────────┐
│  DOCUMENT BROWSER: users                     [1-25 of 45231] │
│                                                              │
│  Filter: { "status": "active" }                              │
│  Sort: { "created_at": -1 }                                  │
│                                                              │
│  ┌──────────────────────────────────────────────────────────┐│
│  │ {                                                        ││
│  │   "_id": ObjectId("..."),                                ││
│  │   "name": "John Doe",                                    ││
│  │   "email": "john@example.com",                           ││
│  │   "status": "active",                                    ││
│  │   "created_at": ISODate("2024-01-15")                    ││
│  │ }                                                        ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
│  [Insert] [Edit] [Delete] [Duplicate] [Copy to...]           │
│                                                              │
│  [◀ Prev]  Page 1 of 1810  [Next ▶]                          │
└──────────────────────────────────────────────────────────────┘
```

| Priority | Feature | Description |
|:--------:|---------|-------------|
| P0 | Find documents | Query with filter |
| P0 | View document | Display raw JSON |
| P0 | Insert document | Add new document |
| P0 | Edit document | Modify and replace |
| P0 | Delete document | Remove with confirmation |
| P1 | Projection | Select specific fields to display |
| P1 | Sort | Order results by field |
| P1 | Limit / Skip | Pagination through results |
| P1 | Duplicate document | Clone existing document |
| P1 | Bulk insert | Insert multiple documents |
| P1 | Copy to collection | Copy selected docs elsewhere |
| P2 | Operator updates | Use $set, $inc, $push, etc. |
| P2 | Bulk update | Update all matching documents |
| P2 | Bulk delete | Delete all matching documents |
| P2 | Explain plan | Show query execution details |
| P2 | Field diff | Compare two document versions |
| P3 | Undo last write | Best-effort rollback |
| P3 | Inline field edit | Edit single field without modal |
| P3 | Schema hints | Autocomplete field names |

```mermaid
flowchart TB
    subgraph "Document CRUD"
        Q[Query/Filter] --> R[Results List]
        R --> V[View]
        R --> E[Edit]
        R --> D[Delete]
        R --> C[Copy]
        I[Insert] --> R
    end
```

---

## 5. Query & Aggregation

```
┌──────────────────────────────────────────────────────────────┐
│  QUERY EDITOR                                                │
│                                                              │
│  ┌──────────────────────────────────────────────────────────┐│
│  │ Filter:                                                  ││
│  │ { "age": { "$gte": 21 }, "status": "active" }            ││
│  ├──────────────────────────────────────────────────────────┤│
│  │ Sort:                    Limit:                          ││
│  │ { "created": -1 }        100                             ││
│  ├──────────────────────────────────────────────────────────┤│
│  │ Projection:                                              ││
│  │ { "name": 1, "email": 1, "age": 1 }                      ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
│  [Run Query]  [Save Query]  [Export Results]                 │
└──────────────────────────────────────────────────────────────┘
```

```
┌──────────────────────────────────────────────────────────────┐
│  AGGREGATION PIPELINE                                        │
│                                                              │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐   │
│  │ $match  │ -> │ $group  │ -> │ $sort   │ -> │ $limit  │   │
│  │         │    │         │    │         │    │         │   │
│  │ {status:│    │ {_id:   │    │ {total: │    │ 10      │   │
│  │  "active│    │  "$type"│    │   -1}   │    │         │   │
│  │ }       │    │ }       │    │         │    │         │   │
│  └─────────┘    └─────────┘    └─────────┘    └─────────┘   │
│       ↓              ↓              ↓              ↓        │
│   [Preview]     [Preview]      [Preview]      [Preview]     │
│                                                              │
│  [+ Add Stage]  [Run Pipeline]  [Export]                     │
└──────────────────────────────────────────────────────────────┘
```

| Priority | Feature | Description |
|:--------:|---------|-------------|
| P0 | JSON filter input | Write MongoDB query |
| P1 | Aggregation pipeline | Build multi-stage pipelines |
| P1 | Stage preview | See output at each stage |
| P2 | Query history | Access recent queries |
| P2 | Saved queries | Persist favorite queries |
| P2 | Explain plan | Analyze query performance |
| P3 | Visual query builder | Drag-and-drop construction |
| P3 | Export to code | Convert to JS/Python/etc. |

---

## 6. Import / Export

```mermaid
flowchart LR
    subgraph "Import"
        J1[JSON File] --> I[Import]
        C1[CSV File] --> I
        I --> DB[(Collection)]
    end

    subgraph "Export"
        DB2[(Collection)] --> E[Export]
        E --> J2[JSON File]
        E --> C2[CSV File]
    end
```

| Priority | Feature | Description |
|:--------:|---------|-------------|
| P1 | Export to JSON | Save documents as JSON array |
| P1 | Export to CSV | Save as spreadsheet format |
| P1 | Import from JSON | Load JSON documents |
| P2 | Import from CSV | Load from spreadsheet |
| P2 | Export with filter | Export only matching docs |
| P2 | Export query results | Save current query output |
| P3 | BSON export | Native MongoDB format |

---

## 7. Global / Cross-Cutting

```
┌──────────────────────────────────────────────────────────────┐
│  ┌──────────────────────────────────────────────────────────┐│
│  │                    KEYBOARD NAVIGATION                   ││
│  │                                                          ││
│  │   ↑/k  Move up          /   Search                       ││
│  │   ↓/j  Move down        n   New item                     ││
│  │   ←/h  Collapse         C   Copy                         ││
│  │   →/l  Expand           D   Delete                       ││
│  │   Tab  Switch panel     ?   Help                         ││
│  │   Enter Select          q   Quit                         ││
│  │                                                          ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
│  ┌──────────────────────────────────────────────────────────┐│
│  │ Status: Connected to localhost:27017 | v7.0.4 | 12 DBs   ││
│  └──────────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────────────┘
```

| Priority | Feature | Description |
|:--------:|---------|-------------|
| P0 | Keyboard navigation | Full vim-style controls |
| P0 | Destructive confirmations | Confirm before drop/delete |
| P0 | Status bar | Current state and context |
| P0 | Error display | Clear error messages |
| P1 | Saved queries | Persist and reuse queries |
| P1 | Query history | Browse recent queries |
| P1 | Export query results | Save filtered data |
| P2 | Server metrics | Memory, connections, ops/sec |
| P2 | Slow query viewer | Browse profiler output |
| P2 | Current operations | View running operations |
| P3 | Audit/log viewer | Browse server logs |
| P3 | Theming | Color scheme customization |
| P3 | Keymap customization | Rebind keyboard shortcuts |

---

## Feature Summary by Priority

### P0 - Critical (Core Functionality)

- [ ] Connection: add, test, connect/disconnect, list databases, status
- [ ] Database: list collections, create, drop, stats, refresh
- [ ] Collection: list, create, drop, rename, stats, browse docs
- [ ] Document: find, view, insert, edit, delete
- [ ] Global: keyboard nav, confirmations, status bar, errors

### P1 - Important (Expected Features)

- [ ] Connection: edit, remove, profiles, favorites
- [ ] Database: copy with options, rename
- [ ] Collection: copy, indexes (list/create/drop), import/export, aggregation
- [ ] Document: sort, project, paginate, duplicate, bulk insert
- [ ] Query: saved queries, history

### P2 - Useful (Enhanced Experience)

- [ ] Connection: health ping, server info, read-only mode
- [ ] Database: users/roles, profiler
- [ ] Collection: schema analysis, validation rules, TTL
- [ ] Document: operator updates, bulk ops, explain, diff
- [ ] Admin: server metrics, slow queries, current ops

### P3 - Polish (Power User)

- [ ] Connection: groups, SSH tunnel
- [ ] Database: schema summary
- [ ] Collection: compare, sync, filtered clone
- [ ] Document: undo, inline edit, schema hints
- [ ] Global: theming, keymap customization, audit logs

---

## Implementation Status Matrix

**Status Legend:** ✅ Implemented | ⚠️ Partial | ❌ Not implemented

### 1. Connection Features

| Feature | Priority | Status | Notes |
|---------|----------|--------|-------|
| Add connection | P0 | ✅ | Full modal with validation |
| Test connection | P0 | ✅ | Error categorization, latency |
| Connect/Disconnect | P0 | ✅ | Keyboard shortcuts c/d |
| List databases | P0 | ✅ | Auto-loads on connect |
| Connection status | P0 | ✅ | Visual indicators |
| Error display | P0 | ✅ | Shows in details panel |
| Edit connection | P1 | ✅ | Modal with e key |
| Remove connection | P1 | ✅ | x key with confirm |
| Connection profiles | P1 | ⚠️ | Types exist, no UI |
| Tagging/Favorites | P1 | ❌ | Not implemented |
| Import/Export connections | P1 | ❌ | Not implemented |
| Health ping | P2 | ✅ | Latency in test |
| Server info | P2 | ⚠️ | Version only, no topology UI |
| Read-only mode | P2 | ❌ | Not implemented |

### 2. Database Features

| Feature | Priority | Status | Notes |
|---------|----------|--------|-------|
| List collections | P0 | ✅ | In tree navigation |
| Create database | P0 | ✅ | Modal with n key |
| Drop database | P0 | ⚠️ | Service exists, no UI trigger |
| Refresh | P0 | ⚠️ | Connection level only |
| Database stats | P0 | ✅ | Collection/doc count, size |
| Copy database | P1 | ⚠️ | Modal exists, not wired |
| Rename database | P1 | ❌ | Not implemented |
| Copy options | P1 | ⚠️ | UI exists, not functional |

### 3. Collection Features

| Feature | Priority | Status | Notes |
|---------|----------|--------|-------|
| List collections | P0 | ✅ | With doc counts |
| Create collection | P0 | ⚠️ | Modal exists, create commented out |
| Drop collection | P0 | ❌ | Service exists, no UI |
| Rename collection | P0 | ❌ | Service exists, no UI |
| Collection stats | P0 | ✅ | Doc count, size, indexes |
| Open document browser | P0 | ✅ | Full implementation |
| Copy collection | P1 | ✅ | Full modal with options |
| List indexes | P1 | ⚠️ | Service exists, no UI |
| Create/Drop index | P1 | ❌ | Not implemented |
| Import/Export | P1 | ❌ | Not implemented |
| Aggregation editor | P1 | ❌ | Not implemented |

### 4. Document Features

| Feature | Priority | Status | Notes |
|---------|----------|--------|-------|
| Find with filter | P0 | ✅ | With autocomplete |
| View document | P0 | ✅ | JSON tree view |
| Insert document | P0 | ⚠️ | Service exists, no UI |
| Edit document | P0 | ✅ | Tree navigation mode |
| Delete document | P0 | ⚠️ | Service exists, no UI |
| Projection | P1 | ⚠️ | Service only, no UI |
| Sort | P1 | ⚠️ | Service only, no UI |
| Pagination | P1 | ✅ | Full UI with <> keys |
| Duplicate document | P1 | ⚠️ | Service exists, no UI |

### 5. Global Features

| Feature | Priority | Status | Notes |
|---------|----------|--------|-------|
| Keyboard navigation | P0 | ✅ | Vim-style controls |
| Destructive confirmations | P0 | ✅ | Confirm dialog |
| Status bar | P0 | ✅ | Bottom action hints |
| Error display | P0 | ✅ | In panels and modals |
