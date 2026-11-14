//! Execution lineage for forked sessions.

use chronofork_core::{SessionId, SnapshotId};

/// Provenance of one session within a fork tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lineage {
    parent_session: Option<SessionId>,
    origin_snapshot: Option<SnapshotId>,
    depth: u32,
}

impl Lineage {
    /// Creates the root lineage of a newly created session.
    #[must_use]
    pub const fn root() -> Self {
        Self {
            parent_session: None,
            origin_snapshot: None,
            depth: 0,
        }
    }

    /// Creates a lineage for a session imported from serialized snapshot bytes.
    #[must_use]
    pub const fn imported(snapshot: SnapshotId) -> Self {
        Self {
            parent_session: None,
            origin_snapshot: Some(snapshot),
            depth: 0,
        }
    }

    /// Creates a child lineage for a fork.
    #[must_use]
    pub const fn child(parent: SessionId, snapshot: SnapshotId, parent_depth: u32) -> Self {
        Self {
            parent_session: Some(parent),
            origin_snapshot: Some(snapshot),
            depth: parent_depth.saturating_add(1),
        }
    }

    /// Returns the parent session, if this session was forked.
    #[must_use]
    pub const fn parent_session(&self) -> Option<SessionId> {
        self.parent_session
    }

    /// Returns the checkpoint this session was forked or imported from.
    #[must_use]
    pub const fn origin_snapshot(&self) -> Option<SnapshotId> {
        self.origin_snapshot
    }

    /// Returns the fork depth (zero for roots).
    #[must_use]
    pub const fn depth(&self) -> u32 {
        self.depth
    }
}
