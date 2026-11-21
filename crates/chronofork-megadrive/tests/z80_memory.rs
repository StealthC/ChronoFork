//! Z80 memory map, bank register and the 68000-visible Z80 RAM window.

use std::error::Error;

use chronofork_megadrive::audio::YmBank;
use chronofork_megadrive::audio::psg::OPEN_BUS_VALUE;
use chronofork_megadrive::bus::{
    PSG_Z80_ALIAS, PSG_Z80_BASE, YM2612_Z80_BASE, YM2612_Z80_END, Z80_BANK_REGISTER,
    Z80_BANK_REGISTER_ADDRESS, Z80_ROM_WINDOW_BASE,
};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::z80::{Z80Bus, Z80Error};
use chronofork_megadrive::{BusError, MegadriveBus};

const IMAGE_LEN: usize = 0x2_0000;

/// Synthetic 128 KiB image with a marker byte in each 32 KiB bank.
fn rom() -> Vec<u8> {
    let mut rom = vec![0_u8; IMAGE_LEN];
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the test image holds a header"
    );
    rom[0] = 0x11;
    rom[0x8000] = 0x22;
    rom[0x1_0000] = 0x33;
    rom[0x1_8000] = 0x44;
    rom
}

fn bus() -> Result<MegadriveBus, Box<dyn Error>> {
    Ok(MegadriveBus::new(Cartridge::parse(rom())?))
}

#[test]
fn z80_ram_is_8k_and_mirrored_through_the_first_16k() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    assert_eq!(bus.z80_ram().len(), 0x2000);
    Z80Bus::write8(&mut bus, 0x0000, 0x5A)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x0000)?, 0x5A);
    assert_eq!(Z80Bus::read8(&mut bus, 0x2000)?, 0x5A, "mirror of 0x0000");
    assert_eq!(Z80Bus::read8(&mut bus, 0x1FFF)?, 0x00);
    Z80Bus::write8(&mut bus, 0x3FFF, 0x77)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x1FFF)?, 0x77, "mirror of 0x3FFF");
    assert_eq!(Z80Bus::read8(&mut bus, 0x0000)?, 0x5A);
    Ok(())
}

#[test]
fn cpu_window_maps_the_eight_kib_z80_ram() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    bus.write8(0x00A0_0000, 0x12)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x0000)?, 0x12);
    bus.write16(0x00A0_0000, 0xABCD)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x0000)?, 0xAB);
    assert_eq!(Z80Bus::read8(&mut bus, 0x0001)?, 0xCD);
    assert_eq!(bus.read16(0x00A0_0000)?, 0xABCD);
    Z80Bus::write8(&mut bus, 0x1FFF, 0x99)?;
    assert_eq!(bus.read8(0x00A0_1FFF)?, 0x99);
    assert_eq!(bus.z80_ram()[0x1FFF], 0x99);

    assert!(matches!(
        bus.read8(0x00A0_2000),
        Err(BusError::UnmappedRead { .. })
    ));
    assert!(matches!(
        bus.write8(0x00A0_2000, 0),
        Err(BusError::UnmappedWrite { .. })
    ));
    Ok(())
}

#[test]
fn z80_bank_selects_32k_rom_window() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    assert_eq!(Z80Bus::read8(&mut bus, Z80_ROM_WINDOW_BASE)?, 0x11);
    bus.write16(Z80_BANK_REGISTER, 0x0001)?;
    assert_eq!(bus.z80_bank(), 1);
    assert_eq!(Z80Bus::read8(&mut bus, 0x8000)?, 0x22);
    assert_eq!(
        Z80Bus::read8(&mut bus, 0xFFFF)?,
        0x00,
        "second half of bank 1"
    );
    bus.write16(Z80_BANK_REGISTER, 0x0002)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x8000)?, 0x33);
    bus.write16(Z80_BANK_REGISTER, 0x0003)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x8000)?, 0x44);

    // Only nine bits are latched.
    bus.write16(Z80_BANK_REGISTER, 0x0201)?;
    assert_eq!(bus.z80_bank(), 0x0001);
    Ok(())
}

