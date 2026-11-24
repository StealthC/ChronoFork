#![allow(clippy::expect_used)]

//! CRAM/VSRAM word addressing and control-port command decoding.
//!
//! The VDP data port addresses VRAM, CRAM and VSRAM through the command code
//! latched on the control port. CRAM has 64 words (the 7-bit CRAM address's
//! bit 6 is not decoded), VSRAM has 40 words behind a 9-bit address that wraps
//! modulo its length, and one transferred word advances the auto-increment
//! register exactly once — byte accesses included.

use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, CRAM_WORDS, DATA_PORT_OFFSET, SNAPSHOT_LEN, VSRAM_WORDS, Vdp, VdpError,
    VramTarget,
};

// The latched command code is `CD5..CD0`: `CD1:CD0` come from the first control
// word (its bits 15..14) and `CD5:CD2` from the second (its bits 7..4). The low
// nibble `CD3..CD0` selects target *and* direction on hardware:
//
//   0b0000 VRAM read    0b0001 VRAM write   0b0011 CRAM write
//   0b0100 VSRAM read   0b0101 VSRAM write  0b1000 CRAM read
//   0b1100 8-bit VRAM read (one VRAM byte into the low half of the read buffer)

/// Low command code bits `CD1:CD0 = 0b11` for the CRAM write code `0b0011`.
const CODE_1_1: u16 = 0x3;
/// Low command code bits `CD1:CD0 = 0b01` for the VSRAM write code `0b0101`.
const CODE_0_1: u16 = 0x1;
/// Second-word nibble `CD5:CD2 = 0b0001`, i.e. `CD2` set: shared by the VSRAM
/// read (`0b0100`) and write (`0b0101`) codes.
const NIBBLE_CD2: u16 = 0x1;
/// Second-word nibble `CD5:CD2 = 0b0010`, i.e. `CD3` set: the CRAM read code.
const NIBBLE_CD3: u16 = 0x2;
/// Second-word nibble `CD5:CD2 = 0b0011`: the `0b1100` 8-bit VRAM read code.
/// It is implemented: one VRAM byte at the bit-inverted address replaces the
/// low half of the read buffer and the upper half is retained.
const NIBBLE_CD3_CD2: u16 = 0x3;

fn set_command(
    vdp: &mut Vdp,
    address: u16,
    low_code: u16,
    high_nibble: u16,
) -> Result<(), VdpError> {
    vdp.write16(CONTROL_PORT_OFFSET, (low_code << 14) | (address & 0x3FFF))?;
    vdp.write16(
        CONTROL_PORT_OFFSET,
        ((address >> 14) & 0x0003) | (high_nibble << 4),
    )
}

fn set_auto_increment(vdp: &mut Vdp, value: u8) -> Result<(), VdpError> {
    vdp.write16(CONTROL_PORT_OFFSET, 0x8F00 | u16::from(value))
}

fn set_cram_write(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
    set_command(vdp, address, CODE_1_1, 0)
}

fn set_cram_read(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
    set_command(vdp, address, 0, NIBBLE_CD3)
}

fn set_vsram_write(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
    // `0b0101`: CD1:CD0 = 0b01 with CD2 set. This is the boot fixtures' macro
    // (`0x4000` / `0x0010`); the earlier helper built `0b0100`, the read code.
    set_command(vdp, address, CODE_0_1, NIBBLE_CD2)
}

fn set_vsram_read(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
    // `0b0100`: CD1:CD0 = 0b00 with CD2 set.
    set_command(vdp, address, 0, NIBBLE_CD2)
}

#[test]
fn cram_and_vsram_are_word_addressable_with_wraparound() {
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::Cram, 0, 0x0EEE).expect("cram");
    assert_eq!(vdp.cram_entry(0), Some(0x0EEE));
    vdp.write_word(VramTarget::Vsram, 39, 0x7FF).expect("vsram");
    assert_eq!(vdp.vsram_word(39), Some(0x7FF));
    // CRAM has 64 entries; address 64 wraps to 0.
    vdp.write_word(VramTarget::Cram, 64, 0x0001).expect("wrap");
    assert_eq!(vdp.cram_entry(0), Some(0x0001));
}

