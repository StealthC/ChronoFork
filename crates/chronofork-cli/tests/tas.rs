//! Process-level acceptance tests for `chronofork tas record|play|verify`.
//!
//! Every invocation spawns the built `chronofork` binary with `Command::output`, so
//! each call is a separate operating-system process. The command line, exit status
//! and the two output streams are the whole contract observed here; movie
//! encoding, decoding, checkpoint import and first-divergence reporting are owned
//! and already tested by the `chronofork-sessions` suites.
//!
//! The tests return `Result` instead of panicking helpers so the workspace
//! `expect_used` lint is satisfied without adding a file-level allowance.

use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use chronofork_core::{StateHash, Tick, hash_bytes};
use chronofork_sessions::{Movie, MovieStep};

type TestResult = Result<(), Box<dyn Error>>;
/// One parsed `key=value` record block.
type Block = BTreeMap<String, String>;

fn temp_dir(name: &str) -> io::Result<PathBuf> {
    let dir =
        std::env::temp_dir().join(format!("chronofork-cli-tas-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn path_str(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str()
        .ok_or_else(|| format!("temporary path is not UTF-8: {}", path.display()).into())
}

fn run(args: &[&str]) -> io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .output()
}

fn streams(output: &Output) -> Result<(String, String), Box<dyn Error>> {
    Ok((
        String::from_utf8(output.stdout.clone())?,
        String::from_utf8(output.stderr.clone())?,
    ))
}

/// Runs the CLI, requiring exit 0, empty stderr and stdout data.
fn ok(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if !output.status.success() {
        return Err(
            format!("command failed: {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}").into(),
        );
    }
    if !stderr.is_empty() {
        return Err(format!("successful command wrote to stderr: {args:?}\n{stderr}").into());
    }
    Ok(stdout)
}

/// Parses the documented `key=value` block format, rejecting malformed output.
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
        if current.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!("duplicate field `{key}` inside one record block").into());
        }
    }
    if current.is_empty() {
        return Err("output must not end with a blank block".to_owned().into());
    }
    blocks.push(current);
    Ok(blocks)
}

fn block(stdout: &str) -> Result<Block, Box<dyn Error>> {
    let mut blocks = parse_blocks(stdout)?;
    if blocks.len() != 1 {
        return Err(format!(
            "expected exactly one record block, got {}:\n{stdout}",
            blocks.len()
        )
        .into());
    }
    Ok(blocks.remove(0))
}

fn field<'a>(block: &'a Block, key: &str) -> Result<&'a str, Box<dyn Error>> {
    block
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("record block is missing `{key}`: {block:?}").into())
}

