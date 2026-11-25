//! YM2612 registers, two-port protocol, timers/status, DAC and bus wiring.

use std::error::Error;

use chronofork_megadrive::MegadriveBus;
use chronofork_megadrive::audio::ym2612::{
    SELECTOR_MAX, SNAPSHOT_LEN, SNAPSHOT_MAGIC, SNAPSHOT_SELECTOR_OFFSET, SNAPSHOT_VERSION,
};
use chronofork_megadrive::audio::{AudioSnapshotError, EgState, Ym2612, YmBank, YmIrq, YmPort};
use chronofork_megadrive::bus::{
    YM2612_BASE, YM2612_Z80_BASE, YM2612_Z80_SOUND_END, Z80_STALL_MASTER_CYCLES,
};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::z80::Z80Bus;

/// Selects `bank`/`address` through that bank's address port, then writes
/// `value` through the first data port.
///
/// The data port carries no bank: it commits to whatever the global selector
/// names, which the preceding address write just set. That is the point of the
/// model, so the helper makes the two steps explicit instead of hiding the bank
/// inside a data write.
fn write_register(ym: &mut Ym2612, bank: YmBank, address: u8, value: u8) {
    ym.write_address(bank, address);
    ym.write_port(YmPort::Data, value);
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
fn one_global_selector_names_the_destination_of_both_data_ports() {
    // The last address write wins, and it carries the bank: selecting bank 0
    // register 0x40 and then bank 1 register 0x50 leaves the selector on
    // bank 1/0x50, so a data write through the FIRST port reaches bank 1/0x50.
    // A per-bank latch model would have written bank 0/0x40 instead.
    let mut ym = Ym2612::new();
    ym.write_address(YmBank::One, 0x40);
    ym.write_address(YmBank::Two, 0x50);
    assert_eq!(ym.selector(), 0x150);
    assert_eq!(ym.selector_bank(), YmBank::Two);
    assert_eq!(ym.selector_register(), 0x50);
    ym.write_port(YmPort::Data, 0x01);
    assert_eq!(ym.register(YmBank::Two, 0x50), 0x01);
    assert_eq!(
        ym.register(YmBank::One, 0x40),
        0x00,
        "the superseded bank 0 register must not change"
    );

    // Both data ports are equivalent: the second one commits to the same
    // selector the first one did.
    let mut second = Ym2612::new();
    second.write_address(YmBank::One, 0x40);
    second.write_address(YmBank::Two, 0x50);
    second.write_port(YmPort::Data, 0x02);
    assert_eq!(second.register(YmBank::Two, 0x50), 0x02);
    assert_eq!(second.register(YmBank::One, 0x40), 0x00);
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
    assert_eq!(ym.read_port(YmPort::Data), ym.status());
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

// ---------------------------------------------------------------------------
// The global bank+register selector (ADR-0018)
// ---------------------------------------------------------------------------

/// The bank/register split of the selector, and the selector's reset value.
#[test]
fn selector_is_one_global_bank_and_register() {
    let mut ym = Ym2612::new();
    assert_eq!(ym.selector(), 0, "the selector resets to bank 1/register 0");
    assert_eq!(ym.selector_bank(), YmBank::One);
    assert_eq!(ym.selector_register(), 0x00);

    ym.write_address(YmBank::One, 0xFF);
    assert_eq!(ym.selector(), 0x0FF);
    assert_eq!(ym.selector_bank(), YmBank::One);
    assert_eq!(ym.selector_register(), 0xFF);

    ym.write_address(YmBank::Two, 0x00);
    assert_eq!(ym.selector(), 0x100, "bank 2 starts one part up");
    assert_eq!(ym.selector_bank(), YmBank::Two);
    assert_eq!(ym.selector_register(), 0x00);

    ym.write_address(YmBank::Two, 0xFF);
    assert_eq!(ym.selector(), SELECTOR_MAX);
    assert_eq!(ym.selector_bank(), YmBank::Two);
    assert_eq!(ym.selector_register(), 0xFF);
}

/// `A0(30); A1(40); D0(11)` writes bank 1/0x40 and leaves bank 0/0x30 alone.
#[test]
fn forward_crossed_write_targets_the_last_selected_register() {
    let mut ym = Ym2612::new();
    ym.write_address(YmBank::One, 0x30);
    ym.write_address(YmBank::Two, 0x40);
    ym.write_port(YmPort::Data, 0x11);
    assert_eq!(ym.register(YmBank::Two, 0x40), 0x11);
    assert_eq!(ym.register(YmBank::One, 0x30), 0x00);
    assert_eq!(ym.register(YmBank::Two, 0x30), 0x00);
    assert_eq!(ym.register(YmBank::One, 0x40), 0x00);
}

/// `A1(40); A0(30); D1(22)` writes bank 0/0x30 through the **second** data port.
#[test]
fn reverse_crossed_write_targets_the_last_selected_register() {
    let mut ym = Ym2612::new();
    ym.write_address(YmBank::Two, 0x40);
    ym.write_address(YmBank::One, 0x30);
    ym.write_port(YmPort::Data, 0x22);
    assert_eq!(ym.register(YmBank::One, 0x30), 0x22);
    assert_eq!(ym.register(YmBank::Two, 0x40), 0x00);
    assert_eq!(ym.register(YmBank::One, 0x40), 0x00);
    assert_eq!(ym.register(YmBank::Two, 0x30), 0x00);
}

/// `A0(30); A1(30); D0(11)` must change **bank 1**/0x30: the bank comes from the
/// last address write, not from the data port. This is the vector that rules out
/// a "shared 8-bit address, bank from the data port" model.
#[test]
fn same_register_number_across_banks_still_selects_the_last_bank() {
    let mut ym = Ym2612::new();
    ym.write_address(YmBank::One, 0x30);
    ym.write_address(YmBank::Two, 0x30);
    assert_eq!(ym.selector(), 0x130);
    ym.write_port(YmPort::Data, 0x11);
    assert_eq!(ym.register(YmBank::Two, 0x30), 0x11);
    assert_eq!(ym.register(YmBank::One, 0x30), 0x00);
}

/// Paired same-port writes keep their meaning: the destination is unchanged from
/// the per-bank model for every non-interleaved sequence.
#[test]
fn paired_writes_still_target_their_own_port_pair() {
    let mut ym = Ym2612::new();
    write_register(&mut ym, YmBank::One, 0x30, 0x11);
    write_register(&mut ym, YmBank::Two, 0x40, 0x22);
    assert_eq!(ym.register(YmBank::One, 0x30), 0x11);
    assert_eq!(ym.register(YmBank::Two, 0x40), 0x22);
    assert_eq!(ym.register(YmBank::One, 0x40), 0x00);
    assert_eq!(ym.register(YmBank::Two, 0x30), 0x00);
}

/// Crossed writes reach the audio model, not only the shadow: key-on through one
/// port pair after selecting through the other keys the selected bank's channel.
#[test]
fn crossed_key_on_reaches_the_selected_banks_channel() {
    let mut ym = Ym2612::new();
    // Select bank 2/0x28 and write key-on through the FIRST data port.
    ym.write_address(YmBank::Two, 0x28);
    ym.write_port(YmPort::Data, 0xF4);
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
        "0xF4 keys channel 4 whatever port carried it"
    );
}

/// The 68000 window, the Z80 window and the `0x5F8B` mirror share one selector.
#[test]
fn all_buses_and_the_mirror_share_one_global_selector() -> Result<(), Box<dyn Error>> {
    // 68000: A0(30); A1(40); D0(11).
    let mut m68k = bus()?;
    m68k.write8(YM2612_BASE, 0x30)?;
    m68k.write8(YM2612_BASE + 2, 0x40)?;
    m68k.write8(YM2612_BASE + 1, 0x11)?;
    assert_eq!(m68k.ym().selector(), 0x140);
    assert_eq!(m68k.ym().register(YmBank::Two, 0x40), 0x11);
    assert_eq!(m68k.ym().register(YmBank::One, 0x30), 0x00);

    // The same sequence on the Z80 side.
    let mut z80 = bus()?;
    Z80Bus::write8(&mut z80, YM2612_Z80_BASE, 0x30)?;
    Z80Bus::write8(&mut z80, YM2612_Z80_BASE + 2, 0x40)?;
    Z80Bus::write8(&mut z80, YM2612_Z80_BASE + 1, 0x11)?;
    assert_eq!(z80.ym().selector(), 0x140);
    assert_eq!(z80.ym().register(YmBank::Two, 0x40), 0x11);
    assert_eq!(z80.ym().register(YmBank::One, 0x30), 0x00);

    // The mirror 0x5F8B is the same bank-2 data port as 0x4003.
    let mut mirror = bus()?;
    Z80Bus::write8(&mut mirror, 0x4000, 0x30)?;
    Z80Bus::write8(&mut mirror, 0x4002, 0x40)?;
    Z80Bus::write8(&mut mirror, 0x5F8B, 0x11)?;
    assert_eq!(mirror.ym().selector(), 0x140);
    assert_eq!(mirror.ym().register(YmBank::Two, 0x40), 0x11);
    assert_eq!(mirror.ym().register(YmBank::One, 0x30), 0x00);
    Ok(())
}

/// The four logical ports repeat across the whole `SOUND` region with period 4.
#[test]
fn z80_sound_region_repeats_the_four_logical_ports() -> Result<(), Box<dyn Error>> {
    for base in (YM2612_Z80_BASE..=YM2612_Z80_SOUND_END).step_by(4) {
        let mut bus = bus()?;
        Z80Bus::write8(&mut bus, base, 0x30)?;
        Z80Bus::write8(&mut bus, base + 2, 0x40)?;
        Z80Bus::write8(&mut bus, base + 1, 0x11)?;
        assert_eq!(
            bus.ym().selector(),
            0x140,
            "address ports at 0x{base:04X} and +2 select bank 1/0x40"
        );
        assert_eq!(
            bus.ym().register(YmBank::Two, 0x40),
            0x11,
            "the data port at 0x{:04X} commits to the selector",
            base + 1
        );
        assert_eq!(bus.ym().register(YmBank::One, 0x30), 0x00);
    }
    Ok(())
}

/// Status reads are unchanged: every one of the four ports and the mirror
/// returns the status byte on both buses.
#[test]
fn status_reads_are_unchanged_on_every_port() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    let (ym, _) = bus.audio_mut();
    write_register(ym, YmBank::One, 0x27, 0x05);
    ym.timers_elapsed(1_000_000);
    let status = bus.ym().status();
    assert_eq!(status & 0x01, 0x01, "timer A has overflowed");
    for offset in 0..4 {
        assert_eq!(bus.read8(YM2612_BASE + offset)?, status);
    }
    for offset in 0..4 {
        assert_eq!(Z80Bus::read8(&mut bus, 0x4000 + offset as u16)?, status);
    }
    assert_eq!(Z80Bus::read8(&mut bus, 0x5F8B)?, status);
    Ok(())
}

