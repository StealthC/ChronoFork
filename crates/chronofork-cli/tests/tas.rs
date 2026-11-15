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
        "tas compare",
        "--movie",
        "--against",
        "--steps",
        "--movie-id",
        "--root",
        "--from <TICK>",
        "--until <TICK>",
        "--ports <P1,P2,P3>",
        "--axes <A0,A1,A2,A3>",
        "--keys <U64>",
        "--anchor-interval <N>",
        "format v2",
        "prefix_verified=0",
        "anchors=<count>",
        "2000000",
        "65536",
        "rewind",
        "rerecord",
        "real-hardware input TAS",
        "not implemented",
        "not complete",
        "re-simulation",
        "record=comparison",
        "max 4194304",
    ] {
        assert!(
            tas_help.contains(needle),
            "tas help is missing `{needle}`:\n{tas_help}"
        );
    }
    assert_eq!(ok(&["tas", "record", "--help"])?, tas_help);
    assert_eq!(ok(&["tas", "play", "--help"])?, tas_help);
    assert_eq!(ok(&["tas", "verify", "--help"])?, tas_help);
    assert_eq!(ok(&["tas", "compare", "--help"])?, tas_help);

    let top_help = ok(&["help"])?;
    for needle in [
        "chronofork tas record",
        "chronofork tas play",
        "chronofork tas verify",
        "chronofork tas compare",
        "chronofork tas --help",
        "--from <TICK>",
        "--until <TICK>",
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

/// Runs a command expecting exit 1 with a `chronofork:` diagnostic, returning
/// the single data-only block printed on stdout (used for compare mismatches).
fn failure_block(args: &[&str], needle: &str) -> Result<Block, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(1) {
        return Err(format!(
            "expected exit 1 for {args:?}, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        )
        .into());
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
    block(&stdout)
}

fn write_movie(dir: &Path, name: &str, movie: &Movie) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join(name);
    std::fs::write(&path, movie.to_bytes()?)?;
    Ok(path)
}

#[test]
fn ranged_play_and_verify_report_exact_windows() -> TestResult {
    let dir = temp_dir("ranged")?;
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
    ])?)?;
    let final_hash = field(&record, "final_state_hash")?.to_owned();
    let movie = Movie::from_bytes(&std::fs::read(&movie_path)?)?;
    let hash_at = |index: usize| -> Result<String, Box<dyn Error>> {
        Ok(movie
            .steps()
            .get(index)
            .ok_or_else(|| format!("expected recorded step {index}"))?
            .state_hash()
            .to_string())
    };
    let bytes_before = std::fs::read(&movie_path)?;

    // No flags keep the exact pre-existing single summary block.
    let default = block(&ok(&["tas", "play", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&default, "record")?, "playback");
    assert_eq!(field(&default, "start_tick")?, "0");
    assert_eq!(field(&default, "end_tick")?, "9");
    assert_eq!(field(&default, "steps")?, "9");
    assert_eq!(field(&default, "final_tick")?, "9");
    assert_eq!(field(&default, "final_state_hash")?, final_hash);
    for range_field in [
        "range_from",
        "range_until",
        "prefix_steps",
        "window_steps",
        "replayed_steps",
    ] {
        assert!(
            !default.contains_key(range_field),
            "default playback output must not grow `{range_field}`: {default:?}"
        );
    }

    // Explicit window (2, 6]: ticks 3..=6, with ticks 1 and 2 as the prefix.
    let ranged = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "2",
        "--until",
        "6",
    ])?)?;
    assert_eq!(field(&ranged, "record")?, "playback");
    assert_eq!(field(&ranged, "start_tick")?, "0");
    assert_eq!(field(&ranged, "end_tick")?, "9");
    assert_eq!(field(&ranged, "steps")?, "9");
    assert_eq!(field(&ranged, "final_tick")?, "6");
    assert_eq!(field(&ranged, "final_state_hash")?, hash_at(5)?);
    assert_eq!(field(&ranged, "range_from")?, "2");
    assert_eq!(field(&ranged, "range_until")?, "6");
    assert_eq!(field(&ranged, "prefix_steps")?, "2");
    assert_eq!(field(&ranged, "window_steps")?, "4");
    assert_eq!(field(&ranged, "replayed_steps")?, "6");

    // `verify` accepts the same range and reports the same window.
    let ranged_verify = block(&ok(&[
        "tas",
        "verify",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "2",
        "--until",
        "6",
    ])?)?;
    assert_eq!(field(&ranged_verify, "record")?, "verification");
    assert_eq!(field(&ranged_verify, "verified")?, "1");
    assert_eq!(field(&ranged_verify, "range_from")?, "2");
    assert_eq!(field(&ranged_verify, "range_until")?, "6");
    assert_eq!(field(&ranged_verify, "prefix_steps")?, "2");
    assert_eq!(field(&ranged_verify, "window_steps")?, "4");
    assert_eq!(field(&ranged_verify, "replayed_steps")?, "6");
    assert_eq!(field(&ranged_verify, "final_tick")?, "6");
    assert_eq!(field(&ranged_verify, "final_state_hash")?, hash_at(5)?);

    // An explicit full range prints the range fields without changing meaning.
    let explicit_full = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "0",
        "--until",
        "9",
    ])?)?;
    assert_eq!(field(&explicit_full, "range_from")?, "0");
    assert_eq!(field(&explicit_full, "range_until")?, "9");
    assert_eq!(field(&explicit_full, "prefix_steps")?, "0");
    assert_eq!(field(&explicit_full, "window_steps")?, "9");
    assert_eq!(field(&explicit_full, "replayed_steps")?, "9");
    assert_eq!(field(&explicit_full, "final_tick")?, "9");
    assert_eq!(field(&explicit_full, "final_state_hash")?, final_hash);

    // One supplied boundary defaults the other to the movie start/end tick.
    let from_only = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "6",
    ])?)?;
    assert_eq!(field(&from_only, "range_from")?, "6");
    assert_eq!(field(&from_only, "range_until")?, "9");
    assert_eq!(field(&from_only, "prefix_steps")?, "6");
    assert_eq!(field(&from_only, "window_steps")?, "3");
    assert_eq!(field(&from_only, "replayed_steps")?, "9");
    assert_eq!(field(&from_only, "final_tick")?, "9");
    assert_eq!(field(&from_only, "final_state_hash")?, final_hash);

    let until_only = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--until",
        "4",
    ])?)?;
    assert_eq!(field(&until_only, "range_from")?, "0");
    assert_eq!(field(&until_only, "range_until")?, "4");
    assert_eq!(field(&until_only, "prefix_steps")?, "0");
    assert_eq!(field(&until_only, "window_steps")?, "4");
    assert_eq!(field(&until_only, "replayed_steps")?, "4");
    assert_eq!(field(&until_only, "final_tick")?, "4");
    assert_eq!(field(&until_only, "final_state_hash")?, hash_at(3)?);

    // Frame advance is a one-step window: (4, 5].
    let advance = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "4",
        "--until",
        "5",
    ])?)?;
    assert_eq!(field(&advance, "window_steps")?, "1");
    assert_eq!(field(&advance, "prefix_steps")?, "4");
    assert_eq!(field(&advance, "replayed_steps")?, "5");
    assert_eq!(field(&advance, "final_tick")?, "5");
    assert_eq!(field(&advance, "final_state_hash")?, hash_at(4)?);

    // Seek to an earlier boundary.
    let seek = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--until",
        "3",
    ])?)?;
    assert_eq!(field(&seek, "final_tick")?, "3");
    assert_eq!(field(&seek, "final_state_hash")?, hash_at(2)?);

    // Read-only commands never modify the movie file.
    assert_eq!(std::fs::read(&movie_path)?, bytes_before);
    Ok(())
}

