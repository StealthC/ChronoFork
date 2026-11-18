//! Cycle-table coverage: the documented timing anchors and EA costs.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{Cartridge, Ea, M68000, MegadriveBus, Size, base_cycles, ea_cycles};

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

fn step_cycles(
    program: &[u8],
    setup: impl FnOnce(&mut M68000, &mut MegadriveBus),
) -> Result<u16, Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(program, &[])?;
    setup(&mut cpu, &mut bus);
    let outcome = cpu.step(&mut bus)?;
    Ok(outcome.cycles)
}

#[test]
fn timing_anchors_match_the_manual() -> Result<(), Box<dyn Error>> {
    assert_eq!(step_cycles(&[0x4E, 0x71], |_, _| {})?, 4); // NOP
    assert_eq!(step_cycles(&[0x30, 0x00], |_, _| {})?, 4); // MOVE.W D0,D0
    assert_eq!(
        step_cycles(&[0x30, 0x10], |cpu, _| cpu.set_a(0, 0x00FF_0000))?,
        8,
        "MOVE.W (A0),D0: base 4 + EA 4"
    );
    assert_eq!(
        step_cycles(&[0x20, 0x3C, 0x12, 0x34, 0x56, 0x78], |_, _| {})?,
        12,
        "MOVE.L #imm,D0: base 4 + long immediate 8"
    );
    assert_eq!(step_cycles(&[0xD0, 0x41], |_, _| {})?, 4); // ADD.W D1,D0
    // The worse-case multiply anchor uses an all-ones source: MULU is
    // 38 + 2n where n counts the source's set bits.
    assert_eq!(
        step_cycles(&[0xC0, 0xC1], |cpu, _| cpu.set_d(1, 0xFFFF))?,
        70,
        "MULU D1,D0 with source 0xFFFF"
    );
    assert_eq!(
        step_cycles(&[0x80, 0xC1], |cpu, _| {
            cpu.set_d(0, 10);
            cpu.set_d(1, 2);
        })?,
        140,
        "DIVU D1,D0"
    );
    Ok(())
}

#[test]
fn taken_conditional_branches_cost_ten_cycles() -> Result<(), Box<dyn Error>> {
    // BNE.B taken (Z clear) costs 10; not taken (Z set) costs 8.
    assert_eq!(step_cycles(&[0x66, 0x02], |cpu, _| cpu.set_sr(0x2700))?, 10);
    assert_eq!(step_cycles(&[0x66, 0x02], |cpu, _| cpu.set_sr(0x2704))?, 8);
    // BNE.W taken costs 10; not taken costs 12.
    assert_eq!(
        step_cycles(&[0x66, 0x00, 0x00, 0x04], |cpu, _| cpu.set_sr(0x2700))?,
        10
    );
    assert_eq!(
        step_cycles(&[0x66, 0x00, 0x00, 0x04], |cpu, _| cpu.set_sr(0x2704))?,
        12
    );
    // BRA.B/BSR.B and their word forms are unconditional.
    assert_eq!(step_cycles(&[0x60, 0x02], |_, _| {})?, 10);
    assert_eq!(step_cycles(&[0x61, 0x02], |_, _| {})?, 18);
    Ok(())
}

#[test]
fn long_register_arithmetic_adds_two_cycles() -> Result<(), Box<dyn Error>> {
    // ADD/SUB/AND/OR.L with a Dn/An/immediate source cost 6 + 2, not 6.
    assert_eq!(step_cycles(&[0xD0, 0x81], |_, _| {})?, 8); // ADD.L D1,D0
    assert_eq!(step_cycles(&[0x90, 0x81], |_, _| {})?, 8); // SUB.L D1,D0
    assert_eq!(step_cycles(&[0xC0, 0x81], |_, _| {})?, 8); // AND.L D1,D0
    assert_eq!(step_cycles(&[0x80, 0x81], |_, _| {})?, 8); // OR.L D1,D0
    // A memory source keeps the plain long base plus the long EA time.
    assert_eq!(
        step_cycles(&[0xD0, 0x90], |cpu, _| cpu.set_a(0, 0x00FF_1000))?,
        14,
        "ADD.L (A0),D0: base 6 + long EA 8"
    );
    Ok(())
}

