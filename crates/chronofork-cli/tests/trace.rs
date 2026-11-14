#![allow(clippy::expect_used)]

//! Integration tests for `chronofork megadrive trace`.
//!
//! They run the built binary against an independently authored synthetic ROM and
//! check the documented trace/state formats, determinism and option validation.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Builds a tiny synthetic cartridge: reset vectors plus a five-instruction
/// program that computes `1 + 2`, stores the word at `0x00FF0000` and continues
/// with NOPs. The ROM contains no copyrighted data.
fn synthetic_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes()); // top of work RAM
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x70..0x74].copy_from_slice(&0x0000_0300_u32.to_be_bytes()); // HBlank autovector
    rom[0x78..0x7C].copy_from_slice(&0x0000_0320_u32.to_be_bytes()); // VBlank autovector
    rom[0x200..0x20E].copy_from_slice(&[
        0x70, 0x01, // MOVEQ #1,D0
        0x72, 0x02, // MOVEQ #2,D1
        0xD0, 0x41, // ADD.W D1,D0
        0x33, 0xC0, 0x00, 0xFF, 0x00, 0x00, // MOVE.W D0,(0x00FF0000).L
        0x4E, 0x71, // NOP
    ]);
    rom
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "chronofork-cli-trace-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_rom(dir: &Path) -> PathBuf {
    let path = dir.join("synthetic.rom");
    std::fs::write(&path, synthetic_rom()).expect("write synthetic ROM");
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .output()
        .expect("chronofork runs")
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("temporary path is UTF-8")
}

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
            "--instructions-per-line",
            "1000000",
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
        "tick=5",
        "instructions_per_line=1000000",
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
        "--instructions-per-line",
        "1000000",
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
        "--instructions-per-line",
        "1000000",
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
fn default_scheduler_quantum_is_still_64_and_is_never_hidden() {
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
    assert!(state.lines().any(|line| line == "instructions_per_line=64"));
    assert!(
        stderr.contains("does not gate interrupts with the SR mask (E2-15)"),
        "the interrupt-mask limitation must not be hidden: {stderr}"
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
        "--instructions-per-line",
        "Trace format:",
        r"<index>\t<pc_hex>\t<opcode_hex>",
        "Final-state format:",
        "chronofork-megadrive-state v1",
        "ram_base=00ff0000",
        "E2-15",
        "--instructions-per-line 1000000",
    ] {
        assert!(
            stdout.contains(expected),
            "help text is missing `{expected}`"
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
        "--instructions-per-line",
        "1000000",
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
