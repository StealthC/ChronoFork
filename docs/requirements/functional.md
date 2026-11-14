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
| FR-SESSION-006 | Support named/persistent session metadata and ownership. | Planned | Future persistence and access-control layer |

## Snapshots

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-SNAPSHOT-001 | Capture complete backend state with tick and state hash. | Implemented | `BackendSnapshot`; synthetic snapshot tests |
| FR-SNAPSHOT-002 | Serialize with magic, format version and SHA-256 checksum. | Implemented | `Snapshot::to_bytes` / `from_bytes` |
| FR-SNAPSHOT-003 | Restore a checkpoint and reject incompatible/corrupt input. | Implemented | Round-trip, corruption, version tests |
| FR-SNAPSHOT-004 | Import serialized bytes into a new manager/session. | Implemented | `create_session_from_snapshot_bytes`; acceptance test |
| FR-SNAPSHOT-005 | Version backend payloads independently from the outer envelope. | Implemented | `BACKEND_SNAPSHOT_FORMAT_VERSION`; synthetic payload version |
| FR-SNAPSHOT-006 | Persist checkpoints to storage with retention policy. | Planned | No database/file repository yet |
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
| FR-EXEC-005 | Detect divergence at the first differing tick. | Partial | Hashes compared; tick-indexed trace diff planned |
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
| FR-MD-002 | Execute a documented 68000 subset deterministically. | Implemented | `crates/chronofork-megadrive`; 13 tests |
| FR-MD-003 | Map ROM and 64 KiB work RAM big-endian. | Implemented | Bus tests |
| FR-MD-004 | Reject unmapped, odd-address and ROM-write accesses. | Implemented | Bus/backend error tests |
| FR-MD-005 | Snapshot/restore complete slice state with config/state hashes. | Implemented | Snapshot round-trip and mismatch tests |
| FR-MD-006 | Fork/restore a real-core session without source mutation. | Implemented | Session integration test |
| FR-MD-007 | VDP status/control, VRAM and DMA. | Planned | Not implemented |
| FR-MD-008 | YM2612, PSG and audio. | Planned | Not implemented |
| FR-MD-009 | Z80, interrupts and exception processing. | Planned | Not implemented |
| FR-MD-010 | Controller input and multi-port schema. | Planned | Input is not consumed yet |
| FR-MD-011 | ROM header/region/TMSS and cartridge mapper. | Planned | Not implemented |
| FR-MD-012 | Differential traces against the Exodus oracle. | Blocked | Ninja/clang/libpng and a ROM/oracle build absent |

Tick semantics: one executed 68000 instruction equals one ChronoFork tick in this
slice. This is deterministic but **not** cycle-accurate hardware timing.

## Debugging, tracing, TAS and interfaces

| ID | Requirement | Status |
|---|---|---|
| FR-DEBUG-001 | Register inspection | Implemented (synthetic + minimal 68000) |
| FR-DEBUG-002 | Conditional breakpoints | Planned |
| FR-DEBUG-003 | Memory watchpoints | Planned |
| FR-DEBUG-004 | Event breakpoints, logpoints, interrupt monitoring | Planned |
| FR-TRACE-001 | Structured instruction/bus/device traces with emulated timestamps | Planned |
| FR-TRACE-002 | Trace comparison and historical queries | Planned |
| FR-TAS-001 | Movie record/playback, frame advance, rerecord, seek | Planned |
| FR-TAS-002 | Versioned canonical movie format with branch metadata | Planned |
| FR-API-001 | Minimal CLI for implemented features only | Implemented |
| FR-API-002 | HTTP API over the same application services | Planned |
| FR-API-003 | MCP over Streamable HTTP, not the source of truth | Planned |
| FR-API-004 | External SDKs | Planned |
| FR-SCRIPT-001 | Sandboxed scripting with explicit limits | Planned |
| FR-GUI-001 | Optional GUI that does not own canonical state | Planned |

## Licensing and provenance

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| FR-LICENSE-001 | Preserve license/attribution for every incorporated component. | Implemented | `LICENSE`, `THIRD_PARTY_NOTICES.md`, provenance |
| FR-LICENSE-002 | Do not commit ROMs, firmware or unlicensed fixtures. | Implemented | `.gitignore`, synthetic program only |
| FR-LICENSE-003 | Record exact upstream revisions for research. | Implemented | `docs/research/upstream-repositories.md` |
| FR-LICENSE-004 | Require approval before changing project license or incorporating restrictive code. | Implemented | Policy + ADR-0002 proposal |
