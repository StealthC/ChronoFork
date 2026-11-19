#![allow(clippy::expect_used)]

//! VDP device snapshot v2 (`CFVDP002`) coverage.
//!
//! Version 2 replaces the v1 leaky abstraction with the full device state:
//! CRAM, VSRAM, the write FIFO, DMA state, the frame index and the line/dot
//! position, alongside the registers, VRAM and the existing command latches.
//!
//! The crafted payload below is an independent encoder for the documented
//! layout. It must agree with [`Vdp::snapshot_bytes`] byte for byte, so a field
//! written or read at the wrong offset fails instead of silently round-tripping
//! through the implementation's own reader.

use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, CRAM_WORDS, DATA_PORT_OFFSET, DmaMode, FIFO_CAPACITY, REGISTER_COUNT,
    SNAPSHOT_LEN, SNAPSHOT_MAGIC, SNAPSHOT_VERSION, VRAM_SIZE, VSRAM_WORDS, Vdp, VdpError,
};

/// Exact length of a `CFVDP001` v1 snapshot: header (38 bytes) + 64 KiB VRAM.
const V1_SNAPSHOT_LEN: usize = 8 + 2 + 1 + 1 + 2 + 0x18 + VRAM_SIZE;
/// Magic of the rejected v1 snapshot format.
const V1_SNAPSHOT_MAGIC: &[u8; 8] = b"CFVDP001";

// v2 layout offsets. Keep in sync with `Vdp::snapshot_bytes` documentation;
// this is the independent decoder that pins the format.
const OFFSET_FLAGS: usize = 10;
const OFFSET_CODE: usize = 11;
const OFFSET_ADDRESS: usize = 12;
const OFFSET_REGISTERS: usize = 14;
const OFFSET_CRAM: usize = OFFSET_REGISTERS + REGISTER_COUNT;
const OFFSET_VSRAM: usize = OFFSET_CRAM + CRAM_WORDS * 2;
const OFFSET_FIFO_COUNT: usize = OFFSET_VSRAM + VSRAM_WORDS * 2;
const OFFSET_FIFO_WORDS: usize = OFFSET_FIFO_COUNT + 1;
const OFFSET_DMA_ACTIVE: usize = OFFSET_FIFO_WORDS + FIFO_CAPACITY * 2;
const OFFSET_DMA_SOURCE: usize = OFFSET_DMA_ACTIVE + 1;
const OFFSET_DMA_LENGTH: usize = OFFSET_DMA_SOURCE + 4;
const OFFSET_DMA_MODE: usize = OFFSET_DMA_LENGTH + 2;
const OFFSET_FRAME_INDEX: usize = OFFSET_DMA_MODE + 1;
const OFFSET_LINE: usize = OFFSET_FRAME_INDEX + 8;
const OFFSET_DOT: usize = OFFSET_LINE + 2;
const OFFSET_VRAM: usize = OFFSET_DOT + 2;

// Flag bits: vblank, half-latched command, odd frame, sprite collision and
// sprite overflow.
const FLAG_VBLANK: u8 = 1 << 0;
const FLAG_COMMAND_PENDING: u8 = 1 << 1;
const FLAG_ODD_FRAME: u8 = 1 << 2;
const FLAG_SPRITE_COLLISION: u8 = 1 << 3;
const FLAG_SPRITE_OVERFLOW: u8 = 1 << 4;

