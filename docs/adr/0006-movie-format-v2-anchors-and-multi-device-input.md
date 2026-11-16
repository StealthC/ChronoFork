# ADR-0006: Movie Format v2 — Checkpoint Anchors and Multi-Device Input

- **Status:** Accepted; implemented and independently verified for the deterministic
  synthetic backend (t7 PASS, 2026-11-15)
- **Date:** 2026-11-15
- **Decision owner:** Project owner
- **Approval evidence:** The project owner explicitly approved ADR-0006 as proposed with
  the statement "Aprovar ADR-0006 como proposto (Recomendado)" at
  2026-11-15T17:06:17-03:00 (America/Fortaleza), 2026-11-15T20:06:17Z. The approval covers
  the proposed contract as recorded in this ADR, including the v2 limits (2,000,000 steps;
  65,536 anchors; 64 MiB embedded/anchor snapshot; 48 MiB cumulative anchor bytes; 256 MiB
  file), anchored ranged replay that may skip the pre-anchor prefix with
  `prefix_verified=0` while full-range verify remains complete, and the CLI flags/v2
  selection rule (`--ports`, `--axes`, `--keys`, `--anchor-interval`; plain `record`
  remains v1). At approval time this recorded the decision only; it did not assert
  implementation or verification, and no commit or push is authorized by it. The later
  implementation/verification is recorded below and in Evidence.
- **Supersedes:** None. This ADR extends [ADR-0004](0004-tas-movie-format.md) and
  [ADR-0005](0005-partial-replay-and-branch-comparison.md) without changing the accepted
  v1 container, the verified increment-2 behavior for v1 and anchor-less movies, or any
  M1/M2/M3 evidence.
- **Implementation status:** Implemented and independently verified for the deterministic
  synthetic backend (t7 PASS, 2026-11-15). The v1 `.cfmovie` format, record/playback,
  ranged replay/seek, branch comparison and CLI surfaces remain exactly as independently
  verified by M3-T4 and M3-T5 (2026-11-15). The v2 container, anchor table, anchored seek,
  schema-2 multi-device input and CLI surface are implemented and verified; the deferred
  scope in §13 remains unimplemented and M3 is **not complete**.
- **Verification evidence:** the independent t7 verifier executed M3-A18–A27 and returned
  PASS (2026-11-15); per-row commands, exit codes and artifacts are in
  `.research/m3-v2/verification/` and the dated result block is in
  [quality gates](../quality/quality-gates.md). Scope is the deterministic synthetic
  backend on Linux; no commit or push was made.
- **Related:** [ADR-0004: TAS movie format and deterministic
  replay](0004-tas-movie-format.md), [ADR-0005: partial replay, seek and branch
  comparison](0005-partial-replay-and-branch-comparison.md), [ADR-0003: durable session
  storage](0003-durable-session-storage.md), [execution
  model](../architecture/execution-model.md), [session
  model](../architecture/session-model.md), [M3 first-increment acceptance
  plan](../quality/m3-tas-replay-acceptance.md), [M3 increment-2 acceptance
  plan](../quality/m3-partial-replay-acceptance.md), [M3 format-v2 acceptance
  plan](../quality/m3-format-v2-acceptance.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md),
  [milestones](../roadmap/milestones.md), [anti-slop
  policy](../quality/anti-slop-policy.md), [quality gates](../quality/quality-gates.md).

> **Implementation and verification status.** The decision is **Accepted** (owner
> approval 2026-11-15T17:06:17-03:00) and the increment is **implemented and
> independently verified** for the deterministic synthetic backend (t7 PASS,
> 2026-11-15). `chronofork_sessions::movie` now reads and writes v1 and v2 files through
> one codec, v2 anchors are canonical checkpoint envelopes bound to recorded steps,
> anchored seek reuses the single ranged replay path, schema-2 multi-device frames are
> recorded and replayed, and the CLI exposes the v2 surface described in §10. The
> [M3 format-v2 acceptance plan](../quality/m3-format-v2-acceptance.md) rows are
> `executed - passed`; raw evidence is under the ignored
> `.research/m3-v2/verification/` path and the dated gate-result block is in
> [quality gates](../quality/quality-gates.md). The v1 contract stays authoritative
> through [ADR-0004](0004-tas-movie-format.md); ranged replay and branch comparison stay
> authoritative through
> [ADR-0005](0005-partial-replay-and-branch-comparison.md). The deferred scope in §13
> remains unimplemented: in-process rewind/state cache and interactive stepping, rerecord
> and branch editing, real-hardware input TAS, import/export adapters and
> HTTP/MCP/GUI/daemon. M3 is **not complete**; no hardware input or cross-backend
> portability is claimed.

## Context

The accepted and independently verified baseline this design must extend is:

- **v1 movie container and replay (ADR-0004, implemented and verified).**
  `chronofork_sessions::movie` owns one `Movie::from_bytes`/`Movie::to_bytes`, the
  validating constructor `Movie::from_parts`, `Movie::record` and `Movie::verify`. A v1
  movie is one file with magic `CFMOVIE1`, format version 1, input schema version 1, a
  bounded name and parent id, `start_tick`, the verbatim canonical
  `Snapshot::to_bytes()` checkpoint, `step_count` and 44-byte step records
  `(outcome_tick, u32 input, state_hash)`, followed by a trailing SHA-256 over every
  preceding byte. Per-step expectations make the first divergence exact. The v1 limits
  are 256 MiB per file, 4,194,304 steps and a 256-byte name. Replay imports the embedded
  checkpoint through `SessionManager::create_session_from_snapshot_bytes`, so backend
  compatibility is decided only by the existing `validate_envelope` predicate.
- **Ranged replay, seek and comparison (ADR-0005, implemented and verified).**
  `chronofork_sessions::replay` owns `ReplayRange` (the `(from, until]` window), its typed
  validation and the single replay loop; `Movie::verify` delegates to the full range.
  Every replay currently starts at the embedded checkpoint and re-simulates linearly,
  verifying the prefix as well as the window. `compare_movies` diffs recorded
  `(outcome_tick, state_hash)` records without re-simulating; metadata and input masks
  never decide equality.
- **Input model (v1).** `chronofork_core::InputFrame` is a backend-defined `u32` button
  mask with `InputFrame::new`, `buttons()` and `NEUTRAL`. The synthetic backend consumes
  only the low byte of that mask in `APPLY_INPUT` and advances exactly one tick per step.
  The Mega Drive slice ignores input (`fn step(&mut self, _input: InputFrame)`), so it is
  not an input-TAS backend.
- **Checkpoint bounds.** The durable storage default bound is 64 MiB per checkpoint
  (`StorageLimits::max_checkpoint_bytes`), and ADR-0004 §2 sized the v1 movie limit so a
  64 MiB checkpoint fits. A v2 anchor is another checkpoint-shaped object, so this ADR
  reuses the same 64 MiB per-snapshot budget.
- **Deferred items this ADR addresses.** ADR-0004 §12 deferred multi-controller, ports,
  analog axes and keyboard input and a future input schema version. ADR-0004 §12 and
  ADR-0005 §8 deferred periodic anchors/checkpoints and seek via checkpoints (a future
  format v2). The current milestone keeps both items unchecked.
- **Research context.** [TAS and debugging](../research/tas-and-debugging.md) records
  that emulator TAS tooling seeks by loading the nearest cached state and replaying
  forward; seek cost is proportional to the distance from that state, and O(1)
  random-access seek must not be promised. That is upstream context, not a ChronoFork
  decision. This ADR adopts anchors only as an explicit, bounded optimization over the
  same deterministic re-simulation.

Read on 2026-11-15 in the `agent/m3-complete` checkout (HEAD `9643d74`, no v2 code), the
implementation confirms the extension points above: one movie reader/writer, one
embedded-checkpoint parser, one replay loop, one compatibility predicate, one
`InputFrame`, and a synthetic `APPLY_INPUT` that reads `input.buttons() & 0xff`. This ADR
designs on top of that tree; it changes no code.

## Problem

How should ChronoFork add checkpoint anchors and multi-device input to the movie artifact
so that:

1. **v1 is untouched.** Existing v1 files decode, verify and re-encode byte-identically;
   a movie recorded with no v2 feature is byte-identical to the pre-change v1 output;
   every M3-A1–A17 result and the M2/M1 evidence stay valid.
2. **v2 is one portable file.** A v2 movie embeds its initial checkpoint, its ordered
   anchors, its ordered multi-device step records and a trailing checksum, and replays
   after a restart without a storage root.
3. **Anchors are canonical, not a second state format.** Each anchor is a verbatim
   canonical `Snapshot::to_bytes()` checkpoint, parsed only by `Snapshot::from_bytes`
   and bound to a recorded step. There is no second snapshot serializer, reader or
   compatibility predicate.
4. **Bounds precede allocation and execution.** Every declared count and length is
   checked against a fixed v2 limit and the bytes that actually remain before allocation,
   with checked arithmetic; arbitrary bytes never panic; incompatible versions are
   rejected with no migration.
5. **Anchored seek is exact where it executes.** The nearest anchor at or before the
   requested prefix end is imported through the existing checkpoint import path, and only
   the steps after it up to `until` are executed and verified. For the same `until`, the
   final tick and state hash and the first divergence after the anchor are identical to a
   full replay; the prefix semantics are stated honestly.
