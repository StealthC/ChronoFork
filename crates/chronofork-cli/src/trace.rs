//! `chronofork trace` — thin adapter over the `chronofork-trace` library.
//!
//! The command group owns argument parsing, bounded file I/O and deterministic
//! `key=value` report blocks. It never re-implements the canonical codec, the
//! capture driver, the query matcher or the first-divergence comparison: every
//! behavior comes from `chronofork_trace` and every capture goes through
//! `SessionManager::capture_trace`.

use std::error::Error;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use chronofork_core::hash_bytes;
use chronofork_sessions::SessionError;
use chronofork_trace::{
    DEFAULT_EVENT_CAPACITY, DEFAULT_TRACE_CAPACITY, MAX_CAPTURE_STEPS, MAX_EVENT_CAPACITY,
    MAX_QUERY_LIMIT, MAX_TRACE_BYTES, MAX_TRACE_CAPACITY, TRACE_FORMAT_VERSION, Trace, TraceConfig,
    TraceError, TraceKind, TraceKindMask, TraceQuery, TraceRecord, TraceStart, TraceStop,
};

use crate::{
    CliError, begin_record, ensure, option_value, parse_hex_u32, parse_u32, parse_u64, parse_usize,
    set_once,
};

/// Default step budget for `trace capture`.
const DEFAULT_STEPS: u64 = 1_000;
/// Default query limit; every matching record up to the library maximum.
const DEFAULT_QUERY_LIMIT: usize = MAX_QUERY_LIMIT;

/// Runs one `chronofork trace` command.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            ensure(args.len() == 1, "`trace --help` takes no other arguments")?;
            print_help();
            Ok(())
        }
        Some("capture") => capture_command(&args[1..]),
        Some("info") => info_command(&args[1..]),
        Some("query") => query_command(&args[1..]),
        Some("compare") => compare_command(&args[1..]),
        Some("export") => export_command(&args[1..]),
        None => Err(CliError(
            "`trace` requires a subcommand: capture, info, query, compare or export".to_owned(),
        )
        .into()),
        Some(other) => Err(CliError(format!(
            "unknown `trace` subcommand `{other}`; expected capture, info, query, compare or \
             export"
        ))
        .into()),
    }
}

/// One of the four `--kinds`/`--kind` groups.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KindGroup {
    Instruction,
    Bus,
    Device,
    Interrupt,
}

impl KindGroup {
    const ALL: [Self; 4] = [Self::Instruction, Self::Bus, Self::Device, Self::Interrupt];

    const fn name(self) -> &'static str {
        match self {
            Self::Instruction => "instruction",
            Self::Bus => "bus",
            Self::Device => "device",
            Self::Interrupt => "interrupt",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Instruction => 1,
            Self::Bus => 2,
            Self::Device => 4,
            Self::Interrupt => 8,
        }
    }

    fn kinds(self) -> &'static [TraceKind] {
        match self {
            Self::Instruction => &[TraceKind::Instruction],
            Self::Bus => &[TraceKind::BusWrite],
            Self::Device => &[TraceKind::VdpPort, TraceKind::ControllerRead],
            Self::Interrupt => &[TraceKind::Interrupt],
        }
    }

    fn mask(self) -> TraceKindMask {
        let mut mask = TraceKindMask::none();
        for kind in self.kinds() {
            mask = mask.with(*kind);
        }
        mask
    }
}

fn parse_kind_group(value: &str, flag: &str) -> Result<KindGroup, CliError> {
    KindGroup::ALL
        .into_iter()
        .find(|group| group.name() == value)
        .ok_or_else(|| {
            CliError(format!(
                "`{flag}` expects one of instruction, bus, device or interrupt, got `{value}`"
            ))
        })
}

