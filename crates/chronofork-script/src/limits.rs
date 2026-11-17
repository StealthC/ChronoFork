//! Documented script limits, defaults and maxima.
//!
//! Every bound is validated before a run starts. The same values are enforced
//! by the evaluator before the corresponding work is performed, so a rejected
//! run never executes a partial advance, inserts a variable or returns output
//! beyond its budget.

use crate::error::LimitsError;

/// Default maximum source size: 64 KiB.
pub const DEFAULT_SOURCE_BYTES: usize = 64 * 1024;
/// Maximum configurable source size: 1 MiB.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Default operation budget: 1,000,000.
pub const DEFAULT_OPERATIONS: u64 = 1_000_000;
/// Maximum configurable operation budget: 100,000,000.
pub const MAX_OPERATIONS: u64 = 100_000_000;
/// Default loop-iteration budget: 1,000,000.
pub const DEFAULT_LOOP_ITERATIONS: u64 = 1_000_000;
/// Maximum configurable loop-iteration budget: 100,000,000.
pub const MAX_LOOP_ITERATIONS: u64 = 100_000_000;
/// Default number of live variables: 256.
pub const DEFAULT_VARIABLES: usize = 256;
/// Maximum configurable number of live variables: 4,096.
pub const MAX_VARIABLES: usize = 4096;
/// Default aggregate text budget: 1 MiB.
pub const DEFAULT_STRING_BYTES: usize = 1024 * 1024;
/// Maximum configurable aggregate text budget: 16 MiB.
pub const MAX_STRING_BYTES: usize = 16 * 1024 * 1024;
/// Default report text budget: 64 KiB.
pub const DEFAULT_REPORT_BYTES: usize = 64 * 1024;
/// Maximum configurable report text budget: 1 MiB.
pub const MAX_REPORT_BYTES: usize = 1024 * 1024;
/// Default backend step budget: 1,000,000.
pub const DEFAULT_STEPS: u64 = 1_000_000;
/// Maximum configurable backend step budget: 100,000,000.
pub const MAX_STEPS: u64 = 100_000_000;
/// Default retained trace-record budget: 1,000,000.
pub const DEFAULT_TRACE_RECORDS: u64 = 1_000_000;
/// Maximum retained trace-record budget: 1,000,000.
pub const MAX_TRACE_RECORDS: u64 = 1_000_000;
/// Default checkpoint budget: 1,024.
pub const DEFAULT_SNAPSHOTS: usize = 1024;
/// Maximum checkpoint budget: 1,024.
pub const MAX_SNAPSHOTS: usize = 1024;
/// Default wall-time deadline in milliseconds: 30,000.
pub const DEFAULT_WALL_TIME_MS: u64 = 30_000;
/// Upper bound for a single `Text` value authored by a script, in UTF-8 bytes.
pub const MAX_TEXT_BYTES: usize = 4 * 1024;
/// Wall-time checks additionally happen every this many operations.
pub const WALL_CHECK_INTERVAL_OPERATIONS: u64 = 256;

/// Sandbox limits for one script run.
///
/// Defaults and maxima are documented by the `DEFAULT_*` and `MAX_*` constants
/// in this module. `wall_time_ms` is the only host-time input; `None` disables
/// the deadline, and a successful run's report never contains elapsed time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScriptLimits {
    /// Maximum UTF-8 source size in bytes, checked before lexing.
    pub source_bytes: usize,
    /// Maximum executed statements, expression nodes, loop iterations and
    /// builtin calls, checked before the work.
    pub operations: u64,
    /// Maximum combined `repeat` and `while` iterations.
    pub loop_iterations: u64,
    /// Maximum number of distinct live variables.
    pub variables: usize,
    /// Maximum aggregate live variable, print, report and trace text in bytes.
    pub string_bytes: usize,
    /// Maximum report key/value text in bytes.
    pub report_bytes: usize,
    /// Maximum backend steps, including steps requested by later host builtins.
    pub steps: u64,
    /// Maximum retained trace records in the stored trace.
    pub trace_records: u64,
    /// Maximum checkpoints created by the script session set.
    pub snapshots: usize,
    /// Wall-time deadline in milliseconds; `None` disables enforcement.
    pub wall_time_ms: Option<u64>,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            source_bytes: DEFAULT_SOURCE_BYTES,
            operations: DEFAULT_OPERATIONS,
            loop_iterations: DEFAULT_LOOP_ITERATIONS,
            variables: DEFAULT_VARIABLES,
            string_bytes: DEFAULT_STRING_BYTES,
            report_bytes: DEFAULT_REPORT_BYTES,
            steps: DEFAULT_STEPS,
            trace_records: DEFAULT_TRACE_RECORDS,
            snapshots: DEFAULT_SNAPSHOTS,
            wall_time_ms: Some(DEFAULT_WALL_TIME_MS),
        }
    }
}

impl ScriptLimits {
    /// Validates every configured bound against its documented range.
    ///
    /// Zero is rejected for every numeric limit; `wall_time_ms` accepts either
    /// `Some` (including zero, meaning an immediate deadline) or `None`.
    ///
    /// # Errors
    ///
    /// Returns a typed [`LimitsError`] naming the first invalid field.
    pub fn validate(&self) -> Result<(), LimitsError> {
        check_usize("source_bytes", self.source_bytes, MAX_SOURCE_BYTES)?;
        check_u64("operations", self.operations, MAX_OPERATIONS)?;
        check_u64("loop_iterations", self.loop_iterations, MAX_LOOP_ITERATIONS)?;
        check_usize("variables", self.variables, MAX_VARIABLES)?;
        check_usize("string_bytes", self.string_bytes, MAX_STRING_BYTES)?;
        check_usize("report_bytes", self.report_bytes, MAX_REPORT_BYTES)?;
        check_u64("steps", self.steps, MAX_STEPS)?;
        check_u64("trace_records", self.trace_records, MAX_TRACE_RECORDS)?;
        check_usize("snapshots", self.snapshots, MAX_SNAPSHOTS)?;
        Ok(())
    }
}

fn check_usize(field: &'static str, value: usize, maximum: usize) -> Result<(), LimitsError> {
    let value = u64::try_from(value).unwrap_or(u64::MAX);
    let maximum = u64::try_from(maximum).unwrap_or(u64::MAX);
    check_u64(field, value, maximum)
}

fn check_u64(field: &'static str, value: u64, maximum: u64) -> Result<(), LimitsError> {
    if value == 0 {
        return Err(LimitsError::zero(field, maximum));
    }
    if value > maximum {
        return Err(LimitsError::above(field, value, maximum));
    }
    Ok(())
}
