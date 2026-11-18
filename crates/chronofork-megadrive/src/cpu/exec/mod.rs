//! Instruction execution groups for the 68000 core.
//!
//! Each group is a pure function over the register file and the bus. The
//! group's `execute` function returns the instruction's documented 68000-cycle
//! cost; the backend advances its master-cycle timeline by that cost times seven.
//!
//! [`crate::cpu::decode`] is the single opcode classifier; a group only decodes
//! its own operand fields (size, direction and effective address) once the
//! classifier has selected it.

use super::{CpuError, M68000, Size};

pub mod arith;
pub mod bit;
pub mod control;
pub mod logic;
pub mod r#move;
pub mod shift;
pub mod system;

/// Sets the condition codes for a logic result.
///
/// This is the single implementation of the logic flag rule: `N`/`Z` follow the
/// result, `V`/`C` are cleared and `X` is preserved.
pub fn set_logic_flags(cpu: &mut M68000, value: u32, size: Size) {
    cpu.set_logic_flags(value, size);
}

/// Builds a typed illegal-opcode error.
///
/// `instruction_pc` is the address of the opcode word, captured before the
/// group consumes extension words.
pub(crate) fn illegal_opcode(instruction_pc: u32, opcode: u16) -> CpuError {
    CpuError::IllegalOpcode {
        pc: instruction_pc,
        opcode,
    }
}
