//! Effective-address decode and access coverage for the 68000 core.
//!
//! These tests exercise the shared `cpu::ea` machinery directly: decoding all
//! twelve addressing modes, extension-word ordering, size-aware pointer
//! updates, sign extension and typed address faults.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{
    Cartridge, CpuError, Ea, M68000, MegadriveBus, Size, decode_ea, ea_address, read_ea, write_ea,
};

/// Builds a bus with reset vectors and `program` at address 0x200.
fn machine(program: &[u8]) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&0x00FF_FF00_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    Ok((cpu, bus))
}

#[test]
fn decodes_all_twelve_addressing_modes() -> Result<(), Box<dyn Error>> {
    let (_cpu, _bus) = machine(&[])?;
    for register in 0..8_u8 {
        assert_eq!(decode_ea(0, register, Size::Long)?, Ea::Dn(register));
        assert_eq!(decode_ea(1, register, Size::Long)?, Ea::An(register));
        assert_eq!(decode_ea(2, register, Size::Long)?, Ea::Indirect(register));
        assert_eq!(decode_ea(3, register, Size::Long)?, Ea::PostInc(register));
        assert_eq!(decode_ea(4, register, Size::Long)?, Ea::PreDec(register));
        assert_eq!(decode_ea(5, register, Size::Long)?, Ea::Disp16(register));
        assert_eq!(decode_ea(6, register, Size::Long)?, Ea::Index(register));
    }
    assert_eq!(decode_ea(7, 0, Size::Long)?, Ea::Abs16);
    assert_eq!(decode_ea(7, 1, Size::Long)?, Ea::Abs32);
    assert_eq!(decode_ea(7, 2, Size::Long)?, Ea::PcDisp16);
    assert_eq!(decode_ea(7, 3, Size::Long)?, Ea::PcIndex);
    assert_eq!(decode_ea(7, 4, Size::Long)?, Ea::Imm);
    assert!(matches!(
        decode_ea(7, 5, Size::Long),
        Err(CpuError::Unsupported(_))
    ));
    assert!(matches!(
        decode_ea(1, 0, Size::Byte),
        Err(CpuError::Unsupported(_))
    ));
    Ok(())
}

#[test]
fn post_increment_and_pre_decrement_are_size_aware() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[])?;
    cpu.set_a(0, 0x00FF_0000);
    write_ea(&mut cpu, &mut bus, Ea::PostInc(0), Size::Word, 0xBEEF)?;
    assert_eq!(cpu.address_register(0), Some(0x00FF_0002));
    assert_eq!(bus.read16(0x00FF_0000)?, 0xBEEF);
    write_ea(&mut cpu, &mut bus, Ea::PreDec(0), Size::Byte, 0x7F)?;
    assert_eq!(cpu.address_register(0), Some(0x00FF_0001));
    assert_eq!(bus.read8(0x00FF_0001)?, 0x7F);
    cpu.set_a(1, 0x00FF_0008);
    write_ea(&mut cpu, &mut bus, Ea::PostInc(1), Size::Long, 0x1234_5678)?;
    assert_eq!(cpu.address_register(1), Some(0x00FF_000C));
    assert_eq!(bus.read32(0x00FF_0008)?, 0x1234_5678);
    Ok(())
}

#[test]
fn reads_step_by_byte_word_and_long() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[])?;
    cpu.set_a(0, 0x00FF_0000);
    bus.write32(0x00FF_0000, 0x1122_3344)?;
    assert_eq!(
        read_ea(&mut cpu, &mut bus, Ea::PostInc(0), Size::Byte)?,
        0x11
    );
    assert_eq!(cpu.address_register(0), Some(0x00FF_0001));
    cpu.set_a(0, 0x00FF_0000);
    assert_eq!(
        read_ea(&mut cpu, &mut bus, Ea::PostInc(0), Size::Word)?,
        0x1122
    );
    assert_eq!(cpu.address_register(0), Some(0x00FF_0002));
    cpu.set_a(0, 0x00FF_0000);
    assert_eq!(
        read_ea(&mut cpu, &mut bus, Ea::PostInc(0), Size::Long)?,
        0x1122_3344
    );
    assert_eq!(cpu.address_register(0), Some(0x00FF_0004));
    Ok(())
}

#[test]
fn absolute_short_sign_extends_and_short_immediates_fetch_one_word() -> Result<(), Box<dyn Error>> {
    // A NOP advances PC to the extension word at 0x202.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71, 0xFF, 0xF0, 0x00, 0x7F])?;
    cpu.step(&mut bus)?;
    assert_eq!(ea_address(&mut cpu, &mut bus, Ea::Abs16)?, 0xFFFF_FFF0);
    assert_eq!(read_ea(&mut cpu, &mut bus, Ea::Imm, Size::Byte)?, 0x7F);
    Ok(())
}

#[test]
fn pc_relative_base_is_the_extension_word_address() -> Result<(), Box<dyn Error>> {
    // NOP at 0x200; displacement word at 0x202 holds +0x10.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71, 0x00, 0x10])?;
    cpu.step(&mut bus)?;
    assert_eq!(ea_address(&mut cpu, &mut bus, Ea::PcDisp16)?, 0x212);
    Ok(())
}

#[test]
fn indexed_addressing_adds_signed_displacement_and_sign_extended_index()
-> Result<(), Box<dyn Error>> {
    // NOP at 0x200; extension word 0x1010 selects A/D=0 (D1), word-sized,
    // displacement +0x10.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71, 0x10, 0x10])?;
    cpu.set_a(0, 0x00FF_0000);
    cpu.set_d(1, 0x0000_FFF0);
    cpu.step(&mut bus)?;
    // 0x00FF0000 + 0x10 displacement + (-16) sign-extended index = 0x00FF0000.
    assert_eq!(ea_address(&mut cpu, &mut bus, Ea::Index(0))?, 0x00FF_0000);
    Ok(())
}

#[test]
fn post_increment_write_that_faults_leaves_the_pointer_untouched() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[])?;
    cpu.set_a(0, 0x00FF_FFFF);
    assert!(matches!(
        write_ea(&mut cpu, &mut bus, Ea::PostInc(0), Size::Word, 0xBEEF),
        Err(CpuError::Bus(_))
    ));
    assert_eq!(cpu.address_register(0), Some(0x00FF_FFFF));
    Ok(())
}

#[test]
fn byte_access_to_an_is_a_typed_address_error() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[])?;
    cpu.set_a(3, 0x00FF_0004);
    assert_eq!(
        write_ea(&mut cpu, &mut bus, Ea::An(3), Size::Byte, 0x7F),
        Err(CpuError::AddressError {
            address: 0x00FF_0004
        })
    );
    assert_eq!(
        read_ea(&mut cpu, &mut bus, Ea::An(3), Size::Byte),
        Err(CpuError::AddressError {
            address: 0x00FF_0004
        })
    );
    assert_eq!(cpu.address_register(3), Some(0x00FF_0004));
    Ok(())
}

#[test]
fn word_and_long_writes_to_an_use_the_documented_widths() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[])?;
    cpu.set_d(0, 0x1234_5678);
    write_ea(&mut cpu, &mut bus, Ea::An(0), Size::Word, 0xFFFF_8001)?;
    assert_eq!(cpu.address_register(0), Some(0xFFFF_8001));
    write_ea(&mut cpu, &mut bus, Ea::An(0), Size::Long, 0x1234_5678)?;
    assert_eq!(cpu.address_register(0), Some(0x1234_5678));
    Ok(())
}
