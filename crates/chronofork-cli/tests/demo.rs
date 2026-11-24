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

/// Complete stdout of `chronofork megadrive demo`: 1833 bytes with this SHA-256.
///
/// The golden is a single pinned digest rather than a run-vs-run comparison.
/// The demo's stdout is profile-independent (the debug print path and the release
/// path emit the same bytes) and host-independent, so one run against the golden
/// detects any drift, and a run under the other Cargo profile detects a
/// profile-dependent one. Pinning the digest is what makes the check affordable:
/// measured on the reference host, one debug run costs ~82 s and one release run
/// ~7 s, while the previous two-debug-run comparison cost ~166 s.
const MEGADRIVE_DEMO_STDOUT_SHA256: &str =
    "1659642e29ff9b7ab91e8959db4493fed743d0e7a1b50ab8a0108ce3d6e90b39";

/// Bytes of the pinned `chronofork megadrive demo` stdout.
///
/// Re-minted twice while merging `main` into this branch, and once more by W1Z.
/// Phase 8.4 added the Z80 and 48 kHz audio surface to the narration (1833 bytes
/// before it); the W1 boot work then changed the canonical state the demo prints -
/// the fork/source state hashes - while leaving the narration byte-identical, so
/// only those hash values moved. W1Z (payload v8, the VDP read cache) moved them
/// again for the same reason: the payload/state identity is printed by the demo.
/// Re-minting is the honest response to a state change: this check exists to catch
/// drift. The value was measured from this tree's build, not copied from a report.
const MEGADRIVE_DEMO_STDOUT_LEN: usize = 1994;

/// Enables the "release build must exist" half of the check:
/// `$CHRONOFORK_CROSS_PROFILE_GOLDEN=1`.
///
/// The golden path already runs the release binary whenever that build exists,
/// because release emits the same bytes and costs ~7 s against ~82 s for debug.
/// Setting this variable removes the silent fallback: a run that claims
/// cross-profile parity must have the release build, so a debug-only checkout
/// fails instead of quietly verifying the golden against debug. The release CI
/// job builds both profiles, so it verifies the golden and the debug/release
/// parity in the same run.
const CROSS_PROFILE_ENV: &str = "CHRONOFORK_CROSS_PROFILE_GOLDEN";

/// Runs the built CLI binary at an explicit path.
fn run_binary(binary: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(binary)
        .args(args)
        .output()
        .expect("chronofork runs")
}

/// Returns the release build of `chronofork`, when this tree has one.
///
/// `CARGO_BIN_EXE_chronofork` is the binary of the profile this test was built
/// for, so swapping the profile directory finds the sibling build. Returns the
/// current binary when the test is already built for release.
fn release_binary() -> Option<std::path::PathBuf> {
    let current = std::path::PathBuf::from(env!("CARGO_BIN_EXE_chronofork"));
    if !cfg!(debug_assertions) {
        return current.is_file().then_some(current);
    }
    let candidate = current
        .components()
        .map(|component| {
            if component.as_os_str() == std::ffi::OsStr::new("debug") {
                std::path::PathBuf::from("release")
            } else {
                std::path::PathBuf::from(component.as_os_str())
            }
        })
        .collect::<std::path::PathBuf>();
    candidate.is_file().then_some(candidate)
}

#[test]
fn megadrive_demo_output_is_deterministic() {
    // One release run against the pinned golden is the whole check. This is the
    // cross-profile comparison in the only direction worth paying for: release
    // and debug must emit the same bytes, and release is the fast side, so the
    // previous two-debug-run comparison (~166 s) becomes one ~7 s run.
    let cross_profile = matches!(
        std::env::var(CROSS_PROFILE_ENV).as_deref(),
        Ok("1") | Ok("true")
    );
    let binary = release_binary().unwrap_or_else(|| {
        assert!(
            !cross_profile,
            "${CROSS_PROFILE_ENV} is set, so the release `chronofork` build must exist to verify \
             cross-profile parity; only the debug build is present"
        );
        std::path::PathBuf::from(env!("CARGO_BIN_EXE_chronofork"))
    });

    let first = run_binary(&binary, &["megadrive", "demo"]);
    assert!(first.status.success(), "megadrive demo failed");
    assert!(
        first.stderr.is_empty(),
        "megadrive demo wrote to stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        first.stdout.len(),
        MEGADRIVE_DEMO_STDOUT_LEN,
        "the megadrive demo byte count must not drift"
    );
    assert_eq!(
        chronofork_core::hash_bytes(&first.stdout).to_hex(),
        MEGADRIVE_DEMO_STDOUT_SHA256,
        "the megadrive demo stdout must stay byte-identical to the pinned golden"
    );

    // A debug build compares its own output with the release golden, so one
    // debug run still proves parity instead of only asserting the pin twice.
    if cfg!(debug_assertions) {
        let debug = std::path::Path::new(env!("CARGO_BIN_EXE_chronofork"));
        if debug != binary.as_path() {
            let second = run_binary(debug, &["megadrive", "demo"]);
            assert!(second.status.success(), "the debug build failed to run");
            assert_eq!(
                first.stdout, second.stdout,
                "megadrive demo output must be byte-identical across Cargo profiles"
            );
        }
    }
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
