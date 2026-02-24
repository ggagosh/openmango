<p align="center">
  <img src="assets/logo/openmango-1024.png" width="128" alt="OpenMango logo" />
</p>

<h1 align="center">OpenMango</h1>

<p align="center">
  <strong>GPU-accelerated MongoDB client for macOS</strong><br/>
  No Electron. No web views. Just fast.
</p>

<p align="center">
  <a href="https://github.com/ggagosh/openmango/releases/latest"><img src="https://img.shields.io/github/v/release/ggagosh/openmango?label=release" alt="Latest Release" /></a>
  <a href="https://github.com/ggagosh/openmango/actions/workflows/ci.yml"><img src="https://github.com/ggagosh/openmango/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/ggagosh/openmango/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue.svg" alt="License: GPL-3.0" /></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey.svg" alt="Platform: macOS" />
</p>

<p align="center">
  <img src="assets/initial.gif" width="800" alt="OpenMango demo" />
</p>

---

## Install

**[Download the latest release](https://github.com/ggagosh/openmango/releases/latest)** — signed and notarized for macOS (Apple Silicon & Intel).

OpenMango includes a built-in auto-updater so you'll always be on the latest version.

<!--
**Homebrew** (coming soon):

```sh
brew install --cask openmango
```
-->

Or [build from source](#development) if you prefer.

---

## Features

### Forge Shell

Built-in query shell powered by a [Bun/TypeScript sidecar](tools/forge-sidecar/) with mongosh-compatible syntax, schema-aware completions, and inline results.

### Aggregation Builder

Visual pipeline editor — add/reorder/toggle stages, preview intermediate results, and copy the pipeline as code.

### Schema Explorer

Sample documents to discover field types, cardinality, type drift, and outliers across your collection.

### Document Browser

Filter, sort, project, and paginate documents. Edit fields inline in the tree view or open the full JSON editor with validation.

### Transfer System

Import and export JSON, NDJSON, CSV, and BSON. Copy documents between collections or databases with progress tracking.

### Explain Plan

Visualize the winning query plan, index usage, scanned-vs-returned doc counts, and stage costs.

### Connectivity

Standard connections, SRV records, SSH tunneling, and SOCKS5 proxy support. Connection import/export with optional encryption.

### Keyboard-First

40+ keybindings for navigation, tabs, editing, and search. Everything is reachable without a mouse.

### Themes

13 built-in themes — Vercel Dark, Darcula, Tokyo Night, Nord, One Dark, Catppuccin (Mocha & Latte), Solarized (Dark & Light), Ros&eacute; Pine (Dark & Dawn), and Gruvbox (Dark & Light).

See [`docs/features.md`](docs/features.md) for the complete feature matrix.

---

## Roadmap

Upcoming highlights from the [full roadmap](docs/features.md):

- Index diagnostics and "why is this query slow" hints
- Query history with restore
- Saved query snippets and templates
- Validation rule editor
- Live server health panel
- Change stream viewer
- Split view (side-by-side tabs)

---

## Architecture

OpenMango is a native macOS app written in **Rust**.

| Layer | Technology |
|-------|-----------|
| UI framework | [GPUI](https://gpui.rs) — GPU-accelerated via Metal |
| Async runtime | Tokio |
| MongoDB driver | Official Rust driver (`mongodb` crate) |
| Shell sidecar | Bun + TypeScript (mongosh-compatible) |
| Packaging | Signed, notarized `.app` with auto-updater |

The entire UI runs on the GPU through Metal — no web views, no DOM, no CSS layout engine.

---

## Development

**Prerequisites:** Rust (stable), [just](https://github.com/casey/just), MongoDB (local or remote), Docker (for integration tests), Bun (for the Forge sidecar)

```sh
git clone https://github.com/ggagosh/openmango.git
cd openmango
just dev
```

| Command | Description |
|---------|------------|
| `just dev` | Run in development mode |
| `just debug` | Run with `RUST_LOG=debug` |
| `just check` | Fast compile verification |
| `just lint` | Clippy with warnings denied |
| `just fmt-check` | Enforce formatting |
| `just test` | Run all tests |
| `just ci` | Local CI parity |

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contributor guide.

---

## AI Disclosure

OpenMango is fully written by AI — architecture, implementation, tests, and tooling. Human-directed, machine-authored.

---

## License

[GPL-3.0](LICENSE)
