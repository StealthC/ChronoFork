//! Data movement group: MOVE, MOVEA, MOVEQ, MOVEM, MOVEP, LEA, PEA, CLR, TST,
//! EXT and SWAP.
//!
//! Operand widths, flag behaviour and extension-word order follow the MC68000
//! programmer's reference. `MOVE` sets `N`/`Z` from the moved value and clears
//! `V`/`C`; `MOVEA`, `LEA`, `PEA`, `MOVEM` and `MOVEP` do not change flags.

use crate::bus::MegadriveBus;

use super::illegal_opcode;
use crate::cpu::ea::{
    Ea, Size, add_signed, decode_ea, ea_address, is_data_alterable, read_ea, write_ea,
};
use crate::cpu::{CpuError, M68000};

/// Executes one data movement instruction selected by the decode table.
///
/// Returns the instruction's 68000-cycle cost, which is zero until the timing
/// table (Task 14) lands.
pub fn execute(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    if (opcode & 0xF100) == 0x7000 {
        return moveq(cpu, opcode);
    }
    if matches!(opcode & 0xF000, 0x1000 | 0x2000 | 0x3000) {
        return move_form(cpu, bus, opcode);
    }
    if matches!(opcode & 0xFFF8, 0x4880 | 0x48C0) {
        return ext(cpu, opcode);
    }
    if (opcode & 0xFFF8) == 0x4840 {
        return swap(cpu, opcode);
    }
    if (opcode & 0xFB80) == 0x4880 {
        return movem(cpu, bus, opcode);
    }
    if (opcode & 0xFFC0) == 0x4840 {
        return pea(cpu, bus, opcode);
    }
    if (opcode & 0xF1C0) == 0x41C0 {
        return lea(cpu, bus, opcode);
    }
    if matches!(opcode & 0xF1F8, 0x0108 | 0x0148 | 0x0188 | 0x01C8) {
        return movep(cpu, bus, opcode);
    }
    let size_bits = (opcode >> 6) & 0x3;
    if size_bits != 0x3 && (opcode & 0xFF00) == 0x4200 {
        return clear(cpu, bus, opcode, size_of(opcode));
    }
    if size_bits != 0x3 && (opcode & 0xFF00) == 0x4A00 {
        return test(cpu, bus, opcode, size_of(opcode));
    }
    Err(illegal_opcode(instruction_pc, opcode))
}

/// Decodes the two-bit size field. The caller has already checked its class.
fn size_of(opcode: u16) -> Size {
    match (opcode >> 6) & 0x3 {
        0 => Size::Byte,
        1 => Size::Word,
        2 => Size::Long,
        _ => Size::Long,
    }
}

fn moveq(cpu: &mut M68000, opcode: u16) -> Result<u16, CpuError> {
    let register = usize::from((opcode >> 9) & 0x7);
    let value = (opcode & 0xFF) as u8 as i8 as i32 as u32;
    cpu.set_d(register, value);
    cpu.set_logic_flags(value, Size::Long);
    Ok(0)
}

