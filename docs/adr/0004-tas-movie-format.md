# ADR-0004: TAS Movie Format and Deterministic Replay

- **Status:** Accepted
- **Date:** 2026-11-15
- **Decision owner:** Project owner (approved the staged M3 team plan on 2026-11-15)
- **Approval evidence:** the owner approved the staged M3 team plan (baseline capture,
  this Fase A design, the movie codec, the `chronofork tas` CLI, independent
  verification, documentation promotion and final verification) on 2026-11-15; the
  recorded plan approval timestamp is 2026-11-15T01:03:50-03:00. The approved plan
  fixes the scope, the container/checkpoint/input/verification decisions, the deferred
  list and the M3 acceptance structure recorded here. See §14 for the approved
  decisions.
- **Implementation status:** implemented and independently verified for the
  deterministic synthetic backend (M3-T4 PASS, 2026-11-15); the deferred scope in §12
  is unchanged and M3 is not complete.
- **Supersedes:** none
- **Related:** [execution model](../architecture/execution-model.md),
  [session model](../architecture/session-model.md),
  [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md),
  [ADR-0003](0003-durable-session-storage.md), [ADR-0001](0001-rust-workspace.md),
  [requirements](../requirements/functional.md),
  [non-functional requirements](../requirements/non-functional.md),
  [milestones](../roadmap/milestones.md)

> **Implementation status.** The decision is accepted and was implemented and
> independently verified for the deterministic synthetic backend (M3-T4 PASS,
> 2026-11-15). `crates/chronofork-sessions/src/movie.rs` contains the single v1
> reader/writer and recorder/verifier; `crates/chronofork-cli/src/main.rs` exposes
> `chronofork tas record|play|verify`; the tests are the movie unit tests,
> `crates/chronofork-sessions/tests/movie.rs` and `crates/chronofork-cli/tests/tas.rs`.
> The executed acceptance record is the [M3 acceptance
> plan](../quality/m3-tas-replay-acceptance.md); raw evidence is under the ignored
> `.research/m3-tas/verification/` path and the dated gate block is in
> [quality gates](../quality/quality-gates.md). The deferred scope in §12 remains
> unimplemented: multi-controller/ports/analog/keyboard input, frame advance, rewind,
> seek, rerecord, branch editing, real-hardware input TAS, import/export adapters,
> checkpoint-reference movies, compression/encryption, resynchronization/partial
> replay and HTTP/MCP/GUI/daemon. M3 is **not complete**. Do not cite this ADR as
> evidence for any deferred item.

## Context

ChronoFork can already execute a deterministic synthetic backend, fork sessions,
capture, serialize, restore and durably persist canonical checkpoints, and compare
sessions by tick and state hash. The building blocks this decision must reuse:

- `chronofork-core` owns `InputFrame` (a backend-defined `u32` button mask),
  `Tick`, `StateHash`, `SessionId`, `SnapshotId`, `hash_bytes` and the shared
  bounds-checked `codec::{Reader, Writer}`.
- `chronofork-backend-api` owns `Backend::step(InputFrame) -> StepOutcome`, where
  `StepOutcome` carries the outcome tick, state hash and halted flag, plus the single
  backend compatibility predicate `validate_envelope` (backend ID, exact version,
  payload format version, configuration hash).
- `chronofork-sessions` owns the canonical outer snapshot envelope:
  `Snapshot::to_bytes` and `Snapshot::from_bytes` validate magic `CFSNAP01`, the outer
  format version, identity, backend identity, tick, configuration and state hashes,
  opaque payload length, payload, a trailing SHA-256 checksum and trailing bytes.
  `SessionManager::create_session_from_snapshot_bytes` is the existing import path:
  it decodes the snapshot, creates a fresh backend, restores the checkpoint (which
  calls `validate_envelope`) and registers a new root session.
- `chronofork-test-backend` provides the deterministic synthetic backend. Its
  `APPLY_INPUT` instruction consumes the low byte of the button mask, one `InputFrame`
  is consumed per step, and one step advances exactly one tick.
- `chronofork-cli` is a thin adapter with stable `key=value` output blocks for the M2
  session commands. It performs no serialization, compatibility or lock logic itself.
- `chronofork-megadrive` is a minimal real-hardware slice whose `step` currently
  ignores input (`fn step(&mut self, _input: InputFrame)` in
  `crates/chronofork-megadrive/src/backend.rs`), so it cannot support input TAS yet.

The tree has no movie format. Before this ADR,
`docs/architecture/execution-model.md` said only that a replay is "a checkpoint plus
an ordered input sequence keyed by tick" and that no TAS movie format existed. The
roadmap lists Stage 3 (TAS and deterministic replay) with a canonical movie format,
record/playback, frame advance, rewind, seek, rerecord, multi-controller input and
divergence reporting, and the milestone plan keeps every M3 implementation item
unchecked.

## Problem

How should ChronoFork represent a recorded TAS input sequence as one portable,
versioned, self-verifying artifact so that:

1. the same file replays to the same ticks and state hashes in a new process;
2. the replay reports the **exact first diverging step** (tick, step index, expected
   hash and observed hash), not just a generic failure;
3. corruption, truncation, an unsupported version and a hostile length/count are
   rejected with typed errors before any unbounded allocation or execution;
4. the initial state is unambiguous and is not a second snapshot format;
5. recording never advances or mutates the source session, and the movie is never
   treated as a state source of truth;
6. host time and other nondeterministic host inputs can never affect canonical bytes,
   equality or replay; and
7. the first increment stays small enough to implement and verify without
   pretending to support multi-controller, analog, frame advance, rerecord, seek or
   real-hardware input.

The decision must answer the container layout, metadata, initial checkpoint, input
schema, verification records, compatibility rules, record/playback semantics, module
ownership, resource bounds and deferred scope without inventing a second snapshot
serializer, reader or backend compatibility predicate.

## Alternatives