#[test]
fn cram_addresses_decode_only_the_low_six_bits() {
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::Cram, 0, 0x0010)
        .expect("entry 0");
    vdp.write_word(VramTarget::Cram, 64, 0x0040)
        .expect("address 64 wraps");
    vdp.write_word(VramTarget::Cram, 65, 0x0041)
        .expect("address 65 wraps");
    vdp.write_word(VramTarget::Cram, 0x7F, 0x007F)
        .expect("address 0x7F wraps");

    assert_eq!(vdp.cram_entry(0), Some(0x0040));
    assert_eq!(vdp.cram_entry(1), Some(0x0041));
    assert_eq!(vdp.cram_entry(CRAM_WORDS - 1), Some(0x007F));
    assert_eq!(vdp.cram_entry(CRAM_WORDS), None);
    assert_eq!(vdp.cram().len(), CRAM_WORDS);
}

#[test]
fn vsram_addresses_use_nine_bits_and_wrap_modulo_forty() {
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::Vsram, 0, 0x0001)
        .expect("entry 0");
    vdp.write_word(VramTarget::Vsram, 40, 0x0028)
        .expect("address 40 wraps");
    vdp.write_word(VramTarget::Vsram, 41, 0x0029)
        .expect("address 41 wraps");
    vdp.write_word(VramTarget::Vsram, 0x1FF, 0x01FF)
        .expect("nine-bit address 0x1FF");
    vdp.write_word(VramTarget::Vsram, 0x200, 0x0200)
        .expect("bit 9 is masked");

    // 40 % 40 = 0; 41 % 40 = 1; 0x1FF = 511, 511 % 40 = 31; 0x200 & 0x1FF = 0.
    assert_eq!(vdp.vsram_word(0), Some(0x0200));
    assert_eq!(vdp.vsram_word(1), Some(0x0029));
    assert_eq!(vdp.vsram_word(31), Some(0x01FF));
    assert_eq!(vdp.vsram_word(VSRAM_WORDS), None);
    assert_eq!(vdp.vsram().len(), VSRAM_WORDS);
}

#[test]
fn control_port_commands_decode_into_cram_and_vsram_writes() {
    let mut vdp = Vdp::new();
    set_auto_increment(&mut vdp, 2).expect("auto-increment 2");

    set_cram_write(&mut vdp, 0).expect("cram write command");
    vdp.write16(DATA_PORT_OFFSET, 0x1111).expect("cram word 0");
    vdp.write16(DATA_PORT_OFFSET, 0x2222).expect("cram word 1");
    assert_eq!(vdp.cram_entry(0), Some(0x1111));
    assert_eq!(vdp.cram_entry(1), Some(0x2222));
    assert_eq!(vdp.address(), 4, "one increment per transferred word");

    // CRAM/VSRAM command addresses are byte offsets; two bytes per entry.
    set_vsram_write(&mut vdp, 76).expect("vsram write command");
    vdp.write16(DATA_PORT_OFFSET, 0x3333)
        .expect("vsram word 38");
    vdp.write16(DATA_PORT_OFFSET, 0x4444)
        .expect("vsram word 39");
    assert_eq!(vdp.vsram_word(38), Some(0x3333));
    assert_eq!(vdp.vsram_word(39), Some(0x4444));
    assert_eq!(vdp.address(), 80);
}

#[test]
fn byte_writes_duplicate_into_both_halves_and_increment_once() {
    let mut vdp = Vdp::new();
    set_auto_increment(&mut vdp, 2).expect("auto-increment 2");

    set_cram_write(&mut vdp, 0).expect("cram write command");
    vdp.write8(DATA_PORT_OFFSET, 0x5A).expect("cram byte");
    assert_eq!(vdp.cram_entry(0), Some(0x5A5A));
    assert_eq!(vdp.address(), 2, "a byte write transfers one word");

    set_vsram_write(&mut vdp, 78).expect("vsram write command");
    vdp.write8(DATA_PORT_OFFSET, 0xA5).expect("vsram byte");
    assert_eq!(vdp.vsram_word(39), Some(0xA5A5));
    assert_eq!(vdp.address(), 80);
}