fn move_form(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    // MOVE carries its size in the top nibble: 1 byte, 2 long, 3 word.
    let size = match opcode >> 12 {
        1 => Size::Byte,
        2 => Size::Long,
        3 => Size::Word,
        _ => return Err(illegal_opcode(instruction_pc, opcode)),
    };
    let source = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    let destination_mode = ((opcode >> 6) & 0x7) as u8;
    let destination_register = ((opcode >> 9) & 0x7) as u8;

    if destination_mode == 1 {
        // MOVEA: byte size does not exist and is an illegal encoding.
        if size == Size::Byte {
            return Err(illegal_opcode(instruction_pc, opcode));
        }
        let value = read_ea(cpu, bus, source, size)?;
        cpu.write_an(usize::from(destination_register), size.sign_extend(value));
        return Ok(0);
    }
    if destination_mode == 7 && destination_register > 1 {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let destination = decode_ea(destination_mode, destination_register, size)?;
    let value = read_ea(cpu, bus, source, size)?;
    write_ea(cpu, bus, destination, size, value)?;
    cpu.set_logic_flags(value, size);
    Ok(0)
}

fn ext(cpu: &mut M68000, opcode: u16) -> Result<u16, CpuError> {
    let register = usize::from(opcode & 0x7);
    if (opcode & 0xFFF8) == 0x4880 {
        let low = cpu.read_dn(register, Size::Byte);
        let value = (low as u8 as i8) as i32 as u32;
        cpu.write_dn(register, value, Size::Word);
        cpu.set_logic_flags(value, Size::Word);
    } else {
        let low = cpu.read_dn(register, Size::Word);
        let value = (low as u16 as i16) as i32 as u32;
        cpu.set_d(register, value);
        cpu.set_logic_flags(value, Size::Long);
    }
    Ok(0)
}

fn swap(cpu: &mut M68000, opcode: u16) -> Result<u16, CpuError> {
    let register = usize::from(opcode & 0x7);
    let value = cpu.read_dn(register, Size::Long);
    let swapped = value.rotate_right(16);
    cpu.set_d(register, swapped);
    cpu.set_logic_flags(swapped, Size::Long);
    Ok(0)
}

fn clear(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    size: Size,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let ea = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_data_alterable(ea) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    write_ea(cpu, bus, ea, size, 0)?;
    cpu.set_logic_flags(0, size);
    Ok(0)
}

fn test(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    opcode: u16,
    size: Size,
) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let ea = decode_ea(((opcode >> 3) & 0x7) as u8, (opcode & 0x7) as u8, size)?;
    if !is_data_alterable(ea) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    let value = read_ea(cpu, bus, ea, size)?;
    cpu.set_logic_flags(value, size);
    Ok(0)
}

fn lea(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let destination = usize::from((opcode >> 9) & 0x7);
    let ea = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Long,
    )?;
    let address = ea_address(cpu, bus, ea)?;
    cpu.write_an(destination, address);
    Ok(0)
}

fn pea(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let ea = decode_ea(
        ((opcode >> 3) & 0x7) as u8,
        (opcode & 0x7) as u8,
        Size::Long,
    )?;
    let address = ea_address(cpu, bus, ea)?;
    cpu.push_long(bus, address)?;
    Ok(0)
}

fn movep(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let data = usize::from((opcode >> 9) & 0x7);
    let address_register = usize::from(opcode & 0x7);
    let operation = (opcode >> 6) & 0x7;
    let long = operation & 1 != 0;
    let size = if long { Size::Long } else { Size::Word };
    let memory_to_register = operation < 6;
    let displacement = cpu.fetch_u16(bus)? as i16;
    let base = cpu.read_an(address_register);
    let address = add_signed(base, i32::from(displacement))?;

    if memory_to_register {
        let mut value = 0_u32;
        for byte_index in 0..size.bytes() {
            let offset = byte_index.checked_mul(2).ok_or(CpuError::AddressOverflow)?;
            let address = address
                .checked_add(offset)
                .ok_or(CpuError::AddressOverflow)?;
            value = (value << 8) | u32::from(bus.read8(address)?);
        }
        cpu.write_dn(data, value, size);
    } else {
        let value = cpu.read_dn(data, size);
        let bytes = value.to_be_bytes();
        let source = if long { &bytes[..] } else { &bytes[2..] };
        for (byte_index, byte) in source.iter().enumerate() {
            let offset = (byte_index as u32)
                .checked_mul(2)
                .ok_or(CpuError::AddressOverflow)?;
            let address = address
                .checked_add(offset)
                .ok_or(CpuError::AddressOverflow)?;
            bus.write8(address, *byte)?;
        }
    }
    Ok(0)
}

