# EPIC-0002 — Minimal Mega Drive 68000 Slice

- **Status:** In progress (68000 + ROM/RAM + VDP port + interrupt-scheduler slices
  implemented; E2-13 absolute-long `MOVE` decoding repair landed; E2-15 SR-mask
  gating, `RTE` and VDP VINT pending/acknowledge implemented with backend, CLI and
  snapshot tests; the TH-high 3-button controller reads on ports 1–2 were added by
  the M3 real-hardware input TAS increment; the current Phase 8.2 harness covers
  eleven synthetic ROMs and passes 11/11 address, per-instruction cycle, final-state
  and negative-control comparisons, while the historical three-ROM 3/3 run and its
  default-quantum masked-ROM negative gating control are preserved as history.
  Broader addressing coverage, E2-11 ROM identity and the remaining hardware are
  still open, tracked in H1–H5 below)
- **Milestone:** M1 (core slice, complete and merged); M3 delivered the controller
  input TAS extension and M4 added the debugging surface (merged to `main` at
  `4c595a2`, verified on `agent/m4-debugging-depth` at `6d30243`). The epic's open
  hardware work is tracked in the parallel hardware-depth tracks H1–H5, not in a new
  milestone.
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

- `chronofork-megadrive` crate with the documented MC68000 instruction set.
- Big-endian ROM window and 64 KiB work RAM.
- Reset vectors, status register and a small flag model.
- Minimal VDP status/control/VRAM port window behind the bus.
- Deterministic master-cycle VBlank/HBlank interrupt scheduler and 68000 interrupt
  entry, including SR interrupt-priority-mask gating, `RTE` return and VDP VINT
  pending/acknowledge wiring.
- Backend contract implementation with versioned snapshots, state hashing and
  device state (VDP + scheduler) in payloads and hashes.
- Session integration: create, step, snapshot, fork, memory isolation, restore.
- Independently authored synthetic ROMs and CLI demo.
- Bounded, debug-only instruction trace and a `megadrive trace` CLI.
- External-oracle differential evidence for eleven independently authored synthetic
  ROMs on the master-cycle timeline, including address, per-instruction cycle,
  final-state and negative-control comparisons; the historical default-quantum
  masked-ROM control is preserved.
- Tests and documentation of exact limitations.

## Out of scope

- Full 68000 instruction set, prefetch or cycle accuracy.
- VDP rendering, DMA, CRAM/VSRAM, raster timing or audio.
- YM2612, PSG or audio.
- Z80, timer interrupts, the controller protocol beyond the delivered TH-high
  3-button reads, mapper/SRAM, other 68000 exception groups and privilege
  enforcement.
- ROM header/region detection or a commercial ROM loader.
- Cycle/timing equivalence, interrupt-timing or interrupt-delivery equivalence
  beyond the implemented mask-gating tests and the masked-ROM negative control,
  game compatibility and full-hardware emulation.
- GUI, HTTP or MCP. TAS movies were delivered by M3 on this slice; this epic does
  not own TAS UI, transports or import/export adapters.

## Workstreams and tasks

| ID | Task | Status |
|---|---|---|
| E2-01 | Create crate and workspace wiring | Done |
| E2-02 | Implement ROM/RAM big-endian bus with atomic writes | Done |
| E2-03 | Implement documented 68000 instruction coverage and flags | Done |
| E2-04 | Implement reset vectors and SR initialization | Done |
| E2-05 | Implement backend contract, snapshots and state hash | Done |
| E2-06 | Integrate with sessions and prove fork/restore isolation | Done |
| E2-07 | Add synthetic ROM and CLI `megadrive demo` | Done |
| E2-08 | Document scope, limitations and next increments | Done |
| E2-09 | Add VDP status/control and a minimal VRAM window | Done |
| E2-10 | Add deterministic interrupt/tick scheduler | Done |
| E2-11 | Add ROM header/region/config identity and cartridge loader | Planned |
| E2-12 | Add external-oracle differential traces | Done — historical post-repair two-ROM PASS 2/2 (`rom_alu` 99/99, `rom_mem_vdp` 51/51, projection digests equal, final states equal, negative controls 2/2); E2-15 extended it to the historical 3-ROM masked control. The current Phase 8.2 harness covers eleven synthetic ROMs at 11/11 address/cycle/final-state and negative controls. |
| E2-13 | Expand instruction/addressing coverage with tests | In progress — absolute-long `MOVE` load decoding repaired (`0x..39`; `0x..38` remains typed illegal) with regression tests; broader instruction/addressing coverage remains future work |
| E2-14 | Integrate VDP + interrupts into backend, snapshots and CLI | Done |
| E2-15 | Wire VINT-pending/controller acknowledge and implement `RTE`/mask gating | Done for the modelled slice — delivery is gated by the 68000 rule (level 7 always; other levels strictly above the SR IPM); masked requests stay pending with their issue tick and coalesce per level; only the delivered level is acknowledged; `RTE` restores raw SR/IPM/CCR and PC; privileged `MOVE.W #imm,SR` writes the raw SR for mask control (user-mode writes take vector 8; covered by `privileged_write_sr_faults_in_user_mode`); VDP VINT-pending status tracks the pending VBlank request (set while pending, even masked; cleared on delivery; status reads never acknowledge); snapshots/state hashes cover pending state and restore recomputes the derived latch; CLI demo and masked default-quantum differential control pass. Controller-port interrupt acknowledge remains absent because no controller interrupt source is modelled |

