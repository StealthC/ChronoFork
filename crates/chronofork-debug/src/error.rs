//! Typed host-debugger errors.

use std::error::Error;
use std::fmt;

use chronofork_backend_api::BackendError;

use crate::trigger::TriggerId;

/// The trigger table that cannot accept another entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggerKind {
    /// The breakpoint table.
    Breakpoint,
    /// The host watchpoint table.
    Watchpoint,
    /// The event breakpoint table.
    EventBreakpoint,
    /// The logpoint table.
    Logpoint,
}

impl fmt::Display for TriggerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Breakpoint => "breakpoint",
            Self::Watchpoint => "watchpoint",
            Self::EventBreakpoint => "event breakpoint",
            Self::Logpoint => "logpoint",
        })
    }
}

/// Why a logpoint message was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidLogMessage {
    /// The message contains no bytes.
    Empty,
    /// The message exceeds
    /// [`MAX_LOG_MESSAGE_BYTES`](crate::MAX_LOG_MESSAGE_BYTES).
    TooLong {
        /// Observed message length, in bytes.
        length: usize,
    },
    /// The message contains a line break (`\n` or `\r`).
    ContainsLineBreak,
}

impl fmt::Display for InvalidLogMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("logpoint message must not be empty"),
            Self::TooLong { length } => write!(
                f,
                "logpoint message is {length} bytes, exceeding the {} byte limit",
                crate::MAX_LOG_MESSAGE_BYTES
            ),
            Self::ContainsLineBreak => {
                f.write_str("logpoint message must not contain a line break")
            }
        }
    }
}

/// A failure in host-debugger validation or execution.
///
/// Every failure is a typed value: unknown trigger ids, unknown registers,
/// missing `pc`, invalid ranges, condition lengths, logpoint messages or limits,
/// full trigger tables, unsupported backend observation and backend errors are
/// distinct variants. No stringly fallback and no silent no-op exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebugError {
    /// The backend rejected an operation.
    Backend(BackendError),
    /// The backend does not expose debug observation through
    /// [`Backend::debug_access`](chronofork_backend_api::Backend::debug_access),
    /// so a watchpoint cannot be installed and events cannot be read.
    WatchpointsUnsupported,
    /// A location match needed the `pc` register, but the inspection does not
    /// expose it.
    MissingProgramCounter,
    /// No installed trigger has the given id.
    UnknownTrigger(TriggerId),
    /// A condition names a register that
    /// [`MachineInspection::registers`](chronofork_backend_api::MachineInspection::registers)
    /// does not contain.
    UnknownRegister(String),
    /// The backend watchpoint id of a new watchpoint is already installed.
    DuplicateWatchpointId(u32),
    /// An inclusive location range has `start > end`.
    InvalidRange {
        /// First address of the rejected range.
        start: u32,
        /// Last address of the rejected range.
        end: u32,
    },
    /// A memory condition length is outside `1..=8` bytes.
    InvalidConditionLength(u8),
    /// A logpoint message was rejected.
    InvalidLogMessage(InvalidLogMessage),
    /// A step limit is outside `1..=MAX_RUN_STEP_LIMIT`.
    InvalidStepLimit(u64),
    /// An event-ring capacity is outside `1..=MAX_EVENT_CAPACITY`.
    InvalidEventCapacity(usize),
    /// A trigger table already holds its maximum number of entries.
    TableFull(TriggerKind),
}

impl fmt::Display for DebugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(error) => write!(f, "backend error: {error}"),
            Self::WatchpointsUnsupported => {
                f.write_str("the backend does not support debug observation")
            }
            Self::MissingProgramCounter => {
                f.write_str("the inspection does not expose the required pc register")
            }
            Self::UnknownTrigger(id) => {
                write!(f, "unknown {:?} trigger id {}", id, id.value())
            }
            Self::UnknownRegister(name) => {
                write!(f, "unknown register {name}")
            }
            Self::DuplicateWatchpointId(id) => {
                write!(f, "backend watchpoint id {id} is already installed")
            }
            Self::InvalidRange { start, end } => write!(
                f,
                "invalid inclusive location range: start {start} is greater than end {end}"
            ),
            Self::InvalidConditionLength(length) => {
                write!(f, "memory condition length {length} is outside 1..=8 bytes")
            }
            Self::InvalidLogMessage(reason) => {
                write!(f, "invalid logpoint message: {reason}")
            }
            Self::InvalidStepLimit(limit) => write!(
                f,
                "step limit {limit} is outside 1..={}",
                crate::MAX_RUN_STEP_LIMIT
            ),
            Self::InvalidEventCapacity(capacity) => write!(
                f,
                "event capacity {capacity} is outside 1..={}",
                crate::MAX_EVENT_CAPACITY
            ),
            Self::TableFull(kind) => {
                write!(f, "the {kind} table is full")
            }
        }
    }
}

impl Error for DebugError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BackendError> for DebugError {
    fn from(error: BackendError) -> Self {
        Self::Backend(error)
    }
}
