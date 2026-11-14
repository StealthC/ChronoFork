//! Session-layer errors.

use std::error::Error;
use std::fmt;

use chronofork_backend_api::BackendError;
use chronofork_core::{SessionId, SnapshotId};

use crate::session::SessionState;

/// A failure in session lifecycle, checkpointing or snapshot handling.
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
    /// The session or snapshot identifier space is exhausted.
    IdSpaceExhausted,
    /// A serialized snapshot is malformed or fails its checksum.
    InvalidSnapshot(String),
    /// A serialized snapshot uses an unsupported format version.
    IncompatibleSnapshot(String),
    /// The backend rejected an operation.
    Backend(BackendError),
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
            Self::IdSpaceExhausted => f.write_str("session or snapshot identifier space exhausted"),
            Self::InvalidSnapshot(message) => write!(f, "invalid snapshot: {message}"),
            Self::IncompatibleSnapshot(message) => {
                write!(f, "incompatible snapshot: {message}")
            }
            Self::Backend(error) => write!(f, "backend error: {error}"),
        }
    }
}

impl Error for SessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BackendError> for SessionError {
    fn from(error: BackendError) -> Self {
        Self::Backend(error)
    }
}
