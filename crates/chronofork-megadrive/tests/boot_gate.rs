//! G-BOOT-0 and G-BOOT-1 boot gates (phase W1.6).
//!
//! Two independent gates, both positive and both reproducible:
//!
//! - **G-BOOT-0 — host stability.** A run must complete exactly the requested
//!   number of steps for the gate to pass. **Any early stop fails the gate**,
//!   including a stop caused by a typed, known blocker: a typed rejection is
//!   evidence that the ROM cannot boot yet, not a pass. A guest exception the
//!   ROM's own handler services is not an early stop; it is input to G-BOOT-1.
//! - **G-BOOT-1 — observable bootstrap markers.** Per ROM, the gate requires a
//!   positive marker set, never "registers are not zero": the reset vector's
//!   init PC is the pinned one, the VDP register file leaves its reset value, at
//!   least one VRAM/CRAM/VSRAM write reaches the data port, the PC leaves the
//!   init region and no constant-PC window (`PC_RUN_WINDOW` steps) ever occurs,
//!   and the ROM's own exception handler is never entered.
//!
//! The synthetic controls run in the default suite and are mandatory: the
//! `BRA *` negative control must fail G-BOOT-1, a synthetic ROM that programs a
//! 68000-memory DMA must fail G-BOOT-0 with that typed blocker named, and a
//! synthetic `TST.L $00A10008` / `TST.L $00A1000C` bootstrap must pass both
//! gates (the regression the W1 bus work exists to prevent).
//!
//! The two commercial fixtures are local-only (`.research/roms/`, never
//! committed) and the fixture gate needs 300 000 instruction steps per fixture,
//! which is only practical in a release build. The fixture half of the gate is
//! therefore **opt-in**: set `CHRONOFORK_BOOT_GATE=1` and run
//! `cargo test --release -p chronofork-megadrive --test boot_gate`. Without the
//! switch the fixture test skips with an explicit reason — it never reports the
//! gate as passing by absence, and a fixture-gate run with **zero** fixtures
//! present is a hard failure.
//!
//! # History of the measured profile (the numbers live in the evidence, not here)
//!
//! The gate was written when both fixtures stopped early (93 steps) and the run
//! was red; each blocker it named was then reproduced, attributed and fixed in
//! the emulator: the 16-bit branch-displacement base (`w1d.md` §3), the VDP DMA
//! length-minus-one encoding (`w1g-gate.md` §3.2), a 68000 predecrement that
//! aborted the host instead of decrementing modulo 2^32, the VSRAM command codes,
//! and the port 3/EXT data store. At the W1 freeze both fixtures complete the
//! 300 000-step budget with no host error and pass every marker below; the
//! measured profile of each revision is recorded under `.research/w1-2026-11-21/`
//! (`w1d.md`, `w1g-gate.md`, `w1l-gate.md`, `g3-gate.md`). Keeping the numbers in
//! the ignored evidence instead of this doc comment is deliberate: a test header
//! that promises a count goes stale the moment the gate moves.
//!
//! Two marker semantics are worth knowing when reading a red result:
//!
//! - Marker (c) can only observe a **content** change. A successful write of
//!   zeros (a DMA fill that clears VRAM) leaves VRAM at its reset value, so the
//!   marker fails while the write was accepted; the profile reports the data-port
//!   access separately, and the failure message distinguishes "no write ever
//!   reached the data port" from "a data-port write was observed but VRAM, CRAM
//!   and VSRAM are still at their reset values".
//! - Marker (e) flags *any* entry into the ROM's own exception handler, which is
//!   a conservative proxy: it caught a real device divergence (a `0xA10007` port 3
//!   data store the hardware accepts was raising a guest address error) and the
//!   emulator was fixed for it — the marker was never weakened to make the gate
//!   pass. A ROM that deliberately raises and services an exception would trip it
//!   too; that is the documented cost of a positive marker.

use std::collections::HashSet;
use std::error::Error;

use chronofork_backend_api::{Backend, BackendError, DebugConfig, DebugEventKind};
use chronofork_core::InputFrame;
use chronofork_megadrive::vdp::DmaMode;
use chronofork_megadrive::{MegadriveConfig, MegadriveFactory, TmssModel, Vdp, test_support};

