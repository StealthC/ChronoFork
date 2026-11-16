//! Bounded, execution-cross-checked disassembler for the 68000 subset this
//! slice executes.
//!
//! The CPU decoder in [`crate::cpu`] is authoritative. This module is a pure
//! viewer: it never executes, never mutates a machine and never loads state. Its
//! opcode masks, the order in which those masks are tried and the instruction
//! lengths mirror [`M68000::step`](crate::cpu::M68000::step) exactly, and a test
//! executes one CPU step for every supported form to prove that the printed
//! length equals the observed `PC` delta and that the reported opcode word is
//! the word fetched at the start address.
//!
//! # Supported subset
//!
//! Only the encodings the CPU executes are decoded. Every other opcode word is
//! rendered as `DC.W` with length 2, including encodings that share a mask with
//! a supported instruction but use a form this slice rejects (for example
//! `LEA (abs).W` or an unsupported branch condition).
//!
//! | Encoding | Text | Bytes |
//! |---|---|---|
//! | `MOVEQ #imm,Dn` | `MOVEQ #$FE,D0` | 2 |
//! | `BRA`/`BSR`/`BCC`/`BCS`/`BEQ`/`BNE`/`BMI`/`BPL` | `BRA $00000202` | 2 |
//! | `MOVE.W #imm,SR` | `MOVE.W #$2000,SR` | 4 |
//! | `NOP`, `RTS`, `RTE` | `NOP` | 2 |
//! | `JMP (abs).L`, `JSR (abs).L` | `JMP ($00000206).L` | 6 |
//! | `CLR.W Dn`, `CLR.L Dn` | `CLR.W D0` | 2 |
//! | `LEA (abs).L,An` | `LEA ($12345678).L,A3` | 6 |
//! | `PEA (abs).L` | `PEA ($00FF1000).L` | 6 |
//! | `TST.B/W/L Dn` | `TST.W D1` | 2 |
//! | `MOVE.W/L #imm,Dn` | `MOVE.L #$12345678,D1` | 4/6 |
//! | `MOVE.B/W/L Dn,Dm` | `MOVE.B D0,D3` | 2 |
//! | `MOVE.B/W/L Dn,(abs).L` | `MOVE.W D0,($00FF0000).L` | 6 |
//! | `MOVE.B/W/L (abs).L,Dn` | `MOVE.B ($00FF0000).L,D0` | 6 |
//! | `CMP.B/W/L Dn,Dm` | `CMP.L D1,D0` | 2 |
//! | `DBRA Dn,label` | `DBRA D0,$00000206` | 4 |
//! | `ADDQ/SUBQ.W/L #imm,Dn` | `ADDQ.W #1,D0` | 2 |
//! | `ADD.B/W/L Dn,Dm`, `SUB.B/W/L Dn,Dm` | `ADD.L D1,D0` | 2 |
//! | anything else | `DC.W $4AFC` | 2 |
//!
//! # Text format
//!
//! The text is deterministic and ASCII: an uppercase mnemonic, an optional
//! `.B`/`.W`/`.L` size suffix, then a comma-separated operand list with no
//! spaces. Operand spellings:
//!
//! - data and address registers are `D0`..`D7` and `A0`..`A7`;
//! - immediates are `#` followed by `$` and fixed-width uppercase hexadecimal
//!   digits: 2 for the `MOVEQ` byte, 4 for a word and 8 for a long word;
//! - an absolute-long effective address is `($` plus 8 hexadecimal digits plus
//!   `).L`;
//! - a branch or `DBRA` target is `$` plus 8 hexadecimal digits of the absolute
//!   target address, computed from the instruction address, the encoded
//!   displacement and the instruction length, exactly like the CPU;
//! - `ADDQ`/`SUBQ` print their encoded count as decimal `#1`..`#8`, because the
//!   field is a count and not a data value;
//! - an unknown or unsupported opcode word is `DC.W $` plus 4 hexadecimal
//!   digits.
//!
//! # Limits and errors
//!
//! A request disassembles `1..=`[`MAX_DISASSEMBLY_COUNT`] instructions starting
//! at an even address. Every opcode and extension word must be available from
//! the supplied [`DisasmMemory`]; the output is allocated after the count is
//! validated and is bounded by it. The view never reads a word that an
//! instruction does not encode, so a window does not need to extend past the
//! final requested instruction. A branch or `DBRA` whose target address is not
//! representable in the 32-bit address space, and a request that would step
//! past `0xFFFF_FFFF`, are typed errors instead of a wrapped address. No code
//! and data inference, control-flow graph, label or jump-table recovery is
//! performed or claimed.

use crate::bus::ROM_END;

/// Maximum number of instructions one [`disassemble`] request may produce.
pub const MAX_DISASSEMBLY_COUNT: usize = 4_096;

/// A byte-addressed window the disassembler reads instruction words from.
///
/// A view is read-only by construction: the only method that can expose state
/// takes `&self` and returns bytes. Implementations decide which addresses they
/// map; an address they cannot serve returns `None`, which [`disassemble`]
/// reports as [`DisasmError::UnavailableWord`].
pub trait DisasmMemory {
    /// Returns the byte at `address`, or `None` when the window does not map it.
    fn read_byte(&self, address: u32) -> Option<u8>;

