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

Snapshot IDs are per session. The outer serialized envelope carries the origin
session and snapshot ID for diagnostics, not as a global identity guarantee.

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

For future transports:

- One session has one mutable owner at a time.
- Concurrent reads may be allowed only if the backend guarantees side-effect-free
  inspection.
- A shared session must use an explicit synchronization policy (for example a
  per-session mutex) at the transport/service layer.
- Silent races are forbidden. A lock timeout or contention must be a typed error.
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
- backend failure.

Operations validate before mutating. A failed restore must not leave a session
half-restored; the backend contract requires atomic rejection of incompatible
snapshots. This invariant needs dedicated backend-specific tests when a real
backend is adopted.

## Limitations

- No background execution, run loop, frame pacing or wall-clock scheduling.
- No copy-on-write or memory-page sharing between forks.
- No persistence database; snapshots are in memory plus explicit byte envelopes.
- No multi-process isolation.
- No per-session access control or ownership metadata.
- Session IDs reset per manager; they are not globally unique across processes.

These are roadmap items, not hidden behavior.
