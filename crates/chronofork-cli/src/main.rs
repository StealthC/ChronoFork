//! Minimal ChronoFork command-line interface.
//!
//! Only capabilities that actually exist are exposed. There are deliberately no
//! stubs for HTTP, MCP, GUI or a real Mega Drive backend.

use std::error::Error;
use std::fmt;
use std::process::ExitCode;
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_core::InputFrame;
use chronofork_megadrive::{MegadriveFactory, RAM_BASE, SchedulerConfig, VideoStandard, demo_rom};
use chronofork_sessions::{SessionManager, Snapshot};
use chronofork_test_backend::SyntheticBackendFactory;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chronofork: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    let second = args.next();
    if args.next().is_some() {
        return Err(CliError("too many arguments; see `chronofork help`".to_owned()).into());
    }
    match (first.as_deref(), second.as_deref()) {
        (None, None) | (Some("help"), None) => {
            print_usage();
            Ok(())
        }
        (Some("--version" | "-V"), None) => {
            println!("chronofork {VERSION}");
            Ok(())
        }
        (Some("demo"), None) => run_demo(),
        (Some("megadrive"), Some("demo")) => run_megadrive_demo(),
        (Some("backend"), Some("describe")) => describe_backend(),
        (Some("backend"), None) => {
            Err(CliError("`backend` requires a subcommand: describe".to_owned()).into())
        }
        (Some("megadrive"), None) => {
            Err(CliError("`megadrive` requires a subcommand: demo".to_owned()).into())
        }
        _ => Err(CliError("unknown command; see `chronofork help`".to_owned()).into()),
    }
}

fn print_usage() {
    println!("chronofork {VERSION}");
    println!();
    println!("A deterministic, forkable execution foundation. This build ships a");
    println!("synthetic test backend, not a Sega Mega Drive emulator.");
    println!();
    println!("Usage:");
    println!(
        "  chronofork demo               Run the deterministic synthetic fork/restore scenario"
    );
    println!("  chronofork megadrive demo      Run the minimal 68000 + VDP + interrupt slice");
    println!("  chronofork backend describe   Show synthetic backend identity and capabilities");
    println!("  chronofork --version          Show version");
    println!("  chronofork help               Show this help");
}

fn describe_backend() -> Result<(), Box<dyn Error>> {
    let factory = SyntheticBackendFactory::default();
    let descriptor = factory.descriptor();
    let capabilities = descriptor.capabilities();
    println!("backend: {}", descriptor.id());
    println!("name: {}", descriptor.display_name());
    println!("version: {}", descriptor.version());
    println!("test_backend: {}", descriptor.is_test_backend());
    println!("capability.execution: {}", capabilities.execution);
    println!("capability.save_states: {}", capabilities.save_states);
    println!("capability.memory_read: {}", capabilities.memory_read);
    println!("capability.memory_write: {}", capabilities.memory_write);
    println!(
        "capability.register_inspection: {}",
        capabilities.register_inspection
    );
    println!(
        "capability.instruction_stepping: {}",
        capabilities.instruction_stepping
    );
    println!("capability.state_cloning: {}", capabilities.state_cloning);
    println!(
        "capability.deterministic_replay: {}",
        capabilities.deterministic_replay
    );
    println!("capability.breakpoints: {}", capabilities.breakpoints);
    println!("capability.watchpoints: {}", capabilities.watchpoints);
    println!("capability.tracing: {}", capabilities.tracing);
    println!("capability.rendering: {}", capabilities.rendering);
    println!("capability.audio: {}", capabilities.audio);
    Ok(())
}

