//! Durable, versioned snapshot storage.
//!
//! This module implements the storage half of
//! [ADR-0003](../../../../docs/adr/0003-durable-session-storage.md): a caller-supplied
//! root with a versioned marker, one directory per durable session and snapshot,
//! a canonical `checkpoint.cfsnap` file containing exactly
//! [`Snapshot::to_bytes`](crate::Snapshot::to_bytes) output, a separate metadata
//! record and an atomic staging-directory publish.
//!
//! Storage never serializes backend state itself and never decides backend
//! compatibility: it calls [`Snapshot::from_bytes`](crate::Snapshot::from_bytes) and
//! leaves [`validate_envelope`](chronofork_backend_api::validate_envelope) to the
//! session restore path. Cross-process ownership is explicit: a per-session
//! [`writer.lock`](crate::SnapshotRepository::acquire_writer_lock) serializes
//! mutations of one session, the root `state/allocation.lock` is held only while a
//! session key is minted, and readers take no lock.
//!
//! Host timestamps are advisory metadata only. They are never part of a checkpoint
//! envelope, a state hash, list ordering or a replay result.

mod lock;
mod metadata;
mod repository;
mod root;

pub use lock::{LockIdentity, MAX_LOCK_BYTES, WriterLock};
pub use repository::SnapshotRepository;

use std::fmt;
use std::path::PathBuf;

use chronofork_core::{StateHash, Tick};

use crate::error::SessionError;

/// Storage layout version written into the root marker and directory tree.
pub const STORAGE_LAYOUT_VERSION: u16 = 1;

/// Version of the storage-level metadata record format.
pub const STORAGE_METADATA_FORMAT_VERSION: u16 = 1;

/// Maximum number of UTF-8 bytes in a session or snapshot name.
pub const MAX_NAME_BYTES: usize = 256;

/// Per-operation resource limits.
///
/// The defaults match ADR-0003 §11. A caller may lower them through
/// [`SnapshotRepository::open_with_limits`]; raising a default is a documented
/// contract change, not an ordinary configuration tweak.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageLimits {
    /// Maximum size of a `checkpoint.cfsnap` file.
    pub max_checkpoint_bytes: u64,
    /// Maximum size of a `session.cfmeta` or `snapshot.cfmeta` record.
    pub max_metadata_bytes: u64,
    /// Maximum UTF-8 length of a session or snapshot name.
    pub max_name_bytes: usize,
    /// Maximum number of entries one list scan may observe.
    pub max_list_entries: usize,
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            max_checkpoint_bytes: 64 * 1024 * 1024,
            max_metadata_bytes: 64 * 1024,
            max_name_bytes: MAX_NAME_BYTES,
            max_list_entries: 100_000,
        }
    }
}

