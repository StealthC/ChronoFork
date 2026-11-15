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

First, second, third and fourth increments implemented and independently verified on
2026-11-15 for the deterministic synthetic backend (M3-T4, M3-T5, the t7 format-v2
verification PASS and the t5 rewind verification PASS); see
[ADR-0004](../adr/0004-tas-movie-format.md),
[ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md),
[ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md),
[ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md) and the [M3
acceptance plan](../quality/m3-tas-replay-acceptance.md), the [increment-2 acceptance
plan](../quality/m3-partial-replay-acceptance.md), the [format-v2 acceptance
plan](../quality/m3-format-v2-acceptance.md) and the [M3 rewind acceptance
plan](../quality/m3-rewind-acceptance.md). The v2 increment adds the `CFMOVIE2`
checkpoint-anchor container, nearest-anchor seek and schema-2 multi-device input. The
fourth increment adds a bounded in-process rewind state cache, deterministic rewind by
restore plus verified forward replay and the interactive `chronofork tas studio` stepping
surface. M3 is not complete: rerecord/branch editing, real-hardware input TAS,
import/export adapters, checkpoint-reference movies and HTTP/MCP/GUI/daemon remain
deferred.

- [x] Canonical movie format with versioning and metadata (v1 `.cfmovie` and v2
      `CFMOVIE2`).
- [x] Record/playback of a per-tick input sequence on the deterministic synthetic
      backend.
- [x] Replay verification with exact first-divergence reporting.
- [x] Ranged replay/seek, forward single-tick advance and rewind by deterministic
      linear re-simulation from the embedded checkpoint (typed range bounds; no
      carried state).
- [x] Branch comparison of recorded `(tick, state hash)` records with first-divergence,
      length and embedded-checkpoint identity reporting (no re-simulation).
- [x] Periodic anchors/checkpoints and seek via checkpoints (movie format v2) —
      implemented and independently verified (M3-A18–A21/A24; ADR-0006, t7 PASS).
- [x] In-process rewind buffer/state cache and interactive stepping — implemented as the
      session-layer `Player`/`StateCache` and `chronofork tas studio`, and independently
      verified for the deterministic synthetic backend (M3-A28–A34; ADR-0007, t5 PASS,
      2026-11-15). The cache is process-local; no persistent/shared cache or O(1) seek is
      claimed.
- [ ] Rerecording and branch editing.
- [x] Multi-controller, ports, analog and keyboard input — schema-2 multi-device frames
      (four digital ports, four analog axes and a keyboard bitmap) are implemented and
      independently verified for the deterministic synthetic backend (M3-A22/A24; ADR-0006,
      t7 PASS). Real-hardware controller input remains absent; the Mega Drive `step`
      still ignores input.
- [ ] Real-hardware input TAS (the Mega Drive `step` ignores input).
- [ ] Import/export adapters, documented only, not implemented.

**Exit (first four increments met):** a recorded input sequence reproduces every
recorded tick and state hash exactly, reports the first divergence exactly, can be
replayed/seeked over an exact tick window or from a checkpoint anchor and compared
against another recording, and can be rewound and stepped interactively through a bounded
in-process state cache, on the deterministic synthetic backend. Full Stage 3 exit also
requires rerecord and branch editing, and real-hardware input TAS, which remain deferred.
The schema-2 multi-device model is synthetic-backend only and does not satisfy
real-hardware controller input.

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