#[test]
fn ranged_replay_is_independent_across_processes() -> TestResult {
    let dir = temp_dir("ranged-independent")?;
    let movie_path = dir.join("baseline.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "8",
        "--input",
        "3",
    ])?;
    let movie = path_str(&movie_path)?;

    // A late replay followed by an earlier one in separate processes, then the
    // earlier one again: a rewind is a new invocation and carries no state.
    let late_first = ok(&["tas", "play", "--movie", movie, "--from", "6"])?;
    let early_first = ok(&["tas", "play", "--movie", movie, "--until", "3"])?;
    let late_again = ok(&["tas", "play", "--movie", movie, "--from", "6"])?;
    let early_after_late = ok(&["tas", "play", "--movie", movie, "--until", "3"])?;
    assert_eq!(late_first, late_again);
    assert_eq!(early_first, early_after_late);
    assert_ne!(
        late_first, early_first,
        "different windows must report different results"
    );
    Ok(())
}

#[test]
fn invalid_ranges_exit_one_without_output() -> TestResult {
    let dir = temp_dir("ranged-invalid")?;
    let movie_path = dir.join("baseline.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "9",
        "--input",
        "1",
    ])?;
    let movie = path_str(&movie_path)?;

    expect_failure(
        &["tas", "play", "--movie", movie, "--until", "10"],
        "is after the movie end tick 9",
    )?;
    expect_failure(
        &[
            "tas", "play", "--movie", movie, "--from", "6", "--until", "2",
        ],
        "is inverted",
    )?;
    expect_failure(
        &[
            "tas", "play", "--movie", movie, "--from", "4", "--until", "4",
        ],
        "contains no recorded step",
    )?;
    // until == start_tick on a stepped movie is the typed empty-range rejection.
    expect_failure(
        &["tas", "verify", "--movie", movie, "--until", "0"],
        "contains no recorded step",
    )?;
    expect_failure(
        &["tas", "verify", "--movie", movie, "--from", "10"],
        "is inverted",
    )?;
    expect_failure(
        &["tas", "play", "--movie", movie, "--from", "abc"],
        "`--from` expects a decimal integer",
    )?;
    expect_failure(
        &[
            "tas", "play", "--movie", movie, "--from", "1", "--from", "2",
        ],
        "`--from` was given more than once",
    )?;
    expect_failure(
        &["tas", "verify", "--movie", movie, "--until"],
        "`--until` requires a value",
    )?;
    Ok(())
}

#[test]
fn compare_identical_including_metadata_only_difference() -> TestResult {
    let dir = temp_dir("compare-identical")?;
    let first_path = dir.join("first.cfmovie");
    let second_path = dir.join("second.cfmovie");
    let first = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&first_path)?,
        "--steps",
        "9",
        "--input",
        "7",
        "--movie-id",
        "1",
        "--name",
        "branch-a",
    ])?)?;
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&second_path)?,
        "--steps",
        "9",
        "--input",
        "7",
        "--movie-id",
        "5",
        "--name",
        "branch-b",
    ])?;

    // Metadata differs (and so do the file bytes), but the recorded execution is
    // identical, so comparison exits 0 with the identical block.
    let identical = block(&ok(&[
        "tas",
        "compare",
        "--movie",
        path_str(&first_path)?,
        "--against",
        path_str(&second_path)?,
    ])?)?;
    assert_eq!(field(&identical, "record")?, "comparison");
    assert_eq!(field(&identical, "identical")?, "1");
    assert_eq!(field(&identical, "start_tick")?, "0");
    assert_eq!(field(&identical, "steps")?, "9");
    assert_eq!(
        field(&identical, "final_state_hash")?,
        field(&first, "final_state_hash")?
    );

    // The same file may be compared with itself.
    let self_compare = block(&ok(&[
        "tas",
        "compare",
        "--movie",
        path_str(&first_path)?,
        "--against",
        path_str(&first_path)?,
    ])?)?;
    assert_eq!(field(&self_compare, "identical")?, "1");
    Ok(())
}

