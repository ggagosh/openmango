# OpenMango

![OpenMango logo](assets/logo/openmango-1024.png)

GPU-accelerated MongoDB client built with GPUI.

## Quick start

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

## Notes
- Requires Rust stable.
- macOS release script supports codesign + notarization when env vars are set.
