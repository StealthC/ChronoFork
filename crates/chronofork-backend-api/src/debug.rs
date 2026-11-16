//! Bounded debug observation for backends that can see their own bus.
//!
//! [`Backend::debug_access`](crate::Backend::debug_access) is optional and
//! defaulted to `None`, so a backend that cannot observe emulated bus accesses
//! compiles unchanged and the host reports observation as unavailable. A backend
//! that can observe its bus implements [`DebugAccess`] over one bounded event
//! ring ([`DebugEventLog`]) plus one bounded watchpoint table
//! ([`WatchpointSpec`]).
//!
//! # Non-canonical debug state
//!
//! Everything in this module is debug data. Observation must never enter a
//! snapshot, the state hash, the configuration hash or the payload version, and
//! restoring a snapshot must not rewind the event ring, the watchpoint table or
//! any other observer state. Debugger memory reads and writes must not produce
//! events either: only emulated execution may. See
//! [ADR-0010](../../../docs/adr/0010-debugging-depth.md).
//!
//! # Bounds
//!
//! Event capacity is `1..=MAX_DEBUG_EVENT_CAPACITY` and the watchpoint table
//! holds at most [`MAX_WATCHPOINTS`] specs with nonzero, unique ids. A full ring
//! evicts the oldest event and counts it in [`DebugEventLog::dropped`]; a full
//! table is a typed error. Every rejection uses
//! [`BackendError::InvalidConfiguration`]; there is no silent clipping.

use std::collections::VecDeque;

use chronofork_core::Tick;

use crate::error::BackendError;

/// Event-ring capacity used by a caller that does not choose one.
pub const DEFAULT_DEBUG_EVENT_CAPACITY: usize = 1024;
/// Largest accepted event-ring capacity, in events.
pub const MAX_DEBUG_EVENT_CAPACITY: usize = 65_536;
/// Largest accepted watchpoint table, in specs.
pub const MAX_WATCHPOINTS: usize = 64;

/// Direction of one emulated bus access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchpointAccess {
    /// A read access.
    Read,
    /// A write access.
    Write,
}

/// An inclusive emulated address range `start..=end`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebugAddressRange {
    start: u32,
    end: u32,
}

impl DebugAddressRange {
    /// Creates an inclusive address range.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::InvalidConfiguration`] when `start` is greater
    /// than `end`, because that range contains no address. `start == end` is a
    /// single-address range and is accepted.
    pub fn new(start: u32, end: u32) -> Result<Self, BackendError> {
        if start > end {
            return Err(BackendError::InvalidConfiguration(format!(
                "debug address range start 0x{start:08X} is greater than end 0x{end:08X}"
            )));
        }
        Ok(Self { start, end })
    }

    /// Returns the first address in the range.
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the last address in the range.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }

    /// Returns whether `address` lies inside the inclusive range.
    #[must_use]
    pub const fn contains(self, address: u32) -> bool {
        address >= self.start && address <= self.end
    }
}

/// One installed watchpoint: a nonzero id, an inclusive range and an access
/// direction.
///
/// A watchpoint matches one emulated **byte** access whose direction equals
/// [`WatchpointSpec::access`] and whose address lies inside
/// [`WatchpointSpec::range`]. A backend that serves a wider access as several
/// byte accesses reports one event per matching byte, which is why the host can
/// always map an event back to an exact address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchpointSpec {
    id: u32,
    range: DebugAddressRange,
    access: WatchpointAccess,
}

impl WatchpointSpec {
    /// Creates a watchpoint spec.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::InvalidConfiguration`] when `id` is zero. Zero is
    /// reserved for "no watchpoint" so a reported event id is always a real
    /// installed spec. Duplicate ids are rejected by [`DebugConfig::new`], which
    /// owns the complete table.
    pub fn new(
        id: u32,
        range: DebugAddressRange,
        access: WatchpointAccess,
    ) -> Result<Self, BackendError> {
        if id == 0 {
            return Err(BackendError::InvalidConfiguration(
                "debug watchpoint id 0 is reserved for \"no watchpoint\"".to_owned(),
            ));
        }
        Ok(Self { id, range, access })
    }

