//! `chronofork tas studio`: stdin-driven in-process rewind and stepping.
//!
//! This adapter implements the interactive half of
//! [ADR-0007](../../../docs/adr/0007-in-process-rewind-and-interactive-stepping.md):
//! it opens exactly one [`Player`], reads one command per stdin line, prints one
//! stable `key=value` block per command and keeps every diagnostic on stderr.
//!
//! The player owns the private session, the decoded movie and the bounded state
//! cache; this module only parses flags and commands, maps typed failures to the
//! process exit codes and formats the fixed blocks. It opens the movie read-only
//! with the existing size bound, writes no file, storage root or cache, and reads
//! no host clock or entropy, so valid stdout is data only. The session runs on the
//! unchanged synthetic backend by default, or on the minimal Mega Drive backend
//! when `--rom <FILE>` selects it (ADR-0009).

use std::error::Error;
use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use chronofork_core::{StateHash, Tick};
use chronofork_sessions::{
    CacheOptions, DEFAULT_CACHE_BYTE_LIMIT, DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_INTERVAL_STEPS,
    Movie, MovieError, Player, PlayerError, PlayerState, SeekProgress, SeekReport,
};

use crate::{CliError, begin_record, option_value, parse_u64, set_once};

/// Runs one `chronofork tas studio` invocation.
///
/// The flags are validated and the player is opened before the first line is
/// read, so a flag, movie or cache-configuration failure exits 1 with empty
/// stdout. One block is printed per command; `quit` and EOF print the quit block
/// and exit 0, and every typed command, target, boundary or replay failure exits
/// 1 with a stderr diagnostic and no block for that line.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        crate::ensure(
            args.len() == 1,
            "`tas studio --help` takes no other arguments",
        )?;
        crate::tas::print_help();
        return Ok(());
    }
    let options = parse_options(args)?;
    // The library's validating constructor is the single cache-bound rule; it
    // rejects every value outside the fixed ADR-0007 ranges before a session is
    // imported, so no cache structure is sized from an unchecked flag.
    let cache = CacheOptions::new(options.interval, options.capacity, options.byte_limit)?;
    let movie = Movie::from_bytes(&crate::tas::read_movie_file(&options.movie)?)?;
    let factory = crate::tas::backend_factory(options.rom.as_deref())?;
    let mut player = Player::open(factory, movie, cache)?;

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut output = Output::default();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            output.block(&quit_block("eof", player.state()))?;
            return Ok(());
        }
        let tokens = line.split_ascii_whitespace().collect::<Vec<_>>();
        let Some(command) = tokens.first().copied() else {
            continue;
        };
        match command {
            "next" => {
                require_no_arguments(&tokens, "next")?;
                let result = player.step_forward();
                emit_seek(&mut output, result)?;
            }
            "back" => {
                require_no_arguments(&tokens, "back")?;
                let result = player.step_back();
                emit_seek(&mut output, result)?;
            }
            "goto" => {
                if tokens.len() != 2 {
                    return Err(CliError(format!(
                        "`goto` expects exactly one <TICK> argument, got {} token(s)",
                        tokens.len().saturating_sub(1)
                    ))
                    .into());
                }
                let tick = parse_tick(tokens[1])?;
                let result = player.seek(tick);
                emit_seek(&mut output, result)?;
            }
            "state" => {
                require_no_arguments(&tokens, "state")?;
                output.block(&state_block(&player))?;
            }
            "help" => {
                require_no_arguments(&tokens, "help")?;
                output.block(&help_block())?;
            }
            "quit" => {
                require_no_arguments(&tokens, "quit")?;
                output.block(&quit_block("command", player.state()))?;
                return Ok(());
            }
            other => {
                return Err(CliError(format!(
                    "unknown `tas studio` command `{other}`; expected next, back, goto <TICK>, \
                     state, help or quit"
                ))
                .into());
            }
        }
    }
}

/// Parsed and platform-converted `tas studio` flags.
#[derive(Debug)]
struct Options {
    movie: PathBuf,
    interval: usize,
    capacity: usize,
    byte_limit: usize,
    rom: Option<PathBuf>,
}

