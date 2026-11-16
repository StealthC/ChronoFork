//! One independent execution session.

use std::collections::BTreeMap;
use std::fmt;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendSnapshot, MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, SessionId, SnapshotId};

use crate::error::SessionError;
use crate::lineage::Lineage;
use crate::snapshot::Snapshot;

/// Lifecycle state of a session.
///
/// `Ready` means execution is permitted; it does not imply a background thread
/// is running. ChronoFork's bootstrap API is synchronous and callers decide when
/// to advance a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    /// The session may execute and be mutated.
    Ready,
    /// Execution is paused but inspection and checkpointing remain available.
    Paused,
    /// The session is closed and rejects further operations.
    Closed,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => f.write_str("ready"),
            Self::Paused => f.write_str("paused"),
            Self::Closed => f.write_str("closed"),
        }
    }
}

/// A debugger-oriented view of a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionInspection {
    /// Session identifier.
    pub id: SessionId,
    /// Current lifecycle state.
    pub state: SessionState,
    /// Fork provenance.
    pub lineage: Lineage,
    /// Backend descriptor.
    pub backend: BackendDescriptor,
    /// Machine state.
    pub machine: MachineInspection,
}

/// A single independent emulated machine plus its checkpoints and lineage.
pub struct Session {
    id: SessionId,
    backend: Box<dyn Backend>,
    state: SessionState,
    lineage: Lineage,
    initial_state: BackendSnapshot,
    snapshots: BTreeMap<SnapshotId, Snapshot>,
    next_snapshot_id: u64,
}

impl Session {
    pub(crate) fn new(
        id: SessionId,
        backend: Box<dyn Backend>,
        lineage: Lineage,
        initial_state: BackendSnapshot,
    ) -> Self {
        Self {
            id,
            backend,
            state: SessionState::Ready,
            lineage,
            initial_state,
            snapshots: BTreeMap::new(),
            next_snapshot_id: 1,
        }
    }

    /// Returns the session identifier.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns fork provenance.
    #[must_use]
    pub const fn lineage(&self) -> &Lineage {
        &self.lineage
    }

    /// Returns the backend descriptor.
    #[must_use]
    pub fn descriptor(&self) -> &BackendDescriptor {
        self.backend.descriptor()
    }

    /// Returns the backend snapshot captured at creation.
    #[must_use]
    pub const fn initial_state(&self) -> &BackendSnapshot {
        &self.initial_state
    }

    /// Returns a checkpoint, if it belongs to this session.
    #[must_use]
    pub fn snapshot_record(&self, id: SnapshotId) -> Option<&Snapshot> {
        self.snapshots.get(&id)
    }

    /// Produces an inspection view.
    pub fn inspect(&self) -> Result<SessionInspection, SessionError> {
        self.ensure_open()?;
        let machine = self.backend.inspect()?;
        Ok(SessionInspection {
            id: self.id,
            state: self.state,
            lineage: self.lineage.clone(),
            backend: self.descriptor().clone(),
            machine,
        })
    }

    /// Advances one backend-defined quantum.
    pub fn step(&mut self, input: InputFrame) -> Result<StepOutcome, SessionError> {
        self.ensure_open()?;
        if self.state != SessionState::Ready {
            return Err(SessionError::NotRunning {
                session: self.id,
                state: self.state,
            });
        }
        match self.backend.step(input) {
            Ok(outcome) => Ok(outcome),
            Err(BackendError::Halted) => Err(SessionError::Halted(self.id)),
            Err(error) => Err(SessionError::Backend(error)),
        }
    }

    /// Captures a checkpoint and stores it in this session.
    pub fn snapshot(&mut self) -> Result<SnapshotId, SessionError> {
        self.ensure_open()?;
        let backend = self.backend.snapshot()?;
        let id = self.allocate_snapshot_id()?;
        let snapshot = Snapshot::new(id, self.id, backend);
        self.snapshots.insert(id, snapshot);
        Ok(id)
    }