fn parse_kinds(value: &str) -> Result<TraceKindMask, CliError> {
    if value.is_empty() {
        return Err(CliError(
            "`--kinds` requires a non-empty comma list of instruction, bus, device and interrupt"
                .to_owned(),
        ));
    }
    let mut mask = TraceKindMask::none();
    let mut seen = 0_u8;
    for name in value.split(',') {
        let group = parse_kind_group(name, "--kinds")?;
        if seen & group.bit() != 0 {
            return Err(CliError(format!(
                "`--kinds` lists `{}` more than once",
                group.name()
            )));
        }
        seen |= group.bit();
        for kind in group.kinds() {
            mask = mask.with(*kind);
        }
    }
    Ok(mask)
}

/// Returns whether at least one kind of `group` is enabled in `mask`.
fn group_enabled(mask: TraceKindMask, group: KindGroup) -> bool {
    group.kinds().iter().any(|kind| mask.contains(*kind))
}

/// Formats the enabled groups as a fixed-order comma list, or `none`.
fn kinds_name(mask: TraceKindMask) -> String {
    let names: Vec<&str> = KindGroup::ALL
        .into_iter()
        .filter(|group| group_enabled(mask, *group))
        .map(KindGroup::name)
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(",")
    }
}

fn write_enabled_groups(output: &mut String, mask: TraceKindMask) {
    for group in KindGroup::ALL {
        let _ = writeln!(
            output,
            "{}_enabled={}",
            group.name(),
            u8::from(group_enabled(mask, group))
        );
    }
}

fn trace_error(context: &str, error: TraceError) -> Box<dyn Error> {
    CliError(format!("{context}: [{}] {error}", error.code())).into()
}

fn capture_session_error(error: SessionError) -> Box<dyn Error> {
    match error {
        SessionError::Trace(error) => trace_error("trace capture failed", error),
        other => CliError(format!("trace capture failed: {other}")).into(),
    }
}

fn non_empty_path(value: &str, flag: &str) -> Result<PathBuf, CliError> {
    if value.is_empty() {
        return Err(CliError(format!("`{flag}` requires a non-empty path")));
    }
    Ok(PathBuf::from(value))
}

/// Reads a trace file through one bounded, read-only path.
fn read_trace_file(path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
    let file = File::open(path).map_err(|error| {
        CliError(format!(
            "failed to open trace file `{}`: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        CliError(format!(
            "failed to inspect trace file `{}`: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(CliError(format!(
            "trace path `{}` is not a regular file",
            path.display()
        ))
        .into());
    }
    let limit = u64::try_from(MAX_TRACE_BYTES)
        .map_err(|_| CliError("trace byte limit does not fit u64".to_owned()))?;
    if metadata.len() > limit {
        return Err(CliError(format!(
            "trace file `{}` is {} bytes, limit is {MAX_TRACE_BYTES}",
            path.display(),
            metadata.len()
        ))
        .into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError(format!(
                "failed to read trace file `{}`: {error}",
                path.display()
            ))
        })?;
    if bytes.len() as u64 > limit {
        return Err(CliError(format!(
            "trace file `{}` exceeds the {MAX_TRACE_BYTES}-byte limit",
            path.display()
        ))
        .into());
    }
    Ok(bytes)
}

/// Decodes one trace file through the single reject-only codec path.
fn read_trace(path: &Path) -> Result<(Trace, Vec<u8>), Box<dyn Error>> {
    let bytes = read_trace_file(path)?;
    let trace = Trace::from_bytes(&bytes).map_err(|error| {
        CliError(format!(
            "invalid trace file `{}`: [{}] {error}",
            path.display(),
            error.code()
        ))
    })?;
    Ok((trace, bytes))
}

