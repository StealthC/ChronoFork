//! The application service: operation dispatch, ownership and synchronization.
//!
//! `SessionManager` stays synchronous; this type is the single documented
//! synchronization boundary and now owns **one execution owner per session**.
//! A long `session.run` holds only its target session's owner, so independent
//! sessions execute in parallel and control/read operations on other sessions
//! never wait for it. See ADR-0016 §1 and architecture §7.
//!
//! Lock discipline, in acquisition order:
//!
//! 1. the per-session writer lease table (`leases`), held only for a lookup/insert
//!    or a removal, so a second mutation of the same session fails immediately
//!    instead of blocking;
//! 2. the ownership registry (`registry`), held only to look up a handle/owner or
//!    to insert/update one entry, and **never while waiting for a session owner**;
//! 3. the session owner (`SessionHandle`), taken after the registry has been
//!    released, and never held while the registry is re-taken;
//! 4. the session layer's own lock (`sessions`), taken only by `create`/`fork` for
//!    construction and identifier allocation, and never while a session owner of a
//!    *different* session is held; `compare` (the only two-owner operation) takes
//!    no session-layer lock.
//!
//! Invariants this module maintains (ADR-0016 §1, K5/K6):
//!
//! - **I1:** the registry lock is never held while waiting for a session owner, and
//!   a session owner is never acquired while the registry is held;
//! - **I2:** the registry is the only holder of a session owner this service uses;
//!   the private session layer is a factory and an identifier source, never an
//!   execution path, so no other code can wait on a session owner;
//! - **I3:** admission (`max_sessions`) is one atomic check-and-insert under the
//!   registry lock and never waits for a run;
//! - **I4:** `compare` takes the two session owners in identifier order, and
//!   `create`/`fork` take the session-layer lock **before** inserting into the
//!   registry, so no cycle exists between the two locks;
//! - **I5:** `list` never takes a session owner at all: it reports cheap metadata
//!   plus a `running` marker, so it cannot wait for a run (ADR-0016 §1, K1/K8).
//!   `inspect` is the exact-state read and takes only its own session's owner.
//!
//! Reads never take a writer lease, so they are never rejected with `conflict`.
//!
//! W3.2 adds the control path (ADR-0016 §2): a lock-free cancel signal, a host
//! deadline, progress read from the operation journal, and a bounded reconciliation
//! horizon. The control path (`session.cancel`, `session.progress`,
//! `session.operation`) reads and writes **only** the operation journal — never a
//! session owner — so it is served while a run holds its session, which is what
//! makes the reserved worker capacity in ADR-0016 §1 effective.
//!
//! W3.3 adds the lifecycle (ADR-0016 §3):
//!
//! - **Per-session quotas are strict and checked before the allocation**:
//!   `max_snapshots_per_session` and `max_bytes_per_session` are enforced by the
//!   session layer under this service's owner, and a rejected capture leaves the
//!   session, its checkpoints and the reported accounting untouched;
//! - **`close` releases immediately**: it drops the session's checkpoint records and
//!   reports `released_checkpoint_bytes`, then hands the session off from the private
//!   session layer (`detach_session`) so the registry's handle is the last owner. A
//!   `create -> close` loop therefore cannot accumulate closed checkpoint or backend
//!   memory;
//! - **`reap` removes entries** (`session.reap`, authorized like `close`): it reports
//!   the reaped ids and the registry accounting, a removed session answers
//!   `not_found` afterwards, an entry with a live owner is skipped rather than waited
//!   on, and the `Arc` handle keeps an in-flight reader safe;
//! - **closed entries are bounded independently of the live count**:
//!   `max_closed_retained` and `closed_retention_ttl` force cleanup at admission, so a
//!   low live count cannot hide unbounded closed state;
//! - **the global store-byte quota is declared best-effort** (ADR-0016 §3): the
//!   committed checkpoint bytes are accounted and reported by `session.reap`
//!   (`retained_bytes`, `max_store_bytes`, `max_store_bytes_best_effort`), but the
//!   bound is not an atomic cross-key gate — a strict one would need a reservation
//!   taken while a session owner is held, which would invert the lock order of I1.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use chronofork_backend_api::{BackendDescriptor, BackendFactory};
use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick, hash_bytes};
use chronofork_sessions::{
    Lineage, SessionHandle, SessionInspection, SessionManager, SessionState,
};

use crate::auth::Principal;
use crate::catalog::{Args, CATALOG, OpKind, OpSpec};
use crate::error::{ErrorCode, ServiceError, WIRE_VERSION};
use crate::json::JsonValue;
use crate::limits::ServiceLimits;
use crate::runtime::{
    Admission, CancelFlag, Deadline, Disposition, OperationJournal, OperationProgress,
    OperationState, ProcessEpoch, expired_outcome, journal_full, key_conflict, key_in_flight, now,
    unjournaled_key, unknown_outcome,
};
use crate::tool::ToolSpec;
use crate::values::{descriptor_json, hex, optional_name, optional_unsigned, unsigned};
use crate::version::ApiVersion;

/// Version reported by `capabilities` and MCP `serverInfo`.
pub const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cancellation is observed once per instruction, so the deadline is sampled on a
/// fixed stride of steps instead of every step.
///
/// Reading a monotonic clock per instruction would put host time on the hot path
/// for no benefit: the contract is that the deadline chooses how many steps are
/// applied (ADR-0016 §2, K3), and the run reports the exact `performed` count it
/// reached. A stride of 64 keeps the sampled point within one 64-step batch of the
/// true deadline while leaving cancellation checked at every boundary.
const DEADLINE_SAMPLE_STRIDE: u64 = 64;

/// Progress published into the journal on this stride of steps.
///
/// The publish is a plain integer write under the journal lock; a reader sees a
/// value at most this many steps behind, which is well inside the cost of one
/// `session.progress` round trip.
const PROGRESS_PUBLISH_STRIDE: u64 = 64;

/// Why one `session.run` stopped.
///
/// The order is the total order of ADR-0016 §New limits: among conditions observed
/// at the same boundary `cancelled` beats `deadline` beats `halted`. The
/// `stop_condition` variant is the declared slot for the W3.4 stop grammar and is
/// reported by this build only when a caller-supplied condition stops a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StopReason {
    /// The requested step budget completed.
    Completed,
    /// The machine halted.
    Halted,
    /// An explicit `session.cancel` interrupted the run.
    Cancelled,
    /// The host wall-clock deadline expired.
    Deadline,
    // `stop_condition` is the remaining value of the ADR-0016 §New limits total
    // order, introduced by the `stop_on` grammar of W3.4. It is deliberately absent
    // here: this build cannot produce it, and advertising a stop reason no request
    // can reach would be a capability claim without a test behind it.
}

impl StopReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Halted => "halted",
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline",
        }
    }
}

/// Per-request control state a dispatched operation may publish into.
///
/// It is what makes `cancel` and `progress` cheap: both are read through the
/// journal, and the cancel signal is a shared atomic the run loop polls, so neither
/// ever waits for the session owner the run holds.
pub(crate) struct OperationContext<'a> {
    /// Journal key of this request, when it was registered.
    key: Option<&'a str>,
    /// Live cancellation signal of this attempt.
    cancel: &'a Arc<CancelFlag>,
    /// The bounded operation journal.
    journal: &'a Mutex<OperationJournal>,
    /// Set once the operation's work actually began.
    ///
    /// A dispatch that fails before this flag is set (a rejected precondition, a
    /// contended lease, an authorization failure) abandons its journal entry, so a
    /// retry with the same request key re-executes instead of replaying a failure
    /// for an operation the server never started.
    started: AtomicBool,
}

impl OperationContext<'_> {
    /// Returns the registered journal key, when this request owns one.
    fn key(&self) -> Option<&str> {
        self.key
    }

    /// Returns the live cancellation signal of this attempt.
    fn cancel(&self) -> &Arc<CancelFlag> {
        self.cancel
    }

    /// Marks that this operation's work has begun.
    pub(crate) fn begin(&self) {
        self.started.store(true, Ordering::Release);
    }

    /// Returns whether this operation's work began.
    fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    /// Publishes progress for this request, when it is journaled.
    fn publish_progress(&self, performed: u64, tick: Option<Tick>) {
        let Some(key) = self.key() else {
            return;
        };
        if let Ok(mut journal) = self.journal.lock() {
            journal.publish_progress(key, performed, tick);
        }
    }
}

/// A host-clock deadline started when a run is admitted.
///
/// The budget is reflected in the result; the elapsed time is deliberately not,
/// because a host duration is not part of a reproducible document and the applied
/// prefix is already reported as `performed` — the authority of ADR-0016 §2.
struct RunDeadline {
    deadline: Deadline,
}

impl RunDeadline {
    fn new(budget_ms: u64) -> Self {
        Self {
            deadline: Deadline::after_millis(budget_ms),
        }
    }

    fn expired(&self) -> bool {
        self.deadline.is_expired()
    }
}

/// Configuration for one service instance.
pub struct ServiceConfig {
    /// Backend family used for every session this instance creates.
    pub factory: Arc<dyn BackendFactory>,
    /// Optional durable storage root.
    pub storage_root: Option<PathBuf>,
    /// Optional content-addressed experiment artifact root.
    pub artifacts_root: Option<PathBuf>,
    /// Resource limits applied to every operation.
    pub limits: ServiceLimits,
}

impl fmt::Debug for ServiceConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceConfig")
            .field("backend", self.factory.descriptor().id())
            .field("storage_root", &self.storage_root)
            .field("artifacts_root", &self.artifacts_root)
            .field("limits", &self.limits)
            .finish()
    }
}

/// Committed metadata of one session, as of its last completed mutating operation.
///
/// This is the cheap read source of `list` (ADR-0016 §1, K1/K8): it is written
/// under the session owner and published after that owner is released, so `list`
/// can report every session without taking a session owner and therefore without
/// ever waiting for a run.
#[derive(Clone)]
struct SessionMetadata {
    /// Lifecycle state at the last commit.
    state: SessionState,
    /// Tick at the last commit.
    tick: Tick,
    /// Canonical state digest at the last commit.
    state_hash: StateHash,
    /// Fork provenance at the last commit.
    lineage: Lineage,
    /// Serialized bytes of the checkpoints retained at the last commit.
    ///
    /// The session layer owns the number (it knows a checkpoint's size) and the
    /// service keeps the committed copy, so a quota rejection and the account it is
    /// checked against cannot drift apart. `close` publishes zero here when it
    /// releases the checkpoint memory, which is what the best-effort global bound
    /// reports (ADR-0016 §3).
    snapshot_bytes: usize,
}

/// One live-session slot reserved before the mutation allocates anything.
///
/// The guard is the RAII half of admission: [`AppService::admit_reserved`] consumes it
/// with the session record, and any early return — a rejected precondition, a backend
/// that fails to build, a refused child — drops it and releases the slot. A reservation
/// that is still held counts against `max_sessions`, so the bound holds even while
/// several admissions are in flight.
struct SessionReservation<'a> {
    registry: &'a Mutex<Registry>,
    armed: bool,
}

impl SessionReservation<'_> {
    /// Marks the reservation as satisfied by an admitted session.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for SessionReservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut registry) = self.registry.lock() {
            registry.reserved = registry.reserved.saturating_sub(1);
        }
    }
}

/// A built-but-not-yet-admitted session.
///
/// The guard is the RAII half of `create`: the session layer owns the backend as soon as
/// the session is built, so *every* early return between the build and the registry insert
/// — a contended owner, a failed inspection, a metadata error — must remove it or the
/// backend stays alive invisibly. `disarm` hands the session over once it is registered.
struct PendingSession<'a> {
    service: &'a AppService,
    session: Option<SessionId>,
}

impl<'a> PendingSession<'a> {
    const fn new(service: &'a AppService, session: SessionId) -> Self {
        Self {
            service,
            session: Some(session),
        }
    }

    /// Marks the session as admitted: dropping the guard no longer removes it.
    fn disarm(mut self) {
        self.session = None;
    }
}

impl Drop for PendingSession<'_> {
    fn drop(&mut self) {
        if let Some(session) = self.session {
            self.service.detach_session(session);
        }
    }
}

/// Service-level metadata for one registered session.
///
/// The entry is deliberately cheap: it carries the execution owner, the admission
/// fields and the last committed metadata. Session state is never read through this
/// entry by a mutating path, and the committed copy is only ever a bounded,
/// documented step behind a running session.
struct SessionRecord {
    /// Principal name recorded when the session was created.
    owner: String,
    /// Whether `session.close` has been applied.
    closed: bool,
    /// The session's execution owner; this service is its only owner (I2).
    ///
    /// `None` once the session is closed: closing drops the registry's reference so
    /// the backend and checkpoint memory are released **at close** (ADR-0016 §3,
    /// K9/V3-2), while the entry itself stays as a tombstone so a later `inspect`
    /// reports `closed` (not `not_found`) until it is reaped. An `Arc` a reader
    /// already holds keeps that reader safe; the service holds none.
    handle: Option<SessionHandle>,
    /// Committed metadata published by the last completed operation.
    committed: SessionMetadata,
    /// Monotonic revision of this session.
    ///
    /// The revision is the precondition token a retry uses: `restore` can move the tick
    /// backwards and two writes can share one tick, so a tick is not a revision
    /// (ADR-0016 §2, K3/race R6). It advances once at every committed mutation of the
    /// session — `run` when it applied steps, `restore` and `close` — so an ABA
    /// (snapshot/restore) round trip still changes it and a retry pinned to the earlier
    /// revision is refused. A forked child starts at zero and publishes that value.
    revision: u64,
    /// Host time `session.close` was applied, or `None` while the session is live.
    ///
    /// It is the `closed_retention_ttl` clock of ADR-0016 §3. It never enters a
    /// canonical document: it only decides when a closed entry is eligible for
    /// forced cleanup, and the cleanup itself is observable through the generation.
    closed_at: Option<SystemTime>,
}

