//! `chronofork debug` process boundary: host triggers and read-only views over
//! the minimal Mega Drive slice.
//!
//! This module is a thin, line-oriented adapter. Breakpoint, watchpoint, event
//! breakpoint and logpoint semantics live in [`chronofork_debug::Debugger`];
//! event observation and watchpoint matching live in `chronofork-megadrive`;
//! decoded CPU/VDP/interrupt/bus state and the bounded disassembler live in
//! `chronofork-megadrive::inspect` and `chronofork-megadrive::disasm`. The CLI
//! parses the documented ADR-0010 section 3.4 grammar, reads one ROM through the
//! existing bounded `tas` path, and prints deterministic `key=value` record
//! blocks to stdout with `chronofork:` diagnostics on stderr.
//!
//! Every command is one-shot and headless: a stop is an instruction boundary,
//! debug state is process-local and non-canonical, and no interactive REPL,
//! reverse execution, mid-instruction or cycle-exact stop is implemented.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use chronofork_backend_api::{
    Backend, DebugAddressRange, DebugEvent, DebugEventKind, MachineInspection, WatchpointAccess,
    WatchpointSpec,
};
use chronofork_core::{InputFrame, hash_bytes};
use chronofork_debug::{
    BreakpointSpec, Condition, ConditionOperand, DebugError, DebugRun, Debugger, EventFilter,
    InterruptPhase, Location, LogEntry, MAX_EVENT_CAPACITY, MAX_RUN_STEP_LIMIT, Relation,
    StopReason,
};
use chronofork_megadrive::{
    BusView, CpuView, InterruptView, MAX_DISASSEMBLY_COUNT, MegadriveBackend, MegadriveFactory,
    RAM_BASE, RAM_END, RomMemory, VRAM_SIZE, VdpView, VideoStandard, WindowMemory, disassemble,
};

use crate::{CliError, VERSION, begin_record, ensure, option_value, parse_u64, set_once};

/// Default `debug disasm --count` value.
const DEFAULT_DISASSEMBLY_COUNT: usize = 8;
/// Largest accepted `debug memory --length` value, in bytes.
const MAX_MEMORY_DUMP_BYTES: usize = 4_096;
/// Largest accepted `debug vdp --vram-length` value, in bytes.
const MAX_VRAM_WINDOW_BYTES: usize = 4_096;
/// Message used by a `--logpoint` that omits `:<MESSAGE>`.
const DEFAULT_LOGPOINT_MESSAGE: &str = "hit";
/// Largest accepted `--show-events` value.
const MAX_SHOW_EVENTS: usize = MAX_EVENT_CAPACITY;
/// Largest accepted `--steps` value; the debugger run-loop limit.
const MAX_DEBUG_STEPS: u64 = MAX_RUN_STEP_LIMIT;
/// Largest 68000 interrupt level a filter may name.
const MAX_INTERRUPT_LEVEL: u8 = 7;

/// Runs one `chronofork debug` subcommand.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("run") => debug_run(&args[1..]),
        Some("inspect") => debug_inspect(&args[1..]),
        Some("memory") => debug_memory(&args[1..]),
        Some("disasm") => debug_disasm(&args[1..]),
        Some("vdp") => debug_vdp(&args[1..]),
        Some("interrupts") => debug_interrupts(&args[1..]),
        Some("bus") => debug_bus(&args[1..]),
        Some("--help" | "-h") => {
            ensure(args.len() == 1, "`debug --help` takes no other arguments")?;
            print_help();
            Ok(())
        }
        None => Err(CliError(
            "`debug` requires a subcommand: run, inspect, memory, disasm, vdp, interrupts or bus"
                .to_owned(),
        )
        .into()),
        Some(other) => Err(CliError(format!(
            "unknown `debug` subcommand `{other}`; expected run, inspect, memory, disasm, vdp, \
             interrupts or bus"
        ))
        .into()),
    }
}

/// Flags shared by every debug subcommand: `--rom` and `--steps`.
#[derive(Debug, Default)]
struct CommonOptions {
    rom: Option<PathBuf>,
    steps: Option<u64>,
}