/// Requires a typed failure: exit 1, no stdout data and a `chronofork:` diagnostic.
fn expect_failure(args: &[&str], needle: &str) -> TestResult {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(1) {
        return Err(format!(
            "expected exit 1 for {args:?}, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        )
        .into());
    }
    if !stdout.is_empty() {
        return Err(format!("typed failure must keep stdout data-only: {args:?}\n{stdout}").into());
    }
    if !stderr.starts_with("chronofork: ") {
        return Err(
            format!("stderr must carry a `chronofork:` diagnostic: {args:?}\n{stderr}").into(),
        );
    }
    if !stderr.contains(needle) {
        return Err(
            format!("expected `{needle}` in the diagnostic for {args:?}, got:\n{stderr}").into(),
        );
    }
    Ok(())
}

/// Requires a corrupted-movie rejection through `tas verify`.
fn expect_movie_rejected(path: &Path, needle: &str) -> TestResult {
    let output = run(&["tas", "verify", "--movie", path_str(path)?])?;
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(1) {
        return Err(format!(
            "expected exit 1 for `{}`, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            path.display(),
            output.status.code()
        )
        .into());
    }
    if !stdout.is_empty() {
        return Err(format!(
            "rejected movie must not print stdout data: `{}`\n{stdout}",
            path.display()
        )
        .into());
    }
    if !stderr.starts_with("chronofork: ") {
        return Err(format!("expected a `chronofork:` diagnostic, got:\n{stderr}").into());
    }
    if !stderr.contains(needle) {
        return Err(format!(
            "expected `{needle}` for `{}`, got:\n{stderr}",
            path.display()
        )
        .into());
    }
    Ok(())
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

/// Recomputes the trailing SHA-256 after a deliberate structural mutation.
fn resign(bytes: &mut [u8]) {
    let body_len = bytes.len() - 32;
    let checksum = hash_bytes(&bytes[..body_len]);
    bytes[body_len..].copy_from_slice(checksum.as_bytes());
}

fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let path = dir.join(name);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn altered_hash(hash: StateHash) -> StateHash {
    let mut bytes = *hash.as_bytes();
    bytes[0] ^= 0x5a;
    StateHash::from_bytes(bytes)
}

#[test]
fn record_play_verify_across_separate_processes() -> TestResult {
    let dir = temp_dir("record-play-verify")?;
    let movie_path = dir.join("baseline.cfmovie");

    let record = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "9",
        "--input",
        "7",
        "--name",
        "baseline",
    ])?)?;
    assert_eq!(field(&record, "record")?, "movie");
    assert_eq!(field(&record, "movie_version")?, "1");
    assert_eq!(field(&record, "input_schema")?, "1");
    assert_eq!(field(&record, "movie_id")?, "1");
    assert_eq!(field(&record, "name")?, "baseline");
    assert_eq!(field(&record, "start_tick")?, "0");
    assert_eq!(field(&record, "end_tick")?, "9");
    assert_eq!(field(&record, "steps")?, "9");
    let final_hash = field(&record, "final_state_hash")?.to_owned();
    let bytes = std::fs::read(&movie_path)?;
    assert!(
        bytes.starts_with(b"CFMOVIE1"),
        "record must write the canonical movie magic"
    );
    assert_eq!(
        field(&record, "movie_sha256")?,
        hash_bytes(&bytes).to_string()
    );

    // Process 2 replays from the movie file only.
    let play = block(&ok(&["tas", "play", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&play, "record")?, "playback");
    assert_eq!(field(&play, "start_tick")?, "0");
    assert_eq!(field(&play, "end_tick")?, "9");
    assert_eq!(field(&play, "steps")?, "9");
    assert_eq!(field(&play, "final_tick")?, "9");
    assert_eq!(field(&play, "final_state_hash")?, final_hash);

    // Process 3 independently verifies the recorded expectations.
    let verify = block(&ok(&["tas", "verify", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&verify, "record")?, "verification");
    assert_eq!(field(&verify, "verified")?, "1");
    assert_eq!(field(&verify, "start_tick")?, "0");
    assert_eq!(field(&verify, "end_tick")?, "9");
    assert_eq!(field(&verify, "steps")?, "9");
    assert_eq!(field(&verify, "final_tick")?, "9");
    assert_eq!(field(&verify, "final_state_hash")?, final_hash);

    // A zero-step movie is valid and verifies the embedded checkpoint as the
    // final state; an absent name prints as an empty field.
    let zero_path = dir.join("zero.cfmovie");
    let zero = block(&ok(&["tas", "record", "--movie", path_str(&zero_path)?])?)?;
    assert_eq!(field(&zero, "steps")?, "0");
    assert_eq!(field(&zero, "name")?, "");
    assert_eq!(field(&zero, "start_tick")?, field(&zero, "end_tick")?);
    let zero_verify = block(&ok(&["tas", "verify", "--movie", path_str(&zero_path)?])?)?;
    assert_eq!(field(&zero_verify, "verified")?, "1");
    assert_eq!(
        field(&zero_verify, "final_tick")?,
        field(&zero_verify, "start_tick")?
    );
    Ok(())
}

#[test]
fn record_reports_metadata_and_bounded_input_mask() -> TestResult {
    let dir = temp_dir("metadata")?;
    let movie_path = dir.join("named.cfmovie");
    let record = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "2",
        "--input",
        &u32::MAX.to_string(),
        "--movie-id",
        "7",
        "--name",
        "branch a",
    ])?)?;
    assert_eq!(field(&record, "movie_id")?, "7");
    assert_eq!(field(&record, "name")?, "branch a");
    assert_eq!(field(&record, "steps")?, "2");
    assert_eq!(field(&record, "start_tick")?, "0");
    assert_eq!(field(&record, "end_tick")?, "2");

    let movie = Movie::from_bytes(&std::fs::read(&movie_path)?)?;
    assert_eq!(movie.meta().movie_id(), 7);
    assert_eq!(movie.meta().name(), Some("branch a"));
    let first = movie
        .steps()
        .first()
        .ok_or_else(|| "recorded movie must have steps".to_owned())?;
    assert_eq!(first.input().buttons(), u32::MAX);
    Ok(())
}

