# Stable/nightly updater workflow research

Reviewed 2026-09-11 on `fix/updater-workflow`; implementation belongs to the parent task.
Sources: the local updater/release workflow, published GPUI Kit/Base/Component 0.6.0, GitHub REST documentation, Rust/tempfile documentation, and Apple distribution guidance.
Local observations describe the baseline inspected before the concurrent updater changes, not a completed diagnosis or runtime verification.

## What the reported timing means

The baseline emits 100% when the response bytes have arrived, then synchronizes the file, checks SHA-256, removes the old cache ZIP, and renames the partial file.
Therefore **100% downloaded is not the same as a verified package ready to install**.
An error at that transition can come from cache promotion; it does not by itself implicate `ditto`, a mounted image, or restart.
The baseline uses shared `.OpenMango-update.download` and `OpenMango-update.zip` names and deletes/replaces them during attempts.
Another attempt or stale task can therefore interfere with the path still owned by an earlier attempt; this is a concrete mechanism to test, not proof that it caused every reported ENOENT.
[Baseline updater](../src/state/commands/updater.rs), [updater UI state](../src/state/app_state/updater.rs)

Report operation and path context separately: checking release, downloading, flushing, verifying checksum, promoting package, extracting, validating bundle, swapping, or launching.
For example: **Could not prepare the downloaded update** with the underlying rename error in expandable details.
Avoid a single unqualified “Update failed” message that makes all phases look identical.

## GPUI Kit supplies native controls, not an updater service

