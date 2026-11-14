# EPIC-0002 — Minimal Mega Drive 68000 Slice

- **Status:** In progress (68000 + ROM/RAM + VDP port + interrupt-scheduler slices
  implemented; differential evidence and remaining hardware pending)
- **Milestone:** M1
- **Owner:** Lead architect
- **Created:** 2026-11-14
- **Depends on:** [EPIC-0001](EPIC-0001-foundation.md), accepted
  [ADR-0002](../adr/0002-backend-boundaries.md)

## Purpose

Begin the Rust-native Mega Drive production backend under the accepted staged-hybrid
strategy. Prove a real 68000 reset/execute/VDP/interrupt/snapshot/fork path behind
the existing backend and session contracts, while keeping the synthetic backend as
the contract validator.

## Scope

- `chronofork-megadrive` crate with a minimal documented 68000 subset.
- Big-endian ROM window and 64 KiB work RAM.
- Reset vectors, status register and a small flag model.
- Minimal VDP status/control/VRAM port window behind the bus.
- Deterministic instruction-quantum VBlank/HBlank interrupt scheduler and 68000
  interrupt entry.
- Backend contract implementation with versioned snapshots, state hashing and
  device state (VDP + scheduler) in payloads and hashes.
- Session integration: create, step, snapshot, fork, memory isolation, restore.
- Independently authored synthetic ROM and CLI demo.
- Tests and documentation of exact limitations.

## Out of scope

- Full 68000 instruction set, prefetch or cycle accuracy.
- VDP rendering, DMA, CRAM/VSRAM, raster timing or audio.
- YM2612, PSG or audio.
- Z80, timer interrupts, `RTE`, SR mask gating, controllers, mapper/SRAM.
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
| E2-09 | Add VDP status/control and a minimal VRAM window | Done |
| E2-10 | Add deterministic interrupt/tick scheduler | Done |
| E2-11 | Add ROM header/region/config identity and cartridge loader | Planned |
| E2-12 | Add external-oracle differential traces | Blocked on Ninja/clang/libpng approval |
| E2-13 | Expand instruction/addressing coverage with tests | In progress |
| E2-14 | Integrate VDP + interrupts into backend, snapshots and CLI | Done |
| E2-15 | Wire VINT-pending/controller acknowledge and implement `RTE`/mask gating | Planned |

## Acceptance criteria for this slice

1. Workspace compiles with the new crate.
2. Reset vectors load `SP` and `PC`; `SR = 0x2700`.
3. The synthetic ROM executes deterministically and computes the expected value.
4. Snapshot/restore round-trips state and rejects configuration mismatch.
5. Session fork/restore/isolation works on the real core.
6. Invalid opcodes, unmapped access, odd addresses and ROM writes are typed errors.
7. Multi-byte writes are atomic with respect to invalid ranges.
8. Capabilities report no unimplemented rendering/audio/tracing features.
9. VDP ports `0xC00000..0xC0000F` are mapped through the bus, while PSG/Z80/
   controller windows remain explicit errors.
10. A synthetic program can write a VDP register and round-trip VRAM through the
    data port.
11. HBlank/VBlank requests are generated deterministically from instruction ticks,
    are inspectable, and are delivered through the 68000 interrupt entry API with
    VBlank priority.
12. Snapshot payloads, state hashes and restore cover VDP and scheduler state;
    old payload versions are rejected.
13. `cargo fmt`, check, Clippy, tests and rustdoc pass.
14. CLI `megadrive demo` exits 0 and demonstrates VDP status, interrupt state and
    fork/restore without claiming rendering or audio.

Criteria 1–14 pass for the implemented slice; the epic remains open for E2-11,
E2-12, E2-13 and E2-15.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Subset mistaken for a complete emulator | Misleading users | Explicit naming, docs, capabilities and CLI text |
| Wrong 68000 decoding | Silent bad execution | Unit tests, differential tests next, errors over skipping |
| Snapshot state missing future devices | Broken restore/fork | Versioned payload v2; device blobs validated before commit |
| Tick = instruction conflated with cycles | False timing claims | Documented; scheduler quantum is an explicit model parameter |
| Interrupt mask not enforced | Handler behavior differs from hardware | Documented limitation; `RTE` and mask work scheduled as E2-15 |
| VDP status overclaimed | Users expect VINT/rendering | Only VBlank/FIFO-empty can change; other bits documented constants |
| Oracle build unavailable | No differential evidence yet | Blocked item; request package approval |

## Definition of Done (slice)

All acceptance criteria pass, docs state exact limits, no ROMs/upstream code are
committed, and the next increment is prioritized.
