#![allow(clippy::expect_used)]

//! Canonical audio capture, the Z80 inside `Backend::step`, and payload v6.
//!
//! The synthetic 68000 program below installs a tiny Z80 program in Z80 RAM
//! while the reset line is asserted, then releases the reset and requests the
//! Z80 bus. From that instruction on, ordinary 68000 steps run the Z80 in their
//! master-cycle window; the Z80 programs a PSG tone and the channel-6 DAC, and
//! the mixer turns both devices into 48 kHz stereo chunks.

use std::error::Error;

use chronofork_backend_api::{Backend, BackendError, BackendFactory, BackendSnapshot, MediaSource};
use chronofork_core::{InputFrame, StateHash, hash_bytes};
use chronofork_megadrive::backend::{CONFIG_MAGIC, PAYLOAD_MAGIC, PAYLOAD_VERSION, STATE_MAGIC};
use chronofork_megadrive::{MegadriveBackend, MegadriveFactory, demo_rom};

/// Z80 program installed at Z80 RAM address 0.
///
/// It programs PSG tone channel 0 at divider 1 and full volume, enables the
/// YM2612 channel-6 DAC at its maximum, then increments the accumulator forever.
fn z80_program() -> Vec<u8> {
    let mut program = Vec::new();
    // LD A,<latch>; LD (0x7F11),A programs the PSG.
    for value in [0x90_u8, 0x81, 0x00] {
        program.extend_from_slice(&[0x3E, value, 0x32, 0x11, 0x7F]);
    }
    // LD A,<register>; LD (0x4000),A; LD A,<data>; LD (0x4001),A programs the
    // YM2612 DAC enable, DAC value and channel-6 stereo bits.
    for (register, data) in [(0x2B_u8, 0x80_u8), (0x2A, 0xFF), (0xB6, 0xC0)] {
        program.extend_from_slice(&[0x3E, register, 0x32, 0x00, 0x40]);
        program.extend_from_slice(&[0x3E, data, 0x32, 0x01, 0x40]);
    }
    program.extend_from_slice(&[0x3C, 0x18, 0xFD]); // INC A; JR back to INC A
    program
}

/// Emits `MOVE.B #value,(address).L` at `offset`.
fn emit_move_byte(rom: &mut [u8], offset: &mut usize, value: u8, address: u32) {
    rom[*offset..*offset + 2].copy_from_slice(&0x13FC_u16.to_be_bytes());
    rom[*offset + 2..*offset + 4].copy_from_slice(&u16::from(value).to_be_bytes());
    rom[*offset + 4..*offset + 8].copy_from_slice(&address.to_be_bytes());
    *offset += 8;
}

/// Builds a synthetic ROM that installs [`z80_program`] and hands the bus over.
///
/// The 68000 asserts the bus request at `0xA11100` (hardware polarity: the
/// request stops the Z80 and gives the 68000 the Z80 window), writes every Z80
/// program byte through the `0xA00000` window while the Z80 is held in reset,
/// then clears the active-low reset bit at `0xA11200` and negates the request at
/// `0xA11100` so the Z80 owns the bus. Afterwards it spins on a `BRA.S` so every
/// step leaves the Z80 a small master-cycle window.
fn z80_step_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x1000];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x00FF_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    let mut offset = 0x200;
    emit_move_byte(&mut rom, &mut offset, 0x01, 0x00A1_1100); // assert the bus request
    for (index, byte) in z80_program().iter().enumerate() {
        let address = 0x00A0_0000 + u32::try_from(index).expect("program offset fits");
        emit_move_byte(&mut rom, &mut offset, *byte, address);
    }
    emit_move_byte(&mut rom, &mut offset, 0x01, 0x00A0_6000); // Z80 bank 0x100
    emit_move_byte(&mut rom, &mut offset, 0x01, 0x00A1_1200); // release Z80 reset
    emit_move_byte(&mut rom, &mut offset, 0x00, 0x00A1_1100); // release the bus to the Z80
    rom[offset..offset + 2].copy_from_slice(&[0x60, 0xFE]); // BRA.S -2
    assert!(chronofork_megadrive::test_support::stamp_header(
        &mut rom, "JUE"
    ));
    rom
}