/// Parses the fixed `tas studio` flag set.
///
/// Missing, duplicate, empty, non-numeric or unknown flags (including `--at`,
/// `--from`, `--until`, `--root`, `--session`, `--steps` and `--against`) and
/// positional arguments are typed CLI errors; no flag defaults to another
/// subcommand's semantics. `--rom <FILE>` selects the minimal Mega Drive backend
/// through the shared bounded ROM reader; without it the unchanged synthetic
/// factory is used.
fn parse_options(args: &[String]) -> Result<Options, CliError> {
    let mut movie: Option<PathBuf> = None;
    let mut interval: Option<u64> = None;
    let mut capacity: Option<u64> = None;
    let mut byte_limit: Option<u64> = None;
    let mut rom: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--movie" => set_once(
                &mut movie,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--rom" => set_once(
                &mut rom,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--cache-interval" => set_once(
                &mut interval,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--cache-capacity" => set_once(
                &mut capacity,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--cache-bytes" => set_once(
                &mut byte_limit,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `tas studio` option `{other}`; expected --movie <FILE> \
                     --rom <FILE> --cache-interval <N> --cache-capacity <N> --cache-bytes <N>"
                )));
            }
        }
        index += 1;
    }
    let movie =
        movie.ok_or_else(|| CliError("`tas studio` requires `--movie <FILE>`".to_owned()))?;
    Ok(Options {
        movie,
        interval: platform_usize(
            interval.unwrap_or(DEFAULT_CACHE_INTERVAL_STEPS as u64),
            "--cache-interval",
        )?,
        capacity: platform_usize(
            capacity.unwrap_or(DEFAULT_CACHE_CAPACITY as u64),
            "--cache-capacity",
        )?,
        byte_limit: platform_usize(
            byte_limit.unwrap_or(DEFAULT_CACHE_BYTE_LIMIT as u64),
            "--cache-bytes",
        )?,
        rom,
    })
}

/// Converts one decimal flag to `usize` without truncation.
///
/// The fixed maxima all fit a 32-bit `usize`; this check only keeps a
/// platform-narrowing conversion from wrapping or panicking.
fn platform_usize(value: u64, flag: &str) -> Result<usize, CliError> {
    usize::try_from(value)
        .map_err(|_| CliError(format!("`{flag}` is too large for this platform: {value}")))
}

/// Rejects arguments after a command that takes none.
fn require_no_arguments(tokens: &[&str], command: &str) -> Result<(), CliError> {
    if tokens.len() != 1 {
        return Err(CliError(format!("`{command}` takes no arguments")));
    }
    Ok(())
}

/// Parses one decimal `goto` target tick.
fn parse_tick(token: &str) -> Result<Tick, CliError> {
    token
        .parse::<u64>()
        .map(Tick::from_raw)
        .map_err(|_| CliError(format!("`goto` expects a decimal tick, got `{token}`")))
}

/// Prints one fully formatted seek block, or reports the typed failure.
///
/// A state-hash divergence is the only failure that writes stdout: it prints the
/// fixed divergence block and then returns the stderr diagnostic. A recorded-tick
/// mismatch, replay failure, target/boundary rejection or cache failure stays
/// stderr-only and prints no block for that line.
fn emit_seek(
    output: &mut Output,
    result: Result<SeekReport, PlayerError>,
) -> Result<(), Box<dyn Error>> {
    match result {
        Ok(report) => {
            output.block(&seek_block(&report))?;
            Ok(())
        }
        Err(PlayerError::Movie { source, progress }) => match (*source, progress) {
            (
                MovieError::Divergence {
                    tick,
                    input_index,
                    expected,
                    observed,
                },
                Some(progress),
            ) => {
                output.block(&divergence_block(
                    &progress,
                    tick,
                    input_index,
                    expected,
                    observed,
                ))?;
                Err(CliError(format!(
                    "state divergence at tick {} (input index {input_index}): expected \
                     {expected}, observed {observed}",
                    tick.raw()
                ))
                .into())
            }
            (other, progress) => Err(PlayerError::Movie {
                source: Box::new(other),
                progress,
            }
            .into()),
        },
        Err(error) => Err(error.into()),
    }
}

