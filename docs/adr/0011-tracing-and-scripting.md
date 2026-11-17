# ADR-0011: Tracing and Scripting — Host Trace Surface and Sandboxed Script Runtime

- **Status:** **Accepted** (owner-approved 2026-11-16 by the explicit Stage 5
  implementation directive; see "Approval evidence"). Acceptance authorizes design,
  implementation and independent verification under the [M5 acceptance
  plan](../quality/m5-tracing-and-scripting-acceptance.md). It does **not** authorize a
  commit, push, merge, rebase, tag, branch change or release.
- **Date:** 2026-11-16
- **Decision owner:** Project owner
- **Approval evidence:** the project owner's Stage 5 directive recorded on 2026-11-16:
  "`/agent-teams implemente completamente o stage 5 do roadmap`". The directive covers
  roadmap [Stage 5 — Tracing and scripting](../roadmap/roadmap.md#stage-5--tracing-and-scripting)
  and [M5 — Tracing and scripting](../roadmap/milestones.md#m5--tracing-and-scripting),
  with the scope, limits, deferred items and Git restrictions stated in this ADR and in
  the brief `.agents/notes/m5/BRIEF.md` (ignored). It neither approves publication nor
  converts implementation into verification.
- **Implementation status:** **Implemented and independently verified for the documented
  minimal-slice scope** (final independent T12 PASS, 2026-11-16, on branch
  `agent/m5-tracing-scripting` at HEAD/base
  `84f1476466c14e72f911689845c390173982fd81`; uncommitted, integration pending). Every row
  of the [M5 acceptance plan](../quality/m5-tracing-and-scripting-acceptance.md) is
  `executed - passed`. The earlier T7 verification returned `needs_revision` with findings
  F1–F10; T11 repaired them and T12 independently re-verified every row and fix. T10 then
  reviewed the integrated result and returned PASS. The authoritative dated gate block is
  in [quality gates](../quality/quality-gates.md). The deferred scope in §3.7 is unchanged.
- **Supersedes:** none. It extends [ADR-0002](0002-backend-boundaries.md) (backend
  contract and capabilities), [ADR-0003](0003-durable-session-storage.md) (session
  ownership), [ADR-0004](0004-tas-movie-format.md) and
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md) (movies),
  [ADR-0008](0008-rerecord-and-branch-editing.md) (branch artifacts) and
  [ADR-0010](0010-debugging-depth.md) (optional host debug observation). It changes no
  canonical snapshot byte, state hash, configuration hash, payload version, movie byte or
  previously verified behavior.
- **Related:** [tracing and scripting architecture](../architecture/tracing-and-scripting.md),
  [backend model](../architecture/backend-model.md),
  [session model](../architecture/session-model.md),
  [execution model](../architecture/execution-model.md), [Mega Drive
  slice](../architecture/megadrive-slice.md), [M5 acceptance
  plan](../quality/m5-tracing-and-scripting-acceptance.md), [quality
  gates](../quality/quality-gates.md), [anti-slop policy](../quality/anti-slop-policy.md),
  [functional requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [roadmap](../roadmap/roadmap.md), [research: TAS and
  debugging](../research/tas-and-debugging.md), [oracle differential
  evidence](../research/oracle-differential-evidence.md).

## 1. Context

Read on 2026-11-16 in the `m5-tracing-scripting` worktree, branch
`agent/m5-tracing-scripting`, based on `main` at
`84f1476466c14e72f911689845c390173982fd81`. The accepted, implemented and independently
verified M1–M4 surfaces below are the baseline; none of them is re-decided here.

- **M1 — deterministic execution.** [ADR-0002](0002-backend-boundaries.md) and the
  [backend model](../architecture/backend-model.md): `Backend` exposes `step`,
  `snapshot`/`restore`, side-effect-free `inspect` and `read_memory`, optional
  `debug_access`, virtual [ticks](../architecture/execution-model.md), typed input and
  canonical state hashes. The synthetic backend and the minimal Mega Drive slice
  implement this slice. `Capabilities::tracing` exists but is `false` for both backends;
  the Mega Drive slice has a concrete `megadrive trace` TSV ring with no timestamps,
  events or version.
- **M2 — sessions.** [ADR-0003](0003-durable-session-storage.md) and the [session
  model](../architecture/session-model.md): a `Session` owns exactly one
  `Box<dyn Backend>`; the session layer owns lifecycle, checkpoints, forks, durable
  storage and rehydration, and never exposes the backend. Debug observation is already
  non-canonical: it is absent from snapshots, state hashes, configuration hashes and
  payload versions, and `restore` does not rewind it.
- **M3 — movies and replay.** [ADR-0004](0004-tas-movie-format.md),
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md) and
  [ADR-0008](0008-rerecord-and-branch-editing.md): versioned `.cfmovie` containers,
  deterministic replay, exact first-divergence reporting, ranged replay, anchors,
  multi-device input and branch artifacts. This is the established pattern for a
  versioned format with typed rejection, bounded sizes and no silent reinterpretation.
- **M4 — bounded observation.** [ADR-0010](0010-debugging-depth.md): the optional
  `DebugAccess` contract with a bounded, contiguous-index `DebugEventLog`
  (`BusWrite`, `VdpPort`, `ControllerRead`, `InterruptRequested/Delivered/Acknowledged`,
  `Watchpoint`), the host `chronofork-debug` layer, and
  `Capabilities::debug_events` (`true` on the Mega Drive slice, `false` on the synthetic
  backend). Host breakpoints are implemented over `inspect`; the backend capability
  `breakpoints` stays `false`. The M4 verifier proved 1,700 per-step off/on
  non-perturbation comparisons and unchanged demos, movies and oracle evidence.
- **Requirements and roadmap (state at decision time).** At acceptance time
  `FR-TRACE-001` and `FR-TRACE-002` were **Partial**, `FR-SCRIPT-001` was **Planned**,
  `NFR-SEC-003` required sandboxing untrusted scripts, and
  [M5](../roadmap/milestones.md#m5--tracing-and-scripting) was `Next`. The roadmap scope
  named structured traces with emulated timestamps, triggered/circular capture,
  comparison and historical queries, a sandboxed script runtime with limits, script APIs
  for sessions/forks/inspection/TAS/experiments, and process-level CLI tests. The final
  verified state is recorded in the implementation-status bullet above and in the
  [milestone](../roadmap/milestones.md#m5--tracing-and-scripting).
- **Current gap.** There is no versioned trace format, no cross-backend capture, no
  trigger configuration, circular retention, query or trace comparison surface, and no
  script runtime. The existing Mega Drive TSV ring is backend-specific, event-free and
  debug-only; the seven `DebugEventKind` classes are observable but not persisted or
  queryable. Requirement text lists Rhai/Luau/WASM/Python SDKs as examples; no such
  dependency exists in the workspace, whose only non-dev external dependency is `sha2`
(`proptest` is a dev-dependency).

## 2. Problem

Stage 5 must add two products without weakening the properties that make ChronoFork
useful:

1. **Determinism and non-perturbation.** Trace capture, trigger configuration, script
   state and experiment reports must not change canonical state, state hashes,
   configuration hashes, snapshot payloads/versions or movie bytes. No host wall-clock,
   pid, path or entropy may enter trace bytes, comparison results or successful script
   results.
2. **Ownership and honest capabilities.** Trace capture is a host concern that works over
   any backend with `register_inspection` + `memory_read`; only bus/device/interrupt
   events require `debug_events`. The backend `tracing` capability must stay `false`, as
   with M4 host breakpoints, or the project would advertise a feature no backend
   guarantees.
3. **One canonical, versioned format.** Import must be reject-only: wrong magic/version,
   unknown record kinds, malformed indexes, out-of-bounds metadata and truncation are
   typed errors, never silently reinterpreted bytes. Export must be byte-deterministic.
4. **Bounded resources.** Every ring, table, string, file, step and query needs a
   documented bound and a typed rejection validated before allocation; arbitrary input
   must not panic, allocate unboundedly or lose records silently.
5. **Sandboxed scripting.** Scripts must run deterministic, reproducible experiments
   without host-language plugins, with explicit memory/time limits, typed errors and no
   ambient filesystem, network, process, clock or entropy access.
6. **Reachability and evidence.** Every implemented capability must be reachable from the
   CLI and covered by process-level tests with deterministic output and typed failures.
7. **No new supply-chain surface without review.** The dependency graph is deliberately
   minimal; embedding a host language or a WASM runtime is a major license, audit and
   determinism decision, not an implementation detail.

## 3. Decision

### 3.1 Host-provided tracing over the backend observation contract

1. **New crate `chronofork-trace`.** It depends only on `chronofork-core` and
   `chronofork-backend-api`; test-only dev-dependencies may use the two concrete backends
   and `proptest`. It owns the canonical trace format, capture driver, query, comparison
   and TSV export. `chronofork-sessions` may depend on it and exposes the session capture
   entry point because it owns the backend. `chronofork-script` may depend on core,
   backend-api, sessions and trace; it must not depend on a concrete backend, the CLI or a
   transport. The CLI is a thin adapter.
2. **No backend contract or capability change.** `Capabilities::tracing` stays `false`
   for both backends and for any future backend that merely exposes inspection/memory
   access. The host trace surface is a host layer feature and is reported as such, exactly
   as ADR-0010 reports host breakpoints separately from backend watchpoints. The backend
   model already states that host-implemented features must not be disguised as backend
   capabilities.
3. **Capture sources.** Instruction records come from `inspect` (the `pc` register) and a
   big-endian `memory_read` of the 2-byte opcode word at `pc`. Bus/device/interrupt
   records come only from `DebugAccess` events when `Capabilities::debug_events` is
   `true`; otherwise the trace is instruction-only and records `events_observed = false`.
   If a backend advertises `debug_events = true` but `debug_access()` returns `None`, that
   is a typed `EventObservationUnavailable` error, not a silent downgrade.
4. **PC convention.** The `pc` register is required and must fit in `u32`; an absent
   register is `ProgramCounterUnavailable`, a value that does not fit or an opcode read
   failure is a typed error. `pc` is the address of the instruction about to execute;
   the opcode is the 16-bit big-endian word at that address. Multi-word instructions
   produce one instruction record for the first word, not one per extension word.
5. **Record taxonomy.** v1 has exactly five record kinds: instruction, bus write, VDP
   port access, controller read and interrupt phase. There is **no** generic bus-read
   record: no backend currently observes generic data reads through `DebugAccess`, and a
   dead producer would be speculation. Controller reads are device records, not bus
   writes. `Watchpoint` events are host debugger matches and are never mapped into trace
   records.
6. **Non-canonical.** A trace, `TraceConfig`, trigger state, comparison result and TSV
   export are process-local observation artifacts. They never enter snapshots, state
   hashes, configuration hashes, payload versions or movie bytes; capture may install a
   backend `DebugConfig`, which M4 already proved non-canonical.

### 3.2 Canonical `chronofork-trace` v1 format

The public model is:

```rust
pub enum InterruptPhase { Requested, Delivered, Acknowledged }
pub enum TraceKind { Instruction, BusWrite, VdpPort, ControllerRead, Interrupt }

pub struct TraceKindMask(u8); // bits 0..=4 in TraceKind order; 0 = no records

pub enum TraceRecord {
    Instruction { index: u64, tick: Tick, pc: u32, opcode: u16 },
    BusWrite { index: u64, tick: Tick, address: u32, value: u32, width: u8 },
    VdpPort { index: u64, tick: Tick, offset: u8, value: u32, width: u8, write: bool },
    ControllerRead { index: u64, tick: Tick, port: u8, value: u8 },
    Interrupt { index: u64, tick: Tick, level: u8, phase: InterruptPhase },
}

pub struct TraceMeta {
    pub backend: String,          // 1..=128 UTF-8 bytes
    pub backend_version: String,  // 1..=64 UTF-8 bytes
    pub state_hash: StateHash,    // canonical state hash at capture start
    pub start_tick: Tick,
    pub end_tick: Tick,           // final observed tick, >= start_tick
    pub capacity: usize,          // 1..=MAX_TRACE_CAPACITY
    pub first_index: u64,         // == dropped
    pub dropped: u64,
    pub stop_reason: TraceStopReason,
    pub triggered: bool,
    pub events_observed: bool,
}

pub struct Trace { /* meta + Vec<TraceRecord> */ }
```

`TraceRecord` exposes `index()`, `tick()`, `kind()` and a deterministic one-line summary
`<kind> index=<n> tick=<n> <kind fields>` used by CLI query/compare; the field order
is the record's declared order (for example `instruction index=3 tick=3 pc=000200
opcode=7001`), and absent fields never appear. `Trace::new(meta, records) -> Result<Trace,
TraceError>` enforces every invariant; `Trace::from_bytes(&[u8]) -> Result<Trace,
TraceError>` is reject-only; `Trace::to_bytes() -> Vec<u8>` is infallible after
construction and byte-deterministic.

Constants:

| Constant | Value |
|---|---|
| `TRACE_FORMAT_VERSION` | `1` |
| `TRACE_MAGIC` | `b"CFTRACE1"` |
| `MAX_TRACE_BYTES` | 64 MiB |
| `MAX_TRACE_RECORDS` | 1,000,000 |
| `MAX_TRACE_CAPACITY` | 1,000,000 |
| `DEFAULT_TRACE_CAPACITY` | 4,096 |
| `MAX_BACKEND_ID_BYTES` | 128 |
| `MAX_BACKEND_VERSION_BYTES` | 64 |
| `MAX_CAPTURE_STEPS` | 1,000,000 |
| `DEFAULT_EVENT_CAPACITY` | 1,024 |
| `MAX_EVENT_CAPACITY` | 65,536 |
| `MAX_QUERY_LIMIT` | 10,000 |

#### 3.2.1 Binary layout

Big-endian, no padding, no alignment gaps, no trailing bytes. The file starts with the
8-byte magic `CFTRACE1`, followed by the header:

| Field | Width | Notes |
|---|---|---|
| `version` u16 | 2 | must be 1 |
| `flags` u16 | 2 | bit 0 `events_observed`, bit 1 `triggered`; all other bits zero |
| `stop_reason` u8 | 1 | 0 `StepLimit`, 1 `Halted`, 2 `StopCondition` |
| `record_count` u32 | 4 | 0..=1,000,000 and <= `capacity` |
| `capacity` u32 | 4 | 1..=1,000,000 |
| `first_index` u64 | 8 | must equal `dropped` |
| `dropped` u64 | 8 | evicted records |
| `start_tick` u64 | 8 | capture start tick |
| `end_tick` u64 | 8 | `>= start_tick` |
| `state_hash` | 32 | SHA-256 canonical state hash at capture start |
| `backend_len` u16 + UTF-8 | 2 + n | 1..=128 bytes, non-empty |
| `backend_version_len` u16 + UTF-8 | 2 + n | 1..=64 bytes, non-empty |

Each record starts with `kind` u8, `index` u64 and `tick` u64, then a kind-specific
payload:

| Kind code | Record | Payload after kind/index/tick |
|---|---|---|
| 0 | Instruction | `pc` u32, `opcode` u16 |
| 1 | BusWrite | `address` u32, `value` u32, `width` u8 (1..=4) |
| 2 | VdpPort | `offset` u8, `value` u32, `width` u8 (1..=2), `write` u8 (0/1) |
| 3 | ControllerRead | `port` u8 (1..=2), `value` u8 |
| 4 | Interrupt | `level` u8 (1..=7), `phase` u8 (0 Requested, 1 Delivered, 2 Acknowledged) |

#### 3.2.2 Construction invariants

`Trace::new` rejects, with a typed error and no partial construction: metadata strings
empty or over their byte bounds; invalid UTF-8 is impossible for `String` input but is
checked on parse; `capacity` outside `1..=MAX_TRACE_CAPACITY`; `record_count >
MAX_TRACE_RECORDS`; `record_count > capacity`; `end_tick < start_tick`;
`first_index != dropped`; a record index other than `first_index + position`; a tick
decreasing relative to the previous record; a record tick outside
`start_tick..=end_tick`; an out-of-range field value (width, port, level, phase). Empty
traces are valid when `first_index == dropped` and `end_tick >= start_tick`.

#### 3.2.3 Parse rules (reject-only)

`Trace::from_bytes` validates the entire byte string before exposing a `Trace` and
rejects: wrong magic; `version != 1`; unknown flag bits; `stop_reason > 2`; a
`record_count` above `MAX_TRACE_RECORDS` or above `capacity`; a capacity of zero or above
`MAX_TRACE_CAPACITY`; `first_index != dropped`; truncated or missing header/string/record
bytes; trailing bytes after the last record; non-UTF-8 or empty/oversized backend strings;
unknown kind codes; invalid widths/ports/levels/phases; non-contiguous indexes; decreasing
ticks; record ticks outside `start_tick..=end_tick`; `end_tick < start_tick`; and a total
encoded size above `MAX_TRACE_BYTES`. The declared record count is validated against the
remaining byte count before any `Vec::with_capacity`, so a forged huge count cannot force
a large allocation. There is no migration, best-effort parse or version coercion: v1 is
the only accepted version.

#### 3.2.4 Limits and errors

`TraceError` is one typed enum with a stable `code()` string for diagnostics: invalid
magic, unsupported version, unknown record kind, truncated input, trailing bytes, invalid
metadata field, invalid record count/capacity, non-contiguous index, decreasing tick,
invalid record field, too large, program counter unavailable, opcode read failed, event
observation unavailable, event ring overflow, invalid configuration, invalid query and
backend error. It implements `Display` and `std::error::Error`; production code never
`unwrap`s on it.

#### 3.2.5 TSV export (one-way)

`Trace::to_tsv() -> String` is a deterministic, tab-separated, one-way export with no
import path. It begins with the comment lines:

```text
# chronofork trace v1
# backend=<backend>
# backend_version=<version>
# state_hash=<64 lowercase hex>
# start_tick=<decimal>
# end_tick=<decimal>
# capacity=<decimal>
# records=<decimal>
# first_index=<decimal>
# dropped=<decimal>
# stop_reason=<step_limit|halted|stop_condition>
# triggered=<0|1>
# events_observed=<0|1>
index	kind	tick	pc	address	value	width	extra
```

Each record is one row. `index` and `tick` are decimal; `kind` is
`instruction|bus_write|vdp_port|controller_read|interrupt`; `pc` is six uppercase hex
digits, `address` and `value` are eight uppercase hex digits, `width` is decimal, and
absent fields are `-`. `extra` is `-` except `vdp_port` (`read`/`write`) and `interrupt`
(`requested`/`delivered`/`acknowledged`). Fields are mapped as: instruction
`pc`/opcode; bus write `address`/`value`/`width`; VDP port `offset`/`value`/`width` plus
`extra`; controller read `port`/`value`; interrupt `level` in `value`, `extra` = phase.
An empty trace still emits the complete comment header and table header. The TSV row is
the export form; `TraceRecord::summary()` is the separate one-line unprefixed
`<kind> index=... tick=... <fields>` form used by
CLI query/compare output (§3.2).

### 3.3 Capture model

```rust
pub struct TraceConfig {
    pub capacity: usize,
    pub start: TraceStart,
    pub stop: TraceStop,
    pub kinds: TraceKindMask,
    pub event_capacity: usize,
    pub input: InputFrame,
}
pub enum TraceStart { Immediate, Pc(u32), Tick(Tick) }
pub enum TraceStop { None, Pc(u32), Tick(Tick) }
```

`TraceConfig::new(capacity)` supplies defaults (`Immediate`, `None`, all kinds,
`DEFAULT_EVENT_CAPACITY`, `InputFrame::NEUTRAL`) and every builder validates its bound.
`capture(backend: &mut dyn Backend, config: &TraceConfig, steps: u64) -> Result<Trace,
TraceError>`:

1. Require `steps` in `1..=MAX_CAPTURE_STEPS`.
2. Inspect once for tick, `pc`, state hash and halted flag; require the `pc` register and
   a value that fits `u32`. At each boundary where an instruction is to be recorded, read
   the 2-byte big-endian opcode through `read_memory` at `pc`.
3. If `debug_events` is advertised, install
   `DebugConfig::new(config.event_capacity, Vec::new())`; otherwise set
   `events_observed = false`. A missing `debug_access` with an advertised capability is
   `EventObservationUnavailable`.
4. Loop, inspecting at each instruction boundary: evaluate the stop trigger first and
   break without executing when it matches; then stop with `Halted` if the CPU is halted;
   then evaluate the start trigger; once triggered, append the instruction record (tick =
   pre-step tick, opcode = big-endian word at `pc`) when the instruction kind is enabled;
   step once with `config.input`; drain new `DebugEvent`s in order into records mapped by
   §3.3.1; if the backend event ring advanced its dropped count between drains, return
   `EventRingOverflow` instead of silently losing events.
5. Stop when the executed step count reaches `steps` (`StepLimit`), when a step reports
   halted (`Halted`), or when the stop trigger matches (`StopCondition`). `end_tick` is
   the final observed tick: the entry/stop-boundary tick when no step was executed, or the
   `StepOutcome` tick of the last executed step. `executed` counts completed steps.
6. Retain the newest `capacity` records in a circular buffer; each stored record keeps its
   absolute index. `first_index` equals `dropped` (the first retained absolute index), and
   indexes start at 0 for the post-trigger capture stream even when no trigger fires.

**Triggers.** `TraceStart::Immediate` always triggers. `Pc` and `Tick` trigger
inclusively (`pc == value`, `tick >= value`) and are evaluated at boundaries; if the stop
trigger also matches the same boundary, the stop wins, nothing is recorded and
`triggered` records whether a start had already matched. `TraceStop::Pc` stops before
executing the instruction at that PC; `TraceStop::Tick` stops before the first step whose
pre-step tick is `>= value`. Capturing fewer than `steps` steps is success with the
recorded stop reason; it is never converted into an error.

**Tick convention.** Records carry emulated `Tick` values only; no host timestamp, elapsed
time, pid, path or entropy enters a record, metadata or comparison. An instruction record
uses the pre-step tick. Backend events keep the pre-step tick assigned by ADR-0010, so an
instruction record and the events produced by that step share the tick and the sequence is
non-decreasing.

#### 3.3.1 Event mapping

| `DebugEventKind` | Trace record |
|---|---|
| `InterruptRequested { level }` | `Interrupt { level, phase: Requested }` |
| `InterruptDelivered { level }` | `Interrupt { level, phase: Delivered }` |
| `InterruptAcknowledged { level }` | `Interrupt { level, phase: Acknowledged }` |
| `BusWrite { address, value, width }` | `BusWrite` |
| `VdpPort { offset, value, width, write }` | `VdpPort` |
| `ControllerRead { port, value }` | `ControllerRead` |
| `Watchpoint { .. }` | no trace record (host debugger trigger only) |

Records disabled by `kinds` are dropped after mapping and do not consume ring capacity;
the backend event ring is drained on every step regardless.

#### 3.3.2 Non-perturbation

Capture never calls `snapshot`, `restore`, `write_memory` or any canonical mutator other
than `step`. For the same start state, step count and input frame, the canonical state
hash, configuration hash, snapshot payload bytes/version and later movie bytes must be
identical with capture disabled and enabled. Trigger configuration, event capacity and
installed watchpoints are non-canonical. This is an executable requirement, not an
assumption proven by [M5-A35](../quality/m5-tracing-and-scripting-acceptance.md).

#### 3.3.3 Errors

`capture` returns typed errors for invalid steps/capacity/event capacity/mask, missing
`pc`, oversized `pc`, opcode read failure, missing event observation, event ring overflow,
and any wrapped `BackendError`. It never panics and never writes a partial `Trace` file;
CLI export builds and validates the `Trace` in memory before opening the output file.

### 3.4 Session capture entry point

The session layer owns the backend, so it owns the single capture entry point:

```rust
impl Session {
    pub fn capture_trace(&mut self, config: &TraceConfig, steps: u64)
        -> Result<Trace, SessionError>;
}
impl SessionManager {
    pub fn capture_trace(&mut self, id: SessionId, config: &TraceConfig, steps: u64)
        -> Result<Trace, SessionError>;
}
```

The manager looks up the session and delegates; no second capture loop exists. The CLI
`trace capture` and the script `trace_capture` builtin both call this entry point and never
run their own loop. Capture requires a `Ready` session (no backend halt or `Paused`/`Closed`
state), advances the session by the executed steps, and wraps a `TraceError` in one typed
`SessionError::Trace` variant. It allocates no snapshot identity and leaves the snapshot
map and lineage unchanged.

### 3.5 Deterministic script runtime (`chronofork-script`)

**Pipeline.** `source bytes -> lexer -> parser (typed AST) -> evaluator with a host
context -> ScriptRun report`. The canonical grammar below is the only script language;
the parser is total over its grammar and returns typed errors with 1-based line and column
for parse errors and the statement line for evaluation errors.

**Values.** `Int(i64)`, `Bool(bool)`, `Text(String)` (UTF-8). Text authored in a literal
or produced by concatenation is `<= 4 KiB` per value; the runtime-produced `read` text has
its own bound below and is charged to `string_bytes` when it becomes live. No
floats, arrays, maps, user functions, closures, imports, macros, host environment
variables, network, process, clock, random or filesystem access except the sandboxed
builtins. There is no truthiness: conditions and `!` require `Bool`.

**Grammar** (newline-separated statements, `#` comments to end of line, newlines inside
parentheses ignored):

```text
program    := stmt+
stmt       := 'let' IDENT '=' expr NEWLINE
            | 'set' IDENT '=' expr NEWLINE
            | 'print' expr NEWLINE
            | 'assert' expr NEWLINE
            | 'if' expr block ('else' (if_stmt | block))?
            | 'repeat' expr block
            | 'while' expr block
            | call NEWLINE
block      := '{' NEWLINE? stmt* '}' NEWLINE?
expr       := or_expr
or_expr    := and_expr ('||' and_expr)*
and_expr   := cmp_expr ('&&' cmp_expr)*
cmp_expr   := add_expr (('=='|'!='|'<'|'<='|'>'|'>=') add_expr)?
add_expr   := mul_expr (('+'|'-') mul_expr)*
mul_expr   := unary (('*'|'/'|'%') unary)*
unary      := ('-'|'!') unary | primary
primary    := INT | STRING | 'true' | 'false' | IDENT | call | '(' expr ')'
call       := IDENT '(' [expr (',' expr)*] ')'
```

An empty or comment-only source contains no statement and is rejected with the typed
`parse.empty_program` error at line 1, column 1 (`program := stmt+`).

**Semantics.** Variables live in one flat, program-global environment; blocks introduce
no scope. `let` on an existing name is a typed error; `set` on an unknown name is a typed
error. String literals are double-quoted with `\\`, `\"`, `\n`, `\r`, `\t` escapes; an
unknown escape is a parse error; the unescaped value is bounded to 4 KiB. Integer literals
are decimal by default, or hexadecimal when prefixed with `0x`/`0X` (ASCII hexadecimal
digits, case-insensitive); both forms must fit `i64`. Unary minus is checked. Arithmetic
is checked `i64`
(overflow, division/modulo by zero and `i64::MIN / -1` are typed errors); `+` concatenates
two `Text` values and is type-checked otherwise; equality requires equal types; ordering
supports `Int` and `Text`; `&&`/`||` short-circuit and require `Bool`; `!` requires `Bool`.
`if` requires a `Bool` condition, executes one branch and supports `else if`; `repeat`
requires a non-negative `Int` not above `max_loop_iterations` and executes the body that
many times; `while` re-evaluates a `Bool` condition and counts each iteration. Calls are
expressions evaluated once, left to right; a statement-level call discards its value.
Every executed statement, every evaluated expression node and every loop iteration counts
one operation; the counter is checked before the work. `step(n)` charges exactly `n`
against `max_steps`; `trace_capture(n, ...)` and `tas_verify` first reject a request larger
than the remaining step budget (no partial advance) and then reserve their whole requested
/recorded step count as a conservative charge, even when the backend halts or the replay
diverges early. The reservation is a deterministic upper bound: it never changes a
successful report and never allows more work than the budget permits. Every `assert` is
counted; a false assertion fails the run.

**Builtins** (fixed arity, typed; unknown name, arity or argument type is a typed error):

- `open() -> Int`: creates the configured manager and one root session; typed error if
  already open.
- `step(n) -> Int`: executes exactly `n` steps with neutral input and returns the new
  tick; the step budget is checked before any step.
- `reset() -> Int`, `tick() -> Int`, `state_hash() -> Text`: restore the creation state
  (returning 0), read the current tick, and read the 64-lowercase-hex state hash.
- `register(name) -> Int`, `read(address, length) -> Text`: read one register (unknown is
  typed) and 1..=4096 bytes, returned as `2 * length` lowercase hex characters (up to 8192
  for length 4096; out-of-window is typed). The returned text is charged to `string_bytes`
  when it becomes live variable, print or report text; the 4 KiB authored-text bound does
  not apply to this runtime-produced value.
- `fork() -> Int`: returns the new current session id and leaves the source unchanged.
- `checkpoint() -> Int`, `restore(id) -> Int`, `select(id) -> Int`: create a snapshot,
  restore it (returning 0), and switch the current session (returning 0); unknown ids and
  the snapshot limit are typed.
- `trace_capture(steps, capacity) -> Int`: captures from the current session with neutral
  input, advances it, stores the latest trace and returns the retained record count.
- `trace_records() -> Int`: retained records in the stored trace (0 when none).
- `trace_export(path) -> Int`: writes canonical bytes to a sandbox file and returns the
  byte count.
- `trace_compare(path) -> Int`: first divergence index against a sandbox trace file, `-1`
  when identical.
- `tas_verify(path) -> Bool`: decodes and verifies a sandbox movie through sessions and the
  configured factory.
- `tas_compare(left, right) -> Int`: first divergent recorded tick, `-1` when identical.
- `report(key, value) -> Int`: adds one custom report field and returns 0.
- `len(text) -> Int`, `str(value) -> Text`: UTF-8 byte length and the deterministic
  decimal/`true`/`false`/text form.
- `fail(text) -> bottom`: typed runtime failure with a deterministic message.

`report` keys match `[A-Za-z_][A-Za-z0-9_]*`, are at most 64 bytes and unique; values are
`Int`, `Bool` or `Text`; `print` accepts any of the three and renders it like `str`.
File builtins require the sandbox; `trace_export`,
`trace_compare`, `tas_verify` and `tas_compare` return `NoStoredTrace`/`NoSandbox`/typed
decode errors rather than panicking. The library receives an `Arc<dyn BackendFactory>` from
the caller; the CLI injects the synthetic factory by default and `MegadriveFactory` with
`--rom`. The script crate never names a concrete backend. Builtins that require an open
manager or current session fail with a typed `NotOpen` before `open()`.

**Limits** (`ScriptLimits`, validated at construction; defaults and maxima):

- `source_bytes`: default 64 KiB, maximum 1 MiB; the source is read bounded and rejected
  before lexing.
- `operations`: default 1,000,000, maximum 100,000,000; checked before every statement,
  expression node, loop iteration and builtin call.
- `loop_iterations`: default 1,000,000, maximum 100,000,000; checked before each
  `repeat`/`while` iteration.
- `variables`: default 256, maximum 4,096; checked before a `let` inserts.
- `string_bytes`: default 1 MiB, maximum 16 MiB; aggregate live variable text plus print
  and report text plus stored trace bytes, checked before allocation.
- `report_bytes`: default 64 KiB, maximum 1 MiB; report key/value text, checked before
  appending.
- `steps`: default 1,000,000, maximum 100,000,000; backend steps, checked before each
  requested advance.
- `trace_records`: default and maximum 1,000,000; retained records in the stored trace;
  the capture capacity/request is checked first.
- `snapshots`: default and maximum 1,024; checkpoints held by the script session set,
  checked before creation.
- `wall_time_ms`: default `Some(30_000)`, `None` disables; injected-clock deadline checked
  every 256 operations and at each statement boundary.

Wall time is the only host-time input. It may abort a run with `WallTimeLimit`, but it can
never change a successful report: a successful run's report is byte-identical whether the
clock advances or not. Production uses `std::time::Instant`; unit tests inject a fake
clock so the enforcement point and statistics are deterministic. `None` (CLI
`--wall-time-ms 0`) disables the deadline.

**Sandbox.** File builtins take a relative path and use only the explicit
`--sandbox <DIR>` root. The root is canonicalized once and must be an existing directory.
A path is rejected when empty, absolute, or containing `.` or `..` components or a NUL
byte; the joined target is canonicalized and must remain under the root, be a regular file
for reads, and be a regular file or absent for writes (never a directory or special
file). Reads are bounded (64 MiB for trace files, the existing movie bound for movies) and
export bytes are bounded by `MAX_TRACE_BYTES`. Without a sandbox, file builtins fail with
`SandboxUnavailable`; non-file builtins still work. The sandbox is a deterministic
capability boundary for experiments, **not** an OS security boundary against hostile
native code: scripts cannot execute native code, spawn processes, open sockets, read host
environment variables or write outside the root. Failure messages never echo a host path.
This satisfies `NFR-SEC-003` for the in-process DSL and is stated honestly rather than
claiming process isolation.

**Report and status.** `ScriptRun` carries a deterministic `ScriptStatus`
(`passed`/`failed`), an optional stable failure code and deterministic message, the source
SHA-256, statistics (`statements`, `operations`, `assertions`, `steps`, `variables`,
`report_bytes`, `trace_records`), ordered print lines, custom report fields, and the
backend id. Report fields are emitted sorted by key. The report contains no wall-clock
time, pid, hostname, path, random value or unordered map. A script-level failure is a
failed run with a stable code, never a panic or an untyped error; a successful run is
reproducible byte-for-byte. An "experiment" in M5 is exactly one script run and its
report; there is no separate persistent experiment store or format in v1.

**Embedding rejection.** Rhai, Luau, Lua, WASM (wasmtime/wasmer), Python (`pyo3` or a
subprocess) and any host-language plugin are explicitly **rejected** for v1:
licensing and attribution, dependency and supply-chain review, toolchain/build-script and
native-code surface, non-determinism (GC, hash/map ordering, float semantics, clock,
random, locale, environment), difficulty of enforcing byte-accurate memory/time limits
inside a foreign runtime, and the impossibility of guaranteeing stable typed errors and
byte-identical reports across runtime versions. Shelling out to a host interpreter is
rejected for the same reasons plus host privilege exposure; the roadmap names those
engines as examples, not as a mandate. A purpose-built lexer/parser/evaluator over
`i64`/`bool`/`text` is small, auditable, dependency-free relative to the existing graph
and deterministic by construction.

### 3.6 CLI surfaces

**`chronofork trace`** (thin adapter over `chronofork-trace`; default synthetic backend,
`--rom` selects the minimal Mega Drive slice and its factory):

```text
chronofork trace capture [--rom <FILE>] [--steps <N>] [--input <U32>]
    [--capacity <N>] [--event-capacity <N>] [--kinds <LIST>]
    [--start-pc <ADDR>] [--start-tick <T>] [--stop-pc <ADDR>] [--stop-tick <T>]
    --out <FILE>
chronofork trace info --file <FILE>
chronofork trace query --file <FILE> [--from-tick <T>] [--to-tick <T>]
    [--kind <instruction|bus|device|interrupt>] [--pc <ADDR>] [--address <ADDR>]
    [--limit <N>]
chronofork trace compare --left <FILE> --right <FILE>
chronofork trace export --file <FILE> [--format tsv|canonical] --out <FILE>
chronofork trace --help
```

Defaults: `--steps 1000`, capacity 4096, event capacity 1024, all kinds and neutral input;
`--rom` selects `MegadriveFactory`, otherwise the synthetic factory. `--input <U32>` sets
the port-0 input mask for every step (`0` is neutral); it is a thin pass-through to
`TraceConfig::with_input`. Instruction records carry only PC/opcode, so input sensitivity
is proved on the Mega Drive controller-read device records and by the backend tests.
`--kinds` is a comma list of
`instruction,bus,device,interrupt`; `bus` = BusWrite, `device` = VdpPort + ControllerRead;
duplicates, unknown names and an empty list are typed errors. At most one start flag and
one stop flag may be given. `--out` is required for
`capture` and `export`: stdout stays a pure report and never receives binary or TSV
payload, resolving the brief's optional `[--out]` notation. A completed run prints one or
more `record=<kind>` blocks of `key=value` fields separated by one blank line; paths never
appear in stdout. Typed failures exit 1 with empty stdout and a `chronofork: ...` stderr;
help exits 0 with usage on stdout.

- `trace capture` prints `record=trace_capture`, `backend`, `backend_version`,
  `start_tick`, `end_tick`, `steps`, `capacity`, `records`, `first_index`, `dropped`,
  `stop_reason`, `triggered`, `events_observed`, `kinds`, `instruction_enabled`,
  `bus_enabled`, `device_enabled`, `interrupt_enabled`, `bytes`, `sha256`. Unselected
  groups are reported as `*_enabled=0` instead of a record table. `steps` is the captured
  tick delta `end_tick - start_tick`, not a separate executed-step counter; both
  acceptance backends advance the tick once per executed step, so it equals the
  executed-step count there.
- `trace info` prints `record=trace_info`, `format_version`, `backend`, `backend_version`,
  `state_hash`, `start_tick`, `end_tick`, `capacity`, `records`, `first_index`, `dropped`,
  `stop_reason`, `triggered`, `events_observed`, `bytes`, `sha256`.
- `trace query` prints a `record=trace_query` summary (`matched`, `returned`, `truncated`,
  `from_tick`, `to_tick`, `kind`, `instruction_enabled`, `bus_enabled`, `device_enabled`,
  `interrupt_enabled`, `pc`, `address`, `limit`) followed by one
  `record=trace_record` block per returned record with `index`, `tick`, `kind` and only
  that kind's fields (`pc`/`opcode`; `address`/`value`/`width`; `offset`/`value`/`width`/
  `write`; `port`/`value`; `level`/`phase`). A zero-match query still prints the summary.
- `trace compare` prints `record=trace_compare`, `identical`, `metadata_equal`,
  `divergence`, `divergence_tick`, `reason` (`record|left_missing|right_missing|none`),
  `left`, `right` (TSV-row summaries or `-`). A divergent comparison is a completed
  command (exit 0); an unreadable or invalid file is exit 1.
- `trace export` validates the input trace, writes canonical bytes byte-identically or the
  TSV export, and prints `record=trace_export`, `format`, `records`, `bytes`, `sha256` of
  the written payload. `--format` defaults to `tsv`.

**`chronofork script`** (thin adapter over `chronofork-script`; the CLI constructs the
factory and the sandbox root):

```text
chronofork script run --file <FILE> [--rom <FILE>] [--sandbox <DIR>]
    [--max-operations <N>] [--max-loop-iterations <N>] [--max-steps <N>]
    [--max-memory-bytes <N>] [--max-source-bytes <N>] [--max-report-bytes <N>]
    [--max-variables <N>] [--wall-time-ms <N>]
chronofork script --help
```

`--rom` selects `MegadriveFactory`, otherwise the synthetic factory. `--sandbox` enables
the file builtins. `--max-memory-bytes` maps to `ScriptLimits::string_bytes`; the other
flags map one-to-one. `trace_records` and `snapshots` keep their documented defaults in v1
(no CLI override). Zero is rejected except `--wall-time-ms 0`, which disables the
deadline. Output is one `record=script_print` block per print with `text=<escaped>`
(`\\`, `\n`, `\r`, `\t` escapes), then exactly one `record=script_report` block with
`status`, `failure`, `message` (failure only), `source_sha256`, `statements`,
`operations`, `assertions`, `steps`, `variables`, `report_bytes`, `trace_records`,
`backend` and custom `report.<key>` fields sorted by key. Exit 0 on `passed`, exit 1 on a
script-level `failed` (report still printed, stderr empty), and exit 1 with empty stdout
plus `chronofork: ...` stderr for CLI/file/usage/sandbox errors. No path, pid, host time
or hostname is printed in the report or stdout; path diagnostics stay on stderr.

**Process tests** (`crates/chronofork-cli/tests/trace.rs` and `script.rs`) spawn the built
binary and assert exact fields, exit codes, empty/non-empty streams, two-run byte
determinism, the typed rejection matrix and both backends where relevant, using only
synthetic fixtures generated in test code.

### 3.7 Deferred scope and non-goals

Not implemented and not claimed by M5: DMA, audio and Z80 trace sources (H2/H3);
rendering, frame pacing and a host-time run loop (M7/H2); GUI trace viewer (M7); HTTP,
MCP or typed agent tools (M6); hardware completeness, cycle accuracy, mid-instruction
stops, reverse execution, cross-platform runtime evidence (H5); commercial ROMs or
BIOS/firmware; a general bus-read record; instruction-fetch events without a watchpoint;
trace persistence beyond canonical files; trace import from TSV; compressed/encrypted
traces; trace migration; script imports, user functions, arrays, maps, floats, concurrency,
network, processes, environment access or persistent experiment stores; OS/process
isolation or an audited hostile-code security boundary; dynamic language plugins; and any
new external crate, `unsafe`, `#[allow]`, `#[ignore]`, weakened test or relaxed lint.

## 4. Rationale

- **Host trace, backend events.** Instructions are already observable through the generic
  contract; only bus/device/interrupt events need backend participation. Keeping capture
  in a host crate gives one format, one ring, one trigger model and one comparison semantic
  for every backend, and keeps `Capabilities::tracing = false` honest.
- **Canonical binary, TSV one-way.** A compact big-endian record stream with explicit
  bounds makes byte determinism, cheap parsing and typed rejection straightforward. TSV is
  a human/agent convenience, not a compatibility surface; supporting import would need a
  second parser and compatibility predicate.
- **Absolute indexes.** The ring keeps the absolute index with each retained record, so
  overflow does not destroy comparison identity; `first_index == dropped` makes the
  eviction state explicit instead of implicit.
- **Trigger at boundaries.** The instruction-quantum slice has no mid-instruction points;
  defining start/stop at inspected boundaries is testable and honest. Stop-before-execute
  matches M4 breakpoint semantics.
- **Purpose-built DSL.** A tiny typed language with explicit limits is auditable, has no
  hidden host inputs and can produce byte-identical reports; foreign engines would add
  license, supply-chain and determinism risk without adding required capability.
- **Sandbox as capability boundary.** File access is the only ambient resource the
  builtins need; a canonicalized root plus bounded reads makes experiments reproducible
  and prevents accidental host writes without pretending to be a security boundary.
- **Single source of truth.** The format codec, trigger matcher, query matcher, grammar,
  limits and report serializer each exist once in a library; the CLI and tests consume
  them instead of re-implementing.

## 5. Alternatives considered

1. **Backend-owned `Capabilities::tracing` and per-backend trace APIs.** Rejected: every
   backend would duplicate capture, trigger and ring logic, cross-backend comparison would
   be impossible, and `tracing` could only become `true` after a real backend implemented
   it—forcing either a false capability or no M5 surface. Host breakpoints follow the
   same ownership rule.
2. **Extend the existing Mega Drive TSV ring as the canonical trace.** Rejected: it is
   backend-specific, has no ticks, event classes, magic/version, metadata or rejection
   semantics, and is one-way text; it remains a debug facility.
3. **Serde/JSON/MessagePack/bincode or another external codec.** Rejected: adds a
   dependency and build/license/supply-chain surface, and JSON/MessagePack bring
   encoding/ordering ambiguities and migration questions that a fixed big-endian layout
   does not. The existing snapshot and movie codecs set the precedent for hand-written,
   validated canonical bytes.
4. **Embed a canonical `Snapshot::to_bytes` payload inside a trace container.** Rejected:
   it duplicates checkpoint ownership, makes the trace opaque to queries and changes what
   restore means; a trace is an observation stream, not state.
5. **Make TSV the canonical import/export format.** Rejected: text parsing raises
   locale/format/precision questions, needs a second compatibility predicate and loses
   strict byte determinism. TSV stays a one-way export.
6. **An unbounded `Vec<TraceRecord>` with no ring.** Rejected: an adversarial step count
   could exhaust memory; the circular buffer with `dropped`/`first_index` is already the
   required retention model.
7. **Return trace events from `Backend::step` or store a trace in canonical state.**
   Rejected: it changes the step contract, every caller, snapshots, hashes and movie
   bytes. M4's optional `DebugAccess` already provides the observation point without
   perturbing canonical state.
8. **Host wall-clock or filesystem timestamps in trace records.** Rejected: breaks
   determinism and byte-identical comparison. Ticks are the only trace time.
9. **Multiple simultaneous start/stop triggers in v1.** Rejected: it multiplies precedence
   cases without proving more behavior; one start and one stop with documented precedence
   (stop wins) is testable and sufficient for triggered capture.
10. **A generic `BusRead` record.** Rejected: no backend observes generic data reads
    through `DebugAccess`, and controller reads are device records; adding a producer would
    be speculative API and untestable.
11. **Rhai, Luau, Lua, WASM or Python embedding, or shelling out to a host interpreter.**
    Rejected for v1: license/attribution and dependency-review obligations, supply-chain
    and build-script/native-code exposure, non-deterministic runtime behavior (GC, map
    ordering, floats, clock/random/env), inability to enforce byte-accurate memory/time
    accounting inside a foreign runtime, unstable typed errors/report bytes across runtime
    versions, and host-privilege exposure when shelling out. The stage text lists them as
    examples, not a requirement.
12. **Process/OS isolation instead of an in-process sandbox.** Deferred: it would add IPC,
    OS-specific behavior and nondeterministic scheduling, and it does not by itself solve
    format/report determinism. The in-process capability sandbox is stated as such, and a
    future process/WASM isolation layer could reuse the same script/limit/report API.
13. **A scripted experiment store or database.** Rejected for v1: the deterministic
    `ScriptRun` report plus sandboxed file export is sufficient for reproducible
    experiments without new persistence, migration or transport scope.
14. **Reusing the M4 `Debugger` as the trace driver.** Rejected: the debugger models
    triggers/logpoints, not a bounded record stream, query or comparison format; coupling
    them would force unrelated debug policy into trace bytes.

## 6. Consequences

- New workspace crates `chronofork-trace` and `chronofork-script`; no new external
  dependency. `chronofork-sessions` depends on trace and gains the single capture entry
  point plus one typed `SessionError` variant. `chronofork-cli` gains `trace` and
  `script` command groups and process tests.
- `Capabilities::tracing` remains `false` for both backends; `debug_events` semantics are
  unchanged. The host trace surface is documented as host-provided.
- No canonical format change: snapshot payload version, state hashes, configuration
  hashes, movie formats and existing artifacts are unchanged. Trace files are a new,
  independent, versioned format with reject-only v1 import.
- New verified scope: canonical v1 trace round-trip,
  rejection, capture on the synthetic and minimal Mega Drive backends, triggers, circular
  retention, queries, first-divergence comparison, TSV export, `chronofork trace` CLI, the
  purpose-built deterministic script DSL with limits and sandbox, `chronofork script`
  CLI and process tests. The claim is limited to the deterministic synthetic backend and
  the minimal Mega Drive slice on Linux.
- The trace/script surface is bounded and process-local; no daemon, transport, GUI,
  persistent experiment store, hardware completeness, cycle accuracy or cross-platform
  runtime claim is made.
- Existing M1–M4 acceptance evidence remains the regression baseline; any changed demo,
  state, movie, store or oracle byte is a stop condition.
- Verification is owned by the independent verifier against the [M5 acceptance
  plan](../quality/m5-tracing-and-scripting-acceptance.md); this scope was verified by T12
  on 2026-11-16 (T10 integrated review PASS), while merge and release remain separately
  unauthorized.

## 7. Authorization boundary

This ADR authorizes design, implementation and independent verification inside the
`m5-tracing-scripting` worktree on branch `agent/m5-tracing-scripting`. It explicitly does
**not** authorize a commit, push, merge, rebase, tag, release, branch deletion or worktree
change. Integration and publication require a separate explicit authorization. The
durable documentation promotion (requirements, roadmap, milestones, README, docs index,
`AGENTS.md`, quality-gate block) happens only after the independent verifier PASS, and it
records verified evidence rather than anticipated results.

## 8. Evidence requirement

The contract is satisfied only when every row of the [M5 acceptance
plan](../quality/m5-tracing-and-scripting-acceptance.md) is `executed - passed` with the
exact command, tree state, observed output and ignored evidence artifact recorded, the
full quality gate and rustdoc pass, both demos and existing state/movie/store/oracle bytes
are unchanged, and the durable docs are promoted. That requirement was met by the final
T12 PASS on 2026-11-16 and this promotion; the ADR is therefore an implemented and
independently verified contract for the documented deterministic minimal-slice scope, not
a broader claim.
