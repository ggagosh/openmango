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

# All checks before commit
precommit: fmt-check lint test
