//! YM2612 registers, two-port protocol, timers/status, DAC and bus wiring.

use std::error::Error;

use chronofork_megadrive::MegadriveBus;
use chronofork_megadrive::audio::{EgState, Ym2612, YmBank, YmIrq, YmPort};
use chronofork_megadrive::bus::{YM2612_BASE, Z80_STALL_MASTER_CYCLES};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::z80::Z80Bus;

fn write_register(ym: &mut Ym2612, bank: YmBank, address: u8, value: u8) {
    ym.write_bank(bank, YmPort::Address, address);
    ym.write_bank(bank, YmPort::Data, value);
}

/// Reads operator 1's envelope state for all six channels.
fn key_states(ym: &Ym2612) -> [EgState; 6] {
    std::array::from_fn(|channel| match ym.envelope(channel, 0) {
        Some(view) => view.state,
        None => EgState::Off,
    })
}

const ALL_OFF: [EgState; 6] = [EgState::Off; 6];

fn bus() -> Result<MegadriveBus, Box<dyn Error>> {
    let mut image = vec![0_u8; 0x4000];
    assert!(chronofork_megadrive::test_support::stamp_header(
        &mut image, "JUE"
    ));
    Ok(MegadriveBus::new(Cartridge::parse(image)?))
}

#[test]
fn register_write_read_selects_bank() {
    let mut ym = Ym2612::default();
    write_register(&mut ym, YmBank::One, 0x30, 0x0A);
    write_register(&mut ym, YmBank::Two, 0x30, 0x0B);
    assert_eq!(ym.register(YmBank::One, 0x30), 0x0A);
    assert_eq!(ym.register(YmBank::Two, 0x30), 0x0B);
    assert_eq!(ym.register(YmBank::One, 0x31), 0x00);
}

#[test]
fn single_port_write_and_read_use_bank_zero() {
    let mut ym = Ym2612::new();
    ym.write(YmPort::Address, 0x30);
    ym.write(YmPort::Data, 0x0C);
    assert_eq!(ym.register(YmBank::One, 0x30), 0x0C);
    assert_eq!(ym.read(YmPort::Data), ym.status());
}

#[test]
fn address_latches_are_separate_per_bank() {
    let mut ym = Ym2612::new();
    ym.write_bank(YmBank::One, YmPort::Address, 0x40);
    ym.write_bank(YmBank::Two, YmPort::Address, 0x50);
    ym.write_bank(YmBank::One, YmPort::Data, 0x01);
    ym.write_bank(YmBank::Two, YmPort::Data, 0x02);
    assert_eq!(ym.register(YmBank::One, 0x40), 0x01);
    assert_eq!(ym.register(YmBank::Two, 0x50), 0x02);
    assert_eq!(ym.register(YmBank::One, 0x50), 0x00);
}

#[test]
fn key_on_data_byte_selects_the_channel_ignoring_the_port_bank() {
    let mut ym = Ym2612::default();
    assert_eq!(key_states(&ym), ALL_OFF);

    // 0xF0: bits 0-1 = 0 and bit 2 clear, so the channel is 1 and all four
    // operators are keyed. Channels 3 and 4 must stay Off.
    write_register(&mut ym, YmBank::One, 0x28, 0xF0);
    assert_eq!(
        key_states(&ym),
        [
            EgState::Attack,
            EgState::Off,
            EgState::Off,
            EgState::Off,
            EgState::Off,
            EgState::Off
        ],
        "0xF0 keys only channel 1"
    );

    // 0xF4 sets bit 2, which selects the second group: channel 4 only.
    let mut ym = Ym2612::default();
    write_register(&mut ym, YmBank::One, 0x28, 0xF4);
    assert_eq!(
        key_states(&ym),
        [
            EgState::Off,
            EgState::Off,
            EgState::Off,
            EgState::Attack,
            EgState::Off,
            EgState::Off
        ],
        "0xF4 keys only channel 4"
    );

    // The port bank must not select the channel: bank 2 with 0xF4 still keys 4.
    let mut ym = Ym2612::default();
    write_register(&mut ym, YmBank::Two, 0x28, 0xF4);
    assert_eq!(
        key_states(&ym),
        [
            EgState::Off,
            EgState::Off,
            EgState::Off,
            EgState::Attack,
            EgState::Off,
            EgState::Off
        ],
        "the data byte names the channel, not the port pair"
    );

    // Bits 0-1 = 3 are reserved and key nothing.
    let mut ym = Ym2612::default();
    write_register(&mut ym, YmBank::One, 0x28, 0xF3);
    assert_eq!(key_states(&ym), ALL_OFF, "channel selector 3 is reserved");

    // The operator mask applies to the selected channel only.
    let mut ym = Ym2612::default();
    write_register(&mut ym, YmBank::One, 0x28, 0x10);
    assert_eq!(
        key_states(&ym),
        [
            EgState::Attack,
            EgState::Off,
            EgState::Off,
            EgState::Off,
            EgState::Off,
            EgState::Off
        ],
        "0x10 keys operator 1 of channel 1 only"
    );
    let envelope = ym.envelope(0, 1).map(|view| view.state);
    assert_eq!(envelope, Some(EgState::Off), "operator 2 was not keyed");
}

#[test]
fn timer_a_overflow_sets_status_flag() {
    let mut ym = Ym2612::new();
    write_register(&mut ym, YmBank::One, 0x24, 0x00);
    write_register(&mut ym, YmBank::One, 0x25, 0x00);
    write_register(&mut ym, YmBank::One, 0x27, 0x05); // load A + enable A
    assert_eq!(ym.status(), 0);
    ym.timers_elapsed(1_000_000);
    assert_eq!(ym.status() & 0x01, 0x01);
    assert_eq!(ym.take_irq(), Some(YmIrq::TimerA));
    assert_eq!(ym.take_irq(), None, "the IRQ line latch is consumed");
    assert_eq!(ym.status() & 0x01, 0x01, "the status flag is still set");

    write_register(&mut ym, YmBank::One, 0x27, 0x15); // reset the flags
    assert_eq!(ym.status(), 0);
    assert_eq!(ym.take_irq(), None);
}

