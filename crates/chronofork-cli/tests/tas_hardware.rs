//! Process-level acceptance tests for the ADR-0009 `--rom` backend selection.
//!
//! Every CLI invocation spawns the built `chronofork` binary with `Command`, so a
//! record, verify, play, studio or edit run is a separate operating-system
//! process. The command line, the exit status and the two output streams are the
//! whole contract observed here; movie encoding, checkpoint import, replay and the
//! compatibility predicate stay owned and tested by `chronofork-sessions`.
//!
//! The cartridge is the independently authored [`controller_rom`] fixture from
//! `chronofork-megadrive`: a deterministic 68000 program that reads controller
//! ports 1 and 2 and stores the observed bytes in work RAM, so input changes
//! canonical state. No commercial ROM, BIOS or copyrighted asset is used, and no
//! test writes a ROM or a source movie file.
//!
//! The tests return `Result` instead of panicking helpers so the workspace
//! `expect_used`/`unwrap_used` lints are satisfied without a file-level allowance.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_core::{InputFrame, hash_bytes};
use chronofork_megadrive::{MegadriveFactory, ROM_END, controller_rom};
use chronofork_sessions::SessionManager;

type TestResult = Result<(), Box<dyn Error>>;
/// One parsed `key=value` record block.
type Block = BTreeMap<String, String>;

/// Creates (and clears) a per-test temporary directory for CLI artifacts.
fn temp_dir(name: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "chronofork-cli-hardware-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn path_str(path: &Path) -> Result<&str, Box<dyn Error>> {
    match path.to_str() {
        Some(value) => Ok(value),
        None => Err(format!("temporary path is not UTF-8: {}", path.display()).into()),
    }
}

/// Runs the built CLI binary without stdin.
fn run(args: &[&str]) -> io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .output()
}

/// Runs the built CLI binary with a fixed stdin script.
fn run_with_stdin(args: &[&str], script: &str) -> Result<Output, Box<dyn Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    match child.stdin.take() {
        Some(mut stdin) => {
            stdin.write_all(script.as_bytes())?;
        }
        None => return Err("child stdin was not piped".into()),
    }
    Ok(child.wait_with_output()?)
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

/// Runs the CLI, requiring exit 1, empty stdout and one `chronofork:` diagnostic.
///
/// The stderr diagnostic is returned for a caller that checks its typed content.
fn fail(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(1) {
        return Err(format!(
            "expected exit code 1, got {:?} for {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        )
        .into());
    }
    if !stdout.is_empty() {
        return Err(format!("a typed failure must print empty stdout: {args:?}\n{stdout}").into());
    }
    if !stderr.starts_with("chronofork: ") {
        return Err(
            format!("diagnostics must start with `chronofork: `: {args:?}\n{stderr}").into(),
        );
    }
    Ok(stderr)
}

/// Parses the documented blank-line-separated `key=value` block format.
fn parse_blocks(stdout: &str) -> Result<Vec<Block>, Box<dyn Error>> {
    if stdout.is_empty() {
        return Err("expected at least one record block, got empty stdout".into());
    }
    if !stdout.ends_with('\n') || stdout.ends_with("\n\n") {
        return Err(format!("blocks end with exactly one newline:\n{stdout}").into());
    }
    let mut blocks = Vec::new();
    for chunk in stdout.split("\n\n") {
        let mut block = Block::new();
        for line in chunk.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("malformed output line `{line}`:\n{stdout}").into());
            };
            if block.insert(key.to_owned(), value.to_owned()).is_some() {
                return Err(format!("duplicate field `{key}`:\n{stdout}").into());
            }
        }
        if block.is_empty() {
            return Err(format!("empty record block:\n{stdout}").into());
        }
        blocks.push(block);
    }
    Ok(blocks)
}

fn single_block(stdout: &str) -> Result<Block, Box<dyn Error>> {
    let mut blocks = parse_blocks(stdout)?;
    let count = blocks.len();
    if count != 1 {
        return Err(format!("expected exactly one block, got {count}:\n{stdout}").into());
    }
    match blocks.pop() {
        Some(block) => Ok(block),
        None => Err("expected one block, got none".into()),
    }
}

fn field<'a>(block: &'a Block, key: &str) -> Result<&'a str, Box<dyn Error>> {
    match block.get(key) {
        Some(value) => Ok(value.as_str()),
        None => Err(format!("missing field `{key}` in block {block:?}").into()),
    }
}