/// Publishes bytes through a sibling temporary file renamed on success.
///
/// The complete payload is written before the rename, so a failed validation or
/// write never leaves a partial output file. The temporary name contains only the
/// process id; no host timestamp enters a payload or report.
fn write_payload(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let file_name = path.file_name().ok_or_else(|| {
        CliError(format!(
            "`--out` must name a file, not a directory: `{}`",
            path.display()
        ))
    })?;
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut temporary_name = OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".tmp-{}", std::process::id()));
    let temporary = parent.join(temporary_name);
    std::fs::write(&temporary, bytes).map_err(|error| {
        CliError(format!(
            "failed to write trace output `{}`: {error}",
            temporary.display()
        ))
    })?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(CliError(format!(
            "failed to publish trace output `{}`: {error}",
            path.display()
        ))
        .into());
    }
    Ok(())
}

fn print_trace_help(args: &[String]) -> Result<bool, Box<dyn Error>> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        ensure(
            args.len() == 1,
            "`trace <subcommand> --help` takes no other arguments",
        )?;
        print_help();
        return Ok(true);
    }
    Ok(false)
}

/// Options for `trace capture`.
#[derive(Debug)]
struct CaptureOptions {
    rom: Option<PathBuf>,
    steps: u64,
    input: Option<u32>,
    capacity: usize,
    event_capacity: usize,
    kinds: TraceKindMask,
    start: TraceStart,
    stop: TraceStop,
    out: PathBuf,
}

fn parse_capture_options(args: &[String]) -> Result<Option<CaptureOptions>, Box<dyn Error>> {
    if print_trace_help(args)? {
        return Ok(None);
    }
    let mut rom: Option<PathBuf> = None;
    let mut steps: Option<u64> = None;
    let mut input: Option<u32> = None;
    let mut capacity: Option<usize> = None;
    let mut event_capacity: Option<usize> = None;
    let mut kinds: Option<TraceKindMask> = None;
    let mut start_pc: Option<u32> = None;
    let mut start_tick: Option<u64> = None;
    let mut stop_pc: Option<u32> = None;
    let mut stop_tick: Option<u64> = None;
    let mut out: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--rom" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut rom, non_empty_path(value, &flag)?, &flag)?;
            }
            "--steps" => set_once(
                &mut steps,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--input" => set_once(
                &mut input,
                parse_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--capacity" => set_once(
                &mut capacity,
                parse_usize(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--event-capacity" => set_once(
                &mut event_capacity,
                parse_usize(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--kinds" => set_once(
                &mut kinds,
                parse_kinds(option_value(args, &mut index, &flag)?)?,
                &flag,
            )?,
            "--start-pc" => set_once(
                &mut start_pc,
                parse_hex_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--start-tick" => set_once(
                &mut start_tick,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--stop-pc" => set_once(
                &mut stop_pc,
                parse_hex_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--stop-tick" => set_once(
                &mut stop_tick,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--out" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut out, non_empty_path(value, &flag)?, &flag)?;
            }
            other => {
                return Err(CliError(format!("unknown `trace capture` flag `{other}`")).into());
            }
        }
        index += 1;
    }

    let steps = steps.unwrap_or(DEFAULT_STEPS);
    if steps == 0 || steps > MAX_CAPTURE_STEPS {
        return Err(CliError(format!(
            "`--steps` must be in 1..={MAX_CAPTURE_STEPS}, got {steps}"
        ))
        .into());
    }
    let capacity = capacity.unwrap_or(DEFAULT_TRACE_CAPACITY);
    if capacity == 0 || capacity > MAX_TRACE_CAPACITY {
        return Err(CliError(format!(
            "`--capacity` must be in 1..={MAX_TRACE_CAPACITY}, got {capacity}"
        ))
        .into());
    }
    let event_capacity = event_capacity.unwrap_or(DEFAULT_EVENT_CAPACITY);
    if event_capacity == 0 || event_capacity > MAX_EVENT_CAPACITY {
        return Err(CliError(format!(
            "`--event-capacity` must be in 1..={MAX_EVENT_CAPACITY}, got {event_capacity}"
        ))
        .into());
    }
    if start_pc.is_some() && start_tick.is_some() {
        return Err(
            CliError("`--start-pc` and `--start-tick` are mutually exclusive".to_owned()).into(),
        );
    }
    if stop_pc.is_some() && stop_tick.is_some() {
        return Err(
            CliError("`--stop-pc` and `--stop-tick` are mutually exclusive".to_owned()).into(),
        );
    }
    let start = match (start_pc, start_tick) {
        (Some(pc), None) => TraceStart::Pc(pc),
        (None, Some(tick)) => TraceStart::Tick(chronofork_core::Tick::from_raw(tick)),
        _ => TraceStart::Immediate,
    };
    let stop = match (stop_pc, stop_tick) {
        (Some(pc), None) => TraceStop::Pc(pc),
        (None, Some(tick)) => TraceStop::Tick(chronofork_core::Tick::from_raw(tick)),
        _ => TraceStop::None,
    };
    let out = out.ok_or_else(|| CliError("`trace capture` requires `--out <FILE>`".to_owned()))?;
    Ok(Some(CaptureOptions {
        rom,
        steps,
        input,
        capacity,
        event_capacity,
        kinds: kinds.unwrap_or_else(TraceKindMask::all),
        start,
        stop,
        out,
    }))
}