    /// Returns the watchpoint id.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.id
    }

    /// Returns the watched inclusive address range.
    #[must_use]
    pub const fn range(self) -> DebugAddressRange {
        self.range
    }

    /// Returns the watched access direction.
    #[must_use]
    pub const fn access(self) -> WatchpointAccess {
        self.access
    }

    /// Returns whether one byte access matches this watchpoint.
    ///
    /// The direction must be equal and the address must lie inside the
    /// inclusive range.
    #[must_use]
    pub const fn matches(self, access: WatchpointAccess, address: u32) -> bool {
        // `==` is not available in a `const fn` for a non-primitive enum, so the
        // two defined directions are compared explicitly.
        let direction_matches = match (self.access, access) {
            (WatchpointAccess::Read, WatchpointAccess::Read)
            | (WatchpointAccess::Write, WatchpointAccess::Write) => true,
            (WatchpointAccess::Read, WatchpointAccess::Write)
            | (WatchpointAccess::Write, WatchpointAccess::Read) => false,
        };
        direction_matches && self.range.contains(address)
    }
}

/// One typed observation produced by emulated execution.
///
/// Interrupt events name the 68000 interrupt level. Bus and device events carry
/// the data width of the access that produced them. A [`DebugEventKind::Watchpoint`]
/// is always one byte access, so its `width` is `1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugEventKind {
    /// A scheduler interrupt source raised a request for `level`.
    InterruptRequested {
        /// 68000 interrupt level.
        level: u8,
    },
    /// The CPU entered the handler for `level`.
    InterruptDelivered {
        /// 68000 interrupt level.
        level: u8,
    },
    /// The scheduler acknowledged exactly the delivered `level`.
    InterruptAcknowledged {
        /// 68000 interrupt level.
        level: u8,
    },
    /// A work-RAM byte write performed by emulated execution.
    BusWrite {
        /// Written bus address.
        address: u32,
        /// Written byte value.
        value: u32,
        /// Access width in bytes.
        width: u8,
    },
    /// A VDP port access performed by emulated execution.
    VdpPort {
        /// Port offset relative to the VDP base address.
        offset: u8,
        /// Accessed value (byte or word, according to `width`).
        value: u32,
        /// Access width in bytes.
        width: u8,
        /// `true` for a write, `false` for a read.
        write: bool,
    },
    /// A controller data-register read performed by emulated execution.
    ControllerRead {
        /// Mega Drive controller port number (1 or 2).
        port: u8,
        /// Data-register byte returned by the read.
        value: u8,
    },
    /// A byte access matched an installed watchpoint.
    Watchpoint {
        /// Id of the matching [`WatchpointSpec`].
        watchpoint_id: u32,
        /// Access direction that matched.
        access: WatchpointAccess,
        /// Accessed byte address.
        address: u32,
        /// Byte value read or written.
        value: u32,
        /// Access width in bytes; always `1` because watchpoints are byte
        /// granular.
        width: u8,
    },
}

/// One observed event with the emulated tick it belongs to.
///
/// A backend stamps the tick that was current **before** the instruction that
/// produced the event, so an event explains the state transition it caused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebugEvent {
    /// Emulated tick at the start of the step that produced the event.
    pub tick: Tick,
    /// The observation itself.
    pub kind: DebugEventKind,
}

/// A bounded ring of [`DebugEvent`]s with a dropped counter.
///
/// The ring retains at most [`DebugEventLog::capacity`] events. Recording into a
/// full ring evicts the oldest retained event and increments
/// [`DebugEventLog::dropped`], so memory is bounded by the configured capacity
/// and no event is silently replaced without being counted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugEventLog {
    capacity: usize,
    entries: VecDeque<DebugEvent>,
    next_index: u64,
    dropped: u64,
}

