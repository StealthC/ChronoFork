#![allow(clippy::expect_used)]

//! SN76489-compatible PSG: latch/data protocol, tone and noise channels,
//! sixteen attenuation levels, deterministic sampling and the open-bus read.

use std::error::Error;

use chronofork_megadrive::MegadriveBus;
use chronofork_megadrive::audio::Psg;
use chronofork_megadrive::audio::psg::{
    ATTENUATION_LEVELS, DIVIDER_MASK, LFSR_RESET_VALUE, MASTER_CYCLES_PER_SAMPLE,
    MAX_CHANNEL_AMPLITUDE, NOISE_CLOCK_DIVIDERS, OPEN_BUS_VALUE,
};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::cycles::MASTER_CLOCK_HZ_NTSC;
use chronofork_megadrive::z80::Z80Bus;

/// Programs tone channel 0 with `divider` (low ten bits) at full volume.
fn programmed_tone(divider: u16) -> Psg {
    let mut psg = Psg::new();
    psg.write(0x80 | u8::try_from(divider & 0x0F).expect("low nibble fits"));
    psg.write(u8::try_from((divider >> 4) & 0x3F).expect("upper six bits fit"));
    psg.write(0x90); // channel 0 volume, attenuation 0
    psg
}

/// Programs the noise channel with an internal divider selected by `select`.
fn programmed_noise(select: u8) -> Psg {
    let mut psg = Psg::new();
    psg.write(0xE0 | (select & 0x07)); // latch the noise channel control
    psg.write(0xF0); // noise channel volume, attenuation 0
    psg
}

fn bus() -> Result<MegadriveBus, Box<dyn Error>> {
    let mut image = vec![0_u8; 0x4000];
    assert!(chronofork_megadrive::test_support::stamp_header(
        &mut image, "JUE"
    ));
    Ok(MegadriveBus::new(Cartridge::parse(image)?))
}

#[test]
fn read_returns_the_documented_open_value() {
    let mut psg = Psg::new();
    assert_eq!(psg.read(), OPEN_BUS_VALUE);
    psg.write(0x9F);
    assert_eq!(psg.read(), OPEN_BUS_VALUE);
    assert_eq!(OPEN_BUS_VALUE, 0xFF, "the open bus floats high");
}

#[test]
fn latch_and_data_writes_build_a_ten_bit_divider() {
    let mut psg = Psg::new();
    psg.write(0x80 | 0x0F); // latch tone 0, low nibble
    psg.write(0x2A); // data byte, upper six bits
    assert_eq!(psg.tone_divider(0), Some((0x2A << 4) | 0x0F));
    assert!(
        psg.tone_divider(0)
            .is_some_and(|divider| divider <= DIVIDER_MASK)
    );
    psg.write(0x80 | 0x10 | 0x0A); // latch channel 0 volume, attenuation 10
    assert_eq!(psg.attenuation(0), Some(0x0A));
    assert_eq!(
        psg.attenuation(1),
        Some(0x0F),
        "reset silences every channel"
    );
    assert_eq!(
        psg.tone_divider(3),
        None,
        "the noise channel has no tone divider"
    );
}

#[test]
fn attenuation_has_sixteen_monotonic_levels_and_level_fifteen_is_silent() {
    assert_eq!(ATTENUATION_LEVELS, 16);
    let mut previous = MAX_CHANNEL_AMPLITUDE;
    for level in 0..ATTENUATION_LEVELS {
        let mut psg = Psg::new();
        psg.write(0x90 | level);
        psg.write(0x80); // latch tone 0, divider 0 (the slowest tone)
        psg.write(0x00);
        let sample = psg.sample().abs();
        assert!(
            sample <= previous,
            "attenuation {level} is louder than {level}-1"
        );
        previous = sample;
        if level == 0 {
            assert_eq!(sample, MAX_CHANNEL_AMPLITUDE);
        }
        if level == ATTENUATION_LEVELS - 1 {
            assert_eq!(sample, 0, "level 15 is silent");
        }
    }
}

