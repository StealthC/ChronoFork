//! Stable, typed identifiers.
//!
//! Identifiers are deliberately opaque. They are allocated by the session
//! manager, not by backends, and they must not be interpreted as array indexes.

use std::fmt;

/// Identifies one independent execution session owned by a session manager.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionId(u64);

impl SessionId {
    /// Creates an identifier from a raw value.
    ///
    /// Intended for persistence adapters and tests. Normal callers receive
    /// identifiers from [`crate::SessionId`]-producing services.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw value for persistence or diagnostics.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session-{}", self.0)
    }
}

/// Identifies one checkpoint stored by a session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SnapshotId(u64);

impl SnapshotId {
    /// Creates an identifier from a raw value.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw value for persistence or diagnostics.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "snapshot-{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionId, SnapshotId};

    #[test]
    fn ids_round_trip_through_raw_values() {
        assert_eq!(SessionId::from_raw(7).raw(), 7);
        assert_eq!(SnapshotId::from_raw(9).raw(), 9);
    }

    #[test]
    fn ids_have_readable_display_forms() {
        assert_eq!(SessionId::from_raw(3).to_string(), "session-3");
        assert_eq!(SnapshotId::from_raw(4).to_string(), "snapshot-4");
    }
}
