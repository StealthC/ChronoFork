// Anti-slop policy §10 exception: owner cli-engineer; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It
// mirrors the existing headers in tests/m6.rs, tests/debug.rs, tests/demo.rs,
// tests/session_storage.rs and tests/trace.rs; no production code is affected.
#![allow(clippy::expect_used)]

//! Process-level acceptance tests for the Stage 7 `chronofork gui` surface.
//!
//! Every test spawns the built `chronofork` binary as a real operating-system process, so
//! each invocation crosses the documented process boundary: stdin, stdout, stderr and the
//! exit status are the whole contract. The tests cover
//! [`M7-A15`](../../../docs/quality/m7-gui-acceptance.md#m7-a15--pulse-exposes-the-cadence-deterministically)
//! and `M7-A17` through `M7-A24`, plus the local-mode step/frame, multi-pane layout and
//! attach/detach non-perturbation scenarios of the same rows.
//!
//! No result depends on wall clock time: local mode runs the deterministic `pulse`
//! command and asserts frame counters, and a daemon is used only through its published
//! `bind=` record with a deadline-polled wait. Two servers are always stopped before the
//! test returns.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use chronofork_core::hash_bytes;
use chronofork_megadrive::BACKEND_ID as MEGADRIVE_BACKEND_ID;
use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, parse};
use chronofork_sessions::Movie;
use chronofork_test_backend::BACKEND_ID as SYNTHETIC_BACKEND_ID;

/// A `chronofork.tokens.v1` table with one read/write `operator` principal.
const OPERATOR_TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]}]}"#;

/// A table whose **first** principal is read-only, as M7-A24 requires.
const READER_TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"reader","token":"reader-token-0001","scopes":["read"]},
 {"name":"writer","token":"writer-token-0001","scopes":["read","write"]}]}"#;

/// Bearer tokens of [`READER_TOKENS`]; neither may ever appear in an output.
const READER_TOKEN: &str = "reader-token-0001";
const WRITER_TOKEN: &str = "writer-token-0001";

/// Bearer token of [`OPERATOR_TOKENS`].
const OPERATOR_TOKEN: &str = "operator-token-0001";

/// The response bound of the canonical codec, used to parse envelopes in assertions.
fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

// ---------------------------------------------------------------------------
// Process harness
// ---------------------------------------------------------------------------

/// Creates (and clears) a per-test temporary directory.
fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("chronofork-gui-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Returns a UTF-8 path string for CLI arguments.
fn path_str(path: &Path) -> &str {
    path.to_str().expect("temporary path is UTF-8")
}

/// Runs the built CLI binary with `args` and no standard input.
fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .output()
        .expect("chronofork runs")
}

/// Runs the built CLI binary with `script` as its standard input.
///
/// A script may stop being read at any point: a process that fails on an early line exits
/// fail-closed and closes its end of the pipe, which turns the remaining write into
/// `BrokenPipe`. That one error is tolerated (the exit status, stdout and stderr are the
/// contract and are asserted by the caller); every other write error fails the harness.
fn run_script(args: &[&str], script: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn chronofork");
    let mut stdin = child.stdin.take().expect("child stdin");
    if let Err(error) = stdin.write_all(script.as_bytes()) {
        // A process that stops on an early failing line closes its end of the pipe before
        // the whole script has been written, so this write fails with `BrokenPipe`. That is
        // the documented fail-closed behaviour and the caller still asserts the exit status,
        // stdout and stderr, so only this error is tolerated. Any other write failure is a
        // real harness failure and must not be hidden.
        assert_eq!(
            error.kind(),
            ErrorKind::BrokenPipe,
            "writing the script to the child failed: {error}"
        );
    }
    drop(stdin);
    child.wait_with_output().expect("chronofork exits")
}

/// Splits one invocation's streams into UTF-8 text.
fn streams(output: &Output) -> (String, String) {
    (
        String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8"),
        String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8"),
    )
}

/// Requires exit 0 and empty stderr, and returns stdout.
fn expect_success(output: &Output) -> String {
    let (stdout, stderr) = streams(output);
    assert!(
        output.status.success(),
        "command failed\nexit: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    assert!(
        stderr.is_empty(),
        "a successful run writes nothing to stderr:\n{stderr}"
    );
    stdout
}

/// Requires exit 1 and exactly one newline-terminated stderr line, and returns both streams.
fn expect_failure(output: &Output) -> (String, String) {
    let (stdout, stderr) = streams(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "typed failures exit 1\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(!stderr.is_empty(), "a failure carries a diagnostic");
    assert!(
        stderr.ends_with('\n') && stderr.lines().count() == 1,
        "a failure is one newline-terminated stderr line:\n{stderr}"
    );
    (stdout, stderr)
}

// ---------------------------------------------------------------------------
// Output contract helpers
// ---------------------------------------------------------------------------

/// One `key=value` block of the documented line protocol.
#[derive(Debug)]
struct Block {
    /// The `record=` name.
    record: String,
    /// The field values by name.
    fields: BTreeMap<String, String>,
    /// The field names in the order they were printed.
    order: Vec<String>,
}

impl Block {
    /// Returns one field value.
    fn field(&self, name: &str) -> &str {
        self.fields
            .get(name)
            .map(String::as_str)
            .expect("the documented block field is present")
    }
}

/// Splits stdout into the chunks the protocol separates with exactly one blank line.
fn chunks(stdout: &str) -> Vec<&str> {
    stdout
        .split("\n\n")
        .map(|chunk| chunk.strip_suffix('\n').unwrap_or(chunk))
        .filter(|chunk| !chunk.is_empty())
        .collect()
}

/// Returns the blocks of one invocation, in order.
fn parse_blocks(stdout: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    for chunk in chunks(stdout) {
        let mut lines = chunk.lines();
        let Some(first) = lines.next() else {
            continue;
        };
        let Some(record) = first.strip_prefix("record=") else {
            continue;
        };
        let mut fields = BTreeMap::new();
        let mut order = Vec::new();
        for line in lines {
            let (key, value) = line.split_once('=').expect("block line is `key=value`");
            order.push(key.to_owned());
            fields.insert(key.to_owned(), value.to_owned());
        }
        blocks.push(Block {
            record: record.to_owned(),
            fields,
            order,
        });
    }
    blocks
}

/// Returns the record names of a block list, in order.
fn records(blocks: &[Block]) -> Vec<&str> {
    blocks.iter().map(|block| block.record.as_str()).collect()
}

/// Returns the blocks with the given record name, in order.
fn with_record<'a>(blocks: &'a [Block], record: &str) -> Vec<&'a Block> {
    blocks
        .iter()
        .filter(|block| block.record == record)
        .collect()
}

/// Returns the frame grids of one invocation, in order.
fn frame_grids(stdout: &str) -> Vec<&str> {
    chunks(stdout)
        .into_iter()
        .filter(|chunk| chunk.starts_with("chronofork gui layout="))
        .collect()
}

/// Returns the HTML documents of one invocation, in order.
fn html_documents(stdout: &str) -> Vec<&str> {
    chunks(stdout)
        .into_iter()
        .filter(|chunk| chunk.starts_with("<!DOCTYPE html>"))
        .collect()
}

