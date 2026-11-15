# Session Model

## What a session is

A session is one independent emulated machine plus its lifecycle, checkpoints and
fork lineage. It owns exactly one mutable backend instance. Session identity is
allocated by the host and is not a backend concept.

```text
Session
  id: SessionId
  backend: Box<dyn Backend>     ← sole mutable hardware owner
  state: Ready | Paused | Closed
  lineage: Lineage
  initial_state: BackendSnapshot
  snapshots: SnapshotId → Snapshot
```

## Lifecycle

| Operation | Semantics |
|---|---|
| `create` | Fresh backend, capture initial snapshot, root lineage |
| `step` | Advance one backend-defined quantum; only in `Ready` |
| `pause` | Reject further `step`; inspection/checkpoint still allowed |
| `resume` | Permit `step` again |
| `reset` | Restore the creation snapshot |
| `snapshot` | Capture current state, store with a new ID |
| `restore` | Replace current state with a stored checkpoint |
| `fork_from_snapshot` | New session restored from a checkpoint |
| `fork_current` | Snapshot current state, then fork |
| `close` | Reject further operations; idempotent |

The bootstrap API is synchronous. `Ready` means execution is permitted, not that a
thread is running. A future scheduler can wrap this API but must preserve the same
state transitions.

## Snapshot ownership and lineage

A snapshot belongs to the session that created it. When a session forks from a
snapshot, the child **adopts** that origin snapshot so it can restore to its own
starting point. The child's lineage records:

- `parent_session`: source session;
- `origin_snapshot`: checkpoint used;
- `depth`: parent depth + 1.

A snapshot imported from bytes creates a root session whose lineage records the
origin snapshot but has no local parent.

Session IDs are per manager; snapshot IDs are per session and allocated by each
`Session` from its own `next_snapshot_id`
(`crates/chronofork-sessions/src/session.rs`). The outer serialized envelope carries
the origin session and snapshot ID for diagnostics, not as a global identity
guarantee.

## Persistence and durable identity (implemented; Linux acceptance platform)

> Status: implemented by `SnapshotRepository` in `chronofork-sessions` and
> independently verified in two rounds (2026-11-15). Round 1 found one medium,
> non-blocking CLI lock-scope finding (F1); the repair made the continue-and-save
> workflow hold the writer lock across read-validate-publish, and round 2 re-verified
> F1 closed with no regression. The authoritative contract, layout and limits are in
> [ADR-0003](../adr/0003-durable-session-storage.md); the execution evidence is in the
> [M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md) and [quality
> gates](../quality/quality-gates.md).

The durable contract treats a persisted checkpoint as the existing canonical
`Snapshot::to_bytes()` envelope stored verbatim. Storage defines no second snapshot
serializer, reader or compatibility predicate: it hands bounded bytes to
`Snapshot::from_bytes` and lets the existing backend restore path validate
compatibility.

Durable storage has an identity that outlives one process. Runtime `SessionId`
remains manager-scoped, while `SnapshotId` remains session-scoped; neither is a global
key, so storage adds durable keys minted under explicit locks. A rehydrated process
maps those keys to fresh runtime handles and seeds the session and snapshot allocators;
a snapshot's storage key is aligned with the envelope snapshot ID, while the envelope's
origin-session ID stays diagnostic provenance. The storage identity, layout and
allocation rules live in [ADR-0003](../adr/0003-durable-session-storage.md); this
document does not restate them.

What survives a restart:

- the canonical checkpoint envelope bytes;
- the versioned session and snapshot metadata records;
- durable lineage keys and the logical creation sequence;
- the backend identity required to select a compatible backend.

What does not survive as canonical state:

- the live mutable backend instance (it is reconstructed by restoring a checkpoint);
- the in-process session registry and runtime identifiers;
- writer locks and leases;
- advisory host timestamps or filesystem modification times.

Rehydration is a validate-then-register operation: list only valid objects, restore a
canonical checkpoint into a fresh backend, and register the session only after the
restore succeeds. An incompatible or corrupt checkpoint returns a typed error and
leaves no half-registered session. The isolation invariants below are unchanged by
persistence; storage is not a mutation channel between sessions.

Persistence invariants (verified on the Linux acceptance platform):

1. Restoring a committed checkpoint after a restart yields the same tick and state
   hash as before the restart.
2. Metadata names and host timestamps never change checkpoint bytes, the state hash
   or replay results.
3. A crash during publish never exposes a partially written snapshot as committed; a
   post-rename parent-sync failure is reported as `DurabilityUnconfirmed`.
4. An unsupported storage, metadata, outer-envelope or backend version is rejected
   before any restore.
5. Deleting one snapshot never rewrites another session's checkpoint or lineage.
6. Writer contention produces a typed error; the supported read-modify-publish
   workflow (`with_writer_lock` plus `save_snapshot_with_lock`, used by the CLI) holds
   one writer lock across the read, validate and publish sequence.

## Snapshot metadata (implemented)

> Status: implemented and verified (M2-A2, M2-A11); evidence as above.

The metadata record adds named, durable information without changing the canonical
checkpoint. Its authoritative fields, encoding and validation order are in
[ADR-0003](../adr/0003-durable-session-storage.md); this document keeps only the
session-level consequences. Durable metadata preserves the durable session/snapshot
identity and lineage; runtime IDs are diagnostics, an optional name is a label rather
than an identity authority, and host timestamps are advisory and excluded from hashing,
equality, ordering and replay. The logical creation sequence, not host time, orders
snapshots. Renaming or re-annotating a checkpoint rewrites only its metadata record;
the checkpoint file remains immutable (verified).

## Storage ownership and concurrency (implemented; cooperative lock)

