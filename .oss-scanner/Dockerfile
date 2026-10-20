# Build image for Anthropic's OSS Scanner (github.com/anthropics/oss-scanner). The scanner builds
# this with the repository as context, then audits inside it with no network: everything cargo
# needs is fetched and compiled here.
FROM rust:1-bookworm
ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HUSKY_DONT_INSTALL_HOOKS=true \
    CARGO_INCREMENTAL=0

# The same system packages CI and the release builds install.
COPY scripts/bootstrap_linux.sh /tmp/bootstrap_linux.sh
RUN bash /tmp/bootstrap_linux.sh && rm -rf /var/lib/apt/lists/* /tmp/bootstrap_linux.sh

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src

# rust-toolchain.toml pins the stable channel; install it now, there is no network later.
RUN rustup toolchain install stable --profile minimal \
    && cargo fetch --locked

# Debug profile: the integration-test seams in src/ are gated on cfg(debug_assertions).
RUN cargo build --locked

# The integration suites (tests/*_tests.rs) start MongoDB through Testcontainers and need Docker,
# so only the unit tests are built and run. Built in their own step: compiling this crate's lib
# and lib tests side by side was killed for memory in an 8 GB builder.
RUN cargo test --locked --lib -- --test-threads=1 || echo "WARNING: unit tests reported failures"