6. **Multi-device input is real, bounded and backward compatible.** Schema 2 covers at
   least four digital ports, analog axes and a keyboard bitmap; `InputFrame` keeps the v1
   constructor/accessor and neutral behavior; the synthetic backend consumes every new
   field deterministically without changing the v1 state layout, payload version or the
   hashes of existing v1 inputs; schema-1 movies replay identically.
7. **The artifact stays deterministic.** Anchor placement is a caller/recorder policy,
   not host time; no timestamp, host name, path, nonce or entropy enters canonical bytes;
   identical inputs produce identical v2 bytes.
8. **One codec and one replay loop remain.** v2 is a version branch of the single movie
   module, not a second format module, and anchor selection extends the single ranged
   replay path rather than creating a second replay implementation.
9. **No claim is wider than the evidence.** The increment is synthetic-backend only, does
   not implement rewind caching, rerecord/branch editing or real-hardware input TAS, and
   does not complete M3.

## Alternatives

### Version and container strategy

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. New magic `CFMOVIE2` and version 2 in the existing `movie` module, with a single public reader/writer dispatching on the magic and validating each family's version** | v1 bytes and v1 code path are untouched; rejection is explicit; one module keeps one codec; v1 files remain readable forever | Two encode/decode branches in one module; the module grows and may need an ADR-approved submodule split (AS-14) | **Accepted** |
| B. Keep magic `CFMOVIE1` and use only the version field | No new magic | The magic currently embeds the version digit, so accepting `CFMOVIE1` with format 2 weakens the existing exact-magic rejection and makes corrupt files harder to classify; v1 readers would see a valid magic and a confusing version | Rejected |
| C. Extend v1 in place by appending anchors after the step records | No version bump | Invalidates every verified v1 byte/rejection result; old readers would reject a "v1" file or misparse it; no migration path | Rejected |
| D. Sidecar anchor/checkpoint file | Smaller movie files | Two artifacts, a second resolution path and a weaker portability/restart guarantee; already rejected in ADR-0005 | Rejected |
| E. Interleave anchors with step records | Locality | Variable stride destroys fixed-record indexing, complicates bounds and seek selection, and gives no replay benefit; an ordered table is simpler to validate | Rejected |
| F. Anchor table after step records | Steps can be read first | Still needs both sections before binding validation; placing anchors before steps matches the declared layout, keeps one forward decode order, and makes anchor bounds reusable if a future version streams steps | Rejected for v2 |

### Anchor payload

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Verbatim canonical `Snapshot::to_bytes()` bytes** | Reuses the one snapshot envelope, checksum, version and restore path; anchor import is the existing import path; no second parser | Anchors inherit snapshot size (backend-dependent); a real backend's RAM/VDP image can make movies large | **Accepted** |
| B. Store only backend payload plus a movie-level backend identity/tick/hash | Slightly smaller than an envelope | Recreates the envelope fields inside the movie and forces a second compatibility predicate and snapshot parser, violating AS-01/AS-02 | Rejected |
| C. Store only tick and state hash, no restorable state | Tiny | Cannot start execution at the anchor; the anchor is useless for seek | Rejected |
| D. Reference a durable checkpoint by storage key | Small movie | Replay needs the root, the exact commit and a compatible backend; breaks the self-contained/restart requirement; already rejected in ADR-0004 | Rejected |

### Anchor-based seek

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Nearest anchor at or before the requested prefix end; import it; execute only later steps up to `until`** | Cuts work by the distance from the anchor; preserves window coverage and first divergence inside the window; no anchors falls back to the current path | Anchored ranged replay does not re-execute the pre-anchor prefix; the anchor must be validated and bound to a recorded step | **Accepted** |
| B. Keep linear replay and use anchors only as integrity cross-checks | No semantic delta | No seek benefit; contradicts the increment's purpose | Rejected |
| C. Select the nearest anchor to `until`, even if it is after `from` | Could skip more work | Skips window steps, so the reported window is incomplete and a divergence inside the skipped window is missed; violates ADR-0005 window semantics | Rejected |
| D. Binary search the sorted anchor table by step index | Exact, O(log a) selection, no allocation | None beyond the sort invariant the format already enforces | **Chosen within A** |
| E. Linear scan of anchors | Simpler | O(a) per replay for no reason; a is bounded but binary search is straightforward | Rejected |

### Input schema and backend consumption

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Fixed `InputFrame` fields: four 32-bit ports, four signed 16-bit axes and a 64-bit keyboard bitmap; one 72-byte schema-2 step record; extend the existing synthetic `APPLY_INPUT` to consume all fields, with a neutral field having no effect** | Keeps `new`/`buttons`/`NEUTRAL`; v1 inputs have neutral extras, so their state hashes and payload are unchanged; the default synthetic program exercises every device class; one input per step keeps timing deterministic | A backend must define port/axis/key semantics; unsupported device classes are inert, and device-enabled movies are synthetic-backend only | **Accepted** |
| B. Add new synthetic opcodes for the new device classes and leave `APPLY_INPUT` alone | Zero risk to existing instruction semantics | The default program never consumes the new fields, so CLI v2 recordings would be behaviorally vacuous; changing the default program would change its config hash and v1 fixture hashes; rejected | Rejected |
| C. Backend-agnostic device event queue timed by emulated ticks | More realistic input scheduling | New event/timing model, new state and determinism questions, far beyond the increment; ADR-0005 deferred it | Rejected |
| D. Replace `InputFrame` with an enum of per-device events | Expressive | Unbounded/variable encoding, per-device timing, no backend to consume it, breaks the v1 step signature | Rejected |
| E. Opaque fixed device blob | Simple bytes | No typed accessors, no per-field semantics or tests, easy to misuse; the backend contract would be undocumented bytes | Rejected |

## Decision (accepted)

### 1. Scope and canonical home

This ADR is the canonical home for the **v2 movie container, anchor semantics,
anchor-based seek, schema-2 multi-device input, v2 recording and the v2 CLI surface**. It
links to ADR-0004 for the v1 container and to ADR-0005 for ranged replay and comparison
instead of restating them. Where v2 behavior must differ from ADR-0005, the delta is
stated in §5 and applies only to v2 movies.

This is an **accepted** decision (owner approval 2026-11-15T17:06:17-03:00).
Implementation remains a separate stage that must use the [M3 format-v2 acceptance
plan](../quality/m3-format-v2-acceptance.md). The public names in this ADR (`Movie`,
`MovieAnchor`, `MovieMeta`, `MovieStep`, `MovieError`, `ReplayOutcome`, `InputFrame`,
constants) describe the contract shape; spelling or parameter grouping may be refined by
the implementation, but the canonical bytes, limits, typed rejection boundaries and
semantics are the contract.

The increment must not add a second snapshot serializer, reader or compatibility
predicate, a new crate or a new dependency.

### 2. v2 container: one canonical `.cfmovie` file

A v2 movie is exactly one file with the conventional `.cfmovie` extension. As in v1, the
extension is not a trust boundary; the decoder validates the magic, versions and
checksum.

```text
magic "CFMOVIE2"                          8 bytes
u16 movie_format_version                  2 bytes, little-endian, value 2
u16 input_schema_version                  2 bytes, little-endian, value 2
u64 movie_id                              8 bytes, little-endian, non-zero
u16 name_length                           2 bytes, little-endian; 0 means no name
name                                      name_length UTF-8 bytes, at most 256
u64 parent_movie_id                       8 bytes, little-endian; 0 means no parent
u64 start_tick                            8 bytes, little-endian
u64 snapshot_length                       8 bytes, little-endian
embedded snapshot                         snapshot_length bytes, verbatim
                                          Snapshot::to_bytes() output
u64 anchor_count                          8 bytes, little-endian
anchor records                            anchor_count records, ordered by step index:
                                            u64 anchor_step_index          8
                                            u64 anchor_snapshot_length     8
                                            anchor snapshot                length bytes,
                                                                          verbatim
                                                                          Snapshot::to_bytes()
u64 step_count                            8 bytes, little-endian
step records                              step_count × 72 bytes, ordered:
                                            u64 outcome_tick               8
                                            u32 ports[0]                   4
                                            u32 ports[1]                   4
                                            u32 ports[2]                   4
                                            u32 ports[3]                   4
                                            i16 axes[0]                    2
                                            i16 axes[1]                    2
                                            i16 axes[2]                    2
                                            i16 axes[3]                    2
                                            u64 keyboard                   8
                                            state hash                    32
trailing SHA-256                          32 bytes over every preceding byte
```

All integer fields are little-endian. Signed axes are encoded as their two's-complement
16-bit value (write the `i16` as `u16`), not as text. State hashes are the raw 32 bytes of
`StateHash::as_bytes`. The fixed overhead without a name is **94 bytes**:

```text
8 magic + 2 format + 2 schema + 8 movie id + 2 name length + 8 parent id
+ 8 start tick + 8 snapshot length + 8 anchor count + 8 step count + 32 checksum
= 94 bytes
```