impl DebugEventLog {
    /// Creates an empty ring with a bounded capacity in events.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::InvalidConfiguration`] when `capacity` is zero or
    /// exceeds [`MAX_DEBUG_EVENT_CAPACITY`].
    pub fn new(capacity: usize) -> Result<Self, BackendError> {
        if capacity == 0 || capacity > MAX_DEBUG_EVENT_CAPACITY {
            return Err(BackendError::InvalidConfiguration(format!(
                "debug event capacity must be in 1..={MAX_DEBUG_EVENT_CAPACITY}, got {capacity}"
            )));
        }
        Ok(Self {
            capacity,
            entries: VecDeque::new(),
            next_index: 0,
            dropped: 0,
        })
    }

    /// Returns the configured ring capacity in events.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of retained events, which never exceeds the capacity.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no event is retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns how many of the oldest events were evicted after the ring filled.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Returns the absolute index the next recorded event will receive.
    ///
    /// Indexes start at zero after [`DebugEventLog::new`] or
    /// [`DebugEventLog::clear`] and saturate at `u64::MAX` (unreachable in
    /// practice) rather than wrapping.
    #[must_use]
    pub const fn next_index(&self) -> u64 {
        self.next_index
    }

    /// Returns the absolute index of the oldest retained event, or `None` when
    /// the ring is empty.
    ///
    /// For a non-saturated log this is the number of evicted events, so
    /// `first_index + len == next_index` and the retained indexes stay
    /// contiguous. A non-zero value proves the ring overflowed.
    #[must_use]
    pub fn first_index(&self) -> Option<u64> {
        if self.entries.is_empty() {
            None
        } else {
            Some(self.dropped)
        }
    }

    /// Iterates the retained events from oldest to newest.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = DebugEvent> + ExactSizeIterator + '_ {
        self.entries.iter().copied()
    }

    /// Returns the retained event at `position` (`0` is the oldest), or `None`.
    #[must_use]
    pub fn get(&self, position: usize) -> Option<DebugEvent> {
        self.entries.get(position).copied()
    }

    /// Appends one event, evicting the oldest retained event when the ring is
    /// full.
    pub fn record(&mut self, event: DebugEvent) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.entries.push_back(event);
        self.next_index = self.next_index.saturating_add(1);
    }

    /// Discards every retained event and restarts indexes at zero, keeping the
    /// capacity.
    ///
    /// This also clears [`DebugEventLog::dropped`] and
    /// [`DebugEventLog::next_index`].
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_index = 0;
        self.dropped = 0;
    }
}

/// A validated observation configuration: one event capacity plus a watchpoint
/// table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugConfig {
    event_capacity: usize,
    watchpoints: Vec<WatchpointSpec>,
}

impl DebugConfig {
    /// Creates a validated observation configuration.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::InvalidConfiguration`] when `event_capacity` is
    /// outside `1..=MAX_DEBUG_EVENT_CAPACITY`, when more than
    /// [`MAX_WATCHPOINTS`] specs are supplied, or when two specs share an id.
    pub fn new(
        event_capacity: usize,
        watchpoints: Vec<WatchpointSpec>,
    ) -> Result<Self, BackendError> {
        if event_capacity == 0 || event_capacity > MAX_DEBUG_EVENT_CAPACITY {
            return Err(BackendError::InvalidConfiguration(format!(
                "debug event capacity must be in 1..={MAX_DEBUG_EVENT_CAPACITY}, got \
                 {event_capacity}"
            )));
        }
        if watchpoints.len() > MAX_WATCHPOINTS {
            return Err(BackendError::InvalidConfiguration(format!(
                "at most {MAX_WATCHPOINTS} debug watchpoints are supported, got {}",
                watchpoints.len()
            )));
        }
        for (index, spec) in watchpoints.iter().enumerate() {
            if spec.id() == 0 {
                return Err(BackendError::InvalidConfiguration(
                    "debug watchpoint id 0 is reserved for \"no watchpoint\"".to_owned(),
                ));
            }
            if watchpoints[..index]
                .iter()
                .any(|other| other.id() == spec.id())
            {
                return Err(BackendError::InvalidConfiguration(format!(
                    "duplicate debug watchpoint id {}",
                    spec.id()
                )));
            }
        }
        Ok(Self {
            event_capacity,
            watchpoints,
        })
    }

