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
plan](../quality/m2-durable-sessions-acceptance.md). Fase B implemented the durable
snapshot repository, named metadata, reject-only compatibility, storage-backed
rehydration, cooperative writer ownership and the `chronofork session
save/load/list/delete` CLI.

Two independent verification rounds (2026-11-15) executed every acceptance criterion
and the full gate on this tree. Round 1 passed M2-A1–A11 and found one medium,
non-blocking CLI lock-scope finding (F1): `save --session` read the latest base before
taking the writer lock. The repair made the continue-and-save workflow acquire the
per-session lock before the read and hold it through rehydrate/advance/capture/publish
(`with_writer_lock` plus `save_snapshot_with_lock`); round 2 re-verified F1 closed with
typed contention, no stale-base double publish and no regression. Raw evidence is in
the ignored `.research/m2-durable-sessions/fase-b/verification/` (round 1 plus
`round2/`) and `.research/m2-durable-sessions/fase-b-repair-f1/`; the durable record is
the [M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md) and the dated
[quality gates](../quality/quality-gates.md) result.

Residual limits: durability is verified only on local Linux filesystems (Windows and
network filesystems unverified); the writer lock is cooperative metadata, not
authentication; holding the lock across long in-memory `--steps` advances lengthens
contention by design; a crashed holder needs explicit owner/generation-checked
recovery; retention is manual with no automatic GC; there is no encryption,
authentication or format migration; the public convenience `save_snapshot` remains
publish-only for third-party callers (the CLI uses the lock-aware workflow); and
`NFR-CONC-003` remains **Partial** because only sequential lock-free-reader evidence
exists — a concurrent publish/read or delete/read interleaving is not separately tested.
The checked M2 items rest on the executed M2-A criteria, not on that NFR clause.

- [x] Checkpoints persist across process restarts.
  - [x] Versioned storage root and marker (`CFSTORE1`, layout v1) with an explicit
        caller-supplied root and no implicit creation.
  - [x] Snapshot publish writes the canonical `Snapshot::to_bytes()` envelope verbatim
        through the tmp+fsync+rename sequence; a crash never exposes a partial snapshot
        as committed.
  - [x] A new process opens the root, validates the object and restores the recorded
        tick and state hash.
  - [x] Staging, tombstone and orphan objects are reported as typed anomalies and are
        never repaired or silently loaded.
  - [x] Restart, crash-recovery and canonical-bytes acceptance tests are executed and
        recorded.
- [x] Named snapshots and metadata.
  - [x] One versioned metadata record round-trips name, logical creation sequence,
        lineage keys and the mirrored envelope fields.
  - [x] Metadata-only rename changes no checkpoint bytes, state hash or replay result.
  - [x] Listing is deterministic; retention is caller-driven and deletion is explicit;
        no operation performs automatic garbage collection.
  - [x] Metadata and retention acceptance tests are executed and recorded.
- [x] Version compatibility policy tested.
  - [x] Reject-only matrix for storage layout, metadata, outer envelope, backend
        envelope and backend payload.
  - [x] Corrupt, truncated, checksum-bad, swapped and identity-mismatched input produces
        typed errors; no silent migration or reinterpretation.
  - [x] Compatibility acceptance tests are executed and recorded.
- [x] Concurrency/ownership policy implemented at the service boundary.
  - [x] One writer per durable session through an explicit lock/lease; contention is a
        typed error with no busy-wait or silent retry.
  - [x] Stale and corrupt lock recovery is explicit and owner-checked; no lock is stolen
        automatically.
  - [x] Readers are lock-free and never observe a torn checkpoint.
  - [x] There is no hidden global lock; only the bounded root key-allocation lock is
        shared, and it is never held during emulation, serialization or checkpoint I/O.
  - [x] Ownership and concurrency acceptance tests are executed and recorded.
- [x] No mutable state shared across sessions.
  - [x] Rehydrated sessions own independent mutable state; forks never share it.
  - [x] Publishing or deleting one session/snapshot never changes another's checkpoint
        bytes, state hash or lineage.
  - [x] In-process and durable isolation acceptance tests are executed and recorded.

## M3 — TAS and replay

Status: the first increment is implemented and independently verified (M3-T4 PASS,
2026-11-15): the canonical v1 `.cfmovie` format ([ADR-0004](../adr/0004-tas-movie-format.md)),
record/playback of a per-tick input sequence, replay verification with first-divergence
reporting, restart and M2-seeded replay, and the `chronofork tas record/play/verify`
CLI. The checked subitems below are backed by the executed [M3 acceptance
plan](../quality/m3-tas-replay-acceptance.md) (M3-A1–A12) and the dated [quality
gates](../quality/quality-gates.md) result. M3 is **not complete**: frame advance,
rewind, seek, rerecord, branch editing, multi-controller/ports/analog/keyboard input,
real-hardware input TAS, import/export adapters, checkpoint-reference movies and
HTTP/MCP/GUI/daemon remain deferred and unchecked.