#[test]
fn compare_reports_the_exact_first_divergence() -> TestResult {
    let dir = temp_dir("compare-divergence")?;
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
    let movie = Movie::from_bytes(&std::fs::read(&movie_path)?)?;
    let step_index = 3_usize;
    let original_hash = movie
        .steps()
        .get(step_index)
        .ok_or_else(|| "recorded movie must have steps".to_owned())?
        .state_hash();
    let altered = altered_hash(original_hash);
    let mut steps = movie.steps().to_vec();
    let recorded = steps[step_index];
    steps[step_index] = MovieStep::new(recorded.outcome_tick(), recorded.input(), altered);
    let divergent =
        Movie::from_parts(movie.meta().clone(), movie.snapshot_bytes().to_vec(), steps)?;
    let divergent_path = write_movie(&dir, "divergent.cfmovie", &divergent)?;

    let block = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&movie_path)?,
            "--against",
            path_str(&divergent_path)?,
        ],
        "recorded branch divergence",
    )?;
    assert_eq!(field(&block, "record")?, "comparison");
    assert_eq!(field(&block, "identical")?, "0");
    assert_eq!(field(&block, "diverged")?, "1");
    assert_eq!(field(&block, "divergence_index")?, "3");
    assert_eq!(field(&block, "tick_a")?, "4");
    assert_eq!(field(&block, "tick_b")?, "4");
    assert_eq!(field(&block, "state_hash_a")?, original_hash.to_string());
    assert_eq!(field(&block, "state_hash_b")?, altered.to_string());
    assert_eq!(field(&block, "steps_a")?, "8");
    assert_eq!(field(&block, "steps_b")?, "8");

    // Several altered indices report the earliest one.
    let mut several = divergent.steps().to_vec();
    several[5] = MovieStep::new(
        several[5].outcome_tick(),
        several[5].input(),
        altered_hash(several[5].state_hash()),
    );
    let several = Movie::from_parts(
        movie.meta().clone(),
        movie.snapshot_bytes().to_vec(),
        several,
    )?;
    let several_path = write_movie(&dir, "several.cfmovie", &several)?;
    let block = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&movie_path)?,
            "--against",
            path_str(&several_path)?,
        ],
        "recorded branch divergence",
    )?;
    assert_eq!(field(&block, "divergence_index")?, "3");

    // A tick-only alteration reports both ticks and equal recorded hashes.
    let last = movie
        .steps()
        .len()
        .checked_sub(1)
        .ok_or_else(|| "recorded movie must have steps".to_owned())?;
    let original_tick = movie.steps()[last].outcome_tick();
    let moved_tick = Tick::from_raw(original_tick.raw() + 5);
    let mut tick_steps = movie.steps().to_vec();
    tick_steps[last] = MovieStep::new(
        moved_tick,
        tick_steps[last].input(),
        tick_steps[last].state_hash(),
    );
    let tick_movie = Movie::from_parts(
        movie.meta().clone(),
        movie.snapshot_bytes().to_vec(),
        tick_steps,
    )?;
    let tick_path = write_movie(&dir, "tick.cfmovie", &tick_movie)?;
    let block = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&movie_path)?,
            "--against",
            path_str(&tick_path)?,
        ],
        "recorded branch divergence",
    )?;
    assert_eq!(field(&block, "divergence_index")?, &last.to_string());
    assert_eq!(field(&block, "tick_a")?, original_tick.raw().to_string());
    assert_eq!(field(&block, "tick_b")?, moved_tick.raw().to_string());
    assert_eq!(
        field(&block, "state_hash_a")?,
        field(&block, "state_hash_b")?
    );

    // A divergence in the common prefix has precedence over the length mismatch
    // and both step counts stay visible.
    let short_path = dir.join("short.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&short_path)?,
        "--steps",
        "5",
        "--input",
        "1",
    ])?;
    let block = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&divergent_path)?,
            "--against",
            path_str(&short_path)?,
        ],
        "recorded branch divergence",
    )?;
    assert_eq!(field(&block, "divergence_index")?, "3");
    assert_eq!(field(&block, "steps_a")?, "8");
    assert_eq!(field(&block, "steps_b")?, "5");
    Ok(())
}

