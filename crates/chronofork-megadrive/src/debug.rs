//! Debug-only bus and interrupt observation for the minimal Mega Drive slice.
//!
//! The observer is bounded, deterministic and **non-canonical**. It is absent
//! from the backend's serialized state body, state hash, configuration hash and
//! payload version, and `restore` does not rewind it, exactly like the
//! instruction trace in [`crate::trace`]. Observation only reads the bus,
//! device and scheduler state the machine already owns, so it cannot change
//! emulated behavior.
//!
//! # Event sources
//!
//! - One [`DebugEventKind::Watchpoint`] per matching byte lane of every bus
//!   access, read or write, on ROM, work RAM, the VDP port window and the two
//!   mapped controller data registers. Instruction fetches are ordinary read
//!   accesses and therefore match a watchpoint like any other read.
//! - One [`DebugEventKind::BusWrite`] per work-RAM byte write. VDP writes are
//!   reported as [`DebugEventKind::VdpPort`] events instead, never duplicated.
//! - One [`DebugEventKind::VdpPort`] per VDP port access, carrying the port
//!   offset, the accessed value, the access width and its direction.
//! - One [`DebugEventKind::ControllerRead`] per controller data-register read.
//! - [`DebugEventKind::InterruptRequested`] when a scheduler per-level issued
//!   counter advances without a matching coalesced increment, so a request that
//!   merges into an already-pending request of the same level does not produce a
//!   duplicate event.
//! - [`DebugEventKind::InterruptDelivered`] after a successful CPU interrupt
//!   entry and [`DebugEventKind::InterruptAcknowledged`] after the scheduler
//!   acknowledges exactly the delivered level.
//!
//! Every event created during a step carries the tick that was current before
//! that step, so an event explains the transition it caused.
//!
//! The debugger-style [`crate::MegadriveBus::read_memory`] and
//! [`crate::MegadriveBus::write_memory`] paths bypass this observer: they are
//! not emulated bus accesses and must never fabricate an event.

use chronofork_backend_api::{
    BackendError, DebugConfig, DebugEvent, DebugEventKind, DebugEventLog, WatchpointAccess,
    WatchpointSpec,
};
use chronofork_core::Tick;

use crate::interrupts::{HBLANK_INTERRUPT_LEVEL, VBLANK_INTERRUPT_LEVEL};

/// Interrupt levels observed by the backend, in [`crate::InterruptLevel::ALL`]
/// order.
///
/// The position in this array is the index used for the per-level scheduler
/// counters, so request detection walks the same order the scheduler reports.
const OBSERVED_LEVELS: [u8; 2] = [HBLANK_INTERRUPT_LEVEL, VBLANK_INTERRUPT_LEVEL];

/// Debug-only observation state for one bus.
///
/// The event ring is `None` until observation is configured, which keeps a
/// machine without a debugger free of event storage and recording cost. The
/// watchpoint table is empty in the same state.
#[derive(Clone, Debug, Default)]
pub(crate) struct DebugObserver {
    log: Option<DebugEventLog>,
    watchpoints: Vec<WatchpointSpec>,
    tick: Tick,
    issued: [u64; 2],
    coalesced: [u64; 2],
}

impl DebugObserver {
    /// Replaces the event ring and watchpoint table, discarding retained events.
    ///
    /// [`DebugConfig`] validates its capacity, so every valid configuration
    /// produces a ring. The result is propagated instead of unwrapped so a
    /// rejected configuration can never panic the machine.
    pub(crate) fn configure(&mut self, config: DebugConfig) -> Result<(), BackendError> {
        let log = DebugEventLog::new(config.event_capacity())?;
        self.log = Some(log);
        self.watchpoints = config.watchpoints().to_vec();
        Ok(())
    }

    /// Returns the installed event ring, or `None` while observation is off.
    pub(crate) fn event_log(&self) -> Option<&DebugEventLog> {
        self.log.as_ref()
    }

    /// Removes the event ring, disabling further recording.
    pub(crate) fn take_event_log(&mut self) -> Option<DebugEventLog> {
        self.log.take()
    }

    /// Records the pre-step tick that every event of the next step carries.
    pub(crate) fn begin_step(&mut self, tick: Tick) {
        self.tick = tick;
    }

    /// Appends one event at the current pre-step tick, if observation is on.
    pub(crate) fn record(&mut self, kind: DebugEventKind) {
        if let Some(log) = &mut self.log {
            log.record(DebugEvent {
                tick: self.tick,
                kind,
            });
        }
    }

