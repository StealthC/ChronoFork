# ADR-0003: Durable Session Storage, Ownership and Metadata

- **Status:** Accepted
- **Date:** 2026-11-14
- **Decision owner:** Project owner (approved 2026-11-14)
- **Approval evidence:** owner accepted every recommended option during the M2 Fase A
  review and authorized a local docs commit (no push); see the owner decisions below
- **Supersedes:** none
- **Related:** [session model](../architecture/session-model.md),
  [backend model](../architecture/backend-model.md),
  [ADR-0001](0001-rust-workspace.md), [ADR-0002](0002-backend-boundaries.md),
  [requirements](../requirements/functional.md)

> **Implementation status.** The decision is accepted; nothing described here is
> implemented. The current tree keeps sessions and checkpoints in memory and exposes
> an explicit byte envelope; it has no storage repository, file layout, lock,
> metadata record, retry, retention or rehydration path. Every behavior below is a
> **Fase B obligation** and still requires a separately approved implementation
> plan before it is built. Do not cite this ADR as evidence that persistence, named
> snapshots or ownership exist.

## Context

ChronoFork can already capture, serialize, restore and fork checkpoints inside one
process. The building blocks this decision must reuse:

- `chronofork-sessions` owns the canonical outer snapshot envelope. `Snapshot::to_bytes`
  writes magic, outer format version, snapshot and origin-session IDs, backend
  identity, tick, configuration and state hashes, payload length, the opaque payload
  and a SHA-256 checksum; `Snapshot::from_bytes` validates all of it.
- `chronofork-backend-api` owns the single compatibility predicate,
  `validate_envelope`, which compares backend ID, exact backend version, payload
  format version and configuration hash.
- `chronofork-core` owns `SessionId`, `SnapshotId`, `StateHash`, `Tick` and the
  shared bounds-checked `codec::{Reader, Writer}`.
- `SessionManager` holds sessions in a `BTreeMap`; `Session` holds its snapshots in
  a `BTreeMap`. Mutations require `&mut`, so in-process ownership is explicit.
- `SessionId` is per-manager: `SessionManager` starts at one and allocates from its
  own counter (`crates/chronofork-sessions/src/manager.rs`). `SnapshotId` is
  per-session: each `Session` starts at one and allocates from its own
  `next_snapshot_id` (`crates/chronofork-sessions/src/session.rs`). Neither ID is
  globally unique across processes.

The tree has no durable storage for session state. The CLI reads a caller-supplied
ROM and writes trace/state output files, but `chronofork-sessions` performs no file
I/O and has no storage abstraction. The roadmap lists durable checkpoints, named
snapshots, a version compatibility policy and a concurrency/ownership policy as unmet
M2 items in [milestones](../roadmap/milestones.md).

## Problem

How should ChronoFork persist sessions and checkpoints across process restarts so
that a reopened checkpoint restores the same state, corruption and incompatible
input are rejected, concurrent writers cannot silently clobber each other, and
storage can never change emulated determinism or the canonical snapshot bytes?

The decision must answer all of the following without inventing a second snapshot
format:

1. On-disk layout and the unit of atomic commit.
2. What is canonical state versus named, non-canonical metadata.
3. How integrity, identity and compatibility are validated exactly once.
4. How partial writes, crashes, orphan directories and stale locks are handled.
5. How readers, writers, retention and deletion interact.
6. How the storage root is confined and bounded.
7. Which dependencies are acceptable.

## Alternatives

A. **Versioned per-snapshot directory with a canonical checkpoint file and a
   separate metadata record** (recommended).
B. **Single-file bundle per snapshot** containing a storage wrapper around the
   canonical envelope and its metadata.
C. **Embedded database** (for example SQLite or an embedded key-value store) with
   state stored as blobs.
D. **Status quo**: in-memory snapshots plus caller-managed byte exports.

| Alternative | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| A. Per-snapshot directory | Canonical `Snapshot::to_bytes` bytes are stored verbatim; checkpoint and metadata have independent versions; a staging directory gives one atomic publish unit; published checkpoints are immutable; future artifacts fit without a format rewrite | More directory entries; depends on directory `fsync`/rename semantics that differ by platform; needs explicit staging/tombstone recovery rules | **Recommended** |
| B. Single-file bundle | One path per snapshot; one rename; easy to copy manually | Requires a second container format and a second reader/writer around the existing envelope, which invites AS-01/AS-03 duplication; metadata and state share one version and checksum; changing metadata rewrites the checkpoint file; no independent versioning; still needs a durable unique naming scheme | Rejected for v1 |
| C. Embedded database | Transactions, indexes and queries | A new dependency with license/maintenance cost; duplicates the snapshot format inside database blobs; harder to inspect and recover by hand; overkill for append-mostly immutable checkpoints; conflicts with the std-only default | Rejected for v1 |
| D. Status quo | No new format or failure modes | Does not survive restart, provides no retention or ownership, and leaves M2 unmet | Rejected as the M2 answer |

Alternative B's decisive problem is the one the anti-slop policy calls out: a
storage wrapper that re-encodes the canonical envelope's fields creates a second
reader/writer and a second compatibility predicate. The whole envelope would have to
be copied into the wrapper or the wrapper would have to be parsed before
`Snapshot::from_bytes` could see the original bytes. Alternative A lets storage hand
the exact checkpoint bytes to the existing reader.

