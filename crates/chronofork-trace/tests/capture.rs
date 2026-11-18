//! Host capture acceptance tests over the synthetic and Mega Drive backends.
//!
//! Every test uses the single [`chronofork_trace::capture`] implementation and
//! independently derived expectations from a second backend instance or from the
//! synthetic ROM bytes. They return `Result` instead of unwrapping so the
//! workspace `expect_used` lint stays satisfied without a file-level allowance.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendId, BackendSnapshot,
    Capabilities, DebugAccess, MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, Tick};
use chronofork_megadrive::{
    MegadriveFactory, SchedulerConfig, controller_rom, demo_rom, interrupt_rom,
};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};
use chronofork_trace::{
    InterruptPhase, Trace, TraceConfig, TraceError, TraceKind, TraceKindMask, TraceRecord,
    TraceStart, TraceStop, TraceStopReason, capture,
};

fn program_counter(inspection: &MachineInspection) -> Result<u32, Box<dyn Error>> {
    let value = *inspection
        .registers
        .get("pc")
        .ok_or("inspection has no pc register")?;
    Ok(u32::try_from(value)?)
}

fn opcode_at(backend: &dyn Backend, pc: u32) -> Result<u16, Box<dyn Error>> {
    let bytes = backend.read_memory(u64::from(pc), 2)?;
    let pair = <[u8; 2]>::try_from(bytes.as_slice())?;
    Ok(u16::from_be_bytes(pair))
}

fn bus_writes(trace: &Trace) -> Vec<(u32, u32, u8)> {
    trace
        .records()
        .iter()
        .filter_map(|record| match record {
            TraceRecord::BusWrite {
                address,
                value,
                width,
                ..
            } => Some((*address, *value, *width)),
            _ => None,
        })
        .collect()
}

fn vdp_ports(trace: &Trace) -> Vec<(u8, u32, u8, bool, u64)> {
    trace
        .records()
        .iter()
        .filter_map(|record| match record {
            TraceRecord::VdpPort {
                offset,
                value,
                width,
                write,
                tick,
                ..
            } => Some((*offset, *value, *width, *write, tick.raw())),
            _ => None,
        })
        .collect()
}

fn controller_reads(trace: &Trace) -> Vec<(u8, u8)> {
    trace
        .records()
        .iter()
        .filter_map(|record| match record {
            TraceRecord::ControllerRead { port, value, .. } => Some((*port, *value)),
            _ => None,
        })
        .collect()
}

fn interrupts(trace: &Trace) -> Vec<(u8, InterruptPhase, u64)> {
    trace
        .records()
        .iter()
        .filter_map(|record| match record {
            TraceRecord::Interrupt {
                level, phase, tick, ..
            } => Some((*level, *phase, tick.raw())),
            _ => None,
        })
        .collect()
}

fn assert_events_share_instruction_ticks(trace: &Trace) {
    let instruction_ticks: Vec<u64> = trace
        .records()
        .iter()
        .filter_map(|record| match record {
            TraceRecord::Instruction { tick, .. } => Some(tick.raw()),
            _ => None,
        })
        .collect();
    for record in trace.records() {
        if record.kind() != TraceKind::Instruction {
            assert!(
                instruction_ticks.contains(&record.tick().raw()),
                "event record {record:?} must share a pre-step tick with an instruction"
            );
        }
    }
}

fn instruction_pcs(trace: &Trace) -> Vec<(u32, u16, u64)> {
    trace
        .records()
        .iter()
        .filter_map(|record| match record {
            TraceRecord::Instruction {
                pc, opcode, tick, ..
            } => Some((*pc, *opcode, tick.raw())),
            _ => None,
        })
        .collect()
}