### Container and initial-state alternatives

A. **Single-file `.cfmovie` with an embedded canonical snapshot and per-step expected
   outcomes** (recommended).
B. **Movie references a durable checkpoint by storage key and checksum** instead of
   embedding it.
C. **Input-only movie with a final state hash only** (no per-step expectations).
D. **Archive or directory container** holding the checkpoint, inputs and metadata as
   separate files.
E. **Text/JSON movie format** for human readability.
F. **Reuse the `Snapshot` envelope** by placing the input sequence inside a
   backend-owned snapshot payload.

| Alternative | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| A. Single self-contained `.cfmovie` | One portable file; embedded checkpoint is authoritative; no storage root needed to replay or restart; per-step expectations enable exact first-divergence reporting; reuses `Snapshot::to_bytes`/`from_bytes` and the shared codec; one artifact to checksum, copy and archive | A movie with a large checkpoint is as large as that checkpoint; the file is binary and needs tooling; the embedded snapshot is copied into the movie | **Recommended** |
| B. Reference a durable checkpoint | Small movie files; the checkpoint has one canonical home; storage already provides integrity and identity | Replay needs the storage root, the backend and the exact commit; deleting or moving the checkpoint breaks the movie; the "restart from the embedded checkpoint" requirement is unmet; adds a second resolution path and new failure modes; the checkpoint may be absent where the movie is used | Deferred: a future explicit reference mode, not v1. It requires a separate decision and cannot replace the embedded form |
| C. Input-only + final hash | Smallest possible movie (32 bytes total for verification) | A mismatch only says "the final state differs"; it cannot report the first diverging tick or input index; localization requires external re-runs and instrumentation, so the increment's central acceptance criterion is unmet | Rejected for v1 |
| D. Archive/directory of files | Components are independently inspectable; existing storage layout ideas apply | No single artifact; needs a manifest, member names and a second container parser; portability and "one file" simplicity are lost; ordering and checksum coverage become a new protocol | Rejected for v1 |
| E. Text/JSON movie | Human-readable diffs and tooling | Canonical byte equality, deterministic float/number formatting and ordering need a second serializer and probably a new dependency (`serde`), contradicting AS-01/AS-11; input masks and 32-byte hashes are bulky in text; a hand-written JSON parser is worse | Rejected for v1 |
| F. Inputs inside the backend payload | No new top-level format | The payload is opaque and backend-owned; the session layer would have to parse backend bytes to find inputs, violating the opaque-payload boundary and AS-01; every backend would need a compatible payload version and input encoding; the snapshot envelope cannot describe per-step expectations without changing its schema | Rejected for v1 |

### Verification-record alternatives

| Alternative | Size cost | What it can report | Verdict |
|---|---|---|---|
| Per-step expected `(outcome tick, state hash)` records | 44 bytes per step (8 + 4 + 32); 1,000,000 steps = 44,000,000 bytes plus the checkpoint | The exact first mismatching step, tick, input index, expected hash and observed hash | **Chosen** |
| Final state hash only | 32 bytes total | Only that the final state differed; no step, tick or input index | Rejected for v1 |
| Periodic checkpoints every N steps | One snapshot per N steps; a synthetic snapshot is roughly 460 bytes, so N = 1,000 is about 0.46 bytes/step, but a real backend payload can be a large RAM/VDP image, making the cost dominate | The first mismatching **window**, not the exact step; identifying the tick still requires a second per-step replay or instrumentation; adds many state copies and makes the "which state is canonical" question ambiguous | Rejected for v1 |
| Final hash plus periodic checkpoints | Sum of both | Still only a window | Rejected for v1 |

Periodic checkpoints can be cheaper per step for a large N, but they do not meet the
contract: the increment must name the exact first diverging tick and input index. A
hybrid is also rejected because it inherits the imprecision and the extra state
copies. Per-step records are self-describing, independent of backend snapshot size and
straightforward to verify.

## Decision (accepted)

### 1. Scope and canonical home

This ADR is the canonical home for the v1 movie and replay contract. Architecture,
requirements, roadmap and acceptance documents link here instead of restating the
layout, compatibility matrix or limits.

Fase A changes no code. Fase B implements this contract in the existing workspace; it
must not add a second snapshot serializer, reader or compatibility predicate, and it
must not add a new crate or dependency. The public names in this ADR (`Movie`,
`MovieMeta`, `MovieError`, `ReplayReport`, constants) describe the contract shape;
Fase B may refine spelling or parameter grouping, but the canonical bytes, the typed
rejection boundaries and the semantics are the contract.

### 2. Container: one canonical `.cfmovie` file

A movie is exactly one file with the conventional extension `.cfmovie`. The
extension is a naming convention, not a trust boundary: the decoder validates the
magic and version, never the file name.

```text
magic "CFMOVIE1"                      8 bytes
u16 movie_format_version              2 bytes, little-endian, value 1
u16 input_schema_version              2 bytes, little-endian, value 1
u64 movie_id                          8 bytes, little-endian, non-zero
u16 name_length                       2 bytes, little-endian; 0 means no name
name                                  name_length UTF-8 bytes, at most 256
u64 parent_movie_id                   8 bytes, little-endian; 0 means no parent
u64 start_tick                        8 bytes, little-endian
u64 snapshot_length                   8 bytes, little-endian
embedded snapshot                     snapshot_length bytes, verbatim
                                      Snapshot::to_bytes() output
u64 step_count                        8 bytes, little-endian
step records                          step_count × 44 bytes, ordered:
                                        u64 outcome_tick  (8)
                                        u32 input         (4)
                                        state_hash        (32)
trailing SHA-256                      32 bytes over every preceding byte
```

All integer fields are little-endian, matching the snapshot envelope. State hashes
are the raw 32 bytes of `StateHash::as_bytes`, not hexadecimal text. The fixed
overhead is 86 bytes (header fields plus the trailing checksum) before the optional
name, embedded snapshot and step records.

