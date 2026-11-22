//! Integration coverage for the ADR-0009 controller contract.
//!
//! These tests use only the public Mega Drive backend, factory and bus API. They
//! exercise the TH-high 3-button data-register subset of controller ports 1 and 2
//! from the per-step `InputFrame`, prove that input is transient and never
//! canonical, and check that the independently authored [`controller_rom`]
//! observes both ports in work RAM. The adversarial snapshot/parser properties
//! stay in `tests/snapshot_properties.rs`.

use std::error::Error;

use chronofork_backend_api::{Backend, BackendError};
use chronofork_core::{InputFrame, hash_bytes};
use chronofork_megadrive::audio::psg::OPEN_BUS_VALUE;
use chronofork_megadrive::{
    CONTROLLER_NEUTRAL, CONTROLLER_PORT_1, CONTROLLER_PORT_2, Cartridge, MegadriveBus,
    MegadriveFactory, controller_rom,
};

/// Test result type: `?` keeps the target free of unwrap/expect helpers and
/// explicit panics while the test harness still reports the failing error.
type TestResult = Result<(), Box<dyn Error>>;

fn controller_factory() -> Result<MegadriveFactory, BackendError> {
    MegadriveFactory::new(controller_rom())
}

fn controller_machine() -> Result<chronofork_megadrive::MegadriveBackend, BackendError> {
    controller_factory()?.create_machine()
}

/// Builds a ROM whose first instruction is the unimplemented `ILLEGAL` opcode,
/// followed by a controller port-1 read and a work-RAM store, so the step after
/// the failed one uses its own input frame.
fn illegal_then_controller_rom() -> Vec<u8> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x200); // 0x400 bytes of NOP fill
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x202].copy_from_slice(&[0x4A, 0xFC]); // ILLEGAL
    rom[0x202..0x208].copy_from_slice(&[0x10, 0x39, 0x00, 0xA1, 0x00, 0x03]); // MOVE.B (P1).L,D0
    rom[0x208..0x20E].copy_from_slice(&[0x13, 0xC0, 0x00, 0xFF, 0x00, 0x00]); // MOVE.B D0,(0xFF0000).L
    rom[0x20E..0x210].copy_from_slice(&[0x60, 0xFE]); // BRA.S self
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the controller image holds a header"
    );
    rom
}

#[test]
fn idle_bus_reports_neutral_controller_bytes_and_rejects_debugger_access() -> TestResult {
    let mut bus = MegadriveBus::new(Cartridge::parse(controller_rom())?);
    assert_eq!(bus.read8(CONTROLLER_PORT_1)?, CONTROLLER_NEUTRAL);
    assert_eq!(bus.read8(CONTROLLER_PORT_2)?, CONTROLLER_NEUTRAL);

    // The side-effect-free debugger path still refuses the step-scoped data
    // registers: a per-step input value is not a machine register.
    assert!(bus.peek8(CONTROLLER_PORT_1).is_err());
    assert!(bus.read_memory(u64::from(CONTROLLER_PORT_1), 1).is_err());
    // A write to a data register is acknowledged and discarded, as on hardware:
    // the hardware has no latch there that can fail, so the value is unchanged.
    bus.write8(CONTROLLER_PORT_1, 0x00)?;
    bus.write8(CONTROLLER_PORT_2, 0x00)?;
    assert_eq!(bus.read8(CONTROLLER_PORT_1)?, CONTROLLER_NEUTRAL);
    assert_eq!(bus.read8(CONTROLLER_PORT_2)?, CONTROLLER_NEUTRAL);
    // The port 1/2/3/EXT control file is a readable, writable stub and the
    // undecoded rest of the window is open bus.
    assert_eq!(bus.read8(0x00A1_0008)?, 0x00);
    bus.write8(0x00A1_0008, 0x5A)?;
    assert_eq!(bus.read8(0x00A1_0008)?, 0x5A);
    assert_eq!(bus.read8(0x00A1_0000)?, OPEN_BUS_VALUE);
    Ok(())
}

