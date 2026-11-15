//! Process-level acceptance tests for `chronofork tas studio` (ADR-0007 §6).
//!
//! Every invocation spawns the built `chronofork` binary as a separate operating
//! system process, feeds a script over stdin and checks the command line, exit
//! code and the two output streams. The tests cover the fixed `key=value` blocks,
//! forward/backward/goto stepping, cache and floor boundaries, EOF/quit, invalid
//! input, divergence, v1/v2 movies, determinism and read-only behavior.
//!
//! The tests return `Result` instead of panicking helpers so the workspace
//! `expect_used` lint is satisfied without adding a file-level allowance.

use std::error::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chronofork_core::{StateHash, Tick, hash_bytes};
use chronofork_sessions::{Movie, MovieStep};

type TestResult = Result<(), Box<dyn Error>>;
/// One parsed `key=value` block, in output order (duplicate keys are kept).
type Block = Vec<(String, String)>;

const STATE_KEYS: &[&str] = &[
    "record",
    "tick",
    "applied_steps",
    "state_hash",
    "movie_version",
    "input_schema",
    "start_tick",
    "end_tick",
    "steps",
    "cache_interval",
    "cache_capacity",
    "cache_byte_limit",
    "cache_entries",
    "cache_bytes",
    "cache_evictions",
];

const SEEK_KEYS: &[&str] = &[
    "record",
    "tick",
    "applied_steps",
    "target_tick",
    "start",
    "start_tick",
    "replayed_steps",
    "state_hash",
    "cache_entries",
    "cache_bytes",
    "cache_evictions",
];

const SEEK_KEYS_CACHED: &[&str] = &[
    "record",
    "tick",
    "applied_steps",
    "target_tick",
    "start",
    "start_tick",
    "start_step",
    "replayed_steps",
    "state_hash",
    "cache_entries",
    "cache_bytes",
    "cache_evictions",
];

const QUIT_KEYS: &[&str] = &["record", "reason", "tick", "applied_steps"];

const DIVERGENCE_KEYS: &[&str] = &[
    "record",
    "diverged",
    "tick",
    "step",
    "expected",
    "observed",
    "start",
    "start_tick",
    "replayed_steps",
    "cache_entries",
    "cache_bytes",
];

const HELP_KEYS: &[&str] = &[
    "record", "command", "command", "command", "command", "command", "command",
];

fn temp_dir(name: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "chronofork-cli-studio-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn path_str(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str()
        .ok_or_else(|| format!("temporary path is not UTF-8: {}", path.display()).into())
}