## Decision (accepted)

### 1. Scope and canonical home

This ADR is the canonical home for the durable-storage contract. Architecture,
requirements and roadmap documents link here instead of restating the layout,
compatibility matrix, lock protocol or limits. Fase A changes no code. Fase B must
implement this contract behind an explicit storage adapter; it must not add a second
snapshot serializer, a second snapshot reader, or a second backend compatibility
predicate.

The public names in this ADR (error variants, operations, constants) describe the
contract shape. Fase B may refine spelling, but the semantics and the typed
rejection boundaries are the contract.

### 2. Storage root and versioned layout

The caller supplies an explicit storage root. There is no default global location,
no implicit directory discovery and no hidden state.

- The root is identified by a marker file `chronofork.store` containing the ASCII
  magic `CFSTORE1` and a little-endian `u16` value of the supported
  `STORAGE_LAYOUT_VERSION`.
- Initialization requires an empty directory or an already-initialized root. The
  marker is created exclusively (`create_new`); an unrecognized non-empty directory
  returns a typed `RootNotEmpty` error instead of being adopted or cleaned.
- Opening a root without a marker returns `NotInitialized`. A different magic
  returns `NotAStorageRoot`. An unsupported layout version returns
  `UnsupportedLayoutVersion` with the found and supported values.
- The managed tree has a fixed depth. No operation walks arbitrary depth or follows
  a path supplied by a caller.

```text
<root>/
  chronofork.store                     magic CFSTORE1 + layout version
  v1/                                  storage layout version directory
    state/
      session-keys                     persisted next durable session key
      allocation.lock                  explicit, short-lived key-allocation lock
    sessions/
      session-<key>/
        session.cfmeta                 session metadata record
        writer.lock                    explicit per-session writer lock/lease
        snapshots/
          snapshot-<key>/
            checkpoint.cfsnap          exact Snapshot::to_bytes() bytes
            snapshot.cfmeta            snapshot metadata record
```

Transient entries are named with a reserved prefix and are never treated as
committed data:

- `.staging-<kind>-<key>-<nonce>/` is a publish-in-progress directory.
- `.trash-<kind>-<key>-<nonce>/` is a deleted object awaiting physical removal.
- `*.tmp` files are in-progress replacements inside a published directory.
- `writer.lock.<token>.tmp`, `writer.lock.releasing.<token>`,
  `writer.lock.recovering.<token>` and their `allocation.lock.*` counterparts are
  private lock-protocol records. Only the exact canonical `writer.lock` or
  `allocation.lock` name is ever treated as a lock.

`<key>` is a decimal `u64` with canonical formatting: no sign, no leading zeroes and
no overflow. Session and snapshot keys start at one and are never zero. The fixed
directory names, not user strings, determine paths.

### 3. One canonical checkpoint file per snapshot

`snapshot-<key>/checkpoint.cfsnap` is the only checkpoint state file. Its bytes are
exactly the output of `Snapshot::to_bytes()` for the in-memory checkpoint. Storage
must not prepend, append, reorder, re-hash or re-serialize any part of the envelope.

- The existing `Snapshot::from_bytes` is the only checkpoint reader. Storage calls it
  on a bounded byte slice and propagates `SessionError::InvalidSnapshot` and
  `SessionError::IncompatibleSnapshot` unchanged.
- Storage never computes or rewrites the state hash. The state hash remains the
  backend's canonical digest of the serialized state body; storage only records and
  compares the value it received.
- The envelope's snapshot ID equals the storage snapshot key and the directory
  component. The envelope's origin session ID remains the runtime `SessionId` of the
  process that produced it; it is diagnostic provenance, not a durable key (see
  §8).
- A published `checkpoint.cfsnap` is immutable. Renaming a snapshot, changing its
  name or changing lineage metadata rewrites only `snapshot.cfmeta`. Deleting is an
  explicit operation; there is no in-place checkpoint update and no compaction.
- One snapshot directory contains exactly one checkpoint file. Additional artifact
  kinds would require a new decision and a versioned layout.

### 4. Atomic publish, fsync and crash recovery

Publishing a session or snapshot is a directory rename. The commit point is the
successful rename of the staging directory to its final name; before that point the
object is invisible to list/read operations.

Snapshot publish (under the session writer lock):

1. Reserve the snapshot key and canonical creation sequence in the session record
   (persist before publishing, so a crash leaves a gap, never a reused key).
2. Create the staging directory and write `checkpoint.cfsnap.tmp` exclusively.
3. Write all bytes, `sync_all` the checkpoint file, rename it to
   `checkpoint.cfsnap`, and sync the staging directory.
4. Write `snapshot.cfmeta.tmp`, `sync_all`, rename to `snapshot.cfmeta`, and sync
   the staging directory again.
5. Rename the staging directory to `snapshot-<key>`. This rename is the commit point;
   then sync the `snapshots` parent directory. A failure of that post-commit sync is
   handled as described below.
6. Release the writer lock.

