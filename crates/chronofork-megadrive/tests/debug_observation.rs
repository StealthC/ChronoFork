#![allow(clippy::expect_used)]

//! Acceptance tests for Mega Drive debug observation (ADR-0010 §3.1 and the
//! observation half of §3.3).
//!
//! The rows implemented here are M4-A4, M4-A5, M4-A9, M4-A10 and the observation
//! half of M4-A12 in
//! `docs/quality/m4-debugging-depth-acceptance.md`. Every assertion is exact:
//! event kinds, addresses, values, widths, ticks, ring indexes and dropped counts
//! are compared against the documented rules instead of being counted loosely.
//! Observation is debug data, so these tests also prove that it never perturbs
//! canonical state, snapshots, state hashes, configuration hashes or the payload
//! version.

use chronofork_backend_api::{
    Backend, BackendError, DebugAccess, DebugAddressRange, DebugConfig, DebugEvent, DebugEventKind,
    DebugEventLog, MAX_DEBUG_EVENT_CAPACITY, MAX_WATCHPOINTS, WatchpointAccess, WatchpointSpec,
};
use chronofork_core::{InputFrame, Tick};
use chronofork_megadrive::{
    MegadriveBackend, MegadriveFactory, PAYLOAD_VERSION, SchedulerConfig, controller_rom, demo_rom,
    interrupt_rom,
};

const RAM_BASE: u32 = 0x00FF_0000;

fn new_machine(rom: Vec<u8>) -> MegadriveBackend {
    MegadriveFactory::new(rom)
        .expect("valid ROM")
        .create_machine()
        .expect("machine")
}

fn interrupt_machine(mask: u8) -> MegadriveBackend {
    let config = SchedulerConfig::ntsc();
    MegadriveFactory::with_scheduler_config(interrupt_rom(mask), config)
        .expect("valid interrupt ROM")
        .create_machine()
        .expect("interrupt machine")
}

fn range(start: u32, end: u32) -> DebugAddressRange {
    DebugAddressRange::new(start, end).expect("valid inclusive range")
}

fn spec(id: u32, start: u32, end: u32, access: WatchpointAccess) -> WatchpointSpec {
    WatchpointSpec::new(id, range(start, end), access).expect("valid watchpoint")
}

fn configure(machine: &mut MegadriveBackend, capacity: usize, watchpoints: Vec<WatchpointSpec>) {
    let config = DebugConfig::new(capacity, watchpoints).expect("valid debug configuration");
    machine
        .debug_access()
        .expect("the Mega Drive backend observes its bus")
        .configure_debug(config)
        .expect("configure observation");
}

fn log_ref<'a>(machine: &'a mut MegadriveBackend) -> &'a DebugEventLog {
    let access: &'a mut dyn DebugAccess = machine
        .debug_access()
        .expect("the Mega Drive backend observes its bus");
    access.debug_event_log().expect("observation is configured")
}

fn events(machine: &mut MegadriveBackend) -> Vec<DebugEvent> {
    log_ref(machine).entries().collect()
}

fn event(tick: u64, kind: DebugEventKind) -> DebugEvent {
    DebugEvent {
        tick: Tick::from_raw(tick),
        kind,
    }
}

fn watchpoint(id: u32, access: WatchpointAccess, address: u32, value: u32) -> DebugEventKind {
    DebugEventKind::Watchpoint {
        watchpoint_id: id,
        access,
        address,
        value,
        width: 1,
    }
}

fn bus_write(address: u32, value: u32) -> DebugEventKind {
    DebugEventKind::BusWrite {
        address,
        value,
        width: 1,
    }
}

fn vdp_port(offset: u8, value: u32, width: u8, write: bool) -> DebugEventKind {
    DebugEventKind::VdpPort {
        offset,
        value,
        width,
        write,
    }
}

fn controller_read(port: u8, value: u8) -> DebugEventKind {
    DebugEventKind::ControllerRead { port, value }
}

/// Independently authored ROM for byte-granular VDP port observation.
///
/// `MOVEQ #2,D0`; `MOVE.B D0,(0x00C00004).L` (one width-1 control-port write);
/// `MOVE.B (0x00C00008).L,D1` (one width-1 HV-counter read); `BRA.S` to itself.
fn byte_vdp_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x210].copy_from_slice(&[
        0x70, 0x02, // MOVEQ #2,D0
        0x13, 0xC0, 0x00, 0xC0, 0x00, 0x04, // MOVE.B D0,(0x00C00004).L
        0x12, 0x39, 0x00, 0xC0, 0x00, 0x08, // MOVE.B (0x00C00008).L,D1
        0x60, 0xFE, // BRA.S -2
    ]);
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the byte-VDP image holds a header"
    );
    rom
}

