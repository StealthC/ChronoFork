//! Modelled Mega Drive cartridge: header, Sega mapper, battery SRAM and TMSS.
//!
//! The cartridge owns the ROM image, its parsed header, the mapper register
//! file, the 64 KiB battery SRAM and the TMSS latch. Everything here is
//! deterministic: no host time, environment or filesystem input reaches canonical
//! state, and the only host policy seam is [`Cartridge::import_sram`], which a
//! caller may use at machine construction to load a save file.
//!
//! # Address windows
//!
//! * `0x000000..0x3FFFFF` is the 68000 cartridge window. Bank 0 is fixed to the
//!   first 512 KiB; windows `1..=6` select a 512 KiB page through the nine-bit
//!   mapper registers `0xA130F3`, `0xA130F5`, ..., `0xA130FF`.
//! * `0x00200000..0x0020FFFF` is the 64 KiB battery-SRAM window while bit 0 of
//!   `0xA130F1` is set; bit 1 write-protects it.
//! * `0xA130F0..0xA130FF` is the write-only mapper register file. The documented
//!   registers select banks and SRAM visibility/protection; other bytes are
//!   acknowledged and ignored, and the whole window reads as unmapped.
//! * `0xA14000..0xA14003` is the TMSS register. A long write whose bit 9 is set
//!   (`"SEGA"` does) unlocks VDP writes when TMSS is enabled.
//!
//! The mapper is always present and defaults to identity, so images at or below
//! 4 MiB behave exactly like a plain linear ROM.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::bus::ROM_END;
use crate::interrupts::VideoStandard;

/// First address of the mapper register window.
pub const MAPPER_REGISTER_BASE: u32 = 0x00A1_30F0;
/// Exclusive end of the mapper register window.
pub const MAPPER_REGISTER_END: u32 = MAPPER_REGISTER_BASE + 0x0010;
/// First address of the battery-SRAM window.
pub const SRAM_BASE: u32 = 0x0020_0000;
/// Inclusive last address of the battery-SRAM window.
pub const SRAM_END: u32 = 0x0020_FFFF;
/// Size of the canonical battery SRAM in bytes.
pub const SRAM_SIZE: usize = 65_536;
/// First address of the TMSS register.
pub const TMSS_REGISTER_BASE: u32 = 0x00A1_4000;
/// Inclusive last address of the TMSS register.
pub const TMSS_REGISTER_END: u32 = TMSS_REGISTER_BASE + 0x0003;
/// Big-endian byte address of the version register the cartridge reports.
pub const VERSION_REGISTER: u32 = 0x00A1_0001;
/// Low bit of `0xA130F1`: the SRAM window is visible.
const SRAM_CONTROL_VISIBLE: u8 = 0x01;
/// Bit 1 of `0xA130F1`: the visible SRAM is write-protected.
const SRAM_CONTROL_PROTECTED: u8 = 0x02;
/// Bit 9 of the TMSS long write: the lockout is released.
const TMSS_UNLOCK_BIT: u32 = 0x0000_0200;
/// Size of a mapper bank page.
const BANK_PAGE_SIZE: u32 = 0x0008_0000;
/// Number of mapper bank windows after the fixed window 0.
const BANK_WINDOWS: usize = 7;
/// Nine-bit bank mask.
const BANK_MASK: u16 = 0x01FF;
/// Highest image size the 9-bit mapper can address (64 MiB).
pub const MAX_IMAGE_SIZE: usize = 0x0400_0000;
/// Minimum image size that can contain a full cartridge header.
const HEADER_LEN: usize = 0x200;

