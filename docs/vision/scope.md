# Scope

## Current bootstrap scope (implemented)

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

## Next scope (not yet implemented)

- Approval and implementation of a real Mega Drive backend strategy.
- CPU/VDP/audio device contracts and a bus model for the selected backend.
- Trace records with emulated timestamps and structured export.
- Breakpoints, watchpoints and conditional events.
- TAS movie format, rerecording checkpoints and deterministic seek.
- HTTP API and MCP adapters over the same application services.
- Scripting runtime evaluation and sandboxing.
- GUI/video/audio presentation independent of canonical state.

## Explicitly outside the bootstrap

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