The maximum name adds 256 bytes, so the fixed overhead including the largest name is 350
bytes. A v2 step record is exactly **72 bytes**; an anchor record adds **16 bytes of
framing** plus its snapshot bytes.

Canonical encoding rules:

- There is exactly **one public writer** (`Movie::to_bytes`) and **one public reader**
  (`Movie::from_bytes`) in `chronofork_sessions::movie`, both built on
  `chronofork_core::codec::{Reader, Writer}`. v2 is a version branch of that one format
  family, not a second movie module or a second codec.
- Field order, widths, endianness and presence rules are fixed. The only optional
  encodings are the bounded name, the parent id sentinel `0` and a zero count. A
  zero-anchor and/or zero-step v2 movie is valid.
- No maps, sets, iteration order, padding, alignment, compression, encryption,
  normalization or host-dependent content. Anchor order is the explicit table order,
  which the format requires to be strictly increasing by step index.
- The trailing SHA-256 is computed with `chronofork_core::hash_bytes` over **all** bytes
  preceding it, including the magic, metadata, the embedded checkpoint (and its own
  checksum), the anchor count, every anchor record and snapshot, and every step record.
  The embedded checkpoint and each anchor snapshot keep their own independent checksum.
- A v2 movie is canonical: two movies are byte-equal exactly when their format version,
  metadata, embedded checkpoint bytes, ordered anchor records (index plus snapshot
  bytes) and ordered step records are equal. Appending, prepending or reordering bytes is
  a rejection, never a reinterpretation.

#### v2 limits

| Constant | Value | Applies to |
|---|---|---|
| `MAX_MOVIE_BYTES` | 256 MiB (268,435,456 bytes) | one complete v1 or v2 movie file |
| `MAX_STEPS` | 4,194,304 (2^22) | v1 step count, unchanged |
| `MAX_V2_STEPS` | 2,000,000 | v2 step count |
| `V2_FIXED_OVERHEAD` | 94 bytes | fixed v2 header fields plus the trailing checksum, without a name |
| `V2_STEP_RECORD_LEN` | 72 bytes | one schema-2 step record |
| `ANCHOR_RECORD_OVERHEAD` | 16 bytes | one anchor's index and length fields, excluding its snapshot |
| `MAX_ANCHORS` | 65,536 (2^16) | anchor table entry count |
| `MAX_EMBEDDED_SNAPSHOT_BYTES` | 64 MiB (67,108,864 bytes) | the embedded initial checkpoint |
| `MAX_ANCHOR_SNAPSHOT_BYTES` | 64 MiB (67,108,864 bytes) | one anchor snapshot; the same per-checkpoint budget class as the embedded checkpoint |
| `MAX_ANCHOR_BYTES` | 48 MiB (50,331,648 bytes) | cumulative snapshot bytes over all anchors; the effective multi-anchor bound |
| `MAX_NAME_BYTES` | 256 UTF-8 bytes | the optional movie name, unchanged from v1 |

The limits and the file bound are consistent. A maximal v2 movie is:

```text
94 fixed bytes
+ 256 bytes maximum name
+ 67,108,864 bytes maximum embedded checkpoint (64 MiB)
+ 65,536 × 16 = 1,048,576 bytes anchor framing
+ 50,331,648 bytes maximum cumulative anchor snapshots (48 MiB)
+ 2,000,000 × 72 = 144,000,000 bytes step records
= 262,489,438 bytes
```

which is below `MAX_MOVIE_BYTES` by 5,946,018 bytes. Every combination of the individual
limits therefore fits the file bound. Raising any v2 limit requires an ADR amendment, new
rejection tests and verifier evidence; an implementation must not silently raise one.
`MAX_V2_STEPS` is deliberately lower than `MAX_STEPS` because a schema-2 record is 72
bytes rather than 44; that is a documented format cost, not a performance target.

Decoding a maximal movie allocates at most the declared step vector
(`step_count × size_of::<MovieStep>`), the cumulative anchor snapshot bytes and the
embedded checkpoint; each is bounded by the limits above, and no allocation is sized from
a declared value before that value has been checked against the remaining bytes.

A declared length or count is validated against these limits **and** against the bytes
that actually remain before a buffer is allocated, a slice is taken or a `Vec` is
reserved. All arithmetic is checked; overflow is a typed rejection. Bounded decode is
specified in §4.

### 3. v1 compatibility and the reject-only version matrix

v1 remains a first-class, permanently readable format:

- `Movie::from_bytes` accepts both v1 and v2 files. The v1 decode path is the existing
  code, moved only if necessary into an internal version function; its field order,
  validation order, typed errors, limits and bytes must not change.
- `Movie::to_bytes` writes the version stored in the decoded/constructed `Movie`. A v1
  movie re-encodes to exactly the same bytes as before. A movie recorded through the
  existing `Movie::record` with no v2 feature is a v1 movie and remains byte-identical.
- v1 files keep `input_schema_version = 1`, 44-byte step records and no anchor table.
- The v1 reader must not require, invent or normalize an anchor table.

**Writer selection.** There are two explicit construction/recording entry points, not a
mode flag that can silently change v1 bytes:

| Entry point | Produces | Used when |
|---|---|---|
| `Movie::from_parts` / `Movie::record` | v1 (format 1, schema 1, no anchors) | no v2 feature is requested; the existing API and CLI default |
| `Movie::from_v2_parts` / `Movie::record_v2` | v2 (format 2, schema 2, optional anchors) | the caller requests anchors and/or multi-device input |

The CLI's default `tas record` invocation uses the v1 path. The v2 path is used only when
the caller asks for a v2 feature (§9). Invoking `Movie::record_v2`/`Movie::from_v2_parts`
is itself the explicit v2 request; the CLI only invokes those entry points when a v2 flag
is present. There is no automatic upgrade and no downgrade.

**Version/schema matrix (reject-only, no migration).** The public reader verifies the
trailing checksum and then dispatches on the exact 8-byte magic: `CFMOVIE1` enters the
retained v1 decode path and `CFMOVIE2` enters the v2 decode path. Each path validates its
own format and schema version, so a recognized magic carrying the other family's version
is an `UnsupportedFormatVersion` naming that family's supported version. This keeps the
existing v1 rejection taxonomy exactly intact (a `CFMOVIE1` file with format 2 is still
`UnsupportedFormatVersion { found: 2, supported: 1 }`, as the v1 tests require) and adds
no third "magic/version mismatch" error category.

| Magic (8 bytes) | `movie_format_version` | `input_schema_version` | Result |
|---|---|---|---|
| `CFMOVIE1` | 1 | 1 | Accepted as v1; existing behavior |
| `CFMOVIE2` | 2 | 2 | Accepted as v2 |
| `CFMOVIE1` | 1 | anything else | `UnsupportedInputSchema { found, supported: 1 }` |
| `CFMOVIE2` | 2 | anything else | `UnsupportedInputSchema { found, supported: 2 }` |
| `CFMOVIE1` | anything other than 1 (including 2 or 3) | any | `UnsupportedFormatVersion { found, supported: 1 }` |
| `CFMOVIE2` | anything other than 2 (including 1 or 3) | any | `UnsupportedFormatVersion { found, supported: 2 }` |
| any other 8 bytes | any | any | `BadMagic`; nothing further is parsed |
| `CFMOVIE1`/`CFMOVIE2` with a recognized pair | 1/1 or 2/2 | | Only accepted combinations; there is no format-2/schema-1 or format-1/schema-2 mode |

A future v3 magic is rejected as `BadMagic` until a future reader recognizes it; this is
deliberate fail-closed behavior, not an implicit upgrade path. **No migration exists:**
v1 files are never rewritten to v2, v2 files are never downgraded, and an unsupported
version is never reinterpreted as a nearby version.

**Format-agnostic operations.** `compare_movies` compares recorded
`(outcome_tick, state_hash)` expectations for v1 and v2 alike. The format version, anchor
table, input-schema version and metadata never decide equality. A v1 and a v2 movie with
the same checkpoint identity and the same recorded expectation sequence therefore compare
identical; anchors are seek metadata, not execution expectations. Full-range `verify`
(no `--from`) always imports the embedded checkpoint and executes every step for both
versions, so it remains the complete verification path.

### 4. Decode order, bounds and typed rejection

The v2 decoder is a pure byte parser: no filesystem I/O, no emulation. The CLI reads the
file with a bound before calling the reader, and the reader re-checks the slice length so
library callers get the same protection. The public entry point performs the shared
length, checksum and magic dispatch; the two version branches then own their own field
decoding.

Decode order (fail closed; all arithmetic checked):

1. Reject a slice shorter than the 86-byte v1 minimum fixed overhead (the smallest of the
   two formats) or longer than `MAX_MOVIE_BYTES` before reading a field. A v2 slice shorter
   than 94 bytes is rejected by the v2 branch after dispatch (step 4).
2. Split the final 32 bytes from the body and verify `SHA-256(body) == trailing
   checksum`. Reject on mismatch before parsing any field.
3. Read the 8-byte magic. Accept exactly `CFMOVIE1` or `CFMOVIE2`; anything else is
   `BadMagic`. `CFMOVIE1` enters the retained v1 decode path, whose behavior and typed
   errors are unchanged. `CFMOVIE2` enters the v2 path below.
