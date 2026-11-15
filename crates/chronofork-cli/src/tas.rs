//! `chronofork tas` process boundary: record, play, verify and compare.
//!
//! This module is a thin, line-oriented adapter over [`chronofork_sessions::Movie`]
//! and the session replay/comparison API: it parses flags, reads or writes one
//! movie file with an explicit size bound, and prints `key=value` blocks.
//! Encoding, decoding, checkpoint import, replay, range validation and branch
//! comparison stay in `chronofork-sessions`; this code never re-implements the
//! movie codec, a range predicate or a comparison predicate.

use std::error::Error;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use chronofork_core::{StateHash, Tick, hash_bytes};
use chronofork_sessions::{
    ComparisonReport, ComparisonVerdict, LoadedSnapshot, MAX_MOVIE_BYTES, MAX_NAME_BYTES,
    MAX_STEPS, MOVIE_FORMAT_VERSION, MOVIE_INPUT_SCHEMA_VERSION, Movie, MovieError, MovieMeta,
    ReplayOutcome, ReplayRange, SnapshotRepository, compare_movies,
};

use crate::{
    CliError, VERSION, begin_record, ensure, input_frame, option_value, parse_key, parse_u32,
    parse_u64, set_once, synthetic_session_manager,
};

/// Runs one `chronofork tas` subcommand.
///
/// The TAS commands are a thin, line-oriented process boundary over
/// [`chronofork_sessions::Movie`]: they parse flags, read or write one movie file
/// with an explicit size bound, and print `key=value` blocks. Encoding, decoding,
/// checkpoint import, replay range validation, branch comparison and
/// first-divergence reporting stay in `chronofork-sessions`; this code never
/// re-implements them.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("record") => tas_record(&args[1..]),
        Some("play") => tas_play(&args[1..]),
        Some("verify") => tas_verify(&args[1..]),
        Some("compare") => tas_compare(&args[1..]),
        Some("--help" | "-h") => {
            ensure(args.len() == 1, "`tas --help` takes no other arguments")?;
            print_help();
            Ok(())
        }
        None => Err(CliError(
            "`tas` requires a subcommand: record, play, verify or compare".to_owned(),
        )
        .into()),
        Some(other) => Err(CliError(format!(
            "unknown `tas` subcommand `{other}`; expected record, play, verify or compare"
        ))
        .into()),
    }
}

/// Returns whether the arguments request subcommand help.
///
/// Help is accepted only as the sole argument, matching the existing CLI shape.
fn help_requested(args: &[String], command: &str) -> Result<bool, CliError> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        if args.len() != 1 {
            return Err(CliError(format!(
                "`{command} --help` takes no other arguments"
            )));
        }
        return Ok(true);
    }
    Ok(false)
}

/// Parsed `tas record` flags, before required-flag and combination validation.
#[derive(Debug, Default)]
struct RecordOptions {
    movie: Option<PathBuf>,
    movie_id: Option<u64>,
    name: Option<String>,
    steps: Option<u64>,
    input: Option<u32>,
    root: Option<PathBuf>,
    session: Option<u64>,
    snapshot: Option<u64>,
}

fn parse_record_options(args: &[String]) -> Result<RecordOptions, CliError> {
    let mut parsed = RecordOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--movie" => set_once(
                &mut parsed.movie,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--movie-id" => set_once(
                &mut parsed.movie_id,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--name" => set_once(
                &mut parsed.name,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--steps" => set_once(
                &mut parsed.steps,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--input" => set_once(
                &mut parsed.input,
                parse_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--root" => set_once(
                &mut parsed.root,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--session" => set_once(
                &mut parsed.session,
                parse_key(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--snapshot" => set_once(
                &mut parsed.snapshot,
                parse_key(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `tas record` option `{other}`; expected --movie <FILE> \
                     --steps <N> --input <U32> --movie-id <U64> --name <NAME> \
                     --root <DIR> --session <KEY> --snapshot <KEY>"
                )));
            }
        }
        index += 1;
    }
    Ok(parsed)
}