#[test]
fn tone_channel_toggles_once_per_divider_and_is_a_square_wave() {
    let mut fast = programmed_tone(1);
    let samples: Vec<i16> = (0..4).map(|_| fast.sample()).collect();
    assert_eq!(
        samples,
        vec![
            MAX_CHANNEL_AMPLITUDE,
            -MAX_CHANNEL_AMPLITUDE,
            MAX_CHANNEL_AMPLITUDE,
            -MAX_CHANNEL_AMPLITUDE
        ],
        "divider 1 toggles on every PSG tick"
    );

    // A divider write restarts the half-period counter, so the first tick
    // completes a partial half; the steady state holds each level for `divider`
    // ticks.
    let mut slow = programmed_tone(2);
    let samples: Vec<i16> = (0..7).map(|_| slow.sample()).skip(1).take(4).collect();
    assert_eq!(
        samples,
        vec![
            MAX_CHANNEL_AMPLITUDE,
            MAX_CHANNEL_AMPLITUDE,
            -MAX_CHANNEL_AMPLITUDE,
            -MAX_CHANNEL_AMPLITUDE
        ],
        "divider 2 holds each level for two PSG ticks"
    );
}

#[test]
fn noise_rates_follow_the_documented_internal_dividers() {
    // The documented shift periods are the chip-clock divisions N/512, N/1024
    // and N/2048; one internal tick is 16 chip clocks, so the tick periods are
    // 32, 64 and 128.
    assert_eq!(NOISE_CLOCK_DIVIDERS, [32, 64, 128]);
    for (select, divider) in NOISE_CLOCK_DIVIDERS.iter().enumerate() {
        let select = u8::try_from(select).expect("three divisors");
        let mut psg = programmed_noise(select);
        assert_eq!(psg.noise_lfsr(), LFSR_RESET_VALUE);
        for tick in 0..*divider - 1 {
            psg.sample();
            assert_eq!(
                psg.noise_lfsr(),
                LFSR_RESET_VALUE,
                "select {select} shifted before tick {}",
                tick + 1
            );
        }
        psg.sample();
        assert_eq!(
            psg.noise_lfsr(),
            LFSR_RESET_VALUE >> 1,
            "select {select} shifts once every {divider} ticks"
        );
    }
}

#[test]
fn noise_rates_match_the_documented_hardware_frequencies() {
    // One internal tick is MASTER_CYCLES_PER_SAMPLE master cycles, so the three
    // selectors shift at MCLK / (32|64|128 * 240) Hz. Selector 0 is the
    // documented ~6991 Hz SN76489 noise frequency at the NTSC master clock.
    let rate = |ticks: u16| MASTER_CLOCK_HZ_NTSC / (u64::from(ticks) * MASTER_CYCLES_PER_SAMPLE);
    assert_eq!(rate(NOISE_CLOCK_DIVIDERS[0]), 6991);
    assert_eq!(rate(NOISE_CLOCK_DIVIDERS[1]), 3495);
    assert_eq!(rate(NOISE_CLOCK_DIVIDERS[2]), 1747);
}

#[test]
fn noise_lfsr_repeats_deterministically_when_clocked_by_tone_two() {
    // Tone channel 2 at divider 1 toggles every tick, so the noise register is
    // clocked once per **full** tone-2 period: one shift every two ticks.
    fn programmed() -> Psg {
        let mut psg = Psg::new();
        psg.write(0xC1); // latch tone 2, divider low nibble 1
        psg.write(0x00); // divider 1
        psg.write(0xDF); // tone 2 volume, attenuation 15 (silent)
        psg.write(0xE7); // noise: white feedback, clocked by tone channel 2
        psg.write(0xF0); // noise volume 0
        psg
    }

    // The rising half period does not shift; the falling edge completes the
    // period and shifts once.
    let mut edge = programmed();
    assert_eq!(edge.noise_lfsr(), LFSR_RESET_VALUE);
    edge.sample();
    assert_eq!(
        edge.noise_lfsr(),
        LFSR_RESET_VALUE,
        "the half period does not clock the register"
    );
    edge.sample();
    assert_eq!(
        edge.noise_lfsr(),
        LFSR_RESET_VALUE >> 1,
        "the full tone-2 period clocks exactly one shift"
    );

    let mut first = programmed();
    let mut second = programmed();
    assert_eq!(first.noise_lfsr(), LFSR_RESET_VALUE);
    let period = (1 << 15) - 1;
    let mut first_samples = Vec::with_capacity(1024);
    let mut second_samples = Vec::with_capacity(1024);
    for _ in 0..2 * period {
        let sample = first.sample();
        if first_samples.len() < 1024 {
            first_samples.push(sample);
        }
        if second_samples.len() < 1024 {
            second_samples.push(second.sample());
        } else {
            second.sample();
        }
    }
    assert_eq!(
        first.noise_lfsr(),
        LFSR_RESET_VALUE,
        "the 15-bit white-noise sequence repeats after 2^15 - 1 shifts"
    );
    assert_eq!(
        first_samples, second_samples,
        "two identical runs emit identical PCM"
    );
}