4. **v2:** reject a slice shorter than the 94-byte v2 fixed overhead with
   `TooShort { minimum: 94, found }`.
5. Read `movie_format_version` and require exactly 2; otherwise
   `UnsupportedFormatVersion { found, supported: 2 }`. Read `input_schema_version` and
   require exactly 2; otherwise `UnsupportedInputSchema { found, supported: 2 }`.
6. Read `movie_id`, `name_length`, the name, `parent_movie_id` and `start_tick` with the
   v1 rules: non-zero movie id; `name_length` bounded by `MAX_NAME_BYTES` and by the
   remaining body before reading; the name bytes validated for UTF-8 and control
   characters after reading; parent non-zero and different from the movie id; and
   `name_length == 0` meaning absent.
7. Read `snapshot_length` as `u64`, convert with `usize::try_from`, and reject above
   `MAX_EMBEDDED_SNAPSHOT_BYTES` before slicing. Then require the remaining body to
   contain `snapshot_length + 8 + 8` bytes with checked arithmetic (the anchor count and
   step count must still fit) **before** slicing or copying. Decode the embedded bytes with
   `Snapshot::from_bytes` only and require `start_tick == snapshot.tick()`. A malformed,
   truncated, checksum-bad or unsupported snapshot is a typed movie error wrapping the
   session error (`InvalidEmbeddedSnapshot`).
8. Read `anchor_count`. Reject above `MAX_ANCHORS` before allocation. Compute
   `anchor_count × 16` with checked multiplication and require
   `remaining >= framing + 8` (the step count must still fit) before reserving the anchor
   vector. Then, for each anchor in order:
   - read `anchor_step_index` as `u64` and keep the raw value for the later range check;
     do not convert it to `usize` before the step count is known;
   - read `anchor_snapshot_length` (`u64`);
   - reject a per-anchor length above `MAX_ANCHOR_SNAPSHOT_BYTES`;
   - add the length to a checked cumulative total and reject above `MAX_ANCHOR_BYTES`;
   - require the remaining bytes to contain `length + 8` (at minimum the step count) with
     checked arithmetic **before** allocating or slicing the snapshot;
   - read the snapshot bytes and parse them with `Snapshot::from_bytes` only, wrapping any
     error as `InvalidAnchorSnapshot { anchor_ordinal, source }`; keep the parsed tick and
     state hash and the raw bytes, and drop the parsed backend state;
   - require the anchor indices to be strictly increasing as they are read
     (`AnchorIndexNotIncreasing`).
9. Read `step_count`. Reject above `MAX_V2_STEPS` before allocation. Compute
   `step_count × 72` with checked multiplication and require the remaining bytes to
   contain the product **before** `Vec::with_capacity`. Only then allocate and decode the
   records.
10. Validate the step tick sequence exactly as v1 does: strictly increasing and the first
    outcome tick greater than `start_tick`. The format does not require a fixed `+1`
    quantum because the quantum is backend-defined.
11. Validate every anchor binding now that both sections are decoded, in anchor order:
    - `anchor_step_index` (compared as `u64`) `< step_count`, otherwise
      `AnchorIndexOutOfRange` (a zero-step movie with any anchor fails here); only after
      this check is the index converted to `usize`;
    - the anchor snapshot tick equals the recorded outcome tick of that step, otherwise
      `AnchorTickMismatch`;
    - the anchor snapshot state hash equals the recorded expected state hash of that step,
      otherwise `AnchorStateHashMismatch`.
12. Call `Reader::finish()` and reject any trailing bytes inside the body.
13. Return the validated `Movie`. Backend compatibility is decided only when a replay
    imports a checkpoint through the existing path (§5, §6).

The writer's validating v2 constructor applies the **same** checks (metadata, bounds,
strictly increasing indices, index range, snapshot parse, tick/hash binding, total file
size) before handing the value to `Movie::to_bytes`; it is not a second, weaker encoder.
A test that needs a deliberately divergent v2 movie builds it through that constructor
with a valid trailing checksum.

**Anchor canonicality.** The decoder does not re-encode an anchor snapshot to prove it is
canonical. It accepts what `Snapshot::from_bytes` accepts and stores the bytes verbatim;
the writer is responsible for embedding the exact `Snapshot::to_bytes()` output, and
round-trip byte-equality tests enforce that. Re-encoding at decode would allocate a
second copy solely to compare it and would duplicate the writer's obligation without
strengthening the checksum, which already covers the stored bytes.

Typed v2 error categories (names are the contract shape; the implementation may refine
spelling, not boundaries):

| Category | Examples | Contract |
|---|---|---|
| Container/bounds | `TooShort`, `FileTooLarge`, `Truncated`, `TrailingBytes`, `TooManySteps { limit, found }` (v2 limit), `TooManyAnchors { limit, found }`, `EmbeddedSnapshotTooLarge { limit, found }`, `AnchorSnapshotTooLarge { anchor_ordinal, limit, found }`, `AnchorTableTooLarge { limit, found }` | Rejected before allocation or reinterpretation |
| Integrity | `ChecksumMismatch` | No field is trusted before the trailing checksum passes |
| Format/schema | `BadMagic`, `UnsupportedFormatVersion { found, supported }`, `UnsupportedInputSchema { found, supported }` | Explicit rejection; no migration |
| Metadata | `InvalidMovieId`, `InvalidParentMovieId`, `InvalidName` | Same v1 rules; no guessing, no path use |
| Binding | `StartTickMismatch { declared, snapshot }`, `AnchorIndexNotIncreasing { anchor_ordinal, previous, found }`, `AnchorIndexOutOfRange { anchor_ordinal, index, step_count }`, `AnchorTickMismatch { anchor_ordinal, index, snapshot_tick, recorded_tick }`, `AnchorStateHashMismatch { anchor_ordinal, index, snapshot_hash, recorded_hash }` | Structural binding validated at decode and construction |
| Embedded checkpoint | `InvalidEmbeddedSnapshot(SessionError)` preserving `InvalidSnapshot`/`IncompatibleSnapshot` | Never flattened to a string |
| Anchor snapshot | `InvalidAnchorSnapshot { anchor_ordinal, source: SessionError }` | Parsed only by `Snapshot::from_bytes`; never by a second parser |
| Recording | `SnapshotCapture(SessionError)`, `AnchorCaptureFailed { anchor_step, source: SessionError }`, `RecordFailed { input_index, source: SessionError }`, `InvalidAnchorInterval { interval }` | Typed failure before a movie is returned; the source session is never advanced or mutated |
| Replay import | `SnapshotImport(SessionError)` for the embedded checkpoint, `AnchorImport { anchor_step, source: SessionError }` for an anchor | Compatibility decided only by the existing import path/`validate_envelope` |
| Replay | `TickMismatch`, `Divergence`, `ReplayFailed { input_index, source }`, plus the existing ranged-replay categories | First mismatch only; no resynchronization, no partial pass |

No error is stringly typed. `Display` and `Error` preserve the underlying session/backend
error where one exists. Arbitrary bytes of any length must be rejected without a panic;
every decoder gets an arbitrary-byte/no-panic test (anti-slop AS-12), and a declared huge
count or length is tested on a tiny file to prove rejection before allocation. Because
the trailing checksum is verified first, a rejection test for a later structural field
must rebuild a valid trailing checksum so the intended validator is exercised.

### 5. Anchors: format, validation and recording

**Definition.** An anchor is a restorable checkpoint of the recording session captured
**after** the recorded step at zero-based index `k`. Its `anchor_step_index` is `k` (not
`k + 1`), so `0 <= k < step_count`. The embedded initial checkpoint is the implicit
anchor before step 0 and is not repeated in the anchor table. A zero-step movie has no
valid anchor position and must have `anchor_count == 0`.

**Payload.** An anchor record stores only `k`, a length and the exact
`Snapshot::to_bytes()` bytes of the checkpoint captured after step `k`. It does not store
a second copy of the tick, state hash, backend identity or configuration hash; those are
read from the snapshot by `Snapshot::from_bytes`. The snapshot's snapshot id, origin
session id, backend id/version and config hash remain diagnostic envelope values and are
not anchor identity fields. The anchor is valid only when its snapshot tick and state
hash match the recorded expectation of step `k`.

Anchors are built through the validating constructor
`MovieAnchor::new(step_index: usize, snapshot_bytes: Vec<u8>)`, which parses the snapshot
once with `Snapshot::from_bytes` and stores the index, bytes, tick and state hash behind
private fields and read-only accessors. `Movie::from_v2_parts` then validates ordering,
range and step binding. Tests build deliberately invalid anchors through the same public
paths, not by mutating fields or re-parsing bytes in a second implementation.

**Ordering and spacing.** `anchor_step_index` is strictly increasing and unique. The v2
format does **not** require evenly spaced anchors; constant spacing is a recorder policy
(§7), not a format invariant. Readers must accept any strictly increasing subset.

**Binding validation.** At decode and at validating construction, in anchor order:

1. `anchor_count <= MAX_ANCHORS`.
2. Each snapshot length `<= MAX_ANCHOR_SNAPSHOT_BYTES`, and the cumulative snapshot bytes
   `<= MAX_ANCHOR_BYTES` (checked addition).