fn list_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// One captured process result.
#[derive(Debug)]
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs the CLI with `stdin_text` as the child's complete standard input.
fn run(args: &[&str], stdin_text: &str, cwd: Option<&Path>) -> Result<Run, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_chronofork"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn()?;
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(stdin_text.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    Ok(Run {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn studio_args<'a>(movie: &'a Path, extra: &[&'a str]) -> Result<Vec<&'a str>, Box<dyn Error>> {
    let mut args = vec!["tas", "studio", "--movie", path_str(movie)?];
    args.extend_from_slice(extra);
    Ok(args)
}

/// Requires a successful studio run: exit 0, empty stderr, stdout data.
fn studio_ok(movie: &Path, extra: &[&str], stdin_text: &str) -> Result<String, Box<dyn Error>> {
    let args = studio_args(movie, extra)?;
    let outcome = run(&args, stdin_text, None)?;
    if outcome.code != Some(0) {
        return Err(format!(
            "studio must exit 0 for script {stdin_text:?}\nstdout:\n{}\nstderr:\n{}",
            outcome.stdout, outcome.stderr
        )
        .into());
    }
    if !outcome.stderr.is_empty() {
        return Err(format!(
            "successful studio must keep stderr empty:\n{}",
            outcome.stderr
        )
        .into());
    }
    Ok(outcome.stdout)
}

/// Requires a typed studio failure and returns the captured streams.
fn studio_failure(
    movie: &Path,
    extra: &[&str],
    stdin_text: &str,
    needle: &str,
) -> Result<Run, Box<dyn Error>> {
    let args = studio_args(movie, extra)?;
    let outcome = run(&args, stdin_text, None)?;
    expect_failure(
        &outcome,
        &format!("{args:?} with stdin {stdin_text:?}"),
        needle,
    )?;
    Ok(outcome)
}

fn expect_failure(outcome: &Run, label: &str, needle: &str) -> TestResult {
    if outcome.code != Some(1) {
        return Err(format!(
            "expected exit 1 for {label}, got {:?}\nstdout:\n{}\nstderr:\n{}",
            outcome.code, outcome.stdout, outcome.stderr
        )
        .into());
    }
    if !outcome.stderr.starts_with("chronofork: ") {
        return Err(format!(
            "stderr must carry a `chronofork:` diagnostic for {label}:\n{}",
            outcome.stderr
        )
        .into());
    }
    if !outcome.stderr.contains(needle) {
        return Err(format!(
            "expected `{needle}` in the diagnostic for {label}, got:\n{}",
            outcome.stderr
        )
        .into());
    }
    Ok(())
}

/// Requires a CLI success outside the studio stdin path (`--help`).
fn cli_ok(args: &[&str]) -> Result<Run, Box<dyn Error>> {
    let outcome = run(args, "", None)?;
    if outcome.code != Some(0) || !outcome.stderr.is_empty() {
        return Err(format!(
            "expected a clean success for {args:?}, got {:?}\nstdout:\n{}\nstderr:\n{}",
            outcome.code, outcome.stdout, outcome.stderr
        )
        .into());
    }
    Ok(outcome)
}

/// Requires a typed CLI failure with empty stdout (`tas studio` flag errors).
fn cli_failure(args: &[&str], needle: &str) -> TestResult {
    let outcome = run(args, "", None)?;
    expect_failure(&outcome, &format!("{args:?}"), needle)?;
    if !outcome.stdout.is_empty() {
        return Err(format!(
            "a flag failure must keep stdout empty for {args:?}:\n{}",
            outcome.stdout
        )
        .into());
    }
    Ok(())
}

/// Parses the documented blocks, keeping field order and duplicate keys.
fn parse_blocks(stdout: &str) -> Result<Vec<Block>, Box<dyn Error>> {
    if stdout.is_empty() {
        return Err("expected at least one record block, got empty stdout"
            .to_owned()
            .into());
    }
    if !stdout.ends_with('\n') || stdout.ends_with("\n\n") {
        return Err(format!("blocks end with exactly one newline:\n{stdout}").into());
    }
    let mut blocks = Vec::new();
    let mut current = Block::new();
    for line in stdout.lines() {
        if line.is_empty() {
            if current.is_empty() {
                return Err("output must not contain empty blocks".to_owned().into());
            }
            blocks.push(std::mem::take(&mut current));
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("every record line is `key=value`, got `{line}`"))?;
        if key.is_empty() {
            return Err("record key must not be empty".to_owned().into());
        }
        current.push((key.to_owned(), value.to_owned()));
    }
    if current.is_empty() {
        return Err("output must not end with a blank block".to_owned().into());
    }
    blocks.push(current);
    Ok(blocks)
}

fn field<'a>(block: &'a Block, key: &str) -> Result<&'a str, Box<dyn Error>> {
    block
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| format!("record block is missing `{key}`: {block:?}").into())
}

fn number(block: &Block, key: &str) -> Result<u64, Box<dyn Error>> {
    let value = field(block, key)?;
    value
        .parse()
        .map_err(|_| format!("`{key}` is not a decimal value: {block:?}").into())
}

fn assert_keys(block: &Block, expected: &[&str]) -> TestResult {
    let actual = block
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<Vec<_>>();
    if actual.as_slice() != expected {
        return Err(format!("expected fields {expected:?}, got {actual:?}: {block:?}").into());
    }
    Ok(())
}

/// Records a movie through the unchanged `tas record` command.
fn record_movie(
    dir: &Path,
    name: &str,
    steps: usize,
    extra: &[&str],
) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join(name);
    let steps_text = steps.to_string();
    let mut args = vec![
        "tas",
        "record",
        "--movie",
        path_str(&path)?,
        "--steps",
        steps_text.as_str(),
        "--input",
        "5",
    ];
    args.extend_from_slice(extra);
    let outcome = run(&args, "", None)?;
    if outcome.code != Some(0) || !outcome.stderr.is_empty() {
        return Err(format!(
            "record failed: {args:?}\nstdout:\n{}\nstderr:\n{}",
            outcome.stdout, outcome.stderr
        )
        .into());
    }
    Ok(path)
}

/// Records and decodes a deterministic fixture movie.
fn fixture(
    dir: &Path,
    name: &str,
    steps: usize,
    v2: bool,
) -> Result<(PathBuf, Movie), Box<dyn Error>> {
    let extra: &[&str] = if v2 { &["--anchor-interval", "1"] } else { &[] };
    let path = record_movie(dir, name, steps, extra)?;
    let movie = Movie::from_bytes(&std::fs::read(&path)?)?;
    Ok((path, movie))
}

fn step_tick(movie: &Movie, index: usize) -> Result<u64, Box<dyn Error>> {
    movie
        .steps()
        .get(index)
        .map(|step| step.outcome_tick().raw())
        .ok_or_else(|| format!("movie has no recorded step {index}").into())
}

fn step_hash(movie: &Movie, index: usize) -> Result<String, Box<dyn Error>> {
    movie
        .steps()
        .get(index)
        .map(|step| step.state_hash().to_string())
        .ok_or_else(|| format!("movie has no recorded step {index}").into())
}

fn altered_hash(hash: StateHash) -> StateHash {
    let mut bytes = *hash.as_bytes();
    bytes[0] ^= 0x5a;
    StateHash::from_bytes(bytes)
}

/// Expected fixed seek fields for one `assert_seek` check.
struct SeekExpectation<'a> {
    tick: u64,
    applied_steps: u64,
    target_tick: u64,
    start: &'a str,
    start_tick: u64,
    start_step: Option<u64>,
    replayed_steps: u64,
    state_hash: &'a str,
}