    /// Serializes a stored checkpoint.
    pub fn snapshot_bytes(&self, id: SnapshotId) -> Result<Vec<u8>, SessionError> {
        self.ensure_open()?;
        let snapshot = self
            .snapshots
            .get(&id)
            .ok_or(SessionError::UnknownSnapshot {
                session: self.id,
                snapshot: id,
            })?;
        snapshot.to_bytes()
    }

    /// Restores a stored checkpoint into this session.
    pub fn restore(&mut self, id: SnapshotId) -> Result<(), SessionError> {
        self.ensure_open()?;
        let snapshot = self
            .snapshots
            .get(&id)
            .ok_or(SessionError::UnknownSnapshot {
                session: self.id,
                snapshot: id,
            })?
            .clone();
        self.backend
            .restore(snapshot.backend())
            .map_err(map_backend_compatibility)?;
        Ok(())
    }

    /// Restores the state captured when the session was created.
    pub fn reset(&mut self) -> Result<(), SessionError> {
        self.ensure_open()?;
        self.backend.restore(&self.initial_state)?;
        Ok(())
    }

    /// Captures the canonical backend state without creating a checkpoint.
    ///
    /// This is the crate-internal capture path of the in-process rewind cache
    /// (ADR-0007 §2.5). It allocates no [`SnapshotId`], inserts nothing into the
    /// session's snapshot map and does not advance the allocator: the returned
    /// [`BackendSnapshot`] is a transient canonical state, not a user-visible
    /// checkpoint. Restore it with [`Self::restore_state`].
    pub(crate) fn capture_state(&mut self) -> Result<BackendSnapshot, SessionError> {
        self.ensure_open()?;
        Ok(self.backend.snapshot()?)
    }

    /// Restores a canonical state captured by [`Self::capture_state`].
    ///
    /// Compatibility is decided by the same backend restore path and the same
    /// `validate_envelope` predicate as [`Self::restore`].
    pub(crate) fn restore_state(&mut self, state: &BackendSnapshot) -> Result<(), SessionError> {
        self.ensure_open()?;
        self.backend
            .restore(state)
            .map_err(map_backend_compatibility)
    }

    /// Drops stored checkpoint records while retaining the creation state.
    ///
    /// The in-process player owns a private imported session that exposes no
    /// checkpoint identity. Discarding the import record keeps that session's
    /// snapshot map empty (ADR-0007 §2.5, M3-A28.2) and avoids retaining a
    /// second copy of the canonical floor state, which already lives in
    /// [`Self::initial_state`].
    pub(crate) fn discard_snapshot_records(&mut self) {
        self.snapshots.clear();
    }

    /// Returns the number of stored checkpoint records and the next allocator.
    #[cfg(test)]
    pub(crate) fn snapshot_identity(&self) -> (usize, u64) {
        (self.snapshots.len(), self.next_snapshot_id)
    }

    /// Pauses execution. Pausing an already paused session is a no-op.
    pub fn pause(&mut self) -> Result<(), SessionError> {
        self.ensure_open()?;
        self.state = SessionState::Paused;
        Ok(())
    }

    /// Resumes execution. Resuming a ready session is a no-op.
    pub fn resume(&mut self) -> Result<(), SessionError> {
        self.ensure_open()?;
        self.state = SessionState::Ready;
        Ok(())
    }

    /// Closes the session. Closing twice is a no-op.
    pub fn close(&mut self) -> Result<(), SessionError> {
        self.state = SessionState::Closed;
        Ok(())
    }

    /// Reads emulated memory through the debugger path.
    pub fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, SessionError> {
        self.ensure_open()?;
        Ok(self.backend.read_memory(address, length)?)
    }

