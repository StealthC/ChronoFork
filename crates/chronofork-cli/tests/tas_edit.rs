//! Process-level acceptance tests for `chronofork tas edit`.
//!
//! Every invocation spawns the built `chronofork` binary with `Command::output`,
//! so the command line, exit status, stdout block, stderr diagnostic and the
//! filesystem effect are the whole contract observed here. The edit algorithm,
//! anchor rules and movie codec are owned and tested by the `chronofork-sessions`
//! suites; these tests check the ADR-0008 CLI surface, its strict parsing, the
//! source-path guard, atomic publish and the stable `record=edit` block.
//!
//! The tests return `Result` instead of panicking helpers so the workspace
//! `expect_used` lint is satisfied without a file-level allowance.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use chronofork_core::hash_bytes;
use chronofork_sessions::Movie;

type TestResult = Result<(), Box<dyn Error>>;
/// One parsed `key=value` record block.
type Block = BTreeMap<String, String>;

/// The exact `record=edit` field order fixed by ADR-0008 §7.
const EDIT_BLOCK_ORDER: [&str; 22] = [
    "record",
    "movie_version",
    "input_schema",
    "source_movie_id",
    "movie_id",
    "parent_movie_id",
    "name",
    "start_tick",
    "end_tick",
    "steps",
    "edits",
    "truncate_after",
    "prefix_steps",
    "resimulated_steps",
    "prefix_verified",
    "anchor_policy",
    "anchors_retained",
    "anchors_dropped",
    "anchors_rebuilt",
    "final_state_hash",
    "movie_sha256",
    "source_movie_sha256",
];

fn temp_dir(name: &str) -> io::Result<PathBuf> {
    let dir =
        std::env::temp_dir().join(format!("chronofork-cli-edit-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
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

fn run_in(dir: &Path, args: &[&str]) -> io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .current_dir(dir)
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
fn ok_output(output: Output, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(0) {
        return Err(
            format!("command failed: {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}").into(),
        );
    }
    if !stderr.is_empty() {
        return Err(format!("successful command wrote to stderr: {args:?}\n{stderr}").into());
    }
    Ok(stdout)
}

fn ok(args: &[&str]) -> Result<String, Box<dyn Error>> {
    ok_output(run(args)?, args)
}

fn ok_in(dir: &Path, args: &[&str]) -> Result<String, Box<dyn Error>> {
    ok_output(run_in(dir, args)?, args)
}

/// Requires a typed failure: exit 1, no stdout data and a `chronofork:` diagnostic.
fn check_failure(output: &Output, needle: &str, args: &[&str]) -> TestResult {
    let (stdout, stderr) = streams(output)?;
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

fn failure(args: &[&str], needle: &str) -> TestResult {
    let output = run(args)?;
    check_failure(&output, needle, args)
}

fn failure_in(dir: &Path, args: &[&str], needle: &str) -> TestResult {
    let output = run_in(dir, args)?;
    check_failure(&output, needle, args)
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

/// Returns the block field names in their printed order.
fn field_order(stdout: &str) -> Result<Vec<&str>, Box<dyn Error>> {
    let mut keys = Vec::new();
    for line in stdout.lines().filter(|line| !line.is_empty()) {
        let (key, _) = line
            .split_once('=')
            .ok_or_else(|| format!("every record line is `key=value`, got `{line}`"))?;
        keys.push(key);
    }
    Ok(keys)
}

fn assert_fields(block: &Block, expected: &[(&str, &str)]) -> TestResult {
    for (key, value) in expected {
        let actual = field(block, key)?;
        if actual != *value {
            return Err(format!("field `{key}`: expected `{value}`, got `{actual}`").into());
        }
    }
    Ok(())
}

fn assert_no_temp_files(dir: &Path) -> TestResult {
    let mut leftovers = Vec::new();
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.contains(".tmp-") {
            leftovers.push(name);
        }
    }
    if !leftovers.is_empty() {
        return Err(format!("temporary files were left behind: {leftovers:?}").into());
    }
    Ok(())
}

fn decode(path: &Path) -> Result<Movie, Box<dyn Error>> {
    Ok(Movie::from_bytes(&fs::read(path)?)?)
}

/// Records a format v1 source movie of a fixed port-0 mask.
fn record_v1(dir: &Path, name: &str, steps: u64, input: u32) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join(name);
    let steps = steps.to_string();
    let input = input.to_string();
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&path)?,
        "--steps",
        &steps,
        "--input",
        &input,
    ])?;
    Ok(path)
}

/// Records a format v2 source movie with anchors every `interval` steps.
fn record_v2(
    dir: &Path,
    name: &str,
    steps: u64,
    input: u32,
    interval: u64,
) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join(name);
    let steps = steps.to_string();
    let input = input.to_string();
    let interval = interval.to_string();
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&path)?,
        "--steps",
        &steps,
        "--input",
        &input,
        "--anchor-interval",
        &interval,
    ])?;
    Ok(path)
}

