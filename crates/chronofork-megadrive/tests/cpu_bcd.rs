//! BCD coverage: ABCD, SBCD and NBCD (register and memory forms).

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
fn abcd_adds_with_extend_and_decimal_adjust() -> Result<(), Box<dyn Error>> {
    // ABCD D0,D1 with 0x45 + 0x38 + 0.
    let (mut cpu, mut bus) = machine(&[0xC3, 0x00])?;
    cpu.set_d(0, 0x0000_0038);
    cpu.set_d(1, 0x0000_0045);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0083));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V), "V is cleared by convention");
    assert!(!flag(&cpu, FLAG_C));
    assert!(!flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn abcd_adjusts_when_the_low_nibble_sum_carries() -> Result<(), Box<dyn Error>> {
    // ABCD D0,D1 with 0x19 + 0x27 + X=0. The binary sum 0x40 has a low digit
    // below nine, but 9+7 = 16 decimal must still add the BCD correction, so
    // the documented result is 0x46, not 0x40.
    let (mut cpu, mut bus) = machine(&[0xC3, 0x00])?;
    cpu.set_d(0, 0x0000_0019);
    cpu.set_d(1, 0x0000_0027);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0046));
    assert!(!flag(&cpu, FLAG_C));
    assert!(!flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn abcd_carries_through_the_high_nibble() -> Result<(), Box<dyn Error>> {
    // ABCD D0,D1 with 0x99 + 0x00 + X=1 -> 0x00, C=X=1.
    let (mut cpu, mut bus) = machine(&[0xC3, 0x00])?;
    cpu.set_d(0, 0);
    cpu.set_d(1, 0x0000_0099);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    assert!(!flag(&cpu, FLAG_V));
    Ok(())
}

#[test]
fn sbcd_borrows_and_nbcd_negates_in_decimal() -> Result<(), Box<dyn Error>> {
    // SBCD D1,D0 with 0x00 - 0x01 = 0x99, C=X=1.
    let (mut cpu, mut bus) = machine(&[0x81, 0x01])?;
    cpu.set_d(1, 0x0000_0001);
    cpu.set_d(0, 0);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0099));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    assert!(flag(&cpu, FLAG_N));

    // NBCD D0 with 0x12 and X=0 -> 0x88, C=1.
    let (mut cpu, mut bus) = machine(&[0x48, 0x00])?;
    cpu.set_d(0, 0x0000_0012);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0088));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn nbcd_folds_the_incoming_extend() -> Result<(), Box<dyn Error>> {
    // NBCD D0 with 0x00 and X=1 -> 0x99, C=X=1.
    let (mut cpu, mut bus) = machine(&[0x48, 0x00])?;
    cpu.set_d(0, 0);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0099));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn abcd_memory_form_predecrements_both_operands() -> Result<(), Box<dyn Error>> {
    // ABCD -(A1),-(A0)
    let (mut cpu, mut bus) = machine(&[0xC1, 0x09])?;
    cpu.set_a(0, 0x00FF_0010);
    cpu.set_a(1, 0x00FF_0020);
    bus.write8(0x00FF_000F, 0x45)?;
    bus.write8(0x00FF_001F, 0x38)?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(0), Some(0x00FF_000F));
    assert_eq!(cpu.address_register(1), Some(0x00FF_001F));
    assert_eq!(bus.read8(0x00FF_000F)?, 0x83);
    Ok(())
}

#[test]
fn nbcd_memory_form_writes_the_byte() -> Result<(), Box<dyn Error>> {
    // NBCD (A0)
    let (mut cpu, mut bus) = machine(&[0x48, 0x10])?;
    cpu.set_a(0, 0x00FF_0000);
    bus.write8(0x00FF_0000, 0x12)?;
    cpu.step(&mut bus)?;
    assert_eq!(bus.read8(0x00FF_0000)?, 0x88);
    assert!(flag(&cpu, FLAG_C));
    Ok(())
}