#[test]
fn z80_bank_window_reads_raw_rom_and_never_passes_through_the_mapper() -> Result<(), Box<dyn Error>>
{
    let mut bus = bus()?;
    // Select a different 68000 mapper bank; the Z80 window must ignore it.
    bus.write8(0x00A1_30F3, 0x02)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x8000)?, 0x11);
    Ok(())
}

#[test]
fn z80_side_bank_write_selects_the_same_window() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    Z80Bus::write8(&mut bus, Z80_BANK_REGISTER_ADDRESS, 0x01)?;
    assert_eq!(bus.z80_bank(), 1);
    assert_eq!(Z80Bus::read8(&mut bus, 0x8000)?, 0x22);

    // The byte write is eight bits wide and clears bit 8.
    bus.write16(Z80_BANK_REGISTER, 0x0100)?;
    assert_eq!(bus.z80_bank(), 0x0100);
    Z80Bus::write8(&mut bus, Z80_BANK_REGISTER_ADDRESS, 0x02)?;
    assert_eq!(bus.z80_bank(), 0x0002);
    Ok(())
}

#[test]
fn bank_beyond_the_image_is_a_typed_memory_error() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    bus.write16(Z80_BANK_REGISTER, 0x01FF)?;
    assert!(matches!(
        Z80Bus::read8(&mut bus, 0x8000),
        Err(Z80Error::Memory { address: 0x8000 })
    ));
    Ok(())
}

#[test]
fn device_windows_are_decoded_and_do_not_alias_ram_or_rom() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    // The YM2612 window is wired and reaches the device.
    chronofork_megadrive::z80::Z80Bus::write8(&mut bus, YM2612_Z80_BASE, 0x30)?;
    chronofork_megadrive::z80::Z80Bus::write8(&mut bus, YM2612_Z80_BASE + 1, 0x0B)?;
    assert_eq!(bus.ym().register(YmBank::One, 0x30), 0x0B);
    assert_eq!(
        chronofork_megadrive::z80::Z80Bus::read8(&mut bus, YM2612_Z80_END)?,
        bus.ym().status()
    );
    // The PSG window is decoded, write-only and cannot alias RAM or ROM: a read
    // returns the open value and a write reaches the chip's attenuation register.
    for address in [PSG_Z80_BASE, 0x7F03, PSG_Z80_ALIAS] {
        assert_eq!(
            Z80Bus::read8(&mut bus, address),
            Ok(OPEN_BUS_VALUE),
            "read {address:#06x} returns the PSG open value"
        );
        Z80Bus::write8(&mut bus, address, 0x9F)?;
        assert_eq!(bus.psg().attenuation(0), Some(0x0F));
    }
    for address in [0x4004, 0x7F04] {
        assert!(
            matches!(
                Z80Bus::read8(&mut bus, address),
                Err(Z80Error::Memory { .. })
            ),
            "read {address:#06x} must be decoded, not aliased"
        );
        assert!(
            matches!(
                Z80Bus::write8(&mut bus, address, 0x55),
                Err(Z80Error::Memory { .. })
            ),
            "write {address:#06x} must be decoded, not aliased"
        );
    }
    assert_eq!(bus.z80_ram()[0], 0x00);
    assert_eq!(bus.rom()[0], 0x11);
    assert!(matches!(
        Z80Bus::write8(&mut bus, 0x8000, 0x00),
        Err(Z80Error::Memory { .. })
    ));
    assert_eq!(bus.rom()[0], 0x11, "banked ROM writes are rejected");
    Ok(())
}

#[test]
fn z80_io_space_is_not_wired() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    assert_eq!(
        Z80Bus::input(&mut bus, 0x1234),
        Err(Z80Error::Io { port: 0x1234 })
    );
    assert_eq!(
        Z80Bus::output(&mut bus, 0x1234, 0x56),
        Err(Z80Error::Io { port: 0x1234 })
    );
    Ok(())
}
