//! Control-flow group: BRA, BSR, Bcc, DBcc, Scc, JMP, JSR and RTS.
//!
//! All conditional instructions share [`cc_to_bool`], the one 68000 condition
//! predicate. `BSR`/`JSR` push the return address and take the address-error
//! exception on an odd target; `DBcc` only decrements its data register when
//! the condition is false; `Scc` writes a byte `0xFF` or `0x00`.

use crate::bus::MegadriveBus;

use super::illegal_opcode;
use crate::cpu::ea::{Size, decode_ea, ea_address, is_data_alterable, write_ea};
use crate::cpu::{CpuError, ExceptionFrame, M68000, VECTOR_ADDRESS_ERROR};

/// Evaluates the 68000 condition-code predicate `cc`.
///
/// This is the single condition implementation for branches, `DBcc` and `Scc`.
#[must_use]
pub fn cc_to_bool(cpu: &M68000, cc: u8) -> bool {
    match cc & 0xF {
        0x0 => true,
        0x1 => false,
        0x2 => !cpu.flag(crate::cpu::FLAG_C) && !cpu.flag(crate::cpu::FLAG_Z),
        0x3 => cpu.flag(crate::cpu::FLAG_C) || cpu.flag(crate::cpu::FLAG_Z),
        0x4 => !cpu.flag(crate::cpu::FLAG_C),
        0x5 => cpu.flag(crate::cpu::FLAG_C),
        0x6 => !cpu.flag(crate::cpu::FLAG_Z),
        0x7 => cpu.flag(crate::cpu::FLAG_Z),
        0x8 => !cpu.flag(crate::cpu::FLAG_V),
        0x9 => cpu.flag(crate::cpu::FLAG_V),
        0xA => !cpu.flag(crate::cpu::FLAG_N),
        0xB => cpu.flag(crate::cpu::FLAG_N),
        0xC => cpu.flag(crate::cpu::FLAG_N) == cpu.flag(crate::cpu::FLAG_V),
        0xD => cpu.flag(crate::cpu::FLAG_N) != cpu.flag(crate::cpu::FLAG_V),
        0xE => {
            !cpu.flag(crate::cpu::FLAG_Z)
                && (cpu.flag(crate::cpu::FLAG_N) == cpu.flag(crate::cpu::FLAG_V))
        }
        _ => {
            cpu.flag(crate::cpu::FLAG_Z)
                || (cpu.flag(crate::cpu::FLAG_N) != cpu.flag(crate::cpu::FLAG_V))
        }
    }
}

/// Executes one control-flow instruction selected by the decode table.
///
/// Returns the instruction's 68000-cycle cost, which is zero until the timing
/// table (Task 14) lands.
pub fn execute(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    if (opcode & 0xF000) == 0x6000 {
        return branch(cpu, bus, opcode);
    }
    if (opcode & 0xF000) == 0x5000 && (opcode & 0x00C0) == 0x00C0 {
        let condition = ((opcode >> 8) & 0xF) as u8;
        if (opcode & 0x0038) == 0x0008 {
            return dbcc(cpu, bus, opcode, condition);
        }
        return scc(cpu, bus, opcode, condition);
    }
    if opcode == 0x4E75 {
        let pc = cpu.pop_long(bus)?;
        cpu.write_pc(pc)?;
        return Ok(0);
    }
    if (opcode & 0xFFC0) == 0x4EC0 {
        return jump(cpu, bus, opcode, false);
    }
    if (opcode & 0xFFC0) == 0x4E80 {
        return jump(cpu, bus, opcode, true);
    }
    let instruction_pc = cpu.pc().saturating_sub(2);
    Err(illegal_opcode(instruction_pc, opcode))
}

fn branch(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let condition = ((opcode >> 8) & 0xF) as u8;
    let displacement = i32::from((opcode & 0xFF) as u8 as i8);
    let taken = condition == 1 || cc_to_bool(cpu, condition);
    if !taken {
        return Ok(0);
    }
    let target = cpu.relative_target(displacement)?;
    if condition == 1 {
        let return_pc = cpu.pc();
        cpu.push_long(bus, return_pc)?;
    }
    cpu.write_pc(target)?;
    Ok(0)
}

fn dbcc(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    condition: u8,
) -> Result<u16, CpuError> {
    let displacement = cpu.fetch_u16(bus)? as i16;
    if cc_to_bool(cpu, condition) {
        return Ok(0);
    }
    let register = usize::from(opcode & 0x7);
    let counter = cpu.read_dn(register, Size::Word);
    let decremented = counter.wrapping_sub(1) & Size::Word.mask();
    cpu.write_dn(register, decremented, Size::Word);
    if decremented == 0xFFFF {
        return Ok(0);
    }
    let target = cpu.relative_target(i32::from(displacement))?;
    cpu.write_pc(target)?;
    Ok(0)
}

fn scc(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    condition: u8,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let destination = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Byte,
    )?;
    if !is_data_alterable(destination) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let value = if cc_to_bool(cpu, condition) { 0xFF } else { 0 };
    write_ea(cpu, bus, destination, Size::Byte, value)?;
    Ok(0)
}

fn jump(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    subroutine: bool,
) -> Result<u16, CpuError> {
    let ea = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Long,
    )?;
    let target = ea_address(cpu, bus, ea)?;
    if target & 1 != 0 {
        return cpu.raise_exception(
            bus,
            VECTOR_ADDRESS_ERROR,
            ExceptionFrame::AddressError {
                address: target,
                opcode,
            },
        );
    }
    if subroutine {
        let return_pc = cpu.pc();
        cpu.push_long(bus, return_pc)?;
    }
    cpu.write_pc(target)?;
    Ok(0)
}
