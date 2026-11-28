//! Host capture driver over any [`Backend`].
//!
//! Capture is a host feature. It observes a backend through inspection, memory
//! reads and the optional [`chronofork_backend_api::DebugAccess`] event ring; it never writes canonical
//! machine state and never changes a snapshot, state hash, configuration hash or
//! payload version. The backend capability `tracing` therefore stays `false`
//! for every backend.

use std::collections::VecDeque;

use chronofork_backend_api::{
    Backend, BackendError, DebugConfig, DebugEvent, DebugEventKind, MachineInspection,
};
use chronofork_core::{InputFrame, Tick};

use crate::error::TraceError;
use crate::meta::{TraceMeta, TraceStopReason};
use crate::record::{InterruptPhase, TraceKind, TraceKindMask, TraceRecord};
use crate::trace::Trace;
use crate::{
    DEFAULT_EVENT_CAPACITY, DEFAULT_TRACE_CAPACITY, MAX_BACKEND_ID_BYTES,
    MAX_BACKEND_VERSION_BYTES, MAX_CAPTURE_STEPS, MAX_EVENT_CAPACITY, MAX_TRACE_CAPACITY,
};

/// Name of the required program-counter register in a backend inspection.
pub const PROGRAM_COUNTER_REGISTER: &str = "pc";

/// A validated capture configuration.
///
/// [`TraceConfig::new`] supplies the documented defaults: an `Immediate` start,
/// no stop trigger, all record kinds, [`DEFAULT_EVENT_CAPACITY`] and
/// [`InputFrame::NEUTRAL`]. Every builder validates its own bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceConfig {
    /// Ring capacity, `1..=`[`MAX_TRACE_CAPACITY`].
    pub capacity: usize,
    /// Start boundary predicate.
    pub start: TraceStart,
    /// Stop boundary predicate.
    pub stop: TraceStop,
    /// Record kinds to retain.
    pub kinds: TraceKindMask,
    /// Backend event-ring capacity, `1..=`[`MAX_EVENT_CAPACITY`].
    pub event_capacity: usize,
    /// Input frame supplied to every executed step.
    pub input: InputFrame,
}

impl TraceConfig {
    /// Creates a configuration with the documented defaults.
    ///
    /// # Errors
    ///
    /// Returns [`TraceError::InvalidConfiguration`] when `capacity` is zero or
    /// above [`MAX_TRACE_CAPACITY`].
    pub fn new(capacity: usize) -> Result<Self, TraceError> {
        validate_capacity(capacity)?;
        Ok(Self {
            capacity,
            start: TraceStart::Immediate,
            stop: TraceStop::None,
            kinds: TraceKindMask::all(),
            event_capacity: DEFAULT_EVENT_CAPACITY,
            input: InputFrame::NEUTRAL,
        })
    }

    /// Sets the start boundary predicate.
    #[must_use]
    pub fn with_start(mut self, start: TraceStart) -> Self {
        self.start = start;
        self
    }

    /// Sets the stop boundary predicate.
    #[must_use]
    pub fn with_stop(mut self, stop: TraceStop) -> Self {
        self.stop = stop;
        self
    }

    /// Sets the enabled record kinds.
    #[must_use]
    pub fn with_kinds(mut self, kinds: TraceKindMask) -> Self {
        self.kinds = kinds;
        self
    }

    /// Sets the backend event-ring capacity.
    ///
    /// # Errors
    ///
    /// Returns [`TraceError::InvalidConfiguration`] when `event_capacity` is
    /// zero or above [`MAX_EVENT_CAPACITY`].
    pub fn with_event_capacity(mut self, event_capacity: usize) -> Result<Self, TraceError> {
        if event_capacity == 0 || event_capacity > MAX_EVENT_CAPACITY {
            return Err(TraceError::InvalidConfiguration(format!(
                "event capacity {event_capacity} is outside 1..={MAX_EVENT_CAPACITY}"
            )));
        }
        self.event_capacity = event_capacity;
        Ok(self)
    }

    /// Sets the input frame supplied to every executed step.
    #[must_use]
    pub fn with_input(mut self, input: InputFrame) -> Self {
        self.input = input;
        self
    }
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_TRACE_CAPACITY,
            start: TraceStart::Immediate,
            stop: TraceStop::None,
            kinds: TraceKindMask::all(),
            event_capacity: DEFAULT_EVENT_CAPACITY,
            input: InputFrame::NEUTRAL,
        }
    }
}

