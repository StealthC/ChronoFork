#![allow(clippy::expect_used)]

//! Acceptance tests for the host debugger (ADR-0010 §3.2).
//!
//! The rows implemented here are M4-A1, M4-A2, M4-A3, the host half of M4-A4,
//! the host half of M4-A5 and M4-A11 in
//! `docs/quality/m4-debugging-depth-acceptance.md`. Breakpoints and logpoints
//! are exercised on the synthetic backend; watchpoints and event breakpoints use
//! the independently authored Mega Drive fixture ROMs (`demo_rom`,
//! `interrupt_rom`, `controller_rom`), never a commercial image. Every assertion
//! is exact: stop reasons, step counts, ticks, state hashes, events, log entries,
//! dropped counts and typed errors are compared against the documented rules.

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAddressRange,
    DebugEvent, DebugEventKind, MachineInspection, StepOutcome, WatchpointAccess, WatchpointSpec,
};
use chronofork_core::{InputFrame, Tick};
use chronofork_debug::{
    BreakpointId, BreakpointSpec, Condition, ConditionOperand, DEFAULT_LOG_CAPACITY, DebugError,
    DebugRun, Debugger, EventFilter, InterruptPhase, InvalidLogMessage, Location, LogEntry,
    MAX_BREAKPOINTS, MAX_EVENT_BREAKPOINTS, MAX_EVENT_CAPACITY, MAX_LOG_MESSAGE_BYTES,
    MAX_LOGPOINTS, MAX_RUN_STEP_LIMIT, MAX_WATCHPOINTS, Relation, StopReason, TriggerId,
    TriggerKind,
};
use chronofork_megadrive::{
    MegadriveBackend, MegadriveFactory, SchedulerConfig, controller_rom, demo_rom, interrupt_rom,
};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

/// First work-RAM byte of the Mega Drive slice.
const RAM_BASE: u32 = 0x00FF_0000;
/// Synthetic instruction boundary where `STORE_RAM` executes.
const SYNTHETIC_STORE_PC: u32 = 4;
/// Synthetic PC the default program's `JMP` loops back to.
const SYNTHETIC_LOOP_PC: u32 = 0;
/// Mega Drive PC where the demo ROM stores the computed word to work RAM.
const DEMO_STORE_PC: u32 = 0x206;
/// Mega Drive PC immediately after that store.
const DEMO_AFTER_STORE_PC: u32 = 0x20C;

fn synthetic_machine() -> Box<dyn Backend> {
    SyntheticBackendFactory::default()
        .create()
        .expect("synthetic machine")
}

fn megadrive_machine(rom: Vec<u8>) -> MegadriveBackend {
    MegadriveFactory::new(rom)
        .expect("valid fixture ROM")
        .create_machine()
        .expect("Mega Drive machine")
}

fn interrupt_machine(mask: u8) -> MegadriveBackend {
    let scheduler = SchedulerConfig::ntsc();
    MegadriveFactory::with_scheduler_config(interrupt_rom(mask), scheduler)
        .expect("valid interrupt fixture ROM")
        .create_machine()
        .expect("interrupt machine")
}

