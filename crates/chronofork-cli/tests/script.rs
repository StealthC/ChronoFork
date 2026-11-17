// Draft of `crates/chronofork-cli/tests/script.rs` for T6.
// Kept in ignored notes until t6 is ready so the shared worktree keeps a green
// test target for the other M5 tasks in the meantime. Copy verbatim on claim.

//! Process-level acceptance tests for `chronofork script run` (ADR-0011 §3.6).
//!
//! Every invocation spawns the built `chronofork` binary, so the command line,
//! exit status, the two streams and the sandbox file effects are the whole
//! contract observed here. The lexer/parser/evaluator, limits, sandbox and report
//! model are owned and tested by `chronofork-script`; this target only checks the
//! thin CLI adapter and its exact rendered fields.
//!
//! Tests return `Result` instead of panicking helpers so the workspace
//! `expect_used` lint is satisfied without adding a file-level allowance.

use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use chronofork_core::hash_bytes;
use chronofork_megadrive::demo_rom;

type TestResult = Result<(), Box<dyn Error>>;
/// One parsed `key=value` record block.
type Block = BTreeMap<String, String>;

const SYNTHETIC_BACKEND: &str = "chronofork.test.synthetic";
const MEGADRIVE_BACKEND: &str = "chronofork.megadrive.m68000";

fn temp_dir(name: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "chronofork-cli-script-{}-{name}",
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

fn write_source(dir: &Path, name: &str, source: &str) -> io::Result<PathBuf> {
    let path = dir.join(name);
    std::fs::write(&path, source)?;
    Ok(path)
}

fn write_rom(dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join("synthetic.rom");
    std::fs::write(&path, demo_rom())?;
    Ok(path)
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

fn field<'a>(block: &'a Block, key: &str) -> Result<&'a str, Box<dyn Error>> {
    block
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("record block is missing `{key}`: {block:?}").into())
}

fn expect_field(block: &Block, key: &str, expected: &str) -> TestResult {
    let actual = field(block, key)?;
    if actual != expected {
        return Err(format!("field `{key}`: expected `{expected}`, got `{actual}`").into());
    }
    Ok(())
}

fn number(block: &Block, key: &str) -> Result<u64, Box<dyn Error>> {
    let value = field(block, key)?;
    value
        .parse::<u64>()
        .map_err(|_| format!("field `{key}` is not a decimal integer: `{value}`").into())
}

fn record(block: &Block) -> Result<&str, Box<dyn Error>> {
    field(block, "record")
}

fn last_block(blocks: &[Block]) -> Result<&Block, Box<dyn Error>> {
    blocks
        .last()
        .ok_or_else(|| "expected at least one record block".to_owned().into())
}