/// Parses one of the shared `--rom`/`--steps` flags.
///
/// Returns `Ok(false)` when `flag` is not shared, so the caller can report an
/// unknown option with its own list of implemented flags.
fn parse_common(
    args: &[String],
    index: &mut usize,
    flag: &str,
    common: &mut CommonOptions,
) -> Result<bool, CliError> {
    match flag {
        "--rom" => {
            let value = option_value(args, index, flag)?;
            if value.is_empty() {
                return Err(CliError("`--rom` requires a non-empty value".to_owned()));
            }
            set_once(&mut common.rom, PathBuf::from(value), flag)?;
        }
        "--steps" => set_once(
            &mut common.steps,
            parse_u64(option_value(args, index, flag)?, flag)?,
            flag,
        )?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Returns the required `--rom` path for one subcommand.
fn require_rom(subcommand: &str, common: &CommonOptions) -> Result<PathBuf, CliError> {
    common
        .rom
        .clone()
        .ok_or_else(|| CliError(format!("`debug {subcommand}` requires `--rom <FILE>`")))
}

/// Validates `--steps` for one subcommand.
///
/// `debug run` requires an explicit limit in `1..=MAX_DEBUG_STEPS`; every other
/// subcommand defaults to zero steps and accepts `0..=MAX_DEBUG_STEPS`.
fn steps_for(common: &CommonOptions, subcommand: &str, required: bool) -> Result<u64, CliError> {
    let steps = match common.steps {
        Some(steps) => steps,
        None if required => {
            return Err(CliError(format!(
                "`debug {subcommand}` requires `--steps <N>`"
            )));
        }
        None => 0,
    };
    if steps > MAX_DEBUG_STEPS {
        return Err(CliError(format!(
            "`--steps` must be at most {MAX_DEBUG_STEPS}, got {steps}"
        )));
    }
    if required && steps == 0 {
        return Err(CliError(
            "`--steps` must be at least 1 for `debug run`".to_owned(),
        ));
    }
    Ok(steps)
}

/// Parses a decimal or `0x`/`0X` hexadecimal unsigned number.
fn parse_number(value: &str, flag: &str) -> Result<u64, CliError> {
    let parsed = if let Some(digits) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        if digits.is_empty() {
            None
        } else {
            u64::from_str_radix(digits, 16).ok()
        }
    } else {
        value.parse::<u64>().ok()
    };
    parsed.ok_or_else(|| {
        CliError(format!(
            "`{flag}` expects a decimal or `0x` hexadecimal number in the 64-bit range, got \
             `{value}`"
        ))
    })
}

/// Parses a 32-bit address in decimal or `0x`/`0X` hexadecimal.
fn parse_address(value: &str, flag: &str) -> Result<u32, CliError> {
    let number = parse_number(value, flag)?;
    u32::try_from(number).map_err(|_| {
        CliError(format!(
            "`{flag}` address `{value}` exceeds the 32-bit address space"
        ))
    })
}

/// Parses a count in `min..=max` from decimal or `0x`/`0X` hexadecimal.
fn parse_count(value: &str, flag: &str, min: usize, max: usize) -> Result<usize, CliError> {
    let number = parse_number(value, flag)?;
    let count = usize::try_from(number).map_err(|_| {
        CliError(format!(
            "`{flag}` count `{value}` is too large for this platform"
        ))
    })?;
    if count < min || count > max {
        return Err(CliError(format!(
            "`{flag}` must be between {min} and {max}, got {count}"
        )));
    }
    Ok(count)
}

/// Parses an inclusive `START-END` or single-`ADDR` range.
fn parse_inclusive_range(value: &str, flag: &str) -> Result<DebugAddressRange, CliError> {
    let (start, end) = match value.split_once('-') {
        Some((start, end)) => (parse_address(start, flag)?, parse_address(end, flag)?),
        None => {
            let address = parse_address(value, flag)?;
            (address, address)
        }
    };
    DebugAddressRange::new(start, end)
        .map_err(|error| CliError(format!("`{flag}` range `{value}` was rejected: {error}")))
}

/// Parses a breakpoint/logpoint location: `ADDR` or `START-END` inclusive.
fn parse_location(value: &str, flag: &str) -> Result<Location, CliError> {
    if value.contains('-') {
        let range = parse_inclusive_range(value, flag)?;
        Ok(Location::Range {
            start: range.start(),
            end: range.end(),
        })
    } else {
        Ok(Location::Address(parse_address(value, flag)?))
    }
}

/// Splits `<operand><op><value>` at the first documented comparison operator.
fn split_relation(text: &str) -> Option<(&str, Relation, &str)> {
    const OPERATORS: [(&str, Relation); 6] = [
        ("==", Relation::Equal),
        ("!=", Relation::NotEqual),
        ("<=", Relation::LessOrEqual),
        (">=", Relation::GreaterOrEqual),
        ("<", Relation::Less),
        (">", Relation::Greater),
    ];
    for (token, relation) in OPERATORS {
        if let Some((left, right)) = text.split_once(token) {
            return Some((left, relation, right));
        }
    }
    None
}

/// Parses `reg:<name><op><value>` or `mem:<address>:<length><op><value>`.
fn parse_condition(text: &str, flag: &str) -> Result<Condition, CliError> {
    if let Some(operand) = text.strip_prefix("reg:") {
        let (name, relation, value) = split_relation(operand).ok_or_else(|| {
            CliError(format!(
                "`{flag}` register condition `{operand}` must be `<name><op><value>` with op one \
                 of ==, !=, <, <=, >, >="
            ))
        })?;
        if name.is_empty() {
            return Err(CliError(format!(
                "`{flag}` register condition is missing a register name"
            )));
        }
        Ok(Condition {
            operand: ConditionOperand::Register(name.to_owned()),
            relation,
            value: parse_number(value, flag)?,
        })
    } else if let Some(rest) = text.strip_prefix("mem:") {
        let (address, comparison) = rest.split_once(':').ok_or_else(|| {
            CliError(format!(
                "`{flag}` memory condition `{text}` must be \
                 `mem:<address>:<length><op><value>`"
            ))
        })?;
        let (length, relation, value) = split_relation(comparison).ok_or_else(|| {
            CliError(format!(
                "`{flag}` memory condition `{text}` must be \
                 `mem:<address>:<length><op><value>` with op one of ==, !=, <, <=, >, >="
            ))
        })?;
        let length = length.parse::<u8>().map_err(|_| {
            CliError(format!(
                "`{flag}` memory condition length `{length}` must be a decimal byte count in 1..=8"
            ))
        })?;
        Ok(Condition {
            operand: ConditionOperand::Memory {
                address: parse_address(address, flag)?,
                length,
            },
            relation,
            value: parse_number(value, flag)?,
        })
    } else {
        Err(CliError(format!(
            "`{flag}` condition `{text}` must start with `reg:` or `mem:`"
        )))
    }
}

/// Parses `LOCATION[:CONDITION]` into a host breakpoint specification.
fn parse_breakpoint(value: &str, flag: &str) -> Result<BreakpointSpec, CliError> {
    let (location, condition) = match value.split_once(':') {
        Some((location, condition)) => (location, Some(condition)),
        None => (value, None),
    };
    Ok(BreakpointSpec {
        location: parse_location(location, flag)?,
        condition: condition
            .map(|text| parse_condition(text, flag))
            .transpose()?,
    })
}

/// Parses one `--watchpoint` value and appends the backend specs it installs.
///
/// `rw` installs one read and one write backend watchpoint, in that order; the
/// host debugger assigns its own trigger id to each installed spec.
fn parse_watchpoint(
    value: &str,
    next_id: &mut u32,
    specs: &mut Vec<WatchpointSpec>,
) -> Result<(), CliError> {
    let (range_text, access_text) = value.rsplit_once(':').ok_or_else(|| {
        CliError(format!(
            "`--watchpoint` expects `<ADDR|START-END>:<r|w|rw>`, got `{value}`"
        ))
    })?;
    let range = parse_inclusive_range(range_text, "--watchpoint")?;
    let (first, second) = match access_text {
        "r" => (WatchpointAccess::Read, None),
        "w" => (WatchpointAccess::Write, None),
        "rw" => (WatchpointAccess::Read, Some(WatchpointAccess::Write)),
        other => {
            return Err(CliError(format!(
                "`--watchpoint` access `{other}` must be `r`, `w` or `rw`"
            )));
        }
    };
    for access in std::iter::once(first).chain(second) {
        let id = *next_id;
        *next_id += 1;
        specs.push(
            WatchpointSpec::new(id, range, access)
                .map_err(|error| CliError(format!("`--watchpoint` was rejected: {error}")))?,
        );
    }
    Ok(())
}

/// Parses one `--eventpoint` filter.
fn parse_event_point(value: &str) -> Result<EventFilter, CliError> {
    if let Some(rest) = value.strip_prefix("interrupt:") {
        let mut parts = rest.split(':');
        let phase = match parts.next() {
            Some("requested") => InterruptPhase::Requested,
            Some("delivered") => InterruptPhase::Delivered,
            Some("acknowledged") => InterruptPhase::Acknowledged,
            _ => {
                return Err(CliError(format!(
                    "`--eventpoint` interrupt phase in `{value}` must be `requested`, `delivered` \
                     or `acknowledged`"
                )));
            }
        };
        let level = match parts.next() {
            Some(text) => Some(parse_interrupt_level(text)?),
            None => None,
        };
        if parts.next().is_some() {
            return Err(CliError(format!(
                "`--eventpoint` filter `{value}` has too many `:` fields"
            )));
        }
        Ok(EventFilter::Interrupt { level, phase })
    } else if let Some(rest) = value.strip_prefix("vdp:") {
        let mut parts = rest.split(':');
        let write = match parts.next() {
            Some("read") => Some(false),
            Some("write") => Some(true),
            _ => {
                return Err(CliError(format!(
                    "`--eventpoint` VDP direction in `{value}` must be `read` or `write`"
                )));
            }
        };
        let offset = match parts.next() {
            Some(text) => Some(parse_vdp_offset(text)?),
            None => None,
        };
        if parts.next().is_some() {
            return Err(CliError(format!(
                "`--eventpoint` filter `{value}` has too many `:` fields"
            )));
        }
        Ok(EventFilter::Vdp { write, offset })
    } else if let Some(rest) = value.strip_prefix("controller:") {
        let port = match rest {
            "any" => None,
            text => Some(parse_controller_port(text)?),
        };
        Ok(EventFilter::ControllerRead { port })
    } else if let Some(rest) = value.strip_prefix("bus-write") {
        let range = match rest {
            "" => None,
            rest => {
                let range_text = rest.strip_prefix(':').ok_or_else(|| {
                    CliError(format!(
                        "`--eventpoint` bus-write filter `{value}` must be `bus-write` or \
                         `bus-write:<START-END>`"
                    ))
                })?;
                Some(parse_inclusive_range(range_text, "--eventpoint")?)
            }
        };
        Ok(EventFilter::BusWrite { range })
    } else {
        Err(CliError(format!(
            "`--eventpoint` filter `{value}` must be `interrupt:<phase>[:<level>]`, \
             `vdp:<read|write>[:<offset>]`, `controller:<1|2|any>` or \
             `bus-write[:<START-END>]`"
        )))
    }
}

/// Parses a 68000 interrupt level in `1..=7`.
fn parse_interrupt_level(value: &str) -> Result<u8, CliError> {
    let level = parse_number(value, "--eventpoint")?;
    let level = u8::try_from(level).map_err(|_| {
        CliError(format!(
            "`--eventpoint` interrupt level `{value}` must be between 1 and {MAX_INTERRUPT_LEVEL}"
        ))
    })?;
    if !(1..=MAX_INTERRUPT_LEVEL).contains(&level) {
        return Err(CliError(format!(
            "`--eventpoint` interrupt level `{value}` must be between 1 and {MAX_INTERRUPT_LEVEL}"
        )));
    }
    Ok(level)
}

/// Parses a VDP port offset as one unsigned byte.
fn parse_vdp_offset(value: &str) -> Result<u8, CliError> {
    let offset = parse_number(value, "--eventpoint")?;
    u8::try_from(offset).map_err(|_| {
        CliError(format!(
            "`--eventpoint` VDP offset `{value}` must be an unsigned byte in decimal or `0x` \
             hexadecimal"
        ))
    })
}

/// Parses a Mega Drive controller port number.
///
/// The minimal slice exposes the hardware data registers for ports 1 and 2 only.
fn parse_controller_port(value: &str) -> Result<u8, CliError> {
    let port = parse_number(value, "--eventpoint")?;
    match u8::try_from(port) {
        Ok(1) => Ok(1),
        Ok(2) => Ok(2),
        _ => Err(CliError(format!(
            "`--eventpoint` controller port `{value}` must be `1`, `2` or `any`"
        ))),
    }
}

/// Parses `LOCATION[:MESSAGE]` into a host logpoint.
fn parse_logpoint(value: &str) -> Result<(Location, String), CliError> {
    if value.is_empty() {
        return Err(CliError("`--logpoint` requires a location".to_owned()));
    }
    match value.split_once(':') {
        Some((location, message)) => {
            Ok((parse_location(location, "--logpoint")?, message.to_owned()))
        }
        None => Ok((
            parse_location(value, "--logpoint")?,
            DEFAULT_LOGPOINT_MESSAGE.to_owned(),
        )),
    }
}

/// Builds the unknown-option diagnostic for one subcommand.
fn unknown_option(subcommand: &str, flag: &str, expected: &str) -> CliError {
    CliError(format!(
        "unknown `debug {subcommand}` option `{flag}`; expected {expected}"
    ))
}

/// Reads one ROM through the bounded `tas` path and creates a default-scheduler
/// machine.
fn open_machine(rom: &Path) -> Result<MegadriveBackend, Box<dyn Error>> {
    let bytes = crate::tas::read_rom_file(rom)?;
    let factory = MegadriveFactory::new(bytes)?;
    Ok(factory.create_machine()?)
}

/// Executes `steps` neutral-input instructions.
fn run_instructions(
    machine: &mut MegadriveBackend,
    steps: u64,
    subcommand: &str,
) -> Result<(), Box<dyn Error>> {
    let mut executed = 0_u64;
    while executed < steps {
        machine.step(InputFrame::NEUTRAL).map_err(|error| {
            CliError(format!(
                "`debug {subcommand}` execution failed after {executed} instruction(s): {error}"
            ))
        })?;
        executed += 1;
    }
    Ok(())
}

/// Formats a 32-bit address as `0x` plus eight uppercase hexadecimal digits.
fn hex32(value: u32) -> String {
    format!("0x{value:08X}")
}

/// Formats a program counter as `0x` plus at least eight uppercase digits.
fn hex_pc(value: u64) -> String {
    format!("0x{value:08X}")
}

/// Formats a 16-bit value as `0x` plus four uppercase hexadecimal digits.
fn hex16(value: u16) -> String {
    format!("0x{value:04X}")
}

/// Formats a byte as `0x` plus two uppercase hexadecimal digits.
fn hex8(value: u8) -> String {
    format!("0x{value:02X}")
}

/// Formats an optional address field, using `-` when the event has none.
fn optional_address(value: Option<u32>) -> String {
    value.map_or_else(|| "-".to_owned(), hex32)
}

/// Formats an optional port-offset field, using `-` when the event has none.
fn optional_offset(value: Option<u8>) -> String {
    value.map_or_else(|| "-".to_owned(), hex8)
}

/// Formats an optional event value as one 32-bit hexadecimal field.
fn optional_value(value: Option<u32>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| format!("0x{value:08X}"))
}

