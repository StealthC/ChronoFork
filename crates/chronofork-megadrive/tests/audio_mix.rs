#![allow(clippy::expect_used)]

//! Integer mixer and exact 48 kHz resampler: output length, determinism,
//! centre-panned PSG, saturating mix and the rational phase accumulator.

use chronofork_core::{StateHash, hash_bytes};
use chronofork_megadrive::audio::mixer::{AudioMixer, MAX_PENDING_FRAMES, OUTPUT_SAMPLE_RATE_HZ};
use chronofork_megadrive::audio::resample::{RationalResampler, ResampleError};
use chronofork_megadrive::audio::{Psg, Ym2612, YmBank, YmPort};
use chronofork_megadrive::cycles::MASTER_CLOCK_HZ_NTSC;

/// Writes one YM2612 register through bank 0.
fn write_ym(ym: &mut Ym2612, address: u8, value: u8) {
    write_ym_bank(ym, YmBank::One, address, value);
}

/// Writes one YM2612 register through the selected port pair.
fn write_ym_bank(ym: &mut Ym2612, bank: YmBank, address: u8, value: u8) {
    ym.write_bank(bank, YmPort::Address, address);
    ym.write_bank(bank, YmPort::Data, value);
}

/// Programs channel 1 with a simple four-operator FM voice.
fn programmed_ym(fnum_low: u8, fnum_high: u8) -> Ym2612 {
    let mut ym = Ym2612::new();
    for operator in 0..4_u8 {
        write_ym(&mut ym, 0x30 + operator * 4, 0x01); // multiple 1
        write_ym(&mut ym, 0x40 + operator * 4, 0x00); // loudest total level
        write_ym(&mut ym, 0x50 + operator * 4, 0x1F); // fastest attack
        write_ym(&mut ym, 0x80 + operator * 4, 0x00); // sustain level 0
    }
    write_ym(&mut ym, 0xA0, fnum_low);
    write_ym(&mut ym, 0xA4, fnum_high);
    write_ym(&mut ym, 0xB0, 0x07); // algorithm 7, no feedback
    write_ym(&mut ym, 0xB4, 0xC0); // left and right enabled
    write_ym(&mut ym, 0x28, 0xF0); // key on all four operators
    ym
}

/// Programs tone channel 0 at a low divider and full volume.
fn programmed_psg() -> Psg {
    let mut psg = Psg::new();
    psg.write(0x81); // latch tone 0, divider low nibble 1
    psg.write(0x00); // divider 1
    psg.write(0x90); // channel 0 volume 0
    psg
}