Canonical encoding rules:

- There is exactly one writer (`Movie::to_bytes`) and one reader
  (`Movie::from_bytes`), both built on `chronofork_core::codec::{Reader, Writer}`.
- Field order, widths, endianness and the presence rules above are fixed. No field is
  optional in the encoding other than the bounded name, the parent id (sentinel 0)
  and a zero-step sequence.
- Bytes are deterministic: no maps, sets, iteration order, padding, alignment,
  compression, encryption or host-dependent content.
- The trailing SHA-256 is computed with `chronofork_core::hash_bytes` over **all**
  bytes preceding it, including the magic, metadata and the embedded snapshot's own
  checksum. The embedded snapshot's own checksum remains a second, independent
  integrity check.
- A movie is canonical: two movies are byte-equal exactly when their metadata,
  embedded checkpoint bytes and per-step records are equal. Appending, prepending or
  reordering bytes is a rejection, never a reinterpretation.

Limits (v1):

| Constant | Value | Applies to |
|---|---|---|
| `MAX_MOVIE_BYTES` | 256 MiB (268,435,456 bytes) | one complete movie file |
| `MAX_STEPS` | 4,194,304 (2^22) | recorded step count |
| `MAX_NAME_BYTES` | 256 UTF-8 bytes | the optional movie name; the v1 value is fixed by the movie format, and the implementation reuses the existing session-layer constant rather than defining a divergent second value |

The 256-byte name bound is part of the v1 format: if the session-layer name constant
ever changes, a v1 reader must still enforce 256 bytes until the movie format version
is bumped with updated rejection tests.

The step bound and the file bound are consistent: 4,194,304 steps × 44 bytes =
184,549,376 bytes (176 MiB), which leaves about 80 MiB of the file bound for the
embedded checkpoint and metadata. A 64 MiB checkpoint, the existing checkpoint bound,
therefore fits. Raising either limit requires an ADR amendment, updated rejection
tests and verifier evidence; an implementation must not silently raise a limit.

Bounded reading is mandatory and is specified in §7. A declared length or count is
validated against the limits and against the bytes that actually remain **before**
allocating a buffer or slicing the input.

### 3. Metadata and identity

The movie-level metadata is deliberately small and non-canonical where it is a label.
It never determines emulated state.

| Field | Kind | Rule |
|---|---|---|
| `movie_id` | caller-supplied logical identity | A non-zero `u64`. It is a label, not a global registry key: v1 does not guarantee global uniqueness and never resolves it by search. It never affects replay, ordering or state hashes. |
| `name` | non-canonical label | Optional bounded UTF-8, at most `MAX_NAME_BYTES` (256). It must not contain control characters. It is never a path component, never an identity authority and never affects replay. An absent name encodes as length 0. |
| `parent_movie_id` | branch/provenance metadata | Optional non-zero `u64`; 0 encodes absence. It must differ from `movie_id`. It is not a foreign key: v1 never resolves, validates the existence of, or repairs a parent, and a dangling parent is reported only as metadata. It never affects replay. |
| `start_tick` | mirror of the embedded checkpoint | Must equal the tick returned by `Snapshot::from_bytes` on the embedded bytes. A mismatch is a typed rejection, not a byte reinterpretation. |
| embedded checkpoint identity | owned by the embedded snapshot | Snapshot id, origin session id, backend id, backend version, backend payload format version, tick, configuration hash and state hash are read **only** from the embedded snapshot. The movie does not mirror backend identity or hashes at movie level, so there is no second compatibility predicate. |

Movie ids are logical labels supplied by the caller. The CLI defaults an omitted
`--movie-id` to `1`; callers that need distinct branch labels pass distinct ids. The
embedded snapshot's `SnapshotId` and origin session id stay diagnostic values and need
not match the movie id; nothing aligns them.

**No host time in canonical bytes.** v1 stores no creation or modification
timestamp, no process id, no host name, no random nonce and no file name inside the
movie. Host time may be used for logging, timeouts or a sibling temporary file name,
never inside the movie bytes. Therefore host time cannot affect bytes, ordering,
equality, state hashes or replay. A real-world timestamp must never be added without
a new format version and an explicit decision.

### 4. Initial checkpoint: the embedded canonical snapshot

v1 embeds the movie's initial state as the exact bytes returned by
`Snapshot::to_bytes()` for the source checkpoint. The writer takes those bytes and
frames them with `snapshot_length`; it does not parse, re-order, re-encode or
re-checksum any snapshot field.

- The embedded snapshot is authoritative for the initial state: backend id, backend
  version, backend payload format version, tick, configuration hash, state hash and
  the opaque payload all come from it.
- Decoding uses `Snapshot::from_bytes` only. The movie reader must not add a second
  snapshot parser, a second checksum rule or a second backend compatibility predicate.
  A malformed, truncated, checksum-bad or unsupported embedded snapshot is a typed
  movie error that preserves the session-layer error as its source.
- Backend compatibility is decided only when a replay imports the checkpoint through
  the existing `SessionManager::create_session_from_snapshot_bytes` path, which calls
  the existing `validate_envelope`. A structurally valid movie with a foreign-backend
  or wrong-config checkpoint therefore decodes and then fails replay with a typed
  incompatibility; that is a replay rejection, not a movie-format reinterpretation.
- The movie does not store a second copy of the backend identity, configuration hash
  or state hash at movie level. `start_tick` is the only mirrored value, and it is a
  binding check, not a compatibility check.
- A `Snapshot` captured by the existing checkpoint path is already the canonical
  envelope. No movie-specific snapshot variant, wrapper or replacement format exists.
- **Deferred:** referencing a durable checkpoint by storage key and checksum instead
  of embedding it. v1 deliberately embeds the bytes so a movie replays in a new
  process and on another machine without access to the original storage root. A future
  reference mode needs its own decision, explicit fallback rules and evidence; it is
  not part of this increment.