/// Asserts the fixed seek fields and their exact values.
fn assert_seek(block: &Block, expected: &SeekExpectation<'_>) -> TestResult {
    assert_keys(
        block,
        if expected.start_step.is_some() {
            SEEK_KEYS_CACHED
        } else {
            SEEK_KEYS
        },
    )?;
    assert_eq!(field(block, "record")?, "seek");
    assert_eq!(number(block, "tick")?, expected.tick);
    assert_eq!(number(block, "applied_steps")?, expected.applied_steps);
    assert_eq!(number(block, "target_tick")?, expected.target_tick);
    assert_eq!(field(block, "start")?, expected.start);
    assert_eq!(number(block, "start_tick")?, expected.start_tick);
    if let Some(start_step) = expected.start_step {
        assert_eq!(number(block, "start_step")?, start_step);
    }
    assert_eq!(number(block, "replayed_steps")?, expected.replayed_steps);
    assert_eq!(field(block, "state_hash")?, expected.state_hash);
    Ok(())
}

#[test]
fn scripted_next_back_and_goto_report_exact_blocks() -> TestResult {
    let dir = temp_dir("scripted")?;
    let (movie_path, movie) = fixture(&dir, "scripted.cfmovie", 5, false)?;
    let checkpoint = movie.checkpoint_state_hash().to_string();
    let script = format!(
        "\nstate\nnext\nnext\nback\nstate\ngoto {}\ngoto {}\nhelp\nquit\n",
        movie.start_tick().raw(),
        step_tick(&movie, 2)?
    );
    let stdout = studio_ok(&movie_path, &[], &script)?;
    let blocks = parse_blocks(&stdout)?;
    assert_eq!(
        blocks.len(),
        9,
        "one block per non-blank command:\n{stdout}"
    );

    // `state` at the floor executes nothing and reports the configured bounds.
    assert_keys(&blocks[0], STATE_KEYS)?;
    assert_eq!(field(&blocks[0], "record")?, "state");
    assert_eq!(number(&blocks[0], "tick")?, 0);
    assert_eq!(number(&blocks[0], "applied_steps")?, 0);
    assert_eq!(field(&blocks[0], "state_hash")?, checkpoint);
    assert_eq!(number(&blocks[0], "movie_version")?, 1);
    assert_eq!(number(&blocks[0], "input_schema")?, 1);
    assert_eq!(number(&blocks[0], "start_tick")?, 0);
    assert_eq!(number(&blocks[0], "end_tick")?, 5);
    assert_eq!(number(&blocks[0], "steps")?, 5);
    assert_eq!(number(&blocks[0], "cache_interval")?, 1);
    assert_eq!(number(&blocks[0], "cache_capacity")?, 256);
    assert_eq!(number(&blocks[0], "cache_byte_limit")?, 16_777_216);
    assert_eq!(number(&blocks[0], "cache_entries")?, 0);
    assert_eq!(number(&blocks[0], "cache_bytes")?, 0);
    assert_eq!(number(&blocks[0], "cache_evictions")?, 0);

    // First `next`: floor start, one replayed step, first cached state.
    assert_seek(
        &blocks[1],
        &SeekExpectation {
            tick: 1,
            applied_steps: 1,
            target_tick: 1,
            start: "floor",
            start_tick: 0,
            start_step: None,
            replayed_steps: 1,
            state_hash: &step_hash(&movie, 0)?,
        },
    )?;
    assert_eq!(number(&blocks[1], "cache_entries")?, 1);
    assert!(number(&blocks[1], "cache_bytes")? > 0);
    assert_eq!(number(&blocks[1], "cache_evictions")?, 0);

    // Second `next`: cached start at step 0, one replayed step.
    assert_seek(
        &blocks[2],
        &SeekExpectation {
            tick: 2,
            applied_steps: 2,
            target_tick: 2,
            start: "cache",
            start_tick: 1,
            start_step: Some(0),
            replayed_steps: 1,
            state_hash: &step_hash(&movie, 1)?,
        },
    )?;
    assert_eq!(number(&blocks[2], "cache_entries")?, 2);
    let two_entry_bytes = number(&blocks[2], "cache_bytes")?;
    assert!(two_entry_bytes > 0);

    // `back`: target is exactly the cached state, so nothing is replayed.
    assert_seek(
        &blocks[3],
        &SeekExpectation {
            tick: 1,
            applied_steps: 1,
            target_tick: 1,
            start: "cache",
            start_tick: 1,
            start_step: Some(0),
            replayed_steps: 0,
            state_hash: &step_hash(&movie, 0)?,
        },
    )?;
    assert_eq!(number(&blocks[3], "cache_entries")?, 2);
    assert_eq!(number(&blocks[3], "cache_bytes")?, two_entry_bytes);
    assert_eq!(number(&blocks[3], "cache_evictions")?, 0);

    // `state` after a seek reports the same verified position.
    assert_eq!(number(&blocks[4], "tick")?, 1);
    assert_eq!(number(&blocks[4], "applied_steps")?, 1);
    assert_eq!(field(&blocks[4], "state_hash")?, step_hash(&movie, 0)?);
    assert_eq!(number(&blocks[4], "cache_entries")?, 2);

    // `goto` the movie start tick: floor start, no replay.
    assert_seek(
        &blocks[5],
        &SeekExpectation {
            tick: 0,
            applied_steps: 0,
            target_tick: 0,
            start: "floor",
            start_tick: 0,
            start_step: None,
            replayed_steps: 0,
            state_hash: &checkpoint,
        },
    )?;
    assert_eq!(number(&blocks[5], "cache_entries")?, 2);

    // `goto` the third recorded tick: cached start at step 1, one replay.
    assert_seek(
        &blocks[6],
        &SeekExpectation {
            tick: 3,
            applied_steps: 3,
            target_tick: 3,
            start: "cache",
            start_tick: 2,
            start_step: Some(1),
            replayed_steps: 1,
            state_hash: &step_hash(&movie, 2)?,
        },
    )?;
    assert_eq!(number(&blocks[6], "cache_entries")?, 3);
    assert!(number(&blocks[6], "cache_bytes")? > two_entry_bytes);
    assert_eq!(number(&blocks[6], "cache_evictions")?, 0);

    assert_keys(&blocks[7], HELP_KEYS)?;
    assert_eq!(field(&blocks[7], "record")?, "help");
    let commands = blocks[7]
        .iter()
        .filter(|(key, _)| key == "command")
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec!["next", "back", "goto <TICK>", "state", "help", "quit"]
    );

    assert_keys(&blocks[8], QUIT_KEYS)?;
    assert_eq!(field(&blocks[8], "record")?, "quit");
    assert_eq!(field(&blocks[8], "reason")?, "command");
    assert_eq!(number(&blocks[8], "tick")?, 3);
    assert_eq!(number(&blocks[8], "applied_steps")?, 3);
    Ok(())
}