#[test]
fn compare_reports_length_and_start_mismatches() -> TestResult {
    let dir = temp_dir("compare-mismatch")?;
    let long_path = dir.join("long.cfmovie");
    let short_path = dir.join("short.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&long_path)?,
        "--steps",
        "8",
        "--input",
        "2",
    ])?;
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&short_path)?,
        "--steps",
        "5",
        "--input",
        "2",
    ])?;

    let block = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&long_path)?,
            "--against",
            path_str(&short_path)?,
        ],
        "recorded branch length mismatch",
    )?;
    assert_eq!(field(&block, "identical")?, "0");
    assert_eq!(field(&block, "length_mismatch")?, "1");
    assert_eq!(field(&block, "common_steps")?, "5");
    assert_eq!(field(&block, "steps_a")?, "8");
    assert_eq!(field(&block, "steps_b")?, "5");

    // A movie recorded from a later durable checkpoint has a different embedded
    // checkpoint start tick, which is reported separately from step records.
    let root = dir.join("root");
    ok(&[
        "session",
        "save",
        "--root",
        path_str(&root)?,
        "--steps",
        "6",
        "--input",
        "3",
    ])?;
    let seeded_path = dir.join("seeded.cfmovie");
    ok(&[
        "tas",
        "record",
        "--root",
        path_str(&root)?,
        "--movie",
        path_str(&seeded_path)?,
        "--steps",
        "3",
        "--input",
        "9",
    ])?;
    let fresh_path = dir.join("fresh.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&fresh_path)?,
        "--steps",
        "3",
        "--input",
        "9",
    ])?;
    let block = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&fresh_path)?,
            "--against",
            path_str(&seeded_path)?,
        ],
        "recorded branch start mismatch",
    )?;
    assert_eq!(field(&block, "identical")?, "0");
    assert_eq!(field(&block, "checkpoint_mismatch")?, "1");
    assert_eq!(field(&block, "start_tick_a")?, "0");
    assert_eq!(field(&block, "start_tick_b")?, "6");
    assert_ne!(
        field(&block, "checkpoint_state_hash_a")?,
        field(&block, "checkpoint_state_hash_b")?
    );
    assert_eq!(field(&block, "steps_a")?, "3");
    assert_eq!(field(&block, "steps_b")?, "3");

    // Same start tick but different embedded checkpoint state is also a start
    // mismatch, not a step divergence.
    let first_root = dir.join("first-root");
    let second_root = dir.join("second-root");
    ok(&[
        "session",
        "save",
        "--root",
        path_str(&first_root)?,
        "--steps",
        "6",
        "--input",
        "1",
    ])?;
    ok(&[
        "session",
        "save",
        "--root",
        path_str(&second_root)?,
        "--steps",
        "6",
        "--input",
        "2",
    ])?;
    let first_seeded = dir.join("first-seeded.cfmovie");
    let second_seeded = dir.join("second-seeded.cfmovie");
    ok(&[
        "tas",
        "record",
        "--root",
        path_str(&first_root)?,
        "--movie",
        path_str(&first_seeded)?,
        "--steps",
        "2",
        "--input",
        "0",
    ])?;
    ok(&[
        "tas",
        "record",
        "--root",
        path_str(&second_root)?,
        "--movie",
        path_str(&second_seeded)?,
        "--steps",
        "2",
        "--input",
        "0",
    ])?;
    let block = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&first_seeded)?,
            "--against",
            path_str(&second_seeded)?,
        ],
        "recorded branch start mismatch",
    )?;
    assert_eq!(field(&block, "start_tick_a")?, "6");
    assert_eq!(field(&block, "start_tick_b")?, "6");
    assert_ne!(
        field(&block, "checkpoint_state_hash_a")?,
        field(&block, "checkpoint_state_hash_b")?
    );
    assert_eq!(field(&block, "steps_a")?, "2");
    assert_eq!(field(&block, "steps_b")?, "2");

    // Corruption and a missing file stay typed, exit 1 and print no block.
    let mut corrupt = std::fs::read(&long_path)?;
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff;
    let corrupt_path = write_fixture(&dir, "corrupt.cfmovie", &corrupt)?;
    expect_failure(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&long_path)?,
            "--against",
            path_str(&corrupt_path)?,
        ],
        "movie checksum mismatch",
    )?;
    let missing = dir.join("missing.cfmovie");
    expect_failure(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&missing)?,
            "--against",
            path_str(&long_path)?,
        ],
        "failed to open movie",
    )?;
    Ok(())
}

#[test]
fn compare_argument_errors_are_typed() -> TestResult {
    let dir = temp_dir("compare-arguments")?;
    let movie_path = dir.join("unused.cfmovie");
    let movie = path_str(&movie_path)?;
    expect_failure(
        &["tas", "compare"],
        "`tas compare` requires `--movie <FILE>`",
    )?;
    expect_failure(
        &["tas", "compare", "--movie", movie],
        "`tas compare` requires `--against <FILE>`",
    )?;
    expect_failure(
        &["tas", "compare", "--movie", movie, "--against"],
        "`--against` requires a value",
    )?;
    expect_failure(
        &[
            "tas",
            "compare",
            "--movie",
            movie,
            "--against",
            movie,
            "--from",
            "1",
        ],
        "unknown `tas compare` option `--from`",
    )?;
    expect_failure(
        &[
            "tas",
            "compare",
            "--movie",
            movie,
            "--movie",
            movie,
            "--against",
            movie,
        ],
        "`--movie` was given more than once",
    )?;
    Ok(())
}

