//! The stop report returned by one [`Debugger::run`](crate::Debugger::run) call.

use chronofork_backend_api::DebugEvent;
use chronofork_core::{StateHash, Tick};

use crate::trigger::{BreakpointId, EventBreakpointId, LogEntry, WatchpointId};

/// Why a debugger run stopped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// An enabled breakpoint matched before the next instruction executed.
    Breakpoint {
        /// The matching breakpoint.
        id: BreakpointId,
    },
    /// At least one host watchpoint matched a byte access of the last
    /// instruction, reported after that instruction.
    Watchpoint {
        /// Every matching host watchpoint, in first-hit order and deduplicated.
        ids: Vec<WatchpointId>,
    },
    /// The first matching enabled event breakpoint in id order matched a retained
    /// event of the last instruction.
    Event {
        /// The matching event breakpoint.
        id: EventBreakpointId,
    },
    /// The machine was already halted at an instruction boundary.
    Halted,
    /// The requested step limit was reached at an instruction boundary.
    StepLimit,
}

/// The exact state and debug records at a stop.
///
/// `steps` counts the instructions this call executed. For a
/// [`StopReason::Breakpoint`], [`StopReason::Halted`] and
/// [`StopReason::StepLimit`] stop, `tick` and `state_hash` describe the state
/// **before** the next instruction; for a [`StopReason::Watchpoint`] and
/// [`StopReason::Event`] stop they describe the state **after** the instruction
/// that produced the event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugRun {
    /// Why the run stopped.
    pub reason: StopReason,
    /// Instructions executed by this call.
    pub steps: u64,
    /// Emulated tick at the stop.
    pub tick: Tick,
    /// State digest at the stop.
    pub state_hash: StateHash,
    /// Whether the machine has halted at the stop.
    pub halted: bool,
    /// Events of the stopping instruction that matched an enabled event
    /// breakpoint, in recording order.
    ///
    /// This is empty for a pre-step stop and for a step that matched none.
    pub matched_events: Vec<DebugEvent>,
    /// Events the backend event ring has evicted so far.
    pub dropped_events: u64,
    /// Logpoint entries retained by the bounded log ring, oldest first.
    pub log_entries: Vec<LogEntry>,
    /// Logpoint entries the bounded ring has evicted so far.
    pub dropped_log_entries: u64,
}
