# OpenMango

<img src="assets/logo/openmango-1024.png" alt="OpenMango logo" width="240" />

GPU-accelerated MongoDB desktop client built with GPUI. Fast navigation, JSON-first editing, and a tabbed workspace.

## Highlights

- Connection manager with test/connect/disconnect
- Database & collection browser with stats and refresh
- Document browser with filter, sort, projection, and pagination
- Inline edits, JSON editor, duplicate, insert, and delete
- Paste documents from clipboard (JSON object/array or NDJSON)
- Bulk delete (filtered or all)
- Index management (list/create/drop)
- Tabs with preview/permanent behavior and session restore
- Status bar + error banner feedback

See `docs/features.md` for the detailed feature matrix.

## Roadmap

- Import/export data and connections
- Aggregation pipeline editor + explain plan
- Query history and saved queries
- Read-only/safe mode
- Theming and keymap customization
- Multi-window / split views
- Schema explorer + validation editor

## Quick start

Requirements:
- Rust stable
- MongoDB instance reachable (local or remote)

```sh
just dev
```

## Build

```sh
just release
```

## Release packaging

```sh
scripts/release_macos.sh
scripts/release_linux.sh
```

## Tests

```sh
cargo test
```

Optional smoke tests (needs a live MongoDB):

```sh
MONGO_URI="mongodb://localhost:27017" cargo test smoke_core_flows
MONGO_URI="mongodb://localhost:27017" cargo test crud_sanity
MONGO_URI="mongodb://localhost:27017" cargo test query_sanity
MONGO_URI="mongodb://localhost:27017" cargo test indexes_sanity
MONGO_URI="mongodb://localhost:27017" cargo test stats_sanity
```

## Troubleshooting

- Linux build fails with `-lxkbcommon-x11` or X11-related errors: install X11 dev packages for your distro.
- macOS build fails with `-fuse-ld=lld`: remove any lld linker flags from `.cargo/config.toml`.

## Notes
- Requires Rust stable.
- macOS release script supports codesign + notarization when env vars are set.
