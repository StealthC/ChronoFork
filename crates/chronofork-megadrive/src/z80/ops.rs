//! Main (unprefixed) Z80 opcode table.
//!
//! Every opcode byte is decoded, so [`execute`] is total over `u8`; the
//! `CB`/`ED`/`DD`/`FD` prefix bytes are dispatched by [`crate::z80::Z80::step`]
//! and are treated as a one-byte `NOP` if this table is called directly.
//!
//! **Documented instruction set only.** The undocumented `DD`/`FD` byte
//! registers `IXH`/`IXL`/`IYH`/`IYL` are not modelled: a `DD`/`FD` prefix on an
//! opcode that does not address `HL` or `(HL)` is a wasted 4 T-states and the
//! unprefixed instruction executes, which is listed as a limitation rather than
//! an attempt at the undocumented file. `WZ`/`MEMPTR` is not modelled.
//!
//! **Flags.** `S`, `Z`, `H`, `PV`, `N` and `C` follow the documented Z80 rules.
//! The undocumented `Y`/`X` copies follow result bits 5 and 3 where real
//! hardware copies them; instructions whose `Y`/`X` behaviour is undocumented
//! and unused by software are handled by the same rule and not claimed as
//! hardware-exact.
//!
//! **Timing.** Each function returns the documented total T-state count for the
//! instruction, including its opcode fetch. Indexed forms include the `DD`/`FD`
//! prefix fetch. Conditional calls and jumps use the documented taken/not-taken
//! costs (the `JP cc,nn` cost is 10 either way).

use super::{
    FLAG_C, FLAG_H, FLAG_N, FLAG_PV, FLAG_S, FLAG_X, FLAG_Y, FLAG_Z, Target, Z80, Z80Bus, Z80Error,
};

/// Executes one unprefixed opcode and returns its T-state count.
///
/// # Errors
///
/// Propagates a [`Z80Bus`] failure from any operand fetch or data access.
pub fn execute(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    execute_with(z80, bus, opcode, Target::hl())
}

