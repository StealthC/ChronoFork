//! Minimal deterministic Sega Mega Drive VDP status/control/VRAM slice.
//!
//! **Status:** partial VDP emulation, not a rendering engine. This module
//! implements only the parts of the 315-5313 VDP required by the current
//! backend slice.
//!
//! **Implemented:**
//!
//! - 64 KiB of flat VRAM (`0x0000..=0xFFFF`), stored big-endian.
//! - The VDP port window at offsets `0x00..=0x0F` relative to `0xC00000`:
//!   `0x00..=0x03` is the data port (word accesses at `0x00`/`0x02`),
//!   `0x04..=0x07` is the control port (word accesses at `0x04`/`0x06`) and
//!   `0x08..=0x0F` is the HV counter. HV-counter reads return zero because no
//!   raster timing is modelled; HV-counter writes are rejected.
//! - Control-port two-word command setup. The first word latches command bits
//!   `1..=0` (bits `15..=14`) and address bits `13..=0`; the second word
//!   latches command bits `5..=2` (bits `7..=4`) and address bits `15..=14`
//!   (bits `1..=0`).
//! - Control-port register writes (`0x8000 | register << 8 | value`) for
//!   registers `0x00..=0x17`.
//! - Data-port VRAM word reads and writes with auto-increment from register
//!   `0x0F`. Byte accesses transfer a full word: byte writes duplicate the
//!   byte into both halves (the documented VDP data-port byte-write
//!   behaviour), and byte reads return the requested byte lane of the
//!   transferred word.
//! - The status-register bits covered by [`STATUS_MASK`]. VBlank is controlled
//!   by [`Vdp::set_vblank`]. FIFO-empty is always set because data writes
//!   complete immediately and no write FIFO is modelled. Every other status
//!   bit is a documented constant with no hardware behind it yet.
//! - Versioned, fixed-length snapshots through [`Vdp::snapshot_bytes`] and
//!   [`Vdp::restore_bytes`].
//!
//! **Not implemented (and not claimed):** rendering, planes, window, sprites,
//! scrolling, CRAM, VSRAM, DMA, FIFO/access timing, H/V counters, interrupts,
//! Z80/PSG interaction, audio, the byte-swapped/interleaved physical VRAM
//! layout, and cycle-accurate port timing.
//!
//! **Deterministic reset state:** VRAM is zeroed and all 24 registers are
//! zero. In particular register `0x0F` (auto-increment) starts at zero, so a
//! program must write it before relying on address advancement.

use std::error::Error;
use std::fmt;

/// Size of VDP VRAM in bytes.
pub const VRAM_SIZE: usize = 0x0001_0000;
/// Number of VDP registers implemented by this slice (`0x00..=0x17`).
pub const REGISTER_COUNT: usize = 0x18;
/// VDP register that holds the data-port auto-increment value.
pub const AUTO_INCREMENT_REGISTER: usize = 0x0F;
/// Number of port bytes covered by this slice (`0x00..=0x0F`).
pub const PORT_WINDOW_SIZE: usize = 0x10;
/// Offset of the data port relative to `0xC00000`.
pub const DATA_PORT_OFFSET: u8 = 0x00;
/// Offset of the control port relative to `0xC00000`.
pub const CONTROL_PORT_OFFSET: u8 = 0x04;
/// Offset of the HV-counter port relative to `0xC00000`.
pub const HV_COUNTER_OFFSET: u8 = 0x08;

/// PAL region flag (status bit 0). Always 0: region is not modelled.
pub const STATUS_PAL: u16 = 0x0001;
/// DMA-busy flag (status bit 1). Always 0: DMA is not implemented.
pub const STATUS_DMA_BUSY: u16 = 0x0002;
/// HBlank flag (status bit 2). Always 0: no raster timing.
pub const STATUS_HBLANK: u16 = 0x0004;
/// VBlank flag (status bit 3). Controlled by [`Vdp::set_vblank`].
pub const STATUS_VBLANK: u16 = 0x0008;
/// Odd interlace frame flag (status bit 4). Always 0: no interlace.
pub const STATUS_ODD_FRAME: u16 = 0x0010;
/// Sprite-collision flag (status bit 5). Always 0: no sprites.
pub const STATUS_SPRITE_COLLISION: u16 = 0x0020;
/// Sprite-overflow flag (status bit 6). Always 0: no sprites.
pub const STATUS_SPRITE_OVERFLOW: u16 = 0x0040;
/// VBlank-interrupt-pending flag (status bit 7). Always 0: the external
/// interrupt scheduler is not wired into the VDP status register yet.
pub const STATUS_VINT_PENDING: u16 = 0x0080;
/// FIFO-empty flag (status bit 8). Always set: no write FIFO is modelled, so
/// every accepted data-port write has already completed.
pub const STATUS_FIFO_EMPTY: u16 = 0x0100;
/// FIFO-full flag (status bit 9). Always 0: no write FIFO is modelled.
pub const STATUS_FIFO_FULL: u16 = 0x0200;
/// Mask of the ten status bits defined by the VDP. The upper six bits read as
/// zero here; real hardware leaves them floating.
pub const STATUS_MASK: u16 = 0x03FF;

