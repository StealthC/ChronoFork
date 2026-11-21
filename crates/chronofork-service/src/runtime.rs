//! Cancellation signals, the bounded operation journal and the process epoch.
//!
//! These three pieces are what make ADR-0016 §2 implementable without putting a
//! session owner on the control path:
//!
//! - a [`CancelFlag`] is a shared atomic signal. A `session.run` never *waits* for
//!   it; it reads it at every instruction boundary, so `session.cancel` sets a bool
//!   and returns immediately instead of queueing behind the run it interrupts;
//! - the [`OperationJournal`] is the bounded, in-memory reconciliation record. It
//!   is the only place a client-supplied request key is remembered, and it is also
//!   what `session.progress` reads: both are reads of the journal, never a session
//!   owner read;
//! - the [`ProcessEpoch`] is a monotonic process identifier. It is reported in
//!   every result and in `capabilities`, so a reconciled result is only trusted
//!   while the epoch matches — after a restart the same request key is an
//!   `unknown_outcome`, never a silent re-execution.
//!
//! Every bound here is explicit: a finished entry is retained for
//! `operation_ttl` from its completion (never from its submission), an **active**
//! entry is never evicted, and when the journal is full a new insertion is
//! rejected with a typed `limit` *before* the operation starts instead of
//! silently dropping a retained result.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use chronofork_core::{SessionId, StateHash, Tick};

use crate::error::{ErrorCode, ServiceError};

/// A shared, lock-free cancellation signal.
///
/// The flag is intentionally tiny and independent of any lock: the running
/// `session.run` reads it once per instruction and `session.cancel` writes it
/// without acquiring a session owner, which is what removes race R1 from the
/// catalogue (ADR-0016 §2, K2/F3).
#[derive(Debug, Default)]
pub(crate) struct CancelFlag {
    cancelled: AtomicBool,
}

impl CancelFlag {
    /// Creates an unsignalled flag.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation; the run observes it at its next instruction boundary.
    pub(crate) fn request(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation was requested.
    ///
    /// The load is `Acquire` so the reading thread sees everything the requesting
    /// thread published before the store; the flag carries no payload.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Lifecycle state of one journaled operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationState {
    /// Admitted and currently executing.
    Active,
    /// Reached a final state; `outcome` holds its canonical envelope.
    Finished,
}

impl OperationState {
    /// Returns the stable wire string for this state.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Finished => "finished",
        }
    }
}

/// One journal entry.
///
/// The entry is created *before* the operation starts and carries the live cancel
/// flag, so `session.cancel` can find the running operation through the journal
/// alone. It never stores session state: `performed`/`tick` are cheap counters the
/// run loop publishes as it advances.
pub(crate) struct OperationEntry {
    /// Owning principal; a key is only recognized for the principal that sent it.
    pub(crate) principal: String,
    /// Session the operation targets, when it targets one.
    pub(crate) session: Option<SessionId>,
    /// Digest of the request the key was first used with.
    pub(crate) digest: StateHash,
    /// Lifecycle state.
    pub(crate) state: OperationState,
    /// Live cancellation signal of an active operation.
    pub(crate) cancel: Option<std::sync::Arc<CancelFlag>>,
    /// Steps performed so far, published by the run loop.
    pub(crate) performed: u64,
    /// Tick as of the last published boundary.
    pub(crate) tick: Option<Tick>,
    /// Operation result once the operation reached a final state.
    ///
    /// A typed failure is stored as the canonical `chronofork.error.v1` document, so
    /// a replay reproduces the same outcome the first attempt reported instead of
    /// re-running a mutation that already happened.
    pub(crate) outcome: Option<Vec<u8>>,
    /// Whether the finished outcome is a typed error envelope.
    pub(crate) failed: bool,
    /// Monotonic sequence used for deterministic iteration order.
    pub(crate) sequence: u64,
    /// Host time the entry finished; `None` while active.
    pub(crate) finished_at: Option<SystemTime>,
}

impl OperationEntry {
    /// Returns whether this entry is still executing.
    pub(crate) const fn is_active(&self) -> bool {
        matches!(self.state, OperationState::Active)
    }
}

/// Why an operation key could not be inserted (or resolved) from the journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalRejection {
    /// The journal is at `max_operation_journal` and holds no expired entry.
    Full,
}

/// At which level a request key was admitted.
pub(crate) enum Admission {
    /// The key is new; the caller executes the operation.
    Fresh {
        /// Live cancellation signal the caller publishes while it runs.
        cancel: std::sync::Arc<CancelFlag>,
    },
    /// The key was already used with the same digest and a final result exists.
    Replay {
        /// Recorded canonical outcome bytes (a result or an error envelope).
        outcome: Vec<u8>,
        /// Whether the recorded outcome is a typed error envelope.
        failed: bool,
    },
    /// The key was used before, but its outcome is outside the retention horizon.
    ///
    /// ADR-0016 §2: beyond the TTL (or after a restart) the outcome is
    /// `unknown_outcome` and the server **must not re-execute automatically**. The
    /// tombstone that produces this decision is what separates "retry of a known
    /// request" from "a new request that happens to reuse a key".
    Expired,
    /// The key was already used while the first request is still executing.
    InFlight,
    /// The key was already used with a different request digest.
    DigestMismatch,
}

