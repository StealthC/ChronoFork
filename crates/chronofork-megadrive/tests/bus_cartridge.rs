//! Bus delegation to the modelled cartridge device.
//!
//! The bus keeps its typed [`BusError`](chronofork_megadrive::BusError) mapping
//! while the cartridge owns the ROM, mapper, SRAM and TMSS windows. The images
//! here are synthetic headers written by this test, never commercial data.

#![allow(clippy::expect_used)]

use chronofork_megadrive::cartridge::{Cartridge, TmssModel};
use chronofork_megadrive::{BusError, MegadriveBus};

const IMAGE_LEN: usize = 0x4000;

/// Writes the shared synthetic header over a zeroed 16 KiB image.
fn rom_with_header() -> Vec<u8> {
    let mut rom = vec![0_u8; IMAGE_LEN];
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the test image holds a header"
    );
    rom
}

fn bus() -> MegadriveBus {
    MegadriveBus::new(Cartridge::parse(rom_with_header()).expect("parse"))
}

#[test]
fn bus_reads_rom_and_mapper_through_the_cartridge() {
    let mut bus = bus();
    assert_eq!(bus.read16(0x000000).expect("word"), 0x0000);
    bus.write8(0x00A1_30F1, 0x01).expect("enable sram");
    bus.write8(0x0020_0000, 0x11).expect("sram write");
    assert_eq!(bus.read8(0x0020_0000).expect("sram read"), 0x11);
    assert_eq!(bus.tmss().model(), TmssModel::Disabled);
    assert_eq!(bus.cartridge().sram()[0], 0x11);
}

#[test]
fn bus_maps_the_version_register_from_the_cartridge() {
    let mut bus = bus();
    assert_eq!(bus.read8(0x00A1_0001).expect("version"), 0xA1);
    assert_eq!(bus.peek8(0x00A1_0001).expect("peek version"), 0xA1);
}

#[test]
fn bus_maps_cartridge_errors_to_typed_bus_errors() {
    let mut bus = bus();
    bus.write8(0x00A1_30F3, 9).expect("select blank bank");
    assert_eq!(
        bus.read8(0x080000),
        Err(BusError::UnmappedRead { address: 0x080000 })
    );
    assert_eq!(
        bus.read16(0x080000),
        Err(BusError::UnmappedRead { address: 0x080000 })
    );
    assert!(matches!(
        bus.read16(0x080001),
        Err(BusError::OddAddress { .. })
    ));
    assert_eq!(
        bus.write8(0x000000, 0),
        Err(BusError::WriteProtected { address: 0x000000 })
    );
}

#[test]
fn tmss_lockout_consumes_vdp_writes_without_touching_the_vdp() {
    let mut cartridge = Cartridge::parse(rom_with_header()).expect("parse");
    cartridge.set_tmss_model(TmssModel::Enabled);
    let mut bus = MegadriveBus::new(cartridge);
    assert!(bus.tmss().locked());

    // A register write while locked is acknowledged but discarded: register 15
    // and the address latch stay at their reset values.
    bus.write16(0x00C0_0004, 0x8F02)
        .expect("locked control write is consumed");
    assert_eq!(bus.vdp().register(15), Some(0));
    assert_eq!(bus.vdp().address(), 0);

    bus.write32(0x00A1_4000, 0x5345_4741).expect("SEGA unlock");
    assert!(!bus.tmss().locked());
    bus.write16(0x00C0_0004, 0x8F02)
        .expect("unlocked control write");
    assert_eq!(bus.vdp().register(15), Some(2));
}

#[test]
fn only_a_long_tmss_write_unlocks_on_the_bus() {
    let mut cartridge = Cartridge::parse(rom_with_header()).expect("parse");
    cartridge.set_tmss_model(TmssModel::Enabled);
    let mut bus = MegadriveBus::new(cartridge);
    bus.write8(0x00A1_4001, 0x02)
        .expect("byte write acknowledged");
    assert!(bus.tmss().locked());
    bus.write16(0x00A1_4002, 0x0200)
        .expect("word write acknowledged");
    assert!(bus.tmss().locked());
    bus.write32(0x00A1_4000, 0x5345_4741)
        .expect("SEGA long write");
    assert!(!bus.tmss().locked());
}

