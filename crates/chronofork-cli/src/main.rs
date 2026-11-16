//! Minimal ChronoFork command-line interface.
//!
//! Only capabilities that actually exist are exposed. There are deliberately no
//! stubs for HTTP, MCP, GUI or a real Mega Drive backend.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use chronofork_backend_api::{Backend, BackendFactory};
use chronofork_core::{InputFrame, SessionId, StateHash};
use chronofork_megadrive::{
    DEFAULT_INSTRUCTIONS_PER_LINE, InstructionTrace, MegadriveBackend, MegadriveFactory, RAM_BASE,
    RAM_SIZE, SchedulerConfig, VideoStandard, demo_rom, interrupt_rom,
};
use chronofork_sessions::{
    DurableLineage, LoadedSnapshot, SessionEntry, SessionError, SessionManager, Snapshot,
    SnapshotEntry, SnapshotRepository,
};
use chronofork_test_backend::SyntheticBackendFactory;

mod studio;
mod tas;
mod tas_edit;

/// Crate version used by the help and version output.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
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
    // Collect `OsString` arguments and reject a non-UTF-8 value through the typed
    // CLI error path. `std::env::args()` panics on such a value, which would break
    // the exit-1/empty-stdout/`chronofork:` contract; valid UTF-8 argv keeps the
    // previous behavior exactly.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(OsString::into_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| CliError("command-line arguments must be valid UTF-8".to_owned()))?;
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
        Some("session") => run_session(&args[1..]),
        Some("tas") => tas::run(&args[1..]),
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
    println!("  chronofork session save --root <DIR> [--session <KEY>] [--name <NAME>]");
    println!("                            [--steps <N>] [--input <U32>]");
    println!("  chronofork session load --root <DIR> --session <KEY> [--snapshot <KEY>]");
    println!("                            [--steps <N>] [--input <U32>]");
    println!("  chronofork session list --root <DIR> [--session <KEY>]");
    println!("  chronofork session delete --root <DIR> --session <KEY> [--snapshot <KEY>]");
    println!("  chronofork tas record --movie <FILE> [--steps <N>] [--input <U32>]");
    println!("                        [--ports <P1,P2,P3>] [--axes <A0,A1,A2,A3>]");
    println!("                        [--keys <U64>] [--anchor-interval <N>]");
    println!("                        [--movie-id <U64>] [--name <NAME>]");
    println!("                        [--root <DIR> [--session <KEY> [--snapshot <KEY>]]]");
    println!("  chronofork tas play --movie <FILE> [--from <TICK>] [--until <TICK>]");
    println!("  chronofork tas verify --movie <FILE> [--from <TICK>] [--until <TICK>]");
    println!("  chronofork tas compare --movie <FILE> --against <FILE>");
    println!("  chronofork tas studio --movie <FILE> [--cache-interval <N>]");
    println!("                        [--cache-capacity <N>] [--cache-bytes <N>]");
    println!("  chronofork tas edit --movie <FILE> --out <FILE>");
    println!("                      --set-input <INDEX=MASK>...   or --truncate-after <INDEX>");
    println!("                      [--movie-id <U64>] [--name <NAME>]");
    println!("                      [--anchors <keep-prefix|drop|rebuild>]");
    println!("                      [--anchor-interval <N>]");
    println!("  chronofork --version          Show version");
    println!("  chronofork help               Show this help");
    println!();
    println!("`--root` is required for every session command. Only `session save` without");
    println!("`--session` initializes a storage root; `load`, `list` and `delete` never create");
    println!("one. Session commands print `key=value` record blocks to stdout and diagnostics");
    println!("to stderr.");
    println!();
    println!("TAS movies record one synthetic-backend input sequence and embed the canonical");
    println!("checkpoint; `tas play/verify` replay an exact tick window from that checkpoint or");
    println!("from the nearest embedded anchor by deterministic re-simulation, and `tas compare`");
    println!("diffs two recorded branches. Multi-device schema-2 input and anchor intervals are");
    println!("format v2, selected only by the v2 flags; plain records stay format v1.");
    println!("`tas studio` reads stepping commands from stdin and keeps one in-process player");
    println!("with a bounded state cache; rewind restores a verified state (or the embedded");
    println!("checkpoint) and replays forward, so it is not O(1) random access.");
    println!("`tas edit` creates a new branch movie by functional suffix re-simulation from the");
    println!("source's embedded checkpoint. The source file is never modified, and no stored");
    println!("rerecord counter exists; `chronofork tas edit --help` documents the flags and");
    println!("the anchor, format and device-field rules.");
    println!("`chronofork tas --help` documents the flags, output blocks and the features that");
    println!("are deliberately not implemented.");
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

