#![allow(clippy::expect_used)]

//! SN76489-compatible PSG: latch/data protocol, tone and noise channels,
//! sixteen attenuation levels, deterministic sampling and the open-bus read.

use std::error::Error;

use chronofork_megadrive::MegadriveBus;
use chronofork_megadrive::audio::Psg;
use chronofork_megadrive::audio::psg::{
    ATTENUATION_LEVELS, DIVIDER_MASK, LFSR_RESET_VALUE, MAX_CHANNEL_AMPLITUDE,
    NOISE_CLOCK_DIVIDERS, OPEN_BUS_VALUE,
};
use chronofork_megadrive::cartridge::Cartridge;
use chronofork_megadrive::z80::{Z80Bus, Z80Error};

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
fn noise_lfsr_repeats_deterministically() {
    // Tone channel 2 at divider 1 toggles every tick, which clocks the noise
    // LFSR at the PSG tick rate.
    fn programmed() -> Psg {
        let mut psg = Psg::new();
        psg.write(0xC1); // latch tone 2, divider low nibble 1
        psg.write(0x00); // divider 1
        psg.write(0xDF); // tone 2 volume, attenuation 15 (silent)
        psg.write(0xE7); // noise: white feedback, clocked by tone channel 2
        psg.write(0xF0); // noise volume 0
        psg
    }

    let mut first = programmed();
    let mut second = programmed();
    assert_eq!(first.noise_lfsr(), LFSR_RESET_VALUE);
    let period = (1 << 15) - 1;
    let mut first_samples = Vec::with_capacity(1024);
    let mut second_samples = Vec::with_capacity(1024);
    for _ in 0..period {
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
    assert!(matches!(
        Z80Bus::read8(&mut bus, 0x7F04),
        Err(Z80Error::Memory { address: 0x7F04 })
    ));
    Ok(())
}
