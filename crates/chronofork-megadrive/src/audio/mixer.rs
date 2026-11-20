//! Deterministic integer mixer and the exact 48 kHz output clock.
//!
//! The YM2612 and the PSG run at rational fractions of the master clock. This
//! module advances their sample clocks by elapsed master cycles, resamples both
//! to 48 kHz with [`RationalResampler`], mixes them with saturating integer
//! arithmetic into signed 16-bit stereo and queues finished frames for
//! [`AudioMixer::take_chunk`].
//!
//! # Rates
//!
//! The YM2612 produces one sample every [`YM_MASTER_CYCLES_PER_SAMPLE`] master
//! cycles and the PSG one every [`PSG_MASTER_CYCLES_PER_SAMPLE`]. Output frames
//! are emitted exactly when `master cycles * 48_000` reaches the configured
//! master-clock frequency in hertz, so one second of emulated time contains
//! exactly 48 000 stereo frames at that clock. No floating point, host clock or
//! audio device enters the path.
//!
//! # Pan and mix
//!
//! The PSG is a mono chip and is panned centre: the same mixed value is added to
//! both sides. The YM2612 output already carries its documented left/right
//! channel-enable bits (the channel-6 DAC convention uses those same bits), so
//! its two samples are resampled independently. YM and PSG meet in exactly one
//! place, [`saturating_mix`], which clamps to the `i16` range instead of
//! wrapping.
//!
//! Each device's interpolated output lags its newest sample by at most one
//! device period (1008 master cycles for the YM2612, 240 for the PSG). The two
//! delays differ by at most 768 master cycles (about 14 microseconds of emulated
//! time); that is the documented linear-interpolation latency, not host
//! latency.
//!
//! # Queue
//!
//! Finished frames are queued up to [`MAX_PENDING_FRAMES`] (one second at
//! 48 kHz). A single [`AudioMixer::advance_master`] call that generates more than
//! that discards the oldest frames and counts them in
//! [`AudioMixer::dropped_frames`]; the output stays deterministic. The queue is
//! derived output, never canonical state.

use std::collections::VecDeque;

use chronofork_backend_api::AudioChunk;

use crate::audio::psg::MASTER_CYCLES_PER_SAMPLE as PSG_MASTER_CYCLES_PER_SAMPLE;
use crate::audio::psg::Psg;
use crate::audio::resample::RationalResampler;
use crate::audio::ym2612::MASTER_CYCLES_PER_SAMPLE as YM_MASTER_CYCLES_PER_SAMPLE;
use crate::audio::ym2612::Ym2612;
use crate::cycles::{MASTER_CLOCK_HZ_NTSC, MASTER_CLOCK_HZ_PAL};
use crate::interrupts::VideoStandard;

/// Frames per second of the mixer output.
pub const OUTPUT_SAMPLE_RATE_HZ: u32 = 48_000;

/// Interleaved channels of the mixer output.
pub const OUTPUT_CHANNELS: u8 = 2;

/// Frames the pending queue holds before it drops the oldest ones.
pub const MAX_PENDING_FRAMES: usize = OUTPUT_SAMPLE_RATE_HZ as usize;

/// YM2612 and PSG integer mixer with an exact 48 kHz output clock.
#[derive(Clone, Debug)]
pub struct AudioMixer {
    /// Master clock the 48 kHz ratio is defined against.
    master_clock_hz: u64,
    /// Accumulated `master cycles * 48_000` since the last emitted frame.
    frame_accumulator: u64,
    /// YM2612 left channel resampler.
    ym_left: RationalResampler,
    /// YM2612 right channel resampler.
    ym_right: RationalResampler,
    /// Centre-panned PSG resampler.
    psg: RationalResampler,
    /// Interleaved pending stereo frames.
    pending: VecDeque<i16>,
    /// Frames discarded because the queue was full.
    dropped_frames: u64,
}

impl Default for AudioMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioMixer {
    /// Creates a mixer for the NTSC master clock.
    #[must_use]
    pub fn new() -> Self {
        Self::for_video_standard(VideoStandard::Ntsc)
    }

    /// Creates a mixer whose 48 kHz reference is the given video standard's
    /// master clock.
    ///
    /// One second of emulated time is one second of that clock, so the PAL and
    /// NTSC mixers both produce exactly 48 000 frames per emulated second.
    #[must_use]
    pub fn for_video_standard(video: VideoStandard) -> Self {
        let master_clock_hz = match video {
            VideoStandard::Ntsc => MASTER_CLOCK_HZ_NTSC,
            VideoStandard::Pal => MASTER_CLOCK_HZ_PAL,
        };
        Self {
            master_clock_hz,
            frame_accumulator: 0,
            ym_left: RationalResampler::with_period(YM_MASTER_CYCLES_PER_SAMPLE),
            ym_right: RationalResampler::with_period(YM_MASTER_CYCLES_PER_SAMPLE),
            psg: RationalResampler::with_period(PSG_MASTER_CYCLES_PER_SAMPLE),
            pending: VecDeque::new(),
            dropped_frames: 0,
        }
    }