### 5. Input schema v1

v1 `input_schema_version = 1` means exactly one backend-defined input per execution
step, in recorded order:

- `input` is the raw `u32` `InputFrame::buttons()` mask. ChronoFork does not interpret
  the bits; the backend contract defines them. The synthetic backend consumes the low
  byte in `APPLY_INPUT`; a real backend must document its own mapping.
- The schema has one input per step. There are no multiple ports, no controller types,
  no analog axes, no keyboard events, no per-scanline input and no input scheduled by
  host arrival time.
- The step's `outcome_tick` records the tick returned by `Backend::step` after that
  input was applied. Emulated timing is `Tick`-based, never host time.
- Steps are ordered by the file's step index. Replay applies input `i` to step `i`
  exactly once; it never coalesces, repeats, drops or reorders inputs.
- Tick ordering and continuity are validated from `start_tick`: the sequence must be
  strictly increasing, and the first outcome tick must be greater than `start_tick`.
  The format deliberately does **not** require a fixed +1 quantum because the quantum
  is backend-defined and `chronofork-sessions` must not encode a concrete backend's
  step semantics. The synthetic backend produces exactly `start_tick + i + 1`; a
  movie whose ticks do not match its embedded backend's actual execution fails replay
  with the typed tick mismatch at the first differing step. A zero-step movie is
  valid and has `end_tick == start_tick`.
- **Future schema versions** (multi-controller, ports, analog, keyboard, event-based
  timing) are a different `input_schema_version` with their own record layout. A v1
  reader rejects any other schema version with a typed error; it never guesses,
  upgrades, downgrades or reinterprets the records.

### 6. Verification records, cost and first divergence

Each step record stores the expected `outcome_tick`, the applied `input` and the
expected `state_hash`. The expected values are produced by the recording execution and
are only expectations: replay recomputes state from the embedded checkpoint and the
inputs and compares.

Cost is exactly 44 bytes per step:

```text
u64 outcome tick        8
u32 input mask          4
32-byte state hash     32
                       --
                       44 bytes per step
```

Examples: 1,000 steps = 44,000 bytes; 100,000 steps = 4,400,000 bytes; 1,000,000
steps = 44,000,000 bytes (about 42 MiB); the v1 maximum 4,194,304 steps =
184,549,376 bytes (about 176 MiB). The embedded checkpoint and metadata are added to
those figures. The cost is independent of backend snapshot size, which is the property
that makes per-step verification practical across backends.

**First-divergence report.** Replay checks step records in order and stops at the
first mismatch. It reports:

- `tick` — the recorded expected outcome tick of the first mismatching step;
- `input_index` — the zero-based step index whose input was applied;
- `expected` — the recorded 32-byte state hash for that step;
- `observed` — the state hash the backend actually produced.

A differing outcome tick is reported as a separate typed mismatch with the input index
and the expected/observed ticks. A backend or session failure during replay is a typed
replay failure carrying the input index and the preserved source error. No mismatch is
converted into a skip, a warning, a resynchronization or a partial pass.

**Rejected alternatives.** Final-hash-only recording cannot report a step or tick and
would make the increment's acceptance criterion unverifiable. Periodic checkpoints
report only a window, add many state copies and make the canonical state ambiguous;
even when their per-step size is smaller, they do not provide the exact tick. Both
alternatives are documented in the decision matrix above and are not v1.

### 7. Decode order, bounds and typed rejection

The movie reader is a pure byte parser. It performs no filesystem I/O and executes no
emulation. The CLI reads the file with a bound before calling the reader, and the
reader re-checks the slice length so library callers get the same protection.

Decode order (fail closed; all arithmetic checked):

1. Reject a byte slice shorter than the 86-byte fixed overhead or longer than
   `MAX_MOVIE_BYTES`.
2. Split the final 32 bytes from the body and verify `SHA-256(body) == trailing
   checksum`. Reject on mismatch before parsing any field.
3. Read and validate the magic, `movie_format_version` and `input_schema_version`.
4. Read `name_length`; reject it above `MAX_NAME_BYTES` or beyond the remaining
   bytes before reading. Read the name bytes, validate UTF-8 and control characters,
   and treat length 0 as absent.
5. Read `movie_id` and `parent_movie_id`; reject a zero `movie_id` and a parent equal
   to the movie id.
6. Read `start_tick`.
7. Read `snapshot_length` as a `u64`, convert it with `usize::try_from`, then require
   `remaining >= snapshot_length + 8 + 32` with checked arithmetic (the step count and
   trailing checksum must still fit) **before** slicing or allocating. Decode the
   embedded bytes with `Snapshot::from_bytes` only, and require
   `start_tick == snapshot.tick()`.
8. Read `step_count`; reject it above `MAX_STEPS`; compute `step_count * 44` with
   checked multiplication; require `remaining >= 32 + product` before
   `Vec::with_capacity`. Only then allocate and decode the records.
9. Validate that outcome ticks are strictly increasing and that the first is greater
   than `start_tick`.
10. Call `Reader::finish()` and reject any trailing bytes inside the body.
11. Return the validated `Movie`. Execution and backend compatibility checks happen
    only at replay import.

A failed decode returns a typed error and no partially usable `Movie`; it never
mutates a backend, a session or a file. Arbitrary bytes must be rejected without a
panic; every decoder gets an arbitrary-byte/no-panic test (anti-slop AS-12), and a
declared huge count or length must be rejected before allocation. Because the checksum
is verified first, a rejection test for a later structural field (magic, versions,
metadata, ticks) must rebuild a valid trailing checksum so the intended validator is
exercised; an unmodified byte flip is the checksum case, not a version case.

Typed error categories (final spelling may be refined in Fase B; the categories and
their boundaries are the contract):

