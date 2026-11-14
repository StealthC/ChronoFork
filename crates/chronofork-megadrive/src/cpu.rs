//! A deliberately small, documented subset of the Motorola 68000.
//!
//! This is the first real-hardware slice of ChronoFork, not a complete 68000.
//! It exists to prove reset-vector loading, big-endian execution, deterministic
//! state, snapshots and session forking with an independently authored program.
//!
//! Implemented today:
//!
//! - `MOVEQ`, `MOVE.B/W/L` between `Dn`, immediate and absolute-long operands;
//! - `ADD.B/W/L`/`SUB.B/W/L` between data registers, plus `ADDQ`/`SUBQ` word/long;
//! - `CLR.W/L`, `NOP`, `BRA`, `BSR`, `BEQ`, `BNE`, `JMP`, `JSR`, `RTS`.
//!
//! Not implemented: effective-address modes beyond the listed forms, exception
//! processing, interrupts, supervisor/user separation, prefetch, cycle-accurate
//! timing, and the rest of the instruction set. Unsupported opcodes are errors,
//! never silently skipped.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::bus::{BusError, MegadriveBus};

const FLAG_C: u16 = 0x0001;
const FLAG_V: u16 = 0x0002;
const FLAG_Z: u16 = 0x0004;
const FLAG_N: u16 = 0x0008;
const FLAG_X: u16 = 0x0010;
const SR_SUPERVISOR: u16 = 0x2000;
const SR_INTERRUPT_MASK: u16 = 0x0700;
const RESET_SR: u16 = SR_SUPERVISOR | SR_INTERRUPT_MASK;

/// A failure while executing one 68000 instruction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CpuError {
    /// The CPU was already halted.
    Halted,
    /// The opcode is not implemented by this slice.
    IllegalOpcode {
        /// Address of the opcode.
        pc: u32,
        /// Fetched opcode.
        opcode: u16,
    },
    /// A recognized opcode uses an addressing/size form not implemented here.
    Unsupported(String),
    /// The program counter or branch target is not word-aligned.
    AddressError {
        /// Offending address.
        address: u32,
    },
    /// Address arithmetic overflowed.
    AddressOverflow,
    /// The stack pointer moved outside the representable address space.
    StackOverflow,
    /// The bus rejected an access.
    Bus(BusError),
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Halted => f.write_str("CPU is halted"),
            Self::IllegalOpcode { pc, opcode } => {
                write!(f, "illegal opcode 0x{opcode:04X} at 0x{pc:06X}")
            }
            Self::Unsupported(message) => write!(f, "unsupported 68000 form: {message}"),
            Self::AddressError { address } => {
                write!(f, "68000 address error at 0x{address:06X}")
            }
            Self::AddressOverflow => f.write_str("68000 address overflow"),
            Self::StackOverflow => f.write_str("68000 stack pointer overflow"),
            Self::Bus(error) => write!(f, "68000 bus error: {error}"),
        }
    }
}

