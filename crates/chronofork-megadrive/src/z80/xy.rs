//! `DD`/`FD` prefixes: `IX`/`IY` and the `(IX+d)`/`(IY+d)` displacement.
//!
//! The prefix re-dispatches the main and `CB` tables with `HL` replaced by
//! `IX`/`IY` and fetches one displacement byte for instructions that address
//! `(HL)`. Opcodes that do not address `HL` or `(HL)` execute unchanged and the
//! prefix is a wasted four T-states; the undocumented `IXH`/`IXL`/`IYH`/`IYL`
//! byte registers are not modelled (see the module-level limits in
//! [`crate::z80`]).
//!
//! `DD ED ...`/`FD ED ...`: the prefix is ignored and the `ED` instruction
//! executes, as on hardware. `DD CB d op`/`FD CB d op` fetch the displacement
//! before the `CB` opcode and use the documented 20/23 T-state totals.
//!
//! A repeated prefix (`DD DD ...`) is a normal hardware case: the first prefix
//! is wasted. [`crate::z80::Z80::step`] peels that case before dispatching so a
//! stream of prefixes always advances; a direct call to this module returns the
//! 4 T-states of the extra prefix.

use super::{IndexKind, Target, Z80, Z80Bus, Z80Error, cb, ed, ops};

/// Executes the instruction selected by one `DD` (`IX`) or `FD` (`IY`) prefix.
///
/// The prefix byte itself was fetched by [`crate::z80::Z80::step`]; the returned
/// total includes that fetch.
///
/// # Errors
///
/// Propagates a [`Z80Bus`] failure from an opcode, displacement or operand
/// access.
pub fn execute(z80: &mut Z80, bus: &mut impl Z80Bus, prefix: u8) -> Result<u32, Z80Error> {
    let kind = if prefix == 0xDD {
        IndexKind::Ix
    } else {
        IndexKind::Iy
    };
    let opcode = z80.fetch_opcode(bus)?;
    match opcode {
        0xDD | 0xFD => Ok(4),
        0xCB => {
            let displacement = z80.read_imm8(bus)? as i8;
            let extended = z80.fetch_opcode(bus)?;
            cb::execute_indexed(z80, bus, extended, Target::indexed(kind, displacement))
        }
        0xED => {
            let extended = z80.fetch_opcode(bus)?;
            Ok(4 + ed::execute(z80, bus, extended)?)
        }
        _ => {
            if uses_hl_memory(opcode) {
                let displacement = z80.read_imm8(bus)? as i8;
                ops::execute_with(z80, bus, opcode, Target::indexed(kind, displacement))
            } else {
                ops::execute_with(z80, bus, opcode, Target::indexed(kind, 0))
            }
        }
    }
}

/// Returns whether `opcode` addresses `(HL)` and therefore reads a displacement.
///
/// `0x76` (`HALT`) shares the `0x70..=0x77` range but is not an `(HL)` form.
const fn uses_hl_memory(opcode: u8) -> bool {
    if opcode == 0x76 {
        return false;
    }
    matches!(
        opcode,
        0x34 | 0x35 | 0x36 | 0x70
            ..=0x77
                | 0x46
                | 0x4E
                | 0x56
                | 0x5E
                | 0x66
                | 0x6E
                | 0x7E
                | 0x86
                | 0x8E
                | 0x96
                | 0x9E
                | 0xA6
                | 0xAE
                | 0xB6
                | 0xBE
    )
}