#[test]
fn durable_seeded_record_verifies_without_the_storage_root() -> TestResult {
    let dir = temp_dir("durable")?;
    let root = dir.join("root");
    let save = parse_blocks(&ok(&[
        "session",
        "save",
        "--root",
        path_str(&root)?,
        "--name",
        "durable",
        "--steps",
        "6",
        "--input",
        "3",
    ])?)?;
    assert_eq!(
        save.len(),
        2,
        "session save prints session and snapshot blocks"
    );
    let session_key = field(&save[0], "session_key")?.to_owned();
    let snapshot_key = field(&save[1], "snapshot_key")?.to_owned();
    assert_eq!(field(&save[1], "tick")?, "6");

    let checkpoint = root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{snapshot_key}/checkpoint.cfsnap"
    ));
    let files_before = list_files(&root)?;
    let checkpoint_before = std::fs::read(&checkpoint)?;

    // Process 2 records from the latest committed checkpoint by default and must
    // not write storage.
    let movie_path = dir.join("seeded.cfmovie");
    let record = block(&ok(&[
        "tas",
        "record",
        "--root",
        path_str(&root)?,
        "--steps",
        "4",
        "--input",
        "9",
        "--movie",
        path_str(&movie_path)?,
    ])?)?;
    assert_eq!(field(&record, "start_tick")?, "6");
    assert_eq!(field(&record, "end_tick")?, "10");
    assert_eq!(field(&record, "steps")?, "4");
    assert_eq!(
        list_files(&root)?,
        files_before,
        "record must not write storage"
    );
    assert_eq!(std::fs::read(&checkpoint)?, checkpoint_before);

    // Explicit session/snapshot selection continues from the same checkpoint.
    let explicit_path = dir.join("explicit.cfmovie");
    let explicit = block(&ok(&[
        "tas",
        "record",
        "--root",
        path_str(&root)?,
        "--session",
        &session_key,
        "--snapshot",
        &snapshot_key,
        "--steps",
        "2",
        "--movie",
        path_str(&explicit_path)?,
    ])?)?;
    assert_eq!(field(&explicit, "start_tick")?, "6");
    assert_eq!(field(&explicit, "end_tick")?, "8");

    // Delete the storage root: process 3 verifies from the embedded checkpoint
    // only, with no root argument and no shared handle.
    std::fs::remove_dir_all(&root)?;
    let verify = block(&ok(&["tas", "verify", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&verify, "verified")?, "1");
    assert_eq!(field(&verify, "start_tick")?, "6");
    assert_eq!(
        field(&verify, "final_state_hash")?,
        field(&record, "final_state_hash")?
    );
    Ok(())
}

