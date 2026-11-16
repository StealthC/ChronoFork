# Milestones

Milestones are verifiable checkpoints, not dates. A milestone is complete only when its
exit criteria are demonstrated by executed tests or recorded evidence. There is one
milestone per roadmap stage (M0–M8) plus the parallel hardware-depth track H1–H5, which is
a track, not a stage and not M9. This page is the canonical home of status, integration
state, evidence and follow-ups; [Roadmap](roadmap.md) records stage intent.

## How to read this document

- **One milestone per stage.** M0–M8 map one-to-one to roadmap Stages 0–8. The mapping is
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
| M5 | [Stage 5 — Tracing and scripting](roadmap.md#stage-5--tracing-and-scripting) | `Next` | not started |
| M6 | [Stage 6 — Agent-native interfaces](roadmap.md#stage-6--agent-native-interfaces) | `Planned` | not started |
| M7 | [Stage 7 — Human-facing GUI](roadmap.md#stage-7--human-facing-gui) | `Planned` | not started |
| M8 | [Stage 8 — Multi-backend expansion](roadmap.md#stage-8--multi-backend-expansion) | `Planned` | not started |

Hardware-depth track (parallel, not a stage):

| Track | Status | Objective |
|---|---|---|
| [H1 — CPU and instruction completeness](#h1--cpu-and-instruction-completeness) | `Partial` | Broader 68000 instruction/addressing coverage, privilege, exception groups |
| [H2 — Video, VDP and DMA](#h2--video-vdp-and-dma) | `Partial` | Rendering, CRAM/VSRAM, DMA, raster and H/V timing |
| [H3 — Audio and Z80](#h3--audio-and-z80) | `Planned` | Z80, bus arbitration, YM2612 and PSG |
| [H4 — Cartridge, controllers and timers](#h4--cartridge-controllers-and-timers) | `Partial` | ROM identity, mapper/SRAM, full controller protocol, timers |
| [H5 — Accuracy and evidence](#h5--accuracy-and-evidence) | `Partial` | Differential corpus, interrupt timing, cycle accuracy, cross-platform determinism |

## Next

**M5 — Tracing and scripting is `Next`.** It starts from the M4 observation surface plus
the M2/M3 session and movie layers, so it is not blocked by hardware-depth work; H1–H5
proceed in parallel. M0–M4 are merged to `main`; M4 was merged at `4c595a2` (PR #2) after
its verification on `agent/m4-debugging-depth` at `6d30243`.

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
- [x] Differential trace evidence against the pinned Exodus oracle: three synthetic ROMs
      match on projected address streams and selected final state, negative controls are
      caught, and the masked-ROM default-quantum control crosses two HBlank boundaries with
      the SR mask at 7 and no handler entry. Current run, controlled windows and pre-repair
      history are in [oracle differential
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
- Limits carried by the modelled slice and owned by H: privilege is not enforced; only
  HBlank level 4 and VBlank level 6 sources exist, with level 7 a predicate rule only; no
  interrupt-timing equivalence is claimed; VDP interrupt-enable registers and VDP-side
  acknowledge logic are not modelled; controller input is the TH-high 3-button subset on
  ports 1–2 ([Mega Drive slice](../architecture/megadrive-slice.md#not-implemented)).
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
      stayed PASS 3/3.
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
- **Status:** `Next`
- **Integration state:** not started
- **Objective:** structured, timestamped traces and a sandboxed script runtime for
  reproducible experiments without host-language plugins.
- **Depends on:** M4 (bounded observation surface), M2 (sessions and storage) and M3
  (movies). Not blocked by H1–H5.
- **Evidence:** none yet. The scope is [Stage 5](roadmap.md#stage-5--tracing-and-scripting);
  current capability status is FR-TRACE-001, FR-TRACE-002 and FR-SCRIPT-001 in
  [functional requirements](../requirements/functional.md).

### Work items

- [ ] Versioned structured trace format for instruction, bus and device events, with
      emulated timestamps and bounded retention.
- [ ] Structured export and import with typed rejection of incompatible versions.
- [ ] Triggered and circular tracing with bounded buffers.
- [ ] Trace comparison and historical queries with first-divergence reporting.
- [ ] Sandboxed script runtime with memory and time limits and typed errors.
- [ ] Script APIs for sessions, forks, inspection, TAS and experiments.
- [ ] Process-level CLI tests for every trace and script surface.

### Exit criteria

- Scripts can run controlled experiments and produce reproducible reports without
  host-language plugins.
- Trace export round-trips, and trace comparison reports the first divergence.
- Sandbox limits are enforced, typed and tested.
- FR-TRACE-001, FR-TRACE-002 and FR-SCRIPT-001 are updated to match the delivered scope.

### Deferred and follow-ups

- Frame pacing and a host-time run loop belong to [M7](#m7--gui), not to tracing.
- GUI trace viewers are [M7](#m7--gui); HTTP/MCP transport is
  [M6](#m6--agent-interfaces).
- Trace sources for DMA, audio and Z80 wait for
  [H2](#h2--video-vdp-and-dma)/[H3](#h3--audio-and-z80); performance evidence is
  [H5](#h5--accuracy-and-evidence).

## M6 — Agent interfaces

- **Roadmap stage:** [Stage 6 — Agent-native interfaces](roadmap.md#stage-6--agent-native-interfaces)
- **Status:** `Planned`
- **Integration state:** not started
- **Objective:** expose the same services to humans, scripts and agents through CLI, HTTP
  and MCP with identical semantics.
- **Depends on:** the M2–M5 services; transport types must not enter core, backend or session
  APIs.
- **Evidence:** none yet. The scope is
  [Stage 6](roadmap.md#stage-6--agent-native-interfaces); capability status is FR-API-002,
  FR-API-003 and FR-API-004 in
  [functional requirements](../requirements/functional.md).

### Work items

- [ ] HTTP API over the existing application services.
- [ ] MCP over Streamable HTTP, backed by the same services.
- [ ] Typed tool surface instead of hundreds of loose endpoints.
- [ ] Session ownership, permissions and concurrency controls at the transport boundary.
- [ ] Machine-consumable experiment artifacts.
- [ ] CLI reimplemented as a first-class client of the shared services.
- [ ] Semantic parity tests across CLI, HTTP and MCP.
- [ ] Daemon/deployment shape for long-running sessions.

### Exit criteria

- Identical operations through CLI, HTTP and MCP with identical semantics, proven by parity
  tests.
- No transport type appears in core, backend or session APIs.
- The session layer remains the only canonical state owner.

### Deferred and follow-ups

- The GUI is [M7](#m7--gui); additional systems are [M8](#m8--multi-backend).
- Any network surface follows the repository security constraints: no telemetry and no
  upload path without explicit design approval.

## M7 — GUI

- **Roadmap stage:** [Stage 7 — Human-facing GUI](roadmap.md#stage-7--human-facing-gui)
- **Status:** `Planned`
- **Integration state:** not started
- **Objective:** optional human-facing windows for observing and controlling sessions
  without owning canonical state.
- **Depends on:** M6 (attach/detach transport) and
  [H2](#h2--video-vdp-and-dma)/[H3](#h3--audio-and-z80) (video and audio to present).
- **Evidence:** none yet. The scope is [Stage 7](roadmap.md#stage-7--human-facing-gui);
  capability status is FR-GUI-001 in
  [functional requirements](../requirements/functional.md).

### Work items

- [ ] Video presentation decoupled from canonical state.
- [ ] Audio presentation decoupled from canonical state.
- [ ] Frame pacing and a host-time run loop.
- [ ] Debugger panels, memory viewers, hardware inspectors and TAS timelines.
- [ ] Single-session and multi-session window/mosaic layouts.
- [ ] Remote observation and human/agent control handoff.
- [ ] Attach/detach tests proving unchanged emulation results.

### Exit criteria

- The GUI attaches to and detaches from sessions without changing emulation results.
- Presentation never owns or mutates canonical state.
- Human/agent control handoff is observable and deterministic.

### Deferred and follow-ups

- Frame pacing is intentionally here rather than in M5: it is a presentation-cadence concern
  and depends on [H2](#h2--video-vdp-and-dma) for video.
- Rendering and audio depth stay in H2/H3; scripting and HTTP/MCP stay in M5/M6.

## M8 — Multi-backend

- **Roadmap stage:** [Stage 8 — Multi-backend expansion](roadmap.md#stage-8--multi-backend-expansion)
- **Status:** `Planned`
- **Integration state:** not started
- **Objective:** integrate a second system behind the same capability model without changing
  session, storage or transport semantics.
- **Depends on:** the M2–M7 shared layers and the boundaries in
  [ADR-0002](../adr/0002-backend-boundaries.md).
- **Evidence:** none yet. The scope is
  [Stage 8](roadmap.md#stage-8--multi-backend-expansion); capability status is FR-API-004 in
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
- **Objective:** a documented, testable 68000 model instead of a subset that happens to run
  the current fixtures.
- **Depends on:** the M1 slice and its differential harness.
- **Evidence:** implemented subset in [Mega Drive
  slice](../architecture/megadrive-slice.md#68000-subset); [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md)
  E2-03/E2-13; FR-MD-002 and FR-MD-009 in
  [functional requirements](../requirements/functional.md).

### Work items

- [x] Documented 68000 subset with reset vectors, flags and big-endian bus discipline.
- [x] E2-13 absolute-long `MOVE` decoding repair, verified by differential evidence.
- [ ] Broader instruction and addressing-mode coverage, driven by differential tests
      (EPIC-0002 E2-13 continued).
- [ ] Privilege enforcement and separate user/supervisor stacks.
- [ ] 68000 exception groups beyond the implemented interrupt path.
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
  M3-A45–A53; FR-MD-010 and FR-MD-011 in
  [functional requirements](../requirements/functional.md).

### Work items

- [x] TH-high 3-button controller data-register reads on ports 1–2, driven by the per-step
      `InputFrame` with no canonical state change (M3-A45–A53,
      [ADR-0009](../adr/0009-real-hardware-input-tas.md)).
- [ ] TH latch and TH-low phase, with A and Start readable.
- [ ] 6-button pad support (X/Y/Z/Mode).
- [ ] Controller timing and controller/timer interrupt behavior.
- [ ] Port 3 and analog/keyboard hardware.
- [ ] ROM header, region, TMSS and checksum handling, plus a loader and ROM identity
      (EPIC-0002 E2-11).
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

- [x] Pinned-oracle differential for three synthetic ROMs, with negative controls and a
      default-quantum masked-ROM negative gating control.
- [ ] A broader differential corpus beyond the three synthetic ROMs.
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
