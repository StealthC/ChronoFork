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
    assert_eq!(step_cycles(&[0xC0, 0xC1], |_, _| {})?, 70); // MULU D1,D0
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
    assert_eq!(base_cycles(0xC0C1), Some(70));
    assert_eq!(base_cycles(0x80C1), Some(140));
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
