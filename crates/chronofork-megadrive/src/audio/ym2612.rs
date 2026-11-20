//! Yamaha YM2612 (OPN2) six-channel FM synthesiser.
//!
//! This is a deterministic integer model of the chip's register interface, two
//! address/data port pairs, timer/status block, six four-operator FM channels,
//! the channel-6 PCM DAC and the LFO. There is no floating point and no host
//! clock: [`Ym2612::sample`] produces one stereo frame from generated state, and
//! [`Ym2612::timers_elapsed`] advances the timer block by elapsed master cycles.
//!
//! # Register interface
//!
//! The chip has two port pairs. Bank 0 (`0x4000`/`0x4001` on the Z80,
//! `0x00A04000`/`0x00A04001` on the 68000) holds channels 1-3 and the global
//! registers; bank 1 (`0x4002`/`0x4003`, `0x00A04002`/`0x00A04003`) holds
//! channels 4-6. Each pair has its own address latch. The same register address
//! selects channels 1-3 through bank 0 and channels 4-6 through bank 1; that is
//! the "bank" of the register file. [`Ym2612::write`] and [`Ym2612::read`]
//! address bank 0 (the plan's single-port convenience form); [`Ym2612::write_bank`]
//! and [`Ym2612::read_bank`] expose both hardware port pairs.
//!
//! Hardware register reads return the status byte (the data ports do not read
//! registers back). [`Ym2612::register`] exposes the modelled write-only shadow
//! for tests and inspection; it is not a guest-visible read.
//!
//! # Honest limits
//!
//! * Envelope rates are a documented functional integer model (a four-bit rate
//!   plus key scaling selects a per-sample attenuation increment) rather than a
//!   cycle-exact OPN EG counter. The four-bit effective rate selects a per-sample
//!   increment of whole 0.75 dB steps; the attenuation unit is 0.09375 dB, so the
//!   ten-bit counter spans 96 dB and one total-level step is 0.75 dB.
//! * Operator amplitude uses an integer monotonic approximation of the
//!   exponential attenuation curve, not a hardware-exact dB table.
//! * `SSG-EG` (`0x90`-`0x9F`), channel-3 special mode, CSM mode and the
//!   `0x21`/`0x2B` test bits are stored but not applied.
//! * LFO AM/PM are deterministic integer approximations of the documented
//!   shapes, not cycle-exact.
//! * The status byte models only the timer A/B overflow flags; the busy bit and
//!   the undocumented read behaviour of a discrete YM2612 are not modelled.
//! * Timer A/B IRQs only latch the chip's IRQ line ([`Ym2612::take_irq`]);
//!   delivery to the Z80 is phase 8.5.

use chronofork_core::codec::{Reader, Writer};

use crate::audio::{AudioSnapshotError, decode_flag, snapshot_error};

/// Number of FM channels.
pub const CHANNELS: usize = 6;
/// Operators per channel.
pub const OPERATORS_PER_CHANNEL: usize = 4;
/// Entries in the generated sine table (one full wave).
pub const SINE_TABLE_LEN: usize = 256;
/// Maximum envelope attenuation (silence); the scale is 96 dB.
pub const ENVELOPE_MAX: i16 = 0x03FF;
/// Status bit: timer A overflow.
pub const STATUS_TIMER_A: u8 = 0x01;
/// Status bit: timer B overflow.
pub const STATUS_TIMER_B: u8 = 0x02;
/// YM2612 clock divisor from the master clock (VCLK = MCLK / 7).
pub const YM_CLOCK_DIVIDER: u64 = 7;
/// YM2612 sample clock divisor from its own clock (144 YM clocks per sample).
pub const SAMPLE_CLOCK_DIVIDER: u64 = 144;
/// Master cycles per generated sample.
pub const MASTER_CYCLES_PER_SAMPLE: u64 = YM_CLOCK_DIVIDER * SAMPLE_CLOCK_DIVIDER;
/// Phase accumulator bits; one sine period is `1 << PHASE_BITS`.
pub const PHASE_BITS: u32 = 20;
/// Magic and version of [`Ym2612::snapshot_bytes`].
pub const SNAPSHOT_MAGIC: [u8; 8] = *b"CFYM2612";
/// Version of the fixed YM2612 snapshot layout.
pub const SNAPSHOT_VERSION: u8 = 1;
/// Fixed byte length of [`Ym2612::snapshot_bytes`].
pub const SNAPSHOT_LEN: usize = 856;
/// Detune table in 1/512 units of the base increment.
const DETUNE: [i32; 8] = [0, 16, 32, 48, 0, -16, -32, -48];
/// LFO rate table: phase units added to the 16-bit LFO phase per sample.
const LFO_RATE: [u16; 8] = [1, 3, 6, 12, 24, 48, 96, 192];
/// `pi / 2` in Q15 radians.
const PI_OVER_2_Q15: i64 = 51_472;
/// Phase mask for the 20-bit accumulator.
const PHASE_MASK: u32 = (1 << PHASE_BITS) - 1;
/// Envelope attenuation units per whole 0.75 dB step.
///
/// The ten-bit attenuation counter spans 96 dB, so one unit is 0.09375 dB and a
/// documented 0.75 dB envelope step is eight units.
const ENVELOPE_STEP_UNITS: i16 = 8;
/// Per-sample envelope advance in 0.75 dB steps, indexed by the four-bit
/// effective rate.
const ENVELOPE_STEP_TABLE: [i16; 16] = [0, 1, 1, 1, 2, 2, 3, 3, 4, 5, 6, 7, 8, 10, 12, 14];
/// Timer A step: 72 YM clocks expressed in master cycles.
const TIMER_A_MASTER_CYCLES_PER_STEP: u64 = 72 * YM_CLOCK_DIVIDER;
/// Timer B step: 16 * 144 YM clocks expressed in master cycles.
const TIMER_B_MASTER_CYCLES_PER_STEP: u64 = 16 * 144 * YM_CLOCK_DIVIDER;

