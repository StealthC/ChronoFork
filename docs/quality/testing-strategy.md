# Testing and Quality Strategy

Status: strategy grounded in the checked-out upstream test infrastructure and verified
against the bootstrap workspace as implemented on 2026-11-13.
Audit date: 2026-11-13.

This document does **not** copy upstream tests or fixtures. It extracts testable
methodology from them and defines what ChronoFork should build for itself. The
licensing constraints that shape this strategy are in
[`docs/research/licensing-assessment.md`](../research/licensing-assessment.md).

At audit time the workspace declares six crates (`chronofork-core`,
`chronofork-backend-api`, `chronofork-sessions`, `chronofork-test-backend`,
`chronofork-megadrive`, `chronofork-cli`) and strict workspace lints. The lead's bootstrap implementation
was present by the end of the audit: the workspace has 40 passing tests, including
core-primitive, test-backend, snapshot, session-acceptance and CLI tests. There is
still no CI workflow, no dependency-policy configuration, no fixture manifest and
no property/fuzz target. The gates below are the target contract; §2.3 maps what
already exists to it and lists the remaining gaps.

## 1. Principles

1. **Determinism is the product.** A fork of an emulator session is only
   meaningful if the same inputs produce the same state. Determinism is tested
   directly, not assumed from "it usually boots".
2. **The oracle must be independent of the thing being tested.** Golden values
   are useful only when their provenance is recorded and the test cannot pass by
   silently weakening comparison.
3. **Prefer generated fixtures to shipped artifacts.** A program a test can
   generate is auditable, redistributable, and predicts its own expected output.
4. **Fail fast and locally.** Pure unit and contract tests run before expensive
   end-to-end runs; a broken primitive should never wait for an emulator boot.
5. **One failure must not hide the rest.** Session/protocol checks accumulate
   results, then fail once with a list, rather than aborting at the first
   mismatch.
6. **No silent skips.** Missing external artifacts are reported as skipped with
   a reason, and known failures are explicit and tracked.
7. **Measure, then claim.** Documentation claims (determinism, isolation,
   capability) must be backed by an executable check.

## 2. Upstream infrastructure and the current ChronoFork baseline

### 2.1 Inventory

| Source | Infrastructure | Scale / evidence | What is reusable | What must not be copied |
|---|---|---|---|---|
| Exodus `08f388f7…` | `ExodusTests.sln` and `Support Libraries/MarshalSupport/Tests/` only | Marshalling unit and performance tests; no emulator-level tests, no CI config in the checkout | Layered separation: SDK/support-library tests exist independently of the GUI; test data compare helpers | The C++ test sources and their marshalling harness |
| exodus-for-ais `ec005615…` | Standalone C++ unit tests, a scripted MCP session, generated-ROM end-to-end test, trace-equivalence gate, persistence round-trip | `host/tests/PathSemanticsTest.cpp` (133 lines), `mcp/tests/JsonTest.cpp` (236), `TextCodecTest.cpp` (226), `mcp/tests/session_test.py` (1,010 lines / 186 checks), `tests/rom/make_test_rom.py` (266) + `rom_test.py` (407), `tests/equivalence/compare_traces.py` (174), `tests/hierarchicalstorage/RoundTripTest.cpp` (682); no CI workflow in checkout | *Methodology only*: predict-then-verify, first-divergence traces, protocol conformance, fail-fast runners, real-data round-trips | The C++/Python test files, skill text, generated test ROM bytes, and any trace dumps |
| BizHawk `d148601a…` | MSTest unit projects plus external test-ROM suites, CI test stage, state/movie tests, determinism bans, stability fixtures | 61 test `.cs` files across `BizHawk.Tests.Common`, `.Emulation.Common`, `.Client.Common`, `.Testroms*`; 3 test-ROM suites; `.gitlab-ci.yml` `run_tests` stage; `MainSln.runsettings`; `BannedSymbols.BannedApiAnalyzers.txt`; `src/BizHawk.Tests.Client.Common/data/dearchive/` | *Methodology only*: external-artifact test ROMs with hashes, known-failure quarantine, stability fixtures, state-manager tests with fakes, compile-time nondeterminism bans, JUnit reporting | All BizHawk source, test code, fixtures, test-ROM artifacts, binaries, and assets (see licensing assessment) |

### 2.2 Specific upstream patterns worth learning from

**Predict-then-verify with a generated program (exodus-for-ais).**
`tests/rom/make_test_rom.py` emits a 32 KB Mega Drive cartridge whose vector table,
header, jump table, data table, handlers, subroutine and trap handler are laid out at
known addresses. `rom_test.py` asserts the emulator's trace against the *predicted*
instruction sequence and address set, not against a previously recorded run. It also
contrasts a no-cartridge trace (CPU walking zero-filled memory) with the cartridge trace,
so the test proves the cartridge mattered. The expected opening trace is derived from the
generator, not from the emulator's output.

**First-divergence trace comparison (exodus-for-ais).**
`tests/equivalence/compare_traces.py` sorts fields into different strictness classes:
`currentCycle` and `address` exact; `opcode`/`args` exact by default with an explicit
`--ignore-disassembly` opt-out; `currentTime` within `1e-9`. Malformed lines are fatal
(not skipped), a length difference is reported separately from an execution divergence,
and comparison stops at the first execution divergence because later differences are
consequences. The README documents a negative control: corrupt one record and confirm the
tool fails at the right point, so a pass cannot be an artefact of permissive comparison.
The published gate is 18,941 identical records with the same SHA-256 across MSVC/Windows
and clang/Linux.

