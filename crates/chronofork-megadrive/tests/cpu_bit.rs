//! Bit-operation coverage: BTST/BCHG/BCLR/BSET, static and dynamic bit numbers.

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
fn btst_static_sets_only_z() -> Result<(), Box<dyn Error>> {
    // BTST #7,D0 with every flag set and D0 bit 7 set.
    let (mut cpu, mut bus) = machine(&[0x08, 0x00, 0x00, 0x07])?;
    cpu.set_d(0, 0x0000_0080);
    cpu.set_sr(0x271F);
    cpu.step(&mut bus)?;
    assert!(!flag(&cpu, FLAG_Z), "tested bit was set");
    assert!(
        flag(&cpu, FLAG_N | FLAG_V | FLAG_C | FLAG_X),
        "only Z changes"
    );
    assert_eq!(cpu.data_register(0), Some(0x0000_0080));
    Ok(())
}

#[test]
fn btst_static_sets_zero_for_a_clear_bit() -> Result<(), Box<dyn Error>> {
    // BTST #6,D0 with bit 6 clear.
    let (mut cpu, mut bus) = machine(&[0x08, 0x00, 0x00, 0x06])?;
    cpu.set_d(0, 0x0000_0080);
    cpu.step(&mut bus)?;
    assert!(flag(&cpu, FLAG_Z));
    Ok(())
}

#[test]
fn bset_bclr_bchg_memory_use_modulo_eight() -> Result<(), Box<dyn Error>> {
    // BSET #9,(A0) toggles bit 1; BCLR #9,(A0); BCHG #9,(A0).
    let (mut cpu, mut bus) = machine(&[
        0x08, 0xD0, 0x00, 0x09, // BSET #9,(A0)
        0x08, 0x90, 0x00, 0x09, // BCLR #9,(A0)
        0x08, 0x50, 0x00, 0x09, // BCHG #9,(A0)
    ])?;
    cpu.set_a(0, 0x00FF_0000);
    bus.write8(0x00FF_0000, 0)?;
    cpu.step(&mut bus)?;
    assert_eq!(bus.read8(0x00FF_0000)?, 0x02);
    cpu.step(&mut bus)?;
    assert_eq!(bus.read8(0x00FF_0000)?, 0x00);
    cpu.step(&mut bus)?;
    assert_eq!(bus.read8(0x00FF_0000)?, 0x02);
    Ok(())
}

#[test]
fn dynamic_bit_number_reads_the_source_data_register() -> Result<(), Box<dyn Error>> {
    // BTST D1,D0 with D1=7 and D0 bit 7 set.
    let (mut cpu, mut bus) = machine(&[0x03, 0x00])?;
    cpu.set_d(0, 0x0000_0080);
    cpu.set_d(1, 7);
    cpu.step(&mut bus)?;
    assert!(!flag(&cpu, FLAG_Z));
    assert_eq!(cpu.data_register(0), Some(0x0000_0080));

    // BSET D1,(A0) with D1=1 sets memory bit 1.
    let (mut cpu, mut bus) = machine(&[0x03, 0xD0])?;
    cpu.set_d(1, 1);
    cpu.set_a(0, 0x00FF_0000);
    bus.write8(0x00FF_0000, 0)?;
    cpu.step(&mut bus)?;
    assert_eq!(bus.read8(0x00FF_0000)?, 0x02);
    Ok(())
}

#[test]
fn data_register_bit_numbers_mask_by_31() -> Result<(), Box<dyn Error>> {
    // BTST #33,D0 tests bit 1.
    let (mut cpu, mut bus) = machine(&[0x08, 0x00, 0x00, 0x21])?;
    cpu.set_d(0, 0x0000_0002);
    cpu.step(&mut bus)?;
    assert!(!flag(&cpu, FLAG_Z));
    Ok(())
}
