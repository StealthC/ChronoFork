#![allow(clippy::expect_used)]

//! Cartridge header, region, mapper, SRAM and TMSS behaviour.
//!
//! Every image here is independently authored: the synthetic builder below
//! writes a documented Mega Drive header over caller-provided bytes. The two
//! fixture tests read the local-only `.research/roms/` images through
//! `required_rom_path`, which makes an absent fixture a declared skip that
//! prints its reason — and a hard failure when
//! `$CHRONOFORK_REQUIRE_ROM_FIXTURES=1` claims the fixtures are available. No ROM
//! bytes are checked into the repository or embedded in this file.

use chronofork_megadrive::CartridgeView;
use chronofork_megadrive::cartridge::{
    Cartridge, CartridgeError, CartridgeSystem, Region, Tmss, TmssModel,
};
use chronofork_megadrive::interrupts::VideoStandard;
use chronofork_megadrive::test_support::required_rom_path;

const IMAGE_LEN: usize = 0x4000;

/// Builds an image with the shared synthetic header and this test's name facts.
///
/// The shared writer computes the hardware checksum over `0x200..`, so the
/// header-only overrides below (names, serial, region, ROM end) cannot
/// invalidate it.
fn header_rom(region: &str, rom_end: u32) -> Vec<u8> {
    let mut rom = vec![0_u8; IMAGE_LEN];
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, region),
        "the test image holds a header"
    );
    rom[0x120..0x130].copy_from_slice(b"KID CHAMELEON   ");
    rom[0x150..0x160].copy_from_slice(b"KID CHAMELEON   ");
    rom[0x180..0x18E].copy_from_slice(b"GM MK-1010 -00");
    rom[0x1A4..0x1A8].copy_from_slice(&rom_end.to_be_bytes());
    rom
}

#[test]
fn parses_header_and_region() {
    let cartridge = Cartridge::parse(header_rom("JUE", 0x3FFF)).expect("header parses");
    let header = cartridge.header();
    assert_eq!(header.system, CartridgeSystem::MegaDrive);
    assert_eq!(header.domestic_name, "KID CHAMELEON");
    assert_eq!(header.overseas_name, "KID CHAMELEON");
    assert_eq!(header.serial, "GM MK-1010 -00");
    assert_eq!(header.region, Region::parse("JUE").expect("valid region"));
    assert_eq!(header.region.to_string(), "JUE");
    assert!(header.region.contains(Region::J));
    assert!(header.region.contains(Region::U));
    assert!(header.region.contains(Region::E));
    assert!(header.region.is_export());
    assert!(!header.region.is_pal());
    assert_eq!(header.checksum_stored, header.checksum_computed);
    assert_eq!(header.rom_start, 0);
    assert_eq!(header.rom_end, 0x3FFF);
    assert_eq!(header.sram, None);
}

#[test]
fn classifies_genesis_and_derives_the_video_standard() {
    let mut rom = header_rom("E", 0x3FFF);
    rom[0x100..0x110].copy_from_slice(b"SEGA GENESIS    ");
    let genesis = Cartridge::parse(rom).expect("genesis header");
    assert_eq!(genesis.header().system, CartridgeSystem::Genesis);
    assert_eq!(genesis.derived_video_standard(), VideoStandard::Pal);

    let export = Cartridge::parse(header_rom("JUE", 0x3FFF)).expect("header");
    assert_eq!(export.derived_video_standard(), VideoStandard::Ntsc);
    let japan = Cartridge::parse(header_rom("J", 0x3FFF)).expect("header");
    assert_eq!(japan.derived_video_standard(), VideoStandard::Ntsc);
}

#[test]
fn rejects_unknown_region_missing_region_and_short_image() {
    assert!(matches!(
        Cartridge::parse(header_rom("JUX", 0x3FFF)),
        Err(CartridgeError::UnknownRegion { value: b'X' })
    ));
    assert!(matches!(
        Cartridge::parse(header_rom("", 0x3FFF)),
        Err(CartridgeError::MissingRegion)
    ));
    assert!(matches!(
        Cartridge::parse(vec![0; 7]),
        Err(CartridgeError::TooShort)
    ));
}

#[test]
fn rejects_a_declared_end_beyond_the_image_and_other_systems() {
    assert!(matches!(
        Cartridge::parse(header_rom("JUE", 0x5000)),
        Err(CartridgeError::HeaderMismatch {
            declared_end: 0x5000,
            image_len: 0x4000,
        })
    ));

    let mut rom = header_rom("JUE", 0x3FFF);
    rom[0x100..0x110].copy_from_slice(b"SEGA 32X        ");
    assert!(matches!(
        Cartridge::parse(rom),
        Err(CartridgeError::MissingSegaSignature { .. })
    ));
}

