//! Arithmetic group: ADD/ADDA/ADDQ/ADDI, SUB/SUBA/SUBQ/SUBI,
//! CMP/CMPA/CMPI/CMPM, NEG and NEGX.
//!
//! Carry and overflow are computed for each operand width. `ADD`/`SUB` (and
//! their immediate, quick and `NEG` forms) set `X` from the carry; the compare
//! family leaves `X` unchanged. `ADDA`/`SUBA` and `ADDQ`/`SUBQ` to an address
//! register use the full 32-bit register and do not touch the flags.

use crate::bus::MegadriveBus;

use super::illegal_opcode;
use crate::cpu::ea::{Ea, Size, decode_ea, is_data_alterable, modify_ea, read_ea};
use crate::cpu::{CpuError, ExceptionFrame, FLAG_N, FLAG_X, FLAG_Z, M68000, VECTOR_ZERO_DIVIDE};

/// Sets the condition codes for an addition result.
///
/// This is the shared helper the shift and BCD groups use as well; the
/// destination and source are the pre-operation operands.
pub fn add_flags(cpu: &mut M68000, destination: u32, source: u32, result: u32, size: Size) {
    cpu.set_add_flags(destination, source, result, size);
}

/// Sets the condition codes for a subtraction result (`X` follows the carry).
pub fn sub_flags(cpu: &mut M68000, destination: u32, source: u32, result: u32, size: Size) {
    cpu.set_sub_flags(destination, source, result, size);
}

/// Executes one arithmetic instruction selected by the decode table.
///
/// Returns the instruction's 68000-cycle cost, which is zero until the timing
/// table (Task 14) lands.
pub fn execute(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    if matches!(opcode & 0xF1F8, 0xC100 | 0xC108 | 0x8100 | 0x8108) {
        return bcd(cpu, bus, opcode);
    }
    if (opcode & 0xFFC0) == 0x4800 {
        return nbcd(cpu, bus, opcode);
    }
    if matches!(opcode & 0xF1C0, 0xC0C0 | 0xC1C0 | 0x80C0 | 0x81C0) {
        return multiply_or_divide(cpu, bus, opcode);
    }
    match opcode & 0xF000 {
        0xD000 => return add_or_sub(cpu, bus, opcode, true),
        0x9000 => return add_or_sub(cpu, bus, opcode, false),
        0x5000 => return addq_or_subq(cpu, bus, opcode),
        0xB000 => return compare(cpu, bus, opcode),
        _ => {}
    }
    match opcode & 0xFF00 {
        0x0600 => return immediate_arithmetic(cpu, bus, opcode, true),
        0x0400 => return immediate_arithmetic(cpu, bus, opcode, false),
        0x0C00 => return compare_immediate(cpu, bus, opcode),
        0x4000 => return negate(cpu, bus, opcode, true),
        0x4400 => return negate(cpu, bus, opcode, false),
        _ => {}
    }
    Err(illegal_opcode(instruction_pc, opcode))
}

fn bcd(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let subtract = opcode & 0xF000 == 0x8000;
    let memory_form = opcode & 0x8 != 0;
    let destination_register = usize::from((opcode >> 9) & 0x7);
    let source_register = usize::from(opcode & 0x7);
    let extend = u32::from(cpu.flag(FLAG_X));

    let (source, destination, destination_address, source_address) = if memory_form {
        let source_address = cpu
            .read_an(source_register)
            .checked_sub(1)
            .ok_or(CpuError::StackOverflow)?;
        let destination_address = cpu
            .read_an(destination_register)
            .checked_sub(1)
            .ok_or(CpuError::StackOverflow)?;
        let source = u32::from(bus.read8(source_address)?);
        let destination = u32::from(bus.read8(destination_address)?);
        (
            source,
            destination,
            Some(destination_address),
            Some(source_address),
        )
    } else {
        (
            cpu.read_dn(source_register, Size::Byte),
            cpu.read_dn(destination_register, Size::Byte),
            None,
            None,
        )
    };

    let (result, carry) = if subtract {
        bcd_subtract(destination as u8, source as u8, extend as u8)
    } else {
        bcd_add(destination as u8, source as u8, extend as u8)
    };
    if let (Some(address), Some(source_address)) = (destination_address, source_address) {
        bus.write8(address, result)?;
        cpu.write_an(destination_register, address);
        cpu.write_an(source_register, source_address);
    } else {
        cpu.write_dn(destination_register, u32::from(result), Size::Byte);
    }
    let zero = cpu.flag(FLAG_Z) && result == 0;
    cpu.set_ccr(carry, false, zero, result & 0x80 != 0, carry);
    Ok(0)
}