fn movem(cpu: &mut M68000, bus: &mut MegadriveBus, opcode: u16) -> Result<u16, CpuError> {
    let instruction_pc = cpu.pc().saturating_sub(2);
    let size = if opcode & 0x0040 != 0 {
        Size::Long
    } else {
        Size::Word
    };
    let memory_to_registers = opcode & 0x0400 != 0;
    let mode = ((opcode >> 3) & 0x7) as u8;
    let register = (opcode & 0x7) as u8;
    let mask = cpu.fetch_u16(bus)?;
    let ea = decode_ea(mode, register, size)?;
    let predecrement = matches!(ea, Ea::PreDec(_));
    if memory_to_registers && predecrement {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    if !memory_to_registers && matches!(ea, Ea::PostInc(_)) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    if memory_to_registers && !is_movem_source(ea) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }
    if !memory_to_registers && !is_movem_destination(ea) {
        return Err(illegal_opcode(instruction_pc, opcode));
    }

    let registers = movem_registers(mask, predecrement);
    let mut address = match ea {
        Ea::PostInc(register) | Ea::PreDec(register) => cpu.read_an(usize::from(register)),
        _ => ea_address(cpu, bus, ea)?,
    };
    let width = size.bytes();
    let long = size == Size::Long;

    if memory_to_registers {
        for (is_address, index) in &registers {
            let value = if long {
                bus.read32(address)?
            } else {
                Size::Word.sign_extend(u32::from(bus.read16(address)?))
            };
            if *is_address {
                cpu.write_an(usize::from(*index), value);
            } else {
                cpu.write_dn(usize::from(*index), value, Size::Long);
            }
            address = address
                .checked_add(width)
                .ok_or(CpuError::AddressOverflow)?;
        }
        if let Ea::PostInc(register) = ea {
            cpu.write_an(usize::from(register), address);
        }
    } else {
        for (is_address, index) in &registers {
            let value = if *is_address {
                cpu.read_an(usize::from(*index))
            } else {
                cpu.read_dn(usize::from(*index), size)
            };
            if predecrement {
                address = address.checked_sub(width).ok_or(CpuError::StackOverflow)?;
            }
            if long {
                bus.write32(address, value)?;
            } else {
                bus.write16(address, value as u16)?;
            }
            if !predecrement {
                address = address
                    .checked_add(width)
                    .ok_or(CpuError::AddressOverflow)?;
            }
        }
        if let Ea::PreDec(register) = ea {
            cpu.write_an(usize::from(register), address);
        }
    }
    Ok(0)
}

/// Returns the register list in transfer order for `MOVEM`.
///
/// Normal masks put `D0` in bit 0 and `A7` in bit 15, so ascending bits are also
/// ascending memory addresses. The pre-decrement form reverses that mapping:
/// `A7` is bit 0 and the transfer order is `A7..D0`, which is why assemblers
/// emit a reversed mask for `-(An)`.
fn movem_registers(mask: u16, predecrement: bool) -> Vec<(bool, u8)> {
    let mut registers = Vec::new();
    for bit in 0..16_u8 {
        if mask & (1_u16 << bit) == 0 {
            continue;
        }
        let index = if predecrement { 15 - bit } else { bit };
        if index < 8 {
            registers.push((false, index));
        } else {
            registers.push((true, index - 8));
        }
    }
    registers
}

fn is_movem_source(ea: Ea) -> bool {
    matches!(
        ea,
        Ea::Indirect(_)
            | Ea::PostInc(_)
            | Ea::Disp16(_)
            | Ea::Index(_)
            | Ea::Abs16
            | Ea::Abs32
            | Ea::PcDisp16
            | Ea::PcIndex
    )
}

fn is_movem_destination(ea: Ea) -> bool {
    matches!(
        ea,
        Ea::Indirect(_) | Ea::PreDec(_) | Ea::Disp16(_) | Ea::Index(_) | Ea::Abs16 | Ea::Abs32
    )
}