Session creation uses the same pattern with `session.cfmeta` and the `sessions`
parent directory. Metadata-only updates (name or lineage) write a sibling temporary
record and rename it over the published metadata; the checkpoint file is never
touched, and the post-rename parent-sync rule below applies to that rename too.

Recovery rules at open and list:

- A `.staging-*` directory is never loaded. It is reported as an incomplete publish
  diagnostic.
- A `.trash-*` directory is never loaded. It is reported as a pending deletion
  diagnostic.
- A published directory whose checkpoint or metadata record is missing, malformed
  or fails validation is reported as a typed anomaly. It is not repaired, not
  migrated and not deleted automatically.
- A rename that did not reach the parent directory after a power loss yields either
  the complete object or no object, subject to the filesystem's rename and fsync
  guarantees. There is no state in which a half-written checkpoint is visible as
  committed.
- On an in-operation error, the writer must remove only its own staging directory as
  best effort. Failure to remove leaves a diagnosable staging directory; it is not
  treated as success.
- A `sync_all` failure before the commit rename is a typed I/O error: the operation
  aborts before publication. The new object remains invisible; an existing published
  object that a metadata-only replacement would have replaced remains unchanged.
- A `sync_all` failure on the parent directory after the commit rename returns typed
  `DurabilityUnconfirmed`. The object may be visible, but the publish is neither
  durable nor successful. The caller must re-open the object and validate it, and may
  run reconciliation (retry the parent sync and re-validate); it must not blindly
  retry the publish because the key may already exist. `published-but-unconfirmed` is
  an operation outcome, not a persisted marker, so a later list must not be read as
  proof of durability. If the entry disappeared, re-open reports the loss; if a later
  parent sync succeeds, the entry may be treated as confirmed. The writer lock is
  released before the error is returned; reconciliation reacquires it. Storage never
  reports success for an unconfirmed publish.
- On open, under the session writer lock, key and creation-sequence reservations are
  advanced to at least the maximum committed value plus one and the persisted
  reservation. A crash can leave a gap; it can never cause a value to be reused.

The durability claim is limited to local filesystems whose directory `fsync` and
rename behavior is known to be sound. The standard library exposes directory
`sync_all` portably on Unix; Windows and network-filesystem behavior must be
verified in Fase B before the same claim is made there. Until then, those platforms
are documented as weaker or unverified rather than silently equated.

### 5. Metadata records

One versioned metadata record format is used for both record kinds. It is a
storage-level record built on `chronofork_core::codec::{Reader, Writer}`; it is not a
snapshot format and never contains backend state bytes. The record has its own magic,
format version and SHA-256 checksum. Reading requires the exact record length bound,
magic, supported version, checksum and no trailing bytes.

| Field | Kind | Canonical? | Rule |
|---|---|---|---|
| `metadata_version` | both | format | Supported versions are accepted; unknown versions are rejected |
| `record_kind` | both | format | Session or snapshot; a mismatch with the file name is an error |
| `storage_session_key` | both | durable identity | Matches the `session-<key>` directory |
| `storage_snapshot_key` | snapshot | durable identity | Matches the `snapshot-<key>` directory and the envelope snapshot ID |
| `recorded_session_id` | both | diagnostic | Runtime `SessionId` at production time; manager-scoped, never a durable key |
| `recorded_snapshot_id` | snapshot | diagnostic | Runtime `SnapshotId`; must equal the envelope snapshot ID |
| `name` | both | non-canonical label | Optional; bounded UTF-8; no control characters; not used in paths; non-unique in v1 |
| `backend_id`, `backend_version`, `backend_format_version` | snapshot | mirror | Must equal the envelope values exactly; the envelope is authoritative |
| `tick` | snapshot | mirror | Must equal the envelope value exactly |
| `config_hash` | snapshot | mirror | Must equal the envelope value exactly |
| `state_hash` | snapshot | mirror | Must equal the envelope value exactly |
| `checkpoint_length`, `checkpoint_sha256` | snapshot | binding | Byte length and SHA-256 of `checkpoint.cfsnap` |
| `parent_session_key` | both | lineage | Optional durable parent session key |
| `origin_snapshot_key` | both | lineage | Optional durable origin snapshot key |
| `creation_sequence` | snapshot | canonical order | Per-session logical counter assigned under the session lock; used to order listings |
| `next_snapshot_key` | session | reservation | Persisted before a snapshot publish so keys are never reused |
| `created_at_unix_ms` | both | advisory | Optional host timestamp; may be absent or wrong; never canonical |

The mirrored fields exist so a caller can list and diagnose a checkpoint without
parsing the backend payload and so a swapped file is detectable. They are not a
second compatibility predicate: storage verifies exact equality against the values
returned by `Snapshot::from_bytes`, and any mismatch is a typed `MetadataMismatch`
rejection. Compatibility decisions still come from `Snapshot::from_bytes` and
`validate_envelope` only.

`creation_sequence` is logical, not wall-clock. It is allocated from a persisted
per-session counter while the writer lock is held; gaps are allowed after a crash,
but a value is never reused. Listing orders snapshots by `creation_sequence`, then by
snapshot key for the impossible-but-checked tie. Host timestamps and filesystem
modification times are never used for identity, ordering, equality, compatibility,
retention or replay. They are never part of a checkpoint's bytes or state hash, so
replaying or comparing the same state produces the same result regardless of when it
was written.