fn record_block(block: &Block, expected: &str) -> Result<(), Box<dyn Error>> {
    if field(block, "record")? != expected {
        return Err(format!(
            "expected `record={expected}`, got `{}`",
            field(block, "record")?
        )
        .into());
    }
    Ok(())
}

/// Writes the independently authored controller-reading ROM into `dir`.
fn write_controller_rom(dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join("controller.rom");
    std::fs::write(&path, controller_rom())?;
    Ok(path)
}

/// Captures every regular file name and its bytes, to prove a command wrote nothing.
fn dir_snapshot(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, Box<dyn Error>> {
    let mut entries = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            return Err(format!("unexpected non-file entry `{}`", entry.path().display()).into());
        }
        let name = entry.file_name().into_string().map_err(|_| {
            format!(
                "temporary file name is not UTF-8: {}",
                entry.path().display()
            )
        })?;
        entries.insert(name, std::fs::read(entry.path())?);
    }
    Ok(entries)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[test]
fn hardware_record_play_verify_and_rom_non_mutation() -> TestResult {
    let dir = temp_dir("record")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let rom_hash = hash_bytes(&std::fs::read(&rom)?);
    let movie = dir.join("hardware.cfmovie");
    let movie_path = path_str(&movie)?;
    let again = dir.join("hardware-again.cfmovie");
    let again_path = path_str(&again)?;

    let recorded = single_block(&ok(&[
        "tas", "record", "--rom", rom_path, "--movie", movie_path, "--steps", "64", "--input", "17",
    ])?)?;
    record_block(&recorded, "movie")?;
    assert_eq!(field(&recorded, "movie_version")?, "1");
    assert_eq!(field(&recorded, "input_schema")?, "1");
    assert_eq!(field(&recorded, "steps")?, "64");
    assert_eq!(field(&recorded, "start_tick")?, "0");
    assert_eq!(field(&recorded, "end_tick")?, "5404");
    let final_hash = field(&recorded, "final_state_hash")?;
    let movie_hash = field(&recorded, "movie_sha256")?;

    // The ROM is opened read-only; a successful record does not mutate it.
    assert_eq!(hash_bytes(&std::fs::read(&rom)?), rom_hash);
    // Valid stdout is data only: no host path appears in the block.
    let recorded_stdout = ok(&[
        "tas", "record", "--rom", rom_path, "--movie", again_path, "--steps", "64", "--input", "17",
    ])?;
    assert!(
        !recorded_stdout.contains(rom_path)
            && !recorded_stdout.contains(&dir.display().to_string()),
        "valid stdout must not contain host paths:\n{recorded_stdout}"
    );

    // A second fresh process with the same ROM, steps and mask is byte-identical.
    let repeated = single_block(&recorded_stdout)?;
    assert_eq!(field(&repeated, "movie_sha256")?, movie_hash);
    assert_eq!(std::fs::read(&movie)?, std::fs::read(&again)?);

    // verify and play import the embedded checkpoint in new processes.
    let verified = single_block(&ok(&[
        "tas", "verify", "--rom", rom_path, "--movie", movie_path,
    ])?)?;
    record_block(&verified, "verification")?;
    assert_eq!(field(&verified, "verified")?, "1");
    assert_eq!(
        field(&verified, "start_tick")?,
        field(&recorded, "start_tick")?
    );
    assert_eq!(field(&verified, "end_tick")?, field(&recorded, "end_tick")?);
    assert_eq!(field(&verified, "steps")?, "64");
    assert_eq!(field(&verified, "final_tick")?, "5404");
    assert_eq!(field(&verified, "final_state_hash")?, final_hash);

    let played = single_block(&ok(&[
        "tas", "play", "--rom", rom_path, "--movie", movie_path,
    ])?)?;
    record_block(&played, "playback")?;
    assert_eq!(played.get("verified"), None);
    assert_eq!(field(&played, "final_tick")?, "5404");
    assert_eq!(field(&played, "final_state_hash")?, final_hash);
    Ok(())
}

