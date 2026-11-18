//! The single 68000 opcode classifier used by [`M68000::step`].
//!
//! Every opcode that the CPU executes is classified here exactly once; the
//! execution groups in [`crate::cpu::exec`] then perform the operation. A
//! group must never re-implement the top-level dispatch decision, so the
//! decode table stays the one place that maps an encoding to an instruction
//! class.
//!
//! The classification follows the MC68000 encoding map. Classes are grouped by
//! instruction family rather than by mnemonic detail: the group's `execute`
//! function decodes its own operand fields (size, direction, effective
//! address), which are part of the instruction's encoding, not of the dispatch
//! decision.

/// One 68000 instruction class, as selected by [`classify`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstructionClass {
    /// `MOVE` and `MOVEA` (the destination mode selects `MOVEA`).
    Move,
    /// `MOVEQ`.
    Moveq,
    /// `MOVEM`.
    Movem,
    /// `MOVEP`.
    Movep,
    /// `LEA`.
    Lea,
    /// `PEA`.
    Pea,
    /// `CLR`.
    Clr,
    /// `TST`.
    Tst,
    /// `EXT`.
    Ext,
    /// `SWAP`.
    Swap,
    /// `ADD`, `ADDA` and `ADDQ`/`ADDI` share this family.
    Add,
    /// `ADDA` (word/long to an address register).
    Adda,
    /// `ADDQ`.
    Addq,
    /// `ADDI`.
    Addi,
    /// `SUB`, `SUBA` and `SUBQ`/`SUBI` share this family.
    Sub,
    /// `SUBA` (word/long to an address register).
    Suba,
    /// `SUBQ`.
    Subq,
    /// `SUBI`.
    Subi,
    /// `CMP` (effective address to a data register).
    Cmp,
    /// `CMPA` (effective address to an address register).
    Cmpa,
    /// `CMPI` (immediate compare).
    Cmpi,
    /// `CMPM` (post-increment memory compare).
    Cmpm,
    /// `NEG`.
    Neg,
    /// `NEGX`.
    Negx,
    /// `AND`.
    And,
    /// `ANDI`.
    Andi,
    /// `OR`.
    Or,
    /// `ORI`.
    Ori,
    /// `EOR`.
    Eor,
    /// `EORI`.
    Eori,
    /// `NOT`.
    Not,
    /// Register and memory shifts and rotates.
    Shift,
    /// `BRA`, `BSR` and the conditional branches.
    Branch,
    /// `NOP`.
    Nop,
    /// `RTE`.
    Rte,
    /// `RTS`.
    Rts,
    /// `JMP` through an absolute-long effective address.
    Jmp,
    /// `JSR` through an absolute-long effective address.
    Jsr,
    /// `MOVE.W #imm,SR`.
    MoveToSr,
    /// `DBRA`.
    Dbra,
    /// An encoding no implemented group owns yet.
    Other,
}

/// Classifies one opcode word.
///
/// The order is significant where encodings overlap: `MOVEQ`/branches before
/// the generic data movement map, exact system opcodes before their families,
/// and `EXT`/`SWAP` before the `MOVEM`/`PEA` regions they share bits with.
/// `M68000::step` uses the returned class for its dispatch; there is no second
/// classifier on the execution path.
#[must_use]
pub const fn classify(opcode: u16) -> InstructionClass {
    // 0111 rrr 0 dddddddd: MOVEQ. Bit 8 must be clear.
    if (opcode & 0xF100) == 0x7000 {
        return InstructionClass::Moveq;
    }
    // 0110 cccc dddddddd: BRA/BSR/Bcc.
    if (opcode & 0xF000) == 0x6000 {
        return InstructionClass::Branch;
    }
    // Exact system and control encodings handled by the core state machine.
    match opcode {
        0x46FC => return InstructionClass::MoveToSr,
        0x4E71 => return InstructionClass::Nop,
        0x4E73 => return InstructionClass::Rte,
        0x4E75 => return InstructionClass::Rts,
        0x4EB9 => return InstructionClass::Jsr,
        0x4EF9 => return InstructionClass::Jmp,
        _ => {}
    }

    let size = (opcode >> 6) & 0x3;
    let mode = (opcode >> 3) & 0x7;

    // 00ss ddd mmm rrr: MOVE/MOVEA. The destination mode 001 selects MOVEA.
    if matches!(opcode & 0xF000, 0x1000 | 0x2000 | 0x3000) {
        return InstructionClass::Move;
    }
    // EXT.W/EXT.L Dn share the MOVEM region with mode 000.
    if matches!(opcode & 0xFFF8, 0x4880 | 0x48C0) {
        return InstructionClass::Ext;
    }
    // SWAP Dn shares the PEA region with mode 000.
    if (opcode & 0xFFF8) == 0x4840 {
        return InstructionClass::Swap;
    }
    // MOVEM: 0100 1d00 1S mmm rrr.
    if (opcode & 0xFB80) == 0x4880 {
        return InstructionClass::Movem;
    }
    // PEA: 0100 1000 01 mmm rrr.
    if (opcode & 0xFFC0) == 0x4840 {
        return InstructionClass::Pea;
    }
    // LEA: 0100 ddd 111 mmm rrr.
    if (opcode & 0xF1C0) == 0x41C0 {
        return InstructionClass::Lea;
    }
    // MOVEP: 0000 ddd 1xx 001 aaa.
    if matches!(opcode & 0xF1F8, 0x0108 | 0x0148 | 0x0188 | 0x01C8) {
        return InstructionClass::Movep;
    }

    // CLR, TST, NEG, NEGX and NOT: 0100 xxxx ss mmm rrr with a valid size.
    if size != 0x3 {
        match opcode & 0xFF00 {
            0x4000 => return InstructionClass::Negx,
            0x4200 => return InstructionClass::Clr,
            0x4400 => return InstructionClass::Neg,
            0x4600 => return InstructionClass::Not,
            0x4A00 => return InstructionClass::Tst,
            _ => {}
        }
    }

    // Immediate arithmetic and logic: opcode 0000 0xxx ss mmm rrr.
    if size != 0x3 {
        match opcode & 0xFF00 {
            0x0000 => return InstructionClass::Ori,
            0x0200 => return InstructionClass::Andi,
            0x0400 => return InstructionClass::Subi,
            0x0600 => return InstructionClass::Addi,
            0x0A00 => return InstructionClass::Eori,
            0x0C00 => return InstructionClass::Cmpi,
            _ => {}
        }
    }

    match opcode & 0xF000 {
        0x8000 => InstructionClass::Or,
        0x9000 => InstructionClass::Sub,
        0xB000 => match (opcode >> 6) & 0x7 {
            0..=2 => InstructionClass::Cmp,
            3 | 7 => InstructionClass::Cmpa,
            4..=6 => {
                if mode == 1 {
                    InstructionClass::Cmpm
                } else {
                    InstructionClass::Eor
                }
            }
            _ => InstructionClass::Other,
        },
        0xC000 => InstructionClass::And,
        0xD000 => InstructionClass::Add,
        0x5000 => {
            if size == 0x3 {
                if (opcode & 0xFFF8) == 0x51C8 {
                    InstructionClass::Dbra
                } else {
                    InstructionClass::Other
                }
            } else if opcode & 0x0100 != 0 {
                InstructionClass::Subq
            } else {
                InstructionClass::Addq
            }
        }
        0xE000 => {
            if (opcode & 0xFFC0) == 0xE0C0 || size != 0x3 {
                InstructionClass::Shift
            } else {
                InstructionClass::Other
            }
        }
        _ => InstructionClass::Other,
    }
}
