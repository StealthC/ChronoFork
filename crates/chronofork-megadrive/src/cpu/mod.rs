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
//! - interrupt-priority-mask gating through [`interrupt_level_deliverable`]: an
//!   interrupt at level 7 is always taken, and every other level is taken only
//!   when it is strictly greater than the `SR` interrupt-priority mask. The
//!   CPU exposes the same rule as [`M68000::accepts_interrupt_level`];
//! - interrupt exception entry ([`M68000::interrupt`]) at an explicit accepted
//!   level: the previous raw `SR` and return PC are pushed, `S` is set, the CCR
//!   is preserved and the interrupt-priority mask becomes the accepted level;
//!   `RTE` (0x4E73) restores the raw `SR` and program counter from that frame
//!   and advances `A7` by six;
//! - `MOVE.W #imm,SR` (0x46FC), so a program can change its interrupt mask;
//! - an optional bounded instruction trace (see [`crate::trace`]) recording each
//!   fetched address/opcode pair. It is disabled by default and is debug data:
//!   it is not part of snapshots, state hashes or registers.
//!
//! The register file carries separate user and supervisor stack pointers
//! (`USP`/`SSP`) selected by `SR` bit 13; the instruction paths in this slice
//! do not yet select between them.
//!
//! Not implemented: effective-address modes beyond the listed forms, automatic
//! interrupt-vector selection (the caller supplies the vector address), other
//! exception groups, privilege enforcement (`RTE` and `MOVE.W #imm,SR` execute
//! in either mode here), automatic stack switching on exception entry and
//! `RTE`, prefetch, cycle-accurate timing, and the rest of the instruction set.
//! Unsupported opcodes are errors, never silently skipped.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::bus::{BusError, MegadriveBus};
use crate::trace::{InstructionTrace, TraceError};

pub mod decode;
pub mod ea;
pub mod exec;

use decode::InstructionClass;

pub use ea::{Ea, Size, decode_ea, ea_address, read_ea, write_ea};

pub(crate) const FLAG_C: u16 = 0x0001;
pub(crate) const FLAG_V: u16 = 0x0002;
pub(crate) const FLAG_Z: u16 = 0x0004;
pub(crate) const FLAG_N: u16 = 0x0008;
pub(crate) const FLAG_X: u16 = 0x0010;
const SR_SUPERVISOR: u16 = 0x2000;
const SR_INTERRUPT_MASK: u16 = 0x0700;
const RESET_SR: u16 = SR_SUPERVISOR | SR_INTERRUPT_MASK;

/// Returns the 68000 interrupt-priority mask (IPM) encoded in
/// `status_register` (bits 10-8).
#[must_use]
pub const fn interrupt_priority_mask(status_register: u16) -> u8 {
    ((status_register & SR_INTERRUPT_MASK) >> 8) as u8
}

/// Returns whether the 68000 takes an interrupt request at `level` while the
/// status register holds `status_register`.
///
/// This is the single source of truth for the 68000 delivery rule in this
/// slice: level 7 is always taken, and every other level is taken only when it
/// is strictly greater than the interrupt-priority mask,
/// `(status_register >> 8) & 7`. Only the 68000 interrupt levels 1..=7 are
/// valid inputs: level 0 is the 68000's "no request" level and values above 7
/// do not exist, so both are never deliverable. The rule is independent of the
/// `S` bit and of the condition codes.
///
/// The backend gates delivery through [`M68000::accepts_interrupt_level`] and
/// the scheduler through
/// [`InterruptScheduler::next_deliverable`](crate::interrupts::InterruptScheduler::next_deliverable),
/// so both apply exactly this predicate.
#[must_use]
pub const fn interrupt_level_deliverable(level: u8, status_register: u16) -> bool {
    matches!(level, 1..=7) && (level == 7 || level > interrupt_priority_mask(status_register))
}

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
    /// The requested interrupt level is not a 68000 interrupt level.
    InvalidInterruptLevel {
        /// Rejected level.
        level: u8,
    },
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
            Self::InvalidInterruptLevel { level } => {
                write!(
                    f,
                    "68000 interrupt level {level} is invalid (expected 1..=7)"
                )
            }
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