fn synthetic_after(steps: u64) -> MachineInspection {
    let mut machine = synthetic_machine();
    for _ in 0..steps {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    machine.inspect().expect("inspect")
}

fn megadrive_after(rom: Vec<u8>, steps: u64) -> MachineInspection {
    let mut machine = megadrive_machine(rom);
    for _ in 0..steps {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    machine.inspect().expect("inspect")
}

fn interrupt_after(mask: u8, steps: u64) -> MachineInspection {
    let mut machine = interrupt_machine(mask);
    for _ in 0..steps {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    machine.inspect().expect("inspect")
}

fn watchpoint_spec(id: u32, start: u32, end: u32, access: WatchpointAccess) -> WatchpointSpec {
    WatchpointSpec::new(
        id,
        DebugAddressRange::new(start, end).expect("valid inclusive range"),
        access,
    )
    .expect("valid watchpoint spec")
}

fn event(tick: u64, kind: DebugEventKind) -> DebugEvent {
    DebugEvent {
        tick: Tick::from_raw(tick),
        kind,
    }
}

fn d0_condition(relation: Relation, value: u64) -> Condition {
    Condition {
        operand: ConditionOperand::Register("d0".to_owned()),
        relation,
        value,
    }
}

/// Runs one conditional breakpoint on a fresh demo ROM machine.
fn conditional_run(
    location: Location,
    condition: Condition,
    step_limit: u64,
) -> (DebugRun, BreakpointId) {
    let mut machine = megadrive_machine(demo_rom());
    let mut debugger = Debugger::new(&mut machine).expect("debugger");
    let id = debugger
        .add_breakpoint(BreakpointSpec {
            location,
            condition: Some(condition),
        })
        .expect("breakpoint");
    let run = debugger.run(step_limit).expect("run");
    (run, id)
}

/// M4-A1 — a PC breakpoint stops before the instruction executes.
///
/// The stop state equals a fresh inspection at the same instruction boundary,
/// not the state after the instruction, the step count is exact, and two
/// identical runs report identical values.
#[test]
fn pc_breakpoint_stops_before_the_instruction() {
    let mut machine = synthetic_machine();
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    let breakpoint = debugger
        .add_breakpoint(BreakpointSpec {
            location: Location::Address(SYNTHETIC_STORE_PC),
            condition: None,
        })
        .expect("breakpoint");
    let run = debugger.run(10).expect("run");

    assert_eq!(run.reason, StopReason::Breakpoint { id: breakpoint });
    assert_eq!(run.steps, 3, "three instructions execute before the store");
    assert_eq!(
        run.tick,
        Tick::from_raw(3),
        "the stop is a pre-step boundary"
    );
    assert!(!run.halted);
    assert!(run.matched_events.is_empty());
    assert!(run.log_entries.is_empty());
    assert_eq!(run.dropped_events, 0);
    assert_eq!(run.dropped_log_entries, 0);

    let boundary = synthetic_after(3);
    assert_eq!(
        run.tick, boundary.tick,
        "the reported tick is the boundary inspection, not the state after the step"
    );
    assert_eq!(run.state_hash, boundary.state_hash);
    let inspection = debugger.inspect().expect("inspect");
    assert_eq!(inspection.registers["pc"], u64::from(SYNTHETIC_STORE_PC));
    assert_eq!(inspection.tick, run.tick);
    assert_eq!(inspection.state_hash, run.state_hash);
    assert_eq!(
        debugger.read_memory(0x20, 1).expect("read ram"),
        vec![0],
        "the store at pc 4 must not have executed"
    );

    let after_the_instruction = synthetic_after(4);
    assert_ne!(
        run.state_hash, after_the_instruction.state_hash,
        "the reported state must not be the state after executing the instruction"
    );
    assert_eq!(after_the_instruction.registers["pc"], 6);

    let mut repeated_machine = synthetic_machine();
    let mut repeated_debugger = Debugger::new(&mut *repeated_machine).expect("debugger");
    repeated_debugger
        .add_breakpoint(BreakpointSpec {
            location: Location::Address(SYNTHETIC_STORE_PC),
            condition: None,
        })
        .expect("breakpoint");
    let repeated = repeated_debugger.run(10).expect("run");
    assert_eq!(
        repeated, run,
        "an identical run must report identical values"
    );
}

/// M4-A2 — a range breakpoint matches its inclusive bounds and misses outside.
#[test]
fn range_breakpoint_matches_inclusive_bounds() {
    // The lower bound is inclusive: the first PC inside {3, 4} is 3.
    let mut machine = synthetic_machine();
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    let breakpoint = debugger
        .add_breakpoint(BreakpointSpec {
            location: Location::Range { start: 3, end: 4 },
            condition: None,
        })
        .expect("breakpoint");
    let run = debugger.run(10).expect("run");
    assert_eq!(run.reason, StopReason::Breakpoint { id: breakpoint });
    assert_eq!(run.steps, 2);
    assert_eq!(debugger.inspect().expect("inspect").registers["pc"], 3);

    // The upper bound is inclusive: {2, 3} first matches at the end address 3,
    // which an exclusive range would skip.
    let mut machine = synthetic_machine();
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    let breakpoint = debugger
        .add_breakpoint(BreakpointSpec {
            location: Location::Range { start: 2, end: 3 },
            condition: None,
        })
        .expect("breakpoint");
    let run = debugger.run(10).expect("run");
    assert_eq!(run.reason, StopReason::Breakpoint { id: breakpoint });
    assert_eq!(run.steps, 2);
    assert_eq!(debugger.inspect().expect("inspect").registers["pc"], 3);

    // A range that contains no instruction boundary misses and reaches the step
    // limit; the PC jumps from 4 to 6, so 5 is never a boundary.
    let mut machine = synthetic_machine();
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    debugger
        .add_breakpoint(BreakpointSpec {
            location: Location::Range { start: 5, end: 5 },
            condition: None,
        })
        .expect("breakpoint");
    let run = debugger.run(3).expect("run");
    assert_eq!(run.reason, StopReason::StepLimit);
    assert_eq!(run.steps, 3);
    assert_eq!(debugger.inspect().expect("inspect").registers["pc"], 4);
}

/// M4-A2 — register and bounded big-endian memory conditions, ANDed with the
/// location; a condition that never holds reaches `StepLimit`.
#[test]
fn conditional_breakpoint_requires_register_and_memory_conditions() {
    let at_store = Location::Address(DEMO_STORE_PC);
    // `d0` is 3 at the store (MOVEQ #1; MOVEQ #2; ADD.W D1,D0), so every
    // relation has one exact hit and one exact miss at the same boundary.
    let cases = [
        (Relation::Equal, 3, true),
        (Relation::Equal, 4, false),
        (Relation::NotEqual, 3, false),
        (Relation::NotEqual, 2, true),
        (Relation::Less, 4, true),
        (Relation::Less, 3, false),
        (Relation::LessOrEqual, 3, true),
        (Relation::LessOrEqual, 2, false),
        (Relation::Greater, 2, true),
        (Relation::Greater, 3, false),
        (Relation::GreaterOrEqual, 3, true),
        (Relation::GreaterOrEqual, 4, false),
    ];
    for (relation, value, hits) in cases {
        let (run, breakpoint) = conditional_run(at_store, d0_condition(relation, value), 6);
        if hits {
            assert_eq!(
                run.reason,
                StopReason::Breakpoint { id: breakpoint },
                "{relation:?} {value} must stop at the store"
            );
            assert_eq!(run.steps, 3);
        } else {
            assert_eq!(
                run.reason,
                StopReason::StepLimit,
                "{relation:?} {value} must not stop at the store"
            );
            assert_eq!(run.steps, 6);
        }
    }

    // The store writes the big-endian word 0x0003 at 0x00FF0000; the breakpoint
    // at the following instruction sees those two bytes.
    let memory = |value: u64| Condition {
        operand: ConditionOperand::Memory {
            address: RAM_BASE,
            length: 2,
        },
        relation: Relation::Equal,
        value,
    };
    let (run, breakpoint) = conditional_run(Location::Address(0x214), memory(0x0003), 8);
    assert_eq!(run.reason, StopReason::Breakpoint { id: breakpoint });
    assert_eq!(run.steps, 6);

    // A little-endian reading of 0x0003 would be 0x0300 and must not match.
    let (run, _) = conditional_run(Location::Address(0x214), memory(0x0300), 8);
    assert_eq!(run.reason, StopReason::StepLimit);
    assert_eq!(run.steps, 8);
}

/// M4-A3 — trigger tables are bounded, removable and enable/disable works for
/// every trigger kind.
#[test]
fn trigger_tables_are_bounded_and_removable() {
    // Ids are allocated per trigger kind, so each table starts at one.
    {
        let mut machine = synthetic_machine();
        let mut debugger = Debugger::new(&mut *machine).expect("debugger");
        let breakpoint = debugger
            .add_breakpoint(BreakpointSpec {
                location: Location::Address(SYNTHETIC_LOOP_PC),
                condition: None,
            })
            .expect("breakpoint");
        let logpoint = debugger
            .add_logpoint(Location::Address(SYNTHETIC_LOOP_PC), "loop")
            .expect("logpoint");
        let event_breakpoint = debugger
            .add_event_breakpoint(EventFilter::Interrupt {
                level: None,
                phase: InterruptPhase::Delivered,
            })
            .expect("event breakpoint");
        assert_eq!(breakpoint.value(), 1);
        assert_eq!(logpoint.value(), 1);
        assert_eq!(event_breakpoint.value(), 1);

        assert_eq!(debugger.triggers().breakpoints().len(), 1);
        assert_eq!(debugger.triggers().logpoints().len(), 1);
        assert_eq!(debugger.triggers().event_breakpoints().len(), 1);
        assert!(debugger.triggers().watchpoints().len() == 0);

        // A disabled trigger is still installed but is not evaluated.
        debugger
            .set_enabled(TriggerId::Breakpoint(breakpoint), false)
            .expect("disable breakpoint");
        assert!(
            !debugger
                .triggers()
                .breakpoints()
                .next()
                .expect("breakpoint")
                .2
        );
        debugger
            .set_enabled(TriggerId::Logpoint(logpoint), false)
            .expect("disable logpoint");
        debugger
            .set_enabled(TriggerId::EventBreakpoint(event_breakpoint), false)
            .expect("disable event breakpoint");
        assert!(!debugger.triggers().logpoints().next().expect("logpoint").2);
        assert!(
            !debugger
                .triggers()
                .event_breakpoints()
                .next()
                .expect("event breakpoint")
                .2
        );
        // Disabled triggers are not silently evaluated: a breakpoint and a
        // logpoint at the current PC would stop and record when enabled.
        let run = debugger.run(1).expect("run");
        assert_eq!(run.reason, StopReason::StepLimit);
        assert_eq!(run.steps, 1);
        assert!(run.log_entries.is_empty());
        debugger
            .set_enabled(TriggerId::Breakpoint(breakpoint), true)
            .expect("enable breakpoint");

        debugger
            .remove_breakpoint(breakpoint)
            .expect("remove breakpoint");
        debugger.remove_logpoint(logpoint).expect("remove logpoint");
        debugger
            .remove_event_breakpoint(event_breakpoint)
            .expect("remove event breakpoint");
        assert!(debugger.triggers().breakpoints().len() == 0);
        assert!(debugger.triggers().logpoints().len() == 0);
        assert!(debugger.triggers().event_breakpoints().len() == 0);
        assert_eq!(
            debugger.remove_breakpoint(breakpoint),
            Err(DebugError::UnknownTrigger(TriggerId::Breakpoint(
                breakpoint
            )))
        );
        assert_eq!(
            debugger.remove_logpoint(logpoint),
            Err(DebugError::UnknownTrigger(TriggerId::Logpoint(logpoint)))
        );
        assert_eq!(
            debugger.remove_event_breakpoint(event_breakpoint),
            Err(DebugError::UnknownTrigger(TriggerId::EventBreakpoint(
                event_breakpoint
            )))
        );
        assert_eq!(
            debugger.set_enabled(TriggerId::Breakpoint(breakpoint), true),
            Err(DebugError::UnknownTrigger(TriggerId::Breakpoint(
                breakpoint
            )))
        );
    }

    // Watchpoint lifecycle on the Mega Drive slice.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let watchpoint = debugger
            .add_watchpoint(watchpoint_spec(
                0x10,
                RAM_BASE,
                RAM_BASE,
                WatchpointAccess::Write,
            ))
            .expect("watchpoint");
        assert_eq!(watchpoint.value(), 1);
        assert_eq!(debugger.triggers().watchpoints().len(), 1);
        debugger
            .set_enabled(TriggerId::Watchpoint(watchpoint), false)
            .expect("disable watchpoint");
        assert!(
            !debugger
                .triggers()
                .watchpoints()
                .next()
                .expect("watchpoint")
                .2
        );
        debugger
            .set_enabled(TriggerId::Watchpoint(watchpoint), true)
            .expect("enable watchpoint");
        debugger
            .remove_watchpoint(watchpoint)
            .expect("remove watchpoint");
        assert_eq!(
            debugger.remove_watchpoint(watchpoint),
            Err(DebugError::UnknownTrigger(TriggerId::Watchpoint(
                watchpoint
            )))
        );
        assert_eq!(
            debugger.set_enabled(TriggerId::Watchpoint(watchpoint), false),
            Err(DebugError::UnknownTrigger(TriggerId::Watchpoint(
                watchpoint
            )))
        );
    }

    // Every table rejects the entry after its documented maximum.
    {
        let mut machine = synthetic_machine();
        let mut debugger = Debugger::new(&mut *machine).expect("debugger");
        for _ in 0..MAX_BREAKPOINTS {
            debugger
                .add_breakpoint(BreakpointSpec {
                    location: Location::Address(SYNTHETIC_LOOP_PC),
                    condition: None,
                })
                .expect("breakpoint");
        }
        assert_eq!(
            debugger.add_breakpoint(BreakpointSpec {
                location: Location::Address(SYNTHETIC_LOOP_PC),
                condition: None,
            }),
            Err(DebugError::TableFull(TriggerKind::Breakpoint))
        );
        assert_eq!(debugger.triggers().breakpoints().len(), MAX_BREAKPOINTS);

        for _ in 0..MAX_LOGPOINTS {
            debugger
                .add_logpoint(Location::Address(SYNTHETIC_LOOP_PC), "loop")
                .expect("logpoint");
        }
        assert_eq!(
            debugger.add_logpoint(Location::Address(SYNTHETIC_LOOP_PC), "loop"),
            Err(DebugError::TableFull(TriggerKind::Logpoint))
        );
        assert_eq!(debugger.triggers().logpoints().len(), MAX_LOGPOINTS);

        for _ in 0..MAX_EVENT_BREAKPOINTS {
            debugger
                .add_event_breakpoint(EventFilter::BusWrite { range: None })
                .expect("event breakpoint");
        }
        assert_eq!(
            debugger.add_event_breakpoint(EventFilter::BusWrite { range: None }),
            Err(DebugError::TableFull(TriggerKind::EventBreakpoint))
        );
        assert_eq!(
            debugger.triggers().event_breakpoints().len(),
            MAX_EVENT_BREAKPOINTS
        );
    }
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        for id in 1..=MAX_WATCHPOINTS {
            let id = u32::try_from(id).expect("fits in u32");
            debugger
                .add_watchpoint(watchpoint_spec(
                    id,
                    RAM_BASE + id,
                    RAM_BASE + id,
                    WatchpointAccess::Write,
                ))
                .expect("watchpoint");
        }
        assert_eq!(
            debugger.add_watchpoint(watchpoint_spec(
                0x40,
                RAM_BASE,
                RAM_BASE,
                WatchpointAccess::Write
            )),
            Err(DebugError::TableFull(TriggerKind::Watchpoint))
        );
        assert_eq!(debugger.triggers().watchpoints().len(), MAX_WATCHPOINTS);
    }

    // A duplicate backend watchpoint id is a typed error, not a silent replace.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        debugger
            .add_watchpoint(watchpoint_spec(
                0x2A,
                RAM_BASE,
                RAM_BASE,
                WatchpointAccess::Write,
            ))
            .expect("watchpoint");
        assert_eq!(
            debugger.add_watchpoint(watchpoint_spec(
                0x2A,
                RAM_BASE + 2,
                RAM_BASE + 2,
                WatchpointAccess::Write
            )),
            Err(DebugError::DuplicateWatchpointId(0x2A))
        );
    }
}

