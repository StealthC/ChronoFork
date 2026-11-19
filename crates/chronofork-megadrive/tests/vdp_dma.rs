#![allow(clippy::expect_used)]

//! VDP DMA transfer coverage.
//!
//! Phase 8.3 performs DMA eagerly: `start_dma` validates the whole source and
//! destination range before mutating anything, performs the transfer, records
//! the slot cost and leaves DMA_BUSY set until the owner takes the cost. In this
//! slice the 68000-address source operand names a VRAM byte address (the VDP has
//! no external memory view), documented as a modelling limit.

use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, DATA_PORT_OFFSET, DmaMode, STATUS_DMA_BUSY, Vdp, VdpError, VramTarget,
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

#[test]
fn dma_registers_program_a_triggered_transfer() {
    let mut vdp = Vdp::new();
    write_vram(&mut vdp, 0x0100, &[0xABCD, 0x1234]).expect("source");
    write_register(&mut vdp, 0x13, 0x00).expect("source low");
    write_register(&mut vdp, 0x14, 0x01).expect("source high");
    write_register(&mut vdp, 0x15, 0x00).expect("source bank/mode");
    write_register(&mut vdp, 0x16, 0x02).expect("length low");
    write_register(&mut vdp, 0x17, 0x00).expect("length high, 68k->VRAM");

    // DMA command: CD5 set in the second control word, destination address 0.
    vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
        .expect("first word");
    vdp.write16(CONTROL_PORT_OFFSET, 0x0080)
        .expect("second word");
    vdp.write16(DATA_PORT_OFFSET, 0)
        .expect("data-port access triggers the transfer");

    assert_eq!(vram_word(&vdp, 0x0000), 0xABCD);
    assert_eq!(vram_word(&vdp, 0x0002), 0x1234);
    assert_eq!(vdp.take_dma_cycles(), 4);
}
