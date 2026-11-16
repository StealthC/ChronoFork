# ADR-0008: Rerecord and Branch Editing

- **Status:** **Accepted** (owner-approved 2026-11-15T21:09:29-03:00,
  America/Fortaleza; 2026-11-16T00:09:29Z); **implemented and independently verified for
  the deterministic synthetic backend** (t10 repair-round-2 PASS, 2026-11-16). M3 remains
  incomplete.
- **Date:** 2026-11-15
- **Decision owner:** Project owner (approved 2026-11-15)
- **Approval evidence:** The project owner explicitly approved ADR-0008 as proposed with
  the statement "Aprovar ADR-0008 como proposto (Recomendado)" at
  2026-11-15T21:09:29-03:00 (America/Fortaleza), 2026-11-16T00:09:29Z. The approval covers
  the proposed contract as recorded in this ADR: verified prefix plus deterministic suffix
  re-simulation into a new artifact; full `InputFrame` edits at the library layer and
  port-0 edits from the CLI; zero-based `--truncate-after` requiring at least one dropped
  step; no stored rerecord counter and no format v3 (per-invocation diagnostics plus
  immediate-parent provenance); branch output id defaulting to source `+ 1` with
  `parent_movie_id` equal to the source and no inherited name; default `keep-prefix`
  anchors with explicit `drop`/`rebuild` and an unchanged embedded checkpoint; v1 → v1
  and v2 → v2 reject-only compatibility; the exact `tas edit` flags, output block, atomic
  publish and source-alias guard; new `chronofork_sessions::edit` and
  `chronofork_cli::tas_edit` modules; and no new dependency, `unsafe`, `#[allow]` or
  `#[ignore]`. The approval records the decision only; it does not assert implementation
  or verification, and the deferred scope is unchanged.
- **Supersedes:** none. This ADR extends [ADR-0004](0004-tas-movie-format.md),
  [ADR-0005](0005-partial-replay-and-branch-comparison.md),
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md) and
  [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md) without changing any
  movie byte, format version, input-schema version, limit, anchor rule, emulation
  semantic, state hash, storage layout or previously verified behavior.
- **Implementation status:** **Implemented and independently verified** for the
  deterministic synthetic backend (t10 repair-round-2 PASS, 2026-11-16). The contract is
  implemented in `crates/chronofork-sessions/src/edit.rs`,
  `crates/chronofork-sessions/tests/edit.rs`, `crates/chronofork-cli/src/tas_edit.rs` and
  `crates/chronofork-cli/tests/tas_edit.rs`, with `main.rs`/`tas.rs` wiring. The companion
  [M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md) rows M3-A35–M3-A44
  are `executed - passed` against
  `.research/m3-rerecord/verification-round-2/`. The round-1 FAIL findings F1/F2/F3/F5
  and gaps F6 were repaired and re-verified; the deferred scope in §11 is unchanged and
  M3 is **not complete**.
- **Verification evidence:** independent t10 repair-round-2 PASS (2026-11-16) on HEAD
  `3f76d5f` plus the repaired uncommitted increment paths. Raw evidence is the ignored
  `.research/m3-rerecord/verification-round-2/` path (`SUMMARY.txt`, per-row
  `rows/A35-*.txt`–`A44-*.txt`, `target-matrix.txt`, `gate/`, `targets/`, `cli/`, `edit/`,
  `demos/`, `probes/`, `audit/`, `tree/` and `MANIFEST.sha256`); round-1 evidence is
  preserved under `.research/m3-rerecord/verification/`. No commit or push was made.