/// When capture begins recording.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceStart {
    /// Record from the first instruction boundary.
    Immediate,
    /// Begin inclusively when the program counter equals this address.
    Pc(u32),
    /// Begin inclusively when the boundary tick is at least this value.
    Tick(Tick),
}

/// When capture stops before executing another instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceStop {
    /// Never stop early.
    None,
    /// Stop before executing the instruction at this program counter.
    Pc(u32),
    /// Stop before the first step whose pre-step tick is at least this value.
    Tick(Tick),
}

/// Captures a bounded structured trace from an existing backend.
///
/// The backend is advanced by the executed steps with [`TraceConfig::input`];
/// no other canonical mutator is called. Instruction records use the `pc`
/// register and the big-endian two-byte word at that address, and the pre-step
/// tick. Bus, VDP, controller and interrupt records come only from
/// [`chronofork_backend_api::DebugAccess`] events; they are recorded only after the start trigger has
/// matched. The stop trigger is evaluated before the start trigger at every
/// boundary and wins when both match.
///
/// `end_tick` is the final observed tick: the boundary tick when no step was
/// executed, otherwise the `StepOutcome` tick of the last executed step. Records
/// are kept in a circular ring of `capacity` entries; `first_index == dropped`.
///
/// # Errors
///
/// Returns a typed [`TraceError`] for invalid steps/capacity/event capacity, a
/// missing or oversized `pc`, a failed opcode read, missing event observation, an
/// event-ring overflow between drains or any wrapped [`BackendError`]. Capture
/// does not panic and never returns a partial trace.
pub fn capture(
    backend: &mut dyn Backend,
    config: &TraceConfig,
    steps: u64,
) -> Result<Trace, TraceError> {
    if steps == 0 || steps > MAX_CAPTURE_STEPS {
        return Err(TraceError::InvalidConfiguration(format!(
            "capture steps {steps} is outside 1..={MAX_CAPTURE_STEPS}"
        )));
    }
    validate_capacity(config.capacity)?;
    if config.event_capacity == 0 || config.event_capacity > MAX_EVENT_CAPACITY {
        return Err(TraceError::InvalidConfiguration(format!(
            "event capacity {} is outside 1..={MAX_EVENT_CAPACITY}",
            config.event_capacity
        )));
    }
    let (backend_id, backend_version) = backend_metadata(backend)?;
    let events_observed = backend.descriptor().capabilities().debug_events;

    let initial = backend.inspect().map_err(TraceError::Backend)?;
    let start_tick = initial.tick;
    let state_hash = backend.state_hash().map_err(TraceError::Backend)?;
    if events_observed {
        let event_capacity = config.event_capacity;
        let debug = backend
            .debug_access()
            .ok_or(TraceError::EventObservationUnavailable)?;
        let debug_config =
            DebugConfig::new(event_capacity, Vec::new()).map_err(TraceError::Backend)?;
        debug
            .configure_debug(debug_config)
            .map_err(TraceError::Backend)?;
    }

    let mut inspection = initial;
    let mut ring = CaptureRing::new(config.capacity);
    let mut executed = 0_u64;
    let mut triggered = false;
    let mut consumed_events = 0_u64;
    let (stop_reason, end_tick) = loop {
        let pc = program_counter(&inspection)?;
        if stop_matches(config.stop, inspection.tick, pc) {
            break (TraceStopReason::StopCondition, inspection.tick);
        }
        if inspection.halted {
            break (TraceStopReason::Halted, inspection.tick);
        }
        if start_matches(config.start, inspection.tick, pc) {
            triggered = true;
        }
        if triggered && config.kinds.contains(TraceKind::Instruction) {
            let opcode = read_opcode(backend, pc)?;
            let index = ring.next_index();
            ring.push(TraceRecord::Instruction {
                index,
                tick: inspection.tick,
                pc,
                opcode,
            });
        }
        let outcome = match backend.step(config.input) {
            Ok(outcome) => outcome,
            Err(BackendError::Halted) => {
                break (TraceStopReason::Halted, inspection.tick);
            }
            Err(error) => return Err(TraceError::Backend(error)),
        };
        executed = executed.checked_add(1).ok_or_else(|| {
            TraceError::InvalidConfiguration("capture step counter overflowed u64".to_owned())
        })?;
        if events_observed {
            drain_events(
                backend,
                config,
                triggered,
                &mut ring,
                &mut consumed_events,
                start_tick,
                outcome.tick,
            )?;
        }
        if outcome.halted {
            break (TraceStopReason::Halted, outcome.tick);
        }
        if executed >= steps {
            break (TraceStopReason::StepLimit, outcome.tick);
        }
        inspection = backend.inspect().map_err(TraceError::Backend)?;
    };

    let (records, dropped) = ring.into_parts();
    let meta = TraceMeta {
        backend: backend_id,
        backend_version,
        state_hash,
        start_tick,
        end_tick,
        capacity: config.capacity,
        first_index: dropped,
        dropped,
        stop_reason,
        triggered,
        events_observed,
    };
    Trace::new(meta, records)
}