#[test]
fn bus_window_reaches_the_psg_and_reads_open_bus() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    Z80Bus::write8(&mut bus, 0x7F11, 0x9F)?; // channel 0 volume 15
    assert_eq!(bus.psg().attenuation(0), Some(0x0F));
    assert_eq!(
        Z80Bus::read8(&mut bus, 0x7F11)?,
        OPEN_BUS_VALUE,
        "the PSG is write-only; the window returns the open value"
    );
    for address in [0x7F00, 0x7F01, 0x7F02, 0x7F03] {
        Z80Bus::write8(&mut bus, address, 0x90)?; // channel 0 volume 0
        assert_eq!(bus.psg().attenuation(0), Some(0));
        assert_eq!(Z80Bus::read8(&mut bus, address)?, OPEN_BUS_VALUE);
    }
    // `0x7F04` owns no device; it is served by the undecoded channel (W1R).
    assert_eq!(
        Z80Bus::read8(&mut bus, 0x7F04)?,
        chronofork_megadrive::bus::Z80_UNDECODED_READ_VALUE
    );
    Ok(())
}

// --- W1Q: the 68000 PSG window is the same chip as the Z80 window ----------

#[test]
fn the_68000_psg_window_drives_the_same_chip_as_the_z80_alias() -> Result<(), Box<dyn Error>> {
    // `0xC00010..=0xC00017` reaches the PSG through the VDP, which is selected
    // for the whole `0xC00000..=0xC0001F` block
    // (`Data/Modules/Sega Mega Drive 1600.xml:318`, PSG reference at `:319`) and
    // forwards its `10*` port cases to the referenced SN76489
    // (`Devices/315-5313/S315-5313_Ports.cpp` around `:1639`). The arbiter ORs
    // the Z80 `0x7F00` window offset into `0xC00000`
    // (`Devices/MD1600IO/MDBusArbiter.cpp:352`/`:516`), so `0x7F11` is the same
    // register. Both windows must drive one instance with one latch/data state.
    let mut via_68000 = bus()?;
    let mut via_z80 = bus()?;

    // The same latch + data sequence through each window leaves equal state.
    for value in [0x80 | 0x2, 0x0F, 0x90, 0x00, 0xE4, 0xF5] {
        let value = u8_to_hex(value);
        via_68000.write8(0x00C0_0011, value)?;
        Z80Bus::write8(&mut via_z80, 0x7F11, value)?;
        assert_eq!(
            via_68000.psg().snapshot_bytes(),
            via_z80.psg().snapshot_bytes(),
            "both windows reach the same PSG state after {value:#04x}"
        );
    }
    // The last two bytes latched the noise channel and set its attenuation.
    assert_eq!(via_68000.psg().attenuation(0), Some(0x00));
    assert_eq!(via_68000.psg().attenuation(3), Some(0x05));
    Ok(())
}

/// Identity helper that keeps the test's byte type obvious.
const fn u8_to_hex(value: u8) -> u8 {
    value
}

#[test]
fn every_byte_of_the_68000_psg_window_mirrors_the_chip() -> Result<(), Box<dyn Error>> {
    // No odd-byte-only rule: each of the eight bytes drives the chip.
    const PSG_BASE: u32 = 0x00C0_0010;
    let mut bus = bus()?;
    for offset in 0..8 {
        let address = PSG_BASE + offset;
        bus.write8(address, 0x90)?; // latch channel 0 at attenuation 0
        bus.write8(address, 0x9F)?; // channel 0 volume 15
        assert_eq!(
            bus.psg().attenuation(0),
            Some(0x0F),
            "byte {offset} of the window reaches the PSG"
        );
    }
    Ok(())
}