fn pcm_hash(chunk: &[i16]) -> StateHash {
    let mut bytes = Vec::with_capacity(chunk.len() * 2);
    for sample in chunk {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    hash_bytes(&bytes)
}

/// Runs one second of emulated time and returns the interleaved PCM.
fn one_second(ym: &mut Ym2612, psg: &mut Psg) -> Vec<i16> {
    let mut mixer = AudioMixer::new();
    mixer.advance_master(MASTER_CLOCK_HZ_NTSC, ym, psg);
    assert_eq!(
        mixer.pending_frames(),
        usize::try_from(OUTPUT_SAMPLE_RATE_HZ).expect("48 kHz fits usize")
    );
    mixer
        .take_chunk(usize::MAX)
        .expect("one second fills one chunk")
        .samples
}

#[test]
fn one_second_yields_exactly_48000_frames() {
    let mut mixer = AudioMixer::new();
    let mut ym = programmed_ym(0x20, 0x22);
    let mut psg = programmed_psg();
    mixer.advance_master(MASTER_CLOCK_HZ_NTSC, &mut ym, &mut psg);
    assert_eq!(
        mixer.pending_frames(),
        usize::try_from(OUTPUT_SAMPLE_RATE_HZ).expect("48 kHz fits usize"),
        "tolerance 0: one second must be exactly 48_000 frames"
    );
    let chunk = mixer.take_chunk(usize::MAX).expect("a filled chunk");
    assert_eq!(chunk.sample_rate_hz, OUTPUT_SAMPLE_RATE_HZ);
    assert_eq!(chunk.channels, 2);
    assert_eq!(
        chunk.samples.len(),
        2 * usize::try_from(OUTPUT_SAMPLE_RATE_HZ).expect("48 kHz fits usize")
    );
    assert!(chunk.samples.iter().any(|sample| *sample != 0));
    assert!(mixer.take_chunk(64).is_none(), "the queue holds one chunk");
}

#[test]
fn two_identical_runs_produce_identical_pcm_hashes() {
    let first = one_second(&mut programmed_ym(0x20, 0x22), &mut programmed_psg());
    let second = one_second(&mut programmed_ym(0x20, 0x22), &mut programmed_psg());
    assert_eq!(pcm_hash(&first), pcm_hash(&second));
    let reference = pcm_hash(&first);

    let detuned = one_second(&mut programmed_ym(0x21, 0x22), &mut programmed_psg());
    assert_ne!(
        pcm_hash(&detuned),
        reference,
        "a different FM frequency must change the PCM hash"
    );
}

#[test]
fn psg_is_panned_centre() {
    let pcm = one_second(&mut Ym2612::new(), &mut programmed_psg());
    for frame in pcm.chunks_exact(2) {
        assert_eq!(
            frame[0], frame[1],
            "PSG must be added equally to both sides"
        );
    }
    assert!(pcm.iter().any(|sample| *sample != 0));
}

#[test]
fn output_length_does_not_depend_on_the_advance_chunking() {
    let one_shot = one_second(&mut programmed_ym(0x20, 0x22), &mut programmed_psg());

    let mut mixer = AudioMixer::new();
    let mut ym = programmed_ym(0x20, 0x22);
    let mut psg = programmed_psg();
    let mut remaining = MASTER_CLOCK_HZ_NTSC;
    while remaining > 0 {
        let step = remaining.min(7);
        mixer.advance_master(step, &mut ym, &mut psg);
        remaining -= step;
    }
    assert_eq!(
        mixer.pending_frames(),
        usize::try_from(OUTPUT_SAMPLE_RATE_HZ).expect("48 kHz fits usize")
    );
    let chunked = mixer
        .take_chunk(usize::MAX)
        .expect("a filled chunk")
        .samples;
    assert_eq!(pcm_hash(&one_shot), pcm_hash(&chunked));
}

#[test]
fn pending_queue_is_bounded_and_drops_the_oldest_frames() {
    let mut mixer = AudioMixer::new();
    let mut ym = Ym2612::new();
    let mut psg = Psg::new();
    mixer.advance_master(2 * MASTER_CLOCK_HZ_NTSC, &mut ym, &mut psg);
    assert_eq!(mixer.pending_frames(), MAX_PENDING_FRAMES);
    assert_eq!(
        mixer.dropped_frames(),
        u64::from(OUTPUT_SAMPLE_RATE_HZ),
        "the second second overflows the one-second queue"
    );
    let chunk = mixer.take_chunk(MAX_PENDING_FRAMES).expect("a full queue");
    assert_eq!(chunk.samples.len(), 2 * MAX_PENDING_FRAMES);
}

#[test]
fn take_chunk_honours_the_requested_frame_limit() {
    let mut mixer = AudioMixer::new();
    let mut ym = Ym2612::new();
    let mut psg = Psg::new();
    mixer.advance_master(MASTER_CLOCK_HZ_NTSC / 100, &mut ym, &mut psg);
    let pending = mixer.pending_frames();
    assert!(pending > 0 && pending < usize::try_from(OUTPUT_SAMPLE_RATE_HZ).expect("48 kHz fits"));
    let first = mixer.take_chunk(10).expect("a partial chunk");
    assert_eq!(first.samples.len(), 20);
    assert_eq!(mixer.pending_frames(), pending - 10);
    assert!(
        mixer.take_chunk(0).is_none(),
        "a zero-frame request yields nothing"
    );
}

#[test]
fn rational_resampler_advances_an_exact_phase() {
    let mut resampler = RationalResampler::new(240).expect("a non-zero period");
    resampler.push(0);
    resampler.push(1000);
    assert_eq!(resampler.samples_due(120), 0);
    assert_eq!(
        resampler.advance(120),
        500,
        "linear interpolation at 120/240"
    );
    assert_eq!(resampler.phase(), (120, 240));
    assert_eq!(resampler.advance(120), 0);
    assert_eq!(resampler.phase(), (0, 240));
    assert_eq!(resampler.samples_due(241), 1);
    assert_eq!(
        resampler.advance(240),
        0,
        "the phase wraps to the previous sample"
    );
    assert_eq!(resampler.phase(), (0, 240));
    assert!(matches!(
        RationalResampler::new(0),
        Err(ResampleError::ZeroInputPeriod)
    ));
}

#[test]
fn rational_resampler_holds_a_constant_input() {
    let mut resampler = RationalResampler::new(240).expect("a non-zero period");
    resampler.push(-1234);
    for _ in 0..16 {
        assert_eq!(resampler.advance(37), -1234);
    }
}

#[test]
fn mixer_accepts_zero_cycles_without_output() {
    let mut mixer = AudioMixer::new();
    let mut ym = Ym2612::new();
    let mut psg = Psg::new();
    mixer.advance_master(0, &mut ym, &mut psg);
    assert_eq!(mixer.pending_frames(), 0);
    assert!(mixer.take_chunk(1).is_none());
}

/// Verifies the mixer's saturating integer add without needing a loud device
/// programme: the helper is the single place where YM and PSG meet.
#[test]
fn mixer_saturates_instead_of_wrapping() {
    // Two seconds of the loudest DAC value plus a full-volume PSG tone: the
    // mix must clamp to the i16 range instead of wrapping.
    let mut ym = Ym2612::new();
    write_ym(&mut ym, 0x2B, 0x80); // enable the channel-6 DAC
    write_ym(&mut ym, 0x2A, 0xFF); // DAC at its maximum
    write_ym_bank(&mut ym, YmBank::Two, 0xB6, 0xC0); // channel 6 left and right enabled
    let pcm = one_second(&mut ym, &mut programmed_psg());
    assert!(pcm.contains(&i16::MAX), "the positive rail must saturate");
}
