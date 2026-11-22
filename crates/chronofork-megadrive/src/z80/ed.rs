//! `ED`-prefixed Z80 instructions.
//!
//! Implemented groups: the 16-bit `ADC`/`SBC`, the extended `LD (nn),rr` and
//! `LD rr,(nn)`, `NEG`, `RETN`/`RETI`, the interrupt modes, `LD I,A`/`LD R,A`/
//! `LD A,I`/`LD A,R`, `RRD`/`RLD`, the `IN`/`OUT` forms and the block
//! move/search/input/output instructions.
//!
//! **Undocumented `ED` opcodes** (every value not listed above) are decoded by
//! real Z80 hardware as a two-byte, 8 T-state `NOP`; that is what this table
//! does. `IN (C)` (`ED 70`) and `OUT (C),0` (`ED 71`) are the undocumented but
//! well-defined forms and are implemented. The block I/O instructions set the
//! documented `Z`/`N` flags and leave the documented-as-undefined flags
//! untouched, so repeated runs stay deterministic.
//!
//! Block repeat variants rewind `PC` by two bytes and cost 21 T-states while
//! the repeat condition holds, 16 when it ends.

use super::{
    FLAG_C, FLAG_H, FLAG_N, FLAG_PV, FLAG_S, FLAG_Z, Z80, Z80Bus, Z80Error, ops::set_szxy16,
};

/// Executes one `ED`-prefixed opcode and returns its T-state count.
///
/// # Errors
///
/// Propagates a [`Z80Bus`] failure from a block move, search or I/O access.
pub fn execute(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    match opcode {
        0x40 | 0x48 | 0x50 | 0x58 | 0x60 | 0x68 | 0x70 | 0x78 => input_register(z80, bus, opcode),
        0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x71 | 0x79 => output_register(z80, bus, opcode),
        0x42 | 0x52 | 0x62 | 0x72 => subtract_with_carry(z80, opcode),
        0x4A | 0x5A | 0x6A | 0x7A => add_with_carry(z80, opcode),
        0x43 | 0x53 | 0x63 | 0x73 => load_memory_pair(z80, bus, opcode),
        0x4B | 0x5B | 0x6B | 0x7B => load_pair_memory(z80, bus, opcode),
        0x44 | 0x4C | 0x54 | 0x5C | 0x64 | 0x6C | 0x74 | 0x7C => {
            negate(z80);
            Ok(8)
        }
        0x45 | 0x4D | 0x55 | 0x5D | 0x65 | 0x6D | 0x75 | 0x7D => return_from_interrupt(z80, bus),
        0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x76 | 0x7E => {
            set_interrupt_mode(z80, opcode);
            Ok(8)
        }
        0x47 => {
            z80.i = z80.a;
            Ok(9)
        }
        0x4F => {
            z80.r = z80.a;
            Ok(9)
        }
        0x57 => load_accumulator_i(z80),
        0x5F => load_accumulator_r(z80),
        0x67 => rotate_bcd_right(z80, bus),
        0x6F => rotate_bcd_left(z80, bus),
        0xA0..=0xA3 | 0xA8..=0xAB | 0xB0..=0xB3 | 0xB8..=0xBB => block(z80, bus, opcode),
        // Every other `ED` opcode is an undocumented two-byte NOP.
        _ => Ok(8),
    }
}

fn input_register(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    let register = (opcode >> 3) & 7;
    let value = bus.input(z80.bc())?;
    if register != 6 {
        z80.set_reg8(register, value);
    }
    z80.set_szxy_parity(value);
    z80.set_flag(FLAG_H, false);
    z80.set_flag(FLAG_N, false);
    // The carry flag is documented as unaffected by `IN r,(C)`.
    Ok(12)
}

fn output_register(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    let register = (opcode >> 3) & 7;
    let value = if register == 6 { 0 } else { z80.reg8(register) };
    bus.output(z80.bc(), value)?;
    Ok(12)
}

