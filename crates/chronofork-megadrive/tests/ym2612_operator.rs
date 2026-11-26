//! YM2612 operators: sine table, phase generator, algorithms, feedback and LFO.

use std::error::Error;

use chronofork_megadrive::audio::ym2612::{MASTER_CYCLES_PER_SAMPLE, SINE_TABLE_LEN};
use chronofork_megadrive::audio::{Ym2612, YmBank, YmPort};

/// Selects `bank`/`address` through that bank's address port, then writes
/// `value` through a data port. The data port carries no bank: it commits to
/// the register the global selector names.
fn write_register(ym: &mut Ym2612, bank: YmBank, address: u8, value: u8) {
    ym.write_address(bank, address);
    ym.write_port(YmPort::Data, value);
}

/// Programs one channel with all four operators at full level and fast attack.
fn program_channel(
    ym: &mut Ym2612,
    channel: usize,
    algorithm: u8,
    feedback: u8,
    stereo: u8,
    pms: u8,
    lfo_rate: u8,
) {
    let bank = YmBank::of_channel(channel);
    let channel_in_bank = channel % 3;
    for operator in 0..4_u8 {
        let offset = operator * 4;
        write_register(ym, bank, 0x30 + offset, 0x01); // DT=0, MUL=1
        write_register(ym, bank, 0x40 + offset, 0x00); // TL=0
        write_register(ym, bank, 0x50 + offset, 0x1F); // KS=0, AR=31
        write_register(ym, bank, 0x60 + offset, 0x00);
        write_register(ym, bank, 0x70 + offset, 0x00);
        write_register(ym, bank, 0x80 + offset, 0x00);
    }
    write_register(ym, bank, 0xB0, (algorithm << 3) | (feedback & 0x07));
    write_register(ym, bank, 0xB4, stereo | (pms & 0x07));
    write_register(ym, bank, 0xA0 + channel_in_bank as u8, 0x69);
    write_register(ym, bank, 0xA4 + channel_in_bank as u8, 0x22);
    if lfo_rate != 0 {
        write_register(ym, bank, 0x22, 0x08 | (lfo_rate & 0x07));
    }
    // The key-on data byte names the channel (bit 2 selects the second group);
    // the port bank does not select it.
    let group = u8::from(channel >= 3);
    write_register(ym, bank, 0x28, 0xF0 | (group << 2));
}

fn settle(ym: &mut Ym2612) {
    for _ in 0..48 {
        ym.sample();
    }
}

fn operator_output(ym: &Ym2612, operator: usize) -> Result<i32, Box<dyn Error>> {
    match ym.operator_output(0, operator) {
        Some(output) => Ok(output),
        None => Err("operator output missing".into()),
    }
}

fn clamp(value: i32) -> i16 {
    value.clamp(-32768, 32767) as i16
}

#[test]
fn sine_table_is_generated_without_float_and_is_well_formed() {
    let ym = Ym2612::new();
    let table = ym.sine_table();
    assert_eq!(table.len(), SINE_TABLE_LEN);
    assert_eq!(table[0], 0);
    assert!(table[64] > 32_000, "quarter wave peaks");
    assert_eq!(table[128], 0);
    assert!(table[192] < -32_000);
    for index in 1..64 {
        assert_eq!(table[128 + index], -table[index]);
        assert_eq!(table[SINE_TABLE_LEN - index], -table[index]);
    }
    assert!(table[..=64].windows(2).all(|pair| pair[0] <= pair[1]));
    assert!((1..=64).all(|index| table[index] >= 0));
}

#[test]
fn multiple_and_detune_change_the_phase_increment() -> Result<(), Box<dyn Error>> {
    fn chip(multiply: u8, detune: u8) -> Result<u32, Box<dyn Error>> {
        let mut ym = Ym2612::new();
        write_register(&mut ym, YmBank::One, 0xA0, 0x84); // fnum 644
        write_register(&mut ym, YmBank::One, 0xA4, 0x22); // block 4
        write_register(&mut ym, YmBank::One, 0x30, (detune << 4) | multiply);
        match ym.operator_phase_increment(0, 0) {
            Some(increment) => Ok(increment),
            None => Err("missing increment".into()),
        }
    }
    let single = chip(1, 0)?;
    assert_eq!(single, 644 << 3);
    assert_eq!(chip(2, 0)?, single * 2);
    assert_eq!(chip(0, 0)?, single / 2);
    assert!(chip(1, 1)? > single, "positive detune raises the increment");
    assert!(chip(1, 5)? < single, "negative detune lowers the increment");
    Ok(())
}