#[test]
fn locked_data_port_write_leaves_the_preset_vdp_address_and_registers_unchanged() {
    let cartridge = Cartridge::parse(rom_with_header()).expect("parse");
    let mut bus = MegadriveBus::new(cartridge);
    // Preset the command/address latch through an unlocked control-port address
    // write: VRAM write, address 0x10.
    bus.write16(0x00C0_0004, 0x4010).expect("command word");
    bus.write16(0x00C0_0004, 0x0000).expect("address word");
    let address_before = bus.vdp().address();
    let register_before = bus.vdp().register(15);
    let vram_before = bus.vdp().vram()[0x10..0x14].to_vec();
    assert_eq!(address_before, 0x10);

    // Re-lock TMSS after the latch was preset, then attempt data-port and
    // register writes: both are consumed and neither the latch, register 15 nor
    // VRAM changes.
    bus.cartridge_mut().set_tmss_model(TmssModel::Enabled);
    assert!(bus.tmss().locked());
    bus.write16(0x00C0_0000, 0x1234)
        .expect("locked data write is consumed");
    bus.write8(0x00C0_0000, 0x5A)
        .expect("locked byte data write is consumed");
    bus.write16(0x00C0_0004, 0x8F02)
        .expect("locked register write is consumed");

    assert_eq!(bus.vdp().address(), address_before);
    assert_eq!(bus.vdp().register(15), register_before);
    assert_eq!(&bus.vdp().vram()[0x10..0x14], vram_before.as_slice());
}

#[test]
fn bus_protected_sram_writes_are_rejected_atomically() {
    let mut bus = bus();
    bus.write8(0x00A1_30F1, 0x03).expect("enable and protect");
    assert_eq!(
        bus.write16(0x0020_0000, 0xABCD),
        Err(BusError::WriteProtected {
            address: 0x0020_0000
        })
    );
    assert_eq!(
        bus.write32(0x0020_0000, 0xABCD_EF01),
        Err(BusError::WriteProtected {
            address: 0x0020_0000
        })
    );
    bus.write8(0x00A1_30F1, 0x01).expect("unprotect");
    assert_eq!(&bus.cartridge().sram()[..4], &[0, 0, 0, 0]);
}

#[test]
fn debugger_peeks_never_mutate_mapper_or_sram() {
    let mut bus = bus();
    bus.write8(0x00A1_30F3, 2).expect("select bank 2");
    bus.write8(0x00A1_30F1, 0x01).expect("enable sram");
    bus.write8(0x0020_0000, 0x5A).expect("sram write");
    let banks_before = bus.cartridge().banks();
    let control_before = bus.cartridge().sram_control();

    assert_eq!(bus.peek8(0x0020_0000).expect("sram peek"), 0x5A);
    // The mapper register file is write-only: reads are typed unmapped, but the
    // peek path still never mutates the selected bank.
    assert_eq!(
        bus.peek8(0x00A1_30F3),
        Err(BusError::UnmappedRead {
            address: 0x00A1_30F3
        })
    );
    assert_eq!(
        bus.read8(0x00A1_30F3),
        Err(BusError::UnmappedRead {
            address: 0x00A1_30F3
        })
    );
    // The debugger range path deliberately exposes the raw image windows rather
    // than the mapped device view; the SRAM address is outside this image.
    assert!(matches!(
        bus.read_memory(0x0020_0000, 1),
        Err(BusError::UnmappedRead { .. })
    ));
    assert_eq!(bus.read_memory(0, 2).expect("rom read"), vec![0, 0]);

    assert_eq!(bus.cartridge().banks(), banks_before);
    assert_eq!(bus.cartridge().sram_control(), control_before);
    assert_eq!(bus.cartridge().sram()[0], 0x5A);
}