/// Requires a frame grid of exactly `height` lines of `width` printable ASCII columns.
fn assert_grid_shape(grid: &str, width: usize, height: usize) {
    let lines: Vec<&str> = grid.lines().collect();
    assert_eq!(lines.len(), height, "the grid has exactly `height` lines");
    for line in lines {
        assert_eq!(
            line.chars().count(),
            width,
            "every grid line is exactly `width` columns: {line:?}"
        );
        assert!(
            line.bytes().all(|byte| (0x20..=0x7e).contains(&byte)),
            "a plain grid line is printable ASCII: {line:?}"
        );
    }
}

/// Returns whether `text` contains a 64-character hexadecimal run (a SHA-256 digest).
///
/// A frame renders canonical values only from decoded views, so a digest inside a grid would
/// mean a panel leaked a raw `state_hash` field; M7-A7 requires that no frame line carries
/// one.
fn contains_hex_digest(text: &str) -> bool {
    let mut run = 0_usize;
    for byte in text.bytes() {
        if byte.is_ascii_hexdigit() {
            run += 1;
            if run >= 64 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Removes the SGR sequences the `--ansi` renderer inserts.
fn strip_sgr(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                index += 1;
            }
            index += 1;
        } else {
            output.push(char::from(bytes[index]));
            index += 1;
        }
    }
    output
}