/// `A0(27); D1(05)` starts timer A across the port pairs and raises status bit 0
/// after enough emulated time; the same-pair control reads 1 and a no-start
/// negative control reads 0.
#[test]
fn crossed_timer_write_starts_timer_a() {
    fn configure(ym: &mut Ym2612) {
        write_register(ym, YmBank::One, 0x24, 0xFF);
        write_register(ym, YmBank::One, 0x25, 0x03);
    }

    // Positive control: the same pair starts the timer.
    let mut same = Ym2612::new();
    configure(&mut same);
    write_register(&mut same, YmBank::One, 0x27, 0x05);
    same.timers_elapsed(1_000_000);
    assert_eq!(same.status() & 0x01, 0x01, "same-pair control overflows");

    // Discriminator: the crossed port starts the timer, because the data port
    // commits to the register the last address write selected.
    let mut crossed = Ym2612::new();
    configure(&mut crossed);
    crossed.write_address(YmBank::One, 0x27);
    crossed.write_port(YmPort::Data, 0x05);
    crossed.timers_elapsed(1_000_000);
    assert_eq!(
        crossed.status() & 0x01,
        0x01,
        "the crossed data port starts it"
    );

    // Negative control: no start bit, so the flag stays clear.
    let mut negative = Ym2612::new();
    configure(&mut negative);
    write_register(&mut negative, YmBank::One, 0x27, 0x00);
    negative.timers_elapsed(1_000_000);
    assert_eq!(negative.status() & 0x01, 0x00, "no start, no overflow");
}

