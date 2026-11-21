# M3 Partial Replay, Seek and Branch Comparison — Executable Acceptance Plan

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** Fase A design accepted (the project owner approved the staged plan on
  2026-11-15); the increment is **implemented and independently verified** for the
  deterministic synthetic backend (t5 PASS, 2026-11-15). Every acceptance row below is
  `executed - passed`.
- **Date:** Fase A design 2026-11-15 (owner-approved); execution record 2026-11-15
- **Companion contract:** [ADR-0005: partial replay, seek and branch
  comparison](../adr/0005-partial-replay-and-branch-comparison.md) (Accepted
  2026-11-15; implemented and independently verified for the deterministic synthetic
  backend). The accepted v1 format contract it extends is
  [ADR-0004](../adr/0004-tas-movie-format.md) (Accepted; implemented and independently
  verified for the deterministic synthetic backend, M3-T4 PASS).
- **Related:** [execution model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [M3 first-increment acceptance
  plan](m3-tas-replay-acceptance.md), [M2 durable-sessions acceptance
  plan](m2-durable-sessions-acceptance.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md),
  [milestones](../roadmap/milestones.md), [testing strategy](testing-strategy.md),
  [anti-slop policy](anti-slop-policy.md), [quality gates](quality-gates.md),
  [research: TAS and debugging](../research/tas-and-debugging.md).

> **Verification record (2026-11-15).** The independent t5 verifier executed
> M3-A13–A17 on the uncommitted increment-2 tree at HEAD
> `c6c502de54932459a1d3bc0cd0ada10ceba97043` (branch `main`, equal to `origin/main`)
> plus the increment-2 working-tree paths (sessions `replay.rs`, `lib.rs`, `movie.rs`,
> `tests/replay.rs`; CLI `main.rs`, `tas.rs`, `tests/tas.rs`; this contract set) with
> `CARGO_HOME=/tmp/t7-cargo`, and returned **PASS**. The full gate, M2 and increment-1
> v1 regression, both demos and the checkpoint probe are green; per-row commands,
> results and artifact names are in §5 and §9. Raw evidence is in the ignored
> `.research/m3-tas-inc2/verification/` directory (`SUMMARY.txt`,
> `target-matrix.txt`, `A13-*.txt`–`A17-*.txt`, `A13-A17-execution-record.txt`,
> `acceptance-checks.txt`, `logs/`, `targets/`, `cli/`, `demos/`, `probes/`,
> `MANIFEST.sha256`); the pre-change baseline is
> `.research/m3-tas-inc2/baseline/`. The dated gate-result block is in
> [quality gates](quality-gates.md). M3 is **not complete**: the deferred items in §10
> remain unimplemented. t5 observation O-1 (the process-level spelling for an
> import-incompatible embedded checkpoint) is recorded in the M3-A17 row; O-3 is
> behavior-equal with no code or documentation change required.

> **Owner approval (2026-11-15).** The project owner approved the staged M3 increment 2
> plan `chronofork-m3-inc2` through the Web pre-run review UI; the approval committed at
> 2026-11-15T12:35:03-03:00 (America/Fortaleza).
> [ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md) is Accepted, and its
> implementation is now independently verified by the t5 record above. The approved
> decisions and the deferred alternatives are in
> [ADR-0005 §8 and
> §10](../adr/0005-partial-replay-and-branch-comparison.md#8-deferred-work-and-stated-limitations).

## 1. Status rules

1. **`planned - not yet executed`** means the criterion has a planned test name,
   command and evidence artifact, but no executed result or verifier evidence exists.
   Before the t5 execution record this was the only status any row in this document had.
2. A row becomes **`executed - passed`** only when all of the following are recorded:
   the exact command, exit code, tree state (HEAD plus uncommitted paths), test output
   summary, the ignored evidence artifact path and an independent verifier review that
   returns PASS. The authoritative gate record belongs in
   [quality gates](quality-gates.md); this document links to it and does not copy
   volatile counts.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact or unverified claim; a failed row blocks this increment. A known
   failure is never converted into a skip, an `#[ignore]`, a relaxed assertion or a
   narrower claim after the fact.
4. A test may be committed before the implementation as a red test only if it fails for
   the expected typed reason. A test that cannot run is not acceptance evidence.
5. No behavior in this increment may be described as implemented, working, supported or
   complete on the strength of ADR-0005, this plan, a code review, a demo run or a single
   happy-path run. The v1 format and M3-A1–A12 are evidence only for v1, not for this
   increment.
6. A planned test whose command cannot execute because its module or test target does not
   exist remains `planned - not yet executed`; it does not become "passed" because the
   absent target produced no failure.
7. A row marked passed must cite a verifier artifact whose SHA-256 matches the recorded
   manifest. Late or reconstructed evidence does not reopen a failed row.
8. A row may not be satisfied by re-running an existing v1 capability. Each row requires
   the new ranged replay, range rejection, comparison or CLI behavior described here; a
   green M3-A1–A12 run is a regression precondition, never acceptance for A13–A17.

As of the 2026-11-15 t5 execution record every M3-A13–A17 row is `executed - passed`;
none is failed.

## 2. Scope, definitions and evidence

- **Partial replay:** execution of a recorded step sequence over an explicit tick
  window, always starting from the movie's embedded checkpoint in a fresh imported
  session; the embedded checkpoint is the only anchor (ADR-0005 §2).
- **Aligned tick:** the movie's `start_tick`, its `end_tick`, or an existing recorded
  step outcome tick. A boundary that is not aligned is rejected, never rounded.
- **Range:** the pair `(from, until]` with `from <= until`, both aligned. The default
  full range is `[start_tick, end_tick]`; an omitted `--from` means the start tick and an
  omitted `--until` means the end tick, so no range is open-ended or unbounded.
- **Window and prefix:** the window is the recorded steps with `from < outcome_tick <=
  until`; the prefix is the recorded steps with `outcome_tick <= from`. Prefix steps are
  executed **and verified**, but not reported as the window (ADR-0005 §3).
- **Frame advance:** one backend step, which is exactly one tick on the synthetic
  acceptance backend. Expressed by a new replay whose window contains the next recorded
  outcome tick. This is not an interactive stepping loop; no state is carried between
  invocations.
- **Rewind:** a new replay invocation with an earlier `until` (or an earlier
  `from`/`until` pair). It re-simulates from the embedded checkpoint in a fresh session;
  there is no in-process rewind buffer and no carried state.
- **Compare:** a value comparison of two decoded movies' recorded step records by index
  on `(outcome_tick, state_hash)`, with embedded checkpoint identity (start tick and
  checkpoint state hash) and step-count differences reported separately. Compare reads
  recorded expectations and does **not** re-simulate, import a checkpoint or decide
  backend compatibility (ADR-0005 §4). Verification remains the re-simulation path.
- **First divergence:** the first recorded step, in file order, whose recorded
  `(outcome_tick, state_hash)` differs between the two movies, reported as `index`,
  `tick_a`, `tick_b`, `state_hash_a` and `state_hash_b`. No later index is reported
  instead.
- **Contract metadata is not equality:** `movie_id`, `name` and `parent_movie_id` never
  decide comparison equality; input masks are not equality fields in this increment.
- **Source and bystander sessions:** any session that existed before a replay or compare
  operation. Its backend tick, state hash and mutable state must be unchanged, and a
  valid replay may register only its own one fresh imported session.
- **Acceptance backend:** the deterministic synthetic backend
  (`chronofork-test-backend`) is the only acceptance backend. Its `APPLY_INPUT` consumes
  one input per step and one step advances one tick. The Mega Drive slice is not an
  acceptance backend for input TAS because its `step` ignores input, so real-hardware
  input TAS is deferred (ADR-0005 §8).
- **Platform:** Linux is the acceptance platform, matching the M2 and M3 first-increment
  evidence platform. Windows and macOS are compile-checked in CI; runtime behavior of
  partial replay or comparison there is not claimed.
- **Evidence artifacts:** raw output is stored under the ignored
  `.research/m3-tas-inc2/verification/` path: `SUMMARY.txt`, `target-matrix.txt`,
  `A13-*.txt` … `A17-*.txt`, `A13-A17-execution-record.txt`, `acceptance-checks.txt`,
  `logs/`, `targets/`, `cli/`, `demos/`, `probes/` and `MANIFEST.sha256`. The pre-change
  baseline is `.research/m3-tas-inc2/baseline/` (t1). Each artifact records the command,
  tree state, exit code and the relevant stdout/stderr/SHA-256; the manifest binds the
  artifact set. Artifacts are diagnostic evidence, not durable claims; this document
  summarizes results without volatile counts. The dated gate-result block is in
  [quality gates](quality-gates.md).
- **No host time in canonical data or reports:** movie bytes contain no timestamp,
  process id, host label, nonce or path (ADR-0004 §3). Replay ranges, comparison verdicts
  and reports use ticks, state hashes, step indices, movie fields and SHA-256, never
  wall-clock time or filesystem modification time.

## 3. Increment exit criteria map

The capabilities delivered by this increment are listed below. Each maps to acceptance
rows that are `executed - passed` in the t5 record.

| Increment capability (delivered) | Acceptance criteria |
|---|---|
| Partial replay over an explicit window; prefix equality; forward single-tick advance; rewind by a new invocation | M3-A13 |
| Typed range rejection before any checkpoint import, execution or mutation | M3-A14 |
| Branch comparison of recorded expectations, with separate length and checkpoint-identity reports | M3-A15 |
| v1/M2 non-regression, non-mutation, byte-identical demos and the full gate | M3-A16 |
| CLI `play`/`verify --from/--until` and `tas compare` process contract, help and corruption handling | M3-A17 |

**This increment is complete:** every row in this document is `executed - passed`, the
M3 first-increment rows (M3-A1–A12) and the M2 acceptance rows passed unchanged, the full
repository gate and both demos are byte-identical to the increment baseline, and the
independent verifier returned PASS bound to the tree under test (§9).

**This increment does not complete M3.** The milestone and roadmap entries now distinguish
the delivered linear re-simulation path from the deferred checkpoint-anchored path.
Periodic anchors/checkpoints and seek via checkpoints (a future format v2), an in-process
rewind buffer/state cache and interactive stepping, rerecord and branch editing,
multi-controller/ports/analog/keyboard input, real-hardware input TAS, import/export
adapters and the UI/HTTP/MCP surfaces remain deferred (ADR-0005 §8) and unchecked.

## 4. Executed test targets (t5)

These targets exist and were executed on the increment-2 tree with
`CARGO_HOME=/tmp/t7-cargo`. Per-target counts and the increment-baseline comparison are in
`.research/m3-tas-inc2/verification/target-matrix.txt`; raw stdout/stderr is in
`targets/` and `logs/`. The target command runs every test in the target; exact counts
stay in the evidence, not in this durable document.

| Target / command | Executed command | Criteria | Result |
|---|---|---|---|
| Replay/comparison unit tests in `crates/chronofork-sessions/src/replay.rs` (range validation, window/prefix arithmetic, comparison verdicts) | `cargo test -p chronofork-sessions --lib` | M3-A13, M3-A14, M3-A15 | exit 0 (t5) |
| Integration target `crates/chronofork-sessions/tests/replay.rs` (ranged replay, seek, rewind, prefix soundness, compare, non-mutation) | `cargo test -p chronofork-sessions --test replay -- --nocapture` | M3-A13, M3-A14, M3-A15, M3-A16 | exit 0 (t5) |
| Existing movie targets (delegation equality and v1 regression) | `cargo test -p chronofork-sessions --test movie` and `cargo test -p chronofork-sessions --release --test movie` | M3-A13, M3-A16 | exit 0 (t5) |
| Release-mode replay comparison | `cargo test -p chronofork-sessions --release --test replay` | M3-A16 | exit 0 (t5) |
| CLI process target `crates/chronofork-cli/tests/tas.rs` (ranged play/verify, compare, flag rejection, corruption, help, default-output equality) | `cargo test -p chronofork-cli --test tas` | M3-A13, M3-A15, M3-A17 | exit 0 (t5) |
| Existing CLI regression | `cargo test -p chronofork-cli` | M3-A16, M3-A17 | exit 0 (t5) |
| M2 session target matrix plus `cargo test -p chronofork-cli --test session_storage` | the M2 matrix commands from the [M3 plan §4](m3-tas-replay-acceptance.md#4-executed-test-targets-m3-t4) | M3-A16 | every target exit 0; counts equal t1 (t5) |
| Full gate | `./scripts/check.sh`; `cargo fmt --all -- --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`; `git diff --check` | M3-A16, M3-A17 | exit 0 (t5) |
| Demos | `cargo run -p chronofork-cli -- demo` and `cargo run -p chronofork-cli -- megadrive demo`, three runs each, `cmp` and SHA-256 against the increment baseline | M3-A16 | exit 0; all six runs byte-identical to baseline (t5) |
| Serialized-state probe | the baseline `session save/load` probe on two independent roots | M3-A16 | exit 0; identical to baseline (t5) |
| Verifier probe and CLI harness | the verifier-authored public-API probe (debug and release) and the CLI acceptance harness under `cli/`, `acceptance-checks.txt` | M3-A13–A17 | PASS (t5) |

## 5. Acceptance criteria

Every row below is `executed - passed` in the t5 record. The planned assertions are
retained; each row records the observed result and the evidence artifacts without
volatile counts. The typed categories and boundaries are fixed by
[ADR-0005 §3, §4 and §6](../adr/0005-partial-replay-and-branch-comparison.md#3-replay-semantics-range-boundaries-prefix-frame-advance-and-rewind);
the implemented spelling is recorded in the row results.

### M3-A13 — Partial replay: prefix equality, seek, forward advance and rewind

**Passes only when** all of the following are demonstrated on the deterministic
synthetic backend:

1. **Delegation equality with v1.** For the increment fixture movies (a zero-step movie,
   a one-step movie, a multi-step movie, and a movie seeded by an M2 committed
   checkpoint), the full-range replay through the new single ranged path produces exactly
   the same `ReplayReport` field values and the same first-divergence behavior as the
   increment baseline's `Movie::verify`; the replay loop and the checkpoint import exist
   exactly once (structural check plus behavior equality).
2. **Window semantics with prefix verification.** For a movie with `n >= 2` and a `from`
   equal to the recorded outcome tick `t_k` (`0 <= k < n - 1`), a replay over
   `(t_k, end_tick]` reports `range_from = t_k`, `range_until = end_tick`,
   `prefix_steps = k + 1`, `window_steps = n - 1 - k`, `replayed_steps = n`,
   `final_tick = end_tick` and a `final_state_hash` equal to both the recorded `h_{n-1}`
   and the full-range final hash. The prefix is executed and verified: a movie altered at
   a prefix step `j < k` and re-encoded with a valid trailing checksum fails at `j` with
   the existing typed divergence, never as a passing window.
3. **Seek to an earlier boundary.** A replay over `[start_tick, t_j]` reports
   `final_tick = t_j` and `final_state_hash = h_j`, and that hash equals the state hash
   observed at step `j` during a full replay. Boundary cases are covered: `from =
   start_tick` (empty prefix), `until = t_0` (single-step window), `from = t_{n-2}`,
   `until = end_tick`, and `from = start_tick, until = end_tick` (full range).
4. **Forward advance by one tick.** A replay whose window is exactly one recorded step
   (`from = t_k`, `until = t_{k+1}`, including `from = start_tick, until = t_0`) reports
   `window_steps = 1`, `final_tick = t_{k+1}` and `final_state_hash = h_{k+1}`. This is
   the frame-advance contract: one backend step, one tick on this backend, as a new
   invocation. A request to advance to a tick that is not a recorded outcome tick is
   rejected by M3-A14 and is never interpolated.
5. **Rewind by a new invocation.** After a replay to `end_tick`, a second invocation with
   `until = t_j` (`j < n - 1`) returns exactly the same outcome as a first-ever
   invocation over the same range: same ticks, hashes and window/prefix counts. No state
   is carried between invocations, no session is reused, and a pre-existing bystander
   session's tick and state hash are unchanged. Each valid replay registers exactly one
   fresh imported session in the caller's manager.
6. **Zero-step movie.** Its only valid range is the full range `[start_tick, start_tick]`:
   zero executed steps, `prefix_steps = 0`, `window_steps = 0`, `final_tick =
   start_tick`, `final_state_hash` equal to the embedded checkpoint state hash, matching
   the M3-A3 contract.
7. **Deterministic reports.** Two identical ranged replays of the same fixture movie and
   range, in separate invocations (and in separate processes for the CLI path), report
   identical ticks, hashes and counts; no host-dependent value appears in a report.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Replay unit tests | `cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt` |
| Replay integration test | `cargo test -p chronofork-sessions --test replay -- --nocapture` | `targets/sessions-replay.stdout.txt`, `cli/A13-partial-replay.txt` |
| Movie delegation equality | `cargo test -p chronofork-sessions --test movie` | `targets/sessions-movie.stdout.txt` |
| CLI ranged replay | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `cli/A13-*` |
| Manual seek/rewind transcript | separate-process ranged `tas play`/`tas verify` invocations with parsed fields and exit codes | `cli/A13-seek-rewind-transcript.txt` |

**Observed result:** PASS. Prefix equality and suffix window counts matched the recorded
boundaries; a one-step window reported `window_steps = 1` and the next recorded state
hash; seek ended at the recorded hash; repeat and rewind invocations in separate processes
reported identical outcomes with no carried state; the zero-step movie accepted only its
full range. An independently written public-API probe re-checked recorded-hash equality
at sampled boundaries. Evidence: `targets/sessions-lib.stdout.txt`,
`targets/sessions-replay.stdout.txt`, `cli/A13-partial-replay.txt`,
`cli/A13-seek-rewind-transcript.txt`, `probe-transcript-debug.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A14 — Typed range rejection before import, execution or mutation

**Passes only when** every requested range below returns its named typed category
without a panic, without a partial outcome, without importing a checkpoint, without
executing a backend step, without creating a session and without mutating any movie,
session or file. The exact spelling is an implementation detail; the categories and their
boundaries are fixed by
[ADR-0005 §3 and §6](../adr/0005-partial-replay-and-branch-comparison.md#3-replay-semantics-range-boundaries-prefix-frame-advance-and-rewind).

| Requested boundary | Required typed result |
|---|---|
| `from < start_tick` (fixture movie with `start_tick > 0`, e.g. the M2-seeded fixture) | before-start, naming both ticks |
| `until > end_tick` | after-end, naming both ticks |
| `from > until` | inverted, naming both ticks |
| `from` not aligned (for example `start_tick + 1` when that is not a recorded outcome tick, or a tick strictly between two recorded ticks) | unaligned, naming the offending tick |
| `until` not aligned | unaligned, naming the offending tick |
| `from == until` on a movie with at least one step | empty window |
| `from == end_tick` with `until == end_tick` on a movie with steps | empty window |
| `until == start_tick` with `from` omitted (defaults to `start_tick`) on a movie with steps | empty window |
| `u64::MAX` for `from` or `until` (and tick values near `u64::MAX`) | the applicable before-start, after-end or inverted category; no overflow, wrap or panic |
| Zero-step movie, default full range | accepted (negative control; see M3-A13 item 6) |
| `from = t_k`, `until = t_m` with `0 <= k < m <= n-1` | accepted (negative control) |

In addition:

1. A rejected range returns before importing the checkpoint, which is demonstrated
   executably: a structurally valid movie whose embedded checkpoint this build would
   reject at import (foreign backend identity or configuration) returns the **range
   error**, not a typed snapshot-import failure, when an invalid range is requested.
2. A rejected range leaves the caller's `SessionManager` session count unchanged and
   does not execute any backend step; there is no partially usable outcome value.
3. Range validation is pure arithmetic over already-decoded ticks: it performs no
   allocation proportional to the requested range or step count, does not decode the
   embedded checkpoint a second time, and performs no file I/O.
4. Arbitrary 64-bit tick values never panic and never wrap; a range is never clamped,
   rounded to a neighbouring tick or reinterpreted.
5. CLI flag-level failures (missing value, non-numeric value, duplicate flag, unknown
   flag) are typed CLI errors with exit 1 and empty stdout; the process contract and its
   cases are owned by M3-A17.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Range unit tests | `cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt`, `cli/A14-range-matrix.txt` |
| Range integration tests | `cargo test -p chronofork-sessions --test replay -- --nocapture` | `targets/sessions-replay.stdout.txt`, `cli/A14-range-matrix.txt` |
| Reject-before-import proof | `cargo test -p chronofork-sessions --test replay -- --nocapture` | `cli/A14-range-matrix.txt` |
| CLI flag rejection | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `cli/A14-*` |

**Observed result:** PASS. Before-start, after-end, inverted, from-unaligned,
until-unaligned, empty (equal boundaries and `until == start`) and `u64::MAX` ranges
returned their typed variants with no panic or wrap; a rejected range registered no
session and executed no step; an import-incompatible (foreign configuration) movie
returned the range error for an invalid range and the typed snapshot-import failure for a
valid range; CLI invalid ranges exited 1 with empty stdout and a `chronofork:` diagnostic.
Evidence: `cli/A14-range-matrix.txt`, `targets/sessions-lib.stdout.txt`,
`targets/sessions-replay.stdout.txt`, `probe-transcript-debug.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A15 — Branch comparison of recorded expectations

**Passes only when** comparison is shown to read recorded step records only, never to
re-simulate, and to report in this order: embedded checkpoint identity, first
`(outcome_tick, state_hash)` divergence, step-count mismatch, identical. Reported fields
must name both movies' values; a mismatch is a typed report value, not an `Err`.

| Case | Required comparison result |
|---|---|
| A movie compared with itself | identical |
| Two independently recorded movies with the same checkpoint, metadata and inputs | identical |
| Same checkpoint and step records, different `movie_id`, `name` and/or `parent_movie_id` | identical (metadata never decides equality) |
| One recorded step hash altered at index `k`, valid trailing checksum | first divergence at `k`; `tick_a == tick_b == t_k`; `state_hash_a` original, `state_hash_b` altered; later indices not reported |
| One recorded outcome tick altered at index `k` to another valid strictly-increasing tick | first divergence at `k`; ticks differ, hashes equal |
| Tick and hash altered at the same index `k` | first divergence at `k`; both fields differ |
| Alterations at several indices | the earliest altered index is reported |
| Same common prefix, right movie has one extra valid step | step-count mismatch with `common_steps = n`, `steps_a = n`, `steps_b = n + 1`; no divergence |
| Same common prefix, left movie has one fewer valid step | step-count mismatch with the symmetric counts |
| A first divergence in the common prefix **and** different step counts | the divergence verdict at its exact index; both step counts visible in the report |
| Different embedded checkpoint start tick (for example a checkpoint captured later) | checkpoint mismatch with both start ticks and both checkpoint state hashes; no step divergence reported |
| Same start tick, different embedded checkpoint state hash | checkpoint mismatch naming both checkpoint hashes; no step divergence reported |
| Same recorded `(outcome_tick, state_hash)` sequence but different input masks | identical (input masks are not equality fields in this increment) |
| Structurally valid movie whose embedded checkpoint this build would reject at import | comparison still returns a verdict (it imports nothing); `verify` on the same movie fails with the typed snapshot-import incompatibility |

In addition:

1. **No re-simulation, structurally.** The comparison entry point takes only the two
   decoded movies; it takes no `SessionManager`, does not call
   `create_session_from_snapshot_bytes`, does not call `validate_envelope`, does not call
   `Backend::step` and performs no file I/O. A structural audit records this.
2. **No re-simulation, executably.** The incompatible-checkpoint case above proves the
   import path is not used, and a compare call leaves a caller's manager session set
   unchanged (or the API has no manager; both are recorded).
3. **Cost and bounds.** Comparison is O(min(n, m)) over already-decoded records, bounded
   by the v1 `MAX_STEPS` limit; it allocates no per-step buffer and executes no backend
   step.
4. **Malformed input.** A malformed movie still fails at decode with the existing typed
   `MovieError` before any comparison; comparison never sees an unvalidated movie.
5. **Report shape.** The report carries `identical` or one mismatch verdict plus both
   start ticks, both checkpoint state hashes, both step counts and, where applicable, the
   divergence index and both tick/hash values; no stringly error and no panic on any pair
   of decoded movies.
6. **Compare is not verification.** A compare PASS on two movies that both fail `verify`
   on this build is reported as identical, and the docs/help state that this is by
   contract (ADR-0005 §4); no test or document claims compare proves determinism.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Comparison unit tests | `cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt` |
| Comparison integration tests | `cargo test -p chronofork-sessions --test replay -- --nocapture` | `targets/sessions-replay.stdout.txt`, `cli/A15-compare.txt` |
| CLI compare behavior | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `cli/A15-*` |
| No-re-simulation structural audit | `rg -n "SessionManager\|create_session_from_snapshot_bytes\|validate_envelope\|\.step\(" crates/chronofork-sessions/src/replay.rs` and review of the comparison entry point | `cli/A15-compare.txt` |

**Observed result:** PASS. Self-comparison, two independent recordings, metadata-only and
input-mask-only differences compared identical; a hash divergence and a tick-only
divergence were reported at their exact indices and later alterations were not reported;
reversed comparison swapped sides; a length mismatch reported `common_steps` and both
counts; checkpoint mismatch by start tick and by state hash was reported separately with
no divergence; a divergence in the common prefix outranked a length difference; two
movies with an import-incompatible checkpoint compared identical while `verify` failed at
import; the comparison entry point takes no manager and calls no backend step. Evidence:
`cli/A15-compare.txt`, `A16-structural-audit.txt`, `probe-transcript-debug.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A16 — Non-mutation, v1/M2 regression and byte-identical demos

**Passes only when** the increment is proven additive:

1. **v1 unchanged.** `MOVIE_FORMAT_VERSION` and `MOVIE_INPUT_SCHEMA_VERSION` are still
   `1`; the container has no new field; a movie file recorded before the increment
   decodes, verifies and re-encodes byte-identically; a movie recorded after the
   increment with the same inputs and metadata is byte-identical to the baseline movie;
   the existing rejection matrix (magic, versions, checksum, truncation, trailing bytes,
   limits, metadata, tick/count, embedded checkpoint) still produces its typed
   categories; decode/verify behavior is unchanged.
2. **Default CLI output unchanged.** `tas play` and `tas verify` with no range flags
   produce stdout byte-identical to the increment baseline (equal SHA-256), exit 0 on
   the same fixtures, and keep the existing divergence/tick-mismatch behavior.
3. **Non-mutation.** Ranged replay and comparison never modify movie bytes or movie
   metadata; a pre-existing session's tick, state hash and mutable state are unchanged; a
   valid replay registers exactly one fresh imported session; an invalid range registers
   no session and executes no step; a replay that fails after checkpoint import may leave
   that single fresh imported session registered, exactly as v1 `Movie::verify` did,
   while never touching pre-existing sessions or files; no operation writes storage or a
   file.
4. **No coverage loss and no weakening.** Every pre-existing test target in the workspace
   is at least at its increment-baseline count; the M2 acceptance targets and the M3
   movie targets keep their assertions; no test, assertion, lint or allow is removed or
   relaxed; `git diff -U0` adds no `#[allow]`, `#[ignore]`, `unsafe` or `todo!`.
5. **No dependency or structural drift.** `Cargo.toml`/`Cargo.lock` are unchanged;
   `cargo tree -d` is unchanged; there is exactly one v1 reader/writer in
   `chronofork_sessions::movie`, one embedded-checkpoint decode path, one backend
   compatibility predicate and one replay loop; no new crate, module-level duplicate
   parser or dead public item appears.
6. **Full gate and demos.** The full gate (`./scripts/check.sh` or its individual fmt,
   check, clippy `-D warnings`, test and rustdoc commands) exits 0; both demos run three
   times with empty stderr, exit 0 and stdout byte-identical to the increment baseline;
   the serialized-state probe matches the baseline bytes, tick and state hash on two
   independent roots; no output contains a timestamp, host path or network content.
7. **Debug/release equality.** The same fixture movies produce equal replay reports,
   seek results and comparison verdicts in debug and release artifacts (compared across
   recorded artifacts, not inside one process).

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| v1 movie and rejection targets | `cargo test -p chronofork-sessions --lib` and `cargo test -p chronofork-sessions --test movie` | `targets/*`, `A16-regression.txt` |
| Ranged replay/compare regression | `cargo test -p chronofork-sessions --test replay` | `targets/sessions-replay.stdout.txt` |
| Release equality | `cargo test -p chronofork-sessions --release --test replay` and `--release --test movie` | `targets/*-release.stdout.txt`, `A16-regression.txt` |
| M2 target matrix | the M2 matrix from the [M3 plan §4](m3-tas-replay-acceptance.md#4-executed-test-targets-m3-t4) plus the CLI session-storage target | `target-matrix.txt`, `targets/*` |
| Full gate | the gate commands in §4 | `logs/` (per-command `command`/`exit`/`meta`/`stdout`/`stderr` artifacts) |
| Demos | both demos, three runs each, `cmp` and SHA-256 against the increment baseline | `demos/`, `demo-runs.txt` |
| Serialized-state probe | baseline-stage `session save/load` probe on two independent roots | `probes/`, `A16-regression.txt` |
| Structural/dependency audit | `rg` scans for one reader/writer and one predicate; `git diff -- Cargo.toml Cargo.lock`; `cargo tree -d` | `A16-structural-audit.txt`, `logs/tree-duplicates.stdout.txt`, `logs/cargo-manifest-diff.stdout.txt` |

**Observed result:** PASS. `MOVIE_FORMAT_VERSION` and `MOVIE_INPUT_SCHEMA_VERSION`
remained `1` with no new container field; the increment-1 movie fixture decoded, verified
and re-encoded byte-identically, and a fresh recording was byte-identical to it; default
CLI `play`/`verify` stdout was byte-identical to the baseline archives; ranged replay and
compare never modified movie bytes or source sessions and wrote no storage or files; M2
targets and counts were unchanged; release-mode targets passed and the debug/release probe
transcripts were byte-identical; the full gate and demos were green; `Cargo.toml`/
`Cargo.lock` and `cargo tree -d` were unchanged; no test, lint, assertion or allow was
weakened. Evidence: `A16-regression.txt`, `A16-structural-audit.txt`,
`cli/A16-v1-cli.txt`, `target-matrix.txt`, `demo-runs.txt`, `checkpoint-probe.txt`,
`m3-v1-regression.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A17 — CLI partial replay and comparison process contract

**Passes only when** the CLI implements the planned surface in §6 with the exact flags,
defaults, `key=value` blocks, stderr diagnostics and exit codes:

1. **Flags.** `play` and `verify` accept optional `--from <TICK>` and `--until <TICK>`;
   `record` and `compare` reject them as unknown options; `--steps`/`--input` remain
   record-only; `--against` is compare-only and is rejected by `record`, `play` and
   `verify`; `compare` requires both `--movie <FILE>` and `--against <FILE>` and rejects
   a third, duplicate or missing file argument; missing flag values, non-numeric values
   and duplicate flags are typed CLI errors (exit 1, no panic).
2. **Default output unchanged.** With no range flags, `play`/`verify` print exactly the
   M3-A7 block shape (no new fields); `record` output is unchanged.
3. **Ranged output.** A valid ranged `play` prints `record=playback` with
   `start_tick`, `end_tick`, `steps`, `final_tick`, `final_state_hash`, `range_from`,
   `range_until`, `prefix_steps`, `window_steps` and `replayed_steps`; a valid ranged
   `verify` prints `record=verification` with `verified=1` and the same fields; both
   exit 0.
4. **Compare output.** `tas compare` prints `record=comparison` with `identical=1` and
   exits 0 for identical movies; for a divergence it prints `identical=0`, `diverged=1`,
   `divergence_index`, `tick_a`, `tick_b`, `state_hash_a`, `state_hash_b`, `steps_a` and
   `steps_b` and exits 1; for a checkpoint mismatch it prints `identical=0`,
   `checkpoint_mismatch=1`, `start_tick_a`, `start_tick_b`, `checkpoint_state_hash_a`,
   `checkpoint_state_hash_b`, `steps_a`, `steps_b` and exits 1; for a step-count
   mismatch it prints `identical=0`, `length_mismatch=1`, `common_steps`, `steps_a`,
   `steps_b` and exits 1.
5. **Rejections.** An invalid range exits 1 with a typed stderr message
   (`chronofork: <error>`) and empty stdout; a state-hash divergence during a ranged
   replay uses the existing divergence block and exit 1; a recorded-tick mismatch remains
   a stderr-only typed failure with no divergence block; a missing file, directory,
   unreadable file, oversized file, truncated file, byte-flipped file, wrong magic,
   unsupported format or input-schema version or corrupt embedded checkpoint exits 1 for `play`,
   `verify` and `compare` with typed stderr and no partial stdout block.
6. **Two-process boundary.** A record in process P1, a ranged `play`/`verify` in P2/P3
   using only the movie file, a rewind in a new P3 invocation and a `compare` in P4 all
   reproduce the expected reports with no shared handles, no storage root and no carried
   state; the movie file SHA-256 is unchanged by every read-only command.
7. **Help and honesty.** `--help` documents the new flags (including `--movie` and
   `--against` for `compare`) and the `compare` subcommand, states that seek is
   re-simulation from the embedded checkpoint (not O(1) random access) and that in-process
   rewind, checkpoint anchors/format v2, rerecord, branch
   editing, multi-controller/ports/analog/keyboard input, real-hardware input TAS and
   import/export remain unimplemented; help does not claim M3 completion.
8. **Streams and hygiene.** Valid stdout carries only `key=value` data; no timestamp,
   host name, path, network access or telemetry appears; exit code is 0 on success and 1
   on any typed failure; directives go to stderr only; no diagnostic contaminates stdout.
9. **Existing CLI.** Every pre-existing CLI command and test is unchanged and green;
   `cargo test -p chronofork-cli` and `cargo test -p chronofork-cli --test tas` pass.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| CLI TAS process tests | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `cli/A17-cli-transcript.txt`, `cli/` |
| Existing CLI regression | `cargo test -p chronofork-cli` | `targets/cli-all.stdout.txt` |
| Manual separate-process transcript | record P1; ranged play/verify P2/P3; rewind P3; `compare --movie <FILE> --against <FILE>` P4; parse blocks and exit codes; SHA-256 before/after | `cli/A17-cli-transcript.txt`, `cli/` |
| Help and deferred text | `cargo run -p chronofork-cli -- tas --help` and the subcommand help; compare against ADR-0005 §8 | `cli/A17-help.txt.stdout.txt` |

**Observed result:** PASS. `play`/`verify --movie [--from] [--until]` and
`compare --movie/--against` implemented the specified flags, defaults, blocks and exit
codes; `record`/`compare` rejected range flags and `--against` was compare-only;
duplicate/missing/unknown/non-numeric flags were typed exit-1 failures with empty stdout;
ranged blocks carried `range_from`/`range_until`/`prefix_steps`/`window_steps`/
`replayed_steps` and `verify` added `verified=1`; compare emitted one stable block with
`identical=1` (exit 0) or `checkpoint_mismatch=1`/`diverged=1`/`length_mismatch=1`
(exit 1); state-hash divergence used the existing block; recorded-tick mismatch stayed
stderr-only; corrupt/missing/directory/oversized/wrong-magic/bad-version/bad-schema/
truncated/trailing/corrupt-checkpoint movies exited 1 with typed stderr and no stdout
block; help documented the new surface and the deferred list; the movie file SHA-256 was
unchanged by read-only commands. **O-1 (t5 observation, recorded).** A structurally valid
movie whose embedded checkpoint is import-incompatible (foreign backend identity, version,
payload format or configuration hash) is reported with the typed
`SnapshotImport(SessionError::IncompatibleSnapshot)` wrapper and the observed process
message `cannot import the embedded checkpoint: incompatible snapshot: …`; a malformed,
truncated or checksum-bad embedded checkpoint fails earlier with the
`InvalidEmbeddedSnapshot` wrapper. The earlier shorthand "invalid embedded checkpoint"
names the embedded-checkpoint wrapper category, not the incompatible-import message;
category and exit behavior are correct and no contract change is implied. O-1 evidence:
`cli/a14-foreign-valid.stderr.txt`, probe check `a14_reject_before_import` in
`probe-transcript-debug.txt`. Row evidence: `cli/A17-cli-transcript.txt`,
`cli/A17-help.txt.stdout.txt`, `targets/cli-tas.stdout.txt`,
`targets/cli-all.stdout.txt`, `acceptance-checks.txt`.

**Status:** executed - passed (t5, 2026-11-15).

## 6. Owner-approved minimal CLI surface (implemented and verified)

The surface below is the approved contract shape from
[ADR-0005 §7](../adr/0005-partial-replay-and-branch-comparison.md#7-cli-contract-shape-planned).
It is implemented in `chronofork-cli` (`tas.rs`) and was exercised end to end by the t5
verification. It is deliberately small, std-only and line-oriented. Command names and
flags must not grow beyond this in the increment; adding a flag would need a contract
change. The blocks below are observed shapes; exact values remain in `cli/`.

```text
chronofork tas play    --movie <FILE> [--from <TICK>] [--until <TICK>]
chronofork tas verify  --movie <FILE> [--from <TICK>] [--until <TICK>]
chronofork tas compare --movie <FILE> --against <FILE>
```

Behavior contract:

- **`--movie`** remains required for `play`/`verify`; `compare` requires `--movie` for
  the left movie and `--against` for the right movie. A missing file, directory,
  unreadable file or file larger than `MAX_MOVIE_BYTES` is a typed failure (exit 1),
  never a panic.
- **`--from`/`--until`** are optional replay-only boundaries. Omitted values default to
  the movie's start tick and end tick. A supplied value must be the start tick, the end
  tick or an existing recorded outcome tick; anything else is a typed failure. A range
  with `from == until` on a movie with steps is rejected (empty window); the zero-step
  movie's default full range is valid.
- **`record`** behavior is unchanged from the M3-A7 contract; it never accepts
  `--from`/`--until` and never writes a partially validated movie.
- **`compare`** loads both movies named by `--movie` and `--against` (each with the
  existing size bound), compares recorded expectations only and never replays. It
  accepts the same file twice (identical).
- **Output** is stdout-only `key=value` blocks separated by one blank line, in the
  existing style. The first field is `record=playback`, `record=verification` or
  `record=comparison`.

Default `play` block (unchanged from M3-A7; no range fields):

```text
record=playback
start_tick=0
end_tick=64
steps=64
final_tick=64
final_state_hash=<lowercase hex>
```

Default `verify` success block (unchanged from M3-A7):

```text
record=verification
verified=1
start_tick=0
end_tick=64
steps=64
final_tick=64
final_state_hash=<lowercase hex>
```

Ranged `play`/`verify` block (observed shape; the following example uses a full movie of
64 steps and the window `(16, 32]`):

```text
record=playback
start_tick=0
end_tick=64
steps=64
final_tick=32
final_state_hash=<lowercase hex>
range_from=16
range_until=32
prefix_steps=16
window_steps=16
replayed_steps=32
```

`record=verification` adds `verified=1` in the same place as today. `start_tick`,
`end_tick` and `steps` remain movie-level facts; `final_tick` is the `range_until`
boundary and `final_state_hash` is the observed hash there. A divergence during the
window prints the existing divergence block (`diverged=1`, `tick`, `input_index`,
`expected`, `observed`) and exits 1; a divergence during the prefix is reported at its
own index, never as a window pass.

Compare blocks (observed shapes):

```text
record=comparison
identical=1
start_tick=0
steps=64
final_state_hash=<lowercase hex>
```

```text
record=comparison
identical=0
diverged=1
divergence_index=3
tick_a=4
tick_b=4
state_hash_a=<lowercase hex>
state_hash_b=<lowercase hex>
steps_a=64
steps_b=64
```

```text
record=comparison
identical=0
checkpoint_mismatch=1
start_tick_a=0
start_tick_b=5
checkpoint_state_hash_a=<lowercase hex>
checkpoint_state_hash_b=<lowercase hex>
steps_a=64
steps_b=63
```

```text
record=comparison
identical=0
length_mismatch=1
common_steps=32
steps_a=64
steps_b=33
```

`state_hash_a`/`state_hash_b` and `checkpoint_state_hash_*` are lowercase hexadecimal.
For the `identical=1` block the two movies' values are equal, so single
`start_tick`/`steps`/`final_state_hash` fields are printed instead of A/B pairs.
Metadata (`movie_id`, `name`, `parent_movie_id`) is never printed as an equality signal
and never printed at all in a comparison block; file paths are not printed in valid
blocks. A metadata-only difference therefore produces the `identical=1` block.

Exit and stream contract: `0` on success and `1` on any typed failure or mismatch;
stdout carries only the data format; stderr carries `chronofork: <error>` diagnostics; no
network access and no telemetry. No timestamp or host path appears in valid bytes or
stdout.

**Executed CLI tests (t5):**

| Test target | Command | Evidence |
|---|---|---|
| CLI `tas` process target | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `cli/A17-cli-transcript.txt`, `cli/` |
| Existing CLI regression | `cargo test -p chronofork-cli` | `targets/cli-all.stdout.txt` |

**Observed result:** PASS. The t5 record includes the flag/default/block/exit matrix, the
compare verdict matrix, invalid-range and corruption rejections with typed stderr and
empty stdout, the help/deferred text, and default-output equality with the increment
baseline. Evidence: `cli/A17-cli-transcript.txt`, `cli/A17-help.txt.stdout.txt`,
`targets/cli-tas.stdout.txt`, `targets/cli-all.stdout.txt`, `acceptance-checks.txt`.

## 7. Baseline preconditions

These capabilities existed before this increment and remain preconditions. They are not
evidence for M3-A13–A17; the executed rows in §5 are.

| Existing evidence | What it shows | What it does not cover here |
|---|---|---|
| `crates/chronofork-sessions/src/movie.rs` (v1 reader/writer, `record`, `verify`) | The v1 container, recorder and full-range verifier exist and are verified (M3-A1–A12, M3-T4 PASS) | Any tick range, window/prefix reporting, seek, rewind, frame advance or comparison |
| `crates/chronofork-cli/src/main.rs` (`tas record/play/verify`) | Stable blocks, exit codes and process behavior for v1 (M3-A7) | `--from`/`--until`, `tas compare`, help text for the new surface |
| `SessionManager::create_session_from_snapshot_bytes` | A canonical checkpoint imports into a fresh session in one process | Ranged reuse, repeated imports with no carried state, invalid-range rejection before import |
| `SessionManager::compare` (`FR-FORK-006`) | Compares two sessions by tick and state digest | Movie recorded-expectation comparison, first divergent index, length/checkpoint reports |
| M2 storage and target matrix | Durable checkpoints, metadata, restart, bounds, rejection | Any movie range or comparison behavior |
| `crates/chronofork-test-backend` | Deterministic one-input-per-step execution and `APPLY_INPUT` | A TAS range or comparison surface |
| `crates/chronofork-megadrive/src/backend.rs` | The minimal Mega Drive slice executes deterministically | Input is ignored (`_input`), so it is not an input-TAS backend and cannot back this increment |
| [Research: TAS and debugging](../research/tas-and-debugging.md) | Upstream evidence that seek is replay from a cached state, not random access | A ChronoFork implementation; research is context only |

As of the t5 record, the previously absent pieces exist:
`crates/chronofork-sessions/src/replay.rs`, `crates/chronofork-sessions/tests/replay.rs`,
`crates/chronofork-cli/src/tas.rs`, the CLI range flags and `tas compare`. They are
increment-2 evidence and are recorded in §4, §5 and §9, not in this baseline table. The
increment baseline for byte-identity and coverage comparisons is
`.research/m3-tas-inc2/baseline/` (t1).

## 8. Traceability

| Criterion | Requirements | ADR-0005 sections |
|---|---|---|
| M3-A13 partial replay, seek, forward advance, rewind | FR-TAS-001, FR-EXEC-004, FR-EXEC-005, FR-EXEC-007 (partial coverage only; frame pacing/run loop remain Planned), NFR-DET-003, NFR-DET-004 | §2, §3, §10 |
| M3-A14 typed range rejection | FR-TAS-001, NFR-QUAL-004, NFR-STOR-002, NFR-ISO-001, AS-12 | §3, §6, §10 |
| M3-A15 branch comparison | FR-TAS-001, FR-TAS-002, FR-EXEC-005, FR-FORK-006 (session compare is related, not replaced), NFR-DET-003 | §4, §10 |
| M3-A16 non-mutation, v1/M2 regression, gate/demos | NFR-DET-003, NFR-DET-004, NFR-ISO-001, NFR-ISO-002, NFR-QUAL-001, NFR-QUAL-005, NFR-DUR-005, AS-01, AS-02, AS-11, AS-15, plus M3-A1–A12 and M2-A1–A11 as regression preconditions | §5, §6, §9 |
| M3-A17 CLI range/compare contract | FR-API-001, FR-TAS-001, NFR-QUAL-004, NFR-SEC-004 | §7, §10 |

At the t5 execution record, the requirement statuses in
[functional.md](../requirements/functional.md) are: `FR-TAS-001` **Partial** (v1
record/playback and verification, plus ranged replay/seek, forward single-tick advance and
rewind by deterministic linear re-simulation and branch comparison of recorded records are
implemented and verified; rerecord/branch editing, checkpoint anchors/format v2,
in-process rewind/state cache, multi-controller/ports/analog/keyboard and real-hardware
input remain deferred), `FR-TAS-002` **Implemented** (v1 format, unchanged),
`FR-EXEC-005` **Implemented** (exact first divergence; comparison additionally reports the
first differing recorded index/tick without re-simulation), `FR-EXEC-007` **Planned**
(frame pacing, run loop and in-process rewind), `FR-FORK-006` **Implemented** (session
compare only; movie comparison is related but separate). M3 remains incomplete as
required by §3.

## 9. Execution record (2026-11-15)

**Executed by the independent t5 verification: PASS.** Tree under test: HEAD
`c6c502de54932459a1d3bc0cd0ada10ceba97043` (branch `main`, equal to `origin/main`) plus
the uncommitted increment-2 paths (`crates/chronofork-sessions/src/{replay,lib,movie}.rs`,
`crates/chronofork-sessions/tests/replay.rs`,
`crates/chronofork-cli/src/{main,tas}.rs`, `crates/chronofork-cli/tests/tas.rs`); worktree
diff sha256 `70ccf322…dfd7` at start and unchanged by the implementation side; the
pre-existing owner-authorized top-level `README.md` change is excluded from scope. No
commit or push. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

1. **Full gate:** `cargo fmt --all -- --check`, `cargo check --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`,
   `./scripts/check.sh` and `git diff --check` all exited 0; the gate logs are in
   `logs/`.
2. **Test targets:** every target in §4 exited 0. The per-target counts and increment
   baseline comparison are in `target-matrix.txt`; no pre-existing target was below its
   baseline and the M2 targets kept their counts.
3. **Demos and probe:** both demos ran three times with 0-byte program stderr and
   byte-identical stdout to the increment baseline; the independent-root checkpoint
   probe and the increment-1 v1 movie regression matched the baseline bytes and hashes
   (`demos/`, `probes/`, `checkpoint-probe.txt`, `m3-v1-regression.txt`).
4. **Acceptance rows:** M3-A13–A17 all executed and passed; the per-row commands and
   artifacts are in §5 and `A13-A17-execution-record.txt`. The verifier's CLI harness and
   independent public-API probe passed and are evidence tooling under the ignored path,
   not product code. t5 findings: none blocking. Observation O-1 is recorded in the
   M3-A17 row; O-3 is behavior-equal (default CLI bytes identical) and requires no code
   or documentation change.
5. **Evidence:** `.research/m3-tas-inc2/verification/` (`SUMMARY.txt`,
   `target-matrix.txt`, `A13-*.txt`–`A17-*.txt`, `A13-A17-execution-record.txt`,
   `acceptance-checks.txt`, `logs/`, `targets/`, `cli/`, `demos/`, `probes/`,
   `MANIFEST.sha256`); the dated gate-result block is in
   [quality gates](quality-gates.md). The verifier's verdict is PASS.

This is an uncommitted working tree; the evidence binds that tree, not a commit. Any
later edit to an implementation path invalidates the equality claims until re-verified.

## 10. Out of scope and stated limitations

This increment delivered forward partial replay/seek by re-simulation from the embedded
checkpoint, single-tick forward advance and rewind as new invocations, and comparison of
recorded expectations. The following remain **not implemented and must not be claimed**:

- **Synthetic backend only.** The acceptance claim covers the deterministic synthetic
  backend. It is a test fixture, not a consumer emulator, and no Mega Drive or other
  hardware accuracy is claimed.
- **No real-hardware input TAS.** The Mega Drive slice's `step` ignores input, so its
  recorded inputs have no effect. Changing that is an emulation-semantics change and is
  deferred (ADR-0005 option C).
- **No multi-controller, ports, analog or keyboard.** Input schema v1 is one
  backend-defined `u32` mask per step; other schemas are a future version (option B).
- **No checkpoint anchors or format v2.** The embedded checkpoint is the only anchor.
  Periodic anchors, anchor-based fast seek, anchor validation and any format-version
  bump are deferred.
- **No O(1) or random-access seek.** Seek is deterministic re-simulation from the
  embedded checkpoint; cost is proportional to the `until` boundary and repeated seeks
  repeat the work.
- **No in-process rewind buffer, state cache or interactive stepping.** Rewind is a new
  invocation; frame advance is one backend step expressed by a new invocation. There is
  no timeline UI, pause/resume loop or ring buffer.
- **No rerecord or branch editing.** No tool edits a movie in place, no rerecord counter
  exists, and `parent_movie_id` remains passive metadata.
- **No import/export adapters.** BizHawk BK2 and other TAS formats are not implemented
  and would need a separate decision (option D).
- **No input-mask comparison.** Compare does not treat inputs as equality fields.
- **Compare is not verification.** A compare PASS is not a determinism or compatibility
  proof; `verify` remains the re-simulation path.
- **No HTTP, MCP, GUI or daemon.** All operations are local, synchronous and std-only;
  Stage 6 interfaces are deferred.
- **No compression, encryption, authentication or deduplication.** Movie files can be
  large and contain emulated memory; protecting them is the operator's responsibility.
- **No cross-backend or cross-platform claim.** A movie from another backend version or
  configuration is rejected at replay import; runtime behavior on Windows and macOS is
  not claimed.
- **No performance or capacity target.** The replay cost and the v1 byte limits are
  format/algorithm costs, not benchmarks.
- **No commits or pushes.** Evidence remains under ignored `.research/` paths until the
  owner explicitly authorizes a commit.
- **M3 is not complete.** This is the second increment of Stage 3, not the whole stage;
  periodic anchors/checkpoints and seek via checkpoints (format v2), an in-process rewind
  buffer/state cache and interactive stepping remain unchecked.

## 11. Change control

- A change to a criterion, expected typed category, command, target name, output field or
  artifact path requires an explicit ADR/plan amendment; it must not be made by quietly
  renaming a test or loosening an assertion.
- The names in this document describe the planned contract shape. The owner-specified
  `compare` flags (`--movie`/`--against`) are fixed; other spelling may be refined by the
  approved Fase B plan. The semantics, range boundaries, equality rules, exit codes and
  rejection boundaries must remain equivalent.
- If a planned command cannot run because its target is absent, the row remains
  `planned - not yet executed`; the gap is reported, not hidden.
- Once a row has executed, its result is append-only evidence linked from
  [quality gates](quality-gates.md); do not rewrite history or relabel a failure.
- A failed row blocks this increment. Never weaken, delete or skip a test to obtain a
  green gate, and never widen a claim beyond the verified synthetic-backend scope.
- This plan does not authorize code, a format change, a dependency, a commit or a push.
- **2026-11-15 (docs correction, t9).** The `compare` CLI shape was corrected to the
  owner-specified `--movie <FILE> --against <FILE>`, and M3-A16 item 3's failed-replay
  session-registration wording was aligned with verified v1 behavior (a replay that
  fails after checkpoint import may leave its single fresh imported session registered;
  it never touches pre-existing sessions or files). No other criterion, range/prefix
  semantics, comparison field, precedence, exit code or deferred item changed.
- **2026-11-15 (execution promotion, t6).** The t5 independent verification PASS promoted
  M3-A13–A17 to `executed - passed`, filled §9 with the tree/commands/artifacts and
  recorded t5 observation O-1 in the M3-A17 row. No criterion, command, typed category,
  output field, exit code or deferred item changed; the deferred list and M3's incomplete
  status are unchanged.