| Category | Examples | Contract |
|---|---|---|
| Container/bounds | `FileTooLarge`, `TooShort`, `Truncated`, `TrailingBytes` | Rejected before allocation or reinterpretation |
| Integrity | `ChecksumMismatch` | No field is trusted before the checksum passes |
| Format/schema | `BadMagic`, `UnsupportedFormatVersion { found, supported }`, `UnsupportedInputSchema { found, supported }` | Explicit version rejection; no migration |
| Metadata | `InvalidMovieId`, `InvalidParentMovieId`, `InvalidName` | No guessing, no path use, no parent resolution |
| Binding | `StartTickMismatch { declared, snapshot }` | The embedded snapshot is authoritative |
| Steps | `TooManySteps { limit, found }`, `TickSequence { input_index, previous, found }` | Declared counts checked before allocation; ordering is explicit |
| Embedded checkpoint | A wrapper that preserves the typed `SessionError` (`InvalidSnapshot`, `IncompatibleSnapshot`, backend payload error) | Never flattened to a string; never re-parsed by a second reader |
| Replay | `Divergence { tick, input_index, expected, observed }`, a typed tick mismatch, a typed replay failure carrying the input index and source `SessionError` | First mismatch only; no resync; no partial pass |

No error is stringly typed. `Display` and `Error` implementations preserve the
underlying session/backend error where one exists.

### 8. Compatibility matrix

Compatibility is explicit and reject-only. There is no silent migration, no
rewrite-on-open and no best-effort reinterpretation of a future version.

| Component | Accepted in movie v1 | Check owner | On mismatch |
|---|---|---|---|
| File magic | `CFMOVIE1` exactly | Movie reader | `BadMagic`; nothing parsed |
| Movie format version | Exact `MOVIE_FORMAT_VERSION` (1) | Movie reader | `UnsupportedFormatVersion` with found/supported |
| Input schema version | Exact `MOVIE_INPUT_SCHEMA_VERSION` (1) | Movie reader | `UnsupportedInputSchema` with found/supported |
| Trailing file checksum | SHA-256 over every preceding byte | Movie reader | `ChecksumMismatch` |
| Embedded checkpoint outer envelope | Exact `SNAPSHOT_FORMAT_VERSION`, envelope magic, checksum and field decoding | `Snapshot::from_bytes` | Typed movie error wrapping `SessionError::InvalidSnapshot` or `IncompatibleSnapshot` |
| Embedded backend envelope | Exact backend id, exact backend version, exact `BACKEND_SNAPSHOT_FORMAT_VERSION`, exact configuration hash | Existing `validate_envelope` through the existing import path | Typed movie error wrapping `SessionError::IncompatibleSnapshot`; no second predicate |
| Embedded backend payload | Backend-owned payload format version | Backend `restore` during replay import | Typed movie error wrapping the backend/session error; the movie never inspects the payload |
| Start tick | Exact equality with the embedded snapshot tick | Movie reader | `StartTickMismatch` |
| Step count and tick sequence | `step_count <= MAX_STEPS`; strictly increasing from `start_tick` | Movie reader | `TooManySteps` / `TickSequence` |
| Movie id, name, parent id | Logical metadata only; not compatibility fields | Movie reader validates shape only | `InvalidMovieId`, `InvalidName`, `InvalidParentMovieId` |
| Snapshot id and origin session id | Diagnostic values inside the embedded envelope | Not a compatibility field | Never aligned with the movie id; never used as a durable key |

A format change requires all of: a version bump, updated rejection tests, an entry in
this matrix, updated requirements if user-visible, and verifier evidence. No migration
exists in v1; a future migration must be an explicit operator-invoked tool under a
separate decision and must never run implicitly on open.

### 9. Record and playback semantics

**Recording** captures the source checkpoint and executes the input sequence in a
separate imported session:

1. The recorder captures the source session's current checkpoint through the existing
   checkpoint path and obtains its canonical `Snapshot::to_bytes()` bytes.
2. It reads the caller-supplied movie id, optional name and optional parent id, and
   the start tick from the captured snapshot.
3. It imports those bytes into a fresh session with
   `SessionManager::create_session_from_snapshot_bytes` and steps the recorded inputs
   in that imported session, collecting each `StepOutcome` as an expected record. If
   any step fails or halts before the input sequence ends, recording fails with a
   typed error and produces no movie.
4. It constructs the movie through one validating constructor and encodes it with
   `Movie::to_bytes`.

The source session's backend state, tick and state hash are never advanced or
mutated. Capturing a checkpoint through `SessionManager::snapshot` stores that
checkpoint record in the source session (the existing checkpoint semantics); that
record allocation is the only permitted source-side effect. The recorded execution
itself happens in a distinct imported session. The recorder may close the imported
session after use; it remains a distinct runtime session in the manager and is never
the source.

**Playback and verification** import the embedded checkpoint into a fresh session and
recompute state:

1. Decode the movie with `Movie::from_bytes`; no execution happens here.
2. Import the embedded snapshot bytes with
   `SessionManager::create_session_from_snapshot_bytes`, so backend identity, version,
   payload version and configuration compatibility are decided only by the existing
   `validate_envelope` path.
3. Apply each recorded input in order to that fresh session and compare the observed
   outcome tick and state hash with the recorded expectation.
4. Stop at the first mismatch and return the typed divergence; otherwise return a
   `ReplayReport` with `start_tick`, `end_tick`, `steps`, `final_tick` and
   `final_state_hash`.

The movie is **never a state source of truth**. Expected hashes are expectations, not
restorable state; replay always restores the embedded canonical checkpoint and
recomputes state. There is no "trust the movie's final hash" shortcut and no code path
that applies the movie's expectations as state. Verification does not mutate any
pre-existing session; it registers one fresh imported session in the caller's manager
(the existing import contract), leaves that session independent and never touches the
movie's source session.

`play` and `verify` in the CLI share the same replay/compare implementation. They
differ only in the output record they print; there is no second comparison path.