## Hardware-depth mapping (H1–H5)

The epic remains open after M1 because the machine model is deliberately narrow. The
remaining work is tracked in the parallel hardware-depth tracks defined in the
[milestones](../roadmap/milestones.md), not as an open milestone item, so no verified
milestone has to be reopened:

| Open work | Track |
|---|---|
| E2-13 broader instruction/addressing coverage; the remaining 68000 exception groups (including guest bus error and trace) | H1 |
| VDP rendering, planes, sprites and scrolling; CRAM/VSRAM; DMA; raster and H/V timing; VDP interrupt-enable/acknowledge and FIFO behaviour | H2 |
| YM2612, PSG, the Z80 and bus arbitration | H3 |
| E2-11 ROM header/region/TMSS/checksum and cartridge loader; mapper/SRAM; timer interrupts; the controller protocol beyond the delivered TH-high 3-button subset | H4 |
| Broader differential corpus, positive interrupt-delivery evidence, cycle accuracy and cross-platform determinism | H5 |

Delivered and not part of that open set: the documented MC68000 instruction set,
ROM/RAM, the VDP port window and VRAM, the master-cycle interrupt scheduler with
SR-mask gating/`RTE`/VINT pending-acknowledge, the bounded debug trace, the eleven-ROM
differential harness and the TH-high 3-button controller reads (M3). The historical
three-ROM differential run remains preserved as history. M4 later added the optional
non-canonical debugging surface; it was verified on `agent/m4-debugging-depth` at
`6d30243` and is merged to `main` at `4c595a2`.

## Differential method and controlled conditions

The current evidence is recorded in
[Oracle differential instruction-trace evidence](../research/oracle-differential-evidence.md).
The harness runs a pinned Exodus headless oracle and ChronoFork's trace over the
same generated ROM, projects both to the executed-address sequence before a marker
self-loop, asserts address equality, and reports the first divergence, record counts
and projection digests. Every ChronoFork projected opcode word is also checked
against the ROM word at that address.
It also compares selected final state (D0–D7, A0–A7, PC, effective SR fields and 64
work-RAM bytes) through the oracle's MCP register/memory reads, runs a negative
control that must fail at a deliberately corrupted index, and records the per-ROM
scheduler quantum plus the masked-ROM gating control. Oracle `cycle`/`time` columns
and ChronoFork ticks are **not** compared.

Controlled conditions:

- **Explicit interrupt-free window for two ROMs:** the oracle honours reset SR mask 7;
  `rom_alu` and `rom_mem_vdp` also pass an explicit
  `--instructions-per-line 1000000`, so no HBlank/VBlank request is raised in their
  short measured windows. This is a controlled workaround, not interrupt equivalence.
- **Default-quantum masked window:** `rom_masked` omits `--instructions-per-line` and
  runs at the CLI default quantum of 64 (read back from the state file). It contains
  no SR-writing opcode, so both sides stay at mask 7; its 155 executed instructions
  cross HBlank boundaries at ticks 64 and 128, no handler executes, and the VINT /
  delivery behavior is not compared. This is **negative evidence for mask gating**,
  not a positive comparison of interrupt delivery.
- **Marker-bounded windows:** comparisons stop before a known self-loop, removing
  frame-length and loop-count differences.
- **Explicit register initialisation:** every ROM initialises every compared
  data/address register it uses and the reset vector sets the same stack pointer,
  because the oracle's reset D/A values are undefined.