fn tas_record(args: &[String]) -> Result<(), Box<dyn Error>> {
    if help_requested(args, "tas record")? {
        print_help();
        return Ok(());
    }
    let options = parse_record_options(args)?;
    let movie_path = options
        .movie
        .ok_or_else(|| CliError("`tas record` requires `--movie <FILE>`".to_owned()))?;
    if options.session.is_some() && options.root.is_none() {
        return Err(
            CliError("`--session` requires `--root <DIR>` in `tas record`".to_owned()).into(),
        );
    }
    if options.snapshot.is_some() && options.session.is_none() {
        return Err(
            CliError("`--snapshot` requires `--session <KEY>` in `tas record`".to_owned()).into(),
        );
    }
    let steps = options.steps.unwrap_or(0);
    if steps > MAX_STEPS as u64 {
        return Err(CliError(format!(
            "`--steps` must be at most {MAX_STEPS} (the movie step limit), got {steps}"
        ))
        .into());
    }
    let steps = usize::try_from(steps)
        .map_err(|_| CliError(format!("`--steps` is too large for this platform: {steps}")))?;
    // Validate metadata before doing any execution so an invalid movie id or name
    // fails without recording and without touching the output path.
    let meta = MovieMeta::new(options.movie_id.unwrap_or(1), options.name, None)?;
    let inputs = vec![input_frame(options.input); steps];

    let mut manager = synthetic_session_manager();
    let source = match options.root {
        Some(root) => {
            // A record never initializes or writes storage: open an existing root,
            // load the selected committed checkpoint and replay in memory only.
            let repository = SnapshotRepository::open(root)?;
            let loaded = select_checkpoint(&repository, options.session, options.snapshot)?;
            manager
                .create_session_from_stored_snapshot(loaded.snapshot, loaded.entry.session_key)?
        }
        None => manager.create_session()?,
    };
    let movie = Movie::record(&mut manager, source, meta, &inputs)?;
    let bytes = movie.to_bytes()?;
    write_movie_file(&movie_path, &bytes)?;

    let mut output = String::new();
    write_movie_block(&mut output, &movie, &bytes);
    print!("{output}");
    Ok(())
}

fn tas_play(args: &[String]) -> Result<(), Box<dyn Error>> {
    if help_requested(args, "tas play")? {
        print_help();
        return Ok(());
    }
    let options = parse_replay_options("play", args)?;
    replay_movie("playback", false, &options)
}

fn tas_verify(args: &[String]) -> Result<(), Box<dyn Error>> {
    if help_requested(args, "tas verify")? {
        print_help();
        return Ok(());
    }
    let options = parse_replay_options("verify", args)?;
    replay_movie("verification", true, &options)
}

/// Parsed `tas play`/`tas verify` flags.
///
/// Omitted boundaries default in the library to the movie's start tick and end
/// tick. Ticks are parsed as unsigned decimals here and validated against the
/// decoded movie by [`ReplayRange::new`], never by this adapter.
#[derive(Debug)]
struct ReplayOptions {
    movie: PathBuf,
    from: Option<Tick>,
    until: Option<Tick>,
}

fn parse_replay_options(
    subcommand: &str,
    args: &[String],
) -> Result<ReplayOptions, Box<dyn Error>> {
    let mut movie: Option<PathBuf> = None;
    let mut from: Option<u64> = None;
    let mut until: Option<u64> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--movie" => set_once(
                &mut movie,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--from" => set_once(
                &mut from,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--until" => set_once(
                &mut until,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `tas {subcommand}` option `{other}`; expected --movie <FILE> \
                     --from <TICK> --until <TICK>"
                ))
                .into());
            }
        }
        index += 1;
    }
    let Some(movie) = movie else {
        return Err(CliError(format!("`tas {subcommand}` requires `--movie <FILE>`")).into());
    };
    Ok(ReplayOptions {
        movie,
        from: from.map(Tick::from_raw),
        until: until.map(Tick::from_raw),
    })
}