fn nbcd(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let destination = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Byte,
    )?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let extend = u32::from(cpu.flag(FLAG_X)) as u8;
    let mut carry = false;
    let (_, result) = modify_ea(cpu, bus, destination, Size::Byte, |value| {
        let (result, borrowed) = bcd_subtract(0, value as u8, extend);
        carry = borrowed;
        u32::from(result)
    })?;
    let zero = cpu.flag(FLAG_Z) && result == 0;
    cpu.set_ccr(carry, false, zero, result & 0x80 != 0, carry);
    Ok(0)
}

fn bcd_add(destination: u8, source: u8, extend: u8) -> (u8, bool) {
    let mut result = u16::from(destination) + u16::from(source) + u16::from(extend);
    if result & 0x000F > 9 {
        result += 6;
    }
    let mut carry = false;
    if result > 0x99 {
        result += 0x60;
        carry = true;
    }
    ((result & 0xFF) as u8, carry)
}

fn bcd_subtract(destination: u8, source: u8, extend: u8) -> (u8, bool) {
    let subtrahend = u16::from(source) + u16::from(extend);
    let mut result = u16::from(destination).wrapping_sub(subtrahend);
    if u16::from(destination) & 0x000F < subtrahend & 0x000F {
        result = result.wrapping_sub(6);
    }
    let carry = subtrahend > u16::from(destination);
    if carry {
        result = result.wrapping_sub(0x60);
    }
    ((result & 0xFF) as u8, carry)
}

fn multiply_or_divide(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
) -> Result<u16, CpuError> {
    let register = usize::from((opcode >> 9) & 0x7);
    let mode = ((opcode >> 3) & 0x7) as u8;
    let ea_register = (opcode & 0x7) as u8;
    let source = read_ea(
        cpu,
        bus,
        decode_ea(mode, ea_register, Size::Word)?,
        Size::Word,
    )?;
    let signed = opcode & 0x0100 != 0;
    let extend = cpu.flag(FLAG_X);

    if opcode & 0xF000 == 0xC000 {
        let left = u32::from(cpu.read_dn(register, Size::Word) as u16);
        let value = if signed {
            (left as u16 as i16 as i32).wrapping_mul(source as u16 as i16 as i32) as u32
        } else {
            left.wrapping_mul(u32::from(source as u16))
        };
        cpu.set_d(register, value);
        cpu.set_ccr(false, false, value == 0, value & 0x8000_0000 != 0, extend);
        return Ok(0);
    }

    let divisor = u32::from(source as u16);
    if divisor == 0 {
        return cpu.raise_exception(bus, VECTOR_ZERO_DIVIDE, ExceptionFrame::Short);
    }
    let dividend = cpu.read_dn(register, Size::Long);
    if signed {
        let dividend = dividend as i32;
        let divisor = dividend_divisor(source);
        let Some(quotient) = dividend.checked_div(divisor) else {
            return set_divide_overflow(cpu, extend);
        };
        if !(-32_768..=32_767).contains(&quotient) {
            return set_divide_overflow(cpu, extend);
        }
        let remainder = dividend % divisor;
        let value = ((remainder as u16 as u32) << 16) | (quotient as u16 as u32);
        cpu.set_d(register, value);
        cpu.set_ccr(false, false, quotient == 0, quotient & 0x8000 != 0, extend);
    } else {
        let quotient = dividend / divisor;
        if quotient > 0xFFFF {
            return set_divide_overflow(cpu, extend);
        }
        let remainder = dividend % divisor;
        let value = (remainder << 16) | quotient;
        cpu.set_d(register, value);
        cpu.set_ccr(false, false, quotient == 0, quotient & 0x8000 != 0, extend);
    }
    Ok(0)
}