/// One of the two YM2612 register banks (port pairs).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YmBank {
    /// Channels 1-3 and the global registers.
    One,
    /// Channels 4-6.
    Two,
}

impl YmBank {
    /// Zero-based index into the register and address-latch arrays.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::One => 0,
            Self::Two => 1,
        }
    }

    /// The bank owning `channel` (`0..6`).
    #[must_use]
    pub const fn of_channel(channel: usize) -> Self {
        if channel < 3 { Self::One } else { Self::Two }
    }
}

/// Which latch of a port pair an access targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YmPort {
    /// Address latch.
    Address,
    /// Data latch.
    Data,
}

/// Timer source of a latched YM2612 interrupt request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YmIrq {
    /// Timer A overflow.
    TimerA,
    /// Timer B overflow.
    TimerB,
}

/// Envelope generator state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EgState {
    /// Silent and unkeyed.
    Off,
    /// Attack towards zero attenuation.
    Attack,
    /// Decay towards the sustain level.
    Decay,
    /// Holding the sustain level.
    Sustain,
    /// Release towards silence after key-off.
    Release,
}

/// Read-only view of one operator envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeView {
    /// Current envelope state.
    pub state: EgState,
    /// Current attenuation in 0.09375 dB units (`0` loudest, [`ENVELOPE_MAX`] silent).
    pub attenuation: i16,
}

/// One operator's envelope generator.
#[derive(Clone, Copy, Debug)]
struct Envelope {
    state: EgState,
    attenuation: i16,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            state: EgState::Off,
            attenuation: ENVELOPE_MAX,
        }
    }
}

/// Deterministic YM2612 state.
#[derive(Clone, Debug)]
pub struct Ym2612 {
    /// Write-only register shadow, indexed by bank then address.
    registers: [[u8; 256]; 2],
    address_latch: [u8; 2],
    envelopes: [[Envelope; OPERATORS_PER_CHANNEL]; CHANNELS],
    phases: [[u32; OPERATORS_PER_CHANNEL]; CHANNELS],
    feedback_history: [[i32; 2]; CHANNELS],
    last_outputs: [[i32; OPERATORS_PER_CHANNEL]; CHANNELS],
    sine: [i16; SINE_TABLE_LEN],
    lfo_phase: u16,
    status: u8,
    irq_timer_a: bool,
    irq_timer_b: bool,
    timer_a_accumulator: u64,
    timer_b_accumulator: u64,
}

impl Default for Ym2612 {
    fn default() -> Self {
        Self::new()
    }
}

