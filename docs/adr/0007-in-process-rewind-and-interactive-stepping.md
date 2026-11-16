# ADR-0007: In-Process Rewind State Cache and Interactive Stepping

- **Status:** **Accepted** (owner-approved 2026-11-15T19:17:48-03:00,
  America/Fortaleza; 2026-11-15T22:17:48Z); **implemented and independently verified for
  the deterministic synthetic backend** (t5 PASS, 2026-11-15). The verified scope is the
  deterministic synthetic backend on Linux; no hardware input or cross-backend claim is
  made and M3 remains incomplete.
- **Date:** 2026-11-15
- **Decision owner:** Project owner (approved 2026-11-15)
- **Approval evidence:** the project owner approved ADR-0007 as proposed with the statement
  "Aprovar ADR-0007 como proposto (Recomendado)" at 2026-11-15T19:17:48-03:00
  (America/Fortaleza), 2026-11-15T22:17:48Z. The approved decisions are the eight recorded
  in §10, unchanged from the proposal. The approval authorizes implementation under the
  [M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md) only; it does not
  authorize a commit or push and does not assert implementation or verification. Drafted by
  the documentation architect (`t2`); the approval was recorded by `t8`.
- **Supersedes:** none. This ADR extends [ADR-0004](0004-tas-movie-format.md),
  [ADR-0005](0005-partial-replay-and-branch-comparison.md) and
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md) without changing any
  movie byte, format version, schema version, limit, anchor rule, emulation semantic,
  state hash, storage layout or previously verified behavior.
- **Implementation status:** **Implemented and independently verified** for the
  deterministic synthetic backend (t5 PASS, 2026-11-15). `chronofork_sessions::player`
  (with the permitted `player/cache.rs` split) and `chronofork_cli::studio` implement the
  contract; the M3-A28–A34 rows in the [M3 rewind acceptance
  plan](../quality/m3-rewind-acceptance.md) are `executed - passed` against the recorded
  tree and evidence. The deferred scope in §9 is unchanged; the verified scope is the
  deterministic synthetic backend on Linux, and M3 is **not complete**.
- **Related:** [execution model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [M3 rewind acceptance
  plan](../quality/m3-rewind-acceptance.md), [M3 first-increment acceptance
  plan](../quality/m3-tas-replay-acceptance.md), [M3 increment-2 acceptance
  plan](../quality/m3-partial-replay-acceptance.md), [M3 format-v2 acceptance
  plan](../quality/m3-format-v2-acceptance.md), [research: TAS and
  debugging](../research/tas-and-debugging.md),
  [functional requirements](../requirements/functional.md),
  [non-functional requirements](../requirements/non-functional.md),
  [milestones](../roadmap/milestones.md), [anti-slop
  policy](../quality/anti-slop-policy.md), [quality gates](../quality/quality-gates.md).

> **Accepted (owner-approved 2026-11-15); implemented and independently verified (t5
> PASS, 2026-11-15).** The project owner approved this ADR as proposed at
> 2026-11-15T19:17:48-03:00 (America/Fortaleza), 2026-11-15T22:17:48Z, and the increment
> was implemented under the [M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md)
> and independently verified: every M3-A28–A34 row is `executed - passed`. The verified
> scope is the deterministic synthetic backend on Linux, and the increment does not complete
> M3. Rerecord/branch editing and real-hardware input TAS remain later increments and are
> not designed here beyond the deferred list in §9.

## Context

Read on 2026-11-15 in the `agent/m3-complete` checkout at HEAD
`b8b0f935e40e08854c2155f1b5208f6fc91424d8` (clean tree; branch `agent/m3-complete`). The
following accepted, implemented and independently verified pieces are the baseline this
proposal builds on. None of them is restated here as a new decision.

- **Movie format v1 and v2** ([ADR-0004](0004-tas-movie-format.md),
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md), M3-A1–A12 and
  M3-A18–A27 PASS). `chronofork_sessions::movie` owns exactly one reader/writer
  (`Movie::from_bytes`, `Movie::to_bytes`), one validating constructor per version, the
  recorder, the full-range verifier, the ordered anchor table and the single
  `Snapshot::from_bytes`-based embedded-checkpoint decoder. `Movie` exposes
  `start_tick`, `end_tick`, `step_count`, `snapshot_bytes`, `steps()` and `anchors()`;
  `MovieStep` exposes `outcome_tick()`, `input()` and `state_hash()`.
- **Ranged replay, seek and comparison** ([ADR-0005](0005-partial-replay-and-branch-comparison.md),
  M3-A13–A17 PASS). `chronofork_sessions::replay` owns `ReplayRange` (the `(from, until]`
  window), its typed validation, the single replay loop `replay_validated`, the typed
  `ReplayOutcome` and the pure `compare_movies`/`ComparisonReport`.
- **Anchor-based seek** ([ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md),
  M3-A21 PASS). A v2 ranged replay may start from the nearest embedded anchor at or
  before the prefix end; it imports that anchor through the same checkpoint path and
  documents that it does not re-execute the pre-anchor prefix (`prefix_verified=0`).
- **Session and manager model** (`crates/chronofork-sessions/src/session.rs`,
  `manager.rs`). A `Session` owns exactly one mutable `Box<dyn Backend>` plus
  `initial_state`, a `snapshots: BTreeMap<SnapshotId, Snapshot>` map and
  `next_snapshot_id`. `Session::snapshot()` captures the backend, **allocates a new
  `SnapshotId` and inserts the canonical `Snapshot` into that map**; `restore(id)`
  restores a stored checkpoint and maps incompatibility through
  `map_backend_compatibility`; `snapshot_bytes(id)` serializes a stored checkpoint;
  `inspect()` returns tick, state hash, halted flag, registers and a memory window.
  `SessionManager` is created from an `Arc<dyn BackendFactory>`, owns the session
  registry, and `create_session_from_snapshot_bytes` is the one import path: it decodes
  the canonical envelope, creates a backend, restores it (so `validate_envelope` remains
  the single compatibility predicate) and registers a fresh root session.
- **CLI TAS adapter** (`crates/chronofork-cli/src/tas.rs`, `main.rs`). `tas
  record|play|verify|compare` print stdout-only `key=value` blocks separated by one blank
  line (`begin_record` in `main.rs`), send diagnostics to stderr as `chronofork: <error>`
  and exit 0/1. `tas.rs` and `main.rs` have no in-file test module; the process targets
  are `crates/chronofork-cli/tests/tas.rs` and `tests/session_storage.rs`.
- **Deterministic acceptance backend.** `chronofork-test-backend` consumes one
  `InputFrame` per step, advances exactly one tick per step and exposes a canonical
  state body; the Mega Drive slice still ignores input. All M3 evidence is
  synthetic-backend only and Linux-hosted.
- **Deferred items this proposal addresses.** ADR-0004 §12 deferred "frame advance,
  rewind and seek via checkpoints"; ADR-0005 §8 deferred "an in-process rewind/ring
  buffer, state-cache/eviction policy, interactive stepping, pause/resume and any
  timeline or UI surface"; ADR-0006 §13 deferred "in-process rewind/state cache and
  interactive stepping". At design time, [Milestones](../roadmap/milestones.md) kept
  "In-process rewind buffer/state cache and interactive stepping" unchecked, the
  [roadmap](../roadmap/roadmap.md) Stage 3 bullet was open, `FR-TAS-001` was **Partial**,
  `FR-EXEC-007` was **Planned**, and `tas --help` stated that rewind caching and
  interactive stepping were not implemented.