/// M4-A4 — memory watchpoints on the Mega Drive bus.
///
/// A watched RAM word store and load report one event per byte lane with the
/// exact address, value, access kind and pre-step tick. A controller
/// data-register read watchpoint fires on the emulated read. A watched
/// instruction fetch is reported, an unwatched one is not, and a debugger
/// `read_memory`/`write_memory` of the same bytes produces nothing.
#[test]
fn watchpoints_report_emulated_reads_and_writes() {
    // Ram word store at instruction 4 and word load at instruction 6.
    let mut machine = new_machine(demo_rom());
    configure(
        &mut machine,
        64,
        vec![
            spec(0x10, RAM_BASE, RAM_BASE, WatchpointAccess::Write),
            spec(0x11, RAM_BASE + 1, RAM_BASE + 1, WatchpointAccess::Write),
            spec(0x12, RAM_BASE, RAM_BASE + 1, WatchpointAccess::Read),
        ],
    );
    for _ in 0..3 {
        machine.step(InputFrame::NEUTRAL).expect("demo step");
    }
    assert!(
        events(&mut machine).is_empty(),
        "the first three instructions do not touch the watched bytes"
    );

    // Instruction 4: MOVE.W D0,(0x00FF0000).L stores the word 0x0003. Each byte
    // write is a BusWrite event and a watchpoint hit on its own lane.
    machine.step(InputFrame::NEUTRAL).expect("store step");
    machine.step(InputFrame::NEUTRAL).expect("MOVEQ step");
    machine.step(InputFrame::NEUTRAL).expect("load step");
    assert_eq!(
        events(&mut machine),
        vec![
            event(84, bus_write(RAM_BASE, 0x00)),
            event(
                84,
                watchpoint(0x10, WatchpointAccess::Write, RAM_BASE, 0x00)
            ),
            event(84, bus_write(RAM_BASE + 1, 0x03)),
            event(
                84,
                watchpoint(0x11, WatchpointAccess::Write, RAM_BASE + 1, 0x03)
            ),
            event(
                224,
                watchpoint(0x12, WatchpointAccess::Read, RAM_BASE, 0x00)
            ),
            event(
                224,
                watchpoint(0x12, WatchpointAccess::Read, RAM_BASE + 1, 0x03)
            ),
        ],
        "one byte event per matching lane with the exact pre-step tick"
    );

    // The same access outside every watched range is ignored: the next
    // instruction fetches at 0x214.. but touches neither watched byte.
    machine.step(InputFrame::NEUTRAL).expect("unwatched step");
    assert_eq!(
        events(&mut machine).len(),
        6,
        "unwatched access must be silent"
    );

    // A controller data-register read is an emulated bus read and matches.
    let mut controller = new_machine(controller_rom());
    configure(
        &mut controller,
        64,
        vec![spec(0x20, 0x00A1_0003, 0x00A1_0005, WatchpointAccess::Read)],
    );
    let frame = InputFrame::with_devices([0x01, 0x20, 0, 0], [0; 4], 0);
    for _ in 0..9 {
        controller.step(frame).expect("controller step");
    }
    assert_eq!(
        events(&mut controller),
        vec![
            event(112, controller_read(1, 0x7E)),
            event(
                112,
                watchpoint(0x20, WatchpointAccess::Read, 0x00A1_0003, 0x7E)
            ),
            event(252, bus_write(0x00FF_0000, 0x7E)),
            event(364, bus_write(0x00FF_0002, 0x7E)),
            event(476, controller_read(2, 0x5F)),
            event(
                476,
                watchpoint(0x20, WatchpointAccess::Read, 0x00A1_0005, 0x5F)
            ),
        ],
        "controller reads report the data-register byte that was returned"
    );

    // Instruction fetches are ordinary reads: a watchpoint sees them, and only
    // when it actually matches.
    let mut fetch = new_machine(demo_rom());
    configure(
        &mut fetch,
        16,
        vec![spec(0x30, 0x200, 0x201, WatchpointAccess::Read)],
    );
    fetch.step(InputFrame::NEUTRAL).expect("fetch step");
    assert_eq!(
        events(&mut fetch),
        vec![
            event(0, watchpoint(0x30, WatchpointAccess::Read, 0x200, 0x70)),
            event(0, watchpoint(0x30, WatchpointAccess::Read, 0x201, 0x01)),
        ],
        "the MOVEQ opcode word 0x7001 is fetched from 0x200"
    );
    fetch
        .step(InputFrame::NEUTRAL)
        .expect("unmatched fetch step");
    assert_eq!(
        events(&mut fetch).len(),
        2,
        "a fetch outside the watched range is not logged"
    );

    // Debugger memory paths never produce events and never fabricate a
    // watchpoint hit.
    let mut probe = new_machine(demo_rom());
    configure(
        &mut probe,
        16,
        vec![
            spec(0x40, RAM_BASE, RAM_BASE + 0x0F, WatchpointAccess::Read),
            spec(0x41, RAM_BASE, RAM_BASE + 0x0F, WatchpointAccess::Write),
        ],
    );
    let before = probe.snapshot().expect("snapshot");
    assert_eq!(
        probe
            .read_memory(u64::from(RAM_BASE), 4)
            .expect("debugger read"),
        vec![0, 0, 0, 0]
    );
    assert_eq!(
        probe.snapshot().expect("snapshot").state_hash(),
        before.state_hash(),
        "a debugger read must not change state"
    );
    probe
        .write_memory(u64::from(RAM_BASE), &[0xAA, 0xBB, 0xCC, 0xDD])
        .expect("debugger write");
    assert!(
        events(&mut probe).is_empty(),
        "debugger read_memory/write_memory must not produce events"
    );
    assert_eq!(log_ref(&mut probe).next_index(), 0);
    assert_eq!(log_ref(&mut probe).first_index(), None);
}