/// Selects the checkpoint a `record --root` reads.
///
/// With `--session`, the selected snapshot or that session's latest committed
/// snapshot is used. Without `--session`, the checkpoint with the greatest
/// `(creation_sequence, snapshot_key, session_key)` across all durable sessions is
/// used; the tie-break keeps the choice deterministic. Nothing is written.
fn select_checkpoint(
    repository: &SnapshotRepository,
    session: Option<u64>,
    snapshot: Option<u64>,
) -> Result<LoadedSnapshot, Box<dyn Error>> {
    let (session_key, snapshot_key) = match session {
        Some(session_key) => {
            let snapshot_key = match snapshot {
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
            (session_key, snapshot_key)
        }
        None => latest_committed_checkpoint(repository)?,
    };
    Ok(repository.load_snapshot(session_key, snapshot_key)?)
}

fn latest_committed_checkpoint(
    repository: &SnapshotRepository,
) -> Result<(u64, u64), Box<dyn Error>> {
    let sessions = repository.list_sessions()?;
    let mut best: Option<(u64, u64, u64)> = None;
    for session in &sessions.entries {
        let Some(snapshot) = repository.latest_snapshot(session.session_key)? else {
            continue;
        };
        let candidate = (
            snapshot.creation_sequence,
            snapshot.snapshot_key,
            session.session_key,
        );
        if best.is_none_or(|current| candidate > current) {
            best = Some(candidate);
        }
    }
    match best {
        Some((_, snapshot_key, session_key)) => Ok((session_key, snapshot_key)),
        None => Err(CliError(
            "storage root has no committed checkpoint; save one with `chronofork session save`"
                .to_owned(),
        )
        .into()),
    }
}

/// Replays and verifies one movie over the requested tick window.
///
/// `play` and `verify` share this single replay implementation; only the record
/// name and the `verified=1` field differ. Without range flags the output is the
/// unchanged M3-A7 summary block over the full range. With at least one range flag
/// the movie-level block is extended by `range_from`, `range_until`,
/// `prefix_steps`, `window_steps` and `replayed_steps`; `final_tick` and
/// `final_state_hash` are the observed values at `range_until`.
///
/// Every invocation imports the embedded checkpoint into a fresh in-process
/// session, so a rewind is a new invocation and no state is carried between
/// process runs. Range validation is owned by [`ReplayRange::new`] and runs before
/// the checkpoint import; an invalid range exits 1 with a stderr diagnostic and
/// no stdout block. A state-hash divergence prints the record block plus the exact
/// divergence fields and then exits 1; a tick mismatch remains a stderr-only
/// diagnostic.
fn replay_movie(
    record: &str,
    verified: bool,
    options: &ReplayOptions,
) -> Result<(), Box<dyn Error>> {
    let bytes = read_movie_file(&options.movie)?;
    let movie = Movie::from_bytes(&bytes)?;
    let ranged = options.from.is_some() || options.until.is_some();
    let range = if ranged {
        ReplayRange::new(&movie, options.from, options.until)?
    } else {
        ReplayRange::full(&movie)
    };
    let mut manager = synthetic_session_manager();
    match range.replay(&mut manager) {
        Ok(outcome) => {
            let mut output = String::new();
            if ranged {
                write_ranged_replay_block(&mut output, record, verified, &outcome);
            } else {
                write_replay_block(&mut output, record, verified, &outcome);
            }
            print!("{output}");
            Ok(())
        }
        Err(MovieError::Divergence {
            tick,
            input_index,
            expected,
            observed,
        }) => {
            let mut output = String::new();
            write_divergence_block(
                &mut output,
                record,
                tick.raw(),
                input_index,
                expected,
                observed,
            );
            print!("{output}");
            Err(CliError(format!(
                "state divergence at tick {} (input index {input_index}): \
                 expected {expected}, observed {observed}",
                tick.raw()
            ))
            .into())
        }
        Err(other) => Err(other.into()),
    }
}

/// Parsed `tas compare` flags.
#[derive(Debug, Default)]
struct CompareOptions {
    movie: Option<PathBuf>,
    against: Option<PathBuf>,
}

fn parse_compare_options(args: &[String]) -> Result<CompareOptions, CliError> {
    let mut parsed = CompareOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--movie" => set_once(
                &mut parsed.movie,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--against" => set_once(
                &mut parsed.against,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `tas compare` option `{other}`; expected --movie <FILE> --against <FILE>"
                )));
            }
        }
        index += 1;
    }
    Ok(parsed)
}