/// M4-A3 — unknown registers, missing `pc` and invalid input are typed errors.
#[test]
fn unknown_register_and_missing_pc_are_typed_errors() {
    let mut machine = synthetic_machine();
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    assert_eq!(
        debugger.add_breakpoint(BreakpointSpec {
            location: Location::Address(SYNTHETIC_STORE_PC),
            condition: Some(d0_condition(Relation::Equal, 3)),
        }),
        Err(DebugError::UnknownRegister("d0".to_owned()))
    );
    assert_eq!(
        debugger.triggers().breakpoints().len(),
        0,
        "a rejected trigger must not be installed"
    );

    for length in [0_u8, 9] {
        assert_eq!(
            debugger.add_breakpoint(BreakpointSpec {
                location: Location::Address(SYNTHETIC_STORE_PC),
                condition: Some(Condition {
                    operand: ConditionOperand::Memory {
                        address: 0x20,
                        length,
                    },
                    relation: Relation::Equal,
                    value: 0,
                }),
            }),
            Err(DebugError::InvalidConditionLength(length))
        );
    }

    let inverted = Location::Range { start: 2, end: 1 };
    let range_error = DebugError::InvalidRange { start: 2, end: 1 };
    assert_eq!(
        debugger.add_breakpoint(BreakpointSpec {
            location: inverted,
            condition: None,
        }),
        Err(range_error.clone())
    );
    assert_eq!(debugger.add_logpoint(inverted, "never"), Err(range_error));
    assert_eq!(
        debugger.add_logpoint(Location::Address(0), ""),
        Err(DebugError::InvalidLogMessage(InvalidLogMessage::Empty))
    );
    assert_eq!(
        debugger.add_logpoint(Location::Address(0), &"x".repeat(MAX_LOG_MESSAGE_BYTES + 1)),
        Err(DebugError::InvalidLogMessage(InvalidLogMessage::TooLong {
            length: MAX_LOG_MESSAGE_BYTES + 1
        }))
    );
    assert_eq!(
        debugger.add_logpoint(Location::Address(0), "line\nbreak"),
        Err(DebugError::InvalidLogMessage(
            InvalidLogMessage::ContainsLineBreak
        ))
    );

    assert_eq!(debugger.run(0), Err(DebugError::InvalidStepLimit(0)));
    assert_eq!(
        debugger.run(MAX_RUN_STEP_LIMIT + 1),
        Err(DebugError::InvalidStepLimit(MAX_RUN_STEP_LIMIT + 1))
    );

    // Event capacity is validated independently of the backend.
    let mut machine = synthetic_machine();
    assert!(matches!(
        Debugger::with_event_capacity(&mut *machine, 0),
        Err(DebugError::InvalidEventCapacity(0))
    ));
    let mut machine = synthetic_machine();
    assert!(matches!(
        Debugger::with_event_capacity(&mut *machine, MAX_EVENT_CAPACITY + 1),
        Err(DebugError::InvalidEventCapacity(capacity))
            if capacity == MAX_EVENT_CAPACITY + 1
    ));

    // A backend whose register map lacks `pc` rejects every location trigger
    // with a typed error instead of panicking or ignoring the trigger.
    let mut machine: Box<dyn Backend> = Box::new(WithoutProgramCounter {
        inner: synthetic_machine(),
    });
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    assert!(
        !debugger
            .inspect()
            .expect("inspect")
            .registers
            .contains_key("pc")
    );
    assert_eq!(
        debugger.add_breakpoint(BreakpointSpec {
            location: Location::Address(SYNTHETIC_STORE_PC),
            condition: None,
        }),
        Err(DebugError::MissingProgramCounter)
    );
    assert_eq!(
        debugger.add_logpoint(Location::Address(SYNTHETIC_LOOP_PC), "tick"),
        Err(DebugError::MissingProgramCounter)
    );
    assert_eq!(
        debugger.triggers().breakpoints().len(),
        0,
        "a rejected trigger must not be installed"
    );
    // Without a location trigger the backend still steps normally.
    let run = debugger.run(1).expect("run without pc triggers");
    assert_eq!(run.reason, StopReason::StepLimit);
    assert_eq!(run.steps, 1);
}

