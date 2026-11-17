# GUI Architecture

This page is the architecture contract for
[ADR-0013](../adr/0013-human-facing-gui.md): the crate split, the view/model/render
pipeline, the attach/detach and run-loop sequences, the CLI composition, the handoff state
machine and the limits. It is how the presentation layer is built; the
[M7 acceptance plan](../quality/m7-gui-acceptance.md) is how that layer is proven, and the
**normative command grammar and process output contract** live in
[ADR-0013 §3.9](../adr/0013-human-facing-gui.md#39-the-chronofork-gui-run-process-surface-normative).
This page adds the type-level mapping, the block field order and the numeric bounds.

- **Status:** **Implemented and independently verified** (2026-11-17). The final independent
  verification PASS4 executed all 30 acceptance rows on the stable uncommitted worktree
  `agent/m7-gui` at base `93d47ed` and the final independent review REVIEW5 returned PASS;
  the dated result is the [2026-11-17 M7 block](../quality/quality-gates.md#2026-11-17--m7-human-facing-gui-independent-final-verification-pass4)
  in [quality gates](../quality/quality-gates.md), the executed rows are in the [M7
  acceptance plan](../quality/m7-gui-acceptance.md), and the milestone is `Complete with
  integration pending` (no commit, merge or publication was authorized). Evidence:
  `.agents/notes/m7/verify/final2/PASS4.md` and `.agents/notes/m7/review/REVIEW5.md`
  (ignored). The decision and normative grammar are in
  [ADR-0013](../adr/0013-human-facing-gui.md).

## 1. Layers and modules

```text
                     chronofork-cli  (composition root)
        ┌──────────────┬─────────────┴──────────────┐
        │              │                            │
 chronofork-transport  │                    chronofork-sessions
 (HttpClient)          │                    (movie loader, bounded read-only)
        │              │                            │
        ▼              ▼                            │
   RemoteChannel    LocalChannel                   │
        │              │                            │
        └──────┬───────┘                            │
               ▼                                    │
        chronofork-gui  (presentation only)         │
   command · channel · view · model · render ·      │
   pacing · presentation · control                  │
               │                                    │
               ▼                                    │
        chronofork-service  (canonical JSON, codec, limits, errors)
               │                                    │
               ▼                                    │
        chronofork-core  (digests)  ◄────────────────┘
```

| Crate | Responsibility | Must not contain |
|---|---|---|
| `chronofork-gui` | Command grammar and executor, immutable views, window/pane/mosaic model, panels, renderers, pacing/run loop, capability-gated presentation, controller lease | Sockets, HTTP/MCP types, tokens, files, environment, host clock (except through `HostClock`), canonical state, session/backend handles, a second JSON codec |
| `chronofork-cli` | Flags, token file, principal, local/remote channel composition, movie loading, stdin/stdout/stderr, `--html` write, block text formatting, exit codes | Presentation rules, a second renderer, a second codec, a second command parser |

Dependency rule: `chronofork-gui` depends on `chronofork-core` and `chronofork-service`
only. It **never** depends on `chronofork-transport`, `chronofork-sessions`,
`chronofork-backend-api` or a concrete backend; nothing depends on `chronofork-gui` except
the CLI. `cargo tree -p chronofork-gui` and a grep for `chronofork_transport` /
`chronofork_sessions` / `std::net` / `TcpStream` / `std::fs` / `std::env` in
`crates/chronofork-gui` are the executable checks (M7-A1).

Modules (one responsibility each):

| Module | Contents |
|---|---|
| `lib.rs` | Crate documentation, module map, the numeric limits (§7), re-exports, and the crate's single typed error `GuiError` with its closed `GuiErrorCode` set |
| `command.rs` | `CommandSession`, the parser/executor of the normative grammar ([ADR-0013 §3.9](../adr/0013-human-facing-gui.md#39-the-chronofork-gui-run-process-surface-normative)), `CommandOutcome`, `Block`, `BlockField`, `Record`, `CommandConfig`, `QuitReason` |
| `channel.rs` | `OperationChannel`, `ChannelFault`, and the canonical request/response helpers `request_bytes`, `parse_response`, `result_for`, `unsigned_input` |
| `view.rs` | Immutable view values and their strict decoders (`CapabilityView`, `SessionList`, `SessionSummary`, `SessionView`, `MachineView`, `MemoryView`, `CreateView`, `RunView`, `TimelineView`, `TimelineAnchor`) plus the bounded presentation value types and contracts (`PresentationFrame`, `AudioChunk`, `VideoSource`, `AudioSink`) |
| `model.rs` | `GuiModel`, `WindowId`, `PaneId`, `Pane`, `Layout`, `PanelKind`, `PanelState`, `Mosaic`, `Rect`, `SplitAxis`, `AttachReport`, `DetachReport`, `EventLog`, and the read-only attach/create sequences |
| `render/mod.rs`, `render/text.rs`, `render/html.rs` | `Frame` (the validated plain grid), `RenderOptions`, the plain/ANSI text renderer and the self-contained HTML renderer |
| `pacing.rs` | `HostClock`, `SystemClock`, `ManualClock`, `Pacer`, `PacerConfig`, `PulseReport`, `ElapsedReport` |
| `presentation.rs` | `Presentation<T>` (`Available`/`Unavailable`), the `Unavailable` reasons and the capability-gated video/audio pipeline |
| `control.rs` | `Controller`, `Lease` and the human/agent handoff state machine |

Panels, panes, layouts and the video/audio seams live in the modules above; the crate has
no other source files, so a new file is a new responsibility, not a rename of an existing
one.

Every public type has a `Debug` implementation (`missing_debug_implementations` is a
workspace warning); `expect_used` stays denied in library paths and is allowed only in
tests, exactly as in `chronofork-service`.

## 2. The view layer: canonical operation → immutable view

The channel is the only way out, and it exchanges **canonical bytes**:

```rust
pub trait OperationChannel: Send + Sync {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault>;
}

pub struct ChannelFault {
    /// One bounded, path-free, token-free line for the CLI diagnostic.
    pub detail: String,
    /// Canonical `chronofork.error.v1` bytes when the peer produced them.
    pub envelope: Option<Vec<u8>>,
}
```

Requests are encoded with `chronofork_service::json::to_canonical_bytes` and responses are
parsed with `chronofork_service::json::parse`, both under `ServiceLimits::default()`. The
service codec is therefore the only JSON implementation in the path, and the canonical
envelope rules are unchanged.

Decoding is strict and field-exact: a missing, mistyped or unknown field in a response is a
typed `GuiError::Protocol` (the response is not the documented operation result), never a
default value. The mapping is:

| View | Source operation | Fields consumed |
|---|---|---|
| `CapabilityView` | `capabilities` | `service_version`, `backend.{id,display_name,version,test_backend,capabilities.*}`, `limits.*` |
| `SessionList` / `SessionSummary` | `session.list` | `truncated`; `sessions[].{session,state,tick,state_hash,lineage}` |
| `SessionView` | `session.inspect` | `session`, `state`, `lineage`, `backend.*`, `machine.{tick,state_hash,halted,registers,memory_base,memory_length}` |
| `MemoryView` | `session.read_memory` | `session`, `address`, `length`, `bytes` (hex) |
| `CreateView` | `session.create` | `session`, `backend_id`, `state`, `tick`, `state_hash`, `lineage` |
| `RunView` | `session.run` | `session`, `steps`, `tick`, `state_hash`, `halted` |
| `TimelineView` / `TimelineAnchor` | CLI-provided value | `format_version`, `steps`, `end_tick`, `anchors[].{step,tick}` |

Two rules are normative:

1. **`session.create` yields `backend_id` only.** It returns `session`, `state`, `tick`,
   `state_hash`, `lineage` and `backend_id`; the descriptor and the capability flags come
   from `session.inspect` (per session) or `capabilities` (service level). The GUI never
   reads a field an operation does not return, and never synthesizes a descriptor from a
   backend id.
2. **The per-session descriptor gates that session's presentation.** `rendering` and
   `audio` from `session.inspect` are the gate: a `false` flag yields
   `Unavailable{CapabilityDisabled}` and the source/sink is never called; a `true` flag
   yields `Available` only when a source or sink is configured, and v1 configures none, so
   the process reports `Unavailable{NoOutputDevice}` (§6). The startup `capabilities` call
   gates the window-level pipeline and fills the `capabilities` panel; a disagreement
   between the two is reported as observed and not interpreted.

Views are plain values: integers, strings and `Vec`s. They contain no handle, no lifetime
and no reference into a service or a backend, so a view cannot outlive or alias canonical
state.

## 3. The model: panes, layout, panels, event log

`GuiModel` (one per window, `GuiModel::new(WindowId)`) owns presentation state only. The
lease and the pacer are **not** model fields: `control::Lease` (which carries the
`Controller`, the `generation` and the `sequence`) and `pacing::Pacer` are injected into the
command session (§6.1) and passed to the command engine and the renderer, so the model
cannot advance a canonical tick or authorize control by itself.

- Panes are keyed by `PaneId` (`1..=MAX_PANES`) and hold the attached session's views plus a
  per-attachment `generation`. `Layout` is `single` or `mosaic`; `Mosaic` splits the frame
  `Rect` into up to four regions (`Rect`, `SplitAxis`, `Mosaic::quadrants`).
- Panels are a `BTreeMap<PanelKind, PanelState>`. `PanelKind` is exactly the eight
  documented names — `status`, `lineage`, `capabilities`, `registers`, `memory`,
  `inspectors`, `timeline`, `events` — with no video or audio panel: presentation
  availability is reported on the frame status line, not as a panel. `PanelState` carries
  **visibility only**: the v1 renderers draw one clipped line per visible panel and the
  process grammar has no scroll command, so a scroll offset would be state no surface can
  set or read and is deliberately absent. Panels are seeded from the documented defaults
  (`status`, `lineage`, `capabilities`, `registers`, `events` visible; `memory`,
  `inspectors`, `timeline` hidden).
- `frame_index`, `paused` and the bounded `EventLog` complete the model.

The model is also where the two read/create sequences live, because they are view
transitions rather than command syntax: `GuiModel::create` issues `session.create` over the
injected `OperationChannel`, and `GuiModel::attach` issues `session.inspect` plus one
bounded `session.read_memory` (§5.1). They are the **only** channel operations in the model;
the command engine calls them and must not reimplement `session.inspect` or
`session.read_memory` itself. The remaining transitions are `detach`, `focus`, `set_layout`,
`set_panel`, `apply_run` (folding a `RunView` into the pane), `advance_frames` and
`toggle_pause`; `AttachReport`/`DetachReport` are the typed results the command engine turns
into blocks.

`TimelineView` validates canonical integer ranges and strictly increasing anchors, but it
imposes **no anchor-count limit of its own**: the CLI's bounded `.cfmovie` loader is the
single source of that bound.

`EventLog` is bounded by `EVENT_LOG_CAPACITY`: `Event` entries (`EventKind`, sequence, pane,
session) have a monotonic sequence and oldest-first eviction plus a `dropped` counter. It
records presentation events (`create`, `attach`, `detach`, `focus`, `layout`, `panel`,
`step`, `pulse`, `pause`, `handoff`) and never host time, paths or tokens. The `events` panel
is not backed by a canonical view, so unlike the six canonical panels it stays
`available=true` with no attached pane. Its empty form `events 0 last=none sequence=none` is
the **formatter's fallback**, not a state a rendered frame reaches: a pane exists only after
an attach, and an attach always records at least the `attach` event, so any frame that draws
a pane carries a non-empty log. The fallback is covered by a direct unit test in
`render/text.rs` (render-engineer task), not by a frame comparison.

Invariants the model must maintain (each is asserted by a unit test):

1. A pane holds a view for exactly the session it names; a detach clears the slot.
2. Focus is either `None` or an attached pane; a detach moves focus to the lowest attached
   pane id, or to `None`.
3. `generation` increments on every effective attach of a pane and never decreases; an
   idempotent re-attach of the same session reports `reused=true` and does not increment it.
4. `frame_index` increments by exactly the number of presentation frames advanced.
5. No model field is derived from a host clock; the only time value is the pacer's report.
6. The event log length never exceeds `EVENT_LOG_CAPACITY`.

## 4. Render pipeline: service → view → model → render

```text
chronofork-cli: one stdin line
        │
        ▼
chronofork_gui::command::CommandSession::execute(line)      ← grammar (ADR-0013 §3.9)
        │
        ├── parse ──► Err(GuiError) ──► CLI: one stderr line + exit 1
        │
        ├── control gate (Lease) ──► control_denied (typed, generation/sequence)
        │
        ├── channel.call(canonical request bytes)   ← create/attach run inside GuiModel
        │        │
        │        ▼  canonical response bytes (or a ChannelFault)
        │   view decoder (strict, field-exact)
        │        │
        │        ▼
        │   GuiModel (copies only) · Pacer (injected clock) · EventLog
        │
        ▼
CommandOutcome::Block | Grid | Html { document, block }
        │
        ▼
chronofork-cli: formats `record=`/`key=value` lines, prints the grid or the document,
               writes `--html`, maps errors to stderr and the exit code
```

The parser, the control gate, the operation set and the model updates all live inside
`command.rs`; the CLI never inspects a view, a pane or a lease. That is what makes the
"exact operation set" and the attach/detach flow testable as library tests
(`cargo test -p chronofork-gui --test command`) instead of only through a spawned process.

Frame structure (both renderers, same grid):

| Row | Content |
|---|---|
| 1 | `chronofork gui layout=<single\|mosaic> controller=<human\|agent> frame=<n> fps=<fps>` |
| 2 | `-` repeated `width` times |
| 3..=`height-1` | Pane regions (single: one region; mosaic: 2×2, row-major, empty regions blank) |
| `height` | `panes=<n> focused=<pane\|none> tick=<tick> state=<state> rendering=<available\|unavailable> audio=<available\|unavailable>` |

A pane region's first line is `pane <id> session <id> tick <tick> state <state>` plus
` focused` when focused; the remaining region lines are the visible panels in the fixed
order `status`, `lineage`, `capabilities`, `registers`, `memory`, `inspectors`, `timeline`,
`events`, one line each, clipped to the region width:

| Panel line | Form |
|---|---|
| `status` | `status tick=<t> state=<s> halted=<bool> controller=<c> generation=<n> sequence=<n>` |
| `lineage` | `lineage parent=<int\|null> origin=<int\|null> depth=<n>` |
| `capabilities` | `capabilities <backend-id> <flag>=<bool>` for each capability field in `Capabilities` order; the **identifier** is the only descriptor text with a rendering path — `display_name` and `version` never reach the frame (showing them would be a new CLI surface and contract) |
| `registers` | `registers <name>=<value> ...` in the inspection's sorted order |
| `memory` | `memory base=<hex> length=<n> <hex bytes>` |
| `inspectors` | `inspectors cpu=available vdp=unavailable:not_exposed_by_transport ...` |
| `timeline` | `timeline version=<n> steps=<n> end_tick=<t> anchors=<n>` or `timeline unavailable reason=<reason>` |
| `events` | `events <n> last=<event\|none> sequence=<n\|none>` |
| any unavailable panel | `<name> unavailable reason=<reason>` |

Renderer rules (asserted, not promised):

1. Exactly `height` lines; each line exactly `width` printable ASCII columns, space-padded,
   never wrapped, clipped at the region edge.
2. Only printable ASCII and `\n` are emitted; any other byte from a view is rendered as
   `?`. There is no locale, no width table and no Unicode layout.
3. No host time, path, token or environment value can appear: none reaches the model.
4. ANSI mode inserts SGR emphasis only — `ESC[1m` for the title and pane headers with
   `ESC[0m` after — and never cursor movement, clearing or alternate-screen sequences.
   Removing SGR from an ANSI frame yields the plain frame byte for byte.
5. HTML mode renders the same frame grids inside `<pre>` blocks, with the five
   metacharacters escaped and non-printable/non-ASCII bytes replaced; the document has no
   `<script>`, no event-handler attribute, no external URL, no `<iframe>`, no CSS
   `url(...)`, and its size is bounded by `MAX_HTML_BYTES`.
6. A byte-for-byte frame comparison across an attach/detach is a comparison of the
   canonical panels: the presentation `events` log line changes by design, so such a
   comparison hides the panel (`panel events off`) or masks that line. Nothing else in the
   frame may differ for an unchanged session.

## 5. Attach/detach and the run loop

### 5.1 Attach sequence (read-only by construction)

```text
attach 3   (executed by GuiModel::attach)
  1. channel.call({"op":"session.inspect","input":{"session":3}})
  2. decode SessionView (state, lineage, backend, machine)
  3. if machine.memory_length > 0:
       channel.call({"op":"session.read_memory",
                     "input":{"session":3,"address":machine.memory_base,
                              "length":min(machine.memory_length, MAX_MEMORY_PANEL_BYTES)}})
  4. allocate the lowest free pane id (or reuse the pane already holding session 3)
  5. generation += 1 (only on a new attachment); focused = pane
  6. append an `attach` event; render on demand
```

Detach performs no call at all: it clears the slot, moves focus and appends an event. That
is why attach/detach cannot change emulation results — there is no mutating operation in
the sequence. Re-attaching after a detach repeats steps 1–3 and must produce the **same
canonical panel content and the same tick/state** for an unchanged session. The
presentation `events` panel is the only line that changes by design (it logs the
attach/detach sequence), so a byte-for-byte frame comparison hides it with
`panel events off` or masks that line. Both sequences are executed by `command.rs`; the CLI
only supplies the `attach`/`detach` line.

### 5.2 Run loop and pacing

```text
frame_ns      = 1_000_000_000 / fps              (integer division)
elapsed path: due      = (now_nanos - last_nanos) / frame_ns
              advanced = min(due, max_catch_up_frames)
              dropped  = due - advanced
              last_nanos += advanced * frame_ns
pulse path:   advanced = frames (exactly); dropped = 0
              last_nanos += frames * frame_ns; elapsed_nanos = last_nanos
```

Each advanced frame performs `ticks_per_frame` canonical steps through one
`session.run {session, steps:ticks_per_frame}` call on the focused pane (neutral input),
then increments `frame_index`. `pulse` is the explicit path: it never drops, never advances
a partial frame and never depends on the clock value, which is what makes the process bytes
reproducible. `advance_elapsed` is the host-time run loop required by
[FR-EXEC-007](../requirements/functional.md): it is driven by `ManualClock` in unit tests
at chosen instants (catch-up cap and drop accounting included), `SystemClock` is the
library's real-clock implementation, and `CommandSession::advance_elapsed` (§6.1) is its
public entry point. No v1 CLI process path builds `SystemClock` or calls `advance_elapsed` —
the CLI injects `ManualClock` and exposes only the deterministic `pulse` — so live
host-time playback is a follow-up. Host time is never written to the model, to a block or to
a frame.

`pause` toggles the presentation pause flag only. It is not a canonical pause: the M6
catalog has no pause operation, and the GUI never claims one. While the flag is set,
`CommandSession::advance_elapsed` pins the clock reading to `pacer.last_nanos()`, so `due`,
`advanced`, `dropped` and `steps` are all zero, no `session.run` is issued and the elapsed
time is not consumed; the next unpaused step sees that time as due, caps the advance at
`max_catch_up_frames` and reports the remainder as dropped (the §5.2 rule). The explicit
`pulse` path is **not** gated by the flag. The v1 CLI never auto-advances, so `pause`
reports the flag and gates the library loop. The behaviour is covered by
`pause_gates_the_host_time_loop_but_not_pulse` in `crates/chronofork-gui/tests/command.rs`.
`step`, `run` and `pulse` are dispatched by `command.rs`, which applies the control gate
before any channel call and advances the pacer; the CLI never calls `session.run` itself.

## 6. CLI composition

The CLI is the composition root; it owns flags, the token file, the principal, the movie
file, stdin/stdout/stderr, the `--html` write, the block text formatting and the exit code.
It does **not** parse the grammar or touch a view, pane, panel, pacer or lease: that is
`command.rs`.

### 6.1 Command session contract

```rust
/// The name of one block field.
///
/// `list` interpolates a canonical session id into its per-session field names
/// (`session.<id>.state`), which a `&'static str` cannot express without leaking memory,
/// so fixed names stay borrowed statics and only that one family is owned.
pub type FieldName = Cow<'static, str>;

/// Presentation configuration assembled from CLI flags (never a file or a token).
pub struct CommandConfig {
    pub width: u32,                     // --width, validated against MIN_WIDTH..=MAX_WIDTH
    pub height: u32,                    // --height, validated against MIN_HEIGHT..=MAX_HEIGHT
    pub render: RenderOptions,          // --ansi
    pub pacer: PacerConfig,             // --fps, ticks_per_frame, catch-up
    pub timeline: Option<TimelineView>, // --movie, decoded by the CLI
}

pub enum QuitReason { Line, Eof }

pub struct CommandSession { /* channel, clock, model, pacer, lease, config */ }

impl CommandSession {
    /// Validates the frame bounds, issues `capabilities` once, decodes it, and builds
    /// the model, the pacer and the lease.
    pub fn open(
        channel: Arc<dyn OperationChannel>,
        clock: Arc<dyn HostClock>,
        config: CommandConfig,
    ) -> Result<Self, GuiError>;

    /// Parses and executes one line. `Ok(None)` for an empty/whitespace-only line.
    pub fn execute(&mut self, line: &str) -> Result<Option<CommandOutcome>, GuiError>;

    /// The `quit` block for a `quit` line or for EOF (the CLI calls this at EOF).
    pub fn quit(&mut self, reason: QuitReason) -> Block;

    /// The host-time run-loop step required by FR-EXEC-007: advances the frames the
    /// injected clock says are due, capped by the pacer's catch-up bound, and reports
    /// the advanced/dropped counts. The deterministic `pulse` path never uses it.
    pub fn advance_elapsed(&mut self) -> Result<ElapsedReport, GuiError>;
}

pub enum CommandOutcome {
    /// Any block command, including `quit` — there is no separate quit variant.
    Block(Block),
    /// The serialized frame grid: `height` `\n`-terminated lines, no trailing blank
    /// line, already plain or ANSI per `RenderOptions`.
    Grid(String),
    /// The `html` command: the document plus the `html` block (`bytes`, `sha256`).
    Html { document: String, block: Block },
}

pub struct Block {
    pub record: Record,             // the `record=` name
    pub fields: Vec<BlockField>,    // the documented order below
}

pub enum Record { Create, List, Attach, Detach, Focus, Layout, Panel, Step, Run, Pulse,
                  Pause, Handoff, Html, Quit }

pub enum BlockField {
    Uint(FieldName, u64),
    Bool(FieldName, bool),
    Text(FieldName, String),            // already sanitized by the crate
    OptionalUint(FieldName, Option<u64>),
}
```

The crate's single typed error is `GuiError` (`lib.rs`), whose `code()` returns exactly one
`GuiErrorCode` of the closed set in
[ADR-0013 §3.9 item 4](../adr/0013-human-facing-gui.md#39-the-chronofork-gui-run-process-surface-normative);
there is no second command-error type. An operation failure is `GuiError::Channel`, which keeps
the canonical `chronofork.error.v1` bytes so the CLI can print them verbatim; a response that
violates the operation contract is `GuiError::Protocol`, reported under the same `channel`
code. Codec bounds are `codec_limits()` (`ServiceLimits::default()`), not a per-session
field, so the crate holds no limit number of its own.

`FieldName` is `Cow<'static, str>` because the `list` block generates one dynamic name family
per session (`session.<id>.state`, `.tick`, `.state_hash`, `.lineage_depth`), which a
`&'static str` cannot express without leaking memory; every other name is a borrowed static.
The CLI therefore has one name source — the vector order — and no second mapping table.

The CLI formats a block by writing `record=<name>` and then one `key=value` line per field
in the vector order; it never reorders, renames or interprets a field. Field values
serialize exactly as: `Uint` → decimal; `Bool` → `true` or `false`; `OptionalUint` → the
decimal value or `none` (the root `lineage_parent`/`lineage_origin_snapshot` are `none`);
`Text` → the string, which `command.rs` has already sanitized by substituting `_` for every
character outside `!`..=`~`, so no space, control or non-ASCII byte can break the line or
move a terminal — the block counterpart of the frame renderer's `?` rule. The CLI maps
`GuiError` to the single stderr line and exit 1.

`quit` and EOF both return `CommandOutcome::Block` with `record=quit` (the CLI calls
`quit(QuitReason::Eof)` at EOF); there is no separate quit outcome. The `Grid` outcome is
written to stdout verbatim (the trailing `\n` is part of the grid). The `html` outcome is
printed as the document, or written to `--html` and printed as its block. The crate also
exposes read-only `model()` and `lease()` accessors for its integration tests
(M7-A2/A4/A6/A16); they are not part of the process contract and no CLI code path depends on
them.

### 6.2 Startup and per-command execution

**Startup order:** parse and validate flags (`bad_value` on any invalid, missing or repeated
option) → (remote) read the bounded token file, select the first principal, build
`HttpClient` → (local) build the backend factory and the in-process `AppService` with an
admin principal → read `--movie` with the existing bounded loader into a `TimelineView`
(`movie` on failure) → `CommandSession::open(channel, ManualClock, config)`, which issues
`capabilities {}` once and decodes `CapabilityView` (`channel` on failure) → read stdin.
Every startup failure exits 1 with empty stdout and one `chronofork: gui: <code>: <detail>`
line.

| Command | Execution path inside `command.rs` | Operation(s) |
|---|---|---|
| `create` | parse → `GuiModel::create` (the model owns the call) | `session.create {}` |
| `list` | parse → channel | `session.list {}` |
| `attach` | parse → `GuiModel::attach` (the model owns both calls) | `session.inspect`, plus `session.read_memory` when the machine exposes memory |
| `detach`, `focus`, `layout`, `panel`, `pause` | parse → model only | none |
| `step`, `run` | parse → control gate → channel → model (`apply_run`) | `session.run {session, steps}` |
| `pulse` | parse → control gate → pacer → channel → model (`apply_run`) | `session.run {session, steps:ticks_per_frame}` per frame |
| `handoff` | parse → lease | none (lease transition) |
| `frame` | parse → renderer (model + lease) | none |
| `html` | parse → renderer (model + lease) → `Html { document, block }` | none |
| `quit` | parse → `quit(QuitReason::Line)` | none |

`create` and `attach` are the only rows where the channel call happens inside `GuiModel`;
the command engine never re-implements `session.inspect` or `session.read_memory`.

Block field order (the `record=` name is always line 1; values are printable ASCII without
spaces):

| Block | Fields in order |
|---|---|
| `create` | `session`, `backend_id`, `state`, `tick`, `state_hash`, `lineage_parent`, `lineage_origin_snapshot`, `lineage_depth` |
| `list` | `sessions`, `truncated`, then per session ascending `session.<id>.state`, `session.<id>.tick`, `session.<id>.state_hash`, `session.<id>.lineage_depth` |
| `attach` | `pane`, `session`, `generation`, `attached`, `focused`, `layout`, `panes`, `backend`, `backend_version`, `test_backend`, `rendering`, `audio`, `state`, `tick`, `state_hash`, `halted`, `lineage_parent`, `lineage_origin_snapshot`, `lineage_depth`, `memory_base`, `memory_length` |
| `detach` | `pane`, `session`, `generation`, `attached`, `focused`, `layout`, `panes` |
| `focus` | `pane`, `session`, `focused`, `layout`, `panes` |
| `layout` | `layout`, `panes`, `focused` |
| `panel` | `panel`, `visible`, `available`, `reason` |
| `step` / `run` | `pane`, `session`, `requested`, `steps`, `tick`, `state_hash`, `halted`, `controller` |
| `pulse` | `pane`, `session`, `frames`, `frame`, `tick`, `state_hash`, `halted`, `ticks_per_frame`, `fps`, `elapsed_nanos`, `controller` |
| `pause` | `paused` |
| `handoff` | `controller`, `previous`, `changed`, `generation`, `sequence` |
| `html` | `bytes`, `sha256` |
| `quit` | `reason`, `controller`, `generation`, `sequence` |

The `html` block is always built by `command.rs` from the exact document bytes; the CLI
prints the document (and no block) when `--html` is absent, or writes the document to the
path and prints the block when `--html` is present. `bytes` and `sha256` are over the exact
bytes written/printed.

Semantics of values that are easy to misread:

- `step`/`run`: `requested` is the requested step count, `steps` is the count the service
  actually performed (`session.run`'s `steps` field, which stops at a halt).
- `pulse`: `frames` is the requested frame count, `frame` is the model's frame index after
  the pulse, `elapsed_nanos` is the pacer's **cumulative** manual-clock time after the pulse
  (`last_nanos` in §5.2), so a first pulse from a fresh process equals
  `frames * (1_000_000_000 / fps)` and later pulses add to it.
- `attach`: `focused` is `true` after an attach; `panes` is the number of attached panes.
- `detach`: `focused` is the new focused pane id or `none`.
- `panel`: `available` reports whether the panel has data to show — a canonical view, or the
  presentation log for `events`, which needs no session; `reason` is `none` or one of
  `no_session`, `no_movie`, `not_exposed_by_transport`.
- Video/audio availability is reported by the frame status line
  (`rendering=<available|unavailable>`, `audio=<available|unavailable>`) and by the
  descriptor flags in the `attach` block. v1 configures no pixel or audio sink, so the
  process surface never reports either as available: `rendering=false`/`audio=false` yields
  `CapabilityDisabled`, and a descriptor that advertises the capability still yields
  `NoOutputDevice` (see [ADR-0013 §3.6](../adr/0013-human-facing-gui.md#36-video-and-audio-are-capability-gated-pipelines-never-fabricated-semantics)).
  The `Available` path is exercised by the crate's presentation tests with a test-pattern
  source and a recording sink.

## 7. Limits

| Constant | Value | Bound on |
|---|---|---|
| `MAX_PANES` | 4 | Attached sessions per window |
| `DEFAULT_WIDTH` / `MIN_WIDTH` / `MAX_WIDTH` | 80 / 40 / 200 | Frame columns |
| `DEFAULT_HEIGHT` / `MIN_HEIGHT` / `MAX_HEIGHT` | 24 / 10 / 100 | Frame lines |
| `MAX_MEMORY_PANEL_BYTES` | 256 | Bytes per memory panel refresh |
| `EVENT_LOG_CAPACITY` | 256 | GUI event log entries |
| `MAX_HTML_BYTES` | 1 048 576 | HTML document size |
| `MAX_PULSE_FRAMES` | 600 | Frames per `pulse` |
| `DEFAULT_FPS` / `MIN_FPS` / `MAX_FPS` | 60 / 1 / 240 | Presentation cadence |
| `DEFAULT_TICKS_PER_FRAME` / `MAX_TICKS_PER_FRAME` | 1 / 1000 | Canonical steps per presented frame |
| `DEFAULT_MAX_CATCH_UP_FRAMES` / `MAX_MAX_CATCH_UP_FRAMES` | 4 / 16 | Frames a late host may execute in one step |
| `MAX_PRESENTATION_PIXELS` | 1 048 576 | `PresentationFrame` pixels |
| `MAX_AUDIO_SAMPLES` | 65 536 | Samples per `AudioChunk` |

The `step`/`run` bound is not a new constant: it is the `max_steps_per_request` reported
by the startup `capabilities` view, so the GUI cannot accept a batch the service would
reject. Every bound is validated **before** the bounded work starts and
produces a typed error (`limit_exceeded` or `bad_value`), never a partial allocation.

## 8. Handoff state machine

```text
             handoff agent (changed=true, generation+1, sequence+1)
   ┌─────────┐ ─────────────────────────────────────────────► ┌─────────┐
   │  Human  │                                                 │  Agent  │
   │ (start) │ ◄───────────────────────────────────────────── │         │
   └─────────┘   handoff human (changed=true, generation+1,     └─────────┘
                 sequence+1)
        │                                                            │
        │ step | run | pulse                          step | run | pulse
        ▼                                                            ▼
   executes (session.run)                    control_denied (typed, exit 1,
                                             reports generation+sequence)
```

| From | Command | Result |
|---|---|---|
| `Human` | `handoff agent` | `controller=agent`, `previous=human`, `changed=true`, both counters +1 |
| `Agent` | `handoff human` | `controller=human`, `previous=agent`, `changed=true`, both counters +1 |
| `Human` | `handoff human` | `changed=false`, counters unchanged |
| `Agent` | `handoff agent` | `changed=false`, counters unchanged |
| `Human` | `step`/`run`/`pulse` | executed through the channel |
| `Agent` | `step`/`run`/`pulse` | `chronofork: gui: control_denied: ...`, exit 1, no block |
| any | observation commands | always executed |

The lease is window-scoped and **cooperative**. It does not lock the canonical session and
is not an authorization mechanism: a second process holding a valid token can always call
`session.run`, and the GUI observes the result on its next `session.inspect`. The hard
boundaries are the M6 ones — scopes, ownership and the per-session writer lease. The gate
itself is one function in `command.rs`: `step`/`run`/`pulse` consult `Lease` before any
channel call, and `handoff` is the only command that mutates it.

## 9. Security

- **The token exists only in the CLI.** The crate cannot name a token, a URL or a socket;
  the token file is read bounded, the first principal is selected, and the token is never
  echoed, logged or rendered. Local mode reads no token at all.
- **HTML is fully escaped and script-free.** Only `backend.id()` has a rendering path by
  contract (the `capabilities` panel line); a scripted channel supplies an adversarial
  descriptor whose identifier, display name and version carry distinct payloads
  (`<script>`, `"`, `'`, `&` and control bytes), and only the identifier reaches the
  document, escaped exactly once. A second test proves that any grid content carrying the
  other strings is escaped, so the document contains no executable construct or external
  reference.
- **No ambient capability.** No socket, file, environment, process or host clock read
  inside the crate; the CLI connects only to the explicit `--remote` URL and writes only
  the explicit `--html` path.
- **Bounded, path-free diagnostics.** Operation failures render the canonical
  `chronofork.error.v1` envelope verbatim; every other failure (flags, startup, protocol)
  renders one `chronofork: gui: <code>: <detail>` line with no token, ROM bytes, host path
  or backend payload.
- **Read-only inputs.** The `.cfmovie` is opened read-only with the existing bounded loader
  and never written; `--html` is the only write path.

## 10. Non-goals and follow-ups

Not in this increment: native toolkit windows, browser/JS frontend, streaming transport,
live audio output, GUI input injection, GUI state persistence, multi-window or
multi-display, fork/snapshot/restore/close/compare/store operations from the window,
VDP/bus/disassembly/trace panel content (**M6-FU1**), and any rendering or audio semantics
(**H2**/**H3**).

Follow-ups are separate tasks: native frontend after H2/H3; real `VideoSource`/`AudioSink`
implementations; live host-time playback (v1 exposes the deterministic `pulse`); inspector
content when M6-FU1 exposes debug operations; GUI input editing and settings. The
documentation promotion of this page is recorded in the [documentation
index](../README.md), [architecture overview](overview.md), [roadmap](../roadmap/roadmap.md),
[milestones](../roadmap/milestones.md), [functional requirements](../requirements/functional.md)
and [quality gates](../quality/quality-gates.md) (2026-11-17).

## 11. Verification

- [ADR-0013](../adr/0013-human-facing-gui.md) — the accepted decision and the normative
  process contract.
- [M7 acceptance plan](../quality/m7-gui-acceptance.md) — rows `M7-A1`–`M7-A30` are
  `executed - passed` (final PASS4, 2026-11-17), each with an exact command and evidence
  path; the verification history records the earlier FAIL/FAIL3 rounds and the repairs.
- The dated [2026-11-17 M7 block](../quality/quality-gates.md#2026-11-17--m7-human-facing-gui-independent-final-verification-pass4)
  in [quality gates](../quality/quality-gates.md) — `./scripts/check.sh` exit 0; the dated
  block carries the target/passed counts and the baseline comparison, and both demos are
  byte-identical to the M6 baseline with the frozen M6 crates unchanged.
- The final independent review, `.agents/notes/m7/review/REVIEW5.md` (PASS; N1–N7/A24/L2
  confirmed, no regression), and the verifier record
  `.agents/notes/m7/verify/final2/PASS4.md` under the ignored `.agents/notes/` tree.
- [Anti-slop policy](../quality/anti-slop-policy.md) and [testing
  strategy](../quality/testing-strategy.md) — the rules every row follows.
- Baseline preserved: the M6 gate and demo digests recorded in the captain provenance
  note; no canonical byte changed.
