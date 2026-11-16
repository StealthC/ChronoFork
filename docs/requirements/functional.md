# Functional Requirements

Status values: **Implemented** (exercised by tests), **Partial** (contract exists,
behavior incomplete), **Planned** (roadmap), **Deferred** (explicitly out of
bootstrap).

## Sessions and lifecycle

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-SESSION-001 | Create an independent session with its own mutable backend state. | Implemented | `SessionManager::create_session`; acceptance tests |
| FR-SESSION-002 | Support step, pause, resume, reset and close with explicit transitions. | Implemented | `SessionState`; lifecycle test |
| FR-SESSION-003 | Reject execution while paused or closed with a typed error. | Implemented | `SessionError::NotRunning` / `Closed`; lifecycle test |
| FR-SESSION-004 | Inspect tick, state digest, registers, memory and lineage without side effects. | Implemented | `SessionInspection`; `Backend::inspect` |
| FR-SESSION-005 | Close is idempotent and does not corrupt other sessions. | Implemented | `Session::close`; lifecycle test |
| FR-SESSION-006 | Persist named session metadata and explicit writer ownership across process restarts. | Implemented | `SnapshotRepository` layout v1, session/snapshot metadata and durable keys; explicit per-session writer lock/lease with typed contention and owner-checked recovery. Restart, metadata, ownership, isolation and retention targets pass; M2-A1/A2/A5/A11 verified in two independent rounds (2026-11-15), including the repaired CLI continue-and-save lock scope. See the [M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md) and [quality gates](../quality/quality-gates.md). |

## Snapshots

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-SNAPSHOT-001 | Capture complete backend state with tick and state hash. | Implemented | `BackendSnapshot`; synthetic snapshot tests |
| FR-SNAPSHOT-002 | Serialize with magic, format version and SHA-256 checksum. | Implemented | `Snapshot::to_bytes` / `from_bytes` |
| FR-SNAPSHOT-003 | Restore a checkpoint and reject incompatible/corrupt input. | Implemented | Round-trip, corruption, version tests |
| FR-SNAPSHOT-004 | Import serialized bytes into a new manager/session. | Implemented | `create_session_from_snapshot_bytes`; acceptance test |
| FR-SNAPSHOT-005 | Version backend payloads independently from the outer envelope. | Implemented | `BACKEND_SNAPSHOT_FORMAT_VERSION`; synthetic payload version |
| FR-SNAPSHOT-006 | Persist checkpoints to a versioned storage layout with explicit list/delete retention and no automatic garbage collection. | Implemented | Versioned `CFSTORE1` root with per-session/per-snapshot directories; `checkpoint.cfsnap` is exactly `Snapshot::to_bytes()`; deterministic list, explicit non-cascading delete and no automatic GC. Compatibility/rejection, crash-recovery, determinism, retention and CLI targets pass; M2-A1–A4/A8/A11 and the CLI contract verified in two independent rounds (2026-11-15). See the [M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md) and [quality gates](../quality/quality-gates.md). |
| FR-SNAPSHOT-007 | Guarantee atomic restore on failure for real backends. | Partial | Contract states it; synthetic backend validates before mutation |

## Forks and lineage

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-FORK-001 | Fork from a stored checkpoint into a fresh independent backend. | Implemented | `fork_from_snapshot`; three-way fork test |
| FR-FORK-002 | Fork from current state. | Implemented | `fork_current`; acceptance test |
| FR-FORK-003 | Record parent session, origin snapshot and depth. | Implemented | `Lineage`; acceptance test |
| FR-FORK-004 | Ensure a fork starts at the checkpoint tick/hash. | Implemented | Acceptance test |
| FR-FORK-005 | Ensure branches do not mutate each other. | Implemented | State and memory isolation tests |
| FR-FORK-006 | Compare two sessions by tick and state digest. | Implemented | `SessionManager::compare` |
| FR-FORK-007 | Copy-on-write or shared-page forks. | Deferred | Correctness first; not implemented |

