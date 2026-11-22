//! Exception delivery through synthetic ROM handlers, including RTE return.

use std::error::Error;

use chronofork_megadrive::BusError;
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
fn reserved_tas_address_register_form_takes_vector_4() -> Result<(), Box<dyn Error>> {
    // TAS A0 is not a byte-addressable operand; it must raise the guest
    // illegal-instruction exception rather than executing as TAS on the address.
    let (mut cpu, mut bus) = machine(
        &[0x4A, 0xC8, 0x4E, 0x71],
        &[(4, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.set_a(0, 0x1234_5678);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(cpu.address_register(0), Some(0x1234_5678));
    assert_eq!(bus.read32(STACK - 4)?, 0x202);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
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
    assert_eq!(
        bus.read16(STACK - 2)?,
        0x0005,
        "function code: supervisor data write"
    );
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
fn exception_entry_wraps_the_stack_pointer_into_the_mirror() -> Result<(), Box<dyn Error>> {
    // W1K: the frame push is mod 2^32, so A7 = 0 pushes at 0xFFFF_FFFA, which
    // the 24-bit bus decodes as 0x00FF_FFFA — work RAM mirror space. The entry
    // is legal; the raw A7 keeps its wrapped value. (This test used to pin the
    // removed `StackOverflow` host guard.)
    let (mut cpu, mut bus) = machine(&[0x4E, 0x42], &[(VECTOR_TRAP_BASE + 2, HANDLER)], None)?;
    cpu.set_a7(0);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(cpu.address_register(7), Some(0xFFFF_FFFA));
    assert_eq!(bus.read16(0x00FF_FFFA)?, 0x2700, "SR in the frame");
    assert_eq!(bus.read32(0x00FF_FFFC)?, 0x0000_0202, "return PC");
    Ok(())
}

#[test]
fn exception_entry_with_an_unmapped_wrapped_frame_stays_a_typed_error() -> Result<(), Box<dyn Error>>
{
    // Truncation is still applied: A7 = 0x0180_0006 wraps the frame to
    // 0x0180_0000, which truncates to 0x0080_0000 — the hole between the
    // cartridge window and the Z80 RAM — so the push is a typed bus error and no
    // state is committed.
    let (mut cpu, mut bus) = machine(&[0x4E, 0x42], &[(VECTOR_TRAP_BASE + 2, HANDLER)], None)?;
    cpu.set_a7(0x0180_0006);
    let result = cpu.step(&mut bus);
    assert!(
        matches!(result, Err(CpuError::Bus(BusError::UnmappedWrite { .. }))),
        "an unmapped wrapped frame is a typed bus error: {result:?}"
    );
    assert_eq!(cpu.sr(), 0x2700, "SR untouched");
    assert_eq!(
        cpu.address_register(7),
        Some(0x0180_0006),
        "stack untouched"
    );
    assert_eq!(cpu.pc(), 0x202, "only the opcode word was fetched");
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

#[test]
fn unmapped_data_access_takes_vector_3_and_rte_returns() -> Result<(), Box<dyn Error>> {
    // MOVE.L (0x00800000).L,D0; NOP. Handler: RTE.
    let (mut cpu, mut bus) = machine(
        &[0x20, 0x39, 0x00, 0x80, 0x00, 0x00, 0x4E, 0x71],
        &[(3, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER, "guest vector 3, not a host bus error");
    assert_eq!(cpu.address_register(7), Some(STACK - 14));
    assert_eq!(bus.read16(STACK - 14)?, 0x2700, "SR");
    assert_eq!(bus.read32(STACK - 12)?, 0x206, "return PC");
    assert_eq!(bus.read32(STACK - 8)?, 0x0080_0000, "access address");
    assert_eq!(bus.read16(STACK - 4)?, 0x2039, "instruction register");
    assert_eq!(
        bus.read16(STACK - 2)?,
        0x0015,
        "function code: supervisor data read"
    );
    assert_eq!(cpu.data_register(0), Some(0), "no half-updated destination");
    cpu.step(&mut bus)?;
    assert_eq!(
        cpu.pc(),
        0x206,
        "RTE returns after the faulting instruction"
    );
    Ok(())
}

#[test]
fn unmapped_data_write_takes_vector_3_with_a_write_function_code() -> Result<(), Box<dyn Error>> {
    // MOVE.L D0,(0x00800000).L; NOP. Handler: RTE.
    let (mut cpu, mut bus) = machine(
        &[0x23, 0xC0, 0x00, 0x80, 0x00, 0x00, 0x4E, 0x71],
        &[(3, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(bus.read32(STACK - 8)?, 0x0080_0000);
    assert_eq!(bus.read16(STACK - 4)?, 0x23C0);
    assert_eq!(
        bus.read16(STACK - 2)?,
        0x0005,
        "function code: supervisor data write"
    );
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x206);
    Ok(())
}

#[test]
fn failed_frame_write_is_atomic_for_short_and_address_error_frames() -> Result<(), Box<dyn Error>> {
    // ILLEGAL with A7=0x0100_0002: the six-byte frame would cross RAM_END.
    let (mut cpu, mut bus) = machine(&[0x4A, 0xFC], &[(4, HANDLER)], None)?;
    cpu.set_a7(0x0100_0002);
    bus.write16(0x00FF_FFFC, 0xAAAA)?;
    let result = cpu.step(&mut bus);
    assert!(
        matches!(result, Err(CpuError::Bus(BusError::UnmappedWrite { .. }))),
        "a frame that does not fit is a typed host error: {result:?}"
    );
    assert_eq!(cpu.sr(), 0x2700, "SR untouched");
    assert_eq!(cpu.address_register(7), Some(0x0100_0002), "A7 untouched");
    assert_eq!(bus.read16(0x00FF_FFFC)?, 0xAAAA, "no partial frame write");

    // An odd word store raises an address error, but its fourteen-byte frame
    // also crosses RAM_END. The failed entry leaves no frame residue.
    let (mut cpu, mut bus) = machine(&[0x33, 0xC0, 0x00, 0xFF, 0xFF, 0xFF], &[(3, HANDLER)], None)?;
    cpu.set_a7(0x0100_0002);
    bus.write16(0x00FF_FFF4, 0xBBBB)?;
    let result = cpu.step(&mut bus);
    assert!(
        matches!(result, Err(CpuError::Bus(BusError::UnmappedWrite { .. }))),
        "a fourteen-byte frame that does not fit is a typed host error: {result:?}"
    );
    assert_eq!(cpu.sr(), 0x2700);
    assert_eq!(cpu.address_register(7), Some(0x0100_0002));
    assert_eq!(bus.read16(0x00FF_FFF4)?, 0xBBBB, "no partial frame write");
    Ok(())
}

#[test]
fn exception_frame_pushes_into_the_mirror() -> Result<(), Box<dyn Error>> {
    // The Kid Chameleon reset stack is 0xFFFFF7FE. Before the 24-bit
    // truncation and the `0xE00000..=0xFFFFFF` work RAM mirror this frame push
    // aborted the host; now it lands in work RAM and moves only the active
    // stack, while the raw 32-bit register value is never normalized.
    let (mut cpu, mut bus) = machine(&[0x4A, 0xFC], &[(4, HANDLER)], Some(&[0x4E, 0x73]))?;
    cpu.set_a7(0xFFFF_F7FE);
    // A pre-existing marker proves the mirror aliases the primary window's
    // block: A7-0xE00000 is the primary-window address of the same byte.
    bus.write8(0x00FF_F7F0, 0xAA)?;
    assert_eq!(bus.read8(0x00E0_F7F0)?, 0xAA, "mirror read");
    assert_eq!(bus.validate_write_range(0xFFFF_F7F8_u32, 6), Ok(()));
    assert_eq!(
        bus.validate_write_range(0xFFFF_FFF9_u32, 8),
        Err(BusError::UnmappedWrite {
            address: 0x0100_0000
        }),
        "a range whose end wraps the block is rejected, not wrapped"
    );

    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER, "the handler ran instead of aborting");
    assert_eq!(
        cpu.address_register(7),
        Some(0xFFFF_F7F8),
        "the active stack moved by the six-byte frame"
    );
    assert_eq!(cpu.a7(), 0xFFFF_F7F8);
    // The frame is visible through both the raw register value and the primary
    // window: SR word then return PC long, big-endian.
    assert_eq!(bus.read16(0xFFFF_F7F8)?, 0x2700, "SR at the stack pointer");
    assert_eq!(bus.read16(0x00FF_F7F8)?, 0x2700, "SR via the alias");
    assert_eq!(bus.read32(0xFFFF_F7FA)?, 0x0000_0202, "return PC");
    assert_eq!(bus.read8(0xFFFF_F7F0)?, 0xAA, "the marker is intact");

    // RTE returns to the instruction after ILLEGAL and pops the same frame.
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    assert_eq!(cpu.address_register(7), Some(0xFFFF_F7FE));
    Ok(())
}

#[test]
fn interrupt_frame_failure_is_atomic() -> Result<(), Box<dyn Error>> {
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71], &[(30, HANDLER)], None)?;
    cpu.set_a7(0x0100_0002);
    bus.write16(0x00FF_FFFC, 0xCCCC)?;
    let result = cpu.interrupt(6, 0x78, &mut bus);
    assert!(
        matches!(result, Err(CpuError::Bus(BusError::UnmappedWrite { .. }))),
        "an interrupt frame that does not fit is a typed host error: {result:?}"
    );
    assert_eq!(cpu.sr(), 0x2700);
    assert_eq!(cpu.address_register(7), Some(0x0100_0002));
    assert_eq!(bus.read16(0x00FF_FFFC)?, 0xCCCC);
    Ok(())
}

#[test]
fn user_mode_exception_uses_ssp_and_rte_restores_usp() -> Result<(), Box<dyn Error>> {
    let ssp = 0x00FF_2000;
    let usp = 0x00FF_3000;
    let (mut cpu, mut bus) = machine(
        &[0x4A, 0xFC, 0x4E, 0x71],
        &[(4, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.set_a7(ssp);
    cpu.set_sr(0x0700);
    cpu.set_a7(usp);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(cpu.a7(), ssp - 6, "the handler runs on SSP");
    assert_eq!(cpu.ssp(), ssp - 6);
    assert_eq!(cpu.usp(), usp, "the user stack is preserved");
    assert_eq!(bus.read16(ssp - 6)?, 0x0700, "user SR in the frame");
    assert_eq!(bus.read32(ssp - 4)?, 0x202);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x202);
    assert_eq!(cpu.a7(), usp, "RTE restores the user stack");
    assert_eq!(cpu.usp(), usp);
    assert_eq!(cpu.ssp(), ssp);
    assert_eq!(cpu.sr(), 0x0700);
    Ok(())
}

#[test]
fn user_mode_interrupt_uses_ssp_and_rte_restores_usp() -> Result<(), Box<dyn Error>> {
    let ssp = 0x00FF_2000;
    let usp = 0x00FF_3000;
    let (mut cpu, mut bus) = machine(&[0x4E, 0x71], &[(30, HANDLER)], Some(&[0x4E, 0x73]))?;
    cpu.set_a7(ssp);
    cpu.set_sr(0x0700);
    cpu.set_a7(usp);
    cpu.interrupt(6, 0x78, &mut bus)?;
    assert_eq!(cpu.pc(), HANDLER);
    assert_eq!(cpu.a7(), ssp - 6, "the handler runs on SSP");
    assert_eq!(cpu.usp(), usp);
    assert_eq!(bus.read16(ssp - 6)?, 0x0700);
    assert_eq!(bus.read32(ssp - 4)?, 0x200);
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), 0x200);
    assert_eq!(cpu.a7(), usp);
    assert_eq!(cpu.ssp(), ssp);
    assert_eq!(cpu.sr(), 0x0700);
    Ok(())
}

#[test]
fn write_protected_rom_takes_vector_3_with_the_documented_frame() -> Result<(), Box<dyn Error>> {
    // MOVE.L D0,(0x00000800).L targets the ROM window; the cartridge rejects the
    // write and the guest must see an address error, not a host bus error.
    let (mut cpu, mut bus) = machine(
        &[0x23, 0xC0, 0x00, 0x00, 0x08, 0x00, 0x4E, 0x71],
        &[(3, HANDLER)],
        Some(&[0x4E, 0x73]),
    )?;
    cpu.step(&mut bus)?;
    assert_eq!(cpu.pc(), HANDLER, "guest vector 3, not a host bus error");
    assert_eq!(cpu.address_register(7), Some(STACK - 14));
    assert_eq!(bus.read16(STACK - 14)?, 0x2700, "SR");
    assert_eq!(bus.read32(STACK - 12)?, 0x206, "return PC");
    assert_eq!(bus.read32(STACK - 8)?, 0x0000_0800, "access address");
    assert_eq!(bus.read16(STACK - 4)?, 0x23C0, "instruction register");
    assert_eq!(
        bus.read16(STACK - 2)?,
        0x0005,
        "function code: supervisor data write"
    );
    cpu.step(&mut bus)?;
    assert_eq!(
        cpu.pc(),
        0x206,
        "RTE returns after the faulting instruction"
    );
    Ok(())
}
