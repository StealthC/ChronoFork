//! Multiply and divide coverage: MULU/MULS (16x16->32) and DIVU/DIVS (32/16).

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{Cartridge, M68000, MegadriveBus};

const FLAG_C: u16 = 0x0001;
const FLAG_V: u16 = 0x0002;
const FLAG_Z: u16 = 0x0004;
const FLAG_N: u16 = 0x0008;
const FLAG_X: u16 = 0x0010;
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
fn mulu_multiplies_sixteen_bit_operands_into_32_bits() -> Result<(), Box<dyn Error>> {
    // MULU D0,D1
    let (mut cpu, mut bus) = machine(&[0xC2, 0xC0], &[])?;
    cpu.set_d(0, 0x0000_FFFF);
    cpu.set_d(1, 0x0000_FFFF);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0xFFFE_0001));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X), "MULU preserves X");
    Ok(())
}

#[test]
fn muls_sign_extends_both_operands() -> Result<(), Box<dyn Error>> {
    // MULS D0,D1 with -1 * 2.
    let (mut cpu, mut bus) = machine(&[0xC3, 0xC0], &[])?;
    cpu.set_d(0, 0x0000_FFFF);
    cpu.set_d(1, 0x0000_0002);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0xFFFF_FFFE));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_V));
    Ok(())
}

#[test]
fn divu_writes_quotient_and_remainder() -> Result<(), Box<dyn Error>> {
    // DIVU D0,D1 with 0x00010000 / 2.
    let (mut cpu, mut bus) = machine(&[0x82, 0xC0], &[])?;
    cpu.set_d(0, 0x0000_0002);
    cpu.set_d(1, 0x0001_0000);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_8000));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn divu_overflow_sets_v_and_leaves_the_destination() -> Result<(), Box<dyn Error>> {
    // DIVU D0,D1 with 0x00020000 / 1 -> quotient does not fit 16 bits.
    let (mut cpu, mut bus) = machine(&[0x82, 0xC0], &[])?;
    cpu.set_d(0, 1);
    cpu.set_d(1, 0x0002_0000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0002_0000));
    assert!(flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    Ok(())
}

#[test]
fn divs_produces_signed_quotient_and_remainder() -> Result<(), Box<dyn Error>> {
    // DIVS D0,D1 with -10 / 3.
    let (mut cpu, mut bus) = machine(&[0x83, 0xC0], &[])?;
    cpu.set_d(0, 0x0000_0003);
    cpu.set_d(1, 0xFFFF_FFF6);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0xFFFF_FFFD));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_V));
    Ok(())
}

#[test]
fn divs_overflow_sets_v_and_leaves_the_destination() -> Result<(), Box<dyn Error>> {
    // DIVS D0,D1 with 0x80000000 / -1.
    let (mut cpu, mut bus) = machine(&[0x83, 0xC0], &[])?;
    cpu.set_d(0, 0x0000_FFFF);
    cpu.set_d(1, 0x8000_0000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x8000_0000));
    assert!(flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    Ok(())
}

#[test]
fn divide_by_zero_requests_vector_five() -> Result<(), Box<dyn Error>> {
    // DIVU D0,D1 with a zero divisor; vector 5 points at 0x400.
    let (mut cpu, mut bus) = machine(&[0x82, 0xC0], &[(5, 0x0000_0400)])?;
    cpu.set_d(0, 0);
    cpu.set_d(1, 0x0001_0000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    assert_ne!(
        cpu.sr() & 0x2000,
        0,
        "exception entry enters supervisor mode"
    );
    assert_eq!(cpu.address_register(7), Some(STACK - 6));
    assert_eq!(bus.read16(STACK - 6)?, 0x2700, "pushed SR");
    assert_eq!(bus.read32(STACK - 4)?, 0x202, "return PC after DIVU");
    Ok(())
}