## Execution and determinism

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-EXEC-001 | Advance explicit virtual time; no implicit host-clock input to hardware. | Implemented | `Tick`; synthetic backend |
| FR-EXEC-002 | Inject input deterministically at defined execution points. | Implemented | `InputFrame`; synthetic `APPLY_INPUT` tests. Schema 2 multi-device frames (four digital ports, four analog axes and a keyboard bitmap) are implemented and independently verified for the deterministic synthetic backend (M3-A22, t7 PASS 2026-11-15); the v1 constructor/accessor/neutral behavior is preserved and v1 state hashes are unchanged. See [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) and FR-TAS-004. Real-hardware controller input is implemented and independently verified for the Mega Drive TH-high 3-button subset on ports 1–2 (M3-A45–A53, [M3 hardware acceptance plan](../quality/m3-hardware-acceptance.md); t5 PASS 2026-11-16): the per-step `InputFrame` drives `0x00A10003`/`0x00A10005` without being serialized into the snapshot/state/config; A/Start, TH/6-button, timing, analog/keyboard hardware and port 3 remain deferred ([ADR-0009](../adr/0009-real-hardware-input-tas.md)). |
| FR-EXEC-003 | Produce a canonical state digest after each step. | Implemented | `StepOutcome::state_hash` |
| FR-EXEC-004 | Restore and replay identical inputs to the same digest. | Implemented | Restore/replay acceptance test |
| FR-EXEC-005 | Detect divergence at the first differing tick. | Implemented | `Movie::verify` reports the first mismatching step as the recorded tick, input index, expected hash and observed hash; the library and CLI paths are verified by M3-A4 in the [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md) and the dated [quality gates](../quality/quality-gates.md) result. Replay stops at the first mismatch; there is no resynchronization. Branch comparison additionally reports the first differing recorded index, tick and state hash between two movies without re-simulating (M3-A15, t5 PASS 2026-11-15); a compare PASS is a record diff, not a verification. |
| FR-EXEC-006 | Guarantee cross-platform determinism for a real backend. | Planned | Requires differential tests |
| FR-EXEC-007 | Frame pacing, run loop and rewind. | Partial | Rewind is implemented: an in-process rewind state cache plus deterministic rewind by restoring the nearest verified cached state (or the embedded checkpoint) and replaying recorded inputs, with the interactive `chronofork tas studio` stepping loop. Implemented and independently verified for the deterministic synthetic backend ([ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md); M3-A28–A34 in the [M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md); t5 PASS 2026-11-15). Frame pacing and a host-time run loop are still absent, so this requirement stays **Partial** and is never promoted to Implemented on rewind alone. The cache is process-local (no persistent or shared cache) and rewind is not O(1). |
| FR-EXEC-008 | Bounded in-process rewind state cache and interactive stepping over a recorded movie. | Implemented (synthetic backend and minimal Mega Drive movie) | [ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md) is implemented and independently verified: a per-player `SessionManager` with one imported session and a bounded canonical-state cache (one-step default interval, 256-entry / 16 MiB defaults, 65,536-entry / 64 MiB maxima, deterministic `payload+256` metric, evict-lowest-first), deterministic rewind by restore plus verified forward replay with exact first divergence, and the stdin `chronofork tas studio` surface with fixed blocks and fail-closed errors. Evidence: M3-A28–A34 in the [M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md) (t5 PASS 2026-11-15). Scope includes the deterministic synthetic backend and the minimal Mega Drive slice (hardware `tas studio` was verified in M3-A47); no persistent/cross-process cache, O(1)/reverse-execution seek, controller-timing or hardware-accuracy claim is made. Rerecord/branch editing is covered by FR-TAS-005. |

