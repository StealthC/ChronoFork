# M3 TAS and Deterministic Replay — Executable Acceptance Plan

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** Fase A design accepted; Fase B implemented and independently verified
  (M3-T4 PASS, 2026-11-15). Every acceptance row below is `executed - passed`.
- **Date:** Fase A design 2026-11-15; execution record 2026-11-15
- **Companion contract:** [ADR-0004: TAS movie format and deterministic
  replay](../adr/0004-tas-movie-format.md) (Accepted 2026-11-15; implemented and
  independently verified for the deterministic synthetic backend)
- **Related:** [execution model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md),
  [milestones](../roadmap/milestones.md), [M2 durable-sessions acceptance
  plan](m2-durable-sessions-acceptance.md), [testing strategy](testing-strategy.md),
  [quality gates](quality-gates.md), [anti-slop policy](anti-slop-policy.md).

> **Verification record (2026-11-15).** The independent M3-T4 verifier executed
> M3-A1–A12 on the uncommitted tree at HEAD
> `bcd6c6fae7edd8014aec27e89689933e182c246a` (7 modified tracked paths plus 5
> untracked implementation/contract paths; no commit or push) with
> `CARGO_HOME=/tmp/t7-cargo`, and returned **PASS**. The full gate, M2 regression,
> both demos and the state probe are green; per-row commands, results and artifact
> names are in §4, §5 and §9. Raw evidence is in the ignored
> `.research/m3-tas/verification/` directory (`SUMMARY.txt`, `target-matrix.txt`,
> `A1-*.txt`–`A12-*.txt`, `logs/`, `targets/`, `cli/`, `demos/`, `probes/`,
> `MANIFEST.sha256`); the pre-change baseline is
> `.research/m3-tas/baseline/`. The dated gate-result block is in
> [quality gates](quality-gates.md). M3 is **not complete**: the deferred items in
> §10 remain unimplemented. t4 observations are incorporated: the checksum-first
> truncation taxonomy (O-1, corrected in M3-A2), this documentation promotion (O-2)
> and the unchanged `validate_envelope` coverage (O-3).

