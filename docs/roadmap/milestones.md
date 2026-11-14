# Milestones

Milestones are verifiable checkpoints, not dates. A milestone is complete only when
its exit criteria are demonstrated by tests or documented evidence.

## M0 — Bootstrap foundation

- [x] Environment inspected and recorded.
- [x] Persistent planning initialized.
- [x] Reference repositories cloned under ignored paths with exact revisions.
- [x] Rust workspace compiles.
- [x] Synthetic backend + sessions + snapshots + forks implemented.
- [x] Acceptance tests pass.
- [x] Format, check, Clippy and test gates run.
- [x] Official docs and `AGENTS.md` exist.
- [x] Licensing/provenance policy and notices exist.
- [x] Real-backend decision approved (ADR-0002 accepted 2026-11-13).

## M1 — Backend decision and minimal hardware slice

- [x] ADR-0002 approved or replaced.
- [x] Build path for the Rust-native slice reproducible on Linux.
- [x] ROM identity (`config_hash`) and a minimal hardware configuration represented.
- [x] Minimal synthetic ROM executes deterministically on the real backend.
- [x] Snapshot/restore/fork passes on the real backend.
- [x] Minimal VDP status/control and VRAM window.
- [x] Deterministic interrupt/tick scheduler.
- [x] VDP + interrupt scheduler integrated into backend snapshots, state hashes and CLI.
- [x] Differential trace evidence against the approved reference — post-repair PASS
  2/2: `rom_alu` 99/99 projected addresses (projection SHA-256 `2b6f191c…`),
  `rom_mem_vdp` 51/51 (`ba4f3948…`), both final states equal, negative controls
  caught indices 49 and 25 with exit 1. Controlled with an explicit
  1,000,000-instructions-per-scanline quantum (SR mask gating is pending E2-15),
  marker-bounded address projection, effective-SR-only comparison and excluded VDP
  data reads. Pre-repair FAIL and the E2-13 repair are recorded in
  [Oracle differential evidence](../research/oracle-differential-evidence.md).
- [x] Capabilities report matches reality for the implemented slice.

## M2 — Durable sessions

- [ ] Checkpoints persist across process restarts.
- [ ] Named snapshots and metadata.
- [ ] Version compatibility policy tested.
- [ ] Concurrency/ownership policy implemented at the service boundary.
- [ ] No mutable state shared across sessions.

## M3 — TAS and replay

- [ ] Versioned movie format.
- [ ] Record/playback and frame advance.
- [ ] Rerecord and seek via checkpoints.
- [ ] Replay verification with first-divergence reporting.
- [ ] Multi-controller input.

## M4 — Debugging depth

- [ ] Breakpoints and watchpoints.
- [ ] Register/memory/disassembly views.
- [ ] Interrupt/bus/device event observation.
- [ ] Mega Drive-specific inspectors.

## M5 — Tracing and scripting

- [ ] Structured trace format and export.
- [ ] Triggered/circular tracing.
- [ ] Trace comparison.
- [ ] Sandboxed scripting with limits.

## M6 — Agent interfaces

- [ ] HTTP API.
- [ ] MCP Streamable HTTP adapter.
- [ ] Typed tool surface and permissions.
- [ ] CLI/API/MCP semantic parity tests.

## M7 — GUI

- [ ] Video/audio presentation independent of canonical state.
- [ ] Debugger/TAS panels.
- [ ] Multi-session windows.
- [ ] Attach/detach without behavior change.

## M8 — Multi-backend

- [ ] Second backend integrated.
- [ ] Capability differences honored.
- [ ] Cross-backend differential tests.
- [ ] SDK stability policy.

## Current milestone

**M1 is complete.** The differential item is ticked on the post-repair PASS 2/2
evidence: both synthetic ROMs match the pinned oracle's projected address streams,
selected final state and negative controls under the documented controlled
conditions. EPIC-0002 remains open for E2-15 (interrupt-mask/`RTE`) and E2-11, and
Stage 1's broader production-hardware work (rendering, Z80, audio, DMA) is still
pending; M2 is the next milestone but has not started. See
[EPIC-0002](../epics/EPIC-0002-megadrive-slice.md),
[oracle differential evidence](../research/oracle-differential-evidence.md) and
[ADR-0002](../adr/0002-backend-boundaries.md).