/// Formats the `record=seek` block in the fixed ADR-0007 field order.
fn seek_block(report: &SeekReport) -> String {
    let mut output = String::new();
    begin_record(&mut output, "seek");
    let _ = writeln!(output, "tick={}", report.tick.raw());
    let _ = writeln!(output, "applied_steps={}", report.applied_steps);
    let _ = writeln!(output, "target_tick={}", report.target_tick.raw());
    let _ = writeln!(output, "start={}", report.start);
    let _ = writeln!(output, "start_tick={}", report.start_tick.raw());
    if let Some(step) = report.start_step {
        let _ = writeln!(output, "start_step={step}");
    }
    let _ = writeln!(output, "replayed_steps={}", report.replayed_steps);
    let _ = writeln!(output, "state_hash={}", report.state_hash);
    let _ = writeln!(output, "cache_entries={}", report.cache.entries);
    let _ = writeln!(output, "cache_bytes={}", report.cache.bytes);
    let _ = writeln!(output, "cache_evictions={}", report.cache.evictions);
    output
}

/// Formats the `record=state` block: current position plus configured bounds.
fn state_block(player: &Player) -> String {
    let state = player.state();
    let movie = player.movie();
    let cache = player.cache_stats();
    let mut output = String::new();
    begin_record(&mut output, "state");
    let _ = writeln!(output, "tick={}", state.tick.raw());
    let _ = writeln!(output, "applied_steps={}", state.applied_steps);
    let _ = writeln!(output, "state_hash={}", state.state_hash);
    let _ = writeln!(output, "movie_version={}", movie.format_version());
    let _ = writeln!(output, "input_schema={}", movie.input_schema_version());
    let _ = writeln!(output, "start_tick={}", movie.start_tick().raw());
    let _ = writeln!(output, "end_tick={}", movie.end_tick().raw());
    let _ = writeln!(output, "steps={}", movie.step_count());
    let _ = writeln!(output, "cache_interval={}", cache.interval);
    let _ = writeln!(output, "cache_capacity={}", cache.capacity);
    let _ = writeln!(output, "cache_byte_limit={}", cache.byte_limit);
    let _ = writeln!(output, "cache_entries={}", cache.entries);
    let _ = writeln!(output, "cache_bytes={}", cache.bytes);
    let _ = writeln!(output, "cache_evictions={}", cache.evictions);
    output
}

/// Formats the fixed `record=help` block.
fn help_block() -> String {
    let mut output = String::new();
    begin_record(&mut output, "help");
    for command in ["next", "back", "goto <TICK>", "state", "help", "quit"] {
        let _ = writeln!(output, "command={command}");
    }
    output
}

/// Formats the `record=quit` block for a command or EOF exit.
fn quit_block(reason: &str, state: PlayerState) -> String {
    let mut output = String::new();
    begin_record(&mut output, "quit");
    let _ = writeln!(output, "reason={reason}");
    let _ = writeln!(output, "tick={}", state.tick.raw());
    let _ = writeln!(output, "applied_steps={}", state.applied_steps);
    output
}

/// Formats the `record=divergence` block for the exact first mismatch.
fn divergence_block(
    progress: &SeekProgress,
    tick: Tick,
    input_index: usize,
    expected: StateHash,
    observed: StateHash,
) -> String {
    let mut output = String::new();
    begin_record(&mut output, "divergence");
    let _ = writeln!(output, "diverged=1");
    let _ = writeln!(output, "tick={}", tick.raw());
    let _ = writeln!(output, "step={input_index}");
    let _ = writeln!(output, "expected={expected}");
    let _ = writeln!(output, "observed={observed}");
    let _ = writeln!(output, "start={}", progress.start);
    let _ = writeln!(output, "start_tick={}", progress.start_tick.raw());
    let _ = writeln!(output, "replayed_steps={}", progress.replayed_steps);
    let _ = writeln!(output, "cache_entries={}", progress.cache_entries);
    let _ = writeln!(output, "cache_bytes={}", progress.cache_bytes);
    output
}

/// Writes fully formatted blocks to stdout, separating them by one blank line.
///
/// A block is built completely before it is written, so a typed failure never
/// leaves a partial block. The `started` flag keeps the first block flush against
/// the start of the stream and puts exactly one blank line between later blocks,
/// matching the existing `begin_record` block convention.
#[derive(Debug, Default)]
struct Output {
    started: bool,
}

impl Output {
    fn block(&mut self, block: &str) -> Result<(), Box<dyn Error>> {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        if self.started {
            stdout.write_all(b"\n")?;
        }
        stdout.write_all(block.as_bytes())?;
        stdout.flush()?;
        self.started = true;
        Ok(())
    }
}