> **Owner decisions (approved 2026-11-15).** The project owner approved the staged M3
> team plan on 2026-11-15 (plan approval timestamp 2026-11-15T01:03:50-03:00). The
> approved decisions relevant here: a single self-contained `.cfmovie` with an
> embedded canonical snapshot; per-step expected `(outcome tick, input, state hash)`
> records at 44 bytes per step; input schema v1 as one backend-defined `u32` mask per
> step; reject-only compatibility with no migration; no host timestamps in canonical
> bytes; recording in a separate imported session and replay from the embedded
> checkpoint; the minimal `chronofork tas record|play|verify` CLI; and the deferred
> list in §10. Details are in [ADR-0004 §14](../adr/0004-tas-movie-format.md#14-owner-decisions-approved-2026-11-15).

## 1. Status rules

1. **`planned - not yet executed`** means the criterion has a planned test name,
   command and evidence artifact, but no implementation exists to run.
2. A row becomes **`executed - passed`** only when all of the following are recorded:
   the exact command, exit code, tree state (HEAD plus uncommitted paths), test output
   summary, the ignored evidence artifact path and an independent verifier review. The
   authoritative gate record belongs in [quality gates](quality-gates.md); this
   document links to it and does not copy volatile counts.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error,
   timeout, missing artifact or unverified claim; a failed row blocks M3. A known
   failure is never converted into a skip, an `#[ignore]`, a relaxed assertion or a
   narrower claim after the fact.
4. A test may be committed before the implementation as a red test only if it fails
   for the expected typed reason. A test that cannot run is not acceptance evidence.
5. No M3 behavior may be described as implemented, working, supported or complete on
   the strength of ADR-0004, this plan, a code review, a demo run or a single
   happy-path run.
6. A planned test whose command cannot execute because its crate, module or test
   target does not exist remains `planned - not yet executed`; it does not become
   "passed" because the absent target produced no failure.
7. A row marked passed must cite a t4-verified artifact whose SHA-256 matches the
   recorded manifest. Late or reconstructed evidence does not reopen a failed row.

As of 2026-11-15 every M3 row is `executed - passed` in the M3-T4 record; no row is
`planned` and none is failed.

## 2. Scope, definitions and evidence

- **Movie:** one `.cfmovie` file as specified by
  [ADR-0004 §2](../adr/0004-tas-movie-format.md#2-container-one-canonical-cfmovie-file):
  magic, format and input-schema versions, metadata, embedded canonical snapshot,
  ordered step records and a trailing SHA-256.
- **Record:** capture a source checkpoint and execute the input sequence in a separate
  imported session, producing the movie and its per-step expected outcomes
  (ADR-0004 §9).
- **Playback:** load a movie, import its embedded checkpoint into a fresh session,
  apply the recorded inputs in order and report the final observed tick and hash.
- **Verification:** playback that compares every recorded `(outcome tick, state hash)`
  and reports a typed mismatch; `play` and `verify` share one replay/compare
  implementation (ADR-0004 §9, §11).
- **First divergence:** the first step, in file order, whose observed outcome differs
  from the recorded expectation, reported as `tick`, `input_index`, `expected` and
  `observed` (ADR-0004 §6). Replay never resynchronizes or reports a later mismatch
  instead.
- **Restart:** a second operating-system process replays the movie file after the
  recording process has exited, with no shared handles, no in-memory state and no
  dependence on the original storage root. M3-A5 is specifically this process
  boundary.
- **Durable-seeded record:** a recording whose source session was rehydrated from an
  M2 committed checkpoint under an explicit storage root. The record operation reads
  storage and never writes it; the resulting movie is self-contained.
- **Source session:** the session passed to the recorder. Its backend tick, state hash
  and mutable state must be unchanged by recording and by verifying any movie.
- **Acceptance backend:** the deterministic synthetic backend
  (`chronofork-test-backend`) is the only acceptance backend for this increment. Its
  `APPLY_INPUT` consumes one input per step and one step advances one tick. The Mega
  Drive slice is not an acceptance backend for input TAS because its `step` currently
  ignores input.
- **Platform:** Linux is the acceptance platform, matching the M2 evidence platform.
  Windows and macOS are compile-checked in CI; runtime movie behavior there is not
  claimed.
- **Evidence artifacts:** raw output is stored under the ignored
  `.research/m3-tas/verification/` path: `SUMMARY.txt`, `target-matrix.txt`,
  `A1-*.txt` … `A12-*.txt`, `logs/`, `targets/`, `cli/`, `demos/`, `probes/` and
  `MANIFEST.sha256`. The pre-change baseline is `.research/m3-tas/baseline/` from
  M3-T0. Each artifact records the command, tree state, exit code and the relevant
  stdout/stderr/SHA-256; the manifest binds the artifact set. Artifacts are diagnostic
  evidence, not durable claims; this document summarizes results without volatile
  counts. The dated gate-result block is in [quality gates](quality-gates.md).
- **No host time in canonical data:** movie bytes contain no timestamp, process id,
  host label, nonce, path or entropy (ADR-0004 §3). Comparisons use tick, state hash,
  step index, movie bytes and SHA-256, never wall-clock time or filesystem
  modification time.

## 3. M3 exit criteria map

The milestone subitems are in [milestones](../roadmap/milestones.md). A milestone
item stays unchecked until every criterion mapped to it is `executed - passed`.

| M3 milestone subitem | Acceptance criteria |
|---|---|
| Versioned canonical movie format | M3-A1, M3-A2, M3-A8, M3-A11, M3-A12 |
| Record/playback of a per-tick input sequence on the deterministic synthetic backend | M3-A3, M3-A6, M3-A11 |
| Replay verification with first-divergence reporting | M3-A4 |
| Restart replay from the embedded checkpoint (and record seeded by an M2 durable checkpoint) | M3-A5 |
| Minimal `chronofork tas record\|play\|verify` CLI | M3-A7 |
| M2 regression and no behavior change | M3-A9 |
| Scope honesty and explicit deferred list | M3-A10 |

**This first increment is complete only when** ADR-0004 is accepted, every criterion
in this document is `executed - passed`, the M2 acceptance rows still pass unchanged,
the full repository gate plus both demos are byte-identical to the M3-T0 baseline, and
an independent verifier records a PASS. Those conditions were satisfied by M3-T4 on
2026-11-15 (§9).

**M3 is not complete.** Frame advance, rewind, seek, rerecord, branch editing,
multi-controller/ports/analog/keyboard input, real-hardware input TAS, import/export
adapters, checkpoint-reference movies, HTTP/MCP/GUI and the Stage 6 daemon remain
deferred and unchecked in [milestones](../roadmap/milestones.md).

## 4. Executed test targets (M3-T4)

These targets exist and were executed on the M3-T4 tree with
`CARGO_HOME=/tmp/t7-cargo`. Per-target counts and the M3-T0 comparison are in
`.research/m3-tas/verification/target-matrix.txt`; raw stdout/stderr is in
`targets/` and `logs/`. The target command runs every test in the target; exact counts
stay in the evidence, not in this durable document.

| Target / command | Executed command | Criteria | Result |
|---|---|---|---|
| Movie unit tests in `crates/chronofork-sessions/src/movie.rs` | `cargo test -p chronofork-sessions --lib` | M3-A1, M3-A2, M3-A8, M3-A11, M3-A12 | exit 0 (M3-T4) |
| Movie integration target `crates/chronofork-sessions/tests/movie.rs` | `cargo test -p chronofork-sessions --test movie` | M3-A1, M3-A3, M3-A4, M3-A5 (reopen complement), M3-A6, M3-A11 | exit 0 (M3-T4) |
| Release-mode equality | `cargo test -p chronofork-sessions --release --test movie` | M3-A8 | exit 0 (M3-T4) |
| CLI process target `crates/chronofork-cli/tests/tas.rs` | `cargo test -p chronofork-cli --test tas` | M3-A4, M3-A5, M3-A7, M3-A11 | exit 0 (M3-T4) |
| M2 session targets | `cargo test -p chronofork-sessions --test restart`, `--test metadata`, `--test fork`, `--test rejection`, `--test ownership`, `--test confinement`, `--test bounds`, `--test determinism`, `--test isolation`, `--test crash_recovery`, `--test retention`, and `cargo test -p chronofork-cli --test session_storage` | M3-A9 | every target exit 0; counts equal M3-T0 (M3-T4) |
| Full gate | `./scripts/check.sh`; `cargo fmt --all -- --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | M3-A9, M3-A12 | exit 0 (M3-T4) |
| Demos | `cargo run -p chronofork-cli -- demo` and `cargo run -p chronofork-cli -- megadrive demo`, three runs each, `cmp` and SHA-256 against the M3-T0 baseline | M3-A9 | exit 0; all six runs byte-identical to baseline (M3-T4) |
| Manual transcripts | two-process record/play/verify; M2-seeded record and root deletion; divergence with `-- --nocapture`; CLI rejection matrix | M3-A5, M3-A4, M3-A7 | `A4-*.txt`, `A5-*.txt`, `A7-*.txt` (M3-T4) |

In the sandbox every Cargo command used `CARGO_HOME=/tmp/t7-cargo`; the verifier
records it in `SUMMARY.txt` and the gate logs.

## 5. Acceptance criteria

### M3-A1 — Movie round-trip and canonical deterministic bytes

**Passes only when** a movie encoded from a known checkpoint, metadata and input
sequence decodes to exactly the same movie id, optional name, optional parent id,
start tick, embedded snapshot bytes and ordered step records; the embedded bytes are
exactly the source `Snapshot::to_bytes()` output and are decoded only through
`Snapshot::from_bytes`; recording the same checkpoint bytes, metadata and inputs twice
in independent runs (including one run separated in time) produces byte-identical
movies and equal SHA-256; a zero-step movie is valid with `end_tick == start_tick` and
a final hash equal to the embedded checkpoint state hash; and the trailing SHA-256
covers every preceding byte (flipping any covered byte is rejected). A metadata-only
name or parent change may change the bytes but must not change the start/end ticks,
step count, final state hash or replay result.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| Movie unit tests | `cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt`, `A1-movie-roundtrip.txt` |
| Movie integration test | `cargo test -p chronofork-sessions --test movie` | `targets/sessions-movie.stdout.txt` |

**Observed result:** PASS. `round_trip_preserves_metadata_checkpoint_and_steps`,
`embedded_checkpoint_is_exactly_snapshot_to_bytes`,
`encoding_is_byte_deterministic_across_independent_runs`,
`zero_step_movie_is_valid_and_ends_at_the_checkpoint`,
`checksum_covers_every_preceding_byte` and
`metadata_changes_do_not_change_recorded_execution` passed; manual recordings were
byte-identical twice 1.2 s apart, from two independent roots and in debug/release.
Evidence: `A1-movie-roundtrip.txt`, `targets/sessions-lib.stdout.txt`,
`targets/sessions-movie.stdout.txt`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A2 — Bounded decode and typed rejection matrix

**Passes only when** every case below returns its named typed category, without a
panic, without a partial `Movie`, without unbounded allocation, without executing a
backend and without modifying a valid file or session. The exact error spelling may be
refined in Fase B; the categories and their boundaries are fixed by
[ADR-0004 §7](../adr/0004-tas-movie-format.md#7-decode-order-bounds-and-typed-rejection).

| Mutated input | Required typed result |
|---|---|
| Wrong magic | `BadMagic` |
| Movie format version 0 or 2 | `UnsupportedFormatVersion` with found/supported values |
| Input schema version 0 or 2 | `UnsupportedInputSchema` with found/supported values |
| Any covered byte flipped | `ChecksumMismatch`; no field trusted before the checksum passes |
| Truncation at each boundary: every cut from 0 to the encoded length | Every cut is typed and never panics. A cut below the fixed overhead returns `TooShort`; a raw cut of a longer movie returns `ChecksumMismatch` because the trailing checksum is verified first; a re-signed short body returns `Truncated`. |
| Bytes appended after the checksum, or inserted before it with a recomputed checksum | checksum rejection or `TrailingBytes`; never reinterpretation |
| File larger than `MAX_MOVIE_BYTES` | `FileTooLarge` before reading/allocating the body |
| `step_count > MAX_STEPS` | `TooManySteps` before allocation |
| Declared step-record bytes larger than the remaining body | `Truncated`/`TooManySteps` before `Vec::with_capacity` |
| Declared `snapshot_length` larger than the remaining body | `Truncated` before slicing/allocating |
| Name longer than `MAX_NAME_BYTES`, invalid UTF-8 or a control character | `InvalidName` |
| `movie_id == 0` | `InvalidMovieId` |
| `parent_movie_id == movie_id` | `InvalidParentMovieId` |
| `start_tick != embedded snapshot tick` | `StartTickMismatch` |
| Outcome ticks not strictly increasing, or the first not greater than `start_tick` | `TickSequence` naming the input index |
| Empty or truncated embedded snapshot | typed movie error wrapping `SessionError::InvalidSnapshot` |
| Embedded snapshot with an unsupported outer version | typed movie error wrapping `SessionError::IncompatibleSnapshot` |
| Embedded snapshot with a foreign backend id, backend version, payload format version or configuration hash | movie decodes structurally; replay import fails with a typed `SessionError::IncompatibleSnapshot`; no second predicate is added |
| Corrupt embedded snapshot checksum or trailing bytes | typed movie error wrapping `SessionError::InvalidSnapshot` |
| Arbitrary bytes of length 0 through 2048 | rejected or explicitly valid; never a panic |

A declared huge count or length must be rejected by arithmetic and remaining-byte
checks **before** allocation; a test must demonstrate the rejection on a tiny file
whose declared count/length is far larger than the file. Because the trailing checksum
is verified first, a test for a later structural rejection (magic, versions, metadata,
ticks) must rebuild a valid checksum after the mutation; a raw byte flip is the
checksum case. The embedded snapshot must be parsed by `Snapshot::from_bytes` exactly
once per phase; the movie reader must not contain a second envelope parser or checksum
rule.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| Movie unit rejection tests | `cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt` |
| Movie integration rejection test | `cargo test -p chronofork-sessions --test movie -- --nocapture` | `targets/sessions-movie.stdout.txt`, `A2-rejection-matrix.txt` |
| CLI rejection tests | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `cli/A2-*` |

**Observed result:** PASS. The unit target covers magic/version/schema, every byte
flip, every truncation, trailing bytes, declared-length/count bounds before
allocation, metadata, start-tick binding, tick sequencing, embedded-snapshot errors and
arbitrary bytes without panic; the integration target covers the foreign-checkpoint
replay import; the CLI covers bad magic, versions, schema, start tick, count, embedded
checkpoint, name, checksum and arbitrary rejection with typed stderr and empty stdout.
t4 observation O-1 is reconciled here: a raw truncation of a movie at least 86 bytes
long is rejected as `ChecksumMismatch` because the trailing checksum is verified first;
`TooShort` applies below the fixed overhead and `Truncated` applies to a re-signed
short body. Evidence: `A2-rejection-matrix.txt`,
`targets/{sessions-lib,sessions-movie,cli-tas}.stdout.txt`, `cli/A2-*`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A3 — Record/replay reproduces every recorded tick and state hash

**Passes only when** recording a non-empty input sequence from a fresh deterministic
synthetic session produces a movie whose embedded checkpoint is the source checkpoint
and whose step records equal the outcomes observed during recording; verifying that
movie in a fresh imported session reproduces every recorded outcome tick and state
hash, applies each input exactly once in order, returns a report with the expected
`start_tick`, `end_tick`, `steps`, `final_tick` and `final_state_hash`, and reaches a
final hash equal to the last recorded expected hash. A movie recorded in an earlier
process verifies to the same final tick and hash. A zero-step movie verifies the
embedded checkpoint as the final state.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| Movie integration test | `cargo test -p chronofork-sessions --test movie -- --nocapture` | `targets/sessions-movie.stdout.txt`, `A3-record-replay.txt` |
| CLI process tests | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt` |

**Observed result:** PASS. `record_and_verify_reproduce_every_tick_and_state_hash`
verified every step tick and state hash (nocapture report `start_tick=5 end_tick=17
steps=12 final_tick=17 final_state_hash=05630078…`); a fresh record in P1, playback in
P2 and verification in P3 all reproduced `final_tick=64` and
`final_state_hash=6addcba7…`; a zero-step movie's final hash equalled the embedded
checkpoint hash. Evidence: `A3-record-replay.txt`,
`targets/{sessions-movie,cli-tas}.stdout.txt`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A4 — First divergence at the exact tick and input index

**Passes only when** a valid movie is altered at exactly one chosen step `k` so its
recorded expected state hash is wrong (re-encoded with a valid trailing checksum) and
verification returns the typed divergence with `tick` equal to the recorded outcome
tick of step `k`, `input_index == k`, `expected` equal to the altered expected hash and
`observed` equal to the state hash that the unaltered execution actually produced at
step `k`. No later mismatch may be reported instead, and replay must stop at the first
mismatch. Negative controls must show that (a) altering only step `j > k` reports
`j`, not `k`; (b) an intact movie reports no divergence; and (c) a recorded outcome
tick changed without changing the expected hash reports the typed tick mismatch at
that index. The values must be visible in `-- --nocapture` output and through the CLI
`verify` block, which exits 1 with `diverged=1`, `tick`, `input_index`, `expected` and
`observed`.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| Movie integration divergence test | `cargo test -p chronofork-sessions --test movie -- --nocapture` | `A4-divergence.txt`, `targets/sessions-movie.stdout.txt` |
| CLI divergence test | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `cli/A4-*` |

**Observed result:** PASS. The library test printed `divergence: tick=7 input_index=3
expected=f5bb66f1… observed=0abb66f1…`; the CLI `verify` and `play` both printed
`diverged=1`, `tick=4`, `input_index=3`, the altered expected hash and the observed
hash, then exited 1; altering only a later step (index 10) reported index 10 and not
index 3; an intact movie verified; a recorded-tick mismatch was a stderr-only typed
failure with no divergence block. Evidence: `A4-divergence.txt`,
`targets/{sessions-movie,cli-tas}.stdout.txt`, `cli/A4-*`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A5 — Restart replay in a new process and M2-seeded recording

**Passes only when** all three process paths are demonstrated:

1. **Fresh record, new-process replay.** Process P1 runs
   `chronofork tas record --movie M --steps N --input X` against the fresh synthetic
   session and exits 0; no handle survives. Process P2 runs
   `chronofork tas verify --movie M` using only the movie file (no storage root, no
   shared state) and exits 0 with `verified=1`, the same final tick and the same final
   state hash as the record report. A third process runs `chronofork tas play --movie M`
   with the same result.
2. **M2-seeded record.** Process P1 first publishes an M2 checkpoint under an explicit
   root (`chronofork session save --root R …`), then runs
   `chronofork tas record --root R --steps N --input X --movie M`, which loads the
   committed checkpoint, records from it and never writes storage; the root's
   checkpoint bytes, list output and session/snapshot keys are unchanged by the record
   command. P2 then verifies `M` without the root.
3. **Embedded-checkpoint-only replay.** After recording, the root may be deleted or
   the movie moved to another directory; `tas verify` still succeeds, proving the
   embedded checkpoint is sufficient.

No in-process handle may be shared between the record and replay processes, and the
record process must have exited before the replay process starts.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| CLI process tests | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt` |
| Reopen/restart complement | `cargo test -p chronofork-sessions --test movie` | `targets/sessions-movie.stdout.txt` |
| Manual process transcript | record in P1, play/verify in separate processes; M2 save then record; root file set before/after; root deletion | `A5-restart-transcript.txt` |

**Observed result:** PASS. P1 `tas record` → P2 `tas play` → P3 `tas verify` reproduced
the same `final_tick=64` and `final_state_hash=6addcba7…` with no shared handles; an
M2-seeded record started at tick 5, ended at tick 8 and left the root file set and
`session list` output byte-identical; after deleting the root a new process still
verified the movie. The recorder captures a fresh canonical envelope (snapshot/origin
ids differ from the stored checkpoint file), while the checkpoint tick, state hash and
payload are equal; this is the documented capture semantics, not a second snapshot
format. Evidence: `A5-restart-transcript.txt`,
`targets/{cli-tas,sessions-movie}.stdout.txt`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A6 — Source non-mutation and isolation

**Passes only when** recording from a source session leaves the source backend tick,
state hash, memory and register state unchanged and never steps or restores it;
capturing the checkpoint record through the existing checkpoint path is the only
permitted source-side effect; verifying a movie touches no pre-existing session and
registers only its own fresh imported session; two movies recorded from the same
checkpoint with different inputs verify independently, neither changes the source, the
other movie's bytes or the other's replay result; and a failed record (backend halt or
step error) returns a typed error, leaves the source unchanged and writes no movie.
The imported replay session must not be the source session and must own independent
mutable state.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| Movie integration tests | `cargo test -p chronofork-sessions --test movie -- --nocapture` | `targets/sessions-movie.stdout.txt`, `A6-isolation.txt` |
| CLI process tests | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt` |

**Observed result:** PASS. Source tick and state hash were captured before recording
and asserted unchanged after recording and after verification; a bystander session in
the verifying manager was unchanged; two movies from one checkpoint verified
independently and neither rewrote the other; a halted recording returned a typed
`RecordFailed` and left the source unchanged. In the manual branch run, movies A and B
from the same durable root reproduced their own final hashes, the root file set was
unchanged and movie A's bytes were unchanged after verifying A, B and A again.
Evidence: `A6-isolation.txt`, `targets/{sessions-movie,cli-tas}.stdout.txt`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A7 — CLI record/play/verify process contract

**Passes only when** the CLI implements the minimal surface in §6 with the exact
flags, defaults, `key=value` stdout blocks, stderr diagnostics and exit codes:
`record` writes a canonical movie and prints `record=movie` with `movie_version`,
`input_schema`, `movie_id`, `name`, `start_tick`, `end_tick`, `steps`,
`final_state_hash` and `movie_sha256`; `play` prints `record=playback` and `verify`
prints `record=verification` with `verified=1`; divergence adds `diverged=1`, `tick`,
`input_index`, `expected`, `observed` and exits 1. Unknown/duplicate flags, missing
required flags and malformed numbers are typed CLI errors (exit 1, no panic); the
corruption/rejection cases exit 1 with a typed message and no partial output on
stdout; `movie_sha256` equals the SHA-256 of the exact file bytes; `--help` and the
usage text document the surface; no host timestamp or network access appears; a failed
record does not overwrite an existing movie file; and every pre-existing command and
its tests are unchanged.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| CLI tas tests | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt` |
| Existing CLI regression | `cargo test -p chronofork-cli` | `targets/cli-all.stdout.txt` |
| Manual transcript | separate-process `tas record/play/verify` invocations with exit codes, stdout/stderr split and parsed SHA-256 | `A7-cli-transcript.txt`, `cli/*` |

**Observed result:** PASS. Help documents the `tas` surface and the deliberate
omissions; the success blocks match §6; `movie_sha256` equals the file SHA-256; 12
argument/combination failures, a corrupt file and a missing file exit 1 with typed
stderr and empty stdout; a failed record preserved an existing sentinel movie; and
`cargo test -p chronofork-cli` kept the existing demo, trace and session-storage
targets green. Evidence: `A7-cli-transcript.txt`, `targets/cli-tas.stdout.txt`,
`targets/cli-all.stdout.txt`, `cli/`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A8 — Determinism and no host time in canonical bytes

**Passes only when** recording the same checkpoint bytes, metadata and input sequence
twice, in independent runs separated in time and from independent sessions or roots,
produces byte-identical movies and equal SHA-256; decoding and re-encoding a movie
reproduces identical bytes; no host timestamp, process id, host name, path, nonce or
entropy appears in canonical bytes; a metadata-only name/parent change changes bytes
but not the start/end ticks, step count, final state hash or replay result; debug and
release builds produce equal movie bytes and equal verification reports (compared
across two recorded artifacts, not inside one process); and `play`/`verify` stdout
contains no timestamp or other host-dependent value. The movie module must not use
host time in encoding; a file's filesystem modification time is never read for identity
or ordering.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| Movie unit tests | `cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt` |
| Movie integration tests | `cargo test -p chronofork-sessions --test movie` | `targets/sessions-movie.stdout.txt` |
| Release-mode equality | `cargo test -p chronofork-sessions --release --test movie` | `targets/sessions-movie-release.stdout.txt` |
| Manual determinism transcript | record twice about one second apart; compare SHA-256 and report fields | `A8-determinism-manual.txt` |

**Observed result:** PASS. Two recordings 1.2 s apart were byte-identical
(`b6b23b82…`), two independent roots were byte-identical (`c55f435b…`), and
debug/release were byte-identical. The parser reported every field and no
timestamp/nonce/path; the movie module uses no filesystem, clock or environment API,
and the CLI's process-id use is confined to the sibling temporary file name, never
movie bytes or stdout. A metadata-only name/parent change altered bytes but not
start/end ticks, step count, final hash or replay. Evidence:
`A8-determinism-manual.txt`, `targets/sessions-lib.stdout.txt`,
`targets/sessions-movie.stdout.txt`, `targets/sessions-movie-release.stdout.txt`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A9 — M2 regression, byte-identical demos and the full gate

**Passes only when** every M2 acceptance target still passes and no pre-existing test
target is below its M3-T0 baseline count; both demos run three times with empty
stderr, exit 0 and stdout byte-identical to the M3-T0 baseline (`cmp` exit 0 and equal
SHA-256); the full repository gate (`./scripts/check.sh`, or the individual fmt,
check, clippy `-D warnings`, test and rustdoc commands) exits 0 on the final tree; the
serialized-state probe or equivalent byte check is unchanged; and no existing test,
assertion, lint or allow is weakened. Any intentional behavior change must be declared
with a new baseline under the anti-slop policy; a movie feature is additive and should
not change existing state hashes, payloads, schedules or CLI output.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| M2 target matrix | the M2 session target matrix plus `cargo test -p chronofork-cli --test session_storage` (see §4) | `target-matrix.txt`, `targets/*` |
| Full gate | `./scripts/check.sh` and the individual fmt/check/clippy/test/doc commands | `logs/gate-*`, `logs/*.exit.txt` |
| Demos | `cargo run -p chronofork-cli -- demo` and `cargo run -p chronofork-cli -- megadrive demo`, three runs each, compared with the M3-T0 baseline | `demos/`, `logs/gate-demo*`, `logs/gate-megadrive-demo*` |
| Serialized-state probe | the M3-T0 `session save/load` probe on two independent roots | `probes/`, `A9-regression.txt` |

**Observed result:** PASS. Every gate exited 0, `cargo test --workspace` passed, no
pre-existing target was below its M3-T0 count, all twelve M2 targets passed with
unchanged counts, all six demo runs were byte-identical to baseline with empty program
stderr, and the checkpoint probe matched the baseline SHA-256 and tick/state hash on
both roots. No test, lint or assertion was weakened. Evidence: `A9-regression.txt`,
`target-matrix.txt`, `logs/`, `demos/`, `probes/`; the dated gate block is in
[quality gates](quality-gates.md).

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A10 — Scope honesty and the explicit deferred list

**Passes only when** the durable docs state exactly what is implemented and what is
not: no claim of multi-controller, ports, analog or keyboard input; no claim of frame
advance, rewind or seek; no claim of rerecording or branch editing; no claim of
real-hardware input TAS (the Mega Drive `step` ignores input); no claim of HTTP, MCP,
GUI or a daemon; no claim of import/export adapters; no claim that M3 is complete; no
claim of cross-backend or cross-platform movie portability; and no capability in
`backend describe`, the CLI help or the docs advertises a movie feature that the
acceptance rows did not verify. The deferred list in ADR-0004 §12, this plan §10, the
requirements and the milestones must agree. A row that is not executed stays
`planned`, and a failed row is reported as failed, never relabeled.

**Executed evidence (M3-T4):**

| Check | Command | Evidence |
|---|---|---|
| Docs vs capability | read ADR-0004, this plan, requirements, milestones, README and execution model; `rg -n "TAS\|movie\|tas " docs crates/chronofork-cli/src` | `A10-scope-audit.txt` |
| No false capability | `cargo run -p chronofork-cli -- backend describe`, `chronofork help`, `chronofork tas --help` | `cli/A10-backend-describe.{stdout,stderr}.txt`, `A10-scope-audit.txt` |
| Milestone state | inspect that only executed rows are checked and M3 is not complete | `A10-scope-audit.txt`, [milestones](../roadmap/milestones.md) |

**Observed result:** PASS at verification time. `backend describe` advertised no TAS
or movie capability, the `tas --help` text listed the deferred features as not
implemented, and the durable docs listed the deferred scope while M3 was not claimed
complete. This document's promotion is the t6 follow-up t4 identified; the promoted
status still leaves every deferred item explicitly unimplemented and does not claim M3
complete.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A11 — Branch metadata round-trip and isolation

**Passes only when** a non-zero parent movie id and an optional name round-trip
exactly; absent values encode as `parent_movie_id == 0` and `name_length == 0` and
decode as absent; `parent_movie_id == movie_id` is rejected with a typed error; a
dangling parent id is accepted as metadata, never resolved or repaired; two movies
sharing a parent verify independently and do not alter each other; changing only the
name or parent id changes the bytes but not `start_tick`, `end_tick`, `steps`, the
per-step records or the replay result; and the name/parent are never used as a path
component, identity key or ordering input. The CLI records and prints `--name`
correctly; v1 does not expose a parent flag, and the docs say branch creation/editing
is deferred.

**Executed targets (M3-T4):**

| Target | Command | Evidence |
|---|---|---|
| Movie unit/integration tests | `cargo test -p chronofork-sessions --lib` and `cargo test -p chronofork-sessions --test movie` | `targets/sessions-lib.stdout.txt`, `targets/sessions-movie.stdout.txt`, `A11-branch-metadata.txt` |
| CLI name round-trip | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt` |

**Observed result:** PASS. Unit tests covered name/parent validation, the zero
sentinels and metadata-only byte changes; the integration test round-tripped
`parent=3`/`name=branch-a` and the absent/absent movie, accepted a dangling parent,
verified both independently and left each movie's bytes and the source tick/hash
unchanged; the CLI printed `name=baseline` and exposed no parent flag. Evidence:
`A11-branch-metadata.txt`, `targets/{sessions-lib,sessions-movie,cli-tas}.stdout.txt`.

**Status:** executed - passed (M3-T4, 2026-11-15).

### M3-A12 — One codec, one reader/writer, one compatibility predicate, no new dependencies

**Passes only when** a structural audit shows exactly one movie reader and one movie
writer in `crates/chronofork-sessions/src/movie.rs`, both built on
`chronofork_core::codec::{Reader, Writer}`; the embedded snapshot is encoded only by
`Snapshot::to_bytes` and decoded only by `Snapshot::from_bytes`; backend compatibility
is decided only by the existing `validate_envelope` reached through the existing import
path; `play` and `verify` share one replay/compare implementation; there is no second
snapshot serializer, checksum rule, envelope parser or backend-id/config comparison;
`chronofork-sessions` performs no file I/O in the movie module; no new crate or
dependency appears in `Cargo.toml`/`Cargo.lock` and the `cargo tree -d` output is
unchanged from the M3-T0 baseline; no `unsafe` is added; no new `#[allow]`, `#[ignore]`
or deleted/relaxed test appears; no emulation, backend payload, state hash, scheduler
or existing CLI behavior changes; and the changed paths are limited to the approved
movie module, its crate exports, the justified error type location, the movie
integration test, the CLI `tas` command and its test target.

**Executed evidence (M3-T4):**

| Check | Command | Evidence |
|---|---|---|
| Reader/writer count | `rg -n "struct .*Reader\|struct .*Writer\|fn to_bytes\|fn from_bytes" crates/chronofork-sessions/src` | `A12-structural-audit.txt` |
| One compatibility predicate | `rg -n "fn validate_envelope" crates` (exactly one definition) and review the import path | `A12-structural-audit.txt` |
| Dependency diff | `git diff -- Cargo.toml Cargo.lock`; `cargo tree -d` | `A12-structural-audit.txt`, `logs/cargo-tree-duplicates.*` |
| Lint/allow/unsafe scan | `git diff -U0 \| rg "^\+.*(allow\\(\|#\\[ignore\|unsafe)"` | `A12-structural-audit.txt` |
| One replay comparison | review `movie.rs` and the CLI `tas` path; no second comparison body | `A12-structural-audit.txt` |

**Observed result:** PASS. `movie.rs` has one `from_bytes`, one `to_bytes` and one
validating constructor/record path on the shared codec; the embedded checkpoint is
decoded only by `Snapshot::from_bytes`; `validate_envelope` has exactly one definition
and movie/CLI contain no backend/config comparison; `play` and `verify` share
`replay_movie` over `Movie::verify`; the movie module has no filesystem, network or
clock use; `Cargo.toml`/`Cargo.lock` are unchanged and `cargo tree -d` is empty; no
added `unsafe`, `#[allow]` or `#[ignore]` and no removed assertions; core, backend-api,
test-backend and megadrive have zero changed lines. Evidence:
`A12-structural-audit.txt`, `logs/cargo-tree-duplicates.*`.

**Status:** executed - passed (M3-T4, 2026-11-15).

## 6. Owner-approved minimal CLI surface (implemented and verified)

The surface below is the approved contract shape. It is implemented in
`chronofork-cli` and was exercised end to end by M3-T4. It is deliberately small,
std-only and line-oriented. Command names and flags must not grow beyond this in the
increment; adding a flag needs a contract change.

```text
chronofork tas record --movie <FILE> --steps <N> [--input <U32>]
                      [--movie-id <U64>] [--name <NAME>]
                      [--root <DIR> [--session <KEY> [--snapshot <KEY>]]]
chronofork tas play --movie <FILE>
chronofork tas verify --movie <FILE>
```

Behavior contract:

- **`--movie` is required** for all three subcommands. `record` writes it; `play` and
  `verify` read it. A missing file, a directory, an unreadable file or a file larger
  than `MAX_MOVIE_BYTES` is a typed failure (exit 1), never a panic.
- **`record` without `--root`** creates a fresh deterministic synthetic-backend
  session, applies the fixed `--input` mask for `--steps` steps (`--steps` defaults to
  0, `--input` defaults to 0/neutral), records the canonical movie and prints the
  `record=movie` block. Each such invocation is independent and creates no storage.
- **`record` with `--root`** opens an existing initialized M2 storage root, loads a
  committed checkpoint (the latest snapshot by `creation_sequence` by default, or the
  selected `--session`/`--snapshot`), rehydrates it in memory, records the same way and
  never writes storage. A missing/uninitialized root, unknown session or unknown
  snapshot is a typed failure. `record` never initializes a root.
- **`--movie-id`** is a non-zero logical id, default `1`. **`--name`** is optional
  bounded UTF-8 with no control characters. Parent movie id is a format/library field;
  this CLI does not expose a parent flag in v1, and branch creation/editing is deferred.
- **`play`** loads and replays the movie, prints `record=playback` and exits 0 on a
  clean replay. **`verify`** performs the same replay/compare and prints
  `record=verification` with `verified=1` on success. On divergence both print the
  exact divergence fields and exit 1. They share one implementation.
- **`--steps` and `--input` are record-only.** `play`/`verify` reject them as unknown
  flags for those subcommands (no hidden extension of the recorded sequence).
- **Output** is stdout-only `key=value` blocks separated by one blank line, in the M2
  block style. The first field is `record=movie`, `record=playback` or
  `record=verification`.

`record` block (observed):

```text
record=movie
movie_version=1
input_schema=1
movie_id=1
name=baseline
start_tick=0
end_tick=64
steps=64
final_state_hash=<lowercase hex>
movie_sha256=<lowercase hex of the exact file bytes>
```

`play` block (observed):

```text
record=playback
start_tick=0
end_tick=64
steps=64
final_tick=64
final_state_hash=<lowercase hex>
```

`verify` success block (observed):

```text
record=verification
verified=1
start_tick=0
end_tick=64
steps=64
final_tick=64
final_state_hash=<lowercase hex>
```

Divergence addition (observed on `play` or `verify`):

```text
diverged=1
tick=<recorded expected outcome tick>
input_index=<zero-based step index>
expected=<recorded expected state hash>
observed=<actual state hash>
```

`expected` and `observed` are lowercase hexadecimal. A missing/absent name prints
`name=` (empty). There is no timestamp, host name, path or JSON field in valid
output. A recorded-tick mismatch is a typed replay failure reported on stderr with the
step index and the expected/observed ticks and exit 1; the `diverged=1` block is
specifically a state-hash divergence.

Exit and stream contract: `0` on success and `1` on any typed failure; stdout carries
only the data format; stderr carries `chronofork: <error>` diagnostics; no diagnostic
contaminates stdout; no network access and no telemetry. `record` encodes the complete
movie before writing, writes through a sibling temporary path and renames it on
success, so a failed run never replaces an existing valid movie file.

**Executed CLI tests (M3-T4):**

| Test target | Command | Evidence |
|---|---|---|
| CLI `tas` process target | `cargo test -p chronofork-cli --test tas` | `targets/cli-tas.stdout.txt`, `A7-cli-transcript.txt`, `cli/` |
| Existing CLI regression | `cargo test -p chronofork-cli` | `targets/cli-all.stdout.txt` |

**Observed result:** PASS. Help documents the surface and the deliberate omissions;
the success blocks match the shapes above; `movie_sha256` equals the file SHA-256; the
12 argument/combination failures, corrupt file, missing file and directory cases exit
1 with typed stderr and empty stdout; the failed record preserved an existing sentinel
movie; `--steps` accepts at most 4,194,304; `replay_movie` (the shared replay path)
is used by both `play` and `verify`; and the existing CLI targets stayed green. The
`tas` help text states that multi-controller/ports/analog input, frame advance, rewind,
seek, rerecord, branch editing and real-hardware input TAS are not implemented.
Evidence: `A7-cli-transcript.txt`, `targets/cli-tas.stdout.txt`, `targets/cli-all.stdout.txt`,
`cli/`.

**Status:** executed - passed (M3-T4, 2026-11-15).

## 7. Baseline used as preconditions

These capabilities existed before M3 and remain useful preconditions. They are
superseded as M3 evidence by the executed targets in §4 and the row evidence in §5;
authoritative gate results belong in [quality gates](quality-gates.md).

| Existing evidence | What it is designed to show | What it did not cover before M3 |
|---|---|---|
| `crates/chronofork-core/src/input.rs` (`InputFrame`) | A backend-defined `u32` button mask exists | Per-step recording, a schema version or a movie |
| `crates/chronofork-backend-api/src/traits.rs` (`StepOutcome`) | A step returns an outcome tick, state hash and halted flag | Aggregating outcomes into a file or comparing them later |
| `crates/chronofork-sessions/src/snapshot.rs` | Canonical checkpoint envelope round-trip, checksum and version rejection | A movie container or embedding/framing |
| `crates/chronofork-sessions/src/manager.rs` (`create_session_from_snapshot_bytes`) | Serialized bytes import into a fresh session in one process | Movie replay, first divergence or restart from a movie |
| `crates/chronofork-sessions/tests/acceptance.rs::restore_and_replay_reproduce_the_branch_state` | Restore plus replay reproduces tick and hash in one process | A recorded input sequence, a movie file, per-step expectations or a new process |
| `crates/chronofork-sessions/src/storage/` and the M2 targets | Durable checkpoints, metadata, restart, ownership, bounds, rejection | A movie format or a recording seeded by a durable checkpoint |
| `crates/chronofork-cli/src/main.rs` (`session` commands) and `crates/chronofork-cli/tests/session_storage.rs` | Stable block output, exit codes and process tests | A `tas` command, movie I/O or divergence output |
| `crates/chronofork-test-backend/src/lib.rs` | Deterministic one-input-per-step execution and `APPLY_INPUT` | A TAS file, a recorder or multi-controller input |
| `crates/chronofork-megadrive/src/backend.rs` | The minimal Mega Drive slice executes deterministically | Input is ignored (`_input`), so it is not an input-TAS backend |

As of M3-T4, the previously absent pieces exist: `crates/chronofork-sessions/src/movie.rs`,
`crates/chronofork-sessions/tests/movie.rs`, `crates/chronofork-cli/tests/tas.rs` and
the `chronofork tas` command. They are M3 evidence and are recorded in §4, §5 and §9,
not in this baseline table.

## 8. Traceability

| Criterion | Requirements | ADR-0004 sections |
|---|---|---|
| M3-A1 round-trip/canonical bytes | FR-TAS-002, FR-EXEC-004, NFR-DET-003, NFR-QUAL-007 | §2, §3, §4, §6 |
| M3-A2 bounds/typed rejection | FR-TAS-002, NFR-DUR-003, NFR-DUR-004, NFR-STOR-002, NFR-QUAL-007 | §2, §4, §7 |
| M3-A3 record/replay reproduction | FR-TAS-001, FR-EXEC-002, FR-EXEC-003, FR-EXEC-004, NFR-DET-003, NFR-DET-004 | §5, §6, §9 |
| M3-A4 first divergence | FR-TAS-001, FR-EXEC-005, NFR-DET-006 | §6, §9 |
| M3-A5 restart and M2-seeded record | FR-TAS-001, FR-TAS-002, FR-SNAPSHOT-004, FR-SNAPSHOT-006, NFR-DUR-001 | §3, §4, §9 |
| M3-A6 source non-mutation/isolation | FR-FORK-005, NFR-ISO-001, NFR-ISO-002, NFR-ISO-003 | §9 |
| M3-A7 CLI contract | FR-API-001, FR-TAS-001, FR-TAS-002 | §11 |
| M3-A8 determinism/no host time | FR-EXEC-001, NFR-DET-001, NFR-DET-003, NFR-DUR-005 | §2, §3, §6 |
| M3-A9 M2 regression/gate/demos | NFR-QUAL-001, NFR-QUAL-005, M2-A1–A11 | §13 |
| M3-A10 scope honesty | NFR-QUAL-005, NFR-DOC-001, NFR-DOC-003 | §12 |
| M3-A11 branch metadata | FR-FORK-003, FR-TAS-002, NFR-ISO-001 | §3, §9 |
| M3-A12 one codec/predicate/no deps | NFR-QUAL-002, NFR-QUAL-004, NFR-QUAL-006, AS-01, AS-02, AS-11, AS-15 | §10 |

At the M3-T4 execution record, FR-TAS-001 is **Partial** (record/playback of a
per-tick input sequence and replay verification are implemented and verified on the
deterministic synthetic backend; frame advance, rewind, seek, rerecord, branch editing
and multi-controller input remain deferred; there is no hardware input), FR-TAS-002 is
**Implemented** (the canonical v1 `.cfmovie` format is verified) and FR-EXEC-005 is
**Implemented** (exact first-divergence reporting is verified). The
[requirements](../requirements/functional.md) and the t4 evidence are the authority;
see also [quality gates](quality-gates.md).

## 9. Execution record (2026-11-15)

**Executed by the independent M3-T4 verification: PASS.** Tree under test: HEAD
`bcd6c6fae7edd8014aec27e89689933e182c246a` (branch `main`, equal to `origin/main`)
plus 7 modified tracked paths and 5 untracked implementation/contract paths. No commit
or push was made. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

1. **Full gate:** `cargo fmt --all -- --check`, `cargo check --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, both demos,
   `./scripts/check.sh` and `git diff --check` all exited 0; the gate logs are in
   `logs/`.
2. **Test targets:** every target in §4 exited 0. The per-target counts and M3-T0
   comparison are in `target-matrix.txt`; no pre-existing target was below its baseline
   and the M2 targets kept their counts.
3. **Demos and state probe:** both demos ran three times with 0-byte program stderr and
   byte-identical stdout to the M3-T0 baseline; the independent-root checkpoint probe
   matched the baseline bytes, tick and state hash (`demos/`, `probes/`).
4. **Acceptance rows:** M3-A1–A12 all executed and passed; the per-row commands and
   artifacts are in §5. No criterion failed and no test, lint or assertion was weakened.
   t4 findings: none blocking. Observations: O-1 was the checksum-first truncation
   taxonomy, reconciled in M3-A2; O-2 is this documentation promotion; O-3 required no
   docs change because foreign-backend/configuration rejection remains the unchanged
   `validate_envelope` path.
5. **Evidence:** `.research/m3-tas/verification/` (`SUMMARY.txt`,
   `target-matrix.txt`, `A1-*.txt`–`A12-*.txt`, `logs/`, `targets/`, `cli/`, `demos/`,
   `probes/`, `MANIFEST.sha256`); the dated gate-result block is in
   [quality gates](quality-gates.md). The verifier's verdict is PASS.

This is an uncommitted working tree; the evidence binds that tree, not a commit. Any
later edit to an implementation path invalidates the equality claims until re-verified.

## 10. Out of scope and stated limitations

The first increment delivered the canonical v1 format, record/playback of a per-step
input sequence, replay verification with exact first-divergence reporting, restart and
M2-seeded replay from the embedded checkpoint, and the minimal `chronofork tas`
CLI. The following are **not implemented and must not be claimed**:

- **Synthetic backend only.** The acceptance claim covers the deterministic synthetic
  backend. It is a test fixture, not a consumer emulator, and no Mega Drive or other
  hardware accuracy is claimed.
- **No real-hardware input TAS.** The Mega Drive slice's `step` ignores input, so its
  recorded inputs have no effect. Real-hardware input TAS is deferred.
- **No multi-controller, ports, analog or keyboard.** Input schema v1 is one
  backend-defined `u32` mask per step; other schemas are a future version with
  explicit rejection.
- **No frame advance, rewind or seek.** The movie is recorded in one pass and replayed
  from its start to its end; there is no timeline UI, frame stepping, seeking or
  partial replay.
- **No rerecord or branch editing.** v1 defines only passive `parent_movie_id` and
  `name` metadata; the CLI does not expose branch creation, and no tool edits a movie
  in place.
- **No import/export adapters.** Interoperability with BizHawk, Gens or other TAS
  formats is documented as future work and is not implemented.
- **No HTTP, MCP, GUI or daemon.** The format and replay are local, synchronous,
  std-only operations. Stage 6 interfaces are deferred.
- **No checkpoint-reference movie.** v1 embeds the canonical checkpoint; referencing a
  durable checkpoint by key/checksum is deferred and would need its own decision and
  fallback rules.
- **No compression, encryption, authentication or deduplication.** Movie files can be
  large and contain emulated memory; protecting them is the operator's
  responsibility.
- **No resynchronization, partial replay or branch comparison tooling.** A divergence
  stops replay at the first mismatch.
- **No cross-backend or cross-platform portability claim.** A movie from another
  backend version or configuration is rejected at replay import.
- **No performance or capacity target.** The 44-bytes-per-step and 176 MiB step-cap
  figures are format costs, not benchmarks.
- **v1 format limits apply.** A movie is bounded to 256 MiB, 4,194,304 steps and a
  256-byte name; a larger movie is rejected before allocation, and raising a limit
  needs an ADR amendment and new rejection evidence.
- **No regression of the M2 baseline.** All M2 targets and both demos remain green on
  the M3 tree; the durable gates are recorded in [quality gates](quality-gates.md).
- **No commits or pushes.** M3 evidence remains under ignored `.research/` paths until
  the owner explicitly authorizes a commit.
- **M3 is not complete.** This increment is the first vertical slice of Stage 3, not
  the whole stage.

## 11. Change control

- A change to a criterion, expected typed category, command, target name or artifact
  path requires an explicit ADR/plan amendment; it must not be made by quietly
  renaming a test or loosening an assertion.
- The names in this document describe the approved contract shape. Final spelling may
  be refined by the approved Fase B plan; the semantics, canonical bytes, limits and
  rejection boundaries must remain equivalent.
- If a planned command cannot run because its target is absent, the row remains
  `planned - not yet executed`; the gap is reported, not hidden.
- Once a row has executed, its result is append-only evidence linked from
  [quality gates](quality-gates.md); do not rewrite history or relabel a failure.
- A failed row blocks M3. Never weaken, delete or skip a test to obtain a green gate,
  and never widen a claim beyond the verified synthetic-backend scope.