#[test]
fn divergence_is_exact_through_play_and_verify() -> TestResult {
    let dir = temp_dir("divergence")?;
    let movie_path = dir.join("baseline.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "8",
        "--input",
        "1",
    ])?;

    // Build a deliberately divergent movie through the public sessions API:
    // alter one expected state hash at step 3 and re-encode with a valid checksum.
    let movie = Movie::from_bytes(&std::fs::read(&movie_path)?)?;
    let step_index = 3_usize;
    let original_hash = movie
        .steps()
        .get(step_index)
        .ok_or_else(|| "recorded movie must have four steps".to_owned())?
        .state_hash();
    let expected_hash = altered_hash(original_hash);
    let mut steps = movie.steps().to_vec();
    let recorded = steps[step_index];
    steps[step_index] = MovieStep::new(recorded.outcome_tick(), recorded.input(), expected_hash);
    let divergent =
        Movie::from_parts(movie.meta().clone(), movie.snapshot_bytes().to_vec(), steps)?;
    let divergent_path = dir.join("divergent.cfmovie");
    std::fs::write(&divergent_path, divergent.to_bytes()?)?;
    let expected_tick = movie
        .steps()
        .get(step_index)
        .ok_or_else(|| "recorded movie must have four steps".to_owned())?
        .outcome_tick();

    for (subcommand, record) in [("play", "playback"), ("verify", "verification")] {
        let output = run(&["tas", subcommand, "--movie", path_str(&divergent_path)?])?;
        let (stdout, stderr) = streams(&output)?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "divergence must exit 1: {subcommand}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stderr.starts_with("chronofork: "),
            "stderr must carry a `chronofork:` diagnostic:\n{stderr}"
        );
        assert!(
            stderr.contains("state divergence"),
            "stderr must name the divergence:\n{stderr}"
        );
        // stdout stays a data-only block with the exact first-divergence fields.
        let divergence = block(&stdout)?;
        assert_eq!(field(&divergence, "record")?, record);
        assert_eq!(field(&divergence, "diverged")?, "1");
        assert_eq!(field(&divergence, "tick")?, expected_tick.raw().to_string());
        assert_eq!(field(&divergence, "input_index")?, step_index.to_string());
        assert_eq!(field(&divergence, "expected")?, expected_hash.to_string());
        assert_eq!(field(&divergence, "observed")?, original_hash.to_string());
        assert!(
            !divergence.contains_key("verified"),
            "a divergent verification must not claim verified=1"
        );
    }

    // A changed recorded outcome tick is the separate typed replay failure: exit
    // 1, no stdout data, and stderr naming the index plus expected/observed ticks.
    let last = movie
        .steps()
        .len()
        .checked_sub(1)
        .ok_or_else(|| "recorded movie must have steps".to_owned())?;
    let original_tick = movie.steps()[last].outcome_tick();
    let altered_tick = Tick::from_raw(original_tick.raw() + 5);
    let mut tick_steps = movie.steps().to_vec();
    tick_steps[last] = MovieStep::new(
        altered_tick,
        tick_steps[last].input(),
        tick_steps[last].state_hash(),
    );
    let tick_movie = Movie::from_parts(
        movie.meta().clone(),
        movie.snapshot_bytes().to_vec(),
        tick_steps,
    )?;
    let tick_path = dir.join("tick.cfmovie");
    std::fs::write(&tick_path, tick_movie.to_bytes()?)?;
    let output = run(&["tas", "verify", "--movie", path_str(&tick_path)?])?;
    let (stdout, stderr) = streams(&output)?;
    assert_eq!(
        output.status.code(),
        Some(1),
        "a recorded-tick mismatch must exit 1:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.is_empty(),
        "a recorded-tick mismatch is reported on stderr without a divergence block:\n{stdout}"
    );
    assert!(
        stderr.contains(&format!("recorded tick mismatch at input index {last}")),
        "stderr must name the mismatching index:\n{stderr}"
    );
    assert!(
        stderr.contains(&format!("expected {altered_tick}"))
            && stderr.contains(&format!("observed {original_tick}")),
        "stderr must carry expected and observed ticks:\n{stderr}"
    );

    // The intact movie still verifies cleanly.
    let verify = block(&ok(&["tas", "verify", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&verify, "verified")?, "1");
    assert_eq!(
        field(&verify, "final_state_hash")?,
        movie.final_state_hash().to_string()
    );
    Ok(())
}

#[test]
fn corrupt_movies_are_rejected_with_typed_diagnostics() -> TestResult {
    let dir = temp_dir("corrupt")?;
    let movie_path = dir.join("baseline.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "2",
        "--input",
        "4",
    ])?;
    let valid = std::fs::read(&movie_path)?;

    let mut bad_magic = valid.clone();
    bad_magic[0] ^= 0xff;
    resign(&mut bad_magic);
    let bad_magic = write_fixture(&dir, "bad-magic.cfmovie", &bad_magic)?;
    expect_movie_rejected(&bad_magic, "movie magic mismatch")?;

    let mut bad_version = valid.clone();
    bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
    resign(&mut bad_version);
    let bad_version = write_fixture(&dir, "bad-version.cfmovie", &bad_version)?;
    expect_movie_rejected(&bad_version, "unsupported movie format version 2")?;

    let mut bad_schema = valid.clone();
    bad_schema[10..12].copy_from_slice(&2_u16.to_le_bytes());
    resign(&mut bad_schema);
    let bad_schema = write_fixture(&dir, "bad-schema.cfmovie", &bad_schema)?;
    expect_movie_rejected(&bad_schema, "unsupported movie input schema version 2")?;

    let mut corruption = valid.clone();
    let last = corruption.len() - 1;
    corruption[last] ^= 0xff;
    let corruption = write_fixture(&dir, "checksum.cfmovie", &corruption)?;
    expect_movie_rejected(&corruption, "movie checksum mismatch")?;

    let too_short = write_fixture(&dir, "short.cfmovie", &valid[..40])?;
    expect_movie_rejected(&too_short, "minimum is 86")?;

    // A structurally truncated body with a rebuilt trailing checksum reaches the
    // declared-count/remaining-bytes check.
    let mut truncated = valid[..valid.len() - 32 - 10].to_vec();
    truncated.extend_from_slice(&[0; 32]);
    resign(&mut truncated);
    let truncated = write_fixture(&dir, "truncated.cfmovie", &truncated)?;
    expect_movie_rejected(&truncated, "movie is truncated")?;

    // One byte inserted before the checksum with a rebuilt checksum is a trailing
    // byte, never a reinterpretation.
    let mut trailing = valid[..valid.len() - 32].to_vec();
    trailing.push(0);
    trailing.extend_from_slice(&[0; 32]);
    resign(&mut trailing);
    let trailing = write_fixture(&dir, "trailing.cfmovie", &trailing)?;
    expect_movie_rejected(&trailing, "trailing byte")?;

    // A missing file is a typed CLI failure, not a panic.
    let missing = dir.join("missing.cfmovie");
    expect_failure(
        &["tas", "verify", "--movie", path_str(&missing)?],
        "failed to open movie",
    )?;
    Ok(())
}