/// A bounded circular buffer that keeps the newest records and counts evictions.
struct CaptureRing {
    capacity: usize,
    records: VecDeque<TraceRecord>,
    dropped: u64,
    next_index: u64,
}

impl CaptureRing {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            records: VecDeque::new(),
            dropped: 0,
            next_index: 0,
        }
    }

    const fn next_index(&self) -> u64 {
        self.next_index
    }

    fn push(&mut self, record: TraceRecord) {
        if self.records.len() == self.capacity {
            self.records.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.records.push_back(record);
        self.next_index = self.next_index.saturating_add(1);
    }

    fn into_parts(self) -> (Vec<TraceRecord>, u64) {
        (self.records.into_iter().collect(), self.dropped)
    }
}

/// Reads and validates the backend identity used in trace metadata.
fn backend_metadata(backend: &dyn Backend) -> Result<(String, String), TraceError> {
    let descriptor = backend.descriptor();
    let id = descriptor.id().as_str();
    if id.is_empty() {
        return Err(TraceError::InvalidMetadata(
            "backend identifier must not be empty".to_owned(),
        ));
    }
    if id.len() > MAX_BACKEND_ID_BYTES {
        return Err(TraceError::InvalidMetadata(format!(
            "backend identifier is {} bytes, limit is {MAX_BACKEND_ID_BYTES}",
            id.len()
        )));
    }
    let version = descriptor.version();
    if version.is_empty() {
        return Err(TraceError::InvalidMetadata(
            "backend version must not be empty".to_owned(),
        ));
    }
    if version.len() > MAX_BACKEND_VERSION_BYTES {
        return Err(TraceError::InvalidMetadata(format!(
            "backend version is {} bytes, limit is {MAX_BACKEND_VERSION_BYTES}",
            version.len()
        )));
    }
    Ok((id.to_owned(), version.to_owned()))
}

/// Returns the required program counter as a `u32`.
fn program_counter(inspection: &MachineInspection) -> Result<u32, TraceError> {
    let value = inspection
        .registers()
        .get(PROGRAM_COUNTER_REGISTER)
        .copied()
        .ok_or(TraceError::ProgramCounterUnavailable)?;
    u32::try_from(value).map_err(|_| TraceError::InvalidProgramCounter { value })
}

/// Reads the big-endian two-byte opcode word at `pc`.
fn read_opcode(backend: &dyn Backend, pc: u32) -> Result<u16, TraceError> {
    let bytes =
        backend
            .read_memory(u64::from(pc), 2)
            .map_err(|error| TraceError::OpcodeReadFailed {
                pc,
                message: error.to_string(),
            })?;
    let pair = <[u8; 2]>::try_from(bytes.as_slice()).map_err(|_| TraceError::OpcodeReadFailed {
        pc,
        message: format!(
            "backend returned {} bytes for a 2-byte opcode window",
            bytes.len()
        ),
    })?;
    Ok(u16::from_be_bytes(pair))
}