#[test]
fn eviction_and_floor_fallback_stay_exact() -> TestResult {
    let dir = temp_dir("eviction")?;
    let (movie_path, movie) = fixture(&dir, "eviction.cfmovie", 8, false)?;
    let end = movie.end_tick().raw();
    let script = format!("goto {end}\nback\nback\nstate\nquit\n");
    let stdout = studio_ok(&movie_path, &["--cache-capacity", "2"], &script)?;
    let blocks = parse_blocks(&stdout)?;
    assert_eq!(blocks.len(), 5, "four commands plus quit:\n{stdout}");

    // Replaying to the end captures eight cadence states; capacity 2 keeps the
    // two highest step indices and evicts the other six.
    assert_seek(
        &blocks[0],
        &SeekExpectation {
            tick: end,
            applied_steps: 8,
            target_tick: end,
            start: "floor",
            start_tick: 0,
            start_step: None,
            replayed_steps: 8,
            state_hash: &step_hash(&movie, 7)?,
        },
    )?;
    assert_eq!(number(&blocks[0], "cache_entries")?, 2);
    assert!(number(&blocks[0], "cache_bytes")? > 0);
    assert_eq!(number(&blocks[0], "cache_evictions")?, 6);

    // `back` lands exactly on retained step 6: cached start, no replay.
    assert_seek(
        &blocks[1],
        &SeekExpectation {
            tick: step_tick(&movie, 6)?,
            applied_steps: 7,
            target_tick: step_tick(&movie, 6)?,
            start: "cache",
            start_tick: step_tick(&movie, 6)?,
            start_step: Some(6),
            replayed_steps: 0,
            state_hash: &step_hash(&movie, 6)?,
        },
    )?;
    assert_eq!(number(&blocks[1], "cache_entries")?, 2);
    assert_eq!(number(&blocks[1], "cache_evictions")?, 6);

    // `back` again targets position 6; the retained entry starts at step 6, which
    // is not at or before that position, so the verified floor is the fallback.
    // The six replayed steps are verified and recaptured on the cadence, evicting
    // the older retained entries: six more evictions and the two highest
    // verified step indices remain (5 and the previously retained 7).
    assert_seek(
        &blocks[2],
        &SeekExpectation {
            tick: step_tick(&movie, 5)?,
            applied_steps: 6,
            target_tick: step_tick(&movie, 5)?,
            start: "floor",
            start_tick: 0,
            start_step: None,
            replayed_steps: 6,
            state_hash: &step_hash(&movie, 5)?,
        },
    )?;
    assert_eq!(number(&blocks[2], "cache_entries")?, 2);
    assert!(number(&blocks[2], "cache_bytes")? > 0);
    assert_eq!(number(&blocks[2], "cache_evictions")?, 12);

    assert_eq!(number(&blocks[3], "tick")?, step_tick(&movie, 5)?);
    assert_eq!(number(&blocks[3], "applied_steps")?, 6);
    assert_eq!(number(&blocks[3], "cache_capacity")?, 2);
    assert_eq!(number(&blocks[3], "cache_entries")?, 2);
    assert_eq!(number(&blocks[3], "cache_evictions")?, 12);
    assert_eq!(field(&blocks[4], "reason")?, "command");
    Ok(())
}

