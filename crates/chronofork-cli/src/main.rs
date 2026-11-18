//! ChronoFork command-line interface.
//!
//! Only capabilities that actually exist are exposed. Stage 6 adds the agent-native
//! surfaces (`serve`, `api`, `mcp`, `experiment`) and routes the durable `session`
//! commands through `chronofork-service`; these are clients and hosts of the shared
//! service, not a second implementation of it. There are deliberately no stubs for a
//! GUI or a complete emulator.

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
use chronofork_core::InputFrame;
use chronofork_megadrive::cycles::MASTER_CYCLES_PER_LINE;
use chronofork_megadrive::{
    Cartridge, InstructionTrace, MegadriveBackend, MegadriveConfig, MegadriveFactory, RAM_BASE,
    RAM_SIZE, SchedulerConfig, TmssModel, VideoStandard, boot_marker, demo_rom, interrupt_rom,
};
use chronofork_sessions::{SessionManager, Snapshot};
use chronofork_test_backend::SyntheticBackendFactory;

mod agent;
mod debug;
mod gui;
mod script;
mod session;
mod studio;
mod tas;
mod tas_edit;
mod trace;

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
            // A script-level failed run already printed its report, and a typed remote
            // failure already printed its canonical envelope: both contracts keep
            // stderr to their own single diagnostic without a `chronofork:` line.
            if error.downcast_ref::<script::ScriptFailed>().is_none()
                && error.downcast_ref::<agent::FailureReported>().is_none()
            {
                eprintln!("chronofork: {error}");
            }
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
        Some("serve") => agent::run_serve(&args[1..]),
        Some("api") => agent::run_api(&args[1..]),
        Some("mcp") => agent::run_mcp(&args[1..]),
        Some("experiment") => agent::run_experiment(&args[1..]),
        Some("gui") => gui::run(&args[1..]),
        Some("session") => session::run(&args[1..]),
        Some("tas") => tas::run(&args[1..]),
        Some("trace") => trace::run(&args[1..]),
        Some("script") => script::run(&args[1..]),
        Some("debug") => debug::run(&args[1..]),
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
        Some("cartridge") => run_megadrive_cartridge(&args[1..]),
        Some("boot") => run_megadrive_boot(&args[1..]),
        None => Err(CliError(
            "`megadrive` requires a subcommand: demo, trace, cartridge or boot".to_owned(),
        )
        .into()),
        Some(other) => Err(CliError(format!(
            "unknown `megadrive` subcommand `{other}`; expected demo, trace, cartridge or boot"
        ))
        .into()),
    }
}

/// Options for `megadrive cartridge`.
#[derive(Debug)]
struct CartridgeOptions {
    rom: PathBuf,
    sram_in: Option<PathBuf>,
    sram_out: Option<PathBuf>,
}

/// Parses `megadrive cartridge` arguments.
///
/// Returns `Ok(None)` when the caller should print the detailed help text.
fn parse_cartridge_options(args: &[String]) -> Result<Option<CartridgeOptions>, Box<dyn Error>> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        ensure(
            args.len() == 1,
            "`megadrive cartridge --help` takes no other arguments",
        )?;
        return Ok(None);
    }
    let mut rom: Option<PathBuf> = None;
    let mut sram_in: Option<PathBuf> = None;
    let mut sram_out: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--rom" => set_once(
                &mut rom,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--sram-in" => set_once(
                &mut sram_in,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--sram-out" => set_once(
                &mut sram_out,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `megadrive cartridge` option `{other}`; see `chronofork megadrive cartridge --help`"
                ))
                .into());
            }
        }
        index += 1;
    }
    let rom =
        rom.ok_or_else(|| CliError("`megadrive cartridge` requires `--rom <FILE>`".to_owned()))?;
    if sram_in.is_some() && sram_in == sram_out {
        return Err(
            CliError("`--sram-in` and `--sram-out` must name different files".to_owned()).into(),
        );
    }
    Ok(Some(CartridgeOptions {
        rom,
        sram_in,
        sram_out,
    }))
}