**Scripted protocol conformance over the real transport (exodus-for-ais).**
`mcp/tests/session_test.py` starts the real daemon, speaks newline-delimited JSON-RPC over
stdio, and checks handshake, tool listing and schemas, guards before a module is loaded,
malformed JSON, unknown tools, state after load/step/reset/shutdown, handle invalidation
and non-reuse across reload, trace ring-buffer behaviour, device reflection, and active
disassembly. It captures stderr separately to prove diagnostics did not contaminate the
protocol stream, uses a `Checker` that accumulates failures, and fails the process with a
summary.

**Fail-fast, dependency-light unit tests (exodus-for-ais).**
`mcp/tests/build_and_run.sh` compiles and runs the JSON and text-codec tests before
starting the daemon: "if the wire format is broken there is no point starting the daemon".
`host/tests/build_and_run.sh` builds the path-semantics test without the full CMake build
because the functions have no dependencies beyond the standard library.

**Round-trip persistence on real data (exodus-for-ais).**
`tests/hierarchicalstorage/RoundTripTest.cpp` loads real `Data/Modules/*.xml` documents
and checks save/load behaviour rather than only synthetic structures. The equivalence
README notes that this port work found duplicate-attribute and UTF-32 bugs that the
original compiler never instantiated, which is a useful reminder that persistence and
encoding tests deserve real files.

**External artifacts and fixture provenance (BizHawk).**
- `src/BizHawk.Tests.Testroms.GB/readme.md`: suites live under `res/` and are downloaded
  from `libre-roms-ci` CI artifacts; firmware is copied manually by the user; missing
  suites are skipped.
- `src/BizHawk.Tests.Testroms/.download_from_ci.sh`: fetch at test time, no ROMs in the
  repository.
- `src/BizHawk.Tests.Client.Common/data/dearchive/readme.md`: the one committed GB fixture
  is the MIT-licensed Mealybug Tearoom test ROM; the readme records the exact source and
  commit, and `DearchivalTests.cs` asserts its SHA-1 before use. This is the model to
  imitate if a third-party fixture is ever unavoidable.
- `src/Testroms.props` / `TestUtils.cs`: known failures are skipped by default, can be run
  with `BIZHAWKTEST_RUN_KNOWN_FAILURES=1`, and their list must be sorted; this prevents
  silent regressions from accumulating.

**State and replay tests with fakes (BizHawk).**
`PagedStateManagerTests` and `ZwinderStateManagerTests` exercise rewind buffers,
eviction, reserved states, save/load round-trips and boundary cases against a
deterministic fake `IStatable`, without needing a real emulator. `TasMovieTests` exercises
movie editing invalidation and multitrack operations. These are behavioural tests of the
session/state layer, exactly the split ChronoFork needs between `chronofork-sessions` and
a backend.

**Compile-time determinism bans (BizHawk).**
`src/BannedSymbols.BannedApiAnalyzers.txt` bans `Guid.NewGuid()`, `Random` constructors,
`String.GetHashCode`, `Util.GetRandomUUIDStr` and host-endian `BitConverter` calls in
core paths with explicit reasons ("this is a source of nondeterminism"). ChronoFork
should express the same rule with workspace lints, dependency policy, and a runtime
determinism test.

### 2.3 Verified bootstrap baseline (2026-11-13)

> This table records the bootstrap baseline as executed on 2026-11-13 (40 tests).
> The current stage-1b tree passes 98 tests; the authoritative current gate results
> live in [quality gates](quality-gates.md).

The quality workstream ran the repository's existing gates against the implemented
workspace. Full results are recorded in `docs/quality/quality-gates.md`; the summary is:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | passed |
| `cargo check --workspace` | passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| `cargo test --workspace` | 40 passed, 0 failed, 0 ignored |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | passed |
| `cargo run -p chronofork-cli -- demo` | passed (`chronofork demo: PASS`) |
| `./scripts/check.sh` | passed (wraps the five commands above) |

Test breakdown: `chronofork-core` 8 unit; `chronofork-sessions` 4 snapshot-codec +
5 acceptance; `chronofork-test-backend` 7 unit; `chronofork-cli` 2 integration; doc tests 0.
G0, G1 and the rustdoc part of G4 are runnable now via `scripts/check.sh`.

Mapping of the existing tests to the invariant set in §6:

