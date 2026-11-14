# ADR-0001: Rust Workspace and Crate Boundaries

- **Status:** Accepted
- **Date:** 2026-11-13
- **Decision owner:** Project owner (mission requirement)
- **Supersedes:** none

## Context

ChronoFork must be a cross-platform platform whose session and storage layers are
independent of any one emulated system. The initial mission explicitly requires a
Rust Cargo workspace, a deterministic test backend, and tests for session
isolation, snapshots and forking. The development host is WSL2 Ubuntu 24.04 with no
preinstalled Rust.

## Problem

How should the initial code be split so that backend-specific hardware does not
leak into generic session, storage or transport layers, without overengineering a
multi-service architecture?

## Alternatives

1. **Single crate.** Fastest to start, but encourages hardware/session coupling and
   makes the backend boundary unenforceable.
2. **Many small crates per concept** (ids, time, errors, snapshots, fork, etc.).
   Strong separation, but excessive ceremony and premature abstraction.
3. **Five focused crates** with a one-directional dependency graph.

## Decision

Use a Cargo workspace with six crates:

- `chronofork-core` — backend-agnostic vocabulary (IDs, ticks, input, hashes).
- `chronofork-backend-api` — capability, snapshot, inspection and backend traits.
- `chronofork-sessions` — session lifecycle, checkpoints, lineage, fork/restore.
- `chronofork-test-backend` — deterministic synthetic implementation.
- `chronofork-megadrive` — minimal real 68000 + ROM/RAM backend slice.
- `chronofork-cli` — minimal presentation/composition layer.

Use Rust edition 2024, workspace-level lints, `unsafe_code = "forbid"`, and a
`rust-toolchain.toml` pinned to stable with rustfmt and clippy.

## Rationale

- The dependency graph makes the backend boundary structural, not a convention.
- Five crates are enough to separate concerns without inventing services.
- A synthetic backend validates the contracts before a real emulator is selected.
- The CLI demonstrates composition without making presentation part of the core.

## Consequences

Positive:

- Backend APIs can be tested without hardware.
- A future HTTP/MCP/GUI adapter can reuse `chronofork-sessions`.
- Workspace lints and tests provide a repeatable quality gate.

Negative / trade-offs:

- More manifests and crate boundaries than a single binary.
- Cross-crate API changes require coordinated versioning.
- A real backend may reveal missing contract operations; additive changes are
  expected.

## Evidence

- Toolchain: `rustc 1.100.0`, `cargo 1.100.0`, rustfmt 1.11.0, clippy 0.1.100.
- `cargo check --workspace` passes.
- `cargo test --workspace` passes 40 tests after the Mega Drive slice.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- Layout is recorded in [architecture overview](../architecture/overview.md).