#[test]
fn controller_input_changes_state_and_compare_is_backend_agnostic() -> TestResult {
    let dir = temp_dir("input")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let neutral_movie = dir.join("neutral.cfmovie");
    let neutral_path = path_str(&neutral_movie)?;
    let pressed_movie = dir.join("pressed.cfmovie");
    let pressed_path = path_str(&pressed_movie)?;

    let neutral = single_block(&ok(&[
        "tas",
        "record",
        "--rom",
        rom_path,
        "--movie",
        neutral_path,
        "--steps",
        "64",
        "--input",
        "0",
    ])?)?;
    let pressed = single_block(&ok(&[
        "tas",
        "record",
        "--rom",
        rom_path,
        "--movie",
        pressed_path,
        "--steps",
        "64",
        "--input",
        "17",
    ])?)?;
    assert_ne!(
        field(&neutral, "final_state_hash")?,
        field(&pressed, "final_state_hash")?,
        "port-1 input must change canonical state"
    );
    assert_ne!(
        field(&neutral, "movie_sha256")?,
        field(&pressed, "movie_sha256")?
    );

    // Both recordings verify on their own.
    assert_eq!(
        field(
            &single_block(&ok(&[
                "tas",
                "verify",
                "--rom",
                rom_path,
                "--movie",
                neutral_path
            ])?)?,
            "final_state_hash"
        )?,
        field(&neutral, "final_state_hash")?
    );
    assert_eq!(
        field(
            &single_block(&ok(&[
                "tas",
                "verify",
                "--rom",
                rom_path,
                "--movie",
                pressed_path
            ])?)?,
            "final_state_hash"
        )?,
        field(&pressed, "final_state_hash")?
    );

    // compare needs no ROM: it diffs recorded expectations only. The first
    // divergence is the first executed controller read (step index 4), because
    // the first four instructions only clear registers.
    let output = run(&[
        "tas",
        "compare",
        "--movie",
        neutral_path,
        "--against",
        pressed_path,
    ])?;
    let (stdout, stderr) = streams(&output)?;
    assert_eq!(output.status.code(), Some(1));
    let comparison = single_block(&stdout)?;
    record_block(&comparison, "comparison")?;
    assert_eq!(field(&comparison, "identical")?, "0");
    assert_eq!(field(&comparison, "diverged")?, "1");
    assert_eq!(field(&comparison, "divergence_index")?, "4");
    assert_eq!(field(&comparison, "steps_a")?, "64");
    assert_eq!(field(&comparison, "steps_b")?, "64");
    assert!(
        stderr.contains("recorded branch divergence at index 4"),
        "compare must report the first divergence: {stderr}"
    );

    // `--rom` is an unknown option on the backend-agnostic compare command.
    let stderr = fail(&[
        "tas",
        "compare",
        "--rom",
        rom_path,
        "--movie",
        neutral_path,
        "--against",
        pressed_path,
    ])?;
    assert!(
        stderr.contains("unknown `tas compare` option `--rom`"),
        "compare must reject --rom: {stderr}"
    );
    Ok(())
}

#[test]
fn two_controller_mapping_and_inert_schema2_fields() -> TestResult {
    let dir = temp_dir("two-controller")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let p1_movie = dir.join("p1.cfmovie");
    let p1_path = path_str(&p1_movie)?;
    let swapped_movie = dir.join("swapped.cfmovie");
    let swapped_path = path_str(&swapped_movie)?;
    let inert_movie = dir.join("inert.cfmovie");
    let inert_path = path_str(&inert_movie)?;

    // Port 1 (Up) and port 2 (C) through the schema-2 multi-device record path.
    let p1 = single_block(&ok(&[
        "tas", "record", "--rom", rom_path, "--movie", p1_path, "--steps", "64", "--input", "1",
        "--ports", "32,0,0",
    ])?)?;
    assert_eq!(field(&p1, "movie_version")?, "2");
    assert_eq!(field(&p1, "input_schema")?, "2");
    assert_eq!(
        field(
            &single_block(&ok(&[
                "tas", "verify", "--rom", rom_path, "--movie", p1_path
            ])?)?,
            "final_state_hash"
        )?,
        field(&p1, "final_state_hash")?
    );

    // Swapping the two controller masks changes the recorded state.
    let swapped = single_block(&ok(&[
        "tas",
        "record",
        "--rom",
        rom_path,
        "--movie",
        swapped_path,
        "--steps",
        "64",
        "--input",
        "32",
        "--ports",
        "1,0,0",
    ])?)?;
    assert_ne!(
        field(&p1, "final_state_hash")?,
        field(&swapped, "final_state_hash")?
    );

    // Non-neutral ports 2/3, axes and keys are recorded but inert: the canonical
    // state and every recorded expectation match the same port-1/port-2 run.
    let inert = single_block(&ok(&[
        "tas", "record", "--rom", rom_path, "--movie", inert_path, "--steps", "64", "--input", "1",
        "--ports", "32,0,0", "--axes", "1,2,3,4", "--keys", "255",
    ])?)?;
    assert_eq!(
        field(&inert, "final_state_hash")?,
        field(&p1, "final_state_hash")?
    );
    let identical = single_block(&ok(&[
        "tas",
        "compare",
        "--movie",
        p1_path,
        "--against",
        inert_path,
    ])?)?;
    record_block(&identical, "comparison")?;
    assert_eq!(field(&identical, "identical")?, "1");

    // A and Start are accepted and recorded but not readable by the TH-high
    // subset, so they change no canonical state.
    let mask_movie = dir.join("mask.cfmovie");
    let mask_path = path_str(&mask_movie)?;
    let neutral = single_block(&ok(&[
        "tas", "record", "--rom", rom_path, "--movie", mask_path, "--steps", "64", "--input", "0",
    ])?)?;
    let a_only = single_block(&ok(&[
        "tas", "record", "--rom", rom_path, "--movie", mask_path, "--steps", "64", "--input", "64",
    ])?)?;
    let start_only = single_block(&ok(&[
        "tas", "record", "--rom", rom_path, "--movie", mask_path, "--steps", "64", "--input", "128",
    ])?)?;
    assert_eq!(
        field(&a_only, "final_state_hash")?,
        field(&neutral, "final_state_hash")?
    );
    assert_eq!(
        field(&start_only, "final_state_hash")?,
        field(&neutral, "final_state_hash")?
    );
    Ok(())
}