/// A snapshot taken after both address writes and before the data write restores
/// the exact global selector, and continuing produces bytes identical to the
/// path that never snapshotted.
#[test]
fn selector_survives_a_snapshot_between_the_address_and_data_writes() {
    // Path with a checkpoint between address and data.
    let mut snapshotted = Ym2612::new();
    snapshotted.write_address(YmBank::One, 0x30);
    snapshotted.write_address(YmBank::Two, 0x40);
    let pending = snapshotted.snapshot_bytes();
    assert_eq!(
        u16::from_le_bytes([
            pending[SNAPSHOT_SELECTOR_OFFSET],
            pending[SNAPSHOT_SELECTOR_OFFSET + 1]
        ]),
        0x140,
        "the pending snapshot carries the bank-2/0x40 selector"
    );
    let mut restored = Ym2612::new();
    // The restore must succeed: a self-produced v2 snapshot is by construction
    // valid, so a rejection here is the test failing. The assertion reports the
    // typed error it received, which is the same failure visibility the previous
    // `expect` form gave.
    assert_eq!(
        restored.restore_bytes(&pending),
        Ok(()),
        "a self-produced v2 snapshot must restore"
    );
    assert_eq!(restored.selector(), 0x140);
    restored.write_port(YmPort::Data, 0x11);

    // Reference path without the checkpoint.
    let mut direct = Ym2612::new();
    direct.write_address(YmBank::One, 0x30);
    direct.write_address(YmBank::Two, 0x40);
    direct.write_port(YmPort::Data, 0x11);

    assert_eq!(restored.register(YmBank::Two, 0x40), 0x11);
    assert_eq!(restored.register(YmBank::One, 0x30), 0x00);
    assert_eq!(
        restored.snapshot_bytes(),
        direct.snapshot_bytes(),
        "the restored path continues byte-identically"
    );
}

