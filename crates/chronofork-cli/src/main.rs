//! Minimal ChronoFork command-line interface.
//!
//! Only capabilities that actually exist are exposed. There are deliberately no
//! stubs for HTTP, MCP, GUI or a real Mega Drive backend.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use chronofork_backend_api::{Backend, BackendFactory};
use chronofork_core::InputFrame;
use chronofork_megadrive::{
    DEFAULT_INSTRUCTIONS_PER_LINE, InstructionTrace, MegadriveBackend, MegadriveFactory, RAM_BASE,
    RAM_SIZE, SchedulerConfig, VideoStandard, demo_rom, interrupt_rom,
};
use chronofork_sessions::{SessionManager, Snapshot};
use chronofork_test_backend::SyntheticBackendFactory;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Default instruction budget for `megadrive trace`.
const DEFAULT_TRACE_INSTRUCTIONS: u64 = 1_000;
/// Default cap for the trace ring when it is sized from the instruction budget.
///
/// An explicit `--trace-capacity` overrides the cap. This keeps an accidentally
/// huge budget from allocating an unbounded trace.
const MAX_DEFAULT_TRACE_CAPACITY: usize = 1_000_000;
/// Default number of work-RAM bytes written to the final-state file.
const DEFAULT_TRACE_RAM_BYTES: usize = 32;

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("help") => {
            ensure(args.len() <= 1, "`help` takes no arguments")?;
            print_usage();
            Ok(())
        }
        Some("--version" | "-V") => {
            ensure(args.len() == 1, "`--version` takes no arguments")?;
            println!("chronofork {VERSION}");
            Ok(())
        }
        Some("demo") => {
            ensure(args.len() == 1, "`demo` takes no arguments")?;
            run_demo()
        }
        Some("megadrive") => run_megadrive(&args[1..]),
        Some("backend") => match args.get(1).map(String::as_str) {
            Some("describe") => {
                ensure(args.len() == 2, "`backend describe` takes no arguments")?;
                describe_backend()
            }
            None => Err(CliError("`backend` requires a subcommand: describe".to_owned()).into()),
            Some(other) => Err(CliError(format!(
                "unknown `backend` subcommand `{other}`; expected describe"
            ))
            .into()),
        },
        Some(other) => {
            Err(CliError(format!("unknown command `{other}`; see `chronofork help`")).into())
        }
    }
}

fn run_megadrive(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("demo") => {
            ensure(args.len() == 1, "`megadrive demo` takes no arguments")?;
            run_megadrive_demo()
        }
        Some("trace") => run_megadrive_trace(&args[1..]),
        None => Err(CliError("`megadrive` requires a subcommand: demo or trace".to_owned()).into()),
        Some(other) => Err(CliError(format!(
            "unknown `megadrive` subcommand `{other}`; expected demo or trace"
        ))
        .into()),
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
    println!(
        "  chronofork megadrive trace     Run a bounded instruction trace and write final state"
    );
    println!("  chronofork backend describe   Show synthetic backend identity and capabilities");
    println!("  chronofork --version          Show version");
    println!("  chronofork help               Show this help");
    println!();
    println!("For trace file formats and options run:");
    println!("  chronofork megadrive trace --help");
}

/// Options for the `megadrive trace` subcommand.
#[derive(Debug)]
struct TraceOptions {
    rom: PathBuf,
    state_out: PathBuf,
    trace_out: Option<PathBuf>,
    instructions: u64,
    stop_pc: Option<u32>,
    scheduler: SchedulerConfig,
    trace_capacity: Option<usize>,
    ram_bytes: usize,
}