/// Requires the canonical `chronofork.error.v1` envelope carrying `code` as stderr.
fn assert_envelope(stderr: &str, code: &str) {
    let document = parse(stderr.trim_end().as_bytes(), &limits()).expect("canonical envelope");
    assert_eq!(
        document.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1"),
        "the diagnostic is the canonical error envelope: {stderr}"
    );
    let error = document.get("error").expect("error object");
    assert_eq!(
        error.get("code").and_then(JsonValue::as_str),
        Some(code),
        "the envelope carries the documented code: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Daemon harness
// ---------------------------------------------------------------------------

/// Writes one token table into `dir` and returns its path.
fn tokens_file(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write token file");
    path
}

/// Spawns `chronofork serve` on an ephemeral loopback port and reads its records.
fn spawn_serve(tokens: &Path, extra: &[&str]) -> (Child, String, BTreeMap<String, String>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_chronofork"));
    command
        .args([
            "serve",
            "--token-file",
            path_str(tokens),
            "--bind",
            "127.0.0.1:0",
        ])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn `chronofork serve`");
    let stdout = child.stdout.take().expect("serve stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut records = BTreeMap::new();
    let mut failure = String::new();
    for _ in 0..5 {
        match lines.next() {
            Some(Ok(line)) => {
                let Some((key, value)) = line.split_once('=') else {
                    failure = format!("serve record line is not `key=value`: {line}");
                    break;
                };
                records.insert(key.to_owned(), value.to_owned());
            }
            Some(Err(error)) => {
                failure = format!("failed to read a serve record: {error}");
                break;
            }
            None => {
                failure = "serve closed stdout before its records".to_owned();
                break;
            }
        }
    }
    if !failure.is_empty() {
        let stderr = stop_child(&mut child);
        assert!(
            failure.is_empty(),
            "serve did not publish its records ({failure}): {stderr}"
        );
    }
    assert_eq!(records.get("record").map(String::as_str), Some("serve"));
    let bind = records.get("bind").expect("serve prints `bind=`").clone();
    let address: SocketAddr = bind.parse().expect("bind is a socket address");
    assert_eq!(address.ip().to_string(), "127.0.0.1");
    let base = format!("http://{bind}");
    (child, base, records)
}

/// Stops a child, returning whatever it wrote to stderr.
fn stop_child(child: &mut Child) -> String {
    let _ = child.kill();
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    let _ = child.wait();
    stderr
}

/// Issues one `chronofork api call` and returns the parsed canonical envelope.
fn api_call(
    base: &str,
    tokens: &Path,
    principal: Option<&str>,
    op: &str,
    input: &str,
) -> JsonValue {
    let mut args: Vec<String> = vec![
        "api".to_owned(),
        "call".to_owned(),
        "--url".to_owned(),
        base.to_owned(),
        "--token-file".to_owned(),
        path_str(tokens).to_owned(),
    ];
    if let Some(principal) = principal {
        args.push("--principal".to_owned());
        args.push(principal.to_owned());
    }
    args.push("--op".to_owned());
    args.push(op.to_owned());
    args.push("--input".to_owned());
    args.push(input.to_owned());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let stdout = expect_success(&run_cli(&borrowed));
    parse(stdout.trim_end().as_bytes(), &limits()).expect("canonical envelope")
}

/// Returns the `result` object of one canonical envelope.
fn result_of(envelope: &JsonValue) -> &JsonValue {
    envelope.get("result").expect("the envelope has a result")
}

/// Returns one nested `result` value as an unsigned integer.
fn result_unsigned(envelope: &JsonValue, field: &str) -> u64 {
    result_of(envelope)
        .get(field)
        .and_then(JsonValue::as_u64)
        .expect("the result carries the documented unsigned field")
}

/// Creates session 1 on a daemon and returns its canonical create envelope.
fn create_session(base: &str, tokens: &Path, principal: Option<&str>) -> JsonValue {
    api_call(base, tokens, principal, "session.create", "{}")
}

// ---------------------------------------------------------------------------
// M7-A15: `pulse` exposes the cadence deterministically
// ---------------------------------------------------------------------------

#[test]
fn pulse_reports_the_documented_cadence() {
    let script = "create\nattach 1\npulse 3\npulse 1\nquit\n";
    let args = ["gui", "run", "--fps", "50"];
    let first = expect_success(&run_script(&args, script));
    let second = expect_success(&run_script(&args, script));
    assert_eq!(
        first, second,
        "two runs of the same stdin are byte-identical"
    );
    assert!(
        first.starts_with("record=create\n"),
        "the first block is the create block"
    );
    assert!(
        !first.ends_with("\n\n"),
        "there is no trailing blank line after the last block"
    );

    let blocks = parse_blocks(&first);
    assert_eq!(
        records(&blocks),
        ["create", "attach", "pulse", "pulse", "quit"],
        "exactly one block per command, in order"
    );

    let pulses = with_record(&blocks, "pulse");
    assert_eq!(pulses.len(), 2);
    assert_eq!(
        pulses[0].order,
        [
            "pane",
            "session",
            "frames",
            "frame",
            "tick",
            "state_hash",
            "halted",
            "ticks_per_frame",
            "fps",
            "elapsed_nanos",
            "controller",
        ],
        "the pulse block prints the documented fields in order"
    );
    assert_eq!(pulses[0].field("frames"), "3");
    assert_eq!(pulses[0].field("frame"), "3");
    assert_eq!(pulses[0].field("tick"), "3");
    assert_eq!(pulses[0].field("ticks_per_frame"), "1");
    assert_eq!(pulses[0].field("fps"), "50");
    assert_eq!(pulses[0].field("elapsed_nanos"), "60000000");
    assert_eq!(pulses[0].field("halted"), "false");
    assert_eq!(pulses[0].field("controller"), "human");
    assert_eq!(pulses[1].field("frames"), "1");
    assert_eq!(pulses[1].field("frame"), "4");
    assert_eq!(pulses[1].field("tick"), "4");
    assert_eq!(pulses[1].field("elapsed_nanos"), "80000000");

    // The `quit` block reports the lease and the stop reason only — `reason`,
    // `controller`, `generation` and `sequence` (ADR-0013 §3.9, `gui.md` §6.2). The frame
    // index after the second pulse is asserted on the `pulse` block above, which is where
    // the documented field table puts it; `quit` never carries `frame`.
    let quit = with_record(&blocks, "quit");
    assert_eq!(quit.len(), 1);
    assert_eq!(quit[0].field("reason"), "line");
    assert_eq!(quit[0].field("controller"), "human");
    assert_eq!(quit[0].field("generation"), "0");
    assert_eq!(quit[0].field("sequence"), "0");
}

// ---------------------------------------------------------------------------
// M7-A17: handoff process flow H→A→H in local mode
// ---------------------------------------------------------------------------

#[test]
fn handoff_process_flow_in_local_mode() {
    // The effective transition both ways, then a control command that executes under
    // `Human` again.
    let script = "create\nattach 1\nhandoff agent\nhandoff human\nstep\nquit\n";
    let stdout = expect_success(&run_script(&["gui", "run"], script));
    let parsed = parse_blocks(&stdout);
    assert_eq!(
        records(&parsed),
        ["create", "attach", "handoff", "handoff", "step", "quit"]
    );
    let handoffs = with_record(&parsed, "handoff");
    assert_eq!(handoffs[0].field("controller"), "agent");
    assert_eq!(handoffs[0].field("previous"), "human");
    assert_eq!(handoffs[0].field("changed"), "true");
    assert_eq!(handoffs[0].field("generation"), "1");
    assert_eq!(handoffs[0].field("sequence"), "1");
    assert_eq!(handoffs[1].field("controller"), "human");
    assert_eq!(handoffs[1].field("previous"), "agent");
    assert_eq!(handoffs[1].field("changed"), "true");
    assert_eq!(handoffs[1].field("generation"), "2");
    assert_eq!(handoffs[1].field("sequence"), "2");

    let steps = with_record(&parsed, "step");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].field("requested"), "1");
    assert_eq!(steps[0].field("steps"), "1");
    assert_eq!(steps[0].field("tick"), "1");
    assert_eq!(steps[0].field("controller"), "human");

    let quit = with_record(&parsed, "quit");
    assert_eq!(quit[0].field("controller"), "human");
    assert_eq!(quit[0].field("generation"), "2");
    assert_eq!(quit[0].field("sequence"), "2");

    // Under `Agent` a control command is refused: exit 1, one diagnostic, no block.
    let denied = "create\nattach 1\nhandoff agent\nstep\nquit\n";
    let (stdout, stderr) = expect_failure(&run_script(&["gui", "run"], denied));
    let parsed = parse_blocks(&stdout);
    assert_eq!(
        records(&parsed),
        ["create", "attach", "handoff"],
        "the denied step emits no block and stops the process"
    );
    assert!(
        stderr.starts_with("chronofork: gui: control_denied:"),
        "the refusal carries the documented code: {stderr}"
    );

    // Observation commands work while the agent holds the lease.
    let observing = "create\nattach 1\nhandoff agent\nframe\nhtml\nquit\n";
    let stdout = expect_success(&run_script(&["gui", "run"], observing));
    assert_eq!(frame_grids(&stdout).len(), 1, "`frame` prints one grid");
    assert_eq!(
        html_documents(&stdout).len(),
        1,
        "`html` prints one document"
    );
    assert_eq!(
        records(&parse_blocks(&stdout)),
        ["create", "attach", "handoff", "quit"],
        "`frame` is a grid and `html` prints the document, so neither adds a block"
    );
}

// ---------------------------------------------------------------------------
// M7-A18: local mode requires `create` and selects the backend honestly
// ---------------------------------------------------------------------------

#[test]
fn local_mode_requires_create_and_selects_the_backend() {
    // A fresh local process starts with zero sessions: `attach` fails with the canonical
    // service envelope, not with a GUI protocol code.
    let (stdout, stderr) = expect_failure(&run_script(&["gui", "run"], "attach 1\n"));
    assert!(stdout.is_empty(), "the failing step writes nothing");
    assert_envelope(&stderr, "not_found");

    let stdout = expect_success(&run_script(&["gui", "run"], "create\nattach 1\nquit\n"));
    let blocks = parse_blocks(&stdout);
    let create = with_record(&blocks, "create");
    assert_eq!(create[0].field("session"), "1");
    assert_eq!(create[0].field("backend_id"), SYNTHETIC_BACKEND_ID);
    let attach = with_record(&blocks, "attach");
    assert_eq!(attach[0].field("backend"), SYNTHETIC_BACKEND_ID);
    assert_eq!(attach[0].field("test_backend"), "true");
    assert_eq!(attach[0].field("rendering"), "false");
    assert_eq!(attach[0].field("audio"), "false");
    assert_eq!(attach[0].field("tick"), "0");
    assert_eq!(attach[0].field("halted"), "false");

    // `--rom` selects the minimal Mega Drive slice and reports it verbatim.
    let dir = temp_dir("local-rom");
    let rom = dir.join("demo.rom");
    std::fs::write(&rom, chronofork_megadrive::demo_rom()).expect("write demo ROM");
    let stdout = expect_success(&run_script(
        &["gui", "run", "--rom", path_str(&rom)],
        "create\nattach 1\nquit\n",
    ));
    let blocks = parse_blocks(&stdout);
    let create = with_record(&blocks, "create");
    assert_eq!(create[0].field("backend_id"), MEGADRIVE_BACKEND_ID);
    let attach = with_record(&blocks, "attach");
    assert_eq!(attach[0].field("backend"), MEGADRIVE_BACKEND_ID);
    assert_eq!(attach[0].field("test_backend"), "false");
    // The minimal Mega Drive slice advertises deterministic headless rendering
    // and 48 kHz audio capture; the GUI presentation pipeline still has no video
    // or audio sink, so its status line remains `rendering=unavailable
    // audio=unavailable`.
    assert_eq!(attach[0].field("rendering"), "true");
    assert_eq!(attach[0].field("audio"), "true");
}

// ---------------------------------------------------------------------------
// Local create + attach + step + frame, layout and detach
// ---------------------------------------------------------------------------

#[test]
fn local_step_and_frame_follow_the_documented_grid() {
    let script = "create\nattach 1\nstep 2\nframe\nquit\n";
    let stdout = expect_success(&run_script(&["gui", "run"], script));
    let blocks = parse_blocks(&stdout);
    let step = with_record(&blocks, "step");
    assert_eq!(step[0].field("requested"), "2");
    assert_eq!(step[0].field("steps"), "2");
    assert_eq!(step[0].field("tick"), "2");
    assert_eq!(step[0].field("halted"), "false");
    assert_eq!(step[0].field("controller"), "human");
    assert_eq!(step[0].field("state_hash").len(), 64, "canonical digest");

    let grids = frame_grids(&stdout);
    assert_eq!(grids.len(), 1);
    assert_grid_shape(grids[0], 80, 24);
    let lines: Vec<&str> = grids[0].lines().collect();
    assert_eq!(
        lines[0].trim_end(),
        "chronofork gui layout=single controller=human frame=0 fps=60"
    );
    assert_eq!(lines[1], "-".repeat(80));
    assert!(lines[2].starts_with("pane 1 session 1 tick 2 state ready"));
    assert!(lines[2].contains("focused"));
    let status = lines[23].trim_end();
    assert_eq!(
        status,
        "panes=1 focused=1 tick=2 state=ready rendering=unavailable audio=unavailable"
    );
}

#[test]
fn layout_and_detach_span_two_panes() {
    let script =
        "create\ncreate\nattach 1\nattach 2\nlayout mosaic\nframe\nfocus 1\ndetach\ndetach\nquit\n";
    let stdout = expect_success(&run_script(&["gui", "run"], script));
    let blocks = parse_blocks(&stdout);
    assert_eq!(
        records(&blocks),
        [
            "create", "create", "attach", "attach", "layout", "focus", "detach", "detach", "quit"
        ]
    );
    let layouts = with_record(&blocks, "layout");
    assert_eq!(layouts[0].field("layout"), "mosaic");
    assert_eq!(layouts[0].field("panes"), "2");
    assert_eq!(layouts[0].field("focused"), "2");

    let grids = frame_grids(&stdout);
    assert_eq!(grids.len(), 1);
    assert_grid_shape(grids[0], 80, 24);
    assert!(
        grids[0].contains("pane 1 session 1 tick 0 state ready"),
        "the mosaic draws pane 1 in its quadrant:\n{}",
        grids[0]
    );
    assert!(
        grids[0].contains("pane 2 session 2 tick 0 state ready"),
        "the mosaic draws pane 2 in its quadrant:\n{}",
        grids[0]
    );

    let detaches = with_record(&blocks, "detach");
    assert_eq!(detaches[0].field("pane"), "1");
    assert_eq!(detaches[0].field("session"), "1");
    assert_eq!(detaches[0].field("generation"), "1");
    assert_eq!(detaches[0].field("attached"), "false");
    assert_eq!(detaches[0].field("focused"), "2");
    assert_eq!(detaches[0].field("panes"), "1");
    assert_eq!(detaches[1].field("pane"), "2");
    assert_eq!(detaches[1].field("focused"), "none");
    assert_eq!(detaches[1].field("panes"), "0");

    // A detached pane is no longer focusable, and detaching twice is refused.
    let (stdout, stderr) = expect_failure(&run_script(
        &["gui", "run"],
        "create\nattach 1\ndetach\nfocus 1\n",
    ));
    assert!(!stdout.contains("record=focus"));
    assert!(
        stderr.starts_with("chronofork: gui: unknown_pane:"),
        "{stderr}"
    );
}

#[test]
fn attach_detach_moves_only_the_presentation_log() {
    // The `events` panel is the presentation log, so it legitimately changes with
    // attach/detach. Hiding it isolates the canonical content: after `detach` + `attach 1`
    // the frame is byte-identical to the frame of the first attach.
    //
    // The fresh-session state hash belongs to the canonical `create`/`attach` blocks, not to
    // the frame: no grid line carries a digest (M7-A7). The hash is the canonical value M7-A7
    // records for a fresh synthetic session.
    const FRESH_STATE_HASH: &str =
        "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";
    let once = "create\nattach 1\npanel events off\nframe\nquit\n";
    let twice = "create\nattach 1\npanel events off\nframe\ndetach\nattach 1\nframe\nquit\n";
    let first = expect_success(&run_script(&["gui", "run"], once));
    let second = expect_success(&run_script(&["gui", "run"], twice));

    let first_grids = frame_grids(&first);
    let second_grids = frame_grids(&second);
    assert_eq!(first_grids.len(), 1);
    assert_eq!(second_grids.len(), 2);
    assert_eq!(
        first_grids[0], second_grids[0],
        "the first frame is the same content"
    );
    assert_eq!(
        first_grids[0], second_grids[1],
        "detach + attach does not perturb the canonical content of the frame"
    );

    // `tick=0` and the canonical hash, before and after the detach, in the blocks that own
    // the canonical view.
    for (label, stdout, attaches) in [("once", &first, 1_usize), ("twice", &second, 2_usize)] {
        let blocks = parse_blocks(stdout);
        let create = with_record(&blocks, "create");
        assert_eq!(create.len(), 1, "{label}: one create block");
        assert_eq!(
            create[0].field("tick"),
            "0",
            "{label}: the session was never run"
        );
        assert_eq!(
            create[0].field("state_hash"),
            FRESH_STATE_HASH,
            "{label}: the create block carries the canonical fresh-session hash"
        );
        let attach = with_record(&blocks, "attach");
        assert_eq!(
            attach.len(),
            attaches,
            "{label}: one attach block per attach"
        );
        for block in attach {
            assert_eq!(block.field("tick"), "0", "{label}: attach tick");
            assert_eq!(
                block.field("state_hash"),
                FRESH_STATE_HASH,
                "{label}: every attach block carries the same canonical hash"
            );
        }
    }

    // No grid line carries the digest, and the frame keeps the documented canonical lines.
    for grid in [first_grids[0], second_grids[0], second_grids[1]] {
        assert!(
            !grid.contains(FRESH_STATE_HASH),
            "no frame line carries the state hash:\n{grid}"
        );
        assert!(
            !contains_hex_digest(grid),
            "no frame line carries a digest at all:\n{grid}"
        );
        assert!(
            grid.lines()
                .any(|line| line.starts_with("pane 1 session 1 tick 0 state ready")),
            "the frame carries the pane line:\n{grid}"
        );
    }
    let status = second_grids[1]
        .lines()
        .nth(23)
        .expect("the grid has a status line")
        .trim_end();
    assert_eq!(
        status, "panes=1 focused=1 tick=0 state=ready rendering=unavailable audio=unavailable",
        "the canonical tick and state are unchanged by detach + attach"
    );
}

// ---------------------------------------------------------------------------
// M7-A19: CLI grammar, help and exit codes
// ---------------------------------------------------------------------------

#[test]
fn grammar_help_and_exit_codes() {
    // Both help forms: usage on stdout, every documented flag and command name, silent
    // stderr, exit 0.
    let documented = [
        "--remote",
        "--token-file",
        "--rom",
        "--width",
        "--height",
        "--ansi",
        "--html",
        "--movie",
        "--fps",
        "--help",
        "create",
        "list",
        "attach",
        "detach",
        "focus",
        "layout",
        "panel",
        "step",
        "run",
        "pulse",
        "pause",
        "handoff",
        "frame",
        "html",
        "quit",
    ];
    let help_forms: [&[&str]; 2] = [&["gui", "--help"], &["gui", "run", "--help"]];
    for help in help_forms {
        let stdout = expect_success(&run_cli(help));
        assert!(stdout.contains("Usage:"), "usage is printed");
        for name in documented {
            assert!(stdout.contains(name), "help lists `{name}`:\n{stdout}");
        }
    }

    // Flag failures: exit 1, empty stdout, one `bad_value` diagnostic.
    let dir = temp_dir("grammar");
    let tokens = tokens_file(&dir, "tokens.json", OPERATOR_TOKENS);
    let rom = dir.join("demo.rom");
    std::fs::write(&rom, chronofork_megadrive::demo_rom()).expect("write demo ROM");
    let flag_cases: [(&[&str], &str); 6] = [
        (&["gui", "run", "--width", "39"], "bad_value"),
        (&["gui", "run", "--height", "101"], "bad_value"),
        (&["gui", "run", "--fps", "0"], "bad_value"),
        (
            &["gui", "run", "--remote", "http://127.0.0.1:1"],
            "bad_value",
        ),
        (&["gui", "run", "--fps", "50", "--fps", "60"], "bad_value"),
        (&["gui", "run", "--bogus"], "bad_value"),
    ];
    for (args, code) in flag_cases {
        let (stdout, stderr) = expect_failure(&run_cli(args));
        assert!(stdout.is_empty(), "a flag failure keeps stdout empty");
        assert!(
            stderr.starts_with(&format!("chronofork: gui: {code}:")),
            "`{}` reports {code}: {stderr}",
            args.join(" ")
        );
    }

    // `--rom` and `--remote` are mutually exclusive, and `--token-file` needs `--remote`.
    let (stdout, stderr) = expect_failure(&run_cli(&[
        "gui",
        "run",
        "--remote",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--rom",
        path_str(&rom),
    ]));
    assert!(stdout.is_empty());
    assert!(
        stderr.starts_with("chronofork: gui: bad_value:"),
        "{stderr}"
    );
    let (stdout, stderr) =
        expect_failure(&run_cli(&["gui", "run", "--token-file", path_str(&tokens)]));
    assert!(stdout.is_empty());
    assert!(
        stderr.starts_with("chronofork: gui: bad_value:"),
        "{stderr}"
    );

    // A closed port with a valid token file is a channel failure.
    let (stdout, stderr) = expect_failure(&run_cli(&[
        "gui",
        "run",
        "--remote",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
    ]));
    assert!(stdout.is_empty());
    assert!(stderr.starts_with("chronofork: gui: channel:"), "{stderr}");

    // Line-grammar failures, one per documented protocol code.
    let line_cases: [(&str, &str); 8] = [
        ("bogus\n", "unknown_command"),
        ("step abc\n", "bad_value"),
        ("step 1 2\n", "bad_arity"),
        ("focus 9\n", "unknown_pane"),
        ("panel bogus on\n", "unknown_panel"),
        ("detach\n", "not_attached"),
        ("pulse 601\n", "limit_exceeded"),
        ("layout bogus\n", "bad_value"),
    ];
    for (script, code) in line_cases {
        let (stdout, stderr) = expect_failure(&run_script(&["gui", "run"], script));
        assert!(
            stdout.is_empty(),
            "`{script}` keeps stdout empty on failure"
        );
        assert!(
            stderr.starts_with(&format!("chronofork: gui: {code}:")),
            "`{script}` reports {code}: {stderr}"
        );
    }

    // `quit` exits 0; a clean end of input exits 0 with `reason=eof`.
    let stdout = expect_success(&run_script(&["gui", "run"], "quit\n"));
    let blocks = parse_blocks(&stdout);
    assert_eq!(blocks[0].field("reason"), "line");
    let stdout = expect_success(&run_script(&["gui", "run"], "\n\n  \n"));
    let blocks = parse_blocks(&stdout);
    assert_eq!(blocks.len(), 1, "empty lines produce no block");
    assert_eq!(blocks[0].record, "quit");
    assert_eq!(blocks[0].field("reason"), "eof");
}

// ---------------------------------------------------------------------------
// M7-A20: process-level determinism, local and remote
// ---------------------------------------------------------------------------

#[test]
fn deterministic_process_output_local_and_remote() {
    let script = "create\nattach 1\npanel events off\nframe\nhtml\nquit\n";
    let first = expect_success(&run_script(&["gui", "run"], script));
    let second = expect_success(&run_script(&["gui", "run"], script));
    assert_eq!(first, second, "local stdout is byte-identical");
    assert_eq!(
        html_documents(&first).len(),
        1,
        "`html` without `--html` prints the document, not a block"
    );
    assert!(!first.contains(std::env::temp_dir().to_string_lossy().as_ref()));

    // ANSI is an emphasis layer: stripping the SGR sequences gives the plain run back.
    let plain = expect_success(&run_script(
        &["gui", "run"],
        "create\nattach 1\nframe\nquit\n",
    ));
    let ansi = expect_success(&run_script(
        &["gui", "run", "--ansi"],
        "create\nattach 1\nframe\nquit\n",
    ));
    assert!(ansi.contains('\u{1b}'), "`--ansi` inserts SGR sequences");
    assert_eq!(
        strip_sgr(&ansi),
        plain,
        "ANSI differs from plain only by SGR sequences"
    );

    // The HTML document is byte-identical across runs, whether written or printed.
    let dir = temp_dir("deterministic-html");
    let left = dir.join("left.html");
    let right = dir.join("right.html");
    let script = "create\nattach 1\nhtml\nquit\n";
    let left_stdout = expect_success(&run_script(
        &["gui", "run", "--html", path_str(&left)],
        script,
    ));
    let right_stdout = expect_success(&run_script(
        &["gui", "run", "--html", path_str(&right)],
        script,
    ));
    assert_eq!(left_stdout, right_stdout, "the `html` block is identical");
    let left_bytes = std::fs::read(&left).expect("left document");
    let right_bytes = std::fs::read(&right).expect("right document");
    assert_eq!(
        left_bytes.len(),
        right_bytes.len(),
        "the document length is identical"
    );
    assert_eq!(
        hash_bytes(&left_bytes).to_hex(),
        hash_bytes(&right_bytes).to_hex(),
        "the document digest is identical"
    );
    let printed_source = expect_success(&run_script(&["gui", "run"], script));
    let printed = html_documents(&printed_source);
    let written = std::str::from_utf8(&left_bytes).expect("the document is UTF-8");
    let written_body = written
        .strip_suffix('\n')
        .expect("the document ends with one newline");
    assert_eq!(
        printed[0], written_body,
        "the printed document equals the written document"
    );

    // Remote mode: two fresh daemons produce byte-identical output.
    let tokens = tokens_file(&dir, "tokens.json", OPERATOR_TOKENS);
    let mut remote = Vec::new();
    for _ in 0..2 {
        let (mut child, base, _records) = spawn_serve(&tokens, &[]);
        let run = run_script(
            &[
                "gui",
                "run",
                "--remote",
                &base,
                "--token-file",
                path_str(&tokens),
            ],
            script,
        );
        let stderr = stop_child(&mut child);
        assert!(stderr.is_empty(), "the daemon writes nothing to stderr");
        remote.push(expect_success(&run));
    }
    assert_eq!(
        remote[0], remote[1],
        "remote stdout is byte-identical across fresh daemons"
    );
    let remote_stdout = &remote[0];
    assert!(
        !remote_stdout.contains("127.0.0.1"),
        "no block or frame carries the daemon address"
    );
    assert!(
        !remote_stdout.contains(OPERATOR_TOKEN),
        "no output carries a token"
    );
    assert!(
        !remote_stdout.contains("://"),
        "no block or frame carries a URL"
    );
}

// ---------------------------------------------------------------------------
// M7-A21: remote observation over the real M6 daemon
// ---------------------------------------------------------------------------

#[test]
fn remote_observe_reads_the_daemon_without_mutation() {
    let dir = temp_dir("remote-observe");
    let tokens = tokens_file(&dir, "tokens.json", OPERATOR_TOKENS);
    let (mut child, base, _records) = spawn_serve(&tokens, &[]);

    let created = create_session(&base, &tokens, Some("operator"));
    assert_eq!(result_unsigned(&created, "session"), 1);
    let before = api_call(
        &base,
        &tokens,
        Some("operator"),
        "session.inspect",
        "{\"session\":1}",
    );
    let machine = result_of(&before).get("machine").expect("machine view");
    let tick_hash = machine
        .get("state_hash")
        .and_then(JsonValue::as_str)
        .expect("state hash")
        .to_owned();
    assert_eq!(machine.get("tick").and_then(JsonValue::as_u64), Some(0));

    let script = "list\nattach 1\nframe\nhtml\nquit\n";
    let run = run_script(
        &[
            "gui",
            "run",
            "--remote",
            &base,
            "--token-file",
            path_str(&tokens),
        ],
        script,
    );
    let stdout = expect_success(&run);
    let blocks = parse_blocks(&stdout);
    assert_eq!(
        records(&blocks),
        ["list", "attach", "quit"],
        "the GUI issues no mutating operation in this scenario"
    );

    let list = with_record(&blocks, "list");
    assert_eq!(list[0].field("sessions"), "1");
    assert_eq!(list[0].field("session.1.tick"), "0");
    assert_eq!(list[0].field("session.1.state_hash"), tick_hash);

    let attach = with_record(&blocks, "attach");
    assert_eq!(attach[0].field("session"), "1");
    assert_eq!(attach[0].field("tick"), "0");
    assert_eq!(attach[0].field("state_hash"), tick_hash);
    let backend = result_of(&before)
        .get("backend")
        .and_then(|backend| backend.get("id"))
        .and_then(JsonValue::as_str)
        .expect("backend id");
    assert_eq!(attach[0].field("backend"), backend);
    assert_eq!(attach[0].field("rendering"), "false");
    assert_eq!(attach[0].field("audio"), "false");

    assert_eq!(frame_grids(&stdout).len(), 1, "`frame` prints one grid");
    assert_grid_shape(frame_grids(&stdout)[0], 80, 24);
    assert_eq!(
        html_documents(&stdout).len(),
        1,
        "`html` prints one document"
    );
    assert!(
        html_documents(&stdout)[0].contains("chronofork gui layout=single"),
        "the document embeds the frame"
    );

    // The canonical session is untouched: the GUI only read.
    let after = api_call(
        &base,
        &tokens,
        Some("operator"),
        "session.inspect",
        "{\"session\":1}",
    );
    assert_eq!(
        result_of(&after)
            .get("machine")
            .and_then(|machine| machine.get("tick"))
            .and_then(JsonValue::as_u64),
        Some(0)
    );
    assert_eq!(
        result_of(&after)
            .get("machine")
            .and_then(|machine| machine.get("state_hash"))
            .and_then(JsonValue::as_str),
        Some(tick_hash.as_str())
    );
    // The M6 transport keeps no per-request log, so "the daemon recorded only read
    // operations" is asserted through its observable equivalent: no mutating command was
    // issued by the GUI and the canonical tick and state hash are unchanged.
    stop_child(&mut child);
}

// ---------------------------------------------------------------------------
// M7-A22: remote handoff with a second process as the agent
// ---------------------------------------------------------------------------

#[test]
fn remote_handoff_with_a_second_process_as_the_agent() {
    let dir = temp_dir("remote-handoff");
    let tokens = tokens_file(&dir, "tokens.json", OPERATOR_TOKENS);
    let (mut child, base, _records) = spawn_serve(&tokens, &[]);
    let created = create_session(&base, &tokens, Some("operator"));
    assert_eq!(result_unsigned(&created, "session"), 1);

    let gui = |script: &str| {
        run_script(
            &[
                "gui",
                "run",
                "--remote",
                &base,
                "--token-file",
                path_str(&tokens),
            ],
            script,
        )
    };

    // First GUI run: observe tick 0, take the lease as the agent, and be refused a step.
    let (stdout, stderr) = expect_failure(&gui("attach 1\nhandoff agent\nstep\nquit\n"));
    let blocks = parse_blocks(&stdout);
    assert_eq!(records(&blocks), ["attach", "handoff"]);
    let attach = with_record(&blocks, "attach");
    assert_eq!(attach[0].field("tick"), "0");
    let handoff = with_record(&blocks, "handoff");
    assert_eq!(handoff[0].field("controller"), "agent");
    assert_eq!(handoff[0].field("changed"), "true");
    assert_eq!(handoff[0].field("generation"), "1");
    assert_eq!(handoff[0].field("sequence"), "1");
    assert!(
        stderr.starts_with("chronofork: gui: control_denied:"),
        "the refused step is the documented control denial: {stderr}"
    );

    // The agent is a second process: a plain `chronofork api call` with the same token.
    let agent = api_call(
        &base,
        &tokens,
        Some("operator"),
        "session.run",
        "{\"session\":1,\"steps\":5}",
    );
    assert_eq!(result_unsigned(&agent, "tick"), 5);

    // Second GUI run: a fresh attach issues `session.inspect`, so the frame observes the
    // agent's tick. The model deliberately does not re-inspect on `frame`, and a second
    // `attach` in the same window is idempotent, so a new process is the documented
    // "second GUI run" path of M7-A22.
    let stdout = expect_success(&gui("attach 1\nframe\nquit\n"));
    let blocks = parse_blocks(&stdout);
    let attach = with_record(&blocks, "attach");
    assert_eq!(attach[0].field("tick"), "5", "the agent's tick is observed");
    let inspected = api_call(
        &base,
        &tokens,
        Some("operator"),
        "session.inspect",
        "{\"session\":1}",
    );
    let hash = result_of(&inspected)
        .get("machine")
        .and_then(|machine| machine.get("state_hash"))
        .and_then(JsonValue::as_str)
        .expect("state hash");
    assert_eq!(attach[0].field("state_hash"), hash);
    let grid = frame_grids(&stdout);
    assert_eq!(grid.len(), 1);
    assert!(
        grid[0].contains("tick=5"),
        "the frame shows the observed tick:\n{}",
        grid[0]
    );

    // The same process order again, but handing control back: the human step executes.
    let stdout = expect_success(&gui("attach 1\nhandoff agent\nhandoff human\nstep\nquit\n"));
    let blocks = parse_blocks(&stdout);
    let step = with_record(&blocks, "step");
    assert_eq!(step.len(), 1);
    assert_eq!(step[0].field("tick"), "6");
    assert_eq!(step[0].field("controller"), "human");
    let inspected = api_call(
        &base,
        &tokens,
        Some("operator"),
        "session.inspect",
        "{\"session\":1}",
    );
    assert_eq!(
        result_of(&inspected)
            .get("machine")
            .and_then(|machine| machine.get("tick"))
            .and_then(JsonValue::as_u64),
        Some(6)
    );
    stop_child(&mut child);
}

// ---------------------------------------------------------------------------
// M7-A23: read-only `.cfmovie` timeline
// ---------------------------------------------------------------------------

#[test]
fn timeline_panel_reads_the_movie_read_only() {
    let dir = temp_dir("timeline");
    let movie_path = dir.join("timeline.cfmovie");
    expect_success(&run_cli(&[
        "tas",
        "record",
        "--movie",
        path_str(&movie_path),
        "--steps",
        "5",
    ]));
    let before_bytes = std::fs::read(&movie_path).expect("movie bytes");
    let before_hash = hash_bytes(&before_bytes).to_hex();
    let movie = Movie::from_bytes(&before_bytes).expect("decode the recorded movie");

    let script = "create\nattach 1\npanel timeline on\nframe\nquit\n";
    let stdout = expect_success(&run_script(
        &["gui", "run", "--movie", path_str(&movie_path)],
        script,
    ));
    let blocks = parse_blocks(&stdout);
    let panel = with_record(&blocks, "panel");
    assert_eq!(panel[0].field("panel"), "timeline");
    assert_eq!(panel[0].field("visible"), "true");
    assert_eq!(panel[0].field("available"), "true");
    assert_eq!(panel[0].field("reason"), "none");

    let expected = format!(
        "timeline version={} steps={} end_tick={} anchors={}",
        movie.format_version(),
        movie.step_count(),
        movie.end_tick().raw(),
        movie.anchors().len()
    );
    let grids = frame_grids(&stdout);
    assert_eq!(grids.len(), 1);
    assert!(
        grids[0].contains(&expected),
        "the panel reports the movie's own values, expected `{expected}`:\n{}",
        grids[0]
    );

    // No `--movie`: the panel is honestly unavailable.
    let stdout = expect_success(&run_script(
        &["gui", "run"],
        "create\nattach 1\npanel timeline on\nframe\nquit\n",
    ));
    let blocks = parse_blocks(&stdout);
    let panel = with_record(&blocks, "panel");
    assert_eq!(panel[0].field("available"), "false");
    assert_eq!(panel[0].field("reason"), "no_movie");
    assert!(frame_grids(&stdout)[0].contains("timeline unavailable reason=no_movie"));

    // A truncated movie is a typed `movie` failure with empty stdout.
    let truncated = dir.join("truncated.cfmovie");
    let half = before_bytes.len() / 2;
    std::fs::write(&truncated, &before_bytes[..half]).expect("write truncated movie");
    let (stdout, stderr) = expect_failure(&run_script(
        &["gui", "run", "--movie", path_str(&truncated)],
        "quit\n",
    ));
    assert!(stdout.is_empty());
    assert!(
        stderr.starts_with("chronofork: gui: movie:"),
        "a truncated movie is a typed movie failure: {stderr}"
    );

    // The movie file is never written, moved or truncated.
    let after_bytes = std::fs::read(&movie_path).expect("movie bytes after the run");
    assert_eq!(
        after_bytes.len(),
        before_bytes.len(),
        "the movie length is unchanged"
    );
    assert_eq!(
        hash_bytes(&after_bytes).to_hex(),
        before_hash,
        "the movie digest is unchanged"
    );
}

// ---------------------------------------------------------------------------
// M7-A24: token and security boundary
// ---------------------------------------------------------------------------

#[test]
fn token_boundary_and_forbidden_operations() {
    let dir = temp_dir("token");
    let tokens = tokens_file(&dir, "tokens.json", READER_TOKENS);
    let (mut child, base, _records) = spawn_serve(&tokens, &[]);
    let created = create_session(&base, &tokens, Some("writer"));
    assert_eq!(result_unsigned(&created, "session"), 1);

    // The GUI always uses the first principal, which is read-only here.
    let gui = |script: &str| {
        run_script(
            &[
                "gui",
                "run",
                "--remote",
                &base,
                "--token-file",
                path_str(&tokens),
            ],
            script,
        )
    };

    // Observation succeeds: `list` sees only the read-only principal's own sessions, and
    // `frame`/`html` need no session at all.
    let stdout = expect_success(&gui("list\nframe\nhtml\nquit\n"));
    let blocks = parse_blocks(&stdout);
    let list = with_record(&blocks, "list");
    assert_eq!(list[0].field("sessions"), "0");
    assert_eq!(frame_grids(&stdout).len(), 1);
    assert_eq!(html_documents(&stdout).len(), 1);

    // `create` needs the write scope: the canonical `forbidden` envelope, empty stdout.
    let (stdout, stderr) = expect_failure(&gui("create\n"));
    assert!(stdout.is_empty(), "the failing step writes nothing");
    assert_envelope(&stderr, "forbidden");

    // `attach` on a session owned by another principal is refused by the M6 ownership
    // rule (a scope-only check would let it through), and `step` never runs.
    let (stdout, stderr) = expect_failure(&gui("attach 1\nstep\nquit\n"));
    assert!(stdout.is_empty(), "the failing step writes nothing");
    assert_envelope(&stderr, "forbidden");

    // No token ever reaches stdout, stderr or the HTML document of any run.
    for output in [
        gui("list\nframe\nhtml\nquit\n"),
        gui("create\n"),
        gui("attach 1\nstep\nquit\n"),
    ] {
        let (stdout, stderr) = streams(&output);
        for token in [READER_TOKEN, WRITER_TOKEN] {
            assert!(!stdout.contains(token), "stdout must not carry a token");
            assert!(!stderr.contains(token), "stderr must not carry a token");
        }
        assert!(!stdout.contains(path_str(&tokens)));
    }
    stop_child(&mut child);

    // An oversized token file is rejected before it is buffered.
    let oversized = dir.join("oversized.json");
    let limit = limits().max_request_bytes;
    std::fs::write(&oversized, vec![b' '; limit + 1]).expect("write oversized token file");
    let (stdout, stderr) = expect_failure(&run_cli(&[
        "gui",
        "run",
        "--remote",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&oversized),
    ]));
    assert!(stdout.is_empty());
    assert!(
        stderr.starts_with("chronofork: gui: limit_exceeded:"),
        "an oversized token file is a bounded read failure: {stderr}"
    );

    // A malformed token file is a bad option value, and local mode reads no token file.
    let malformed = dir.join("malformed.json");
    std::fs::write(&malformed, "{}").expect("write malformed token file");
    let (stdout, stderr) = expect_failure(&run_cli(&[
        "gui",
        "run",
        "--remote",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&malformed),
    ]));
    assert!(stdout.is_empty());
    assert!(
        stderr.starts_with("chronofork: gui: bad_value:"),
        "{stderr}"
    );
    let (stdout, stderr) =
        expect_failure(&run_cli(&["gui", "run", "--token-file", path_str(&tokens)]));
    assert!(stdout.is_empty());
    assert!(
        stderr.starts_with("chronofork: gui: bad_value:"),
        "{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Closed stdout: the help path is a checked write, never a print panic
// ---------------------------------------------------------------------------

/// A stdout the child can never write to: a socket pair whose peer is already dropped.
///
/// This is the deterministic form of "stdout piped with the read end discarded". A
/// `Stdio::piped()` read end cannot be closed before the child runs — the parent only
/// receives it after `spawn` — so dropping the peer before `spawn` removes the race in
/// which the whole help text could land in the pipe buffer and the run would succeed. The
/// child sees the same failure either way: a stream whose reader is gone, so its first
/// write returns `EPIPE`.
#[cfg(unix)]
fn unreadable_stdout() -> Stdio {
    let (writer, reader) = std::os::unix::net::UnixStream::pair().expect("socket pair");
    drop(reader);
    Stdio::from(std::os::fd::OwnedFd::from(writer))
}

/// Runs the CLI with a caller-supplied stdout, capturing stderr only.
#[cfg(unix)]
fn run_with_stdout(args: &[&str], stdout: Stdio) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .output()
        .expect("chronofork runs")
}