/// Drains events recorded by the last step and maps the new ones into records.
///
/// Draining happens on every step so a backend ring cannot silently lose events
/// between boundaries. Events are mapped only after the start trigger matched;
/// before that they are consumed and discarded.
fn drain_events(
    backend: &mut dyn Backend,
    config: &TraceConfig,
    triggered: bool,
    ring: &mut CaptureRing,
    consumed: &mut u64,
    start_tick: Tick,
    end_tick: Tick,
) -> Result<(), TraceError> {
    let (next_consumed, events) = {
        let debug = backend
            .debug_access()
            .ok_or(TraceError::EventObservationUnavailable)?;
        let log = debug
            .debug_event_log()
            .ok_or(TraceError::EventObservationUnavailable)?;
        if *consumed < log.dropped() {
            return Err(TraceError::EventRingOverflow);
        }
        let offset = usize::try_from(*consumed - log.dropped())
            .map_err(|_| TraceError::EventRingOverflow)?;
        if offset > log.len() {
            return Err(TraceError::EventRingOverflow);
        }
        let events: Vec<DebugEvent> = log.entries().skip(offset).collect();
        (log.next_index(), events)
    };
    *consumed = next_consumed;
    if !triggered {
        return Ok(());
    }
    for event in events {
        if event.tick < start_tick || event.tick > end_tick {
            return Err(TraceError::InvalidRecordField(format!(
                "backend event tick {} is outside the capture window {start_tick}..={end_tick}",
                event.tick
            )));
        }
        let record = match event.kind {
            DebugEventKind::Watchpoint { .. } => continue,
            DebugEventKind::BusWrite {
                address,
                value,
                width,
            } => {
                if !(1..=4).contains(&width) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "backend bus write width {width} is outside 1..=4"
                    )));
                }
                if !config.kinds.contains(TraceKind::BusWrite) {
                    continue;
                }
                TraceRecord::BusWrite {
                    index: ring.next_index(),
                    tick: event.tick,
                    address,
                    value,
                    width,
                }
            }
            DebugEventKind::VdpPort {
                offset,
                value,
                width,
                write,
            } => {
                if !(1..=2).contains(&width) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "backend VDP port width {width} is outside 1..=2"
                    )));
                }
                if !config.kinds.contains(TraceKind::VdpPort) {
                    continue;
                }
                TraceRecord::VdpPort {
                    index: ring.next_index(),
                    tick: event.tick,
                    offset,
                    value,
                    width,
                    write,
                }
            }
            DebugEventKind::ControllerRead { port, value } => {
                if !(1..=2).contains(&port) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "backend controller port {port} is outside 1..=2"
                    )));
                }
                if !config.kinds.contains(TraceKind::ControllerRead) {
                    continue;
                }
                TraceRecord::ControllerRead {
                    index: ring.next_index(),
                    tick: event.tick,
                    port,
                    value,
                }
            }
            DebugEventKind::InterruptRequested { level }
            | DebugEventKind::InterruptDelivered { level }
            | DebugEventKind::InterruptAcknowledged { level } => {
                if !(1..=7).contains(&level) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "backend interrupt level {level} is outside 1..=7"
                    )));
                }
                if !config.kinds.contains(TraceKind::Interrupt) {
                    continue;
                }
                let phase = match event.kind {
                    DebugEventKind::InterruptRequested { .. } => InterruptPhase::Requested,
                    DebugEventKind::InterruptDelivered { .. } => InterruptPhase::Delivered,
                    DebugEventKind::InterruptAcknowledged { .. } => InterruptPhase::Acknowledged,
                    DebugEventKind::Watchpoint { .. }
                    | DebugEventKind::BusWrite { .. }
                    | DebugEventKind::VdpPort { .. }
                    | DebugEventKind::ControllerRead { .. } => {
                        return Err(TraceError::InvalidRecordField(
                            "interrupt phase arm matched a non-interrupt event".to_owned(),
                        ));
                    }
                };
                TraceRecord::Interrupt {
                    index: ring.next_index(),
                    tick: event.tick,
                    level,
                    phase,
                }
            }
        };
        ring.push(record);
    }
    Ok(())
}

fn validate_capacity(capacity: usize) -> Result<(), TraceError> {
    if capacity == 0 || capacity > MAX_TRACE_CAPACITY {
        return Err(TraceError::InvalidConfiguration(format!(
            "capacity {capacity} is outside 1..={MAX_TRACE_CAPACITY}"
        )));
    }
    Ok(())
}

