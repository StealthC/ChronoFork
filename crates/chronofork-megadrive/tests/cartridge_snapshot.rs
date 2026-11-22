//! Payload v4, state-hash v4 and configuration-hash v4 coverage for the
//! modelled cartridge.
//!
//! The images are synthetic headers written by this test; no commercial ROM is
//! read or embedded.

#![allow(clippy::expect_used)]

use chronofork_backend_api::{
    BACKEND_SNAPSHOT_FORMAT_VERSION, Backend, BackendError, BackendSnapshot,
};
use chronofork_core::InputFrame;
use chronofork_megadrive::vdp::SNAPSHOT_LEN as VDP_SNAPSHOT_LEN;
use chronofork_megadrive::{
    CONFIG_MAGIC, MegadriveConfig, MegadriveFactory, PAYLOAD_MAGIC, PAYLOAD_VERSION, RAM_SIZE,
    SRAM_SIZE, STATE_MAGIC, TmssModel, VideoStandard,
};

const IMAGE_LEN: usize = 0x4000;

/// Writes the shared synthetic header over a NOP-filled image with reset
/// vectors at the top of work RAM and the program at `0x200`.
fn rom_with_header() -> Vec<u8> {
    let mut rom = vec![0_u8; IMAGE_LEN];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the test image holds a header"
    );
    rom
}

/// A 1 MiB image whose mapper bank 2 page starts with a unique byte.
fn banked_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x100001];
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the banked image holds a header"
    );
    rom[0x100000] = 0x42;
    rom
}

fn pal_rom() -> Vec<u8> {
    let mut rom = rom_with_header();
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "E"),
        "the PAL image holds a header"
    );
    rom
}

fn factory() -> MegadriveFactory {
    MegadriveFactory::new(rom_with_header()).expect("valid synthetic cartridge")
}

#[test]
fn payload_v4_and_its_magics_are_the_documented_versions() {
    assert_eq!(PAYLOAD_VERSION, 6);
    assert_eq!(&PAYLOAD_MAGIC, b"CFMD68K6");
    assert_eq!(STATE_MAGIC, b"CFMDSTATE6");
    assert_eq!(CONFIG_MAGIC, b"CFMDCONF6");
}

#[test]
fn snapshot_round_trips_mapper_sram_and_tmss() {
    let factory = MegadriveFactory::with_config(
        rom_with_header(),
        MegadriveConfig {
            video: Some(VideoStandard::Ntsc),
            tmss: TmssModel::Enabled,
        },
    )
    .expect("factory");
    let mut machine = factory.create_machine().expect("machine");
    machine.write_cartridge(0x00A1_30F1, 0x01).expect("sram on");
    machine
        .write_cartridge(0x0020_0000, 0x31)
        .expect("sram byte");
    machine
        .write_cartridge(0x00A1_30F3, 0x02)
        .expect("select bank 2");
    let snapshot = machine.snapshot().expect("snapshot");

    let mut restored = factory.create_machine().expect("machine");
    restored.restore(&snapshot).expect("restore");
    assert_eq!(restored.cartridge().sram()[0], 0x31);
    assert!(restored.cartridge().sram_visible());
    assert_eq!(restored.cartridge().banks()[0], 2);
    assert_eq!(restored.tmss().model(), TmssModel::Enabled);
    assert!(restored.tmss().locked());
    assert_eq!(
        restored.cartridge().sram_control(),
        machine.cartridge().sram_control()
    );
}