### 6. Validation order and typed rejection

Storage validates before it exposes or acts on an object, in this order:

1. Root marker, layout version and canonical path shape.
2. Metadata file size bound, checksum, magic, record version and record kind.
3. Checkpoint file size bound and SHA-256 binding against the metadata record.
4. `Snapshot::from_bytes` for the canonical envelope (magic, outer version,
   checksum, trailing bytes, field decoding).
5. Identity binding: directory keys, metadata keys, envelope snapshot ID and
   envelope origin-session ID agree.
6. Backend compatibility via the existing `validate_envelope` on restore or
   rehydration, never from this storage layer.

Every failure is a typed error with a specific category. Error categories:

| Category | Examples | Contract |
|---|---|---|
| Root/layout | `NotInitialized`, `NotAStorageRoot`, `RootNotEmpty`, `UnsupportedLayoutVersion` | No auto-creation, no auto-migration |
| Metadata | `CorruptMetadata`, `UnsupportedMetadataVersion`, `MetadataMismatch` | No reinterpretation, no repair |
| Checkpoint | `SessionError::InvalidSnapshot`, `SessionError::IncompatibleSnapshot` | Wrapped unchanged; not flattened to strings |
| Identity | `IdentityMismatch`, `SessionConflict`, `SnapshotExists`, `UnknownSession`, `UnknownSnapshot` | No overwrite, no silent merge |
| Bounds | `FileTooLarge`, `ListLimitExceeded`, `InvalidName` | Rejected before allocation or truncation |
| Confinement | `PathEscape`, `SymlinkRejected`, `WrongFileType` | No traversal, no following |
| Concurrency | `WriterContended`, `NotLockOwner`, `CorruptLock`, `LockPrimitiveUnsupported` | Explicit contention; an unreadable lock never grants access; no busy-wait or silent retry |
| Durability | `DurabilityUnconfirmed` | A visibility or durability outcome is unconfirmed; never reported as success |
| Allocation | `CorruptAllocationCounter` | Fail closed; never reset to one automatically |
| I/O | `Io { operation, path, source }` | Operation and path context preserved |

A corrupt metadata record never authorizes a fallback that guesses the missing
fields from a directory name. A missing metadata record never causes the checkpoint
to be loaded as an unnamed snapshot. A dangling lineage key is reported, not
resolved by search.

### 7. Compatibility policy and matrix

Compatibility is explicit and reject-only. There is no silent migration, no
rewrite-on-open, and no best-effort reinterpretation of a future version.

| Component | Accepted in layout v1 | Check owner | On mismatch |
|---|---|---|---|
| Root marker and storage layout version | `CFSTORE1`, exact supported value of `STORAGE_LAYOUT_VERSION` | Storage | `UnsupportedLayoutVersion`; leave the tree untouched |
| Metadata record version | exact supported value of `STORAGE_METADATA_FORMAT_VERSION` | Storage | `UnsupportedMetadataVersion`; do not guess |
| Canonical outer envelope version | exact supported value of `SNAPSHOT_FORMAT_VERSION` | `Snapshot::from_bytes` | Existing `SessionError::IncompatibleSnapshot` |
| Backend snapshot envelope | `BACKEND_SNAPSHOT_FORMAT_VERSION`, exact `backend_id`, exact `backend_version`, exact `config_hash` | `validate_envelope` | Existing typed `BackendError::IncompatibleSnapshot`, translated once at the session boundary |
| Backend payload | Backend-owned payload format version | Backend `restore` | Backend's typed error; storage never inspects the payload |
| Runtime session/snapshot IDs | Not a compatibility field | Storage mapping | Recorded as diagnostics; never used as durable identity |

A format change requires all of: a version bump, updated rejection tests, an entry
in this matrix and the session model, updated requirements if user-visible, and
verifier evidence. A migration requires a separate explicit decision and an
operator-invoked tool; it must never run implicitly on open. Layout `v1` has no
migration and no compatibility with future layout directories.

Backend versions are compared exactly as `validate_envelope` already does. Storage
must not add a semver range, a fallback to the nearest version, or a second
backend-ID comparison.

### 8. Identity, lineage and rehydration

Two identity layers are kept separate:

- **Storage identity** is durable and belongs to this storage format. A session has a
  `storage_session_key`; a snapshot has a `(storage_session_key, storage_snapshot_key)`
  pair. Keys are minted under explicit locks, persisted before publication and never
  reused. Session keys are minted from `state/session-keys` under the short-lived
  `allocation.lock`; snapshot keys are reserved in the session record under the
  session writer lock.
- **Runtime identity** is the existing `SessionId`/`SnapshotId`. `SessionManager`
  allocates session IDs per manager; each `Session` allocates snapshot IDs per
  session from its own `next_snapshot_id`
  (`crates/chronofork-sessions/src/session.rs`). Both reset across processes and
  neither is a durable key. Rehydration maps storage keys to fresh runtime handles,
  seeds the manager's session allocator and each session's snapshot allocator, so
  newly captured snapshots cannot collide with imported ones.

