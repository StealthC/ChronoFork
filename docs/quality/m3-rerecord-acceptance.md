# M3 Rerecord and Branch Editing — Executable Acceptance Plan

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** **Executed; every row M3-A35–M3-A44 is `executed - passed`** (independent
  t10 repair-round-2 PASS, 2026-11-16). Round-1 FAIL findings F1/F2/F3/F5 and gaps F6
  were repaired and re-verified. The increment is complete for the deterministic synthetic
  backend; M3 is **not complete**.
- **Date:** design 2026-11-15; execution record 2026-11-16
- **Companion contract:** [ADR-0008: rerecord and branch
  editing](../adr/0008-rerecord-and-branch-editing.md) — **Accepted** (owner-approved
  2026-11-15T21:09:29-03:00, America/Fortaleza; 2026-11-16T00:09:29Z) and **implemented
  and independently verified** for the deterministic synthetic backend (t10
  repair-round-2 PASS, 2026-11-16).
- **Extends:** the accepted v1 contract [ADR-0004](../adr/0004-tas-movie-format.md), the
  accepted increment-2 contract
  [ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md), the accepted v2
  contract [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) and
  the accepted rewind contract
  [ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md). Their executed
  rows (M3-A1–A12, M3-A13–A17, M3-A18–A27, M3-A28–A34) are **regression preconditions**,
  never acceptance for M3-A35–A44.