fn run_demo() -> Result<(), Box<dyn Error>> {
    let factory = Arc::new(SyntheticBackendFactory::default());
    let mut manager = SessionManager::new(factory);
    let descriptor = manager.backend_descriptor().clone();

    println!("chronofork demo: deterministic session vertical slice");
    println!(
        "backend: {} ({})",
        descriptor.display_name(),
        descriptor.version()
    );
    ensure(
        descriptor.is_test_backend(),
        "demo requires the synthetic test backend",
    )?;

    let source = manager.create_session()?;
    for _ in 0..12 {
        manager.step(source, InputFrame::NEUTRAL)?;
    }
    let checkpoint = manager.snapshot(source)?;
    let checkpoint_view = manager.inspect(source)?;
    println!(
        "checkpoint: {} at tick {} hash {}",
        checkpoint, checkpoint_view.machine.tick, checkpoint_view.machine.state_hash
    );

    let bytes = manager.snapshot_bytes(source, checkpoint)?;
    let decoded = Snapshot::from_bytes(&bytes)?;
    ensure(
        decoded.tick() == checkpoint_view.machine.tick,
        "serialized snapshot tick mismatch",
    )?;
    ensure(
        decoded.state_hash() == checkpoint_view.machine.state_hash,
        "serialized snapshot state hash mismatch",
    )?;
    println!(
        "serialized checkpoint: {} bytes, checksum verified",
        bytes.len()
    );

    let branch_a = manager.fork_from_snapshot(source, checkpoint)?;
    let branch_b = manager.fork_from_snapshot(source, checkpoint)?;
    let branch_c = manager.fork_from_snapshot(source, checkpoint)?;
    println!("forks: {branch_a}, {branch_b}, {branch_c}");

    for branch in [branch_a, branch_b, branch_c] {
        let view = manager.inspect(branch)?;
        ensure(
            view.machine.state_hash == checkpoint_view.machine.state_hash,
            "fork did not start at the checkpoint state",
        )?;
        ensure(
            view.machine.tick == checkpoint_view.machine.tick,
            "fork did not start at the checkpoint tick",
        )?;
    }

    let inputs = [1_u32, 0x7f, 0xff];
    let branches = [branch_a, branch_b, branch_c];
    for (branch, input) in branches.iter().zip(inputs) {
        for _ in 0..6 {
            manager.step(*branch, InputFrame::new(input))?;
        }
    }

    let mut hashes = Vec::new();
    for branch in branches {
        let view = manager.inspect(branch)?;
        println!(
            "branch {}: tick {} hash {}",
            branch, view.machine.tick, view.machine.state_hash
        );
        hashes.push(view.machine.state_hash);
    }
    ensure(hashes[0] != hashes[1], "branches A and B did not diverge")?;
    ensure(hashes[1] != hashes[2], "branches B and C did not diverge")?;
    ensure(hashes[0] != hashes[2], "branches A and C did not diverge")?;

    let source_view = manager.inspect(source)?;
    ensure(
        source_view.machine.state_hash == checkpoint_view.machine.state_hash,
        "source session was mutated by branch execution",
    )?;
    println!(
        "source isolation: source remains at tick {} hash {}",
        source_view.machine.tick, source_view.machine.state_hash
    );

    let branch_a_memory_before = manager.read_memory(branch_a, 0x20, 1)?;
    manager.write_memory(branch_c, 0x20, &[0xaa])?;
    let branch_a_memory_after = manager.read_memory(branch_a, 0x20, 1)?;
    let branch_c_memory = manager.read_memory(branch_c, 0x20, 1)?;
    ensure(
        branch_a_memory_before == branch_a_memory_after,
        "memory write in branch C affected branch A",
    )?;
    ensure(
        branch_c_memory == vec![0xaa],
        "memory write in branch C did not take effect",
    )?;
    println!("memory isolation: branch C mutation did not affect branch A");

    let branch_a_hash = hashes[0];
    manager.restore(branch_a, checkpoint)?;
    for _ in 0..6 {
        manager.step(branch_a, InputFrame::new(1))?;
    }
    let replayed = manager.inspect(branch_a)?;
    ensure(
        replayed.machine.state_hash == branch_a_hash,
        "restore and replay produced a different state",
    )?;
    println!("replay verification: restored branch A reproduced its state hash");

    let comparison = manager.compare(branch_a, branch_b)?;
    ensure(
        !comparison.equivalent,
        "independent branches compared as equal",
    )?;
    println!(
        "compare: branch A and B differ at tick {} ({})",
        comparison.left_tick, comparison.left_hash
    );

    println!("chronofork demo: PASS");
    Ok(())
}

