//! Validated trace metadata.

use chronofork_core::{StateHash, Tick};

/// Why capture stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceStopReason {
    /// The requested step count was reached.
    StepLimit,
    /// The machine halted before the step count was reached.
    Halted,
    /// The configured stop trigger matched at an instruction boundary.
    StopCondition,
}

impl TraceStopReason {
    /// Returns the stable binary code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::StepLimit => 0,
            Self::Halted => 1,
            Self::StopCondition => 2,
        }
    }

    /// Returns the stable lowercase name used by TSV export.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::StepLimit => "step_limit",
            Self::Halted => "halted",
            Self::StopCondition => "stop_condition",
        }
    }

    /// Decodes a canonical stop-reason byte.
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::StepLimit),
            1 => Some(Self::Halted),
            2 => Some(Self::StopCondition),
            _ => None,
        }
    }
}

/// Validated metadata describing one captured trace.
///
/// [`crate::Trace::new`] enforces every bound and relationship documented here;
/// the fields are public because they are plain, validated data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceMeta {
    /// Stable backend identifier, 1 to [`crate::MAX_BACKEND_ID_BYTES`] UTF-8 bytes.
    pub backend: String,
    /// Backend version string, 1 to [`crate::MAX_BACKEND_VERSION_BYTES`] UTF-8 bytes.
    pub backend_version: String,
    /// Canonical state hash at capture start.
    pub state_hash: StateHash,
    /// Emulated tick at capture start.
    pub start_tick: Tick,
    /// Last observed tick; never earlier than `start_tick`.
    pub end_tick: Tick,
    /// Ring capacity, `1..=`[`crate::MAX_TRACE_CAPACITY`].
    pub capacity: usize,
    /// Absolute index of the first retained record; always equals `dropped`.
    pub first_index: u64,
    /// Number of records evicted by the circular buffer.
    pub dropped: u64,
    /// Why capture stopped.
    pub stop_reason: TraceStopReason,
    /// Whether the start trigger matched.
    pub triggered: bool,
    /// Whether backend debug events were available and configured.
    pub events_observed: bool,
}
