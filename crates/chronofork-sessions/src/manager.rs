//! Session registry and fork orchestration.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use chronofork_backend_api::{
    BackendDescriptor, BackendFactory, BackendSnapshot, BatchOutcome, InputSequence, RunControl,
    StepOutcome, StopSet, VerificationPolicy, VerifiedStepOutcome,
};
use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick};
use chronofork_trace::{Trace, TraceConfig};

use crate::error::SessionError;
use crate::lineage::Lineage;
use crate::session::{Session, SessionInspection, SessionState, map_backend_compatibility};
use crate::snapshot::Snapshot;

/// A shared handle to one registered session.
///
/// The handle is the **per-session execution owner**: everything that mutates a
/// session's canonical state holds the session's mutex, and holding one session's
/// mutex never blocks a different session. [`SessionManager`] owns exactly one
/// handle per registered session and [`SessionManager::session_handle`] hands out
/// clones, so an adapter (for example `chronofork-service`) can run long synchronous
/// work on one session without holding a manager-wide lock.
///
/// Synchronous only: [`SessionHandle::lock`] blocks the calling thread and never
/// spawns a background executor. An adapter that holds several locks documents its
/// own acquisition order; this type only provides the owner.
#[derive(Clone, Debug)]
pub struct SessionHandle {
    id: SessionId,
    owner: Arc<Mutex<Session>>,
}

impl SessionHandle {
    /// Wraps one session into an execution owner.
    ///
    /// A session built outside a [`SessionManager`] (for example by
    /// [`SessionHandle::fork_from_snapshot`]) is wrapped here so the owner type
    /// stays the only way to reach a session.
    #[must_use]
    pub fn new(session: Session) -> Self {
        let id = session.id();
        Self {
            id,
            owner: Arc::new(Mutex::new(session)),
        }
    }

    /// Locks this session's owner and returns the guard.
    ///
    /// The guard is the exclusive execution owner: while it is held, no other
    /// thread can step, snapshot or inspect this session, and a different session
    /// stays unaffected. A poisoned owner (a panicking holder) is reported as a
    /// typed [`SessionError::Poisoned`] instead of panicking the caller.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Poisoned`] when a previous holder panicked while
    /// holding this session's owner.
    pub fn lock(&self) -> Result<MutexGuard<'_, Session>, SessionError> {
        self.owner
            .lock()
            .map_err(|_| SessionError::Poisoned(self.id))
    }

    /// Tries to lock this session's owner without waiting.
    ///
    /// This is the cheap-read path of a status/listing operation (ADR-0016 §1,
    /// K1/K8): a caller that must not wait for a run can distinguish "the owner is
    /// held" from "the owner is free" and report the session as running instead of
    /// blocking. `None` means the owner is held by another thread or poisoned;
    /// neither is an error for a non-committal read.
    #[must_use]
    pub fn try_lock(&self) -> Option<MutexGuard<'_, Session>> {
        self.owner.try_lock().ok()
    }

    /// Returns this session's identifier without locking the owner.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// Assigns this handle its final identifier.
    ///
    /// Only a caller that registers the session allocates from the shared counter
    /// and assigns the result; the holder of a handle never reassigns an
    /// identifier, so the value is write-once in practice. The session object is
    /// updated under its own owner, so the identifier a session reports through
    /// [`Session::id`] and the identifier arithmetic use agree.
    pub fn assign_id(&mut self, id: SessionId) {
        self.id = id;
        if let Ok(mut owner) = self.owner.lock() {
            owner.set_id(id);
        }
    }

    /// Forks this session from one of its stored checkpoints into a new handle.
    ///
    /// This is the hand-off form of a fork: the source is addressed by its own
    /// handle, so a session that a caller already owns (and that is no longer
    /// registered with the [`SessionManager`] that created it) can still be
    /// forked. The child is a fresh session with the source as its lineage parent;
    /// it is **not** registered with any manager, so the caller pairs it with
    /// `factory` ownership itself.
    ///
    /// The source owner is held only while the checkpoint is copied. The returned
    /// handle is independent, so the caller may release the source lock first.
    ///
    /// # Errors
    ///
    /// Returns the compatibility, snapshot and backend errors of the existing
    /// fork path. The child carries the placeholder identifier the internal
    /// constructor assigns; a caller that registers it assigns the real identifier
    /// itself (see [`SessionHandle::assign_id`]).
    pub fn fork_from_snapshot(
        &self,
        snapshot_id: SnapshotId,
        factory: &Arc<dyn BackendFactory>,
    ) -> Result<Self, SessionError> {
        let (snapshot, parent_depth, parent_id) = {
            let source = self.lock()?;
            let snapshot = source
                .snapshot_record(snapshot_id)
                .ok_or(SessionError::UnknownSnapshot {
                    session: source.id(),
                    snapshot: snapshot_id,
                })?
                .clone();
            (snapshot, source.lineage().depth(), source.id())
        };
        let mut backend = factory.create()?;
        backend
            .restore(snapshot.backend())
            .map_err(map_backend_compatibility)?;
        let initial_state = backend.snapshot()?;
        let lineage = Lineage::child(parent_id, snapshot_id, parent_depth);
        let mut child = Session::new(backend, lineage, initial_state);
        child.adopt_snapshot(snapshot)?;
        Ok(Self::new(child))
    }

    /// Returns one of this session's stored checkpoint records.
    ///
    /// The record is returned by value so the caller never holds the session owner
    /// while it serializes or publishes the checkpoint. One clone of the record is
    /// the cost of not letting a guard escape this type.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Poisoned`] for a panicked holder and
    /// [`SessionError::UnknownSnapshot`] for a checkpoint this session does not
    /// store.
    pub fn snapshot_record(&self, snapshot: SnapshotId) -> Result<Snapshot, SessionError> {
        let record = self.lock()?.snapshot_record(snapshot).cloned();
        record.ok_or(SessionError::UnknownSnapshot {
            session: self.id,
            snapshot,
        })
    }
}