#[test]
fn disabled_capture_reports_the_floor_but_stays_exact() -> TestResult {
    let dir = temp_dir("disabled")?;
    let (movie_path, movie) = fixture(&dir, "disabled.cfmovie", 4, false)?;
    let end = movie.end_tick().raw();
    let script = format!("next\ngoto {end}\nback\nstate\nquit\n");
    let stdout = studio_ok(
        &movie_path,
        &[
            "--cache-interval",
            "0",
            "--cache-capacity",
            "7",
            "--cache-bytes",
            "1000000",
        ],
        &script,
    )?;
    let blocks = parse_blocks(&stdout)?;
    assert_eq!(blocks.len(), 5);

    assert_seek(
        &blocks[0],
        &SeekExpectation {
            tick: 1,
            applied_steps: 1,
            target_tick: 1,
            start: "floor",
            start_tick: 0,
            start_step: None,
            replayed_steps: 1,
            state_hash: &step_hash(&movie, 0)?,
        },
    )?;
    assert_eq!(number(&blocks[0], "cache_entries")?, 0);
    assert_eq!(number(&blocks[0], "cache_bytes")?, 0);

    assert_seek(
        &blocks[1],
        &SeekExpectation {
            tick: end,
            applied_steps: 4,
            target_tick: end,
            start: "floor",
            start_tick: 0,
            start_step: None,
            replayed_steps: 4,
            state_hash: &step_hash(&movie, 3)?,
        },
    )?;
    assert_eq!(number(&blocks[1], "cache_entries")?, 0);
    assert_eq!(number(&blocks[1], "cache_evictions")?, 0);

    assert_seek(
        &blocks[2],
        &SeekExpectation {
            tick: step_tick(&movie, 2)?,
            applied_steps: 3,
            target_tick: step_tick(&movie, 2)?,
            start: "floor",
            start_tick: 0,
            start_step: None,
            replayed_steps: 3,
            state_hash: &step_hash(&movie, 2)?,
        },
    )?;

    assert_eq!(number(&blocks[3], "tick")?, step_tick(&movie, 2)?);
    assert_eq!(number(&blocks[3], "applied_steps")?, 3);
    assert_eq!(number(&blocks[3], "cache_interval")?, 0);
    assert_eq!(number(&blocks[3], "cache_capacity")?, 7);
    assert_eq!(number(&blocks[3], "cache_byte_limit")?, 1_000_000);
    assert_eq!(number(&blocks[3], "cache_entries")?, 0);
    assert_eq!(number(&blocks[3], "cache_bytes")?, 0);
    assert_eq!(number(&blocks[3], "cache_evictions")?, 0);
    Ok(())
}

#[test]
fn eof_quit_and_blank_lines_follow_the_documented_rules() -> TestResult {
    let dir = temp_dir("eof")?;
    let (movie_path, _movie) = fixture(&dir, "eof.cfmovie", 3, false)?;

    // Empty stdin is an immediate EOF quit.
    let blocks = parse_blocks(&studio_ok(&movie_path, &[], "")?)?;
    assert_eq!(blocks.len(), 1);
    assert_keys(&blocks[0], QUIT_KEYS)?;
    assert_eq!(field(&blocks[0], "reason")?, "eof");
    assert_eq!(number(&blocks[0], "tick")?, 0);
    assert_eq!(number(&blocks[0], "applied_steps")?, 0);

    // A final line without a trailing newline is still processed.
    let blocks = parse_blocks(&studio_ok(&movie_path, &[], "state\nnext")?)?;
    assert_eq!(blocks.len(), 3);
    assert_eq!(field(&blocks[1], "record")?, "seek");
    assert_eq!(number(&blocks[1], "tick")?, 1);
    assert_eq!(field(&blocks[2], "reason")?, "eof");
    assert_eq!(number(&blocks[2], "tick")?, 1);
    assert_eq!(number(&blocks[2], "applied_steps")?, 1);

    // `quit` prints its block, exits 0 and never processes later lines.
    let blocks = parse_blocks(&studio_ok(&movie_path, &[], "quit\nnext\nstate\n")?)?;
    assert_eq!(blocks.len(), 1);
    assert_keys(&blocks[0], QUIT_KEYS)?;
    assert_eq!(field(&blocks[0], "reason")?, "command");
    assert_eq!(number(&blocks[0], "tick")?, 0);
    assert_eq!(number(&blocks[0], "applied_steps")?, 0);

    // Blank and whitespace-only lines print nothing.
    let blocks = parse_blocks(&studio_ok(&movie_path, &[], "\n   \n\nhelp\n\n")?)?;
    assert_eq!(blocks.len(), 2);
    assert_keys(&blocks[0], HELP_KEYS)?;
    assert_eq!(field(&blocks[1], "reason")?, "eof");
    Ok(())
}

#[test]
fn invalid_commands_and_targets_fail_closed() -> TestResult {
    let dir = temp_dir("invalid")?;
    let (movie_path, movie) = fixture(&dir, "invalid.cfmovie", 3, false)?;
    let end = movie.end_tick().raw();
    let cases = vec![
        ("bogus\n".to_owned(), "unknown `tas studio` command"),
        ("next 1\n".to_owned(), "`next` takes no arguments"),
        ("Goto 1\n".to_owned(), "unknown `tas studio` command"),
        (
            "goto\n".to_owned(),
            "`goto` expects exactly one <TICK> argument",
        ),
        (
            "goto 1 2\n".to_owned(),
            "`goto` expects exactly one <TICK> argument",
        ),
        ("goto abc\n".to_owned(), "`goto` expects a decimal tick"),
        ("goto -1\n".to_owned(), "`goto` expects a decimal tick"),
        (format!("goto {}\n", end + 1), "is after movie end tick"),
        ("back\n".to_owned(), "already at the start position"),
        ("quit now\n".to_owned(), "`quit` takes no arguments"),
    ];
    for (script, needle) in cases {
        let outcome = studio_failure(&movie_path, &[], &script, needle)?;
        assert!(
            outcome.stdout.is_empty(),
            "a typed failure must print no block for {script:?}:\n{}",
            outcome.stdout
        );
    }

    // A failure after a completed command keeps that command's block and stops
    // before the later line, so no position change is ever reported.
    let outcome = studio_failure(
        &movie_path,
        &[],
        "next\nbogus\nstate\n",
        "unknown `tas studio` command",
    )?;
    let blocks = parse_blocks(&outcome.stdout)?;
    assert_eq!(blocks.len(), 1);
    assert_eq!(field(&blocks[0], "record")?, "seek");
    assert_eq!(number(&blocks[0], "tick")?, 1);

    // `next` at the end position is a typed boundary failure after the block.
    let outcome = studio_failure(
        &movie_path,
        &[],
        &format!("goto {end}\nnext\n"),
        "already at the end position",
    )?;
    let blocks = parse_blocks(&outcome.stdout)?;
    assert_eq!(blocks.len(), 1);
    assert_eq!(number(&blocks[0], "target_tick")?, end);
    Ok(())
}