/// Formats an optional width field, using `-` when the event has none.
fn optional_width(value: Option<u8>) -> String {
    value.map_or_else(|| "-".to_owned(), |width| width.to_string())
}

/// Formats an optional access or phase name, using `-` when the event has none.
fn optional_name(value: Option<&str>) -> String {
    value.map_or_else(|| "-".to_owned(), str::to_owned)
}

/// Formats an optional interrupt level, using `-` when the event has none.
fn optional_level(value: Option<u8>) -> String {
    value.map_or_else(|| "-".to_owned(), |level| level.to_string())
}

/// Formats an optional controller port, using `-` when the event has none.
fn optional_port(value: Option<u8>) -> String {
    value.map_or_else(|| "-".to_owned(), |port| port.to_string())
}

/// Formats an optional watchpoint id, using `-` when the event has none.
fn optional_id(value: Option<u32>) -> String {
    value.map_or_else(|| "-".to_owned(), |id| id.to_string())
}

/// Returns the stable name of a video standard.
fn video_standard_name(standard: VideoStandard) -> &'static str {
    match standard {
        VideoStandard::Ntsc => "ntsc",
        VideoStandard::Pal => "pal",
    }
}

/// Options for `debug run`.
#[derive(Debug, Default)]
struct RunOptions {
    machine: CommonOptions,
    breakpoints: Vec<BreakpointSpec>,
    watchpoints: Vec<WatchpointSpec>,
    event_points: Vec<EventFilter>,
    logpoints: Vec<(Location, String)>,
    max_events: Option<usize>,
    show_events: Option<usize>,
}

/// Parses every `debug run` flag.
fn parse_run_options(args: &[String]) -> Result<RunOptions, CliError> {
    let mut options = RunOptions::default();
    let mut next_watchpoint_id = 1_u32;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        if !parse_common(args, &mut index, &flag, &mut options.machine)? {
            match flag.as_str() {
                "--breakpoint" => {
                    let value = option_value(args, &mut index, &flag)?;
                    options.breakpoints.push(parse_breakpoint(value, &flag)?);
                }
                "--watchpoint" => {
                    let value = option_value(args, &mut index, &flag)?;
                    parse_watchpoint(value, &mut next_watchpoint_id, &mut options.watchpoints)?;
                }
                "--eventpoint" => {
                    let value = option_value(args, &mut index, &flag)?;
                    options.event_points.push(parse_event_point(value)?);
                }
                "--logpoint" => {
                    let value = option_value(args, &mut index, &flag)?;
                    options.logpoints.push(parse_logpoint(value)?);
                }
                "--max-events" => {
                    let value = option_value(args, &mut index, &flag)?;
                    set_once(
                        &mut options.max_events,
                        parse_count(value, &flag, 1, MAX_EVENT_CAPACITY)?,
                        &flag,
                    )?;
                }
                "--show-events" => {
                    let value = option_value(args, &mut index, &flag)?;
                    set_once(
                        &mut options.show_events,
                        parse_count(value, &flag, 0, MAX_SHOW_EVENTS)?,
                        &flag,
                    )?;
                }
                other => {
                    return Err(unknown_option(
                        "run",
                        other,
                        "--rom <FILE> --steps <N> [--breakpoint <LOCATION>[:<CONDITION>]]... \
                         [--watchpoint <ADDR|START-END>:<r|w|rw>]... [--eventpoint <FILTER>]... \
                         [--logpoint <LOCATION>[:<MESSAGE>]]... [--max-events <N>] \
                         [--show-events <N>]",
                    ));
                }
            }
        }
        index += 1;
    }
    Ok(options)
}

/// Maps a typed debugger error to the CLI diagnostic of the operation.
fn debugger_error(operation: &str, error: DebugError) -> CliError {
    CliError(format!("`debug {operation}`: {error}"))
}

