//! The MC68000 instruction timing table.
//!
//! Cycle counts are 68000 clock cycles, not master cycles: the backend owns
//! the `cycles * 7` master-clock conversion. The table follows the documented
//! base and effective-address times. Effective-address cost is added exactly
//! once per operand; an instruction that reads and writes the same operand
//! (for example `ADD Dn,(An)`) pays it once.

use super::decode::{self, InstructionClass};
use super::ea::{Ea, Size};

/// The result of executing one instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstructionOutcome {
    /// The instruction's 68000-cycle cost.
    pub cycles: u16,
}

/// Returns the documented effective-address calculation time.
///
/// The byte and word columns of the MC68000 timing table are identical; only
/// `long` differs. `is_read` is part of the table's interface so a later phase
/// can distinguish read and write penalties where a device adds them.
#[must_use]
pub fn ea_cycles(ea: &Ea, size: Size, _is_read: bool) -> u16 {
    let word = match ea {
        Ea::Dn(_) | Ea::An(_) => 0,
        Ea::Indirect(_) | Ea::PostInc(_) => 4,
        Ea::PreDec(_) => 6,
        Ea::Disp16(_) | Ea::Abs16 | Ea::PcDisp16 => 8,
        Ea::Index(_) | Ea::PcIndex => 10,
        Ea::Abs32 => 12,
        Ea::Imm => 4,
    };
    if size != Size::Long {
        return word;
    }
    match ea {
        Ea::Dn(_) | Ea::An(_) => 0,
        Ea::Indirect(_) | Ea::PostInc(_) => 8,
        Ea::PreDec(_) => 10,
        Ea::Disp16(_) | Ea::Abs16 | Ea::PcDisp16 => 12,
        Ea::Index(_) | Ea::PcIndex => 14,
        Ea::Abs32 => 16,
        Ea::Imm => 8,
    }
}

/// Returns the documented base time for `opcode`, excluding effective-address
/// calculation.
///
/// `None` means the opcode is not an implemented instruction.
#[must_use]
pub fn base_cycles(opcode: u16) -> Option<u16> {
    let class = decode::classify(opcode);
    let bits = (opcode >> 6) & 0x3;
    let opmode = (opcode >> 6) & 0x7;
    Some(match class {
        InstructionClass::Moveq => 4,
        InstructionClass::Branch => match (opcode >> 8) & 0xF {
            0x0 => 10,
            0x1 => 18,
            _ => 8,
        },
        InstructionClass::Nop => 4,
        InstructionClass::Rte => 20,
        InstructionClass::Rts => 16,
        InstructionClass::Jmp => 4,
        InstructionClass::Jsr => 12,
        InstructionClass::Move => 4,
        InstructionClass::Movem => {
            if opcode & 0x0040 != 0 {
                12
            } else {
                8
            }
        }
        InstructionClass::Movep => {
            if opcode & 0x0040 != 0 {
                24
            } else {
                16
            }
        }
        InstructionClass::Lea => 4,
        InstructionClass::Pea => 12,
        InstructionClass::Clr
        | InstructionClass::Tst
        | InstructionClass::Neg
        | InstructionClass::Negx => {
            if bits == 2 {
                6
            } else {
                4
            }
        }
        InstructionClass::Ext | InstructionClass::Swap => 4,
        InstructionClass::Add
        | InstructionClass::Adda
        | InstructionClass::Sub
        | InstructionClass::Suba => match opmode {
            0..=2 => {
                if opmode == 2 {
                    6
                } else {
                    4
                }
            }
            3 | 7 => {
                if opmode == 7 {
                    6
                } else {
                    8
                }
            }
            4..=6 => {
                if opmode == 6 {
                    12
                } else {
                    8
                }
            }
            _ => 0,
        },
        InstructionClass::Addq | InstructionClass::Subq => {
            if (opcode >> 3) & 0x7 == 1 || bits == 2 {
                8
            } else {
                4
            }
        }
        InstructionClass::Addi | InstructionClass::Subi => {
            if bits == 2 {
                16
            } else {
                8
            }
        }
        InstructionClass::Cmp => {
            if bits == 2 {
                6
            } else {
                4
            }
        }
        InstructionClass::Cmpa => 6,
        InstructionClass::Cmpi => {
            if bits == 2 {
                14
            } else {
                8
            }
        }
        InstructionClass::Cmpm => {
            if bits == 2 {
                20
            } else {
                12
            }
        }
        InstructionClass::And | InstructionClass::Or => {
            if opcode & 0x0100 != 0 {
                if bits == 2 { 12 } else { 8 }
            } else if bits == 2 {
                6
            } else {
                4
            }
        }
        InstructionClass::Andi | InstructionClass::Ori | InstructionClass::Eori => {
            if bits == 2 {
                16
            } else {
                8
            }
        }
        InstructionClass::Eor => {
            if bits == 2 {
                12
            } else {
                8
            }
        }
        InstructionClass::Not => {
            if bits == 2 {
                6
            } else {
                4
            }
        }
        InstructionClass::MulDiv => {
            if opcode & 0xF000 == 0xC000 {
                70
            } else {
                140
            }
        }
        InstructionClass::Bcd => 6,
        InstructionClass::Bit => 4,
        InstructionClass::Shift => {
            if bits == 2 {
                8
            } else {
                6
            }
        }
        InstructionClass::Dbra => 10,
        InstructionClass::Scc => 4,
        InstructionClass::System => match opcode {
            0x4E70 => 132,
            0x4E72 => 4,
            0x4E76 => 4,
            0x4E77 => 20,
            0x4AFC => 34,
            _ if (opcode & 0xFFF0) == 0x4E40 => 34,
            _ if (opcode & 0xFFF8) == 0x4E50 => 16,
            _ if (opcode & 0xFFF8) == 0x4E58 => 12,
            _ if (opcode & 0xFFF8) == 0x4E60 || (opcode & 0xFFF8) == 0x4E68 => 4,
            _ if (opcode & 0xF1C0) == 0x4180 => 10,
            _ if matches!(opcode, 0x003C | 0x007C | 0x023C | 0x027C | 0x0A3C | 0x0A7C) => 8,
            _ if matches!(opcode & 0xFFC0, 0x40C0 | 0x42C0 | 0x44C0 | 0x46C0) => 4,
            _ => 4,
        },
        InstructionClass::Other => return None,
    })
}