/// The v2 layout is `CFYM2612` version 2 and still exactly 856 bytes.
#[test]
fn snapshot_is_cfym2612_version_two_at_the_same_length() {
    assert_eq!(SNAPSHOT_MAGIC, *b"CFYM2612");
    assert_eq!(SNAPSHOT_VERSION, 2);
    assert_eq!(SNAPSHOT_LEN, 856);
    assert_eq!(SNAPSHOT_SELECTOR_OFFSET, 521);
    let bytes = Ym2612::new().snapshot_bytes();
    assert_eq!(bytes.len(), SNAPSHOT_LEN);
    assert_eq!(bytes[..8], SNAPSHOT_MAGIC);
    assert_eq!(bytes[8], SNAPSHOT_VERSION);
}

/// A version-1 payload is rejected by **version** and its two per-bank latch
/// bytes are never reinterpreted as a selector; the device is untouched.
#[test]
fn version_one_snapshot_is_rejected_without_mutation() {
    let mut ym = Ym2612::new();
    ym.write_address(YmBank::Two, 0x40);
    ym.write_port(YmPort::Data, 0x11);
    let before = ym.snapshot_bytes();

    let mut tampered = ym.snapshot_bytes();
    tampered[8] = 1;
    assert_eq!(
        ym.restore_bytes(&tampered),
        Err(AudioSnapshotError::Version { found: 1 })
    );
    assert_eq!(
        ym.snapshot_bytes(),
        before,
        "a rejected restore mutates nothing"
    );

    // An old magic is rejected by magic, before the version byte is trusted.
    let mut bad_magic = ym.snapshot_bytes();
    bad_magic[0] = b'X';
    assert_eq!(ym.restore_bytes(&bad_magic), Err(AudioSnapshotError::Magic));
    assert_eq!(ym.snapshot_bytes(), before);
}