#[test]
fn multi_device_v2_round_trip() -> TestResult {
    let dir = temp_dir("v2-multi-device")?;
    let movie_path = dir.join("devices.cfmovie");
    let record_stdout = ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "5",
        "--input",
        "1",
        "--ports",
        "2,3,4",
        "--axes",
        "-1,0,32767,-32768",
        "--keys",
        "18446744073709551615",
        "--name",
        "devices",
    ])?;
    let record = block(&record_stdout)?;
    assert_eq!(field(&record, "record")?, "movie");
    assert_eq!(field(&record, "movie_version")?, "2");
    assert_eq!(field(&record, "input_schema")?, "2");
    assert_eq!(field(&record, "name")?, "devices");
    assert_eq!(field(&record, "steps")?, "5");
    assert!(
        !record.contains_key("anchors"),
        "a v2 movie without anchors must not gain an anchors line: {record:?}"
    );
    assert!(
        !record_stdout.contains(path_str(&dir)?),
        "valid stdout must not contain host paths:\n{record_stdout}"
    );

    let bytes = std::fs::read(&movie_path)?;
    assert!(
        bytes.starts_with(b"CFMOVIE2"),
        "a multi-device record must write the v2 magic"
    );
    assert_eq!(
        field(&record, "movie_sha256")?,
        hash_bytes(&bytes).to_string()
    );
    let movie = Movie::from_bytes(&bytes)?;
    assert_eq!(movie.format_version(), 2);
    assert_eq!(movie.input_schema_version(), 2);
    assert!(movie.anchors().is_empty());
    let first = movie
        .steps()
        .first()
        .ok_or_else(|| "recorded movie must have steps".to_owned())?;
    let frame = first.input();
    assert_eq!(frame.ports(), [1, 2, 3, 4]);
    assert_eq!(frame.axes(), [-1, 0, 32767, -32768]);
    assert_eq!(frame.keys(), u64::MAX);

    // Independent processes replay and verify the multi-device movie from its
    // embedded checkpoint and reproduce the recorded final state hash.
    let final_hash = field(&record, "final_state_hash")?.to_owned();
    let play = block(&ok(&["tas", "play", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&play, "record")?, "playback");
    assert_eq!(field(&play, "final_tick")?, "5");
    assert_eq!(field(&play, "final_state_hash")?, final_hash);
    let verify = block(&ok(&["tas", "verify", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&verify, "record")?, "verification");
    assert_eq!(field(&verify, "verified")?, "1");
    assert_eq!(field(&verify, "final_state_hash")?, final_hash);

    // Read-only commands never modify the movie file.
    assert_eq!(std::fs::read(&movie_path)?, bytes);
    Ok(())
}

#[test]
fn anchor_interval_and_anchored_replay_report() -> TestResult {
    let dir = temp_dir("v2-anchors")?;
    let movie_path = dir.join("anchors.cfmovie");
    let record = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "9",
        "--input",
        "7",
        "--anchor-interval",
        "3",
        "--name",
        "anchors",
    ])?)?;
    assert_eq!(field(&record, "movie_version")?, "2");
    assert_eq!(field(&record, "input_schema")?, "2");
    assert_eq!(field(&record, "anchors")?, "3");
    let bytes = std::fs::read(&movie_path)?;
    assert!(bytes.starts_with(b"CFMOVIE2"));
    let movie = Movie::from_bytes(&bytes)?;
    let indices: Vec<usize> = movie
        .anchors()
        .iter()
        .map(|anchor| anchor.step_index())
        .collect();
    assert_eq!(indices, vec![2, 5, 8]);
    let ticks: Vec<u64> = movie
        .anchors()
        .iter()
        .map(|anchor| anchor.tick().raw())
        .collect();
    assert_eq!(ticks, vec![3, 6, 9]);

    // A full-range replay starts at the embedded checkpoint and prints no anchor
    // field; the anchor table alone never changes the default block.
    for (subcommand, record_name) in [("play", "playback"), ("verify", "verification")] {
        let full = block(&ok(&[
            "tas",
            subcommand,
            "--movie",
            path_str(&movie_path)?,
        ])?)?;
        assert_eq!(field(&full, "record")?, record_name);
        for key in ["anchored", "anchor_step", "anchor_tick", "prefix_verified"] {
            assert!(
                !full.contains_key(key),
                "`tas {subcommand}` without a range must not print `{key}`: {full:?}"
            );
        }
    }

    // `--from` exactly on the first anchor step: steps 1-3 are the prefix and the
    // anchor at step 2 covers indices 0-2, so only indices 3-8 execute.
    let anchored = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "3",
        "--until",
        "9",
    ])?)?;
    assert_eq!(field(&anchored, "anchored")?, "1");
    assert_eq!(field(&anchored, "anchor_step")?, "2");
    assert_eq!(field(&anchored, "anchor_tick")?, "3");
    assert_eq!(field(&anchored, "prefix_verified")?, "0");
    assert_eq!(field(&anchored, "prefix_steps")?, "3");
    assert_eq!(field(&anchored, "window_steps")?, "6");
    assert_eq!(field(&anchored, "replayed_steps")?, "6");
    assert_eq!(field(&anchored, "final_tick")?, "9");
    assert_eq!(
        field(&anchored, "final_state_hash")?,
        field(&record, "final_state_hash")?
    );

    // Between two anchors the earlier one is selected, and `replayed_steps` is
    // the executed suffix length, not prefix + window.
    let between = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "4",
        "--until",
        "9",
    ])?)?;
    assert_eq!(field(&between, "anchor_step")?, "2");
    assert_eq!(field(&between, "anchor_tick")?, "3");
    assert_eq!(field(&between, "prefix_steps")?, "4");
    assert_eq!(field(&between, "window_steps")?, "5");
    assert_eq!(field(&between, "replayed_steps")?, "6");

    // An anchor on the final step (tick 9) is a valid format value but cannot be
    // selected by a non-empty range: `--from 6 --until 9` selects step 5.
    let final_anchor = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "6",
        "--until",
        "9",
    ])?)?;
    assert_eq!(field(&final_anchor, "anchor_step")?, "5");
    assert_eq!(field(&final_anchor, "anchor_tick")?, "6");
    assert_eq!(field(&final_anchor, "replayed_steps")?, "3");

    // `verify` accepts the same range, reports verified=1 and the anchor fields.
    let anchored_verify = block(&ok(&[
        "tas",
        "verify",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "3",
        "--until",
        "9",
    ])?)?;
    assert_eq!(field(&anchored_verify, "verified")?, "1");
    assert_eq!(field(&anchored_verify, "anchored")?, "1");
    assert_eq!(field(&anchored_verify, "anchor_step")?, "2");
    assert_eq!(field(&anchored_verify, "prefix_verified")?, "0");
    assert_eq!(
        field(&anchored_verify, "final_state_hash")?,
        field(&record, "final_state_hash")?
    );

    // An empty prefix (`--from` equals the start tick) can never select an anchor.
    let linear = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&movie_path)?,
        "--from",
        "0",
        "--until",
        "9",
    ])?)?;
    assert!(!linear.contains_key("anchored"));
    assert_eq!(field(&linear, "prefix_steps")?, "0");
    assert_eq!(field(&linear, "replayed_steps")?, "9");

    // A divergence after the selected anchor reuses the existing divergence block
    // and exit 1: the altered expectation at step 6 is reached from the anchor.
    let altered_index = 6_usize;
    let original_hash = movie
        .steps()
        .get(altered_index)
        .ok_or_else(|| format!("recorded movie must have step {altered_index}"))?
        .state_hash();
    let altered = altered_hash(original_hash);
    let mut steps = movie.steps().to_vec();
    steps[altered_index] = MovieStep::new(
        steps[altered_index].outcome_tick(),
        steps[altered_index].input(),
        altered,
    );
    let divergent = Movie::from_v2_parts(
        movie.meta().clone(),
        movie.snapshot_bytes().to_vec(),
        movie.anchors().to_vec(),
        steps,
    )?;
    let divergent_path = dir.join("divergent.cfmovie");
    std::fs::write(&divergent_path, divergent.to_bytes()?)?;
    let divergence = failure_block(
        &[
            "tas",
            "play",
            "--movie",
            path_str(&divergent_path)?,
            "--from",
            "3",
            "--until",
            "9",
        ],
        "state divergence",
    )?;
    assert_eq!(field(&divergence, "diverged")?, "1");
    assert_eq!(
        field(&divergence, "input_index")?,
        &altered_index.to_string()
    );
    assert_eq!(field(&divergence, "expected")?, &altered.to_string());
    assert_eq!(field(&divergence, "observed")?, &original_hash.to_string());

    // Read-only anchored replays leave the movie file unchanged.
    assert_eq!(std::fs::read(&movie_path)?, bytes);
    Ok(())
}