/// Returns the complete documented cycle cost of `opcode`, adding each
/// effective-address calculation exactly once.
#[must_use]
pub(crate) fn instruction_cycles(opcode: u16) -> u16 {
    let class = decode::classify(opcode);
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    let mut cycles = base_cycles(opcode).unwrap_or(0);
    match class {
        InstructionClass::Move => {
            let size = match opcode >> 12 {
                1 => Size::Byte,
                2 => Size::Long,
                _ => Size::Word,
            };
            if let Ok(ea) = super::decode_ea(mode, register, size) {
                cycles += ea_cycles(&ea, size, true);
            }
            let destination_mode = ((opcode >> 6) & 0x7) as u8;
            let destination_register = ((opcode >> 9) & 0x7) as u8;
            if destination_mode != 1 {
                if let Ok(ea) = super::decode_ea(destination_mode, destination_register, size) {
                    cycles += ea_cycles(&ea, size, false);
                }
            }
        }
        InstructionClass::Movem => {
            let size = if opcode & 0x0040 != 0 {
                Size::Long
            } else {
                Size::Word
            };
            if let Ok(ea) = super::decode_ea(mode, register, size) {
                cycles += ea_cycles(&ea, size, true);
            }
        }
        InstructionClass::Clr
        | InstructionClass::Tst
        | InstructionClass::Neg
        | InstructionClass::Negx
        | InstructionClass::Not => {
            let size = match bits_of(opcode) {
                0 => Size::Byte,
                1 => Size::Word,
                _ => Size::Long,
            };
            if let Ok(ea) = super::decode_ea(mode, register, size) {
                cycles += ea_cycles(&ea, size, false);
            }
        }
        InstructionClass::Scc => {
            if let Ok(ea) = super::decode_ea(mode, register, Size::Byte) {
                cycles += ea_cycles(&ea, Size::Byte, false);
            }
        }
        InstructionClass::Bit => {
            let size = if mode == 0 { Size::Long } else { Size::Byte };
            if let Ok(ea) = super::decode_ea(mode, register, size) {
                cycles += ea_cycles(&ea, size, false);
            }
        }
        InstructionClass::Add
        | InstructionClass::Adda
        | InstructionClass::Sub
        | InstructionClass::Suba
        | InstructionClass::Addq
        | InstructionClass::Subq
        | InstructionClass::Addi
        | InstructionClass::Subi
        | InstructionClass::Cmp
        | InstructionClass::Cmpa
        | InstructionClass::Cmpi
        | InstructionClass::And
        | InstructionClass::Or
        | InstructionClass::Andi
        | InstructionClass::Ori
        | InstructionClass::Eor
        | InstructionClass::Eori
        | InstructionClass::Lea
        | InstructionClass::Pea
        | InstructionClass::Jmp
        | InstructionClass::Jsr => {
            let size = match class {
                InstructionClass::Cmp | InstructionClass::And | InstructionClass::Or => {
                    match bits_of(opcode) {
                        0 => Size::Byte,
                        1 => Size::Word,
                        _ => Size::Long,
                    }
                }
                InstructionClass::Lea
                | InstructionClass::Pea
                | InstructionClass::Jmp
                | InstructionClass::Jsr => Size::Long,
                _ => match bits_of(opcode) {
                    0 => Size::Byte,
                    1 => Size::Word,
                    _ => Size::Long,
                },
            };
            if let Ok(ea) = super::decode_ea(mode, register, size) {
                cycles += ea_cycles(&ea, size, true);
            }
        }
        InstructionClass::Shift => {
            if bits_of(opcode) == 3 {
                if let Ok(ea) = super::decode_ea(mode, register, Size::Word) {
                    cycles += ea_cycles(&ea, Size::Word, false);
                }
            }
        }
        InstructionClass::Bcd => {
            if (opcode & 0xFFC0) == 0x4800 {
                if let Ok(ea) = super::decode_ea(mode, register, Size::Byte) {
                    cycles += ea_cycles(&ea, Size::Byte, false);
                }
            }
        }
        InstructionClass::System => {
            let has_ea_operand = (opcode & 0xF1C0) == 0x4180
                || matches!(opcode & 0xFFC0, 0x40C0 | 0x42C0 | 0x44C0 | 0x46C0);
            let ea = has_ea_operand
                .then(|| super::decode_ea(mode, register, Size::Word))
                .and_then(Result::ok);
            if let Some(ea) = ea {
                cycles += ea_cycles(&ea, Size::Word, true);
            }
        }
        _ => {}
    }
    cycles
}

fn bits_of(opcode: u16) -> u16 {
    (opcode >> 6) & 0x3
}
