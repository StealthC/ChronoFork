//! Typed trace failures with stable diagnostic codes.

use std::error::Error;
use std::fmt;

use chronofork_backend_api::BackendError;

/// A trace format, capture or query failure.
///
/// Every variant carries a stable [`TraceError::code`] for CLI diagnostics and
/// implements [`Display`](fmt::Display) with deterministic text: no host path,
/// host time, process id or locale-dependent formatting appears in a message.
/// Production code never panics on a trace failure; it returns this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceError {
    /// The input does not start with [`crate::TRACE_MAGIC`].
    InvalidMagic,
    /// The input declares a format version this crate does not accept.
    UnsupportedVersion {
        /// Version found in the input.
        found: u16,
    },
    /// A record uses an undefined kind code.
    UnknownRecordKind {
        /// Kind code found in the input.
        kind: u8,
    },
    /// The input ended before a declared field or body.
    TruncatedInput {
        /// Bytes the decoder needed.
        needed: usize,
        /// Offset at which the read started.
        offset: usize,
        /// Bytes available at that offset.
        remaining: usize,
    },
    /// Bytes remain after the last declared record.
    TrailingBytes {
        /// Number of unexpected bytes.
        count: usize,
    },
    /// A metadata field is empty, over its byte bound or inconsistent.
    InvalidMetadata(String),
    /// The declared record count is above a limit or above the capacity.
    InvalidRecordCount {
        /// Declared record count.
        count: usize,
        /// Declared capacity, or the maximum record count when unavailable.
        capacity: usize,
    },
    /// A capacity is zero or above [`crate::MAX_TRACE_CAPACITY`].
    InvalidCapacity {
        /// Capacity that was rejected.
        capacity: usize,
    },
    /// Retained records are not contiguous from `first_index`.
    NonContiguousIndex {
        /// Absolute index the position requires.
        expected: u64,
        /// Absolute index found in the record.
        found: u64,
    },
    /// A record tick is smaller than its predecessor.
    DecreasingTick {
        /// Raw tick of the predecessor.
        previous: u64,
        /// Raw tick of the offending record.
        found: u64,
    },
    /// A record field is outside its documented range or its capture window.
    InvalidRecordField(String),
    /// The encoded trace or the requested buffer exceeds a documented limit.
    TooLarge {
        /// Limit in bytes.
        limit: usize,
        /// Observed size in bytes.
        found: usize,
    },
    /// The backend inspection has no register named `pc`.
    ProgramCounterUnavailable,
    /// The backend `pc` register does not fit the trace record's `u32` field.
    InvalidProgramCounter {
        /// Register value that was rejected.
        value: u64,
    },
    /// The 2-byte big-endian opcode read failed at an instruction boundary.
    OpcodeReadFailed {
        /// Program counter whose opcode could not be read.
        pc: u32,
        /// Backend diagnostic text.
        message: String,
    },
    /// The backend advertises `debug_events` but exposes no debug access.
    EventObservationUnavailable,
    /// The backend event ring evicted events before the capture drained them.
    EventRingOverflow,
    /// A capture or query configuration value is outside its documented bound.
    InvalidConfiguration(String),
    /// A structured trace query is invalid.
    InvalidQuery(String),
    /// A wrapped backend failure.
    Backend(BackendError),
}

