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
- [ ] Real-backend decision approved (M0 can close with a proposal pending user
      approval; implementation is M1).

## M1 — Backend decision and minimal hardware slice

- [ ] ADR-0002 approved or replaced.
- [ ] Build path for the chosen backend reproducible on Linux.
- [ ] ROM identity and hardware configuration represented in `config_hash`.
- [ ] Minimal synthetic ROM executes deterministically on the real backend.
- [ ] Snapshot/restore/fork passes on the real backend.
- [ ] Differential trace evidence against the approved reference.
- [ ] Capabilities report matches reality.

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

**M0**, with the real-backend decision explicitly pending. See
[EPIC-0001](../epics/EPIC-0001-foundation.md) and
[ADR-0002](../adr/0002-backend-boundaries.md).