## Backend contract

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-BACKEND-001 | Report capabilities as real guarantees. | Implemented | `Capabilities`; CLI output |
| FR-BACKEND-002 | Identify test backends explicitly. | Implemented | `is_test_backend`; CLI check |
| FR-BACKEND-003 | Validate snapshot backend identity, version and config hash. | Implemented | Synthetic and Mega Drive `restore` tests |
| FR-BACKEND-004 | Provide side-effect-free inspection and memory reads. | Implemented | `inspect`, `read_memory` |
| FR-BACKEND-005 | Provide debugger memory writes distinct from bus writes. | Implemented | `write_memory`; isolation test |
| FR-BACKEND-006 | Support multiple independent backend instances per manager. | Implemented | 3-way fork test |
| FR-BACKEND-007 | Select and integrate a real Mega Drive backend. | Partial | ADR-0002 accepted; minimal 68000 + ROM/RAM slice implemented |

## Mega Drive minimal slice

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-MD-001 | Load reset vectors from ROM and initialize `SP`/`PC`/`SR`. | Implemented | `M68000::reset`; CPU/backend tests |
| FR-MD-002 | Execute a documented 68000 subset deterministically. | Implemented | `crates/chronofork-megadrive`; unit/integration tests (see [quality gates](../quality/quality-gates.md)) |
| FR-MD-003 | Map ROM and 64 KiB work RAM big-endian. | Implemented | Bus tests |
| FR-MD-004 | Reject unmapped, odd-address and ROM-write accesses. | Implemented | Bus/backend error tests |
| FR-MD-005 | Snapshot/restore complete slice state with config/state hashes. | Implemented | Payload v2 covers CPU, RAM, VDP and scheduler; round-trip/mismatch tests |
| FR-MD-006 | Fork/restore a real-core session without source mutation. | Implemented | Session integration test |
| FR-MD-007 | VDP status/control, VRAM and DMA. | Partial | Port window, registers, 64 KiB VRAM and versioned VDP snapshot implemented; DMA, CRAM/VSRAM, rendering and raster timing absent |
| FR-MD-008 | YM2612, PSG and audio. | Planned | Not implemented |
| FR-MD-009 | Z80, interrupts and exception processing. | Partial | Deterministic HBlank/VBlank scheduler, 68000 SR-mask gated interrupt entry, `RTE` handler return and VDP VINT-pending/acknowledge implemented for the modelled slice; no Z80, timer/controller interrupt sources, other exception groups, privilege enforcement or interrupt-timing equivalence; only HBlank level 4 and VBlank level 6 sources exist, and level 7 is a predicate rule only |
| FR-MD-010 | Controller input and multi-port schema. | Implemented (3-button TH-high subset; ports 1–2) | The Mega Drive slice reads the controller data registers `0x00A10003`/`0x00A10005` from the per-step `InputFrame` ports 1–2, active-low TH-high 3-button subset (Up/Down/Left/Right/B/C), with a transient non-serialized step-scoped field and no canonical snapshot/state/config change. `--rom` selects the backend for `tas record/play/verify/studio/edit`; `compare` stays backend-agnostic. [ADR-0009](../adr/0009-real-hardware-input-tas.md) is accepted, implemented and independently verified; [M3 hardware acceptance plan](../quality/m3-hardware-acceptance.md) rows M3-A45–A53 are `executed - passed` (t5 PASS 2026-11-16), with the canonical payload/state/config, demos, M1 evidence and existing movies byte-identical to baseline. Limits: A and Start, TH latch/6-button/Start multiplexing, controller timing, analog/keyboard hardware, port 3, real commercial ROMs and cycle accuracy are not implemented or claimed; the acceptance fixture is the independently authored synthetic `controller_rom()`. |
| FR-MD-011 | ROM header/region/TMSS and cartridge mapper. | Planned | Not implemented |
| FR-MD-012 | Differential traces against the Exodus oracle. | Implemented | Pinned oracle built; post-repair harness passes 3/3 ROMs (`rom_alu` 99/99 and `rom_mem_vdp` 51/51 at the explicit 1,000,000-instruction controlled window, `rom_masked` 155/155 at the CLI default quantum 64 with the flag omitted), all final states equal, negative controls 3/3; the earlier two-ROM run is preserved as pre-E2-15 controlled-condition history, and the pre-repair divergence/E2-13 repair remain recorded |