#[test]
fn old_payload_magic_and_version_are_rejected() {
    let mut machine = factory().create_machine().expect("machine");
    for _ in 0..3 {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    let snapshot = machine.snapshot().expect("snapshot");

    for old_magic in [b"CFMD68K1", b"CFMD68K2"] {
        let mut old_payload = snapshot.payload().to_vec();
        old_payload[..8].copy_from_slice(old_magic);
        let old = replace_payload(&snapshot, old_payload);
        assert!(
            machine.restore(&old).is_err(),
            "old payload magic must be rejected"
        );
    }

    let mut version_two = snapshot.payload().to_vec();
    version_two[8..10].copy_from_slice(&2_u16.to_le_bytes());
    let bad_version = replace_payload(&snapshot, version_two);
    assert!(
        machine.restore(&bad_version).is_err(),
        "payload version 2 must be rejected"
    );

    let truncated = snapshot.payload()[..snapshot.payload().len() - 1].to_vec();
    let short = replace_payload(&snapshot, truncated);
    assert!(
        machine.restore(&short).is_err(),
        "a truncated payload must be rejected"
    );
}

#[test]
fn rejected_restore_leaves_the_machine_untouched() {
    let mut machine = factory().create_machine().expect("machine");
    for _ in 0..4 {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    machine.write_cartridge(0x00A1_30F1, 0x01).expect("sram on");
    machine
        .write_cartridge(0x0020_0000, 0x77)
        .expect("sram byte");
    let before_hash = machine.inspect().expect("inspect").state_hash;
    let before_sram = machine.cartridge().sram()[0];
    let snapshot = machine.snapshot().expect("snapshot");

    // Flip one RAM byte inside the payload without re-signing the state hash.
    let mut tampered = snapshot.payload().to_vec();
    let ram_offset = 10 + 32 + 32 + 4 + 2 + 1 + 8;
    tampered[ram_offset] ^= 0xFF;
    let tampered = replace_payload(&snapshot, tampered);
    assert!(machine.restore(&tampered).is_err());

    assert_eq!(
        machine.inspect().expect("inspect").state_hash,
        before_hash,
        "a rejected restore must not change canonical state"
    );
    assert_eq!(machine.cartridge().sram()[0], before_sram);
}

#[test]
fn state_hash_covers_sram_and_the_tmss_latch() {
    let plain = factory().create_machine().expect("machine");
    let mut with_sram = factory().create_machine().expect("machine");
    with_sram
        .write_cartridge(0x00A1_30F1, 0x01)
        .expect("sram on");
    with_sram
        .write_cartridge(0x0020_0000, 0x01)
        .expect("sram byte");
    assert_ne!(
        plain.inspect().expect("inspect").state_hash,
        with_sram.inspect().expect("inspect").state_hash,
        "SRAM bytes are canonical state"
    );

    let tmss_factory = MegadriveFactory::with_config(
        rom_with_header(),
        MegadriveConfig {
            video: None,
            tmss: TmssModel::Enabled,
        },
    )
    .expect("factory");
    let locked = tmss_factory.create_machine().expect("machine");
    assert!(locked.tmss().locked());
    assert_ne!(
        plain.inspect().expect("inspect").state_hash,
        locked.inspect().expect("inspect").state_hash,
        "the TMSS latch is canonical state"
    );
}

#[test]
fn configuration_hash_covers_rom_video_tmss_and_features_but_not_sram() {
    let base = factory();
    assert_eq!(base.config_hash(), factory().config_hash());

    let mut changed = rom_with_header();
    changed[0x210] = 0x00;
    let changed = MegadriveFactory::new(changed).expect("changed cartridge");
    assert_ne!(
        base.config_hash(),
        changed.config_hash(),
        "ROM identity is configuration"
    );

    let pal = MegadriveFactory::with_config(
        rom_with_header(),
        MegadriveConfig {
            video: Some(VideoStandard::Pal),
            tmss: TmssModel::Disabled,
        },
    )
    .expect("factory");
    assert_ne!(
        base.config_hash(),
        pal.config_hash(),
        "the video standard is configuration"
    );

    let tmss = MegadriveFactory::with_config(
        rom_with_header(),
        MegadriveConfig {
            video: None,
            tmss: TmssModel::Enabled,
        },
    )
    .expect("factory");
    assert_ne!(
        base.config_hash(),
        tmss.config_hash(),
        "the TMSS model is configuration"
    );

    // SRAM bytes and the TMSS latch are mutable state: writing them never
    // changes the factory configuration hash.
    let mut machine = tmss.create_machine().expect("machine");
    machine.write_cartridge(0x00A1_30F1, 0x01).expect("sram on");
    machine
        .write_cartridge(0x0020_0000, 0x5A)
        .expect("sram byte");
    assert_eq!(
        machine.snapshot().expect("snapshot").config_hash(),
        tmss.config_hash()
    );
}

#[test]
fn the_factory_derives_pal_only_for_an_e_only_header_and_accepts_overrides() {
    let derived_pal = MegadriveFactory::new(pal_rom()).expect("factory");
    assert_eq!(derived_pal.video_standard(), VideoStandard::Pal);
    assert_eq!(
        derived_pal.scheduler_config().standard(),
        VideoStandard::Pal
    );

    let derived_ntsc = factory();
    assert_eq!(derived_ntsc.video_standard(), VideoStandard::Ntsc);

    let override_ntsc = MegadriveFactory::with_config(
        pal_rom(),
        MegadriveConfig {
            video: Some(VideoStandard::Ntsc),
            tmss: TmssModel::Disabled,
        },
    )
    .expect("factory");
    assert_eq!(override_ntsc.video_standard(), VideoStandard::Ntsc);
    let machine = override_ntsc.create_machine().expect("machine");
    assert_eq!(machine.bus().peek8(0x00A1_0001).expect("version"), 0xA1);
}

#[test]
fn derived_pal_machine_reports_the_pal_version_register() {
    let factory = MegadriveFactory::new(pal_rom()).expect("factory");
    let machine = factory.create_machine().expect("machine");
    // E-only is an export PAL header: export bit and PAL bit set.
    assert_eq!(machine.bus().peek8(0x00A1_0001).expect("version"), 0xE1);
}

/// Re-wraps a mutated payload in the same snapshot envelope.
fn replace_payload(snapshot: &BackendSnapshot, payload: Vec<u8>) -> BackendSnapshot {
    BackendSnapshot::new(
        snapshot.backend_id().clone(),
        snapshot.backend_version(),
        BACKEND_SNAPSHOT_FORMAT_VERSION,
        snapshot.tick(),
        snapshot.config_hash(),
        snapshot.state_hash(),
        payload,
    )
}

#[test]
fn inspection_exposes_bounded_cartridge_pseudo_registers() {
    let machine = factory().create_machine().expect("machine");
    let inspection = machine.inspect().expect("inspect");
    let registers = &inspection.registers;
    assert_eq!(registers["cartridge.system"], 0);
    assert_eq!(registers["cartridge.region"], 0b0111);
    let cartridge = chronofork_megadrive::Cartridge::parse(rom_with_header()).expect("parse");
    let header = cartridge.header();
    assert_eq!(
        registers["cartridge.checksum_stored"],
        u64::from(header.checksum_stored)
    );
    assert_eq!(
        registers["cartridge.checksum_computed"],
        u64::from(header.checksum_computed)
    );
    assert!(registers.contains_key("cartridge.serial"));
    for window in 0..7 {
        assert_eq!(
            registers[&format!("mapper.bank{window}")],
            window as u64 + 1
        );
    }
    assert_eq!(registers["sram.visible"], 0);
    assert_eq!(registers["sram.protected"], 0);
    assert_eq!(registers["tmss.locked"], 0);
}

/// v4 extended CPU state: USP, SSP, 68000 cycles and the stopped tag.
const V4_EXTENDED_STATE_LEN: usize = 4 + 4 + 8 + 1;
/// Byte offset of the first mapper bank in a v4 payload state body.
const BANKS_OFFSET: usize = PAYLOAD_MAGIC.len()
    + 2
    + 32
    + 32
    + 4
    + 2
    + 1
    + 8
    + V4_EXTENDED_STATE_LEN
    + RAM_SIZE
    + VDP_SNAPSHOT_LEN;
/// Byte offset of the SRAM control byte in a v4 payload state body.
const SRAM_CONTROL_OFFSET: usize = BANKS_OFFSET + 7 * 2;
/// Byte offset of the canonical 64 KiB SRAM in a v4 payload state body.
const SRAM_OFFSET: usize = SRAM_CONTROL_OFFSET + 1;
/// Byte offset of the TMSS model tag.
const TMSS_MODEL_OFFSET: usize = SRAM_OFFSET + SRAM_SIZE;
/// Byte offset of the TMSS lock latch.
const TMSS_LOCK_OFFSET: usize = TMSS_MODEL_OFFSET + 1;

/// Re-wraps a mutated payload with a state hash recomputed over its new body.
///
/// This models a deliberately consistent but semantically invalid snapshot: the
/// envelope and state hash match, so only the cartridge validation can reject it.
fn rehashed(snapshot: &BackendSnapshot, payload: Vec<u8>) -> BackendSnapshot {
    let body = &payload[PAYLOAD_MAGIC.len() + 2..];
    let mut input = Vec::with_capacity(STATE_MAGIC.len() + body.len());
    input.extend_from_slice(STATE_MAGIC);
    input.extend_from_slice(body);
    let state_hash = chronofork_core::hash_bytes(&input);
    BackendSnapshot::new(
        snapshot.backend_id().clone(),
        snapshot.backend_version(),
        BACKEND_SNAPSHOT_FORMAT_VERSION,
        snapshot.tick(),
        snapshot.config_hash(),
        state_hash,
        payload,
    )
}

#[test]
fn mapper_write_survives_snapshot_and_reads_the_selected_bank() {
    let factory = MegadriveFactory::new(banked_rom()).expect("banked cartridge");
    let mut machine = factory.create_machine().expect("machine");
    machine
        .write_cartridge(0x00A1_30F3, 0x02)
        .expect("select bank 2");
    let snapshot = machine.snapshot().expect("snapshot");
    let mut restored = factory.create_machine().expect("machine");
    restored.restore(&snapshot).expect("restore");
    assert_eq!(restored.cartridge().banks()[0], 2);
    assert_eq!(
        restored.bus().peek8(0x0008_0000).expect("banked byte"),
        0x42,
        "the restored mapper must select the same bank page"
    );
}

#[test]
fn rehashed_payload_with_an_out_of_range_bank_is_rejected_without_mutation() {
    let factory = factory();
    let mut machine = factory.create_machine().expect("machine");
    machine.write_cartridge(0x00A1_30F1, 0x01).expect("sram on");
    machine
        .write_cartridge(0x0020_0000, 0x77)
        .expect("sram byte");
    for _ in 0..3 {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    let before = machine.inspect().expect("inspect");
    let before_pc = machine.cpu().pc();
    let snapshot = machine.snapshot().expect("snapshot");

    let mut payload = snapshot.payload().to_vec();
    payload[BANKS_OFFSET..BANKS_OFFSET + 2].copy_from_slice(&0xFFFF_u16.to_be_bytes());
    let tampered = rehashed(&snapshot, payload);
    let error = machine
        .restore(&tampered)
        .expect_err("bank must be rejected");
    assert!(
        matches!(error, BackendError::InvalidSnapshot(_)),
        "invalid bank must be a typed InvalidSnapshot, got {error:?}"
    );
    assert_eq!(
        machine.inspect().expect("inspect").state_hash,
        before.state_hash
    );
    assert_eq!(machine.cpu().pc(), before_pc);
    assert_eq!(machine.cartridge().sram()[0], 0x77);
}

#[test]
fn rehashed_payload_with_a_disabled_locked_tmss_pair_is_rejected() {
    let factory = factory();
    let mut machine = factory.create_machine().expect("machine");
    for _ in 0..2 {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    let before = machine.inspect().expect("inspect");
    let snapshot = machine.snapshot().expect("snapshot");

    let mut payload = snapshot.payload().to_vec();
    // Model 0 (Disabled) with a locked latch is impossible.
    payload[TMSS_MODEL_OFFSET] = 0;
    payload[TMSS_LOCK_OFFSET] = 1;
    let tampered = rehashed(&snapshot, payload);
    let error = machine
        .restore(&tampered)
        .expect_err("pair must be rejected");
    assert!(
        matches!(error, BackendError::InvalidSnapshot(_)),
        "an impossible TMSS pair must be a typed InvalidSnapshot, got {error:?}"
    );
    assert_eq!(
        machine.inspect().expect("inspect").state_hash,
        before.state_hash
    );
}

#[test]
fn rehashed_payload_with_a_different_tmss_model_is_rejected() {
    let factory = MegadriveFactory::with_config(
        rom_with_header(),
        MegadriveConfig {
            video: None,
            tmss: TmssModel::Enabled,
        },
    )
    .expect("factory");
    let mut machine = factory.create_machine().expect("machine");
    for _ in 0..2 {
        machine.step(InputFrame::NEUTRAL).expect("step");
    }
    let before = machine.inspect().expect("inspect");
    let before_pc = machine.cpu().pc();
    let snapshot = machine.snapshot().expect("snapshot");

    let mut payload = snapshot.payload().to_vec();
    payload[TMSS_MODEL_OFFSET] = 0; // Disabled, unlike the configured model.
    payload[TMSS_LOCK_OFFSET] = 0;
    let tampered = rehashed(&snapshot, payload);
    let error = machine
        .restore(&tampered)
        .expect_err("model must be rejected");
    assert!(
        matches!(error, BackendError::InvalidSnapshot(_)),
        "a different TMSS model must be a typed InvalidSnapshot, got {error:?}"
    );
    assert_eq!(
        machine.inspect().expect("inspect").state_hash,
        before.state_hash
    );
    assert_eq!(machine.cpu().pc(), before_pc);
    assert!(machine.tmss().locked());
}