- [x] Versioned canonical movie format.
  - [x] Single-file `.cfmovie` v1 with magic, format and input-schema versions,
        metadata, an embedded canonical checkpoint, ordered per-step records and a
        trailing SHA-256 (ADR-0004 §2–§4).
  - [x] One reader and one writer in `chronofork_sessions::movie`, built on the shared
        codec and `Snapshot::to_bytes`/`from_bytes` only.
  - [x] Bounded decode with typed rejection for versions, checksum, truncation,
        trailing bytes, limits, metadata, tick/count inconsistency and a corrupt or
        incompatible embedded checkpoint; no allocation before validation.
- [x] Record/playback of a per-tick input sequence on the deterministic synthetic
      backend.
  - [x] Input schema v1: one backend-defined `u32` mask per step with the outcome tick.
  - [x] Recording captures the source checkpoint and executes a separate imported
        session; the source is never advanced or mutated.
  - [x] Playback imports the embedded checkpoint into a fresh session and recomputes
        every tick and state hash.
- [x] Replay verification with first-divergence reporting.
  - [x] Per-step expected `(tick, state hash)` records at 44 bytes per step.
  - [x] The first mismatch reports the exact tick, input index, expected hash and
        observed hash; no resynchronization or later-mismatch substitution.
- [x] Restart replay and M2-seeded record.
  - [x] A second process verifies a movie file with no shared state and no storage
        root.
  - [x] A record seeded by an M2 committed checkpoint verifies without that root.
- [x] Minimal `chronofork tas record|play|verify` CLI.
  - [x] Stable `key=value` blocks, stdout data only, stderr diagnostics, exit 0/1.
  - [x] Process-level record/play/verify and divergence tests.
- [ ] Frame advance, rewind and seek via checkpoints — deferred, not in this
      increment.
- [ ] Rerecord and branch editing — deferred; v1 carries only passive parent-movie
      metadata and the CLI exposes no branch editing.
- [ ] Multi-controller, ports, analog and keyboard input — deferred; v1 is one `u32`
      mask per step and a future input schema version is required.
- [ ] Real-hardware input TAS — deferred; the Mega Drive `step` ignores input.

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

**M1 and M2 are complete. M3 (TAS and replay) is the current milestone: its first
increment is implemented and independently verified (M3-T4 PASS, 2026-11-15), and M3
is not complete.** The canonical v1 `.cfmovie` format, per-tick record/playback on the
deterministic synthetic backend, exact first-divergence reporting, restart and
M2-seeded replay from the embedded checkpoint, and the `chronofork tas
record/play/verify` CLI are done and backed by the executed [M3 acceptance
plan](../quality/m3-tas-replay-acceptance.md) and the dated [quality
gates](../quality/quality-gates.md) result. Frame advance, rewind, seek, rerecord,
branch editing, multi-controller/ports/analog/keyboard input, real-hardware input TAS,
import/export adapters, checkpoint-reference movies and HTTP/MCP/GUI/daemon remain
deferred. M1's differential item is ticked on the current PASS 3/3 evidence: three
synthetic ROMs match the pinned oracle's projected address streams, selected final
state and negative controls, and the masked-ROM control at the CLI default quantum 64
crosses two HBlank boundaries with the SR mask at 7 and no handler entry (negative
gating evidence, not interrupt-timing/delivery equivalence). The earlier
explicit-1,000,000-quantum two-ROM evidence is preserved as controlled-condition
history. EPIC-0002 E2-15 (SR-mask gating, `RTE`, VINT pending/acknowledge) is done; the
epic remains open for E2-11 (ROM identity), broader E2-13 instruction/addressing
coverage and remaining hardware (rendering, Z80, audio, DMA, controllers, timers,
other exception groups, privilege enforcement). See
[EPIC-0002](../epics/EPIC-0002-megadrive-slice.md),
[oracle differential evidence](../research/oracle-differential-evidence.md) and
[ADR-0002](../adr/0002-backend-boundaries.md).

**M2 is complete at the implementation level and independently verified.** Fase B
implemented the durable storage repository, named metadata, typed compatibility and
ownership, storage-backed rehydration and the `chronofork session` CLI; round 1
executed M2-A1–A11 and found the medium CLI lock-scope finding F1, and round 2 closed
F1 after the repair with no regression. All M2 checkboxes above are checked on that
evidence. Residual limits (not blockers) remain stated in the M2 section and in the
[M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md) §10: Linux-local
filesystem durability only, cooperative writer locking, long lock holds with large
`--steps`, explicit stale-lock recovery, manual retention, no encryption/auth or
migration, and the publish-only convenience `save_snapshot` caveat for third-party
callers.