#[test]
fn psg_reads_are_deterministic_and_identical_on_both_sides() -> Result<(), Box<dyn Error>> {
    let mut bus = bus()?;
    // Byte reads: the two windows cannot disagree, and the value is the single
    // documented modelling choice (see the `audio::psg` module docs).
    for address in 0x00C0_0010..=0x00C0_0017 {
        assert_eq!(bus.read8(address)?, OPEN_BUS_VALUE, "byte {address:#08x}");
        assert_eq!(
            bus.read8(address)?,
            Z80Bus::read8(&mut bus, 0x7F11)?,
            "both sides return the same open value"
        );
    }
    // Word and long reads over the window decompose into those byte reads.
    assert_eq!(bus.read16(0x00C0_0010)?, 0xFFFF);
    assert_eq!(bus.read32(0x00C0_0014)?, 0xFFFF_FFFF);
    Ok(())
}

#[test]
fn word_and_long_accesses_reach_the_same_psg_bytes() -> Result<(), Box<dyn Error>> {
    // A 68000 word write asserts both byte lanes, and this bus delivers the
    // window byte by byte (high then low). Latch channel 2 at 0x0F, then clear
    // its attenuation in the same word.
    let mut via_word = bus()?;
    via_word.write16(0x00C0_0010, 0x9F00)?;
    assert_eq!(via_word.psg().attenuation(0), Some(0x00));

    let mut via_bytes = bus()?;
    via_bytes.write8(0x00C0_0010, 0x9F)?;
    via_bytes.write8(0x00C0_0011, 0x00)?;
    assert_eq!(
        via_word.psg().snapshot_bytes(),
        via_bytes.psg().snapshot_bytes(),
        "the word write is the two documented byte writes"
    );

    // A long write covers four window bytes the same way.
    let mut via_long = bus()?;
    via_long.write32(0x00C0_0010, 0x9F00_8000)?;
    let mut via_long_bytes = bus()?;
    for (offset, byte) in [0x9F_u8, 0x00, 0x80, 0x00].iter().enumerate() {
        via_long_bytes.write8(0x00C0_0010 + offset as u32, *byte)?;
    }
    assert_eq!(
        via_long.psg().snapshot_bytes(),
        via_long_bytes.psg().snapshot_bytes()
    );
    Ok(())
}

#[test]
fn the_psg_mirror_window_is_bounded_and_beyond_it_is_undecoded() -> Result<(), Box<dyn Error>> {
    // The Z80 mirror derived from the arbiter's `0xC00000 | offset` translation
    // is `0x7F10..=0x7F17`; `0x7F64` maps to `0xC00064`, outside the VDP block,
    // and stays a typed error until the undecoded-access policy lands (W1R).
    let mut bus = bus()?;
    // Reset attenuation is 0x0F (silent), so these two writes must move the
    // value in both directions to be meaningful.
    Z80Bus::write8(&mut bus, 0x7F10, 0x90)?;
    assert_eq!(bus.psg().attenuation(0), Some(0x00));
    Z80Bus::write8(&mut bus, 0x7F17, 0x9F)?;
    assert_eq!(bus.psg().attenuation(0), Some(0x0F));

    // Neither address owns a device: they are served by the undecoded channel, so
    // they must not reach the PSG and must not end the run (W1R).
    let attenuation = bus.psg().attenuation(0);
    let before = bus.unmapped_z80_accesses();
    for address in [0x7F18, 0x7F64] {
        Z80Bus::write8(&mut bus, address, 0x90)?;
        assert_eq!(
            bus.psg().attenuation(0),
            attenuation,
            "{address:#06x} is not the PSG window"
        );
    }
    assert_eq!(bus.unmapped_z80_accesses(), before + 2);
    assert_eq!(
        Z80Bus::read8(&mut bus, 0x7F64)?,
        chronofork_megadrive::bus::Z80_UNDECODED_READ_VALUE
    );
    // The 68000 window is bounded too: the byte past it keeps its typed error.
    assert!(matches!(
        bus.write8(0x00C0_0018, 0x9F),
        Err(chronofork_megadrive::BusError::UnmappedWrite { .. })
    ));
    Ok(())
}