/// Error raised by a cartridge device access or header parse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CartridgeError {
    /// The image is smaller than the 512-byte cartridge header.
    TooShort,
    /// The image exceeds the mapper's 64 MiB addressable ceiling.
    TooLarge,
    /// The system field is not a supported Mega Drive/Genesis signature.
    MissingSegaSignature {
        /// The bounded ASCII text found at the signature offset.
        found: String,
    },
    /// The region field contains a letter other than `J`, `U` or `E`.
    UnknownRegion {
        /// The offending byte.
        value: u8,
    },
    /// The region field is empty.
    MissingRegion,
    /// The declared ROM end does not describe the image.
    HeaderMismatch {
        /// End offset declared at `0x1A4`.
        declared_end: u32,
        /// Actual image length in bytes.
        image_len: u32,
    },
    /// A read addresses a bank page that lies beyond the image.
    UnmappedRom {
        /// Requested bus address.
        address: u32,
    },
    /// A write targets read-only memory or protected SRAM.
    WriteProtected {
        /// Requested bus address.
        address: u32,
    },
    /// A word or long access was requested at an odd address.
    OddAddress {
        /// Requested bus address.
        address: u32,
    },
    /// A save-file import did not contain exactly 64 KiB.
    BadSramLength {
        /// Supplied length in bytes.
        length: usize,
    },
    /// A mapper window index outside `0..=6` was requested.
    InvalidWindow {
        /// Requested window index.
        window: usize,
    },
    /// A restored bank value does not fit the nine-bit mapper register.
    InvalidBank {
        /// Restored value.
        value: u16,
    },
    /// A restored TMSS latch is impossible for the stored model.
    InvalidTmssState {
        /// Stored locked latch.
        locked: bool,
    },
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => f.write_str("cartridge image is shorter than the 512-byte header"),
            Self::TooLarge => {
                f.write_str("cartridge image exceeds the mapper's 64 MiB addressable ceiling")
            }
            Self::MissingSegaSignature { found } => write!(
                f,
                "unsupported cartridge system signature `{found}`; expected SEGA MEGA DRIVE or SEGA GENESIS"
            ),
            Self::UnknownRegion { value } => {
                write!(f, "unknown cartridge region byte 0x{value:02X}")
            }
            Self::MissingRegion => f.write_str("cartridge header declares no region"),
            Self::HeaderMismatch {
                declared_end,
                image_len,
            } => write!(
                f,
                "cartridge header declares ROM end 0x{declared_end:06X} but the image is {image_len} byte(s)"
            ),
            Self::UnmappedRom { address } => {
                write!(f, "cartridge read at 0x{address:06X} is beyond the image")
            }
            Self::WriteProtected { address } => {
                write!(f, "cartridge write at 0x{address:06X} is write-protected")
            }
            Self::OddAddress { address } => {
                write!(f, "cartridge access at odd address 0x{address:06X}")
            }
            Self::BadSramLength { length } => write!(
                f,
                "battery SRAM import must be exactly {SRAM_SIZE} bytes, got {length}"
            ),
            Self::InvalidWindow { window } => {
                write!(f, "mapper window {window} is outside 0..={BANK_WINDOWS}")
            }
            Self::InvalidBank { value } => {
                write!(f, "mapper bank value 0x{value:04X} does not fit nine bits")
            }
            Self::InvalidTmssState { locked } => write!(
                f,
                "TMSS latch locked={locked} is invalid for a disabled TMSS model"
            ),
        }
    }
}

impl Error for CartridgeError {}

/// The console family declared by the cartridge signature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CartridgeSystem {
    /// `SEGA MEGA DRIVE`.
    MegaDrive,
    /// `SEGA GENESIS`.
    Genesis,
}

impl CartridgeSystem {
    /// Returns the signature text this system was parsed from.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MegaDrive => "SEGA MEGA DRIVE",
            Self::Genesis => "SEGA GENESIS",
        }
    }
}

/// A parsed region bitfield.
///
/// The bits are the documented `J`, `U` and `E` letters. A header may declare
/// one region or an export combination such as `JUE`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Region(u8);

impl Region {
    /// Japan.
    pub const J: Self = Self(0b0001);
    /// USA.
    pub const U: Self = Self(0b0010);
    /// Europe.
    pub const E: Self = Self(0b0100);

    /// Parses a region text such as `JUE`.
    ///
    /// Trailing padding (NUL or space) is ignored; any other byte outside the
    /// three region letters is rejected.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::MissingRegion`] for an empty field and
    /// [`CartridgeError::UnknownRegion`] for any other letter.
    pub fn parse(text: &str) -> Result<Self, CartridgeError> {
        Self::parse_bytes(text.as_bytes())
    }