fn capture_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_capture_options(args)? else {
        return Ok(());
    };
    let mut manager = crate::tas::session_manager(options.rom.as_deref())?;
    let session = manager.create_session()?;
    let input = crate::input_frame(options.input);
    let config = TraceConfig::new(options.capacity)
        .and_then(|config| config.with_event_capacity(options.event_capacity))
        .map_err(|error| trace_error("invalid trace configuration", error))?
        .with_start(options.start)
        .with_stop(options.stop)
        .with_kinds(options.kinds)
        .with_input(input);
    let trace = manager
        .capture_trace(session, &config, options.steps)
        .map_err(capture_session_error)?;
    let bytes = trace.to_bytes();
    write_payload(&options.out, &bytes)?;

    let mut output = String::new();
    let meta = trace.meta();
    begin_record(&mut output, "trace_capture");
    let _ = writeln!(output, "backend={}", meta.backend);
    let _ = writeln!(output, "backend_version={}", meta.backend_version);
    let _ = writeln!(output, "start_tick={}", meta.start_tick);
    let _ = writeln!(output, "end_tick={}", meta.end_tick);
    // The report field is the captured tick delta (ADR-0011 §3.6), not a separate
    // executed-step counter: both acceptance backends advance one tick per step.
    let _ = writeln!(
        output,
        "steps={}",
        meta.end_tick.raw().saturating_sub(meta.start_tick.raw())
    );
    let _ = writeln!(output, "capacity={}", meta.capacity);
    let _ = writeln!(output, "records={}", trace.record_count());
    let _ = writeln!(output, "first_index={}", meta.first_index);
    let _ = writeln!(output, "dropped={}", meta.dropped);
    let _ = writeln!(output, "stop_reason={}", meta.stop_reason.name());
    let _ = writeln!(output, "triggered={}", u8::from(meta.triggered));
    let _ = writeln!(output, "events_observed={}", u8::from(meta.events_observed));
    let _ = writeln!(output, "kinds={}", kinds_name(options.kinds));
    write_enabled_groups(&mut output, options.kinds);
    let _ = writeln!(output, "bytes={}", bytes.len());
    let _ = writeln!(output, "sha256={}", hash_bytes(&bytes).to_hex());
    print!("{output}");
    Ok(())
}

