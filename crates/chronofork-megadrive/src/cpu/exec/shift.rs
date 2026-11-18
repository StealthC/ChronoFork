//! Shift and rotate group: ASL/ASR, LSL/LSR, ROL/ROR and ROXL/ROXR in
//! register and memory forms.
//!
//! The count is masked modulo 64. A count of zero changes no flags at all.
//! Arithmetic and logical shifts and the extend rotates set `X` from the last
//! bit shifted out; the plain rotates leave `X` alone. `V` is set only for an
//! arithmetic left shift whose sign bit changes during the operation, and
//! `N`/`Z` follow the result.

use crate::bus::MegadriveBus;

use super::illegal_opcode;
use crate::cpu::ea::{Ea, Size, decode_ea, modify_ea};
use crate::cpu::{CpuError, FLAG_X, M68000};

const KIND_ARITHMETIC: u8 = 0;
const KIND_LOGICAL: u8 = 1;
const KIND_ROTATE_EXTEND: u8 = 2;
const KIND_ROTATE: u8 = 3;

/// Executes one shift/rotate instruction selected by the decode table.
///
/// Returns the instruction's 68000-cycle cost, which is zero until the timing
/// table (Task 14) lands.
pub fn execute(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    if (opcode & 0xF8C0) == 0xE0C0 {
        return memory_shift(cpu, bus, opcode);
    }
    let instruction_pc = cpu.pc().saturating_sub(2);
    if (opcode & 0xF000) != 0xE000 || (opcode & 0x00C0) == 0x00C0 {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    register_shift(cpu, opcode)
}

fn register_shift(cpu: &mut M68000, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_of(opcode).ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
    let left = opcode & 0x0100 != 0;
    let kind = ((opcode >> 3) & 0x3) as u8;
    let count = if opcode & 0x0020 != 0 {
        let count_register = usize::from((opcode >> 9) & 0x7);
        cpu.read_dn(count_register, Size::Long) % 64
    } else {
        match (opcode >> 9) & 0x7 {
            0 => 8,
            value => u32::from(value),
        }
    };
    if count == 0 {
        return Ok(0);
    }
    let register = usize::from(opcode & 0x7);
    let value = cpu.read_dn(register, size);
    let (result, carry, overflow, extend) =
        shift_value(value, size, left, kind, count, cpu.flag(FLAG_X));
    cpu.write_dn(register, result, size);
    cpu.set_ccr(
        carry,
        overflow,
        result == 0,
        result & size.sign_bit() != 0,
        extend,
    );
    Ok(0)
}

fn memory_shift(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = Size::Word;
    // Memory shifts encode the direction in bit 8 and the operation in bits
    // 10-9; the low six bits are the effective address.
    let left = opcode & 0x0100 != 0;
    let kind = ((opcode >> 9) & 0x3) as u8;
    let ea = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_memory_alterable(ea) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let initial_extend = cpu.flag(FLAG_X);
    let mut flags = (false, false, initial_extend);
    let (_, result) = modify_ea(cpu, bus, ea, size, |value| {
        let (result, carry, overflow, extend) =
            shift_value(value, size, left, kind, 1, initial_extend);
        flags = (carry, overflow, extend);
        result
    })?;
    let (carry, overflow, extend) = flags;
    cpu.set_ccr(
        carry,
        overflow,
        result == 0,
        result & size.sign_bit() != 0,
        extend,
    );
    Ok(0)
}

fn shift_value(
    value: u32,
    size: Size,
    left: bool,
    kind: u8,
    count: u32,
    initial_extend: bool,
) -> (u32, bool, bool, bool) {
    let sign = size.sign_bit();
    let mask = size.mask();
    let mut result = value & mask;
    let mut carry = false;
    let mut overflow = false;
    let mut extend = initial_extend;
    for _ in 0..count {
        match kind {
            KIND_ARITHMETIC => {
                if left {
                    let before = result & sign != 0;
                    carry = result & sign != 0;
                    result = (result << 1) & mask;
                    let after = result & sign != 0;
                    overflow |= before != after;
                } else {
                    carry = result & 1 != 0;
                    let sign_bits = result & sign;
                    result = (result >> 1) | sign_bits;
                }
                extend = carry;
            }
            KIND_LOGICAL => {
                if left {
                    carry = result & sign != 0;
                    result = (result << 1) & mask;
                } else {
                    carry = result & 1 != 0;
                    result >>= 1;
                }
                extend = carry;
            }
            KIND_ROTATE_EXTEND => {
                if left {
                    let outgoing = result & sign != 0;
                    result = ((result << 1) & mask) | u32::from(extend);
                    extend = outgoing;
                    carry = outgoing;
                } else {
                    let outgoing = result & 1 != 0;
                    result = (result >> 1) | if extend { sign } else { 0 };
                    extend = outgoing;
                    carry = outgoing;
                }
            }
            KIND_ROTATE => {
                if left {
                    let outgoing = result & sign != 0;
                    result = ((result << 1) & mask) | u32::from(outgoing);
                    carry = outgoing;
                } else {
                    let outgoing = result & 1 != 0;
                    result = (result >> 1) | if outgoing { sign } else { 0 };
                    carry = outgoing;
                }
            }
            _ => {}
        }
    }
    (result, carry, overflow, extend)
}

fn size_of(opcode: u16) -> Option<Size> {
    match (opcode >> 6) & 0x3 {
        0 => Some(Size::Byte),
        1 => Some(Size::Word),
        2 => Some(Size::Long),
        _ => None,
    }
}

fn is_memory_alterable(ea: Ea) -> bool {
    matches!(
        ea,
        Ea::Indirect(_)
            | Ea::PostInc(_)
            | Ea::PreDec(_)
            | Ea::Disp16(_)
            | Ea::Index(_)
            | Ea::Abs16
            | Ea::Abs32
    )
}