3. Each snapshot parses only through `Snapshot::from_bytes`; failure is
   `InvalidAnchorSnapshot` with the ordinal and the preserved `SessionError`.
4. Each index is strictly greater than its predecessor (`AnchorIndexNotIncreasing`).
5. Each index is `< step_count` when compared in `u64` (`AnchorIndexOutOfRange`); the
   conversion to `usize` happens only after this check.
6. `snapshot.tick() == steps[k].outcome_tick()` (`AnchorTickMismatch`).
7. `snapshot.state_hash() == steps[k].state_hash()` (`AnchorStateHashMismatch`).

A structurally valid anchor whose snapshot is import-incompatible with this build (for
example a foreign backend identity or configuration hash) decodes successfully and fails
only when a replay imports it, with `AnchorImport`; that is a replay rejection, not a
format reinterpretation, and there is still only one compatibility predicate.

### 6. Anchor-based seek

**Range validation first.** `ReplayRange::new` validates the requested `(from, until]`
exactly as in ADR-0005, before any anchor selection, checkpoint import, session creation
or step. An invalid range returns its typed range category and leaves the manager, the
movie and the files unchanged, even when the movie carries a valid or an
import-incompatible anchor.

**Selection.** After range validation, the replay chooses the **nearest anchor at or
before the requested prefix end**:

- Let `p` be the greatest step index with `steps[p].outcome_tick() <= from`, or none when
  `from == start_tick` (the prefix is empty).
- Choose the anchor with the greatest `anchor_step_index <= p`. Because anchors are
  sorted, this is a binary search over the anchor table. An anchor whose step tick is
  after `from` is never selected, so the window is never partially skipped.
- If no anchor qualifies, the replay falls back to the embedded checkpoint and the
  current linear path.

**Execution.** With an anchor at `k`:

1. Import the anchor's verbatim snapshot bytes with
   `SessionManager::create_session_from_snapshot_bytes`, so backend identity, version,
   payload version and configuration compatibility are still decided by the existing
   `validate_envelope` predicate. An import failure is
   `AnchorImport { anchor_step: k, source }`; no second session is created.
2. Initialize the observed state from the anchor's parsed tick and state hash.
3. Apply the recorded inputs for steps `k + 1` through the step whose recorded outcome
   tick is `until`, each exactly once, comparing the observed outcome tick and state hash
   with that step's recorded expectation exactly as the current replay does.
4. Stop at the first mismatch with the existing `TickMismatch`/`Divergence` (exact index,
   tick, expected and observed hash) or a typed `ReplayFailed`; never resynchronize and
   never report a later mismatch instead.

Without an anchor, steps 0 through `until` are executed from the embedded checkpoint,
which is the current ADR-0005 behavior unchanged.

**Report (`ReplayOutcome`) additions.**

| Field | Meaning |
|---|---|
| `anchored` | `true` when the replay started from an anchor snapshot, `false` for the embedded checkpoint |
| `anchor_step` | `Some(k)` when anchored, `None` otherwise |
| `anchor_tick` | `Some(recorded outcome tick of step k)` when anchored, `None` otherwise |
| `prefix_steps` | Unchanged ADR-0005 meaning: recorded steps with `outcome_tick <= from` (a logical range fact, not necessarily executed) |
| `window_steps` | Unchanged ADR-0005 meaning |
| `replayed_steps` | Steps **actually executed and verified** in this invocation. Unanchored it still equals `prefix_steps + window_steps`; anchored it is `(index at until) - k`, so some prefix steps may be covered by the anchor instead of being executed |
| `final_tick`, `final_state_hash` | The observed values at `until`; for a clean anchored replay they equal the recorded boundary and the full-replay values |

The invariant is `anchored == anchor_step.is_some() == anchor_tick.is_some()`. For the
same `until`, an anchored replay must report the same `final_tick` and
`final_state_hash`, and the same first divergence **after** the anchor, as a full replay;
`prefix_steps` and `window_steps` remain equal because they describe the movie range, not
the execution strategy.

**What prefix verification means when an anchor is used.** This is the deliberate,
documented delta from ADR-0005:

- ADR-0005 range replay executes and verifies every step from the embedded checkpoint to
  `until`; the range only narrows what is reported.
- A v2 anchored replay does **not** re-execute the steps at or before the selected
  anchor. The anchor boundary is instead validated structurally: the anchor snapshot's
  tick and state hash must equal the recorded expectation of the anchor step, and the
  snapshot bytes are covered by the movie's trailing checksum and the snapshot's own
  checksum. Steps after the anchor through `until` are executed and verified exactly.
- Therefore an anchored ranged replay verifies the recorded suffix from the anchor
  boundary onward. It does **not** independently recompute the pre-anchor prefix, and it
  cannot detect a consistently tampered pre-anchor expectation that was re-signed into
  the movie. Full-range `verify` (no `--from`, so `from == start_tick`) never selects an
  anchor and still verifies every step; it remains the complete verification path.
- CLI help and durable documentation must state this difference, and anchored output
  reports `prefix_verified=0`. Absence of the anchor fields means the invocation used the
  unchanged ADR-0005 path.

**Fallback and edge cases.**

- A v1 movie has no anchors, so every replay uses the embedded checkpoint and behaves
  exactly as ADR-0005.
- A v2 movie with `anchor_count == 0` has the same fallback and reports no anchor fields.
- `from == start_tick`: the prefix is empty, no anchor can be at or before it, and the
  replay is linear.
- `from == end_tick` on a movie with one or more steps is an empty range and is rejected
  before selection; `until == end_tick` with a smaller `from` selects the greatest anchor
  at or before that `from`. An anchor on the final recorded step is a valid format value
  but cannot be selected by a valid non-empty range, because `from < until <= end_tick`.
- A zero-step movie has no anchors; its only valid range is `[start_tick, start_tick]`
  and the final hash is the embedded checkpoint hash.
- A rejected range creates no session and executes no step. A valid replay registers
  exactly one fresh imported session (anchor or embedded checkpoint); a valid replay that
  fails after import may leave that single fresh session registered, exactly as the v1
  contract already allows, while never touching a pre-existing session, a movie or a
  file.

**Cost.** Anchored execution costs O(distance from the anchor to the `until` step) plus
O(log a) selection, bounded by the v2 step limit. It is not O(1), does not promise a
bound when no anchor exists, and repeated seeks re-import and re-execute. No timing
figure is an equality gate.

### 7. Input schema 2: multi-device frames

**`InputFrame` shape.** `chronofork_core::InputFrame` gains fixed device fields while
keeping the v1 surface:

```text
InputFrame
  ports:  [u32; 4]   port 0 = the v1 backend-defined button mask;
                     ports 1..=3 = additional digital ports
  axes:   [i16; 4]   signed analog axes, 0 = neutral
  keys:   u64        keyboard bitmap, bit i = key i
```

- `InputFrame::new(buttons: u32)` keeps its signature and sets `ports[0] = buttons` with
  all other fields neutral (zero). `buttons()` returns `ports[0]`. `NEUTRAL` and
  `Default` remain all-zero, so a v1 frame is semantically unchanged.
- The constructor `InputFrame::with_devices` takes the four `u32` port
  masks, the four `i16` axes and the `u64` keyboard bitmap and creates a multi-device
  frame. Read-only accessors `ports()`, `axes()` and `keys()` expose the fields. The exact
  constructor spelling may be refined by implementation; the field widths, neutral
  semantics and v1 preservation are the contract.
- One `InputFrame` is still supplied to exactly one `Backend::step`. There is no event
  queue, no per-scanline input and no input scheduled by host arrival time. Port 0 keeps
  the v1 backend-defined bit contract; ports 1–3, the axes and the keyboard bitmap are
  backend-defined and must be documented by a backend that consumes them.

**Schema-2 step record.** The 72-byte record in §2 encodes exactly the fields above, in
the fixed order `outcome_tick`, `ports[0..4]`, `axes[0..4]`, `keyboard`, `state_hash`.
The `input` accessor of `MovieStep` returns an `InputFrame` reconstructed by
`InputFrame::with_devices` (or the equivalent); a v1 movie reconstructs
`InputFrame::new(mask)`. `MovieStep::new` keeps working for v1 callers.

**Backend behavior.** A backend that consumes only port 0 must ignore the other fields
and document that limitation; it is not evidence of multi-device support. The Mega Drive
slice currently ignores all input and remains outside the input-TAS claim. Backends do
not gain a new trait method, capability bit or device-negotiation protocol in this
increment: the step signature is unchanged, and unsupported device classes have no
effect. Recording such a backend produces well-formed v2 input records whose device
fields have no emulated effect; the artifact must not be described as real-hardware input
TAS.

**v1 non-regression.** A schema-1 movie decodes its 44-byte records into
`InputFrame::new(mask)`, with ports 1–3, axes and keyboard neutral. Replaying it is
identical to the pre-change build: same inputs to the backend, same state hashes, same
first divergence. A v2 recording with neutral device fields produces the same observed
state trajectory as the corresponding v1 recording on a backend whose port-0 behavior is
unchanged; only the container/record size differs.

### 8. Synthetic backend deterministic consumption