| Invariants / category | Existing tests | Status |
|---|---|---|
| INV-01, INV-02 (fresh-state and step determinism) | `identical_machines_converge_on_identical_inputs`, `different_inputs_diverge`, `three_way_fork_is_isolated_and_diverges` | Covered in-process |
| INV-03 (tick monotonicity/overflow) | `tick_advances_monotonically`, `tick_overflow_is_an_error` | Covered |
| INV-04 (snapshot round-trip) | `snapshot_restore_round_trips_exact_state`, `snapshot_round_trips_through_bytes` | Covered |
| INV-05 (restore-continue equivalence) | `restore_and_replay_reproduce_the_branch_state` | Covered |
| INV-06 (canonical/stable snapshot bytes) | round-trip only | Partial: no golden bytes, no cross-version fixture |
| INV-07 (incompatible/corrupt snapshot rejection) | `snapshot_from_other_program_is_rejected`, `unsupported_outer_version_is_rejected`, `corruption_is_rejected`, `truncation_is_rejected` | Covered |
| INV-08 (fork independence) | `three_way_fork_is_isolated_and_diverges`, `fork_current_does_not_mutate_the_source` | Covered |
| INV-09 (factory isolation) | two independent machines in `identical_machines_…` | Partial: no explicit two-session factory test |
| INV-10 (inspection purity) | none | Missing |
| INV-11 (`write_memory` debug boundary) | `memory_access_is_bounds_checked`, fork memory isolation | Partial: bounds and isolation only |
| INV-12 (halted behaviour) | `halted_machine_rejects_further_steps` | Covered |
| INV-13 (replay from input tape) | `restore_and_replay_reproduce_the_branch_state` | Covered at API level |
| INV-14 (lifecycle scoping) | `lifecycle_guards_paused_and_closed_sessions` | Partial: no ID-reuse/reload case |
| INV-15 (capability honesty) | `backend_describe_reports_real_capabilities` | Partial: printed flags only |
| INV-16 (persistence/versioning) | `serialized_checkpoint_can_seed_a_new_manager`, snapshot codec tests | Covered |
| INV-17 (concurrency) | none | Missing (no concurrent API yet) |
| INV-18 (failed-operation atomicity) | rejection tests assert the error | Partial: state-after-failure not asserted |
| INV-19 (bounds) | `memory_access_is_bounds_checked` | Covered |
| INV-20 (no host time/entropy) | none | Missing enforcement |
| T6 CLI error paths | demo/describe success only | Partial |
| T8 property/fuzz | none | Missing |
| F1–F10 fixture policy | none | Missing (no fixtures yet) |

**Remaining gaps, in priority order:**

1. **CI invocation and reporting.** `scripts/check.sh` exists, but no workflow runs it
   and no JUnit/artifact reporting exists. This is the highest-value gap because it
   makes every other gate automatic.
2. **Canonical snapshot stability (INV-06).** Add a golden snapshot byte/hash fixture
   and a deliberate format-version bump test. Round-trip alone does not catch silent
   canonicalization changes.
3. **Inspection purity (INV-10) and failed-operation atomicity (INV-18).** Small tests:
   hash before/after repeated `inspect`/`read_memory`; hash before/after a rejected
   restore or out-of-bounds write.
4. **CLI error paths and exit codes (T6).** Unknown command, missing subcommand, too
   many arguments, and failure output stability.
5. **Fixture policy (F1–F10).** Implement the manifest/check even while the current
   fixture set is one in-code synthetic program; otherwise the first real fixture will
   bypass review.
6. **Dependency/license gate (G3).** Add `deny.toml` and a `cargo deny check` job; the
   current tree has one direct dependency and no policy file.
7. **Cross-process and debug/release determinism (D2).** Run the acceptance scenario in
   two processes and in release mode; compare state hashes and demo output.
8. **Capability honesty for false flags (INV-15)** and factory isolation (INV-09).
9. **Property tests for snapshot decoding (T8).** Arbitrary bytes must produce typed
   errors, never panic; `proptest` can cover this cheaply before full fuzzing.
10. **Known-failure quarantine and test-count guard.** Adopt the BizHawk-style explicit
    quarantine plus a CI check that fails when zero tests are discovered.

Concurrency (INV-17), fuzzing, cross-platform differential runs and performance
benchmarks remain deferred until the API and a real backend justify them.

## 3. Reusable methodology

| ID | Method | Upstream basis | ChronoFork application |
|---|---|---|---|
| M1 | Predict-then-verify | Generated cartridge + expected trace | Synthetic backend programs/tapes with expected tick/register/hash sequences; expected values derived from the program definition |
| M2 | Strict/tolerant trace diff with first-divergence stop | `compare_traces.py` | A `trace-diff` test utility: exact fields (tick, state hash, machine identity), tolerant formatting fields; fatal malformed input; separate length mismatch; context before divergence |
| M3 | Negative control for the oracle | Corrupt one trace record | Every comparison gate has a self-test that corrupts/truncates input and must fail at the expected point |
| M4 | Golden state hashes | Equivalence SHA-256 claim | Store `StateHash` values for committed scenarios; compare across runs, processes and (later) platforms |
| M5 | Scripted protocol conformance over real transport | `session_test.py` | Same shape for ChronoFork CLI/session/JSON APIs: handshake/version, schema, guards, errors, state transitions, transport purity, accumulated checks |
| M6 | Fail-fast layering | Standalone unit tests run first | `cargo test --lib` → contract suite → state/replay → CLI acceptance; cheap failures first |
| M7 | Real-data round-trip | Hierarchical storage round-trip | Snapshot/serialization tests use real crate types and real session state, not only hand-built structs |
| M8 | External artifacts with provenance | Test-ROM CI downloads; Mealybug readme+SHA-1 | Fixture manifest with source, revision, SPDX license, SHA-256; missing artifacts skip with a reason; never vendor commercial ROMs |
| M9 | Known-failure quarantine | Sorted known failures, opt-in run | A tracked quarantine list with issue links; default CI skips them; a separate job runs them and fails loudly if they unexpectedly pass |
| M10 | Stability fixtures | `SerializationStabilityTests`, `CorePickerStabilityTests` | Golden serialized snapshot/session metadata for format version 1; any change requires an explicit version bump and migration test |
| M11 | State-manager behavioural tests with a fake backend | Paged/Zwinder tests | Test session checkpoint selection, fork lineage and eviction with `chronofork-test-backend`, independent of a real emulator |
| M12 | Compile-time nondeterminism bans | `BannedSymbols.BannedApiAnalyzers.txt` | Deny `std::time::SystemTime::now`, `Instant::now`, `rand` in state paths, `HashMap` iteration in serialized state, pointer-derived hashes; enforce with clippy config/deny rules and review |
| M13 | Reporting as a gate | JUnit XML artifacts, `TreatNoTestsAsError`, 5-minute test-session timeout | JUnit output, non-zero exit if no tests ran, per-test timeouts, CI artifacts for failure diagnosis |

