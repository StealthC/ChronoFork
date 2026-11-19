#![allow(clippy::expect_used)]

//! VDP DMA transfer coverage.
//!
//! Phase 8.3 performs DMA eagerly: `start_dma` validates the whole source and
//! destination range before mutating anything, performs the transfer, records
//! the slot cost and leaves DMA_BUSY set until the owner takes the cost. In this
//! slice the 68000-address source operand names a VRAM byte address (the VDP has
//! no external memory view), documented as a modelling limit.

use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, DATA_PORT_OFFSET, DmaMode, STATUS_DMA_BUSY, Vdp, VdpError,
    VdpInterruptSource, VramTarget,
};

fn write_register(vdp: &mut Vdp, register: u8, value: u8) -> Result<(), VdpError> {
    vdp.write16(
        CONTROL_PORT_OFFSET,
        0x8000 | (u16::from(register) << 8) | u16::from(value),
    )
}

fn write_vram(vdp: &mut Vdp, address: u16, words: &[u16]) -> Result<(), VdpError> {
    write_register(vdp, 0x0F, 2)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0x4000 | (address & 0x3FFF))?;
    vdp.write16(CONTROL_PORT_OFFSET, (address >> 14) & 0x0003)?;
    for word in words {
        vdp.write16(DATA_PORT_OFFSET, *word)?;
    }
    Ok(())
}

fn set_vram_write(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
    write_register(vdp, 0x0F, 2)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0x4000 | (address & 0x3FFF))?;
    vdp.write16(CONTROL_PORT_OFFSET, (address >> 14) & 0x0003)
}

fn set_cram_write(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
    write_register(vdp, 0x0F, 2)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0xC000 | (address & 0x3FFF))?;
    vdp.write16(CONTROL_PORT_OFFSET, (address >> 14) & 0x0003)
}

fn set_vsram_write(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
    write_register(vdp, 0x0F, 2)?;
    vdp.write16(CONTROL_PORT_OFFSET, address & 0x3FFF)?;
    vdp.write16(CONTROL_PORT_OFFSET, ((address >> 14) & 0x0003) | 0x0010)
}

fn vram_word(vdp: &Vdp, address: u16) -> u16 {
    u16::from_be_bytes([
        vdp.vram()[usize::from(address)],
        vdp.vram()[usize::from(address) + 1],
    ])
}

#[test]
fn memory_to_vram_transfers_words_in_order() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0x1111, 0x2222, 0x3333]).expect("source");
    set_vram_write(&mut vdp, 0x0200).expect("destination");

    assert!(!vdp.dma_active());
    vdp.start_dma(0x0100, 3, DmaMode::MemoryToVram)
        .expect("dma");
    assert_eq!(vram_word(&vdp, 0x0200), 0x1111);
    assert_eq!(vram_word(&vdp, 0x0202), 0x2222);
    assert_eq!(vram_word(&vdp, 0x0204), 0x3333);
    assert_eq!(vram_word(&vdp, 0x0100), 0x1111, "source is unchanged");

    assert!(
        vdp.dma_active(),
        "DMA_BUSY is visible until the cost is taken"
    );
    assert_eq!(vdp.status() & STATUS_DMA_BUSY, STATUS_DMA_BUSY);
    assert_eq!(vdp.dma_words_remaining(), 0, "eager transfer is complete");
    assert_eq!(vdp.take_dma_cycles(), 6, "three words at two master cycles");
    assert!(
        !vdp.dma_active(),
        "taking the cost completes the busy window"
    );
    assert_eq!(vdp.take_dma_cycles(), 0);
}

#[test]
fn memory_to_cram_and_vsram_write_their_entries() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0x0EEE, 0x0123]).expect("source");

    set_cram_write(&mut vdp, 0).expect("cram destination");
    vdp.start_dma(0x0100, 2, DmaMode::MemoryToCram)
        .expect("cram dma");
    assert_eq!(vdp.cram_entry(0), Some(0x0EEE));
    assert_eq!(vdp.cram_entry(1), Some(0x0123));
    let _ = vdp.take_dma_cycles();

    set_vsram_write(&mut vdp, 0).expect("vsram destination");
    vdp.start_dma(0x0100, 2, DmaMode::MemoryToVsram)
        .expect("vsram dma");
    assert_eq!(vdp.vsram_word(0), Some(0x0EEE));
    assert_eq!(vdp.vsram_word(1), Some(0x0123));
}

#[test]
fn fill_writes_the_documented_constant_word() {
    let mut vdp = Vdp::new();
    set_vram_write(&mut vdp, 0x0300).expect("destination");
    // The low byte of the source operand is the fill value, duplicated.
    vdp.start_dma(0x00AB, 4, DmaMode::Fill).expect("fill");
    for offset in (0..8).step_by(2) {
        assert_eq!(vram_word(&vdp, 0x0300 + offset), 0xABAB);
    }
    assert_eq!(vdp.take_dma_cycles(), 8);
}