/// M4-A4 — a host watchpoint stops after the instruction that performed the
/// access, and debugger memory reads never trigger it.
#[test]
fn watchpoint_stops_after_the_writing_instruction() {
    let mut machine = megadrive_machine(demo_rom());
    let mut debugger = Debugger::new(&mut machine).expect("debugger");
    let watchpoint = debugger
        .add_watchpoint(watchpoint_spec(
            0x10,
            RAM_BASE,
            RAM_BASE,
            WatchpointAccess::Write,
        ))
        .expect("watchpoint");
    let run = debugger.run(10).expect("run");

    assert_eq!(
        run.reason,
        StopReason::Watchpoint {
            ids: vec![watchpoint]
        },
        "the first watched write is the word store at 0x206"
    );
    assert_eq!(run.steps, 4, "the stopping instruction is the store");
    assert_eq!(
        run.tick,
        Tick::from_raw(196),
        "the stop is after the instruction"
    );
    assert!(!run.halted);
    assert!(run.matched_events.is_empty());

    let after = megadrive_after(demo_rom(), 4);
    assert_eq!(run.tick, after.tick);
    assert_eq!(
        run.state_hash, after.state_hash,
        "the stop state equals a fresh inspection after the writing instruction"
    );
    let inspection = debugger.inspect().expect("inspect");
    assert_eq!(inspection.registers["pc"], u64::from(DEMO_AFTER_STORE_PC));
    assert_eq!(
        debugger.read_memory(u64::from(RAM_BASE), 2).expect("read"),
        vec![0x00, 0x03],
        "the store executed before the stop"
    );

    // Debugger memory reads and writes are not emulated bus accesses.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        debugger
            .add_watchpoint(watchpoint_spec(
                0x11,
                RAM_BASE,
                RAM_BASE + 1,
                WatchpointAccess::Read,
            ))
            .expect("read watchpoint");
        debugger
            .add_watchpoint(watchpoint_spec(
                0x12,
                RAM_BASE,
                RAM_BASE + 1,
                WatchpointAccess::Write,
            ))
            .expect("write watchpoint");
        assert_eq!(
            debugger.read_memory(u64::from(RAM_BASE), 2).expect("read"),
            vec![0x00, 0x00]
        );
        debugger
            .write_memory(u64::from(RAM_BASE), &[0xAA, 0xBB])
            .expect("debugger write");
        assert_eq!(
            debugger.read_memory(u64::from(RAM_BASE), 2).expect("read"),
            vec![0xAA, 0xBB]
        );
        let run = debugger.run(1).expect("run");
        assert_eq!(
            run.reason,
            StopReason::StepLimit,
            "debugger memory paths must not fabricate a watchpoint stop"
        );
        assert_eq!(run.steps, 1);
    }

    // A watched write outside the range is ignored.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        debugger
            .add_watchpoint(watchpoint_spec(
                0x12,
                RAM_BASE + 2,
                RAM_BASE + 3,
                WatchpointAccess::Write,
            ))
            .expect("watchpoint");
        let run = debugger.run(10).expect("run");
        assert_eq!(run.reason, StopReason::StepLimit);
        assert_eq!(run.steps, 10);
    }

    // A read watchpoint stops after the load instruction at 0x20E.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let watchpoint = debugger
            .add_watchpoint(watchpoint_spec(
                0x13,
                RAM_BASE,
                RAM_BASE + 1,
                WatchpointAccess::Read,
            ))
            .expect("watchpoint");
        let run = debugger.run(10).expect("run");
        assert_eq!(
            run.reason,
            StopReason::Watchpoint {
                ids: vec![watchpoint]
            }
        );
        assert_eq!(run.steps, 6, "the stopping instruction is the word load");
        assert_eq!(run.tick, Tick::from_raw(336));
        assert_eq!(debugger.inspect().expect("inspect").registers["pc"], 0x214);
    }

    // A disabled watchpoint is not installed in the backend configuration and
    // never stops.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let watchpoint = debugger
            .add_watchpoint(watchpoint_spec(
                0x14,
                RAM_BASE,
                RAM_BASE,
                WatchpointAccess::Write,
            ))
            .expect("watchpoint");
        debugger
            .set_enabled(TriggerId::Watchpoint(watchpoint), false)
            .expect("disable watchpoint");
        let run = debugger.run(10).expect("run");
        assert_eq!(run.reason, StopReason::StepLimit);
        assert_eq!(run.steps, 10);
    }

    // A watchpoint stop wins over a matching event breakpoint of the same step,
    // and the matched events stay in the report.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let watchpoint = debugger
            .add_watchpoint(watchpoint_spec(
                0x15,
                RAM_BASE,
                RAM_BASE,
                WatchpointAccess::Write,
            ))
            .expect("watchpoint");
        debugger
            .add_event_breakpoint(EventFilter::BusWrite {
                range: Some(DebugAddressRange::new(RAM_BASE, RAM_BASE + 1).expect("valid range")),
            })
            .expect("event breakpoint");
        assert!(
            debugger
                .triggers()
                .event_breakpoints()
                .next()
                .expect("event breakpoint")
                .2,
            "the matching event breakpoint stays enabled and loses to the watchpoint"
        );
        let run = debugger.run(10).expect("run");
        assert_eq!(
            run.reason,
            StopReason::Watchpoint {
                ids: vec![watchpoint]
            }
        );
        assert_eq!(run.steps, 4);
        assert_eq!(
            run.matched_events,
            vec![
                event(
                    84,
                    DebugEventKind::BusWrite {
                        address: RAM_BASE,
                        value: 0x00,
                        width: 1,
                    }
                ),
                event(
                    84,
                    DebugEventKind::BusWrite {
                        address: RAM_BASE + 1,
                        value: 0x03,
                        width: 1,
                    }
                ),
            ],
            "the watchpoint stop wins but the matching events remain in the report"
        );
    }
}