#[test]
fn text_fields_are_bounded_ascii() {
    let mut rom = header_rom("JUE", 0x3FFF);
    // A non-printable byte terminates the field instead of being decoded.
    rom[0x120 + 4] = 0xFF;
    let cartridge = Cartridge::parse(rom).expect("header");
    assert_eq!(cartridge.header().domestic_name, "KID");
    assert!(cartridge.header().serial.len() <= 14);
}

#[test]
fn sram_spec_is_parsed_from_the_ra_marker_offsets() {
    let mut rom = header_rom("JUE", 0x3FFF);
    // Documented layout: "RA" marker at 0x1B0, type at 0x1B2, start at 0x1B4,
    // end at 0x1B8.
    rom[0x1B0..0x1B2].copy_from_slice(b"RA");
    rom[0x1B2..0x1B4].copy_from_slice(&[0xF8, 0x20]);
    rom[0x1B4..0x1B8].copy_from_slice(&0x0020_0001_u32.to_be_bytes());
    rom[0x1B8..0x1BC].copy_from_slice(&0x0020_FFFF_u32.to_be_bytes());
    let cartridge = Cartridge::parse(rom).expect("header");
    let sram = cartridge.header().sram.expect("declared SRAM");
    assert_eq!(sram.start, 0x0020_0001);
    assert_eq!(sram.end, 0x0020_FFFF);
}

#[test]
fn sram_spec_is_absent_when_the_fields_are_padded_or_the_marker_is_not_ra() {
    // The shared header leaves 0x1B0.. zero-filled.
    let padded = Cartridge::parse(header_rom("JUE", 0x3FFF)).expect("header");
    assert_eq!(padded.header().sram, None);

    let mut spaces = header_rom("JUE", 0x3FFF);
    spaces[0x1B0..0x1B4].fill(b' ');
    let spaces = Cartridge::parse(spaces).expect("header");
    assert_eq!(spaces.header().sram, None);

    // "RO" declares ROM, not battery SRAM, even with a range present.
    let mut rom_only = header_rom("JUE", 0x3FFF);
    rom_only[0x1B0..0x1B2].copy_from_slice(b"RO");
    rom_only[0x1B4..0x1B8].copy_from_slice(&0x0000_0001_u32.to_be_bytes());
    rom_only[0x1B8..0x1BC].copy_from_slice(&0x0000_FFFF_u32.to_be_bytes());
    let rom_only = Cartridge::parse(rom_only).expect("header");
    assert_eq!(rom_only.header().sram, None);
}

/// Resolves a commercial-ROM fixture, or declares the absence of the fixtures.
///
/// Returns `None` only when the fixture is absent *and* the run does not claim
/// the fixtures are available. With `$CHRONOFORK_REQUIRE_ROM_FIXTURES=1` the
/// absence is a hard failure here, so a job that sets the variable can never
/// report a green run without having exercised the fixture.
fn fixture_or_declared_skip(name: &str) -> Option<std::path::PathBuf> {
    match required_rom_path(name) {
        Ok(path) => Some(path),
        Err(reason) => {
            assert!(
                !chronofork_megadrive::test_support::required_rom_fixtures(),
                "{reason}; $CHRONOFORK_REQUIRE_ROM_FIXTURES is set, so this run must not pass without it"
            );
            eprintln!("skipped: {reason}");
            None
        }
    }
}

#[test]
fn fixture_headers_parse_when_present() {
    let Some(path) = fixture_or_declared_skip("Kid Chameleon (USA, Europe).bin") else {
        return;
    };
    let cartridge = Cartridge::parse(std::fs::read(path).expect("read")).expect("parse");
    let header = cartridge.header();
    assert_eq!(header.system, CartridgeSystem::MegaDrive);
    assert_eq!(header.region, Region::parse("JUE").expect("region"));
    assert_eq!(header.serial, "GM MK-1010 -00");
    assert_eq!(header.rom_end, 0xFFFFF);
    assert_eq!(header.checksum_stored, header.checksum_computed);
    assert!(
        header.sram.is_none(),
        "Kid Chameleon declares no battery SRAM"
    );
}

#[test]
fn chakan_fixture_header_parses_when_present() {
    let Some(path) = fixture_or_declared_skip("Chakan (USA, Europe).bin") else {
        return;
    };
    let cartridge = Cartridge::parse(std::fs::read(path).expect("read")).expect("parse");
    let header = cartridge.header();
    assert_eq!(header.system, CartridgeSystem::Genesis);
    assert_eq!(header.region, Region::parse("U").expect("region"));
    assert_eq!(header.serial, "GM MK-1047 -00");
    assert_eq!(header.rom_end, 0xFFFFF);
    assert_eq!(header.checksum_stored, header.checksum_computed);
    assert!(header.sram.is_none(), "Chakan declares no battery SRAM");
    assert!(!CartridgeView::new(&cartridge).sram_declared());
}