fn info_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    if print_trace_help(args)? {
        return Ok(());
    }
    let mut file: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--file" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut file, non_empty_path(value, &flag)?, &flag)?;
            }
            other => return Err(CliError(format!("unknown `trace info` flag `{other}`")).into()),
        }
        index += 1;
    }
    let file = file.ok_or_else(|| CliError("`trace info` requires `--file <FILE>`".to_owned()))?;
    let (trace, bytes) = read_trace(&file)?;
    let meta = trace.meta();
    let mut output = String::new();
    begin_record(&mut output, "trace_info");
    let _ = writeln!(output, "format_version={TRACE_FORMAT_VERSION}");
    let _ = writeln!(output, "backend={}", meta.backend);
    let _ = writeln!(output, "backend_version={}", meta.backend_version);
    let _ = writeln!(output, "state_hash={}", meta.state_hash);
    let _ = writeln!(output, "start_tick={}", meta.start_tick);
    let _ = writeln!(output, "end_tick={}", meta.end_tick);
    let _ = writeln!(output, "capacity={}", meta.capacity);
    let _ = writeln!(output, "records={}", trace.record_count());
    let _ = writeln!(output, "first_index={}", meta.first_index);
    let _ = writeln!(output, "dropped={}", meta.dropped);
    let _ = writeln!(output, "stop_reason={}", meta.stop_reason.name());
    let _ = writeln!(output, "triggered={}", u8::from(meta.triggered));
    let _ = writeln!(output, "events_observed={}", u8::from(meta.events_observed));
    let _ = writeln!(output, "bytes={}", bytes.len());
    let _ = writeln!(output, "sha256={}", hash_bytes(&bytes).to_hex());
    print!("{output}");
    Ok(())
}

fn write_query_record(output: &mut String, record: &TraceRecord) {
    begin_record(output, "trace_record");
    let _ = writeln!(output, "index={}", record.index());
    let _ = writeln!(output, "tick={}", record.tick());
    let _ = writeln!(output, "kind={}", record.kind().name());
    match record {
        TraceRecord::Instruction { pc, opcode, .. } => {
            let _ = writeln!(output, "pc={pc:06X}");
            let _ = writeln!(output, "opcode={opcode:04X}");
        }
        TraceRecord::BusWrite {
            address,
            value,
            width,
            ..
        } => {
            let _ = writeln!(output, "address={address:08X}");
            let _ = writeln!(output, "value={value:08X}");
            let _ = writeln!(output, "width={width}");
        }
        TraceRecord::VdpPort {
            offset,
            value,
            width,
            write,
            ..
        } => {
            let _ = writeln!(output, "offset={offset:02X}");
            let _ = writeln!(output, "value={value:08X}");
            let _ = writeln!(output, "width={width}");
            let _ = writeln!(output, "write={}", u8::from(*write));
        }
        TraceRecord::ControllerRead { port, value, .. } => {
            let _ = writeln!(output, "port={port}");
            let _ = writeln!(output, "value={value:02X}");
        }
        TraceRecord::Interrupt { level, phase, .. } => {
            let _ = writeln!(output, "level={level}");
            let _ = writeln!(output, "phase={}", phase.name());
        }
    }
}

/// Options for `trace query`.
#[derive(Debug)]
struct QueryOptions {
    file: PathBuf,
    from_tick: Option<u64>,
    to_tick: Option<u64>,
    kind: Option<KindGroup>,
    pc: Option<u32>,
    address: Option<u32>,
    limit: usize,
}

fn parse_query_options(args: &[String]) -> Result<Option<QueryOptions>, Box<dyn Error>> {
    if print_trace_help(args)? {
        return Ok(None);
    }
    let mut file: Option<PathBuf> = None;
    let mut from_tick: Option<u64> = None;
    let mut to_tick: Option<u64> = None;
    let mut kind: Option<KindGroup> = None;
    let mut pc: Option<u32> = None;
    let mut address: Option<u32> = None;
    let mut limit: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--file" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut file, non_empty_path(value, &flag)?, &flag)?;
            }
            "--from-tick" => set_once(
                &mut from_tick,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--to-tick" => set_once(
                &mut to_tick,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--kind" => set_once(
                &mut kind,
                parse_kind_group(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--pc" => set_once(
                &mut pc,
                parse_hex_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--address" => set_once(
                &mut address,
                parse_hex_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--limit" => set_once(
                &mut limit,
                parse_usize(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            other => return Err(CliError(format!("unknown `trace query` flag `{other}`")).into()),
        }
        index += 1;
    }
    let file = file.ok_or_else(|| CliError("`trace query` requires `--file <FILE>`".to_owned()))?;
    let limit = limit.unwrap_or(DEFAULT_QUERY_LIMIT);
    if limit == 0 || limit > MAX_QUERY_LIMIT {
        return Err(CliError(format!(
            "`--limit` must be in 1..={MAX_QUERY_LIMIT}, got {limit}"
        ))
        .into());
    }
    Ok(Some(QueryOptions {
        file,
        from_tick,
        to_tick,
        kind,
        pc,
        address,
        limit,
    }))
}

