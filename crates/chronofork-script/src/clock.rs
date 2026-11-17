//! Injected wall-time source for deterministic limit enforcement.
//!
//! Wall time is the only host-time input of the script runtime. It is read
//! through this trait so production can use [`std::time::Instant`] while tests
//! inject a fake clock. A passing report never contains elapsed time, so clock
//! advances cannot change a successful result.

use std::fmt;
use std::time::Instant;

/// A monotonic millisecond clock used only to enforce `wall_time_ms`.
///
/// Implementations must be non-decreasing. `now_millis` reports elapsed
/// milliseconds since an arbitrary origin chosen by the implementation; the
/// runtime compares it with the configured deadline and never includes it in a
/// report.
pub trait ScriptClock: Send + Sync {
    /// Returns monotonic elapsed milliseconds.
    fn now_millis(&self) -> u64;
}

/// Production clock backed by [`std::time::Instant`].
#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Creates a clock whose origin is the current instant.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptClock for SystemClock {
    fn now_millis(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

impl fmt::Debug for dyn ScriptClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<script clock>")
    }
}
