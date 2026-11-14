# EPIC-0002 — Minimal Mega Drive 68000 Slice

- **Status:** In progress (first slice implemented)
- **Milestone:** M1
- **Owner:** Lead architect
- **Created:** 2026-11-14
- **Depends on:** [EPIC-0001](EPIC-0001-foundation.md), accepted
  [ADR-0002](../adr/0002-backend-boundaries.md)

## Purpose

Begin the Rust-native Mega Drive production backend under the accepted staged-hybrid
strategy. Prove a real 68000 reset/execute/snapshot/fork path behind the existing
backend and session contracts, while keeping the synthetic backend as the contract
validator.

## Scope

- `chronofork-megadrive` crate with a minimal documented 68000 subset.
- Big-endian ROM window and 64 KiB work RAM.
- Reset vectors, status register and a small flag model.
- Backend contract implementation with versioned snapshots and state hashing.
- Session integration: create, step, snapshot, fork, memory isolation, restore.
- Independently authored synthetic ROM and CLI demo.
- Tests and documentation of exact limitations.

## Out of scope

- Full 68000 instruction set, prefetch or cycle accuracy.
- VDP, VRAM, DMA, rendering.
- YM2612, PSG or audio.
- Z80, interrupts, exceptions, controllers, mapper/SRAM.
- ROM header/region detection or a commercial ROM loader.
- Differential trace evidence against the Exodus oracle (pending build approval).
- GUI, HTTP, MCP or TAS.

## Workstreams and tasks

| ID | Task | Status |
|---|---|---|
| E2-01 | Create crate and workspace wiring | Done |
| E2-02 | Implement ROM/RAM big-endian bus with atomic writes | Done |
| E2-03 | Implement documented 68000 subset and flags | Done |
| E2-04 | Implement reset vectors and SR initialization | Done |
| E2-05 | Implement backend contract, snapshots and state hash | Done |
| E2-06 | Integrate with sessions and prove fork/restore isolation | Done |
| E2-07 | Add synthetic ROM and CLI `megadrive demo` | Done |
| E2-08 | Document scope, limitations and next increments | Done |
| E2-09 | Add VDP status/control and a minimal VRAM window | Planned |
| E2-10 | Add deterministic interrupt/tick scheduler | Planned |
| E2-11 | Add ROM header/region/config identity and cartridge loader | Planned |
| E2-12 | Add external-oracle differential traces | Blocked on Ninja/clang/libpng approval |
| E2-13 | Expand instruction/addressing coverage with tests | Planned |

## Acceptance criteria for this slice

1. Workspace compiles with the new crate.
2. Reset vectors load `SP` and `PC`; `SR = 0x2700`.
3. The synthetic ROM executes deterministically and computes the expected value.
4. Snapshot/restore round-trips state and rejects configuration mismatch.
5. Session fork/restore/isolation works on the real core.
6. Invalid opcodes, unmapped access, odd addresses and ROM writes are typed errors.
7. Multi-byte writes are atomic with respect to invalid ranges.
8. Capabilities report no unimplemented VDP/audio/tracing features.
9. `cargo fmt`, check, Clippy, tests and rustdoc pass.
10. CLI `megadrive demo` exits 0 and proves fork/restore.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Subset mistaken for a complete emulator | Misleading users | Explicit naming, docs, capabilities and CLI text |
| Wrong 68000 decoding | Silent bad execution | Unit tests, differential tests next, errors over skipping |
| Snapshot state missing future devices | Broken restore/fork | Versioned payload; add device state before claiming support |
| Tick = instruction conflated with cycles | False timing claims | Documented; no cycle-accuracy claim |
| Oracle build unavailable | No differential evidence yet | Blocked item; request package approval |

## Definition of Done (slice)

All acceptance criteria pass, docs state exact limits, no ROMs/upstream code are
committed, and the next increment is prioritized.