#[test]
fn captures_instruction_records_on_the_synthetic_backend() -> Result<(), Box<dyn Error>> {
    let factory = SyntheticBackendFactory::default();
    let mut traced = factory.create()?;
    let mut reference = factory.create()?;
    let initial = traced.inspect()?;
    let config = TraceConfig::new(64)?;
    let trace = capture(traced.as_mut(), &config, 10)?;

    assert_eq!(trace.record_count(), 10);
    assert_eq!(trace.meta().state_hash, initial.state_hash);
    assert_eq!(trace.meta().start_tick, Tick::ZERO);
    assert_eq!(trace.meta().end_tick, Tick::from_raw(10));
    assert_eq!(trace.meta().stop_reason, TraceStopReason::StepLimit);
    assert!(trace.meta().triggered);
    assert!(!trace.meta().events_observed);
    assert_eq!(trace.meta().first_index, 0);
    assert_eq!(trace.meta().dropped, 0);

    for position in 0..10 {
        let record = &trace.records()[position];
        let inspection = reference.inspect()?;
        let expected_pc = program_counter(&inspection)?;
        let expected_opcode = opcode_at(reference.as_ref(), expected_pc)?;
        match record {
            TraceRecord::Instruction {
                index,
                tick,
                pc,
                opcode,
            } => {
                assert_eq!(*index, position as u64);
                assert_eq!(usize::try_from(*index)?, position);
                assert_eq!(*tick, inspection.tick);
                assert_eq!(tick.raw(), position as u64);
                assert_eq!(*pc, expected_pc);
                assert_eq!(*opcode, expected_opcode);
            }
            other => {
                return Err(format!("expected only instruction records, got {other:?}").into());
            }
        }
        reference.step(InputFrame::NEUTRAL)?;
    }

    let mut second_backend = factory.create()?;
    let second = capture(second_backend.as_mut(), &config, 10)?;
    assert_eq!(
        trace.to_bytes(),
        second.to_bytes(),
        "two independent captures must be byte-identical"
    );
    Ok(())
}

#[test]
fn captures_instruction_records_on_the_megadrive_slice() -> Result<(), Box<dyn Error>> {
    let factory = MegadriveFactory::new(demo_rom())?;
    let rom = factory.rom().to_vec();
    let mut traced = factory.create()?;
    let mut reference = factory.create()?;
    let config = TraceConfig::new(256)?;
    let trace = capture(traced.as_mut(), &config, 10)?;

    assert!(
        trace.meta().events_observed,
        "the Mega Drive slice advertises debug_events"
    );
    let instructions = instruction_pcs(&trace);
    assert_eq!(instructions.len(), 10);
    let mut previous_tick = 0_u64;
    for (pc, opcode, tick) in &instructions {
        let inspection = reference.inspect()?;
        assert_eq!(inspection.tick.raw(), *tick);
        assert!(tick >= &previous_tick);
        previous_tick = *tick;
        assert_eq!(*pc, program_counter(&inspection)?);
        let index = usize::try_from(*pc)?;
        let expected = u16::from_be_bytes([rom[index], rom[index + 1]]);
        assert_eq!(*opcode, expected, "opcode word at pc 0x{pc:04X}");
        reference.step(InputFrame::NEUTRAL)?;
    }

    // A multi-word instruction produces one record for its first word: the
    // expected PC sequence jumps by the instruction length, not by two.
    let pcs: Vec<u32> = instructions.iter().map(|(pc, _, _)| *pc).collect();
    assert_eq!(pcs[0], 0x0000_0200);
    assert_eq!(pcs[1], 0x0000_0202);
    assert_eq!(pcs[2], 0x0000_0204);
    assert_eq!(pcs[3], 0x0000_0206);
    assert_eq!(pcs[4], 0x0000_020C);
    assert!(
        trace
            .records()
            .windows(2)
            .all(|window| window[0].tick() <= window[1].tick()),
        "record ticks must be non-decreasing"
    );
    assert!(
        trace.records().iter().all(|record| {
            record.tick() >= trace.meta().start_tick && record.tick() <= trace.meta().end_tick
        }),
        "every record tick stays inside the capture window"
    );

    let mut second_backend = factory.create()?;
    let second = capture(second_backend.as_mut(), &config, 10)?;
    assert_eq!(trace.to_bytes(), second.to_bytes());
    Ok(())
}

