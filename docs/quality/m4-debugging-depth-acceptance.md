# M4 Debugging Depth — Executable Acceptance Plan

- **Status:** **Executed; every row M4-A1–M4-A17 is `executed - passed`** (independent
  verifier PASS, 2026-11-16). The design contract
  [ADR-0010](../adr/0010-debugging-depth.md) is accepted and implemented for the documented
  deterministic minimal-slice scope on Linux. The post-PASS capability-report and wording
  fix at `6b2a567` was re-gated and audited; M4 is complete for that scope.
- **Date:** design 2026-11-16; execution and independent verification 2026-11-16.
- **Companion contract:** [ADR-0010: Debugging depth](../adr/0010-debugging-depth.md) —
  **Accepted** (owner-approved 2026-11-16 by the explicit M4 implementation directive).
- **Milestone:** [M4 — Debugging depth](../roadmap/milestones.md#m4--debugging-depth).
  Every row below is `executed - passed`, the independent verifier returned PASS and the
  durable docs are promoted, so M4 is complete for the documented minimal-slice scope.
- **Related:** [backend model](../architecture/backend-model.md), [Mega Drive
  slice](../architecture/megadrive-slice.md), [execution
  model](../architecture/execution-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [testing
  strategy](testing-strategy.md), [quality gates](quality-gates.md), [anti-slop
  policy](anti-slop-policy.md), [research: TAS and
  debugging](../research/tas-and-debugging.md), [research: emulator
  architecture](../research/emulator-architecture.md).

> **Verification record (2026-11-16).** The independent verifier executed all M4-A1–A17
> rows on the committed tree at HEAD
> `df983214bb145a3e2e073949d81814cf808b69a0` (branch `agent/m4-debugging-depth`, clean
> tree): 22 row commands plus 2 supplementary row parts, every exit 0, every expected test
> count met, 0 failed and 0 ignored. `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` exited 0
> (fmt, check, Clippy `-D warnings`, workspace tests, rustdoc `-D warnings`); the matrix
> grew from the 39-target / 532-passed baseline to 45 targets / 596 passed with no target
> below baseline and no test weakening. Both demos were byte-identical to the baseline in
> six runs each (hashes `9b4dd564…` and `912a5bb6…`, empty stderr), the non-perturbation
> probe made 1,700 per-step off/on comparisons with 0 mismatches, existing movie bytes were
> identical between the baseline build and HEAD (11/11 checks), and the pinned-oracle
> differential remained PASS 3/3 including negative and scheduler controls. The claim audit
> found no prohibited overclaim and recorded three low-severity precision notes, all fixed
> in the promotion (fixed 256-entry log ring wording, adjacent instruction-fetch caveat, and
> `backend describe` now prints `capability.debug_events`). Raw evidence is in the ignored
> `.research/m4/verification/` tree (`SUMMARY.txt`, `rows/`, `gate/`, `demos/`, `audit/`,
> `target-matrix.txt`, `MANIFEST.sha256`); the dated gate block is in [quality
> gates](quality-gates.md).

## 1. Status rules

1. **`planned - not yet executed`** means the row has a planned test target, command,
   expected observation and evidence artifact, but no executed result and no independent
   verifier record exists. Every row starts in that state.
2. A row becomes **`executed - passed`** only when the exact command, exit code, tree
   state (HEAD plus any uncommitted paths), test summary, ignored evidence artifact and an
   independent verifier PASS are recorded together. The authoritative dated gate block
   belongs in [quality gates](quality-gates.md); this plan links to it and does not copy
   volatile counts.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact or unverified claim. A failed row blocks M4. A known failure is never
   converted into a skip, an `#[ignore]`, a relaxed assertion or a narrower claim after
   the fact.
4. A test may be committed before the implementation as a red test only when it fails for
   the expected typed reason. A test that cannot run is not acceptance evidence.
5. A row may not be satisfied by an existing capability. A green M1/M2/M3 suite is a
   regression precondition, never acceptance for M4.
6. The M1/M2/M3 canonical artifacts are a hard non-regression precondition: snapshot
   payload bytes/version, state hashes, configuration hashes, movie bytes, both demo
   outputs and the M1 oracle evidence must stay byte-identical to the recorded baseline.
   Any unexplained difference stops the row; it is never explained away.
7. Acceptance is limited to the minimal Mega Drive slice and the deterministic synthetic
   backend on the Linux acceptance platform. No mid-instruction, cycle-exact,
   reverse-execution, cross-platform, UI, HTTP or MCP claim is made.
8. The implementation commits are not verification. Only the independent verifier
   execution under §5 produces the PASS.

## 2. Scope, definitions and evidence

- **Baseline:** `.research/m4/baseline/` in the `agent/m4-debugging-depth` worktree at
  HEAD `e816d793ae8ca31209971b8b27bdcf2ae0fafaf9` (clean tree). It contains the gate
  log, the 39-target / 532-passed test matrix, the Git tree state and three runs of each
  demo (`demo-1..3.txt`, `md-1..3.txt`, empty stderr) with SHA-256
  `9b4dd564…` for `demo` and `912a5bb6…` for `megadrive demo`.
- **Implementation tree:** the commits on `agent/m4-debugging-depth` after the accepted
  contract commit. The verifier records the exact HEAD and any uncommitted paths.
- **Breakpoint:** a host trigger that stops **before** an instruction whose PC matches the
  location and whose optional condition holds. Conditions are one register comparison or
  one bounded big-endian memory comparison; location and condition are ANDed.
- **Watchpoint:** a backend bus trigger over an inclusive address range and read/write
  access. The backend reports every matching byte access; the host returns a stop
  **after** the instruction that produced it. It is never a memory-diff approximation.
- **Event breakpoint:** a host trigger that stops after a step when a retained backend
  event matches an `EventFilter`.
- **Logpoint:** a host trigger that appends a bounded log entry when the PC matches and
  never stops.
- **Debug event:** one typed `DebugEvent` with the pre-step tick of the instruction that
  produced it. Bus events are byte-granular; VDP events carry the device access width.
- **Debug view:** a pure decoded `CpuView`, `VdpView`, `InterruptView`, `BusView` or
  disassembly record. Views read state; they do not mutate it.
- **Non-canonical debug state:** event log, watchpoint configuration, breakpoints,
  logpoints and the instruction trace. It is absent from snapshots, state hashes,
  configuration hashes and payload versions, and `restore` does not rewind it.
- **Acceptance backend:** `chronofork-megadrive` (minimal slice) for watchpoints, events,
  views, disassembly execution cross-checks and the CLI; `chronofork-test-backend` for
  the host breakpoint/logpoint engine and the unsupported-observation path.
- **Platform:** Linux, matching M1/M2/M3. Compile checks on other platforms are not
  runtime evidence.
- **Raw evidence (executed):** `.research/m4/verification/` with `SUMMARY.txt`,
  `target-matrix.txt`, `rows/M4-A*.txt`, `gate/`, `demos/`, `audit/`,
  `MANIFEST.sha256`. Paths are ignored and never committed.
- **No host time in data or reports:** valid output contains no timestamp, process id,
  host label, nonce, path or entropy. Comparisons use ticks, state hashes, bytes, block
  fields and SHA-256, never wall-clock or filesystem modification time.

## 3. Row map

| Milestone item | Acceptance rows |
|---|---|
| Breakpoints and watchpoints | M4-A1 – M4-A5 |
| Register/memory/disassembly views | M4-A6 – M4-A8 |
| Interrupt/bus/device event observation | M4-A9 – M4-A12 |
| Mega Drive-specific inspectors | M4-A13 – M4-A15 |
| CLI contract and regression | M4-A16 – M4-A17 |

## 4. Acceptance rows

At design time every row was **`planned - not yet executed`**; the verifier executed each
one and every row below is now **`executed - passed`** with its exact command, exit code,
tree state and evidence artifact recorded. The command column names the lowest useful
target; the verifier could add the full-package command as context but recorded the exact
row command and exit code. All commands ran with `CARGO_HOME=/tmp/t7-cargo` in the
acceptance worktree.

### M4-A1 — PC breakpoint stops before the instruction

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-debug/tests/debugger.rs` (new target
  `debugger`), test `pc_breakpoint_stops_before_the_instruction`.
- **Command:** `cargo test -p chronofork-debug --test debugger pc_breakpoint_stops_before_the_instruction -- --exact`.
- **Expected:** a breakpoint at a known PC in the default synthetic program stops with
  `StopReason::Breakpoint`; the reported tick/state hash/PC equal a fresh inspection at
  that boundary, not the state after executing the instruction; `steps` is the exact
  number of executed steps; the same run repeated twice reports identical values.
- **Evidence:** row log with the stop block and both repeated values.

### M4-A2 — Range and conditional breakpoints

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** same target, tests `range_breakpoint_matches_inclusive_bounds` and
  `conditional_breakpoint_requires_register_and_memory_conditions`.
- **Command:** `cargo test -p chronofork-debug --test debugger -- range_breakpoint_matches_inclusive_bounds conditional_breakpoint_requires_register_and_memory_conditions`.
- **Expected:** a range stop hits at the first enabled PC inside the inclusive bounds and
  misses immediately outside; a register condition (`d0==3`) stops only at the matching
  state; a memory condition (big-endian 2 bytes at a known RAM address) stops only when
  the value matches; a condition that never holds reaches `StepLimit` instead of stopping
  or erroring.
- **Evidence:** row log with hit and miss observations.

### M4-A3 — Trigger lifecycle, limits and typed errors

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** same target, tests `trigger_tables_are_bounded_and_removable` and
  `unknown_register_and_missing_pc_are_typed_errors`.
- **Command:** `cargo test -p chronofork-debug --test debugger -- trigger_tables_are_bounded_and_removable unknown_register_and_missing_pc_are_typed_errors`.
- **Expected:** add/remove/disable/enable for breakpoints, watchpoints, event breakpoints
  and logpoints; removal of an unknown id and a full table return typed `DebugError`
  values; an unknown register is `UnknownRegister`; a backend whose register map lacks
  `pc` returns `MissingProgramCounter`; no panic and no silently ignored trigger.
- **Evidence:** row log with every error variant and the table counts.

### M4-A4 — Memory watchpoints on the Mega Drive bus

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/debug_observation.rs`, test
  `watchpoints_report_emulated_reads_and_writes`; `crates/chronofork-debug/tests/debugger.rs`,
  test `watchpoint_stops_after_the_writing_instruction`.
- **Command:** `cargo test -p chronofork-megadrive --test debug_observation watchpoints_report_emulated_reads_and_writes` and `cargo test -p chronofork-debug --test debugger watchpoint_stops_after_the_writing_instruction`.
- **Expected:** a write watchpoint over a RAM range produces a byte event with the exact
  address, value, access kind and pre-step tick; the host stops after the instruction and
  the stop state hash equals a fresh inspection after that step; a read watchpoint fires
  on an emulated controller data-register read; a debugger `read_memory` of the same
  address produces no event and no stop; the same access outside the range is ignored.
- **Evidence:** both row logs with address/value/tick/hash and the negative controls.

### M4-A5 — Observation is non-canonical and unsupported backends are typed

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/debug_observation.rs`, tests
  `debug_observation_does_not_change_state_or_payload` and
  `watchpoint_configuration_is_bounded`; `crates/chronofork-debug/tests/debugger.rs`, test
  `watchpoints_require_backend_support`.
- **Command:** `cargo test -p chronofork-megadrive --test debug_observation -- debug_observation_does_not_change_state_or_payload watchpoint_configuration_is_bounded` and `cargo test -p chronofork-debug --test debugger watchpoints_require_backend_support`.
- **Expected:** a machine with observation enabled and watchpoints installed produces the
  same state hash, `to_bytes` payload/version and config hash as the same machine with
  observation disabled, at every checked tick; restore leaves the event log untouched;
  invalid capacity, an empty/inverted range, duplicate or zero ids and more than 64 specs
  return typed backend errors; the synthetic backend reports no debug access, so adding a
  watchpoint returns `WatchpointsUnsupported`.
- **Evidence:** row log with before/after hashes and payload byte lengths, plus every
  typed rejection.

### M4-A6 — Register/SR inspection view

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-cli/tests/debug.rs`, test
  `debug_inspect_prints_stable_cpu_and_device_fields`; `crates/chronofork-megadrive/tests/inspect.rs`,
  test `cpu_view_decodes_sr_fields`.
- **Command:** `cargo test -p chronofork-cli --test debug debug_inspect_prints_stable_cpu_and_device_fields` and `cargo test -p chronofork-megadrive --test inspect cpu_view_decodes_sr_fields`.
- **Expected:** `chronofork debug inspect --rom <fixture> --steps N` prints one
  deterministic `record=debug-inspect` block containing tick, state hash, halted, PC,
  SR, supervisor, trace, interrupt mask, X/N/Z/V/C, VDP status/address/auto-increment and
  scheduler frame/line/quantum; two runs are byte-identical; the CPU view decodes every
  SR field independently of the raw word.
- **Evidence:** row log with stdout and SHA-256 of two runs.

### M4-A7 — Memory view

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-cli/tests/debug.rs`, test
  `debug_memory_dump_is_exact_and_bounded`.
- **Command:** `cargo test -p chronofork-cli --test debug debug_memory_dump_is_exact_and_bounded`.
- **Expected:** `debug memory --address --length` prints the exact big-endian bytes in
  the documented 16-byte hex/ASCII rows with no timestamps; a zero length prints an empty
  body after the header; an unaligned (odd) address and a length crossing the mapped
  window return exit 1, empty stdout and a `chronofork:` stderr; a debugger read never
  mutates the state hash.
- **Evidence:** row log with the exact dump, bounds failure and before/after hashes.

### M4-A8 — Disassembly view and execution cross-check

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/src/disasm.rs` unit tests (module target
  `chronofork_megadrive`) and `crates/chronofork-cli/tests/debug.rs`, test
  `debug_disasm_matches_supported_encodings`.
- **Command:** `cargo test -p chronofork-megadrive disasm::` and `cargo test -p chronofork-cli --test debug debug_disasm_matches_supported_encodings`.
- **Expected:** every supported encoding prints the accepted mnemonic/operands and a
  length equal to the CPU's executed PC delta; unsupported encodings print `DC.W` with
  length 2; a bad count, odd address, truncation and out-of-window extension word return
  typed errors; two CLI runs are byte-identical.
- **Evidence:** row log with the encoding table, the execution cross-check and the CLI
  output.

### M4-A9 — Interrupt event observation

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/debug_observation.rs`, test
  `interrupt_events_are_requested_delivered_and_acknowledged`.
- **Command:** `cargo test -p chronofork-megadrive --test debug_observation interrupt_events_are_requested_delivered_and_acknowledged`.
- **Expected:** with the independently authored `interrupt_rom` and a short quantum the
  log shows `InterruptRequested`, then `InterruptDelivered` and
  `InterruptAcknowledged` for the delivered level, in that order and with the expected
  pre-step ticks; a masked run shows `InterruptRequested` with no delivery or
  acknowledgement; only the delivered level is acknowledged; the log contains no
  duplicate request for a coalesced level.
- **Evidence:** row log with the event sequence for both runs.

### M4-A10 — Bus and device event observation with bounds

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/debug_observation.rs`, tests
  `bus_and_device_events_capture_the_documented_windows` and
  `event_log_is_bounded_and_deterministic`.
- **Command:** `cargo test -p chronofork-megadrive --test debug_observation -- bus_and_device_events_capture_the_documented_windows event_log_is_bounded_and_deterministic`.
- **Expected:** controller reads, VDP port reads/writes and RAM byte writes appear with
  exact address/offset/value/width and pre-step tick; instruction fetches appear only when
  a watchpoint matches them; a small-capacity log evicts oldest entries, reports the
  dropped count and keeps contiguous indices; a repeated run produces identical event
  sequences; the synthetic backend reports `debug_events=false`.
- **Evidence:** row log with event sequences, eviction counts and repeat comparison.

### M4-A11 — Event breakpoints and logpoints

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-debug/tests/debugger.rs`, tests
  `event_breakpoint_stops_on_a_matching_event` and
  `logpoint_records_without_stopping`.
- **Command:** `cargo test -p chronofork-debug --test debugger -- event_breakpoint_stops_on_a_matching_event logpoint_records_without_stopping`.
- **Expected:** an interrupt-delivery filter stops after the delivering step and reports
  the matching event; a VDP-write, controller-read and bus-write-range filter each stop
  on the first matching event; a logpoint at a repeated PC records every hit, increments
  its hit count, never changes the stop reason and is bounded by the log capacity with a
  dropped count; the run still stops at `StepLimit` when only logpoints are configured.
- **Evidence:** row log with stop reasons, event blocks and log entries.

### M4-A12 — Debug observation does not perturb canonical artifacts

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/debug_observation.rs`, test
  `debug_observation_does_not_change_state_or_payload` (shared with A5), plus the
  command-level probe `debug_observation_off_on_differential`.
- **Command:** `cargo test -p chronofork-megadrive --test debug_observation debug_observation_does_not_change_state_or_payload` and a verifier-run differential using the CLI/`tas` outputs recorded under `.research/m4/verification/audit/`.
- **Expected:** for the same ROM, steps and input, state hashes and snapshot payload bytes
  are identical with observation off and on at every probed tick; configuration hash and
  payload version are unchanged; both demos remain byte-identical to the baseline; an
  existing `.cfmovie` records/replays to identical bytes and hashes; the M1 oracle
  evidence remains PASS 3/3.
- **Evidence:** differential probe output, demo `cmp` results and the oracle evidence
  reference.

### M4-A13 — VDP inspector

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/inspect.rs`, test
  `vdp_view_matches_direct_device_accessors`; CLI test
  `debug_vdp_prints_registers_status_and_vram_digest`.
- **Command:** `cargo test -p chronofork-megadrive --test inspect vdp_view_matches_direct_device_accessors` and `cargo test -p chronofork-cli --test debug debug_vdp_prints_registers_status_and_vram_digest`.
- **Expected:** status word and decoded VBlank/VINT bits, address, auto-increment and all
  24 registers equal the direct `Vdp` accessors after a known program; the VRAM SHA-256
  equals a hash of `vdp.vram()`; a requested VRAM window prints the exact bytes and is
  bounds-checked; two CLI runs are byte-identical.
- **Evidence:** row log with values and digest.

### M4-A14 — Interrupt/scheduler inspector

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/inspect.rs`, test
  `interrupt_view_matches_scheduler_state`; CLI test
  `debug_interrupts_prints_pending_issue_tick_and_counters`.
- **Command:** `cargo test -p chronofork-megadrive --test inspect interrupt_view_matches_scheduler_state` and `cargo test -p chronofork-cli --test debug debug_interrupts_prints_pending_issue_tick_and_counters`.
- **Expected:** standard, quantum, frame/line/quantum position, pending requests with
  exact issue tick and the CPU-mask deliverability predicate, and per-level issued and
  coalesced counters equal the scheduler accessors; a masked run reports
  `deliverable=0` while the request remains pending; a delivered run reports the
  post-delivery state; two CLI runs are byte-identical.
- **Evidence:** row log with before/after-delivery values.

### M4-A15 — Bus and CPU inspectors

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-megadrive/tests/inspect.rs`, test
  `bus_view_lists_the_mapped_windows`; CLI test `debug_bus_and_cpu_view_are_stable`.
- **Command:** `cargo test -p chronofork-megadrive --test inspect bus_view_lists_the_mapped_windows` and `cargo test -p chronofork-cli --test debug debug_bus_and_cpu_view_are_stable`.
- **Expected:** the bus view lists exactly the ROM, work-RAM, VDP and controller windows
  with correct inclusive/exclusive bounds and writability; ROM/RAM lengths match the
  backend; the CPU/disassembly view at PC shows the instruction the CPU will execute
  next; a debugger memory write is visible in the view and in the next execution; two
  runs are byte-identical.
- **Evidence:** row log with the window table and instruction-at-PC.

### M4-A16 — CLI grammar, errors and exit codes

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** `crates/chronofork-cli/tests/debug.rs`, test
  `debug_command_rejects_invalid_input_with_typed_errors`.
- **Command:** `cargo test -p chronofork-cli --test debug debug_command_rejects_invalid_input_with_typed_errors`.
- **Expected:** missing/unknown subcommand, missing `--rom`, unreadable/empty/oversized
  ROM, unknown flag, malformed breakpoint/watchpoint/eventpoint/logpoint spec, odd
  address, out-of-window memory/disassembly request, too many steps and a full trigger
  table all exit 1 with empty stdout for the failing operation and a `chronofork:`
  stderr; no panic; `debug` is present in `chronofork help` and `debug --help` documents
  the implemented flags only.
- **Evidence:** row log with every command, exit code and stderr.

### M4-A17 — Full gate, regression and documentation promotion

- **Status:** **executed - passed** (independent verifier PASS, 2026-11-16).

- **Target:** repository quality gate and documentation audit.
- **Command:** `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh`, both demos three times,
  `git status --short --branch`, and the documentation link/status audit.
- **Expected:** every gate command exits 0; no test target is below the 39-target /
  532-passed baseline; both demos are byte-identical to the baseline in all three runs
  with empty stderr; the new `chronofork-debug` crate is documented and linked; M4 items
  are ticked only after rows A1–A16 pass; `FR-DEBUG-002/003/004`, the new view and
  inspector requirements, `Capabilities::debug_events` and the milestone/roadmap text are
  promoted from planned to implemented with the verifier evidence; no prohibited asset,
  secret or reference clone is tracked; the handoff states the exact commits, branch,
  verification scope, limitations and integration status.
- **Evidence:** gate log, target matrix, demo hashes, documentation audit and handoff
  report.

## 5. Independent verification procedure

1. Confirm the worktree, branch, HEAD and `git status --short --branch`; record any
   uncommitted path.
2. Recreate the pre-change baseline if missing (`.research/m4/baseline/`) and verify its
   manifest.
3. Execute rows M4-A1 – M4-A16 with the exact commands, capturing stdout, stderr, exit
   code, test summary and the tree state in `.research/m4/verification/rows/`.
4. Run the full gate and both demos three times; compare byte-for-byte with the baseline.
5. Execute the non-perturbation differential (A12) and re-reference the M1 oracle PASS.
6. Audit claims against code and runner output; do not accept an ADR, a commit message, a
   plan row or a code review as execution evidence.
7. Return **PASS** only if every row is `executed - passed` and no claim exceeds the
   verified scope; otherwise return **FAIL** with the exact failed row, expected value,
   observed value and required repair. Do not edit implementation paths while verifying.

## 6. Promotion rule

After PASS, the documentation owner promotes: this plan's row statuses and execution
record, [ADR-0010](../adr/0010-debugging-depth.md) implementation status, the [M4
milestone](../roadmap/milestones.md), [roadmap](../roadmap/roadmap.md), [functional
requirements](../requirements/functional.md), [backend model](../architecture/backend-model.md),
[Mega Drive slice](../architecture/megadrive-slice.md), [docs index](../README.md) and
the dated [quality gates](quality-gates.md) block. Promotion records evidence links and
known limits; it never rewrites a failed row or converts a planned test into a pass.
