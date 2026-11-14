# EPIC-0002 — Minimal Mega Drive 68000 Slice

- **Status:** In progress (68000 + ROM/RAM + VDP port + interrupt-scheduler slices
  implemented; bounded instruction trace and final oracle differential evidence
  recorded with both synthetic ROMs passing; E2-13 absolute-long `MOVE` decoding
  repair landed while broader instruction/addressing coverage remains in progress;
  E2-15 interrupt-mask/`RTE` work and remaining hardware pending)
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
- Bounded, debug-only instruction trace and a `megadrive trace` CLI.
- External-oracle differential evidence for independently authored synthetic ROMs.
- Tests and documentation of exact limitations.

## Out of scope

- Full 68000 instruction set, prefetch or cycle accuracy.
- VDP rendering, DMA, CRAM/VSRAM, raster timing or audio.
- YM2612, PSG or audio.
- Z80, timer interrupts, `RTE`, SR mask gating, controllers, mapper/SRAM.
- ROM header/region detection or a commercial ROM loader.
- Cycle/timing equivalence, interrupt-behaviour equivalence, game compatibility and
  full-hardware emulation.
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
| E2-12 | Add external-oracle differential traces | Done — post-repair PASS 2/2: `rom_alu` 99/99 and `rom_mem_vdp` 51/51 projected addresses, projection digests equal, final states equal, negative controls 2/2 |
| E2-13 | Expand instruction/addressing coverage with tests | In progress — absolute-long `MOVE` load decoding repaired (`0x..39`; `0x..38` remains typed illegal) with regression tests; broader instruction/addressing coverage remains future work |
| E2-14 | Integrate VDP + interrupts into backend, snapshots and CLI | Done |
| E2-15 | Wire VINT-pending/controller acknowledge and implement `RTE`/mask gating | Planned |

## Differential method and controlled conditions

The final evidence is recorded in
[Oracle differential instruction-trace evidence](../research/oracle-differential-evidence.md).
The harness runs a pinned Exodus headless oracle and ChronoFork's trace over the
same generated ROM, projects both to the executed-address sequence before a marker
self-loop, asserts address equality, and reports the first divergence, record counts
and projection digests. Every ChronoFork projected opcode word is also checked
against the ROM word at that address.
It also compares selected final state (D0–D7, A0–A7, PC, effective SR fields and 64
work-RAM bytes) through the oracle's MCP register/memory reads, and runs a negative
control that must fail at a deliberately corrupted index. Oracle `cycle`/`time`
columns and ChronoFork ticks are **not** compared.

Controlled conditions:

- **Interrupt-free window:** the oracle honours reset SR mask 7, while ChronoFork
  does not gate interrupts with the SR mask yet (E2-15). The harness passes an
  explicit `--instructions-per-line 1000000`; the CLI default quantum of 64 is
  unchanged. Interrupt delivery is therefore not part of the evidence.
- **Marker-bounded windows:** comparisons stop before a known self-loop, removing
  frame-length and loop-count differences.
- **Explicit register initialisation:** both ROMs initialise every compared
  data/address register they use and the reset vector sets the same stack pointer,
  because the oracle's reset D/A values are undefined.
- **Effective SR only:** raw SR words differ (`0x7FE0` vs `0x2700`); only IPM, S and
  CCR are compared.
- **VDP writes only:** VDP control/data-port writes are part of the instruction
  stream; VDP data-port reads are excluded because oracle prefetch differs from the
  minimal VDP model.

At the final post-repair revision both ROMs pass: `rom_alu` has 2375 oracle total /
99 projected records and 99/99 ChronoFork projected records with projection SHA-256
`2b6f191cf42101174b7b64b2272cf6277c72ff62a9608eeb23ca977fa70df569` on both sides;
`rom_mem_vdp` has 2363 / 51 and 51/51 with projection SHA-256
`ba4f39489fa56b5fa068af4f4bd21e5aaf311a8138df22231f6c0e04bfe62b47` on both sides.
Both final states match (D0–D7, A0–A7, PC, effective SR and 64 work-RAM bytes), and
the negative controls catch indices 49 and 25 with exit 1. Every ChronoFork
projected opcode word also matches the ROM word at its address.

The pre-repair run is preserved as history: `rom_alu` passed, while `rom_mem_vdp`
diverged at projected index 28 (`0x00027A`, opcode `0x2239`,
`MOVE.L $00FF0020,D1`) because the core decoded the absolute-**word** form
(`0x..38`) for absolute-long loads. The real 68000 encoding is `0x..39`. E2-13
repaired the decoder (`0x1039`/`0x2039`/`0x3039` now decode; `0x1038`/`0x2038`/
`0x3038` remain typed illegal) with regression tests, and the post-repair run is
the final evidence above. This establishes instruction-address and selected-state
agreement for two synthetic ROMs; it is **not** cycle, timing, interrupt,
rendering, audio, Z80, DMA or game-compatibility equivalence.

## Acceptance criteria for this slice

1. Workspace compiles with the new crate.
2. Reset vectors load `SP` and `PC`; `SR = 0x2700`.
3. The synthetic ROM executes deterministically and computes the expected value.
4. Snapshot/restore round-trips state and rejects configuration mismatch.
5. Session fork/restore/isolation works on the real core.
6. Invalid opcodes, unmapped access, odd addresses and ROM writes are typed errors.
7. Multi-byte writes are atomic with respect to invalid ranges.
8. Capabilities report no unimplemented rendering/audio/tracing features; the
   concrete debug trace is not advertised as a generic `tracing` capability.
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
E2-13's broader coverage and E2-15. E2-12 is done: the post-repair differential
harness passes both synthetic ROMs, final states match, negative controls pass, and
the M1 differential-evidence milestone item is ticked on that evidence. E2-13's
absolute-long `MOVE` decoding repair has landed with regression tests, but broader
instruction/addressing coverage remains future work.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Subset mistaken for a complete emulator | Misleading users | Explicit naming, docs, capabilities and CLI text |
| Wrong 68000 decoding | Silent bad execution | Unit tests plus the differential harness; the E2-13 absolute-long repair added regression tests for the `0x..39` loads and `0x..38` rejection, while broader decoding coverage remains future work |
| Snapshot state missing future devices | Broken restore/fork | Versioned payload v2; device blobs validated before commit |
| Tick = instruction conflated with cycles | False timing claims | Documented; scheduler quantum is an explicit model parameter |
| Interrupt mask not enforced | Handler behavior differs from hardware | Documented limitation; `RTE` and mask work scheduled as E2-15 |
| VDP status overclaimed | Users expect VINT/rendering | Only VBlank/FIFO-empty can change; other bits documented constants |
| Differential evidence overclaimed as hardware equivalence | False accuracy claims | Compare instruction addresses, ROM-byte consistency and selected state only; oracle cycle/time ignored; limits stated explicitly |
| Differential evidence limited to two synthetic ROMs | Overgeneralized conformance claims | Keep scope explicit; broader instruction/addressing coverage remains future work beyond the E2-13 repair |

## Definition of Done (slice)

All acceptance criteria pass, docs state exact limits, no ROMs/upstream code are
committed, the E2-12 pre- and post-repair evidence is recorded with its controlled
conditions and negative controls, and the next increment is prioritized.