- **Related:** [execution model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [roadmap](../roadmap/roadmap.md), [M3 first-increment acceptance
  plan](../quality/m3-tas-replay-acceptance.md), [M3 increment-2 acceptance
  plan](../quality/m3-partial-replay-acceptance.md), [M3 format-v2 acceptance
  plan](../quality/m3-format-v2-acceptance.md), [M3 rewind acceptance
  plan](../quality/m3-rewind-acceptance.md), [M3 rerecord acceptance
  plan](../quality/m3-rerecord-acceptance.md), [research: TAS and
  debugging](../research/tas-and-debugging.md), [anti-slop
  policy](../quality/anti-slop-policy.md), [quality gates](../quality/quality-gates.md).

> **Status and honesty statement.** This ADR is **Accepted** (owner-approved
> 2026-11-15T21:09:29-03:00, America/Fortaleza; 2026-11-16T00:09:29Z) and **implemented
> and independently verified** for the deterministic synthetic backend (t10
> repair-round-2 PASS, 2026-11-16). The functional edit command `chronofork tas edit`, the
> library `edit` module, the suffix re-simulation path, the branch artifact and every
> acceptance row described below are backed by the executed rows M3-A35–A44 and the
> round-2 artifacts. The round-1 FAIL findings F1/F2/F3/F5 and gaps F6 were repaired and
> re-verified; the round-1 evidence is preserved.
> The accepted and independently verified contracts remain
> [ADR-0004](0004-tas-movie-format.md) (v1 movie and replay),
> [ADR-0005](0005-partial-replay-and-branch-comparison.md) (ranged replay, seek and
> comparison), [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md) (v2
> anchors and schema-2 input) and
> [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md) (in-process rewind and
> `tas studio`). The acceptance claim is limited to the deterministic synthetic backend on
> Linux: real-hardware input TAS, import/export, timeline editing, persisted counters,
> in-place editing and O(1) seek remain unimplemented, and M3 is **not complete**.

## Context

Read on 2026-11-15 in the clean `agent/m3-complete` checkout at HEAD
`3f76d5f` (`3f76d5ffee680d2c7271c7c1d81d8c5a44055675`) before drafting. The pre-change
baseline for this increment is already captured under the ignored
`.research/m3-rerecord/baseline/` path (Git state, environment, gate logs, per-target
lists, both demos and fixtures; captured 2026-11-15T23:49:31Z on the same HEAD and clean
tree). That baseline is a precondition for a later implementation stage, not evidence for
this ADR.

The accepted, implemented and independently verified baseline this design builds on:

- **v1 movie container and replay** ([ADR-0004](0004-tas-movie-format.md), M3-A1–A12
  PASS). `chronofork_sessions::movie` owns exactly one public reader/writer
  (`Movie::from_bytes`, `Movie::to_bytes`), the validating v1 constructor
  `Movie::from_parts`, the recorder `Movie::record` and the full-range verifier
  `Movie::verify`. A v1 movie is one canonical `.cfmovie` file with metadata, the
  verbatim canonical `Snapshot::to_bytes()` embedded checkpoint, 44-byte step records
  `(outcome_tick, u32 input, state_hash)` and a trailing SHA-256. `Movie` exposes
  `meta()`, `format_version()`, `input_schema_version()`, `start_tick()`, `end_tick()`,
  `step_count()`, `snapshot_bytes()`, `steps()`, `final_state_hash()` and
  `checkpoint_state_hash()`; `MovieStep` exposes `outcome_tick()`, `input()` and
  `state_hash()`.
- **v2 container, anchors and schema-2 input** ([ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md),
  M3-A18–A27 PASS). A v2 `CFMOVIE2` file adds an ordered anchor table and 72-byte step
  records. `Movie::from_v2_parts` is the single v2 validating constructor and
  `Movie::record_v2` the single v2 recorder. `MovieAnchor::new(step_index,
  snapshot_bytes)` parses an anchor exactly once with `Snapshot::from_bytes`, keeps the
  index, verbatim bytes, tick and state hash, and the constructor/reader enforce
  strictly increasing indices, index range and the anchor snapshot tick/state-hash
  binding to the recorded step. The embedded initial checkpoint is the implicit state
  before step 0; the explicit anchor table starts after recorded steps and never repeats
  it.
- **Ranged replay, seek and comparison** ([ADR-0005](0005-partial-replay-and-branch-comparison.md),
  M3-A13–A17 PASS). `chronofork_sessions::replay` owns `ReplayRange`, typed range
  validation, the single replay loop and the pure `compare_movies` function. Replay
  imports the embedded checkpoint (or a selected v2 anchor for a v2 ranged replay) into
  a fresh session through the existing path and compares every executed step's observed
  outcome tick and state hash with the recorded expectation; the first mismatch is the
  typed `TickMismatch`/`Divergence`, never a resynchronization. Comparison diffs
  recorded expectations only, never re-simulates, and never lets metadata decide
  equality.
- **In-process rewind and interactive stepping** ([ADR-0007](0007-in-process-rewind-and-interactive-stepping.md),
  M3-A28–A34 PASS). `chronofork_sessions::player` and `chronofork_cli::studio` add a
  bounded, process-local state cache and a stdin stepping surface. Neither is extended
  by this design; the player replays a decoded movie and does not edit it.
- **Session and manager path** (`crates/chronofork-sessions/src/session.rs`,
  `manager.rs`). `SessionManager::create_session_from_snapshot_bytes` is the one import
  path: it decodes the canonical envelope, creates a fresh backend, restores it through
  the existing `validate_envelope` predicate and registers a new root session.
  `SessionManager::snapshot`, `snapshot_record` and `snapshot_bytes` are the existing
  checkpoint capture path used by v2 anchor recording. There is no second snapshot
  parser, compatibility predicate or import path.
- **Input frame** (`crates/chronofork-core/src/input.rs`). `InputFrame` carries four
  `u32` digital ports (port 0 is the v1 backend-defined button mask), four signed
  `i16` axes and one `u64` keyboard bitmap. `new`, `buttons()` and `NEUTRAL` preserve
  the v1 surface; `with_devices`, `ports()`, `axes()` and `keys()` expose the schema-2
  fields. A v1 movie reconstructs neutral extra fields from its port-0 mask.
- **CLI adapter** (`crates/chronofork-cli/src/tas.rs`, `studio.rs`, `main.rs`).
  `chronofork tas record|play|verify|compare|studio` are thin, line-oriented adapters
  over library values. They print stdout-only `key=value` blocks separated by one blank
  line, send `chronofork: <error>` diagnostics to stderr, exit 0/1, read movie files
  with the existing size bound, and publish movies through a sibling temporary file
  renamed on success (`write_movie_file`). They perform no emulation, format, range or
  comparison logic themselves.
- **Deterministic acceptance backend.** `chronofork-test-backend` consumes one
  `InputFrame` per step and advances exactly one tick per step. All M3 evidence is
  synthetic-backend only, on Linux. The Mega Drive slice ignores input
  (`fn step(&mut self, _input: InputFrame)`), so it is not an input-TAS backend.
- **Deferred items this design addresses.** ADR-0004 §12 deferred "rerecording and
  branch editing (v1 defines only passive parent-movie metadata; the CLI exposes no
  branch editing)"; ADR-0005 §8 deferred "rerecording and branch editing; in-place movie
  editing; rerecord counters"; ADR-0006 §13 deferred "rerecording, branch editing,
  in-place movie editing and rerecord counters"; ADR-0007 §9 deferred "rerecording and
  branch editing; in-place movie editing; rerecord counters; switching branches inside
  the player". At design time the [M3 milestone](../roadmap/milestones.md) keeps
  "Rerecord and branch editing" unchecked, the [roadmap](../roadmap/roadmap.md) Stage 3
  bullet is open, and `FR-TAS-001` remains **Partial**.
- **Research context.** [TAS and debugging](../research/tas-and-debugging.md) §4.3
  records that BizHawk's rerecording is not a special emulation mode: it is the result
  of input-log truncation on rewind, state invalidation on edits and savestate loads,
  and that `Rerecords` is "a heuristic counter attached to input/state operations, not a
  proof that emulation was re-run or that determinism held"; §10.2 repeats "Rerecord
  count is a heuristic" and recommends treating it as user-facing metadata, never as a
  determinism proof. §4.3 and §4.5 also record the reusable edit pattern: an edit at
  frame `N` invalidates states after `N`, then the tool restores the nearest prior state
  and replays forward. §5.3 records that state caching is session-layer policy, not a
  backend contract. This is upstream-derived context, not a ChronoFork decision, and
  none of it is adopted verbatim.

## Problem

How should ChronoFork add deterministic input editing, truncation and branch creation on
top of the verified M3 baseline so that:

1. **Nothing accepted changes.** No existing movie byte, format/schema version, limit,
   anchor rule, decode/rejection category, replay result, state hash, scheduler
   behavior, storage layout, session API or existing CLI output changes; the M1/M2/M3
   evidence stays valid, including the v1 and v2 containers and the verified rewind
   surface.
2. **Edits are functional and exact.** A caller can replace the `InputFrame` of one or
   more existing recorded steps and optionally truncate the movie after a chosen step.
   The unchanged prefix is copied and verified against its recorded expectations, every
   edited step and every later step is re-simulated deterministically, every expectation
   is recomputed, and the result is a new canonical movie. The source movie value and
   source file are never mutated.
3. **Rerecord semantics are stated and bounded.** "Rerecord" here means the suffix
   re-simulation caused by an edit, not a new emulation mode. Repeated edits are
   represented as a chain of new artifacts with immediate-parent provenance, not as a
   mutated file. No new canonical field or format version is added; a cumulative
   rerecord counter, if shown at all, must be derivable or per-invocation diagnostics,
   never stored movie bytes.
4. **Branches are real but isolated.** The output is a new artifact with its own non-zero
   movie id, `parent_movie_id` equal to the source movie id (including the passive,
   never-resolved dangling-parent rule), optional non-inherited name, and no effect on
   the source movie, session, file, storage root or the existing comparison paths.
5. **Anchors stay canonical and honest.** The embedded initial checkpoint is copied
   verbatim. Anchors before the earliest edited or truncated step remain structurally
   valid and are retained by default; anchors at or after it are dropped or rebuilt
   deterministically. There is no second anchor format, no anchor-based skipping of
   prefix verification, and no silent mixing of retained and rebuilt anchors.
6. **Compatibility is reject-only and typed.** Editing a v1 movie produces a v1 movie;
   editing a v2 movie produces a v2 movie; there is no migration, upgrade, downgrade or
   silent truncation. Device-field edits that a format cannot encode are rejected. All
   indices, counts, truncation bounds, anchor budgets and output sizes are typed and
   checked before allocation or execution.
7. **The CLI surface is minimal and testable.** One `chronofork tas edit` subcommand
   with exact flags for movie input/output, index edits, truncation, branch metadata and
   anchor policy; an atomic sibling-temp+rename publish that can never overwrite the
   source path; a stable `key=value` output block; stderr diagnostics; exit 0/1; help and
   honesty text. Existing `record/play/verify/compare/studio` behavior stays unchanged
   except a declared additive help/dispatch update.
8. **Ownership and determinism are structural.** No global state, no source mutation, no
   host time/path/nonce in canonical bytes, bounded resource use, one codec, one editing
   implementation, one snapshot parser, one compatibility predicate and one
   first-divergence comparison rule; no new dependency, `unsafe`, `#[allow]` or
   `#[ignore]`.
9. **The increment stays honest.** The design is synthetic-backend only, does not
   implement a timeline UI, scripted input files, persisted counters, real-hardware
   input TAS or import/export, and does not complete M3.

## Alternatives

### Editing model

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Functional edit: decode the source, import its embedded checkpoint into a fresh session, copy and verify the unchanged prefix, apply edited `InputFrame`s, truncate if requested, re-simulate the suffix, recompute every expectation and construct a new canonical movie** | Preserves provenance and source isolation; prefix exactness is verified, not assumed; the output is an ordinary movie accepted by every existing command; no format change; reuses the one constructor/writer, the one import path and the one checkpoint path | Cost is O(output steps) execution per edit; a late edit re-simulates a long suffix; large edits are bounded by the existing step/file limits | **Recommended for this increment** |
| B. In-place mutation of the decoded movie (replace a record, then rewrite the same file) | No new artifact | Requires re-simulating the suffix anyway to make expectations valid, but silently destroys the source, breaks provenance and atomicity, can leave a half-written file, and makes "source unchanged" impossible; a failed edit corrupts a recording | Rejected |
| C. Delta/patch sidecar file ("base movie + edit list") | Small patches; source retains identity | A second artifact, a second resolution/application protocol and a new compatibility contract; replay needs patch application; the output is not a self-contained `.cfmovie`; needs its own versioning and rejection tests | Rejected for this increment; a future patch format is a separate decision |
| D. Cross-format or scripted re-record from a text input script | Familiar TAS workflow | Needs a scripted/variable input-file model and parser that does not exist; a second input surface and format; out of scope and deferred | Rejected for this increment |
| E. Re-record the whole movie from scratch with a full edited input list and ignore the source prefix | Simple loop, no prefix logic | Loses the verified prefix/provenance distinction, can silently diverge on the prefix, cannot guarantee the unchanged prefix bytes, and still needs every input; no advantage over A | Rejected |
| F. Add a persisted rerecord counter (and a v3 format field) to carry edit history | Explicit user-facing count | Changes canonical bytes and the limits/compatibility matrix, invalidates verified format evidence, and a count is a heuristic, not a determinism proof (research §4.3/§10.2); the task prefers no new field | Rejected; no format v3 |

### Truncation coordinates

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Zero-based `--truncate-after <INDEX>`: retain steps `0..=INDEX`, drop every later step; reject a no-op truncation that drops nothing** | Matches the zero-based step index used by edits and by the movie format; unambiguous; no tick search | Requires an explicit rule that at least one step is dropped, so the command never degenerates into a metadata-only copy | **Chosen** |
| B. `--steps <N>`: retain the first `N` steps | Familiar count | Two coordinate systems (index and count) invite off-by-one errors in one command; conversion still needs the index rule | Rejected for the CLI |
| C. `--truncate-at <TICK>` | Tick-oriented | The boundary may be unaligned and would need the ADR-0005 alignment rule; a zero-step or start-tick boundary is ambiguous; no benefit over the index | Rejected |
| D. `--from/--until` range pair | Reuses replay vocabulary | A range expresses a window, not a truncation; truncation drops the suffix and keeps a prefix, so the pair adds an invalid-state surface | Rejected |

### Anchor policy

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Default `keep-prefix`: retain each source anchor whose `step_index` is before the earliest affected step, verbatim and in order; drop every anchor at or after it** | Retained anchors are still bound to unchanged, re-verified prefix steps; no capture work; deterministic; no second anchor format; the embedded checkpoint stays authoritative | A late edit drops almost all anchors; a truncated movie loses anchors in the removed suffix | **Default and chosen** |
| B. Retain every anchor | Maximum seek coverage | Anchors at/after an edit no longer match the new state trajectory, so the output would be an invalid v2 movie (binding rejection) or, if unchecked, silently wrong for seek | Rejected |
| C. Rebuild the whole anchor table at a caller interval | Restores seek coverage after an edit; deterministic cadence already exists in v2 recording | Costs checkpoint captures and bytes; only meaningful for v2; interval and budgets must be validated; all source anchors are discarded | **Supported as an explicit option**, not the default |
| D. Mix retained prefix anchors with rebuilt suffix anchors | Keeps old coverage and adds new | Needs collision/ordering rules and a policy for an interval that lands on a retained index; two anchor sources in one output make "which capture produced this anchor?" ambiguous | Deferred; not in this increment |
| E. Drop all anchors by default | Simplest | Silently degrades a v2 movie's seek behavior when most anchors are still valid; loses useful metadata for no correctness gain | Rejected as default; `drop` is an explicit option |

### Branch identity

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. `--movie-id` optional; default to `source.movie_id + 1` with checked arithmetic; explicit id must be non-zero and differ from the source; `parent_movie_id` is always the source id and is not user-settable** | Deterministic, convenient, never requires a registry; provenance cannot be misrepresented; the parent-dangling rule is preserved | Default ids are only labels and two branches can collide; the format already says ids are logical labels, not a registry key | **Chosen** |
| B. Require an explicit `--movie-id` | Fully caller-controlled | Every script must know the source id; repeated use is error-prone; no correctness benefit | Rejected |
| C. Generate a random/UUID id or resolve a global registry | Globally unique-looking | Host entropy is forbidden in canonical bytes; a registry adds global state, resolution and failure modes the format deliberately avoids | Rejected |
| D. Allow the output id to equal the source id | No collision error | Contradicts provenance (`parent_movie_id` must differ from `movie_id`) and makes branch artifacts indistinguishable | Rejected |

### Command shape and placement

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. One `chronofork tas edit` subcommand; one `chronofork_sessions::edit` library module; one `chronofork-cli/src/tas_edit.rs` adapter** | One command, one semantics, one implementation; the CLI stays thin; the library owns execution policy in the session layer; both new modules can stay below the AS-14 review trigger | Two new modules and their tests | **Chosen** |
| B. Add a `tas rerecord` synonym | Matches TAS vocabulary | Two spellings for one operation means a duplicate parser or an alias contract; "rerecord" is the semantic description, not a different command | Rejected |
| C. Extend `tas record` with `--edit-from`/`--truncate-after` | No new subcommand | Overloads a command whose source is a live session, not an existing movie; mixes two different input models, output blocks and failure contracts | Rejected |
| D. Put the edit logic in `movie.rs`/`tas.rs` | No new files | Grows two already-large modules past the AS-14 trigger, mixes format ownership with edit policy, and makes the CLI own execution policy if put in `tas.rs` | Rejected |
| E. Library-only, no CLI | Smallest surface | The increment's deliverable includes a CLI edit surface and process contract; a library-only change would not be an executable operator tool | Rejected |
| F. New crate or dependency | Isolated code | `chronofork-sessions` is the correct layer; a crate/dependency is a liability with no demonstrated need (AS-11, AGENTS.md dependency rules) | Rejected |

### Prefix strategy

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Execute and verify every prefix step from the embedded checkpoint** | The prefix is proven to reproduce its recorded expectations; no anchor caveat; one start-state source | Cost O(earliest affected step), which is bounded by the movie limits; a very late edit costs almost a full replay | **Chosen** |
| B. Start from a source anchor at or before the edit and skip the prefix | Faster late edits | The anchor is structurally bound to one step but the pre-anchor history is not re-verified (the documented ADR-0006 prefix delta); an edit must not inherit that caveat silently, and mixing the anchor path with edit construction adds a second start-state source | Rejected for this increment |
| C. Trust the source prefix records and copy them without executing | Cheapest | A movie with a tampered or corrupt prefix would be silently propagated into the branch; "copy/verify the unchanged prefix" is unmet | Rejected |
| D. Copy the prefix and spot-check a sample | Cheaper than A | Sampling cannot prove the prefix; a divergence could be missed; exactness claims would be false | Rejected |

## Decision (accepted 2026-11-15)

### 1. Scope and canonical home

This ADR is the canonical home for **rerecord and branch editing**: the functional edit
model, suffix re-simulation, truncation, branch artifact identity/isolation, anchor
handling on edit, the edit-specific v1/v2 compatibility and bounds, and the
`chronofork tas edit` surface. [ADR-0004](0004-tas-movie-format.md) remains canonical
for the movie bytes, [ADR-0005](0005-partial-replay-and-branch-comparison.md) for ranged
replay and comparison, [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md)
for v2 anchors and schema-2 input, and [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md)
for the in-process rewind cache and `tas studio`. Architecture, requirements, roadmap and
acceptance documents link here instead of restating the edit semantics.

This is an **Accepted** decision (owner-approved 2026-11-15T21:09:29-03:00) and was
implemented under the executable [M3 rerecord
acceptance plan](../quality/m3-rerecord-acceptance.md). Rows M3-A35–M3-A44 were executed
and independently verified by the t10 repair-round-2 PASS (2026-11-16). The public names
below (`MovieEdit`, `EditMeta`, `EditOutcome`, `EditError`, `AnchorPolicy`, constants)
describe the implemented shape; further spelling or parameter changes require an
amendment, because the semantics, typed rejection boundaries, canonical bytes, limits and
CLI field contract are fixed.

The increment adds no crate, dependency, `unsafe`, `#[allow]`, `#[ignore]`, deleted test
or relaxed lint, and no new canonical format field or version. The only declared
additive CLI behavior is the new subcommand plus the help/dispatch text that lists it.

### 2. Editing model and algorithm

**Definitions.** Let the decoded source movie `S` have `n = S.step_count()` recorded
steps, `start_tick = S.start_tick()`, embedded checkpoint bytes
`S.snapshot_bytes()`, and step `i` with recorded `outcome_tick t_i`, input `in_i` and
state hash `h_i`. Let `E` be a set of distinct edited step indices, each with a
replacement `InputFrame`, and let `T` be an optional last retained step index
(`--truncate-after T`).

- **Output step count:** `m = T + 1` when truncation is requested, otherwise `m = n`.
- **Earliest affected step:** `e = min({ i | i ∈ E } ∪ { T + 1 } when truncation is
  requested)`. Every step `i < e` is the **prefix**; every step `i >= e` is the
  **suffix**. At least one of `E` or `T` exists by request validation, so `e` is always
  defined.
- **Prefix:** steps `0..e` (exclusive). Prefix inputs are the source inputs, unchanged.
- **Suffix:** steps `e..m`. Each uses its edited frame when present, otherwise the
  source input.

**Execution.** The edit function takes the decoded `&Movie`, a `SessionManager` and a
validated edit request; it performs no filesystem I/O:

1. **Validate the request before importing or executing anything** (§6): indices,
   duplicates, truncation, branch identity, name, anchor policy and inherited bounds.
   An invalid request creates no session and executes no step.
2. **Import the embedded checkpoint** with
   `SessionManager::create_session_from_snapshot_bytes(S.snapshot_bytes())`. Backend
   identity, backend version, payload version and configuration compatibility are decided
   only by the existing `validate_envelope` predicate through this path. The embedded
   checkpoint bytes are the output's embedded checkpoint, copied verbatim; they are never
   re-serialized or replaced.
3. **Execute output steps `0..m` in ascending order** in that one fresh session. For step
   `i`:
   - choose the input: the source input for the prefix; the replacement frame when
     `i ∈ E`; otherwise the source input;
   - `manager.step(session, input)`; a session/backend failure is a typed edit failure
     carrying the zero-based output step index and the preserved source error;
   - a halted outcome before the last output step is a typed failure and produces no
     movie (the same rule as recording); a halt exactly on the last output step is
     accepted;
   - **prefix (`i < e`):** compare the observed outcome tick and state hash with the
     source record `(t_i, h_i)`; a mismatch is the existing typed
     `TickMismatch`/`Divergence` (first mismatch only, exact index, tick, expected and
     observed hash) and produces no movie, because a branch must not inherit a divergent
     prefix. On success, push the **source step record verbatim** — same tick, input and
     hash — so the prefix bytes are exactly the source bytes.
   - **suffix (`i >= e`):** push `MovieStep::new(outcome_tick, input, state_hash)` using
     the observed outcome. Every suffix expectation is recomputed; no source expectation
     is copied.
4. **Anchors:** apply the anchor policy (§5) using the source anchor table and, for
   `rebuild`, captures taken at the validated cadence during the same execution.
5. **Validate and construct the new movie with the source's format:**
   `Movie::from_parts(meta, S.snapshot_bytes().to_vec(), steps)` for a v1 source,
   `Movie::from_v2_parts(meta, S.snapshot_bytes().to_vec(), anchors, steps)` for a v2
   source. The constructor applies the same metadata, step-sequence, anchor-binding and
   size checks as the reader; the writer may not emit a movie it could not read back.
6. Return the new `Movie` plus an `EditOutcome` report. The command does not run a second
   full `verify` pass (that would double execution); the output is a normal movie and the
   existing `tas verify` remains the complete verification path. Prefix verification is
   performed here because the branch must not copy an unverified prefix.

There is exactly **one editing implementation** (`chronofork_sessions::edit`). The edit
loop is not a second replay entry point: `replay.rs` keeps the single replay loop, and
the first-mismatch comparison is factored into one crate-private step-expectation helper
used by both replay and edit, so there is one `TickMismatch`/`Divergence` rule and no
second comparator.

**Property.** Given a valid source movie, a valid edit request and a compatible backend,
the output movie replays successfully: every prefix record is a verified source record,
and every suffix record is an observed outcome produced by the same imported checkpoint
and the output input sequence. The output is byte-canonical for its metadata, embedded
checkpoint, anchors and records under the existing writers. The cost is
`O(m)` backend steps plus `O(|E|)` request bookkeeping, `O(log n)` per constructed-step
lookup and, for `rebuild`, one checkpoint capture per cadence step; it is bounded by the
source step limit, the anchor budgets and `MAX_MOVIE_BYTES`.

**Tick sequence.** The edit does not assume a fixed `+1` tick quantum: the output tick
sequence is whatever the backend reports for the edited inputs, and the normalizing
constructor enforces the format's strictly increasing rule. On a backend whose step
quantum responds to input, an output whose ticks are not strictly increasing is a typed
rejection, not a rewritten sequence. The synthetic acceptance backend advances exactly one
tick per step.

**Truncation does not execute dropped steps.** A pure truncation (no edited inputs)
executes and verifies only the retained prefix `0..=T`; steps `T+1..n` are not executed
and no expectation is invented for them. An edit at an index beyond `T` is rejected
before execution.

**Frame replacement at the library layer is full-frame.** A library caller supplies an
`InputFrame`; the replacement may change any device field. The CLI is narrower (§7): it
sets only port 0 and preserves the other fields.

### 3. Rerecord semantics and the rerecord counter

**Definition.** In this ADR, **rerecord** means *the deterministic suffix re-simulation
that an edit causes*, not a new emulation mode and not a mutation of an existing file.
Each `tas edit` invocation is one rerecord operation: it copies and verifies the
unchanged prefix, applies the requested frames, re-simulates from the earliest affected
step, recomputes every later expectation and emits a new artifact.

**Repeated edits.** Repeated edits are represented as a **chain of new artifacts**, not
as state inside one file. Editing artifact `B` (whose `parent_movie_id` is `A`) produces
artifact `C` with `parent_movie_id = B.movie_id`; the immediate parent edge is the only
provenance record. The command never resolves, validates or repairs the parent chain, and
never requires the parent file to still exist (the passive dangling-parent rule of
ADR-0004 §3 is unchanged). A caller can follow the chain by reading each artifact's
metadata, but no automatic ancestry traversal, cycle detection or global ordering exists
in this increment.

**No stored counter.** This design adds **no** `rerecord`/`rerecords` field, no history
list, no edit log and no format version bump. Rationale: the canonical bytes are an
execution artifact, not a user-activity log; a cumulative counter would change the format
and its verified evidence, and upstream evidence treats the count as a heuristic, not a
determinism proof (research §4.3, §10.2). Instead, the CLI prints **per-invocation
diagnostics** derivable from the request and source: `edits=<count>`,
`truncate_after=<index or empty>`, `prefix_steps`, `resimulated_steps` and the anchor
counts (§7). These values describe this operation only; they are not a cumulative
rerecord count, and no document may call them one. A persisted, user-facing rerecord
counter is explicitly deferred to a future metadata decision (it would not need to be a
movie field; a sidecar or external database would still be a new contract).

Editing with no effective request (no `--set-input` and no `--truncate-after`) is
rejected as a typed `NoEditsRequested`; a metadata-only re-issue of a movie (pure branch
copy) is not part of this increment. A same-input replacement is accepted and does
re-simulate, because it is still an edit request and can reveal a divergent source
prefix.

### 4. Branch metadata, identity and isolation

The output is a **new artifact**, never a mutated source:

| Property | Rule |
|---|---|
| `movie_id` | Non-zero `u64`. `--movie-id` is optional; the default is the source `movie_id` checked `+ 1`. An explicit id must be non-zero and must differ from the source id; equality is a typed rejection, and an overflowing default is a typed rejection requiring an explicit id. Ids are logical labels, not a registry; collision between two branches is possible and is not resolved. |
| `parent_movie_id` | Always the source `movie_id`, encoded as the existing optional parent. It is not user-settable. It must differ from the output id (guaranteed by the rule above). |
| Dangling parent | The source movie's id is a label. The command does not resolve, validate, require or repair the parent, and the output decodes, replays, compares and opens in `tas studio` after the source file is deleted. There is no parent lookup, registry or foreign key. |
| `name` | Optional non-canonical label. It is **not inherited** from the source; with no `--name` the output has no name (length 0). A supplied name is bounded by `MAX_NAME_BYTES` (256), must be non-empty UTF-8 without control characters, and is never a path component or identity authority. |
| Embedded checkpoint | Copied verbatim from the source; its snapshot id, origin session id, backend identity/version, tick, configuration hash and state hash are unchanged. The movie does not mirror backend identity at movie level. |
| Source movie value | Immutable `&Movie`; the edit constructs a new value and never mutates or re-encodes the source value. |
| Source file | Opened read-only with the existing size bound; never opened for writing; source bytes and SHA-256 unchanged. The command never renames, truncates or replaces the source path. |
| Source session/storage | There is no source session and no storage root. The edit registers exactly one fresh imported session in the caller's manager through the existing import path. Pre-existing sessions in that manager keep their tick, state hash and snapshot map; they are never addressed or mutated. |
| Existing output path | The destination may name an existing **non-source** file; it is replaced atomically on success. On any failure the previous destination bytes remain and no temporary file is left behind. |

**Isolation is structural.** The library edit function receives a decoded movie value and
a manager, creates one session and returns a value; it has no file handle to the source,
no global state and no registry. Two concurrent or sequential edits of the same source
are independent, and the second sees the same source bytes as the first.

**Relationship to the existing paths.** The output is an ordinary v1/v2 movie, so
`tas play`, `tas verify`, `tas play|verify --from/--until`, `tas compare`, anchored v2
seek and `tas studio` accept it without change. `tas compare` between a source and a
branch is still a recorded-expectation diff: an input edit that changes no recorded
`(outcome tick, state hash)` can compare identical (as ADR-0005 defines equality), a
state-changing edit reports the first divergent index/tick/hash, and a truncation reports
a length mismatch; metadata never decides equality and a compare PASS is not a
verification. `tas verify` on the output remains the re-simulation path.

### 5. Anchor handling

An anchor is meaningful only for the exact state after its recorded step. Editing or
truncating at or after that step changes or removes the state, so its anchor is no longer
valid for the output. The exact rule:

Let `e` be the earliest affected step from §2. Let `A = S.anchors()` be the source anchor
table in its already-validated strictly increasing order.

| Policy | Output anchor table | Reporting |
|---|---|---|
| `keep-prefix` (default) | Every source anchor with `step_index() < e`, copied verbatim with the same index and bytes, in source order. Anchors at or after `e` are dropped. | `anchors_retained` and `anchors_dropped` counts. |
| `drop` (explicit) | Empty table for a v2 output; anchors are never carried. V1 has no table. | `anchors_retained=0`, `anchors_dropped` = source anchor count. |
| `rebuild` (explicit) | All source anchors are dropped and a new table is built at the v2 recording cadence: after output steps `N-1, 2N-1, …` while the index is below `m`, where `N = --anchor-interval`; each anchor captures the canonical checkpoint of the edit session after that verified output step and must match the just-recorded outcome tick and state hash. | `anchors_retained=0`, `anchors_dropped` = source anchor count, `anchors_rebuilt=m / N` (floor, computed before execution). |

Rules:

1. **The embedded initial checkpoint is always copied unchanged** and is not an anchor;
   it remains the implicit before-step-0 state. No anchor is placed at or before step 0,
   and an output is never zero-step (an edit requires a non-empty source), so no output
   anchor position is invalid.
2. **Retained anchors are re-validated, not re-parsed by a second reader.** They are
   passed to `Movie::from_v2_parts`, which checks strictly increasing order, index range
   and the anchor snapshot tick/state-hash binding against the output step at that index.
   Because every retained prefix step is copied from a verified source record, the
   binding holds by construction and any violation is a typed rejection (no output).
3. **Rebuilt anchors use the one existing capture path** (`SessionManager::snapshot`,
   `snapshot_record`/`snapshot_bytes`, `MovieAnchor::new`) and the one anchor parser
   (`Snapshot::from_bytes`). Per-anchor `MAX_ANCHOR_SNAPSHOT_BYTES` and cumulative
   `MAX_ANCHOR_BYTES` bounds are enforced during capture; the predicted count
   `m / N <= MAX_ANCHORS` is checked before execution; any failure produces no movie.
   Each rebuilt envelope is re-encoded with a fixed canonical snapshot/origin-session
   identity through the one `Snapshot::from_bytes`/`Snapshot::to_bytes` pair, so its
   bytes do not embed the caller manager's session allocation and two identical edits
   are byte-identical even in one manager. Only fixed-width identity fields change: the
   envelope length, tick, state digest, backend fields and payload are unchanged, and
   v2 recording keeps its session-assigned identity, so recording bytes are unchanged.
4. **No mixing.** `keep-prefix` retains prefix anchors only; `rebuild` drops all source
   anchors and builds only new ones; `drop` produces none. Retaining prefix anchors while
   rebuilding suffix anchors is deferred because it needs collision and precedence rules
   and would create two anchor sources in one artifact.
5. **No anchor-based prefix skipping.** The edit always verifies the prefix from the
   embedded checkpoint; source anchors are never used as edit start states. This avoids
   inheriting the ADR-0006 anchored-replay prefix caveat and keeps the output's prefix
   evidence complete.
6. **v1 has no anchor table.** An explicit anchor policy or interval on a v1 source is a
   typed rejection, never a silent no-op; the default `keep-prefix` on v1 is simply the
   empty implicit rule.
7. **One anchor format.** There is no second anchor serializer, loader or compatibility
   predicate; the output's anchor table is the same canonical table ADR-0006 defines.

### 6. v1/v2 compatibility, bounds and typed errors

**Format preservation.** The output format is the source format; there is no migration,
upgrade, downgrade or silent truncation:

| Source | Output | Embedded checkpoint | Steps | Anchors | Device-field edit |
|---|---|---|---|---|---|
| v1 (`CFMOVIE1`, format 1, schema 1) | v1 | copied verbatim | 44-byte v1 records | none; explicit anchor flags are a typed rejection | v1 can encode only port 0. The CLI sets port 0 only. A library full-frame replacement whose extra fields are non-neutral is rejected with the existing `UnsupportedInputSchema` naming schema 2; it is never truncated to port 0. |
| v2 (`CFMOVIE2`, format 2, schema 2) | v2 | copied verbatim | 72-byte schema-2 records | `keep-prefix`, `drop` or `rebuild` | Full frame encodable; the CLI still sets port 0 only and preserves the recorded ports 1–3, axes and keys. |

- **No v3.** No new magic, version, schema, field, checksum coverage or limit. A v1
  source is constructed with `Movie::from_parts`; a v2 source with
  `Movie::from_v2_parts`. Calling the other constructor is a defect, not a fallback.
- **Reject-only.** Unknown/future versions are still rejected by the existing reader
  before any edit; editing does not add a migration or a rewrite-on-open path.
- **Backend compatibility** is decided only when the edit imports the embedded
  checkpoint through `SessionManager::create_session_from_snapshot_bytes`, hence by the
  existing `validate_envelope`. An incompatible checkpoint is a typed edit failure
  preserving the session error; no output is written. The command has no backend
  selection flag and uses the same deterministic synthetic `BackendFactory` as the other
  TAS commands. Mega Drive movies are not an edit acceptance surface (its `step` ignores
  input), and no hardware input TAS is claimed.

**Typed bounds (all inherited; no new format constant):**

| Bound | Value / rule | Enforcement point |
|---|---|---|
| Source/output movie size | `MAX_MOVIE_BYTES` (256 MiB) | CLI read bound before decode; the existing writer check for the output |
| v1 step count | `MAX_STEPS` (4,194,304) | inherited from the source; output `m <= n` so it cannot grow |
| v2 step count | `MAX_V2_STEPS` (2,000,000) | inherited from the source; output `m <= n` |
| Edit index | `u64` parsed, range-checked `< n`, then converted to `usize`; distinct per index | request validation before import/execution |
| Truncation index | `< n` and `T + 1 < n` (at least one step dropped); zero-step/none effective is a typed rejection | request validation |
| Edit within truncation | every edited index `< T + 1` when truncation is requested | request validation |
| Output step count | `m = T + 1` or `n`; `1 <= m <= n` | request validation and constructor |
| Rebuilt anchors | `N >= 1`, `N <= m`; `m / N <= MAX_ANCHORS`; per-anchor `<= MAX_ANCHOR_SNAPSHOT_BYTES`; cumulative `<= MAX_ANCHOR_BYTES` | request validation and capture |
| Retained anchors | subset of the source table, which already satisfies the v2 bounds | request validation and `from_v2_parts` |
| Name | `MAX_NAME_BYTES` (256), non-empty UTF-8, no control characters | `MovieMeta::new` |
| Movie ids | non-zero `u64`; output id != source id; parent = source id | `MovieMeta::new` plus edit request validation |
| Output id default | source id checked `+ 1`; overflow is a typed rejection | request validation |

No allocation is sized from an unvalidated CLI number: numeric flags are parsed as `u64`
and range-checked before any `usize` conversion, vector reservation or checkpoint
import. The output step vector is bounded by `m`, which is derived from the source step
count; the edit request stores at most one frame per distinct index, and its size is
bounded by `n` because duplicate/out-of-range indices are rejected. All size arithmetic
is checked; overflow is a typed error, never a wrap or panic.

**Typed errors.** One edit-boundary error type preserves the existing movie and session
errors as sources; no stringly error, no flattened `String`, no panic, no silent clamp:

| Category | Examples | Contract |
|---|---|---|
| Request | `NoEditsRequested`, `EditIndexOutOfRange { index, step_count }`, `DuplicateEditIndex { index }`, `EditBeyondTruncation { index, truncate_after }`, `TruncateIndexOutOfRange { index, step_count }`, `TruncationNoOp { index, step_count }` | Rejected before import, execution or allocation |
| Branch identity | `BranchIdEqualsSource { movie_id }`, `BranchIdOverflow { source_movie_id }`, plus `MovieError::InvalidMovieId` / `InvalidParentMovieId` / `InvalidName` through `MovieMeta::new` | Rejected before import |
| Anchor policy | `AnchorsUnsupportedForV1`, `ConflictingAnchorPolicy`, `InvalidAnchorInterval { interval, steps }`, `TooManyRebuiltAnchors { limit, found }`, plus the existing anchor size/binding errors | Rejected before execution when predicted; during capture/binding otherwise; no output |
| Movie/format | `UnsupportedInputSchema` (v1 device edit), `FileTooLarge`, `InvalidEmbeddedSnapshot`, `StartTickMismatch`, `TooManySteps`, `InvalidAnchorSnapshot`, anchor binding categories | Preserved `MovieError`; no second error taxonomy |
| Session | The existing `SessionError` for import, step, halt or capture failures, carrying the zero-based output step index where applicable | Preserved source error |
| Verification | The existing `MovieError::TickMismatch` / `Divergence` for a source prefix that does not reproduce its own records; `ReplayFailed`-equivalent category preserving the session error and index | First mismatch only; no resynchronization, no output |

### 7. CLI surface, output block, atomicity and process contract

**Command (accepted shape):**

```text
chronofork tas edit --movie <FILE> --out <FILE>
                    --set-input <INDEX=MASK>...   or --truncate-after <INDEX>
                    [--movie-id <U64>] [--name <NAME>]
                    [--anchors <keep-prefix|drop|rebuild>]
                    [--anchor-interval <N>]
chronofork tas edit --help
```

- **`--movie <FILE>`** is the source: required, must exist and be readable, opened
  read-only and read with the existing `MAX_MOVIE_BYTES` bound before decode. It is never
  opened for writing, renamed or removed.
- **`--out <FILE>`** is the destination: required, must not be a directory, and must not
  identify the source (see atomicity). It may name an existing non-source file, which is
  replaced atomically on success. A missing parent directory is a typed I/O failure.
- **`--set-input <INDEX=MASK>`** is repeatable. `INDEX` is a zero-based decimal `u64`
  with `INDEX < source.step_count()`; `MASK` is a decimal `u32` for port 0. The edit
  replaces **only** `ports[0]` of that step and preserves ports 1–3, axes and keys as
  recorded. Duplicate indices are a typed rejection (no last-wins). This is the entire
  CLI device-field surface: ports 1–3, axes and keys are preserved, not settable, in this
  increment (full-frame replacement stays library-only, and is deferred as a CLI
  surface).
- **`--truncate-after <INDEX>`** retains steps `0..=INDEX` and drops every later step.
  `INDEX` is a zero-based decimal `u64`; it must satisfy
  `INDEX + 1 < source.step_count()` so at least one step is dropped (a no-op truncation
  is a typed rejection, and the rule is stated without an underflowing subtraction). At
  least one of
  `--set-input` or `--truncate-after` is required. When both are present, every edited
  index must be `< INDEX + 1`.
- **`--movie-id <U64>`** is optional; default source id checked `+ 1`. It must be
  non-zero and differ from the source id.
- **`--name <NAME>`** is optional. The output name is absent unless supplied; it is never
  inherited from the source. Bounded by `MAX_NAME_BYTES`, non-empty, no control
  characters.
- **`--anchors <keep-prefix|drop|rebuild>`** defaults to `keep-prefix`.
  `--anchor-interval <N>` is valid only with `rebuild` and is a decimal `usize` with
  `1 <= N <= output step count`; an interval without `rebuild`, `rebuild` without an
  interval, or any explicit anchor flag on a v1 source are typed errors. Predicted `m / N > MAX_ANCHORS`, interval
  above `m`, and per/cumulative anchor byte overruns are typed errors and produce no
  output.
- **Rejected arguments:** positional arguments; missing/duplicate/empty values for
  single-valued flags; unknown flags including `--from`, `--until`, `--against`,
  `--steps`, `--root`, `--session`, `--snapshot`, `--ports`, `--axes`, `--keys` (the
  record device flags are not edit flags), and any foreign subcommand syntax. A
  non-UTF-8 command-line value is also a typed rejection: the CLI collects
  `std::env::args_os()` and reports it through the `chronofork: <error>` path instead of
  panicking. Each produces stderr `chronofork: <error>`, empty stdout and exit 1 before
  any file write.

**Atomic publish and source protection.** The command builds the complete output bytes in
memory (bounded by `MAX_MOVIE_BYTES`) and uses the **existing** sibling-temporary-file +
rename publish helper shared with `record`; there is one publish implementation, not two.
The destination must not identify the source:

1. Compare the two paths after making them absolute and lexically normalizing `.`/`..`,
   without following symlinks; equality is rejected with the stable `OutputAliasesSource`
   CLI diagnostic before any temporary file is created.
2. On the acceptance platform (Unix), if the destination already exists, also compare
   its device and inode with the source's; a hard link or symlink-resolved alias is
   rejected, so the source inode is never unlinked or rewritten. Platforms without that
   identity check fall back to the path rule; the acceptance claim is Linux.
3. Write the sibling temporary file (the existing naming convention may include the
   process id in the **filesystem name only**), rename on success, and remove the
   temporary file on failure. A failure never leaves a partial output at `--out` and
   never replaces an existing destination. The source file's bytes, SHA-256 and path are
   unchanged on success and on failure. This is atomic visibility against concurrent
   readers and failed writes, matching the existing movie publish helper; it is not a
   power-loss durability guarantee (no fsync) and no durability claim is made.

**Output block.** Exactly one stdout block, in this fixed order, with values from the
edit outcome and the written bytes:

```text
record=edit
movie_version=<1|2>
input_schema=<1|2>
source_movie_id=<u64>
movie_id=<u64>
parent_movie_id=<u64>
name=<NAME or empty>
start_tick=<t>
end_tick=<t>
steps=<output step count>
edits=<number of distinct edited steps>
truncate_after=<INDEX or empty when not truncated>
prefix_steps=<e, the number of copied and verified prefix steps>
resimulated_steps=<m - e, the number of suffix steps executed>
prefix_verified=1
anchor_policy=<keep-prefix|drop|rebuild>
anchors_retained=<n>
anchors_dropped=<n>
anchors_rebuilt=<n>
final_state_hash=<lowercase hex>
movie_sha256=<lowercase hex of the exact output file bytes>
source_movie_sha256=<lowercase hex of the exact source file bytes read>
```

`movie_sha256` and `source_movie_sha256` are copy/identity aids, not movie fields.
`prefix_verified=1` asserts that every prefix step was executed and matched its source
record; it never asserts that a dropped suffix was executed. `resimulated_steps=0` with
`truncate_after` present is the pure-truncation case. `edits`, `truncate_after`,
`prefix_steps` and `resimulated_steps` are per-invocation diagnostics, **not** a
cumulative rerecord counter. A rejected edit prints no block; a typed failure prints
`chronofork: <error>` on stderr and exits 1.

**Streams, exit codes and help.** Exit 0 on a successful publish and 1 on any validation,
decode, execution, anchor, I/O or write failure. Stdout carries only the block; stderr
carries only diagnostics. No timestamp, host name, path, process id, nonce, entropy or
telemetry appears in the block or canonical bytes. `tas edit --help` and the `tas --help`
summary must state: the flags and index conventions; that editing is a functional
suffix re-simulation from the embedded checkpoint, not an in-place or O(1) operation;
that the source is never modified and the output is a new artifact with the source as
parent; the anchor policy and the embedded-checkpoint rule; that no rerecord counter is
stored; the v1/v2 preservation and device-field limitation; the synthetic-backend scope;
the deferred list; and that M3 is not complete. Adding `edit` changes the `tas` dispatch
error text and `tas --help`, which is a declared additive behavior change; every other
existing command's stdout must stay byte-identical.

### 8. Architecture, ownership, determinism and resources

- **New `crates/chronofork-sessions/src/edit.rs`** (module `chronofork_sessions::edit`,
  exported from `lib.rs`). It owns `MovieEdit`, `EditMeta`, `AnchorPolicy`, `EditOutcome`,
  `EditReport` and `EditError`, and the single edit function. It uses `Movie`/`MovieMeta`/`MovieStep`/`MovieAnchor`,
  `SessionManager::create_session_from_snapshot_bytes`/`step`/`snapshot`/
  `snapshot_record`/`snapshot_bytes`, `InputFrame`, `Tick` and `StateHash`. It performs no
  filesystem I/O and reaches into no backend internals.
- **`chronofork_sessions::movie`** keeps the one reader/writer and the validating
  constructors. It gains no edit function. A read-only accessor may be added only if the
  edit module genuinely cannot use an existing one; any new `pub`/`pub(crate)` item needs
  a caller and a documented reason (AS-05).
- **`chronofork_sessions::replay`** keeps the single replay loop. The first-mismatch
  step comparison is extracted into one crate-private helper used by both replay and
  edit; no second divergence predicate or replay entry point is created.
- **New `crates/chronofork-cli/src/tas_edit.rs`** owns the `tas edit` adapter: flag
  parsing, the output-alias guard, the source read with the existing bound, the output
  block writer and the help text. `tas.rs` keeps subcommand dispatch and the summary
  help; `main.rs` keeps top-level dispatch. The existing publish helper is reused, not
  copied.
- **One rule per concern.** One movie codec (`Movie::from_bytes`/`to_bytes`), one
  validating constructor per version, one embedded-checkpoint parser
  (`Snapshot::from_bytes`), one anchor parser, one compatibility predicate
  (`validate_envelope`), one checkpoint import path
  (`create_session_from_snapshot_bytes`), one checkpoint capture path for rebuilt
  anchors, one comparison rule and one edit implementation. No second format, parser,
  predicate or publish helper.
- **No global state.** No static mutable state, thread-local, singleton or process-wide
  registry. The edit function mutates only the caller's `SessionManager` by registering
  one imported session and the new movie value it returns; all other inputs are borrowed
  immutably.
- **Determinism.** For identical source bytes, request, output metadata and anchor
  policy, the output bytes are identical across separate processes, repeated runs,
  debug/release builds, working directories and source/output file names. A changed-host-clock
  variation was not separately exercised in the acceptance sandbox; no clock API or
  host-dependent value is used in the edit paths, and canonical bytes were scanned for
  host-dependent content. No host time, path, process id, nonce, randomness or filesystem
  metadata enters canonical bytes. Rebuilt anchor snapshots come from the existing deterministic
  checkpoint path and are re-encoded with the canonical fixed snapshot/origin-session
  identity described in §5.3, so an unrelated session or a prior edit in the caller's
  manager cannot change a later edit's bytes; v2 recording keeps its session-assigned
  identity, so its bytes are unchanged. Movie ids and names are caller-supplied labels
  and are the only intentional byte-level difference between otherwise identical
  branches.
- **Bounded resources.** The source is read with `MAX_MOVIE_BYTES`; execution is bounded
  by `m <= n` and the format step limit; the edit map is bounded by `n` distinct indices;
  rebuilt anchors are bounded by the count/per-anchor/cumulative limits; the output is
  bounded by `MAX_MOVIE_BYTES`; all conversions and size arithmetic are checked (AS-12).
- **No new dependency, crate, `unsafe`, `#[allow]`, `#[ignore]`, deleted test or relaxed
  lint.** The standard library and existing workspace crates are sufficient. No backend
  trait, capability, payload version, state hash, storage layout or emulation behavior
  changes.
- **Tests at the lowest useful layer (AS-08):** edit-request validation, prefix/suffix
  exactness, truncation, anchor policy and error preservation in `chronofork-sessions`
  unit/integration tests; flag parsing, alias guard, atomic publish, block format, help
  and exit codes in `chronofork-cli` process tests; no emulation semantics re-asserted in
  the CLI.
- **AS-14 discipline.** `edit.rs` and `tas_edit.rs` are split by responsibility and must
  stay below the 800-non-test-line review trigger; measured line counts belong to
  evidence, not to this document (AS-13). `movie.rs` and `tas.rs` may grow only by the
  minimal accessor/dispatch wiring justified above.

### 9. Acceptance plan and current status

The executable acceptance plan is the [M3 rerecord acceptance
plan](../quality/m3-rerecord-acceptance.md), rows **M3-A35–M3-A44**. All ten rows are
`executed - passed` in the independent t10 repair-round-2 verification (2026-11-16); the
round-1 FAIL findings F1/F2/F3/F5 and gaps F6 were repaired and re-verified, and the
round-1 evidence is preserved. The plan covers:

| Row | Capability |
|---|---|
| M3-A35 | Editing model and suffix re-simulation exactness (prefix copy/verify, edited frame application, multi-index edits, differential reference against a full re-record, output self-verification) |
| M3-A36 | Truncation, index/count bounds and typed request errors (including dropped-suffix non-execution and no-op/huge values) |
| M3-A37 | Anchor handling: `keep-prefix` retention, `drop`, deterministic `rebuild`, binding, budgets, embedded checkpoint unchanged, one anchor format |
| M3-A38 | Branch metadata, identity and isolation (default/explicit id, parent edge and dangling parent, name, source/session/file non-mutation, edit chains, existing command acceptance) |
| M3-A39 | v1/v2 preservation, device-field boundary and reject-only compatibility |
| M3-A40 | `tas edit` process contract, output-alias guard, atomic publish, failure atomicity, streams, exit codes, help, declared help/dispatch delta |
| M3-A41 | Determinism, no host-dependent input, bounded resources, independent managers/players |
| M3-A42 | Module ownership, one codec/one edit implementation, no dependency/`unsafe`/`#[allow]`/`#[ignore]`, typed errors |
| M3-A43 | Non-regression: M3-A1–A34, M2/M1, full gate, demos, CLI/movie/probe baselines, release equality |
| M3-A44 | Scope honesty, deferred list, requirement/milestone accuracy, no implementation or M3-complete claim |

The pre-change baseline is `.research/m3-rerecord/baseline/` (captured
2026-11-15T23:49:31Z at HEAD `3f76d5f`, clean tree). The round-2 PASS is bound to HEAD
`3f76d5f` plus the repaired uncommitted paths recorded in
`.research/m3-rerecord/verification-round-2/tree/`; a later edit to an implementation path
invalidates it until re-verified. The dated gate result is the verifier-owned round-2
block in [quality gates](../quality/quality-gates.md).

### 10. Owner decisions (approved 2026-11-15)

The project owner approved the following decisions as proposed with the statement
"Aprovar ADR-0008 como proposto (Recomendado)" at 2026-11-15T21:09:29-03:00
(America/Fortaleza), 2026-11-16T00:09:29Z. They were implemented as approved under the
[M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md) and independently
verified for the deterministic synthetic backend (t10 repair-round-2 PASS, 2026-11-16).
The approval recorded the decision only and did not authorize a commit or push; the
verification record below is separate evidence.

1. **Functional edit model.** One `chronofork tas edit` command and one library edit
   function produce a new canonical movie by importing the source's embedded checkpoint,
   copying and verifying the unchanged prefix, applying edited `InputFrame`s, truncating
   if requested, re-simulating the suffix, recomputing every expectation and validating
   through the existing version constructor. The source movie value, file and any
   pre-existing session are never mutated. In-place editing, delta/patch sidecars and
   full re-record-without-prefix are rejected for this increment.
2. **Input-edit surface.** Full `InputFrame` replacement at the library layer; the CLI
   sets only port 0 (`--set-input INDEX=MASK`) and preserves ports 1–3, axes and keys. A
   v1 movie rejects a non-neutral extra-field replacement (no silent truncation); other
   device-field CLI editing is deferred.
3. **Truncation.** Zero-based `--truncate-after INDEX` retains `0..=INDEX` and drops the
   suffix; at least one step must be dropped; dropped steps are not executed. A no-op
   truncation is a `TruncationNoOp`; a metadata-only copy is `NoEditsRequested`.
4. **Rerecord semantics and no counter.** "Rerecord" means suffix re-simulation after an
   edit. Repeated edits form a chain of new artifacts with immediate-parent provenance.
   No rerecord counter, edit log, history field or format v3 is stored; only
   per-invocation `edits`/`truncate_after`/`prefix_steps`/`resimulated_steps`
   diagnostics and the parent edge exist. A persisted counter is deferred.
5. **Branch identity.** The output has a non-zero movie id (default source `+ 1`, explicit
   override distinct from the source), `parent_movie_id` always equal to the source id
   and not user-settable, an optional name never inherited from the source, and the
   passive dangling-parent rule unchanged. Source isolation and existing command
   acceptance are as stated in §4.
6. **Anchor rule.** Default `keep-prefix`: retain only anchors with
   `step_index < earliest affected step`, verbatim; explicit `drop` or `rebuild` (v2 only)
   are the alternatives. The embedded checkpoint is copied unchanged; anchors at or after
   the edit are never retained; retained/rebuilt mixing is deferred; no second anchor
   format and no anchor-based prefix skipping.
7. **Compatibility and bounds.** v1 → v1 and v2 → v2 with the same format/schema
   versions; no migration, upgrade, downgrade or silent truncation; reject-only device
   edits where a format cannot encode them; all indices, truncation, counts, anchor
   budgets and output size bounded by the inherited constants and checked before
   execution/allocation; typed edit errors preserving existing sources.
8. **CLI contract.** The exact flags, output block, stderr/exit contract, atomic
   sibling-temp+rename publish, source-path/alias guard, help and honesty text in §7, with
   the declared additive `tas` help/dispatch change and no other existing output change.
9. **Placement and quality.** New `chronofork_sessions::edit` and
   `chronofork_cli::tas_edit` modules; one codec/parser/predicate/import/capture/publish
   path and one comparison rule; no new dependency, crate, `unsafe`, `#[allow]`,
   `#[ignore]`, deleted test or relaxed lint; AS-14 discipline.
10. **Process.** Implementation starts only after this ADR is **Accepted** and under the
    [M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md); rows
    M3-A35–M3-A44 must be executed and independently verified. This design approval does
    not authorize a commit, push or merge, and no approval completes M3.

### 11. Deferred work and stated limitations

**In scope for this increment (implemented and independently verified for the
deterministic synthetic backend):** the functional input-edit/truncation model,
deterministic suffix re-simulation, prefix verification, branch artifact
identity/isolation, the anchor policy rules, v1/v2 preservation and bounds, the typed edit
boundary, the `chronofork tas edit` surface, the executable acceptance rows and honest
documentation.

**Deferred, not implemented and never to be claimed by this increment:**

- in-place movie editing, rewrite-on-open and mutation of an existing source artifact;
- full timeline insert/delete/reorder editing and any timeline, TUI or GUI editor;
- scripted or variable input files, per-step input scripts, text input logs and
  import/export adapters (BizHawk BK2 or others);
- a persisted or cumulative rerecord counter, edit history, undo/redo, markers, lag log,
  branch switching inside the player and automatic parent-chain resolution or traversal;
- real-hardware input TAS: the Mega Drive `step` ignores input, and wiring controllers is
  an emulation-semantics change under its own decision;
- checkpoint-reference movies, delta/patch sidecar formats and cross-format migration;
- mixing retained prefix anchors with rebuilt suffix anchors, and anchor-based skipping
  of edit prefix verification;
- CLI device-field editing of ports 1–3, axes or keys (library full-frame replacement
  only);
- O(1)/random-access seek, reverse execution, parallel/threaded edit or replay, and any
  performance, latency or capacity target;
- compression, encryption, authentication, deduplication and automatic retention/GC;
- HTTP API, MCP adapter, GUI and the Stage 6 daemon;
- cross-backend canonicalization, cross-platform runtime determinism and any
  hardware-accuracy claim;
- M3 completion. Rerecord/branch editing does not complete M3; real-hardware input TAS
  and the other deferred items remain open.

**Stated limitations.** Editing costs O(output steps) execution from the embedded
checkpoint and repeats the prefix work on every invocation; a late edit is close to a
full replay. The output is a new file, not a patch, so branch chains consume storage and
the parent chain is not resolved automatically. Movie ids are labels and can collide.
A cumulative rerecord count does not exist; the printed diagnostics describe one
invocation. Anchor coverage after an edit is reduced unless the caller explicitly
rebuilds it, and rebuilding costs checkpoint bytes. A v1 source cannot carry anchors or
multi-device fields, and a device-field edit that a format cannot encode is rejected
rather than truncated. Prefix verification proves the source prefix reproduces its own
recorded expectations, but the suffix is only as deterministic as the backend; the
acceptance scope is the deterministic synthetic backend on Linux. A movie whose embedded
checkpoint is incompatible with this build is rejected at import, and comparison can
still compare records without proving replay. The implementation and verification claim is
limited to the deterministic synthetic backend on Linux; the deferred items above remain
unimplemented and M3 is **not complete**.

## Rationale

- **A new artifact is the only model that preserves both provenance and the source.**
  In-place mutation makes "source unchanged" impossible and turns a failed edit into data
  loss. A delta sidecar would need a second artifact protocol and a new compatibility
  contract. A new canonical movie reuses every accepted reader, replay and comparison
  path and carries an explicit parent edge.
- **Verifying the prefix is what makes the branch trustworthy.** The edit does not trust
  the source's expected hashes; it re-executes every prefix step and compares. A source
  with a tampered or corrupt prefix fails instead of being propagated into a new artifact.
  This is stronger than copying bytes and cheaper than a second full verification pass.
- **Suffix re-simulation is the minimal correct way to recompute expectations.** An
  input edit changes state from that step onward, so every later `(outcome tick, state
  hash)` is stale. There is no valid shortcut: the suffix must be executed on the same
  deterministic backend from the same checkpoint. This is also exactly what "rerecord"
  means in upstream TAS tooling, without adopting its UI, counters or partial-invalidity
  heuristics.
- **No counter and no v3 keep the accepted bytes intact.** A stored count would alter the
  format and invalidate verified evidence for a value upstream research itself calls a
  heuristic. The parent edge and per-invocation diagnostics provide provenance and
  observability without touching canonical bytes.
- **Retaining prefix anchors is safe because their steps are unchanged and re-verified.**
  An anchor at or after an edit describes a state that no longer exists, so it must be
  dropped. Rebuilding is useful but optional and costs bytes, which is why it is explicit
  rather than the default.
- **Format preservation is non-negotiable.** A v1 movie edited into a v2 movie (or the
  reverse) would silently change the artifact's compatibility and invalidate the
  source/output relationship. Using the source's own validating constructor keeps the
  version and schema fixed and reuses all rejection rules.
- **The CLI stays a thin adapter with a hard source-protection rule.** Path/alias
  rejection plus a sibling temp+rename means a failed edit can never corrupt or replace
  the source, and windowed/partial output is impossible. Reusing the existing publish
  helper keeps one implementation.
- **One new library module and one new CLI module match the responsibility boundary.**
  The session layer owns execution policy; the CLI owns process I/O, flags and reporting.
  Splitting by responsibility keeps the format module and the CLI TAS module within the
  AS-14 review trigger and keeps one codec.

## Consequences

Positive (implemented and verified for the deterministic synthetic backend):

- A recorded movie's inputs can be edited and its suffix deterministically re-simulated
  into a new canonical branch artifact, with the unchanged prefix verified rather than
  assumed.
- Branch artifacts carry explicit immediate-parent provenance and preserve the source
  file, value and sessions, so experiments are reproducible and non-destructive.
- The v1 and v2 formats, all existing commands and the M1/M2/M3 evidence stay unchanged;
  the output is accepted by play, verify, compare, anchored seek and studio.
- Anchor handling is explicit and safe: valid prefix anchors survive by default, invalid
  ones are dropped, and rebuilding is a bounded opt-in.
- All index, truncation, anchor and size bounds are typed and checked before execution,
  and the source path is structurally protected by the atomic publish rule.

Negative / costs:

- Each edit re-executes the output length from the embedded checkpoint; editing a late
  step costs nearly a full replay, and repeated edits repeat that work.
- Branch chains create new full movie files; there is no delta storage, deduplication or
  automatic retention.
- The CLI gains a subcommand, eight flag families, an output block and a help/dispatch
  delta that must be re-baselined; the library and CLI each gain a module and tests.
- Anchor coverage is lost from the earliest affected step unless the caller rebuilds,
  which costs checkpoint captures and bytes.
- The absence of a cumulative rerecord counter may surprise users accustomed to TAS
  tooling; help and docs must explain that the printed values are per-invocation.

Risks / limitations:

- The acceptance claim is the deterministic synthetic backend on Linux only; the Mega
  Drive `step` ignores input, so no hardware input TAS, branch playback on hardware or
  cross-backend portability is claimed.
- A source movie whose prefix does not reproduce its own records cannot be edited; that
  is intentional and reported as the existing typed divergence, not as a silent copy.
- Movie ids are logical labels, so branch ids can collide; nothing resolves or enforces
  global uniqueness.
- Anchor rebuilding and prefix verification are deterministic within the documented
  backend conditions; rebuilt anchors use the canonical fixed snapshot/origin identity,
  so caller-manager history does not affect their bytes, while v1/v2 recording retains its
  session-assigned identity.
- This ADR is **Accepted** (owner-approved 2026-11-15T21:09:29-03:00), implemented and
  independently verified only for the deterministic synthetic backend (t10
  repair-round-2 PASS, 2026-11-16). It asserts no real-hardware input TAS, no
  cross-backend portability, no M3 completion and none of the deferred items in §11.

## Status

**Accepted (owner-approved 2026-11-15T21:09:29-03:00, America/Fortaleza;
2026-11-16T00:09:29Z); implemented and independently verified for the deterministic
synthetic backend (t10 repair-round-2 PASS, 2026-11-16).** The project owner explicitly
approved ADR-0008 as proposed with the statement "Aprovar ADR-0008 como proposto
(Recomendado)" at that time. The approval recorded the decision only: it did not assert
implementation or verification and authorized no commit, push, tag or merge.

The design was drafted on 2026-11-15 from the accepted ADR-0004/0005/0006/0007 baseline,
the v1/v2 movie, replay and player code at HEAD `3f76d5f`, the existing M3 acceptance
plans and the research in [TAS and debugging](../research/tas-and-debugging.md)
§4.3/§5.3/§10.2. The increment was then implemented under the approved plan and
independently verified by t10 repair-round 2: every M3-A35–A44 row is `executed - passed`,
the round-1 FAIL findings F1/F2/F3/F5 and gaps F6 were repaired and re-verified, and the
round-1 evidence is preserved. The PASS is bound to HEAD `3f76d5f` plus the repaired
uncommitted paths; no commit or push was made. The claim is limited to the deterministic
synthetic backend on Linux; real-hardware input TAS and the other deferred items remain
unimplemented, and M3 is **not complete**.

## Evidence

This section records the design inputs, the owner approval, the implemented contract and
the executed verification.

- **Owner approval:** the project owner's statement "Aprovar ADR-0008 como proposto
  (Recomendado)" at 2026-11-15T21:09:29-03:00 (America/Fortaleza),
  2026-11-16T00:09:29Z, approving the decisions in §10 as proposed. The approval recorded
  the decision only; it did not assert implementation or verification and authorized no
  commit, push, tag or merge.
- **Implemented contract (t10 verified):** `crates/chronofork-sessions/src/edit.rs`
  (single `edit_movie` plus `MovieEdit`, `EditMeta`, `AnchorPolicy`, `EditOutcome`,
  `EditReport`, `EditError`), `crates/chronofork-sessions/tests/edit.rs`,
  `crates/chronofork-cli/src/tas_edit.rs`, `crates/chronofork-cli/tests/tas_edit.rs`, and
  the `main.rs`/`tas.rs` wiring. The edit reuses the one movie codec, validating
  constructors, snapshot/anchor parser, compatibility predicate, import path and publish
  helper; rebuilt anchors use the shared capture path with the canonical fixed
  snapshot/origin identity. No new crate, dependency, `unsafe`, `#[allow]` or `#[ignore]`
  was added, and no v1/v2 recording bytes changed.
- **Executed verification (t10 repair-round-2 PASS, 2026-11-16):** every M3-A35–A44 row is
  `executed - passed`; the full gate (fmt, check, clippy `-D warnings`, workspace tests,
  rustdoc `-D warnings`, `./scripts/check.sh`, `git diff --check`) exited 0 with every
  target at or above the pre-change baseline; both demos were byte-identical to the
  baseline; the v1/v2/M1/M2/M3 regression checks, edit-target tests, independent harness,
  CLI probes, mutation probes and structural/suppression audits passed. Raw evidence:
  `.research/m3-rerecord/verification-round-2/` (`SUMMARY.txt`, `rows/`, `target-matrix.txt`,
  `gate/`, `targets/`, `cli/`, `edit/`, `demos/`, `probes/`, `audit/`, `tree/`,
  `MANIFEST.sha256`; manifest verified). Round-1 evidence is preserved under
  `.research/m3-rerecord/verification/`; round-1 blockers F1/F2/F3/F5 and gaps F6 were
  closed and re-executed.
- **Accepted contracts this ADR extends, not restates:**
  [ADR-0004](0004-tas-movie-format.md) (v1 container, limits, record/playback, first
  divergence, deferred list), [ADR-0005](0005-partial-replay-and-branch-comparison.md)
  (range replay, seek, comparison), [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md)
  (v2 container, anchors, anchored seek, schema-2 input) and
  [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md) (in-process rewind and
  `tas studio`), with their executed acceptance plans
  [M3-A1–A12](../quality/m3-tas-replay-acceptance.md),
  [M3-A13–A17](../quality/m3-partial-replay-acceptance.md),
  [M3-A18–A27](../quality/m3-format-v2-acceptance.md) and
  [M3-A28–A34](../quality/m3-rewind-acceptance.md).
- **Movie implementation read:** `crates/chronofork-sessions/src/movie.rs`
  (`Movie::from_bytes`/`to_bytes`, `Movie::from_parts`, `Movie::from_v2_parts`,
  `Movie::record`, `Movie::record_v2`, `Movie::verify`, `MovieStep`, `MovieMeta`,
  `MovieAnchor`, `MovieError`, the v1/v2 limits and the single anchor parser use).
- **Replay implementation read:** `crates/chronofork-sessions/src/replay.rs`
  (`ReplayRange`, `ReplayOutcome`, the single replay loop, `compare_movies`).
- **Session/manager implementation read:** `crates/chronofork-sessions/src/session.rs`
  and `manager.rs` (`create_session_from_snapshot_bytes`, `snapshot`, `snapshot_record`,
  `snapshot_bytes`, `step`, `restore`, `map_backend_compatibility`).
- **Input implementation read:** `crates/chronofork-core/src/input.rs` (`InputFrame`,
  `new`, `buttons`, `with_devices`, `ports`, `axes`, `keys`, `NEUTRAL`).
- **CLI implementation read:** `crates/chronofork-cli/src/tas.rs` (subcommand dispatch,
  flag parsers, movie read bound, `write_movie_file` sibling-temp+rename publish, block
  writers, help), `studio.rs` and `main.rs` (`begin_record`, `CliError`, dispatch).
- **Deterministic backend read:** `crates/chronofork-test-backend/src/lib.rs` (one
  `InputFrame` per step, one tick per step, `APPLY_INPUT`); Mega Drive input is ignored
  in `crates/chronofork-megadrive/src/backend.rs`.
- **Pre-change baseline (precondition for a later implementation stage, not evidence of
  this design):** `.research/m3-rerecord/baseline/` (captured 2026-11-15T23:49:31Z,
  branch `agent/m3-complete`, HEAD `3f76d5f`, clean tree; `git/`, `env/`, `gate/`,
  `demos/`, `fixtures/`, `tools/`).
- **Research basis:** [TAS and debugging](../research/tas-and-debugging.md) §4.3
  (rerecording is input truncation/invalidation/savestate load; `Rerecords` is a
  heuristic), §4.5 (edit invalidates states after the edited frame, then restore/replay),
  §5.3 (state caching is session-layer policy) and §10.2 ("Rerecord count is a
  heuristic"; keep it as user-facing metadata, never a determinism proof).
- **Acceptance plan:** [M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md),
  rows M3-A35–M3-A44, all `executed - passed` (t10 repair-round-2 PASS, 2026-11-16).
- **Tree state:** branch `agent/m3-complete`, HEAD
  `3f76d5ffee680d2c7271c7c1d81d8c5a44055675` plus the uncommitted rerecord
  implementation/test/doc paths recorded in
  `.research/m3-rerecord/verification-round-2/tree/`. The verifier's only tracked edit was
  the dated round-2 PASS block in
  [quality gates](../quality/quality-gates.md); no commit, push, tag or merge was made.
  This promotion changes documentation status/evidence text only.

## Change control

- A change to the editing semantics, prefix-verification rule, suffix-re-simulation
  rule, truncation rule, branch identity/parent rule, anchor rule, compatibility matrix,
  typed bounds, CLI flags, output block, atomicity/source-protection rule or module
  ownership requires an explicit amendment to this ADR, an updated
  [M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md), and
  re-verification of the affected rows. It must not be made by quietly renaming a test,
  raising a constant or loosening an assertion.
- The owner approval recorded in the header (2026-11-15) moved the status to Accepted.
  Approval changes only the status/approval text; it does not assert implementation or
  verification and does not authorize a commit, push or merge.
- Implementation and verification status are promoted only from executed evidence
  (per-row commands, exit codes, artifacts and an independent verifier PASS), never from
  this document, the acceptance plan, a code review or a demo run.
- Deferred items in §11 stay deferred; a later increment must design real-hardware input
  TAS, import/export, a timeline editor, a persisted rerecord counter or mixing retained
  and rebuilt anchors on their own decisions and evidence.
- If implementation would alter a v1/v2 movie byte, a state hash, an existing CLI
  default block or an M3-A1–A34 result, stop: that is a change to an accepted contract
  and needs its own decision.
- **2026-11-15 (owner approval, t8).** ADR-0008 moved from Proposed to Accepted on the
  owner's statement "Aprovar ADR-0008 como proposto (Recomendado)" at
  2026-11-15T21:09:29-03:00 (America/Fortaleza), 2026-11-16T00:09:29Z. The header, §1,
  §10, the Status section, the risk note, the [ADR index](README.md) and the documentation
  status/index lines record the approval. No contract detail, limit, command, typed
  boundary, deferred item, capability claim or implementation/verification status changed;
  no commit or push was made.
- **2026-11-15 (repair-round clarifications, t9).** No decision, limit, flag, block
  field, canonical movie byte, embedded-checkpoint byte or v1/v2 recording byte changes.
  The clarifications record implementation detail required to satisfy the accepted
  contract: rebuilt anchor envelopes carry a fixed canonical snapshot/origin-session
  identity (§5.3, §8) while v2 recording keeps its session-assigned identity; the CLI
  rejects a non-UTF-8 command-line value through the typed error path instead of
  panicking (§7); the CLI adapter/test target is implemented at the accepted `tas_edit`
  spelling (§8, A42.1); the speculative `EditOutcome::into_movie` accessor was removed;
  and executable size-budget fixtures cover the per-anchor, cumulative and writer
  output-size rejections (M3-A36.6, M3-A37.5) in the edit test target. These are
  clarifications under this section, not a change to the accepted decision; the
  capability and verification status stays whatever the executed evidence and the
  independent verifier record.
- **2026-11-16 (verification and documentation promotion, t10/t6).** The independent
  repair-round-2 verification executed M3-A35–A44 on the repaired uncommitted tree at HEAD
  `3f76d5f` and returned **PASS**; the round-1 FAIL findings F1/F2/F3/F5 and gaps F6 are
  closed. This promotion updates the status, verified scope, evidence and deferred-list
  framing from that record. No contract detail, limit, CLI flag, output block field,
  format/schema version, v1/v2 recording byte, typed boundary, acceptance criterion or
  deferred item changed; the round-1 evidence remains preserved and the deferred items in
  §11 remain unimplemented. M3 is **not complete**; no commit or push was made.
