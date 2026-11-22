//! Control-flow coverage: BRA/BSR/Bcc/DBcc/Scc/JMP/JSR/RTS and the shared
//! condition-code predicate.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{Cartridge, M68000, MegadriveBus};

const STACK: u32 = 0x00FF_FF00;

fn machine(
    program: &[u8],
    vectors: &[(u8, u32)],
) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    for (vector, target) in vectors {
        let offset = usize::from(*vector) * 4;
        rom[offset..offset + 4].copy_from_slice(&target.to_be_bytes());
    }
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    Ok((cpu, bus))
}

#[test]
fn bcc_uses_the_shared_condition_predicate() -> Result<(), Box<dyn Error>> {
    // BEQ.S +4 with Z set and clear.
    let (mut cpu, mut bus) = machine(&[0x67, 0x04], &[])?;
    cpu.set_sr(0x2704);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x206);
    let (mut cpu, mut bus) = machine(&[0x67, 0x04], &[])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);

    // BGE.S: N == V is taken.
    let (mut cpu, mut bus) = machine(&[0x6C, 0x04], &[])?;
    cpu.set_sr(0x2708);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202, "N=1,V=0 is negative");
    let (mut cpu, mut bus) = machine(&[0x6C, 0x04], &[])?;
    cpu.set_sr(0x270A);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x206, "N=1,V=1 satisfies N == V");
    Ok(())
}

#[test]
fn bsr_pushes_the_return_address_and_bra_does_not() -> Result<(), Box<dyn Error>> {
    // BSR.S +4 to 0x206; RTS at 0x206.
    let (mut cpu, mut bus) = machine(&[0x61, 0x04, 0x4E, 0x71, 0x4E, 0x71, 0x4E, 0x75], &[])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x206);
    assert_eq!(cpu.address_register(7), Some(STACK - 4));
    assert_eq!(bus.read32(STACK - 4)?, 0x202, "return address after BSR");
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    assert_eq!(cpu.address_register(7), Some(STACK));
    Ok(())
}

#[test]
fn word_branch_forms_use_the_16_bit_extension_displacement() -> Result<(), Box<dyn Error>> {
    // A 68000 branch displacement is relative to the branch instruction's
    // address plus two -- the address of the 16-bit displacement word itself in
    // the word form. So BRA.W +4 from 0x200 targets 0x200 + 2 + 4 = 0x206, and
    // the instruction length stays four bytes. (This test used to pin the target
    // as 0x204 + 4 = 0x208, which is the two-bytes-too-far base the pinned
    // oracle rejected for BRA.W at 0x200.)
    let (mut cpu, mut bus) = machine(&[0x60, 0x00, 0x00, 0x04], &[])?;
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x206);
    assert_eq!(outcome.cycles, 10);

    // BNE.W taken with Z clear: 0x200 + 2 + 8 = 0x20A.
    let (mut cpu, mut bus) = machine(&[0x66, 0x00, 0x00, 0x08], &[])?;
    cpu.set_sr(0x2700);
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x20A);
    assert_eq!(outcome.cycles, 10);

    // BNE.W not taken with Z set advances past both words and costs 12; the
    // displacement is irrelevant on the not-taken path.
    let (mut cpu, mut bus) = machine(&[0x66, 0x00, 0x00, 0x08], &[])?;
    cpu.set_sr(0x2704);
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x204);
    assert_eq!(outcome.cycles, 12);

    // BSR.W targets 0x202 + 0x100 = 0x302 and pushes the address after the
    // extension word, which is unaffected by the relative base.
    let (mut cpu, mut bus) = machine(&[0x61, 0x00, 0x01, 0x00], &[])?;
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x302);
    assert_eq!(bus.read32(STACK - 4)?, 0x204);
    assert_eq!(outcome.cycles, 18);

    // A negative word displacement: BRA.W -2 from 0x200 targets 0x200 itself,
    // the two-byte instruction-plus-two base canceling the displacement.
    let (mut cpu, mut bus) = machine(&[0x60, 0x00, 0xFF, 0xFE], &[])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x200);
    Ok(())
}

#[test]
fn taken_dbcc_with_a_nonzero_counter_branches_from_instruction_plus_two()
-> Result<(), Box<dyn Error>> {
    // The defect this covers: with the post-extension PC as the base, every
    // taken DBcc landed two bytes past its target. DBRA D2,+4 at 0x200 targets
    // 0x200 + 2 + 4 = 0x206 and decrements D2 from 2 to 1.
    let (mut cpu, mut bus) = machine(&[0x51, 0xCA, 0x00, 0x04], &[])?;
    cpu.set_d(2, 2);
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(
        cpu.pc(),
        0x206,
        "taken DBcc target is instruction + 2 + disp"
    );
    assert_eq!(cpu.data_register(2), Some(1), "counter decremented");
    assert_eq!(outcome.cycles, 10);

    // A negative displacement loops back onto the instruction itself: the
    // self-loop idiom is disp -2, not -4.
    let (mut cpu, mut bus) = machine(&[0x51, 0xCA, 0xFF, 0xFE], &[])?;
    cpu.set_d(2, 3);
    for _ in 0..2 {
        cpu.step(&mut bus)?;
        assert_eq!(cpu.pc(), 0x200, "self-loop stays on the DBRA opcode");
    }
    assert_eq!(cpu.data_register(2), Some(1));
    // The third pass expires (1 -> 0 takes the branch, then 0 -> 0xFFFF falls
    // through), so loop until the counter reaches 0xFFFF.
    let mut guard = 0;
    while cpu.data_register(2) != Some(0xFFFF) {
        cpu.step(&mut bus)?;
        guard += 1;
        assert!(guard < 4, "DBRA did not expire");
    }
    assert_eq!(cpu.pc(), 0x204, "expiry falls through past both words");
    Ok(())
}