#[test]
fn captures_bus_write_and_controller_records() -> Result<(), Box<dyn Error>> {
    let factory = MegadriveFactory::new(controller_rom())?;
    let mut backend = factory.create()?;
    let config = TraceConfig::new(512)?;
    let trace = capture(backend.as_mut(), &config, 20)?;
    assert!(trace.meta().events_observed);

    let writes = bus_writes(&trace);
    assert!(
        writes
            .iter()
            .any(|(address, value, width)| *address == 0x00FF_0000
                && *value == 0x7F
                && *width == 1),
        "the neutral controller accumulation must produce a byte bus write: {writes:?}"
    );
    assert!(
        trace.records().iter().any(|record| matches!(
            record,
            TraceRecord::BusWrite {
                address,
                value,
                width,
                tick,
                ..
            } if *address == 0x00FF_0000 && *value == 0x7F && *width == 1 && tick.raw() == 252
        )),
        "the bus write must carry the pre-step tick"
    );
    assert!(
        trace.records().iter().any(|record| matches!(
            record,
            TraceRecord::ControllerRead {
                port,
                value,
                tick,
                ..
            } if *port == 1 && *value == 0x7F && tick.raw() == 112
        )),
        "the controller read must carry the pre-step tick"
    );
    let reads = controller_reads(&trace);
    assert!(
        reads.contains(&(1, 0x7F)),
        "controller port 1 must report the neutral byte: {reads:?}"
    );
    assert!(
        reads.contains(&(2, 0x7F)),
        "controller port 2 must report the neutral byte: {reads:?}"
    );

    // A host debugger read is not an emulated bus access and produces no event.
    let before = backend
        .debug_access()
        .and_then(|access| access.debug_event_log())
        .map(|log| (log.len(), log.dropped(), log.next_index()));
    backend.read_memory(0x200, 2)?;
    let after = backend
        .debug_access()
        .and_then(|access| access.debug_event_log())
        .map(|log| (log.len(), log.dropped(), log.next_index()));
    assert_eq!(before, after, "host memory reads must not produce events");

    // VDP writes are device records and are never duplicated as bus writes.
    let vdp_factory = MegadriveFactory::new(demo_rom())?;
    let mut vdp_backend = vdp_factory.create()?;
    let vdp_trace = capture(vdp_backend.as_mut(), &config, 20)?;
    let vdp_records = vdp_ports(&vdp_trace);
    assert!(
        vdp_records
            .iter()
            .any(|(offset, _, width, write, _)| { *offset == 4 && *width == 2 && *write })
    );
    assert!(
        !bus_writes(&vdp_trace)
            .iter()
            .any(|(address, _, _)| *address == 0x00C0_0004),
        "a VDP port write must not also be a BusWrite record"
    );

    // Disabling a kind drops its records without leaving index gaps.
    let filtered_config = TraceConfig::new(512)?.with_kinds(
        TraceKindMask::none()
            .with(TraceKind::Instruction)
            .with(TraceKind::ControllerRead),
    );
    let mut filtered_backend = factory.create()?;
    let filtered = capture(filtered_backend.as_mut(), &filtered_config, 20)?;
    assert!(bus_writes(&filtered).is_empty());
    assert!(!controller_reads(&filtered).is_empty());
    for (position, record) in filtered.records().iter().enumerate() {
        assert_eq!(record.index(), position as u64);
    }

    assert_events_share_instruction_ticks(&trace);
    let mut repeat_backend = factory.create()?;
    let repeat = capture(repeat_backend.as_mut(), &config, 20)?;
    assert_eq!(trace.to_bytes(), repeat.to_bytes());
    Ok(())
}