/// Records a format v2 source movie that carries non-neutral extra device fields.
fn record_v2_devices(dir: &Path, name: &str, steps: u64) -> Result<PathBuf, Box<dyn Error>> {
    let path = dir.join(name);
    let steps = steps.to_string();
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&path)?,
        "--steps",
        &steps,
        "--input",
        "1",
        "--ports",
        "5,6,7",
        "--axes",
        "1,-1,2,3",
        "--keys",
        "9",
    ])?;
    Ok(path)
}

/// Runs `tas compare` and returns its single divergence block (exit 1 expected).
fn comparison_block(args: &[&str]) -> Result<Block, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(1) {
        return Err(format!(
            "expected compare divergence (exit 1) for {args:?}, got {:?}\nstdout:\n{stdout}\n\
             stderr:\n{stderr}",
            output.status.code()
        )
        .into());
    }
    if !stderr.starts_with("chronofork: ") {
        return Err(format!("compare divergence needs a stderr diagnostic:\n{stderr}").into());
    }
    block(&stdout)
}

#[test]
fn single_edit_resimulates_the_suffix_and_keeps_the_source() -> TestResult {
    let dir = temp_dir("single")?;
    let source = record_v1(&dir, "source.cfmovie", 6, 1)?;
    let source_bytes = fs::read(&source)?;
    let out = dir.join("branch.cfmovie");

    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--set-input",
        "2=9",
    ])?;
    // The block has exactly the ADR fields, in the ADR order.
    if field_order(&stdout)? != EDIT_BLOCK_ORDER {
        return Err(format!("unexpected `record=edit` field order:\n{stdout}").into());
    }
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("record", "edit"),
            ("movie_version", "1"),
            ("input_schema", "1"),
            ("source_movie_id", "1"),
            ("movie_id", "2"),
            ("parent_movie_id", "1"),
            ("name", ""),
            ("start_tick", "0"),
            ("end_tick", "6"),
            ("steps", "6"),
            ("edits", "1"),
            ("truncate_after", ""),
            ("prefix_steps", "2"),
            ("resimulated_steps", "4"),
            ("prefix_verified", "1"),
            ("anchor_policy", "keep-prefix"),
            ("anchors_retained", "0"),
            ("anchors_dropped", "0"),
            ("anchors_rebuilt", "0"),
        ],
    )?;

    // The source file is byte-identical and the output hashes match the block.
    let output_bytes = fs::read(&out)?;
    if fs::read(&source)? != source_bytes {
        return Err("the edit changed the source file bytes".into());
    }
    assert_fields(
        &values,
        &[
            ("movie_sha256", &hash_bytes(&output_bytes).to_string()),
            (
                "source_movie_sha256",
                &hash_bytes(&source_bytes).to_string(),
            ),
        ],
    )?;

    // The decoded output agrees with the block and the edited frame is applied.
    let source_movie = decode(&source)?;
    let output_movie = decode(&out)?;
    assert_fields(
        &values,
        &[
            (
                "final_state_hash",
                &output_movie.final_state_hash().to_string(),
            ),
            ("end_tick", &output_movie.end_tick().raw().to_string()),
            ("steps", &output_movie.step_count().to_string()),
        ],
    )?;
    assert_eq!(output_movie.meta().movie_id(), 2);
    assert_eq!(output_movie.meta().parent_movie_id(), Some(1));
    assert_eq!(output_movie.meta().name(), None);
    let edited = output_movie
        .steps()
        .get(2)
        .ok_or("output movie is missing step 2")?;
    assert_eq!(edited.input().buttons(), 9);
    let prefix = output_movie
        .steps()
        .get(1)
        .ok_or("output movie is missing step 1")?;
    assert_eq!(
        prefix.input(),
        source_movie.steps().get(1).ok_or("missing")?.input()
    );

    // The output replays, and the recorded branch diverges at the edited step.
    ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    let comparison = comparison_block(&[
        "tas",
        "compare",
        "--movie",
        path_str(&source)?,
        "--against",
        path_str(&out)?,
    ])?;
    assert_fields(
        &comparison,
        &[
            ("divergence_index", "2"),
            ("steps_a", "6"),
            ("steps_b", "6"),
        ],
    )?;
    assert_no_temp_files(&dir)
}