fn debug_run(args: &[String]) -> Result<(), Box<dyn Error>> {
    if crate::tas::help_requested(args, "debug run")? {
        print_help();
        return Ok(());
    }
    let options = parse_run_options(args)?;
    let rom = require_rom("run", &options.machine)?;
    let steps = steps_for(&options.machine, "run", true)?;
    let mut machine = open_machine(&rom)?;

    let mut debugger = match options.max_events {
        Some(capacity) => Debugger::with_event_capacity(&mut machine, capacity),
        None => Debugger::new(&mut machine),
    }
    .map_err(|error| debugger_error("run", error))?;
    for spec in options.breakpoints {
        debugger
            .add_breakpoint(spec)
            .map_err(|error| debugger_error("run", error))?;
    }
    for spec in options.watchpoints {
        debugger
            .add_watchpoint(spec)
            .map_err(|error| debugger_error("run", error))?;
    }
    for filter in options.event_points {
        debugger
            .add_event_breakpoint(filter)
            .map_err(|error| debugger_error("run", error))?;
    }
    for (location, message) in &options.logpoints {
        debugger
            .add_logpoint(*location, message)
            .map_err(|error| debugger_error("run", error))?;
    }

    let run = debugger
        .run(steps)
        .map_err(|error| debugger_error("run", error))?;
    let inspection = debugger
        .inspect()
        .map_err(|error| debugger_error("run", error))?;
    let retained = debugger
        .retained_events()
        .map_err(|error| debugger_error("run", error))?;
    let log_entries = debugger.log_entries().to_vec();
    drop(debugger);

    let mut output = String::new();
    write_run_block(&mut output, &run, &inspection, retained.len())?;
    let show_events = options.show_events.unwrap_or(0);
    write_event_blocks(&mut output, &run, &retained, show_events);
    write_log_blocks(&mut output, &log_entries);
    print!("{output}");
    Ok(())
}

/// Writes the `record=debug-run` stop report.
fn write_run_block(
    output: &mut String,
    run: &DebugRun,
    inspection: &MachineInspection,
    retained: usize,
) -> Result<(), CliError> {
    let pc = inspection.registers.get("pc").copied().ok_or_else(|| {
        CliError("`debug run`: the inspection does not expose the required pc register".to_owned())
    })?;
    let breakpoint_id = match &run.reason {
        StopReason::Breakpoint { id } => id.value().to_string(),
        _ => "-".to_owned(),
    };
    let watchpoint_ids = match &run.reason {
        StopReason::Watchpoint { ids } if !ids.is_empty() => ids
            .iter()
            .map(|id| id.value().to_string())
            .collect::<Vec<_>>()
            .join(","),
        _ => "-".to_owned(),
    };
    let eventpoint_id = match &run.reason {
        StopReason::Event { id } => id.value().to_string(),
        _ => "-".to_owned(),
    };
    begin_record(output, "debug-run");
    let _ = writeln!(output, "status=stopped");
    let _ = writeln!(output, "reason={}", stop_reason_name(&run.reason));
    let _ = writeln!(output, "tick={}", run.tick.raw());
    let _ = writeln!(output, "state_hash={}", run.state_hash);
    let _ = writeln!(output, "pc={}", hex_pc(pc));
    let _ = writeln!(output, "steps={}", run.steps);
    let _ = writeln!(output, "halted={}", u8::from(run.halted));
    let _ = writeln!(output, "breakpoint_id={breakpoint_id}");
    let _ = writeln!(output, "watchpoint_ids={watchpoint_ids}");
    let _ = writeln!(output, "eventpoint_id={eventpoint_id}");
    let _ = writeln!(output, "events_matched={}", run.matched_events.len());
    let _ = writeln!(output, "events_retained={retained}");
    let _ = writeln!(output, "events_dropped={}", run.dropped_events);
    let _ = writeln!(output, "log_entries={}", run.log_entries.len());
    let _ = writeln!(output, "log_dropped={}", run.dropped_log_entries);
    Ok(())
}

/// Returns the stable `reason` name of one stop.
fn stop_reason_name(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::Breakpoint { .. } => "breakpoint",
        StopReason::Watchpoint { .. } => "watchpoint",
        StopReason::Event { .. } => "event",
        StopReason::Halted => "halted",
        StopReason::StepLimit => "step-limit",
    }
}

/// Writes the `record=debug-event` blocks of one run.
///
/// Every event that matched the stopping event breakpoint is printed exactly
/// once, with `matched=1`; `--show-events N` additionally prints the last N
/// retained events, oldest first, with `matched=0` unless they are also a match.
/// Matched events are located inside the retained ring by walking the events of
/// the stopping instruction, whose pre-step tick is `run.tick - 1` for a
/// post-instruction stop. An event evicted from a too-small ring is still
/// reported once, after the retained blocks.
fn write_event_blocks(
    output: &mut String,
    run: &DebugRun,
    retained: &[DebugEvent],
    show_events: usize,
) {
    let matched = &run.matched_events;
    let step_start = match &run.reason {
        StopReason::Watchpoint { .. } | StopReason::Event { .. } => {
            Some(run.tick.raw().saturating_sub(1))
        }
        StopReason::Breakpoint { .. } | StopReason::Halted | StopReason::StepLimit => None,
    };
    let step_start = step_start.map_or(retained.len(), |tick| {
        retained.partition_point(|event| event.tick.raw() < tick)
    });
    let mut marks = vec![false; retained.len()];
    let mut matched_seen = 0;
    for (index, event) in retained.iter().enumerate().skip(step_start) {
        if matched_seen < matched.len() && *event == matched[matched_seen] {
            marks[index] = true;
            matched_seen += 1;
        }
    }
    let tail_start = retained.len().saturating_sub(show_events);
    for (index, event) in retained.iter().enumerate() {
        if marks[index] || index >= tail_start {
            write_event_block(output, event, marks[index]);
        }
    }
    // A very small event ring can evict some of the stopping step's events; the
    // matched ones remain part of the stop report.
    for event in &matched[matched_seen..] {
        write_event_block(output, event, true);
    }
}

/// Writes one uniform `record=debug-event` block.
fn write_event_block(output: &mut String, event: &DebugEvent, matched: bool) {
    let fields = event_fields(event.kind);
    begin_record(output, "debug-event");
    let _ = writeln!(output, "tick={}", event.tick.raw());
    let _ = writeln!(output, "kind={}", fields.kind);
    let _ = writeln!(output, "matched={}", u8::from(matched));
    let _ = writeln!(output, "address={}", optional_address(fields.address));
    let _ = writeln!(output, "offset={}", optional_offset(fields.offset));
    let _ = writeln!(output, "port={}", optional_port(fields.port));
    let _ = writeln!(output, "value={}", optional_value(fields.value));
    let _ = writeln!(output, "width={}", optional_width(fields.width));
    let _ = writeln!(output, "access={}", optional_name(fields.access));
    let _ = writeln!(output, "level={}", optional_level(fields.level));
    let _ = writeln!(output, "phase={}", optional_name(fields.phase));
    let _ = writeln!(
        output,
        "watchpoint_id={}",
        optional_id(fields.watchpoint_id)
    );
}

/// The printable fields of one event class.
struct EventFields {
    kind: &'static str,
    address: Option<u32>,
    offset: Option<u8>,
    port: Option<u8>,
    value: Option<u32>,
    width: Option<u8>,
    access: Option<&'static str>,
    level: Option<u8>,
    phase: Option<&'static str>,
    watchpoint_id: Option<u32>,
}