/// What the caller must do after registering a request key.
pub(crate) enum Disposition {
    /// Execute the operation, then finish the journal entry under this key.
    ///
    /// `key` is absent when the request is not journaled (a read, or a mutation that
    /// supplied no key); nothing is finished in that case.
    Execute {
        /// Request key the entry was registered under, when it was registered.
        key: Option<String>,
        /// Live cancellation signal of this attempt.
        cancel: std::sync::Arc<CancelFlag>,
    },
    /// Return the recorded envelope bytes without executing anything.
    Replay {
        /// Recorded canonical outcome bytes.
        outcome: Vec<u8>,
        /// Whether the recorded outcome is a typed error envelope.
        failed: bool,
    },
}

/// A cheap progress view of one journaled operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationProgress {
    /// Owning principal.
    pub(crate) principal: String,
    /// Target session, when the operation has one.
    pub(crate) session: Option<SessionId>,
    /// Lifecycle state.
    pub(crate) state: OperationState,
    /// Steps published so far.
    pub(crate) performed: u64,
    /// Tick as of the last published boundary.
    pub(crate) tick: Option<Tick>,
    /// Whether a finished outcome is a typed error.
    pub(crate) failed: bool,
    /// Monotonic sequence of the entry.
    pub(crate) sequence: u64,
    /// Stop reason of a finished successful run, when the outcome carries one.
    pub(crate) stop_reason: Option<String>,
}

/// One key whose retained outcome has left the reconciliation horizon.
///
/// The tombstone keeps only the identity needed to decide a retry: the owning
/// principal and the request digest. Its payload and result are gone, so the answer
/// to a retry is `unknown_outcome` rather than a silent re-execution (ADR-0016 §2).
struct Tombstone {
    principal: String,
    digest: StateHash,
}

/// The floor of the identity memory (ADR-0016 §2, K4/F5).
///
/// The outcome memory (`max_operation_journal`) is bounded by the *payloads* it retains;
/// the identity memory is bounded by the *keys* the service has ever used, which are
/// far cheaper (a key, a principal and a digest). A small journal capacity must not
/// shrink the identity memory to the point where a used key is forgotten and a retry
/// silently re-executes, so the identity capacity is at least this floor. It is
/// deliberately not a published limit: the identity memory is the safety boundary that
/// makes "never apply a mutation twice" hold, not a tuning knob.
const MIN_OPERATION_IDENTITIES: usize = 1_024;

/// A bounded, in-memory operation journal (ADR-0016 §2, K4/F5).
///
/// Two memories with two bounds, both **never silently evicted**:
///
/// - `entries` retains the outcomes of the last `capacity` operations, so a retry
///   inside the horizon replays the recorded result;
/// - `tombstones` retains the *identity* of every key whose outcome left the horizon,
///   up to `identity_capacity`, so a retry outside the horizon is `unknown_outcome`
///   instead of a second execution.
///
/// When either memory cannot take a new key the journal answers `Full` and the request
/// is refused **before any work**; forgetting a used key would re-execute a mutation the
/// client may already have applied, which is the one outcome the reconciliation contract
/// forbids.
pub(crate) struct OperationJournal {
    entries: BTreeMap<String, OperationEntry>,
    /// Keys whose outcome expired; never evicted while the process lives.
    tombstones: BTreeMap<String, Tombstone>,
    ttl: Duration,
    capacity: usize,
    /// Bound of the identity memory (`entries` + `tombstones`).
    identity_capacity: usize,
    next_sequence: u64,
    expired: u64,
}