    /// Advances emulated time by `cycles` master cycles.
    ///
    /// Generates every YM2612 and PSG sample that becomes due in that window,
    /// mixes the interpolated result and queues output frames on the exact
    /// 48 kHz clock. Chunking does not change the result: the caller may advance
    /// in small or large steps.
    pub fn advance_master(&mut self, cycles: u64, ym: &mut Ym2612, psg: &mut Psg) {
        let mut remaining = cycles;
        while remaining > 0 {
            let step = remaining.min(self.cycles_to_next_frame());
            let (left, right) = self.advance_devices(step, ym, psg);
            remaining -= step;
            if self.advance_frame_clock(step) {
                self.push_frame(left, right);
            }
        }
    }

    /// Takes up to `max_frames` queued stereo frames.
    ///
    /// Returns `None` when the queue is empty or `max_frames` is zero.
    pub fn take_chunk(&mut self, max_frames: usize) -> Option<AudioChunk> {
        let available = self.pending_frames();
        let frames = max_frames.min(available);
        if frames == 0 {
            return None;
        }
        let samples = self.pending.drain(..frames * 2).collect();
        // Frames are queued in stereo pairs, so the sample count is aligned by
        // construction; `AudioChunk`'s validation cannot fail here.
        Some(AudioChunk {
            samples,
            sample_rate_hz: OUTPUT_SAMPLE_RATE_HZ,
            channels: OUTPUT_CHANNELS,
        })
    }

    /// Returns the number of fully queued stereo frames.
    #[must_use]
    pub fn pending_frames(&self) -> usize {
        self.pending.len() / 2
    }

    /// Returns how many frames were discarded because the queue was full.
    #[must_use]
    pub const fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    /// Returns the whole master cycles needed to reach the next output frame.
    fn cycles_to_next_frame(&self) -> u64 {
        let missing = self.master_clock_hz - self.frame_accumulator;
        missing.div_ceil(u64::from(OUTPUT_SAMPLE_RATE_HZ))
    }

    /// Advances the 48 kHz clock by `cycles`; returns whether a frame is due.
    fn advance_frame_clock(&mut self, cycles: u64) -> bool {
        self.frame_accumulator += cycles * u64::from(OUTPUT_SAMPLE_RATE_HZ);
        if self.frame_accumulator >= self.master_clock_hz {
            self.frame_accumulator -= self.master_clock_hz;
            true
        } else {
            false
        }
    }

    /// Generates due device samples and returns the mixed interpolated frame.
    fn advance_devices(&mut self, cycles: u64, ym: &mut Ym2612, psg: &mut Psg) -> (i16, i16) {
        let due = self.ym_left.samples_due(cycles);
        for _ in 0..due {
            let (left, right) = ym.sample();
            self.ym_left.push(left);
            self.ym_right.push(right);
        }
        let ym_left = self.ym_left.advance(cycles);
        let ym_right = self.ym_right.advance(cycles);
        let due = self.psg.samples_due(cycles);
        for _ in 0..due {
            self.psg.push(psg.sample());
        }
        let psg = self.psg.advance(cycles);
        (saturating_mix(ym_left, psg), saturating_mix(ym_right, psg))
    }

    /// Queues one stereo frame, dropping the oldest pair when the queue is full.
    fn push_frame(&mut self, left: i16, right: i16) {
        if self.pending.len() >= MAX_PENDING_FRAMES * 2 {
            self.pending.pop_front();
            self.pending.pop_front();
            self.dropped_frames += 1;
        }
        self.pending.push_back(left);
        self.pending.push_back(right);
    }
}

/// Mixes one YM2612 sample and one centre-panned PSG sample.
///
/// This is the only place where the two devices meet: the integer sum saturates
/// at the `i16` range instead of wrapping.
fn saturating_mix(ym: i16, psg: i16) -> i16 {
    (i32::from(ym) + i32::from(psg)).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::saturating_mix;

    #[test]
    fn saturating_mix_clamps_instead_of_wrapping() {
        assert_eq!(saturating_mix(i16::MAX, 1), i16::MAX);
        assert_eq!(saturating_mix(i16::MIN, -1), i16::MIN);
        assert_eq!(saturating_mix(100, -40), 60);
        assert_eq!(saturating_mix(-100, 40), -60);
    }
}
