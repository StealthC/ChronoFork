//! The MC68000 instruction timing table.
//!
//! Cycle counts are 68000 clock cycles, not master cycles: the backend owns
//! the `cycles * 7` master-clock conversion. The table follows the documented
//! base and effective-address times. Effective-address cost is added exactly
//! once per operand; an instruction that reads and writes the same operand
//! (for example `ADD Dn,(An)`) pays it once.
//!
//! Most costs are a function of the opcode alone. Four forms also need state
//! the execution groups already have: a conditional branch's outcome, a
//! register shift's count, the source word of `MULU`/`MULS`, and the number of
//! registers transferred by `MOVEM`. Those groups call the matching helper
//! (`branch_cycles`, `shift_cycles`, `multiply_divide_cycles`, `movem_cycles`)
//! with the observed value; `instruction_cycles` keeps a state-free fallback
//! for the remaining instructions.

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
/// `None` means the opcode is not an implemented instruction. For the four
/// forms whose total cost also needs execution state, this is the state-free
/// part of the documented cost; the execution groups use the matching
/// state-aware helper for the total.
#[must_use]
pub fn base_cycles(opcode: u16) -> Option<u16> {
    let class = decode::classify(opcode);
    let bits = (opcode >> 6) & 0x3;
    let opmode = (opcode >> 6) & 0x7;
    let mode = (opcode >> 3) & 0x7;
    Some(match class {
        InstructionClass::Moveq => 4,
        InstructionClass::Branch => branch_cycles(opcode, false),
        InstructionClass::Nop => 4,
        InstructionClass::Rte => 20,
        InstructionClass::Rts => 16,
        InstructionClass::Jmp => jmp_cycles(opcode),
        InstructionClass::Jsr => jsr_cycles(opcode),
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
        InstructionClass::Lea => lea_cycles(opcode),
        InstructionClass::Pea => pea_cycles(opcode),
        InstructionClass::Clr
        | InstructionClass::Neg
        | InstructionClass::Negx
        | InstructionClass::Not => {
            if mode == 0 {
                if bits == 2 { 6 } else { 4 }
            } else if bits == 2 {
                12
            } else {
                8
            }
        }
        InstructionClass::Tst => 4,
        InstructionClass::Ext | InstructionClass::Swap => 4,
        InstructionClass::Add
        | InstructionClass::Adda
        | InstructionClass::Sub
        | InstructionClass::Suba => match opmode {
            3 => 8,
            2 if is_register_source(mode) => 8,
            2 => 6,
            7 if is_register_source(mode) => 8,
            7 => 6,
            0 | 1 => 4,
            6 => 12,
            _ => 8,
        },
        InstructionClass::Addq | InstructionClass::Subq => {
            let size = size_of_bits(bits);
            if mode == 1 {
                8
            } else if mode == 0 {
                if size == Size::Long { 8 } else { 4 }
            } else if size == Size::Long {
                12
            } else {
                8
            }
        }
        InstructionClass::Addi | InstructionClass::Subi => {
            let size = size_of_bits(bits);
            if mode == 0 {
                if size == Size::Long { 16 } else { 8 }
            } else if size == Size::Long {
                20
            } else {
                12
            }
        }
        InstructionClass::Addx | InstructionClass::Subx => {
            if mode == 0 {
                if opmode == 6 { 8 } else { 4 }
            } else if opmode == 6 {
                30
            } else {
                18
            }
        }
        InstructionClass::Exg => 6,
        InstructionClass::Tas => {
            if mode == 0 {
                4
            } else {
                10
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
            let size = size_of_bits(bits);
            if mode == 0 {
                if size == Size::Long { 14 } else { 8 }
            } else if size == Size::Long {
                12
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
            } else if opmode == 2 {
                if is_register_source(mode) { 8 } else { 6 }
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
        InstructionClass::MulDiv => {
            if opcode & 0xF000 == 0xC000 {
                38
            } else if opcode & 0x0100 != 0 {
                168
            } else {
                140
            }
        }
        InstructionClass::Bcd => {
            if (opcode & 0xFFC0) == 0x4800 {
                if mode == 0 { 6 } else { 8 }
            } else if opcode & 0x8 != 0 {
                18
            } else {
                6
            }
        }
        InstructionClass::Bit => 4,
        InstructionClass::Shift => {
            if is_memory_shift(opcode) || bits == 2 {
                8
            } else {
                6
            }
        }
        InstructionClass::Dbra => 10,
        InstructionClass::Scc => {
            if mode == 0 {
                4
            } else {
                8
            }
        }
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
            _ if matches!(opcode, 0x003C | 0x007C | 0x023C | 0x027C | 0x0A3C | 0x0A7C) => 20,
            _ if matches!(opcode & 0xFFC0, 0x40C0 | 0x42C0 | 0x44C0 | 0x46C0) => {
                match opcode & 0xFFC0 {
                    // MOVE from SR: 8 for Dn/An, 6 for memory destinations. The
                    // pinned oracle charges the memory form as a flat 6 cycles.
                    0x40C0 => {
                        if mode == 0 || mode == 1 {
                            8
                        } else {
                            6
                        }
                    }
                    // MOVE from CCR is a 68010 extension, outside the MC68000
                    // table; keep its existing nominal base.
                    0x42C0 => 4,
                    // MOVE to SR and MOVE to CCR: 12 + source EA.
                    _ => 12,
                }
            }
            _ => 4,
        },
        InstructionClass::Other => return None,
    })
}

/// Returns the complete conditional-branch cost.
///
/// A taken `Bcc` costs 10 cycles for both displacement sizes; an untaken one
/// costs 8 with an 8-bit displacement and 12 with a 16-bit displacement. `BRA`
/// is always taken (10) and `BSR` costs 18.
#[must_use]
pub fn branch_cycles(opcode: u16, taken: bool) -> u16 {
    match (opcode >> 8) & 0xF {
        0x0 => 10,
        0x1 => 18,
        _ if taken => 10,
        _ if opcode & 0xFF == 0 => 12,
        _ => 8,
    }
}

/// Returns the complete register or memory shift/rotate cost.
///
/// Register forms are `6 + 2n` for byte/word and `8 + 2n` for long, where `n`
/// is the count after the modulo-64 mask. Memory forms are word only and cost
/// the 8-cycle base plus the effective-address time.
#[must_use]
pub fn shift_cycles(opcode: u16, count: u32) -> u16 {
    if is_memory_shift(opcode) {
        let mode = ((opcode >> 3) & 0x7) as u8;
        let register = (opcode & 0x7) as u8;
        let mut cycles = 8;
        if let Ok(ea) = super::decode_ea(mode, register, Size::Word) {
            cycles += ea_cycles(&ea, Size::Word, false);
        }
        return cycles;
    }
    let base = if bits_of(opcode) == 2 { 8 } else { 6 };
    base + (count as u16).saturating_mul(2)
}

/// Returns the complete `MULU`/`MULS`/`DIVU`/`DIVS` cost.
///
/// `source` is the word the instruction read from its effective address.
/// `MULU` costs `38 + 2n` where `n` is the source's set-bit count; `MULS` costs
/// `38 + 2n` where `n` counts bit transitions from an assumed leading zero.
/// `DIVU`/`DIVS` keep their documented worst-case `140`/`168` base. The source
/// effective-address time is added in every case.
#[must_use]
pub fn multiply_divide_cycles(opcode: u16, source: u16) -> u16 {
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    let mut cycles = if opcode & 0xF000 == 0xC000 {
        let steps = if opcode & 0x0100 != 0 {
            sign_transitions(source)
        } else {
            source.count_ones()
        };
        38 + (steps as u16).saturating_mul(2)
    } else if opcode & 0x0100 != 0 {
        168
    } else {
        140
    };
    if let Ok(ea) = super::decode_ea(mode, register, Size::Word) {
        cycles += ea_cycles(&ea, Size::Word, true);
    }
    cycles
}

/// Returns the complete `MOVEM` cost for `register_count` transferred registers.
///
/// The documented tables charge a base that depends on the direction and
/// addressing mode plus 4 cycles per word or 8 cycles per long register:
/// registers-to-memory is 8+ per register for `(An)`/`-(An)` and
/// memory-to-registers is 12+ per register for `(An)`/`(An)+`.
#[must_use]
pub fn movem_cycles(opcode: u16, register_count: u16) -> u16 {
    let long = opcode & 0x0040 != 0;
    let memory_to_registers = opcode & 0x0400 != 0;
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    let Ok(ea) = super::decode_ea(mode, register, if long { Size::Long } else { Size::Word })
    else {
        return 0;
    };
    let base = match (&ea, memory_to_registers) {
        (Ea::Indirect(_) | Ea::PostInc(_), true) => 12,
        (Ea::Disp16(_) | Ea::Abs16 | Ea::PcDisp16, true) => 16,
        (Ea::Index(_) | Ea::PcIndex, true) => 18,
        (Ea::Abs32, true) => 20,
        (Ea::Indirect(_) | Ea::PreDec(_), false) => 8,
        (Ea::Disp16(_) | Ea::Abs16, false) => 12,
        (Ea::Index(_), false) => 14,
        (Ea::Abs32, false) => 16,
        _ => 0,
    };
    let per_register = if long { 8 } else { 4 };
    base + register_count.saturating_mul(per_register)
}

/// Returns the complete cost of `LEA <ea>,An`.
#[must_use]
pub fn lea_cycles(opcode: u16) -> u16 {
    match decode_control_ea(opcode) {
        Some(Ea::Indirect(_)) => 4,
        Some(Ea::Disp16(_) | Ea::Abs16 | Ea::PcDisp16) => 8,
        Some(Ea::Index(_) | Ea::PcIndex | Ea::Abs32) => 12,
        _ => 0,
    }
}

/// Returns the complete cost of `PEA <ea>`.
#[must_use]
pub fn pea_cycles(opcode: u16) -> u16 {
    match decode_control_ea(opcode) {
        Some(Ea::Indirect(_)) => 12,
        Some(Ea::Disp16(_) | Ea::Abs16 | Ea::PcDisp16) => 16,
        Some(Ea::Index(_) | Ea::PcIndex | Ea::Abs32) => 20,
        _ => 0,
    }
}

/// Returns the complete cost of `JMP <ea>`.
#[must_use]
pub fn jmp_cycles(opcode: u16) -> u16 {
    match decode_control_ea(opcode) {
        Some(Ea::Indirect(_)) => 8,
        Some(Ea::Disp16(_) | Ea::Abs16 | Ea::PcDisp16) => 10,
        Some(Ea::Abs32) => 12,
        Some(Ea::Index(_) | Ea::PcIndex) => 14,
        _ => 0,
    }
}

/// Returns the complete cost of `JSR <ea>`.
#[must_use]
pub fn jsr_cycles(opcode: u16) -> u16 {
    match decode_control_ea(opcode) {
        Some(Ea::Indirect(_)) => 16,
        Some(Ea::Disp16(_) | Ea::Abs16 | Ea::PcDisp16) => 18,
        Some(Ea::Abs32) => 20,
        Some(Ea::Index(_) | Ea::PcIndex) => 22,
        _ => 0,
    }
}

/// Returns the complete documented cycle cost of `opcode` without execution
/// state, adding each effective-address calculation exactly once.
#[must_use]
pub(crate) fn instruction_cycles(opcode: u16) -> u16 {
    let class = decode::classify(opcode);
    if class == InstructionClass::Branch {
        return branch_cycles(opcode, false);
    }
    if class == InstructionClass::Shift {
        let count = if opcode & 0x0020 != 0 {
            0
        } else {
            match (opcode >> 9) & 0x7 {
                0 => 8,
                value => u32::from(value),
            }
        };
        return shift_cycles(opcode, count);
    }
    if class == InstructionClass::MulDiv {
        return multiply_divide_cycles(opcode, 0);
    }
    if class == InstructionClass::Movem {
        return movem_cycles(opcode, 0);
    }
    let mut cycles = base_cycles(opcode).unwrap_or(0);
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    match class {
        InstructionClass::Move => {
            let size = move_size(opcode);
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
        InstructionClass::Clr
        | InstructionClass::Tst
        | InstructionClass::Neg
        | InstructionClass::Negx
        | InstructionClass::Not => {
            let size = size_of_bits(bits_of(opcode));
            if let Ok(ea) = super::decode_ea(mode, register, size) {
                cycles += ea_cycles(&ea, size, false);
            }
        }
        InstructionClass::Scc => {
            if let Ok(ea) = super::decode_ea(mode, register, Size::Byte) {
                cycles += ea_cycles(&ea, Size::Byte, false);
            }
        }
        InstructionClass::Tas => {
            if mode != 0 {
                if let Ok(ea) = super::decode_ea(mode, register, Size::Byte) {
                    cycles += ea_cycles(&ea, Size::Byte, false);
                }
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
        | InstructionClass::Eori => {
            let size = match class {
                InstructionClass::Adda | InstructionClass::Suba | InstructionClass::Cmpa => {
                    if (opcode >> 6) & 0x7 == 3 {
                        Size::Word
                    } else {
                        Size::Long
                    }
                }
                _ => size_of_bits(bits_of(opcode)),
            };
            if let Ok(ea) = super::decode_ea(mode, register, size) {
                cycles += ea_cycles(&ea, size, true);
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
            // CHK and the MOVE-to-SR/CCR forms pay a source effective-address
            // cost. MOVE from SR's documented memory cost is flat (6 cycles),
            // and MOVE from CCR is the 68010 extension kept on its old model.
            let has_ea_operand =
                (opcode & 0xF1C0) == 0x4180 || matches!(opcode & 0xFFC0, 0x42C0 | 0x44C0 | 0x46C0);
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

fn decode_control_ea(opcode: u16) -> Option<Ea> {
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    super::decode_ea(mode, register, Size::Long).ok()
}

fn move_size(opcode: u16) -> Size {
    match opcode >> 12 {
        1 => Size::Byte,
        2 => Size::Long,
        _ => Size::Word,
    }
}

fn is_register_source(mode: u16) -> bool {
    matches!(mode, 0 | 1 | 7)
}

fn is_memory_shift(opcode: u16) -> bool {
    (opcode & 0xF8C0) == 0xE0C0
}

fn sign_transitions(source: u16) -> u32 {
    let mut changes = 0;
    let mut last = false;
    for bit in 0..16 {
        let current = source & (1_u16 << bit) != 0;
        if current != last {
            changes += 1;
            last = current;
        }
    }
    changes
}

fn size_of_bits(bits: u16) -> Size {
    match bits {
        0 => Size::Byte,
        1 => Size::Word,
        _ => Size::Long,
    }
}

fn bits_of(opcode: u16) -> u16 {
    (opcode >> 6) & 0x3
}