// Task 2: Sega mapper and ROM reads.

/// A 1 MiB image whose bank-2 page starts with a unique byte.
fn banked_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x100001];
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the banked image holds a header"
    );
    rom[0x100000] = 0x42;
    rom
}

#[test]
fn identity_banks_read_rom_linearly() {
    let cartridge = Cartridge::parse(banked_rom()).expect("parse");
    assert_eq!(cartridge.read8(0x000000).expect("byte"), 0x00); // reset vector high byte
    assert_eq!(cartridge.read8(0x100000).expect("byte"), 0x42);
    assert_eq!(cartridge.banks(), [1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn mapper_register_switches_a_window() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    cartridge.write8(0x00A1_30F3, 2).expect("select bank 2");
    assert_eq!(cartridge.read8(0x080000).expect("byte"), 0x42);
    assert_eq!(cartridge.banks()[0], 2);
    // A byte register carries eight bits; the direct setter applies the same
    // nine-bit mask a wider restoring path could supply.
    cartridge
        .write8(0x00A1_30F5, 0xFF)
        .expect("mask bank register");
    assert_eq!(cartridge.banks()[1], 0xFF);
    cartridge.set_bank(1, 0x01FF).expect("nine-bit bank");
    assert_eq!(cartridge.banks()[1], 0x01FF);
    assert!(matches!(
        cartridge.set_bank(7, 0),
        Err(CartridgeError::InvalidWindow { window: 7 })
    ));
}

#[test]
fn mapper_registers_are_write_only() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    cartridge.write8(0x00A1_30F3, 2).expect("select bank 2");
    assert_eq!(
        cartridge.read8(0x00A1_30F3),
        Err(CartridgeError::UnmappedRom {
            address: 0x00A1_30F3
        })
    );
    assert_eq!(
        cartridge.read16(0x00A1_30F2),
        Err(CartridgeError::UnmappedRom {
            address: 0x00A1_30F2
        })
    );
    // The modelled state is observable through the accessors, not reads.
    assert_eq!(cartridge.banks()[0], 2);
}

#[test]
fn bank_beyond_image_is_unmapped_and_odd_long_is_rejected() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    cartridge.write8(0x00A1_30F3, 9).expect("select blank bank");
    assert!(matches!(
        cartridge.read8(0x080000),
        Err(CartridgeError::UnmappedRom { .. })
    ));
    assert!(matches!(
        cartridge.read32(0x080000),
        Err(CartridgeError::UnmappedRom { .. })
    ));
    assert!(matches!(
        cartridge.read16(0x080001),
        Err(CartridgeError::OddAddress { .. })
    ));
    assert!(matches!(
        cartridge.read32(0x080001),
        Err(CartridgeError::OddAddress { .. })
    ));
}

#[test]
fn rom_window_writes_are_write_protected() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    assert!(matches!(
        cartridge.write8(0x000000, 0),
        Err(CartridgeError::WriteProtected { address: 0x000000 })
    ));
    assert!(matches!(
        cartridge.write16(0x000000, 0),
        Err(CartridgeError::WriteProtected { .. })
    ));
}

// Task 3: battery SRAM.

#[test]
fn sram_is_visible_only_when_mapper_enables_it() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    assert!(!cartridge.sram_visible());
    assert_eq!(cartridge.sram().len(), 65_536);
    cartridge.write8(0x00A1_30F1, 0x01).expect("enable sram");
    cartridge.write8(0x0020_0000, 0x5A).expect("write sram");
    assert_eq!(cartridge.read8(0x0020_0000).expect("read sram"), 0x5A);
    cartridge.write8(0x00A1_30F1, 0x00).expect("disable sram");
    assert!(matches!(
        cartridge.read8(0x0020_0000),
        Err(CartridgeError::UnmappedRom { .. })
    ));
}

#[test]
fn sram_write_protect_and_import_length_are_enforced() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    cartridge
        .write8(0x00A1_30F1, 0x03)
        .expect("enable and protect");
    assert!(cartridge.sram_write_protected());
    assert!(matches!(
        cartridge.write8(0x0020_0000, 0x5A),
        Err(CartridgeError::WriteProtected { .. })
    ));
    // The protected write did not mutate the byte.
    cartridge.write8(0x00A1_30F1, 0x01).expect("unprotect");
    assert_eq!(cartridge.read8(0x0020_0000).expect("sram byte"), 0x00);
    assert!(matches!(
        cartridge.import_sram(&[0_u8; 1024]),
        Err(CartridgeError::BadSramLength { length: 1024 })
    ));
    cartridge.import_sram(&[7_u8; 65_536]).expect("import");
    assert_eq!(cartridge.sram()[0], 7);
}