Snapshot IDs are aligned: the envelope snapshot ID, the storage snapshot key and
the runtime `SnapshotId` for that checkpoint are the same value. Session IDs are not
aligned: the envelope records the producing runtime session ID for diagnostics,
while the storage session key is the durable directory identity. The envelope is
never rewritten to hide that difference.

Lineage metadata records the parent storage session key and origin storage snapshot
key. A fork rehydrates the origin checkpoint and then publishes new checkpoints
under the child's own storage session key; it does not copy mutable state between
sessions and does not duplicate the parent's checkpoint. Deleting a parent or origin
does not rewrite child metadata: dangling references are reported by diagnostics and
are never auto-repaired. Cross-session provenance does not relax the isolation
invariants in the [session model](../architecture/session-model.md).

### 9. Listing, retention and deletion

There is no automatic garbage collection in v1. Opening a root, capturing a
checkpoint, closing a session and process exit never delete committed data, another
operation's uncommitted data, tombstones or unreferenced checkpoints. A failed publish
may remove only its own staging directory, as described in §4.

- **List** returns only entries that pass the full validation order. It is
  deterministic: sessions by storage session key, snapshots by `creation_sequence`
  then key. Entries that fail validation are reported as typed anomalies, not
  silently skipped and not repaired. A list scan is bounded by a configured entry
  cap.
- **Retention is caller-driven.** A caller lists snapshots, applies its own policy
  (age by logical sequence, named set, keep-last, and so on) and calls delete
  explicitly. Named snapshots are not automatically protected by the storage layer;
  the caller's policy decides.
- **Delete** is explicit, requires the session writer lock, validates the target,
  renames the object to `.trash-<kind>-<key>-<nonce>` (the visibility-removing
  commit point), syncs the parent directory, then removes the tree. A post-rename
  parent-sync failure returns `DurabilityUnconfirmed`: the caller must re-list and
  validate, because a crashed delete may reappear. A crash leaves a tombstone that is
  reported but never resurrected. Deleting a snapshot does not cascade to children or
  rewrite lineage. Deleting a session requires its snapshots to be removed
  explicitly first; the empty session directory is then removed explicitly.
- **No overwrite.** Publishing an existing session or snapshot key returns a typed
  conflict. Replacing a checkpoint in place is not a v1 operation.

A future automatic retention or garbage collector needs its own decision, explicit
opt-in and tests; it is not part of M2.

### 10. Ownership and concurrency

The in-process rules do not change: mutation requires `&mut SessionManager` or
`&mut Session`, and a session has one mutable owner at a time. Durable storage adds
cross-process ownership at the filesystem boundary.

| Scope | Mechanism | Contention result |
|---|---|---|
| In-process session | Existing `&mut` borrow and `SessionManager` ownership | Rust compile-time error; no silent sharing |
| Cross-process session writer | `writer.lock` published atomically from a fully written private record with `hard_link`; release/recovery use the owner-checked rename-aside protocol below | `WriterContended` naming the observed owner; an empty, partial or unreadable lock is still `WriterContended`, never `CorruptLock`; no busy-wait and no silent retry |
| Snapshot publish | Under the per-session writer lock; checkpoint files are immutable after commit | The session lock serializes publishes; no second writer can modify a committed checkpoint |
| Session-key minting | `state/allocation.lock` uses the same publication/recovery protocol and is held only for the key reservation and persisted-counter update | Typed contention; never held during emulation, serialization or checkpoint I/O |
| Readers | No lock; atomic directory rename and checksum validation | A reader sees a complete old object, a complete new object, or a typed `NotFound`; never a torn file |

`std` has no portable compare-and-swap on file contents. Acquisition is an atomic
create-if-absent through `hard_link`; release and recovery are cooperative, not CAS.
They prevent silent success and protect a successor's record, but a process that
ignores the protocol can still interfere. The lock record is a separate, bounded,
versioned storage record with one reader and one writer built on the shared codec; it
is not a snapshot format. It contains the owner token, generation, process ID, host
label, acquisition host timestamp and an advisory lease deadline. The generation is a
per-record `u64` chosen at acquisition; owner token plus generation is the expected
pair, and neither value is derived from host time or used for emulation determinism.
The token used in a private filename is generated by storage from a bounded safe
alphabet; it is never a caller-supplied path component. It is cooperative ownership
metadata, not authentication and not a security boundary. The writer lock is acquired
for each mutating storage operation and released afterward; a long-lived writer handle
is an explicit option, never hidden state.

**Writer-lock publication.** `writer.lock` is never created by
`create_new`-then-write. Acquisition:

1. Create a private record `writer.lock.<owner-token>.tmp` exclusively in the session
   directory; write the complete record, `sync_all`, and close it. A private-name
   collision uses a fresh token with a bounded retry; after those retries it returns
   `Io`, and it never overwrites a record.
2. `hard_link` that record to `writer.lock`. The link is the atomic create-if-absent
   claim; the canonical name is never visible with partial content.
3. On success, the lock is acquired. Remove the temp name best effort; a leftover temp
   is a diagnostic, never a lock, and cleanup failure does not revoke acquisition.
