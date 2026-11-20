//! Z80 core: register file, T-state time base, instruction dispatch and a
//! versioned snapshot.
//!
//! **Scope.** This module implements the documented Z80 instruction set: the
//! main opcode table ([`ops`]), the `CB` prefix ([`cb`]), the `ED` prefix
//! ([`ed`]) and the `DD`/`FD` index-register prefixes ([`xy`]). Interrupt
//! delivery and NMI are later phase-8.4 tasks; this core models only the state
//! they need (`IFF1`/`IFF2`, interrupt mode, the `EI` one-instruction acceptance
//! delay and `HALT`). The Z80 memory map, banking and 68000 bus arbitration are
//! also later tasks, so a caller supplies a [`Z80Bus`] view of memory and I/O.
//!
//! **Timing.** [`Z80::step`] executes exactly one instruction and returns its
//! documented T-state count. One Z80 T-state is
//! [`MASTER_CYCLES_PER_Z80_T_STATE`] master cycles, so the backend converts
//! instruction cost to canonical time with [`Z80Outcome::master_cycles`] and
//! never with a host clock.
//!
//! **Honest limits.** Undocumented instruction behaviour is listed in each
//! table module; the undocumented `IXH`/`IXL`/`IYH`/`IYL` byte registers are not
//! modelled, so a `DD`/`FD` prefix on an opcode that does not address `HL` or
//! `(HL)` is a wasted prefix (documented in [`ops`]). `WZ`/`MEMPTR` is not
//! modelled because no implemented instruction exposes it. Interrupt acceptance,
//! NMI, `R` refresh timing beyond the per-M1 increment and the `DD`/`FD` prefix
//! peek are described where they are implemented.

use chronofork_core::codec::{CodecError, Reader, Writer};

use crate::cycles::MASTER_CYCLES_PER_Z80_T_STATE;

pub mod cb;
pub mod ed;
pub mod interrupts;
pub mod ops;
mod target;
pub mod xy;

pub(crate) use target::{IndexKind, Target};

/// Flag bit: sign.
pub const FLAG_S: u8 = 0x80;
/// Flag bit: zero.
pub const FLAG_Z: u8 = 0x40;
/// Flag bit: undocumented copy of result bit 5.
pub const FLAG_Y: u8 = 0x20;
/// Flag bit: half carry (carry out of bit 3).
pub const FLAG_H: u8 = 0x10;
/// Flag bit: undocumented copy of result bit 3.
pub const FLAG_X: u8 = 0x08;
/// Flag bit: parity/overflow.
pub const FLAG_PV: u8 = 0x04;
/// Flag bit: add/subtract.
pub const FLAG_N: u8 = 0x02;
/// Flag bit: carry.
pub const FLAG_C: u8 = 0x01;

/// Snapshot header magic, eight bytes.
pub const SNAPSHOT_MAGIC: [u8; 8] = *b"CFZ80\0\0\0";

/// Version of the fixed-length Z80 snapshot layout.
///
/// Version 2 adds the latched maskable interrupt request, its vector byte and
/// the NMI request latch.
pub const SNAPSHOT_VERSION: u8 = 2;

/// Length in bytes of [`Z80::snapshot_bytes`].
///
/// Layout: eight magic bytes, one version byte, the current byte registers
/// (`A F B C D E H L`), the alternate set in the same order, `IX`, `IY`, `SP`
/// and `PC` as little-endian words, `I`, `R`, then one byte each for `IFF1`,
/// `IFF2`, interrupt mode, `HALT`, the `EI` delay latch, a maskable interrupt
/// request latch, its vector byte and the NMI request latch.
pub const SNAPSHOT_LEN: usize = 43;

/// A typed Z80 failure.
///
/// A [`Z80Bus`] implementation translates its own memory-map errors into
/// [`Z80Error::Memory`] or [`Z80Error::Io`]; snapshot parsing never panics and
/// reports malformed input as [`Z80Error::Snapshot`] or a field-specific error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Z80Error {
    /// The bus rejected a memory access at `address`.
    Memory {
        /// Address the CPU tried to read or write.
        address: u16,
    },
    /// The bus rejected an I/O access at `port`.
    Io {
        /// Port the CPU tried to read or write.
        port: u16,
    },
    /// The snapshot byte sequence is truncated or has trailing bytes.
    Snapshot(CodecError),
    /// The snapshot does not begin with [`SNAPSHOT_MAGIC`].
    SnapshotMagic,
    /// The snapshot layout version is not [`SNAPSHOT_VERSION`].
    SnapshotVersion {
        /// Version byte found in the snapshot.
        found: u8,
    },
    /// A snapshot field holds a value that cannot occur in canonical state.
    SnapshotField {
        /// Which field was invalid.
        field: Z80Field,
    },
}