/// Executes one main-table opcode with an `HL`, `IX` or `IY` operand target.
///
/// The prefix module calls this after fetching the displacement byte; the plain
/// table calls it with [`Target::hl`].
pub(crate) fn execute_with(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    match opcode {
        0x00 => Ok(cost(4, target)),
        0x08 => {
            exchange_af(z80);
            Ok(cost(4, target))
        }
        0x10 => djnz(z80, bus, target),
        0x18 => jump_relative(z80, bus, target),
        0x20 | 0x28 | 0x30 | 0x38 => jump_relative_conditional(z80, bus, opcode, target),
        0x01 | 0x11 | 0x21 | 0x31 => load_pair_immediate(z80, bus, opcode, target),
        0x02 | 0x12 => load_indirect_a(z80, bus, opcode, target),
        0x0A | 0x1A => load_a_indirect(z80, bus, opcode, target),
        0x22 => load_memory_pair(z80, bus, target),
        0x2A => load_pair_memory(z80, bus, target),
        0x32 => load_memory_a(z80, bus, target),
        0x3A => load_a_memory(z80, bus, target),
        0x03 | 0x13 | 0x23 | 0x33 => increment_pair(z80, opcode, target, true),
        0x0B | 0x1B | 0x2B | 0x3B => increment_pair(z80, opcode, target, false),
        0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
            increment_byte(z80, bus, opcode, target, true)
        }
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
            increment_byte(z80, bus, opcode, target, false)
        }
        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
            load_immediate(z80, bus, opcode, target)
        }
        0x07 | 0x0F | 0x17 | 0x1F => rotate_accumulator(z80, opcode, target),
        0x09 | 0x19 | 0x29 | 0x39 => add_pair(z80, opcode, target),
        0x27 => {
            decimal_adjust(z80);
            Ok(cost(4, target))
        }
        0x2F => {
            complement_accumulator(z80);
            Ok(cost(4, target))
        }
        0x37 => {
            set_carry(z80);
            Ok(cost(4, target))
        }
        0x3F => {
            complement_carry(z80);
            Ok(cost(4, target))
        }
        0x76 => {
            z80.halted = true;
            Ok(cost(4, target))
        }
        0x40..=0x7F => load_register(z80, bus, opcode, target),
        0x80..=0xBF => alu_register(z80, bus, opcode, target),
        0xC0 | 0xC8 | 0xD0 | 0xD8 | 0xE0 | 0xE8 | 0xF0 | 0xF8 => {
            return_conditional(z80, bus, opcode, target)
        }
        0xC1 | 0xD1 | 0xE1 | 0xF1 => pop_pair(z80, bus, opcode, target),
        0xC2 | 0xCA | 0xD2 | 0xDA | 0xE2 | 0xEA | 0xF2 | 0xFA => {
            jump_conditional(z80, bus, opcode, target)
        }
        0xC3 => jump_absolute(z80, bus, target),
        0xC4 | 0xCC | 0xD4 | 0xDC | 0xE4 | 0xEC | 0xF4 | 0xFC => {
            call_conditional(z80, bus, opcode, target)
        }
        0xC5 | 0xD5 | 0xE5 | 0xF5 => push_pair(z80, bus, opcode, target),
        0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
            alu_immediate(z80, bus, opcode, target)
        }
        0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => restart(z80, bus, opcode, target),
        0xC9 => return_from_call(z80, bus, target),
        0xCD => call_absolute(z80, bus, target),
        0xD3 => output_immediate(z80, bus, target),
        0xD9 => {
            exchange_register_sets(z80);
            Ok(cost(4, target))
        }
        0xDB => input_immediate(z80, bus, target),
        0xE3 => exchange_stack_pair(z80, bus, target),
        0xE9 => jump_pair(z80, target),
        0xEB => {
            exchange_de_hl(z80);
            Ok(cost(4, target))
        }
        0xF3 => {
            z80.iff1 = false;
            z80.iff2 = false;
            z80.ei_delay = false;
            Ok(cost(4, target))
        }
        0xF9 => load_stack_pair(z80, target),
        0xFB => {
            z80.iff1 = true;
            z80.iff2 = true;
            z80.ei_delay = true;
            Ok(cost(4, target))
        }
        // The four prefix bytes are dispatched by `Z80::step` before this table;
        // a direct call treats them as a one-byte NOP.
        0xCB | 0xDD | 0xED | 0xFD => Ok(cost(4, target)),
    }
}

/// Adds the four-T-state prefix fetch for instructions a prefix does not affect.
const fn cost(base: u32, target: Target) -> u32 {
    if target.is_prefixed() { base + 4 } else { base }
}

fn djnz(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let displacement = z80.read_imm8(bus)?;
    let counter = z80.reg8(0).wrapping_sub(1);
    z80.set_reg8(0, counter);
    if counter == 0 {
        Ok(cost(8, target))
    } else {
        z80.pc = z80.pc.wrapping_add_signed(i16::from(displacement as i8));
        Ok(cost(13, target))
    }
}

fn jump_relative(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let displacement = z80.read_imm8(bus)?;
    z80.pc = z80.pc.wrapping_add_signed(i16::from(displacement as i8));
    Ok(cost(12, target))
}

fn jump_relative_conditional(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    // `JR` only encodes the four conditions `NZ`, `Z`, `NC` and `C` (bits 4–3).
    let condition = (opcode >> 3) & 3;
    let displacement = z80.read_imm8(bus)?;
    if z80.condition(condition) {
        z80.pc = z80.pc.wrapping_add_signed(i16::from(displacement as i8));
        Ok(cost(12, target))
    } else {
        Ok(cost(7, target))
    }
}

fn load_pair_immediate(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let pair = (opcode >> 4) & 3;
    let value = z80.read_imm16(bus)?;
    if pair == 2 && target.is_prefixed() {
        target.set_pair(z80, value);
    } else {
        z80.write_pair(pair, value);
    }
    Ok(cost(10, target))
}