#[test]
fn seeded_movie_start_tick_is_enforced() -> TestResult {
    let dir = temp_dir("seeded")?;
    let root = dir.join("root");
    let saved = run(
        &[
            "session",
            "save",
            "--root",
            path_str(&root)?,
            "--steps",
            "6",
        ],
        "",
        None,
    )?;
    if saved.code != Some(0) || !saved.stderr.is_empty() {
        return Err(format!(
            "session save failed\nstdout:\n{}\nstderr:\n{}",
            saved.stdout, saved.stderr
        )
        .into());
    }
    let movie_path = record_movie(&dir, "seeded.cfmovie", 2, &["--root", path_str(&root)?])?;
    let movie = Movie::from_bytes(&std::fs::read(&movie_path)?)?;
    assert_eq!(movie.start_tick().raw(), 6);
    assert_eq!(movie.end_tick().raw(), 8);

    let blocks = parse_blocks(&studio_ok(&movie_path, &[], "state\n")?)?;
    assert_eq!(number(&blocks[0], "start_tick")?, 6);
    assert_eq!(number(&blocks[0], "end_tick")?, 8);
    assert_eq!(number(&blocks[0], "steps")?, 2);

    // A target below the movie start is rejected before any restore or step.
    let outcome = studio_failure(&movie_path, &[], "goto 5\n", "is before movie start tick 6")?;
    assert!(outcome.stdout.is_empty());

    // The first recorded tick (7) is aligned and reachable from the floor.
    let first_tick = step_tick(&movie, 0)?;
    assert_eq!(first_tick, 7);
    let blocks = parse_blocks(&studio_ok(
        &movie_path,
        &[],
        &format!("goto {first_tick}\n"),
    )?)?;
    assert_seek(
        &blocks[0],
        &SeekExpectation {
            tick: 7,
            applied_steps: 1,
            target_tick: 7,
            start: "floor",
            start_tick: 6,
            start_step: None,
            replayed_steps: 1,
            state_hash: &step_hash(&movie, 0)?,
        },
    )?;
    assert_eq!(number(&blocks[0], "cache_entries")?, 1);
    Ok(())
}

#[test]
fn unaligned_targets_and_tick_mismatches_are_typed() -> TestResult {
    let dir = temp_dir("gapped")?;
    let (_, base) = fixture(&dir, "gapped-base.cfmovie", 2, false)?;
    let steps = base.steps().to_vec();
    let gapped_steps = vec![
        MovieStep::new(Tick::from_raw(10), steps[0].input(), steps[0].state_hash()),
        MovieStep::new(Tick::from_raw(20), steps[1].input(), steps[1].state_hash()),
    ];
    let gapped = Movie::from_parts(
        base.meta().clone(),
        base.snapshot_bytes().to_vec(),
        gapped_steps,
    )?;
    let gapped_path = dir.join("gapped.cfmovie");
    std::fs::write(&gapped_path, gapped.to_bytes()?)?;

    // `state` reports the gapped end tick and executes nothing.
    let blocks = parse_blocks(&studio_ok(&gapped_path, &[], "state\n")?)?;
    assert_eq!(number(&blocks[0], "start_tick")?, 0);
    assert_eq!(number(&blocks[0], "end_tick")?, 20);
    assert_eq!(number(&blocks[0], "steps")?, 2);

    // 5 lies between recorded ticks 0 and 10: rejected before any restore or step.
    let outcome = studio_failure(
        &gapped_path,
        &[],
        "goto 5\n",
        "not the start tick or a recorded outcome tick",
    )?;
    assert!(outcome.stdout.is_empty());

    // 10 is aligned but replaying its input reports tick 1: a recorded-tick
    // mismatch is stderr-only and must not become a divergence block.
    let outcome = studio_failure(&gapped_path, &[], "goto 10\n", "recorded tick mismatch")?;
    assert!(
        outcome.stdout.is_empty(),
        "a tick mismatch must not print a divergence block:\n{}",
        outcome.stdout
    );
    Ok(())
}