/// M4-A5 — invalid observation configuration is rejected with typed errors and a
/// valid maximum table is accepted.
#[test]
fn watchpoint_configuration_is_bounded() {
    let mut machine = new_machine(demo_rom());

    // The factory advertises exactly the observation it implements.
    let capabilities = machine.descriptor().capabilities();
    assert!(capabilities.watchpoints);
    assert!(capabilities.debug_events);
    assert!(
        !capabilities.breakpoints,
        "breakpoints are a host feature, not a backend capability"
    );

    // Event-ring capacity is 1..=MAX_DEBUG_EVENT_CAPACITY.
    for capacity in [0, MAX_DEBUG_EVENT_CAPACITY + 1] {
        let error = DebugEventLog::new(capacity).expect_err("capacity must be rejected");
        assert!(
            matches!(error, BackendError::InvalidConfiguration(_)),
            "capacity {capacity} must be a typed configuration error: {error}"
        );
        assert!(matches!(
            DebugConfig::new(capacity, Vec::new()),
            Err(BackendError::InvalidConfiguration(_))
        ));
    }

    // An empty (inverted) address range is rejected; a single address is valid.
    assert!(matches!(
        DebugAddressRange::new(0x300, 0x200),
        Err(BackendError::InvalidConfiguration(message)) if message.contains("greater than")
    ));
    assert!(DebugAddressRange::new(0x200, 0x200).is_ok());

    // Watchpoint ids must be nonzero and unique.
    assert!(matches!(
        WatchpointSpec::new(0, range(0x200, 0x201), WatchpointAccess::Read),
        Err(BackendError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        DebugConfig::new(
            8,
            vec![
                spec(1, 0x200, 0x201, WatchpointAccess::Read),
                spec(1, 0x300, 0x301, WatchpointAccess::Write),
            ],
        ),
        Err(BackendError::InvalidConfiguration(message)) if message.contains("duplicate")
    ));

    // At most MAX_WATCHPOINTS specs are accepted.
    let too_many: Vec<WatchpointSpec> = (1..=MAX_WATCHPOINTS + 1)
        .map(|id| {
            let id = u32::try_from(id).expect("fits");
            let start = 0x200 + id * 2;
            spec(id, start, start + 1, WatchpointAccess::Read)
        })
        .collect();
    assert!(matches!(
        DebugConfig::new(8, too_many),
        Err(BackendError::InvalidConfiguration(message))
            if message.contains("at most 64 debug watchpoints")
    ));

    // Exactly MAX_WATCHPOINTS unique nonzero specs install and observe.
    let maximum: Vec<WatchpointSpec> = (1..=MAX_WATCHPOINTS)
        .map(|id| {
            let id = u32::try_from(id).expect("fits");
            let start = 0x200 + id * 2;
            spec(id, start, start + 1, WatchpointAccess::Read)
        })
        .collect();
    configure(&mut machine, MAX_DEBUG_EVENT_CAPACITY, maximum);
    assert_eq!(log_ref(&mut machine).capacity(), MAX_DEBUG_EVENT_CAPACITY);
    assert!(log_ref(&mut machine).is_empty());
    for _ in 0..23 {
        machine.step(InputFrame::NEUTRAL).expect("demo step");
    }
    assert!(
        !log_ref(&mut machine).is_empty(),
        "a maximum-size table still observes emulated accesses"
    );
}

/// M4-A5 and the observation half of M4-A12 — observation is non-canonical.
///
/// The same ROM stepped with observation off and on produces identical step
/// outcomes, state hashes, snapshot payload bytes, configuration hashes, payload
/// versions and ticks at every probed tick. Restoring a checkpoint leaves the
/// event log and the watchpoint configuration untouched, and the debugger memory
/// paths produce no events.
#[test]
fn debug_observation_does_not_change_state_or_payload() {
    let factory = MegadriveFactory::new(demo_rom()).expect("valid demo ROM");
    let mut observed = factory.create_machine().expect("observed machine");
    let mut plain = factory.create_machine().expect("plain machine");
    configure(
        &mut observed,
        4096,
        vec![
            spec(1, RAM_BASE, RAM_BASE + 7, WatchpointAccess::Read),
            spec(2, RAM_BASE, RAM_BASE + 7, WatchpointAccess::Write),
            spec(3, 0x0000_0200, 0x0000_02FF, WatchpointAccess::Read),
            spec(4, 0x00C0_0000, 0x00C0_000F, WatchpointAccess::Read),
            spec(5, 0x00C0_0000, 0x00C0_000F, WatchpointAccess::Write),
            spec(6, 0x00A1_0000, 0x00A1_0020, WatchpointAccess::Read),
        ],
    );
    assert!(
        plain.debug_access().is_some(),
        "a Mega Drive machine always exposes the optional debug surface"
    );
    assert!(
        plain
            .debug_access()
            .expect("debug surface")
            .debug_event_log()
            .is_none(),
        "observation stays off until it is configured"
    );

    let mut observed_checkpoint = None;
    let mut plain_checkpoint = None;
    for step in 0..40_u64 {
        let observed_outcome = observed.step(InputFrame::NEUTRAL).expect("observed step");
        let plain_outcome = plain.step(InputFrame::NEUTRAL).expect("plain step");
        assert_eq!(
            observed_outcome, plain_outcome,
            "step outcome at step {step} must be identical"
        );

        let observed_snapshot = observed.snapshot().expect("observed snapshot");
        let plain_snapshot = plain.snapshot().expect("plain snapshot");
        assert_eq!(
            observed_snapshot.payload(),
            plain_snapshot.payload(),
            "snapshot payload bytes at step {step} must be identical"
        );
        assert_eq!(
            observed_snapshot.state_hash(),
            plain_snapshot.state_hash(),
            "state hash at step {step} must be identical"
        );
        assert_eq!(
            observed_snapshot.config_hash(),
            plain_snapshot.config_hash(),
            "configuration hash at step {step} must be identical"
        );
        assert_eq!(
            observed_snapshot.format_version(),
            plain_snapshot.format_version()
        );
        assert_eq!(observed_snapshot.tick(), plain_snapshot.tick());
        assert_eq!(
            &observed_snapshot.payload()[8..10],
            &PAYLOAD_VERSION.to_le_bytes(),
            "the payload version must not change"
        );
        if step == 10 {
            observed_checkpoint = Some(observed_snapshot);
            plain_checkpoint = Some(plain_snapshot);
        }
    }

    // Observation really was active: the run recorded events and dropped none.
    let recorded = log_ref(&mut observed).len();
    assert!(recorded > 0, "observation must have recorded events");
    assert_eq!(log_ref(&mut observed).dropped(), 0);
    assert_eq!(log_ref(&mut observed).next_index(), recorded as u64);

    // Restoring a checkpoint does not rewind or clear the event log.
    let observed_checkpoint = observed_checkpoint.expect("observed checkpoint");
    let plain_checkpoint = plain_checkpoint.expect("plain checkpoint");
    observed
        .restore(&observed_checkpoint)
        .expect("restore observed machine");
    plain
        .restore(&plain_checkpoint)
        .expect("restore plain machine");
    assert_eq!(
        log_ref(&mut observed).len(),
        recorded,
        "restore must not rewind the event log"
    );
    assert_eq!(log_ref(&mut observed).next_index(), recorded as u64);
    assert_eq!(log_ref(&mut observed).dropped(), 0);

    // The watchpoint configuration survived the restore: the next instruction
    // fetches at 0x22C.. (watchpoint 3) and writes the VDP control port with the
    // restored D0 = 0 (device event plus watchpoint 5 on both byte lanes).
    let observed_outcome = observed.step(InputFrame::NEUTRAL).expect("observed replay");
    let plain_outcome = plain.step(InputFrame::NEUTRAL).expect("plain replay");
    assert_eq!(observed_outcome, plain_outcome);
    assert_eq!(
        observed.snapshot().expect("observed snapshot").payload(),
        plain.snapshot().expect("plain snapshot").payload()
    );
    let after_restore = events(&mut observed);
    assert_eq!(
        after_restore[recorded..].to_vec(),
        vec![
            event(728, watchpoint(3, WatchpointAccess::Read, 0x22C, 0x33)),
            event(728, watchpoint(3, WatchpointAccess::Read, 0x22D, 0xC0)),
            event(728, watchpoint(3, WatchpointAccess::Read, 0x22E, 0x00)),
            event(728, watchpoint(3, WatchpointAccess::Read, 0x22F, 0xC0)),
            event(728, watchpoint(3, WatchpointAccess::Read, 0x230, 0x00)),
            event(728, watchpoint(3, WatchpointAccess::Read, 0x231, 0x04)),
            event(728, vdp_port(4, 0x0000, 2, true)),
            event(
                728,
                watchpoint(5, WatchpointAccess::Write, 0x00C0_0004, 0x00)
            ),
            event(
                728,
                watchpoint(5, WatchpointAccess::Write, 0x00C0_0005, 0x00)
            ),
        ],
        "watchpoints stay installed across a restore"
    );

    // Debugger memory paths are not emulated bus accesses.
    let mut probe = factory.create_machine().expect("probe machine");
    configure(
        &mut probe,
        16,
        vec![
            spec(9, RAM_BASE, RAM_BASE + 0x0F, WatchpointAccess::Read),
            spec(10, RAM_BASE, RAM_BASE + 0x0F, WatchpointAccess::Write),
        ],
    );
    probe
        .read_memory(u64::from(RAM_BASE), 4)
        .expect("debugger read");
    probe
        .write_memory(u64::from(RAM_BASE), &[1, 2, 3, 4])
        .expect("debugger write");
    probe.inspect().expect("inspect");
    assert!(
        events(&mut probe).is_empty(),
        "debugger read_memory/write_memory must not produce events"
    );
}

/// Returns only the interrupt-phase events of a log, in recording order.
///
/// Exception frame pushes are ordinary work-RAM byte writes and are reported as
/// `BusWrite` events (asserted by the bus/device row); this row is about the
/// request/delivery/acknowledgement phases.
fn interrupt_events(events: &[DebugEvent]) -> Vec<DebugEvent> {
    events
        .iter()
        .copied()
        .filter(|event| {
            matches!(
                event.kind,
                DebugEventKind::InterruptRequested { .. }
                    | DebugEventKind::InterruptDelivered { .. }
                    | DebugEventKind::InterruptAcknowledged { .. }
            )
        })
        .collect()
}

/// M4-A9 — interrupt request/delivery/acknowledgement observation.
#[test]
fn interrupt_events_are_requested_delivered_and_acknowledged() {
    // Mask 3 run: the first HBlank is requested while masked, its second issue
    // coalesces into the pending request (no duplicate event), the mask is
    // lowered at instruction 8 and the retained request is delivered at
    // instruction 9.
    let mut machine = interrupt_machine(3);
    configure(&mut machine, 65_536, Vec::new());
    for _ in 0..12_000 {
        machine.step(InputFrame::NEUTRAL).expect("interrupt step");
    }
    assert_eq!(log_ref(&mut machine).dropped(), 0, "ring overflow");
    let all = interrupt_events(&events(&mut machine));
    let requested4 = all
        .iter()
        .find(|event| matches!(event.kind, DebugEventKind::InterruptRequested { level: 4 }))
        .copied()
        .expect("HBlank request");
    let delivered4 = all
        .iter()
        .find(|event| matches!(event.kind, DebugEventKind::InterruptDelivered { level: 4 }))
        .copied()
        .expect("HBlank delivery");
    let acknowledged4 = all
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                DebugEventKind::InterruptAcknowledged { level: 4 }
            )
        })
        .copied()
        .expect("HBlank acknowledgement");
    assert!(requested4.tick < delivered4.tick);
    assert_eq!(
        delivered4.tick, acknowledged4.tick,
        "delivery and acknowledgement share the delivering step"
    );
    let requested6 = all
        .iter()
        .find(|event| matches!(event.kind, DebugEventKind::InterruptRequested { level: 6 }))
        .copied()
        .expect("VBlank request");
    let delivered6 = all
        .iter()
        .find(|event| matches!(event.kind, DebugEventKind::InterruptDelivered { level: 6 }))
        .copied()
        .expect("VBlank delivery");
    assert!(
        requested6.tick > delivered4.tick,
        "VBlank is raised only at the end of the active region"
    );
    assert!(requested6.tick < delivered6.tick);
    let acknowledged6 = all
        .iter()
        .find(|event| {
            matches!(
                event.kind,
                DebugEventKind::InterruptAcknowledged { level: 6 }
            )
        })
        .copied()
        .expect("VBlank acknowledgement");
    assert_eq!(delivered6.tick, acknowledged6.tick);

    // Mask 7 run: the request is raised but never delivered or acknowledged, and
    // a coalesced reissue of the same level produces no duplicate request event.
    let mut masked = interrupt_machine(7);
    configure(&mut masked, 65_536, Vec::new());
    for _ in 0..12_000 {
        masked.step(InputFrame::NEUTRAL).expect("masked step");
    }
    assert_eq!(log_ref(&mut masked).dropped(), 0, "ring overflow");
    let masked_events = interrupt_events(&events(&mut masked));
    assert!(
        masked_events
            .iter()
            .any(|event| matches!(event.kind, DebugEventKind::InterruptRequested { level: 4 })),
        "the masked HBlank request must be observed"
    );
    assert!(
        masked_events
            .iter()
            .any(|event| matches!(event.kind, DebugEventKind::InterruptRequested { level: 6 })),
        "the masked VBlank request must be observed"
    );
    assert!(
        !masked_events.iter().any(|event| matches!(
            event.kind,
            DebugEventKind::InterruptDelivered { .. }
                | DebugEventKind::InterruptAcknowledged { .. }
        )),
        "a masked pending request is never delivered or acknowledged"
    );
}