fn add_with_carry(z80: &mut Z80, opcode: u8) -> Result<u32, Z80Error> {
    let source = (opcode >> 4) & 3;
    let left = z80.hl();
    let right = z80.read_pair(source);
    let carry = u32::from(z80.is_flag(FLAG_C));
    let sum = u32::from(left) + u32::from(right) + carry;
    let result = sum as u16;
    z80.set_hl(result);
    set_szxy16(z80, result);
    z80.set_flag(FLAG_H, (left ^ right ^ result) & 0x1000 != 0);
    z80.set_flag(FLAG_PV, (left ^ result) & (right ^ result) & 0x8000 != 0);
    z80.set_flag(FLAG_N, false);
    z80.set_flag(FLAG_C, sum > 0xFFFF);
    Ok(15)
}

fn subtract_with_carry(z80: &mut Z80, opcode: u8) -> Result<u32, Z80Error> {
    let source = (opcode >> 4) & 3;
    let left = z80.hl();
    let right = z80.read_pair(source);
    let borrow = u32::from(z80.is_flag(FLAG_C));
    let difference = u32::from(left)
        .wrapping_sub(u32::from(right))
        .wrapping_sub(borrow);
    let result = difference as u16;
    z80.set_hl(result);
    set_szxy16(z80, result);
    z80.set_flag(FLAG_H, (left ^ right ^ result) & 0x1000 != 0);
    z80.set_flag(FLAG_PV, (left ^ right) & (left ^ result) & 0x8000 != 0);
    z80.set_flag(FLAG_N, true);
    z80.set_flag(FLAG_C, u32::from(left) < u32::from(right) + borrow);
    Ok(15)
}

fn load_memory_pair(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    let address = z80.read_imm16(bus)?;
    let value = z80.read_pair((opcode >> 4) & 3);
    bus.write8(address, value as u8)?;
    bus.write8(address.wrapping_add(1), (value >> 8) as u8)?;
    Ok(20)
}

fn load_pair_memory(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    let pair = (opcode >> 4) & 3;
    let address = z80.read_imm16(bus)?;
    let low = bus.read8(address)?;
    let high = bus.read8(address.wrapping_add(1))?;
    z80.write_pair(pair, u16::from_le_bytes([low, high]));
    Ok(20)
}

fn negate(z80: &mut Z80) {
    let value = z80.a;
    let result = 0_u8.wrapping_sub(value);
    z80.a = result;
    z80.set_szxy(result);
    z80.set_flag(FLAG_H, (value ^ result) & 0x10 != 0);
    z80.set_flag(FLAG_PV, value == 0x80);
    z80.set_flag(FLAG_N, true);
    z80.set_flag(FLAG_C, value != 0);
}

fn return_from_interrupt(z80: &mut Z80, bus: &mut impl Z80Bus) -> Result<u32, Z80Error> {
    z80.pc = z80.pop16(bus)?;
    z80.iff1 = z80.iff2;
    Ok(14)
}

fn set_interrupt_mode(z80: &mut Z80, opcode: u8) {
    z80.im = match (opcode >> 3) & 3 {
        0 | 1 => 0,
        2 => 1,
        _ => 2,
    };
}

fn load_accumulator_i(z80: &mut Z80) -> Result<u32, Z80Error> {
    let value = z80.i;
    z80.a = value;
    z80.set_szxy(value);
    z80.set_flag(FLAG_H, false);
    z80.set_flag(FLAG_N, false);
    z80.set_flag(FLAG_PV, z80.iff2);
    // The carry flag is documented as unaffected by `LD A,I`.
    Ok(9)
}

fn load_accumulator_r(z80: &mut Z80) -> Result<u32, Z80Error> {
    let value = z80.r;
    z80.a = value;
    z80.set_szxy(value);
    z80.set_flag(FLAG_H, false);
    z80.set_flag(FLAG_N, false);
    z80.set_flag(FLAG_PV, z80.iff2);
    Ok(9)
}

