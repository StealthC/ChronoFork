//! Shift and rotate coverage: ASL/ASR/LSL/LSR/ROL/ROR/ROXL/ROXR in register
//! and memory forms, with immediate and data-register counts.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{Cartridge, M68000, MegadriveBus};

const FLAG_C: u16 = 0x0001;
const FLAG_V: u16 = 0x0002;
const FLAG_Z: u16 = 0x0004;
const FLAG_N: u16 = 0x0008;
const FLAG_X: u16 = 0x0010;

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

fn flag(cpu: &M68000, flag: u16) -> bool {
    cpu.sr() & flag != 0
}

#[test]
fn roxl_rotates_through_extend() -> Result<(), Box<dyn Error>> {
    // ROXL.B #1,D0 with D0=0x40 and X=1 -> 0x81, X=C=0.
    let (mut cpu, mut bus) = machine(&[0xE3, 0x10])?;
    cpu.set_d(0, 0x0000_0040);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0081));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    assert!(!flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn roxr_rotates_through_extend_to_the_right() -> Result<(), Box<dyn Error>> {
    // ROXR.B #1,D0 with D0=0x01 and X=1 -> 0x80, X=C=1.
    let (mut cpu, mut bus) = machine(&[0xE2, 0x10])?;
    cpu.set_d(0, 0x0000_0001);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0080));
    assert!(flag(&cpu, FLAG_N));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn register_shift_count_zero_leaves_every_flag_unchanged() -> Result<(), Box<dyn Error>> {
    // ASL.B D1,D0 with D1=0.
    let (mut cpu, mut bus) = machine(&[0xE3, 0x20])?;
    cpu.set_d(0, 0x0000_0081);
    cpu.set_d(1, 0);
    cpu.set_sr(0x271F);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0081));
    assert_eq!(cpu.sr() & !0xFF00, 0x1F, "every CCR bit survives count 0");
    Ok(())
}

#[test]
fn immediate_count_zero_means_eight() -> Result<(), Box<dyn Error>> {
    // ASL.B #8,D0 encoded with an immediate field of zero.
    let (mut cpu, mut bus) = machine(&[0xE1, 0x00])?;
    cpu.set_d(0, 0x0000_0001);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0));
    assert!(flag(&cpu, FLAG_Z));
    assert!(flag(&cpu, FLAG_C), "the last bit shifted out was bit 0");
    assert!(flag(&cpu, FLAG_X));
    assert!(flag(&cpu, FLAG_V), "the sign bit changed during the shift");
    Ok(())
}

#[test]
fn shift_count_is_masked_modulo_sixty_four() -> Result<(), Box<dyn Error>> {
    // ASL.L D1,D0 with D1=65 behaves as a count of 1.
    let (mut cpu, mut bus) = machine(&[0xE3, 0xA0])?;
    cpu.set_d(0, 0x0000_0001);
    cpu.set_d(1, 65);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0002));
    assert!(!flag(&cpu, FLAG_C));
    let (mut cpu, mut bus) = machine(&[0xE3, 0x80])?;
    cpu.set_d(0, 1);
    cpu.step(&mut bus)?;
    assert_eq!(
        cpu.data_register(0),
        Some(0x0000_0002),
        "immediate #1 and register count 65 agree"
    );
    Ok(())
}

#[test]
fn asr_sign_extends_and_sets_carry_from_the_last_bit() -> Result<(), Box<dyn Error>> {
    // ASR.B #1,D0 with D0=0x81 -> 0xC0, C=X=1.
    let (mut cpu, mut bus) = machine(&[0xE2, 0x00])?;
    cpu.set_d(0, 0x0000_0081);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_00C0));
    assert!(flag(&cpu, FLAG_N));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    assert!(!flag(&cpu, FLAG_V));
    Ok(())
}

#[test]
fn lsr_and_lsl_set_carry_and_extend_from_the_shifted_out_bit() -> Result<(), Box<dyn Error>> {
    // LSR.B #1,D0 with D0=0x03 -> 0x01, C=X=1.
    let (mut cpu, mut bus) = machine(&[0xE2, 0x08])?;
    cpu.set_d(0, 0x0000_0003);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0001));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    assert!(!flag(&cpu, FLAG_N));

    // LSL.B #1,D0 with D0=0x80 -> 0, C=X=1, Z set.
    let (mut cpu, mut bus) = machine(&[0xE3, 0x08])?;
    cpu.set_d(0, 0x0000_0080);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    assert!(flag(&cpu, FLAG_Z));
    Ok(())
}

#[test]
fn asl_sets_overflow_when_the_sign_bit_changes() -> Result<(), Box<dyn Error>> {
    // ASL.B #1,D0 with D0=0x40 -> 0x80, V=1.
    let (mut cpu, mut bus) = machine(&[0xE3, 0x00])?;
    cpu.set_d(0, 0x0000_0040);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0080));
    assert!(flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    Ok(())
}

#[test]
fn rol_and_ror_leave_extend_alone() -> Result<(), Box<dyn Error>> {
    // ROL.B #1,D0 with D0=0x01 -> 0x02, X unchanged.
    let (mut cpu, mut bus) = machine(&[0xE3, 0x18])?;
    cpu.set_d(0, 0x0000_0001);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0002));
    assert!(!flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X), "ROL does not update X");

    // ROR.B #1,D0 with D0=0x01 -> 0x80, C=1, X still unchanged.
    let (mut cpu, mut bus) = machine(&[0xE2, 0x18])?;
    cpu.set_d(0, 0x0000_0001);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0080));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn memory_shifts_are_word_sized_with_an_implicit_count_of_one() -> Result<(), Box<dyn Error>> {
    // ASR.W (A1); ASL.W (0x00FF0002).L
    let (mut cpu, mut bus) = machine(&[0xE0, 0xD1, 0xE1, 0xF9, 0x00, 0xFF, 0x00, 0x02])?;
    cpu.set_a(1, 0x00FF_0000);
    bus.write16(0x00FF_0000, 0x8000)?;
    bus.write16(0x00FF_0002, 0x4000)?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202, "first shift consumed its opcode only");
    assert_eq!(bus.read16(0x00FF_0000)?, 0xC000);
    assert!(flag(&cpu, FLAG_N));
    cpu.step(&mut bus)?;
    assert_eq!(bus.read16(0x00FF_0002)?, 0x8000);
    assert!(flag(&cpu, FLAG_N));
    assert!(flag(&cpu, FLAG_V), "ASL sets V when the sign bit changes");
    Ok(())
}

#[test]
fn shifting_a_register_never_changes_the_other_data_registers() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0xE3, 0x00])?;
    cpu.set_d(0, 0x0000_0001);
    cpu.set_d(7, 0xFFFF_FFFF);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0002));
    assert_eq!(cpu.data_register(7), Some(0xFFFF_FFFF));
    Ok(())
}
