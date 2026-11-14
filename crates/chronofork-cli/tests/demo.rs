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
    for expected in [
        "68000 + VDP port + interrupt scheduler slice",
        "no rendering, audio, Z80",
        // VDP/VRAM evidence is retained.
        "vdp: register 15 (auto-increment)=2",
        "vdp: program wrote and read back VRAM word 0x1234",
        // Masked pending: mask 7 keeps HBlank/VBlank pending, no handler runs.
        "interrupt: masked HBlank pending at tick 64",
        "interrupt: mask 7 blocked delivery at tick 65; request retained, no handler ran",
        "interrupt: masked VBlank pending level 6 vector 0x78",
        "VDP VBlank/VINT bits set",
        // Delivery after the ROM lowers the mask, handler execution and RTE.
        "interrupt: interrupt ROM retained HBlank at tick 7 under mask 7 (no handler)",
        "MOVE.W #$2300,SR lowered the mask to 3 at tick 8",
        "HBlank delivered at tick 9 with handler IPM 4; handler ran; RTE at tick 11",
        "SR=0x2300 (mask 3, CCR restored)",
        "VBlank entered at IPM 6 and blocked the pending HBlank (no nested handler)",
        "snapshot preserved the pending VBlank/VINT state",
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
    for stale in ["does not gate interrupts", "E2-15", "1000000"] {
        assert!(
            !stdout.contains(stale),
            "stale interrupt text `{stale}` must be gone:\n{stdout}"
        );
    }
}

#[test]
fn megadrive_demo_output_is_deterministic() {
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_chronofork"))
            .args(["megadrive", "demo"])
            .output()
            .expect("megadrive demo runs")
    };
    let first = run();
    let second = run();
    assert!(first.status.success(), "first run failed");
    assert!(second.status.success(), "second run failed");
    assert_eq!(
        first.stdout, second.stdout,
        "megadrive demo output must be byte-identical across runs"
    );
    assert_eq!(first.stderr, second.stderr);
}
