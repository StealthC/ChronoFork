//! Exact rational phase accumulator with linear interpolation.
//!
//! The audio devices produce one sample every fixed number of master cycles, so
//! their rate is a rational function of emulated time rather than a host clock.
//! [`RationalResampler`] tracks the position between two consecutive device
//! samples as an integer fraction of that period and interpolates linearly with
//! integer arithmetic only.
//!
//! # Interpolation model
//!
//! The resampler holds the two most recent input samples (`previous`, `current`)
//! and a phase in master cycles inside the current input period. The interpolated
//! value is `previous + (current - previous) * phase / period`, so the output
//! lags the newest input sample by at most one input period. That latency is
//! deterministic and part of the documented model; it is not host latency.

use std::error::Error;
use std::fmt;

use crate::audio::AudioSnapshotError;

/// A malformed resampler configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResampleError {
    /// The input period was zero master cycles.
    ZeroInputPeriod,
}

impl fmt::Display for ResampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInputPeriod => formatter.write_str("resampler input period is zero"),
        }
    }
}

impl Error for ResampleError {}

/// Integer phase accumulator and linear interpolation for one device clock.
///
/// The phase is measured in master cycles inside one input period, which keeps
/// the fraction exact: a device sample every 240 master cycles gives the phases
/// `0/240`, `1/240`, ... with no accumulated rounding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RationalResampler {
    /// Master cycles between two input samples; always non-zero.
    input_period: u64,
    /// Master cycles elapsed inside the current input period.
    phase: u64,
    /// Second most recent input sample.
    previous: i16,
    /// Most recent input sample.
    current: i16,
    /// Whether at least one input sample has been pushed.
    primed: bool,
}

impl RationalResampler {
    /// Creates a resampler for an input period of `input_period` master cycles.
    ///
    /// # Errors
    ///
    /// Returns [`ResampleError::ZeroInputPeriod`] when `input_period` is zero,
    /// because a zero-length period has no phase to accumulate.
    pub fn new(input_period: u64) -> Result<Self, ResampleError> {
        if input_period == 0 {
            return Err(ResampleError::ZeroInputPeriod);
        }
        Ok(Self::with_period(input_period))
    }

    /// Creates a resampler from a period the caller has already validated.
    pub(crate) fn with_period(input_period: u64) -> Self {
        debug_assert!(input_period > 0, "device sample periods are non-zero");
        Self {
            input_period,
            phase: 0,
            previous: 0,
            current: 0,
            primed: false,
        }
    }

    /// Returns how many input samples become due after `cycles` master cycles.
    pub fn samples_due(&self, cycles: u64) -> u64 {
        self.phase.saturating_add(cycles) / self.input_period
    }

    /// Records the next input sample.
    ///
    /// The first sample seeds both interpolation endpoints so the output starts
    /// at the input level instead of ramping up from zero.
    pub fn push(&mut self, sample: i16) {
        if self.primed {
            self.previous = self.current;
        } else {
            self.previous = sample;
            self.primed = true;
        }
        self.current = sample;
    }

    /// Advances the phase by `cycles` and returns the interpolated sample.
    ///
    /// Callers push [`RationalResampler::samples_due`] samples before advancing.
    /// Before the first sample the output is zero.
    pub fn advance(&mut self, cycles: u64) -> i16 {
        self.phase = self.phase.saturating_add(cycles) % self.input_period;
        if !self.primed {
            return 0;
        }
        interpolate(self.previous, self.current, self.phase, self.input_period)
    }

    /// Returns the current phase as `(elapsed, period)` in master cycles.
    #[must_use]
    pub const fn phase(&self) -> (u64, u64) {
        (self.phase, self.input_period)
    }

    /// Returns the raw interpolation state for a containing device snapshot.
    pub(crate) const fn snapshot_parts(&self) -> (u64, i16, i16, bool) {
        (self.phase, self.previous, self.current, self.primed)
    }

    /// Restores the raw interpolation state from a containing device snapshot.
    pub(crate) fn restore_parts(
        &mut self,
        phase: u64,
        previous: i16,
        current: i16,
        primed: bool,
    ) -> Result<(), AudioSnapshotError> {
        if phase >= self.input_period {
            return Err(AudioSnapshotError::Value {
                field: "resampler phase",
            });
        }
        self.phase = phase;
        self.previous = previous;
        self.current = current;
        self.primed = primed;
        Ok(())
    }
}

/// Linearly interpolates between two samples at `phase / period`.
///
/// The 128-bit intermediate keeps the product exact for every `u64` period; the
/// result saturates at the `i16` range.
fn interpolate(previous: i16, current: i16, phase: u64, period: u64) -> i16 {
    let delta = i128::from(current) - i128::from(previous);
    let offset = delta * i128::from(phase) / i128::from(period);
    (i128::from(previous) + offset).clamp(i128::from(i16::MIN), i128::from(i16::MAX)) as i16
}
