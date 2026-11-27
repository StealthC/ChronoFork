//! Deterministic 68000/Z80 bus arbitration and its master-cycle stall.

use std::error::Error;

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::bus::{Z80_BUS_REQUEST, Z80_RESET, Z80_STALL_MASTER_CYCLES};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::{MegadriveBackend, MegadriveBus, MegadriveFactory};

const STACK: u32 = 0x00FF_FF00;

/// `MOVE.W #$0100,$00A11100.L` asserts the Z80 bus request.
const REQUEST_BUS: [u8; 8] = [0x33, 0xFC, 0x01, 0x00, 0x00, 0xA1, 0x11, 0x00];
/// `MOVE.W #$0000,$00A11100.L` releases the Z80 bus request.
const RELEASE_BUS: [u8; 8] = [0x33, 0xFC, 0x00, 0x00, 0x00, 0xA1, 0x11, 0x00];
/// `MOVE.W #$0100,$00A11200.L` releases the Z80 reset (active-low bit 8).
const RELEASE_RESET: [u8; 8] = [0x33, 0xFC, 0x01, 0x00, 0x00, 0xA1, 0x12, 0x00];
/// `MOVE.W $00A00000.L,D0` reads the 68000-visible Z80 window.
const LOAD_Z80_WORD: [u8; 6] = [0x30, 0x39, 0x00, 0xA0, 0x00, 0x00];
/// `MOVE.W $00FF0000.L,D0` reads work RAM with the same encoding shape.
const LOAD_WORK_WORD: [u8; 6] = [0x30, 0x39, 0x00, 0xFF, 0x00, 0x00];
/// The hardware bus-request handshake: read `0xA11100`, mask bit 8, loop until
/// the Z80 has yielded.
///
/// `MOVE.W $00A11100.L,D0` / `ANDI.W #$0100,D0` / `BNE.S` back to the first
/// instruction. The branch displacement is `-12`: the loop body is ten bytes and
/// the displacement is relative to the word after the branch opcode.
const POLL_BUS_REQUEST: [u8; 12] = [
    0x30, 0x39, 0x00, 0xA1, 0x11, 0x00, // MOVE.W $00A11100.L,D0
    0x02, 0x40, 0x01, 0x00, // ANDI.W #$0100,D0
    0x66, 0xF4, // BNE.S poll
];

/// Builds a 16 KiB ROM with reset vectors, the program at `0x200`, and NOP fill.
fn rom(program: &[u8]) -> Vec<u8> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x2000);
    rom[0..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the test image holds a header"
    );
    rom
}

fn bus() -> Result<MegadriveBus, Box<dyn Error>> {
    let mut image = vec![0_u8; 0x4000];
    assert!(chronofork_megadrive::test_support::stamp_header(
        &mut image, "JUE"
    ));
    Ok(MegadriveBus::new(Cartridge::parse(image)?))
}

fn machine(program: &[u8]) -> Result<MegadriveBackend, Box<dyn Error>> {
    Ok(MegadriveFactory::new(rom(program))?.create_machine()?)
}

fn concatenate(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|part| part.iter().copied()).collect()
}

#[test]
fn request_polarity_matches_the_hardware_zbr_line() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    bus.write16(Z80_RESET, 0x0100)?;
    assert!(bus.arbitration().z80_runs());
    assert!(
        bus.arbitration().z80_owns_bus(),
        "the Z80 owns the bus while no request is asserted"
    );
    assert_eq!(
        bus.read16(Z80_BUS_REQUEST)?,
        0x0100,
        "0xA11100 bit 8 reads 1 while the Z80 still owns the bus"
    );

    // Asserting the request stops the Z80 and hands the window to the 68000.
    bus.write16(Z80_BUS_REQUEST, 0x0100)?;
    assert!(!bus.arbitration().z80_runs(), "the request stops the Z80");
    assert!(!bus.arbitration().z80_owns_bus());
    assert_eq!(
        bus.read16(Z80_BUS_REQUEST)?,
        0x0000,
        "0xA11100 bit 8 reads 0 once the Z80 has yielded"
    );

    // Releasing the request returns the bus to the Z80.
    bus.write16(Z80_BUS_REQUEST, 0x0000)?;
    assert!(bus.arbitration().z80_runs());
    assert_eq!(bus.read16(Z80_BUS_REQUEST)?, 0x0100);
    Ok(())
}

