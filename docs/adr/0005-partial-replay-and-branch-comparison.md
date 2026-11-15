# ADR-0005: Partial Replay, Seek and Branch Comparison

- **Status:** Accepted (owner approved 2026-11-15); implemented and independently
  verified for the deterministic synthetic backend (t5 PASS, 2026-11-15)
- **Date:** 2026-11-15
- **Decision owner:** Project owner (approved 2026-11-15)
- **Approval evidence:** the project owner approved the staged M3 increment 2 AgentTeams
  plan `chronofork-m3-inc2` through the Web pre-run review UI; the approval committed at
  2026-11-15T12:35:03-03:00 (America/Fortaleza). The approved scope includes Option A:
  partial replay/seek by deterministic linear re-simulation from the embedded
  checkpoint, branch comparison, and no change to the canonical `.cfmovie` v1 format or
  emulation semantics. The approved staged plan covers the baseline, this Fase A
  decision record and acceptance plan, the library replay/compare work, the CLI
  `--from`/`--until` and `tas compare` work, independent verification and documentation
  promotion. This approval is a decision/plan approval; it does not by itself authorize a
  commit or push.
- **Supersedes:** none. This ADR extends [ADR-0004](0004-tas-movie-format.md) without
  changing its v1 container, limits, compatibility matrix, record/playback contract or
  verification records.
- **Implementation status:** Implemented and independently verified for the deterministic
  synthetic backend (t5 PASS, 2026-11-15). Every M3-A13–A17 row in the
  [increment 2 acceptance plan](../quality/m3-partial-replay-acceptance.md) is
  `executed - passed` against the t5 evidence; the canonical v1 format, decode/verify
  behavior and deferred scope are unchanged. The acceptance claim is limited to that
  backend and does not complete M3.