- **Research context.** [TAS and debugging](../research/tas-and-debugging.md) §5.3
  records that BizHawk's `IStateManager` is a bounded state cache whose
  `GetStateClosestToFrame` must always return a state at or before the requested frame
  (guaranteed by holding frame 0), with paged and tiered managers and a ring buffer;
  the reusable idea is "keep dense recent states, sparse older states, always keep frame
  0". §6.3 records that TAStudio "does not have a random-access seek": it loads the
  nearest prior cached state and replays forward, so seek cost is proportional to the
  distance from that state. §10.2 concludes "Seeking is replay, not random access" and
  that O(1) seek must not be promised. §6.2 places pause/seek semantics in the session
  layer, not the backend contract. This is upstream-derived context, not a ChronoFork
  decision, and none of it is adopted verbatim.

## Problem

How should ChronoFork add an in-process rewind buffer/state cache and interactive
stepping on top of the verified M3 baseline so that:

1. **Nothing accepted changes.** No movie byte, format/schema version, limit, anchor
   rule, decode/rejection category, replay result, state hash, scheduler behavior,
   storage layout or CLI output of an existing command changes, and the M1/M2/M3
   evidence stays valid.
2. **Ownership is explicit.** One in-process owner holds one live session and its
   cache; there is no global or static mutable state, no backend-trait change, no second
   session registry and no storage or file write from the library.
3. **The cache does not grow the session's checkpoint map.** A cached state is a
   canonical backend state, not a new durable checkpoint: capturing one must neither
   allocate a `SnapshotId` nor insert into `Session::snapshots`, whose size is otherwise
   unbounded by design and whose entries carry canonical-checkpoint semantics.
4. **Replay stays the only truth mechanism.** A rewind restores a state that this
   process actually verified, replays the recorded inputs forward, verifies every
   replayed expectation and reports the exact first divergence; it never interpolates,
   never applies a recorded hash as state and never claims O(1) random access.
5. **Resources are bounded and reported.** Cache entries are bounded by count and by a
   deterministic byte metric, evicted in a documented deterministic order, and the
   reported memory is reproducible rather than allocator-dependent.
6. **Rewind is exact at every target class.** A target may precede the earliest cached
   state, follow an eviction, equal the current position, or equal the movie start; each
   case lands on the exact recorded tick with the recorded state hash or a typed
   rejection, never a clamp or an approximation.
7. **Interactive stepping has one explicit contract.** A stdin-driven `tas studio`
   command owns exactly one in-process player, verifies expectations on every forward
   step, prints stable `key=value` data, writes no storage or files and behaves
   deterministically at EOF and on invalid input.
8. **Isolation and determinism are structural.** The player cannot touch a pre-existing
   caller session, two players are independent, and identical command sequences produce
   identical bytes with no host time, entropy, process id or path in valid output.
9. **The increment stays small and honest.** It adds no crate, dependency, format
   version, migration, GUI, HTTP or MCP surface, keeps one snapshot parser, one
   compatibility predicate, one alignment rule and one replay/verification comparison,
   and does not pretend to implement rerecord/branch editing or real-hardware input TAS.

## Alternatives