fn z80_machine() -> MegadriveBackend {
    MegadriveFactory::new(z80_step_rom())
        .expect("factory")
        .create_machine()
        .expect("machine")
}

fn fresh_machine() -> MegadriveBackend {
    MegadriveFactory::new(z80_step_rom())
        .expect("factory")
        .create_machine()
        .expect("machine")
}

fn pcm_hash(samples: &[i16]) -> StateHash {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    hash_bytes(&bytes)
}

fn tampered(snapshot: &BackendSnapshot, magic: &[u8; 8], version: u16) -> BackendSnapshot {
    let mut payload = snapshot.payload().to_vec();
    payload[..8].copy_from_slice(magic);
    payload[8..10].copy_from_slice(&version.to_le_bytes());
    BackendSnapshot::new(
        snapshot.backend_id().clone(),
        snapshot.backend_version().to_owned(),
        snapshot.format_version(),
        snapshot.tick(),
        snapshot.config_hash(),
        snapshot.state_hash(),
        payload,
    )
}

#[test]
fn version_seven_identity_is_published_and_the_media_seam_is_advertised() {
    assert_eq!(PAYLOAD_VERSION, 7);
    assert_eq!(&PAYLOAD_MAGIC, b"CFMD68K7");
    assert_eq!(STATE_MAGIC, b"CFMDSTATE7");
    assert_eq!(CONFIG_MAGIC, b"CFMDCONF7");
    let factory = MegadriveFactory::new(demo_rom()).expect("factory");
    let capabilities = factory.descriptor().capabilities();
    assert!(capabilities.rendering, "frame production is implemented");
    assert!(
        capabilities.audio,
        "the 48 kHz audio media seam is implemented"
    );
}

#[test]
fn z80_program_progresses_during_ordinary_steps() -> Result<(), Box<dyn Error>> {
    let mut machine = z80_machine();
    let loop_start = z80_program().len() - 3;
    let loop_pc = u16::try_from(loop_start).expect("program fits the Z80 address space");
    // Power-on holds the Z80 in reset and asserts no bus request, so the Z80
    // stays put until the program releases the reset and negates the request.
    assert!(!machine.bus().arbitration().z80_runs());
    let mut saw_running = false;
    let mut entered_loop = false;
    for _ in 0..800 {
        machine.step(InputFrame::NEUTRAL)?;
        if machine.bus().arbitration().z80_runs() {
            saw_running = true;
        }
        if saw_running && machine.z80().pc() >= loop_pc {
            entered_loop = true;
            break;
        }
    }
    assert!(saw_running, "the 68000 program must release the Z80 bus");
    assert!(
        entered_loop,
        "the Z80 must execute its program during ordinary 68000 steps, pc={:#06X}",
        machine.z80().pc()
    );
    let before = machine.z80().a();
    for _ in 0..200 {
        machine.step(InputFrame::NEUTRAL)?;
    }
    let after = machine.z80().a();
    assert_ne!(
        after, before,
        "the Z80 INC A loop must keep advancing during ordinary steps"
    );
    Ok(())
}

