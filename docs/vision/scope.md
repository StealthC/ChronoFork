# Scope

## Delivered scope (M0–M5)

Bootstrap foundation (M0) delivered:

- Rust 2024 workspace with a small, explicit crate graph.
- Generic IDs, virtual ticks, input frames and SHA-256 state digests.
- Capability-based backend contract with versioned backend snapshots.
- Independent session lifecycle: create, step, pause, resume, reset, close.
- Snapshot creation, serialization, checksum validation, restore and import.
- Fork from a checkpoint or from current state, with lineage.
- Session comparison, memory isolation and deterministic replay tests.
- Synthetic deterministic backend (explicitly not Mega Drive hardware).
- Minimal CLI with `demo` and `backend describe`.
- Official documentation, licensing policy, roadmap and EPIC-0001.

Delivered after the bootstrap:

- **Minimal hardware core (M1).** A documented 68000 subset, ROM and work RAM, a
  VDP status/control/VRAM port window, a deterministic instruction-quantum
  VBlank/HBlank scheduler with SR-mask gating, `RTE` and VINT
  pending/acknowledge, a bounded debug trace and three-ROM oracle differential
  evidence.
- **Durable sessions (M2).** Versioned on-disk checkpoints, named metadata, a
  cooperative writer lock, storage-backed rehydration and the
  `chronofork session save/load/list/delete` CLI.
- **TAS and deterministic replay (M3).** Canonical v1/v2 movie formats,
  record/play/verify with exact first divergence, ranged replay and anchors,
  branch comparison, rewind and `tas studio`, rerecord/branch editing, and
  real-hardware input TAS with the TH-high 3-button controller subset on ports
  1–2 (`--rom`).
- **Debugging depth (M4).** PC/range/conditional breakpoints, byte-granular
  watchpoints, event breakpoints and logpoints, bounded register/memory/
  disassembly views and Mega Drive CPU/VDP/interrupt/bus inspectors behind
  `chronofork debug`. M4 was verified on `agent/m4-debugging-depth` at `6d30243`
  and is merged to `main` at `4c595a2`.
- **Tracing and scripting (M5).** A host-provided `CFTRACE1` v1 trace format for
  instruction, bus-write, VDP-port, controller-read and interrupt-phase records with
  emulated ticks, triggered/circular capture, typed reject-only import, queries,
  first-divergence comparison and one-way TSV export (`chronofork trace`); and a
  purpose-built deterministic script DSL with explicit limits, a canonicalized capability
  sandbox and a byte-deterministic report (`chronofork script`). M5 is implemented and
  independently verified on `agent/m5-tracing-scripting` at base `84f1476` (final T12
  PASS, 2026-11-16) and is integration-pending, not merged. No host-language engine is
  embedded; hardware trace sources, TSV import, GUI/MCP transport and process isolation
  remain deferred.

Milestone status and integration state are recorded once in the
[milestone status table](../roadmap/milestones.md); this page does not duplicate
them.

## Next capability scope (not yet delivered)

- **Agent-native interfaces (M6).** HTTP and MCP over the same application
  services, with a typed tool surface, ownership and permissions.
- **Human-facing GUI (M7).** Video, audio, frame pacing and debugger/TAS panels
  that attach to and detach from sessions without owning canonical state.
- **Multi-backend expansion (M8).** A second backend behind the same capability
  model, with honest capability differences and cross-backend differential tests.

## Hardware-depth tracks (parallel, not stages)

Hardware completeness is tracked outside the capability stages, so a milestone
can be complete while the machine model is still narrow. Details and checkpoints
live in the [milestones](../roadmap/milestones.md) document.

- **H1 — CPU/instruction completeness.** Broader 68000 instruction and addressing
  coverage, privilege enforcement, separate user/supervisor stacks and the
  remaining exception groups.
- **H2 — Video/VDP/DMA.** Rendering and planes, CRAM/VSRAM, DMA, raster and H/V
  timing, VDP interrupt-enable/acknowledge logic and FIFO behaviour.
- **H3 — Audio/Z80.** YM2612, PSG, the Z80 and bus arbitration.
- **H4 — Cartridge/controllers/timers.** ROM header/region/TMSS/checksum, mapper
  and SRAM, the full controller protocol (A/Start, TH latch, 6-button, port 3,
  analog/keyboard) and timer interrupts.
- **H5 — Accuracy/evidence.** A broader differential corpus, positive
  interrupt-delivery and timing evidence, cycle accuracy, cross-platform
  determinism and performance work after correctness.

## Explicitly outside the current scope

- A complete Mega Drive emulator.
- Cycle-accurate CPU/VDP/audio claims without tests.
- Commercial ROMs, BIOS/firmware images or game assets.
- Multiplayer networking, cloud execution or remote session infrastructure.
- A production GUI, installer or release pipeline.
- Process isolation, copy-on-write forks or parallel session execution.
- Any reuse of BizHawk code or GPL-incompatible components.
- A final CI provider decision or remote repository publication.

## Scope rules

- New work must preserve the dependency direction: core ← backend-api ← sessions ←
  application transports.
- A capability must be backed by tests before it is advertised.
- A major backend, license or infrastructure decision requires an approved ADR or
  explicit user approval.
- Temporary experiments belong in `.planning/` or ignored research paths, not in
  official docs or tracked fixtures.
