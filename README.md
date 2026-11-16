# ChronoFork

**Fork the past. Trace the cause.**

ChronoFork is an open-source, cross-platform emulation and reverse-engineering
platform. It treats an emulated machine as a timeline you can pause, save, branch
and compare. Instead of a single live window, every experiment is a session: a
self-contained machine state that can be checkpointed, cloned into independent
branches and replayed exactly.

The project is written in Rust and designed so that people, scripts and AI
agents use the same services, not three different worlds.

> **Project status:** the foundation is complete; milestones M0–M4 are complete and
> merged to `main` (M4 merged at `4c595a2`), but ChronoFork is **not a finished Mega
> Drive emulator** and does not run commercial games. See [what works today](#what-works-today)
> and [what comes next](#what-comes-next) for the honest picture.

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
| **Interfaces** | A command-line tool today; HTTP, MCP and a GUI are planned. |
| **Sessions and storage** | The heart of the platform: start, step, save, restore, fork, compare and persist. |
| **Backend contract** | The rules and guarantees that any emulated machine must satisfy. |
| **Backends** | The machines themselves: a synthetic one for validation and a growing Mega Drive slice. |
| **Foundations** | Shared vocabulary: identities, virtual time, input and state hashes. |

Dependencies point in one direction: interfaces talk to sessions, sessions talk to
the backend contract, and concrete backends implement it. The session layer never
knows which machine is behind it, and no presentation layer owns the canonical
emulated state.

```text
interfaces      CLI today; HTTP, MCP and GUI planned
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
| M5 — Tracing and scripting | Structured traces, comparison and sandboxed scripts | Next — not started |
| M6 — Agent interfaces | HTTP and MCP over the same services | Planned |
| M7 — GUI | Presentation that never owns canonical state | Planned |
| M8 — More systems | A second backend behind the same contract | Planned |

M0–M4 are merged to `main`; M4 was merged at `4c595a2` after being verified on
`agent/m4-debugging-depth` at `6d30243`, so a current `main` checkout exposes the
`chronofork debug` surface. The
[milestone status table](docs/roadmap/milestones.md) is the canonical record of
milestone status and integration state.

Hardware completeness is tracked in parallel tracks rather than stages: **H1**
CPU and instruction coverage, **H2** video/VDP/DMA, **H3** audio/Z80, **H4**
cartridge/controllers/timers and **H5** accuracy and evidence. A milestone can be
complete while a hardware track is partial, so starting M5 does not claim a
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
not a test fixture. It runs a documented subset of the main 68000 processor
against small synthetic programs, with ROM, work RAM, a basic video-chip port
window and a deterministic interrupt scheduler. It covers interrupt priority
masking, the interrupt-return path and how the video chip holds and acknowledges
an interrupt request, and includes CPU, video and scheduler state in checkpoints
and forks. A bounded debug trace is available from the CLI, and an optional
comparison harness checks three synthetic programs against a pinned Exodus
reference: executed instruction addresses and selected final state match, and
deliberately broken controls are caught.

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

**Quality gates.** `./scripts/check.sh` runs formatting, compilation, Clippy, the
workspace tests and rustdoc as one local gate, and CI runs on every push and pull
request.

**Honest limits.** The Mega Drive slice is deliberately narrow. Rendering, DMA,
audio (YM2612/PSG), the Z80, timer interrupts, ROM header/region detection and
cartridge mapping, privilege enforcement, most 68000 instructions and the other
68000 exception groups are still missing. Controller input covers only the TH-high
3-button subset on ports 1–2: A and Start, the TH/6-button multiplexing, controller
timing, port 3 and analog or keyboard hardware are absent. Time moves in
instruction-sized steps rather than real clock cycles. The synthetic backend is a
test fixture, not hardware emulation. TAS recording runs on the synthetic backend by
default and on the minimal Mega Drive slice with `--rom`; neither path implies game
compatibility.

Debugging stops at instruction boundaries: there is no reverse execution, no
code/data analysis or symbol mapping, and no DMA, audio or mapper inspection.
Durable storage is verified on local Linux filesystems only: the writer lock is
cooperative, retention is manual, and encryption, authentication and format
migration are not implemented. No commercial ROMs, BIOS or firmware are included
or required.

## What comes next

M0–M4 are merged to `main` (M4 at `4c595a2`); M5, tracing and scripting, is the
next milestone. Hardware depth runs in parallel, so the stage sequence does not
wait for a complete machine model:

- **Make traces and scripts first-class (M5).** Structured trace formats with
  comparison and export, triggered and circular tracing, followed by a sandboxed
  scripting runtime.
- **Deepen the Mega Drive model (H1–H5).** Broader 68000 and exception coverage,
  rendering/VDP/DMA, audio and the Z80, ROM identity and a cartridge loader,
  complete controller and timer behaviour, and stronger accuracy evidence.
- **Close the replay follow-ups.** Import/export adapters (for example BK2),
  checkpoint-reference movies and in-place/timeline editing are documented
  follow-ups, not delivered features; 6-button and analog controller hardware
  belongs to H4.
- **Open up to agents (M6).** HTTP and MCP interfaces over the same services, with
  a typed tool surface and permissions rather than a pile of loose endpoints.
- **Build the GUI (M7).** Video, audio, frame pacing and debugger panels that
  attach to and detach from sessions without changing results.
- **Grow beyond one system (M8).** A second backend behind the same contract, with
  capability differences reported honestly.

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
