//! Master-cycle timeline integration: HBlank/VBlank boundaries and backend ticks.

#![allow(clippy::expect_used)]

use std::error::Error;

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::cycles::MASTER_CYCLES_PER_LINE;
use chronofork_megadrive::{
    InterruptLevel, MegadriveBackend, MegadriveFactory, SchedulerConfig, VideoStandard, demo_rom,
};

fn machine() -> Result<MegadriveBackend, Box<dyn Error>> {
    Ok(MegadriveFactory::new(demo_rom())?.create_machine()?)
}

fn pal_machine() -> Result<MegadriveBackend, Box<dyn Error>> {
    let factory = MegadriveFactory::with_scheduler_config(demo_rom(), SchedulerConfig::pal())?;
    Ok(factory.create_machine()?)
}

fn step_until(
    machine: &mut MegadriveBackend,
    predicate: impl Fn(&MegadriveBackend) -> bool,
    what: &str,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..2_000_000 {
        if predicate(machine) {
            return Ok(());
        }
        machine.step(InputFrame::NEUTRAL)?;
    }
    Err(format!("timed out waiting for {what}").into())
}

#[test]
fn hblank_falls_on_the_cycle_boundary() -> Result<(), Box<dyn Error>> {
    let mut machine = machine()?;
    step_until(
        &mut machine,
        |machine| machine.master_cycles() >= MASTER_CYCLES_PER_LINE,
        "first scanline",
    )?;
    assert_eq!(machine.interrupts().ticks(), machine.master_cycles());
    assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
    assert_eq!(
        machine
            .interrupts()
            .pending_events()
            .next()
            .expect("HBlank pending")
            .issued_tick(),
        MASTER_CYCLES_PER_LINE,
        "the request belongs to the 3420-cycle boundary, not the next instruction"
    );
    assert_eq!(
        machine.master_cycles(),
        machine.cpu().cycles() * 7,
        "canonical time is only the 68000 cycle count times seven"
    );
    Ok(())
}

#[test]
fn backend_advances_by_instruction_cycles_times_seven() -> Result<(), Box<dyn Error>> {
    let mut machine = machine()?;
    for _ in 0..20 {
        let before_master = machine.master_cycles();
        let before_cpu = machine.cpu().cycles();
        machine.step(InputFrame::NEUTRAL)?;
        let delta_cpu = machine.cpu().cycles() - before_cpu;
        assert!(delta_cpu > 0);
        assert_eq!(
            machine.master_cycles() - before_master,
            delta_cpu * 7,
            "each step advances by cycles * 7 master cycles"
        );
        assert_eq!(machine.tick().raw(), machine.master_cycles());
    }
    Ok(())
}

#[test]
fn ntsc_vblank_is_at_active_line_224_with_262_lines_per_frame() -> Result<(), Box<dyn Error>> {
    let mut machine = machine()?;
    let active = u64::from(VideoStandard::Ntsc.active_lines());
    step_until(
        &mut machine,
        |machine| machine.master_cycles() >= active * MASTER_CYCLES_PER_LINE,
        "NTSC VBlank",
    )?;
    assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
    assert_eq!(machine.interrupts().line_in_frame(), 224);
    assert_eq!(machine.interrupts().frame(), 0);
    step_until(
        &mut machine,
        |machine| machine.master_cycles() >= 262 * MASTER_CYCLES_PER_LINE,
        "first NTSC frame",
    )?;
    assert_eq!(machine.interrupts().frame(), 1);
    assert_eq!(machine.interrupts().line_in_frame(), 0);
    Ok(())
}

#[test]
fn pal_vblank_is_at_active_line_240_with_313_lines_per_frame() -> Result<(), Box<dyn Error>> {
    let mut machine = pal_machine()?;
    assert_eq!(machine.scheduler_config().standard(), VideoStandard::Pal);
    step_until(
        &mut machine,
        |machine| machine.master_cycles() >= 240 * MASTER_CYCLES_PER_LINE,
        "PAL VBlank",
    )?;
    assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
    assert_eq!(machine.interrupts().line_in_frame(), 240);
    step_until(
        &mut machine,
        |machine| machine.master_cycles() >= 313 * MASTER_CYCLES_PER_LINE,
        "first PAL frame",
    )?;
    assert_eq!(machine.interrupts().frame(), 1);
    assert_eq!(machine.interrupts().line_in_frame(), 0);
    Ok(())
}

#[test]
fn scheduler_config_is_video_only() {
    let ntsc = SchedulerConfig::ntsc();
    let pal = SchedulerConfig::pal();
    assert_eq!(ntsc.standard(), VideoStandard::Ntsc);
    assert_eq!(pal.standard(), VideoStandard::Pal);
    assert_eq!(ntsc.master_cycles_per_line(), 3420);
    assert_eq!(pal.master_cycles_per_line(), 3420);
    assert_eq!(SchedulerConfig::new(VideoStandard::Ntsc), ntsc);
    assert_eq!(SchedulerConfig::default(), ntsc);
}
