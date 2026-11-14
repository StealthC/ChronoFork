# Getting Started

## Prerequisites

- Linux, WSL2, macOS or Windows with a Rust-capable C linker.
- Git.
- Rust 1.85+ (tested with 1.100.0). `rust-toolchain.toml` requests stable with
  rustfmt and clippy.

The synthetic backend has no external runtime dependencies and does not need a
BIOS, firmware or commercial ROM.

## Install Rust (user-local)

If `cargo` is already available, skip this section.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal -c rustfmt -c clippy
. "$HOME/.cargo/env"
```

This installs into `~/.cargo` and `~/.rustup`; it does not require `sudo`.

## Build and test

```sh
cargo check --workspace
cargo test --workspace
```

Run the full local gate:

```sh
./scripts/check.sh
```

## Run the demonstration

```sh
cargo run -p chronofork-cli -- demo
```

Expected evidence:

- backend is explicitly identified as a synthetic test backend;
- a checkpoint is created at tick 12;
- three forks start with the checkpoint hash;
- branches diverge under different inputs;
- the source session is unchanged;
- a memory write in one branch does not affect another;
- restoring and replaying reproduces the branch hash;
- serialized checkpoint checksum is verified;
- final line: `chronofork demo: PASS`.

Inspect advertised capabilities:

```sh
cargo run -p chronofork-cli -- backend describe
```

Only implemented capabilities should report `true`.

## Workspace layout

```text
Cargo.toml                 workspace + lints
rust-toolchain.toml        stable + rustfmt/clippy
crates/
  chronofork-core/         IDs, ticks, input, SHA-256
  chronofork-backend-api/  capability/backend contracts
  chronofork-sessions/     lifecycle, snapshots, forks, lineage
  chronofork-test-backend/ synthetic deterministic machine
  chronofork-cli/          minimal demo CLI
docs/                      official documentation
scripts/check.sh           local quality gate
tests/                     reserved for cross-crate fixtures (none yet)
```

Ignored directories:

- `.research/upstream/` — reference clones; never commit.
- `.planning/` — operational planning memory; never commit.
- `target/` — build output.

## What not to do

- Do not add commercial ROMs, BIOS/firmware or game assets.
- Do not present the synthetic backend as Mega Drive emulation.
- Do not add HTTP/MCP/GUI code before the service boundary is designed.
- Do not commit `.research/` or `.planning/`.
- Do not push or create a remote repository without authorization.

## Troubleshooting

**`cargo: command not found`** — source `$HOME/.cargo/env` or open a new shell.

**`rustfmt`/`clippy` missing** — `rustup component add rustfmt clippy`.

**Clippy fails on a new lint** — fix the code; do not add `#[allow]` without a
reason and review.

**Tests fail after a snapshot format change** — bump the format version and update
decode tests; never silently reinterpret old bytes.

**A reference build fails** — the headless Exodus port requires Ninja, clang and
`expat`/`zlib`/`libpng`/`libjpeg` development packages. Those are not installed in
the bootstrap environment; request approval before adding them.