#[test]
fn multi_step_edit_applies_every_frame_and_sets_branch_metadata() -> TestResult {
    let dir = temp_dir("multi")?;
    // The synthetic program consumes port 0 on recorded steps 2, 7, 12, ...;
    // editing those steps changes the re-simulated suffix.
    let source = record_v1(&dir, "source.cfmovie", 12, 1)?;
    let source_bytes = fs::read(&source)?;
    let out = dir.join("branch.cfmovie");

    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--set-input",
        "2=4",
        "--set-input",
        "7=7",
        "--movie-id",
        "99",
        "--name",
        "branch",
    ])?;
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("movie_id", "99"),
            ("parent_movie_id", "1"),
            ("source_movie_id", "1"),
            ("name", "branch"),
            ("edits", "2"),
            ("prefix_steps", "2"),
            ("resimulated_steps", "10"),
            ("steps", "12"),
            ("truncate_after", ""),
        ],
    )?;
    let output_movie = decode(&out)?;
    assert_eq!(output_movie.meta().name(), Some("branch"));
    assert_eq!(
        output_movie
            .steps()
            .get(2)
            .ok_or("missing step 2")?
            .input()
            .buttons(),
        4
    );
    assert_eq!(
        output_movie
            .steps()
            .get(7)
            .ok_or("missing step 7")?
            .input()
            .buttons(),
        7
    );
    // Untouched steps keep the source input.
    assert_eq!(
        output_movie
            .steps()
            .get(3)
            .ok_or("missing step 3")?
            .input()
            .buttons(),
        1
    );
    ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    let comparison = comparison_block(&[
        "tas",
        "compare",
        "--movie",
        path_str(&source)?,
        "--against",
        path_str(&out)?,
    ])?;
    assert_fields(
        &comparison,
        &[
            ("divergence_index", "2"),
            ("steps_a", "12"),
            ("steps_b", "12"),
        ],
    )?;
    if fs::read(&source)? != source_bytes {
        return Err("the multi-step edit changed the source file bytes".into());
    }
    assert_no_temp_files(&dir)
}

#[test]
fn truncation_retains_the_prefix_and_does_not_execute_the_dropped_suffix() -> TestResult {
    let dir = temp_dir("truncate")?;
    let source = record_v1(&dir, "source.cfmovie", 6, 1)?;
    let source_bytes = fs::read(&source)?;
    let source_movie = decode(&source)?;
    let out = dir.join("branch.cfmovie");

    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--truncate-after",
        "2",
    ])?;
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("steps", "3"),
            ("edits", "0"),
            ("truncate_after", "2"),
            ("prefix_steps", "3"),
            ("resimulated_steps", "0"),
            ("prefix_verified", "1"),
        ],
    )?;
    let retained_tick = source_movie
        .steps()
        .get(2)
        .ok_or("missing source step 2")?
        .outcome_tick();
    assert_fields(&values, &[("end_tick", &retained_tick.raw().to_string())])?;
    let output_movie = decode(&out)?;
    assert_eq!(output_movie.step_count(), 3);
    assert_eq!(output_movie.end_tick(), retained_tick);
    assert_eq!(
        output_movie.steps(),
        source_movie
            .steps()
            .get(..3)
            .ok_or("source prefix is shorter than three steps")?
    );
    ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    if fs::read(&source)? != source_bytes {
        return Err("truncation changed the source file bytes".into());
    }
    assert_no_temp_files(&dir)
}

#[test]
fn edit_and_truncation_combine_under_the_last_retained_step() -> TestResult {
    let dir = temp_dir("edit-truncate")?;
    let source = record_v1(&dir, "source.cfmovie", 6, 1)?;
    let out = dir.join("branch.cfmovie");
    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--truncate-after",
        "3",
        "--set-input",
        "2=5",
    ])?;
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("steps", "4"),
            ("edits", "1"),
            ("truncate_after", "3"),
            ("prefix_steps", "2"),
            ("resimulated_steps", "2"),
        ],
    )?;
    let output_movie = decode(&out)?;
    assert_eq!(output_movie.step_count(), 4);
    assert_eq!(
        output_movie
            .steps()
            .get(2)
            .ok_or("missing step 2")?
            .input()
            .buttons(),
        5
    );
    ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    Ok(())
}

#[test]
fn v2_keep_prefix_retains_only_anchors_before_the_edit() -> TestResult {
    let dir = temp_dir("v2-keep")?;
    // Eight steps with an anchor after steps 1, 3, 5 and 7.
    let source = record_v2(&dir, "source.cfmovie", 8, 1, 2)?;
    let source_bytes = fs::read(&source)?;
    let source_movie = decode(&source)?;
    assert_eq!(source_movie.anchors().len(), 4);
    let out = dir.join("branch.cfmovie");

    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--set-input",
        "3=9",
    ])?;
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("movie_version", "2"),
            ("input_schema", "2"),
            ("anchor_policy", "keep-prefix"),
            ("anchors_retained", "1"),
            ("anchors_dropped", "3"),
            ("anchors_rebuilt", "0"),
            ("prefix_steps", "3"),
            ("resimulated_steps", "5"),
        ],
    )?;
    let output_movie = decode(&out)?;
    assert_eq!(output_movie.format_version(), 2);
    assert_eq!(output_movie.anchors().len(), 1);
    let retained = output_movie
        .anchors()
        .first()
        .ok_or("output has no retained anchor")?;
    assert_eq!(retained.step_index(), 1);
    // The retained anchor is copied verbatim from the source.
    assert_eq!(Some(retained), source_movie.anchors().first());
    ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    if fs::read(&source)? != source_bytes {
        return Err("the v2 edit changed the source file bytes".into());
    }
    assert_no_temp_files(&dir)
}