    fn parse_bytes(bytes: &[u8]) -> Result<Self, CartridgeError> {
        let mut bits = 0_u8;
        let mut seen = false;
        for &byte in bytes {
            match byte {
                0 | b' ' => break,
                b'J' => {
                    bits |= Self::J.0;
                    seen = true;
                }
                b'U' => {
                    bits |= Self::U.0;
                    seen = true;
                }
                b'E' => {
                    bits |= Self::E.0;
                    seen = true;
                }
                other => return Err(CartridgeError::UnknownRegion { value: other }),
            }
        }
        if !seen {
            return Err(CartridgeError::MissingRegion);
        }
        Ok(Self(bits))
    }

    /// Returns whether every letter of `other` is present in this region.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns whether the region declares an export territory (`U` or `E`).
    #[must_use]
    pub const fn is_export(self) -> bool {
        self.contains(Self::U) || self.contains(Self::E)
    }

    /// Returns whether the region is `E`-only, the only PAL derivation.
    #[must_use]
    pub const fn is_pal(self) -> bool {
        self.0 == Self::E.0
    }

    /// Returns the raw region bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.contains(Self::J) {
            f.write_str("J")?;
        }
        if self.contains(Self::U) {
            f.write_str("U")?;
        }
        if self.contains(Self::E) {
            f.write_str("E")?;
        }
        Ok(())
    }
}

/// The declared battery-SRAM range from the header's SRAM specification.
///
/// The documented header layout is the two-byte marker `RA` at `0x1B0`, a type
/// byte at `0x1B2`, the start address at `0x1B4` and the end address at `0x1B8`.
/// A space- or zero-filled specification (and a non-`RA` marker such as `RO`)
/// means the header declares no battery SRAM. This is diagnostic data: the
/// hardware window is always the full 64 KiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SramSpec {
    /// Declared first byte.
    pub start: u32,
    /// Declared last byte.
    pub end: u32,
}

/// The parsed standard cartridge header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CartridgeHeader {
    /// Console family from the signature.
    pub system: CartridgeSystem,
    /// Declared region bitfield.
    pub region: Region,
    /// Domestic name, bounded ASCII, trimmed.
    pub domestic_name: String,
    /// Overseas name, bounded ASCII, trimmed.
    pub overseas_name: String,
    /// Product serial, bounded ASCII, trimmed.
    pub serial: String,
    /// Checksum stored at `0x18E`.
    pub checksum_stored: u16,
    /// Checksum computed from the image.
    pub checksum_computed: u16,
    /// Declared ROM start offset.
    pub rom_start: u32,
    /// Declared ROM end offset.
    pub rom_end: u32,
    /// Declared SRAM range, if the header declares one.
    pub sram: Option<SramSpec>,
}

/// Whether the modelled TMSS lockout exists on the console.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TmssModel {
    /// No lockout: VDP writes are always allowed.
    #[default]
    Disabled,
    /// TMSS present: VDP writes are blocked until the `"SEGA"` long write.
    Enabled,
}

/// The TMSS lockout latch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tmss {
    model: TmssModel,
    locked: bool,
}

impl Tmss {
    /// Creates a latch in the reset state for `model`.
    #[must_use]
    pub const fn new(model: TmssModel) -> Self {
        Self {
            model,
            locked: matches!(model, TmssModel::Enabled),
        }
    }

    /// Returns the configured model.
    #[must_use]
    pub const fn model(&self) -> TmssModel {
        self.model
    }

    /// Returns whether VDP writes are currently locked out.
    #[must_use]
    pub const fn locked(&self) -> bool {
        self.locked
    }

    /// Returns whether emulated VDP writes may reach the VDP.
    #[must_use]
    pub const fn vdp_writes_allowed(&self) -> bool {
        !self.locked
    }

