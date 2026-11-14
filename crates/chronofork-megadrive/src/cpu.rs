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
//! - `CMP.B/W/L Dn,Dn`, `TST.B/W/L Dn`, `CLR.W/L`, `LEA (abs).L,An`, `PEA (abs).L`;
//! - `NOP`, `BRA`, `BSR`, `BEQ`, `BNE`, `BCC`, `BCS`, `BMI`, `BPL`, `DBRA`,
//!   `JMP`, `JSR`, `RTS`;
//! - interrupt exception entry (`M68000::interrupt`) through a vector-table
//!   entry, with a supervisor stack frame and the `SR` supervisor bit;
//! - an optional bounded instruction trace (see [`crate::trace`]) recording each
//!   fetched address/opcode pair. It is disabled by default and is debug data:
//!   it is not part of snapshots, state hashes or registers.
//!
//! Not implemented: effective-address modes beyond the listed forms, interrupt
//! levels/priority masks and automatic vector selection, other exception groups,
//! separate user/supervisor stack pointers, prefetch, cycle-accurate timing, and
//! the rest of the instruction set. Unsupported opcodes are errors, never
//! silently skipped.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::bus::{BusError, MegadriveBus};
use crate::trace::{InstructionTrace, TraceError};

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
    /// Debug-only instruction trace; never part of snapshots or state hashes.
    trace: Option<InstructionTrace>,
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
            trace: None,
        }
    }

    /// Loads the initial supervisor stack pointer and program counter from the
    /// reset vectors at addresses 0 and 4.
    ///
    /// The instruction trace, if enabled, is left untouched; call
    /// [`M68000::reset_trace`] explicitly to discard pre-reset records.
    pub fn reset(&mut self, bus: &mut MegadriveBus) -> Result<(), CpuError> {
        self.d = [0; 8];
        self.a = [0; 8];
        self.a[7] = bus.read32(0)?;
        self.pc = bus.read32(4)?;
        self.sr = RESET_SR;
        self.halted = false;
        self.ensure_even_pc()
    }

    /// Enters a 68000 interrupt exception through one vector-table entry.
    ///
    /// The processor builds its exception frame on the (supervisor) stack: a
    /// word containing the current status register at the lowest address and a
    /// long containing the return program counter immediately above it, so the
    /// stack pointer decreases by six. The target program counter is then read
    /// as a big-endian long from `vector_address` (for example `0x78` for the
    /// level-6 autovector normally used by VBlank), the supervisor bit in `SR`
    /// is set, and execution jumps to the handler. An interrupt also clears the
    /// halted latch, so a halted processor resumes at the handler.
    ///
    /// The caller owns interrupt prioritisation: this slice models neither
    /// interrupt levels nor the `SR` interrupt mask, pending-request
    /// bookkeeping, automatic vector selection or the other exception groups.
    /// An odd or unmapped `vector_address`, a stack pointer without six bytes
    /// below it, or a vector entry holding an odd program counter is an error.
    /// The frame is pushed and the supervisor bit is set before the vector is
    /// fetched, like real hardware, so a bad vector leaves both behind.
    pub fn interrupt(
        &mut self,
        vector_address: u32,
        bus: &mut MegadriveBus,
    ) -> Result<(), CpuError> {
        let stack = self.a[7].checked_sub(6).ok_or(CpuError::StackOverflow)?;
        let return_pc = self.pc;
        let previous_sr = self.sr;

        bus.write16(stack, previous_sr)?;
        let return_pc_address = stack.checked_add(2).ok_or(CpuError::AddressOverflow)?;
        bus.write32(return_pc_address, return_pc)?;
        self.a[7] = stack;
        self.sr = previous_sr | SR_SUPERVISOR;

        let target = bus.read32(vector_address)?;
        self.write_pc(target)?;

        self.halted = false;
        Ok(())
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

    /// Enables the bounded instruction trace, replacing any existing trace.
    ///
    /// The trace is disabled by default. Every subsequent [`M68000::step`] that
    /// successfully fetches an opcode appends one record. Recording stops when
    /// the trace is disabled or taken.
    ///
    /// The trace is debug data: it does not affect registers, snapshots, the
    /// backend state hash or the snapshot payload version, and restoring a
    /// snapshot leaves it untouched. Enable it only on a machine whose state you
    /// are willing to inspect, not as part of canonical state.
    ///
    /// # Errors
    ///
    /// Returns [`TraceError::ZeroCapacity`] when `capacity` is zero. On error an
    /// already-enabled trace is left unchanged.
    pub fn enable_trace(&mut self, capacity: usize) -> Result<(), TraceError> {
        let trace = InstructionTrace::new(capacity)?;
        self.trace = Some(trace);
        Ok(())
    }

    /// Disables the instruction trace and discards its records.
    pub fn disable_trace(&mut self) {
        self.trace = None;
    }

    /// Clears the retained records and restarts indexes at zero.
    ///
    /// This keeps tracing enabled and preserves the configured capacity. It is a
    /// no-op when tracing is disabled.
    pub fn reset_trace(&mut self) {
        if let Some(trace) = &mut self.trace {
            trace.clear();
        }
    }

    /// Returns the active instruction trace, or `None` when tracing is disabled.
    #[must_use]
    pub const fn trace(&self) -> Option<&InstructionTrace> {
        self.trace.as_ref()
    }

    /// Removes and returns the instruction trace, disabling further recording.
    ///
    /// The returned trace keeps its records, capacity, dropped count and next
    /// index. Afterwards [`M68000::trace`] returns `None` until tracing is
    /// enabled again.
    pub fn take_trace(&mut self) -> Option<InstructionTrace> {
        self.trace.take()
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
    ///
    /// When the instruction trace is enabled, the fetched instruction is recorded
    /// before decoding, so an instruction that fails as illegal or unsupported is
    /// still present in the trace.
    pub fn step(&mut self, bus: &mut MegadriveBus) -> Result<(), CpuError> {
        if self.halted {
            return Err(CpuError::Halted);
        }
        let instruction_pc = self.pc;
        let opcode = self.fetch_u16(bus)?;
        if let Some(trace) = &mut self.trace {
            trace.record(instruction_pc, opcode);
        }

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

        // LEA (abs).L,An. The absolute-long encoding ends in 0xF9; the
        // absolute-word (0xF8) and PC-relative (0xFA/0xFB) forms are not
        // implemented and must fall through to the illegal-opcode error.
        if (opcode & 0xF1FF) == 0x41F9 {
            let register = usize::from((opcode >> 9) & 0x7);
            let address = self.fetch_u32(bus)?;
            self.a[register] = address;
            return Ok(());
        }

        // PEA (abs).L, whose full opcode is 0x4879. Do not accept the
        // absolute-word (0x4878) or PC-relative (0x487A) encodings.
        if opcode == 0x4879 {
            let address = self.fetch_u32(bus)?;
            self.push_long(bus, address)?;
            return Ok(());
        }

        // TST.B/W/L Dn
        if (opcode & 0xFFF8) == 0x4A00 {
            let register = usize::from(opcode & 0x7);
            let value = self.read_dn(register, Size::Byte);
            self.set_logic_flags(value, Size::Byte);
            return Ok(());
        }
        if (opcode & 0xFFF8) == 0x4A40 {
            let register = usize::from(opcode & 0x7);
            let value = self.read_dn(register, Size::Word);
            self.set_logic_flags(value, Size::Word);
            return Ok(());
        }
        if (opcode & 0xFFF8) == 0x4A80 {
            let register = usize::from(opcode & 0x7);
            let value = self.d[register];
            self.set_logic_flags(value, Size::Long);
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

        // MOVE.B/W/L (abs).L,Dn. The 68000 encodes source mode/register
        // 111/001 (absolute-long) as low byte 0x39. The absolute-word form
        // (111/000, low byte 0x38) and the PC-relative forms are not in this
        // subset and fall through to the illegal-opcode error rather than being
        // executed with a 32-bit fetch.
        if (opcode & 0xF1FF) == 0x1039 {
            let destination = usize::from((opcode >> 9) & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = u32::from(bus.read8(address)?);
            self.write_dn(destination, value, Size::Byte);
            self.set_logic_flags(value, Size::Byte);
            return Ok(());
        }
        if (opcode & 0xF1FF) == 0x2039 {
            let destination = usize::from((opcode >> 9) & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = bus.read32(address)?;
            self.d[destination] = value;
            self.set_logic_flags(value, Size::Long);
            return Ok(());
        }
        if (opcode & 0xF1FF) == 0x3039 {
            let destination = usize::from((opcode >> 9) & 0x7);
            let address = self.fetch_u32(bus)?;
            let value = u32::from(bus.read16(address)?);
            self.write_dn(destination, value, Size::Word);
            self.set_logic_flags(value, Size::Word);
            return Ok(());
        }

        // CMP.B/W/L Dn,Dn. As with ADD/SUB, the destination mode is bits 5-3
        // and the size is bits 7-6; bit 8 distinguishes CMP from EOR.
        if (opcode & 0xF100) == 0xB000 && (opcode & 0x0038) == 0 {
            let source = usize::from(opcode & 0x7);
            let destination = usize::from((opcode >> 9) & 0x7);
            let size = match opcode & 0x00C0 {
                0x0000 => Size::Byte,
                0x0040 => Size::Word,
                0x0080 => Size::Long,
                _ => {
                    return Err(CpuError::Unsupported(
                        "CMP only supports data-register operands".to_owned(),
                    ));
                }
            };
            let source_value = self.read_dn(source, size);
            let destination_value = self.read_dn(destination, size);
            self.set_cmp_flags(destination_value, source_value, size);
            return Ok(());
        }

        // DBRA Dn,disp16 (the DBcc encoding with condition 0x1).
        if (opcode & 0xFFF8) == 0x51C8 {
            let register = usize::from(opcode & 0x7);
            let displacement = self.fetch_u16(bus)? as i16;
            let counter = self.read_dn(register, Size::Word);
            let decremented = counter.wrapping_sub(1) & Size::Word.mask();
            self.write_dn(register, decremented, Size::Word);
            if decremented == 0xFFFF {
                return Ok(());
            }
            let target = self.relative_target(i32::from(displacement))?;
            return self.write_pc(target);
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
        let taken = match condition {
            // BRA, and BSR which also pushes a return address.
            0x0 | 0x1 => true,
            // BCC/BCS.
            0x4 => !self.flag(FLAG_C),
            0x5 => self.flag(FLAG_C),
            // BEQ/BNE.
            0x6 => !self.flag(FLAG_Z),
            0x7 => self.flag(FLAG_Z),
            // BMI/BPL.
            0xA => !self.flag(FLAG_N),
            0xB => self.flag(FLAG_N),
            _ => {
                return Err(CpuError::Unsupported(format!(
                    "branch condition 0x{condition:X} is not implemented"
                )));
            }
        };
        let target = if taken {
            Some(self.relative_target(displacement)?)
        } else {
            None
        };
        if condition == 0x1 {
            let return_pc = self.pc;
            self.push_long(bus, return_pc)?;
        }
        match target {
            Some(target) => self.write_pc(target),
            None => Ok(()),
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
        let (carry, overflow, zero, negative) =
            Self::subtract_condition_codes(destination, source, result, size);
        self.set_ccr(carry, overflow, zero, negative, carry);
    }

    /// Sets flags for `CMP`, which is a subtraction that leaves `X` untouched.
    fn set_cmp_flags(&mut self, destination: u32, source: u32, size: Size) {
        let mask = size.mask();
        let result = (destination & mask).wrapping_sub(source & mask) & mask;
        let (carry, overflow, zero, negative) =
            Self::subtract_condition_codes(destination, source, result, size);
        let extend = self.flag(FLAG_X);
        self.set_ccr(carry, overflow, zero, negative, extend);
    }

    fn subtract_condition_codes(
        destination: u32,
        source: u32,
        result: u32,
        size: Size,
    ) -> (bool, bool, bool, bool) {
        let mask = size.mask();
        let destination = destination & mask;
        let source = source & mask;
        let result = result & mask;
        let borrow = u64::from(destination) < u64::from(source);
        let overflow = ((destination ^ source) & (destination ^ result) & size.sign_bit()) != 0;
        (borrow, overflow, result == 0, result & size.sign_bit() != 0)
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

    use super::{CpuError, M68000};
    use crate::bus::{BusError, MegadriveBus};
    use crate::trace::TraceError;

    /// Stack top used by the new-instruction helpers. Work RAM on this bus ends
    /// at 0x01000000; starting near the top keeps downward pushes inside RAM.
    const INITIAL_STACK: u32 = 0x00FF_FF00;

    fn bus_with_vectors() -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&0x00FF_0000_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        MegadriveBus::new(Arc::from(rom.into_boxed_slice()))
    }

    /// Builds a bus whose reset vectors load `program` at address 0x200.
    fn program_bus(program: &[u8]) -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&INITIAL_STACK.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        rom[0x200..0x200 + program.len()].copy_from_slice(program);
        MegadriveBus::new(Arc::from(rom.into_boxed_slice()))
    }

    fn cpu_at_program(program: &[u8]) -> (M68000, MegadriveBus) {
        let mut bus = program_bus(program);
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).expect("reset");
        (cpu, bus)
    }

    /// Builds a bus with a NOP at 0x200 and `target` in the vector table.
    fn vector_bus(vector_address: u32, target: u32) -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&INITIAL_STACK.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        rom[0x200..0x202].copy_from_slice(&[0x4E, 0x71]);
        let start = vector_address as usize;
        rom[start..start + 4].copy_from_slice(&target.to_be_bytes());
        MegadriveBus::new(Arc::from(rom.into_boxed_slice()))
    }

    fn cpu_with_vector(vector_address: u32, target: u32) -> (M68000, MegadriveBus) {
        let mut bus = vector_bus(vector_address, target);
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).expect("reset");
        (cpu, bus)
    }

    fn run_branch(opcode: u16, sr: u16) -> u32 {
        let (mut cpu, mut bus) = cpu_at_program(&opcode.to_be_bytes());
        cpu.sr = sr;
        cpu.step(&mut bus).expect("conditional branch");
        cpu.pc()
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

    #[test]
    fn lea_abs_long_loads_an_address_register() {
        // LEA (0x12345678).L,A3
        let (mut cpu, mut bus) = cpu_at_program(&[0x47, 0xF9, 0x12, 0x34, 0x56, 0x78]);
        let flags = cpu.sr();
        cpu.step(&mut bus).expect("lea");
        assert_eq!(cpu.address_register(3), Some(0x1234_5678));
        assert_eq!(cpu.pc(), 0x206);
        assert_eq!(cpu.sr(), flags, "LEA must not change the condition codes");
    }

    #[test]
    fn pea_abs_long_pushes_the_address() {
        // PEA (0x00FF1000).L
        let (mut cpu, mut bus) = cpu_at_program(&[0x48, 0x79, 0x00, 0xFF, 0x10, 0x00]);
        let flags = cpu.sr();
        cpu.step(&mut bus).expect("pea");
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK - 4));
        assert_eq!(
            bus.read32(INITIAL_STACK - 4).expect("pushed address"),
            0x00FF_1000
        );
        assert_eq!(cpu.sr(), flags, "PEA must not change the condition codes");
    }

    #[test]
    fn absolute_word_and_pc_relative_forms_are_rejected_as_illegal() {
        // These encodings are LEA/PEA, but not absolute-long: the old masks
        // silently executed them with a 32-bit fetch. They must be errors.
        for (bytes, opcode) in [
            ([0x41, 0xF8], 0x41F8_u16), // LEA (abs).W,A0
            ([0x48, 0x78], 0x4878),     // PEA (abs).W
            ([0x41, 0xFA], 0x41FA),     // LEA (d16,PC),A0
            ([0x48, 0x7A], 0x487A),     // PEA (d16,PC)
        ] {
            let (mut cpu, mut bus) = cpu_at_program(&bytes);
            assert_eq!(
                cpu.step(&mut bus),
                Err(CpuError::IllegalOpcode { pc: 0x200, opcode }),
                "opcode 0x{opcode:04X} must not be executed as absolute-long"
            );
            assert_eq!(cpu.pc(), 0x202, "only the opcode word is fetched");
            assert_eq!(cpu.address_register(7), Some(INITIAL_STACK));
            assert_eq!(cpu.sr(), 0x2700);
        }
    }

    #[test]
    fn tst_byte_sets_negative_and_preserves_extend() {
        // TST.B D0 with the byte sign bit set; X already set and must survive.
        let (mut cpu, mut bus) = cpu_at_program(&[0x4A, 0x00]);
        cpu.d[0] = 0xFFFF_FF80;
        cpu.sr = 0x2710;
        cpu.step(&mut bus).expect("tst.b");
        assert_eq!(cpu.sr(), 0x2718, "N set, Z/C/V clear, X preserved");
        assert_eq!(cpu.data_register(0), Some(0xFFFF_FF80));
    }

    #[test]
    fn tst_word_zero_sets_zero_and_clears_carry_and_overflow() {
        // TST.W D1: only the low word decides the flags.
        let (mut cpu, mut bus) = cpu_at_program(&[0x4A, 0x41]);
        cpu.d[1] = 0x0001_0000;
        cpu.sr = 0x2703;
        cpu.step(&mut bus).expect("tst.w");
        assert_eq!(cpu.sr(), 0x2704, "Z set, C/V/N clear, X stays clear");
    }

    #[test]
    fn tst_long_uses_all_32_bits() {
        // TST.L D2: a sign bit outside the low word must set N.
        let (mut cpu, mut bus) = cpu_at_program(&[0x4A, 0x82]);
        cpu.d[2] = 0x8000_0000;
        cpu.step(&mut bus).expect("tst.l");
        assert_eq!(cpu.sr(), 0x2708);
    }

    #[test]
    fn cmp_byte_sets_borrow_and_negative_without_changing_extend() {
        // CMP.B D1,D0 compares 0x10 with 0x20: 0x10 - 0x20 = 0xF0.
        let (mut cpu, mut bus) = cpu_at_program(&[0xB0, 0x01]);
        cpu.d[0] = 0x10;
        cpu.d[1] = 0x20;
        cpu.sr = 0x2710;
        cpu.step(&mut bus).expect("cmp.b");
        assert_eq!(cpu.sr(), 0x2719, "N and C set, Z/V clear, X preserved");
        assert_eq!(cpu.data_register(0), Some(0x10));
        assert_eq!(cpu.data_register(1), Some(0x20));
    }

    #[test]
    fn cmp_word_equal_sets_zero_without_carry() {
        // CMP.W D1,D0: 0x1234 - 0x1234 = 0.
        let (mut cpu, mut bus) = cpu_at_program(&[0xB0, 0x41]);
        cpu.d[0] = 0x1234;
        cpu.d[1] = 0x1234;
        cpu.step(&mut bus).expect("cmp.w");
        assert_eq!(cpu.sr(), 0x2704, "Z set, N/C/V clear");
    }

    #[test]
    fn cmp_long_detects_signed_overflow() {
        // CMP.L D1,D0: 0x80000000 - 1 overflows into 0x7FFFFFFF.
        let (mut cpu, mut bus) = cpu_at_program(&[0xB0, 0x81]);
        cpu.d[0] = 0x8000_0000;
        cpu.d[1] = 1;
        cpu.step(&mut bus).expect("cmp.l");
        assert_eq!(cpu.sr(), 0x2702, "V set, N/C/Z clear");
        assert_eq!(cpu.data_register(0), Some(0x8000_0000));
    }

    #[test]
    fn dbra_loops_until_the_counter_wraps_to_minus_one() {
        // MOVEQ #2,D0; DBRA D0,-4; NOP
        let (mut cpu, mut bus) = cpu_at_program(&[0x70, 0x02, 0x51, 0xC8, 0xFF, 0xFC, 0x4E, 0x71]);
        cpu.step(&mut bus).expect("moveq");
        let flags_after_moveq = cpu.sr();
        let mut iterations = 0;
        while cpu.pc() != 0x206 {
            cpu.step(&mut bus).expect("dbra");
            iterations += 1;
            assert!(iterations <= 4, "DBRA did not terminate");
        }
        assert_eq!(iterations, 3, "counter 2, 1, 0 then 0xFFFF");
        assert_eq!(cpu.data_register(0), Some(0xFFFF));
        assert_eq!(cpu.sr(), flags_after_moveq, "DBRA must not change flags");
        assert_eq!(cpu.pc(), 0x206);
    }

    #[test]
    fn dbra_odd_target_is_an_address_error() {
        // DBRA D0,-3 targets 0x201 after the two displacement bytes.
        let (mut cpu, mut bus) = cpu_at_program(&[0x51, 0xC8, 0xFF, 0xFD]);
        cpu.d[0] = 2;
        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::AddressError { address: 0x201 })
        );
    }

    #[test]
    fn bcc_branches_when_carry_is_clear() {
        assert_eq!(run_branch(0x6402, 0x2700), 0x204);
        assert_eq!(run_branch(0x6402, 0x2701), 0x202);
    }

    #[test]
    fn bcs_branches_when_carry_is_set() {
        assert_eq!(run_branch(0x6502, 0x2701), 0x204);
        assert_eq!(run_branch(0x6502, 0x2700), 0x202);
    }

    #[test]
    fn bmi_branches_when_negative_is_set() {
        assert_eq!(run_branch(0x6B02, 0x2708), 0x204);
        assert_eq!(run_branch(0x6B02, 0x2700), 0x202);
    }

    #[test]
    fn bpl_branches_when_negative_is_clear() {
        assert_eq!(run_branch(0x6A02, 0x2700), 0x204);
        assert_eq!(run_branch(0x6A02, 0x2708), 0x202);
    }

    #[test]
    fn interrupt_pushes_frame_sets_supervisor_and_jumps_through_vector() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.step(&mut bus).expect("nop");
        let previous_sr = cpu.sr();
        cpu.interrupt(0x78, &mut bus).expect("interrupt");
        assert_eq!(cpu.pc(), 0x300);
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK - 6));
        assert_eq!(
            bus.read16(INITIAL_STACK - 6).expect("pushed sr"),
            previous_sr
        );
        assert_eq!(bus.read32(INITIAL_STACK - 4).expect("pushed pc"), 0x202);
        assert!(cpu.sr() & 0x2000 != 0, "entry must be in supervisor mode");
        assert!(!cpu.halted());
    }

    #[test]
    fn interrupt_enters_supervisor_mode_and_preserves_condition_codes() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.sr = 0x0004; // User mode with Z set.
        cpu.interrupt(0x78, &mut bus).expect("interrupt");
        assert_eq!(bus.read16(INITIAL_STACK - 6).expect("pushed sr"), 0x0004);
        assert_eq!(bus.read32(INITIAL_STACK - 4).expect("pushed pc"), 0x200);
        assert_eq!(cpu.sr(), 0x2004);
    }

    #[test]
    fn interrupt_wakes_a_halted_processor() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.halted = true;
        cpu.interrupt(0x78, &mut bus).expect("interrupt");
        assert!(!cpu.halted());
        assert_eq!(cpu.pc(), 0x300);
    }

    #[test]
    fn interrupt_rejects_stack_underflow() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.a[7] = 0;
        assert_eq!(cpu.interrupt(0x78, &mut bus), Err(CpuError::StackOverflow));
        assert_eq!(cpu.pc(), 0x200);
    }

    #[test]
    fn interrupt_rejects_a_rom_stack() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.a[7] = 0x0000_0300;
        cpu.sr = 0x0000;
        assert_eq!(
            cpu.interrupt(0x78, &mut bus),
            Err(CpuError::Bus(BusError::WriteProtected {
                address: 0x0000_02FA
            }))
        );
        assert_eq!(cpu.sr(), 0x0000, "a failed first push leaves SR untouched");
        assert_eq!(cpu.pc(), 0x200);
    }

    #[test]
    fn interrupt_rejects_an_unmapped_vector() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        assert_eq!(
            cpu.interrupt(0x0080_0000, &mut bus),
            Err(CpuError::Bus(BusError::UnmappedRead {
                address: 0x0080_0000
            }))
        );
        // The exception frame is pushed and supervisor mode entered before the
        // vector fetch fails.
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK - 6));
        assert_eq!(cpu.pc(), 0x200);
        assert_ne!(cpu.sr() & 0x2000, 0);
    }

    #[test]
    fn interrupt_rejects_an_odd_vector_address() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        assert_eq!(
            cpu.interrupt(0x79, &mut bus),
            Err(CpuError::Bus(BusError::OddAddress { address: 0x79 }))
        );
        assert_ne!(cpu.sr() & 0x2000, 0);
    }

    #[test]
    fn interrupt_rejects_an_odd_target_pc() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0301);
        cpu.sr = 0x0000;
        assert_eq!(
            cpu.interrupt(0x78, &mut bus),
            Err(CpuError::AddressError { address: 0x301 })
        );
        assert_eq!(cpu.pc(), 0x200);
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK - 6));
        assert_eq!(
            cpu.sr(),
            0x2000,
            "supervisor mode is entered before the jump"
        );
    }

    #[test]
    fn trace_records_the_exact_executed_sequence() {
        // MOVEQ #1,D0; MOVEQ #2,D1; ADD.W D1,D0; NOP
        let (mut cpu, mut bus) = cpu_at_program(&[0x70, 0x01, 0x72, 0x02, 0xD0, 0x41, 0x4E, 0x71]);
        assert!(cpu.trace().is_none(), "tracing is disabled by default");
        cpu.enable_trace(8).expect("trace capacity 8");
        for _ in 0..4 {
            cpu.step(&mut bus).expect("step");
        }
        let trace = cpu.trace().expect("trace enabled");
        assert_eq!(trace.capacity(), 8);
        assert_eq!(trace.len(), 4);
        assert_eq!(trace.dropped(), 0);
        assert_eq!(trace.first_index(), Some(0));
        assert_eq!(trace.next_index(), 4);
        let records: Vec<(u64, u32, u16)> = trace
            .entries()
            .map(|entry| (entry.index(), entry.pc(), entry.opcode()))
            .collect();
        assert_eq!(
            records,
            vec![
                (0, 0x200, 0x7001),
                (1, 0x202, 0x7202),
                (2, 0x204, 0xD041),
                (3, 0x206, 0x4E71),
            ]
        );
    }

    #[test]
    fn disabled_trace_records_nothing_and_take_disables_it() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x71, 0x4E, 0x71, 0x4E, 0x71]);
        cpu.step(&mut bus).expect("step without trace");
        assert!(cpu.trace().is_none());

        cpu.enable_trace(4).expect("trace capacity 4");
        cpu.step(&mut bus).expect("step with trace");
        let taken = cpu.take_trace().expect("trace was enabled");
        assert_eq!(taken.len(), 1);
        assert_eq!(taken.get(0).map(|entry| entry.pc()), Some(0x202));
        assert!(cpu.trace().is_none(), "take disables further recording");

        cpu.step(&mut bus).expect("step after take");
        assert!(cpu.trace().is_none());
    }

    #[test]
    fn trace_ring_is_bounded_and_evicts_oldest_records() {
        // Four NOPs: one opcode per word.
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x71, 0x4E, 0x71, 0x4E, 0x71, 0x4E, 0x71]);
        cpu.enable_trace(2).expect("trace capacity 2");
        for _ in 0..4 {
            cpu.step(&mut bus).expect("step");
        }
        let trace = cpu.trace().expect("trace enabled");
        assert_eq!(trace.capacity(), 2);
        assert_eq!(trace.len(), 2);
        assert_eq!(trace.dropped(), 2);
        assert_eq!(trace.first_index(), Some(2));
        assert_eq!(trace.next_index(), 4);
        let records: Vec<(u64, u32)> = trace
            .entries()
            .map(|entry| (entry.index(), entry.pc()))
            .collect();
        assert_eq!(records, vec![(2, 0x204), (3, 0x206)]);
    }

    #[test]
    fn trace_still_records_an_instruction_that_fails_after_the_fetch() {
        let (mut cpu, mut bus) = cpu_at_program(&[0xFF, 0xFF]);
        cpu.enable_trace(4).expect("trace capacity 4");
        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::IllegalOpcode {
                pc: 0x200,
                opcode: 0xFFFF
            })
        );
        let trace = cpu.trace().expect("trace enabled");
        assert_eq!(trace.len(), 1);
        let entry = trace.get(0).expect("recorded instruction");
        assert_eq!(
            (entry.index(), entry.pc(), entry.opcode()),
            (0, 0x200, 0xFFFF)
        );
    }

    #[test]
    fn reset_trace_keeps_tracing_enabled_and_restarts_indexes() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x71, 0x4E, 0x71]);
        cpu.enable_trace(4).expect("trace capacity 4");
        cpu.step(&mut bus).expect("step");
        cpu.reset_trace();
        let trace = cpu.trace().expect("trace remains enabled");
        assert!(trace.is_empty());
        assert_eq!(trace.next_index(), 0);
        assert_eq!(trace.capacity(), 4);
        cpu.step(&mut bus).expect("step after reset");
        let entry = cpu.trace().expect("trace enabled").get(0).expect("record");
        assert_eq!((entry.index(), entry.pc()), (0, 0x202));
    }

    #[test]
    fn zero_capacity_is_rejected_without_replacing_an_enabled_trace() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x71]);
        assert_eq!(
            cpu.enable_trace(0),
            Err(TraceError::ZeroCapacity),
            "capacity 0 must be a typed error"
        );
        assert!(cpu.trace().is_none());

        cpu.enable_trace(4).expect("trace capacity 4");
        cpu.step(&mut bus).expect("step");
        assert_eq!(cpu.enable_trace(0), Err(TraceError::ZeroCapacity));
        assert_eq!(
            cpu.trace().expect("trace still enabled").len(),
            1,
            "a rejected reconfiguration must not disturb the active trace"
        );
    }

    #[test]
    fn restore_state_does_not_restore_trace_contents() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x71, 0x4E, 0x71, 0x4E, 0x71, 0x4E, 0x71]);
        cpu.enable_trace(8).expect("trace capacity 8");
        cpu.step(&mut bus).expect("step");
        cpu.step(&mut bus).expect("step");
        let checkpoint = cpu.snapshot_state();
        cpu.step(&mut bus).expect("step");
        cpu.step(&mut bus).expect("step");
        assert_eq!(cpu.trace().expect("trace").len(), 4);

        cpu.restore_state(
            checkpoint.0,
            checkpoint.1,
            checkpoint.2,
            checkpoint.3,
            checkpoint.4,
        );
        let trace = cpu.trace().expect("trace survives restore");
        assert_eq!(trace.len(), 4, "restore must not rewind trace contents");
        assert_eq!(trace.next_index(), 4);
        assert_eq!(trace.get(3).map(|entry| entry.pc()), Some(0x206));
    }

    #[test]
    fn move_abs_long_load_forms_read_byte_word_and_long_from_ram() {
        // MOVE.L (0x00FF0000).L,D0; MOVE.W (0x00FF0004).L,D1;
        // MOVE.B (0x00FF0008).L,D2. Source mode/register 111/001 -> low byte 0x39.
        let (mut cpu, mut bus) = cpu_at_program(&[
            0x20, 0x39, 0x00, 0xFF, 0x00, 0x00, //
            0x32, 0x39, 0x00, 0xFF, 0x00, 0x04, //
            0x14, 0x39, 0x00, 0xFF, 0x00, 0x08,
        ]);
        bus.write32(0x00FF_0000, 0x1122_3344).expect("ram long");
        bus.write16(0x00FF_0004, 0xABCD).expect("ram word");
        bus.write8(0x00FF_0008, 0x7F).expect("ram byte");
        cpu.d[1] = 0xAAAA_0000;
        cpu.d[2] = 0xBBBB_BB00;

        cpu.step(&mut bus).expect("move.l (abs).l,d0");
        assert_eq!(cpu.data_register(0), Some(0x1122_3344));
        assert_eq!(cpu.sr(), 0x2700, "positive long load: N/Z/C/V clear");
        cpu.step(&mut bus).expect("move.w (abs).l,d1");
        assert_eq!(
            cpu.data_register(1),
            Some(0xAAAA_ABCD),
            "word load must preserve the upper word"
        );
        assert_eq!(cpu.sr(), 0x2708, "negative word load sets N");
        cpu.step(&mut bus).expect("move.b (abs).l,d2");
        assert_eq!(
            cpu.data_register(2),
            Some(0xBBBB_BB7F),
            "byte load must preserve the upper bytes"
        );
        assert_eq!(cpu.sr(), 0x2700);
        assert_eq!(cpu.pc(), 0x212);
    }

    #[test]
    fn move_abs_word_load_encodings_are_illegal() {
        // 0x..38 is source mode/register 111/000 (absolute-word), which is not in
        // the subset; it must not be executed with a 32-bit address fetch.
        for opcode in [0x1038_u16, 0x1238, 0x2038, 0x2438, 0x3038, 0x3638] {
            let (mut cpu, mut bus) = cpu_at_program(&opcode.to_be_bytes());
            assert_eq!(
                cpu.step(&mut bus),
                Err(CpuError::IllegalOpcode { pc: 0x200, opcode }),
                "opcode 0x{opcode:04X} must be an illegal absolute-word form"
            );
            assert_eq!(cpu.pc(), 0x202, "only the opcode word is fetched");
            assert_eq!(cpu.sr(), 0x2700);
            assert_eq!(cpu.address_register(7), Some(INITIAL_STACK));
        }
    }

    #[test]
    fn move_abs_long_store_encodings_still_write_byte_word_and_long() {
        // MOVE.B D0,(0x00FF0000).L; MOVE.W D1,(0x00FF0002).L;
        // MOVE.L D2,(0x00FF0004).L. Destination mode/register 111/001.
        let (mut cpu, mut bus) = cpu_at_program(&[
            0x13, 0xC0, 0x00, 0xFF, 0x00, 0x00, //
            0x33, 0xC1, 0x00, 0xFF, 0x00, 0x02, //
            0x23, 0xC2, 0x00, 0xFF, 0x00, 0x04,
        ]);
        cpu.d[0] = 0x1234_56A1;
        cpu.d[1] = 0xCCCC_BEEF;
        cpu.d[2] = 0xDEAD_BEEF;

        cpu.step(&mut bus).expect("move.b dn,(abs).l");
        cpu.step(&mut bus).expect("move.w dn,(abs).l");
        cpu.step(&mut bus).expect("move.l dn,(abs).l");

        assert_eq!(bus.read8(0x00FF_0000), Ok(0xA1));
        assert_eq!(bus.read16(0x00FF_0002), Ok(0xBEEF));
        assert_eq!(bus.read32(0x00FF_0004), Ok(0xDEAD_BEEF));
        assert_eq!(cpu.pc(), 0x212);
        assert_eq!(cpu.sr(), 0x2708, "the final long store sets N");
    }
}