/// Compares two decoded movies' recorded branches without re-simulating.
///
/// The comparison is a pure value operation over recorded step records: it never
/// imports a checkpoint or executes a backend step. Metadata changes never change
/// the verdict. Exit 0 means the recorded branches are identical; a checkpoint
/// (start) mismatch, the first recorded `(outcome tick, state hash)` divergence or
/// a step-count mismatch prints its one stable block and exits 1, with the exact
/// values repeated in a stderr diagnostic.
fn tas_compare(args: &[String]) -> Result<(), Box<dyn Error>> {
    if help_requested(args, "tas compare")? {
        print_help();
        return Ok(());
    }
    let options = parse_compare_options(args)?;
    let movie_path = options
        .movie
        .ok_or_else(|| CliError("`tas compare` requires `--movie <FILE>`".to_owned()))?;
    let against_path = options
        .against
        .ok_or_else(|| CliError("`tas compare` requires `--against <FILE>`".to_owned()))?;
    let movie = Movie::from_bytes(&read_movie_file(&movie_path)?)?;
    let against = Movie::from_bytes(&read_movie_file(&against_path)?)?;
    let report = compare_movies(&movie, &against);
    let mut output = String::new();
    match &report.verdict {
        ComparisonVerdict::Identical => {
            write_comparison_identical_block(&mut output, &report, &movie);
            print!("{output}");
            Ok(())
        }
        ComparisonVerdict::CheckpointMismatch => {
            write_comparison_start_mismatch_block(&mut output, &report);
            print!("{output}");
            Err(CliError(format!(
                "recorded branch start mismatch: start_tick_a={} start_tick_b={} \
                 checkpoint_state_hash_a={} checkpoint_state_hash_b={}",
                report.start_tick_a,
                report.start_tick_b,
                report.checkpoint_state_hash_a,
                report.checkpoint_state_hash_b
            ))
            .into())
        }
        ComparisonVerdict::StepDivergence {
            index,
            tick_a,
            tick_b,
            state_hash_a,
            state_hash_b,
        } => {
            write_comparison_divergence_block(
                &mut output,
                &report,
                *index,
                *tick_a,
                *tick_b,
                *state_hash_a,
                *state_hash_b,
            );
            print!("{output}");
            Err(CliError(format!(
                "recorded branch divergence at index {index}: tick_a={tick_a} tick_b={tick_b} \
                 state_hash_a={state_hash_a} state_hash_b={state_hash_b}"
            ))
            .into())
        }
        ComparisonVerdict::LengthMismatch { common_steps } => {
            write_comparison_length_mismatch_block(&mut output, &report, *common_steps);
            print!("{output}");
            Err(CliError(format!(
                "recorded branch length mismatch: common_steps={common_steps} \
                 steps_a={} steps_b={}",
                report.steps_a, report.steps_b
            ))
            .into())
        }
    }
}

/// Reads one movie file with an explicit size bound applied before decoding.
fn read_movie_file(path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
    let file = File::open(path).map_err(|error| {
        CliError(format!(
            "failed to open movie `{}`: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        CliError(format!(
            "failed to inspect movie `{}`: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(CliError(format!(
            "movie path `{}` is not a regular file",
            path.display()
        ))
        .into());
    }
    let limit = MAX_MOVIE_BYTES as u64;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError(format!(
                "failed to read movie `{}`: {error}",
                path.display()
            ))
        })?;
    if bytes.len() > MAX_MOVIE_BYTES {
        return Err(CliError(format!(
            "movie `{}` exceeds the {MAX_MOVIE_BYTES}-byte limit",
            path.display()
        ))
        .into());
    }
    Ok(bytes)
}

/// Publishes movie bytes through a sibling temporary file renamed on success.
///
/// The complete movie is written before the rename, so a failed record never
/// replaces an existing movie file. The temporary file name contains only the
/// process id; no host timestamp enters the movie bytes or stdout.
fn write_movie_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let file_name = path.file_name().ok_or_else(|| {
        CliError(format!(
            "`--movie` must name a file, not a directory: `{}`",
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
            "failed to write movie `{}`: {error}",
            temporary.display()
        ))
    })?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(CliError(format!(
            "failed to publish movie `{}`: {error}",
            path.display()
        ))
        .into());
    }
    Ok(())
}

fn write_movie_block(output: &mut String, movie: &Movie, bytes: &[u8]) {
    begin_record(output, "movie");
    let _ = writeln!(output, "movie_version={MOVIE_FORMAT_VERSION}");
    let _ = writeln!(output, "input_schema={MOVIE_INPUT_SCHEMA_VERSION}");
    let _ = writeln!(output, "movie_id={}", movie.meta().movie_id());
    let _ = writeln!(output, "name={}", movie.meta().name().unwrap_or(""));
    let _ = writeln!(output, "start_tick={}", movie.start_tick().raw());
    let _ = writeln!(output, "end_tick={}", movie.end_tick().raw());
    let _ = writeln!(output, "steps={}", movie.step_count());
    let _ = writeln!(output, "final_state_hash={}", movie.final_state_hash());
    let _ = writeln!(output, "movie_sha256={}", hash_bytes(bytes));
}