#[test]
fn missing_short_large_and_non_file_roms_fail_typed() -> TestResult {
    let dir = temp_dir("rom-errors")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let movie = dir.join("unused.cfmovie");
    let movie_path = path_str(&movie)?;
    // A failed record must not replace an existing movie file.
    std::fs::write(&movie, b"sentinel")?;

    let missing = dir.join("missing.rom");
    let stderr = fail(&[
        "tas",
        "record",
        "--rom",
        path_str(&missing)?,
        "--movie",
        movie_path,
        "--steps",
        "1",
    ])?;
    assert!(stderr.contains("failed to open ROM"), "{stderr}");
    assert_eq!(std::fs::read(&movie)?, b"sentinel");

    let stderr = fail(&[
        "tas",
        "record",
        "--rom",
        path_str(&dir)?,
        "--movie",
        movie_path,
        "--steps",
        "1",
    ])?;
    assert!(stderr.contains("is not a regular file"), "{stderr}");

    let short = dir.join("short.rom");
    std::fs::write(&short, [0x00_u8, 0x01, 0x02, 0x03])?;
    let stderr = fail(&[
        "tas",
        "record",
        "--rom",
        path_str(&short)?,
        "--movie",
        movie_path,
        "--steps",
        "1",
    ])?;
    assert!(stderr.contains("512-byte header"), "{stderr}");

    // A sparse file larger than the 4 MiB window is rejected from metadata before
    // any read or factory allocation.
    let large = dir.join("large.rom");
    let file = File::create(&large)?;
    file.set_len(u64::from(ROM_END) + 1)?;
    let stderr = fail(&[
        "tas",
        "record",
        "--rom",
        path_str(&large)?,
        "--movie",
        movie_path,
        "--steps",
        "1",
    ])?;
    assert!(
        stderr.contains("exceeding the 4194304-byte (4 MiB)"),
        "{stderr}"
    );
    assert_eq!(std::fs::read(&movie)?, b"sentinel");

    // `--rom` with durable storage is rejected before any storage access.
    let stderr = fail(&[
        "tas",
        "record",
        "--rom",
        rom_path,
        "--root",
        path_str(&dir.join("store"))?,
        "--movie",
        movie_path,
        "--steps",
        "1",
    ])?;
    assert!(
        stderr.contains("cannot be combined with `--root`"),
        "{stderr}"
    );
    assert_eq!(std::fs::read(&movie)?, b"sentinel");
    Ok(())
}

#[cfg(unix)]
#[test]
fn unreadable_rom_is_a_typed_failure() -> TestResult {
    let dir = temp_dir("unreadable")?;
    let rom = write_controller_rom(&dir)?;
    set_mode(&rom, 0o000)?;
    // A privileged process (for example CI running as root) can read any file;
    // the premise of this test is unavailable there, so restore and stop.
    if File::open(&rom).is_ok() {
        set_mode(&rom, 0o600)?;
        return Ok(());
    }
    let movie = dir.join("unused.cfmovie");
    let stderr = fail(&[
        "tas",
        "record",
        "--rom",
        path_str(&rom)?,
        "--movie",
        path_str(&movie)?,
        "--steps",
        "1",
    ])?;
    assert!(stderr.contains("failed to open ROM"), "{stderr}");
    assert!(!movie.exists(), "a rejected record must not create a movie");
    set_mode(&rom, 0o600)?;
    Ok(())
}