    /// Returns the big-endian word at the even address `address`.
    ///
    /// The default implementation composes two [`DisasmMemory::read_byte`]
    /// calls, so it is `None` unless both bytes are mapped. Implementations may
    /// override it for efficiency.
    fn read_word(&self, address: u32) -> Option<u16> {
        let high = self.read_byte(address)?;
        let low = self.read_byte(address.checked_add(1)?)?;
        Some(u16::from_be_bytes([high, low]))
    }
}

/// A disassembly window over a cartridge image mapped at address zero.
///
/// This mirrors the bus cartridge window (whose base is zero): byte `n` of the
/// image is at address `n`, and only addresses below [`ROM_END`] and inside the
/// image are mapped. A larger image is not addressable past the 4 MiB cartridge
/// window.
#[derive(Clone, Copy, Debug)]
pub struct RomMemory<'a> {
    rom: &'a [u8],
}

impl<'a> RomMemory<'a> {
    /// Creates a cartridge window over `rom`.
    #[must_use]
    pub const fn new(rom: &'a [u8]) -> Self {
        Self { rom }
    }
}

impl DisasmMemory for RomMemory<'_> {
    fn read_byte(&self, address: u32) -> Option<u8> {
        if address >= ROM_END {
            return None;
        }
        let index = usize::try_from(address).ok()?;
        self.rom.get(index).copied()
    }
}

/// A disassembly window over an arbitrary byte slice mapped at `base`.
///
/// This is how a caller disassembles work RAM or another contiguous region
/// without inventing a cartridge image: byte `n` of `bytes` is at address
/// `base + n`. Offsets wrap-free: an address below `base`, or one whose offset
/// is past the slice, is unmapped.
#[derive(Clone, Copy, Debug)]
pub struct WindowMemory<'a> {
    base: u32,
    bytes: &'a [u8],
}

impl<'a> WindowMemory<'a> {
    /// Creates a window over `bytes` whose first byte is at `base`.
    #[must_use]
    pub const fn new(base: u32, bytes: &'a [u8]) -> Self {
        Self { base, bytes }
    }
}

impl DisasmMemory for WindowMemory<'_> {
    fn read_byte(&self, address: u32) -> Option<u8> {
        let offset = address.checked_sub(self.base)?;
        let index = usize::try_from(offset).ok()?;
        self.bytes.get(index).copied()
    }
}

/// One decoded instruction in a [`disassemble`] result.
///
/// The opcode word is reported verbatim so a caller can correlate the view with
/// the CPU's instruction trace. `length` is the encoded instruction length in
/// bytes (2, 4 or 6), which equals the `PC` delta a successful execution of a
/// sequential instruction produces; control-flow instructions use it to compute
/// their target, so their `PC` delta is the distance to that target instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisasmInstruction {
    address: u32,
    opcode: u16,
    length: u8,
    text: String,
}

impl DisasmInstruction {
    /// Returns the address the opcode word was read from.
    #[must_use]
    pub const fn address(&self) -> u32 {
        self.address
    }

    /// Returns the fetched opcode word.
    #[must_use]
    pub const fn opcode(&self) -> u16 {
        self.opcode
    }

    /// Returns the encoded instruction length in bytes.
    #[must_use]
    pub const fn length(&self) -> u8 {
        self.length
    }

    /// Returns the documented mnemonic and operand text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A rejected disassembly request or an encoding the view cannot render.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisasmError {
    /// The requested count is zero or above [`MAX_DISASSEMBLY_COUNT`].
    InvalidCount {
        /// Rejected count.
        count: usize,
    },
    /// The start address is not word-aligned.
    OddAddress {
        /// Rejected address.
        address: u32,
    },
    /// An instruction address or a control-flow target is outside the 32-bit
    /// address space.
    ///
    /// The field is the address of the instruction whose address arithmetic
    /// overflowed.
    AddressOverflow {
        /// Instruction address whose arithmetic overflowed.
        address: u32,
    },
    /// The window does not map an opcode or extension word the request needs.
    UnavailableWord {
        /// First missing address.
        address: u32,
    },
}

impl std::fmt::Display for DisasmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCount { count } => write!(
                formatter,
                "disassembly count {count} is outside 1..={MAX_DISASSEMBLY_COUNT}"
            ),
            Self::OddAddress { address } => {
                write!(formatter, "disassembly address 0x{address:06X} is odd")
            }
            Self::AddressOverflow { address } => {
                write!(formatter, "disassembly address overflow at 0x{address:06X}")
            }
            Self::UnavailableWord { address } => {
                write!(
                    formatter,
                    "disassembly word at 0x{address:06X} is unavailable"
                )
            }
        }
    }
}

impl std::error::Error for DisasmError {}