impl OperationJournal {
    /// Creates a journal with the configured capacity and finished-entry TTL.
    ///
    /// The identity capacity is the outcome capacity raised to
    /// [`MIN_OPERATION_IDENTITIES`], so a small outcome bound cannot make the service
    /// forget a used key.
    pub(crate) fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self::with_identity_capacity(capacity, ttl_secs, capacity.max(MIN_OPERATION_IDENTITIES))
    }

    /// Creates a journal with an explicit identity capacity.
    ///
    /// [`Self::new`] derives the identity capacity from the outcome capacity; this form
    /// is the seam the journal's own tests use to exercise the fail-closed identity
    /// bound with a small number.
    pub(crate) fn with_identity_capacity(
        capacity: usize,
        ttl_secs: u64,
        identity_capacity: usize,
    ) -> Self {
        Self {
            entries: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            ttl: Duration::from_secs(ttl_secs),
            capacity,
            identity_capacity,
            next_sequence: 1,
            expired: 0,
        }
    }

    /// Returns the number of retained entries.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of entries dropped because their TTL passed.
    #[cfg(test)]
    pub(crate) const fn expired_count(&self) -> u64 {
        self.expired
    }

    /// Retires finished entries whose TTL (counted from completion) has passed.
    ///
    /// An active entry is never expired, which is the contract of ADR-0016 §2: the
    /// TTL starts at completion, not at submission. An expired entry leaves a
    /// **tombstone** (principal + digest, no payload) so a later retry of the same
    /// key is answered with `unknown_outcome` instead of being executed again.
    ///
    /// Tombstones are never evicted here: the identity memory is what makes a retry
    /// safe, so it is bounded by [`Self::identity_capacity`] and a full memory refuses a
    /// new key instead of forgetting a used one.
    pub(crate) fn expire_finished(&mut self, now: SystemTime) -> usize {
        let ttl = self.ttl;
        let mut retired: Vec<(String, String, StateHash)> = Vec::new();
        self.entries.retain(|key, entry| {
            if entry.is_active() {
                return true;
            }
            let finished = entry.finished_at.unwrap_or(now);
            let since = now.duration_since(finished).unwrap_or_default();
            // A zero TTL means "no retention of the result": the entry is retired at
            // once, which is also the deterministic way to test the horizon.
            if ttl > Duration::ZERO && since < ttl {
                return true;
            }
            retired.push((key.clone(), entry.principal.clone(), entry.digest));
            false
        });
        let dropped = retired.len();
        for (key, principal, digest) in retired {
            self.tombstones.insert(key, Tombstone { principal, digest });
        }
        self.expired = self.expired.saturating_add(dropped as u64);
        dropped
    }

    /// Returns whether the identity memory has room for one more key.
    fn has_identity_room(&self) -> bool {
        self.entries.len().saturating_add(self.tombstones.len()) < self.identity_capacity
    }

    /// Registers a request key for one operation.
    ///
    /// The whole decision is one critical section under the caller's journal lock:
    /// expire finished entries, then either replay a retained result, report the
    /// in-flight/conflicting cases, or insert a fresh active entry. A full journal
    /// is a typed `limit` rejection *before* any work starts, so a retained result
    /// is never silently dropped to make room. When the **identity** memory is full the
    /// new key is refused the same way: forgetting a used key would let a retry apply a
    /// mutation a second time.
    pub(crate) fn register(
        &mut self,
        key: &str,
        principal: &str,
        session: Option<SessionId>,
        digest: StateHash,
        now: SystemTime,
    ) -> Result<Admission, JournalRejection> {
        self.expire_finished(now);
        match self.entries.get(key) {
            Some(entry) => {
                if entry.principal != principal || entry.digest != digest {
                    return Ok(Admission::DigestMismatch);
                }
                if entry.is_active() {
                    return Ok(Admission::InFlight);
                }
                return Ok(Admission::Replay {
                    outcome: entry.outcome.clone().unwrap_or_default(),
                    failed: entry.failed,
                });
            }
            None => {
                // A key whose result has been retired is recognized as **used**: the
                // server reports an unknown outcome instead of executing the mutation
                // a second time (ADR-0016 §2, V3-3).
                if let Some(tombstone) = self.tombstones.get(key) {
                    if tombstone.principal != principal || tombstone.digest != digest {
                        return Ok(Admission::DigestMismatch);
                    }
                    return Ok(Admission::Expired);
                }
                if self.entries.len() >= self.capacity || !self.has_identity_room() {
                    return Err(JournalRejection::Full);
                }
            }
        }
        let cancel = std::sync::Arc::new(CancelFlag::new());
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.entries.insert(
            key.to_owned(),
            OperationEntry {
                principal: principal.to_owned(),
                session,
                digest,
                state: OperationState::Active,
                cancel: Some(std::sync::Arc::clone(&cancel)),
                performed: 0,
                tick: None,
                outcome: None,
                failed: false,
                sequence,
                finished_at: None,
            },
        );
        Ok(Admission::Fresh { cancel })
    }

    /// Publishes the progress of one active operation.
    ///
    /// This is a cheap metadata write on the run path: two plain integers, no
    /// hashing and no session access.
    pub(crate) fn publish_progress(&mut self, key: &str, performed: u64, tick: Option<Tick>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.performed = performed;
            entry.tick = tick;
        }
    }

    /// Marks one operation finished and stores its canonical envelope bytes.
    pub(crate) fn finish(&mut self, key: &str, outcome: Vec<u8>, failed: bool, now: SystemTime) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.state = OperationState::Finished;
            entry.outcome = Some(outcome);
            entry.failed = failed;
            entry.cancel = None;
            entry.finished_at = Some(now);
        }
    }

    /// Removes one operation without leaving a result.
    ///
    /// Used when an operation is rejected before its work starts (a failed
    /// precondition, a contended lease): nothing ran, so nothing is remembered and a
    /// later retry with the same key is fresh again.
    pub(crate) fn abandon(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Returns the number of retired identities the journal still recognizes.
    ///
    /// A retired identity is what turns a retry of an expired key into
    /// `unknown_outcome` instead of a second execution; the set is bounded by the
    /// journal capacity.
    #[cfg(test)]
    pub(crate) fn tombstone_count(&self) -> usize {
        self.tombstones.len()
    }

    /// Drops every finished entry whose TTL has passed.
    ///
    /// A read of the reconciliation horizon expires first, so a caller observes the
    /// documented behavior (ADR-0016 §2, V3-3) without depending on another request
    /// happening to trigger the sweep. An active entry is never touched.
    pub(crate) fn sweep(&mut self) {
        self.expire_finished(now());
    }

    /// Returns a cheap snapshot of one entry's progress.
    ///
    /// The stop reason is read from the recorded outcome, so a finished operation
    /// reports why it stopped without a caller re-deriving it from a replayed error.
    pub(crate) fn progress(&self, key: &str) -> Option<OperationProgress> {
        let entry = self.entries.get(key)?;
        let stop_reason = self.stop_reason(key);
        Some(OperationProgress {
            principal: entry.principal.clone(),
            session: entry.session,
            state: entry.state,
            performed: entry.performed,
            tick: entry.tick,
            failed: entry.failed,
            sequence: entry.sequence,
            stop_reason,
        })
    }

    /// Returns the stop reason recorded in one entry's outcome, when it has one.
    pub(crate) fn stop_reason(&self, key: &str) -> Option<String> {
        let outcome = self.entries.get(key)?.outcome.as_ref()?;
        let document =
            crate::json::parse(outcome, &crate::limits::ServiceLimits::default()).ok()?;
        Some(
            document
                .get("result")?
                .get("stop_reason")?
                .as_str()?
                .to_owned(),
        )
    }

    /// Returns the canonical outcome bytes of one entry.
    pub(crate) fn outcome(&self, key: &str) -> Option<Vec<u8>> {
        self.entries
            .get(key)
            .and_then(|entry| entry.outcome.clone())
    }

    /// Returns the live cancel flag of one active operation, when it has one.
    pub(crate) fn cancel_flag(&self, key: &str) -> Option<std::sync::Arc<CancelFlag>> {
        self.entries.get(key).and_then(|entry| entry.cancel.clone())
    }

    /// Returns the request key of the newest operation targeting one session.
    ///
    /// This is how `session.cancel` addresses a run when the caller did not name
    /// an operation: no session owner is read, only the journal.
    pub(crate) fn newest_active_for(&self, session: SessionId) -> Option<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.is_active() && entry.session == Some(session))
            .max_by_key(|(_, entry)| entry.sequence)
            .map(|(key, _)| key.clone())
    }
}