#[test]
fn captures_vdp_port_records() -> Result<(), Box<dyn Error>> {
    let factory = MegadriveFactory::new(demo_rom())?;
    let mut backend = factory.create()?;
    let config = TraceConfig::new(512)?;
    let trace = capture(backend.as_mut(), &config, 40)?;
    let ports = vdp_ports(&trace);
    assert!(!ports.is_empty(), "the demo ROM accesses the VDP ports");

    for (offset, _, width, _, _) in &ports {
        assert!(
            *offset == 0 || *offset == 4,
            "offset {offset} outside the VDP window"
        );
        assert!(
            (1..=2).contains(width),
            "VDP access width {width} outside 1..=2"
        );
    }
    assert!(
        ports
            .iter()
            .any(|(offset, value, width, write, _)| *offset == 4
                && *value == 0x8F02
                && *width == 2
                && *write),
        "control-port register write must be a word VdpPort record"
    );
    assert!(
        ports
            .iter()
            .any(|(offset, value, width, write, _)| *offset == 0
                && *value == 0x1234
                && *width == 2
                && *write),
        "data-port word write must be a VdpPort record"
    );
    assert!(
        ports
            .iter()
            .any(|(offset, value, width, write, _)| *offset == 0
                && *value == 0x1234
                && *width == 2
                && !*write),
        "data-port word read must be a VdpPort record"
    );
    assert!(
        ports.iter().any(|(_, _, _, write, _)| *write),
        "the demo must produce VDP writes"
    );
    assert!(
        ports.iter().any(|(_, _, _, write, _)| !*write),
        "the demo must produce VDP reads"
    );

    assert_events_share_instruction_ticks(&trace);
    let mut repeat_backend = factory.create()?;
    let repeat = capture(repeat_backend.as_mut(), &config, 40)?;
    assert_eq!(trace.to_bytes(), repeat.to_bytes());
    Ok(())
}

#[test]
fn captures_interrupt_records() -> Result<(), Box<dyn Error>> {
    let scheduler = SchedulerConfig::ntsc();
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), scheduler)?;
    let mut backend = factory.create()?;
    let config = TraceConfig::new(512)?;
    let trace = capture(backend.as_mut(), &config, 200)?;
    let phases = interrupts(&trace);
    assert!(
        phases.len() >= 3,
        "the delivery scenario must record requested/delivered/acknowledged"
    );
    assert_eq!(
        phases[0],
        (4, InterruptPhase::Requested, phases[0].2),
        "the retained HBlank request is stamped with its pre-step tick"
    );
    assert_eq!(phases[1], (4, InterruptPhase::Delivered, phases[1].2));
    assert_eq!(phases[2], (4, InterruptPhase::Acknowledged, phases[2].2));
    assert_eq!(
        phases[1].2, phases[2].2,
        "delivery and acknowledgement share the delivering step"
    );
    for (level, _, _) in &phases {
        assert!(
            (1..=7).contains(level),
            "interrupt level {level} outside 1..=7"
        );
    }

    // A fully masked run records the request but no delivery or acknowledgement.
    let masked_factory = MegadriveFactory::with_scheduler_config(interrupt_rom(7), scheduler)?;
    let mut masked_backend = masked_factory.create()?;
    let masked = capture(masked_backend.as_mut(), &config, 200)?;
    let masked_phases = interrupts(&masked);
    assert!(
        masked_phases
            .iter()
            .any(|(level, phase, _)| *level == 4 && *phase == InterruptPhase::Requested),
        "the masked HBlank request must still be observed"
    );
    assert!(
        !masked_phases.iter().any(|(_, phase, _)| {
            *phase == InterruptPhase::Delivered || *phase == InterruptPhase::Acknowledged
        }),
        "masked VDP levels must not be delivered or acknowledged"
    );
    assert_eq!(
        masked_phases
            .iter()
            .filter(|(level, phase, _)| *level == 4 && *phase == InterruptPhase::Requested)
            .count(),
        1,
        "coalesced requests must not be reported as duplicate request events"
    );
    assert_events_share_instruction_ticks(&masked);

    let mut repeat_backend =
        MegadriveFactory::with_scheduler_config(interrupt_rom(3), SchedulerConfig::ntsc())?
            .create()?;
    let repeat = capture(repeat_backend.as_mut(), &config, 200)?;
    assert_eq!(trace.to_bytes(), repeat.to_bytes());
    Ok(())
}