fn query_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_query_options(args)? else {
        return Ok(());
    };
    let (trace, _) = read_trace(&options.file)?;
    let mask = options
        .kind
        .map_or_else(TraceKindMask::all, KindGroup::mask);
    let mut query = TraceQuery::new(options.limit).with_kinds(mask);
    if let Some(from) = options.from_tick {
        query = query.with_from_tick(chronofork_core::Tick::from_raw(from));
    }
    if let Some(to) = options.to_tick {
        query = query.with_to_tick(chronofork_core::Tick::from_raw(to));
    }
    if let Some(pc) = options.pc {
        query = query.with_pc(pc);
    }
    if let Some(address) = options.address {
        query = query.with_address(address);
    }
    let result = trace
        .query(&query)
        .map_err(|error| trace_error("trace query failed", error))?;

    let mut output = String::new();
    begin_record(&mut output, "trace_query");
    let _ = writeln!(output, "matched={}", result.matched);
    let _ = writeln!(output, "returned={}", result.returned.len());
    let _ = writeln!(output, "truncated={}", u8::from(result.truncated));
    let _ = writeln!(output, "from_tick={}", optional_number(options.from_tick));
    let _ = writeln!(output, "to_tick={}", optional_number(options.to_tick));
    let _ = writeln!(
        output,
        "kind={}",
        options.kind.map_or("all", |group| group.name())
    );
    write_enabled_groups(&mut output, mask);
    let _ = writeln!(output, "pc={}", optional_hex(options.pc));
    let _ = writeln!(output, "address={}", optional_hex(options.address));
    let _ = writeln!(output, "limit={}", options.limit);
    for record in &result.returned {
        write_query_record(&mut output, record);
    }
    print!("{output}");
    Ok(())
}

fn optional_number(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

fn optional_hex(value: Option<u32>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| format!("{value:08X}"))
}

fn compare_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    if print_trace_help(args)? {
        return Ok(());
    }
    let mut left: Option<PathBuf> = None;
    let mut right: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--left" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut left, non_empty_path(value, &flag)?, &flag)?;
            }
            "--right" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut right, non_empty_path(value, &flag)?, &flag)?;
            }
            other => return Err(CliError(format!("unknown `trace compare` flag `{other}`")).into()),
        }
        index += 1;
    }
    let left =
        left.ok_or_else(|| CliError("`trace compare` requires `--left <FILE>`".to_owned()))?;
    let right =
        right.ok_or_else(|| CliError("`trace compare` requires `--right <FILE>`".to_owned()))?;
    let (left_trace, _) = read_trace(&left)?;
    let (right_trace, _) = read_trace(&right)?;
    let comparison = left_trace.compare(&right_trace);
    let mut output = String::new();
    begin_record(&mut output, "trace_compare");
    let _ = writeln!(output, "identical={}", u8::from(comparison.identical));
    let _ = writeln!(
        output,
        "metadata_equal={}",
        u8::from(comparison.metadata_equal)
    );
    match &comparison.divergence {
        Some(divergence) => {
            let _ = writeln!(output, "divergence={}", divergence.index);
            let _ = writeln!(output, "divergence_tick={}", divergence.tick);
            let _ = writeln!(output, "reason={}", divergence.reason.name());
            let _ = writeln!(
                output,
                "left={}",
                divergence
                    .left
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), TraceRecord::summary)
            );
            let _ = writeln!(
                output,
                "right={}",
                divergence
                    .right
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), TraceRecord::summary)
            );
        }
        None => {
            let _ = writeln!(output, "divergence=-1");
            let _ = writeln!(output, "divergence_tick=-1");
            let _ = writeln!(output, "reason=none");
            let _ = writeln!(output, "left=-");
            let _ = writeln!(output, "right=-");
        }
    }
    print!("{output}");
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExportFormat {
    Tsv,
    Canonical,
}