/// Requires the documented channel failure for an unwritable stdout.
#[cfg(unix)]
fn assert_unwritable_stdout_failure(args: &[&str], output: &Output) {
    let (stdout, stderr) = streams(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "`{}` must exit 1, never 101 from a print panic\nstderr:\n{stderr}",
        args.join(" ")
    );
    // The stdout of these runs is the unwritable stream itself, so nothing can be read
    // back from it: an empty `Output::stdout` documents that no fallback copy of the help
    // was written to a readable descriptor.
    assert!(
        stdout.is_empty(),
        "the configured stdout carries no data: {stdout:?}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("failed printing to stdout"),
        "no panic text on stderr:\n{stderr}"
    );
    assert_eq!(
        stderr.lines().count(),
        1,
        "the failure is exactly one stderr line:\n{stderr}"
    );
    assert!(
        stderr.starts_with("chronofork: gui: channel: "),
        "the failure carries the documented channel code:\n{stderr}"
    );
    assert!(
        stderr.contains("the output stream could not be written"),
        "the diagnostic is the one the data path uses:\n{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn help_with_an_unwritable_stdout_is_one_channel_line() {
    // Both help forms share `print_help`, so both are covered.
    let help_forms: [&[&str]; 2] = [&["gui", "--help"], &["gui", "run", "--help"]];
    for args in help_forms {
        assert_unwritable_stdout_failure(args, &run_with_stdout(args, unreadable_stdout()));
        // `/dev/full` rejects every write with `ENOSPC`; it exists on Linux and the
        // scenario is skipped where it does not.
        if let Ok(device) = std::fs::File::create("/dev/full") {
            assert_unwritable_stdout_failure(args, &run_with_stdout(args, Stdio::from(device)));
        }
    }

    // Positive control: the same path still succeeds with a writable stdout.
    let stdout = expect_success(&run_cli(&["gui", "--help"]));
    assert!(stdout.contains("Usage:"), "{stdout}");
}

// ---------------------------------------------------------------------------
// Remote error bodies: only the canonical envelope may reach stderr
// ---------------------------------------------------------------------------

/// The canonical `chronofork.error.v1` document a ChronoFork daemon answers with.
///
/// The `wire_version` member is part of the envelope (ADR-0016 §2): the v2 control
/// codes are declared by the version beside the schema string.
const CANONICAL_ERROR_ENVELOPE: &str = "{\"error\":{\"code\":\"forbidden\",\"message\":\"read only\",\"retryable\":false},\"schema\":\"chronofork.error.v1\",\"wire_version\":2}";

/// A non-ChronoFork 403 body: HTML, several lines, and text a terminal must not receive.
const FOREIGN_ERROR_BODY: &str =
    "<html>\n<body>\n<h1>403 Forbidden</h1>\n<p>proxy says no</p>\n</body>\n</html>\n";

/// Serves exactly one HTTP answer and returns the base URL the CLI should use.
fn spawn_fake_peer(
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind the fake peer");
    let address = listener.local_addr().expect("fake peer address");
    let handle = std::thread::spawn(move || {
        let (mut stream, _peer) = listener.accept().expect("the client connects");
        read_http_request(&mut stream);
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).expect("write the head");
        stream.write_all(&body).expect("write the body");
        stream.flush().expect("flush the answer");
    });
    (format!("http://{address}"), handle)
}