    /// Applies a long write to the TMSS register.
    ///
    /// Writes outside `0xA14000..=0xA14003` are ignored. When the model is
    /// enabled and bit 9 of `value` is set the lockout is released.
    pub fn write_long(&mut self, address: u32, value: u32) {
        if !(TMSS_REGISTER_BASE..=TMSS_REGISTER_END).contains(&address) {
            return;
        }
        if value & TMSS_UNLOCK_BIT != 0 {
            self.locked = false;
        }
    }

    pub(crate) fn restore_state(model: TmssModel, locked: bool) -> Result<Self, CartridgeError> {
        if model == TmssModel::Disabled && locked {
            return Err(CartridgeError::InvalidTmssState { locked });
        }
        Ok(Self { model, locked })
    }
}

/// A parsed cartridge device: image, header, mapper, SRAM and TMSS.
#[derive(Clone, Debug)]
pub struct Cartridge {
    rom: Arc<[u8]>,
    header: CartridgeHeader,
    banks: [u16; BANK_WINDOWS],
    sram: Vec<u8>,
    sram_control: u8,
    tmss: Tmss,
}

impl Cartridge {
    /// Parses and validates a cartridge image.
    ///
    /// The checksum is computed with the documented Mega Drive rule: the
    /// wrapping sum of big-endian 16-bit words from `0x200` to the end of the
    /// image. A stored/computed mismatch is kept as diagnostic data and never
    /// fails construction.
    ///
    /// # Errors
    ///
    /// Returns a typed [`CartridgeError`] for a short or over-large image, an
    /// unsupported system signature, a missing or unknown region, or a declared
    /// ROM end that does not fit the image.
    pub fn parse(rom: Vec<u8>) -> Result<Self, CartridgeError> {
        if rom.len() < HEADER_LEN {
            return Err(CartridgeError::TooShort);
        }
        if rom.len() > MAX_IMAGE_SIZE {
            return Err(CartridgeError::TooLarge);
        }
        let system = parse_system(&rom)?;
        let domestic_name = ascii_field(&rom, 0x120, 0x30);
        let overseas_name = ascii_field(&rom, 0x150, 0x30);
        let serial = ascii_field(&rom, 0x180, 0x0E);
        let checksum_stored = read_u16(&rom, 0x18E)?;
        let checksum_computed = compute_checksum(&rom);
        let rom_start = read_u32(&rom, 0x1A0)?;
        let rom_end = read_u32(&rom, 0x1A4)?;
        let image_len = u32::try_from(rom.len()).map_err(|_| CartridgeError::TooLarge)?;
        if rom_end >= image_len || rom_end < rom_start {
            return Err(CartridgeError::HeaderMismatch {
                declared_end: rom_end,
                image_len,
            });
        }
        let sram = parse_sram_spec(&rom)?;
        let region_bytes = rom.get(0x1F0..0x200).ok_or(CartridgeError::TooShort)?;
        let region = Region::parse_bytes(region_bytes)?;
        let header = CartridgeHeader {
            system,
            region,
            domestic_name,
            overseas_name,
            serial,
            checksum_stored,
            checksum_computed,
            rom_start,
            rom_end,
            sram,
        };
        Ok(Self {
            rom: Arc::from(rom.into_boxed_slice()),
            header,
            banks: [1, 2, 3, 4, 5, 6, 7],
            sram: vec![0; SRAM_SIZE],
            sram_control: 0,
            tmss: Tmss::new(TmssModel::Disabled),
        })
    }

    /// Returns the parsed header.
    #[must_use]
    pub const fn header(&self) -> &CartridgeHeader {
        &self.header
    }

    /// Returns the immutable ROM image.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Returns the video standard implied by the header region.
    ///
    /// Only an `E`-only header derives PAL; every other region derives NTSC.
    #[must_use]
    pub const fn derived_video_standard(&self) -> VideoStandard {
        if self.header.region.is_pal() {
            VideoStandard::Pal
        } else {
            VideoStandard::Ntsc
        }
    }

    /// Returns the mapped bank register values.
    #[must_use]
    pub const fn banks(&self) -> [u16; BANK_WINDOWS] {
        self.banks
    }

