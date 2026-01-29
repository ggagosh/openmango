# Import / Export / Copy (Design + UX)

This document defines a complete import/export/copy feature set for OpenMango, including formats, UX flows, options, and implementation guidance.

## 1) Goals

- Make data movement safe, explicit, and recoverable.
- Cover common workflows:
  - Export/import collections and databases.
  - Copy collection -> collection and database -> database.
  - Copy between connections (self-hosted <-> Atlas, prod <-> staging).
  - Export aggregation results.
- Preserve BSON types when desired, with clear tradeoffs for CSV.
- Provide progress, cancel, and validation for long-running transfers.

## 2) Non-goals (v1)

- Full point-in-time backups or restore. (Use mongodump/mongorestore or managed backups.)
- CDC or live replication.

## 3) User expectations (Compass + MongoDB tools)

Why this section exists: users already know Compass and MongoDB tools, so our UX and format choices should match their mental model.

- Compass supports collection import/export for JSON and CSV. JSON can be newline-delimited or a single JSON array. CSV expects a header line. Compass is not a backup tool. It can export filtered subsets and aggregation results. (https://www.mongodb.com/docs/compass/import-export/) 
- MongoDB tools use Extended JSON v2.0. By default, mongoexport outputs Relaxed mode; Canonical mode is available with --jsonFormat. Mongoimport expects Extended JSON v2 by default. (https://www.mongodb.com/docs/database-tools/mongoexport/mongoexport-behavior/ , https://www.mongodb.com/docs/v8.0/reference/mongodb-extended-json/)
- mongoimport expects UTF-8 and can insert in random order unless maintainInsertionOrder is set. (https://www.mongodb.com/docs/database-tools/mongoimport/mongoimport-behavior/)
- mongoimport supports CSV type declarations (columnsHaveTypes). (https://www.mongodb.com/docs/database-tools/mongoimport/mongoimport-examples/)
- mongodump output includes collection metadata (options, UUIDs, indexes) when writing to a directory, uses Extended JSON v2 canonical for metadata, and index data is rebuilt on restore. (https://www.mongodb.com/docs/database-tools/mongodump/mongodump-behavior/)
- Field names with $ or . can cause import/export conflicts (Extended JSON wrappers and CSV dot-notation). (https://www.mongodb.com/docs/v8.0/core/dot-dollar-considerations/)

## 4) Feature matrix (OpenMango)

| Feature | Scope | UI entry | Default format | Notes |
| --- | --- | --- | --- | --- |
| Export collection | Single collection | Collection view | JSON (Extended JSON) | CSV optional, warn on type loss. |
| Import collection | Single collection | Collection view | JSON (Extended JSON) | CSV/TSV optional. |
| Export database | All collections | Database view | Dump (BSON + metadata) | JSON optional, lower fidelity than dump. |
| Import database | All collections | Database view | Dump (BSON + metadata) | Use mongorestore or internal importer. |
| Copy collection | Collection -> collection | Transfer dialog | Live cursor stream | Supports cross-connection. |
| Copy database | DB -> DB | Transfer dialog | Live cursor stream | Includes indexes/options if requested. |
| Export aggregation results | Aggregation view | Export button | JSON (array or JSONL) | CSV for flat results. |

## 5) Formats and fidelity

### 5.1 JSON (Extended JSON v2)
- Relaxed mode: human-friendly, may lose some type fidelity.
- Canonical mode: full fidelity, less readable.
(https://www.mongodb.com/docs/v8.0/reference/mongodb-extended-json/)

### 5.2 JSON container style
- JSON Lines (one document per line) is ideal for streaming and large files.
- JSON Array is easier for tools that expect a single JSON value. Mongoexport supports a JSON array with --jsonArray; default is one JSON document per line. (https://www.mongodb.com/docs/v3.0/reference/program/mongoexport/)
- Compass accepts both JSONL and JSON array inputs. (https://www.mongodb.com/docs/compass/import-export/)

### 5.3 CSV / TSV
- Best for flat data and spreadsheets.
- Loses BSON type fidelity and nested structure detail. (https://www.mongodb.com/docs/compass/import-export/)
- Use explicit column types where possible (mongoimport columnsHaveTypes). (https://www.mongodb.com/docs/database-tools/mongoimport/mongoimport-examples/)

### 5.4 Binary dump (BSON + metadata)
- Highest fidelity; includes collection options and indexes when dumped to a directory. (https://www.mongodb.com/docs/database-tools/mongodump/mongodump-behavior/)
- Recommended for database export/import and full-fidelity backup workflows.

## 6) UX / UI design

### 6.1 Entry points
- Collection view: Import / Export (toolbar buttons + sidebar menu).
- Database view: Export DB / Import DB (sidebar menu).
- Aggregation view: Export results.
- Transfer tab: copy collection or database between connections.
- Sidebar action menu (left click): Export / Import / Copy.

### 6.1.1 Keyboard shortcuts (sidebar selection)
- Export: Cmd+Alt+E (Ctrl+Alt+E)
- Import: Cmd+Alt+I (Ctrl+Alt+I)
- Copy: Cmd+Alt+C (Ctrl+Alt+C)

### 6.2 Collection export flow
1) Choose format (JSON, JSONL, CSV, TSV)
2) Optional filter/project/sort/limit (uses existing query bar)
3) JSON options: Canonical vs Relaxed, pretty-print, array vs lines
4) CSV options: delimiter, header, flattening
5) Confirm and export

### 6.3 Collection import flow
1) Choose file (JSON/JSONL/CSV/TSV)
2) Detect format + preview
3) Map fields/types if CSV
4) Insert mode (insert/upsert/replace)
5) Validation and error handling
6) Run import with progress

### 6.4 Database export/import flow
- Export DB: select collections, output format, compression
- Import DB: select source, drop/merge strategy, restore indexes/options

### 6.5 Transfer tab (copy between connections)
Step 1: Source (connection -> database -> collection/all)
Step 2: Destination (connection -> database -> collection/new name)
Step 3: Options (overwrite/merge, indexes, filters)
Step 4: Confirm summary

Note: the current UI shows a mocked options form (UI only) for all modes and formats.

### 6.6 Aggregation export
- Export pipeline results to JSON array/JSONL or CSV.
- Option to save pipeline JSON alongside output for reproducibility.

## 7) Options by operation

### Export collection
- Filter / projection / sort / limit
- Format: JSONL, JSON array, CSV, TSV
- JSON mode: Relaxed or Canonical Extended JSON
- Pretty-print
- Compression: gzip
- CSV: delimiter, quote char, header, flatten arrays (stringify vs join)
- Optional: include indexes metadata (separate file)

### Import collection
- Format: JSONL, JSON array, CSV, TSV
- JSON mode: accept Extended JSON v2 (Relaxed/Canonical)
- Optional: accept shell-style syntax for convenience (ObjectId(...), ISODate(...))
- Insert mode: insert / upsert / replace
- Match field for upsert (default _id)
- Drop target collection before import (dangerous toggle)
- Batch size, ordered vs unordered
- Stop on error vs continue
- Ignore empty strings (CSV)
- CSV mapping and type casting (int, double, bool, date, ObjectId)
- Encoding: require UTF-8 (surface error early). (https://www.mongodb.com/docs/database-tools/mongoimport/mongoimport-behavior/)
- Maintain insertion order option if user cares about deterministic ordering. (https://www.mongodb.com/docs/database-tools/mongoimport/mongoimport-behavior/)

### Export database
- Include/exclude collections
- Format: dump (BSON + metadata), or JSON/CSV (lower fidelity)
- Output: directory vs archive
- Compression: gzip
- Warn about case-insensitive file systems when dumping to a directory

### Import database
- Source: dump directory or archive
- Drop existing collections (dangerous)
- Namespace mapping (rename DB/collection)
- Restore indexes/options

### Copy collection (live)
- Filter / projection / pipeline
- Destination name (rename)
- Overwrite/merge behavior
- Indexes/options: copy indexes, collation, validation rules
- Batch size, ordered vs unordered

### Copy database (live)
- Include/exclude collections
- Overwrite/merge behavior
- Copy indexes/options
- Batch size, ordered vs unordered

### Aggregation export
- Format: JSON array, JSONL, CSV
- Flattening for CSV
- Limit/sample results
- Save pipeline JSON

## 8) Validation, warnings, and safety

- Warn when CSV is chosen due to type loss. (https://www.mongodb.com/docs/compass/import-export/)
- Warn when field names include $ or . because mongoimport/mongoexport and Extended JSON can conflict. (https://www.mongodb.com/docs/v8.0/core/dot-dollar-considerations/)
- Warn when importing JSON with mixed types (example: number vs string in same column for CSV)
- Validate permissions before starting transfer
- Require explicit confirmation for drop/overwrite
- Provide dry-run count/estimate before large jobs

## 9) Implementation architecture

### 9.1 Transfer job abstraction
Pipeline: Source -> Transform -> Sink

- Source: collection cursor, pipeline cursor, or file reader
- Transform: optional mapping, projection, renaming, flattening
- Sink: file writer (JSON/CSV), BSON dump writer, or live insert

### 9.2 Internal vs external tooling
- Internal path: use MongoDB driver cursors for streaming copy/import/export.
- External tools: optionally integrate mongodump/mongorestore for full-fidelity DB export/import (when installed).

### 9.3 Progress and cancellation
- Periodic progress events (docs processed, bytes written, errors)
- Cancel safely (stop cursor, flush file handle)
- Retry on transient network errors

## 10) Roadmap

Phase 1:
- Collection export/import (JSON + CSV)
- Aggregation export (JSON + CSV)

Phase 2:
- Live copy collection between connections
- Live copy database between connections

Phase 3:
- Dump/restore integration (mongodump/mongorestore)
- Job history and scheduled exports

## References (source docs)

- Compass import/export: https://www.mongodb.com/docs/compass/import-export/
- Extended JSON v2: https://www.mongodb.com/docs/v8.0/reference/mongodb-extended-json/
- mongoexport behavior: https://www.mongodb.com/docs/database-tools/mongoexport/mongoexport-behavior/
- mongoimport behavior: https://www.mongodb.com/docs/database-tools/mongoimport/mongoimport-behavior/
- mongoimport examples (CSV types): https://www.mongodb.com/docs/database-tools/mongoimport/mongoimport-examples/
- mongoexport jsonArray: https://www.mongodb.com/docs/v3.0/reference/program/mongoexport/
- mongodump behavior: https://www.mongodb.com/docs/database-tools/mongodump/mongodump-behavior/
- dot and dollar considerations: https://www.mongodb.com/docs/v8.0/core/dot-dollar-considerations/
