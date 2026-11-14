//! Emulated time.
//!
//! ChronoFork never uses host wall-clock time as an implicit input to emulated
//! hardware. A [`Tick`] is a backend-defined quantum (an instruction, a frame,
//! a master-clock cycle) that must advance monotonically.

use std::fmt;

use crate::error::CoreError;

/// A logical instant in one emulated machine's timeline.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Tick(u64);

impl Tick {
    /// The initial tick of a fresh machine.
    pub const ZERO: Self = Self(0);

    /// Creates a tick from a raw count.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw count.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Adds `increment`, reporting overflow instead of wrapping.
    pub const fn checked_add(self, increment: u64) -> Result<Self, CoreError> {
        match self.0.checked_add(increment) {
            Some(value) => Ok(Self(value)),
            None => Err(CoreError::TickOverflow {
                current: self.0,
                increment,
            }),
        }
    }

    /// Advances by one tick.
    pub const fn next(self) -> Result<Self, CoreError> {
        self.checked_add(1)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Tick;

    #[test]
    fn tick_advances_monotonically() {
        let first = Tick::ZERO.next().expect("zero can advance");
        let second = first.next().expect("one can advance");
        assert_eq!(first.raw(), 1);
        assert_eq!(second.raw(), 2);
        assert!(second > first);
    }

    #[test]
    fn tick_overflow_is_an_error() {
        let last = Tick::from_raw(u64::MAX);
        assert!(last.next().is_err());
    }
}
