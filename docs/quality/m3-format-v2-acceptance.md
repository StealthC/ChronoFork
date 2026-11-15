# M3 Format v2 — Executable Acceptance Plan

- **Status:** Executed; every row M3-A18–A27 is `executed - passed` (independent t7 PASS,
  2026-11-15). The increment is complete for the deterministic synthetic backend; M3 is
  **not complete**.
- **Date:** design 2026-11-15; execution record 2026-11-15
- **Companion contract:** [ADR-0006: movie format v2 — checkpoint anchors and multi-device
  input](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) (**Accepted**;
  implemented and independently verified for the deterministic synthetic backend).
- **Extends:** the accepted v1 contract [ADR-0004](../adr/0004-tas-movie-format.md) and
  the accepted increment-2 contract
  [ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md). Their executed rows
  (M3-A1–A12 and M3-A13–A17) are **regression preconditions**, never acceptance for
  M3-A18–A27.
- **Related:** [execution model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md),
  [milestones](../roadmap/milestones.md), [M3 first-increment acceptance
  plan](m3-tas-replay-acceptance.md), [M3 increment-2 acceptance
  plan](m3-partial-replay-acceptance.md), [testing strategy](testing-strategy.md),
  [quality gates](quality-gates.md), [anti-slop policy](anti-slop-policy.md).

> **Verification record (2026-11-15).** The independent t7 verifier executed
> M3-A18–A27 on the uncommitted increment-3 tree at HEAD
> `9643d74fd37097ebe487a398fe0a0259a3b3c7aa` (branch `agent/m3-complete`) plus the
> increment-3 working-tree paths with `CARGO_HOME=/tmp/t7-cargo`, and returned **PASS**.
> The full gate, both demos, the v1/M2 regression, the serialized-state probe and the
> structural audits are green; per-row commands, results and artifact names are in §5 and
> §9. Raw evidence is in the ignored `.research/m3-v2/verification/` directory
> (`SUMMARY.txt`, `target-matrix.txt`, `rows/A18-*.txt`–`A27-*.txt`, `audit/`, `targets/`,
> `cli/`, `demos/`, `probes/`, `gate/`, `tree/`, `MANIFEST.sha256`); the pre-change
> baseline is `.research/m3-v2/baseline/`. The dated gate-result block is in
> [quality gates](quality-gates.md). M3 is **not complete**: in-process rewind/state
> cache and interactive stepping, rerecord/branch editing and real-hardware input TAS
> remain unimplemented. Non-blocking observations O1–O4 are recorded in §9.

## 1. Status rules

1. **`planned - not yet executed`** means the criterion has a planned test target,
   command and evidence artifact, but no executed result and no independent verifier
   record exists. No row in this document has that status after the t7 execution record.
2. A row becomes **`executed - passed`** only when all of the following are recorded: the
   exact command, exit code, tree state (HEAD plus uncommitted paths), test output
   summary, the ignored evidence artifact path and an independent verifier review that
   returns PASS. The authoritative dated gate block belongs in
   [quality gates](quality-gates.md); this document links to it and does not copy volatile
   counts.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact or unverified claim; a failed row blocks this increment. A known
   failure is never converted into a skip, an `#[ignore]`, a relaxed assertion or a
   narrower claim after the fact.
4. A test may be committed before the implementation as a red test only if it fails for
   the expected typed reason. A test that cannot run is not acceptance evidence.
5. No v2 behavior may be described as implemented, working, supported or complete on the
   strength of ADR-0006, this plan alone, a code review, a demo run or a single happy-path
   run. The t7 executed artifacts are the v2 evidence; the v1 and increment-2 evidence
   (M3-A1–A17) is evidence for v1 behavior only.
6. A planned test whose command cannot execute because its module or test target does not
   exist remains `planned - not yet executed`; it does not become "passed" because the
   absent target produced no failure.
7. A row marked passed must cite a verifier artifact whose SHA-256 matches the recorded
   manifest. Late or reconstructed evidence does not reopen a failed row.
8. A row may not be satisfied by re-running an existing v1 capability. Each row requires
   the new v2 container, anchor, seek, multi-device or CLI behavior described here; a green
   M3-A1–A17 run is a regression precondition, never acceptance for A18–A27.
9. Acceptance is limited to the deterministic synthetic backend on the Linux acceptance
   platform. The Mega Drive slice ignores input and is not an acceptance backend for input
   TAS; no hardware input or accuracy claim is made.

As of the t7 execution record every M3-A18–A27 row is `executed - passed`; none is
failed. The v1 and increment-2 rows were re-executed as regression preconditions.

## 2. Scope, definitions and evidence

