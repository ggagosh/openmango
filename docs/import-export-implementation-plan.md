# Import/Export/Copy Implementation Plan

This document defines the implementation plan for OpenMango's import/export/copy feature based on the design doc and UX decisions.

## Summary of Decisions

| Feature | Decision |
|---------|----------|
| Preview Pane | Full preview (3-5 docs in side panel) |
| Formats | JSON, JSONL, CSV + BSON (mongodump) |
| Insert Modes | Insert + Upsert + Replace |
| Compression | Skip for v1 (except BSON archive) |
| Copy Options | Data only (no indexes) |
| Query Filter | No filtering (export all) |
| Scope | Collection + Database |
| Progress | Simple status message |
| Warnings | Yes, inline warnings |
| Estimate/Dry-run | No |
| CSV Mapping | No mapping UI |
| Extended JSON | Dropdown (Relaxed/Canonical) |
| Aggregation Export | Skip for now |
| Keyboard Shortcuts | Yes (Cmd+Alt+E/I/C) |
| Confirmations | Yes for destructive ops |
| Options UI | Collapsible sections |
| Pretty Print | Yes |
| BSON Export | Bundle mongodump/mongorestore (macOS only) |
| BSON Output | Both folder and archive formats |
| BSON Scope | Database only |
| BSON Restore | Full restore (data + indexes + options) |

---

## Phase 1: State & Types