#[test]
fn honours_start_and_stop_triggers() -> Result<(), Box<dyn Error>> {
    let program = SyntheticProgram::new(vec![
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00,
    ])?;
    let factory = SyntheticBackendFactory::new(program);
    let capacity = 64;

    let mut immediate = factory.create()?;
    let trace = capture(immediate.as_mut(), &TraceConfig::new(capacity)?, 4)?;
    assert!(trace.meta().triggered);
    assert_eq!(trace.records()[0].tick(), Tick::ZERO);

    let mut from_pc = factory.create()?;
    let trace = capture(
        from_pc.as_mut(),
        &TraceConfig::new(capacity)?.with_start(TraceStart::Pc(2)),
        6,
    )?;
    assert!(trace.meta().triggered);
    assert_eq!(instruction_pcs(&trace)[0].2, 2);
    assert_eq!(instruction_pcs(&trace).len(), 4);

    let mut from_tick = factory.create()?;
    let trace = capture(
        from_tick.as_mut(),
        &TraceConfig::new(capacity)?.with_start(TraceStart::Tick(Tick::from_raw(2))),
        6,
    )?;
    assert!(trace.meta().triggered);
    assert_eq!(instruction_pcs(&trace)[0].2, 2);

    let mut stop_pc = factory.create()?;
    let trace = capture(
        stop_pc.as_mut(),
        &TraceConfig::new(capacity)?.with_stop(TraceStop::Pc(4)),
        10,
    )?;
    assert_eq!(trace.meta().stop_reason, TraceStopReason::StopCondition);
    assert_eq!(trace.meta().end_tick, Tick::from_raw(4));
    let stopped_pcs: Vec<u32> = instruction_pcs(&trace)
        .iter()
        .map(|(pc, _, _)| *pc)
        .collect();
    assert_eq!(stopped_pcs, vec![0, 1, 2, 3]);

    let mut stop_tick = factory.create()?;
    let trace = capture(
        stop_tick.as_mut(),
        &TraceConfig::new(capacity)?.with_stop(TraceStop::Tick(Tick::from_raw(3))),
        10,
    )?;
    assert_eq!(trace.meta().stop_reason, TraceStopReason::StopCondition);
    assert_eq!(trace.meta().end_tick, Tick::from_raw(3));
    assert_eq!(instruction_pcs(&trace).len(), 3);

    // A stop before the first instruction executes produces an empty trace.
    let mut stop_zero = factory.create()?;
    let trace = capture(
        stop_zero.as_mut(),
        &TraceConfig::new(capacity)?.with_stop(TraceStop::Pc(0)),
        5,
    )?;
    assert_eq!(trace.meta().stop_reason, TraceStopReason::StopCondition);
    assert!(!trace.meta().triggered);
    assert_eq!(trace.record_count(), 0);
    assert_eq!(trace.meta().end_tick, Tick::ZERO);

    // A stop and a start at the same boundary: the stop wins and triggered keeps
    // its prior value.
    let mut both = factory.create()?;
    let trace = capture(
        both.as_mut(),
        &TraceConfig::new(capacity)?
            .with_start(TraceStart::Pc(2))
            .with_stop(TraceStop::Pc(2)),
        10,
    )?;
    assert_eq!(trace.meta().stop_reason, TraceStopReason::StopCondition);
    assert!(!trace.meta().triggered);
    assert_eq!(trace.record_count(), 0);
    assert_eq!(trace.meta().end_tick, Tick::from_raw(2));

    // A start that never matches records nothing but still reaches StepLimit.
    let mut never = factory.create()?;
    let trace = capture(
        never.as_mut(),
        &TraceConfig::new(capacity)?.with_start(TraceStart::Pc(0xFFFF)),
        4,
    )?;
    assert_eq!(trace.meta().stop_reason, TraceStopReason::StepLimit);
    assert!(!trace.meta().triggered);
    assert_eq!(trace.record_count(), 0);
    assert_eq!(trace.meta().end_tick, Tick::from_raw(4));

    let halted_factory = SyntheticBackendFactory::new(SyntheticProgram::new(vec![0x00, 0xFF])?);
    let mut halted = halted_factory.create()?;
    let trace = capture(halted.as_mut(), &TraceConfig::new(capacity)?, 10)?;
    assert_eq!(trace.meta().stop_reason, TraceStopReason::Halted);
    assert_eq!(instruction_pcs(&trace).len(), 2);
    assert_eq!(trace.meta().end_tick, Tick::from_raw(2));

    let mut window_backend = factory.create()?;
    let window_trace = capture(window_backend.as_mut(), &TraceConfig::new(capacity)?, 4)?;
    assert!(window_trace.meta().end_tick >= window_trace.meta().start_tick);
    assert!(window_trace.records().iter().all(|record| {
        record.tick() >= window_trace.meta().start_tick
            && record.tick() <= window_trace.meta().end_tick
    }));
    Ok(())
}