#[test]
fn request_release_and_reset_are_idempotent() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    assert!(!bus.arbitration().requested(), "no bus request at power-on");
    assert!(
        bus.arbitration().reset_asserted(),
        "power-on holds Z80 reset"
    );
    assert!(!bus.arbitration().z80_runs());
    assert!(
        bus.arbitration().z80_owns_bus(),
        "the Z80 owns the bus until the 68000 requests it"
    );

    bus.write16(Z80_BUS_REQUEST, 0x0100)?;
    bus.write16(Z80_BUS_REQUEST, 0x0100)?;
    assert!(bus.arbitration().requested());
    assert!(!bus.arbitration().z80_runs(), "the request stops the Z80");
    assert_eq!(bus.read16(Z80_BUS_REQUEST)?, 0x0000);

    bus.write16(Z80_RESET, 0x0000)?;
    bus.write16(Z80_RESET, 0x0000)?;
    assert_eq!(bus.read16(Z80_RESET)?, 0x0000);
    assert!(bus.arbitration().reset_asserted());
    assert!(!bus.arbitration().z80_runs());

    // Out of reset but with the request asserted the Z80 still does not run.
    bus.write16(Z80_BUS_REQUEST, 0x0000)?;
    bus.write16(Z80_RESET, 0x0100)?;
    assert_eq!(bus.read16(Z80_RESET)?, 0x0100);
    assert!(!bus.arbitration().reset_asserted());
    assert!(bus.arbitration().z80_runs());
    Ok(())
}

#[test]
fn z80_window_access_stalls_only_while_the_z80_owns_the_bus() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    // Power-on: reset asserted, the Z80 does not run, so the window does not stall.
    let _ = bus.read8(0x00A0_0000)?;
    assert_eq!(bus.take_z80_stall_cycles(), 0);

    // Out of reset with no request: the Z80 owns the bus, so the 68000 stalls.
    bus.write16(Z80_RESET, 0x0100)?;
    let _ = bus.read8(0x00A0_0000)?;
    assert_eq!(bus.take_z80_stall_cycles(), Z80_STALL_MASTER_CYCLES);
    let _ = bus.read16(0x00A0_0000)?;
    assert_eq!(
        bus.take_z80_stall_cycles(),
        2 * Z80_STALL_MASTER_CYCLES,
        "a word access stalls per byte lane"
    );
    bus.write8(0x00A0_0001, 0x00)?;
    assert_eq!(bus.take_z80_stall_cycles(), Z80_STALL_MASTER_CYCLES);

    // Request asserted: the 68000 owns the window, so the window does not stall.
    bus.write16(Z80_BUS_REQUEST, 0x0100)?;
    let _ = bus.read8(0x00A0_0000)?;
    assert_eq!(bus.take_z80_stall_cycles(), 0);

    // Control registers are not the RAM window and never stall.
    let _ = bus.read16(Z80_BUS_REQUEST)?;
    let _ = bus.read16(Z80_RESET)?;
    assert_eq!(bus.take_z80_stall_cycles(), 0);
    Ok(())
}

#[test]
fn z80_does_not_run_while_requested_or_during_reset() -> Result<(), Box<dyn Error>> {
    let program = concatenate(&[&RELEASE_RESET, &REQUEST_BUS, &[0x4E, 0x71]]);
    let mut machine = machine(&program)?;
    let start = machine.tick().raw();
    assert_eq!(
        machine.advance_z80()?.t_states,
        0,
        "power-on reset holds the Z80"
    );
    assert_eq!(machine.tick().raw(), start);

    machine.step(InputFrame::NEUTRAL)?; // release reset: the Z80 owns the bus
    assert!(!machine.bus().arbitration().reset_asserted());
    assert!(machine.bus().arbitration().z80_runs());
    let before = machine.tick().raw();
    let outcome = machine.advance_z80()?;
    assert_eq!(outcome.t_states, 4, "Z80 RAM is zero-filled (NOP)");
    assert_eq!(outcome.master_cycles(), 60);
    assert_eq!(machine.tick().raw(), before + 60);

    machine.step(InputFrame::NEUTRAL)?; // request the bus: the Z80 stops
    assert!(machine.bus().arbitration().requested());
    let after_request = machine.tick().raw();
    assert_eq!(
        machine.advance_z80()?.t_states,
        0,
        "the asserted bus request stops the Z80"
    );
    assert_eq!(machine.tick().raw(), after_request);
    Ok(())
}

