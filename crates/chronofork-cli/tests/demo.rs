#![allow(clippy::expect_used)]

//! Integration tests for the `chronofork` command wiring.
//!
//! The demo commands are user-facing smoke tests: they must run, exit
//! successfully, stay quiet on stderr and print their key evidence. Semantic
//! fork/isolation/replay invariants are asserted by the sessions acceptance
//! suite; Mega Drive instruction, VDP and interrupt invariants are asserted by
//! the backend tests. Trace and final-state *formats* are contracts and are
//! checked by `trace.rs`.

mod common;

use common::{path_str, run, temp_dir, write_rom};

#[test]
fn demo_command_runs_and_prints_key_evidence() {
    let output = run(&["demo"]);
    let stdout = String::from_utf8(output.stdout).expect("demo stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("demo stderr is UTF-8");
    assert!(
        output.status.success(),
        "demo failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stderr.is_empty(), "demo wrote to stderr:\n{stderr}");
    for expected in [
        "chronofork demo: deterministic session vertical slice",
        "not a Mega Drive emulator",
        "checkpoint:",
        "serialized checkpoint:",
        "forks:",
        "source isolation:",
        "memory isolation:",
        "replay verification:",
        "compare:",
        "chronofork demo: PASS",
    ] {
        assert!(
            stdout.contains(expected),
            "demo output is missing `{expected}`:\n{stdout}"
        );
    }
}

#[test]
fn synthetic_demo_stdout_stays_byte_identical_to_the_baseline() {
    // The synthetic backend demo is not part of the Mega Drive v3 state-format
    // bump, so its complete stdout is pinned: 986 bytes with this SHA-256.
    let output = run(&["demo"]);
    assert!(output.status.success(), "demo failed");
    assert!(
        output.stderr.is_empty(),
        "demo wrote to stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout.len(),
        986,
        "the synthetic demo byte count must not drift"
    );
    assert_eq!(
        chronofork_core::hash_bytes(&output.stdout).to_hex(),
        "9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652"
    );
}

#[test]
fn backend_describe_reports_real_capabilities() {
    let output = run(&["backend", "describe"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("describe stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("describe stderr is UTF-8");
    assert!(stderr.is_empty(), "describe wrote to stderr:\n{stderr}");
    assert!(stdout.contains("backend: chronofork.test.synthetic"));
    assert!(stdout.contains("test_backend: true"));
    assert!(stdout.contains("capability.breakpoints: false"));
    assert!(stdout.contains("capability.debug_events: false"));
    assert!(stdout.contains("capability.tracing: false"));
}

#[test]
fn megadrive_demo_runs_the_minimal_68000_slice() {
    let output = run(&["megadrive", "demo"]);
    let stdout = String::from_utf8(output.stdout).expect("megadrive stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("megadrive stderr is UTF-8");
    assert!(
        output.status.success(),
        "megadrive demo failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.is_empty(),
        "megadrive demo wrote to stderr:\n{stderr}"
    );
    for expected in [
        "chronofork megadrive demo:",
        "68000 + Z80 + VDP + 48 kHz audio capture slice",
        "deterministic headless video frames and 48 kHz stereo audio chunks only",
        "no host window or audio device",
        "YM2612/PSG models and a 48 kHz integer audio mixer",
        // VDP/VRAM evidence is retained.
        "vdp: register 15 (auto-increment)=2",
        "vdp: program wrote and read back VRAM word 0x1234",
        // Masked pending: mask 7 keeps the request pending, no handler runs.
        "interrupt: masked HBlank pending at tick 3472",
        "interrupt: masked VBlank pending level 6 vector 0x78",
        // Delivery after the ROM lowers the mask, handler execution and RTE.
        "interrupt: HBlank delivered at tick 7546 with handler IPM 4; handler ran; RTE at tick 7714 returned to pc=0x218 with SR=0x2300 (mask 3, CCR restored)",
        "interrupt: VBlank entered at tick 766164 with handler IPM 6 and blocked the pending HBlank (level 4, no nested handler)",
        "snapshot restored the pending VBlank/VINT state (level 6)",
        // Fork/restore evidence is retained.
        "fork isolation:",
        "snapshot: VDP, scheduler and CPU state reproduced from the checkpoint",
        "chronofork megadrive demo: PASS",
    ] {
        assert!(
            stdout.contains(expected),
            "megadrive demo output is missing `{expected}`:\n{stdout}"
        );
    }
    for stale in [
        "does not gate interrupts",
        "E2-15",
        "1000000",
        "no rendering",
        "no audio, Z80, interlace or game compatibility",
        "VBlank entered at IPM 6; RTE returned",
    ] {
        assert!(
            !stdout.contains(stale),
            "stale interrupt/rendering text `{stale}` must be gone:\n{stdout}"
        );
    }
}

#[test]
fn megadrive_demo_output_is_deterministic() {
    let first = run(&["megadrive", "demo"]);
    let second = run(&["megadrive", "demo"]);
    assert!(first.status.success(), "first run failed");
    assert!(second.status.success(), "second run failed");
    assert!(first.stderr.is_empty(), "first run wrote to stderr");
    assert!(second.stderr.is_empty(), "second run wrote to stderr");
    assert_eq!(
        first.stdout, second.stdout,
        "megadrive demo output must be byte-identical across runs"
    );
    assert_eq!(first.stderr, second.stderr);
}

#[test]
fn megadrive_trace_is_wired_to_an_external_rom() {
    let dir = temp_dir("trace-wiring");
    let rom = write_rom(&dir);
    let state_path = dir.join("state.txt");
    let output = run(&[
        "megadrive",
        "trace",
        "--rom",
        path_str(&rom),
        "--instructions",
        "2",
        "--state-out",
        path_str(&state_path),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("trace stderr is UTF-8");
    assert!(output.status.success(), "trace run failed:\n{stderr}");
    assert!(
        stderr.contains("executed 2 instruction(s)"),
        "trace summary is missing:\n{stderr}"
    );
    assert!(
        state_path.is_file(),
        "the trace command must write the final-state file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