fn rotate_bcd_right(z80: &mut Z80, bus: &mut impl Z80Bus) -> Result<u32, Z80Error> {
    let address = z80.hl();
    let memory = bus.read8(address)?;
    let accumulator = z80.a;
    bus.write8(address, (accumulator << 4) | (memory >> 4))?;
    z80.a = (accumulator & 0xF0) | (memory & 0x0F);
    z80.set_szxy_parity(z80.a);
    z80.set_flag(FLAG_H, false);
    z80.set_flag(FLAG_N, false);
    // The carry flag is unaffected.
    Ok(18)
}

fn rotate_bcd_left(z80: &mut Z80, bus: &mut impl Z80Bus) -> Result<u32, Z80Error> {
    let address = z80.hl();
    let memory = bus.read8(address)?;
    let accumulator = z80.a;
    bus.write8(address, (memory << 4) | (accumulator & 0x0F))?;
    z80.a = (accumulator & 0xF0) | (memory >> 4);
    z80.set_szxy_parity(z80.a);
    z80.set_flag(FLAG_H, false);
    z80.set_flag(FLAG_N, false);
    Ok(18)
}

/// Executes `LDI`/`LDD`/`LDIR`/`LDDR`, `CPI`/`CPD`/`CPIR`/`CPDR`,
/// `INI`/`IND`/`INIR`/`INDR` and `OUTI`/`OUTD`/`OTIR`/`OTDR`.
fn block(z80: &mut Z80, bus: &mut impl Z80Bus, opcode: u8) -> Result<u32, Z80Error> {
    let operation = opcode & 0x07;
    let decrement = opcode & 0x08 != 0;
    let repeat = opcode & 0x10 != 0;
    let step = if decrement { u16::MAX } else { 1 };
    match operation {
        0 => {
            let value = bus.read8(z80.hl())?;
            bus.write8(z80.de(), value)?;
            z80.set_hl(z80.hl().wrapping_add(step));
            z80.set_de(z80.de().wrapping_add(step));
            z80.set_bc(z80.bc().wrapping_sub(1));
            z80.set_flag(FLAG_H, false);
            z80.set_flag(FLAG_PV, z80.bc() != 0);
            z80.set_flag(FLAG_N, false);
            // `S`, `Z` and `C` are documented as unaffected; the undocumented
            // `Y`/`X` bits are not modelled for block moves and stay clear.
        }
        1 => {
            let value = bus.read8(z80.hl())?;
            let result = z80.a.wrapping_sub(value);
            z80.set_flag(FLAG_S, result & 0x80 != 0);
            z80.set_flag(FLAG_Z, result == 0);
            z80.set_flag(FLAG_H, (z80.a ^ value ^ result) & 0x10 != 0);
            z80.set_flag(FLAG_N, true);
            z80.set_hl(z80.hl().wrapping_add(step));
            z80.set_bc(z80.bc().wrapping_sub(1));
            z80.set_flag(FLAG_PV, z80.bc() != 0);
            // `C` is documented as unaffected.
        }
        2 => {
            let value = bus.input(z80.bc())?;
            bus.write8(z80.hl(), value)?;
            z80.set_reg8(0, z80.reg8(0).wrapping_sub(1));
            z80.set_hl(z80.hl().wrapping_add(step));
            z80.set_flag(FLAG_Z, z80.reg8(0) == 0);
            z80.set_flag(FLAG_N, value & 0x80 != 0);
        }
        _ => {
            let value = bus.read8(z80.hl())?;
            z80.set_reg8(0, z80.reg8(0).wrapping_sub(1));
            // As on hardware, the port address is `BC` after the `B` decrement.
            bus.output(z80.bc(), value)?;
            z80.set_hl(z80.hl().wrapping_add(step));
            z80.set_flag(FLAG_Z, z80.reg8(0) == 0);
            z80.set_flag(FLAG_N, value & 0x80 != 0);
        }
    }
    let repeat_condition = match operation {
        0 => z80.bc() != 0,
        1 => z80.bc() != 0 && !z80.is_flag(FLAG_Z),
        _ => z80.reg8(0) != 0,
    };
    if repeat && repeat_condition {
        z80.pc = z80.pc.wrapping_sub(2);
        Ok(21)
    } else {
        Ok(16)
    }
}
