//! Instruction execution groups for the 68000 core.
//!
//! Each group is a pure function over the register file and the bus. The
//! group's `execute` function returns the instruction's 68000-cycle cost; the
//! timing table lands in a later task, so the groups currently report zero and
//! the backend keeps its instruction-quantum clock.
//!
//! [`crate::cpu::decode`] is the single opcode classifier; a group only decodes
//! its own operand fields (size, direction and effective address) once the
//! classifier has selected it.

use super::CpuError;

pub mod r#move;

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