    /// Selects a mapper window directly.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::InvalidWindow`] when `window` is not `0..=6`.
    pub fn set_bank(&mut self, window: usize, bank: u16) -> Result<(), CartridgeError> {
        if window >= BANK_WINDOWS {
            return Err(CartridgeError::InvalidWindow { window });
        }
        self.banks[window] = bank & BANK_MASK;
        Ok(())
    }

    /// Returns the SRAM control register (`0xA130F1`) byte.
    #[must_use]
    pub const fn sram_control(&self) -> u8 {
        self.sram_control
    }

    /// Returns the canonical 64 KiB battery SRAM.
    #[must_use]
    pub fn sram(&self) -> &[u8] {
        &self.sram
    }

    /// Returns whether the SRAM window is currently mapped.
    #[must_use]
    pub const fn sram_visible(&self) -> bool {
        self.sram_control & SRAM_CONTROL_VISIBLE != 0
    }

    /// Returns whether the mapped SRAM rejects writes.
    #[must_use]
    pub const fn sram_write_protected(&self) -> bool {
        self.sram_control & SRAM_CONTROL_PROTECTED != 0
    }

    /// Replaces the canonical SRAM from a host save file.
    ///
    /// This is construction-time host policy, not an emulated write: it never
    /// runs during a session.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::BadSramLength`] unless `bytes` is exactly
    /// [`SRAM_SIZE`] bytes.
    pub fn import_sram(&mut self, bytes: &[u8]) -> Result<(), CartridgeError> {
        if bytes.len() != SRAM_SIZE {
            return Err(CartridgeError::BadSramLength {
                length: bytes.len(),
            });
        }
        self.sram.copy_from_slice(bytes);
        Ok(())
    }

    /// Returns the TMSS latch.
    #[must_use]
    pub const fn tmss(&self) -> &Tmss {
        &self.tmss
    }

    /// Selects the TMSS model and resets its latch.
    pub fn set_tmss_model(&mut self, model: TmssModel) {
        self.tmss = Tmss::new(model);
    }

    /// Reports the modelled version register byte.
    ///
    /// Layout: bit 7 export, bit 6 PAL, bit 5 no-disk, bit 0 the modelled
    /// console's TMSS presence. Bits 0-3 are hardware version bits on real
    /// hardware; this slice reports the one documented TMSS-capable model.
    #[must_use]
    pub const fn version_register(&self, video: VideoStandard) -> u8 {
        let export = self.header.region.is_export() as u8;
        let pal = matches!(video, VideoStandard::Pal) as u8;
        (export << 7) | (pal << 6) | 0x20 | 0x01
    }

    /// Reads one byte through the cartridge address windows.
    ///
    /// This is side-effect free and never mutates mapper or SRAM state.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::UnmappedRom`] for an address outside the
    /// cartridge windows, for a read of the write-only mapper register file,
    /// and for a read beyond the selected bank page.
    pub fn read8(&self, address: u32) -> Result<u8, CartridgeError> {
        if mapper_offset(address).is_some() {
            // The mapper register file is write-only; unmodelled registers do not
            // read back, and modelled state is exposed by the accessors instead.
            return Err(CartridgeError::UnmappedRom { address });
        }
        if let Some(offset) = sram_offset(address) {
            if self.sram_visible() {
                return Ok(self.sram[offset]);
            }
        }
        let offset = self
            .translated_rom_offset(address)
            .ok_or(CartridgeError::UnmappedRom { address })?;
        let index = usize::try_from(offset).map_err(|_| CartridgeError::UnmappedRom { address })?;
        self.rom
            .get(index)
            .copied()
            .ok_or(CartridgeError::UnmappedRom { address })
    }

    /// Reads one big-endian word through the cartridge windows.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::OddAddress`] for an odd address and the same
    /// typed errors as [`Cartridge::read8`].
    pub fn read16(&self, address: u32) -> Result<u16, CartridgeError> {
        if address & 1 != 0 {
            return Err(CartridgeError::OddAddress { address });
        }
        let second = address
            .checked_add(1)
            .ok_or(CartridgeError::UnmappedRom { address })?;
        Ok(u16::from_be_bytes([
            self.read8(address)?,
            self.read8(second)?,
        ]))
    }