Tick semantics: one executed 68000 instruction equals one ChronoFork tick in this
slice. The interrupt scheduler maps a configurable number of instruction ticks to
one scanline (default 64) and derives NTSC/PAL line and frame constants from that.
This is deterministic but **not** cycle-accurate hardware timing.

## Debugging, tracing, TAS and interfaces

| ID | Requirement | Status | Evidence / scope |
|---|---|---|---|
| FR-DEBUG-001 | Register inspection | Implemented (synthetic + minimal 68000) | `Backend::inspect`; backend tests |
| FR-DEBUG-002 | Conditional breakpoints | Planned | Not implemented |
| FR-DEBUG-003 | Memory watchpoints | Planned | Not implemented |
| FR-DEBUG-004 | Event breakpoints, logpoints, interrupt monitoring | Partial | Pending interrupt/vector inspection; no breakpoints or logpoints |
| FR-TRACE-001 | Structured instruction/bus/device traces with emulated timestamps | Partial | Narrow debug-only `index`/`pc`/`opcode` ring and TSV export; no timestamps, bus/device events or general structured format |
| FR-TRACE-002 | Trace comparison and historical queries | Partial | Executed-address projection, first-divergence report, opcode/ROM-byte audit and negative control against the pinned oracle; no historical queries or general trace tooling |
| FR-TAS-001 | Movie record/playback, frame advance, rerecord, seek | Implemented | Implemented and independently verified: v1/v2 record/playback of a per-step input sequence, replay verification with exact first divergence, restart and M2-seeded replay, the `chronofork tas record/play/verify` CLI, ranged replay/seek, forward single-tick advance and rewind by deterministic re-simulation, branch comparison, `tas studio`/rewind cache, `tas edit` rerecord/branch editing, and real-hardware input TAS on the minimal Mega Drive slice (`--rom` on record/play/verify/studio/edit; TH-high 3-button controller reads). Evidence: [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md) M3-A1–A12, [increment-2](../quality/m3-partial-replay-acceptance.md) M3-A13–A17, [format-v2](../quality/m3-format-v2-acceptance.md) M3-A18–A27, [rewind](../quality/m3-rewind-acceptance.md) M3-A28–A34, [rerecord](../quality/m3-rerecord-acceptance.md) M3-A35–A44 and [hardware](../quality/m3-hardware-acceptance.md) M3-A45–A53, all `executed - passed` (t5 PASS 2026-11-16), with the dated [quality gates](../quality/quality-gates.md) results. Import/export adapters (for example BizHawk BK2) remain documented-only and are not claimed; the hardware claim is the synthetic 68000 controller-reading ROM and 3-button subset, not real commercial ROMs or cycle accuracy. M3 is complete for the documented deterministic minimal-slice scope. |
| FR-TAS-002 | Versioned canonical movie format with branch metadata | Implemented | The canonical v1 `.cfmovie` container in `chronofork_sessions::movie` embeds the verbatim canonical checkpoint and per-step expected outcomes, carries optional name/parent metadata, applies the 256 MiB / 4,194,304-step / 256-byte limits and rejects incompatible input with no migration. Verified by M3-A1, M3-A2, M3-A8, M3-A11 and M3-A12 (M3-T4 PASS, 2026-11-15); see [ADR-0004](../adr/0004-tas-movie-format.md), the [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md) and [quality gates](../quality/quality-gates.md). The v2 `CFMOVIE2` container (format 2 / schema 2, ordered canonical anchors, 72-byte multi-device step records, reject-only v1/v2 compatibility and no migration) is implemented and independently verified for the deterministic synthetic backend (M3-A18, M3-A19, M3-A23–A27, t7 PASS 2026-11-15); see [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) and FR-TAS-003. |
| FR-TAS-003 | Checkpoint anchors in a versioned movie container and anchored seek | Implemented (synthetic backend) | [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) is implemented and independently verified: the `CFMOVIE2` container carries an ordered table of canonical checkpoint anchors, each parsed only by `Snapshot::from_bytes` and bound to its recorded step tick/state hash; nearest-anchor-at-or-before-prefix seek reuses the single ranged replay path and the existing checkpoint import. Anchored ranged replay reports `anchored=1`, `anchor_step`, `anchor_tick` and `prefix_verified=0`, does not re-execute the pre-anchor prefix, and full-range `verify` remains complete. Evidence: M3-A18–A21/A24–A25 in the [format-v2 acceptance plan](../quality/m3-format-v2-acceptance.md) (t7 PASS 2026-11-15). The anchor/seek acceptance claim is the deterministic synthetic backend on Linux; no O(1) claim is made. Hardware controller input is covered by FR-MD-010 and FR-TAS-001. |
| FR-TAS-004 | Versioned multi-device input schema (ports, analog axes, keyboard) | Implemented (synthetic backend) | [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) is implemented and independently verified: `InputFrame` carries four 32-bit digital ports (port 0 keeps the v1 mask), four signed 16-bit axes and a 64-bit keyboard bitmap, one frame per step; `new`/`buttons`/`NEUTRAL` keep v1 behavior; the synthetic backend consumes non-neutral fields deterministically with unchanged payload layout and unchanged v1 state hashes; v2 record/replay and CLI flags are covered. Evidence: M3-A22 and M3-A24 in the [format-v2 acceptance plan](../quality/m3-format-v2-acceptance.md) (t7 PASS 2026-11-15). The generic schema is implemented and verified for the deterministic synthetic backend; the Mega Drive backend consumes ports 0/1 for the TH-high 3-button subset and treats ports 2/3, axes and keys as inert, as verified by M3-A45–A53. |
| FR-TAS-005 | Edit a recorded movie's step inputs and/or truncate it, deterministically re-simulate the suffix and emit a new branch artifact without mutating the source. | Implemented (synthetic backend) | [ADR-0008](../adr/0008-rerecord-and-branch-editing.md) is implemented and independently verified: `chronofork_sessions::edit` and `chronofork tas edit` perform full-`InputFrame` replacement at the library layer (port 0 only from the CLI), zero-based `--truncate-after`, prefix copy/verification, deterministic suffix re-simulation with every expectation recomputed, and a new artifact with its own non-zero movie id and `parent_movie_id` = the source id. `keep-prefix`/`drop`/`rebuild` anchor handling preserves the embedded checkpoint and uses the canonical fixed rebuilt-anchor identity; v1→v1 and v2→v2 preservation is reject-only with typed index/count/anchor/output bounds and an atomic publish that never writes the source. Evidence: [M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md) rows M3-A35–A44 `executed - passed` (t10 repair-round-2 PASS 2026-11-16), raw artifacts under `.research/m3-rerecord/verification-round-2/` and the dated [quality gates](../quality/quality-gates.md) result. The edit claim is the deterministic synthetic backend on Linux; real-hardware input TAS is implemented and independently verified for the TH-high 3-button subset (M3-A45–A53), while in-place/timeline editing, persisted counters, import/export and O(1) seek remain unimplemented. `FR-TAS-001` is **Implemented** and M3 is complete for the documented deterministic minimal-slice scope. |
| FR-API-001 | Minimal CLI for implemented features only | Implemented | Existing `demo`, `megadrive`, `backend`, `session` and `tas record/play/verify/compare/studio/edit` surfaces; the v1 `tas` surface is verified by M3-A7, the v2 flags/anchored/compare surface by M3-A24 (t7 PASS 2026-11-15), the `tas studio` stdin surface by M3-A30 (t5 PASS 2026-11-15) and the `tas edit` process/atomicity surface by M3-A40 (t10 repair-round-2 PASS 2026-11-16). The `--rom <FILE>` backend-selection flag on `tas record/play/verify/studio/edit` is implemented and independently verified (`compare` stays backend-agnostic), with bounded ROM reads, typed wrong-ROM/checkpoint errors, the rejected `record --rom --root` combination and an unchanged synthetic default; evidence M3-A49 in the [M3 hardware acceptance plan](../quality/m3-hardware-acceptance.md) (t5 PASS 2026-11-16). No HTTP, MCP or GUI surface is exposed. |
| FR-API-002 | HTTP API over the same application services | Planned | Not implemented |
| FR-API-003 | MCP over Streamable HTTP, not the source of truth | Planned | Not implemented |
| FR-API-004 | External SDKs | Planned | Not implemented |
| FR-SCRIPT-001 | Sandboxed scripting with explicit limits | Planned | Not implemented |
| FR-GUI-001 | Optional GUI that does not own canonical state | Planned | Not implemented |