/// Magic prefix of a serialized VDP snapshot.
pub const SNAPSHOT_MAGIC: [u8; 8] = *b"CFVDP001";
/// Version of the serialized VDP snapshot format.
pub const SNAPSHOT_VERSION: u16 = 1;
/// Exact byte length of a serialized VDP snapshot.
pub const SNAPSHOT_LEN: usize = 8 + 2 + 1 + 1 + 2 + REGISTER_COUNT + VRAM_SIZE;

const SNAPSHOT_FLAG_VBLANK: u8 = 1 << 0;
const SNAPSHOT_FLAG_COMMAND_PENDING: u8 = 1 << 1;
const SNAPSHOT_FLAG_MASK: u8 = SNAPSHOT_FLAG_VBLANK | SNAPSHOT_FLAG_COMMAND_PENDING;
const SNAPSHOT_HEADER_LEN: usize = 8 + 2 + 1 + 1 + 2 + REGISTER_COUNT;

// Command-code bits. CD0/CD1 are latched from the first control word, CD2..CD5
// from the second. CD4 inhibits read targets after a partial command; CD5 flags
// a DMA transfer request.
const CODE_READ_INHIBIT: u8 = 0x10;
const CODE_DMA: u8 = 0x20;
const CODE_VRAM_READ: u8 = 0x00;
const CODE_VRAM_WRITE: u8 = 0x01;
const CODE_REGISTER_WRITE: u8 = 0x02;

/// A VDP port access or snapshot that this slice cannot satisfy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VdpError {
    /// A port offset outside `0x00..=0x0F`.
    UnsupportedOffset {
        /// Requested port offset.
        offset: u8,
    },
    /// A word access at an odd port offset.
    OddOffset {
        /// Requested port offset.
        offset: u8,
    },
    /// A write to the read-only HV-counter port.
    UnsupportedWrite {
        /// Requested port offset.
        offset: u8,
    },
    /// A VDP register number outside `0x00..=0x17`.
    UnsupportedRegister {
        /// Requested register number.
        register: u8,
    },
    /// A data-port access whose command code does not select implemented VRAM
    /// read or write behaviour (including CRAM/VSRAM and half-completed
    /// commands).
    UnsupportedTarget {
        /// Current six-bit command code.
        code: u8,
    },
    /// A data-port access while the command code requests DMA.
    DmaUnsupported {
        /// Current six-bit command code.
        code: u8,
    },
    /// A snapshot whose length is not [`SNAPSHOT_LEN`].
    SnapshotLengthMismatch {
        /// Required length.
        expected: usize,
        /// Supplied length.
        found: usize,
    },
    /// A snapshot without the expected magic prefix.
    SnapshotMagicMismatch,
    /// A snapshot with an unsupported format version.
    SnapshotVersionUnsupported {
        /// Version found in the snapshot header.
        found: u16,
    },
    /// A snapshot whose flag byte has reserved bits set.
    SnapshotFlagsInvalid {
        /// Flag byte found in the snapshot header.
        flags: u8,
    },
    /// A snapshot whose command code exceeds six bits.
    SnapshotCodeInvalid {
        /// Command code found in the snapshot header.
        code: u8,
    },
}