#[test]
fn v2_drop_and_rebuild_replace_the_anchor_table() -> TestResult {
    let dir = temp_dir("v2-policies")?;
    let source = record_v2(&dir, "source.cfmovie", 8, 1, 2)?;
    let source_bytes = fs::read(&source)?;

    let dropped = dir.join("dropped.cfmovie");
    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&dropped)?,
        "--set-input",
        "3=9",
        "--anchors",
        "drop",
    ])?;
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("anchor_policy", "drop"),
            ("anchors_retained", "0"),
            ("anchors_dropped", "4"),
            ("anchors_rebuilt", "0"),
        ],
    )?;
    assert!(decode(&dropped)?.anchors().is_empty());
    ok(&["tas", "verify", "--movie", path_str(&dropped)?])?;

    let rebuilt = dir.join("rebuilt.cfmovie");
    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&rebuilt)?,
        "--set-input",
        "3=9",
        "--anchors",
        "rebuild",
        "--anchor-interval",
        "3",
    ])?;
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("anchor_policy", "rebuild"),
            ("anchors_retained", "0"),
            ("anchors_dropped", "4"),
            ("anchors_rebuilt", "2"),
        ],
    )?;
    let rebuilt_movie = decode(&rebuilt)?;
    let indices: Vec<usize> = rebuilt_movie
        .anchors()
        .iter()
        .map(|anchor| anchor.step_index())
        .collect();
    assert_eq!(indices, vec![2, 5]);
    ok(&["tas", "verify", "--movie", path_str(&rebuilt)?])?;
    if fs::read(&source)? != source_bytes {
        return Err("the anchor-policy edits changed the source file bytes".into());
    }
    assert_no_temp_files(&dir)
}

#[test]
fn v2_port_zero_edit_preserves_the_recorded_extra_device_fields() -> TestResult {
    let dir = temp_dir("v2-devices")?;
    let source = record_v2_devices(&dir, "source.cfmovie", 4)?;
    let source_movie = decode(&source)?;
    let out = dir.join("branch.cfmovie");
    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--set-input",
        "1=2",
    ])?;
    let values = block(&stdout)?;
    assert_fields(
        &values,
        &[
            ("movie_version", "2"),
            ("input_schema", "2"),
            ("edits", "1"),
            ("anchors_retained", "0"),
            ("anchors_dropped", "0"),
            ("anchor_policy", "keep-prefix"),
        ],
    )?;
    let output_movie = decode(&out)?;
    let edited = output_movie
        .steps()
        .get(1)
        .ok_or("missing output step 1")?
        .input();
    let recorded = source_movie
        .steps()
        .get(1)
        .ok_or("missing source step 1")?
        .input();
    assert_eq!(edited.ports(), [2, 5, 6, 7]);
    assert_eq!(edited.axes(), recorded.axes());
    assert_eq!(edited.keys(), recorded.keys());
    assert_eq!(
        output_movie
            .steps()
            .first()
            .ok_or("missing output step 0")?
            .input(),
        source_movie
            .steps()
            .first()
            .ok_or("missing source step 0")?
            .input()
    );
    ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    Ok(())
}

#[test]
fn explicit_anchor_flags_are_rejected_on_a_v1_source() -> TestResult {
    let dir = temp_dir("v1-anchors")?;
    let source = record_v1(&dir, "source.cfmovie", 4, 1)?;
    let source_bytes = fs::read(&source)?;
    let out = dir.join("branch.cfmovie");
    let source_str = path_str(&source)?;
    let out_str = path_str(&out)?;
    for extra in [
        vec!["--anchors", "keep-prefix"],
        vec!["--anchors", "drop"],
        vec!["--anchors", "rebuild", "--anchor-interval", "2"],
    ] {
        let mut args = vec![
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--set-input",
            "1=2",
        ];
        args.extend_from_slice(&extra);
        failure(&args, "v1 movie has no anchor table")?;
        if out.exists() {
            return Err(format!("a rejected v1 anchor edit created `{}`", out.display()).into());
        }
    }
    if fs::read(&source)? != source_bytes {
        return Err("a rejected v1 anchor edit changed the source file bytes".into());
    }
    assert_no_temp_files(&dir)
}