fn load_indirect_a(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let address = z80.read_pair((opcode >> 4) & 1);
    bus.write8(address, z80.a)?;
    Ok(cost(7, target))
}

fn load_a_indirect(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let address = z80.read_pair((opcode >> 4) & 1);
    z80.a = bus.read8(address)?;
    Ok(cost(7, target))
}

fn load_memory_pair(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let address = z80.read_imm16(bus)?;
    let value = target.pair(z80);
    bus.write8(address, value as u8)?;
    bus.write8(address.wrapping_add(1), (value >> 8) as u8)?;
    Ok(cost(16, target))
}

fn load_pair_memory(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let address = z80.read_imm16(bus)?;
    let low = bus.read8(address)?;
    let high = bus.read8(address.wrapping_add(1))?;
    target.set_pair(z80, u16::from_le_bytes([low, high]));
    Ok(cost(16, target))
}

fn load_memory_a(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let address = z80.read_imm16(bus)?;
    bus.write8(address, z80.a)?;
    Ok(cost(13, target))
}

fn load_a_memory(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let address = z80.read_imm16(bus)?;
    z80.a = bus.read8(address)?;
    Ok(cost(13, target))
}

fn increment_pair(
    z80: &mut Z80,
    opcode: u8,
    target: Target,
    increment: bool,
) -> Result<u32, Z80Error> {
    let pair = (opcode >> 4) & 3;
    let value = if pair == 2 && target.is_prefixed() {
        target.pair(z80)
    } else {
        z80.read_pair(pair)
    };
    let updated = if increment {
        value.wrapping_add(1)
    } else {
        value.wrapping_sub(1)
    };
    if pair == 2 && target.is_prefixed() {
        target.set_pair(z80, updated);
        Ok(10)
    } else {
        z80.write_pair(pair, updated);
        Ok(cost(6, target))
    }
}

fn increment_byte(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
    increment: bool,
) -> Result<u32, Z80Error> {
    let code = (opcode >> 3) & 7;
    if code == 6 {
        let address = target.address(z80);
        let value = bus.read8(address)?;
        let result = if increment {
            increment_value(z80, value)
        } else {
            decrement_value(z80, value)
        };
        bus.write8(address, result)?;
        Ok(if target.is_prefixed() { 23 } else { 11 })
    } else {
        let value = z80.reg8(code);
        let result = if increment {
            increment_value(z80, value)
        } else {
            decrement_value(z80, value)
        };
        z80.set_reg8(code, result);
        Ok(cost(4, target))
    }
}

fn load_immediate(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let code = (opcode >> 3) & 7;
    if code == 6 {
        if target.is_prefixed() {
            let address = target.address(z80);
            let value = z80.read_imm8(bus)?;
            bus.write8(address, value)?;
            Ok(19)
        } else {
            let value = z80.read_imm8(bus)?;
            bus.write8(z80.hl(), value)?;
            Ok(10)
        }
    } else {
        let value = z80.read_imm8(bus)?;
        z80.set_reg8(code, value);
        Ok(cost(7, target))
    }
}

fn rotate_accumulator(z80: &mut Z80, opcode: u8, target: Target) -> Result<u32, Z80Error> {
    let value = z80.a;
    let carry_in = z80.is_flag(FLAG_C);
    let (result, carry) = match opcode {
        0x07 => (value.rotate_left(1), value & 0x80 != 0),
        0x0F => (value.rotate_right(1), value & 0x01 != 0),
        0x17 => ((value << 1) | u8::from(carry_in), value & 0x80 != 0),
        _ => (
            (value >> 1) | (if carry_in { 0x80 } else { 0 }),
            value & 0x01 != 0,
        ),
    };
    z80.a = result;
    z80.set_flag(FLAG_C, carry);
    z80.set_flag(FLAG_H, false);
    z80.set_flag(FLAG_N, false);
    z80.set_xy(result);
    Ok(cost(4, target))
}