#[test]
fn v1_default_blocks_and_selection_unchanged() -> TestResult {
    let dir = temp_dir("v1-default")?;
    let first_path = dir.join("first.cfmovie");
    let second_path = dir.join("second.cfmovie");
    let third_path = dir.join("third.cfmovie");
    let record = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&first_path)?,
        "--steps",
        "4",
        "--input",
        "5",
    ])?)?;
    assert_eq!(field(&record, "movie_version")?, "1");
    assert_eq!(field(&record, "input_schema")?, "1");
    assert!(
        !record.contains_key("anchors"),
        "a v1 record must not print an anchors line: {record:?}"
    );
    let first_bytes = std::fs::read(&first_path)?;
    assert!(first_bytes.starts_with(b"CFMOVIE1"));

    // `--anchor-interval 0` alone is not a v2 request: same v1 bytes and block.
    let zero_record = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&second_path)?,
        "--steps",
        "4",
        "--input",
        "5",
        "--anchor-interval",
        "0",
    ])?)?;
    assert_eq!(zero_record, record);
    assert_eq!(std::fs::read(&second_path)?, first_bytes);

    // A separate process reproduces the same v1 bytes.
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&third_path)?,
        "--steps",
        "4",
        "--input",
        "5",
    ])?;
    assert_eq!(std::fs::read(&third_path)?, first_bytes);

    // Default play/verify blocks keep exactly the pre-v2 fields and values.
    let final_hash = field(&record, "final_state_hash")?.to_owned();
    let mut expected_play = Block::new();
    expected_play.insert("record".to_owned(), "playback".to_owned());
    expected_play.insert("start_tick".to_owned(), "0".to_owned());
    expected_play.insert("end_tick".to_owned(), "4".to_owned());
    expected_play.insert("steps".to_owned(), "4".to_owned());
    expected_play.insert("final_tick".to_owned(), "4".to_owned());
    expected_play.insert("final_state_hash".to_owned(), final_hash.clone());
    let play = block(&ok(&["tas", "play", "--movie", path_str(&first_path)?])?)?;
    assert_eq!(play, expected_play);

    let mut expected_verify = expected_play.clone();
    expected_verify.insert("record".to_owned(), "verification".to_owned());
    expected_verify.insert("verified".to_owned(), "1".to_owned());
    let verify = block(&ok(&["tas", "verify", "--movie", path_str(&first_path)?])?)?;
    assert_eq!(verify, expected_verify);

    // A ranged replay on a v1 movie adds only the range/work fields.
    let ranged = block(&ok(&[
        "tas",
        "play",
        "--movie",
        path_str(&first_path)?,
        "--from",
        "2",
    ])?)?;
    assert_eq!(field(&ranged, "range_from")?, "2");
    assert_eq!(field(&ranged, "range_until")?, "4");
    assert_eq!(field(&ranged, "prefix_steps")?, "2");
    assert_eq!(field(&ranged, "window_steps")?, "2");
    assert_eq!(field(&ranged, "replayed_steps")?, "4");
    for key in ["anchored", "anchor_step", "anchor_tick", "prefix_verified"] {
        assert!(
            !ranged.contains_key(key),
            "a v1 ranged replay must not print `{key}`: {ranged:?}"
        );
    }
    Ok(())
}