#[test]
fn bus_request_handshake_polls_until_the_z80_yields() -> Result<(), Box<dyn Error>> {
    let program = concatenate(&[
        &REQUEST_BUS,
        &POLL_BUS_REQUEST,
        &LOAD_Z80_WORD,
        &RELEASE_RESET,
        &RELEASE_BUS,
    ]);
    let mut machine = machine(&program)?;

    machine.step(InputFrame::NEUTRAL)?; // assert the request
    assert!(machine.bus().arbitration().requested());
    assert_eq!(
        machine.bus().peek8(Z80_BUS_REQUEST)?,
        0x00,
        "the poll observes the yield"
    );
    assert!(!machine.bus().arbitration().z80_runs());

    // The poll loop (MOVE.W, ANDI.W, BNE.S) falls through instead of spinning.
    for _ in 0..3 {
        machine.step(InputFrame::NEUTRAL)?;
    }
    assert!(
        machine.bus().arbitration().requested(),
        "the handshake has not released the bus yet"
    );

    machine.step(InputFrame::NEUTRAL)?; // read the Z80 window with the 68000 owning it
    assert!(
        !machine.bus().arbitration().z80_runs(),
        "the Z80 is still held in reset"
    );

    machine.step(InputFrame::NEUTRAL)?; // release the Z80 reset
    assert!(!machine.bus().arbitration().reset_asserted());
    assert!(
        !machine.bus().arbitration().z80_runs(),
        "the asserted request still holds the bus"
    );

    machine.step(InputFrame::NEUTRAL)?; // release the request
    assert!(!machine.bus().arbitration().requested());
    assert_eq!(machine.bus().peek8(Z80_BUS_REQUEST)?, 0x01);
    assert!(
        machine.bus().arbitration().z80_runs(),
        "the Z80 owns the bus again"
    );
    Ok(())
}

#[test]
fn cpu_window_access_while_z80_owns_the_bus_stalls() -> Result<(), Box<dyn Error>> {
    let z80_program = concatenate(&[&RELEASE_RESET, &LOAD_Z80_WORD]);
    let ram_program = concatenate(&[&RELEASE_RESET, &LOAD_WORK_WORD]);
    let mut with_z80 = machine(&z80_program)?;
    let mut control = machine(&ram_program)?;
    with_z80.step(InputFrame::NEUTRAL)?;
    control.step(InputFrame::NEUTRAL)?;
    assert!(with_z80.bus().arbitration().z80_runs());

    let before_z80 = with_z80.tick().raw();
    let before_control = control.tick().raw();
    // The assertion compares digests, so it uses the verified path (W2-2).
    let z80_outcome = with_z80.step_verified(InputFrame::NEUTRAL)?;
    let control_outcome = control.step_verified(InputFrame::NEUTRAL)?;
    let z80_delta = with_z80.tick().raw() - before_z80;
    let control_delta = control.tick().raw() - before_control;
    assert_eq!(
        z80_delta,
        control_delta + 2 * Z80_STALL_MASTER_CYCLES,
        "the word access pays one documented stall per byte lane"
    );
    assert_ne!(
        z80_outcome.state_hash, control_outcome.state_hash,
        "the stall changes the master-cycle tick and therefore the state hash"
    );
    Ok(())
}

#[test]
fn arbitration_is_deterministic_across_identical_runs() -> Result<(), Box<dyn Error>> {
    let program = concatenate(&[&REQUEST_BUS, &RELEASE_RESET, &LOAD_Z80_WORD]);
    let mut first = machine(&program)?;
    let mut second = machine(&program)?;
    for _ in 0..3 {
        // W2-2: ordinary outcomes carry no digest, so determinism is compared through
        // the verified path; comparing two `None`s would pass trivially.
        let a = first.step_verified(InputFrame::NEUTRAL)?;
        let b = second.step_verified(InputFrame::NEUTRAL)?;
        assert_eq!(a.tick, b.tick);
        assert_eq!(a.state_hash, b.state_hash);
    }
    assert_eq!(first.tick(), second.tick());
    Ok(())
}

#[test]
fn backend_request_and_release_writes_are_idempotent() -> Result<(), Box<dyn Error>> {
    let program = concatenate(&[
        &REQUEST_BUS,
        &REQUEST_BUS,
        &RELEASE_RESET,
        &RELEASE_RESET,
        &RELEASE_BUS,
    ]);
    let mut machine = machine(&program)?;
    machine.step(InputFrame::NEUTRAL)?;
    assert!(machine.bus().arbitration().requested());
    machine.step(InputFrame::NEUTRAL)?;
    assert!(machine.bus().arbitration().requested());
    assert!(machine.bus().arbitration().reset_asserted());
    machine.step(InputFrame::NEUTRAL)?;
    assert!(!machine.bus().arbitration().reset_asserted());
    assert!(
        !machine.bus().arbitration().z80_runs(),
        "the asserted request still holds the bus"
    );
    machine.step(InputFrame::NEUTRAL)?;
    assert!(!machine.bus().arbitration().reset_asserted());
    assert!(!machine.bus().arbitration().z80_runs());
    machine.step(InputFrame::NEUTRAL)?;
    assert!(!machine.bus().arbitration().requested());
    assert!(machine.bus().arbitration().z80_runs());
    Ok(())
}
