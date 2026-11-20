//! YM2612 envelope generator: key-on/off, attack/decay/sustain/release and
//! rate/key scaling.

use std::error::Error;

use chronofork_megadrive::audio::{EgState, EnvelopeView, Ym2612, YmBank, YmPort};

fn write_register(ym: &mut Ym2612, bank: YmBank, address: u8, value: u8) {
    ym.write_bank(bank, YmPort::Address, address);
    ym.write_bank(bank, YmPort::Data, value);
}

#[derive(Clone, Copy)]
struct Setup {
    attack: u8,
    decay: u8,
    key_scale: u8,
    sustain_level: u8,
    release: u8,
    total_level: u8,
    block: u8,
}

const BASE: Setup = Setup {
    attack: 31,
    decay: 8,
    key_scale: 0,
    sustain_level: 8,
    release: 8,
    total_level: 0,
    block: 4,
};

fn program(ym: &mut Ym2612, setup: Setup) {
    let bank = YmBank::One;
    for operator in 0..4_u8 {
        let offset = operator * 4;
        write_register(ym, bank, 0x30 + offset, 0x01); // DT=0, MUL=1
        write_register(ym, bank, 0x40 + offset, setup.total_level);
        write_register(
            ym,
            bank,
            0x50 + offset,
            (setup.key_scale << 6) | setup.attack,
        );
        write_register(ym, bank, 0x60 + offset, setup.decay);
        write_register(ym, bank, 0x70 + offset, 0x00);
        write_register(
            ym,
            bank,
            0x80 + offset,
            (setup.sustain_level << 4) | setup.release,
        );
    }
    write_register(ym, bank, 0xB0, 0x00); // algorithm 0, feedback 0
    write_register(ym, bank, 0xB4, 0xC0); // both stereo bits
    let channel = 0_u8;
    write_register(ym, bank, 0xA0 + channel, 0x69);
    write_register(ym, bank, 0xA4 + channel, (setup.block << 3) | 0x02);
}

fn key(ym: &mut Ym2612, on: bool) {
    let value = if on { 0xF0 } else { 0x00 };
    write_register(ym, YmBank::One, 0x28, value);
}

fn view(ym: &Ym2612) -> Result<EnvelopeView, Box<dyn Error>> {
    match ym.envelope(0, 0) {
        Some(view) => Ok(view),
        None => Err("operator envelope missing".into()),
    }
}

#[test]
fn attack_reaches_maximum_and_moves_to_decay() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program(&mut ym, BASE);
    key(&mut ym, true);
    let initial = view(&ym)?;
    assert_eq!(initial.state, EgState::Attack);
    assert_eq!(initial.attenuation, 0x03FF, "key-on starts from silence");

    ym.sample();
    let after_one = view(&ym)?;
    assert_eq!(after_one.state, EgState::Attack);
    assert!(after_one.attenuation < 0x03FF);

    let mut samples = 0;
    while view(&ym)?.state == EgState::Attack {
        ym.sample();
        samples += 1;
        assert!(samples < 64, "attack must finish");
    }
    let peaked = view(&ym)?;
    assert_eq!(peaked.state, EgState::Decay);
    assert_eq!(peaked.attenuation, 0, "the attack reaches maximum level");

    let mut samples = 0;
    while view(&ym)?.state == EgState::Decay {
        ym.sample();
        samples += 1;
        assert!(samples < 1024, "decay must reach the sustain level");
    }
    let sustain = view(&ym)?;
    assert_eq!(sustain.state, EgState::Sustain);
    assert_eq!(sustain.attenuation, i16::from(BASE.sustain_level) * 32);
    Ok(())
}

#[test]
fn key_off_enters_release() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program(&mut ym, BASE);
    key(&mut ym, true);
    while view(&ym)?.state != EgState::Sustain {
        ym.sample();
    }
    let sustained = view(&ym)?;
    key(&mut ym, false);
    assert_eq!(view(&ym)?.state, EgState::Release);
    assert_eq!(view(&ym)?.attenuation, sustained.attenuation);

    let mut samples = 0;
    while view(&ym)?.state != EgState::Off {
        ym.sample();
        samples += 1;
        assert!(samples < 1024, "release must reach silence");
    }
    assert_eq!(view(&ym)?.attenuation, 0x03FF);
    Ok(())
}

#[test]
fn rate_scaling_changes_counter_speed() -> Result<(), Box<dyn Error>> {
    let mut slow = Ym2612::new();
    program(&mut slow, Setup { attack: 6, ..BASE });
    key(&mut slow, true);
    slow.sample();

    let mut fast = Ym2612::new();
    program(&mut fast, Setup { attack: 12, ..BASE });
    key(&mut fast, true);
    fast.sample();

    let slow_view = view(&slow)?;
    let fast_view = view(&fast)?;
    assert!(
        slow_view.attenuation > fast_view.attenuation,
        "higher AR advances faster"
    );
    Ok(())
}

#[test]
fn key_scaling_speeds_higher_blocks() -> Result<(), Box<dyn Error>> {
    let mut unscaled = Ym2612::new();
    program(
        &mut unscaled,
        Setup {
            attack: 8,
            key_scale: 0,
            block: 7,
            ..BASE
        },
    );
    key(&mut unscaled, true);
    unscaled.sample();

    let mut scaled = Ym2612::new();
    program(
        &mut scaled,
        Setup {
            attack: 8,
            key_scale: 3,
            block: 7,
            ..BASE
        },
    );
    key(&mut scaled, true);
    scaled.sample();

    assert!(
        view(&scaled)?.attenuation < view(&unscaled)?.attenuation,
        "key scaling with KS=3 advances faster at the same block"
    );
    Ok(())
}

#[test]
fn release_starts_from_the_current_attenuation() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program(&mut ym, Setup { attack: 6, ..BASE });
    key(&mut ym, true);
    ym.sample();
    let before = view(&ym)?;
    assert_eq!(before.state, EgState::Attack);
    assert!(before.attenuation > 0 && before.attenuation < 0x03FF);

    key(&mut ym, false);
    assert_eq!(view(&ym)?.state, EgState::Release);
    assert_eq!(view(&ym)?.attenuation, before.attenuation);
    ym.sample();
    assert!(view(&ym)?.attenuation > before.attenuation);
    Ok(())
}

#[test]
fn attack_rate_zero_does_not_move() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program(&mut ym, Setup { attack: 0, ..BASE });
    key(&mut ym, true);
    for _ in 0..16 {
        ym.sample();
    }
    let view = view(&ym)?;
    assert_eq!(view.state, EgState::Attack);
    assert_eq!(view.attenuation, 0x03FF);
    Ok(())
}

#[test]
fn sustain_level_holds_its_attenuation() -> Result<(), Box<dyn Error>> {
    let mut ym = Ym2612::new();
    program(&mut ym, BASE);
    key(&mut ym, true);
    while view(&ym)?.state != EgState::Sustain {
        ym.sample();
    }
    let sustained = view(&ym)?;
    for _ in 0..128 {
        ym.sample();
    }
    assert_eq!(view(&ym)?, sustained, "the sustain phase holds the level");
    Ok(())
}
