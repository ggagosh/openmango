# Repository Guidelines

## Project Structure & Module Organization
- `src/` contains application code: `app/` (shell/sidebar), `state/` (state + commands), `connection/` (MongoDB ops), `views/` (screens), `components/` (reusable UI), plus `models/` and `helpers/`.
- `tests/` contains integration-style suites named `*_tests.rs` and shared utilities in `tests/common/`.
- `assets/`, `themes/`, and `resources/` hold bundled UI/static files.
- `tools/forge-sidecar/` contains the Bun/TypeScript sidecar used by Forge-related features.
- `scripts/` contains release and tooling scripts.

## Build, Test, and Development Commands
- `just dev`: run the app locally.
- `just debug`: run with debug logging (`RUST_LOG=debug`).
- `just check`: fast compile verification.
- `just lint`: clippy with warnings denied.
- `just fmt-check`: enforce formatting.
- `just test`: run all Rust tests.
- `just ci`: local CI parity (`fmt-check`, `lint`, `check`, `check-sidecar`).
- `cargo test --test transfer_tests -- --test-threads=1`: run a single integration suite.

## Coding Style & Naming Conventions
- Follow `rustfmt` (`max_width = 100`) and keep code clippy-clean.
- Naming: `snake_case` for functions/modules, `PascalCase` for types/traits, `SCREAMING_SNAKE_CASE` for constants.
- Keep logic in domain folders (for example `state/commands/*` or `connection/ops/*`) rather than growing large mixed modules.

## Testing Guidelines
- Unit-focused checks: `cargo test --bin openmango -- --test-threads=1`.
- Integration suites under `tests/*_tests.rs` use Testcontainers and require Docker for full coverage.
- Add or extend tests with behavior changes; avoid merging without green unit and relevant integration tests.

## Commit & Pull Request Guidelines
- Prefer short imperative commit subjects (current history style: `fix srv error`, `add changelog`).
- PRs should include: purpose, linked issue (if available), test commands executed, and screenshots for UI changes.
- Before opening a PR, run: `just fmt-check && just lint && just test`.

## Security & Configuration Tips
- Never commit credentials, local connection secrets, or machine-specific cert material.
- Keep downloads/tooling reproducible via existing scripts and verify changed build/release behavior in CI.