impl SessionRecord {
    /// Returns whether this record is closed and older than the retention TTL.
    fn retention_expired(&self, now: SystemTime, ttl_secs: u64) -> bool {
        let Some(closed_at) = self.closed_at else {
            return false;
        };
        if ttl_secs == 0 {
            return true;
        }
        now.duration_since(closed_at).unwrap_or_default()
            >= std::time::Duration::from_secs(ttl_secs)
    }
}

/// The ownership registry: one execution owner and its admission metadata.
#[derive(Default)]
struct Registry {
    records: BTreeMap<SessionId, SessionRecord>,
    /// Slots reserved by an admission that has not built its session yet.
    ///
    /// The reservation is what lets `create` and `fork` decide admission **before** a
    /// backend is constructed or a checkpoint is captured, while still keeping the
    /// check and the insert atomic for concurrent callers (ADR-0016 §3). A reserved slot
    /// counts against `max_sessions` exactly like a live session, so two concurrent
    /// admissions cannot both take the last slot.
    reserved: usize,
}

impl Registry {
    /// Number of admitted slots: live sessions plus in-flight reservations.
    fn admitted_count(&self) -> usize {
        self.live_count().saturating_add(self.reserved)
    }
    /// Number of registered sessions, including closed ones.
    ///
    /// This is the registry size the reported accounting uses; it is **not** the
    /// admission bound. Admission counts [`Self::live_count`] (a closed entry never
    /// occupies a slot), and `session.reap` plus the forced cleanup of
    /// `max_closed_retained`/`closed_retention_ttl` keep the closed part bounded.
    fn registered_count(&self) -> usize {
        self.records.len()
    }

    /// Number of sessions that are not closed, reported by [`AppService::session_count`].
    fn live_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| !record.closed)
            .count()
    }

    /// Number of closed sessions still registered.
    fn closed_count(&self) -> usize {
        self.records.values().filter(|record| record.closed).count()
    }

    /// Total checkpoint bytes every registered session commits to retain.
    ///
    /// This is the accounting unit of the **best-effort** `max_store_bytes` bound
    /// (ADR-0016 §3): it reads the committed metadata the registry already holds, so
    /// the check costs one pass over the registry and no session owner.
    fn retained_bytes(&self) -> usize {
        self.records.values().fold(0, |total, record| {
            total.saturating_add(record.committed.snapshot_bytes)
        })
    }

    /// Returns the sessions eligible for forced cleanup, oldest close first.
    ///
    /// An entry is eligible when `session.close` was applied and either its
    /// `closed_retention_ttl` has passed or the closed count exceeds
    /// `max_closed_retained`. The private bound is what stops a low live count from
    /// hiding unbounded closed state (ADR-0016 §3, K9/V3-2).
    fn reap_candidates(
        &self,
        now: SystemTime,
        max_closed_retained: usize,
        closed_retention_ttl: u64,
    ) -> Vec<SessionId> {
        let mut closed: Vec<(SessionId, SystemTime)> = self
            .records
            .iter()
            .filter(|(_, record)| record.closed)
            .map(|(session, record)| (*session, record.closed_at.unwrap_or(now)))
            .collect();
        // Oldest close first: the entries closest to their retention bound are the
        // ones forced cleanup removes.
        closed.sort_by_key(|(session, closed_at)| (*closed_at, *session));
        let excess = closed.len().saturating_sub(max_closed_retained);
        closed
            .into_iter()
            .enumerate()
            .filter_map(|(index, (session, closed_at))| {
                let over_count = index < excess;
                let expired = self
                    .records
                    .get(&session)
                    .is_some_and(|record| record.retention_expired(now, closed_retention_ttl));
                if expired {
                    let _ = closed_at;
                    Some(session)
                } else if over_count {
                    Some(session)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Removes one record and returns whether it was present.
    fn remove(&mut self, session: SessionId) -> bool {
        self.records.remove(&session).is_some()
    }

    /// Resolves one session's ownership and lifecycle without taking its owner.
    ///
    /// A closed entry has released its execution owner, so an idempotent `close` must be
    /// able to answer without one; this is the ownership check it uses.
    fn authorized_state(
        &self,
        principal: &Principal,
        session: SessionId,
    ) -> Result<bool, ServiceError> {
        let record = self
            .records
            .get(&session)
            .ok_or_else(|| ServiceError::new(ErrorCode::NotFound, "unknown session"))?;
        if !principal.is_admin() && record.owner != principal.name() {
            return Err(ServiceError::new(
                ErrorCode::Forbidden,
                "session belongs to another principal",
            ));
        }
        Ok(record.closed)
    }

    /// Resolves an explicitly named session that is closed and may be reaped.
    ///
    /// A live session is a typed conflict rather than a silent no-op: `reap` removes
    /// only closed entries, so asking it to remove a running one is a request the
    /// server cannot satisfy as stated (the caller closes first).
    fn authorized_closed(
        &self,
        principal: &Principal,
        session: SessionId,
    ) -> Result<(), ServiceError> {
        let record = self
            .records
            .get(&session)
            .ok_or_else(|| ServiceError::new(ErrorCode::NotFound, "unknown session"))?;
        if !principal.is_admin() && record.owner != principal.name() {
            return Err(ServiceError::new(
                ErrorCode::Forbidden,
                "session belongs to another principal",
            ));
        }
        if !record.closed {
            return Err(ServiceError::new(
                ErrorCode::Conflict,
                "session is not closed; only a closed session can be reaped",
            ));
        }
        Ok(())
    }

    /// Returns the committed metadata of every record this principal may observe.
    ///
    /// The handles are deliberately **not** returned: `list` reports this copy and
    /// never takes a session owner, so it cannot wait for a run.
    fn visible_rows(
        &self,
        principal: &Principal,
    ) -> Vec<(SessionId, Option<SessionHandle>, SessionMetadata)> {
        self.records
            .iter()
            .filter(|(_, record)| {
                !record.closed && (principal.is_admin() || record.owner == principal.name())
            })
            .map(|(session, record)| (*session, record.handle.clone(), record.committed.clone()))
            .collect()
    }

    /// Returns one authorized session's execution owner.
    ///
    /// Authorization happens in the same critical section as the lookup, so the
    /// caller can release the registry before it takes the owner or the session
    /// layer. Admission is **not** checked here: reading or mutating an existing
    /// session does not allocate a slot, and `create`/`fork` are the only admission
    /// points (ADR-0016 §3, `max_live_sessions`).
    fn authorized(
        &self,
        principal: &Principal,
        session: SessionId,
    ) -> Result<SessionHandle, ServiceError> {
        let record = self
            .records
            .get(&session)
            .ok_or_else(|| ServiceError::new(ErrorCode::NotFound, "unknown session"))?;
        if !principal.is_admin() && record.owner != principal.name() {
            return Err(ServiceError::new(
                ErrorCode::Forbidden,
                "session belongs to another principal",
            ));
        }
        // A closed session has released its execution owner; the entry is a tombstone,
        // so the operation fails with the lifecycle error rather than `not_found`.
        let handle = record.handle.clone().ok_or_else(|| {
            ServiceError::from(chronofork_sessions::SessionError::Closed(session))
        })?;
        Ok(handle)
    }
}

/// Active per-session writer leases.
#[derive(Debug, Default)]
struct LeaseTable {
    held: BTreeMap<SessionId, u64>,
    next: u64,
}

/// One in-process application service.
pub struct AppService {
    /// Private session-layer factory: `create`/`fork` build sessions here and
    /// immediately hand them to the registry (I2). It is never on the step path.
    sessions: Mutex<SessionManager>,
    registry: Mutex<Registry>,
    leases: Mutex<LeaseTable>,
    /// Bounded reconciliation journal and the live cancel signals (ADR-0016 §2).
    ///
    /// The control path (`cancel`, `progress`, `operation`) takes only this lock, so
    /// it never waits for a session owner. Lock order: this lock is taken **after**
    /// any session owner is released and **never** while one is held.
    journal: Mutex<OperationJournal>,
    /// Monotonic process identifier reported by run/progress/operation results.
    epoch: ProcessEpoch,
    /// Whether `epoch` comes from the durable identity file.
    ///
    /// A durable service reconciles request keys **across restarts**, so a keyed
    /// mutation must name the epoch it was minted under: without it the server cannot
    /// tell a fresh intention from a retry of a mutation an earlier process may have
    /// applied, and it refuses (ADR-0016 §2). An in-process service keeps the
    /// in-process journal as its identity and accepts a key without an epoch.
    epoch_is_durable: bool,
    /// Best-effort global checkpoint-byte counter (ADR-0016 §3).
    ///
    /// It is an atomic, not the registry lock, so a reader can publish it without
    /// inverting the lock order of I1. It is refreshed under the registry lock at
    /// every close, reap, admission and removal, so it stays consistent with the
    /// committed per-session usage; the **bound** it accompanies is declared
    /// best-effort and outside the acceptance gate.
    store_bytes: AtomicUsize,
    /// Sequence used to mint an operation key when a caller did not supply one.
    ///
    /// A run always owns a journal entry, so cancellation can address it without
    /// requiring the caller to invent a key; a caller-supplied key overrides this.
    sequence: AtomicU64,
    factory: Arc<dyn BackendFactory>,
    storage_root: Option<PathBuf>,
    artifacts_root: Option<PathBuf>,
    limits: ServiceLimits,
}

impl fmt::Debug for AppService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppService")
            .field("backend", self.backend().id())
            .field("storage_root", &self.storage_root)
            .field("artifacts_root", &self.artifacts_root)
            .field("limits", &self.limits)
            .finish()
    }
}

/// A held writer lease that is released when it goes out of scope.
struct Lease<'a> {
    service: &'a AppService,
    session: SessionId,
    token: u64,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.service.leases.lock() {
            if leases.held.get(&self.session) == Some(&self.token) {
                leases.held.remove(&self.session);
            }
        }
    }
}

impl AppService {
    /// Creates a service over the configured backend factory.
    ///
    /// # Errors
    ///
    /// Currently infallible; the signature is frozen by ADR-0012 so a future
    /// validation step can be added without a breaking change.
    pub fn new(config: ServiceConfig) -> Result<Self, ServiceError> {
        let epoch = ProcessEpoch::load(config.storage_root.as_deref());
        Self::build(config, epoch, false)
    }

    /// Creates a service whose process epoch is **durable** beside `identity_root`.
    ///
    /// This is the `serve` form of a service (ADR-0016 §2): the identity counter lives in
    /// a file beside the configured token or store root, is created only here, and is
    /// incremented on every start, so a restarted process never reuses an epoch. Keyed
    /// mutations must then carry the epoch they were minted under (`operation_epoch`),
    /// and a retry that names an earlier epoch is an `unknown_outcome` instead of a
    /// re-execution.
    ///
    /// # Errors
    ///
    /// Returns a typed storage failure when the identity file cannot be read, locked or
    /// written: a durable service refuses to start with an unknown identity.
    pub fn new_durable(config: ServiceConfig, identity_root: &Path) -> Result<Self, ServiceError> {
        let path = crate::runtime::epoch_file_beside(identity_root);
        let epoch = ProcessEpoch::load_durable(&path)?;
        Self::build(config, epoch, true)
    }

    fn build(
        config: ServiceConfig,
        epoch: ProcessEpoch,
        epoch_is_durable: bool,
    ) -> Result<Self, ServiceError> {
        let sessions = SessionManager::new(Arc::clone(&config.factory));
        let journal = OperationJournal::new(
            config.limits.max_operation_journal,
            config.limits.operation_ttl,
        );
        Ok(Self {
            sessions: Mutex::new(sessions),
            registry: Mutex::new(Registry::default()),
            leases: Mutex::new(LeaseTable::default()),
            journal: Mutex::new(journal),
            epoch,
            epoch_is_durable,
            store_bytes: AtomicUsize::new(0),
            sequence: AtomicU64::new(0),
            factory: config.factory,
            storage_root: config.storage_root,
            artifacts_root: config.artifacts_root,
            limits: config.limits,
        })
    }

    /// Returns the backend descriptor for this instance.
    #[must_use]
    pub fn backend(&self) -> &BackendDescriptor {
        self.factory.descriptor()
    }

    /// Returns the configured storage root, when durable operations are enabled.
    #[must_use]
    pub fn storage_root(&self) -> Option<&Path> {
        self.storage_root.as_deref()
    }