fn add_pair(z80: &mut Z80, opcode: u8, target: Target) -> Result<u32, Z80Error> {
    let source = (opcode >> 4) & 3;
    let left = target.pair(z80);
    let right = if source == 2 {
        target.pair(z80)
    } else {
        z80.read_pair(source)
    };
    let sum = u32::from(left) + u32::from(right);
    let result = sum as u16;
    z80.set_flag(FLAG_H, (left ^ right ^ result) & 0x1000 != 0);
    z80.set_flag(FLAG_N, false);
    z80.set_flag(FLAG_C, sum > 0xFFFF);
    z80.set_xy((result >> 8) as u8);
    target.set_pair(z80, result);
    Ok(cost(11, target))
}

fn load_register(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let destination = (opcode >> 3) & 7;
    let source = opcode & 7;
    if destination == 6 {
        // `0x76` is `HALT` and is dispatched separately, so `source` is not 6.
        let value = z80.reg8(source);
        bus.write8(target.address(z80), value)?;
        return Ok(if target.is_prefixed() { 19 } else { 7 });
    }
    if source == 6 {
        let value = bus.read8(target.address(z80))?;
        z80.set_reg8(destination, value);
        return Ok(if target.is_prefixed() { 19 } else { 7 });
    }
    let value = z80.reg8(source);
    z80.set_reg8(destination, value);
    Ok(cost(4, target))
}

fn alu_register(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let operation = (opcode >> 3) & 7;
    let code = opcode & 7;
    if code == 6 {
        let value = bus.read8(target.address(z80))?;
        apply_alu(z80, operation, value);
        Ok(if target.is_prefixed() { 19 } else { 7 })
    } else {
        let value = z80.reg8(code);
        apply_alu(z80, operation, value);
        Ok(cost(4, target))
    }
}

fn alu_immediate(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let operation = (opcode >> 3) & 7;
    let value = z80.read_imm8(bus)?;
    apply_alu(z80, operation, value);
    Ok(cost(7, target))
}

fn return_conditional(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let condition = (opcode >> 3) & 7;
    if z80.condition(condition) {
        z80.pc = z80.pop16(bus)?;
        Ok(cost(11, target))
    } else {
        Ok(cost(5, target))
    }
}

fn pop_pair(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let pair = (opcode >> 4) & 3;
    let value = z80.pop16(bus)?;
    if pair == 2 && target.is_prefixed() {
        target.set_pair(z80, value);
    } else {
        z80.write_pair(pair, value);
    }
    Ok(cost(10, target))
}

fn jump_conditional(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let condition = (opcode >> 3) & 7;
    let address = z80.read_imm16(bus)?;
    if z80.condition(condition) {
        z80.pc = address;
    }
    Ok(cost(10, target))
}

fn jump_absolute(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let address = z80.read_imm16(bus)?;
    z80.pc = address;
    Ok(cost(10, target))
}

fn call_conditional(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let condition = (opcode >> 3) & 7;
    let address = z80.read_imm16(bus)?;
    if z80.condition(condition) {
        z80.push16(bus, z80.pc)?;
        z80.pc = address;
        Ok(cost(17, target))
    } else {
        Ok(cost(10, target))
    }
}

fn push_pair(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    let pair = (opcode >> 4) & 3;
    let value = if pair == 2 && target.is_prefixed() {
        target.pair(z80)
    } else {
        z80.read_pair(pair)
    };
    z80.push16(bus, value)?;
    Ok(cost(11, target))
}

fn restart(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    opcode: u8,
    target: Target,
) -> Result<u32, Z80Error> {
    z80.push16(bus, z80.pc)?;
    z80.pc = u16::from(opcode & 0x38);
    Ok(cost(11, target))
}

fn return_from_call(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    z80.pc = z80.pop16(bus)?;
    Ok(cost(10, target))
}

fn call_absolute(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let address = z80.read_imm16(bus)?;
    z80.push16(bus, z80.pc)?;
    z80.pc = address;
    Ok(cost(17, target))
}

fn output_immediate(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let low = z80.read_imm8(bus)?;
    let port = (u16::from(z80.a) << 8) | u16::from(low);
    bus.output(port, z80.a)?;
    Ok(cost(11, target))
}

