# M5 Tracing and Scripting — Executable Acceptance Plan

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** **Executed and independently verified.** Every row M5-A1–M5-A39 is
  `executed - passed`. The final independent verdict is **T12 PASS (2026-11-16)** after
  the T11 repair round. The original T7 verification returned `needs_revision` and is
  **not** the promotion basis; its row executions remain round-1 evidence only.
- **Companion contract:** [ADR-0011: Tracing and
  scripting](../adr/0011-tracing-and-scripting.md) — **Accepted** (owner-approved
  2026-11-16 by the explicit Stage 5 implementation directive). The ADR is the normative
  contract for the trace format, capture semantics, script grammar, limits, sandbox, CLI
  grain and deferred scope; this plan defines how it is proven.
- **Milestone:** [M5 — Tracing and scripting](../roadmap/milestones.md#m5--tracing-and-scripting)
  (`Complete`; merged to `main` at `0c4e9f0`, PR #4; verified on
  `agent/m5-tracing-scripting` at base `84f1476` before merge); roadmap stage [Stage 5 —
  Tracing and scripting](../roadmap/roadmap.md#stage-5--tracing-and-scripting).

- **Related:** [tracing and scripting architecture](../architecture/tracing-and-scripting.md),
  [backend model](../architecture/backend-model.md), [execution
  model](../architecture/execution-model.md), [session model](../architecture/session-model.md),
  [Mega Drive slice](../architecture/megadrive-slice.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [testing strategy](testing-strategy.md),
  [quality gates](quality-gates.md), [anti-slop policy](anti-slop-policy.md), [oracle
  differential evidence](../research/oracle-differential-evidence.md), [research: TAS and
  debugging](../research/tas-and-debugging.md).

> **Final verification record (2026-11-16).** The independent verifier T12 re-executed all
> 36 test-row commands (M5-A1–A34 plus the two A35 parts) with their exact plan commands on
> the then-uncommitted, implementation-complete tree at branch `agent/m5-tracing-scripting`,
> HEAD/base `84f1476466c14e72f911689845c390173982fd81` before merge: all exit 0, 0 failed,
> 0 ignored, every filter matched at least one test. A36–A39 were re-verified by their component commands (demos,
> artifacts, M1 oracle rerun, full gate and capability audit). The full gate
> `./scripts/check.sh` exited 0 with **57 targets / 662 passed / 0 failed / 0 ignored**
> against the pre-M5 baseline of 45 targets / 596 passed; rustdoc `-D warnings` passed;
> the contract commands (CLI script target 3, CLI package 117, fmt, Clippy) exited 0. Both
> demos were 3/3 byte-identical (`9b4dd564…`, `912a5bb6…`, empty stderr), baseline
> artifacts were 37/37 OK, trace capture bytes were unchanged (`e39cb42f…`, `5afe37…`),
> the M1 oracle rerun matched 3/3 with schema `chronofork.oracle-diff.v2`, the
> non-perturbation probe made 734 comparisons with 0 mismatches, and the M1 state anchors
> passed. T7's earlier round returned `needs_revision` with findings F1–F10; T11 repaired
> every finding and T12 independently confirmed each fix before this promotion. T10 then
> reviewed the integrated result and returned PASS. The authoritative dated gate block is
> the 2026-11-16 M5 block in [quality gates](quality-gates.md).
> Raw row and gate evidence is under the ignored `.agents/notes/m5/t12/` tree
> (`rows/`, `gate/`, `demos/`, `audit/`, `t12-verification.md`).

## 1. Status rules

1. **`planned - not yet executed`** means the row has a target file/test, exact command,
   expected observation and evidence path, but no executed result and no independent
   verifier record. At design time every row started in that state; the final T12
   re-execution and this promotion are what changed them to `executed - passed`.
2. A row becomes **`executed - passed`** only when the exact command, exit code, tree
   state (branch, base commit, HEAD and any uncommitted paths), test summary, observed
   values and ignored evidence artifact are recorded together and an independent verifier
   returns PASS. The authoritative dated gate block belongs in [quality
   gates](quality-gates.md); this plan links to it and does not copy volatile counts.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact, changed baseline byte or unverified claim. A failed row blocks M5 and
   is never converted into a skip, an `#[ignore]`, a relaxed assertion, a narrower claim
   or a deleted test after the fact.
4. A red test may exist in the worktree before its implementation only when it fails for
   the expected typed reason; no commit is authorized. A test that cannot run is not
   acceptance evidence. A filter that matches zero tests is a failed row, not a pass.
5. A row may not be satisfied by an existing M1–M4 capability. A green pre-M5 suite is a
   regression precondition, never M5 acceptance.
6. The M1–M4 canonical artifacts are a hard non-regression precondition: snapshot payload
   bytes/version, state hashes, configuration hashes, movie bytes, durable-store hashes,
   both demo outputs and the M1 oracle evidence must stay byte-identical to the recorded
   baseline. Any unexplained difference stops the task and is reported, never explained
   away.
7. Acceptance is limited to the deterministic synthetic backend and the minimal Mega
   Drive slice on the Linux acceptance platform. No cycle accuracy, mid-instruction stop,
   hardware completeness, commercial-ROM, game compatibility, UI, HTTP, MCP or
   cross-platform runtime claim is made.
8. Implementation is not verification. Only the independent verifier execution under §5
   produces PASS; the documentation owner records the executed results afterwards.

## 2. Scope, definitions and evidence

### 2.1 Checkout and baseline

- **Worktree:** `/home/stealthc/projects/ChronoFork/.agents/worktrees/m5-tracing-scripting`.
- **Branch:** `agent/m5-tracing-scripting`; **base:** `main` at
  `84f1476466c14e72f911689845c390173982fd81`.
- **Baseline run (captured before any M5 source edit):**
  `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` exit 0; **45 test targets, 596 passed,
  0 failed, 0 ignored**. Raw evidence: `.research/m5/baseline/gate.log`,
  `test-results.txt`, `git-state.txt`.
- **Demo stdout SHA-256:** `demo` =
  `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652`
  (3/3 byte-identical, empty stderr); `megadrive demo` =
  `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996` (3/3 identical,
  empty stderr). Evidence: `.research/m5/baseline/demo-1..3.txt`, `md-1..3.txt`,
  `demo-hashes.txt`.
- **Movie:** baseline `demo.cfmovie` SHA-256
  `160453c69fb157271b0f7b28d029ad796b01edbdb46b4f97461e91fef82c4477`.
- **Mega Drive state:** `md-state.bin` SHA-256
  `e42c66edec6d5baa0ba38abdbaae606b61a55915bcc7cfce7232a3324dfc2ceb`; M1 slice
  `md-trace.tsv` SHA-256 `b15d95bbeb87184824abebf19a0f368e9554587d49d3b7ad21262362f8484508`.
- **Durable store:** `.research/m5/baseline/store/…` hashes are recorded in
  `artifacts.sha256`: store marker `24fc2f54…`, session metadata `2ab51705…`, snapshot-1
  `checkpoint.cfsnap` `4de16840…` and snapshot-2 `checkpoint.cfsnap` `062cb9c4…`; these
  baseline checkpoint hashes must not change.
- **M4 state-hash anchors:** reset
  `7dcab59e97446accafdd27a3fb5f6b4053579b0337af9fea1012670e97c31304`, five steps
  `d354cb249fb3a38c1edde145b8183ebe9fe17e8497e6ed0cb966315b32d92c7b`.
- **Implementation tree:** the changes in the worktree on `agent/m5-tracing-scripting`
  (M5 crates and docs). At verification time they were uncommitted, and the verifier
  recorded `git status --short --branch` and the tree state exactly. The owner's later
  delivery instruction authorized commit/push/PR/merge; M5 was merged at `0c4e9f0`
  (PR #4) with the same tree.
- **Raw evidence (executed):** the ignored `.agents/notes/m5/t12/` tree with
  `t12-verification.md`, `rows/` (all 36 test-row commands), `gate/` (check log, target
  matrix, contract commands, demos/artifacts/trace, anchors), `demos/` and `audit/`
  (non-perturbation, oracle rerun, capability/status, scope audit), plus `t10-review.md`
  and `t11-repair.md`. The `.research/m5/baseline/` tree holds the pre-M5 baseline.
  Paths are ignored and never committed.
- **Environment:** run Cargo with `CARGO_HOME=/tmp/t7-cargo`; never `cargo clean`. The
  M5 crates add no external dependency beyond the existing workspace graph.

### 2.2 Definitions

- **Trace:** a `chronofork_trace::Trace` value: validated metadata plus a vector of
  retained records. It is a host observation artifact, never canonical state.
- **Trace record:** one typed instruction, bus-write, VDP-port, controller-read or
  interrupt-phase event with an absolute `index` and an emulated `Tick`.
- **Canonical trace file:** the v1 `CFTRACE1` byte stream defined by
  [ADR-0011](../adr/0011-tracing-and-scripting.md) §3.2; the only trace import format.
- **Trigger:** one start (`Immediate`, `Pc`, `Tick`) and one stop (`None`, `Pc`, `Tick`)
  boundary predicate; stop wins when both match.
- **Ring / `first_index` / `dropped`:** a fixed-capacity buffer keeping the newest records;
  `dropped` counts evicted records and `first_index == dropped` is the first retained
  absolute index.
- **Non-perturbation:** at the same tick, canonical state hash, configuration hash,
  snapshot payload bytes/version and later movie bytes are identical with tracing off and
  on.
- **Script run:** one `chronofork_script` evaluation of a bounded source producing a
  deterministic `ScriptRun`; the M5 "experiment" artifact.
- **Sandbox root:** the canonicalized `--sandbox <DIR>` directory; the only filesystem
  visible to file builtins.
- **Injected clock:** the script wall-time source; production uses `Instant`, unit tests
  use a fake clock so enforcement is deterministic.
- **Host trace surface:** the `chronofork trace`/`chronofork script` capabilities; they
  are host-provided and never make `Capabilities::tracing` true.

### 2.3 Design-time decisions taken here

The following points resolve ambiguities in the brief and are normative through
[ADR-0011](../adr/0011-tracing-and-scripting.md):

- `chronofork trace capture` and `chronofork trace export` **require `--out`**; stdout
  stays a pure `key=value` report and never receives binary or TSV payload.
- `chronofork script --max-memory-bytes` maps to `ScriptLimits::string_bytes`;
  `trace_records` and `snapshots` keep their documented defaults (no CLI override in v1).
- `TraceRecord::summary()` is the documented unprefixed `<kind> index=… tick=… <fields>`
  one-line form, reused by `trace query`/`trace compare`.
- Text authored in a literal or produced by concatenation is bounded to 4 KiB per value;
  `read(address, length)` returns `2 * length` lowercase hex characters (up to 8192 for
  length 4096), and that runtime-produced text is charged to `string_bytes` when it becomes
  live variable, print or report text.
- Integer literals are decimal unless prefixed `0x`/`0X`; both forms must fit `i64`.
- `program := stmt+`: an empty or comment-only source is the typed `parse.empty_program`
  rejection at line 1, column 1.
- `chronofork trace capture`'s `steps` field is the tick delta `end_tick - start_tick`;
  both acceptance backends advance one tick per executed step.
- `trace_capture` reserves the requested step count and `tas_verify` reserves the movie's
  recorded step count before executing or replaying; the charge is a conservative upper
  bound when the backend halts or the replay diverges.
- `Trace::compare` walks the union of retained absolute-index windows; the first missing
  or different index is the divergence. `identical` means records are identical;
  `metadata_equal` is reported separately.
- A backend that advertises `debug_events` but returns no `debug_access` is a typed
  `EventObservationUnavailable`; an event-ring overflow between drains is a typed error.
- Script variables are program-global (blocks do not scope); string literals support
  `\\`, `\"`, `\n`, `\r`, `\t`; conditions are strict `Bool`.
- The sandbox is a capability boundary, not an OS security boundary; this limit is
  documented rather than overclaimed.

## 3. Row map

| Milestone item | Acceptance rows |
|---|---|
| Canonical format round-trip, rejection and metadata bounds | M5-A1 – M5-A3 |
| Instruction/device capture, triggers and circular retention | M5-A4 – M5-A10 |
| Queries, comparison and TSV export | M5-A11 – M5-A13 |
| `chronofork trace` CLI surfaces | M5-A14 – M5-A18 |
| Script lexer/parser/type/arithmetic/variables/control flow | M5-A19 – M5-A23 |
| Script session/fork/inspection/trace/TAS/report APIs | M5-A24 – M5-A28 |
| Script limits (including fake-clock wall time) and sandbox | M5-A29 – M5-A31 |
| `chronofork script` CLI surfaces | M5-A32 – M5-A34 |
| Non-perturbation and canonical regression | M5-A35 – M5-A36 |
| M1 oracle, full gate/rustdoc and claim honesty | M5-A37 – M5-A39 |

## 4. Acceptance rows

Every row below is **`executed - passed`** after the final independent T12
re-verification (2026-11-16); the T7 round-1 execution was `needs_revision`, so the T11
repair and T12 re-execution are the promotion basis. Test target paths and test names are
part of the contract. All commands ran in the acceptance worktree with
`CARGO_HOME=/tmp/t7-cargo`; a package/test filter matching zero tests would have failed the
row.

### M5-A1 — Canonical round-trip and byte determinism

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/format.rs`, tests
  `round_trip_preserves_all_record_kinds_and_metadata` and
  `encoding_is_byte_deterministic`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test format -- round_trip_preserves_all_record_kinds_and_metadata encoding_is_byte_deterministic`
- **Expected:** a constructed `Trace` containing all five record kinds, boundary metadata
  strings, nonzero `first_index == dropped` and a valid state hash round-trips through
  `to_bytes`/`from_bytes` with equal metadata and equal record values in order; encoding the
  same value twice is byte-identical and starts with `CFTRACE1`; a separately decoded copy
  of the same bytes re-encodes identically; the record summary is the documented
  one-line form with the record's fields in order. No host
  time, path, locale or entropy appears in the bytes.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A01.log` with the hex prefix, both
  byte strings and the round-trip values.

### M5-A2 — Canonical rejection matrix

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/format.rs`, test
  `rejects_malformed_canonical_input`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test format -- rejects_malformed_canonical_input`
- **Expected:** every rejection class from [ADR-0011](../adr/0011-tracing-and-scripting.md)
  §3.2.3 returns its typed `TraceError` without panic: wrong magic; `version` 0/2/65535;
  unknown flag bits; invalid `stop_reason`; truncated magic/header/string/record; trailing
  bytes; empty, oversized or non-UTF-8 backend strings; unknown kind codes 5..=255;
  invalid bus/VDP widths, controller port, interrupt level and phase; `record_count` above
  `capacity`, above `MAX_TRACE_RECORDS` or inconsistent with the body; capacity 0 or above
  `MAX_TRACE_CAPACITY`; `first_index != dropped`; non-contiguous and decreasing indexes;
  decreasing ticks and ticks outside `start_tick..=end_tick`; `end_tick < start_tick`;
  body above `MAX_TRACE_BYTES`. A forged header declaring a huge record count over a short
  buffer returns a typed error before a large allocation. No input is partially accepted
  or reinterpreted.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A02.log` listing one crafted input,
  expected code and observed code per class.

### M5-A3 — Metadata and construction bounds

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/format.rs`, test
  `trace_construction_bounds_are_enforced`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test format -- trace_construction_bounds_are_enforced`
- **Expected:** `Trace::new` accepts boundary values (backend 1 and 128 bytes, version 1
  and 64 bytes, capacity 1 and 1,000,000, empty trace with `first_index == dropped`,
  `end_tick == start_tick`) and rejects empty/oversized strings, capacity 0 or above
  `MAX_TRACE_CAPACITY`, `record_count > capacity`, `end_tick < start_tick`,
  `first_index != dropped`, out-of-order indexes/ticks and out-of-range record fields with
  typed errors; the error message is deterministic and contains no host data.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A03.log` with boundary accept/reject
  values.

### M5-A4 — Instruction capture on the synthetic backend

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `captures_instruction_records_on_the_synthetic_backend`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- captures_instruction_records_on_the_synthetic_backend`
- **Expected:** capture of `N` steps from a fresh synthetic backend yields exactly `N`
  contiguous instruction records (unless a documented halt/step limit stops earlier),
  each `tick` equal to the pre-step tick, `pc` equal to the inspected PC and `opcode`
  equal to the big-endian 2 bytes at that PC; indexes are 0..N-1 for a ring without
  overflow; `state_hash` equals the backend's capture-start state hash; `events_observed`
  is false; two independent captures produce identical bytes; the stop reason is
  `step_limit`.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A04.log` with the record table, start
  hash and two byte hashes.

### M5-A5 — Instruction capture on the minimal Mega Drive slice

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `captures_instruction_records_on_the_megadrive_slice`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- captures_instruction_records_on_the_megadrive_slice`
- **Expected:** for a test ROM, instructions are recorded with the 68000 PC of the
  instruction about to execute and the big-endian opcode word; a multi-word instruction
  produces one record for its first word and its extension words are not separate
  instruction records; record ticks are the pre-step ticks and non-decreasing;
  `events_observed` is true because the slice advertises `debug_events`; two runs are
  byte-identical; no rendering, DMA, Z80 or audio source appears.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A05.log` with the opcode/PC table and
  repeat hashes.

### M5-A6 — Bus-write and controller-read records

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `captures_bus_write_and_controller_records`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- captures_bus_write_and_controller_records`
- **Expected:** a work-RAM byte write produces one `BusWrite` record with the exact
  address, value and width and the pre-step tick; a controller data-register read produces
  one `ControllerRead` record with port and value; a VDP port write is a `VdpPort` record
  and is not duplicated as `BusWrite`; a host `read_memory` produces no trace record and
  does not change the ring; disabling the kind drops the record without altering indexes of
  enabled records; repeating the run yields identical bytes.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A06.log` with matched and negative
  controls.

### M5-A7 — VDP port records

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `captures_vdp_port_records`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- captures_vdp_port_records`
- **Expected:** VDP control/data-port byte and word reads and writes produce `VdpPort`
  records with exact `offset`, `value`, `width` (1 or 2), `write` flag and pre-step tick;
  a width outside 1..=2 cannot be constructed; the record order within a step is
  deterministic; two runs are identical.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A07.log` with the read/write table.

### M5-A8 — Interrupt records

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `captures_interrupt_records`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- captures_interrupt_records`
- **Expected:** an interrupt-ROM run records `InterruptRequested`, then `Delivered` and
  `Acknowledged` for the delivered level, in that order, with the level and the pre-step
  ticks from `DebugEvent`; a masked run records the request with no delivery or
  acknowledgement; only the delivered level is acknowledged and a coalesced request is not
  duplicated; level is 1..=7 and phase is one of the three typed variants; repeat runs are
  byte-identical.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A08.log` with both sequences.

### M5-A9 — Start/stop triggers, ticks and stop reasons

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `honours_start_and_stop_triggers`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- honours_start_and_stop_triggers`
- **Expected:** `Immediate` triggers and records from the first step; `Pc`/`Tick` start
  triggers begin inclusively at the named boundary and `triggered=1`; a stop `Pc` stops
  **before** executing the instruction at that PC and writes no record for it; a stop
  `Tick` stops before the first step whose pre-step tick is at or above it; a stop and a
  start match at the same boundary stop without executing and preserve the prior
  `triggered` value; `steps` fewer than requested yields `StopCondition` as success;
  `Halted` and `StepLimit` are reported in their rows; `start_tick`/`end_tick` bound every
  record tick and `end_tick >= start_tick`.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A09.log` with each trigger/stop case
  and the observed boundary record.

### M5-A10 — Circular retention, dropped, first_index and event overflow

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `circular_retention_reports_dropped_and_first_index`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- circular_retention_reports_dropped_and_first_index`
- **Expected:** capturing more steps than a small capacity retains exactly `capacity`
  records (the newest ones), `dropped == first_index == steps - capacity`, retained
  indexes are contiguous and absolute, and the retained values equal the tail of an
  equivalent large-capacity capture; encoding/decoding preserves `dropped`/`first_index`;
  a capacity larger than the step count yields `dropped == 0`; a deliberately tiny event
  capacity whose backend ring overflows between drains produces the typed event-overflow
  error instead of silent loss; repeat runs are byte-identical.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A10.log` with counts, indexes and the
  overflow error.

### M5-A11 — Query filters and limits

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/query.rs`, tests
  `query_filters_by_tick_kind_pc_and_address` and
  `query_limit_and_bounds_are_validated`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test query -- query_filters_by_tick_kind_pc_and_address query_limit_and_bounds_are_validated`
- **Expected:** inclusive `from_tick`/`to_tick`, kind masks, `pc` (instruction only) and
  numeric `address` (bus-write address, VDP offset, controller port) return exactly the
  matching records in order; `matched` counts all matches while `returned` is capped by
  `limit` and `truncated = matched > returned`; `limit=0` and `limit > MAX_QUERY_LIMIT`
  are typed errors; `from > to` is a typed error; a zero-match query returns
  `matched=0`, empty records, `truncated=false`; the same query repeated is identical.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A11.log` with each filter's matched/
  returned counts.

### M5-A12 — Comparison first divergence (record and length)

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/query.rs`, test
  `compare_reports_record_and_length_first_divergence`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test query -- compare_reports_record_and_length_first_divergence`
- **Expected:** identical traces report `identical=true`, `metadata_equal=true` and no
  divergence; a single changed record reports the first differing absolute index/tick with
  `reason=Record` and both records; one extra retained record reports
  `LeftMissing`/`RightMissing` at the first extra index; differences after a non-zero
  `first_index` are compared by absolute index, not vector position; a metadata-only
  difference reports `metadata_equal=false` while record identity and `identical` are
  unaffected; comparison is deterministic and never re-simulates.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A12.log` with each comparison result.

### M5-A13 — TSV export layout and determinism

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/query.rs`, test
  `tsv_export_is_fixed_and_deterministic`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test query -- tsv_export_is_fixed_and_deterministic`
- **Expected:** `to_tsv` emits the documented comment header followed by the fixed
  `index	kind	tick	pc	address	value	width	extra` table header and one row per
  record; widths, hex casing/padding, `-` placeholders, `read`/`write` and phase extras
  match [ADR-0011](../adr/0011-tracing-and-scripting.md) §3.2.5; an empty trace still emits
  the full header; two exports are byte-identical; the output contains no path, host time
  or locale-dependent formatting; there is no TSV import API.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A13.log` with the exact header and a
  sample of rows plus two hashes.

### M5-A14 — CLI trace capture (synthetic and Mega Drive)

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/trace.rs`, tests
  `trace_capture_writes_a_valid_canonical_file` and
  `trace_capture_targets_the_megadrive_slice`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test trace -- trace_capture_writes_a_valid_canonical_file trace_capture_targets_the_megadrive_slice`
- **Expected:** `chronofork trace capture --out <file>` exits 0, writes a file beginning
  `CFTRACE1` that `trace info` re-reads with identical metadata, prints exactly one
  `record=trace_capture` block with the documented fields (including `kinds`, the four
  `*_enabled` flags, `bytes` and `sha256`), and writes nothing to stderr; two runs are
  byte-identical and the file hashes match; the
  `--rom` variant selects the Mega Drive factory, records `events_observed=1`, and the
  synthetic default records `events_observed=0`; defaults are steps 1000, capacity 4096,
  event capacity 1024 and all kinds; `--input`, `--kinds` and trigger/stop flags change
  only the documented behavior.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A14.log` with stdout blocks, file
  hashes and the two-backend comparison.

### M5-A15 — CLI trace info and query

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/trace.rs`, test
  `trace_info_and_query_print_stable_blocks`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test trace -- trace_info_and_query_print_stable_blocks`
- **Expected:** `trace info` prints one `record=trace_info` block whose
  `format_version`, `backend`, `backend_version`, `state_hash`, ticks, capacity, records,
  `first_index`, `dropped`, `stop_reason`, `triggered`, `events_observed`, `bytes` and
  `sha256` match the file; `trace query` prints the summary block (including the four
  `*_enabled` flags for its effective `--kind` filter) then one
  `record=trace_record` block per record with only that kind's fields; filters, `--limit`
  and truncation match M5-A11; a zero-match query prints the summary only; two runs of
  each command are byte-identical; stdout contains no paths.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A15.log` with exact blocks and two-run
  hashes.

### M5-A16 — CLI trace compare

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/trace.rs`, test
  `trace_compare_reports_first_divergence`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test trace -- trace_compare_reports_first_divergence`
- **Expected:** comparing a file with itself prints `record=trace_compare`, `identical=1`,
  `metadata_equal=1`, `divergence=-1`, `reason=none` and exit 0; a trace with one changed
  record prints `identical=0`, the first divergence index/tick, `reason=record` and the
  `left`/`right` summaries; an extra retained record reports `left_missing`/`right_missing`
  at the first extra index; a metadata-only difference reports `metadata_equal=0` without a
  record divergence; unreadable/invalid files exit 1 with empty stdout and a
  `chronofork:` stderr; a divergent comparison is still exit 0.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A16.log` with both exit codes and
  streams.

### M5-A17 — CLI trace export

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/trace.rs`, test
  `trace_export_is_one_way_and_deterministic`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test trace -- trace_export_is_one_way_and_deterministic`
- **Expected:** `trace export --format canonical --out <file>` exits 0, writes bytes
  byte-identical to `Trace::to_bytes` (and to the source trace file) and prints
  `record=trace_export` with `format=canonical`, `records`, `bytes` and `sha256`; `--format
  tsv` writes exactly `Trace::to_tsv`; omitting `--out` is a typed usage error with empty
  stdout; an invalid input file is rejected before any output file is created; two runs
  are identical; no TSV import command exists.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A17.log` with byte comparisons and the
  missing-`--out` rejection.

### M5-A18 — CLI trace help, usage and rejection matrix

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/trace.rs`, test
  `trace_help_and_typed_rejections_follow_the_contract`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test trace -- trace_help_and_typed_rejections_follow_the_contract`
- **Expected:** `chronofork trace --help` exits 0 and documents exactly the implemented
  subcommands/flags (including required `--out`, the `--kinds` grammar and defaults);
  `chronofork help` lists `trace`; missing/unknown subcommand, unknown flag, missing value,
  non-numeric/overflowing number, zero or oversized steps/capacity/event capacity/limit,
  empty/duplicate/unknown `--kinds`, two start flags, two stop flags, missing `--out`,
  missing/unreadable/empty/oversized/corrupt trace file and a directory output path each
  exit 1 with empty stdout and a deterministic `chronofork:` stderr; no panic and no
  partial output file.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A18.log` with every command, exit code
  and stderr.

### M5-A19 — Script lexer/parser errors with line and column

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/language.rs`, test
  `lexer_and_parser_report_line_and_column`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test language -- lexer_and_parser_report_line_and_column`
- **Expected:** unknown token, unterminated string, unknown string escape, missing `=`,
  missing `)`, missing/extra `}`, missing newline where required, unexpected end of file,
  stray `else`, empty source, comment-only source and blank-line handling each return a
  typed parse error with a 1-based line and column and a stable code; a source above
  `source_bytes` is a distinct typed limit error before lexing; no panic and no partial
  AST evaluation.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A19.log` with source snippet, expected
  position and observed position per case.

### M5-A20 — Script type, name, arity and scope errors

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/language.rs`, test
  `type_and_name_errors_are_typed`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test language -- type_and_name_errors_are_typed`
- **Expected:** strict `Bool` conditions for `if`/`while`/`!`/`&&`/`||`; `repeat` requires
  `Int`; equality requires equal types; ordering rejects `Bool`; arithmetic rejects
  `Text`; unknown variable in an expression or `set`; duplicate `let`; unknown builtin;
  wrong arity; wrong argument type; `assert`/`print` type errors; all are typed errors with
  the statement line and stable codes, no truthiness, no coercion, no panic. A valid script
  using each accepted type combination still passes in the same target.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A20.log` with one rejected and one
  accepted case per rule.

### M5-A21 — Checked script arithmetic

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/language.rs`, test
  `arithmetic_is_checked_i64`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test language -- arithmetic_is_checked_i64`
- **Expected:** `i64` overflow on `+`, `-`, `*` and unary `-` is a typed
  `ArithmeticOverflow`; division and modulo by zero are typed; `i64::MIN / -1` is typed;
  `+` concatenates two `Text` values exactly (UTF-8 preserved) and rejects mixed types;
  integer division truncates toward zero and modulo follows Rust `i64` semantics
  deterministically; every failure is a failed run with a stable code and no panic.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A21.log` with the boundary operands and
  observed codes.

### M5-A22 — `let`/`set`, variables and the variable limit

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/language.rs`, test
  `let_set_and_variables_are_flat`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test language -- let_set_and_variables_are_flat`
- **Expected:** `let` defines and `set` updates the same flat program-global environment;
  `let` on an existing name and `set` on an unknown name are typed; a variable defined
  inside an `if`/`repeat`/`while` block remains visible after the block (no block scope);
  assigning `Int`, `Bool` and `Text` works and keeps the value type; exceeding the
  `variables` limit is a typed `VariableLimit`/limit failure before insertion; the same
  program repeated produces an identical report.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A22.log` with the scope trace and the
  limit failure.

### M5-A23 — `if`/`repeat`/`while` determinism

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/language.rs`, test
  `if_repeat_and_while_are_deterministic`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test language -- if_repeat_and_while_are_deterministic`
- **Expected:** `if`/`else if`/`else` select exactly one branch; `repeat 0` executes zero
  iterations and `repeat N` exactly N; `while` re-evaluates the condition each iteration
  and terminates at the expected count; nested blocks and assertions produce the documented
  print/report output; `repeat -1` and a count above `max_loop_iterations` are typed before
  executing; a `while` whose condition never becomes false hits the loop/operation limit
  with a stable code; two runs of each valid program produce byte-identical reports.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A23.log` with branch/iteration counts
  and repeat hashes.

### M5-A24 — Session/inspection builtins

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/host.rs`, test
  `session_inspection_builtins_match_the_session`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test host -- session_inspection_builtins_match_the_session`
- **Expected:** `open()` creates the configured manager and one root session and returns
  its id; a second `open()` is typed `AlreadyOpen`; `step(n)` executes exactly `n` steps
  with neutral input, returns the new tick and counts against `max_steps`; `tick()`,
  `state_hash()` (64 lowercase hex matching the session inspection), `register("pc")` and
  `read(address, length)` (lowercase hex) match the session at the same point; `reset()`
  returns 0 and restores tick 0/creation state; unknown register, length 0/4097,
  out-of-window read and exceeding `max_steps` are typed and execute no partial step.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A24.log` with session/library values
  side by side.

### M5-A25 — Fork, checkpoint, restore and select

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/host.rs`, test
  `fork_checkpoint_restore_and_select_preserve_isolation`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test host -- fork_checkpoint_restore_and_select_preserve_isolation`
- **Expected:** `fork()` returns a new current session id with the same tick/state hash as
  the source checkpoint and the source is unchanged by later child steps; `checkpoint()`
  returns a snapshot id and `restore(id)` restores the exact tick/state hash; `select(id)`
  switches the current session (including back to the source) and returns 0; unknown
  session/snapshot ids are typed; the `snapshots` limit rejects one checkpoint beyond the
  bound before creation; repeated runs produce identical report values and session ids.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A25.log` with before/after hashes and
  the isolation check.

### M5-A26 — Script trace builtins

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/host.rs`, test
  `trace_builtins_capture_export_and_compare`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test host -- trace_builtins_capture_export_and_compare`
- **Expected:** `trace_capture(steps, capacity)` captures from the current session with
  neutral input, advances it, stores the latest trace and returns the retained count;
  `trace_records()` returns that count (0 before any capture); `trace_export(path)` writes
  canonical bytes inside the sandbox and returns the byte count; `trace_compare(path)`
  returns `-1` for an identical file and the first divergence index for a changed one;
  export/compare without a stored trace or without a sandbox are typed errors; invalid
  steps/capacity and a `trace_records` budget overrun are typed before advancing;
  the same script run repeated is byte-identical.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A26.log` with counts, byte count and
  comparison results.

### M5-A27 — Script TAS builtins

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/host.rs`, test
  `tas_builtins_verify_and_compare_movies`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test host -- tas_builtins_verify_and_compare_movies`
- **Expected:** a synthetic movie fixture generated by the test verifies through
  `tas_verify(path)` as `true`; a corrupted expectation returns `false` (or a typed decode
  error for a malformed container), never a panic; `tas_compare(left, right)` returns `-1`
  for equal recorded outcomes and the first divergent recorded tick for different ones;
  missing files, paths outside the sandbox, unsupported movie versions and truncated
  headers are typed errors; the script library uses only the injected factory and
  `chronofork-sessions`, never a concrete backend name; repeated runs are identical.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A27.log` with both movies and results.

### M5-A28 — Print/report/text builtins and pass/fail report determinism

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/host.rs`, tests
  `report_print_and_text_builtins` and
  `script_report_is_passed_failed_and_deterministic`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test host -- report_print_and_text_builtins script_report_is_passed_failed_and_deterministic`
- **Expected:** `print` appends ordered lines; `report(key, value)` accepts `Int`/`Bool`/
  `Text` with a key matching `[A-Za-z_][A-Za-z0-9_]*` and at most 64 bytes, rejects
  duplicate/oversized/invalid keys and returns 0; custom fields are reported sorted by
  key; `len` counts UTF-8 bytes (multi-byte text included); `str` renders
  decimal/`true`/`false`/text deterministically; `fail` produces a failed run with a stable
  code and no panic; a passing run reports `status=passed` with source SHA-256, statements,
  operations, assertions, steps, variables, report_bytes and trace_records; a failing
  assertion and a failing `fail()` report `status=failed` with stable code/message; two
  runs of each program are byte-identical; no wall time, pid or path appears.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A28.log` with passing/failing reports
  and repeat hashes.

### M5-A29 — Execution, memory, step, snapshot and trace limits

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/limits.rs`, test
  `execution_and_memory_limits_are_enforced`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test limits -- execution_and_memory_limits_are_enforced`
- **Expected:** each limit is enforced with a typed, stable failure **before** the bounded
  work: `source_bytes` (oversized file/source), `operations` (statement/expression/loop/
  builtin counter), `loop_iterations` (`repeat`/`while`), `steps` (`step` request aborts
  before any partial step), `variables`, `string_bytes` (aggregate live variable text +
  report text + stored trace bytes), `report_bytes`, `trace_records`, `snapshots`; a value
  at the maximum succeeds where the limit is a maximum and one above it fails; invalid
  `ScriptLimits` construction (zero where not allowed, above maximum, `wall_time_ms` only
  `Some`/`None`) is typed; every failure is a failed `ScriptRun`, not a panic, and no
  limit failure depends on host speed or allocator behavior.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A29.log` with one boundary accept and
  reject per limit.

### M5-A30 — Wall-time limit via injected fake clock

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/limits.rs`, test
  `wall_time_limit_uses_injected_clock`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test limits -- wall_time_limit_uses_injected_clock`
- **Expected:** a fake clock that never advances lets the same program pass and produce a
  report byte-identical to a run with the deadline disabled; a fake clock that crosses
  `wall_time_ms` aborts deterministically at the documented check points (every 256
  operations and statement boundaries) with a stable `WallTimeLimit` code and statistics;
  `None` and CLI `--wall-time-ms 0` disable enforcement; the report of a passing run
  contains no elapsed time and is independent of clock advances; no test depends on real
  elapsed time or host speed.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A30.log` with the fake-clock trace,
  abort statistics and passing-run hashes.

### M5-A31 — Sandbox path escapes and bounded file access

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-script/tests/sandbox.rs`, test
  `sandbox_rejects_escapes_and_bounds_reads`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-script --test sandbox -- sandbox_rejects_escapes_and_bounds_reads`
- **Expected:** without a sandbox, `trace_export`/`trace_compare`/`tas_verify`/
  `tas_compare` fail with `SandboxUnavailable` while non-file builtins still run; with a
  sandbox root, empty, absolute, `.`/`..`, NUL and symlink-escaping paths are typed
  rejections; a path resolving outside the root, a directory, a special file and an
  oversized read are typed; a relative path inside the root is accepted; export writes only
  inside the root and never leaves a partial file on rejection; failure messages and report
  fields never echo the host path; the root must exist and be a directory; non-file
  builtins are unaffected.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A31.log` with every rejected path and
  the accepted control.

### M5-A32 — CLI script run report, exit codes and streams

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/script.rs`, test
  `script_run_prints_report_and_uses_exit_codes`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test script -- script_run_prints_report_and_uses_exit_codes`
- **Expected:** a passing script exits 0, prints one `record=script_print` block per print
  (in order, escaped) then exactly one `record=script_report` block with `status=passed`,
  `failure=-`, `source_sha256`, `statements`, `operations`, `assertions`, `steps`,
  `variables`, `report_bytes`, `trace_records`, `backend` and sorted `report.<key>`
  fields, and writes empty stderr; a failing assertion or `fail()` exits 1, still prints
  the report with `status=failed` plus `failure`/`message` and empty stderr; missing
  `--file`, unreadable/oversized source, unknown flag, missing sandbox root and invalid
  limit values exit 1 with **empty stdout** and a `chronofork:` stderr; two runs are
  byte-identical; no path, pid or host time appears.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A32.log` with stdout/stderr/exit per
  case and two-run hashes.

### M5-A33 — CLI script limits, options and both backends

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/script.rs`, test
  `script_run_supports_limits_and_both_backends`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test script -- script_run_supports_limits_and_both_backends`
- **Expected:** each documented limit flag overrides its default and produces the typed
  failure at the same boundary as the library row (`--max-operations`,
  `--max-loop-iterations`, `--max-steps`, `--max-memory-bytes` = `string_bytes`,
  `--max-source-bytes`, `--max-report-bytes`, `--max-variables`, `--wall-time-ms`);
  zero is rejected for all except `--wall-time-ms 0`, which disables the deadline;
  `--rom` selects the Mega Drive factory and reports its backend id while the default is
  synthetic; `--sandbox` enables file builtins; defaults apply when flags are omitted;
  repeated runs are byte-identical.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A33.log` with each flag and the two
  backend report blocks.

### M5-A34 — CLI script help, usage and rejection matrix

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/script.rs`, test
  `script_help_and_typed_rejections_follow_the_contract`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test script -- script_help_and_typed_rejections_follow_the_contract`
- **Expected:** `chronofork script --help` exits 0 and documents only implemented
  subcommands/flags, defaults and the exit-code contract; `chronofork help` lists
  `script`; unknown subcommand/flag, missing value, non-numeric/negative/overflowing
  limit, zero where not allowed, missing/oversized/empty source and missing sandbox
  directory each exit 1 with empty stdout and deterministic `chronofork:` stderr; no
  panic; no help text advertises an unimplemented engine, import, network or GUI feature.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A34.log` with commands, exit codes and
  streams.

### M5-A35 — Non-perturbation of canonical state and artifacts

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-trace/tests/capture.rs`, test
  `capture_does_not_perturb_state_or_snapshot`; `crates/chronofork-sessions/src/session.rs`
  unit test `capture_trace_does_not_perturb_snapshot` (module target `chronofork_sessions`).
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-trace --test capture -- capture_does_not_perturb_state_or_snapshot` and `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib capture_trace_does_not_perturb_snapshot`
- **Expected:** two fresh backends stepped the same number of times with the same neutral
  input produce the same tick, state hash, configuration hash, snapshot payload
  bytes/version and inspect values whether one of them was captured or not; the session
  `capture_trace` entry point produces the same snapshot bytes/state hash at the same tick
  as a plain stepped session; trigger/event configuration is absent from the snapshot and
  hashes; a recorded movie generated after tracing is byte-identical to one generated
  without tracing; no backend `snapshot`/`restore`/`write_memory` is called by capture.
- **Evidence:** `.agents/notes/m5/t12/rows/M5-A35a.log` (trace capture non-perturbation)
  and `M5-A35b.log` (session capture_trace snapshot non-perturbation), with the per-tick
  off/on comparison and zero mismatches.

### M5-A36 — Demos, state, movie and durable store unchanged

- **Status:** `executed - passed`.
- **Target:** repository baseline artifacts and demo commands.
- **Command:** `for i in 1 2 3; do CARGO_HOME=/tmp/t7-cargo cargo run -q -p chronofork-cli -- demo > .agents/notes/m5/t12/demos/demo-$i.txt 2> .agents/notes/m5/t12/demos/demo-$i.err; CARGO_HOME=/tmp/t7-cargo cargo run -q -p chronofork-cli -- megadrive demo > .agents/notes/m5/t12/demos/md-$i.txt 2> .agents/notes/m5/t12/demos/md-$i.err; done` followed by `sha256sum` over the demo outputs, `.research/m5/baseline/demo.cfmovie`, `md-state.bin`, `md-trace.tsv` and the `store/` tree, compared with `.research/m5/baseline/artifacts.sha256`.
- **Expected:** all three `demo` runs are byte-identical to the baseline
  (`9b4dd564…`), all three `megadrive demo` runs are identical to the baseline
  (`912a5bb6…`), every stderr is empty; `demo.cfmovie` (`160453c6…`), `md-state.bin`
  (`e42c66ed…`), `md-trace.tsv` (`b15d95bb…`) and every durable-store/checkpoint hash
  (`24fc2f54…`, `2ab51705…`, `4de16840…`, `062cb9c4…`) match `artifacts.sha256`; the full
  gate in M5-A38 re-runs the M2 rehydration/restart suite; the M4 state-hash anchors
  (`7dcab59e…`, `d354cb24…`) still hold; any mismatch stops the row.
- **Evidence:** `.agents/notes/m5/t12/demos/` (the `--out` destination above) with all six
  runs and empty stderr; artifact and trace-byte checks in
  `.agents/notes/m5/t12/gate/demos-artifacts-trace.log`.

### M5-A37 — M1 oracle differential unchanged

- **Status:** `executed - passed`.
- **Target:** `scripts/oracle-diff.py` and the canonical oracle evidence.
- **Command:** `python3 scripts/oracle-diff.py --skip-build --oracle-root /home/stealthc/projects/ChronoFork/.research/upstream/exodus-headless --oracle-bin-dir .research/oracle/build --chronofork-bin target/debug/chronofork --out-dir .research/m5/verification/oracle-t12` (the oracle binaries are copied from the pinned parent build into `.research/oracle/build` as recorded in the T7/T12 evidence; point `--oracle-root` at the pinned checkout if its location differs and record the resolved directory).
- **Expected:** the three synthetic ROMs match the pinned oracle in projected executed
  addresses and selected final state (3/3 PASS), the negative control exits 1 at the
  corrupted index, the masked-ROM gating control reports no handler execution, and the
  canonical evidence schema remains `chronofork.oracle-diff.v2`; if the pinned oracle
  binaries are genuinely unavailable, the row records the exact reason and the unchanged
  canonical evidence hash, and the `FR-TRACE`/M1 claim is not re-run — it is not reported as
  a pass.
- **Evidence:** `.agents/notes/m5/t12/audit/oracle-rerun.log`, whose command used the same
  `--oracle-root` and wrote `.research/m5/verification/oracle-t12/`, with the 3/3 trace,
  final-state, negative-control and scheduler-control results and schema
  `chronofork.oracle-diff.v2`.

### M5-A38 — Full quality gate, rustdoc and target matrix

- **Status:** `executed - passed`.
- **Target:** repository quality gate and documentation build.
- **Command:** `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` and `CARGO_HOME=/tmp/t7-cargo RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.
- **Expected:** formatting, `cargo check --workspace`, Clippy
  `--workspace --all-targets -- -D warnings`, `cargo test --workspace` and rustdoc
  `-D warnings` all exit 0; 0 failed and 0 ignored; no test target below the pre-M5
  **45 targets / 596 passed** baseline (the matrix grows with the new trace/script targets);
  no production `#[allow]`, no `#[ignore]`, no deleted test, no relaxed lint and no new
  external dependency beyond the existing workspace graph (the single scoped test-only
  `cfg_attr(test, allow(clippy::expect_used))` in `chronofork-trace` is recorded in the
  dated quality-gate block); `chronofork-trace` and `chronofork-script` are workspace
  members with documented public items and no `unsafe`.
- **Evidence:** `.agents/notes/m5/t12/gate/` with the full log, per-target matrix and
  the exact exit codes.

### M5-A39 — Capability and claim honesty

- **Status:** `executed - passed`.
- **Target:** CLI capability report and documentation claim audit.
- **Command:** `CARGO_HOME=/tmp/t7-cargo cargo run -q -p chronofork-cli -- backend describe` and a documentation audit of
  `docs/adr/0011-tracing-and-scripting.md`,
  `docs/architecture/tracing-and-scripting.md`,
  `docs/quality/m5-tracing-and-scripting-acceptance.md`, `README.md`, `docs/README.md`,
  `AGENTS.md`, `docs/requirements/functional.md`, `docs/roadmap/milestones.md` and
  `docs/roadmap/roadmap.md`.
- **Expected:** `backend describe` still reports `capability.tracing: false` (and the
  synthetic backend's real `capability.debug_events: false`), with no new capability line
  or `true` value for the host trace surface; help text and durable docs state that tracing
  is host-provided over backend inspection/memory access and that bus/device records need
  `debug_events`; no doc claims Rhai/Luau/Lua/WASM/Python embedding, process isolation or a
  hostile-code security boundary, DMA/audio/Z80 trace sources, rendering, cycle accuracy,
  commercial-ROM/game compatibility, HTTP/MCP, a GUI viewer or cross-platform runtime
  evidence. At T12 execution time the durable docs still carried pending/design statuses
  and no implemented/passed claim existed before verification; T8 then promoted those
  statuses from the T12 result (not from T7).
- **Evidence:** `.agents/notes/m5/t12/audit/` with the capability output, a claims
  matrix and the exact doc lines checked.

## 5. Independent verification procedure

> **Execution record.** T7 executed this procedure in round 1 and returned
> `needs_revision` (F1–F10). T11 repaired the findings and T12 re-executed the procedure;
> T12's PASS is the final evidence. The steps below are retained as the reproducible
> procedure used by that final verification.

1. Confirm the worktree, branch `agent/m5-tracing-scripting`, base
   `84f1476`, `git status --short --branch` and the exact tree state; record every
   uncommitted path. Do not edit implementation paths while verifying.
2. Verify `.research/m5/baseline/` still matches its manifest; recreate nothing that would
   overwrite the baseline.
3. Execute rows M5-A1 – M5-A34 in order, capturing stdout, stderr, exit code, test
   summary, the matched test names and the tree state under the ignored
   `.agents/notes/m5/t12/rows/` tree (T7 used `.agents/notes/m5/t7/rows/` in round 1).
   A filter matching zero tests is a failure.
4. Execute M5-A35 (non-perturbation differential) and record the per-tick comparisons.
5. Execute M5-A36 (demos/state/movie/store), M5-A37 (oracle), M5-A38 (gate/rustdoc) and
   M5-A39 (claim audit).
6. Audit claims against code and runner output: a passing unit test, a green commit or an
   ADR statement is not evidence of a different row. Do not accept an unexecuted row, a
   copied hash or a narrowed claim.
7. Return **PASS** only if every row is `executed - passed`, every expected value
   matches, the baseline is byte-identical and no claim exceeds the verified scope;
   otherwise return **FAIL** with the exact failed row, expected value, observed value and
   required repair.
8. Record the authoritative dated gate block in [quality gates](quality-gates.md) through
   the documentation owner; this plan links to it and does not copy volatile counts.

## 6. Promotion rule

After the independent PASS (T12, 2026-11-16), the documentation owner promoted: the M5
row statuses and execution record in this plan, [ADR-0011](../adr/0011-tracing-and-scripting.md)
implementation status, the [M5 milestone](../roadmap/milestones.md#m5--tracing-and-scripting),
[roadmap](../roadmap/roadmap.md), [functional requirements](../requirements/functional.md)
(`FR-TRACE-001`, `FR-TRACE-002`, `FR-SCRIPT-001`), [README](../../README.md),
[docs index](../README.md), [AGENTS](../../AGENTS.md) crate map, the [tracing and scripting
architecture](../architecture/tracing-and-scripting.md) and the dated [quality
gates](quality-gates.md) block. Promotion records evidence links and known limits; it
never rewrites a failed row, converts a planned test into a pass or claims a feature that
was not executed. No commit, push, merge or release happens without separate explicit
authorization.