#[test]
fn controller_rom_observes_per_bit_and_neutral_values() -> TestResult {
    // Every documented TH-high button bit, neutral, A/Start-only masks and a
    // mask with high bits set. A and Start are accepted but have no effect.
    let cases = [
        (0x0000_u32, 0x7F_u8), // neutral
        (0x0001, 0x7E),        // Up
        (0x0002, 0x7D),        // Down
        (0x0004, 0x7B),        // Left
        (0x0008, 0x77),        // Right
        (0x0010, 0x6F),        // B
        (0x0020, 0x5F),        // C
        (0x0040, 0x7F),        // A (not readable)
        (0x0080, 0x7F),        // Start (not readable)
        (0x00C0, 0x7F),        // A + Start
        (0x0101, 0x7E),        // high bit ignored, Up pressed
        (0x003F, 0x40),        // every mapped button pressed
    ];
    for (mask, expected) in cases {
        // Eight steps reach the first port-1 store and the last-observation
        // store, so both the accumulator and the observed byte land in RAM.
        let mut machine = controller_machine()?;
        for _ in 0..8 {
            machine.step(InputFrame::new(mask))?;
        }
        let inspection = machine.inspect()?;
        assert_eq!(
            inspection.memory[0], expected,
            "port-1 accumulator for {mask:#06X}"
        );
        assert_eq!(
            inspection.memory[2], expected,
            "port-1 observation for {mask:#06X}"
        );
        assert_eq!(inspection.memory[1], 0, "port 2 not read yet");
        assert_eq!(inspection.memory[3], 0, "port 2 not read yet");
        assert_eq!(inspection.registers["d0"], u64::from(expected));
        assert_eq!(inspection.registers["d2"], u64::from(expected));
    }
    Ok(())
}

#[test]
fn two_controller_ports_are_independent_and_inert_fields_are_ignored() -> TestResult {
    // Twelve steps complete one loop iteration: both ports are read, accumulated
    // and stored at 0xFF0000..=0xFF0003.
    let first = InputFrame::with_devices([0x01, 0x20, 0, 0], [0; 4], 0);
    let mut machine = controller_machine()?;
    for _ in 0..12 {
        machine.step(first)?;
    }
    let inspection = machine.inspect()?;
    assert_eq!(
        &inspection.memory[0..4],
        &[0x7E, 0x5F, 0x7E, 0x5F],
        "port 1 and port 2 use independent masks"
    );
    let first_hash = inspection.state_hash;

    // Swapping the masks swaps the observed ports.
    let mut swapped = controller_machine()?;
    for _ in 0..12 {
        swapped.step(InputFrame::with_devices([0x20, 0x01, 0, 0], [0; 4], 0))?;
    }
    let swapped_inspection = swapped.inspect()?;
    assert_eq!(&swapped_inspection.memory[0..4], &[0x5F, 0x7E, 0x5F, 0x7E]);
    assert_ne!(swapped_inspection.state_hash, first_hash);

    // Ports 2/3, axes and keys are recorded by movies but inert on this backend:
    // they must not be reinterpreted as buttons or hardware.
    let mut inert = controller_machine()?;
    for _ in 0..12 {
        inert.step(InputFrame::with_devices(
            [0x01, 0x20, u32::MAX, u32::MAX],
            [i16::MIN, -1, 1, i16::MAX],
            u64::MAX,
        ))?;
    }
    let inert_inspection = inert.inspect()?;
    assert_eq!(&inert_inspection.memory[0..4], &[0x7E, 0x5F, 0x7E, 0x5F]);
    assert_eq!(inert_inspection.state_hash, first_hash);
    Ok(())
}

#[test]
fn controller_step_uses_its_own_frame_without_leaking_the_previous_one() -> TestResult {
    let pressed = InputFrame::new(0x01);
    let mut machine = controller_machine()?;
    for _ in 0..12 {
        machine.step(pressed)?;
    }
    let pressed_inspection = machine.inspect()?;
    assert_eq!(pressed_inspection.memory[2], 0x7E);
    assert_eq!(pressed_inspection.registers["d2"], u64::from(0x7E_u8));

    // The next loop iteration branches and reads port 1 again. That read happens
    // in a step whose own frame is neutral, so it must observe 0x7F rather than
    // the previous step's pressed value.
    machine.step(InputFrame::NEUTRAL)?; // BRA.S
    machine.step(InputFrame::NEUTRAL)?; // MOVE.B (0xA10003).L,D2
    assert_eq!(
        machine.inspect()?.registers["d2"],
        u64::from(CONTROLLER_NEUTRAL),
        "the read must use the current step's neutral frame"
    );
    machine.step(InputFrame::NEUTRAL)?; // ADD.B D2,D0
    machine.step(InputFrame::NEUTRAL)?; // store accumulator
    machine.step(InputFrame::NEUTRAL)?; // store observed port-1 byte
    assert_eq!(machine.inspect()?.memory[2], CONTROLLER_NEUTRAL);

    // The port-2 observation later in the same iteration is neutral as well.
    machine.step(InputFrame::NEUTRAL)?; // MOVE.B (0xA10005).L,D3
    assert_eq!(
        machine.inspect()?.registers["d3"],
        u64::from(CONTROLLER_NEUTRAL)
    );
    Ok(())
}

