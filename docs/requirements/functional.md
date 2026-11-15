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
| FR-EXEC-002 | Inject input deterministically at defined execution points. | Implemented | `InputFrame`; synthetic `APPLY_INPUT` tests |
| FR-EXEC-003 | Produce a canonical state digest after each step. | Implemented | `StepOutcome::state_hash` |
| FR-EXEC-004 | Restore and replay identical inputs to the same digest. | Implemented | Restore/replay acceptance test |
| FR-EXEC-005 | Detect divergence at the first differing tick. | Implemented | `Movie::verify` reports the first mismatching step as the recorded tick, input index, expected hash and observed hash; the library and CLI paths are verified by M3-A4 in the [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md) and the dated [quality gates](../quality/quality-gates.md) result. Replay stops at the first mismatch; there is no resynchronization. Branch comparison additionally reports the first differing recorded index, tick and state hash between two movies without re-simulating (M3-A15, t5 PASS 2026-11-15); a compare PASS is a record diff, not a verification. |
| FR-EXEC-006 | Guarantee cross-platform determinism for a real backend. | Planned | Requires differential tests |
| FR-EXEC-007 | Frame pacing, run loop and rewind. | Planned | Not in bootstrap |

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
| FR-MD-010 | Controller input and multi-port schema. | Planned | Input is not consumed yet; M3 v1 records only synthetic-backend input and real-hardware input TAS remains deferred because the Mega Drive `step` ignores input ([ADR-0004](../adr/0004-tas-movie-format.md) §12). |
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
| FR-TAS-001 | Movie record/playback, frame advance, rerecord, seek | Partial | Implemented and independently verified for the deterministic synthetic backend: v1 record/playback of a per-tick input sequence, replay verification with exact first divergence, restart and M2-seeded replay, and the `chronofork tas record/play/verify` CLI (M3-A3–A7, [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md); M3-T4 PASS 2026-11-15). Increment 2 adds ranged replay/seek over an explicit `(from, until]` window with typed range bounds, forward single-tick advance and rewind by deterministic linear re-simulation from the embedded checkpoint (O(n); no carried state), and branch comparison of recorded `(tick, state hash)` records that reports the first divergence plus separate length and checkpoint-identity mismatches without re-simulating (M3-A13–A17, [increment-2 acceptance plan](../quality/m3-partial-replay-acceptance.md); t5 PASS 2026-11-15). Still deferred: rerecord and branch editing, checkpoint anchors/periodic checkpoints and seek via checkpoints (format v2), an in-process rewind buffer/state cache, multi-controller/ports/analog/keyboard input, and real-hardware input TAS. See [ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md), [ADR-0004](../adr/0004-tas-movie-format.md) and [quality gates](../quality/quality-gates.md). |
| FR-TAS-002 | Versioned canonical movie format with branch metadata | Implemented | The canonical v1 `.cfmovie` container in `chronofork_sessions::movie` embeds the verbatim canonical checkpoint and per-step expected outcomes, carries optional name/parent metadata, applies the 256 MiB / 4,194,304-step / 256-byte limits and rejects incompatible input with no migration. Verified by M3-A1, M3-A2, M3-A8, M3-A11 and M3-A12 (M3-T4 PASS, 2026-11-15); see [ADR-0004](../adr/0004-tas-movie-format.md), the [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md) and [quality gates](../quality/quality-gates.md). |
| FR-API-001 | Minimal CLI for implemented features only | Implemented | Existing `demo`, `megadrive`, `backend`, `session` and `tas record/play/verify` surfaces; the `tas` surface is verified by M3-A7. No HTTP, MCP or GUI surface is exposed. |
| FR-API-002 | HTTP API over the same application services | Planned | Not implemented |
| FR-API-003 | MCP over Streamable HTTP, not the source of truth | Planned | Not implemented |
| FR-API-004 | External SDKs | Planned | Not implemented |
| FR-SCRIPT-001 | Sandboxed scripting with explicit limits | Planned | Not implemented |
| FR-GUI-001 | Optional GUI that does not own canonical state | Planned | Not implemented |

FR-TAS-001 is **Partial**: v1 input-sequence record/playback, replay verification, and
the increment-2 ranged replay/seek, forward single-tick advance, rewind by re-simulation
and branch comparison are implemented and independently verified on the deterministic
synthetic backend, while rerecord and branch editing, checkpoint-anchored seek (format
v2), an in-process rewind buffer, multi-controller/ports/analog/keyboard input and
real-hardware input TAS remain deferred. FR-TAS-002 and FR-EXEC-005 are **Implemented**
for the same verified scope. M3 is **not complete**; see the [M3
acceptance plan](../quality/m3-tas-replay-acceptance.md), the [increment-2 acceptance
plan](../quality/m3-partial-replay-acceptance.md) and the dated [quality
gates](../quality/quality-gates.md) result.

## Licensing and provenance

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-LICENSE-001 | Preserve license/attribution for every incorporated component. | Implemented | `LICENSE`, `THIRD_PARTY_NOTICES.md`, provenance |
| FR-LICENSE-002 | Do not commit ROMs, firmware or unlicensed fixtures. | Implemented | `.gitignore`, synthetic program only |
| FR-LICENSE-003 | Record exact upstream revisions for research. | Implemented | `docs/research/upstream-repositories.md` |
| FR-LICENSE-004 | Require approval before changing project license or incorporating restrictive code. | Implemented | Policy + ADR-0002 proposal |