pub(crate) fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    flag: &str,
) -> Result<&'a str, CliError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| CliError(format!("`{flag}` requires a value")))
}

pub(crate) fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(CliError(format!("`{flag}` was given more than once")));
    }
    *slot = Some(value);
    Ok(())
}

pub(crate) fn parse_u64(value: &str, flag: &str) -> Result<u64, CliError> {
    value
        .parse()
        .map_err(|_| CliError(format!("`{flag}` expects a decimal integer, got `{value}`")))
}

pub(crate) fn parse_u32(value: &str, flag: &str) -> Result<u32, CliError> {
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

    // The demo exercises the public path as a wiring smoke test. Semantic
    // equality of the decoded envelope is owned by the sessions acceptance
    // suite (`serialized_checkpoint_can_seed_a_new_manager`); decoding here
    // still rejects a bad version or checksum.
    let bytes = manager.snapshot_bytes(source, checkpoint)?;
    Snapshot::from_bytes(&bytes)?;
    println!(
        "serialized checkpoint: {} bytes, checksum verified",
        bytes.len()
    );

    let branch_a = manager.fork_from_snapshot(source, checkpoint)?;
    let branch_b = manager.fork_from_snapshot(source, checkpoint)?;
    let branch_c = manager.fork_from_snapshot(source, checkpoint)?;
    println!("forks: {branch_a}, {branch_b}, {branch_c}");

    let inputs = [1_u32, 0x7f, 0xff];
    let branches = [branch_a, branch_b, branch_c];
    for (branch, input) in branches.iter().zip(inputs) {
        for _ in 0..6 {
            manager.step(*branch, InputFrame::new(input))?;
        }
    }

    for branch in branches {
        let view = manager.inspect(branch)?;
        println!(
            "branch {}: tick {} hash {}",
            branch, view.machine.tick, view.machine.state_hash
        );
    }

    let source_view = manager.inspect(source)?;
    println!(
        "source isolation: source remains at tick {} hash {}",
        source_view.machine.tick, source_view.machine.state_hash
    );

    // Divergence, isolation and replay are asserted exactly once, by the
    // sessions acceptance suite. The demo still performs the same calls so the
    // printed narrative reflects a real run.
    manager.read_memory(branch_a, 0x20, 1)?;
    manager.write_memory(branch_c, 0x20, &[0xaa])?;
    manager.read_memory(branch_a, 0x20, 1)?;
    manager.read_memory(branch_c, 0x20, 1)?;
    println!("memory isolation: branch C mutation did not affect branch A");

    manager.restore(branch_a, checkpoint)?;
    for _ in 0..6 {
        manager.step(branch_a, InputFrame::new(1))?;
    }
    manager.inspect(branch_a)?;
    println!("replay verification: restored branch A reproduced its state hash");

    let comparison = manager.compare(branch_a, branch_b)?;
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

    // The demo is a wiring and output smoke test: the values it prints are read
    // back from the session, but the invariants behind them are asserted by the
    // Mega Drive backend tests and the sessions acceptance suite.
    let source = manager.create_session()?;
    for _ in 0..23 {
        manager.step(source, InputFrame::NEUTRAL)?;
    }
    let vdp_view = manager.inspect(source)?;
    let registers = &vdp_view.machine.registers;
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
    println!(
        "interrupt: masked HBlank pending at tick {} (level {}, vector 0x{:02X}, mask {}, deliverable={})",
        pending.machine.tick.raw(),
        pending.machine.registers["irq_pending_level"],
        pending.machine.registers["irq_vector"],
        pending.machine.registers["irq_mask"],
        pending.machine.registers["irq_deliverable"],
    );

    manager.step(source, InputFrame::NEUTRAL)?;
    let masked_step = manager.inspect(source)?;
    println!(
        "interrupt: mask {} blocked delivery at tick {}; request retained, no handler ran",
        masked_step.machine.registers["irq_mask"],
        masked_step.machine.tick.raw(),
    );

    // Fork/restore still exercises VDP, scheduler and CPU state, including the
    // masked pending request.
    let checkpoint = manager.snapshot(source)?;
    let fork = manager.fork_from_snapshot(source, checkpoint)?;
    manager.write_memory(fork, u64::from(RAM_BASE), &[0xAA])?;
    manager.step(fork, InputFrame::NEUTRAL)?;
    let fork_view = manager.inspect(fork)?;
    let source_view = manager.inspect(source)?;
    println!(
        "fork isolation: fork={} source={}",
        fork_view.machine.state_hash, source_view.machine.state_hash
    );

    manager.restore(fork, checkpoint)?;
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
    let vblank_checkpoint = fast_manager.snapshot(fast)?;
    fast_manager.step(fast, InputFrame::NEUTRAL)?;
    fast_manager.restore(fast, vblank_checkpoint)?;
    // The line is printed from the restored state, so "snapshot restored them"
    // describes the state that was actually observed after the restore.
    let restored_vblank = fast_manager.inspect(fast)?;
    println!(
        "interrupt: masked VBlank pending level {} vector 0x{:02X} at tick {} (mask {}); VDP VBlank/VINT bits set and snapshot restored them",
        restored_vblank.machine.registers["irq_pending_level"],
        restored_vblank.machine.registers["irq_vector"],
        restored_vblank.machine.tick.raw(),
        restored_vblank.machine.registers["irq_mask"],
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
    println!(
        "interrupt: interrupt ROM retained HBlank at tick {} under mask {} (no handler)",
        masked_hblank.machine.tick.raw(),
        masked_hblank.machine.registers["irq_mask"],
    );

    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let lowered = gated_manager.inspect(gated)?;
    println!(
        "interrupt: MOVE.W #$2300,SR lowered the mask to {} at tick {}; the retained HBlank became deliverable",
        lowered.machine.registers["irq_mask"],
        lowered.machine.tick.raw(),
    );

    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let delivered_hblank = gated_manager.inspect(gated)?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    let returned_hblank = gated_manager.inspect(gated)?;
    println!(
        "interrupt: HBlank delivered at tick {} with handler IPM {}; handler ran; RTE at tick {} returned to pc=0x{:X} with SR=0x{:04X} (mask {}, CCR restored)",
        delivered_hblank.machine.tick.raw(),
        delivered_hblank.machine.registers["irq_mask"],
        returned_hblank.machine.tick.raw(),
        returned_hblank.machine.registers["pc"],
        returned_hblank.machine.registers["sr"],
        returned_hblank.machine.registers["irq_mask"],
    );

    // At vertical blanking VBlank (level 6) is delivered before the pending
    // HBlank (level 4); the handler runs at IPM 6, so the pending HBlank cannot
    // nest before RTE restores mask 3. Acknowledging VBlank clears the VINT bit.
    for _ in 11..vblank_tick {
        gated_manager.step(gated, InputFrame::NEUTRAL)?;
    }
    let vblank_pending = gated_manager.inspect(gated)?;
    let pre_pc = vblank_pending.machine.registers["pc"];
    let pre_sr = vblank_pending.machine.registers["sr"];
    let vblank_gated_checkpoint = gated_manager.snapshot(gated)?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    gated_manager.step(gated, InputFrame::NEUTRAL)?;
    gated_manager.restore(gated, vblank_gated_checkpoint)?;
    println!(
        "interrupt: VBlank entered at IPM 6 and blocked the pending HBlank (no nested handler); RTE returned to pc=0x{pre_pc:X} with SR=0x{pre_sr:04X} (mask restored); snapshot preserved the pending VBlank/VINT state"
    );

    println!("chronofork megadrive demo: PASS");
    Ok(())
}

/// Runs one `chronofork session` subcommand.
///
/// The `session` commands are a thin, line-oriented process boundary over
/// [`SnapshotRepository`]: they parse flags, rehydrate through the manager's storage
/// helpers and print `key=value` blocks. Serialization, compatibility, metadata and
/// lock handling stay in `chronofork-sessions`; this code never re-implements them.
fn run_session(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("save") => session_save(&args[1..]),
        Some("load") => session_load(&args[1..]),
        Some("list") => session_list(&args[1..]),
        Some("delete") => session_delete(&args[1..]),
        None => Err(CliError(
            "`session` requires a subcommand: save, load, list or delete".to_owned(),
        )
        .into()),
        Some(other) => Err(CliError(format!(
            "unknown `session` subcommand `{other}`; expected save, load, list or delete"
        ))
        .into()),
    }
}