#[test]
fn timer_b_overflow_sets_status_flag() {
    let mut ym = Ym2612::new();
    write_register(&mut ym, YmBank::One, 0x26, 0x00);
    write_register(&mut ym, YmBank::One, 0x27, 0x0A); // load B + enable B
    ym.timers_elapsed(5_000_000);
    assert_eq!(ym.status() & 0x02, 0x02);
    assert_eq!(ym.take_irq(), Some(YmIrq::TimerB));
    write_register(&mut ym, YmBank::One, 0x27, 0x12); // reset + load B
    assert_eq!(ym.status(), 0);
}

#[test]
fn timer_without_enable_raises_status_but_no_irq() {
    let mut ym = Ym2612::new();
    write_register(&mut ym, YmBank::One, 0x27, 0x01); // load A only
    ym.timers_elapsed(1_000_000);
    assert_eq!(ym.status() & 0x01, 0x01);
    assert_eq!(ym.take_irq(), None, "IRQ delivery is phase 8.5");
}

#[test]
fn status_read_returns_the_latched_flags() {
    let mut ym = Ym2612::new();
    write_register(&mut ym, YmBank::One, 0x27, 0x05);
    ym.timers_elapsed(1_000_000);
    assert_eq!(ym.read(YmPort::Address), ym.status());
    assert_eq!(ym.read(YmPort::Data), ym.status());
    assert_eq!(ym.read_bank(YmBank::Two, YmPort::Data), ym.status());
}

#[test]
fn dac_register_updates_channel_six() {
    let mut ym = Ym2612::new();
    // Channel 6 is in bank 2; enable both stereo bits.
    write_register(&mut ym, YmBank::Two, 0xB4 + 2, 0xC0);
    write_register(&mut ym, YmBank::One, 0x2B, 0x80); // DAC enable
    write_register(&mut ym, YmBank::One, 0x2A, 0xFF);
    let high = ym.sample();
    write_register(&mut ym, YmBank::One, 0x2A, 0x00);
    let low = ym.sample();
    assert!(high.0 > 0 && high.1 > 0, "0xFF is a positive DAC sample");
    assert!(low.0 < 0 && low.1 < 0, "0x00 is a negative DAC sample");

    write_register(&mut ym, YmBank::One, 0x2B, 0x00); // back to FM
    let (left, right) = ym.sample();
    assert_eq!((left, right), (0, 0), "the unkeyed FM channel is silent");
}

#[test]
fn bus_windows_reach_the_same_ym2612() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    Z80Bus::write8(&mut bus, 0x4000, 0x22)?;
    Z80Bus::write8(&mut bus, 0x4001, 0x08)?;
    assert_eq!(bus.ym().register(YmBank::One, 0x22), 0x08);

    bus.write8(YM2612_BASE + 2, 0x30)?;
    bus.write8(YM2612_BASE + 3, 0x0B)?;
    assert_eq!(bus.ym().register(YmBank::Two, 0x30), 0x0B);
    assert_eq!(bus.read8(YM2612_BASE)?, bus.ym().status());
    Ok(())
}

#[test]
fn z80_ym2612_window_does_not_alias_ram_or_rom() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    Z80Bus::write8(&mut bus, 0x4000, 0x2A)?;
    Z80Bus::write8(&mut bus, 0x4001, 0x7F)?;
    assert_eq!(bus.ym().register(YmBank::One, 0x2A), 0x7F);
    assert_eq!(Z80Bus::read8(&mut bus, 0x0000)?, 0x00);
    assert_eq!(bus.rom()[0], 0x00);
    Ok(())
}

#[test]
fn ym2612_window_access_does_not_charge_a_z80_stall() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    // The Z80 owns the bus: no request asserted and out of reset.
    bus.set_arbitration(false, false);
    let _ = bus.read8(YM2612_BASE)?;
    assert_eq!(bus.take_z80_stall_cycles(), 0);
    let _ = bus.read8(0x00A0_0000)?;
    assert_eq!(bus.take_z80_stall_cycles(), Z80_STALL_MASTER_CYCLES);
    Ok(())
}

#[test]
fn samples_are_deterministic_for_identical_register_writes() {
    fn programmed() -> Ym2612 {
        let mut ym = Ym2612::new();
        write_register(&mut ym, YmBank::One, 0xB0, 0x07 << 3 | 0x07); // alg 7, feedback 7
        write_register(&mut ym, YmBank::One, 0xB4, 0xC0);
        for operator in 0..4 {
            let offset = (operator * 4) as u8;
            write_register(&mut ym, YmBank::One, 0x30 + offset, 0x01);
            write_register(&mut ym, YmBank::One, 0x40 + offset, 0x00);
            write_register(&mut ym, YmBank::One, 0x50 + offset, 0x1F);
            write_register(&mut ym, YmBank::One, 0x60 + offset, 0x00);
            write_register(&mut ym, YmBank::One, 0x70 + offset, 0x00);
            write_register(&mut ym, YmBank::One, 0x80 + offset, 0x00);
        }
        write_register(&mut ym, YmBank::One, 0xA0, 0x69);
        write_register(&mut ym, YmBank::One, 0xA4, 0x22);
        write_register(&mut ym, YmBank::One, 0x28, 0xF0);
        ym
    }
    let mut first = programmed();
    let mut second = programmed();
    for _ in 0..64 {
        assert_eq!(first.sample(), second.sample());
    }
}
