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
//! [`OPEN_BUS_VALUE`]. Both windows reach this one chip: the 68000 window
//! `0xC00010..=0xC00017` and the Z80 window `0x7F00..=0x7F03`/`0x7F11` plus its
//! mirror `0x7F10..=0x7F17`, so the latch/data sequence and the byte a read
//! returns are identical on both sides.
//!
//! The read value itself is a documented modelling choice, not a measured
//! hardware level: the pinned oracle's PSG inherits `Device:356`'s `read`
//! without filling the data, and there is no uniform executable proof of
//! `0xFF` for it. This slice keeps the single deterministic
//! [`OPEN_BUS_VALUE`] on *both* sides rather than letting them disagree.

use chronofork_core::codec::{Reader, Writer};

use crate::audio::{AudioSnapshotError, decode_flag, snapshot_error};
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

/// Internal sample-clock periods of the three fixed noise rates, in ticks.
///
/// The documented SN76489 shift periods are the chip-clock divisions `N/512`,
/// `N/1024` and `N/2048`; one internal sample tick is [`PSG_CLOCK_DIVIDER`]
/// chip clocks, so those periods are **32, 64 and 128 ticks**. At the NTSC
/// master clock the fastest selector therefore shifts at
/// `53 693 175 / (32 * 240)` ≈ 6991 Hz, the documented SN76489 noise frequency
/// for that selector. The three assertions below pin that derivation at compile
/// time so the constants cannot drift away from the clock.
pub const NOISE_CLOCK_DIVIDERS: [u16; 3] = [32, 64, 128];

const _: () = assert!(NOISE_CLOCK_DIVIDERS[0] as u64 * PSG_CLOCK_DIVIDER == 512);
const _: () = assert!(NOISE_CLOCK_DIVIDERS[1] as u64 * PSG_CLOCK_DIVIDER == 1024);
const _: () = assert!(NOISE_CLOCK_DIVIDERS[2] as u64 * PSG_CLOCK_DIVIDER == 2048);

/// Noise control value that clocks the shift register from tone channel 2.
pub const NOISE_TONE2_SELECT: u8 = 3;

/// Value returned by a PSG read.
///
/// The chip is write-only and does not drive the data bus, so the Mega Drive
/// open bus reads high; tests pin the value.
pub const OPEN_BUS_VALUE: u8 = 0xFF;

/// Magic and version of [`Psg::snapshot_bytes`].
pub const SNAPSHOT_MAGIC: [u8; 8] = *b"CFPSG\0\0\0";
/// Version of the fixed PSG snapshot layout.
pub const SNAPSHOT_VERSION: u8 = 1;
/// Fixed byte length of [`Psg::snapshot_bytes`].
pub const SNAPSHOT_LEN: usize = 34;

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
            // Tone-2 clocking advances the shift register once per **full**
            // tone-2 period, which is the falling edge of the square wave, not
            // once per half period.
            NOISE_TONE2_SELECT => tone_toggled[2] && !self.tone_output[2],
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

    /// Returns the last latch byte.
    #[must_use]
    pub const fn latched_register(&self) -> u8 {
        self.latch
    }

    /// Returns the three noise control bits (feedback and shift rate).
    #[must_use]
    pub const fn noise_control(&self) -> u8 {
        self.noise_control
    }

    /// Serializes the complete mutable PSG state into [`SNAPSHOT_LEN`] bytes.
    ///
    /// The attenuation table is derived from constants, not serialized. The latch,
    /// tone dividers and counters, square-wave levels, attenuation registers,
    /// noise control, counter and shift register are canonical state.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(SNAPSHOT_LEN);
        writer.write_bytes(&SNAPSHOT_MAGIC);
        writer.write_u8(SNAPSHOT_VERSION);
        writer.write_u8(self.latch);
        for divider in self.tone_divider {
            writer.write_u16_le(divider);
        }
        for counter in self.tone_counter {
            writer.write_u16_le(counter);
        }
        for output in self.tone_output {
            writer.write_u8(u8::from(output));
        }
        writer.write_bytes(&self.attenuation);
        writer.write_u8(self.noise_control);
        writer.write_u16_le(self.noise_counter);
        writer.write_u16_le(self.noise_shift);
        writer.into_bytes()
    }

    /// Restores the PSG state from [`SNAPSHOT_LEN`] bytes.
    ///
    /// The complete snapshot is validated before any field is assigned, so a
    /// rejected input leaves the device untouched.
    ///
    /// # Errors
    ///
    /// Returns [`AudioSnapshotError`] for a wrong length, magic or version, or a
    /// field outside the ten-bit divider, four-bit attenuation, three-bit noise
    /// control or fifteen-bit shift-register range.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), AudioSnapshotError> {
        let mut reader = Reader::new(bytes);
        if reader.read_array::<8>().map_err(snapshot_error)? != SNAPSHOT_MAGIC {
            return Err(AudioSnapshotError::Magic);
        }
        let version = reader.read_u8().map_err(snapshot_error)?;
        if version != SNAPSHOT_VERSION {
            return Err(AudioSnapshotError::Version { found: version });
        }
        let latch = reader.read_u8().map_err(snapshot_error)?;
        let mut tone_divider = [0_u16; TONE_CHANNELS];
        for divider in &mut tone_divider {
            *divider = reader.read_u16_le().map_err(snapshot_error)?;
            if *divider > DIVIDER_MASK {
                return Err(AudioSnapshotError::Value {
                    field: "tone divider",
                });
            }
        }
        let mut tone_counter = [0_u16; TONE_CHANNELS];
        for counter in &mut tone_counter {
            *counter = reader.read_u16_le().map_err(snapshot_error)?;
            if *counter > 1_u16 << DIVIDER_BITS {
                return Err(AudioSnapshotError::Value {
                    field: "tone counter",
                });
            }
        }
        let mut tone_output = [false; TONE_CHANNELS];
        for output in &mut tone_output {
            *output = decode_flag(reader.read_u8().map_err(snapshot_error)?, "tone output")?;
        }
        let mut attenuation = [0_u8; CHANNELS];
        for level in &mut attenuation {
            *level = reader.read_u8().map_err(snapshot_error)?;
            if *level >= ATTENUATION_LEVELS {
                return Err(AudioSnapshotError::Value {
                    field: "attenuation",
                });
            }
        }
        let noise_control = reader.read_u8().map_err(snapshot_error)?;
        if noise_control > 0x07 {
            return Err(AudioSnapshotError::Value {
                field: "noise control",
            });
        }
        let noise_counter = reader.read_u16_le().map_err(snapshot_error)?;
        if noise_counter > NOISE_CLOCK_DIVIDERS[2] {
            return Err(AudioSnapshotError::Value {
                field: "noise counter",
            });
        }
        let noise_shift = reader.read_u16_le().map_err(snapshot_error)?;
        if noise_shift > (1_u16 << NOISE_SHIFT_BITS) - 1 {
            return Err(AudioSnapshotError::Value {
                field: "noise shift register",
            });
        }
        reader.finish().map_err(snapshot_error)?;
        self.latch = latch;
        self.tone_divider = tone_divider;
        self.tone_counter = tone_counter;
        self.tone_output = tone_output;
        self.attenuation = attenuation;
        self.noise_control = noise_control;
        self.noise_counter = noise_counter;
        self.noise_shift = noise_shift;
        Ok(())
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