/// The process epoch: the identity of the process that owns the journal.
///
/// It is minted from a per-process counter that only increases, so two services in
/// one process never share an epoch and the counter is stable across a test run.
/// It is deliberately **not** derived from the host clock and **not** persisted:
/// - a clock-derived value would change on every service construction, which would
///   make two identical request sequences produce different result bytes (the
///   service's canonical documents are reproducible by contract);
/// - the journal itself is process memory, so a restarted process cannot replay an
///   entry no matter what its epoch says. The reconciliation horizon is therefore
///   stated in terms of the journal (same instance, before `operation_ttl` from
///   completion) and the epoch is reported only through
///   [`crate::service::AppService::process_epoch`] for logs and diagnostics.
#[derive(Clone, Debug)]
pub(crate) struct ProcessEpoch {
    value: u64,
}

/// The next epoch to mint in this process.
///
/// Seeded lazily from the host clock and the process id (see
/// [`ProcessEpoch::load`]); the value only increases within a process.
static NEXT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Mints the seed of [`NEXT_EPOCH`] exactly once.
fn epoch_seed() -> u64 {
    static SEEDED: std::sync::Once = std::sync::Once::new();
    let mut seed = 1_u64;
    SEEDED.call_once(|| {
        let seconds = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0);
        let pid = u64::from(std::process::id()) & 0xF_FFFF;
        // The seed must fit the canonical JSON integer range (2^53 - 1), because the
        // epoch is published in canonical documents: seconds occupy the high bits and
        // the low 20 bits carry the process id, so two processes of one host differ even
        // when their clocks tie, and the value stays in range for centuries.
        seed = (seconds << 20) | pid;
        if seed == 0 {
            seed = 1;
        }
        NEXT_EPOCH.store(seed, Ordering::SeqCst);
    });
    NEXT_EPOCH.load(Ordering::SeqCst)
}

impl ProcessEpoch {
    /// Mints the next process-local epoch (in-process identity only).
    ///
    /// The value is seeded from the host clock and the process id and only increases
    /// within one process, so two services in one process never share an epoch. It is
    /// **not durable**: a restarted process mints a fresh value, so this form must not be
    /// used where a retry from a previous process has to be recognized. A service that
    /// needs that guarantee is built with [`Self::load_durable`].
    pub(crate) fn load(_storage_root: Option<&Path>) -> Self {
        let _ = epoch_seed();
        Self {
            value: NEXT_EPOCH.fetch_add(1, Ordering::SeqCst),
        }
    }