No updater/auto-update service is exported by the inspected published Kit, Base, or Component crates.
Release lookup, download lifetime, verification, rollback, channel policy, and relaunch remain application responsibilities.
Reuse the native controls already in this dependency graph; no UI-library upgrade is needed for the workflow below.
[Published Kit](https://docs.rs/crate/gpui-kit/0.6.0/source/src/lib.rs), [Component](https://docs.rs/crate/gpui-component/0.6.0/source/src/lib.rs), [Base](https://docs.rs/crate/gpui-base/0.6.0/source/src/lib.rs)

| Need | Verified 0.6.0 API | Limit |
| --- | --- | --- |
| Measured download progress | `Progress::new(id).value(percent)` | Percentage is 0–100; input is clamped, not inferred |
| Accessible progress name | `.accessibility_label(...)` | Application supplies meaningful phase text |
| Unknown-length download / verification | `Spinner::new()` | Does not measure bytes or completion |
| Pending action | `Button::loading(bool)` | Request exclusion/cancellation still belongs to application state |
| Persistent local failure | `Alert::error(id, message)` | Add a retry action appropriate to the failed phase |
| Background notification | `Notification::error(message).autohide(false)` | Keep the same error accessible in the update surface too |
| Technical details | Existing `Collapsible` and native text controls | Do not put raw implementation details in the primary action label |

[Progress](https://docs.rs/crate/gpui-component/0.6.0/source/src/progress/progress.rs), [Spinner](https://docs.rs/crate/gpui-component/0.6.0/source/src/spinner.rs), [Button](https://docs.rs/crate/gpui-component/0.6.0/source/src/button/button.rs), [Alert](https://docs.rs/crate/gpui-component/0.6.0/source/src/alert.rs), [Notification](https://docs.rs/crate/gpui-component/0.6.0/source/src/notification.rs)

## A truthful, recoverable update flow

| Phase | User-facing state | Available action |
| --- | --- | --- |
| Checking | Checking for updates… | Avoid starting a duplicate check |
| Available | Version/build, channel, architecture, release notes | Download update |
| Downloading | Received bytes and percentage when total size is known | Cancel only if implemented correctly |
| Verifying/preparing | Download complete; verifying update… | Retain the package until verification/promotion completes |
| Ready | Update downloaded and verified | Install and restart; Later |
| Installing | Installing update… | Prevent another installation from starting |
| Restarting | Restarting OpenMango… | Preserve failure details if relaunch fails |
| Failed | Specific failed phase and readable reason | Retry download, retry install, or open the release page as appropriate |

For missing `Content-Length`, show an indeterminate indicator and received bytes rather than an invented percentage.
Do not let delayed download-progress messages turn Ready, Failed, or a newer request back into Downloading.
Use one active operation identity/generation and check it at every progress/completion update; filesystem isolation is still needed across app instances.
Keep a verified download available after a cancelled restart or an installation failure when retrying it remains valid.
Resolve unsaved work before replacing the installed bundle, and recheck it at the final destructive transition if preparation was asynchronous.

## Stable and nightly are explicit release identities

GitHub's `/releases/latest` returns the latest published non-draft, non-prerelease release.
The documented ordering uses `created_at`, meaning the release commit date rather than draft/publication time; do not treat it as a generic “newest uploaded artifact” endpoint.
Nightly needs `/releases/tags/nightly` or another explicit channel identity.
The baseline checks both and prefers a newer stable SemVer; otherwise a differing nightly SHA may be offered. That is OpenMango policy, not Kit/GitHub behavior.
Make the selected channel and target build visible, and do not silently cross channels merely because another SHA differs.
A different SHA establishes a different build, not by itself that the build is newer than a local development checkout.
[GitHub: latest release](https://docs.github.com/en/rest/releases/releases#get-the-latest-release), [release by tag](https://docs.github.com/en/rest/releases/releases#get-a-release-by-tag-name)

For nightly ancestry, call `GET /repos/{owner}/{repo}/compare/{LOCAL_SHA}...{REMOTE_SHA}` with the local build as BASE and the candidate as HEAD.
The documented commit comparison corresponds to `git log BASE..HEAD`; use its comparison status, not SHA inequality or commit-list length.
`ahead` means the candidate is ahead of the local base; `identical` means no change; `behind` means an older candidate; `diverged` is not a fast-forward update.
Only `ahead` should produce an automatic newer-nightly offer. An unknown local SHA, unavailable comparison, or divergent history needs an explicit unavailable/manual path, not a silent downgrade or an unsupported “up to date” claim.
Direction was also verified through read-only GitHub API responses for the known Kit commits: `3a25a73...36b5181` returned ahead by 7; the inverse returned behind by 7.
[GitHub: compare two commits](https://docs.github.com/en/rest/commits/commits#compare-two-commits)

The repository's nightly workflow deletes and recreates the `nightly` release, with ZIP and matching `.sha256` assets.
A tag/filename URL can consequently name different bytes later or disappear during replacement.
Capture the selected release ID, asset ID/name, architecture, size, expected checksum/digest, and build identity together.
Do not fetch an unpinned old ZIP and a newly replaced checksum under the same mutable tag and call the pair one release.
If the pinned asset disappears, return to release checking and offer the newly resolved candidate rather than silently substituting it mid-download.
[Local nightly publication](../.github/workflows/nightly.yml), [GitHub: release asset API](https://docs.github.com/en/rest/releases/assets#get-a-release-asset)

GitHub asset metadata includes ID, state, size, download URL, and a digest when provided; accept a completed uploaded asset of the expected architecture/format.
The asset-ID endpoint with `Accept: application/octet-stream` may return bytes directly or redirect; clients must handle both 200 and 302.
GitHub rejects uploading another asset under an existing filename unless the old asset is removed, so filename equality is not immutable identity.
Validate the bytes received against the selected metadata and the project's checksum requirement; an HTTP success alone does not verify an update.
[GitHub: download and upload assets](https://docs.github.com/en/rest/releases/assets)

## Isolated staging and safe promotion

Use one owned staging file/directory per attempt, preferably under the destination cache filesystem.
`tempfile` 3.26 is already a project dependency; `NamedTempFile::new_in(cache_dir)` provides unique file creation without a shared-name delete/create sequence.
Complete the write, synchronize as needed, verify the digest, then promote the same owned artifact and retain its lifetime through Ready/Install.
Do not drop a temporary-file owner and keep only its path: automatic cleanup can make that path disappear.
`persist(...)` can atomically replace an existing file and returns ownership on failure; it does not synchronize file/directory contents and cannot cross filesystems.
[Existing dependency](../Cargo.toml), [tempfile 3.26: NamedTempFile](https://docs.rs/tempfile/3.26.0/tempfile/struct.NamedTempFile.html)

Prefer an attempt-specific verified ZIP path over a globally shared “latest ZIP” filename; a second app instance must not replace the package another instance is about to install.
Cleanup should remove only artifacts owned by that attempt, with stale cleanup separate from active work.
When a fixed destination is necessary, do not delete the existing destination before a same-filesystem file promotion.
Rust `rename` can fail when the source no longer exists or paths cross mount points; POSIX replacement provides an atomic name transition for the individual rename.
That guarantee does not make a multi-step `.app → backup → new .app` installation a single atomic transaction.
[Rust: rename](https://doc.rust-lang.org/std/fs/fn.rename.html), [POSIX: rename](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)

## macOS paths, bundles, and relaunch

The inspected updater consumes ZIP assets and uses `ditto`; it does not attach a DMG, so an `hdiutil` rewrite is not indicated by this report.
Apple documents that apps launched directly from ZIP/disk-image distribution locations may have randomized/translocated bundle paths.
Treat an executable-derived bundle path as runtime location, not proof of a writable installation target.
Handle mounted/read-only/translocated locations explicitly, and do not silently replace an unrelated `/Applications/OpenMango.app` when running a development binary.
Apple also recommends testing upgrades, duplicate installations, different locations, and different installing/running user accounts.
[Apple: Packaging Mac software](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution)

Verify the staged app's signature and expected identity before replacing the installed app; keep a recoverable previous bundle until installation is confirmed.
ZIP checksums and signed/notarized application contents serve different purposes; retaining both checks is appropriate for direct distribution.
Apple identifies direct-download software updates as developer-managed, with Developer ID/notarization used for trusted distribution.
[Apple: Distributing software on macOS](https://developer.apple.com/macos/distribution/)

`codesign --verify` alone does not establish that the candidate is OpenMango from the expected publisher.
Apple TN3127 documents an explicit `-R` requirement: bind the signing identifier and the expected Team ID, not merely `anchor apple generic` (which accepts identities issued by Apple generally).
For a Developer ID-only channel, the requirement can also constrain the Developer ID intermediate and Application certificate OIDs.
The expected identity must come from trusted application/build configuration, never from the downloaded candidate itself.
Example requirement text, with placeholders replaced by trusted values:
`anchor apple generic and identifier "EXPECTED_BUNDLE_ID" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "EXPECTED_TEAM_ID"`
Pass it as a test requirement (Apple's command examples use `-R '=...'`) alongside strict verification. Quote the Team ID, including when it starts with a digit.
[Apple: TN3127 requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements), [requirement language](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/RequirementLang/RequirementLang.html), [Apple DTS: quoted Team IDs](https://developer.apple.com/forums/thread/801478)

Use known executable paths such as `/usr/bin/ditto` and `/usr/bin/open`, or a native launch API, and pass paths as separate arguments.
Rust searches `PATH` for non-absolute program names, so a process-spawn ENOENT can mean a missing executable rather than a missing update archive.
Capture spawn errors and command exit status/stderr; do not discard relaunch failure and then close all windows as though restart succeeded.
[Rust: Command](https://doc.rust-lang.org/std/process/struct.Command.html), [Apple: NSWorkspace application launch](https://developer.apple.com/documentation/appkit/nsworkspace/openapplication(at:configuration:completionhandler:))

If a helper performs replacement after the old process exits, transfer ownership of its staging path before quitting and give it absolute validated input/output paths.
The helper must survive parent exit, retain the verified package, report swap/rollback/launch outcomes, and clean only its own files.
Do not place it solely inside a temporary owner that the parent will drop or inside the bundle it is about to remove.
These are lifecycle requirements, not evidence that Kit supplies a helper or that a helper is required to fix the post-download error.

## Focused verification targets for implementation

- Two concurrent attempts/app instances cannot remove or promote each other's partial ZIPs.
- Missing source at promotion produces phase-specific context, and no failed/stale attempt can publish Ready.
- Unknown-length and interrupted responses never show invented completion; 100% transitions to verification before Ready.
- Nightly replacement between check/download produces a controlled refresh/retry, not an unexplained checksum or missing-file loop.
- Download/install retries, Later, and cancelled dirty-work prompts retain the correct verified artifact.
- Installer destination, backup restoration, read-only/translocated launch locations, and restart failure are exercised without replacing the development user's installed app.

Research was read-only except for this document; no app launch, build, test, release mutation, or dependency change was performed.