The acceptance backend must consume the new fields, or v2 multi-device records would be
behaviorally empty. The design keeps the synthetic default program, payload format
version 1 and canonical state body unchanged, and extends the existing `APPLY_INPUT`
(`0x09`) instruction:

1. **Port 0 (v1 effect, always).** `acc = acc.wrapping_add(u32::from((ports[0] & 0xff) as
   u8))`; `ram[0x10] = (acc & 0xff) as u8`. This is the exact existing behavior.
2. **Ports 1, 2, 3 (only when non-neutral).** For `p` in `1..=3`, if `ports[p] != 0`:
   `acc = acc.wrapping_add(u32::from((ports[p] & 0xff) as u8))`;
   `ram[0x10 + p] = (acc & 0xff) as u8`. Order is ascending port index.
3. **Axes 0–3 (only when non-neutral).** For `a` in `0..4`, if `axes[a] != 0`:
   `acc = acc.wrapping_add(u32::from(axes[a] as u16))`;
   `ram[0x14 + a] = (acc & 0xff) as u8`. Order is ascending axis index.
4. **Keyboard (only when non-neutral).** If `keys != 0`, for `i` in `0..8`:
   `let byte = ((keys >> (8 * i)) & 0xff) as u8`;
   `acc = acc.wrapping_add(u32::from(byte))`;
   `ram[0x18 + i] = byte`. Bit 0 is the least-significant bit of byte 0.

A neutral device field has **no state effect**: it neither adds to the accumulator nor
writes RAM, which is why v1 frames (neutral extras) have exactly the old state hash. A
non-neutral field adds its defined contribution and stores a byte at the fixed address
above. The work is constant and bounded (at most three ports, four axes and eight
keyboard bytes per instruction), the order is fixed, and no host time, randomness or map
iteration is involved. The state body still contains exactly `pc`, `acc`, `tick`, `steps`,
`halted` and `ram` in the existing order; `PAYLOAD_FORMAT_VERSION` remains 1; the default
program bytes and therefore the default config hash are unchanged.

Rejected alternatives: adding separate opcodes would leave the default program (and the
CLI) without any device consumption or would change the default program/config hash and
invalidate v1 fixture hashes; changing the state body or payload version would invalidate
M1/M2/M3 evidence. Both are outside this increment.

### 9. Recording semantics

**v1 recording is unchanged.** `Movie::record` keeps capturing the source checkpoint
through `SessionManager::snapshot`/`snapshot_bytes`, importing it into a separate fresh
session, executing the inputs there and constructing a v1 movie. The source session's
tick, state hash and mutable state are never advanced or mutated; the checkpoint record
stored by `snapshot` is the only permitted source-side effect.

**v2 recording.** `Movie::record_v2(manager, source, meta, inputs, anchor_interval)`
follows the same isolation rules and adds anchors:

1. **Preflight.** Reject `inputs.len() > MAX_V2_STEPS`; when `anchor_interval > 0`, reject
   an empty input sequence and a predicted anchor count
   `inputs.len() / anchor_interval > MAX_ANCHORS` before any execution. `anchor_interval
   == 0` means "no anchors" and is valid for a v2 movie (multi-device input only).
2. **Capture and import.** Capture the source checkpoint and import it into a separate
   fresh recording session exactly as v1 does. The source is not advanced.
3. **Execute and record.** For each input in order, step the recording session, append
   the schema-2 step expectation `(outcome_tick, full InputFrame, state_hash)` (72 bytes
   when encoded), and when `(input_index + 1) % anchor_interval == 0` (with
   `anchor_interval > 0`), capture an anchor:
   - `snapshot_id = manager.snapshot(recording_session)`;
   - read the captured record with `manager.snapshot_record(...)` to obtain the canonical
     tick and state hash (the existing checkpoint path, not a second parser);
   - `bytes = manager.snapshot_bytes(recording_session, snapshot_id)`, which is exactly
     `Snapshot::to_bytes()` output;
   - require the captured tick and state hash to equal the just-recorded step's outcome
     (`AnchorTickMismatch`/`AnchorStateHashMismatch` otherwise);
   - enforce the per-anchor and cumulative anchor byte bounds before storing the anchor.
   A failed step, checkpoint capture, snapshot read/serialize or byte-budget check
   returns a typed error (`RecordFailed`, `AnchorCaptureFailed`, `AnchorSnapshotTooLarge`,
   `AnchorTableTooLarge`) and produces **no movie**; the source and any existing output
   file are untouched.
4. **Construct and encode.** Pass metadata, embedded checkpoint bytes, anchors and steps
   to the validating v2 constructor, which repeats the decode-time binding, ordering and
   size checks before encoding. The writer never emits a movie it could not read back.

**Anchor interval policy.** `anchor_interval = N > 0` places an anchor after recorded
steps `N - 1`, `2N - 1`, `3N - 1`, … while the index is `< step_count`; the count is
`step_count / N`. `N = 0` places none. The interval itself is not stored: only the
resulting ordered anchor table is canonical. The CLI rejects `N > 0` when `N` exceeds the
recorded step count (no anchor position exists), and rejects a predicted count above
`MAX_ANCHORS`. This policy is deterministic and independent of host time, input values
and machine speed.

**Determinism scope.** Anchor indices are a pure function of the step count and the
requested interval. Anchor snapshot bytes come from the deterministic checkpoint path and
contain the snapshot's diagnostic identity fields, just as the v1 embedded checkpoint
already does. Canonical v2 bytes are therefore deterministic for identical inputs
**including identical session/snapshot identity conditions**: two no-root CLI invocations
use a fresh manager and are byte-identical, and library callers that want byte equality
must start from the same identity conditions. There is no host time, path, process id, nonce,
randomness or filesystem metadata in the format. This ADR does not claim that a recording
made in a differently populated manager is byte-identical to one made in a fresh manager;
that limitation already exists for the v1 embedded checkpoint and is not widened here.

### 10. CLI surface (accepted)

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

- **`record` port flags.** `--input` remains port 0 (a `u32`, default 0). `--ports` takes
  exactly three comma-separated decimal `u32` values for ports 1–3. `--axes` takes exactly
  four comma-separated decimal signed 16-bit values for axes 0–3. `--keys` takes a decimal
  `u64` keyboard bitmap. Each value is fixed for every recorded step, matching the
  existing fixed-`--input` recording model. Per-step/variable input scripts are deferred.
- **`--anchor-interval <N>`** is a decimal `u64`/`usize`: `0` (default) records no
  anchors; `N > 0` records one anchor after every `N` recorded steps. `N > 0` with
  `--steps 0` or `N > steps` is a typed CLI failure, as is a predicted anchor count above
  `MAX_ANCHORS`.
- **v2 selection.** The v2 writer is used only when `--ports`, `--axes` or `--keys` is
  present, or `--anchor-interval N` with `N > 0` is present. A plain `record` invocation,
  and `--anchor-interval 0` with no multi-device flag, use the v1 writer and produce
  byte-identical v1 output. A v2 request with `--steps` above `MAX_V2_STEPS` is a typed
  CLI failure that names the v2 limit; `--root` seeding is unchanged.
- **`record` output.** The existing `record=movie` block is unchanged in field order and
  meaning; `movie_version=2` and `input_schema=2` appear naturally for a v2 movie. When
  the movie has anchors, append `anchors=<count>` after `movie_sha256`; a v2 movie with no
  anchors does not gain an `anchors` line. `movie_sha256` remains the SHA-256 of the exact
  file bytes.
- **`play`/`verify` output.** Default (no range flags) and all v1/anchor-less ranged
  output stays byte-identical to ADR-0005: no anchor fields. When an anchor is actually
  used, append to the ranged block, in this order, `anchored=1`, `anchor_step=<k>`,
  `anchor_tick=<recorded outcome tick>`, `prefix_verified=0`. `replayed_steps` continues
  to be the executed-and-verified count (smaller than `prefix_steps + window_steps` when
  anchored). The absence of `anchored=1` means the replay used the embedded checkpoint and
  the ADR-0005 path. `verified=1` on `verify` means the executed suffix verified and the
  anchor boundary binding holds; help text and docs explain that the pre-anchor prefix was
  not re-simulated.
- **`compare`.** Accepts v1 and v2 movies with the existing `record=comparison` block,
  verdicts and exit 0/1. Format version, anchors, input schema and metadata never decide
  equality; a v1 and v2 movie with equal checkpoint identity and equal
  `(outcome_tick, state_hash)` sequences compare identical. Malformed input still fails at
  decode before comparison.
- **Help and honesty.** `--help` documents every new flag and numeric format, the v2
  limits, the anchor selection rule, the anchored prefix limitation, and that anchors are
  a bounded seek optimization (never O(1)). It must state that in-process rewind/state
  cache and interactive stepping, rerecord/branch editing, real-hardware input TAS,
  import/export adapters and HTTP/MCP/GUI/daemon remain unimplemented, that the acceptance
  backend is the deterministic synthetic fixture, and that M3 is not complete.
- **Exit and stream contract (unchanged).** Exit 0 on success and 1 on any typed failure;
  stdout carries only `key=value` data; stderr carries `chronofork: <error>` diagnostics;
  no timestamp, host path, network access or telemetry in valid output; a failed record
  encodes the complete bytes first and writes through a sibling temporary path renamed on
  success, so it never replaces an existing valid movie file.

