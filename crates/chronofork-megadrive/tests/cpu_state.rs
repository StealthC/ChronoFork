//! Register-file behaviour of the 68000 core: supervisor and user stacks.

use chronofork_megadrive::cpu::M68000;

#[test]
fn supervisor_and_user_stacks_are_separate() {
    let mut cpu = M68000::new();
    cpu.set_sr(0x2700);
    cpu.set_a7(0x00FF_0000); // supervisor stack
    cpu.set_sr(0x0700); // switch to user mode
    cpu.set_a7(0x0000_1000); // user stack
    assert_eq!(cpu.usp(), 0x0000_1000);
    assert_eq!(cpu.ssp(), 0x00FF_0000);
    cpu.set_sr(0x2700);
    assert_eq!(cpu.a7(), 0x00FF_0000);
}

#[test]
fn switching_modes_preserves_the_inactive_stack_pointer() {
    let mut cpu = M68000::new();
    cpu.set_sr(0x2700);
    cpu.set_a7(0x00FF_0000);
    cpu.set_sr(0x0700);
    cpu.set_a7(0x0000_1000);
    cpu.set_sr(0x2700);
    cpu.set_a7(0x00FE_0000);
    // The supervisor's A7 was saved into SSP, not into USP.
    assert_eq!(cpu.ssp(), 0x00FE_0000);
    assert_eq!(cpu.usp(), 0x0000_1000);
    cpu.set_sr(0x0700);
    assert_eq!(cpu.a7(), 0x0000_1000);
    assert_eq!(
        cpu.sr() & 0x2000,
        0,
        "user mode has the supervisor bit clear"
    );
}

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{Cartridge, MegadriveBus};

const STACK: u32 = 0x00FF_FF00;

fn machine(program: &[u8]) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    Ok((cpu, bus))
}

#[test]
fn instruction_writes_to_a7_update_the_active_stack_mirror() -> Result<(), Box<dyn Error>> {
    // MOVEA.L #$00FF2000,A7; MOVE.W D0,-(A7); ADDQ.L #1,A7; LEA $00FF3000.L,A7
    let (mut cpu, mut bus) = machine(&[
        0x2E, 0x7C, 0x00, 0xFF, 0x20, 0x00, 0x3F, 0x00, 0x52, 0x8F, 0x4F, 0xF9, 0x00, 0xFF, 0x30,
        0x00,
    ])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.a7(), 0x00FF_2000);
    assert_eq!(cpu.ssp(), 0x00FF_2000, "MOVEA.L #imm,A7 updates SSP");
    assert_eq!(cpu.usp(), 0, "the inactive USP is unchanged");

    cpu.step(&mut bus)?;
    assert_eq!(cpu.a7(), 0x00FF_1FFE, "predecrement updates the mirror");
    assert_eq!(cpu.ssp(), 0x00FF_1FFE);

    cpu.step(&mut bus)?;
    assert_eq!(cpu.a7(), 0x00FF_1FFF, "ADDQ.L to A7 updates the mirror");
    assert_eq!(cpu.ssp(), 0x00FF_1FFF);

    cpu.step(&mut bus)?;
    assert_eq!(cpu.a7(), 0x00FF_3000, "LEA to A7 updates the mirror");
    assert_eq!(cpu.ssp(), 0x00FF_3000);
    assert_eq!(cpu.usp(), 0);
    Ok(())
}

#[test]
fn user_mode_a7_write_updates_usp_and_leaves_ssp() -> Result<(), Box<dyn Error>> {
    // MOVE.W #$0700,SR; MOVEA.L #$00FF3000,A7
    let (mut cpu, mut bus) =
        machine(&[0x46, 0xFC, 0x07, 0x00, 0x2E, 0x7C, 0x00, 0xFF, 0x30, 0x00])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr() & 0x2000, 0, "user mode");
    cpu.step(&mut bus)?;
    assert_eq!(cpu.a7(), 0x00FF_3000);
    assert_eq!(cpu.usp(), 0x00FF_3000, "user A7 writes update USP");
    assert_eq!(cpu.ssp(), STACK, "SSP is unchanged");
    Ok(())
}
