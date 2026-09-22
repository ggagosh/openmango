# Comparison and sync validation

## Selective sync and undo — MongoDB 8

Measured locally on 2026-09-22 against a disposable MongoDB **8.2.3** container in OrbStack,
using the optimized Rust test profile. The native unordered bulk writer batches up to 1,000 rows;
larger BSON batches are split by retained-read bytes and the driver's wire-message limits.

Final local checks passed:

- `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings`.
- 693 library tests (1 unrelated ignored test), including native checkbox interaction, selection,
  confirmation invalidation, encrypted log authentication, corruption, cleanup, and BSON guards.
- 8 sync integration tests, 8 comparison integration tests, and 9 settings/workspace tests.
- The ignored 100,000-document bulk benchmark, run explicitly.

Unit tests now use isolated temporary configuration directories, so a saved Compare tab in the
developer's workspace cannot affect tab tests. Live native app interaction has not been manually
verified; the GPUI checks use the test harness. The existing `block 0.1.6` dependency emits a
future-compatibility notice.

| Operation | Documents | Elapsed | Documents/s |
| --- | ---: | ---: | ---: |
| Sync replacements, including two-sided revalidation and encrypted, flushed undo records | 100,000 | 7.149 s | 13,987 |
| Guarded undo, including log decryption and batched target reads | 100,000 | 4.805 s | 20,813 |

The fixture has matching integer `_id` values, a 256-byte payload, and one changed string in
every document. Both measured operations assert 100,000 successful writes; the final target
contents are checked. Seeding and comparison are outside the measured intervals. This is one
local run, without UI rendering or simulated network latency; it is not a production throughput
or peak-memory guarantee.

Reproduce using a disposable container (Docker must be available):

```sh
DOCKER_HOST=unix:///Users/cpo/.orbstack/run/docker.sock \
  cargo test --test compare_sync_tests compare_sync_bulk_benchmark -- --ignored --nocapture
```

The sync integration suite covers both directions, custom/dotted compound keys, target `_id`
preservation, BSON round trips, stale source/target data, filtered-out documents, newly duplicated
keys, unique-index and `_id` collisions with partial success, cancellation, guarded undo conflicts,
9 MiB document replacements and undo, and restoring a deleted `_id` reused by an insertion
(including different numeric BSON types). MongoDB 7 targets and views are refused before writing;
MongoDB 7 remains supported as a source for a MongoDB 8 target.

## Read-only comparison measurements

Measured locally on 2026-09-22 on Apple Silicon macOS, MongoDB 7.0 in a disposable OrbStack
container. Rust used the repository's optimized test profile. These are local measurements,
not remote-network or production throughput guarantees.

## Correctness

- `cargo test --lib compare::tests -- --test-threads=1`: 12 passed.
- `DOCKER_HOST=unix:///Users/cpo/.orbstack/run/docker.sock cargo test --test compare_tests -- --test-threads=1`:
  8 passed, 1 manual benchmark ignored.
- Explicitly running that ignored benchmark: passed twice. All three cases in both runs found
  exactly 990,000 identical pairs and 10,000 different pairs, with no other buckets populated.

## Million-document benchmark

Seed: `scripts/compare-bench-seed.js`. Three collections, each with 1,000,000 roughly 1 KB
documents; 1% differ. The `sku` comparison uses different `_id` values on the two sides.
Each measured case reads **2,000,000 documents total**, so documents/s is aggregate read
throughput, not matched pairs/s. Results were drained without rendering a UI.

| Match key | First elapsed | First documents/s | Repeat elapsed | Repeat documents/s |
| --- | ---: | ---: | ---: | ---: |
| `_id` | 3.200 s | 625,070 | 3.007 s | 665,153 |
| indexed `sku` | 3.931 s | 508,841 | 3.717 s | 538,087 |
| unindexed field | 5.138 s | 389,276 | 4.837 s | 413,461 |

`/usr/bin/time -l` around the compiled benchmark executable reported maximum RSS of
138,510,336 bytes (132.1 MiB), and peak memory footprint of 91,734,616 bytes (87.5 MiB).
The measurements exclude compilation and the MongoDB server's memory.

The repeat sampled process RSS every 250 ms. After startup it reached about 107.5 MiB during
the `_id` pass, 131.2–131.3 MiB during the indexed custom-key pass, and 115.3–116.6 MiB during
the later unindexed pass. This supports bounded memory for these fixtures. It does not establish
the cap's worst case for large BSON keys, a different fraction of differences, or UI rendering.

## Query plans

`explain("executionStats")` used the same simple collation, key predicates, and sort as the scan:

| Key | Winning stages | Returned | Documents examined | Keys examined | Server elapsed |
| --- | --- | ---: | ---: | ---: | ---: |
| `_id` | FETCH → IXSCAN | 1,000,000 | 1,000,000 | 1,000,000 | 679 ms |
| `sku` | FETCH → IXSCAN | 1,000,000 | 1,000,000 | 1,000,000 | 890 ms |
| unindexed | SORT → COLLSCAN | 1,000,000 | 1,000,000 | 0 | 1,785 ms |

The `$exists` / `$not $type: array` predicates did not prevent the custom-key index from
providing order in this fixture.

## Reproduction

Use a disposable server; the seed script refuses existing benchmark collections.

```sh
mongosh <local-test-uri> scripts/compare-bench-seed.js
OPENMANGO_COMPARE_BENCH_URI=<local-test-uri> cargo test --test compare_tests \
  compare_million_document_benchmark -- --ignored --nocapture
```

To isolate RSS from the compiler, run the resulting `target/debug/deps/compare_tests-*`
executable directly with the same test filter under `/usr/bin/time -l` on macOS.
Logs from this session are in ignored `target/compare-*.log` files.

Not measured: remote latency, UI throughput/responsiveness during a million-document scan,
long-idle session refresh, sharded servers, or peak memory at the result-storage cap.

## Read-only Compare tab validation

The final library run passed **683 tests**, with no failures and one ignored test. This includes
the Compare layout test at 430/900/1200 px, populated result/detail rendering, tab switching,
offline configuration restoration, connected restoration without duplicate tabs, filtered key
detail queries, cancellation on tab-state drop, and row-background contrast checks.

The final integration rerun also passed: **8 comparison tests** against MongoDB and
**9 settings/workspace tests**. The manual benchmark remained ignored in this ordinary rerun;
its two explicit runs are recorded above. `cargo fmt --all -- --check` and
`cargo clippy --lib --bin openmango --tests -- -D warnings` passed on the final source.

An earlier sandboxed run blocked five existing MCP listener tests with `Operation not permitted`.
The full rerun with local-listener access passed; those were environment restrictions.

Independent source-review disposition: **ship** for the reviewed code findings.

| Finding | Final review |
| --- | --- |
| Compare configuration must restore without a successful connection | Resolved; startup restore and regression tests |
| Key-based detail queries must preserve the comparison filter | Resolved; filter and eligibility predicates retained |
| Retained results must name their original scope | Resolved; summary uses the saved run configuration |

The native app was not launched. Screenshot appearance, live keyboard interaction, and
million-document UI responsiveness remain unverified. The disposable benchmark container was
removed after measurements; the user's existing MongoDB container was not modified.

The subsequently reported header overlap exposed a gap in the original parent-only layout
test. [The layout repair and stronger regression](COMPARE_UI_REVIEW.md) now cover actual
dropdown hitboxes, collection statistics, toolbar containment, and Options/dropdown interaction.
