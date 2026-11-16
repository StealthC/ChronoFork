//! Host trigger types: ids, locations, conditions, event filters and logpoints.

use chronofork_backend_api::{DebugAddressRange, DebugEventKind};
use chronofork_core::Tick;

macro_rules! trigger_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            /// Returns the numeric value assigned by the debugger.
            #[must_use]
            pub const fn value(self) -> u32 {
                self.0
            }

            /// Creates an id from a table key.
            pub(crate) const fn new(value: u32) -> Self {
                Self(value)
            }
        }
    };
}

trigger_id!(
    /// Identifies one installed host breakpoint.
    ///
    /// Breakpoint ids are assigned per table, start at one and are never shared
    /// with another trigger kind.
    BreakpointId
);
trigger_id!(
    /// Identifies one installed host watchpoint.
    ///
    /// This is the host trigger id, not the backend
    /// [`WatchpointSpec`](chronofork_backend_api::WatchpointSpec) id that a
    /// `Watchpoint` event carries.
    WatchpointId
);
trigger_id!(
    /// Identifies one installed host event breakpoint.
    EventBreakpointId
);
trigger_id!(
    /// Identifies one installed host logpoint.
    LogpointId
);

/// Identifies one installed trigger of any kind.
///
/// The variants keep the per-kind id spaces distinct, so a breakpoint id can
/// never be mistaken for a watchpoint, event breakpoint or logpoint id.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriggerId {
    /// A host breakpoint.
    Breakpoint(BreakpointId),
    /// A host watchpoint.
    Watchpoint(WatchpointId),
    /// A host event breakpoint.
    EventBreakpoint(EventBreakpointId),
    /// A host logpoint.
    Logpoint(LogpointId),
}

impl TriggerId {
    /// Returns the numeric id inside its trigger kind.
    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Self::Breakpoint(id) => id.value(),
            Self::Watchpoint(id) => id.value(),
            Self::EventBreakpoint(id) => id.value(),
            Self::Logpoint(id) => id.value(),
        }
    }
}

/// The program-counter range a breakpoint or logpoint applies to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Location {
    /// Matches exactly one program counter.
    Address(u32),
    /// Matches every program counter in the inclusive range `start..=end`.
    Range {
        /// First matching program counter.
        start: u32,
        /// Last matching program counter.
        end: u32,
    },
}

impl Location {
    /// Returns whether `pc` lies inside this location.
    pub(crate) fn matches(self, pc: u64) -> bool {
        match self {
            Self::Address(address) => pc == u64::from(address),
            Self::Range { start, end } => pc >= u64::from(start) && pc <= u64::from(end),
        }
    }
}

/// Comparison operator of a breakpoint condition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Relation {
    /// Actual equals expected.
    Equal,
    /// Actual differs from expected.
    NotEqual,
    /// Actual is less than expected.
    Less,
    /// Actual is less than or equal to expected.
    LessOrEqual,
    /// Actual is greater than expected.
    Greater,
    /// Actual is greater than or equal to expected.
    GreaterOrEqual,
}

impl Relation {
    /// Returns whether the comparison holds for `actual` and `expected`.
    pub(crate) fn holds(self, actual: u64, expected: u64) -> bool {
        match self {
            Self::Equal => actual == expected,
            Self::NotEqual => actual != expected,
            Self::Less => actual < expected,
            Self::LessOrEqual => actual <= expected,
            Self::Greater => actual > expected,
            Self::GreaterOrEqual => actual >= expected,
        }
    }
}

/// One side of a breakpoint condition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConditionOperand {
    /// A register name that must be an exact key in
    /// [`MachineInspection::registers`](chronofork_backend_api::MachineInspection::registers).
    Register(String),
    /// A big-endian memory comparison over `1..=8` bytes.
    Memory {
        /// First compared address.
        address: u32,
        /// Compared byte count, in `1..=8`.
        length: u8,
    },
}

/// A register or bounded memory comparison attached to a breakpoint.
///
/// Location and condition are ANDed: the breakpoint hits only when both hold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Condition {
    /// The compared value source.
    pub operand: ConditionOperand,
    /// The comparison operator.
    pub relation: Relation,
    /// The expected value.
    pub value: u64,
}

/// A host breakpoint: a location plus an optional condition.
///
/// The location is tested against the `pc` register before the instruction
/// executes, so a hit describes the state before that instruction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BreakpointSpec {
    /// Program-counter location that must match.
    pub location: Location,
    /// Optional condition that must also hold.
    pub condition: Option<Condition>,
}

