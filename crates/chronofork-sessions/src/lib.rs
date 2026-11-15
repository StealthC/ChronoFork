//! Session lifecycle, checkpoints and forkable execution histories.
//!
//! A [`Session`] owns exactly one backend instance. Forking creates a fresh
//! backend, restores a checkpoint into it, and registers a new session with
//! explicit lineage. There is no implicit sharing of mutable hardware state.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod error;
pub mod lineage;
pub mod manager;
pub mod movie;
pub mod player;
pub mod replay;
pub mod session;
pub mod snapshot;
pub mod storage;

pub use error::SessionError;
pub use lineage::Lineage;
pub use manager::{SessionComparison, SessionManager, SessionSummary};
pub use movie::{
    ANCHOR_RECORD_OVERHEAD, MAX_ANCHOR_BYTES, MAX_ANCHOR_SNAPSHOT_BYTES, MAX_ANCHORS,
    MAX_EMBEDDED_SNAPSHOT_BYTES, MAX_MOVIE_BYTES, MAX_STEPS, MAX_V2_STEPS, MOVIE_FORMAT_VERSION,
    MOVIE_INPUT_SCHEMA_VERSION, Movie, MovieAnchor, MovieError, MovieMeta, MovieStep, ReplayReport,
    V2_FIXED_OVERHEAD, V2_MOVIE_FORMAT_VERSION, V2_MOVIE_INPUT_SCHEMA_VERSION, V2_STEP_RECORD_LEN,
};
pub use player::{
    CacheOptions, CacheStats, DEFAULT_CACHE_BYTE_LIMIT, DEFAULT_CACHE_CAPACITY,
    DEFAULT_CACHE_INTERVAL_STEPS, MAX_CACHE_BYTE_LIMIT, MAX_CACHE_CAPACITY, Player, PlayerError,
    PlayerState, STATE_CACHE_ENTRY_OVERHEAD, SeekProgress, SeekReport, StartKind,
};
pub use replay::{ComparisonReport, ComparisonVerdict, ReplayOutcome, ReplayRange, compare_movies};
pub use session::{Session, SessionInspection, SessionState};
pub use snapshot::{SNAPSHOT_FORMAT_VERSION, Snapshot};
pub use storage::{
    DurableLineage, LoadedSnapshot, LockIdentity, MAX_LOCK_BYTES, MAX_NAME_BYTES,
    ReconciliationOutcome, STORAGE_LAYOUT_VERSION, STORAGE_METADATA_FORMAT_VERSION, SessionEntry,
    SnapshotEntry, SnapshotRepository, StorageAnomaly, StorageAnomalyKind, StorageLimits,
    StorageListing, WriterLock,
};