#[test]
fn read_modify_write_memory_forms_add_the_destination_ea() -> Result<(), Box<dyn Error>> {
    let address = |cpu: &mut M68000, _: &mut MegadriveBus| cpu.set_a(0, 0x00FF_1000);
    assert_eq!(step_cycles(&[0xD1, 0x50], address)?, 12); // ADD.W D1,(A0)
    assert_eq!(step_cycles(&[0xD1, 0x90], address)?, 20); // ADD.L D1,(A0)
    assert_eq!(step_cycles(&[0x91, 0x50], address)?, 12); // SUB.W D1,(A0)
    assert_eq!(step_cycles(&[0xC1, 0x50], address)?, 12); // AND.W D1,(A0)
    assert_eq!(step_cycles(&[0x81, 0x50], address)?, 12); // OR.W D1,(A0)
    assert_eq!(step_cycles(&[0xB1, 0x50], address)?, 12); // EOR.W D1,(A0)
    Ok(())
}

#[test]
fn quick_arithmetic_to_memory_adds_the_destination_ea() -> Result<(), Box<dyn Error>> {
    let address = |cpu: &mut M68000, _: &mut MegadriveBus| cpu.set_a(0, 0x00FF_1000);
    assert_eq!(step_cycles(&[0x52, 0x50], address)?, 12); // ADDQ.W #1,(A0)
    assert_eq!(step_cycles(&[0x52, 0x90], address)?, 20); // ADDQ.L #1,(A0)
    assert_eq!(step_cycles(&[0x53, 0x50], address)?, 12); // SUBQ.W #1,(A0)
    Ok(())
}

#[test]
fn test_clear_negate_and_not_use_documented_base_times() -> Result<(), Box<dyn Error>> {
    assert_eq!(step_cycles(&[0x4A, 0x80], |_, _| {})?, 4); // TST.L D0
    assert_eq!(step_cycles(&[0x42, 0x80], |_, _| {})?, 6); // CLR.L D0
    assert_eq!(step_cycles(&[0x42, 0x40], |_, _| {})?, 4); // CLR.W D0
    assert_eq!(step_cycles(&[0x44, 0x80], |_, _| {})?, 6); // NEG.L D0
    let address = |cpu: &mut M68000, _: &mut MegadriveBus| cpu.set_a(0, 0x00FF_1000);
    assert_eq!(step_cycles(&[0x42, 0x90], address)?, 20); // CLR.L (A0)
    assert_eq!(step_cycles(&[0x46, 0x50], address)?, 12); // NOT.W (A0)
    Ok(())
}

#[test]
fn jump_and_lea_use_control_addressing_times() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        step_cycles(&[0x4E, 0xF9, 0x00, 0x00, 0x03, 0x12], |_, _| {})?,
        12,
        "JMP (abs).L"
    );
    assert_eq!(
        step_cycles(&[0x4E, 0xB9, 0x00, 0x00, 0x03, 0x12], |_, _| {})?,
        20,
        "JSR (abs).L"
    );
    assert_eq!(
        step_cycles(&[0x4E, 0x90], |cpu, _| cpu.set_a(0, 0x0000_0300))?,
        16,
        "JSR (A0)"
    );
    assert_eq!(
        step_cycles(&[0x4E, 0xD0], |cpu, _| cpu.set_a(0, 0x0000_0300))?,
        8,
        "JMP (A0)"
    );
    assert_eq!(
        step_cycles(&[0x47, 0xF9, 0x00, 0x00, 0x03, 0x12], |_, _| {})?,
        12,
        "LEA (abs).L,A3"
    );
    assert_eq!(
        step_cycles(&[0x48, 0x79, 0x00, 0x00, 0x03, 0x12], |_, _| {})?,
        20,
        "PEA (abs).L"
    );
    assert_eq!(
        step_cycles(&[0x47, 0xD0], |cpu, _| cpu.set_a(0, 0x0000_0300))?,
        4,
        "LEA (A0),A3"
    );
    assert_eq!(
        step_cycles(&[0x48, 0x50], |cpu, _| cpu.set_a(0, 0x0000_0300))?,
        12,
        "PEA (A0)"
    );
    Ok(())
}

