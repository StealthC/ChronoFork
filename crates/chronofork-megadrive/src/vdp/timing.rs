//! VDP line/dot position and frame timing state.
//!
//! The device advances on the master-cycle timeline owned by the backend. This
//! module exposes the position fields the device snapshots in version 2;
//! phase 8.3 task 4 adds `advance_master`, the H/V counters and the status
//! derivation for HBlank/VBlank here, keeping every timing and status rule in
//! one place.
//!
//! The position is canonical state and must never be driven by host time. The
//! H counter stays an approximation (the documented per-line table), not a
//! dot-exact value, and interlace is not modelled.

use super::Vdp;

impl Vdp {
    /// Returns the current line within the video frame.
    #[must_use]
    pub const fn line(&self) -> u16 {
        self.line
    }

    /// Returns the current master-cycle dot within the line.
    #[must_use]
    pub const fn dot(&self) -> u16 {
        self.dot
    }

    /// Returns how many complete frames have elapsed.
    #[must_use]
    pub const fn frame_index(&self) -> u64 {
        self.frame_index
    }
}
