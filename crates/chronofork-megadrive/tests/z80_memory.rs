//! Z80 memory map, bank register and the 68000-visible Z80 RAM window.

use std::error::Error;

use chronofork_megadrive::audio::YmBank;
use chronofork_megadrive::audio::psg::OPEN_BUS_VALUE;
use chronofork_megadrive::bus::{
    PSG_Z80_ALIAS, YM2612_Z80_BASE, YM2612_Z80_END, YM2612_Z80_SOUND_END, Z80_BANK_REGISTER,
    Z80_BANK_REGISTER_ADDRESS, Z80_ROM_WINDOW_BASE, Z80_UNDECODED_READ_VALUE,
};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::vdp::VdpError;
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
fn banked_z80_write_to_decoded_work_ram_is_not_silently_dropped() -> Result<(), Box<dyn Error>> {
    // The banked window is not universal ROM: the arbiter translates the Z80
    // address into the 68000 space and forwards the access
    // (`Devices/MD1600IO/MDBusArbiter.cpp:537-560`). Bank 0x1FE selects 0xFF0000,
    // which is work RAM, so a write there must land.
    let mut bus = bus()?;
    bus.write16(Z80_BANK_REGISTER, 0x01FE)?;
    bus.write8(0x00FF_0000, 0x12)?;
    Z80Bus::write8(&mut bus, 0x8000, 0xA5)?;
    assert_eq!(
        bus.read8(0x00FF_0000)?,
        0xA5,
        "the Z80 write reached the translated RAM target"
    );
    assert_eq!(
        bus.unmapped_z80_accesses(),
        0,
        "a served access is not counted as a discard"
    );

    // A bank that translates into a hole has no effect and is counted instead of
    // pretending to succeed silently.
    bus.write16(Z80_BANK_REGISTER, 0x0100)?; // 0x100 << 15 = 0x800000, unmapped
    Z80Bus::write8(&mut bus, 0x8000, 0xA5)?;
    assert_eq!(bus.unmapped_z80_accesses(), 1);

    // A bank aimed at the cartridge stays read-only: the write has no effect and
    // the target is decoded, so it is not counted.
    bus.write16(Z80_BANK_REGISTER, 0x0000)?;
    Z80Bus::write8(&mut bus, 0x8000, 0x00)?;
    assert_eq!(bus.rom()[0], 0x11, "ROM untouched");
    assert_eq!(
        bus.unmapped_z80_accesses(),
        1,
        "ROM is decoded, not discarded"
    );
    Ok(())
}

#[test]
fn z80_byte_read_of_the_eight_bit_vram_mode_is_not_a_capability_gap() -> Result<(), Box<dyn Error>>
{
    // The `0b1100` 8-bit VRAM read used to be a documented operation this slice did
    // not implement (W1X / W1U-F2). It is implemented from W1Z on, so reading it
    // through the Z80 VDP window returns the data-port byte and counts in neither
    // the capability-gap nor the invalid-target discard bucket.
    let mut bus = bus()?;
    // Command code 0x0C: word 1 CD1:CD0 = 00, word 2 CD5:CD2 = 0b0011.
    bus.write16(0x00C0_0004, 0x0000)?;
    bus.write16(0x00C0_0004, 0x0030)?;
    assert_eq!(bus.vdp().capability_gap_accesses(), 0);
    // Zero VRAM and an empty FIFO: the read buffer is 0x0000 and the even portal
    // byte 0x7F00 selects its high half.
    assert_eq!(Z80Bus::read8(&mut bus, 0x7F00), Ok(0x00));
    assert_eq!(
        bus.vdp().capability_gap_accesses(),
        0,
        "an implemented operation is not a capability gap"
    );
    assert_eq!(
        bus.vdp().discarded_data_port_accesses(),
        0,
        "an implemented operation is not an invalid-target discard"
    );

    // The 68000 path returns the same read-buffer half.
    assert_eq!(bus.read8(0x00C0_0000), Ok(0x00));
    assert_eq!(bus.vdp().capability_gap_accesses(), 0);
    assert_eq!(bus.vdp().discarded_data_port_accesses(), 0);
    Ok(())
}

/// Programs the VDP for a 68000-memory DMA into VRAM (command code `0x21`) at address 0.
fn program_memory_dma(bus: &mut MegadriveBus) -> Result<(), Box<dyn Error>> {
    bus.write16(0x00C0_0004, 0x8110)?; // register 0x01: DMA enabled
    bus.write16(0x00C0_0004, 0x4000)?; // command word 1: CD1:CD0 = 01
    bus.write16(0x00C0_0004, 0x0080)?; // command word 2: code 0x21 = memory -> VRAM
    Ok(())
}