#[test]
fn missing_duplicate_unknown_and_empty_flags_are_typed_failures() -> TestResult {
    let dir = temp_dir("flags")?;
    let source = record_v1(&dir, "source.cfmovie", 4, 1)?;
    let source_str = path_str(&source)?;
    let out = dir.join("branch.cfmovie");
    let out_str = path_str(&out)?;

    failure(
        &["tas", "edit", "--out", out_str, "--set-input", "0=1"],
        "requires `--movie <FILE>`",
    )?;
    failure(
        &["tas", "edit", "--movie", source_str, "--set-input", "0=1"],
        "requires `--out <FILE>`",
    )?;
    failure(
        &[
            "tas", "edit", "--movie", source_str, "--out", out_str, "--name", "",
        ],
        "requires a non-empty value",
    )?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            "",
            "--out",
            out_str,
            "--set-input",
            "0=1",
        ],
        "requires a non-empty value",
    )?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--movie",
            source_str,
            "--set-input",
            "0=1",
        ],
        "was given more than once",
    )?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--from",
            "0",
            "--set-input",
            "0=1",
        ],
        "unknown `tas edit` option `--from`",
    )?;
    for foreign in [
        "positional",
        "--steps",
        "--root",
        "--session",
        "--snapshot",
        "--ports",
        "--axes",
        "--keys",
        "--until",
        "--against",
    ] {
        failure(
            &[
                "tas", "edit", "--movie", source_str, "--out", out_str, foreign,
            ],
            &format!("unknown `tas edit` option `{foreign}`"),
        )?;
    }
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--set-input",
            "0=1",
            "--help",
        ],
        "takes no other arguments",
    )?;
    if out.exists() {
        return Err("a rejected flag set created the output file".into());
    }
    assert_no_temp_files(&dir)
}

#[test]
fn malformed_numbers_and_request_bounds_are_typed_failures() -> TestResult {
    let dir = temp_dir("requests")?;
    let source = record_v1(&dir, "source.cfmovie", 6, 1)?;
    let source_bytes = fs::read(&source)?;
    let source_str = path_str(&source)?;
    let out = dir.join("branch.cfmovie");
    let out_str = path_str(&out)?;
    let base = ["tas", "edit", "--movie", source_str, "--out", out_str];

    let cases: [(&[&str], &str); 15] = [
        (
            &[],
            "must replace at least one input or truncate after a step",
        ),
        (&["--set-input", "1"], "expects `<INDEX=MASK>`"),
        (&["--set-input", "x=1"], "zero-based decimal step index"),
        (&["--set-input", "1=x"], "zero-based decimal step index"),
        (
            &["--set-input", "1=4294967296"],
            "zero-based decimal step index",
        ),
        (&["--set-input", "1=1", "--set-input", "1=2"], "is repeated"),
        (
            &["--set-input", "99=1"],
            "out of range for a movie with 6 step(s)",
        ),
        (&["--truncate-after", "abc"], "expects a decimal integer"),
        (
            &["--truncate-after", "99"],
            "truncate-after 99 is out of range",
        ),
        (&["--truncate-after", "5"], "drops no step"),
        (
            &["--truncate-after", "2", "--set-input", "3=1"],
            "beyond truncate-after 2",
        ),
        (
            &["--set-input", "0=1", "--movie-id", "1"],
            "equals the source movie id",
        ),
        (
            &["--set-input", "0=1", "--movie-id", "0"],
            "movie id must be non-zero",
        ),
        (
            &["--set-input", "0=1", "--anchors", "nope"],
            "expects `keep-prefix`, `drop` or `rebuild`",
        ),
        (
            &["--set-input", "0=1", "--anchor-interval", "2"],
            "valid only with `--anchors rebuild`",
        ),
    ];
    for (extra, needle) in cases {
        let mut args = base.to_vec();
        args.extend_from_slice(extra);
        failure(&args, needle)?;
        if out.exists() {
            return Err(format!("a rejected request created `{}`", out.display()).into());
        }
    }

    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--set-input",
            "0=1",
            "--anchors",
            "rebuild",
        ],
        "requires `--anchor-interval <N>`",
    )?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--set-input",
            "0=1",
            "--anchors",
            "keep-prefix",
            "--anchor-interval",
            "2",
        ],
        "valid only with `--anchors rebuild`",
    )?;
    // An anchor interval is validated against a v2 source (v1 rejects any
    // explicit anchor policy before the interval is considered).
    let v2_source = record_v2(&dir, "v2.cfmovie", 6, 1, 2)?;
    let v2_source_str = path_str(&v2_source)?;
    let v2_out = dir.join("v2-branch.cfmovie");
    let v2_out_str = path_str(&v2_out)?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            v2_source_str,
            "--out",
            v2_out_str,
            "--set-input",
            "0=1",
            "--anchors",
            "rebuild",
            "--anchor-interval",
            "0",
        ],
        "anchor rebuild interval 0 is invalid",
    )?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            v2_source_str,
            "--out",
            v2_out_str,
            "--set-input",
            "0=1",
            "--anchors",
            "rebuild",
            "--anchor-interval",
            "7",
        ],
        "anchor rebuild interval 7 is invalid",
    )?;
    let long_name = "a".repeat(257);
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--set-input",
            "0=1",
            "--name",
            &long_name,
        ],
        "limit is 256",
    )?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            out_str,
            "--set-input",
            "0=1",
            "--name",
            "bad\u{7}name",
        ],
        "control characters",
    )?;
    // The default branch id (source id + 1) is checked against overflow.
    let overflow_source = dir.join("overflow.cfmovie");
    ok(&[
        "tas",
        "record",
        "--movie",
        path_str(&overflow_source)?,
        "--steps",
        "2",
        "--input",
        "1",
        "--movie-id",
        "18446744073709551615",
    ])?;
    let overflow_out = dir.join("overflow-out.cfmovie");
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            path_str(&overflow_source)?,
            "--out",
            path_str(&overflow_out)?,
            "--set-input",
            "0=1",
        ],
        "default branch movie id overflows",
    )?;
    for rejected_output in [&out, &v2_out, &overflow_out] {
        if rejected_output.exists() {
            return Err(
                format!("a rejected request created `{}`", rejected_output.display()).into(),
            );
        }
    }
    if fs::read(&source)? != source_bytes {
        return Err("rejected requests changed the source file bytes".into());
    }
    assert_no_temp_files(&dir)
}