#[test]
fn cram_reads_use_the_data_port_and_select_the_requested_byte_lane() {
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::Cram, 0, 0x1234)
        .expect("cram entry");
    vdp.write_word(VramTarget::Cram, 1, 0x5678)
        .expect("cram entry");
    set_auto_increment(&mut vdp, 2).expect("auto-increment 2");

    set_cram_read(&mut vdp, 0).expect("cram read command");
    assert_eq!(
        vdp.read16(DATA_PORT_OFFSET).expect("cram word read"),
        0x1234
    );
    assert_eq!(vdp.address(), 2);

    set_cram_read(&mut vdp, 0).expect("cram read command");
    assert_eq!(vdp.read8(DATA_PORT_OFFSET).expect("high byte"), 0x12);
    assert_eq!(
        vdp.address(),
        2,
        "a byte read transfers one word and increments once"
    );
    // The odd offset selects the low lane of the word at the advanced address.
    assert_eq!(vdp.read8(DATA_PORT_OFFSET + 1).expect("low byte"), 0x78);
    assert_eq!(vdp.address(), 4);
}

#[test]
fn vsram_reads_use_the_data_port_with_the_same_word_address_and_wrap() {
    // `0b0100` is the VSRAM read code; the address decode and the 9-bit
    // modulo-40 wrap are shared with the write path (`address >> 1`).
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::Vsram, 0, 0x1234)
        .expect("vsram 0");
    vdp.write_word(VramTarget::Vsram, 39, 0xABCD)
        .expect("vsram 39");
    set_vsram_read(&mut vdp, 0).expect("vsram read command");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x1234));
    // Byte address 78 selects word 39 (78 >> 1), and the 9-bit wrap means an
    // address of 80 reads word 0 again.
    set_vsram_read(&mut vdp, 78).expect("vsram read command 39");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0xABCD));
    set_vsram_read(&mut vdp, 80).expect("vsram read command wrapped");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x1234));
}

