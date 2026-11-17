# ADR-0013: Human-facing GUI — presentation over canonical views, not a state owner

- **Status:** **Accepted and implemented** (owner-approved 2026-11-17 by the explicit Stage 7
  implementation directive, "implemente completamente o stage 7 do roadmap"; implementation
  independently verified 2026-11-17 — final PASS4 and review REVIEW5 PASS, see
  [§7](#7-verification-and-evidence)). M7 is `Complete with integration pending`: verified
  on the uncommitted `agent/m7-gui` worktree and not committed, merged or published.
- **Date:** 2026-11-17.
- **Milestone:** [M7 — GUI](../roadmap/milestones.md#m7--gui).
- **Roadmap stage:** [Stage 7 — Human-facing GUI](../roadmap/roadmap.md#stage-7--human-facing-gui).
- **Companion contract:** [GUI architecture](../architecture/gui.md) (crate modules, view
  types, command-session contract, pacing formulas, handoff state machine, numeric limits)
  and the executable [M7 acceptance plan](../quality/m7-gui-acceptance.md). §3.9 below is
  the **normative** command grammar; every other section states a decision and its
  justification.
- **Supersedes:** nothing. Extends [ADR-0001](0001-rust-workspace.md) (crate boundaries),
  [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md) (why a TUI was rejected
  for that increment), [ADR-0010](0010-debugging-depth.md) (headless-first views) and
  [ADR-0012](0012-agent-native-interfaces.md) (the service/transport boundary this layer
  consumes).

## 1. Context

Stages 0–6 delivered a deterministic session layer, durable storage, TAS movies, a host
debugger, structured tracing, a sandboxed script runtime and — in M6 — a
transport-agnostic application service with bounded HTTP/1.1 and MCP adapters. Every
capability is reachable from a process surface, and a second process can already attach to
a live session over the M6 transport. What does not exist is a **human-facing presentation
layer**: no windows, no panels, no frame pacing, no attach/detach model and no human/agent
control handoff. [FR-GUI-001](../requirements/functional.md) is `Planned` and
[FR-EXEC-007](../requirements/functional.md) is `Partial` precisely because frame pacing
and a host-time run loop are M7 work.

The baseline this decision builds on (all accepted, implemented and independently
verified):

- **Service/transport boundary** ([ADR-0012](0012-agent-native-interfaces.md),
  [agent interfaces](../architecture/agent-interfaces.md)): one curated 16-operation
  catalog, canonical strict JSON, token/scope/ownership policy and the only
  synchronization boundary. `session.create` returns a session, its state/tick/state hash,
  its lineage and a `backend_id` — **not** a backend descriptor; the full descriptor and
  its capability flags come from `session.inspect` or the `capabilities` operation.
  `chronofork-transport` is the only crate that knows HTTP/MCP framing.
- **Headless-first views** ([ADR-0010](0010-debugging-depth.md) §2 item 6, [testing
  strategy](../quality/testing-strategy.md) §8 G5): every observation surface must be
  usable and provable without a display. The M4/M5/M6 acceptance rows are all executed as
  process or library tests, never through a window.
- **A rejected TUI** ([ADR-0007](0007-in-process-rewind-and-interactive-stepping.md)
  alternatives table): "TUI/REPL with line editing (readline, ratatui)" was rejected for
  that increment because of "new dependency, terminal assumptions, hard to test in CI and
  non-deterministic rendering; the GUI stage is separate". That rejection binds this ADR:
  a GUI that needs a terminal library, a display or non-deterministic drawing would
  contradict a decision already accepted.
- **Honest capabilities**: a capability is advertised only when tests back it
  ([anti-slop policy](../quality/anti-slop-policy.md)), and `Capabilities::rendering` /
  `Capabilities::audio` are `false` for both backends today (the M6 smoke test in the
  captain provenance record shows `rendering=false`, `audio=false`).
- **Repository rules** ([AGENTS.md](../../AGENTS.md)): no `unsafe`, minimal dependencies,
  no implicit host wall-clock input to emulated state, one source of truth per rule and
  format, bounded resources.

Stage 7 depends on M6 (attach/detach transport) **and** on
[H2](../roadmap/roadmap.md#h2--video-vdp-and-dma)/[H3](../roadmap/roadmap.md#h3--audio-and-z80)
for video and audio to present. H2 and H3 are partial/planned: there is no renderer and no
audio device. A GUI contract that pretended otherwise would be the exact failure the
anti-slop policy forbids.

## 2. Problem

What shape can a human-facing GUI take **now** so that all of the following hold at once?

1. It never owns, mutates or re-derives canonical emulated state; the session layer stays
   the only owner.
2. It is verifiable headlessly in CI — byte-deterministic, no display, no timing
   dependence — because acceptance in this repository is executed, not observed.
3. It adds no third-party dependency and no `unsafe`, and keeps the crate dependency
   direction acyclic (`core ← backend-api ← sessions ← service ← transport`).
4. It reports video and audio honestly as unavailable until H2/H3 exist, while still
   carrying the capability-gated pipeline that will consume them.
5. It supports attach/detach across process boundaries (M6) and a human/agent control
   handoff with observable, deterministic results.
6. It exposes frame pacing and a host-time run loop (FR-EXEC-007) **without** letting host
   time enter canonical state or emitted bytes.

## 3. Decision

### 3.1 A new presentation crate, `chronofork-gui`, with one injected operation channel

A new workspace crate, `crates/chronofork-gui`, owns the command grammar and its executor,
the presentation model, the renderers, the pacing/run loop, the panel/availability rules and
the controller lease.

- **Dependencies:** direct workspace dependencies are `chronofork-core` (digests) and
  `chronofork-service` (canonical JSON value, codec, `ServiceLimits`, `ServiceError`).
  The crate **never** depends on `chronofork-transport`, `chronofork-sessions`,
  `chronofork-backend-api` or a concrete backend. No third-party dependency is added; the
  workspace `unsafe_code = "forbid"` lint applies unchanged.
- **No ambient capabilities:** the crate opens no socket, reads no file, no environment
  variable, no token and no host clock except through the injected `HostClock`. The CLI
  owns every one of those.
- **One channel trait.** The crate talks to the outside world through exactly one seam:

  ```rust
  /// One canonical operation call over an already-encoded request document.
  pub trait OperationChannel: Send + Sync {
      fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault>;
  }
  ```

  The request is the canonical `{"op":...,"input":...}` document and the response is the
  canonical envelope, so the crate reuses the service codec and never invents a second
  one. `ChannelFault` carries a bounded, path-free `detail` for the diagnostic and the
  canonical `chronofork.error.v1` bytes when the peer produced them.
- **The CLI is the composition root** and the only crate that knows HTTP/MCP. It
  implements the trait twice: `RemoteChannel` over `chronofork_transport::HttpClient`
  (`call_raw`), and `LocalChannel` over `chronofork_service::AppService::dispatch` with an
  in-process admin principal. Token reading, principal selection, flag parsing, stdin and
  stdout, the `--html` write, formatting the returned block data and process exit codes
  stay in the CLI.
- **The grammar is executed in the crate, not in the CLI.** A `command` module owns the
  normative grammar of §3.9: `CommandSession` parses one stdin line and executes it over
  the injected `OperationChannel`, the `GuiModel`, the `Pacer` and the `Lease`, returning
  typed outcomes — a block (typed, ordered field values the CLI renders as `key=value`
  lines), a serialized frame grid, or the HTML document plus its block — or a typed
  `GuiError` whose `code()` is one of §3.9's. The exact Rust shape is in
  [gui.md §6.1](../architecture/gui.md#61-command-session-contract). This is what makes the
  exact operation set, the control gate and the attach/detach flow provable by library
  tests instead of only through a spawned process, and it keeps a single implementation of
  the grammar for local and remote modes.
- **Why not put the presentation in the CLI:** the model, renderers, pacing, lease and
  command grammar would then be untestable without spawning the process, and the layer
  limits could not be enforced by `cargo tree`. **Why not put it in the service:**
  presentation types, a frame grid and a stdin grammar have no business in the application
  catalog, and the service must stay free of UI policy exactly as it stays free of wire
  framing.

### 3.2 Presentation consumes immutable views and controls only through authorized operations

The model holds **copies of values**, never a handle to a session, a manager, a backend or
a snapshot. It reads only these canonical operations:

| View | Operation | Scope | Notes |
|---|---|---|---|
| Service/backend capabilities | `capabilities` | read | fetched once at startup; gates the video/audio pipeline |
| Session list | `session.list` | read | ascending session id; `truncated` preserved |
| Session view (state, lineage, descriptor, machine) | `session.inspect` | read | the **only** source of the full descriptor and its capability flags |
| Memory window | `session.read_memory` | read | bounded window of `MAX_MEMORY_PANEL_BYTES` (see [gui.md §7](../architecture/gui.md#7-limits)) |

It mutates only through `session.create` — always allowed, because it is the local-mode
bootstrap and an observation-path command — and `session.run` (`step`, `run`, `pulse`),
which executes only while the active controller is `Human` (§3.8). It never calls
`session.fork`, `session.snapshot`, `session.restore`, `session.close`, `session.compare`,
`store.*` or `experiment.run`; those remain CLI/agent surfaces and adding them later is a
new decision, not an implied permission.

Two consequences are normative:

1. **`session.create` is not a descriptor.** It returns `session`, `state`, `tick`,
   `state_hash`, `lineage` and `backend_id` only. The GUI must obtain the descriptor and
   capability flags from `session.inspect` (or `capabilities`); it must not assume fields
   the operation does not return.
2. **Attach is a read.** Attaching performs `session.inspect` (plus one bounded
   `session.read_memory` when the machine exposes memory) and nothing else, so attach/detach
   cannot change emulation results. That is the structural reason the M7 exit criterion
   holds, not a promise.

Because the crate cannot name a session, backend or snapshot type, "the GUI never owns
canonical state" is a compile-time property of the dependency graph, not a code-review
convention.

### 3.3 Windows, panes, mosaic layouts and attach/detach with a generation

- One **window** is the single rendered surface of one `chronofork gui run` process (one
  stdout, one HTML artifact). A **pane** is one attached session slot; a **layout** is
  `single` (render the focused pane) or `mosaic` (render up to four panes in a 2×2 grid).
- At most `MAX_PANES` sessions are attached at once (four); a fifth attach is a typed
  `limit_exceeded`. One pane is focused at a time; the focused pane is the target of
  `detach`, `step`, `run` and `pulse`.
- **Attach/detach generation.** Every attach on a pane increments that pane's
  `generation` (starting at 1) and reports it in the `attach` block, which is the pane's
  attachment identity; the status panel reports the controller lease's `generation` and
  `sequence`, not the attach generation. Detaching clears the attachment. Re-attaching the same session into the same pane after a
  detach produces a new generation and the **same canonical panel content and the same
  tick/state**; the canonical state never changes. The presentation `events` log is the one
  panel that changes by design — it records the attach/detach sequence — so a
  byte-for-byte frame comparison must hide it (`panel events off`) or mask its line, and the
  acceptance plan compares the canonical panels that way.
- **Idempotent attach.** Attaching a session that is already attached focuses its pane and
  does not increment the generation. A `focus` on a pane that holds no session is a typed
  `unknown_pane`; `detach` with no attachment is a typed `not_attached`.
- Frames are fixed-size: `--width` × `--height` cells, printable ASCII only, content
  clipped (never wrapped), every line padded to exactly the configured width. Mosaic
  regions are computed from the frame size, not from host font metrics, so the same model
  always renders the same bytes.

### 3.4 Deterministic text/ANSI and self-contained HTML renderers are the GUI v1

The frontend of record is the **terminal**: a plain-text renderer and an ANSI renderer that
adds SGR emphasis only (no cursor movement, no clearing, no alternate screen, no terminal
size query). The second frontend is a **self-contained HTML document**: one file, inline
CSS, total escaping, **no JavaScript**, no external reference of any kind.

Rejected or deferred, with reasons:

| Alternative | Status | Why |
|---|---|---|
| Native toolkit (`winit`, `egui`, `minifb`, …) | **Rejected for M7** | A windowing dependency tree (plus platform backends, MSRV and provenance work) for a layer that has no video to present (`rendering=false`); unverifiable headlessly in CI, where every acceptance row in this repository must execute; it repeats the surface [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md) rejected for that increment (new dependency, terminal/display assumptions, non-deterministic rendering, hard to test in CI). |
| Browser frontend with JavaScript (canvas/WebGL) | **Deferred** | Needs a JS runtime contract, a browser in CI and an XSS/escape surface that a no-JS document does not have; no H2 renderer exists to feed it. |
| Native toolkit **after** H2/H3 | **Follow-up** | Becomes defensible once frames exist; then the pipeline in §3.6 already has a source and a sink to plug into. |
| WebSocket/SSE live view | **Rejected for M7** | M6 has no streaming transport (M6-FU4); a GUI that needed one would reopen an accepted transport decision. |

The HTML renderer is not decoration: it is the shareable artifact that proves the model is
presentation-only and that escaping is total. Both renderers render the **same** model and
the same frame grid; the ANSI output must equal the plain output after removing SGR
sequences, and the HTML `<pre>` blocks must equal the plain frames after entity decoding.
Both facts are executable assertions, not claims.

### 3.5 Frame pacing and a host-time run loop with an injected clock

- `HostClock` (`now_nanos`) is injected. `ManualClock` is the deterministic implementation
  used by tests **and by the CLI**, so no emitted byte depends on host time. `SystemClock`
  is the library's real-clock implementation of the same trait: no v1 process path builds
  it, and live host-time playback is a follow-up.
- `Pacer` owns the cadence: `fps` (frames per second), `ticks_per_frame` (canonical
  `session.run` steps per presented frame) and `max_catch_up_frames` (how many frames a
  late host may execute in one step before the rest are counted as dropped). Two entry
  points exist, and they are deliberately different:
  - `pulse(frames)` — explicit, clock-independent: advances exactly `frames` whole
    presentation frames, never drops, never partially advances a frame. The CLI's `pulse`
    command uses it, which is why the process surface is byte-deterministic.
  - `advance_elapsed(now_nanos)` — the host-time run loop: converts elapsed host time into
    due frames, advances at most `max_catch_up_frames`, and reports the remainder as
    `dropped` so a slow host cannot spiral. It is the **library** implementation of the
    FR-EXEC-007 run loop, proven by unit tests driven by `ManualClock` at chosen instants;
    the v1 CLI process does not call it (it exposes only the deterministic `pulse`), and
    live playback is a follow-up.
- **Host time never enters canonical state or emitted data.** The pacer advances the
  session only through `session.run`; it holds no session. Elapsed time appears only in the
  pacer report and in the `pulse` block, computed from the manual clock, so it is a
  deterministic function of the requested frames and `fps`.
- **`pause` gates the host-time loop, not `pulse`.** `pause` toggles the presentation pause
  flag. While the flag is set, `CommandSession::advance_elapsed` pins the clock reading to
  `pacer.last_nanos()`, so `due`, `advanced`, `dropped` and `steps` are all zero, no
  `session.run` is issued and the time that elapses while paused is not consumed; the next
  unpaused step sees that time as due, caps the advance at `max_catch_up_frames` and reports
  the remainder as dropped (the §5.2 rule). The explicit `pulse(frames)` path is
  deliberately **not** gated by the flag. The v1 CLI never auto-advances, so `pause` reports
  the flag and gates the library loop; it is still never a canonical pause. The behaviour is
  covered by `pause_gates_the_host_time_loop_but_not_pulse` in
  `crates/chronofork-gui/tests/command.rs`.
- **Determinism over live playback.** A host-time auto-advancing loop inside the CLI would
  make stdout depend on wall-clock scheduling and therefore break the byte-comparison
  acceptance model. v1 therefore exposes deterministic `pulse` and keeps the host-time
  loop as a library capability with a real clock implementation; live playback arrives with
  the native toolkit follow-up.

### 3.6 Video and audio are capability-gated pipelines, never fabricated semantics

The crate defines the presentation seam and nothing more:

- `PresentationFrame` (bounded RGBA8 buffer plus a sequence number) and `AudioChunk`
  (bounded `i16` samples plus a sequence number and sample rate);
- `VideoSource` and `AudioSink` traits;
- `Presentation<T>` = `Available(T) | Unavailable(reason)` where the reasons are
  `CapabilityDisabled` (the descriptor says `rendering=false` / `audio=false`),
  `NoOutputDevice` (v1 has no audio device) and `NotExposedByTransport` (the M6 surface
  does not carry the data).

Rules:

1. The gate is the **capability flag from the canonical descriptor**. When
   `rendering=false`, the video pipeline is `Unavailable{CapabilityDisabled}` and the
   source is never called; when `audio=false`, the audio pipeline is
   `Unavailable{CapabilityDisabled}` and the sink is never called. Both current backends
   report `false`, so this is the only honest path today; the frame status line reports
   `rendering=unavailable` / `audio=unavailable` and the `attach` block reports the
   descriptor flags that produced that state.
2. **v1 has no output device.** The terminal and HTML frontends present no pixels and play
   no samples, so the CLI configures no video or audio sink: with a descriptor that
   advertises `rendering=true` or `audio=true`, the pipeline is
   `Unavailable{NoOutputDevice}` rather than a fabricated picture or a silent claim of
   playback. A process never reports video or audio as available in v1.
3. A deterministic **test-pattern source** proves the `Available` path. It is a test
   fixture, not production code: no shipped path fabricates a picture or a sound from
   nothing.
4. The crate adds **no** rendering, timing, resampling or device semantics. H2 owns video
   depth and H3 owns audio depth; when they land, they implement `VideoSource`/`AudioSink`
   and this contract does not change.
5. Frame digests use `chronofork-core`'s SHA-256, so a presented frame is comparable
   byte-for-byte like every other canonical artifact.

### 3.7 Panels, their honest availability, and the TAS timeline

The window renders a fixed set of panels: `status`, `lineage`, `capabilities`,
`registers`, `memory`, `inspectors`, `timeline`, `events`. Each is backed by a canonical
view or is explicitly `Unavailable` with a reason; **no panel invents data**.

- `status`, `lineage`, `capabilities`, `registers`, `memory` come from `session.inspect`
  and `session.read_memory`.
- `inspectors` reports the debugger inspector sections **section by section**: `cpu` is
  available from the inspection registers; `vdp`, `interrupts`, `bus` and `disassembly` are
  `Unavailable{NotExposedByTransport}` because the M6 transport does not expose debug
  operations (**M6-FU1**). The panel never renders a placeholder picture of a device.
- `timeline` is the TAS timeline. The GUI crate cannot read a file and must not depend on
  `chronofork-sessions`, so the **CLI** opens the `.cfmovie` read-only with the existing
  bounded loader and passes a plain `TimelineView` value into the model. Without
  `--movie` the panel is `Unavailable{NoMovie}`. The movie file is never written, moved or
  modified.
- `events` is a bounded in-process GUI event log (attach, detach, focus, layout, panel,
  step, pulse, handoff) with monotonic sequence numbers, oldest-first eviction and a
  dropped counter. It records **presentation** events only: it is not a canonical trace and
  it carries no host time.

### 3.8 Human/agent handoff is a cooperative lease, and the hard boundary stays in the service

- `Controller` is `Human` (default) or `Agent`. The lease is window-scoped and carries a
  `generation` and a `sequence`, both incremented on every effective transition; handing
  off to the controller that already holds the lease is a no-op that reports
  `changed=false` without touching either counter.
- **Control commands** (`step`, `run`, `pulse`) execute only while the active controller is
  `Human`. Under `Agent` they are rejected with the typed code `control_denied`, which
  reports the current generation and sequence. `handoff` itself is always allowed — it is
  the transition, not a control action — and observation commands (`create`, `list`,
  `attach`, `detach`, `focus`, `layout`, `panel`, `frame`, `html`, `quit`) always work.
- Every transition and every rejected control attempt is recorded in the presentation event
  log as its own event (`handoff`, `step`, `pulse`, …). An `Event` carries its own monotonic
  `sequence`, its `EventKind`, the pane and the session; the lease's `generation`/`sequence`
  are reported by the `handoff`/`step`/`pulse` blocks and by the status panel, not stored in
  the event. That is what makes "a human and an agent handed control back and forth with
  observable, deterministic results" a byte-level assertion.
- **The lease is cooperative, not enforcing.** It prevents a human surface from driving
  while an agent holds control; it cannot and does not lock the canonical session. The hard
  boundaries remain the M6 ones: bearer-token scopes, per-principal ownership and the
  per-session writer lease. An agent that holds the token can always call `session.run`
  regardless of the GUI's lease, and the GUI must not claim otherwise. This is documented
  in the architecture page and stated in the ADR consequences.
- The remote handoff test uses exactly that shape: a **second process**,
  `chronofork api call --op session.run`, is the agent. The GUI observes the resulting tick
  change through `session.inspect`, cannot step while the lease is `Agent`, and steps again
  after `handoff human`.

### 3.9 The `chronofork gui run` process surface (normative)

`chronofork gui --help` and `chronofork gui run --help` print usage to stdout and exit 0.

```text
chronofork gui run [OPTIONS]
```

Options (each at most once; an unknown, repeated or malformed option exits 1 with empty
stdout):

| Flag | Meaning |
|---|---|
| `--remote <URL>` | Remote mode: attach to a running `chronofork serve` daemon at this base URL. Requires `--token-file`; mutually exclusive with `--rom`. |
| `--token-file <FILE>` | `chronofork.tokens.v1` bearer-token file for remote mode. Required iff `--remote`; the first principal is used. Never echoed. |
| `--rom <FILE>` | Local mode backend selection: the minimal Mega Drive slice instead of the default synthetic backend. Local mode only. |
| `--width <N>` | Frame width in cells. Default `DEFAULT_WIDTH`. |
| `--height <N>` | Frame height in lines. Default `DEFAULT_HEIGHT`. |
| `--ansi` | Emit ANSI frames (SGR emphasis only). |
| `--html <OUT>` | Write the HTML document to `OUT` when the `html` command runs. |
| `--movie <FILE>` | Read-only `.cfmovie` used by the timeline panel. |
| `--fps <N>` | Presentation cadence. Default `DEFAULT_FPS`. |
| `--help`, `-h` | Usage. |

Modes: **remote** composes `RemoteChannel` over the M6 typed client; **local** composes a
backend factory (synthetic by default, `--rom` for the Mega Drive slice) into an in-process
`AppService` with no store and no artifacts, and talks to it through `LocalChannel` with an
in-process admin principal. Local mode starts with **zero sessions**, so `create` is
required before `attach`. No token file is read in local mode.

**Line protocol.** After startup the process reads one command per stdin line, tokenized on
ASCII whitespace; empty lines are ignored; leading and trailing whitespace is ignored. The
lines are parsed and executed by `chronofork_gui::command::CommandSession::execute`
([gui.md §6.1](../architecture/gui.md#61-command-session-contract)); the process surface only
feeds lines, renders the returned typed outcome and maps `GuiError` to stderr and the
exit code. The grammar is exactly:

| Command | Arguments | Class | Effect |
|---|---|---|---|
| `create` | none | observation (creates) | `session.create {}`; prints the `create` block |
| `list` | none | observation | `session.list {}`; prints the `list` block |
| `attach` | `<SESSION>` | observation | `session.inspect` plus one bounded `session.read_memory` when the machine exposes memory; attaches and focuses a pane |
| `detach` | none | observation | detaches the focused pane |
| `focus` | `<PANE>` | observation | focuses an attached pane |
| `layout` | `<single\|mosaic>` | observation | sets the layout |
| `panel` | `<NAME> <on\|off>` | observation | sets panel visibility |
| `step` | `[<N>]` | **control** | `session.run {session, steps:N}` (default `N=1`, neutral input) |
| `run` | `<N>` | **control** | `session.run {session, steps:N}` |
| `pulse` | `<FRAMES>` | **control** | advances `FRAMES` presentation frames through the pacer |
| `pause` | none | observation | toggles the presentation pause flag (never a canonical pause); while set, the host-time loop advances zero frames and `pulse` is unaffected |
| `handoff` | `<human\|agent>` | transition | transfers the controller lease |
| `frame` | none | observation | prints the current frame grid |
| `html` | none | observation | prints (or writes, with `--html`) the HTML document |
| `quit` | none | observation | prints the `quit` block and exits 0; EOF behaves the same with `reason=eof` |

Panel names are exactly `status`, `lineage`, `capabilities`, `registers`, `memory`,
`inspectors`, `timeline`, `events`; an unknown name is a typed `unknown_panel`.

**Output contract.**

1. Exactly one block per command, except `frame` (a grid) and `html` without `--html` (a
   document). A block is `key=value` lines, first line `record=<name>`, values printable
   ASCII without spaces, blocks separated by exactly one blank line, no trailing blank
   line. Field order within a block is fixed by the tables in
   [gui.md §6](../architecture/gui.md#6-cli-composition) and in the acceptance rows.
2. A frame grid is exactly `height` lines of exactly `width` printable ASCII columns
   (with `--ansi`, SGR sequences may be inserted; stripping them yields exactly `width`
   columns), each line `\n`-terminated.
3. stdout carries data only (blocks, grids, documents). stderr carries diagnostics only.
4. **Exit 0** on `quit` or EOF after a clean run. **Exit 1** on any failure. A failure
   prints exactly one stderr line, and stdout stays empty for the failing step: the
   canonical `chronofork.error.v1` envelope for an **operation** failure, otherwise
   `chronofork: gui: <code>: <detail>` — one shape for flag, startup and per-command
   protocol failures alike. `<code>` is one of `unknown_command`, `bad_arity`, `bad_value`
   (a flag, its value or a command argument is invalid, missing or repeated),
   `not_attached`, `unknown_pane`, `unknown_panel`, `control_denied`, `limit_exceeded`,
   `movie`, `channel`. A failing command emits no block for that line and the process stops
   (fail-closed, the same rule as `chronofork tas studio`).
5. Bounds: `step`/`run` `N` is bounded by the service's `max_steps_per_request`; `pulse`
   `FRAMES` by `MAX_PULSE_FRAMES`; `--width`/`--height`/`--fps` by their documented
   ranges; an out-of-range value is `bad_value` (or `limit_exceeded` where the bound is a
   resource limit). The numeric constants are listed once in
   [gui.md §7](../architecture/gui.md#7-limits).

### 3.10 Limits, non-goals and follow-ups

Limits stated honestly:

- **No rendering and no audio.** Both backends report `rendering=false` and `audio=false`,
  and v1 has no pixel or audio device, so the GUI reports both as `Unavailable`
  (`CapabilityDisabled` or `NoOutputDevice`) and never as available. There is no picture and
  no sound.
- **No native toolkit, no browser frontend, no streaming.** The frontends are the terminal
  and a static HTML document (§3.4).
- **No canonical ownership and no GUI-side mutation beyond `create`/`run`.** Fork,
  snapshot, restore, close, compare, store and experiment operations stay CLI/agent
  surfaces.
- **The controller lease is cooperative** and window-scoped (§3.8); it is not a lock.
- **No GUI input injection.** v1 sends neutral input frames; controller/input editing is
  not part of this surface.
- **No GUI state persistence.** No settings, layout or history file; the process is
  stateless across runs.
- **No debug/trace/script/TAS data over transport**, therefore no VDP/bus/disassembly
  inspector content (**M6-FU1**).

Follow-ups (each a new task, never an implied permission): native toolkit frontend after
H2/H3; real video/audio sources and sinks (H2/H3); VDP/bus/disassembly inspector content
when M6-FU1 exposes it; GUI input injection; multi-window/multi-display; GUI trace viewers
(Stage 7 scope, blocked on the same transport gap). No local commit, merge, push or release
is authorized by this ADR; M7 is verified on its worktree and remains integration-pending.

## 4. Alternatives considered and rejected

| Alternative | Why rejected |
|---|---|
| Put the GUI in `chronofork-cli` | Presentation model, renderers, pacing, lease and the command grammar would only be testable through a spawned process, the layer limits could not be checked by `cargo tree`, and the CLI would grow a second, untested presentation implementation. |
| Put the GUI in `chronofork-service` | The service must stay free of UI policy and of any type that is not an operation; a frame grid is not an application operation. |
| Let the GUI depend on `chronofork-sessions` or a backend | It would make "the GUI never owns canonical state" a convention instead of a dependency-graph property, and it would let a frontend reach a backend directly and bypass the M6 validation/auth/ownership boundary. |
| Give the GUI its own HTTP/MCP code | Duplicates framing and the byte codec, and would make `chronofork-transport` non-authoritative. The channel trait keeps exactly one wire implementation. |
| Have the GUI call `session.snapshot`/`session.restore` for panels | Panels are views; capturing checkpoints from a window would mutate canonical state for presentation reasons and create a second writer path. |
| A ratatui/readline TUI | Already rejected in [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md): new dependency, terminal assumptions, non-deterministic rendering, hard to test in CI. |
| Emit ANSI-only output (no plain renderer) | The plain renderer is what makes frames comparable and reviewable in evidence; ANSI is an emphasis layer on top of it, and the equality of the two is an assertion. |
| Render HTML with a template engine or JS | A template dependency for a fixed document, plus a script surface and escaping risk; a hand-built, fully escaped document is smaller and testable. |
| Drive the GUI from the host clock in the CLI | stdout would depend on wall-clock scheduling, breaking the byte-comparison acceptance model and the repository rule that host time never influences observable deterministic output. |
| A GUI-owned session registry or state cache | A second source of truth; the session list is a view of `session.list`, refreshed on demand. |
| Advertise a "rendering pipeline ready" capability | `rendering=false` is a hard statement; the pipeline is capability-gated, and claiming otherwise is exactly the slop the policy forbids. |

## 5. Consequences

### Positive

- The M7 exit criteria become structural: attach/detach cannot perturb results (read-only
  views), presentation cannot own state (dependency graph), and handoff is observable and
  deterministic (lease counters + event log).
- The GUI is verifiable in CI with no display, no toolkit and no timing dependence; every
  acceptance row is a library or process command.
- The capability-gated pipeline lands now, so H2/H3 plug in without reopening this
  contract, and the honest `Unavailable` path is the one that ships.
- No new dependency, no `unsafe`, no new transport, no new service operation: M6 stays a
  frozen baseline.
- One channel seam plus one command executor make the local and remote modes share every
  grammar and presentation rule, which is what makes "remote observation" a parity claim
  rather than a separate code path.

### Negative and limits (stated honestly)

- v1 presents **text and HTML only**: no window, no picture, no sound. A reader expecting
  a graphical emulator frontend is not served by this increment, and the ADR says so.
- The lease is cooperative and window-scoped; it is not an authorization mechanism and
  cannot stop another process holding a token.
- The process surface is stdin-driven, so it is scriptable and deterministic but not an
  interactive full-screen application; line editing and live key handling are absent.
- Panels show less than a debugger could: no VDP, bus, disassembly or trace content until
  M6-FU1 exposes it over transport.
- `pause` is presentation state only: it suspends the library host-time loop and does not
  gate `pulse`, and the M6 catalog has no pause operation, so the GUI never claims to pause
  a canonical session.
- Frames are fixed-size ASCII grids; long values are clipped, not wrapped, so a narrow
  frame shows less detail rather than reflowing.

## 6. Security considerations

- **Token material stays in the CLI.** The GUI crate cannot name a token, a URL or a
  socket; the token is read from a file in remote mode, never echoed, and never placed in
  a block, frame or HTML document.
- **HTML is fully escaped and script-free.** The five HTML metacharacters are escaped,
  non-printable and non-ASCII bytes are replaced, there is no `<script>`, no event-handler
  attribute, no external URL, no `<iframe>` and no CSS `url(...)`; the document size is
  bounded. Escaping is proven with an adversarial backend descriptor supplied by a scripted
  channel: only `backend.id()` has a rendering path by contract, the identifier reaches the
  document escaped exactly once, and a second test proves that any grid content carrying
  the name/version payloads is escaped too.
- **No new attack surface.** The crate opens no socket, file, environment variable or
  process; the CLI connects only to an explicit `--remote` URL and writes only the explicit
  `--html` path.
- **Error text is bounded and path-free**, following the M6 rule: service envelopes are
  rendered verbatim, local diagnostics never include a token, ROM bytes, host path or
  backend payload.
- **The hard boundary is unchanged**: scopes, ownership and writer leases live in
  `chronofork-service`; the GUI's controller lease adds no authorization claim.
- **The movie is read-only** and bounded by the existing movie loader; no GUI command
  writes, moves or deletes a file except the explicit `--html` output.

## 7. Verification and evidence

- [M7 acceptance plan](../quality/m7-gui-acceptance.md) rows `M7-A1`–`M7-A30` are
  `executed - passed` with exact commands, observed values and evidence paths; the plan's
  verification history records the round-1 FAIL (F1–F4), the FAIL3 round (M7-A24 as
  written) and the repairs, and the promotion basis is the final independent **PASS4**
  (2026-11-17).
- Final independent review: `.agents/notes/m7/review/REVIEW5.md` — **PASS**, N1–N7/A24/L2
  confirmed and no regression. Verifier record:
  `.agents/notes/m7/verify/final2/PASS4.md` (ignored evidence).
- Repository gates and the dated [2026-11-17 M7
  block](../quality/quality-gates.md#2026-11-17--m7-human-facing-gui-independent-final-verification-pass4)
  in [quality gates](../quality/quality-gates.md): `./scripts/check.sh` exit 0, both demos
  3/3 byte-identical and the frozen M6 crates unchanged. The dated block carries the
  target/passed counts and the baseline comparison — the single source of those volatile
  numbers. The block is written by the documentation promotion task, not by this ADR.
- Architecture and limits: [GUI architecture](../architecture/gui.md).
- Baseline preserved: the M6 gate and both demo SHA-256 values, recorded in the captain
  provenance note and in the dated M6 block of [quality gates](../quality/quality-gates.md);
  M7 changed no canonical byte. Volatile counts belong to the dated block, not to this ADR.
- Requirements: FR-GUI-001 is **Implemented** for the documented text/HTML presentation
  scope and FR-EXEC-007 is **Implemented** for the verified library pacing/run loop plus
  rewind in [functional requirements](../requirements/functional.md).