#[test]
fn movem_cost_depends_on_the_mask_and_direction() -> Result<(), Box<dyn Error>> {
    let address = |cpu: &mut M68000, _: &mut MegadriveBus| cpu.set_a(0, 0x00FF_1000);
    // MOVEM.L A7/A6/A5,-(A0): predecrement long is 8 + 8 per register.
    assert_eq!(step_cycles(&[0x48, 0xE0, 0x00, 0x07], address)?, 32);
    // MOVEM.L (A0)+,D3/D4/D5: postincrement long is 12 + 8 per register.
    assert_eq!(step_cycles(&[0x4C, 0xD8, 0x00, 0x38], address)?, 36);
    // Word forms: predecrement 8 + 4 per register, postincrement 12 + 4.
    assert_eq!(step_cycles(&[0x48, 0xA0, 0x00, 0x03], address)?, 16);
    assert_eq!(step_cycles(&[0x4C, 0x98, 0x00, 0x03], address)?, 20);
    Ok(())
}

#[test]
fn register_shift_cost_includes_twice_the_count() -> Result<(), Box<dyn Error>> {
    assert_eq!(step_cycles(&[0xE3, 0x48], |_, _| {})?, 8); // LSL.W #1,D0
    assert_eq!(
        step_cycles(&[0xE2, 0x68], |cpu, _| cpu.set_d(1, 2))?,
        10,
        "LSR.W D1,D0 with count 2"
    );
    assert_eq!(step_cycles(&[0xE5, 0x88], |_, _| {})?, 12); // LSL.L #2,D0
    assert_eq!(step_cycles(&[0xE0, 0x80], |_, _| {})?, 24); // ASR.L #8,D0
    assert_eq!(step_cycles(&[0xE3, 0x98], |_, _| {})?, 10); // ROL.L #1,D0
    let address = |cpu: &mut M68000, _: &mut MegadriveBus| cpu.set_a(0, 0x00FF_1000);
    assert_eq!(step_cycles(&[0xE2, 0xD8], address)?, 12); // LSR.W (A0)+
    assert_eq!(step_cycles(&[0xE2, 0xD0], address)?, 12); // LSR.W (A0)
    Ok(())
}

#[test]
fn multiply_uses_early_out_and_divide_uses_worst_case() -> Result<(), Box<dyn Error>> {
    // MULU: 38 + 2 * popcount(source). 3 has two set bits; 0xFFFF has sixteen.
    assert_eq!(
        step_cycles(&[0xC0, 0xC1], |cpu, _| cpu.set_d(1, 3))?,
        42,
        "MULU D1,D0 with source 3"
    );
    assert_eq!(
        step_cycles(&[0xC0, 0xC1], |cpu, _| cpu.set_d(1, 0xFFFF))?,
        70,
        "MULU D1,D0 with source 0xFFFF"
    );
    // MULS: 38 + 2 * bit transitions from an assumed leading zero. 9 makes
    // four transitions (0->1, 1->0, 0->1, 1->0).
    assert_eq!(
        step_cycles(&[0xC1, 0xC2], |cpu, _| cpu.set_d(2, 9))?,
        46,
        "MULS D2,D0 with source 9"
    );
    assert_eq!(
        step_cycles(&[0x80, 0xC1], |cpu, _| {
            cpu.set_d(0, 100);
            cpu.set_d(1, 7);
        })?,
        140,
        "DIVU D1,D0"
    );
    assert_eq!(
        step_cycles(&[0x81, 0xC1], |cpu, _| {
            cpu.set_d(0, 100);
            cpu.set_d(1, 7);
        })?,
        168,
        "DIVS D1,D0"
    );
    Ok(())
}

