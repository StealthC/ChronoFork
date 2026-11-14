//! Small, explicit error type for core invariants.

use std::error::Error;
use std::fmt;

/// Errors that can arise while manipulating core values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreError {
    /// Adding to a [`crate::Tick`] would overflow `u64`.
    TickOverflow {
        /// The tick that could not be advanced.
        current: u64,
        /// The requested increment.
        increment: u64,
    },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TickOverflow { current, increment } => write!(
                f,
                "emulated tick overflow: {current} + {increment} exceeds u64::MAX"
            ),
        }
    }
}

impl Error for CoreError {}