- **Effective SR only:** raw SR words differ (`0x7FE0` vs `0x2700`); only IPM, S and
  CCR are compared.
- **VDP writes only:** VDP control/data-port writes are part of the instruction
  stream; VDP data-port reads are excluded because oracle prefetch differs from the
  minimal VDP model.

The historical three-ROM harness run was **PASS 3/3**: 3/3 projected address streams,
3/3 final states, 3/3 negative controls and 3/3 scheduler controls (masked gating
1/1), exit 0. The current Phase 8.2 harness runs eleven synthetic ROMs and passes
11/11 address, per-instruction cycle, final-state and negative controls.

- `rom_alu`: 2375 oracle total / 99 projected records and 99/99 ChronoFork projected
  records, projection SHA-256
  `2b6f191cf42101174b7b64b2272cf6277c72ff62a9608eeb23ca977fa70df569` on both sides,
  negative control index 49.
- `rom_mem_vdp`: 2363 / 51 and 51/51, projection SHA-256
  `ba4f39489fa56b5fa068af4f4bd21e5aaf311a8138df22231f6c0e04bfe62b47` on both sides,
  negative control index 25.
- `rom_masked`: 2388 / 155 and 155/155 at the then-default instruction quantum
  (superseded by the master-cycle timeline), projection
  SHA-256 `02891e15765a342496196564e17d1649f10d39a069b4494fa96d7fb11ca3cd11` on both
  sides, negative control index 77, SR mask 7 on both sides, 2 HBlank boundaries
  crossed and zero handler entries.

All final states match (D0–D7, A0–A7, PC, effective SR and 64 work-RAM bytes), and
every ChronoFork projected opcode word matches the ROM word at its address.

The pre-repair run is preserved as history: `rom_alu` passed, while `rom_mem_vdp`
diverged at projected index 28 (`0x00027A`, opcode `0x2239`,
`MOVE.L $00FF0020,D1`) because the core decoded the absolute-**word** form
(`0x..38`) for absolute-long loads. The real 68000 encoding is `0x..39`. E2-13
repaired the decoder (`0x1039`/`0x2039`/`0x3039` now decode; `0x1038`/`0x2038`/
`0x3038` remain typed illegal) with regression tests. The earlier canonical
two-ROM Phase-10 evidence at the explicit 1,000,000-instruction quantum is
preserved unchanged under ignored `.research/evidence/before-e2-15/` as the
controlled-condition history. The historical three-ROM evidence establishes
instruction-address and selected-state agreement; the current Phase 8.2 harness
also compares per-instruction 68000 cycle costs. Neither is interrupt-timing,
interrupt-delivery, rendering, audio, Z80, DMA or game-compatibility equivalence.

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
15. Interrupt delivery is gated by the 68000 rule: level 7 is always admissible and
    every other level only when strictly above the `SR` interrupt-priority mask.
    Masked HBlank/VBlank requests stay pending with their original issue tick, are
    not acknowledged or dropped while masked, are delivered at a later instruction
    boundary after the mask is lowered, and VBlank is delivered before HBlank.
16. `RTE` restores the raw `SR` (IPM, S and CCR) and PC from the six-byte exception
    frame and advances A7; `MOVE.W #imm,SR` replaces the raw `SR`, including the
    interrupt mask.
17. The VDP VINT-pending status bit is derived from the scheduler's pending VBlank
    request: set while pending (even when masked), cleared only on delivery, and
    never cleared by a status-port read. Snapshots/restores preserve pending
    requests and recompute the derived bit; state hashes include pending state.
18. The historical differential harness reported PASS 3/3 for three synthetic ROMs, including
    the masked ROM at the then-default instruction quantum with two HBlank boundaries crossed
    at mask 7 and no handler execution, while the earlier explicit-1,000,000-quantum
    two-ROM evidence remains preserved as controlled-condition history.

Two later increments extend the delivered slice without rewriting these criteria.
M3 mapped the controller data registers `0x00A10003`/`0x00A10005` for reads (TH-high
3-button subset, ports 1–2), so criterion 9's "controller windows remain explicit
errors" now applies to every other controller access. M4 added optional
non-canonical debug observation ([ADR-0010](../adr/0010-debugging-depth.md)). Neither
is part of this epic's exit criteria.