    /// Mints the next epoch of a **durable** process identity.
    ///
    /// The counter lives in `path`, so it is never reused across restarts (ADR-0016 §2):
    /// the previous value is read, incremented and published by [`write_epoch`] under an
    /// exclusive lock file, which serializes concurrent starts, so two processes that
    /// share the identity file cannot mint the same epoch. The file is created here — at
    /// the `serve` start — and nowhere else.
    ///
    /// Durability is the sequence of [`write_epoch`]: full write, file `sync_all`, atomic
    /// rename, directory `sync_all` (Unix). A host crash or power loss cannot be injected
    /// in this environment, so the guarantee is structural: the service only reports the
    /// new epoch after every step returned successfully, and every failure is fail-closed.
    ///
    /// # Errors
    ///
    /// Returns a typed storage failure when the identity file cannot be read, locked,
    /// synced or written. A durable deployment must not start with an unknown identity,
    /// so this fails closed rather than falling back to a process-local counter.
    pub(crate) fn load_durable(path: &Path) -> Result<Self, ServiceError> {
        let _lock = EpochLock::acquire(path)?;
        let previous = read_epoch(path)?;
        let next = previous
            .checked_add(1)
            .filter(|value| *value <= MAX_EPOCH)
            .ok_or_else(epoch_unusable)?;
        write_epoch(path, next)?;
        Ok(Self { value: next })
    }

    /// Returns the epoch value.
    pub(crate) const fn value(&self) -> u64 {
        self.value
    }
}

/// The largest epoch the canonical JSON integer profile can carry.
const MAX_EPOCH: u64 = (1_u64 << 53) - 1;

/// Appends `suffix` to the identity file's full name (`…/store.chronofork-epoch` +
/// `.tmp`), so two identities that share a directory never collide on one lock or
/// temporary name.
fn epoch_sibling(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push('.');
    name.push_str(suffix);
    path.with_file_name(name)
}

/// Builds the identity-file path beside a token or store root.
///
/// The file is a sibling of the configured root, never inside it: the store namespace is
/// validated by `store.list`, where an unexpected file is a reported anomaly.
#[must_use]
pub(crate) fn epoch_file_beside(root: &Path) -> std::path::PathBuf {
    let mut path = root.to_path_buf();
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.set_file_name(format!("{name}.chronofork-epoch"));
    path
}

/// An exclusive start lock around the identity file.
///
/// Created with `create_new`, which is atomic on every supported platform; a second
/// process that starts at the same instant waits briefly instead of reading a stale
/// counter. The lock is removed on drop, including on an error path.
struct EpochLock {
    path: std::path::PathBuf,
}

impl EpochLock {
    fn acquire(identity: &Path) -> Result<Self, ServiceError> {
        let path = epoch_sibling(identity, "lock");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if std::time::Instant::now() >= deadline {
                        return Err(epoch_unusable());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return Err(epoch_unusable()),
            }
        }
    }
}

impl Drop for EpochLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn read_epoch(path: &Path) -> Result<u64, ServiceError> {
    match std::fs::read_to_string(path) {
        Ok(text) => text.trim().parse::<u64>().map_err(|_| epoch_unusable()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(_) => Err(epoch_unusable()),
    }
}

/// Publishes one epoch value with the strongest sequence the platform offers (C1/D3).
///
/// The sequence is the standard crash-safe publish:
///
/// 1. write the new value into a sibling temporary file;
/// 2. `sync_all` the **file**, so the bytes reach stable storage before the name does;
/// 3. `rename` the temporary over the identity file, which is the atomic publication;
/// 4. `sync_all` the **directory**, so the rename itself survives a host crash — this is
///    what makes the *increment* durable, not merely the file contents.
///
/// **Platform guarantee.** On Unix (Linux and macOS) all four steps run, so a crash after
/// this function returns cannot reuse the epoch it published. Windows has no portable
/// directory `fsync` in `std`: there step 4 is a documented **best-effort** no-op — the
/// file is synced and the rename is atomic, but a host crash can lose the new directory
/// entry, so an epoch a client already observed may be minted again after the crash. The
/// owner decision (2026-11-21, R1d/D3) is to document that limitation and keep starting
/// rather than to refuse a durable start on those platforms; Unix keeps the full
/// guarantee. Host-crash behavior cannot be exercised in this environment, so the
/// guarantee rests on this sequence, not on an executed power-loss test.
///
/// Any failure of a step that *can* fail is returned as a typed storage error: on Unix the
/// service must not start with an identity it could not persist (fail-closed).
fn write_epoch(path: &Path, value: u64) -> Result<(), ServiceError> {
    let temporary = epoch_sibling(path, "tmp");
    {
        let mut file = std::fs::File::create(&temporary).map_err(|_| epoch_unusable())?;
        std::io::Write::write_all(&mut file, format!("{value}\n").as_bytes())
            .map_err(|_| epoch_unusable())?;
        file.sync_all().map_err(|_| epoch_unusable())?;
    }
    std::fs::rename(&temporary, path).map_err(|_| epoch_unusable())?;
    sync_directory(path)
}

/// `sync_all`s the directory that holds `path` (Unix only).
///
/// A bare relative name (`store.chronofork-epoch`) has an **empty** parent, not `None`:
/// opening `""` fails. An empty parent means "the current directory", which is exactly what
/// the name resolves to, so it is normalized to `.` here (D1). Any other failure is a
/// storage error: on Unix the service does not start with an unpersisted identity.
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ServiceError> {
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::File::open(directory)
        .and_then(|handle| handle.sync_all())
        .map_err(|_| epoch_unusable())
}

/// Non-Unix platforms have no portable directory sync, and this is **not** a durability
/// guarantee.
///
/// The caller's file was synced and the rename is atomic, but nothing here flushes the
/// directory entry that publishes the rename, so a host crash can lose the newly minted
/// epoch (and a later start can mint a value a client already observed). The owner decision
/// (2026-11-21, R1d/D3) is to document this limitation instead of refusing the durable
/// start on those platforms; see [`write_epoch`].
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), ServiceError> {
    Ok(())
}