impl Ym2612 {
    /// Creates a YM2612 in its deterministic reset state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registers: [[0; 256]; 2],
            address_latch: [0; 2],
            envelopes: [[Envelope::default(); OPERATORS_PER_CHANNEL]; CHANNELS],
            phases: [[0; OPERATORS_PER_CHANNEL]; CHANNELS],
            feedback_history: [[0; 2]; CHANNELS],
            last_outputs: [[0; OPERATORS_PER_CHANNEL]; CHANNELS],
            sine: build_sine_table(),
            lfo_phase: 0,
            status: 0,
            irq_timer_a: false,
            irq_timer_b: false,
            timer_a_accumulator: 0,
            timer_b_accumulator: 0,
        }
    }

    /// Writes one byte through bank 0's address/data latch.
    ///
    /// This is the plan's convenience form; use [`Ym2612::write_bank`] for the
    /// second hardware port pair.
    pub fn write(&mut self, port: YmPort, value: u8) {
        self.write_bank(YmBank::One, port, value);
    }

    /// Reads the status byte through bank 0's address/data latch.
    ///
    /// Hardware returns the status byte for both register reads; the latched
    /// register value is available through [`Ym2612::register`].
    pub fn read(&mut self, _port: YmPort) -> u8 {
        self.status
    }

    /// Writes one byte through a hardware port pair.
    pub fn write_bank(&mut self, bank: YmBank, port: YmPort, value: u8) {
        let index = bank.index();
        match port {
            YmPort::Address => self.address_latch[index] = value,
            YmPort::Data => self.write_register(bank, self.address_latch[index], value),
        }
    }

    /// Reads the status byte through the selected port pair.
    pub fn read_bank(&mut self, _bank: YmBank, _port: YmPort) -> u8 {
        self.status
    }

    /// Returns the current timer status byte.
    #[must_use]
    pub const fn status(&self) -> u8 {
        self.status
    }

    /// Returns the current 16-bit LFO phase.
    #[must_use]
    pub const fn lfo_phase(&self) -> u16 {
        self.lfo_phase
    }

    /// Returns the latched register address of a port pair.
    #[must_use]
    pub const fn address_latch(&self, bank: YmBank) -> u8 {
        self.address_latch[bank.index()]
    }

    /// Takes a latched timer interrupt request, if any.
    ///
    /// The status flag stays set until register `0x27` bit 4 clears it; taking
    /// the IRQ line does not clear the flag. Delivery to the Z80 is phase 8.5.
    pub fn take_irq(&mut self) -> Option<YmIrq> {
        if self.irq_timer_a {
            self.irq_timer_a = false;
            return Some(YmIrq::TimerA);
        }
        if self.irq_timer_b {
            self.irq_timer_b = false;
            return Some(YmIrq::TimerB);
        }
        None
    }

    /// Advances the timer block by `master_cycles` master clocks.
    ///
    /// Timer A counts 72 YM clocks per step over its 10-bit `(1024 - TA)`
    /// period; timer B counts `16 * 144` YM clocks over `(256 - TB)`. A running
    /// timer raises its status bit on overflow; the matching `0x27` enable bit
    /// additionally latches the chip's IRQ line.
    pub fn timers_elapsed(&mut self, master_cycles: u64) {
        if master_cycles == 0 {
            return;
        }
        let control = self.timer_control();
        if control & 0x01 != 0 {
            let period = self.timer_a_period();
            let total = self.timer_a_accumulator.saturating_add(master_cycles);
            let overflows = total / period;
            self.timer_a_accumulator = total % period;
            if overflows > 0 {
                self.status |= STATUS_TIMER_A;
                if control & 0x04 != 0 {
                    self.irq_timer_a = true;
                }
            }
        } else {
            self.timer_a_accumulator = 0;
        }
        if control & 0x02 != 0 {
            let period = self.timer_b_period();
            let total = self.timer_b_accumulator.saturating_add(master_cycles);
            let overflows = total / period;
            self.timer_b_accumulator = total % period;
            if overflows > 0 {
                self.status |= STATUS_TIMER_B;
                if control & 0x08 != 0 {
                    self.irq_timer_b = true;
                }
            }
        } else {
            self.timer_b_accumulator = 0;
        }
    }

    /// Generates one stereo FM frame.
    ///
    /// The left/right channel enables are the documented `0xB4`/`0xB5`/`0xB6`
    /// stereo bits; a channel whose bits are clear is silent. Channel 6 outputs
    /// the DAC value instead of FM while `0x2B` bit 7 is set, and its FM operator
    /// chain (including envelopes) does not advance during that time.
    pub fn sample(&mut self) -> (i16, i16) {
        if self.lfo_enabled() {
            let rate = self.lfo_rate();
            self.lfo_phase = self.lfo_phase.wrapping_add(rate);
        }
        let mut left = 0_i32;
        let mut right = 0_i32;
        for channel in 0..CHANNELS {
            let output = if channel == 5 && self.dac_enabled() {
                (i32::from(self.dac_value()) - 128) * 256
            } else {
                self.channel_output(channel)
            };
            let (left_enabled, right_enabled) = self.channel_stereo(channel);
            if left_enabled {
                left += output;
            }
            if right_enabled {
                right += output;
            }
        }
        (clamp_i16(left), clamp_i16(right))
    }

    /// Returns the latched register value for an address in a bank.
    ///
    /// This is a model readback for tests and inspection; the chip itself
    /// returns status on register reads.
    #[must_use]
    pub fn register(&self, bank: YmBank, address: u8) -> u8 {
        self.registers[bank.index()][usize::from(address)]
    }

    /// Returns one operator's envelope state and attenuation.
    #[must_use]
    pub fn envelope(&self, channel: usize, operator: usize) -> Option<EnvelopeView> {
        let envelope = self.envelopes.get(channel)?.get(operator)?;
        Some(EnvelopeView {
            state: envelope.state,
            attenuation: envelope.attenuation,
        })
    }

    /// Returns the last generated output of one operator, in Q15 units.
    #[must_use]
    pub fn operator_output(&self, channel: usize, operator: usize) -> Option<i32> {
        self.last_outputs.get(channel)?.get(operator).copied()
    }

    /// Returns one operator's base phase increment before LFO PM.
    #[must_use]
    pub fn operator_phase_increment(&self, channel: usize, operator: usize) -> Option<u32> {
        if channel >= CHANNELS || operator >= OPERATORS_PER_CHANNEL {
            return None;
        }
        Some(self.phase_increment_for(channel, operator))
    }

    /// Returns the generated sine table.
    #[must_use]
    pub fn sine_table(&self) -> &[i16; SINE_TABLE_LEN] {
        &self.sine
    }

    fn write_register(&mut self, bank: YmBank, address: u8, value: u8) {
        self.registers[bank.index()][usize::from(address)] = value;
        // Bank 1 addresses mirror the channel register space. Global side
        // effects (timers, DAC, LFO and channel-3 mode) are reached only
        // through bank 0; key-on/off (`0x28`) applies to the bank's channels.
        match address {
            0x27 if bank == YmBank::One => {
                if value & 0x10 != 0 {
                    self.status = 0;
                    self.irq_timer_a = false;
                    self.irq_timer_b = false;
                }
                if value & 0x01 != 0 {
                    self.timer_a_accumulator = 0;
                }
                if value & 0x02 != 0 {
                    self.timer_b_accumulator = 0;
                }
            }
            0x28 => {
                // The key-on/off data byte names the channel: its low two bits
                // select the channel inside a group (three is reserved and keys
                // nothing) and bit 2 selects the group, so `0x28` reaches
                // channels 4-6 whichever port pair carried it. The high nibble
                // keys operators 1-4 individually.
                let ch = usize::from(value & 0x03);
                if ch == 3 {
                    return;
                }
                let channel = ch + usize::from((value >> 2) & 1) * 3;
                let key_bits = (value >> 4) & 0x0F;
                for operator in 0..OPERATORS_PER_CHANNEL {
                    let on = key_bits & (1 << operator) != 0;
                    self.key_operator(channel, operator, on);
                }
            }
            _ => {}
        }
    }

    fn key_operator(&mut self, channel: usize, operator: usize, on: bool) {
        let envelope = &mut self.envelopes[channel][operator];
        if on {
            envelope.state = EgState::Attack;
            self.phases[channel][operator] = 0;
            self.feedback_history[channel] = [0, 0];
        } else if envelope.state != EgState::Off {
            envelope.state = EgState::Release;
        }
    }

    fn timer_control(&self) -> u8 {
        self.registers[0][0x27]
    }

    fn timer_a_value(&self) -> u16 {
        (u16::from(self.registers[0][0x24]) << 2) | u16::from(self.registers[0][0x25] & 0x03)
    }

    fn timer_a_period(&self) -> u64 {
        u64::from(1024 - self.timer_a_value()) * TIMER_A_MASTER_CYCLES_PER_STEP
    }

    fn timer_b_period(&self) -> u64 {
        u64::from(256 - u16::from(self.registers[0][0x26])) * TIMER_B_MASTER_CYCLES_PER_STEP
    }

    fn lfo_enabled(&self) -> bool {
        self.registers[0][0x22] & 0x08 != 0
    }

    fn lfo_rate(&self) -> u16 {
        LFO_RATE[usize::from(self.registers[0][0x22] & 0x07)]
    }

    fn dac_enabled(&self) -> bool {
        self.registers[0][0x2B] & 0x80 != 0
    }

    fn dac_value(&self) -> u8 {
        self.registers[0][0x2A]
    }

    fn channel_register(&self, channel: usize, address: u8) -> u8 {
        self.registers[YmBank::of_channel(channel).index()][usize::from(address)]
    }

    fn operator_register(&self, channel: usize, operator: usize, base: u8) -> u8 {
        let offset = (channel % 3 + operator * 4) as u8;
        self.channel_register(channel, base.wrapping_add(offset))
    }

    fn channel_stereo(&self, channel: usize) -> (bool, bool) {
        let register = self.channel_register(channel, 0xB4 + (channel % 3) as u8);
        (register & 0x40 != 0, register & 0x80 != 0)
    }

    fn channel_algorithm(&self, channel: usize) -> (u8, u8) {
        let register = self.channel_register(channel, 0xB0 + (channel % 3) as u8);
        (register & 0x07, (register >> 3) & 0x07)
    }

    fn channel_block(&self, channel: usize) -> u8 {
        let high = self.channel_register(channel, 0xA4 + (channel % 3) as u8);
        (high >> 3) & 0x07
    }

    fn phase_increment_for(&self, channel: usize, operator: usize) -> u32 {
        let low = self.channel_register(channel, 0xA0 + (channel % 3) as u8);
        let high = self.channel_register(channel, 0xA4 + (channel % 3) as u8);
        let fnum = i64::from(u16::from(high & 0x07) << 8 | u16::from(low));
        let block = i64::from((high >> 3) & 0x07);
        let register = self.operator_register(channel, operator, 0x30);
        let multiple = i64::from(register & 0x0F);
        let detune = DETUNE[usize::from((register >> 4) & 0x07)];

        // Documented rate: fnum * 2^(block-1), with block 0 halving and
        // multiple 0 halving again. Detune is a 1/512 fraction of that.
        let mut increment = if block == 0 {
            fnum / 2
        } else {
            fnum << (block - 1)
        };
        increment = if multiple == 0 {
            increment / 2
        } else {
            increment * multiple
        };
        increment = increment * i64::from(512 + detune) / 512;
        increment.clamp(0, i64::from(PHASE_MASK)) as u32
    }

    fn lfo_pm(&self) -> i32 {
        if !self.lfo_enabled() {
            return 0;
        }
        i32::from(self.lfo_phase >> 8) - 128
    }

    fn lfo_am_offset(&self, channel: usize) -> i16 {
        if !self.lfo_enabled() {
            return 0;
        }
        let register = self.channel_register(channel, 0xB4 + (channel % 3) as u8);
        let depth = i32::from((register >> 4) & 0x03);
        if depth == 0 {
            return 0;
        }
        let level = i32::from(self.lfo_phase >> 8);
        let triangle = if level < 128 { level } else { 255 - level };
        (triangle * depth / 2) as i16
    }

    fn apply_lfo_pm(&self, channel: usize, increment: u32) -> u32 {
        let register = self.channel_register(channel, 0xB4 + (channel % 3) as u8);
        let depth = i32::from(register & 0x07);
        if depth == 0 {
            return increment;
        }
        let base = i64::from(increment);
        let adjusted = base + base * i64::from(self.lfo_pm()) * i64::from(depth) / 1024;
        adjusted.clamp(0, i64::from(PHASE_MASK)) as u32
    }

    fn channel_output(&mut self, channel: usize) -> i32 {
        let (feedback, algorithm) = self.channel_algorithm(channel);
        let feedback_input = if feedback == 0 {
            0
        } else {
            let shift = 10 - u32::from(feedback);
            (self.feedback_history[channel][0] + self.feedback_history[channel][1]) >> shift
        };
        let output = match algorithm {
            0 => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, o1);
                let o3 = self.operator_sample(channel, 2, o2);
                self.operator_sample(channel, 3, o3)
            }
            1 => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, o1);
                let o3 = self.operator_sample(channel, 2, o1 + o2);
                self.operator_sample(channel, 3, o3)
            }
            2 => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, 0);
                let o3 = self.operator_sample(channel, 2, o2);
                self.operator_sample(channel, 3, o1 + o3)
            }
            3 => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, o1);
                let o3 = self.operator_sample(channel, 2, 0);
                self.operator_sample(channel, 3, o2 + o3)
            }
            4 => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, o1);
                let o3 = self.operator_sample(channel, 2, 0);
                let o4 = self.operator_sample(channel, 3, o3);
                o2 + o4
            }
            5 => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, o1);
                let o3 = self.operator_sample(channel, 2, o1);
                let o4 = self.operator_sample(channel, 3, o1);
                o2 + o3 + o4
            }
            6 => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, o1);
                let o3 = self.operator_sample(channel, 2, 0);
                let o4 = self.operator_sample(channel, 3, 0);
                o2 + o3 + o4
            }
            _ => {
                let o1 = self.operator_sample(channel, 0, feedback_input);
                let o2 = self.operator_sample(channel, 1, 0);
                let o3 = self.operator_sample(channel, 2, 0);
                let o4 = self.operator_sample(channel, 3, 0);
                o1 + o2 + o3 + o4
            }
        };
        self.feedback_history[channel] = [
            self.last_outputs[channel][0],
            self.feedback_history[channel][0],
        ];
        output
    }

    fn operator_sample(&mut self, channel: usize, operator: usize, modulation: i32) -> i32 {
        self.advance_envelope(channel, operator);
        let increment = self.apply_lfo_pm(channel, self.phase_increment_for(channel, operator));
        let phase = self.phases[channel][operator].wrapping_add(increment);
        self.phases[channel][operator] = phase;
        // Modulation is a documented integer phase offset: one Q15 operator
        // output unit is a quarter of a phase unit.
        let lookup = phase.wrapping_add((modulation as u32).wrapping_mul(4));
        let sine = self.sine_lookup(lookup);
        let amplitude = i32::from(self.operator_amplitude(channel, operator));
        let output = (sine * amplitude) >> 10;
        self.last_outputs[channel][operator] = output;
        output
    }

    fn operator_amplitude(&self, channel: usize, operator: usize) -> i16 {
        let envelope = self.envelopes[channel][operator].attenuation;
        let total_level = i16::from(self.operator_register(channel, operator, 0x40) & 0x7F) * 8;
        let mut attenuation = envelope.saturating_add(total_level);
        if self.operator_register(channel, operator, 0x60) & 0x80 != 0 {
            attenuation = attenuation.saturating_add(self.lfo_am_offset(channel));
        }
        let attenuation = attenuation.clamp(0, ENVELOPE_MAX);
        let loudness = i32::from(ENVELOPE_MAX - attenuation);
        // Monotonic integer approximation of the exponential attenuation curve.
        ((loudness * loudness) >> 10) as i16
    }

    fn sine_lookup(&self, phase: u32) -> i32 {
        let index = ((phase >> 12) & 0xFF) as usize;
        let fraction = (phase & 0x0FFF) as i32;
        let current = i32::from(self.sine[index]);
        let next = i32::from(self.sine[(index + 1) & 0xFF]);
        current + (((next - current) * fraction) >> 12)
    }

    fn advance_envelope(&mut self, channel: usize, operator: usize) {
        let rates = self.operator_rates(channel, operator);
        let block = self.channel_block(channel);
        let sustain = i16::from(self.sustain_level(channel, operator)) * 32;
        let envelope = &mut self.envelopes[channel][operator];
        match envelope.state {
            EgState::Attack => {
                let increment = envelope_increment(rates.attack, rates.key_scale, block);
                if increment > 0 {
                    envelope.attenuation = (envelope.attenuation - increment).max(0);
                    if envelope.attenuation == 0 {
                        envelope.state = EgState::Decay;
                    }
                }
            }
            EgState::Decay => {
                let increment = envelope_increment(rates.decay, rates.key_scale, block);
                if increment > 0 {
                    envelope.attenuation = (envelope.attenuation + increment).min(sustain);
                    if envelope.attenuation >= sustain {
                        envelope.state = EgState::Sustain;
                    }
                }
            }
            EgState::Sustain => {}
            EgState::Release => {
                let increment = envelope_increment(rates.release, rates.key_scale, block);
                if increment > 0 {
                    envelope.attenuation = (envelope.attenuation + increment).min(ENVELOPE_MAX);
                    if envelope.attenuation >= ENVELOPE_MAX {
                        envelope.state = EgState::Off;
                    }
                }
            }
            EgState::Off => {}
        }
    }

    fn sustain_level(&self, channel: usize, operator: usize) -> u8 {
        (self.operator_register(channel, operator, 0x80) >> 4) & 0x0F
    }

    fn operator_rates(&self, channel: usize, operator: usize) -> OperatorRates {
        let attack_register = self.operator_register(channel, operator, 0x50);
        let decay_register = self.operator_register(channel, operator, 0x60);
        let release_register = self.operator_register(channel, operator, 0x80);
        OperatorRates {
            key_scale: attack_register >> 6,
            attack: attack_register & 0x1F,
            decay: decay_register & 0x1F,
            release: release_register & 0x0F,
        }
    }

    /// Serializes the complete mutable YM2612 state into [`SNAPSHOT_LEN`] bytes.
    ///
    /// The generated sine table is derived from constants, not serialized. The
    /// register shadow, both address latches, every operator envelope and phase,
    /// feedback history, last outputs, LFO phase, status/IRQ latches and timer
    /// accumulators are canonical state and are all covered.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(SNAPSHOT_LEN);
        writer.write_bytes(&SNAPSHOT_MAGIC);
        writer.write_u8(SNAPSHOT_VERSION);
        for bank in &self.registers {
            writer.write_bytes(bank);
        }
        for latch in self.address_latch {
            writer.write_u8(latch);
        }
        for channel in &self.envelopes {
            for envelope in channel {
                writer.write_u8(eg_state_code(envelope.state));
                writer.write_u16_le(envelope.attenuation as u16);
            }
        }
        for channel in &self.phases {
            for phase in channel {
                writer.write_u32_le(*phase);
            }
        }
        for channel in &self.feedback_history {
            for value in channel {
                writer.write_u32_le(*value as u32);
            }
        }
        for channel in &self.last_outputs {
            for value in channel {
                writer.write_u32_le(*value as u32);
            }
        }
        writer.write_u16_le(self.lfo_phase);
        writer.write_u8(self.status);
        writer.write_u8(u8::from(self.irq_timer_a));
        writer.write_u8(u8::from(self.irq_timer_b));
        writer.write_u64_le(self.timer_a_accumulator);
        writer.write_u64_le(self.timer_b_accumulator);
        writer.into_bytes()
    }

    /// Restores the YM2612 state from [`SNAPSHOT_LEN`] bytes.
    ///
    /// The complete snapshot is validated before any field is assigned, so a
    /// rejected input leaves the device untouched. The generated sine table is
    /// not part of the payload and is left as constructed.
    ///
    /// # Errors
    ///
    /// Returns [`AudioSnapshotError`] for a wrong length, magic or version, or a
    /// non-canonical envelope state or attenuation value.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), AudioSnapshotError> {
        let mut reader = Reader::new(bytes);
        if reader.read_array::<8>().map_err(snapshot_error)? != SNAPSHOT_MAGIC {
            return Err(AudioSnapshotError::Magic);
        }
        let version = reader.read_u8().map_err(snapshot_error)?;
        if version != SNAPSHOT_VERSION {
            return Err(AudioSnapshotError::Version { found: version });
        }
        let mut registers = [[0_u8; 256]; 2];
        for bank in &mut registers {
            *bank = reader.read_array::<256>().map_err(snapshot_error)?;
        }
        let mut address_latch = [0_u8; 2];
        for latch in &mut address_latch {
            *latch = reader.read_u8().map_err(snapshot_error)?;
        }
        let mut envelopes = [[Envelope::default(); OPERATORS_PER_CHANNEL]; CHANNELS];
        for channel in &mut envelopes {
            for envelope in channel {
                let state = eg_state_from_code(reader.read_u8().map_err(snapshot_error)?)?;
                let attenuation = reader.read_u16_le().map_err(snapshot_error)? as i16;
                if !(0..=ENVELOPE_MAX).contains(&attenuation) {
                    return Err(AudioSnapshotError::Value {
                        field: "envelope attenuation",
                    });
                }
                *envelope = Envelope { state, attenuation };
            }
        }
        let mut phases = [[0_u32; OPERATORS_PER_CHANNEL]; CHANNELS];
        for channel in &mut phases {
            for phase in channel {
                *phase = reader.read_u32_le().map_err(snapshot_error)?;
            }
        }
        let mut feedback_history = [[0_i32; 2]; CHANNELS];
        for channel in &mut feedback_history {
            for value in channel {
                *value = reader.read_u32_le().map_err(snapshot_error)? as i32;
            }
        }
        let mut last_outputs = [[0_i32; OPERATORS_PER_CHANNEL]; CHANNELS];
        for channel in &mut last_outputs {
            for value in channel {
                *value = reader.read_u32_le().map_err(snapshot_error)? as i32;
            }
        }
        let lfo_phase = reader.read_u16_le().map_err(snapshot_error)?;
        let status = reader.read_u8().map_err(snapshot_error)?;
        let irq_timer_a = decode_flag(
            reader.read_u8().map_err(snapshot_error)?,
            "timer A IRQ latch",
        )?;
        let irq_timer_b = decode_flag(
            reader.read_u8().map_err(snapshot_error)?,
            "timer B IRQ latch",
        )?;
        let timer_a_accumulator = reader.read_u64_le().map_err(snapshot_error)?;
        let timer_b_accumulator = reader.read_u64_le().map_err(snapshot_error)?;
        reader.finish().map_err(snapshot_error)?;
        self.registers = registers;
        self.address_latch = address_latch;
        self.envelopes = envelopes;
        self.phases = phases;
        self.feedback_history = feedback_history;
        self.last_outputs = last_outputs;
        self.lfo_phase = lfo_phase;
        self.status = status;
        self.irq_timer_a = irq_timer_a;
        self.irq_timer_b = irq_timer_b;
        self.timer_a_accumulator = timer_a_accumulator;
        self.timer_b_accumulator = timer_b_accumulator;
        Ok(())
    }
}