/// Snapshot fields validated for canonical range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Z80Field {
    /// Interrupt mode must be 0, 1 or 2.
    InterruptMode,
    /// `IFF1` must be a single boolean byte.
    Iff1,
    /// `IFF2` must be a single boolean byte.
    Iff2,
    /// `HALT` must be a single boolean byte.
    Halted,
    /// The `EI` delay latch must be a single boolean byte.
    EiDelay,
    /// The maskable interrupt request latch must be a single boolean byte.
    InterruptRequest,
    /// The NMI request latch must be a single boolean byte.
    NmiRequest,
}

impl std::fmt::Display for Z80Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory { address } => {
                write!(formatter, "Z80 memory access failed at {address:#06x}")
            }
            Self::Io { port } => write!(formatter, "Z80 I/O access failed at {port:#06x}"),
            Self::Snapshot(error) => write!(formatter, "Z80 snapshot is malformed: {error}"),
            Self::SnapshotMagic => formatter.write_str("Z80 snapshot magic does not match"),
            Self::SnapshotVersion { found } => write!(
                formatter,
                "Z80 snapshot version {found} is not supported (expected {SNAPSHOT_VERSION})"
            ),
            Self::SnapshotField { field } => {
                write!(formatter, "Z80 snapshot field {field:?} is out of range")
            }
        }
    }
}

impl std::error::Error for Z80Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Snapshot(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CodecError> for Z80Error {
    fn from(error: CodecError) -> Self {
        Self::Snapshot(error)
    }
}

/// Result of executing one Z80 instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Z80Outcome {
    /// T-states consumed by the instruction.
    pub t_states: u32,
}

impl Z80Outcome {
    /// Converts the instruction cost to master-clock cycles.
    ///
    /// One Z80 T-state is [`MASTER_CYCLES_PER_Z80_T_STATE`] master cycles; the
    /// conversion is integer-only so canonical time stays deterministic.
    #[must_use]
    pub const fn master_cycles(self) -> u64 {
        self.t_states as u64 * MASTER_CYCLES_PER_Z80_T_STATE
    }
}

/// Memory and I/O view the Z80 core executes against.
///
/// The implementer owns address decoding (Z80 RAM, banked ROM, YM2612, PSG and
/// bank register) and translates its own failures into [`Z80Error`]. Reads and
/// writes here are one memory cycle each; the core accounts for their T-states
/// in the instruction table.
pub trait Z80Bus {
    /// Reads one byte from `address`.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is not mapped or the read fails.
    fn read8(&mut self, address: u16) -> Result<u8, Z80Error>;
    /// Writes one byte to `address`.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is not mapped or the write fails.
    fn write8(&mut self, address: u16, value: u8) -> Result<(), Z80Error>;
    /// Reads one byte from I/O `port`.
    ///
    /// # Errors
    ///
    /// Returns an error when the port read fails.
    fn input(&mut self, port: u16) -> Result<u8, Z80Error>;
    /// Writes one byte to I/O `port`.
    ///
    /// # Errors
    ///
    /// Returns an error when the port write fails.
    fn output(&mut self, port: u16, value: u8) -> Result<(), Z80Error>;
}

/// The documented Z80 register file and control state.
///
/// All fields are canonical state and are included in
/// [`Z80::snapshot_bytes`]. `WZ`/`MEMPTR` is not modelled (no implemented
/// instruction exposes it), and the undocumented `IXH`/`IXL`/`IYH`/`IYL` byte
/// registers are not present.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Z80 {
    a: u8,
    f: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    a_alt: u8,
    f_alt: u8,
    b_alt: u8,
    c_alt: u8,
    d_alt: u8,
    e_alt: u8,
    h_alt: u8,
    l_alt: u8,
    ix: u16,
    iy: u16,
    sp: u16,
    pc: u16,
    i: u8,
    r: u8,
    iff1: bool,
    iff2: bool,
    im: u8,
    halted: bool,
    ei_delay: bool,
    /// Latched maskable interrupt request; the vector is supplied by the device.
    interrupt_pending: bool,
    /// Vector byte of the latched maskable interrupt request.
    interrupt_vector: u8,
    /// Latched non-maskable interrupt request.
    nmi_pending: bool,
}