### 11. Architecture, module ownership and dependencies

- **`chronofork-sessions::movie`.** One public reader/writer family and one validating
  constructor per version. v1 functions stay; v2 decode/encode, `MovieAnchor`, `Movie`
  version/anchor fields, `Movie::from_v2_parts`, `Movie::record_v2` and the v2 limits live
  here. The implemented v2 branch is the private `movie::v2` submodule inside `movie.rs`;
  the optional `movie/v2.rs` file split was not needed and is not used. `Movie` exposes
  `format_version()`, `input_schema_version()`, `anchors()` and a read-only
  `anchor_at_or_before(tick)` selector used by replay; the codec stays shared and the v2
  branch is not a second reader/writer.
- **`chronofork-sessions::replay`.** The single `ReplayRange`/replay loop selects the
  nearest anchor at or before the prefix end and passes its start index into the same
  loop. There is still exactly one replay loop, one `ReplayOutcome` construction site and
  one comparison implementation. `Movie::verify` continues to delegate the full range.
- **`chronofork-core::InputFrame`.** Implemented fixed device fields (`[u32; 4]` ports,
  `[i16; 4]` axes, `u64` keys), `with_devices` and accessors. No new dependency, no trait
  change, no new capability bit.
- **`chronofork-test-backend`.** Implemented `APPLY_INPUT` extension in §8 and its unit
  tests. The default program, payload format version and state-body field list do not
  change.
- **`chronofork-cli::tas`.** Implemented record flags, v2 selection, block fields, help and
  process tests. `main.rs` keeps dispatch and shared helpers only.
- **No storage, backend-traits, Mega Drive, manifest or lockfile change.** No new crate,
  dependency, `unsafe`, `#[allow]`, `#[ignore]` or weakened test. `chronofork-sessions`
  still does not depend on a concrete backend.
- **Tests at the lowest useful layer (AS-08):** core input tests; synthetic backend
  instruction tests; sessions movie/replay v2 unit and integration tests; CLI
  process/format/exit tests. The CLI must not re-assert emulation semantics.

### 12. Verification plan and current status

The executable acceptance plan is [M3 format-v2
acceptance](../quality/m3-format-v2-acceptance.md), rows M3-A18 through M3-A27. It was
executed by the independent t7 verifier on 2026-11-15 and returned **PASS**: every row is
`executed - passed` with commands, exit codes, observed values and artifacts in
`.research/m3-v2/verification/` (`SUMMARY.txt`, `rows/A18-*.txt`–`A27-*.txt`,
`target-matrix.txt`, `audit/`, `targets/`, `cli/`, `demos/`, `probes/`, `gate/`, `tree/`
and `MANIFEST.sha256`). The dated gate-result block is in the verifier-owned
[quality gates](../quality/quality-gates.md). The tree under test was
`9643d74fd37097ebe487a398fe0a0259a3b3c7aa` (branch `agent/m3-complete`) plus the
increment-3 working-tree paths; `tree/state-comparison.txt` records that the
implementation/doc path set was unchanged by verification and that the verifier's only
tracked edit was the dated gate block. No commit or push was made.

The verification covers the v2 container and exact canonical sizes, the bounded decode
and typed rejection matrix, anchor capture/ordering/binding, nearest-anchor seek with the
documented prefix delta, schema-2 multi-device input, v1 byte/behavior non-regression, the
CLI process contract, determinism and debug/release equality, the full gate and demos,
M2/M3-A1–A17 regression and scope honesty. It does not verify any deferred item in §13.
This ADR and the acceptance plan are documentation, not executable evidence; the raw
artifacts are the evidence.

### 13. Deferred work and stated limitations

**In scope for this increment (delivered and verified for the deterministic synthetic
backend):** the v2 container and limits; anchor capture, validation and ordering;
anchor-based seek in the single ranged replay path with the reported fields; schema-2
multi-device input in core and the synthetic backend; v2 recording and CLI surface;
executable tests; honest documentation.

**Deferred, not implemented and never to be claimed by this increment:**

- in-process rewind buffer, state cache, eviction policy and interactive stepping;
- rerecording, branch editing, in-place movie editing and rerecord counters;
- real-hardware input TAS: the Mega Drive `step` ignores input, and wiring controllers is
  an emulation-semantics change under its own decision;
- import/export adapters for other TAS formats (BizHawk BK2 and others);
- checkpoint-reference movies, sidecar anchors and durable-anchor lookup;
- per-step or scripted input sequences, device events timed by emulated ticks, and
  per-scanline input;
- compression, encryption, authentication, deduplication and automatic retention;
- HTTP API, MCP, GUI and the Stage 6 daemon;
- O(1)/random-access seek, parallel/threaded replay, process isolation of replay and any
  performance or capacity target;
- cross-backend canonicalization, cross-platform runtime determinism or any
  hardware-accuracy claim.

**Stated limitations.** v2 movies are larger and capped at 2,000,000 steps; anchors cost
checkpoint bytes and only help when present before the requested prefix end; anchored
ranged replay does not re-verify the pre-anchor prefix; anchors do not make seek O(1) and
do not cache anything between invocations; the acceptance backend is the synthetic
fixture; unsupported device classes and the Mega Drive input path have no effect; a v2
movie from another backend version or configuration is rejected at replay import; and
M3 is not complete.

## Rationale

- **Leaving v1 untouched preserves every verified byte.** v1 was accepted, implemented and
  independently verified. A new magic/version with a retained v1 code path is the only
  option that adds features without invalidating M3-A1–A17 or the M1/M2 evidence.
- **A reject-only matrix is safer than a migration.** Explicitly accepting only `(1,1)`
  and `(2,2)`, with distinct typed errors for unknown magic, unsupported format and
  unsupported schema, makes corruption and version confusion diagnosable and prevents a
  reader from guessing.
- **Anchors reuse the one canonical snapshot envelope.** Storing verbatim
  `Snapshot::to_bytes()` bytes and parsing only with `Snapshot::from_bytes` keeps one
  serializer, one parser and one compatibility predicate. Any other anchor payload would
  duplicate the envelope or the predicate.
- **Binding anchors to recorded steps makes them trustworthy for seek.** A snapshot alone
  proves a state existed, not that it belongs to this recording. Requiring the snapshot
  tick and state hash to equal the recorded expectation of its index ties the anchor to
  the same execution that produced the expectations, and the trailing checksum covers the
  bytes.
- **Anchors at or before `from` preserve the window contract.** Selecting an anchor after
  `from` would skip window steps and could miss a divergence the range is supposed to
  report. Selecting at or before the prefix end keeps the whole window executed and
  verified; only the prefix is skipped, and that delta is stated and reported.
- **Neutral-field no-ops preserve v1 state hashes.** Extending `APPLY_INPUT` with
  conditional effects means a v1 frame (neutral extras) executes exactly the old path.
  Adding separate opcodes or always writing new RAM bytes would either leave the default
  program unable to consume devices or change existing hashes.
- **Fixed-width records make bounds simple.** A 72-byte step record and a 16-byte anchor
  framing let the reader validate declared counts against remaining bytes with checked
  multiplication before allocation, and give the writer one exact size formula.
- **A lower v2 step cap keeps the file bound honest.** A 72-byte record at the v1 step cap
  would exceed 256 MiB before metadata, checkpoint or anchors. The documented v2 cap keeps
  every maximal combination inside the existing file bound without raising it.
- **Std-only keeps the trust surface small.** The format needs fixed-width integers, the
  existing hash and the existing snapshot envelope; a serialization framework, archive
  library or database would add license, maintenance and failure surface without solving a
  demonstrated problem.

## Consequences

Positive:

- A v2 movie can seek from a deterministic anchor without re-simulating the pre-anchor
  prefix, while full-range verification remains complete.
- Multi-device input becomes a versioned, bounded schema that preserves every v1 file and
  the v1 `InputFrame` surface.
- v2 stays one portable, self-verifying file with explicit limits and typed rejection.
- The existing snapshot, compatibility, replay and comparison sources of truth remain
  single.
- The incremental cost is confined to one module family, one input type, one synthetic
  instruction and the thin CLI adapter.

Negative / costs:

- v2 movies are larger: a 72-byte step and one full checkpoint per anchor.
- v2 caps steps at 2,000,000 and anchors at 65,536 / 48 MiB cumulative, which are lower
  than a naive reading of the 256 MiB file bound might suggest.
- Anchored ranged replay is not a full prefix verification; users who need that must run a
  full-range `verify`.
- The movie module and the CLI grow, and the AS-14 review trigger may require an internal
  submodule split.
- The synthetic backend's `APPLY_INPUT` has more deterministic branches to test.

Risks / limitations:

- The acceptance claim is synthetic-backend only; the Mega Drive `step` still ignores
  input, so real-hardware input TAS is not implemented.
- Anchors add a second checkpoint-shaped object to the movie; a large real backend image
  makes anchor-heavy movies large, which is why the cumulative budget exists.
- A tampered pre-anchor expectation cannot be detected by an anchored ranged replay; this
  is a documented trade-off, and full-range verify is the check for it.
- A backend that ignores the new fields produces valid but behaviorally inert v2 records;
  the CLI and docs must not present that as device support.