/// M4-A5 — a backend without debug observation rejects watchpoints and event
/// reads with a typed error while host triggers keep working.
#[test]
fn watchpoints_require_backend_support() {
    let mut machine = synthetic_machine();
    let capabilities = machine.descriptor().capabilities();
    assert!(
        !capabilities.watchpoints,
        "the synthetic backend must not advertise watchpoints"
    );
    assert!(
        !capabilities.debug_events,
        "the synthetic backend must not advertise debug events"
    );

    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    assert_eq!(
        debugger.add_watchpoint(watchpoint_spec(1, 0, 1, WatchpointAccess::Write)),
        Err(DebugError::WatchpointsUnsupported)
    );
    assert_eq!(
        debugger.retained_events(),
        Err(DebugError::WatchpointsUnsupported)
    );
    assert!(
        debugger.triggers().watchpoints().len() == 0,
        "an unsupported watchpoint must not be installed"
    );

    // Host-only triggers still work on a backend without observation.
    let breakpoint = debugger
        .add_breakpoint(BreakpointSpec {
            location: Location::Address(SYNTHETIC_LOOP_PC),
            condition: None,
        })
        .expect("breakpoint");
    let run = debugger.run(1).expect("run");
    assert_eq!(run.reason, StopReason::Breakpoint { id: breakpoint });
    assert_eq!(run.steps, 0);
}