#[test]
fn copy_transfers_vram_words_to_the_command_address() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0xCAFE, 0xBEEF]).expect("source");
    set_vram_write(&mut vdp, 0x0400).expect("destination");
    vdp.start_dma(0x0100, 2, DmaMode::Copy).expect("copy");
    assert_eq!(vram_word(&vdp, 0x0400), 0xCAFE);
    assert_eq!(vram_word(&vdp, 0x0402), 0xBEEF);
}

#[test]
fn out_of_range_transfers_are_rejected_before_mutation() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0x1111, 0x2222]).expect("source");
    set_vram_write(&mut vdp, 0xFFFE).expect("top of vram");
    let before = vdp.snapshot_bytes();

    assert_eq!(
        vdp.start_dma(0x0100, 2, DmaMode::MemoryToVram),
        Err(VdpError::DmaDestinationOutOfRange {
            target: VramTarget::Vram,
            word_index: 0x7FFF,
            words: 2,
            capacity: 0x8000,
        })
    );
    assert!(!vdp.dma_active());
    assert_eq!(vdp.snapshot_bytes(), before, "a rejected DMA never mutates");

    // Source outside the modelled 64 KiB VRAM view.
    set_vram_write(&mut vdp, 0x0200).expect("destination");
    assert_eq!(
        vdp.start_dma(0xFFFE, 2, DmaMode::MemoryToVram),
        Err(VdpError::DmaSourceOutOfRange {
            source: 0xFFFE,
            words: 2,
        })
    );

    // Overlapping source and destination are rejected.
    assert_eq!(
        vdp.start_dma(0x0200, 4, DmaMode::Copy),
        Err(VdpError::DmaSourceDestinationOverlap {
            source: 0x0200,
            destination: 0x0200,
            words: 4,
        })
    );

    // CRAM only has 64 words.
    set_cram_write(&mut vdp, 126).expect("last cram entry");
    let before_cram = vdp.snapshot_bytes();
    assert_eq!(
        vdp.start_dma(0x0100, 2, DmaMode::MemoryToCram),
        Err(VdpError::DmaDestinationOutOfRange {
            target: VramTarget::Cram,
            word_index: 63,
            words: 2,
            capacity: 64,
        })
    );
    assert_eq!(vdp.cram_entry(63), Some(0));
    assert_eq!(
        vdp.snapshot_bytes(),
        before_cram,
        "rejections leave state intact"
    );
}

/// Sets register `0x01` bit 4 (DMA enable).
fn enable_dma(vdp: &mut Vdp) {
    write_register(vdp, 0x01, 0x10).expect("dma enable");
}

/// Programs the hardware DMA registers: `0x13`/`0x14` length, `0x15`/`0x16`/
/// `0x17` source low/mid/high plus the mode bits.
fn program_dma(vdp: &mut Vdp, source: u32, length: u16, mode_bits: u8) {
    write_register(vdp, 0x13, (length & 0x00FF) as u8).expect("length low");
    write_register(vdp, 0x14, ((length >> 8) & 0x00FF) as u8).expect("length high");
    write_register(vdp, 0x15, (source & 0x00FF) as u8).expect("source low");
    write_register(vdp, 0x16, ((source >> 8) & 0x00FF) as u8).expect("source mid");
    write_register(vdp, 0x17, (((source >> 16) & 0x003F) as u8) | mode_bits)
        .expect("source high and mode");
}

/// Latches a DMA command code and then triggers it with a data-port access.
fn trigger_dma(vdp: &mut Vdp, code: u8, address: u16) {
    let first = (u16::from(code & 0x03) << 14) | (address & 0x3FFF);
    let second = (u16::from((code >> 2) & 0x0F) << 4) | ((address >> 14) & 0x0003);
    vdp.write16(CONTROL_PORT_OFFSET, first).expect("first word");
    vdp.write16(CONTROL_PORT_OFFSET, second)
        .expect("second word");
    vdp.write16(DATA_PORT_OFFSET, 0)
        .expect("data-port access triggers the transfer");
}

#[test]
fn dma_registers_program_a_triggered_transfer() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0xABCD, 0x1234]).expect("source");
    enable_dma(&mut vdp);
    program_dma(&mut vdp, 0x0100, 2, 0x00);

    // DMA command code 0x21: CD5 (DMA) plus the VRAM-write target, address 0.
    trigger_dma(&mut vdp, 0x21, 0x0000);

    assert_eq!(vram_word(&vdp, 0x0000), 0xABCD);
    assert_eq!(vram_word(&vdp, 0x0002), 0x1234);
    assert_eq!(vdp.take_dma_cycles(), 4);
}