impl ExportFormat {
    const fn name(self) -> &'static str {
        match self {
            Self::Tsv => "tsv",
            Self::Canonical => "canonical",
        }
    }
}

fn parse_export_format(value: &str) -> Result<ExportFormat, CliError> {
    match value {
        "tsv" => Ok(ExportFormat::Tsv),
        "canonical" => Ok(ExportFormat::Canonical),
        other => Err(CliError(format!(
            "`--format` expects `tsv` or `canonical`, got `{other}`"
        ))),
    }
}

fn export_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    if print_trace_help(args)? {
        return Ok(());
    }
    let mut file: Option<PathBuf> = None;
    let mut format: Option<ExportFormat> = None;
    let mut out: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--file" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut file, non_empty_path(value, &flag)?, &flag)?;
            }
            "--format" => set_once(
                &mut format,
                parse_export_format(option_value(args, &mut index, &flag)?)?,
                &flag,
            )?,
            "--out" => {
                let value = option_value(args, &mut index, &flag)?;
                set_once(&mut out, non_empty_path(value, &flag)?, &flag)?;
            }
            other => return Err(CliError(format!("unknown `trace export` flag `{other}`")).into()),
        }
        index += 1;
    }
    let file =
        file.ok_or_else(|| CliError("`trace export` requires `--file <FILE>`".to_owned()))?;
    let out = out.ok_or_else(|| CliError("`trace export` requires `--out <FILE>`".to_owned()))?;
    let format = format.unwrap_or(ExportFormat::Tsv);
    let (trace, _) = read_trace(&file)?;
    let payload = match format {
        ExportFormat::Canonical => trace.to_bytes(),
        ExportFormat::Tsv => trace.to_tsv().into_bytes(),
    };
    write_payload(&out, &payload)?;
    let mut output = String::new();
    begin_record(&mut output, "trace_export");
    let _ = writeln!(output, "format={}", format.name());
    let _ = writeln!(output, "records={}", trace.record_count());
    let _ = writeln!(output, "bytes={}", payload.len());
    let _ = writeln!(output, "sha256={}", hash_bytes(&payload).to_hex());
    print!("{output}");
    Ok(())
}