    /// Writes emulated memory through the debugger path.
    ///
    /// Writes are permitted while paused as well as while ready, because
    /// debugger workflows commonly modify memory before resuming.
    pub fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), SessionError> {
        self.ensure_open()?;
        Ok(self.backend.write_memory(address, data)?)
    }

    pub(crate) fn adopt_snapshot(&mut self, snapshot: Snapshot) -> Result<(), SessionError> {
        let next = snapshot
            .id()
            .raw()
            .checked_add(1)
            .ok_or(SessionError::IdSpaceExhausted)?;
        self.next_snapshot_id = self.next_snapshot_id.max(next);
        self.snapshots.insert(snapshot.id(), snapshot);
        Ok(())
    }

    fn ensure_open(&self) -> Result<(), SessionError> {
        if self.state == SessionState::Closed {
            Err(SessionError::Closed(self.id))
        } else {
            Ok(())
        }
    }

    fn allocate_snapshot_id(&mut self) -> Result<SnapshotId, SessionError> {
        let id = self.next_snapshot_id;
        self.next_snapshot_id = id.checked_add(1).ok_or(SessionError::IdSpaceExhausted)?;
        Ok(SnapshotId::from_raw(id))
    }
}

/// Translates a backend compatibility failure into the session error contract.
///
/// Backend snapshot compatibility is validated in exactly one place,
/// [`chronofork_backend_api::validate_envelope`]. The session layer only maps its
/// typed error to [`SessionError::IncompatibleSnapshot`] so callers keep the
/// session-level contract without repeating the backend-id/config-hash rules.
pub(crate) fn map_backend_compatibility(error: BackendError) -> SessionError {
    match error {
        BackendError::IncompatibleSnapshot {
            field,
            expected,
            found,
        } => SessionError::IncompatibleSnapshot(format!(
            "snapshot {field} mismatch: expected {expected}, found {found}"
        )),
        other => SessionError::from(other),
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("lineage", &self.lineage)
            .field("backend", &self.descriptor().id())
            .field("snapshots", &self.snapshots.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::{BackendFactory, BackendId, BackendSnapshot};
    use chronofork_core::{SessionId, SnapshotId, StateHash};
    use chronofork_test_backend::SyntheticBackendFactory;

    use super::Session;
    use crate::error::SessionError;
    use crate::lineage::Lineage;
    use crate::snapshot::Snapshot;

    fn session() -> Session {
        let factory = SyntheticBackendFactory::default();
        let backend = factory.create().expect("backend");
        let initial = backend.snapshot().expect("initial snapshot");
        Session::new(SessionId::from_raw(1), backend, Lineage::root(), initial)
    }

    fn snapshot_with(
        initial: &BackendSnapshot,
        backend_id: &str,
        config_hash: StateHash,
    ) -> Snapshot {
        let backend = BackendSnapshot::new(
            BackendId::new(backend_id),
            initial.backend_version(),
            initial.format_version(),
            initial.tick(),
            config_hash,
            initial.state_hash(),
            initial.payload().to_vec(),
        );
        Snapshot::new(SnapshotId::from_raw(1), SessionId::from_raw(1), backend)
    }

    #[test]
    fn restore_rejects_a_snapshot_with_a_different_backend_id() {
        let mut session = session();
        let initial = session.initial_state().clone();
        let snapshot = snapshot_with(&initial, "chronofork.other", initial.config_hash());
        let id = snapshot.id();
        session.adopt_snapshot(snapshot).expect("adopt snapshot");
        let error = session
            .restore(id)
            .expect_err("mismatched backend must fail");
        assert!(
            matches!(&error, SessionError::IncompatibleSnapshot(message) if message.contains("backend_id")),
            "expected a session-level incompatibility error, got {error}"
        );
    }

    #[test]
    fn restore_rejects_a_snapshot_with_a_different_config_hash() {
        let mut session = session();
        let initial = session.initial_state().clone();
        let different = StateHash::from_bytes([0xAB; 32]);
        assert_ne!(different, initial.config_hash());
        let snapshot = snapshot_with(&initial, initial.backend_id().as_str(), different);
        let id = snapshot.id();
        session.adopt_snapshot(snapshot).expect("adopt snapshot");
        let error = session.restore(id).expect_err("config mismatch must fail");
        assert!(
            matches!(&error, SessionError::IncompatibleSnapshot(message) if message.contains("config_hash")),
            "expected a session-level incompatibility error, got {error}"
        );
    }
}