#[test]
fn circular_retention_reports_dropped_and_first_index() -> Result<(), Box<dyn Error>> {
    let factory = SyntheticBackendFactory::default();

    let mut small_backend = factory.create()?;
    let small = capture(small_backend.as_mut(), &TraceConfig::new(5)?, 20)?;
    assert_eq!(small.record_count(), 5);
    assert_eq!(small.meta().capacity, 5);
    assert_eq!(small.meta().dropped, 15);
    assert_eq!(small.meta().first_index, 15);
    let indexes: Vec<u64> = small.records().iter().map(TraceRecord::index).collect();
    assert_eq!(indexes, vec![15, 16, 17, 18, 19]);
    assert!(
        small
            .records()
            .iter()
            .zip(15_u64..)
            .all(|(record, tick)| record.tick().raw() == tick)
    );

    let mut large_backend = factory.create()?;
    let large = capture(large_backend.as_mut(), &TraceConfig::new(64)?, 20)?;
    assert_eq!(large.meta().dropped, 0);
    assert_eq!(large.record_count(), 20);
    assert_eq!(
        small.records(),
        &large.records()[15..],
        "the retained window must be the newest tail of the same run"
    );

    let decoded = Trace::from_bytes(&small.to_bytes())?;
    assert_eq!(decoded.meta().dropped, 15);
    assert_eq!(decoded.meta().first_index, 15);
    assert_eq!(decoded, small);

    let mut oversized_backend = factory.create()?;
    let oversized = capture(oversized_backend.as_mut(), &TraceConfig::new(64)?, 10)?;
    assert_eq!(oversized.meta().dropped, 0);
    assert_eq!(oversized.meta().first_index, 0);

    // A tiny backend event ring that overflows inside one step is a typed error,
    // never silent loss. Interrupt delivery observes two phases in one step.
    let scheduler = SchedulerConfig::ntsc();
    let interrupt_factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), scheduler)?;
    let mut overflow_backend = interrupt_factory.create()?;
    let overflow_config = TraceConfig::new(64)?.with_event_capacity(1)?;
    let overflow = capture_error(overflow_backend.as_mut(), &overflow_config, 200)?;
    assert_eq!(overflow.code(), "event_ring_overflow");
    assert!(matches!(overflow, TraceError::EventRingOverflow));

    let mut repeat_backend = factory.create()?;
    let repeat = capture(repeat_backend.as_mut(), &TraceConfig::new(5)?, 20)?;
    assert_eq!(small.to_bytes(), repeat.to_bytes());
    Ok(())
}

/// A backend wrapper that delegates everything and counts canonical mutators
/// that capture is forbidden to call.
struct CountedBackend {
    inner: Box<dyn Backend>,
    snapshot_calls: Arc<AtomicUsize>,
    restore_calls: Arc<AtomicUsize>,
    write_calls: Arc<AtomicUsize>,
}