#[test]
fn divergence_reports_the_exact_first_mismatch_from_cache_and_floor() -> TestResult {
    let dir = temp_dir("divergence")?;
    let (_, movie) = fixture(&dir, "base.cfmovie", 6, false)?;
    let index = 4_usize;
    let recorded = movie
        .steps()
        .get(index)
        .ok_or_else(|| format!("movie has no recorded step {index}"))?;
    let observed = recorded.state_hash().to_string();
    let expected = altered_hash(recorded.state_hash()).to_string();
    let mut steps = movie.steps().to_vec();
    steps[index] = MovieStep::new(
        recorded.outcome_tick(),
        recorded.input(),
        altered_hash(recorded.state_hash()),
    );
    let divergent =
        Movie::from_parts(movie.meta().clone(), movie.snapshot_bytes().to_vec(), steps)?;
    let divergent_path = dir.join("divergent.cfmovie");
    std::fs::write(&divergent_path, divergent.to_bytes()?)?;
    let recorded_tick = recorded.outcome_tick().raw();

    // From a cache start: four verified steps capture steps 0-3, then the goto
    // replays exactly the failing fifth input from the cached step 3.
    let script = format!("next\nnext\nnext\nnext\ngoto {recorded_tick}\nquit\n");
    let outcome = studio_failure(&divergent_path, &[], &script, "state divergence")?;
    let blocks = parse_blocks(&outcome.stdout)?;
    assert_eq!(
        blocks.len(),
        5,
        "four seek blocks plus one divergence block:\n{}",
        outcome.stdout
    );
    assert_keys(&blocks[4], DIVERGENCE_KEYS)?;
    assert_eq!(field(&blocks[4], "record")?, "divergence");
    assert_eq!(number(&blocks[4], "diverged")?, 1);
    assert_eq!(number(&blocks[4], "tick")?, recorded_tick);
    assert_eq!(number(&blocks[4], "step")?, 4);
    assert_eq!(field(&blocks[4], "expected")?, expected);
    assert_eq!(field(&blocks[4], "observed")?, observed);
    assert_eq!(field(&blocks[4], "start")?, "cache");
    assert_eq!(number(&blocks[4], "start_tick")?, step_tick(&movie, 3)?);
    assert_eq!(number(&blocks[4], "replayed_steps")?, 1);
    assert_eq!(number(&blocks[4], "cache_entries")?, 4);
    assert!(number(&blocks[4], "cache_bytes")? > 0);

    // From the floor: five inputs are replayed before the mismatch, and the four
    // verified steps before it are cached.
    let outcome = studio_failure(
        &divergent_path,
        &[],
        &format!("goto {recorded_tick}\n"),
        "state divergence",
    )?;
    let blocks = parse_blocks(&outcome.stdout)?;
    assert_eq!(blocks.len(), 1);
    assert_keys(&blocks[0], DIVERGENCE_KEYS)?;
    assert_eq!(field(&blocks[0], "start")?, "floor");
    assert_eq!(number(&blocks[0], "start_tick")?, 0);
    assert_eq!(number(&blocks[0], "replayed_steps")?, 5);
    assert_eq!(number(&blocks[0], "cache_entries")?, 4);
    Ok(())
}

#[test]
fn v1_and_v2_movies_step_identically_and_ignore_anchors() -> TestResult {
    let dir = temp_dir("versions")?;
    let (v1_path, v1) = fixture(&dir, "v1.cfmovie", 4, false)?;
    let (v2_path, v2) = fixture(&dir, "v2.cfmovie", 4, true)?;
    assert_eq!(v2.format_version(), 2);
    assert_eq!(v2.input_schema_version(), 2);
    assert!(
        !v2.anchors().is_empty(),
        "the v2 fixture must carry anchors to prove they are not used"
    );

    let script = format!(
        "state\nnext\nnext\nback\ngoto {}\nquit\n",
        step_tick(&v1, 2)?
    );
    let v1_blocks = parse_blocks(&studio_ok(&v1_path, &[], &script)?)?;
    let v2_blocks = parse_blocks(&studio_ok(&v2_path, &[], &script)?)?;
    assert_eq!(v1_blocks.len(), 6);
    assert_eq!(v2_blocks.len(), 6);

    // The state block differs only in the declared movie/input version.
    assert_eq!(number(&v1_blocks[0], "movie_version")?, 1);
    assert_eq!(number(&v2_blocks[0], "movie_version")?, 2);
    assert_eq!(number(&v1_blocks[0], "input_schema")?, 1);
    assert_eq!(number(&v2_blocks[0], "input_schema")?, 2);
    for key in [
        "tick",
        "applied_steps",
        "state_hash",
        "start_tick",
        "end_tick",
        "steps",
        "cache_interval",
        "cache_capacity",
        "cache_byte_limit",
        "cache_entries",
        "cache_bytes",
        "cache_evictions",
    ] {
        assert_eq!(
            field(&v1_blocks[0], key)?,
            field(&v2_blocks[0], key)?,
            "state field `{key}` must match between v1 and v2"
        );
    }

    // Every seek and the quit block are byte-identical: schema-2 frames and v2
    // anchors never change stepping, cache contents or reported values.
    for index in [1, 2, 3, 4, 5] {
        assert_eq!(
            v1_blocks[index], v2_blocks[index],
            "block {index} must be identical between v1 and v2"
        );
    }
    Ok(())
}