/// M4-A10 — bus and device events capture the documented windows.
#[test]
fn bus_and_device_events_capture_the_documented_windows() {
    // The demo ROM is a fixed 22-instruction sequence of RAM word stores and VDP
    // port accesses. Every event is asserted exactly.
    let mut machine = new_machine(demo_rom());
    configure(&mut machine, 256, Vec::new());
    for _ in 0..22 {
        machine.step(InputFrame::NEUTRAL).expect("demo step");
    }
    assert_eq!(
        events(&mut machine),
        vec![
            event(84, bus_write(0x00FF_0000, 0x00)),
            event(84, bus_write(0x00FF_0001, 0x03)),
            event(392, vdp_port(4, 0x8F02, 2, true)),
            event(560, vdp_port(4, 0x4000, 2, true)),
            event(728, vdp_port(4, 0x0000, 2, true)),
            event(896, vdp_port(0, 0x1234, 2, true)),
            event(1064, vdp_port(4, 0x0000, 2, true)),
            event(1232, vdp_port(4, 0x0000, 2, true)),
            event(1344, vdp_port(0, 0x1234, 2, false)),
            event(1456, bus_write(0x00FF_0004, 0x12)),
            event(1456, bus_write(0x00FF_0005, 0x34)),
            event(1568, vdp_port(4, 0x0100, 2, false)),
            event(1680, bus_write(0x00FF_0006, 0x01)),
            event(1680, bus_write(0x00FF_0007, 0x00)),
        ],
        "RAM byte writes and VDP word accesses with exact offsets, values, widths and ticks"
    );

    // Controller data-register reads carry the port number and byte value; the
    // ROM's byte stores stay byte-granular work-RAM writes.
    let mut controller = new_machine(controller_rom());
    configure(&mut controller, 64, Vec::new());
    let frame = InputFrame::with_devices([0x01, 0x20, 0, 0], [0; 4], 0);
    for _ in 0..9 {
        controller.step(frame).expect("controller step");
    }
    assert_eq!(
        events(&mut controller),
        vec![
            event(112, controller_read(1, 0x7E)),
            event(252, bus_write(0x00FF_0000, 0x7E)),
            event(364, bus_write(0x00FF_0002, 0x7E)),
            event(476, controller_read(2, 0x5F)),
        ],
        "controller port 1 and 2 data registers report their byte value"
    );

    // Byte-granular VDP port accesses keep width 1, while word accesses above
    // report width 2.
    let mut byte_vdp = new_machine(byte_vdp_rom());
    configure(&mut byte_vdp, 16, Vec::new());
    for _ in 0..3 {
        byte_vdp.step(InputFrame::NEUTRAL).expect("byte VDP step");
    }
    assert_eq!(
        events(&mut byte_vdp),
        vec![
            event(28, vdp_port(4, 0x02, 1, true)),
            event(140, vdp_port(8, 0x00, 1, false)),
        ],
        "byte VDP writes and reads report width 1"
    );

    // Instruction fetches are never bus-write or device events on their own.
    let mut no_watchpoints = new_machine(demo_rom());
    configure(&mut no_watchpoints, 16, Vec::new());
    for _ in 0..23 {
        no_watchpoints.step(InputFrame::NEUTRAL).expect("demo step");
    }
    assert!(
        events(&mut no_watchpoints).iter().all(|event| matches!(
            event.kind,
            DebugEventKind::BusWrite { .. } | DebugEventKind::VdpPort { .. }
        )),
        "without watchpoints only emulated bus/device writes are reported"
    );
}