impl StorageLimits {
    /// Validates a name against the configured byte bound and the shared rule.
    pub(crate) fn validate_name(&self, name: &str) -> Result<(), SessionError> {
        if name.is_empty() {
            return Err(SessionError::InvalidName(
                "name must not be empty".to_owned(),
            ));
        }
        if name.len() > self.max_name_bytes {
            return Err(SessionError::InvalidName(format!(
                "name is {} bytes, limit is {}",
                name.len(),
                self.max_name_bytes
            )));
        }
        if name.chars().any(char::is_control) {
            return Err(SessionError::InvalidName(
                "name must not contain control characters".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_optional_name(&self, name: Option<&str>) -> Result<(), SessionError> {
        match name {
            Some(name) => self.validate_name(name),
            None => Ok(()),
        }
    }
}

/// Durable lineage recorded for a session or snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DurableLineage {
    /// Durable parent session key, when this object was forked.
    pub parent_session_key: Option<u64>,
    /// Durable origin snapshot key, when this object was forked or imported.
    pub origin_snapshot_key: Option<u64>,
}

impl DurableLineage {
    /// Creates an empty lineage for a root session.
    #[must_use]
    pub const fn root() -> Self {
        Self {
            parent_session_key: None,
            origin_snapshot_key: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), SessionError> {
        if self.parent_session_key == Some(0) {
            return Err(SessionError::IdentityMismatch(
                "parent session key must be at least one".to_owned(),
            ));
        }
        if self.origin_snapshot_key == Some(0) {
            return Err(SessionError::IdentityMismatch(
                "origin snapshot key must be at least one".to_owned(),
            ));
        }
        Ok(())
    }
}

/// One durable session record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEntry {
    /// Durable session key; matches the `session-<key>` directory.
    pub session_key: u64,
    /// Optional non-canonical label.
    pub name: Option<String>,
    /// Durable parent session key, when the session was forked.
    pub parent_session_key: Option<u64>,
    /// Durable origin snapshot key, when the session was forked or imported.
    pub origin_snapshot_key: Option<u64>,
    /// Next snapshot key reserved by the session record.
    pub next_snapshot_key: u64,
    /// Next logical creation sequence reserved by the session record.
    pub next_creation_sequence: u64,
    /// Advisory host creation timestamp; never canonical.
    pub created_at_unix_ms: Option<u64>,
}

/// One validated durable snapshot record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotEntry {
    /// Durable session key.
    pub session_key: u64,
    /// Durable snapshot key; matches the `snapshot-<key>` directory and envelope ID.
    pub snapshot_key: u64,
    /// Optional non-canonical label.
    pub name: Option<String>,
    /// Logical per-session ordering value assigned under the session writer.
    pub creation_sequence: u64,
    /// Tick recorded by the canonical envelope.
    pub tick: Tick,
    /// State digest recorded by the canonical envelope.
    pub state_hash: StateHash,
    /// Durable parent session key, inherited from the session record.
    pub parent_session_key: Option<u64>,
    /// Durable origin snapshot key, inherited from the session record.
    pub origin_snapshot_key: Option<u64>,
    /// Exact `checkpoint.cfsnap` byte length.
    pub checkpoint_length: u64,
    /// SHA-256 of the exact `checkpoint.cfsnap` bytes.
    pub checkpoint_sha256: StateHash,
    /// Advisory host creation timestamp; never canonical.
    pub created_at_unix_ms: Option<u64>,
}

/// A validated checkpoint together with its storage record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedSnapshot {
    /// Canonical checkpoint parsed by [`Snapshot::from_bytes`](crate::Snapshot::from_bytes).
    pub snapshot: crate::Snapshot,
    /// Validated storage metadata for the checkpoint.
    pub entry: SnapshotEntry,
}

/// Outcome of reconciling a publish whose post-rename parent sync failed.
///
/// A publish is durable and successful only after both the visibility-changing
/// rename and the parent-directory sync succeed and the object re-validates. These
/// outcomes are computed on demand; they are never persisted as success markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationOutcome {
    /// The retried parent sync succeeded and the visible object re-validated.
    VisibleConfirmed,
    /// The object is visible and re-validated, but the parent sync still failed, so
    /// durability is unconfirmed and success must not be reported.
    VisibleUnconfirmed,
    /// The object disappeared before reconciliation; the loss is reported and
    /// nothing is resurrected or repaired.
    Lost,
}

/// A deterministic list result plus typed anomalies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageListing<T> {
    /// Entries that passed the full validation order, in contract order.
    pub entries: Vec<T>,
    /// Entries that failed validation, sorted by path; never repaired.
    pub anomalies: Vec<StorageAnomaly>,
}
impl<T> StorageListing<T> {
    pub(crate) fn new(entries: Vec<T>, mut anomalies: Vec<StorageAnomaly>) -> Self {
        anomalies.sort_by(|left, right| left.path.cmp(&right.path));
        Self { entries, anomalies }
    }
}

/// A typed diagnostic for a storage entry that is not committed data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageAnomaly {
    /// Path of the anomalous entry.
    pub path: PathBuf,
    /// Category of the anomaly.
    pub kind: StorageAnomalyKind,
}

impl fmt::Display for StorageAnomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.kind)
    }
}

/// Category of a storage anomaly reported by a list scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageAnomalyKind {
    /// A `.staging-*` directory left by an incomplete publish.
    IncompletePublish,
    /// A `.trash-*` directory left by an incomplete delete.
    PendingDeletion,
    /// A directory name outside the fixed layout.
    UnexpectedEntry,
    /// A private lock-protocol record left by a crash or failed cleanup.
    ///
    /// It is never treated as a lock and is never reclaimed automatically.
    LockProtocolRecord,
    /// A future layout-version directory that is never read as v1 data.
    FutureLayoutVersion(u16),
    /// A keyed directory name that is not canonical numeric text.
    InvalidKey,
    /// A published object that fails validation.
    CorruptObject(SessionError),
}

impl fmt::Display for StorageAnomalyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompletePublish => f.write_str("incomplete publish; never loaded"),
            Self::PendingDeletion => f.write_str("pending deletion; never resurrected"),
            Self::UnexpectedEntry => f.write_str("unexpected entry outside the fixed layout"),
            Self::LockProtocolRecord => {
                f.write_str("leftover private lock-protocol record; never treated as a lock")
            }
            Self::FutureLayoutVersion(version) => write!(
                f,
                "unsupported future layout directory v{version}; never read or migrated"
            ),
            Self::InvalidKey => f.write_str("non-canonical durable key component"),
            Self::CorruptObject(error) => write!(f, "invalid storage object: {error}"),
        }
    }
}