impl Error for CpuError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Bus(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BusError> for CpuError {
    fn from(error: BusError) -> Self {
        Self::Bus(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Size {
    Byte,
    Word,
    Long,
}

impl Size {
    const fn mask(self) -> u32 {
        match self {
            Self::Byte => 0x0000_00FF,
            Self::Word => 0x0000_FFFF,
            Self::Long => 0xFFFF_FFFF,
        }
    }

    const fn sign_bit(self) -> u32 {
        match self {
            Self::Byte => 0x0000_0080,
            Self::Word => 0x0000_8000,
            Self::Long => 0x8000_0000,
        }
    }
}

/// 68000 architectural state for the implemented slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct M68000 {
    d: [u32; 8],
    a: [u32; 8],
    pc: u32,
    sr: u16,
    halted: bool,
}

impl M68000 {
    /// Creates a CPU with all registers clear.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            d: [0; 8],
            a: [0; 8],
            pc: 0,
            sr: RESET_SR,
            halted: false,
        }
    }

    /// Loads the initial supervisor stack pointer and program counter from the
    /// reset vectors at addresses 0 and 4.
    pub fn reset(&mut self, bus: &mut MegadriveBus) -> Result<(), CpuError> {
        self.d = [0; 8];
        self.a = [0; 8];
        self.a[7] = bus.read32(0)?;
        self.pc = bus.read32(4)?;
        self.sr = RESET_SR;
        self.halted = false;
        self.ensure_even_pc()
    }

    /// Returns the program counter.
    #[must_use]
    pub const fn pc(&self) -> u32 {
        self.pc
    }

    /// Returns the status register.
    #[must_use]
    pub const fn sr(&self) -> u16 {
        self.sr
    }

    /// Returns whether the CPU is halted.
    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }

    /// Reads a data register.
    #[must_use]
    pub fn data_register(&self, register: usize) -> Option<u32> {
        self.d.get(register).copied()
    }

    /// Reads an address register.
    #[must_use]
    pub fn address_register(&self, register: usize) -> Option<u32> {
        self.a.get(register).copied()
    }

    /// Returns a debugger-friendly register map.
    #[must_use]
    pub fn registers(&self) -> BTreeMap<String, u64> {
        let mut registers = BTreeMap::new();
        for (index, value) in self.d.iter().enumerate() {
            registers.insert(format!("d{index}"), u64::from(*value));
        }
        for (index, value) in self.a.iter().enumerate() {
            registers.insert(format!("a{index}"), u64::from(*value));
        }
        registers.insert("pc".to_owned(), u64::from(self.pc));
        registers.insert("sr".to_owned(), u64::from(self.sr));
        registers
    }

    pub(crate) const fn snapshot_state(&self) -> ([u32; 8], [u32; 8], u32, u16, bool) {
        (self.d, self.a, self.pc, self.sr, self.halted)
    }

    pub(crate) fn restore_state(
        &mut self,
        d: [u32; 8],
        a: [u32; 8],
        pc: u32,
        sr: u16,
        halted: bool,
    ) {
        self.d = d;
        self.a = a;
        self.pc = pc;
        self.sr = sr;
        self.halted = halted;
    }

    /// Executes one instruction.
    pub fn step(&mut self, bus: &mut MegadriveBus) -> Result<(), CpuError> {
        if self.halted {
            return Err(CpuError::Halted);
        }
        let instruction_pc = self.pc;
        let opcode = self.fetch_u16(bus)?;

        // 0111 rrr 0 dddddddd
        if (opcode & 0xF100) == 0x7000 {
            let register = usize::from((opcode >> 9) & 0x7);
            let value = i32::from(i8::from_be_bytes([(opcode & 0xFF) as u8]));
            self.d[register] = value as u32;
            self.set_logic_flags(value as u32, Size::Long);
            return Ok(());
        }

        // Branches and BSR.
        if (opcode & 0xF000) == 0x6000 {
            return self.execute_branch(bus, opcode);
        }

        match opcode {
            0x4E71 => Ok(()),
            0x4E75 => {
                let pc = self.pop_long(bus)?;
                self.write_pc(pc)
            }
            0x4EF9 => {
                let target = self.fetch_u32(bus)?;
                self.write_pc(target)
            }
            0x4EB9 => {
                let target = self.fetch_u32(bus)?;
                self.ensure_even(target)?;
                let return_pc = self.pc;
                self.push_long(bus, return_pc)?;
                self.pc = target;
                Ok(())
            }
            _ => self.execute_other(bus, opcode, instruction_pc),
        }
    }

    fn execute_other(
        &mut self,
        bus: &mut MegadriveBus,
        opcode: u16,
        instruction_pc: u32,
    ) -> Result<(), CpuError> {
        // CLR.W / CLR.L Dn
        if (opcode & 0xFFC0) == 0x4240 {
            let register = usize::from(opcode & 0x7);
            self.write_dn(register, 0, Size::Word);
            self.set_logic_flags(0, Size::Word);
            return Ok(());
        }
        if (opcode & 0xFFC0) == 0x4280 {
            let register = usize::from(opcode & 0x7);
            self.d[register] = 0;
            self.set_logic_flags(0, Size::Long);
            return Ok(());
        }

        // MOVE.W/L #imm,Dn
        if (opcode & 0xF1FF) == 0x303C {
            let register = usize::from((opcode >> 9) & 0x7);
            let value = u32::from(self.fetch_u16(bus)?);
            self.write_dn(register, value, Size::Word);
            self.set_logic_flags(value, Size::Word);
            return Ok(());
        }
        if (opcode & 0xF1FF) == 0x203C {
            let register = usize::from((opcode >> 9) & 0x7);
            let value = self.fetch_u32(bus)?;
            self.d[register] = value;
            self.set_logic_flags(value, Size::Long);
            return Ok(());
        }

        // MOVE.B/W/L Dn,Dm
        if (opcode & 0x01C0) == 0 && (opcode & 0x0038) == 0 {
            let source = usize::from(opcode & 0x7);
            let destination = usize::from((opcode >> 9) & 0x7);
            match opcode & 0xF000 {
                0x1000 => {
                    let value = self.read_dn(source, Size::Byte);
                    self.write_dn(destination, value, Size::Byte);
                    self.set_logic_flags(value, Size::Byte);
                    return Ok(());
                }
                0x2000 => {
                    let value = self.d[source];
                    self.d[destination] = value;
                    self.set_logic_flags(value, Size::Long);
                    return Ok(());
                }
                0x3000 => {
                    let value = self.read_dn(source, Size::Word);
                    self.write_dn(destination, value, Size::Word);
                    self.set_logic_flags(value, Size::Word);
                    return Ok(());
                }
                _ => {}
            }
        }

        // MOVE.B/W/L Dn,(abs).L
        if (opcode & 0xFFF8) == 0x13C0 {
            let source = usize::from(opcode & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = self.read_dn(source, Size::Byte);
            bus.write8(address, value as u8)?;
            self.set_logic_flags(value, Size::Byte);
            return Ok(());
        }
        if (opcode & 0xFFF8) == 0x23C0 {
            let source = usize::from(opcode & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = self.d[source];
            bus.write32(address, value)?;
            self.set_logic_flags(value, Size::Long);
            return Ok(());
        }
        if (opcode & 0xFFF8) == 0x33C0 {
            let source = usize::from(opcode & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = self.read_dn(source, Size::Word);
            bus.write16(address, value as u16)?;
            self.set_logic_flags(value, Size::Word);
            return Ok(());
        }

        // MOVE.B/W/L (abs).L,Dn
        if (opcode & 0xF1FF) == 0x1038 {
            let destination = usize::from((opcode >> 9) & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = u32::from(bus.read8(address)?);
            self.write_dn(destination, value, Size::Byte);
            self.set_logic_flags(value, Size::Byte);
            return Ok(());
        }
        if (opcode & 0xF1FF) == 0x2038 {
            let destination = usize::from((opcode >> 9) & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = bus.read32(address)?;
            self.d[destination] = value;
            self.set_logic_flags(value, Size::Long);
            return Ok(());
        }
        if (opcode & 0xF1FF) == 0x3038 {
            let destination = usize::from((opcode >> 9) & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = u32::from(bus.read16(address)?);
            self.write_dn(destination, value, Size::Word);
            self.set_logic_flags(value, Size::Word);
            return Ok(());
        }

        // ADDQ / SUBQ (Dn only).
        if (opcode & 0xF000) == 0x5000 {
            let subtract = opcode & 0x0100 != 0;
            let mut immediate = u32::from((opcode >> 9) & 0x7);
            if immediate == 0 {
                immediate = 8;
            }
            let mode = (opcode >> 3) & 0x7;
            let size_bits = (opcode >> 6) & 0x3;
            let destination = usize::from(opcode & 0x7);
            if mode != 0 {
                return Err(CpuError::Unsupported(
                    "ADDQ/SUBQ only supports data-register destinations".to_owned(),
                ));
            }
            let size = match size_bits {
                1 => Size::Word,
                2 => Size::Long,
                _ => {
                    return Err(CpuError::Unsupported(
                        "ADDQ/SUBQ only supports word and long sizes".to_owned(),
                    ));
                }
            };
            if subtract {
                self.subtract(destination, immediate, size);
            } else {
                self.add(destination, immediate, size);
            }
            return Ok(());
        }

        // ADD/SUB Dn,Dn (byte/word/long). For this form the destination
        // mode is bits 5-3; the size is bits 7-6.
        if (opcode & 0x0038) == 0 {
            let source = usize::from(opcode & 0x7);
            let destination = usize::from((opcode >> 9) & 0x7);
            let size = match opcode & 0x00C0 {
                0x0000 => Some(Size::Byte),
                0x0040 => Some(Size::Word),
                0x0080 => Some(Size::Long),
                _ => None,
            };
            if let Some(size) = size {
                let source_value = self.read_dn(source, size);
                if (opcode & 0xF000) == 0xD000 {
                    self.add(destination, source_value, size);
                    return Ok(());
                }
                if (opcode & 0xF000) == 0x9000 {
                    self.subtract(destination, source_value, size);
                    return Ok(());
                }
            }
        }

        Err(CpuError::IllegalOpcode {
            pc: instruction_pc,
            opcode,
        })
    }

    fn execute_branch(&mut self, bus: &mut MegadriveBus, opcode: u16) -> Result<(), CpuError> {
        let displacement = i32::from(i8::from_be_bytes([(opcode & 0xFF) as u8]));
        let condition = (opcode >> 8) & 0xF;
        match condition {
            0x0 | 0x1 => {
                let target = self.relative_target(displacement)?;
                if condition == 0x1 {
                    let return_pc = self.pc;
                    self.push_long(bus, return_pc)?;
                }
                self.write_pc(target)
            }
            0x6 => {
                if self.flag(FLAG_Z) {
                    Ok(())
                } else {
                    let target = self.relative_target(displacement)?;
                    self.write_pc(target)
                }
            }
            0x7 => {
                if self.flag(FLAG_Z) {
                    let target = self.relative_target(displacement)?;
                    self.write_pc(target)
                } else {
                    Ok(())
                }
            }
            _ => Err(CpuError::Unsupported(format!(
                "branch condition 0x{condition:X} is not implemented"
            ))),
        }
    }

    fn relative_target(&self, displacement: i32) -> Result<u32, CpuError> {
        let target = i64::from(self.pc) + i64::from(displacement);
        if target < 0 || target > i64::from(u32::MAX) {
            return Err(CpuError::AddressOverflow);
        }
        let target = u32::try_from(target).map_err(|_| CpuError::AddressOverflow)?;
        self.ensure_even(target)?;
        Ok(target)
    }

    fn add(&mut self, destination: usize, source: u32, size: Size) {
        let destination_value = self.read_dn(destination, size);
        let source_value = source & size.mask();
        let result = destination_value.wrapping_add(source_value) & size.mask();
        self.write_dn(destination, result, size);
        self.set_add_flags(destination_value, source_value, result, size);
    }

    fn subtract(&mut self, destination: usize, source: u32, size: Size) {
        let destination_value = self.read_dn(destination, size);
        let source_value = source & size.mask();
        let result = destination_value.wrapping_sub(source_value) & size.mask();
        self.write_dn(destination, result, size);
        self.set_sub_flags(destination_value, source_value, result, size);
    }

    fn read_dn(&self, register: usize, size: Size) -> u32 {
        self.d.get(register).copied().unwrap_or(0) & size.mask()
    }

    fn write_dn(&mut self, register: usize, value: u32, size: Size) {
        if let Some(register) = self.d.get_mut(register) {
            *register = match size {
                Size::Byte => (*register & 0xFFFF_FF00) | (value & 0xFF),
                Size::Word => (*register & 0xFFFF_0000) | (value & 0xFFFF),
                Size::Long => value,
            };
        }
    }

    fn set_logic_flags(&mut self, value: u32, size: Size) {
        let masked = value & size.mask();
        let negative = masked & size.sign_bit() != 0;
        let zero = masked == 0;
        let extend = self.flag(FLAG_X);
        self.set_ccr(false, false, zero, negative, extend);
    }

    fn set_add_flags(&mut self, destination: u32, source: u32, result: u32, size: Size) {
        let mask = size.mask();
        let destination = destination & mask;
        let source = source & mask;
        let result = result & mask;
        let carry = match size {
            Size::Byte => u64::from(destination) + u64::from(source) > 0xFF,
            Size::Word => u64::from(destination) + u64::from(source) > 0xFFFF,
            Size::Long => u64::from(destination) + u64::from(source) > u64::from(u32::MAX),
        };
        let overflow = (!(destination ^ source) & (destination ^ result) & size.sign_bit()) != 0;
        self.set_ccr(
            carry,
            overflow,
            result == 0,
            result & size.sign_bit() != 0,
            carry,
        );
    }

    fn set_sub_flags(&mut self, destination: u32, source: u32, result: u32, size: Size) {
        let mask = size.mask();
        let destination = destination & mask;
        let source = source & mask;
        let result = result & mask;
        let borrow = u64::from(destination) < u64::from(source);
        let overflow = ((destination ^ source) & (destination ^ result) & size.sign_bit()) != 0;
        self.set_ccr(
            borrow,
            overflow,
            result == 0,
            result & size.sign_bit() != 0,
            borrow,
        );
    }

    fn set_ccr(&mut self, carry: bool, overflow: bool, zero: bool, negative: bool, extend: bool) {
        let mut ccr = self.sr & 0xFF00;
        if carry {
            ccr |= FLAG_C;
        }
        if overflow {
            ccr |= FLAG_V;
        }
        if zero {
            ccr |= FLAG_Z;
        }
        if negative {
            ccr |= FLAG_N;
        }
        if extend {
            ccr |= FLAG_X;
        }
        self.sr = ccr;
    }

    fn flag(&self, flag: u16) -> bool {
        self.sr & flag != 0
    }

    fn fetch_u16(&mut self, bus: &mut MegadriveBus) -> Result<u16, CpuError> {
        let value = bus.read16(self.pc)?;
        self.pc = self.pc.checked_add(2).ok_or(CpuError::AddressOverflow)?;
        Ok(value)
    }

    fn fetch_u32(&mut self, bus: &mut MegadriveBus) -> Result<u32, CpuError> {
        let high = u32::from(self.fetch_u16(bus)?);
        let low = u32::from(self.fetch_u16(bus)?);
        Ok((high << 16) | low)
    }

    fn push_long(&mut self, bus: &mut MegadriveBus, value: u32) -> Result<(), CpuError> {
        let stack = self.a[7].checked_sub(4).ok_or(CpuError::StackOverflow)?;
        bus.write32(stack, value)?;
        self.a[7] = stack;
        Ok(())
    }

    fn pop_long(&mut self, bus: &mut MegadriveBus) -> Result<u32, CpuError> {
        let stack = self.a[7];
        let value = bus.read32(stack)?;
        self.a[7] = stack.checked_add(4).ok_or(CpuError::StackOverflow)?;
        Ok(value)
    }

    fn write_pc(&mut self, pc: u32) -> Result<(), CpuError> {
        self.ensure_even(pc)?;
        self.pc = pc;
        Ok(())
    }

    fn ensure_even_pc(&self) -> Result<(), CpuError> {
        self.ensure_even(self.pc)
    }

    fn ensure_even(&self, address: u32) -> Result<(), CpuError> {
        if address & 1 != 0 {
            Err(CpuError::AddressError { address })
        } else {
            Ok(())
        }
    }
}

impl Default for M68000 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::M68000;
    use crate::bus::MegadriveBus;

    fn bus_with_vectors() -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&0x00FF_0000_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        MegadriveBus::new(Arc::from(rom.into_boxed_slice()))
    }

    #[test]
    fn reset_loads_vectors_and_clears_registers() {
        let mut bus = bus_with_vectors();
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).expect("reset");
        assert_eq!(cpu.pc(), 0x200);
        assert_eq!(cpu.address_register(7), Some(0x00FF_0000));
        assert_eq!(cpu.data_register(0), Some(0));
        assert_eq!(cpu.sr(), 0x2700);
    }

    #[test]
    fn moveq_and_add_execute() {
        // The bus is ROM-backed, so build the program into a fresh ROM.
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&0x00FF_0000_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        rom[0x200..0x206].copy_from_slice(&[0x70, 0x01, 0x72, 0x02, 0xD0, 0x41]);
        let mut bus = MegadriveBus::new(Arc::from(rom.into_boxed_slice()));
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).expect("reset");
        cpu.step(&mut bus).expect("moveq 1");
        cpu.step(&mut bus).expect("moveq 2");
        cpu.step(&mut bus).expect("add");
        assert_eq!(cpu.data_register(0), Some(3));
        assert_eq!(cpu.data_register(1), Some(2));
    }
}
