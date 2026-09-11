# Updater workflow changes

Branch: `fix/updater-workflow`, based on merged PR #15 (`900cf38`).

## Findings

- The app log records repeated download failures with `No such file or directory`. The report places the failure after progress reaches 100%.
- Downloads previously shared `.OpenMango-update.download` and `OpenMango-update.zip`. A second attempt or another attempt's cleanup could remove the first attempt's file before its final rename. The existing logs do not identify which competing operation triggered each occurrence.
- Checks could replace an active download's state, and the independent progress task could overwrite completion with a late percentage update.
- Stable installations also checked nightly and accepted any different commit as an update, including older history.
- Installation replaced the app before asking about unsaved work and fell back to `/Applications` when launched from a development executable.

## Changes

- Each download owns a unique temporary file. Verification passes ownership to the ready-to-install state; there is no shared-file deletion or final rename. Canceling or failing an attempt cleans up only its own file.
- Checking and downloading have cancellation and request IDs. Progress, verification, and completion use one ordered stream, and download progress stops at 99% until verification finishes.
- Settings expose Stable and Nightly explicitly, defaulting to the build's release channel. Same-channel nightly updates require GitHub to report forward ancestry. Downloads pin GitHub asset IDs and verify size and SHA-256.
- A Software Update dialog uses Kit buttons, channel menus, progress, spinner, and error alerts. It shows the current build, selected channel, available build, download details, verification, preparation, and recovery actions.
- Installation uses isolated staging and a filesystem lock, verifies signatures and the installed app's signing identity, and asks about unsaved work before swapping bundles. Failed swaps or launches restore the previous app; failed restoration preserves the recovery copy.
- Development executables never install over `/Applications/OpenMango.app`. Translocated apps receive instructions to move and reopen the app. Closed windows stop their periodic update checks.

## Validation

Local validation passed on macOS:

- `just fmt-check`
- `just lint` (all targets)
- `just check-release` and `just lint-release` (with `mimalloc`)
- `just check-sidecar`
- `cargo test -- --test-threads=1` with the active Docker daemon: 657 passed, 0 failed, 1 ignored, including the Docker integration suites.

The focused updater tests passed for independent download-file cleanup, checksum/build identity parsing, nightly ancestry, development-executable detection, and rollback after a failed bundle swap. The GUI and a real signed macOS installation were not exercised; verify that flow manually before release.

Manual scenarios: download/cancel/retry; check while downloading; stable/nightly selection; checksum failure; missing cached file; read-only/translocated installation; unsaved-work cancellation; signature rejection; launch failure and rollback.

See [primary-source research](UPDATER_WORKFLOW_RESEARCH.md) for component APIs and distribution guidance.