    /// Records one watchpoint event per matching byte lane of `width` bytes.
    ///
    /// `value` holds the access value: for a width-1 access only its least
    /// significant byte is on the bus, for width 2 and 4 the byte lanes are the
    /// big-endian bytes of `value`. Lanes are reported in ascending address
    /// order, and a lane that matches no watchpoint is silent.
    pub(crate) fn observe_watchpoints(
        &mut self,
        access: WatchpointAccess,
        address: u32,
        value: u32,
        width: u8,
    ) {
        let lanes = match width {
            1 | 2 | 4 => usize::from(width),
            _ => return,
        };
        if self.watchpoints.is_empty() {
            return;
        }
        let tick = self.tick;
        let watchpoints = &self.watchpoints;
        let Some(log) = &mut self.log else {
            return;
        };
        let bytes = value.to_be_bytes();
        let first_lane = bytes.len() - lanes;
        for lane in 0..lanes {
            let Ok(lane_offset) = u32::try_from(lane) else {
                continue;
            };
            let Some(lane_address) = address.checked_add(lane_offset) else {
                continue;
            };
            let lane_value = u32::from(bytes[first_lane + lane]);
            for spec in watchpoints {
                if !spec.matches(access, lane_address) {
                    continue;
                }
                log.record(DebugEvent {
                    tick,
                    kind: DebugEventKind::Watchpoint {
                        watchpoint_id: spec.id(),
                        access,
                        address: lane_address,
                        value: lane_value,
                        width: 1,
                    },
                });
            }
        }
    }

    /// Records an `InterruptRequested` event for every level whose scheduler
    /// issued counter advanced since the previous observation.
    ///
    /// A scheduler request that coalesced into an already-pending request of the
    /// same level also advances that level's coalesced counter; it is not a new
    /// request and produces no event. Both counter pairs are then remembered so
    /// the next step compares against the current state.
    pub(crate) fn observe_scheduler(&mut self, issued: [u64; 2], coalesced: [u64; 2]) {
        for (index, level) in OBSERVED_LEVELS.into_iter().enumerate() {
            if issued[index] != self.issued[index] && coalesced[index] == self.coalesced[index] {
                self.record(DebugEventKind::InterruptRequested { level });
            }
        }
        self.issued = issued;
        self.coalesced = coalesced;
    }

    /// Rebases the request-detection counters without recording anything.
    ///
    /// Used when observation is installed and after a snapshot restore, so
    /// requests that were raised before the baseline are never reported as new
    /// events.
    pub(crate) fn sync_scheduler(&mut self, issued: [u64; 2], coalesced: [u64; 2]) {
        self.issued = issued;
        self.coalesced = coalesced;
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::{
        DebugAddressRange, DebugConfig, DebugEventKind, DebugEventLog, WatchpointAccess,
        WatchpointSpec,
    };
    use chronofork_core::Tick;

    use super::{DebugObserver, OBSERVED_LEVELS};

    fn spec(id: u32, start: u32, end: u32, access: WatchpointAccess) -> WatchpointSpec {
        WatchpointSpec::new(
            id,
            DebugAddressRange::new(start, end).expect("range"),
            access,
        )
        .expect("spec")
    }

    fn configured(watchpoints: Vec<WatchpointSpec>) -> DebugObserver {
        let mut observer = DebugObserver::default();
        let config = DebugConfig::new(16, watchpoints).expect("config");
        observer.configure(config).expect("configure");
        observer.begin_step(Tick::from_raw(5));
        observer
    }

    fn events(observer: &DebugObserver) -> Vec<DebugEventKind> {
        observer
            .event_log()
            .expect("configured")
            .entries()
            .map(|event| event.kind)
            .collect()
    }

    #[test]
    fn observation_is_off_until_configured() {
        let mut observer = DebugObserver::default();
        observer.begin_step(Tick::from_raw(3));
        observer.record(DebugEventKind::InterruptDelivered { level: 4 });
        observer.observe_watchpoints(WatchpointAccess::Read, 0x200, 0, 1);
        observer.observe_scheduler([1, 1], [0, 0]);
        assert!(observer.event_log().is_none());
        assert!(observer.take_event_log().is_none());

        let mut observer = configured(Vec::new());
        observer.record(DebugEventKind::InterruptDelivered { level: 4 });
        assert_eq!(
            observer.event_log().expect("configured").entries().count(),
            1
        );
        let taken = observer.take_event_log().expect("log");
        assert_eq!(taken.len(), 1);
        assert!(observer.event_log().is_none());
    }

    #[test]
    fn watchpoints_report_each_matching_byte_lane_in_address_order() {
        let mut observer = configured(vec![
            spec(1, 0x00FF_0000, 0x00FF_0000, WatchpointAccess::Write),
            spec(2, 0x00FF_0001, 0x00FF_0001, WatchpointAccess::Write),
            spec(3, 0x00FF_0000, 0x00FF_0001, WatchpointAccess::Read),
        ]);
        observer.begin_step(Tick::from_raw(9));
        // A word write fires only the two write specs, one byte each.
        observer.observe_watchpoints(WatchpointAccess::Write, 0x00FF_0000, 0x1234, 2);
        // A word read fires the read spec twice; the write specs are silent.
        observer.observe_watchpoints(WatchpointAccess::Read, 0x00FF_0000, 0x1234, 2);
        let log = observer.event_log().expect("log");
        let entries: Vec<_> = log.entries().collect();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].tick, Tick::from_raw(9));
        assert_eq!(
            entries[0].kind,
            DebugEventKind::Watchpoint {
                watchpoint_id: 1,
                access: WatchpointAccess::Write,
                address: 0x00FF_0000,
                value: 0x12,
                width: 1,
            }
        );
        assert_eq!(
            entries[1].kind,
            DebugEventKind::Watchpoint {
                watchpoint_id: 2,
                access: WatchpointAccess::Write,
                address: 0x00FF_0001,
                value: 0x34,
                width: 1,
            }
        );
        assert_eq!(
            entries[2].kind,
            DebugEventKind::Watchpoint {
                watchpoint_id: 3,
                access: WatchpointAccess::Read,
                address: 0x00FF_0000,
                value: 0x12,
                width: 1,
            }
        );
        assert_eq!(
            entries[3].kind,
            DebugEventKind::Watchpoint {
                watchpoint_id: 3,
                access: WatchpointAccess::Read,
                address: 0x00FF_0001,
                value: 0x34,
                width: 1,
            }
        );