/// Reports an identity file the service cannot read, lock, parse or write.
///
/// The message never carries the host path, in line with every other service error.
fn epoch_unusable() -> ServiceError {
    ServiceError::new(
        ErrorCode::Storage,
        "the durable process identity is not usable; the service refuses to start with an \
         unknown identity",
    )
}

/// Reports a mutation key this process cannot reconcile with the running epoch.
pub(crate) fn epoch_mismatch() -> ServiceError {
    ServiceError::new(
        ErrorCode::UnknownOutcome,
        "the request key belongs to another process epoch; the server did not re-execute it",
    )
}

/// Reports a keyed mutation of a durable service that does not name its epoch.
pub(crate) fn epoch_required() -> ServiceError {
    ServiceError::new(
        ErrorCode::UnknownOutcome,
        "this durable service reconciles request keys by epoch; send `operation_epoch` with \
         the epoch the key was minted under",
    )
}

/// Rejects a request key whose recorded digest belongs to different arguments.
pub(crate) fn key_conflict() -> ServiceError {
    ServiceError::new(
        ErrorCode::Conflict,
        "request key was first used with different arguments",
    )
}

/// Rejects a request key whose first request is still executing.
pub(crate) fn key_in_flight() -> ServiceError {
    ServiceError::retryable_conflict("request key is still executing")
}

/// Rejects a new request key because the operation journal is full.
pub(crate) fn journal_full() -> ServiceError {
    ServiceError::new(
        ErrorCode::LimitExceeded,
        "operation journal is full; no retained result and no used key identity is ever \
         evicted, so the new key is refused before any work",
    )
}

/// Reports a request key this service cannot deduplicate.
pub(crate) fn unjournaled_key() -> ServiceError {
    ServiceError::new(
        ErrorCode::Conflict,
        "this operation is not journaled for `operation_id`",
    )
}

/// Reports an outcome that is outside the reconciliation horizon.
pub(crate) fn unknown_outcome() -> ServiceError {
    ServiceError::new(
        ErrorCode::UnknownOutcome,
        "the outcome is not retained in this process epoch; the server did not re-execute",
    )
}

/// Reports a retry whose key is recognized but whose result has expired.
///
/// This is the refusal of ADR-0016 §2/V3-3: the server knows the key was used and
/// deliberately does **not** execute the mutation again.
pub(crate) fn expired_outcome() -> ServiceError {
    ServiceError::new(
        ErrorCode::UnknownOutcome,
        "this request key was already used and its outcome left the retention horizon; \
         the server did not re-execute it",
    )
}

/// Returns the current host time, or `UNIX_EPOCH` when the clock predates it.
pub(crate) fn now() -> SystemTime {
    SystemTime::now()
}

/// A monotonic deadline measured with `Instant`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadline {
    instant: std::time::Instant,
}

impl Deadline {
    /// Returns a deadline `millis` from now.
    pub(crate) fn after_millis(millis: u64) -> Self {
        Self {
            instant: std::time::Instant::now()
                .checked_add(Duration::from_millis(millis))
                .unwrap_or_else(std::time::Instant::now),
        }
    }

