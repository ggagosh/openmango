# OpenMango Development Commands

# Development
dev:
    cargo run

debug:
    RUST_LOG=debug cargo run

watch:
    bacon run

# Quality
lint:
    cargo clippy --all-targets -- -D warnings

lint-release:
    cargo clippy --release --features mimalloc -- -D warnings

fmt:
    cargo fmt

fmt-check:
    cargo fmt -- --check

check:
    cargo check

check-release:
    cargo check --release --features mimalloc

# Build
build:
    cargo build

release:
    cargo build --release --features mimalloc

bundle:
    cargo bundle --release --features mimalloc

# Testing
test:
    cargo test

test-verbose:
    cargo test -- --nocapture

unit-test:
    cargo test --bin openmango -- --test-threads=1

# Maintenance
udeps:
    cargo +nightly udeps

bloat:
    cargo bloat --release --crates --bin openmango

clean:
    cargo clean

# CI checks (matches GitHub Actions quality job; integration-tests still require Docker)
ci: fmt-check check-release lint-release check-sidecar unit-test

# All checks before commit
precommit: ci test

# Download MongoDB tools for BSON export/import support
download-tools:
    ./scripts/download_tools.sh

# Build Forge mongosh sidecar (compiled binary via Bun)
build-sidecar:
    ./scripts/build_mongosh_sidecar.sh

# Verify Forge mongosh sidecar source bundles cleanly
check-sidecar:
    ./scripts/check_mongosh_sidecar.sh