/// Decodes one event kind into its printable fields.
fn event_fields(kind: DebugEventKind) -> EventFields {
    let base = EventFields {
        kind: "",
        address: None,
        offset: None,
        port: None,
        value: None,
        width: None,
        access: None,
        level: None,
        phase: None,
        watchpoint_id: None,
    };
    match kind {
        DebugEventKind::InterruptRequested { level } => EventFields {
            kind: "interrupt-requested",
            level: Some(level),
            phase: Some("requested"),
            ..base
        },
        DebugEventKind::InterruptDelivered { level } => EventFields {
            kind: "interrupt-delivered",
            level: Some(level),
            phase: Some("delivered"),
            ..base
        },
        DebugEventKind::InterruptAcknowledged { level } => EventFields {
            kind: "interrupt-acknowledged",
            level: Some(level),
            phase: Some("acknowledged"),
            ..base
        },
        DebugEventKind::BusWrite {
            address,
            value,
            width,
        } => EventFields {
            kind: "bus-write",
            address: Some(address),
            value: Some(value),
            width: Some(width),
            access: Some("write"),
            ..base
        },
        DebugEventKind::VdpPort {
            offset,
            value,
            width,
            write,
        } => EventFields {
            kind: "vdp-port",
            offset: Some(offset),
            value: Some(value),
            width: Some(width),
            access: Some(if write { "write" } else { "read" }),
            ..base
        },
        DebugEventKind::ControllerRead { port, value } => EventFields {
            kind: "controller-read",
            port: Some(port),
            value: Some(u32::from(value)),
            width: Some(1),
            access: Some("read"),
            ..base
        },
        DebugEventKind::Watchpoint {
            watchpoint_id,
            access,
            address,
            value,
            width,
        } => EventFields {
            kind: "watchpoint",
            watchpoint_id: Some(watchpoint_id),
            address: Some(address),
            value: Some(value),
            width: Some(width),
            access: Some(match access {
                WatchpointAccess::Read => "read",
                WatchpointAccess::Write => "write",
            }),
            ..base
        },
    }
}

/// Writes the retained `record=debug-log` blocks, oldest first.
fn write_log_blocks(output: &mut String, entries: &[LogEntry]) {
    for entry in entries {
        begin_record(output, "debug-log");
        let _ = writeln!(output, "logpoint_id={}", entry.logpoint_id.value());
        let _ = writeln!(output, "tick={}", entry.tick.raw());
        let _ = writeln!(output, "pc={}", hex_pc(entry.pc));
        let _ = writeln!(output, "hits={}", entry.hits);
        let _ = writeln!(output, "message={}", entry.message);
    }
}

fn debug_inspect(args: &[String]) -> Result<(), Box<dyn Error>> {
    if crate::tas::help_requested(args, "debug inspect")? {
        print_help();
        return Ok(());
    }
    let mut common = CommonOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        if !parse_common(args, &mut index, &flag, &mut common)? {
            return Err(unknown_option("inspect", &flag, "--rom <FILE> [--steps <N>]").into());
        }
        index += 1;
    }
    let rom = require_rom("inspect", &common)?;
    let steps = steps_for(&common, "inspect", false)?;
    let mut machine = open_machine(&rom)?;
    run_instructions(&mut machine, steps, "inspect")?;

    let inspection = machine.inspect()?;
    let cpu = CpuView::new(machine.cpu());
    let vdp = VdpView::new(machine.vdp());
    let interrupts = InterruptView::new(machine.interrupts(), machine.cpu().sr());
    let mut output = String::new();
    write_inspect_block(&mut output, &inspection, &cpu, &vdp, &interrupts, steps);
    print!("{output}");
    Ok(())
}

/// Writes the `record=debug-inspect` block.
fn write_inspect_block(
    output: &mut String,
    inspection: &MachineInspection,
    cpu: &CpuView,
    vdp: &VdpView,
    interrupts: &InterruptView,
    steps: u64,
) {
    begin_record(output, "debug-inspect");
    let _ = writeln!(output, "tick={}", inspection.tick.raw());
    let _ = writeln!(output, "state_hash={}", inspection.state_hash);
    let _ = writeln!(output, "halted={}", u8::from(cpu.halted()));
    let _ = writeln!(output, "steps={steps}");
    let _ = writeln!(output, "pc={}", hex32(cpu.pc()));
    let _ = writeln!(output, "sr={}", hex16(cpu.sr()));
    let _ = writeln!(output, "supervisor={}", u8::from(cpu.supervisor()));
    let _ = writeln!(output, "trace={}", u8::from(cpu.trace()));
    let _ = writeln!(output, "interrupt_mask={}", cpu.interrupt_mask());
    let _ = writeln!(output, "x={}", u8::from(cpu.x()));
    let _ = writeln!(output, "n={}", u8::from(cpu.n()));
    let _ = writeln!(output, "z={}", u8::from(cpu.z()));
    let _ = writeln!(output, "v={}", u8::from(cpu.v()));
    let _ = writeln!(output, "c={}", u8::from(cpu.c()));
    let _ = writeln!(output, "vdp_status={}", hex16(vdp.status()));
    let _ = writeln!(output, "vdp_vblank={}", u8::from(vdp.vblank()));
    let _ = writeln!(output, "vdp_vint_pending={}", u8::from(vdp.vint_pending()));
    let _ = writeln!(output, "vdp_address={}", hex16(vdp.address()));
    let _ = writeln!(output, "vdp_auto_increment={}", vdp.auto_increment());
    let _ = writeln!(
        output,
        "scheduler_standard={}",
        video_standard_name(interrupts.standard())
    );
    let _ = writeln!(
        output,
        "scheduler_instructions_per_line={}",
        interrupts.instructions_per_line()
    );
    let _ = writeln!(
        output,
        "scheduler_lines_per_frame={}",
        interrupts.lines_per_frame()
    );
    let _ = writeln!(
        output,
        "scheduler_active_lines={}",
        interrupts.active_lines()
    );
    let _ = writeln!(output, "scheduler_frame={}", interrupts.frame());
    let _ = writeln!(output, "scheduler_line={}", interrupts.line());
    let _ = writeln!(output, "scheduler_quantum={}", interrupts.quantum());
}

/// Options for `debug memory`.
#[derive(Debug, Default)]
struct MemoryOptions {
    machine: CommonOptions,
    address: Option<u32>,
    length: Option<usize>,
}

fn debug_memory(args: &[String]) -> Result<(), Box<dyn Error>> {
    if crate::tas::help_requested(args, "debug memory")? {
        print_help();
        return Ok(());
    }
    let mut options = MemoryOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        if !parse_common(args, &mut index, &flag, &mut options.machine)? {
            match flag.as_str() {
                "--address" => set_once(
                    &mut options.address,
                    parse_address(option_value(args, &mut index, &flag)?, &flag)?,
                    &flag,
                )?,
                "--length" => set_once(
                    &mut options.length,
                    parse_count(
                        option_value(args, &mut index, &flag)?,
                        &flag,
                        0,
                        MAX_MEMORY_DUMP_BYTES,
                    )?,
                    &flag,
                )?,
                other => {
                    return Err(unknown_option(
                        "memory",
                        other,
                        "--rom <FILE> [--steps <N>] --address <ADDR> --length <N>",
                    )
                    .into());
                }
            }
        }
        index += 1;
    }
    let rom = require_rom("memory", &options.machine)?;
    let steps = steps_for(&options.machine, "memory", false)?;
    let address = options
        .address
        .ok_or_else(|| CliError("`debug memory` requires `--address <ADDR>`".to_owned()))?;
    let length = options
        .length
        .ok_or_else(|| CliError("`debug memory` requires `--length <N>`".to_owned()))?;
    if address & 1 != 0 {
        return Err(CliError(format!(
            "`debug memory` address {} is odd; a memory dump must start at an even address",
            hex32(address)
        ))
        .into());
    }
    let mut machine = open_machine(&rom)?;
    run_instructions(&mut machine, steps, "memory")?;
    let inspection = machine.inspect()?;
    let bytes = if length == 0 {
        Vec::new()
    } else {
        machine
            .read_memory(u64::from(address), length)
            .map_err(|error| CliError(format!("`debug memory`: {error}")))?
    };

    let mut output = String::new();
    begin_record(&mut output, "debug-memory");
    let _ = writeln!(output, "address={}", hex32(address));
    let _ = writeln!(output, "length={length}");
    let _ = writeln!(output, "state_hash={}", inspection.state_hash);
    let _ = writeln!(output, "rows={}", bytes.len().div_ceil(16));
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let row_address = address + u32::try_from(row).unwrap_or(0) * 16;
        let _ = writeln!(output, "{}", hex_row(row_address, chunk));
    }
    print!("{output}");
    Ok(())
}