4. On `AlreadyExists`, inspect `writer.lock` with no-follow metadata, then read it
   with the lock-record size bound. A symlink returns `SymlinkRejected`; any other
   non-regular file returns `WrongFileType`; neither is read or removed. If the
   canonical name disappeared before the read, retry acquisition from step 1 once; a
   second disappearance returns `WriterContended` with an unreadable owner. Otherwise:
   - a complete supported record returns `WriterContended` with the observed
     owner/generation;
   - an empty, partial, unsupported or otherwise unreadable record returns
     `WriterContended` with an unreadable owner. Occupancy is authoritative: an
     unreadable record never grants access and is never reported as `CorruptLock`
     during acquisition.
5. On every `AlreadyExists` outcome, remove this acquisition's temp best effort after
   classification; a leftover is a diagnostic and cleanup failure does not change the
   contention result.
6. Any other I/O failure returns `Io` and removes only this acquisition's temp. If the
   filesystem does not support hard links, return `LockPrimitiveUnsupported` and fail
   closed; do not fall back to `create_new`-then-write.

**Writer-lock release and recovery.** Release takes the expected owner token and
generation from the writer handle; recovery additionally requires explicit operator
confirmation that no writer is live. Both use the same protocol:

1. Inspect `writer.lock` with no-follow metadata and read it. A symlink is never
   followed and is never removed: release returns `NotLockOwner`, recovery returns
   `CorruptLock`. If the record is missing or its owner/generation does not match the
   expected pair, return `NotLockOwner` and remove nothing. If it is otherwise
   unreadable, release returns `NotLockOwner` and recovery returns `CorruptLock`;
   both remove nothing.
2. Rename `writer.lock` to a unique private
   `writer.lock.releasing.<token>` / `writer.lock.recovering.<token>`. This one
   directory operation takes the canonical name out of service without deleting data.
3. Re-read the renamed record. If owner/generation match, delete that private record
   and return success. If they do not match, a successor replaced the lock between
   steps 1 and 2: attempt to restore the successor with `hard_link` to `writer.lock`;
   on restore success remove the private name, which leaves the canonical successor
   record intact. Delete no canonical lock record and return `NotLockOwner`. If
   restore fails because a new lock already exists, leave the moved record as a
   diagnostic; the live successor was not deleted by this actor.
4. A crash between rename and delete leaves a private `.releasing`/`.recovering`
   record and no canonical lock. A successor may acquire; the leftover is reported as
   an anomaly, is never treated as a live lock, and is not reclaimed automatically.

Stale-lock policy:

- An expired lease is a diagnostic, not an authorization to take the lock. Acquisition
  never steals.
- Writer-lock recovery requires the observed owner and generation, explicit operator
  confirmation that no writer is live, and the protocol above. A changed record
  returns `NotLockOwner`; no lock record is deleted. If the observed lease has not
  expired, recovery returns `WriterContended` unless the operator explicitly requests
  forced recovery; forcing still requires the owner/generation check and the
  rename-aside protocol. Recovery never runs concurrently with a live writer;
  liveness is a cooperative operator assertion because `std` cannot prove it.
- An empty, partial or unreadable `writer.lock` is `WriterContended` for acquisition.
  That classification applies to acquisition reads; an explicit recovery read of the
  same unreadable record returns `CorruptLock` and removes nothing. The operator must
  confirm no writer and remove the lock out of band. Storage never deletes an
  unreadable lock.

**Session-key allocation lock and recovery.** `state/allocation.lock` uses the same
publication, release and rename-aside recovery protocol. Its critical section is only:
read `state/session-keys`, persist the incremented 8-byte little-endian counter
(tmp+fsync+rename and parent sync), then release; it is never held during checkpoint
I/O. The allocation lock is never stolen, and its explicit recovery requires the same
observed owner/generation check and confirmation that no allocator is live. Recovery
clears the lock only; it does not change the counter.

- A missing, short, oversized or unreadable `state/session-keys` returns
  `CorruptAllocationCounter` and fails closed. It is never reset to one automatically,
  because that could reuse a key already referenced by a published session or lineage.
  Explicit repair must establish the highest published key and write a greater
  counter.
- The counter update must be durably committed before a reserved key is used to
  publish a session. If that update is unconfirmed, return `DurabilityUnconfirmed`
  and publish nothing. On re-open, read the counter: if it advanced past the reserved
  key, that key is a gap and is never reused; if it still holds the old value, the key
  was never reserved and may be minted again, because no published object referenced
  it.
- An empty, partial or unreadable `allocation.lock` follows the writer-lock rule:
  acquisition returns `WriterContended`, explicit recovery returns `CorruptLock` and
  removes nothing.

There is no hidden global lock. The only root-level serialization is the explicitly
documented, short-lived key-allocation lock. Distinct sessions can be written
concurrently; one session has one writer at a time; a checkpoint has one writer for
its entire published lifetime.

### 11. Security and resource bounds

Storage is confined to the explicit root and treats all input as untrusted.

- **Confinement.** All managed paths are built from fixed literal components and
  canonical numeric keys. No caller-supplied string becomes a path component. The
  root is canonicalized once at open; every managed path must stay beneath it.
  Components equal to `.` or `..`, or containing a path separator, NUL or
  non-canonical numeric text, are rejected with `PathEscape` before any filesystem
  call.
- **File types and links.** Managed paths must be regular files or directories.
  Symlinks are rejected for managed components and are never followed during
  traversal. Windows reparse-point behavior must be verified in Fase B; std-only
  detection is not claimed to be complete.