impl TraceError {
    /// Returns the stable diagnostic code of this failure.
    ///
    /// The code never changes between releases for the same variant, so a CLI
    /// can print it without depending on message wording.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidMagic => "invalid_magic",
            Self::UnsupportedVersion { .. } => "unsupported_version",
            Self::UnknownRecordKind { .. } => "unknown_record_kind",
            Self::TruncatedInput { .. } => "truncated_input",
            Self::TrailingBytes { .. } => "trailing_bytes",
            Self::InvalidMetadata(_) => "invalid_metadata",
            Self::InvalidRecordCount { .. } => "invalid_record_count",
            Self::InvalidCapacity { .. } => "invalid_capacity",
            Self::NonContiguousIndex { .. } => "non_contiguous_index",
            Self::DecreasingTick { .. } => "decreasing_tick",
            Self::InvalidRecordField(_) => "invalid_record_field",
            Self::TooLarge { .. } => "too_large",
            Self::ProgramCounterUnavailable => "program_counter_unavailable",
            Self::InvalidProgramCounter { .. } => "invalid_program_counter",
            Self::OpcodeReadFailed { .. } => "opcode_read_failed",
            Self::EventObservationUnavailable => "event_observation_unavailable",
            Self::EventRingOverflow => "event_ring_overflow",
            Self::InvalidConfiguration(_) => "invalid_configuration",
            Self::InvalidQuery(_) => "invalid_query",
            Self::Backend(_) => "backend_error",
        }
    }
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => f.write_str("trace magic is not CFTRACE1"),
            Self::UnsupportedVersion { found } => write!(
                f,
                "trace format version {found} is unsupported, expected {}",
                crate::TRACE_FORMAT_VERSION
            ),
            Self::UnknownRecordKind { kind } => write!(f, "unknown trace record kind {kind}"),
            Self::TruncatedInput {
                needed,
                offset,
                remaining,
            } => write!(
                f,
                "trace input is truncated: needed {needed} bytes at offset {offset}, \
                 {remaining} remain"
            ),
            Self::TrailingBytes { count } => {
                write!(f, "trace input has {count} trailing byte(s)")
            }
            Self::InvalidMetadata(message) => write!(f, "invalid trace metadata: {message}"),
            Self::InvalidRecordCount { count, capacity } => write!(
                f,
                "invalid trace record count {count} for capacity {capacity}"
            ),
            Self::InvalidCapacity { capacity } => write!(
                f,
                "trace capacity {capacity} is outside 1..={}",
                crate::MAX_TRACE_CAPACITY
            ),
            Self::NonContiguousIndex { expected, found } => write!(
                f,
                "non-contiguous trace index: expected {expected}, found {found}"
            ),
            Self::DecreasingTick { previous, found } => {
                write!(
                    f,
                    "trace tick decreased: previous {previous}, found {found}"
                )
            }
            Self::InvalidRecordField(message) => write!(f, "invalid trace record: {message}"),
            Self::TooLarge { limit, found } => {
                write!(f, "trace is {found} bytes, limit is {limit}")
            }
            Self::ProgramCounterUnavailable => {
                f.write_str("backend inspection does not expose a \"pc\" register")
            }
            Self::InvalidProgramCounter { value } => {
                write!(f, "backend program counter {value} does not fit in 32 bits")
            }
            Self::OpcodeReadFailed { pc, message } => {
                write!(f, "cannot read the opcode at pc 0x{pc:08X}: {message}")
            }
            Self::EventObservationUnavailable => {
                f.write_str("backend advertises debug_events but exposes no debug access")
            }
            Self::EventRingOverflow => {
                f.write_str("backend event ring overflowed before capture drained it")
            }
            Self::InvalidConfiguration(message) => {
                write!(f, "invalid trace configuration: {message}")
            }
            Self::InvalidQuery(message) => write!(f, "invalid trace query: {message}"),
            Self::Backend(error) => write!(f, "backend error: {error}"),
        }
    }
}

impl Error for TraceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BackendError> for TraceError {
    fn from(error: BackendError) -> Self {
        Self::Backend(error)
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::BackendError;

    use super::TraceError;

    #[test]
    fn every_variant_has_a_stable_code_and_display_text() {
        let cases = [
            (TraceError::InvalidMagic, "invalid_magic"),
            (
                TraceError::UnsupportedVersion { found: 2 },
                "unsupported_version",
            ),
            (
                TraceError::UnknownRecordKind { kind: 9 },
                "unknown_record_kind",
            ),
            (
                TraceError::TruncatedInput {
                    needed: 2,
                    offset: 1,
                    remaining: 0,
                },
                "truncated_input",
            ),
            (TraceError::TrailingBytes { count: 3 }, "trailing_bytes"),
            (
                TraceError::InvalidMetadata("empty backend".to_owned()),
                "invalid_metadata",
            ),
            (
                TraceError::InvalidRecordCount {
                    count: 4,
                    capacity: 2,
                },
                "invalid_record_count",
            ),
            (
                TraceError::InvalidCapacity { capacity: 0 },
                "invalid_capacity",
            ),
            (
                TraceError::NonContiguousIndex {
                    expected: 3,
                    found: 4,
                },
                "non_contiguous_index",
            ),
            (
                TraceError::DecreasingTick {
                    previous: 4,
                    found: 3,
                },
                "decreasing_tick",
            ),
            (
                TraceError::InvalidRecordField("width 0".to_owned()),
                "invalid_record_field",
            ),
            (TraceError::TooLarge { limit: 8, found: 9 }, "too_large"),
            (
                TraceError::ProgramCounterUnavailable,
                "program_counter_unavailable",
            ),
            (
                TraceError::InvalidProgramCounter {
                    value: u64::from(u32::MAX) + 1,
                },
                "invalid_program_counter",
            ),
            (
                TraceError::OpcodeReadFailed {
                    pc: 0x200,
                    message: "out of bounds".to_owned(),
                },
                "opcode_read_failed",
            ),
            (
                TraceError::EventObservationUnavailable,
                "event_observation_unavailable",
            ),
            (TraceError::EventRingOverflow, "event_ring_overflow"),
            (
                TraceError::InvalidConfiguration("steps 0".to_owned()),
                "invalid_configuration",
            ),
            (
                TraceError::InvalidQuery("limit 0".to_owned()),
                "invalid_query",
            ),
            (TraceError::Backend(BackendError::Halted), "backend_error"),
        ];
        for (error, expected) in cases {
            assert_eq!(error.code(), expected, "code for {error:?}");
            assert!(
                !error.to_string().is_empty(),
                "display text must not be empty for {error:?}"
            );
        }
    }

    #[test]
    fn backend_errors_are_the_only_source() {
        use std::error::Error;

        let wrapped = TraceError::Backend(BackendError::Halted);
        assert!(wrapped.source().is_some());
        assert!(TraceError::InvalidMagic.source().is_none());
        assert_eq!(
            wrapped.to_string(),
            "backend error: machine has halted",
            "the wrapped backend text is preserved"
        );
        assert_eq!(
            TraceError::InvalidMagic.to_string(),
            "trace magic is not CFTRACE1"
        );
    }
}