## 4. Test categories and ownership

| # | Category | What it proves | Where it lives | Speed / gate |
|---|---|---|---|---|
| T1 | Core primitives | `Tick` monotonicity/overflow, `StateHash` determinism, `SessionId`/`SnapshotId` opacity and display, `InputFrame` round-trip | `crates/chronofork-core` unit tests (`#[cfg(test)]`) | milliseconds; always |
| T2 | Backend contract | Every `Backend`/`BackendFactory` implementation honours its documented invariants and errors | A reusable conformance suite (integration test or shared test crate) run against `chronofork-test-backend` and later real backends | fast; always |
| T3 | Determinism and state | Same inputs → same state; snapshot round-trip; restore-and-continue equality; fork independence | `chronofork-backend-api` + `chronofork-test-backend` integration tests | fast; always |
| T4 | Isolation and concurrency | Independent instances, no shared mutable state, safe parallel stepping where promised | Contract suite + `Send` boundary tests | fast; always |
| T5 | Session lifecycle | Create/fork/restore/close, lineage and IDs, invalidation rules, capacity and error behaviour | `chronofork-sessions` tests | fast; always |
| T6 | CLI acceptance | End-to-end user workflows, stable JSON output and exit codes, failure messages | `crates/chronofork-cli/tests/` | medium; always |
| T7 | Protocol/transport (future) | JSON-RPC/MCP/HTTP conformance, schema, error codes, stream purity | New test crate when a transport exists | medium; gate when introduced |
| T8 | Property/fuzz | No panics on arbitrary snapshot/input/id bytes; typed errors; bounded allocation | `proptest`/`cargo-fuzz` targets | minutes; nightly or when the surface changes |
| T9 | Differential/oracle (extended) | Cross-backend and cross-platform agreement; optional external oracle | Separate `xtask`/script + long-running CI job | slow; nightly/optional |
| T10 | Performance regression | Step throughput and snapshot size do not collapse | Benchmark harness (`criterion` or custom) | slow; informational, not a bootstrap blocker |

T2 is the keystone: write the contract suite once, run it against the deterministic test
backend first, and require every future backend to pass it unchanged. The suite is the
backend's executable specification.

## 5. Determinism, trace and differential testing

### 5.1 Trace format for ChronoFork

Define a ChronoFork trace record rather than imitating an upstream dump. Suggested
fields, mapped to existing types:

```text
tick, state_hash, halted, input_buttons, machine_id, note
```

- `tick` (`Tick`) and `state_hash` (`StateHash`) are the strict ground truth.
- `halted` and `input_buttons` are strict.
- `machine_id` is strict when comparing the same backend, tolerant when comparing
  different backends by design.
- `note`/diagnostics are non-strict and excluded from comparison by default.

Field policy must be documented and tested, exactly as the Exodus equivalence README
documents its field classes. The comparison tool must:

1. reject malformed records rather than skip them;
2. stop at the first execution/state divergence and print context;
3. distinguish a length mismatch from a divergence;
4. support an explicit tolerance only for fields where formatting, not behaviour, can
   differ;
5. have a negative-control test proving that one corrupted record fails.

### 5.2 Differential opportunities

| ID | Comparison | What it catches | Bootstrap status |
|---|---|---|---|
| D1 | Same backend, same input tape, two runs in one process | Hidden nondeterminism, iteration order, global state | Required |
| D2 | Same run in two processes / debug vs release | Allocator/layout/optimisation sensitivity, host entropy | Required (debug vs release; two-process optional) |
| D3 | Continuous run vs restore from checkpoint and continue | Incomplete snapshot, state that is not captured | Required |
| D4 | Fork from checkpoint vs original continuing with the same input | Fork isolation, shared mutable state, copy-on-write mistakes | Required |
| D5 | Test backend vs real backend on a shared contract subset | Contract drift; not emulation accuracy | Deferred until a real backend exists |
| D6 | ChronoFork trace vs exodus-for-ais trace for a generated program | Instruction-level disagreement with a known-good MIT emulator | Post-repair PASS for two synthetic ROMs (address projection, opcode/ROM-byte audit, final state, negative control). No timing or interrupt comparison |
| D7 | Linux vs Windows vs macOS; x86_64 vs aarch64 | Endianness, float formatting, host assumptions | Deferred; gate before a cross-platform release |
| D8 | Mutated trace/checkpoint bytes | Comparison and validation robustness | Required self-test |
| D9 | Session replay from an input log | Replay completeness, event ordering | Required once session replay exists |

