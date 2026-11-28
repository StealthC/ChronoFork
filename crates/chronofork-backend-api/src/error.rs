//! Backend-facing errors.
//!
//! Backend errors are converted into session errors at the boundary. They never
//! carry host paths or transport details, keeping the contract reusable by CLI,
//! HTTP, MCP and embedded callers.

use std::error::Error;
use std::fmt;

use chronofork_core::CoreError;

/// A failure reported by an emulation backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    /// The supplied program, ROM or machine configuration is invalid.
    InvalidConfiguration(String),
    /// A snapshot payload is malformed or fails its internal checks.
    InvalidSnapshot(String),
    /// A snapshot belongs to a different backend family or incompatible version.
    IncompatibleSnapshot {
        /// Field that failed compatibility checking.
        field: &'static str,
        /// Value required by this backend instance.
        expected: String,
        /// Value found in the snapshot.
        found: String,
    },
    /// The machine has halted and cannot execute another quantum.
    Halted,
    /// The backend failed while executing an instruction or accessing its bus.
    Execution(String),
    /// A bounded request exceeded a declared backend limit.
    Limit {
        /// The limited field, named for diagnostics.
        field: &'static str,
        /// The declared limit.
        limit: u64,
        /// The value that was requested.
        requested: u64,
    },
    /// A memory access falls outside the backend's address space.
    OutOfBounds {
        /// First requested address.
        address: u64,
        /// Requested byte count.
        length: usize,
        /// Exclusive upper bound of the addressable region.
        limit: u64,
    },
    /// Emulated time would overflow its representation.
    TickOverflow,
    /// An internal invariant failed in a way the backend cannot classify better.
    Internal(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(f, "invalid backend configuration: {message}")
            }
            Self::InvalidSnapshot(message) => write!(f, "invalid snapshot: {message}"),
            Self::IncompatibleSnapshot {
                field,
                expected,
                found,
            } => write!(
                f,
                "incompatible snapshot {field}: expected {expected}, found {found}"
            ),
            Self::Limit {
                field,
                limit,
                requested,
            } => write!(
                f,
                "backend limit `{field}` is {limit}, requested {requested}"
            ),
            Self::Halted => f.write_str("machine has halted"),
            Self::Execution(message) => write!(f, "backend execution error: {message}"),
            Self::OutOfBounds {
                address,
                length,
                limit,
            } => write!(
                f,
                "memory access out of bounds: address {address}, length {length}, limit {limit}"
            ),
            Self::TickOverflow => f.write_str("emulated tick overflow"),
            Self::Internal(message) => write!(f, "backend internal error: {message}"),
        }
    }
}

impl Error for BackendError {}

impl From<CoreError> for BackendError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::TickOverflow { .. } => Self::TickOverflow,
        }
    }
}
