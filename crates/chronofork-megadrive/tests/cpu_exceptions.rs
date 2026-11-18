//! Exception delivery through synthetic ROM handlers, including RTE return.

use std::error::Error;

use chronofork_megadrive::test_support::stamp_header;
use chronofork_megadrive::{
    Cartridge, CpuError, ExceptionFrame, M68000, MegadriveBus, VECTOR_TRAP_BASE,
};

const STACK: u32 = 0x00FF_FF00;
const HANDLER: u32 = 0x400;

fn machine(
    program: &[u8],
    vectors: &[(u8, u32)],
    handler: Option<&[u8]>,
) -> Result<(M68000, MegadriveBus), Box<dyn Error>> {
    let mut rom = [0x4E_u8, 0x71].repeat(0x1000);
    rom[0..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    for (vector, target) in vectors {
        let offset = usize::from(*vector) * 4;
        rom[offset..offset + 4].copy_from_slice(&target.to_be_bytes());
    }
    if let Some(handler) = handler {
        let start = HANDLER as usize;
        rom[start..start + handler.len()].copy_from_slice(handler);
    }
    assert!(stamp_header(&mut rom, "JUE"), "ROM holds a valid header");
    let mut bus = MegadriveBus::new(Cartridge::parse(rom)?);
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;
    Ok((cpu, bus))
}

#[test]
fn illegal_opcode_takes_vector_4_and_rte_returns() -> Result<(), Box<dyn Error>> {
    // ILLEGAL; MOVEQ #1,D0; BRA.S self. Handler: RTE.
    let (mut cpu, mut bus) = machine(
        &[0x4A, 0xFC, 0x70, 0x01, 0x60, 0xFE],
        &[(4, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(cpu.address_register(7), Some(STACK - 6));
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.data_register(0), Some(1));
    Ok(())
}

#[test]
fn unrecognised_opcode_takes_vector_4_and_rte_returns() -> Result<(), Box<dyn Error>> {
    // 0xA000 is an unassigned line-A encoding; NOP follows. Handler: RTE.
    let (mut cpu, mut bus) = machine(
        &[0xA0, 0x00, 0x4E, 0x71],
        &[(4, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    let outcome = cpu.step(&mut bus)?;
    assert_eq!(outcome.cycles, 34, "exception entry cost");
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(cpu.address_register(7), Some(STACK - 6));
    assert_eq!(bus.read16(STACK - 6)?, 0x2700, "short frame SR");
    assert_eq!(bus.read32(STACK - 4)?, 0x202, "return PC after the opcode");
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202, "RTE returns after the illegal opcode");
    Ok(())
}

#[test]
fn reserved_movem_destination_takes_vector_4() -> Result<(), Box<dyn Error>> {
    // MOVEM.L D0,(A0)+ is not a documented destination form. The mask word is
    // consumed before the reserved form is rejected, so the return PC is 0x204.
    let (mut cpu, mut bus) = machine(
        &[0x48, 0xD8, 0x00, 0x01, 0x4E, 0x71],
        &[(4, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(bus.read32(STACK - 4)?, 0x204);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x204);
    Ok(())
}

#[test]
fn privilege_violation_reaches_a_user_written_handler() -> Result<(), Box<dyn Error>> {
    // NOP; MOVE.W #0,SR in user mode; handler RTE.
    let (mut cpu, mut bus) = machine(
        &[0x4E, 0x71, 0x46, 0xFC, 0x00, 0x00],
        &[(8, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.set_sr(0x0700);
    cpu.set_a7(STACK);
    cpu.step(&mut bus)?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x204, "RTE returns after the privileged opcode");
    assert_eq!(cpu.sr() & 0x00FF, 0x00);
    Ok(())
}

#[test]
fn zero_divide_reaches_its_handler_and_returns() -> Result<(), Box<dyn Error>> {
    // DIVU D0,D1 with D0=0; NOP. Handler RTE.
    let (mut cpu, mut bus) = machine(
        &[0x82, 0xC0, 0x4E, 0x71],
        &[(5, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.set_d(0, 0);
    cpu.set_d(1, 0x0001_0000);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    Ok(())
}

#[test]
fn trap_reaches_vector_32_plus_n_and_returns() -> Result<(), Box<dyn Error>> {
    let trap_two = VECTOR_TRAP_BASE + 2;
    let (mut cpu, mut bus) = machine(
        &[0x4E, 0x42, 0x4E, 0x71],
        &[(trap_two, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    Ok(())
}

#[test]
fn interrupt_entry_and_rte_round_trip() -> Result<(), Box<dyn Error>> {
    // Level 6 autovector is vector 30 at address 0x78.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71], &[(30, HANDLER)], Some(&[0x4E, 0x73]))?;
    cpu.step(&mut bus)?;
    cpu.interrupt(6, 0x78, &mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    Ok(())
}

#[test]
fn address_error_pushes_the_documented_fourteen_byte_frame() -> Result<(), Box<dyn Error>> {
    // MOVE.W D0,(0x00FF0001).L is an odd word store.
    let (mut cpu, mut bus) = machine(
        &[0x33, 0xC0, 0x00, 0xFF, 0x00, 0x01, 0x4E, 0x71],
        &[(3, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(cpu.address_register(7), Some(STACK - 14));
    assert_eq!(bus.read16(STACK - 14)?, 0x2700, "SR");
    assert_eq!(bus.read32(STACK - 12)?, 0x206, "return PC");
    assert_eq!(bus.read32(STACK - 8)?, 0x00FF_0001, "access address");
    assert_eq!(bus.read16(STACK - 4)?, 0x33C0, "instruction register");
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x206);
    Ok(())
}

#[test]
fn exception_entry_reports_its_documented_cycle_cost() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71], &[(4, HANDLER)], None)?;
    let cycles = cpu.raise_exception(&mut bus, 4, ExceptionFrame::Short)?;
    assert_eq!(cycles, 34);
    Ok(())
}

#[test]
fn failed_frame_access_is_a_host_error_without_half_updating_state() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0x42], &[(VECTOR_TRAP_BASE + 2, HANDLER)], None)?;
    cpu.set_a7(0);
    let result = cpu.step(&mut bus);
    assert!(matches!(result, Err(CpuError::StackOverflow)));
    assert_eq!(cpu.sr(), 0x2700, "SR untouched");
    assert_eq!(cpu.address_register(7), Some(0), "stack untouched");
    Ok(())
}

#[test]
fn an_odd_vector_target_is_a_typed_host_error() -> Result<(), Box<dyn Error>> {
    // Leave vector 34 at the odd ROM fill value.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x42], &[], None)?;
    let result = cpu.step(&mut bus);
    assert!(
        matches!(result, Err(CpuError::AddressError { .. })),
        "odd vector target must stay a host error"
    );
    assert_eq!(cpu.address_register(7), Some(STACK), "no frame pushed");
    assert_eq!(cpu.sr(), 0x2700);
    Ok(())
}