/// Instruction budget G-BOOT-0 requires for each commercial fixture.
const G_BOOT_STEPS: usize = 300_000;
/// Budget for the synthetic control ROMs. They prove the gate machinery, not a
/// game boot, so they run a short, deterministic budget in the default suite.
const SYNTHETIC_STEPS: usize = 4_000;
/// A constant PC for this many consecutive steps is a fatal loop (G-BOOT-1).
const PC_RUN_WINDOW: usize = 1_000;
/// Debug-event ring capacity and drain interval for data-port observation.
const EVENT_CAPACITY: usize = 65_536;
const EVENT_DRAIN_INTERVAL: usize = 512;
/// Opt-in switch for the commercial-fixture half of the gate.
const FIXTURE_GATE_ENV: &str = "CHRONOFORK_BOOT_GATE";

/// Why a run stopped before its instruction budget.
#[derive(Clone, Debug, Eq, PartialEq)]
enum BootStop {
    /// The backend returned a typed error at `step` (`pc` is the pre-step PC).
    HostError {
        step: usize,
        pc: u32,
        message: String,
    },
    /// The backend reported a halted machine at `step`.
    Halted { step: usize, pc: u32 },
}

/// Everything the two gates observe about one run.
#[derive(Clone, Debug)]
struct BootProfile {
    fixture: String,
    requested_steps: usize,
    completed_steps: usize,
    stop: Option<BootStop>,
    init_pc: u32,
    last_pc: u32,
    distinct_pcs: usize,
    longest_constant_pc_run: usize,
    first_forbidden_pc_hit: Option<(usize, u32)>,
    first_vdp_register_change: Option<(usize, u8, u8)>,
    first_data_port_write: Option<(usize, u32)>,
    first_data_port_read: Option<(usize, u32)>,
    vram_written: bool,
    cram_written: bool,
    vsram_written: bool,
    final_state_hash: Option<String>,
}

/// Per-ROM G-BOOT-1 expectations.
#[derive(Clone, Copy, Debug)]
struct BootMarkers {
    /// The reset vector's init PC, pinned by measurement.
    init_pc: u32,
    /// The ROM's own exception/trap handler; entering it proves a guest fault
    /// the ROM did not recover from.
    handler_pc: Option<u32>,
}

impl BootProfile {
    /// G-BOOT-0: every requested step completed with no host error and no halt.
    fn g_boot_0_failures(&self) -> Vec<String> {
        let mut failures = Vec::new();
        if self.completed_steps != self.requested_steps {
            failures.push(format!(
                "completed {} of {} requested steps (an early stop fails G-BOOT-0)",
                self.completed_steps, self.requested_steps
            ));
        }
        match &self.stop {
            Some(BootStop::HostError { step, pc, message }) => failures.push(format!(
                "host error at step {step} (pc=0x{pc:06X}): {message}"
            )),
            Some(BootStop::Halted { step, pc }) => {
                failures.push(format!("machine halted at step {step} (pc=0x{pc:06X})"));
            }
            None => {}
        }
        failures
    }