    /// Returns the configured artifact root, when artifacts are persisted.
    #[must_use]
    pub fn artifacts_root(&self) -> Option<&Path> {
        self.artifacts_root.as_deref()
    }

    /// Returns the applied resource limits.
    #[must_use]
    pub const fn limits(&self) -> &ServiceLimits {
        &self.limits
    }

    /// Returns this instance's process epoch (ADR-0016 §2).
    ///
    /// The epoch is the identity of the process that owns the reconciliation
    /// journal, minted from a counter that only increases. It is deliberately
    /// **not** a member of any result or error document: those must stay
    /// byte-reproducible, and a value that changes with the process would make two
    /// identical request sequences produce different bytes (the service's canonical
    /// documents are reproducible by contract). The contract the client uses is
    /// stated in terms of the journal instead: a request key replays while its entry
    /// is retained in the **same instance** and, beyond that horizon (or after a
    /// restart), the outcome is `unknown_outcome` with no automatic re-execution.
    #[must_use]
    pub const fn process_epoch(&self) -> u64 {
        self.epoch.value()
    }

    /// Returns the number of live in-memory sessions.
    ///
    /// Closed sessions stay registered in the session layer but are not live and are
    /// not counted.
    pub fn session_count(&self) -> Result<usize, ServiceError> {
        Ok(self.lock_registry()?.live_count())
    }

    /// Returns the catalog of machine-callable operations.
    #[must_use]
    pub fn tools(&self) -> Vec<ToolSpec> {
        crate::catalog::tools(&self.limits)
    }

    /// Dispatches one canonical request document from an authenticated principal.
    ///
    /// The request document is `{"op": <name>, "input": {<fields>}}`. A successful call
    /// returns the canonical bytes of `{"op": <name>, "result": {<fields>}}`; every
    /// failure returns a typed [`ServiceError`] whose [`ServiceError::to_canonical_bytes`]
    /// form is the `chronofork.error.v1` envelope. The transport maps the error code to a
    /// status and never rewrites a result field.
    ///
    /// A mutating request may carry `operation_id`, a key the client chose **before**
    /// sending the mutation. While that key is retained in this process epoch the
    /// server answers a retry with the recorded outcome instead of re-apply a
    /// mutation whose result was lost (ADR-0016 §2, K3). The key is optional; a
    /// `session.run` always owns a journal entry under a server-minted key so
    /// `session.cancel` can address it.
    ///
    /// # Errors
    ///
    /// Every documented `ErrorCode` can be returned; no error message contains a host
    /// path, ROM bytes or token material.
    pub fn dispatch(&self, principal: &Principal, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        self.dispatch_versioned(principal, request, ApiVersion::V1)
    }

    /// Dispatches one canonical request document for a declared API wire version.
    ///
    /// The version is the **request's** declaration (the transport reads it from the
    /// wire and defaults to v1). It selects the input surface and the result/error
    /// shape, never the semantics: the same call applies the same mutation on both
    /// wires (ADR-0016 §Compatibility).
    ///
    /// # Errors
    ///
    /// Every documented `ErrorCode` can be returned; a v1 request that names a v2-only
    /// operation or field is rejected **before any work** with `unknown_op` or
    /// `invalid_request`, and no error message contains a host path, ROM bytes or token
    /// material.
    pub fn dispatch_versioned(
        &self,
        principal: &Principal,
        request: &[u8],
        version: ApiVersion,
    ) -> Result<Vec<u8>, ServiceError> {
        let document = crate::json::parse(request, &self.limits)?;
        let (op, input) = request_parts(&document)?;
        if !version.is_v2() && crate::version::is_v2_only_op(op) {
            // The operation does not exist on the v1 wire: refusing it here is what
            // keeps a v1 client from depending on a v2-only shape.
            return Err(ServiceError::new(ErrorCode::UnknownOp, "unknown operation"));
        }
        if !version.is_v2() {
            if let Some(object) = input.as_object() {
                for key in object.keys() {
                    if crate::version::is_v2_only_field(key) {
                        return Err(invalid("unknown field in `input`"));
                    }
                }
            }
        }
        let spec = crate::catalog::find(op)
            .ok_or_else(|| ServiceError::new(ErrorCode::UnknownOp, "unknown operation"))?;
        if !principal.has_scope(spec.scope) {
            return Err(ServiceError::new(
                ErrorCode::Forbidden,
                format!("principal lacks the `{}` scope", spec.scope.as_str()),
            ));
        }
        let args = crate::catalog::validate(spec, input, &self.limits)?;
        // A keyed mutation must agree with this process's identity before anything
        // happens: a key minted under another epoch may already have been applied there,
        // so it is an `unknown_outcome` — never a re-execution (ADR-0016 §2, F2).
        self.check_mutation_epoch(&args)?;
        let digest = request_digest(spec.name, input, &self.limits)?;
        let requested_key = args.operation("operation_id");
        let key = self.operation_key(spec, &args);
        // Register before any work: the journal entry is what `session.cancel` and
        // `session.progress` find, and a retained result is replayed instead of
        // re-applied. Registration happens under the journal lock only — no session
        // owner is taken on this path (ADR-0016 §1, K2/F5).
        let (key, cancel) = match self.register_operation(principal, &args, key, digest)? {
            Disposition::Execute { key, cancel } => (key, cancel),
            // A retained outcome is returned **with its classification**: a recorded
            // failure must reach the caller as the same typed error (and the same
            // transport status) as the first attempt, never as a 200 body.
            Disposition::Replay { outcome, failed } => {
                if failed {
                    let document = crate::json::parse(&outcome, &self.limits)?;
                    return Err(restore_error(&document));
                }
                // The retained outcome is stored in its v2 shape; a v1 retry receives
                // the v1 projection of the same result.
                return project_outcome(op, outcome, version, &self.limits);
            }
        };
        if requested_key.is_some() && key.is_none() {
            return Err(unjournaled_key());
        }
        let context = OperationContext {
            key: key.as_deref(),
            cancel: &cancel,
            journal: &self.journal,
            started: AtomicBool::new(false),
        };
        let outcome = self.execute(spec, principal, &args, &context);
        let (value, failed) = match outcome {
            Ok(result) => {
                let result = if spec.mutating {
                    self.publish_epoch(result, version)?
                } else {
                    result
                };
                (
                    JsonValue::object([("op", JsonValue::string(spec.name)), ("result", result)]),
                    false,
                )
            }
            Err(error) => (error.to_json(), true),
        };
        // An entry whose operation never started (a rejected precondition, a
        // contended lease) leaves nothing to reconcile: dropping it keeps a retry
        // with the same key usable instead of reporting a retained failure for an
        // operation the server never began.
        if key.is_some() && !context.started() {
            self.abandon_operation(key.as_deref());
        }
        let bytes = crate::json::to_canonical_bytes(&value, &self.limits)?;
        self.finish_operation(key.as_deref(), bytes.clone(), failed);
        // The journal always stores the v2 shape; the response is projected to the
        // caller's version (v1 removes exactly the fields v2 added).
        let bytes = if version.is_v2() {
            bytes
        } else {
            project_outcome(op, bytes, version, &self.limits)?
        };
        if failed {
            // The typed error is returned, not the bytes: `wire` re-serializes it, and
            // the document is identical because both go through the one codec.
            return Err(restore_error(&value));
        }
        Ok(bytes)
    }