fn run_megadrive_cartridge(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_cartridge_options(args)? else {
        print_cartridge_help();
        return Ok(());
    };
    let bytes = crate::tas::read_rom_file_with_limit(
        &options.rom,
        u64::try_from(chronofork_megadrive::MAX_IMAGE_SIZE)?,
        "64 MiB mapper ceiling",
    )?;
    let mut cartridge = Cartridge::parse(bytes).map_err(|error| {
        CliError(format!(
            "invalid cartridge `{}`: {error}",
            options.rom.display()
        ))
    })?;
    if let Some(path) = &options.sram_in {
        let data = std::fs::read(path).map_err(|error| {
            CliError(format!("failed to read SRAM `{}`: {error}", path.display()))
        })?;
        cartridge.import_sram(&data).map_err(|error| {
            CliError(format!(
                "failed to import SRAM `{}`: {error}",
                path.display()
            ))
        })?;
    }
    let header = cartridge.header();
    println!("system={}", header.system.as_str());
    println!("region={}", header.region);
    println!("serial={}", header.serial);
    println!("domestic_name={}", header.domestic_name);
    println!("overseas_name={}", header.overseas_name);
    println!("rom_start=0x{:06X}", header.rom_start);
    println!("rom_end=0x{:06X}", header.rom_end);
    println!("checksum_stored=0x{:04X}", header.checksum_stored);
    println!("checksum_computed=0x{:04X}", header.checksum_computed);
    println!(
        "checksum_ok={}",
        header.checksum_stored == header.checksum_computed
    );
    println!("sram_declared={}", header.sram.is_some());
    let banks = cartridge
        .banks()
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    println!("mapper_banks={banks}");
    if let Some(path) = &options.sram_out {
        std::fs::write(path, cartridge.sram()).map_err(|error| {
            CliError(format!(
                "failed to write SRAM `{}`: {error}",
                path.display()
            ))
        })?;
    }
    println!("PASS");
    Ok(())
}

fn print_cartridge_help() {
    println!("chronofork {VERSION} — megadrive cartridge");
    println!();
    println!("Parses a Mega Drive cartridge header and reports its facts as");
    println!("`key=value` lines, then `PASS`.");
    println!();
    println!("Usage:");
    println!(
        "  chronofork megadrive cartridge --rom <FILE> [--sram-in <FILE>] [--sram-out <FILE>]"
    );
    println!();
    println!("Options:");
    println!("  --rom <FILE>       Required cartridge image");
    println!("  --sram-in <FILE>   Import exactly 65536 bytes of battery SRAM");
    println!("  --sram-out <FILE>  Export the current 65536-byte battery SRAM");
    println!("  --help             Show this help");
    println!();
    println!("Fields: system, region, serial, domestic_name, overseas_name, rom_start,");
    println!("rom_end, checksum_stored, checksum_computed, checksum_ok, sram_declared,");
    println!("mapper_banks. The checksum is the Mega Drive word sum over 0x200..");
}

/// Options for `megadrive boot`.
#[derive(Debug)]
struct BootOptions {
    rom: PathBuf,
    frames: u64,
    tmss: TmssModel,
}

/// Default bounded frame budget for `megadrive boot`.
const DEFAULT_BOOT_FRAMES: u64 = 2;

/// Parses `megadrive boot` arguments.
///
/// Returns `Ok(None)` when the caller should print the detailed help text.
fn parse_boot_options(args: &[String]) -> Result<Option<BootOptions>, Box<dyn Error>> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        ensure(
            args.len() == 1,
            "`megadrive boot --help` takes no other arguments",
        )?;
        return Ok(None);
    }
    let mut rom: Option<PathBuf> = None;
    let mut frames: Option<u64> = None;
    let mut tmss: Option<TmssModel> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--rom" => set_once(
                &mut rom,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--frames" => {
                let value = parse_u64(option_value(args, &mut index, &flag)?, &flag)?;
                if value == 0 {
                    return Err(CliError(
                        "`--frames` must be at least 1 (the frame budget is bounded)".to_owned(),
                    )
                    .into());
                }
                set_once(&mut frames, value, &flag)?;
            }
            "--tmss" => set_once(
                &mut tmss,
                parse_tmss(option_value(args, &mut index, &flag)?)?,
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `megadrive boot` option `{other}`; see `chronofork megadrive boot --help`"
                ))
                .into());
            }
        }
        index += 1;
    }
    let rom = rom.ok_or_else(|| CliError("`megadrive boot` requires `--rom <FILE>`".to_owned()))?;
    Ok(Some(BootOptions {
        rom,
        frames: frames.unwrap_or(DEFAULT_BOOT_FRAMES),
        tmss: tmss.unwrap_or(TmssModel::Disabled),
    }))
}