> Status: implemented and verified (M2-A5, M2-A9); the round-1 CLI lock-scope finding
> F1 is closed in round 2.

The in-process rules in the Concurrency rules section remain in force. Across
processes, a durable session has one writer at a time; the lock/lease protocol and its
typed outcomes are specified only in
[ADR-0003](../adr/0003-durable-session-storage.md). `SnapshotRepository::with_writer_lock`
provides the supported read-modify-publish workflow: the caller's closure runs while
one lock is held and the lock is released on every success and error path;
`save_snapshot_with_lock` rejects a released or foreign-store handle with a typed
`NotLockOwner`. The `chronofork session save --session` CLI path uses that workflow and
therefore fails at acquisition with a typed `WriterContended` before reading a stale
base. The convenience `save_snapshot` still acquires its own lock only at publish time,
so a third-party caller that reads a base before calling it can reproduce the older
stale-base window; the CLI no longer does this. Readers take no writer lock and rely on
the immutable checkpoint and the atomic publication rule, so they do not observe a
partially published checkpoint. The tested lock-free-reader scope is sequential: a
reader loads complete canonical bytes while a writer lock is held (no publish in
progress), and independent readers are side-effect-free. A concurrent publish/read or
delete/read interleaving is not separately tested and remains a **Partial**
`NFR-CONC-003` clause resting on the immutable-object/atomic-rename argument.
Persistence is not a mutation channel between sessions: it cannot relax the isolation
invariants, and a session with an active writer must not be mutated through a second
storage handle.

## Storage limitations (remaining)

The canonical storage limitations, filesystem scope, durability bounds, size limits,
confidentiality and retention policy are in
[ADR-0003](../adr/0003-durable-session-storage.md). For the session model the
consequences are:

- a durable session can be blocked by a stale writer lock until explicit
  owner/generation-checked recovery; the lock is cooperative metadata, not
  authentication;
- `save --session` holds the writer lock across rehydration and any in-memory
  `--steps` advance, so a long advance lengthens contention for other writers (a
  deliberate correctness trade-off);
- a committed checkpoint is restorable only while its storage root, metadata record
  and a compatible backend are available;
- deleting an origin checkpoint can leave child lineage dangling; the child cannot
  restore to that origin unless a future owner decision copies or pins it;
- persistence adds no cross-session sharing, authentication or access control, and
  there is no encryption, compression, deduplication, migration or automatic garbage
  collection;
- durability is verified only on local Linux filesystems; native Windows and network
  filesystem durability is unverified (ADR-0003 §4).

## Isolation invariants

These are executable requirements, not aspirations:

1. A fork starts with the same tick and state hash as its source checkpoint.
2. Stepping one session never changes another session's state hash.
3. Writing debugger memory in one session never changes another session's memory.
4. Restoring a checkpoint replaces the intended machine state.
5. Replaying the same inputs from the same checkpoint reproduces the state hash.
6. Closing or pausing a session cannot corrupt another session.
7. An incompatible snapshot is rejected before mutating the destination.

The acceptance suite in `crates/chronofork-sessions/tests/acceptance.rs` covers
invariants 1–6 directly. Snapshot corruption and version rejection are covered by
unit tests in `crates/chronofork-sessions/src/snapshot.rs`.

## Concurrency rules

The bootstrap exposes mutations through `&mut SessionManager` and `&mut Session`.
There is no hidden global lock or background thread. This is a deliberate
correctness choice.

For transports and durable storage:

- One session has one mutable owner at a time.
- Concurrent reads may be allowed only if the backend guarantees side-effect-free
  inspection.
- A shared session must use an explicit synchronization policy (for example a
  per-session mutex) at the transport/service layer.
- Silent races are forbidden. A lock timeout or contention must be a typed error.
- Durable repository ownership is implemented as a cooperative per-session writer
  lock in `chronofork-sessions`; readers are lock-free and never observe a torn
  checkpoint. The protocol and its limits are in
  [ADR-0003](../adr/0003-durable-session-storage.md).
- Forking is the preferred way to run independent experiments; sharing one mutable
  session is not a substitute for isolation.

## Error model

`SessionError` distinguishes:

- unknown session/snapshot;
- closed session;
- operation not allowed in the current state;
- backend halt;
- identifier exhaustion;
- malformed or incompatible serialized snapshot;
- backend failure;
- durable storage failure: uninitialized or foreign root, unsupported layout or
  metadata version, corrupt metadata, metadata/identity mismatch, unknown durable
  session/snapshot, non-empty session, invalid name, path escape, symlink or wrong
  file type, file/list bounds, writer contention, not lock owner, corrupt lock,
  unsupported lock primitive, durability unconfirmed and corrupt allocation counter.

Operations validate before mutating. A failed restore must not leave a session
half-restored; the backend contract requires atomic rejection of incompatible
snapshots. This invariant needs dedicated backend-specific tests when a real
backend is adopted.

## Limitations

- No background execution, run loop, frame pacing or wall-clock scheduling.
- No copy-on-write or memory-page sharing between forks.
- Durable storage, named metadata, storage-backed rehydration and cooperative
  per-session writer ownership are implemented and verified. Remaining limits: no
  Windows/network-filesystem durability, no encryption or authentication, a
  cooperative (not enforced) writer lock, manual retention without automatic GC, and
  no format migration.
- No parallel execution of one session across processes; durable ownership is
  cooperative, not process isolation.
- No per-session access control; the writer lock is cooperative metadata, not an
  authorization boundary.
- Session IDs reset per manager; they are not globally unique across processes. This
  is why ADR-0003 defines separate durable storage keys rather than treating a
  runtime `SessionId` as a durable identity.

These are roadmap items, not hidden behavior.