impl fmt::Display for VdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOffset { offset } => write!(
                f,
                "VDP access at unsupported offset 0x{offset:02X} (window is 0x00..=0x0F)"
            ),
            Self::OddOffset { offset } => {
                write!(f, "odd VDP word access at offset 0x{offset:02X}")
            }
            Self::UnsupportedWrite { offset } => {
                write!(f, "write to read-only VDP HV-counter offset 0x{offset:02X}")
            }
            Self::UnsupportedRegister { register } => write!(
                f,
                "VDP register 0x{register:02X} is outside the implemented 0x00..=0x17 range"
            ),
            Self::UnsupportedTarget { code } => write!(
                f,
                "VDP data-port access with unsupported command code 0x{code:02X}"
            ),
            Self::DmaUnsupported { code } => write!(
                f,
                "VDP data-port access with DMA command code 0x{code:02X}; DMA is not implemented"
            ),
            Self::SnapshotLengthMismatch { expected, found } => {
                write!(f, "VDP snapshot is {found} bytes, expected {expected}")
            }
            Self::SnapshotMagicMismatch => f.write_str("VDP snapshot magic mismatch"),
            Self::SnapshotVersionUnsupported { found } => {
                write!(f, "VDP snapshot version {found} is not supported")
            }
            Self::SnapshotFlagsInvalid { flags } => {
                write!(
                    f,
                    "VDP snapshot flag byte 0x{flags:02X} has reserved bits set"
                )
            }
            Self::SnapshotCodeInvalid { code } => {
                write!(f, "VDP snapshot command code 0x{code:02X} exceeds six bits")
            }
        }
    }
}

impl Error for VdpError {}

/// Partial 315-5313 VDP: control port, status register and 64 KiB VRAM.
///
/// Accesses are made through port offsets relative to `0xC00000`, matching the
/// big-endian 16-bit bus of the Mega Drive. All state changes are confined to
/// the methods on this type; there is no hidden global state and no host clock
/// input.
#[derive(Clone)]
pub struct Vdp {
    vram: Vec<u8>,
    registers: [u8; REGISTER_COUNT],
    code: u8,
    address: u16,
    command_pending: bool,
    vblank: bool,
}