        // A byte access uses only the least-significant lane of the value.
        observer.observe_watchpoints(WatchpointAccess::Write, 0x00FF_0001, 0x5A, 1);
        assert_eq!(
            observer
                .event_log()
                .expect("log")
                .get(4)
                .map(|event| event.kind),
            Some(DebugEventKind::Watchpoint {
                watchpoint_id: 2,
                access: WatchpointAccess::Write,
                address: 0x00FF_0001,
                value: 0x5A,
                width: 1,
            })
        );
    }

    #[test]
    fn coalesced_requests_do_not_duplicate_request_events() {
        let mut observer = configured(Vec::new());
        // First request on both levels: one event each, in level order.
        observer.observe_scheduler([1, 1], [0, 0]);
        // A later request of each level coalesces into the pending request.
        observer.observe_scheduler([2, 2], [1, 1]);
        // A delivery acknowledge leaves the counters unchanged.
        observer.observe_scheduler([2, 2], [1, 1]);
        // A new HBlank after the pending one was acknowledged is a new request.
        observer.observe_scheduler([3, 2], [1, 1]);
        assert_eq!(
            events(&observer),
            vec![
                DebugEventKind::InterruptRequested {
                    level: OBSERVED_LEVELS[0]
                },
                DebugEventKind::InterruptRequested {
                    level: OBSERVED_LEVELS[1]
                },
                DebugEventKind::InterruptRequested {
                    level: OBSERVED_LEVELS[0]
                },
            ]
        );
    }

    #[test]
    fn sync_rebases_counters_without_recording_events() {
        let mut observer = configured(Vec::new());
        observer.observe_scheduler([7, 8], [0, 0]);
        assert_eq!(events(&observer).len(), 2);

        observer.sync_scheduler([9, 9], [2, 3]);
        observer.observe_scheduler([9, 9], [2, 3]);
        assert_eq!(
            observer.event_log().expect("log").len(),
            2,
            "a rebase must not replay suppressed requests"
        );

        observer.observe_scheduler([10, 9], [2, 3]);
        assert_eq!(
            observer.event_log().expect("log").len(),
            3,
            "the next genuinely new request is still reported"
        );
    }

    #[test]
    fn take_event_log_stops_recording_until_reconfigured() {
        let mut observer = configured(Vec::new());
        observer.record(DebugEventKind::InterruptDelivered { level: 6 });
        assert!(observer.take_event_log().is_some());
        observer.record(DebugEventKind::InterruptDelivered { level: 6 });
        assert!(observer.event_log().is_none());

        let config = DebugConfig::new(4, Vec::new()).expect("config");
        observer.configure(config).expect("reconfigure");
        observer.begin_step(Tick::from_raw(2));
        observer.record(DebugEventKind::InterruptDelivered { level: 6 });
        let log: DebugEventLog = observer.take_event_log().expect("reconfigured log");
        assert_eq!(log.len(), 1);
        assert_eq!(log.next_index(), 1);
    }
}
