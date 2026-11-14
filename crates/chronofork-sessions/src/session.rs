//! One independent execution session.

use std::collections::BTreeMap;
use std::fmt;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendSnapshot, MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, SessionId, SnapshotId, Tick};

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

    /// Returns the tick of the initial state.
    #[must_use]
    pub const fn initial_tick(&self) -> Tick {
        self.initial_state.tick()
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

    /// Returns all checkpoint identifiers in ascending order.
    pub fn snapshot_ids(&self) -> impl Iterator<Item = SnapshotId> + '_ {
        self.snapshots.keys().copied()
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
        self.ensure_compatible(&snapshot)?;
        self.backend.restore(snapshot.backend())?;
        Ok(())
    }

    /// Restores the state captured when the session was created.
    pub fn reset(&mut self) -> Result<(), SessionError> {
        self.ensure_open()?;
        self.backend.restore(&self.initial_state)?;
        Ok(())
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

    fn ensure_compatible(&self, snapshot: &Snapshot) -> Result<(), SessionError> {
        if snapshot.backend().backend_id() != self.descriptor().id() {
            return Err(SessionError::IncompatibleSnapshot(format!(
                "snapshot backend {} does not match session backend {}",
                snapshot.backend().backend_id(),
                self.descriptor().id()
            )));
        }
        if snapshot.backend().config_hash() != self.initial_state.config_hash() {
            return Err(SessionError::IncompatibleSnapshot(
                "snapshot machine configuration hash does not match this session".to_owned(),
            ));
        }
        Ok(())
    }

    fn allocate_snapshot_id(&mut self) -> Result<SnapshotId, SessionError> {
        let id = self.next_snapshot_id;
        self.next_snapshot_id = id.checked_add(1).ok_or(SessionError::IdSpaceExhausted)?;
        Ok(SnapshotId::from_raw(id))
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
