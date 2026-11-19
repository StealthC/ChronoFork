//! VDP DMA transfer state.
//!
//! The 315-5313 has five transfer modes: 68000 memory to VRAM, CRAM or VSRAM,
//! VRAM fill and VRAM copy. The eager transfer engine in `mod.rs` validates the
//! complete range and performs the transfer; this module owns the mode enum and
//! the latched source/length/busy state that the engine reports and the status
//! register exposes. The register-programmed path uses the hardware layout:
//! `0x13`/`0x14` are the little-endian word length, `0x15`/`0x16`/`0x17` the
//! source low/mid/high, and `0x17` bits 7..=6 select 68k-to-VDP (destination
//! from the command code), fill or copy.

/// A VDP DMA transfer mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DmaMode {
    /// 68000 memory to VRAM.
    #[default]
    MemoryToVram,
    /// 68000 memory to CRAM.
    MemoryToCram,
    /// 68000 memory to VSRAM.
    MemoryToVsram,
    /// VRAM fill.
    Fill,
    /// VRAM to VRAM copy.
    Copy,
}

impl DmaMode {
    /// Returns the mode's serialized code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::MemoryToVram => 0,
            Self::MemoryToCram => 1,
            Self::MemoryToVsram => 2,
            Self::Fill => 3,
            Self::Copy => 4,
        }
    }

    /// Decodes a serialized mode code, rejecting unknown values.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::MemoryToVram),
            1 => Some(Self::MemoryToCram),
            2 => Some(Self::MemoryToVsram),
            3 => Some(Self::Fill),
            4 => Some(Self::Copy),
            _ => None,
        }
    }
}

/// Canonical DMA state.
///
/// `active` drives status bit 1 (DMA busy). `source` and `length` are the
/// latched transfer parameters; in this slice `source` is a VRAM byte address
/// (the VDP has no 68000-memory view), `length` is the transfer word count, and
/// the fill mode reuses the source low byte as the fill value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DmaState {
    source: u32,
    length: u16,
    mode: DmaMode,
    active: bool,
}

impl DmaState {
    /// Builds state from validated snapshot fields.
    pub(super) const fn from_parts(source: u32, length: u16, mode: DmaMode, active: bool) -> Self {
        Self {
            source,
            length,
            mode,
            active,
        }
    }

    /// Returns the latched DMA source.
    #[must_use]
    pub const fn source(&self) -> u32 {
        self.source
    }

    /// Returns the latched transfer length.
    #[must_use]
    pub const fn length(&self) -> u16 {
        self.length
    }

    /// Returns the selected transfer mode.
    #[must_use]
    pub const fn mode(&self) -> DmaMode {
        self.mode
    }

    /// Returns whether a transfer is in progress.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.active
    }
}
