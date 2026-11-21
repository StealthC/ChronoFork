//! `HL`-operand substitution shared by the main, `CB` and prefix tables.
//!
//! A `DD`/`FD` prefix replaces `HL`/`(HL)` with `IX`/`IY` and adds a signed
//! displacement for the memory forms. [`Target`] carries that choice and the
//! fetched displacement so the instruction tables do not need to know how the
//! bytes were obtained.

use super::Z80;

/// Which 16-bit pair an instruction addresses as its `HL` operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IndexKind {
    /// Plain `HL` / `(HL)`.
    Hl,
    /// `IX` / `(IX+d)` selected by a `DD` prefix.
    Ix,
    /// `IY` / `(IY+d)` selected by an `FD` prefix.
    Iy,
}

/// The `HL`-operand substitution of the main and `CB` tables.
///
/// `displacement` is only read by instructions that address `(HL)`; prefix
/// modules fetch the displacement byte from the instruction stream and pass it
/// here so the table code never has to know how it was obtained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    kind: IndexKind,
    displacement: i8,
}

impl Target {
    /// The unprefixed `HL`/`(HL)` target.
    pub(crate) const fn hl() -> Self {
        Self {
            kind: IndexKind::Hl,
            displacement: 0,
        }
    }

    /// An `IX`/`IY` target with its fetched displacement.
    pub(crate) const fn indexed(kind: IndexKind, displacement: i8) -> Self {
        Self { kind, displacement }
    }

    /// Whether the target is `IX`/`IY` rather than `HL`.
    pub(crate) const fn is_prefixed(self) -> bool {
        match self.kind {
            IndexKind::Hl => false,
            IndexKind::Ix | IndexKind::Iy => true,
        }
    }

    /// The 16-bit pair value (`HL`, `IX` or `IY`) without displacement.
    pub(crate) fn pair(self, z80: &Z80) -> u16 {
        match self.kind {
            IndexKind::Hl => z80.hl(),
            IndexKind::Ix => z80.ix,
            IndexKind::Iy => z80.iy,
        }
    }

    /// Writes the pair value used by instructions that address the pair.
    pub(crate) fn set_pair(self, z80: &mut Z80, value: u16) {
        match self.kind {
            IndexKind::Hl => z80.set_hl(value),
            IndexKind::Ix => z80.ix = value,
            IndexKind::Iy => z80.iy = value,
        }
    }

    /// The effective memory address for `(HL)` or `(IX+d)`/`(IY+d)`.
    pub(crate) fn address(self, z80: &Z80) -> u16 {
        self.pair(z80)
            .wrapping_add_signed(i16::from(self.displacement))
    }
}
