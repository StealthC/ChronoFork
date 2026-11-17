//! `chronofork script` process boundary: one sandboxed deterministic run.
//!
//! This module is a thin, line-oriented adapter over [`chronofork_script`]: it
//! parses flags, reads one bounded UTF-8 source file, injects the backend factory
//! and the optional sandbox root, and renders the deterministic report as
//! `record=script_print` / `record=script_report` blocks. The grammar, limits,
//! sandbox checks and report field order stay in the library; this code never
//! re-implements a lexer, a limit predicate or the report model.

use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_script::{ScriptLimits, ScriptOptions, ScriptRun, run_source};

use crate::{CliError, VERSION, begin_record, ensure, option_value, parse_u64, set_once};

/// Error marker for a script-level failed run whose report was already printed.
///
/// `main` recognizes this value and exits 1 without printing a `chronofork:`
/// diagnostic, keeping a failed run's stderr empty as promised by ADR-0011.
#[derive(Debug)]
pub(crate) struct ScriptFailed;

impl fmt::Display for ScriptFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("script run failed")
    }
}

impl Error for ScriptFailed {}

/// Runs one `chronofork script` subcommand.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("run") => script_run(&args[1..]),
        Some("--help" | "-h") => {
            ensure(args.len() == 1, "`script --help` takes no other arguments")?;
            print_help();
            Ok(())
        }
        None => Err(CliError("`script` requires a subcommand: run".to_owned()).into()),
        Some(other) => Err(CliError(format!(
            "unknown `script` subcommand `{other}`; expected run"
        ))
        .into()),
    }
}

/// Parsed `script run` flags, before limit validation.
#[derive(Debug, Default)]
struct RunOptions {
    file: Option<PathBuf>,
    rom: Option<PathBuf>,
    sandbox: Option<PathBuf>,
    max_operations: Option<u64>,
    max_loop_iterations: Option<u64>,
    max_steps: Option<u64>,
    max_memory_bytes: Option<u64>,
    max_source_bytes: Option<u64>,
    max_report_bytes: Option<u64>,
    max_variables: Option<u64>,
    wall_time_ms: Option<u64>,
}

fn parse_run_options(args: &[String]) -> Result<RunOptions, CliError> {
    let mut parsed = RunOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--file" => set_once(
                &mut parsed.file,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--rom" => set_once(
                &mut parsed.rom,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--sandbox" => set_once(
                &mut parsed.sandbox,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--max-operations" => set_once(
                &mut parsed.max_operations,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--max-loop-iterations" => set_once(
                &mut parsed.max_loop_iterations,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--max-steps" => set_once(
                &mut parsed.max_steps,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--max-memory-bytes" => set_once(
                &mut parsed.max_memory_bytes,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--max-source-bytes" => set_once(
                &mut parsed.max_source_bytes,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--max-report-bytes" => set_once(
                &mut parsed.max_report_bytes,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--max-variables" => set_once(
                &mut parsed.max_variables,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--wall-time-ms" => set_once(
                &mut parsed.wall_time_ms,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `script run` option `{other}`; see `chronofork script --help`"
                )));
            }
        }
        index += 1;
    }
    Ok(parsed)
}

/// Applies the documented CLI limit overrides to the library defaults.
///
/// Every bound, including the zero/above-maximum rejection, stays in
/// [`ScriptLimits::validate`]; this function only converts the parsed decimal and
/// maps each CLI flag to its documented field (`--max-memory-bytes` =>
/// `string_bytes`). `--wall-time-ms 0` disables the deadline by selecting `None`.
fn apply_limits(parsed: &RunOptions) -> Result<ScriptLimits, CliError> {
    let mut limits = ScriptLimits::default();
    if let Some(value) = parsed.max_operations {
        limits.operations = value;
    }
    if let Some(value) = parsed.max_loop_iterations {
        limits.loop_iterations = value;
    }
    if let Some(value) = parsed.max_steps {
        limits.steps = value;
    }
    if let Some(value) = parsed.max_memory_bytes {
        limits.string_bytes = platform_usize(value, "--max-memory-bytes")?;
    }
    if let Some(value) = parsed.max_source_bytes {
        limits.source_bytes = platform_usize(value, "--max-source-bytes")?;
    }
    if let Some(value) = parsed.max_report_bytes {
        limits.report_bytes = platform_usize(value, "--max-report-bytes")?;
    }
    if let Some(value) = parsed.max_variables {
        limits.variables = platform_usize(value, "--max-variables")?;
    }
    if let Some(value) = parsed.wall_time_ms {
        limits.wall_time_ms = (value != 0).then_some(value);
    }
    limits
        .validate()
        .map_err(|error| CliError(format!("invalid script limits: {error}")))?;
    Ok(limits)
}

fn platform_usize(value: u64, flag: &str) -> Result<usize, CliError> {
    usize::try_from(value)
        .map_err(|_| CliError(format!("`{flag}` is too large for this platform: {value}")))
}