    /// Returns the operation key for one request, minting one when the caller did not.
    ///
    /// A `session.run` always owns a key so the control path can address it; every
    /// other operation is journaled only when the caller supplied `operation_id`.
    fn operation_key(&self, spec: &OpSpec, args: &Args) -> Option<String> {
        if let Some(key) = args.operation("operation_id") {
            return Some(key.to_string());
        }
        if !spec.mutating || spec.kind != OpKind::Run {
            return None;
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        Some(format!("run-{sequence}"))
    }

    /// Registers a request key in the bounded journal.
    ///
    /// Returns `Disposition::Replay` with the recorded envelope bytes when the key
    /// was already used for the same request and its result is retained, so the
    /// mutation is **not** applied a second time. A key first seen while the original
    /// request still executes is a retryable conflict; the same key with different
    /// arguments is a typed conflict; a full journal is a typed `limit` rejected
    /// **before** any work starts.
    fn register_operation(
        &self,
        principal: &Principal,
        args: &Args,
        key: Option<String>,
        digest: StateHash,
    ) -> Result<Disposition, ServiceError> {
        let Some(key) = key else {
            return Ok(Disposition::Execute {
                key: None,
                cancel: Arc::new(CancelFlag::new()),
            });
        };
        let session = self.observed_session(args);
        let mut journal = self.lock_journal()?;
        match journal.register(&key, principal.name(), session, digest, now()) {
            Ok(Admission::Fresh { cancel, .. }) => Ok(Disposition::Execute {
                key: Some(key),
                cancel,
            }),
            Ok(Admission::Replay { outcome, failed }) => {
                Ok(Disposition::Replay { outcome, failed })
            }
            // The key was used and its result is no longer retained: the server says so
            // and does **not** re-execute the mutation (ADR-0016 §2, V3-3).
            Ok(Admission::Expired) => Err(expired_outcome()),
            Ok(Admission::InFlight) => Err(key_in_flight()),
            Ok(Admission::DigestMismatch) => Err(key_conflict()),
            Err(_) => Err(journal_full()),
        }
    }

    /// Drops one journaled operation without recording an outcome.
    fn abandon_operation(&self, key: Option<&str>) {
        let Some(key) = key else {
            return;
        };
        if let Ok(mut journal) = self.journal.lock() {
            journal.abandon(key);
        }
    }

    /// Marks one journaled operation finished with its canonical envelope bytes.
    fn finish_operation(&self, key: Option<&str>, outcome: Vec<u8>, failed: bool) {
        let Some(key) = key else {
            return;
        };
        if let Ok(mut journal) = self.journal.lock() {
            journal.finish(key, outcome, failed, now());
        }
    }

    /// Returns the session one request names, when it names one.
    fn observed_session(&self, args: &Args) -> Option<SessionId> {
        args.optional_unsigned("session")
            .ok()
            .flatten()
            .map(SessionId::from_raw)
    }

    fn execute(
        &self,
        spec: &OpSpec,
        principal: &Principal,
        args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        match spec.kind {
            OpKind::Capabilities => self.capabilities(),
            OpKind::Create => self.create(principal, args, context),
            OpKind::List => self.list(principal),
            OpKind::Inspect => {
                let session = self.authorized_session(principal, args)?;
                self.inspect(session)
            }
            OpKind::Run => self.run(principal, args, context),
            OpKind::Fork => self.fork(principal, args, context),
            OpKind::Snapshot => self.snapshot(principal, args, context),
            OpKind::Restore => self.restore(principal, args, context),
            OpKind::Compare => self.compare(principal, args),
            OpKind::Close => self.close(principal, args, context),
            OpKind::Reap => self.reap(principal, args, context),
            OpKind::ReadMemory => self.read_memory(principal, args),
            OpKind::Cancel => self.cancel(principal, args),
            OpKind::Progress => self.progress(principal, args),
            OpKind::Operation => self.operation(principal, args),
            // The durable and artifact mutations are journaled exactly like the
            // session mutations: once their work has applied, the entry must be
            // retained so a retry replays instead of publishing a second object.
            OpKind::StoreList => {
                let root = self.require_storage_root()?;
                crate::durable::list(root, &self.limits, args.optional_unsigned("session_key")?)
            }
            OpKind::StoreLoad => {
                let root = self.require_storage_root()?;
                crate::durable::load(
                    root,
                    &self.factory,
                    args.unsigned("session_key")?,
                    args.optional_unsigned("snapshot_key")?,
                    args.unsigned_or_zero("steps")?,
                    args.buttons()?,
                )
            }
            OpKind::StoreSave => {
                let root = self.require_storage_root()?;
                // The durable write itself is the first irreversible effect of this
                // mutation, so `durable::save` marks the journal entry immediately
                // before it happens (F4); all fallible work precedes it.
                crate::durable::save(
                    root,
                    &self.factory,
                    args.optional_unsigned("session_key")?,
                    args.unsigned_or_zero("steps")?,
                    args.buttons()?,
                    args.name("name"),
                    context,
                )
            }
            OpKind::StoreDelete => {
                let root = self.require_storage_root()?;
                let result = crate::durable::delete(
                    root,
                    args.unsigned("session_key")?,
                    args.optional_unsigned("snapshot_key")?,
                )?;
                context.begin();
                Ok(result)
            }
            OpKind::ExperimentRun => {
                let result = self.experiment(args)?;
                context.begin();
                Ok(result)
            }
        }
    }

    /// Requests cancellation of one running operation (control path).
    ///
    /// The whole operation reads the journal and sets an atomic: it never takes a
    /// session owner, so it cannot queue behind the run it interrupts (ADR-0016 §2,
    /// K2/F3, race R1). `operation` names the key explicitly; `session` cancels that
    /// session's newest running operation instead.
    fn cancel(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let requested = if let Some(operation) = args.operation("operation") {
            let key = operation.to_string();
            let (flag, owner) = {
                let mut journal = self.lock_journal()?;
                journal.sweep();
                let entry = journal.progress(&key).ok_or_else(unknown_outcome)?;
                (journal.cancel_flag(&key), entry.principal.clone())
            };
            if owner != principal.name() && !principal.is_admin() {
                return Err(ServiceError::new(
                    ErrorCode::Forbidden,
                    "operation belongs to another principal",
                ));
            }
            match flag {
                Some(flag) => {
                    flag.request();
                    key
                }
                // The entry exists but already reached a final state: the caller is
                // asking about a finished operation, not cancelling a live one.
                None => {
                    return Err(ServiceError::new(
                        ErrorCode::Cancelled,
                        "operation already reached a final state",
                    ));
                }
            }
        } else {
            let session = self.authorized_session(principal, args)?;
            let (key, flag) = {
                let mut journal = self.lock_journal()?;
                journal.sweep();
                let key = journal.newest_active_for(session).ok_or_else(|| {
                    ServiceError::new(
                        ErrorCode::Conflict,
                        "no running operation to cancel for this session",
                    )
                })?;
                let flag = journal.cancel_flag(&key);
                (key, flag)
            };
            match flag {
                Some(flag) => {
                    flag.request();
                    key
                }
                None => {
                    return Err(ServiceError::new(
                        ErrorCode::Conflict,
                        "no running operation to cancel for this session",
                    ));
                }
            }
        };
        Ok(JsonValue::object([
            ("operation", JsonValue::string(requested)),
            ("cancelled", JsonValue::from_bool(true)),
        ]))
    }

    /// Reads one operation's progress from the journal (control path).
    fn progress(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let key = args
            .operation("operation")
            .ok_or_else(|| ServiceError::new(ErrorCode::InvalidRequest, "`operation` is required"))?
            .to_string();
        self.check_epoch(args)?;
        let mut journal = self.lock_journal()?;
        journal.sweep();
        let progress = journal.progress(&key);
        drop(journal);
        let Some(progress) = progress else {
            return Err(unknown_outcome());
        };
        if progress.principal != principal.name() && !principal.is_admin() {
            return Err(ServiceError::new(
                ErrorCode::Forbidden,
                "operation belongs to another principal",
            ));
        }
        self.progress_json(&key, &progress)
    }

    /// Reads a retained operation result inside the reconciliation horizon.
    fn operation(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let key = args
            .operation("operation")
            .ok_or_else(|| ServiceError::new(ErrorCode::InvalidRequest, "`operation` is required"))?
            .to_string();
        self.check_epoch(args)?;
        let (progress, outcome) = {
            let mut journal = self.lock_journal()?;
            journal.sweep();
            let Some(progress) = journal.progress(&key) else {
                return Err(unknown_outcome());
            };
            if progress.principal != principal.name() && !principal.is_admin() {
                return Err(ServiceError::new(
                    ErrorCode::Forbidden,
                    "operation belongs to another principal",
                ));
            }
            (progress, journal.outcome(&key))
        };
        let envelope = match outcome {
            Some(bytes) => {
                crate::json::parse(&bytes, &self.limits).map_err(|_| unknown_outcome())?
            }
            None => {
                return Err(ServiceError::new(
                    ErrorCode::Conflict,
                    "operation has not reached a final state",
                ));
            }
        };
        let mut entry = self.progress_json(&key, &progress)?;
        if let JsonValue::Object(fields) = &mut entry {
            fields.insert("outcome".to_owned(), envelope);
        }
        Ok(entry)
    }

    /// Rejects a control request that names a different process epoch.
    ///
    /// A caller that reconciled against epoch `e` must not read a result produced by a
    /// different process: the journal is process memory, so the answer is
    /// `unknown_outcome` and no re-execution follows (ADR-0016 §2, V3-3).
    fn check_epoch(&self, args: &Args) -> Result<(), ServiceError> {
        match args.optional_unsigned("epoch")? {
            Some(expected) if expected != self.epoch.value() => Err(unknown_outcome()),
            _ => Ok(()),
        }
    }

    /// Validates the epoch of a keyed mutation **before any work happens**.
    ///
    /// The key of a mutation is only meaningful inside the process epoch it was minted
    /// under: the journal that would recognize a retry is process memory, so a key from
    /// another epoch may already have been applied there. A mismatch — and, on a durable
    /// service, a key that does not name its epoch at all — is an `unknown_outcome`
    /// rather than a second application (ADR-0016 §2, F2).
    fn check_mutation_epoch(&self, args: &Args) -> Result<(), ServiceError> {
        if args.operation("operation_id").is_none() {
            return Ok(());
        }
        match args.optional_unsigned("operation_epoch")? {
            Some(expected) if expected != self.epoch.value() => {
                Err(crate::runtime::epoch_mismatch())
            }
            Some(_) => Ok(()),
            None if self.epoch_is_durable => Err(crate::runtime::epoch_required()),
            None => Ok(()),
        }
    }

    /// Adds the process epoch to one mutation result.
    ///
    /// The epoch is operational metadata, not emulated state: it tells a caller which
    /// process's journal a reconciliation result belongs to (ADR-0016 §2). It is added
    /// to the v2 shape only; a v1 response projects it away, and the journal retains the
    /// v2 bytes so both wires replay from one stored outcome.
    fn publish_epoch(
        &self,
        result: JsonValue,
        version: ApiVersion,
    ) -> Result<JsonValue, ServiceError> {
        if !version.is_v2() {
            return Ok(result);
        }
        let JsonValue::Object(mut fields) = result else {
            return Err(ServiceError::internal("mutation result must be an object"));
        };
        fields.insert("process_epoch".to_owned(), unsigned(self.epoch.value())?);
        Ok(JsonValue::Object(fields))
    }

    fn progress_json(
        &self,
        key: &str,
        progress: &OperationProgress,
    ) -> Result<JsonValue, ServiceError> {
        let stop_reason = match stop_reason_of(progress) {
            Some(reason) => JsonValue::string(reason),
            None => JsonValue::Null,
        };
        Ok(JsonValue::object([
            ("operation", JsonValue::string(key)),
            ("state", JsonValue::string(progress.state.as_str())),
            (
                "session",
                optional_unsigned(progress.session.map(SessionId::raw))?,
            ),
            ("performed", unsigned(progress.performed)?),
            ("tick", optional_unsigned(progress.tick.map(Tick::raw))?),
            ("failed", JsonValue::from_bool(progress.failed)),
            ("stop_reason", stop_reason),
            // The epoch the result belongs to (ADR-0016 §2): a caller compares it with
            // the epoch it reconciled against before trusting a retained outcome.
            ("epoch", unsigned(self.epoch.value())?),
        ]))
    }

    fn capabilities(&self) -> Result<JsonValue, ServiceError> {
        let ops: Vec<JsonValue> = CATALOG
            .iter()
            .map(|spec| {
                JsonValue::object([
                    ("name", JsonValue::string(spec.name)),
                    ("mutating", JsonValue::from_bool(spec.mutating)),
                ])
            })
            .collect();
        let limits = self.limits_json()?;
        Ok(JsonValue::object([
            ("schema", JsonValue::string("chronofork.capabilities.v1")),
            ("service_version", JsonValue::string(SERVICE_VERSION)),
            ("api_version", unsigned(WIRE_VERSION)?),
            ("backend", descriptor_json(self.backend())),
            ("ops", JsonValue::Array(ops)),
            ("limits", limits),
            (
                "store",
                JsonValue::object([(
                    "configured",
                    JsonValue::from_bool(self.storage_root.is_some()),
                )]),
            ),
            (
                "artifacts",
                JsonValue::object([(
                    "configured",
                    JsonValue::from_bool(self.artifacts_root.is_some()),
                )]),
            ),
            // Operational metadata, not emulated state: the epoch identifies the process
            // that owns the journal, so a reconciled result is only trusted while it
            // matches (ADR-0016 §2). v1 projections remove it.
            ("process_epoch", unsigned(self.epoch.value())?),
        ]))
    }

    fn limits_json(&self) -> Result<JsonValue, ServiceError> {
        let limits = self.limits;
        Ok(JsonValue::object([
            (
                "max_request_bytes",
                unsigned(bytes_to_u64(limits.max_request_bytes))?,
            ),
            (
                "max_response_bytes",
                unsigned(bytes_to_u64(limits.max_response_bytes))?,
            ),
            (
                "max_steps_per_request",
                unsigned(limits.max_steps_per_request)?,
            ),
            (
                "max_memory_read",
                unsigned(bytes_to_u64(limits.max_memory_read))?,
            ),
            ("max_sessions", unsigned(bytes_to_u64(limits.max_sessions))?),
            (
                "max_store_entries",
                unsigned(bytes_to_u64(limits.max_store_entries))?,
            ),
            (
                "max_checkpoints",
                unsigned(bytes_to_u64(limits.max_checkpoints))?,
            ),
            (
                "json_max_depth",
                unsigned(bytes_to_u64(limits.json_max_depth))?,
            ),
            ("max_run_wall_ms", unsigned(limits.max_run_wall_ms)?),
            (
                "max_operation_journal",
                unsigned(bytes_to_u64(limits.max_operation_journal))?,
            ),
            ("operation_ttl", unsigned(limits.operation_ttl)?),
            (
                "max_closed_retained",
                unsigned(bytes_to_u64(limits.max_closed_retained))?,
            ),
            (
                "closed_retention_ttl",
                unsigned(limits.closed_retention_ttl)?,
            ),
            (
                "max_snapshots_per_session",
                unsigned(bytes_to_u64(limits.max_snapshots_per_session))?,
            ),
            (
                "max_bytes_per_session",
                unsigned(bytes_to_u64(limits.max_bytes_per_session))?,
            ),
            (
                "max_store_bytes",
                unsigned(bytes_to_u64(limits.max_store_bytes))?,
            ),
        ]))
    }

    /// Creates a session under one atomic admission critical section (I3/I4).
    ///
    /// Admission is a **reservation taken before the backend exists**: a create refused
    /// by `max_sessions` constructs nothing, and the reservation makes the live bound
    /// hold across concurrent creates without holding the registry while a backend is
    /// built. The session layer is taken after the reservation and released again before
    /// the insert, so the two locks never form a cycle and `create` never queues behind a
    /// running session.
    fn create(
        &self,
        principal: &Principal,
        _args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        let reservation = self.reserve_session_slot()?;
        // A build failure drops the reservation (RAII), so the slot is released without
        // this path having to remember it.
        let (session, handle) = self.build_session()?;
        // The built session is not visible yet: the guard removes it from the session
        // layer on **every** early return below, including the `?` of a failed
        // inspection, so a rejected create can never leave an invisible backend alive.
        let pending = PendingSession::new(self, session);
        let (view, committed) = {
            let owner = handle.lock()?;
            let view = owner.inspect().map_err(ServiceError::from)?;
            let committed = metadata_with_usage(&view, Some(owner.snapshot_bytes_retained()));
            (view, committed)
        };
        // The registry insert is the first irreversible effect of this mutation, so the
        // journal entry is retained from here on: a retry replays the admitted session
        // instead of creating a second one (F4).
        context.begin();
        self.admit_reserved(
            reservation,
            session,
            SessionRecord {
                owner: principal.name().to_owned(),
                closed: false,
                handle: Some(handle),
                committed,
                revision: 0,
                closed_at: None,
            },
        )?;
        // The session is registered: the guard must not remove it again.
        pending.disarm();
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("state", JsonValue::string(state_name(view.state))),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("lineage", lineage_json(&view.lineage)?),
            ("backend_id", JsonValue::string(view.backend.id().as_str())),
        ]))
    }

    /// Builds one fresh session and its owner in the session layer (I2).
    fn build_session(&self) -> Result<(SessionId, SessionHandle), ServiceError> {
        let mut sessions = self.lock_sessions()?;
        let id = sessions.create_session().map_err(ServiceError::from)?;
        let handle = sessions.session_handle(id).map_err(ServiceError::from)?;
        Ok((id, handle))
    }

    /// Builds the child of a fork through the source handle (no manager re-entry).
    fn build_forked_session(
        &self,
        parent: SessionHandle,
        snapshot: SnapshotId,
    ) -> Result<(SessionId, SessionHandle), ServiceError> {
        let mut sessions = self.lock_sessions()?;
        let mut child = parent
            .fork_from_snapshot(snapshot, &self.factory)
            .map_err(ServiceError::from)?;
        let id = sessions.allocate_session_id().map_err(ServiceError::from)?;
        child.assign_id(id);
        Ok((id, child))
    }

    /// Lists cheap committed metadata without ever taking a session owner (I5).
    ///
    /// The entry's fields come from the metadata published by that session's last
    /// completed mutating operation, and the `running` marker comes from a
    /// non-blocking attempt on the session owner. A session whose owner is held by a
    /// run is therefore reported with its pre-run metadata and `running: true`,
    /// rather than making the listing wait. No digest is recomputed and no machine
    /// memory is copied, which is the declared freshness contract of ADR-0016 §1.
    fn list(&self, principal: &Principal) -> Result<JsonValue, ServiceError> {
        let rows = {
            let registry = self.lock_registry()?;
            registry.visible_rows(principal)
        };
        let mut sessions = Vec::new();
        let mut truncated = false;
        for (session, handle, committed) in rows {
            if sessions.len() >= self.limits.max_store_entries {
                truncated = true;
                break;
            }
            // Only open sessions are visible here, so the owner is present; a probe
            // that finds it held reports the `running` marker (I5).
            let running = handle.is_some_and(|handle| handle.try_lock().is_none());
            let lineage = lineage_json(&committed.lineage)?;
            sessions.push(summary_json(session, &committed, lineage, running)?);
        }
        Ok(JsonValue::object([
            ("sessions", JsonValue::Array(sessions)),
            ("truncated", JsonValue::from_bool(truncated)),
        ]))
    }

    /// Inspects one session under its own owner.
    ///
    /// `revision` is the monotonic precondition token (ADR-0016 §2, K3): `tick` can
    /// move backwards through `restore` and can be unchanged by two writes, so a
    /// retry pins the revision, not the tick.
    fn inspect(&self, session: SessionId) -> Result<JsonValue, ServiceError> {
        let handle = self.session_handle(session)?;
        let revision = self.revision(session);
        let owner = handle.lock()?;
        let view = owner.inspect().map_err(ServiceError::from)?;
        let mut registers = BTreeMap::new();
        for (name, value) in &view.machine.registers {
            registers.insert(name.clone(), unsigned(*value)?);
        }
        let machine = JsonValue::object([
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("halted", JsonValue::from_bool(view.machine.halted)),
            ("registers", JsonValue::Object(registers)),
            ("memory_base", unsigned(view.machine.memory_base)?),
            (
                "memory_length",
                unsigned(bytes_to_u64(view.machine.memory.len()))?,
            ),
        ]);
        drop(owner);
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("state", JsonValue::string(state_name(view.state))),
            ("lineage", lineage_json(&view.lineage)?),
            ("revision", unsigned(revision)?),
            ("backend", descriptor_json(&view.backend)),
            ("machine", machine),
        ]))
    }

    /// Runs a session holding only that session's owner for the whole run.
    ///
    /// The registry is released before the owner is taken (I1), so `list`,
    /// `inspect` of another session, `create`, `fork` and `close` of another
    /// session are never blocked by this loop. Determinism is unchanged: the step
    /// sequence and the reported `tick`/`state_hash` are exactly the ones the
    /// previous manager-locked loop produced.
    ///
    /// W3.2 adds three host-policy inputs that never enter canonical state
    /// (ADR-0016 §2, K3):
    ///
    /// - the **cancel signal** is read once per instruction from a shared atomic,
    ///   so `session.cancel` never waits for this owner (race R1);
    /// - the **wall-clock deadline** chooses how many steps `k` are applied; the
    ///   result reports `performed`, so a replay records `k` plus the inputs;
    /// - **progress** is published into the journal on a fixed stride, so
    ///   `session.progress` is a journal read and never a session read.
    ///
    /// The same-boundary precedence of ADR-0016 §New limits is applied here:
    /// `cancelled` beats `deadline` beats `halted` beats `completed`.
    fn run(
        &self,
        principal: &Principal,
        args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let steps = args.unsigned("steps")?;
        let buttons = args.buttons()?;
        let budget_ms = args
            .optional_unsigned("max_wall_ms")?
            .unwrap_or(self.limits.max_run_wall_ms)
            .min(self.limits.max_run_wall_ms);
        let expected_revision = args.optional_unsigned("expected_revision")?;
        let expected_tick = args.optional_unsigned("expected_tick")?;
        let _lease = self.acquire_lease(session)?;
        let handle = self.session_handle(session)?;
        let mut owner = handle.lock()?;
        // The precondition is checked **under the session owner**, so the test and
        // the first step cannot interleave with another mutation (ADR-0016 §2, K3).
        // A rejected precondition applies nothing, so the journal entry stays
        // abandoned and a retry with the same key is fresh again.
        if let Some(expected) = expected_revision {
            let observed = self.revision(session);
            if observed != expected {
                return Err(ServiceError::new(
                    ErrorCode::Conflict,
                    format!("session revision is {observed}, not {expected}"),
                ));
            }
        }
        if let Some(expected) = expected_tick {
            let observed = owner
                .inspect()
                .map_err(ServiceError::from)?
                .machine
                .tick
                .raw();
            if observed != expected {
                return Err(ServiceError::new(
                    ErrorCode::Conflict,
                    format!("session tick is {observed}, not {expected}"),
                ));
            }
        }
        // Work begins here: every step below can change canonical state, so the entry
        // must be retained even if the run stops on its first instruction.
        context.begin();
        let deadline = RunDeadline::new(budget_ms);
        let input = InputFrame::new(buttons);
        let mut performed = 0_u64;
        let mut halted = false;
        let mut stop = StopReason::Completed;
        let mut step_error: Option<ServiceError> = None;
        while performed < steps {
            // Cancellation is checked at **every** instruction boundary, so a
            // cancelled run is a deterministic prefix whose length depends only on
            // when the signal arrived, never on a batch size (ADR-0016 §2, V3-2).
            if context.cancel().is_cancelled() {
                stop = StopReason::Cancelled;
                break;
            }
            // The deadline is sampled on a stride, so the monotonic clock stays off
            // the per-instruction path while the applied prefix stays bounded.
            if performed % DEADLINE_SAMPLE_STRIDE == 0 && deadline.expired() {
                stop = StopReason::Deadline;
                break;
            }
            let outcome = match owner.step(input) {
                Ok(outcome) => outcome,
                Err(error) => {
                    // A backend failure **after** steps were applied still leaves a
                    // deterministic prefix: it must be published with the revision it
                    // reached before the error is reported, otherwise a caller could
                    // not reconcile what the run actually did (ADR-0016 §2, K3).
                    step_error = Some(ServiceError::from(error));
                    break;
                }
            };
            performed = performed
                .checked_add(1)
                .ok_or_else(|| ServiceError::internal("step counter overflow"))?;
            halted = outcome.halted;
            if halted {
                stop = StopReason::Halted;
                break;
            }
            if performed % PROGRESS_PUBLISH_STRIDE == 0 {
                context.publish_progress(performed, None);
            }
        }
        let view = owner.inspect().map_err(ServiceError::from)?;
        let committed = metadata_of(&view);
        drop(owner);
        // Any run that applied at least one step advances the revision once, whatever
        // stopped it (budget, cancel, deadline, halt or a backend failure); the prefix
        // is published with the revision it reached, so a retry can pin it.
        let revision = if performed > 0 {
            self.bump_revision(session, committed.clone())
        } else {
            self.publish(session, committed.clone());
            self.revision(session)
        };
        // The run's final boundary is always published, so a `session.progress`
        // issued after the run returns the exact end state instead of a stale stride.
        context.publish_progress(performed, Some(view.machine.tick));
        if let Some(error) = step_error {
            return Err(error);
        }
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("steps", unsigned(performed)?),
            ("performed", unsigned(performed)?),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("halted", JsonValue::from_bool(halted)),
            ("stop_reason", JsonValue::string(stop.as_str())),
            ("revision", unsigned(revision)?),
            ("max_wall_ms", unsigned(budget_ms)?),
        ]))
    }

    /// Forks one session from its current checkpoint.
    ///
    /// Order (I1/I4): registry lookup, source owner, release, then the session
    /// layer builds the child and detaches it, and only then the registry is
    /// re-taken to admit the child under the same atomic check as `create`. The
    /// source owner is never held while the session layer or the registry is
    /// locked, so `fork` cannot invert the lock order.
    fn fork(
        &self,
        principal: &Principal,
        args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        let parent = SessionId::from_raw(args.unsigned("session")?);
        // The source's ownership and lifecycle are checked first: a closed (or foreign)
        // source is a lifecycle error, not an admission one, whatever the live count is.
        let parent_closed = self.lock_registry()?.authorized_state(principal, parent)?;
        if parent_closed {
            return Err(ServiceError::from(
                chronofork_sessions::SessionError::Closed(parent),
            ));
        }
        // Admission is decided **before anything is captured or built** (C2): a fork
        // refused by `max_sessions` adds no checkpoint to the parent, consumes no
        // checkpoint identifier and constructs no child backend. The reservation counts
        // against `max_sessions`, so two concurrent forks cannot both take the last slot.
        let reservation = self.reserve_session_slot()?;
        let parent_handle = self.session_handle(parent)?;
        let _lease = self.acquire_lease(parent)?;
        // The fork's origin checkpoint goes through the **same quota-aware capture**
        // as `session.snapshot`: forking cannot accumulate checkpoints past the
        // parent's per-session quota (ADR-0016 §3).
        let (snapshot, parent_usage, parent_view) = {
            let mut owner = parent_handle.lock()?;
            let snapshot = owner
                .snapshot_within(
                    self.limits.max_snapshots_per_session,
                    self.limits.max_bytes_per_session,
                )
                .map_err(ServiceError::from)?;
            // The parent's origin checkpoint is the first irreversible effect of this
            // mutation, so the entry is retained from here on — **before** any fallible
            // metadata read (C5): a failure after it replays the recorded failure instead
            // of capturing the parent's checkpoint again.
            context.begin();
            let usage = owner.snapshot_bytes_retained();
            let view = owner.inspect().map_err(ServiceError::from)?;
            (snapshot, usage, view)
        };
        self.publish(
            parent,
            metadata_with_usage(&parent_view, Some(parent_usage)),
        );
        let (child, child_handle) = self.build_forked_session(parent_handle.clone(), snapshot)?;
        let (view, committed, child_bytes) = {
            let owner = child_handle.lock()?;
            let view = owner.inspect().map_err(ServiceError::from)?;
            let bytes = owner.snapshot_bytes_retained();
            let committed = metadata_with_usage(&view, Some(bytes));
            (view, committed, bytes)
        };
        // The child inherits exactly one checkpoint (its origin). It must fit the
        // child's own byte quota, otherwise the fork is rejected whole (the parent keeps
        // the origin checkpoint, and the retained entry replays this failure).
        if child_bytes > self.limits.max_bytes_per_session {
            self.detach_session(child);
            return Err(ServiceError::from(
                chronofork_sessions::SessionError::LimitExceeded {
                    quota: chronofork_sessions::SessionQuota::Bytes,
                    limit: self.limits.max_bytes_per_session,
                    current: child_bytes,
                },
            ));
        }
        if let Err(error) = self.admit_reserved(
            reservation,
            child,
            SessionRecord {
                owner: principal.name().to_owned(),
                closed: false,
                handle: Some(child_handle),
                committed,
                revision: 0,
                closed_at: None,
            },
        ) {
            // A registry failure after the build must not leak the child either.
            self.detach_session(child);
            return Err(error);
        }
        Ok(JsonValue::object([
            ("session", unsigned(child.raw())?),
            ("parent", unsigned(parent.raw())?),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("lineage", lineage_json(&view.lineage)?),
            ("revision", unsigned(self.revision(child))?),
        ]))
    }

    fn snapshot(
        &self,
        principal: &Principal,
        args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let _lease = self.acquire_lease(session)?;
        let handle = self.session_handle(session)?;
        // The quota is checked and applied while this session's owner is held, so the
        // count and byte bound are enforced against this session's own checkpoint set
        // (ADR-0016 §3). A rejection publishes the current usage and leaves the
        // session, its checkpoints and the identifier counter untouched.
        let (snapshot, usage) = {
            let mut owner = handle.lock()?;
            let outcome = owner
                .snapshot_within(
                    self.limits.max_snapshots_per_session,
                    self.limits.max_bytes_per_session,
                )
                .map_err(ServiceError::from);
            let usage = (owner.snapshot_count(), owner.snapshot_bytes_retained());
            match outcome {
                Ok(snapshot) => {
                    // The checkpoint is stored: this is the mutation's **first
                    // irreversible effect**, so the entry is retained immediately — before
                    // the fallible metadata read below (C5). A failure after this point
                    // replays the recorded failure instead of capturing a second
                    // checkpoint on retry.
                    context.begin();
                    let view = owner.inspect().map_err(ServiceError::from)?;
                    drop(owner);
                    self.publish(session, metadata_with_usage(&view, Some(usage.1)));
                    self.refresh_store_bytes();
                    (snapshot, usage)
                }
                Err(error) => {
                    // Nothing was stored: publish the current usage and apply nothing.
                    let view = owner.inspect().map_err(ServiceError::from)?;
                    drop(owner);
                    self.publish(session, metadata_with_usage(&view, Some(usage.1)));
                    self.refresh_store_bytes();
                    return Err(error);
                }
            }
        };
        let record = handle
            .snapshot_record(snapshot)
            .map_err(ServiceError::from)?;
        let bytes = record.to_bytes().map_err(ServiceError::from)?;
        let tick = record.tick();
        let state_hash = record.state_hash();
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("snapshot", unsigned(snapshot.raw())?),
            ("name", optional_name(args.name("name"))),
            ("tick", unsigned(tick.raw())?),
            ("state_hash", JsonValue::string(state_hash.to_hex())),
            ("bytes_length", unsigned(bytes_to_u64(bytes.len()))?),
            (
                "bytes_sha256",
                JsonValue::string(hash_bytes(&bytes).to_hex()),
            ),
            // The usage the quota was just checked against, so a caller can size its
            // next call without guessing (ADR-0016 §Limits table).
            ("snapshots", unsigned(bytes_to_u64(usage.0))?),
            ("snapshot_bytes", unsigned(bytes_to_u64(usage.1))?),
        ]))
    }

    fn restore(
        &self,
        principal: &Principal,
        args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let snapshot = SnapshotId::from_raw(args.unsigned("snapshot")?);
        let expected_revision = args.optional_unsigned("expected_revision")?;
        let _lease = self.acquire_lease(session)?;
        let handle = self.session_handle(session)?;
        let (view, usage) = {
            let mut owner = handle.lock()?;
            // The precondition is evaluated under the session owner, so a stale retry
            // cannot slip between the check and the restore (ADR-0016 §2, K3).
            if let Some(expected) = expected_revision {
                let observed = self.revision(session);
                if observed != expected {
                    return Err(ServiceError::new(
                        ErrorCode::Conflict,
                        format!("session revision is {observed}, not {expected}"),
                    ));
                }
            }
            owner.restore(snapshot).map_err(ServiceError::from)?;
            // The state moved: this is the first irreversible effect of the mutation, so
            // the entry is retained from here on even if the inspection below fails.
            context.begin();
            let usage = owner.snapshot_bytes_retained();
            let view = owner.inspect().map_err(ServiceError::from)?;
            (view, usage)
        };
        // A restore moves canonical state, so it advances the monotonic revision: the
        // tick can move backwards (ABA), and only the revision can tell a retry that
        // the state it pinned is gone (ADR-0016 §2, K3/race R6).
        let committed = metadata_with_usage(&view, Some(usage));
        let revision = self.bump_revision(session, committed);
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("snapshot", unsigned(snapshot.raw())?),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("revision", unsigned(revision)?),
        ]))
    }

    /// Compares two sessions by taking their owners in identifier order (I4).
    ///
    /// The deterministic order removes the two-owner deadlock class: two
    /// concurrent comparisons of the same pair always acquire the same owners in
    /// the same order, and a session compared with itself is locked once.
    fn compare(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let left = SessionId::from_raw(args.unsigned("left")?);
        let right = SessionId::from_raw(args.unsigned("right")?);
        self.authorize(principal, left)?;
        self.authorize(principal, right)?;
        let left_handle = self.session_handle(left)?;
        let (left_tick, left_hash, right_tick, right_hash) = if left == right {
            let owner = left_handle.lock()?;
            let view = owner.inspect().map_err(ServiceError::from)?;
            (
                view.machine.tick,
                view.machine.state_hash,
                view.machine.tick,
                view.machine.state_hash,
            )
        } else {
            let right_handle = self.session_handle(right)?;
            let (first, second) = if left <= right {
                (left_handle, right_handle)
            } else {
                (right_handle, left_handle)
            };
            let first_owner = first.lock()?;
            let second_owner = second.lock()?;
            let (left_owner, right_owner) = if left <= right {
                (&first_owner, &second_owner)
            } else {
                (&second_owner, &first_owner)
            };
            let left_view = left_owner.inspect().map_err(ServiceError::from)?;
            let right_view = right_owner.inspect().map_err(ServiceError::from)?;
            (
                left_view.machine.tick,
                left_view.machine.state_hash,
                right_view.machine.tick,
                right_view.machine.state_hash,
            )
        };
        let equivalent = left_tick == right_tick && left_hash == right_hash;
        Ok(JsonValue::object([
            ("left", unsigned(left.raw())?),
            ("right", unsigned(right.raw())?),
            ("left_tick", unsigned(left_tick.raw())?),
            ("right_tick", unsigned(right_tick.raw())?),
            ("left_state_hash", JsonValue::string(left_hash.to_hex())),
            ("right_state_hash", JsonValue::string(right_hash.to_hex())),
            ("equivalent", JsonValue::from_bool(equivalent)),
        ]))
    }

    fn close(
        &self,
        principal: &Principal,
        args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        let session = SessionId::from_raw(args.unsigned("session")?);
        // Closing is idempotent: a closed entry has already released its owner, so the
        // ownership/lifecycle probe comes first and a second close answers the same
        // document instead of a lifecycle error.
        let already_closed = self.lock_registry()?.authorized_state(principal, session)?;
        if already_closed {
            context.begin();
            return Ok(JsonValue::object([
                ("session", unsigned(session.raw())?),
                ("state", JsonValue::string("closed")),
                ("released_checkpoint_bytes", unsigned(0)?),
            ]));
        }
        let _lease = self.acquire_lease(session)?;
        let handle = self.session_handle(session)?;
        let released = {
            let mut owner = handle.lock()?;
            // Closing is idempotent (closing twice is a documented no-op) and a closed
            // session cannot be inspected, so the committed metadata is only refreshed
            // for a session that is open at this instant. The committed copy of an
            // already-closed session was published by the close that closed it.
            let committed = match owner.inspect() {
                Ok(view) => Some(metadata_with_usage(&view, Some(0))),
                Err(_) => None,
            };
            owner.close().map_err(ServiceError::from)?;
            // Releasing the checkpoints here is what makes "close frees memory" literal
            // (ADR-0016 §3, K9/V3-2): a `create -> close` loop cannot accumulate closed
            // checkpoint state, and the published usage drops to zero in the same
            // commit. The session object itself stays alive for in-flight readers.
            let released = owner.release_snapshot_records();
            drop(owner);
            let mut registry = self.lock_registry()?;
            if let Some(record) = registry.records.get_mut(&session) {
                record.closed = true;
                record.closed_at = Some(now());
                record.revision = record.revision.saturating_add(1);
                // Dropping the registry's owner here — with the session layer's copy
                // already handed off below — releases the backend **at close**, not at
                // reap. Only an `Arc` an in-flight reader already holds keeps the
                // session object alive, which is the ADR's contract (K9/V3-2).
                record.handle = None;
                if let Some(committed) = committed {
                    record.committed = committed;
                }
            }
            record_store_bytes(&self.store_bytes, registry.retained_bytes());
            released
        };
        // The session layer must stop owning the session, otherwise its map would keep
        // the backend alive and "close frees the backend" would be aspirational.
        self.detach_session(session);
        // Forced cleanup of closed entries is independent of the general cap: closing a
        // session is exactly when the `max_closed_retained`/`closed_retention_ttl`
        // bound can be exceeded (ADR-0016 §3, K9/V3-2).
        self.force_cleanup();
        // The close applied: the commit point of this mutation.
        context.begin();
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("state", JsonValue::string("closed")),
            (
                "released_checkpoint_bytes",
                unsigned(bytes_to_u64(released))?,
            ),
        ]))
    }

    /// Removes closed session entries and releases their registration slots.
    ///
    /// The removal is the three-phase protocol of ADR-0016 §1 applied to the removal
    /// case: candidates are selected under the registry lock, each candidate's owner is
    /// probed **without** the registry held (a session with a live run is skipped, never
    /// waited on and never faulting — race R5), and the registry is re-taken to remove
    /// the entry. The entry's `Arc` keeps the session alive for a reader already in
    /// flight, and after the removal a later `inspect` reports `not_found` instead of
    /// `closed`.
    fn reap(
        &self,
        principal: &Principal,
        args: &Args,
        context: &OperationContext<'_>,
    ) -> Result<JsonValue, ServiceError> {
        let target = args.optional_unsigned("session")?;
        let session = target.map(SessionId::from_raw);
        if let Some(session) = session {
            self.lock_registry()?
                .authorized_closed(principal, session)?;
        }
        let reaped = self.reap_locked(principal, session)?;
        // A reap may have removed entries; the commit is recorded so a retry replays
        // the same removal report instead of reaping whatever appeared meanwhile.
        context.begin();
        let ids: Vec<JsonValue> = reaped
            .iter()
            .map(|session| unsigned(session.raw()))
            .collect::<Result<_, _>>()?;
        // The result reports the registry accounting after the removal: the number of
        // registered/live/closed entries and the committed checkpoint bytes. The global
        // byte bound is **best-effort** (ADR-0016 §3) and this report is where that
        // accounting is observable; it is not a hard admission gate.
        let (registered, live, closed, bytes) = self.registry_totals()?;
        Ok(JsonValue::object([
            ("reaped", unsigned(bytes_to_u64(reaped.len()))?),
            ("sessions", JsonValue::Array(ids)),
            ("registered", unsigned(bytes_to_u64(registered))?),
            ("live", unsigned(bytes_to_u64(live))?),
            ("closed", unsigned(bytes_to_u64(closed))?),
            ("retained_bytes", unsigned(bytes_to_u64(bytes))?),
            (
                "max_store_bytes",
                unsigned(bytes_to_u64(self.limits.max_store_bytes))?,
            ),
            ("max_store_bytes_best_effort", JsonValue::from_bool(true)),
        ]))
    }

    /// Returns `(registered, live, closed, retained checkpoint bytes)`.
    fn registry_totals(&self) -> Result<(usize, usize, usize, usize), ServiceError> {
        let registry = self.lock_registry()?;
        Ok((
            registry.registered_count(),
            registry.live_count(),
            registry.closed_count(),
            registry.retained_bytes(),
        ))
    }

    /// Removes the entries forced cleanup is allowed to drop, in any ownership.
    ///
    /// This is the maintenance half of ADR-0016 §3: when the registry is at
    /// `max_sessions`, closed entries beyond `max_closed_retained` (or past
    /// `closed_retention_ttl`) are removed so a bounded set of closed entries can
    /// never block admission, independently of the live count.
    fn force_cleanup(&self) -> Vec<SessionId> {
        let candidates = match self.lock_registry() {
            Ok(registry) => registry.reap_candidates(
                now(),
                self.limits.max_closed_retained,
                self.limits.closed_retention_ttl,
            ),
            Err(_) => return Vec::new(),
        };
        self.remove_sessions(candidates)
    }

    /// Performs the reap protocol, optionally restricted to one authorized session.
    ///
    /// An explicit reap removes the closed entries this principal may observe (its
    /// own, or every entry for an administrator); it is the operation a client calls
    /// to release registration slots on purpose. The `max_closed_retained` /
    /// `closed_retention_ttl` bounds are enforced separately by [`Self::force_cleanup`]
    /// so they hold even when nobody calls `reap`.
    fn reap_locked(
        &self,
        principal: &Principal,
        only: Option<SessionId>,
    ) -> Result<Vec<SessionId>, ServiceError> {
        let candidates = match only {
            Some(session) => vec![session],
            None => {
                let registry = self.lock_registry()?;
                registry
                    .records
                    .iter()
                    .filter(|(_, record)| {
                        record.closed && (principal.is_admin() || record.owner == principal.name())
                    })
                    .map(|(session, _)| *session)
                    .collect()
            }
        };
        Ok(self.remove_sessions(candidates))
    }

    /// Removes the given entries, skipping any session with a live owner.
    ///
    /// The removal is the three-phase protocol of ADR-0016 §1 applied to the removal
    /// case: the candidate handle is read under the registry lock, the owner is probed
    /// **without** the registry held (a held owner means a reader or writer is inside
    /// the session: the entry stays registered, a later reap retries it, and race R5's
    /// `Arc` keeps that in-flight reader safe), and the registry is re-taken only to
    /// remove the entry.
    fn remove_sessions(&self, candidates: Vec<SessionId>) -> Vec<SessionId> {
        let mut reaped = Vec::new();
        for session in candidates {
            let handle = {
                let Ok(registry) = self.registry.lock() else {
                    break;
                };
                match registry.records.get(&session) {
                    Some(record) if record.closed => record.handle.clone(),
                    _ => continue,
                }
            };
            // A closed entry has already released its owner, so there is nothing to
            // probe; an open owner means a reader is inside and the entry stays.
            if handle.is_some_and(|handle| handle.try_lock().is_none()) {
                continue;
            }
            let Ok(mut registry) = self.registry.lock() else {
                break;
            };
            let removable = registry
                .records
                .get(&session)
                .is_some_and(|record| record.closed);
            if removable && registry.remove(session) {
                reaped.push(session);
                record_store_bytes(&self.store_bytes, registry.retained_bytes());
            }
            drop(registry);
            self.detach_session(session);
        }
        reaped
    }

    /// Removes a session from the private session layer's map.
    ///
    /// The service owns the session through the registry handle; the session layer is
    /// only a factory and an identifier source (I2). Dropping its copy is what makes a
    /// released session's memory really released, and it is best effort because a
    /// session already gone from that map is not an error for a lifecycle operation.
    fn detach_session(&self, session: SessionId) {
        if let Ok(mut sessions) = self.sessions.lock() {
            let _ = sessions.detach(session);
        }
    }

    /// Refreshes the best-effort global checkpoint-byte counter from the registry.
    fn refresh_store_bytes(&self) {
        if let Ok(registry) = self.registry.lock() {
            record_store_bytes(&self.store_bytes, registry.retained_bytes());
        }
    }

    fn read_memory(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let address = args.unsigned("address")?;
        let length = usize::try_from(args.unsigned_or_zero("length")?)
            .map_err(|_| ServiceError::internal("validated memory length is out of range"))?;
        let handle = self.session_handle(session)?;
        let bytes = handle
            .lock()?
            .read_memory(address, length)
            .map_err(ServiceError::from)?;
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("address", unsigned(address)?),
            ("length", unsigned(bytes_to_u64(bytes.len()))?),
            ("bytes", JsonValue::string(hex(&bytes))),
        ]))
    }

    fn experiment(&self, args: &Args) -> Result<JsonValue, ServiceError> {
        let experiment = crate::experiment::run(
            &self.factory,
            &self.limits,
            args.unsigned("steps")?,
            args.buttons()?,
            args.checkpoints(),
        )?;
        if let Some(root) = &self.artifacts_root {
            crate::experiment::persist(root, &experiment.artifact_id, &experiment.bytes)?;
        }
        let crate::experiment::Experiment {
            artifact,
            artifact_id,
            ..
        } = experiment;
        Ok(JsonValue::object([
            ("artifact", artifact),
            ("artifact_id", JsonValue::string(artifact_id)),
        ]))
    }

    /// Resolves the `session` field and enforces per-principal ownership.
    fn authorized_session(
        &self,
        principal: &Principal,
        args: &Args,
    ) -> Result<SessionId, ServiceError> {
        let session = SessionId::from_raw(args.unsigned("session")?);
        self.authorize(principal, session)?;
        Ok(session)
    }

    /// Enforces that the principal may observe and mutate `session`.
    ///
    /// The registry is taken and released before the caller acquires any session
    /// owner (I1).
    fn authorize(&self, principal: &Principal, session: SessionId) -> Result<(), ServiceError> {
        self.lock_registry()?
            .authorized(principal, session)
            .map(|_| ())
    }

    /// Returns one registered session's execution owner.
    ///
    /// The registry is released before the caller locks the returned handle (I1).
    fn session_handle(&self, session: SessionId) -> Result<SessionHandle, ServiceError> {
        let registry = self.lock_registry()?;
        registry
            .records
            .get(&session)
            .ok_or_else(|| ServiceError::new(ErrorCode::NotFound, "unknown session"))?
            .handle
            .clone()
            .ok_or_else(|| ServiceError::from(chronofork_sessions::SessionError::Closed(session)))
    }

    /// Takes the writer lease for one session, failing immediately on contention.
    fn acquire_lease(&self, session: SessionId) -> Result<Lease<'_>, ServiceError> {
        let mut leases = self.leases.lock().map_err(|_| poisoned())?;
        if leases.held.contains_key(&session) {
            return Err(ServiceError::retryable_conflict(
                "session is being mutated by another request",
            ));
        }
        let token = leases.next;
        leases.next = token.wrapping_add(1);
        leases.held.insert(session, token);
        Ok(Lease {
            service: self,
            session,
            token,
        })
    }

    /// Publishes the committed metadata of one session after its owner is released.
    ///
    /// The write is best effort: a session removed from the registry between the
    /// operation and the publish has no entry to update.
    fn publish(&self, session: SessionId, committed: SessionMetadata) {
        if let Ok(mut registry) = self.registry.lock() {
            if let Some(record) = registry.records.get_mut(&session) {
                record.committed = committed;
            }
        }
    }

    fn require_storage_root(&self) -> Result<&Path, ServiceError> {
        self.storage_root.as_deref().ok_or_else(|| {
            ServiceError::new(
                ErrorCode::NotConfigured,
                "durable storage is not configured",
            )
        })
    }

    /// Reserves one live-session slot, or returns the typed limit error.
    ///
    /// The check and the reservation are **one critical section**, and the reservation is
    /// taken before any backend is built or any checkpoint is captured: a refused
    /// admission allocates nothing (ADR-0016 §3, "never partially applies"). The slot
    /// counts against `max_sessions` while it is held, so two concurrent admissions
    /// cannot both take the last slot. It is released unless
    /// [`Self::admit_reserved`] consumes it.
    fn reserve_session_slot(&self) -> Result<SessionReservation<'_>, ServiceError> {
        // Opportunistic cleanup of closed entries before the check, without the
        // registry held: cleanup probes session owners, and an owner is never waited
        // on while the registry is held (I1).
        let closed = self.lock_registry()?.closed_count();
        if closed > 0 {
            self.force_cleanup();
        }
        let mut registry = self.lock_registry()?;
        if registry.admitted_count() >= self.limits.max_sessions {
            return Err(session_limit());
        }
        registry.reserved = registry.reserved.saturating_add(1);
        Ok(SessionReservation {
            registry: &self.registry,
            armed: true,
        })
    }

    /// Admits a reserved slot as its live session: the insert, the reservation release
    /// and the byte accounting are one critical section.
    fn admit_reserved(
        &self,
        reservation: SessionReservation<'_>,
        session: SessionId,
        record: SessionRecord,
    ) -> Result<(), ServiceError> {
        let mut registry = self.lock_registry()?;
        registry.records.insert(session, record);
        registry.reserved = registry.reserved.saturating_sub(1);
        reservation.disarm();
        record_store_bytes(&self.store_bytes, registry.retained_bytes());
        Ok(())
    }

    /// Reads one session's committed revision.
    ///
    /// The revision is the monotonic precondition token of ADR-0016 §2 (K3): it
    /// advances at every committed mutation of the session (a run that applied steps,
    /// `restore`, `close`), so a retry can detect that state moved on even when the
    /// tick did not — including an ABA restore that moved the tick backwards. A
    /// forked child starts at zero; a session with no registry entry reports zero.
    fn revision(&self, session: SessionId) -> u64 {
        self.lock_registry()
            .ok()
            .and_then(|registry| registry.records.get(&session).map(|record| record.revision))
            .unwrap_or(0)
    }

    /// Publishes committed metadata and advances one session's revision by one.
    fn bump_revision(&self, session: SessionId, committed: SessionMetadata) -> u64 {
        let mut registry = match self.registry.lock() {
            Ok(registry) => registry,
            // A poisoned registry cannot publish; the operation already happened and
            // its result is reported with the revision the caller can re-read.
            Err(_) => return 0,
        };
        let Some(record) = registry.records.get_mut(&session) else {
            return 0;
        };
        record.committed = committed;
        record.revision = record.revision.saturating_add(1);
        record.revision
    }

    fn lock_journal(&self) -> Result<MutexGuard<'_, OperationJournal>, ServiceError> {
        self.journal.lock().map_err(|_| poisoned())
    }

    fn lock_sessions(&self) -> Result<MutexGuard<'_, SessionManager>, ServiceError> {
        self.sessions.lock().map_err(|_| poisoned())
    }

    fn lock_registry(&self) -> Result<MutexGuard<'_, Registry>, ServiceError> {
        self.registry.lock().map_err(|_| poisoned())
    }
}