#[test]
fn algorithm_seven_sums_four_operators() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program_channel(&mut ym, 0, 7, 0, 0xC0, 0, 0);
    settle(&mut ym);
    let (left, right) = ym.sample();
    let mut sum = 0;
    for operator in 0..4 {
        sum += operator_output(&ym, operator)?;
    }
    assert_eq!(left, clamp(sum));
    assert_eq!(right, clamp(sum));
    Ok(())
}

#[test]
fn algorithm_zero_uses_only_the_last_operator() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program_channel(&mut ym, 0, 0, 0, 0xC0, 0, 0);
    settle(&mut ym);
    let (left, _) = ym.sample();
    assert_eq!(left, clamp(operator_output(&ym, 3)?));
    Ok(())
}

#[test]
fn algorithm_four_adds_the_two_carriers() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program_channel(&mut ym, 0, 4, 0, 0xC0, 0, 0);
    settle(&mut ym);
    let (left, _) = ym.sample();
    let expected = operator_output(&ym, 1)? + operator_output(&ym, 3)?;
    assert_eq!(left, clamp(expected));
    Ok(())
}

#[test]
fn feedback_modulates_the_first_operator() -> Result<(), Box<dyn Error>> {
    let mut quiet = Ym2612::new();
    program_channel(&mut quiet, 0, 7, 0, 0xC0, 0, 0);
    settle(&mut quiet);
    let mut loud = Ym2612::new();
    program_channel(&mut loud, 0, 7, 7, 0xC0, 0, 0);
    settle(&mut loud);

    let mut differs = false;
    for _ in 0..64 {
        quiet.sample();
        loud.sample();
        if operator_output(&quiet, 0)? != operator_output(&loud, 0)? {
            differs = true;
        }
    }
    assert!(differs, "feedback changes the first operator's output");
    Ok(())
}

#[test]
fn lfo_pm_changes_the_phase_deterministically() -> Result<(), Box<dyn Error>> {
    fn chip(lfo_rate: u8) -> Ym2612 {
        let mut ym = Ym2612::new();
        program_channel(&mut ym, 0, 7, 0, 0xC0, 7, lfo_rate);
        settle(&mut ym);
        ym
    }
    let mut plain = chip(0);
    let mut modulated = chip(3);
    let mut repeat = chip(3);
    let mut differs = false;
    for _ in 0..32 {
        plain.sample();
        modulated.sample();
        repeat.sample();
        assert_eq!(
            modified_output(&modulated),
            modified_output(&repeat),
            "the LFO is deterministic"
        );
        if modified_output(&plain) != modified_output(&modulated) {
            differs = true;
        }
    }
    assert!(differs, "PM changes the operator phase");

    fn modified_output(ym: &Ym2612) -> Option<i32> {
        ym.operator_output(0, 0)
    }
    Ok(())
}

#[test]
fn stereo_bits_gate_the_output_channels() -> Result<(), Box<dyn Error>> {
    let mut left_only = Ym2612::new();
    program_channel(&mut left_only, 0, 7, 0, 0x40, 0, 0);
    settle(&mut left_only);
    let (left, right) = left_only.sample();
    assert_ne!(left, 0);
    assert_eq!(right, 0);

    let mut right_only = Ym2612::new();
    program_channel(&mut right_only, 0, 7, 0, 0x80, 0, 0);
    settle(&mut right_only);
    let (left, right) = right_only.sample();
    assert_eq!(left, 0);
    assert_ne!(right, 0);

    let mut silent = Ym2612::new();
    program_channel(&mut silent, 0, 7, 0, 0x00, 0, 0);
    settle(&mut silent);
    assert_eq!(silent.sample(), (0, 0));
    Ok(())
}

#[test]
fn sample_clock_is_the_documented_ratio() {
    assert_eq!(MASTER_CYCLES_PER_SAMPLE, 7 * 144);
}

#[test]
fn channel_four_reaches_the_operator_chain_from_the_key_byte() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program_channel(&mut ym, 3, 7, 0, 0xC0, 0, 0); // channel 4
    settle(&mut ym);
    assert_ne!(ym.sample(), (0, 0), "channel 4 produces audio");
    assert!(ym.envelope(3, 0).is_some());
    assert_eq!(
        ym.envelope(0, 0).map(|view| view.state),
        Some(chronofork_megadrive::audio::EgState::Off)
    );
    Ok(())
}
