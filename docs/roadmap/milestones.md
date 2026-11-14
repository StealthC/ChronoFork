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
- [x] Differential trace evidence against the approved reference — current PASS 3/3:
  `rom_alu` 99/99 projected addresses (projection SHA-256 `2b6f191c…`),
  `rom_mem_vdp` 51/51 (`ba4f3948…`) and `rom_masked` 155/155 (`02891e15…`) at the
  CLI default quantum 64 with the flag omitted. All final states equal; negative
  controls caught indices 49, 25 and 77 with exit 1; scheduler/gating control 3/3
  (masked gating 1/1). `rom_masked` crosses two HBlank boundaries while the SR mask
  stays 7 and enters no handler, so it is negative gating evidence, not an
  interrupt-delivery comparison. `rom_alu`/`rom_mem_vdp` still use the explicit
  1,000,000-instructions-per-scanline controlled window; the pre-E2-15 two-ROM
  evidence at that quantum is preserved under ignored
  `.research/evidence/before-e2-15/`. Pre-repair FAIL and the E2-13 repair are
  recorded in
  [Oracle differential evidence](../research/oracle-differential-evidence.md).
- [x] Capabilities report matches reality for the implemented slice.

## M2 — Durable sessions

Design status: Fase A produced [ADR-0003](../adr/0003-durable-session-storage.md)
(Accepted 2026-11-14) and the [M2 acceptance
plan](../quality/m2-durable-sessions-acceptance.md). **M2 is not complete.** No durable
storage, named metadata, persistence ownership/lock, storage-backed rehydration path or
`chronofork session` CLI command exists, and no M2 acceptance test has been executed.
Fase B requires a separately approved implementation plan; the owner decisions were
approved on 2026-11-14.

- [ ] Checkpoints persist across process restarts.
  - [ ] Versioned storage root and marker (`CFSTORE1`, layout v1) with an explicit
        caller-supplied root and no implicit creation.
  - [ ] Snapshot publish writes the canonical `Snapshot::to_bytes()` envelope verbatim
        through the tmp+fsync+rename sequence; a crash never exposes a partial snapshot
        as committed.
  - [ ] A new process opens the root, validates the object and restores the recorded
        tick and state hash.
  - [ ] Staging, tombstone and orphan objects are reported as typed anomalies and are
        never repaired or silently loaded.
  - [ ] Restart, crash-recovery and canonical-bytes acceptance tests are executed and
        recorded.
- [ ] Named snapshots and metadata.
  - [ ] One versioned metadata record round-trips name, logical creation sequence,
        lineage keys and the mirrored envelope fields.
  - [ ] Metadata-only rename changes no checkpoint bytes, state hash or replay result.
  - [ ] Listing is deterministic; retention is caller-driven and deletion is explicit;
        no operation performs automatic garbage collection.
  - [ ] Metadata and retention acceptance tests are executed and recorded.
- [ ] Version compatibility policy tested.
  - [ ] Reject-only matrix for storage layout, metadata, outer envelope, backend
        envelope and backend payload.
  - [ ] Corrupt, truncated, checksum-bad, swapped and identity-mismatched input produces
        typed errors; no silent migration or reinterpretation.
  - [ ] Compatibility acceptance tests are executed and recorded.
- [ ] Concurrency/ownership policy implemented at the service boundary.
  - [ ] One writer per durable session through an explicit lock/lease; contention is a
        typed error with no busy-wait or silent retry.
  - [ ] Stale and corrupt lock recovery is explicit and owner-checked; no lock is stolen
        automatically.
  - [ ] Readers are lock-free and never observe a torn checkpoint.
  - [ ] There is no hidden global lock; only the bounded root key-allocation lock is
        shared, and it is never held during emulation, serialization or checkpoint I/O.
  - [ ] Ownership and concurrency acceptance tests are executed and recorded.
- [ ] No mutable state shared across sessions.
  - [ ] Rehydrated sessions own independent mutable state; forks never share it.
  - [ ] Publishing or deleting one session/snapshot never changes another's checkpoint
        bytes, state hash or lineage.
  - [ ] In-process and durable isolation acceptance tests are executed and recorded.

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

**M1 is complete.** The differential item is ticked on the current PASS 3/3
evidence: three synthetic ROMs match the pinned oracle's projected address streams,
selected final state and negative controls, and the masked-ROM control at the CLI
default quantum 64 crosses two HBlank boundaries with the SR mask at 7 and no handler
entry (negative gating evidence, not interrupt-timing/delivery equivalence). The
earlier explicit-1,000,000-quantum two-ROM evidence is preserved as
controlled-condition history. EPIC-0002 E2-15 (SR-mask gating, `RTE`, VINT
pending/acknowledge) is done; the epic remains open for E2-11 (ROM identity), broader
E2-13 instruction/addressing coverage and remaining hardware (rendering, Z80, audio,
DMA, controllers, timers, other exception groups, privilege enforcement). See
[EPIC-0002](../epics/EPIC-0002-megadrive-slice.md),
[oracle differential evidence](../research/oracle-differential-evidence.md) and
[ADR-0002](../adr/0002-backend-boundaries.md).

**M2 is the current milestone. The M2 design is accepted and implementation has NOT
started, so M2 is NOT complete.** Fase A has produced an Accepted
[ADR-0003](../adr/0003-durable-session-storage.md) and a planned, not-yet-executed
[M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md). No durable storage,
named metadata, persistence ownership/lock, storage-backed rehydration path or
`chronofork session` CLI command exists, and no M2 acceptance test has been executed.
The M2 checkboxes above stay unchecked until their acceptance criteria run, pass and are
recorded. Fase B starts only under a separately approved implementation plan.