#[test]
fn byte_branch_forms_still_use_the_8_bit_displacement() -> Result<(), Box<dyn Error>> {
    // BNE.B taken with Z clear: target 0x202 + 2 = 0x204, cost 10.
    let (mut cpu, mut bus) = machine(&[0x66, 0x02], &[])?;
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x204);
    assert_eq!(outcome.cycles, 10);
    // BNE.B not taken with Z set: next instruction, cost 8.
    let (mut cpu, mut bus) = machine(&[0x66, 0x02], &[])?;
    cpu.set_sr(0x2704);
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    assert_eq!(outcome.cycles, 8);
    Ok(())
}

#[test]
fn dbcc_skips_without_decrementing_when_the_condition_is_true() -> Result<(), Box<dyn Error>> {
    // DBEQ D0,-2 with Z set: no decrement, no branch.
    let (mut cpu, mut bus) = machine(&[0x57, 0xC8, 0xFF, 0xFE], &[])?;
    cpu.set_d(0, 3);
    cpu.set_sr(0x2704);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(3));
    assert_eq!(cpu.pc(), 0x204);
    Ok(())
}

#[test]
fn dbcc_decrements_the_low_word_and_branches_while_not_minus_one() -> Result<(), Box<dyn Error>> {
    // DBRA D0,-2 from 1: instruction address plus two, minus two, is itself.
    let (mut cpu, mut bus) = machine(&[0x51, 0xC8, 0xFF, 0xFE], &[])?;
    cpu.set_d(0, 0x1234_0001);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x1234_0000));
    assert_eq!(cpu.pc(), 0x200);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0x1234_FFFF));
    assert_eq!(cpu.pc(), 0x204);
    Ok(())
}

#[test]
fn scc_writes_a_byte_truth_value() -> Result<(), Box<dyn Error>> {
    // SNE D0 with Z clear; SEQ D1 with Z set; SEQ D2 with Z clear.
    let (mut cpu, mut bus) = machine(&[0x56, 0xC0, 0x57, 0xC1, 0x57, 0xC2], &[])?;
    cpu.set_d(1, 0xAAAA_AAAA);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(0xFF));
    cpu.set_sr(0x2704);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(1), Some(0xAAAA_AAFF));
    cpu.set_sr(0x2700);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(2), Some(0));
    Ok(())
}

#[test]
fn jsr_pushes_the_return_address_and_rts_returns() -> Result<(), Box<dyn Error>> {
    // JSR (0x000300).L.
    let (mut cpu, mut bus) = machine(&[0x4E, 0xB9, 0x00, 0x00, 0x03, 0x00], &[])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x300);
    assert_eq!(bus.read32(STACK - 4)?, 0x206);

    // RTS with a return address already on the stack.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x75], &[])?;
    bus.write32(STACK - 4, 0x0000_0300)?;
    cpu.set_a(7, STACK - 4);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x300);
    assert_eq!(cpu.address_register(7), Some(STACK));
    Ok(())
}

#[test]
fn jmp_indirect_and_absolute_forms_set_the_program_counter() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0xD0], &[])?;
    cpu.set_a(0, 0x0000_0300);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x300);
    Ok(())
}

#[test]
fn jsr_odd_target_pushes_the_documented_address_error_frame() -> Result<(), Box<dyn Error>> {
    // JSR (0x000301).L with vector 3 pointing at 0x400.
    let (mut cpu, mut bus) = machine(&[0x4E, 0xB9, 0x00, 0x00, 0x03, 0x01], &[(3, 0x400)])?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x400);
    assert_ne!(
        cpu.sr() & 0x2000,
        0,
        "exception entry enters supervisor mode"
    );
    assert_eq!(cpu.address_register(7), Some(STACK - 14));
    assert_eq!(bus.read16(STACK - 14)?, 0x2700, "pushed SR");
    assert_eq!(bus.read32(STACK - 12)?, 0x206, "pushed return PC");
    assert_eq!(bus.read32(STACK - 8)?, 0x301, "faulting address");
    assert_eq!(bus.read16(STACK - 4)?, 0x4EB9, "instruction register");
    Ok(())
}

#[test]
fn flags_are_not_changed_by_control_instructions() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0xD0], &[])?;
    cpu.set_a(0, 0x300);
    cpu.set_sr(0x271F);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.sr(), 0x271F, "JMP changes no flags");
    Ok(())
}