const fn dividend_divisor(source: u32) -> i32 {
    (source as u16) as i16 as i32
}

/// Sets the documented divide-overflow flags without touching the destination.
///
/// `V` is set, `C` is cleared and the other condition codes are left as they
/// were; the destination register was never written.
fn set_divide_overflow(cpu: &mut M68000, extend: bool) -> Result<u16, CpuError> {
    let zero = cpu.flag(FLAG_Z);
    let negative = cpu.flag(FLAG_N);
    cpu.set_ccr(false, true, zero, negative, extend);
    Ok(0)
}

fn size_from_bits(bits: u16) -> Option<Size> {
    match bits {
        0 => Some(Size::Byte),
        1 => Some(Size::Word),
        2 => Some(Size::Long),
        _ => None,
    }
}

fn add_or_sub(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    add: bool,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let opmode = (opcode >> 6) & 0x7;
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    let data_register = usize::from((opcode >> 9) & 0x7);

    // ADDA/SUBA: opmode 011/111, destination An in bits 11-9, source EA in
    // bits 5-0. The operand is sign-extended to 32 bits and no flags change.
    if opmode == 3 || opmode == 7 {
        let size = if opmode == 3 { Size::Word } else { Size::Long };
        let source = read_ea(cpu, bus, decode_ea(mode, register, size)?, size)?;
        let value = size.sign_extend(source);
        let destination = cpu.read_an(data_register);
        let result = if add {
            destination.wrapping_add(value)
        } else {
            destination.wrapping_sub(value)
        };
        cpu.write_an(data_register, result);
        return Ok(0);
    }

    if let Some(size) = size_from_bits(opmode) {
        // ADD <ea>,Dn / SUB <ea>,Dn
        let source = read_ea(cpu, bus, decode_ea(mode, register, size)?, size)?;
        let destination = cpu.read_dn(data_register, size);
        let result = if add {
            destination.wrapping_add(source)
        } else {
            destination.wrapping_sub(source)
        };
        cpu.write_dn(data_register, result, size);
        set_arithmetic_flags(cpu, add, destination, source, result, size);
        return Ok(0);
    }

    if (4..=6).contains(&opmode) {
        // ADD Dn,<ea> / SUB Dn,<ea>
        let size =
            size_from_bits(opmode - 4).ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
        let destination = decode_ea(mode, register, size)?;
        if !is_data_alterable(destination) {
            return Err(illegal_opcode(instruction_pc, opcode));
        }
        let source = cpu.read_dn(data_register, size);
        let (destination_value, result) = modify_ea(cpu, bus, destination, size, |value| {
            if add {
                value.wrapping_add(source)
            } else {
                value.wrapping_sub(source)
            }
        })?;
        set_arithmetic_flags(cpu, add, destination_value, source, result, size);
        return Ok(0);
    }

    Err(illegal_opcode(instruction_pc, opcode))
}

fn immediate_arithmetic(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    add: bool,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_from_bits((opcode >> 6) & 0x3)
        .ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
    let immediate = read_ea(cpu, bus, Ea::Imm, size)?;
    let destination = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let (destination_value, result) = modify_ea(cpu, bus, destination, size, |value| {
        if add {
            value.wrapping_add(immediate)
        } else {
            value.wrapping_sub(immediate)
        }
    })?;
    set_arithmetic_flags(cpu, add, destination_value, immediate, result, size);
    Ok(0)
}