/// A compact view of one session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    /// Session identifier.
    pub id: SessionId,
    /// Lifecycle state.
    pub state: SessionState,
    /// Current tick.
    pub tick: Tick,
    /// Current state digest.
    pub state_hash: StateHash,
    /// Fork provenance.
    pub lineage: Lineage,
}

/// Result of comparing two sessions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionComparison {
    /// Left session.
    pub left: SessionId,
    /// Right session.
    pub right: SessionId,
    /// Left tick.
    pub left_tick: Tick,
    /// Right tick.
    pub right_tick: Tick,
    /// Left state digest.
    pub left_hash: StateHash,
    /// Right state digest.
    pub right_hash: StateHash,
    /// Whether both sessions have the same tick and state digest.
    pub equivalent: bool,
}

/// Owns all independent sessions created from one backend factory.
///
/// Mutations require `&mut self`. ChronoFork intentionally does not hide a
/// global lock or a background executor inside this type; transport adapters
/// that need shared access must add an explicit synchronization boundary and
/// document it. Each session is stored behind its own mutex, and
/// [`SessionManager::session_handle`] exposes that per-session owner so an
/// adapter can run one session without serializing on a manager-wide lock.
pub struct SessionManager {
    factory: Arc<dyn BackendFactory>,
    sessions: BTreeMap<SessionId, SessionHandle>,
    durable_session_keys: BTreeMap<SessionId, u64>,
    next_session_id: u64,
}

impl SessionManager {
    /// Creates a manager using a backend factory.
    #[must_use]
    pub fn new(factory: Arc<dyn BackendFactory>) -> Self {
        Self {
            factory,
            sessions: BTreeMap::new(),
            durable_session_keys: BTreeMap::new(),
            next_session_id: 1,
        }
    }

    /// Returns the backend descriptor.
    #[must_use]
    pub fn backend_descriptor(&self) -> &BackendDescriptor {
        self.factory.descriptor()
    }

    /// Registers one freshly built session and returns its identifier.
    ///
    /// The identifier is allocated here, so every registration site (create,
    /// import, stored checkpoint, fork, detach-then-build) shares one monotonic
    /// counter and no session object is ever built against a hand-picked id.
    fn register(&mut self, mut session: Session) -> Result<SessionId, SessionError> {
        let id = self.allocate_session_id()?;
        session.set_id(id);
        self.sessions.insert(id, SessionHandle::new(session));
        Ok(id)
    }

