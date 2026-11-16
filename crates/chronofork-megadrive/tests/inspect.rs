//! Acceptance coverage for the ADR-0010 §3.3 Mega Drive inspectors.
//!
//! These tests implement the library half of rows M4-A6, M4-A13, M4-A14 and
//! M4-A15 in `docs/quality/m4-debugging-depth-acceptance.md`: every decoded view
//! field is compared with the direct CPU, VDP, scheduler or bus accessor it came
//! from, and constructing a view is proven not to change machine state.
//!
//! Only the public crate API is used, exactly like the CLI that consumes it.

use std::error::Error;
use std::sync::Arc;

use chronofork_core::hash_bytes;
use chronofork_megadrive::{
    BusView, CONTROLLER_WINDOW_BASE, CONTROLLER_WINDOW_END, CpuView, DEFAULT_INSTRUCTIONS_PER_LINE,
    InterruptLevel, InterruptScheduler, InterruptView, M68000, MegadriveBus, MegadriveFactory,
    PendingInterruptView, RAM_BASE, RAM_END, RAM_SIZE, ROM_BASE, ROM_END, SchedulerConfig,
    VDP_BASE, VDP_END, Vdp, VdpView, VideoStandard, demo_rom,
    vdp::{AUTO_INCREMENT_REGISTER, CONTROL_PORT_OFFSET, DATA_PORT_OFFSET},
};

/// Test result type: `?` keeps the target free of unwrap/expect helpers.
type TestResult = Result<(), Box<dyn Error>>;

/// Builds a ROM whose reset vectors load `program` at address 0x200.
fn program_rom(program: &[u8]) -> Vec<u8> {
    let mut rom = vec![0_u8; 0x400];
    rom[0..4].copy_from_slice(&0x00FF_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    rom
}

#[test]
fn cpu_view_decodes_sr_fields() -> TestResult {
    // MOVE.W #$A71F,SR sets T, S and every condition code; the second
    // instruction clears everything but S so each decode is observed both ways.
    let rom = program_rom(&[
        0x46, 0xFC, 0xA7, 0x1F, // MOVE.W #$A71F,SR
        0x46, 0xFC, 0x20, 0x00, // MOVE.W #$2000,SR
    ]);
    let mut bus = MegadriveBus::new(Arc::from(rom.into_boxed_slice()));
    let mut cpu = M68000::new();
    cpu.reset(&mut bus)?;

    // Reset state: supervisor, mask 7, no trace bit and no condition codes.
    let view = CpuView::new(&cpu);
    assert_eq!(view.pc(), cpu.pc());
    assert_eq!(view.sr(), cpu.sr());
    assert_eq!(view.halted(), cpu.halted());
    assert_eq!(view.supervisor(), cpu.sr() & 0x2000 != 0);
    assert_eq!(view.trace(), cpu.sr() & 0x8000 != 0);
    assert_eq!(view.interrupt_mask(), cpu.interrupt_priority_mask());
    assert_eq!(view.interrupt_mask(), 7);
    assert!(view.supervisor());
    assert!(!view.trace());
    assert!(!view.x() && !view.n() && !view.z() && !view.v() && !view.c());

    cpu.step(&mut bus)?;
    let view = CpuView::new(&cpu);
    assert_eq!(view.pc(), 0x204);
    assert_eq!(view.pc(), cpu.pc());
    assert_eq!(view.sr(), 0xA71F);
    assert_eq!(view.halted(), cpu.halted());
    assert_eq!(view.interrupt_mask(), cpu.interrupt_priority_mask());
    assert_eq!(view.interrupt_mask(), 7);
    assert!(view.supervisor());
    assert!(view.trace());
    assert!(view.x() && view.n() && view.z() && view.v() && view.c());

    cpu.step(&mut bus)?;
    let view = CpuView::new(&cpu);
    assert_eq!(view.pc(), 0x208);
    assert_eq!(view.sr(), 0x2000);
    assert_eq!(view.interrupt_mask(), 0);
    assert!(view.supervisor());
    assert!(!view.trace());
    assert!(!view.x() && !view.n() && !view.z() && !view.v() && !view.c());

    // Reading a view is not a mutation: the register map is unchanged.
    let registers = cpu.registers();
    let _first = CpuView::new(&cpu);
    let _second = CpuView::new(&cpu);
    assert_eq!(cpu.registers(), registers);
    Ok(())
}

#[test]
fn vdp_view_matches_direct_device_accessors() -> TestResult {
    let mut vdp = Vdp::new();
    // Auto-increment 2, then write one VRAM word at address 0.
    vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0x4000)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0x0000)?;
    vdp.write16(DATA_PORT_OFFSET, 0x1234)?;
    vdp.set_vblank(true);
    vdp.set_vint_pending(true);

    let before = vdp.snapshot_bytes();
    let view = VdpView::new(&vdp);
    let again = VdpView::new(&vdp);
    assert_eq!(view, again, "view construction is deterministic");

    assert_eq!(view.status(), vdp.status());
    assert_eq!(view.vblank(), vdp.status() & 0x0008 != 0);
    assert_eq!(view.vint_pending(), vdp.vint_pending());
    assert!(view.vblank());
    assert!(view.vint_pending());
    assert_eq!(view.address(), vdp.address());
    assert_eq!(view.address(), 2, "auto-increment advanced the address");
    assert_eq!(
        view.auto_increment(),
        vdp.registers()[AUTO_INCREMENT_REGISTER]
    );
    assert_eq!(view.auto_increment(), 2);
    assert_eq!(view.registers(), vdp.registers());
    assert_eq!(view.vram_len(), vdp.vram().len());
    assert_eq!(view.vram_sha256(), hash_bytes(vdp.vram()));
    assert_eq!(&vdp.vram()[..2], &[0x12, 0x34][..]);

    // The view is derived data only: the VDP snapshot is byte-identical.
    assert_eq!(vdp.snapshot_bytes(), before);
    Ok(())
}