#[test]
fn register_dma_takes_the_destination_from_the_command_code() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0x0EEE, 0x0123]).expect("source");
    enable_dma(&mut vdp);
    program_dma(&mut vdp, 0x0100, 2, 0x00);

    // CRAM write (command code 0x23).
    trigger_dma(&mut vdp, 0x23, 0x0000);
    assert_eq!(vdp.cram_entry(0), Some(0x0EEE));
    assert_eq!(vdp.cram_entry(1), Some(0x0123));
    let _ = vdp.take_dma_cycles();

    // VSRAM write (command code 0x24).
    trigger_dma(&mut vdp, 0x24, 0x0000);
    assert_eq!(vdp.vsram_word(0), Some(0x0EEE));
    assert_eq!(vdp.vsram_word(1), Some(0x0123));
    assert_eq!(vdp.take_dma_cycles(), 4);
}

#[test]
fn register_dma_fill_and_copy_use_their_mode_bits() {
    let mut vdp = Vdp::new();
    enable_dma(&mut vdp);

    // Fill: mode 10, source low byte 0xAB is the fill value.
    program_dma(&mut vdp, 0x00AB, 4, 0x80);
    trigger_dma(&mut vdp, 0x21, 0x0300);
    for offset in (0..8).step_by(2) {
        assert_eq!(vram_word(&vdp, 0x0300 + offset), 0xABAB);
    }
    assert_eq!(vdp.take_dma_cycles(), 8);

    // Copy: mode 11 plus command CD4; source VRAM 0x0100 to command address.
    write_vram(&mut vdp, 0x0100, &[0xCAFE, 0xBEEF]).expect("source");
    program_dma(&mut vdp, 0x0100, 2, 0xC0);
    trigger_dma(&mut vdp, 0x30, 0x0400);
    assert_eq!(vram_word(&vdp, 0x0400), 0xCAFE);
    assert_eq!(vram_word(&vdp, 0x0402), 0xBEEF);
    assert_eq!(vdp.take_dma_cycles(), 4);
}

#[test]
fn register_dma_rejects_an_unknown_target_and_copy_without_cd4() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0x1111]).expect("source");
    enable_dma(&mut vdp);
    program_dma(&mut vdp, 0x0100, 1, 0x00);

    // Command code 0x20 requests DMA but selects no destination data space.
    assert_eq!(vdp.write16(CONTROL_PORT_OFFSET, 0x0000), Ok(()));
    assert_eq!(vdp.write16(CONTROL_PORT_OFFSET, 0x0080), Ok(()));
    assert_eq!(
        vdp.write16(DATA_PORT_OFFSET, 0),
        Err(VdpError::DmaUnsupported { code: 0x20 })
    );
    assert!(!vdp.dma_active());

    // Copy (mode 11) requires command bit CD4.
    program_dma(&mut vdp, 0x0100, 1, 0xC0);
    assert_eq!(vdp.write16(CONTROL_PORT_OFFSET, 0x4000), Ok(()));
    assert_eq!(vdp.write16(CONTROL_PORT_OFFSET, 0x0080), Ok(()));
    assert_eq!(
        vdp.write16(DATA_PORT_OFFSET, 0),
        Err(VdpError::DmaUnsupported { code: 0x21 })
    );
    assert!(!vdp.dma_active());
}

#[test]
fn disabled_dma_register_transfer_is_a_no_op() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0xABCD]).expect("source");
    // Register 0x01 bit 4 stays clear.
    program_dma(&mut vdp, 0x0100, 1, 0x00);

    trigger_dma(&mut vdp, 0x21, 0x0200);

    assert_eq!(vram_word(&vdp, 0x0200), 0x0000, "no word was transferred");
    assert!(!vdp.dma_active(), "DMA_BUSY never asserts");
    assert_eq!(vdp.take_dma_cycles(), 0);
}

#[test]
fn taking_dma_cycles_preserves_fifo_waits_and_interrupt_pendings() {
    let mut vdp = Vdp::new();
    write_register(&mut vdp, 0x00, 0x30).expect("enable both interrupt sources");
    vdp.request_interrupt(VdpInterruptSource::VBlank);
    vdp.request_interrupt(VdpInterruptSource::HBlank);

    // Four writes fill the queue; the fifth adds exactly one wait slot.
    set_vram_write(&mut vdp, 0x0000).expect("destination");
    for _ in 0..5 {
        vdp.write16(DATA_PORT_OFFSET, 0x1234).expect("queued write");
    }

    vdp.start_dma(0x0000, 1, DmaMode::Fill).expect("dma");
    assert_eq!(vdp.take_dma_cycles(), 2, "one fill word at two cycles");

    assert_eq!(
        vdp.take_fifo_wait_cycles(),
        1,
        "taking the DMA cost must not discard the FIFO wait accounting"
    );
    let pending = vdp.take_interrupts();
    assert!(
        pending.vblank_pending,
        "taking the DMA cost must not acknowledge VBlank"
    );
    assert!(
        pending.hblank_pending,
        "taking the DMA cost must not acknowledge HBlank"
    );
}
