//! Effective-address decode and access coverage for the 68000 core.
//!
//! These tests exercise the shared `cpu::ea` machinery directly: decoding all
//! twelve addressing modes, extension-word ordering, size-aware pointer
//! updates, sign extension and typed address faults.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{
    AddressAccess, Cartridge, CpuError, Ea, M68000, MegadriveBus, Size, decode_ea, ea_address,
    read_ea, write_ea,
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
        Err(CpuError::AddressError {
            address: 0x00FF_FFFF,
            access: AddressAccess::DataWrite,
        })
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
            address: 0x00FF_0004,
            access: AddressAccess::DataWrite,
        })
    );
    assert_eq!(
        read_ea(&mut cpu, &mut bus, Ea::An(3), Size::Byte),
        Err(CpuError::AddressError {
            address: 0x00FF_0004,
            access: AddressAccess::DataRead,
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

// --- W1K: address-register arithmetic wraps modulo 2^32 -------------------
//
// The 68000 has no overflow or underflow trap for `(An)+`/`-(An)`, for MOVEM's
// predecrement or for the stack pointer. The register wraps, the bus truncates
// the value to 24 bits and the memory map decides whether the access is legal.
// The boot fixtures depend on that: `MOVE.L D0,-(A6)` with A6 = 0 must write
// through the work RAM mirror instead of aborting the host.

/// MOVE.L D0,-(A6): the clean-64-KiB-RAM idiom of the boot fixtures.
const MOVE_L_D0_TO_PREDEC_A6: &[u8] = &[0x2D, 0x00];
/// MOVE.L (A6)+,D0.
const MOVE_L_POSTINC_A6_TO_D0: &[u8] = &[0x20, 0x1E];
/// MOVEM.L D0,-(A6) followed by the register mask.
///
/// The predecrement form uses the reversed mask encoding: bit 15 is D0 and bit 0
/// is A7 (the hardware walks the mask from bit 15 down to bit 0), so a single
/// `D0` is `0x8000`, not `0x0001`. The oracle corpus `rom_movem` pins the same
/// interpretation (a predecrement store of mask `0x0007` moves A7/a6/a5).
const MOVEM_L_D0_TO_PREDEC_A6: &[u8] = &[0x48, 0xE6, 0x80, 0x00];

#[test]
fn pre_decrement_from_zero_wraps_into_the_ram_mirror() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(MOVE_L_D0_TO_PREDEC_A6)?;
    cpu.set_d(0, 0x1122_3344);
    cpu.set_a(6, 0);
    cpu.step(&mut bus)?;
    // The raw register keeps the wrapped 32-bit value...
    assert_eq!(cpu.address_register(6), Some(0xFFFF_FFFC));
    // ...and the write landed in work RAM, four bytes before the wrap point:
    // 0xFFFFFFFC..0xFFFFFFFF truncates to the mirror at 0x00FF_FFFC.
    assert_eq!(bus.read32(0x00FF_FFFC)?, 0x1122_3344);
    assert_eq!(
        bus.read32(0xFFFF_FFFC)?,
        0x1122_3344,
        "same bytes, wide address"
    );
    Ok(())
}

#[test]
fn post_increment_from_the_top_wraps_to_zero() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(MOVE_L_POSTINC_A6_TO_D0)?;
    bus.write32(0x00FF_FFFC, 0xCAFE_BEEF)?;
    cpu.set_a(6, 0xFFFF_FFFC);
    cpu.step(&mut bus)?;
    assert_eq!(
        cpu.data_register(0),
        Some(0xCAFE_BEEF),
        "read via the mirror"
    );
    assert_eq!(cpu.address_register(6), Some(0), "A6 wrapped past 2^32");
    Ok(())
}

#[test]
fn movem_pre_decrement_wraps_the_address_register() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(MOVEM_L_D0_TO_PREDEC_A6)?;
    cpu.set_d(0, 0xABCD_0123);
    cpu.set_a(6, 0);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(6), Some(0xFFFF_FFFC));
    assert_eq!(bus.read32(0x00FF_FFFC)?, 0xABCD_0123);
    Ok(())
}

#[test]
fn normal_pre_decrement_and_post_increment_stay_inside_the_map() -> Result<(), Box<dyn Error>> {
    // Unchanged behaviour well away from the wrap point: A6 = 0x00FF1004.
    let (mut cpu, mut bus) = machine(MOVE_L_D0_TO_PREDEC_A6)?;
    cpu.set_d(0, 0x55AA_55AA);
    cpu.set_a(6, 0x00FF_1004);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(6), Some(0x00FF_1000));
    assert_eq!(bus.read32(0x00FF_1000)?, 0x55AA_55AA);

    let (mut cpu, mut bus) = machine(MOVE_L_POSTINC_A6_TO_D0)?;
    bus.write32(0x00FF_1000, 0x0102_0304)?;
    cpu.set_a(6, 0x00FF_1000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0102_0304));
    assert_eq!(cpu.address_register(6), Some(0x00FF_1004));
    Ok(())
}