### 1.1 New Enums (`src/state/app_state/types.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum InsertMode {
    #[default]
    Insert,
    Upsert,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExtendedJsonMode {
    #[default]
    Relaxed,
    Canonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BsonOutputFormat {
    #[default]
    Folder,
    Archive,
}
```

### 1.2 Update `TransferFormat`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferFormat {
    #[default]
    JsonLines,
    JsonArray,
    Csv,
    Bson,  // Only available for Database scope
}
```

### 1.3 Expand `TransferTabState`

```rust
pub struct TransferTabState {
    // Existing fields...
    pub mode: TransferMode,
    pub scope: TransferScope,
    pub source_connection_id: Option<Uuid>,
    pub source_database: String,
    pub source_collection: String,
    pub destination_connection_id: Option<Uuid>,
    pub destination_database: String,
    pub destination_collection: String,
    pub format: TransferFormat,
    pub file_path: String,

    // New fields
    pub insert_mode: InsertMode,
    pub json_mode: ExtendedJsonMode,
    pub pretty_print: bool,
    pub drop_before_import: bool,
    pub stop_on_error: bool,
    pub batch_size: u32,
    pub bson_output: BsonOutputFormat,

    // Preview state
    pub preview_docs: Vec<Document>,
    pub preview_loading: bool,
}
```

---

## Phase 2: Backend Infrastructure

### 2.1 Bundle mongo-tools (macOS only)

**Files to add:**
- `resources/bin/macos/mongodump`
- `resources/bin/macos/mongorestore`

**Build script changes:**
- Copy binaries to app bundle during release build
- Set executable permissions

**Runtime detection:**
```rust
// src/connection/tools.rs
pub fn mongodump_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // Check bundled location first
        let bundled = std::env::current_exe()
            .ok()?
            .parent()?
            .join("../Resources/bin/mongodump");
        if bundled.exists() {
            return Some(bundled);
        }
    }
    // Fallback to PATH
    which::which("mongodump").ok()
}

pub fn mongorestore_path() -> Option<PathBuf> {
    // Similar logic
}

pub fn tools_available() -> bool {
    mongodump_path().is_some() && mongorestore_path().is_some()
}
```

### 2.2 Export Operations (`src/connection/export.rs`)

```rust
pub enum ExportResult {
    Success { count: u64, path: PathBuf },
    Error { message: String },
}

// JSON/JSONL export
pub async fn export_collection_json(
    client: &Client,
    database: &str,
    collection: &str,
    format: JsonContainerFormat,  // Lines or Array
    json_mode: ExtendedJsonMode,
    pretty_print: bool,
    path: &Path,
) -> Result<u64, Error>;

// CSV export
pub async fn export_collection_csv(
    client: &Client,
    database: &str,
    collection: &str,
    path: &Path,
) -> Result<u64, Error>;

// BSON export (calls mongodump)
pub async fn export_database_bson(
    connection_string: &str,
    database: &str,
    output: BsonOutputFormat,
    path: &Path,
) -> Result<(), Error>;
```

### 2.3 Import Operations (`src/connection/import.rs`)

```rust
pub async fn import_collection_json(
    client: &Client,
    database: &str,
    collection: &str,
    path: &Path,
    insert_mode: InsertMode,
    stop_on_error: bool,
    batch_size: u32,
) -> Result<u64, Error>;

pub async fn import_collection_csv(
    client: &Client,
    database: &str,
    collection: &str,
    path: &Path,
    insert_mode: InsertMode,
    stop_on_error: bool,
    batch_size: u32,
) -> Result<u64, Error>;

pub async fn import_database_bson(
    connection_string: &str,
    database: &str,
    path: &Path,
    drop_before: bool,
) -> Result<(), Error>;
```

### 2.4 Copy Operations (`src/connection/copy.rs`)

```rust
pub async fn copy_collection(
    source_client: &Client,
    source_db: &str,
    source_coll: &str,
    dest_client: &Client,
    dest_db: &str,
    dest_coll: &str,
    batch_size: u32,
) -> Result<u64, Error>;

pub async fn copy_database(
    source_client: &Client,
    source_db: &str,
    dest_client: &Client,
    dest_db: &str,
    batch_size: u32,
) -> Result<u64, Error>;
```

### 2.5 Preview Generation (`src/connection/preview.rs`)

```rust
pub async fn generate_export_preview(
    client: &Client,
    database: &str,
    collection: &str,
    format: TransferFormat,
    json_mode: ExtendedJsonMode,
    pretty_print: bool,
    limit: usize,  // 3-5 docs
) -> Result<Vec<String>, Error>;
```

---

## Phase 3: Commands Layer

### 3.1 Transfer Commands (`src/state/commands/transfer.rs`)

```rust
impl AppCommands {
    // Collection export
    pub fn export_collection(
        state: Entity<AppState>,
        transfer_id: Uuid,
        cx: &mut App,
    );

    // Collection import
    pub fn import_collection(
        state: Entity<AppState>,
        transfer_id: Uuid,
        cx: &mut App,
    );

    // Database export (JSON or BSON)
    pub fn export_database(
        state: Entity<AppState>,
        transfer_id: Uuid,
        cx: &mut App,
    );

    // Database import (JSON or BSON)
    pub fn import_database(
        state: Entity<AppState>,
        transfer_id: Uuid,
        cx: &mut App,
    );

    // Collection copy
    pub fn copy_collection(
        state: Entity<AppState>,
        transfer_id: Uuid,
        cx: &mut App,
    );

    // Database copy
    pub fn copy_database(
        state: Entity<AppState>,
        transfer_id: Uuid,
        cx: &mut App,
    );

    // Load preview docs
    pub fn load_transfer_preview(
        state: Entity<AppState>,
        transfer_id: Uuid,
        cx: &mut App,
    );
}
```

---

## Phase 4: UI Components

### 4.1 File Picker Component

**New file:** `src/components/file_picker.rs`

```rust
pub struct FilePicker {
    mode: FilePickerMode,  // Save or Open
    filters: Vec<FileFilter>,
    on_select: Box<dyn Fn(PathBuf)>,
}
```

Uses `rfd` crate for native file dialogs.

### 4.2 Collapsible Section Component

**New file:** `src/components/collapsible.rs`

```rust
pub struct CollapsibleSection {
    title: String,
    expanded: bool,
    children: AnyElement,
}
```

### 4.3 Warning Banner Component

**New file:** `src/components/warning_banner.rs`

```rust
pub struct WarningBanner {
    message: String,
    severity: WarningSeverity,  // Info, Warning, Error
}
```

### 4.4 Confirmation Dialog

Use existing `gpui_component::modal` with confirmation pattern.

---

## Phase 5: Transfer View Overhaul

### 5.1 Layout Structure

```
┌─────────────────────────────────────────────────────────────┐
│ Header: Transfer | Import, export, or copy data    [Run]    │
├─────────────────────────────────────────────────────────────┤
│ [Export] [Import] [Copy]                    Scope ▼ Format ▼│
├──────────────────────────┬──────────────────────────────────┤
│ Source Panel             │ Preview Panel                    │
│ ├─ Connection [▼]        │ ┌──────────────────────────────┐ │
│ ├─ Database [▼]          │ │ {"_id": "...", ...}          │ │
│ └─ Collection [▼]        │ │ {"_id": "...", ...}          │ │
├──────────────────────────┤ │ {"_id": "...", ...}          │ │
│ Destination Panel        │ └──────────────────────────────┘ │
│ ├─ File [Browse...]      │                                  │
│ └─ Format: JSON Lines    │                                  │
├──────────────────────────┴──────────────────────────────────┤
│ ⚠️ Warning: CSV export will lose BSON type fidelity         │
├─────────────────────────────────────────────────────────────┤
│ Options                                              [─] [+] │
│ ┌─ Essential ───────────────────────────────────────────┐   │
│ │ Extended JSON: [Relaxed ▼]  Pretty print: [✓]         │   │
│ └───────────────────────────────────────────────────────┘   │
│ ┌─ Advanced (collapsed) ────────────────────────────────┐   │
│ │ ▶ Batch size, Stop on error, etc.                     │   │
│ └───────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### 5.2 Functional Dropdowns

Replace static text with real `MenuButton` dropdowns:

- **Connection selector**: Populated from `state.connections`
- **Database selector**: Populated from selected connection's databases
- **Collection selector**: Populated from selected database's collections
- **Insert mode**: Insert / Upsert / Replace
- **Extended JSON mode**: Relaxed / Canonical
- **BSON output**: Folder / Archive (when BSON format selected)

### 5.3 Conditional UI

```rust
// Show BSON format only for Database scope
let formats = if transfer_state.scope == TransferScope::Database {
    vec![JsonLines, JsonArray, Csv, Bson]
} else {
    vec![JsonLines, JsonArray, Csv]
};

// Show BSON output format only when BSON selected
if transfer_state.format == TransferFormat::Bson {
    // Show Folder/Archive dropdown
}

// Show warning when CSV selected
if transfer_state.format == TransferFormat::Csv {
    // Render warning banner
}
```

### 5.4 Preview Panel

- Right-side panel (or bottom on narrow screens)
- Shows 3-5 documents formatted according to current options
- Updates when format/options change
- Loading indicator while fetching

---

## Phase 6: Keyboard Shortcuts & Actions

### 6.1 Actions (`src/app/actions.rs`)

```rust
actions!(transfer, [TransferExport, TransferImport, TransferCopy]);
```

### 6.2 Key Bindings (`src/keyboard.rs`)

```rust
// macOS
("cmd-alt-e", TransferExport),
("cmd-alt-i", TransferImport),
("cmd-alt-c", TransferCopy),

// Linux/Windows
("ctrl-alt-e", TransferExport),
("ctrl-alt-i", TransferImport),
("ctrl-alt-c", TransferCopy),
```

### 6.3 Action Handlers

When triggered:
1. Create new TransferTabState with mode pre-selected
2. Pre-fill source from current sidebar selection
3. Open Transfer tab

---

## Phase 7: Warnings System

### 7.1 Warning Conditions

| Condition | Warning Message |
|-----------|-----------------|
| CSV format selected | "CSV export will lose BSON type fidelity (ObjectId, Date, etc.)" |
| Source has `$` or `.` in field names | "Fields with $ or . may cause import issues with some tools" |
| Drop before import enabled | Handled by confirmation dialog |
| BSON tools not available | "mongodump/mongorestore not found. BSON format unavailable." |

### 7.2 Warning Detection

```rust
impl TransferTabState {
    pub fn warnings(&self) -> Vec<TransferWarning> {
        let mut warnings = vec![];

        if self.format == TransferFormat::Csv {
            warnings.push(TransferWarning::CsvTypeLoss);
        }

        // Check for problematic field names would require
        // sampling the collection (done during preview load)

        warnings
    }
}
```

---

## Phase 8: Confirmation Dialogs

### 8.1 Drop Before Import

When user enables "Drop target before import":

```
┌─────────────────────────────────────────┐
│ ⚠️ Destructive Operation                │
├─────────────────────────────────────────┤
│ This will permanently delete all        │
│ documents in:                           │
│                                         │
│   mydb.mycollection                     │
│                                         │
│ This cannot be undone.                  │
├─────────────────────────────────────────┤
│              [Cancel]  [Delete & Import]│
└─────────────────────────────────────────┘
```

---

## File Changes Summary

| File | Changes |
|------|---------|
| `src/state/app_state/types.rs` | Add InsertMode, ExtendedJsonMode, BsonOutputFormat enums; expand TransferTabState |
| `src/state/commands/transfer.rs` | Implement all transfer commands |
| `src/connection/mod.rs` | Add export, import, copy, preview, tools modules |
| `src/connection/export.rs` | New - JSON/CSV/BSON export logic |
| `src/connection/import.rs` | New - JSON/CSV/BSON import logic |
| `src/connection/copy.rs` | New - cross-connection copy logic |
| `src/connection/preview.rs` | New - preview generation |
| `src/connection/tools.rs` | New - mongodump/mongorestore detection |
| `src/views/transfer.rs` | Complete overhaul with functional UI |
| `src/components/file_picker.rs` | New - native file dialog wrapper |
| `src/components/collapsible.rs` | New - collapsible section |
| `src/components/warning_banner.rs` | New - warning banner |
| `src/components/mod.rs` | Export new components |
| `src/app/actions.rs` | Add/verify transfer actions |
| `src/keyboard.rs` | Add transfer shortcuts |
| `Cargo.toml` | Add `rfd` crate for file dialogs |
| `build.rs` or justfile | Bundle mongo-tools for macOS |
| `resources/bin/macos/` | New - mongodump, mongorestore binaries |

---

## Implementation Order

### Sprint 1: Foundation
1. State types (enums, TransferTabState)
2. File picker component
3. Basic export (JSON Lines) working end-to-end

### Sprint 2: Core Formats
4. JSON Array export
5. CSV export
6. JSON import (all modes: Insert/Upsert/Replace)
7. CSV import

### Sprint 3: BSON & Copy
8. Bundle mongo-tools (macOS)
9. BSON export (folder + archive)
10. BSON import
11. Collection copy
12. Database copy

### Sprint 4: UI Polish
13. Preview pane
14. Collapsible sections
15. Warning banners
16. Confirmation dialogs
17. Keyboard shortcuts

### Sprint 5: Testing & Edge Cases
18. Error handling
19. Large file handling
20. Permission checks
21. Status messages

---

## Dependencies to Add

```toml
[dependencies]
rfd = "0.15"           # Native file dialogs
csv = "1.3"            # CSV parsing/writing
flate2 = "1.0"         # For future gzip support
```

---

## Testing Checklist

- [ ] Export 10 docs to JSON Lines
- [ ] Export 10 docs to JSON Array
- [ ] Export 10 docs to CSV
- [ ] Import JSON Lines (insert mode)
- [ ] Import JSON Lines (upsert mode)
- [ ] Import JSON Lines (replace mode)
- [ ] Import CSV
- [ ] Export database to BSON folder
- [ ] Export database to BSON archive
- [ ] Import database from BSON folder
- [ ] Import database from BSON archive
- [ ] Copy collection (same connection)
- [ ] Copy collection (cross-connection)
- [ ] Copy database
- [ ] Preview updates on format change
- [ ] CSV warning appears
- [ ] Drop confirmation dialog works
- [ ] Keyboard shortcuts work
- [ ] File picker opens native dialog
- [ ] Large collection export (10k+ docs)
- [ ] Import with errors (stop vs continue)
