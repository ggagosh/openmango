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

fmt:
    cargo fmt

fmt-check:
    cargo fmt -- --check

check:
    cargo check

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

# Maintenance
udeps:
    cargo +nightly udeps

bloat:
    cargo bloat --release --crates --bin openmango

clean:
    cargo clean

# CI checks (matches GitHub Actions)
ci: fmt-check lint check check-sidecar

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