Criteria 1–18 pass for the implemented slice; the epic remains open for E2-11, broader
addressing coverage and the remaining hardware/evidence gaps (rendering and frame
timing, VDP data reads, DMA, audio, Z80, timers, the remaining controller protocol and
the guest bus-error/trace exception groups). Privilege enforcement and the documented
MC68000 instruction set are delivered.
E2-12's two-ROM differential run and E2-15's 3/3 masked-ROM control are historical;
the current Phase 8.2 harness covers eleven synthetic ROMs at 11/11 address, cycle,
final-state and negative controls, and the M1 differential-evidence item is ticked on
that evidence. E2-15's mask-gating, `RTE`, VINT
pending/acknowledge and privileged `MOVE.W #imm,SR` behavior is covered by Rust
tests plus the CLI demo; the differential evidence remains negative gating evidence,
not an interrupt-delivery or interrupt-timing comparison.

Representative E2-15 tests: `interrupt_priority_mask_selects_only_levels_above_the_mask`,
`interrupt_loads_the_accepted_level_into_ipm`,
`move_immediate_to_sr_sets_the_raw_status_register_and_mask`,
`rte_restores_raw_sr_pc_and_advances_the_stack_pointer`,
`masked_hblank_is_retained_until_the_program_lowers_the_mask`,
`vblank_handler_ipm_six_blocks_hblank_reentry_until_rte`,
`masked_vblank_keeps_vint_pending_across_many_steps_without_acknowledging`,
`status_port_reads_do_not_acknowledge_masked_requests`,
`pending_vblank_state_survives_snapshot_restore`,
`vint_pending_bit_is_driven_and_status_reads_never_clear_it`,
`snapshot_excludes_the_derived_vint_latch`,
`megadrive_demo_runs_the_minimal_68000_slice`,
`megadrive_demo_output_is_deterministic`,
`scheduler_config_is_video_only`,
`default_master_cycle_timeline_is_reported` and
`v4_snapshot_round_trips_master_cycles_stacks_and_cpu_cycles`. `cargo test --workspace` is the source
of truth for the current count.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Subset mistaken for a complete emulator | Misleading users | Explicit naming, docs, capabilities and CLI text |
| Wrong 68000 decoding | Silent bad execution | Unit tests plus the differential harness; the E2-13 absolute-long repair added regression tests for the `0x..39` loads and `0x..38` rejection, while broader decoding coverage remains future work |
| Snapshot state missing future devices | Broken restore/fork | Versioned payload v4; device blobs validated before commit; pending interrupt state and the derived VINT latch are covered by tests |
| Tick = instruction conflated with cycles | False timing claims | Canonical time is the master-cycle timeline (instruction cost x 7); functional prefetch, bus contention and interrupt latency remain out of scope |
| Mask gating mistaken for interrupt timing/delivery equivalence | False hardware-equivalence claims | Unit/backend tests cover the mask rule, pending retention, acknowledge-per-level, VBlank priority and no nesting; the differential masked-ROM pass is documented as negative gating evidence only |
| Privilege enforcement regression | `MOVE.W #imm,SR`/`RTE` could run in the wrong mode | Implemented for the documented system instructions; `privileged_write_sr_faults_in_user_mode` and the RTE/stack tests cover the behaviour. |
| VDP status overclaimed | Users expect rendering or full VINT control | Only VBlank, FIFO-empty and the scheduler-derived VINT-pending latch can change; VDP interrupt-enable registers, VDP-side acknowledge logic and other bits are documented constants |
| Differential evidence overclaimed as hardware equivalence | False accuracy claims | Compare instruction addresses, per-instruction 68000 cycle costs, ROM-byte consistency and selected state; the oracle's absolute cycle base, time column and interrupt timing are not compared; limits stated explicitly |
| Differential evidence historically limited to three synthetic ROMs | Overgeneralized conformance claims | Keep scope explicit; the current harness covers eleven synthetic ROMs and the historical masked ROM remains a negative gating control, not a positive interrupt comparison; broader addressing/hardware coverage remains future work |

## Definition of Done (slice)

All acceptance criteria pass, docs state exact limits, no ROMs/upstream code are
committed, the E2-12 pre- and post-repair evidence is recorded with its controlled
conditions and negative controls, the E2-15 mask-gating/`RTE`/VINT behavior is
covered by the backend, CLI and snapshot tests and demonstrated by the CLI demo, the
historical three-ROM differential evidence (including the default-quantum masked-ROM
negative gating control) is recorded with every number/digest reproducible, the current
eleven-ROM 11/11 master-cycle differential is recorded, the earlier
explicit-1,000,000-quantum evidence is preserved as history, and the next increment is
prioritized.