#[test]
fn protected_sram_word_and_long_writes_mutate_nothing() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    cartridge
        .write8(0x00A1_30F1, 0x03)
        .expect("enable and protect");
    assert!(matches!(
        cartridge.write16(0x0020_0000, 0xABCD),
        Err(CartridgeError::WriteProtected { .. })
    ));
    assert!(matches!(
        cartridge.write32(0x0020_0000, 0xABCD_EF01),
        Err(CartridgeError::WriteProtected { .. })
    ));
    cartridge.write8(0x00A1_30F1, 0x01).expect("unprotect");
    assert_eq!(
        &cartridge.sram()[..4],
        &[0, 0, 0, 0],
        "rejected writes must not partially mutate SRAM"
    );
}

// Task 4: TMSS lockout and the version register.

#[test]
fn tmss_locks_vdp_until_sega_written() {
    let mut tmss = Tmss::new(TmssModel::Enabled);
    assert!(tmss.locked());
    assert!(!tmss.vdp_writes_allowed());
    assert_eq!(tmss.model(), TmssModel::Enabled);
    tmss.write_long(0x00A1_4000, 0x5345_4741); // "SEGA"
    assert!(!tmss.locked());
    assert!(tmss.vdp_writes_allowed());
    // A write that does not set bit 9 leaves the latch alone.
    let mut locked = Tmss::new(TmssModel::Enabled);
    locked.write_long(0x00A1_4000, 0x0000_0000);
    assert!(locked.locked());
    locked.write_long(0x00A1_4000, 0x0000_0200);
    assert!(!locked.locked());
}

#[test]
fn tmss_disabled_starts_unlocked_and_ignores_other_addresses() {
    let mut tmss = Tmss::new(TmssModel::Disabled);
    assert!(!tmss.locked());
    assert!(tmss.vdp_writes_allowed());
    tmss.write_long(0x00A1_4004, 0x0000_0200);
    assert!(!tmss.locked());
}

#[test]
fn only_a_long_tmss_write_unlocks_the_lockout() {
    let mut cartridge = Cartridge::parse(header_rom("JUE", 0x3FFF)).expect("parse");
    cartridge.set_tmss_model(TmssModel::Enabled);
    assert!(cartridge.tmss().locked());

    // A byte whose value would place bit 9 in a word, and a word whose value has
    // bit 9 set, are acknowledged but cannot unlock.
    cartridge
        .write8(0x00A1_4001, 0x02)
        .expect("byte write acknowledged");
    assert!(cartridge.tmss().locked());
    cartridge
        .write16(0x00A1_4002, 0x0200)
        .expect("word write acknowledged");
    assert!(cartridge.tmss().locked());

    cartridge
        .write32(0x00A1_4000, 0x5345_4741)
        .expect("SEGA long write");
    assert!(!cartridge.tmss().locked());
}

#[test]
fn version_register_reports_export_pal_and_tmss() {
    let cartridge = Cartridge::parse(header_rom("JUE", 0x3FFF)).expect("parse");
    assert_eq!(cartridge.version_register(VideoStandard::Ntsc), 0xA1);
    let japan = Cartridge::parse(header_rom("J", 0x3FFF)).expect("parse");
    assert_eq!(japan.version_register(VideoStandard::Pal), 0x61);
    let usa = Cartridge::parse(header_rom("U", 0x3FFF)).expect("parse");
    assert_eq!(usa.version_register(VideoStandard::Ntsc), 0xA1);
}

#[test]
fn cartridge_view_decodes_header_mapper_sram_and_tmss() {
    let mut cartridge = Cartridge::parse(banked_rom()).expect("parse");
    cartridge.write8(0x00A1_30F1, 0x01).expect("enable sram");
    cartridge.write8(0x00A1_30F3, 2).expect("select bank");
    cartridge.set_tmss_model(TmssModel::Enabled);
    let view = CartridgeView::new(&cartridge);
    assert_eq!(view.system(), CartridgeSystem::MegaDrive);
    assert_eq!(view.region_bits(), 0b0111);
    assert_eq!(view.region_text(), "JUE");
    assert_eq!(view.domestic_name(), "CHRONOFORK TEST");
    assert_eq!(view.overseas_name(), "CHRONOFORK TEST");
    assert_eq!(view.serial(), "GM 00000000-00");
    assert_eq!(view.checksum_stored(), view.checksum_computed());
    assert_eq!(view.rom_start(), 0);
    assert_eq!(view.rom_end(), 0x100000);
    assert!(!view.sram_declared());
    assert_eq!(view.banks()[0], 2);
    assert!(view.sram_visible());
    assert!(!view.sram_write_protected());
    assert_eq!(view.tmss_model(), TmssModel::Enabled);
    assert!(view.tmss_locked());
}