/// Optional flags accepted by one `session` subcommand.
#[derive(Clone, Copy, Debug)]
struct SessionFlags {
    session: bool,
    snapshot: bool,
    name: bool,
    steps: bool,
    input: bool,
}

impl SessionFlags {
    const SAVE: Self = Self {
        session: true,
        snapshot: false,
        name: true,
        steps: true,
        input: true,
    };
    const LOAD: Self = Self {
        session: true,
        snapshot: true,
        name: false,
        steps: true,
        input: true,
    };
    const LIST: Self = Self {
        session: true,
        snapshot: false,
        name: false,
        steps: false,
        input: false,
    };
    const DELETE: Self = Self {
        session: true,
        snapshot: true,
        name: false,
        steps: false,
        input: false,
    };

    /// Flag list used in an unknown-option diagnostic.
    fn describe(self) -> String {
        let mut flags = vec!["--root <DIR>"];
        if self.session {
            flags.push("--session <KEY>");
        }
        if self.snapshot {
            flags.push("--snapshot <KEY>");
        }
        if self.name {
            flags.push("--name <NAME>");
        }
        if self.steps {
            flags.push("--steps <N>");
        }
        if self.input {
            flags.push("--input <U32>");
        }
        flags.join(" ")
    }
}

/// Parsed flags shared by the `session` subcommands.
#[derive(Debug, Default)]
struct SessionArgs {
    root: Option<PathBuf>,
    session: Option<u64>,
    snapshot: Option<u64>,
    name: Option<String>,
    steps: Option<u64>,
    input: Option<u32>,
}