/// Renders one documented 16-byte hex/ASCII memory row.
///
/// The line is `<address:8 uppercase hex digits>  <16 "XX " byte columns>|<16
/// ASCII characters>|`, with three spaces for each byte past the window and `.`
/// for every non-printable ASCII byte.
fn hex_row(address: u32, bytes: &[u8]) -> String {
    let mut line = format!("{address:08X}  ");
    for index in 0..16 {
        match bytes.get(index) {
            Some(byte) => {
                let _ = write!(line, "{byte:02X} ");
            }
            None => line.push_str("   "),
        }
    }
    line.push('|');
    for index in 0..16 {
        match bytes.get(index) {
            Some(byte) if (0x20..=0x7E).contains(byte) => line.push(char::from(*byte)),
            Some(_) => line.push('.'),
            None => line.push(' '),
        }
    }
    line.push('|');
    line
}

/// Options for `debug disasm`.
#[derive(Debug, Default)]
struct DisasmOptions {
    machine: CommonOptions,
    address: Option<u32>,
    count: Option<usize>,
}

fn debug_disasm(args: &[String]) -> Result<(), Box<dyn Error>> {
    if crate::tas::help_requested(args, "debug disasm")? {
        print_help();
        return Ok(());
    }
    let mut options = DisasmOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        if !parse_common(args, &mut index, &flag, &mut options.machine)? {
            match flag.as_str() {
                "--address" => set_once(
                    &mut options.address,
                    parse_address(option_value(args, &mut index, &flag)?, &flag)?,
                    &flag,
                )?,
                "--count" => set_once(
                    &mut options.count,
                    parse_count(
                        option_value(args, &mut index, &flag)?,
                        &flag,
                        1,
                        MAX_DISASSEMBLY_COUNT,
                    )?,
                    &flag,
                )?,
                other => {
                    return Err(unknown_option(
                        "disasm",
                        other,
                        "--rom <FILE> [--steps <N>] [--address <ADDR>] [--count <N>]",
                    )
                    .into());
                }
            }
        }
        index += 1;
    }
    let rom = require_rom("disasm", &options.machine)?;
    let steps = steps_for(&options.machine, "disasm", false)?;
    let mut machine = open_machine(&rom)?;
    run_instructions(&mut machine, steps, "disasm")?;

    let address = options.address.unwrap_or_else(|| machine.cpu().pc());
    if address & 1 != 0 {
        return Err(CliError(format!(
            "`debug disasm` address {} is odd; disassembly starts at an even address",
            hex32(address)
        ))
        .into());
    }
    let count = options.count.unwrap_or(DEFAULT_DISASSEMBLY_COUNT);
    let rom_image = machine.bus().rom();
    let ram_image = machine.bus().ram();
    let instructions = if usize::try_from(address).is_ok_and(|value| value < rom_image.len()) {
        disassemble(&RomMemory::new(rom_image), address, count)
    } else if (RAM_BASE..RAM_END).contains(&address) {
        disassemble(&WindowMemory::new(RAM_BASE, ram_image), address, count)
    } else {
        return Err(CliError(format!(
            "`debug disasm` address {} is not mapped in the cartridge image or work RAM",
            hex32(address)
        ))
        .into());
    }
    .map_err(|error| CliError(format!("`debug disasm`: {error}")))?;

    let mut output = String::new();
    begin_record(&mut output, "debug-disasm");
    let _ = writeln!(output, "address={}", hex32(address));
    let _ = writeln!(output, "count={count}");
    let _ = writeln!(output, "instructions={}", instructions.len());
    for instruction in &instructions {
        let _ = writeln!(
            output,
            "{:08X}  {:04X}  {}",
            instruction.address(),
            instruction.opcode(),
            instruction.text()
        );
    }
    print!("{output}");
    Ok(())
}

/// Options for `debug vdp`.
#[derive(Debug, Default)]
struct VdpOptions {
    machine: CommonOptions,
    vram_address: Option<u32>,
    vram_length: Option<usize>,
}

fn debug_vdp(args: &[String]) -> Result<(), Box<dyn Error>> {
    if crate::tas::help_requested(args, "debug vdp")? {
        print_help();
        return Ok(());
    }
    let mut options = VdpOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        if !parse_common(args, &mut index, &flag, &mut options.machine)? {
            match flag.as_str() {
                "--vram-address" => set_once(
                    &mut options.vram_address,
                    parse_address(option_value(args, &mut index, &flag)?, &flag)?,
                    &flag,
                )?,
                "--vram-length" => set_once(
                    &mut options.vram_length,
                    parse_count(
                        option_value(args, &mut index, &flag)?,
                        &flag,
                        0,
                        MAX_VRAM_WINDOW_BYTES,
                    )?,
                    &flag,
                )?,
                other => {
                    return Err(unknown_option(
                        "vdp",
                        other,
                        "--rom <FILE> [--steps <N>] [--vram-address <ADDR> --vram-length <N>]",
                    )
                    .into());
                }
            }
        }
        index += 1;
    }
    let rom = require_rom("vdp", &options.machine)?;
    let steps = steps_for(&options.machine, "vdp", false)?;
    let window = match (options.vram_address, options.vram_length) {
        (Some(address), Some(length)) => Some((address, length)),
        (None, None) => None,
        _ => {
            return Err(CliError(
                "`--vram-address` and `--vram-length` must be given together".to_owned(),
            )
            .into());
        }
    };
    let vram_size = u64::try_from(VRAM_SIZE)
        .map_err(|_| CliError("VRAM size does not fit this platform".to_owned()))?;
    if let Some((address, length)) = window {
        let end = u64::from(address)
            .checked_add(u64::try_from(length).unwrap_or(u64::MAX))
            .ok_or_else(|| {
                CliError("`--vram-address` + `--vram-length` overflows the 64-bit range".to_owned())
            })?;
        if end > vram_size {
            return Err(CliError(format!(
                "`--vram-address` + `--vram-length` exceeds the {vram_size}-byte VRAM image"
            ))
            .into());
        }
    }

    let mut machine = open_machine(&rom)?;
    run_instructions(&mut machine, steps, "vdp")?;
    let inspection = machine.inspect()?;
    let vdp = VdpView::new(machine.vdp());

    let mut output = String::new();
    begin_record(&mut output, "debug-vdp");
    let _ = writeln!(output, "tick={}", inspection.tick.raw());
    let _ = writeln!(output, "state_hash={}", inspection.state_hash);
    let _ = writeln!(output, "vdp_status={}", hex16(vdp.status()));
    let _ = writeln!(output, "vdp_vblank={}", u8::from(vdp.vblank()));
    let _ = writeln!(output, "vdp_vint_pending={}", u8::from(vdp.vint_pending()));
    let _ = writeln!(output, "vdp_address={}", hex16(vdp.address()));
    let _ = writeln!(output, "vdp_auto_increment={}", vdp.auto_increment());
    for (index, value) in vdp.registers().iter().enumerate() {
        let _ = writeln!(output, "vdp_register_{index:02}={}", hex8(*value));
    }
    let _ = writeln!(output, "vdp_vram_len={}", vdp.vram_len());
    let _ = writeln!(output, "vdp_vram_sha256={}", vdp.vram_sha256());
    if let Some((address, length)) = window {
        let start = usize::try_from(address)
            .map_err(|_| CliError("`--vram-address` does not fit this platform".to_owned()))?;
        let end = start + length;
        let bytes = machine.vdp().vram().get(start..end).ok_or_else(|| {
            CliError(format!(
                "`--vram-address` + `--vram-length` exceeds the {vram_size}-byte VRAM image"
            ))
        })?;
        let _ = writeln!(output, "vram_address={}", hex32(address));
        let _ = writeln!(output, "vram_length={length}");
        let _ = write!(output, "vram_hex=");
        for byte in bytes {
            let _ = write!(output, "{byte:02X}");
        }
        output.push('\n');
    }
    print!("{output}");
    Ok(())
}

