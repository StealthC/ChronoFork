//! One independent execution session.

use std::collections::BTreeMap;
use std::fmt;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendSnapshot, BatchOutcome, InputSequence,
    MachineInspection, MachineStatus, RunControl, StepOutcome, StopBudget, StopSet,
    VerificationPolicy, VerifiedStepOutcome,
};
use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash};
use chronofork_trace::{Trace, TraceConfig, capture};

use crate::error::SessionError;
use crate::lineage::Lineage;
use crate::snapshot::{Snapshot, snapshot_record_bytes};

/// Which per-session quota a rejection exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionQuota {
    /// `max_snapshots_per_session`
    Snapshots,
    /// `max_bytes_per_session`
    Bytes,
}

impl SessionQuota {
    /// Returns the stable label of this quota.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Snapshots => "max_snapshots_per_session",
            Self::Bytes => "max_bytes_per_session",
        }
    }
}

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
    /// Machine state (registers; no digest inside).
    pub machine: MachineInspection,
    /// Canonical state digest, computed explicitly once for this view.
    pub state_hash: StateHash,
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
    /// Payload bytes of the most recent state this session observed.
    ///
    /// It is the prediction input of the byte-quota preflight: the quota is checked
    /// **before** a checkpoint is captured, so the session needs a size estimate that
    /// costs nothing to obtain. It starts at the creation state and is refreshed by
    /// every capture, restore and reset.
    state_bytes: usize,
    /// Whether `state_bytes` still describes the current backend state.
    ///
    /// A `step`, a debugger write or a trace can change the payload size, so they clear
    /// this flag. The byte preflight then uses the bound the backend **declares**
    /// ([`Backend::snapshot_payload_bound`]) instead of the stale size, and when the
    /// backend declares none it refuses a quota-checked capture instead of allocating it
    /// (ADR-0016 §3).
    state_observed: bool,
    /// Serialized size of every stored checkpoint, captured when it was stored.
    ///
    /// The byte quota is decided from this cache: re-serializing the retained records for
    /// every preflight would allocate and walk them just to count bytes, which is the
    /// accounting the quota is supposed to bound.
    record_bytes: BTreeMap<SnapshotId, usize>,
    /// Exact serialized size of the last checkpoint this session stored.
    last_record_bytes: Option<usize>,
    /// Payload bytes of that last stored checkpoint.
    last_record_payload: usize,
}

impl Session {
    /// Builds a session with a placeholder identifier.
    ///
    /// The registrar assigns the real identifier before the session is exposed
    /// (see [`crate::SessionHandle::assign_id`]), so identifier allocation lives in
    /// exactly one place instead of at each construction site.
    pub(crate) fn new(
        backend: Box<dyn Backend>,
        lineage: Lineage,
        initial_state: BackendSnapshot,
    ) -> Self {
        let state_bytes = initial_state.payload().len();
        Self {
            id: SessionId::from_raw(0),
            backend,
            state: SessionState::Ready,
            lineage,
            initial_state,
            snapshots: BTreeMap::new(),
            next_snapshot_id: 1,
            state_bytes,
            state_observed: true,
            record_bytes: BTreeMap::new(),
            last_record_bytes: None,
            last_record_payload: 0,
        }
    }

    /// Returns the payload bound the byte-quota preflight may assume, when it can.
    ///
    /// The exact observed size is a bound only while that observation still describes the
    /// current state. Once a `step`, a trace or a debug write changed the state, the
    /// previous size says nothing about the new one: the bound must then come from the
    /// backend's declaration ([`Backend::snapshot_payload_bound`]). `None` means the
    /// layer cannot bound the capture and must refuse it rather than allocate it.
    fn payload_bound(&self) -> Option<usize> {
        if self.state_observed {
            Some(self.state_bytes)
        } else {
            self.backend
                .snapshot_payload_bound()
                .map(|declared| declared.max(self.state_bytes))
        }
    }

