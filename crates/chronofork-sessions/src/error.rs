//! Session-layer errors.

use std::error::Error;
use std::fmt;
use std::io::ErrorKind;
use std::path::PathBuf;

use chronofork_backend_api::BackendError;
use chronofork_core::{SessionId, SnapshotId};
use chronofork_trace::TraceError;

use crate::session::{SessionQuota, SessionState};
use crate::storage::LockIdentity;

/// A failure in session lifecycle, checkpointing, snapshot handling or durable
/// storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    /// The requested session is not registered with this manager.
    UnknownSession(SessionId),
    /// The requested snapshot is not stored by the given session.
    UnknownSnapshot {
        /// Session that was asked for the snapshot.
        session: SessionId,
        /// Snapshot identifier.
        snapshot: SnapshotId,
    },
    /// The session was closed and cannot be mutated further.
    Closed(SessionId),
    /// The session is not in a state that permits execution.
    NotRunning {
        /// Session identifier.
        session: SessionId,
        /// Current lifecycle state.
        state: SessionState,
    },
    /// The backend halted and cannot execute another step.
    Halted(SessionId),
    /// A previous holder panicked while owning the session's execution lock.
    ///
    /// The session object itself is intact; the owner is poisoned by the
    /// [`std::sync::Mutex`] rule, so the service reports a typed internal failure
    /// instead of panicking the calling thread.
    Poisoned(SessionId),
    /// The session or snapshot identifier space is exhausted.
    IdSpaceExhausted,
    /// A per-session quota would be exceeded by the requested allocation.
    ///
    /// The rejection happens before the allocation (ADR-0016 §3): the session's
    /// state and its stored checkpoints are unchanged, and the caller sees the
    /// quota, its limit and the value the operation would have reached.
    LimitExceeded {
        /// Which per-session quota was exceeded.
        quota: SessionQuota,
        /// Configured limit of that quota.
        limit: usize,
        /// Value the operation would have reached.
        current: usize,
    },
    /// A byte quota cannot be preflighted because the backend declares no payload bound.
    ///
    /// The capture was **refused before allocating**: the state changed since the last
    /// observation, so the previous payload size is not a bound, and the backend's
    /// `snapshot_payload_bound` returned `None`. It is reported under the byte quota, but it
    /// is a *missing declaration* rather than an exceeded limit (see the Mega Drive W1.8
    /// follow-up).
    UnboundedSnapshot {
        /// Configured limit of the byte quota that could not be preflighted.
        limit: usize,
        /// Bytes already retained by this session's checkpoints.
        retained: usize,
    },
    /// A serialized snapshot is malformed or fails its checksum.
    InvalidSnapshot(String),
    /// A serialized snapshot uses an unsupported format version.
    IncompatibleSnapshot(String),
    /// The backend rejected an operation.
    Backend(BackendError),
    /// The host trace capture or trace operation failed.
    Trace(TraceError),
    /// The storage root has no `chronofork.store` marker.
    NotInitialized,
    /// The storage marker is present but is not a ChronoFork store marker.
    NotAStorageRoot(String),
    /// Initialization found a non-empty directory that is not a store root.
    RootNotEmpty(PathBuf),
    /// The store layout version is not supported by this build.
    UnsupportedLayoutVersion {
        /// Version found in the marker.
        found: u16,
        /// Version supported by this build.
        supported: u16,
    },
    /// A metadata record is malformed or fails its checksum.
    CorruptMetadata(String),
    /// A metadata record uses an unsupported record format version.
    UnsupportedMetadataVersion {
        /// Version found in the record.
        found: u16,
        /// Version supported by this build.
        supported: u16,
    },
    /// A metadata field disagrees with the checkpoint or the file name.
    MetadataMismatch(String),
    /// A durable key or diagnostic identity disagrees with its directory.
    IdentityMismatch(String),
    /// A committed object is missing a required record.
    MissingRecord(PathBuf),
    /// The requested durable session key is not stored.
    UnknownStorageSession(u64),
    /// The requested durable snapshot key is not stored.
    UnknownStorageSnapshot {
        /// Durable session key.
        session: u64,
        /// Durable snapshot key.
        snapshot: u64,
    },
    /// A durable session key already exists and is never overwritten.
    SessionConflict(u64),
    /// A durable snapshot key already exists and is never overwritten.
    SnapshotExists {
        /// Durable session key.
        session: u64,
        /// Durable snapshot key.
        snapshot: u64,
    },
    /// A session still contains snapshots and cannot be deleted yet.
    SessionNotEmpty(u64),
    /// A file exceeds the configured size bound.
    FileTooLarge {
        /// File that was inspected.
        path: PathBuf,
        /// Configured maximum size in bytes.
        limit: u64,
        /// Observed size in bytes.
        found: u64,
    },
    /// A list operation would exceed the configured entry bound.
    ListLimitExceeded {
        /// Configured maximum number of entries.
        limit: usize,
    },
    /// A snapshot or session name is empty, over-long or contains control characters.
    InvalidName(String),
    /// A managed path component is not canonical or would escape the root.
    PathEscape(PathBuf),
    /// A managed path is a symbolic link and is never followed.
    SymlinkRejected(PathBuf),
    /// A managed path has the wrong file type.
    WrongFileType(PathBuf),
    /// A visibility-changing rename completed but its parent directory sync failed.
    DurabilityUnconfirmed {
        /// Operation whose durability is unconfirmed.
        operation: &'static str,
        /// Directory that could not be synced.
        path: PathBuf,
    },
    /// The durable session-key counter is missing, malformed or unreadable.
    CorruptAllocationCounter(String),
    /// A writer or allocation lock is held by another owner.
    ///
    /// Acquisition never waits and never steals: an empty, partial or otherwise
    /// unreadable lock is still contention and names no readable owner.
    WriterContended {
        /// Durable session whose writer lock is held; `None` for the root
        /// key-allocation lock.
        session: Option<u64>,
        /// Observed owner and generation when the lock record was readable.
        owner: Option<LockIdentity>,
    },
    /// Release or explicit recovery was attempted with an identity that no longer
    /// owns the lock. Nothing was deleted.
    NotLockOwner {
        /// Durable session whose lock was checked; `None` for the root
        /// key-allocation lock.
        session: Option<u64>,
        /// Owner/generation pair the caller expected to remove.
        expected: LockIdentity,
    },
    /// A lock record exists but is not a readable, complete record, and explicit
    /// recovery was requested. Storage removes nothing.
    CorruptLock {
        /// Path of the unreadable canonical lock record.
        path: PathBuf,
        /// Observed problem.
        message: String,
    },
    /// The filesystem cannot publish a lock record atomically because it does not
    /// support hard links. Acquisition fails closed and is never retried through a
    /// non-atomic `create_new`-then-write fallback.
    LockPrimitiveUnsupported {
        /// Directory where atomic publication was attempted.
        path: PathBuf,
    },
    /// A filesystem operation failed.
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// Path the operation targeted.
        path: PathBuf,
        /// Portable error category.
        kind: ErrorKind,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession(session) => write!(f, "unknown session: {session}"),
            Self::UnknownSnapshot { session, snapshot } => {
                write!(f, "unknown snapshot {snapshot} for {session}")
            }
            Self::Closed(session) => write!(f, "session is closed: {session}"),
            Self::NotRunning { session, state } => {
                write!(f, "session {session} is not running (state: {state})")
            }
            Self::Halted(session) => write!(f, "session {session} halted"),
            Self::Poisoned(session) => {
                write!(
                    f,
                    "session {session} owner was poisoned by a panicking holder"
                )
            }
            Self::IdSpaceExhausted => f.write_str("session or snapshot identifier space exhausted"),
            Self::LimitExceeded {
                quota,
                limit,
                current,
            } => write!(
                f,
                "{} would reach {current}, above the limit of {limit}",
                quota.as_str()
            ),
            Self::UnboundedSnapshot { limit, retained } => write!(
                f,
                "the backend declares no snapshot payload bound, so {} (limit {limit}, \
                 {retained} bytes retained) cannot be preflighted; the capture was refused \
                 before allocating",
                SessionQuota::Bytes.as_str()
            ),
            Self::InvalidSnapshot(message) => write!(f, "invalid snapshot: {message}"),
            Self::IncompatibleSnapshot(message) => {
                write!(f, "incompatible snapshot: {message}")
            }
            Self::Backend(error) => write!(f, "backend error: {error}"),
            Self::Trace(error) => write!(f, "trace error: {error}"),
            Self::NotInitialized => f.write_str("storage root is not initialized"),
            Self::NotAStorageRoot(message) => write!(f, "not a ChronoFork storage root: {message}"),
            Self::RootNotEmpty(path) => {
                write!(f, "storage root is not empty: {}", path.display())
            }
            Self::UnsupportedLayoutVersion { found, supported } => write!(
                f,
                "unsupported storage layout version {found}, expected {supported}"
            ),
            Self::CorruptMetadata(message) => write!(f, "corrupt storage metadata: {message}"),
            Self::UnsupportedMetadataVersion { found, supported } => write!(
                f,
                "unsupported storage metadata version {found}, expected {supported}"
            ),
            Self::MetadataMismatch(message) => write!(f, "storage metadata mismatch: {message}"),
            Self::IdentityMismatch(message) => write!(f, "storage identity mismatch: {message}"),
            Self::MissingRecord(path) => {
                write!(f, "missing storage record: {}", path.display())
            }
            Self::UnknownStorageSession(session) => {
                write!(f, "unknown durable session key: {session}")
            }
            Self::UnknownStorageSnapshot { session, snapshot } => write!(
                f,
                "unknown durable snapshot key {snapshot} for durable session {session}"
            ),
            Self::SessionConflict(session) => {
                write!(f, "durable session key already exists: {session}")
            }
            Self::SnapshotExists { session, snapshot } => write!(
                f,
                "durable snapshot key {snapshot} already exists for durable session {session}"
            ),
            Self::SessionNotEmpty(session) => write!(
                f,
                "durable session {session} still contains snapshots; delete them explicitly first"
            ),
            Self::FileTooLarge { path, limit, found } => write!(
                f,
                "file {} is {found} bytes, limit is {limit}",
                path.display()
            ),
            Self::ListLimitExceeded { limit } => {
                write!(f, "storage listing exceeds the {limit}-entry limit")
            }
            Self::InvalidName(message) => write!(f, "invalid storage name: {message}"),
            Self::PathEscape(path) => write!(
                f,
                "managed path escapes the storage root: {}",
                path.display()
            ),
            Self::SymlinkRejected(path) => {
                write!(f, "managed path is a symbolic link: {}", path.display())
            }
            Self::WrongFileType(path) => {
                write!(
                    f,
                    "managed path has the wrong file type: {}",
                    path.display()
                )
            }
            Self::DurabilityUnconfirmed { operation, path } => write!(
                f,
                "{operation} is visible but not durably confirmed: parent sync failed for {}",
                path.display()
            ),
            Self::CorruptAllocationCounter(message) => {
                write!(f, "corrupt durable session-key counter: {message}")
            }
            Self::WriterContended { session, owner } => {
                let scope = match session {
                    Some(session) => format!("durable session {session} writer lock"),
                    None => "storage key-allocation lock".to_owned(),
                };
                match owner {
                    Some(owner) => write!(f, "{scope} is held by owner {owner}"),
                    None => write!(f, "{scope} is held by an unreadable owner"),
                }
            }
            Self::NotLockOwner { session, expected } => {
                let scope = match session {
                    Some(session) => format!("durable session {session} writer lock"),
                    None => "storage key-allocation lock".to_owned(),
                };
                write!(
                    f,
                    "{scope} is not owned by the expected owner {expected}; nothing was removed"
                )
            }
            Self::CorruptLock { path, message } => write!(
                f,
                "unreadable lock record {}: {message}; explicit recovery removes nothing",
                path.display()
            ),
            Self::LockPrimitiveUnsupported { path } => write!(
                f,
                "filesystem does not support hard links, so lock publication cannot be \
                 atomic in {}",
                path.display()
            ),
            Self::Io {
                operation,
                path,
                kind,
            } => write!(
                f,
                "storage I/O error during {operation} for {}: {kind:?}",
                path.display()
            ),
        }
    }
}

impl Error for SessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            Self::Trace(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BackendError> for SessionError {
    fn from(error: BackendError) -> Self {
        Self::Backend(error)
    }
}
