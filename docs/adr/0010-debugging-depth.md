# ADR-0010: Debugging Depth — Breakpoints, Watchpoints, Views and Event Observation

- **Status:** **Accepted** (owner-approved 2026-11-16 by the explicit directive to
  implement M4 completely and commit each successful phase; see "Approval evidence").
  Implementation and independent verification are **pending** at acceptance time; the
  [M4 debugging-depth acceptance plan](../quality/m4-debugging-depth-acceptance.md) is
  the only execution record.
- **Date:** 2026-11-16
- **Decision owner:** Project owner
- **Approval evidence:** the project owner's goal directive, recorded in this session on
  2026-11-16: "use agent-teams e implemente totalmente o M4 — Debugging depth, faça
  commits em cada fase de sucesso, só interrompa se encontrar algum bloqueio ou se for
  necessário alguma decisão do usuário." The directive authorizes the decisions in this
  ADR and implementation under the M4 acceptance plan. It does not authorize a push, a
  merge or a release.
- **Supersedes:** none. It extends [ADR-0002](0002-backend-boundaries.md) (backend
  contract), [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md) (bounded,
  debug-only auxiliary state) and [ADR-0009](0009-real-hardware-input-tas.md) (minimal
  Mega Drive slice) without changing a canonical snapshot byte, state hash, payload
  version, configuration hash, movie byte or previously verified behavior.
