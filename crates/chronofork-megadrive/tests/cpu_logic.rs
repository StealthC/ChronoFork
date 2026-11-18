//! Logic group coverage: AND/ANDI, OR/ORI, EOR/EORI and NOT.

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
fn andi_clears_overflow_and_carry_but_preserves_extend() -> Result<(), Box<dyn Error>> {
    // ANDI.B #$0F,D0 with X set.
    let (mut cpu, mut bus) = machine(&[0x02, 0x00, 0x00, 0x0F])?;
    cpu.set_d(0, 0x0000_00FF);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_000F));
    assert!(!flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X), "ANDI preserves X");
    Ok(())
}

#[test]
fn and_or_and_eor_register_forms_use_logic_flags() -> Result<(), Box<dyn Error>> {
    // AND.W D0,D1; OR.W D0,D1; EOR.W D0,D1.
    let (mut cpu, mut bus) = machine(&[0xC2, 0x40, 0x82, 0x40, 0xB1, 0x41])?;
    cpu.set_d(0, 0x0000_0F0F);
    cpu.set_d(1, 0x0000_FFFF);
    cpu.set_sr(0x2710);

    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0F0F));
    assert!(!flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(flag(&cpu, FLAG_X));

    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0F0F));

    cpu.set_d(1, 0x0000_FFFF);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_F0F0));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn eori_byte_and_not_memory_are_supported() -> Result<(), Box<dyn Error>> {
    // EORI.B #$FF,D0; NOT.W (A1).
    let (mut cpu, mut bus) = machine(&[0x0A, 0x00, 0x00, 0xFF, 0x46, 0x51])?;
    cpu.set_d(0, 0x1234_5600);
    cpu.set_a(1, 0x00FF_0000);
    bus.write16(0x00FF_0000, 0x0F0F)?;
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x1234_56FF));
    assert!(flag(&cpu, FLAG_N));
    assert!(flag(&cpu, FLAG_X));
    cpu.step(&mut bus)?;
    assert_eq!(bus.read16(0x00FF_0000)?, 0xF0F0);
    assert!(flag(&cpu, FLAG_N));
    assert!(flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn and_dn_to_memory_writes_the_byte_result() -> Result<(), Box<dyn Error>> {
    // AND.B D0,(A1)
    let (mut cpu, mut bus) = machine(&[0xC1, 0x11])?;
    cpu.set_d(0, 0x0000_003C);
    cpu.set_a(1, 0x00FF_0000);
    bus.write8(0x00FF_0000, 0xF0)?;
    cpu.step(&mut bus)?;
    assert_eq!(bus.read8(0x00FF_0000)?, 0x30);
    assert_eq!(cpu.data_register(0), Some(0x0000_003C));
    Ok(())
}

#[test]
fn ori_zero_sets_z_and_or_with_zero_preserves_the_operand() -> Result<(), Box<dyn Error>> {
    // ORI.W #$0000,D0.
    let (mut cpu, mut bus) = machine(&[0x00, 0x40, 0x00, 0x00])?;
    cpu.set_d(0, 0x0000_1234);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_1234));
    assert!(!flag(&cpu, FLAG_Z));
    let (mut cpu, mut bus) = machine(&[0x00, 0x40, 0x00, 0x00])?;
    cpu.set_d(0, 0);
    cpu.step(&mut bus)?;
    assert!(flag(&cpu, FLAG_Z));
    Ok(())
}