#[test]
fn output_path_aliases_of_the_source_are_rejected() -> TestResult {
    let dir = temp_dir("aliases")?;
    let source = record_v1(&dir, "source.cfmovie", 4, 1)?;
    let source_bytes = fs::read(&source)?;
    let source_str = path_str(&source)?;

    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            source_str,
            "--set-input",
            "1=2",
        ],
        "OutputAliasesSource",
    )?;
    // A lexically equal relative path and a `..` path are rejected too.
    failure_in(
        &dir,
        &[
            "tas",
            "edit",
            "--movie",
            "source.cfmovie",
            "--out",
            "./source.cfmovie",
            "--set-input",
            "1=2",
        ],
        "OutputAliasesSource",
    )?;
    failure_in(
        &dir,
        &[
            "tas",
            "edit",
            "--movie",
            "source.cfmovie",
            "--out",
            "missing/../source.cfmovie",
            "--set-input",
            "1=2",
        ],
        "OutputAliasesSource",
    )?;
    if fs::read(&source)? != source_bytes {
        return Err("an alias rejection changed the source file bytes".into());
    }
    assert_no_temp_files(&dir)
}

#[cfg(unix)]
#[test]
fn hard_link_and_symlink_aliases_of_the_source_are_rejected() -> TestResult {
    let dir = temp_dir("link-aliases")?;
    let source = record_v1(&dir, "source.cfmovie", 4, 1)?;
    let source_bytes = fs::read(&source)?;
    let source_str = path_str(&source)?;

    let hard_link = dir.join("hard-link.cfmovie");
    fs::hard_link(&source, &hard_link)?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            path_str(&hard_link)?,
            "--set-input",
            "1=2",
        ],
        "OutputAliasesSource",
    )?;
    let symlink = dir.join("symlink.cfmovie");
    std::os::unix::fs::symlink("source.cfmovie", &symlink)?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            path_str(&symlink)?,
            "--set-input",
            "1=2",
        ],
        "OutputAliasesSource",
    )?;
    if fs::read(&source)? != source_bytes {
        return Err("a link-alias rejection changed the source file bytes".into());
    }
    fs::symlink_metadata(&symlink)?;
    fs::metadata(&hard_link)?;
    assert_no_temp_files(&dir)
}

#[test]
fn publish_replaces_only_a_non_source_destination() -> TestResult {
    let dir = temp_dir("publish")?;
    let source = record_v1(&dir, "source.cfmovie", 4, 1)?;
    let source_bytes = fs::read(&source)?;
    let out = dir.join("destination.cfmovie");
    fs::write(&out, b"previous destination")?;

    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--set-input",
        "1=2",
    ])?;
    let values = block(&stdout)?;
    let output_bytes = fs::read(&out)?;
    assert_ne!(output_bytes.as_slice(), b"previous destination");
    assert_fields(
        &values,
        &[("movie_sha256", &hash_bytes(&output_bytes).to_string())],
    )?;
    if fs::read(&source)? != source_bytes {
        return Err("publishing replaced or changed the source file".into());
    }
    ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    assert_no_temp_files(&dir)
}