#[test]
fn the_eight_bit_vram_read_substitutes_only_the_low_byte() {
    // `0b1100` is the documented 8-bit VRAM read (the pinned oracle's 0x0C, W1Z).
    // It reads one VRAM byte at the bit-inverted command address and writes it
    // into the *low* byte of the read buffer; the high byte keeps whatever the
    // buffer already held. A *write* with the same code is the oracle's write
    // `default:` (its write decode looks at the low four bits and `0b1100`
    // matches none of `0b0001`/`0b0011`/`0b0101`), so it is still discarded and
    // counted.
    // The seed writes use the direct `write_word` seam (the data port would leave
    // four or more FIFO entries behind, which the next reads would retire as
    // seeds). Even address 0x0000 -> inverted byte address 0x0001 -> the low byte
    // 0x34 of the word 0x1234; the high byte is the fresh buffer's 0x00.
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::Vram, 0x0000, 0x1234)
        .expect("seed vram 0x0000");
    vdp.write_word(VramTarget::Vram, 0x0002, 0xABCD)
        .expect("seed vram 0x0002");
    vdp.write_word(VramTarget::Vram, 0x0004, 0xEF01)
        .expect("seed vram 0x0004");
    set_auto_increment(&mut vdp, 0).expect("auto-increment 0");
    set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x0034));

    // The LSB of the command address is inverted, so an odd address reads the
    // other byte of the same word: 0x0001 inverts to 0x0000 -> 0x12 replaces the
    // low byte while the upper byte keeps the previous read's 0x00.
    set_command(&mut vdp, 1, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x0012));
    // LSB inversion is exact on later words too: 0x0002 -> 0x0003 -> 0xCD,
    // 0x0004 -> 0x0005 -> 0x01.
    set_command(&mut vdp, 2, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x00CD));
    set_command(&mut vdp, 4, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x0001));

    // A fresh device whose buffer starts at 0x0000 shows each byte lane
    // unambiguously: an even port offset is the high byte, an odd one the low.
    let mut byte_lanes = Vdp::new();
    byte_lanes
        .write_word(VramTarget::Vram, 0x0000, 0x5678)
        .expect("seed vram 0x0000");
    set_auto_increment(&mut byte_lanes, 0).expect("auto-increment 0");
    set_command(&mut byte_lanes, 0, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(byte_lanes.read8(DATA_PORT_OFFSET), Ok(0x00));
    assert_eq!(byte_lanes.read8(DATA_PORT_OFFSET + 1), Ok(0x78));

    // t4 vector 11 — a pending data-port write must not become the result. The
    // cache step only runs while the write FIFO is empty (`General.cpp:1748`,
    // `:1850-1856`), so a read with a pending write returns the buffer's current
    // value (the fresh 0x0000 here), consumes nothing from the FIFO and never
    // substitutes the written word for the buffer.
    let mut pending = Vdp::new();
    pending
        .write_word(VramTarget::Vram, 0x0002, 0xABCD)
        .expect("seed vram 0x0002");
    set_auto_increment(&mut pending, 0).expect("auto-increment 0");
    // The write targets command address 0; VRAM 0x0002 keeps 0xABCD.
    set_command(&mut pending, 0, CODE_0_1, 0).expect("vram write command");
    assert_eq!(pending.write16(DATA_PORT_OFFSET, 0xAB00), Ok(()));
    assert_eq!(pending.fifo_len(), 1, "the write is pending");
    set_command(&mut pending, 2, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(
        pending.read16(DATA_PORT_OFFSET),
        Ok(0x0000),
        "the pending write does not become the result"
    );
    assert_eq!(
        pending.fifo_len(),
        1,
        "a read neither consumes nor drains the FIFO entry"
    );
    assert_eq!(
        pending.discarded_data_port_accesses(),
        0,
        "the write is a VRAM write, not a discard"
    );

    // The missing vector: once the owner advances and the window drains, the same
    // command refreshes the buffer — seed 0 (empty window) with the low byte from
    // VRAM[address ^ 1].
    pending.drain_fifo();
    assert_eq!(pending.fifo_len(), 0);
    set_command(&mut pending, 2, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(pending.read16(DATA_PORT_OFFSET), Ok(0x00CD));

    // A control-port write invalidates any cached read value.
    vdp.write_word(VramTarget::Cram, 0, 0x1234)
        .expect("seed cram");
    set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(vdp.write16(DATA_PORT_OFFSET, 0x4242), Ok(()));
    assert_eq!(vdp.cram_entry(0), Some(0x1234), "no CRAM mutation");
}

#[test]
fn the_eight_bit_vram_read_follows_the_auto_increment_register() {
    // One completed data-port operation advances the command address by register
    // 15 exactly once, whatever the target (`General.cpp:2473-2477`).
    fn device(increment: u8) -> Vdp {
        let mut vdp = Vdp::new();
        for (address, word) in [(0x0000u16, 0x0034u16), (0x0002, 0x00CD), (0x0004, 0x0001)] {
            vdp.write_word(VramTarget::Vram, address, word)
                .expect("seed vram word");
        }
        set_auto_increment(&mut vdp, increment).expect("auto-increment register");
        set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
        vdp
    }

    // Increment 0: the address never moves, so every read returns the same byte.
    let mut parked = device(0);
    assert_eq!(parked.read16(DATA_PORT_OFFSET), Ok(0x0034));
    assert_eq!(parked.read16(DATA_PORT_OFFSET), Ok(0x0034));

    // Words are seeded at 0x0000 = 0x0034, 0x0002 = 0x00CD, 0x0004 = 0x0001, so
    // VRAM bytes read 00/34, 00/CD, 00/01. Increment 1 visits 0,1,2: the low byte
    // 0x34, the odd neighbour's byte 0x00, then word 0x0002's low byte 0xCD.
    let mut by_one = device(1);
    assert_eq!(by_one.read16(DATA_PORT_OFFSET), Ok(0x0034));
    assert_eq!(by_one.read16(DATA_PORT_OFFSET), Ok(0x0000));
    assert_eq!(by_one.read16(DATA_PORT_OFFSET), Ok(0x00CD));

    // Increment 2 visits 0 then 2: 0x34 then word 0x0002's low byte 0xCD.
    let mut by_two = device(2);
    assert_eq!(by_two.read16(DATA_PORT_OFFSET), Ok(0x0034));
    assert_eq!(by_two.read16(DATA_PORT_OFFSET), Ok(0x00CD));
}

#[test]
fn a_snapshot_taken_between_the_command_and_the_read_restores_the_same_value() {
    // The read buffer and its flags are canonical VDP state: a checkpoint taken
    // after the command is set up must restore a device whose next data-port read
    // returns exactly what the original would have returned.
    let mut original = Vdp::new();
    original
        .write_word(VramTarget::Vram, 0x0000, 0xABCD)
        .expect("seed vram 0x0000");
    original
        .write_word(VramTarget::Vram, 0x0002, 0xEF01)
        .expect("seed vram 0x0002");
    set_auto_increment(&mut original, 2).expect("auto-increment 2");
    set_command(&mut original, 0, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    let checkpoint = original.snapshot_bytes();

    let mut restored = Vdp::new();
    restored.restore_bytes(&checkpoint).expect("restore");
    assert_eq!(
        restored.snapshot_bytes(),
        checkpoint,
        "the checkpoint round-trips byte for byte"
    );

    let first_original = original.read16(DATA_PORT_OFFSET).expect("original read");
    let first_restored = restored.read16(DATA_PORT_OFFSET).expect("restored read");
    assert_eq!(first_original, 0x00CD);
    assert_eq!(
        first_restored, first_original,
        "the restored machine returns the same first read"
    );
    // A second read on both machines is checked for agreement after the restore.
    // It is not a seed discriminator: with an empty window the seed is zero, so
    // both reads return the low byte with a zero upper byte. The seed itself is
    // pinned by `the_completed_word_read_seed_is_observable_in_canonical_state`.
    assert_eq!(
        restored
            .read16(DATA_PORT_OFFSET)
            .expect("restored second read"),
        original
            .read16(DATA_PORT_OFFSET)
            .expect("original second read"),
        "consecutive reads agree after a restore"
    );
}

#[test]
fn the_vram_copy_target_shares_the_vram_address_space() {
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::VramCopy, 0x20, 0xCAFE)
        .expect("vram copy write");
    assert_eq!(vdp.read_word(VramTarget::Vram, 0x20), Ok(0xCAFE));
    assert_eq!(vdp.read_word(VramTarget::VramCopy, 0x20), Ok(0xCAFE));
    assert_eq!(&vdp.vram()[0x20..0x22], &[0xCA, 0xFE]);
}

#[test]
fn word_helpers_do_not_disturb_the_port_address() {
    let mut vdp = Vdp::new();
    set_command(&mut vdp, 0x0100, 0, 0).expect("vram read command");
    vdp.write_word(VramTarget::Vram, 0x0200, 0xBEEF)
        .expect("direct vram word");
    assert_eq!(vdp.address(), 0x0100, "the helper uses its own address");
    assert_eq!(vdp.read_word(VramTarget::Vram, 0x0200), Ok(0xBEEF));
}

// ---------------------------------------------------------------------------
// Control-write invalidation of the canonical read cache (T21-F1)
// ---------------------------------------------------------------------------

/// Offset of the flag byte in a serialized `CFVDP003` snapshot.
const SNAPSHOT_FLAGS_OFFSET: usize = 10;
/// The two read-cache flag bits (`read_data_available` bit 6, `read_data_half_cached`
/// bit 7).
const READ_CACHE_FLAGS: u8 = 0xC0;

fn flags_byte(snapshot: &[u8]) -> u8 {
    snapshot[SNAPSHOT_FLAGS_OFFSET]
}

/// The cached 16-bit read buffer, serialized little-endian at the snapshot tail.
fn cached_word(snapshot: &[u8]) -> u16 {
    let tail = &snapshot[SNAPSHOT_LEN - 2..];
    u16::from_le_bytes([tail[0], tail[1]])
}

/// A device whose canonical read buffer holds `buffer` and whose serialized flags
/// byte is `flags`, with VRAM `0x0000 = 0xABCD` as the next `0x0C` target.
fn primed_device(buffer: u16, flags: u8) -> Vdp {
    let mut source = Vdp::new();
    source
        .write_word(VramTarget::Vram, 0x0000, 0xABCD)
        .expect("seed vram 0");
    source
        .write_word(VramTarget::Vram, 0x0002, buffer)
        .expect("seed vram 2");
    set_auto_increment(&mut source, 0).expect("auto-increment 0");
    set_command(&mut source, 2, 0, 0).expect("0x00 read at 2");
    assert_eq!(
        source.read16(DATA_PORT_OFFSET),
        Ok(buffer),
        "the 0x00 read primes the buffer"
    );
    let mut bytes = source.snapshot_bytes();
    bytes[SNAPSHOT_FLAGS_OFFSET] = flags;
    let mut vdp = Vdp::new();
    vdp.restore_bytes(&bytes)
        .expect("restore the primed snapshot");
    assert_eq!(cached_word(&vdp.snapshot_bytes()), buffer);
    vdp
}

#[test]
fn a_control_write_clears_a_nonzero_cached_read_value() {
    // Each flag bit is defined in `CFVDP003`, so a snapshot can carry either one
    // alone. A new command must clear both and fetch the new target, never return
    // the stale 0xEF01 nor keep suppressing the seed through a stale 0x80
    // (`Ports.cpp:1613-1617`).
    for flags in [0x40_u8, 0x80] {
        let mut vdp = primed_device(0xEF01, flags);
        set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("new 0x0C command");
        let snapshot = vdp.snapshot_bytes();
        assert_eq!(
            flags_byte(&snapshot) & READ_CACHE_FLAGS,
            0,
            "flags 0x{flags:02X} must be cleared by the control write"
        );
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET),
            Ok(0x00CD),
            "the next read fetches VRAM[0 ^ 1] with a zeroed seed (flags 0x{flags:02X})"
        );
        assert_eq!(cached_word(&vdp.snapshot_bytes()), 0x00CD);
    }
}

