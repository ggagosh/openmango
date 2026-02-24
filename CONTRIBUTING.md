# Contributing to OpenMango

Thanks for your interest in contributing! This guide will help you get set up and submit your first PR.

## Prerequisites

- **Rust** (stable toolchain)
- **[just](https://github.com/casey/just)** — command runner
- **MongoDB** — local instance or remote connection for manual testing
- **Docker** — required for integration tests (Testcontainers)
- **[Bun](https://bun.sh)** — for the Forge shell sidecar (`tools/forge-sidecar/`)

## Getting Started

```sh
git clone https://github.com/ggagosh/openmango.git
cd openmango
just dev
```

This compiles and launches the app in development mode.

## Development Commands

| Command | Description |
|---------|------------|
| `just dev` | Run in development mode |
| `just debug` | Run with `RUST_LOG=debug` |
| `just check` | Fast compile verification |
| `just lint` | Clippy with `-D warnings` |
| `just fmt-check` | Check formatting |
| `just test` | Run all tests |
| `just ci` | Full local CI (`fmt-check` + `lint` + `check` + `check-sidecar`) |

Always use `just` commands rather than calling `cargo` directly.

## Project Structure

```
src/
  app/          # Shell, sidebar, top-level layout
  state/        # Application state and commands
  connection/   # MongoDB operations
  views/        # Screens (documents, indexes, aggregation, etc.)
  components/   # Reusable UI components
  models/       # Data models
  helpers/      # Utility functions
tests/          # Integration tests (*_tests.rs) + shared utilities
themes/         # 13 built-in color themes (JSON)
tools/forge-sidecar/  # Bun/TypeScript sidecar for Forge shell
scripts/        # Release and tooling scripts
assets/         # Icons, logos, bundled resources
```

## Coding Standards

- **Formatter:** `rustfmt` with `max_width = 100`
- **Linter:** Clippy with warnings denied — all code must be clippy-clean
- **Naming:** `snake_case` for functions/modules, `PascalCase` for types/traits, `SCREAMING_SNAKE_CASE` for constants
- **Organization:** Keep logic in domain folders (`state/commands/*`, `connection/ops/*`) rather than growing large mixed modules

## Testing

**Unit tests:**

```sh
just test
```

**Integration tests** (requires Docker):

```sh
cargo test --test transfer_tests -- --test-threads=1
```

Integration suites under `tests/` use [Testcontainers](https://testcontainers.com) to spin up MongoDB instances automatically.

Add or extend tests whenever you change behavior. PRs without relevant test coverage may be asked to add it.

## Pull Request Process

1. **Before opening a PR**, run the full local CI check:

   ```sh
   just ci
   ```

2. **PR description** should include:
   - What the change does and why
   - Linked issue (if applicable)
   - Test commands you ran
   - Screenshots for any UI changes

3. **Commit style:** short imperative subjects (e.g., `fix srv error`, `add changelog`)

## Reporting Issues

Found a bug or have a feature idea? [Open an issue](https://github.com/ggagosh/openmango/issues/new/choose) using one of the templates.

## Security

Never commit credentials, connection secrets, or machine-specific certificate material.

## License

By contributing, you agree that your contributions will be licensed under [GPL-3.0](LICENSE).