    /// G-BOOT-1: the positive marker set, never "registers are not zero".
    fn g_boot_1_failures(&self, markers: &BootMarkers) -> Vec<String> {
        let mut failures = Vec::new();
        if self.init_pc != markers.init_pc {
            failures.push(format!(
                "(a) init pc is 0x{:06X}, expected 0x{:06X}",
                self.init_pc, markers.init_pc
            ));
        }
        match self.first_vdp_register_change {
            Some((_, register, _)) => {
                if register >= 0x18 {
                    failures.push(format!(
                        "(b) first register transition wrote out-of-range register 0x{register:02X}"
                    ));
                }
            }
            None => {
                failures.push("(b) the VDP register file never left its reset value".to_owned())
            }
        }
        match self.first_data_port_write {
            Some(_) => {
                if !(self.vram_written || self.cram_written || self.vsram_written) {
                    failures.push(
                        "(c) a data-port write was observed but VRAM, CRAM and VSRAM are all \
                         still at their reset values"
                            .to_owned(),
                    );
                }
            }
            None => {
                failures.push("(c) no VRAM/CRAM/VSRAM write ever reached the data port".to_owned())
            }
        }
        if self.longest_constant_pc_run >= PC_RUN_WINDOW {
            failures.push(format!(
                "(d) the PC stayed constant for {} of {PC_RUN_WINDOW} steps",
                self.longest_constant_pc_run
            ));
        }
        if self.last_pc == markers.init_pc {
            failures.push(format!(
                "(d) the run ended at the init pc 0x{:06X}",
                markers.init_pc
            ));
        }
        if let (Some(handler), Some((step, pc))) = (markers.handler_pc, self.first_forbidden_pc_hit)
        {
            failures.push(format!(
                "(e) the ROM's exception handler 0x{handler:06X} ran at step {step} \
                 (pc=0x{pc:06X}): a guest fault the ROM did not recover from"
            ));
        }
        failures
    }

    fn print(&self) {
        println!("g-boot profile: {}", self.fixture);
        println!(
            "  steps: completed={} requested={}",
            self.completed_steps, self.requested_steps
        );
        println!(
            "  init pc=0x{:06X} last pc=0x{:06X}",
            self.init_pc, self.last_pc
        );
        println!(
            "  distinct pcs={} longest constant-pc run={}",
            self.distinct_pcs, self.longest_constant_pc_run
        );
        match &self.stop {
            Some(BootStop::HostError { step, pc, message }) => {
                println!("  stop: host error at step {step} pc=0x{pc:06X}: {message}");
            }
            Some(BootStop::Halted { step, pc }) => {
                println!("  stop: halted at step {step} pc=0x{pc:06X}");
            }
            None => println!("  stop: none (budget completed)"),
        }
        match self.first_vdp_register_change {
            Some((step, register, value)) => println!(
                "  first vdp register transition: step {step} register 0x{register:02X} = 0x{value:02X}"
            ),
            None => println!("  first vdp register transition: none"),
        }
        match self.first_data_port_write {
            Some((step, value)) => {
                println!("  first data-port write: step {step} value=0x{value:04X}");
            }
            None => println!("  first data-port write: none"),
        }
        match self.first_data_port_read {
            Some((step, value)) => {
                println!("  first data-port read: step {step} value=0x{value:04X}");
            }
            None => println!("  first data-port read: none"),
        }
        println!(
            "  vram_written={} cram_written={} vsram_written={}",
            self.vram_written, self.cram_written, self.vsram_written
        );
        match &self.final_state_hash {
            Some(hash) => println!("  final state hash={hash}"),
            None => println!("  final state hash: unavailable"),
        }
    }
}

