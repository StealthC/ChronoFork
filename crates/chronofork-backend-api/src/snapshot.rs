//! Versioned backend snapshots.
//!
//! A [`BackendSnapshot`] is the backend-owned half of a ChronoFork checkpoint.
//! The session layer adds stable identifiers, lineage and an outer serialization
//! format without inspecting the payload.

use chronofork_core::{StateHash, Tick};

use crate::descriptor::BackendId;
use crate::error::BackendError;

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
}

/// Validates the compatibility fields shared by every backend snapshot envelope.
///
/// The fields are checked in the order backends have always reported them:
/// backend id, backend version, envelope format version and configuration hash.
/// The first mismatch is returned, so a rejected restore leaves the backend
/// untouched and reports the same field a hand-written check would have named.
///
/// # Errors
///
/// Returns [`BackendError::IncompatibleSnapshot`] naming the first mismatched
/// field, with the expected and found values rendered for diagnostics.
pub fn validate_envelope(
    snapshot: &BackendSnapshot,
    backend_id: &BackendId,
    backend_version: &str,
    format_version: u16,
    config_hash: StateHash,
) -> Result<(), BackendError> {
    if snapshot.backend_id() != backend_id {
        return Err(BackendError::IncompatibleSnapshot {
            field: "backend_id",
            expected: backend_id.to_string(),
            found: snapshot.backend_id().to_string(),
        });
    }
    if snapshot.backend_version() != backend_version {
        return Err(BackendError::IncompatibleSnapshot {
            field: "backend_version",
            expected: backend_version.to_owned(),
            found: snapshot.backend_version().to_owned(),
        });
    }
    if snapshot.format_version() != format_version {
        return Err(BackendError::IncompatibleSnapshot {
            field: "backend_format_version",
            expected: format_version.to_string(),
            found: snapshot.format_version().to_string(),
        });
    }
    if snapshot.config_hash() != config_hash {
        return Err(BackendError::IncompatibleSnapshot {
            field: "config_hash",
            expected: config_hash.to_string(),
            found: snapshot.config_hash().to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chronofork_core::{StateHash, Tick};

    use super::{BackendSnapshot, validate_envelope};
    use crate::descriptor::BackendId;
    use crate::error::BackendError;

    const BACKEND_ID: &str = "chronofork.test.synthetic";
    const BACKEND_VERSION: &str = "0.1.0";
    const FORMAT_VERSION: u16 = 1;
    const CONFIG_HASH: StateHash = StateHash::from_bytes([0x11; 32]);

    fn snapshot_with(
        backend_id: &str,
        backend_version: &str,
        format_version: u16,
        config_hash: StateHash,
    ) -> BackendSnapshot {
        BackendSnapshot::new(
            BackendId::new(backend_id),
            backend_version,
            format_version,
            Tick::from_raw(42),
            config_hash,
            StateHash::from_bytes([0x22; 32]),
            vec![1, 2, 3],
        )
    }

    fn validate(snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        validate_envelope(
            snapshot,
            &BackendId::new(BACKEND_ID),
            BACKEND_VERSION,
            FORMAT_VERSION,
            CONFIG_HASH,
        )
    }

    #[test]
    fn matching_envelope_is_accepted() {
        let snapshot = snapshot_with(BACKEND_ID, BACKEND_VERSION, FORMAT_VERSION, CONFIG_HASH);
        assert_eq!(validate(&snapshot), Ok(()));
    }

    #[test]
    fn each_compatibility_field_is_rejected_by_name() {
        let wrong_id = snapshot_with(
            "chronofork.other",
            BACKEND_VERSION,
            FORMAT_VERSION,
            CONFIG_HASH,
        );
        assert_eq!(
            validate(&wrong_id),
            Err(BackendError::IncompatibleSnapshot {
                field: "backend_id",
                expected: BACKEND_ID.to_owned(),
                found: "chronofork.other".to_owned(),
            })
        );

        let wrong_version = snapshot_with(BACKEND_ID, "9.9.9", FORMAT_VERSION, CONFIG_HASH);
        assert_eq!(
            validate(&wrong_version),
            Err(BackendError::IncompatibleSnapshot {
                field: "backend_version",
                expected: BACKEND_VERSION.to_owned(),
                found: "9.9.9".to_owned(),
            })
        );

        let wrong_format = snapshot_with(BACKEND_ID, BACKEND_VERSION, 2, CONFIG_HASH);
        assert_eq!(
            validate(&wrong_format),
            Err(BackendError::IncompatibleSnapshot {
                field: "backend_format_version",
                expected: "1".to_owned(),
                found: "2".to_owned(),
            })
        );

        let wrong_config = snapshot_with(
            BACKEND_ID,
            BACKEND_VERSION,
            FORMAT_VERSION,
            StateHash::from_bytes([0x33; 32]),
        );
        assert_eq!(
            validate(&wrong_config),
            Err(BackendError::IncompatibleSnapshot {
                field: "config_hash",
                expected: CONFIG_HASH.to_string(),
                found: StateHash::from_bytes([0x33; 32]).to_string(),
            })
        );
    }

    #[test]
    fn backend_id_mismatch_wins_over_later_fields() {
        let all_wrong = snapshot_with(
            "chronofork.other",
            "9.9.9",
            2,
            StateHash::from_bytes([0x33; 32]),
        );
        assert_eq!(
            validate(&all_wrong),
            Err(BackendError::IncompatibleSnapshot {
                field: "backend_id",
                expected: BACKEND_ID.to_owned(),
                found: "chronofork.other".to_owned(),
            })
        );
    }
}
