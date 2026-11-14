//! Versioned backend snapshots.
//!
//! A [`BackendSnapshot`] is the backend-owned half of a ChronoFork checkpoint.
//! The session layer adds stable identifiers, lineage and an outer serialization
//! format without inspecting the payload.

use chronofork_core::{StateHash, Tick};

use crate::descriptor::BackendId;

/// Version of the generic backend snapshot envelope.
pub const BACKEND_SNAPSHOT_FORMAT_VERSION: u16 = 1;

/// Opaque state produced by one backend instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendSnapshot {
    backend_id: BackendId,
    backend_version: String,
    format_version: u16,
    tick: Tick,
    config_hash: StateHash,
    state_hash: StateHash,
    payload: Vec<u8>,
}

impl BackendSnapshot {
    /// Creates a snapshot envelope.
    #[must_use]
    pub fn new(
        backend_id: BackendId,
        backend_version: impl Into<String>,
        format_version: u16,
        tick: Tick,
        config_hash: StateHash,
        state_hash: StateHash,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            backend_id,
            backend_version: backend_version.into(),
            format_version,
            tick,
            config_hash,
            state_hash,
            payload,
        }
    }

    /// Returns the backend family that produced this snapshot.
    #[must_use]
    pub fn backend_id(&self) -> &BackendId {
        &self.backend_id
    }

    /// Returns the producing backend version.
    #[must_use]
    pub fn backend_version(&self) -> &str {
        &self.backend_version
    }

    /// Returns the backend payload format version.
    #[must_use]
    pub const fn format_version(&self) -> u16 {
        self.format_version
    }

    /// Returns the emulated tick at snapshot creation.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }

    /// Returns the digest of immutable machine configuration (for example ROM
    /// identity) required to interpret the snapshot.
    #[must_use]
    pub const fn config_hash(&self) -> StateHash {
        self.config_hash
    }

    /// Returns the digest of mutable machine state.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.state_hash
    }

    /// Returns the backend-owned payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Consumes the envelope and returns its payload.
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}
