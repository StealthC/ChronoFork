//! SN76489-compatible programmable sound generator.
//!
//! The Mega Drive's PSG is a write-only SN76489-compatible chip with three
//! square-wave tone channels, one noise channel, sixteen attenuation levels
//! (roughly 2 dB per step, level 15 silent) and ten-bit tone dividers. This
//! module models it as deterministic integer state: there is no floating point,
//! no host audio device and no wall clock in the sampling path.
//!
//! # Time base
//!
//! One PSG sample is one tick of the chip's internal sample clock, the Z80
//! clock divided by sixteen, which is [`MASTER_CYCLES_PER_SAMPLE`] master
//! cycles. [`Psg::sample`] advances the tone and noise counters by exactly one
//! tick, so a caller that invokes it once per canonical period reproduces the
//! documented PSG rate (`MCLK / 15 / 16`); the mixer is that caller.
//!
//! # Register protocol
//!
//! Writes follow the documented latch/data protocol. A byte with bit 7 set is a
//! latch byte: bits 6-5 select the channel (0-2 are tone, 3 is noise) and bit 4
//! selects the attenuation register instead of the tone/noise register. For a
//! tone channel the latch byte's low nibble is divider bits 0-3; a following
//! data byte (bit 7 clear) carries divider bits 4-9. For the noise channel the
//! latch byte's low three bits select the feedback and the shift rate.
//!
//! # Limits
//!
//! The attenuation table is a monotonic integer approximation of the documented
//! 2 dB steps; the absolute output scale is this model's choice, not a measured
//! hardware level. The PSG is write-only on hardware and the Mega Drive bus is
//! not driven by it, so [`Psg::read`] returns the documented open value
//! [`OPEN_BUS_VALUE`]. The 68000-side `0xC00011` PSG window remains out of scope
//! for this phase; the Z80 window `0x7F00..=0x7F03`/`0x7F11` reaches the chip.

use crate::cycles::MASTER_CYCLES_PER_Z80_T_STATE;

/// Number of square-wave tone channels.
pub const TONE_CHANNELS: usize = 3;

/// Index of the noise channel.
pub const NOISE_CHANNEL: usize = 3;

/// Total number of channels, tone then noise.
pub const CHANNELS: usize = 4;

/// Number of attenuation levels per channel, including silence.
pub const ATTENUATION_LEVELS: u8 = 16;

/// Width of a tone divider in bits.
pub const DIVIDER_BITS: u32 = 10;

/// Mask selecting a ten-bit tone divider.
pub const DIVIDER_MASK: u16 = (1_u16 << DIVIDER_BITS) - 1;

/// PSG clock divisor against the Z80 T-state clock.
pub const PSG_CLOCK_DIVIDER: u64 = 16;

/// Master cycles in one PSG sample tick.
///
/// The PSG runs at `MCLK / 15 / 16`, so one internal tick is fifteen Z80
/// T-states, i.e. this many master cycles.
pub const MASTER_CYCLES_PER_SAMPLE: u64 = MASTER_CYCLES_PER_Z80_T_STATE * PSG_CLOCK_DIVIDER;

/// Width of the maximal-length noise shift register.
pub const NOISE_SHIFT_BITS: u32 = 15;

/// Reset value of the noise shift register.
pub const LFSR_RESET_VALUE: u16 = 1_u16 << (NOISE_SHIFT_BITS - 1);

/// Internal sample-clock divisors selected by noise control bits 1-0.
pub const NOISE_CLOCK_DIVIDERS: [u16; 3] = [512, 1024, 2048];

/// Noise control value that clocks the shift register from tone channel 2.
pub const NOISE_TONE2_SELECT: u8 = 3;

/// Value returned by a PSG read.
///
/// The chip is write-only and does not drive the data bus, so the Mega Drive
/// open bus reads high; tests pin the value.
pub const OPEN_BUS_VALUE: u8 = 0xFF;