    /// Creates a fresh session from the factory's initial state.
    pub fn create_session(&mut self) -> Result<SessionId, SessionError> {
        let backend = self.factory.create()?;
        let initial_state = backend.snapshot()?;
        let session = Session::new(backend, Lineage::root(), initial_state);
        self.register(session)
    }

    /// Returns a shared handle to one registered session.
    ///
    /// The handle is the per-session execution owner (see [`SessionHandle`]): a
    /// caller locks it to run or observe that session and never holds a
    /// manager-wide lock while doing so. The manager keeps its own handle, so the
    /// session stays registered; the clone only shares ownership of the same
    /// session object.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnknownSession`] for an unregistered identifier.
    pub fn session_handle(&self, session: SessionId) -> Result<SessionHandle, SessionError> {
        self.sessions
            .get(&session)
            .cloned()
            .ok_or(SessionError::UnknownSession(session))
    }

    /// Removes a session from this manager and returns its owner handle.
    ///
    /// This is the hand-off used by an adapter that becomes the session's
    /// **single execution owner** (for example `chronofork-service`, which keeps
    /// its own registry of handles). After the hand-off the manager can no longer
    /// observe or mutate that session, so no path can wait on or alias a session
    /// owner the adapter holds. The durable session key, if any, is removed with
    /// it; the session object itself stays alive through the returned handle.
    ///
    /// A manager that never calls this method keeps full ownership of every
    /// session it created.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnknownSession`] for an unregistered identifier.
    pub fn detach(&mut self, session: SessionId) -> Result<SessionHandle, SessionError> {
        let handle = self
            .sessions
            .remove(&session)
            .ok_or(SessionError::UnknownSession(session))?;
        self.durable_session_keys.remove(&session);
        Ok(handle)
    }

    /// Creates a session by importing serialized snapshot bytes.
    ///
    /// The imported session is a root in this manager's fork tree, but its
    /// lineage records the snapshot it came from.
    pub fn create_session_from_snapshot_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<SessionId, SessionError> {
        let snapshot = Snapshot::from_bytes(bytes)?;
        let mut backend = self.factory.create()?;
        backend
            .restore(snapshot.backend())
            .map_err(map_backend_compatibility)?;
        let initial_state = backend.snapshot()?;
        let mut session = Session::new(backend, Lineage::imported(snapshot.id()), initial_state);
        session.adopt_snapshot(snapshot)?;
        self.register(session)
    }

    /// Creates a session by rehydrating a checkpoint already validated by the
    /// durable storage layer.
    ///
    /// The checkpoint is restored through the existing backend compatibility
    /// path, the runtime session is registered, and its durable session key is
    /// recorded so a later fork can name the durable parent.
    pub fn create_session_from_stored_snapshot(
        &mut self,
        snapshot: Snapshot,
        storage_session_key: u64,
    ) -> Result<SessionId, SessionError> {
        if storage_session_key == 0 {
            return Err(SessionError::UnknownStorageSession(0));
        }
        let mut backend = self.factory.create()?;
        backend
            .restore(snapshot.backend())
            .map_err(map_backend_compatibility)?;
        let initial_state = backend.snapshot()?;
        let mut session = Session::new(backend, Lineage::imported(snapshot.id()), initial_state);
        session.adopt_snapshot(snapshot)?;
        let session = self.register(session)?;
        self.durable_session_keys
            .insert(session, storage_session_key);
        Ok(session)
    }

    /// Records the durable session key for a runtime session.
    ///
    /// A session that is not stored yet has no durable key; callers attach one
    /// after [`SnapshotRepository::create_session`](crate::SnapshotRepository::create_session)
    /// publishes it. Attaching a different key to an already-attached session is
    /// a typed conflict.
    pub fn attach_storage_session(
        &mut self,
        session: SessionId,
        storage_session_key: u64,
    ) -> Result<(), SessionError> {
        self.session(session)?;
        if storage_session_key == 0 {
            return Err(SessionError::UnknownStorageSession(0));
        }
        if let Some(existing) = self.durable_session_keys.get(&session) {
            if *existing != storage_session_key {
                return Err(SessionError::SessionConflict(*existing));
            }
        }
        self.durable_session_keys
            .insert(session, storage_session_key);
        Ok(())
    }