fn input_immediate(z80: &mut Z80, bus: &mut impl Z80Bus, target: Target) -> Result<u32, Z80Error> {
    let low = z80.read_imm8(bus)?;
    let port = (u16::from(z80.a) << 8) | u16::from(low);
    z80.a = bus.input(port)?;
    Ok(cost(11, target))
}

fn exchange_stack_pair(
    z80: &mut Z80,
    bus: &mut impl Z80Bus,
    target: Target,
) -> Result<u32, Z80Error> {
    let pair = target.pair(z80);
    let low = bus.read8(z80.sp)?;
    let high = bus.read8(z80.sp.wrapping_add(1))?;
    bus.write8(z80.sp, pair as u8)?;
    bus.write8(z80.sp.wrapping_add(1), (pair >> 8) as u8)?;
    target.set_pair(z80, u16::from_le_bytes([low, high]));
    Ok(cost(19, target))
}

fn jump_pair(z80: &mut Z80, target: Target) -> Result<u32, Z80Error> {
    z80.pc = target.pair(z80);
    Ok(cost(4, target))
}

fn load_stack_pair(z80: &mut Z80, target: Target) -> Result<u32, Z80Error> {
    z80.sp = target.pair(z80);
    Ok(cost(6, target))
}

fn exchange_af(z80: &mut Z80) {
    std::mem::swap(&mut z80.a, &mut z80.a_alt);
    std::mem::swap(&mut z80.f, &mut z80.f_alt);
}

fn exchange_de_hl(z80: &mut Z80) {
    std::mem::swap(&mut z80.d, &mut z80.h);
    std::mem::swap(&mut z80.e, &mut z80.l);
}

fn exchange_register_sets(z80: &mut Z80) {
    std::mem::swap(&mut z80.b, &mut z80.b_alt);
    std::mem::swap(&mut z80.c, &mut z80.c_alt);
    std::mem::swap(&mut z80.d, &mut z80.d_alt);
    std::mem::swap(&mut z80.e, &mut z80.e_alt);
    std::mem::swap(&mut z80.h, &mut z80.h_alt);
    std::mem::swap(&mut z80.l, &mut z80.l_alt);
}

/// Applies one ALU operation (`ADD`, `ADC`, `SUB`, `SBC`, `AND`, `XOR`, `OR`,
/// `CP`) to `A`.
fn apply_alu(z80: &mut Z80, operation: u8, value: u8) {
    let accumulator = z80.a;
    match operation {
        0 => add_value(z80, accumulator, value, false),
        1 => add_value(z80, accumulator, value, z80.is_flag(FLAG_C)),
        2 => {
            subtract_value(z80, accumulator, value, false, true);
        }
        3 => {
            subtract_value(z80, accumulator, value, z80.is_flag(FLAG_C), true);
        }
        4 => {
            z80.a = accumulator & value;
            z80.set_szxy_parity(z80.a);
            z80.set_flag(FLAG_H, true);
            z80.set_flag(FLAG_N, false);
            z80.set_flag(FLAG_C, false);
        }
        5 => {
            z80.a = accumulator ^ value;
            z80.set_szxy_parity(z80.a);
            z80.set_flag(FLAG_H, false);
            z80.set_flag(FLAG_N, false);
            z80.set_flag(FLAG_C, false);
        }
        6 => {
            z80.a = accumulator | value;
            z80.set_szxy_parity(z80.a);
            z80.set_flag(FLAG_H, false);
            z80.set_flag(FLAG_N, false);
            z80.set_flag(FLAG_C, false);
        }
        _ => {
            subtract_value(z80, accumulator, value, false, false);
        }
    }
}

fn add_value(z80: &mut Z80, left: u8, right: u8, carry: bool) {
    let carry_in = u16::from(carry);
    let sum = u16::from(left) + u16::from(right) + carry_in;
    let result = sum as u8;
    z80.a = result;
    z80.set_szxy(result);
    z80.set_flag(FLAG_H, (left ^ right ^ result) & 0x10 != 0);
    z80.set_flag(FLAG_PV, (left ^ result) & (right ^ result) & 0x80 != 0);
    z80.set_flag(FLAG_N, false);
    z80.set_flag(FLAG_C, sum > 0xFF);
}

