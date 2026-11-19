#![allow(clippy::expect_used)]

//! VDP write FIFO and CPU wait-cycle coverage.
//!
//! The VDP data port accepts four queued words. The fifth write while the queue
//! is full adds one 68000-cycle wait slot and is still written (eager data path),
//! never dropped or reordered. The owning backend drains the queue after every
//! step's advance, takes the accumulated wait slots through
//! `MegadriveBus::take_vdp_wait_cycles` and adds `wait * 7` master cycles to the
//! instruction cost.

use chronofork_megadrive::vdp::{CONTROL_PORT_OFFSET, DATA_PORT_OFFSET, Vdp, VdpError};
use chronofork_megadrive::{Cartridge, MegadriveBus, VDP_BASE, test_support::stamp_header};

fn write_register(vdp: &mut Vdp, register: u8, value: u8) -> Result<(), VdpError> {
    vdp.write16(
        CONTROL_PORT_OFFSET,
        0x8000 | (u16::from(register) << 8) | u16::from(value),
    )
}

fn prepare_data_port(vdp: &mut Vdp) -> Result<(), VdpError> {
    write_register(vdp, 0x0F, 2)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0x4000)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
}

fn queued_words(vdp: &Vdp) -> Vec<u16> {
    vdp.vram()[..64]
        .chunks_exact(2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .take_while(|word| *word != 0)
        .collect()
}

#[test]
fn four_words_fit_and_the_fifth_adds_one_wait_slot() {
    let mut vdp = Vdp::new();
    prepare_data_port(&mut vdp).expect("data port");

    for word in 1..=4_u16 {
        vdp.write16(DATA_PORT_OFFSET, word).expect("queued write");
    }
    assert_eq!(vdp.fifo_len(), 4);
    assert_eq!(vdp.take_fifo_wait_cycles(), 0, "four words fit");

    vdp.write16(DATA_PORT_OFFSET, 5).expect("overflow write");
    assert_eq!(vdp.fifo_len(), 4, "the queue stays bounded");
    assert_eq!(vdp.take_fifo_wait_cycles(), 1, "one overflow slot");
    assert_eq!(vdp.take_fifo_wait_cycles(), 0, "the counter is consumed");

    // Every write reached VRAM in order, including the overflowing one.
    assert_eq!(queued_words(&vdp), vec![1, 2, 3, 4, 5]);
}

#[test]
fn a_full_fifo_never_drops_or_reorders_writes() {
    let mut vdp = Vdp::new();
    prepare_data_port(&mut vdp).expect("data port");

    let written: Vec<u16> = (0x1111..=0x6666).step_by(0x1111).collect();
    for word in &written {
        vdp.write16(DATA_PORT_OFFSET, *word).expect("data write");
    }
    assert_eq!(vdp.fifo_len(), 4);
    assert_eq!(vdp.take_fifo_wait_cycles(), 2, "two overflow writes");
    assert_eq!(queued_words(&vdp), written);
}

#[test]
fn draining_releases_the_queue_without_losing_writes() {
    let mut vdp = Vdp::new();
    prepare_data_port(&mut vdp).expect("data port");
    for word in 1..=4_u16 {
        vdp.write16(DATA_PORT_OFFSET, word).expect("queued write");
    }
    assert_eq!(vdp.fifo_len(), 4);

    vdp.drain_fifo();
    assert_eq!(vdp.fifo_len(), 0);
    assert_eq!(queued_words(&vdp), vec![1, 2, 3, 4]);

    // A drained queue accepts a new word without a wait slot.
    vdp.write16(DATA_PORT_OFFSET, 9).expect("post-drain write");
    assert_eq!(vdp.fifo_len(), 1);
    assert_eq!(vdp.take_fifo_wait_cycles(), 0);
    assert_eq!(queued_words(&vdp), vec![1, 2, 3, 4, 9]);
}

fn test_bus() -> MegadriveBus {
    let mut rom = vec![0_u8; 0x400];
    assert!(stamp_header(&mut rom, "JUE"), "synthetic header");
    MegadriveBus::new(Cartridge::parse(rom).expect("valid cartridge"))
}

#[test]
fn the_bus_surfaces_the_accumulated_wait_slots() {
    let mut bus = test_bus();
    bus.write16(VDP_BASE + 4, 0x8F02).expect("auto-increment");
    bus.write16(VDP_BASE + 4, 0x4000)
        .expect("command first word");
    bus.write16(VDP_BASE + 4, 0x0000)
        .expect("command second word");
    for word in 1..=5_u16 {
        bus.write16(VDP_BASE, word).expect("data write");
    }

    assert_eq!(bus.vdp().fifo_len(), 4);
    assert_eq!(bus.take_vdp_wait_cycles(), 1);
    assert_eq!(bus.take_vdp_wait_cycles(), 0);

    // Every write reached VRAM in order even though the queue was full.
    let mut address = 0_usize;
    for expected in 1..=5_u16 {
        let word = u16::from_be_bytes([bus.vdp().vram()[address], bus.vdp().vram()[address + 1]]);
        assert_eq!(word, expected);
        address += 2;
    }
}