#[test]
fn z80_vdp_memory_dma_is_classified_as_dma_not_an_invalid_target() -> Result<(), Box<dyn Error>> {
    // W1Y-F1 case 1: a memory-source DMA through the Z80 portal used to answer `Ok` and land
    // in the invalid-target discard bucket. It is a refused DMA: its own bucket, and the
    // typed rejection surfaces for the access.
    let mut bus = bus()?;
    program_memory_dma(&mut bus)?;
    // The 68000 path keeps the typed rejection (the policy that must not be weakened).
    assert!(matches!(
        bus.write8(0x00C0_0000, 0xAA),
        Err(BusError::Vdp(VdpError::DmaMemorySourceUnavailable { .. }))
    ));
    assert_eq!(bus.vdp().discarded_data_port_accesses(), 0);
    assert_eq!(bus.vdp().capability_gap_accesses(), 0);

    // The same rejected transfer through the Z80 window is classified as DMA-unavailable.
    assert!(matches!(
        Z80Bus::write8(&mut bus, 0x7F00, 0xAA),
        Err(Z80Error::Memory { address: 0x7F00 })
    ));
    assert_eq!(bus.z80_dma_unavailable_accesses(), 1);
    assert_eq!(bus.vdp().discarded_data_port_accesses(), 0, "not a discard");
    assert_eq!(
        bus.vdp().capability_gap_accesses(),
        1,
        "an unavailable transfer is a capability gap, isolated in its own DMA bucket too"
    );
    assert_eq!(bus.unmapped_z80_accesses(), 0, "not a hole");
    Ok(())
}

#[test]
fn z80_banked_window_to_the_vdp_keeps_the_dma_classification() -> Result<(), Box<dyn Error>> {
    // W1Y-F1 case 2: bank 0x180 -> 0xC00000 (the VDP data port) used to answer `Ok` and count
    // as unmapped. The target is decoded, and the refusal is the same DMA rejection.
    let mut bus = bus()?;
    program_memory_dma(&mut bus)?;
    bus.write16(0x00A0_6000, 0x0180)?; // nine-bit bank register
    assert!(matches!(
        Z80Bus::write8(&mut bus, 0x8000, 0xAA),
        Err(Z80Error::Memory { address: 0x8000 })
    ));
    assert_eq!(bus.z80_dma_unavailable_accesses(), 1);
    assert_eq!(
        bus.unmapped_z80_accesses(),
        0,
        "a decoded VDP target is not an unmapped hole"
    );
    assert_eq!(bus.vdp().discarded_data_port_accesses(), 0);

    // A bank that really lands in a hole keeps the unmapped bucket.
    bus.write16(0x00A0_6000, 0x0100)?; // 0x100 << 15 = 0x800000, unowned
    Z80Bus::write8(&mut bus, 0x8000, 0xAA)?;
    assert_eq!(bus.unmapped_z80_accesses(), 1);
    assert_eq!(
        bus.z80_dma_unavailable_accesses(),
        1,
        "buckets stay separate"
    );
    Ok(())
}

#[test]
fn z80_portal_reads_the_eight_bit_vram_mode_without_refreshing_on_a_pending_write()
-> Result<(), Box<dyn Error>> {
    // The Z80 VDP window `0x7F00..=0x7F03` maps to the VDP data port lanes
    // (`Devices/MD1600IO/MDBusArbiter.cpp:347-384`). The cache step only runs while
    // the write FIFO is empty (`Devices/315-5313/S315-5313_General.cpp:1748`,
    // `:1850-1856`), so with a pending write each portal byte returns the read
    // buffer's current value and consumes nothing from the FIFO.
    let mut bus = bus()?;
    bus.write16(0x00C0_0004, 0x4000)?; // command word 1: VRAM write at 0
    bus.write16(0x00C0_0004, 0x0000)?; // command word 2
    bus.write16(0x00C0_0000, 0x1234)?; // pending data-port write
    // Command code 0x0C at address 0x0000.
    bus.write16(0x00C0_0004, 0x0000)?;
    bus.write16(0x00C0_0004, 0x0030)?;

    assert_eq!(Z80Bus::read8(&mut bus, 0x7F00)?, 0x00);
    assert_eq!(Z80Bus::read8(&mut bus, 0x7F01)?, 0x00);
    assert_eq!(
        bus.vdp().fifo_len(),
        1,
        "a portal read neither consumes nor drains the pending write"
    );
    assert_eq!(
        bus.vdp().capability_gap_accesses(),
        0,
        "an implemented operation is not a capability gap"
    );
    assert_eq!(bus.vdp().discarded_data_port_accesses(), 0);
    Ok(())
}