    /// Returns the configured event-ring capacity.
    #[must_use]
    pub const fn event_capacity(&self) -> usize {
        self.event_capacity
    }

    /// Returns the configured watchpoint table.
    #[must_use]
    pub fn watchpoints(&self) -> &[WatchpointSpec] {
        &self.watchpoints
    }
}

/// Optional, bounded observation implemented by a backend that owns its bus.
///
/// The trait is exposed through [`Backend::debug_access`](crate::Backend::debug_access)
/// and is absent from backends that cannot observe emulated bus accesses.
pub trait DebugAccess {
    /// Installs a validated observation configuration, discarding any previously
    /// retained events.
    ///
    /// # Errors
    ///
    /// Returns a typed [`BackendError`] when the configuration cannot be
    /// installed. No canonical machine state is read or written either way.
    fn configure_debug(&mut self, config: DebugConfig) -> Result<(), BackendError>;

    /// Returns the installed event ring, or `None` while observation is off or
    /// after [`DebugAccess::take_debug_event_log`].
    fn debug_event_log(&self) -> Option<&DebugEventLog>;

    /// Removes and returns the installed event ring, disabling further
    /// recording until observation is configured again.
    fn take_debug_event_log(&mut self) -> Option<DebugEventLog>;
}

#[cfg(test)]
mod tests {
    use chronofork_core::Tick;

    use super::{
        DEFAULT_DEBUG_EVENT_CAPACITY, DebugAddressRange, DebugConfig, DebugEvent, DebugEventKind,
        DebugEventLog, MAX_DEBUG_EVENT_CAPACITY, MAX_WATCHPOINTS, WatchpointAccess, WatchpointSpec,
    };
    use crate::error::BackendError;

    fn range(start: u32, end: u32) -> DebugAddressRange {
        DebugAddressRange::new(start, end).expect("valid range")
    }

    fn spec(id: u32, start: u32, end: u32, access: WatchpointAccess) -> WatchpointSpec {
        WatchpointSpec::new(id, range(start, end), access).expect("valid spec")
    }

    fn event(tick: u64, kind: DebugEventKind) -> DebugEvent {
        DebugEvent {
            tick: Tick::from_raw(tick),
            kind,
        }
    }

    fn bus_write(address: u32) -> DebugEventKind {
        DebugEventKind::BusWrite {
            address,
            value: 0x5A,
            width: 1,
        }
    }

    #[test]
    fn documented_capacities_and_limits_are_stable() {
        assert_eq!(DEFAULT_DEBUG_EVENT_CAPACITY, 1024);
        assert_eq!(MAX_DEBUG_EVENT_CAPACITY, 65_536);
        assert_eq!(MAX_WATCHPOINTS, 64);
    }

    #[test]
    fn event_log_rejects_zero_and_oversized_capacity() {
        for capacity in [0, MAX_DEBUG_EVENT_CAPACITY + 1] {
            let error = DebugEventLog::new(capacity).expect_err("capacity must be rejected");
            assert!(
                matches!(error, BackendError::InvalidConfiguration(_)),
                "capacity {capacity} must be a typed configuration error, got {error}"
            );
        }
        // Both bounds are inclusive.
        assert_eq!(DebugEventLog::new(1).expect("minimum").capacity(), 1);
        assert_eq!(
            DebugEventLog::new(MAX_DEBUG_EVENT_CAPACITY)
                .expect("maximum")
                .capacity(),
            MAX_DEBUG_EVENT_CAPACITY
        );
    }