/// Reads one HTTP/1.1 request completely, so closing the socket does not reset it.
fn read_http_request(stream: &mut std::net::TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone the connection"));
    let mut content_length = 0_usize;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).expect("read the request head") == 0 {
            return;
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line
            .strip_prefix("Content-Length:")
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            content_length = value;
        }
    }
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body).expect("read the request body");
}

#[test]
fn remote_error_body_must_be_the_canonical_envelope() {
    let dir = temp_dir("remote-error-body");
    let tokens = tokens_file(&dir, "tokens.json", OPERATOR_TOKENS);
    let run = |base: &str| {
        run_script(
            &[
                "gui",
                "run",
                "--remote",
                base,
                "--token-file",
                path_str(&tokens),
            ],
            "quit\n",
        )
    };

    // A foreign 403 with an HTML multi-line page: one local diagnostic, no body echoed.
    let (base, peer) = spawn_fake_peer(
        "403 Forbidden",
        "text/html",
        FOREIGN_ERROR_BODY.as_bytes().to_vec(),
    );
    let output = run(&base);
    peer.join().expect("the fake peer finished");
    let (stdout, stderr) = expect_failure(&output);
    assert!(stdout.is_empty(), "the failing step writes nothing");
    assert!(
        stderr.starts_with("chronofork: gui: channel: "),
        "the failure is the documented channel diagnostic: {stderr}"
    );
    assert_eq!(
        stderr.lines().count(),
        1,
        "a foreign body never adds lines to stderr: {stderr}"
    );
    for fragment in ["<html>", "403 Forbidden", "proxy says no", "</html>"] {
        assert!(
            !stderr.contains(fragment),
            "the foreign body is not echoed ({fragment}): {stderr}"
        );
    }
    assert!(
        stderr.contains("without a canonical error envelope"),
        "the local diagnostic explains the rejected body: {stderr}"
    );

    // The canonical envelope is still printed verbatim as the single diagnostic line.
    let (base, peer) = spawn_fake_peer(
        "403 Forbidden",
        "application/json",
        CANONICAL_ERROR_ENVELOPE.as_bytes().to_vec(),
    );
    let output = run(&base);
    peer.join().expect("the fake peer finished");
    let (stdout, stderr) = expect_failure(&output);
    assert!(stdout.is_empty(), "the failing step writes nothing");
    assert_eq!(
        stderr,
        format!("{CANONICAL_ERROR_ENVELOPE}\n"),
        "the canonical envelope is the one stderr line, byte for byte"
    );
    assert!(
        !stderr.contains("chronofork: gui:"),
        "a canonical envelope is not wrapped in a local diagnostic: {stderr}"
    );

    // Canonicality is byte-exact: the documented shape with an extra trailing newline is not
    // the canonical encoding, so it is rejected instead of printed.
    let (base, peer) = spawn_fake_peer(
        "403 Forbidden",
        "application/json",
        format!("{CANONICAL_ERROR_ENVELOPE}\n").into_bytes(),
    );
    let output = run(&base);
    peer.join().expect("the fake peer finished");
    let (stdout, stderr) = expect_failure(&output);
    assert!(stdout.is_empty(), "the failing step writes nothing");
    assert!(
        stderr.starts_with("chronofork: gui: channel: "),
        "a non-canonical encoding is a local diagnostic: {stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "one stderr line: {stderr}");
    assert!(
        !stderr.contains("forbidden"),
        "the rejected body is not echoed: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Harness robustness: a child that stops before reading the whole script
// ---------------------------------------------------------------------------

#[test]
fn harness_tolerates_a_child_that_stops_before_reading_the_whole_script() {
    // The first line fails, so the process exits fail-closed while the rest of the script is
    // still being written. The script is larger than a pipe buffer, so the write cannot fit
    // in the kernel buffer and the child's exit turns it into `BrokenPipe`: the harness has
    // to absorb exactly that error instead of panicking, and the documented failure still has
    // to be asserted normally.
    let mut script = String::from("bogus\n");
    script.push_str(&"x".repeat(100_000));
    script.push('\n');
    let (stdout, stderr) = expect_failure(&run_script(&["gui", "run"], &script));
    assert!(stdout.is_empty(), "the failing step writes nothing");
    assert!(
        stderr.starts_with("chronofork: gui: unknown_command:"),
        "the first line is still the reported failure: {stderr}"
    );
}