### Ownership and module placement

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. New `chronofork_sessions::player` module; a `Player` owns a private `SessionManager` with exactly one imported session, the decoded `Movie` and its bounded cache** | One explicit owner per player; the one-session rule is structural (no other session exists in the player's manager), so pre-existing caller sessions cannot be mutated; the session layer keeps canonical-state ownership; testable at the lowest useful layer; no new crate, dependency or backend change | The player duplicates a manager instance per player (cheap, in-memory only); the cache needs a crate-internal capture path that bypasses the session checkpoint map | **Accepted** (chosen) |
| B. Cache fields inside `Session` | No new type | Every session would pay rewind policy and memory even when no one rewinds; `Session`'s snapshot map is the canonical checkpoint map, not a bounded cache; expands the public session contract for one consumer; mixes lifecycle and TAS policy | Rejected |
| C. Cache inside `SessionManager` | One cache per manager | The manager is a registry and ownership boundary, not an execution policy; a shared cache would couple unrelated sessions and weaken the one-owner rule | Rejected |
| D. CLI-owned cache (`chronofork-cli` holds the states) | No library change | The CLI would own canonical emulated state, violating AGENTS.md architectural rule 5, and the behavior could not be tested at the session layer (AS-08) | Rejected |
| E. New crate (for example `chronofork-rewind`) or a new dependency | Clean dependency boundary | New crate/dependency is a liability (AS-11, AGENTS.md dependency rules); `chronofork-sessions` is the correct layer; no demonstrated need | Rejected |
| F. Global/static or thread-local cache | Trivial access | Hidden global mutable state is forbidden; two players would interfere and tests could not be isolated | Rejected |
| G. Cache in `chronofork-core` | Very low layer | Core owns values (ticks, hashes, input), not sessions or execution policy; would invert the dependency direction | Rejected |

### Cache entry payload and capture path

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Hold typed `BackendSnapshot` values captured through one crate-internal capture/restore pair that allocates no `SnapshotId` and inserts nothing into `Session::snapshots`** | No per-capture serialization, checksum or allocation of checkpoint identity; restore reuses the existing backend `restore` path and `map_backend_compatibility`, so `validate_envelope` remains the single predicate; the session's checkpoint map is provably untouched | Adds two crate-private session/manager accessors (not public API); states live only in memory, so nothing survives the process | **Accepted** (chosen) |
| B. Reuse `Session::snapshot()` / `SessionManager::snapshot()` | No new accessor | Every cached state allocates a `SnapshotId` and grows the session's `BTreeMap` without any eviction hook; the map would become a de-facto cache with checkpoint semantics, breaking the "bounded cache" property and the snapshot-identity contract | Rejected |
| C. Hold serialized canonical bytes via `Snapshot::to_bytes()` per entry | Uniform byte representation; a future persistent cache could reuse it | Serializes and SHA-256s on every capture, then re-parses on every restore; needs a `Snapshot` (and therefore an id) or a synthetic envelope; duplicates every state in memory; no benefit until a persistent cache exists (deferred) | Rejected for this increment |
| D. Hold only tick/hash and always replay from the embedded checkpoint | No memory at all | Rewind cost is always the full distance from the movie start; the increment would add no cache, so its purpose is unmet | Rejected |
| E. Hold the backend payload only, without the canonical envelope fields | Slightly smaller | Creates a second state representation and a second compatibility rule (AS-01/AS-02), and bypasses the existing restore contract | Rejected |

### Capture cadence

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Every `N` recorded steps, `N` fixed per player (default 1), capturing the state after recorded steps `N-1, 2N-1, …`, the same index convention as v2 anchors** | Simple, deterministic, reuses the anchor convention so there is one cadence rule; `N = 1` gives the most responsive single-step stepping; `N > 1` trades stepped-rewind cost for longer coverage | A rewind of one step costs up to `N-1` replayed steps; with small `N` and a count bound, coverage is shallow | **Accepted** (chosen) |
| B. Capture after every step, no interval | Maximal responsiveness | Same as `N = 1`; an interval is still needed to let a caller trade memory, and for a real backend a full state per step is expensive | Rejected as a separate mode (equivalent to `N = 1`) |
| C. Tiered or exponentially spaced states (dense recent, sparse old) | Better coverage for the same memory; matches the research idea | Extra policy, more eviction cases and tests, and a second cadence rule; not needed to prove in-process rewind | Deferred to a later decision |
| D. Capture only when a rewind is requested | No work during forward execution | Cannot rewind before the current position without re-executing from the floor anyway; no coverage benefit; cache contents would depend on request timing | Rejected |
| E. Seed the cache from v2 anchors | No new capture work for v2 movies | Anchors are optional movie artifacts; importing them per start would add a second start-state source with the documented pre-anchor prefix caveat (ADR-0006 §6), break the invariant that every cached state was verified in this process, and make first-divergence attribution ambiguous; v1 movies have none | Rejected for this increment (anchors stay a one-shot seek optimization) |

### Eviction policy

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Evict the lowest recorded step index first until both the count and byte bounds hold; the floor (embedded checkpoint) is outside the cache and always retained; the newest entry is never the victim unless it alone exceeds the byte limit (typed error)** | Deterministic, trivially testable, keeps a dense recent window that matches interactive stepping, no access-order state, no timing dependence | Rewind beyond the retained window falls back to the floor and replays a possibly long prefix | **Accepted** (chosen) |
| B. LRU by access order | Reuses recently visited regions | Eviction depends on the command history more subtly, so cache contents are harder to reason about and to document; no benefit for a playhead-local tool | Rejected |
| C. Evict largest entry first | Frees the most bytes per eviction | May evict the most useful recent state, and forces a size-ordering structure alongside the ordered map | Rejected |
| D. Stop capturing when full | Bounds are trivially met | Coverage silently stops improving and the newest states, which matter most for stepping, may be missing | Rejected |
| E. Random or allocator-driven eviction | Simple | Nondeterministic cache contents and reported metrics; violates the determinism requirement | Rejected |

### Rewind strategy

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Choose the greatest verified cached state at or before the target, or the floor; restore it; replay recorded inputs forward through the target verifying every step; report the first divergence** | Exact; cost bounded by the distance from the chosen start; reuses the single verification rule; the start state is always a state this process verified, so a rewind cannot silently start from an unverified prefix | Rewind cost grows with the gap to the nearest retained state; a long rewind can approach a full replay | **Accepted** (chosen) |
| B. Step the backend backwards | Constant cost | No backend provides reverse execution; adding it would change the backend trait and every backend's semantics | Rejected |
| C. Restore the target's recorded state hash as state | O(1)-looking | A hash is not restorable state; this would fabricate state and invalidate determinism | Rejected |
| D. Interpolate between cached states | Smoother reporting | Emulation has no interpolation; every reported tick/hash must be observed | Rejected |
| E. Keep rewind as a new invocation only (`play --until <earlier tick>`) | Already verified (ADR-0005) | Not in-process, no cache reuse, repeats the full prefix on every step, and cannot support interactive stepping; remains available but does not satisfy this increment | Rejected as the increment (retained as existing behavior) |
| F. Persistent on-disk cache | Survives processes | New format, storage writes, eviction and compatibility rules; explicitly out of scope | Deferred |

### Interactive surface

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. `chronofork tas studio --movie <FILE>`: a stdin-driven line protocol that owns one in-process player and prints one `key=value` block per command** | Scriptable and testable through stdin/stdout; no tty, no dependency, no GUI; one process holds exactly one explicit player; matches the existing CLI output style | The block protocol must be specified exactly (fields, order, exit/EOF, invalid input) | **Accepted** (chosen) |
| B. One-shot `tas play --until` per step | Reuses existing code | Not interactive; each `back` re-simulates from the checkpoint; no cache; cannot maintain a player | Rejected |
| C. TUI/REPL with line editing (readline, ratatui) | Nicer for humans | New dependency, terminal assumptions, hard to test in CI and non-deterministic rendering; the GUI stage is separate | Rejected for this increment |
| D. HTTP/MCP surface | Remote and agent-friendly | Stage 6 scope; no transport in this increment; canonical-state ownership questions | Deferred |
| E. GUI timeline | Rich | Stage 7; must never own canonical state | Deferred |

### Invalid interactive input

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Fail closed: validate the whole command line first; on any parse, boundary or alignment error print `chronofork: <error>` to stderr and exit 1 without changing the player position** | Deterministic, testable, scripted runs cannot hide a typo; mirrors the existing CLI's typed-failure exit code | A human must restart the studio after a typo | **Accepted** (chosen) |
| B. Print the error and continue | Friendlier interactive use | A scripted run can silently proceed from a wrong position; output would need an error block and a second contract | Rejected |
| C. Ignore unknown lines | Simplest | Silent failure; unacceptable for a tool whose purpose is exact stepping | Rejected |

## Decision (accepted 2026-11-15)

### 1. Scope and canonical home

This ADR is the canonical home for the in-process rewind state cache, the player
semantics and the interactive stepping contract. [ADR-0004](0004-tas-movie-format.md)
remains canonical for the movie bytes, [ADR-0005](0005-partial-replay-and-branch-comparison.md)
for range replay and comparison, and [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md)
for v2 anchors and schema-2 input. Architecture, requirements, roadmap and acceptance
documents link here instead of restating the cache bounds, cadence, rewind rule or CLI
protocol.

This ADR is **Accepted** (owner-approved 2026-11-15; §10), implemented and independently
verified for the deterministic synthetic backend (t5 PASS, 2026-11-15; §8 and Evidence).
The implementation followed the [M3 rewind acceptance
plan](../quality/m3-rewind-acceptance.md) rows M3-A28–M3-A34; no commit, push or merge was
authorized or made. The
public names below (`Player`, `CacheOptions`, `PlayerState`, `SeekReport`, `StartKind`,
`PlayerError`, constants) describe the contract shape; spelling or parameter grouping may
be refined during implementation, but the ownership rule, the captured state
representation, the bounds, the cadence convention, the rewind semantics, the typed
rejection boundaries and the CLI protocol are the contract.

The increment adds no crate, dependency, `unsafe`, `#[allow]`, `#[ignore]`, deleted test
or relaxed lint. It changes no backend trait, snapshot format, state hash or scheduler
behavior.

### 2. Ownership and module placement

**New `crates/chronofork-sessions/src/player.rs`** (module
`chronofork_sessions::player`, exported from `lib.rs`). It owns:

```text
Player
  manager: SessionManager           private; contains exactly one session
  session: SessionId                the player's only session, imported at open
  movie: Movie                      decoded movie; read-only source of truth for inputs
  position: Position                current verified position (applied step count + tick)
  cache: StateCache                 private bounded cache (not public API)
  options: CacheOptions             interval, capacity, byte limit
```

Rules:

1. **One live session per player, structurally.** `Player::open` takes an
   `Arc<dyn BackendFactory>` (not a caller `SessionManager`), creates a **private**
   `SessionManager`, and imports the movie's embedded checkpoint through the existing
   `SessionManager::create_session_from_snapshot_bytes` path. Exactly one session exists
   in that manager, so no pre-existing caller session can be addressed, mutated or
   closed by the player. Two players are independent: they have separate managers,
   sessions and caches.
2. **No global state.** No static, thread-local, singleton or process-wide cache. The
   player is the only owner and is passed by `&mut self`.
3. **No backend-trait change, no capability bit, no new dependency.** The player works
   through `Backend`, `SessionManager`, `Movie` and `InputFrame` only. It does not reach
   into backend internals and does not inspect the opaque payload beyond its length.
4. **No storage or file I/O in the library.** The player never opens a storage root,
   never writes a checkpoint, movie or cache file, and never reads a file. Bounded movie
   file reading stays in the CLI (`chronofork-cli`), which passes a decoded `Movie`.
5. **One crate-internal capture/restore pair, no public session API growth.** The cache
   needs to capture a canonical state without creating a checkpoint record. The
   implementation adds these crate-private accessors:

   ```text
   SessionManager::capture_session_state(session) -> Result<BackendSnapshot, SessionError>
   SessionManager::restore_session_state(session, &BackendSnapshot) -> Result<(), SessionError>
   SessionManager::discard_session_checkpoints(session)
   ```

   `capture_session_state` calls the backend's canonical `snapshot()` and returns the
   `BackendSnapshot`; it allocates **no** `SnapshotId`, inserts nothing into
   `Session::snapshots` and does not touch `next_snapshot_id`.
   `restore_session_state` calls the backend `restore` and maps a compatibility failure
   through the existing `map_backend_compatibility`, so **`validate_envelope` remains the
   one compatibility predicate**. `discard_session_checkpoints` drops the checkpoint
   record adopted by the import so the player session's map stays empty while the floor
   remains in `initial_state`; it allocates no `SnapshotId` and exposes no identity. All
   three are `pub(crate)`; no public `Session` or `SessionManager` method is added, renamed
   or removed (AS-05). A crate-internal unit test asserts that a full player run leaves the
   session's snapshot map empty and its snapshot-id allocator unchanged. The verified
   implementation uses the `player.rs` plus `player/cache.rs` split; both are below the
   AS-14 review trigger (sizes and counts stay in the evidence).
6. **One alignment rule.** Resolving an aligned tick to its recorded step index is one
   implementation. Add one read-only `pub(crate)` helper on `Movie` (for example
   `recorded_step_index(tick) -> Option<usize>`, binary search over the already-validated
   strictly increasing ticks) and have both `replay.rs` range alignment and the player's
   target validation use it; no second alignment predicate appears.
7. **AS-14 discipline.** The implementation keeps the player in `player.rs` and the cache
   in `player/cache.rs` (the permitted split by responsibility); both are below the
   AS-14 800-non-test-line review trigger. `movie.rs` grew only by the small read-only
   alignment helper, and the CLI work is in `crates/chronofork-cli/src/studio.rs` with
   `tas.rs` keeping subcommand dispatch and help wiring only. Measured line counts belong
   to evidence, not to this document (AS-13).

### 3. Cache model

**Entry.** One cache entry is:

```text
step_index: usize        zero-based recorded step whose outcome produced the state
tick: Tick               observed outcome tick of that step
state_hash: StateHash    observed state digest of that step
state: BackendSnapshot   full canonical backend state (typed, not serialized)
```

**Keying.** Entries are keyed by `step_index` in an ordered map
(`BTreeMap<usize, CachedState>`). Because recorded ticks are strictly increasing
(validated by the movie reader), the greatest entry at or before a target tick is the
greatest `step_index` at or before the target's recorded step index: one `range(..=k)`
lookup, no scan, no allocation.

**Floor.** The player always retains the **floor**: the canonical state imported from the
movie's embedded checkpoint at `open`, at `start_tick`, outside the cache. The floor is
never evicted and is the documented fallback when no cached entry qualifies. The floor
state is verified at open: the observed tick and state hash after import must equal
`movie.start_tick()` and the embedded checkpoint's state hash, otherwise `open` fails with
a typed error.

**Capture rule.** A state enters the cache only after the recorded step that produced it
was executed and **verified** against its recorded expectation, and only when its recorded
step index satisfies the cadence (step indices `N-1, 2N-1, …` for interval `N`). An
entry is therefore always a state this process verified; the cache never holds an
unverified state, and a divergence terminates the player before any later entry can be
captured. No entry is ever derived from a movie expectation, a movie anchor or a stored
checkpoint.

**Restore rule.** A rewind restores an entry with `restore_session_state` and then checks
the restored state: `inspect()` must report the stored tick and state hash. A mismatch
fails with a typed error rather than reporting a successful rewind. This check is
required because a seek to exactly a cached tick executes no replay step, so the restore
itself is the only evidence for that position.

**Bounds.** Both bounds are enforced on every capture:

| Bound | Default | Maximum | Meaning |
|---|---|---|---|
| entries | 256 | 65,536 | number of cached states (floor excluded) |
| bytes | 16 MiB (16,777,216) | 64 MiB (67,108,864) | sum of the deterministic per-entry metric below |

**Deterministic byte metric.** `entry_bytes = state.payload().len() +
STATE_CACHE_ENTRY_OVERHEAD`, with `STATE_CACHE_ENTRY_OVERHEAD = 256` bytes accounting for
the envelope fields, the `BackendSnapshot` structure, the hash, the tick and the map
node. `cache_bytes` is the sum over entries. This metric is the enforced and reported
value because it is reproducible: an allocator measurement or a timing figure would make
valid output non-deterministic. It is an accounting estimate, not a measurement of
allocator bytes; the true retained state is at most `capacity` full states whose payloads
are the dominant term, so the count bound plus the per-entry cap is the real resource
bound.

**Per-entry cap.** A capture whose entry metric alone exceeds the byte limit fails with a
typed `CacheEntryTooLarge { bytes, limit }` instead of being silently skipped, because a
silently disabled cache would make rewind cost depend on an invisible condition. A caller
can raise `--cache-bytes` up to `MAX_CACHE_BYTE_LIMIT`; above that, the state does not fit a
bounded cache and rewind remains available from the floor.

**Eviction.** When a capture would exceed either bound, evict the entry with the lowest
`step_index` repeatedly until both bounds hold, then insert the new entry. The floor is
outside the cache and is never a victim; the new entry has the highest step index and is
therefore never the victim. Evictions are counted (`cache_evictions`). Eviction is a pure
function of the captures performed, so cache contents are a deterministic function of the
movie, the options and the command sequence executed in this process; they never depend
on host time, allocation addresses or request timing.

**No hidden state.** The cache lives only in the `Player` value. Dropping the player drops
the cache, the private manager and the session; nothing is written anywhere and the next
process starts empty.

**Reporting.** Every report carries `cache_entries`, `cache_bytes` and `cache_evictions`;
the `state` block additionally carries `cache_interval`, `cache_capacity` and
`cache_byte_limit` (the configured bounds, not measurements).

### 4. Cadence, defaults and interactions

| Constant | Value | Meaning |
|---|---|---|
| `DEFAULT_CACHE_INTERVAL_STEPS` | 1 | capture after every recorded step; a larger `N` captures after steps `N-1, 2N-1, …` |
| `DEFAULT_CACHE_CAPACITY` | 256 | cached entries |
| `MAX_CACHE_CAPACITY` | 65,536 | hard entry bound |
| `DEFAULT_CACHE_BYTE_LIMIT` | 16 MiB (16,777,216) | default byte bound |
| `MAX_CACHE_BYTE_LIMIT` | 64 MiB (67,108,864) | hard byte bound (the per-checkpoint budget class of ADR-0006) |
| `STATE_CACHE_ENTRY_OVERHEAD` | 256 bytes | deterministic accounting overhead |
| interval | 0..=4,194,304 | `0` disables capture; a value above the v1 `MAX_STEPS` is a typed config error |

- **Interval 0 disables capture.** It is a valid configuration: every rewind then starts
  from the floor and replays; it is exact but costs the distance from the movie start.
- **Interval 1 is the default** because the increment's purpose is interactive stepping:
  a one-step `back` should cost one restore, not up to `N-1` replayed steps. The price is
  a short retained window (`capacity` steps), after which rewind falls back to the floor
  — which is the same trade-off a rewind buffer makes in the research (§5.3: dense recent
  states, always keep the origin state).
- **Partial tail.** A movie whose step count is not a multiple of `N` simply ends with no
  capture at the final step; the final state is reached by replay from the last captured
  state, exactly as any other non-cadence tick.
- **Validation.** `CacheOptions::new` (or the equivalent validating constructor) rejects
  `capacity == 0`, `capacity > MAX_CACHE_CAPACITY`, `byte_limit == 0`,
  `byte_limit > MAX_CACHE_BYTE_LIMIT` and `interval > MAX_STEPS` with typed config errors
  before the player imports a session.
- **v1 and v2 movies.** Both are accepted through the same `Movie` API. The embedded
  canonical checkpoint is the floor for both; step records are the expectations for both;
  `MovieStep::input()` supplies the schema-1 or schema-2 frame, so the player needs no
  version branch.
- **v2 anchors are not used.** A player never imports, reads or reports anchors and never
  seeds its cache from them. Anchors remain the movie-level one-shot optimization of
  ADR-0006 §6; a player's rewind is always based on states it verified itself. An
  anchored and an anchor-less v2 movie with the same embedded checkpoint and step records
  produce identical `tas studio` behavior.
- **No format, storage or identity change.** The cache is transient runtime memory. It
  writes no movie field, no anchor, no checkpoint, no storage object and no metadata; the
  movie bytes and their SHA-256 are unchanged by any studio run.

### 5. Rewind semantics

Positions are `start` (the floor, `applied_steps = 0`) and one position per recorded step
(`applied_steps = step_index + 1`, tick = that step's recorded outcome tick). The observed
tick at a position equals the recorded tick; the observed state hash equals the recorded
hash.

**Target validation.** A seek target must be `start_tick` or a recorded outcome tick
(resolved by the single alignment helper). Every other tick is rejected before any restore
or step with a typed category: before the movie start, after the movie end, or unaligned.
No target is rounded, clamped, interpolated or widened. `next` and `back` are sugar for the
adjacent position in the recorded order: `next` moves from `start` to step 0 and from step
`k` to step `k+1`; `back` moves from step 0 to `start` and from step `k` to step `k-1`.
`next` at the end position and `back` at the start position are typed errors
(`AlreadyAtEnd` / `AlreadyAtStart`) and change nothing.

**Selection.** Let `k` be the target's recorded step index (`None` for `start_tick`).
Choose the greatest cached entry with `step_index <= k`, or the floor when no entry
qualifies. A target equal to the current position executes no step. A target before the
earliest retained entry — including every target after eviction of that region and every
target with caching disabled — falls back to the floor.

**Execution.** Restore the chosen state, verify the restored tick and state hash against
the stored value, then apply the recorded inputs for the steps after the chosen state
through the target step, one input per step, each exactly once, comparing the observed
outcome tick and state hash with the recorded expectation. Capture cadence entries as
verified steps pass. Stop at the first mismatch and report it as the existing typed
`TickMismatch` (recorded tick differs) or `Divergence` (state hash differs) with the exact
recorded tick, zero-based input index, expected hash and observed hash; a backend or
session failure is a typed `ReplayFailed` carrying the input index and the source error.
There is no resynchronization, no skip, no partial pass and no later-mismatch substitution.

**Cost and claims.** Rewind costs O(distance from the chosen start state), at most
O(target step index) when the floor is used; cache capture adds one canonical backend
snapshot per cadence step. This is **not** O(1) and no random-access or performance target
is claimed: repeated long rewinds repeat the work, and with a small cache the cost of a
far rewind approaches a full replay. Timing is never an equality gate.

**Exactness invariant.** For the same target tick, a rewind must report the same observed
tick and state hash as a full `Movie::verify` and as `ReplayRange` replay from the
embedded checkpoint, and must report the same first divergence after its chosen start
state. Because every start state was verified in this process (or is the verified floor),
a rewind cannot start from an unverified prefix; the player never reproduces the anchored
replay's documented pre-anchor prefix caveat.

**No cross-invocation state.** Nothing is carried between processes. Two `tas studio`
invocations with the same input script produce the same blocks; a second invocation's
result equals a first-ever invocation. Only the explicit in-process player holds state,
and only for the lifetime of its process.

### 6. Interactive stepping CLI

New subcommand, deliberately small and line-oriented:

```text
chronofork tas studio --movie <FILE>
                      [--cache-interval <N>] [--cache-capacity <N>] [--cache-bytes <N>]
chronofork tas studio --help
```

- **Flags.** `--movie <FILE>` is required. `--cache-interval` is a decimal u64, default
  1, where `0` disables capture; `--cache-capacity` is a decimal `usize`, default 256,
  range 1..=65,536; `--cache-bytes` is a decimal `usize`, default 16,777,216, range
  1..=67,108,864. Duplicate, missing, empty, non-numeric, out-of-range or unknown flags
  (including `--from`, `--until`, `--steps`, `--input`, `--root`, `--session`,
  `--snapshot`, `--ports`, `--axes`, `--keys`, `--against`) are typed CLI errors: stderr
  `chronofork: <error>`, empty stdout, exit 1.
- **Backend.** The CLI passes the same deterministic synthetic `BackendFactory` that
  `tas record|play|verify|compare` use. `studio` has no backend-selection flag and no
  storage-root flag, so it cannot be pointed at another backend or at durable storage.
- **Start position.** The player opens at the floor position (`start_tick`, no step
  applied) and executes nothing until the first command. Starting at another tick is
  deferred; a caller can `goto <TICK>` first.
- **Commands** (one per stdin line, ASCII-whitespace separated tokens, case-sensitive):

  | Command | Meaning |
  |---|---|
  | `next` | seek to the next recorded position |
  | `back` | seek to the previous recorded position (`start` is a position) |
  | `goto <TICK>` | seek to `start_tick` or a recorded outcome tick |
  | `state` | report the current position and cache configuration/metrics; executes nothing |
  | `help` | print the command list as a block |
  | `quit` | print the quit block and exit 0 |

  Blank or whitespace-only lines are ignored and print nothing. Any other line is a typed
  error (unknown command, wrong arity, malformed tick, before-start, after-end, unaligned
  target, `next` at the end, `back` at the start): it is validated **before** any restore
  or step, printed to stderr as `chronofork: <error>`, and the process exits 1 with no
  stdout block for that line and no position change.
- **Output.** Exactly one `key=value` block per command, separated from the previous block
  by one blank line (the existing `begin_record` convention); field names, values and
  order are fixed:

  ```text
  record=seek
  tick=<observed tick at the position>            # equals target_tick on success
  applied_steps=<n>                               # recorded steps applied from the movie start
  target_tick=<t>
  start=floor|cache
  start_tick=<t>                                  # floor start tick, or the cached entry tick
  start_step=<k>                                  # only when start=cache
  replayed_steps=<n>                              # steps executed by this command
  state_hash=<hex>                                # observed hash at the position
  cache_entries=<n>
  cache_bytes=<n>
  cache_evictions=<n>
  ```

  ```text
  record=state
  tick=<t>
  applied_steps=<n>
  state_hash=<hex>
  movie_version=<1|2>
  input_schema=<1|2>
  start_tick=<movie start tick>
  end_tick=<movie end tick>
  steps=<movie recorded step count>
  cache_interval=<n>
  cache_capacity=<n>
  cache_byte_limit=<n>
  cache_entries=<n>
  cache_bytes=<n>
  cache_evictions=<n>
  ```

  ```text
  record=help
  command=next
  command=back
  command=goto <TICK>
  command=state
  command=help
  command=quit
  ```

  ```text
  record=quit
  reason=command|eof
  tick=<current observed tick>
  applied_steps=<n>
  ```

  On a state-hash divergence the command emits a `record=divergence` block and exits 1:
  `diverged=1`, `tick` (the recorded expected tick of the first mismatch), `step` (the
  zero-based input index), `expected`, `observed`, `start`, `start_tick`, `replayed_steps`,
  `cache_entries`, `cache_bytes`. A recorded-tick mismatch, a replay failure or any other
  typed failure is stderr-only with exit 1, matching ADR-0004/ADR-0005 behavior; it is not
  reported as a divergence block.
- **EOF.** End of stdin behaves exactly like `quit`: print the quit block with
  `reason=eof` and exit 0. A final line without a trailing newline is processed normally.
- **Exit codes.** 0 for `quit`/EOF and for any completed command run that ends that way; 1
  for every typed failure (flag, command, target, boundary, divergence, tick mismatch,
  replay failure, I/O). No other exit code, no panic, no partial block, no diagnostic on
  stdout.
- **Streams and non-mutation.** Valid stdout is `key=value` data only: no host time,
  timestamp, process id, host name, file path, movie SHA-256, entropy or telemetry.
  Diagnostics go to stderr as `chronofork: <error>`. The command opens the movie
  read-only, never writes the movie, a cache file, a storage root or any other file, and
  leaves the movie's SHA-256 and directory contents unchanged.
- **Help text.** `tas studio --help` and the `tas --help` summary must state: the stdin
  command set, the block fields, the cache interval/capacity/byte defaults and maxima,
  that rewind restores a verified cached state (or the embedded checkpoint) and replays
  forward (not O(1), no random access, cost proportional to the distance), that nothing is
  written or persisted, and the deferred list (rerecord/branch editing, real-hardware
  input TAS, persistent cache, O(1) seek, GUI/HTTP/MCP). It must also state that the
  acceptance backend is the deterministic synthetic fixture and that M3 is not complete.
  The existing deferred-list sentence in `tas --help` changes from "an in-process
  rewind/state cache and interactive stepping" to the narrower deferred list only when the
  capability is implemented and verified; that help-text change is a declared, re-baselined
  output change (see the acceptance plan M3-A33).

### 7. Bounds, typed errors and isolation

**Bounds.** Entries and the byte metric are bounded by §3; the session count is exactly
one; imported work is bounded by the movie's own validated step count and checkpoint
sizes; a command's replay work is bounded by the movie step count; the CLI reads the movie
with the existing size bound before decoding. No allocation is sized from a user-supplied
number before it is validated (AS-12): `--cache-capacity` and `--cache-bytes` are
validated against the fixed maxima before the player is opened, and the player never
allocates a buffer proportional to a declared value that has not been checked. Byte
totals, payload lengths and step-index arithmetic are checked: an overflow is a typed
error, never a wrap, truncation or panic. Eviction itself is a normal, counted operation
and not an error; only a single entry that cannot fit the byte limit, or a configuration
outside the fixed maxima, is rejected.

**Typed errors.** One `PlayerError` enum at the player boundary, preserving sources
(never a flattened `String`):

| Category | Examples | Contract |
|---|---|---|
| Configuration | `InvalidCacheConfig { field, reason }` | Rejected before a session is imported |
| Cache | `CacheEntryTooLarge { bytes, limit }` | A single captured state cannot fit the byte bound |
| Session | `Session(SessionError)` | Import, restore, step or closed-session failure preserved |
| Replay | `Movie(MovieError)` | The existing `TickMismatch`, `Divergence`, `ReplayFailed` and range categories, unre-wrapped |
| Target | `TargetBeforeStart { tick, start_tick }`, `TargetAfterEnd { tick, end_tick }`, `TargetUnaligned { tick }` | Rejected before restore/step |
| Boundary | `AlreadyAtStart { tick }`, `AlreadyAtEnd { tick }` | `back` at start / `next` at end; no state change |
| Consistency | `FloorMismatch { expected_tick, observed_tick, expected_hash, observed_hash }`, `RestoreMismatch { .. }` | The imported or restored state does not match its canonical identity |

`Display`/`Error` preserve the underlying `SessionError`/`MovieError` where one exists.
The CLI keeps one match and maps every typed failure to stderr + exit 1; the divergence
block is the only failure that emits stdout data (`record=divergence`), exactly as the
existing `play`/`verify` contract already does for divergence.

**Isolation and non-mutation.** The player owns its private manager and its single
session. It never accepts, addresses or closes a session from another manager; it never
mutates a movie, a stored checkpoint, a storage root or a file. A failed seek reports the
error and leaves the player at its previous position except when the failure occurs after
a restore or during replay: in that case the player is in a failed state and must not be
used again — the CLI exits 1, so the process, not a caller, is the unit of recovery. No
pre-existing session, movie or file is affected in either case.

**Determinism.** Identical movie bytes, options and command sequence produce identical
blocks, identical `cache_entries`/`cache_bytes`/`cache_evictions` and identical exit codes
in separate processes and in debug/release builds. No host time, allocator address,
environment variable, process id, path or entropy influences the cache or the reports.

### 8. Acceptance coverage (executed 2026-11-15)

Every row was executed by the independent t5 verification and passed; commands, observed
values and artifacts are recorded in the [M3 rewind acceptance
plan](../quality/m3-rewind-acceptance.md) §5/§9 and the ignored
`.research/m3-rewind/verification/` evidence.

| Row | Capability | Status |
|---|---|---|
| M3-A28 | Cache construction, keying, cadence, count/byte bounds, deterministic eviction, memory reporting and the no-snapshot-record invariant | executed - passed |
| M3-A29 | Rewind exactness versus fresh replay at every target class, including before the earliest cache, after eviction, caching disabled, zero-step movies and first-divergence attribution | executed - passed |
| M3-A30 | `tas studio` process contract: flags, help, one block per command, fields, EOF/quit, invalid input, divergence, streams, exit codes, read-only movie and no writes | executed - passed |
| M3-A31 | Isolation, one-session ownership, determinism and absence of host time/entropy/path in library and output | executed - passed |
| M3-A32 | Module ownership and bounds audit: one player/cache implementation, no new crate/dependency/backend change/public session API growth, one alignment rule/parser/predicate, AS-12/AS-14/AS-05 checks | executed - passed |
| M3-A33 | Non-regression and gates: v1/v2/M1/M2/M3 targets, full gate, both demos byte-identical, serialized-state probe, target matrix, declared help-text changes only | executed - passed |
| M3-A34 | Scope honesty: docs and help match executed evidence, deferred list agreed, no M3-complete or O(1)/hardware/persistent-cache claim | executed - passed |

The verification was bound to the uncommitted tree at HEAD
`b8b0f935e40e08854c2155f1b5208f6fc91424d8` plus the increment working tree; the verifier's
only tracked edit was its dated [quality gates](../quality/quality-gates.md) block. A later
edit to an implementation path invalidates the PASS until re-verified. The documented
limitations and deferred items in §9 are unchanged.

### 9. Deferred work and stated limitations

**Implemented and verified for this increment (deterministic synthetic backend):** the
session-layer `Player`, its bounded `StateCache`, deterministic rewind by restore plus
verified forward replay, the `chronofork tas studio` stdin surface, executable tests for
M3-A28–A34, and the promoted documentation. Evidence: the [M3 rewind acceptance
plan](../quality/m3-rewind-acceptance.md) and `.research/m3-rewind/verification/`.

**Deferred, not designed as implemented and not to be claimed:**

- rerecording and branch editing; in-place movie editing; rerecord counters; switching
  branches inside the player;
- real-hardware input TAS: the Mega Drive `step` ignores input, and changing that is an
  emulation-semantics change under its own decision;
- O(1)/random-access seek, bidirectional or reverse execution, and any performance,
  latency or throughput target;
- a persistent/on-disk cache, a cache format or version, cache sharing between processes,
  cache import/export, and cache persistence across invocations;
- tiered/exponential state spacing, delta or compression-based states (for example a
  Zelda-style tiered buffer), and reserved/pinned states beyond the floor;
- using v2 anchors as cache seeds or as a second start-state source;
- pause/resume scheduling, a host-time run loop, frame pacing, turbo/seek-to-frame
  playback and scripted per-step input playback (a future scheduling decision);
- a GUI, TUI, timeline, HTTP API, MCP adapter or daemon surface;
- multi-controller/multi-device studio commands; the player replays whatever
  `MovieStep::input()` carries and invents no device interaction;
- cross-backend or cross-platform runtime portability; the acceptance scope is the
  deterministic synthetic backend on Linux.

**Stated limitations.** Rewind is replay from the nearest verified state, never random
access: with caching disabled or after eviction it costs the distance from the movie
start. The cache is process-local and unbounded only by its configured bounds; it is not
a checkpoint and cannot be resumed. The block protocol is line-oriented and not a
scripting language; a malformed line ends the session. The player does not verify the
pre-history of a state it did not observe because it never starts from one. M3 remains
incomplete after this increment (rerecord/branch editing and real-hardware input TAS are
still open).

### 10. Owner decisions (approved 2026-11-15)

The project owner approved these decisions as proposed with the statement "Aprovar
ADR-0007 como proposto (Recomendado)" at 2026-11-15T19:17:48-03:00 (America/Fortaleza),
2026-11-15T22:17:48Z. They were implemented as approved and independently verified (t5
PASS, 2026-11-15). A change to any of them requires an amendment (see Change control).

1. **Placement and ownership.** A new `chronofork_sessions::player` module whose `Player`
   owns a private `SessionManager` with exactly one imported session, the decoded movie
   and a bounded cache; no global state, no backend-trait change, no storage or file
   writes, no new crate or dependency.
2. **Captured state representation.** Cached entries hold typed `BackendSnapshot` values
   captured through two new crate-private `SessionManager` accessors that allocate no
   `SnapshotId` and never insert into the session's snapshot map; the session's public
   checkpoint API is unchanged.
3. **Bounds and cadence.** Cadence every `N` recorded steps with the v2-anchor index
   convention (steps `N-1, 2N-1, …`), default `N = 1`; default capacity 256 and 16 MiB;
   maxima 65,536 entries and 64 MiB; deterministic `payload + 256 bytes` accounting metric;
   eviction of the lowest step index first; the embedded-checkpoint floor and the newest
   entry always retained; a single entry above the byte limit is a typed error.
4. **Rewind semantics.** Restore the greatest verified cached state at or before the target
   (or the floor), replay recorded inputs forward verifying every step, report the exact
   first divergence, and never interpolate, never seek in O(1), never use anchors as start
   states and never carry state across invocations.
5. **Interactive surface.** `chronofork tas studio --movie <FILE> [--cache-interval N]
   [--cache-capacity N] [--cache-bytes N]`, stdin commands
   `next|back|goto <TICK>|state|help|quit`, the fixed blocks and field order in §6, one
   block per command, fail-closed invalid input (exit 1), EOF equals `quit` (exit 0), and
   no file/storage write and no host-dependent value in valid stdout.
6. **One rule per concern.** One alignment helper shared with ranged replay, one snapshot
   parser, one compatibility predicate, one replay/verification comparison; a new
   `crates/chronofork-cli/src/studio.rs` adapter with `tas.rs` keeping dispatch.
7. **Declared output change.** Adding `studio` changes the `tas` dispatch error text and
   `tas --help`, which must be re-baselined as a declared behavior change; every other
   existing CLI output stays byte-identical.
8. **Process.** Implementation only after this ADR is accepted, under the
   [M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md); rows M3-A28–A34 must
   be executed and independently verified; no commit, push or merge is authorized by this
   ADR or by its acceptance.

## Rationale

- **The session layer is the right owner.** Needs are rewinding and stepping a live
  session; a new session-layer type keeps canonical state in `chronofork-sessions`,
  avoids a new dependency, keeps the CLI a thin adapter and lets the behavior be tested
  where session semantics are tested (AS-08).
- **A private manager makes isolation structural.** If the player cannot address a caller
  session, "does not mutate pre-existing sessions" is a type-level property rather than a
  discipline, and two players cannot interfere.
- **A separate capture path keeps checkpoints canonical.** `Session::snapshot` is the
  durable-checkpoint constructor; using it for a rewind buffer would mint identities and
  grow an unbounded map. The crate-private capture returns the same canonical backend
  state without any identity, which is exactly what a transient cache needs.
- **Typed full states beat serialized bytes here.** A cache restore should be the backend's
  restore, not a parse; serialization would add checksum work per capture and per restore
  for no benefit until a persistent cache exists, which is deferred.
- **Verified-start rewind is the honesty mechanism.** Every start state comes from a step
  this process verified (or from the verified floor), so a rewind's first divergence is
  attributable and the anchored-replay prefix caveat does not apply. That property is worth
  the cache's memory.
- **Deterministic accounting is a contract, not a benchmark.** Memory reporting must be
  reproducible to be testable; a payload-plus-overhead metric is stable across allocators,
  builds and hosts, while a timing or allocator figure would not be.
- **Dense-recent eviction matches the interaction.** Interactive stepping is playhead-local;
  a dense recent window with a verified origin floor is the simplest policy that serves it.
  Spread-out coverage is what movie anchors are for, and they stay separate.
- **A line protocol keeps the CLI honest.** stdin commands with one `key=value` block each
  are scriptable, diffable and testable without a tty; the existing stream and exit-code
  conventions already cover the failure modes.
- **Fail-closed input is the only deterministic option.** A malformed line that is
  "skipped" would silently change the position a later command starts from, which is the
  opposite of exact stepping.

## Consequences

Positive:

- A bounded, deterministic in-process rewind state cache with a documented ownership rule
  and no effect on the accepted formats.
- Interactive stepping becomes possible with exact first-divergence reporting and no
  cross-invocation state.
- The session checkpoint map, durable storage, snapshot identity and backend contracts are
  untouched.
- Memory use is bounded, reported and reproducible, and eviction is deterministic and
  testable.
- The CLI gains a scriptable stepping surface without a new dependency, GUI or transport.

Negative / costs:

- The player holds a full backend state per cached step and a private manager; at the
  default interval memory grows quickly until the bounds and eviction engage.
- A rewind beyond the retained window replays a long prefix; with caching disabled, every
  rewind is a full-prefix replay.
- Two more concepts (verified cache entries and the floor) must be documented and tested,
  and the block protocol is a new user-visible contract to maintain.
- Adding `studio` changes the `tas` help/dispatch text, so an existing output baseline is
  re-captured.
- `player.rs`/`studio.rs` add module surface that must stay within the AS-14 discipline.

Risks / limitations:

- The acceptance claim is the deterministic synthetic backend on Linux only; no
  real-hardware rewind or input TAS is claimed.
- No O(1) or random-access seek, no persistent cache, no cross-process reuse; a caller
  expecting instant seeks would be misled by anything but the documented cost.
- The byte metric is an estimate; only the count bound and per-entry payload are hard
  resource guarantees.
- A player that hits a divergence or a restore failure must be discarded; the CLI exits, so
  failure recovery is per process.
- This ADR is Accepted (owner-approved 2026-11-15) but asserts no implementation, no
  verification, no acceptance row and no M3 completion.

## Status

**Accepted (owner-approved 2026-11-15); implemented and independently verified (t5 PASS,
2026-11-15).** The project owner approved ADR-0007 as proposed with the statement "Aprovar
ADR-0007 como proposto (Recomendado)" at 2026-11-15T19:17:48-03:00 (America/Fortaleza),
2026-11-15T22:17:48Z. The approved decisions are the eight recorded in §10, unchanged from
the proposal; acceptance changed no contract detail, limit, command, typed boundary or
deferred item.

The ADR was drafted by the documentation architect on 2026-11-15 from the accepted
ADR-0004/0005/0006 baseline, the session/CLI implementation at HEAD
`b8b0f935e40e08854c2155f1b5208f6fc91424d8`, the M3 acceptance plans and the research in
[TAS and debugging](../research/tas-and-debugging.md) §5.3/§6.

The increment was implemented under the approved plan and the independent t5 verification
executed every M3-A28–A34 row and returned PASS on 2026-11-15 (§8 and Evidence). The
promotion is bound to the uncommitted tree at that HEAD plus the increment working tree;
no commit or push was made. The acceptance claim is limited to the deterministic synthetic
backend on Linux. The deferred items in §9 remain unimplemented, and M3 is **not
complete**: rerecord/branch editing and real-hardware input TAS are still open.

## Evidence

This ADR is implemented and independently verified for the deterministic synthetic
backend (t5 PASS, 2026-11-15). The bullets labelled *read* below are context from the
design, not evidence; the owner-approval, implemented-contract, executed-verification and
promotion bullets are the evidence for the capability.

- **Owner approval:** the project owner's statement "Aprovar ADR-0007 como proposto
  (Recomendado)" at 2026-11-15T19:17:48-03:00 (America/Fortaleza), 2026-11-15T22:17:48Z,
  approving the eight decisions in §10 as proposed. The approval changed no contract
  detail, limit, command, typed boundary or deferred item and authorizes no commit or push.
- **Implemented contract:** `crates/chronofork-sessions/src/player.rs`,
  `crates/chronofork-sessions/src/player/cache.rs`,
  `crates/chronofork-sessions/tests/player.rs`,
  `crates/chronofork-cli/src/studio.rs`,
  `crates/chronofork-cli/tests/studio.rs`, the crate-private `Session`/`SessionManager`
  capture/restore/discard accessors, `Movie::recorded_step_index`, and the small `lib.rs`,
  `movie.rs`, `replay.rs`, `tas.rs`, `main.rs` wiring. One player/cache implementation, one
  alignment rule, one codec/parser/predicate/import path and one replay loop remain; the
  cache is private.
- **Executed verification (t5 PASS, 2026-11-15):** independent verification of the
  uncommitted tree at HEAD `b8b0f935e40e08854c2155f1b5208f6fc91424d8` plus the increment
  working tree (pre-edit working-tree diff hash `f09d2a83…bc7a7`); every M3-A28–A34 row
  `executed - passed`, the full gate exited 0 with every target at or above its baseline
  (counts are in the acceptance plan §9 and `target-matrix.txt`), both demos were
  byte-identical, and the regression/scope audits were green. Evidence:
  `.research/m3-rewind/verification/` (`SUMMARY.txt`, `target-matrix.txt`, `rows/`,
  `audit/`, `targets/`, `cli/`, `studio/`, `demos/`, `probes/`, `gate/`, `tree/`,
  `MANIFEST.sha256`); pre-change baseline `.research/m3-rewind/baseline/`; dated gate block
  in [quality gates](../quality/quality-gates.md). The verifier's only tracked edit was its
  dated gate block; no commit, push or tag was made.
- **Promotion:** the t6 documentation promotion updates this ADR and the durable docs from
  the t5 artifacts; it changes no contract and records the residual CLI help understatement
  (the `tas --help` "not independently verified" sentence needs a separate code change) as
  an observation in the acceptance plan §9 (O4).
- **Tree under design and verification:** branch `agent/m3-complete`, HEAD
  `b8b0f935e40e08854c2155f1b5208f6fc91424d8` with the increment's changes uncommitted;
  captured baseline under the ignored `.research/m3-rewind/baseline/` path
  (2026-11-15T22:00:12Z). The design and approval changed documentation only; the
  implementation and verification are recorded above.
- **Accepted movie/TAS contracts and their executed rows:**
  [ADR-0004](0004-tas-movie-format.md) (v1 container, record/playback, first divergence),
  [ADR-0005](0005-partial-replay-and-branch-comparison.md) (ranged replay, seek,
  comparison), [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md) (v2
  container, anchors, anchored seek, schema-2 input), and the executed
  [M3](../quality/m3-tas-replay-acceptance.md), [increment-2](../quality/m3-partial-replay-acceptance.md)
  and [format-v2](../quality/m3-format-v2-acceptance.md) acceptance plans linked from the
  [quality gates](../quality/quality-gates.md) result blocks.
- **Movie implementation read:** `crates/chronofork-sessions/src/movie.rs`
  (`Movie::from_bytes`/`to_bytes`, `Movie::record`, `Movie::verify`, `Movie::from_parts`,
  `MovieAnchor`, `MovieStep` accessors, `MovieError`).
- **Replay implementation read:** `crates/chronofork-sessions/src/replay.rs`
  (`ReplayRange`, `ReplayOutcome`, the single `replay_validated` loop, `compare_movies`).
- **Session/manager implementation read:** `crates/chronofork-sessions/src/session.rs`
  (`Session::snapshot` inserting into `snapshots: BTreeMap<SnapshotId, Snapshot>`,
  `restore`, `snapshot_bytes`, `inspect`) and `manager.rs`
  (`create_session_from_snapshot_bytes`, `snapshot`, `step`, `inspect`,
  `map_backend_compatibility`).
- **CLI implementation read:** `crates/chronofork-cli/src/tas.rs` (subcommand dispatch,
  block writers, help and deferred-list text), `main.rs` (`begin_record` block separator,
  `CliError`, command dispatch). No in-file test modules exist in either file; process
  targets are under `crates/chronofork-cli/tests/`.
- **Research basis:** [TAS and debugging](../research/tas-and-debugging.md) §5.3 (bounded
  state managers, `GetStateClosestToFrame` at or before the requested frame, always hold
  the origin state, dense-recent/sparse-older idea), §6.2 (pause/seek belong to the
  session layer) and §6.3 ("TAStudio does not have a random-access seek"; seek cost is
  proportional to the distance from the nearest cached state), plus §10.2 ("Seeking is
  replay, not random access"; do not promise O(1) seek).
- **Requirements and milestone state after promotion:** `FR-TAS-001` remains **Partial**
  (rerecord/branch editing and real-hardware input TAS absent); `FR-EXEC-007` is
  **Partial** (rewind and interactive stepping done; frame pacing and a host-time run loop
  absent); `FR-EXEC-008` is **Implemented** for the deterministic synthetic backend;
  `FR-API-001` covers the existing surfaces including `tas studio`; `FR-MD-010` remains
  **Planned**. After the t5 PASS, the [milestone M3](../roadmap/milestones.md) item
  "In-process rewind buffer/state cache and interactive stepping" is checked while M3
  remains incomplete.

## Change control

- A change to the ownership rule, the captured-state representation, the bounds or
  defaults, the eviction order, the rewind semantics, the command set or the block fields
  requires an explicit amendment to this ADR and an updated
  [M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md); it must not be made by
  quietly renaming a test or relaxing an assertion.
- The owner approval recorded in the header (2026-11-15) moved the status to Accepted.
  Acceptance authorizes implementation under the acceptance plan only; it does not
  authorize a commit, push or merge, and it does not complete M3.
- Implementation and verification status are promoted only from executed evidence
  (per-row commands, exit codes, artifacts and an independent verifier PASS), never from
  this document, a plan, a code review or a demo run.
- Deferred items in §9 stay deferred; a later increment must design rerecord/branch editing
  and real-hardware input TAS on their own decisions and evidence.
- **2026-11-15 (owner approval, t8).** ADR-0007 moved from Proposed to Accepted on the
  owner's statement "Aprovar ADR-0007 como proposto (Recomendado)" at
  2026-11-15T19:17:48-03:00 (America/Fortaleza), 2026-11-15T22:17:48Z. The header, §1, the
  Status section, §10 and Evidence record the approval, and documentation status lines
  that named the ADR Proposed were updated to Accepted. No contract detail, limit, command,
  typed boundary, deferred item, capability claim or implementation/verification status
  changed; no commit or push was made.
- **2026-11-15 (execution promotion, t6).** The independent t5 verification PASS promoted
  the header, §1, §8, the Status section, Evidence and the milestone/requirement annotation
  to implemented and verified for the deterministic synthetic backend (M3-A28–A34
  `executed - passed`). The promotion records the permitted `player/cache.rs` split, the
  crate-private `discard_session_checkpoints` helper (no `SnapshotId`, no identity, map
  kept empty), the residual CLI help understatement (acceptance plan O4) and the additive
  top-level help lines (acceptance plan O2). No contract detail, limit, command, typed
  boundary or deferred item changed; the deferred scope in §9 and M3's incomplete status
  are unchanged. The verifier-owned [quality gates](../quality/quality-gates.md) block was
  not edited here.