fn write_replay_block(output: &mut String, record: &str, verified: bool, outcome: &ReplayOutcome) {
    begin_record(output, record);
    if verified {
        let _ = writeln!(output, "verified=1");
    }
    let _ = writeln!(output, "start_tick={}", outcome.start_tick.raw());
    let _ = writeln!(output, "end_tick={}", outcome.end_tick.raw());
    let _ = writeln!(output, "steps={}", outcome.total_steps);
    let _ = writeln!(output, "final_tick={}", outcome.final_tick.raw());
    let _ = writeln!(output, "final_state_hash={}", outcome.final_state_hash);
}

/// Writes the ranged summary block: the unchanged summary fields plus the
/// validated window, prefix and work counts from the library outcome.
fn write_ranged_replay_block(
    output: &mut String,
    record: &str,
    verified: bool,
    outcome: &ReplayOutcome,
) {
    write_replay_block(output, record, verified, outcome);
    let _ = writeln!(output, "range_from={}", outcome.range_from.raw());
    let _ = writeln!(output, "range_until={}", outcome.range_until.raw());
    let _ = writeln!(output, "prefix_steps={}", outcome.prefix_steps);
    let _ = writeln!(output, "window_steps={}", outcome.window_steps);
    let _ = writeln!(output, "replayed_steps={}", outcome.replayed_steps);
}

fn write_comparison_identical_block(output: &mut String, report: &ComparisonReport, movie: &Movie) {
    begin_record(output, "comparison");
    let _ = writeln!(output, "identical=1");
    let _ = writeln!(output, "start_tick={}", report.start_tick_a.raw());
    let _ = writeln!(output, "steps={}", report.steps_a);
    let _ = writeln!(output, "final_state_hash={}", movie.final_state_hash());
}

fn write_comparison_start_mismatch_block(output: &mut String, report: &ComparisonReport) {
    begin_record(output, "comparison");
    let _ = writeln!(output, "identical=0");
    let _ = writeln!(output, "checkpoint_mismatch=1");
    let _ = writeln!(output, "start_tick_a={}", report.start_tick_a.raw());
    let _ = writeln!(output, "start_tick_b={}", report.start_tick_b.raw());
    let _ = writeln!(
        output,
        "checkpoint_state_hash_a={}",
        report.checkpoint_state_hash_a
    );
    let _ = writeln!(
        output,
        "checkpoint_state_hash_b={}",
        report.checkpoint_state_hash_b
    );
    let _ = writeln!(output, "steps_a={}", report.steps_a);
    let _ = writeln!(output, "steps_b={}", report.steps_b);
}

fn write_comparison_divergence_block(
    output: &mut String,
    report: &ComparisonReport,
    index: usize,
    tick_a: Tick,
    tick_b: Tick,
    state_hash_a: StateHash,
    state_hash_b: StateHash,
) {
    begin_record(output, "comparison");
    let _ = writeln!(output, "identical=0");
    let _ = writeln!(output, "diverged=1");
    let _ = writeln!(output, "divergence_index={index}");
    let _ = writeln!(output, "tick_a={}", tick_a.raw());
    let _ = writeln!(output, "tick_b={}", tick_b.raw());
    let _ = writeln!(output, "state_hash_a={state_hash_a}");
    let _ = writeln!(output, "state_hash_b={state_hash_b}");
    let _ = writeln!(output, "steps_a={}", report.steps_a);
    let _ = writeln!(output, "steps_b={}", report.steps_b);
}

fn write_comparison_length_mismatch_block(
    output: &mut String,
    report: &ComparisonReport,
    common_steps: usize,
) {
    begin_record(output, "comparison");
    let _ = writeln!(output, "identical=0");
    let _ = writeln!(output, "length_mismatch=1");
    let _ = writeln!(output, "common_steps={common_steps}");
    let _ = writeln!(output, "steps_a={}", report.steps_a);
    let _ = writeln!(output, "steps_b={}", report.steps_b);
}

fn write_divergence_block(
    output: &mut String,
    record: &str,
    tick: u64,
    input_index: usize,
    expected: StateHash,
    observed: StateHash,
) {
    begin_record(output, record);
    let _ = writeln!(output, "diverged=1");
    let _ = writeln!(output, "tick={tick}");
    let _ = writeln!(output, "input_index={input_index}");
    let _ = writeln!(output, "expected={expected}");
    let _ = writeln!(output, "observed={observed}");
}