The library is backend-agnostic through the `Backend` trait, but this increment
accepts and claims results only for the deterministic synthetic backend. Recording a
backend whose `step` ignores input (the Mega Drive slice) stores inputs that have no
effect; it is not a hardware input TAS and must not be described as one.

### 10. Architecture, module ownership and dependencies

- The implementation lives in the existing `chronofork-sessions` crate in the module
  `chronofork_sessions::movie`. There is no new crate. The module owns exactly one
  reader and one writer.
- It uses `chronofork_core::codec::{Reader, Writer}` for all primitive fields,
  `chronofork_core::hash_bytes` for the trailing SHA-256, `InputFrame`, `Tick` and
  `StateHash` for values, `Snapshot::to_bytes`/`Snapshot::from_bytes` for the embedded
  checkpoint, and `SessionManager::create_session_from_snapshot_bytes` for replay
  import. It does not reach into backend internals and does not inspect the opaque
  payload.
- The module performs no filesystem I/O. `Movie::to_bytes` returns bytes and
  `Movie::from_bytes` consumes a bounded slice. The CLI owns reading and writing the
  file, with a bound applied before reading.
- Encoding has exactly one entry point: a validating constructor (for example
  `Movie::from_parts`) that checks metadata, the embedded snapshot length and the step
  records, then hands the value to `Movie::to_bytes`. The recorder and tests use that
  constructor; it is not a second encoder and must not bypass the same validation the
  reader applies. It is how a test or a future tool builds a deliberately divergent
  movie with a valid trailing checksum without exposing mutable movie fields.
- No new dependency is approved. The standard library plus the existing workspace
  crates (`chronofork-core`, `chronofork-backend-api`) are sufficient. No `serde`,
  JSON/binary framework, compression, encryption, archive or random-number crate is
  added. No `unsafe`; the workspace `unsafe_code = "forbid"` lint applies. No new
  `#[allow]` or `#[ignore]`.
- `chronofork-sessions` must not depend on a concrete backend. The movie module works
  through the `Backend` trait and `SessionManager`; tests use the synthetic backend as
  a dev-dependency the same way existing session tests do.
- The CLI is a thin adapter in `chronofork-cli`: it parses flags, calls the movie API,
  prints stable `key=value` records and maps typed errors to exit code 1. It does not
  parse or encode the movie format itself.
- No HTTP, MCP, GUI, daemon or network path is added. The movie format and replay are
  local, synchronous operations.

### 11. Minimal CLI contract shape

The owner-approved increment surface is deliberately small:

```text
chronofork tas record --movie <FILE> --steps <N> [--input <U32>]
                      [--movie-id <U64>] [--name <NAME>]
                      [--root <DIR> [--session <KEY> [--snapshot <KEY>]]]
chronofork tas play --movie <FILE>
chronofork tas verify --movie <FILE>
```

- **`record` without `--root`** creates a fresh deterministic synthetic session,
  records `--steps` steps applying the fixed `--input` mask (default 0, neutral;
  `--steps` default 0), and writes the canonical movie file.
- **`record` with `--root`** opens an existing M2 storage root, loads a committed
  checkpoint (the latest snapshot by default, or the selected `--session`/`--snapshot`),
  rehydrates it in memory, records the same way and does not write storage. This
  proves the M2 checkpoint to movie path. A missing or uninitialized root is a typed
  failure; `record` never initializes a root.
- **`--movie-id`** defaults to `1`. **`--name`** is the optional bounded label.
  Parent movie id is part of the format and library contract, but branch creation and
  editing are deferred; this minimal CLI does not expose a parent flag in v1.
- **`play`** and **`verify`** load only the movie file; no storage root is needed. They
  replay from the embedded checkpoint and share one implementation.
- Output is stdout-only `key=value` blocks in the existing CLI style. `record` prints
  `record=movie` with `movie_version`, `input_schema`, `movie_id`, `name` (empty when
  absent), `start_tick`, `end_tick`, `steps`, `final_state_hash` and `movie_sha256`
  (the SHA-256 of the exact file bytes, lowercase hexadecimal; a copy/identity aid, not
  a field inside the movie). `play` prints `record=playback`; `verify` prints
  `record=verification` with `verified=1`. On divergence the block also carries
  `diverged=1`, `tick`, `input_index`, `expected` and `observed`, and the process exits
  1. A recorded-tick mismatch is a separate typed replay failure reported on stderr
  with the step index and expected/observed ticks; the `diverged=1` block is
  specifically a state-hash divergence. Diagnostics go to stderr as
  `chronofork: <error>`; no diagnostic contaminates stdout.
- Exit code is 0 on success and 1 on any typed failure. No panic, no network access, no
  host timestamp in the movie or stdout data.
- A failed `record` must not replace an existing movie file: the CLI encodes the
  complete bytes first and writes through a sibling temporary path renamed on success.
  Existing commands and their output are unchanged.

The full behavioral contract and its process-level tests are in the
[M3 acceptance plan](../quality/m3-tas-replay-acceptance.md) §6.

### 12. Scope, deferred work and stated limitations

**In scope for this first M3 increment:**

- canonical v1 movie container, reader and writer;
- record/playback of one per-step input sequence on the deterministic synthetic
  backend;
- replay verification with exact first-divergence reporting;
- restart replay in a new process from the embedded checkpoint, including recording
  seeded by an M2 durable checkpoint;
- the minimal `chronofork tas record|play|verify` CLI;
- executable tests and honest documentation listing what remains.

**Deferred, not implemented and not to be claimed:**

- multi-controller, multiple ports, analog axes, keyboard events and device-specific
  peripherals;
- frame advance, rewind and seek via checkpoints;
- rerecording and branch editing (v1 defines only passive parent-movie metadata; the
  CLI exposes no branch editing);
- real-hardware input TAS: the Mega Drive slice's `step` ignores input, so its input
  records have no effect;
- import/export adapters for other TAS formats (BizHawk, Gens and others), which are
  documented as future work only;