- **Bounded reads.** File length is read from metadata and checked against a limit
  before any allocation or full read. Reads are additionally capped with a bounded
  reader (`take(limit + 1)` pattern) so a file that grows after the length check is
  rejected with `FileTooLarge` rather than allocated without bound. A declared
  payload or metadata length is validated before it is used.
- **Limits.** Default limits are applied per operation and may be lowered by
  configuration; raising them requires an explicit documentation and evidence
  update. No path is read without a limit.

| Limit | Default | Applies to |
|---|---|---|
| `MAX_CHECKPOINT_BYTES` | 64 MiB | `checkpoint.cfsnap` |
| `MAX_METADATA_BYTES` | 64 KiB | `session.cfmeta`, `snapshot.cfmeta` |
| `MAX_LOCK_BYTES` | 4 KiB | `writer.lock` |
| `MAX_NAME_BYTES` | 256 UTF-8 bytes | session and snapshot names |
| `MAX_LIST_ENTRIES` | 100,000 | one list operation |

- **Permissions and trust.** Storage relies on the host filesystem's permissions and
  ACLs. It does not authenticate users, encrypt checkpoints, redact emulated memory
  or provide multi-tenant isolation. The ownership lock is cooperative and can be
  bypassed by a process that ignores it. A shared root is supported only when the
  operating system's permissions already enforce the intended boundary.
- **No network path.** Storage performs no upload, telemetry or network access.
  Checkpoint contents are emulated machine state and may be sensitive; the operator
  chooses and protects the root.
- **No `unsafe`.** The storage implementation must obey the workspace's
  `unsafe_code = "forbid"` lint and use no FFI.

### 12. Dependencies

New functionality uses the standard library only: `std::fs`, `std::io`, `std::path`
and `std::time`, with the already-approved `sha2` reached through `chronofork-core`
for SHA-256. No new crate is approved for layout v1. In particular, do not add
`serde`, a JSON/binary framework,
`tempfile`, `fs2`/file-lock crates, `walkdir`, `rand` or `chrono` to obtain this
contract.

Any future dependency must pass [AS-11](../quality/anti-slop-policy.md), have a
license compatible with the project license and `THIRD_PARTY_NOTICES.md`, show
active maintenance, and be approved by the project owner in an ADR amendment. A
crate must not be added merely to replace a small amount of std code. Platform
specific lock or directory-sync behavior is documented as a limitation until it is
implemented and tested, not papered over with an unapproved dependency.

### 13. Verification required in Fase B

These are planned Fase B gates, not current results:

- round-trip: capture, restart, reopen, restore and compare tick and state hash;
- crash/fault injection at each publish step, proving staging is never visible as
  committed and no partial object is loaded;
- a post-rename parent-sync failure returns `DurabilityUnconfirmed`, and
  reconciliation distinguishes visible-confirmed, visible-unconfirmed and lost;
- corruption, truncation, wrong checksum, swapped checkpoint, wrong directory key and
  metadata mismatch all produce typed errors;
- the compatibility matrix, including future/unknown versions and exact backend
  version rejection;
- lock-record publication with a crash between publish and temp cleanup; empty,
  partial and unreadable locks classified as contention on acquisition; explicit
  recovery outcomes; release/recovery against a changed successor; forced recovery
  with the owner/generation check;
- allocation-lock recovery, a corrupt or missing `state/session-keys`, counter gaps
  and the no-reuse rule;
- two-writer contention, stale/corrupt lock recovery and owner mismatch;
- path traversal, symlinks, wrong file types, oversized files and list caps;
- listing order, explicit delete, tombstone/crash behavior and proof that no
  operation performs automatic GC;
- metadata and host timestamps changing without changing checkpoint bytes, state
  hash or replay result;
- rehydration through the existing `Snapshot::from_bytes` and `validate_envelope`
  paths, with no second reader or compatibility predicate.

### 14. Owner decisions (approved 2026-11-14)

The project owner approved the following during the M2 Fase A review:

1. The versioned per-snapshot directory layout and the canonical checkpoint file as
   the unit of durable state.
2. Reject-only compatibility with no silent migration, and manual list/delete
   retention with no automatic garbage collection in v1.
3. The per-session writer lock/lease with hard-link publication and owner-checked
   rename-aside recovery, manual stale-lock recovery, and the explicitly bounded root
   key-allocation lock with its matching recovery protocol.
4. Host timestamps as advisory metadata only: never in checkpoint bytes, state hash,
   ordering, equality, compatibility or replay.
5. The default resource limits and the local-filesystem durability scope.
6. The minimal CLI surface `chronofork session save/load/list/delete` documented in
   the [M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md), as the
   contract shape for Fase B; it is not implemented.
7. Fase B implementation planning under a separately approved plan, after this ADR's
   acceptance.

## Rationale

- **Canonical bytes stay canonical.** Storing `Snapshot::to_bytes()` verbatim means
  the existing envelope, checksum, version and compatibility tests remain the only
  authority. Storage cannot drift from the snapshot contract because it never
  rewrites it. This directly satisfies AS-01 and AS-03.