#[test]
fn wrong_rom_and_incompatible_checkpoints_fail_typed() -> TestResult {
    let dir = temp_dir("wrong-rom")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let hardware_movie = dir.join("hardware.cfmovie");
    let hardware_path = path_str(&hardware_movie)?;
    ok(&[
        "tas",
        "record",
        "--rom",
        rom_path,
        "--movie",
        hardware_path,
        "--steps",
        "32",
        "--input",
        "1",
    ])?;

    // A second valid ROM of the same size has a different configuration hash.
    let other_rom = dir.join("other.rom");
    let mut other_bytes = controller_rom();
    // Change one program byte without touching the reset vectors.
    other_bytes[0x203] ^= 0x01;
    std::fs::write(&other_rom, &other_bytes)?;
    let other_path = path_str(&other_rom)?;

    // A hardware movie without --rom cannot import into the synthetic factory.
    let stderr = fail(&["tas", "verify", "--movie", hardware_path])?;
    assert!(stderr.contains("incompatible snapshot"), "{stderr}");
    // A different valid ROM is the same typed incompatibility.
    require_incompatible(&[
        "tas",
        "verify",
        "--rom",
        other_path,
        "--movie",
        hardware_path,
    ])?;
    let stderr = fail(&["tas", "play", "--rom", other_path, "--movie", hardware_path])?;
    assert!(stderr.contains("incompatible snapshot"), "{stderr}");

    // studio and edit use the same import path.
    let stderr = fail(&["tas", "studio", "--movie", hardware_path])?;
    assert!(stderr.contains("incompatible snapshot"), "{stderr}");
    let branch = dir.join("branch.cfmovie");
    let stderr = fail(&[
        "tas",
        "edit",
        "--movie",
        hardware_path,
        "--out",
        path_str(&branch)?,
        "--set-input",
        "0=1",
    ])?;
    assert!(stderr.contains("incompatible snapshot"), "{stderr}");
    assert!(!branch.exists());

    // A synthetic movie is incompatible with --rom in the same typed way.
    let synthetic_movie = dir.join("synthetic.cfmovie");
    let synthetic_path = path_str(&synthetic_movie)?;
    ok(&[
        "tas",
        "record",
        "--movie",
        synthetic_path,
        "--steps",
        "4",
        "--input",
        "0",
    ])?;
    let stderr = fail(&[
        "tas",
        "verify",
        "--rom",
        rom_path,
        "--movie",
        synthetic_path,
    ])?;
    assert!(stderr.contains("incompatible snapshot"), "{stderr}");
    let studio_stderr = fail(&[
        "tas",
        "studio",
        "--rom",
        rom_path,
        "--movie",
        synthetic_path,
    ])?;
    assert!(
        studio_stderr.contains("incompatible snapshot"),
        "{studio_stderr}"
    );
    let stderr = fail(&[
        "tas",
        "edit",
        "--rom",
        rom_path,
        "--movie",
        synthetic_path,
        "--out",
        path_str(&dir.join("synthetic-branch.cfmovie"))?,
        "--set-input",
        "0=1",
    ])?;
    assert!(stderr.contains("incompatible snapshot"), "{stderr}");
    Ok(())
}

/// Reads a captured `incompatible snapshot` diagnostic from one CLI failure.
fn require_incompatible(args: &[&str]) -> TestResult {
    let stderr = fail(args)?;
    if !stderr.contains("incompatible snapshot") {
        return Err(format!("expected a typed incompatible snapshot, got: {stderr}").into());
    }
    Ok(())
}

