//! `CB`-prefixed Z80 instructions: rotates, shifts and bit operations.
//!
//! The table is total over the prefix byte: bits 7–6 select the group
//! (`rotate/shift`, `BIT`, `RES`, `SET`), bits 5–3 select the bit number and
//! bits 2–0 select the register (`6` is `(HL)`).
//!
//! **Undocumented `DD`/`FD CB d` form.** Real hardware operates on `(IX+d)` or
//! `(IY+d)` and, when the register field is not `(HL)`, also copies the result
//! into that register. Both behaviours are implemented; `BIT` only sets flags.
//! Timings follow the documented totals: 8 T-states for register forms, 15 for
//! `(HL)` shifts and `SET`/`RES`, 12 for `BIT (HL)`, 23 for the indexed
//! shift/`SET`/`RES` forms and 20 for indexed `BIT`.

use super::{
    FLAG_C, FLAG_H, FLAG_N, FLAG_PV, FLAG_S, FLAG_X, FLAG_Y, FLAG_Z, Target, Z80, Z80Bus, Z80Error,
};

/// Executes one `CB`-prefixed operation and returns its T-state count.
///
/// # Errors
///
/// Propagates a [`Z80Bus`] failure from a `(HL)` read or write.
pub fn execute(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    execute_with(z80, bus, opcode, Target::hl())
}

/// Executes one `DD CB d`/`FD CB d` operation against `(IX+d)`/`(IY+d)`.
pub(crate) fn execute_indexed(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    execute_with(z80, bus, opcode, target)
}

fn execute_with(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let group = opcode >> 6;
    let bit = (opcode >> 3) & 7;
    let register = opcode & 7;
    let memory = register == 6 || target.is_prefixed();
    let address = target.address(z80);
    match group {
        0 => {
            let value = read_operand(z80, bus, memory, register, address)?;
            let (result, carry) = rotate(value, bit, z80.is_flag(FLAG_C));
            z80.set_szxy(result);
            z80.set_flag(FLAG_H, false);
            z80.set_flag(FLAG_N, false);
            z80.set_flag(FLAG_PV, result.count_ones() % 2 == 0);
            z80.set_flag(FLAG_C, carry);
            write_operand(z80, bus, memory, register, address, result)?;
            Ok(cost(memory, target, false))
        }
        1 => {
            let value = read_operand(z80, bus, memory, register, address)?;
            let set = value & (1 << bit) != 0;
            z80.set_flag(FLAG_Z, !set);
            z80.set_flag(FLAG_PV, !set);
            z80.set_flag(FLAG_H, true);
            z80.set_flag(FLAG_N, false);
            z80.set_flag(FLAG_S, bit == 7 && set);
            z80.set_flag(FLAG_Y, value & 0x20 != 0);
            z80.set_flag(FLAG_X, value & 0x08 != 0);
            Ok(cost(memory, target, true))
        }
        _ => {
            let value = read_operand(z80, bus, memory, register, address)?;
            let mask = 1_u8 << bit;
            let result = if group == 2 {
                value & !mask
            } else {
                value | mask
            };
            write_operand(z80, bus, memory, register, address, result)?;
            Ok(cost(memory, target, false))
        }
    }
}

fn read_operand(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    memory: bool,
    register: u8,
    address: u16,
) -> Result<u8, Z80Error> {
    if memory {
        bus.read8(address)
    } else {
        Ok(z80.reg8(register))
    }
}

fn write_operand(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    memory: bool,
    register: u8,
    address: u16,
    value: u8,
) -> Result<(), Z80Error> {
    if memory {
        bus.write8(address, value)?;
        if register != 6 {
            z80.set_reg8(register, value);
        }
    } else {
        z80.set_reg8(register, value);
    }
    Ok(())
}

/// Documented `CB` T-state totals.
const fn cost(memory: bool, target: Target, bit: bool) -> u32 {
    if target.is_prefixed() {
        if bit { 20 } else { 23 }
    } else if memory {
        if bit { 12 } else { 15 }
    } else {
        8
    }
}

/// Applies `RLC`, `RRC`, `RL`, `RR`, `SLA`, `SRA`, `SLL` or `SRL`.
///
/// Returns the result and the bit shifted out into the carry flag.
const fn rotate(value: u8, operation: u8, carry_in: bool) -> (u8, bool) {
    match operation {
        0 => (value.rotate_left(1), value & 0x80 != 0),
        1 => (value.rotate_right(1), value & 0x01 != 0),
        2 => (
            (value << 1) | if carry_in { 1 } else { 0 },
            value & 0x80 != 0,
        ),
        3 => (
            (value >> 1) | if carry_in { 0x80 } else { 0 },
            value & 0x01 != 0,
        ),
        4 => (value << 1, value & 0x80 != 0),
        5 => ((value >> 1) | (value & 0x80), value & 0x01 != 0),
        6 => ((value << 1) | 0x01, value & 0x80 != 0),
        _ => (value >> 1, value & 0x01 != 0),
    }
}