/// Disassembles up to `count` instructions from `memory` starting at `address`.
///
/// See the module documentation for the supported subset, the text format and
/// the limits. The result is ordered by ascending address; a branch target is
/// printed but not followed, so the output is a straight-line view of the
/// requested range and never recursive.
///
/// # Errors
///
/// Returns [`DisasmError::InvalidCount`] for a count outside
/// `1..=`[`MAX_DISASSEMBLY_COUNT`], [`DisasmError::OddAddress`] for an odd start
/// address, [`DisasmError::UnavailableWord`] when the window does not supply an
/// opcode or extension word, and [`DisasmError::AddressOverflow`] when a target
/// or the address after the last requested instruction is not representable.
pub fn disassemble(
    memory: &dyn DisasmMemory,
    address: u32,
    count: usize,
) -> Result<Vec<DisasmInstruction>, DisasmError> {
    if count == 0 || count > MAX_DISASSEMBLY_COUNT {
        return Err(DisasmError::InvalidCount { count });
    }
    if address & 1 != 0 {
        return Err(DisasmError::OddAddress { address });
    }

    let mut instructions = Vec::with_capacity(count);
    let mut current = address;
    for remaining in (0..count).rev() {
        let opcode = memory
            .read_word(current)
            .ok_or(DisasmError::UnavailableWord { address: current })?;
        let decoded = decode(memory, current, opcode)?;
        instructions.push(DisasmInstruction {
            address: current,
            opcode,
            length: decoded.length,
            text: decoded.text,
        });
        if remaining > 0 {
            current = current
                .checked_add(u32::from(decoded.length))
                .ok_or(DisasmError::AddressOverflow { address: current })?;
        }
    }
    Ok(instructions)
}

/// One decoded instruction before it is attached to its address and opcode.
struct Decoded {
    text: String,
    length: u8,
}

impl Decoded {
    const fn new(text: String, length: u8) -> Self {
        Self { text, length }
    }
}

/// Renders an unknown or unsupported opcode as a two-byte data word.
fn data_word(opcode: u16) -> Decoded {
    Decoded::new(format!("DC.W ${opcode:04X}"), 2)
}

/// Fetches the extension word at `instruction + offset`.
fn extension_word(
    memory: &dyn DisasmMemory,
    instruction: u32,
    offset: u32,
) -> Result<u16, DisasmError> {
    let address = instruction
        .checked_add(offset)
        .ok_or(DisasmError::AddressOverflow {
            address: instruction,
        })?;
    memory
        .read_word(address)
        .ok_or(DisasmError::UnavailableWord { address })
}

/// Fetches the big-endian long extension word at `instruction + offset`.
fn extension_long(
    memory: &dyn DisasmMemory,
    instruction: u32,
    offset: u32,
) -> Result<u32, DisasmError> {
    let high = extension_word(memory, instruction, offset)?;
    let low = extension_word(memory, instruction, offset + 2)?;
    Ok((u32::from(high) << 16) | u32::from(low))
}

/// Computes a control-flow target the way the CPU's `relative_target` does.
///
/// `length` is the number of bytes consumed before the displacement is applied,
/// so the base is the address immediately after the instruction. A target
/// outside the 32-bit address space is a typed error. An odd target is printed
/// as encoded; the CPU decides at execution time whether it is an address error,
/// and this view does not execute.
fn relative_target(address: u32, length: u32, displacement: i32) -> Result<u32, DisasmError> {
    let target = i64::from(address) + i64::from(length) + i64::from(displacement);
    if target < 0 || target > i64::from(u32::MAX) {
        return Err(DisasmError::AddressOverflow { address });
    }
    // Proven by the check above: the value is inside the `u32` range.
    Ok(target as u32)
}

/// Decodes one opcode word, mirroring the CPU dispatch order and masks.
fn decode(memory: &dyn DisasmMemory, address: u32, opcode: u16) -> Result<Decoded, DisasmError> {
    // `M68000::step`: 0111 rrr 0 dddddddd.
    if (opcode & 0xF100) == 0x7000 {
        let register = (opcode >> 9) & 0x7;
        let immediate = opcode & 0x00FF;
        return Ok(Decoded::new(
            format!("MOVEQ #${immediate:02X},D{register}"),
            2,
        ));
    }

    // `M68000::step`: every 0110 branch encoding, before any exact opcode.
    if (opcode & 0xF000) == 0x6000 {
        return decode_branch(address, opcode);
    }

    match opcode {
        0x46FC => {
            let value = extension_word(memory, address, 2)?;
            Ok(Decoded::new(format!("MOVE.W #${value:04X},SR"), 4))
        }
        0x4E71 => Ok(Decoded::new("NOP".to_owned(), 2)),
        0x4E73 => Ok(Decoded::new("RTE".to_owned(), 2)),
        0x4E75 => Ok(Decoded::new("RTS".to_owned(), 2)),
        0x4EF9 => {
            let target = extension_long(memory, address, 2)?;
            Ok(Decoded::new(format!("JMP (${target:08X}).L"), 6))
        }
        0x4EB9 => {
            let target = extension_long(memory, address, 2)?;
            Ok(Decoded::new(format!("JSR (${target:08X}).L"), 6))
        }
        _ => decode_other(memory, address, opcode),
    }
}

/// Decodes one 0110 branch encoding.
///
/// Only the conditions the CPU executes are rendered; every other condition has
/// the same shape but is rejected during execution, so it falls back to
/// `DC.W`.
fn decode_branch(address: u32, opcode: u16) -> Result<Decoded, DisasmError> {
    let mnemonic = match (opcode >> 8) & 0xF {
        0x0 => "BRA",
        0x1 => "BSR",
        0x4 => "BCC",
        0x5 => "BCS",
        0x6 => "BNE",
        0x7 => "BEQ",
        0xA => "BPL",
        0xB => "BMI",
        _ => return Ok(data_word(opcode)),
    };
    let displacement = i32::from(i8::from_be_bytes([(opcode & 0xFF) as u8]));
    let target = relative_target(address, 2, displacement)?;
    Ok(Decoded::new(format!("{mnemonic} ${target:08X}"), 2))
}