#[test]
fn failed_edits_leave_no_partial_output_or_temporary_file() -> TestResult {
    let dir = temp_dir("atomic-failure")?;
    let source = record_v1(&dir, "source.cfmovie", 4, 1)?;
    let source_bytes = fs::read(&source)?;
    let source_str = path_str(&source)?;

    // An existing non-source destination is untouched by a rejected request.
    let existing = dir.join("existing.cfmovie");
    fs::write(&existing, b"keep me")?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            path_str(&existing)?,
            "--set-input",
            "99=1",
        ],
        "out of range",
    )?;
    assert_eq!(fs::read(&existing)?.as_slice(), b"keep me");
    assert_no_temp_files(&dir)?;

    // An absent destination stays absent.
    let absent = dir.join("absent.cfmovie");
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            path_str(&absent)?,
            "--truncate-after",
            "99",
        ],
        "out of range",
    )?;
    if absent.exists() {
        return Err("a rejected edit created its output file".into());
    }
    assert_no_temp_files(&dir)?;

    // A directory destination is rejected before any temporary file exists.
    let directory = dir.join("as-directory");
    fs::create_dir(&directory)?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            path_str(&directory)?,
            "--set-input",
            "1=2",
        ],
        "must name a file, not a directory",
    )?;
    fs::metadata(&directory)?;
    assert_no_temp_files(&dir)?;

    // A missing parent directory is an I/O failure with no partial output.
    let missing_parent = dir.join("missing").join("out.cfmovie");
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            path_str(&missing_parent)?,
            "--set-input",
            "1=2",
        ],
        "failed to write movie",
    )?;
    if missing_parent.exists() {
        return Err("a failed publish created its output file".into());
    }
    assert_no_temp_files(&dir)?;

    // A parent path that is a regular file is also a typed I/O failure.
    let file_parent = dir.join("not-a-directory");
    fs::write(&file_parent, b"file")?;
    let nested = file_parent.join("out.cfmovie");
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            source_str,
            "--out",
            path_str(&nested)?,
            "--set-input",
            "1=2",
        ],
        "failed to write movie",
    )?;
    assert_no_temp_files(&dir)?;

    if fs::read(&source)? != source_bytes {
        return Err("failed edits changed the source file bytes".into());
    }
    Ok(())
}

#[test]
fn missing_or_unreadable_sources_are_typed_failures() -> TestResult {
    let dir = temp_dir("bad-source")?;
    let missing = dir.join("missing.cfmovie");
    let out = dir.join("out.cfmovie");
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            path_str(&missing)?,
            "--out",
            path_str(&out)?,
            "--set-input",
            "0=1",
        ],
        "failed to open movie",
    )?;
    if out.exists() {
        return Err("a missing source created an output file".into());
    }
    // A non-movie file is rejected by the one movie decoder, never edited.
    let garbage = dir.join("garbage.cfmovie");
    fs::write(&garbage, b"not a movie")?;
    let garbage_bytes = fs::read(&garbage)?;
    failure(
        &[
            "tas",
            "edit",
            "--movie",
            path_str(&garbage)?,
            "--out",
            path_str(&out)?,
            "--set-input",
            "0=1",
        ],
        "movie",
    )?;
    if fs::read(&garbage)? != garbage_bytes {
        return Err("a decode rejection changed the source file".into());
    }
    if out.exists() {
        return Err("a decode rejection created an output file".into());
    }
    assert_no_temp_files(&dir)
}

#[cfg(unix)]
#[test]
fn invalid_utf8_arguments_are_typed_failures_not_panics() -> TestResult {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = temp_dir("invalid-utf8")?;
    let source = record_v1(&dir, "source.cfmovie", 3, 1)?;
    let out = dir.join("branch.cfmovie");
    // `0x80` is not valid UTF-8 on its own. It is passed as the `--name` value.
    let invalid = OsString::from_vec(vec![0xff, 0xfe]);
    let args = ["tas", "edit", "--movie", path_str(&source)?];
    let output = Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .args(["--out", path_str(&out)?, "--set-input", "0=1", "--name"])
        .arg(&invalid)
        .output()?;
    check_failure(&output, "UTF-8", &args)?;
    let (_, stderr) = streams(&output)?;
    if stderr.contains("panicked") {
        return Err(format!("a non-UTF-8 argument must not panic:\n{stderr}").into());
    }
    if out.exists() {
        return Err(format!(
            "a rejected edit must not create an output file: {}",
            out.display()
        )
        .into());
    }
    assert_no_temp_files(&dir)?;
    // A valid argv still succeeds.
    let stdout = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--set-input",
        "0=1",
        "--name",
        "valid",
    ])?;
    assert_eq!(field(&block(&stdout)?, "name")?, "valid");
    Ok(())
}