fn print_help() {
    println!("chronofork {VERSION} — tas record|play|verify|compare");
    println!();
    println!("Records, replays, verifies and compares canonical `.cfmovie` input sequences on");
    println!("the deterministic synthetic test backend. The movie embeds its canonical");
    println!("checkpoint, so replay needs only the movie file. Partial replay and seek are");
    println!("deterministic re-simulation from that embedded checkpoint, not O(1) random");
    println!("access; a rewind is a new invocation and no state is carried between");
    println!("invocations. Checkpoint anchors and a movie format v2, an in-process rewind");
    println!("buffer and interactive stepping, rerecord and branch editing,");
    println!("multi-controller/ports/analog/keyboard input, real-hardware input TAS and TAS");
    println!("import/export are not implemented. HTTP, MCP, GUI and a daemon are absent.");
    println!();
    println!("Usage:");
    println!("  chronofork tas record --movie <FILE> [--steps <N>] [--input <U32>]");
    println!("                        [--movie-id <U64>] [--name <NAME>]");
    println!("                        [--root <DIR> [--session <KEY> [--snapshot <KEY>]]]");
    println!("  chronofork tas play --movie <FILE> [--from <TICK>] [--until <TICK>]");
    println!("  chronofork tas verify --movie <FILE> [--from <TICK>] [--until <TICK>]");
    println!("  chronofork tas compare --movie <FILE> --against <FILE>");
    println!();
    println!("record:");
    println!("  --movie <FILE>     Required output movie file, published atomically on success");
    println!("  --steps <N>        Inputs to record, decimal (default 0, max {MAX_STEPS})");
    println!("  --input <U32>      Fixed backend input mask applied every step (default 0)");
    println!("  --movie-id <U64>   Non-zero logical movie id (default 1)");
    println!(
        "  --name <NAME>      Optional label, at most {MAX_NAME_BYTES} UTF-8 bytes, no control characters"
    );
    println!(
        "  --root <DIR>       Seed from an existing M2 storage root (never initialized or written)"
    );
    println!(
        "  --session <KEY>    Durable session to seed from (default: latest committed checkpoint)"
    );
    println!("  --snapshot <KEY>   Durable snapshot to seed from (requires --session)");
    println!();
    println!("play/verify:");
    println!("  --movie <FILE>     Required input movie file");
    println!("  --from <TICK>      Exclusive prefix boundary: recorded steps with outcome tick");
    println!("                     <= --from are executed and verified as the prefix (default:");
    println!("                     the movie start tick)");
    println!("  --until <TICK>     Inclusive window end: recorded steps after --from and up to");
    println!("                     --until are reported (default: the movie end tick).");
    println!("                     Both boundaries must be the start tick, the end tick or a");
    println!("                     recorded outcome tick; an invalid range exits 1 before any");
    println!("                     checkpoint is imported.");
    println!();
    println!("compare:");
    println!("  --movie <FILE>     First recorded branch file");
    println!("  --against <FILE>   Second recorded branch file");
    println!();
    println!("Output is one or more `key=value` blocks on stdout; diagnostics go to stderr");
    println!("as `chronofork: <error>`. Exit code is 0 on success and 1 on any typed failure.");
    println!("play/verify without range flags print the single summary block (`record`,");
    println!("`start_tick`, `end_tick`, `steps`, `final_tick`, `final_state_hash`). With");
    println!("--from and/or --until they add `range_from`, `range_until`, `prefix_steps`,");
    println!("`window_steps` and `replayed_steps`; the window is (from, until], `final_tick`");
    println!("is `range_until` and `final_state_hash` is the observed hash there.");
    println!("On a state-hash divergence, play/verify print the matching record block plus");
    println!("`diverged=1`, `tick`, `input_index`, `expected` and `observed`, then exit 1.");
    println!("A recorded-tick mismatch is reported on stderr without a divergence block.");
    println!();
    println!("compare prints exactly one `record=comparison` block. `identical=1` also");
    println!("prints `start_tick`, `steps` and `final_state_hash` and exits 0. A mismatch");
    println!("prints `identical=0` plus `checkpoint_mismatch=1` (with both start ticks and");
    println!("checkpoint state hashes), `diverged=1` (with `divergence_index`, `tick_a`,");
    println!("`tick_b`, `state_hash_a`, `state_hash_b`) or `length_mismatch=1` (with");
    println!("`common_steps`), always with both step counts, and exits 1. Comparison reads");
    println!("recorded expectations only and never re-simulates; movie metadata and input");
    println!("masks never decide equality.");
}