- **v2 movie:** the single file defined by
  [ADR-0006 §2](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md#2-v2-container-one-canonical-cfmovie-file):
  magic `CFMOVIE2`, format version 2, input-schema version 2, metadata, the embedded
  verbatim canonical checkpoint, an ordered anchor table, 72-byte multi-device step
  records and a trailing SHA-256.
- **Anchor:** a verbatim canonical `Snapshot::to_bytes()` checkpoint of the recording
  session captured after recorded step index `k` (`0 <= k < step_count`), stored as
  `anchor_step_index`, `anchor_snapshot_length` and the snapshot bytes
  (ADR-0006 §5). The embedded checkpoint is the implicit pre-step-0 anchor and is not
  repeated.
- **Anchor binding:** the decode/construction checks that the anchor indices are strictly
  increasing, each index is below `step_count`, the anchor snapshot tick equals the
  recorded outcome tick of that step, and the anchor snapshot state hash equals the
  recorded expected state hash of that step (ADR-0006 §4–§5).
- **Anchored seek:** a ranged replay over `(from, until]` that selects the nearest anchor
  at or before the prefix end, imports that anchor through
  `SessionManager::create_session_from_snapshot_bytes`, and executes only the recorded
  steps after the anchor up to `until` (ADR-0006 §6).
- **Prefix verification with an anchor:** the pre-anchor prefix is **not** re-simulated.
  The anchor boundary is verified structurally against the recorded expectation of its
  step and covered by the movie and snapshot checksums; the executed suffix is verified
  exactly. Full-range `verify` (no `--from`) selects no anchor and still verifies every
  step (ADR-0006 §6).
- **Multi-device frame:** `InputFrame` with four 32-bit digital ports (port 0 is the v1
  mask), four signed 16-bit analog axes and a 64-bit keyboard bitmap; one frame per step,
  no event queue and no host-timed input (ADR-0006 §7).
- **v1 non-regression:** every v1 file decodes, verifies and re-encodes byte-identically; a
  recording with no v2 feature is byte-identical to the pre-change v1 output; default CLI
  blocks are byte-identical; M3-A1–A17 and the M2/M1 evidence stay green.
- **Acceptance backend:** the deterministic synthetic backend
  (`chronofork-test-backend`) is the only acceptance backend. Its `APPLY_INPUT` consumes
  port 0 exactly as today and consumes ports 1–3, axes and keyboard deterministically only
  when non-neutral (ADR-0006 §8). The Mega Drive slice ignores input and is not an
  acceptance backend.
- **Platform:** Linux is the acceptance platform, matching the M2 and M3 evidence
  platforms. Windows and macOS are compile-checked in CI; runtime v2 behavior there is not
  claimed.
- **Evidence artifacts:** raw output is under the ignored
  `.research/m3-v2/verification/` path: `SUMMARY.txt`, `target-matrix.txt`,
  `rows/A18-*.txt` … `rows/A27-*.txt`, `audit/`, `targets/`, `cli/`, `demos/`, `probes/`,
  `gate/`, `tree/` and `MANIFEST.sha256`. The pre-change baseline is
  `.research/m3-v2/baseline/`. Each artifact records the command, tree state, exit code
  and the relevant stdout/stderr/SHA-256; the manifest binds the artifact set. Artifacts
  are diagnostic evidence, not durable claims; this document summarizes results without
  volatile counts. The dated gate-result block is in
  [quality gates](quality-gates.md) and is verifier-owned.
- **No host time in canonical data or reports:** v2 bytes contain no timestamp, process
  id, host label, nonce, path or entropy. Comparisons use ticks, state hashes, step
  indices, movie bytes and SHA-256, never wall-clock time or filesystem modification time.
  Anchor placement is a step-interval policy, never host time.

## 3. Increment exit criteria map

Every mapped row is `executed - passed` and the independent t7 verifier recorded PASS.
This increment is complete for the deterministic synthetic backend; it does not complete
M3.

| Increment capability (verified) | Acceptance criteria |
|---|---|
| v2 single-file container, canonical bytes, limits and deterministic anchor table | M3-A18 |
| Bounded v2 decode and the full typed rejection matrix | M3-A19 |
| Anchor capture, ordering and tick/state-hash binding validation | M3-A20 |
| Anchor-based seek in the single ranged replay path, with fallback and first divergence | M3-A21 |
| Schema-2 multi-device input and deterministic synthetic-backend consumption | M3-A22 |
| v1 non-regression, one codec/replay loop and no weakened tests | M3-A23 |
| CLI v2 record/play/verify/compare process contract and honest help | M3-A24 |
| Determinism, no host time and debug/release equality | M3-A25 |
| Full gate, demos, M2/M1 regression and the v1 M3 rows | M3-A26 |
| Scope honesty, explicit deferred list and no M3-complete claim | M3-A27 |

**This increment is complete:** ADR-0006 is accepted (owner-approved
2026-11-15T17:06:17-03:00) and implemented; every row below is `executed - passed`; the
M3-A1–A17 rows still pass unchanged; the full gate and both demos are green against the
captured baseline; no pre-existing test target lost coverage; and the independent t7
verifier recorded PASS bound to the tree under test (§9).

**This increment does not complete M3.** In-process rewind buffer/state cache and
interactive stepping, rerecord and branch editing, and real-hardware input TAS (the Mega
Drive `step` ignores input) remain deferred and unchecked in
[milestones](../roadmap/milestones.md).

## 4. Executed test targets (t7)

These targets were executed by the independent t7 verifier on the increment-3 tree with
`CARGO_HOME=/tmp/t7-cargo`. Per-target counts and the baseline comparison are in
`.research/m3-v2/verification/target-matrix.txt`; raw stdout/stderr is in `targets/` and
`gate/`. The exact counts stay in the evidence, not in this durable document.

| Target | Executed command | Criteria |
|---|---|---|
| Core input unit tests in `crates/chronofork-core/src/input.rs` | `cargo test -p chronofork-core` | M3-A22 |
| Synthetic-backend unit tests in `crates/chronofork-test-backend/src/lib.rs` | `cargo test -p chronofork-test-backend` | M3-A22 |
| Movie v2 unit tests in `crates/chronofork-sessions/src/movie.rs` (or an ADR-approved `movie/v2` submodule) | `cargo test -p chronofork-sessions --lib` | M3-A18, M3-A19, M3-A20 |
| Movie v2 integration target `crates/chronofork-sessions/tests/movie_v2.rs` | `cargo test -p chronofork-sessions --test movie_v2` | M3-A18, M3-A19, M3-A20, M3-A23 |
| Release-mode v2 equality | `cargo test -p chronofork-sessions --release --test movie_v2` | M3-A25 |
| Replay anchor tests in `crates/chronofork-sessions/src/replay.rs` and `crates/chronofork-sessions/tests/replay.rs` | `cargo test -p chronofork-sessions --lib` and `cargo test -p chronofork-sessions --test replay` | M3-A21, M3-A23 |
| Release-mode anchor replay | `cargo test -p chronofork-sessions --release --test replay` | M3-A25 |
| CLI process target `crates/chronofork-cli/tests/tas.rs` | `cargo test -p chronofork-cli --test tas` | M3-A21, M3-A22, M3-A24 |
| Existing CLI regression | `cargo test -p chronofork-cli` | M3-A23, M3-A24 |
| M2 session target matrix plus the CLI session-storage target | the M2 matrix from the [M3 plan §4](m3-tas-replay-acceptance.md#4-executed-test-targets-m3-t4) | M3-A23, M3-A26 |
| Existing v1 movie/replay targets | `cargo test -p chronofork-sessions --test movie`, `cargo test -p chronofork-sessions --release --test movie` | M3-A23, M3-A26 |
| Full gate | `./scripts/check.sh`; `cargo fmt --all -- --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`; `git diff --check` | M3-A23, M3-A26 |
| Demos | `cargo run -p chronofork-cli -- demo` and `cargo run -p chronofork-cli -- megadrive demo`, three runs each, `cmp` and SHA-256 against the baseline | M3-A26 |
| Manual/process transcripts | separate-process v2 record/play/verify/compare; multi-device and anchor-interval runs; rejection matrix; help capture | M3-A18, M3-A21, M3-A24, M3-A25 |

## 5. Acceptance criteria

Every row below is `executed - passed` in the t7 record. The pass conditions are retained
as the verified contract; each row records the executed targets, the observed result and
the artifact paths without volatile counts. The typed categories and boundaries are fixed
by [ADR-0006 §4–§6](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md#4-decode-order-bounds-and-typed-rejection).

### M3-A18 — v2 container round-trip, canonical bytes, limits and deterministic anchors

**Passes only when** all of the following are demonstrated on the deterministic synthetic
backend:

1. **Round-trip.** A v2 movie built from known metadata, a known canonical checkpoint, a
   known ordered anchor table and a known input sequence decodes to exactly the same
   format version (2), input-schema version (2), movie id, name, parent id, start tick,
   embedded snapshot bytes, anchor count, anchor indices and anchor snapshot bytes, step
   count and step records (outcome tick, all four port masks, all four axes, keyboard
   bitmap and state hash). The embedded bytes and every anchor bytes are exactly the
   source `Snapshot::to_bytes()` output and are decoded only through
   `Snapshot::from_bytes`.
2. **Exact canonical size.** Fabricated v2 movies have the exact encoded length: the
   94-byte fixed overhead (which already includes the trailing 32-byte checksum), plus the
   name bytes, plus the embedded snapshot length, plus `16 + snapshot_length` per anchor,
   plus `72 × step_count`. A zero-step, zero-anchor v2 movie with a named metadata is
   valid and has the expected length; a single anchor and a single step each add exactly
   their documented bytes.
3. **Canonical determinism.** Recording the same checkpoint inputs, metadata, multi-device
   frames and anchor interval twice in independent runs (including a run separated in time
   and two fresh no-root CLI processes) produces byte-identical v2 movies and equal
   SHA-256; decoding and re-encoding reproduces identical bytes. Debug and release
   artifacts are byte-identical. Anchor placement is identical for identical step counts
   and intervals.
4. **Anchor table determinism.** For `step_count = n` and interval `N > 0`, the recorded
   anchor indices are exactly `N - 1, 2N - 1, … < n`; the count is `n / N`; the interval
   value itself is not stored; changing only the interval changes the anchor table but not
   the step records or their expected hashes.
5. **Metadata and anchors are not state.** Changing only the movie id, name or parent id
   changes the bytes but not start/end ticks, step count, step records, final state hash or
   replay result. Changing only the anchor table (valid anchors) changes the bytes but not
   the step records or the anchor-less replay result. Anchors are seek metadata, not
   execution expectations.
6. **Limits and checked arithmetic.** The writer refuses a v2 movie above
   `MAX_V2_STEPS`, `MAX_ANCHORS`, `MAX_EMBEDDED_SNAPSHOT_BYTES`,
   `MAX_ANCHOR_SNAPSHOT_BYTES`, `MAX_ANCHOR_BYTES` or `MAX_MOVIE_BYTES` with the
   documented typed category before producing bytes; a declared count or length that would
   overflow is rejected; the maximal documented combination is shown to fit below
   `MAX_MOVIE_BYTES`. Raising a limit without an ADR amendment and new tests fails review.
7. **No host time or host state.** The v2 module uses no clock, filesystem metadata,
   environment, process id, path, nonce or entropy in encoding. A parser dump of a v2
   movie reports only the documented fields, and a recording's bytes are unchanged when
   the wall clock and the file's modification time change.
8. **Zero-step/zero-anchor and v2 feature combinations.** A zero-step, zero-anchor v2
   movie round-trips; a v2 movie with anchors and neutral device fields round-trips; a v2
   movie with non-neutral device frames and no anchors round-trips; a v1 movie remains
   format 1 with 44-byte records and no anchor table.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Movie v2 unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt`, `rows/A18-container-roundtrip.txt` |
| Movie v2 integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test movie_v2 -- --nocapture` | `targets/sessions-movie_v2.stdout.txt`, `rows/A18-container-roundtrip.txt` |
| CLI v2 record | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas` | `cli/v2-record-1.*`, `cli/A18-movie-fields.txt`, `targets/cli-tas.stdout.txt` |
| Determinism transcript | record twice in separate processes with `--anchor-interval` and multi-device flags; `cmp` and SHA-256 | `cli/v2-record-1.*`, `cli/v2-record-2.*`, `cli/v2-record-interval-4.*`, `cli/results.txt`, `audit/A25-host-time-audit.txt` |

**Observed result:** PASS. Library/integration tests plus separate-process CLI runs demonstrate the v2 format-2/schema-2 round-trip (metadata, embedded checkpoint, ordered anchors and device step records), exact canonical sizes (`v2-a` = 94 + 8 + 459 + 3 × (16 + 459) + 720 bytes; `v2-neutral` and `v1-plain` match their formulas), deterministic bytes across two processes and a release build, anchor indices 2/5/8 for interval 3, metadata/anchor independence, every documented limit and the maximal combination, no host-time input, and zero-step/zero-anchor plus v1-shape cases. Evidence: `rows/A18-container-roundtrip.txt`, `cli/A18-movie-fields.txt`, `cli/v2-*`, `targets/sessions-*`, `audit/A25-host-time-audit.txt`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A19 — Bounded v2 decode and typed rejection matrix

**Passes only when** every case below returns its named typed category, without a panic,
without a partially usable `Movie`, without unbounded allocation, without executing a
backend and without modifying a valid file or session. The exact error spelling may be
refined during implementation; the categories and boundaries are fixed by
[ADR-0006 §4](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md#4-decode-order-bounds-and-typed-rejection).

| Mutated input | Required typed result |
|---|---|
| Magic other than `CFMOVIE1`/`CFMOVIE2` | `BadMagic`; nothing further parsed |
| `CFMOVIE1` with format 0, 2 or 3, or `CFMOVIE2` with format 0, 1 or 3 | `UnsupportedFormatVersion` with found/supported (supported is 1 for the `CFMOVIE1` path, 2 for the `CFMOVIE2` path) |
| `CFMOVIE1`/format 1 with schema 0 or 2, or `CFMOVIE2`/format 2 with schema 0, 1 or 3 | `UnsupportedInputSchema` with found/supported (supported is 1 for format 1, 2 for format 2) |
| Any covered byte flipped in a valid v2 movie | `ChecksumMismatch`; no field is trusted before the checksum passes |
| Truncation at each boundary from 0 to the encoded v2 length | Every cut is typed and never panics. A cut below the 86-byte shared minimum returns `TooShort`; a raw cut of a longer v2 movie returns `ChecksumMismatch` because the trailing checksum is verified first; a re-signed short body returns `Truncated`, and a re-signed v2 body below the 94-byte v2 minimum returns `TooShort { minimum: 94 }` |
| Bytes appended after the checksum, or inserted before it with a recomputed checksum | checksum rejection or `TrailingBytes`; never reinterpretation |
| File larger than `MAX_MOVIE_BYTES` | `FileTooLarge` before reading/allocating the body |
| `step_count > MAX_V2_STEPS` | `TooManySteps { limit: MAX_V2_STEPS, found }` before allocation |
| Declared step-record bytes larger than the remaining body | `Truncated`/`TooManySteps` before `Vec::with_capacity` |
| `anchor_count > MAX_ANCHORS` | `TooManyAnchors` before allocation |
| `anchor_count × 16` larger than the remaining body | `Truncated` before allocation |
| Declared `snapshot_length` larger than the remaining body | `Truncated` before slicing/allocating |
| Declared `snapshot_length` above `MAX_EMBEDDED_SNAPSHOT_BYTES` | `EmbeddedSnapshotTooLarge` naming the limit |
| Anchor snapshot length above `MAX_ANCHOR_SNAPSHOT_BYTES` | `AnchorSnapshotTooLarge` naming the anchor ordinal |
| Cumulative anchor snapshot bytes above `MAX_ANCHOR_BYTES` | `AnchorTableTooLarge` |
| Declared `anchor_snapshot_length` larger than the remaining body | `Truncated` before slicing/allocating |
| Name longer than `MAX_NAME_BYTES`, invalid UTF-8 or a control character | `InvalidName` |
| `movie_id == 0` | `InvalidMovieId` |
| `parent_movie_id == movie_id` | `InvalidParentMovieId` |
| `start_tick != embedded snapshot tick` | `StartTickMismatch` |
| Outcome ticks not strictly increasing, or the first not greater than `start_tick` | `TickSequence` naming the input index |
| Empty, truncated, checksum-bad or unsupported embedded snapshot | `InvalidEmbeddedSnapshot` wrapping the preserved `SessionError` |
| Corrupt/truncated/checksum-bad anchor snapshot | `InvalidAnchorSnapshot` naming the anchor ordinal and wrapping `SessionError::InvalidSnapshot` |
| Structurally valid anchor snapshot with a foreign backend id/version/payload/config | movie decodes; a valid replay fails with `AnchorImport` wrapping `SessionError::IncompatibleSnapshot`; no second compatibility predicate |
| Arbitrary bytes of length 0 through 2048 | rejected or explicitly valid; never a panic |
| A tiny file declaring a huge step count, anchor count or snapshot length | rejected by arithmetic and remaining-byte checks **before** allocation |

In addition:

1. A declared huge count or length must be rejected before `with_capacity` or slicing,
   demonstrated on a tiny file whose declared value is far larger than the file.
2. Because the trailing checksum is verified first, each test for a later structural
   rejection (magic, versions, schema, metadata, counts, anchor binding) rebuilds a valid
   trailing checksum so the intended validator is exercised; an unmodified byte flip is the
   checksum case.
3. The embedded checkpoint and every anchor snapshot are parsed by `Snapshot::from_bytes`
   only; a structural audit shows no second snapshot parser, checksum rule or
   compatibility predicate.
4. Every rejection returns no partially usable `Movie`, creates no session, executes no
   step and leaves any valid input file and pre-existing session unchanged.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Movie v2 unit rejection tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt`, `rows/A19-rejection-matrix.txt` |
| Movie v2 integration rejection | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test movie_v2 -- --nocapture` | `targets/sessions-movie_v2.stdout.txt`, `rows/A19-rejection-matrix.txt` |
| CLI v2 rejection | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas` | `cli/mut-*.*`, `targets/cli-tas.stdout.txt`, `rows/A19-rejection-matrix.txt` |
| Structural audit | `rg -n "struct .*Reader\|struct .*Writer\|fn to_bytes\|fn from_bytes\|fn validate_envelope" crates/chronofork-sessions crates/chronofork-backend-api` | `audit/A19-structural-audit.txt` |

**Observed result:** PASS. Library rejection tests plus the re-signed CLI mutation matrix exercise the full typed matrix (magic/format/schema, checksum, metadata, start tick, tick sequence, step/anchor/snapshot limits, anchor order/range/tick, corrupt embedded/anchor snapshots, invalid UTF-8, trailing/truncated/arbitrary bytes). Every CLI case exits 1 with a typed `chronofork:` stderr diagnostic and empty stdout; huge declared values are rejected before allocation; the checksum-first taxonomy is preserved; one snapshot parser and one compatibility predicate remain. Evidence: `rows/A19-rejection-matrix.txt`, `cli/mut-*`, `targets/sessions-*`, `targets/cli-tas.*`, `audit/A19-structural-audit.txt`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A20 — Anchor capture, ordering and binding validation

**Passes only when** all of the following are demonstrated:

1. **Recording isolation and capture.** A v2 recording with `anchor_interval = N > 0`
   captures each anchor in the separate imported recording session through
   `SessionManager::snapshot` plus `snapshot_bytes`; the source session's tick, state hash
   and mutable state are unchanged by the recording, and only the source checkpoint record
   from the pre-recording capture is a permitted source-side effect. Each anchor snapshot
   is the exact canonical `Snapshot::to_bytes()` output for the state after its recorded
   step.
2. **Deterministic indices.** For `n` recorded steps and interval `N`, the anchor indices
   are exactly `N - 1, 2N - 1, … < n`; there are `n / N` anchors; they are strictly
   increasing; `N = 0` records no anchors; a predicted count above `MAX_ANCHORS` is
   rejected before execution. An interval greater than the step count yields zero anchors
   at the library layer, and the CLI rejects it before execution as a typed flag error.
3. **Accepted shapes.** Valid anchors are accepted at the first step, the last step and at
   multiple non-uniformly spaced indices (the format does not require constant spacing);
   a v2 movie with zero anchors is accepted; a zero-step movie with zero anchors is
   accepted.
4. **Binding rejection.** Each of the following is rejected with its typed category at
   both decode and the validating constructor, without a partial movie, session or
   execution: equal or decreasing anchor indices (`AnchorIndexNotIncreasing`), an index
   `>= step_count` (`AnchorIndexOutOfRange`), any anchor on a zero-step movie
   (`AnchorIndexOutOfRange`), an anchor snapshot tick different from the recorded outcome
   tick (`AnchorTickMismatch`), and an anchor snapshot state hash different from the
   recorded expected state hash (`AnchorStateHashMismatch`).
5. **Snapshot failure.** A corrupt, truncated or checksum-bad anchor snapshot is
   `InvalidAnchorSnapshot` naming its ordinal and preserving the `SessionError`; a missing
   or over-long declared anchor snapshot is `Truncated` or `AnchorSnapshotTooLarge` before
   allocation.
6. **Import failure is deferred.** A structurally valid anchor snapshot whose backend
   identity, version, payload version or configuration hash this build rejects passes
   decode; a valid anchored replay fails with `AnchorImport` wrapping the existing
   `SessionError::IncompatibleSnapshot`, and no second compatibility predicate exists.
7. **Failed capture is reported.** A recording whose anchor checkpoint capture, snapshot
   read/serialize or byte-budget check fails returns a typed error
   (`AnchorCaptureFailed`, `AnchorSnapshotTooLarge` or `AnchorTableTooLarge`), produces no
   movie bytes, leaves the source unchanged and never replaces an existing movie file.
8. **One validation implementation.** The writer's validating constructor and the reader
   apply the same checks (a structural check plus behavior equality on the shared test
   matrix); there is exactly one anchor validator, one snapshot parser and one set of
   binding rules.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Movie v2 anchor unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt`, `rows/A20-anchor-binding.txt` |
| Movie v2 anchor integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test movie_v2 -- --nocapture` | `targets/sessions-movie_v2.stdout.txt`, `rows/A20-anchor-binding.txt` |
| Recording isolation transcript | library test plus CLI record with and without `--root`, source/bystander tick and hash before/after | `cli/record-anchored-from-root.*`, `cli/seed-*.*`, `cli/results.txt` |
| Structural audit | `rg -n "MovieAnchor\|AnchorIndex\|Snapshot::from_bytes\|validate_envelope" crates/chronofork-sessions` | `audit/A20-structural-audit.txt` |

**Observed result:** PASS. Anchors are captured in the imported recording session through the canonical snapshot path; an anchored record seeded from a durable root leaves the source tick/state hash unchanged; indices are deterministic and non-uniform tables are accepted; index order/range, tick and state-hash mismatches are rejected through decode and the public constructors; corrupt/over-long anchors are typed before allocation; a structurally valid foreign anchor decodes and fails only at replay with `AnchorImport`; failed capture is typed and leaves no movie; the writer and reader share one anchor validator. Evidence: `rows/A20-anchor-binding.txt`, `cli/record-anchored-from-root.*`, `cli/seed-*`, `cli/mut-anchor-*`, `audit/A20-structural-audit.txt`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A21 — Anchor-based seek, fallback and first divergence

**Passes only when** all of the following are demonstrated on the deterministic synthetic
backend:

1. **Nearest-anchor selection.** For a v2 movie with anchors at several indices, a ranged
   replay over `(from, until]` selects the greatest anchor step index whose recorded
   outcome tick is `<= from`. Cases: `from == start_tick` selects no anchor; `from` exactly
   on an anchor step selects that anchor; `from` between two anchors selects the earlier
   one; an anchor after `from` is never selected; an anchor after `until` is never
   selected. An anchor on the final recorded step is a valid format value but cannot be
   selected by a valid non-empty range, because `from < until <= end_tick`; that case is
   asserted explicitly rather than silently assumed.
2. **Boundary results equal a full replay.** For the same `until`, an anchored replay
   reports the same `range_from`, `range_until`, `prefix_steps`, `window_steps`,
   `final_tick` and `final_state_hash` as a full replay from the embedded checkpoint to that
   `until` (and as the recorded boundary expectation); `replayed_steps` is the actual
   executed count and equals `(index at until) - anchor_step`, which is smaller than
   `prefix_steps + window_steps` when an anchor is used. The report has `anchored == true`,
   `anchor_step == Some(k)` and `anchor_tick == Some(recorded tick of step k)`.
3. **Import path.** The anchor is imported only through
   `SessionManager::create_session_from_snapshot_bytes`; an import-incompatible anchor
   fails with `AnchorImport { anchor_step, source }` and creates no second session. A
   structurally valid movie with an import-incompatible anchor returns the **range error**
   for an invalid range, proving range validation precedes anchor import.
4. **First divergence after the anchor.** A valid v2 movie altered at one anchor-following
   step to have a wrong expected state hash makes a full replay fail at the existing
   `Divergence` index/tick/expected/observed, and the anchored replay reports the same first
   divergence at the same index when that step is after the anchor. Altering only a later
   step reports the later index, not an earlier one. A recorded-tick mismatch remains the
   existing typed tick mismatch.
5. **Documented prefix delta.** A movie altered at a step **before** the selected anchor
   (re-signed with a valid trailing checksum) is reported as a passing anchored replay at
   the same `until`, because the prefix is not re-executed; a full-range `verify` on the
   same movie fails at the altered prefix index. This negative control proves the
   documented limitation, not a hidden verification claim.
6. **Fallback.** A v1 movie and a v2 movie with zero anchors replay exactly as ADR-0005:
   linear from the embedded checkpoint, identical range validation, prefix/window counts,
   first divergence and default output. A v2 movie with anchors but an empty prefix also
   falls back to the embedded checkpoint.
7. **Invalid ranges before work.** Every ADR-0005 range category (before-start, after-end,
   inverted, unaligned `from`/`until`, empty window) is still rejected before anchor
   selection, checkpoint import, session creation or execution; a rejected range leaves
   the manager, the movie and the file unchanged.
8. **One replay loop.** The anchored path is a start-index parameter of the existing single
   replay loop, not a second loop; `Movie::verify` still delegates the full range; there is
   one checkpoint import and one first-divergence path.
9. **Isolation and determinism.** A valid replay registers exactly one fresh imported
   session (anchor or embedded checkpoint) and touches no pre-existing session, movie or
   file; two identical anchored replays in separate invocations report identical ticks,
   hashes, counts and anchor fields with no carried state and no host-dependent value.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Replay anchor unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt`, `rows/A21-anchor-seek.txt` |
| Replay anchor integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test replay -- --nocapture` | `targets/sessions-replay.stdout.txt`, `rows/A21-anchor-seek.txt` |
| CLI anchored replay | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas` | `cli/play-anchored-*.*`, `cli/results.txt`, `targets/cli-tas.stdout.txt` |
| Two-process seek transcript | record in P1; anchored `play`/`verify --from/--until` in P2/P3; full verify on the tampered prefix; compare reports | `cli/play-anchored-*.*`, `cli/verify-anchored-*.*`, `cli/verify-full-v2.*`, `cli/results.txt` |
| One-loop structural audit | `rg -n "fn replay_validated\|ReplayOutcome \{|create_session_from_snapshot_bytes\|AnchorImport" crates/chronofork-sessions/src/replay.rs` | `audit/A21-structural-audit.txt` |

**Observed result:** PASS. CLI runs select the nearest anchor at or before the prefix (from exactly on an anchor; between anchors; no anchor at `from == start_tick`; the final-step anchor is not selectable), report the same final tick/hash as the linear boundary (`prefix_steps=3`, `window_steps=4`, `replayed_steps=4` in the sample), skip the pre-anchor prefix by contract (negative control passes anchored, fails full verify at the altered prefix step), and fall back for v1 and anchor-less v2 movies. Import goes only through the existing snapshot path, invalid ranges are rejected before import, exactly one replay loop remains, and reports are deterministic and isolated. Evidence: `rows/A21-anchor-seek.txt`, `cli/play-anchored-*`, `cli/verify-full-v2.*`, `cli/results.txt`, `audit/A21-structural-audit.txt`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A22 — Schema-2 multi-device input and synthetic-backend consumption

**Passes only when** all of the following are demonstrated:

1. **v1 surface preserved.** `InputFrame::new(buttons)` has its existing signature and
   sets port 0 only; `buttons()` returns port 0; `InputFrame::NEUTRAL` and `Default` are
   all-zero; `InputFrame::new(0).buttons() == 0`; existing uses compile with no call-site
   change. A schema-1 movie decodes into frames whose ports 1–3, axes and keyboard are
   neutral.
2. **Multi-device surface.** A multi-device constructor and read-only accessors round-trip
   four 32-bit ports, four signed 16-bit axes (including `-32768`, `-1`, `0`, `1` and
   `32767`) and a 64-bit keyboard bitmap (including bit 0 and bit 63); equality, `Hash` and
   `Copy` behavior are deterministic and value-based; neutral equality holds.
3. **v1 semantic equivalence.** For the default program and the existing custom-program
   fixtures, every v1 input-mask exercise makes a v1 frame produce exactly the same tick,
   state hash and snapshot payload as the pre-change build. The synthetic state body still
   contains exactly `pc`, `acc`, `tick`, `steps`, `halted` and `ram` in the existing order;
   `PAYLOAD_FORMAT_VERSION` remains 1; the default program and default config hash are
   unchanged.
4. **Deterministic device consumption.** Each new device class has a deterministic,
   documented effect in the synthetic `APPLY_INPUT`: non-neutral ports 1–3 add their low
   byte and store at the documented RAM addresses; non-neutral signed axes add their
   two's-complement 16-bit value and store at the documented addresses; a non-neutral
   keyboard bitmap adds each of its eight bytes in order and stores them at the documented
   addresses. Repeating the same frame sequence twice produces equal outcomes; neutral
   fields have no state effect, so a v1 frame remains exactly the old path.
5. **Multi-device round-trip.** A v2 movie recorded with non-neutral ports, axes and
   keyboard verifies every recorded tick and state hash; changing any one device field
   changes the recorded trajectory deterministically; the same fields replay identically
   in a fresh session and in a new process.
6. **Unsupported backends.** A backend that does not consume the new fields (including the
   Mega Drive slice, which ignores input) produces well-formed records whose fields have no
   effect; `backend describe` and the docs do not advertise multi-device support; no new
   capability bit or trait method is added.
7. **No equality impact.** `compare_movies` does not use input masks, device fields or
   schema version as equality fields; two v2 movies whose `(outcome_tick, state_hash)`
   sequences are equal compare identical even when their device fields differ; a v1 and v2
   movie with equal trajectories compare identical.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Core input unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-core` | `targets/core.stdout.txt`, `rows/A22-input-schema.txt` |
| Synthetic backend unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-test-backend` | `targets/test-backend.stdout.txt`, `rows/A22-input-schema.txt` |
| Movie v2 multi-device integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test movie_v2` | `targets/sessions-movie_v2.stdout.txt`, `cli/v2-*.*` |
| Replay multi-device integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test replay` | `targets/sessions-replay.stdout.txt`, `cli/v2-*.*` |
| CLI multi-device round-trip | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas` | `cli/v2-*.*`, `cli/compare-*.*`, `targets/cli-tas.stdout.txt` |
| Payload/layout audit | inspect the synthetic state-body writer, `PAYLOAD_FORMAT_VERSION` and `DEFAULT_PROGRAM` before/after | `audit/A22-payload-layout.txt` |

**Observed result:** PASS. `InputFrame::new`/`buttons()`/`NEUTRAL`/`Default` keep v1 behavior; a golden pre-schema-2 state hash and 0..=255 v1-mask equivalence prove v1 frames are unchanged; ports 1–3, axes (two's complement) and keyboard bytes are consumed in fixed ascending order with fixed addresses; the payload format version and state-body layout are unchanged; multi-device record/replay round-trips and changes trajectory when a device field changes; `backend describe` advertises no new capability; comparison ignores device fields and schema. Evidence: `rows/A22-input-schema.txt`, `targets/core.*`, `targets/test-backend.*`, `cli/v2-*`, `audit/A22-payload-layout.txt`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A23 — v1 non-regression, one codec and no weakened tests

**Passes only when** the increment is proven additive:

1. **v1 bytes unchanged.** `MOVIE_FORMAT_VERSION` and `MOVIE_INPUT_SCHEMA_VERSION` remain
   `1` for the v1 path; the v1 container has no new field and no anchor table; a v1 movie
   recorded before the increment decodes, verifies and re-encodes byte-identically; a
   recording with no v2 flag is byte-identical to the pre-change baseline movie; the
   existing rejection matrix (magic, versions, checksum, truncation, trailing bytes,
   limits, metadata, tick/count, embedded checkpoint) still produces its typed categories.
2. **v1 behavior unchanged.** Default and ranged `tas play`/`verify`, `tas compare` and
   default `tas record` produce stdout byte-identical to the captured baseline (equal
   SHA-256), exit codes are unchanged, and the existing divergence/tick-mismatch behavior
   is unchanged. A v1 movie replayed through the v2-aware build reports the same outputs
   and no anchor fields.
3. **Existing rows.** M3-A1–A17 all still pass, and the M2 acceptance target matrix keeps
   its counts and assertions.
4. **One codec and one replay loop.** There is exactly one public movie reader/writer
   family in `chronofork_sessions::movie` (one v1 branch, one v2 branch, shared codec and
   shared snapshot parser), one embedded-checkpoint decode path, one anchor snapshot
   decode path using only `Snapshot::from_bytes`, one `validate_envelope` predicate and one
   replay loop. No second movie module, parser, checksum rule, compatibility predicate or
   comparison implementation appears.
5. **No weakening.** Every pre-existing test target in the workspace is at least at its
   baseline count; no test, assertion, lint or allow is removed or relaxed;
   `git diff -U0` adds no `#[allow]`, `#[ignore]`, `unsafe`, `todo!` or `unimplemented!`;
   no new dependency appears and `Cargo.toml`/`Cargo.lock` are unchanged;
   `cargo tree -d` is unchanged.
6. **No emulation change.** For existing v1 inputs the synthetic backend produces the same
   state body, payload bytes, state hashes, tick sequence and scheduler behavior; no Mega
   Drive, storage, backend-trait or capability change is present.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Existing v1 movie targets | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test movie` and `--release --test movie` | `targets/*`, `rows/A23-v1-regression.txt` |
| Existing v1 replay targets | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test replay` and `--release --test replay` | `targets/*`, `rows/A23-v1-regression.txt` |
| Existing CLI matrix | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli` | `targets/cli-*`, `rows/A23-v1-regression.txt` |
| M2 target matrix | the M2 commands from the [M3 plan §4](m3-tas-replay-acceptance.md#4-executed-test-targets-m3-t4) plus `cargo test -p chronofork-cli --test session_storage` | `target-matrix.txt`, `targets/*` |
| Structural/dependency audit | reader/writer, parser, predicate, replay-loop and comparison `rg` scans; `git diff -- Cargo.toml Cargo.lock`; `cargo tree -d`; suppression scan | `audit/A23-structural-audit.txt`, `probes/tree/` |
| v1 transcript equality | record/play/verify/compare and ranged replay against the baseline transcript; SHA-256 and `cmp` | `probes/regression-results.txt`, `cli/`, `rows/A23-v1-regression.txt` |

**Observed result:** PASS. Fresh v1 recordings and pre-change fixtures are byte-identical to the baseline (`probe-a`, `probe-b`, `probe-a2` hashes), every v1 CLI transcript matches its baseline SHA-256, every pre-existing target is at or above its baseline (the only new target is `movie_v2`), one reader/writer and one snapshot parser/predicate/replay loop remain, no `#[allow]`/`#[ignore]`/`unsafe`/`todo!` was added, manifests and dependencies are unchanged, and Mega Drive/backend-api have zero diff. Evidence: `rows/A23-v1-regression.txt`, `probes/regression-results.txt`, `target-matrix.txt`, `audit/A23-structural-audit.txt`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A24 — CLI v2 record/play/verify/compare process contract

**Passes only when** the CLI implements the surface in §6 with the exact flags,
defaults, `key=value` blocks, stderr diagnostics and exit codes:

1. **Record flags.** `record` accepts `--ports` with exactly three decimal `u32` values,
   `--axes` with exactly four decimal signed 16-bit values, `--keys` with a decimal `u64`,
   and `--anchor-interval` with a decimal value; duplicate, missing, malformed,
   empty-field, out-of-range, extra-field and incompatible combinations are typed CLI
   errors (exit 1, no panic, empty stdout).
2. **v2 selection.** No v2 flag (and `--anchor-interval 0` alone) uses the v1 writer and
   produces byte-identical v1 output; any multi-device flag or `--anchor-interval N > 0`
   uses the v2 writer. A v2 request above `MAX_V2_STEPS`, `--anchor-interval N > 0` with
   `--steps 0` or `N > --steps`, or a predicted anchor count above `MAX_ANCHORS` fails
   before execution with a typed message naming the limit.
3. **Record output.** The `record=movie` block keeps its existing fields and order;
   `movie_version=2` and `input_schema=2` appear for v2; `anchors=<count>` appears after
   `movie_sha256` only when the movie has at least one anchor; `movie_sha256` equals the
   SHA-256 of the exact file bytes; a failed record preserves an existing movie file.
4. **Play/verify output.** Default (no range flags), all v1/anchor-less ranged output, and
   a ranged replay that falls back to the embedded checkpoint are byte-identical to the
   baseline and add no anchor field. An anchored ranged block adds, after the existing
   fields, `anchored=1`, `anchor_step=<k>`, `anchor_tick=<tick>` and `prefix_verified=0`;
   `replayed_steps` is the executed count. `verify` adds `verified=1` as today. An anchored
   divergence uses the existing divergence block and exit 1; a recorded-tick mismatch
   remains stderr-only.
5. **Compare.** `tas compare --movie <FILE> --against <FILE>` accepts v1 and v2 movies,
   prints the existing `record=comparison` block and verdict shapes, exits 0 for identical
   and 1 for every mismatch, and never uses format version, anchors, input schema or
   metadata as equality fields. A v1 versus v2 pair with equal checkpoint identity and
   equal recorded expectations prints `identical=1`.
6. **Rejections and streams.** Missing/unreadable/directory/oversized/altered magic/bad
   version/bad schema/truncated/checksum-bad/anchor-corrupt v2 files exit 1 for `play`,
   `verify` and `compare` with typed `chronofork: <error>` stderr and no partial stdout
   block. Valid stdout carries only `key=value` data; no timestamp, host name, path,
   network access or telemetry appears; exit is 0 on success and 1 on any typed failure.
7. **Two-process boundary.** Record in P1; anchored ranged `play`/`verify` and `compare` in
   separate P2/P3/P4 processes using only movie files; all reproduce the expected reports
   with no shared handles, no storage root and no carried state; read-only commands leave
   the file SHA-256 unchanged.
8. **Help and honesty.** `--help` documents `--ports`, `--axes`, `--keys`,
   `--anchor-interval`, numeric formats, the v2 limits, the nearest-anchor selection rule,
   the anchored prefix limitation (`prefix_verified=0`, not a full verification) and the
   deferred items: in-process rewind/state cache and interactive stepping, rerecord/branch
   editing, real-hardware input TAS, import/export adapters and HTTP/MCP/GUI/daemon. Help
   states the synthetic-backend scope and does not claim M3 completion.
9. **Existing CLI.** Every pre-existing command and test is unchanged and green;
   `cargo test -p chronofork-cli` and `cargo test -p chronofork-cli --test tas` pass.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| CLI TAS process target | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `rows/A24-cli-transcript.txt`, `cli/` |
| Existing CLI regression | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli` | `targets/cli-*`, `cli/` |
| Manual process transcript | v2 record P1; anchored play/verify P2/P3; compare P4; parse blocks/exit codes/SHA-256 | `rows/A24-cli-transcript.txt`, `cli/results.txt`, `cli/*.command.txt` |
| Help capture | `cargo run -p chronofork-cli -- tas --help` and subcommand help | `cli/help-tas.*`, `cli/help-tas-record.*`, `cli/help-tas-play.*`, `cli/help-tas-verify.*`, `cli/help-tas-compare.*` |

**Observed result:** PASS. The CLI process suite exercises strict `--ports`/`--axes`/`--keys`/`--anchor-interval` parsing and typed rejections, the v2 selection rule (plain record and `--anchor-interval 0` stay byte-identical v1), record/play/verify/compare blocks, anchored fields only when an anchor is used, empty stdout on failures, separate-process replay from movie files only, read-only non-mutation and honest help. Evidence: `rows/A24-cli-transcript.txt`, `cli/results.txt`, `cli/help-tas.*`, `cli/*.command.txt`, `targets/cli-tas.*`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A25 — Determinism, no host time and debug/release equality

**Passes only when** all of the following are demonstrated:

1. **Cross-run equality.** Recording the same checkpoint bytes, metadata, multi-device
   frames and anchor interval twice in separate processes (separated in time) produces
   byte-identical v2 movies and equal SHA-256; two library recordings from identical
   manager/session identity conditions are byte-identical; decoding and re-encoding a v2
   movie reproduces identical bytes.
2. **Debug/release equality.** Debug and release builds produce equal v2 movie bytes,
   equal anchor tables and equal replay reports for the same fixtures (compared across
   recorded artifacts, not inside one process).
3. **No host time in bytes or reports.** No timestamp, process id, host name, path, nonce
   or entropy appears in canonical v2 bytes; the movie module uses no clock, environment
   or filesystem metadata API; replay reports and valid stdout contain no host-dependent
   value; a file's modification time is never read for identity or ordering.
4. **Metadata/anchor independence.** A metadata-only change changes bytes but not
   start/end ticks, step records, final state hash or replay result; an anchor-interval-only
   change changes the anchor table and bytes but not the step records or an anchor-less
   replay result; a clock change between runs changes no byte.
5. **Report determinism.** Two identical anchored replays, in separate invocations and
   processes, report identical ticks, hashes, counts, anchor fields and `prefix_verified`;
   no timing value is used as a pass/fail gate.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Movie v2 determinism tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib` and `--test movie_v2` | `targets/sessions-*.stdout.txt`, `rows/A25-determinism.txt` |
| Release-mode equality | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --release --test movie_v2` and `--release --test replay` | `targets/release-*`, `rows/A25-determinism.txt` |
| Two-process recordings | record twice with the same flags in separate processes; compare bytes and SHA-256; inspect parsed fields | `rows/A25-determinism.txt`, `cli/movies/`, `cli/v2-record-*.*`, `cli/results.txt` |
| Source scan | inspect the movie module for clock/environment/filesystem use | `audit/A25-host-time-audit.txt` |

**Observed result:** PASS. Two separate CLI processes produce byte-identical v2 movies and reports; a release build produces identical bytes and an identical anchored block; library determinism and round-trip tests pass; no clock/environment/filesystem-metadata/entropy API appears in the movie module; metadata-only and anchor-interval-only changes do not change the step records or final hash; repeated anchored reports are identical. Evidence: `rows/A25-determinism.txt`, `cli/movies/`, `cli/v2-record-*`, `cli/play-anchored-*`, `targets/release-*`, `audit/A25-host-time-audit.txt`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A26 — Full gate, demos, M2/M1 and v1-M3 regression

**Passes only when** all of the following are recorded on the final tree:

1. **Full gate.** `./scripts/check.sh` and the individual `cargo fmt --all -- --check`,
   `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
   and `git diff --check` all exit 0.
2. **Demos.** Both `cargo run -p chronofork-cli -- demo` and
   `cargo run -p chronofork-cli -- megadrive demo` run three times with exit 0,
   empty program stderr and stdout byte-identical to the captured pre-change baseline
   (`cmp` exit 0, equal SHA-256).
3. **No coverage loss.** Every pre-existing test target is at least at its captured
   baseline count; any increase is explained by the new v2 tests and no assertion was
   removed or relaxed. The M2 acceptance target matrix and M3-A1–A17 keep their counts and
   results.
4. **Serialized state and dependencies.** The serialized `session save/load` probe on two
   independent roots matches the baseline bytes, tick and state hash;
   `Cargo.toml`/`Cargo.lock` and `cargo tree -d` are unchanged; no new crate, dependency,
   `unsafe`, `#[allow]` or `#[ignore]` appears.
5. **Scheduler/Mega Drive untouched.** The Mega Drive slice, its payload, state hashes and
   scheduler behavior are unchanged, and the M1 oracle differential evidence still matches;
   no backend-trait or capability change is present.
6. **Final-tree binding.** The verifier records HEAD, branch, `git status --short
   --branch`, the changed-path set and artifact SHA-256s, and the PASS is bound to that
   tree. A later edit to an implementation path invalidates the claim until re-verified.

**Executed targets (t7):**

| Target | Command | Evidence |
|---|---|---|
| Full gate | the gate commands above | `gate/` |
| Target matrix | every target in §4 and the M2 matrix, with baseline comparison | `target-matrix.txt`, `targets/*` |
| Demos | both demos, three runs each, `cmp` and SHA-256 against baseline | `demos/`, `demos/demo-runs.txt` |
| Serialized-state probe | baseline `session save/load` probe on two roots | `probes/serialized-state.txt`, `probes/regression-results.txt` |
| Dependency/manifest audit | `git diff -- Cargo.toml Cargo.lock`; `cargo tree -d`; suppression scan | `audit/A23-structural-audit.txt`, `probes/tree/` |

**Observed result:** PASS. Every gate command (fmt, check, clippy `-D warnings`, workspace tests, rustdoc `-D warnings`, `./scripts/check.sh`, `git diff --check`) exits 0; both demos run three times with empty program stderr and stdout byte-identical to the pre-change baseline; every target is at or above baseline; the serialized two-root state probe, v1 movie/transcript regression, `cargo tree -d` and manifest hashes match; Mega Drive/backend-api are untouched; the verdict is bound to the recorded tree. Evidence: `rows/A26-regression.txt`, `gate/`, `demos/`, `target-matrix.txt`, `probes/`, `audit/A23-structural-audit.txt`, `tree/`.

**Status:** executed - passed (t7, 2026-11-15).

### M3-A27 — Scope honesty and the explicit deferred list

**Passes only when** the durable docs, CLI help and capability surfaces state exactly what
is implemented and what is not:

1. **Matches evidence.** Every v2 capability claim in ADR-0006, this plan, the execution
   model, requirements, milestones, roadmap and README cites an executed row/artifact; a
   claim without evidence is a finding.
2. **No false capability.** `backend describe`, `tas --help` and the docs do not advertise
   rewind caching, interactive stepping, rerecord/branch editing, real-hardware input TAS,
   import/export adapters, HTTP/MCP/GUI/daemon, O(1) seek or cross-backend portability.
   Anchors are described as a bounded seek optimization with the pre-anchor prefix
   limitation; the synthetic backend is described as a fixture.
3. **Deferred agreement.** The deferred lists in ADR-0006 §13, this plan §10, the
   requirements, milestones and roadmap agree; in-process rewind/state cache and
   interactive stepping, rerecord/branch editing and real-hardware input TAS remain
   explicitly unimplemented.
4. **Milestone state.** No M3 item is checked unless its mapped rows are executed and
   passed; M3 is not claimed complete; only the v2 capabilities actually verified may be
   described as implemented.
5. **Requirement honesty.** `FR-TAS-001` remains **Partial** because rewind/rerecord and
   real-hardware input TAS remain unimplemented; `FR-TAS-003` and `FR-TAS-004` are
   **Implemented only for the deterministic synthetic backend** after their rows passed;
   `FR-MD-010` remains **Planned** for real hardware. No requirement or capability is
   widened beyond the verified synthetic-backend scope.
6. **No migration claim.** The docs state that v1 remains readable, v2 is reject-only and
   there is no migration, rewrite-on-open or silent upgrade.

**Executed targets (t7):**

| Check | Command | Evidence |
|---|---|---|
| Docs vs capability | read ADR-0006, this plan, requirements, milestones, roadmap, README and CLI help; `rg -n "anchor\|v2\|multi-device\|ports\|analog\|keyboard" docs crates/chronofork-cli/src` | `rows/A27-scope-audit.txt`, `audit/A27-scope-audit.txt` |
| Help honesty | `cargo run -p chronofork-cli -- tas --help`, subcommand help and `backend describe` | `cli/help-tas.*`, `cli/backend-describe.*`, `audit/A27-scope-audit.txt` |
| Milestone/requirement state | inspect that only executed rows are checked and M3 is not complete | `audit/A27-scope-audit.txt`, [milestones](../roadmap/milestones.md) |

**Observed result:** PASS at verification time. `tas --help` and the durable docs matched the executed surface; `backend describe` advertised no v2/anchor/multi-device capability; the deferred list (in-process rewind/state cache and interactive stepping, rerecord/branch editing, real-hardware input TAS, import/export, HTTP/MCP/GUI/daemon) remained explicitly unimplemented; no M3 item was checked and the v2 requirements remained Planned at verification; no migration claim was made. O1 (non-blocking) noted that this plan and ADR-0006 still carried pre-verification status text; the t8 documentation promotion updates that status from the t7 evidence, with no capability or criterion change. Evidence: `rows/A27-scope-audit.txt`, `audit/A27-scope-audit.txt`, `cli/help-tas.stdout.txt`, `cli/backend-describe.stdout.txt`.

**Status:** executed - passed (t7, 2026-11-15).

## 6. CLI surface (implemented and verified)

The surface below is the accepted contract shape from
[ADR-0006 §10](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md#10-cli-surface-accepted).
It is deliberately small, std-only and line-oriented. Command and flag spelling may be
refined only with an ADR/plan amendment; the semantics, defaults, exit codes and stream
split are the contract. The fields below are the shapes observed in the t7 artifacts; see
`rows/A24-cli-transcript.txt` and `cli/results.txt` for the full process suite.

```text
chronofork tas record --movie <FILE> --steps <N> [--input <U32>]
                      [--ports <P1,P2,P3>] [--axes <A0,A1,A2,A3>]
                      [--keys <U64>] [--anchor-interval <N>]
                      [--movie-id <U64>] [--name <NAME>]
                      [--root <DIR> [--session <KEY> [--snapshot <KEY>]]]
chronofork tas play    --movie <FILE> [--from <TICK>] [--until <TICK>]
chronofork tas verify  --movie <FILE> [--from <TICK>] [--until <TICK>]
chronofork tas compare --movie <FILE> --against <FILE>
```

Observed behavior:

- **`--input`** remains port 0; **`--ports`** supplies ports 1–3; **`--axes`** supplies four
  signed axes; **`--keys`** supplies the keyboard bitmap. All are fixed for every recorded
  step, matching the existing fixed-input recording model.
- **`--anchor-interval N`**: `0` (default) means no anchors; `N > 0` records an anchor
  after recorded steps `N - 1, 2N - 1, …`. `N > 0` with `--steps 0` or `N > --steps`, or a
  predicted count above `MAX_ANCHORS`, is a typed failure before execution.
- **v2 selection**: a multi-device flag or `N > 0` selects format 2 / schema 2; otherwise
  the v1 writer and byte-identical v1 output.
- **Record block**: existing fields and order, plus `anchors=<count>` after
  `movie_sha256` only when `count > 0`.
- **Anchored play/verify block**: existing ranged fields plus `anchored=1`,
  `anchor_step`, `anchor_tick` and `prefix_verified=0`; `replayed_steps` is the executed
  count. No anchor fields on the default or anchor-less paths.
- **Compare block**: unchanged verdict shapes and exit codes; v1 and v2 accepted,
  format/anchors/metadata never equality fields.
- **Exit and streams**: 0 success, 1 typed failure or mismatch; stdout data only; stderr
  `chronofork: <error>` only; no timestamp, host path, network access or telemetry.

Default/v1 block shapes remain exactly the ADR-0004/ADR-0005 shapes; the v2 additions
above appear only when a v2 capability is actually used. A v2 `record` with
`movie_version=2` and `input_schema=2` changes only those existing values; it does not
reorder fields.

## 7. Baseline preconditions

These capabilities exist before this increment and are preconditions, not v2 evidence.
The verifier must capture the pre-change baseline under `.research/m3-v2/baseline/`
before any behavior-bearing edit, including per-target counts, demo bytes and the v1
regression fixtures.

| Existing evidence | What it shows | What it does not cover here |
|---|---|---|
| `crates/chronofork-sessions/src/movie.rs` (v1 reader/writer, record, verify) | The v1 container, recorder and full-range verifier exist and are verified (M3-A1–A12) | Any v2 anchor table, schema-2 record or v2 construction/recording path |
| `crates/chronofork-sessions/src/replay.rs` | Ranged replay over an exact tick window, typed validation, one replay loop and comparison exist and are verified (M3-A13–A17) | Anchor selection, anchor import, anchored start index, anchor report fields |
| `crates/chronofork-core/src/input.rs` | A backend-defined `u32` mask with `new`/`buttons`/`NEUTRAL` | Ports 1–3, analog axes, keyboard bitmap or `with_devices` |
| `crates/chronofork-test-backend/src/lib.rs` | Deterministic one-input-per-step execution and `APPLY_INPUT` port-0 behavior | Non-neutral extra ports, axes and keyboard consumption |
| `crates/chronofork-cli/src/tas.rs` | Stable record/play/verify/compare blocks, exit codes and process behavior | `--ports`/`--axes`/`--keys`/`--anchor-interval`, anchored output fields, v2 help |
| `SessionManager::create_session_from_snapshot_bytes`, `snapshot`, `snapshot_record`, `snapshot_bytes` | Canonical checkpoint import, capture and serialization in one process | Anchor capture in the recording session and anchor import selection |
| M2 storage and target matrix | Durable checkpoints, metadata, restart, bounds, rejection | Any v2 movie or anchor behavior |
| `crates/chronofork-megadrive/src/backend.rs` | The minimal Mega Drive slice executes deterministically | Input is ignored (`_input`), so it is not an input-TAS backend |
| [Research: TAS and debugging](../research/tas-and-debugging.md) | Upstream evidence that seek is replay from a cached state | A ChronoFork implementation; research is context only |

As of the t7 record the previously absent pieces exist:
`crates/chronofork-sessions/src/movie.rs` (v2 branch, `MovieAnchor`,
`Movie::from_v2_parts`/`record_v2`), `crates/chronofork-sessions/src/replay.rs`
(anchor seek and report fields), `crates/chronofork-core/src/input.rs` (schema-2
`InputFrame`), `crates/chronofork-test-backend/src/lib.rs` (device consumption),
`crates/chronofork-cli/src/tas.rs` (v2 flags/blocks/help),
`crates/chronofork-sessions/tests/movie_v2.rs` and the extended replay/CLI targets. They
are v2 evidence recorded in §4, §5 and §9, not baseline preconditions.

## 8. Traceability

| Criterion | Requirements | ADR-0006 sections |
|---|---|---|
| M3-A18 v2 container, bytes, limits, deterministic anchors | FR-TAS-002, FR-TAS-003, FR-EXEC-004, NFR-DET-003, NFR-QUAL-007 | §2, §3, §5, §9 |
| M3-A19 bounded v2 decode and typed rejection | FR-TAS-002, FR-TAS-003, NFR-DUR-003, NFR-DUR-004, NFR-STOR-002, NFR-QUAL-007, AS-12 | §2, §4 |
| M3-A20 anchor capture and binding | FR-TAS-003, FR-SNAPSHOT-001, FR-SNAPSHOT-004, NFR-DET-003 | §4, §5, §9 |
| M3-A21 anchor-based seek, fallback and divergence | FR-TAS-001, FR-TAS-003, FR-EXEC-005, NFR-DET-003, NFR-DET-006 | §5, §6 |
| M3-A22 schema-2 multi-device input and backend consumption | FR-EXEC-002, FR-TAS-004, FR-MD-010 (still Planned), NFR-DET-003 | §7, §8 |
| M3-A23 v1 non-regression, one codec and no weak tests | FR-TAS-002, NFR-QUAL-001, NFR-QUAL-002, NFR-QUAL-004, NFR-QUAL-006, AS-01, AS-02, AS-11, AS-15 | §1, §3, §11 |
| M3-A24 CLI v2 process contract | FR-API-001, FR-TAS-003, FR-TAS-004, NFR-QUAL-004 | §10 |
| M3-A25 determinism and no host time | FR-EXEC-001, NFR-DET-001, NFR-DET-003, NFR-DUR-005 | §2, §9 |
| M3-A26 gate, demos and M1/M2/v1 regression | NFR-QUAL-001, NFR-QUAL-005, M2-A1–A11, M3-A1–A17 | §12 |
| M3-A27 scope honesty and deferred list | NFR-QUAL-005, NFR-DOC-001, NFR-DOC-003 | §13 |

At the t7 execution record, `FR-TAS-001` remains **Partial** (rewind/rerecord/hardware
scope still deferred), `FR-TAS-002` is **Implemented** (v1 and v2), and `FR-TAS-003` and
`FR-TAS-004` are **Implemented for the deterministic synthetic backend** with the rows
above as evidence. `FR-MD-010` (real Mega Drive controller input) remains **Planned**.
The requirements and the verifier's executed rows are the authority; M3 remains
incomplete.

## 9. Execution record (2026-11-15)

**Executed by the independent t7 verification: PASS.** Tree under test: HEAD
`9643d74fd37097ebe487a398fe0a0259a3b3c7aa` (branch `agent/m3-complete`) plus the
increment-3 working-tree paths (implementation, tests and docs); `tree/state-comparison.txt`
records that the implementation/doc path set was unchanged by verification and that the
verifier's only tracked edit was the dated [quality gates](quality-gates.md) block. No
commit, push or tag was made. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

1. **Full gate:** `cargo fmt --all -- --check`, `cargo check --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `./scripts/check.sh` and
   `git diff --check` all exited 0; per-command logs are in `gate/`.
2. **Target matrix:** every target in §4 exited 0 and is at or above the pre-change
   baseline; the only new target is `movie_v2`. Per-target counts and the baseline
   comparison are in `target-matrix.txt`; release-mode lib/movie/movie_v2/replay targets
   also passed.
3. **Demos and probes:** both demos ran three times with empty program stderr and stdout
   byte-identical to the pre-change baseline (`demos/`); the serialized two-root state
   probe, v1 movie/transcript regression and manifest hashes match the baseline
   (`probes/`); `cargo tree -d` is clean and `Cargo.toml`/`Cargo.lock` are unchanged.
4. **Acceptance rows:** M3-A18–A27 all executed and passed; the per-row commands, observed
   values and artifacts are in §5 and `rows/`. No criterion failed and no test, lint or
   assertion was weakened; the new v2 tests add coverage and none was removed.
5. **Audits:** no added `#[allow]`/`#[ignore]`/`unsafe`/`todo!`/`unimplemented!`; one public
   reader/writer, one snapshot parser, one compatibility predicate and one replay loop;
   Mega Drive and backend-api have zero diff; payload version and golden v1 state hashes
   are unchanged; no host time/entropy in movie/replay code (`audit/`).
6. **Scope:** help and durable docs matched the verified surface; the deferred list
   remained explicitly unimplemented; no M3 item was checked and the v2 requirements
   remained Planned at verification; M3 is not complete (`audit/A27-scope-audit.txt`).
7. **Evidence:** `.research/m3-v2/verification/` (`SUMMARY.txt`, `target-matrix.txt`,
   `rows/`, `audit/`, `targets/`, `cli/`, `demos/`, `probes/`, `gate/`, `tree/`,
   `MANIFEST.sha256`); the dated gate-result block is in
   [quality gates](quality-gates.md). The verifier's verdict is **PASS**.

Non-blocking observations: **O1** — this plan and ADR-0006 carried pre-verification status
text at verification time; the t8 documentation promotion updates that status from the
evidence without changing a criterion or capability claim. **O2** — anchored ranged replay
intentionally does not re-execute the pre-anchor prefix and reports `prefix_verified=0`;
full-range `verify` remains complete. **O3** — the decode taxonomy stays checksum-first as
in v1, so a raw byte flip is a checksum mismatch and later structural rejections are
re-signed. **O4** — `.cfmeta` files embed a wall-clock field, so canonical checkpoint/store/
session-key bytes and state hashes, not raw metadata hashes, are the stable comparison.

This was an uncommitted working tree; the evidence binds that tree, not a commit. The t8
promotion edits only documentation status/evidence text.

## 10. Out of scope and stated limitations

This plan does not implement or claim:

- **In-process rewind/state cache and interactive stepping.** No ring buffer, cached
  state, pause/resume loop, timeline UI or state carried between invocations.
- **Rerecord and branch editing.** No in-place movie edit, rerecord counter or parent
  resolution; `parent_movie_id` remains passive metadata.
- **Real-hardware input TAS.** The Mega Drive `step` ignores input; recording it yields
  inert fields and is not hardware input TAS.
- **Import/export adapters.** BizHawk BK2 and other TAS formats are future work.
- **Checkpoint-reference movies or sidecar anchors.** v2 embeds its checkpoints and
  anchors in one file.
- **O(1)/random-access seek.** Anchors bound the replayed distance; they do not eliminate
  it, and full verification still requires a full-range verify.
- **Full pre-anchor prefix verification under an anchor.** Anchored ranged replay verifies
  the suffix from the anchor; the pre-anchor prefix is not re-simulated, and full-range
  `verify` remains the complete check.
- **Per-step/scripted input, device event timing, per-scanline input.** Schema 2 is one
  fixed frame per step.
- **Compression, encryption, authentication, deduplication, automatic retention.** Movie
  files can be large and contain emulated memory.
- **HTTP/MCP/GUI/daemon and Stage 6 interfaces.** All operations are local, synchronous
  and std-only.
- **Cross-backend or cross-platform runtime portability.** A v2 movie from another backend
  version or configuration is rejected at replay import; Windows/macOS runtime behavior is
  not claimed.
- **Any performance or capacity target.** Record sizes and seek cost are format/algorithm
  costs, not benchmarks.
- **M3 completion.** This is one increment of Stage 3; rewind caching, rerecord/branch
  editing and real-hardware input TAS remain unimplemented.
- **Any implementation claim beyond the t7-verified synthetic-backend scope.** The v2
  container, anchors, seek, multi-device input and CLI are verified only for that backend
  and platform.

## 11. Change control

- A change to a criterion, expected typed category, command, target name, output field or
  artifact path requires an explicit ADR/plan amendment; it must not be made by quietly
  renaming a test or loosening an assertion.
- The names in this document describe the accepted contract shape. Implementation
  spelling may be refined by the approved implementation plan; the semantics, canonical
  bytes, limits, range/anchor boundaries and rejection categories must remain equivalent.
- If a planned command cannot run because its target is absent, the row remains
  `planned - not yet executed`; the gap is reported, not hidden.
- Once a row has executed, its result is append-only evidence linked from
  [quality gates](quality-gates.md); do not rewrite history or relabel a failure.
- A failed row blocks this increment. Never weaken, delete or skip a test to obtain a
  green gate, and never widen a claim beyond the verifier's synthetic-backend scope.
- This plan does not authorize code, a format, a dependency, a commit or a push.
  ADR-0006 is **Accepted** (owner approval 2026-11-15T17:06:17-03:00); the increment is
  implemented and independently verified, and every row is `executed - passed`.
- **2026-11-15 (owner approval).** ADR-0006 moved from Proposed to Accepted on the
  owner's statement "Aprovar ADR-0006 como proposto (Recomendado)" at
  2026-11-15T17:06:17-03:00 (America/Fortaleza), 2026-11-15T20:06:17Z. No criterion,
  command, contract byte, limit or deferred item changed.
- **2026-11-15 (execution promotion, t8).** The t7 independent verification PASS promoted
  every M3-A18–A27 row to `executed - passed`, filled §9 with the tree/commands/artifacts
  and observations O1–O4, and updated the header, target list, traceability and status
  text. No criterion, command, contract byte, limit or deferred item changed; the deferred
  scope in §10 and M3's incomplete status are unchanged.