/// Encodes an envelope state as its canonical snapshot byte.
const fn eg_state_code(state: EgState) -> u8 {
    match state {
        EgState::Off => 0,
        EgState::Attack => 1,
        EgState::Decay => 2,
        EgState::Sustain => 3,
        EgState::Release => 4,
    }
}

/// Decodes an envelope state snapshot byte.
fn eg_state_from_code(code: u8) -> Result<EgState, AudioSnapshotError> {
    match code {
        0 => Ok(EgState::Off),
        1 => Ok(EgState::Attack),
        2 => Ok(EgState::Decay),
        3 => Ok(EgState::Sustain),
        4 => Ok(EgState::Release),
        _ => Err(AudioSnapshotError::Value {
            field: "envelope state",
        }),
    }
}

#[derive(Clone, Copy, Debug)]
struct OperatorRates {
    key_scale: u8,
    attack: u8,
    decay: u8,
    release: u8,
}

/// Four-bit effective envelope rate including key scaling.
///
/// `(2 * base + KS + (block >> (3 - KS))) >> 2`, clamped to 15. Key scaling
/// makes higher notes and larger `KS` values advance the envelope faster, which
/// is the documented purpose of the `KS` field; the exact OPN rate table is not
/// claimed.
fn effective_rate(base: u8, key_scale: u8, block: u8) -> u8 {
    let mut rate = u16::from(base) * 2 + u16::from(key_scale);
    if key_scale != 0 {
        rate += u16::from(block) >> (3 - key_scale);
    }
    u8::try_from((rate >> 2).min(15)).unwrap_or(15)
}