#[test]
fn a_wrapped_address_that_truncates_into_an_unmapped_hole_still_faults()
-> Result<(), Box<dyn Error>> {
    // A6 = 0x0180_0004 makes `-(A6)` wrap to 0x0180_0000, which truncates to
    // 0x0080_0000 — the hole between the cartridge window and the Z80 RAM. The
    // guest takes the documented address-error exception (vector 3), so the wrap
    // does not turn an unmapped access into a silent one.
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&0x00FF_FF00_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x0C..0x10].copy_from_slice(&0x0000_0400_u32.to_be_bytes()); // vector 3
    rom[0x200..0x202].copy_from_slice(MOVE_L_D0_TO_PREDEC_A6);
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    cpu.set_d(0, 0x1111_1111);
    cpu.set_a(6, 0x0180_0004);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400, "vector 3 ran");
    // This slice deliberately leaves the address register untouched when the
    // access itself faults (the same documented contract as
    // `post_increment_write_that_faults_leaves_the_pointer_untouched`), so the
    // wrap is proven by the fault address, not by the register value. Hardware
    // would already have decremented it; recorded as residual in W1K.
    assert_eq!(cpu.address_register(6), Some(0x0180_0004));
    Ok(())
}

// --- W1S: effective-address displacement arithmetic is mod 2^32 ------------
//
// The EA sum wraps; the 24-bit bus then truncates and the memory map decides
// whether the access is legal. There is no "address overflow" host error left to
// raise, matching the register-arithmetic rule the W1K phase applied.

/// Builds a machine whose program sits at `0x200` and whose vector 3 points at
/// `0x400`, so a guest address error is observable as a PC change.
fn machine_with_trap(program: &[u8]) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&0x00FF_FF00_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x0C..0x10].copy_from_slice(&0x0000_0400_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    Ok((cpu, bus))
}

#[test]
fn a_negative_d16_wraps_into_the_ram_mirror_like_the_chakan_instruction()
-> Result<(), Box<dyn Error>> {
    // `42 2D B8 C6` is CLR.B (0xB8C6,A5) — the exact instruction at Chakan
    // pc=0x3046 (the task description called it NOT.B; the opcode 0x422D is CLR.B,
    // which is a write, so the wrapping behaviour is the same). With A5 = 0x200 the
    // sum is 0xFFFF_BAC6, which truncates to 0x00FF_BAC6 — the work-RAM mirror — so
    // this is a legal write, not a host abort.
    let (mut cpu, mut bus) = machine_with_trap(&[0x42, 0x2D, 0xB8, 0xC6])?;
    cpu.set_a(5, 0x0000_0200);
    bus.write8(0x00FF_BAC6, 0x5A)?;
    cpu.step(&mut bus)?;
    assert_eq!(
        bus.read8(0x00FF_BAC6)?,
        0x00,
        "CLR.B cleared the mirrored byte"
    );
    assert_eq!(cpu.address_register(5), Some(0x0000_0200), "An untouched");
    assert_eq!(cpu.pc(), 0x204, "the instruction completed");
    Ok(())
}

#[test]
fn a_plain_negative_d16_reaches_the_mapped_address() -> Result<(), Box<dyn Error>> {
    // `MOVE.B -2(A0),D0` with A0 = 0x00FF1000 reads 0x00FF0FFE.
    let (mut cpu, mut bus) = machine_with_trap(&[0x10, 0x28, 0xFF, 0xFE])?;
    bus.write8(0x00FF_0FFE, 0x5A)?;
    cpu.set_a(0, 0x00FF_1000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x5A));
    Ok(())
}

#[test]
fn a_negative_d8_index_reaches_the_mapped_address() -> Result<(), Box<dyn Error>> {
    // `MOVE.B (0x10,A0,D1.W),D0` with A0 = 0x00FF2000 and D1.W = 0xFFE0 (-32):
    // the base plus the d8 (0x10) plus the sign-extended index (-32) is
    // 0x00FF1FF0.
    let (mut cpu, mut bus) = machine_with_trap(&[0x10, 0x30, 0x10, 0x10])?;
    bus.write8(0x00FF_1FF0, 0xA5)?;
    cpu.set_a(0, 0x00FF_2000);
    cpu.set_d(1, 0xFFFF_FFE0);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0xA5));
    Ok(())
}

#[test]
fn a_wrapped_ea_that_truncates_into_an_unmapped_hole_still_faults() -> Result<(), Box<dyn Error>> {
    // `NOT.B (0,A5)` with A5 = 0x0180_0000: the truncation lands in the hole at
    // 0x0080_0000, so the guest takes the documented address error (vector 3) and
    // no host error ends the run.
    let (mut cpu, mut bus) = machine_with_trap(&[0x42, 0x2D, 0x00, 0x00])?;
    cpu.set_a(5, 0x0180_0000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400, "vector 3 ran");
    Ok(())
}
