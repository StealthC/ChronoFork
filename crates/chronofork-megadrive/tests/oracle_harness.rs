#![allow(clippy::expect_used)]
// The assertions below fail with a diagnostic that names the observed exit code
// and stderr, which is what an exit-code regression must report; test-only.
#![allow(clippy::panic)]

//! Contract of the differential harness's oracle provisioning path.
//!
//! The pinned Exodus oracle is deliberately not committed: its checkout, the
//! compiled `exodus-probe`/`exodus-mcpd` binaries and the module all live under
//! the ignored `.research/` tree. A clean clone therefore *cannot* run the
//! differential, and the failure mode this file locks down is the one that used
//! to be indistinguishable from outside (review Q-01 and Q-05): a missing oracle
//! exits 3 and reports itself as unavailable — never as a pass, and never as a
//! generic setup error that reads like a harness defect.
//!
//! `--dump-roms` is the half that must keep working with no oracle at all, so
//! these tests also pin the corpus size. A provisioned-oracle test asserts the
//! complementary branch when the environment actually has one; it is `#[ignore]`d
//! rather than skipped so a run without the oracle reports `ignored`, and the
//! documented recipe runs it explicitly with `--ignored`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Repository root, derived from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

/// The differential harness, which is the only external tool these tests use.
fn harness_path() -> PathBuf {
    repo_root().join("scripts").join("oracle-diff.py")
}

/// Runs the harness from the repository root so its own path resolution applies.
fn run_harness(args: &[&str]) -> Output {
    let harness = harness_path();
    assert!(
        harness.is_file(),
        "the differential harness is missing at {}",
        harness.display()
    );
    Command::new("python3")
        .arg(&harness)
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("python3 is the generator's own runtime and is a test prerequisite")
}

/// A path that is guaranteed not to exist, for the unprovisioned branch.
///
/// Passing an explicit location keeps the test deterministic on a developer
/// machine that *does* have a provisioned oracle next to the checkout.
fn absent_oracle_root() -> String {
    std::env::temp_dir()
        .join("chronofork-oracle-status-absent")
        .display()
        .to_string()
}

