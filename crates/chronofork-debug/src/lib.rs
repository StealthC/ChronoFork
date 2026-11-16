//! Host-layer debugger for ChronoFork backends.
//!
//! [`Debugger`] borrows one [`Backend`](chronofork_backend_api::Backend) and adds
//! host-owned triggers on top of it: PC/range/conditional breakpoints,
//! read/write watchpoints, event breakpoints and logpoints, evaluated by a
//! deterministic run loop. The crate depends only on `chronofork-core` and
//! `chronofork-backend-api`: it owns no session, storage, transport or UI state.
//!
//! # Host and backend responsibilities
//!
//! A breakpoint, logpoint or event breakpoint only needs the next instruction
//! address, a register map, side-effect-free memory reads and the backend's
//! retained debug events, so the host evaluates them over
//! [`inspect`](chronofork_backend_api::Backend::inspect),
//! [`read_memory`](chronofork_backend_api::Backend::read_memory) and
//! [`DebugAccess`](chronofork_backend_api::DebugAccess). A watchpoint needs the
//! emulated access itself, so the host installs it in the backend
//! [`DebugConfig`](chronofork_backend_api::DebugConfig) and maps the backend's
//! `Watchpoint` events back to host triggers. The host never approximates a
//! watchpoint by diffing memory windows, and a backend without debug observation
//! yields [`DebugError::WatchpointsUnsupported`].
//!
//! # Run order
//!
//! [`Debugger::run`] inspects the state before each instruction, in trigger id
//! order: a matching enabled breakpoint stops **before** the instruction, then
//! matching logpoints append bounded entries, then a halted machine stops with
//! [`StopReason::Halted`] and then a reached step limit stops with
//! [`StopReason::StepLimit`]. Otherwise one neutral step executes; when its
//! retained events contain a watchpoint hit the run stops with
//! [`StopReason::Watchpoint`], and otherwise the first matching enabled event
//! breakpoint in id order stops with [`StopReason::Event`]. A watchpoint stop
//! wins over an event breakpoint stop for the same step, and a breakpoint at the
//! current PC wins over `Halted` and `StepLimit`.
//!
//! # Bounds
//!
//! At most [`MAX_BREAKPOINTS`], [`MAX_WATCHPOINTS`],
//! [`MAX_EVENT_BREAKPOINTS`] and [`MAX_LOGPOINTS`] triggers are stored; a full
//! table is a [`DebugError::TableFull`]. The backend event ring is bounded by
//! [`MAX_EVENT_CAPACITY`] (default [`DEFAULT_EVENT_CAPACITY`]) and the logpoint
//! ring by [`MAX_LOG_CAPACITY`] (default [`DEFAULT_LOG_CAPACITY`]); both count
//! evictions instead of growing without bound. [`Debugger::run`] accepts a step
//! limit in `1..=MAX_RUN_STEP_LIMIT`.
//!
//! # Non-canonical state
//!
//! Triggers, the logpoint ring and the backend event ring are debug data. They
//! are absent from snapshots, state hashes and configuration hashes, and
//! restoring a backend snapshot does not rewind them. Debugger memory reads and
//! writes are not emulated bus accesses and never produce events.
//!
//! See [ADR-0010](../../../docs/adr/0010-debugging-depth.md) and the
//! [M4 debugging-depth acceptance plan](../../../docs/quality/m4-debugging-depth-acceptance.md).

#![cfg_attr(test, allow(clippy::expect_used))]

mod debugger;
mod error;
mod run;
mod trigger;

pub use chronofork_backend_api::{
    DebugAddressRange, DebugEvent, DebugEventKind, WatchpointAccess, WatchpointSpec,
};
pub use debugger::{Debugger, TriggerSnapshot};
pub use error::{DebugError, InvalidLogMessage, TriggerKind};
pub use run::{DebugRun, StopReason};
pub use trigger::{
    BreakpointId, BreakpointSpec, Condition, ConditionOperand, EventBreakpointId, EventFilter,
    InterruptPhase, Location, LogEntry, Logpoint, LogpointId, Relation, TriggerId, WatchpointId,
};

/// Event-ring capacity used by [`Debugger::new`], in events.
pub const DEFAULT_EVENT_CAPACITY: usize = chronofork_backend_api::DEFAULT_DEBUG_EVENT_CAPACITY;
/// Largest event-ring capacity accepted by [`Debugger::with_event_capacity`].
pub const MAX_EVENT_CAPACITY: usize = chronofork_backend_api::MAX_DEBUG_EVENT_CAPACITY;
/// Largest accepted breakpoint table, in breakpoints.
pub const MAX_BREAKPOINTS: usize = 64;
/// Largest accepted host watchpoint table, in watchpoints.
///
/// This is the backend observation limit
/// ([`chronofork_backend_api::MAX_WATCHPOINTS`]): the host installs its enabled
/// watchpoints in one backend [`DebugConfig`](chronofork_backend_api::DebugConfig).
pub const MAX_WATCHPOINTS: usize = chronofork_backend_api::MAX_WATCHPOINTS;
/// Largest accepted event breakpoint table, in event breakpoints.
pub const MAX_EVENT_BREAKPOINTS: usize = 64;
/// Largest accepted logpoint table, in logpoints.
pub const MAX_LOGPOINTS: usize = 64;
/// Logpoint ring capacity used by a debugger, in entries.
pub const DEFAULT_LOG_CAPACITY: usize = 256;
/// Largest logpoint ring capacity, in entries.
pub const MAX_LOG_CAPACITY: usize = 4_096;
/// Largest accepted logpoint message, in bytes.
pub const MAX_LOG_MESSAGE_BYTES: usize = 128;
/// Largest accepted [`Debugger::run`] step limit.
pub const MAX_RUN_STEP_LIMIT: u64 = 1_000_000;

/// The default logpoint ring must fit inside the documented hard bound.
const _: () = assert!(DEFAULT_LOG_CAPACITY <= MAX_LOG_CAPACITY);