**Oracle rules.** A trace generated by running an external emulator is data, not code,
but do not commit upstream binaries, ROMs, or fixture dumps. Generate the program with a
ChronoFork-owned generator, run the oracle locally/CI-only, record the oracle's name,
commit, build, and the comparison result, and gate only on the fields that are meaningful.
Do not make bootstrap acceptance depend on an oracle that cannot be reproduced from the
repository.

### 5.3 Determinism rules to enforce

- No host wall-clock time, `Instant`, random number generator, process ID, thread ID,
  address value, or hash-map iteration order may enter emulated state, state hashes, or
  serialized snapshots.
- Prefer ordered containers (`BTreeMap`) in any serialized structure; the existing
  `MachineInspection` already uses `BTreeMap` for registers.
- State hash inputs are canonical bytes, not `Debug` output or field-order-dependent
  serialization.
- IDs are allocated by the session layer, never by backends, and are not array indexes
  (already documented in `chronofork-core::id`).
- Capability flags must be able to say "not supported" without constructing a fake
  implementation.

## 6. State, replay and isolation invariants

These invariants become the backbone of T2/T3/T5. Each needs a test ID in the test code
and a failure message naming the invariant.

| ID | Invariant | Why / upstream analogue | Test shape |
|---|---|---|---|
| INV-01 | A freshly created backend has a deterministic initial state and hash | Equivalence gate's "same reset state" requirement | Create twice, compare descriptor/tick/hash |
| INV-02 | Same snapshot + same input sequence → same tick and hash after N steps | Trace determinism | Run twice; compare hash sequence |
| INV-03 | Ticks advance monotonically and overflow is a typed error | `Tick::checked_add` contract | Unit + backend test at boundary |
| INV-04 | `snapshot()` → `restore()` → `snapshot()` yields identical bytes/hash | Hierarchical round-trip; BizHawk state managers | Round-trip at several ticks |
| INV-05 | Restore-then-continue equals continuous execution | Snapshot completeness | Compare hash sequence after restore vs uninterrupted run |
| INV-06 | Snapshot serialization is canonical and stable within a format version | BizHawk `SerializationStabilityTests` | Golden bytes/hash; change requires version bump |
| INV-07 | An incompatible backend id/version/format is rejected with `IncompatibleSnapshot`, never a panic or partial restore | Versioned snapshot envelope | Mutate each field; assert typed error and unchanged state |
| INV-08 | Two forks from the same checkpoint are independent; advancing one never changes the other or the parent | ChronoFork's core purpose; BizHawk rewind/state isolation | Fork twice, step differently, compare hashes |
| INV-09 | A factory never returns two handles to the same mutable machine | `BackendFactory` doc contract | Create two, step one, hash/state of the other unchanged |
| INV-10 | `descriptor`, `inspect`, `read_memory` and capability queries are side-effect free | Debugger inspection contract; `MachineInspection` doc | Hash before/after repeated calls |
| INV-11 | `write_memory` is a debugger path: deterministic effect, no emulated-bus side effects beyond the documented ones | `write_memory` doc contract | Write then inspect; assert no unexpected tick/flag change; document backend-specific semantics |
| INV-12 | A halted machine rejects `step` with `Halted` but still supports inspect/snapshot/restore | Halt semantics | Drive to halt; assert step error; snapshot/restore works |
| INV-13 | Replaying an input log from the initial state reproduces the final hash; replay from a checkpoint reproduces the suffix | TAS/movie replay analogue | Build a short input tape; replay and compare hashes |
| INV-14 | Handle/session scoping is explicit: what invalidates on reset vs shutdown vs reload, and IDs are never reused | exodus-for-ais session test: reset keeps handles, shutdown invalidates, reload gives new handles | Session tests with explicit lifecycle transition matrix |
| INV-15 | Reported capabilities match behaviour: a `false` capability cannot be exercised successfully; a `true` one can | `Capabilities` doc: false is a hard statement | For each flag, assert the contract test outcome |
| INV-16 | Persistence round-trips (serialize → deserialize → serialize) are stable and versioned | BizHawk serialization and state-manager tests | Golden files/bytes plus migration test |
| INV-17 | Independent backend instances are safe to step concurrently when the API permits it | `Backend: Send`; no shared mutable state | Two threads, independent instances, repeated hashes |
| INV-18 | Failed operations do not leave partially mutated state | Error handling | Induce `OutOfBounds`/`InvalidSnapshot`; verify usable state/hash unchanged or documented atomicity |
| INV-19 | Memory reads/writes report exact bounds and never panic | `BackendError::OutOfBounds` | Boundary table: 0, 1, len-1, len, len+1 |
| INV-20 | No hidden host time/entropy is observable in state or hashes | Determinism bans | Run under a guard that denies `SystemTime`/`Instant`/`rand` in state paths (compile or test) |

INV-04 through INV-09 are the core of ChronoFork's value proposition; they should be
implemented before any real emulation backend is attempted.

## 7. Fixtures and redistribution

