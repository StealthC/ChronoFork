//! Logic group: AND/ANDI, OR/ORI, EOR/EORI and NOT.
//!
//! Every logic operation sets `N`/`Z` from the result, clears `V`/`C` and
//! leaves `X` unchanged. The shared [`crate::cpu::exec::set_logic_flags`]
//! helper is the single implementation of that rule.

use crate::bus::MegadriveBus;

use crate::cpu::timing;

use super::{illegal_opcode, set_logic_flags};
use crate::cpu::ea::{Size, decode_ea, is_data_alterable, modify_ea, read_ea};
use crate::cpu::{CpuError, M68000};

#[derive(Clone, Copy)]
enum Operation {
    And,
    Or,
    Xor,
}

impl Operation {
    fn apply(self, left: u32, right: u32) -> u32 {
        match self {
            Self::And => left & right,
            Self::Or => left | right,
            Self::Xor => left ^ right,
        }
    }
}

/// Executes one logic instruction selected by the decode table.
///
/// Returns the instruction's 68000-cycle cost, which is zero until the timing
/// table (Task 14) lands.
pub fn execute(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    match opcode & 0xF000 {
        0xC000 => return binary(cpu, bus, opcode, Operation::And),
        0x8000 => return binary(cpu, bus, opcode, Operation::Or),
        0xB000 => return eor(cpu, bus, opcode),
        _ => {}
    }
    let size_bits = (opcode >> 6) & 0x3;
    if size_bits != 0x3 {
        match opcode & 0xFF00 {
            0x0200 => return immediate(cpu, bus, opcode, Operation::And),
            0x0000 => return immediate(cpu, bus, opcode, Operation::Or),
            0x0A00 => return immediate(cpu, bus, opcode, Operation::Xor),
            0x4600 => return not(cpu, bus, opcode),
            _ => {}
        }
    }
    let instruction_pc = cpu.pc().saturating_sub(2);
    Err(illegal_opcode(instruction_pc, opcode))
}

fn binary(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    operation: Operation,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_of(opcode, instruction_pc)?;
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;

    if opcode & 0x0100 == 0 {
        // <ea>,Dn
        let source = read_ea(cpu, bus, decode_ea(mode, register, size)?, size)?;
        let data_register = usize::from((opcode >> 9) & 0x7);
        let destination = cpu.read_dn(data_register, size);
        let result = operation.apply(destination, source) & size.mask();
        cpu.write_dn(data_register, result, size);
        set_logic_flags(cpu, result, size);
    } else {
        // Dn,<ea>
        let destination = decode_ea(mode, register, size)?;
        if !is_data_alterable(destination) {
            return Err(illegal_opcode(instruction_pc, opcode));
        }
        let source = cpu.read_dn(usize::from((opcode >> 9) & 0x7), size);
        let (_, result) = modify_ea(cpu, bus, destination, size, |value| {
            operation.apply(value, source)
        })?;
        set_logic_flags(cpu, result, size);
    }
    Ok(timing::instruction_cycles(opcode))
}

fn eor(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_of(opcode, instruction_pc)?;
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    // Mode 001 carries CMPM, which the arithmetic group owns.
    if mode == 1 || opcode & 0x0100 == 0 {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let destination = decode_ea(mode, register, size)?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let source = cpu.read_dn(usize::from((opcode >> 9) & 0x7), size);
    let (_, result) = modify_ea(cpu, bus, destination, size, |value| value ^ source)?;
    set_logic_flags(cpu, result, size);
    Ok(timing::instruction_cycles(opcode))
}

fn immediate(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    operation: Operation,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_of(opcode, instruction_pc)?;
    let immediate = read_ea(cpu, bus, crate::cpu::ea::Ea::Imm, size)?;
    let destination = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let (_, result) = modify_ea(cpu, bus, destination, size, |value| {
        operation.apply(value, immediate)
    })?;
    set_logic_flags(cpu, result, size);
    Ok(timing::instruction_cycles(opcode))
}

fn not(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = size_of(opcode, instruction_pc)?;
    let destination = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let (_, result) = modify_ea(cpu, bus, destination, size, |value| !value)?;
    set_logic_flags(cpu, result, size);
    Ok(timing::instruction_cycles(opcode))
}

fn size_of(opcode: u16, instruction_pc: u32) -> Result<Size, CpuError> {
    match (opcode >> 6) & 0x3 {
        0 => Ok(Size::Byte),
        1 => Ok(Size::Word),
        2 => Ok(Size::Long),
        _ => Err(illegal_opcode(instruction_pc, opcode)),
    }
}