/// Full-volume amplitude of one channel.
///
/// Four channels at full volume sum to twice this value, so the mixed sample
/// stays inside `i32` before the final `i16` saturation. The absolute level is
/// this model's scale, not a hardware measurement.
pub const MAX_CHANNEL_AMPLITUDE: i16 = 8192;

/// Attenuation level to linear amplitude.
///
/// Level 0 is [`MAX_CHANNEL_AMPLITUDE`], each following step loses about 2 dB
/// (the documented SN76489 step) through integer arithmetic, and level 15 is
/// silent.
const ATTENUATION_TABLE: [i16; ATTENUATION_LEVELS as usize] = build_attenuation_table();

const fn build_attenuation_table() -> [i16; ATTENUATION_LEVELS as usize] {
    let mut table = [0_i16; ATTENUATION_LEVELS as usize];
    let mut level = 0;
    let mut amplitude = MAX_CHANNEL_AMPLITUDE as i32;
    while level < ATTENUATION_LEVELS as usize - 1 {
        table[level] = amplitude as i16;
        amplitude = amplitude * 794 / 1000;
        level += 1;
    }
    table
}

/// Deterministic SN76489-compatible PSG state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Psg {
    /// Last latch byte; a data write targets the channel and register it names.
    latch: u8,
    /// Ten-bit tone dividers.
    tone_divider: [u16; TONE_CHANNELS],
    /// Per-channel down-counters, one tick per [`Psg::sample`] call.
    tone_counter: [u16; TONE_CHANNELS],
    /// Square-wave levels.
    tone_output: [bool; TONE_CHANNELS],
    /// Four-bit attenuation per channel (0 is loudest, 15 is silent).
    attenuation: [u8; CHANNELS],
    /// Noise feedback/rate control (bit 2 feedback, bits 1-0 rate).
    noise_control: u8,
    /// Noise down-counter in internal ticks.
    noise_counter: u16,
    /// Fifteen-bit noise shift register.
    noise_shift: u16,
}

impl Default for Psg {
    fn default() -> Self {
        Self::new()
    }
}