fn write_word(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn crafted_state() -> (Vec<u8>, [u16; CRAM_WORDS], [u16; VSRAM_WORDS], Vec<u8>) {
    let mut registers = [0_u8; REGISTER_COUNT];
    for (index, register) in registers.iter_mut().enumerate() {
        *register = 0x80 | index as u8;
    }
    let mut cram = [0_u16; CRAM_WORDS];
    for (index, entry) in cram.iter_mut().enumerate() {
        *entry = 0x0100 + index as u16;
    }
    let mut vsram = [0_u16; VSRAM_WORDS];
    for (index, entry) in vsram.iter_mut().enumerate() {
        *entry = 0x0200 + index as u16 * 2;
    }
    let vram: Vec<u8> = (0..VRAM_SIZE).map(|index| (index & 0xFF) as u8).collect();

    let mut bytes = vec![0_u8; SNAPSHOT_LEN];
    bytes[..8].copy_from_slice(SNAPSHOT_MAGIC.as_slice());
    write_word(&mut bytes, 8, SNAPSHOT_VERSION);
    bytes[OFFSET_FLAGS] = FLAG_COMMAND_PENDING
        | FLAG_VBLANK
        | FLAG_ODD_FRAME
        | FLAG_SPRITE_COLLISION
        | FLAG_SPRITE_OVERFLOW;
    bytes[OFFSET_CODE] = 0x15;
    write_word(&mut bytes, OFFSET_ADDRESS, 0x1234);
    bytes[OFFSET_REGISTERS..OFFSET_REGISTERS + REGISTER_COUNT].copy_from_slice(&registers);
    for (index, entry) in cram.iter().enumerate() {
        write_word(&mut bytes, OFFSET_CRAM + index * 2, *entry);
    }
    for (index, entry) in vsram.iter().enumerate() {
        write_word(&mut bytes, OFFSET_VSRAM + index * 2, *entry);
    }
    bytes[OFFSET_FIFO_COUNT] = 3;
    for (index, word) in [0xAAAA_u16, 0xBBBB, 0xCCCC, 0x0000].iter().enumerate() {
        write_word(&mut bytes, OFFSET_FIFO_WORDS + index * 2, *word);
    }
    bytes[OFFSET_DMA_ACTIVE] = 1;
    bytes[OFFSET_DMA_SOURCE..OFFSET_DMA_SOURCE + 4].copy_from_slice(&0x1234_5678_u32.to_le_bytes());
    write_word(&mut bytes, OFFSET_DMA_LENGTH, 0x0042);
    bytes[OFFSET_DMA_MODE] = DmaMode::Fill.code();
    bytes[OFFSET_FRAME_INDEX..OFFSET_FRAME_INDEX + 8]
        .copy_from_slice(&0x0102_0304_0506_0708_u64.to_le_bytes());
    write_word(&mut bytes, OFFSET_LINE, 0x00E7);
    write_word(&mut bytes, OFFSET_DOT, 0x00AB);
    bytes[OFFSET_VRAM..].copy_from_slice(&vram);

    (bytes, cram, vsram, vram)
}

#[test]
fn snapshot_format_is_the_documented_v2() {
    assert_eq!(&SNAPSHOT_MAGIC, b"CFVDP002");
    assert_eq!(SNAPSHOT_VERSION, 2);
    assert_eq!(SNAPSHOT_LEN, 65_811);
    assert_eq!(OFFSET_VRAM + VRAM_SIZE, SNAPSHOT_LEN);
}

#[test]
fn vdp_snapshot_round_trips_new_state() {
    let mut vdp = Vdp::new();
    vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
        .expect("auto-increment 2");
    vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
        .expect("command first half");
    vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
        .expect("command second half");
    vdp.write16(DATA_PORT_OFFSET, 0x1234).expect("vram word");
    let snapshot = vdp.snapshot_bytes();
    let mut restored = Vdp::new();
    restored.restore_bytes(&snapshot).expect("restore");
    assert_eq!(restored.vram(), vdp.vram());
    assert_eq!(restored.registers(), vdp.registers());
}

#[test]
fn v2_round_trips_every_new_field() {
    let (crafted, cram, vsram, vram) = crafted_state();
    let mut vdp = Vdp::new();
    vdp.restore_bytes(&crafted).expect("restore crafted v2");

    assert_eq!(vdp.registers().len(), REGISTER_COUNT);
    for (index, register) in vdp.registers().iter().enumerate() {
        assert_eq!(*register, 0x80 | index as u8, "register {index}");
    }
    assert_eq!(vdp.address(), 0x1234);
    assert_eq!(vdp.cram(), &cram);
    assert_eq!(vdp.vsram(), &vsram);
    assert_eq!(vdp.fifo_len(), 3);
    assert!(vdp.dma().active());
    assert_eq!(vdp.dma().source(), 0x1234_5678);
    assert_eq!(vdp.dma().length(), 0x0042);
    assert_eq!(vdp.dma().mode(), DmaMode::Fill);
    assert_eq!(vdp.frame_index(), 0x0102_0304_0506_0708);
    assert_eq!(vdp.line(), 0x00E7);
    assert_eq!(vdp.dot(), 0x00AB);
    let latches = vdp.status_latches();
    assert!(latches.vblank);
    assert!(latches.odd_frame);
    assert!(latches.sprite_collision);
    assert!(latches.sprite_overflow);
    assert_eq!(vdp.vram(), vram.as_slice());

    // The same bytes re-encode exactly: every field is canonical and the FIFO
    // keeps its oldest-first order.
    assert_eq!(vdp.snapshot_bytes(), crafted);

    // Status bits are driven by the restored state, not by constants.
    let status = vdp.status();
    assert_ne!(status & 0x0008, 0, "vblank");
    assert_ne!(status & 0x0010, 0, "odd frame");
    assert_ne!(status & 0x0020, 0, "sprite collision");
    assert_ne!(status & 0x0040, 0, "sprite overflow");
    assert_ne!(status & 0x0002, 0, "DMA busy");
    assert_eq!(
        status & 0x0300,
        0,
        "a three-word FIFO is neither empty nor full"
    );
}

#[test]
fn v1_snapshots_are_rejected_with_typed_errors() {
    let mut vdp = Vdp::new();
    vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
        .expect("auto-increment 2");
    let before = vdp.snapshot_bytes();

    // A genuine v1 payload has the old fixed length, so the exact-length
    // contract rejects it before any field is interpreted.
    let old_length = vec![0_u8; V1_SNAPSHOT_LEN];
    assert_eq!(
        vdp.restore_bytes(&old_length),
        Err(VdpError::SnapshotLengthMismatch {
            expected: SNAPSHOT_LEN,
            found: V1_SNAPSHOT_LEN,
        })
    );

    // A v2-length payload that still declares the v1 magic is rejected as a
    // magic mismatch.
    let mut v1_magic = before.clone();
    v1_magic[..8].copy_from_slice(V1_SNAPSHOT_MAGIC.as_slice());
    assert_eq!(
        vdp.restore_bytes(&v1_magic),
        Err(VdpError::SnapshotMagicMismatch)
    );

    // A v2-length `CFVDP002` payload with version 1 is rejected by the version
    // check, proving the change is versioned rather than silently reinterpreted.
    let mut v1_version = before.clone();
    write_word(&mut v1_version, 8, 1);
    assert_eq!(
        vdp.restore_bytes(&v1_version),
        Err(VdpError::SnapshotVersionUnsupported { found: 1 })
    );

    assert_eq!(vdp.snapshot_bytes(), before, "rejections never mutate");
}

#[test]
fn invalid_v2_field_values_are_typed_errors() {
    let mut vdp = Vdp::new();
    let before = vdp.snapshot_bytes();

    let mut fifo_count = before.clone();
    fifo_count[OFFSET_FIFO_COUNT] = FIFO_CAPACITY as u8 + 1;
    assert_eq!(
        vdp.restore_bytes(&fifo_count),
        Err(VdpError::SnapshotFifoCountInvalid {
            count: FIFO_CAPACITY as u8 + 1,
        })
    );

    let mut dma_mode = before.clone();
    dma_mode[OFFSET_DMA_MODE] = 0xFF;
    assert_eq!(
        vdp.restore_bytes(&dma_mode),
        Err(VdpError::SnapshotDmaModeInvalid { mode: 0xFF })
    );

    let mut dma_active = before.clone();
    dma_active[OFFSET_DMA_ACTIVE] = 2;
    assert_eq!(
        vdp.restore_bytes(&dma_active),
        Err(VdpError::SnapshotBooleanInvalid {
            field: "DMA active",
            value: 2,
        })
    );

    let mut reserved_flags = before.clone();
    reserved_flags[OFFSET_FLAGS] = 0x80;
    assert_eq!(
        vdp.restore_bytes(&reserved_flags),
        Err(VdpError::SnapshotFlagsInvalid { flags: 0x80 })
    );

    let mut wide_code = before.clone();
    wide_code[OFFSET_CODE] = 0x40;
    assert_eq!(
        vdp.restore_bytes(&wide_code),
        Err(VdpError::SnapshotCodeInvalid { code: 0x40 })
    );

    assert_eq!(
        vdp.restore_bytes(&before[..SNAPSHOT_LEN - 1]),
        Err(VdpError::SnapshotLengthMismatch {
            expected: SNAPSHOT_LEN,
            found: SNAPSHOT_LEN - 1,
        })
    );

    assert_eq!(vdp.snapshot_bytes(), before, "rejections never mutate");
}