impl Vdp {
    /// Creates a VDP in the deterministic reset state described by the module
    /// documentation: zeroed VRAM, zeroed registers and no pending command.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_SIZE],
            registers: [0; REGISTER_COUNT],
            code: 0,
            address: 0,
            command_pending: false,
            vblank: false,
        }
    }

    /// Returns the ten-bit status register.
    ///
    /// Only VBlank and FIFO-empty can be set by this slice; the remaining bits
    /// are documented constants. Upper bits are zero rather than the floating
    /// values real hardware returns.
    #[must_use]
    pub fn status(&self) -> u16 {
        let mut status = STATUS_FIFO_EMPTY;
        if self.vblank {
            status |= STATUS_VBLANK;
        }
        status
    }

    /// Sets or clears the VBlank status flag.
    ///
    /// This is the only status source that can change in this slice. A future
    /// interrupt scheduler is expected to drive it from its deterministic
    /// virtual clock.
    pub fn set_vblank(&mut self, active: bool) {
        self.vblank = active;
    }

    /// Returns the flat 64 KiB VRAM contents.
    #[must_use]
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// Returns one VDP register, or `None` when `index` is outside
    /// `0x00..=0x17`.
    #[must_use]
    pub fn register(&self, index: usize) -> Option<u8> {
        self.registers.get(index).copied()
    }

    /// Returns all 24 implemented VDP registers.
    #[must_use]
    pub const fn registers(&self) -> &[u8; REGISTER_COUNT] {
        &self.registers
    }

    /// Returns the current 16-bit command/address register.
    #[must_use]
    pub const fn address(&self) -> u16 {
        self.address
    }

    /// Reads one byte from the VDP port at `offset`.
    ///
    /// Even offsets select the high byte and odd offsets the low byte of the
    /// word returned by [`Vdp::read16`] semantics. A data-port byte read still
    /// transfers a whole word and applies auto-increment, matching the VDP's
    /// word-oriented data port.
    pub fn read8(&mut self, offset: u8) -> Result<u8, VdpError> {
        Self::validate_offset(offset)?;
        let word = match offset & 0x0C {
            0x00 => self.read_data_port()?,
            0x04 => self.read_control_port(),
            _ => 0,
        };
        if offset & 1 == 0 {
            Ok((word >> 8) as u8)
        } else {
            Ok(word as u8)
        }
    }

    /// Writes one byte to the VDP port at `offset`.
    ///
    /// Byte writes to the data and control ports duplicate the byte into both
    /// halves of the 16-bit port word before it is processed. This matches the
    /// documented data-port byte-write behaviour and gives control-port byte
    /// writes a deterministic meaning. HV-counter writes are rejected, and a
    /// data-port byte write while only the first word of a two-word command is
    /// latched fails with [`VdpError::UnsupportedTarget`] without touching
    /// VRAM.
    pub fn write8(&mut self, offset: u8, value: u8) -> Result<(), VdpError> {
        Self::validate_offset(offset)?;
        let word = (u16::from(value) << 8) | u16::from(value);
        match offset & 0x0C {
            0x00 => self.write_data_port(word),
            0x04 => self.write_control_port(word),
            _ => Err(VdpError::UnsupportedWrite { offset }),
        }
    }

    /// Reads a big-endian word from the VDP port at `offset`.
    ///
    /// Word accesses require an even offset. Data-port reads use the command
    /// code set up through the control port and apply auto-increment; reads
    /// from the control port return the status register and cancel any
    /// half-completed command; HV-counter reads return zero.
    pub fn read16(&mut self, offset: u8) -> Result<u16, VdpError> {
        Self::validate_offset(offset)?;
        if offset & 1 != 0 {
            return Err(VdpError::OddOffset { offset });
        }
        match offset & 0x0C {
            0x00 => self.read_data_port(),
            0x04 => Ok(self.read_control_port()),
            _ => Ok(0),
        }
    }

    /// Writes a big-endian word to the VDP port at `offset`.
    ///
    /// Word accesses require an even offset. Data-port writes use the command
    /// code set up through the control port and apply auto-increment; a
    /// data-port write while only the first word of a two-word command is
    /// latched fails with [`VdpError::UnsupportedTarget`] without touching
    /// VRAM. Control-port writes implement register selection and the two-word
    /// command setup described in the module documentation. HV-counter writes
    /// are rejected.
    pub fn write16(&mut self, offset: u8, value: u16) -> Result<(), VdpError> {
        Self::validate_offset(offset)?;
        if offset & 1 != 0 {
            return Err(VdpError::OddOffset { offset });
        }
        match offset & 0x0C {
            0x00 => self.write_data_port(value),
            0x04 => self.write_control_port(value),
            _ => Err(VdpError::UnsupportedWrite { offset }),
        }
    }

    /// Serializes the complete VDP state into [`SNAPSHOT_LEN`] bytes.
    ///
    /// Layout, in order: 8-byte magic, little-endian `u16` format version,
    /// one flag byte (bit 0 VBlank, bit 1 pending command half), one command
    /// code byte, little-endian `u16` address, 24 registers and 64 KiB of
    /// VRAM. The format is versioned and self-describing; consumers must not
    /// reinterpret bytes from another version.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(SNAPSHOT_LEN);
        bytes.extend_from_slice(&SNAPSHOT_MAGIC);
        bytes.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        let mut flags = 0;
        if self.vblank {
            flags |= SNAPSHOT_FLAG_VBLANK;
        }
        if self.command_pending {
            flags |= SNAPSHOT_FLAG_COMMAND_PENDING;
        }
        bytes.push(flags);
        bytes.push(self.code);
        bytes.extend_from_slice(&self.address.to_le_bytes());
        bytes.extend_from_slice(&self.registers);
        bytes.extend_from_slice(&self.vram);
        bytes
    }

    /// Restores state from a [`Vdp::snapshot_bytes`] payload.
    ///
    /// Length, magic, version, reserved flag bits and the command-code width
    /// are validated before any field is mutated, so a rejected snapshot
    /// leaves the VDP exactly as it was.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), VdpError> {
        if bytes.len() != SNAPSHOT_LEN {
            return Err(VdpError::SnapshotLengthMismatch {
                expected: SNAPSHOT_LEN,
                found: bytes.len(),
            });
        }
        if &bytes[..SNAPSHOT_MAGIC.len()] != SNAPSHOT_MAGIC.as_slice() {
            return Err(VdpError::SnapshotMagicMismatch);
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != SNAPSHOT_VERSION {
            return Err(VdpError::SnapshotVersionUnsupported { found: version });
        }
        let flags = bytes[10];
        if flags & !SNAPSHOT_FLAG_MASK != 0 {
            return Err(VdpError::SnapshotFlagsInvalid { flags });
        }
        let code = bytes[11];
        if code & !0x3F != 0 {
            return Err(VdpError::SnapshotCodeInvalid { code });
        }
        let address = u16::from_le_bytes([bytes[12], bytes[13]]);
        let mut registers = [0; REGISTER_COUNT];
        registers.copy_from_slice(&bytes[14..14 + REGISTER_COUNT]);

        self.vblank = flags & SNAPSHOT_FLAG_VBLANK != 0;
        self.command_pending = flags & SNAPSHOT_FLAG_COMMAND_PENDING != 0;
        self.code = code;
        self.address = address;
        self.registers = registers;
        self.vram.copy_from_slice(&bytes[SNAPSHOT_HEADER_LEN..]);
        Ok(())
    }

    fn validate_offset(offset: u8) -> Result<(), VdpError> {
        if usize::from(offset) >= PORT_WINDOW_SIZE {
            return Err(VdpError::UnsupportedOffset { offset });
        }
        Ok(())
    }

    fn read_data_port(&mut self) -> Result<u16, VdpError> {
        self.command_pending = false;
        if self.code & CODE_DMA != 0 {
            return Err(VdpError::DmaUnsupported { code: self.code });
        }
        if self.code & 0x0F != CODE_VRAM_READ || self.code & CODE_READ_INHIBIT != 0 {
            return Err(VdpError::UnsupportedTarget { code: self.code });
        }
        // VRAM word accesses align down to an even byte address. The physical
        // byte-swapped/interleaved layout and odd-address byte-ordering quirk
        // are not modelled.
        let index = usize::from(self.address & 0xFFFE);
        let word = u16::from_be_bytes([self.vram[index], self.vram[index + 1]]);
        self.increment_address();
        Ok(word)
    }

    fn write_data_port(&mut self, value: u16) -> Result<(), VdpError> {
        if self.command_pending {
            // Only the first word of a two-word control command has been
            // latched, so the write target and address are not fully decoded
            // yet. Reject the access without mutating VRAM; the half-latched
            // command stays pending and can still be completed by its second
            // control word.
            return Err(VdpError::UnsupportedTarget { code: self.code });
        }
        if self.code & CODE_DMA != 0 {
            return Err(VdpError::DmaUnsupported { code: self.code });
        }
        if self.code & 0x0F != CODE_VRAM_WRITE {
            return Err(VdpError::UnsupportedTarget { code: self.code });
        }
        let index = usize::from(self.address & 0xFFFE);
        let bytes = value.to_be_bytes();
        self.vram[index] = bytes[0];
        self.vram[index + 1] = bytes[1];
        self.increment_address();
        Ok(())
    }

    fn read_control_port(&mut self) -> u16 {
        self.command_pending = false;
        self.status()
    }

    fn write_control_port(&mut self, word: u16) -> Result<(), VdpError> {
        if self.command_pending {
            self.code = (self.code & 0x03) | (((word >> 4) as u8 & 0x0F) << 2);
            self.address |= (word & 0x0003) << 14;
            self.command_pending = false;
            return Ok(());
        }

        let low_code = ((word >> 14) & 0x03) as u8;
        if low_code == CODE_REGISTER_WRITE {
            // Register write: 1 0 RRRRR DDDDDDDD. The register range is
            // validated before any state, including the latched command
            // registers, is mutated.
            let register = ((word >> 8) & 0x1F) as u8;
            let index = usize::from(register);
            if index >= REGISTER_COUNT {
                return Err(VdpError::UnsupportedRegister { register });
            }
            // Real hardware latches the first command word before performing
            // the register write, leaving the read-inhibit bit set and the
            // data-port target invalid until a new address is set up.
            self.code = low_code | CODE_READ_INHIBIT;
            self.address = word & 0x3FFF;
            self.registers[index] = (word & 0xFF) as u8;
            self.command_pending = false;
            return Ok(());
        }

        self.code = low_code | CODE_READ_INHIBIT;
        self.address = word & 0x3FFF;
        self.command_pending = true;
        Ok(())
    }

    fn increment_address(&mut self) {
        let increment = u16::from(self.registers[AUTO_INCREMENT_REGISTER]);
        self.address = self.address.wrapping_add(increment);
    }
}