fn debug_interrupts(args: &[String]) -> Result<(), Box<dyn Error>> {
    if crate::tas::help_requested(args, "debug interrupts")? {
        print_help();
        return Ok(());
    }
    let mut common = CommonOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        if !parse_common(args, &mut index, &flag, &mut common)? {
            return Err(unknown_option("interrupts", &flag, "--rom <FILE> [--steps <N>]").into());
        }
        index += 1;
    }
    let rom = require_rom("interrupts", &common)?;
    let steps = steps_for(&common, "interrupts", false)?;
    let mut machine = open_machine(&rom)?;
    run_instructions(&mut machine, steps, "interrupts")?;

    let inspection = machine.inspect()?;
    let view = InterruptView::new(machine.interrupts(), machine.cpu().sr());
    let mut output = String::new();
    begin_record(&mut output, "debug-interrupts");
    let _ = writeln!(output, "tick={}", inspection.tick.raw());
    let _ = writeln!(output, "state_hash={}", inspection.state_hash);
    let _ = writeln!(output, "video={}", video_standard_name(view.standard()));
    let _ = writeln!(
        output,
        "instructions_per_line={}",
        view.instructions_per_line()
    );
    let _ = writeln!(output, "lines_per_frame={}", view.lines_per_frame());
    let _ = writeln!(output, "active_lines={}", view.active_lines());
    let _ = writeln!(output, "frame={}", view.frame());
    let _ = writeln!(output, "line={}", view.line());
    let _ = writeln!(output, "quantum={}", view.quantum());
    let _ = writeln!(output, "pending={}", view.pending().len());
    for (index, pending) in view.pending().iter().enumerate() {
        let _ = writeln!(output, "pending_{index}_level={}", pending.level().level());
        let _ = writeln!(
            output,
            "pending_{index}_issued_tick={}",
            pending.issued_tick()
        );
        let _ = writeln!(
            output,
            "pending_{index}_deliverable={}",
            u8::from(pending.deliverable())
        );
    }
    for (index, counter) in view.counters().iter().enumerate() {
        let _ = writeln!(output, "counter_{index}_level={}", counter.level().level());
        let _ = writeln!(output, "counter_{index}_issued={}", counter.issued());
        let _ = writeln!(output, "counter_{index}_coalesced={}", counter.coalesced());
    }
    print!("{output}");
    Ok(())
}

fn debug_bus(args: &[String]) -> Result<(), Box<dyn Error>> {
    if crate::tas::help_requested(args, "debug bus")? {
        print_help();
        return Ok(());
    }
    let mut common = CommonOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        if !parse_common(args, &mut index, &flag, &mut common)? {
            return Err(unknown_option("bus", &flag, "--rom <FILE>").into());
        }
        index += 1;
    }
    let rom = require_rom("bus", &common)?;
    if common.steps.is_some() {
        return Err(CliError(
            "`debug bus` does not accept `--steps`; the bus layout is state-independent".to_owned(),
        )
        .into());
    }
    let machine = open_machine(&rom)?;
    let view = BusView::new(machine.bus());
    let rom_hash = hash_bytes(machine.bus().rom());
    let ram_hash = hash_bytes(machine.bus().ram());
    let mut output = String::new();
    begin_record(&mut output, "debug-bus");
    let _ = writeln!(output, "rom_len={}", view.rom_len());
    let _ = writeln!(output, "rom_sha256={rom_hash}");
    let _ = writeln!(output, "ram_len={}", view.ram_len());
    let _ = writeln!(output, "ram_sha256={ram_hash}");
    for (index, window) in view.windows().iter().enumerate() {
        let _ = writeln!(output, "window_{index}_name={}", window.name());
        let _ = writeln!(output, "window_{index}_start={}", hex32(window.start()));
        let _ = writeln!(output, "window_{index}_end={}", hex32(window.end()));
        let _ = writeln!(
            output,
            "window_{index}_writable={}",
            u8::from(window.writable())
        );
    }
    print!("{output}");
    Ok(())
}

