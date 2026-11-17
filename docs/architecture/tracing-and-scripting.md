# Tracing and Scripting Architecture

- **Status:** **implemented and independently verified** for the documented minimal-slice
  scope (final T12 PASS, 2026-11-16, on branch `agent/m5-tracing-scripting` at HEAD/base
  `84f1476466c14e72f911689845c390173982fd81`; implementation complete and uncommitted,
  integration pending). All M5-A rows in the [M5 acceptance
  plan](../quality/m5-tracing-and-scripting-acceptance.md) are `executed - passed`; the
  T7 round returned `needs_revision`, T11 repaired F1–F10, and T12 re-verified the result.
  The authoritative contract, exact byte layout, grammar, limits and deferred scope live in
  [ADR-0011](../adr/0011-tracing-and-scripting.md); this page describes how the layers
  fit together and does not restate the format tables.
- **Scope:** roadmap [Stage 5 — Tracing and
  scripting](../roadmap/roadmap.md#stage-5--tracing-and-scripting) and
  [M5](../roadmap/milestones.md#m5--tracing-and-scripting) on the deterministic synthetic
  backend and the minimal Mega Drive slice, Linux acceptance platform.

## Purpose

M5 adds two products over the verified M1–M4 surface:

1. a versioned, bounded, structured **trace** of instructions and observed device events
   with emulated timestamps, triggered capture, circular retention, canonical export and
   import with typed rejection, historical queries and first-divergence comparison;
2. a sandboxed, deterministic **script** runtime whose experiments produce reproducible
   reports and can drive sessions, forks, inspection, traces and TAS movies.

Neither product owns canonical emulated state. Both are host layers over the existing
backend and session contracts.

## Layer diagram

```text
┌──────────────────────────────────────────────────────────────────────┐
│ chronofork-cli                                                       │
│   trace capture/info/query/compare/export     script run             │
├──────────────────────────────────────────────────────────────────────┤
│ chronofork-script                                                    │
│   lexer → parser → evaluator, limits, sandbox, report                │
│   host bindings: sessions, forks, trace, TAS                         │
├──────────────────────────────────────────────────────────────────────┤
│ chronofork-sessions                                                  │
│   Session/SessionManager, snapshots, forks, movies, capture_trace     │
├──────────────────────────────────────────────────────────────────────┤
│ chronofork-trace                                                     │
│   canonical v1 codec, capture ring, triggers, query, compare, TSV    │
├──────────────────────────────────────────────────────────────────────┤
│ chronofork-backend-api                                               │
│   Backend, inspect/read_memory, DebugAccess/DebugEventLog, capability │
├──────────────────────────────────────────────────────────────────────┤
│ chronofork-core                    backends (synthetic, Mega Drive)   │
│   Tick, InputFrame, StateHash      implement backend-api only         │
└──────────────────────────────────────────────────────────────────────┘
```

Dependency direction is one-way and matches [ADR-0011](../adr/0011-tracing-and-scripting.md)
§3.1:

- `chronofork-trace` depends only on core and backend-api.
- `chronofork-sessions` depends on trace because it owns the backend and exposes
  `Session::capture_trace`/`SessionManager::capture_trace`.
- `chronofork-script` depends on core, backend-api, sessions and trace; it never depends on
  a concrete backend, the CLI, a transport or a GUI.
- `chronofork-cli` composes a selected factory (synthetic by default, the minimal Mega
  Drive slice with `--rom`) with the trace and script libraries.

## Ownership boundaries

- **`chronofork-trace`** owns the canonical v1 codec, `TraceMeta`/`Trace`, the capture
  driver, trigger/ring/query/compare semantics, TSV export and `TraceError`. It must not
  own sessions, snapshots, concrete backends, transports, CLI parsing or host time.
- **`chronofork-sessions`** owns session lifecycle, the single `capture_trace` entry
  point, backend ownership and the movie decode/verify path used by script TAS bindings.
  It must not own the trace byte format, the script grammar or CLI output.
- **`chronofork-script`** owns the lexer/parser/evaluator, values, typed errors,
  `ScriptLimits`, sandbox path policy, report serialization and host bindings over
  sessions/trace/movies. It must not own concrete backend choice, session storage policy,
  CLI flags or host clock values in results.
- **`chronofork-cli`** owns argument parsing, factory/sandbox injection, deterministic
  `record=`/`key=value` rendering, exit codes and stderr diagnostics. It must not own a
  second codec, matcher, limit, grammar or capture loop.
- **Backends** own `inspect`/`read_memory` and, where advertised, `DebugAccess` events with
  their documented non-canonical semantics. They must not own trace files, queries, script
  execution or host policy.
- **`chronofork-debug` (M4)** owns host breakpoints, watchpoints/event triggers, logpoints
  and views. It must not own the trace format or a second record ring; its `Watchpoint`
  events are not trace records.

The backend `Capabilities::tracing` flag stays `false` on both backends. The trace surface
is host-provided over `register_inspection` + `memory_read`, with bus/device/interrupt
records only where `Capabilities::debug_events` is true. This mirrors the M4 rule that a
host feature is reported by the host, never by rewriting a backend capability. See the
[backend model](backend-model.md) and [ADR-0010](../adr/0010-debugging-depth.md).

## Capture data flow

```text
CLI trace capture ──► SessionManager::capture_trace
                         └─► Session::capture_trace
                              └─► trace::capture(&mut dyn Backend, config, steps)
                                    ├─ inspect() ── pc, tick, state hash, halted
                                    ├─ read_memory(pc, 2) ── big-endian opcode
                                    ├─ DebugAccess::configure_debug(DebugConfig)
                                    └─ loop:
                                         stop trigger? → stop before executing
                                         halted?       → stop
                                         start trigger? → mark triggered
                                         record instruction (pre-step tick)
                                         backend.step(config.input)
                                         DebugEventLog → BusWrite/VdpPort/
                                                         ControllerRead/Interrupt
                                                         records (newest ring)
                              └─ Trace { meta, records }
CLI trace export ◄── Trace::to_bytes / Trace::to_tsv
```

`trace capture` builds and validates the whole `Trace` in memory before opening `--out`, so
a typed failure never leaves a partial file. Import (`trace info/query/compare/export
--file`) goes through the same reject-only `Trace::from_bytes`; no command re-implements
the codec.

**PC and tick conventions.** `pc` is the address of the instruction about to execute, the
opcode is the 16-bit big-endian word there, and the instruction record uses the pre-step
tick. Backend events keep the pre-step tick assigned by ADR-0010, so a step's instruction
record and its events share a tick and the retained sequence is non-decreasing. Stops are
instruction-boundary only; there is no mid-instruction or cycle-exact trace point.

**Triggers and retention.** One start trigger (`Immediate`, `Pc`, `Tick`) and one stop
trigger (`None`, `Pc`, `Tick`) are evaluated at boundaries; the stop wins when both match.
Records are kept in a circular buffer of `capacity` entries; every retained record carries
its absolute index, `dropped` counts evicted records, and `first_index == dropped`. A
backend event-ring overflow between drains is a typed error, never silent loss.

## Format and query/compare semantics

The canonical format is fixed by [ADR-0011](../adr/0011-tracing-and-scripting.md) §3.2:
magic `CFTRACE1`, big-endian header, five record kinds, explicit limits, construction
invariants and reject-only parsing. The architecture-relevant properties are:

- **Single codec.** One encoder/decoder owns magic, version, header, record payloads and
  every rejection; CLI, script and tests call it.
- **Reject-only.** Wrong magic/version, unknown kinds, bad indexes/ticks, out-of-bounds
  metadata and truncation/trailing bytes are typed errors. There is no migration,
  best-effort parse or silent reinterpretation.
- **Byte determinism.** `Trace::to_bytes` is infallible after construction and produces
  identical bytes for identical metadata/records; no host data enters the file.
- **One-way text export.** `Trace::to_tsv` is a deterministic human/agent view with a
  comment header and a fixed table; there is no TSV import and hence no second
  compatibility surface. The TSV text is bounded by the retained record count (at most
  1,000,000 rows, roughly 82 bytes per row) and has no separate byte cap, so it can exceed
  the 64 MiB `MAX_TRACE_BYTES` bound that applies to the canonical binary. Canonical
  export is byte-capped and reject-only.

**Queries.** `Trace::query(&TraceQuery)` filters by inclusive tick range, kind mask,
instruction `pc` and numeric `address` (bus-write address, VDP offset or controller port).
It validates `from <= to` and `limit` in `1..=MAX_QUERY_LIMIT`, counts every match in
`matched`, returns at most `limit` records in `returned`, and reports
`truncated = matched > returned`. The CLI prints one summary block plus one
`record=trace_record` block per returned record.

**Comparison.** `Trace::compare(&other)` walks the union of the retained absolute-index
windows in ascending order. At the first absent index it reports `LeftMissing` or
`RightMissing`; at the first present-and-different index it reports `Record` with its
index, tick and the two records. Extra retained records on one side are therefore a
divergence at the first extra index. Metadata equality is reported separately
(`metadata_equal`) and never decides record identity. `identical` means no record
divergence; comparing is not a verification of a run and does not re-simulate.

## Script runtime and limit model

```text
source bytes ──► lexer ──► parser ──► typed AST
                                        │
                                 evaluator + HostContext
                                 ├─ SessionManager (factory-injected)
                                 ├─ Trace / session capture
                                 ├─ movie verify/compare (sessions)
                                 ├─ ScriptLimits counters
                                 ├─ Sandbox root (optional)
                                 └─ Clock (injected; wall deadline only)
                                        │
                                 ScriptRun { status, failure, source_sha256,
                                             statistics, prints, report fields }
```

Values are `Int(i64)`, `Bool` and `Text` (UTF-8; authored or concatenated text is
per-value bounded, while `read` text is bounded by its byte-length argument); the language is
line-oriented with `let`/`set`, `print`, `assert`, `if`/`else`, `repeat`, `while`,
expressions over the three types and a fixed builtin set for sessions, forks, inspection,
traces, TAS and report fields. The grammar, builtin table, semantics and exact limits are
normative in [ADR-0011](../adr/0011-tracing-and-scripting.md) §3.5; this page keeps the
model.

**Limit model.** Every limit is a validated field with a default and a maximum:
`source_bytes`, `operations`, `loop_iterations`, `variables`, `string_bytes`,
`report_bytes`, `steps`, `trace_records`, `snapshots` and optional `wall_time_ms`. Counters
are checked before the work they bound (statement/expression/loop/builtin operation, step
request, variable insertion, string/report/trace allocation, checkpoint creation), so a
limit failure is deterministic, typed and cheap. Source has its own bounded read; the
aggregate memory counter covers live variable, print and report text plus stored trace
records rather than relying on the host allocator's behavior. A `read(address, 4096)`
result is 8192 lowercase hex characters; it is charged to `string_bytes` when it becomes
live variable, print or report text, and the 4 KiB authored-text bound does not apply to
it. `step(n)` charges exactly `n`; `trace_capture` and `tas_verify` reject a request above
the remaining budget before work and then reserve the whole requested/recorded count, a
deterministic conservative upper bound when the backend halts or a replay diverges early.

**Wall time.** `wall_time_ms` is the only host-time input. It is checked against an
injected clock every 256 operations and at every statement boundary; production uses
`Instant`, tests use a fake clock. A single long backend call (`step`, `trace_capture`,
`tas_verify`) is bounded by the step/movie limits but is not preempted inside the call, so
the deadline can be observed shortly after it passes. Wall time can abort a run
(`WallTimeLimit`) but never changes a successful report: a passing run's bytes are identical
regardless of elapsed time.

**Report.** A run returns `ScriptRun` with a deterministic `passed`/`failed` status, a
stable failure code and message on failure, the source SHA-256, statistics, ordered print
lines, custom report fields and the backend id. An M5 experiment is exactly one script run
and its report; there is no separate experiment store, database or transport in v1.

**Factory injection.** The script library accepts an `Arc<dyn BackendFactory>`; it never
names a concrete backend. The CLI constructs the synthetic factory by default or
`MegadriveFactory` with `--rom`. TAS builtins decode movies through `chronofork-sessions`
and the injected factory, so the same script source can target either verified slice
without a library change.

## Sandbox boundary

Scripts have no ambient resources: no host environment variables, network, process
spawning, dynamic loading, randomness, clock, locale or filesystem except the file
builtins gated by an explicit `--sandbox <DIR>` root. The root is canonicalized once; a
path must be relative, non-empty, free of `.`/`..`/NUL components, resolve under the root,
and be a regular file for reads (or a regular file/absent for writes). Reads are bounded
by the trace/movie size limits, and failures never echo a host path into a report field.
Without a sandbox, file builtins fail with `SandboxUnavailable` while the remaining
builtins still run.

This is a **capability boundary for deterministic experiments, not an OS security
boundary** against hostile native code: the DSL cannot call native code, but the
interpreter shares the host process. `NFR-SEC-003` is satisfied for the in-process DSL by
that limitation plus the absence of ambient capabilities; it is not a process-isolation
claim. See [ADR-0011](../adr/0011-tracing-and-scripting.md) §3.5 and the
[non-functional requirements](../requirements/non-functional.md).

## Determinism and non-perturbation

- Trace records contain emulated ticks only. No wall-clock, pid, path, hostname, locale,
  random value or entropy enters trace bytes, TSV, comparisons or successful reports.
- Capture calls `inspect`/`read_memory` (side-effect-free) and `step`; it never mutates
  canonical state through a snapshot/write path. Installing a backend `DebugConfig` is
  non-canonical, as M4 already proved.
- Trace/trigger/report state is absent from snapshots, state hashes, configuration hashes,
  payload versions and movie bytes; `restore` does not rewind it.
- The executable invariant is a differential comparison at the same tick: state hash,
  configuration hash, snapshot payload bytes/version and later movie bytes are identical
  with tracing off and on (M5-A35 of the [acceptance
  plan](../quality/m5-tracing-and-scripting-acceptance.md)).
- Two runs of the same capture/script with the same inputs produce byte-identical trace
  files, TSV exports and script reports; wall time may only abort, never alter a passing
  result.

## CLI surfaces

Both command groups print deterministic `record=<kind>` blocks of `key=value` fields
separated by one blank line; diagnostics and path notices go to stderr prefixed
`chronofork:`; typed failures exit 1 with empty stdout. A trace comparison that finds a
divergence exits 0 because the comparison completed (the `identical`/`reason` fields carry
the result); a script-level failed run exits 1 with its report printed and empty stderr.
No command prints host time, pids or paths in stdout, and no command re-implements a codec,
matcher or limit.

```text
chronofork trace capture [--rom <FILE>] [--steps <N>] ... --out <FILE>
chronofork trace info --file <FILE>
chronofork trace query --file <FILE> [--from-tick <T>] ... [--limit <N>]
chronofork trace compare --left <FILE> --right <FILE>
chronofork trace export --file <FILE> [--format tsv|canonical] --out <FILE>
chronofork script run --file <FILE> [--rom <FILE>] [--sandbox <DIR>] [limits...]
```

`trace capture`'s `steps` field is the captured tick delta `end_tick - start_tick`, not a
separate executed-step counter; both acceptance backends advance one tick per executed
step. Unselected kind groups are reported as `*_enabled=0` fields rather than a record
table. No path, pid, host time or hostname appears in a report or on stdout; file-error
diagnostics may name the path on stderr. The exact flag grammar, defaults, field names and
exit-code rules are normative in [ADR-0011](../adr/0011-tracing-and-scripting.md) §3.6.
Process tests in
`crates/chronofork-cli/tests/trace.rs` and `script.rs` spawn the built binary and assert
streams, exit codes, exact fields, two-run determinism and the typed rejection matrix.

## Honest limits

- Trace sources are instruction, bus write, VDP port, controller read and interrupt phase
  only. DMA, audio and Z80 sources wait for H2/H3; there is no generic bus-read record, no
  instruction-fetch event without a watchpoint and no rendering/CRAM/VSRAM trace.
- Stops are instruction-boundary only; there is no cycle-accurate timing, mid-instruction
  stop or reverse trace.
- The canonical format is v1 only. There is no migration, compression, encryption, TSV
  import or cross-version compatibility.
- The script language is a small purpose-built DSL with three value types and a fixed
  builtin set. There are no user functions, arrays, maps, floats, imports, concurrency,
  network, process, environment access or persistent experiment store, and no
  Rhai/Luau/Lua/WASM/Python embedding.
- The sandbox is an in-process capability boundary, not process isolation or an audited
  hostile-code security boundary. Its file checks are metadata-then-operation and are not
  race-free against a concurrent host process that mutates the sandbox root.
- TSV export is bounded by the retained record count, not by `MAX_TRACE_BYTES`; canonical
  export is byte-capped at 64 MiB. Wall-time checks happen at statement boundaries and
  every 256 operations, so a single step/movie-bounded builtin is not preempted mid-call.
- Verification is limited to the deterministic synthetic backend and the minimal Mega
  Drive slice on Linux; no hardware completeness, commercial ROMs, game compatibility,
  cycle accuracy or cross-platform runtime evidence is claimed.
- M6 HTTP/MCP, the M7 GUI trace viewer, frame pacing and a host-time run loop are outside
  this page.

## References

- [ADR-0011: Tracing and Scripting](../adr/0011-tracing-and-scripting.md) — normative
  contract (format, grammar, limits, CLI, deferred scope).
- [M5 tracing/scripting acceptance plan](../quality/m5-tracing-and-scripting-acceptance.md)
  — executable rows and evidence rules.
- [Functional requirements](../requirements/functional.md) — `FR-TRACE-001/002`,
  `FR-SCRIPT-001`.
- [Non-functional requirements](../requirements/non-functional.md) — `NFR-SEC-003`.
- [Backend model](backend-model.md), [session model](session-model.md), [execution
  model](execution-model.md), [architecture overview](overview.md).
- [Mega Drive slice](megadrive-slice.md) — the only event-producing backend in M5.
- [ADR-0002](../adr/0002-backend-boundaries.md), [ADR-0003](../adr/0003-durable-session-storage.md),
  [ADR-0004](../adr/0004-tas-movie-format.md), [ADR-0010](../adr/0010-debugging-depth.md)
  — inherited contracts.
- [Quality gates](../quality/quality-gates.md), [testing
  strategy](../quality/testing-strategy.md), [anti-slop policy](../quality/anti-slop-policy.md).
- [Roadmap Stage 5](../roadmap/roadmap.md#stage-5--tracing-and-scripting), [M5
  milestone](../roadmap/milestones.md#m5--tracing-and-scripting).