impl CountedBackend {
    fn new(inner: Box<dyn Backend>) -> Self {
        Self {
            inner,
            snapshot_calls: Arc::new(AtomicUsize::new(0)),
            restore_calls: Arc::new(AtomicUsize::new(0)),
            write_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn assert_untouched(&self, context: &str) {
        assert_eq!(
            self.snapshot_calls.load(Ordering::SeqCst),
            0,
            "{context}: snapshot"
        );
        assert_eq!(
            self.restore_calls.load(Ordering::SeqCst),
            0,
            "{context}: restore"
        );
        assert_eq!(
            self.write_calls.load(Ordering::SeqCst),
            0,
            "{context}: write_memory"
        );
    }
}

impl Backend for CountedBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.inner.step(input)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.snapshot()
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        self.restore_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.restore(snapshot)
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        self.inner.inspect()
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        self.inner.read_memory(address, length)
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        self.write_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.write_memory(address, data)
    }

    fn debug_access(&mut self) -> Option<&mut dyn DebugAccess> {
        self.inner.debug_access()
    }
}

fn assert_same_canonical_state(
    traced: &dyn Backend,
    plain: &dyn Backend,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let traced_view = traced.inspect()?;
    let plain_view = plain.inspect()?;
    assert_eq!(traced_view.tick, plain_view.tick, "{context}: tick");
    assert_eq!(
        traced_view.state_hash, plain_view.state_hash,
        "{context}: state hash"
    );
    assert_eq!(traced_view.halted, plain_view.halted, "{context}: halted");
    assert_eq!(
        traced_view.registers, plain_view.registers,
        "{context}: registers"
    );
    assert_eq!(
        traced_view.memory_base, plain_view.memory_base,
        "{context}: memory base"
    );
    assert_eq!(traced_view.memory, plain_view.memory, "{context}: memory");

    let traced_snapshot = traced.snapshot()?;
    let plain_snapshot = plain.snapshot()?;
    assert_eq!(
        traced_snapshot.backend_id(),
        plain_snapshot.backend_id(),
        "{context}: backend id"
    );
    assert_eq!(
        traced_snapshot.backend_version(),
        plain_snapshot.backend_version(),
        "{context}: backend version"
    );
    assert_eq!(
        traced_snapshot.format_version(),
        plain_snapshot.format_version(),
        "{context}: payload version"
    );
    assert_eq!(
        traced_snapshot.tick(),
        plain_snapshot.tick(),
        "{context}: snapshot tick"
    );
    assert_eq!(
        traced_snapshot.config_hash(),
        plain_snapshot.config_hash(),
        "{context}: config hash"
    );
    assert_eq!(
        traced_snapshot.state_hash(),
        plain_snapshot.state_hash(),
        "{context}: snapshot hash"
    );
    assert_eq!(
        traced_snapshot.payload(),
        plain_snapshot.payload(),
        "{context}: snapshot payload bytes"
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum PcMode {
    Normal,
    Remove,
    Wide,
}

/// A delegating backend that injects one typed capture failure.
struct ProbeBackend {
    inner: Box<dyn Backend>,
    descriptor: BackendDescriptor,
    pc_mode: PcMode,
    fail_reads: bool,
}

impl ProbeBackend {
    fn new(inner: Box<dyn Backend>) -> Self {
        Self {
            descriptor: inner.descriptor().clone(),
            inner,
            pc_mode: PcMode::Normal,
            fail_reads: false,
        }
    }

    fn with_pc_mode(inner: Box<dyn Backend>, pc_mode: PcMode) -> Self {
        Self {
            pc_mode,
            ..Self::new(inner)
        }
    }

    fn with_failed_reads(inner: Box<dyn Backend>) -> Self {
        Self {
            fail_reads: true,
            ..Self::new(inner)
        }
    }

    fn advertising_events(inner: Box<dyn Backend>) -> Self {
        let descriptor = inner.descriptor();
        let capabilities = Capabilities {
            debug_events: true,
            ..descriptor.capabilities()
        };
        let advertised = BackendDescriptor::new(
            BackendId::new("chronofork.probe.advertised"),
            descriptor.display_name(),
            descriptor.version(),
            capabilities,
            true,
        );
        Self {
            descriptor: advertised,
            inner,
            pc_mode: PcMode::Normal,
            fail_reads: false,
        }
    }
}

impl Backend for ProbeBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.inner.step(input)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.inner.snapshot()
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        self.inner.restore(snapshot)
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        let mut inspection = self.inner.inspect()?;
        match self.pc_mode {
            PcMode::Normal => {}
            PcMode::Remove => {
                inspection.registers.remove("pc");
            }
            PcMode::Wide => {
                inspection
                    .registers
                    .insert("pc".to_owned(), u64::from(u32::MAX) + 1);
            }
        }
        Ok(inspection)
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        if self.fail_reads {
            Err(BackendError::Execution(format!(
                "probe cannot read {length} bytes at {address}"
            )))
        } else {
            self.inner.read_memory(address, length)
        }
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        self.inner.write_memory(address, data)
    }
}