#[test]
fn compare_accepts_v2_movies() -> TestResult {
    let dir = temp_dir("compare-v2")?;
    let anchored_path = dir.join("anchored.cfmovie");
    let neutral_path = dir.join("neutral.cfmovie");
    let v1_path = dir.join("v1.cfmovie");
    let different_path = dir.join("different.cfmovie");
    let anchored = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&anchored_path)?,
        "--steps",
        "6",
        "--input",
        "7",
        "--anchor-interval",
        "3",
        "--name",
        "anchored",
    ])?)?;
    // A different metadata set, a v2 request without anchors and neutral device
    // fields never decide equality: the recorded expectations are identical.
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&neutral_path)?,
        "--steps",
        "6",
        "--input",
        "7",
        "--ports",
        "0,0,0",
        "--axes",
        "0,0,0,0",
        "--keys",
        "0",
        "--movie-id",
        "9",
        "--name",
        "neutral",
    ])?;
    let identical = block(&ok(&[
        "tas",
        "compare",
        "--movie",
        path_str(&anchored_path)?,
        "--against",
        path_str(&neutral_path)?,
    ])?)?;
    assert_eq!(field(&identical, "record")?, "comparison");
    assert_eq!(field(&identical, "identical")?, "1");
    assert_eq!(field(&identical, "start_tick")?, "0");
    assert_eq!(field(&identical, "steps")?, "6");
    assert_eq!(
        field(&identical, "final_state_hash")?,
        field(&anchored, "final_state_hash")?
    );

    // A v1 movie with the same checkpoint identity and expectations is identical
    // to a v2 movie: format version, anchors and input schema are not equality
    // fields.
    let v1 = block(&ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&v1_path)?,
        "--steps",
        "6",
        "--input",
        "7",
    ])?)?;
    let cross_format = block(&ok(&[
        "tas",
        "compare",
        "--movie",
        path_str(&anchored_path)?,
        "--against",
        path_str(&v1_path)?,
    ])?)?;
    assert_eq!(field(&cross_format, "identical")?, "1");
    assert_eq!(
        field(&cross_format, "final_state_hash")?,
        field(&v1, "final_state_hash")?
    );

    // A non-neutral device frame changes the recorded trajectory: the existing
    // divergence block and exit 1.
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&different_path)?,
        "--steps",
        "6",
        "--input",
        "7",
        "--ports",
        "1,0,0",
        "--axes",
        "0,0,0,0",
        "--keys",
        "0",
    ])?;
    let divergent = failure_block(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&anchored_path)?,
            "--against",
            path_str(&different_path)?,
        ],
        "recorded branch divergence",
    )?;
    assert_eq!(field(&divergent, "identical")?, "0");
    assert_eq!(field(&divergent, "diverged")?, "1");
    assert_eq!(field(&divergent, "steps_a")?, "6");
    assert_eq!(field(&divergent, "steps_b")?, "6");

    // A malformed v2 file still fails at decode before any comparison and keeps
    // stdout empty.
    let mut corrupt = std::fs::read(&different_path)?;
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff;
    let corrupt_path = write_fixture(&dir, "corrupt-v2.cfmovie", &corrupt)?;
    expect_failure(
        &[
            "tas",
            "compare",
            "--movie",
            path_str(&anchored_path)?,
            "--against",
            path_str(&corrupt_path)?,
        ],
        "movie checksum mismatch",
    )?;
    Ok(())
}

#[test]
fn v2_flag_rejections_are_typed() -> TestResult {
    let dir = temp_dir("v2-arguments")?;
    let movie_path = dir.join("rejected.cfmovie");
    let movie = path_str(&movie_path)?;

    let cases: &[(&[&str], &str)] = &[
        (
            &["--ports", "1,2"],
            "`--ports` expects exactly three comma-separated decimal u32 values",
        ),
        (
            &["--ports", "1,2,3,4"],
            "`--ports` expects exactly three comma-separated decimal u32 values",
        ),
        (
            &["--ports", "1,,3"],
            "`--ports` expects exactly three comma-separated decimal u32 values",
        ),
        (
            &["--ports", "1,2,abc"],
            "`--ports` expects exactly three comma-separated decimal u32 values",
        ),
        (
            &["--ports", "1,2,4294967296"],
            "`--ports` expects exactly three comma-separated decimal u32 values",
        ),
        (
            &["--axes", "1,2,3"],
            "`--axes` expects exactly four comma-separated decimal signed 16-bit values",
        ),
        (
            &["--axes", "1,2,3,4,5"],
            "`--axes` expects exactly four comma-separated decimal signed 16-bit values",
        ),
        (
            &["--axes", "1,2,3,32768"],
            "`--axes` expects exactly four comma-separated decimal signed 16-bit values",
        ),
        (
            &["--axes", "1,2,3,-32769"],
            "`--axes` expects exactly four comma-separated decimal signed 16-bit values",
        ),
        (&["--keys", "abc"], "`--keys` expects a decimal integer"),
        (
            &["--keys", "18446744073709551616"],
            "`--keys` expects a decimal integer",
        ),
        (
            &["--anchor-interval", "abc"],
            "`--anchor-interval` expects a decimal integer",
        ),
        (
            &["--anchor-interval", "18446744073709551616"],
            "`--anchor-interval` expects a decimal integer",
        ),
    ];
    for (flags, needle) in cases {
        let mut args = vec!["tas", "record", "--movie", movie, "--steps", "3"];
        args.extend_from_slice(flags);
        expect_failure(&args, needle)?;
    }

    // Duplicate and missing-value flags are typed failures.
    expect_failure(
        &[
            "tas", "record", "--movie", movie, "--steps", "3", "--ports", "1,2,3", "--ports",
            "4,5,6",
        ],
        "`--ports` was given more than once",
    )?;
    expect_failure(
        &[
            "tas", "record", "--movie", movie, "--steps", "3", "--keys", "1", "--keys", "2",
        ],
        "`--keys` was given more than once",
    )?;
    expect_failure(
        &[
            "tas",
            "record",
            "--movie",
            movie,
            "--steps",
            "3",
            "--anchor-interval",
            "2",
            "--anchor-interval",
            "2",
        ],
        "`--anchor-interval` was given more than once",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--keys"],
        "`--keys` requires a value",
    )?;
    expect_failure(
        &["tas", "record", "--movie", movie, "--axes"],
        "`--axes` requires a value",
    )?;

    // Incompatible v2 flag combinations fail before execution.
    expect_failure(
        &[
            "tas",
            "record",
            "--movie",
            movie,
            "--steps",
            "0",
            "--anchor-interval",
            "1",
        ],
        "requires at least one recorded step",
    )?;
    expect_failure(
        &[
            "tas",
            "record",
            "--movie",
            movie,
            "--steps",
            "3",
            "--anchor-interval",
            "4",
        ],
        "exceeds `--steps 3`",
    )?;
    expect_failure(
        &[
            "tas",
            "record",
            "--movie",
            movie,
            "--steps",
            "65537",
            "--anchor-interval",
            "1",
        ],
        "above the 65536-anchor limit",
    )?;
    expect_failure(
        &[
            "tas", "record", "--movie", movie, "--steps", "2000001", "--ports", "0,0,0",
        ],
        "the format v2 movie step limit",
    )?;
    expect_failure(
        &[
            "tas", "record", "--movie", movie, "--steps", "4194305", "--input", "1",
        ],
        "`--steps` must be at most 4194304",
    )?;
    expect_failure(
        &[
            "tas",
            "record",
            "--movie",
            movie,
            "--steps",
            "3",
            "--anchor-interval",
            "2",
            "--session",
            "1",
        ],
        "`--session` requires `--root <DIR>`",
    )?;
    assert!(
        !movie_path.exists(),
        "a rejected v2 record must not create a movie"
    );
    Ok(())
}