/// Prints the `chronofork debug` help: implemented subcommands, flags, blocks,
/// limits and the behavior this increment does not claim.
fn print_help() {
    println!("chronofork {VERSION} — debug run|inspect|memory|disasm|vdp|interrupts|bus");
    println!();
    println!("One-shot, headless debugging over the minimal Mega Drive 68000 slice. Every");
    println!("subcommand requires `--rom <FILE>` and reads it through the same bounded,");
    println!("read-only 4 MiB cartridge-window path as `chronofork tas`; the machine uses the");
    println!("default NTSC scheduler (64 instructions per scanline), and no flag changes it.");
    println!("Stops are instruction boundaries: a breakpoint or logpoint matches the state");
    println!("before the next instruction, and a watchpoint or event breakpoint reports the");
    println!("state after the instruction that produced the event. Debug state is process-local");
    println!("and non-canonical: it never changes a snapshot, state hash or configuration hash.");
    println!();
    println!("Usage:");
    println!("  chronofork debug run --rom <FILE> --steps <N>");
    println!("      [--breakpoint <LOCATION>[:<CONDITION>]]...");
    println!("      [--watchpoint <ADDR|START-END>:<r|w|rw>]...");
    println!("      [--eventpoint <FILTER>]...");
    println!("      [--logpoint <LOCATION>[:<MESSAGE>]]...");
    println!("      [--max-events <N>] [--show-events <N>]");
    println!("  chronofork debug inspect --rom <FILE> [--steps <N>]");
    println!("  chronofork debug memory --rom <FILE> [--steps <N>] --address <ADDR> --length <N>");
    println!(
        "  chronofork debug disasm --rom <FILE> [--steps <N>] [--address <ADDR>] [--count <N>]"
    );
    println!("  chronofork debug vdp --rom <FILE> [--steps <N>]");
    println!("      [--vram-address <ADDR> --vram-length <N>]");
    println!("  chronofork debug interrupts --rom <FILE> [--steps <N>]");
    println!("  chronofork debug bus --rom <FILE>");
    println!("  chronofork debug --help");
    println!();
    println!("run:");
    println!("  --rom <FILE>       Required cartridge image, read-only and bounded to 4 MiB");
    println!("  --steps <N>        Required instruction budget, decimal 1..={MAX_DEBUG_STEPS}");
    println!("  --breakpoint <LOCATION>[:<CONDITION>]  Repeatable. Stops before the next");
    println!("      instruction whose PC matches. LOCATION is ADDR or START-END (inclusive).");
    println!("      CONDITION is reg:<name><op><value> or mem:<address>:<length><op><value>,");
    println!("      with <op> one of ==, !=, <, <=, >, >= and a memory length of 1..=8 bytes");
    println!("      compared big-endian through the side-effect-free memory path. A register");
    println!("      name must be a key of the backend inspection (`d0`-`d7`, `a0`-`a7`, `pc`,");
    println!("      `sr`, `irq_mask`, ...). Location and condition are ANDed; an unknown");
    println!("      register, a bad condition or an inverted range is a typed failure.");
    println!("  --watchpoint <ADDR|START-END>:<r|w|rw>  Repeatable. Installs backend bus");
    println!("      watchpoints over an inclusive range and byte-granular access. `rw` installs");
    println!("      one read and one write watchpoint, in that order. A match stops after the");
    println!("      instruction that performed the access and reports the host watchpoint id.");
    println!("      This is real bus observation, never a memory-diff approximation, so a");
    println!("      backend without observation is a typed failure.");
    println!("  --eventpoint <FILTER>  Repeatable. Stops after a step whose retained events");
    println!("      match the filter: interrupt:<requested|delivered|acknowledged>[:<level>],");
    println!("      vdp:<read|write>[:<offset>], controller:<1|2|any> or");
    println!("      bus-write[:<START-END>] (inclusive work-RAM byte write range).");
    println!("  --logpoint <LOCATION>[:<MESSAGE>]  Repeatable. Appends a bounded log entry when");
    println!("      the next instruction's PC matches and never stops. MESSAGE is 1..=128 bytes");
    println!(
        "      without a line break; omitted MESSAGE defaults to `{DEFAULT_LOGPOINT_MESSAGE}`."
    );
    println!("  --max-events <N>   Backend event-ring capacity, 1..={MAX_EVENT_CAPACITY}");
    println!("      (default 1024). A full ring evicts the oldest event and counts it.");
    println!(
        "  --show-events <N>   Print up to N retained events, oldest first, 0..={MAX_SHOW_EVENTS}"
    );
    println!("      (default 0). Events that matched the stopping event breakpoint are always");
    println!("      printed, so the printed set is their union, in recording order.");
    println!();
    println!("  Trigger ids are assigned in registration order: breakpoints, watchpoints,");
    println!("  event breakpoints, then logpoints; each table starts at 1. `rw` consumes two");
    println!("  watchpoint ids (read, then write). Tables are bounded at 64 entries each; a full");
    println!("  table is a typed failure.");
    println!();
    println!("  Run order: a matching breakpoint (location and optional condition) stops before");
    println!("  the instruction; matching logpoints record; a halted CPU stops with `halted`; a");
    println!("  reached budget stops with `step-limit`. Otherwise one neutral-input instruction");
    println!("  executes, and its retained events stop the run with `watchpoint` (all matching");
    println!("  host ids, deduplicated) or `event` (the first matching event breakpoint id). A");
    println!("  watchpoint stop wins over an event stop for the same instruction, and a");
    println!("  breakpoint at the current PC wins over `halted` and `step-limit`.");
    println!();
    println!("inspect:");
    println!("  --rom <FILE>       Required cartridge image");
    println!(
        "  --steps <N>        Neutral-input instructions before the view, 0..={MAX_DEBUG_STEPS}"
    );
    println!("                     (default 0)");
    println!("  Prints one `record=debug-inspect` block: tick, state hash, halted, steps, PC, SR,");
    println!("  supervisor, trace, interrupt mask, X/N/Z/V/C, VDP status/VBlank/VINT-pending/");
    println!("  address/auto-increment and scheduler standard/frame/line/quantum.");
    println!();
    println!("memory:");
    println!("  --address <ADDR>   Required even start address (cartridge image or work RAM)");
    println!("  --length <N>       Required byte count, 0..={MAX_MEMORY_DUMP_BYTES}; 0 prints an");
    println!("                     empty body after the header");
    println!("  Prints `record=debug-memory` with address, length, state hash and rows, plus");
    println!("  one 16-byte row per line: `<address:8 hex>  <16 \"XX \" columns>|<16 ASCII");
    println!("  characters>|`. Bytes past the requested length and non-printable ASCII are");
    println!("  printed as spaces and `.`. The printed state hash proves the side-effect-free");
    println!("  read did not change the machine state.");
    println!();
    println!("disasm:");
    println!("  --address <ADDR>   Even start address, default the current PC; the cartridge");
    println!("                     image and work RAM are the supported windows");
    println!("  --count <N>        Instruction count, 1..={MAX_DISASSEMBLY_COUNT} (default");
    println!("                     {DEFAULT_DISASSEMBLY_COUNT})");
    println!("  Prints `record=debug-disasm` with address/count/instructions and one row per");
    println!("  line: `<address:8 hex>  <opcode:4 hex>  <text>`. Only encodings the 68000");
    println!("  subset executes are rendered; every unknown encoding prints `DC.W` with length");
    println!("  2. An odd address, a missing opcode or extension word, and an unmapped");
    println!("  start are typed failures.");
    println!();
    println!("vdp:");
    println!("  --vram-address <ADDR>  Optional window start, 0..{VRAM_SIZE}; requires");
    println!("                         `--vram-length`");
    println!("  --vram-length <N>      Optional window length, 0..={MAX_VRAM_WINDOW_BYTES}; the");
    println!("                         window must stay inside the 64 KiB VRAM image");
    println!("  Prints `record=debug-vdp` with status bits, address, auto-increment, all 24");
    println!("  registers as `vdp_register_00`..`vdp_register_23`, VRAM length and SHA-256, and,");
    println!("  for a requested window, its address, length and contiguous uppercase `vram_hex`");
    println!("  bytes.");
    println!();
    println!("interrupts:");
    println!("  Prints `record=debug-interrupts` with tick, state hash, video standard,");
    println!("  instructions per line, lines per frame, active lines, frame/line/quantum");
    println!("  position, the pending request count, each pending request's level, issue tick");
    println!("  and CPU-mask `deliverable` flag, and the per-level issued/coalesced counters in");
    println!("  HBlank, VBlank order. A masked request stays pending and reports deliverable=0.");
    println!();
    println!("bus:");
    println!("  Prints `record=debug-bus` with ROM and work-RAM lengths, their SHA-256 digests");
    println!("  and the four mapped windows as `window_<index>_name`, `window_<index>_start`,");
    println!("  `window_<index>_end` and `window_<index>_writable`. `end` is exclusive; windows");
    println!("  are printed in ascending address order.");
    println!();
    println!("Number grammar:");
    println!("  Addresses and values accept decimal or `0x`/`0X` hexadecimal. Addresses are");
    println!("  32-bit; an overflow, a malformed token, an empty field or an unknown keyword is");
    println!("  a typed CLI failure (exit 1, empty stdout). Hex output is uppercase: addresses");
    println!("  are `0x` plus 8 digits, 16-bit values `0x` plus 4, bytes `0x` plus 2, and event");
    println!("  values `0x` plus 8. Ticks and `--steps` are decimal; row counts and lengths");
    println!("  also accept `0x`/`0X` hexadecimal.");
    println!();
    println!("Output:");
    println!("  `record=<kind>` / `key=value` blocks separated by exactly one blank line on");
    println!("  stdout, in this order for `run`: `debug-run`, then `debug-event` blocks, then");
    println!("  `debug-log` blocks. `debug-run` carries status, reason");
    println!("  (breakpoint|watchpoint|event|halted|step-limit), tick, state_hash, pc, steps,");
    println!("  halted, the matched breakpoint_id/watchpoint_ids/eventpoint_id (`-` when none),");
    println!("  events_matched/events_retained/events_dropped and log_entries/log_dropped.");
    println!("  Every `debug-event` block carries tick, kind, matched, address, offset, port,");
    println!("  value, width, access, level, phase and watchpoint_id, using `-` for a field the");
    println!("  event class does not define. `debug-log` carries logpoint_id, tick, pc, hits and");
    println!("  message. Valid output contains no timestamp, process id, host path or entropy.");
    println!();
    println!("Exit codes:");
    println!("  0  the run or read completed, including every successful stop reason");
    println!("  1  a typed failure; the diagnostic starts with `chronofork: ` on stderr and");
    println!("     stdout stays empty for the failing operation");
    println!();
    println!("Limits and non-goals:");
    println!("  Event observation is byte-granular on the bus: a work-RAM word write records two");
    println!("  `bus-write` and two matching `watchpoint` events, and a VDP long access records");
    println!("  two width-2 `vdp-port` events; a VDP word access is one width-2 event.");
    println!("  Controller events name hardware ports 1 and 2. Debugger memory reads and writes");
    println!("  never produce events, instruction fetches are logged only when a watchpoint");
    println!("  matches them, and an event carries the tick at the start of its instruction.");
    println!("  Not implemented: mid-instruction or cycle-exact stops, reverse execution,");
    println!("  call-stack/step-over/step-out, conditional watchpoint values, bus-read logging");
    println!("  without a watchpoint, TH/6-button controller protocols, DMA/Z80/audio/rendering");
    println!("  inspection, persistent debugger state, an interactive REPL, a GUI, HTTP or MCP.");
}