fn parse_tmss(value: &str) -> Result<TmssModel, CliError> {
    match value {
        "enabled" => Ok(TmssModel::Enabled),
        "disabled" => Ok(TmssModel::Disabled),
        other => Err(CliError(format!(
            "`--tmss` expects `enabled` or `disabled`, got `{other}`"
        ))),
    }
}

fn run_megadrive_boot(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_boot_options(args)? else {
        print_boot_help();
        return Ok(());
    };
    let bytes = crate::tas::read_rom_file_with_limit(
        &options.rom,
        u64::try_from(chronofork_megadrive::MAX_IMAGE_SIZE)?,
        "64 MiB mapper ceiling",
    )?;
    let factory = MegadriveFactory::with_config(
        bytes,
        MegadriveConfig {
            video: None,
            tmss: options.tmss,
        },
    )?;
    let mut machine = factory.create_machine()?;
    let scheduler = machine.scheduler_config();
    let budget = options
        .frames
        .saturating_mul(u64::from(scheduler.lines_per_frame()))
        .saturating_mul(MASTER_CYCLES_PER_LINE);
    let marker = boot_marker();
    let mut executed = 0_u64;
    let mut reached = machine.cpu().pc() == marker;
    while !reached && machine.master_cycles() < budget {
        machine.step(InputFrame::NEUTRAL)?;
        executed += 1;
        reached = machine.cpu().pc() == marker;
    }
    let inspection = machine.inspect()?;
    println!("reached_marker={reached}");
    println!("pc=0x{:08X}", machine.cpu().pc());
    println!("tick={}", machine.tick().raw());
    println!("state_hash={}", inspection.state_hash);
    println!("frames={}", options.frames);
    println!("instructions={executed}");
    println!(
        "tmss={}",
        match options.tmss {
            TmssModel::Enabled => "enabled",
            TmssModel::Disabled => "disabled",
        }
    );
    if !reached {
        return Err(CliError(format!(
            "boot marker 0x{marker:08X} not reached within {executed} instruction(s) ({}) frame(s)",
            options.frames
        ))
        .into());
    }
    println!("PASS");
    Ok(())
}

