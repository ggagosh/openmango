# Changelog

All notable changes to OpenMango will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Theme system with Vercel Dark and Darcula Dark themes, runtime switching
- Window vibrancy effect

### Fixed
- Collection data not loading / spinner stuck on empty collections
- SRV connection string resolution errors
- Password redaction in connection display
- Sidecar build for x86_64 release target
- Text overflow in JSON editor
- Forge shell spinner not appearing

### Changed
- Switched sidecar runtime from Node.js to Bun
- Updated JSON editor font
- Integration tests now share one MongoDB container per test binary instead of spawning one per test (121 → 9 containers), with UUID-namespaced databases for isolation
- Upgraded test MongoDB image from 5.0.6 (EOL) to 7.0 LTS
- Fixed MongoDB 7.0 compatibility in stats tests (`i64` field types, removed `indexDetails` option, `currentOp` admin-only enforcement)

## [0.1.6] - 2026-02-07

### Added
- Forge query shell (mongosh-compatible REPL per database)
- Transfer progress tracking for database-scope operations
- Aggregation pipeline list performance improvements

### Fixed
- Node sign display issues
- Forge shell state persistence
- Editor inline editing bugs
- Export/import edge cases

### Changed
- Major internal refactoring of editor and state management
- Custom fonts (KAPO)

## [0.1.5] - 2026-01-31

### Added
- Aggregation pipeline builder
- Import/Export/Copy transfer system (JSON, JSONL, CSV, BSON formats)
- Multi-connection support
- Bulk update operations
- Document key assignments
- Extended JSON support (Relaxed & Canonical modes)
- Action bar with common operations
- Cancel in-progress async operations
- Copy/paste for sidebar tree items

### Fixed
- Inline editing regressions
- Tab close behavior
- Expand/collapse state bugs

### Changed
- Major architecture refactor (session-per-tab model)

## [0.1.4] - 2026-01-20

### Added
- Connection manager
- Keyboard navigation for document tree
- Delete and paste operations
- Read-only mode for views

### Fixed
- Long text editing overflow

## [0.1.3] - 2026-01-18

### Added
- Error banner notifications
- Context menu actions for properties

## [0.1.2] - 2026-01-16

### Added
- Document search (Cmd+F)
- Index creation dialog
- Property-level actions (copy, add, delete)

## [0.1.1] - 2026-01-15

### Changed
- Initial improvements after first release

## [0.1.0] - 2026-01-15

### Added
- Initial release
- Connect to MongoDB and browse databases/collections
- Tree-based document viewer with expand/collapse
- Inline BSON value editing
- Pagination
- BSON syntax highlighting