const fn stop_matches(stop: TraceStop, tick: Tick, pc: u32) -> bool {
    match stop {
        TraceStop::None => false,
        TraceStop::Pc(value) => pc == value,
        TraceStop::Tick(value) => tick.raw() >= value.raw(),
    }
}

const fn start_matches(start: TraceStart, tick: Tick, pc: u32) -> bool {
    match start {
        TraceStart::Immediate => true,
        TraceStart::Pc(value) => pc == value,
        TraceStart::Tick(value) => tick.raw() >= value.raw(),
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::BackendFactory;
    use chronofork_core::{InputFrame, Tick};
    use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

    use super::{TraceConfig, TraceStart, TraceStop, capture};
    use crate::error::TraceError;
    use crate::meta::TraceStopReason;
    use crate::record::{TraceKind, TraceKindMask, TraceRecord};
    use crate::{DEFAULT_EVENT_CAPACITY, DEFAULT_TRACE_CAPACITY, MAX_EVENT_CAPACITY};

    fn factory() -> SyntheticBackendFactory {
        SyntheticBackendFactory::default()
    }

    #[test]
    fn documented_defaults_and_bounds_are_stable() {
        assert_eq!(DEFAULT_TRACE_CAPACITY, 4_096);
        assert_eq!(DEFAULT_EVENT_CAPACITY, 1_024);
        assert_eq!(MAX_EVENT_CAPACITY, 65_536);
        let config = TraceConfig::default();
        assert_eq!(config.capacity, DEFAULT_TRACE_CAPACITY);
        assert_eq!(config.event_capacity, DEFAULT_EVENT_CAPACITY);
        assert_eq!(config.start, TraceStart::Immediate);
        assert_eq!(config.stop, TraceStop::None);
        assert_eq!(config.kinds, TraceKindMask::all());
        assert_eq!(config.input, InputFrame::NEUTRAL);
        assert_eq!(TraceConfig::new(1).expect("minimum").capacity, 1);
    }

    #[test]
    fn configuration_bounds_are_validated() {
        for capacity in [0, crate::MAX_TRACE_CAPACITY + 1] {
            assert!(matches!(
                TraceConfig::new(capacity),
                Err(TraceError::InvalidConfiguration(_))
            ));
        }
        assert!(TraceConfig::new(crate::MAX_TRACE_CAPACITY).is_ok());
        let valid = TraceConfig::new(4).expect("valid");
        for event_capacity in [0, MAX_EVENT_CAPACITY + 1] {
            assert!(matches!(
                valid.clone().with_event_capacity(event_capacity),
                Err(TraceError::InvalidConfiguration(_))
            ));
        }
        assert_eq!(
            valid
                .with_event_capacity(MAX_EVENT_CAPACITY)
                .expect("maximum event capacity")
                .event_capacity,
            MAX_EVENT_CAPACITY
        );
    }

    #[test]
    fn capture_records_instructions_with_pre_step_ticks_and_ring_indexes() {
        let mut backend = factory().create().expect("backend");
        let config = TraceConfig::new(4).expect("config");
        let trace = capture(backend.as_mut(), &config, 6).expect("capture");
        assert_eq!(trace.record_count(), 4);
        assert_eq!(trace.meta().capacity, 4);
        assert_eq!(trace.meta().first_index, 2);
        assert_eq!(trace.meta().dropped, 2);
        assert_eq!(trace.meta().stop_reason, TraceStopReason::StepLimit);
        assert!(trace.meta().triggered);
        assert!(!trace.meta().events_observed);
        assert_eq!(trace.meta().start_tick, Tick::ZERO);
        assert_eq!(trace.meta().end_tick, Tick::from_raw(6));
        assert_eq!(
            trace
                .records()
                .iter()
                .map(TraceRecord::index)
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
        );
        assert!(
            trace
                .records()
                .iter()
                .all(|record| matches!(record, TraceRecord::Instruction { .. }))
        );
    }

    #[test]
    fn invalid_steps_are_rejected_before_inspection() {
        let mut backend = factory().create().expect("backend");
        let config = TraceConfig::default();
        assert!(matches!(
            capture(backend.as_mut(), &config, 0),
            Err(TraceError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            capture(backend.as_mut(), &config, crate::MAX_CAPTURE_STEPS + 1),
            Err(TraceError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn custom_program_and_start_stop_triggers_are_inclusive() {
        let program = SyntheticProgram::new(vec![
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00,
        ])
        .expect("valid program");
        let factory = SyntheticBackendFactory::new(program);

        let mut immediate = factory.create().expect("backend");
        let trace = capture(
            immediate.as_mut(),
            &TraceConfig::new(16).expect("config"),
            4,
        )
        .expect("capture");
        assert!(trace.meta().triggered);
        assert_eq!(trace.records().len(), 4);
        assert_eq!(trace.records()[0].tick(), Tick::ZERO);

        let mut from_pc = factory.create().expect("backend");
        let config = TraceConfig::new(16)
            .expect("config")
            .with_start(TraceStart::Pc(2));
        let trace = capture(from_pc.as_mut(), &config, 6).expect("capture");
        assert!(trace.meta().triggered);
        assert_eq!(trace.records().len(), 4);
        let first = trace.records()[0].summary();
        assert!(first.starts_with("instruction index=0 tick=2"), "{first}");

        let mut from_tick = factory.create().expect("backend");
        let config = TraceConfig::new(16)
            .expect("config")
            .with_start(TraceStart::Tick(Tick::from_raw(2)));
        let trace = capture(from_tick.as_mut(), &config, 6).expect("capture");
        assert_eq!(trace.records()[0].tick(), Tick::from_raw(2));
        assert_eq!(trace.records().len(), 4);

        let mut stop_pc = factory.create().expect("backend");
        let config = TraceConfig::new(16)
            .expect("config")
            .with_stop(TraceStop::Pc(4));
        let trace = capture(stop_pc.as_mut(), &config, 10).expect("capture");
        assert_eq!(trace.meta().stop_reason, TraceStopReason::StopCondition);
        assert_eq!(trace.meta().end_tick, Tick::from_raw(4));
        assert_eq!(trace.record_count(), 4);
        assert!(!trace.records().iter().any(|record| matches!(
            record,
            TraceRecord::Instruction { pc, .. } if *pc == 4
        )));

        let mut stop_tick = factory.create().expect("backend");
        let config = TraceConfig::new(16)
            .expect("config")
            .with_stop(TraceStop::Tick(Tick::from_raw(3)));
        let trace = capture(stop_tick.as_mut(), &config, 10).expect("capture");
        assert_eq!(trace.meta().stop_reason, TraceStopReason::StopCondition);
        assert_eq!(trace.meta().end_tick, Tick::from_raw(3));
        assert_eq!(trace.record_count(), 3);

        let mut both = factory.create().expect("backend");
        let config = TraceConfig::new(16)
            .expect("config")
            .with_start(TraceStart::Pc(2))
            .with_stop(TraceStop::Pc(2));
        let trace = capture(both.as_mut(), &config, 10).expect("capture");
        assert_eq!(trace.meta().stop_reason, TraceStopReason::StopCondition);
        assert!(!trace.meta().triggered);
        assert_eq!(trace.record_count(), 0);
        assert_eq!(trace.meta().end_tick, Tick::from_raw(2));

        let halted_factory =
            SyntheticBackendFactory::new(SyntheticProgram::new(vec![0x00, 0xFF]).expect("program"));
        let mut halted = halted_factory.create().expect("backend");
        let trace =
            capture(halted.as_mut(), &TraceConfig::new(8).expect("config"), 10).expect("capture");
        assert_eq!(trace.meta().stop_reason, TraceStopReason::Halted);
        assert_eq!(trace.record_count(), 2);
        assert_eq!(trace.meta().end_tick, Tick::from_raw(2));
    }

    #[test]
    fn kind_mask_disables_records_without_leaving_index_gaps() {
        let mut backend = factory().create().expect("backend");
        let config = TraceConfig::new(16)
            .expect("config")
            .with_kinds(TraceKindMask::none().with(TraceKind::Instruction));
        let trace = capture(backend.as_mut(), &config, 6).expect("capture");
        for (position, record) in trace.records().iter().enumerate() {
            assert_eq!(record.kind(), TraceKind::Instruction);
            assert_eq!(record.index(), position as u64);
        }
    }
}