/// Prints the `chronofork trace --help` text.
fn print_help() {
    println!("chronofork trace — host-provided structured execution tracing");
    println!();
    println!("Tracing is a host feature over backend inspection and memory access. Bus, VDP,");
    println!("controller and interrupt records additionally require backend debug_events; the");
    println!("backend capability `tracing` stays false. Traces are observation artifacts, never");
    println!("canonical state.");
    println!();
    println!("Usage:");
    println!("  chronofork trace capture [--rom <FILE>] [--steps <N>] [--input <U32>]");
    println!("      [--capacity <N>] [--event-capacity <N>] [--kinds <LIST>]");
    println!("      [--start-pc <ADDR>] [--start-tick <T>] [--stop-pc <ADDR>]");
    println!("      [--stop-tick <T>] --out <FILE>");
    println!("  chronofork trace info --file <FILE>");
    println!("  chronofork trace query --file <FILE> [--from-tick <T>] [--to-tick <T>]");
    println!("      [--kind <instruction|bus|device|interrupt>] [--pc <ADDR>]");
    println!("      [--address <ADDR>] [--limit <N>]");
    println!("  chronofork trace compare --left <FILE> --right <FILE>");
    println!("  chronofork trace export --file <FILE> [--format tsv|canonical] --out <FILE>");
    println!("  chronofork trace --help");
    println!();
    println!("Defaults: --steps {DEFAULT_STEPS}, --capacity {DEFAULT_TRACE_CAPACITY},");
    println!("--event-capacity {DEFAULT_EVENT_CAPACITY}, all kinds, neutral input, --limit");
    println!("{DEFAULT_QUERY_LIMIT}, --format tsv. Without --rom the deterministic synthetic test");
    println!(
        "backend is used; --rom <FILE> selects the minimal Mega Drive slice. --input <U32> is"
    );
    println!("the port-0 input mask for every step. --start-pc/--stop-pc accept hexadecimal");
    println!("addresses (with or without 0x); --start-tick/--stop-tick are decimal ticks.");
    println!("`steps` in the report is the tick delta end_tick - start_tick; both acceptance");
    println!("backends advance one tick per executed step, so it equals the executed-step count");
    println!("there. A stop trigger can execute fewer than --steps, and stop_reason names why the");
    println!("run ended.");
    println!();
    println!("--kinds is a comma list of instruction, bus, device and interrupt: bus is");
    println!("BusWrite and device is VdpPort + ControllerRead. Duplicates, unknown names and an");
    println!("empty list are typed errors. At most one start flag and one stop flag may be given.");
    println!("--out is required for capture and export, so stdout stays a pure key=value report");
    println!("and never receives binary or TSV payload.");
    println!();
    println!("Limits: trace files are read through the {MAX_TRACE_BYTES}-byte format bound, and");
    println!("invalid, truncated or oversized files are typed rejections.");
    println!();
    println!("Output blocks (record=<name> then key=value, one blank line between blocks):");
    println!("  trace_capture: backend, backend_version, start_tick, end_tick, steps,");
    println!("      capacity, records, first_index, dropped, stop_reason, triggered,");
    println!("      events_observed, kinds, instruction_enabled, bus_enabled,");
    println!("      device_enabled, interrupt_enabled, bytes, sha256");
    println!("  trace_info: format_version, backend, backend_version, state_hash, start_tick,");
    println!("      end_tick, capacity, records, first_index, dropped, stop_reason,");
    println!("      triggered, events_observed, bytes, sha256");
    println!("  trace_query: matched, returned, truncated, from_tick, to_tick, kind,");
    println!("      instruction_enabled, bus_enabled, device_enabled, interrupt_enabled,");
    println!("      pc, address, limit, then one trace_record block per returned record");
    println!("      (kind is the --kind group, or all when no filter is given)");
    println!("  trace_record: index, tick, kind and only that kind's fields");
    println!("      (pc/opcode; address/value/width; offset/value/width/write; port/value;");
    println!("      level/phase)");
    println!("  trace_compare: identical, metadata_equal, divergence, divergence_tick,");
    println!("      reason (record|left_missing|right_missing|none), left, right");
    println!("  trace_export: format, records, bytes, sha256");
    println!();
    println!("Unselected kind groups are reported as <group>_enabled=0. `from_tick`,");
    println!("`to_tick`, `pc`, `address`, `left` and `right` use `-` when absent. A divergent");
    println!("comparison is a completed command (exit 0); an unreadable or invalid file exits 1.");
    println!("Typed failures exit 1 with empty stdout and a `chronofork: ...` stderr line; help");
    println!("exits 0 with usage on stdout. There is no TSV import path and no trace viewer.");
    println!();
    println!("Examples:");
    println!("  chronofork trace capture --steps 100 --out run.cftrace");
    println!("  chronofork trace query --file run.cftrace --kind instruction --limit 10");
    println!("  chronofork trace export --file run.cftrace --format canonical --out copy.cftrace");
}