fn run_megadrive_demo() -> Result<(), Box<dyn Error>> {
    let factory = MegadriveFactory::new(demo_rom())?;
    let mut manager = SessionManager::new(Arc::new(factory));
    let descriptor = manager.backend_descriptor().clone();

    println!("chronofork megadrive demo: 68000 + VDP port + interrupt scheduler slice");
    println!(
        "backend: {} ({})",
        descriptor.display_name(),
        descriptor.version()
    );
    println!(
        "scope: 68000 subset, ROM/RAM, VDP status/control/VRAM window, deterministic interrupt scheduler"
    );
    println!("limitations: no rendering, audio, Z80, DMA, controllers or cycle-accurate timing");

    let source = manager.create_session()?;
    for _ in 0..23 {
        manager.step(source, InputFrame::NEUTRAL)?;
    }
    let vdp_view = manager.inspect(source)?;
    let registers = &vdp_view.machine.registers;
    let computed = manager.read_memory(source, 0x00FF_0000, 2)?;
    ensure(
        computed == vec![0x00, 0x03],
        "demo program did not compute 1 + 2",
    )?;
    ensure(
        registers["d2"] == 0x1234,
        "demo program did not read the VDP data port",
    )?;
    ensure(
        registers["vdp_auto_increment"] == 2,
        "demo program did not set VDP register 15",
    )?;
    ensure(
        registers["vdp_status"] == 0x0100,
        "VDP status should be FIFO-empty with VBlank clear",
    )?;
    let vram_readback = manager.read_memory(source, 0x00FF_0004, 2)?;
    ensure(
        vram_readback == vec![0x12, 0x34],
        "demo program did not read back VRAM word 0x1234",
    )?;
    let stored_status = manager.read_memory(source, 0x00FF_0006, 2)?;
    ensure(
        stored_status == vec![0x01, 0x00],
        "demo program did not capture the VDP status word",
    )?;
    println!(
        "vdp: register 15 (auto-increment)={} status=0x{:04X} address=0x{:04X}",
        registers["vdp_auto_increment"], registers["vdp_status"], registers["vdp_address"]
    );
    println!("vdp: program wrote and read back VRAM word 0x1234 through the data port");

    // A request raised at the end of one instruction is visible in inspection and
    // taken at the next instruction boundary.
    for _ in 23..64 {
        manager.step(source, InputFrame::NEUTRAL)?;
    }
    let pending = manager.inspect(source)?;
    ensure(
        pending.machine.tick.raw() == 64,
        "unexpected tick before HBlank",
    )?;
    ensure(
        pending.machine.registers["irq_pending_level"] == 4,
        "HBlank should be pending at tick 64",
    )?;
    ensure(
        pending.machine.registers["irq_vector"] == 0x70,
        "HBlank should select autovector 0x70",
    )?;
    println!(
        "interrupt: HBlank pending at tick 64 (level 4, vector 0x70, mask {})",
        pending.machine.registers["irq_mask"]
    );

    manager.step(source, InputFrame::NEUTRAL)?;
    let delivered = manager.inspect(source)?;
    ensure(
        delivered.machine.registers["d7"] == 1,
        "HBlank handler did not run exactly once",
    )?;
    ensure(
        delivered.machine.registers["irq_pending_level"] == 0,
        "HBlank request was not acknowledged",
    )?;
    println!(
        "interrupt: HBlank delivered; handler pc=0x{:X} D7={}",
        delivered.machine.registers["pc"], delivered.machine.registers["d7"]
    );

    // VBlank needs 224 active scanlines. A shorter instruction quantum keeps the
    // demonstration compact while exercising the same scheduler path.
    let fast_config = SchedulerConfig::new(VideoStandard::Ntsc, 4)?;
    let fast_factory = MegadriveFactory::with_scheduler_config(demo_rom(), fast_config)?;
    let mut fast_manager = SessionManager::new(Arc::new(fast_factory));
    let fast = fast_manager.create_session()?;
    let vblank_tick = u64::from(VideoStandard::Ntsc.active_lines()) * 4;
    for _ in 0..vblank_tick {
        fast_manager.step(fast, InputFrame::NEUTRAL)?;
    }
    let vblank_view = fast_manager.inspect(fast)?;
    ensure(
        vblank_view.machine.registers["irq_pending_level"] == 6,
        "VBlank should have priority at the start of vertical blanking",
    )?;
    ensure(
        vblank_view.machine.registers["irq_vector"] == 0x78,
        "VBlank should select autovector 0x78",
    )?;
    ensure(
        vblank_view.machine.registers["vdp_status"] & 0x0008 != 0,
        "VDP VBlank status flag should be set",
    )?;
    let vblank_checkpoint = fast_manager.snapshot(fast)?;
    fast_manager.step(fast, InputFrame::NEUTRAL)?;
    ensure(
        fast_manager.inspect(fast)?.machine.registers["d6"] == 1,
        "VBlank handler did not run exactly once",
    )?;
    fast_manager.restore(fast, vblank_checkpoint)?;
    ensure(
        fast_manager.inspect(fast)?.machine.registers["irq_pending_level"] == 6,
        "snapshot restore did not preserve the pending VBlank request",
    )?;
    println!(
        "interrupt: VBlank pending level 6 vector 0x78 at tick {vblank_tick} (quantum=4); snapshot restored it"
    );

    let checkpoint = manager.snapshot(source)?;
    let before = manager.inspect(source)?;
    let fork = manager.fork_from_snapshot(source, checkpoint)?;
    manager.write_memory(fork, u64::from(RAM_BASE), &[0xAA])?;
    manager.step(fork, InputFrame::NEUTRAL)?;
    let fork_view = manager.inspect(fork)?;
    let source_view = manager.inspect(source)?;
    ensure(
        fork_view.machine.state_hash != before.machine.state_hash,
        "fork did not diverge after a RAM mutation",
    )?;
    ensure(
        source_view.machine.state_hash == before.machine.state_hash,
        "source was mutated by the fork",
    )?;
    println!(
        "fork isolation: fork={} source={}",
        fork_view.machine.state_hash, source_view.machine.state_hash
    );

    manager.restore(fork, checkpoint)?;
    let restored = manager.inspect(fork)?;
    ensure(
        restored.machine.state_hash == before.machine.state_hash,
        "restore did not reproduce the checkpoint",
    )?;
    println!("snapshot: VDP, scheduler and CPU state reproduced from the checkpoint");
    println!("chronofork megadrive demo: PASS");
    Ok(())
}

fn ensure(condition: bool, message: &str) -> Result<(), Box<dyn Error>> {
    if condition {
        Ok(())
    } else {
        Err(CliError(message.to_owned()).into())
    }
}

#[derive(Debug)]
struct CliError(String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for CliError {}