- **Related:** [backend model](../architecture/backend-model.md), [execution
  model](../architecture/execution-model.md), [session model](../architecture/session-model.md),
  [Mega Drive slice](../architecture/megadrive-slice.md), [M4 debugging-depth acceptance
  plan](../quality/m4-debugging-depth-acceptance.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [quality gates](../quality/quality-gates.md), [anti-slop policy](../quality/anti-slop-policy.md),
  [research: TAS and debugging](../research/tas-and-debugging.md),
  [research: emulator architecture](../research/emulator-architecture.md).

## 1. Context

Read on 2026-11-16 in the `agent/m4-debugging-depth` worktree at HEAD
`e816d793ae8ca31209971b8b27bdcf2ae0fafaf9` (clean `main`). The following accepted,
implemented and independently verified pieces are the baseline this decision builds on.
None of them is restated as a new decision.

- **Backend contract** ([ADR-0002](0002-backend-boundaries.md),
  [backend model](../architecture/backend-model.md)): `Backend` exposes `step`,
  `snapshot`, `restore`, side-effect-free `inspect`, `read_memory` and debugger
  `write_memory`. `Capabilities` reports only real guarantees; `breakpoints`,
  `watchpoints` and `tracing` are currently `false` for both backends.
- **Sessions** ([session model](../architecture/session-model.md), ADR-0003): a
  `Session` owns one `Box<dyn Backend>`; `SessionInspection` carries tick, state hash,
  halted flag, a register map and one contiguous memory window. The session layer owns
  lifecycle, checkpoints, forks and the m3 `Player`; it does not expose the backend.
- **Minimal Mega Drive slice** ([Mega Drive slice](../architecture/megadrive-slice.md),
  EPIC-0002, ADR-0009): 68000 subset, big-endian ROM/RAM bus, VDP port window,
  deterministic instruction-quantum HBlank/VBlank scheduler with SR-mask gating, `RTE`,
  VINT pending/acknowledge, TH-high 3-button controller reads, versioned payload v2 and
  a bounded debug-only instruction trace. Rendering, DMA, audio, Z80, cycle accuracy and
  other interrupt sources are absent.
- **M3 debugging research** ([TAS and debugging](../research/tas-and-debugging.md) §5–§7,
  [emulator architecture](../research/emulator-architecture.md) §5.9–§5.11): the
  reusable ideas are per-processor breakpoints with location and data conditions, hit
  counters and enable flags; read/write watchpoints; call-stack/step-over as later UI
  work; instruction-boundary stops under a scheduler; separate machine state and debugger
  state; and an execution-guided disassembler whose outputs distinguish observed from
  predicted code. These are upstream observations, not ChronoFork decisions.
- **Current gap:** [Milestones](../roadmap/milestones.md) M4 is entirely unchecked,
  `FR-DEBUG-002`/`FR-DEBUG-003` are **Planned**, `FR-DEBUG-004` is **Partial**, and the
  only debug view is the flat `MachineInspection` register map plus the `megadrive trace`
  TSV ring. There is no way to stop before an address, observe a RAM write, see a
  disassembled instruction, decode `SR`/VDP/scheduler state, or inspect bus and interrupt
  events. The backend advertises `breakpoints = false`, `watchpoints = false` and
  `tracing = false`.

## 2. Problem

Add debugging depth without weakening the properties that make ChronoFork useful:

1. **Determinism and canonical state.** Debugger observation, breakpoints and events must
   not alter the canonical state, state hash, config hash, snapshot payload, movie bytes,
   tick sequence or replay result.
2. **Backend boundaries.** Breakpoints are a host concern; watchpoints and bus/device
   events can only be observed where the backend owns the bus. The generic host layer
   must not encode Mega Drive assumptions, and the Mega Drive crate must not own session,
   transport or UI policy.
3. **Honest capabilities.** A feature is advertised only where it is implemented and
   tested. A backend that cannot observe a bus must report that, and the host debugger
   must return a typed error rather than silently degrading to a state-diff approximation.
4. **Bounded resources.** Event logs, breakpoint/watchpoint tables, logpoints and
   disassembly output must have explicit limits; arbitrary input must produce typed
   errors, not unbounded allocation or panics.
5. **Debug granularity.** The slice advances one instruction per backend step. A stop is
   therefore at an instruction boundary; a watchpoint write is reported after the
   instruction that performed it. No mid-instruction or cycle-exact stop is claimed.
6. **Testability without a GUI.** Every view and stop must be usable headlessly through a
   stable CLI surface and executable acceptance commands.

## 3. Decision

Add four layers, each with one owner:

### 3.1 Backend observation contract (`chronofork-backend-api`)

A new `debug` module defines bounded event observation and watchpoint matching. The
contract is optional: `Backend::debug_access` returns `None` by default, so a backend
that does not implement observation compiles unchanged and the host reports the feature
as unavailable.

```rust
pub const DEFAULT_DEBUG_EVENT_CAPACITY: usize = 1024;
pub const MAX_DEBUG_EVENT_CAPACITY: usize = 65_536;
pub const MAX_WATCHPOINTS: usize = 64;

pub enum WatchpointAccess { Read, Write }

pub struct DebugAddressRange { /* inclusive start..=end */ }
impl DebugAddressRange {
    pub fn new(start: u32, end: u32) -> Result<Self, BackendError>;
    pub const fn start(self) -> u32;
    pub const fn end(self) -> u32;
    pub const fn contains(self, address: u32) -> bool;
}

pub struct WatchpointSpec { /* nonzero id, range, access */ }
impl WatchpointSpec {
    pub fn new(id: u32, range: DebugAddressRange, access: WatchpointAccess)
        -> Result<Self, BackendError>;
    pub const fn id(self) -> u32;
    pub const fn range(self) -> DebugAddressRange;
    pub const fn access(self) -> WatchpointAccess;
    pub const fn matches(self, access: WatchpointAccess, address: u32) -> bool;
}

pub enum DebugEventKind {
    InterruptRequested { level: u8 },
    InterruptDelivered { level: u8 },
    InterruptAcknowledged { level: u8 },
    BusWrite { address: u32, value: u32, width: u8 },
    VdpPort { offset: u8, value: u32, width: u8, write: bool },
    ControllerRead { port: u8, value: u8 },
    Watchpoint { watchpoint_id: u32, access: WatchpointAccess,
                 address: u32, value: u32, width: u8 },
}

pub struct DebugEvent { /* tick + kind */ }

pub struct DebugEventLog { /* capacity, contiguous index ring, dropped count */ }
impl DebugEventLog {
    pub fn new(capacity: usize) -> Result<Self, BackendError>;
    pub const fn capacity(&self) -> usize;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub const fn dropped(&self) -> u64;
    pub const fn next_index(&self) -> u64;
    pub fn first_index(&self) -> Option<u64>;
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = DebugEvent>
        + ExactSizeIterator + '_;
    pub fn get(&self, position: usize) -> Option<DebugEvent>;
    pub fn record(&mut self, event: DebugEvent);
    pub fn clear(&mut self);
}

pub struct DebugConfig { /* event_capacity + watchpoints */ }
impl DebugConfig {
    pub fn new(event_capacity: usize, watchpoints: Vec<WatchpointSpec>)
        -> Result<Self, BackendError>;
    pub const fn event_capacity(&self) -> usize;
    pub fn watchpoints(&self) -> &[WatchpointSpec];
}

pub trait DebugAccess {
    fn configure_debug(&mut self, config: DebugConfig) -> Result<(), BackendError>;
    fn debug_event_log(&self) -> Option<&DebugEventLog>;
    fn take_debug_event_log(&mut self) -> Option<DebugEventLog>;
}
```

`Backend` gains one defaulted method:

```rust
fn debug_access(&mut self) -> Option<&mut dyn DebugAccess> { None }
```

`Capabilities` gains `debug_events: bool`. It is `true` only where the backend can
produce `DebugEvent`s. `breakpoints` and `watchpoints` keep their current meaning:
`breakpoints` is a **backend** capability and remains `false` because breakpoints are
implemented by the host debugger over `inspect`; `watchpoints` becomes `true` on the
Mega Drive slice because the bus itself matches and reports them. The debugger reports
its own host-level support through its typed API and the CLI, never by rewriting a
backend capability.

All configuration errors use the existing typed `BackendError::InvalidConfiguration`.
Validation rules: event capacity is `1..=MAX_DEBUG_EVENT_CAPACITY`; at most
`MAX_WATCHPOINTS` specs; watchpoint ids are nonzero and unique; `start <= end`.

### 3.2 Host debugger (`chronofork-debug`)

A new crate depends only on `chronofork-core` and `chronofork-backend-api`. It borrows a
`&mut dyn Backend`; it does not own sessions, storage, transports or a UI.

```rust
pub struct Debugger<'a> { /* backend + breakpoint tables + log */ }
impl<'a> Debugger<'a> {
    pub fn new(backend: &'a mut dyn Backend) -> Result<Self, DebugError>;
    pub fn with_event_capacity(backend: &'a mut dyn Backend, capacity: usize)
        -> Result<Self, DebugError>;

    pub fn add_breakpoint(&mut self, spec: BreakpointSpec)
        -> Result<BreakpointId, DebugError>;
    pub fn add_watchpoint(&mut self, spec: WatchpointSpec)
        -> Result<WatchpointId, DebugError>;
    pub fn add_event_breakpoint(&mut self, filter: EventFilter)
        -> Result<EventBreakpointId, DebugError>;
    pub fn add_logpoint(&mut self, location: Location, message: &str)
        -> Result<LogpointId, DebugError>;
    pub fn remove_breakpoint(&mut self, id: BreakpointId) -> Result<(), DebugError>;
    pub fn remove_watchpoint(&mut self, id: WatchpointId) -> Result<(), DebugError>;
    pub fn remove_event_breakpoint(&mut self, id: EventBreakpointId)
        -> Result<(), DebugError>;
    pub fn remove_logpoint(&mut self, id: LogpointId) -> Result<(), DebugError>;
    pub fn set_enabled(&mut self, id: TriggerId, enabled: bool)
        -> Result<(), DebugError>;
    pub fn triggers(&self) -> TriggerSnapshot<'_>;

    pub fn inspect(&self) -> Result<MachineInspection, DebugError>;
    pub fn read_memory(&self, address: u64, length: usize)
        -> Result<Vec<u8>, DebugError>;
    pub fn run(&mut self, step_limit: u64) -> Result<DebugRun, DebugError>;
    pub fn write_memory(&mut self, address: u64, data: &[u8])
        -> Result<(), DebugError>;
    pub fn retained_events(&mut self) -> Result<Vec<DebugEvent>, DebugError>;
    pub fn log_entries(&self) -> &[LogEntry];
}
```

**Locations and conditions.** `Location` is `Address(u32)` or `Range { start, end }`
(inclusive). A `BreakpointSpec` has a location and an optional `Condition`; both must
hold for a hit. A `Condition` is a register comparison or a bounded memory comparison:

```rust
pub enum Relation { Equal, NotEqual, Less, LessOrEqual, Greater, GreaterOrEqual }
pub enum ConditionOperand {
    Register(String),                       // exact key in MachineInspection::registers
    Memory { address: u32, length: u8 },    // 1..=8 bytes, big-endian
}
pub struct Condition { operand: ConditionOperand, relation: Relation, value: u64 }
```

The `pc` register must exist for a location match; both current backends expose it.
An unknown register at `add_breakpoint` time is a typed `UnknownRegister` error. Memory
conditions are evaluated through the side-effect-free `read_memory` path.

**Watchpoints.** A host `WatchpointSpec` (the backend type) is installed in the backend
`DebugConfig`; the backend reports a `Watchpoint` event for every matching emulated bus
access. The host maps the event's nonzero watchpoint id back to the host trigger. The
host never approximates a watchpoint by diffing memory windows.

**Event breakpoints.** `EventFilter` selects one event class. A matching retained event
after a step stops the run.

```rust
pub enum InterruptPhase { Requested, Delivered, Acknowledged }
pub enum EventFilter {
    Interrupt { level: Option<u8>, phase: InterruptPhase },
    Vdp { write: Option<bool>, offset: Option<u8> },
    ControllerRead { port: Option<u8> },
    BusWrite { range: Option<DebugAddressRange> },
}
```

**Logpoints.** A logpoint has a location and a bounded message (1..=128 bytes, no
newline). It appends a `LogEntry` and never stops execution. The log is a bounded ring
with a dropped counter.

**Run semantics.** `run(step_limit)` with `step_limit` in `1..=MAX_RUN_STEP_LIMIT`
(`1_000_000`) executes this loop:

1. Inspect. In trigger id order, a matching enabled breakpoint stops **before** the
   instruction executes. Then matching logpoints append entries. Then a halted CPU stops
   with `Halted`; then a reached step limit stops with `StepLimit`.
2. Step once with `InputFrame::NEUTRAL`.
3. Collect retained events since the previous cursor. A watchpoint hit stops with
   `Watchpoint`; otherwise the first matching enabled event breakpoint in id order stops
   with `Event`. Remaining events stay available through `retained_events`.
4. Repeat.

Breakpoint/logpoint triggers therefore describe the state **before** the next
instruction. Watchpoint and event stops describe the state **after** the instruction that
produced the event. When both a watchpoint and an event breakpoint match the same step,
the watchpoint stop wins and all matching events remain in the report. A breakpoint at
the current PC wins over `Halted` and `StepLimit`.

**Stop report.** `DebugRun` records the stop reason, steps executed in this call, tick,
state hash, halted flag, matched events, event dropped count and the retained log entries
with their dropped count. `StopReason` is `Breakpoint { id }`, `Watchpoint { ids }`,
`Event { id }`, `Halted` or `StepLimit`.

**Limits.** At most 64 breakpoints, 64 watchpoints, 64 event breakpoints and 64
logpoints. Event capacity is `1..=65_536` (default 1,024). The logpoint ring is fixed at
256 entries in this increment; the public `MAX_LOG_CAPACITY` constant is only the asserted
compile-time upper bound (4,096), not a configurable capacity. A full table returns a typed
error; a full ring evicts the oldest entry and counts it. All limits are constants in the
crate.

**Errors.** `DebugError` is one typed enum: backend errors, unsupported observation,
missing `pc`, unknown trigger, unknown register, invalid range/condition/message/limit,
and table-full errors. No stringly fallback exists.

### 3.3 Mega Drive observation and views (`chronofork-megadrive`)

**Observation.** The backend owns one optional `DebugAccess` state (an event ring plus
watchpoint specs). It is debug-only: absent from `state_body_bytes`, the state hash, the
configuration hash, payload version and all snapshots; `restore` leaves it untouched,
matching the existing instruction-trace rule. The bus records:

- `Watchpoint` for every matching byte access, read or write, on any mapped or device
  window. The CPU's own opcode and extension-word fetches are ordinary bus reads and
  therefore match a read watchpoint; they are not logged as a separate event class.
- `BusWrite` for work-RAM byte writes (VDP writes are `VdpPort` events, not duplicates).
- `VdpPort` for VDP port byte/word reads and writes with offset, value and width.
- `ControllerRead` for controller data-register reads with port and byte value.

The backend records `InterruptRequested` when the scheduler's per-level issued counter
changes, `InterruptDelivered` after a successful interrupt entry and
`InterruptAcknowledged` after the scheduler acknowledges exactly the delivered level.
Events created during a step carry the tick at the start of that step (the pre-step
`tick`); byte-granular bus events are the documented unit because the bus serves byte
accesses. There is no general instruction-fetch event; a fetch appears only through a matching
watchpoint. Debugger `read_memory`/`write_memory` never produce events.

**Views.** New pure view types in `chronofork-megadrive` expose decoded state without
side effects:

- `CpuView`: PC, SR, halted, supervisor, trace bit, interrupt-priority mask and X/N/Z/V/C.
- `VdpView`: status word, VBlank and VINT-pending bits, address, auto-increment,
  24-register file, VRAM length and VRAM SHA-256.
- `InterruptView`: video standard, instructions per line, lines per frame, active lines,
  quanta, frame, line, quantum, pending requests with level/issue tick/deliverable, and
  per-level issued/coalesced counters.
- `BusView`: ROM length, RAM length and the mapped window table (name, start, end,
  writable).
- A read-only `MegadriveBus` accessor on the backend, so views never reach into private
  state.

**Disassembly.** A bounded, ROM/RAM-memory-window disassembler covers exactly the
instruction encodings the CPU executes: MOVEQ, branches/BSR, MOVE.W #imm,SR, NOP, RTS,
RTE, JMP, JSR, CLR, LEA, PEA, TST, MOVE #imm, MOVE Dn,Dm, MOVE Dn,(abs).L, MOVE
(abs).L,Dn, CMP, DBRA, ADDQ/SUBQ and ADD/SUB, plus `DC.W` for every unknown or
unsupported encoding. Its documented textual mnemonics and lengths must match execution:
for every supported form a test asserts that the disassembled length equals the CPU's
`PC` delta and that the opcode word equals the fetched word. The disassembler is a
viewer, never an execution path; the CPU decoder remains authoritative.

Limits: `1..=4_096` instructions per request; addresses must be even; every extension
word must be inside the supplied window; too many instructions, an odd address or an
out-of-window word is a typed error. No speculative analysis, code/data inference,
control-flow graph export, labels or jump-table recovery is claimed.

### 3.4 CLI (`chronofork-cli`)

A new `chronofork debug` command group operates on the minimal Mega Drive slice and
requires `--rom <FILE>`. It exposes only implemented behavior:

```text
chronofork debug run --rom <FILE> --steps <N>
    [--breakpoint <LOCATION>[:<CONDITION>]]...
    [--watchpoint <START-END|ADDR>:<r|w|rw>]...
    [--eventpoint <FILTER>]...
    [--logpoint <LOCATION>[:<MESSAGE>]]...
    [--max-events <N>] [--show-events <N>]
chronofork debug inspect --rom <FILE> [--steps <N>]
chronofork debug memory --rom <FILE> [--steps <N>] --address <ADDR> --length <N>
chronofork debug disasm --rom <FILE> [--steps <N>] [--address <ADDR>] [--count <N>]
chronofork debug vdp --rom <FILE> [--steps <N>] [--vram-address <ADDR> --vram-length <N>]
chronofork debug interrupts --rom <FILE> [--steps <N>]
chronofork debug bus --rom <FILE>
```

`LOCATION` is `ADDR` or `START-END` (inclusive). `CONDITION` is `reg:<name><op><value>`
or `mem:<address>:<length><op><value>`, with `<op>` one of `==`, `!=`, `<`, `<=`, `>`,
`>=`. `FILTER` is `interrupt:<requested|delivered|acknowledged>[:<level>]`,
`vdp:<read|write>[:<offset>]`, `controller:<port|any>` or `bus-write[:<START-END>]`.
Address and value tokens accept decimal or `0x`/`0X` hexadecimal. All parsers are
total over their grammar and return typed CLI errors otherwise; they never panic on
overflow, an empty range, an unknown register name or an out-of-bounds length.

Output uses the established stdout-only `record=<kind>` / `key=value` blocks separated by
blank lines, stderr diagnostics prefixed `chronofork:` and exit code 0 for a completed
run/read and 1 for any typed failure. A valid `debug run` that stops by breakpoint,
watchpoint, event, halt or step limit is success (exit 0); the `reason` field
distinguishes the stop. Every block is deterministic: no timestamps, process ids, host
labels, paths in data fields, randomness or unordered maps.

## 4. Rationale

- **Host breakpoints, backend watchpoints.** A breakpoint only needs the next
  instruction address and register/memory state, both already in the backend contract.
  A watchpoint needs the write itself, which only the bus can report. Splitting them keeps
  the generic debugger small and avoids pretending a memory diff is a watchpoint.
- **One optional trait, one capability.** `debug_access` is additive and defaulted, so a
  future backend can report observation support without implementing the debugger.
- **Non-canonical debug state.** Matches the verified instruction-trace rule and keeps
  every M1/M2/M3 artifact byte-identical. It also prevents a client from believing that a
  restored snapshot restores breakpoints.
- **Views as pure types.** Decoding SR/VDP/scheduler state in one place prevents the CLI
  and future GUI from duplicating bit layouts.
- **Disassembler as a cross-checked viewer.** The CPU remains the single execution
  authority; the equality test catches drift between the viewer and execution.
- **Bounded everything.** Debugger input is user-supplied and must not become an
  allocation or denial-of-service path.

## 5. Alternatives considered

1. **Install breakpoints inside `Backend`.** Rejected: every backend would duplicate
   stop evaluation, the `step` contract has no stop result, and the host layer would
   still need conditions and a run loop. Watchpoints stay in the backend because the bus
   owns the observation point.
2. **Approximate watchpoints by diffing `MachineInspection.memory`.** Rejected: it misses
   reads, misses writes that restore the same value, cannot see device windows, and
   advertises a guarantee the backend does not make.
3. **Return `DebugEvent`s from `step`.** Rejected: it changes the step contract, the
   snapshot/replay shape and every caller; an optional observation surface is additive.
4. **A full interactive debugger REPL now.** Deferred: one-shot commands plus the
   library API cover headless acceptance; a REPL adds process surface without proving
   more behavior. It can reuse the same debugger crate later.
5. **A complete 68000 disassembler.** Rejected for this increment: it would claim
   coverage the CPU cannot execute. Unknown encodings print `DC.W`.
6. **Persist debugger state in the snapshot.** Rejected: it changes canonical bytes and
   would make breakpoint configuration a compatibility field. Debugger state is
   process-local, like the instruction trace.

## 6. Consequences

- `chronofork-backend-api` gains the `debug` module, `debug_access`, and
  `Capabilities::debug_events`. Existing backend implementations compile unchanged.
- `chronofork-megadrive` gains the observer, views and disassembler, and its descriptor
  reports `watchpoints = true` and `debug_events = true`. `breakpoints` remains `false`
  at the backend level; the host debugger is the feature owner.
- A new `chronofork-debug` workspace crate; `chronofork-cli` gains the `debug` command
  group and its integration tests.
- No canonical format change: payload version stays 2, no snapshot/schema/movie version
  bump, no new canonical state field. Debug artifacts are bounded, process-local and
  absent from hashes and snapshots.
- New verified scope: PC/range/conditional breakpoints, read/write watchpoints, event
  breakpoints, logpoints, register/memory/disassembly views, interrupt/bus/device event
  observation and Mega Drive CPU/VDP/interrupt/bus inspectors for the minimal slice on
  Linux. Not claimed: mid-instruction or cycle-exact stops, reverse execution, call-stack
  or step-over/step-out, conditional watchpoint values, bus-read logging without a
  watchpoint, DMA/Z80/audio/rendering inspection, persistent debugger state, UI, HTTP,
  MCP or cross-platform runtime evidence.
- Verification is owned by the independent verifier against the M4 acceptance plan; a
  successful implementation commit is not a verified milestone.

## 7. Evidence requirement

Implementation is complete only when every row in the [M4 debugging-depth acceptance
plan](../quality/m4-debugging-depth-acceptance.md) is `executed - passed` with the exact
command, tree state, observed values and raw artifact recorded, the full quality gate and
both demos are green, and the durable docs are promoted. Until then this ADR may be cited
for the design only, never as proof of implementation or verification.