    /// Reads one big-endian long word through the cartridge windows.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::OddAddress`] for an odd address and the same
    /// typed errors as [`Cartridge::read8`].
    pub fn read32(&self, address: u32) -> Result<u32, CartridgeError> {
        if address & 1 != 0 {
            return Err(CartridgeError::OddAddress { address });
        }
        let second = address
            .checked_add(2)
            .ok_or(CartridgeError::UnmappedRom { address })?;
        Ok((u32::from(self.read16(address)?) << 16) | u32::from(self.read16(second)?))
    }

    /// Writes one byte to the mapper, SRAM or a read-only ROM window.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::WriteProtected`] for ROM and protected SRAM and
    /// [`CartridgeError::UnmappedRom`] outside the cartridge windows.
    pub fn write8(&mut self, address: u32, value: u8) -> Result<(), CartridgeError> {
        if let Some(offset) = mapper_offset(address) {
            match offset {
                0x01 => self.sram_control = value,
                0x03 | 0x05 | 0x07 | 0x09 | 0x0B | 0x0D | 0x0F => {
                    let window = usize::from((offset - 0x03) / 2);
                    if let Some(slot) = self.banks.get_mut(window) {
                        *slot = u16::from(value) & BANK_MASK;
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        if tmss_offset(address).is_some() {
            // A byte or word access is acknowledged but cannot unlock TMSS: only
            // the documented long write releases the lockout.
            return Ok(());
        }
        if let Some(offset) = sram_offset(address) {
            if self.sram_visible() {
                if self.sram_write_protected() {
                    return Err(CartridgeError::WriteProtected { address });
                }
                self.sram[offset] = value;
                return Ok(());
            }
        }
        if address < ROM_END {
            return Err(CartridgeError::WriteProtected { address });
        }
        Err(CartridgeError::UnmappedRom { address })
    }

    /// Writes one big-endian word.
    ///
    /// The whole span is validated before any byte changes, so a rejected SRAM
    /// write never partially mutates state.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::OddAddress`] for an odd address and the same
    /// typed errors as [`Cartridge::write8`].
    pub fn write16(&mut self, address: u32, value: u16) -> Result<(), CartridgeError> {
        if address & 1 != 0 {
            return Err(CartridgeError::OddAddress { address });
        }
        let bytes = value.to_be_bytes();
        self.write_span(address, &bytes)
    }

    /// Writes one big-endian long word.
    ///
    /// The whole span is validated before any byte changes, so a rejected SRAM
    /// write never partially mutates state.
    ///
    /// # Errors
    ///
    /// Returns [`CartridgeError::OddAddress`] for an odd address and the same
    /// typed errors as [`Cartridge::write8`].
    pub fn write32(&mut self, address: u32, value: u32) -> Result<(), CartridgeError> {
        if address & 1 != 0 {
            return Err(CartridgeError::OddAddress { address });
        }
        let bytes = value.to_be_bytes();
        self.write_span(address, &bytes)
    }

    /// Writes a multi-byte span after validating that the whole span is in one
    /// writable cartridge window.
    fn write_span(&mut self, address: u32, bytes: &[u8]) -> Result<(), CartridgeError> {
        let length =
            u32::try_from(bytes.len()).map_err(|_| CartridgeError::UnmappedRom { address })?;
        let span = length
            .checked_sub(1)
            .ok_or(CartridgeError::UnmappedRom { address })?;
        let end = address
            .checked_add(span)
            .ok_or(CartridgeError::UnmappedRom { address })?;
        if mapper_offset(address).is_some() {
            if mapper_offset(end).is_none() {
                return Err(CartridgeError::UnmappedRom { address });
            }
        } else if tmss_offset(address).is_some() {
            if tmss_offset(end).is_none() {
                return Err(CartridgeError::UnmappedRom { address });
            }
            // Only the documented 32-bit access releases the lockout; a byte or
            // word access is acknowledged and discarded.
            if bytes.len() == 4 {
                let value = bytes
                    .iter()
                    .enumerate()
                    .fold(0_u32, |value, (index, byte)| {
                        value | (u32::from(*byte) << (8 * (3 - index)))
                    });
                self.tmss.write_long(address, value);
            }
            return Ok(());
        } else if sram_offset(address).is_some() {
            if sram_offset(end).is_none() || !self.sram_visible() {
                return Err(CartridgeError::WriteProtected { address });
            }
            if self.sram_write_protected() {
                return Err(CartridgeError::WriteProtected { address });
            }
        } else if address < ROM_END {
            return Err(CartridgeError::WriteProtected { address });
        } else {
            return Err(CartridgeError::UnmappedRom { address });
        }
        for (index, byte) in bytes.iter().enumerate() {
            let offset =
                u32::try_from(index).map_err(|_| CartridgeError::UnmappedRom { address })?;
            let target = address
                .checked_add(offset)
                .ok_or(CartridgeError::UnmappedRom { address })?;
            self.write8(target, *byte)?;
        }
        Ok(())
    }

    /// Translates a cartridge-window address through the mapper.
    fn translated_rom_offset(&self, address: u32) -> Option<u32> {
        if address >= ROM_END {
            return None;
        }
        if address < BANK_PAGE_SIZE {
            return Some(address);
        }
        let window = usize::try_from((address - BANK_PAGE_SIZE) / BANK_PAGE_SIZE).ok()?;
        let bank = u32::from(self.banks.get(window).copied()?);
        let window_base = BANK_PAGE_SIZE + u32::try_from(window).ok()? * BANK_PAGE_SIZE;
        Some(bank * BANK_PAGE_SIZE + (address - window_base))
    }

    /// Restores the serialized cartridge state during a snapshot restore.
    pub(crate) fn restore_state(
        &mut self,
        banks: [u16; BANK_WINDOWS],
        sram_control: u8,
        sram: &[u8],
        tmss_model: TmssModel,
        tmss_locked: bool,
    ) -> Result<(), CartridgeError> {
        if let Some(value) = banks.iter().find(|value| **value & !BANK_MASK != 0) {
            return Err(CartridgeError::InvalidBank { value: *value });
        }
        if sram.len() != SRAM_SIZE {
            return Err(CartridgeError::BadSramLength { length: sram.len() });
        }
        let tmss = Tmss::restore_state(tmss_model, tmss_locked)?;
        self.banks = banks;
        self.sram_control = sram_control;
        self.sram.copy_from_slice(sram);
        self.tmss = tmss;
        Ok(())
    }
}

/// Returns the mapper register offset for `address`, if it is in the window.
fn mapper_offset(address: u32) -> Option<u8> {
    if (MAPPER_REGISTER_BASE..MAPPER_REGISTER_END).contains(&address) {
        u8::try_from(address - MAPPER_REGISTER_BASE).ok()
    } else {
        None
    }
}

/// Returns the SRAM window offset for `address`, if it is in the window.
fn sram_offset(address: u32) -> Option<usize> {
    if (SRAM_BASE..=SRAM_END).contains(&address) {
        usize::try_from(address - SRAM_BASE).ok()
    } else {
        None
    }
}

/// Returns the TMSS register offset for `address`, if it is in the register.
fn tmss_offset(address: u32) -> Option<u8> {
    if (TMSS_REGISTER_BASE..=TMSS_REGISTER_END).contains(&address) {
        u8::try_from(address - TMSS_REGISTER_BASE).ok()
    } else {
        None
    }
}

/// Parses the documented SRAM specification from the header.
///
/// The marker `RA` at `0x1B0` declares battery SRAM; the start and end addresses
/// are the big-endian `u32` fields at `0x1B4` and `0x1B8`. Space- or zero-filled
/// fields, any other marker and a zero/zero range mean the header declares no
/// SRAM.
fn parse_sram_spec(rom: &[u8]) -> Result<Option<SramSpec>, CartridgeError> {
    let marker = rom.get(0x1B0..0x1B2).ok_or(CartridgeError::TooShort)?;
    if marker != b"RA" {
        return Ok(None);
    }
    let start = read_u32(rom, 0x1B4)?;
    let end = read_u32(rom, 0x1B8)?;
    if start == 0 && end == 0 {
        return Ok(None);
    }
    Ok(Some(SramSpec { start, end }))
}

/// Classifies the 16-byte system field at `0x100`.
fn parse_system(rom: &[u8]) -> Result<CartridgeSystem, CartridgeError> {
    let field = rom.get(0x100..0x110).ok_or(CartridgeError::TooShort)?;
    if field.starts_with(b"SEGA MEGA DRIVE") {
        return Ok(CartridgeSystem::MegaDrive);
    }
    if field.starts_with(b"SEGA GENESIS") {
        return Ok(CartridgeSystem::Genesis);
    }
    Err(CartridgeError::MissingSegaSignature {
        found: bounded_ascii(field),
    })
}

/// Decodes a bounded ASCII header field.
fn ascii_field(rom: &[u8], offset: usize, length: usize) -> String {
    let end = offset.saturating_add(length);
    bounded_ascii(rom.get(offset..end).unwrap_or(&[]))
}

/// Decodes up to the first non-printable byte, trimming trailing spaces.
fn bounded_ascii(bytes: &[u8]) -> String {
    let printable = bytes
        .iter()
        .take_while(|byte| (0x20..=0x7E).contains(*byte))
        .copied()
        .collect::<Vec<_>>();
    let trimmed = printable
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(&[][..], |end| &printable[..=end]);
    String::from_utf8_lossy(trimmed).into_owned()
}

/// Reads a big-endian `u16` header field.
fn read_u16(rom: &[u8], offset: usize) -> Result<u16, CartridgeError> {
    let bytes = rom
        .get(offset..offset + 2)
        .ok_or(CartridgeError::TooShort)?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

/// Reads a big-endian `u32` header field.
fn read_u32(rom: &[u8], offset: usize) -> Result<u32, CartridgeError> {
    let bytes = rom
        .get(offset..offset + 4)
        .ok_or(CartridgeError::TooShort)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Computes the documented Mega Drive checksum over `0x200..`.
pub(crate) fn compute_checksum(rom: &[u8]) -> u16 {
    rom.get(HEADER_LEN..)
        .unwrap_or(&[])
        .chunks_exact(2)
        .fold(0_u16, |checksum, chunk| {
            checksum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]))
        })
}

/// Stamps a valid synthetic header on a raw unit-test image.
///
/// The CPU and disassembler unit tests exercise raw opcode images that predate
/// the cartridge header model; this keeps them on the same parsed device without
/// duplicating a header parser. It is never part of the public API.
#[cfg(test)]
pub(crate) fn test_cartridge(mut rom: Vec<u8>) -> Cartridge {
    if rom.len() < 0x200 {
        rom.resize(0x200, 0);
    }
    assert!(
        crate::test_support::stamp_header(&mut rom, "JUE"),
        "unit-test image is large enough for a header"
    );
    Cartridge::parse(rom).expect("synthetic unit-test cartridge header")
}

#[cfg(test)]
mod tests {
    use super::{Cartridge, CartridgeError, Region};

    #[test]
    fn region_parse_and_display_round_trip() {
        let region = Region::parse("JUE").expect("region");
        assert_eq!(region.to_string(), "JUE");
        assert!(region.is_export());
        assert!(!region.is_pal());
        assert_eq!(Region::parse(" "), Err(CartridgeError::MissingRegion));
        assert_eq!(
            Region::parse("JX"),
            Err(CartridgeError::UnknownRegion { value: b'X' })
        );
    }

    #[test]
    fn a_short_image_is_rejected_before_reading_header_fields() {
        assert!(matches!(
            Cartridge::parse(vec![0; 0x1FF]),
            Err(CartridgeError::TooShort)
        ));
    }
}