/// Out-of-range selectors and wrong lengths are typed rejections, and no
/// padding or truncation leniency exists.
#[test]
fn invalid_selector_and_length_are_typed_rejections() {
    let mut ym = Ym2612::new();
    ym.write_address(YmBank::One, 0x30);
    let before = ym.snapshot_bytes();

    for selector in [SELECTOR_MAX + 1, 0xFFFF] {
        let mut tampered = ym.snapshot_bytes();
        tampered[SNAPSHOT_SELECTOR_OFFSET..SNAPSHOT_SELECTOR_OFFSET + 2]
            .copy_from_slice(&selector.to_le_bytes());
        assert_eq!(
            ym.restore_bytes(&tampered),
            Err(AudioSnapshotError::Value {
                field: "ym selector"
            }),
            "selector {selector:#06X} must be rejected by value"
        );
        assert_eq!(ym.snapshot_bytes(), before);
    }

    assert_eq!(
        ym.restore_bytes(&before[..SNAPSHOT_LEN - 1]),
        Err(AudioSnapshotError::Truncated)
    );
    let mut long = before.clone();
    long.push(0);
    assert_eq!(
        ym.restore_bytes(&long),
        Err(AudioSnapshotError::TrailingBytes)
    );
    assert_eq!(ym.snapshot_bytes(), before);
}

/// The expected rejection for one snapshot input, derived from the documented
/// **length-first** rule rather than from a recorded run.
///
/// `restore_bytes` compares the exact length before it reads anything, then
/// checks the magic, then the version, then the field values. This helper is the
/// executable statement of that precedence for a combined-defect input.
fn expected_rejection(bytes: &[u8]) -> AudioSnapshotError {
    if bytes.len() < SNAPSHOT_LEN {
        return AudioSnapshotError::Truncated;
    }
    if bytes.len() > SNAPSHOT_LEN {
        return AudioSnapshotError::TrailingBytes;
    }
    if bytes[..SNAPSHOT_MAGIC.len()] != SNAPSHOT_MAGIC {
        return AudioSnapshotError::Magic;
    }
    let version = bytes[SNAPSHOT_MAGIC.len()];
    if version != SNAPSHOT_VERSION {
        return AudioSnapshotError::Version { found: version };
    }
    let selector = u16::from_le_bytes([
        bytes[SNAPSHOT_SELECTOR_OFFSET],
        bytes[SNAPSHOT_SELECTOR_OFFSET + 1],
    ]);
    if selector > SELECTOR_MAX {
        return AudioSnapshotError::Value {
            field: "ym selector",
        };
    }
    // Unreachable for the inputs this test builds: a full-length, correct-magic,
    // correct-version, in-range-selector snapshot restores.
    AudioSnapshotError::Value {
        field: "envelope attenuation",
    }
}

