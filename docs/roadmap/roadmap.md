# Roadmap

This is the long-term product roadmap. It is not the bootstrap scope. Stages are
sequential only where a dependency requires it; research and documentation can run
in parallel.

## Stage 0 — Bootstrap foundation (current)

**Goal:** prove the architecture and establish durable project infrastructure.

- Environment validation, persistent planning, reference acquisition.
- Upstream research, licensing assessment, provenance.
- Official docs, AGENTS.md, EPIC-0001.
- Rust workspace, backend contracts, synthetic deterministic backend.
- Session lifecycle, snapshots, restore, three-way fork, replay and isolation tests.
- Quality gates and local verification.

**Exit:** workspace compiles, tests pass, docs are navigable, references are ignored,
and the real-backend decision is explicitly proposed.

## Stage 1 — Real backend decision and minimal hardware core

**Goal:** select a real Mega Drive backend strategy and implement the smallest
credible hardware slice.

- Approve/adjust ADR-0002.
- If Rust-native: 68000 execution subset, memory map, reset vectors, minimal VDP
  status/control, deterministic tick scheduler.
- If an adapter is approved: reproducible Linux/Windows build, multi-instance and
  state-export proof.
- ROM identity/config hashing; synthetic test ROMs generated from source.
- Differential trace harness against the approved reference.
- Complete snapshot/restore for the real core, including hidden device state.

**Exit:** a known synthetic ROM executes deterministically; snapshot/restore/fork
works on the real core; differential evidence exists.

## Stage 2 — Sessions, forks and save states at product quality

- Durable checkpoint storage with versioned format and retention policy.
- Named snapshots, session metadata and ownership.
- Snapshot compatibility across backend versions.
- Copy-on-write investigation (only after correctness).
- Multi-process or worker isolation if required.
- Concurrency policy for shared sessions.

**Exit:** users can create, fork, restore and compare sessions across process
restarts without state loss.

## Stage 3 — TAS and deterministic replay

- Canonical movie format with versioning and metadata.
- Record/playback, frame advance, rewind, seek via checkpoints.
- Rerecording and branch editing.
- Multi-controller input.
- Replay verification and divergence reporting.
- Import/export adapters, initially documented rather than implemented.

**Exit:** a recorded input sequence reproduces a state/trace exactly, and branches
can be compared.

## Stage 4 — Deep Mega Drive debugging

- CPU registers, flags, disassembly and instruction stepping.
- Memory inspector/editor and watchpoints.
- Breakpoints, logpoints, event and interrupt breakpoints.
- Bus transaction and device event observation.
- VDP, DMA, YM2612, PSG and mapper introspection.
- Code/data analysis and symbol mapping foundations.

**Exit:** a reverse-engineering workflow can find what wrote a value and step
through the responsible code without a GUI.

## Stage 5 — Tracing and scripting

- Structured instruction/bus/device traces with emulated timestamps.
- Triggered tracing, circular buffers, structured exports.
- Trace comparison and historical queries.
- Script runtime evaluation (Rhai/Luau/WASM/Python SDK) with sandboxing, memory and
  time limits.
- Script APIs for sessions, forks, inspection, TAS and experiments.

**Exit:** scripts can run controlled experiments and produce reproducible reports
without host-language plugins.

## Stage 6 — Agent-native interfaces

- HTTP API and MCP over Streamable HTTP using the same services.
- Typed tool surface, not hundreds of loose endpoints.
- Session ownership, permissions and concurrency controls.
- Experiment artifacts consumable by external tools.
- CLI as a first-class client, not a separate implementation.

**Exit:** humans, scripts and agents can drive identical operations through CLI,
HTTP and MCP with identical semantics.

## Stage 7 — Human-facing GUI

- Optional windows for one or many sessions.
- Live video/audio presentation decoupled from canonical state.
- Debugger panels, memory viewers, hardware inspectors, TAS timelines.
- Remote observation and human/agent control handoff.
- Multi-window/mosaic layouts.

**Exit:** GUI can attach to and detach from sessions without changing emulation
results.

## Stage 8 — Multi-backend expansion

- Additional consoles/backends behind the same capability model.
- Backend capability differences surfaced honestly.
- Shared tracing/TAS/debugging where capabilities allow.
- Differential testing across backends.
- Stable external SDKs.

**Exit:** a second system is integrated without changing session/storage/transport
core semantics.

## Cross-cutting tracks

- Licensing/provenance review at every incorporation point.
- Cross-platform CI (Linux, Windows, macOS where feasible).
- Fuzzing, property tests and regression corpus.
- Performance measurement after correctness.
- Documentation and ADR maintenance.