/// M4-A11 — an event breakpoint stops after the step that produced a matching
/// retained event, for every documented event class.
#[test]
fn event_breakpoint_stops_on_a_matching_event() {
    // Interrupt delivery: the first HBlank is delivered at tick 7350, so the
    // 105th instruction stops.
    let mut machine = interrupt_machine(3);
    let mut debugger = Debugger::new(&mut machine).expect("debugger");
    let event_breakpoint = debugger
        .add_event_breakpoint(EventFilter::Interrupt {
            level: Some(4),
            phase: InterruptPhase::Delivered,
        })
        .expect("event breakpoint");
    let run = debugger.run(200).expect("run");
    let delivered = event(7350, DebugEventKind::InterruptDelivered { level: 4 });
    assert_eq!(
        run.reason,
        StopReason::Event {
            id: event_breakpoint
        }
    );
    assert_eq!(run.steps, 105);
    assert_eq!(run.tick, Tick::from_raw(7378), "the stop is after the step");
    assert_eq!(run.state_hash, interrupt_after(3, 105).state_hash);
    assert_eq!(run.matched_events, vec![delivered]);
    assert!(
        debugger
            .retained_events()
            .expect("retained events")
            .contains(&delivered),
        "the matched event stays readable through retained_events"
    );

    // A VDP write filter stops on the first matching port write.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let event_breakpoint = debugger
            .add_event_breakpoint(EventFilter::Vdp {
                write: Some(true),
                offset: Some(4),
            })
            .expect("event breakpoint");
        let run = debugger.run(20).expect("run");
        assert_eq!(
            run.reason,
            StopReason::Event {
                id: event_breakpoint
            }
        );
        assert_eq!(run.steps, 8);
        assert_eq!(
            run.matched_events,
            vec![event(
                392,
                DebugEventKind::VdpPort {
                    offset: 4,
                    value: 0x8F02,
                    width: 2,
                    write: true,
                }
            )]
        );
    }

    // A controller-read filter stops on the emulated data-register read; the
    // neutral input frame returns 0x7F for the TH-high subset.
    {
        let mut machine = megadrive_machine(controller_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let event_breakpoint = debugger
            .add_event_breakpoint(EventFilter::ControllerRead { port: Some(1) })
            .expect("event breakpoint");
        let run = debugger.run(20).expect("run");
        assert_eq!(
            run.reason,
            StopReason::Event {
                id: event_breakpoint
            }
        );
        assert_eq!(run.steps, 5);
        assert_eq!(
            run.matched_events,
            vec![event(
                112,
                DebugEventKind::ControllerRead {
                    port: 1,
                    value: 0x7F
                }
            )]
        );
    }

    // A bus-write range filter reports every matching event of the step.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let event_breakpoint = debugger
            .add_event_breakpoint(EventFilter::BusWrite {
                range: Some(DebugAddressRange::new(RAM_BASE, RAM_BASE + 1).expect("valid range")),
            })
            .expect("event breakpoint");
        let run = debugger.run(20).expect("run");
        assert_eq!(
            run.reason,
            StopReason::Event {
                id: event_breakpoint
            }
        );
        assert_eq!(run.steps, 4);
        assert_eq!(
            run.matched_events,
            vec![
                event(
                    84,
                    DebugEventKind::BusWrite {
                        address: RAM_BASE,
                        value: 0x00,
                        width: 1,
                    }
                ),
                event(
                    84,
                    DebugEventKind::BusWrite {
                        address: RAM_BASE + 1,
                        value: 0x03,
                        width: 1,
                    }
                ),
            ]
        );
    }

    // When two enabled event breakpoints match the same step, the first one in
    // id order stops the run.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let first = debugger
            .add_event_breakpoint(EventFilter::BusWrite {
                range: Some(DebugAddressRange::new(RAM_BASE, RAM_BASE + 1).expect("valid range")),
            })
            .expect("first event breakpoint");
        debugger
            .add_event_breakpoint(EventFilter::BusWrite {
                range: Some(DebugAddressRange::new(RAM_BASE, RAM_BASE).expect("valid range")),
            })
            .expect("second event breakpoint");
        let run = debugger.run(20).expect("run");
        assert_eq!(run.reason, StopReason::Event { id: first });
        assert_eq!(run.steps, 4);
    }

    // A one-event ring retains the newest event only and reports the eviction.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::with_event_capacity(&mut machine, 1).expect("debugger");
        let event_breakpoint = debugger
            .add_event_breakpoint(EventFilter::BusWrite {
                range: Some(DebugAddressRange::new(RAM_BASE, RAM_BASE + 1).expect("valid range")),
            })
            .expect("event breakpoint");
        let run = debugger.run(10).expect("run");
        assert_eq!(
            run.reason,
            StopReason::Event {
                id: event_breakpoint
            }
        );
        assert_eq!(run.steps, 4);
        assert_eq!(run.dropped_events, 1, "the first byte event was evicted");
        assert_eq!(
            run.matched_events,
            vec![event(
                84,
                DebugEventKind::BusWrite {
                    address: RAM_BASE + 1,
                    value: 0x03,
                    width: 1,
                }
            )],
            "only the newest retained event can match"
        );
        assert_eq!(debugger.retained_events().expect("events").len(), 1);
    }

    // A filter that never matches reaches the step limit; a disabled matching
    // filter does not stop either.
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        debugger
            .add_event_breakpoint(EventFilter::Interrupt {
                level: Some(4),
                phase: InterruptPhase::Requested,
            })
            .expect("event breakpoint");
        let run = debugger.run(20).expect("run");
        assert_eq!(run.reason, StopReason::StepLimit);
        assert_eq!(run.steps, 20);
        assert!(run.matched_events.is_empty());
    }
    {
        let mut machine = megadrive_machine(demo_rom());
        let mut debugger = Debugger::new(&mut machine).expect("debugger");
        let event_breakpoint = debugger
            .add_event_breakpoint(EventFilter::Vdp {
                write: Some(true),
                offset: Some(4),
            })
            .expect("event breakpoint");
        debugger
            .set_enabled(TriggerId::EventBreakpoint(event_breakpoint), false)
            .expect("disable event breakpoint");
        let run = debugger.run(9).expect("run");
        assert_eq!(run.reason, StopReason::StepLimit);
        assert_eq!(run.steps, 9);
        assert!(run.matched_events.is_empty());
    }
}