    #[test]
    fn event_log_evicts_oldest_and_keeps_contiguous_indexes() {
        let mut log = DebugEventLog::new(2).expect("capacity 2");
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.first_index(), None);
        assert_eq!(log.dropped(), 0);
        assert_eq!(log.next_index(), 0);

        for tick in 0..4_u64 {
            log.record(event(tick, bus_write(0x00FF_0000)));
        }
        assert_eq!(log.capacity(), 2);
        assert_eq!(log.len(), 2);
        assert!(!log.is_empty());
        assert_eq!(log.dropped(), 2);
        assert_eq!(log.first_index(), Some(2));
        assert_eq!(log.next_index(), 4);
        assert_eq!(
            log.first_index().expect("retained") + log.len() as u64,
            log.next_index(),
            "retained indexes must stay contiguous"
        );
        assert_eq!(
            log.entries().collect::<Vec<_>>(),
            vec![
                event(2, bus_write(0x00FF_0000)),
                event(3, bus_write(0x00FF_0000))
            ]
        );
        assert_eq!(log.get(0), Some(event(2, bus_write(0x00FF_0000))));
        assert_eq!(log.get(1), Some(event(3, bus_write(0x00FF_0000))));
        assert_eq!(log.get(2), None);
    }

    #[test]
    fn event_log_entries_are_double_ended_and_exact_size() {
        let mut log = DebugEventLog::new(4).expect("capacity 4");
        for tick in 0..3_u64 {
            log.record(event(
                tick,
                bus_write(0x200 + u32::try_from(tick).expect("tick")),
            ));
        }
        let newest_first: Vec<DebugEvent> = log.entries().rev().collect();
        assert_eq!(
            newest_first,
            vec![
                event(2, bus_write(0x202)),
                event(1, bus_write(0x201)),
                event(0, bus_write(0x200))
            ]
        );
        assert_eq!(log.entries().len(), 3);
        assert_eq!(log.entries().count(), 3);
    }

    #[test]
    fn event_log_clear_restarts_indexes_and_keeps_capacity() {
        let mut log = DebugEventLog::new(2).expect("capacity 2");
        for tick in 0..4_u64 {
            log.record(event(tick, bus_write(0x00FF_0000)));
        }
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.capacity(), 2);
        assert_eq!(log.dropped(), 0);
        assert_eq!(log.first_index(), None);
        assert_eq!(log.next_index(), 0);

        log.record(event(7, bus_write(0x00FF_0004)));
        assert_eq!(log.dropped(), 0);
        assert_eq!(log.first_index(), Some(0));
        assert_eq!(log.next_index(), 1);
        assert_eq!(log.get(0), Some(event(7, bus_write(0x00FF_0004))));
    }

    #[test]
    fn address_range_is_inclusive_and_rejects_inverted_bounds() {
        let range = DebugAddressRange::new(0x00FF_0000, 0x00FF_0002).expect("valid range");
        assert_eq!(range.start(), 0x00FF_0000);
        assert_eq!(range.end(), 0x00FF_0002);
        assert!(range.contains(0x00FF_0000));
        assert!(range.contains(0x00FF_0001));
        assert!(range.contains(0x00FF_0002));
        assert!(!range.contains(0x00FE_FFFF));
        assert!(!range.contains(0x00FF_0003));

        assert_eq!(
            DebugAddressRange::new(0x00FF_0001, 0x00FF_0000),
            Err(BackendError::InvalidConfiguration(
                "debug address range start 0x00FF0001 is greater than end 0x00FF0000".to_owned()
            ))
        );
        // A single-address range is valid.
        assert!(DebugAddressRange::new(0x200, 0x200).is_ok());
    }

    #[test]
    fn watchpoint_requires_a_nonzero_id_and_matches_exactly() {
        assert_eq!(
            WatchpointSpec::new(0, range(0x200, 0x201), WatchpointAccess::Read),
            Err(BackendError::InvalidConfiguration(
                "debug watchpoint id 0 is reserved for \"no watchpoint\"".to_owned()
            ))
        );

        let read = spec(7, 0x200, 0x201, WatchpointAccess::Read);
        assert_eq!(read.id(), 7);
        assert_eq!(read.range(), range(0x200, 0x201));
        assert_eq!(read.access(), WatchpointAccess::Read);
        assert!(read.matches(WatchpointAccess::Read, 0x200));
        assert!(read.matches(WatchpointAccess::Read, 0x201));
        assert!(!read.matches(WatchpointAccess::Read, 0x1FF));
        assert!(!read.matches(WatchpointAccess::Read, 0x202));
        assert!(
            !read.matches(WatchpointAccess::Write, 0x200),
            "an access-direction mismatch must not fire"
        );

        let write = spec(8, 0x200, 0x200, WatchpointAccess::Write);
        assert!(write.matches(WatchpointAccess::Write, 0x200));
        assert!(!write.matches(WatchpointAccess::Read, 0x200));
    }

    #[test]
    fn debug_config_rejects_duplicate_ids_and_too_many_watchpoints() {
        let duplicate = DebugConfig::new(
            8,
            vec![
                spec(1, 0x200, 0x201, WatchpointAccess::Read),
                spec(1, 0x300, 0x301, WatchpointAccess::Write),
            ],
        );
        assert_eq!(
            duplicate,
            Err(BackendError::InvalidConfiguration(
                "duplicate debug watchpoint id 1".to_owned()
            ))
        );

        let too_many: Vec<WatchpointSpec> = (1..=MAX_WATCHPOINTS + 1)
            .map(|id| {
                let id = u32::try_from(id).expect("fits");
                spec(id, 0x200, 0x201, WatchpointAccess::Read)
            })
            .collect();
        let error = DebugConfig::new(8, too_many).expect_err("table is bounded");
        assert!(
            matches!(
                &error,
                BackendError::InvalidConfiguration(message)
                    if message.contains("at most 64 debug watchpoints")
            ),
            "an oversized table must be a typed error, got {error}"
        );

        for capacity in [0, MAX_DEBUG_EVENT_CAPACITY + 1] {
            assert!(matches!(
                DebugConfig::new(capacity, Vec::new()),
                Err(BackendError::InvalidConfiguration(_))
            ));
        }
    }

    #[test]
    fn debug_config_accepts_the_maximum_watchpoint_count() {
        let watchpoints: Vec<WatchpointSpec> = (1..=MAX_WATCHPOINTS)
            .map(|id| {
                let id = u32::try_from(id).expect("fits");
                let start = 0x200 + id * 2;
                spec(id, start, start + 1, WatchpointAccess::Write)
            })
            .collect();
        let config = DebugConfig::new(DEFAULT_DEBUG_EVENT_CAPACITY, watchpoints)
            .expect("maximum table is accepted");
        assert_eq!(config.event_capacity(), DEFAULT_DEBUG_EVENT_CAPACITY);
        assert_eq!(config.watchpoints().len(), MAX_WATCHPOINTS);
        assert_eq!(config.watchpoints()[0].id(), 1);
        assert_eq!(config.watchpoints()[MAX_WATCHPOINTS - 1].id(), 64);
    }

    #[test]
    fn event_and_kind_fields_round_trip() {
        let kind = DebugEventKind::Watchpoint {
            watchpoint_id: 3,
            access: WatchpointAccess::Write,
            address: 0x00FF_0001,
            value: 0x34,
            width: 1,
        };
        let event = event(12, kind);
        assert_eq!(event.tick, Tick::from_raw(12));
        assert_eq!(event.kind, kind);

        let mut log = DebugEventLog::new(2).expect("capacity 2");
        log.record(event);
        assert_eq!(log.get(0), Some(event));
    }
}
