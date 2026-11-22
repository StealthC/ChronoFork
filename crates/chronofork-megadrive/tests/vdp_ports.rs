#![allow(clippy::expect_used)]

//! CRAM/VSRAM word addressing and control-port command decoding.
//!
//! The VDP data port addresses VRAM, CRAM and VSRAM through the command code
//! latched on the control port. CRAM has 64 words (the 7-bit CRAM address's
//! bit 6 is not decoded), VSRAM has 40 words behind a 9-bit address that wraps
//! modulo its length, and one transferred word advances the auto-increment
//! register exactly once — byte accesses included.

use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, CRAM_WORDS, DATA_PORT_OFFSET, VSRAM_WORDS, Vdp, VdpError, VramTarget,
};

// The latched command code is `CD5..CD0`: `CD1:CD0` come from the first control
// word (its bits 15..14) and `CD5:CD2` from the second (its bits 7..4). The low
// nibble `CD3..CD0` selects target *and* direction on hardware:
//
//   0b0000 VRAM read    0b0001 VRAM write   0b0011 CRAM write
//   0b0100 VSRAM read   0b0101 VSRAM write  0b1000 CRAM read
//   0b1100 8-bit VRAM read (not modelled)

/// Low command code bits `CD1:CD0 = 0b11` for the CRAM write code `0b0011`.
const CODE_1_1: u16 = 0x3;
/// Low command code bits `CD1:CD0 = 0b01` for the VSRAM write code `0b0101`.
const CODE_0_1: u16 = 0x1;
/// Second-word nibble `CD5:CD2 = 0b0001`, i.e. `CD2` set: shared by the VSRAM
/// read (`0b0100`) and write (`0b0101`) codes.
const NIBBLE_CD2: u16 = 0x1;
/// Second-word nibble `CD5:CD2 = 0b0010`, i.e. `CD3` set: the CRAM read code.
const NIBBLE_CD3: u16 = 0x2;
/// Second-word nibble `CD5:CD2 = 0b0011`: the `0b1100` 8-bit VRAM read code,
/// which this slice does not model yet (a capability gap, not a discard).
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
fn an_undecoded_eight_bit_vram_read_command_is_a_typed_error() {
    // `0b1100` is the documented 8-bit VRAM read (the pinned oracle's 0x0C).
    // This slice does not model that capability yet, so a *read* stays a typed
    // error; it is a capability gap, not a discarded command. A *write* with the
    // same code, on the other hand, is exactly the oracle's write `default:`
    // (its write decode looks at the low four bits and `0b1100` matches none of
    // `0b0001`/`0b0011`/`0b0101`), so it is discarded and counted.
    let mut vdp = Vdp::new();
    set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    // W1X: a documented operation this slice does not implement is its own
    // variant, so a capability gap can never be confused with an invalid target.
    assert_eq!(
        vdp.read16(DATA_PORT_OFFSET),
        Err(VdpError::UnsupportedBitRead { code: 0x0C })
    );
    assert_eq!(
        vdp.read8(DATA_PORT_OFFSET),
        Err(VdpError::UnsupportedBitRead { code: 0x0C })
    );
    assert_eq!(
        vdp.discarded_data_port_accesses(),
        0,
        "a capability gap is not a discard"
    );

    vdp.write_word(VramTarget::Cram, 0, 0x1234)
        .expect("seed cram");
    set_command(&mut vdp, 0, 0, NIBBLE_CD3_CD2).expect("8-bit read command");
    assert_eq!(vdp.write16(DATA_PORT_OFFSET, 0x4242), Ok(()));
    assert_eq!(
        vdp.discarded_data_port_accesses(),
        1,
        "the write is discarded"
    );
    assert_eq!(vdp.cram_entry(0), Some(0x1234), "no CRAM mutation");
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