### 7.1 Policy

**Allowed**

- Programs generated by a checked-in generator with a documented layout, a fixed seed
  where applicable, and an expected trace/hash derived from the generator.
- Hand-authored byte sequences small enough to review in-repo (e.g. an instruction
  encoding table), with a test for their purpose.
- Third-party test ROMs only if permissively licensed, with the license text, source URL,
  pinned commit, author, SHA-256 and a redistribution statement in the manifest, and only
  after review (see Q13 in the licensing assessment).
- External artifacts downloaded by CI from a pinned source and verified by SHA-256,
  never committed.
- Golden serialized states produced by ChronoFork itself, with the producing commit and
  expected hash recorded.

**Prohibited**

- Commercial ROMs, BIOS/firmware images, disc images, and derived assets.
- Upstream test archives, trace dumps, screenshots, savestates, or skill text.
- Any file whose license is missing, unclear, copyleft-incompatible, or
  non-commercial-only (e.g. BizHawk's PicoDrive bundle).
- Binaries from BizHawk or any other emulator.
- Any test data copied from `.research/upstream/`.

### 7.2 Fixture manifest

Every non-source test artifact must be listed in a manifest (for example
`tests/fixtures/manifest.toml`) with:

| Field | Meaning |
|---|---|
| `path` | Repository-relative path |
| `sha256` | Hash of the exact bytes |
| `kind` | `generated`, `hand-authored`, `third-party-test-rom`, `golden-state` |
| `source` | URL and pinned revision, or `chronofork` generator path |
| `author` / `copyright` | Required for third-party fixtures |
| `license` | SPDX identifier and path to license text |
| `redistributable` | Explicit boolean; CI fails if false |
| `generator` | Script and seed for generated fixtures |
| `expected` | Expected hash/trace/exit code, if applicable |
| `reviewed_by` / `reviewed_at` | Human sign-off for third-party fixtures |

### 7.3 Automated checks

| ID | Check | Failure mode |
|---|---|---|
| F1 | Every binary-like file under `tests/` and crate test directories is listed in the manifest | Unlisted fixture |
| F2 | Every manifest entry exists and its SHA-256 matches | Drift/tampering |
| F3 | Generated fixtures are reproducible: run the generator with the recorded seed, compare hash | Non-reproducible fixture |
| F4 | Third-party entries have license text, source, revision and `redistributable = true` | License/provenance gap |
| F5 | No prohibited extension/type is tracked without an explicitly reviewed entry | ROM/BIOS/archive leak |
| F6 | No fixture came from `.research/upstream/` (path/content check) | Upstream contamination |
| F7 | Fixture size cap (e.g. 1 MiB per file, configurable) and total cap | Accidental large dump |
| F8 | Golden states include the producing ChronoFork version and format version | Stale golden file |
| F9 | A "fixture provenance" document/test lists every third-party artifact and its license | Missing attribution |
| F10 | CI re-runs F1–F8 on every change to `tests/` | Policy bypass |

F1–F8 should be a single checked script (for example `scripts/check-fixtures.sh`) or a
small test-support crate if one is added; the lead owns wiring it. This document
specifies the contract, not the code.

### 7.4 Adding a fixture (review checklist)

1. Can it be generated instead? If yes, generate it and commit the generator, not the
   bytes.
2. Is the license permissive and compatible with MIT distribution? Attach the text.
3. Is the source URL and exact revision recorded? Is the hash recorded?
4. Does the fixture contain any game code, firmware, or extracted asset? If yes, reject.
5. Is the expected result derived independently (program definition/spec) rather than
   copied from a run that might be wrong?
6. Is the fixture small, reviewable, and free of personal data?
7. Add the manifest entry and run F1–F8 before review.

## 8. Initial quality gates

Gates are ordered from cheap to expensive. **G0, G1 and the rustdoc part of G4 are
runnable now** (verified in §2.3); **G2 is partial** (in-process determinism and state
invariants pass, cross-process and release-mode checks are not yet wired); **G3, G5 and
the remaining G4 checks are proposed**; G6/G7 are release/nightly. `scripts/check.sh` is
the local entry point for G0/G1/G4.

### G0 — Workspace hygiene (every change)

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

The workspace sets `unsafe_code = "forbid"` and denies `unwrap_used`, `panic`, `todo` and
`unimplemented`; `-D warnings` promotes any violation to a gate. `expect_used` is allowed
for test diagnostics but must not appear in production library paths. Do not add blanket
`#![allow(...)]`; targeted allows need a comment.

### G1 — Unit and contract correctness (every change)

```sh
cargo test --workspace --lib
cargo test --workspace --tests
cargo test --workspace --doc
```

Required coverage at bootstrap:

- T1 core primitives (tick, hash, IDs, input) — present.
- T2 backend contract against `chronofork-test-backend` — present as backend unit
  tests; the reusable cross-backend conformance suite is still to be extracted.
- T5 session lifecycle tests — present.
- T6 CLI acceptance tests — present for the happy path.

`TreatNoTestsAsError` equivalent: the CI job must fail if a test binary discovers zero
tests, so an accidentally disabled suite cannot look green.

### G2 — Determinism and state gate (every change)

```sh
cargo test --workspace
cargo test -p chronofork-sessions --test acceptance -- --test-threads=1
cargo test --workspace --release
```

The exact consolidated `determinism` target does not exist yet; the three commands above
are the current substitute and should be replaced by (or wrapped in) a dedicated target.

The gate must:

- run the same scenario twice and compare tick/hash sequences;
- run debug and release and compare hashes;
- snapshot → restore → compare hash;
- restore-and-continue vs continuous run;
- fork-and-diverge vs parent;
- prove the snapshot format is stable for its version;
- include a negative control (corrupt a record/checkpoint and assert failure).

### G3 — Provenance and supply chain (every change; required before release)

```sh
cargo deny check licenses advisories bans sources
cargo audit            # optional if cargo-deny advisories is configured
# fixture manifest check (F1–F8), once wired
```

Policy baseline:

- Allow: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, CC0-1.0.
- Deny: GPL, AGPL, LGPL in distributed crates, SSPL, non-commercial, unknown.
- Deny wildcard/path dependencies; pin `Cargo.lock`; review new dependencies in PRs.
- No dependency may pull in upstream emulator code or ROM assets.
- `THIRD_PARTY_NOTICES.md` exists and is generated/checked before release.

### G4 — Documentation gate (every change; release-blocking)

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

This is already part of `scripts/check.sh` and passed on 2026-11-13. Still to add:
markdown link checks for `docs/`, and either enable `missing_docs` or review that every
public API has a doc comment. The existing backend-api docs already explain capability
semantics; keep that standard.

### G5 — Acceptance and CLI gate (every change; release-blocking)

- End-to-end workflows in `crates/chronofork-cli/tests/`: create a session, step, snapshot,
  fork, restore, inspect, shut down; assert exit codes, stable JSON fields, and error
  messages.
- Failure-path tests: missing snapshot, incompatible snapshot, out-of-bounds, halted
  machine, invalid input.
- Output must be machine-parseable and stable; no diagnostics on stdout where stdout is a
  data channel (the exodus-for-ais protocol test is the model).

### G6 — Release reproducibility (release)

```sh
cargo build --workspace --release --locked
cargo test --workspace --release --locked
cargo +1.85.0 check --workspace --locked   # MSRV check; advisory until the toolchain is pinned in CI
```

Also: clean checkout build, vendored-dependency audit, `cargo metadata` license check,
and two builds from the same commit compared for the artifacts that are expected to be
reproducible (at minimum, deterministic test outputs and state hashes; binary
reproducibility can be a later goal).

### G7 — Extended/nightly

- Longer trace corpus and cross-process/cross-platform differential runs (D2, D6, D7).
- Fuzz targets (T8) with a time budget; failures are minimized and filed.
- Known-failure quarantine run (`BIZHAWKTEST_RUN_KNOWN_FAILURES` analogue): a separate
  job runs quarantined cases and fails if one unexpectedly passes.
- Performance trend report (T10), non-blocking but tracked.

## 9. CI topology and reporting

| Job | Trigger | Contents |
|---|---|---|
| `fmt-lint` | every PR/push | G0 |
| `test` | every PR/push, after `fmt-lint` | G1, then G2, then G3 |
| `docs` | every PR/push | G4 |
| `acceptance` | every PR/push | G5 |
| `msrv-release` | main/tags | G6 |
| `extended` | nightly/schedule | G7 |

Reporting requirements:

- JUnit XML artifacts for test jobs (the BizHawk `test_output/*.coverage.xml` pattern is
  a useful precedent); a failing job must link the failing test and the invariant ID.
- Per-suite timeouts, with a whole-job timeout; no unbounded integration waits.
- Cache the Cargo registry and `target/` keyed on `Cargo.lock`, never cache downloaded
  fixtures without hash verification.
- Do not put ROMs/firmware in CI caches or artifacts.
- The determinism job should run serially first (`--test-threads=1`) and then in the
  default parallel mode to catch both state leakage and parallel isolation issues.

## 10. Bootstrap acceptance criteria

Status at 2026-11-13 (the implementation may advance after this audit):

| # | Criterion | Status | Evidence / gap |
|---|---|---|---|
| 1 | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass from a clean checkout | **Pass** | Verified in §2.3 (bootstrap, 40 tests) and in the 2026-11-14 stage-1b gate (98 tests) |
| 2 | A reusable `Backend` contract suite exists and passes against `chronofork-test-backend` | **Partial** | Backend unit tests cover determinism, divergence, snapshot, bounds, errors; not yet extracted as a shared conformance suite |
| 3 | INV-01…INV-13 covered by executable tests, including negative control, debug/release determinism, snapshot round-trip, restore-continue and fork isolation | **Partial** | In-process invariants covered; debug/release and cross-process determinism absent; negative control exists for snapshot corruption and for the E2-12 oracle projection comparator |
| 4 | Snapshot format versioning has an `IncompatibleSnapshot` test and a canonical golden state | **Partial** | Version/corruption/truncation rejection present; no golden bytes/canonical fixture |
| 5 | CLI acceptance covers create → step → snapshot → fork → restore → inspect → shutdown, plus main error paths | **Partial** | Demo covers the happy path end to end; error paths and exit codes not tested |
| 6 | Fixture policy implemented: manifest, F1–F8 checks, no prohibited files | **Missing** | No fixtures and no manifest/check yet |
| 7 | `cargo deny` (or equivalent) configured with allow/deny list and run in CI | **Missing** | No `deny.toml`; one direct dependency today |
| 8 | `THIRD_PARTY_NOTICES.md` matches the licensing assessment; no Q4-blocked code copied | **Pass** | `THIRD_PARTY_NOTICES.md`, `LICENSE`, `docs/licensing/provenance.md` present; no upstream code copied |
| 9 | A CI workflow runs the gates and publishes JUnit artifacts | **Missing** | No workflow file; `scripts/check.sh` is local-only |
| 10 | Docs build with `-D warnings`; every public item has a doc comment | **Partial** | `RUSTDOCFLAGS="-D warnings" cargo doc` passes; `missing_docs` is not enabled and markdown link checking is not wired |

The bootstrap's test/quality work is complete when all ten are **Pass**. The most
important remaining work is CI wiring (9), the canonical snapshot fixture (4), the CLI
error paths (5), and the fixture/dependency policies (6, 7) — they protect every future
change rather than adding one more scenario.

## 11. Open questions for the user/lead

| ID | Question | Impact on this strategy |
|---|---|---|
| TQ1 | Is audio (and therefore the YM2612 question) in scope for the first backend? | If yes, Q4 in the licensing assessment blocks any direct reuse of Exodus's tables; if no, the determinism/state gates remain the priority |
| TQ2 | May exodus-for-ais be used as a locally built differential oracle? | Enables D6; otherwise rely on generated expectations and later real-backend comparisons |
| TQ3 | Generated fixtures only, or permissively licensed third-party test ROMs with a manifest? | Determines how much F4/F9 machinery is needed at bootstrap |
| TQ4 | Determinism scope: Linux only at first, or Linux/macOS/Windows from day one? | Determines whether D7 is a bootstrap gate or a release gate |
| TQ5 | Snapshot format stability promise: internal-only until v1, or stable from the first commit? | Determines golden-state churn and migration-test obligations |
| TQ6 | Is fuzzing required at bootstrap or deferred to nightly? | Affects T8 timing and CI budget |
| TQ7 | Which CI provider and runner sizes? | Affects job topology, timeouts and caching; no workflow exists at audit time |
| TQ8 | Coverage target (line/branch) or invariant coverage only? | Prefer invariant coverage; a naive line target can reward trivial tests |
| TQ9 | May test-only dependencies include `proptest`/`criterion`/`cargo-fuzz` under the same license policy? | They are dev-dependencies; confirm the allowlist before adopting |
| TQ10 | Is the deterministic test backend expected to model errors/halting/overflow for contract testing, or only happy paths? | The contract suite needs a backend that can deliberately fail in controlled ways |

## 12. Evidence index

ChronoFork files referenced by this strategy: `scripts/check.sh`,
`docs/quality/quality-gates.md`, `docs/licensing/policy.md`,
`docs/licensing/provenance.md`, `crates/chronofork-core/src/`,
`crates/chronofork-backend-api/src/`, `crates/chronofork-sessions/src/` and
`tests/`, `crates/chronofork-test-backend/src/`, `crates/chronofork-cli/src/` and
`tests/`, and `THIRD_PARTY_NOTICES.md`.

Upstream paths referenced above (all under `.research/upstream/` at the pinned commits):

- Exodus: `ExodusTests.sln`, `Support Libraries/MarshalSupport/Tests/`,
  `Data/Modules/MegaDriveTestSystem*.xml`.
- exodus-for-ais: `host/tests/PathSemanticsTest.cpp`, `host/tests/build_and_run.sh`,
  `mcp/tests/JsonTest.cpp`, `mcp/tests/TextCodecTest.cpp`, `mcp/tests/session_test.py`,
  `mcp/tests/build_and_run.sh`, `tests/rom/make_test_rom.py`, `tests/rom/rom_test.py`,
  `tests/rom/build_and_run.sh`, `tests/equivalence/compare_traces.py`,
  `tests/equivalence/README.md`, `tests/hierarchicalstorage/RoundTripTest.cpp`,
  `tests/hierarchicalstorage/build_and_run.sh`, `README.md` (equivalence and "You supply
  the software" sections), `AGENTS.md` (determinism and reporting sections).
- BizHawk: `src/BizHawk.Tests.Common/`, `src/BizHawk.Tests.Emulation.Common/`,
  `src/BizHawk.Tests.Client.Common/` (Movie, config, dearchive, Lua, API tests),
  `src/BizHawk.Tests.Testroms*/`, `src/Testroms.props`, `src/TestProjects.props`,
  `src/MainSln.runsettings`, `src/BannedSymbols.BannedApiAnalyzers.txt`,
  `src/BizHawk.Tests.Client.Common/data/dearchive/readme.md`,
  `src/BizHawk.Tests.Testroms/.download_from_ci.sh`, `.gitlab-ci.yml` (stage `test`,
  job `run_tests`), `src/BizHawk.Tests.Testroms.GB/readme.md`,
  `src/BizHawk.Tests.Testroms.GB.GambatteSuite/readme.md`.

The upstream tests themselves must not be copied. If a specific test is ever adapted,
treat it as a port under the licensing assessment's notice rules, record the source
file/revision in a provenance manifest, and re-express it in Rust rather than
transliterating it.