/// Returns the canonical digest of one request's operation name and validated input.
///
/// The digest binds a request key to the **arguments** it was first used with, so a
/// reused key with different arguments is a typed conflict instead of a silent
/// re-apply (ADR-0016 §2, K3). It covers only the operation name and the input
/// object, never host time or the process epoch, so a retry after a restart hashes
/// the same.
fn request_digest(
    op: &str,
    input: &JsonValue,
    limits: &ServiceLimits,
) -> Result<StateHash, ServiceError> {
    let digest_input = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
    let bytes = crate::json::to_canonical_bytes(&digest_input, limits)?;
    Ok(hash_bytes(&bytes))
}

/// Rebuilds the typed error from its canonical envelope.
///
/// [`AppService::dispatch`] stores the envelope bytes for reconciliation and must
/// still return the typed error, so the code, message and retryable flag are read
/// back from the document it just serialized.
fn restore_error(value: &JsonValue) -> ServiceError {
    let error = value.get("error");
    let code = error
        .and_then(|error| error.get("code"))
        .and_then(JsonValue::as_str)
        .and_then(ErrorCode::parse)
        .unwrap_or(ErrorCode::Internal);
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(JsonValue::as_str)
        .unwrap_or("operation failed")
        .to_owned();
    let retryable = error
        .and_then(|error| error.get("retryable"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    ServiceError::new(code, message).with_retryable(retryable)
}

/// Returns the recorded stop reason of one finished journal entry.
fn stop_reason_of(progress: &OperationProgress) -> Option<String> {
    if progress.state != OperationState::Finished || progress.failed {
        return None;
    }
    progress.stop_reason.clone()
}

/// Projects one canonical response document onto the requested wire version.
///
/// The document is `{"op": <name>, "result": {...}}`; a v1 result removes exactly the
/// fields v2 added (ADR-0016 §Compatibility). Re-serializing keeps the canonical byte
/// form, so a v1 client and a v2 client each see a stable document.
fn project_outcome(
    op: &str,
    outcome: Vec<u8>,
    version: ApiVersion,
    limits: &ServiceLimits,
) -> Result<Vec<u8>, ServiceError> {
    if version.is_v2() {
        return Ok(outcome);
    }
    let document = crate::json::parse(&outcome, limits)?;
    let Some(envelope) = document.as_object() else {
        return Ok(outcome);
    };
    let Some(result) = envelope.get("result") else {
        return Ok(outcome);
    };
    let projected = project_v1_result(op, result.clone());
    let mut fields = envelope.clone();
    fields.insert("result".to_owned(), projected);
    crate::json::to_canonical_bytes(&JsonValue::Object(fields), limits)
}

/// Removes the v2-only fields from one v1 result, including the list entries.
fn project_v1_result(op: &str, result: JsonValue) -> JsonValue {
    let JsonValue::Object(mut fields) = result else {
        return result;
    };
    for name in crate::version::v1_result_fields(op) {
        fields.remove(*name);
    }
    // The operational fields v2 adds to **every** mutation result are projected away
    // unconditionally: they are not part of the v1 document for any operation.
    for name in crate::version::V2_ONLY_RESULT_FIELDS {
        fields.remove(name);
    }
    if op == "session.list" {
        if let Some(JsonValue::Array(entries)) = fields.get("sessions").cloned() {
            let projected = entries
                .into_iter()
                .map(|entry| match entry {
                    JsonValue::Object(mut row) => {
                        row.remove("running");
                        JsonValue::Object(row)
                    }
                    other => other,
                })
                .collect();
            fields.insert("sessions".to_owned(), JsonValue::Array(projected));
        }
    }
    JsonValue::Object(fields)
}

fn request_parts(document: &JsonValue) -> Result<(&str, &JsonValue), ServiceError> {
    let object = document
        .as_object()
        .ok_or_else(|| invalid("request must be a JSON object"))?;
    for key in object.keys() {
        if key != "op" && key != "input" {
            // The client-supplied key is never echoed: its length is client
            // controlled and would make the error envelope unbounded.
            return Err(invalid("unknown field in the request"));
        }
    }
    let op = object
        .get("op")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid("`op` must be a string"))?;
    let input = object
        .get("input")
        .ok_or_else(|| invalid("missing required field `input`"))?;
    if input.as_object().is_none() {
        return Err(invalid("`input` must be a JSON object"));
    }
    Ok((op, input))
}

/// Renders the cheap per-session metadata `list` returns.
///
/// Only fields the committed metadata already holds are copied: state, tick, state
/// digest, lineage and the checkpoint count. Machine memory is never copied and no
/// digest is recomputed (I5).
fn summary_json(
    session: SessionId,
    committed: &SessionMetadata,
    lineage: JsonValue,
    running: bool,
) -> Result<JsonValue, ServiceError> {
    Ok(JsonValue::object([
        ("session", unsigned(session.raw())?),
        ("running", JsonValue::from_bool(running)),
        ("state", JsonValue::string(state_name(committed.state))),
        ("tick", unsigned(committed.tick.raw())?),
        (
            "state_hash",
            JsonValue::string(committed.state_hash.to_hex()),
        ),
        ("lineage", lineage),
    ]))
}

/// Copies the committed metadata of one locked session.
///
/// The checkpoint usage is left at its `None` default: a caller that is committing a
/// state change (not a checkpoint change) keeps the usage it already published.
fn metadata_of(view: &SessionInspection) -> SessionMetadata {
    metadata_with_usage(view, None)
}

/// Copies the committed metadata of one locked session, including checkpoint bytes.
///
/// The byte count is taken from the session that holds the checkpoints, which is the
/// accounting source ADR-0016 §3 names, and published with the rest of the commit so
/// a quota rejection and the account it is checked against cannot disagree.
fn metadata_with_usage(view: &SessionInspection, snapshot_bytes: Option<usize>) -> SessionMetadata {
    SessionMetadata {
        state: view.state,
        tick: view.machine.tick,
        state_hash: view.machine.state_hash,
        lineage: view.lineage.clone(),
        snapshot_bytes: snapshot_bytes.unwrap_or_default(),
    }
}

fn lineage_json(lineage: &Lineage) -> Result<JsonValue, ServiceError> {
    Ok(JsonValue::object([
        (
            "parent_session",
            optional_unsigned(lineage.parent_session().map(SessionId::raw))?,
        ),
        (
            "origin_snapshot",
            optional_unsigned(lineage.origin_snapshot().map(SnapshotId::raw))?,
        ),
        ("depth", unsigned(u64::from(lineage.depth()))?),
    ]))
}

fn state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Ready => "ready",
        SessionState::Paused => "paused",
        SessionState::Closed => "closed",
    }
}