/// Per-sample attenuation increment for an effective rate.
///
/// Rate 0 means "no movement". Other rates advance the ten-bit attenuation
/// counter by a documented number of whole 0.75 dB steps (eight attenuation
/// units each); this is a functional model, not a hardware-exact EG counter
/// table.
fn envelope_increment(base: u8, key_scale: u8, block: u8) -> i16 {
    if base == 0 {
        return 0;
    }
    let rate = effective_rate(base, key_scale, block);
    ENVELOPE_STEP_TABLE[usize::from(rate)] * ENVELOPE_STEP_UNITS
}

/// Clamps a mixed sample to the `i16` output range.
fn clamp_i16(value: i32) -> i16 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Builds the full-wave sine table from an integer Taylor series.
///
/// The quarter wave is evaluated in Q15 radians with integer divisions only; the
/// other quadrants are mirrored. No floating point is used at construction or at
/// run time.
fn build_sine_table() -> [i16; SINE_TABLE_LEN] {
    let mut table = [0_i16; SINE_TABLE_LEN];
    let mut index = 0;
    while index <= 64 {
        table[index] = quarter_sine(index);
        index += 1;
    }
    index = 1;
    while index < 64 {
        table[128 - index] = table[index];
        table[128 + index] = -table[index];
        index += 1;
    }
    table[128] = 0;
    table[192] = -table[64];
    index = 1;
    while index < 64 {
        table[192 + index] = -table[64 - index];
        index += 1;
    }
    table
}

/// Q15 sine of `index * (pi/2) / 64` using an integer Taylor series.
fn quarter_sine(index: usize) -> i16 {
    let angle = (index as i64) * PI_OVER_2_Q15 / 64;
    let x2 = (angle * angle) >> 15;
    let mut term = angle;
    let mut sum = angle;
    let mut order = 1_i64;
    while order <= 6 {
        let divisor = ((2 * order) * (2 * order + 1)) << 15;
        term = -(term * x2) / divisor;
        sum += term;
        order += 1;
    }
    sum.clamp(0, 32_767) as i16
}