/// Decodes the encodings `M68000::execute_other` tries after the exact opcodes,
/// in the same order and with the same masks.
fn decode_other(
    memory: &dyn DisasmMemory,
    address: u32,
    opcode: u16,
) -> Result<Decoded, DisasmError> {
    // CLR.W / CLR.L Dn.
    if (opcode & 0xFFC0) == 0x4240 {
        return Ok(Decoded::new(format!("CLR.W D{}", opcode & 0x7), 2));
    }
    if (opcode & 0xFFC0) == 0x4280 {
        return Ok(Decoded::new(format!("CLR.L D{}", opcode & 0x7), 2));
    }

    // LEA (abs).L,An. Only the absolute-long encoding ends in 0xF9; the
    // absolute-word and PC-relative forms are not executed by this slice.
    if (opcode & 0xF1FF) == 0x41F9 {
        let register = (opcode >> 9) & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(format!("LEA (${value:08X}).L,A{register}"), 6));
    }

    // PEA (abs).L, whose full opcode is 0x4879.
    if opcode == 0x4879 {
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(format!("PEA (${value:08X}).L"), 6));
    }

    // TST.B/W/L Dn.
    if (opcode & 0xFFF8) == 0x4A00 {
        return Ok(Decoded::new(format!("TST.B D{}", opcode & 0x7), 2));
    }
    if (opcode & 0xFFF8) == 0x4A40 {
        return Ok(Decoded::new(format!("TST.W D{}", opcode & 0x7), 2));
    }
    if (opcode & 0xFFF8) == 0x4A80 {
        return Ok(Decoded::new(format!("TST.L D{}", opcode & 0x7), 2));
    }

    // MOVE.W/L #imm,Dn.
    if (opcode & 0xF1FF) == 0x303C {
        let register = (opcode >> 9) & 0x7;
        let value = extension_word(memory, address, 2)?;
        return Ok(Decoded::new(format!("MOVE.W #${value:04X},D{register}"), 4));
    }
    if (opcode & 0xF1FF) == 0x203C {
        let register = (opcode >> 9) & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(format!("MOVE.L #${value:08X},D{register}"), 6));
    }

    // MOVE.B/W/L Dn,Dm: data-register source and destination only.
    if (opcode & 0x01C0) == 0 && (opcode & 0x0038) == 0 {
        let source = opcode & 0x7;
        let destination = (opcode >> 9) & 0x7;
        let mnemonic = match opcode & 0xF000 {
            0x1000 => Some("MOVE.B"),
            0x2000 => Some("MOVE.L"),
            0x3000 => Some("MOVE.W"),
            _ => None,
        };
        if let Some(mnemonic) = mnemonic {
            return Ok(Decoded::new(
                format!("{mnemonic} D{source},D{destination}"),
                2,
            ));
        }
    }

    // MOVE.B/W/L Dn,(abs).L.
    if (opcode & 0xFFF8) == 0x13C0 {
        let source = opcode & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(
            format!("MOVE.B D{source},(${value:08X}).L"),
            6,
        ));
    }
    if (opcode & 0xFFF8) == 0x23C0 {
        let source = opcode & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(
            format!("MOVE.L D{source},(${value:08X}).L"),
            6,
        ));
    }
    if (opcode & 0xFFF8) == 0x33C0 {
        let source = opcode & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(
            format!("MOVE.W D{source},(${value:08X}).L"),
            6,
        ));
    }

    // MOVE.B/W/L (abs).L,Dn. The low byte 0x39 is absolute-long; the
    // absolute-word (0x38) and PC-relative source forms are not executed.
    if (opcode & 0xF1FF) == 0x1039 {
        let destination = (opcode >> 9) & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(
            format!("MOVE.B (${value:08X}).L,D{destination}"),
            6,
        ));
    }
    if (opcode & 0xF1FF) == 0x2039 {
        let destination = (opcode >> 9) & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(
            format!("MOVE.L (${value:08X}).L,D{destination}"),
            6,
        ));
    }
    if (opcode & 0xF1FF) == 0x3039 {
        let destination = (opcode >> 9) & 0x7;
        let value = extension_long(memory, address, 2)?;
        return Ok(Decoded::new(
            format!("MOVE.W (${value:08X}).L,D{destination}"),
            6,
        ));
    }

    // CMP.B/W/L Dn,Dn. A size field of 0b11 is an unsupported form and falls
    // through to `DC.W`, matching the CPU error.
    if (opcode & 0xF100) == 0xB000 && (opcode & 0x0038) == 0 {
        let source = opcode & 0x7;
        let destination = (opcode >> 9) & 0x7;
        let size = match opcode & 0x00C0 {
            0x0000 => Some("B"),
            0x0040 => Some("W"),
            0x0080 => Some("L"),
            _ => None,
        };
        if let Some(size) = size {
            return Ok(Decoded::new(
                format!("CMP.{size} D{source},D{destination}"),
                2,
            ));
        }
    }

    // DBRA Dn,disp16 (the DBcc encoding with condition 0x1).
    if (opcode & 0xFFF8) == 0x51C8 {
        let register = opcode & 0x7;
        let displacement = i32::from(extension_word(memory, address, 2)? as i16);
        let target = relative_target(address, 4, displacement)?;
        return Ok(Decoded::new(format!("DBRA D{register},${target:08X}"), 4));
    }

    // ADDQ / SUBQ (data-register destinations only). Every other form under
    // this mask is rejected by the CPU, so it is rendered as `DC.W` here.
    if (opcode & 0xF000) == 0x5000 {
        let subtract = opcode & 0x0100 != 0;
        let immediate = match (opcode >> 9) & 0x7 {
            0 => 8,
            value => value,
        };
        let mode = (opcode >> 3) & 0x7;
        let size = match (opcode >> 6) & 0x3 {
            1 => Some("W"),
            2 => Some("L"),
            _ => None,
        };
        if mode == 0 {
            if let Some(size) = size {
                let mnemonic = if subtract { "SUBQ" } else { "ADDQ" };
                return Ok(Decoded::new(
                    format!("{mnemonic}.{size} #{immediate},D{}", opcode & 0x7),
                    2,
                ));
            }
        }
        return Ok(data_word(opcode));
    }

    // ADD/SUB Dn,Dn (byte/word/long). As with the CPU, the destination mode is
    // bits 5-3 and the size is bits 7-6.
    if (opcode & 0x0038) == 0 {
        let source = opcode & 0x7;
        let destination = (opcode >> 9) & 0x7;
        let size = match opcode & 0x00C0 {
            0x0000 => Some("B"),
            0x0040 => Some("W"),
            0x0080 => Some("L"),
            _ => None,
        };
        if let Some(size) = size {
            let mnemonic = match opcode & 0xF000 {
                0x9000 => Some("SUB"),
                0xD000 => Some("ADD"),
                _ => None,
            };
            if let Some(mnemonic) = mnemonic {
                return Ok(Decoded::new(
                    format!("{mnemonic}.{size} D{source},D{destination}"),
                    2,
                ));
            }
        }
    }

    Ok(data_word(opcode))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DisasmError, DisasmMemory, MAX_DISASSEMBLY_COUNT, RomMemory, WindowMemory, disassemble,
    };
    use crate::bus::{MegadriveBus, RAM_BASE, ROM_END};
    use crate::cpu::M68000;

    /// Address of the first instruction in the test program.
    const PROGRAM_ADDRESS: u32 = 0x0000_0200;
    /// Reset supervisor stack pointer used by the cross-check harness.
    const INITIAL_STACK: u32 = 0x00FF_FF00;
    /// Size of the test cartridge image.
    const ROM_LEN: usize = 0x400;

    /// One instruction form the CPU executes, with the state needed to run it.
    struct Case {
        /// Instructions executed before the case to establish flags or a
        /// register value.
        prefix: &'static [u8],
        /// The encoded instruction under test.
        instruction: &'static [u8],
        /// Bytes placed at the initial stack pointer before the step.
        stack: &'static [u8],
        /// Exact documented text of the disassembled instruction.
        text: &'static str,
    }

    const fn case(
        prefix: &'static [u8],
        instruction: &'static [u8],
        stack: &'static [u8],
        text: &'static str,
    ) -> Case {
        Case {
            prefix,
            instruction,
            stack,
            text,
        }
    }

    /// Every instruction form the CPU executes, with the flags/stack setup that
    /// makes one step succeed and, for control flow, land exactly on the address
    /// implied by the instruction length.
    const CASES: &[Case] = &[
        // MOVEQ, all unconditional and conditional branches and BSR. A zero
        // branch displacement makes the taken target equal the next address, so
        // the observed PC delta equals the encoded length.
        case(&[], &[0x72, 0xFE], &[], "MOVEQ #$FE,D1"),
        case(&[], &[0x60, 0x00], &[], "BRA $00000202"),
        case(&[], &[0x61, 0x00], &[], "BSR $00000202"),
        case(&[], &[0x64, 0x00], &[], "BCC $00000202"),
        // CMP.B D1,D0 with D0 = 0 and D1 = 1 sets C and N without touching X.
        case(
            &[0x70, 0x00, 0x72, 0x01, 0xB0, 0x01],
            &[0x65, 0x00],
            &[],
            "BCS $00000208",
        ),
        case(&[0x70, 0x00], &[0x67, 0x00], &[], "BEQ $00000204"),
        case(&[0x70, 0x01], &[0x66, 0x00], &[], "BNE $00000204"),
        case(&[0x70, 0xFF], &[0x6B, 0x00], &[], "BMI $00000204"),
        case(&[0x70, 0x00], &[0x6A, 0x00], &[], "BPL $00000204"),
        // MOVE.W #imm,SR.
        case(&[], &[0x46, 0xFC, 0x20, 0x00], &[], "MOVE.W #$2000,SR"),
        // NOP, RTS and RTE. The stack bytes are the return frame.
        case(&[], &[0x4E, 0x71], &[], "NOP"),
        case(&[], &[0x4E, 0x75], &[0x00, 0x00, 0x02, 0x02], "RTS"),
        case(
            &[],
            &[0x4E, 0x73],
            &[0x27, 0x00, 0x00, 0x00, 0x02, 0x02],
            "RTE",
        ),
        // JMP/JSR to the address immediately after the instruction.
        case(
            &[],
            &[0x4E, 0xF9, 0x00, 0x00, 0x02, 0x06],
            &[],
            "JMP ($00000206).L",
        ),
        case(
            &[],
            &[0x4E, 0xB9, 0x00, 0x00, 0x02, 0x06],
            &[],
            "JSR ($00000206).L",
        ),
        // CLR.W / CLR.L.
        case(&[], &[0x42, 0x40], &[], "CLR.W D0"),
        case(&[], &[0x42, 0x80], &[], "CLR.L D0"),
        // LEA / PEA absolute long.
        case(
            &[],
            &[0x47, 0xF9, 0x12, 0x34, 0x56, 0x78],
            &[],
            "LEA ($12345678).L,A3",
        ),
        case(
            &[],
            &[0x48, 0x79, 0x00, 0xFF, 0x10, 0x00],
            &[],
            "PEA ($00FF1000).L",
        ),
        // TST.B/W/L.
        case(&[], &[0x4A, 0x00], &[], "TST.B D0"),
        case(&[], &[0x4A, 0x41], &[], "TST.W D1"),
        case(&[], &[0x4A, 0x82], &[], "TST.L D2"),
        // MOVE.W/L #imm,Dn.
        case(&[], &[0x32, 0x3C, 0x12, 0x34], &[], "MOVE.W #$1234,D1"),
        case(
            &[],
            &[0x22, 0x3C, 0x12, 0x34, 0x56, 0x78],
            &[],
            "MOVE.L #$12345678,D1",
        ),
        // MOVE.B/W/L Dn,Dm.
        case(&[], &[0x16, 0x00], &[], "MOVE.B D0,D3"),
        case(&[], &[0x34, 0x00], &[], "MOVE.W D0,D2"),
        case(&[], &[0x24, 0x00], &[], "MOVE.L D0,D2"),
        // MOVE.B/W/L Dn,(abs).L into work RAM.
        case(
            &[],
            &[0x13, 0xC0, 0x00, 0xFF, 0x00, 0x00],
            &[],
            "MOVE.B D0,($00FF0000).L",
        ),
        case(
            &[],
            &[0x33, 0xC0, 0x00, 0xFF, 0x00, 0x00],
            &[],
            "MOVE.W D0,($00FF0000).L",
        ),
        case(
            &[],
            &[0x23, 0xC0, 0x00, 0xFF, 0x00, 0x00],
            &[],
            "MOVE.L D0,($00FF0000).L",
        ),
        // MOVE.B/W/L (abs).L,Dn from work RAM.
        case(
            &[],
            &[0x10, 0x39, 0x00, 0xFF, 0x00, 0x00],
            &[],
            "MOVE.B ($00FF0000).L,D0",
        ),
        case(
            &[],
            &[0x30, 0x39, 0x00, 0xFF, 0x00, 0x00],
            &[],
            "MOVE.W ($00FF0000).L,D0",
        ),
        case(
            &[],
            &[0x20, 0x39, 0x00, 0xFF, 0x00, 0x00],
            &[],
            "MOVE.L ($00FF0000).L,D0",
        ),
        // CMP.B/W/L Dn,Dn.
        case(&[], &[0xB0, 0x01], &[], "CMP.B D1,D0"),
        case(&[], &[0xB0, 0x41], &[], "CMP.W D1,D0"),
        case(&[], &[0xB0, 0x81], &[], "CMP.L D1,D0"),
        // DBRA with counter 1 and displacement 0 branches to the next address.
        case(
            &[0x70, 0x01],
            &[0x51, 0xC8, 0x00, 0x00],
            &[],
            "DBRA D0,$00000206",
        ),
        // ADDQ/SUBQ word and long.
        case(&[], &[0x52, 0x40], &[], "ADDQ.W #1,D0"),
        case(&[], &[0x52, 0x80], &[], "ADDQ.L #1,D0"),
        case(&[], &[0x53, 0x40], &[], "SUBQ.W #1,D0"),
        case(&[], &[0x53, 0x80], &[], "SUBQ.L #1,D0"),
        // ADD/SUB byte, word and long.
        case(&[], &[0xD0, 0x01], &[], "ADD.B D1,D0"),
        case(&[], &[0xD0, 0x41], &[], "ADD.W D1,D0"),
        case(&[], &[0xD0, 0x81], &[], "ADD.L D1,D0"),
        case(&[], &[0x90, 0x01], &[], "SUB.B D1,D0"),
        case(&[], &[0x90, 0x41], &[], "SUB.W D1,D0"),
        case(&[], &[0x90, 0x81], &[], "SUB.L D1,D0"),
    ];

    /// Builds the test cartridge image with `prefix` and `instruction` at the
    /// program entry point.
    fn program_rom(prefix: &[u8], instruction: &[u8]) -> Arc<[u8]> {
        assert_eq!(prefix.len() % 2, 0, "prefix must be whole instructions");
        let mut rom = vec![0_u8; ROM_LEN];
        rom[0..4].copy_from_slice(&INITIAL_STACK.to_be_bytes());
        rom[4..8].copy_from_slice(&PROGRAM_ADDRESS.to_be_bytes());
        let start = PROGRAM_ADDRESS as usize;
        rom[start..start + prefix.len()].copy_from_slice(prefix);
        let instruction_start = start + prefix.len();
        rom[instruction_start..instruction_start + instruction.len()].copy_from_slice(instruction);
        Arc::from(rom.into_boxed_slice())
    }

    /// Executes one CPU step from the same bytes and proves the view matches it.
    fn cross_check(case: &Case) {
        let rom = program_rom(case.prefix, case.instruction);
        let mut bus = MegadriveBus::new(Arc::clone(&rom));
        if !case.stack.is_empty() {
            bus.write_memory(u64::from(INITIAL_STACK), case.stack)
                .expect("stack setup");
        }
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).expect("reset");
        for _ in 0..case.prefix.len() / 2 {
            cpu.step(&mut bus).expect("prefix step");
        }

        let instruction_address = PROGRAM_ADDRESS + u32::try_from(case.prefix.len()).expect("fit");
        assert_eq!(cpu.pc(), instruction_address, "prefix advanced the PC");

        // Disassemble from the very bytes the CPU is about to fetch. The
        // borrow of `bus` ends before the mutable step below.
        let record = {
            let memory = RomMemory::new(&rom);
            let mut decoded = disassemble(&memory, instruction_address, 1).expect("decode");
            assert_eq!(decoded.len(), 1);
            decoded.swap_remove(0)
        };
        assert_eq!(record.address(), instruction_address);
        assert_eq!(record.text(), case.text);
        assert_eq!(
            record.opcode(),
            u16::from_be_bytes([case.instruction[0], case.instruction[1]])
        );

        cpu.enable_trace(4).expect("trace");
        let before = cpu.pc();
        cpu.step(&mut bus).expect(case.text);
        let after = cpu.pc();
        assert_eq!(
            i64::from(after) - i64::from(before),
            i64::from(record.length()),
            "{} length must equal the CPU PC delta",
            record.text()
        );
        let trace = cpu.trace().expect("trace enabled");
        let entry = trace.entries().next().expect("one trace record");
        assert_eq!(entry.pc(), before);
        assert_eq!(entry.opcode(), record.opcode());
    }

    #[test]
    fn every_supported_encoding_matches_execution_and_documented_text() {
        for case in CASES {
            cross_check(case);
        }
    }

    #[test]
    fn unsupported_encodings_render_as_data_words_and_the_cpu_rejects_them() {
        // Each pair is an opcode word the CPU does not execute and the error
        // kind one step from the same bytes produces.
        let cases: &[(u16, &str)] = &[
            (0x4AFC, "DC.W $4AFC"), // ILLEGAL
            (0x6200, "DC.W $6200"), // BHI: unsupported branch condition
            (0x4878, "DC.W $4878"), // PEA (abs).W
            (0x41F8, "DC.W $41F8"), // LEA (abs).W
            (0x41FA, "DC.W $41FA"), // LEA (d16,PC)
            (0x50C0, "DC.W $50C0"), // Scc-style ADDQ form with a size field
            (0x4E74, "DC.W $4E74"), // RTR is not implemented
            (0xE000, "DC.W $E000"), // ASR is not implemented
        ];
        for (opcode, text) in cases {
            let bytes = opcode.to_be_bytes();
            let rom = program_rom(&[], &bytes);
            let memory = RomMemory::new(&rom);
            let decoded = disassemble(&memory, PROGRAM_ADDRESS, 1).expect("decode");
            assert_eq!(decoded[0].text(), *text);
            assert_eq!(decoded[0].length(), 2);
            assert_eq!(decoded[0].opcode(), *opcode);

            let mut bus = MegadriveBus::new(Arc::clone(&rom));
            let mut cpu = M68000::new();
            cpu.reset(&mut bus).expect("reset");
            assert!(
                cpu.step(&mut bus).is_err(),
                "0x{opcode:04X} must not execute in this slice"
            );
        }
    }

    #[test]
    fn data_word_fallback_does_not_read_an_extension_word() {
        // The window holds only the opcode word; a two-byte fallback must not
        // require anything beyond it.
        let memory = WindowMemory::new(PROGRAM_ADDRESS, &[0x4A, 0xFC]);
        let decoded = disassemble(&memory, PROGRAM_ADDRESS, 1).expect("decode");
        assert_eq!(decoded[0].text(), "DC.W $4AFC");
        assert_eq!(decoded[0].length(), 2);
    }

    #[test]
    fn a_count_at_the_limit_is_accepted_and_a_lower_count_truncates_the_view() {
        let bytes = [0x4E_u8, 0x71].repeat(MAX_DISASSEMBLY_COUNT);
        let memory = WindowMemory::new(0, &bytes);
        let decoded = disassemble(&memory, 0, MAX_DISASSEMBLY_COUNT).expect("maximum count");
        assert_eq!(decoded.len(), MAX_DISASSEMBLY_COUNT);
        assert_eq!(decoded[MAX_DISASSEMBLY_COUNT - 1].address(), 0x1FFE);
        assert_eq!(decoded[MAX_DISASSEMBLY_COUNT - 1].text(), "NOP");

        let decoded = disassemble(&memory, 0, 2).expect("short count");
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1].address(), 2);
    }

    #[test]
    fn invalid_requests_are_typed_errors() {
        let memory = WindowMemory::new(PROGRAM_ADDRESS, &[0x4E, 0x71, 0x4E, 0x71]);
        assert_eq!(
            disassemble(&memory, PROGRAM_ADDRESS, 0),
            Err(DisasmError::InvalidCount { count: 0 })
        );
        assert_eq!(
            disassemble(&memory, PROGRAM_ADDRESS, MAX_DISASSEMBLY_COUNT + 1),
            Err(DisasmError::InvalidCount {
                count: MAX_DISASSEMBLY_COUNT + 1
            })
        );
        assert_eq!(
            disassemble(&memory, PROGRAM_ADDRESS + 1, 1),
            Err(DisasmError::OddAddress {
                address: PROGRAM_ADDRESS + 1
            })
        );
        assert_eq!(
            disassemble(&memory, PROGRAM_ADDRESS + 4, 1),
            Err(DisasmError::UnavailableWord {
                address: PROGRAM_ADDRESS + 4
            })
        );
        // A half word at the end of a window cannot supply an opcode word.
        let memory = WindowMemory::new(PROGRAM_ADDRESS, &[0x4E]);
        assert_eq!(
            disassemble(&memory, PROGRAM_ADDRESS, 1),
            Err(DisasmError::UnavailableWord {
                address: PROGRAM_ADDRESS
            })
        );
    }

    #[test]
    fn missing_extension_words_are_typed_errors() {
        // LEA (abs).L,An needs two extension words after its opcode.
        let memory = WindowMemory::new(PROGRAM_ADDRESS, &[0x41, 0xF9]);
        assert_eq!(
            disassemble(&memory, PROGRAM_ADDRESS, 1),
            Err(DisasmError::UnavailableWord {
                address: PROGRAM_ADDRESS + 2
            })
        );

        // A long immediate whose low word is truncated reports that word.
        let memory = WindowMemory::new(PROGRAM_ADDRESS, &[0x22, 0x3C, 0x12, 0x34]);
        assert_eq!(
            disassemble(&memory, PROGRAM_ADDRESS, 1),
            Err(DisasmError::UnavailableWord {
                address: PROGRAM_ADDRESS + 4
            })
        );
    }

    #[test]
    fn address_overflow_is_a_typed_error() {
        // Stepping past 0xFFFF_FFFF after the first instruction overflows.
        let memory = WindowMemory::new(0xFFFF_FFFE, &[0x4E, 0x71]);
        let decoded = disassemble(&memory, 0xFFFF_FFFE, 1).expect("last even address");
        assert_eq!(decoded[0].address(), 0xFFFF_FFFE);
        assert_eq!(
            disassemble(&memory, 0xFFFF_FFFE, 2),
            Err(DisasmError::AddressOverflow {
                address: 0xFFFF_FFFE
            })
        );

        // A branch target outside the address space is rejected like the CPU's
        // relative-target overflow.
        let memory = WindowMemory::new(0xFFFF_FFFE, &[0x60, 0x7F]);
        assert_eq!(
            disassemble(&memory, 0xFFFF_FFFE, 1),
            Err(DisasmError::AddressOverflow {
                address: 0xFFFF_FFFE
            })
        );
    }

    #[test]
    fn rom_memory_maps_only_the_cartridge_window() {
        let rom = [0x12_u8, 0x34, 0x56, 0x78];
        let memory = RomMemory::new(&rom);
        assert_eq!(memory.read_byte(0), Some(0x12));
        assert_eq!(memory.read_word(2), Some(0x5678));
        assert_eq!(memory.read_byte(4), None);
        assert_eq!(memory.read_byte(ROM_END), None);
    }

    #[test]
    fn the_supported_subset_is_nonempty_and_covers_every_documented_form() {
        // A guard against silently deleting a case: both the instruction text
        // set and the branch-condition coverage are checked for the forms the
        // ADR names.
        let texts: Vec<&str> = CASES.iter().map(|case| case.text).collect();
        for required in [
            "MOVEQ ",
            "BRA ",
            "BSR ",
            "BCC ",
            "BCS ",
            "BEQ ",
            "BNE ",
            "BMI ",
            "BPL ",
            "MOVE.W #$2000,SR",
            "NOP",
            "RTS",
            "RTE",
            "JMP (",
            "JSR (",
            "CLR.W ",
            "CLR.L ",
            "LEA (",
            "PEA (",
            "TST.B ",
            "TST.W ",
            "TST.L ",
            "MOVE.W #$1234",
            "MOVE.L #$12345678",
            "MOVE.B D0,D3",
            "MOVE.B D0,(",
            "MOVE.B (",
            "CMP.B ",
            "CMP.W ",
            "CMP.L ",
            "DBRA ",
            "ADDQ.W ",
            "SUBQ.W ",
            "ADD.B ",
            "SUB.B ",
        ] {
            assert!(
                texts.iter().any(|text| text.starts_with(required)),
                "missing coverage for {required}"
            );
        }
    }

    #[test]
    fn views_do_not_mutate_the_window_or_the_machine() {
        // The disassembler only reads: the same request twice returns the same
        // text, and disassembling a running machine's ROM does not change any
        // CPU or bus state.
        let rom = program_rom(&[], &[0x70, 0x01]);
        let mut bus = MegadriveBus::new(Arc::clone(&rom));
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).expect("reset");
        let pc_before = cpu.pc();
        let sr_before = cpu.sr();
        let ram_before = bus.ram().to_vec();

        let memory = RomMemory::new(&rom);
        let first = disassemble(&memory, PROGRAM_ADDRESS, 1).expect("first");
        let second = disassemble(&memory, PROGRAM_ADDRESS, 1).expect("second");
        assert_eq!(first, second);

        assert_eq!(cpu.pc(), pc_before);
        assert_eq!(cpu.sr(), sr_before);
        assert_eq!(bus.ram(), ram_before.as_slice());
        // The work RAM window is readable with an explicit base and is not
        // required to hold executable code.
        let ram = WindowMemory::new(RAM_BASE, bus.ram());
        assert_eq!(ram.read_word(RAM_BASE), Some(0x0000));
        assert_eq!(ram.read_byte(RAM_BASE - 1), None);
    }
}