    /// Returns the serialized record size of one checkpoint with `payload` payload bytes.
    ///
    /// The envelope around the payload is derived from the last stored record when one
    /// exists (its envelope minus its own payload), and otherwise from the format's
    /// envelope accounting. It is therefore exact for the common case and an estimate
    /// only for the very first capture of a session.
    fn record_bytes_for(&self, payload: usize) -> usize {
        let backend_id = self.backend.descriptor().id().as_str().len();
        let backend_version = self.backend.descriptor().version().len();
        match self.last_record_bytes {
            Some(total) if total >= self.last_record_payload => {
                total - self.last_record_payload + payload
            }
            _ => snapshot_record_bytes(backend_id, backend_version, payload),
        }
    }

    /// Stores one checkpoint and its exact serialized size in the two accounting maps.
    fn store_snapshot(&mut self, id: SnapshotId, snapshot: Snapshot, record_bytes: usize) {
        self.snapshots.insert(id, snapshot);
        self.record_bytes.insert(id, record_bytes);
    }

    /// Returns the session identifier.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// Assigns the session identifier during registration.
    pub(crate) fn set_id(&mut self, id: SessionId) {
        self.id = id;
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

    /// Returns the cheap status view (no digest, no memory copy).
    pub fn status(&self) -> Result<MachineStatus, SessionError> {
        self.ensure_open()?;
        self.backend.status().map_err(SessionError::Backend)
    }

    /// Returns the canonical state digest explicitly.
    pub fn state_hash(&self) -> Result<StateHash, SessionError> {
        self.ensure_open()?;
        self.backend.state_hash().map_err(SessionError::Backend)
    }

    /// Returns the backend's default inspection memory window metadata.
    pub fn inspection_memory_window(&self) -> (u64, usize) {
        self.backend.inspection_memory_window()
    }

    /// Produces an inspection view with an opt-in memory window.
    pub fn inspect_with_memory(
        &self,
        base: u64,
        length: usize,
    ) -> Result<MachineInspection, SessionError> {
        self.ensure_open()?;
        self.backend
            .inspect_with_memory(base, length)
            .map_err(SessionError::Backend)
    }

    /// Produces an inspection view.
    pub fn inspect(&self) -> Result<SessionInspection, SessionError> {
        self.ensure_open()?;
        let machine = self.backend.inspect()?;
        let state_hash = self.backend.state_hash()?;
        Ok(SessionInspection {
            id: self.id,
            state: self.state,
            lineage: self.lineage.clone(),
            backend: self.descriptor().clone(),
            machine,
            state_hash,
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
        // A step can change the payload size, so the previous observation is no longer a
        // bound: the byte preflight must fall back to the documented capture bound.
        self.state_observed = false;
        match self.backend.step(input) {
            Ok(outcome) => Ok(outcome),
            Err(BackendError::Halted) => Err(SessionError::Halted(self.id)),
            Err(error) => Err(SessionError::Backend(error)),
        }
    }

    /// Runs one deterministic batch and returns its bounded outcome.
    ///
    /// The batch is composed over the single backend primitive (`Backend::run_batch`);
    /// no host time enters the loop. A caller that needs a digest passes a
    /// [`VerificationPolicy`] that produces one, or asks for the canonical digest after
    /// the run.
    ///
    /// # Errors
    ///
    /// Returns the session's lifecycle rejection or the backend's typed step/limit error.
    pub fn run_until(
        &mut self,
        stop: &StopSet,
        steps: u64,
        inputs: &InputSequence,
        policy: VerificationPolicy,
        control: &RunControl,
    ) -> Result<BatchOutcome, SessionError> {
        self.ensure_open()?;
        if self.state != SessionState::Ready {
            return Err(SessionError::NotRunning {
                session: self.id,
                state: self.state,
            });
        }
        self.state_observed = false;
        match self
            .backend
            .run_batch(inputs, StopBudget::Steps(steps), stop, policy, control)
        {
            Ok(outcome) => Ok(outcome),
            Err(BackendError::Halted) => Err(SessionError::Halted(self.id)),
            Err(error) => Err(SessionError::Backend(error)),
        }
    }

    /// Runs one deterministic cycle-budgeted batch with neutral input.
    ///
    /// This is a thin composition over [`Self::run_until`]'s one backend primitive with
    /// [`StopBudget::Cycles`]; it is not a second execution loop.
    ///
    /// # Errors
    ///
    /// As [`Self::run_until`].
    pub fn run_cycles(
        &mut self,
        cycles: u64,
        stop: &StopSet,
        policy: VerificationPolicy,
        control: &RunControl,
    ) -> Result<BatchOutcome, SessionError> {
        self.ensure_open()?;
        if self.state != SessionState::Ready {
            return Err(SessionError::NotRunning {
                session: self.id,
                state: self.state,
            });
        }
        self.state_observed = false;
        let inputs = InputSequence::neutral();
        match self
            .backend
            .run_batch(&inputs, StopBudget::Cycles(cycles), stop, policy, control)
        {
            Ok(outcome) => Ok(outcome),
            Err(BackendError::Halted) => Err(SessionError::Halted(self.id)),
            Err(error) => Err(SessionError::Backend(error)),
        }
    }

    /// Advances one quantum through the backend's verified path.
    ///
    /// A consumer that needs a per-step digest (movie recording, replay comparison,
    /// experiment capture) uses this instead of reading an ordinary-outcome digest,
    /// so the guarantee comes from the backend contract and not from a stale
    /// assumption.
    pub fn step_verified(
        &mut self,
        input: InputFrame,
    ) -> Result<VerifiedStepOutcome, SessionError> {
        self.ensure_open()?;
        if self.state != SessionState::Ready {
            return Err(SessionError::NotRunning {
                session: self.id,
                state: self.state,
            });
        }
        self.state_observed = false;
        match self.backend.step_verified(input) {
            Ok(outcome) => Ok(outcome),
            Err(BackendError::Halted) => Err(SessionError::Halted(self.id)),
            Err(error) => Err(SessionError::Backend(error)),
        }
    }

    /// Captures a checkpoint and stores it in this session.
    ///
    /// No quota is applied: this is the session layer's own primitive, used by the
    /// player, the movie writer and the session-level tests. A service that owns
    /// user-facing quotas calls [`Self::snapshot_within`], which checks the quota
    /// **before** the checkpoint is stored.
    pub fn snapshot(&mut self) -> Result<SnapshotId, SessionError> {
        self.snapshot_within(usize::MAX, usize::MAX)
    }

    /// Captures a checkpoint when it fits the caller's per-session quota.
    ///
    /// The quota is checked before the checkpoint is stored (ADR-0016 §3, race class
    /// "never partially applies"): the count is compared before any allocation, and
    /// the retained size of the captured record is compared before it is inserted.
    /// A rejection therefore leaves this session's state and its stored checkpoints
    /// exactly as they were — the identifier counter is not even advanced.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::LimitExceeded`] when this session already holds
    /// `max_snapshots` checkpoints or the checkpoint would push its retained
    /// checkpoint bytes above `max_bytes`, and otherwise the lifecycle and backend
    /// errors of [`Self::snapshot`].
    pub fn snapshot_within(
        &mut self,
        max_snapshots: usize,
        max_bytes: usize,
    ) -> Result<SnapshotId, SessionError> {
        self.ensure_open()?;
        let count = self.snapshots.len();
        let retained = self.snapshot_bytes_retained();
        if count >= max_snapshots {
            return Err(SessionError::LimitExceeded {
                quota: SessionQuota::Snapshots,
                limit: max_snapshots,
                current: count,
            });
        }
        // **Preflight before allocation.** The bound costs nothing: the exact observed
        // size while the state is unchanged, otherwise the bound the backend declares.
        // A record that cannot fit — or a capture that cannot be bounded at all while a
        // finite quota applies — is refused here, before any state is captured or
        // serialized (ADR-0016 §3, C4).
        match self.payload_bound() {
            Some(payload_bound) => {
                let predicted = self.record_bytes_for(payload_bound);
                if retained.saturating_add(predicted) > max_bytes {
                    return Err(SessionError::LimitExceeded {
                        quota: SessionQuota::Bytes,
                        limit: max_bytes,
                        current: retained.saturating_add(predicted),
                    });
                }
            }
            // No quota is in force, so an unbounded capture cannot exceed it: the
            // unbounded path (movie, player, rewind) keeps working for every backend.
            None if max_bytes == usize::MAX => {}
            None => {
                // The state changed and the backend declares no payload bound: the quota
                // cannot be preflighted, so the capture is refused instead of attempted
                // (fail-closed). The error names the missing declaration, not an exceeded
                // limit: reporting "would reach N, above the limit of M" here was
                // misleading because N is below M.
                return Err(SessionError::UnboundedSnapshot {
                    limit: max_bytes,
                    retained,
                });
            }
        }
        let backend = self.backend.snapshot()?;
        let payload = backend.payload().len();
        let id = self.allocate_snapshot_id()?;
        let snapshot = Snapshot::new(id, self.id, backend);
        let record = snapshot.to_bytes()?.len();
        let used = retained.saturating_add(record);
        if used > max_bytes {
            // The payload exceeded even the declared bound. Nothing is inserted, the
            // session state is untouched, and only the identifier counter moved (which
            // allocates no memory), so an over-budget checkpoint is never retained.
            self.next_snapshot_id = id.raw();
            return Err(SessionError::LimitExceeded {
                quota: SessionQuota::Bytes,
                limit: max_bytes,
                current: used,
            });
        }
        self.store_snapshot(id, snapshot, record);
        self.state_bytes = payload;
        self.state_observed = true;
        self.last_record_bytes = Some(record);
        self.last_record_payload = payload;
        Ok(id)
    }

    /// Returns the number of checkpoints this session currently stores.
    #[must_use]
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Returns the serialized size of one stored checkpoint.
    ///
    /// This is the accounting unit of `max_bytes_per_session`: the bytes a checkpoint
    /// actually retains, which is also exactly what `session.snapshot` reports as
    /// `bytes_length`.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnknownSnapshot`] for a checkpoint this session does
    /// not store.
    pub fn snapshot_record_bytes(&self, id: SnapshotId) -> Result<usize, SessionError> {
        self.record_bytes
            .get(&id)
            .copied()
            .ok_or(SessionError::UnknownSnapshot {
                session: self.id,
                snapshot: id,
            })
    }

    /// Returns the total serialized size of every stored checkpoint.
    ///
    /// The sizes are the ones recorded when each checkpoint was stored, so deciding the
    /// quota never re-serializes the retained records.
    #[must_use]
    pub fn snapshot_bytes_retained(&self) -> usize {
        self.record_bytes
            .values()
            .fold(0, |total, bytes| total.saturating_add(*bytes))
    }

    /// Drops every stored checkpoint and reports the bytes released.
    ///
    /// This is what makes `close` literal (ADR-0016 §3, K9/V3-2): closing a session
    /// releases its checkpoint memory immediately instead of holding it until the
    /// entry is reaped, so a `create -> close` loop cannot accumulate closed state.
    /// The session object itself stays alive through any `Arc` a reader already
    /// holds, and its lifecycle state is unchanged.
    pub fn release_snapshot_records(&mut self) -> usize {
        let released = self.snapshot_bytes_retained();
        self.snapshots.clear();
        self.record_bytes.clear();
        released
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
        // The restored state is exactly this checkpoint's payload, so the byte-quota
        // prediction is exact again after a restore.
        self.state_bytes = snapshot.backend().payload().len();
        self.state_observed = true;
        Ok(())
    }

    /// Restores the state captured when the session was created.
    pub fn reset(&mut self) -> Result<(), SessionError> {
        self.ensure_open()?;
        self.backend.restore(&self.initial_state)?;
        self.state_bytes = self.initial_state.payload().len();
        self.state_observed = true;
        Ok(())
    }

    /// Captures a bounded structured trace from this session's backend.
    ///
    /// This is the single session-level capture entry point: it delegates to
    /// [`chronofork_trace::capture()`] over the session's backend and advances the
    /// session by the executed steps. Capture does not allocate a snapshot
    /// identity, does not change lineage and never modifies canonical state
    /// through any path other than `Backend::step`.
    ///
    /// The session must be [`SessionState::Ready`]; a paused or closed session is
    /// rejected without advancing the backend.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::NotRunning`] or [`SessionError::Closed`] for a
    /// session that cannot execute, and [`SessionError::Trace`] for a typed trace
    /// or backend failure.
    pub fn capture_trace(
        &mut self,
        config: &TraceConfig,
        steps: u64,
    ) -> Result<Trace, SessionError> {
        self.ensure_open()?;
        if self.state != SessionState::Ready {
            return Err(SessionError::NotRunning {
                session: self.id,
                state: self.state,
            });
        }
        let trace = capture(self.backend.as_mut(), config, steps).map_err(SessionError::Trace)?;
        // The trace advanced the machine, so the previous payload observation is no
        // longer a bound for the byte preflight.
        self.state_observed = false;
        Ok(trace)
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
            .map_err(map_backend_compatibility)?;
        self.state_bytes = state.payload().len();
        self.state_observed = true;
        Ok(())
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
        self.record_bytes.clear();
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
        self.backend.write_memory(address, data)?;
        // A debug write changes canonical state, so the payload-size observation is no
        // longer a bound for the next capture.
        self.state_observed = false;
        Ok(())
    }

    pub(crate) fn adopt_snapshot(&mut self, snapshot: Snapshot) -> Result<(), SessionError> {
        let next = snapshot
            .id()
            .raw()
            .checked_add(1)
            .ok_or(SessionError::IdSpaceExhausted)?;
        self.next_snapshot_id = self.next_snapshot_id.max(next);
        let id = snapshot.id();
        let record = snapshot.to_bytes()?.len();
        self.store_snapshot(id, snapshot, record);
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
    use chronofork_backend_api::{Backend, BackendFactory, BackendId, BackendSnapshot};
    use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash};
    use chronofork_test_backend::SyntheticBackendFactory;

    use super::{Session, SessionQuota};
    use crate::error::SessionError;
    use crate::lineage::Lineage;
    use crate::snapshot::Snapshot;

    fn session() -> Session {
        let factory = SyntheticBackendFactory::default();
        let backend = factory.create().expect("backend");
        let initial = backend.snapshot().expect("initial snapshot");
        Session::new(backend, Lineage::root(), initial)
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
    fn a_capture_after_a_state_change_uses_the_backend_declared_bound() {
        let mut session = session();
        session
            .snapshot_within(usize::MAX, usize::MAX)
            .expect("first capture");
        let retained = session.snapshot_bytes_retained();
        assert!(retained > 0, "the first checkpoint retains bytes");

        // A step can change the payload size, so the exact size is no longer a bound: the
        // preflight uses the bound the synthetic backend declares (its payload size is
        // fixed by its format) and refuses a budget that cannot hold it *before* the
        // capture, instead of capturing first and rejecting afterwards.
        let _ = session.step(InputFrame::NEUTRAL).expect("step");
        let error = session
            .snapshot_within(usize::MAX, retained.saturating_add(1))
            .expect_err("the changed state is not bounded by the old size");
        assert!(
            matches!(
                error,
                SessionError::LimitExceeded {
                    quota: SessionQuota::Bytes,
                    ..
                }
            ),
            "expected a byte-quota rejection, got {error}"
        );
        assert_eq!(session.snapshot_count(), 1, "nothing was added");

        // A budget that holds the declared bound admits the capture.
        let id = session
            .snapshot_within(usize::MAX, retained.saturating_mul(2).saturating_add(4096))
            .expect("the bounded capture fits");
        assert_eq!(session.snapshot_count(), 2);
        let _ = id;
    }

    #[test]
    fn the_retained_byte_accounting_comes_from_the_stored_record_sizes() {
        let mut session = session();
        let first = session
            .snapshot_within(usize::MAX, usize::MAX)
            .expect("first capture");
        let second = session
            .snapshot_within(usize::MAX, usize::MAX)
            .expect("second capture");
        // The cached sizes are exactly the serialized record sizes, and the retained
        // total is their sum — deciding a quota never re-serializes them.
        let mut total = 0;
        for id in [first, second] {
            let cached = session.snapshot_record_bytes(id).expect("cached size");
            let serialized = session.snapshot_bytes(id).expect("serialized");
            assert_eq!(cached, serialized.len());
            total += cached;
        }
        assert_eq!(session.snapshot_bytes_retained(), total);
    }

    /// A backend whose payload grows after a step and which declares **no** bound.
    ///
    /// This is the shape the R1c review found: the session layer must not assume a bound
    /// the backend does not guarantee (C4).
    struct UnboundedBackend {
        inner: Box<dyn chronofork_backend_api::Backend>,
        steps: usize,
    }

    impl chronofork_backend_api::Backend for UnboundedBackend {
        fn descriptor(&self) -> &chronofork_backend_api::BackendDescriptor {
            self.inner.descriptor()
        }
        fn state_hash(&self) -> Result<StateHash, chronofork_backend_api::BackendError> {
            self.inner.state_hash()
        }
        fn step(
            &mut self,
            input: InputFrame,
        ) -> Result<chronofork_backend_api::StepOutcome, chronofork_backend_api::BackendError>
        {
            self.steps += 1;
            self.inner.step(input)
        }
        fn snapshot(&self) -> Result<BackendSnapshot, chronofork_backend_api::BackendError> {
            let snapshot = self.inner.snapshot()?;
            if self.steps == 0 {
                return Ok(snapshot);
            }
            let mut payload = snapshot.payload().to_vec();
            payload.resize(payload.len() + 4096, 0);
            Ok(BackendSnapshot::new(
                snapshot.backend_id().clone(),
                snapshot.backend_version(),
                snapshot.format_version(),
                snapshot.tick(),
                snapshot.config_hash(),
                snapshot.state_hash(),
                payload,
            ))
        }
        fn restore(
            &mut self,
            snapshot: &BackendSnapshot,
        ) -> Result<(), chronofork_backend_api::BackendError> {
            self.inner.restore(snapshot)
        }
        fn inspect(
            &self,
        ) -> Result<chronofork_backend_api::MachineInspection, chronofork_backend_api::BackendError>
        {
            self.inner.inspect()
        }
        fn read_memory(
            &self,
            address: u64,
            length: usize,
        ) -> Result<Vec<u8>, chronofork_backend_api::BackendError> {
            self.inner.read_memory(address, length)
        }
        fn write_memory(
            &mut self,
            address: u64,
            data: &[u8],
        ) -> Result<(), chronofork_backend_api::BackendError> {
            self.inner.write_memory(address, data)
        }
    }

    #[test]
    fn an_unbounded_backend_cannot_be_preflighted_and_is_refused() {
        let factory = SyntheticBackendFactory::default();
        let backend = UnboundedBackend {
            inner: factory.create().expect("backend"),
            steps: 0,
        };
        let initial = backend.snapshot().expect("initial snapshot");
        let mut session = Session::new(Box::new(backend), Lineage::root(), initial);
        let _ = session.step(InputFrame::NEUTRAL).expect("step");
        // A finite quota applies and the backend declares no bound: the capture is
        // refused before it is attempted instead of relying on an assumed constant.
        let error = session
            .snapshot_within(usize::MAX, 64 * 1024)
            .expect_err("the changed state has no bound");
        assert!(
            matches!(
                error,
                SessionError::UnboundedSnapshot {
                    limit: 65_536,
                    retained: 0
                }
            ),
            "the rejection names the missing declaration, not an exceeded limit: {error}"
        );
        assert!(
            error
                .to_string()
                .contains("declares no snapshot payload bound"),
            "the message must say why the capture was refused: {error}"
        );
        assert_eq!(session.snapshot_count(), 0);
        // With no quota in force nothing can be exceeded, so the unbounded path (movie,
        // player, rewind) keeps working for every backend.
        session
            .snapshot_within(usize::MAX, usize::MAX)
            .expect("the unbounded path is not refused");
        assert_eq!(session.snapshot_count(), 1);
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

    #[test]
    fn capture_trace_does_not_perturb_snapshot() {
        use std::sync::Arc;

        use chronofork_core::InputFrame;
        use chronofork_test_backend::SyntheticBackendFactory;
        use chronofork_trace::{TraceConfig, TraceStopReason};

        use crate::manager::SessionManager;
        use crate::movie::{Movie, MovieMeta};

        let factory = SyntheticBackendFactory::default();
        let mut traced = SessionManager::new(Arc::new(factory.clone()));
        let mut plain = SessionManager::new(Arc::new(factory));
        let traced_session = traced.create_session().expect("traced session");
        let plain_session = plain.create_session().expect("plain session");
        let config = TraceConfig::default();

        let trace = traced
            .capture_trace(traced_session, &config, 5)
            .expect("capture");
        for _ in 0..5 {
            plain
                .step(plain_session, InputFrame::NEUTRAL)
                .expect("plain step");
        }

        assert_eq!(trace.record_count(), 5);
        assert_eq!(trace.meta().stop_reason, TraceStopReason::StepLimit);
        assert!(trace.meta().triggered);
        assert!(!trace.meta().events_observed);

        let traced_view = traced.inspect(traced_session).expect("traced inspection");
        let plain_view = plain.inspect(plain_session).expect("plain inspection");
        assert_eq!(traced_view.machine.tick, plain_view.machine.tick);
        assert_eq!(traced_view.machine.tick.raw(), 5);
        assert_eq!(
            traced.state_hash(traced_session).expect("traced digest"),
            plain.state_hash(plain_session).expect("plain digest")
        );
        assert_eq!(
            traced_view.machine.registers(),
            plain_view.machine.registers()
        );
        let traced_memory = traced
            .inspect_with_memory(traced_session, 0, 256)
            .expect("traced memory");
        let plain_memory = plain
            .inspect_with_memory(plain_session, 0, 256)
            .expect("plain memory");
        assert_eq!(traced_memory.memory, plain_memory.memory);

        let traced_id = traced.snapshot(traced_session).expect("traced snapshot");
        let plain_id = plain.snapshot(plain_session).expect("plain snapshot");
        let traced_bytes = traced
            .snapshot_bytes(traced_session, traced_id)
            .expect("traced bytes");
        let plain_bytes = plain
            .snapshot_bytes(plain_session, plain_id)
            .expect("plain bytes");
        assert_eq!(
            traced_bytes, plain_bytes,
            "tracing must not change serialized snapshot bytes"
        );

        let meta = MovieMeta::new(7, Some("capture-non-perturbation".to_owned()), None)
            .expect("movie meta");
        let inputs = [InputFrame::NEUTRAL; 4];
        let traced_movie = Movie::record(&mut traced, traced_session, meta.clone(), &inputs)
            .expect("traced movie");
        let plain_movie =
            Movie::record(&mut plain, plain_session, meta, &inputs).expect("plain movie");
        assert_eq!(
            traced_movie.to_bytes().expect("traced movie bytes"),
            plain_movie.to_bytes().expect("plain movie bytes"),
            "a movie recorded after tracing must be byte-identical"
        );

        traced.pause(traced_session).expect("pause");
        let error = traced
            .capture_trace(traced_session, &config, 1)
            .expect_err("a paused session cannot capture");
        assert!(matches!(
            error,
            SessionError::NotRunning {
                session,
                state: crate::session::SessionState::Paused,
            } if session == traced_session
        ));

        let unknown = traced
            .capture_trace(chronofork_core::SessionId::from_raw(9_999), &config, 1)
            .expect_err("unknown session");
        assert!(matches!(unknown, SessionError::UnknownSession(_)));
    }
}