/// Interrupt phase selected by an [`EventFilter::Interrupt`] filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptPhase {
    /// The scheduler raised a request.
    Requested,
    /// The CPU entered the handler.
    Delivered,
    /// The scheduler acknowledged the delivered level.
    Acknowledged,
}

/// Selects one class of retained [`DebugEvent`](chronofork_backend_api::DebugEvent).
///
/// A filter with an optional field set to `Some` also requires that field to
/// match. `None` accepts every value of that field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventFilter {
    /// An interrupt request, delivery or acknowledgement.
    Interrupt {
        /// Required 68000 interrupt level, or every level.
        level: Option<u8>,
        /// Required interrupt phase.
        phase: InterruptPhase,
    },
    /// A VDP port read or write.
    Vdp {
        /// Required direction (`true` for a write), or both.
        write: Option<bool>,
        /// Required VDP port offset, or every offset.
        offset: Option<u8>,
    },
    /// A controller data-register read.
    ControllerRead {
        /// Required Mega Drive hardware port number (1 or 2), or both.
        port: Option<u8>,
    },
    /// A work-RAM byte write inside an inclusive range.
    BusWrite {
        /// Required inclusive address range, or every address.
        range: Option<DebugAddressRange>,
    },
}

impl EventFilter {
    /// Returns whether `kind` matches this filter.
    pub(crate) fn matches(self, kind: DebugEventKind) -> bool {
        match (self, kind) {
            (
                Self::Interrupt {
                    level,
                    phase: InterruptPhase::Requested,
                },
                DebugEventKind::InterruptRequested { level: event_level },
            ) => level.is_none_or(|wanted| wanted == event_level),
            (
                Self::Interrupt {
                    level,
                    phase: InterruptPhase::Delivered,
                },
                DebugEventKind::InterruptDelivered { level: event_level },
            ) => level.is_none_or(|wanted| wanted == event_level),
            (
                Self::Interrupt {
                    level,
                    phase: InterruptPhase::Acknowledged,
                },
                DebugEventKind::InterruptAcknowledged { level: event_level },
            ) => level.is_none_or(|wanted| wanted == event_level),
            (
                Self::Vdp { write, offset },
                DebugEventKind::VdpPort {
                    offset: event_offset,
                    write: event_write,
                    ..
                },
            ) => {
                write.is_none_or(|wanted| wanted == event_write)
                    && offset.is_none_or(|wanted| wanted == event_offset)
            }
            (
                Self::ControllerRead { port },
                DebugEventKind::ControllerRead {
                    port: event_port, ..
                },
            ) => port.is_none_or(|wanted| wanted == event_port),
            (Self::BusWrite { range }, DebugEventKind::BusWrite { address, .. }) => {
                range.is_none_or(|wanted| wanted.contains(address))
            }
            _ => false,
        }
    }
}

/// An installed logpoint: a location plus a validated bounded message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Logpoint {
    location: Location,
    message: String,
}

impl Logpoint {
    /// Creates a logpoint from already validated parts.
    pub(crate) fn new(location: Location, message: String) -> Self {
        Self { location, message }
    }

    /// Returns the program-counter location the logpoint matches.
    #[must_use]
    pub const fn location(&self) -> Location {
        self.location
    }

    /// Returns the validated message appended on every hit.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// One retained logpoint hit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogEntry {
    /// Host logpoint that produced this entry.
    pub logpoint_id: LogpointId,
    /// Pre-step tick of the instruction the logpoint matched.
    pub tick: Tick,
    /// Program counter the logpoint matched.
    pub pc: u64,
    /// Number of hits of that logpoint up to and including this one.
    pub hits: u64,
    /// The logpoint's validated message.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::{DebugAddressRange, DebugEventKind, WatchpointAccess};

    use super::{EventFilter, InterruptPhase, Location, Relation, TriggerId, WatchpointId};

    fn range(start: u32, end: u32) -> DebugAddressRange {
        DebugAddressRange::new(start, end).expect("valid inclusive range")
    }

    fn bus_write(address: u32) -> DebugEventKind {
        DebugEventKind::BusWrite {
            address,
            value: 0x12,
            width: 1,
        }
    }

    fn vdp(offset: u8, write: bool) -> DebugEventKind {
        DebugEventKind::VdpPort {
            offset,
            value: 0x1234,
            width: 2,
            write,
        }
    }

    fn interrupt(level: u8, kind: &str) -> DebugEventKind {
        match kind {
            "requested" => DebugEventKind::InterruptRequested { level },
            "delivered" => DebugEventKind::InterruptDelivered { level },
            _ => DebugEventKind::InterruptAcknowledged { level },
        }
    }