#[test]
fn corrupt_v2_movies_are_rejected_with_typed_diagnostics() -> TestResult {
    let dir = temp_dir("corrupt-v2")?;
    let movie_path = dir.join("valid-v2.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path)?,
        "--steps",
        "6",
        "--input",
        "4",
        "--anchor-interval",
        "3",
        "--name",
        "v2",
    ])?;
    let valid = std::fs::read(&movie_path)?;
    assert!(valid.starts_with(b"CFMOVIE2"));
    let movie = Movie::from_bytes(&valid)?;

    let mut bad_magic = valid.clone();
    bad_magic[0] ^= 0xff;
    resign(&mut bad_magic);
    let bad_magic = write_fixture(&dir, "bad-magic-v2.cfmovie", &bad_magic)?;
    expect_movie_rejected(&bad_magic, "movie magic mismatch")?;

    let mut bad_version = valid.clone();
    bad_version[8..10].copy_from_slice(&3_u16.to_le_bytes());
    resign(&mut bad_version);
    let bad_version = write_fixture(&dir, "bad-version-v2.cfmovie", &bad_version)?;
    expect_movie_rejected(&bad_version, "unsupported movie format version 3")?;

    let mut bad_schema = valid.clone();
    bad_schema[10..12].copy_from_slice(&1_u16.to_le_bytes());
    resign(&mut bad_schema);
    let bad_schema = write_fixture(&dir, "bad-schema-v2.cfmovie", &bad_schema)?;
    expect_movie_rejected(&bad_schema, "unsupported movie input schema version 1")?;

    let mut corruption = valid.clone();
    let last = corruption.len() - 1;
    corruption[last] ^= 0xff;
    let corruption = write_fixture(&dir, "checksum-v2.cfmovie", &corruption)?;
    expect_movie_rejected(&corruption, "movie checksum mismatch")?;

    // Corrupt the first anchor's canonical snapshot and rebuild the trailing
    // checksum so the anchor parser is the rejecting validator.
    let name_len = movie.meta().name().map_or(0, str::len);
    let anchor_offset = 70 + name_len + movie.snapshot_bytes().len();
    let mut bad_anchor = valid.clone();
    bad_anchor[anchor_offset + 10] ^= 0xff;
    resign(&mut bad_anchor);
    let bad_anchor = write_fixture(&dir, "bad-anchor-v2.cfmovie", &bad_anchor)?;
    expect_movie_rejected(&bad_anchor, "invalid anchor 0 snapshot")?;
    // `play` reports the same typed rejection and also keeps stdout empty.
    expect_failure(
        &["tas", "play", "--movie", path_str(&bad_anchor)?],
        "invalid anchor 0 snapshot",
    )?;

    // A v2 file below the 94-byte v2 minimum but above the shared 86-byte
    // minimum is rejected by the v2 branch after magic dispatch.
    let mut short = valid[..58].to_vec();
    short.extend_from_slice(&[0; 32]);
    resign(&mut short);
    let short = write_fixture(&dir, "short-v2.cfmovie", &short)?;
    expect_movie_rejected(&short, "minimum is 94")?;

    // A structurally truncated v2 body with a rebuilt checksum reaches the
    // declared-count/remaining-bytes check before any allocation.
    let mut truncated = valid[..valid.len() - 32 - 10].to_vec();
    truncated.extend_from_slice(&[0; 32]);
    resign(&mut truncated);
    let truncated = write_fixture(&dir, "truncated-v2.cfmovie", &truncated)?;
    expect_movie_rejected(&truncated, "movie is truncated")?;

    // The intact v2 movie still verifies and still reports its anchors.
    let verify = block(&ok(&["tas", "verify", "--movie", path_str(&movie_path)?])?)?;
    assert_eq!(field(&verify, "verified")?, "1");
    assert_eq!(
        field(&verify, "final_state_hash")?,
        movie.final_state_hash().to_string()
    );
    Ok(())
}
