# OpenMango

![OpenMango logo](assets/logo/openmango-1024.png)

GPU-accelerated MongoDB client built with GPUI.

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

## Current gaps

- No index creation UI (list/drop only)
- No import/export flows (data or connections)
- No aggregation pipeline editor or query history
- No read-only/safe mode
- No multi-window or split views

## Notes
- Requires Rust stable.
- macOS release script supports codesign + notarization when env vars are set.