FR-TAS-001 is **Implemented**: v1/v2 input-sequence record/playback, replay verification,
the increment-2 ranged replay/seek, forward single-tick advance, rewind by re-simulation
and branch comparison, the increment-3 `CFMOVIE2` anchors/anchored seek and schema-2
multi-device input, the increment-4 in-process rewind state cache and interactive
`tas studio` stepping, the increment-5 rerecord/branch editing, and the final
real-hardware input TAS increment (`--rom` on `tas record/play/verify/studio/edit`, with
TH-high 3-button controller reads on ports 1–2) are implemented and independently verified;
M3-A45–A53 are `executed - passed` (t5 PASS 2026-11-16). `FR-TAS-005` is **Implemented**
on the same evidence. Import/export adapters (for example BizHawk BK2) remain
documented-only and are not claimed; they are not part of this requirement. The hardware
claim is the independently authored synthetic 68000 `controller_rom()` and the 3-button
subset, not real commercial ROMs, controller timing or cycle accuracy.
FR-EXEC-007 is **Partial** (rewind and interactive stepping done; frame pacing and a
host-time run loop absent); FR-EXEC-008, FR-TAS-002, FR-TAS-003, FR-TAS-004, FR-TAS-005
and FR-EXEC-005 are **Implemented** for the same verified scope; FR-MD-010 is
**Implemented (3-button TH-high subset; ports 1–2)**. **M3 is complete for the documented
deterministic minimal-slice scope** (t5 PASS 2026-11-16; t6 promotion; t7 promotion audit
pending); see the [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md), the
[increment-2 acceptance plan](../quality/m3-partial-replay-acceptance.md), the
[format-v2 acceptance plan](../quality/m3-format-v2-acceptance.md), the [M3 rewind
acceptance plan](../quality/m3-rewind-acceptance.md), the [M3 rerecord acceptance
plan](../quality/m3-rerecord-acceptance.md), the [M3 hardware acceptance
plan](../quality/m3-hardware-acceptance.md) and the dated [quality
gates](../quality/quality-gates.md) result.

## Licensing and provenance

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-LICENSE-001 | Preserve license/attribution for every incorporated component. | Implemented | `LICENSE`, `THIRD_PARTY_NOTICES.md`, provenance |
| FR-LICENSE-002 | Do not commit ROMs, firmware or unlicensed fixtures. | Implemented | `.gitignore`, synthetic program only |
| FR-LICENSE-003 | Record exact upstream revisions for research. | Implemented | `docs/research/upstream-repositories.md` |
| FR-LICENSE-004 | Require approval before changing project license or incorporating restrictive code. | Implemented | Policy + ADR-0002 proposal |
