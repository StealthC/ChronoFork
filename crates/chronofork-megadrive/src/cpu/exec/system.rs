//! System instructions and privilege: SR/CCR/USP moves, ORI/ANDI/EORI to
//! CCR/SR, STOP, RESET, RTR, TRAP, TRAPV, CHK, ILLEGAL, LINK and UNLK.
//!
//! Writes to `SR`, `USP`, `STOP`, `RESET` and `RTE` are privileged; in user
//! mode they raise the privilege-violation exception (vector 8) without
//! performing the instruction's own effect.

use crate::bus::MegadriveBus;

use crate::cpu::timing;

use super::illegal_opcode;
use crate::cpu::ea::{Ea, Size, decode_ea, is_data_alterable, read_ea, write_ea};
use crate::cpu::{
    CpuError, ExceptionFrame, FLAG_V, M68000, SR_SUPERVISOR, VECTOR_CHK, VECTOR_ILLEGAL,
    VECTOR_PRIVILEGE_VIOLATION, VECTOR_TRAP_BASE, VECTOR_TRAPV,
};

const TRAPV: u16 = 0x4E76;

/// Executes one system instruction selected by the decode table.
///
/// Returns the instruction's 68000-cycle cost, which is zero until the timing
/// table (Task 14) lands.
pub fn execute(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    match opcode {
        0x4E70 => return reset(cpu, bus, opcode),
        0x4E72 => return stop(cpu, bus, opcode),
        TRAPV => return trapv(cpu, bus, opcode),
        0x4E77 => return rtr(cpu, bus, opcode),
        0x4AFC => {
            return cpu.raise_exception(bus, VECTOR_ILLEGAL, ExceptionFrame::Short);
        }
        _ => {}
    }
    if (opcode & 0xFFF8) == 0x4E50 {
        return link(cpu, bus, opcode);
    }
    if (opcode & 0xFFF8) == 0x4E58 {
        return unlk(cpu, bus, opcode);
    }
    if (opcode & 0xFFF8) == 0x4E60 {
        return move_to_usp(cpu, bus, opcode);
    }
    if (opcode & 0xFFF8) == 0x4E68 {
        return move_from_usp(cpu, bus, opcode);
    }
    if (opcode & 0xFFF0) == 0x4E40 {
        let vector = VECTOR_TRAP_BASE + (opcode & 0xF) as u8;
        return cpu.raise_exception(bus, vector, ExceptionFrame::Short);
    }
    if (opcode & 0xF1C0) == 0x4180 {
        return chk(cpu, bus, opcode);
    }
    match opcode {
        0x003C | 0x007C | 0x023C | 0x027C | 0x0A3C | 0x0A7C => {
            return logic_to_special(cpu, bus, opcode);
        }
        _ => {}
    }
    if matches!(opcode & 0xFFC0, 0x40C0 | 0x42C0 | 0x44C0 | 0x46C0) {
        return move_status(cpu, bus, opcode);
    }
    let instruction_pc = cpu.pc().saturating_sub(2);
    Err(illegal_opcode(instruction_pc, opcode))
}

fn supervisor(cpu: &mut M68000, bus: &mut MegadriveBus) -> Result<Option<u16>, CpuError> {
    if cpu.sr() & SR_SUPERVISOR != 0 {
        return Ok(None);
    }
    cpu.raise_exception(bus, VECTOR_PRIVILEGE_VIOLATION, ExceptionFrame::Short)
        .map(Some)
}

fn reset(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    if let Some(cycles) = supervisor(cpu, bus)? {
        return Ok(cycles);
    }
    // The slice has no reset-able external device; RESET is architecturally a
    // privileged no-op that only consumes its opcode word.
    Ok(timing::instruction_cycles(opcode))
}

fn stop(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    if let Some(cycles) = supervisor(cpu, bus)? {
        return Ok(cycles);
    }
    let value = cpu.fetch_u16(bus)?;
    cpu.set_sr(value);
    cpu.stop();
    Ok(timing::instruction_cycles(opcode))
}

fn trapv(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    if cpu.flag(FLAG_V) {
        return cpu.raise_exception(bus, VECTOR_TRAPV, ExceptionFrame::Short);
    }
    Ok(timing::instruction_cycles(opcode))
}

fn rtr(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let frame = cpu.a7();
    let pc_address = frame.checked_add(2).ok_or(CpuError::StackOverflow)?;
    let restored_a7 = frame.checked_add(6).ok_or(CpuError::StackOverflow)?;
    let ccr = bus.read16(frame)?;
    let pc = bus.read32(pc_address)?;
    cpu.set_sr((cpu.sr() & 0xFF00) | (ccr & 0x00FF));
    cpu.write_pc(pc)?;
    cpu.set_a7(restored_a7);
    Ok(timing::instruction_cycles(opcode))
}

