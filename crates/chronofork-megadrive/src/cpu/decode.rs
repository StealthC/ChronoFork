//! The single 68000 opcode classifier used by [`crate::M68000::step`].
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
    /// `ADDX` (extend add, register or predecrement memory form).
    Addx,
    /// `SUBX` (extend subtract, register or predecrement memory form).
    Subx,
    /// `EXG` (exchange registers).
    Exg,
    /// `TAS` (test and set byte).
    Tas,
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
    /// `MULU`/`MULS`/`DIVU`/`DIVS`.
    MulDiv,
    /// `ABCD`/`SBCD`/`NBCD`.
    Bcd,
    /// `BTST`/`BCHG`/`BCLR`/`BSET` (static or dynamic bit number).
    Bit,
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
    /// System and privileged instructions.
    System,
    /// `DBcc`.
    Dbra,
    /// `Scc`.
    Scc,
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
        0x4E71 => return InstructionClass::Nop,
        0x4E73 => return InstructionClass::Rte,
        0x4E75 => return InstructionClass::Rts,
        _ => {}
    }

    let size = (opcode >> 6) & 0x3;
    let mode = (opcode >> 3) & 0x7;
    let opmode = (opcode >> 6) & 0x7;

    // ADDX/SUBX use the ADD/SUB "Dn,<ea>" opmode with the An-direct mode field
    // (mode 0 = Dn,Dn; mode 1 = -(An),-(An)). Those encodings are ADDX/SUBX,
    // never ADD/SUB; dispatch them before the general families.
    if (opcode & 0xF000) == 0xD000 && matches!(opmode, 4..=6) && matches!(mode, 0 | 1) {
        return InstructionClass::Addx;
    }
    if (opcode & 0xF000) == 0x9000 && matches!(opmode, 4..=6) && matches!(mode, 0 | 1) {
        return InstructionClass::Subx;
    }

    // EXG occupies three encodings inside the AND family's mask.
    if matches!(opcode & 0xF1F8, 0xC140 | 0xC148 | 0xC188) {
        return InstructionClass::Exg;
    }

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

    // System and privileged instructions.
    if matches!(opcode & 0xFFC0, 0x40C0 | 0x42C0 | 0x44C0 | 0x46C0) {
        return InstructionClass::System;
    }
    if matches!(opcode, 0x003C | 0x007C | 0x023C | 0x027C | 0x0A3C | 0x0A7C) {
        return InstructionClass::System;
    }
    if (opcode & 0xF1C0) == 0x4180 {
        return InstructionClass::System;
    }
    if opcode == 0x4AFC {
        return InstructionClass::System;
    }
    // TAS: 0100 1010 11 mmm rrr. ILLEGAL (0x4AFC) is the one reserved word in
    // this range and is handled above.
    if (opcode & 0xFFC0) == 0x4AC0 {
        return InstructionClass::Tas;
    }
    if (opcode & 0xFFF8) == 0x4E50
        || (opcode & 0xFFF8) == 0x4E58
        || (opcode & 0xFFF8) == 0x4E60
        || (opcode & 0xFFF8) == 0x4E68
        || (opcode & 0xFFF0) == 0x4E40
    {
        return InstructionClass::System;
    }
    if matches!(opcode, 0x4E70 | 0x4E72 | 0x4E76 | 0x4E77) {
        return InstructionClass::System;
    }

    // JMP/JSR take a control effective address.
    if (opcode & 0xFFC0) == 0x4EC0 {
        return InstructionClass::Jmp;
    }
    if (opcode & 0xFFC0) == 0x4E80 {
        return InstructionClass::Jsr;
    }

    // Bit operations: static 0000 1000 tt mmm rrr, dynamic 0000 rrr 1tt mmm rrr.
    if (opcode & 0xF000) == 0 && ((opcode & 0x0F00) == 0x0800 || (opcode & 0x0100) != 0) {
        return InstructionClass::Bit;
    }

    // ABCD/SBCD (-(Ay),-(Ax) or Dy,Dx) and NBCD.
    if matches!(opcode & 0xF1F8, 0xC100 | 0xC108 | 0x8100 | 0x8108) {
        return InstructionClass::Bcd;
    }
    if (opcode & 0xFFC0) == 0x4800 {
        return InstructionClass::Bcd;
    }

    // MULU/MULS (1100 ddd 011 mmm rrr) and DIVU/DIVS (1000 ddd 011 mmm rrr).
    if matches!(opcode & 0xF1C0, 0xC0C0 | 0xC1C0 | 0x80C0 | 0x81C0) {
        return InstructionClass::MulDiv;
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
        0x9000 => match (opcode >> 6) & 0x7 {
            3 | 7 => InstructionClass::Suba,
            _ => InstructionClass::Sub,
        },
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
        0xD000 => match (opcode >> 6) & 0x7 {
            3 | 7 => InstructionClass::Adda,
            _ => InstructionClass::Add,
        },
        0x5000 => {
            if size == 0x3 {
                if (opcode & 0xF0F8) == 0x50C8 {
                    InstructionClass::Dbra
                } else if mode != 1 {
                    InstructionClass::Scc
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
            if (opcode & 0xF8C0) == 0xE0C0 || size != 0x3 {
                InstructionClass::Shift
            } else {
                InstructionClass::Other
            }
        }
        _ => InstructionClass::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::{InstructionClass, classify};

    /// One documented encoding and the family it must classify into.
    ///
    /// This is the audit table for the MC68000 encoding map: every documented
    /// instruction family has a representative encoding, and the special
    /// encodings that live inside another family's mask (EXG in AND, ADDX/SUBX
    /// in ADD/SUB, TAS in the size-3 4Axx region, MOVE to/from SR/CCR,
    /// CDBcc/Scc, MOVEM/EXT/SWAP/PEA) are pinned separately.
    const DOCUMENTED: &[(u16, InstructionClass)] = &[
        (0x7000, InstructionClass::Moveq),
        (0x6002, InstructionClass::Branch),
        (0x4E71, InstructionClass::Nop),
        (0x4E73, InstructionClass::Rte),
        (0x4E75, InstructionClass::Rts),
        (0x4E70, InstructionClass::System),
        (0x4E72, InstructionClass::System),
        (0x4E76, InstructionClass::System),
        (0x4E77, InstructionClass::System),
        (0x4E40, InstructionClass::System),
        (0x4E50, InstructionClass::System),
        (0x4E58, InstructionClass::System),
        (0x4E60, InstructionClass::System),
        (0x4E68, InstructionClass::System),
        (0x4AFC, InstructionClass::System),
        (0x003C, InstructionClass::System),
        (0x007C, InstructionClass::System),
        (0x023C, InstructionClass::System),
        (0x027C, InstructionClass::System),
        (0x0A3C, InstructionClass::System),
        (0x0A7C, InstructionClass::System),
        (0x40C0, InstructionClass::System),
        (0x42C0, InstructionClass::System),
        (0x44C0, InstructionClass::System),
        (0x46C0, InstructionClass::System),
        (0x4180, InstructionClass::System),
        (0x4EF9, InstructionClass::Jmp),
        (0x4EC0, InstructionClass::Jmp),
        (0x4EB9, InstructionClass::Jsr),
        (0x4E80, InstructionClass::Jsr),
        (0x1000, InstructionClass::Move),
        (0x2000, InstructionClass::Move),
        (0x3000, InstructionClass::Move),
        (0x4880, InstructionClass::Ext),
        (0x48C0, InstructionClass::Ext),
        (0x4840, InstructionClass::Swap),
        (0x4890, InstructionClass::Movem),
        (0x48E0, InstructionClass::Movem),
        (0x4CD8, InstructionClass::Movem),
        (0x4848, InstructionClass::Pea),
        (0x41C0, InstructionClass::Lea),
        (0x0108, InstructionClass::Movep),
        (0x0148, InstructionClass::Movep),
        (0x0188, InstructionClass::Movep),
        (0x01C8, InstructionClass::Movep),
        (0x4000, InstructionClass::Negx),
        (0x4200, InstructionClass::Clr),
        (0x4400, InstructionClass::Neg),
        (0x4600, InstructionClass::Not),
        (0x4800, InstructionClass::Bcd),
        (0x4A00, InstructionClass::Tst),
        (0x0800, InstructionClass::Bit),
        (0x0840, InstructionClass::Bit),
        (0x0880, InstructionClass::Bit),
        (0x08C0, InstructionClass::Bit),
        (0x0100, InstructionClass::Bit),
        (0xC100, InstructionClass::Bcd),
        (0xC108, InstructionClass::Bcd),
        (0x8100, InstructionClass::Bcd),
        (0x8108, InstructionClass::Bcd),
        (0xC0C0, InstructionClass::MulDiv),
        (0xC1C0, InstructionClass::MulDiv),
        (0x80C0, InstructionClass::MulDiv),
        (0x81C0, InstructionClass::MulDiv),
        (0x0000, InstructionClass::Ori),
        (0x0200, InstructionClass::Andi),
        (0x0400, InstructionClass::Subi),
        (0x0600, InstructionClass::Addi),
        (0x0A00, InstructionClass::Eori),
        (0x0C00, InstructionClass::Cmpi),
        (0x8000, InstructionClass::Or),
        (0x9000, InstructionClass::Sub),
        (0xB000, InstructionClass::Cmp),
        (0xB0C0, InstructionClass::Cmpa),
        (0xB108, InstructionClass::Cmpm),
        (0xB140, InstructionClass::Eor),
        (0xC000, InstructionClass::And),
        (0xD000, InstructionClass::Add),
        (0xD0C0, InstructionClass::Adda),
        (0x5000, InstructionClass::Addq),
        (0x5100, InstructionClass::Subq),
        (0x50C8, InstructionClass::Dbra),
        (0x50C0, InstructionClass::Scc),
        (0xE000, InstructionClass::Shift),
        (0xE0C0, InstructionClass::Shift),
        (0xC140, InstructionClass::Exg),
        (0xC148, InstructionClass::Exg),
        (0xC188, InstructionClass::Exg),
        (0xD100, InstructionClass::Addx),
        (0xD140, InstructionClass::Addx),
        (0xD180, InstructionClass::Addx),
        (0xD108, InstructionClass::Addx),
        (0xD148, InstructionClass::Addx),
        (0xD188, InstructionClass::Addx),
        (0x9100, InstructionClass::Subx),
        (0x9140, InstructionClass::Subx),
        (0x9180, InstructionClass::Subx),
        (0x9108, InstructionClass::Subx),
        (0x9148, InstructionClass::Subx),
        (0x9188, InstructionClass::Subx),
        (0x4AC0, InstructionClass::Tas),
        (0x4AD0, InstructionClass::Tas),
    ];

    #[test]
    fn documented_encodings_classify_into_their_own_families() {
        for (opcode, expected) in DOCUMENTED {
            assert_eq!(
                classify(*opcode),
                *expected,
                "0x{opcode:04X} must classify as {expected:?}"
            );
        }
    }

    #[test]
    fn special_encodings_never_fall_into_a_general_family() {
        // The EXG-as-AND regression: 0xC141 is EXG D0,D1, not AND.W D0,D1.
        assert_eq!(classify(0xC141), InstructionClass::Exg);
        // ADDX/SUBX register and predecrement forms are not ADD/SUB.
        assert_eq!(classify(0xD181), InstructionClass::Addx);
        assert_eq!(classify(0x9181), InstructionClass::Subx);
        // TAS is not TST or an unassigned word.
        assert_eq!(classify(0x4AC0), InstructionClass::Tas);
        // ILLEGAL remains an exception instruction, not TAS.
        assert_eq!(classify(0x4AFC), InstructionClass::System);
        // Unassigned regions stay Other (guest illegal at execution).
        for opcode in [0xA000_u16, 0xF000, 0x7100, 0x4E74, 0x4E7A, 0x0E00] {
            assert_eq!(
                classify(opcode),
                InstructionClass::Other,
                "0x{opcode:04X} must stay unassigned"
            );
        }
    }
}