impl Default for Vdp {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Vdp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vdp")
            .field("vblank", &self.vblank)
            .field("command_pending", &self.command_pending)
            .field("code", &format_args!("0x{:02X}", self.code))
            .field("address", &format_args!("0x{:04X}", self.address))
            .field("registers", &self.registers)
            .field("vram_len", &self.vram.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_vram_address(vdp: &mut Vdp, address: u16, code: u16) -> Result<(), VdpError> {
        vdp.write16(CONTROL_PORT_OFFSET, code | (address & 0x3FFF))?;
        vdp.write16(CONTROL_PORT_OFFSET, (address >> 14) & 0x0003)
    }

    fn set_vram_write_address(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
        set_vram_address(vdp, address, 0x4000)
    }

    fn set_vram_read_address(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
        set_vram_address(vdp, address, 0x0000)
    }

    #[test]
    fn register_writes_select_register_and_value() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8004)
            .expect("register 0 write");
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("register 15 write");

        assert_eq!(vdp.register(0), Some(0x04));
        assert_eq!(vdp.register(15), Some(0x02));
        assert_eq!(vdp.register(REGISTER_COUNT), None);
        for index in 1..REGISTER_COUNT {
            if index != AUTO_INCREMENT_REGISTER {
                assert_eq!(vdp.register(index), Some(0));
            }
        }
    }

    #[test]
    fn register_write_uses_mirror_and_rejects_unknown_register() {
        let mut vdp = Vdp::new();
        let mut expected = [0_u8; REGISTER_COUNT];
        vdp.write16(CONTROL_PORT_OFFSET + 2, 0x8F01)
            .expect("register write through mirror");
        expected[AUTO_INCREMENT_REGISTER] = 0x01;
        assert_eq!(vdp.registers(), &expected);

        assert_eq!(
            vdp.write16(CONTROL_PORT_OFFSET, 0x9810),
            Err(VdpError::UnsupportedRegister { register: 0x18 })
        );
        assert_eq!(vdp.registers(), &expected);
        // A register write also latches the command word as its address, as on
        // real hardware; the rejected register write must not change that.
        assert_eq!(vdp.address(), 0x0F01);
    }

    #[test]
    fn vram_writes_follow_auto_increment_and_wrap() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        set_vram_write_address(&mut vdp, 0x0100).expect("vram write address");
        assert_eq!(vdp.address(), 0x0100);

        vdp.write16(DATA_PORT_OFFSET, 0x1234).expect("first word");
        vdp.write16(DATA_PORT_OFFSET, 0xABCD).expect("second word");
        assert_eq!(&vdp.vram()[0x0100..0x0104], &[0x12, 0x34, 0xAB, 0xCD]);
        assert_eq!(vdp.address(), 0x0104);

        // The data port mirrors at 0x02 and auto-increment wraps at 64 KiB.
        vdp.write16(DATA_PORT_OFFSET + 2, 0x5678)
            .expect("mirrored word");
        assert_eq!(&vdp.vram()[0x0104..0x0106], &[0x56, 0x78]);
        assert_eq!(vdp.address(), 0x0106);

        set_vram_write_address(&mut vdp, 0xFFFE).expect("top of vram");
        vdp.write16(DATA_PORT_OFFSET, 0xBEEF)
            .expect("wrapping word");
        assert_eq!(&vdp.vram()[0xFFFE..], &[0xBE, 0xEF]);
        assert_eq!(vdp.address(), 0x0000);
    }

    #[test]
    fn vram_word_and_byte_reads_follow_command_code() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        set_vram_write_address(&mut vdp, 0x0200).expect("vram write address");
        vdp.write16(DATA_PORT_OFFSET, 0x1111).expect("first word");
        vdp.write16(DATA_PORT_OFFSET, 0x2222).expect("second word");

        set_vram_read_address(&mut vdp, 0x0200).expect("vram read address");
        assert_eq!(vdp.read16(DATA_PORT_OFFSET).expect("word read"), 0x1111);
        assert_eq!(vdp.read16(DATA_PORT_OFFSET).expect("word read"), 0x2222);
        assert_eq!(vdp.address(), 0x0204);

        // A byte read still transfers a whole word; the lane selects which
        // byte of that word is returned.
        set_vram_read_address(&mut vdp, 0x0200).expect("vram read address");
        assert_eq!(vdp.read8(DATA_PORT_OFFSET).expect("high byte"), 0x11);
        assert_eq!(vdp.read8(DATA_PORT_OFFSET + 1).expect("low byte"), 0x22);
        assert_eq!(vdp.address(), 0x0204);
    }

    #[test]
    fn data_port_byte_writes_duplicate_into_both_halves() {
        let mut vdp = Vdp::new();
        set_vram_write_address(&mut vdp, 0x0300).expect("vram write address");
        vdp.write8(DATA_PORT_OFFSET, 0x5A).expect("byte write");
        assert_eq!(&vdp.vram()[0x0300..0x0302], &[0x5A, 0x5A]);
        assert_eq!(vdp.address(), 0x0300);
    }

    #[test]
    fn status_reports_vblank_and_fifo_empty() {
        let mut vdp = Vdp::new();
        assert_eq!(vdp.status(), STATUS_FIFO_EMPTY);
        assert_eq!(vdp.status() & !STATUS_MASK, 0);
        assert_eq!(vdp.status() & STATUS_VBLANK, 0);

        vdp.set_vblank(true);
        assert_eq!(vdp.status(), STATUS_FIFO_EMPTY | STATUS_VBLANK);
        assert_eq!(
            vdp.read16(CONTROL_PORT_OFFSET).expect("status read"),
            STATUS_FIFO_EMPTY | STATUS_VBLANK
        );
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET).expect("high status byte"),
            0x01
        );
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET + 1).expect("low status byte"),
            0x08
        );

        vdp.set_vblank(false);
        assert_eq!(vdp.status(), STATUS_FIFO_EMPTY);
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET + 1).expect("low status byte"),
            0x00
        );
    }

    #[test]
    fn hv_counter_reads_are_deterministic_and_writes_rejected() {
        let mut vdp = Vdp::new();
        assert_eq!(vdp.read16(HV_COUNTER_OFFSET).expect("hv read"), 0);
        assert_eq!(vdp.read16(0x0E).expect("hv mirror read"), 0);
        assert_eq!(vdp.read8(0x09).expect("hv byte read"), 0);
        assert_eq!(
            vdp.write16(HV_COUNTER_OFFSET, 0),
            Err(VdpError::UnsupportedWrite {
                offset: HV_COUNTER_OFFSET
            })
        );
        assert_eq!(
            vdp.write8(0x0C, 0),
            Err(VdpError::UnsupportedWrite { offset: 0x0C })
        );
    }

    #[test]
    fn unsupported_port_accesses_are_typed_errors() {
        let mut vdp = Vdp::new();
        assert_eq!(
            vdp.read16(PORT_WINDOW_SIZE as u8),
            Err(VdpError::UnsupportedOffset {
                offset: PORT_WINDOW_SIZE as u8
            })
        );
        assert_eq!(
            vdp.write8(0x10, 0),
            Err(VdpError::UnsupportedOffset { offset: 0x10 })
        );
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET + 1),
            Err(VdpError::OddOffset { offset: 1 })
        );
        assert_eq!(
            vdp.write16(CONTROL_PORT_OFFSET + 1, 0),
            Err(VdpError::OddOffset { offset: 5 })
        );
    }

    #[test]
    fn data_port_rejects_unsupported_command_targets() {
        let mut vdp = Vdp::new();
        // CRAM write command: CD3..CD0 = 0x3.
        vdp.write16(CONTROL_PORT_OFFSET, 0xC000)
            .expect("cram first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("cram second half");
        assert_eq!(
            vdp.write16(DATA_PORT_OFFSET, 0),
            Err(VdpError::UnsupportedTarget { code: 0x03 })
        );

        // DMA transfer: CD5 set in the second command word.
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("dma first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0080)
            .expect("dma second half");
        assert_eq!(
            vdp.write16(DATA_PORT_OFFSET, 0),
            Err(VdpError::DmaUnsupported { code: 0x21 })
        );

        // A register write leaves the data-port target invalid.
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("register write");
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET),
            Err(VdpError::UnsupportedTarget { code: 0x12 })
        );

        // A half-completed read command inhibits reads.
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("read first half");
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET),
            Err(VdpError::UnsupportedTarget { code: 0x10 })
        );
    }

    #[test]
    fn data_write_rejects_half_completed_command() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000 | 0x0100)
            .expect("first command half");
        assert_eq!(vdp.address(), 0x0100);

        let before = vdp.vram().to_vec();
        // Only the first control word is latched (code 0x11), so the write
        // target and address are not fully decoded: reject both word and byte
        // data writes without altering VRAM or the latched address.
        assert_eq!(
            vdp.write16(DATA_PORT_OFFSET, 0xDEAD),
            Err(VdpError::UnsupportedTarget { code: 0x11 })
        );
        assert_eq!(
            vdp.write8(DATA_PORT_OFFSET, 0x5A),
            Err(VdpError::UnsupportedTarget { code: 0x11 })
        );
        assert_eq!(vdp.vram(), before.as_slice());
        assert_eq!(vdp.address(), 0x0100);

        // The rejected accesses did not cancel the half-latched command, so
        // completing it still writes normally.
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("second command half");
        vdp.write16(DATA_PORT_OFFSET, 0x1234)
            .expect("completed command write");
        assert_eq!(&vdp.vram()[0x0100..0x0102], &[0x12, 0x34]);
        assert_eq!(vdp.address(), 0x0102);
    }

    #[test]
    fn snapshot_round_trips_complete_state() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8014)
            .expect("register 0 write");
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        set_vram_write_address(&mut vdp, 0x1234).expect("vram write address");
        vdp.write16(DATA_PORT_OFFSET, 0xCAFE).expect("vram write");
        vdp.set_vblank(true);

        let snapshot = vdp.snapshot_bytes();
        assert_eq!(snapshot.len(), SNAPSHOT_LEN);

        let mut restored = Vdp::new();
        restored
            .write16(CONTROL_PORT_OFFSET, 0x8F00)
            .expect("different auto-increment");
        restored.restore_bytes(&snapshot).expect("restore snapshot");

        assert_eq!(restored.snapshot_bytes(), snapshot);
        assert_eq!(restored.status(), vdp.status());
        assert_eq!(restored.registers(), vdp.registers());
        assert_eq!(restored.address(), vdp.address());
        assert_eq!(restored.vram(), vdp.vram());
        assert_eq!(&restored.vram()[0x1234..0x1236], &[0xCA, 0xFE]);
    }

    #[test]
    fn snapshot_preserves_half_completed_command() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("first command half");
        let snapshot = vdp.snapshot_bytes();

        let mut restored = Vdp::new();
        restored.restore_bytes(&snapshot).expect("restore snapshot");
        restored
            .write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("second command half");
        restored
            .write16(DATA_PORT_OFFSET, 0x1234)
            .expect("vram write");
        assert_eq!(&restored.vram()[0..2], &[0x12, 0x34]);
        assert_eq!(restored.address(), 0x0002);
    }

    #[test]
    fn invalid_snapshots_are_rejected_without_mutation() {
        let good = Vdp::new().snapshot_bytes();
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        vdp.set_vblank(true);
        let before = vdp.snapshot_bytes();

        assert_eq!(
            vdp.restore_bytes(&good[..good.len() - 1]),
            Err(VdpError::SnapshotLengthMismatch {
                expected: SNAPSHOT_LEN,
                found: SNAPSHOT_LEN - 1
            })
        );
        assert_eq!(
            vdp.restore_bytes(&[]),
            Err(VdpError::SnapshotLengthMismatch {
                expected: SNAPSHOT_LEN,
                found: 0
            })
        );

        let mut bad_magic = good.clone();
        bad_magic[0] ^= 0xFF;
        assert_eq!(
            vdp.restore_bytes(&bad_magic),
            Err(VdpError::SnapshotMagicMismatch)
        );

        let mut bad_version = good.clone();
        bad_version[8] = SNAPSHOT_VERSION as u8 + 1;
        assert_eq!(
            vdp.restore_bytes(&bad_version),
            Err(VdpError::SnapshotVersionUnsupported {
                found: SNAPSHOT_VERSION + 1
            })
        );

        let mut bad_flags = good.clone();
        bad_flags[10] = 0x80;
        assert_eq!(
            vdp.restore_bytes(&bad_flags),
            Err(VdpError::SnapshotFlagsInvalid { flags: 0x80 })
        );

        let mut bad_code = good.clone();
        bad_code[11] = 0x40;
        assert_eq!(
            vdp.restore_bytes(&bad_code),
            Err(VdpError::SnapshotCodeInvalid { code: 0x40 })
        );

        assert_eq!(vdp.snapshot_bytes(), before);
    }
}
