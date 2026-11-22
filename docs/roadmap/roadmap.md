# Roadmap

This is the long-term product roadmap. It is not the bootstrap scope. Stages describe the
capabilities the platform must gain, ordered only where a dependency requires it; research
and documentation can run in parallel.

The roadmap has two structures, kept deliberately separate:

- **Capability stages 0–9.** Each stage has exactly one checkpoint milestone, M0–M9,
  defined in [Milestones](milestones.md). A stage tracks a product capability, not how
  complete a console model is.
- **Hardware-depth track H1–H5.** A parallel, cross-cutting track that records how complete
  the emulated machine is. It is not a stage and not a stage milestone. Hardware completeness
  does not appear as an open item inside the capability stages, except in **M8 (Functional
  Mega Drive)**, the product milestone that deliberately sequences H1–H5 to completion.

Status, integration state, evidence and follow-ups are recorded once, in
[Milestones](milestones.md#status-at-a-glance). The sections below record intent: goal,
scope, exit criteria, dependencies and non-goals.

## How to read this document

- **Stage ↔ milestone mapping.** One stage, one milestone: Stage 0 ↔ M0 through Stage 9 ↔ M9,
  plus the parallel H1–H5 track. The [stage index](#stage-index) below links both sides;
  [Milestones](milestones.md#status-at-a-glance) holds the reverse mapping and the status.
- **Status vocabulary.** `Complete`, `Complete with integration pending`, `Next` and
  `Planned`, plus `Partial` for hardware-depth rows only. The definitions live in
  [Milestones — status vocabulary](milestones.md#status-vocabulary); this page names the
  vocabulary but does not restate a status.
- **What belongs where.** A stage records capability intent and exit criteria;
  [Milestones](milestones.md) records status, work items, evidence and follow-ups; the
  [architecture docs](../architecture/overview.md) describe what exists; the
  [requirements](../requirements/functional.md) carry FR/NFR statuses; the
  [quality gates](../quality/quality-gates.md) carry executed gate results.
- **Checkboxes.** This page uses plain bullets, never task boxes. Open work lives in
  Milestones as an unchecked work item, in a follow-up, or in H1–H5.

## Stage index

| Stage | Milestone | Focus |
|---|---|---|
| [Stage 0 — Bootstrap foundation](#stage-0--bootstrap-foundation) | [M0](milestones.md#m0--bootstrap-foundation) | Architecture, workspace, synthetic validator, docs, gates |
| [Stage 1 — Real backend decision and minimal hardware core](#stage-1--real-backend-decision-and-minimal-hardware-core) | [M1](milestones.md#m1--backend-decision-and-minimal-hardware-slice) | ADR-0002, documented 68000 set, ROM/RAM, VDP port window, interrupt scheduler, oracle differential |
| [Stage 2 — Sessions, forks and save states at product quality](#stage-2--sessions-forks-and-save-states-at-product-quality) | [M2](milestones.md#m2--durable-sessions) | Durable checkpoints, metadata, compatibility, writer ownership |
| [Stage 3 — TAS and deterministic replay](#stage-3--tas-and-deterministic-replay) | [M3](milestones.md#m3--tas-and-replay) | Movies, replay/seek, rewind, rerecord/edit, hardware input TAS |
| [Stage 4 — Debugging depth](#stage-4--debugging-depth) | [M4](milestones.md#m4--debugging-depth) | Debug observation, breakpoints/watchpoints, views, inspectors, debug CLI |
| [Stage 5 — Tracing and scripting](#stage-5--tracing-and-scripting) | [M5](milestones.md#m5--tracing-and-scripting) | Structured traces, sandboxed scripting |
| [Stage 6 — Agent-native interfaces](#stage-6--agent-native-interfaces) | [M6](milestones.md#m6--agent-interfaces) | HTTP, MCP, typed tools, permissions |
| [Stage 7 — Human-facing GUI](#stage-7--human-facing-gui) | [M7](milestones.md#m7--gui) | Presentation, panels, attach/detach |
| [Stage 8 — Functional Mega Drive](#stage-8--functional-mega-drive) | [M8](milestones.md#m8--functional-mega-drive) | Complete 68000/Z80/VDP/audio, cartridge, playable ROMs, headless capture, native window |
| [Stage 9 — Multi-backend expansion](#stage-9--multi-backend-expansion) | [M9](milestones.md#m9--multi-backend) | Second backend, capability differences, SDKs |
| [Hardware depth H1–H5](#hardware-depth-parallel-track-h1h5) | `H1`–`H5` (track, not a stage) | CPU, video/VDP/DMA, audio/Z80, cartridge/controllers/timers, accuracy/evidence |

## Stage 0 — Bootstrap foundation

- **Goal:** prove the architecture and establish durable project infrastructure.
- **Milestone:** [M0 — Bootstrap foundation](milestones.md#m0--bootstrap-foundation).
- **Depends on:** nothing.

### Scope and deliverables

- Environment validation, persistent planning and reference acquisition under ignored paths.
- Upstream research, licensing assessment and provenance.
- Official documentation, `AGENTS.md` and [EPIC-0001](../epics/EPIC-0001-foundation.md).
- Rust workspace, backend contracts and the deterministic synthetic backend.
- Session lifecycle, snapshots, restore, three-way fork, replay and isolation tests.
- Quality gates and local verification.

### Exit criteria

- The workspace compiles and the required local gate passes, as recorded in
  [quality gates](../quality/quality-gates.md).
- Documentation is navigable from the [documentation index](../README.md), and reference
  material stays under ignored paths.
- The real-backend decision is explicitly proposed and approved:
  [ADR-0002](../adr/0002-backend-boundaries.md), accepted 2026-11-13.

### Non-goals and tracked elsewhere

- No real console backend, transport, GUI or TAS in this stage; those begin at Stages 1, 3,
  6 and 7.

## Stage 1 — Real backend decision and minimal hardware core

- **Goal:** select a real Mega Drive backend strategy and implement the smallest credible
  hardware core behind the existing contracts.
- **Milestone:** [M1 — Backend decision and minimal hardware slice](milestones.md#m1--backend-decision-and-minimal-hardware-slice).
- **Depends on:** M0 (contracts and synthetic validator) and
  [ADR-0002](../adr/0002-backend-boundaries.md).

### Scope and deliverables

- Accepted [ADR-0002](../adr/0002-backend-boundaries.md) (Option E, Rust-native staged
  hybrid).
- The documented MC68000 instruction set with ROM/RAM map, reset vectors and a
  deterministic master-cycle tick (the M1 subset and instruction quantum are
  superseded).
- ROM identity (`config_hash`) and an independently authored synthetic ROM.
- Snapshot/restore/fork on the real core, with slice state in snapshots and state hashes.
- Minimal VDP status/control port window with 64 KiB VRAM.
- Deterministic interrupt/tick scheduler: HBlank level 4 and VBlank level 6 sources, SR-mask
  gating (level 7 always admitted, other levels only when strictly above the mask), `RTE`
  handler return and CPU-side VDP VINT pending/acknowledge.
- Bounded debug-only instruction trace and the `megadrive trace` CLI.
- Pinned-oracle differential evidence for thirteen synthetic ROMs on the master-cycle
  timeline, with address, per-instruction cycle, final-state and negative-control
  comparisons; one ROM (`rom_exception`) carries two named scoped oracle-side
  divergences, and the historical masked-ROM default-quantum control is preserved.

### Exit criteria

- A known synthetic ROM executes deterministically and snapshot/restore/fork works on the
  real core.
- The differential harness matches the pinned Exodus oracle on projected address streams and
  selected final state, with negative controls caught; the current run, its controlled
  windows and the pre-repair history are recorded in
  [oracle differential evidence](../research/oracle-differential-evidence.md).
- The masked-ROM differential is **negative gating evidence**, not interrupt-delivery or
  interrupt-timing equivalence.
- The capability report advertises only behavior that exists
  ([functional requirements](../requirements/functional.md)).

### Non-goals and tracked elsewhere

- **Production hardware completeness is not a Stage 1 exit criterion.** Rendering, DMA,
  CRAM/VSRAM, audio, Z80, cartridge identity/mapper, timers, the full controller protocol
  and cycle accuracy belong to the parallel
  [H1–H5](#hardware-depth-parallel-track-h1h5) track; Stage 1 stays complete for the
  modelled slice while that work continues.
- The slice keeps its documented limits: privilege is enforced for the implemented system
  instructions; only HBlank level 4 and VBlank level 6 sources exist, with level 7 a
  predicate rule only; guest bus-error/trace delivery and cycle accuracy remain absent; no
  interrupt-timing equivalence is claimed; VDP interrupt-enable registers and VDP-side
  acknowledge logic are not modelled (the CPU-side pending/acknowledge path exists);
  controller input is the TH-high 3-button subset on ports 1–2. See [Mega Drive
  slice](../architecture/megadrive-slice.md#not-implemented).
- Coverage beyond the documented MC68000 set is [H1](#h1--cpu-and-instruction-completeness);
  the remaining controller protocol, cartridge identity and timers are
  [H4](#h4--cartridge-controllers-and-timers).

## Stage 2 — Sessions, forks and save states at product quality

- **Goal:** make sessions, forks and save states durable and trustworthy across process
  restarts.
- **Milestone:** [M2 — Durable sessions](milestones.md#m2--durable-sessions).
- **Depends on:** M1 (real core to persist) and the session model.

### Scope and deliverables

- Durable checkpoint storage with a versioned format and explicit retention.
- Named snapshots, session metadata and writer ownership.
- Reject-only snapshot compatibility across backend versions.
- Storage-backed rehydration and the `chronofork session save/load/list/delete` CLI.
- Concurrency policy at the service boundary: one writer per durable session, typed
  contention, owner-checked stale-lock recovery, lock-free readers.
- Copy-on-write investigation only after correctness.

### Exit criteria

- Users can create, fork, restore and compare sessions across process restarts without state
  loss.
- Incompatible or corrupt input is rejected with typed errors; nothing is migrated or
  repaired silently.
- Writing, reading or deleting one session never changes another session's checkpoint bytes,
  state hash or lineage.
- The executed criteria are recorded in the [M2 acceptance
  plan](../quality/m2-durable-sessions-acceptance.md) and [quality
  gates](../quality/quality-gates.md).

### Non-goals and tracked elsewhere

- Copy-on-write/shared-page forks stay deferred (FR-FORK-007) until correctness work is done.
- Encryption, authentication and format migration are out of scope; retention stays manual
  and there is no automatic garbage collection.
- Windows and network-filesystem durability, and a concurrent publish/read interleaving, are
  residual limits tracked as M2-FU1 in [Milestones](milestones.md#m2--durable-sessions).

## Stage 3 — TAS and deterministic replay

- **Goal:** record, replay, verify, rewind and edit deterministic input movies (TAS) on the
  synthetic validator and on the minimal Mega Drive slice.
- **Milestone:** [M3 — TAS and replay](milestones.md#m3--tas-and-replay).
- **Depends on:** M2 (durable checkpoints and storage) and the TH-high 3-button controller
  subset that H4 delivered into the slice.

### Scope and deliverables

- Canonical single-file movie formats: v1 `.cfmovie` and v2 `CFMOVIE2`
  ([ADR-0004](../adr/0004-tas-movie-format.md),
  [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md)).
- Record/playback of per-tick input sequences, replay verification with exact
  first-divergence reporting, restart and M2-seeded replay.
- Ranged replay/seek, forward single-tick advance, and rewind by deterministic
  re-simulation ([ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md)).
- Checkpoint anchors and nearest-anchor seek in the v2 container.
- Schema-2 multi-device input (four digital ports, four analog axes, keyboard bitmap).
- In-process rewind state cache and interactive `chronofork tas studio` stepping
  ([ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md)).
- Rerecord and branch editing through `chronofork tas edit`
  ([ADR-0008](../adr/0008-rerecord-and-branch-editing.md)).
- Real-hardware input TAS: `--rom` backend selection and stateless per-step controller reads
  for the TH-high 3-button subset on ports 1–2
  ([ADR-0009](../adr/0009-real-hardware-input-tas.md)).

### Exit criteria

- Six increments are implemented and independently verified; every recorded tick and state
  hash is reproduced exactly and the first divergence is reported exactly.
- Ranged replay/seek, anchored seek, rewind/stepping and rerecord/edit behave as specified
  on the deterministic synthetic backend, and record/play/verify/edit/step work on the
  minimal Mega Drive slice with per-step controller input.
- The M3 completion audit and the t7 promotion audit passed on 2026-11-16
  ([Milestones — M3](milestones.md#m3--tas-and-replay) records the evidence).
- No unimplemented capability is claimed: the hardware claim is the synthetic
  `controller_rom()` and the 3-button subset, not commercial ROMs, controller timing or
  cycle accuracy.

### Non-goals and tracked elsewhere

- Movie import/export adapters (for example BK2) are deferred to **M3-FU1**; they are not an
  open Stage 3 item.
- Checkpoint-reference movies are deferred to **M3-FU2**.
- In-place/timeline editing, a persisted rerecord counter, format v3, mixed-anchor mode and
  O(1)/persistent-cache seek are deferred to **M3-FU3**.
- The rest of the controller protocol (TH latch, A/Start, 6-button, port 3, analog/keyboard
  hardware) is [H4](#h4--cartridge-controllers-and-timers); controller timing and cycle
  accuracy are [H4](#h4--cartridge-controllers-and-timers)/[H5](#h5--accuracy-and-evidence).
- HTTP/MCP/GUI/daemon surfaces are Stages 6–7, not TAS work.

## Stage 4 — Debugging depth

- **Goal:** let reverse-engineering and automation workflows inspect and stop a session
  without a GUI.
- **Milestone:** [M4 — Debugging depth](milestones.md#m4--debugging-depth).
- **Depends on:** M1 (backend inspection contract and slice) and
  [ADR-0010](../adr/0010-debugging-depth.md).

### Scope and deliverables

- Accepted [ADR-0010](../adr/0010-debugging-depth.md): optional, bounded, non-canonical
  debug observation (`DebugAccess`, `DebugConfig`, typed events, `Capabilities::debug_events`).
- Host debugger layer: PC/range/conditional breakpoints, byte-granular read/write
  watchpoints, event breakpoints and logpoints.
- Register, memory and bounded 68000-subset disassembly views.
- Mega Drive CPU/VDP/interrupt-scheduler/bus inspectors.
- `chronofork debug run|inspect|memory|disasm|vdp|interrupts|bus` CLI surface.
- Non-perturbation evidence: observation off/on leaves canonical state, snapshots, demos and
  movie bytes unchanged.

### Exit criteria

- Every M4-A1–A17 row in the [M4 acceptance
  plan](../quality/m4-debugging-depth-acceptance.md) is `executed - passed`, and the full
  gate passes on the M4 tree ([quality gates](../quality/quality-gates.md)).
- The non-perturbation probe finds zero canonical differences between observation off and
  on, and the demo/movie bytes and pinned-oracle differential are unchanged.
- The promotion audit and the final content review pass ([Milestones —
  M4](milestones.md#m4--debugging-depth) records status and integration state).

### Non-goals and tracked elsewhere

- Mid-instruction or cycle-exact stops, reverse execution, call-stack/step-over/step-out,
  code/data analysis, symbol mapping, conditional watchpoint values and persistent debugger
  configuration are **M4-FU1**.
- DMA, rendering, CRAM/VSRAM, audio/Z80 and mapper inspector views are **M4-FU2**, and they
  cannot be verified before the [H2–H4](#h2--video-vdp-and-dma) device models exist.
- UI, HTTP and MCP surfaces are Stages 6–7; cross-platform runtime evidence is
  [H5](#h5--accuracy-and-evidence).

## Stage 5 — Tracing and scripting

- **Goal:** let scripts run controlled experiments and produce reproducible reports without
  host-language plugins.
- **Milestone:** [M5 — Tracing and scripting](milestones.md#m5--tracing-and-scripting).
- **Depends on:** M4 (bounded observation surface), M2 (sessions and storage) and M3
  (movies). It does **not** depend on hardware-depth work.
- **Delivered:** host-provided structured tracing (`chronofork-trace`, the session capture
  entry point and `chronofork trace`) and a purpose-built deterministic script DSL with
  explicit limits (`chronofork-script` and `chronofork script`). The implementation was
  independently verified on `agent/m5-tracing-scripting` (T12 PASS, 2026-11-16); the
  milestone page is the single record of status/integration and the [acceptance
  plan](../quality/m5-tracing-and-scripting-acceptance.md) holds the executed rows and
  evidence.

### Scope and deliverables

- Structured instruction/bus/device traces with emulated timestamps and structured exports.
- Triggered tracing, circular buffers and bounded trace retention.
- Trace comparison and historical queries.
- A sandboxed, deterministic script runtime (a purpose-built DSL, not an embedded
  host-language engine) with memory and time limits. The earlier Rhai/Luau/WASM/Python list
  named examples, not a mandate; [ADR-0011](../adr/0011-tracing-and-scripting.md) §3.5
  records why embedding those engines is rejected.
- Script APIs for sessions, forks, inspection, TAS and experiments.
- CLI surfaces for every implemented capability, verified by process-level tests.

### Exit criteria

- Scripts can run controlled experiments and produce reproducible reports without
  host-language plugins.
- Trace export round-trips, and trace comparison reports the first divergence.
- Sandbox limits are enforced, typed and tested.
- FR-TRACE-001, FR-TRACE-002 and FR-SCRIPT-001 reflect the delivered scope
  ([functional requirements](../requirements/functional.md)).

### Non-goals and tracked elsewhere

- Frame pacing and a host-time run loop are [M7](milestones.md#m7--gui) work.
- Trace sources for DMA, audio and Z80 wait for [H2](#h2--video-vdp-and-dma) and
  [H3](#h3--audio-and-z80).
- HTTP/MCP transport is Stage 6; GUI trace viewers are Stage 7.
- TSV import, trace migration/compression and embedded host-language engines are not
  delivered; [ADR-0011](../adr/0011-tracing-and-scripting.md) records the rejected
  alternatives and the one-way TSV and canonical-v1 boundaries.
- No performance claim: performance measurement after correctness is
  [H5](#h5--accuracy-and-evidence).

## Stage 6 — Agent-native interfaces

- **Goal:** expose the same application services to humans, scripts and agents through CLI,
  HTTP and MCP with identical semantics.
- **Milestone:** [M6 — Agent interfaces](milestones.md#m6--agent-interfaces).
- **Depends on:** the M2–M5 services (sessions, movies, debug, tracing/scripting).
- **Delivered:** a transport-agnostic service layer (`chronofork-service`: 16-operation typed
  catalog, canonical JSON, token/scope/ownership policy and the documented concurrency
  boundary), bounded HTTP/1.1 and stateless MCP adapters plus a typed client
  (`chronofork-transport`), the daemon and client shapes (`chronofork serve`, `api`, `mcp`,
  `experiment run`), canonical machine-consumable `chronofork.experiment.v1` artifacts and
  byte-level direct/HTTP/MCP parity. No third-party HTTP, async-runtime or serialization
  dependency was added. The implementation was independently verified on
  `agent/m6-agent-interfaces` at base `93d47ed` (pass-2 PASS, 2026-11-17, after the
  M6-R1–R10 repair and a round-2 review pass) and merged to `main` at `cb05836` (PR #6);
  the milestone page is the single record of status/integration and the [acceptance
  plan](../quality/m6-agent-interfaces-acceptance.md) holds the executed rows and evidence.

### Scope and deliverables

- HTTP API and MCP over Streamable HTTP using the same services as the CLI.
- A typed tool surface instead of hundreds of loose endpoints.
- Session ownership, permissions and concurrency controls at the transport boundary.
- Machine-consumable experiment artifacts.
- CLI as a first-class client of those services, not a separate implementation.
- A daemon/deployment shape for long-running use.

### Exit criteria

- Humans, scripts and agents can drive identical operations through CLI, HTTP and MCP with
  identical semantics, proven by parity tests.
- Transport types never appear in core, backend or session APIs.
- No second source of truth: the session layer stays canonical.

### Non-goals and tracked elsewhere

- The GUI is Stage 7; it consumes these services but never owns canonical state.
- Additional systems are Stage 9, and external SDKs (FR-API-004) remain Stage 9 scope.
- Not delivered in Stage 6 and tracked as M6-FU1–M6-FU4 on the milestone page: debug, trace,
  script and TAS operations over transport; durable per-principal ownership and
  restart-persistent live sessions; TLS, token rotation/expiry, audit log and multi-node
  deployment; SSE/streaming, MCP resources/prompts/logging/completion and `Mcp-Session-Id`.
  [ADR-0012](../adr/0012-agent-native-interfaces.md) records the rejected alternatives
  (REST-per-operation, `axum`/`hyper`/`tokio` + `serde`, SSE, MCP stdio, persisting ownership
  in the storage format).

## Stage 7 — Human-facing GUI

- **Goal:** provide optional human-facing presentation surfaces for viewing and controlling
  sessions without owning canonical state.
- **Milestone:** [M7 — GUI](milestones.md#m7--gui) (`Complete`; merged to `main` at
  `cb05836`, PR #6).
- **Depends on:** M6 (attach/detach transport). Real video and audio to present depend on
  [H2](#h2--video-vdp-and-dma)/[H3](#h3--audio-and-z80), which are still partial/planned;
  the delivered pipeline is capability-gated and reports both as unavailable until a real
  source/sink exists.
- **Delivered:** a presentation-only `chronofork-gui` crate (strict view decoders, window/
  pane/mosaic model, eight-panel availability matrix, deterministic fixed-size plain/ANSI
  and self-contained HTML renderers, `HostClock`/`ManualClock` pacing with a host-time run
  loop and pause gate, capability-gated video/audio pipelines and the cooperative
  human/agent controller lease), the `chronofork gui run` stdin-driven process surface in
  the CLI (local and remote via the M6 channel, read-only `.cfmovie` timeline, `pulse`,
  blocks and exit codes), and attach/detach proof that canonical results do not change.
  The implementation was independently verified on `agent/m7-gui` at base `93d47ed`
  (final PASS4 and REVIEW5 PASS, 2026-11-17, after the F1–F4/FAIL3 repairs) and merged to
  `main` at `cb05836` (PR #6); the milestone page is the single record of
  status/integration and the [acceptance plan](../quality/m7-gui-acceptance.md) holds the
  executed rows and evidence.

### Scope and deliverables

- Presentation surfaces for one or many sessions (fixed-size text grids, ANSI emphasis and
  a static HTML document).
- Capability-gated video/audio presentation decoupled from canonical state; honest
  `Unavailable` paths in v1.
- Frame pacing and a host-time run loop with an injected clock; deterministic `pulse`.
- Debugger panels, memory viewers, hardware inspector reasons and read-only TAS timelines.
- Remote observation and human/agent control handoff.
- Single-session and mosaic layouts over up to four attached panes per window.

### Exit criteria

- The GUI can attach to and detach from sessions without changing emulation results
  (M7-A6, A7, A21, A22).
- Presentation consumes state but never owns or mutates canonical emulation state
  (M7-A1–A4, A28).
- A human and an agent can hand control back and forth with observable, deterministic
  results (M7-A16, A17, A22).

### Non-goals and tracked elsewhere

- The GUI adds no emulation semantics; rendering and audio depth are
  [H2](#h2--video-vdp-and-dma)/[H3](#h3--audio-and-z80). There is no native toolkit window,
  no browser/JS frontend, no streaming transport, no live audio output, no GUI input
  injection, no GUI state persistence and no VDP/bus/disassembly/trace panel content until
  M6-FU1 exposes it over transport. No performance or cross-platform claim is made.
- The capability-gated `Available` video/audio path is proven with test doubles only; the
  v1 process always reports `unavailable`.
- Scripting and the HTTP/MCP surface stay in Stages 5–6.

## Stage 8 — Functional Mega Drive

- **Goal:** turn the minimal Mega Drive slice into a functional emulator — the whole 68000
  and Z80, a complete VDP with rendering/DMA/timing, YM2612/PSG audio, cartridge/TMSS/mapper/
  SRAM, timers and controllers — playable with a curated set of commercial ROMs, capturable
  headlessly and presentable in an optional native window.
- **Milestone:** [M8 — Functional Mega Drive](milestones.md#m8--functional-mega-drive).
- **Depends on:** the M1–M7 layers and contracts (backend, sessions, TAS, debug, trace,
  service, presentation seam) and the accepted [ADR-0002](../adr/0002-backend-boundaries.md).
  It consumes the [H1–H5](#hardware-depth-parallel-track-h1h5) checkpoints: M8 is the product
  milestone that sequences them to completion.

### Scope and deliverables

- Cartridge header/region/TMSS/checksum, images above 4 MiB, a banking mapper and SRAM.
- Documented MC68000 instruction set, addressing modes and privilege enforcement are
  delivered by phase 8.2, with canonical time in master cycles; remaining exception
  groups and cycle/prefetch accuracy stay in H1/H5.
- Complete VDP: CRAM/VSRAM, planes/window/sprites/scrolling/priority/shadow-highlight,
  256/320 modes, DMA, FIFO/access timing, H/V counters and interrupt enable/acknowledge.
- Z80 core, 68000/Z80/VDP bus arbitration, YM2612 six-channel FM and PSG.
- Timer interrupts, full TH latch and 3-/6-button controllers on ports 1–2.
- Execution modes: unpaced headless batch, paced 60/50 fps with an injected host clock, and
  manual stepping; run-to-target APIs for agents. Emulated cycle time is canonical; pacing is
  a host policy.
- Deterministic headless capture: PNG frames and WAV audio, hashed and mode-independent.
- Optional native windowed frontend (video, audio, input, pause/step/save-state) behind a
  new ADR that supersedes ADR-0013's no-toolkit constraint.

### Product-consolidation track (inside the open M8)

Owner decision 2026-11-20 (O-2): a **consolidation track** runs inside the open M8 before
8.7/8.8. It covers the review/correction of
[ADR-0014](../adr/0014-v1-scope-single-user-loopback.md)/[0015](../adr/0015-verification-contract.md)/[0016](../adr/0016-session-service-and-agent-api.md)
and their implementation phases W1–W5 (boot/bus hardening, the execution/verification
contract, the service/concurrency work and the reproducible-evidence work). It is a track,
**not** a milestone: M8 stays `In progress`, the Definition of Done is **not** amended, and
completing the track does not claim M8 completion. It also records the other owner decisions
of 2026-11-20: non-loopback bind refuses to start (O-1); the project's own YM2612/PSG/mixer is
completed instead of adopting `ymfm` (O-3); unmapped I/O uses deterministic open bus plus a
typed error where the hardware faults, with `0xA10008–0xA1000F` mapped first (O-4). The
canonical status of the track lives in [Milestones](milestones.md#current-milestone) and the
[planning lifecycle](../planning/ROADMAP.md).

### Exit criteria

- Every criterion in [M8 — Definition of Done](milestones.md#m8--functional-mega-drive) is
  demonstrated by executed evidence, including the curated commercial-ROM compatibility list.
- The same inputs produce identical frame, audio and state hashes in unpaced, paced and
  stepped modes.
- The full quality gate passes and the M1–M7 contracts superseded by the new timing are
  re-evidenced or explicitly marked superseded.
- No commercial ROM, BIOS or game asset is committed.

### Non-goals and tracked elsewhere

- Universal commercial compatibility, cycle-perfect hardware equivalence and performance
  beyond real time are not claimed; compatibility is a documented curated list.
- Dynamic recompilation, run-ahead, skip-frames and savestate-rewind shortcuts are not
  required; each would need its own ADR and determinism analysis.
- Port 3, analog and keyboard hardware are H4 residual limits unless the curated set requires
  them (port 3/EXT control is a stub, port 3/EXT data is a typed error — owner decision O-4).
- Additional systems remain [Stage 9](#stage-9--multi-backend-expansion), after M8.
- TLS/multi-user deployment, async/Tokio, incremental hashing and COW/dedup remain deferred
  (owner decision 2026-11-20, O-2); none is required to close Stage 8.

## Stage 9 — Multi-backend expansion

- **Goal:** add further systems behind the same capability model without changing session,
  storage or transport semantics.
- **Milestone:** [M9 — Multi-backend](milestones.md#m9--multi-backend).
- **Depends on:** the M2–M7 shared layers and the backend boundaries in
  [ADR-0002](../adr/0002-backend-boundaries.md).

### Scope and deliverables

- Additional consoles/backends behind the backend and capability contracts.
- Backend capability differences surfaced honestly, not normalized away.
- Shared tracing/TAS/debugging where capabilities allow.
- Differential testing across backends.
- Stable external SDKs.

### Exit criteria

- A second system is integrated without changing session/storage/transport core semantics.
- The capability matrix distinguishes what each backend really supports.
- Cross-backend differential tests run and are recorded.

### Non-goals and tracked elsewhere

- Hardware completeness for each system is tracked in that system's own hardware-depth track;
  [H1–H5](#hardware-depth-parallel-track-h1h5) cover the Mega Drive model only.
- Feature parity between backends is not required; honest capability differences are.
- Performance and determinism evidence follow the [H5](#h5--accuracy-and-evidence) rules.

## Hardware depth (parallel track H1–H5)

Hardware depth is orthogonal to the capability stages: it tracks how complete the emulated
machine is and proceeds alongside them. H1–H5 are checkpoints, not stage milestones: they are
**not M10**, they never reopen a closed milestone, and their status is recorded in
[Milestones](milestones.md#status-at-a-glance). M8 is the one product milestone that
deliberately sequences H1–H5 to completion; the tracks remain the capability record and the
decomposition of M8's phases.

A stage, epic or requirement that hits a hardware limit references the H item instead of
carrying an unchecked hardware box.

### H1 — CPU and instruction completeness

- **Goal:** a documented, testable 68000 model; the documented set and privilege are
  delivered, with remaining exception groups and coverage beyond that set.
- **Milestone:** [H1](milestones.md#h1--cpu-and-instruction-completeness).
- **Depends on:** the Stage 1 slice and its differential harness.

#### Scope and deliverables

- Coverage beyond the documented MC68000 instruction set and broader differential cases
  (EPIC-0002 E2-13 continued).
- Exception groups beyond the implemented interrupt path (guest bus error and trace).
- Privilege enforcement and separate user/supervisor stacks are delivered; keep them
  regression-tested rather than listing them as open scope.
- A wired level-7 interrupt source instead of a predicate rule only.
- Cycle and prefetch behavior only as far as [H5](#h5--accuracy-and-evidence) requires it.

#### Exit criteria

- The instruction/addressing matrix the slice documents is implemented and
  differential-tested.
- Privilege and user/supervisor separation are enforced and covered by tests; every
  documented exception group has an executable test.
- The H1 work items in [Milestones](milestones.md#h1--cpu-and-instruction-completeness) are
  checked with evidence.

#### Non-goals and tracked elsewhere

- No complete-68000 or complete-disassembly claim until coverage and tests support it; the M4
  disassembler stays bounded to the implemented subset.
- Cycle accuracy, prefetch fidelity and interrupt-delivery equivalence are
  [H5](#h5--accuracy-and-evidence), not H1.

### H2 — Video, VDP and DMA

- **Goal:** a VDP model that renders and transfers data, not only a memory-mapped port
  window.
- **Milestone:** [H2](milestones.md#h2--video-vdp-and-dma).
- **Depends on:** H1 (instruction coverage for realistic ROM code) and the stable
  non-rendering state model from M1.

#### Scope and deliverables

- Rendering: planes, sprites, scrolling and the visible-frame pipeline.
- CRAM and VSRAM.
- DMA transfers and their timing relative to the CPU.
- Raster effects and H/V timing.
- VDP interrupt-enable registers and VDP-side acknowledge logic (the CPU-side
  pending/acknowledge path already exists).
- FIFO behavior where it affects observable results.

#### Exit criteria

- Rendering, DMA, CRAM/VSRAM and raster timing are implemented and covered by differential or
  reference evidence.
- New device state enters canonical snapshots and state hashes through versioned payloads.
- The H2 work items in [Milestones](milestones.md#h2--video-vdp-and-dma) are checked with
  evidence.

#### Non-goals and tracked elsewhere

- DMA/rendering **inspector views** in the debugger are M4-FU2 and can only be verified after
  this track delivers the device model.
- GUI presentation is Stage 7; frame pacing is M7.
- Audio is H3.

### H3 — Audio and Z80

- **Goal:** model the second CPU and the sound chips instead of leaving those windows
  unmapped.
- **Milestone:** [H3](milestones.md#h3--audio-and-z80).
- **Depends on:** H1 (bus and CPU discipline) and H2 (bus-arbitration context).

#### Scope and deliverables

- Z80 CPU core and its memory map.
- Bus arbitration between 68000, Z80 and VDP.
- YM2612 FM synthesis and PSG tone generation, at a documented accuracy level.
- Audio sample production and timing relative to emulated ticks.

#### Exit criteria

- Z80 execution and bus arbitration are implemented and tested.
- YM2612 and PSG produce documented, deterministic output for reference programs.
- The H3 work items in [Milestones](milestones.md#h3--audio-and-z80) are checked with
  evidence.

#### Non-goals and tracked elsewhere

- Audio/Z80 **inspection views** are M4-FU2 until this model exists.
- Bit-exact hardware audio and cycle accuracy are [H5](#h5--accuracy-and-evidence)
  questions.
- Host audio output and GUI presentation are M7.

### H4 — Cartridge, controllers and timers

- **Goal:** load and identify cartridge images and model the full peripheral set the slice
  currently narrows.
- **Milestone:** [H4](milestones.md#h4--cartridge-controllers-and-timers).
- **Depends on:** M1 (the slice and its bus) and H1 for the code paths involved.

#### Scope and deliverables

- ROM header, region, TMSS and checksum handling, plus a cartridge loader and ROM identity
  (EPIC-0002 E2-11).
- Cartridge mapper (including SSF2-style banking) and SRAM.
- The full controller protocol: TH latch and TH-low phase, A/Start, 6-button X/Y/Z/Mode,
  controller timing and interrupts, port 3, analog and keyboard hardware.
- Timer interrupts.

#### Exit criteria

- A cartridge image with a recognized header/region loads, is identified and maps correctly,
  with SRAM persistence where supported.
- The controller protocol extends beyond the TH-high 3-button subset and is verified against a
  reference or synthetic fixture.
- Timer interrupts are implemented and tested.
- The H4 work items in [Milestones](milestones.md#h4--cartridge-controllers-and-timers) are
  checked with evidence.

#### Non-goals and tracked elsewhere

- No commercial-ROM compatibility claim follows from this track alone.
- Mapper/SRAM **inspection views** are M4-FU2; the cartridge loader is not M3 movie
  import/export (M3-FU1).
- Interrupt-delivery equivalence is [H5](#h5--accuracy-and-evidence).

### H5 — Accuracy and evidence

- **Goal:** measure accuracy instead of asserting it.
- **Milestone:** [H5](milestones.md#h5--accuracy-and-evidence).
- **Depends on:** H1–H4 progress and the existing oracle harness.

#### Scope and deliverables

- A broader differential corpus beyond the current thirteen synthetic ROMs.
- Positive differential comparison of interrupt delivery and timing, which needs an oracle
  model of those semantics.
- Cycle accuracy, including prefetch and bus timing, at a documented level.
- Cross-platform determinism measured for a real backend (FR-EXEC-006, NFR-DET-005).
- Performance measurement after correctness.

#### Exit criteria

- Each accuracy claim has executable evidence, not a code-reading argument.
- Interrupt delivery/timing has positive differential evidence, or the architecture docs state
  the absence of that claim.
- Cross-platform determinism is measured, not assumed.
- The H5 work items in [Milestones](milestones.md#h5--accuracy-and-evidence) are checked with
  evidence.

#### Non-goals and tracked elsewhere

- No game-compatibility claim follows from accuracy work alone.
- CI matrices and fuzzing stay cross-cutting (below); H5 owns the accuracy/evidence
  conclusions.

## Cross-cutting tracks

These run alongside every stage and are not milestones:

- Licensing and provenance review at every incorporation point
  ([policy](../licensing/policy.md)).
- Cross-platform CI (Linux, Windows, macOS where feasible).
- Fuzzing, property tests and a regression corpus.
- Documentation and ADR maintenance.

Performance measurement after correctness, and cross-platform determinism evidence, are
[H5](#h5--accuracy-and-evidence) conclusions rather than a separate track.