#[test]
fn a_register_control_write_clears_the_cached_read_value() {
    let mut vdp = primed_device(0xEF01, 0x40);
    set_auto_increment(&mut vdp, 0).expect("register 15 write");
    let snapshot = vdp.snapshot_bytes();
    assert_eq!(
        flags_byte(&snapshot) & READ_CACHE_FLAGS,
        0,
        "a register write also invalidates the cached value"
    );
    assert_eq!(
        cached_word(&snapshot),
        0xEF01,
        "the buffer itself is not rewritten"
    );
    // The register write left the read-inhibit code latched, so a fresh command is
    // required before the data port serves a new target.
    set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("new 0x0C command");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x00CD));
}

#[test]
fn a_rejected_control_write_leaves_the_cached_value_and_flags_untouched() {
    let mut vdp = primed_device(0xEF01, 0x40);
    let before = vdp.snapshot_bytes();
    assert_eq!(
        vdp.write16(CONTROL_PORT_OFFSET, 0x9F00),
        Err(VdpError::UnsupportedRegister { register: 0x1F }),
        "register 0x1F is outside the implemented range"
    );
    assert_eq!(
        vdp.snapshot_bytes(),
        before,
        "a rejected control write mutates nothing, flags and buffer included"
    );
    assert_eq!(
        vdp.read16(DATA_PORT_OFFSET),
        Ok(0xEF01),
        "the cached value is still available after the rejection"
    );
}