- This ADR is **Accepted, implemented and verified for the deterministic synthetic backend
  only**: it asserts no hardware input TAS, no deferred capability and no M3 completion.

## Status

Accepted, implemented and independently verified for the deterministic synthetic backend.
The project owner approved ADR-0006 as proposed with the statement "Aprovar ADR-0006 como
proposto (Recomendado)" at 2026-11-15T17:06:17-03:00 (America/Fortaleza),
2026-11-15T20:06:17Z. The independent t7 verification executed M3-A18–A27 on the
uncommitted increment-3 tree at HEAD `9643d74fd37097ebe487a398fe0a0259a3b3c7aa` plus the
recorded working-tree paths and returned PASS on 2026-11-15; all rows are
`executed - passed` (Evidence and §12). The v1 format/increment-2 behavior remains
unchanged, the deferred items in §13 remain unimplemented, and M3 is **not complete**. The
verification made no commit or push; this documentation promotion changes only the
status/evidence text and claims no commit.

## Evidence

- **Owner approval (2026-11-15):** the project owner explicitly approved ADR-0006 as
  proposed with the statement "Aprovar ADR-0006 como proposto (Recomendado)" at
  2026-11-15T17:06:17-03:00 (America/Fortaleza), 2026-11-15T20:06:17Z. The approval covers
  the proposed contract, the v2 limits (2,000,000 steps; 65,536 anchors; 64 MiB
  embedded/anchor snapshot; 48 MiB cumulative anchor bytes; 256 MiB file), anchored ranged
  replay that may skip the pre-anchor prefix with `prefix_verified=0` while full-range
  verify remains complete, and the CLI flags/v2-selection rule (`--ports`, `--axes`,
  `--keys`, `--anchor-interval`; plain `record` remains v1). It is a decision approval,
  not an implementation or verification claim.
- **Executed verification (increment 3, t7 PASS, 2026-11-15):** tree HEAD
  `9643d74fd37097ebe487a398fe0a0259a3b3c7aa` (branch `agent/m3-complete`) plus the
  increment-3 working-tree paths; `CARGO_HOME=/tmp/t7-cargo`; every gate command exited 0
  (fmt, check, clippy `-D warnings`, workspace tests, rustdoc `-D warnings`,
  `./scripts/check.sh`, `git diff --check`); all targets at or above the pre-change
  baseline; both demos three times byte-identical to baseline; v1 movie/transcript and
  serialized-state regression identical; no added `#[allow]`/`#[ignore]`/`unsafe`;
  `Cargo.toml`/`Cargo.lock` unchanged; one reader/writer, one snapshot parser, one
  compatibility predicate and one replay loop. Raw evidence:
  `.research/m3-v2/verification/` (`SUMMARY.txt`, `rows/A18-*.txt`–`A27-*.txt`,
  `target-matrix.txt`, `audit/`, `targets/`, `cli/`, `demos/`, `probes/`, `gate/`,
  `tree/`, `MANIFEST.sha256`); dated gate block in
  [quality gates](../quality/quality-gates.md); baseline `.research/m3-v2/baseline/`.
  No commit, push or tag was made.
- **Implemented v2 contract (t7 verified):** `crates/chronofork-sessions/src/movie.rs`
  (one public `Movie::from_bytes`/`to_bytes` pair dispatching `CFMOVIE1`/`CFMOVIE2`; private
  `movie::v2` branch with `MovieAnchor`, `Movie::from_v2_parts`, `Movie::record_v2`, v2
  limits and typed rejection); `crates/chronofork-sessions/src/replay.rs` (anchor selection,
  start-index seek, `anchored`/`anchor_step`/`anchor_tick`, `AnchorImport`);
  `crates/chronofork-core/src/input.rs` (schema-2 `InputFrame`);
  `crates/chronofork-test-backend/src/lib.rs` (`APPLY_INPUT` device consumption);
  `crates/chronofork-cli/src/{tas,main}.rs` (v2 flags, anchored block, help);
  `crates/chronofork-sessions/tests/{movie_v2,replay}.rs` and
  `crates/chronofork-cli/tests/tas.rs`. The v2 branch is not a second reader/writer or
  snapshot parser.
- **Existing accepted v1 contract (regression evidence, not v2 acceptance):**
  [ADR-0004](0004-tas-movie-format.md) §2–§12 and its executed acceptance record
  [M3-A1–A12](../quality/m3-tas-replay-acceptance.md) (M3-T4 PASS, 2026-11-15);
  `crates/chronofork-sessions/src/movie.rs` (one `Movie::from_bytes`/`to_bytes`,
  `Movie::from_parts`, `Movie::record`, `Movie::verify`, `MovieStep`, `MovieMeta`,
  `MovieError`, v1 limits), `crates/chronofork-sessions/src/snapshot.rs`
  (`Snapshot::to_bytes`/`from_bytes`), `crates/chronofork-sessions/src/manager.rs`
  (`create_session_from_snapshot_bytes`, `snapshot`, `snapshot_record`,
  `snapshot_bytes`), and `crates/chronofork-backend-api/src/snapshot.rs`
  (`validate_envelope`).
- **Existing increment-2 contract (regression evidence, not v2 acceptance):**
  [ADR-0005](0005-partial-replay-and-branch-comparison.md) and its executed acceptance
  record [M3-A13–A17](../quality/m3-partial-replay-acceptance.md) (t5 PASS, 2026-11-15);
  `crates/chronofork-sessions/src/replay.rs` (`ReplayRange`, `ReplayOutcome`,
  `compare_movies`), `crates/chronofork-sessions/src/movie.rs` (`Movie::verify`
  delegation, `checkpoint_state_hash`) and `crates/chronofork-cli/src/tas.rs`.
- **Existing input and backend behavior before the increment (v1 regression evidence):**
  `crates/chronofork-core/src/input.rs` (`InputFrame`, `new`, `buttons`, `NEUTRAL`);
  `crates/chronofork-test-backend/src/lib.rs` (`DEFAULT_PROGRAM`, `PAYLOAD_FORMAT_VERSION`,
  `SyntheticBackend::step`, `APPLY_INPUT`, state body); `crates/chronofork-megadrive/src/backend.rs`
  (`fn step(&mut self, _input: InputFrame)` ignores input).
- **Existing storage bound reused by this design:** `StorageLimits::max_checkpoint_bytes`
  default 64 MiB in `crates/chronofork-sessions/src/storage/mod.rs`; ADR-0003 §11 limit
  table.
- **Research context:** [TAS and debugging](../research/tas-and-debugging.md) §6.3 and
  §10.2 (seek is replay from the nearest cached state; do not promise O(1) seek).
- **Executed acceptance plan:** [M3 format-v2
  acceptance](../quality/m3-format-v2-acceptance.md), rows M3-A18–A27, all
  `executed - passed` with their t7 commands/artifacts.
- **Tree state at verification:** branch `agent/m3-complete`, HEAD
  `9643d74fd37097ebe487a398fe0a0259a3b3c7aa`, plus the increment-3 working-tree paths
  (implementation, tests and docs). The verifier's only tracked edit was the dated
  [quality gates](../quality/quality-gates.md) block; `tree/state-comparison.txt` records
  the implementation/doc path set unchanged by verification. No commit, push or tag was
  made. This t8 promotion changes only documentation status/evidence text.
- **Evidence scope:** the t7 artifacts are the executable evidence for the v2 contract
  described here. The deferred items in §13 have no implementation or verification
  evidence and must not be cited as implemented.

## Change control

- A change to a byte layout, width, endianness, limit, typed rejection boundary, anchor
  binding rule, selection rule, prefix semantics or CLI field requires an explicit
  amendment to this ADR, updated rejection/acceptance rows and re-verification; it must
  not be made by quietly renaming a test, raising a constant or loosening an assertion.
- Implementation spelling may be refined within the stated contract; the canonical bytes,
  limits, typed categories and semantics may not.
- The decision was promoted from Proposed to Accepted by the owner approval recorded in
  the header and Evidence, and to implemented-and-verified by the t7 executed rows and
  artifacts. A future change to the verified claim requires re-verification.
- v1 and increment-2 behavior is frozen by ADR-0004/ADR-0005. If a v2 change would alter a
  v1 byte, a v1 state hash, an existing CLI default block or an M3-A1–A17 result, stop:
  that is a v1 behavior change and needs its own decision.
- **2026-11-15 (owner approval).** Status changed from Proposed to Accepted on the owner's
  statement "Aprovar ADR-0006 como proposto (Recomendado)" at 2026-11-15T17:06:17-03:00
  (America/Fortaleza), 2026-11-15T20:06:17Z. No contract byte, limit, typed boundary, CLI
  field, acceptance criterion or deferred item changed; no implementation or verification
  is claimed.
- **2026-11-15 (execution promotion, t8).** The independent t7 verification PASS promoted
  the implementation status, callout, §12, Status and Evidence to implemented and verified
  for the deterministic synthetic backend. No contract byte, limit, typed boundary, CLI
  field, acceptance criterion or deferred item changed; the deferred scope in §13 and
  M3's incomplete status are unchanged. The verifier-owned dated gate block was not
  edited here.