/// 68000 architectural state for the implemented slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct M68000 {
    d: [u32; 8],
    a: [u32; 8],
    pc: u32,
    sr: u16,
    /// User stack pointer (`USP`), active while `SR` bit 13 is clear.
    usp: u32,
    /// Supervisor stack pointer (`SSP`), active while `SR` bit 13 is set.
    ssp: u32,
    halted: bool,
    /// Set when a `STOP` instruction suspends the processor.
    stopped: bool,
    /// 68000 cycle accumulator for the instruction timing layer.
    cycles: u64,
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
            usp: 0,
            ssp: 0,
            halted: false,
            stopped: false,
            cycles: 0,
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
        self.ssp = self.a[7];
        self.pc = bus.read32(4)?;
        self.sr = RESET_SR;
        self.usp = 0;
        self.halted = false;
        self.stopped = false;
        self.cycles = 0;
        self.ensure_even_pc()
    }

    /// Enters a 68000 interrupt exception at `level` through one vector-table
    /// entry.
    ///
    /// `level` must be an accepted 68000 interrupt level (1..=7); 0, which means
    /// "no request", and values above 7 are rejected with
    /// [`CpuError::InvalidInterruptLevel`] before any state changes.
    ///
    /// The processor builds its exception frame on the (supervisor) stack: a
    /// word containing the previous raw status register at the lowest address
    /// and a long containing the return program counter immediately above it, so
    /// the stack pointer decreases by six. It then sets the supervisor bit, keeps
    /// every CCR bit and loads the accepted level into the interrupt-priority
    /// mask, `SR = (previous & !0x0700) | (level << 8) | S`, so the same level
    /// and lower levels stay blocked until `RTE` restores the frame.
    ///
    /// The target program counter is then read as a big-endian long from
    /// `vector_address` (for example `0x78` for the level-6 autovector normally
    /// used by VBlank and `0x7C` for level 7), and execution jumps to the
    /// handler. An interrupt also clears the halted latch, so a halted processor
    /// resumes at the handler.
    ///
    /// The caller owns interrupt selection: this method performs the exception
    /// entry it is given and does not itself decide whether a request may be
    /// taken. Use [`M68000::accepts_interrupt_level`] (or the scheduler's
    /// [`next_deliverable`](crate::interrupts::InterruptScheduler::next_deliverable))
    /// to apply the 68000 priority-mask rule before calling this method.
    /// Automatic vector selection, pending-request bookkeeping and the other
    /// exception groups are not modelled.
    /// An odd or unmapped `vector_address`, a stack pointer without six bytes
    /// below it, or a vector entry holding an odd program counter is an error.
    /// The frame is pushed and the new `SR` applied before the vector is fetched,
    /// like real hardware, so a bad vector leaves both behind.
    pub fn interrupt(
        &mut self,
        level: u8,
        vector_address: u32,
        bus: &mut MegadriveBus,
    ) -> Result<(), CpuError> {
        if !(1..=7).contains(&level) {
            return Err(CpuError::InvalidInterruptLevel { level });
        }

        let stack = self.a[7].checked_sub(6).ok_or(CpuError::StackOverflow)?;
        let return_pc = self.pc;
        let previous_sr = self.sr;

        bus.write16(stack, previous_sr)?;
        let return_pc_address = stack.checked_add(2).ok_or(CpuError::AddressOverflow)?;
        bus.write32(return_pc_address, return_pc)?;
        self.a[7] = stack;
        self.sr = (previous_sr & !SR_INTERRUPT_MASK) | (u16::from(level) << 8) | SR_SUPERVISOR;

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

    /// Replaces the status register and selects the matching active stack.
    ///
    /// This is [`M68000::apply_sr`]: the current stack pointer is saved and,
    /// when the new value changes the supervisor bit, the other stack pointer
    /// becomes active.
    pub fn set_sr(&mut self, value: u16) {
        self.apply_sr(value);
    }

    /// Writes the status register, preserving the inactive stack pointer.
    ///
    /// `SR` bit 13 (`S`) selects which saved stack pointer is the live `A7`:
    /// with `S` set, `A7` is the supervisor stack pointer, and with `S` clear
    /// it is the user stack pointer. The current `A7` is first saved into the
    /// stack it belongs to; when the write changes `S`, the other saved stack
    /// pointer is loaded into `A7`, so a mode switch never loses a stack.
    pub fn apply_sr(&mut self, value: u16) {
        let was_supervisor = self.sr & SR_SUPERVISOR != 0;
        if was_supervisor {
            self.ssp = self.a[7];
        } else {
            self.usp = self.a[7];
        }
        self.sr = value;
        let is_supervisor = value & SR_SUPERVISOR != 0;
        if is_supervisor != was_supervisor {
            self.a[7] = if is_supervisor { self.ssp } else { self.usp };
        }
    }

    /// Returns the user stack pointer (`USP`).
    ///
    /// `USP` is active as `A7` while `SR` bit 13 is clear; while the processor
    /// is in supervisor mode it holds the saved user stack.
    #[must_use]
    pub const fn usp(&self) -> u32 {
        self.usp
    }

    /// Returns the supervisor stack pointer (`SSP`).
    ///
    /// `SSP` is active as `A7` while `SR` bit 13 is set (the reset state);
    /// while the processor is in user mode it holds the saved supervisor stack.
    #[must_use]
    pub const fn ssp(&self) -> u32 {
        self.ssp
    }

    /// Returns the active address register 7, the current stack pointer.
    ///
    /// This is `SSP` in supervisor mode and `USP` in user mode.
    #[must_use]
    pub const fn a7(&self) -> u32 {
        self.a[7]
    }

    /// Sets the active address register 7, the current stack pointer.
    ///
    /// The value is written to `A7` and mirrored into the stack pointer that
    /// `SR` bit 13 selects, so [`M68000::usp`] or [`M68000::ssp`] reports it
    /// immediately. The inactive stack is left unchanged.
    pub fn set_a7(&mut self, value: u32) {
        self.a[7] = value;
        if self.sr & SR_SUPERVISOR != 0 {
            self.ssp = value;
        } else {
            self.usp = value;
        }
    }

    /// Returns the accumulated 68000 cycle count.
    ///
    /// The value is accounted by the instruction timing layer; it is zero
    /// until per-instruction cycle accounting is added.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Returns whether a `STOP` instruction has suspended the processor.
    ///
    /// No instruction sets this yet; it exists so the register file carries
    /// the complete architectural state from the start of the CPU split.
    #[must_use]
    pub const fn stopped(&self) -> bool {
        self.stopped
    }

    /// Returns the interrupt-priority mask currently encoded in `SR`.
    #[must_use]
    pub const fn interrupt_priority_mask(&self) -> u8 {
        interrupt_priority_mask(self.sr)
    }

    /// Returns whether this processor would take an interrupt request at `level`
    /// in its current state.
    ///
    /// This applies [`interrupt_level_deliverable`] to the raw current `SR`: the
    /// level is 7 (always taken) or strictly greater than
    /// [`M68000::interrupt_priority_mask`]. It does not consume or acknowledge
    /// any request; callers still own pending bookkeeping and vector selection.
    #[must_use]
    pub const fn accepts_interrupt_level(&self, level: u8) -> bool {
        interrupt_level_deliverable(level, self.sr)
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

    /// Writes a data register.
    ///
    /// The register index is masked to `0..=7`, matching the internal data
    /// register helpers; the 68000 has exactly eight data registers.
    pub fn set_d(&mut self, register: usize, value: u32) {
        self.d[register & 0x7] = value;
    }

    /// Writes an address register.
    ///
    /// The register index is masked to `0..=7`. Writing `A7` goes through
    /// [`M68000::set_a7`], so the active supervisor or user stack pointer is
    /// updated as well.
    pub fn set_a(&mut self, register: usize, value: u32) {
        if register & 0x7 == 7 {
            self.set_a7(value);
        } else {
            self.a[register & 0x7] = value;
        }
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
        // The snapshot layout does not carry USP/SSP yet; keep the mirror for
        // the restored mode consistent with the restored active A7.
        if sr & SR_SUPERVISOR != 0 {
            self.ssp = a[7];
        } else {
            self.usp = a[7];
        }
        self.halted = halted;
    }

    /// Executes one instruction.
    ///
    /// When the instruction trace is enabled, the fetched instruction is recorded
    /// before decoding, so an instruction that fails as illegal or unsupported is
    /// still present in the trace. The opcode is classified exactly once by
    /// [`decode::classify`]; the selected execution group performs the operation.
    pub fn step(&mut self, bus: &mut MegadriveBus) -> Result<(), CpuError> {
        if self.halted {
            return Err(CpuError::Halted);
        }
        let instruction_pc = self.pc;
        let opcode = self.fetch_u16(bus)?;
        if let Some(trace) = &mut self.trace {
            trace.record(instruction_pc, opcode);
        }

        match decode::classify(opcode) {
            InstructionClass::Move
            | InstructionClass::Moveq
            | InstructionClass::Movem
            | InstructionClass::Movep
            | InstructionClass::Lea
            | InstructionClass::Pea
            | InstructionClass::Clr
            | InstructionClass::Tst
            | InstructionClass::Ext
            | InstructionClass::Swap => {
                exec::r#move::execute(self, bus, opcode)?;
            }
            InstructionClass::Branch => return self.execute_branch(bus, opcode),
            InstructionClass::Nop => {}
            InstructionClass::Rte => return self.execute_rte(bus),
            InstructionClass::Rts => {
                let pc = self.pop_long(bus)?;
                return self.write_pc(pc);
            }
            InstructionClass::Jmp => {
                let target = self.fetch_u32(bus)?;
                return self.write_pc(target);
            }
            InstructionClass::Jsr => {
                let target = self.fetch_u32(bus)?;
                self.ensure_even(target)?;
                let return_pc = self.pc;
                self.push_long(bus, return_pc)?;
                self.pc = target;
            }
            InstructionClass::MoveToSr => {
                // Privileged on real hardware; privilege is not enforced yet.
                // The immediate word replaces the raw SR, including the mask.
                let value = self.fetch_u16(bus)?;
                self.sr = value;
            }
            InstructionClass::Dbra => return self.execute_dbra(bus, opcode),
            InstructionClass::And
            | InstructionClass::Andi
            | InstructionClass::Or
            | InstructionClass::Ori
            | InstructionClass::Eor
            | InstructionClass::Eori
            | InstructionClass::Not => {
                exec::logic::execute(self, bus, opcode)?;
            }
            InstructionClass::Add
            | InstructionClass::Adda
            | InstructionClass::Addq
            | InstructionClass::Addi
            | InstructionClass::Sub
            | InstructionClass::Suba
            | InstructionClass::Subq
            | InstructionClass::Subi
            | InstructionClass::Cmp
            | InstructionClass::Cmpa
            | InstructionClass::Cmpi
            | InstructionClass::Cmpm
            | InstructionClass::Neg
            | InstructionClass::Negx => {
                exec::arith::execute(self, bus, opcode)?;
            }
            _ => {
                return Err(CpuError::IllegalOpcode {
                    pc: instruction_pc,
                    opcode,
                });
            }
        }
        Ok(())
    }

    /// Executes `DBRA Dn,disp16` (the `DBcc` encoding with condition 1).
    fn execute_dbra(&mut self, bus: &mut MegadriveBus, opcode: u16) -> Result<(), CpuError> {
        let register = usize::from(opcode & 0x7);
        let displacement = self.fetch_u16(bus)? as i16;
        let counter = self.read_dn(register, Size::Word);
        let decremented = counter.wrapping_sub(1) & Size::Word.mask();
        self.write_dn(register, decremented, Size::Word);
        if decremented == 0xFFFF {
            return Ok(());
        }
        let target = self.relative_target(i32::from(displacement))?;
        self.write_pc(target)
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

    /// Executes `RTE` (0x4E73).
    ///
    /// The exception frame lives on the supervisor stack: the raw status
    /// register word at `A7` and the long return program counter at `A7 + 2`.
    /// The complete frame is read and validated before any canonical state is
    /// changed, so an underflowing stack pointer, a bus error or an odd return
    /// program counter leaves `SR`, `PC` (beyond the normal opcode fetch) and
    /// `A7` untouched. On success the exact raw `SR` word is restored (IPM, `S`
    /// and CCR, with no implicit supervisor re-entry), the program counter is
    /// set to the restored value, and `A7` advances by six.
    fn execute_rte(&mut self, bus: &mut MegadriveBus) -> Result<(), CpuError> {
        let frame = self.a[7];
        let return_pc_address = frame.checked_add(2).ok_or(CpuError::StackOverflow)?;
        let restored_a7 = frame.checked_add(6).ok_or(CpuError::StackOverflow)?;

        let restored_sr = bus.read16(frame)?;
        let restored_pc = bus.read32(return_pc_address)?;
        self.ensure_even(restored_pc)?;

        self.sr = restored_sr;
        self.pc = restored_pc;
        self.a[7] = restored_a7;
        Ok(())
    }

    /// Computes a branch target from the current program counter and a signed
    /// displacement.
    ///
    /// A negative result and a result above `u32::MAX` are
    /// [`CpuError::AddressOverflow`]. The wide bounds check below is the single
    /// proof of the `u32` range, so the narrowing conversion that follows is
    /// lossless and has no error path of its own.
    fn relative_target(&self, displacement: i32) -> Result<u32, CpuError> {
        let target = i64::from(self.pc) + i64::from(displacement);
        if target < 0 || target > i64::from(u32::MAX) {
            return Err(CpuError::AddressOverflow);
        }
        // Proven by the check above: `0 <= target <= u32::MAX`.
        let target = target as u32;
        self.ensure_even(target)?;
        Ok(target)
    }

    /// Reads data register `register`, narrowed to `size`.
    ///
    /// The 68000 has exactly eight data registers and every caller extracts the
    /// index with `& 0x7`, so the masked lookup is total: there is no
    /// out-of-range case to fall back from.
    pub(crate) fn read_dn(&self, register: usize, size: Size) -> u32 {
        self.d[register & 0x7] & size.mask()
    }

    /// Reads address register `register`.
    pub(crate) const fn read_an(&self, register: usize) -> u32 {
        self.a[register & 0x7]
    }

    /// Writes address register `register` without stack-pointer bookkeeping.
    ///
    /// Instruction paths use this for `An`; the active stack pointer is still
    /// `A7` until exception entry selects `USP`/`SSP` in a later phase.
    pub(crate) fn write_an(&mut self, register: usize, value: u32) {
        self.a[register & 0x7] = value;
    }

    /// Writes the low `size` bytes of `value` into data register `register`.
    ///
    /// The index is masked for the same reason as in [`Self::read_dn`].
    pub(crate) fn write_dn(&mut self, register: usize, value: u32, size: Size) {
        let register = &mut self.d[register & 0x7];
        *register = match size {
            Size::Byte => (*register & 0xFFFF_FF00) | (value & 0xFF),
            Size::Word => (*register & 0xFFFF_0000) | (value & 0xFFFF),
            Size::Long => value,
        };
    }

    pub(crate) fn set_logic_flags(&mut self, value: u32, size: Size) {
        let masked = value & size.mask();
        let negative = masked & size.sign_bit() != 0;
        let zero = masked == 0;
        let extend = self.flag(FLAG_X);
        self.set_ccr(false, false, zero, negative, extend);
    }

    pub(crate) fn set_add_flags(&mut self, destination: u32, source: u32, result: u32, size: Size) {
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

    pub(crate) fn set_sub_flags(&mut self, destination: u32, source: u32, result: u32, size: Size) {
        let (carry, overflow, zero, negative) =
            Self::subtract_condition_codes(destination, source, result, size);
        self.set_ccr(carry, overflow, zero, negative, carry);
    }

    /// Sets flags for `CMP`, which is a subtraction that leaves `X` untouched.
    pub(crate) fn set_cmp_flags(&mut self, destination: u32, source: u32, size: Size) {
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

    pub(crate) fn set_ccr(
        &mut self,
        carry: bool,
        overflow: bool,
        zero: bool,
        negative: bool,
        extend: bool,
    ) {
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

    pub(crate) fn flag(&self, flag: u16) -> bool {
        self.sr & flag != 0
    }

    pub(crate) fn fetch_u16(&mut self, bus: &mut MegadriveBus) -> Result<u16, CpuError> {
        let value = bus.read16(self.pc)?;
        self.pc = self.pc.checked_add(2).ok_or(CpuError::AddressOverflow)?;
        Ok(value)
    }

    pub(crate) fn fetch_u32(&mut self, bus: &mut MegadriveBus) -> Result<u32, CpuError> {
        let high = u32::from(self.fetch_u16(bus)?);
        let low = u32::from(self.fetch_u16(bus)?);
        Ok((high << 16) | low)
    }

    pub(crate) fn push_long(&mut self, bus: &mut MegadriveBus, value: u32) -> Result<(), CpuError> {
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
    use super::{CpuError, M68000, Size, interrupt_level_deliverable, interrupt_priority_mask};
    use crate::bus::{BusError, MegadriveBus};
    use crate::trace::TraceError;

    /// Stack top used by the new-instruction helpers. Work RAM on this bus ends
    /// at 0x01000000; starting near the top keeps downward pushes inside RAM.
    const INITIAL_STACK: u32 = 0x00FF_FF00;

    fn bus_with_vectors() -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&0x00FF_0000_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        MegadriveBus::new(crate::cartridge::test_cartridge(rom))
    }

    /// Builds a bus whose reset vectors load `program` at address 0x200.
    fn program_bus(program: &[u8]) -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&INITIAL_STACK.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        rom[0x200..0x200 + program.len()].copy_from_slice(program);
        MegadriveBus::new(crate::cartridge::test_cartridge(rom))
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
        MegadriveBus::new(crate::cartridge::test_cartridge(rom))
    }

    fn cpu_with_vector(vector_address: u32, target: u32) -> (M68000, MegadriveBus) {
        let mut bus = vector_bus(vector_address, target);
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).expect("reset");
        (cpu, bus)
    }

    /// Builds a bus with a NOP at 0x200, `handler` bytes at `target`, and
    /// `target` registered in the vector table, so an exception entry can run a
    /// real handler (for example `RTE`).
    fn cpu_with_vector_and_handler(
        vector_address: u32,
        target: u32,
        handler: &[u8],
    ) -> (M68000, MegadriveBus) {
        let mut rom = vec![0_u8; 0x400];
        rom[0..4].copy_from_slice(&INITIAL_STACK.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        rom[0x200..0x202].copy_from_slice(&[0x4E, 0x71]);
        let handler_start = target as usize;
        rom[handler_start..handler_start + handler.len()].copy_from_slice(handler);
        let start = vector_address as usize;
        rom[start..start + 4].copy_from_slice(&target.to_be_bytes());
        let mut bus = MegadriveBus::new(crate::cartridge::test_cartridge(rom));
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
        let mut bus = MegadriveBus::new(crate::cartridge::test_cartridge(rom));
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
    fn absolute_word_and_pc_relative_control_forms_are_supported() {
        // LEA (0x0300).W,A0; the absolute-short address sign-extends.
        let (mut cpu, mut bus) = cpu_at_program(&[0x41, 0xF8, 0x03, 0x00]);
        cpu.step(&mut bus).expect("lea (abs).w");
        assert_eq!(cpu.address_register(0), Some(0x0300));
        assert_eq!(cpu.pc(), 0x204);

        // PEA (0x0300).W pushes the absolute-short address.
        let (mut cpu, mut bus) = cpu_at_program(&[0x48, 0x78, 0x03, 0x00]);
        cpu.step(&mut bus).expect("pea (abs).w");
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK - 4));
        assert_eq!(
            bus.read32(INITIAL_STACK - 4).expect("pushed address"),
            0x0300
        );

        // LEA (d16,PC),A0: base 0x202 + 4 = 0x206.
        let (mut cpu, mut bus) = cpu_at_program(&[0x41, 0xFA, 0x00, 0x04]);
        cpu.step(&mut bus).expect("lea (d16,pc)");
        assert_eq!(cpu.address_register(0), Some(0x0206));

        // PEA (d16,PC)
        let (mut cpu, mut bus) = cpu_at_program(&[0x48, 0x7A, 0x00, 0x04]);
        cpu.step(&mut bus).expect("pea (d16,pc)");
        assert_eq!(
            bus.read32(INITIAL_STACK - 4).expect("pushed address"),
            0x0206
        );
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
    fn relative_target_reports_overflow_and_keeps_the_u32_boundary() {
        let (mut cpu, _bus) = cpu_at_program(&[]);
        cpu.pc = 0;
        assert_eq!(cpu.relative_target(-1), Err(CpuError::AddressOverflow));
        cpu.pc = u32::MAX;
        assert_eq!(cpu.relative_target(1), Err(CpuError::AddressOverflow));
        // The widest representable even target survives the conversion intact.
        cpu.pc = 0xFFFF_FFFE;
        assert_eq!(cpu.relative_target(0), Ok(0xFFFF_FFFE));
        // One past that is odd, so the target is preserved and then rejected.
        assert_eq!(
            cpu.relative_target(1),
            Err(CpuError::AddressError {
                address: 0xFFFF_FFFF
            })
        );
    }

    #[test]
    fn every_data_register_round_trips_with_size_preservation() {
        let (mut cpu, _bus) = cpu_at_program(&[]);
        for register in 0..8 {
            cpu.write_dn(register, 0x1122_3344, Size::Long);
        }
        for register in 0..8 {
            assert_eq!(cpu.read_dn(register, Size::Long), 0x1122_3344);
        }
        cpu.write_dn(3, 0xAA, Size::Byte);
        cpu.write_dn(4, 0xBBCC, Size::Word);
        assert_eq!(cpu.read_dn(3, Size::Long), 0x1122_33AA);
        assert_eq!(cpu.read_dn(4, Size::Long), 0x1122_BBCC);
        assert_eq!(cpu.read_dn(3, Size::Byte), 0xAA);
        assert_eq!(cpu.read_dn(4, Size::Word), 0xBBCC);
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
        cpu.interrupt(6, 0x78, &mut bus).expect("interrupt");
        assert_eq!(cpu.pc(), 0x300);
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK - 6));
        assert_eq!(
            bus.read16(INITIAL_STACK - 6).expect("pushed sr"),
            previous_sr
        );
        assert_eq!(bus.read32(INITIAL_STACK - 4).expect("pushed pc"), 0x202);
        assert_eq!(cpu.sr(), 0x2600, "level 6 is loaded into IPM with S set");
        assert_eq!(cpu.interrupt_priority_mask(), 6);
        assert!(!cpu.halted());
    }

    #[test]
    fn interrupt_enters_supervisor_mode_and_preserves_condition_codes() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.sr = 0x0004; // User mode with Z set.
        cpu.interrupt(6, 0x78, &mut bus).expect("interrupt");
        assert_eq!(bus.read16(INITIAL_STACK - 6).expect("pushed sr"), 0x0004);
        assert_eq!(bus.read32(INITIAL_STACK - 4).expect("pushed pc"), 0x200);
        assert_eq!(
            cpu.sr(),
            0x2604,
            "S set, IPM becomes the accepted level, CCR preserved"
        );
    }

    #[test]
    fn interrupt_wakes_a_halted_processor() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.halted = true;
        cpu.interrupt(6, 0x78, &mut bus).expect("interrupt");
        assert!(!cpu.halted());
        assert_eq!(cpu.pc(), 0x300);
    }

    #[test]
    fn interrupt_rejects_stack_underflow() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.a[7] = 0;
        assert_eq!(
            cpu.interrupt(6, 0x78, &mut bus),
            Err(CpuError::StackOverflow)
        );
        assert_eq!(cpu.pc(), 0x200);
    }

    #[test]
    fn interrupt_rejects_a_rom_stack() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
        cpu.a[7] = 0x0000_0300;
        cpu.sr = 0x0000;
        assert_eq!(
            cpu.interrupt(6, 0x78, &mut bus),
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
            cpu.interrupt(6, 0x0080_0000, &mut bus),
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
            cpu.interrupt(6, 0x79, &mut bus),
            Err(CpuError::Bus(BusError::OddAddress { address: 0x79 }))
        );
        assert_ne!(cpu.sr() & 0x2000, 0);
    }

    #[test]
    fn interrupt_rejects_an_odd_target_pc() {
        let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0301);
        cpu.sr = 0x0000;
        assert_eq!(
            cpu.interrupt(6, 0x78, &mut bus),
            Err(CpuError::AddressError { address: 0x301 })
        );
        assert_eq!(cpu.pc(), 0x200);
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK - 6));
        assert_eq!(
            cpu.sr(),
            0x2600,
            "supervisor mode and the accepted IPM are applied before the jump"
        );
    }

    #[test]
    fn interrupt_loads_the_accepted_level_into_ipm() {
        // Level 6 entry from mask 3, user mode, Z set. The handler is RTE so the
        // full round trip can be checked in one test.
        let (mut cpu, mut bus) = cpu_with_vector_and_handler(0x78, 0x0000_0300, &[0x4E, 0x73]);
        cpu.sr = 0x2304;
        cpu.interrupt(6, 0x78, &mut bus).expect("level 6 entry");

        assert_eq!(cpu.sr(), 0x2604, "S set, IPM 6, user CCR preserved");
        assert_eq!(cpu.interrupt_priority_mask(), 6);
        assert!(
            !cpu.accepts_interrupt_level(4),
            "a pending HBlank must not nest inside the VBlank handler"
        );
        assert!(!cpu.accepts_interrupt_level(5));
        assert!(!cpu.accepts_interrupt_level(6));
        assert!(cpu.accepts_interrupt_level(7), "level 7 stays always taken");
        assert_eq!(bus.read16(INITIAL_STACK - 6).expect("frame sr"), 0x2304);
        assert_eq!(bus.read32(INITIAL_STACK - 4).expect("frame pc"), 0x200);

        cpu.step(&mut bus).expect("rte");
        assert_eq!(cpu.sr(), 0x2304, "RTE restores the exact pre-interrupt SR");
        assert_eq!(cpu.interrupt_priority_mask(), 3);
        assert!(!cpu.accepts_interrupt_level(3));
        assert!(cpu.accepts_interrupt_level(4));
        assert_eq!(
            cpu.pc(),
            0x200,
            "RTE returns to the interrupted instruction"
        );
    }

    #[test]
    fn interrupt_level_seven_sets_ipm_seven() {
        let (mut cpu, mut bus) = cpu_with_vector(0x7C, 0x0000_0300);
        cpu.sr = 0x2500;
        cpu.interrupt(7, 0x7C, &mut bus).expect("level 7 entry");
        assert_eq!(cpu.sr(), 0x2700);
        assert_eq!(cpu.interrupt_priority_mask(), 7);
        assert!(cpu.accepts_interrupt_level(7));
        assert!(!cpu.accepts_interrupt_level(6));
        assert_eq!(bus.read16(INITIAL_STACK - 6).expect("frame sr"), 0x2500);
    }

    #[test]
    fn interrupt_rejects_invalid_levels_before_touching_state() {
        for level in [0_u8, 8, 0xFF] {
            let (mut cpu, mut bus) = cpu_with_vector(0x78, 0x0000_0300);
            assert_eq!(
                cpu.interrupt(level, 0x78, &mut bus),
                Err(CpuError::InvalidInterruptLevel { level }),
                "level {level} must be a typed error"
            );
            assert_eq!(cpu.pc(), 0x200);
            assert_eq!(cpu.sr(), 0x2700);
            assert_eq!(cpu.address_register(7), Some(INITIAL_STACK));
        }
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
    fn move_abs_word_load_encodings_are_supported() {
        // 0x..38 is source mode/register 111/000 (absolute-word). The address
        // extension word selects 0x0300, which is zero-filled ROM.
        for opcode in [0x1038_u16, 0x1238, 0x2038, 0x2438, 0x3038, 0x3638] {
            let mut program = opcode.to_be_bytes().to_vec();
            program.extend_from_slice(&0x0300_u16.to_be_bytes());
            let (mut cpu, mut bus) = cpu_at_program(&program);
            cpu.step(&mut bus).expect("move (abs).w source");
            assert_eq!(cpu.pc(), 0x204);
            assert_eq!(cpu.sr(), 0x2704, "zero source sets Z and clears V/C");
            let destination = usize::from((opcode >> 9) & 0x7);
            assert_eq!(cpu.data_register(destination), Some(0));
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

    #[test]
    fn interrupt_priority_mask_selects_only_levels_above_the_mask() {
        assert_eq!(interrupt_priority_mask(0x2700), 7);
        // At mask 7 the VDP's VBlank (level 6) and HBlank (level 4) stay
        // pending, while level 7 is always taken.
        assert!(interrupt_level_deliverable(7, 0x2700));
        assert!(!interrupt_level_deliverable(6, 0x2700));
        assert!(!interrupt_level_deliverable(4, 0x2700));
        // The threshold is strict: level == mask is rejected, level > mask is
        // taken. VBlank (6) is admitted from mask 5 and below; HBlank (4) from
        // mask 3 and below.
        assert!(!interrupt_level_deliverable(6, 0x2600));
        assert!(interrupt_level_deliverable(6, 0x2500));
        assert!(!interrupt_level_deliverable(5, 0x2500));
        assert!(!interrupt_level_deliverable(4, 0x2400));
        assert!(interrupt_level_deliverable(5, 0x2400));
        assert!(interrupt_level_deliverable(4, 0x2300));
        assert!(
            interrupt_level_deliverable(4, 0x2300 | 0x2000),
            "the S bit does not change the mask rule"
        );
        assert!(interrupt_level_deliverable(4, 0x0000));
        assert!(
            !interrupt_level_deliverable(0, 0x0000),
            "level 0 is the 68000 'no request' level"
        );
        // Only 68000 levels 1..=7 exist; 0 and values above 7 are never
        // deliverable even at mask 0.
        assert!(!interrupt_level_deliverable(0, 0x2700));
        assert!(!interrupt_level_deliverable(8, 0x0000));
        assert!(!interrupt_level_deliverable(u8::MAX, 0x2700));
        // Level 7 is taken even at the highest mask and in user mode.
        assert!(interrupt_level_deliverable(7, 0x0700));
        assert!(interrupt_level_deliverable(7, 0x0000));
        // Only IPM bits 10-8 form the mask; S and the CCR do not.
        assert_eq!(interrupt_priority_mask(0x20FF), 0);
        assert_eq!(interrupt_priority_mask(0xFFFF), 7);
    }

    #[test]
    fn accepts_interrupt_level_reads_the_current_raw_sr() {
        let (mut cpu, _bus) = cpu_at_program(&[]);
        assert_eq!(cpu.sr(), 0x2700);
        assert_eq!(cpu.interrupt_priority_mask(), 7);
        assert!(cpu.accepts_interrupt_level(7));
        assert!(!cpu.accepts_interrupt_level(6));
        assert!(!cpu.accepts_interrupt_level(4));
        assert!(!cpu.accepts_interrupt_level(8), "level 8 does not exist");

        cpu.sr = 0x2500;
        assert_eq!(cpu.interrupt_priority_mask(), 5);
        assert!(cpu.accepts_interrupt_level(6));
        assert!(!cpu.accepts_interrupt_level(5));
        assert!(!cpu.accepts_interrupt_level(4));

        cpu.sr = 0x2300;
        assert!(
            cpu.accepts_interrupt_level(4),
            "HBlank is admitted at mask 3"
        );
    }

    #[test]
    fn move_immediate_to_sr_sets_the_raw_status_register_and_mask() {
        // MOVE.W #$2000,SR; MOVE.W #$2508,SR
        let (mut cpu, mut bus) = cpu_at_program(&[
            0x46, 0xFC, 0x20, 0x00, //
            0x46, 0xFC, 0x25, 0x08,
        ]);
        assert!(!cpu.accepts_interrupt_level(6), "reset masks level 6");

        cpu.step(&mut bus).expect("move.w #$2000,sr");
        assert_eq!(cpu.sr(), 0x2000);
        assert_eq!(cpu.interrupt_priority_mask(), 0);
        assert!(cpu.accepts_interrupt_level(4));
        assert!(cpu.accepts_interrupt_level(6));

        cpu.step(&mut bus).expect("move.w #$2508,sr");
        assert_eq!(cpu.sr(), 0x2508, "the raw word replaces SR exactly");
        assert_eq!(cpu.interrupt_priority_mask(), 5);
        assert!(cpu.accepts_interrupt_level(6));
        assert!(!cpu.accepts_interrupt_level(4));
        assert_eq!(cpu.pc(), 0x208);
    }

    #[test]
    fn rte_restores_raw_sr_pc_and_advances_the_stack_pointer() {
        // RTE with a frame holding S=1, IPM=5 and CCR=0x35.
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x73]);
        cpu.a[7] = INITIAL_STACK;
        cpu.sr = 0x2700;
        bus.write16(INITIAL_STACK, 0x2535).expect("frame sr");
        bus.write32(INITIAL_STACK + 2, 0x0000_0400)
            .expect("frame pc");

        cpu.step(&mut bus).expect("rte");
        assert_eq!(cpu.sr(), 0x2535, "raw SR incl. IPM/S/CCR is restored");
        assert_eq!(cpu.interrupt_priority_mask(), 5);
        assert!(cpu.accepts_interrupt_level(6));
        assert!(!cpu.accepts_interrupt_level(5));
        assert!(!cpu.accepts_interrupt_level(4));
        assert!(cpu.accepts_interrupt_level(7));
        assert_eq!(cpu.pc(), 0x400);
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK + 6));
    }

    #[test]
    fn rte_restores_user_mode_and_condition_codes_exactly() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x73]);
        cpu.a[7] = INITIAL_STACK;
        bus.write16(INITIAL_STACK, 0x0015).expect("frame sr");
        bus.write32(INITIAL_STACK + 2, 0x0000_0300)
            .expect("frame pc");

        cpu.step(&mut bus).expect("rte");
        assert_eq!(cpu.sr(), 0x0015, "user mode and CCR are restored verbatim");
        assert_eq!(cpu.sr() & 0x2000, 0, "RTE does not force supervisor mode");
        assert_eq!(cpu.pc(), 0x300);
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK + 6));
    }

    #[test]
    fn rte_rejects_stack_underflow_without_touching_state() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x73]);
        cpu.a[7] = 0xFFFF_FFFC;
        assert_eq!(cpu.step(&mut bus), Err(CpuError::StackOverflow));
        assert_eq!(cpu.sr(), 0x2700);
        assert_eq!(cpu.address_register(7), Some(0xFFFF_FFFC));
        assert_eq!(cpu.pc(), 0x202, "only the opcode word was fetched");
    }

    #[test]
    fn rte_rejects_an_unmapped_frame_without_touching_state() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x73]);
        cpu.a[7] = 0x0080_0000;
        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::Bus(BusError::UnmappedRead {
                address: 0x0080_0000
            }))
        );
        assert_eq!(cpu.sr(), 0x2700);
        assert_eq!(cpu.address_register(7), Some(0x0080_0000));
        assert_eq!(cpu.pc(), 0x202);
    }

    #[test]
    fn rte_rejects_an_unmapped_pc_read_after_a_valid_sr_read() {
        // The SR word is readable but the return PC crosses the RAM window end,
        // so the frame must be validated before any state is committed.
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x73]);
        cpu.a[7] = 0x00FF_FFFE;
        cpu.sr = 0x2701;
        bus.write16(0x00FF_FFFE, 0x2004).expect("frame sr");
        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::Bus(BusError::UnmappedRead {
                address: 0x0100_0000
            }))
        );
        assert_eq!(cpu.sr(), 0x2701, "the valid SR read must not be committed");
        assert_eq!(cpu.address_register(7), Some(0x00FF_FFFE));
        assert_eq!(cpu.pc(), 0x202);
    }

    #[test]
    fn rte_rejects_an_odd_restored_pc_without_touching_state() {
        let (mut cpu, mut bus) = cpu_at_program(&[0x4E, 0x73]);
        cpu.a[7] = INITIAL_STACK;
        bus.write16(INITIAL_STACK, 0x2000).expect("frame sr");
        bus.write32(INITIAL_STACK + 2, 0x0000_0401)
            .expect("frame pc");
        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::AddressError { address: 0x401 })
        );
        assert_eq!(cpu.sr(), 0x2700);
        assert_eq!(cpu.address_register(7), Some(INITIAL_STACK));
        assert_eq!(cpu.pc(), 0x202);
    }

    #[test]
    fn trace_records_move_to_sr_and_rte() {
        // MOVE.W #$2000,SR; RTE
        let (mut cpu, mut bus) = cpu_at_program(&[0x46, 0xFC, 0x20, 0x00, 0x4E, 0x73]);
        cpu.a[7] = INITIAL_STACK;
        bus.write16(INITIAL_STACK, 0x2000).expect("frame sr");
        bus.write32(INITIAL_STACK + 2, 0x0000_0300)
            .expect("frame pc");
        cpu.enable_trace(4).expect("trace capacity 4");

        cpu.step(&mut bus).expect("move.w #imm,sr");
        cpu.step(&mut bus).expect("rte");

        let records: Vec<(u64, u32, u16)> = cpu
            .trace()
            .expect("trace enabled")
            .entries()
            .map(|entry| (entry.index(), entry.pc(), entry.opcode()))
            .collect();
        assert_eq!(records, vec![(0, 0x200, 0x46FC), (1, 0x204, 0x4E73)]);
        assert_eq!(cpu.pc(), 0x300);
        assert_eq!(cpu.sr(), 0x2000);
    }
}