- **A directory rename gives one commit point for a pair of files.** The checkpoint
  and its metadata must be validated together; publishing them through one staging
  directory avoids a visible state with only half the pair. Storing both in a single
  wrapper file would require a second format and reader, which is the larger risk.
- **Immutability simplifies ownership.** Published checkpoints never change, so a
  reader needs no lock and a writer only needs to serialize publication. Metadata
  updates are small atomic replacements and cannot affect state hashes.
- **Logical sequence versus host time.** Determinism and reproducible ordering need a
  logical counter. Host timestamps are useful for humans but are nondeterministic,
  can go backwards and can be absent; keeping them advisory preserves both utility
  and determinism.
- **Reject-only compatibility is honest.** Silent migration makes a restore depend on
  code that was never tested against the old bytes. An explicit refusal is cheaper to
  verify and cannot corrupt a session.
- **Explicit locks and deletion avoid surprising data loss.** A hidden global lock
  would serialize unrelated sessions and a background GC could delete data the user
  still needs. Per-session locks, caller-driven retention and manual stale-lock
  recovery keep ownership visible.
- **Std-only keeps the trust surface small.** The operation is filesystem metadata
  handling with an existing hash implementation; adding a database or serialization
  framework would add license, maintenance and failure surface without solving a
  demonstrated problem.

## Consequences

Positive:

- Checkpoints can survive restarts without changing the canonical snapshot bytes or
  the backend compatibility contract.
- Corruption, incompatible versions and identity mismatches fail closed with typed
  errors.
- Sessions can be written concurrently across processes without a hidden global
  lock.
- Named metadata and lineage are durable without contaminating determinism.
- Retention and deletion are predictable and auditable.

Negative / costs:

- More filesystem objects and more recovery cases than a single file or a database.
- Directory `fsync` and rename semantics need platform-specific verification;
  Windows durability is not claimed until tested.
- Rehydration needs explicit runtime-ID mapping and allocator seeding.
- Stale locks require operator action; a crashed writer can block a session until
  recovery, and a crash during key minting can likewise leave the allocation lock
  requiring the explicit owner-checked recovery in §10.
- Metadata duplicates a few envelope fields for lookup and binding; a mismatch is a
  rejection, so the writer must keep them consistent.
- Disk usage grows until explicit deletion; there is no compaction or deduplication.

Risks / limitations:

- The lock is cooperative, not a security control; a hostile local process can
  bypass it. Hard-link publication fails closed on filesystems without hard links, so
  those filesystems cannot provide the writer lock.
- Storage does not provide encryption, authentication, multi-tenant isolation or
  remote replication.
- Network filesystems are out of scope for the durability claim.
- The validation path must remain one implementation; a future "fast path" that
  parses checkpoint fields directly would violate AS-01.
- Named snapshots are non-unique labels in v1; name lookup is not an identity
  authority.
- This ADR is Accepted: acceptance does not assert that any test, storage adapter or
  rehydration path exists.

## Status

Accepted. The project owner approved this decision and the recorded options on
2026-11-14. The durable-storage behavior described here must not be represented as
existing capability until Fase B implements and verifies it. Fase B proceeds only
under a separately approved implementation plan.

## Evidence

- Current in-memory behavior and envelope:
  `crates/chronofork-sessions/src/snapshot.rs` (`Snapshot::to_bytes`,
  `Snapshot::from_bytes`, `SNAPSHOT_FORMAT_VERSION`),
  `crates/chronofork-sessions/src/session.rs` (`snapshots: BTreeMap`),
  `crates/chronofork-sessions/src/manager.rs` (`SessionManager` registry and
  allocators), `crates/chronofork-sessions/src/error.rs` (typed session errors).
- Single compatibility predicate: `crates/chronofork-backend-api/src/snapshot.rs`
  (`validate_envelope`, `BACKEND_SNAPSHOT_FORMAT_VERSION`).
- Shared bounded codec: `crates/chronofork-core/src/codec.rs`.
- Opaque u64 identifiers and allocation scopes:
  `crates/chronofork-core/src/id.rs`; `SessionManager` allocates session IDs in
  `crates/chronofork-sessions/src/manager.rs`; each `Session` allocates snapshot IDs
  from `next_snapshot_id` in `crates/chronofork-sessions/src/session.rs`.
- Existing envelope tests in `crates/chronofork-sessions/src/snapshot.rs` cover
  round-trip, corruption, unsupported outer version, truncation and arbitrary-byte
  rejection; `crates/chronofork-sessions/tests/snapshot_properties.rs` adds property
  coverage. Those tests prove the envelope, not durable storage.
- No durable storage exists: `crates/chronofork-sessions` has no filesystem I/O.
  The CLI reads a caller-supplied ROM (`run_megadrive_trace` in
  `crates/chronofork-cli/src/main.rs`) and writes instruction-trace and final-state
  output files; those are trace/state export paths, not session persistence.
- M2 remains unchecked in [milestones](../roadmap/milestones.md).
- Design constraints cited: AS-01, AS-03, AS-11, AS-12 and AS-13 in the
  [anti-slop policy](../quality/anti-slop-policy.md).
- Reviewed against the tree at commit `31293b5` (2026-11-14). This ADR changes no
  code and no gate result; it records an accepted design that is not implemented.
