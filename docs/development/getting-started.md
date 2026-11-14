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
cargo run -p chronofork-cli -- megadrive demo
```

The first command runs the synthetic contract scenario. The second runs the
minimal real 68000 + ROM/RAM + VDP-port + interrupt-scheduler slice; it is not a
full Mega Drive emulator.

Expected evidence for the synthetic demo:

- backend is explicitly identified as a synthetic test backend;
- a checkpoint is created at tick 12;
- three forks start with the checkpoint hash;
- branches diverge under different inputs;
- the source session is unchanged;
- a memory write in one branch does not affect another;
- restoring and replaying reproduces the branch hash;
- serialized checkpoint checksum is verified;
- final line: `chronofork demo: PASS`.

Expected evidence for the Mega Drive slice:

- reset vectors load PC `0x200` and A7 `0x01000000` (the top of work RAM, so
  interrupt pushes stay inside RAM);
- the synthetic ROM computes `1 + 2`, programs VDP register `0x0F`, writes and
  reads back the VRAM word `0x1234`, and captures the VDP status;
- an HBlank request is pending at tick 64 (level 4, vector `0x70`) and is delivered
  on the next instruction boundary;
- a VBlank request is pending at the start of vertical blanking (level 6, vector
  `0x78`) and survives a snapshot/restore round-trip;
- a fork/RAM mutation diverges while the source stays unchanged;
- restoring the checkpoint reproduces the original CPU, VDP and scheduler hash;
- final line: `chronofork megadrive demo: PASS`.

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
  chronofork-megadrive/    minimal 68000 + ROM/RAM slice
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
`expat`/`zlib`/`libpng`/`libjpeg` development packages; the exact versions used for
the recorded differential evidence are in
[Oracle differential evidence](../research/oracle-differential-evidence.md). This is
an optional external oracle, not part of the default Rust build. Do not install new
system packages without approval.