#[test]
fn audio_chunks_are_48khz_stereo_and_bounded_to_one_per_take() -> Result<(), Box<dyn Error>> {
    let mut machine = z80_machine();
    let mut chunk = None;
    for _ in 0..4_000 {
        machine.step(InputFrame::NEUTRAL)?;
        if let Some(taken) = machine.take_audio_chunk() {
            chunk = Some(taken);
            break;
        }
    }
    let chunk = chunk.expect("audio must accumulate after enough steps");
    assert_eq!(chunk.sample_rate_hz, 48_000);
    assert_eq!(chunk.channels, 2);
    assert_eq!(
        chunk.samples.len(),
        2 * chronofork_megadrive::backend::AUDIO_CHUNK_FRAMES
    );
    assert!(
        chunk.samples.iter().any(|sample| *sample != 0),
        "the Z80-programmed PSG tone and YM DAC must reach the captured audio"
    );
    assert!(
        machine.take_audio_chunk().is_none(),
        "the backend holds one chunk per take"
    );
    Ok(())
}

#[test]
fn two_runs_produce_the_same_state_and_pcm() -> Result<(), Box<dyn Error>> {
    fn run() -> Result<(u64, StateHash, Vec<i16>), Box<dyn Error>> {
        let mut machine = z80_machine();
        let mut pcm = Vec::new();
        for _ in 0..1_500 {
            machine.step(InputFrame::NEUTRAL)?;
            if let Some(chunk) = machine.take_audio_chunk() {
                pcm.extend_from_slice(&chunk.samples);
            }
        }
        let inspection = machine.inspect()?;
        Ok((inspection.tick.raw(), inspection.state_hash, pcm))
    }
    let (tick_a, hash_a, pcm_a) = run()?;
    let (tick_b, hash_b, pcm_b) = run()?;
    assert_eq!(tick_a, tick_b);
    assert_eq!(hash_a, hash_b);
    assert!(!pcm_a.is_empty(), "the run must capture audio");
    assert_eq!(pcm_hash(&pcm_a), pcm_hash(&pcm_b));
    Ok(())
}

#[test]
fn v1_to_v6_payloads_are_rejected_with_typed_errors() -> Result<(), Box<dyn Error>> {
    let mut machine = z80_machine();
    for _ in 0..800 {
        machine.step(InputFrame::NEUTRAL)?;
    }
    let snapshot = machine.snapshot()?;
    let before = machine.snapshot()?.payload().to_vec();
    // The version word alone selects the layout; a v1-v6 version word is
    // rejected by version even when the surrounding bytes are v7.
    for version in 1..=6_u16 {
        let candidate = tampered(&snapshot, &PAYLOAD_MAGIC, version);
        let error = machine
            .restore(&candidate)
            .expect_err("an old payload version is rejected");
        assert!(
            matches!(error, BackendError::InvalidSnapshot(_)),
            "typed rejection for v{version}: {error}"
        );
        assert!(
            error.to_string().contains(&format!("version {version}")),
            "reports the rejected version: {error}"
        );
        assert_eq!(
            machine.snapshot()?.payload(),
            before,
            "a rejected restore must not mutate the machine"
        );
    }
    // Old magics are rejected before any field is reinterpreted.
    for magic in [
        *b"CFMD68K1",
        *b"CFMD68K2",
        *b"CFMD68K3",
        *b"CFMD68K4",
        *b"CFMD68K5",
        *b"CFMD68K6",
    ] {
        let candidate = tampered(&snapshot, &magic, PAYLOAD_VERSION);
        let error = machine
            .restore(&candidate)
            .expect_err("old magic is rejected");
        assert!(matches!(error, BackendError::InvalidSnapshot(_)));
        assert_eq!(machine.snapshot()?.payload(), before);
    }
    Ok(())
}