#[test]
fn studio_rewind_matches_fresh_replay_and_writes_nothing() -> TestResult {
    let dir = temp_dir("studio")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let movie = dir.join("studio.cfmovie");
    let movie_path = path_str(&movie)?;
    let recorded = single_block(&ok(&[
        "tas", "record", "--rom", rom_path, "--movie", movie_path, "--steps", "48", "--input", "1",
    ])?)?;
    let before = dir_snapshot(&dir)?;
    let movie_hash = hash_bytes(&std::fs::read(&movie)?);

    // A fresh ranged replay defines the expected state at the rewind target.
    let replayed = single_block(&ok(&[
        "tas", "play", "--rom", rom_path, "--movie", movie_path, "--until", "1848",
    ])?)?;
    let target_hash = field(&replayed, "final_state_hash")?;

    let output = run_with_stdin(
        &["tas", "studio", "--rom", rom_path, "--movie", movie_path],
        "state\ngoto 1848\nback\ngoto 4032\nquit\n",
    )?;
    let (stdout, stderr) = streams(&output)?;
    if !output.status.success() {
        return Err(format!("studio failed\nstdout:\n{stdout}\nstderr:\n{stderr}").into());
    }
    if !stderr.is_empty() {
        return Err(format!("studio wrote to stderr:\n{stderr}").into());
    }
    let blocks = parse_blocks(&stdout)?;
    if blocks.len() != 5 {
        return Err(format!(
            "expected five studio blocks, got {}:\n{stdout}",
            blocks.len()
        )
        .into());
    }
    record_block(&blocks[0], "state")?;
    assert_eq!(field(&blocks[0], "movie_version")?, "1");
    assert_eq!(field(&blocks[0], "input_schema")?, "1");
    assert_eq!(field(&blocks[0], "start_tick")?, "0");
    assert_eq!(field(&blocks[0], "end_tick")?, "4032");
    assert_eq!(field(&blocks[0], "steps")?, "48");
    assert_eq!(field(&blocks[0], "tick")?, "0");

    record_block(&blocks[1], "seek")?;
    assert_eq!(field(&blocks[1], "tick")?, "1848");
    assert_eq!(field(&blocks[1], "target_tick")?, "1848");
    assert_eq!(field(&blocks[1], "state_hash")?, target_hash);

    record_block(&blocks[2], "seek")?;
    assert_eq!(field(&blocks[2], "tick")?, "1820");

    record_block(&blocks[3], "seek")?;
    assert_eq!(field(&blocks[3], "tick")?, "4032");
    assert_eq!(
        field(&blocks[3], "state_hash")?,
        field(&recorded, "final_state_hash")?
    );

    record_block(&blocks[4], "quit")?;
    assert_eq!(field(&blocks[4], "reason")?, "command");
    assert_eq!(field(&blocks[4], "tick")?, "4032");

    // studio writes nothing and never rewrites the movie; the movie still verifies.
    assert_eq!(hash_bytes(&std::fs::read(&movie)?), movie_hash);
    assert_eq!(dir_snapshot(&dir)?, before);
    let verified = single_block(&ok(&[
        "tas", "verify", "--rom", rom_path, "--movie", movie_path,
    ])?)?;
    assert_eq!(field(&verified, "verified")?, "1");
    let contains_host = stdout.contains(movie_path) || stdout.contains(rom_path);
    assert!(
        !contains_host,
        "studio stdout must not contain host paths:\n{stdout}"
    );
    Ok(())
}

#[test]
fn edit_hardware_movie_with_rom() -> TestResult {
    let dir = temp_dir("edit")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let source = dir.join("source.cfmovie");
    let source_path = path_str(&source)?;
    let branch = dir.join("branch.cfmovie");
    let branch_path = path_str(&branch)?;
    let recorded = single_block(&ok(&[
        "tas",
        "record",
        "--rom",
        rom_path,
        "--movie",
        source_path,
        "--steps",
        "64",
        "--input",
        "0",
    ])?)?;
    let source_hash = field(&recorded, "movie_sha256")?.to_owned();

    // Step 13 is a port-1 read in the controller ROM loop, so editing it must
    // change the re-simulated canonical state.
    let edited = single_block(&ok(&[
        "tas",
        "edit",
        "--rom",
        rom_path,
        "--movie",
        source_path,
        "--out",
        branch_path,
        "--set-input",
        "13=17",
    ])?)?;
    record_block(&edited, "edit")?;
    assert_eq!(field(&edited, "movie_version")?, "1");
    assert_eq!(field(&edited, "input_schema")?, "1");
    assert_eq!(field(&edited, "edits")?, "1");
    assert_eq!(field(&edited, "prefix_verified")?, "1");
    assert_eq!(
        field(&edited, "parent_movie_id")?,
        field(&recorded, "movie_id")?
    );
    assert_eq!(field(&edited, "source_movie_sha256")?, source_hash);
    assert_ne!(
        field(&edited, "final_state_hash")?,
        field(&recorded, "final_state_hash")?
    );

    // The source file is never mutated and the branch verifies with the same ROM.
    assert_eq!(
        hash_bytes(&std::fs::read(&source)?).to_string(),
        source_hash
    );
    let verified = single_block(&ok(&[
        "tas",
        "verify",
        "--rom",
        rom_path,
        "--movie",
        branch_path,
    ])?)?;
    assert_eq!(field(&verified, "verified")?, "1");
    assert_eq!(
        field(&verified, "final_state_hash")?,
        field(&edited, "final_state_hash")?
    );
    Ok(())
}