#[test]
fn the_three_z80_rejection_classes_never_collapse_into_one_bucket() -> Result<(), Box<dyn Error>> {
    // The `0x0C` 8-bit VRAM read is implemented from W1Z on: a successful read
    // feeds neither the capability-gap nor the discard bucket.
    let mut implemented = bus()?;
    implemented.write16(0x00C0_0004, 0x0000)?;
    implemented.write16(0x00C0_0004, 0x0030)?; // code 0x0C
    assert_eq!(Z80Bus::read8(&mut implemented, 0x7F00), Ok(0x00));
    assert_eq!(implemented.vdp().capability_gap_accesses(), 0);
    assert_eq!(implemented.vdp().discarded_data_port_accesses(), 0);
    assert_eq!(implemented.z80_dma_unavailable_accesses(), 0);

    // Invalid target: the VSRAM-write code used as a read target.
    let mut invalid = bus()?;
    invalid.write16(0x00C0_0004, 0x4000)?;
    invalid.write16(0x00C0_0004, 0x0010)?; // code 0x05
    assert!(matches!(Z80Bus::read8(&mut invalid, 0x7F00), Ok(0x00)));
    assert_eq!(invalid.vdp().discarded_data_port_accesses(), 1);
    assert_eq!(invalid.vdp().capability_gap_accesses(), 0);
    assert_eq!(invalid.z80_dma_unavailable_accesses(), 0);

    // DMA unavailable: the memory-source transfer.
    let mut dma = bus()?;
    program_memory_dma(&mut dma)?;
    assert!(matches!(
        Z80Bus::write8(&mut dma, 0x7F00, 0xAA),
        Err(Z80Error::Memory { .. })
    ));
    assert_eq!(dma.z80_dma_unavailable_accesses(), 1);
    assert_eq!(dma.vdp().discarded_data_port_accesses(), 0);
    assert_eq!(
        dma.vdp().capability_gap_accesses(),
        1,
        "capability is the superset; the DMA bucket isolates the sub-class"
    );
    Ok(())
}

#[test]
fn a_bank_beyond_the_image_reads_the_undecoded_value() -> Result<(), Box<dyn Error>> {
    // The bank is selected but the image is shorter than the window, so no device
    // drives the byte: the guest reads the documented value and the run continues
    // instead of the host ending it.
    let mut bus = bus()?;
    bus.write16(Z80_BANK_REGISTER, 0x01FF)?;
    assert_eq!(Z80Bus::read8(&mut bus, 0x8000)?, Z80_UNDECODED_READ_VALUE);
    assert_eq!(bus.unmapped_z80_accesses(), 1);
    Ok(())
}

