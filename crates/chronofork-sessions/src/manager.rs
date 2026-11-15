//! Session registry and fork orchestration.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use chronofork_backend_api::{BackendDescriptor, BackendFactory, BackendSnapshot, StepOutcome};
use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick};

use crate::error::SessionError;
use crate::lineage::Lineage;
use crate::session::{Session, SessionInspection, SessionState, map_backend_compatibility};
use crate::snapshot::Snapshot;

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
/// document it.
pub struct SessionManager {
    factory: Arc<dyn BackendFactory>,
    sessions: BTreeMap<SessionId, Session>,
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

    /// Creates a fresh session from the factory's initial state.
    pub fn create_session(&mut self) -> Result<SessionId, SessionError> {
        let backend = self.factory.create()?;
        let initial_state = backend.snapshot()?;
        let id = self.allocate_session_id()?;
        let session = Session::new(id, backend, Lineage::root(), initial_state);
        self.sessions.insert(id, session);
        Ok(id)
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
        let id = self.allocate_session_id()?;
        let mut session =
            Session::new(id, backend, Lineage::imported(snapshot.id()), initial_state);
        session.adopt_snapshot(snapshot)?;
        self.sessions.insert(id, session);
        Ok(id)
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
        let id = self.allocate_session_id()?;
        let mut session =
            Session::new(id, backend, Lineage::imported(snapshot.id()), initial_state);
        session.adopt_snapshot(snapshot)?;
        self.sessions.insert(id, session);
        self.durable_session_keys.insert(id, storage_session_key);
        Ok(id)
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
    pub fn snapshot_record(
        &self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> Result<&Snapshot, SessionError> {
        self.session(session)?
            .snapshot_record(snapshot)
            .ok_or(SessionError::UnknownSnapshot { session, snapshot })
    }

    /// Forks a session from one of its stored checkpoints.
    pub fn fork_from_snapshot(
        &mut self,
        source_session: SessionId,
        snapshot_id: SnapshotId,
    ) -> Result<SessionId, SessionError> {
        let (snapshot, parent_depth) = {
            let source = self
                .sessions
                .get(&source_session)
                .ok_or(SessionError::UnknownSession(source_session))?;
            let snapshot = source
                .snapshot_record(snapshot_id)
                .ok_or(SessionError::UnknownSnapshot {
                    session: source_session,
                    snapshot: snapshot_id,
                })?
                .clone();
            (snapshot, source.lineage().depth())
        };
        let mut backend = self.factory.create()?;
        backend
            .restore(snapshot.backend())
            .map_err(map_backend_compatibility)?;
        let initial_state = backend.snapshot()?;
        let id = self.allocate_session_id()?;
        let lineage = Lineage::child(source_session, snapshot_id, parent_depth);
        let mut session = Session::new(id, backend, lineage, initial_state);
        session.adopt_snapshot(snapshot)?;
        self.sessions.insert(id, session);
        Ok(id)
    }

    /// Captures the current state of a session and immediately forks it.
    pub fn fork_current(&mut self, source_session: SessionId) -> Result<SessionId, SessionError> {
        let snapshot_id = self.snapshot(source_session)?;
        self.fork_from_snapshot(source_session, snapshot_id)
    }

    /// Captures a checkpoint for a session.
    pub fn snapshot(&mut self, session: SessionId) -> Result<SnapshotId, SessionError> {
        self.session_mut(session)?.snapshot()
    }

    /// Serializes a checkpoint.
    pub fn snapshot_bytes(
        &self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> Result<Vec<u8>, SessionError> {
        self.session(session)?.snapshot_bytes(snapshot)
    }

    /// Restores a checkpoint in place.
    pub fn restore(
        &mut self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> Result<(), SessionError> {
        self.session_mut(session)?.restore(snapshot)
    }

    /// Restores a session's creation state.
    pub fn reset(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session_mut(session)?.reset()
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
        self.session_mut(session)?.capture_state()
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
        self.session_mut(session)?.restore_state(state)
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
        self.session_mut(session)?.discard_snapshot_records();
        Ok(())
    }

    /// Returns a session's checkpoint-record count and next allocator value.
    #[cfg(test)]
    pub(crate) fn session_snapshot_identity(
        &self,
        session: SessionId,
    ) -> Result<(usize, u64), SessionError> {
        self.session(session).map(Session::snapshot_identity)
    }

    /// Advances a session by one backend-defined quantum.
    pub fn step(
        &mut self,
        session: SessionId,
        input: InputFrame,
    ) -> Result<StepOutcome, SessionError> {
        self.session_mut(session)?.step(input)
    }

    /// Pauses a session.
    pub fn pause(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session_mut(session)?.pause()
    }

    /// Resumes a session.
    pub fn resume(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session_mut(session)?.resume()
    }

    /// Closes a session.
    pub fn close(&mut self, session: SessionId) -> Result<(), SessionError> {
        self.session_mut(session)?.close()
    }

    /// Reads emulated memory through the debugger path.
    pub fn read_memory(
        &self,
        session: SessionId,
        address: u64,
        length: usize,
    ) -> Result<Vec<u8>, SessionError> {
        self.session(session)?.read_memory(address, length)
    }

    /// Writes emulated memory through the debugger path.
    pub fn write_memory(
        &mut self,
        session: SessionId,
        address: u64,
        data: &[u8],
    ) -> Result<(), SessionError> {
        self.session_mut(session)?.write_memory(address, data)
    }

    /// Returns a read-only inspection of one session.
    pub fn inspect(&self, session: SessionId) -> Result<SessionInspection, SessionError> {
        self.session(session)?.inspect()
    }

    /// Compares two sessions by tick and canonical state digest.
    pub fn compare(
        &self,
        left: SessionId,
        right: SessionId,
    ) -> Result<SessionComparison, SessionError> {
        let left_view = self.inspect(left)?;
        let right_view = self.inspect(right)?;
        let equivalent = left_view.machine.tick == right_view.machine.tick
            && left_view.machine.state_hash == right_view.machine.state_hash;
        Ok(SessionComparison {
            left,
            right,
            left_tick: left_view.machine.tick,
            right_tick: right_view.machine.tick,
            left_hash: left_view.machine.state_hash,
            right_hash: right_view.machine.state_hash,
            equivalent,
        })
    }

    /// Returns summaries of every session, in identifier order.
    pub fn summaries(&self) -> Result<Vec<SessionSummary>, SessionError> {
        self.sessions
            .values()
            .map(|session| {
                let machine = session.inspect()?.machine;
                Ok(SessionSummary {
                    id: session.id(),
                    state: session.state(),
                    tick: machine.tick,
                    state_hash: machine.state_hash,
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

    fn session(&self, id: SessionId) -> Result<&Session, SessionError> {
        self.sessions
            .get(&id)
            .ok_or(SessionError::UnknownSession(id))
    }

    fn session_mut(&mut self, id: SessionId) -> Result<&mut Session, SessionError> {
        self.sessions
            .get_mut(&id)
            .ok_or(SessionError::UnknownSession(id))
    }

    fn allocate_session_id(&mut self) -> Result<SessionId, SessionError> {
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
