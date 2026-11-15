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

- [x] Approve ADR-0002 (Option E accepted).
- [x] Rust-native 68000 subset, ROM/RAM map, reset vectors, deterministic
  instruction-quantum tick.
- [x] ROM identity/config hash and an independently authored synthetic ROM.
- [x] Snapshot/restore/fork on the real core.
- [x] Minimal VDP status/control and VRAM window.
- [x] Deterministic interrupt/tick scheduler.
- [x] Bounded debug-only instruction trace and `megadrive trace` CLI.
- [x] Differential trace evidence against the Exodus oracle — current PASS 3/3:
  `rom_alu` 99/99 and `rom_mem_vdp` 51/51 at the explicit 1,000,000-instruction
  controlled window, plus `rom_masked` 155/155 at the CLI default quantum 64 with
  the flag omitted, with equal projection SHA-256 and final states, negative
  controls 3/3 and the masked default-quantum gating control. `rom_masked` crosses
  two HBlank boundaries at SR mask 7 with no handler entry, so it is negative
  gating evidence, not interrupt-delivery equivalence. The earlier post-repair
  two-ROM PASS 2/2, the pre-repair divergence at `0x00027A` and the E2-13
  absolute-long `MOVE` repair are preserved as history in
  [oracle differential evidence](../research/oracle-differential-evidence.md).
- [ ] Complete production hardware state, including VDP/audio/Z80.

**Exit:** a known synthetic ROM executes deterministically; snapshot/restore/fork
works on the real core; differential evidence exists. The hardware slice, VDP port
window, interrupt scheduler with SR-mask gating/`RTE`/VINT pending-acknowledge,
bounded debug trace and differential evidence for three synthetic ROMs are done for
the modelled slice, and E2-15 is done. Remaining hardware (rendering, Z80, audio,
DMA) is still pending, and the modelled slice keeps its documented limits:
privilege is not enforced; only HBlank level 4 / VBlank level 6 sources exist, with
level 7 a predicate rule only; no interrupt-timing equivalence is claimed; and VDP
interrupt-enable/acknowledge logic is absent.

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

First increment implemented and independently verified on 2026-11-15 for the
deterministic synthetic backend (M3-T4 PASS); see
[ADR-0004](../adr/0004-tas-movie-format.md) and the [M3 acceptance
plan](../quality/m3-tas-replay-acceptance.md).

- [x] Canonical movie format with versioning and metadata (v1 `.cfmovie`).
- [x] Record/playback of a per-tick input sequence on the deterministic synthetic
      backend.
- [x] Replay verification with exact first-divergence reporting.
- [ ] Frame advance, rewind and seek via checkpoints.
- [ ] Rerecording and branch editing.
- [ ] Multi-controller, ports, analog and keyboard input.
- [ ] Import/export adapters, documented only, not implemented.

**Exit (first increment met):** a recorded input sequence reproduces every recorded
tick and state hash exactly and reports the first divergence exactly, on the
deterministic synthetic backend. Full Stage 3 exit also requires branch comparison,
which remains deferred.

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