/// M4-A11 — logpoints record every hit without stopping, and the bounded ring
/// evicts the oldest entries with a dropped count.
#[test]
fn logpoint_records_without_stopping() {
    // Two logpoints at repeating PCs record every hit in id order.
    {
        let mut machine = synthetic_machine();
        let mut debugger = Debugger::new(&mut *machine).expect("debugger");
        let loop_point = debugger
            .add_logpoint(Location::Address(SYNTHETIC_LOOP_PC), "loop")
            .expect("logpoint");
        let add_point = debugger
            .add_logpoint(Location::Address(1), "add")
            .expect("logpoint");
        let run = debugger.run(7).expect("run");
        assert_eq!(
            run.reason,
            StopReason::StepLimit,
            "a logpoint never changes the stop reason"
        );
        assert_eq!(run.steps, 7);
        assert_eq!(run.dropped_log_entries, 0);
        let expected = vec![
            LogEntry {
                logpoint_id: loop_point,
                tick: Tick::from_raw(0),
                pc: u64::from(SYNTHETIC_LOOP_PC),
                hits: 1,
                message: "loop".to_owned(),
            },
            LogEntry {
                logpoint_id: add_point,
                tick: Tick::from_raw(1),
                pc: 1,
                hits: 1,
                message: "add".to_owned(),
            },
            LogEntry {
                logpoint_id: loop_point,
                tick: Tick::from_raw(5),
                pc: u64::from(SYNTHETIC_LOOP_PC),
                hits: 2,
                message: "loop".to_owned(),
            },
            LogEntry {
                logpoint_id: add_point,
                tick: Tick::from_raw(6),
                pc: 1,
                hits: 2,
                message: "add".to_owned(),
            },
        ];
        assert_eq!(run.log_entries, expected);
        assert_eq!(debugger.log_entries(), expected.as_slice());
        assert_eq!(
            debugger
                .triggers()
                .logpoints()
                .map(|(_, _, enabled, hits)| (enabled, hits))
                .collect::<Vec<_>>(),
            vec![(true, 2), (true, 2)],
            "the hit count increments on every logpoint hit"
        );
    }

    // The ring evicts the oldest entries and counts them.
    {
        let mut machine = synthetic_machine();
        let mut debugger = Debugger::new(&mut *machine).expect("debugger");
        let loop_point = debugger
            .add_logpoint(Location::Address(SYNTHETIC_LOOP_PC), "loop")
            .expect("logpoint");
        let run = debugger.run(1300).expect("run");
        assert_eq!(run.reason, StopReason::StepLimit);
        assert_eq!(run.steps, 1300);
        assert_eq!(run.tick, Tick::from_raw(1300));
        assert_eq!(run.log_entries.len(), DEFAULT_LOG_CAPACITY);
        assert_eq!(
            run.dropped_log_entries,
            261 - DEFAULT_LOG_CAPACITY as u64,
            "261 loop-boundary hits fill the 256-entry ring"
        );
        let last = run.log_entries.last().expect("retained log entry");
        assert_eq!(last.logpoint_id, loop_point);
        assert_eq!(last.hits, 261, "the hit counter is not bounded by the ring");
        assert_eq!(last.pc, u64::from(SYNTHETIC_LOOP_PC));
        assert_eq!(last.tick, Tick::from_raw(1300));
        assert_eq!(debugger.log_entries().len(), DEFAULT_LOG_CAPACITY);
        assert_eq!(
            debugger.triggers().logpoints().next().expect("logpoint").3,
            261
        );
    }
}