impl Z80 {
    /// Creates a Z80 in its deterministic power-on state (all fields zero).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the core to its deterministic power-on state.
    ///
    /// The bus parameter matches the plan's interface; a Z80 reset performs no
    /// memory or I/O access (the 68000 initialises Z80 RAM separately). Every
    /// modelled field is cleared, `PC` becomes `0x0000`, `IFF1`/`IFF2` clear and
    /// interrupt mode becomes 0. Real hardware leaves several registers
    /// undefined; clearing them is the documented deterministic choice.
    pub fn reset(&mut self, _bus: &mut impl Z80Bus) {
        *self = Self::default();
    }

    /// Executes one instruction and returns its documented T-state cost.
    ///
    /// `HALT` is modelled as repeated 4 T-state steps that do not fetch: waking
    /// the core belongs to interrupt delivery (a later phase-8.4 task), so a
    /// halted core keeps reporting 4 T-states and leaves `R` unchanged.
    ///
    /// A `DD`/`FD` prefix followed by another `DD`/`FD` is a wasted 4 T-state
    /// step. To keep the step bounded, the core reads (peeks) the next byte to
    /// detect that case and fetches it again as the M1 opcode on the following
    /// step; for RAM and ROM instruction streams the extra read is invisible.
    ///
    /// # Errors
    ///
    /// Propagates a [`Z80Bus`] failure from any instruction fetch or data access.
    pub fn step(&mut self, bus: &mut impl Z80Bus) -> Result<Z80Outcome, Z80Error> {
        let interrupt_delay_elapsed = self.ei_delay;
        // An interrupt is accepted at an instruction boundary instead of an
        // instruction. `EI`'s one-instruction delay is measured in executed
        // instructions, so an accepted interrupt does not itself clear it.
        if let Some(t_states) = interrupts::accept(self, bus)? {
            return Ok(Z80Outcome { t_states });
        }
        if self.halted {
            if interrupt_delay_elapsed {
                self.ei_delay = false;
            }
            return Ok(Z80Outcome { t_states: 4 });
        }
        let opcode = self.fetch_opcode(bus)?;
        let t_states = match opcode {
            0xCB => {
                let extended = self.fetch_opcode(bus)?;
                cb::execute(self, bus, extended)?
            }
            0xED => {
                let extended = self.fetch_opcode(bus)?;
                ed::execute(self, bus, extended)?
            }
            0xDD | 0xFD => {
                let following = bus.read8(self.pc)?;
                if following == 0xDD || following == 0xFD {
                    4
                } else {
                    xy::execute(self, bus, opcode)?
                }
            }
            _ => ops::execute(self, bus, opcode)?,
        };
        if interrupt_delay_elapsed {
            self.ei_delay = false;
        }
        Ok(Z80Outcome { t_states })
    }