/// Runs one ROM under both gates and returns everything they observe.
fn boot_profile(
    fixture: &str,
    rom: Vec<u8>,
    steps: usize,
    forbidden_pc: Option<u32>,
) -> Result<BootProfile, BackendError> {
    let factory = MegadriveFactory::with_config(
        rom,
        MegadriveConfig {
            video: None,
            tmss: TmssModel::Disabled,
        },
    )?;
    let mut machine = factory.create_machine()?;
    let config = DebugConfig::new(EVENT_CAPACITY, Vec::new())?;
    if let Some(debug) = machine.debug_access() {
        debug.configure_debug(config)?;
    }

    let init_pc = machine.cpu().pc();
    let mut profile = BootProfile {
        fixture: fixture.to_owned(),
        requested_steps: steps,
        completed_steps: 0,
        stop: None,
        init_pc,
        last_pc: init_pc,
        distinct_pcs: 0,
        longest_constant_pc_run: 0,
        first_forbidden_pc_hit: None,
        first_vdp_register_change: None,
        first_data_port_write: None,
        first_data_port_read: None,
        vram_written: false,
        cram_written: false,
        vsram_written: false,
        final_state_hash: None,
    };
    let mut seen_pcs = HashSet::new();
    let mut previous_pc = init_pc;
    let mut run_length = 0_usize;
    let mut registers = *machine.vdp().registers();

    for index in 0..steps {
        let pc = machine.cpu().pc();
        seen_pcs.insert(pc);
        if Some(pc) == forbidden_pc && profile.first_forbidden_pc_hit.is_none() {
            profile.first_forbidden_pc_hit = Some((index, pc));
        }
        match machine.step(InputFrame::NEUTRAL) {
            Ok(outcome) => {
                if outcome.halted {
                    profile.stop = Some(BootStop::Halted { step: index, pc });
                    break;
                }
            }
            Err(error) => {
                profile.stop = Some(BootStop::HostError {
                    step: index,
                    pc,
                    message: error.to_string(),
                });
                break;
            }
        }
        profile.completed_steps = index + 1;

        let next_pc = machine.cpu().pc();
        seen_pcs.insert(next_pc);
        if next_pc == previous_pc {
            run_length += 1;
        } else {
            previous_pc = next_pc;
            run_length = 1;
        }
        profile.longest_constant_pc_run = profile.longest_constant_pc_run.max(run_length);
        profile.last_pc = next_pc;

        if profile.first_vdp_register_change.is_none() {
            let current = *machine.vdp().registers();
            if current != registers {
                if let Some((register, value)) = current
                    .iter()
                    .zip(registers.iter())
                    .enumerate()
                    .find(|(_, (now, before))| now != before)
                    .map(|(index, (now, _))| (index as u8, *now))
                {
                    profile.first_vdp_register_change =
                        Some((profile.completed_steps, register, value));
                }
                registers = current;
            }
        }

        if profile.completed_steps % EVENT_DRAIN_INTERVAL == 0 {
            drain_vdp_events(&mut machine, &mut profile)?;
        }
    }

    drain_vdp_events(&mut machine, &mut profile)?;
    profile.distinct_pcs = seen_pcs.len();
    profile.vram_written = machine.vdp().vram().iter().any(|byte| *byte != 0);
    profile.cram_written = machine.vdp().cram().iter().any(|word| *word != 0);
    profile.vsram_written = machine.vdp().vsram().iter().any(|word| *word != 0);
    if let Ok(snapshot) = machine.snapshot() {
        profile.final_state_hash = Some(snapshot.state_hash().to_string());
    }
    Ok(profile)
}

/// Drains the VDP port events accumulated since the previous drain.
///
/// Data-port access is the one observable that proves a VRAM/CRAM/VSRAM write
/// without hashing 64 KiB of VRAM per step: a data-port byte or word access is
/// impossible without one. The event ring is bounded, so it is emptied and
/// reinstalled instead of being read once at the end of a 300 000-step run.
fn drain_vdp_events(
    machine: &mut chronofork_megadrive::MegadriveBackend,
    profile: &mut BootProfile,
) -> Result<(), BackendError> {
    let events: Vec<DebugEventKind> = {
        let Some(debug) = machine.debug_access() else {
            return Ok(());
        };
        let Some(log) = debug.take_debug_event_log() else {
            return Ok(());
        };
        log.entries().map(|event| event.kind).collect()
    };
    for kind in events {
        let DebugEventKind::VdpPort {
            offset,
            value,
            write,
            ..
        } = kind
        else {
            continue;
        };
        // Only the data port (offsets 0x00..=0x03 of the VDP window) proves a
        // VRAM/CRAM/VSRAM write. Register writes arrive on the control port
        // (0x04..=0x07) and are detected by diffing the register file instead of
        // by decoding control words here.
        if !(0x00..=0x03).contains(&offset) {
            continue;
        }
        let step = profile.completed_steps;
        let slot = if write {
            &mut profile.first_data_port_write
        } else {
            &mut profile.first_data_port_read
        };
        if slot.is_none() {
            *slot = Some((step, value));
        }
    }
    let config = DebugConfig::new(EVENT_CAPACITY, Vec::new())?;
    if let Some(debug) = machine.debug_access() {
        debug.configure_debug(config)?;
    }
    Ok(())
}

