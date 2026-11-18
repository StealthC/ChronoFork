#![allow(clippy::expect_used)]

//! Integration tests for `chronofork megadrive trace`.
//!
//! They run the built binary against the shared, independently authored
//! synthetic ROM from `tests/common` and check the documented trace/state
//! formats, determinism and option validation.

mod common;

use common::{path_str, run, temp_dir, write_rom};

#[test]
fn trace_and_state_are_byte_stable_across_two_runs() {
    let dir = temp_dir("stable");
    let rom = write_rom(&dir);

    let mut results = Vec::new();
    for label in ["a", "b"] {
        let trace_path = dir.join(format!("trace-{label}.tsv"));
        let state_path = dir.join(format!("state-{label}.txt"));
        let output = run(&[
            "megadrive",
            "trace",
            "--rom",
            path_str(&rom),
            "--instructions",
            "5",
            "--trace-out",
            path_str(&trace_path),
            "--state-out",
            path_str(&state_path),
        ]);
        let stderr = String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8");
        assert!(output.status.success(), "trace run failed: {stderr}");
        results.push((
            std::fs::read(&trace_path).expect("read trace file"),
            std::fs::read(&state_path).expect("read state file"),
        ));
    }
    assert_eq!(results[0].0, results[1].0, "trace output is not stable");
    assert_eq!(results[0].1, results[1].1, "state output is not stable");

    let trace = String::from_utf8(results[0].0.clone()).expect("trace is UTF-8");
    assert_eq!(
        trace,
        "0\t000200\t7001\n\
         1\t000202\t7202\n\
         2\t000204\td041\n\
         3\t000206\t33c0\n\
         4\t00020c\t4e71\n"
    );

    let state = String::from_utf8(results[0].1.clone()).expect("state is UTF-8");
    assert_eq!(state.lines().next(), Some("chronofork-megadrive-state v1"));
    for line in [
        "executed=5",
        "tick=224",
        "master_cycles=224",
        "cpu_cycles=32",
        "video=ntsc",
        "halted=0",
        "pc=0000020e",
        "sr=2700",
        "d0=00000003",
        "d1=00000002",
        "a7=01000000",
        "ram_base=00ff0000",
        "ram_len=32",
    ] {
        assert!(state.lines().any(|value| value == line), "missing `{line}`");
    }
    let ram = state
        .lines()
        .find(|line| line.starts_with("ram="))
        .expect("ram line");
    assert!(
        ram.starts_with("ram=0003"),
        "work RAM was not written: {ram}"
    );
    assert_eq!(ram.len(), "ram=".len() + 64);
    assert!(
        ram["ram=0003".len()..].bytes().all(|byte| byte == b'0'),
        "unexpected work-RAM bytes: {ram}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_respects_the_stop_address() {
    let dir = temp_dir("stop");
    let rom = write_rom(&dir);
    let trace_path = dir.join("trace.tsv");
    let state_path = dir.join("state.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--instructions",
        "100",
        "--stop-pc",
        "0x204",
        "--trace-out",
        path_str(&trace_path),
        "--state-out",
        path_str(&state_path),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(output.status.success(), "trace run failed: {stderr}");
    assert!(stderr.contains("stopped before the instruction at pc 0x000204"));
    let trace = std::fs::read_to_string(&trace_path).expect("trace file");
    assert_eq!(trace, "0\t000200\t7001\n1\t000202\t7202\n");
    let state = std::fs::read_to_string(&state_path).expect("state file");
    assert!(state.lines().any(|line| line == "executed=2"));
    assert!(state.lines().any(|line| line == "pc=00000204"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_writes_to_stdout_when_no_trace_file_is_given() {
    let dir = temp_dir("stdout");
    let rom = write_rom(&dir);
    let state_path = dir.join("state.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--instructions",
        "3",
        "--state-out",
        path_str(&state_path),
    ]);
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(output.status.success(), "trace run failed: {stderr}");
    assert_eq!(
        stdout,
        "0\t000200\t7001\n1\t000202\t7202\n2\t000204\td041\n"
    );
    assert!(stderr.contains("executed 3 instruction(s)"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_master_cycle_timeline_is_reported() {
    let dir = temp_dir("default-quantum");
    let rom = write_rom(&dir);
    let state_path = dir.join("state.txt");
    let trace_path = dir.join("trace.tsv");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--instructions",
        "65",
        "--trace-out",
        path_str(&trace_path),
        "--state-out",
        path_str(&state_path),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(output.status.success(), "trace run failed: {stderr}");
    let state = std::fs::read_to_string(&state_path).expect("state file");
    assert!(
        state.lines().any(|line| line == "master_cycles=1904"),
        "the state records the master-cycle clock: {state}"
    );
    assert!(
        state.lines().any(|line| line.starts_with("cpu_cycles=")),
        "the state records the 68000 cycle count: {state}"
    );
    for expected in [
        "master-cycle timeline",
        "interrupt-priority mask",
        "level 7 is always taken",
        "stays pending",
    ] {
        assert!(
            stderr.contains(expected),
            "the gating description is missing `{expected}`: {stderr}"
        );
    }
    for stale in ["does not gate interrupts", "E2-15", "1000000"] {
        assert!(
            !stderr.contains(stale),
            "stale interrupt text `{stale}` must be gone: {stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn instructions_per_line_is_no_longer_an_option() {
    let dir = temp_dir("explicit-quantum");
    let rom = write_rom(&dir);
    let state_path = dir.join("state.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--instructions",
        "3",
        "--instructions-per-line",
        "8",
        "--state-out",
        path_str(&state_path),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(
        !output.status.success(),
        "the removed option must be rejected"
    );
    assert!(
        stderr.contains("unknown `megadrive trace` option `--instructions-per-line`"),
        "typed rejection names the removed option: {stderr}"
    );
    assert!(
        !state_path.exists(),
        "no state is written on rejection"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_help_documents_formats_and_options() {
    let output = run(&["megadrive", "trace", "--help"]);
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(output.status.success());
    for expected in [
        "Usage:",
        "--rom <path>",
        "--state-out <path>",
        "Trace format:",
        r"<index>\t<pc_hex>\t<opcode_hex>",
        "Final-state format:",
        "chronofork-megadrive-state v1",
        "ram_base=00ff0000",
        "level 7 is always taken",
        "stays pending",
        "RTE",
        "master-cycle timeline",
        "3420",
    ] {
        assert!(
            stdout.contains(expected),
            "help text is missing `{expected}`"
        );
    }
    for stale in [
        "does not gate interrupts",
        "E2-15",
        "--instructions-per-line 1000000",
        "must therefore pass",
    ] {
        assert!(
            !stdout.contains(stale),
            "stale interrupt help `{stale}` must be gone: {stdout}"
        );
    }
}

#[test]
fn zero_trace_capacity_is_rejected() {
    let dir = temp_dir("zero-capacity");
    let rom = write_rom(&dir);
    let state_path = dir.join("state.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--trace-capacity",
        "0",
        "--state-out",
        path_str(&state_path),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("must be at least 1"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_and_state_outputs_must_not_be_the_same_file() {
    let dir = temp_dir("same-output");
    let rom = write_rom(&dir);
    let output_path = dir.join("both.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--trace-out",
        path_str(&output_path),
        "--state-out",
        path_str(&output_path),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(
        stderr.contains("must name different files"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_rom_is_a_clear_error() {
    let dir = temp_dir("missing-rom");
    let state_path = dir.join("state.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&dir.join("absent.rom")),
        "--state-out",
        path_str(&state_path),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("failed to read ROM"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn zero_instruction_budget_writes_an_empty_trace_and_initial_state() {
    let dir = temp_dir("zero-budget");
    let rom = write_rom(&dir);
    let trace_path = dir.join("trace.tsv");
    let state_path = dir.join("state.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--instructions",
        "0",
        "--trace-out",
        path_str(&trace_path),
        "--state-out",
        path_str(&state_path),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(output.status.success(), "trace run failed: {stderr}");
    assert_eq!(
        std::fs::read_to_string(&trace_path).expect("trace file"),
        ""
    );
    let state = std::fs::read_to_string(&state_path).expect("state file");
    assert!(state.lines().any(|line| line == "executed=0"));
    assert!(state.lines().any(|line| line == "pc=00000200"));
    assert!(state.lines().any(|line| line == "a7=01000000"));
}

// ---------------------------------------------------------------------------
// `chronofork trace` — host structured trace command group (M5, T4)
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use chronofork_core::hash_bytes;
use chronofork_megadrive::{controller_rom, demo_rom, interrupt_rom};
use chronofork_trace::{MAX_TRACE_BYTES, Trace};

fn cli(args: &[&str]) -> String {
    let output = run(args);
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8");
    assert!(
        output.status.success(),
        "command failed: {args:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "successful command wrote to stderr: {stderr}"
    );
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

fn cli_err(args: &[&str]) -> String {
    let output = run(args);
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8");
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {args:?}"
    );
    assert!(
        stdout.is_empty(),
        "a typed failure must not write stdout: {stdout}"
    );
    assert!(
        stderr.starts_with("chronofork: "),
        "typed failure must use the chronofork diagnostic prefix: {stderr}"
    );
    assert_eq!(
        stderr.lines().count(),
        1,
        "typed failure must be one diagnostic line: {stderr}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "typed failure must not panic: {stderr}"
    );
    stderr
}

fn block(stdout: &str, record: &str) -> String {
    let needle = format!("record={record}\n");
    assert!(
        stdout.contains(&needle),
        "missing record={record} block in:\n{stdout}"
    );
    let start = stdout.find(&needle).unwrap_or(0);
    let rest = &stdout[start + needle.len()..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    rest[..end].to_owned()
}

fn record_blocks(stdout: &str) -> Vec<String> {
    stdout
        .split("\n\n")
        .filter(|block| block.starts_with("record=trace_record\n"))
        .map(str::to_owned)
        .collect()
}

fn field(block: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    let value = block.lines().find_map(|line| line.strip_prefix(&prefix));
    assert!(value.is_some(), "missing `{key}` in block:\n{block}");
    value.unwrap_or("").to_owned()
}

fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

fn write_demo_rom(dir: &Path) -> PathBuf {
    write_fixture(dir, "demo.rom", &demo_rom())
}

fn decode(path: &Path) -> Trace {
    let bytes = std::fs::read(path).expect("read trace file");
    Trace::from_bytes(&bytes).expect("decode canonical trace")
}

fn captured_file(dir: &Path, name: &str, steps: &str) -> PathBuf {
    let out = dir.join(name);
    let stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        steps,
        "--out",
        path_str(&out),
    ]);
    assert!(!field(&block(&stdout, "trace_capture"), "records").is_empty());
    out
}

#[test]
fn trace_capture_writes_a_valid_canonical_file() {
    let dir = temp_dir("t4-capture");
    let first = dir.join("first.cftrace");
    let stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "8",
        "--out",
        path_str(&first),
    ]);
    let capture = block(&stdout, "trace_capture");
    assert!(stdout.starts_with("record=trace_capture\n"));
    assert_eq!(stdout.matches("record=trace_capture").count(), 1);
    assert_eq!(field(&capture, "backend"), "chronofork.test.synthetic");
    assert_eq!(field(&capture, "start_tick"), "0");
    assert_eq!(field(&capture, "end_tick"), "8");
    assert_eq!(field(&capture, "steps"), "8");
    assert_eq!(field(&capture, "capacity"), "4096");
    assert_eq!(field(&capture, "records"), "8");
    assert_eq!(field(&capture, "first_index"), "0");
    assert_eq!(field(&capture, "dropped"), "0");
    assert_eq!(field(&capture, "stop_reason"), "step_limit");
    assert_eq!(field(&capture, "triggered"), "1");
    assert_eq!(field(&capture, "events_observed"), "0");
    assert_eq!(field(&capture, "kinds"), "instruction,bus,device,interrupt");
    for group in ["instruction", "bus", "device", "interrupt"] {
        assert_eq!(field(&capture, &format!("{group}_enabled")), "1");
    }
    let bytes = std::fs::read(&first).expect("read capture file");
    assert_eq!(bytes.len().to_string(), field(&capture, "bytes"));
    assert_eq!(hash_bytes(&bytes).to_hex(), field(&capture, "sha256"));
    assert_eq!(&bytes[..8], b"CFTRACE1");
    let trace = Trace::from_bytes(&bytes).expect("canonical trace");
    assert_eq!(
        trace.to_bytes(),
        bytes,
        "file is byte-identical canonical v1"
    );
    assert_eq!(trace.record_count(), 8);
    assert!(
        trace
            .records()
            .iter()
            .all(|record| matches!(record, chronofork_trace::TraceRecord::Instruction { .. }))
    );

    // The default step budget is 1000; a second identical run is byte-identical.
    let defaults = dir.join("defaults.cftrace");
    let default_stdout = cli(&["trace", "capture", "--out", path_str(&defaults)]);
    let default_block = block(&default_stdout, "trace_capture");
    assert_eq!(field(&default_block, "steps"), "1000");
    assert_eq!(field(&default_block, "capacity"), "4096");
    assert_eq!(field(&default_block, "records"), "1000");
    let second = dir.join("second.cftrace");
    let second_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "8",
        "--out",
        path_str(&second),
    ]);
    assert_eq!(stdout, second_stdout, "capture report is deterministic");
    assert_eq!(
        std::fs::read(&first).expect("first"),
        std::fs::read(&second).expect("second"),
        "capture bytes are deterministic"
    );

    // --input changes only the recorded experiment input.
    let input = dir.join("input.cftrace");
    let input_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "8",
        "--input",
        "5",
        "--out",
        path_str(&input),
    ]);
    assert_eq!(
        field(&block(&input_stdout, "trace_capture"), "records"),
        "8"
    );

    // Circular retention reports absolute indexes and dropped records.
    let ring = dir.join("ring.cftrace");
    let ring_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "8",
        "--capacity",
        "3",
        "--out",
        path_str(&ring),
    ]);
    let ring_block = block(&ring_stdout, "trace_capture");
    assert_eq!(field(&ring_block, "capacity"), "3");
    assert_eq!(field(&ring_block, "records"), "3");
    assert_eq!(field(&ring_block, "dropped"), "5");
    assert_eq!(field(&ring_block, "first_index"), "5");
    let ring_trace = decode(&ring);
    let indexes: Vec<u64> = ring_trace
        .records()
        .iter()
        .map(chronofork_trace::TraceRecord::index)
        .collect();
    assert_eq!(indexes, vec![5, 6, 7]);

    // Unselected kind groups report *_enabled=0 instead of a record table.
    let kinds = dir.join("kinds.cftrace");
    let kinds_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "5",
        "--kinds",
        "device",
        "--out",
        path_str(&kinds),
    ]);
    let kinds_block = block(&kinds_stdout, "trace_capture");
    assert_eq!(field(&kinds_block, "kinds"), "device");
    assert_eq!(field(&kinds_block, "instruction_enabled"), "0");
    assert_eq!(field(&kinds_block, "bus_enabled"), "0");
    assert_eq!(field(&kinds_block, "device_enabled"), "1");
    assert_eq!(field(&kinds_block, "interrupt_enabled"), "0");
    assert_eq!(field(&kinds_block, "records"), "0");
    assert_eq!(decode(&kinds).record_count(), 0);

    // Start and stop triggers are inclusive and stop-before-execute.
    let stop = dir.join("stop.cftrace");
    let stop_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "100",
        "--stop-pc",
        "0x3",
        "--out",
        path_str(&stop),
    ]);
    let stop_block = block(&stop_stdout, "trace_capture");
    assert_eq!(field(&stop_block, "stop_reason"), "stop_condition");
    assert_eq!(field(&stop_block, "triggered"), "1");
    assert_eq!(field(&stop_block, "end_tick"), "2");
    assert_eq!(field(&stop_block, "steps"), "2");
    assert_eq!(field(&stop_block, "records"), "2");
    let stop_trace = decode(&stop);
    assert!(!stop_trace.records().iter().any(|record| matches!(
        record,
        chronofork_trace::TraceRecord::Instruction { pc: 3, .. }
    )));

    let started = dir.join("started.cftrace");
    let started_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "3",
        "--start-pc",
        "0x3",
        "--out",
        path_str(&started),
    ]);
    let started_block = block(&started_stdout, "trace_capture");
    assert_eq!(field(&started_block, "triggered"), "1");
    assert_eq!(field(&started_block, "records"), "1");
    assert_eq!(field(&started_block, "steps"), "3");
    let started_trace = decode(&started);
    assert!(matches!(
        started_trace.records()[0],
        chronofork_trace::TraceRecord::Instruction { pc: 3, tick, .. } if tick.raw() == 2
    ));

    let both = dir.join("both.cftrace");
    let both_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "5",
        "--start-tick",
        "2",
        "--stop-tick",
        "2",
        "--out",
        path_str(&both),
    ]);
    let both_block = block(&both_stdout, "trace_capture");
    assert_eq!(field(&both_block, "stop_reason"), "stop_condition");
    assert_eq!(field(&both_block, "triggered"), "0");
    assert_eq!(field(&both_block, "records"), "0");
    assert_eq!(field(&both_block, "end_tick"), "2");

    let events = dir.join("events.cftrace");
    let events_stdout = cli(&[
        "trace",
        "capture",
        "--steps",
        "4",
        "--event-capacity",
        "16",
        "--out",
        path_str(&events),
    ]);
    assert_eq!(
        field(&block(&events_stdout, "trace_capture"), "events_observed"),
        "0"
    );

    // Info re-reads the same canonical file with matching metadata.
    let info = cli(&["trace", "info", "--file", path_str(&first)]);
    let info_block = block(&info, "trace_info");
    assert!(info_block.starts_with("format_version=1\n"));
    for key in [
        "backend",
        "backend_version",
        "start_tick",
        "end_tick",
        "capacity",
        "records",
        "first_index",
        "dropped",
        "stop_reason",
        "triggered",
        "events_observed",
        "bytes",
        "sha256",
    ] {
        assert_eq!(field(&info_block, key), field(&capture, key), "field {key}");
    }
    assert_eq!(field(&info_block, "state_hash").len(), 64);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_capture_targets_the_megadrive_slice() {
    let dir = temp_dir("t4-megadrive");
    let rom = write_demo_rom(&dir);
    let out = dir.join("demo.cftrace");
    let stdout = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&rom),
        "--steps",
        "40",
        "--out",
        path_str(&out),
    ]);
    let capture = block(&stdout, "trace_capture");
    assert_eq!(field(&capture, "backend"), "chronofork.megadrive.m68000");
    assert!(!field(&capture, "backend_version").is_empty());
    assert_eq!(field(&capture, "start_tick"), "0");
    assert_eq!(field(&capture, "end_tick"), "3052");
    assert_eq!(field(&capture, "steps"), "3052");
    assert_eq!(field(&capture, "capacity"), "4096");
    assert_eq!(field(&capture, "stop_reason"), "step_limit");
    assert_eq!(field(&capture, "triggered"), "1");
    assert_eq!(field(&capture, "events_observed"), "1");
    assert_eq!(field(&capture, "kinds"), "instruction,bus,device,interrupt");
    let records: usize = field(&capture, "records").parse().expect("record count");
    assert!(records > 40, "events must add records: got {records}");
    let trace = decode(&out);
    let instructions: Vec<_> = trace
        .records()
        .iter()
        .filter(|record| record.kind() == chronofork_trace::TraceKind::Instruction)
        .collect();
    assert_eq!(instructions.len(), 40);
    assert!(
        trace
            .records()
            .iter()
            .any(|record| record.kind() == chronofork_trace::TraceKind::BusWrite)
    );
    assert!(
        trace
            .records()
            .iter()
            .any(|record| record.kind() == chronofork_trace::TraceKind::VdpPort)
    );
    assert!(
        !trace
            .records()
            .iter()
            .any(|record| record.kind() == chronofork_trace::TraceKind::ControllerRead)
    );
    let bytes = std::fs::read(&out).expect("read canonical file");
    assert_eq!(field(&capture, "bytes"), bytes.len().to_string());
    assert_eq!(field(&capture, "sha256"), hash_bytes(&bytes).to_hex());

    // `trace info` reports the same metadata and file identity.
    let info = cli(&["trace", "info", "--file", path_str(&out)]);
    let info_block = block(&info, "trace_info");
    assert_eq!(field(&info_block, "format_version"), "1");
    for key in [
        "backend",
        "backend_version",
        "start_tick",
        "end_tick",
        "capacity",
        "records",
        "first_index",
        "dropped",
        "stop_reason",
        "triggered",
        "events_observed",
        "bytes",
        "sha256",
    ] {
        assert_eq!(field(&info_block, key), field(&capture, key), "field {key}");
    }
    assert_eq!(
        field(&info_block, "state_hash"),
        trace.meta().state_hash.to_string()
    );

    // Query: instruction rows carry only instruction fields.
    let query = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&out),
        "--kind",
        "instruction",
        "--limit",
        "5",
    ]);
    let summary = block(&query, "trace_query");
    assert_eq!(field(&summary, "matched"), "40");
    assert_eq!(field(&summary, "returned"), "5");
    assert_eq!(field(&summary, "truncated"), "1");
    assert_eq!(field(&summary, "from_tick"), "-");
    assert_eq!(field(&summary, "to_tick"), "-");
    assert_eq!(field(&summary, "kind"), "instruction");
    assert_eq!(field(&summary, "instruction_enabled"), "1");
    assert_eq!(field(&summary, "bus_enabled"), "0");
    assert_eq!(field(&summary, "device_enabled"), "0");
    assert_eq!(field(&summary, "interrupt_enabled"), "0");
    assert_eq!(field(&summary, "pc"), "-");
    assert_eq!(field(&summary, "address"), "-");
    assert_eq!(field(&summary, "limit"), "5");
    let rows = record_blocks(&query);
    assert_eq!(rows.len(), 5);
    assert_eq!(field(&rows[0], "index"), "0");
    assert_eq!(field(&rows[0], "tick"), "0");
    assert_eq!(field(&rows[0], "kind"), "instruction");
    assert_eq!(field(&rows[0], "pc"), "000200");
    assert_eq!(field(&rows[0], "opcode"), "7001");
    assert!(
        !rows[0].contains("address="),
        "instruction rows must not carry address"
    );
    assert!(
        !rows[0].contains("width="),
        "instruction rows must not carry width"
    );

    // Bus rows carry address/value/width and never pc.
    let bus = cli(&["trace", "query", "--file", path_str(&out), "--kind", "bus"]);
    let bus_summary = block(&bus, "trace_query");
    let bus_matched: usize = field(&bus_summary, "matched").parse().expect("matched");
    assert!(bus_matched > 0);
    assert_eq!(field(&bus_summary, "returned"), bus_matched.to_string());
    assert_eq!(field(&bus_summary, "truncated"), "0");
    assert_eq!(field(&bus_summary, "bus_enabled"), "1");
    assert_eq!(field(&bus_summary, "instruction_enabled"), "0");
    let bus_rows = record_blocks(&bus);
    assert_eq!(bus_rows.len(), bus_matched);
    assert!(bus_rows.iter().all(|row| field(row, "kind") == "bus_write"));
    assert!(
        bus_rows
            .iter()
            .any(|row| field(row, "address") == "00FF0000")
    );
    assert!(bus_rows.iter().all(|row| !row.contains("pc=")));

    // Device rows combine VDP port and controller reads.
    let device = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&out),
        "--kind",
        "device",
    ]);
    let device_summary = block(&device, "trace_query");
    assert_eq!(field(&device_summary, "device_enabled"), "1");
    assert_eq!(field(&device_summary, "bus_enabled"), "0");
    let device_rows = record_blocks(&device);
    assert!(device_rows.iter().any(|row| {
        field(row, "kind") == "vdp_port"
            && field(row, "offset") == "04"
            && field(row, "value") == "00008F02"
            && field(row, "width") == "2"
            && field(row, "write") == "1"
    }));

    // pc/address filters and inclusive tick windows.
    let by_pc = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&out),
        "--kind",
        "instruction",
        "--pc",
        "0x200",
    ]);
    assert_eq!(field(&block(&by_pc, "trace_query"), "matched"), "1");
    let by_address = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&out),
        "--address",
        "00FF0000",
    ]);
    let by_address_summary = block(&by_address, "trace_query");
    assert_ne!(field(&by_address_summary, "matched"), "0");
    assert_eq!(field(&by_address_summary, "kind"), "all");
    assert_eq!(field(&by_address_summary, "address"), "00FF0000");
    let window = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&out),
        "--kind",
        "instruction",
        "--from-tick",
        "28",
        "--to-tick",
        "84",
    ]);
    let window_summary = block(&window, "trace_query");
    assert_eq!(field(&window_summary, "matched"), "3");
    assert_eq!(field(&window_summary, "from_tick"), "28");
    assert_eq!(field(&window_summary, "to_tick"), "84");
    let ticks: Vec<String> = record_blocks(&window)
        .iter()
        .map(|row| field(row, "tick"))
        .collect();
    assert_eq!(ticks, vec!["28", "56", "84"]);

    // A zero-match query prints the summary only.
    let empty = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&out),
        "--pc",
        "0xFFFFF",
    ]);
    let empty_summary = block(&empty, "trace_query");
    assert_eq!(field(&empty_summary, "matched"), "0");
    assert_eq!(field(&empty_summary, "returned"), "0");
    assert_eq!(field(&empty_summary, "truncated"), "0");
    assert!(record_blocks(&empty).is_empty());

    // Controller reads are device records.
    let controller = write_fixture(&dir, "controller.rom", &controller_rom());
    let controller_out = dir.join("controller.cftrace");
    let controller_stdout = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&controller),
        "--steps",
        "20",
        "--out",
        path_str(&controller_out),
    ]);
    assert_eq!(
        field(
            &block(&controller_stdout, "trace_capture"),
            "events_observed"
        ),
        "1"
    );
    let controller_query = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&controller_out),
        "--kind",
        "device",
    ]);
    assert!(record_blocks(&controller_query).iter().any(|row| {
        field(row, "kind") == "controller_read"
            && field(row, "port") == "1"
            && field(row, "value") == "7F"
    }));

    // `--input` reaches the executed steps: a non-neutral controller frame changes
    // the observed controller-read value (neutral 0x7F vs Up 0x7E) and therefore
    // the canonical trace bytes, not just the instruction count.
    let neutral_out = dir.join("controller-neutral.cftrace");
    let pressed_out = dir.join("controller-pressed.cftrace");
    for (input, out) in [("0", &neutral_out), ("1", &pressed_out)] {
        let stdout = cli(&[
            "trace",
            "capture",
            "--rom",
            path_str(&controller),
            "--steps",
            "20",
            "--input",
            input,
            "--out",
            path_str(out),
        ]);
        assert_eq!(
            field(&block(&stdout, "trace_capture"), "steps"),
            "1526",
            "capture steps for --input {input}"
        );
    }
    assert_ne!(
        std::fs::read(&neutral_out).expect("neutral trace"),
        std::fs::read(&pressed_out).expect("pressed trace"),
        "a non-neutral --input must change the canonical trace"
    );
    let pressed_query = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&pressed_out),
        "--kind",
        "device",
    ]);
    assert!(
        record_blocks(&pressed_query).iter().any(|row| {
            field(row, "kind") == "controller_read"
                && field(row, "port") == "1"
                && field(row, "value") == "7E"
        }),
        "non-neutral input was not recorded: {:?}",
        record_blocks(&pressed_query)
    );
    let neutral_query = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&neutral_out),
        "--kind",
        "device",
    ]);
    assert!(record_blocks(&neutral_query).iter().any(|row| {
        field(row, "kind") == "controller_read"
            && field(row, "port") == "1"
            && field(row, "value") == "7F"
    }));

    // Interrupt phases are interrupt records.
    let interrupts = write_fixture(&dir, "interrupt.rom", &interrupt_rom(3));
    let interrupt_out = dir.join("interrupt.cftrace");
    let interrupt_stdout = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&interrupts),
        "--steps",
        "400",
        "--out",
        path_str(&interrupt_out),
    ]);
    assert_eq!(
        field(
            &block(&interrupt_stdout, "trace_capture"),
            "events_observed"
        ),
        "1"
    );
    let interrupt_query = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&interrupt_out),
        "--kind",
        "interrupt",
    ]);
    let interrupt_rows = record_blocks(&interrupt_query);
    for phase in ["requested", "delivered", "acknowledged"] {
        assert!(
            interrupt_rows.iter().any(|row| {
                field(row, "kind") == "interrupt"
                    && field(row, "level") == "4"
                    && field(row, "phase") == phase
            }),
            "missing interrupt phase `{phase}` in {interrupt_rows:?}"
        );
    }
    assert!(interrupt_rows.iter().all(|row| !row.contains("pc=")));

    // Circular retention on the Mega Drive slice.
    let ring = dir.join("ring.cftrace");
    let ring_stdout = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&rom),
        "--steps",
        "40",
        "--capacity",
        "5",
        "--out",
        path_str(&ring),
    ]);
    let ring_block = block(&ring_stdout, "trace_capture");
    assert_eq!(field(&ring_block, "records"), "5");
    let dropped: u64 = field(&ring_block, "dropped").parse().expect("dropped");
    let first_index: u64 = field(&ring_block, "first_index")
        .parse()
        .expect("first index");
    assert!(
        dropped >= 35,
        "the ring evicted at least one record per step: {dropped}"
    );
    assert_eq!(first_index, dropped);

    // A stop-PC trigger stops before executing that instruction.
    let stop = dir.join("stop.cftrace");
    let stop_stdout = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&rom),
        "--steps",
        "100",
        "--stop-pc",
        "0x206",
        "--out",
        path_str(&stop),
    ]);
    let stop_block = block(&stop_stdout, "trace_capture");
    assert_eq!(field(&stop_block, "stop_reason"), "stop_condition");
    assert_eq!(field(&stop_block, "end_tick"), "84");
    assert_eq!(field(&stop_block, "records"), "3");
    let stop_trace = decode(&stop);
    let stopped_pcs: Vec<u32> = stop_trace
        .records()
        .iter()
        .filter_map(|record| match record {
            chronofork_trace::TraceRecord::Instruction { pc, .. } => Some(*pc),
            _ => None,
        })
        .collect();
    assert_eq!(stopped_pcs, vec![0x200, 0x202, 0x204]);

    // Defaults and repeated-run byte identity.
    let defaults_out = dir.join("defaults.cftrace");
    let defaults_stdout = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&rom),
        "--steps",
        "10",
        "--out",
        path_str(&defaults_out),
    ]);
    let defaults_block = block(&defaults_stdout, "trace_capture");
    assert_eq!(field(&defaults_block, "capacity"), "4096");
    assert_eq!(field(&defaults_block, "events_observed"), "1");
    let repeat = dir.join("repeat.cftrace");
    let repeat_stdout = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&rom),
        "--steps",
        "40",
        "--out",
        path_str(&repeat),
    ]);
    assert_eq!(stdout, repeat_stdout);
    assert_eq!(
        std::fs::read(&out).expect("first"),
        std::fs::read(&repeat).expect("repeat")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_compare_reports_first_divergence() {
    let dir = temp_dir("t4-compare");
    let base = captured_file(&dir, "base.cftrace", "8");
    let short = captured_file(&dir, "short.cftrace", "6");
    let longer_input = {
        let path = dir.join("late.cftrace");
        let _ = cli(&[
            "trace",
            "capture",
            "--steps",
            "8",
            "--start-tick",
            "4",
            "--out",
            path_str(&path),
        ]);
        path
    };
    let wider_capacity = {
        let path = dir.join("wide.cftrace");
        let _ = cli(&[
            "trace",
            "capture",
            "--steps",
            "8",
            "--capacity",
            "4097",
            "--out",
            path_str(&path),
        ]);
        path
    };

    let identical = cli(&[
        "trace",
        "compare",
        "--left",
        path_str(&base),
        "--right",
        path_str(&base),
    ]);
    let view = block(&identical, "trace_compare");
    assert_eq!(field(&view, "identical"), "1");
    assert_eq!(field(&view, "metadata_equal"), "1");
    assert_eq!(field(&view, "divergence"), "-1");
    assert_eq!(field(&view, "divergence_tick"), "-1");
    assert_eq!(field(&view, "reason"), "none");
    assert_eq!(field(&view, "left"), "-");
    assert_eq!(field(&view, "right"), "-");
    assert_eq!(identical.matches("record=trace_compare").count(), 1);

    // One side ends first: the first extra absolute index is the divergence.
    let length = cli(&[
        "trace",
        "compare",
        "--left",
        path_str(&base),
        "--right",
        path_str(&short),
    ]);
    let view = block(&length, "trace_compare");
    assert_eq!(field(&view, "identical"), "0");
    assert_eq!(field(&view, "metadata_equal"), "0");
    assert_eq!(field(&view, "divergence"), "6");
    assert_eq!(field(&view, "divergence_tick"), "6");
    assert_eq!(field(&view, "reason"), "right_missing");
    assert!(field(&view, "left").contains("index=6"));
    assert_eq!(field(&view, "right"), "-");

    let reversed = cli(&[
        "trace",
        "compare",
        "--left",
        path_str(&short),
        "--right",
        path_str(&base),
    ]);
    let view = block(&reversed, "trace_compare");
    assert_eq!(field(&view, "divergence"), "6");
    assert_eq!(field(&view, "reason"), "left_missing");
    assert_eq!(field(&view, "left"), "-");
    assert!(field(&view, "right").contains("index=6"));

    // A differing record at the same absolute index wins over metadata.
    let changed = cli(&[
        "trace",
        "compare",
        "--left",
        path_str(&base),
        "--right",
        path_str(&longer_input),
    ]);
    let view = block(&changed, "trace_compare");
    assert_eq!(field(&view, "identical"), "0");
    assert_eq!(field(&view, "divergence"), "0");
    assert_eq!(field(&view, "divergence_tick"), "0");
    assert_eq!(field(&view, "reason"), "record");
    assert!(field(&view, "left").contains("instruction index=0 tick=0"));
    assert!(field(&view, "right").contains("instruction index=0 tick=4"));

    // Metadata-only differences keep record identity.
    let metadata_only = cli(&[
        "trace",
        "compare",
        "--left",
        path_str(&base),
        "--right",
        path_str(&wider_capacity),
    ]);
    let view = block(&metadata_only, "trace_compare");
    assert_eq!(field(&view, "identical"), "1");
    assert_eq!(field(&view, "metadata_equal"), "0");
    assert_eq!(field(&view, "divergence"), "-1");
    assert_eq!(field(&view, "reason"), "none");

    // A divergent comparison is a completed command (cli() asserts exit 0 and
    // empty stderr); invalid files are typed failures.
    let corrupt = write_fixture(&dir, "corrupt.cftrace", b"not a trace");
    let stderr = cli_err(&[
        "trace",
        "compare",
        "--left",
        path_str(&base),
        "--right",
        path_str(&corrupt),
    ]);
    assert!(stderr.contains("invalid trace file"), "stderr: {stderr}");
    assert!(stderr.contains("invalid_magic"), "stderr: {stderr}");
    let _ = cli_err(&[
        "trace",
        "compare",
        "--left",
        path_str(&base),
        "--right",
        path_str(&dir.join("absent.cftrace")),
    ]);
    let _ = cli_err(&["trace", "compare", "--left", path_str(&base)]);
    let _ = cli_err(&["trace", "compare", "--right", path_str(&base)]);
    let _ = cli_err(&["trace", "compare"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_export_is_one_way_and_deterministic() {
    let dir = temp_dir("t4-export");
    let source = captured_file(&dir, "source.cftrace", "10");
    let trace = decode(&source);
    let source_bytes = std::fs::read(&source).expect("source bytes");

    let canonical = dir.join("canonical.cftrace");
    let stdout = cli(&[
        "trace",
        "export",
        "--file",
        path_str(&source),
        "--format",
        "canonical",
        "--out",
        path_str(&canonical),
    ]);
    let view = block(&stdout, "trace_export");
    assert_eq!(field(&view, "format"), "canonical");
    assert_eq!(field(&view, "records"), "10");
    let canonical_bytes = std::fs::read(&canonical).expect("canonical bytes");
    assert_eq!(field(&view, "bytes"), canonical_bytes.len().to_string());
    assert_eq!(
        field(&view, "sha256"),
        hash_bytes(&canonical_bytes).to_hex()
    );
    assert_eq!(
        canonical_bytes, source_bytes,
        "canonical export must be byte-identical to the source"
    );
    assert_eq!(
        canonical_bytes,
        trace.to_bytes(),
        "canonical export must be the library encoder output"
    );

    // Re-exporting the canonical copy is byte-identical and deterministic.
    let round_trip = dir.join("round-trip.cftrace");
    let second_stdout = cli(&[
        "trace",
        "export",
        "--file",
        path_str(&canonical),
        "--format",
        "canonical",
        "--out",
        path_str(&round_trip),
    ]);
    assert_eq!(stdout, second_stdout);
    assert_eq!(
        std::fs::read(&round_trip).expect("round-trip bytes"),
        canonical_bytes
    );

    // --format defaults to tsv and matches the library export exactly.
    let tsv = dir.join("trace.tsv");
    let tsv_stdout = cli(&[
        "trace",
        "export",
        "--file",
        path_str(&source),
        "--out",
        path_str(&tsv),
    ]);
    let tsv_block = block(&tsv_stdout, "trace_export");
    assert_eq!(field(&tsv_block, "format"), "tsv");
    assert_eq!(field(&tsv_block, "records"), "10");
    let tsv_bytes = std::fs::read(&tsv).expect("TSV bytes");
    assert_eq!(field(&tsv_block, "bytes"), tsv_bytes.len().to_string());
    assert_eq!(field(&tsv_block, "sha256"), hash_bytes(&tsv_bytes).to_hex());
    let expected_tsv = trace.to_tsv();
    assert_eq!(
        String::from_utf8(tsv_bytes).expect("TSV is UTF-8"),
        expected_tsv
    );
    assert!(expected_tsv.starts_with("# chronofork trace v1\n"));
    assert!(expected_tsv.contains("\nindex\tkind\ttick\tpc\taddress\tvalue\twidth\textra\n"));
    let explicit_tsv = dir.join("explicit.tsv");
    let explicit_stdout = cli(&[
        "trace",
        "export",
        "--file",
        path_str(&source),
        "--format",
        "tsv",
        "--out",
        path_str(&explicit_tsv),
    ]);
    assert_eq!(tsv_stdout, explicit_stdout);
    assert_eq!(
        std::fs::read(&explicit_tsv).expect("explicit TSV"),
        expected_tsv.as_bytes()
    );

    // An invalid input is rejected before any output file is created.
    let corrupt = write_fixture(&dir, "corrupt.cftrace", b"not a trace");
    let rejected = dir.join("rejected.cftrace");
    let stderr = cli_err(&[
        "trace",
        "export",
        "--file",
        path_str(&corrupt),
        "--format",
        "canonical",
        "--out",
        path_str(&rejected),
    ]);
    assert!(stderr.contains("invalid trace file"), "stderr: {stderr}");
    assert!(!rejected.exists(), "a failed export must not create output");

    // A directory output path is a typed failure and leaves the directory alone.
    let directory = dir.join("outdir");
    std::fs::create_dir(&directory).expect("create output directory");
    let _ = cli_err(&[
        "trace",
        "export",
        "--file",
        path_str(&source),
        "--out",
        path_str(&directory),
    ]);
    assert!(directory.is_dir());
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read temp dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".outdir.tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );

    // Required flags and the one-way contract.
    let _ = cli_err(&["trace", "export", "--file", path_str(&source)]);
    let _ = cli_err(&["trace", "export", "--out", path_str(&tsv)]);
    let _ = cli_err(&[
        "trace",
        "export",
        "--file",
        path_str(&source),
        "--format",
        "json",
        "--out",
        path_str(&tsv),
    ]);
    let _ = cli_err(&["trace", "import", "--file", path_str(&source)]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_help_and_typed_rejections_follow_the_contract() {
    let dir = temp_dir("t4-help");
    let valid = captured_file(&dir, "valid.cftrace", "4");
    let empty = write_fixture(&dir, "empty.cftrace", b"");
    let corrupt = write_fixture(&dir, "corrupt.cftrace", b"not a trace");
    let oversized = dir.join("oversized.cftrace");
    {
        let file = std::fs::File::create(&oversized).expect("create oversized trace");
        file.set_len(u64::try_from(MAX_TRACE_BYTES).expect("limit fits") + 1)
            .expect("extend oversized trace");
    }
    let missing = dir.join("absent.cftrace");
    let output_directory = dir.join("outdir");
    std::fs::create_dir(&output_directory).expect("create output directory");
    let never = dir.join("never.cftrace");

    // Help documents exactly the implemented surface, defaults and exit contract.
    let help = cli(&["trace", "--help"]);
    for needle in [
        "Usage:",
        "chronofork trace capture",
        "chronofork trace info --file <FILE>",
        "chronofork trace query --file <FILE>",
        "chronofork trace compare --left <FILE> --right <FILE>",
        "chronofork trace export --file <FILE>",
        "--out <FILE>",
        "--out is required for capture and export",
        "--steps",
        "1000",
        "--capacity",
        "4096",
        "--event-capacity",
        "1024",
        "--kinds is a comma list of instruction, bus, device and interrupt",
        "--start-pc",
        "--start-tick",
        "--stop-pc",
        "--stop-tick",
        "--format tsv|canonical",
        "trace_capture:",
        "trace_info:",
        "trace_query:",
        "trace_record:",
        "trace_compare:",
        "trace_export:",
        "host feature over backend inspection",
        "debug_events",
        "exit 1",
        "chronofork:",
        "no TSV import",
    ] {
        assert!(help.contains(needle), "help is missing `{needle}`:\n{help}");
    }
    let top = cli(&["help"]);
    assert!(top.contains("chronofork trace capture"));
    assert!(top.contains("chronofork trace --help"));

    // Subcommand help is accepted with no other arguments and rejected otherwise.
    assert_eq!(cli(&["trace", "capture", "--help"]), help);
    let _ = cli_err(&["trace", "--help", "extra"]);
    let _ = cli_err(&["trace", "capture", "--help", "--steps", "2"]);

    // Command group and subcommand discovery.
    let _ = cli_err(&["trace"]);
    let _ = cli_err(&["trace", "bogus"]);

    // Capture flag/value rejection matrix.
    let _ = cli_err(&["trace", "capture", "--bogus"]);
    let _ = cli_err(&["trace", "capture", "--steps"]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--steps",
        "abc",
        "--out",
        path_str(&never),
    ]);
    for steps in ["0", "1000001"] {
        let _ = cli_err(&[
            "trace",
            "capture",
            "--steps",
            steps,
            "--out",
            path_str(&never),
        ]);
    }
    for capacity in ["0", "1000001"] {
        let _ = cli_err(&[
            "trace",
            "capture",
            "--capacity",
            capacity,
            "--out",
            path_str(&never),
        ]);
    }
    for event_capacity in ["0", "65537"] {
        let _ = cli_err(&[
            "trace",
            "capture",
            "--event-capacity",
            event_capacity,
            "--out",
            path_str(&never),
        ]);
    }
    let _ = cli_err(&["trace", "capture", "--kinds", "", "--out", path_str(&never)]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--kinds",
        "instruction,instruction",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--kinds",
        "instruction,bogus",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--start-pc",
        "0x200",
        "--start-tick",
        "1",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--stop-pc",
        "0x200",
        "--stop-tick",
        "1",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--steps",
        "2",
        "--steps",
        "3",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&["trace", "capture", "--steps", "2"]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--steps",
        "2",
        "--out",
        path_str(&output_directory),
    ]);
    for value in ["18446744073709551616", "999999999999999999999999", "-1"] {
        let _ = cli_err(&[
            "trace",
            "capture",
            "--steps",
            value,
            "--out",
            path_str(&never),
        ]);
    }
    let _ = cli_err(&[
        "trace",
        "capture",
        "--start-pc",
        "0x1FFFFFFFF",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--start-pc",
        "0x200",
        "--start-pc",
        "0x202",
        "--out",
        path_str(&never),
    ]);
    let short_rom = write_fixture(&dir, "short.rom", &[0_u8; 4]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--rom",
        path_str(&short_rom),
        "--steps",
        "2",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&[
        "trace",
        "capture",
        "--rom",
        path_str(&missing),
        "--steps",
        "2",
        "--out",
        path_str(&never),
    ]);
    // A tiny backend event ring overflows between drains: a typed error instead
    // of silent loss (and no output file).
    let interrupt_rom_path = write_fixture(&dir, "interrupt.rom", &interrupt_rom(3));
    let stderr = cli_err(&[
        "trace",
        "capture",
        "--rom",
        path_str(&interrupt_rom_path),
        "--steps",
        "400",
        "--event-capacity",
        "1",
        "--out",
        path_str(&never),
    ]);
    assert!(stderr.contains("event_ring_overflow"), "stderr: {stderr}");
    assert!(!never.exists(), "an overflow must not create its output");
    assert!(
        !never.exists(),
        "a rejected capture must not create its output"
    );
    assert!(output_directory.is_dir());

    // Info validates the canonical input and rejects every malformed class.
    let stderr = cli_err(&["trace", "info"]);
    assert!(stderr.contains("--file"), "stderr: {stderr}");
    for path in [&missing, &empty, &corrupt, &oversized, &output_directory] {
        let _ = cli_err(&["trace", "info", "--file", path_str(path)]);
    }
    let stderr = cli_err(&["trace", "info", "--file", path_str(&corrupt)]);
    assert!(stderr.contains("invalid_magic"), "stderr: {stderr}");
    let stderr = cli_err(&["trace", "info", "--file", path_str(&empty)]);
    assert!(stderr.contains("truncated_input"), "stderr: {stderr}");
    let stderr = cli_err(&["trace", "info", "--file", path_str(&oversized)]);
    assert!(stderr.contains("limit is"), "stderr: {stderr}");
    let unreadable = write_fixture(&dir, "unreadable.cftrace", b"CFTRACE1unreadable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000))
            .expect("make trace unreadable");
        let _ = cli_err(&["trace", "info", "--file", path_str(&unreadable)]);
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o600))
            .expect("restore trace permissions");
    }
    let _ = cli_err(&["trace", "info", "--bogus"]);

    // Query filter bounds and flags.
    let _ = cli_err(&[
        "trace",
        "query",
        "--file",
        path_str(&valid),
        "--from-tick",
        "5",
        "--to-tick",
        "2",
    ]);
    for limit in ["0", "10001"] {
        let _ = cli_err(&[
            "trace",
            "query",
            "--file",
            path_str(&valid),
            "--limit",
            limit,
        ]);
    }
    let _ = cli_err(&[
        "trace",
        "query",
        "--file",
        path_str(&valid),
        "--kind",
        "everything",
    ]);
    let _ = cli_err(&["trace", "query", "--file", path_str(&valid), "--bogus"]);
    let _ = cli_err(&["trace", "query"]);
    let _ = cli_err(&["trace", "query", "--file", path_str(&corrupt)]);

    // Compare and export reject missing flags and unreadable inputs.
    let _ = cli_err(&[
        "trace",
        "compare",
        "--left",
        path_str(&missing),
        "--right",
        path_str(&valid),
    ]);
    let _ = cli_err(&[
        "trace",
        "export",
        "--file",
        path_str(&valid),
        "--format",
        "bogus",
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&["trace", "export", "--file", path_str(&valid)]);
    let _ = cli_err(&[
        "trace",
        "export",
        "--file",
        path_str(&missing),
        "--out",
        path_str(&never),
    ]);
    let _ = cli_err(&[
        "trace",
        "export",
        "--file",
        path_str(&valid),
        "--bogus",
        "--format",
        "tsv",
        "--out",
        path_str(&never),
    ]);
    assert!(!never.exists(), "rejected exports must not create output");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_info_and_query_print_stable_blocks() {
    let dir = temp_dir("t4-info-query");
    let synthetic = captured_file(&dir, "synthetic.cftrace", "5");
    let trace = decode(&synthetic);
    let bytes = std::fs::read(&synthetic).expect("read trace file");
    assert!(!path_str(&dir).is_empty());

    let info = cli(&["trace", "info", "--file", path_str(&synthetic)]);
    let info_block = block(&info, "trace_info");
    assert_eq!(info.matches("record=trace_info").count(), 1);
    assert_eq!(field(&info_block, "format_version"), "1");
    assert_eq!(field(&info_block, "backend"), trace.meta().backend);
    assert_eq!(
        field(&info_block, "backend_version"),
        trace.meta().backend_version
    );
    assert_eq!(
        field(&info_block, "state_hash"),
        trace.meta().state_hash.to_string()
    );
    assert_eq!(field(&info_block, "start_tick"), "0");
    assert_eq!(field(&info_block, "end_tick"), "5");
    assert_eq!(field(&info_block, "capacity"), "4096");
    assert_eq!(field(&info_block, "records"), "5");
    assert_eq!(field(&info_block, "first_index"), "0");
    assert_eq!(field(&info_block, "dropped"), "0");
    assert_eq!(field(&info_block, "stop_reason"), "step_limit");
    assert_eq!(field(&info_block, "triggered"), "1");
    assert_eq!(field(&info_block, "events_observed"), "0");
    assert_eq!(field(&info_block, "bytes"), bytes.len().to_string());
    assert_eq!(field(&info_block, "sha256"), hash_bytes(&bytes).to_hex());
    assert_eq!(
        info,
        cli(&["trace", "info", "--file", path_str(&synthetic)]),
        "info output is deterministic"
    );
    assert!(
        !info.contains(path_str(&dir)),
        "stdout must not contain paths"
    );

    let all = cli(&["trace", "query", "--file", path_str(&synthetic)]);
    let summary = block(&all, "trace_query");
    assert_eq!(field(&summary, "matched"), "5");
    assert_eq!(field(&summary, "returned"), "5");
    assert_eq!(field(&summary, "truncated"), "0");
    assert_eq!(field(&summary, "kind"), "all");
    assert_eq!(field(&summary, "instruction_enabled"), "1");
    assert_eq!(field(&summary, "bus_enabled"), "1");
    assert_eq!(field(&summary, "device_enabled"), "1");
    assert_eq!(field(&summary, "interrupt_enabled"), "1");
    assert_eq!(record_blocks(&all).len(), 5);
    let first = &record_blocks(&all)[0];
    assert_eq!(field(first, "index"), "0");
    assert_eq!(field(first, "tick"), "0");
    assert_eq!(field(first, "kind"), "instruction");
    assert_eq!(field(first, "pc"), "000000");
    assert!(first.contains("opcode="));
    assert_eq!(
        all,
        cli(&["trace", "query", "--file", path_str(&synthetic)]),
        "query output is deterministic"
    );
    assert!(
        !all.contains(path_str(&dir)),
        "stdout must not contain paths"
    );

    let limited = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&synthetic),
        "--limit",
        "2",
    ]);
    let limited_summary = block(&limited, "trace_query");
    assert_eq!(field(&limited_summary, "matched"), "5");
    assert_eq!(field(&limited_summary, "returned"), "2");
    assert_eq!(field(&limited_summary, "truncated"), "1");
    assert_eq!(record_blocks(&limited).len(), 2);

    // A retained window with evicted records reports absolute indexes.
    let ring = dir.join("ring.cftrace");
    let _ = cli(&[
        "trace",
        "capture",
        "--steps",
        "8",
        "--capacity",
        "3",
        "--out",
        path_str(&ring),
    ]);
    let ring_query = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&ring),
        "--kind",
        "instruction",
    ]);
    let ring_summary = block(&ring_query, "trace_query");
    assert_eq!(field(&ring_summary, "matched"), "3");
    let ring_rows = record_blocks(&ring_query);
    assert_eq!(field(&ring_rows[0], "index"), "5");
    assert_eq!(field(&ring_rows[2], "index"), "7");

    // Mega Drive info reports observation and device rows keep their fields.
    let rom = write_demo_rom(&dir);
    let megadrive = dir.join("megadrive.cftrace");
    let _ = cli(&[
        "trace",
        "capture",
        "--rom",
        path_str(&rom),
        "--steps",
        "20",
        "--out",
        path_str(&megadrive),
    ]);
    let info = cli(&["trace", "info", "--file", path_str(&megadrive)]);
    assert_eq!(field(&block(&info, "trace_info"), "events_observed"), "1");
    let devices = cli(&[
        "trace",
        "query",
        "--file",
        path_str(&megadrive),
        "--kind",
        "device",
    ]);
    assert_eq!(
        field(&block(&devices, "trace_query"), "device_enabled"),
        "1"
    );
    assert!(record_blocks(&devices).iter().any(|row| {
        field(row, "kind") == "vdp_port"
            && row.contains("offset=")
            && row.contains("value=")
            && row.contains("width=")
            && row.contains("write=")
            && !row.contains("pc=")
    }));

    let _ = std::fs::remove_dir_all(&dir);
}