#[test]
fn ea_cost_is_added_exactly_once_per_operand() -> Result<(), Box<dyn Error>> {
    // MOVE.W D0,(A0) pays only the destination EA.
    let cycles = step_cycles(&[0x30, 0x80], |cpu, _| cpu.set_a(0, 0x00FF_0000))?;
    assert_eq!(cycles, 8);
    // MOVE.W (A0),(A1) pays both EAs.
    let cycles = step_cycles(&[0x32, 0x90], |cpu, _| {
        cpu.set_a(0, 0x00FF_0000);
        cpu.set_a(1, 0x00FF_0010);
    })?;
    assert_eq!(cycles, 12);
    Ok(())
}

#[test]
fn ea_cycles_match_the_documented_table() {
    assert_eq!(ea_cycles(&Ea::Dn(0), Size::Word, true), 0);
    assert_eq!(ea_cycles(&Ea::An(0), Size::Long, true), 0);
    assert_eq!(ea_cycles(&Ea::Indirect(0), Size::Word, true), 4);
    assert_eq!(ea_cycles(&Ea::Indirect(0), Size::Long, true), 8);
    assert_eq!(ea_cycles(&Ea::PostInc(0), Size::Byte, false), 4);
    assert_eq!(ea_cycles(&Ea::PreDec(0), Size::Word, false), 6);
    assert_eq!(ea_cycles(&Ea::PreDec(0), Size::Long, false), 10);
    assert_eq!(ea_cycles(&Ea::Disp16(0), Size::Word, true), 8);
    assert_eq!(ea_cycles(&Ea::Index(0), Size::Long, true), 14);
    assert_eq!(ea_cycles(&Ea::Abs16, Size::Word, true), 8);
    assert_eq!(ea_cycles(&Ea::Abs32, Size::Long, true), 16);
    assert_eq!(ea_cycles(&Ea::PcDisp16, Size::Word, true), 8);
    assert_eq!(ea_cycles(&Ea::PcIndex, Size::Long, true), 14);
    assert_eq!(ea_cycles(&Ea::Imm, Size::Word, true), 4);
    assert_eq!(ea_cycles(&Ea::Imm, Size::Long, true), 8);
}

#[test]
fn base_cycles_are_available_for_implemented_instructions() {
    assert_eq!(base_cycles(0x4E71), Some(4));
    assert_eq!(base_cycles(0x3000), Some(4));
    assert_eq!(base_cycles(0xC0C1), Some(38), "MULU early-out base");
    assert_eq!(base_cycles(0x80C1), Some(140));
    assert_eq!(base_cycles(0x81C1), Some(168), "DIVS worst-case base");
    assert_eq!(base_cycles(0xFFFF), None);
}

#[test]
fn cycles_accumulate_on_the_processor() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71, 0x4E, 0x71], &[])?;
    let first = cpu.step(&mut bus)?;
    assert_eq!(first.cycles, 4);
    assert_eq!(cpu.cycles(), 4);
    let second = cpu.step(&mut bus)?;
    assert_eq!(second.cycles, 4);
    assert_eq!(cpu.cycles(), 8);
    Ok(())
}

#[test]
fn exception_entry_reports_thirty_four_cycles_through_step() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4A, 0xFC], &[(4, 0x400)])?;
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(outcome.cycles, 34);
    assert_eq!(cpu.pc(), 0x400);
    assert_eq!(cpu.cycles(), 34);
    Ok(())
}