    /// Returns the durable session key, if this runtime session is stored.
    #[must_use]
    pub fn durable_session_key(&self, session: SessionId) -> Option<u64> {
        self.durable_session_keys.get(&session).copied()
    }

    /// Returns one stored checkpoint record.
    ///
    /// The record is returned by value: a session is owned by an `Arc<Mutex<...>>`
    /// handle, so a borrowed record could not outlive the guard that owns it. The
    /// clone keeps the lock protocol free of escaping guards and is bounded by the
    /// checkpoint size the caller is about to serialize anyway.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnknownSession`] for an unregistered identifier,
    /// [`SessionError::UnknownSnapshot`] for a checkpoint the session does not
    /// store and [`SessionError::Poisoned`] for a panicked holder.
    pub fn snapshot_record(
        &self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> Result<Snapshot, SessionError> {
        self.session(session)?.snapshot_record(snapshot)
    }

    /// Forks a session from one of its stored checkpoints.
    pub fn fork_from_snapshot(
        &mut self,
        source_session: SessionId,
        snapshot_id: SnapshotId,
    ) -> Result<SessionId, SessionError> {
        // Phase 1: read the source under its own owner and copy the checkpoint, so
        // the source owner is not held while the child backend is built.
        let (snapshot, parent_depth) = {
            let handle = self.session(source_session)?;
            let source = handle.lock()?;
            let snapshot = source
                .snapshot_record(snapshot_id)
                .ok_or(SessionError::UnknownSnapshot {
                    session: source_session,
                    snapshot: snapshot_id,
                })?
                .clone();
            (snapshot, source.lineage().depth())
        };
        // Phase 2: build the child with no session owner and no manager-wide state
        // held, then register it.
        let mut backend = self.factory.create()?;
        backend
            .restore(snapshot.backend())
            .map_err(map_backend_compatibility)?;
        let initial_state = backend.snapshot()?;
        let lineage = Lineage::child(source_session, snapshot_id, parent_depth);
        let mut session = Session::new(backend, lineage, initial_state);
        session.adopt_snapshot(snapshot)?;
        self.register(session)
    }

    /// Captures the current state of a session and immediately forks it.
    pub fn fork_current(&mut self, source_session: SessionId) -> Result<SessionId, SessionError> {
        let snapshot_id = self.snapshot(source_session)?;
        self.fork_from_snapshot(source_session, snapshot_id)
    }

    /// Captures a checkpoint for a session.
    pub fn snapshot(&mut self, session: SessionId) -> Result<SnapshotId, SessionError> {
        self.session(session)?.lock()?.snapshot()
    }

    /// Serializes a checkpoint.
    pub fn snapshot_bytes(
        &self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> Result<Vec<u8>, SessionError> {
        self.session(session)?.lock()?.snapshot_bytes(snapshot)
    }

    /// Restores a checkpoint in place.
    pub fn restore(
        &mut self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> Result<(), SessionError> {
        self.session(session)?.lock()?.restore(snapshot)
    }

    /// Restores a session's creation state.
    pub fn reset(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session(session)?.lock()?.reset()
    }

    /// Captures a bounded structured trace from one session's backend.
    ///
    /// The manager looks up the session and delegates to
    /// [`Session::capture_trace`]; no second capture loop exists. Capture
    /// advances the session by the executed steps and does not allocate a
    /// snapshot identity or change lineage.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnknownSession`] for an unregistered identifier,
    /// the session lifecycle errors of [`Session::capture_trace`] and
    /// [`SessionError::Trace`] for a typed trace or backend failure.
    pub fn capture_trace(
        &mut self,
        session: SessionId,
        config: &TraceConfig,
        steps: u64,
    ) -> Result<Trace, SessionError> {
        self.session(session)?.lock()?.capture_trace(config, steps)
    }

    /// Captures a session's canonical backend state without creating a checkpoint.
    ///
    /// This is the crate-internal capture path of the in-process rewind cache
    /// (ADR-0007 §2.5). It allocates no [`SnapshotId`], inserts nothing into the
    /// session's snapshot map and does not advance its allocator; the returned
    /// value is a transient canonical state, not a user-visible checkpoint. The
    /// only caller is the rewind player.
    pub(crate) fn capture_session_state(
        &mut self,
        session: SessionId,
    ) -> Result<BackendSnapshot, SessionError> {
        self.session(session)?.lock()?.capture_state()
    }

    /// Restores a canonical state captured by [`Self::capture_session_state`].
    ///
    /// The restore goes through the same backend method and the same
    /// compatibility mapping as [`Self::restore`], so `validate_envelope`
    /// remains the single compatibility predicate.
    pub(crate) fn restore_session_state(
        &mut self,
        session: SessionId,
        state: &BackendSnapshot,
    ) -> Result<(), SessionError> {
        self.session(session)?.lock()?.restore_state(state)
    }

    /// Drops a session's stored checkpoint records while retaining its creation
    /// state.
    ///
    /// The in-process player owns a private imported session that exposes no
    /// checkpoint identity; this keeps its snapshot map empty (M3-A28.2)
    /// instead of retaining a duplicate of the canonical floor state. This is
    /// not a public session operation.
    pub(crate) fn discard_session_checkpoints(
        &mut self,
        session: SessionId,
    ) -> Result<(), SessionError> {
        self.session(session)?.lock()?.discard_snapshot_records();
        Ok(())
    }

    /// Returns a session's checkpoint-record count and next allocator value.
    #[cfg(test)]
    pub(crate) fn session_snapshot_identity(
        &self,
        session: SessionId,
    ) -> Result<(usize, u64), SessionError> {
        let identity = self.session(session)?.lock()?.snapshot_identity();
        Ok(identity)
    }

    /// Advances a session by one backend-defined quantum.
    pub fn step(
        &mut self,
        session: SessionId,
        input: InputFrame,
    ) -> Result<StepOutcome, SessionError> {
        self.session(session)?.lock()?.step(input)
    }

    /// Advances a session by one quantum through the backend's verified path.
    pub fn step_verified(
        &mut self,
        session: SessionId,
        input: InputFrame,
    ) -> Result<VerifiedStepOutcome, SessionError> {
        self.session(session)?.lock()?.step_verified(input)
    }

    /// Runs one deterministic batch on a session.
    pub fn run_until(
        &mut self,
        session: SessionId,
        stop: &StopSet,
        steps: u64,
        inputs: &InputSequence,
        policy: VerificationPolicy,
        control: &RunControl,
    ) -> Result<BatchOutcome, SessionError> {
        self.session(session)?
            .lock()?
            .run_until(stop, steps, inputs, policy, control)
    }

    /// Runs one deterministic cycle-budgeted batch on a session.
    pub fn run_cycles(
        &mut self,
        session: SessionId,
        cycles: u64,
        stop: &StopSet,
        policy: VerificationPolicy,
        control: &RunControl,
    ) -> Result<BatchOutcome, SessionError> {
        self.session(session)?
            .lock()?
            .run_cycles(cycles, stop, policy, control)
    }

    /// Pauses a session.
    pub fn pause(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session(session)?.lock()?.pause()
    }

    /// Resumes a session.
    pub fn resume(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session(session)?.lock()?.resume()
    }

    /// Closes a session.
    pub fn close(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session(session)?.lock()?.close()
    }

    /// Reads emulated memory through the debugger path.
    pub fn read_memory(
        &self,
        session: SessionId,
        address: u64,
        length: usize,
    ) -> Result<Vec<u8>, SessionError> {
        self.session(session)?.lock()?.read_memory(address, length)
    }

    /// Writes emulated memory through the debugger path.
    pub fn write_memory(
        &mut self,
        session: SessionId,
        address: u64,
        data: &[u8],
    ) -> Result<(), SessionError> {
        self.session(session)?.lock()?.write_memory(address, data)
    }

    /// Returns a read-only inspection of one session.
    pub fn inspect(&self, session: SessionId) -> Result<SessionInspection, SessionError> {
        self.session(session)?.lock()?.inspect()
    }

    /// Returns the detailed inspection with an opt-in memory window.
    pub fn inspect_with_memory(
        &self,
        session: SessionId,
        base: u64,
        length: usize,
    ) -> Result<chronofork_backend_api::MachineInspection, SessionError> {
        self.session(session)?
            .lock()?
            .inspect_with_memory(base, length)
    }

    /// Returns the cheap status of one session.
    pub fn status(
        &self,
        session: SessionId,
    ) -> Result<chronofork_backend_api::MachineStatus, SessionError> {
        self.session(session)?.lock()?.status()
    }

    /// Returns the canonical state digest of one session explicitly.
    pub fn state_hash(&self, session: SessionId) -> Result<StateHash, SessionError> {
        self.session(session)?.lock()?.state_hash()
    }

    /// Compares two sessions by tick and canonical state digest.
    pub fn compare(
        &self,
        left: SessionId,
        right: SessionId,
    ) -> Result<SessionComparison, SessionError> {
        let left_view = self.inspect(left)?;
        let right_view = self.inspect(right)?;
        let left_hash = self.state_hash(left)?;
        let right_hash = self.state_hash(right)?;
        let equivalent =
            left_view.machine.tick == right_view.machine.tick && left_hash == right_hash;
        Ok(SessionComparison {
            left,
            right,
            left_tick: left_view.machine.tick,
            right_tick: right_view.machine.tick,
            left_hash,
            right_hash,
            equivalent,
        })
    }

    /// Returns summaries of every session, in identifier order.
    pub fn summaries(&self) -> Result<Vec<SessionSummary>, SessionError> {
        self.sessions
            .values()
            .map(|handle| {
                let session = handle.lock()?;
                let status = session.status()?;
                let state_hash = session.state_hash()?;
                Ok(SessionSummary {
                    id: session.id(),
                    state: session.state(),
                    tick: status.tick,
                    state_hash,
                    lineage: session.lineage().clone(),
                })
            })
            .collect()
    }

    /// Returns the number of registered sessions, including closed ones.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Returns whether no sessions are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Returns one session's owner handle.
    ///
    /// The clone is `O(1)` (an `Arc` refcount) and keeps the session registered
    /// and lockable after the caller releases any manager-wide state.
    fn session(&self, id: SessionId) -> Result<SessionHandle, SessionError> {
        self.sessions
            .get(&id)
            .cloned()
            .ok_or(SessionError::UnknownSession(id))
    }

    /// Allocates the next session identifier from this manager's monotonic counter.
    ///
    /// The counter is the only state `create`/`fork` share, so an adapter that owns
    /// sessions itself (and builds them through [`SessionHandle::fork_from_snapshot`])
    /// can mint identifiers from the same space without registering the session
    /// with this manager. Allocation is never reused across a process.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::IdSpaceExhausted`] when the `u64` identifier space
    /// is exhausted.
    pub fn allocate_session_id(&mut self) -> Result<SessionId, SessionError> {
        let id = self.next_session_id;
        self.next_session_id = id.checked_add(1).ok_or(SessionError::IdSpaceExhausted)?;
        Ok(SessionId::from_raw(id))
    }
}

impl fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionManager")
            .field("backend", &self.factory.descriptor().id())
            .field("sessions", &self.sessions.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_backend_api::{BackendId, BackendSnapshot};
    use chronofork_test_backend::SyntheticBackendFactory;

    use super::SessionManager;
    use crate::error::SessionError;
    use crate::snapshot::Snapshot;

    #[test]
    fn importing_a_foreign_snapshot_is_rejected_at_the_session_boundary() {
        let factory = SyntheticBackendFactory::default();
        let mut manager = SessionManager::new(Arc::new(factory));
        let session = manager.create_session().expect("session");
        let id = manager.snapshot(session).expect("snapshot");
        let bytes = manager.snapshot_bytes(session, id).expect("bytes");
        let original = Snapshot::from_bytes(&bytes).expect("decode snapshot");
        let backend = BackendSnapshot::new(
            BackendId::new("chronofork.foreign"),
            original.backend().backend_version(),
            original.backend().format_version(),
            original.backend().tick(),
            original.backend().config_hash(),
            original.backend().state_hash(),
            original.backend().payload().to_vec(),
        );
        let foreign = Snapshot::new(original.id(), original.origin_session(), backend);
        let bytes = foreign.to_bytes().expect("serialize foreign snapshot");
        let error = manager
            .create_session_from_snapshot_bytes(&bytes)
            .expect_err("foreign backend must be rejected");
        assert!(
            matches!(&error, SessionError::IncompatibleSnapshot(message) if message.contains("backend_id")),
            "expected a session-level incompatibility error, got {error}"
        );
    }
}