fn link(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let register = usize::from(opcode & 0x7);
    let displacement = cpu.fetch_u16(bus)? as i16;
    let previous = cpu.read_an(register);
    cpu.push_long(bus, previous)?;
    cpu.write_an(register, cpu.a7());
    let stack = i64::from(cpu.a7()) + i64::from(displacement);
    if stack < 0 || stack > i64::from(u32::MAX) {
        return Err(CpuError::AddressOverflow);
    }
    // Proven by the range check above.
    cpu.set_a7(stack as u32);
    Ok(timing::instruction_cycles(opcode))
}

fn unlk(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let register = usize::from(opcode & 0x7);
    cpu.set_a7(cpu.read_an(register));
    let value = cpu.pop_long(bus)?;
    cpu.write_an(register, value);
    Ok(timing::instruction_cycles(opcode))
}

fn move_to_usp(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    if let Some(cycles) = supervisor(cpu, bus)? {
        return Ok(cycles);
    }
    let register = usize::from(opcode & 0x7);
    cpu.set_usp(cpu.read_an(register));
    Ok(timing::instruction_cycles(opcode))
}

fn move_from_usp(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    if let Some(cycles) = supervisor(cpu, bus)? {
        return Ok(cycles);
    }
    let register = usize::from(opcode & 0x7);
    let value = cpu.usp();
    cpu.write_an(register, value);
    Ok(timing::instruction_cycles(opcode))
}

fn move_status(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let ea = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Word,
    )?;
    match opcode & 0xFFC0 {
        0x40C0 => {
            // MOVE from SR
            if !is_data_alterable(ea) {
                return Err(illegal_opcode(instruction_pc, opcode));
            }
            write_ea(cpu, bus, ea, Size::Word, u32::from(cpu.sr()))?;
        }
        0x42C0 => {
            // MOVE from CCR
            if !is_data_alterable(ea) {
                return Err(illegal_opcode(instruction_pc, opcode));
            }
            write_ea(cpu, bus, ea, Size::Word, u32::from(cpu.sr() & 0x00FF))?;
        }
        0x44C0 => {
            // MOVE to CCR
            if !is_status_source(ea) {
                return Err(illegal_opcode(instruction_pc, opcode));
            }
            let value = read_ea(cpu, bus, ea, Size::Word)? as u16;
            let sr = (cpu.sr() & 0xFF00) | (value & 0x00FF);
            cpu.set_sr(sr);
        }
        0x46C0 => {
            // MOVE to SR (privileged)
            if let Some(cycles) = supervisor(cpu, bus)? {
                return Ok(cycles);
            }
            if !is_status_source(ea) {
                return Err(illegal_opcode(instruction_pc, opcode));
            }
            let value = read_ea(cpu, bus, ea, Size::Word)? as u16;
            cpu.set_sr(value);
        }
        _ => return Err(illegal_opcode(instruction_pc, opcode)),
    }
    Ok(timing::instruction_cycles(opcode))
}

fn logic_to_special(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
) -> Result<u16, CpuError> {
    let to_sr = matches!(opcode, 0x007C | 0x027C | 0x0A7C);
    if to_sr {
        if let Some(cycles) = supervisor(cpu, bus)? {
            return Ok(cycles);
        }
    }
    let immediate_word = cpu.fetch_u16(bus)?;
    // The CCR forms carry a byte immediate in the low half of the extension
    // word; the SR forms use the full 16-bit immediate.
    let (source, immediate) = if to_sr {
        (cpu.sr(), immediate_word)
    } else {
        (cpu.sr() & 0x00FF, immediate_word & 0x00FF)
    };
    let result = match opcode & 0xFF00 {
        0x0000 => source | immediate,
        0x0200 => source & immediate,
        _ => source ^ immediate,
    };
    if to_sr {
        cpu.set_sr(result);
    } else {
        cpu.set_sr((cpu.sr() & 0xFF00) | (result & 0x00FF));
    }
    Ok(timing::instruction_cycles(opcode))
}

fn chk(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let register = usize::from((opcode >> 9) & 0x7);
    let ea = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Word,
    )?;
    if !is_status_source(ea) {
        let instruction_pc = cpu.pc().saturating_sub(2);
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let bound = read_ea(cpu, bus, ea, Size::Word)? as u16 as i16 as i32;
    let value = cpu.read_dn(register, Size::Word) as u16 as i16 as i32;
    if value < 0 || value > bound {
        return cpu.raise_exception(bus, VECTOR_CHK, ExceptionFrame::Short);
    }
    Ok(timing::instruction_cycles(opcode))
}

fn is_status_source(ea: Ea) -> bool {
    matches!(
        ea,
        Ea::Dn(_)
            | Ea::Indirect(_)
            | Ea::PostInc(_)
            | Ea::PreDec(_)
            | Ea::Disp16(_)
            | Ea::Index(_)
            | Ea::Abs16
            | Ea::Abs32
            | Ea::Imm
    )
}