- HTTP API, MCP adapter, GUI and the Stage 6 daemon;
- a movie that references a durable checkpoint instead of embedding it;
- compression, encryption, deduplication, authentication and automatic retention or
  garbage collection;
- replay resynchronization, partial replay, branch comparison tooling and performance
  or capacity targets;
- cross-backend canonicalization and any cross-platform or hardware determinism claim.

The Mega Drive slice remains a partial hardware slice and the synthetic backend is a
test fixture, not a consumer emulator. No movie capability is evidence of emulation
accuracy.

### 13. Verification executed in Fase B (2026-11-15)

These gates were executed by the independent M3-T4 verification, which returned
**PASS**. The authoritative rows, commands and artifacts are M3-A1–M3-A12 in the
[M3 acceptance plan](../quality/m3-tas-replay-acceptance.md) §5, with raw evidence
under `.research/m3-tas/verification/` and the dated gate block in
[quality gates](../quality/quality-gates.md):

- movie round-trip and canonical deterministic bytes, including two recordings
  separated in time;
- bounded decode and the full typed rejection matrix (magic, versions, checksum,
  truncation, trailing bytes, limits, metadata, tick/count inconsistency, corrupt or
  incompatible embedded checkpoint);
- record/replay reproducing every recorded tick and state hash;
- exact first divergence at the recorded tick and input index with expected and
  observed hashes, plus a negative control that later steps are not reported;
- restart replay in a new process from the embedded checkpoint, and a recording seeded
  by an M2 durable checkpoint;
- source non-mutation and isolation between movies, sessions and branches;
- the CLI record/play/verify process contract, exit codes and stream split;
- determinism and proof that host time is absent from canonical bytes;
- M2 regression, byte-identical demos and the full quality gate;
- scope honesty and an explicit deferred list;
- branch metadata round-trip and isolation;
- one codec, one reader/writer, one compatibility predicate, no new dependency, no
  `unsafe`, no new `#[allow]`/`#[ignore]`.

### 14. Owner decisions (approved 2026-11-15)

The project owner approved the staged M3 team plan on 2026-11-15. The decisions
approved for this increment are:

1. A single self-contained `.cfmovie` file with an embedded canonical snapshot and
   per-step expected `(outcome tick, input, state hash)` records.
2. Per-step verification records at 44 bytes per step, so replay can report the exact
   first diverging tick and input index; final-hash-only and periodic-checkpoint
   alternatives are rejected for v1.
3. Input schema v1 is one backend-defined `u32` button mask per step with the outcome
   tick recorded; multi-controller, ports, analog and keyboard input are deferred to a
   future schema version with explicit rejection.
4. The embedded snapshot is authoritative and is embedded as the exact
   `Snapshot::to_bytes()` bytes, decoded only through `Snapshot::from_bytes`; a
   checkpoint-reference movie is deferred.
5. Reject-only compatibility with no silent migration, and the v1 limits of 256 MiB
   per movie, 4,194,304 steps and a 256-byte name.
6. No host timestamps or other host-dependent values in canonical movie bytes.
7. A recorder that captures the source checkpoint and executes a separate imported
   session, never advancing or mutating the source, and a player that imports the
   embedded checkpoint into a fresh session and recomputes state; the movie is never a
   state source of truth.
8. The minimal `chronofork tas record|play|verify` CLI as the contract shape, and
   implementation only under the approved staged plan after this ADR's acceptance.
   No commit or push without explicit owner authorization.

## Rationale

- **One portable file matches the replay requirement.** A movie that embeds its
  checkpoint replays after a restart and on another machine without the original
  storage root. Referencing a checkpoint would make the movie a fragile pointer and
  would add a resolution path that cannot satisfy the restart acceptance criterion.
- **Reusing the canonical envelope keeps one source of truth.** Embedding
  `Snapshot::to_bytes()` verbatim means snapshot identity, checksum, version and
  backend compatibility remain owned by the existing code. The movie adds framing and
  metadata, not a second snapshot format or a second compatibility predicate.
- **Per-step records buy exactness at a documented cost.** Forty-four bytes per step
  is the price of naming the first diverging tick and input index. The cost is
  independent of backend snapshot size, so the same design works for the synthetic
  fixture and for larger real backends. Final-hash-only and periodic-checkpoint
  alternatives cannot meet the exactness requirement.
- **Bounds before allocation is non-negotiable.** A movie is user-supplied input. A
  declared count or length must never drive an allocation before it is checked against
  the remaining bytes and the fixed limits; otherwise a small hostile file can force
  a large allocation.
- **Metadata is not state.** Names, movie ids and parent ids help humans and branch
  bookkeeping, but they cannot influence determinism, ordering, equality or replay.
  Excluding host time from canonical bytes keeps recordings reproducible.
- **Separate record and replay sessions preserve isolation.** Recording in an imported
  session keeps the source at its checkpoint and prevents a recorder from becoming a
  hidden mutation channel. Replaying in a fresh imported session ensures the movie's
  expectations are checked against recomputed state, never trusted as state.
- **Std-only keeps the trust surface small.** The format needs fixed-width integers, a
  hash and an existing snapshot envelope; a serialization framework, archive library or
  database would add license, maintenance and failure surface without solving a
  demonstrated problem.

## Consequences

Positive:

- A recorded input sequence becomes one portable, versioned, self-verifying artifact.
- Exact first-divergence reporting makes determinism regressions attributable to a
  specific tick and input index.
- Corruption, incompatible versions, hostile counts and foreign checkpoints fail closed
  with typed errors.
- Recording and replay do not mutate the source session, so branching and comparison
  stay isolated.
- The format reuses the existing snapshot envelope and shared codec, so snapshot
  improvements and compatibility fixes apply automatically.
- Host time cannot contaminate recording, equality or replay.

Negative / costs:

- Per-step expectations add 44 bytes per step; a long movie is substantially larger
  than its input sequence alone (about 176 MiB at the v1 step cap).