    /// Returns the current `AF` pair.
    #[must_use]
    pub const fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }

    /// Writes the current `AF` pair.
    pub const fn set_af(&mut self, value: u16) {
        [self.a, self.f] = value.to_be_bytes();
    }

    /// Returns the current `BC` pair.
    #[must_use]
    pub const fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }

    /// Writes the current `BC` pair.
    pub const fn set_bc(&mut self, value: u16) {
        [self.b, self.c] = value.to_be_bytes();
    }

    /// Returns the current `DE` pair.
    #[must_use]
    pub const fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }

    /// Writes the current `DE` pair.
    pub const fn set_de(&mut self, value: u16) {
        [self.d, self.e] = value.to_be_bytes();
    }

    /// Returns the current `HL` pair.
    #[must_use]
    pub const fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }

    /// Writes the current `HL` pair.
    pub const fn set_hl(&mut self, value: u16) {
        [self.h, self.l] = value.to_be_bytes();
    }

    /// Returns `A`.
    #[must_use]
    pub const fn a(&self) -> u8 {
        self.a
    }

    /// Writes `A`.
    pub const fn set_a(&mut self, value: u8) {
        self.a = value;
    }

    /// Returns `F`.
    #[must_use]
    pub const fn f(&self) -> u8 {
        self.f
    }

    /// Writes `F`.
    pub const fn set_f(&mut self, value: u8) {
        self.f = value;
    }

    /// Returns `IX`.
    #[must_use]
    pub const fn ix(&self) -> u16 {
        self.ix
    }

    /// Writes `IX`.
    pub const fn set_ix(&mut self, value: u16) {
        self.ix = value;
    }

    /// Returns `IY`.
    #[must_use]
    pub const fn iy(&self) -> u16 {
        self.iy
    }

    /// Writes `IY`.
    pub const fn set_iy(&mut self, value: u16) {
        self.iy = value;
    }

    /// Returns `SP`.
    #[must_use]
    pub const fn sp(&self) -> u16 {
        self.sp
    }

    /// Writes `SP`.
    pub const fn set_sp(&mut self, value: u16) {
        self.sp = value;
    }

    /// Returns `PC`.
    #[must_use]
    pub const fn pc(&self) -> u16 {
        self.pc
    }

    /// Writes `PC`.
    pub const fn set_pc(&mut self, value: u16) {
        self.pc = value;
    }

    /// Returns the interrupt vector base register `I`.
    #[must_use]
    pub const fn i(&self) -> u8 {
        self.i
    }

    /// Writes `I`.
    pub const fn set_i(&mut self, value: u8) {
        self.i = value;
    }

    /// Returns the memory refresh register `R`.
    #[must_use]
    pub const fn r(&self) -> u8 {
        self.r
    }

    /// Writes `R`; bit 7 is preserved across refresh increments.
    pub const fn set_r(&mut self, value: u8) {
        self.r = value;
    }

    /// Returns `IFF1` (interrupt enable flip-flop 1).
    #[must_use]
    pub const fn iff1(&self) -> bool {
        self.iff1
    }

    /// Writes `IFF1`.
    pub const fn set_iff1(&mut self, value: bool) {
        self.iff1 = value;
    }

    /// Returns `IFF2` (the NMI shadow of `IFF1`).
    #[must_use]
    pub const fn iff2(&self) -> bool {
        self.iff2
    }

    /// Writes `IFF2`.
    pub const fn set_iff2(&mut self, value: bool) {
        self.iff2 = value;
    }

    /// Returns the interrupt mode (0, 1 or 2).
    #[must_use]
    pub const fn im(&self) -> u8 {
        self.im
    }

    /// Returns whether the core is halted.
    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }

    /// Returns whether an `EI` acceptance delay is still pending.
    ///
    /// `EI` sets this latch; the instruction executed immediately after `EI`
    /// clears it at its end, which is the point where an interrupt may be
    /// accepted. Interrupt delivery itself is a later phase-8.4 task.
    #[must_use]
    pub const fn ei_delay_pending(&self) -> bool {
        self.ei_delay
    }

    /// Serializes the complete register file into [`SNAPSHOT_LEN`] bytes.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(SNAPSHOT_LEN);
        writer.write_bytes(&SNAPSHOT_MAGIC);
        writer.write_u8(SNAPSHOT_VERSION);
        writer.write_u8(self.a);
        writer.write_u8(self.f);
        writer.write_u8(self.b);
        writer.write_u8(self.c);
        writer.write_u8(self.d);
        writer.write_u8(self.e);
        writer.write_u8(self.h);
        writer.write_u8(self.l);
        writer.write_u8(self.a_alt);
        writer.write_u8(self.f_alt);
        writer.write_u8(self.b_alt);
        writer.write_u8(self.c_alt);
        writer.write_u8(self.d_alt);
        writer.write_u8(self.e_alt);
        writer.write_u8(self.h_alt);
        writer.write_u8(self.l_alt);
        writer.write_u16_le(self.ix);
        writer.write_u16_le(self.iy);
        writer.write_u16_le(self.sp);
        writer.write_u16_le(self.pc);
        writer.write_u8(self.i);
        writer.write_u8(self.r);
        writer.write_u8(u8::from(self.iff1));
        writer.write_u8(u8::from(self.iff2));
        writer.write_u8(self.im);
        writer.write_u8(u8::from(self.halted));
        writer.write_u8(u8::from(self.ei_delay));
        writer.write_u8(u8::from(self.interrupt_pending));
        writer.write_u8(self.interrupt_vector);
        writer.write_u8(u8::from(self.nmi_pending));
        writer.into_bytes()
    }

    /// Restores the register file from [`SNAPSHOT_LEN`] bytes.
    ///
    /// The snapshot is validated in full before any field is assigned, so a
    /// rejected input leaves the core untouched.
    ///
    /// # Errors
    ///
    /// Returns [`Z80Error::Snapshot`] for truncation or trailing bytes,
    /// [`Z80Error::SnapshotMagic`] for a foreign format,
    /// [`Z80Error::SnapshotVersion`] for an unsupported version and
    /// [`Z80Error::SnapshotField`] for a non-canonical flag or interrupt mode.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), Z80Error> {
        let mut reader = Reader::new(bytes);
        let magic = reader.read_array::<8>()?;
        if magic != SNAPSHOT_MAGIC {
            return Err(Z80Error::SnapshotMagic);
        }
        let version = reader.read_u8()?;
        if version != SNAPSHOT_VERSION {
            return Err(Z80Error::SnapshotVersion { found: version });
        }
        let a = reader.read_u8()?;
        let f = reader.read_u8()?;
        let b = reader.read_u8()?;
        let c = reader.read_u8()?;
        let d = reader.read_u8()?;
        let e = reader.read_u8()?;
        let h = reader.read_u8()?;
        let l = reader.read_u8()?;
        let a_alt = reader.read_u8()?;
        let f_alt = reader.read_u8()?;
        let b_alt = reader.read_u8()?;
        let c_alt = reader.read_u8()?;
        let d_alt = reader.read_u8()?;
        let e_alt = reader.read_u8()?;
        let h_alt = reader.read_u8()?;
        let l_alt = reader.read_u8()?;
        let ix = reader.read_u16_le()?;
        let iy = reader.read_u16_le()?;
        let sp = reader.read_u16_le()?;
        let pc = reader.read_u16_le()?;
        let i = reader.read_u8()?;
        let r = reader.read_u8()?;
        let iff1 = decode_flag(reader.read_u8()?, Z80Field::Iff1)?;
        let iff2 = decode_flag(reader.read_u8()?, Z80Field::Iff2)?;
        let im = reader.read_u8()?;
        if im > 2 {
            return Err(Z80Error::SnapshotField {
                field: Z80Field::InterruptMode,
            });
        }
        let halted = decode_flag(reader.read_u8()?, Z80Field::Halted)?;
        let ei_delay = decode_flag(reader.read_u8()?, Z80Field::EiDelay)?;
        let interrupt_pending = decode_flag(reader.read_u8()?, Z80Field::InterruptRequest)?;
        let interrupt_vector = reader.read_u8()?;
        let nmi_pending = decode_flag(reader.read_u8()?, Z80Field::NmiRequest)?;
        reader.finish()?;
        *self = Self {
            a,
            f,
            b,
            c,
            d,
            e,
            h,
            l,
            a_alt,
            f_alt,
            b_alt,
            c_alt,
            d_alt,
            e_alt,
            h_alt,
            l_alt,
            ix,
            iy,
            sp,
            pc,
            i,
            r,
            iff1,
            iff2,
            im,
            halted,
            ei_delay,
            interrupt_pending,
            interrupt_vector,
            nmi_pending,
        };
        Ok(())
    }

    /// Fetches one opcode byte and advances `R` as an M1 cycle does.
    pub(crate) fn fetch_opcode(&mut self, bus: &mut impl Z80Bus) -> Result<u8, Z80Error> {
        let opcode = bus.read8(self.pc)?;
        self.pc = self.pc.wrapping_add(1);
        self.increment_r();
        Ok(opcode)
    }

    /// Reads one immediate byte and advances `PC` without an M1 cycle.
    pub(crate) fn read_imm8(&mut self, bus: &mut impl Z80Bus) -> Result<u8, Z80Error> {
        let value = bus.read8(self.pc)?;
        self.pc = self.pc.wrapping_add(1);
        Ok(value)
    }

    /// Reads one little-endian immediate word and advances `PC`.
    pub(crate) fn read_imm16(&mut self, bus: &mut impl Z80Bus) -> Result<u16, Z80Error> {
        let low = self.read_imm8(bus)?;
        let high = self.read_imm8(bus)?;
        Ok(u16::from_le_bytes([low, high]))
    }

    /// Pushes `value`, leaving `SP` at its low byte.
    pub(crate) fn push16(&mut self, bus: &mut impl Z80Bus, value: u16) -> Result<(), Z80Error> {
        self.sp = self.sp.wrapping_sub(1);
        bus.write8(self.sp, (value >> 8) as u8)?;
        self.sp = self.sp.wrapping_sub(1);
        bus.write8(self.sp, value as u8)?;
        Ok(())
    }

    /// Pops one little-endian word from the stack.
    pub(crate) fn pop16(&mut self, bus: &mut impl Z80Bus) -> Result<u16, Z80Error> {
        let low = bus.read8(self.sp)?;
        self.sp = self.sp.wrapping_add(1);
        let high = bus.read8(self.sp)?;
        self.sp = self.sp.wrapping_add(1);
        Ok(u16::from_le_bytes([low, high]))
    }

    /// Returns byte register opcode index `0..=7`; `6` is `(HL)` and is routed
    /// through the bus by the instruction tables, never through this accessor.
    pub(crate) fn reg8(&self, code: u8) -> u8 {
        match code {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => self.h,
            5 => self.l,
            // Code 7 is `A`; code 6 denotes `(HL)` and only reaches this
            // accessor via a caller that already resolved the memory access.
            6 | 7 => self.a,
            _ => self.a,
        }
    }

    /// Writes byte register opcode index `0..=7`; see [`Z80::reg8`] for `6`.
    pub(crate) fn set_reg8(&mut self, code: u8, value: u8) {
        match code {
            0 => self.b = value,
            1 => self.c = value,
            2 => self.d = value,
            3 => self.e = value,
            4 => self.h = value,
            5 => self.l = value,
            _ => self.a = value,
        }
    }

    /// Returns register pair opcode index `0..=3` (`BC`, `DE`, `HL`, `SP`).
    pub(crate) fn read_pair(&self, code: u8) -> u16 {
        match code {
            0 => self.bc(),
            1 => self.de(),
            2 => self.hl(),
            _ => self.sp,
        }
    }

    /// Writes register pair opcode index `0..=3` (`BC`, `DE`, `HL`, `SP`).
    pub(crate) fn write_pair(&mut self, code: u8, value: u16) {
        match code {
            0 => self.set_bc(value),
            1 => self.set_de(value),
            2 => self.set_hl(value),
            _ => self.sp = value,
        }
    }

    /// Returns whether `mask` is set in `F`.
    pub(crate) const fn is_flag(&self, mask: u8) -> bool {
        self.f & mask != 0
    }

    /// Sets or clears `mask` in `F`.
    pub(crate) const fn set_flag(&mut self, mask: u8, value: bool) {
        if value {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
    }

    /// Sets `S`, `Z` and the undocumented `Y`/`X` copies from `value`.
    pub(crate) fn set_szxy(&mut self, value: u8) {
        self.set_flag(FLAG_S, value & 0x80 != 0);
        self.set_flag(FLAG_Z, value == 0);
        self.set_flag(FLAG_Y, value & 0x20 != 0);
        self.set_flag(FLAG_X, value & 0x08 != 0);
    }

    /// Sets `S`, `Z`, `Y`, `X` and parity/overflow from `value`.
    pub(crate) fn set_szxy_parity(&mut self, value: u8) {
        self.set_szxy(value);
        self.set_flag(FLAG_PV, value.count_ones() % 2 == 0);
    }

    /// Sets only the undocumented `Y`/`X` copies from `value`.
    pub(crate) const fn set_xy(&mut self, value: u8) {
        self.set_flag(FLAG_Y, value & 0x20 != 0);
        self.set_flag(FLAG_X, value & 0x08 != 0);
    }

    /// Evaluates condition code `0..=7` (`NZ`, `Z`, `NC`, `C`, `PO`, `PE`, `P`, `M`).
    pub(crate) const fn condition(&self, code: u8) -> bool {
        match code {
            0 => !self.is_flag(FLAG_Z),
            1 => self.is_flag(FLAG_Z),
            2 => !self.is_flag(FLAG_C),
            3 => self.is_flag(FLAG_C),
            4 => !self.is_flag(FLAG_PV),
            5 => self.is_flag(FLAG_PV),
            6 => !self.is_flag(FLAG_S),
            _ => self.is_flag(FLAG_S),
        }
    }

    /// Increments the low seven bits of `R`; bit 7 is preserved.
    fn increment_r(&mut self) {
        self.r = (self.r & 0x80) | (self.r.wrapping_add(1) & 0x7F);
    }
}

/// Decodes a single boolean byte from a snapshot.
fn decode_flag(byte: u8, field: Z80Field) -> Result<bool, Z80Error> {
    match byte {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Z80Error::SnapshotField { field }),
    }
}
