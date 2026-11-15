//! Session lifecycle, checkpoints and forkable execution histories.
//!
//! A [`Session`] owns exactly one backend instance. Forking creates a fresh
//! backend, restores a checkpoint into it, and registers a new session with
//! explicit lineage. There is no implicit sharing of mutable hardware state.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod error;
pub mod lineage;
pub mod manager;
pub mod session;
pub mod snapshot;
pub mod storage;

pub use error::SessionError;
pub use lineage::Lineage;
pub use manager::{SessionComparison, SessionManager, SessionSummary};
pub use session::{Session, SessionInspection, SessionState};
pub use snapshot::{SNAPSHOT_FORMAT_VERSION, Snapshot};
pub use storage::{
    DurableLineage, LoadedSnapshot, LockIdentity, MAX_LOCK_BYTES, MAX_NAME_BYTES,
    ReconciliationOutcome, STORAGE_LAYOUT_VERSION, STORAGE_METADATA_FORMAT_VERSION, SessionEntry,
    SnapshotEntry, SnapshotRepository, StorageAnomaly, StorageAnomalyKind, StorageLimits,
    StorageListing, WriterLock,
};