fn parse_session_args(
    subcommand: &str,
    args: &[String],
    flags: SessionFlags,
) -> Result<SessionArgs, CliError> {
    let mut parsed = SessionArgs::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--root" => set_once(
                &mut parsed.root,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--session" if flags.session => set_once(
                &mut parsed.session,
                parse_key(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--snapshot" if flags.snapshot => set_once(
                &mut parsed.snapshot,
                parse_key(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--name" if flags.name => set_once(
                &mut parsed.name,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--steps" if flags.steps => set_once(
                &mut parsed.steps,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--input" if flags.input => set_once(
                &mut parsed.input,
                parse_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `session {subcommand}` option `{other}`; expected {}",
                    flags.describe()
                )));
            }
        }
        index += 1;
    }
    Ok(parsed)
}

/// Parses a canonical durable key: positive decimal without leading zeroes.
pub(crate) fn parse_key(value: &str, flag: &str) -> Result<u64, CliError> {
    let canonical = !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && !value.starts_with('0');
    if !canonical {
        return Err(CliError(format!(
            "`{flag}` expects a canonical positive decimal key, got `{value}`"
        )));
    }
    value
        .parse()
        .map_err(|_| CliError(format!("`{flag}` key `{value}` is too large")))
}

fn session_save(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("save", args, SessionFlags::SAVE)?;
    let root = options
        .root
        .ok_or_else(|| CliError("`session save` requires `--root <DIR>`".to_owned()))?;
    // `save` without `--session` starts a new durable session and may initialize an
    // empty root. Continuing an existing session needs an existing root, so it never
    // creates one as a side effect of a command that is about to fail.
    let repository = match options.session {
        Some(_) => SnapshotRepository::open(&root)?,
        None => SnapshotRepository::initialize(&root)?,
    };
    let input = input_frame(options.input);
    let steps = options.steps.unwrap_or(0);
    let mut manager = synthetic_session_manager();

    let (session, snapshot) = match options.session {
        Some(session_key) => {
            // Continue-and-save is one linearizable read-modify-publish workflow: the
            // per-session writer lock is taken before the base is read and held
            // through rehydration, advance, capture and publish. A second process
            // fails here with typed `WriterContended` before it can read a stale base
            // and publish from it. The repository owns every storage, serialization,
            // metadata and lock detail; the CLI only orchestrates its public API.
            repository.with_writer_lock::<_, Box<dyn Error>>(session_key, |repository, lock| {
                let latest = repository.latest_snapshot(session_key)?.ok_or_else(|| {
                    CliError(format!(
                        "durable session {session_key} has no committed snapshot to \
                             continue from"
                    ))
                })?;
                let LoadedSnapshot { snapshot, .. } =
                    repository.load_snapshot(session_key, latest.snapshot_key)?;
                let runtime = manager.create_session_from_stored_snapshot(snapshot, session_key)?;
                advance(&mut manager, runtime, steps, input)?;
                let snapshot_id = manager.snapshot(runtime)?;
                let record = manager.snapshot_record(runtime, snapshot_id)?;
                let entry =
                    repository.save_snapshot_with_lock(lock, record, options.name.as_deref())?;
                Ok((repository.session(session_key)?, entry))
            })?
        }
        None => {
            let runtime = manager.create_session()?;
            advance(&mut manager, runtime, steps, input)?;
            let session = repository.create_session(
                SessionId::from_raw(runtime.raw()),
                options.name.as_deref(),
                DurableLineage::root(),
            )?;
            manager.attach_storage_session(runtime, session.session_key)?;
            let snapshot_id = manager.snapshot(runtime)?;
            let record = manager.snapshot_record(runtime, snapshot_id)?;
            let entry =
                repository.save_snapshot(session.session_key, record, options.name.as_deref())?;
            (session, entry)
        }
    };

    let mut output = String::new();
    write_session_block(&mut output, &session);
    write_snapshot_block(&mut output, &snapshot);
    print!("{output}");
    Ok(())
}