#[test]
fn interrupt_view_matches_scheduler_state() -> TestResult {
    let config = SchedulerConfig::ntsc();
    assert_eq!(
        config.instructions_per_line(),
        DEFAULT_INSTRUCTIONS_PER_LINE
    );
    let mut scheduler = InterruptScheduler::new(config);

    // One line raises HBlank, a second coalesces into it, and the active-region
    // end raises VBlank while HBlank stays pending.
    let quantum = u64::from(config.instructions_per_line());
    scheduler.advance(quantum)?;
    scheduler.advance(quantum)?;
    scheduler.advance(quantum * u64::from(config.active_lines() - 2))?;

    let before = scheduler.snapshot_bytes();
    let view = InterruptView::new(&scheduler, 0x2700);

    assert_eq!(view.standard(), VideoStandard::Ntsc);
    assert_eq!(view.standard(), config.standard());
    assert_eq!(view.instructions_per_line(), config.instructions_per_line());
    assert_eq!(view.lines_per_frame(), config.lines_per_frame());
    assert_eq!(view.active_lines(), config.active_lines());
    assert_eq!(view.ticks(), scheduler.ticks());
    assert_eq!(view.frame(), scheduler.frame());
    assert_eq!(view.line(), scheduler.line_in_frame());
    assert_eq!(view.quantum(), scheduler.quantum_in_line());
    assert_eq!(view.line(), config.active_lines());
    assert_eq!(view.quantum(), 0);

    // Pending requests equal the scheduler's, in delivery-priority order.
    assert_eq!(view.pending().len(), scheduler.pending_len());
    for (entry, event) in view.pending().iter().zip(scheduler.pending_events()) {
        assert_eq!(entry.level(), event.level());
        assert_eq!(entry.issued_tick(), event.issued_tick());
    }
    assert_eq!(view.pending().len(), 2);
    assert_eq!(view.pending()[0].level(), InterruptLevel::VBlank);
    assert_eq!(
        view.pending()[0].issued_tick(),
        quantum * u64::from(config.active_lines())
    );
    assert_eq!(view.pending()[1].level(), InterruptLevel::HBlank);
    assert_eq!(view.pending()[1].issued_tick(), quantum);

    // Per-level counters equal the scheduler accessors, in ALL order.
    for counter in view.counters() {
        assert_eq!(counter.issued(), scheduler.requests_issued(counter.level()));
        assert_eq!(
            counter.coalesced(),
            scheduler.requests_coalesced(counter.level())
        );
    }
    assert_eq!(view.counters()[0].level(), InterruptLevel::HBlank);
    assert_eq!(
        view.counters()[0].issued(),
        u64::from(config.active_lines())
    );
    assert_eq!(
        view.counters()[0].coalesced(),
        u64::from(config.active_lines() - 1)
    );
    assert_eq!(view.counters()[1].level(), InterruptLevel::VBlank);
    assert_eq!(view.counters()[1].issued(), 1);
    assert_eq!(view.counters()[1].coalesced(), 0);

    // The deliverable flag applies the 68000 mask rule against the supplied SR.
    assert!(view.pending().iter().all(|entry| !entry.deliverable()));
    let open = InterruptView::new(&scheduler, 0x2000);
    assert!(open.pending().iter().all(PendingInterruptView::deliverable));
    let mixed = InterruptView::new(&scheduler, 0x2400);
    assert_eq!(mixed.pending()[0].level(), InterruptLevel::VBlank);
    assert!(mixed.pending()[0].deliverable());
    assert_eq!(mixed.pending()[1].level(), InterruptLevel::HBlank);
    assert!(!mixed.pending()[1].deliverable());

    // Every view is read-only: the scheduler snapshot did not change.
    assert_eq!(scheduler.snapshot_bytes(), before);
    Ok(())
}

