//! Arithmetic group coverage: ADD/ADDA/ADDQ/ADDI, SUB/SUBA/SUBQ/SUBI,
//! CMP/CMPA/CMPI/CMPM, NEG/NEGX.

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
fn add_sets_carry_extend_overflow_and_negative() -> Result<(), Box<dyn Error>> {
    // ADD.W #1,D0 with D0=0x7FFF -> 0x8000.
    let (mut cpu, mut bus) = machine(&[0xD0, 0x7C, 0x00, 0x01])?;
    cpu.set_d(0, 0x0000_7FFF);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_8000));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(flag(&cpu, FLAG_V));
    assert!(
        !flag(&cpu, FLAG_C),
        "0x7FFF + 1 does not carry out of a word"
    );
    assert!(!flag(&cpu, FLAG_X));
    Ok(())
}

#[test]
fn addi_byte_and_addq_encode_zero_as_eight() -> Result<(), Box<dyn Error>> {
    // ADDI.B #1,D0; ADDQ.W #8,D1 (encoded immediate 0 means 8).
    let (mut cpu, mut bus) = machine(&[0x06, 0x00, 0x00, 0x01, 0x50, 0x41])?;
    cpu.set_d(0, 0x1234_56FE);
    cpu.set_d(1, 0x0000_FFF8);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x1234_56FF));
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0000));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    assert!(flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_N));
    Ok(())
}

#[test]
fn addq_to_an_uses_full_32_bits_without_flags() -> Result<(), Box<dyn Error>> {
    // ADDQ.L #1,A0
    let (mut cpu, mut bus) = machine(&[0x52, 0x88])?;
    cpu.set_a(0, 0xFFFF_FFFF);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(0), Some(0x0000_0000));
    assert_eq!(cpu.sr(), 0x2710, "ADDQ to An leaves every flag alone");
    Ok(())
}

#[test]
fn sub_sets_borrow_and_signed_overflow() -> Result<(), Box<dyn Error>> {
    // SUB.W #1,D0 with D0=0x8000.
    let (mut cpu, mut bus) = machine(&[0x90, 0x7C, 0x00, 0x01])?;
    cpu.set_d(0, 0x0000_8000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_7FFF));
    assert!(flag(&cpu, FLAG_V), "signed overflow into the positive half");
    assert!(!flag(&cpu, FLAG_C), "no borrow for 0x8000 - 1");
    assert!(!flag(&cpu, FLAG_N));
    Ok(())
}

#[test]
fn suba_and_adda_sign_extend_words_without_flags() -> Result<(), Box<dyn Error>> {
    // SUBA.W #1,A0; ADDA.W #$FFFE,A1.
    let (mut cpu, mut bus) = machine(&[
        0x90, 0xFC, 0x00, 0x01, // SUBA.W #1,A0
        0xD2, 0xFC, 0xFF, 0xFE, // ADDA.W #$FFFE,A1
    ])?;
    cpu.set_a(0, 0x0000_8000);
    cpu.set_a(1, 0x0000_0010);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(0), Some(0x0000_7FFF));
    cpu.step(&mut bus)?;
    assert_eq!(
        cpu.address_register(1),
        Some(0x0000_000E),
        "$FFFE sign-extends to -2"
    );
    assert_eq!(cpu.sr(), 0x2710, "ADDA/SUBA do not change flags");
    Ok(())
}

#[test]
fn add_to_memory_uses_the_destination_width() -> Result<(), Box<dyn Error>> {
    // ADD.W D0,(A1)
    let (mut cpu, mut bus) = machine(&[0xD1, 0x51])?;
    cpu.set_d(0, 0x0000_0100);
    cpu.set_a(1, 0x00FF_0000);
    bus.write16(0x00FF_0000, 0x00FF)?;
    cpu.step(&mut bus)?;
    assert_eq!(bus.read16(0x00FF_0000)?, 0x01FF);
    assert_eq!(cpu.data_register(0), Some(0x0000_0100));
    Ok(())
}

#[test]
fn cmp_does_not_change_extend() -> Result<(), Box<dyn Error>> {
    // CMP.B D0,D1 with D1=0x10 and D0=0x20 -> 0xF0.
    let (mut cpu, mut bus) = machine(&[0xB2, 0x00])?;
    cpu.set_d(0, 0x0000_0020);
    cpu.set_d(1, 0x0000_0010);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0010));
    assert!(flag(&cpu, FLAG_N));
    assert!(flag(&cpu, FLAG_C));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V));
    assert!(flag(&cpu, FLAG_X), "CMP leaves X unchanged");
    Ok(())
}

#[test]
fn cmpi_leaves_extend_and_cmpa_sign_extends_its_source() -> Result<(), Box<dyn Error>> {
    // CMPI.W #0,D0; CMPA.W #$FFFF,A0.
    let (mut cpu, mut bus) = machine(&[
        0x0C, 0x40, 0x00, 0x00, // CMPI.W #0,D0
        0xB0, 0xFC, 0xFF, 0xFF, // CMPA.W #$FFFF,A0
    ])?;
    cpu.set_d(0, 0);
    cpu.set_a(0, 0);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert!(flag(&cpu, FLAG_Z));
    assert!(flag(&cpu, FLAG_X), "CMPI leaves X unchanged");
    cpu.step(&mut bus)?;
    assert!(
        flag(&cpu, FLAG_C),
        "0 - (-1) borrows in the 32-bit comparison"
    );
    assert!(flag(&cpu, FLAG_X), "CMPA leaves X unchanged");
    Ok(())
}

#[test]
fn cmpm_post_increments_both_operands() -> Result<(), Box<dyn Error>> {
    // CMPM.B (A0)+,(A1)+
    let (mut cpu, mut bus) = machine(&[0xB3, 0x08])?;
    cpu.set_a(0, 0x00FF_0000);
    cpu.set_a(1, 0x00FF_0010);
    bus.write8(0x00FF_0000, 0x10)?;
    bus.write8(0x00FF_0010, 0x20)?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(0), Some(0x00FF_0001));
    assert_eq!(cpu.address_register(1), Some(0x00FF_0011));
    assert_eq!(cpu.sr() & (FLAG_Z | FLAG_N | FLAG_C | FLAG_V), 0);
    Ok(())
}

#[test]
fn neg_and_negx_fold_extend_and_set_carry() -> Result<(), Box<dyn Error>> {
    // NEG.B D0 with D0=0x80; NEGX.B D1 with D1=0 and X=1.
    let (mut cpu, mut bus) = machine(&[0x44, 0x00, 0x40, 0x01])?;
    cpu.set_d(0, 0x0000_0080);
    cpu.set_d(1, 0x0000_0000);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_0080));
    assert!(flag(&cpu, FLAG_V));
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X));
    assert!(flag(&cpu, FLAG_N));

    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_00FF));
    assert!(flag(&cpu, FLAG_X), "NEGX keeps the incoming extend");
    assert!(flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    Ok(())
}