/// Parses `megadrive trace` arguments.
///
/// Returns `Ok(None)` when the caller should print the detailed help text.
fn parse_trace_options(args: &[String]) -> Result<Option<TraceOptions>, Box<dyn Error>> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        ensure(
            args.len() == 1,
            "`megadrive trace --help` takes no other arguments",
        )?;
        return Ok(None);
    }

    let mut rom: Option<PathBuf> = None;
    let mut state_out: Option<PathBuf> = None;
    let mut trace_out: Option<PathBuf> = None;
    let mut instructions: Option<u64> = None;
    let mut stop_pc: Option<u32> = None;
    let mut instructions_per_line: Option<u32> = None;
    let mut video: Option<VideoStandard> = None;
    let mut trace_capacity: Option<usize> = None;
    let mut ram_bytes: Option<usize> = None;

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--rom" => set_once(
                &mut rom,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--state-out" => set_once(
                &mut state_out,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--trace-out" => set_once(
                &mut trace_out,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--instructions" => set_once(
                &mut instructions,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--stop-pc" => set_once(
                &mut stop_pc,
                parse_hex_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--instructions-per-line" => set_once(
                &mut instructions_per_line,
                parse_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--video" => set_once(
                &mut video,
                parse_video(option_value(args, &mut index, &flag)?)?,
                &flag,
            )?,
            "--trace-capacity" => set_once(
                &mut trace_capacity,
                parse_capacity(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--ram-bytes" => {
                let value = option_value(args, &mut index, &flag)?;
                let parsed = parse_usize(value, &flag)?;
                if parsed == 0 || parsed > RAM_SIZE {
                    return Err(CliError(format!(
                        "`{flag}` must be between 1 and {RAM_SIZE} (the 64 KiB work-RAM size), got {parsed}"
                    ))
                    .into());
                }
                set_once(&mut ram_bytes, parsed, &flag)?;
            }
            other => {
                return Err(CliError(format!(
                    "unknown `megadrive trace` option `{other}`; see `chronofork megadrive trace --help`"
                ))
                .into());
            }
        }
        index += 1;
    }

    let rom =
        rom.ok_or_else(|| CliError("`megadrive trace` requires `--rom <path>`".to_owned()))?;
    let state_out = state_out
        .ok_or_else(|| CliError("`megadrive trace` requires `--state-out <path>`".to_owned()))?;
    if trace_out.as_deref() == Some(state_out.as_path()) {
        return Err(CliError(
            "`--trace-out` and `--state-out` must name different files".to_owned(),
        )
        .into());
    }
    let instructions = instructions.unwrap_or(DEFAULT_TRACE_INSTRUCTIONS);
    let scheduler = SchedulerConfig::new(
        video.unwrap_or(VideoStandard::Ntsc),
        instructions_per_line.unwrap_or(DEFAULT_INSTRUCTIONS_PER_LINE),
    )?;
    let trace_capacity = match trace_capacity {
        Some(0) => {
            return Err(CliError(
                "`--trace-capacity` must be at least 1; omit it to size the ring from the instruction budget"
                    .to_owned(),
            )
            .into());
        }
        other => other,
    };
    Ok(Some(TraceOptions {
        rom,
        state_out,
        trace_out,
        instructions,
        stop_pc,
        scheduler,
        trace_capacity,
        ram_bytes: ram_bytes.unwrap_or(DEFAULT_TRACE_RAM_BYTES),
    }))
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    flag: &str,
) -> Result<&'a str, CliError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| CliError(format!("`{flag}` requires a value")))
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(CliError(format!("`{flag}` was given more than once")));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, CliError> {
    value
        .parse()
        .map_err(|_| CliError(format!("`{flag}` expects a decimal integer, got `{value}`")))
}

fn parse_u32(value: &str, flag: &str) -> Result<u32, CliError> {
    value
        .parse()
        .map_err(|_| CliError(format!("`{flag}` expects a decimal integer, got `{value}`")))
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, CliError> {
    value
        .parse()
        .map_err(|_| CliError(format!("`{flag}` expects a decimal integer, got `{value}`")))
}

fn parse_hex_u32(value: &str, flag: &str) -> Result<u32, CliError> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    u32::from_str_radix(digits, 16).map_err(|_| {
        CliError(format!(
            "`{flag}` expects a hexadecimal address such as 0x200, got `{value}`"
        ))
    })
}