#[test]
fn synthetic_default_commands_stay_on_the_unchanged_path() -> TestResult {
    let dir = temp_dir("synthetic")?;
    let movie = dir.join("synthetic.cfmovie");
    let movie_path = path_str(&movie)?;
    let recorded = single_block(&ok(&[
        "tas", "record", "--movie", movie_path, "--steps", "8", "--input", "3",
    ])?)?;
    record_block(&recorded, "movie")?;
    assert_eq!(field(&recorded, "movie_version")?, "1");
    assert_eq!(field(&recorded, "input_schema")?, "1");
    assert_eq!(field(&recorded, "steps")?, "8");

    let verified = single_block(&ok(&["tas", "verify", "--movie", movie_path])?)?;
    record_block(&verified, "verification")?;
    assert_eq!(field(&verified, "verified")?, "1");
    assert_eq!(
        field(&verified, "final_state_hash")?,
        field(&recorded, "final_state_hash")?
    );
    let played = single_block(&ok(&["tas", "play", "--movie", movie_path])?)?;
    record_block(&played, "playback")?;
    assert_eq!(
        field(&played, "final_state_hash")?,
        field(&recorded, "final_state_hash")?
    );

    // edit and studio keep the synthetic factory when `--rom` is absent.
    let branch = dir.join("synthetic-branch.cfmovie");
    let edited = single_block(&ok(&[
        "tas",
        "edit",
        "--movie",
        movie_path,
        "--out",
        path_str(&branch)?,
        "--set-input",
        "0=1",
    ])?)?;
    record_block(&edited, "edit")?;
    assert_eq!(field(&edited, "movie_version")?, "1");
    assert_eq!(
        field(&edited, "source_movie_sha256")?,
        field(&recorded, "movie_sha256")?
    );

    let output = run_with_stdin(&["tas", "studio", "--movie", movie_path], "state\nquit\n")?;
    let (stdout, stderr) = streams(&output)?;
    assert!(output.status.success(), "synthetic studio failed: {stderr}");
    assert!(stderr.is_empty(), "studio wrote to stderr: {stderr}");
    let blocks = parse_blocks(&stdout)?;
    assert_eq!(blocks.len(), 2);
    record_block(&blocks[0], "state")?;
    assert_eq!(field(&blocks[0], "movie_version")?, "1");
    record_block(&blocks[1], "quit")?;
    assert_eq!(field(&blocks[1], "reason")?, "command");
    Ok(())
}