fn subtract_value(z80: &mut Z80, left: u8, right: u8, carry: bool, store: bool) -> u8 {
    let borrow = u16::from(carry);
    let difference = u16::from(left)
        .wrapping_sub(u16::from(right))
        .wrapping_sub(borrow);
    let result = difference as u8;
    if store {
        z80.a = result;
    }
    z80.set_szxy(result);
    z80.set_flag(FLAG_H, (left ^ right ^ result) & 0x10 != 0);
    z80.set_flag(FLAG_PV, (left ^ right) & (left ^ result) & 0x80 != 0);
    z80.set_flag(FLAG_N, true);
    z80.set_flag(FLAG_C, u16::from(left) < u16::from(right) + borrow);
    result
}

fn increment_value(z80: &mut Z80, value: u8) -> u8 {
    let result = value.wrapping_add(1);
    z80.set_szxy(result);
    z80.set_flag(FLAG_H, value & 0x0F == 0x0F);
    z80.set_flag(FLAG_PV, value == 0x7F);
    z80.set_flag(FLAG_N, false);
    result
}

fn decrement_value(z80: &mut Z80, value: u8) -> u8 {
    let result = value.wrapping_sub(1);
    z80.set_szxy(result);
    z80.set_flag(FLAG_H, value & 0x0F == 0);
    z80.set_flag(FLAG_PV, value == 0x80);
    z80.set_flag(FLAG_N, true);
    result
}

/// `DAA`: adjusts `A` for packed BCD after an add or subtract.
///
/// The half-carry result is `(old A ^ new A) & 0x10`, the carry is the previous
/// carry or a decimal carry out of the high nibble, and parity follows the
/// adjusted byte. `N` is preserved from the preceding arithmetic.
fn decimal_adjust(z80: &mut Z80) {
    let old = z80.a;
    let subtract = z80.is_flag(FLAG_N);
    let half_carry = z80.is_flag(FLAG_H);
    let carry_in = z80.is_flag(FLAG_C);
    let mut correction = 0_u8;
    if old & 0x0F > 9 || half_carry {
        correction |= 0x06;
    }
    let carry_out = carry_in || old > 0x99;
    if carry_out {
        correction |= 0x60;
    }
    let result = if subtract {
        old.wrapping_sub(correction)
    } else {
        old.wrapping_add(correction)
    };
    z80.a = result;
    z80.set_szxy_parity(result);
    z80.set_flag(FLAG_H, (old ^ result) & 0x10 != 0);
    z80.set_flag(FLAG_N, subtract);
    z80.set_flag(FLAG_C, carry_out);
}

fn complement_accumulator(z80: &mut Z80) {
    z80.a = !z80.a;
    z80.set_flag(FLAG_H, true);
    z80.set_flag(FLAG_N, true);
    z80.set_xy(z80.a);
}

fn set_carry(z80: &mut Z80) {
    z80.set_flag(FLAG_C, true);
    z80.set_flag(FLAG_H, false);
    z80.set_flag(FLAG_N, false);
    z80.set_xy(z80.a);
}

fn complement_carry(z80: &mut Z80) {
    let carry = z80.is_flag(FLAG_C);
    z80.set_flag(FLAG_H, carry);
    z80.set_flag(FLAG_C, !carry);
    z80.set_flag(FLAG_N, false);
    z80.set_xy(z80.a);
}

/// Sets the 16-bit sign/zero and undocumented `Y`/`X` flags.
pub(crate) fn set_szxy16(z80: &mut Z80, value: u16) {
    z80.set_flag(FLAG_S, value & 0x8000 != 0);
    z80.set_flag(FLAG_Z, value == 0);
    z80.set_flag(FLAG_Y, value & 0x2000 != 0);
    z80.set_flag(FLAG_X, value & 0x0800 != 0);
}