fn parse_video(value: &str) -> Result<VideoStandard, CliError> {
    match value {
        "ntsc" => Ok(VideoStandard::Ntsc),
        "pal" => Ok(VideoStandard::Pal),
        other => Err(CliError(format!(
            "`--video` expects `ntsc` or `pal`, got `{other}`"
        ))),
    }
}

fn parse_capacity(value: &str, flag: &str) -> Result<usize, CliError> {
    parse_usize(value, flag)
}

/// Default trace capacity: the instruction budget, clamped to at least one record
/// and to [`MAX_DEFAULT_TRACE_CAPACITY`].
fn default_trace_capacity(instructions: u64) -> usize {
    let budget = usize::try_from(instructions).unwrap_or(usize::MAX);
    budget.clamp(1, MAX_DEFAULT_TRACE_CAPACITY)
}

fn run_megadrive_trace(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_trace_options(args)? else {
        print_trace_help();
        return Ok(());
    };

    let capacity = options
        .trace_capacity
        .unwrap_or_else(|| default_trace_capacity(options.instructions));
    let rom = std::fs::read(&options.rom).map_err(|error| {
        CliError(format!(
            "failed to read ROM `{}`: {error}",
            options.rom.display()
        ))
    })?;
    let factory = MegadriveFactory::with_scheduler_config(rom, options.scheduler)?;
    let mut machine = factory.create_machine()?;
    machine.enable_trace(capacity)?;

    let mut executed = 0_u64;
    let mut stopped_at: Option<u32> = None;
    let mut failure: Option<String> = None;
    while executed < options.instructions {
        if options.stop_pc == Some(machine.cpu().pc()) {
            stopped_at = options.stop_pc;
            break;
        }
        match machine.step(InputFrame::NEUTRAL) {
            Ok(_) => executed += 1,
            Err(error) => {
                // Preserve the evidence for the failing instruction before
                // reporting the error: the trace and state files are still
                // written below.
                failure = Some(error.to_string());
                break;
            }
        }
    }

    let ram = machine.read_memory(u64::from(RAM_BASE), options.ram_bytes)?;
    let trace = machine.take_trace();
    write_trace(options.trace_out.as_deref(), trace.as_ref())?;
    write_state(
        &options.state_out,
        &machine,
        executed,
        options.scheduler,
        &ram,
    )?;

    let (record_count, first_index, dropped) = match &trace {
        Some(trace) => (
            trace.len(),
            trace.first_index().unwrap_or_default(),
            trace.dropped(),
        ),
        None => (0, 0, 0),
    };
    eprintln!(
        "chronofork megadrive trace: executed {executed} instruction(s), {record_count} trace record(s) from index {first_index}, {dropped} dropped, capacity {capacity}"
    );
    if let Some(pc) = stopped_at {
        eprintln!("chronofork megadrive trace: stopped before the instruction at pc 0x{pc:06X}");
    }
    eprintln!(
        "chronofork megadrive trace: state written to {}",
        options.state_out.display()
    );
    if executed > u64::from(options.scheduler.instructions_per_line()) {
        eprintln!(
            "note: scheduler quantum is {} instruction(s) per line (the default is {DEFAULT_INSTRUCTIONS_PER_LINE}); one executed instruction is one quantum, so this is an instruction-quantum interrupt model, not cycle-accurate VDP timing. Interrupt delivery is gated by the 68000 SR interrupt-priority mask: level 7 is always taken and any other level only when strictly above the mask, so a masked VDP request stays pending instead of running a handler.",
            options.scheduler.instructions_per_line()
        );
    }

    if let Some(error) = failure {
        return Err(CliError(format!(
            "execution failed after {executed} instruction(s): {error}"
        ))
        .into());
    }
    Ok(())
}