#[test]
fn rom_flag_is_rejected_where_it_is_not_supported() -> TestResult {
    let dir = temp_dir("rom-flag")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let movie = dir.join("unused.cfmovie");
    let movie_path = path_str(&movie)?;

    // `demo` is unrelated to the TAS surface and takes no arguments at all.
    let stderr = fail(&["demo", "--rom", rom_path])?;
    assert!(stderr.contains("`demo` takes no arguments"), "{stderr}");

    // A repeated `--rom` is a typed CLI failure before any read.
    let stderr = fail(&[
        "tas", "record", "--rom", rom_path, "--rom", rom_path, "--movie", movie_path, "--steps",
        "1",
    ])?;
    assert!(
        stderr.contains("`--rom` was given more than once"),
        "{stderr}"
    );

    // A missing and an empty value are typed CLI failures too.
    let stderr = fail(&["tas", "verify", "--movie", movie_path, "--rom"])?;
    assert!(stderr.contains("`--rom` requires a value"), "{stderr}");
    let branch = dir.join("unused-branch.cfmovie");
    let stderr = fail(&[
        "tas",
        "edit",
        "--movie",
        movie_path,
        "--out",
        path_str(&branch)?,
        "--set-input",
        "0=1",
        "--rom",
        "",
    ])?;
    assert!(
        stderr.contains("`--rom` requires a non-empty value"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn session_snapshot_restore_and_fork_use_the_next_input() -> TestResult {
    let factory: Arc<dyn BackendFactory> = Arc::new(MegadriveFactory::new(controller_rom())?);
    let mut manager = SessionManager::new(factory);
    let source = manager.create_session()?;
    // The first four instructions only clear registers; the checkpoint is
    // input-independent and the next step is the first controller read.
    for _ in 0..4 {
        manager.step(source, InputFrame::NEUTRAL)?;
    }
    let checkpoint = manager.snapshot(source)?;
    let start_hash = manager.inspect(source)?.machine.state_hash;

    let pressed = InputFrame::new(0x11);
    let mut expected = Vec::new();
    for _ in 0..12 {
        expected.push(manager.step_verified(source, pressed)?.state_hash);
    }

    // Restoring the checkpoint and replaying the same frames reproduces every
    // state hash, because no input was serialized in the checkpoint.
    manager.restore(source, checkpoint)?;
    assert_eq!(manager.inspect(source)?.machine.state_hash, start_hash);
    for expected_hash in &expected {
        assert_eq!(
            manager.step_verified(source, pressed)?.state_hash,
            *expected_hash
        );
    }

    // Forks from the same checkpoint diverge at the first controller read when
    // their next frames differ; the source session is untouched.
    manager.restore(source, checkpoint)?;
    let fork_pressed = manager.fork_from_snapshot(source, checkpoint)?;
    let fork_neutral = manager.fork_from_snapshot(source, checkpoint)?;
    for expected_hash in &expected {
        assert_eq!(
            manager.step_verified(fork_pressed, pressed)?.state_hash,
            *expected_hash
        );
    }
    let neutral_first = manager
        .step_verified(fork_neutral, InputFrame::NEUTRAL)?
        .state_hash;
    assert_ne!(neutral_first, expected[0]);
    assert_eq!(manager.inspect(source)?.machine.state_hash, start_hash);
    Ok(())
}

#[test]
fn record_with_an_unusable_reset_vector_fails_typed_without_a_movie() -> TestResult {
    let dir = temp_dir("bad-vector")?;
    // The reset program counter points into the controller window, whose fetch is
    // a typed unmapped read, so the fresh session cannot execute step 0.
    let mut rom = controller_rom();
    // The hole between the cartridge window and the Z80 RAM is genuinely
    // unmapped; `0x00A10000` would no longer do, because the undecoded bytes of
    // the I/O window are deterministic open bus now.
    rom[4..8].copy_from_slice(&0x0080_0000_u32.to_be_bytes());
    let rom_path = dir.join("bad-vector.rom");
    std::fs::write(&rom_path, &rom)?;
    let movie = dir.join("unused.cfmovie");
    let stderr = fail(&[
        "tas",
        "record",
        "--rom",
        path_str(&rom_path)?,
        "--movie",
        path_str(&movie)?,
        "--steps",
        "1",
    ])?;
    assert!(stderr.contains("recording failed"), "{stderr}");
    assert!(stderr.contains("unmapped read"), "{stderr}");
    assert!(!movie.exists(), "a failed record must not create a movie");
    // The ROM was only read; its bytes are unchanged.
    assert_eq!(std::fs::read(&rom_path)?, rom);
    Ok(())
}

#[test]
fn hardware_v2_anchored_ranged_replay_stays_exact() -> TestResult {
    let dir = temp_dir("anchored")?;
    let rom = write_controller_rom(&dir)?;
    let rom_path = path_str(&rom)?;
    let movie = dir.join("anchored.cfmovie");
    let movie_path = path_str(&movie)?;
    let recorded = single_block(&ok(&[
        "tas",
        "record",
        "--rom",
        rom_path,
        "--movie",
        movie_path,
        "--steps",
        "48",
        "--input",
        "1",
        "--anchor-interval",
        "16",
    ])?)?;
    assert_eq!(field(&recorded, "movie_version")?, "2");
    assert_eq!(field(&recorded, "input_schema")?, "2");
    assert_eq!(field(&recorded, "anchors")?, "3");

    let verified = single_block(&ok(&[
        "tas", "verify", "--rom", rom_path, "--movie", movie_path,
    ])?)?;
    assert_eq!(field(&verified, "verified")?, "1");
    assert_eq!(
        field(&verified, "final_state_hash")?,
        field(&recorded, "final_state_hash")?
    );

    // A ranged replay starts at the nearest anchor at or before --from and
    // reports `prefix_verified=0`; the end state is still the recorded one.
    let ranged = single_block(&ok(&[
        "tas", "play", "--rom", rom_path, "--movie", movie_path, "--from", "1162", "--until",
        "4032",
    ])?)?;
    assert_eq!(field(&ranged, "anchored")?, "1");
    assert_eq!(field(&ranged, "anchor_tick")?, "1162");
    assert_eq!(field(&ranged, "prefix_verified")?, "0");
    assert_eq!(field(&ranged, "final_tick")?, "4032");
    assert_eq!(
        field(&ranged, "final_state_hash")?,
        field(&recorded, "final_state_hash")?
    );
    Ok(())
}
