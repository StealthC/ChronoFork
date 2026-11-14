#!/usr/bin/env sh
# Local ChronoFork quality gate. No CI-provider assumptions.
set -eu

if [ -f "${HOME}/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "${HOME}/.cargo/env"
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is not installed or not on PATH" >&2
    echo "install the stable toolchain with rustfmt and clippy first" >&2
    exit 1
fi

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo check --workspace"
cargo check --workspace

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> RUSTDOCFLAGS=\"-D warnings\" cargo doc --workspace --no-deps"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

echo "==> all quality gates passed"