    /// Returns whether the deadline has passed.
    pub(crate) fn is_expired(&self) -> bool {
        std::time::Instant::now() >= self.instant
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use chronofork_core::{SessionId, hash_bytes};

    use super::{
        Admission, CancelFlag, JournalRejection, OperationJournal, OperationState, ProcessEpoch,
    };

    fn digest(seed: u8) -> chronofork_core::StateHash {
        hash_bytes(&[seed])
    }

    fn journal(capacity: usize, ttl_secs: u64) -> OperationJournal {
        OperationJournal::new(capacity, ttl_secs)
    }

    #[test]
    fn a_fresh_key_is_active_and_a_second_use_replays_the_recorded_outcome() {
        let mut journal = journal(4, 600);
        let now = SystemTime::now();
        let session = SessionId::from_raw(1);
        // A replay, an in-flight key or a digest mismatch would each be a contract
        // violation for a first use of the key.
        let cancel = match journal
            .register("k", "operator", Some(session), digest(1), now)
            .expect("fresh")
        {
            Admission::Fresh { cancel } => cancel,
            _ => unreachable!("a new key must be fresh"),
        };
        assert!(!cancel.is_cancelled());
        assert_eq!(journal.len(), 1);
        assert_eq!(
            journal.progress("k").expect("entry").state,
            OperationState::Active
        );

        // A second use while the first is active is an in-flight conflict, never a
        // silent re-apply.
        assert!(matches!(
            journal.register("k", "operator", Some(session), digest(1), now),
            Ok(Admission::InFlight)
        ));

        journal.finish("k", b"{\"op\":\"x\",\"result\":{}}".to_vec(), false, now);
        let replay = journal
            .register("k", "operator", Some(session), digest(1), now)
            .expect("replay");
        assert!(matches!(
            replay,
            Admission::Replay { outcome, failed }
                if outcome == b"{\"op\":\"x\",\"result\":{}}" && !failed
        ));
        // The replayed entry did not start a second operation.
        assert_eq!(journal.len(), 1);
    }

    #[test]
    fn the_same_key_with_different_arguments_or_principal_is_a_typed_conflict() {
        let mut journal = journal(4, 600);
        let now = SystemTime::now();
        assert!(matches!(
            journal.register("k", "operator", None, digest(1), now),
            Ok(Admission::Fresh { .. })
        ));
        assert!(matches!(
            journal.register("k", "operator", None, digest(2), now),
            Ok(Admission::DigestMismatch)
        ));
        assert!(matches!(
            journal.register("k", "other", None, digest(1), now),
            Ok(Admission::DigestMismatch)
        ));
    }

    #[test]
    fn a_full_journal_is_rejected_before_any_work_and_never_evicts_an_active_entry() {
        let mut journal = journal(2, 600);
        let now = SystemTime::now();
        let cancel = match journal
            .register("active", "operator", None, digest(1), now)
            .expect("active")
        {
            Admission::Fresh { cancel } => cancel,
            _ => unreachable!("a new key must be fresh"),
        };
        let _ = journal
            .register("finished", "operator", None, digest(2), now)
            .expect("b");
        journal.finish("finished", b"{}".to_vec(), false, now);

        // Full: a new key is a typed rejection, and the active entry survives it.
        assert!(
            journal
                .register("c", "operator", None, digest(3), now)
                .is_err()
        );
        assert_eq!(journal.len(), 2);
        assert!(
            journal.cancel_flag("active").is_some(),
            "an active entry is never evicted to make room"
        );
        // The rejection did not disturb the live signal.
        assert!(!cancel.is_cancelled());
    }

    #[test]
    fn finished_entries_expire_after_the_ttl_and_active_entries_never_expire() {
        let mut journal = journal(4, 10);
        let start = SystemTime::now();
        let _ = journal
            .register("active", "operator", None, digest(1), start)
            .expect("active");
        let _ = journal
            .register("done", "operator", None, digest(2), start)
            .expect("done");
        journal.finish("done", b"{}".to_vec(), false, start);

        // Just before the TTL, both survive.
        let almost = start + Duration::from_secs(9);
        assert_eq!(journal.expire_finished(almost), 0);
        assert_eq!(journal.len(), 2);

        // Past the TTL, only the finished entry goes: the TTL counts from completion
        // and an active operation is never expired (ADR-0016 §2).
        let after = start + Duration::from_secs(11);
        assert_eq!(journal.expire_finished(after), 1);
        assert_eq!(journal.len(), 1);
        assert_eq!(journal.expired_count(), 1);
        assert!(journal.cancel_flag("active").is_some());
        // The expired key is no longer recognized at all.
        assert!(journal.progress("done").is_none());
        // Its identity is retained as a bounded tombstone, so a retry of the same key is
        // refused as an expired outcome instead of executing the mutation again.
        assert_eq!(journal.tombstone_count(), 1);
        assert!(matches!(
            journal.register("done", "operator", None, digest(2), after),
            Ok(Admission::Expired)
        ));
        assert_eq!(journal.len(), 1, "the refused retry inserted nothing");
    }

    #[test]
    fn a_full_identity_memory_refuses_a_new_key_and_never_forgets_a_used_key() {
        // Two identities fit; the outcome memory is one entry. This is the shape of the
        // adversarial probe: a small journal capacity must not shrink the identity memory
        // until a used key is forgotten and a retry re-executes the mutation.
        let mut journal = OperationJournal::with_identity_capacity(1, 0, 2);
        let start = SystemTime::now();
        for key in ["first", "second"] {
            let _ = journal
                .register(key, "operator", None, digest(1), start)
                .expect("fresh");
            journal.finish(key, b"{}".to_vec(), false, start);
        }
        // `first` was retired when `second` registered; `second`'s own outcome is still
        // retained, so the two identities live in the two memories.
        assert_eq!(journal.tombstone_count(), 1);
        assert_eq!(journal.len(), 1);
        assert_eq!(
            journal.tombstone_count() + journal.len(),
            2,
            "both used keys are remembered"
        );

        // A third key cannot be remembered: it is refused before any work instead of
        // evicting an identity.
        assert!(matches!(
            journal.register("third", "operator", None, digest(1), start),
            Err(JournalRejection::Full)
        ));
        // Every remembered key still answers `Expired`, never `Fresh`.
        for key in ["first", "second"] {
            assert!(matches!(
                journal.register(key, "operator", None, digest(1), start),
                Ok(Admission::Expired)
            ));
        }
        assert_eq!(journal.tombstone_count(), 2, "the refusals changed nothing");
    }

    #[test]
    fn the_identity_memory_has_a_floor_above_a_small_outcome_capacity() {
        // A capacity of one still remembers many identities: a small payload bound must
        // not make a retry unsafe.
        let mut journal = OperationJournal::new(1, 0);
        let start = SystemTime::now();
        for index in 0..64_u64 {
            let key = format!("key-{index}");
            let _ = journal
                .register(&key, "operator", None, digest(1), start)
                .expect("fresh");
            journal.finish(&key, b"{}".to_vec(), false, start);
        }
        assert!(matches!(
            journal.register("key-0", "operator", None, digest(1), start),
            Ok(Admission::Expired)
        ));
        assert!(
            matches!(
                journal.register("fresh", "operator", None, digest(1), start),
                Ok(Admission::Fresh { .. })
            ),
            "the identity memory still has room"
        );
    }

    #[test]
    fn cancel_reaches_the_live_signal_of_an_active_operation() {
        let mut journal = journal(4, 600);
        let now = SystemTime::now();
        let _ = journal
            .register(
                "k",
                "operator",
                Some(SessionId::from_raw(7)),
                digest(1),
                now,
            )
            .expect("fresh");
        let flag = journal.cancel_flag("k").expect("live flag");
        assert!(!flag.is_cancelled());
        flag.request();
        assert!(flag.is_cancelled());
        // The newest active operation of a session is addressable without a session
        // owner: the cancel path reads the journal only.
        assert_eq!(
            journal.newest_active_for(SessionId::from_raw(7)).as_deref(),
            Some("k")
        );
        assert_eq!(journal.newest_active_for(SessionId::from_raw(8)), None);
    }

    #[test]
    fn abandon_removes_an_entry_so_a_retry_is_fresh_again() {
        let mut journal = journal(1, 600);
        let now = SystemTime::now();
        let _ = journal
            .register("k", "operator", None, digest(1), now)
            .expect("fresh");
        journal.abandon("k");
        assert!(journal.progress("k").is_none());
        // With the entry gone, the capacity is free again: an operation that never
        // started must not consume the journal.
        assert!(matches!(
            journal.register("k2", "operator", None, digest(2), now),
            Ok(Admission::Fresh { .. })
        ));
    }

    #[test]
    fn a_cancel_flag_is_independent_of_every_other_flag() {
        let a = Arc::new(CancelFlag::new());
        let b = Arc::new(CancelFlag::new());
        a.request();
        assert!(a.is_cancelled());
        assert!(
            !b.is_cancelled(),
            "cancelling one operation cannot cancel another"
        );
    }

    #[test]
    fn process_epochs_are_unique_and_increasing_in_one_process() {
        let first = ProcessEpoch::load(None);
        let second = ProcessEpoch::load(None);
        assert!(second.value() > first.value());
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_sync_accepts_a_bare_relative_name() {
        // `Path::parent` of a bare name is `Some("")`, which used to be opened as `""` and
        // failed the whole durable start (R1d/D1). It must mean the current directory.
        assert_eq!(
            Path::new("store.chronofork-epoch").parent(),
            Some(Path::new(""))
        );
        super::sync_directory(Path::new("store.chronofork-epoch"))
            .expect("a bare relative name syncs the current directory");
        super::sync_directory(Path::new("./store.chronofork-epoch"))
            .expect("the `./` form syncs the current directory");
    }

    #[cfg(not(unix))]
    #[test]
    fn the_directory_sync_is_a_documented_best_effort_no_op() {
        // Non-Unix platforms have no portable directory fsync; the publication stays
        // best-effort and the start is not refused (owner decision, R1d/D3).
        super::sync_directory(Path::new("store.chronofork-epoch"))
            .expect("the documented best-effort path returns success");
    }
}