fn addq_or_subq(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let subtract = opcode & 0x0100 != 0;
    let size = size_from_bits((opcode >> 6) & 0x3)
        .ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
    let mut immediate = u32::from((opcode >> 9) & 0x7);
    if immediate == 0 {
        immediate = 8;
    }
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;

    if mode == 1 {
        // Address-register destination: full 32-bit operation, no flags.
        if size == Size::Byte {
            return Err(illegal_opcode(instruction_pc, opcode));
        }
        let destination = cpu.read_an(usize::from(register));
        let result = if subtract {
            destination.wrapping_sub(immediate)
        } else {
            destination.wrapping_add(immediate)
        };
        cpu.write_an(usize::from(register), result);
        return Ok(0);
    }

    let destination = decode_ea(mode, register, size)?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let (destination_value, result) = modify_ea(cpu, bus, destination, size, |value| {
        if subtract {
            value.wrapping_sub(immediate)
        } else {
            value.wrapping_add(immediate)
        }
    })?;
    set_arithmetic_flags(cpu, !subtract, destination_value, immediate, result, size);
    Ok(0)
}

fn compare(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let opmode = (opcode >> 6) & 0x7;
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;

    match opmode {
        0..=2 => {
            let size =
                size_from_bits(opmode).ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
            let source = read_ea(cpu, bus, decode_ea(mode, register, size)?, size)?;
            let destination = cpu.read_dn(usize::from((opcode >> 9) & 0x7), size);
            cpu.set_cmp_flags(destination, source, size);
            Ok(0)
        }
        3 | 7 => compare_address(cpu, bus, opcode),
        4..=6 => {
            if mode == 1 {
                compare_memory(cpu, bus, opcode)
            } else {
                Err(illegal_opcode(instruction_pc, opcode))
            }
        }
        _ => Err(illegal_opcode(instruction_pc, opcode)),
    }
}

fn compare_address(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let opmode = (opcode >> 6) & 0x7;
    let size = match opmode {
        3 => Size::Word,
        7 => Size::Long,
        _ => return Err(illegal_opcode(instruction_pc, opcode)),
    };
    let source = read_ea(
        cpu,
        bus,
        decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?,
        size,
    )?;
    let value = size.sign_extend(source);
    let destination = cpu.read_an(usize::from((opcode >> 9) & 0x7));
    cpu.set_cmp_flags(destination, value, Size::Long);
    Ok(0)
}

fn compare_memory(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let opmode = (opcode >> 6) & 0x7;
    let size = size_from_bits(opmode - 4).ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
    let source_register = (opcode & 0x7) as u8;
    let destination_register = ((opcode >> 9) & 0x7) as u8;
    let source = read_ea(cpu, bus, Ea::PostInc(source_register), size)?;
    let destination = read_ea(cpu, bus, Ea::PostInc(destination_register), size)?;
    cpu.set_cmp_flags(destination, source, size);
    Ok(0)
}

fn compare_immediate(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_from_bits((opcode >> 6) & 0x3)
        .ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
    let immediate = read_ea(cpu, bus, Ea::Imm, size)?;
    let destination = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let value = read_ea(cpu, bus, destination, size)?;
    cpu.set_cmp_flags(value, immediate, size);
    Ok(0)
}

fn negate(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    extended: bool,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_from_bits((opcode >> 6) & 0x3)
        .ok_or_else(|| illegal_opcode(instruction_pc, opcode))?;
    let destination = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let extend = if extended {
        u32::from(cpu.flag(FLAG_X))
    } else {
        0
    };
    let previous_zero = cpu.flag(FLAG_Z);
    let (value, result) = modify_ea(cpu, bus, destination, size, |value| {
        0_u32.wrapping_sub(value).wrapping_sub(extend)
    })?;
    let carry = value != 0 || extend != 0;
    let overflow = value & result & size.sign_bit() != 0;
    let zero = if extended {
        previous_zero && result == 0
    } else {
        result == 0
    };
    cpu.set_ccr(carry, overflow, zero, result & size.sign_bit() != 0, carry);
    Ok(0)
}

fn set_arithmetic_flags(
    cpu: &mut M68000,
    add: bool,
    destination: u32,
    source: u32,
    result: u32,
    size: Size,
) {
    if add {
        add_flags(cpu, destination, source, result, size);
    } else {
        sub_flags(cpu, destination, source, result, size);
    }
}