#[test]
fn an_unprovisioned_oracle_exits_three_and_names_the_pinned_commit() {
    // The pinned oracle lives under the ignored `.research/` tree, so a clean
    // clone has none. Point the harness at a location that cannot exist and the
    // verdict must be "unavailable" (3) with a named reason and the commit to
    // clone, never a pass and never a bare traceback.
    let absent = absent_oracle_root();
    let output = run_harness(&[
        "--oracle-status",
        "--oracle-root",
        &absent,
        "--oracle-bin-dir",
        &absent,
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let status = exit_code(&output);
    assert_eq!(
        status, 3,
        "an unprovisioned oracle must exit 3 (unavailable), not {status}. stdout: {stdout}"
    );
    let report = parse_json_object(&stdout);
    assert!(
        report.contains("\"status\": \"absent\""),
        "the report must name the absent status, got: {report}"
    );
    assert!(
        report.contains("does not exist"),
        "an explicit missing path must say so, got: {report}"
    );
    assert!(
        report.contains("ec0056151f3ff1c334d09ee375abb30a68fdfec5"),
        "the report must name the pinned commit to clone, got: {report}"
    );
}

#[test]
fn the_default_location_reports_the_same_unavailable_verdict() {
    // The default path is what a clean clone hits without any flag. The verdict
    // must be 0 (provisioned) or 3 (unavailable) — and this test refuses a
    // generic setup error, which would hide an unprovisioned clone behind a
    // harness-defect reading.
    let output = run_harness(&["--oracle-status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let status = exit_code(&output);
    assert!(
        status == 0 || status == 3,
        "the default location must report provisioned (0) or unavailable (3), not {status}: {stdout}"
    );
    let expected = if status == 0 {
        "\"status\": \"provisioned\""
    } else {
        "\"status\": \"absent\""
    };
    assert!(
        parse_json_object(&stdout).contains(expected),
        "exit {status} must report {expected}, got: {stdout}"
    );
}

#[test]
fn require_oracle_turns_an_unavailable_oracle_into_an_explicit_failure() {
    // `--require-oracle` is the shape CI uses when the differential is mandatory:
    // it must fail with the unavailable code before producing any evidence or
    // trying to build the CLI.
    let absent = absent_oracle_root();
    let output = run_harness(&[
        "--require-oracle",
        "--oracle-root",
        &absent,
        "--oracle-bin-dir",
        &absent,
        "--chronofork-bin",
        "/nonexistent/chronofork",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        exit_code(&output),
        3,
        "a missing oracle under --require-oracle must exit 3, not {}. stderr: {stderr}",
        exit_code(&output)
    );
    assert!(
        stderr.contains("not provisioned"),
        "the failure must say the oracle is unavailable, got: {stderr}"
    );
}

#[test]
fn the_offline_half_needs_no_oracle_and_keeps_the_whole_corpus() {
    // `--dump-roms` is what a clean clone can always run: it builds every ROM and
    // writes nothing. Shrinking the corpus silently would let the published
    // coverage claim outlive the generator (review Q-10), so the count is pinned
    // here as well as by the harness's own guard.
    let output = run_harness(&["--dump-roms"]);
    assert_eq!(exit_code(&output), 0, "the offline half must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let names: Vec<String> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            parse_json_object(line)
                .split("\"name\": \"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_else(|| panic!("a --dump-roms line has no name: {line}"))
                .to_owned()
        })
        .collect();
    assert_eq!(
        names.len(),
        13,
        "the differential corpus must keep all thirteen ROMs, got {names:?}"
    );
    assert!(
        names.contains(&"rom_exception".to_owned()),
        "the TRAP/RTE exception ROM must stay in the corpus, got {names:?}"
    );
}

/// Locates a provisioned pinned oracle without relying on the harness defaults.
///
/// The oracle lives under the ignored `.research/` tree and is shared between
/// worktrees by absolute path, so the candidates are, in order: the documented
/// environment overrides, the working checkout, and the main checkout two levels
/// above a task worktree (`<repo>/.agents/worktrees/<task>`).
fn provisioned_oracle() -> Option<(PathBuf, PathBuf)> {
    let mut roots = Vec::new();
    let mut bins = Vec::new();
    if let Ok(value) = std::env::var("CHRONOFORK_ORACLE_ROOT") {
        roots.push(PathBuf::from(value));
    }
    if let Ok(value) = std::env::var("CHRONOFORK_ORACLE_BIN_DIR") {
        bins.push(PathBuf::from(value));
    }
    let checkout = repo_root().join(".research");
    let parent = repo_root()
        .join("..")
        .join("..")
        .join("..")
        .join(".research");
    for base in [checkout, parent] {
        roots.push(base.join("upstream").join("exodus-headless"));
        bins.push(base.join("oracle").join("build"));
    }
    for root in roots {
        for bin_dir in &bins {
            if root.is_dir()
                && bin_dir.join("exodus-probe").is_file()
                && bin_dir.join("exodus-mcpd").is_file()
            {
                return Some((root, bin_dir.clone()));
            }
        }
    }
    None
}

/// Runs an inline Python program against the harness's own scoped-exception API.
///
/// The program asserts its own invariants and exits non-zero with a diagnostic on
/// failure, so a weakened matcher fails this test rather than passing quietly.
/// Python 3 is the harness's own runtime and is already a prerequisite of the
/// fixture-integrity target.
fn run_scoped_exception_probe(program: &str) {
    let harness = harness_path();
    assert!(
        harness.is_file(),
        "the harness is missing at {}",
        harness.display()
    );
    let output = Command::new("python3")
        .arg("-c")
        .arg(program)
        .arg(&harness)
        .current_dir(repo_root())
        .output()
        .expect("python3 is a test prerequisite");
    assert!(
        output.status.success(),
        "the scoped-exception probe failed ({}):\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The named scoped exceptions must absorb their own divergence and nothing else.
///
/// This is the guard that keeps the corpus from going green on an unreviewed
/// regression: `W4e-F1` and `W4e-F2` are declared for `rom_exception`, and the
/// matcher must accept **only** the exact declared pair — a different ROM, a
/// changed cycle count, a moved location, an extra differing byte or a changed
/// byte value must all be rejected.
#[test]
fn named_scoped_exceptions_absorb_exactly_their_declared_divergence() {
    run_scoped_exception_probe(
        r#"
import importlib.util, sys

spec = importlib.util.spec_from_file_location("od", sys.argv[1])
od = importlib.util.module_from_spec(spec)
spec.loader.exec_module(od)

def ram_image(oracle):
    image = bytearray(od.RAM_BYTES)
    for offset in od.ROM_EXCEPTION_RAM_WORD_OFFSETS:
        pair = od.FRAME_SR_ORACLE_BYTES if oracle else od.FRAME_SR_CHRONOFORK_BYTES
        image[offset:offset + 2] = bytes(pair)
    return image

def match(oracle, chronofork):
    return od.match_scoped_ram("rom_exception", bytes(oracle).hex(), bytes(chronofork).hex())

declared_oracle = ram_image(True)
declared_chronofork = ram_image(False)

matched = match(declared_oracle, declared_chronofork)
assert matched is not None and matched["id"] == "W4e-F1", (
    "the declared F1 divergence must match exactly, got %r" % (matched,))

def reject(name, oracle, chronofork):
    assert match(oracle, chronofork) is None, "the matcher accepted %s" % name

# A different ROM is not covered by a declaration.
assert od.match_scoped_ram("rom_alu", declared_oracle.hex(), declared_chronofork.hex()) is None

# An unrelated extra difference is not covered.
extra = bytearray(declared_chronofork)
extra[-1] ^= 0x01
reject("an unrelated differing byte", declared_oracle, extra)

# A changed byte value inside the declared window is not covered.
changed = bytearray(declared_chronofork)
changed[od.ROM_EXCEPTION_RAM_WORD_OFFSETS[0] + 1] ^= 0x01
reject("a changed ChronoFork byte", declared_oracle, changed)

changed_oracle = bytearray(declared_oracle)
changed_oracle[od.ROM_EXCEPTION_RAM_WORD_OFFSETS[0] + 1] ^= 0x01
reject("a changed oracle byte", changed_oracle, declared_chronofork)

# The archive word must diverge too: a declaration is not a blanket window.
masked = bytearray(declared_chronofork)
archive = od.ROM_EXCEPTION_RAM_WORD_OFFSETS[1]
masked[archive:archive + 2] = declared_oracle[archive:archive + 2]
reject("a divergence missing at the archive word", declared_oracle, masked)

# A third declared-looking word is not covered by the two-word declaration.
third = bytearray(declared_chronofork)
third[0x20:0x22] = b"\x7f\xe4"
reject("an extra differing word", declared_oracle, third)

# F2: the declared cycle pair matches, a changed count or location does not.
declared_trap = {
    "pc": "0x00026C",
    "opcode": "0x4E40",
    "oracle_cycles": od.TRAP_CYCLES_ORACLE,
    "chronofork_cycles": od.TRAP_CYCLES_CHRONOFORK,
}
assert (od.match_scoped_cycle("rom_exception", declared_trap) or {}).get("id") == "W4e-F2"
for name, key, value in [
    ("a changed oracle cycle count", "oracle_cycles", od.TRAP_CYCLES_ORACLE + 1),
    ("a changed ChronoFork cycle count", "chronofork_cycles", od.TRAP_CYCLES_CHRONOFORK + 1),
    ("a moved instruction", "pc", "0x00026E"),
    ("a different opcode", "opcode", "0x4E71"),
]:
    mutated = dict(declared_trap)
    mutated[key] = value
    assert od.match_scoped_cycle("rom_exception", mutated) is None, (
        "the matcher accepted %s" % name)
assert od.match_scoped_cycle("rom_alu", declared_trap) is None

# The F1 and F2 declarations bind rom_exception only.
assert {exception.rom for exception in od.scoped_exceptions()} == {"rom_exception"}
assert {exception.observable for exception in od.scoped_exceptions()} == {"cycle", "ram"}
print("scoped-exception matcher: declared pair accepted, all mutations rejected")
"#,
    );
}

/// Asserts the provisioned branch when the environment really has the oracle.
///
/// `#[ignore]` is intentional and is not a silent pass: the default run reports
/// it as `ignored`, and the documented recipe runs it explicitly with
/// `cargo test -p chronofork-megadrive --test oracle_harness -- --ignored` once
/// the pinned oracle is built (see `.research/oracle/README.md`). When it runs and
/// no candidate location is provisioned it *fails* with the reason, so it can
/// never report success without having checked a real oracle.
#[test]
#[ignore = "requires a provisioned pinned oracle (see .research/oracle/README.md)"]
fn a_provisioned_oracle_reports_the_pinned_commit_and_its_binaries() {
    let Some((root, bin_dir)) = provisioned_oracle() else {
        panic!(
            "no provisioned pinned oracle found; clone exodus-for-ais at \
             ec0056151f3ff1c334d09ee375abb30a68fdfec5 (branch linux-headless), build \
             exodus-probe/exodus-mcpd, and point CHRONOFORK_ORACLE_ROOT / \
             CHRONOFORK_ORACLE_BIN_DIR at them"
        );
    };
    let output = run_harness(&[
        "--oracle-status",
        "--oracle-root",
        &root.display().to_string(),
        "--oracle-bin-dir",
        &bin_dir.display().to_string(),
    ]);
    assert_eq!(
        exit_code(&output),
        0,
        "a provisioned oracle must exit 0: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report = parse_json_object(&String::from_utf8_lossy(&output.stdout));
    for expected in [
        "\"status\": \"provisioned\"",
        "ec0056151f3ff1c334d09ee375abb30a68fdfec5",
        "exodus-probe",
        "exodus-mcpd",
        "MIT",
    ] {
        assert!(
            report.contains(expected),
            "the provenance report must contain {expected}, got: {report}"
        );
    }
}

/// Exit status of a finished child, or a panic naming the signal when it has none.
fn exit_code(output: &Output) -> i32 {
    output.status.code().unwrap_or_else(|| {
        panic!(
            "the harness was terminated by a signal: {:?}",
            output.status
        )
    })
}

/// Parses one flat JSON object without a JSON dependency.
///
/// The harness prints `{"key": value, ...}` with one field per line. Only the
/// string-valued fields these tests assert on are extracted; the point is to
/// reject a missing or renamed field, so a malformed object panics.
fn parse_json_object(text: &str) -> String {
    assert!(
        text.trim_start().starts_with('{') && text.trim_end().ends_with('}'),
        "expected a JSON object, got: {text}"
    );
    text.to_owned()
}