#[test]
fn studio_is_read_only_deterministic_and_prints_no_paths() -> TestResult {
    let dir = temp_dir("read-only")?;
    let (movie_path, _movie) = fixture(&dir, "read-only.cfmovie", 4, false)?;
    let cwd = dir.join("cwd");
    std::fs::create_dir(&cwd)?;
    let before_bytes = std::fs::read(&movie_path)?;
    let before_hash = hash_bytes(&before_bytes);
    let before_files = list_files(&dir)?;
    let script = "state\nnext\ngoto 4\nback\nquit\n";
    let args = studio_args(&movie_path, &[])?;

    let first = run(&args, script, Some(&cwd))?;
    let second = run(&args, script, Some(&cwd))?;
    assert_eq!(first.code, Some(0));
    assert_eq!(second.code, Some(0));
    assert!(
        first.stderr.is_empty() && second.stderr.is_empty(),
        "studio must stay quiet on stderr"
    );
    assert_eq!(
        first.stdout, second.stdout,
        "identical scripts in separate processes must print identical bytes"
    );

    assert_eq!(
        std::fs::read(&movie_path)?,
        before_bytes,
        "the movie file must not be rewritten"
    );
    assert_eq!(hash_bytes(&std::fs::read(&movie_path)?), before_hash);
    assert_eq!(
        list_files(&dir)?,
        before_files,
        "studio must not create or remove a file"
    );
    assert!(
        list_files(&cwd)?.is_empty(),
        "studio must not write its working directory"
    );

    assert!(
        !first.stdout.contains(path_str(&movie_path)?),
        "valid stdout must not contain the movie path"
    );
    assert!(
        !first.stdout.contains(path_str(&dir)?),
        "valid stdout must not contain a host path"
    );
    assert!(
        !first.stdout.contains(&before_hash.to_string()),
        "valid stdout must not contain the movie SHA-256"
    );
    for line in first.stdout.lines().filter(|line| !line.is_empty()) {
        assert!(
            line.contains('='),
            "valid stdout lines are `key=value`, got `{line}`"
        );
    }
    Ok(())
}

#[test]
fn help_and_flags_document_the_studio_contract() -> TestResult {
    let help = cli_ok(&["tas", "--help"])?;
    for needle in [
        "tas studio",
        "--cache-interval",
        "--cache-capacity",
        "--cache-bytes",
        "next, back, goto <TICK>, state, help, quit",
        "record=seek",
        "record=state",
        "record=divergence",
        "16777216",
        "67108864",
        "65536",
        "not O(1)",
        "random access",
        "rerecord",
        "real-hardware input TAS",
        "persistent",
        "synthetic",
        "not complete",
    ] {
        assert!(
            help.stdout.contains(needle),
            "tas help is missing `{needle}`:\n{}",
            help.stdout
        );
    }
    assert_eq!(
        cli_ok(&["tas", "studio", "--help"])?.stdout,
        help.stdout,
        "`tas studio --help` prints the single tas help block"
    );
    assert!(
        cli_ok(&["help"])?.stdout.contains("chronofork tas studio"),
        "top-level help must list the studio command"
    );

    let dir = temp_dir("flags")?;
    let (movie_path, _movie) = fixture(&dir, "flags.cfmovie", 2, false)?;
    let movie = path_str(&movie_path)?;
    let missing = dir.join("missing.cfmovie");
    let missing_text = path_str(&missing)?;
    let cases = vec![
        (
            vec!["tas", "studio"],
            "`tas studio` requires `--movie <FILE>`",
        ),
        (
            vec!["tas", "studio", "--movie"],
            "`--movie` requires a value",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--movie", movie],
            "`--movie` was given more than once",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--cache-interval", "abc"],
            "`--cache-interval` expects a decimal integer",
        ),
        (
            vec![
                "tas",
                "studio",
                "--movie",
                movie,
                "--cache-interval",
                "4194305",
            ],
            "invalid cache_interval",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--cache-capacity", "0"],
            "invalid cache_capacity",
        ),
        (
            vec![
                "tas",
                "studio",
                "--movie",
                movie,
                "--cache-capacity",
                "65537",
            ],
            "invalid cache_capacity",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--cache-bytes", "0"],
            "invalid cache_bytes",
        ),
        (
            vec![
                "tas",
                "studio",
                "--movie",
                movie,
                "--cache-bytes",
                "67108865",
            ],
            "invalid cache_bytes",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--cache-interval"],
            "`--cache-interval` requires a value",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--at", "1"],
            "unknown `tas studio` option `--at`",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--from", "1"],
            "unknown `tas studio` option `--from`",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--until", "1"],
            "unknown `tas studio` option `--until`",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--root", "x"],
            "unknown `tas studio` option `--root`",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--session", "1"],
            "unknown `tas studio` option `--session`",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--steps", "1"],
            "unknown `tas studio` option `--steps`",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "--against", "x"],
            "unknown `tas studio` option `--against`",
        ),
        (
            vec!["tas", "studio", "--movie", movie, "positional"],
            "unknown `tas studio` option `positional`",
        ),
        (
            vec![
                "tas",
                "studio",
                "--movie",
                missing_text,
                "--cache-capacity",
                "0",
            ],
            "invalid cache_capacity",
        ),
        (
            vec!["tas", "studio", "--movie", missing_text],
            "failed to open movie",
        ),
        (vec!["tas", "studio", "--movie", ""], "failed to open movie"),
    ];
    for (args, needle) in cases {
        cli_failure(&args, needle)?;
    }

    cli_failure(
        &["tas", "studio", "--help", "--movie", movie],
        "`tas studio --help` takes no other arguments",
    )?;
    cli_failure(&["tas", "bogus"], "unknown `tas` subcommand")?;
    Ok(())
}