#[test]
fn the_sound_region_repeats_the_four_ym_ports_by_address_mask() -> Result<(), Box<dyn Error>> {
    // The pinned oracle gates the sound chip-select over `0x4000..=0x5FFF`
    // (`Devices/MD1600IO/MDBusArbiter.cpp:1048-1049`) and maps the YM2612 with
    // `AddressMask=0x0003` (`Data/Modules/Sega Mega Drive 1600.xml:340`), so the
    // bank/latch pairs repeat every four bytes. `0x5F8B & 3 == 3` is the bank-2
    // data port — the write the boot fixtures make at step 118 156.
    let mut bus = bus()?;

    // Bank 1: latch a register through `0x4000`, write it through a mirror of the
    // bank-1 data port (`0x4004 + 1 = 0x4005`, `0x5FFD`).
    Z80Bus::write8(&mut bus, YM2612_Z80_BASE, 0x30)?;
    Z80Bus::write8(&mut bus, 0x4005, 0x0B)?;
    assert_eq!(bus.ym().register(YmBank::One, 0x30), 0x0B);

    // Bank 2: latch through `0x4002`, write through `0x5F8B` (& 3 = 3).
    Z80Bus::write8(&mut bus, 0x4002, 0x40)?;
    Z80Bus::write8(&mut bus, 0x5F8B, 0x5A)?;
    assert_eq!(bus.ym().register(YmBank::Two, 0x40), 0x5A);

    // The region's ends decode to the same mask: `0x4004` (& 3 = 0) latches the
    // bank-1 address and `0x5FFF` (& 3 = 3) writes bank-2 data. This slice keeps
    // one address latch per bank, so the data byte must go through the *same*
    // bank's data port (`0x4005` here); the pinned oracle models the two data
    // ports as equivalent and commits whichever address port was written last
    // (`Devices/YM2612/YM2612.cpp:2334-2350`) — a divergence recorded in the W1P
    // report, not fixed here because the YM device is outside this task's scope.
    Z80Bus::write8(&mut bus, 0x4004, 0x44)?;
    Z80Bus::write8(&mut bus, 0x4005, 0x99)?;
    assert_eq!(bus.ym().register(YmBank::One, 0x44), 0x99);

    // A mirrored bank-2 data byte still reaches bank 2.
    Z80Bus::write8(&mut bus, 0x4002, 0x41)?;
    Z80Bus::write8(&mut bus, YM2612_Z80_SOUND_END, 0x77)?;
    assert_eq!(bus.ym().register(YmBank::Two, 0x41), 0x77);

    // Reads decode the same way: any `SOUND` byte returns the status register.
    assert_eq!(Z80Bus::read8(&mut bus, 0x5F8B)?, bus.ym().status());
    assert_eq!(
        Z80Bus::read8(&mut bus, YM2612_Z80_SOUND_END)?,
        bus.ym().status()
    );

    // Outside the region an unowned byte is served by the undecoded path: the
    // write is ignored, the read returns the documented value, and the run
    // continues (W1R continuity policy).
    assert_eq!(bus.unmapped_z80_accesses(), 0);
    Z80Bus::write8(&mut bus, 0x6100, 0)?;
    assert_eq!(bus.unmapped_z80_accesses(), 1);
    assert_eq!(
        Z80Bus::read8(&mut bus, 0x6100)?,
        Z80_UNDECODED_READ_VALUE,
        "an unowned byte reads as the documented value"
    );
    assert_eq!(bus.unmapped_z80_accesses(), 2);
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
    for address in [PSG_Z80_ALIAS, 0x7F10, 0x7F17] {
        assert_eq!(
            Z80Bus::read8(&mut bus, address),
            Ok(OPEN_BUS_VALUE),
            "read {address:#06x} returns the PSG open value"
        );
        Z80Bus::write8(&mut bus, address, 0x9F)?;
        assert_eq!(bus.psg().attenuation(0), Some(0x0F));
    }
    // `0x4004` used to be probed here as an undecoded byte; it is now inside the
    // YM2612 `SOUND` region (`0x4000..=0x5FFF`) and covered by
    // `the_sound_region_repeats_the_four_ym_ports_by_address_mask`. The probes
    // below stay in the remaining gaps: past the bank register and past the PSG
    // window.
    // The gaps are still not aliased to a device: they are served by the
    // undecoded path (W1R), which is observable through the counter.
    let mut counted = bus.unmapped_z80_accesses();
    for address in [0x6100, 0x7F04] {
        assert_eq!(
            Z80Bus::read8(&mut bus, address)?,
            Z80_UNDECODED_READ_VALUE,
            "read {address:#06x} is unowned, not aliased"
        );
        Z80Bus::write8(&mut bus, address, 0x55)?;
        counted += 2;
        assert_eq!(bus.unmapped_z80_accesses(), counted);
    }
    assert_eq!(bus.z80_ram()[0], 0x00);
    assert_eq!(bus.rom()[0], 0x11);
    // The banked ROM window is decoded but read-only: a write there is ignored
    // (mapped, so not counted) and the banked byte is untouched.
    let before = bus.unmapped_z80_accesses();
    Z80Bus::write8(&mut bus, 0x8000, 0x00)?;
    assert_eq!(bus.rom()[0], 0x11, "banked ROM writes are ignored");
    assert_eq!(
        bus.unmapped_z80_accesses(),
        before,
        "ROM is mapped, not unowned"
    );
    Ok(())
}

#[test]
fn z80_io_space_is_undecoded_and_never_fatal() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    // Nothing owns the Z80 I/O space, so it is served by the same undecoded
    // channel: an IN returns the documented value, an OUT is ignored, and neither
    // can end the run.
    assert_eq!(bus.unmapped_z80_accesses(), 0);
    assert_eq!(Z80Bus::input(&mut bus, 0x1234)?, Z80_UNDECODED_READ_VALUE);
    Z80Bus::output(&mut bus, 0x1234, 0x56)?;
    assert_eq!(bus.unmapped_z80_accesses(), 2);
    Ok(())
}
