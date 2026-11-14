#![allow(clippy::expect_used)]

use std::process::Command;

#[test]
fn demo_runs_the_acceptance_scenario() {
    let output = Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .arg("demo")
        .output()
        .expect("chronofork demo runs");
    let stdout = String::from_utf8(output.stdout).expect("demo stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("demo stderr is UTF-8");
    assert!(
        output.status.success(),
        "demo failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("not a Mega Drive emulator"));
    assert!(stdout.contains("checkpoint:"));
    assert!(stdout.contains("serialized checkpoint:"));
    assert!(stdout.contains("source isolation:"));
    assert!(stdout.contains("memory isolation:"));
    assert!(stdout.contains("replay verification:"));
    assert!(stdout.contains("chronofork demo: PASS"));
}

#[test]
fn backend_describe_reports_real_capabilities() {
    let output = Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(["backend", "describe"])
        .output()
        .expect("backend describe runs");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("describe stdout is UTF-8");
    assert!(stdout.contains("backend: chronofork.test.synthetic"));
    assert!(stdout.contains("test_backend: true"));
    assert!(stdout.contains("capability.breakpoints: false"));
    assert!(stdout.contains("capability.tracing: false"));
}

#[test]
fn megadrive_demo_runs_the_minimal_68000_slice() {
    let output = Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(["megadrive", "demo"])
        .output()
        .expect("megadrive demo runs");
    let stdout = String::from_utf8(output.stdout).expect("megadrive stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("megadrive stderr is UTF-8");
    assert!(
        output.status.success(),
        "megadrive demo failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("68000 + VDP port + interrupt scheduler slice"));
    assert!(stdout.contains("no rendering, audio, Z80"));
    assert!(stdout.contains("vdp: register 15 (auto-increment)=2"));
    assert!(stdout.contains("interrupt: HBlank pending at tick 64"));
    assert!(stdout.contains("interrupt: VBlank pending level 6 vector 0x78"));
    assert!(stdout.contains("snapshot: VDP, scheduler and CPU state reproduced"));
    assert!(stdout.contains("chronofork megadrive demo: PASS"));
}