fn script_run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let parsed = parse_run_options(args)?;
    let file = parsed
        .file
        .as_deref()
        .ok_or_else(|| CliError("`script run` requires `--file <FILE>`".to_owned()))?;
    let limits = apply_limits(&parsed)?;
    // The bounded read and the empty/oversized/file-type rejection are the CLI's
    // process boundary (M5-A32); parsing and evaluation belong to the library.
    let source = read_source(file, limits.source_bytes)?;
    let factory: Arc<dyn BackendFactory> = crate::tas::backend_factory(parsed.rom.as_deref())?;
    let mut options = ScriptOptions::new(factory).with_limits(limits);
    if let Some(root) = parsed.sandbox {
        options = options
            .with_sandbox(root)
            .map_err(|error| CliError(format!("invalid sandbox root: {error}")))?;
    }

    let run = run_source(&source, options);
    let stdout = render(&run);
    let mut handle = std::io::stdout();
    handle.write_all(stdout.as_bytes())?;
    handle.flush()?;
    if run.status().as_str() == "failed" {
        return Err(Box::new(ScriptFailed));
    }
    Ok(())
}

/// Reads one script source with the configured byte bound applied before lexing.
fn read_source(path: &Path, source_bytes: usize) -> Result<String, Box<dyn Error>> {
    let file = File::open(path).map_err(|error| {
        CliError(format!(
            "failed to open script `{}`: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        CliError(format!(
            "failed to inspect script `{}`: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(CliError(format!(
            "script path `{}` is not a regular file",
            path.display()
        ))
        .into());
    }
    if metadata.len() == 0 {
        return Err(CliError(format!("script `{}` is empty", path.display())).into());
    }
    let limit = source_bytes as u64;
    if metadata.len() > limit {
        return Err(CliError(format!(
            "script `{}` exceeds the {source_bytes}-byte source limit",
            path.display()
        ))
        .into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError(format!(
                "failed to read script `{}`: {error}",
                path.display()
            ))
        })?;
    if bytes.len() > source_bytes {
        return Err(CliError(format!(
            "script `{}` exceeds the {source_bytes}-byte source limit",
            path.display()
        ))
        .into());
    }
    if bytes.is_empty() {
        return Err(CliError(format!("script `{}` is empty", path.display())).into());
    }
    String::from_utf8(bytes)
        .map_err(|_| CliError(format!("script `{}` is not valid UTF-8", path.display())).into())
}

/// Renders the ordered print blocks and the single report block.
///
/// [`ScriptRun::report_fields`] owns the canonical field order and the
/// `-`/`message` presence rules; this function only renders each line and escapes
/// values so one field never spans two lines. No path, pid or host time is added.
fn render(run: &ScriptRun) -> String {
    let mut output = String::new();
    for line in run.prints() {
        begin_record(&mut output, "script_print");
        let _ = writeln!(output, "text={}", escape(line));
    }
    begin_record(&mut output, "script_report");
    for (key, value) in run.report_fields() {
        let _ = writeln!(output, "{key}={}", escape(&value));
    }
    output
}

/// Escapes the four documented control sequences so one value stays on one line.
fn escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other => output.push(other),
        }
    }
    output
}

fn print_help() {
    println!("chronofork {VERSION} — script");
    println!();
    println!("Run one sandboxed, deterministic script experiment and print its report.");
    println!("The language, limits and report are the M5 purpose-built runtime; no external");
    println!("engine is embedded and scripts have no network, process or hidden host access.");
    println!();
    println!("Usage:");
    println!("  chronofork script run --file <FILE> [--rom <FILE>] [--sandbox <DIR>] [limits...]");
    println!("  chronofork script --help");
    println!();
    println!("Flags:");
    println!("  --file <FILE>              Script source (required, non-empty UTF-8)");
    println!("  --rom <FILE>               Use the minimal Mega Drive slice instead of the");
    println!("                             synthetic backend");
    println!("  --sandbox <DIR>            Existing directory that enables the file builtins");
    println!("  --max-operations <N>       Statement/expression/loop operations (default 1000000)");
    println!("  --max-loop-iterations <N>  repeat/while iterations (default 1000000)");
    println!("  --max-steps <N>            Backend steps (default 1000000)");
    println!("  --max-memory-bytes <N>     Aggregate script string bytes (default 1048576)");
    println!("  --max-source-bytes <N>     Source byte bound (default 65536, max 1048576)");
    println!("  --max-report-bytes <N>     Report text bytes (default 65536, max 1048576)");
    println!("  --max-variables <N>        Script variables (default 256, max 4096)");
    println!("  --wall-time-ms <N>         Wall deadline in milliseconds (default 30000;");
    println!("                             0 disables it)");
    println!();
    println!("Every limit accepts only a positive decimal value, except `--wall-time-ms 0`.");
    println!("Without --sandbox the file builtins fail with `sandbox_unavailable`; the other");
    println!("builtins still run. The sandbox is a deterministic capability boundary, not an");
    println!("OS security boundary against hostile native code.");
    println!();
    println!("Output:");
    println!("  One `record=script_print` block per print line, in order, then exactly one");
    println!("  `record=script_report` block with status, failure, source_sha256, statistics,");
    println!("  backend and sorted `report.<key>` fields. Text values escape \\\\, \\n, \\r and");
    println!("  \\t. No path, pid or host time is printed.");
    println!();
    println!("Exit codes:");
    println!("  exit 0  the run passed");
    println!("  exit 1  the script failed (status=failed; the report is still printed and");
    println!("          stderr stays empty), or the CLI/file/usage/sandbox input was rejected");
    println!("          (empty stdout and one `chronofork: ...` stderr line)");
}