- **Related:** [ADR-0004: TAS movie format and deterministic
  replay](0004-tas-movie-format.md), [ADR-0003: durable session
  storage](0003-durable-session-storage.md), [execution
  model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [M3 first-increment acceptance
  plan](../quality/m3-tas-replay-acceptance.md), [M3 increment 2 acceptance
  plan](../quality/m3-partial-replay-acceptance.md), [research: TAS and
  debugging](../research/tas-and-debugging.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md),
  [milestones](../roadmap/milestones.md), [anti-slop
  policy](../quality/anti-slop-policy.md), [quality gates](../quality/quality-gates.md).

> **Implementation status.** The decision is **Accepted** (owner approval
> 2026-11-15T12:35:03-03:00) and the increment is **implemented and independently
> verified** for the deterministic synthetic backend (t5 PASS, 2026-11-15). The single
> ranged replay path, typed range validation, branch comparison and the CLI
> `play`/`verify --from/--until` plus `tas compare --movie/--against` are implemented and
> every M3-A13–A17 row in the
> [increment 2 acceptance plan](../quality/m3-partial-replay-acceptance.md) is
> `executed - passed` against the evidence under the ignored
> `.research/m3-tas-inc2/verification/` path. The canonical `.cfmovie` v1 format, the
> recorder, the full-range player/verifier and `chronofork tas record|play|verify` remain
> exactly as accepted in ADR-0004 and independently verified by M3-A1–A12 (M3-T4 PASS,
> 2026-11-15). The deferred scope in §8 is unchanged: checkpoint anchors/format v2,
> in-process rewind/state cache and interactive stepping, rerecord/branch editing,
> multi-controller/ports/analog/keyboard input, real-hardware input TAS, import/export
> adapters and HTTP/MCP/GUI/daemon remain unimplemented, and M3 is **not complete**. Do
> not cite this ADR as evidence for any deferred item.

## Context

M3 increment 1 delivered a canonical v1 movie format, record/playback of a per-step
input sequence on the deterministic synthetic backend, exact first-divergence
reporting, restart replay from the embedded checkpoint and the
`chronofork tas record|play|verify` CLI. The durable contract is
[ADR-0004](0004-tas-movie-format.md); its v1 container layout, limits and compatibility
matrix are not restated here. The relevant properties for this decision are:

- `crates/chronofork-sessions/src/movie.rs` owns exactly one v1 reader/writer
  (`Movie::from_bytes`, `Movie::to_bytes`), the validating constructor
  (`Movie::from_parts`), the recorder (`Movie::record`) and the replay verifier
  (`Movie::verify`). `Movie::verify` imports the embedded checkpoint into a fresh
  session through the existing `SessionManager::create_session_from_snapshot_bytes`
  path and compares every recorded `(outcome tick, state hash)` from the first step to
  the last. It takes no tick range.
- `Movie` exposes `start_tick()`, `end_tick()`, `step_count()`, `final_state_hash()`,
  `snapshot_bytes()` and `steps()`. The embedded checkpoint's state digest is decoded
  and kept internally for the report but has no public accessor.
- `ReplayReport` reports `start_tick`, `end_tick`, `steps`, `final_tick` and
  `final_state_hash`. `MovieError` is the single typed error enum for the movie/replay
  boundary.
- `crates/chronofork-cli/src/main.rs` holds the thin CLI adapter. `tas play` and
  `tas verify` accept exactly `--movie <FILE>`, share one `replay_movie` helper and
  print one `key=value` block; `record` accepts its own flags. The file has no in-file
  test module and exceeds the AS-14 800-non-test-line review trigger; `movie.rs` also
  exceeds it (counts measured in the working tree on 2026-11-15 are not restated here,
  per AS-13).
- ADR-0004 §12 deferred "frame advance, rewind and seek via checkpoints" and "replay
  resynchronization, partial replay, branch comparison tooling". At Fase A the M3
  milestone kept the same items unchecked, and
  [milestones](../roadmap/milestones.md) left "Frame advance, rewind and seek via
  checkpoints" open.
- Requirements at Fase A: `FR-TAS-001` was **Partial** (record/playback and verification
  only), `FR-EXEC-005` **Implemented** for exact first divergence, `FR-EXEC-007` (frame
  pacing, run loop and rewind) **Planned**, and `FR-FORK-006` compared *sessions* by tick
  and state digest, not movie expectations.
- Upstream research recorded in [TAS and debugging](../research/tas-and-debugging.md)
  §6.3 and §10.2 supports the strategy: BizHawk's TAStudio "does not have a
  random-access seek"; it loads the nearest cached state and replays forward, so "seek
  cost is proportional to distance from the nearest cached state". The research
  recommendation is "expose 'advance to tick from nearest snapshot' and make snapshot
  density a policy; do not promise O(1) seek". That is upstream-derived context, not a
  ChronoFork decision.

The tree already captures a pre-change baseline for this increment under the ignored
`.research/m3-tas-inc2/baseline/` path (the staged plan's baseline stage). This ADR
does not claim that stage completed.

## Problem

How should ChronoFork add partial replay/seek and branch comparison on the accepted v1
format so that:

1. the canonical `.cfmovie` v1 bytes, limits, compatibility rules, decode/verify
   behavior and every M3-A1–A12 result stay unchanged;
2. no emulation, backend payload, state-hash, scheduler or checkpoint semantics change,
   so the M1/M2/M3 evidence stays valid;
3. "replay only part of the movie" is deterministic and exact: every reported tick and
   state hash is an expectation that was actually verified or an observed value that
   was actually produced, never an interpolation or a guess;
4. an invalid range fails with a typed error **before** any session is imported or any
   step is executed, and never mutates a movie, session or file;
5. two movies can be compared without re-running emulation, with a report that names
   the first divergent step index, tick and hash on both sides, and that separates
   length and embedded-checkpoint mismatches from a per-step divergence;
6. metadata (`movie_id`, `name`, `parent_movie_id`) can never decide equality;
7. the increment stays small, std-only and verifiable on the deterministic synthetic
   backend without pretending to implement multi-controller input, real-hardware TAS,
   import/export adapters or a timeline UI; and
8. the replay and comparison logic plus the CLI TAS surface are split by
   responsibility so the existing modules do not keep growing past the AS-14 trigger,
   without introducing a second codec, reader or compatibility predicate.

## Alternatives

### Increment-scope alternatives

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Partial replay/seek plus branch comparison on v1, no format or emulation change** (recommended) | Reuses the accepted v1 container, the embedded canonical checkpoint and the per-step expectations; no new format version, limit, dependency or compatibility rule; directly addresses the deferred "seek" and "branch comparison" items; verifiable on the one acceptance backend | Ask is expressed as a range over existing records, so a seek to a late tick still re-simulates the prefix; no anchor cache, no in-process rewind buffer; compare is a record comparison, not a re-simulation check | **Recommended for this increment** |
| **B. Movie format / input-schema v2 with multi-controller, ports, analog and keyboard input** | Future-proof input schema; matches the roadmap item; one version bump could also add anchors | Changes `input_schema_version` (and probably `movie_format_version`) and therefore the container, limits, checksum coverage, rejection matrix and accepted M3-A1/A2/A5/A11 evidence; no backend consumes more than one input (the synthetic backend applies one `u32` mask per step and the Mega Drive `step` ignores input), so a v2 schema could not be verified against any backend and would be an unbacked capability; violates this increment's "no format change" constraint | **Deferred** to a separate decision with a backend that can consume the input |
| **C. Real Mega Drive controller input, making input TAS real** | Real-hardware input TAS; would address `FR-MD-010` and `FR-TAS-001` for hardware | The Mega Drive slice's `step(&mut self, _input)` ignores input; wiring controller ports changes emulation semantics, the `CFMDSTATE2` state body and hashes, scheduler/timing expectations and the M1 oracle differential evidence (currently bound to the existing semantics); needs its own ADR, owner approval, ROM/fixture design and evidence; it is not a movie-tooling task | **Deferred**; requires its own decision and does not fit this increment |
| **D. BizHawk BK2 import/export** | Interoperability with an established TAS ecosystem | Depends on option B's multi-port/frame model (BK2 logs per-port frames and per-core headers); BK2 is a ZIP archive, so a compression/archive dependency is added against AS-11 and the licensing policy; BizHawk licensing is heterogeneous and `AGENTS.md` says it must not be reused; rerecord counts are a heuristic, never a determinism proof ([research](../research/tas-and-debugging.md) §10.2); a second import/export surface needs its own validation and rejection contract | **Deferred**; future decision after B |

### Seek-strategy alternatives

| Strategy | Replay cost | Format cost | Verdict |
|---|---|---|---|
| **Linear deterministic re-simulation from the embedded checkpoint** | O(k) backend steps for a boundary at step `k`, independent of the `from` boundary; repeated seeks repeat the work | None: v1 already stores the checkpoint and every expectation; no new bytes, no version bump | **Chosen** |
| Periodic anchors inside the movie container | O(distance from nearest anchor) | Requires a v2 container: per-anchor snapshot bytes (backend-dependent; a real payload can be a large RAM/VDP image), new limits, checksum coverage, anchor validation and rejection rules, and re-verification of the v1 acceptance rows; makes "which anchor is authoritative" a new ambiguity | Deferred to a future format v2 decision |
| Sidecar anchor/checkpoint file next to the movie | Same as anchors | A second artifact and a resolution path; identity, integrity and portability rules would be a new protocol; the restart/portability property of the embedded-only v1 movie is weakened | Rejected for this increment |
| In-memory rewind/ring buffer in the session layer | Cheap backwards stepping while the process lives | New state-cache ownership, eviction, serialization and isolation policy; the research keeps rewind and snapshot persistence separate; not needed for exact partial replay or comparison; process-local only | Deferred to a future session-layer decision |
| No partial replay (status quo) | Full replay only | Does not meet the increment; the deferred item stays deferred | Rejected for this increment |

The cost/size tradeoff is explicit: linear re-simulation adds **zero bytes** to a movie
and needs **no** format change, at the price of executing the prefix again on every
invocation. Anchors would trade movie size and format complexity for lower replay time,
but they change the canonical artifact, need a version bump with new rejection tests,
and would have to be re-verified end to end. Since the increment's goal is exact
partial replay and comparison rather than seek performance, and since the v1 step bound
already caps the work, anchors are deferred with format v2. No performance target is
claimed for either strategy; per the [anti-slop policy](../quality/anti-slop-policy.md),
timing is never an equality gate.

### Comparison alternatives

| Strategy | What it answers | Verdict |
|---|---|---|
| **Compare recorded step records only** (`outcome_tick`, `state_hash` by index) | Do two recordings agree on the recorded expectation sequence? Where is the first differing step, tick and hash? | **Chosen** |
| Re-simulate both movies and compare observed states | Do two movies *currently* replay to the same observed states? | Rejected for this increment: doubles execution, needs two fresh imports/managers, cannot compare movies whose embedded checkpoints this build rejects at import, and reports observed-vs-observed instead of recorded-vs-recorded; verification already owns the observed-state question |
| Compare final state hash only | Do the two movies end at the same hash? | Rejected: cannot name the first divergent index or tick; the same reason final-hash-only verification was rejected in ADR-0004 §6 |
| Compare file bytes (SHA-256) only | Are the two files identical? | Rejected: a metadata-only name change would make files unequal while the execution is identical, so metadata would decide equality; no semantic location of a divergence |

## Decision

### 1. Scope and canonical home

This ADR is the canonical home for the increment-2 partial-replay, seek and
movie-comparison contract. [ADR-0004](0004-tas-movie-format.md) remains the canonical
home for the v1 container, byte layout, limits, compatibility matrix, record/playback
semantics and verification records; this ADR links to it and does not restate or amend
it. Architecture, requirements, roadmap and acceptance documents link here instead of
duplicating the range or comparison semantics.

Fase A changed no code; Fase B implemented the contract in the existing workspace without
adding a second movie reader/writer, snapshot parser, checksum rule or backend
compatibility predicate, and without adding a crate or dependency. The public names in
this ADR (`ReplayRange`, `ReplayOutcome`, `ComparisonReport`, range error categories)
describe the implemented shape; the semantics, typed rejection boundaries and equality
rules are the contract.

**Why a separate ADR instead of amending ADR-0004.** The v1 format does not change, but
this increment is a new behavioral decision with its own alternatives, deferred list,
acceptance rows and ownership, not a clarification of the container. ADR-0004 is
Accepted, implemented and bound to independently verified evidence; editing it in place
would make the verified contract ambiguous (which text was verified?) and would require
restating its record/playback and deferred sections. An append-only successor keeps
[ADR-0004](0004-tas-movie-format.md) as the single source of truth for the v1 bytes and
records the increment as its own status-tracked decision in the same index, so no
reader can confuse the accepted v1 contract with the newer increment-2 implementation. The owner approved this separate-ADR form with the staged plan (§10); if a formal
amendment is ever preferred, the content can move without changing semantics.

This increment does **not** complete the M3 milestone item "Frame advance, rewind and
seek via checkpoints". It adds forward-only partial replay/seek by re-simulation from
the embedded checkpoint, single-tick forward advance as a new invocation, rewind as a
new invocation, and branch comparison of recorded expectations. Checkpoint-anchored
seek, an in-process rewind buffer and interactive stepping remain deferred and the
milestone item stays unchecked until they have their own decision and evidence.

### 2. Seek strategy: linear deterministic re-simulation from the embedded checkpoint

Every ranged replay starts at the embedded checkpoint:

1. decode the movie (no execution);
2. validate the requested range against the decoded movie (typed errors, see §6);
3. import the embedded checkpoint into a **fresh** session through the existing
   `SessionManager::create_session_from_snapshot_bytes` path, so backend identity,
   version, payload version and configuration compatibility stay owned by the existing
   `validate_envelope` predicate;
4. apply the recorded inputs in order from step 0 through the step whose recorded
   outcome tick is the range's `until` boundary, comparing each executed step's outcome
   tick and state hash with that step's recorded expectation;
5. report the window result and never carry state anywhere.

Cost is O(k) backend steps, where `k` is the index of the `until` boundary, and is
independent of the `from` boundary. It is bounded by the v1 `MAX_STEPS` limit inherited
from ADR-0004 §2 and by the embedded checkpoint size already bounded by
`MAX_MOVIE_BYTES`; no new limit constant is introduced. A replay never resumes from a
previous invocation, never caches a checkpoint in memory across invocations and never
writes state back to the movie or to storage.

Consequences that must be stated in documentation and help text: a seek to a late tick
costs almost a full replay; repeated seeks repeat the work; this increment makes partial
replay *exact and expressible*, not *faster*. There is no O(1) or random-access seek and
no performance target.

### 3. Replay semantics: range, boundaries, prefix, frame advance and rewind

Let a movie have `n` recorded steps, `start_tick = s`, `end_tick = e` (`e = s` when
`n = 0`), and recorded step `i` with outcome tick `t_i` and state hash `h_i`. The v1
reader already guarantees `t_0 > s` and strictly increasing ticks (ADR-0004 §5), so
recorded ticks are unique.

**Aligned tick.** A tick is aligned when it is `s`, `e`, or some recorded `t_i`.

**Range.** A replay range is the pair `(from, until]`; both boundaries must be aligned
and `from <= until`. The replay **window** is the set of steps with
`from < t_i <= until`. The **prefix** is the set of steps with `t_i <= from`. The
default range is the full range `[s, e]`: unspecified `--from` means `s` and unspecified
`--until` means `e`, so every evaluated range is closed and bounded by the movie. A
supplied boundary is never left open.

**Validation** (all typed, all before importing a checkpoint, executing a step or
allocating anything proportional to the request):

| Requested boundary | Required typed category |
|---|---|
| `from < s` | before-start |
| `until > e` | after-end |
| `from > until` | inverted |
| `from` or `until` is not an aligned tick | unaligned, naming the offending tick |
| `from == until` on a movie with at least one step | empty window |
| `from == until == s == e` on a zero-step movie | valid; the full range of a zero-step movie |

`from < s` is only representable when `s > 0`; when `s = 0` no smaller `u64` exists, so
that input cannot occur and the invalid case is an unaligned or empty boundary instead.
A boundary is never rounded, clamped or reinterpreted. Validation order is bounds
(before-start, after-end), then inversion, then alignment, then emptiness; when more
than one violation applies, the first in that order is reported, and the executable
rows fix only the single-violation cases.

The exact error spelling is an implementation detail; the categories, their boundaries
and the "reject before execution" rule are the contract. The preferred placement is new
variants on the existing single `MovieError` enum, so the boundary keeps one typed
error type and the CLI keeps one match; a dedicated replay error that preserves
`MovieError` as its source is acceptable only if it does not create a second error
taxonomy.

**What is re-simulated.** Steps `0..=k`, where `k` is the index of `until`, applied in
order to the fresh imported session, each input exactly once. Prefix steps are executed
**and verified**: their recorded tick and state hash are compared exactly as the full
replay does. An implementation must not skip prefix verification, because skipping it
would let a movie with a divergent or corrupt prefix report a passing window while the
state at `from` is not the recorded state. Since the embedded checkpoint is the only
anchor, the prefix cannot be skipped physically either. The range therefore narrows
*what is reported* and bounds *how far execution runs*; it does not weaken
verification.

**What is reported** (`ReplayOutcome`):

- `range_from`, `range_until` — the validated boundaries;
- `prefix_steps` — steps executed before the window (`t_i <= from`);
- `window_steps` — steps in the window (`from < t_i <= until`);
- `replayed_steps` — `prefix_steps + window_steps`;
- `final_tick` — the observed tick at `until` (equal to `range_until` on success);
- `final_state_hash` — the observed state hash at `until` (equal to the recorded `h_k`
  on a clean replay);
- the movie-level `start_tick`, `end_tick` and total step count from `Movie`.

`Movie::verify` keeps its current signature and report: it delegates to the same ranged
path with `ReplayRange::full(movie)` and maps the outcome to the existing
`ReplayReport` (`steps` = `window_steps` = all steps, `final_tick`/`final_state_hash`
as today). There is exactly one replay loop and one checkpoint import; `play`, `verify`
and any ranged replay share it. The first mismatch is still the typed
`TickMismatch`/`Divergence` with the exact index, tick, expected hash and observed hash,
and replay still stops there — no resynchronization and no partial pass.

**Frame advance.** One backend step, which is exactly one tick in the synthetic
acceptance backend (the movie format does not promise a +1 quantum for other backends,
per ADR-0004 §5). A forward advance by one tick is expressed by a new replay whose
`from` boundary is the current boundary tick and whose `until` is the next recorded
outcome tick: the window then contains exactly one step, and the report names that
tick and its observed state hash. There is no interactive stepping loop, no
pause/resume, no UI, no state carried between invocations and no "advance without
re-simulating". An advance request that does not name a recorded outcome tick is
rejected by the unaligned rule; there is no fractional or interpolated tick.

**Rewind.** A new invocation with a smaller `until` (or an earlier `from`/`until`
pair). It re-simulates from the embedded checkpoint in a fresh session and lands on the
earlier recorded tick. No state is carried between invocations, no session is reused and
there is no in-process rewind buffer; a second invocation's result equals a first-ever
invocation at the same range. In-process rewind (ring buffer, cached states) is
deferred.

**Isolation and non-mutation.** A valid replay registers exactly one fresh imported
session in the caller's manager and touches no pre-existing session, no movie and no
file; an invalid range registers no session and executes no step. The movie is never a
state source of truth: recorded expectations are compared, never applied as state.

### 4. Comparison semantics: recorded expectations, never re-simulation

Branch comparison answers one question: do two movies agree on their **recorded**
expectation records, and if not, where do they first differ? It is a pure value
operation over two decoded movies:

- It takes the two `Movie` values only. It takes no `SessionManager`, imports no
  checkpoint, calls no backend, executes no step and allocates nothing proportional to
  the step count beyond the report.
- It does not call `validate_envelope` or the import path, and it does not decide
  backend or configuration compatibility. Two movies that decode structurally can be
  compared even if this build would reject one of their checkpoints at import. That is
  by contract: compare is a comparison of recorded artifacts, not a verification of
  this build.
- Malformed movie bytes still fail at decode with the existing typed `MovieError`
  before any comparison happens.

Checks, in this order:

1. **Embedded checkpoint identity.** Compare `start_tick` and the embedded checkpoint's
   state hash (the checkpoint hash is the movie's start-state digest, already decoded by
   `Movie::from_parts`/`Movie::from_bytes`). A difference is a
   `checkpoint_mismatch` report carrying both start ticks and both checkpoint state
   hashes. A per-step comparison is not attempted: step indices from different starting
   states are not comparable.
2. **First divergent recorded step.** Walk the common index range in ascending order
   and compare `(outcome_tick, state_hash)`. The first index where either field differs
   is reported as the first divergence with `index`, `tick_a`, `tick_b`, `hash_a`,
   `hash_b` and both movie step counts. Because both fields are reported, a tick-only,
   hash-only or combined difference is distinguishable. No later index may be reported
   instead.
3. **Length mismatch.** If the common prefix is equal and the step counts differ, report
   a separate length mismatch with `common_steps`, `steps_a` and `steps_b`. A length
   difference never replaces a first divergence: if a divergence exists in the common
   prefix, the divergence is the reported verdict and the two step counts remain visible
   in the report fields.
4. **Identical.** Otherwise the movies are `identical`.

**What never decides equality.** `movie_id`, `name` and `parent_movie_id` are not
compared; two movies that differ only in metadata compare identical. Wall-clock time,
file modification time, file name and file path are never inputs. Input masks are not
equality fields in this increment: two movies whose recorded `(outcome tick, state
hash)` sequences are equal compare identical even when their inputs differ, because the
state trajectory is the canonical comparison field (`FR-EXEC-003`/`FR-EXEC-004`).
Comparing input masks as a separate diagnostic would need its own report contract and is
deferred.

**Compare is not verification.** A comparison cannot detect a build whose emulation
changed: two movies recorded under the old behavior would still compare identical to
each other. `verify`/ranged replay remains the re-simulation path, and the CLI help,
docs and any future interface must not present `compare` as a determinism or
compatibility check. A compare PASS says only that two recorded expectation sequences
agree.

**Report and exit.** The library returns a typed comparison report with a verdict of
`identical`, `checkpoint_mismatch`, `step_divergence` or `length_mismatch`, plus the
fields above; a mismatch is a normal outcome, not an `Err`. The CLI exits 0 for
`identical` and 1 for every mismatch, printing the report block on stdout; malformed
input remains a typed CLI error on stderr with empty stdout.

### 5. Architecture, module ownership and the AS-14 split

- **One v1 codec, one snapshot parser, one compatibility predicate.** The reader/writer
  stays exactly one in `chronofork_sessions::movie`. The embedded checkpoint is still
  encoded only by `Snapshot::to_bytes` and decoded only by `Snapshot::from_bytes`;
  backend compatibility is still decided only by `validate_envelope` through the
  existing import path. This increment adds no parser, checksum rule or predicate.
- **New `crates/chronofork-sessions/src/replay.rs`.** It owns the replay range type, the
  typed range validation, the single ranged replay entry point and outcome, and the
  comparison report and entry point. `Movie::verify` delegates to the ranged path with
  the full range and maps to the existing `ReplayReport`, so the replay loop and
  checkpoint import exist once. `movie.rs` keeps decode/encode/record and the
  validating constructor; it may gain one read-only accessor for the embedded
  checkpoint state hash so compare does not decode the snapshot a second time.
- **New `crates/chronofork-cli/src/tas.rs`.** The complete TAS surface (record, play,
  verify, compare, their flag parsing, movie file I/O with its existing bound, block
  writing and TAS help) moves there; `main.rs` keeps command dispatch and the shared
  flag helpers (or moves them to one small shared module) without duplicating any
  parser. This is a split by responsibility (movie format vs replay/compare; CLI
  dispatch vs TAS adapter), not a cosmetic line-count split. It is intended to bring
  both modules below the AS-14 review trigger; the measured counts belong to evidence,
  not to this durable document. No new crate.
- **Tests at the lowest useful layer** (AS-08): range validation and comparison
  semantics in `chronofork-sessions` unit/integration tests; CLI flag parsing, block
  format and exit codes in `chronofork-cli` process tests; do not re-assert session or
  emulation semantics in the CLI.
- **No new dependency, no `unsafe`, no new `#[allow]`/`#[ignore]`, no deleted or
  relaxed test.** The standard library and existing workspace crates are sufficient.

### 6. Compatibility, limits and typed errors

- **v1 is unchanged and stays valid.** No `movie_format_version` or
  `input_schema_version` change, no new field, no checksum-coverage change, no limit
  change, no migration. Decode order, bounds checks, corruption/truncation/version
  rejection and the embedded-checkpoint rules in ADR-0004 §7–§8 are untouched. A movie
  produced by increment 1 decodes, verifies and compares without modification; a movie
  produced after this increment is byte-identical for the same inputs and metadata.
- **Decode and full-range verify are unchanged.** The existing unit, integration and
  CLI targets keep their assertions; the full-range report fields and their meaning are
  preserved through the delegation.
- **Bounds.** A ranged replay executes at most `MAX_STEPS` recorded steps (inherited,
  not increased) and reads no bytes beyond the already-bounded movie file. Comparison is
  O(min(n, m)) over already-decoded records. No new limit constant, no allocation before
  validation, no unbounded buffer.
- **Typed range errors.** The five categories in §3 are distinct typed variants with
  named fields; no stringly error, no flattened `String`, no panic, no silent clamp.
  Range validation is pure arithmetic over decoded ticks and runs before the checkpoint
  import, so an invalid request cannot create a session, execute a step or mutate
  anything.
- **Typed comparison report.** A mismatch is a typed report value, not an error; decode
  and I/O failures stay typed `MovieError`/CLI errors. No error taxonomy is duplicated.
- **No emulation, state-hash, scheduler, backend payload, storage or existing CLI
  behavior change.** Default `tas play|verify` output stays exactly as accepted in
  M3-A7; new fields appear only when a range is requested (§7).

### 7. CLI contract shape (planned)

```text
chronofork tas play    --movie <FILE> [--from <TICK>] [--until <TICK>]
chronofork tas verify  --movie <FILE> [--from <TICK>] [--until <TICK>]
chronofork tas compare --movie <FILE> --against <FILE>
```

- `--from` and `--until` are replay-only flags: `record` and `compare` reject them as
  unknown for those subcommands. `--against` is a compare-only flag: `record`, `play`
  and `verify` reject it as unknown. `play` and `verify` keep `record`'s
  `--steps`/`--input` rejection unchanged.
- An unspecified `--from` means the movie's start tick and an unspecified `--until`
  means its end tick. A supplied value must be the start tick, the end tick or an
  existing recorded outcome tick; anything else is a typed CLI failure (exit 1,
  stderr), never a clamp.
- **Default invocation output is unchanged.** With no range flags, `play`/`verify`
  print exactly the M3-A7 blocks (`record=playback` / `record=verification`,
  `verified=1` for verify, `start_tick`, `end_tick`, `steps`, `final_tick`,
  `final_state_hash`) and keep the existing divergence block behavior.
- **Ranged invocation output** prints the same block plus `range_from`, `range_until`,
  `prefix_steps`, `window_steps` and `replayed_steps`, in the same `key=value` style.
  `start_tick`/`end_tick`/`steps` remain movie-level facts; `final_tick` is the
  `range_until` boundary and `final_state_hash` is the observed hash there. A
  divergence, tick mismatch or failure during the prefix is reported by the existing
  typed behavior, not as a window pass.
- **`compare` output** is one block beginning `record=comparison` with `identical=1` or
  `identical=0`; on mismatch it carries the applicable report fields from §4: a
  checkpoint mismatch prints both start ticks and both checkpoint state hashes, a step
  divergence prints `divergence_index`, `tick_a`, `tick_b`, `state_hash_a`,
  `state_hash_b` and both step counts, and a length mismatch prints `common_steps`,
  `steps_a` and `steps_b`. Exit 0 for identical, 1 for any mismatch. `--movie` and
  `--against` are both required, must name two files (the same file may be passed
  twice; it compares identical), and duplicate, missing, third-argument or unknown
  flags are typed errors. This is the owner-specified CLI shape; the semantics (exactly
  two explicit movie inputs, no third, no positional ambiguity) are the contract.
- Output is stdout-only `key=value`; diagnostics go to stderr as `chronofork: <error>`;
  exit 0 on success, 1 on any typed failure; no timestamp, host name, path or network
  content in valid stdout; no panic. Help documents the new surface (including
  `--movie`/`--against` for `compare`), states that seek is re-simulation from the
  embedded checkpoint (not O(1) random access), and states that
  in-process rewind, checkpoint anchors, rerecord, multi-controller input,
  real-hardware input TAS and import/export remain unimplemented.

### 8. Deferred work and stated limitations

**In scope for this increment:** ranged partial replay with typed range validation on
the deterministic synthetic backend; forward single-tick advance and rewind expressed as
new replay invocations; branch comparison of recorded expectations; the CLI flags and
`tas compare`; executable tests; honest documentation of what remains.

**Deferred, not implemented and never to be claimed in this increment:**

- rerecording and branch editing; in-place movie editing; rerecord counters;
- periodic checkpoints/anchors inside a movie, anchor-based O(1)/O(log n) seek and the
  format v2 that would carry them;
- a sidecar checkpoint/anchor artifact or a checkpoint-reference movie mode;
- an in-process rewind/ring buffer, state-cache/eviction policy, interactive stepping,
  pause/resume and any timeline or UI surface;
- multi-controller, multiple ports, analog axes, keyboard events and device-specific
  peripherals (a future input schema version);
- real-hardware input TAS: the Mega Drive `step` ignores input, and changing that is an
  emulation-semantics change under option C;
- BizHawk BK2 or any other TAS import/export adapter;
- input-mask comparison as an equality or diagnostic field in `compare`;
- parallel or threaded replay, process isolation of replay, and any performance or
  capacity target;
- compression, encryption, authentication or deduplication;
- HTTP API, MCP adapter, GUI and the Stage 6 daemon;
- cross-backend canonicalization, cross-platform determinism or any hardware-accuracy
  claim.

Stated limitations: partial replay costs O(k) execution from the embedded checkpoint and
repeats that cost on every invocation; `compare` is a record comparison and is not a
verification; metadata never decides equality; the acceptance claim is synthetic-backend
only; a movie from another backend version or configuration is still rejected at replay
import; M3 is **not complete**.

### 9. Implementation, evidence and status promotion

1. The library and CLI paths above were implemented under the approved staged plan and
   the executable acceptance plan
   [M3 partial-replay acceptance](../quality/m3-partial-replay-acceptance.md) rows
   M3-A13–M3-A17 were executed by the independent verifier and are `executed - passed`
   (t5 PASS, 2026-11-15).
2. The v1 format, decode/verify behavior, the M3-A1–A12 evidence and the M2/M1 evidence
   stayed intact; the full gate, both demos, the M2 target matrix and the
   serialized-state probe are identical to the increment baseline.
3. The execution evidence promotion (acceptance rows,
   [execution model](../architecture/execution-model.md), requirement statuses, milestone
   and roadmap annotations, the documentation index and this ADR's status) is recorded
   from the t5 verification artifacts, not from implementation claims. The
   verifier-owned dated quality-gates block remains the verifier's record and is not
   edited here.
4. The owner approved the staged plan and this decision on 2026-11-15; the approval
   timestamp is recorded in the header. Any implemented/verified claim is limited to the
   t5-verified synthetic-backend scope and the artifact set in the Evidence section; a
   later edit to an implementation path invalidates the equality claims until
   re-verified.

### 10. Owner decisions (approved 2026-11-15)

The project owner approved the staged M3 increment 2 plan `chronofork-m3-inc2` through
the Web pre-run review UI at 2026-11-15T12:35:03-03:00 (America/Fortaleza). The approved
decisions are:

1. Option A: partial replay/seek and branch comparison on v1, with the v1 format,
   emulation semantics, state hashes and schedules unchanged; options B, C and D are
   deferred as recorded in the alternatives.
2. Seek by linear deterministic re-simulation from the embedded checkpoint, O(k) in the
   `until` boundary index `k`, bounded by the existing `MAX_STEPS`; no periodic anchors, no
   sidecar, no format-version bump, and no seek-performance promise.
3. One library replay path parameterized by a tick range that always imports the
   embedded checkpoint into a fresh session; default full range; boundaries must be the
   start/end tick or a recorded outcome tick; typed rejection of before-start,
   after-end, inverted, unaligned and empty ranges; prefix always executed and verified.
4. Frame advance means one backend step (one tick on the synthetic backend) expressed by
   a new replay invocation with the next recorded boundary; rewind means a new invocation
   with an earlier boundary; no state is carried between invocations and no in-process
   rewind exists yet.
5. Compare reads recorded step records only and never re-simulates: checkpoint identity
   (start tick and checkpoint state hash) and length mismatch are reported separately
   from the first `(outcome tick, state hash)` divergence; metadata and input masks never
   decide equality; exit 0 identical, exit 1 any mismatch; `verify` remains the
   re-simulation path.
6. Exactly one v1 reader/writer remains in `chronofork_sessions::movie`; the ranged
   replay and comparison live in a new `crates/chronofork-sessions/src/replay.rs` with
   `Movie::verify` delegating; the CLI TAS surface moves to
   `crates/chronofork-cli/src/tas.rs` with `main.rs` keeping dispatch, to address the
   AS-14 trigger without duplicating the codec.
7. v1 compatibility, bounds and corruption behavior stay unchanged; no new dependency,
   no `unsafe`, no new `#[allow]`/`#[ignore]`, no emulation or storage change; default
   `play`/`verify` output is unchanged.
8. Everything in §8 stays deferred; this increment does not complete M3 and does not
   tick the milestone item "Frame advance, rewind and seek via checkpoints".
9. A separate ADR-0005 that extends ADR-0004 rather than an amendment to ADR-0004, for
   the reasons in §1; the v1 container, limits, compatibility matrix and verified
   record/playback contract are unchanged.

## Rationale

- **Leaving v1 untouched preserves every verified byte.** The format was accepted and
  independently verified; a change would invalidate its evidence and force a
  re-verification of the container, limits and rejection rows. Partial replay needs no
  new bytes: the embedded checkpoint plus per-step expectations already determine every
  recorded tick exactly.
- **Re-simulation is the only exact strategy without anchors.** The checkpoint is the
  only authoritative state in the movie. Anchors would need a format version, new
  rejection rules and a policy for which anchor wins; the research evidence confirms
  that seek is replay, not random access, and that O(1) seek must not be promised.
- **The range narrows the report, not the verification.** Because the prefix cannot be
  skipped physically, and skipping its verification would be unsound, the range is
  defined as a reporting window with an `until` work bound. That is simple to state, to
  test and to explain.
- **Typed rejection before import is non-negotiable.** A user-supplied tick must never
  drive execution or allocation before it is validated against the decoded movie; a
  rejected request must leave the manager and the movie untouched.
- **Compare must not pretend to verify.** Verification recomputes state; comparison
  diffs recorded expectations. Mixing them would double the execution cost, conflate two
  different questions and make the report ambiguous. Keeping compare pure also lets it
  work on movies whose checkpoint this build would reject.
- **Metadata cannot be allowed to decide equality.** A rename or a new logical id is not
  an emulation difference; treating it as one would make branch comparison useless for
  its purpose.
- **The split follows the responsibility boundary.** `movie.rs` owns the canonical
  format; replay/compare are policy over decoded movies; the CLI is a thin adapter. The
  split keeps one codec and one replay loop while reducing the largest modules below the
  AS-14 review trigger.

## Consequences

Positive:

- A movie can be replayed, verified and reported over an exact tick window without
  changing the canonical format.
- Forward single-tick advance and rewind become expressible with deterministic,
  reproducible results and no hidden state.
- Two branches can be compared quickly and precisely, with a first-divergent index,
  tick and hash, plus separate length and checkpoint-identity reports.
- Invalid ranges and malformed input fail closed with typed errors before any execution.
- The existing v1, M2 and M1 evidence remains valid; no format migration exists.
- The replay/comparison logic and the CLI TAS surface get clear homes, and the v1 codec
  remains single-sourced.

Negative / costs:

- Repeated seeks repeat execution; a late seek is close to a full replay.
- `compare` cannot answer whether this build still reproduces a recording; users must
  run `verify` for that.
- Two more concepts (aligned boundary, window vs prefix) must be documented and tested.
- The CLI grows two flags and a subcommand, and the default-output compatibility rule
  adds conditional reporting code.

Risks / limitations:

- The acceptance claim is synthetic-backend only; real-hardware input TAS is still
  absent because the Mega Drive `step` ignores input.
- Anchors and a rewind buffer remain absent; any future addition needs a new decision,
  a format or session-layer version and new rejection tests.
- A movie from another backend version or configuration is still rejected at replay
  import; compare can still compare its records, which is not an accuracy claim.
- Compare PASS is not a determinism proof, and this ADR must not be described as one.
- This ADR is Accepted and verified only for the t5-verified synthetic-backend scope: it
  asserts no hardware accuracy, no deferred capability and no M3 completion.

## Status

Accepted. The project owner approved the staged M3 increment 2 plan and the decisions in
§10 on 2026-11-15 (approval committed at 2026-11-15T12:35:03-03:00, America/Fortaleza).
The increment was implemented under that plan and independently verified for the
deterministic synthetic backend: t5 executed M3-A13–A17 and returned PASS on 2026-11-15
(§9 and Evidence). The v1 format, decode/verify behavior and deferred scope are unchanged,
the deferred items in §8 remain unimplemented and M3 is **not complete**. No commit or
push has been made.

## Evidence

- **Owner approval:** staged plan `chronofork-m3-inc2` approved through the Web pre-run
  review UI; approval committed 2026-11-15T12:35:03-03:00 (America/Fortaleza). The
  approved scope is Option A with no v1 format or emulation-semantics change.
- **Accepted v1 contract this ADR extends, not restates:**
  [ADR-0004](0004-tas-movie-format.md) §2–§12 (container, limits, compatibility matrix,
  record/playback, verification records, deferred list) and its executed acceptance
  record [M3-A1–A12](../quality/m3-tas-replay-acceptance.md) (M3-T4 PASS, 2026-11-15).
- **Executed verification (increment 2):** independent t5 verifier PASS on 2026-11-15 for
  the uncommitted increment-2 tree at HEAD
  `c6c502de54932459a1d3bc0cd0ada10ceba97043` (plus the increment-2 working-tree paths;
  worktree diff sha256 `70ccf322…dfd7`). All M3-A13–A17 rows executed and passed; the full
  gate, M2 and increment-1 v1 regression, both demos and the checkpoint probe are green.
  Evidence: `.research/m3-tas-inc2/verification/` (`SUMMARY.txt`,
  `target-matrix.txt`, `A13-*.txt`–`A17-*.txt`, `A13-A17-execution-record.txt`,
  `acceptance-checks.txt`, `logs/`, `targets/`, `cli/`, `demos/`, `probes/`,
  `MANIFEST.sha256`); pre-change baseline `.research/m3-tas-inc2/baseline/`; dated
  gate-result block in [quality gates](../quality/quality-gates.md). The verifier's CLI
  harness and public-API probe are evidence tooling under the ignored path, not product
  code.
- **Increment-2 implementation paths (t5 verified):**
  `crates/chronofork-sessions/src/replay.rs` (single ranged replay path
  `ReplayRange`/`ReplayOutcome`, typed range validation, `ComparisonReport`/
  `ComparisonVerdict`, pure `compare_movies`),
  `crates/chronofork-sessions/src/movie.rs` (`Movie::verify` delegation and
  `checkpoint_state_hash`), `crates/chronofork-sessions/src/lib.rs`,
  `crates/chronofork-sessions/tests/replay.rs`,
  `crates/chronofork-cli/src/tas.rs` (TAS surface, range flags and compare),
  `crates/chronofork-cli/src/main.rs` (dispatch and shared helpers), and
  `crates/chronofork-cli/tests/tas.rs`. One v1 reader/writer and one replay loop remain;
  no new crate, dependency or format version.
- **Current implementation paths (increment 1, independently verified):**
  `crates/chronofork-sessions/src/movie.rs` (one `Movie::from_bytes`/`to_bytes`,
  `Movie::from_parts`, `Movie::record`, `Movie::verify`, `ReplayReport`, `MovieError`);
  `crates/chronofork-sessions/src/snapshot.rs`;
  `crates/chronofork-sessions/src/manager.rs`
  (`create_session_from_snapshot_bytes`);
  `crates/chronofork-backend-api/src/snapshot.rs` (`validate_envelope`);
  `crates/chronofork-test-backend/src/lib.rs` (`APPLY_INPUT`, one input per step, one
  tick per step); `crates/chronofork-megadrive/src/backend.rs` (`fn step(&mut self,
  _input: InputFrame)` ignores input);
  `crates/chronofork-cli/src/main.rs` (`run_tas`, `tas_record`, `tas_play`,
  `tas_verify`, `replay_movie`, `write_replay_block`, `print_tas_help`);
  `crates/chronofork-sessions/tests/movie.rs`; `crates/chronofork-cli/tests/tas.rs`.
- **AS-14 review trigger:** both `crates/chronofork-sessions/src/movie.rs` (non-test
  section) and `crates/chronofork-cli/src/main.rs` exceed the 800-non-test-line trigger
  in [anti-slop policy](../quality/anti-slop-policy.md) §4.4; counts are deliberately not
  restated in this durable document (AS-13).
- **Research basis for seeking:** [TAS and debugging](../research/tas-and-debugging.md)
  §6.3 (TAStudio has no random-access seek; it loads a state and replays forward; seek
  cost is proportional to distance from the cached state) and §10.2 ("Seeking is replay,
  not random access"; do not promise O(1) seek).
- **Requirements/milestone state (promoted with t5):** `FR-TAS-001` **Partial**
  (record/playback and verification, plus ranged replay/seek, forward single-tick advance
  and rewind by deterministic linear re-simulation and branch comparison of recorded
  records; rerecord/branch editing, anchors/format v2, in-process rewind, multi-controller
  and hardware input deferred), `FR-EXEC-005` **Implemented**, `FR-EXEC-007` **Planned**,
  `FR-FORK-006` session compare only. [Milestones](../roadmap/milestones.md) and the
  [roadmap](../roadmap/roadmap.md) now distinguish the delivered linear re-simulation path
  from the deferred checkpoint-anchored path; M3 remains incomplete.
- **Acceptance plan for this increment:**
  [M3 partial-replay acceptance](../quality/m3-partial-replay-acceptance.md) (rows
  M3-A13–A17 `executed - passed`, t5, 2026-11-15). Raw evidence is under the ignored
  `.research/m3-tas-inc2/verification/` path; the increment baseline under
  `.research/m3-tas-inc2/baseline/`.
- **Tree state:** the implementation and contract docs are uncommitted at HEAD
  `c6c502de54932459a1d3bc0cd0ada10ceba97043` (2026-11-15); the t5 evidence binds that
  working tree, not a commit. The pre-existing top-level `README.md` working-tree
  modification is excluded from increment scope and unchanged. No commit or push has been
  made.

## Change control

- **2026-11-15 (execution promotion, t6).** The t5 independent verification PASS promoted
  the implementation status, Status section and Evidence to implemented and verified for
  the deterministic synthetic backend. No decision, byte format, range/comparison
  semantics, CLI shape or deferred item changed; the deferred scope in §8 and M3's
  incomplete status are unchanged. The verifier-owned dated quality-gates block was not
  edited here.
- **2026-11-15 (docs correction, t9).** The compare CLI flags in §7 were corrected from
  `--movie-a`/`--movie-b` to the owner-specified `--movie <FILE> --against <FILE>`. Only
  the flag spelling changed: both inputs are still required, the same file is still
  allowed, and duplicate/missing/third-argument/unknown-flag handling, comparison
  semantics, report fields, precedence, exit codes and the deferred list are unchanged.
  No replay or comparison decision changed; no acceptance row is executed or promoted.
