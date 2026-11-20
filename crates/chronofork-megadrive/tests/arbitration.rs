//! Deterministic 68000/Z80 bus arbitration and its master-cycle stall.

use std::error::Error;

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::bus::{Z80_BUS_REQUEST, Z80_RESET, Z80_STALL_MASTER_CYCLES};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::{MegadriveBackend, MegadriveBus, MegadriveFactory};

const STACK: u32 = 0x00FF_FF00;

/// `MOVE.W #$0100,$00A11100.L` requests the bus.
const REQUEST_BUS: [u8; 8] = [0x33, 0xFC, 0x01, 0x00, 0x00, 0xA1, 0x11, 0x00];
/// `MOVE.W #$0100,$00A11200.L` releases the Z80 reset (active-low bit 8).
const RELEASE_RESET: [u8; 8] = [0x33, 0xFC, 0x01, 0x00, 0x00, 0xA1, 0x12, 0x00];
/// `MOVE.W $00A00000.L,D0` reads the 68000-visible Z80 window.
const LOAD_Z80_WORD: [u8; 6] = [0x30, 0x39, 0x00, 0xA0, 0x00, 0x00];
/// `MOVE.W $00FF0000.L,D0` reads work RAM with the same encoding shape.
const LOAD_WORK_WORD: [u8; 6] = [0x30, 0x39, 0x00, 0xFF, 0x00, 0x00];

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
fn request_release_and_reset_are_idempotent() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    assert!(!bus.arbitration().requested());
    assert!(
        bus.arbitration().reset_asserted(),
        "power-on holds Z80 reset"
    );
    assert!(!bus.arbitration().z80_runs());

    bus.set_arbitration(true, false);
    let granted = bus.arbitration();
    bus.set_arbitration(true, false);
    assert_eq!(bus.arbitration(), granted);
    assert!(granted.requested());
    assert!(!granted.reset_asserted());
    assert!(granted.z80_runs());

    bus.set_arbitration(false, true);
    let released = bus.arbitration();
    bus.set_arbitration(false, true);
    assert_eq!(bus.arbitration(), released);
    assert!(!released.z80_runs());

    // Register writes are idempotent too, and read bit 8 reports the latch.
    bus.write16(Z80_BUS_REQUEST, 0x0100)?;
    bus.write16(Z80_BUS_REQUEST, 0x0100)?;
    assert_eq!(bus.read16(Z80_BUS_REQUEST)?, 0x0100);
    assert!(bus.arbitration().requested());

    bus.write16(Z80_RESET, 0x0000)?;
    bus.write16(Z80_RESET, 0x0000)?;
    assert_eq!(bus.read16(Z80_RESET)?, 0x0000);
    assert!(bus.arbitration().reset_asserted());
    assert!(!bus.arbitration().z80_runs());

    bus.write16(Z80_RESET, 0x0100)?;
    assert_eq!(bus.read16(Z80_RESET)?, 0x0100);
    assert!(!bus.arbitration().reset_asserted());
    Ok(())
}

#[test]
fn z80_window_access_stalls_only_while_the_z80_owns_the_bus() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    // 68000 owns the bus: no stall.
    let _ = bus.read8(0x00A0_0000)?;
    assert_eq!(bus.take_z80_stall_cycles(), 0);

    bus.set_arbitration(true, false);
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

    // Reset asserted: the Z80 does not run, so the window does not stall.
    bus.set_arbitration(true, true);
    let _ = bus.read8(0x00A0_0000)?;
    assert_eq!(bus.take_z80_stall_cycles(), 0);

    // Control registers are not the RAM window and never stall.
    bus.set_arbitration(true, false);
    let _ = bus.read16(Z80_BUS_REQUEST)?;
    let _ = bus.read16(Z80_RESET)?;
    assert_eq!(bus.take_z80_stall_cycles(), 0);
    Ok(())
}

#[test]
fn z80_does_not_run_without_request_or_during_reset() -> Result<(), Box<dyn Error>> {
    let program = concatenate(&[&REQUEST_BUS, &RELEASE_RESET, &[0x4E, 0x71]]);
    let mut machine = machine(&program)?;
    let start = machine.tick().raw();
    let outcome = machine.advance_z80()?;
    assert_eq!(outcome.t_states, 0);
    assert_eq!(machine.tick().raw(), start);

    machine.step(InputFrame::NEUTRAL)?; // request bus
    assert!(machine.bus().arbitration().requested());
    assert!(machine.bus().arbitration().reset_asserted());
    let after_request = machine.tick().raw();
    assert_eq!(
        machine.advance_z80()?.t_states,
        0,
        "reset still holds the Z80"
    );
    assert_eq!(machine.tick().raw(), after_request);

    machine.step(InputFrame::NEUTRAL)?; // release reset
    assert!(machine.bus().arbitration().z80_runs());
    let before = machine.tick().raw();
    let outcome = machine.advance_z80()?;
    assert_eq!(outcome.t_states, 4, "Z80 RAM is zero-filled (NOP)");
    assert_eq!(outcome.master_cycles(), 60);
    assert_eq!(machine.tick().raw(), before + 60);
    Ok(())
}

#[test]
fn cpu_window_access_while_z80_owns_the_bus_stalls() -> Result<(), Box<dyn Error>> {
    let z80_program = concatenate(&[&REQUEST_BUS, &RELEASE_RESET, &LOAD_Z80_WORD]);
    let ram_program = concatenate(&[&REQUEST_BUS, &RELEASE_RESET, &LOAD_WORK_WORD]);
    let mut with_z80 = machine(&z80_program)?;
    let mut control = machine(&ram_program)?;
    for _ in 0..2 {
        with_z80.step(InputFrame::NEUTRAL)?;
        control.step(InputFrame::NEUTRAL)?;
    }
    assert!(with_z80.bus().arbitration().z80_runs());

    let before_z80 = with_z80.tick().raw();
    let before_control = control.tick().raw();
    let z80_outcome = with_z80.step(InputFrame::NEUTRAL)?;
    let control_outcome = control.step(InputFrame::NEUTRAL)?;
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
        let a = first.step(InputFrame::NEUTRAL)?;
        let b = second.step(InputFrame::NEUTRAL)?;
        assert_eq!(a.tick, b.tick);
        assert_eq!(a.state_hash, b.state_hash);
    }
    assert_eq!(first.tick(), second.tick());
    Ok(())
}

#[test]
fn backend_request_and_release_writes_are_idempotent() -> Result<(), Box<dyn Error>> {
    let program = concatenate(&[&REQUEST_BUS, &REQUEST_BUS, &RELEASE_RESET, &RELEASE_RESET]);
    let mut machine = machine(&program)?;
    machine.step(InputFrame::NEUTRAL)?;
    assert!(machine.bus().arbitration().requested());
    assert!(machine.bus().arbitration().reset_asserted());
    machine.step(InputFrame::NEUTRAL)?;
    assert!(machine.bus().arbitration().requested());
    machine.step(InputFrame::NEUTRAL)?;
    assert!(!machine.bus().arbitration().reset_asserted());
    machine.step(InputFrame::NEUTRAL)?;
    assert!(!machine.bus().arbitration().reset_asserted());
    assert!(machine.bus().arbitration().z80_runs());
    Ok(())
}