/// Combined-defect inputs are reported by the **length** check first, and by the
/// later checks only when the length is exact.
///
/// The claim this pins: a wrong-length input is reported as `Truncated` or
/// `TrailingBytes` **regardless** of any bad magic, bad version or out-of-range
/// selector it also carries, because the exact length is compared before
/// anything is read. Only a byte-exact-length input reaches the magic, version
/// and value checks, in that order.
#[test]
fn combined_defects_are_reported_length_first() {
    let mut ym = Ym2612::new();
    ym.write_address(YmBank::One, 0x30);
    let valid = ym.snapshot_bytes();
    assert_eq!(valid.len(), SNAPSHOT_LEN);
    let untouched = ym.snapshot_bytes();

    // (1) valid v2 plus one extra byte: too long, so `TrailingBytes`, even though
    // every other byte of the input is a perfectly valid snapshot.
    let mut valid_plus_extra = valid.clone();
    valid_plus_extra.push(0);

    // (2) a v1-versioned snapshot plus one extra byte: too long, so
    // `TrailingBytes` wins over the version mismatch.
    let mut v1_plus_extra = valid.clone();
    v1_plus_extra[SNAPSHOT_MAGIC.len()] = 1;
    v1_plus_extra.push(0);

    // (3) a short input that also claims version 1: `Truncated` wins, and the
    // version byte is never consulted.
    let mut short_v1 = valid[..300].to_vec();
    short_v1[SNAPSHOT_MAGIC.len()] = 1;

    // (4) bad magic plus one extra byte: too long, so `TrailingBytes` wins over
    // the magic mismatch.
    let mut bad_magic_plus_extra = valid.clone();
    bad_magic_plus_extra[0] = b'X';
    bad_magic_plus_extra.push(0);

    // (5) bad magic plus an out-of-range selector, both at exact length: the
    // magic check is the first one after the length, so `Magic` wins over the
    // selector value.
    let mut bad_magic_and_selector = valid.clone();
    bad_magic_and_selector[0] = b'X';
    bad_magic_and_selector[SNAPSHOT_SELECTOR_OFFSET..SNAPSHOT_SELECTOR_OFFSET + 2]
        .copy_from_slice(&(SELECTOR_MAX + 1).to_le_bytes());

    // (6) a v1-versioned snapshot at exact length with an out-of-range selector:
    // `Version` wins over the selector value.
    let mut bad_version_and_selector = valid.clone();
    bad_version_and_selector[SNAPSHOT_MAGIC.len()] = 1;
    bad_version_and_selector[SNAPSHOT_SELECTOR_OFFSET..SNAPSHOT_SELECTOR_OFFSET + 2]
        .copy_from_slice(&(SELECTOR_MAX + 1).to_le_bytes());

    for (label, bytes) in [
        (
            "(1) valid v2 with an extra byte",
            valid_plus_extra.as_slice(),
        ),
        (
            "(2) v1 version with an extra byte",
            v1_plus_extra.as_slice(),
        ),
        ("(3) short input claiming version 1", short_v1.as_slice()),
        (
            "(4) bad magic with an extra byte",
            bad_magic_plus_extra.as_slice(),
        ),
        (
            "(5) bad magic and out-of-range selector",
            bad_magic_and_selector.as_slice(),
        ),
        (
            "(6) v1 version and out-of-range selector",
            bad_version_and_selector.as_slice(),
        ),
    ] {
        let expected = expected_rejection(bytes);
        assert_eq!(
            ym.restore_bytes(bytes),
            Err(expected),
            "{label} must be rejected by the documented length-first precedence"
        );
        assert_eq!(
            ym.snapshot_bytes(),
            untouched,
            "{label}: a rejected restore must not mutate the device"
        );
    }

    // The winner is genuinely the length, then the earlier check in order, and
    // not one fixed global preference: (2) is reported as `TrailingBytes` even
    // though its version byte is wrong, while (6) is reported as `Version` even
    // though its selector is out of range.
    assert_eq!(
        ym.restore_bytes(&v1_plus_extra),
        Err(AudioSnapshotError::TrailingBytes)
    );
    assert_eq!(
        ym.restore_bytes(&bad_version_and_selector),
        Err(AudioSnapshotError::Version { found: 1 })
    );

    // The reference helper agrees with the device on a clean snapshot too.
    assert_eq!(ym.restore_bytes(&valid), Ok(()));
    assert_eq!(ym.snapshot_bytes(), valid);
}