/// Runs the CLI, requiring exit 0 and empty stderr, without parsing blocks.
///
/// Used for help output, which is prose rather than `record=` blocks.
fn expect_success(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if !output.status.success() {
        return Err(format!(
            "command failed: {args:?}\nexit: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        )
        .into());
    }
    if !stderr.is_empty() {
        return Err(format!("successful command wrote to stderr: {args:?}\n{stderr}").into());
    }
    if stdout.is_empty() {
        return Err(format!("successful command printed nothing: {args:?}").into());
    }
    Ok(stdout)
}

/// Runs the CLI, requiring exit 0, empty stderr and at least one stdout block.
fn expect_ok(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if !output.status.success() {
        return Err(format!(
            "command failed: {args:?}\nexit: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        )
        .into());
    }
    if !stderr.is_empty() {
        return Err(format!("successful command wrote to stderr: {args:?}\n{stderr}").into());
    }
    parse_blocks(&stdout)?;
    Ok(stdout)
}

/// Runs the CLI, requiring the script-level failed-run contract: exit 1, empty
/// stderr, and a final `record=script_report` block with `status=failed`.
fn expect_script_failure(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(1) {
        return Err(format!(
            "expected exit 1 for a failed script run, got {:?}: {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        )
        .into());
    }
    if !stderr.is_empty() {
        return Err(
            format!("a script-level failure keeps stderr empty: {args:?}\n{stderr}").into(),
        );
    }
    let blocks = parse_blocks(&stdout)?;
    let report = last_block(&blocks)?;
    if record(report)? != "script_report" {
        return Err(format!("last block is not `record=script_report`: {report:?}").into());
    }
    expect_field(report, "status", "failed")?;
    let failure = field(report, "failure")?;
    if failure.is_empty() || failure == "-" {
        return Err(format!("failed report requires a failure code, got `{failure}`").into());
    }
    let message = field(report, "message")?;
    if message.is_empty() {
        return Err("failed report requires a message".to_owned().into());
    }
    Ok(stdout)
}

fn cli_error_once(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = run(args)?;
    let (stdout, stderr) = streams(&output)?;
    if output.status.code() != Some(1) {
        return Err(format!(
            "expected exit 1 for a CLI/usage/file error, got {:?}: {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        )
        .into());
    }
    if !stdout.is_empty() {
        return Err(format!("CLI errors keep stdout empty: {args:?}\n{stdout}").into());
    }
    if !stderr.starts_with("chronofork: ") || !stderr.ends_with('\n') {
        return Err(format!(
            "CLI errors use one `chronofork: ...` stderr line: {args:?}\n{stderr:?}"
        )
        .into());
    }
    Ok(stderr)
}

/// Runs one CLI error twice and requires deterministic stderr.
fn expect_cli_error(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let first = cli_error_once(args)?;
    let second = cli_error_once(args)?;
    if first != second {
        return Err(format!("CLI error stderr is not deterministic: {args:?}").into());
    }
    Ok(first)
}

/// Asserts that rendered stdout carries no host path or process identifier.
fn assert_no_host_data(stdout: &str, dir: &Path) -> TestResult {
    let dir_text = dir.display().to_string();
    if stdout.contains(&dir_text) {
        return Err(format!("stdout leaked the temporary path `{dir_text}`").into());
    }
    if stdout.contains(&format!("pid={}", std::process::id())) {
        return Err("stdout leaked a process id".to_owned().into());
    }
    Ok(())
}

#[test]
fn script_run_prints_report_and_uses_exit_codes() -> TestResult {
    let dir = temp_dir("report")?;

    // Passing run: ordered prints (including escaped text) then exactly one
    // report block with the documented fields and sorted custom fields.
    let passing = "let a = 1 + 2\n\
                   assert a == 3\n\
                   print \"hello\"\n\
                   print a\n\
                   print \"line\\nbreak\"\n\
                   report(\"score\", a)\n\
                   report(\"ok\", true)\n";
    let file = write_source(&dir, "pass.cfscript", passing)?;
    let args = ["script", "run", "--file", path_str(&file)?];
    let stdout = expect_ok(&args)?;
    let second = expect_ok(&args)?;
    if stdout != second {
        return Err("two identical script runs produced different stdout"
            .to_owned()
            .into());
    }
    assert_no_host_data(&stdout, &dir)?;

    let blocks = parse_blocks(&stdout)?;
    if blocks.len() != 4 {
        return Err(format!(
            "expected three print blocks and one report block, got {} blocks:\n{stdout}",
            blocks.len()
        )
        .into());
    }
    for (block, expected) in blocks[..3].iter().zip(["hello", "3", "line\\nbreak"]) {
        expect_field(block, "record", "script_print")?;
        expect_field(block, "text", expected)?;
    }
    let report = &blocks[3];
    expect_field(report, "record", "script_report")?;
    expect_field(report, "status", "passed")?;
    expect_field(report, "failure", "-")?;
    if report.contains_key("message") {
        return Err("a passed report must not carry `message`".to_owned().into());
    }
    expect_field(
        report,
        "source_sha256",
        &hash_bytes(passing.as_bytes()).to_hex(),
    )?;
    expect_field(report, "assertions", "1")?;
    expect_field(report, "steps", "0")?;
    expect_field(report, "variables", "1")?;
    expect_field(report, "trace_records", "0")?;
    expect_field(report, "backend", SYNTHETIC_BACKEND)?;
    for key in ["statements", "operations", "report_bytes"] {
        if number(report, key)? == 0 {
            return Err(format!("`{key}` must be a positive statistic").into());
        }
    }
    expect_field(report, "report.ok", "true")?;
    expect_field(report, "report.score", "3")?;
    let custom: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("report."))
        .collect();
    if custom != ["report.ok=true", "report.score=3"] {
        return Err(format!("custom report fields are not sorted by key: {custom:?}").into());
    }

    // A failing assertion exits 1, keeps stderr empty and still prints the report.
    let assertion = "print \"before\"\nassert 1 == 2\n";
    let file = write_source(&dir, "assert.cfscript", assertion)?;
    let args = ["script", "run", "--file", path_str(&file)?];
    let stdout = expect_script_failure(&args)?;
    let second = expect_script_failure(&args)?;
    if stdout != second {
        return Err("two failing assertion runs produced different stdout"
            .to_owned()
            .into());
    }
    assert_no_host_data(&stdout, &dir)?;
    let blocks = parse_blocks(&stdout)?;
    if blocks.len() != 2 {
        return Err(format!("expected one print and one report block:\n{stdout}").into());
    }
    expect_field(&blocks[0], "record", "script_print")?;
    expect_field(&blocks[0], "text", "before")?;
    expect_field(
        &blocks[1],
        "source_sha256",
        &hash_bytes(assertion.as_bytes()).to_hex(),
    )?;

    // `fail()` is a script-level failure with its deterministic message.
    let file = write_source(&dir, "fail.cfscript", "fail(\"boom\")\n")?;
    let args = ["script", "run", "--file", path_str(&file)?];
    let stdout = expect_script_failure(&args)?;
    let second = expect_script_failure(&args)?;
    if stdout != second {
        return Err("two `fail()` runs produced different stdout"
            .to_owned()
            .into());
    }
    let blocks = parse_blocks(&stdout)?;
    let report = last_block(&blocks)?;
    if !field(report, "message")?.contains("boom") {
        return Err(format!("`fail(\"boom\")` message lost its text: {report:?}").into());
    }

    // CLI/file/usage errors: exit 1, empty stdout, deterministic `chronofork:` stderr.
    expect_cli_error(&["script", "run"])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        path_str(&dir.join("missing.cfscript"))?,
    ])?;
    let oversized = write_source(&dir, "oversized.cfscript", &"#".repeat(4097))?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        path_str(&oversized)?,
        "--max-source-bytes",
        "4096",
    ])?;
    expect_cli_error(&["script", "run", "--file", path_str(&file)?, "--bogus"])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        path_str(&file)?,
        "--sandbox",
        path_str(&dir.join("missing-sandbox"))?,
    ])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        path_str(&file)?,
        "--max-operations",
        "0",
    ])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        path_str(&file)?,
        "--max-steps",
        "not-a-number",
    ])?;

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn script_run_supports_limits_and_both_backends() -> TestResult {
    let dir = temp_dir("limits")?;

    // Each documented limit flag overrides its default and produces the typed
    // script-level failure at the same boundary as the library row.
    let operations = write_source(&dir, "operations.cfscript", "print \"x\"\n")?;
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&operations)?,
        "--max-operations",
        "1000",
    ])?;
    expect_script_failure(&[
        "script",
        "run",
        "--file",
        path_str(&operations)?,
        "--max-operations",
        "1",
    ])?;

    let loops = write_source(&dir, "loops.cfscript", "repeat 4 {\n    print \"x\"\n}\n")?;
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&loops)?,
        "--max-loop-iterations",
        "4",
    ])?;
    expect_script_failure(&[
        "script",
        "run",
        "--file",
        path_str(&loops)?,
        "--max-loop-iterations",
        "3",
    ])?;

    let steps = write_source(
        &dir,
        "steps.cfscript",
        "open()\nlet t = step(4)\nreport(\"tick\", t)\n",
    )?;
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&steps)?,
        "--max-steps",
        "4",
    ])?;
    expect_script_failure(&[
        "script",
        "run",
        "--file",
        path_str(&steps)?,
        "--max-steps",
        "3",
    ])?;

    // `--max-memory-bytes` maps to `ScriptLimits::string_bytes`.
    let memory = write_source(
        &dir,
        "memory.cfscript",
        "let text = \"0123456789012345678901234567890123456789012345678901234567890123\"\n\
         print text\n",
    )?;
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&memory)?,
        "--max-memory-bytes",
        "128",
    ])?;
    expect_script_failure(&[
        "script",
        "run",
        "--file",
        path_str(&memory)?,
        "--max-memory-bytes",
        "32",
    ])?;

    // An oversized source is rejected before lexing with a CLI error.
    let mut padded = String::from("print \"ok\"\n");
    while padded.len() < 64 {
        padded.push('#');
    }
    padded.push('\n');
    if padded.len() != 65 {
        return Err("internal test fixture must be exactly 65 bytes"
            .to_owned()
            .into());
    }
    let source_limited = write_source(&dir, "source.cfscript", &padded)?;
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&source_limited)?,
        "--max-source-bytes",
        "65",
    ])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        path_str(&source_limited)?,
        "--max-source-bytes",
        "64",
    ])?;

    let report = write_source(&dir, "report.cfscript", "report(\"key\", \"0123456789\")\n")?;
    expect_script_failure(&[
        "script",
        "run",
        "--file",
        path_str(&report)?,
        "--max-report-bytes",
        "4",
    ])?;

    let variables = write_source(&dir, "variables.cfscript", "let a = 1\nlet b = 2\n")?;
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&variables)?,
        "--max-variables",
        "2",
    ])?;
    expect_script_failure(&[
        "script",
        "run",
        "--file",
        path_str(&variables)?,
        "--max-variables",
        "1",
    ])?;

    // Zero is rejected for every bounded limit, except wall time where zero
    // disables the deadline instead.
    for flag in [
        "--max-operations",
        "--max-loop-iterations",
        "--max-steps",
        "--max-memory-bytes",
        "--max-source-bytes",
        "--max-report-bytes",
        "--max-variables",
    ] {
        expect_cli_error(&["script", "run", "--file", path_str(&operations)?, flag, "0"])?;
    }
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&operations)?,
        "--wall-time-ms",
        "0",
    ])?;
    expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&operations)?,
        "--wall-time-ms",
        "1000",
    ])?;

    // `--rom` selects the Mega Drive factory; the default stays synthetic.
    let rom = write_rom(&dir)?;
    let backend_script = write_source(
        &dir,
        "backend.cfscript",
        "open()\nlet t = step(1)\nreport(\"tick\", t)\n",
    )?;
    let synthetic = expect_ok(&["script", "run", "--file", path_str(&backend_script)?])?;
    let synthetic_again = expect_ok(&["script", "run", "--file", path_str(&backend_script)?])?;
    if synthetic != synthetic_again {
        return Err("repeated synthetic runs are not byte-identical"
            .to_owned()
            .into());
    }
    let synthetic_blocks = parse_blocks(&synthetic)?;
    let synthetic_report = last_block(&synthetic_blocks)?;
    expect_field(synthetic_report, "status", "passed")?;
    expect_field(synthetic_report, "backend", SYNTHETIC_BACKEND)?;

    let megadrive = expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&backend_script)?,
        "--rom",
        path_str(&rom)?,
    ])?;
    let megadrive_blocks = parse_blocks(&megadrive)?;
    let megadrive_report = last_block(&megadrive_blocks)?;
    expect_field(megadrive_report, "status", "passed")?;
    expect_field(megadrive_report, "backend", MEGADRIVE_BACKEND)?;
    expect_field(megadrive_report, "steps", "1")?;

    // `--sandbox` enables the file builtins; without it the same script fails
    // at the sandbox boundary while non-file builtins still work.
    let sandbox_script = write_source(
        &dir,
        "sandbox.cfscript",
        "open()\n\
         let records = trace_capture(2, 4)\n\
         let bytes = trace_export(\"capture.cftrace\")\n\
         report(\"records\", records)\n\
         report(\"bytes\", bytes)\n",
    )?;
    expect_script_failure(&["script", "run", "--file", path_str(&sandbox_script)?])?;
    let sandbox_root = dir.join("sandbox-root");
    std::fs::create_dir_all(&sandbox_root)?;
    let stdout = expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&sandbox_script)?,
        "--sandbox",
        path_str(&sandbox_root)?,
    ])?;
    let blocks = parse_blocks(&stdout)?;
    let report = last_block(&blocks)?;
    expect_field(report, "status", "passed")?;
    expect_field(report, "report.records", "2")?;
    let exported = sandbox_root.join("capture.cftrace");
    let bytes = std::fs::metadata(&exported)?.len();
    if bytes == 0 {
        return Err("trace_export wrote an empty file".to_owned().into());
    }
    if report.get("report.bytes") != Some(&bytes.to_string()) {
        return Err(format!(
            "report.bytes does not match the exported file: {report:?} ({} bytes)",
            bytes
        )
        .into());
    }

    // The TAS builtins run through the same injected factory and sandbox.
    let movie = sandbox_root.join("script-demo.cfmovie");
    let recorded = run(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie)?,
        "--steps",
        "5",
    ])?;
    let (_, record_stderr) = streams(&recorded)?;
    if !recorded.status.success() {
        return Err(format!("`tas record` fixture failed: {record_stderr}").into());
    }
    let tas_script = write_source(
        &dir,
        "tas.cfscript",
        "let verified = tas_verify(\"script-demo.cfmovie\")\n\
         assert verified\n\
         report(\"verified\", verified)\n",
    )?;
    let stdout = expect_ok(&[
        "script",
        "run",
        "--file",
        path_str(&tas_script)?,
        "--sandbox",
        path_str(&sandbox_root)?,
    ])?;
    let blocks = parse_blocks(&stdout)?;
    let report = last_block(&blocks)?;
    expect_field(report, "status", "passed")?;
    expect_field(report, "report.verified", "true")?;

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn script_help_and_typed_rejections_follow_the_contract() -> TestResult {
    // Help exits 0 on stdout, documents the implemented surface and defaults,
    // and never advertises an engine or feature M5 does not implement.
    let output = run(&["script", "--help"])?;
    let (stdout, stderr) = streams(&output)?;
    if !output.status.success() {
        return Err(format!("`script --help` failed: {stderr}").into());
    }
    if !stderr.is_empty() {
        return Err("`script --help` must keep stderr empty".to_owned().into());
    }
    let help = stdout.to_ascii_lowercase();
    for needle in [
        "chronofork script run",
        "--file",
        "--rom",
        "--sandbox",
        "--max-operations",
        "--max-loop-iterations",
        "--max-steps",
        "--max-memory-bytes",
        "--max-source-bytes",
        "--max-report-bytes",
        "--max-variables",
        "--wall-time-ms",
        "default",
        "exit 0",
        "exit 1",
        "passed",
        "failed",
    ] {
        if !help.contains(needle) {
            return Err(format!("`script --help` is missing `{needle}`:\n{stdout}").into());
        }
    }
    for forbidden in [
        "rhai", "luau", "lua ", "wasm", "python", "http", "mcp", "gui",
    ] {
        if help.contains(forbidden) {
            return Err(format!(
                "`script --help` advertises an unimplemented feature `{forbidden}`:\n{stdout}"
            )
            .into());
        }
    }

    // The top-level help lists the new command group.
    let help_stdout = expect_success(&["help"])?;
    if !help_stdout.contains("chronofork script run") {
        return Err(format!("`chronofork help` does not list `script`:\n{help_stdout}").into());
    }

    let dir = temp_dir("rejections")?;
    let source = write_source(&dir, "ok.cfscript", "print \"ok\"\n")?;
    let source_path = path_str(&source)?;

    // Unknown subcommand and flag, missing value, repeated flag.
    expect_cli_error(&["script", "frobnicate"])?;
    expect_cli_error(&["script", "run", "--file", source_path, "--bogus", "1"])?;
    expect_cli_error(&["script", "run", "--file"])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        source_path,
        "--file",
        source_path,
    ])?;

    // Non-numeric, negative, overflowing and above-maximum limits.
    for value in ["abc", "-1", "18446744073709551616", "100000001"] {
        expect_cli_error(&[
            "script",
            "run",
            "--file",
            source_path,
            "--max-operations",
            value,
        ])?;
    }
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        source_path,
        "--max-source-bytes",
        "1048577",
    ])?;

    // Missing, empty, oversized and directory sources.
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        path_str(&dir.join("absent.cfscript"))?,
    ])?;
    let empty = write_source(&dir, "empty.cfscript", "")?;
    expect_cli_error(&["script", "run", "--file", path_str(&empty)?])?;
    let oversized = write_source(&dir, "huge.cfscript", &"#".repeat(65_537))?;
    expect_cli_error(&["script", "run", "--file", path_str(&oversized)?])?;
    expect_cli_error(&["script", "run", "--file", path_str(&dir)?])?;

    // Sandbox root must be an existing directory.
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        source_path,
        "--sandbox",
        path_str(&dir.join("absent-sandbox"))?,
    ])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        source_path,
        "--sandbox",
        source_path,
    ])?;

    // `--rom` must be a readable cartridge image.
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        source_path,
        "--rom",
        path_str(&dir.join("absent.rom"))?,
    ])?;
    let tiny = dir.join("tiny.rom");
    std::fs::write(&tiny, [0_u8; 3])?;
    expect_cli_error(&[
        "script",
        "run",
        "--file",
        source_path,
        "--rom",
        path_str(&tiny)?,
    ])?;

    // A malformed script is a script-level failure (report printed), not a
    // `chronofork:` CLI error: the CLI only rejects source I/O and limits.
    let malformed = write_source(&dir, "malformed.cfscript", "let = 1\n")?;
    expect_script_failure(&["script", "run", "--file", path_str(&malformed)?])?;

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