- **Related:** [execution model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [roadmap](../roadmap/roadmap.md), [M3 first-increment acceptance
  plan](m3-tas-replay-acceptance.md), [M3 increment-2 acceptance
  plan](m3-partial-replay-acceptance.md), [M3 format-v2 acceptance
  plan](m3-format-v2-acceptance.md), [M3 rewind acceptance
  plan](m3-rewind-acceptance.md), [research: TAS and
  debugging](../research/tas-and-debugging.md), [testing strategy](testing-strategy.md),
  [quality gates](quality-gates.md), [anti-slop policy](anti-slop-policy.md).

> **Verification record (2026-11-16).** The independent t10 repair-round-2 verifier
> executed M3-A35–A44 on the repaired uncommitted increment tree at HEAD `3f76d5f`
> (branch `agent/m3-complete`) plus the recorded working-tree paths, with
> `CARGO_HOME=/tmp/t7-cargo`, and returned **PASS**. Every row is `executed - passed`;
> round-1 FAIL findings F1/F2/F3/F5 and gaps F6 are closed; the round-1 FAIL record is
> preserved, not rewritten. The full gate, both demos, target matrix, v1/v2/M1/M2/M3
> regression, independent edit/CLI harnesses and structural/suppression audits are green.
> Raw evidence: `.research/m3-rerecord/verification-round-2/` (`SUMMARY.txt`, `rows/`,
> `target-matrix.txt`, `gate/`, `targets/`, `cli/`, `edit/`, `demos/`, `probes/`, `audit/`,
> `tree/`, `MANIFEST.sha256`; manifest verified); round-1 evidence:
> `.research/m3-rerecord/verification/`. The verifier's only tracked edit was the dated
> round-2 block in [quality gates](quality-gates.md). The PASS is limited to the
> deterministic synthetic backend on Linux; real-hardware input TAS remains unimplemented
> and M3 is **not complete**. No commit, push, tag or merge is authorized by this plan.
> Non-blocking caveats are recorded in §9.

## 1. Status rules

1. **`planned - not yet executed`** means the criterion has a planned test target,
   command and evidence path, but no executed result and no independent verifier record
   exists. This is the status of every row in this plan at design time.
2. A row becomes **`executed - passed`** only when all of the following are recorded: the
   exact command, exit code, tree state (HEAD plus uncommitted paths), test output
   summary, the ignored evidence artifact path and an independent verifier review that
   returns PASS. The authoritative dated gate block belongs in
   [quality gates](quality-gates.md); this document records the row-level execution
   references and the round-2 execution snapshot in §9.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact or unverified claim; a failed row blocks this increment. A known
   failure is never converted into a skip, an `#[ignore]`, a relaxed assertion or a
   narrower claim after the fact.
4. A test may be added before the implementation as a red test only if it fails for the
   expected typed reason. A test that cannot run because its target or module does not
   exist is not acceptance evidence; the row remains `planned - not yet executed`.
5. No edit/rerecord/branch behavior may be described as implemented, working, supported
   or complete on the strength of this plan, ADR-0008, a code review, a demo run or a
   single happy-path run. The executed artifacts are the only evidence; M3-A1–A34 are
   evidence for the prior increments only.
6. A planned test whose command cannot execute because its module or test target does not
   exist remains `planned - not yet executed`; it does not become "passed" because the
   absent target produced no failure.
7. A row marked passed must cite a verifier artifact whose SHA-256 matches the recorded
   manifest. Late or reconstructed evidence does not reopen a failed row.
8. A row may not be satisfied by re-running an existing capability. A green M3-A1–A34 run
   is a regression precondition, never acceptance for A35–A44; `tas record`, `tas play`,
   `tas verify`, `tas compare`, anchored seek and `tas studio` do not implement editing.
9. **ADR-0008 is Accepted** (owner-approved 2026-11-15). Acceptance alone does not execute
   a row; implementation without an accepted decision fails this plan's change control
   (§11).
10. Acceptance is limited to the deterministic synthetic backend on the Linux acceptance
    platform. The Mega Drive slice ignores input and is not an acceptance backend for
    input TAS; no hardware editing, input or accuracy claim is made.
11. The output of an edit is a normal v1 or v2 movie. Its acceptance does not widen any
    claim about the format, backend, hardware or M3 completion.

As of the t10 repair-round-2 execution record every M3-A35–A44 row is `executed - passed`;
none is failed. The round-1 FAIL is retained in the ignored evidence and in the dated
[quality gates](quality-gates.md) block; it is not relabelled.

## 2. Scope, definitions and evidence

- **Source movie:** the decoded input `Movie` (v1 or v2) and the exact file bytes read by
  the CLI. It is immutable for the edit.
- **Edited step:** a zero-based recorded step index whose `InputFrame` is replaced. The
  library can replace any field; the CLI replaces only port 0 and preserves the recorded
  ports 1–3, axes and keys ([ADR-0008 §7](../adr/0008-rerecord-and-branch-editing.md#7-cli-surface-output-block-atomicity-and-process-contract)).
- **Truncation:** `--truncate-after T` retains steps `0..=T` and drops every later step;
  at least one step must be dropped ([ADR-0008 §6](../adr/0008-rerecord-and-branch-editing.md#6-v1v2-compatibility-bounds-and-typed-errors)).
- **Earliest affected step `e`:** the minimum of the edited indices and `T + 1` when
  truncation is requested. Steps `0..e` are the **prefix**; steps `e..m` are the
  **suffix**; `m` is the output step count.
- **Prefix verification:** every prefix step is executed from the embedded checkpoint and
  its observed outcome tick and state hash must equal the source record; on success the
  source step record is copied verbatim into the output.
- **Suffix re-simulation:** every suffix step is executed from the resulting session and
  its observed outcome becomes the output expectation. "Rerecord" in this plan means this
  re-simulation, not a new emulation mode.
- **Branch artifact:** the new canonical movie produced by an edit, with its own
  non-zero movie id, `parent_movie_id` equal to the source movie id, and optional name
  ([ADR-0008 §4](../adr/0008-rerecord-and-branch-editing.md#4-branch-metadata-identity-and-isolation)).
- **Anchor policy:** `keep-prefix` (default), `drop`, or `rebuild` with an interval;
  anchors at or after `e` are dropped or rebuilt, never retained
  ([ADR-0008 §5](../adr/0008-rerecord-and-branch-editing.md#5-anchor-handling)).
- **Canonical rebuilt-anchor identity:** a rebuilt anchor envelope is re-encoded with a
  fixed snapshot id and origin session id through the one `Snapshot::from_bytes`/
  `Snapshot::to_bytes` pair, so its bytes do not embed the caller manager's session
  allocation; its tick, state digest, backend fields, payload and envelope length are
  unchanged. v2 recording keeps its session-assigned identity and its bytes are
  unchanged
  ([ADR-0008 §5.3, §8](../adr/0008-rerecord-and-branch-editing.md#5-anchor-handling)).
- **Acceptance backend:** the deterministic synthetic backend (`chronofork-test-backend`)
  is the only acceptance backend. The Mega Drive slice ignores input and is not an
  acceptance backend.
- **Platform:** Linux is the acceptance platform, matching the M1/M2/M3 evidence
  platforms. Windows and macOS are compile-checked in CI; runtime behavior there is not
  claimed.
- **Pre-change baseline:** the ignored `.research/m3-rerecord/baseline/` path, captured
  2026-11-15T23:49:31Z on branch `agent/m3-complete` at HEAD
  `3f76d5f` (`3f76d5ffee680d2c7271c7c1d81d8c5a44055675`), clean tree, with Git state,
  environment, gate logs, per-target lists, both demos (three runs each), movie/CLI
  fixtures and tool metadata.
- **Verification evidence (executed):** raw output is under the ignored
  `.research/m3-rerecord/verification-round-2/` path: `SUMMARY.txt`, `target-matrix.txt`,
  `rows/A35-*.txt` … `rows/A44-*.txt`, `rows/M3-A1-A34-regression.txt`, `targets/`,
  `cli/`, `edit/`, `audit/`, `demos/`, `probes/`, `gate/`, `tree/` and
  `MANIFEST.sha256`. Each artifact records the command, tree state, exit code and the
  relevant stdout/stderr/SHA-256. The round-1 FAIL evidence is preserved under
  `.research/m3-rerecord/verification/`. The dated gate-result block is in
  [quality gates](quality-gates.md) and is verifier-owned.
- **No host time in data or reports:** valid output contains no timestamp, process id,
  host label, nonce, path or entropy. Comparisons use ticks, state hashes, step indices,
  block fields, movie bytes and SHA-256, never wall-clock time or filesystem modification
  time.

## 3. Increment exit criteria map

| Verified capability | Acceptance criteria |
|---|---|
| Functional editing: prefix copy/verify, edited-frame application, suffix re-simulation, recomputed expectations, output self-verification | M3-A35 |
| Truncation, index/count bounds and typed request errors | M3-A36 |
| Anchor handling: prefix retention, drop, deterministic rebuild, binding and budgets | M3-A37 |
| Branch metadata, identity and isolation | M3-A38 |
| v1/v2 preservation, device-field boundary and reject-only compatibility | M3-A39 |
| `chronofork tas edit` process contract, atomicity and source protection | M3-A40 |
| Determinism, no host-dependent input and bounded resources | M3-A41 |
| Module ownership, one codec/one edit implementation and suppression audit | M3-A42 |
| Non-regression, full gate, demos and release equality | M3-A43 |
| Scope honesty, deferred list and no M3-complete claim | M3-A44 |

Every mapped row is `executed - passed` and the independent t10 repair-round-2 verifier
recorded PASS (2026-11-16). The increment is complete for the deterministic synthetic
backend; it does not complete M3.

## 4. Executed test targets and commands (round 2)

These are the executed lowest-useful-layer targets from the t10 repair-round-2 record.
Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`; commands, streams, exit codes and
timings are in `targets/`, `gate/`, `cli/` and `edit/`.

| Target | Command | Criteria |
|---|---|---|
| Edit unit tests in `crates/chronofork-sessions/src/edit.rs` | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib` | M3-A35, A36, A37, A38, A39, A41, A42 |
| Edit integration target `crates/chronofork-sessions/tests/edit.rs` | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test edit -- --nocapture` | M3-A35–A39, A41 |
| Release-mode edit integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --release --test edit` | M3-A41, A43 |
| CLI process target `crates/chronofork-cli/tests/tas_edit.rs` | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas_edit -- --nocapture` | M3-A36, A37, A38, A39, A40, A41 |
| Release-mode CLI edit | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --release --test tas_edit` | M3-A40, A41, A43 |
| Existing movie/replay/player/studio/TAS targets | `cargo test -p chronofork-sessions`, `cargo test -p chronofork-cli` | M3-A43 |
| Full gate | `./scripts/check.sh`; `cargo fmt --all -- --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`; `git diff --check` | M3-A43 |
| Demos | `cargo run -p chronofork-cli -- demo` and `-- megadrive demo`, three cargo runs and three direct-binary runs each, compared with the baseline by `cmp` and SHA-256 | M3-A43 |
| Differential reference harness | library-level full re-record over the same embedded checkpoint and edited input sequence, compared record-by-record with the edit output | M3-A35 |
| Process/atomicity transcripts | scripted `tas edit` runs, destination/source alias variants, failure injection, help capture | M3-A36, A37, A39, A40 |
| Structural and scope audits | `rg` scans, `git diff -U0` suppression scan, `cargo tree -d`, manifest diff, docs/help read | M3-A42, A44 |
| Executable size-budget fixtures (repair round t9) | inside `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test edit` | M3-A36.6, A37.5, A41 |

The size-budget fixtures use a test-only deterministic backend whose snapshot payload
length is `base + growth * steps`. They drive the v1 writer output-size rejection (a
source at exactly `MAX_MOVIE_BYTES` whose branch adds a one-byte name), the per-anchor
`MAX_ANCHOR_SNAPSHOT_BYTES` capture rejection and the cumulative `MAX_ANCHOR_BYTES`
capture rejection, each asserting the typed error and that no movie is returned. The
backend is a test fixture, not a product backend, codec, parser or compatibility
predicate. The output-size fixture necessarily holds one source at `MAX_MOVIE_BYTES` in
the test process (roughly 1 GiB peak with the codec's current copies) and takes several
seconds in an unoptimized build; that is a test-only cost and not a product resource or
performance claim.

## 5. Acceptance criteria

Every row below is `executed - passed` (t10 repair-round-2 verification, 2026-11-16). The
pass conditions are retained as the verified contract; each row records the executed
commands, observed result and evidence paths. The round-1 FAIL record is preserved in
`.research/m3-rerecord/verification/` and is not relabelled.

### M3-A35 — Editing model and suffix re-simulation exactness

**Passes only when** all of the following are demonstrated on the deterministic synthetic
backend:

1. **New artifact, source unchanged.** A successful library edit returns a new `Movie`;
   the source `Movie` value is `Eq`-unchanged and its bytes are unchanged. The output is
   a canonical v1/v2 movie for the source's format that decodes and re-encodes
   byte-identically.
2. **Prefix copy and verify.** For one edit at index `i`, output steps `0..i` are
   tuple-equal to the source steps (`outcome_tick`, `input`, `state_hash`) and the edit
   executed and compared every prefix step against the source record. A source whose
   recorded prefix expectation is deliberately wrong (re-signed movie) makes the edit
   fail with the existing typed `TickMismatch`/`Divergence` at the first wrong index, and
   produces no output; a later prefix error is never reported instead of an earlier one.
3. **Edited frame applied exactly.** The output step at a library-edited index has the
   requested full `InputFrame` (all fields), and its expectation is the observed outcome
   from that execution, not the source hash. When the CLI sets port 0, the output step's
   ports 1–3, axes and keys equal the source step's recorded fields.
4. **Suffix recomputed and exact.** Every output step after `e` has an expectation equal
   to a direct execution of the output input sequence from the embedded checkpoint. A
   differential reference build (full re-record over the same checkpoint and full input
   sequence) produces the same `(outcome tick, input, state hash)` vector as the edit
   output. No suffix expectation is copied from the source.
5. **Multi-edit and earliest-affected rule.** With edits at several indices,
   `e = min(edited indices)`, all edited frames appear at their indices, steps before `e`
   are prefix-copied, and every step from `e` onward is re-simulated in ascending order
   exactly once. Edits separated by unedited steps behave the same way.
6. **Output self-verifies.** `Movie::verify` (and `tas verify`) on the output passes for
   every output step, and `tas play` reproduces the output end tick and final state hash.
   `tas compare(source, output)` reports no divergence in the unchanged prefix and the
   expected first divergence/length mismatch otherwise (metadata never decides equality).
7. **Halt and failure semantics.** A backend halt before the last output step is a typed
   edit failure and produces no movie; a halt exactly on the last output step is
   accepted, matching the recording rule. Any execution failure preserves the session
   error and the zero-based output index.
8. **No second full verification pass inside the command.** The edit performs prefix
   verification once and suffix execution once; the acceptance evidence shows the
   execution counts implied by `prefix_steps + resimulated_steps` and does not claim a
   hidden extra replay (the existing `tas verify` is the separate verification path).

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Edit unit/integration tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib`, `--test edit -- --nocapture` | `targets/sessions-lib.stdout.txt`, `targets/sessions-edit.stdout.txt`, `edit/exactness/` |
| Differential reference | library test that records the full edited input sequence over the same checkpoint and compares every record | `edit/exactness/harness-debug.txt` |
| Re-signed prefix divergence | library and CLI controls with deliberately wrong prefix records | `edit/divergence/prefix-*` |
| Output self-verification | `tas verify`/`tas play`/`tas compare` on edit outputs | `cli/edit/results/out-verify`, `out-play`, `compare-src-out` |

**Executed evidence (round 2):** `rows/A35-edit-exactness.txt`, `edit/exactness/`,
`edit/divergence/`, `targets/sessions-edit.stdout.txt`.

**Observed result (round 2):** PASS. The source value and bytes were unchanged; prefix
copy/verification, exact edited-frame application, differential full re-record equality,
multi-edit earliest-affected ordering, first/last-step edits, halt semantics, one backend
step per output step and output verify/play/compare all passed. A re-signed tampered prefix
failed at the first wrong index with the typed `TickMismatch`/`Divergence`, empty stdout
and no output. Evidence: `rows/A35-edit-exactness.txt`,
`edit/exactness/harness-debug.txt`, `edit/divergence/`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A36 — Truncation, bounds and typed request errors

**Passes only when** all of the following are demonstrated:

1. **Truncation semantics.** `--truncate-after T` produces exactly `T + 1` output steps,
   the output `end_tick` equals the source step `T` outcome tick, and the source steps
   `T + 1..n` do not appear in the output. A source with deliberately wrong records in
   the dropped suffix still edits successfully when the retained prefix is valid, proving
   the dropped suffix is not executed.
2. **Pure truncation.** Truncation without edits copies and verifies exactly the retained
   prefix, reports `prefix_steps = T + 1` and `resimulated_steps = 0`, and produces no
   suffix expectation. No dropped step is executed.
3. **Truncation validation.** `T >= n`, `T + 1 == n` (no-op), truncation on a zero-step
   movie, and truncation with an edit at an index `>= T + 1` are each typed rejections
   that create no session, execute no step and write no output.
4. **Index validation.** An edit index `>= n`, a duplicate `--set-input` index, a
   negative/non-decimal/malformed index, and a request with neither edit nor truncation
   are typed rejections before import/execution. The CLI reports them on stderr with
   empty stdout and exit 1.
5. **Numeric bounds.** Maximum `u64` values, values above `usize` on a 32-bit host, an
   out-of-range mask (`> u32::MAX`), an empty/invalid name, a zero movie id and an id
   equal to the source id are rejected without panic, wrap, truncation or
   allocate-before-validate. Argument-count and duplicate-flag handling is bounded and
   deterministic.
6. **Output size.** A request whose output would exceed `MAX_MOVIE_BYTES`, the source's
   format step limit or the anchor budgets fails with the inherited typed error and
   writes no output; no silent truncation or clamp occurs. An executable fixture in the
   `--test edit` target drives the writer rejection with a v1 source at exactly
   `MAX_MOVIE_BYTES` and a one-byte branch name, asserting the typed
   `MovieError::FileTooLarge` and no returned movie.
7. **Typed categories.** Each rejection maps to its documented typed category (request,
   identity, anchor, movie/format or session) with source preservation where applicable;
   no stringly error and no panic.

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Request/bounds unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib`, `--test edit` | `targets/sessions-lib.stdout.txt`, `targets/sessions-edit.stdout.txt`, `rows/A36-truncation-bounds.txt` |
| CLI invalid inputs | scripted invalid flag/index/truncation/id/name cases | `cli/edit/results/edit-*`, `cli/edit/summary.txt` |
| Dropped-suffix control | re-signed source with bad dropped suffix; successful prefix-only edit | `edit/divergence/suffix-tamper-*`, `cli/edit/results/edit-v1-truncate` |
| Size/limit controls | oversized output and anchor-budget fixtures | `edit/bounds/` |

**Executed evidence (round 2):** `rows/A36-truncation-bounds.txt`, `edit/divergence/`,
`edit/bounds/`, `cli/edit/results/`.

**Observed result (round 2):** PASS. Truncation retained `T + 1` steps with the expected
end tick and did not execute the dropped suffix; a tampered dropped suffix truncated while
the same tamper in the retained prefix failed at the first bad index. No-op, out-of-range,
zero-step, edit-beyond-truncation, duplicate/malformed indices, bad ids/names/flags and
over-range masks were typed exit-1 failures with empty stdout and no output. The
`--test edit` bulk fixture executed the writer `FileTooLarge` (source at exactly
`MAX_MOVIE_BYTES`, one-byte branch name) rejection and returned no movie. The 32-bit
`usize` case is not representable on the 64-bit acceptance platform and is not claimed.
Evidence: `rows/A36-truncation-bounds.txt`, `edit/bounds/targeted-budget-and-determinism.*`,
`edit/divergence/suffix-tamper-*`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A37 — Anchor handling

**Passes only when** all of the following are demonstrated:

1. **Default keep-prefix.** For a v2 source with anchors, the output retains exactly the
   anchors with `step_index < e`, verbatim (same indices and snapshot bytes), in source
   order; anchors at or after `e` are absent. Every retained anchor still satisfies the
   v2 binding checks against the output step at its index, and the output decodes and
   verifies.
2. **Anchored seek after edit.** A ranged v2 `play`/`verify` on the output selects a
   retained anchor when one is at or before the prefix end, reports the documented
   anchored fields and produces the same boundary tick/state hash as a full replay; the
   dropped anchors do not exist in the output. A keep-prefix output with no retained
   anchor falls back to the embedded checkpoint.
3. **`drop`.** `--anchors drop` produces a v2 output with zero anchors and identical step
   records to keep-prefix for the same edits; an anchor-less v2 source edited with
   `drop` remains anchor-less. The block reports the source anchor count as dropped.
4. **Deterministic `rebuild`.** `--anchors rebuild --anchor-interval N` drops all source
   anchors and builds exactly `m / N` anchors after output steps `N-1, 2N-1, …`; every
   rebuilt anchor's tick and state hash equal the output step's recorded expectation;
   two identical runs produce byte-identical output and anchor bytes. The interval is not
   stored.
5. **Rebuild bounds.** `N = 0`, `N > m`, `m / N > MAX_ANCHORS`, a single anchor above
   `MAX_ANCHOR_SNAPSHOT_BYTES`, cumulative bytes above `MAX_ANCHOR_BYTES`, `rebuild`
   without an interval, an interval without `rebuild`, conflicting anchor flags, and any
   explicit anchor flag on a v1 source are typed rejections with no output. Executable
   fixtures in the `--test edit` target drive the per-anchor and cumulative capture
   rejections through a test-only payload backend and assert the typed
   `MovieError::AnchorSnapshotTooLarge`/`AnchorTableTooLarge` and no returned movie.
6. **Embedded checkpoint unchanged.** The output's embedded checkpoint bytes are
   byte-identical to the source's for every policy, and the output `start_tick`
   equals the checkpoint tick.
7. **One anchor format.** Rebuilt anchors are canonical `Snapshot::to_bytes()` envelopes
   parsed only by `Snapshot::from_bytes` and constructed only through the existing
   `MovieAnchor`/`from_v2_parts` path; no second anchor serializer, parser, format or
   index convention exists. Retained and rebuilt anchors are never mixed in one output.
8. **Reporting.** The block reports `anchor_policy` and the retained/dropped/rebuilt
   counts that match the output table exactly.

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Anchor-policy unit/integration tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib`, `--test edit`, `--test movie_v2 --test replay` | `targets/sessions-lib.stdout.txt`, `targets/sessions-edit.stdout.txt`, `rows/A37-anchor-policy.txt` |
| CLI anchor cases | `tas edit --anchors keep-prefix\|drop\|rebuild --anchor-interval …` | `cli/edit/results/edit-v2-keep`, `edit-v2-drop`, `edit-v2-rebuild-*` |
| Anchored seek on output | `tas play`/`tas verify` `--from/--until` over retained and rebuilt anchors | `cli/edit/results/out-v2-play-anchored` |
| Structural anchor audit | `rg` scan for anchor parsers/serializers and index conventions | `audit/A42-structural-audit.txt`, `rows/A37-anchor-policy.txt` |

**Executed evidence (round 2):** `rows/A37-anchor-policy.txt`,
`cli/edit/results/`, `edit/bounds/`, `audit/A42-structural-audit.txt`.

**Observed result (round 2):** PASS. With source anchors `[3,7,11]` and edit step 8,
keep-prefix retained `[3,7]` with verbatim bytes and report counts 2/1; drop produced an
empty table; rebuild interval 3 over 12 steps produced `[2,5,8,11]`, each bound to its
step, byte-identical across processes and with the interval not stored. Anchored seek on
the output reported the anchor fields and agreed with full replay; the embedded checkpoint
bytes and `start_tick` were unchanged. Per-anchor and cumulative budget fixtures produced
`AnchorSnapshotTooLarge`/`AnchorTableTooLarge` with no movie, and all invalid policy/interval
cases were typed with no output. Rebuilt envelopes used the canonical fixed
snapshot/origin identity. Evidence: `rows/A37-anchor-policy.txt`,
`cli/edit/results/`, `edit/bounds/targeted-budget-and-determinism.*`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A38 — Branch metadata, identity and isolation

**Passes only when** all of the following are demonstrated:

1. **Default id and parent.** With no `--movie-id`, the output id is the source id + 1
   (checked), `parent_movie_id` is the source id, and the output id differs from the
   parent. The block reports both ids.
2. **Explicit id.** An explicit distinct non-zero id is used; an explicit id equal to the
   source, an id of zero and a `u64::MAX` default overflow are typed rejections.
3. **Dangling parent.** After a successful edit, deleting/renaming the source file leaves
   the output decodable, verifiable, comparable and usable in `tas studio`; the command
   never resolves, validates or repairs the parent, and no parent lookup exists.
4. **Name.** With no `--name`, the output name is absent even when the source had a
   name. An explicit valid name round-trips. Empty, over-256-byte, control-character and
   invalid-UTF-8 names are typed rejections. A metadata-only name/id change never alters
   step records or replay results, and `tas compare` ignores metadata.
5. **Source isolation.** The source file's bytes and SHA-256 are unchanged; the source
   path is never written; a pre-existing `SessionManager` with sessions keeps every
   pre-existing session's tick, state hash and snapshot map unchanged across a library
   edit; no storage root is opened or written.
6. **One fresh session.** A successful library edit registers exactly one fresh imported
   session; a rejected request registers none and executes no step. Two edits of the same
   source are independent and use the same source bytes.
7. **Edit chains.** Editing artifact `B` (parent `A`) produces `C` with
   `parent_movie_id = B.movie_id`, not `A`; no cumulative counter or ancestry traversal
   exists; `tas compare` between B and C remains a recorded-expectation diff and metadata
   never decides equality.
8. **Existing commands accept the output.** `tas play`, `tas verify`, ranged
   `play|verify`, `tas compare` and `tas studio` accept the output as an ordinary v1/v2
   movie with their existing output/exit contracts (no edit-specific fields appear in
   their blocks).

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Identity/isolation library tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib`, `--test edit` | `rows/A38-branch-isolation.txt`, `targets/sessions-edit.stdout.txt` |
| CLI metadata cases | `tas edit --movie-id/--name` valid and invalid sets | `cli/edit/results/edit-v1-metadata`, `edit-id-*`, `edit-name-*` |
| Source non-mutation proof | SHA-256, inode/path listings and working-directory listing before/after | `tree/`, `cli/edit/results/publish-*` |
| Existing-command acceptance | `tas verify`, `tas compare`, `tas studio` on edited artifacts | `cli/edit/results/out-*`, `probes/studio/` |

**Executed evidence (round 2):** `rows/A38-branch-isolation.txt`, `cli/edit/results/`,
`probes/studio/`, `tree/`.

**Observed result (round 2):** PASS. Default id was source + 1 with parent equal to the
source and no inherited name; explicit ids/names round-tripped and zero/source-equal ids
and empty/over-long/control names were typed. The invalid-UTF-8 name case is now a typed
exit-1 rejection with `chronofork: command-line arguments must be valid UTF-8`, empty
stdout, no output/temp file and no panic (round-1 F1 closed). The dangling parent survived
source deletion; source bytes/SHA-256 and pre-existing sessions were unchanged; exactly
one fresh session was imported; edit chains used the immediate parent; and
play/verify/range/compare/studio accepted the output. Evidence:
`rows/A38-branch-isolation.txt`, `cli/edit/results/edit-name-invalid-utf8/`,
`audit/help-delta.txt`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A39 — v1/v2 compatibility and device-field boundary

**Passes only when** all of the following are demonstrated:

1. **Format preservation.** Editing a v1 source produces a valid v1 output (`CFMOVIE1`,
   format 1, schema 1, 44-byte records, no anchor table) and editing a v2 source produces
   a valid v2 output (`CFMOVIE2`, format 2, schema 2, 72-byte records, anchor policy as
   requested); versions are byte-checked, not inferred. No v3, migration, upgrade or
   downgrade exists.
2. **No silent truncation.** A library full-frame replacement with non-neutral ports
   1–3, axes or keys on a v1 source is rejected with the existing
   `UnsupportedInputSchema` naming schema 2 and produces no output; it is never reduced
   to port 0. A v2 full-frame edit encodes and replays all fields.
3. **CLI device surface.** `--set-input` changes only port 0; ports 1–3, axes and keys are
   preserved from the source step for both formats. There is no CLI flag to set the other
   fields; `--ports`, `--axes` and `--keys` are rejected as unknown for `edit`.
4. **Anchor/format coupling.** Explicit anchor flags on a v1 source are typed rejections;
   a v1 output never contains anchors; a v2 source with zero anchors stays zero under
   `keep-prefix` and can remain zero under `drop`.
5. **Reject-only compatibility.** An unsupported/future source version is rejected by the
   existing reader before the edit; an incompatible embedded checkpoint is rejected at
   import through `validate_envelope`; a v2 source is not accepted by the v1 constructor
   or vice versa as a fallback.
6. **Golden bytes.** A v1 edit output has the unchanged v1 field order and widths; a v2
   edit output has the unchanged v2 field order, anchor framing and 72-byte records. A
   zero-edit fixture still round-trips through the existing readers. No checksum
   coverage or limit changes.
7. **Backend boundary.** The CLI uses the same deterministic synthetic `BackendFactory`
   as the other TAS commands and has no backend-selection flag; the acceptance claim
   remains synthetic-backend only and no Mega Drive input is simulated.

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Format tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib`, `--test edit`, `--test movie`, `--test movie_v2` | `rows/A39-v1-v2.txt`, `targets/sessions-movie.stdout.txt`, `targets/sessions-movie-v2.stdout.txt` |
| CLI format/device cases | `tas edit --set-input`, foreign device flags, v1/v2 fixtures | `cli/edit/results/edit-v1-*`, `edit-v2-*` |
| Incompatibility controls | foreign-backend/wrong-version embedded checkpoint fixtures | `edit/divergence/corrupt-checkpoint.movie`, `rows/A39-v1-v2.txt` |
| Byte-layout checks | v1/v2 golden fields and sizes; no v3 | `probes/movies/`, `rows/A39-v1-v2.txt` |

**Executed evidence (round 2):** `rows/A39-v1-v2.txt`, `probes/movies/`,
`cli/edit/results/`.

**Observed result (round 2):** PASS. v1 edits produced v1 output (44-byte records, no
anchors) and v2 edits produced v2 output (72-byte records, requested anchor policy), with
no v3. A v1 full-frame device edit was rejected with `UnsupportedInputSchema` and the CLI
port-0 edit preserved ports 1–3, axes and keys; v2 full-frame edits encoded and
differential-replayed. v1 anchor flags were typed; zero-anchor v2 stayed zero under
keep-prefix/drop. Bad magic/version/checksum/checkpoint and a Mega Drive-backend movie were
rejected with typed errors and unchanged source bytes; golden v1/v2 widths and zero-edit
round-trip held. Evidence: `rows/A39-v1-v2.txt`, `probes/movies/`, `cli/edit/results/`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A40 — `tas edit` process contract, atomicity and source protection

**Passes only when** the CLI implements the surface fixed by
[ADR-0008 §7](../adr/0008-rerecord-and-branch-editing.md#7-cli-surface-output-block-atomicity-and-process-contract)
with the exact flags, defaults, block, streams and exit codes:

1. **Flags.** `tas edit` accepts exactly `--movie`, `--out`, repeatable
   `--set-input <INDEX=MASK>`, optional `--truncate-after`, `--movie-id`, `--name`,
   `--anchors <keep-prefix|drop|rebuild>`, `--anchor-interval`, and `--help`. Missing,
   duplicate, empty, non-numeric, out-of-range, conflicting or foreign flags (`--from`,
   `--until`, `--against`, `--steps`, `--root`, `--session`, `--snapshot`, `--ports`,
   `--axes`, `--keys`) and positional arguments exit 1 with a typed stderr diagnostic and
   empty stdout.
2. **Block shape and values.** A successful edit prints exactly one block beginning
   `record=edit`, with the fields in the ADR order, no missing/extra field, and values
   consistent with the decoded output and the request: `movie_version`, `input_schema`,
   ids, name, ticks, `steps`, `edits`, `truncate_after` (empty when absent),
   `prefix_steps`, `resimulated_steps`, `prefix_verified=1`, `anchor_policy` and counts,
   `final_state_hash`, output SHA-256 and source SHA-256. Values match a second decode of
   the output where applicable.
3. **Stdout/stderr/exit.** Valid stdout is `key=value` data only; diagnostics are stderr
   `chronofork: <error>` only; exit 0 on success and 1 on every typed failure; no other
   exit code, no panic, no partial block, no diagnostic on stdout.
4. **Source protection.** `--out` equal to `--movie` by path normalization, and on Linux
   a destination that resolves to the source's device/inode (hard link or symlink
   alias), are rejected with the `OutputAliasesSource` diagnostic before any temp write. The source file's bytes, SHA-256,
   path and link target are unchanged on success and failure; the source is opened
   read-only; `--movie` is never renamed, removed or truncated.
5. **Atomic publish.** The command writes a sibling temporary file and renames it on
   success, replacing only a non-source destination. Failure injection at request
   validation, decode, import, step, anchor capture, construction, write and rename
   leaves no file at `--out` when it did not exist, leaves an existing non-source
   destination unchanged, and leaves no temporary file behind. This is atomic visibility
   against concurrent readers and failed writes; no power-loss durability (fsync) claim
   is made.
6. **I/O failures.** Missing/unreadable source, missing parent directory, directory
   destination, permission failure and rename failure use the typed failure path with
   exit 1 and no partial output; the source and prior destination remain intact.
   Validation/decode/source/write and existing-destination paths were executed; a
   final-publish rename-failure injection was not separately exercised.
7. **Help.** `tas edit --help` and the `tas --help` summary document the flags, index
   conventions, functional suffix re-simulation and cost, source non-mutation, branch
   identity/parent, anchor policy, no stored counter, v1/v2 preservation and device
   limitation, synthetic-backend scope, deferred list and M3 incompleteness. Help capture
   artifacts are recorded.
8. **Declared delta only.** Adding `edit` changes the `tas` dispatch error text and
   `tas --help`; every other pre-existing command's stdout is byte-identical to the
   captured baseline. The delta is recorded here and in M3-A43.
9. **Process independence.** The same source and request in two separate processes
   produce byte-identical output files and identical blocks; the source is unchanged by
   either.

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| CLI process target | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas_edit -- --nocapture` | `targets/cli-tas-edit.stdout.txt`, `rows/A40-cli-contract.txt` |
| Release-mode CLI | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --release --test tas_edit` | `targets/cli-tas-edit-release.stdout.txt` |
| Invalid flags/requests | scripted invalid flag, index, truncation, id, name and anchor sets | `cli/edit/results/edit-*` |
| Atomicity and failure injection | destination/source alias matrix, read-only dirs, failed writes, existing destination | `cli/edit/results/publish-*`, `alias-*` |
| Help capture | `tas edit --help`, `tas --help`, `chronofork help` | `cli/edit/results/help-*` |
| Block validator | script comparing `record=edit` fields/order/values with the ADR | `cli/edit/block-validation.txt` |

**Executed evidence (round 2):** `rows/A40-cli-contract.txt`, `cli/edit/`,
`targets/cli-tas-edit.stdout.txt`, `targets/cli-tas-edit-release.stdout.txt`.

**Observed result (round 2):** PASS. `cargo test -p chronofork-cli --test tas_edit` (19
passed, debug and release) and the direct-binary CLI probes (301/301 checks) passed. The
accepted flag set was exact; 22 `record=edit` fields appeared in order for eight successful
edits and matched a second decode; streams/exit codes, lexical/`..`/hard-link/symlink
alias protection, read-only source, atomic publish, existing-destination/temp-file
guarantees, I/O failures, help honesty, the declared help/dispatch delta and process
independence were exercised. Final-rename-failure injection was not separately exercised;
the validation/decode/source/write failure paths were. Evidence:
`rows/A40-cli-contract.txt`, `cli/edit/`, `targets/cli-tas-edit.stdout.txt`,
`audit/help-delta.txt`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A41 — Determinism, no host-dependent input and bounded resources

**Passes only when** all of the following are demonstrated:

1. **Cross-process determinism.** The same source bytes, request and metadata produce
   byte-identical output files and identical blocks in separate processes, separated in
   time, under different working directories and different source/output file names.
   Debug and release builds are equal. A changed-host-clock variation was not separately
   exercised; no host-clock API is used in the edit paths and canonical bytes were scanned
   for host-dependent content.
2. **No host-dependent bytes.** A source/help scan and a byte comparison show no
   timestamp, host name, path, process id, nonce, random value, entropy, environment
   variable or filesystem metadata in canonical bytes or valid stdout; the temporary file
   name may include a process id but never enters the movie.
3. **Deterministic anchors.** Rebuilt anchors are byte-identical across separate
   processes, repeated edits in one manager and debug/release, because their envelopes
   carry the canonical fixed snapshot/origin-session identity; retained anchors are
   verbatim source bytes.
4. **Independent operations.** Two managers and repeated edits in one process are
   independent: a prior edit leaves no state in the manager that changes a later edit's
   bytes; two source movies with the same content but different paths produce the same
   output bytes except the caller-supplied metadata.
5. **Bounded resources.** The source read is bounded before decode; output execution is
   bounded by the source step limit and `m`; the edit map is bounded by distinct indices;
   anchor captures are bounded by the count/per-anchor/cumulative limits; output bytes
   are bounded by `MAX_MOVIE_BYTES`; numeric CLI values are validated before `usize`
   conversion/allocation; all size arithmetic is checked. A huge source file, a huge
   declared count and a huge anchor interval are rejected without panic or unbounded
   allocation.
6. **No global state.** A structural audit finds no static mutable state, thread-local,
   singleton or process-wide registry in the edit paths; the library function mutates
   only through its explicit manager/value arguments.
7. **No timing equality gate.** Determinism evidence uses bytes, ticks, hashes and block
   fields only; no wall-clock, latency or throughput value is an acceptance equality.

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Determinism tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test edit`, `--release --test edit` | `targets/sessions-edit.stdout.txt`, `edit/determinism/` |
| Cross-process runs | identical edit runs in separate processes and debug/release binaries | `edit/determinism/`, `cli/edit/results/determinism-*` |
| Host-time/path scan | `rg` audit and clock/path variation runs | `audit/A42-structural-audit.txt`, `cli/edit/results/determinism-other-cwd` |
| Bound/fuzz tests | arbitrary-byte and huge numeric/length fixtures | `edit/bounds/`, `rows/A41-determinism.txt` |

**Executed evidence (round 2):** `rows/A41-determinism.txt`, `edit/determinism/`,
`edit/bounds/`, `audit/A42-structural-audit.txt`.

**Observed result (round 2):** PASS. Cross-process, cwd/name-change, debug/release and
same-manager repeated-rebuild runs were byte-identical; rebuilt anchors carried the
canonical fixed snapshot/origin identity, so an unrelated session or prior edit could not
change a later edit (round-1 F2 closed). v1/v2 recording movies remained byte-identical to
the baseline, no clock/entropy/host-dependent value entered bytes or stdout, bounds were
checked before allocation with executable huge-value/bulk fixtures, and no global state was
found. A changed-host-clock variation was not separately exercised; the byte scan and the
absence of clock APIs support the no-host-time claim. Evidence:
`rows/A41-determinism.txt`, `edit/determinism/`, `edit/bounds/targeted-budget-and-determinism.*`,
`audit/A42-structural-audit.txt`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A42 — Module ownership, one rule per concern and suppression audit

**Passes only when** the increment is proven additive and single-sourced:

1. **Module placement.** The edit implementation lives in
   `crates/chronofork-sessions/src/edit.rs`; the CLI adapter lives in
   `crates/chronofork-cli/src/tas_edit.rs`; `tas.rs` gains dispatch/help wiring only and
   `movie.rs` gains no edit function. No second edit implementation, flag parser, block
   writer or publish helper exists.
2. **One rule per concern.** One movie codec (`Movie::from_bytes`/`to_bytes`), one
   validating constructor per version, one embedded-checkpoint parser
   (`Snapshot::from_bytes`), one anchor parser, one compatibility predicate
   (`validate_envelope`), one checkpoint import path, one checkpoint capture path for
   rebuilt anchors, one first-mismatch comparison helper shared with replay, one aligned
   tick rule, one publish helper. No second format, parser, predicate, replay loop or
   comparison rule is introduced.
3. **Public API discipline.** Every new public item has a caller in the increment or a
   documented external contract (AS-05); the edit request map and internals stay private
   where possible; no existing public session/movie API is renamed or removed.
4. **No dependency, crate or backend change.** `Cargo.toml`/`Cargo.lock` are unchanged,
   `cargo tree -d` is unchanged, backend-api/core/megadrive have zero diff, and no
   backend trait, capability bit, snapshot format, payload version or state hash changes.
5. **No weakened checks.** `git diff -U0` adds no `#[allow]`, `#[ignore]`, `unsafe`,
   `todo!`, `unimplemented!` or deleted/relaxed test/assertion/lint.
6. **Bounds and allocation.** Every external length/count is validated before allocation
   (AS-12); the edit request, output vector, anchor table and temporary output bytes are
   bounded by validated values.
7. **Typed errors.** The edit boundary has one typed error enum preserving
   `MovieError`/`SessionError` sources; the CLI maps every typed failure to stderr and
   exit 1; no stringly error is introduced.
8. **AS-14 discipline.** New modules are split by responsibility and below the
   800-non-test-line review trigger; existing modules grow only by the justified minimal
   wiring. Measured counts are recorded in evidence only (AS-13).

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Structural audit | `rg` scans for codecs, parsers, predicates, import/capture paths, replay loops, comparison rules, publish helpers | `audit/A42-structural-audit.txt`, `rows/A42-structure.txt` |
| Suppression/dependency audit | `git diff -U0` suppression scan, manifest diff, `cargo tree -d`, backend/core diff | `audit/A42-suppressions.txt`, `tree/`, `targets/` |
| Module-size audit | `wc -l` on edit, tas_edit, movie, tas and replay | `audit/A42-structural-audit.txt` |
| Public API scan | every new public name searched across `crates` and `docs` | `audit/A42-naming-audit.txt`, `audit/A42-structural-audit.txt` |

**Executed evidence (round 2):** `rows/A42-structure.txt`, `audit/A42-*.txt`.

**Observed result (round 2):** PASS. The library edit lives in
`crates/chronofork-sessions/src/edit.rs`; the CLI adapter and test target are
`crates/chronofork-cli/src/tas_edit.rs` and `tests/tas_edit.rs`; `tas.rs` has dispatch
only. The structural audit found one codec, one validating constructor per version, one
snapshot/anchor parser, one compatibility predicate, one import/capture path, one
comparison rule and one publish helper. The unused `EditOutcome::into_movie` was removed
(round-1 F5 closed), every new public item had a caller/test, no dependency/backend change
or suppression was added, and the modules are below the AS-14 trigger. Evidence:
`rows/A42-structure.txt`, `audit/A42-structural-audit.txt`,
`audit/A42-suppressions.txt`, `audit/A42-naming-audit.txt`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A43 — Non-regression, full gate, demos and release equality

**Passes only when** all of the following are recorded on the final tree:

1. **M3-A1–A34 regression.** Every prior M3 row still passes; the v1/v2 movie, replay,
   player, studio and TAS targets keep their assertions; v1 and v2 movies replayed,
   compared, anchored or opened in studio through the new build produce their previous
   outputs.
2. **M2/M1 regression.** The M2 acceptance target matrix and the M1 oracle differential
   evidence are re-executed and unchanged.
3. **Full gate.** `./scripts/check.sh` and the individual `cargo fmt --all -- --check`,
   `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
   and `git diff --check` all exit 0.
4. **Demos.** Both `cargo run -p chronofork-cli -- demo` and `-- megadrive demo` run three
   times with exit 0, empty program stderr and stdout byte-identical to the captured
   baseline (`cmp` exit 0, equal SHA-256), plus direct-binary runs.
5. **Declared CLI delta only.** Every pre-existing command's stdout is byte-identical to
   the captured baseline except the declared additive `tas` dispatch/help text for
   `edit`; help-capture diffs are limited to that addition and no other transcript
   changes.
6. **No coverage loss.** Every pre-existing test target is at least at its captured
   baseline count; any increase is explained by the new edit tests, and no assertion is
   removed or relaxed.
7. **Serialized state and dependencies.** The serialized `session save/load` probe on two
   independent roots matches the baseline bytes, tick and state hash; movie fixtures are
   byte-identical where no edit is involved; manifests and `cargo tree -d` are unchanged.
8. **Release equality.** Debug and release edit targets produce equal output bytes,
   blocks and metrics for the same fixtures.
9. **Final-tree binding.** The verifier records HEAD, branch, `git status --short
   --branch`, the changed-path set and artifact SHA-256s, and the PASS is bound to that
   tree. A later edit to an implementation path invalidates the claim until re-verified.

**Executed targets and commands (round 2):**

| Target | Command | Evidence |
|---|---|---|
| Full gate | the gate commands above, all with `CARGO_HOME=/tmp/t7-cargo` | `gate/`, `rows/A43-regression.txt` |
| Target matrix | every target in §4 plus the M2/M3 matrices, with baseline comparison | `target-matrix.txt`, `targets/` |
| Demos | both demos, three cargo runs and three direct-binary runs each, `cmp` and SHA-256 against the baseline | `demos/`, `demos/comparison.txt` |
| Serialized-state and movie probes | two-root `session save/load` probe, movie fixtures and CLI transcripts | `probes/session/`, `probes/movies/`, `probes/transcripts/` |
| M1 oracle differential | `python3 scripts/oracle-diff.py --skip-build` | `probes/oracle/` |
| Release equality | release-mode edit/CLI targets and process runs | `targets/`, `edit/determinism/` |

**Executed evidence (round 2):** `rows/A43-regression.txt`, `gate/`, `demos/`, `probes/`,
`targets/`, `target-matrix.txt`.

**Observed result (round 2):** PASS. The full gate exited 0 (fmt, check, clippy
`-D warnings`, workspace tests, rustdoc `-D warnings`, `./scripts/check.sh`,
`git diff --check`); the target matrix was at or above the 35-target/458-passed baseline
with 37 targets/502 passed and no target below baseline; both demos were byte-identical to
the baseline across three cargo and three direct-binary runs; the v1/v2/M1/M2/M3
regression checks, two-root serialized-state probe, movie fixtures, manifests and
`cargo tree -d` were unchanged; release-mode edit/CLI outputs matched debug. The PASS was
bound to the recorded tree. Evidence: `rows/A43-regression.txt`, `gate/`, `demos/`,
`probes/`, `target-matrix.txt`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

### M3-A44 — Scope honesty, deferred list and no M3-complete claim

**Passes only when** the durable docs, CLI help and capability surfaces state exactly what
is implemented and what is not:

1. **Matches evidence.** Every edit/rerecord/branch capability claim in ADR-0008, this
   plan, the execution model, requirements, milestones, roadmap and `docs/README.md`
   cites an executed row/artifact; a claim without evidence is a finding. ADR-0008 is
   **Accepted** (owner-approved 2026-11-15) and becomes implemented/verified only from
   executed evidence.
2. **No false capability.** The CLI help and docs do not advertise in-place editing,
   timeline insert/delete UI, a persisted rerecord counter, format v3, scripted/variable
   input files, mixing retained and rebuilt anchors, real-hardware input TAS,
   import/export, HTTP/MCP/GUI/daemon, O(1) seek or M3 completion.
3. **Deferred agreement.** The deferred lists in ADR-0008 §11, this plan §10, the
   requirements, milestones, roadmap and `docs/README.md` agree; rerecord/branch editing
   is implemented only if the executed rows exist, and real-hardware input TAS remains
   explicitly unimplemented.
4. **Milestone state.** No M3 item is checked unless its mapped rows are executed and
   passed. If the increment is executed and verified, only the "Rerecord and branch
   editing" item becomes checked; M3 remains incomplete because real-hardware input TAS
   and other deferred items are open.
5. **Requirement honesty.** `FR-TAS-001` remains **Partial** until real-hardware input
   TAS is addressed; `FR-TAS-005` (edit/truncate/branch artifact) is **Implemented**
   (synthetic backend) on the executed rows; `FR-TAS-002`–`FR-TAS-004` and
   `FR-EXEC-005/007/008` keep their verified scope and are not widened.
6. **No format or migration claim.** The docs state that no format/schema version,
   canonical field, counter, limit or checksum coverage changed, that v1/v2 artifacts are
   preserved, and that there is no migration.
7. **Cost honesty.** Help and docs state that an edit re-simulates the output length from
   the embedded checkpoint, that a late edit approaches a full replay, that branch chains
   are full files, that anchor coverage shrinks unless rebuilt, and that movie ids are
   labels that can collide. No performance or capacity target is claimed.
8. **No overclaim after promotion.** The milestone, requirements and docs describe the
   edit capability only from the executed rows and only for the deterministic synthetic
   backend; no claim may exceed that scope, and M3 remains incomplete.

**Executed targets and commands (round 2):**

| Check | Command | Evidence |
|---|---|---|
| Docs vs capability | read ADR-0008, this plan, the requirements, milestones, roadmap, `docs/README.md` and the CLI help; `rg` scope scan | `rows/A44-scope-audit.txt`, `audit/A44-scope-audit.txt` |
| Help honesty | `tas edit --help`, `tas --help`, `chronofork help`, `backend describe` | `cli/edit/results/help-*` |
| Milestone/requirement state | inspect that only executed rows are checked, `FR-TAS-001` is not overstated and M3 is not complete | `audit/A44-scope-audit.txt`, [milestones](../roadmap/milestones.md) |

**Executed evidence (round 2):** `rows/A44-scope-audit.txt`, `audit/A44-scope-audit.txt`,
`cli/edit/results/help-*`.

**Observed result (round 2):** PASS at verification time; this promotion applies the
evidence. CLI help and durable docs advertised no in-place/timeline/persisted-counter/
format-v3/scripted-input/mixed-anchor/real-hardware-TAS/import-export/O(1)-seek or
M3-completion capability; the deferred lists agreed; no M3 item was checked; `FR-TAS-001`
remained Partial and `FR-TAS-005` remained Planned pending promotion. This t6 promotion
updates the durable docs from the executed rows, checks only the rerecord/branch-editing
M3 subitem, keeps real-hardware input TAS deferred and leaves M3 incomplete. Evidence:
`rows/A44-scope-audit.txt`,
`audit/A44-scope-audit.txt`, `audit/A44-adr-plan-audit.txt`, `cli/edit/results/help-*`.

**Status:** executed - passed (t10 repair-round-2 verification, 2026-11-16).

## 6. CLI surface (implemented and verified)

The surface below is the accepted contract shape from
[ADR-0008 §7](../adr/0008-rerecord-and-branch-editing.md#7-cli-surface-output-block-atomicity-and-process-contract)
and is implemented and independently verified (M3-A40, t10 repair-round-2 PASS). Command
and flag spelling may be refined only with an ADR/plan amendment; the semantics, bounds,
block fields, exit codes and stream split are the contract.

```text
chronofork tas edit --movie <FILE> --out <FILE>
                    --set-input <INDEX=MASK>...   or --truncate-after <INDEX>
                    [--movie-id <U64>] [--name <NAME>]
                    [--anchors <keep-prefix|drop|rebuild>]
                    [--anchor-interval <N>]
chronofork tas edit --help
```

`--movie` is read-only; `--out` is written atomically through a sibling temporary file
and rename; the destination must not identify the source. Output is one `key=value`
block beginning `record=edit`, stdout data only, stderr `chronofork: <error>` only, exit
0 on success and 1 on every typed failure. The existing `record/play/verify/compare/
studio` surfaces are unchanged except the declared additive `tas` help/dispatch text that
lists `edit`. The executed round-2 record observed the exact 22-field block for eight
successful edits, 301/301 CLI checks and the declared help delta; final-rename-failure
injection was not separately exercised.

## 7. Baseline preconditions

These capabilities exist before this increment and are preconditions, not rerecord
evidence. The pre-change baseline is captured under `.research/m3-rerecord/baseline/`
(2026-11-15T23:49:31Z, HEAD `3f76d5f`, clean tree).

| Existing evidence | What it shows | What it does not cover here |
|---|---|---|
| `crates/chronofork-sessions/src/movie.rs` (one v1/v2 reader/writer, validating constructors, recorder, verifier, anchors) | Canonical movie bytes, step expectations, anchors and embedded checkpoints are verified (M3-A1–A12, A18–A27) | Any edit, truncation, branch artifact or rerecord semantic |
| `crates/chronofork-sessions/src/replay.rs` (single ranged replay loop, anchor seek, comparison) | Range replay, seek, first divergence and record comparison are verified (M3-A13–A17, A21) | Edit-time prefix verification and suffix re-simulation |
| `crates/chronofork-sessions/src/player.rs` / `player/cache.rs` | Rewind/stepping over a decoded movie is verified (M3-A28–A34) | Any movie mutation or branch creation |
| `crates/chronofork-sessions/src/session.rs` / `manager.rs` | One import path, one checkpoint capture path and one compatibility predicate | An edit policy or output artifact |
| `crates/chronofork-core/src/input.rs` | v1/v2 `InputFrame` fields and neutral semantics | A CLI surface for editing the non-port-0 fields |
| `crates/chronofork-cli/src/tas.rs` / `studio.rs` / `main.rs` | Stable blocks, stderr diagnostics, exit 0/1, bounded read, sibling-temp+rename publish | An edit subcommand, output-alias guard or `record=edit` block |
| `crates/chronofork-test-backend` | Deterministic one-input-per-step execution and canonical hashes | Any edit/rerecord behavior |
| [Research: TAS and debugging](../research/tas-and-debugging.md) §4.3/§5.3/§10.2 | Upstream evidence that rerecording is edit-driven re-simulation and a count is a heuristic | A ChronoFork implementation; research is context only |

The executed increment adds `crates/chronofork-sessions/src/edit.rs`,
`crates/chronofork-sessions/tests/edit.rs`, `crates/chronofork-cli/src/tas_edit.rs` and
`crates/chronofork-cli/tests/tas_edit.rs`, with `main.rs`/`tas.rs` wiring. They are
executed evidence for this increment, not preconditions.

## 8. Traceability

| Criterion | Requirements | ADR-0008 sections |
|---|---|---|
| M3-A35 editing and suffix re-simulation exactness | FR-TAS-001, FR-TAS-005, FR-EXEC-004, FR-EXEC-005, NFR-DET-003, NFR-DET-004 | §2, §3 |
| M3-A36 truncation, bounds and typed request errors | FR-TAS-005, NFR-QUAL-004, NFR-DET-002, AS-12 | §2, §6 |
| M3-A37 anchor handling on edit | FR-TAS-003, FR-TAS-005, FR-EXEC-005, NFR-QUAL-007 | §5, §6 |
| M3-A38 branch metadata, identity and isolation | FR-TAS-002, FR-TAS-005, FR-FORK-005, NFR-ISO-001, NFR-ISO-002, NFR-DET-001 | §4 |
| M3-A39 v1/v2 preservation and device-field boundary | FR-TAS-002, FR-TAS-004, FR-TAS-005, NFR-DET-006, NFR-QUAL-007 | §6 |
| M3-A40 CLI process contract and atomicity | FR-API-001, FR-TAS-005, NFR-QUAL-001, NFR-QUAL-004 | §7 |
| M3-A41 determinism, no host-dependent input, bounds | NFR-DET-001, NFR-DET-003, NFR-ISO-003, NFR-QUAL-005, AS-12 | §8 |
| M3-A42 module ownership and one rule per concern | NFR-QUAL-001, NFR-QUAL-002, NFR-QUAL-006, AS-01, AS-02, AS-05, AS-11, AS-14, AS-15 | §8 |
| M3-A43 non-regression, gate, demos, release equality | NFR-QUAL-001, NFR-QUAL-005, M1/M2/M3-A1–A34 | §2, §8 |
| M3-A44 scope honesty and deferred list | NFR-QUAL-005, NFR-DOC-001, NFR-DOC-003 | §3, §9, §11 |

After the t10 repair-round-2 PASS, `FR-TAS-001` remains **Partial** because real-hardware
input TAS is still absent, while the edit/truncate/branch capability is promoted under
`FR-TAS-005` (implemented for the deterministic synthetic backend). `FR-TAS-002`–`FR-TAS-004`,
`FR-EXEC-005` and `FR-EXEC-008` keep their verified scope; `FR-API-001` covers the
implemented `tas edit` surface; `FR-EXEC-007` remains **Partial** (frame pacing/run loop
absent) and `FR-MD-010` remains **Planned**. M3 remains incomplete.

## 9. Execution record (2026-11-16)

**Executed by the independent t10 repair-round-2 verification: PASS.** Tree under test:
HEAD `3f76d5ffee680d2c7271c7c1d81d8c5a44055675` (branch `agent/m3-complete`) plus the
repaired uncommitted increment paths recorded in
`.research/m3-rerecord/verification-round-2/tree/`; every Cargo command used
`CARGO_HOME=/tmp/t7-cargo`. The verifier's only tracked edit was the dated round-2 PASS
block in [quality gates](quality-gates.md); all other evidence is under the ignored
round-2 root. Round-1 evidence is preserved under `.research/m3-rerecord/verification/`.

1. **Full gate:** `cargo fmt --all -- --check`, `cargo check --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `./scripts/check.sh` and
   `git diff --check` all exited 0.
2. **Target matrix:** 37 targets / 502 passed / 0 failed / 0 ignored against the
   35-target / 458-passed baseline; zero targets below baseline. Edit targets:
   `sessions-lib` 77, `sessions-edit` 22 debug and 22 release, `cli-tas-edit` 19 debug and
   19 release. Per-target counts and the baseline comparison are in `target-matrix.txt`
   and `targets/`.
3. **Demos:** 46/46 checks PASS — three cargo and three direct-binary runs per demo, exit
   0, empty program stderr, byte-identical to the baseline.
4. **Regression:** 134/134 baseline transcript/movie/session checks PASS; v1/v2 recording
   movies, the two-root store/session/checkpoint/state-hash probe, seeded branch, studio
   script, manifests and `cargo tree -d` unchanged; M1 oracle 3/3 traces, final states and
   controls. Evidence: `probes/`, `demos/`, `gate/`.
5. **Acceptance rows:** M3-A35–A44 all executed and passed; the round-1 FAIL findings
   F1/F2/F3/F5 and gaps F6 were repaired and re-verified. Per-row commands and artifacts
   are in `rows/` and cited in §5. No row was passed by inspection or an absent target.
6. **Independent verification beyond product tests:** 13/13 harness scenarios in debug and
   release, 301/301 CLI process-contract checks, 19/19 re-signed mutation probes, and the
   structural/suppression/scope audits in `audit/`. Evidence: `cli/`, `edit/`, `harness/`,
   `audit/`.
7. **Evidence integrity:** `MANIFEST.sha256` covers the round-2 artifact set and verified
   with `sha256sum -c` (1087 entries, exit 0). The dated gate-result block is in
   [quality gates](quality-gates.md).
8. **Caveats:** the changed-host-clock variation was not literally exercised (separate
   processes, cwd/name changes, debug/release, same-manager repeat and a host-byte scan
   were); final-publish rename-failure injection was not separately exercised (validation,
   decode, source, write and existing-destination paths were); the 32-bit `usize` bounds
   case is not representable on the 64-bit acceptance platform and is not claimed; the
   bulk-payload fixtures are test-only backends. The PASS is bound to the recorded tree;
   any later edit to an implementation path invalidates it until re-verified.
9. **No commit, push, tag or merge** was made. The acceptance claim is the deterministic
   synthetic backend on Linux; real-hardware input TAS and the other §10 deferred items
   remain unimplemented and M3 is **not complete**.

## 10. Out of scope and stated limitations

This increment does not implement or claim:

- **In-place editing or rewrite-on-open.** The source artifact is never mutated; the
  output is a new file.
- **A timeline editor.** Insert/delete/reorder steps beyond suffix truncation, a
  timeline UI, a TUI, undo/redo, markers, lag log or branch switching inside the player.
- **Scripted or variable input files.** No input-log parser, per-step script file,
  text format, BK2/other import/export adapter or foreign TAS format.
- **A persisted rerecord counter or ancestry service.** No counter field, edit log,
  global registry, parent lookup, cycle detection or automatic chain traversal; the
  printed `edits`/`prefix_steps`/`resimulated_steps` are per-invocation diagnostics.
- **Real-hardware input TAS at the increment-5 boundary.** At this increment the Mega
  Drive `step` ignored input and no controller or multi-port hardware input was
  implemented; `FR-MD-010` was Planned. It was delivered by the later final increment
  under [ADR-0009](../adr/0009-real-hardware-input-tas.md) (M3-A45–A53 in the [M3 hardware
  acceptance plan](m3-hardware-acceptance.md)); the increment-5 scope above is unchanged.
- **Mixing retained prefix anchors with rebuilt suffix anchors** and anchor-based
  skipping of edit prefix verification.
- **CLI editing of ports 1–3, axes or keys.** Full-frame replacement is library-only;
  the CLI preserves those fields.
- **Format changes.** No v3, new field, format/schema bump, counter, checksum coverage
  or limit change; no migration, upgrade or downgrade.
- **O(1)/random-access seek, reverse execution, parallel/threaded edit or replay, and any
  performance, latency or capacity target.** Editing is bounded re-simulation, not a
  faster path.
- **Persistent or shared caches, compression/encryption, authentication, deduplication,
  automatic retention/GC, HTTP API, MCP adapter, GUI or daemon.**
- **Cross-backend canonicalization, cross-platform runtime claims and hardware-accuracy
  claims.** Acceptance is the deterministic synthetic backend on Linux.
- **A failed row converted into a skip, `#[ignore]`, relaxed assertion or narrower
  claim.**
- **M3 completion.** Even this fully executed and verified rerecord/branch-editing
  increment leaves real-hardware input TAS and the other deferred items open, so M3 is
  not complete.

## 11. Change control

- A change to a criterion, expected typed category, planned command, target name, output
  field or artifact path requires an explicit ADR-0008/plan amendment; it must not be
  made by quietly renaming a test or loosening an assertion.
- The names in this document describe the accepted contract shape. Implementation
  spelling may be refined only by an approved amendment; the semantics, bounds,
  defaults, block fields, typed boundaries and rejection categories must remain
  equivalent.
- **ADR-0008 is Accepted** (owner-approved 2026-11-15), implemented and independently
  verified for the deterministic synthetic backend (t10 repair-round-2 PASS, 2026-11-16).
  The verification authorizes no commit, push, tag or merge by itself.
- If a planned command cannot run because its target is absent, the row remains
  `planned - not yet executed`; the gap is reported, not hidden.
- Once a row has executed, its result is append-only evidence linked from
  [quality gates](quality-gates.md); do not rewrite history or relabel a failure.
- A failed row blocks this increment. Never weaken, delete or skip a test to obtain a
  green gate, and never widen a claim beyond the verifier's synthetic-backend scope.
- Deferred items in §10 stay deferred; a later increment must design them on their own
  decisions and evidence.
- **2026-11-15 (design).** This plan was created with ADR-0008 as the design stage of the
  increment. All rows are `planned - not yet executed`; no criterion, command, contract
  byte, limit, capability or verification result is claimed.
- **2026-11-15 (owner approval).** ADR-0008 moved from Proposed to Accepted on the owner's
  statement "Aprovar ADR-0008 como proposto (Recomendado)" at
  2026-11-15T21:09:29-03:00 (America/Fortaleza), 2026-11-16T00:09:29Z. No criterion,
  command, contract byte, limit, planned row status or capability claim changed.
- **2026-11-15 (repair-round clarifications, t9).** No criterion, command, contract byte,
  limit or row status changes. The plan records the executable fixtures and canonical
  rebuilt-anchor identity used by the repair round: the edit target now carries
  test-only bulk-payload fixtures for M3-A36.6 and M3-A37.5, and A41.3's determinism
  statement no longer assumes identical manager identity. Row status remains whatever
  the executed evidence and the independent verifier record says; this note is not a PASS
  and does not promote any row.
- **2026-11-16 (execution and promotion, t10/t6).** The independent repair-round-2
  verification executed M3-A35–A44 on the repaired uncommitted tree at HEAD `3f76d5f` and
  returned **PASS**; every row is promoted to `executed - passed`, the round-1 FAIL is
  preserved under `.research/m3-rerecord/verification/` and the round-1 findings
  F1/F2/F3/F5/F6 are closed. No criterion, command, contract byte, limit, block field or
  deferred item changed; the plan records the executed commands/artifacts and the
  non-blocking caveats in §9. M3 remains incomplete and no commit or push was made.