#[test]
fn argument_errors_are_typed() -> TestResult {
    let dir = temp_dir("arguments")?;
    let movie_path = dir.join("unused.cfmovie");
    let movie = path_str(&movie_path)?;

    expect_failure(&["tas", "record"], "`tas record` requires `--movie <FILE>`")?;
    expect_failure(&["tas"], "`tas` requires a subcommand")?;
    expect_failure(&["tas", "bogus"], "unknown `tas` subcommand")?;
    expect_failure(
        &["tas", "--help", "extra"],
        "`tas --help` takes no other arguments",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--bogus"],
        "unknown `tas record` option",
    )?;
    expect_failure(
        &[
            "tas", "record", "--movie", movie, "--steps", "1", "--steps", "2",
        ],
        "`--steps` was given more than once",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--steps", "abc"],
        "`--steps` expects a decimal integer",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--input", "4294967296"],
        "`--input` expects a decimal integer",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--steps", "4194305"],
        "`--steps` must be at most 4194304",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--movie-id", "0"],
        "movie id must be non-zero",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--session", "1"],
        "`--session` requires `--root <DIR>`",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--snapshot", "1"],
        "`--snapshot` requires `--session <KEY>`",
    )?;
    // No output file appears after a rejected record.
    assert!(
        !movie_path.exists(),
        "a rejected record must not create a movie"
    );

    expect_failure(&["tas", "play"], "`tas play` requires `--movie <FILE>`")?;
    expect_failure(&["tas", "verify"], "`tas verify` requires `--movie <FILE>`")?;
    expect_failure(
        &["tas", "play", "--movie", movie, "--steps", "1"],
        "unknown `tas play` option `--steps`",
    )?;
    expect_failure(&["tas", "verify", "--movie"], "`--movie` requires a value")?;
    expect_failure(
        &["tas", "verify", "--movie", movie, "--movie", movie],
        "`--movie` was given more than once",
    )?;
    Ok(())
}

#[test]
fn failed_record_never_overwrites_an_existing_movie() -> TestResult {
    let dir = temp_dir("no-overwrite")?;
    let existing = dir.join("existing.cfmovie");
    std::fs::write(&existing, b"sentinel")?;
    let missing_root = dir.join("missing-root");
    expect_failure(
        &[
            "tas",
            "record",
            "--root",
            path_str(&missing_root)?,
            "--movie",
            path_str(&existing)?,
            "--steps",
            "1",
        ],
        "storage root is not initialized",
    )?;
    assert_eq!(
        std::fs::read(&existing)?,
        b"sentinel",
        "a failed record must not replace an existing movie"
    );
    Ok(())
}

#[test]
fn help_documents_tas_and_existing_commands_stay_intact() -> TestResult {
    let tas_help = ok(&["tas", "--help"])?;
    for needle in [
        "tas record",
        "tas play",
        "tas verify",
        "--movie",
        "--steps",
        "--movie-id",
        "--root",
        "max 4194304",
        "not implemented",
    ] {
        assert!(
            tas_help.contains(needle),
            "tas help is missing `{needle}`:\n{tas_help}"
        );
    }
    assert_eq!(ok(&["tas", "record", "--help"])?, tas_help);
    assert_eq!(ok(&["tas", "play", "--help"])?, tas_help);
    assert_eq!(ok(&["tas", "verify", "--help"])?, tas_help);

    let top_help = ok(&["help"])?;
    for needle in [
        "chronofork tas record",
        "chronofork tas play",
        "chronofork tas verify",
        "chronofork tas --help",
    ] {
        assert!(
            top_help.contains(needle),
            "top-level help is missing `{needle}`:\n{top_help}"
        );
    }

    // Pre-existing surface remains wired and unchanged.
    assert!(ok(&["demo"])?.contains("chronofork demo: PASS"));
    assert!(ok(&["backend", "describe"])?.contains("backend: chronofork.test.synthetic"));
    Ok(())
}
