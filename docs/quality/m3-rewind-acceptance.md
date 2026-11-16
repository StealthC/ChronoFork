# M3 Rewind — Executable Acceptance Plan

- **Status:** **Executed; every row M3-A28–M3-A34 is `executed - passed`** (independent
  t5 PASS, 2026-11-15). The increment is complete for the deterministic synthetic backend;
  M3 is **not complete** (rerecord/branch editing and real-hardware input TAS remain
  unimplemented).
- **Date:** design 2026-11-15; execution record 2026-11-15
- **Companion contract:** [ADR-0007: in-process rewind state cache and interactive
  stepping](../adr/0007-in-process-rewind-and-interactive-stepping.md) — **Accepted**
  (owner-approved 2026-11-15); implemented and independently verified for the deterministic
  synthetic backend.
- **Extends:** the accepted v1 contract [ADR-0004](../adr/0004-tas-movie-format.md), the
  accepted increment-2 contract
  [ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md) and the accepted v2
  contract [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md).
  Their executed rows (M3-A1–A12, M3-A13–A17, M3-A18–A27) are **regression
  preconditions**, never acceptance for M3-A28–A34.
- **Related:** [execution model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [M3 first-increment acceptance plan](m3-tas-replay-acceptance.md), [M3 increment-2
  acceptance plan](m3-partial-replay-acceptance.md), [M3 format-v2 acceptance
  plan](m3-format-v2-acceptance.md), [testing strategy](testing-strategy.md), [quality
  gates](quality-gates.md), [anti-slop policy](anti-slop-policy.md).

> **Verification record (2026-11-15).** The independent t5 verifier executed M3-A28–A34 on
> the uncommitted increment-4 tree at HEAD `b8b0f935e40e08854c2155f1b5208f6fc91424d8`
> (branch `agent/m3-complete`) plus the increment-4 working-tree paths with
> `CARGO_HOME=/tmp/t7-cargo`, and returned **PASS**. Every gate command exits 0, the full
> workspace suite reports 35 targets / 458 passed / 0 failed / 0 ignored with no target
> below the 33-target / 422-passed baseline, both demos are byte-identical, and the
> structural and scope audits are green. Per-row commands, observed values and artifact
> names are in §5 and §9. Raw evidence is in the ignored
> `.research/m3-rewind/verification/` directory (`SUMMARY.txt`, `target-matrix.txt`,
> `rows/`, `audit/`, `targets/`, `cli/`, `studio/`, `demos/`, `probes/`, `gate/`, `tree/`,
> `MANIFEST.sha256`); the pre-change baseline is `.research/m3-rewind/baseline/`. The dated
> gate-result block is in [quality gates](quality-gates.md). M3 is **not complete**:
> rerecord/branch editing and real-hardware input TAS remain unimplemented. Non-blocking
> observations O1–O4 are recorded in §9.

## 1. Status rules

1. **`planned - not yet executed`** means the criterion has a planned test target,
   command and evidence artifact, but no executed result and no independent verifier
   record exists. Every row had that status at design time; after the t5 execution record
   every row is `executed - passed` (§9).
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
5. No rewind, state-cache or interactive-stepping behavior may be described as
   implemented, working, supported or complete on the strength of ADR-0007, this plan, a
   code review, a demo run or a single happy-path run. The executed artifacts are the
   only evidence; the v1/v2 rows (M3-A1–A27) are evidence for the prior increments only.
6. A planned test whose command cannot execute because its module or test target does not
   exist remains `planned - not yet executed`; it does not become "passed" because the
   absent target produced no failure.
7. A row marked passed must cite a verifier artifact whose SHA-256 matches the recorded
   manifest. Late or reconstructed evidence does not reopen a failed row.
8. A row may not be satisfied by re-running an existing capability. A green M3-A1–A27 run
   is a regression precondition, never acceptance for A28–A34; an anchored `play` or a
   `play --until` rewind is not in-process rewind.
9. Acceptance is limited to the deterministic synthetic backend on the Linux acceptance
   platform. The Mega Drive slice ignores input and is not an acceptance backend for input
   TAS; no hardware rewind, input or accuracy claim is made.
10. ADR-0007 must be **Accepted** before implementation starts (satisfied: owner-approved
    2026-11-15; see the ADR header). Approval alone does not execute a row; implementation
    without approval fails this plan's change control (§11).

## 2. Scope, definitions and evidence

- **Player:** the session-layer type defined by [ADR-0007 §2](../adr/0007-in-process-rewind-and-interactive-stepping.md#2-ownership-and-module-placement)
  (`chronofork_sessions::player::Player`). It owns a private `SessionManager` with exactly
  one imported session, the decoded `Movie`, its current verified position and one bounded
  state cache.
- **Floor:** the canonical state imported from the movie's embedded checkpoint at the
  movie start tick, verified at `open`, always retained and outside the cache. It is the
  fallback start state when no cached state qualifies.
- **Cache entry:** a verified canonical `BackendSnapshot` plus the recorded step index,
  observed tick and observed state hash of the step that produced it, keyed by step index
  (ADR-0007 §3).
- **Cadence:** a state is captured after recorded steps `N-1, 2N-1, …` for cache interval
  `N` (the [ADR-0006 §5](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md#5-anchors-format-validation-and-recording)
  anchor index convention). `N = 0` disables capture.
- **Deterministic byte metric:** `Σ(payload length + 256)` over cached entries. It is the
  enforced and reported value; it is an accounting estimate, not an allocator measurement
  (ADR-0007 §3).
- **Position:** `start` (floor, zero recorded steps applied) or one position per recorded
  step (`applied_steps = step_index + 1`).
- **Aligned tick:** the movie start tick or one of its recorded outcome ticks. A rewind
  target must be aligned; anything else is rejected, never rounded or interpolated.
- **Rewind:** restore the greatest verified cached state at or before the target (or the
  floor), replay the recorded inputs forward through the target verifying every step, and
  report the exact first divergence. Cost is proportional to the distance from the start
  state; there is no O(1) or random-access seek.
- **`tas studio`:** the stdin-driven interactive adapter defined by
  [ADR-0007 §6](../adr/0007-in-process-rewind-and-interactive-stepping.md#6-interactive-stepping-cli),
  owning exactly one in-process player and printing one `key=value` block per command.
- **Acceptance backend:** the deterministic synthetic backend
  (`chronofork-test-backend`) is the only acceptance backend. The Mega Drive slice ignores
  input and is not an acceptance backend.
- **Platform:** Linux is the acceptance platform, matching the M1/M2/M3 evidence
  platforms. Windows and macOS are compile-checked in CI; runtime behavior there is not
  claimed.
- **Pre-change baseline:** the ignored `.research/m3-rewind/baseline/` path, captured
  2026-11-15T22:00:12Z on branch `agent/m3-complete` at HEAD
  `b8b0f935e40e08854c2155f1b5208f6fc91424d8` (clean tree), with gate logs, per-target
  lists, both demos (three runs each), environment and Git state.
- **Verification evidence (executed):** raw output is under the ignored
  `.research/m3-rewind/verification/` path: `SUMMARY.txt`, `target-matrix.txt`,
  `rows/A28-*.txt` … `rows/A34-*.txt`, `rows/M3-A1-A27-regression.txt`, `targets/`, `cli/`,
  `studio/`, `audit/`, `demos/`, `probes/`, `gate/`, `tree/` and `MANIFEST.sha256`. Each
  artifact records the command, tree state, exit code and the relevant
  stdout/stderr/SHA-256. The dated gate-result block is in
  [quality gates](quality-gates.md) and is verifier-owned.
- **No host time in data or reports:** valid output contains no timestamp, process id,
  host label, nonce, path or entropy. Comparisons use ticks, state hashes, step indices,
  block fields, movie bytes and SHA-256, never wall-clock time or filesystem modification
  time.

## 3. Increment exit criteria map

Every mapped row is `executed - passed` and the independent t5 verifier recorded PASS
(2026-11-15). This increment is complete for the deterministic synthetic backend; it does
not complete M3.

| Increment capability | Acceptance criteria |
|---|---|
| Bounded state cache: capture path, keying, cadence, count/byte bounds, deterministic eviction, memory reporting | M3-A28 |
| Deterministic rewind: exact tick/state hash at every target class, floor fallback, first-divergence attribution | M3-A29 |
| `chronofork tas studio` process contract: flags, blocks, streams, exit codes, EOF, invalid input, divergence, no writes | M3-A30 |
| Isolation, one-session ownership, determinism and no host-dependent input | M3-A31 |
| Module ownership, one-rule-per-concern, bounds and suppression audits | M3-A32 |
| Non-regression, full gate, demos and release equality | M3-A33 |
| Scope honesty, deferred list and no M3-complete or O(1) claim | M3-A34 |

## 4. Executed test targets (t5)

These targets were executed by the independent t5 verifier on the increment-4 tree with
`CARGO_HOME=/tmp/t7-cargo`. Per-target counts and the baseline comparison are in
`target-matrix.txt`; raw stdout/stderr is in `targets/` and `gate/`. The exact counts stay
in the evidence, not in this durable document.

| Executed target | Executed command | Result | Criteria |
|---|---|---|---|
| Player/cache unit tests in `crates/chronofork-sessions/src/player.rs` | `cargo test -p chronofork-sessions --lib` | 74 passed, including the rewind cache unit tests and the alignment test | M3-A28, M3-A29, M3-A31, M3-A32 |
| Player integration target `crates/chronofork-sessions/tests/player.rs` | `cargo test -p chronofork-sessions --test player -- --nocapture` | 16 passed | M3-A28, M3-A29, M3-A31 |
| Release-mode player | `cargo test -p chronofork-sessions --release --test player` | 16 passed | M3-A31, M3-A33 |
| CLI process target `crates/chronofork-cli/tests/studio.rs` | `cargo test -p chronofork-cli --test studio -- --nocapture` | 11 passed | M3-A30, M3-A31 |
| Release-mode studio | `cargo test -p chronofork-cli --release --test studio` | 11 passed | M3-A30, M3-A31, M3-A33 |
| Existing CLI regression | `cargo test -p chronofork-cli` and `--test tas` | 65 passed over six targets; `tas` 21 | M3-A30, M3-A33 |
| Existing v1/v2 movie, replay and session targets | the movie, movie_v2, replay, session_storage, M2 and M3-A1–A27 matrices | every target at or above its baseline count | M3-A33 |
| Full gate | `./scripts/check.sh`; `cargo fmt --all -- --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`; `git diff --check` | all exit 0; 35 targets / 458 passed / 0 failed / 0 ignored | M3-A33 |
| Demos | `cargo run -p chronofork-cli -- demo` and `-- megadrive demo`, three cargo runs and three direct-binary runs each, `cmp` and SHA-256 against the baseline | byte-identical | M3-A33 |
| Help capture | `tas --help`, `tas studio --help`, `chronofork help`, `backend describe` | captured and audited | M3-A30, M3-A34 |
| Process transcripts and audits | scripted studio runs (cache, bounds, eviction, divergence, EOF/quit, invalid), structural, suppression, dependency and scope scans | see §5 rows | M3-A28–A34 |

Every Cargo command was run with `CARGO_HOME=/tmp/t7-cargo` and the variable is recorded in
the evidence.

## 5. Acceptance criteria

Every row below was executed and passed in the t5 record. The pass conditions are retained
unchanged as the verified contract; each row records the executed targets, the observed
result and the artifact paths without volatile counts. The typed categories, bounds,
cadence and rewind semantics are fixed by
[ADR-0007 §2–§7](../adr/0007-in-process-rewind-and-interactive-stepping.md#3-cache-model).

### M3-A28 — Bounded state cache: capture, keying, cadence, bounds, eviction and reporting

**Passes only when** all of the following are demonstrated on the deterministic synthetic
backend:

1. **Capture source and keying.** Every cache entry is keyed by the zero-based recorded
   step index whose outcome produced it, and its stored tick and state hash equal that
   step's recorded expectation. A state is captured only after the step was executed and
   verified. The floor is the state imported from the embedded checkpoint at
   `start_tick` and is outside the cache.
2. **No snapshot record, no checkpoint identity.** A full forward pass with interval 1
   over a movie with several steps leaves the player session's `snapshots` map empty and
   its `next_snapshot_id` allocator unchanged, asserted by a crate-internal test. No
   `SnapshotId` is allocated by a capture, and no stored-checkpoint accessor is used by the
   cache.
3. **Cadence.** For interval `N >= 1`, captured step indices are exactly `N-1, 2N-1, …`
   below the step count; `N = 1` captures every step; `N > step_count` captures nothing;
   interval `0` captures nothing; a partial tail captures nothing extra. The same movie,
   options and command sequence produce the same entry set.
4. **Configuration validation.** `capacity == 0`, `capacity > 65,536`, `byte_limit == 0`,
   `byte_limit > 67,108,864` and `interval > 4,194,304` are typed configuration errors
   raised before any session is imported; valid values are accepted.
5. **Count bound.** The number of cached entries never exceeds the configured capacity.
6. **Byte metric.** The reported `cache_bytes` equals the sum over entries of
   `payload length + 256` exactly, is identical across two separate processes and across
   debug/release, and never depends on allocator behavior, timing or host state.
7. **Byte bound and per-entry cap.** The reported `cache_bytes` never exceeds the
   configured byte limit. A single state whose entry metric exceeds the byte limit fails
   the command with the typed cache-entry-too-large error and leaves no partial entry.
8. **Deterministic eviction.** With capacity `C` and more verified cadence steps than `C`,
   the retained indices are exactly the `C` highest captured indices; the eviction count
   equals captures minus `C`; eviction removes the lowest step index first; the floor and
   the newest entry are never evicted.
9. **Reporting.** The `state` block reports `cache_interval`, `cache_capacity`,
   `cache_byte_limit`, `cache_entries`, `cache_bytes` and `cache_evictions`; each `seek`
   block reports `cache_entries`, `cache_bytes` and `cache_evictions`; equal command
   sequences report equal values in separate processes.
10. **Bounds precede allocation.** Capacity, byte limit and interval are validated before
    the player allocates a cache or imports a session; no cache structure is sized from an
    unvalidated user value; arithmetic on byte totals and step indices is checked.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Cache/alignment unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --lib` | `targets/sessions-lib.stdout.txt`, `rows/A28-cache-bounds.txt` |
| Player integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --test player -- --nocapture` | `targets/sessions-player.stdout.txt`, `rows/A28-cache-bounds.txt` |
| Release-mode player | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-sessions --release --test player` | `targets/release-player.stdout.txt` |
| Studio cache/bound transcripts | scripted `tas studio` runs with `--cache-interval`, `--cache-capacity`, `--cache-bytes` | `studio/cache/` (`interval-*`, `byte-metric.txt`, `after-eviction.*`, `bytes-too-small-*`, `accepted-bounds.*`), `studio/results.txt` |
| Capture and structural audit | scan of the capture/restore path and the snapshot-identity assertion | `audit/A28-capture-audit.txt`, `audit/A32-*.txt` |

**Observed result:** PASS. Cadence was exactly `floor(12/N)` (interval 1→12 entries, 2→6,
3→4, 5→2, 12→1, 13→0, 0→0); the byte metric was 541 = payload + 256 (12 entries→6492,
4→2164, 3×→1623); count and byte bounds held with deterministic evict-lowest-first
(capacity 5→5 entries/7 evictions, capacity 1→1/11, interval-3/capacity-2→2/2); a state
larger than the byte budget was a typed error at limit 540 with empty stdout; the
crate-internal test showed an unchanged `(snapshots.len(), next_snapshot_id)`; equal scripts
in two processes and a release build reported equal metrics. Evidence:
`rows/A28-cache-bounds.txt`, `studio/cache/`, `audit/A28-capture-audit.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A29 — Rewind exactness versus fresh replay at every target class

**Passes only when** all of the following are demonstrated on the deterministic synthetic
backend:

1. **Same-tick equality.** For `start_tick` and for every recorded outcome tick, a seek
   reports observed `tick` and `state_hash` equal to the recorded expectation and equal to
   a full `Movie::verify` and a `ReplayRange` full-range replay of the same movie.
2. **Cached start, no replay.** With interval 1 and a target on a cached tick, the seek
   reports `start=cache`, `start_step=<target step>`, `replayed_steps=0`, and the restore
   consistency check (restored tick and hash equal the stored values) ran; the reported
   observed values equal the recorded expectation.
3. **Cached start with replay.** A target after a cached state reports `start=cache` and
   `replayed_steps = target_step - start_step`; every replayed step was verified; the
   result equals a full replay at the same tick.
4. **Floor fallback before the earliest cache.** A target earlier than the earliest cached
   state (including with interval 0 and after eviction) reports `start=floor`, no
   `start_step`, `replayed_steps` equal to the target's applied step count, and the exact
   recorded tick and state hash.
5. **After eviction.** A cache that has evicted early entries reports the highest retained
   step index at or before the target (never a later one) or the floor; the result stays
   exact for every retained and evicted region.
6. **Caching disabled.** With interval 0 every seek reports `start=floor` and exact
   results; repeated seeks repeat the work and stay exact.
7. **Position classes.** `goto start_tick` reports `applied_steps=0`, `replayed_steps=0`
   and the checkpoint state; a seek to the current position reports `replayed_steps=0`; a
   zero-step movie's only valid target is its start tick.
8. **Invalid and boundary targets.** A target before the start, after the end or unaligned,
   `next` at the end position and `back` at the start position each produce their typed
   category, execute no step, perform no restore and leave the position unchanged.
9. **First divergence.** A re-signed movie whose expectation is wrong at a step after a
   cached state reports `Divergence` with exactly that recorded tick, zero-based input
   index, expected hash and observed hash, both from a cache start and from the floor
   start; a wrong recorded tick reports `TickMismatch`; a later mismatch is never reported
   instead of an earlier one. A movie whose earliest step is wrong fails the first forward
   pass, and no cache entry after the failing step exists in that process.
10. **No interpolation, no O(1) claim.** Every reported tick is `start_tick` or a recorded
    outcome tick; no value between recorded ticks is reported; the plan makes no timing,
    latency or complexity equality assertion, and the cache never supplies a start state
    the process did not verify.
11. **No cross-invocation state.** Two processes running the same command script produce
    identical blocks. A fresh process seeking to the same tick reports the same observed
    tick and state hash (the `start` kind and `replayed_steps` may differ because the
    fresh process has an empty cache), and no cache or session state survives process exit.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Rewind unit/integration tests | `cargo test -p chronofork-sessions --lib`, `--test player`, `--release --test player` | `targets/sessions-lib.stdout.txt`, `targets/sessions-player.stdout.txt`, `targets/release-player.stdout.txt`, `rows/A29-rewind-exactness.txt` |
| Existing ranged replay | `cargo test -p chronofork-sessions --test replay` | `targets/sessions-replay.stdout.txt` |
| Per-tick cross-check | `tas studio` `goto <tick>` versus `tas play\|verify --until <tick>` for every recorded tick | `studio/exactness/cross-check.tsv`, `studio/exactness/` |
| Divergence controls | re-signed tampered movies through the library and the studio process target | `studio/divergence/` (`floor-start`, `cache-start`, `earliest-divergence`, `later-not-reported`, `tick-mismatch`), `rows/A29-rewind-exactness.txt` |
| Two-process and fresh-process equality | the same stdin script in separate processes; per-tick fresh process versus warmed process | `studio/scripts/basic-p1.*`, `studio/scripts/basic-p2.*`, `studio/exactness/` |

**Observed result:** PASS. All 12 recorded ticks matched `tas play --until` and `tas verify
--until` (36 runs, 0 mismatches), and a single process replaying `goto 1 … goto 12` produced
the same hashes. Cached-start/no-replay, cached-start-with-replay, floor fallback, after
eviction, caching disabled, zero-step and current-position classes were exercised; first
divergence was exact from the floor (tick 6, step 5) and from a cache start (start tick 4,
2 replayed), an earlier mismatch won over a later one, and a recorded-tick mismatch stayed
the typed stderr-only failure. Two-process and fresh-process equality held. Evidence:
`rows/A29-rewind-exactness.txt`, `studio/exactness/`, `studio/divergence/`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A30 — `tas studio` process contract

**Passes only when** the CLI implements the surface fixed by
[ADR-0007 §6](../adr/0007-in-process-rewind-and-interactive-stepping.md#6-interactive-stepping-cli)
with the exact flags, defaults, blocks, streams and exit codes:

1. **Flags.** `tas studio` accepts exactly `--movie <FILE>`, `--cache-interval <N>`
   (default 1, `0` allowed), `--cache-capacity <N>` (default 256, range 1..=65,536) and
   `--cache-bytes <N>` (default 16,777,216, range 1..=67,108,864), plus `--help`. Missing,
   duplicate, empty, non-numeric, out-of-range or foreign flags (`--from`, `--until`,
   `--steps`, `--input`, `--root`, `--session`, `--snapshot`, `--ports`, `--axes`,
   `--keys`, `--against`, positional arguments) exit 1 with a typed stderr diagnostic and
   empty stdout.
2. **Help.** `tas studio --help` and the `tas --help` summary document the stdin commands,
   the block fields, the cache defaults and maxima, that rewind restores a verified cached
   state or the embedded checkpoint and replays forward (not O(1), cost proportional to
   the distance), that nothing is written or persisted, the deferred list (rerecord/branch
   editing, real-hardware input TAS, persistent cache, O(1) seek, GUI/HTTP/MCP), the
   synthetic-backend scope and that M3 is not complete. Help capture artifacts are
   recorded.
3. **Block shape.** Each command emits exactly one block separated from the previous block
   by one blank line; the first line is `record=<name>`; the fields are exactly those fixed
   by ADR-0007 §6, one `key=value` line each, in the documented order, with no missing and
   no extra field, for `record=seek`, `record=state`, `record=help`, `record=quit` and
   `record=divergence`.
4. **Values.** At the start position and after every command, `tick`, `applied_steps` and
   `state_hash` match the library position and the recorded expectations; for a seek,
   `tick == target_tick`, `start`, `start_tick` and (only for `start=cache`) `start_step`
   are consistent with the chosen start state, and `replayed_steps` equals the number of
   steps actually executed by that command.
5. **EOF and quit.** End of stdin prints the quit block with `reason=eof` and exits 0; the
   `quit` command prints `reason=command` and exits 0; a final line without a trailing
   newline is processed; blank lines print nothing; no extra output follows the quit
   block.
6. **Invalid input.** Unknown command, wrong arity, extra tokens, malformed tick,
   before-start, after-end, unaligned target, `next` at the end and `back` at the start
   each produce stderr `chronofork: <error>` and exit 1, with no stdout block for that
   line and no position change; a later line in the same script is never processed.
7. **Divergence.** A re-signed tampered movie produces the `record=divergence` block with
   `diverged=1`, the exact recorded `tick`, `step`, `expected`, `observed`, `start`,
   `start_tick`, `replayed_steps`, `cache_entries` and `cache_bytes`, then exits 1; later
   steps are not reported. A recorded-tick mismatch, replay failure or any other typed
   failure is stderr-only with exit 1 and is never labelled a divergence.
8. **Streams and exit codes.** Valid stdout is `key=value` data only with no path,
   timestamp, process id, host name, movie SHA-256 or entropy; diagnostics are stderr only;
   exit codes are 0 for `quit`/EOF and 1 for every typed failure, and no other exit code or
   panic occurs.
9. **Read-only and no writes.** A complete studio run leaves the movie's SHA-256 unchanged,
   creates no file in the working directory, opens no storage root and writes no cache,
   checkpoint, movie or metadata file.
10. **`state` executes nothing.** `state` before any command and after seeks reports the
    current position; two consecutive `state` commands print identical blocks and
    `applied_steps` does not change.
11. **Process independence and equality.** The same stdin script run in two separate
    processes produces byte-identical stdout with empty program stderr; a release build
    produces an identical stdout block set for the same script and movie.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Studio process target | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test studio -- --nocapture` | `targets/cli-studio.stdout.txt`, `rows/A30-studio-contract.txt` |
| Release-mode studio | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --release --test studio` | `targets/release-studio.stdout.txt` |
| Existing CLI regression | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli` | `targets/cli-all.stdout.txt` |
| Help capture | `tas studio --help`, `tas --help`, `chronofork help`, `backend describe` | `studio/help/` |
| Invalid flags/commands and valid scripts | 24 invalid flag sets, 15 invalid command cases, EOF/quit/blank/position scripts | `studio/invalid/`, `studio/scripts/`, `studio/results.txt` |
| Block validator | `python3 .research/m3-rewind/verification/tools/validate-studio.py` | `studio/block-validation.txt` |
| Read-only proof | movie SHA-256 and working-directory listing before/after runs | `studio/non-mutation.txt` |

**Observed result:** PASS. The flags, defaults and maxima were exact, and 24 invalid flag
sets plus 15 invalid command cases failed closed with empty stdout and typed stderr. The
five block types matched ADR-0007 §6 field-for-field (206/206 transcript assertions); 95
studio runs produced only exit codes 0 (46) and 1 (49); EOF/quit/blank-line rules held; the
divergence block reported the exact tick/step/expected/observed; the movie SHA-256 and the
working directory were unchanged; debug and release output were byte-identical. The additive
top-level `chronofork help` lines are recorded as a declared addition in M3-A33 (observation
O2). Evidence: `rows/A30-studio-contract.txt`, `studio/block-validation.txt`,
`studio/results.txt`, `studio/non-mutation.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A31 — Isolation, one-session ownership and determinism

**Passes only when** all of the following are demonstrated:

1. **Private one-session manager.** `Player::open` takes a backend factory, not a caller
   `SessionManager`, and the player's manager holds exactly one session. A caller manager
   that already owns sessions is unaffected: its sessions keep their tick, state hash and
   snapshot records across player creation, seeks and drop.
2. **Two independent players.** Two players opened from the same movie (and from the same
   factory) do not share sessions, positions or caches; seeking one changes neither the
   other's position nor its cache metrics.
3. **No pre-existing session mutation.** A library test keeps an external manager and its
   session, runs a full player command sequence, and asserts the external session's tick,
   state hash and snapshot map are unchanged. No storage root is opened.
4. **No global state.** A structural audit finds no static mutable state, thread-local
   cache, singleton or process-wide registry in the player or studio paths; the player is
   mutated only through `&mut self`.
5. **Cross-process determinism.** The same movie bytes, options and command script produce
   byte-identical stdout and equal cache metrics in separate processes, including runs
   separated in time; the movie file is unchanged by the runs.
6. **Debug/release equality.** Debug and release builds produce equal block sets, equal
   `state_hash` values and equal cache metrics for the same fixtures.
7. **No host-dependent input.** A source scan shows no clock, environment-variable,
   process-id, filesystem-metadata or entropy API in the player and studio paths; valid
   stdout contains no path; changing the host clock between runs changes no byte.
8. **Metrics are reproducible.** `cache_entries`, `cache_bytes` and `cache_evictions`
   never read allocator or timing values and are equal for equal command sequences.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Isolation and unit tests | `cargo test -p chronofork-sessions --test player`, `--lib` | `targets/sessions-player.stdout.txt`, `targets/sessions-lib.stdout.txt`, `rows/A31-isolation.txt` |
| Release-mode equality | `--release --test player`, `--release --test studio` | `targets/release-player.stdout.txt`, `targets/release-studio.stdout.txt` |
| Two-process and release process runs | identical scripts in separate processes and a release binary | `studio/scripts/basic-p1.*`, `basic-p2.*`, `basic-release.*`, `studio/cache/interval-1*` |
| Source audits | clock/environment/global-state scan and capture-path audit | `audit/A31-host-time-audit.txt`, `audit/A28-capture-audit.txt` |

**Observed result:** PASS. The player owned a private one-session manager created from a
factory; two players were independent; an external manager and session were untouched; no
global state, clock, environment, pid or entropy API appeared in the player/cache/studio
paths; equal scripts produced byte-identical stdout across processes and debug/release
builds; cache metrics were equal and derived from payload lengths and step indices
(541 = 285 + 256). Evidence: `rows/A31-isolation.txt`, `audit/A31-host-time-audit.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A32 — Module ownership, one-rule-per-concern and bounds audit

**Passes only when** the increment is proven additive and single-sourced:

1. **Module placement.** The player and cache live in
   `crates/chronofork-sessions/src/player.rs` (or a `player/` submodule split by
   responsibility); the interactive adapter lives in
   `crates/chronofork-cli/src/studio.rs`; `tas.rs` keeps subcommand dispatch only. No
   second player, cache, block writer or command parser appears, and `lib.rs` exports only
   the public items the increment needs.
2. **Public API discipline.** Every new `pub` item has a caller in the increment or a
   documented external contract (AS-05); the cache type is private; no public `Session` or
   `SessionManager` method is added, renamed or removed; the capture/restore accessors are
   `pub(crate)` and their only caller is the player.
3. **One rule per concern.** Exactly one snapshot decoder (`Snapshot::from_bytes`), one
   compatibility predicate (`validate_envelope`), one checkpoint import path
   (`create_session_from_snapshot_bytes`), one replay/verification loop and comparison, and
   one aligned-tick rule shared by ranged replay and the player. No second alignment
   predicate, parser or compatibility check exists.
4. **No dependency, crate or backend change.** `Cargo.toml`/`Cargo.lock` are unchanged,
   `cargo tree -d` is unchanged, backend-api and megadrive have zero diff, and no
   backend-trait method, capability bit, snapshot format or state hash changes.
5. **No weakened checks.** `git diff -U0` adds no `#[allow]`, `#[ignore]`, `unsafe`,
   `todo!` or `unimplemented!`; no test, assertion or lint is removed or relaxed.
6. **Bounds and allocation.** Capacity, byte limit, interval and CLI numeric flags are
   validated against the fixed maxima before a session is imported or a cache allocated;
   no `with_capacity`/`vec![0; …]` is sized from an unvalidated external value; byte totals
   and step-index arithmetic are checked.
7. **AS-14 discipline.** Each new module is below the 800-non-test-line review trigger or
   split by responsibility; `movie.rs` and `tas.rs` grow only by a tiny helper and dispatch
   wiring respectively. Measured counts are recorded in evidence only (AS-13).
8. **Typed errors at the boundary.** `PlayerError` preserves `MovieError`/`SessionError`
   as sources (no flattened strings), the CLI maps each typed failure to stderr and exit 1,
   and the divergence block remains the only stdout failure output.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Structural audit | `rg` scans for readers/writers, parsers, `validate_envelope`, the import path, the replay loop and the alignment helper | `audit/A32-structural-audit.txt` |
| Suppression/dependency audit | `git diff -U0` suppression scan, `git diff -- Cargo.toml Cargo.lock`, `cargo tree -d`, backend/core diff | `audit/A32-suppressions.txt`, `audit/A32-deps-backend.txt` |
| Module-size audit | `wc -l` on the player, cache, studio, movie and tas modules | `audit/A32-module-sizes.txt` |
| API reference scan | every new public name searched across `crates` and `docs` | `audit/A32-public-api.txt` |

**Observed result:** PASS. One player/cache implementation and one `StateCache` exist (the
cache is private); the new `Session`/`SessionManager` accessors are `pub(crate)` called only
by the player; there is one movie codec, snapshot parser, `validate_envelope` definition,
checkpoint import path, replay loop, comparison and alignment rule (the old
`is_recorded_tick` was deleted). No dependency, manifest, backend, emulation, payload or
state-hash change exists; no `#[allow]`/`#[ignore]`/`unsafe`/`todo!` was added and no test or
assertion removed. Non-test module sizes were 656 (player), 294 (cache) and 372 (studio),
below the AS-14 trigger. Evidence: `audit/A32-*.txt`, `rows/A32-structural-audit.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A33 — Non-regression, full gate, demos and release equality

**Passes only when** all of the following are recorded on the final tree:

1. **v1/v2 regression.** Every M3-A1–A27 row still passes, and the movie, movie-v2, replay
   and session targets keep their assertions; a v1 and a v2 movie replayed or compared
   through the new build produce their previous outputs.
2. **CLI output compatibility.** Every pre-existing command's stdout is byte-identical to
   the captured baseline (equal SHA-256) *except* the declared additions recorded in the
   execution record: the `tas` dispatch error text accepts `studio`, `tas --help`/
   `tas studio --help` document the new surface, and the top-level `chronofork help` gains
   two `tas studio` usage lines and three descriptive lines (observation O2). Help-capture
   diffs are limited to those additions; no other transcript changes.
3. **M2/M1 regression.** The M2 acceptance target matrix and the M1 oracle differential
   evidence are re-executed and unchanged.
4. **Full gate.** `./scripts/check.sh` and the individual `cargo fmt --all -- --check`,
   `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
   and `git diff --check` all exit 0.
5. **Demos.** Both `cargo run -p chronofork-cli -- demo` and `-- megadrive demo` run three
   times with exit 0, empty program stderr and stdout byte-identical to the captured
   baseline (`cmp` exit 0, equal SHA-256).
6. **No coverage loss.** Every pre-existing test target is at least at its captured
   baseline count; any increase is explained by the new player/studio tests, and no
   assertion was removed or relaxed.
7. **Serialized state and dependencies.** The serialized `session save/load` probe on two
   independent roots matches the baseline bytes, tick and state hash; movie fixtures are
   byte-identical; manifests and `cargo tree -d` are unchanged.
8. **Final-tree binding.** The verifier records HEAD, branch, `git status --short
   --branch`, the changed-path set and artifact SHA-256s, and the PASS is bound to that
   tree. A later edit to an implementation path invalidates the claim until re-verified.

**Executed targets (t5):**

| Target | Command | Evidence |
|---|---|---|
| Full gate | the gate commands above, all with `CARGO_HOME=/tmp/t7-cargo` | `gate/` |
| Target matrix | every target in §4 plus the M2/M3 matrices, with baseline comparison | `target-matrix.txt`, `targets/` |
| Demos | both demos, three cargo runs and three direct-binary runs each, `cmp` and SHA-256 against the baseline | `demos/`, `demos/comparison.txt` |
| Serialized-state probe | baseline `session save/load` probe on two independent roots | `probes/session/serialized-state.txt` |
| CLI transcript and movie comparison | 32 pre-existing command transcripts and 8 movie fixtures against the baseline | `cli/transcripts-comparison.txt`, `cli/movies-comparison.txt` |
| M1 oracle differential | `python3 scripts/oracle-diff.py --skip-build` | `probes/oracle/` |
| Structural/regression audits | suppression, dependency and regression scans | `audit/A32-*.txt`, `audit/A33-regression-audit.txt` |

**Observed result:** PASS. Every gate command exited 0 (35 targets / 458 passed / 0 failed /
0 ignored; 32 targets unchanged, 1 increased by the new unit tests, 2 new targets, 0
missing/below); both demos were byte-identical across runs and to the baseline; 32/32
pre-existing CLI transcripts and 8/8 movie fixtures were byte-identical; the two-root
serialized-state probe matched the baseline (`tick=7`, `state_hash=a4c253d2…c275`); manifests
and `cargo tree -d` were unchanged; the M1 oracle differential passed 3/3 traces, 3/3 final
states, 3/3 negative controls and 3/3 scheduler controls; the verdict is bound to the
recorded tree. The additive top-level `chronofork help` lines for `tas studio` (two usage
lines and three descriptive lines) are recorded here as a declared additive change per
observation O2; every other pre-existing stdout is byte-identical. Evidence:
`rows/A33-regression.txt`, `gate/`, `demos/`, `cli/`, `probes/`, `audit/A33-regression-audit.txt`.

**Status:** executed - passed (t5, 2026-11-15).

### M3-A34 — Scope honesty, deferred list and no M3-complete claim

**Passes only when** the durable docs, CLI help and capability surfaces state exactly what
is implemented and what is not:

1. **Matches evidence.** Every rewind/cache/studio capability claim in ADR-0007, this
   plan, the execution model, session model, requirements, milestones, roadmap and
   `docs/README.md` cites an executed row/artifact; a claim without evidence is a finding.
   ADR-0007 is **Accepted** (owner-approved 2026-11-15) and, after the t5 PASS, implemented
   and independently verified only from executed evidence (§9).
2. **No false capability.** `tas --help`, `tas studio --help` and the docs do not advertise
   rerecord/branch editing, real-hardware input TAS, O(1)/random-access seek, a persistent
   or cross-process cache, anchor-seeded caches, a GUI, HTTP, MCP or a daemon.
3. **Deferred agreement.** The deferred lists in ADR-0007 §9, this plan §10, the
   requirements, milestones, roadmap and `docs/README.md` agree; rerecord/branch editing
   and real-hardware input TAS remain explicitly unimplemented.
4. **Milestone state.** No M3 item is checked unless its mapped rows are executed and
   passed; after the t5 PASS this increment's M3 item is checked while M3 remains
   incomplete.
5. **Requirement honesty.** `FR-TAS-001` remains **Partial** (rerecord and real-hardware
   input TAS still absent). `FR-EXEC-007` is **Partial** after the t5 PASS (rewind and
   interactive stepping done; frame pacing and a host-time run loop remain absent) and is
   never promoted to **Implemented** on rewind alone; `FR-EXEC-008` records the verified
   cache/stepping capability. `FR-MD-010` was **Planned** at this increment; it was
   delivered by the later final increment under
   [ADR-0009](../adr/0009-real-hardware-input-tas.md) (M3-A45–A53 in the [M3 hardware
   acceptance plan](m3-hardware-acceptance.md)).
6. **No migration or persistence claim.** The docs state that the cache is transient,
   in-memory and per-process, that it writes nothing, and that movie formats, anchors and
   storage are unchanged with no migration.
7. **Cost honesty.** The docs and help state that rewind is restore-plus-replay,
   proportional to the distance from the nearest verified state, that eviction or disabled
   caching increases that distance, and that no performance, latency or memory bound other
   than the configured count and byte metric is claimed.

**Executed targets (t5):**

| Check | Command | Evidence |
|---|---|---|
| Docs vs capability | read ADR-0007, this plan, the requirements, milestones, roadmap, `docs/README.md` and the CLI help; `rg` scope scan | `rows/A34-scope-audit.txt`, `audit/A34-scope-audit.txt` |
| Help honesty | `tas --help`, `tas studio --help`, `chronofork help`, `backend describe` | `studio/help/` |
| Milestone/requirement state | inspect that only executed rows are checked, `FR-EXEC-007` is not overstated and M3 is not complete | `audit/A34-scope-audit.txt`, [milestones](../roadmap/milestones.md) |

**Observed result:** PASS at verification time. The CLI help and durable docs advertised no
false capability, the deferred list was consistent, no M3 item was ticked and no document
claimed M3 complete; the requirement annotations were honest. Observation O1 (docs still
under-claimed at verification time) is resolved by this execution promotion, which updates
the docs from these artifacts. Observation O4 records one residual understatement: the
`tas --help` sentence "implemented under ADR-0007 but not independently verified" is
Rust code outside this documentation task's scope and now understates the PASS; it needs a
separate help-text change. Evidence: `rows/A34-scope-audit.txt`,
`audit/A34-scope-audit.txt`, `studio/help/`.

**Status:** executed - passed (t5, 2026-11-15).

## 6. CLI surface (implemented and verified)

The surface below is the accepted contract shape from
[ADR-0007 §6](../adr/0007-in-process-rewind-and-interactive-stepping.md#6-interactive-stepping-cli)
and is implemented and independently verified (M3-A30, t5 PASS). Command and flag spelling
may be refined only with an ADR/plan amendment; the semantics, defaults, blocks, exit codes
and stream split are the contract.

```text
chronofork tas studio --movie <FILE> [--cache-interval <N>] [--cache-capacity <N>]
                      [--cache-bytes <N>]
chronofork tas studio --help
```

stdin commands: `next`, `back`, `goto <TICK>`, `state`, `help`, `quit`; blank lines are
ignored; EOF behaves like `quit`. Output is one `key=value` block per command, separated by
one blank line, with the fields and order fixed by ADR-0007 §6. Exit 0 for `quit`/EOF, 1 for
every typed failure; stdout is data only, stderr is `chronofork: <error>` only. The studio
opens the movie read-only and never writes a file, a storage root or a cache.

Observed in the t5 execution record: the flags, defaults and maxima were exactly as
specified; 24 invalid flag sets and 15 invalid command cases failed closed; the five block
types matched the ADR field-for-field (206/206 transcript assertions); 95 runs produced only
exit 0/1; the divergence block reported the exact first mismatch; and the movie bytes and
working directory were unchanged (`studio/block-validation.txt`, `studio/results.txt`,
`studio/non-mutation.txt`). One declared additive change is outside this surface: the
top-level `chronofork help` gains two `tas studio` usage lines and three descriptive lines
(observation O2, recorded in M3-A33). The `tas --help` sentence that still says
"implemented under ADR-0007 but not independently verified" is a code-level understatement
recorded as observation O4; updating it needs a Rust help-text change outside this plan.

## 7. Baseline preconditions

These capabilities exist before this increment and are preconditions, not rewind evidence.
The pre-change baseline is already captured under `.research/m3-rewind/baseline/`
(2026-11-15T22:00:12Z, HEAD `b8b0f935e40e08854c2155f1b5208f6fc91424d8`, clean tree).

| Existing evidence | What it shows | What it does not cover here |
|---|---|---|
| `crates/chronofork-sessions/src/movie.rs` (one v1/v2 reader/writer, recorder, verifier, anchors) | Movie bytes, per-step expectations, anchors and embedded checkpoints are canonical and verified (M3-A1–A12, A18–A27) | Any in-memory state cache, player position or interactive surface |
| `crates/chronofork-sessions/src/replay.rs` (single ranged replay loop, anchor seek, comparison) | Rewind as a new invocation, range validation, first divergence and comparison are verified (M3-A13–A17, A21) | Cache-start replay, position tracking, in-process rewind |
| `crates/chronofork-sessions/src/session.rs` / `manager.rs` | Session lifecycle, canonical `snapshot()`/`restore()`, one import path and one compatibility predicate | A capture path that allocates no `SnapshotId`; a one-session player |
| `crates/chronofork-cli/src/tas.rs` / `main.rs` | Stable `key=value` blocks, stderr diagnostics, exit 0/1, block separator | A stdin command loop, a studio subcommand or cache flags |
| `crates/chronofork-test-backend` | Deterministic one-input-per-step execution and canonical state hashes | Any player, cache or interactive behavior |
| M2 storage and target matrix | Durable checkpoints, restart, bounds, rejection | Any cache, rewind or studio behavior |
| [Research: TAS and debugging](../research/tas-and-debugging.md) §5.3/§6.3 | Upstream evidence that seeking loads a cached state and replays forward, and that O(1) seek must not be promised | A ChronoFork implementation; research is context only |

After execution the absent pieces exist as `crates/chronofork-sessions/src/player.rs`,
`crates/chronofork-sessions/src/player/cache.rs`,
`crates/chronofork-sessions/tests/player.rs`, `crates/chronofork-cli/src/studio.rs` and
`crates/chronofork-cli/tests/studio.rs`; they are executed evidence recorded in §5 and §9,
never preconditions. The implementation uses the ADR-0007 §2.7 permitted cache split.

## 8. Traceability

| Criterion | Requirements | ADR-0007 sections |
|---|---|---|
| M3-A28 cache capture, cadence, bounds, eviction, reporting | FR-SNAPSHOT-001, FR-SESSION-001, FR-EXEC-008, NFR-ISO-001, NFR-DET-003, NFR-QUAL-005, AS-05, AS-12 | §2, §3, §4, §7 |
| M3-A29 rewind exactness and first divergence | FR-TAS-001, FR-EXEC-004, FR-EXEC-005, FR-EXEC-007, FR-EXEC-008, NFR-DET-003, NFR-DET-006 | §3, §5, §7 |
| M3-A30 `tas studio` process contract | FR-TAS-001, FR-EXEC-007, FR-EXEC-008, FR-API-001, NFR-QUAL-004 | §6, §7 |
| M3-A31 isolation, ownership and determinism | FR-SESSION-001, FR-FORK-005, FR-EXEC-008, NFR-ISO-001, NFR-ISO-002, NFR-ISO-003, NFR-DET-001, NFR-DET-003 | §2, §5, §7 |
| M3-A32 module ownership, one rule per concern, bounds | NFR-QUAL-001, NFR-QUAL-002, NFR-QUAL-006, AS-01, AS-02, AS-05, AS-11, AS-12, AS-14, AS-15 | §2, §3, §7 |
| M3-A33 non-regression, gate, demos, release equality | NFR-QUAL-001, NFR-QUAL-005, M1/M2/M3-A1–A27 | §2, §6, §9 |
| M3-A34 scope honesty and deferred list | NFR-QUAL-005, NFR-DOC-001, NFR-DOC-003 | §9, §10 |

After the t5 execution record `FR-TAS-001` remains **Partial** (rerecord and real-hardware
input TAS still absent), `FR-EXEC-007` is **Partial** (rewind and interactive stepping
implemented; frame pacing and a host-time run loop absent), `FR-EXEC-008` is
**Implemented** for the deterministic synthetic backend, `FR-API-001` covers the existing
surfaces including `tas studio`, and `FR-MD-010` remains **Planned**. The executed rows are
the authority for these annotations; M3 remains incomplete.

## 9. Execution record (2026-11-15)

**Executed by the independent t5 verification: PASS.** Tree under test: HEAD
`b8b0f935e40e08854c2155f1b5208f6fc91424d8` (branch `agent/m3-complete`) plus the
increment-4 working-tree paths (implementation, tests and docs); `tree/state-comparison.txt`
records that every implementation and test path kept its pre-edit SHA-256 and that the
verifier's only tracked edit was the dated [quality gates](quality-gates.md) block. No
commit, push or tag was made. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

1. **Full gate:** `cargo fmt --all -- --check`, `cargo check --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `./scripts/check.sh` and
   `git diff --check` all exited 0; per-command logs are in `gate/`.
2. **Target matrix:** 35 targets / 458 passed / 0 failed / 0 ignored against the
   33-target / 422-passed baseline; 32 targets unchanged, 1 increased by the new unit tests
   (sessions lib), 2 new targets (`sessions player`, `cli studio`), 0 missing or below.
   Per-target counts and the baseline comparison are in `target-matrix.txt`; release-mode
   player and studio targets also passed.
3. **Demos and probes:** both demos ran three times through `cargo run` and three times
   through the built binary with byte-identical stdout and empty program stderr (`demos/`);
   the two-root serialized-state probe, the 32 CLI transcripts, the 8 movie fixtures and the
   manifest hashes match the baseline (`probes/`, `cli/`); `cargo tree -d` is clean and
   `Cargo.toml`/`Cargo.lock` are unchanged.
4. **Acceptance rows:** M3-A28–A34 all executed and passed; the per-row commands, observed
   values and artifacts are in §5 and `rows/`. No criterion failed and no test, lint or
   assertion was weakened; the new player/studio tests add coverage and none was removed.
5. **Audits:** no added `#[allow]`/`#[ignore]`/`unsafe`/`todo!`/`unimplemented!`; one
   player/cache implementation, one codec, one snapshot parser, one compatibility predicate,
   one import path, one replay loop and one alignment rule; Mega Drive/core/backend-api have
   zero diff; payload versions and state hashes unchanged; no host time, pid, environment or
   entropy in the player/cache/studio paths (`audit/`).
6. **Scope:** the CLI help and durable docs advertised no false capability, the deferred list
   was consistent, no M3 item was ticked and no document claimed M3 complete
   (`audit/A34-scope-audit.txt`). This execution promotion updates the durable docs from the
   evidence.
7. **Evidence:** `.research/m3-rewind/verification/` (`SUMMARY.txt`, `target-matrix.txt`,
   `rows/`, `audit/`, `targets/`, `cli/`, `studio/`, `demos/`, `probes/`, `gate/`, `tree/`,
   `MANIFEST.sha256`); the dated gate-result block is in
   [quality gates](quality-gates.md). The verifier's verdict is **PASS**.

Non-blocking observations: **O1** — the durable docs and CLI help still carried
pre-verification status text at verification time; this t6 execution promotion updates the
docs from the evidence without changing a criterion or capability claim. **O2** — the
top-level `chronofork help` output gains two `tas studio` usage lines and three descriptive
lines; recorded here and in M3-A33 as a declared additive change (no pre-existing stdout
changed). **O3** — the recorded cache path is deterministic but stateful within a process:
repeated `goto` commands re-capture during replay and evict lower indices (with capacity 4,
`goto 12; goto 2; goto 8; goto 10` reports `goto 8` starting from cache step 1); this is the
documented evict-lowest-before-insert rule, is reproduced byte-for-byte in a second process,
and is asserted in M3-A29.5, not nondeterminism. **O4** — the `tas --help` sentence
"implemented under ADR-0007 but not independently verified" is Rust help text outside this
documentation task's scope and now understates the PASS; it needs a separate help-text
change and is recorded as a residual understatement, not a capability claim.

This is an uncommitted working tree; the evidence binds that tree, not a commit. The t6
promotion edits only documentation status/evidence text.

## 10. Out of scope and stated limitations

This increment does not implement or claim:

- **Rerecord and branch editing.** No in-place movie edit, rerecord counter, branch switch
  inside the player or parent resolution; `parent_movie_id` remains passive metadata.
- **Real-hardware input TAS at the increment-4 boundary.** At this increment the Mega
  Drive `step` ignored input; the studio replays whatever `MovieStep::input()` carries
  and invents no device interaction, and `FR-MD-010` was Planned. It was delivered by
  the later final increment under [ADR-0009](../adr/0009-real-hardware-input-tas.md)
  (M3-A45–A53 in the [M3 hardware acceptance plan](m3-hardware-acceptance.md)); the
  increment-4 scope above is unchanged.
- **O(1)/random-access seek, reverse execution and any performance target.** Rewind is
  restore-plus-replay, proportional to the distance from the start state; with caching
  disabled or after eviction it approaches a full replay.
- **A persistent or shared cache.** No cache file, cache format, cache version, cache
  migration, cross-process cache or cache export/import.
- **Tiered/delta/compressed states.** Every entry is a full canonical state; tiering and
  compression are future decisions.
- **Anchors as cache seeds.** v2 anchors remain a movie-level seek optimization; the player
  never starts from one.
- **Pause/resume scheduling, a host-time run loop, frame pacing and turbo seek.** Those
  belong to a future scheduling decision; `FR-EXEC-007` is Partial (rewind/stepping done,
  frame pacing and run loop absent).
- **A GUI, TUI, timeline, HTTP API, MCP adapter or daemon.** The surface is stdin/stdout.
- **Multi-device studio commands.** Schema-2 frames replay unchanged; no new device input
  surface is added.
- **Cross-backend or cross-platform runtime claims.** Acceptance is the deterministic
  synthetic backend on Linux; Windows/macOS runtime behavior is not claimed.
- **Any claim wider than the executed evidence.** The M3-A28–A34 rows bind the increment to
  the deterministic synthetic backend on Linux and to the recorded tree; no hardware input,
  cross-backend portability, O(1) seek or persistent cache is claimed.
- **M3 completion.** This is one increment of Stage 3; rerecord/branch editing and
  real-hardware input TAS remain unimplemented, so M3 is not complete.

## 11. Change control

- A change to a criterion, expected typed category, command, target name, output field or
  artifact path requires an explicit ADR/plan amendment; it must not be made by quietly
  renaming a test or loosening an assertion.
- The names in this document describe the verified contract shape. Implementation spelling
  may be refined only by an approved amendment; the semantics, bounds, defaults, block
  fields, typed boundaries and rejection categories must remain equivalent.
- If a planned command cannot run because its target is absent, the row remains
  `planned - not yet executed`; the gap is reported, not hidden.
- Once a row has executed, its result is append-only evidence linked from
  [quality gates](quality-gates.md); do not rewrite history or relabel a failure.
- A failed row blocks this increment. Never weaken, delete or skip a test to obtain a green
  gate, and never widen a claim beyond the verifier's synthetic-backend scope.
- **This plan does not authorize a commit or a push.** ADR-0007 is Accepted (owner-approved
  2026-11-15) and implemented and independently verified for the deterministic synthetic
  backend; no commit, push, tag or merge is authorized by the increment or by this plan.
- **2026-11-15 (owner approval).** ADR-0007 moved from Proposed to Accepted on the owner's
  statement "Aprovar ADR-0007 como proposto (Recomendado)" at
  2026-11-15T19:17:48-03:00 (America/Fortaleza), 2026-11-15T22:17:48Z. No criterion,
  command, contract byte, limit or deferred item changed.
- **2026-11-15 (execution promotion, t6).** The independent t5 verification PASS promoted
  every M3-A28–A34 row to `executed - passed`, filled §4, §5, §9 and the traceability,
  header and status text from the evidence, recorded the additive top-level help lines as a
  declared change in M3-A33 (O2), and recorded the residual help-text understatement (O4).
  No criterion, command, contract byte, limit or deferred item changed; the deferred scope
  in §10 and M3's incomplete status are unchanged. The verifier-owned
  [quality gates](quality-gates.md) block was not edited here.
