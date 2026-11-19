# Milestones

Milestones are verifiable checkpoints, not dates. A milestone is complete only when its
exit criteria are demonstrated by executed tests or recorded evidence. There is one
milestone per roadmap stage (M0–M9) plus the parallel hardware-depth track H1–H5, which is
a track, not a stage and not M10. This page is the canonical home of status, integration
state, evidence and follow-ups; [Roadmap](roadmap.md) records stage intent.

## How to read this document

- **One milestone per stage.** M0–M9 map one-to-one to roadmap Stages 0–9. The mapping is
  the "Roadmap stage" column in [Status at a glance](#status-at-a-glance); every milestone
  section links its stage and every stage links back.
- **Integration state is separate from status.** A verified branch is not a merge, and a
  merge is not a release. The table names the branch and commit when a milestone is not on
  the integration branch.
- **Checkboxes.** `[x]` requires citable evidence: an executed acceptance row, a dated gate
  result or a merged commit. `[ ]` appears only in `Next`/`Planned` milestones and in the
  `Partial`/`Planned` hardware-depth tracks; once a milestone is closed, remaining work moves
  to its follow-ups, so a closed milestone never keeps an open checkbox.
- **Closed milestones are not reopened by follow-ups.** Reopen only when (1) a verified
  exit criterion is falsified by reproducible evidence — a failing test, a manifest
  mismatch or a regression — or (2) an item is shown to have been checked without its
  evidence. New scope, new formats, new surfaces and additional hardware go to a follow-up
  entry under the closed milestone or to the owning H track; they never change the
  milestone status retroactively. This rule is defined here once and referenced from every
  closed milestone.

### Status vocabulary

| Status | Meaning |
|---|---|
| `Complete` | Every exit criterion is met and every work item is checked with evidence. Merging is recorded separately as integration state. |
| `Complete with integration pending` | Verified on a branch/worktree but not merged to the integration branch. The status line and the table name the branch and commit. |
| `Next` | The first unblocked milestone that has not started. Only one milestone is `Next`. |
| `Planned` | Not started; no active work and no committed start order. |
| `Partial` | Hardware-depth track only: some capability exists, but the track objective is not met. The missing scope is listed as unchecked work items in that track. |

## Status at a glance

| Milestone | Roadmap stage | Status | Integration state |
|---|---|---|---|
| M0 | [Stage 0 — Bootstrap foundation](roadmap.md#stage-0--bootstrap-foundation) | `Complete` | merged to `main` |
| M1 | [Stage 1 — Real backend decision and minimal hardware core](roadmap.md#stage-1--real-backend-decision-and-minimal-hardware-core) | `Complete` | merged to `main` |
| M2 | [Stage 2 — Sessions, forks and save states at product quality](roadmap.md#stage-2--sessions-forks-and-save-states-at-product-quality) | `Complete` | merged to `main` |
| M3 | [Stage 3 — TAS and deterministic replay](roadmap.md#stage-3--tas-and-deterministic-replay) | `Complete` | merged to `main` at `e816d79` |
| M4 | [Stage 4 — Debugging depth](roadmap.md#stage-4--debugging-depth) | `Complete` | merged to `main` at `4c595a2` (PR #2); verified on `agent/m4-debugging-depth` at `6d30243` |
| M5 | [Stage 5 — Tracing and scripting](roadmap.md#stage-5--tracing-and-scripting) | `Complete` | merged to `main` at `0c4e9f0` (PR #4); verified on `agent/m5-tracing-scripting` at base `84f1476` |
| M6 | [Stage 6 — Agent-native interfaces](roadmap.md#stage-6--agent-native-interfaces) | `Complete` | merged to `main` at `cb05836` (PR #6); verified on `agent/m6-agent-interfaces` at base `93d47ed` |
| M7 | [Stage 7 — Human-facing GUI](roadmap.md#stage-7--human-facing-gui) | `Complete` | merged to `main` at `cb05836` (PR #6); verified on `agent/m7-gui` at base `93d47ed` |
| M8 | [Stage 8 — Functional Mega Drive](roadmap.md#stage-8--functional-mega-drive) | `In progress` | phase 8.1 merged to `main` at `f024b7b` (PR #25); phase 8.2 implemented through repair round 9, with independent verification/review and merge pending; phases 8.3–8.8 planned |
| M9 | [Stage 9 — Multi-backend expansion](roadmap.md#stage-9--multi-backend-expansion) | `Planned` | not started |

Hardware-depth track (parallel, not a stage):

| Track | Status | Objective |
|---|---|---|
| [H1 — CPU and instruction completeness](#h1--cpu-and-instruction-completeness) | `Partial` | Documented MC68000 set and privilege delivered; remaining exception groups and coverage beyond the documented set |
| [H2 — Video, VDP and DMA](#h2--video-vdp-and-dma) | `Partial` | Rendering, CRAM/VSRAM, DMA, raster and H/V timing |
| [H3 — Audio and Z80](#h3--audio-and-z80) | `Planned` | Z80, bus arbitration, YM2612 and PSG |
| [H4 — Cartridge, controllers and timers](#h4--cartridge-controllers-and-timers) | `Partial` | ROM identity, mapper/SRAM, full controller protocol, timers |
| [H5 — Accuracy and evidence](#h5--accuracy-and-evidence) | `Partial` | Differential corpus, interrupt timing, cycle accuracy, cross-platform determinism |

## Next

**M8 — Functional Mega Drive is `Next`.** The owner redefined M8 on 2026-11-17: the former
multi-backend milestone became M9, and M8 now completes the Mega Drive model (full 68000 and
Z80, VDP rendering/timing/DMA, YM2612/PSG audio, cartridge, controllers) so a curated set of
commercial ROMs plays, with deterministic headless capture and an optional native window.
Execution is tracked in the committed [planning lifecycle](../planning/ROADMAP.md) (phases
8.1–8.8) and the [milestone design](../superpowers/specs/2026-11-17-milestone-8-megadrive-functional-design.md);
this page remains the canonical record of capability scope, its status vocabulary, evidence and
follow-ups. M7 is `Complete` and merged to `main` at `cb05836`
(PR #6): it was independently verified on `agent/m7-gui` at base `origin/main` `93d47ed`
(final verification PASS4 and final review REVIEW5, 2026-11-17) before the merge. M6 is
also `Complete` and merged at `cb05836`; it was verified on `agent/m6-agent-interfaces` at
the same base. M5 was merged to `main` at `0c4e9f0` (PR #4) after being independently
verified on `agent/m5-tracing-scripting` at base `84f1476`. M8 is the first unblocked
milestone and has started: phase 8.1 (cartridge, region, TMSS and boot) was implemented,
independently reviewed and merged to `main` at `f024b7b` (PR #25), and phases 8.2–8.8 are not
started; it depends on the M1–M7 layers and consumes the H1–H5
checkpoints. M9 — Multi-backend is `Planned` after M8. M0–M7 are merged to
`main`; M4 was merged at `4c595a2` (PR #2) after its verification on
`agent/m4-debugging-depth` at `6d30243`. H1–H5 proceed in parallel and are decomposed by M8's
phases.

## M0 — Bootstrap foundation

- **Roadmap stage:** [Stage 0 — Bootstrap foundation](roadmap.md#stage-0--bootstrap-foundation)
- **Status:** `Complete`
- **Integration state:** merged to `main`
- **Objective:** prove the architecture and establish durable project infrastructure.
- **Depends on:** nothing.
- **Evidence:** [EPIC-0001](../epics/EPIC-0001-foundation.md) (Complete);
  [quality gates](../quality/quality-gates.md) (2026-11-13 bootstrap block, a historical
  snapshot); [ADR-0002](../adr/0002-backend-boundaries.md) (accepted 2026-11-13).

### Work items

- [x] Environment inspected and recorded.
- [x] Persistent planning initialized under an ignored path.
- [x] Reference repositories cloned under ignored paths with exact revisions.
- [x] Rust workspace compiles with the documented architecture boundaries.
- [x] Synthetic backend, sessions, snapshots and forks implemented.
- [x] Acceptance tests pass.
- [x] Format, check, Clippy, test and doc gates run.
- [x] Official docs and `AGENTS.md` exist and are navigable.
- [x] Licensing/provenance policy and notices exist.
- [x] Real-backend decision approved ([ADR-0002](../adr/0002-backend-boundaries.md),
      2026-11-13).

### Exit criteria

- Workspace compiles and the required local gate passes, as recorded in the dated
  [quality gates](../quality/quality-gates.md) block.
- Documentation is navigable from the [documentation index](../README.md).
- Reference material lives under ignored paths and no upstream code is incorporated.
- The real-backend decision is proposed and accepted.

### Follow-ups and residual limits

- No follow-ups. Hardware depth starts with Stage 1 and is tracked as H1–H5; it does not
  reopen M0.
- Reopening follows the [closed-milestone rule](#how-to-read-this-document).

## M1 — Backend decision and minimal hardware slice

- **Roadmap stage:** [Stage 1 — Real backend decision and minimal hardware core](roadmap.md#stage-1--real-backend-decision-and-minimal-hardware-core)
- **Status:** `Complete`
- **Integration state:** merged to `main`
- **Objective:** implement and verify the smallest credible Mega Drive hardware core behind
  the backend contract.
- **Depends on:** M0 and [ADR-0002](../adr/0002-backend-boundaries.md).
- **Evidence:** [ADR-0002](../adr/0002-backend-boundaries.md); [Mega Drive
  slice](../architecture/megadrive-slice.md); [oracle differential
  evidence](../research/oracle-differential-evidence.md); [quality
  gates](../quality/quality-gates.md) (2026-11-14 block); [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md);
  [functional requirements](../requirements/functional.md) FR-MD-001–FR-MD-012.

### Work items

- [x] ADR-0002 approved or replaced.
- [x] Reproducible build path for the Rust-native slice on Linux.
- [x] ROM identity (`config_hash`) and a minimal hardware configuration represented.
- [x] Minimal synthetic ROM executes deterministically on the real backend.
- [x] Snapshot/restore/fork passes on the real backend.
- [x] Minimal VDP status/control and VRAM window.
- [x] Deterministic interrupt/tick scheduler with SR-mask gating, `RTE` return and
      CPU-side VINT pending/acknowledge.
- [x] VDP and interrupt scheduler integrated into backend snapshots, state hashes and CLI.
- [x] Differential trace evidence against the pinned Exodus oracle: the historical three
      synthetic ROMs match on projected address streams and selected final state, and the
      current Phase 8.2 harness covers eleven ROMs at 11/11 address, per-instruction cycle,
      final-state and negative controls. The historical masked-ROM default-quantum control
      crossed two HBlank boundaries with the SR mask at 7 and no handler entry. Current
      evidence, controlled windows and pre-repair history are in [oracle differential
      evidence](../research/oracle-differential-evidence.md); the masked run is negative
      gating evidence, not interrupt-delivery or interrupt-timing equivalence.
- [x] Capabilities report matches reality for the implemented slice.

### Exit criteria

- A known synthetic ROM executes deterministically; snapshot/restore/fork works on the real
  core; differential evidence exists and the earlier explicit-quantum two-ROM run is
  preserved as controlled-condition history.
- The modelled slice behaves as documented, including the limits below.

### Follow-ups and residual limits

- M1 has no follow-up scope: the hardware gaps it exposed are hardware depth, tracked in
  [H1](#h1--cpu-and-instruction-completeness) through [H5](#h5--accuracy-and-evidence), not
  as M1 items.
- Limits carried by the modelled slice and owned by H: privilege is enforced for the
  implemented system instructions; only HBlank level 4 and VBlank level 6 sources exist,
  with level 7 a predicate rule only; guest bus-error/trace delivery and cycle accuracy
  remain absent; no interrupt-timing equivalence is claimed; VDP interrupt-enable registers
  and VDP-side acknowledge logic are not modelled; controller input is the TH-high 3-button
  subset on ports 1–2 ([Mega Drive slice](../architecture/megadrive-slice.md#not-implemented)).
- Reopening follows the [closed-milestone rule](#how-to-read-this-document).

## M2 — Durable sessions

- **Roadmap stage:** [Stage 2 — Sessions, forks and save states at product quality](roadmap.md#stage-2--sessions-forks-and-save-states-at-product-quality)
- **Status:** `Complete`
- **Integration state:** merged to `main`
- **Objective:** persist canonical checkpoints with named metadata, typed compatibility and
  explicit writer ownership across process restarts.
- **Depends on:** M1 (real core) and [ADR-0003](../adr/0003-durable-session-storage.md).
- **Evidence:** [ADR-0003](../adr/0003-durable-session-storage.md) (accepted 2026-11-14);
  [M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md) (M2-A1–A11 executed in
  two independent rounds); [quality gates](../quality/quality-gates.md) (2026-11-15 M2
  round-2 block); [functional requirements](../requirements/functional.md) FR-SESSION-006
  and FR-SNAPSHOT-006; [non-functional
  requirements](../requirements/non-functional.md) NFR-DUR-001–005, NFR-STOR-001–004 and
  NFR-CONC-001–003.

Fase A produced [ADR-0003](../adr/0003-durable-session-storage.md) and the [M2 acceptance
plan](../quality/m2-durable-sessions-acceptance.md); Fase B implemented the durable snapshot
repository, named metadata, reject-only compatibility, storage-backed rehydration,
cooperative writer ownership and the `chronofork session save/load/list/delete` CLI. Round 1
found one medium, non-blocking CLI lock-scope finding (F1): `save --session` read the latest
base before taking the writer lock. The repair acquired the per-session lock before the read
and held it through rehydrate/advance/capture/publish, and round 2 re-verified F1 closed with
typed contention, no stale-base double publish and no regression. Raw evidence is in the
ignored, worktree-local `.research/m2-durable-sessions/fase-b/verification/` (round 1 plus
`round2/`) and `.research/m2-durable-sessions/fase-b-repair-f1/`; the durable record is the
acceptance plan and the dated [quality gates](../quality/quality-gates.md) result.

### Work items

- [x] Checkpoints persist across process restarts.
  - [x] Versioned storage root and marker (`CFSTORE1`, layout v1) with an explicit
        caller-supplied root and no implicit creation.
  - [x] Snapshot publish writes the canonical `Snapshot::to_bytes()` envelope verbatim
        through the tmp+fsync+rename sequence; a crash never exposes a partial snapshot as
        committed.
  - [x] A new process opens the root, validates the object and restores the recorded tick
        and state hash.
  - [x] Staging, tombstone and orphan objects are reported as typed anomalies and are never
        repaired or silently loaded.
  - [x] Restart, crash-recovery and canonical-bytes acceptance tests are executed and
        recorded.
- [x] Named snapshots and metadata.
  - [x] One versioned metadata record round-trips name, logical creation sequence, lineage
        keys and the mirrored envelope fields.
  - [x] Metadata-only rename changes no checkpoint bytes, state hash or replay result.
  - [x] Listing is deterministic; retention is caller-driven and deletion is explicit; no
        operation performs automatic garbage collection.
  - [x] Metadata and retention acceptance tests are executed and recorded.
- [x] Version compatibility policy tested.
  - [x] Reject-only matrix for storage layout, metadata, outer envelope, backend envelope
        and backend payload.
  - [x] Corrupt, truncated, checksum-bad, swapped and identity-mismatched input produces
        typed errors; no silent migration or reinterpretation.
  - [x] Compatibility acceptance tests are executed and recorded.
- [x] Concurrency/ownership policy implemented at the service boundary.
  - [x] One writer per durable session through an explicit lock/lease; contention is a
        typed error with no busy-wait or silent retry.
  - [x] Stale and corrupt lock recovery is explicit and owner-checked; no lock is stolen
        automatically.
  - [x] Readers are lock-free and never observe a torn checkpoint.
  - [x] There is no hidden global lock; only the bounded root key-allocation lock is shared,
        and it is never held during emulation, serialization or checkpoint I/O.
  - [x] Ownership and concurrency acceptance tests are executed and recorded.
- [x] No mutable state shared across sessions.
  - [x] Rehydrated sessions own independent mutable state; forks never share it.
  - [x] Publishing or deleting one session/snapshot never changes another's checkpoint
        bytes, state hash or lineage.
  - [x] In-process and durable isolation acceptance tests are executed and recorded.

### Exit criteria

- Every M2-A1–A11 acceptance criterion is executed and recorded in the [M2 acceptance
  plan](../quality/m2-durable-sessions-acceptance.md), and the full gate passes on the M2
  tree.
- Round 2 re-verified the F1 repair with typed contention, no stale-base double publish and
  no regression.
- Creating, forking, restoring and comparing sessions across process restarts loses no
  state; incompatible input is rejected with typed errors.

### Follow-ups and residual limits

- Residual limits (not blockers): durability is verified only on local Linux filesystems
  (Windows and network filesystems unverified); the writer lock is cooperative metadata, not
  authentication; holding the lock across long in-memory `--steps` advances lengthens
  contention by design; a crashed holder needs explicit owner/generation-checked recovery;
  retention is manual with no automatic GC; there is no encryption, authentication or format
  migration; the public convenience `save_snapshot` remains publish-only for third-party
  callers (the CLI uses the lock-aware workflow); and NFR-CONC-003 remains `Partial` because
  only sequential lock-free-reader evidence exists.
- M2-FU1: cross-platform durability evidence (Windows and network filesystems) and a
  concurrent publish/read or delete/read interleaving test for NFR-CONC-003. Derived from the
  residual limits; it does not reopen M2.
- Reopening follows the [closed-milestone rule](#how-to-read-this-document).

## M3 — TAS and replay

- **Roadmap stage:** [Stage 3 — TAS and deterministic replay](roadmap.md#stage-3--tas-and-deterministic-replay)
- **Status:** `Complete`
- **Integration state:** merged to `main` at `e816d79`
- **Objective:** deterministic input movies, replay verification, seek/rewind, rerecord and
  editing across the synthetic validator and the minimal Mega Drive slice.
- **Depends on:** M2 (durable checkpoints and storage); the TH-high 3-button controller
  subset in the slice ([H4](#h4--cartridge-controllers-and-timers), partial);
  [ADR-0004](../adr/0004-tas-movie-format.md) through
  [ADR-0009](../adr/0009-real-hardware-input-tas.md).
- **Evidence:** the six executed acceptance plans — [M3 TAS/replay](../quality/m3-tas-replay-acceptance.md)
  (M3-A1–A12), [partial replay](../quality/m3-partial-replay-acceptance.md) (M3-A13–A17),
  [format v2](../quality/m3-format-v2-acceptance.md) (M3-A18–A27),
  [rewind](../quality/m3-rewind-acceptance.md) (M3-A28–A34),
  [rerecord](../quality/m3-rerecord-acceptance.md) (M3-A35–A44) and
  [hardware](../quality/m3-hardware-acceptance.md) (M3-A45–A53); the M3 blocks in
  [quality gates](../quality/quality-gates.md); the ADRs linked below; and the t7 promotion
  audit PASS of 2026-11-16, recorded in the ignored
  `.research/m3-hardware/final-verification/` evidence with a verified manifest.

Six increments, each independently verified: v1 movie and replay
([ADR-0004](../adr/0004-tas-movie-format.md)), ranged replay and branch comparison
([ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md)), format-v2 anchors and
multi-device input
([ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md)), rewind and
studio stepping ([ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md)),
rerecord and branch editing
([ADR-0008](../adr/0008-rerecord-and-branch-editing.md)), and real-hardware input TAS
([ADR-0009](../adr/0009-real-hardware-input-tas.md)). Design detail lives in the ADRs; the
verified scope and its limits are the work items and residual limits here.

### Work items

- [x] Versioned canonical movie format: single-file v1 `.cfmovie` and v2 `CFMOVIE2` with an
      embedded canonical checkpoint, ordered anchors, schema-2 multi-device step records and
      reject-only compatibility with no migration (M3-A1/A2/A8/A11/A12 and
      M3-A18/A19/A23–A27; [ADR-0004](../adr/0004-tas-movie-format.md),
      [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md)).
- [x] Record/playback of a per-tick input sequence on the deterministic synthetic backend,
      with the source checkpoint never advanced or mutated (M3-A1–A12).
- [x] Replay verification with exact first-divergence reporting: tick, input index, expected
      hash and observed hash, with no resynchronization (M3-A4).
- [x] Restart replay and M2-seeded record from the embedded checkpoint with no shared state
      or storage root (M3-A1–A12).
- [x] Ranged replay/seek over an exact `(from, until]` window, with the prefix always
      executed and verified, plus forward single-tick advance and rewind by deterministic
      linear re-simulation (M3-A13–A17).
- [x] Branch comparison of recorded `(tick, state hash)` records with first-divergence,
      length and embedded-checkpoint identity reporting and no re-simulation (M3-A15).
- [x] Periodic anchors/checkpoints and nearest-anchor seek in the single ranged replay path,
      reporting anchored fields and `prefix_verified=0` (M3-A18–A21/A24).
- [x] Schema-2 multi-device input: four digital ports, four analog axes and a 64-bit keyboard
      bitmap recorded and replayed deterministically on the synthetic backend (M3-A22/A24).
- [x] In-process rewind buffer/state cache and interactive stepping as the session-layer
      `Player`/`StateCache` and `chronofork tas studio`, bounded by count and the
      deterministic `payload+256` metric with lowest-step-first eviction (M3-A28–A34).
- [x] Rerecord and branch editing in `chronofork_sessions::edit` and `chronofork tas edit`:
      verified-prefix/suffix re-simulation, a new branch artifact with its own movie id and
      `parent_movie_id`, `keep-prefix`/`drop`/`rebuild` anchor handling, and v1→v1 / v2→v2
      preservation (M3-A35–A44).
- [x] Real-hardware input TAS: `--rom` backend selection and stateless per-step controller
      reads of `0x00A10003`/`0x00A10005` for the TH-high 3-button subset on ports 1–2, with
      no canonical snapshot/state/config change (M3-A45–A53;
      [ADR-0009](../adr/0009-real-hardware-input-tas.md)).
- [x] Minimum CLI surfaces `tas record/play/verify/compare/studio/edit` with stable
      `key=value` blocks, stdout data only, stderr diagnostics and exit 0/1, verified by
      process-level tests (M3-A7/A17/A24/A30/A40/A49).

### Exit criteria

- A recorded input sequence reproduces every recorded tick and state hash exactly and
  reports the first divergence exactly.
- Ranged and anchored replay, single-tick advance, rewind/studio and rerecord/edit behave as
  specified on the deterministic synthetic backend; record/play/verify/edit/step work on the
  minimal Mega Drive slice with per-step controller input and `--rom` selection.
- The t5 completion audit covered the five remaining items with executed evidence, the t6
  promotion recorded completion, and the t7 promotion audit passed on 2026-11-16.
- No unimplemented capability is claimed.

### Follow-ups and residual limits

- M3-FU1: movie import/export adapters (for example BizHawk BK2). Documented-only today; the
  movie format is M3-owned, so this is a follow-up rather than a new stage.
- M3-FU2: checkpoint-reference movies, explicitly deferred by M3.
- M3-FU3: in-place/timeline editing, a persisted rerecord counter, format v3, mixed-anchor
  mode and O(1)/persistent-cache seek — user-visible additions M3 explicitly does not claim.
- Residual limits: the hardware claim is the independently authored synthetic
  `controller_rom()` and the TH-high 3-button subset; A/Start, TH/6-button multiplexing,
  controller timing, analog/keyboard hardware, port 3, commercial ROMs, and cycle accuracy
  are not implemented or claimed ([H4](#h4--cartridge-controllers-and-timers),
  [H5](#h5--accuracy-and-evidence)). Checkpoint-reference movies, HTTP/MCP/GUI and the
  daemon remain deferred ([M6](#m6--agent-interfaces), [M7](#m7--gui)).
- Reopening: only per the [closed-milestone rule](#how-to-read-this-document); M3-FU1–FU3
  never reopen M3.

## M4 — Debugging depth

- **Roadmap stage:** [Stage 4 — Debugging depth](roadmap.md#stage-4--debugging-depth)
- **Status:** `Complete`
- **Integration state:** merged to `main` at `4c595a2` (PR #2); independently verified on
  `agent/m4-debugging-depth` at `6d30243` (t5/t6/final-review PASS).
- **Objective:** a bounded, non-canonical debug observation surface plus a host debugger and
  CLI for the minimal Mega Drive slice, without changing canonical state.
- **Depends on:** M1 (backend inspection and the slice) and
  [ADR-0010](../adr/0010-debugging-depth.md).
- **Evidence:** [ADR-0010](../adr/0010-debugging-depth.md) (accepted and implemented
  2026-11-16); [M4 acceptance plan](../quality/m4-debugging-depth-acceptance.md) (every
  M4-A1–A17 row `executed - passed`); [quality gates](../quality/quality-gates.md) (M4
  2026-11-16 block); ignored, worktree-local raw evidence in
  `.research/m4/verification/SUMMARY.txt` (feature verification PASS at `df98321`),
  `.research/m4/final-audit/attempt-2/SUMMARY.txt` (promotion audit PASS at `6d30243`) and
  `.research/m4/final-review/REVIEW.txt` (final content review PASS at `6d30243`).

The scope is the minimal Mega Drive slice on Linux: host PC/range/conditional breakpoints,
byte-granular read/write watchpoints, event breakpoints and logpoints, bounded
register/memory/disassembly views, CPU/VDP/interrupt/bus inspectors and an optional
non-canonical `DebugAccess`. It does not include mid-instruction or cycle-exact stops,
reverse execution, call-stack/step-over, DMA/Z80/audio/rendering inspection, persistent
debugger state, UI/HTTP/MCP or cross-platform runtime evidence.

### Work items

- [x] Debug observation contract: optional `DebugAccess`, typed `DebugConfig`/`DebugEvent`s,
      a bounded event ring and `Capabilities::debug_events`.
- [x] Host debugger layer `chronofork-debug`: PC/range/conditional breakpoints,
      byte-granular read/write watchpoints, event breakpoints and logpoints.
- [x] Bounded register, memory and 68000-subset disassembly views, cross-checked against
      execution.
- [x] Mega Drive CPU, VDP, interrupt-scheduler and bus inspectors.
- [x] `chronofork debug run|inspect|memory|disasm|vdp|interrupts|bus` CLI with deterministic
      record blocks, exit codes and typed errors.
- [x] Non-perturbation evidence: the per-step observation off/on probe found zero canonical
      differences, demo and movie bytes were unchanged, and the pinned-oracle differential
      stayed at its then-current PASS 3/3 (now superseded by the 11/11 run).
- [x] Independent verification of every M4-A1–A17 row, the full gate, the demo/regression
      comparison and the claim audit (t5 PASS 2026-11-16 at `df98321`).
- [x] Post-PASS capability-report/wording fix at `6b2a567` re-gated, then promotion audit
      attempt 2 PASS and final content review PASS at `6d30243`.

### Exit criteria

- Every M4-A1–A17 row in the [M4 acceptance
  plan](../quality/m4-debugging-depth-acceptance.md) is `executed - passed` and the full
  gate is clean on the M4 tree ([quality gates](../quality/quality-gates.md)).
- Observation never changes canonical state, snapshots, state/config hashes or movie bytes.
- The capability report and the docs claim only the verified surface.
- Promotion audit and content review PASS at `6d30243`; merged to `main` at `4c595a2`
  (PR #2).

### Follow-ups and residual limits

- M4-FU1: code/data analysis, symbol mapping, call-stack/step-over/step-out, reverse
  execution, conditional watchpoint values and persistent debugger configuration — debugger
  surface, not hardware.
- M4-FU2: DMA, rendering, CRAM/VSRAM, audio/Z80 and mapper inspector views, which need the
  [H2](#h2--video-vdp-and-dma)–[H4](#h4--cartridge-controllers-and-timers) device models
  before they can be verified.
- Residual limits: stops are instruction-granular; there is no general instruction-fetch
  event, no bus-read logging without a watchpoint and no configurable log-ring capacity; the
  log ring is fixed-size. UI, HTTP and MCP surfaces are
  [M6](#m6--agent-interfaces)/[M7](#m7--gui); cross-platform runtime evidence is
  [H5](#h5--accuracy-and-evidence).
- Follow-ups never reopen M4; reopening follows the
  [closed-milestone rule](#how-to-read-this-document).

## M5 — Tracing and scripting

- **Roadmap stage:** [Stage 5 — Tracing and scripting](roadmap.md#stage-5--tracing-and-scripting)
- **Status:** `Complete`
- **Integration state:** merged to `main` at
  `0c4e9f0e8c3fd2426ca2fb313a48b992c0fbd2ee` (PR #4); verified on branch
  `agent/m5-tracing-scripting` at HEAD/base
  `84f1476466c14e72f911689845c390173982fd81` before the merge.
- **Objective:** structured, timestamped traces and a sandboxed deterministic script
  runtime for reproducible experiments without host-language plugins.
- **Depends on:** M4 (bounded observation surface), M2 (sessions and storage) and M3
  (movies). Not blocked by H1–H5.
- **Evidence:** final independent verification **T12 PASS (2026-11-16)** after the T7
  `needs_revision` round and the T11 repair of findings F1–F10; integrated review T10
  PASS. All 36 test-row commands and A36–A39 are `executed - passed` in the [M5 acceptance
  plan](../quality/m5-tracing-and-scripting-acceptance.md); the dated gate block is the
  2026-11-16 M5 block in [quality gates](../quality/quality-gates.md); contracts are
  [ADR-0011](../adr/0011-tracing-and-scripting.md) and the [tracing and scripting
  architecture](../architecture/tracing-and-scripting.md); requirements are
  FR-TRACE-001, FR-TRACE-002 and FR-SCRIPT-001 in [functional
  requirements](../requirements/functional.md). Raw evidence is under the ignored
  `.agents/notes/m5/t12/` tree (`t12-verification.md`, `rows/`, `gate/`, `demos/`,
  `audit/`) and `t10-review.md`. M5 was merged unchanged at `0c4e9f0` (PR #4).

### Work items

- [x] Versioned structured trace format for instruction, bus and device events, with
      emulated timestamps and bounded retention (M5-A1–A10; `CFTRACE1` v1).
- [x] Structured export and import with typed rejection of incompatible versions
      (M5-A2, M5-A17; TSV is one-way only).
- [x] Triggered and circular tracing with bounded buffers (M5-A9, M5-A10).
- [x] Trace comparison and historical queries with first-divergence reporting
      (M5-A11–A13, M5-A15–A16).
- [x] Sandboxed script runtime with memory and time limits and typed errors
      (M5-A19–A31; purpose-built DSL, not an embedded host-language engine).
- [x] Script APIs for sessions, forks, inspection, TAS and experiments
      (M5-A24–A28; the experiment artifact is the deterministic `ScriptRun` report).
- [x] Process-level CLI tests for every trace and script surface
      (M5-A14–A18 and M5-A32–A34; 16 trace and 3 script process tests).

### Exit criteria

- **Met:** scripts run controlled experiments and produce reproducible reports without
  host-language plugins; byte-identical reports are demonstrated by M5-A28/A32 and the
  report determinism tests.
- **Met:** canonical trace export round-trips byte-identically and trace comparison reports
  the first record or length divergence (M5-A1, M5-A12, M5-A17).
- **Met:** sandbox limits are enforced, typed and tested, including fake-clock wall time and
  path-escape rejection (M5-A29–A31).
- **Met:** FR-TRACE-001, FR-TRACE-002 and FR-SCRIPT-001 are promoted to **Implemented** for
  the documented scope in [functional requirements](../requirements/functional.md).

### Deferred and follow-ups

M5 is closed; the following are tracked elsewhere and do not reopen it:

- Frame pacing and a host-time run loop belong to [M7](#m7--gui), not to tracing.
- GUI trace viewers are [M7](#m7--gui); HTTP/MCP transport is
  [M6](#m6--agent-interfaces).
- DMA, audio and Z80 trace sources wait for
  [H2](#h2--video-vdp-and-dma)/[H3](#h3--audio-and-z80); performance evidence remains
  [H5](#h5--accuracy-and-evidence).
- **M5-FU1:** trace import from TSV, trace migration/compression and a separate TSV byte cap
  (TSV output is bounded by record count, not `MAX_TRACE_BYTES`).
- **M5-FU2:** race-free sandbox file operations and, if untrusted users are ever in scope, a
  process/WASM isolation layer; the current sandbox is a deterministic capability boundary,
  not an OS security boundary.
- **M5-FU3:** preempting a single long backend call against the wall-time deadline (checks
  currently occur at statement and every-256-operation boundaries).

## M6 — Agent interfaces

- **Roadmap stage:** [Stage 6 — Agent-native interfaces](roadmap.md#stage-6--agent-native-interfaces)
- **Status:** `Complete`
- **Integration state:** merged to `main` at `cb05836` (PR #6). It was verified on
  `agent/m6-agent-interfaces`, HEAD and base `origin/main`
  `93d47ed308874be0e8e7060641c238c7f6809e31` before the merge.
- **Objective:** expose the same services to humans, scripts and agents through CLI, HTTP
  and MCP with identical semantics.
- **Depends on:** the M2–M5 services; transport types must not enter core, backend or session
  APIs.
- **Evidence:** independent pass 2 **PASS (2026-11-17)** after the round-1 review returned
  `needs_revision` with findings M6-R1–M6-R10 and the repair closed them; the round-2 review
  returned pass with no regression. All M6-A1–M6-A32 rows are `executed - passed` in the
  [M6 acceptance plan](../quality/m6-agent-interfaces-acceptance.md); the dated gate record
  is the 2026-11-17 M6 block in [quality gates](../quality/quality-gates.md); the gate
  result, target accounting, demo hashes and non-regression comparison are recorded there and
  are not duplicated here. Contracts are
  [ADR-0012](../adr/0012-agent-native-interfaces.md) and the [agent interfaces
  architecture](../architecture/agent-interfaces.md); requirements are FR-API-001,
  FR-API-002 and FR-API-003 in [functional requirements](../requirements/functional.md).
  Raw evidence is under the ignored `.agents/notes/m6/verify/` tree (`PASS2.md`,
  `TREE-STATE-PASS2.md`, `rows2/`, `gate/`, `demos2/`, `parity/`, `audit/`, plus the
  preserved round-1 `ROWS.md` and its directories).
- **Verified behavior changes during the repair (pinned by tests, not silent):**
  `max_sessions` counts all registered sessions including closed ones (closed sessions stay
  unlisted and non-mutatable); the human `session list` prints exactly
  `chronofork: storage listing truncated at 256 entries` on stderr when the service reports
  `truncated:true` (stdout shape unchanged, `--json` keeps the field); locally produced and
  human-facing failures use the base `chronofork: <message>` prefix, with typed codes only
  in canonical envelopes; the invalid `--name` wording now comes from the service catalog.

### Work items

- [x] HTTP API over the existing application services (M6-A11–A13, A16, A18–A21;
      `GET /health`, `GET /v1/tools`, `POST /v1/ops`).
- [x] MCP over Streamable HTTP, backed by the same services (M6-A14, A15, A22; stateless
      JSON-RPC 2.0, protocol `2025-03-26`, tools subset).
- [x] Typed tool surface instead of hundreds of loose endpoints (M6-A6, A11, A14; one
      curated 16-operation catalog mapped identically onto HTTP and MCP).
- [x] Session ownership, permissions and concurrency controls at the transport boundary
      (M6-A3–A5, A8, A12, A31; token file, read/write/admin scopes, per-principal session
      ownership, manager lock plus per-session writer lease).
- [x] Machine-consumable experiment artifacts (M6-A10, A24; deterministic
      `chronofork.experiment.v1`, content-derived `artifact_id`, `experiment run --out`).
- [x] CLI reimplemented as a first-class client of the shared services (M6-A20–A25;
      `serve`, `api`, `mcp`, `experiment run` and the `session` refactor onto the service).
- [x] Semantic parity tests across CLI, HTTP and MCP (M6-A18, A19, A21–A23; byte-level
      parity over all 16 operations and the error matrix).
- [x] Daemon/deployment shape for long-running sessions (M6-A16, A20, A30; bounded worker
      pool, graceful shutdown, durable store shared across restarts).

### Exit criteria

- **Met:** identical operations through CLI, HTTP and MCP with identical semantics, proven by
  byte-level parity tests over all 16 operations and the documented error matrix (M6-A18,
  M6-A19, M6-A21–A23).
- **Met:** no transport type appears in core, backend or session APIs (M6-A26 audit; the
  dependency direction is `core ← backend-api ← sessions ← service ← transport`).
- **Met:** the session layer remains the only canonical state owner; MCP and HTTP are thin
  mappings and the experiment artifact is content-addressed output, not state (M6-A7, A10,
  A26).
- **Met:** FR-API-002 and FR-API-003 are **Implemented** for the documented transport scope in
  [functional requirements](../requirements/functional.md), with FR-API-001 updated and
  FR-API-004 still `Planned` for Stage 9.

### Deferred and follow-ups

M6 is closed for the documented scope; the following are tracked and do not reopen it. The
GUI is [M7](#m7--gui) and additional systems are [M9](#m9--multi-backend).

- **M6-FU1:** debug, trace, script and TAS operations over transport. They stay CLI-local in
  M6 because they are large, sandbox-sensitive surfaces; the catalog is deliberately curated
  and no transport claims them.
- **M6-FU2:** durable per-principal ownership (the M6 storage format carries no owner) and
  restart-persistent live sessions. Ownership is process-local today and lost on daemon
  restart; closed sessions are retained for the process lifetime and keep counting toward
  `max_sessions`.
- **M6-FU3:** TLS, token rotation/expiry, an audit log and multi-node deployment. Tokens are
  coarse and static, the HTTP server is HTTP/1.1 without TLS or keep-alive, and comparison is
  not claimed constant-time. Any network surface still follows the repository security
  constraints: no telemetry and no upload path without explicit design approval.
- **M6-FU4:** SSE/streaming, MCP resources/prompts/logging/completion and `Mcp-Session-Id`.
  `POST /mcp` is stateless JSON-only; there are no server-initiated messages. Revisit only
  with a real streaming consumer.
- Recorded precision limits: `io_timeout` is verified at process level but has no automated
  timeout test; `internal_error` is table-tested but never produced by an operation; no
  performance claim is made (the bounds are correctness/DoS limits); verification ran on the
  Linux/WSL acceptance platform only.

## M7 — GUI

- **Roadmap stage:** [Stage 7 — Human-facing GUI](roadmap.md#stage-7--human-facing-gui)
- **Status:** `Complete`
- **Integration state:** merged to `main` at `cb05836` (PR #6). It was verified on
  `agent/m7-gui`, HEAD and base `origin/main` `93d47ed308874be0e8e7060641c238c7f6809e31`
  (with the verified M6 snapshot) before the merge.
- **Objective:** provide optional human-facing presentation surfaces for observing and
  controlling sessions without owning canonical state.
- **Depends on:** M6 (attach/detach transport). Real video and audio to present depend on
  [H2](#h2--video-vdp-and-dma)/[H3](#h3--audio-and-z80) and are **not delivered**: the
  pipeline is capability-gated and honestly reports `rendering=unavailable` /
  `audio=unavailable` until a real source/sink exists.
- **Evidence:** final independent verification **PASS4 (2026-11-17)**, after the round-1
  FAIL (F1–F4), the FAIL3 round (M7-A24 as written) and the repairs; the final independent
  review **REVIEW5** returned **PASS** with no new finding and no regression. All
  M7-A1–M7-A30 rows are `executed - passed` in the [M7 acceptance
  plan](../quality/m7-gui-acceptance.md); the dated gate record is the 2026-11-17 M7 block
  in [quality gates](../quality/quality-gates.md) — `./scripts/check.sh` exit 0; the dated
  block records the target/passed counts against the M6 baseline, and both demos are 3/3
  byte-identical with the frozen M6 crates unchanged. Contracts are
  [ADR-0013](../adr/0013-human-facing-gui.md) and the [GUI
  architecture](../architecture/gui.md); requirements are FR-GUI-001 and FR-EXEC-007 in
  [functional requirements](../requirements/functional.md). Raw evidence is under the
  ignored `.agents/notes/m7/verify/final2/` tree (`PASS4.md`, `rows/`, `rows/process/`,
  `gate/`, `demos/`, `audit/`), with the independent review in
  `.agents/notes/m7/review/REVIEW5.md`; earlier rounds are preserved in the same ignored
  tree.
- **Repairs recorded during verification (pinned by tests, not silent):** the `detach`
  block field order now matches the implementation and test; the frame title uses single
  spaces; `gui --help` on an unwritable stdout is one typed `channel` diagnostic instead of
  a panic; the empty `events` fallback is documented as a formatter fallback and unit
  tested; the attach/detach tests cover hidden-panel equality and the single visible
  `events` line separately; the CLI GUI test harness tolerates only `BrokenPipe` on a child
  that exits before reading the whole script (100 KB regression test); `SystemClock` is
  documented as a library capability with `ManualClock`/`pulse` as the only v1 process
  path; `PanelState` is visibility-only; the test-only `expect_used` exception and the
  duplicated bounded/token reader are registered with owner **M7** and a revisit condition.

### Work items

- [x] Video presentation decoupled from canonical state (M7-A12, A18, A20; a
      capability-gated `Presentation<VideoSource>` pipeline that never fabricates frames
      and reports `unavailable` in every v1 process configuration).
- [x] Audio presentation decoupled from canonical state (M7-A13, A18, A20; a
      capability-gated `Presentation<AudioSink>` with deterministic chunks in the test
      path and `unavailable` in every v1 process configuration).
- [x] Frame pacing and a host-time run loop (M7-A14, A15; `Pacer` with injected
      `HostClock`/`ManualClock`, `advance_elapsed` catch-up/drop accounting, the pause gate
      and the deterministic `pulse` command).
- [x] Debugger panels, memory viewers, hardware inspectors and TAS timelines (M7-A5–A8,
      A11, A23; eight-panel availability matrix, read-only bounded memory view, inspector
      reasons including `M6-FU1`, and the read-only `.cfmovie` timeline with unchanged
      movie bytes).
- [x] Single-session and multi-session window/mosaic layouts (M7-A5, A9; pane allocation,
      focus, `single`/`mosaic` layouts and deterministic fixed-size frame grids).
- [x] Remote observation and human/agent control handoff (M7-A16, A17, A21, A22; lease
      state machine, local H→A→H flow, real-daemon observation and second-process agent
      handoff).
- [x] Attach/detach tests proving unchanged emulation results (M7-A6, A7, A28; process and
      crate tests plus the frozen-layer audit).

### Exit criteria

- **Met:** the GUI attaches to and detaches from sessions without changing emulation
  results (M7-A6, A7 — the only by-design frame difference is the visible `events` log
  line; the state hash and tick are unchanged; A21/A22 confirm it against a real daemon).
- **Met:** presentation never owns or mutates canonical state (M7-A1 dependency graph:
  `chronofork-gui` depends on core and service only; A2/A3/A4 show the GUI issues only the
  curated read/create/run operations; A28 shows the canonical artifacts unchanged).
- **Met:** human/agent control handoff is observable and deterministic (M7-A16, A17, A22;
  generation/sequence counters, typed `control_denied`, byte-identical process output).
- **Met:** FR-GUI-001 is **Implemented** for the documented text/HTML presentation scope and
  FR-EXEC-007 is **Implemented** for the verified library pacing/run loop plus rewind in
  [functional requirements](../requirements/functional.md); the process surface
  deliberately exposes only the deterministic `pulse` and no native window.

### Deferred and follow-ups

M7 is closed for the documented deterministic presentation scope; the following are tracked
and do not reopen it.

- **Native toolkit frontend (follow-up after H2/H3):** v1 presents text and a static HTML
  document only — no native window, no picture and no sound. The GUI is a presentation
  crate with no third-party toolkit dependency.
- **Real video and audio sources/sinks (H2/H3):** the capability-gated pipeline is the seam;
  the `Available` path is exercised only by test-pattern/recording doubles. No rendering,
  audio-output or performance claim is made.
- **Inspector and trace panel content (M6-FU1):** VDP/bus/disassembly/trace content stays
  absent because those operations are not exposed over transport; the panel matrix reports
  `unavailable:not_exposed_by_transport`.
- **Live host-time playback:** `SystemClock` and `advance_elapsed` are verified library
  capabilities, but no v1 CLI process builds the host clock; playback is the deterministic
  `pulse`. Wiring real host time remains a follow-up.
- **Other non-goals:** GUI input injection, GUI state persistence, multi-window/
  multi-display, fork/snapshot/restore/close/compare/store from the window, browser/JS
  frontend and streaming transport. The controller lease is cooperative, not a lock, and
  the acceptance evidence is Linux/WSL single-platform only.

## M8 — Functional Mega Drive

- **Roadmap stage:** [Stage 8 — Functional Mega Drive](roadmap.md#stage-8--functional-mega-drive)
- **Status:** `In progress`
- **Integration state:** phase 8.1 (cartridge, region, TMSS and boot) merged to `main` at
  `f024b7b` (PR #25); phase 8.2 implemented through repair round 9, with independent
  verification/review and merge pending; phases 8.3–8.8 planned
- **Objective:** complete the Mega Drive backend — full 68000 and Z80, complete VDP with
  rendering/DMA/timing, YM2612/PSG audio, cartridge/TMSS/mapper/SRAM, timers and controllers —
  so a curated commercial ROM set plays with video, audio and input, with deterministic
  headless capture and an optional native window.
- **Depends on:** the M1–M7 layers and contracts; it consumes the
  [H1–H5](#h1--cpu-and-instruction-completeness) checkpoints as its decomposition and depends
  on the accepted [ADR-0002](../adr/0002-backend-boundaries.md).
- **Evidence:** phase 8.1 (cartridge/region/TMSS/boot) is implemented and reviewed, recorded
  in the [Mega Drive slice](../architecture/megadrive-slice.md#implemented-cartridge-region-and-tmss-m8-phase-81).
  The design is the
  [milestone design](../superpowers/specs/2026-11-17-milestone-8-megadrive-functional-design.md);
  phase execution lives in the [planning lifecycle](../planning/ROADMAP.md).

### Work items

- [x] Cartridge header/region/TMSS/checksum, images above 4 MiB, a banking mapper and SRAM
      (phase 8.1, reviewed PASS 2026-11-17).
- [x] Documented MC68000 instruction set, addressing modes, privilege enforcement and
      cycle accounting (phase 8.2; final verification/review and merge pending).
- [ ] Remaining 68000 exception groups (guest bus error/trace), a wired level-7 source
      and cycle/prefetch behavior only as far as H5 requires.
- [ ] Complete VDP: CRAM/VSRAM, planes/window/sprites/scrolling/priority/shadow-highlight,
      256/320 modes, DMA, FIFO/access timing, H/V counters and interrupt enable/acknowledge.
- [ ] Z80 core, 68000/Z80/VDP bus arbitration, YM2612 six-channel FM and PSG.
- [ ] Timer interrupts and the full TH latch / 3- and 6-button controllers on ports 1–2.
- [ ] Cycle-based canonical time plus unpaced/paced/manual execution modes with run-to-target
      APIs for agents.
- [ ] Deterministic headless PNG frame and WAV audio capture.
- [ ] Optional native windowed frontend behind an accepted ADR that supersedes ADR-0013's
      no-toolkit constraint.
- [ ] Curated commercial-ROM compatibility list with executed evidence and measured
      performance.
- [ ] Versioned snapshot payloads for every device, with the superseded M1–M7 timing contracts
      documented.

### Exit criteria

- The complete 68000 and Z80 instruction sets are implemented and differential-tested against
  the pinned Exodus oracle.
- The VDP renders and transfers per H2; YM2612/PSG audio per H3; cartridge, controllers and
  timers per H4.
- A curated set of commercial ROMs boots and plays with video, audio and input; the list and
  its executed evidence are documented and no ROM is committed.
- Unpaced, paced and stepped modes produce identical frame, audio and state hashes for the
  same input.
- Headless capture is hash-pinned; the native window presents video/audio and accepts input.
- Performance is measured at 60/50 fps in paced mode and the optimizations are documented.
- The H1–H5 rows for the implemented scope are checked with evidence.

### Deferred and follow-ups

- Port 3, analog and keyboard hardware remain H4 residual limits unless the curated set
  requires them.
- Dynamic recompilation, run-ahead, skip-frames and savestate-rewind shortcuts are out of
  scope; each would need an ADR and a determinism analysis.
- Universal commercial compatibility and cycle-perfect hardware equivalence are not claimed.

## M9 — Multi-backend

- **Roadmap stage:** [Stage 9 — Multi-backend expansion](roadmap.md#stage-9--multi-backend-expansion)
- **Status:** `Planned`
- **Integration state:** not started
- **Objective:** integrate a second system behind the same capability model without changing
  session, storage or transport semantics.
- **Depends on:** the M2–M7 shared layers and the boundaries in
  [ADR-0002](../adr/0002-backend-boundaries.md).
- **Evidence:** none yet. The scope is
  [Stage 9](roadmap.md#stage-9--multi-backend-expansion); capability status is FR-API-004 in
  [functional requirements](../requirements/functional.md).

### Work items

- [ ] Second backend integrated behind the backend and capability contracts.
- [ ] Capability differences surfaced honestly per backend.
- [ ] Shared tracing/TAS/debugging where capabilities allow.
- [ ] Cross-backend differential tests.
- [ ] SDK stability policy and versioned external SDKs.

### Exit criteria

- A second system is integrated without changing session/storage/transport core semantics.
- Capability differences are reported, not normalized away.
- Cross-backend differential tests run and are recorded.

### Deferred and follow-ups

- Hardware completeness for each system is tracked in that system's own hardware-depth track;
  [H1–H5](#h1--cpu-and-instruction-completeness) cover the Mega Drive model only.
- Feature parity between backends is not required; honest capability differences are.

## H1 — CPU and instruction completeness

- **Roadmap track:** [H1](roadmap.md#h1--cpu-and-instruction-completeness) — parallel
  hardware-depth track, not a stage.
- **Status:** `Partial`
- **Integration state:** n/a (track)
- **Objective:** a documented, testable 68000 model; the documented MC68000 set and
  privilege are delivered, and the remaining track work is exception groups and
  coverage beyond the documented set.
- **Depends on:** the M1 slice and its differential harness.
- **Evidence:** the documented instruction set in [Mega Drive
  slice](../architecture/megadrive-slice.md#68000-instruction-set); [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md)
  E2-03/E2-13; FR-MD-002 and FR-MD-009 in
  [functional requirements](../requirements/functional.md).

### Work items

- [x] Documented MC68000 instruction set with reset vectors, flags and big-endian bus discipline.
- [x] E2-13 absolute-long `MOVE` decoding repair, verified by differential evidence.
- [ ] Coverage beyond the documented MC68000 instruction set and broader differential
      cases (EPIC-0002 E2-13 continued).
- [x] Privilege enforcement and separate user/supervisor stacks, with RTE restore.
- [ ] 68000 exception groups beyond the implemented interrupt path (guest bus error and
      trace).
- [ ] A wired level-7 interrupt source instead of a predicate rule only.

### Exit criteria

- The instruction/addressing matrix the slice documents is implemented and
  differential-tested.
- Privilege and user/supervisor separation are enforced and covered by tests.
- Every documented exception group has an executable test.
- All H1 work items are checked with evidence.

### Deferred and follow-ups

- Prefetch fidelity, cycle accuracy and interrupt-delivery equivalence are
  [H5](#h5--accuracy-and-evidence), not H1.
- A complete 68000 disassembler follows coverage; the M4 disassembler stays bounded to the
  implemented subset ([M4-FU1](#m4--debugging-depth)).

## H2 — Video, VDP and DMA

- **Roadmap track:** [H2](roadmap.md#h2--video-vdp-and-dma) — parallel hardware-depth track,
  not a stage.
- **Status:** `Partial`
- **Integration state:** n/a (track)
- **Objective:** a VDP model that renders and transfers data, not only a memory-mapped port
  window.
- **Depends on:** H1 (instruction coverage for realistic ROM code) and the stable
  non-rendering state model from M1.
- **Evidence:** implemented VDP port/VRAM/VINT path in [Mega Drive
  slice](../architecture/megadrive-slice.md); FR-MD-007 and FR-MD-009 in
  [functional requirements](../requirements/functional.md).

### Work items

- [x] VDP status/control port window, registers, 64 KiB VRAM and versioned VDP state in
      snapshots and state hashes.
- [x] CPU-side VINT pending/acknowledge path with SR-mask gated interrupt entry.
- [x] VDP and bus inspector views for the modelled registers and VRAM (M4).
- [ ] Rendering: planes, sprites, scrolling and the visible-frame pipeline.
- [ ] CRAM and VSRAM.
- [ ] DMA transfers and their timing relative to the CPU.
- [ ] Raster effects and H/V timing.
- [ ] VDP interrupt-enable registers and VDP-side acknowledge logic.
- [ ] FIFO behavior where it affects observable results.

### Exit criteria

- Rendering, DMA, CRAM/VSRAM and raster timing are implemented and covered by differential or
  reference evidence.
- New device state enters canonical snapshots and state hashes through versioned payloads.
- All H2 work items are checked with evidence.

### Deferred and follow-ups

- DMA and rendering inspector views are [M4-FU2](#m4--debugging-depth) and require this model
  first.
- GUI video presentation and frame pacing are [M7](#m7--gui); audio is
  [H3](#h3--audio-and-z80).

## H3 — Audio and Z80

- **Roadmap track:** [H3](roadmap.md#h3--audio-and-z80) — parallel hardware-depth track, not
  a stage.
- **Status:** `Planned`
- **Integration state:** n/a (track)
- **Objective:** model the second CPU and the sound chips instead of returning typed errors
  for those windows.
- **Depends on:** H1 (bus and CPU discipline) and H2 (bus-arbitration context).
- **Evidence:** absent by design; FR-MD-008 in
  [functional requirements](../requirements/functional.md) and the not-implemented list in
  [Mega Drive slice](../architecture/megadrive-slice.md#not-implemented).

### Work items

- [ ] Z80 CPU core and its memory map.
- [ ] Bus arbitration between 68000, Z80 and VDP.
- [ ] YM2612 FM synthesis at a documented accuracy level.
- [ ] PSG tone generation at a documented accuracy level.
- [ ] Audio sample production with timing relative to emulated ticks.
- [ ] Z80 and audio state in snapshots and state hashes through a versioned payload.

### Exit criteria

- Z80 execution and bus arbitration are implemented and tested.
- YM2612 and PSG produce documented, deterministic output for reference programs.
- All H3 work items are checked with evidence.

### Deferred and follow-ups

- Audio/Z80 inspector views are [M4-FU2](#m4--debugging-depth) until the device model exists.
- Bit-exact hardware audio and cycle accuracy are [H5](#h5--accuracy-and-evidence)
  questions; host audio output is [M7](#m7--gui).

## H4 — Cartridge, controllers and timers

- **Roadmap track:** [H4](roadmap.md#h4--cartridge-controllers-and-timers) — parallel
  hardware-depth track, not a stage.
- **Status:** `Partial`
- **Integration state:** n/a (track)
- **Objective:** load and identify cartridge images and model the full peripheral set the
  slice currently narrows.
- **Depends on:** M1 (the slice and its bus) and H1 for the code paths involved.
- **Evidence:** implemented 3-button data-register reads in [Mega Drive
  slice](../architecture/megadrive-slice.md#implemented-controller-port-input-for-tas),
  verified by [M3 hardware acceptance plan](../quality/m3-hardware-acceptance.md) rows
  M3-A45–A53; the phase 8.1 cartridge device (header/region/TMSS/checksum/mapper/SRAM) in
  [Mega Drive
  slice](../architecture/megadrive-slice.md#implemented-cartridge-region-and-tmss-m8-phase-81),
  reviewed PASS 2026-11-17; FR-MD-010 and FR-MD-011 in
  [functional requirements](../requirements/functional.md).

### Work items

- [x] TH-high 3-button controller data-register reads on ports 1–2, driven by the per-step
      `InputFrame` with no canonical state change (M3-A45–A53,
      [ADR-0009](../adr/0009-real-hardware-input-tas.md)).
- [ ] TH latch and TH-low phase, with A and Start readable.
- [ ] 6-button pad support (X/Y/Z/Mode).
- [ ] Controller timing and controller/timer interrupt behavior.
- [ ] Port 3 and analog/keyboard hardware.
- [x] ROM header, region, TMSS and checksum handling, plus a loader and ROM identity
      (EPIC-0002 E2-11): phase 8.1's cartridge device, reviewed PASS 2026-11-17.
- [ ] Mapper variants beyond the Sega banking, images above the 64 MiB ceiling and SRAM
      banking schemes other than the 64 KiB window (phase 8.6 residual).
- [ ] Cartridge mapper (including SSF2-style banking) and SRAM.
- [ ] Timer interrupts.

### Exit criteria

- A cartridge image with a recognized header/region loads, is identified and maps correctly,
  with SRAM persistence where supported.
- The controller protocol extends beyond the TH-high 3-button subset and is verified against a
  reference or synthetic fixture.
- Timer interrupts are implemented and tested.
- All H4 work items are checked with evidence.

### Deferred and follow-ups

- Mapper/SRAM inspector views are [M4-FU2](#m4--debugging-depth).
- Cartridge loading is not M3 movie import/export (M3-FU1).
- Interrupt-delivery equivalence is [H5](#h5--accuracy-and-evidence); no commercial-ROM
  compatibility claim follows from this track.

## H5 — Accuracy and evidence

- **Roadmap track:** [H5](roadmap.md#h5--accuracy-and-evidence) — parallel hardware-depth
  track, not a stage.
- **Status:** `Partial`
- **Integration state:** n/a (track)
- **Objective:** measure accuracy instead of asserting it.
- **Depends on:** H1–H4 progress and the existing oracle harness.
- **Evidence:** [oracle differential evidence](../research/oracle-differential-evidence.md);
  the differential results in [quality gates](../quality/quality-gates.md); FR-EXEC-006 in
  [functional requirements](../requirements/functional.md) and NFR-DET-005 in
  [non-functional requirements](../requirements/non-functional.md).

### Work items

- [x] Pinned-oracle differential for the historical three synthetic ROMs, with negative
      controls and a default-quantum masked-ROM negative gating control.
- [x] A broader differential corpus beyond the historical three synthetic ROMs: eleven
      synthetic ROMs on the master-cycle timeline at 11/11 address, cycle, final-state and
      negative-control comparisons.
- [ ] Positive differential comparison of interrupt delivery and timing, which needs an
      oracle model of those semantics.
- [ ] Cycle accuracy including prefetch and bus timing, at a documented level.
- [ ] Cross-platform determinism measured for a real backend (FR-EXEC-006, NFR-DET-005).
- [ ] Performance measurement after correctness.

### Exit criteria

- Each accuracy claim has executable evidence, not a code-reading argument.
- Interrupt delivery/timing has positive differential evidence, or the architecture docs
  state the absence of that claim.
- Cross-platform determinism is measured, not assumed.
- All H5 work items are checked with evidence.

### Deferred and follow-ups

- No game-compatibility claim follows from accuracy work alone.
- CI matrices and fuzzing stay cross-cutting; H5 owns the accuracy and evidence conclusions.