fn write_trace(
    path: Option<&Path>,
    trace: Option<&InstructionTrace>,
) -> Result<(), Box<dyn Error>> {
    match path {
        Some(path) => {
            let file = File::create(path).map_err(|error| {
                CliError(format!(
                    "failed to create trace file `{}`: {error}",
                    path.display()
                ))
            })?;
            let mut writer = BufWriter::new(file);
            write_trace_records(&mut writer, trace)?;
            writer.flush()?;
        }
        None => {
            let stdout = io::stdout();
            let mut writer = stdout.lock();
            write_trace_records(&mut writer, trace)?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn write_trace_records(
    writer: &mut dyn Write,
    trace: Option<&InstructionTrace>,
) -> Result<(), Box<dyn Error>> {
    let Some(trace) = trace else {
        return Err(
            CliError("instruction trace was not enabled; nothing to write".to_owned()).into(),
        );
    };
    for entry in trace.entries() {
        writeln!(
            writer,
            "{}\t{:06x}\t{:04x}",
            entry.index(),
            entry.pc(),
            entry.opcode()
        )?;
    }
    Ok(())
}

fn write_state(
    path: &Path,
    machine: &MegadriveBackend,
    executed: u64,
    scheduler: SchedulerConfig,
    ram: &[u8],
) -> Result<(), Box<dyn Error>> {
    let file = File::create(path).map_err(|error| {
        CliError(format!(
            "failed to create state file `{}`: {error}",
            path.display()
        ))
    })?;
    let mut writer = BufWriter::new(file);
    let cpu = machine.cpu();

    writeln!(writer, "chronofork-megadrive-state v1")?;
    writeln!(writer, "executed={executed}")?;
    writeln!(writer, "tick={}", machine.tick().raw())?;
    writeln!(
        writer,
        "instructions_per_line={}",
        scheduler.instructions_per_line()
    )?;
    writeln!(
        writer,
        "video={}",
        match scheduler.standard() {
            VideoStandard::Ntsc => "ntsc",
            VideoStandard::Pal => "pal",
        }
    )?;
    writeln!(writer, "halted={}", u8::from(cpu.halted()))?;
    writeln!(writer, "pc={:08x}", cpu.pc())?;
    writeln!(writer, "sr={:04x}", cpu.sr())?;
    for index in 0..8 {
        writeln!(
            writer,
            "d{index}={:08x}",
            cpu.data_register(index).unwrap_or(0)
        )?;
    }
    for index in 0..8 {
        writeln!(
            writer,
            "a{index}={:08x}",
            cpu.address_register(index).unwrap_or(0)
        )?;
    }
    writeln!(writer, "ram_base={RAM_BASE:08x}")?;
    writeln!(writer, "ram_len={}", ram.len())?;
    write!(writer, "ram=")?;
    for byte in ram {
        write!(writer, "{byte:02x}")?;
    }
    writeln!(writer)?;
    writer.flush()?;
    Ok(())
}

fn print_trace_help() {
    println!("chronofork {VERSION} — megadrive trace");
    println!();
    println!("Runs a deterministic instruction budget (or stops before a chosen PC) on");
    println!("the minimal Mega Drive 68000 slice, records a bounded instruction trace");
    println!("and writes a final-state file. The trace is debug data: enabling it does");
    println!("not change snapshots, state hashes or the snapshot payload version.");
    println!();
    println!("Usage:");
    println!("  chronofork megadrive trace --rom <path> --state-out <path> [options]");
    println!();
    println!("Options:");
    println!("  --rom <path>                 Cartridge image (at least 8 reset-vector bytes)");
    println!("  --state-out <path>           Required final-state output file");
    println!("  --trace-out <path>           Trace output file; omit to write to stdout");
    println!(
        "  --instructions <count>       Instruction budget, decimal (default {DEFAULT_TRACE_INSTRUCTIONS})"
    );
    println!("  --stop-pc <hex>              Stop before the instruction at this address");
    println!("  --instructions-per-line <n>  Scheduler quantum, 68000 instructions per scanline");
    println!(
        "                               (default {DEFAULT_INSTRUCTIONS_PER_LINE}, the unchanged default)"
    );
    println!("  --video <ntsc|pal>           Video standard for the scheduler (default ntsc)");
    println!("  --trace-capacity <count>     Ring capacity in records (default: instruction");
    println!("                               budget, capped at {MAX_DEFAULT_TRACE_CAPACITY})");
    println!(
        "  --ram-bytes <count>          Work-RAM bytes in the state file (default {DEFAULT_TRACE_RAM_BYTES}, max {RAM_SIZE})"
    );
    println!("  --help                       Show this help");
    println!();
    println!("Trace format:");
    println!("  One record per executed instruction, tab-separated, oldest first:");
    println!("    <index>\\t<pc_hex>\\t<opcode_hex>");
    println!("    index        decimal, continues past 0 when the ring overflows");
    println!("    pc_hex       lowercase hexadecimal fetch address, at least 6 digits");
    println!("    opcode_hex   lowercase hexadecimal opcode word, 4 digits");
    println!("  The file has no header. The record is appended after the opcode word is");
    println!("  fetched, so an illegal or unsupported instruction is still recorded.");
    println!();
    println!("Final-state format:");
    println!("  Text, `key=value`, one per line, in this order:");
    println!("    chronofork-megadrive-state v1   (header line)");
    println!("    executed=<decimal>");
    println!("    tick=<decimal>");
    println!("    instructions_per_line=<decimal>");
    println!("    video=<ntsc|pal>");
    println!("    halted=<0|1>");
    println!("    pc=<8 lowercase hex digits>");
    println!("    sr=<4 lowercase hex digits, raw SR>");
    println!("    d0=.. through d7=..             (8 hex digits each)");
    println!("    a0=.. through a7=..             (8 hex digits each)");
    println!("    ram_base=00ff0000");
    println!("    ram_len=<decimal>");
    println!("    ram=<2 hex digits per byte, lowercase>");
    println!();
    println!("Interrupts:");
    println!("  Interrupts use an instruction-quantum model, not cycle-accurate VDP timing:");
    println!(
        "  one executed instruction is one scheduler quantum. The default is {DEFAULT_INSTRUCTIONS_PER_LINE}"
    );
    println!("  instructions per scanline, `--instructions-per-line` changes the model,");
    println!("  and the default is never changed implicitly. Delivery is gated by the 68000");
    println!("  SR interrupt-priority mask: level 7 is always taken, and every other level");
    println!("  is taken only when strictly above the mask. A masked VDP request");
    println!("  stays pending — it is not acknowledged or dropped — until the program");
    println!("  lowers the mask (for example with MOVE.W #imm,SR); handlers return with");
    println!("  RTE, which restores the saved raw SR. VDP interrupt-enable registers and");
    println!("  cycle-accurate timing are not modelled, so this is a mask-gating model, not");
    println!("  a timing-equivalence claim.");
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

    // The demo ROM never writes SR, so the reset interrupt-priority mask 7 keeps
    // both VDP levels masked: the request is retained but never delivered.
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
    ensure(
        pending.machine.registers["irq_mask"] == 7,
        "the reset mask should still be 7",
    )?;
    ensure(
        pending.machine.registers["irq_deliverable"] == 0,
        "mask 7 must block HBlank delivery",
    )?;
    ensure(
        pending.machine.registers["d7"] == 0,
        "no handler may run while masked",
    )?;
    println!(
        "interrupt: masked HBlank pending at tick 64 (level 4, vector 0x70, mask 7, deliverable=0)"
    );

    manager.step(source, InputFrame::NEUTRAL)?;
    let masked_step = manager.inspect(source)?;
    ensure(
        masked_step.machine.registers["irq_pending_level"] == 4,
        "the masked HBlank must stay pending at tick 65",
    )?;
    ensure(
        masked_step.machine.registers["d7"] == 0,
        "the masked HBlank must not run a handler",
    )?;
    println!("interrupt: mask 7 blocked delivery at tick 65; request retained, no handler ran");

    // Fork/restore still reproduces VDP, scheduler and CPU state, including the
    // masked pending request.
    let checkpoint = manager.snapshot(source)?;
    let before = manager.inspect(source)?;
    ensure(
        before.machine.registers["irq_pending_level"] == 4,
        "the checkpoint should hold the pending HBlank",
    )?;
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
    ensure(
        source_view.machine.registers["irq_pending_level"] == 4,
        "the source should keep its masked HBlank",
    )?;
    ensure(
        source_view.machine.registers["d7"] == 0,
        "the source must not run a masked handler",
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
    ensure(
        restored.machine.registers["irq_pending_level"] == 4,
        "restore did not preserve the masked request",
    )?;
    println!(
        "snapshot: VDP, scheduler and CPU state reproduced from the checkpoint (masked HBlank preserved)"
    );

    // The same ROM at a shorter quantum reaches vertical blanking, where VBlank
    // (level 6) is pending but still masked; the VDP status exposes it as bits 3
    // and 7.
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
        vblank_view.machine.tick.raw() == vblank_tick,
        "unexpected tick at vertical blank",
    )?;
    ensure(
        vblank_view.machine.registers["irq_mask"] == 7,
        "vertical blank should still be masked",
    )?;
    ensure(
        vblank_view.machine.registers["irq_pending_level"] == 6,
        "VBlank should be pending with priority over HBlank",
    )?;
    ensure(
        vblank_view.machine.registers["irq_vector"] == 0x78,
        "VBlank should select autovector 0x78",
    )?;
    ensure(
        vblank_view.machine.registers["irq_deliverable"] == 0,
        "mask 7 must block VBlank delivery",
    )?;
    ensure(
        vblank_view.machine.registers["d6"] == 0 && vblank_view.machine.registers["d7"] == 0,
        "no handler may run while vertical blank is masked",
    )?;
    ensure(
        vblank_view.machine.registers["vdp_status"] & 0x0088 == 0x0088,
        "VDP VBlank and VINT-pending status bits should both be set",
    )?;
    let vblank_checkpoint = fast_manager.snapshot(fast)?;
    fast_manager.step(fast, InputFrame::NEUTRAL)?;
    let still_masked = fast_manager.inspect(fast)?;
    ensure(
        still_masked.machine.registers["irq_pending_level"] == 6,
        "the masked VBlank must stay pending",
    )?;
    ensure(
        still_masked.machine.registers["d6"] == 0,
        "the masked VBlank must not run a handler",
    )?;
    fast_manager.restore(fast, vblank_checkpoint)?;
    let restored_vblank = fast_manager.inspect(fast)?;
    ensure(
        restored_vblank.machine.state_hash == vblank_view.machine.state_hash,
        "VBlank checkpoint restore changed the state hash",
    )?;
    ensure(
        restored_vblank.machine.registers["irq_pending_level"] == 6,
        "snapshot restore did not preserve the pending VBlank request",
    )?;
    ensure(
        restored_vblank.machine.registers["vdp_status"] & 0x0080 != 0,
        "snapshot restore did not restore the VINT-pending status bit",
    )?;
    println!(
        "interrupt: masked VBlank pending level 6 vector 0x78 at tick {vblank_tick} (mask 7); VDP VBlank/VINT bits set and snapshot restored them"
    );

    // The interrupt ROM lowers the mask with MOVE.W #imm,SR. The retained HBlank
    // is delivered at the next boundary, its handler runs and RTE restores the
    // interrupted PC with the saved raw SR (mask and CCR).
    let gated_config = SchedulerConfig::new(VideoStandard::Ntsc, 4)?;
    let gated_factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), gated_config)?;
    let mut gated_manager = SessionManager::new(Arc::new(gated_factory));
    let gated = gated_manager.create_session()?;
    for _ in 0..7 {
        gated_manager.step(gated, InputFrame::NEUTRAL)?;
    }
    let masked_hblank = gated_manager.inspect(gated)?;
    ensure(
        masked_hblank.machine.tick.raw() == 7,
        "unexpected tick before the mask is lowered",
    )?;
    ensure(
        masked_hblank.machine.registers["irq_mask"] == 7,
        "the interrupt ROM should still be masked at tick 7",
    )?;
    ensure(
        masked_hblank.machine.registers["irq_pending_level"] == 4,
        "HBlank should be retained while masked",
    )?;
    ensure(
        masked_hblank.machine.registers["d7"] == 0,
        "no handler may run before the mask is lowered",
    )?;
    println!("interrupt: interrupt ROM retained HBlank at tick 7 under mask 7 (no handler)");

    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let lowered = gated_manager.inspect(gated)?;
    ensure(
        lowered.machine.registers["irq_mask"] == 3,
        "MOVE.W #imm,SR should lower the mask to 3",
    )?;
    ensure(
        lowered.machine.registers["irq_pending_level"] == 4,
        "the retained HBlank should still be pending at tick 8",
    )?;
    ensure(
        lowered.machine.registers["irq_deliverable"] == 1,
        "mask 3 should admit HBlank at tick 8",
    )?;
    ensure(
        lowered.machine.registers["d7"] == 0,
        "delivery is evaluated at the next instruction boundary",
    )?;
    println!(
        "interrupt: MOVE.W #$2300,SR lowered the mask to 3 at tick 8; the retained HBlank became deliverable"
    );

    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let delivered_hblank = gated_manager.inspect(gated)?;
    ensure(
        delivered_hblank.machine.registers["irq_pending_level"] == 0,
        "HBlank should be acknowledged on delivery",
    )?;
    ensure(
        delivered_hblank.machine.registers["d7"] == 1,
        "the HBlank handler did not run exactly once",
    )?;
    ensure(
        delivered_hblank.machine.registers["pc"] == 0x2C2,
        "unexpected handler pc right after HBlank delivery",
    )?;
    ensure(
        delivered_hblank.machine.registers["irq_mask"] == 4,
        "exception entry should load IPM 4 into the HBlank handler",
    )?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    ensure(
        gated_manager.inspect(gated)?.machine.registers["sr"] == 0x2408,
        "the HBlank handler should set CCR N on top of handler IPM 4",
    )?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let returned_hblank = gated_manager.inspect(gated)?;
    ensure(
        returned_hblank.machine.registers["pc"] == 0x214,
        "RTE did not return to the interrupted pc",
    )?;
    ensure(
        returned_hblank.machine.registers["sr"] == 0x2300,
        "RTE did not restore the saved raw SR",
    )?;
    ensure(
        returned_hblank.machine.registers["irq_mask"] == 3,
        "RTE did not restore the lowered mask",
    )?;
    ensure(
        returned_hblank.machine.registers["a7"] == 0x0100_0000,
        "RTE did not pop the six-byte frame",
    )?;
    ensure(
        returned_hblank.machine.registers["d7"] == 1,
        "the handler counter should survive RTE",
    )?;
    println!(
        "interrupt: HBlank delivered at tick 9 with handler IPM 4; handler ran; RTE at tick 11 returned to pc=0x214 with SR=0x2300 (mask 3, CCR restored)"
    );

    // At vertical blanking VBlank (level 6) is delivered before the pending
    // HBlank (level 4); the handler runs at IPM 6, so the pending HBlank cannot
    // nest before RTE restores mask 3. Acknowledging VBlank clears the VINT bit.
    for _ in 11..vblank_tick {
        gated_manager.step(gated, InputFrame::NEUTRAL)?;
    }
    let vblank_pending = gated_manager.inspect(gated)?;
    ensure(
        vblank_pending.machine.tick.raw() == vblank_tick,
        "unexpected tick at vertical blank",
    )?;
    ensure(
        vblank_pending.machine.registers["irq_pending_level"] == 6,
        "VBlank should be pending with priority over HBlank",
    )?;
    ensure(
        vblank_pending.machine.registers["irq_deliverable"] == 1,
        "mask 3 should admit VBlank",
    )?;
    ensure(
        vblank_pending.machine.registers["d6"] == 0,
        "the VBlank handler should not have run yet",
    )?;
    ensure(
        vblank_pending.machine.registers["vdp_status"] & 0x0080 != 0,
        "the VDP VINT-pending status bit should be set",
    )?;
    let pre_pc = vblank_pending.machine.registers["pc"];
    let pre_sr = vblank_pending.machine.registers["sr"];
    let d7_before_vblank = vblank_pending.machine.registers["d7"];
    let vblank_gated_checkpoint = gated_manager.snapshot(gated)?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let vblank_delivered = gated_manager.inspect(gated)?;
    ensure(
        vblank_delivered.machine.registers["d6"] == 1,
        "the VBlank handler did not run exactly once",
    )?;
    ensure(
        vblank_delivered.machine.registers["pc"] == 0x2E2,
        "unexpected handler pc right after VBlank delivery",
    )?;
    ensure(
        vblank_delivered.machine.registers["irq_mask"] == 6,
        "exception entry should load IPM 6 into the VBlank handler",
    )?;
    ensure(
        vblank_delivered.machine.registers["irq_pending_level"] == 4,
        "the pending HBlank should survive the VBlank acknowledge",
    )?;
    ensure(
        vblank_delivered.machine.registers["vdp_status"] & 0x0080 == 0,
        "VINT pending should clear when the VBlank request is acknowledged",
    )?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let vblank_body = gated_manager.inspect(gated)?;
    ensure(
        vblank_body.machine.registers["irq_mask"] == 6,
        "the VBlank handler should still run at IPM 6",
    )?;
    ensure(
        vblank_body.machine.registers["irq_pending_level"] == 4,
        "the pending HBlank must stay pending inside the VBlank handler",
    )?;
    ensure(
        vblank_body.machine.registers["d7"] == d7_before_vblank,
        "the pending HBlank must not nest inside the VBlank handler",
    )?;
    ensure(
        vblank_body.machine.registers["a7"] == 0x00FF_FFFA,
        "only the VBlank exception frame should be on the stack",
    )?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let vblank_returned = gated_manager.inspect(gated)?;
    ensure(
        vblank_returned.machine.registers["pc"] == pre_pc,
        "VBlank RTE did not return to the interrupted pc",
    )?;
    ensure(
        vblank_returned.machine.registers["sr"] == pre_sr,
        "VBlank RTE did not restore the saved raw SR",
    )?;
    ensure(
        vblank_returned.machine.registers["irq_mask"] == 3,
        "VBlank RTE did not restore the program mask 3",
    )?;
    ensure(
        vblank_returned.machine.registers["a7"] == 0x0100_0000,
        "VBlank RTE did not pop the six-byte frame",
    )?;
    ensure(
        vblank_returned.machine.registers["d6"] == 1,
        "the VBlank counter should survive RTE",
    )?;

    gated_manager.restore(gated, vblank_gated_checkpoint)?;
    let vblank_restored = gated_manager.inspect(gated)?;
    ensure(
        vblank_restored.machine.state_hash == vblank_pending.machine.state_hash,
        "VBlank checkpoint restore changed the state hash",
    )?;
    ensure(
        vblank_restored.machine.registers["irq_pending_level"] == 6,
        "restore did not preserve the pending VBlank request",
    )?;
    ensure(
        vblank_restored.machine.registers["vdp_status"] & 0x0080 != 0,
        "restore did not recompute the VINT-pending status bit",
    )?;
    println!(
        "interrupt: VBlank entered at IPM 6 and blocked the pending HBlank (no nested handler); RTE returned to pc=0x{pre_pc:X} with SR=0x{pre_sr:04X} (mask restored); snapshot preserved the pending VBlank/VINT state"
    );

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