fn bytes_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Replaces the best-effort global checkpoint-byte counter with `used`.
///
/// The counter is written under the registry lock (admission, fork, close, reap) so
/// it stays consistent with the committed per-session usage. It is **not** an
/// enforcement point: ADR-0016 §3 declares the global store-byte quota best-effort
/// and outside the acceptance gate, because a strict cross-key reservation would have
/// to be taken while a session owner is held and would invert the documented lock
/// order (I1). It exists to make the global accounting honest and observable.
fn record_store_bytes(counter: &AtomicUsize, used: usize) {
    counter.store(used, Ordering::Release);
}

fn session_limit() -> ServiceError {
    ServiceError::new(ErrorCode::LimitExceeded, "session limit reached")
}

fn invalid(message: &'static str) -> ServiceError {
    ServiceError::new(ErrorCode::InvalidRequest, message)
}

fn poisoned() -> ServiceError {
    ServiceError::internal("service lock poisoned")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_test_backend::SyntheticBackendFactory;

    use super::{AppService, ServiceConfig, StopReason};
    use crate::auth::{Principal, Scope};
    use crate::error::{ErrorCode, WIRE_VERSION};
    use crate::json::{JsonValue, parse, to_canonical_bytes};
    use crate::limits::ServiceLimits;

    fn service(limits: ServiceLimits) -> AppService {
        AppService::new(ServiceConfig {
            factory: Arc::new(SyntheticBackendFactory::default()),
            storage_root: None,
            artifacts_root: None,
            limits,
        })
        .expect("service")
    }

    fn document(op: &str, input: &str) -> Vec<u8> {
        let limits = ServiceLimits::default();
        let value = JsonValue::object([
            ("op", JsonValue::string(op)),
            (
                "input",
                parse(input.as_bytes(), &limits).expect("input object"),
            ),
        ]);
        to_canonical_bytes(&value, &limits).expect("request bytes")
    }

    fn operator() -> Principal {
        Principal::new("operator", [Scope::Read, Scope::Write])
    }

    #[test]
    fn dispatch_returns_a_canonical_envelope() {
        let service = service(ServiceLimits::default());
        let bytes = service
            .dispatch(&operator(), &document("capabilities", "{}"))
            .expect("capabilities");
        let envelope = parse(&bytes, &ServiceLimits::default()).expect("envelope");
        assert_eq!(
            envelope.get("op").and_then(JsonValue::as_str),
            Some("capabilities")
        );
        assert!(
            envelope
                .get("result")
                .and_then(JsonValue::as_object)
                .is_some()
        );
        assert_eq!(
            to_canonical_bytes(&envelope, &ServiceLimits::default()).expect("bytes"),
            bytes
        );
    }

    #[test]
    fn unknown_operations_and_malformed_requests_are_typed() {
        let service = service(ServiceLimits::default());
        assert_eq!(
            service
                .dispatch(&operator(), &document("session.nope", "{}"))
                .expect_err("unknown op")
                .code(),
            ErrorCode::UnknownOp
        );
        assert_eq!(
            service
                .dispatch(&operator(), b"{}")
                .expect_err("missing fields")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            service
                .dispatch(&operator(), b"{\"op\":1,\"input\":{}}")
                .expect_err("mistyped op")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            service
                .dispatch(
                    &operator(),
                    b"{\"op\":\"capabilities\",\"input\":[],\"x\":1}"
                )
                .expect_err("unknown field")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            service
                .dispatch(&operator(), b"not json")
                .expect_err("parse failure")
                .code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn a_read_only_principal_cannot_mutate() {
        let service = service(ServiceLimits::default());
        let reader = Principal::new("reader", [Scope::Read]);
        assert_eq!(
            service
                .dispatch(&reader, &document("session.create", "{}"))
                .expect_err("forbidden")
                .code(),
            ErrorCode::Forbidden
        );
        assert!(
            service
                .dispatch(&reader, &document("capabilities", "{}"))
                .is_ok(),
            "read scope must allow read operations"
        );
    }

    #[test]
    fn session_count_counts_live_sessions_only() {
        let service = service(ServiceLimits::default());
        assert_eq!(service.session_count().expect("count"), 0);
        let bytes = service
            .dispatch(&operator(), &document("session.create", "{}"))
            .expect("create");
        let envelope = parse(&bytes, &ServiceLimits::default()).expect("envelope");
        assert_eq!(
            envelope
                .get("result")
                .and_then(|result| result.get("session"))
                .and_then(JsonValue::as_u64),
            Some(1)
        );
        assert_eq!(service.session_count().expect("count"), 1);
    }

    #[test]
    fn stop_reasons_are_stable_strings_in_the_documented_precedence_order() {
        // The wire strings are contract: a decoder branches on them, so a rename is a
        // breaking change. The order is the same-boundary total order of ADR-0016
        // §New limits (`cancelled` > `deadline` > `halted` > `stop_condition` >
        // `completed`); `stop_condition` is the W3.4 slot and is not emitted yet.
        let ordered = [
            StopReason::Cancelled,
            StopReason::Deadline,
            StopReason::Halted,
            StopReason::Completed,
        ];
        let names: Vec<&str> = ordered.iter().map(|reason| reason.as_str()).collect();
        assert_eq!(names, ["cancelled", "deadline", "halted", "completed"]);
    }

    #[test]
    fn a_completed_run_reports_the_new_v2_shape() {
        let service = service(ServiceLimits::default());
        let operator = operator();
        let created = service
            .dispatch(&operator, &document("session.create", "{}"))
            .expect("create");
        let session = parse(&created, &ServiceLimits::default())
            .expect("envelope")
            .get("result")
            .and_then(|result| result.get("session"))
            .and_then(JsonValue::as_u64)
            .expect("session id");
        // This test asserts the v2 shape, so it declares v2 instead of the v1 default.
        let run = service
            .dispatch_versioned(
                &operator,
                &document(
                    "session.run",
                    &format!("{{\"session\":{session},\"steps\":4}}"),
                ),
                crate::version::ApiVersion::V2,
            )
            .expect("run");
        let result = parse(&run, &ServiceLimits::default()).expect("envelope");
        let result = result.get("result").expect("result");
        assert_eq!(result.get("performed").and_then(JsonValue::as_u64), Some(4));
        assert_eq!(
            result.get("stop_reason").and_then(JsonValue::as_str),
            Some("completed")
        );
        assert_eq!(result.get("revision").and_then(JsonValue::as_u64), Some(1));
        assert!(
            result.get("elapsed_ms").is_none(),
            "a host duration never enters a reproducible document"
        );
        // The process epoch **is** published in a v2 mutation result: it is operational
        // metadata a caller reconciles against, and byte comparisons neutralize it
        // instead of treating it as emulated state (ADR-0016 §2, F2). The v1 shape
        // projects it away (see the v1 test below).
        assert_eq!(
            result.get("process_epoch").and_then(JsonValue::as_u64),
            Some(service.process_epoch()),
            "the v2 mutation result names the process a reconciliation belongs to"
        );
        assert_eq!(
            result.get("max_wall_ms").and_then(JsonValue::as_u64),
            Some(ServiceLimits::default().max_run_wall_ms)
        );
    }

    #[test]
    fn capabilities_declare_the_wire_version_and_the_control_limits() {
        let service = service(ServiceLimits::default());
        let bytes = service
            .dispatch_versioned(
                &operator(),
                &document("capabilities", "{}"),
                crate::version::ApiVersion::V2,
            )
            .expect("capabilities");
        let envelope = parse(&bytes, &ServiceLimits::default()).expect("envelope");
        let result = envelope.get("result").expect("result");
        assert_eq!(
            result.get("api_version").and_then(JsonValue::as_u64),
            Some(WIRE_VERSION)
        );
        // The v2 capabilities publishes the process epoch (operational metadata, not
        // emulated state): a client compares it before trusting a reconciled result
        // (ADR-0016 §2).
        assert!(
            result
                .get("process_epoch")
                .and_then(JsonValue::as_u64)
                .is_some_and(|epoch| epoch > 0)
        );
        let limits = result.get("limits").expect("limits");
        assert_eq!(
            limits.get("max_run_wall_ms").and_then(JsonValue::as_u64),
            Some(30_000)
        );
        assert_eq!(
            limits
                .get("max_operation_journal")
                .and_then(JsonValue::as_u64),
            Some(1_024)
        );
        assert_eq!(
            limits.get("operation_ttl").and_then(JsonValue::as_u64),
            Some(600)
        );
    }

    #[test]
    fn the_v1_wire_serves_the_original_shapes_and_refuses_v2_surface() {
        let service = service(ServiceLimits::default());
        // v1 capabilities: no version marker and no epoch — the pre-ADR-0016 document a
        // legacy decoder expects.
        let bytes = service
            .dispatch(&operator(), &document("capabilities", "{}"))
            .expect("capabilities");
        let envelope = parse(&bytes, &ServiceLimits::default()).expect("envelope");
        let result = envelope.get("result").expect("result");
        assert!(result.get("api_version").is_none());
        assert!(result.get("process_epoch").is_none());
        // The limits table is shared by both wires (it is not a shape addition).
        assert!(result.get("limits").is_some());

        // v1 run: the original four fields, without the v2 reconciliation additions.
        let created = service
            .dispatch(&operator(), &document("session.create", "{}"))
            .expect("create");
        let session = parse(&created, &ServiceLimits::default())
            .expect("envelope")
            .get("result")
            .and_then(|result| result.get("session"))
            .and_then(JsonValue::as_u64)
            .expect("session id");
        let run = service
            .dispatch(
                &operator(),
                &document(
                    "session.run",
                    &format!("{{\"session\":{session},\"steps\":2}}"),
                ),
            )
            .expect("run");
        let run = parse(&run, &ServiceLimits::default()).expect("envelope");
        let run = run.get("result").expect("result");
        assert_eq!(run.get("steps").and_then(JsonValue::as_u64), Some(2));
        assert!(run.get("performed").is_none());
        assert!(run.get("stop_reason").is_none());
        assert!(run.get("revision").is_none());
        assert!(run.get("max_wall_ms").is_none());

        // A v2-only operation does not exist on the v1 wire.
        assert_eq!(
            service
                .dispatch(&operator(), &document("session.progress", "{}"))
                .expect_err("v2-only op")
                .code(),
            ErrorCode::UnknownOp
        );
        // A v2-only input field is not part of a v1 request.
        assert_eq!(
            service
                .dispatch(
                    &operator(),
                    &document("session.create", "{\"operation_id\":7}"),
                )
                .expect_err("v2-only field")
                .code(),
            ErrorCode::InvalidRequest
        );
        // The same calls are accepted when the request declares v2.
        assert!(
            service
                .dispatch_versioned(
                    &operator(),
                    &document("session.create", "{\"operation_id\":7}"),
                    crate::version::ApiVersion::V2,
                )
                .is_ok()
        );
        assert_eq!(
            service
                .dispatch_versioned(
                    &operator(),
                    // A key no request ever used: the control read reports an unknown
                    // outcome rather than inventing one (the key `7` above belongs to
                    // the create and would legitimately replay its journaled progress).
                    &document("session.progress", "{\"operation\":999}"),
                    crate::version::ApiVersion::V2,
                )
                .expect_err("unknown operation id")
                .code(),
            ErrorCode::UnknownOutcome
        );
    }

    #[test]
    fn the_api_version_selector_round_trips_and_rejects_unknown_values() {
        assert_eq!(
            crate::version::ApiVersion::parse("1"),
            Some(crate::version::ApiVersion::V1)
        );
        assert_eq!(
            crate::version::ApiVersion::parse(" 2 "),
            Some(crate::version::ApiVersion::V2)
        );
        assert_eq!(crate::version::ApiVersion::parse("3"), None);
        assert_eq!(crate::version::ApiVersion::parse(""), None);
        assert_eq!(crate::version::ApiVersion::V1.number(), 1);
        assert_eq!(crate::version::ApiVersion::V2.number(), 2);
        assert!(crate::version::ApiVersion::V2.is_v2());
        assert!(!crate::version::ApiVersion::V1.is_v2());
    }

    #[test]
    fn an_error_envelope_declares_its_wire_version() {
        let service = service(ServiceLimits::default());
        let error = service
            .dispatch(&operator(), &document("session.nope", "{}"))
            .expect_err("unknown op");
        let json = error.to_json();
        assert_eq!(
            json.get("wire_version").and_then(JsonValue::as_u64),
            Some(WIRE_VERSION)
        );
        assert_eq!(
            json.get("schema").and_then(JsonValue::as_str),
            Some(crate::error::ERROR_SCHEMA)
        );
        assert_eq!(
            ErrorCode::parse(error.code().as_str()),
            Some(error.code()),
            "every code round-trips through its wire string"
        );
        for code in [
            ErrorCode::Cancelled,
            ErrorCode::Deadline,
            ErrorCode::UnknownOutcome,
        ] {
            assert_eq!(ErrorCode::parse(code.as_str()), Some(code));
            assert!(!code.as_str().is_empty());
        }
        assert_eq!(ErrorCode::parse("not_a_code"), None);
    }
}
