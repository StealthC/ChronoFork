//! Data movement group coverage: MOVE/MOVEA/MOVEQ/MOVEM/MOVEP/LEA/PEA/CLR/TST/EXT/SWAP.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{Cartridge, M68000, MegadriveBus};

const FLAG_C: u16 = 0x0001;
const FLAG_V: u16 = 0x0002;
const FLAG_Z: u16 = 0x0004;
const FLAG_N: u16 = 0x0008;
const FLAG_X: u16 = 0x0010;

const STACK: u32 = 0x00FF_FF00;

/// Builds a bus with reset vectors, `program` at 0x200 and optional ROM data.
fn machine(
    program: &[u8],
    data: &[(u32, &[u8])],
) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    for (address, bytes) in data {
        let start = usize::try_from(*address)?;
        rom[start..start + bytes.len()].copy_from_slice(bytes);
    }
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    Ok((cpu, bus))
}

fn run(program: &[u8], data: &[(u32, &[u8])]) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(program, data)?;
    cpu.step(&mut bus)?;
    Ok((cpu, bus))
}

fn flag(cpu: &M68000, flag: u16) -> bool {
    cpu.sr() & flag != 0
}

#[test]
fn move_byte_between_data_registers_sets_logic_flags_and_preserves_x() -> Result<(), Box<dyn Error>>
{
    // MOVE.B D0,D1 with D0 = 0x0000_0080 and X set.
    let (mut cpu, mut bus) = machine(&[0x12, 0x00], &[])?;
    cpu.set_d(0, 0x0000_0080);
    cpu.set_sr(0x2710);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0x0000_0080));
    assert!(flag(&cpu, FLAG_N), "byte result is negative");
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X), "MOVE preserves X");
    Ok(())
}

#[test]
fn move_word_and_long_forms_copy_the_documented_widths() -> Result<(), Box<dyn Error>> {
    // MOVE.W D0,D1; MOVE.L D2,D3. The first instruction occupies 0x200..0x202.
    let (mut cpu, mut bus) = machine(&[0x32, 0x00, 0x26, 0x02], &[])?;
    cpu.set_d(0, 0x1234_ABCD);
    cpu.set_d(1, 0xFFFF_1111);
    cpu.set_d(2, 0x89AB_CDEF);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0xFFFF_ABCD));
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(3), Some(0x89AB_CDEF));
    Ok(())
}

#[test]
fn moveq_sign_extends_a_byte_and_sets_n() -> Result<(), Box<dyn Error>> {
    // MOVEQ #$FF,D0
    let (cpu, _bus) = run(&[0x70, 0xFF], &[])?;
    assert_eq!(cpu.data_register(0), Some(0xFFFF_FFFF));
    assert!(flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    Ok(())
}

#[test]
fn movea_word_sign_extends_into_the_address_register() -> Result<(), Box<dyn Error>> {
    // MOVEA.W #$8001,A0
    let (mut cpu, mut bus) = machine(&[0x30, 0x7C, 0x80, 0x01], &[])?;
    cpu.set_sr(0x2700);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(0), Some(0xFFFF_8001));
    assert_eq!(cpu.sr(), 0x2700, "MOVEA does not change the flags");
    Ok(())
}

#[test]
fn move_long_immediate_and_absolute_word_source_are_supported() -> Result<(), Box<dyn Error>> {
    // MOVE.L #$12345678,D3
    let (cpu, _bus) = run(&[0x26, 0x3C, 0x12, 0x34, 0x56, 0x78], &[])?;
    assert_eq!(cpu.data_register(3), Some(0x1234_5678));
    assert!(!flag(&cpu, FLAG_N));

    // MOVE.W (0x0300).W,D1; the ROM word at 0x0300 is 0xBEEF.
    let (cpu, _bus) = run(&[0x32, 0x38, 0x03, 0x00], &[(0x300, &[0xBE, 0xEF])])?;
    assert_eq!(cpu.data_register(1), Some(0x0000_BEEF));
    assert!(flag(&cpu, FLAG_N));
    Ok(())
}

#[test]
fn lea_and_pea_accept_absolute_word_and_pc_relative_control_modes() -> Result<(), Box<dyn Error>> {
    // LEA (0x0300).W,A0; PEA (d16,PC), with +4 displacement from the
    // extension word at 0x0206 to 0x020A.
    let (mut cpu, mut bus) = machine(
        &[0x41, 0xF8, 0x03, 0x00, 0x48, 0x7A, 0x00, 0x04],
        &[(0x300, &[0xAA])],
    )?;
    let flags = cpu.sr();
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(0), Some(0x0300));
    assert_eq!(cpu.sr(), flags, "LEA does not change the flags");
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(7), Some(STACK - 4));
    assert_eq!(bus.read32(STACK - 4)?, 0x020A);
    assert_eq!(cpu.sr(), flags, "PEA does not change the flags");
    Ok(())
}