    #[test]
    fn relation_holds_every_documented_operator() {
        let cases = [
            (Relation::Equal, 3, 3, true),
            (Relation::Equal, 3, 4, false),
            (Relation::NotEqual, 3, 4, true),
            (Relation::NotEqual, 3, 3, false),
            (Relation::Less, 2, 3, true),
            (Relation::Less, 3, 3, false),
            (Relation::LessOrEqual, 3, 3, true),
            (Relation::LessOrEqual, 4, 3, false),
            (Relation::Greater, 4, 3, true),
            (Relation::Greater, 3, 3, false),
            (Relation::GreaterOrEqual, 3, 3, true),
            (Relation::GreaterOrEqual, 2, 3, false),
        ];
        for (relation, actual, expected, holds) in cases {
            assert_eq!(
                relation.holds(actual, expected),
                holds,
                "{relation:?} {actual} vs {expected}"
            );
        }
    }

    #[test]
    fn location_matches_single_address_and_inclusive_range() {
        assert!(Location::Address(0x206).matches(0x206));
        assert!(!Location::Address(0x206).matches(0x207));

        let range = Location::Range {
            start: 0x206,
            end: 0x20C,
        };
        assert!(!range.matches(0x205));
        assert!(range.matches(0x206));
        assert!(range.matches(0x20C));
        assert!(!range.matches(0x20D));
    }

    #[test]
    fn event_filter_selects_every_documented_class() {
        let requested = EventFilter::Interrupt {
            level: Some(4),
            phase: InterruptPhase::Requested,
        };
        assert!(requested.matches(interrupt(4, "requested")));
        assert!(!requested.matches(interrupt(6, "requested")));
        assert!(!requested.matches(interrupt(4, "delivered")));
        assert!(
            EventFilter::Interrupt {
                level: None,
                phase: InterruptPhase::Delivered,
            }
            .matches(interrupt(6, "delivered"))
        );
        assert!(
            EventFilter::Interrupt {
                level: None,
                phase: InterruptPhase::Acknowledged,
            }
            .matches(interrupt(6, "acknowledged"))
        );

        let vdp_write = EventFilter::Vdp {
            write: Some(true),
            offset: Some(4),
        };
        assert!(vdp_write.matches(vdp(4, true)));
        assert!(!vdp_write.matches(vdp(4, false)));
        assert!(!vdp_write.matches(vdp(0, true)));
        assert!(
            EventFilter::Vdp {
                write: None,
                offset: None,
            }
            .matches(vdp(8, false))
        );

        assert!(EventFilter::ControllerRead { port: Some(1) }.matches(
            DebugEventKind::ControllerRead {
                port: 1,
                value: 0x7E
            }
        ));
        assert!(!EventFilter::ControllerRead { port: Some(1) }.matches(
            DebugEventKind::ControllerRead {
                port: 2,
                value: 0x5F
            }
        ));
        assert!(EventFilter::ControllerRead { port: None }.matches(
            DebugEventKind::ControllerRead {
                port: 2,
                value: 0x5F
            }
        ));

        let write_range = EventFilter::BusWrite {
            range: Some(range(0x00FF_0000, 0x00FF_0001)),
        };
        assert!(write_range.matches(bus_write(0x00FF_0000)));
        assert!(write_range.matches(bus_write(0x00FF_0001)));
        assert!(!write_range.matches(bus_write(0x00FF_0002)));
        assert!(EventFilter::BusWrite { range: None }.matches(bus_write(0x00FF_0004)));

        // A watchpoint event is not selected by any event filter class.
        let watchpoint = DebugEventKind::Watchpoint {
            watchpoint_id: 1,
            access: WatchpointAccess::Write,
            address: 0x00FF_0000,
            value: 0,
            width: 1,
        };
        for filter in [
            requested,
            EventFilter::Vdp {
                write: Some(true),
                offset: Some(4),
            },
            EventFilter::ControllerRead { port: Some(1) },
            write_range,
        ] {
            assert!(!filter.matches(watchpoint));
        }
    }

    #[test]
    fn trigger_ids_keep_per_kind_namespaces_distinct() {
        let watchpoint = WatchpointId::new(1);
        assert_eq!(
            TriggerId::Watchpoint(watchpoint),
            TriggerId::Watchpoint(watchpoint)
        );
        assert_ne!(
            TriggerId::Watchpoint(watchpoint).value(),
            0,
            "trigger ids are one-based"
        );
        assert_eq!(watchpoint.value(), 1);
    }
}