- A movie duplicates its initial checkpoint; large checkpoints make large movies.
- In-memory verification holds the embedded checkpoint bytes and a decoded snapshot,
  bounded by the file limit but larger than the checkpoint alone.
- The format is binary, so inspection needs tooling or dedicated tests.
- The embedded checkpoint is a point-in-time copy: a movie does not track later
  changes to a durable session.

Risks / limitations:

- The acceptance claim is synthetic-backend only. Real-hardware input TAS is not
  implemented and the Mega Drive step ignores input.
- Multi-controller, ports, analog, keyboard, frame advance, rewind, seek and rerecord
  are not implemented; a schema or format version that pretends otherwise would be a
  false claim.
- A movie from another backend version or configuration is rejected at replay import;
  there is no cross-backend portability.
- The format has no compression or encryption; movie files can be large and contain
  emulated memory.
- The `MAX_MOVIE_BYTES` and `MAX_STEPS` limits are v1 policy; raising them requires a
  new decision and evidence.
- This ADR is Accepted: acceptance does not assert that any movie, recorder, player,
  CLI command or test exists.

## Status

Accepted. The project owner approved the staged M3 team plan and the decisions in §14
on 2026-11-15. The v1 contract was implemented under the approved plan and
independently verified for the deterministic synthetic backend on 2026-11-15 (M3-T4
PASS); the deferred scope in §12 remains unimplemented and M3 is **not complete**.
Fase A changed no Rust code, test, manifest or lockfile. Fase B added only the
implementation and test paths listed under Evidence and promoted this documentation
(plus the verifier's dated gate-result block); no commit or push was made.

## Evidence

- Canonical snapshot envelope and its single reader/writer:
  `crates/chronofork-sessions/src/snapshot.rs` (`Snapshot::to_bytes`,
  `Snapshot::from_bytes`, `SNAPSHOT_FORMAT_VERSION`).
- Existing import path that restores a checkpoint into a fresh backend:
  `crates/chronofork-sessions/src/manager.rs`
  (`create_session_from_snapshot_bytes`, `create_session_from_stored_snapshot`).
- Single backend compatibility predicate:
  `crates/chronofork-backend-api/src/snapshot.rs` (`validate_envelope`,
  `BACKEND_SNAPSHOT_FORMAT_VERSION`).
- Shared bounded codec: `crates/chronofork-core/src/codec.rs`; hashes:
  `crates/chronofork-core/src/hash.rs`; input mask:
  `crates/chronofork-core/src/input.rs`; ticks and identifiers:
  `crates/chronofork-core/src/{tick,id}.rs`.
- Deterministic fixture backend, per-step outcomes and one input per step:
  `crates/chronofork-test-backend/src/lib.rs` (`SyntheticBackend::step`,
  `APPLY_INPUT`, `StepOutcome`); the trait is in
  `crates/chronofork-backend-api/src/traits.rs`.
- Mega Drive input is currently ignored:
  `crates/chronofork-megadrive/src/backend.rs` (`fn step(&mut self, _input:
  InputFrame)`).
- Durable checkpoint loading for the M2-seeded record path:
  `crates/chronofork-sessions/src/storage/` (`SnapshotRepository::load_snapshot`,
  `LoadedSnapshot`) and the M2 CLI in `crates/chronofork-cli/src/main.rs`.
- Existing CLI output conventions and thin-adapter boundary:
  `crates/chronofork-cli/src/main.rs` (`run_session`, `parse_session_args`,
  `write_snapshot_report`, `CliError`).
- **Implemented contract:** `crates/chronofork-sessions/src/movie.rs` (one
  `Movie::from_bytes`/`Movie::to_bytes`, the validating `Movie::from_parts`,
  `Movie::record`, `Movie::verify`, `MovieMeta`, `MovieStep`, `ReplayReport` and the
  typed `MovieError`); crate exports in `crates/chronofork-sessions/src/lib.rs`;
  `crates/chronofork-sessions/tests/movie.rs`; `chronofork tas` in
  `crates/chronofork-cli/src/main.rs`; `crates/chronofork-cli/tests/tas.rs`. The movie
  module uses only the shared codec, `Snapshot::to_bytes`/`from_bytes` and the existing
  import path; it introduces no second snapshot format or compatibility predicate.
- **Executed verification:** M3-T4 executed M3-A1–A12 on the uncommitted tree
  (`bcd6c6fae7edd8014aec27e89689933e182c246a` plus the M3 paths) with
  `CARGO_HOME=/tmp/t7-cargo` and returned PASS. Evidence:
  `.research/m3-tas/verification/` (`SUMMARY.txt`, `target-matrix.txt`,
  `A1-*.txt`–`A12-*.txt`, `logs/`, `targets/`, `cli/`, `demos/`, `probes/`,
  `MANIFEST.sha256`); pre-change baseline `.research/m3-tas/baseline/`; dated gate
  block in [quality gates](../quality/quality-gates.md). The acceptance record is the
  [M3 acceptance plan](../quality/m3-tas-replay-acceptance.md).
- **Tree state:** the implementation and its tests are uncommitted at
  `bcd6c6fae7edd8014aec27e89689933e182c246a` (2026-11-15); no commit or push was
  made. This ADR and its companion docs are not evidence of a commit.
- **Deferred scope:** see §12. The movie format has no multi-controller/ports/analog/
  keyboard input, frame advance, rewind, seek, rerecord, branch editing, real-hardware
  input TAS, import/export adapters, checkpoint-reference mode, compression/encryption,
  resynchronization/partial replay or HTTP/MCP/GUI/daemon. M3 is not complete.
- Design constraints cited: AS-01, AS-02, AS-03, AS-11, AS-12, AS-13 and AS-15 in the
  [anti-slop policy](../quality/anti-slop-policy.md); architectural rules 1–8 in
  [AGENTS.md](../../AGENTS.md).