#[test]
fn help_documents_the_contract_and_rejects_extra_arguments() -> TestResult {
    let stdout = ok(&["tas", "edit", "--help"])?;
    for needle in [
        "tas edit",
        "--movie",
        "--out",
        "--set-input",
        "--truncate-after",
        "--movie-id",
        "--name",
        "--anchors",
        "--anchor-interval",
        "zero-based",
        "re-simulation",
        "never modified",
        "parent_movie_id",
        "keep-prefix",
        "embedded checkpoint",
        "rerecord counter",
        "Format v1",
        "port 0",
        "synthetic",
        "M3",
        "not complete",
        "record=edit",
        "valid UTF-8",
    ] {
        if !stdout.contains(needle) {
            return Err(format!("edit help is missing `{needle}`:\n{stdout}").into());
        }
    }
    let summary = ok(&["tas", "--help"])?;
    for needle in [
        "tas edit",
        "edit:",
        "--set-input",
        "never modified",
        "not complete",
    ] {
        if !summary.contains(needle) {
            return Err(format!("`tas --help` is missing `{needle}`").into());
        }
    }
    let top = ok(&["help"])?;
    if !top.contains("tas edit") {
        return Err("`chronofork help` does not list `tas edit`".into());
    }
    failure(
        &["tas", "edit", "--help", "--movie", "x"],
        "takes no other arguments",
    )
}

#[test]
fn the_branch_movie_is_accepted_by_every_existing_tas_surface() -> TestResult {
    let dir = temp_dir("accepted")?;
    let source = record_v2(&dir, "source.cfmovie", 6, 1, 2)?;
    let out = dir.join("branch.cfmovie");
    let edit = ok(&[
        "tas",
        "edit",
        "--movie",
        path_str(&source)?,
        "--out",
        path_str(&out)?,
        "--set-input",
        "2=9",
        "--name",
        "accepted",
    ])?;
    let edit_block = block(&edit)?;
    let final_hash = field(&edit_block, "final_state_hash")?.to_owned();

    let verify = ok(&["tas", "verify", "--movie", path_str(&out)?])?;
    assert_fields(&block(&verify)?, &[("verified", "1")])?;
    let play = ok(&["tas", "play", "--movie", path_str(&out)?])?;
    assert_fields(&block(&play)?, &[("final_state_hash", &final_hash)])?;
    let ranged = ok(&[
        "tas",
        "verify",
        "--movie",
        path_str(&out)?,
        "--from",
        "2",
        "--until",
        "5",
    ])?;
    assert_fields(&block(&ranged)?, &[("verified", "1")])?;
    let comparison = comparison_block(&[
        "tas",
        "compare",
        "--movie",
        path_str(&source)?,
        "--against",
        path_str(&out)?,
    ])?;
    assert_fields(&comparison, &[("divergence_index", "2")])?;
    // `tas studio` opens the branch read-only; EOF after no command exits 0.
    ok(&["tas", "studio", "--movie", path_str(&out)?])?;
    Ok(())
}

#[test]
fn separate_processes_produce_identical_bytes_and_blocks() -> TestResult {
    let dir = temp_dir("determinism")?;
    let source = record_v2(&dir, "source.cfmovie", 8, 1, 2)?;
    let source_bytes = fs::read(&source)?;
    let other = temp_dir("determinism-copy")?;
    fs::copy(&source, other.join("renamed.cfmovie"))?;

    let args_a = [
        "tas",
        "edit",
        "--movie",
        "source.cfmovie",
        "--out",
        "a.cfmovie",
        "--set-input",
        "3=9",
        "--anchors",
        "rebuild",
        "--anchor-interval",
        "2",
        "--movie-id",
        "42",
        "--name",
        "same",
    ];
    let mut args_b = args_a;
    args_b[3] = "renamed.cfmovie";
    args_b[5] = "b.cfmovie";
    let stdout_a = ok_in(&dir, &args_a)?;
    let stdout_b = ok_in(&other, &args_b)?;
    if stdout_a != stdout_b {
        return Err("identical edits printed different blocks".into());
    }
    let bytes_a = fs::read(dir.join("a.cfmovie"))?;
    let bytes_b = fs::read(other.join("b.cfmovie"))?;
    if bytes_a != bytes_b {
        return Err("identical edits produced different movie bytes".into());
    }
    if fs::read(&source)? != source_bytes {
        return Err("the determinism runs changed the source file bytes".into());
    }
    // A repeated run of the same request in the same directory is also identical.
    let args_c = [
        "tas",
        "edit",
        "--movie",
        "source.cfmovie",
        "--out",
        "c.cfmovie",
        "--set-input",
        "3=9",
        "--anchors",
        "rebuild",
        "--anchor-interval",
        "2",
        "--movie-id",
        "42",
        "--name",
        "same",
    ];
    let stdout_c = ok_in(&dir, &args_c)?;
    if stdout_c != stdout_a {
        return Err("a repeated edit printed a different block".into());
    }
    if fs::read(dir.join("c.cfmovie"))? != bytes_a {
        return Err("a repeated edit produced different movie bytes".into());
    }
    Ok(())
}