#[test]
fn controller_input_is_absent_from_snapshots_and_restore_uses_the_next_frame() -> TestResult {
    let frame = InputFrame::with_devices([0x01, 0x20, 0, 0], [0; 4], 0);
    let mut pressed = controller_machine()?;
    let mut neutral = controller_machine()?;
    // Four steps only clear registers, so the checkpoint is input-independent.
    for _ in 0..4 {
        pressed.step(frame)?;
        neutral.step(InputFrame::NEUTRAL)?;
    }
    let pressed_snapshot = pressed.snapshot()?;
    let neutral_snapshot = neutral.snapshot()?;
    assert_eq!(
        hash_bytes(pressed_snapshot.payload()),
        hash_bytes(neutral_snapshot.payload()),
        "controller input is not serialized"
    );
    assert_eq!(
        pressed_snapshot.payload().len(),
        neutral_snapshot.payload().len()
    );
    assert_eq!(pressed_snapshot.state_hash(), neutral_snapshot.state_hash());
    assert_eq!(
        pressed_snapshot.config_hash(),
        neutral_snapshot.config_hash()
    );

    // Once the loop reads the controller, the same frame changes canonical state.
    let pressed_outcome = pressed.step(frame)?;
    let neutral_outcome = neutral.step(InputFrame::NEUTRAL)?;
    assert_ne!(pressed_outcome.state_hash, neutral_outcome.state_hash);

    // Replaying the recorded frames from the checkpoint reproduces every tick and
    // state hash; no input was carried in the checkpoint. The first recorded
    // frame is the controller-reading step above.
    let mut expected = vec![(pressed_outcome.tick, pressed_outcome.state_hash)];
    for _ in 0..8 {
        let outcome = pressed.step(frame)?;
        expected.push((outcome.tick, outcome.state_hash));
    }
    let mut restored = controller_machine()?;
    for _ in 0..4 {
        restored.step(InputFrame::NEUTRAL)?;
    }
    restored.restore(&pressed_snapshot)?;
    for (tick, state_hash) in &expected {
        let outcome = restored.step(frame)?;
        assert_eq!(outcome.tick, *tick);
        assert_eq!(outcome.state_hash, *state_hash);
    }

    // Restoring the same checkpoint and stepping neutral diverges at the first
    // controller read, so the machine used the next frame rather than a latch.
    let mut restored_neutral = controller_machine()?;
    for _ in 0..4 {
        restored_neutral.step(InputFrame::NEUTRAL)?;
    }
    restored_neutral.restore(&pressed_snapshot)?;
    assert_ne!(
        restored_neutral.step(InputFrame::NEUTRAL)?.state_hash,
        expected[0].1
    );
    Ok(())
}

#[test]
fn failed_step_does_not_leak_input_into_the_next_step() -> TestResult {
    let factory = MegadriveFactory::new(illegal_then_controller_rom())?;
    let mut machine = factory.create_machine()?;
    // The first step fails on ILLEGAL after consuming the pressed frame.
    assert!(matches!(
        machine.step(InputFrame::new(0x3F)),
        Err(BackendError::Execution(_))
    ));
    // The next two steps read the controller and store the observed byte. If the
    // failed step's input leaked, the store would be 0x40 instead of 0x7F.
    machine.step(InputFrame::NEUTRAL)?;
    machine.step(InputFrame::NEUTRAL)?;
    let inspection = machine.inspect()?;
    assert_eq!(inspection.memory[0], CONTROLLER_NEUTRAL);

    // The debugger still cannot read the step-scoped controller window.
    assert!(matches!(
        machine.read_memory(u64::from(CONTROLLER_PORT_1), 1),
        Err(BackendError::Execution(_))
    ));

    // The failed step leaves canonical state identical to a neutral-input
    // failure: the input was transient, not part of the machine.
    let mut neutral = factory.create_machine()?;
    assert!(matches!(
        neutral.step(InputFrame::NEUTRAL),
        Err(BackendError::Execution(_))
    ));
    neutral.step(InputFrame::NEUTRAL)?;
    neutral.step(InputFrame::NEUTRAL)?;
    assert_eq!(hash_bytes(machine.snapshot()?.payload()), {
        let snapshot = neutral.snapshot()?;
        hash_bytes(snapshot.payload())
    });
    assert_eq!(
        machine.snapshot()?.state_hash(),
        neutral.snapshot()?.state_hash()
    );
    Ok(())
}
