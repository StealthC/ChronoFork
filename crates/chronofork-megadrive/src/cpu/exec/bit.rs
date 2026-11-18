//! Bit-operation group: BTST, BCHG, BCLR and BSET.
//!
//! These are the only instructions in the group and, apart from the operand
//! update, change exactly one flag: `Z` reflects the bit before the operation.
//! Data-register bit numbers are taken modulo 32; memory bit numbers modulo 8.

use crate::bus::MegadriveBus;

use super::illegal_opcode;
use crate::cpu::ea::{Ea, Size, decode_ea, is_data_alterable, modify_ea, read_ea};
use crate::cpu::{CpuError, FLAG_Z, M68000};

const BTST: u16 = 0;
const BCHG: u16 = 1;
const BCLR: u16 = 2;
const BSET: u16 = 3;

/// Executes one bit operation selected by the decode table.
///
/// Returns the instruction's 68000-cycle cost, which is zero until the timing
/// table (Task 14) lands.
pub fn execute(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    if opcode & 0xF000 != 0 {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let static_form = (opcode & 0x0F00) == 0x0800;
    if !static_form && opcode & 0x0100 == 0 {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let operation = (opcode >> 6) & 0x3;
    let bit_number = if static_form {
        u32::from(cpu.fetch_u16(bus)? & 0x00FF)
    } else {
        let source = usize::from((opcode >> 9) & 0x7);
        cpu.read_dn(source, Size::Long)
    };
    let ea = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Byte,
    )?;
    if let Ea::Dn(register) = ea {
        let bit = bit_number & 31;
        let value = cpu.read_dn(usize::from(register), Size::Long);
        let set = value >> bit & 1 != 0;
        cpu.set_condition(FLAG_Z, !set);
        if operation != BTST {
            let mask = 1_u32 << bit;
            let updated = match operation {
                BCHG => value ^ mask,
                BCLR => value & !mask,
                BSET => value | mask,
                _ => value,
            };
            cpu.write_dn(usize::from(register), updated, Size::Long);
        }
        return Ok(0);
    }
    if !is_data_alterable(ea) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let bit = bit_number & 7;
    if operation == BTST {
        let value = read_ea(cpu, bus, ea, Size::Byte)?;
        cpu.set_condition(FLAG_Z, value >> bit & 1 == 0);
        return Ok(0);
    }
    let (previous, _) = modify_ea(cpu, bus, ea, Size::Byte, |value| {
        let mask = 1_u8 << bit;
        match operation {
            BCHG => value ^ u32::from(mask),
            BCLR => value & !u32::from(mask),
            BSET => value | u32::from(mask),
            _ => value,
        }
    })?;
    cpu.set_condition(FLAG_Z, previous >> bit & 1 == 0);
    Ok(0)
}