#[test]
fn bus_view_lists_the_mapped_windows() -> TestResult {
    let rom = vec![0x4E_u8; 0x1000];
    let bus = MegadriveBus::new(Arc::from(rom.into_boxed_slice()));
    let view = BusView::new(&bus);

    assert_eq!(view.rom_len(), bus.rom().len());
    assert_eq!(view.rom_len(), 0x1000);
    assert_eq!(view.ram_len(), bus.ram().len());
    assert_eq!(view.ram_len(), RAM_SIZE);

    let windows = view.windows();
    assert_eq!(windows.len(), 4);
    let expected = [
        ("rom", ROM_BASE, ROM_END, false),
        (
            "controller",
            CONTROLLER_WINDOW_BASE,
            CONTROLLER_WINDOW_END,
            false,
        ),
        ("vdp", VDP_BASE, VDP_END, true),
        ("ram", RAM_BASE, RAM_END, true),
    ];
    for (window, (name, start, end, writable)) in windows.iter().zip(expected) {
        assert_eq!(window.name(), name);
        assert_eq!(window.start(), start, "{name} start");
        assert_eq!(window.end(), end, "{name} exclusive end");
        assert_eq!(window.writable(), writable, "{name} writability");
        assert!(window.start() < window.end(), "{name} must not be empty");
    }
    // The table is ordered and the half-open windows do not overlap.
    assert!(
        windows
            .windows(2)
            .all(|pair| pair[0].end() <= pair[1].start())
    );

    // The backend exposes the same bus read-only, so a caller can build the view
    // without reaching into private state.
    let expected_rom_len = demo_rom().len();
    let machine = MegadriveFactory::new(demo_rom())?.create_machine()?;
    assert_eq!(machine.bus().rom().len(), expected_rom_len);
    assert_eq!(machine.bus().ram().len(), RAM_SIZE);
    let machine_view = BusView::new(machine.bus());
    assert_eq!(machine_view.rom_len(), expected_rom_len);
    assert_eq!(machine_view.windows(), windows);
    Ok(())
}