#[test]
fn clr_tst_ext_and_swap_use_logic_flags() -> Result<(), Box<dyn Error>> {
    // CLR.W (0x00FF0000).L; TST.L (0x00FF0002).L; EXT.W D0; EXT.L D1; SWAP D2.
    let (mut cpu, mut bus) = machine(
        &[
            0x42, 0x79, 0x00, 0xFF, 0x00, 0x00, //
            0x4A, 0xB9, 0x00, 0xFF, 0x00, 0x02, //
            0x48, 0x80, 0x48, 0xC1, 0x48, 0x42,
        ],
        &[],
    )?;
    bus.write32(0x00FF_0000, 0xFFFF_FFFF)?;
    bus.write32(0x00FF_0002, 0x8000_0000)?;
    cpu.set_d(0, 0x1234_0080);
    cpu.set_d(1, 0x0000_8000);
    cpu.set_d(2, 0x1234_5678);
    cpu.set_sr(0x2710);

    cpu.step(&mut bus)?;
    assert_eq!(bus.read16(0x00FF_0000)?, 0);
    assert!(flag(&cpu, FLAG_Z));
    assert!(!flag(&cpu, FLAG_N));
    assert!(!flag(&cpu, FLAG_V));
    assert!(!flag(&cpu, FLAG_C));
    assert!(flag(&cpu, FLAG_X), "CLR preserves X");

    cpu.step(&mut bus)?;
    assert!(flag(&cpu, FLAG_N), "TST.L sees the long sign bit");
    assert!(!flag(&cpu, FLAG_Z));

    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x1234_FF80));
    assert!(flag(&cpu, FLAG_N));
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0xFFFF_8000));
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(2), Some(0x5678_1234));
    assert!(!flag(&cpu, FLAG_N));
    Ok(())
}

#[test]
fn movem_word_predecrement_reverses_the_mask_and_stores_in_register_order()
-> Result<(), Box<dyn Error>> {
    // MOVEM.W D0-D1,-(A7): predecrement mask 0xC000 selects D0/D1.
    let (mut cpu, mut bus) = machine(&[0x48, 0xA7, 0xC0, 0x00], &[])?;
    cpu.set_d(0, 0x1111_2222);
    cpu.set_d(1, 0x3333_4444);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.address_register(7), Some(STACK - 4));
    assert_eq!(
        bus.read16(STACK - 4)?,
        0x2222,
        "D0 ends at the lower address"
    );
    assert_eq!(
        bus.read16(STACK - 2)?,
        0x4444,
        "D1 ends at the higher address"
    );
    Ok(())
}

#[test]
fn movem_word_postincrement_loads_in_mask_order_and_sign_extends_address_registers()
-> Result<(), Box<dyn Error>> {
    // MOVEM.W (A0)+,D0/A1; mask normal order: D0 bit 0, A1 bit 9 -> 0x0201.
    let (mut cpu, mut bus) = machine(&[0x4C, 0x98, 0x02, 0x01], &[])?;
    cpu.set_a(0, 0x00FF_0000);
    bus.write16(0x00FF_0000, 0x1234)?;
    bus.write16(0x00FF_0002, 0x8001)?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x1234));
    assert_eq!(
        cpu.address_register(1),
        Some(0xFFFF_8001),
        "word loads sign-extend into address registers"
    );
    assert_eq!(cpu.address_register(0), Some(0x00FF_0004));
    Ok(())
}

#[test]
fn movep_uses_alternating_byte_order_in_both_directions() -> Result<(), Box<dyn Error>> {
    // MOVEP.W (0x10,A0),D0; MOVEP.L (0x14,A0),D1; then MOVEP.W D0,(0x18,A0).
    let (mut cpu, mut bus) = machine(
        &[
            0x01, 0x08, 0x00, 0x10, // MOVEP.W (0x10,A0),D0
            0x03, 0x48, 0x00, 0x14, // MOVEP.L (0x14,A0),D1
            0x01, 0x88, 0x00, 0x18, // MOVEP.W D0,(0x18,A0)
        ],
        &[],
    )?;
    cpu.set_a(0, 0x00FF_0000);
    bus.write8(0x00FF_0010, 0x12)?;
    bus.write8(0x00FF_0012, 0x34)?;
    bus.write8(0x00FF_0014, 0xDE)?;
    bus.write8(0x00FF_0016, 0xAD)?;
    bus.write8(0x00FF_0018, 0xBE)?;
    bus.write8(0x00FF_001A, 0xEF)?;

    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x0000_1234));
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0xDEAD_BEEF));
    cpu.step(&mut bus)?;
    assert_eq!(bus.read8(0x00FF_0018)?, 0x12);
    assert_eq!(bus.read8(0x00FF_001A)?, 0x34);
    Ok(())
}