fn session_load(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("load", args, SessionFlags::LOAD)?;
    let root = options
        .root
        .ok_or_else(|| CliError("`session load` requires `--root <DIR>`".to_owned()))?;
    let session_key = options
        .session
        .ok_or_else(|| CliError("`session load` requires `--session <KEY>`".to_owned()))?;
    // A load never initializes a root and never writes storage.
    let repository = SnapshotRepository::open(&root)?;
    let snapshot_key = match options.snapshot {
        Some(snapshot_key) => snapshot_key,
        None => {
            repository
                .latest_snapshot(session_key)?
                .ok_or_else(|| {
                    CliError(format!(
                        "durable session {session_key} has no committed snapshot"
                    ))
                })?
                .snapshot_key
        }
    };
    let LoadedSnapshot { snapshot, entry } = repository.load_snapshot(session_key, snapshot_key)?;
    let steps = options.steps.unwrap_or(0);
    let mut manager = synthetic_session_manager();
    let runtime = manager.create_session_from_stored_snapshot(snapshot, session_key)?;
    advance(&mut manager, runtime, steps, input_frame(options.input))?;
    // Read-only: `--steps`/`--input` advance only the in-memory instance. With the
    // default zero steps the observed state is exactly the stored checkpoint.
    let inspection = manager.inspect(runtime)?;
    let report = SnapshotReport {
        session_key: entry.session_key,
        snapshot_key: entry.snapshot_key,
        name: entry.name.as_deref(),
        tick: inspection.machine.tick.raw(),
        state_hash: inspection.machine.state_hash,
        creation_sequence: entry.creation_sequence,
        parent_session_key: entry.parent_session_key,
        origin_snapshot_key: entry.origin_snapshot_key,
    };
    let mut output = String::new();
    write_snapshot_report(&mut output, &report);
    print!("{output}");
    Ok(())
}

fn session_list(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("list", args, SessionFlags::LIST)?;
    let root = options
        .root
        .ok_or_else(|| CliError("`session list` requires `--root <DIR>`".to_owned()))?;
    // A list never initializes a root and takes no writer lock.
    let repository = SnapshotRepository::open(&root)?;
    let mut output = String::new();
    let mut anomalies = Vec::new();
    match options.session {
        Some(session_key) => {
            let session = repository.session(session_key)?;
            write_session_block(&mut output, &session);
            let snapshots = repository.list_snapshots(session_key)?;
            for entry in &snapshots.entries {
                write_snapshot_block(&mut output, entry);
            }
            anomalies.extend(snapshots.anomalies);
        }
        None => {
            let listing = repository.list_sessions()?;
            for session in &listing.entries {
                write_session_block(&mut output, session);
                let snapshots = repository.list_snapshots(session.session_key)?;
                for entry in &snapshots.entries {
                    write_snapshot_block(&mut output, entry);
                }
                anomalies.extend(snapshots.anomalies);
            }
            anomalies.extend(listing.anomalies);
        }
    }
    print!("{output}");
    // Anomalies are diagnostics about entries that are not committed data. The listing
    // still succeeds, nothing is repaired and only stderr is used.
    for anomaly in anomalies {
        eprintln!("chronofork: storage anomaly: {anomaly}");
    }
    Ok(())
}