impl Psg {
    /// Creates a PSG in its reset state: every channel silent, every divider
    /// zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            latch: 0,
            tone_divider: [0; TONE_CHANNELS],
            tone_counter: [tone_period(0); TONE_CHANNELS],
            tone_output: [false; TONE_CHANNELS],
            attenuation: [ATTENUATION_LEVELS - 1; CHANNELS],
            noise_control: 0,
            noise_counter: NOISE_CLOCK_DIVIDERS[0],
            noise_shift: LFSR_RESET_VALUE,
        }
    }

    /// Writes one byte through the latch/data protocol.
    ///
    /// Every byte value is valid; see the module documentation for the bit
    /// layout. A data byte before any latch byte targets tone channel 0.
    pub fn write(&mut self, value: u8) {
        let latched = value & 0x80 != 0;
        if latched {
            self.latch = value;
        }
        let channel = usize::from((self.latch >> 5) & 0x03);
        if self.latch & 0x10 != 0 {
            self.attenuation[channel] = value & 0x0F;
        } else if channel == NOISE_CHANNEL {
            self.write_noise_control(value & 0x07);
        } else if latched {
            self.tone_divider[channel] =
                (self.tone_divider[channel] & 0x3F0) | u16::from(value & 0x0F);
            self.tone_counter[channel] = tone_period(self.tone_divider[channel]);
        } else {
            self.tone_divider[channel] =
                (self.tone_divider[channel] & 0x00F) | (u16::from(value & 0x3F) << 4);
            self.tone_counter[channel] = tone_period(self.tone_divider[channel]);
        }
    }

    /// Reads the PSG.
    ///
    /// The chip has no readable register; a read returns the documented open
    /// value [`OPEN_BUS_VALUE`].
    #[must_use]
    pub const fn read(&self) -> u8 {
        OPEN_BUS_VALUE
    }

    /// Generates one sample tick and returns the mixed channel sum.
    ///
    /// Callers must invoke this exactly once per [`MASTER_CYCLES_PER_SAMPLE`]
    /// master cycles; the mixer does. A tone divider write restarts that
    /// channel's half-period counter, so the first tick after the write completes
    /// a partial half; in the steady state each half lasts `divider` ticks.
    pub fn sample(&mut self) -> i16 {
        let mut tone_toggled = [false; TONE_CHANNELS];
        for channel in 0..TONE_CHANNELS {
            self.tone_counter[channel] -= 1;
            if self.tone_counter[channel] == 0 {
                self.tone_output[channel] = !self.tone_output[channel];
                self.tone_counter[channel] = tone_period(self.tone_divider[channel]);
                tone_toggled[channel] = true;
            }
        }
        let shift_noise = match self.noise_control & 0x03 {
            NOISE_TONE2_SELECT => tone_toggled[2],
            select => {
                self.noise_counter -= 1;
                if self.noise_counter == 0 {
                    self.noise_counter = NOISE_CLOCK_DIVIDERS[usize::from(select)];
                    true
                } else {
                    false
                }
            }
        };
        if shift_noise {
            self.shift_noise();
        }
        let mut mixed = 0_i32;
        for channel in 0..TONE_CHANNELS {
            let amplitude = i32::from(ATTENUATION_TABLE[usize::from(self.attenuation[channel])]);
            mixed += if self.tone_output[channel] {
                amplitude
            } else {
                -amplitude
            };
        }
        let noise = i32::from(ATTENUATION_TABLE[usize::from(self.attenuation[NOISE_CHANNEL])]);
        mixed += if self.noise_shift & 1 != 0 {
            noise
        } else {
            -noise
        };
        mixed.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
    }

    /// Returns a tone channel's ten-bit divider, or `None` for the noise channel.
    #[must_use]
    pub fn tone_divider(&self, channel: usize) -> Option<u16> {
        self.tone_divider.get(channel).copied()
    }

    /// Returns a channel's four-bit attenuation, or `None` past the last channel.
    #[must_use]
    pub fn attenuation(&self, channel: usize) -> Option<u8> {
        self.attenuation.get(channel).copied()
    }

    /// Returns the current noise shift-register value.
    #[must_use]
    pub const fn noise_lfsr(&self) -> u16 {
        self.noise_shift
    }

    /// Latches a noise feedback/rate value and restarts the sequence, which is
    /// what the documented noise register write does.
    fn write_noise_control(&mut self, control: u8) {
        self.noise_control = control;
        self.noise_shift = LFSR_RESET_VALUE;
        self.noise_counter = match control & 0x03 {
            0 => NOISE_CLOCK_DIVIDERS[0],
            1 => NOISE_CLOCK_DIVIDERS[1],
            2 => NOISE_CLOCK_DIVIDERS[2],
            _ => NOISE_CLOCK_DIVIDERS[0],
        };
    }

    /// Shifts the noise register once.
    ///
    /// White noise feeds back the exclusive-or of bits 0 and 1, which gives the
    /// maximal-length 15-bit sequence; periodic noise feeds back bit 0 alone.
    fn shift_noise(&mut self) {
        let feedback = if self.noise_control & 0x04 != 0 {
            (self.noise_shift ^ (self.noise_shift >> 1)) & 1
        } else {
            self.noise_shift & 1
        };
        self.noise_shift = (self.noise_shift >> 1) | (feedback << (NOISE_SHIFT_BITS - 1));
    }
}

/// Returns the down-counter period for a ten-bit divider.
///
/// A zero divider counts as one full ten-bit period; this is the documented
/// behaviour that keeps the lowest tone available instead of stalling.
const fn tone_period(divider: u16) -> u16 {
    if divider == 0 {
        1_u16 << DIVIDER_BITS
    } else {
        divider
    }
}