/// M4-A1 — a halted machine stops with `Halted`, and a breakpoint at the current
/// PC wins over both `Halted` and `StepLimit`.
#[test]
fn halted_machine_stops_with_halted_and_breakpoint_wins() {
    let factory =
        SyntheticBackendFactory::new(SyntheticProgram::new(vec![0xFF]).expect("halting program"));
    let mut machine = factory.create().expect("synthetic machine");
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");

    // The halt takes one step; the next boundary reports `Halted`, not
    // `StepLimit`, even when the limit is already reached.
    let run = debugger.run(1).expect("run");
    assert_eq!(run.reason, StopReason::Halted);
    assert_eq!(run.steps, 1);
    assert_eq!(run.tick, Tick::from_raw(1));
    assert!(run.halted);
    assert!(run.matched_events.is_empty());

    // A breakpoint at the halted PC wins over `Halted` and `StepLimit`.
    let mut machine = factory.create().expect("synthetic machine");
    let mut debugger = Debugger::new(&mut *machine).expect("debugger");
    let breakpoint = debugger
        .add_breakpoint(BreakpointSpec {
            location: Location::Address(1),
            condition: None,
        })
        .expect("breakpoint");
    let run = debugger.run(1).expect("run");
    assert_eq!(
        run.reason,
        StopReason::Breakpoint { id: breakpoint },
        "a breakpoint at the current PC wins over Halted and StepLimit"
    );
    assert_eq!(run.steps, 1);
    assert!(
        run.halted,
        "the halted flag still describes the boundary state"
    );
}

/// Minimal backend wrapper that hides the `pc` register.
struct WithoutProgramCounter {
    inner: Box<dyn Backend>,
}

impl Backend for WithoutProgramCounter {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
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
        inspection.registers.remove("pc");
        Ok(inspection)
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        self.inner.read_memory(address, length)
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        self.inner.write_memory(address, data)
    }
}