/// Reads a 32-bit big-endian vector from a synthetic image.
fn vector(rom: &[u8], offset: usize) -> u32 {
    let mut bytes = [0_u8; 4];
    if let Some(slice) = rom.get(offset..offset + 4) {
        bytes.copy_from_slice(slice);
    }
    u32::from_be_bytes(bytes)
}

/// A 2 KiB synthetic image with the reset vectors, a NOP fill and `program` at
/// `0x200`. Every synthetic ROM in this file is independently authored and
/// contains no copyrighted data.
fn synthetic_rom(program: &[u8]) -> Vec<u8> {
    let mut rom = [0x4E, 0x71].repeat(0x400);
    rom[0..4].copy_from_slice(&0x00FF_F000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    let end = 0x200 + program.len();
    assert!(end <= rom.len(), "synthetic program must fit the image");
    rom[0x200..end].copy_from_slice(program);
    assert!(
        test_support::stamp_header(&mut rom, "JUE"),
        "a 2 KiB synthetic image holds a header"
    );
    rom
}

/// `MOVE.W #<word>,(0x00C00004).L`: one VDP control-port register write.
fn vdp_control_write(word: u16) -> Vec<u8> {
    let mut bytes = vec![0x33, 0xFC];
    bytes.extend_from_slice(&word.to_be_bytes());
    bytes.extend_from_slice(&0x00C0_0004_u32.to_be_bytes());
    bytes
}

/// `MOVE.W #<word>,(0x00C00000).L`: one VDP data-port write.
fn vdp_data_write(word: u16) -> Vec<u8> {
    let mut bytes = vec![0x33, 0xFC];
    bytes.extend_from_slice(&word.to_be_bytes());
    bytes.extend_from_slice(&0x00C0_0000_u32.to_be_bytes());
    bytes
}

/// `TST.L (<address>).L`.
fn tst_long(address: u32) -> Vec<u8> {
    let mut bytes = vec![0x4A, 0xB9];
    bytes.extend_from_slice(&address.to_be_bytes());
    bytes
}

/// `MOVE.L D0,(<address>).L`.
fn move_long_d0_to(address: u32) -> Vec<u8> {
    let mut bytes = vec![0x23, 0xC0];
    bytes.extend_from_slice(&address.to_be_bytes());
    bytes
}

/// The mandatory negative control: a `BRA.S -2` self-loop at the init PC.
///
/// It completes the step budget (so G-BOOT-0 passes) and must fail every
/// G-BOOT-1 marker: it never writes a VDP register, never reaches the data port,
/// never leaves the init PC and never leaves its own loop.
fn bra_star_rom() -> Vec<u8> {
    synthetic_rom(&[0x60, 0xFE])
}

/// A synthetic ROM whose first non-VDP work is the TMSS/version probe of the two
/// commercial fixtures: `TST.L $00A10008`, `TST.L $00A1000C`.
///
/// The vector-3 (address error) handler points at a distinct self-loop at
/// `0x300`, so a guest address error on either read parks the PC there and the
/// run's marker check fails instead of silently continuing.
fn io_control_boot_rom() -> Vec<u8> {
    let mut program = Vec::new();
    program.extend_from_slice(&tst_long(0x00A1_0008));
    program.extend_from_slice(&tst_long(0x00A1_000C));
    // Leave the VDP register file at a non-reset value, program a VRAM write
    // command at address 0 and write one word through the data port.
    program.extend_from_slice(&vdp_control_write(0x8104));
    program.extend_from_slice(&vdp_control_write(0x4000));
    program.extend_from_slice(&vdp_control_write(0x0000));
    program.extend_from_slice(&vdp_data_write(0x1234));
    // Record the visit in work RAM and stop in a distinct self-loop.
    program.extend_from_slice(&[0x70, 0x5A]); // MOVEQ #0x5A,D0
    program.extend_from_slice(&move_long_d0_to(0x00FF_0000));
    program.extend_from_slice(&[0x60, 0xFE]); // BRA.S -2 marker
    let mut rom = synthetic_rom(&program);
    rom[0x0C..0x10].copy_from_slice(&0x0000_0300_u32.to_be_bytes());
    rom[0x300..0x302].copy_from_slice(&[0x60, 0xFE]);
    rom
}

/// A synthetic ROM that programs the 68000-memory DMA the slice deliberately
/// rejects: DMA enable in register `0x01`, length/source in `0x13..=0x17`, then a
/// data-port access with the DMA command code (VRAM write + CD5).
///
/// The machine stops with the typed `VdpError::DmaMemorySourceUnavailable`
/// instead of copying VRAM onto VRAM. The gate must report that as an early stop
/// with the blocker named — never as a pass.
fn dma_memory_source_rom() -> Vec<u8> {
    let mut program = Vec::new();
    program.extend_from_slice(&[0x70, 0x00]); // MOVEQ #0,D0
    program.extend_from_slice(&vdp_control_write(0x8110)); // reg 0x01: DMA enable
    program.extend_from_slice(&vdp_control_write(0x9302)); // reg 0x13: length low
    program.extend_from_slice(&vdp_control_write(0x9400)); // reg 0x14: length high
    program.extend_from_slice(&vdp_control_write(0x9500)); // reg 0x15: source low
    program.extend_from_slice(&vdp_control_write(0x9600)); // reg 0x16: source mid
    program.extend_from_slice(&vdp_control_write(0x9700)); // reg 0x17: mode 0b00
    program.extend_from_slice(&vdp_control_write(0x4000)); // command: VRAM write
    program.extend_from_slice(&vdp_control_write(0x0080)); // command: CD5 = DMA
    program.extend_from_slice(&vdp_data_write(0x0000)); // triggers the rejected DMA
    program.extend_from_slice(&[0x60, 0xFE]);
    synthetic_rom(&program)
}

/// Runs one synthetic ROM under both gates and asserts the exact verdict.
fn assert_synthetic(
    fixture: &str,
    rom: Vec<u8>,
    steps: usize,
    markers: &BootMarkers,
    g_boot_0_must_pass: bool,
    g_boot_1_must_pass: bool,
) -> Result<(), Box<dyn Error>> {
    let profile = boot_profile(fixture, rom, steps, markers.handler_pc)?;
    profile.print();
    let g0 = profile.g_boot_0_failures();
    let g1 = profile.g_boot_1_failures(markers);
    assert_eq!(
        g0.is_empty(),
        g_boot_0_must_pass,
        "G-BOOT-0 for {fixture}: expected pass={g_boot_0_must_pass}, failures: {g0:?}"
    );
    assert_eq!(
        g1.is_empty(),
        g_boot_1_must_pass,
        "G-BOOT-1 for {fixture}: expected pass={g_boot_1_must_pass}, failures: {g1:?}"
    );
    Ok(())
}

/// The `BRA *` negative control: G-BOOT-0 passes and G-BOOT-1 fails.
#[test]
fn bra_star_control_fails_the_bootstrap_markers() -> Result<(), Box<dyn Error>> {
    let rom = bra_star_rom();
    let markers = BootMarkers {
        init_pc: 0x0000_0200,
        handler_pc: None,
    };
    assert_synthetic("bra-star-control", rom, 2_000, &markers, true, false)
}

/// Any early stop fails G-BOOT-0, even when the stop is a typed, known blocker.
///
/// The blocker is asserted twice: at the source (`Vdp::start_dma` returns the
/// typed `VdpError::DmaMemorySourceUnavailable`) and at the gate (the run stops
/// early and the report names the rejected mode and word count).
#[test]
fn g_boot_0_reprova_an_early_stop_even_for_a_typed_blocker() -> Result<(), Box<dyn Error>> {
    // The typed rejection, asserted on the source API.
    let mut vdp = Vdp::new();
    match vdp.start_dma(0, 2, DmaMode::MemoryToVram) {
        Err(chronofork_megadrive::VdpError::DmaMemorySourceUnavailable { mode, words }) => {
            assert_eq!(mode, DmaMode::MemoryToVram);
            assert_eq!(words, 2);
        }
        other => {
            return Err(format!(
                "start_dma must return DmaMemorySourceUnavailable for MemoryToVram, got {other:?}"
            )
            .into());
        }
    }

    let rom = dma_memory_source_rom();
    let markers = BootMarkers {
        init_pc: 0x0000_0200,
        handler_pc: None,
    };
    let profile = boot_profile("dma-memory-source-blocker", rom, SYNTHETIC_STEPS, None)?;
    profile.print();
    assert!(
        !profile.g_boot_0_failures().is_empty(),
        "a run that stops early must fail G-BOOT-0"
    );
    let stop = profile
        .stop
        .as_ref()
        .ok_or("the run must record how it stopped")?;
    let BootStop::HostError { message, .. } = stop else {
        return Err(format!("expected a typed host error, got {stop:?}").into());
    };
    assert!(
        message.contains("MemoryToVram"),
        "the blocker must name the rejected mode, observed: {message}"
    );
    // The synthetic ROM programs registers 0x13/0x14 with the literal pair
    // `02 00`. The hardware length register is length-minus-one, so that pair
    // encodes **three** words, and the typed error must name the decoded count
    // (this assertion is therefore also the harness-side guard for that
    // encoding).
    assert!(
        message.contains("needs a 68000-memory source") && message.contains("3 words"),
        "the blocker must name the reason and the decoded word count, observed: {message}"
    );
    let g1 = profile.g_boot_1_failures(&markers);
    assert!(!g1.is_empty(), "an early stop cannot pass G-BOOT-1 either");
    Ok(())
}

/// The regression guard for the W1 bus work: the fixtures' first instructions
/// (`TST.L $00A10008`, `TST.L $00A1000C`) must not fault, and a ROM that does
/// them can reach the VDP and its own marker loop.
#[test]
fn io_control_bootstrap_passes_both_gates() -> Result<(), Box<dyn Error>> {
    let rom = io_control_boot_rom();
    let handler_pc = vector(&rom, 0x0C);
    assert_eq!(handler_pc, 0x0000_0300, "the address-error vector fixture");
    let markers = BootMarkers {
        init_pc: 0x0000_0200,
        handler_pc: Some(handler_pc),
    };
    assert_synthetic("io-control-bootstrap", rom, 512, &markers, true, true)
}

/// The fixture half of the gate. Needs the ignored local ROMs and a release
/// build, so it is opt-in through `CHRONOFORK_BOOT_GATE=1`.
#[test]
fn fixture_boot_gate() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(FIXTURE_GATE_ENV).is_none() {
        println!(
            "g-boot: fixture gate NOT executed; set {FIXTURE_GATE_ENV}=1 and run \
             `cargo test --release -p chronofork-megadrive --test boot_gate` for the \
             300000-step gate. A run without fixtures is a failure, never a pass."
        );
        return Ok(());
    }

    let fixtures = [
        ("Kid Chameleon (USA, Europe).bin", 0x2D8_u32),
        ("Chakan (USA, Europe).bin", 0x220_u32),
    ];
    let mut executed = 0_usize;
    let mut failures = Vec::new();
    for (name, handler_pc) in fixtures {
        let Some(path) = test_support::rom_path(name) else {
            println!("g-boot: skipping {name}: fixture absent from .research/roms/");
            continue;
        };
        executed += 1;
        let rom = std::fs::read(path)?;
        let init_pc = vector(&rom, 4);
        let markers = BootMarkers {
            init_pc,
            handler_pc: Some(handler_pc),
        };
        let profile = boot_profile(name, rom, G_BOOT_STEPS, Some(handler_pc))?;
        profile.print();
        for failure in profile.g_boot_0_failures() {
            failures.push(format!("{name}: G-BOOT-0 {failure}"));
        }
        for failure in profile.g_boot_1_failures(&markers) {
            failures.push(format!("{name}: G-BOOT-1 {failure}"));
        }
    }
    assert!(
        executed > 0,
        "the fixture gate requires at least one executed fixture; zero fixtures is a failure"
    );
    assert!(
        failures.is_empty(),
        "G-BOOT-0/1 are RED for {executed} fixture(s):\n  {}",
        failures.join("\n  ")
    );
    println!("g-boot: G-BOOT-0 and G-BOOT-1 pass for {executed} fixture(s)");
    Ok(())
}