/// Runs a capture that is expected to fail and returns its typed error.
fn capture_error(
    backend: &mut dyn Backend,
    config: &TraceConfig,
    steps: u64,
) -> Result<TraceError, Box<dyn Error>> {
    match capture(backend, config, steps) {
        Ok(trace) => Err(format!(
            "expected a typed failure, got a {}-record trace",
            trace.record_count()
        )
        .into()),
        Err(error) => Ok(error),
    }
}

#[test]
fn typed_capture_failures_are_reported() -> Result<(), Box<dyn Error>> {
    let factory = SyntheticBackendFactory::default();
    let config = TraceConfig::default();

    let mut missing = ProbeBackend::with_pc_mode(factory.create()?, PcMode::Remove);
    assert_eq!(
        capture_error(&mut missing, &config, 1)?,
        TraceError::ProgramCounterUnavailable
    );

    let mut wide = ProbeBackend::with_pc_mode(factory.create()?, PcMode::Wide);
    assert_eq!(
        capture_error(&mut wide, &config, 1)?,
        TraceError::InvalidProgramCounter {
            value: u64::from(u32::MAX) + 1
        }
    );

    let mut no_memory = ProbeBackend::with_failed_reads(factory.create()?);
    let error = capture_error(&mut no_memory, &config, 1)?;
    assert_eq!(error.code(), "opcode_read_failed");
    assert!(matches!(error, TraceError::OpcodeReadFailed { .. }));

    let mut advertised = ProbeBackend::advertising_events(factory.create()?);
    assert_eq!(
        capture_error(&mut advertised, &config, 1)?,
        TraceError::EventObservationUnavailable
    );
    Ok(())
}

#[test]
fn capture_does_not_perturb_state_or_snapshot() -> Result<(), Box<dyn Error>> {
    let synthetic = SyntheticBackendFactory::default();
    let mut traced = CountedBackend::new(synthetic.create()?);
    let mut plain = synthetic.create()?;
    let synthetic_trace = capture(&mut traced, &TraceConfig::new(64)?, 30)?;
    traced.assert_untouched("synthetic capture");
    for _ in 0..30 {
        plain.step(InputFrame::NEUTRAL)?;
    }
    assert_same_canonical_state(&traced, plain.as_ref(), "synthetic")?;
    assert_eq!(synthetic_trace.record_count(), 30);
    assert!(!synthetic_trace.meta().events_observed);

    // A different trigger/event configuration must be equally non-canonical.
    let mut configured = CountedBackend::new(synthetic.create()?);
    let configured_trace = capture(
        &mut configured,
        &TraceConfig::new(32)?
            .with_start(TraceStart::Tick(Tick::from_raw(5)))
            .with_event_capacity(32)?,
        30,
    )?;
    configured.assert_untouched("configured capture");
    assert_same_canonical_state(&configured, plain.as_ref(), "configured")?;
    assert_eq!(configured_trace.meta().dropped, 0);

    let megadrive = MegadriveFactory::new(demo_rom())?;
    let mut traced = CountedBackend::new(megadrive.create()?);
    let mut plain = megadrive.create()?;
    let md_trace = capture(&mut traced, &TraceConfig::new(64)?, 30)?;
    traced.assert_untouched("Mega Drive capture");
    for _ in 0..30 {
        plain.step(InputFrame::NEUTRAL)?;
    }
    assert_same_canonical_state(&traced, plain.as_ref(), "Mega Drive")?;
    assert!(md_trace.meta().events_observed);
    Ok(())
}