// ---------------------------------------------------------------------------
// The seed is observable in canonical state (T21-F3)
// ---------------------------------------------------------------------------

#[test]
fn the_completed_word_read_seed_is_observable_in_canonical_state() {
    // The reconciled contract: a completed 0x00 read leaves its word in the
    // buffer and the port read clears the flags, so the next 0x0C read applies the
    // `!half_cached` seed — the zeroed next-read slot of an empty window — and
    // keeps only the low byte from `vram[address ^ 1]`. Deleting the seed
    // assignment leaves 0xEFCD here instead of 0x00CD.
    let mut vdp = Vdp::new();
    vdp.write_word(VramTarget::Vram, 0x0000, 0xABCD)
        .expect("seed vram 0");
    vdp.write_word(VramTarget::Vram, 0x0002, 0xEF01)
        .expect("seed vram 2");
    set_auto_increment(&mut vdp, 0).expect("auto-increment 0");
    set_command(&mut vdp, 2, 0, 0).expect("0x00 read at 2");
    assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0xEF01), "0x00 word read");
    let primed = vdp.snapshot_bytes();
    assert_eq!(cached_word(&primed), 0xEF01, "serialized buffer after 0x00");
    assert_eq!(
        flags_byte(&primed) & READ_CACHE_FLAGS,
        0,
        "the data-port read cleared both flags"
    );

    set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("0x0C read at 0");
    assert_eq!(
        vdp.read16(DATA_PORT_OFFSET),
        Ok(0x00CD),
        "seed 0 then low byte 0xCD (the pre-repair vector expected 0xEFCD)"
    );
    let after = vdp.snapshot_bytes();
    assert_eq!(cached_word(&after), 0x00CD, "serialized buffer after 0x0C");
    assert_eq!(flags_byte(&after) & READ_CACHE_FLAGS, 0);
}