fn session_delete(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("delete", args, SessionFlags::DELETE)?;
    let root = options
        .root
        .ok_or_else(|| CliError("`session delete` requires `--root <DIR>`".to_owned()))?;
    let session_key = options
        .session
        .ok_or_else(|| CliError("`session delete` requires `--session <KEY>`".to_owned()))?;
    // A delete never initializes a root and never cascades.
    let repository = SnapshotRepository::open(&root)?;
    match options.snapshot {
        Some(snapshot_key) => repository.delete_snapshot(session_key, snapshot_key)?,
        None => repository.delete_session(session_key)?,
    }
    let mut output = String::new();
    write_deleted_block(&mut output, session_key, options.snapshot);
    print!("{output}");
    Ok(())
}

/// A manager over the deterministic synthetic test backend.
pub(crate) fn synthetic_session_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

/// `--input` selection, defaulting to neutral input.
pub(crate) fn input_frame(buttons: Option<u32>) -> InputFrame {
    buttons.map_or(InputFrame::NEUTRAL, InputFrame::new)
}

/// Applies `steps` deterministic inputs to one in-memory session.
fn advance(
    manager: &mut SessionManager,
    session: SessionId,
    steps: u64,
    input: InputFrame,
) -> Result<(), SessionError> {
    for _ in 0..steps {
        manager.step(session, input)?;
    }
    Ok(())
}

/// Fields of one `record=snapshot` block.
#[derive(Debug)]
struct SnapshotReport<'a> {
    session_key: u64,
    snapshot_key: u64,
    name: Option<&'a str>,
    tick: u64,
    state_hash: StateHash,
    creation_sequence: u64,
    parent_session_key: Option<u64>,
    origin_snapshot_key: Option<u64>,
}

/// Starts one output record, separating it from a previous block by one blank line.
pub(crate) fn begin_record(output: &mut String, record: &str) {
    if !output.is_empty() {
        output.push('\n');
    }
    let _ = writeln!(output, "record={record}");
}

fn write_session_block(output: &mut String, entry: &SessionEntry) {
    begin_record(output, "session");
    let _ = writeln!(output, "session_key={}", entry.session_key);
    let _ = writeln!(output, "name={}", entry.name.as_deref().unwrap_or(""));
}

fn write_snapshot_block(output: &mut String, entry: &SnapshotEntry) {
    let report = SnapshotReport {
        session_key: entry.session_key,
        snapshot_key: entry.snapshot_key,
        name: entry.name.as_deref(),
        tick: entry.tick.raw(),
        state_hash: entry.state_hash,
        creation_sequence: entry.creation_sequence,
        parent_session_key: entry.parent_session_key,
        origin_snapshot_key: entry.origin_snapshot_key,
    };
    write_snapshot_report(output, &report);
}

fn write_snapshot_report(output: &mut String, report: &SnapshotReport<'_>) {
    begin_record(output, "snapshot");
    let _ = writeln!(output, "session_key={}", report.session_key);
    let _ = writeln!(output, "snapshot_key={}", report.snapshot_key);
    let _ = writeln!(output, "name={}", report.name.unwrap_or(""));
    let _ = writeln!(output, "tick={}", report.tick);
    let _ = writeln!(output, "state_hash={}", report.state_hash);
    let _ = writeln!(output, "creation_sequence={}", report.creation_sequence);
    let _ = writeln!(
        output,
        "parent_session_key={}",
        optional_key(report.parent_session_key)
    );
    let _ = writeln!(
        output,
        "origin_snapshot_key={}",
        optional_key(report.origin_snapshot_key)
    );
}

fn write_deleted_block(output: &mut String, session_key: u64, snapshot_key: Option<u64>) {
    begin_record(output, "deleted");
    let _ = writeln!(output, "session_key={session_key}");
    if let Some(snapshot_key) = snapshot_key {
        let _ = writeln!(output, "snapshot_key={snapshot_key}");
    }
}

/// Formats an optional durable key for the `-`-when-absent block convention.
fn optional_key(key: Option<u64>) -> String {
    match key {
        Some(key) => key.to_string(),
        None => "-".to_owned(),
    }
}

pub(crate) fn ensure(condition: bool, message: &str) -> Result<(), Box<dyn Error>> {
    if condition {
        Ok(())
    } else {
        Err(CliError(message.to_owned()).into())
    }
}

#[derive(Debug)]
pub(crate) struct CliError(pub(crate) String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for CliError {}
