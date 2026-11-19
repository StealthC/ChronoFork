//! System instruction and privilege coverage.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{Cartridge, M68000, MegadriveBus};

const FLAG_V: u16 = 0x0002;
const STACK: u32 = 0x00FF_FF00;

fn machine(
    program: &[u8],
    vectors: &[(u8, u32)],
) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    for (vector, target) in vectors {
        let offset = usize::from(*vector) * 4;
        rom[offset..offset + 4].copy_from_slice(&target.to_be_bytes());
    }
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    Ok((cpu, bus))
}

fn flag(cpu: &M68000, flag: u16) -> bool {
    cpu.sr() & flag != 0
}

#[test]
fn privileged_write_sr_faults_in_user_mode() -> Result<(), Box<dyn Error>> {
    // MOVE.W #$0000,SR in user mode; vector 8 points at 0x400.
    let (mut cpu, mut bus) = machine(&[0x46, 0xFC, 0x00, 0x00], &[(8, 0x400)])?;
    cpu.set_sr(0x0700);
    cpu.set_a7(STACK);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    assert_ne!(cpu.sr() & 0x2000, 0, "exception entry sets S");
    assert_eq!(bus.read16(STACK - 6)?, 0x0700, "pre-exception SR");
    assert_eq!(bus.read32(STACK - 4)?, 0x202, "PC after the opcode word");
    Ok(())
}

#[test]
fn move_sr_and_ccr_forms_work_in_supervisor_mode() -> Result<(), Box<dyn Error>> {
    // MOVE.W #$2508,SR; MOVE from SR to D0; MOVE.W #$35,CCR; MOVE from CCR D1.
    let (mut cpu, mut bus) = machine(
        &[
            0x46, 0xFC, 0x25, 0x08, // MOVE.W #$2508,SR
            0x40, 0xC0, // MOVE from SR,D0
            0x44, 0xFC, 0x00, 0x35, // MOVE.W #$35,CCR
            0x42, 0xC1, // MOVE from CCR,D1
        ],
        &[],
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr(), 0x2508);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_2508));
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr(), 0x2535, "upper SR bits survive a CCR write");
    cpu.set_sr(0x2705);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0005));
    Ok(())
}

#[test]
fn stop_halts_until_an_interrupt() -> Result<(), Box<dyn Error>> {
    // STOP #$2700, with vector 6 at 0x78.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x72, 0x27, 0x00], &[(30, 0x300)])?;
    cpu.step(&mut bus)?;
    assert!(cpu.halted());
    assert!(cpu.stopped());
    assert_eq!(cpu.sr(), 0x2700);
    cpu.interrupt(6, 0x78, &mut bus)?;
    assert!(!cpu.halted());
    assert_eq!(cpu.pc(), 0x300);
    Ok(())
}

#[test]
fn rtr_restores_ccr_then_program_counter() -> Result<(), Box<dyn Error>> {
    // RTR with CCR $35 and return PC $00000300 on the stack.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x77], &[])?;
    cpu.set_a7(STACK);
    bus.write16(STACK, 0x0035)?;
    bus.write32(STACK + 2, 0x0000_0300)?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr() & 0x00FF, 0x0035);
    assert_eq!(cpu.pc(), 0x300);
    assert_eq!(cpu.address_register(7), Some(STACK + 6));
    Ok(())
}

#[test]
fn link_and_unlk_build_and_tear_down_the_frame() -> Result<(), Box<dyn Error>> {
    // LINK A6,#-4; UNLK A6.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x56, 0xFF, 0xFC, 0x4E, 0x5E], &[])?;
    cpu.set_a(6, 0x1111_1111);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(7), Some(STACK - 8));
    assert_eq!(cpu.address_register(6), Some(STACK - 4));
    assert_eq!(bus.read32(STACK - 4)?, 0x1111_1111);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(6), Some(0x1111_1111));
    assert_eq!(cpu.address_register(7), Some(STACK));
    Ok(())
}

#[test]
fn move_usp_forms_require_supervisor_mode() -> Result<(), Box<dyn Error>> {
    // MOVE A0,USP; MOVE USP,A1.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x60, 0x4E, 0x69], &[])?;
    cpu.set_a(0, 0x0000_1234);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.usp(), 0x0000_1234);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(1), Some(0x0000_1234));

    let (mut cpu, mut bus) = machine(&[0x4E, 0x60], &[(8, 0x400)])?;
    cpu.set_sr(0x0700);
    cpu.set_a7(STACK);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    Ok(())
}

#[test]
fn trap_uses_vector_32_plus_n_and_trapv_checks_v() -> Result<(), Box<dyn Error>> {
    // TRAP #2 with vector 34.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x42], &[(34, 0x400)])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    assert_eq!(cpu.address_register(7), Some(STACK - 6));

    // TRAPV with V set and vector 7.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x76], &[(7, 0x400)])?;
    cpu.set_sr(0x2700 | FLAG_V);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);

    // TRAPV with V clear is a no-op.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x76], &[(7, 0x400)])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    assert!(!flag(&cpu, FLAG_V));
    Ok(())
}

#[test]
fn chk_raises_vector_six_outside_the_bound() -> Result<(), Box<dyn Error>> {
    // CHK D0,D1 with D1 = -1 and vector 6.
    let (mut cpu, mut bus) = machine(&[0x43, 0x80], &[(6, 0x400)])?;
    cpu.set_d(0, 10);
    cpu.set_d(1, 0xFFFF);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);

    // In-range CHK D0,D1 leaves the PC after the instruction.
    let (mut cpu, mut bus) = machine(&[0x43, 0x80], &[(6, 0x400)])?;
    cpu.set_d(0, 10);
    cpu.set_d(1, 5);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    Ok(())
}

#[test]
fn illegal_instruction_takes_vector_four() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4A, 0xFC], &[(4, 0x400)])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    assert_eq!(bus.read16(STACK - 6)?, 0x2700);
    assert_eq!(bus.read32(STACK - 4)?, 0x202);
    Ok(())
}

#[test]
fn reset_is_a_supervisor_only_no_op() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0x70], &[])?;
    let before = cpu.sr();
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    assert_eq!(cpu.sr(), before);

    let (mut cpu, mut bus) = machine(&[0x4E, 0x70], &[(8, 0x400)])?;
    cpu.set_sr(0x0700);
    cpu.set_a7(STACK);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    Ok(())
}

#[test]
fn rte_in_user_mode_raises_vector_eight() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0x73], &[(8, 0x400)])?;
    cpu.set_sr(0x0700);
    cpu.set_a7(STACK);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    assert_eq!(bus.read16(STACK - 6)?, 0x0700);
    Ok(())
}

#[test]
fn ori_andi_eori_to_sr_use_the_full_word_immediate() -> Result<(), Box<dyn Error>> {
    // ANDI.W #$2700,SR with the low byte set: the full word is masked.
    let (mut cpu, mut bus) = machine(&[0x02, 0x7C, 0x27, 0x00], &[])?;
    cpu.set_sr(0x2707);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr(), 0x2700);
    assert_eq!(cpu.pc(), 0x204);

    // ORI.W #$0001,SR sets the low bit without disturbing S or the mask.
    let (mut cpu, mut bus) = machine(&[0x00, 0x7C, 0x00, 0x01], &[])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr(), 0x2701);

    // EORI.W #$0100,SR toggles the interrupt mask and keeps S set.
    let (mut cpu, mut bus) = machine(&[0x0A, 0x7C, 0x01, 0x00], &[])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr(), 0x2600);
    Ok(())
}
