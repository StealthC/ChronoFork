# ChronoFork

**Fork the past. Trace the cause.**

ChronoFork is an open-source, cross-platform emulation and reverse-engineering
platform. It treats an emulated machine as a timeline you can pause, save, branch
and compare. Instead of a single live window, every experiment is a session: a
self-contained machine state that can be checkpointed, cloned into independent
branches and replayed exactly.

The project is written in Rust and designed so that people, scripts and AI
agents use the same services, not three different worlds.

> **Project status:** the foundation is complete; milestones M0–M7 are complete and
> merged to `main` (M4 merged at `4c595a2`, M5 at `0c4e9f0` via PR #4, M6 and M7 at
> `cb05836` via PR #6). **M8 (functional Mega Drive emulator) is in progress**: phases
> 8.1 (cartridge/region/TMSS), 8.2 (complete 68000 and the cycle model) and 8.3
> (complete VDP, including rendering and DMA) are complete and merged; phases 8.4–8.8
> are pending, and M9 multi-backend is planned after M8. ChronoFork is **not a finished
> Mega Drive emulator** and does not run commercial games. See
> [what works today](#what-works-today) and [what comes next](#what-comes-next) for the
> honest picture; milestone status and integration state live once in the
> [milestone status table](docs/roadmap/milestones.md).

## Why ChronoFork exists

Most emulators are built for playing. Debugging, automation and experiments are
added later, around a machine loop that was never designed to be observed. If you
want to understand how a game reached a particular state, compare two runs or
repeat an experiment, you often end up with fragile scripts and notes kept
somewhere else.

ChronoFork starts from a different question: what if the emulated machine were a
deterministic state machine, and every interaction were an operation on a session
that can be saved, cloned, inspected and replayed? That one idea can serve a
person at a keyboard, a script and an AI agent.

It is built for people who study how software behaves: reverse engineers,
tool-assisted speedrun (TAS) authors, emulator developers, researchers and
automation agents.

## How ChronoFork is put together

The project is split into layers, each with one clear job:

| Layer | Responsibility |
|---|---|
| **Interfaces** | A command-line tool, HTTP and MCP daemon/client surfaces (M6), and a presentation GUI over the same services (M7). |
| **Sessions and storage** | The heart of the platform: start, step, save, restore, fork, compare and persist. |
| **Backend contract** | The rules and guarantees that any emulated machine must satisfy. |
| **Backends** | The machines themselves: a synthetic one for validation and a growing Mega Drive slice. |
| **Foundations** | Shared vocabulary: identities, virtual time, input and state hashes. |

Dependencies point in one direction: interfaces talk to sessions, sessions talk to
the backend contract, and concrete backends implement it. The session layer never
knows which machine is behind it, and no presentation layer owns the canonical
emulated state.

```text
interfaces      CLI, HTTP/MCP daemon and clients, GUI (presentation only)
     │
sessions        lifecycle, checkpoints, forks, storage
     │
contract        what a backend must provide and guarantee
     │
backends        synthetic validation · Mega Drive slice
     │
foundations     identities, virtual time, input, state hashes
```

## The road so far

The work is organized as verifiable milestones rather than dates. The full plan
is in [milestones](docs/roadmap/milestones.md).

| Milestone | Focus | State |
|---|---|---|
| M0 — Foundation | Contracts, synthetic backend, sessions, docs and quality gates | Complete |
| M1 — First hardware | 68000, ROM/RAM, VDP port window and interrupt scheduler | Complete for the modelled slice |
| M2 — Durable sessions | Checkpoints on disk, names, ownership and safe reloads | Complete; verified on local Linux filesystems |
| M3 — TAS and replay | Movie format, record/play/verify, rewind and branch editing | Complete for the documented deterministic minimal-slice scope; independently verified |
| M4 — Debugging | Breakpoints, watchpoints, views/disassembly and hardware inspectors | Complete; merged to `main` at `4c595a2`; verified on `agent/m4-debugging-depth` at `6d30243` |
| M5 — Tracing and scripting | Structured traces, comparison and sandboxed scripts | Complete; merged to `main` at `0c4e9f0` (PR #4); verified on `agent/m5-tracing-scripting` at base `84f1476` |
| M6 — Agent interfaces | HTTP and MCP over the same services | Complete; merged to `main` at `cb05836` (PR #6); verified on `agent/m6-agent-interfaces` at base `93d47ed` |
| M7 — GUI | Presentation that never owns canonical state | Complete; merged to `main` at `cb05836` (PR #6); verified on `agent/m7-gui` at base `93d47ed` |
| M8 — Functional Mega Drive | Complete 68000/Z80/VDP/audio and cartridge; playable commercial ROMs, headless capture and a native window | In progress (phases 8.1–8.3 complete; next: 8.4) |
| M9 — More systems | A second backend behind the same contract | Planned |

M0–M7 are merged to `main`; M4 was merged at `4c595a2` after being verified on
`agent/m4-debugging-depth` at `6d30243`, and M5 was merged at `0c4e9f0` (PR #4) after being
verified on `agent/m5-tracing-scripting` at base `84f1476` (final T12 PASS, 2026-11-16), so
a current `main` checkout exposes the `chronofork debug`, `chronofork trace` and
`chronofork script` surfaces. M6 was verified on `agent/m6-agent-interfaces` at base
`93d47ed` (pass-2 PASS, 2026-11-17) and M7 on `agent/m7-gui` at the same base
(final PASS4 and review REVIEW5 PASS, 2026-11-17); both are merged at `cb05836` (PR #6), so
a current `main` checkout exposes `chronofork serve`, `api`, `mcp`, `experiment` and
`chronofork gui`. The owner redefined M8 on 2026-11-17: it is now the **functional Mega
Drive emulator** (full 68000/Z80, VDP rendering/timing/DMA, YM2612/PSG audio, cartridge,
controllers, playable commercial ROMs, headless capture and a native window), and the former
multi-backend milestone became M9. The
[milestone status table](docs/roadmap/milestones.md) is the canonical record of
milestone status and integration state.

Hardware completeness is tracked in parallel tracks rather than stages: **H1**
CPU and instruction coverage, **H2** video/VDP/DMA, **H3** audio/Z80, **H4**
cartridge/controllers/timers and **H5** accuracy and evidence. A milestone can be
complete while a hardware track is partial, so completing M6 does not claim a
complete Mega Drive.

## What works today

**Deterministic execution.** The same inputs always produce the same state.
Sessions advance on explicit virtual ticks, never on the host clock, and every
state can be identified by a hash for comparison and verification.

**Sessions and branches.** Create, step, pause, resume, reset and close sessions.
Each session owns its machine state, so forks are independent copies: changes in
one branch never leak into another. The family tree of forks and the differences
between states are tracked as well.

**Checkpoints you can keep.** Save named checkpoints to a versioned on-disk
store, list them, reload them in a later process and delete them explicitly.
Restores reproduce the recorded state hash; corrupted or incompatible data is
rejected with a clear error rather than guessed at.

**Replay and verification.** Record an input sequence into a single `.cfmovie`
file, play it back and verify every step. If a replay diverges, ChronoFork reports
the exact first tick and state hash where it happened. Replay can start from the
embedded checkpoint in a fresh process, with no storage root.

**A first real-hardware slice.** The Mega Drive slice is the first machine that is
not a test fixture. It runs the documented MC68000 instruction set against small
synthetic programs, with ROM, work RAM, a basic video-chip port window and a
master-cycle deterministic interrupt scheduler. It covers interrupt priority
masking, the interrupt-return path and how the video chip holds and acknowledges
an interrupt request, and includes CPU, video and scheduler state in checkpoints
and forks. A bounded debug trace is available from the CLI, and an optional
comparison harness checks eleven synthetic programs against a pinned Exodus
reference: executed instruction addresses, per-instruction 68000-cycle costs and
selected final state match, and deliberately broken controls are caught.

**Controller input on the real slice.** The Mega Drive slice maps the controller
data registers `0x00A10003`/`0x00A10005` (ports 1–2) to the per-step input frame,
so a movie can be recorded, played and verified on the real core with `--rom`.
The implemented protocol is the TH-high 3-button subset
(Up/Down/Left/Right/B/C, active-low).

**Debugging depth.** A host debugging layer adds PC/range/conditional breakpoints,
byte-granular read/write watchpoints, event breakpoints and logpoints, bounded
register/memory/disassembly views, and Mega Drive CPU/VDP/interrupt/bus inspectors,
exposed by `chronofork debug run|inspect|memory|disasm|vdp|interrupts|bus`. Stops
are instruction-granular and debugger state is process-local. This surface was
verified on `agent/m4-debugging-depth` at `6d30243` and is merged to `main` at
`4c595a2`.

**Structured traces.** `chronofork-trace` adds a versioned, bounded trace format
(`CFTRACE1` v1) for instructions and observed bus-write, VDP-port, controller-read and
interrupt-phase events with emulated ticks. Capture supports one start and one stop trigger
(PC or tick), a circular buffer with a dropped counter, typed failure for malformed or
incompatible input, historical queries, first-divergence comparison and a deterministic
one-way TSV export. The surface is host-provided over the backend inspection contract, so
`Capabilities::tracing` stays `false` and bus/device records appear only where the backend
advertises `debug_events`. `chronofork trace capture/info/query/compare/export` exposes it.

**Scripted experiments.** `chronofork-script` adds a purpose-built deterministic script
language (integers, booleans and text; `let`/`set`, `print`, `assert`, `if`, `repeat`,
`while`; typed builtins for sessions, forks, inspection, trace capture/export/compare, TAS
verify/compare and report fields) with explicit source, operation, loop, variable, memory,
report, step, trace and wall-time limits, typed errors, a canonicalized `--sandbox`
capability boundary and a byte-deterministic report. `chronofork script run` exposes it.
No Rhai, Luau, Lua, WASM or Python engine is embedded; the reasons are recorded in
[ADR-0011](docs/adr/0011-tracing-and-scripting.md).

**Agent-native interfaces (M6).** A transport-agnostic
`chronofork-service` owns one curated 16-operation catalog, a strict canonical-JSON codec,
token-file authentication with `read`/`write`/`admin` scopes, per-principal session
ownership and an explicit concurrency boundary (one manager lock plus a per-session writer
lease, so a second concurrent mutation of the same session fails immediately with a typed
retryable `conflict`). `chronofork-transport` maps that catalog onto a hand-rolled bounded
HTTP/1.1 server (`GET /health`, `GET /v1/tools`, `POST /v1/ops`) and a stateless MCP JSON-RPC
endpoint at protocol `2025-03-26` (`initialize`, `ping`, `tools/list`, `tools/call`), with a
typed client — and **no new third-party dependency**. `chronofork serve` hosts the daemon;
`chronofork api` and `chronofork mcp` are clients; `chronofork session save/load/list/delete`
now run through the same service and gained `--json`. `experiment run` produces a canonical
`chronofork.experiment.v1` artifact with a content-derived `artifact_id` and no host time,
path or principal, so the same request yields byte-identical bytes on every run and surface.
Direct/HTTP/MCP parity is executed byte-for-byte over all 16 operations, including seven
error sections compared across the surfaces; a separate 19-case rejection matrix covers the
HTTP wire contract.

**Human-facing GUI (M7).** A presentation-only `chronofork-gui`
crate turns canonical service views into fixed-size text/ANSI frames and a self-contained
HTML document. It never owns or mutates canonical state: `chronofork-gui` depends only on
`chronofork-core` and `chronofork-service`, and the CLI is the only crate that composes the
HTTP/MCP transport. `chronofork gui run` is a stdin-driven, deterministic process surface
(`create`, `attach`, `focus`, `layout`, `panel`, `step`, `run`, `pulse`, `pause`, `handoff`,
`frame`, `html`, `detach`, `quit`) in local and remote modes, with a read-only `.cfmovie`
timeline, the eight documented panels, single/mosaic layouts, a built-in 60 fps pacer whose
host-time loop is testable through an injected clock, and a cooperative human/agent
controller lease. Attach/detach is proven non-perturbative, and every block, frame and
document is byte-identical across runs. Video and audio stay honestly `unavailable` in
v1: the pipeline is capability-gated and has no real video or audio source/sink (the
Mega Drive backend reports `rendering=true` since phase 8.3 for deterministic headless
frames and `audio=false`).

**Quality gates.** `./scripts/check.sh` runs formatting, compilation, Clippy, the
workspace tests and rustdoc as one local gate, and CI runs on every push and pull
request.

**Honest limits.** The Mega Drive slice is deliberately narrow. Audio (YM2612/PSG),
the Z80, timer interrupts, ROM header/region detection and cartridge mapping, and
several 68000 exception groups (guest bus error and trace, among others) are still
missing; rendering, DMA, the documented MC68000 instruction set and
privilege enforcement for the implemented system instructions are present (VDP
rendering/DMA since phase 8.3). Controller input covers only the TH-high
3-button subset on ports 1–2: A and Start, the TH/6-button multiplexing, controller
timing, port 3 and analog or keyboard hardware are absent. Canonical time is the
master-cycle timeline (each instruction adds its 68000-cycle cost times seven),
without functional prefetch, bus contention or interrupt-latency modelling. The synthetic backend is a
test fixture, not hardware emulation. TAS recording runs on the synthetic backend by
default and on the minimal Mega Drive slice with `--rom`; neither path implies game
compatibility.

Debugging stops at instruction boundaries: there is no reverse execution, no
code/data analysis or symbol mapping, and no DMA, audio or mapper inspection.
Tracing covers instruction, bus-write, VDP-port, controller-read and interrupt-phase records
only: there are no DMA/audio/Z80 sources, no generic bus-read or instruction-fetch record,
instruction-boundary stops only, canonical v1 only (no migration) and no TSV import. TSV
export is bounded by record count rather than the 64 MiB canonical byte cap. Scripting is a
small fixed DSL, not a host-language engine; its sandbox is a deterministic capability
boundary, not process isolation or a hostile-code security boundary, and wall-time checks
occur at statement and every-256-operation boundaries. Durable storage is verified on local
Linux filesystems only: the writer lock is cooperative, retention is manual, and
encryption, authentication and format migration are not implemented. No commercial ROMs,
BIOS or firmware are included or required.

The agent interfaces are deliberately narrow too. The HTTP server is HTTP/1.1 with one
request per connection and **no TLS, keep-alive, chunked bodies or SSE**, so it suits
loopback or sidecar use and needs a reverse proxy for anything else. `POST /mcp` is
stateless: no `Mcp-Session-Id`, no server-initiated messages, no resources/prompts/logging.
Tokens are coarse and static with no rotation, expiry or audit log. Session ownership and
live sessions are process-local and lost on daemon restart, while durable store objects
carry no per-principal owner. Closed sessions are retained for the process lifetime and keep
counting toward the session bound. Debug, trace, script and TAS operations are **not**
exposed over transport — they stay CLI-local. One backend configuration is chosen per
daemon, and there is no performance claim.

The GUI is deliberately narrow. v1 presents **text and a static HTML document only**: there
is no native toolkit window, no picture and no sound, no browser/JS frontend, no streaming
transport, no live audio output, no GUI input injection, no GUI state persistence and no
VDP/bus/disassembly/trace panel content (those inspector operations are not exposed over
transport, M6-FU1). Real video and audio sources/sinks wait for the H2/H3 hardware tracks,
so the capability-gated pipeline reports both as unavailable in every v1 process
configuration. The CLI never builds the host clock: frame playback is the deterministic
`pulse`, and live host-time playback remains a follow-up; the controller lease is
cooperative and window-scoped, not an authorization mechanism. The evidence is
Linux/WSL-only and makes no performance or cross-platform claim.

## What comes next

M0–M7 are merged to `main` (M4 at `4c595a2`, M5 at `0c4e9f0` via PR #4, M6 and M7 at
`cb05836` via PR #6). **M8 is the functional Mega Drive emulator** and is in progress:
phases 8.1–8.3 are complete and merged, and phases 8.4–8.8 remain. M8 deliberately
sequences the hardware-depth tracks to completion instead of waiting for them in
parallel:

- **Complete the Mega Drive model (M8, decomposed by H1–H5).** The full Z80,
  YM2612/PSG audio, ROM/cartridge work beyond the phase-8.1 header/TMSS/Sega mapper
  (mapper variants and SRAM banking), complete controller and timer behaviour,
  cycle-based time with unpaced/paced execution modes, and deterministic headless
  capture of frames and audio. The complete 68000 (phase 8.2) and the complete VDP with
  rendering/DMA/timing (phase 8.3) are already delivered. A curated commercial-ROM set
  is the playability proof; the native window is the last phase and still needs an
  ADR that supersedes ADR-0013's no-toolkit constraint.
- **Close the replay follow-ups.** Import/export adapters (for example BK2),
  checkpoint-reference movies and in-place/timeline editing are documented
  follow-ups, not delivered features; 6-button and analog controller hardware
  belongs to H4.
- **Land the actionable M6/M7 follow-ups.** Both are merged; the remaining recorded
  follow-ups are the agent operations over transport (M6-FU1, where
  [ADR-0016](docs/adr/0016-session-service-and-agent-api.md) records the owner's selected
  direction and is still `Proposed`), durable per-principal ownership and
  restart-persistent live sessions, TLS with token rotation/expiry and an audit log, and
  SSE/streaming with the wider MCP surface; and on the GUI side a native toolkit frontend
  after H2/H3, real video/audio sources and sinks, live host-time playback, GUI input
  editing/settings and the inspector/trace panel content. Do not implement a `Proposed`
  ADR as accepted.
- **Grow beyond one system (M9).** After M8, a second backend behind the same contract,
  with capability differences reported honestly.

## Try it

Requirements: Rust 1.85 or newer (tested with 1.100.0), Git and a C linker. The
synthetic backend needs no BIOS, firmware or commercial ROM.

```sh
git clone <repository-url> ChronoFork
cd ChronoFork
cargo test --workspace
cargo run -p chronofork-cli -- demo
cargo run -p chronofork-cli -- megadrive demo
```

Both demos print their evidence and end with `PASS`. The synthetic demo shows a
checkpoint, three independent forks, divergent states and an exact replay. The
Mega Drive demo shows the CPU, VDP and interrupt slice, including a snapshot,
fork and restore.

More things to try:

```sh
# See what the current backend honestly claims
cargo run -p chronofork-cli -- backend describe

# Keep a checkpoint on disk
cargo run -p chronofork-cli -- session save --root ./cf-store --name first
cargo run -p chronofork-cli -- session list --root ./cf-store

# Record, replay and verify a short input movie
cargo run -p chronofork-cli -- tas record --movie ./demo.cfmovie --steps 8
cargo run -p chronofork-cli -- tas verify --movie ./demo.cfmovie

# Capture, inspect, query and compare a structured trace
cargo run -p chronofork-cli -- trace capture --steps 8 --capacity 64 --out ./demo.cftrace
cargo run -p chronofork-cli -- trace info --file ./demo.cftrace
cargo run -p chronofork-cli -- trace compare --left ./demo.cftrace --right ./demo.cftrace

# Run a sandboxed deterministic experiment report
printf 'let n = 3\nrepeat n { print n }\nreport("ok", true)\n' > ./demo.cfscript
cargo run -p chronofork-cli -- script run --file ./demo.cfscript

# Drive the presentation surface: create, attach, pulse, frame, quit
printf 'create\nattach 1\npulse 3\nframe\nquit\n' | cargo run -p chronofork-cli -- gui run
```

## What guides the project

- **Determinism before performance.** A fast wrong answer is worse than a
  reproducible one.
- **Isolation before sharing.** Mutable machine state belongs to exactly one
  session.
- **Capabilities are promises.** A backend only advertises what it can actually
  do.
- **Evidence before claims.** Snapshot, replay and accuracy claims need
  executable tests.
- **Generic platform, specific hardware.** Session and storage layers stay free
  of console assumptions.
- **Harness independence.** The project should work for humans and any coding
  agent, not one vendor's tooling.

## Documentation

Technical documentation lives in [`docs/`](docs/README.md). Start with the index,
then follow the area you need:

- **Why and what:** [product vision](docs/vision/product-vision.md),
  [scope](docs/vision/scope.md) and [roadmap](docs/roadmap/roadmap.md).
- **How it works:** [architecture overview](docs/architecture/overview.md),
  [backend model](docs/architecture/backend-model.md),
  [session model](docs/architecture/session-model.md) and
  [execution model](docs/architecture/execution-model.md).
- **The Mega Drive slice:** [implemented hardware and exact
  limits](docs/architecture/megadrive-slice.md).
- **Decisions:** [architecture decision records](docs/adr/README.md).
- **Working on the project:** [getting started](docs/development/getting-started.md),
  [testing strategy](docs/quality/testing-strategy.md) and [quality
  gates](docs/quality/quality-gates.md).
- **Licensing and reuse:** [policy](docs/licensing/policy.md) and
  [provenance](docs/licensing/provenance.md).

## Research and inspiration

ChronoFork learns from existing emulators but is not a reskin, and it contains no
upstream code. The main research inputs are
[RogerSanders/Exodus](https://github.com/RogerSanders/Exodus) (MIT, debugging
oriented), [paulomanrique/exodus-for-ais](https://github.com/paulomanrique/exodus-for-ais)
(MIT, headless port) and [TASEmulators/BizHawk](https://github.com/TASEmulators/BizHawk)
(multi-system emulator with heterogeneous licensing, studied for TAS and
debugging ideas only). Reference clones live under the ignored
`.research/upstream/` directory and are never runtime dependencies. See
[provenance](docs/licensing/provenance.md) for exact revisions and reuse rules.

## Contributing

Contributions are welcome. Start with [`AGENTS.md`](AGENTS.md) and the
[documentation index](docs/README.md), keep changes small and focused, and run
`./scripts/check.sh` before opening a pull request. Durable behavior belongs in
`docs/` or an ADR, not only in code or chat.

## License

ChronoFork's own code is released under the MIT license (provisional while the
upstream licensing review is completed). See [LICENSE](LICENSE) and
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Do not commit commercial ROMs, BIOS/firmware images, game assets or copyrighted
test data; the tests use independently authored synthetic programs. ChronoFork is
not affiliated with or endorsed by Sega, the Exodus authors, the BizHawk team or
any referenced emulator project.