#[test]
fn snapshot_round_trips_z80_ym_psg_and_mixer_phase() -> Result<(), Box<dyn Error>> {
    let mut original = z80_machine();
    for _ in 0..900 {
        original.step(InputFrame::NEUTRAL)?;
    }
    let snapshot = original.snapshot()?;
    let mut restored = fresh_machine();
    restored.restore(&snapshot)?;

    assert_eq!(
        restored.snapshot()?.payload(),
        snapshot.payload(),
        "the v6 payload must round-trip byte for byte"
    );
    assert_eq!(restored.snapshot()?.state_hash(), snapshot.state_hash());
    assert_eq!(
        restored.z80().snapshot_bytes(),
        original.z80().snapshot_bytes()
    );
    assert_eq!(restored.z80().pc(), original.z80().pc());
    assert_eq!(restored.z80().a(), original.z80().a());
    assert_eq!(
        restored.ym2612().snapshot_bytes(),
        original.ym2612().snapshot_bytes()
    );
    assert_eq!(
        restored.psg().snapshot_bytes(),
        original.psg().snapshot_bytes()
    );
    assert_eq!(
        restored.audio_mixer().snapshot_bytes(),
        original.audio_mixer().snapshot_bytes()
    );

    // Queued audio is derived output, not canonical state: a restore starts the
    // media queue empty, like the video frame queue.
    assert!(
        restored.take_audio_chunk().is_none(),
        "restore must start with an empty media queue"
    );

    // Continuing both machines must produce identical canonical state and an
    // identical mixer phase every step: the Z80 master-cycle carry, both device
    // clocks and the 48 kHz resamplers are restored exactly. The PCM samples
    // generated from here on are therefore identical; the frames buffered before
    // the snapshot are not replayed, exactly like a pending video frame.
    for step in 0..600 {
        original.step(InputFrame::NEUTRAL)?;
        restored.step(InputFrame::NEUTRAL)?;
        assert_eq!(
            original.inspect()?.state_hash,
            restored.inspect()?.state_hash,
            "state hash diverged at continuation step {step}"
        );
        assert_eq!(
            original.audio_mixer().snapshot_bytes(),
            restored.audio_mixer().snapshot_bytes(),
            "mixer phase diverged at continuation step {step}"
        );
        assert_eq!(
            original.z80().snapshot_bytes(),
            restored.z80().snapshot_bytes()
        );
        assert_eq!(
            original.ym2612().snapshot_bytes(),
            restored.ym2612().snapshot_bytes()
        );
        assert_eq!(
            original.psg().snapshot_bytes(),
            restored.psg().snapshot_bytes()
        );
    }
    Ok(())
}

#[test]
fn inspect_exposes_z80_arbitration_and_audio_pseudo_registers() -> Result<(), Box<dyn Error>> {
    let mut machine = z80_machine();
    for _ in 0..900 {
        machine.step(InputFrame::NEUTRAL)?;
    }
    let inspection = machine.inspect()?;
    for name in [
        "z80.af",
        "z80.bc",
        "z80.de",
        "z80.hl",
        "z80.ix",
        "z80.iy",
        "z80.sp",
        "z80.pc",
        "z80.i",
        "z80.r",
        "z80.im",
        "z80.iff1",
        "z80.iff2",
        "z80.halted",
        "z80.credit",
        "z80.bank",
        "arbitration.requested",
        "arbitration.reset_asserted",
        "arbitration.z80_runs",
        "ym.status",
        "ym.lfo_phase",
        "ym.address_latch0",
        "ym.address_latch1",
        "psg.latch",
        "psg.tone0",
        "psg.tone1",
        "psg.tone2",
        "psg.attenuation0",
        "psg.attenuation1",
        "psg.attenuation2",
        "psg.attenuation3",
        "psg.noise_control",
        "psg.noise_lfsr",
        "audio.pending_frames",
        "audio.dropped_frames",
        "audio.frame_accumulator",
    ] {
        assert!(
            inspection.registers.contains_key(name),
            "inspection is missing `{name}`"
        );
    }
    assert_eq!(inspection.registers["z80.iff1"], 0);
    assert_eq!(inspection.registers["arbitration.z80_runs"], 1);
    assert_eq!(
        inspection.registers["z80.bank"], 0x100,
        "the 68000 program selects Z80 bank 0x100"
    );
    assert_eq!(
        inspection.registers["psg.tone0"], 1,
        "the Z80 program sets the PSG divider"
    );
    Ok(())
}