fn print_boot_help() {
    println!("chronofork {VERSION} — megadrive boot");
    println!();
    println!("Steps a bounded frame budget and reports whether the synthetic boot");
    println!("marker is reached. Exits non-zero when the marker is not reached.");
    println!();
    println!("Usage:");
    println!("  chronofork megadrive boot --rom <FILE> [--frames <N>] [--tmss enabled|disabled]");
    println!();
    println!("Options:");
    println!("  --rom <FILE>         Required cartridge image");
    println!(
        "  --frames <N>         Bounded frame budget, at least 1 (default {DEFAULT_BOOT_FRAMES})"
    );
    println!("  --tmss <mode>        enabled or disabled (default disabled)");
    println!("  --help               Show this help");
    println!();
    println!("Fields: reached_marker, pc, tick, state_hash, frames, instructions, tmss.");
    println!(
        "The marker address is {:#08X} of the synthetic `cartridge_boot_rom`.",
        boot_marker()
    );
    println!("The boot ROM is generated in code; no commercial image is needed.");
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
    println!(
        "  chronofork megadrive cartridge --rom <FILE> [--sram-in <FILE>] [--sram-out <FILE>]"
    );
    println!("  chronofork megadrive boot --rom <FILE> [--frames <N>] [--tmss enabled|disabled]");
    println!("  chronofork trace capture [--rom <FILE>] [--steps <N>] ... --out <FILE>");
    println!("  chronofork trace info --file <FILE>");
    println!("  chronofork trace query --file <FILE> [--kind GROUP] [--limit <N>]");
    println!("  chronofork trace compare --left <FILE> --right <FILE>");
    println!("  chronofork trace export --file <FILE> [--format tsv|canonical] --out <FILE>");
    println!("  chronofork trace --help       Show trace commands, output blocks and limits");
    println!("  chronofork script run --file <FILE> [--rom <FILE>] [--sandbox <DIR>]");
    println!("                        [limits flags]...");
    println!(
        "  chronofork script --help      Show script commands, defaults, limits and exit codes"
    );
    println!("  chronofork debug run --rom <FILE> --steps <N> [debugger flags]...");
    println!("  chronofork debug inspect --rom <FILE> [--steps <N>]");
    println!("  chronofork debug memory --rom <FILE> [--steps <N>] --address <ADDR> --length <N>");
    println!(
        "  chronofork debug disasm --rom <FILE> [--steps <N>] [--address <ADDR>] [--count <N>]"
    );
    println!(
        "  chronofork debug vdp --rom <FILE> [--steps <N>] [--vram-address <ADDR> --vram-length <N>]"
    );
    println!("  chronofork debug interrupts --rom <FILE> [--steps <N>]");
    println!("  chronofork debug bus --rom <FILE>");
    println!("  chronofork backend describe   Show synthetic backend identity and capabilities");
    println!("  chronofork serve --token-file <FILE> [--bind <ADDR>] [--store-root <DIR>]");
    println!("                   [--artifacts <DIR>] [--rom <FILE>] [--workers <N>]");
    println!("                   [--max-requests <N>]");
    println!("  chronofork api health|tools|call --url <URL> --token-file <FILE>");
    println!("                   [--principal <NAME>] [--op <NAME>]");
    println!("                   [--input <JSON> | --input-file <FILE|->]");
    println!("  chronofork mcp tools|call --url <URL> --token-file <FILE> [--principal <NAME>]");
    println!("                   [--tool <NAME>] [--arguments <JSON>]");
    println!("  chronofork experiment run --steps <N> [--buttons <U32>]");
    println!("                             [--checkpoints <a,b,c>] [--out <FILE>]");
    println!("  chronofork gui run [--remote <URL> --token-file <FILE>] [--rom <FILE>]");
    println!("                     [--width <N>] [--height <N>] [--ansi] [--html <OUT>]");
    println!("                     [--movie <FILE>] [--fps <N>]");
    println!("  chronofork session save --root <DIR> [--session <KEY>] [--name <NAME>]");
    println!("                            [--steps <N>] [--input <U32>] [--json]");
    println!("  chronofork session load --root <DIR> --session <KEY> [--snapshot <KEY>]");
    println!("                            [--steps <N>] [--input <U32>] [--json]");
    println!("  chronofork session list --root <DIR> [--session <KEY>] [--json]");
    println!("  chronofork session delete --root <DIR> --session <KEY> [--snapshot <KEY>]");
    println!("                            [--json]");
    println!("  chronofork tas record --movie <FILE> [--steps <N>] [--input <U32>]");
    println!("                        [--ports <P1,P2,P3>] [--axes <A0,A1,A2,A3>]");
    println!("                        [--keys <U64>] [--anchor-interval <N>]");
    println!("                        [--movie-id <U64>] [--name <NAME>]");
    println!("                        [--root <DIR> [--session <KEY> [--snapshot <KEY>]]]");
    println!("                        [--rom <FILE>]");
    println!("  chronofork tas play --movie <FILE> [--from <TICK>] [--until <TICK>]");
    println!("                      [--rom <FILE>]");
    println!("  chronofork tas verify --movie <FILE> [--from <TICK>] [--until <TICK>]");
    println!("                        [--rom <FILE>]");
    println!("  chronofork tas compare --movie <FILE> --against <FILE>");
    println!("  chronofork tas studio --movie <FILE> [--cache-interval <N>]");
    println!("                        [--cache-capacity <N>] [--cache-bytes <N>] [--rom <FILE>]");
    println!("  chronofork tas edit --movie <FILE> --out <FILE>");
    println!("                      --set-input <INDEX=MASK>...   or --truncate-after <INDEX>");
    println!("                      [--movie-id <U64>] [--name <NAME>] [--rom <FILE>]");
    println!("                      [--anchors <keep-prefix|drop|rebuild>]");
    println!("                      [--anchor-interval <N>]");
    println!("  chronofork --version          Show version");
    println!("  chronofork help               Show this help");
    println!();
    println!("`--root` is required for every session command. Only `session save` without");
    println!("`--session` initializes a storage root; `load`, `list` and `delete` never create");
    println!("one. Session commands print `key=value` record blocks to stdout and diagnostics");
    println!("to stderr; `--json` prints the exact canonical service envelope");
    println!("(`{{\"op\":...,\"result\":...}}`) plus one newline instead, the same bytes the");
    println!("HTTP surface returns for the matching operation. A failed session command prints");
    println!("one `chronofork: <message>` stderr line; storage-anomaly labels and a `storage");
    println!("listing truncated at <N> entries` notice are the only other stderr diagnostics and");
    println!("never change the stdout records. Typed error codes appear only in the canonical");
    println!("JSON envelopes that the `api` and `mcp` client surfaces print on a remote failure.");
    println!();
    println!("`chronofork serve` hosts the Stage 6 HTTP and MCP endpoints over one in-process");
    println!("service; `chronofork api` and `chronofork mcp` are typed clients of a running");
    println!("daemon, and `chronofork experiment run` prints the canonical experiment artifact.");
    println!("`serve --help`, `api --help`, `mcp --help` and `experiment --help` document their");
    println!("flags, output policy and exit codes. Daemons and clients only accept tokens from");
    println!("a `chronofork.tokens.v1` file, never from the command line or environment.");
    println!();
    println!("`chronofork gui run` is the Stage 7 terminal/HTML presentation surface: it reads");
    println!("line commands from stdin and prints blocks, frames or an HTML document to stdout.");
    println!("It reuses the same service and transport surfaces (remote mode talks to a");
    println!("`chronofork serve` daemon; local mode hosts an in-process service) and never owns");
    println!("canonical emulated state. `chronofork gui --help` documents its flags, grammar,");
    println!("block fields, limits and the unavailable rendering/audio paths.");
    println!();
    println!("TAS movies record one backend input sequence (the deterministic synthetic test");
    println!("backend by default, or the minimal Mega Drive slice with `--rom <FILE>`) and");
    println!("embed the canonical checkpoint; `tas play/verify` replay an exact tick window");
    println!("from that checkpoint or from the nearest embedded anchor by deterministic");
    println!("re-simulation, and `tas compare`");
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
    println!("For host structured tracing run:");
    println!("  chronofork trace --help");
    println!("For the sandboxed deterministic script runtime run:");
    println!("  chronofork script --help");
    println!("For the backend-specific instruction trace format run:");
    println!("  chronofork megadrive trace --help");
    println!("`chronofork debug` is the headless one-shot debugger over the minimal Mega Drive");
    println!(
        "slice; `chronofork debug --help` documents its subcommands, flags, record blocks and"
    );
    println!("limits. Every debug subcommand requires `--rom <FILE>` and prints only `key=value`");
    println!("record blocks to stdout.");
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
    let scheduler = SchedulerConfig::new(video.unwrap_or(VideoStandard::Ntsc));
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

pub(crate) fn parse_usize(value: &str, flag: &str) -> Result<usize, CliError> {
    value
        .parse()
        .map_err(|_| CliError(format!("`{flag}` expects a decimal integer, got `{value}`")))
}

pub(crate) fn parse_hex_u32(value: &str, flag: &str) -> Result<u32, CliError> {
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
    if executed > 0 {
        eprintln!(
            "note: interrupts use the master-cycle timeline: one scanline is {MASTER_CYCLES_PER_LINE} master cycles and each instruction advances canonical time by its 68000 cycle cost times seven. The model adds cost at the instruction boundary, not at each internal microcycle, and does not model VDP prefetch or interrupt latency exactly. Interrupt delivery is gated by the 68000 SR interrupt-priority mask: level 7 is always taken and any other level only when strictly above the mask, so a masked VDP request stays pending instead of running a handler."
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
    writeln!(writer, "master_cycles={}", machine.master_cycles())?;
    writeln!(writer, "cpu_cycles={}", cpu.cycles())?;
    writeln!(writer, "usp={:08x}", cpu.usp())?;
    writeln!(writer, "ssp={:08x}", cpu.ssp())?;
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
    println!("    tick=<decimal>                 (master-clock cycles)");
    println!("    master_cycles=<decimal>");
    println!("    cpu_cycles=<decimal>            (sum of 68000 instruction cycles)");
    println!("    usp=<8 lowercase hex digits>");
    println!("    ssp=<8 lowercase hex digits>");
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
    println!("  Interrupts advance on the master-cycle timeline: one scanline is");
    println!(
        "  {MASTER_CYCLES_PER_LINE} master cycles and each instruction adds its 68000 cycle cost"
    );
    println!("  times seven at the instruction boundary. Cost is not distributed across");
    println!("  internal microcycles, and VDP prefetch and interrupt latency are not");
    println!("  modelled, so this is not a full VDP-timing equivalence claim. Delivery is");
    println!("  gated by the 68000 SR interrupt-priority mask: level 7 is always taken,");
    println!("  and every other level only when strictly above the mask. A masked VDP");
    println!("  request stays pending — it is not acknowledged or dropped — until the");
    println!("  program lowers the mask (for example with MOVE.W #imm,SR); handlers return");
    println!("  with RTE, which restores the saved raw SR.");
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
    println!("capability.debug_events: {}", capabilities.debug_events);
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
    // both VDP levels masked: the first HBlank is retained, never delivered.
    for _ in 0..2_000_000 {
        if manager.inspect(source)?.machine.registers["irq_pending_level"] == 4_u64 {
            break;
        }
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

    // A ROM that masks every VDP level reaches vertical blanking with VBlank
    // (level 6) retained; the VDP status exposes it as bits 3 and 7.
    let mut vblank_manager =
        SessionManager::new(Arc::new(MegadriveFactory::new(interrupt_rom(7))?));
    let vblank = vblank_manager.create_session()?;
    for _ in 0..2_000_000 {
        if vblank_manager.inspect(vblank)?.machine.registers["irq_pending_level"] == 6_u64 {
            break;
        }
        vblank_manager.step(vblank, InputFrame::NEUTRAL)?;
    }
    let vblank_checkpoint = vblank_manager.snapshot(vblank)?;
    vblank_manager.step(vblank, InputFrame::NEUTRAL)?;
    vblank_manager.restore(vblank, vblank_checkpoint)?;
    let restored_vblank = vblank_manager.inspect(vblank)?;
    println!(
        "interrupt: masked VBlank pending level {} vector 0x{:02X} at tick {} (mask {}); VDP VBlank/VINT bits set and snapshot restored them",
        restored_vblank.machine.registers["irq_pending_level"],
        restored_vblank.machine.registers["irq_vector"],
        restored_vblank.machine.tick.raw(),
        restored_vblank.machine.registers["irq_mask"],
    );

    // The interrupt ROM delays past two scanlines with the mask still at 7, then
    // lowers it. The retained HBlank is delivered at the next boundary, its
    // handler runs and RTE restores the interrupted PC and saved raw SR.
    let mut gated_manager = SessionManager::new(Arc::new(MegadriveFactory::new(interrupt_rom(3))?));
    let gated = gated_manager.create_session()?;
    for _ in 0..2_000_000 {
        if gated_manager.inspect(gated)?.machine.registers["irq_pending_level"] == 4_u64 {
            break;
        }
        gated_manager.step(gated, InputFrame::NEUTRAL)?;
    }
    let masked_hblank = gated_manager.inspect(gated)?;
    println!(
        "interrupt: interrupt ROM retained HBlank at tick {} under mask {} (no handler)",
        masked_hblank.machine.tick.raw(),
        masked_hblank.machine.registers["irq_mask"],
    );

    for _ in 0..2_000_000 {
        if gated_manager.inspect(gated)?.machine.registers["pc"] == 0x210 {
            break;
        }
        gated_manager.step(gated, InputFrame::NEUTRAL)?;
    }
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

    // At vertical blanking the masked VBlank (level 6) is retained while the
    // pending HBlank (level 4) is delivered every active line. The snapshot
    // preserves both requests and the VINT status bit until RTE restores the
    // interrupted program.
    for _ in 0..2_000_000 {
        if gated_manager.inspect(gated)?.machine.registers["vdp_status"] & 0x0080_u64 == 0x0080_u64
        {
            break;
        }
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

/// A manager over the deterministic synthetic test backend.
pub(crate) fn synthetic_session_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

/// `--input` selection, defaulting to neutral input.
pub(crate) fn input_frame(buttons: Option<u32>) -> InputFrame {
    buttons.map_or(InputFrame::NEUTRAL, InputFrame::new)
}

/// Starts one output record, separating it from a previous block by one blank line.
pub(crate) fn begin_record(output: &mut String, record: &str) {
    if !output.is_empty() {
        output.push('\n');
    }
    let _ = writeln!(output, "record={record}");
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