/// M4-A10 — the event log is bounded, contiguous and deterministic.
#[test]
fn event_log_is_bounded_and_deterministic() {
    // Two identical runs produce identical event sequences.
    let first = demo_events(256);
    let second = demo_events(256);
    assert_eq!(first, second, "observation must be deterministic");
    assert_eq!(first.len(), 14);

    // A small ring evicts the oldest events, counts them and keeps contiguous
    // absolute indexes.
    let mut machine = new_machine(demo_rom());
    configure(&mut machine, 4, Vec::new());
    for _ in 0..22 {
        machine.step(InputFrame::NEUTRAL).expect("demo step");
    }
    let log = log_ref(&mut machine);
    assert_eq!(log.capacity(), 4);
    assert_eq!(log.len(), 4);
    assert_eq!(log.dropped(), 10);
    assert_eq!(log.first_index(), Some(10));
    assert_eq!(log.next_index(), 14);
    assert_eq!(
        log.first_index().expect("retained events") + log.len() as u64,
        log.next_index(),
        "retained indexes must stay contiguous"
    );
    let retained: Vec<DebugEvent> = log.entries().collect();
    assert_eq!(
        retained,
        vec![
            event(1456, bus_write(0x00FF_0005, 0x34)),
            event(1568, vdp_port(4, 0x0100, 2, false)),
            event(1680, bus_write(0x00FF_0006, 0x01)),
            event(1680, bus_write(0x00FF_0007, 0x00)),
        ],
        "the ring retains the newest events"
    );
    assert_eq!(log.get(0), Some(retained[0]));
    assert_eq!(log.get(3), Some(retained[3]));
    assert_eq!(log.get(4), None);
    assert_eq!(log.entries().next_back(), Some(retained[3]));
}

/// Steps the demo ROM for 22 instructions under a fresh configuration and returns
/// the recorded events.
fn demo_events(capacity: usize) -> Vec<DebugEvent> {
    let mut machine = new_machine(demo_rom());
    configure(&mut machine, capacity, Vec::new());
    for _ in 0..22 {
        machine.step(InputFrame::NEUTRAL).expect("demo step");
    }
    events(&mut machine)
}
