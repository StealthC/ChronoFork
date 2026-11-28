//! Host-builtin acceptance tests: session inspection, forks/checkpoints,
//! trace capture/export/compare, TAS verification/comparison, print/report/text
//! builtins and deterministic passed/failed reports.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_backend_api::{
    BACKEND_SNAPSHOT_FORMAT_VERSION, Backend, BackendDescriptor, BackendError, BackendFactory,
    BackendId, BackendSnapshot, Capabilities, MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};
use chronofork_script::{
    ReportValue, ScriptLimits, ScriptOptions, ScriptRun, ScriptStatus, run_source,
};
use chronofork_sessions::{Movie, MovieMeta, MovieStep, SessionManager};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_trace::{Trace, TraceConfig};

fn options() -> ScriptOptions {
    ScriptOptions::new(Arc::new(SyntheticBackendFactory::default()))
}

fn execute(source: &str) -> ScriptRun {
    run_source(source, options())
}

fn require_failure(run: &ScriptRun) -> Result<&chronofork_script::ScriptFailure, String> {
    run.failure()
        .ok_or_else(|| format!("expected a failed run, got status {}", run.status()))
}

fn assert_failure(run: &ScriptRun, code: &str, line: Option<usize>) -> Result<(), String> {
    assert_eq!(run.status(), ScriptStatus::Failed);
    let failure = require_failure(run)?;
    assert_eq!(failure.code(), code, "failure: {failure:?}");
    assert_eq!(failure.line(), line, "failure: {failure:?}");
    Ok(())
}

fn report_text(run: &ScriptRun) -> String {
    let mut text = String::new();
    for (key, value) in run.report_fields() {
        text.push_str(&key);
        text.push('=');
        text.push_str(&value);
        text.push('\n');
    }
    text
}

fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::new();
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn sandbox_options(root: &Path) -> Result<ScriptOptions, String> {
    options()
        .with_sandbox(root.to_path_buf())
        .map_err(|error| format!("sandbox root: {error}"))
}

fn run_in(root: &Path, source: &str) -> Result<ScriptRun, String> {
    Ok(run_source(source, sandbox_options(root)?))
}

fn expected_trace(steps: u64, capacity: usize) -> Result<Trace, String> {
    let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
    let session = manager
        .create_session()
        .map_err(|error| format!("create session: {error}"))?;
    let config = TraceConfig::new(capacity).map_err(|error| format!("trace config: {error}"))?;
    manager
        .capture_trace(session, &config, steps)
        .map_err(|error| format!("capture trace: {error}"))
}

fn movie_bytes(inputs: &[InputFrame]) -> Result<Vec<u8>, String> {
    let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
    let session = manager
        .create_session()
        .map_err(|error| format!("create session: {error}"))?;
    let meta = MovieMeta::new(1, None, None).map_err(|error| format!("movie meta: {error}"))?;
    let movie = Movie::record(&mut manager, session, meta, inputs)
        .map_err(|error| format!("record movie: {error}"))?;
    movie
        .to_bytes()
        .map_err(|error| format!("encode movie: {error}"))
}

fn corrupted_movie_bytes() -> Result<Vec<u8>, String> {
    let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
    let session = manager
        .create_session()
        .map_err(|error| format!("create session: {error}"))?;
    let snapshot = manager
        .snapshot(session)
        .map_err(|error| format!("snapshot: {error}"))?;
    let snapshot_bytes = manager
        .snapshot_bytes(session, snapshot)
        .map_err(|error| format!("snapshot bytes: {error}"))?;
    let meta = MovieMeta::new(1, None, None).map_err(|error| format!("movie meta: {error}"))?;
    let steps = (1..=3_u64)
        .map(|tick| {
            MovieStep::new(
                Tick::from_raw(tick),
                InputFrame::NEUTRAL,
                StateHash::from_bytes([0; 32]),
            )
        })
        .collect();
    let movie = Movie::from_parts(meta, snapshot_bytes, steps)
        .map_err(|error| format!("build corrupted movie: {error}"))?;
    movie
        .to_bytes()
        .map_err(|error| format!("encode corrupted movie: {error}"))
}

fn movie_bytes_after_steps(pre_steps: usize, inputs: &[InputFrame]) -> Result<Vec<u8>, String> {
    let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
    let session = manager
        .create_session()
        .map_err(|error| format!("create session: {error}"))?;
    for _ in 0..pre_steps {
        manager
            .step(session, InputFrame::NEUTRAL)
            .map_err(|error| format!("step: {error}"))?;
    }
    let meta = MovieMeta::new(1, None, None).map_err(|error| format!("movie meta: {error}"))?;
    let movie = Movie::record(&mut manager, session, meta, inputs)
        .map_err(|error| format!("record movie: {error}"))?;
    movie
        .to_bytes()
        .map_err(|error| format!("encode movie: {error}"))
}

/// A test-only backend with a fixed read-only memory window.
///
/// It exists solely to exercise the `read` builtin with a 4096-byte window; it
/// cannot execute instructions and is never used by production code.
#[derive(Debug)]
struct HexMemoryBackend {
    descriptor: BackendDescriptor,
    memory: Vec<u8>,
}

impl Backend for HexMemoryBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        // The fixed memory image is this fixture's whole canonical state.
        Ok(hash_bytes(&self.memory))
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        Err(BackendError::Execution(
            "hex-memory test backend does not step".to_owned(),
        ))
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        Ok(BackendSnapshot::new(
            self.descriptor.id().clone(),
            self.descriptor.version(),
            BACKEND_SNAPSHOT_FORMAT_VERSION,
            Tick::ZERO,
            hash_bytes(b"hex-memory-test-backend"),
            hash_bytes(&self.memory),
            self.memory.clone(),
        ))
    }

    fn restore(&mut self, _snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        Ok(())
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        let mut registers = std::collections::BTreeMap::new();
        registers.insert("pc".to_owned(), 0);
        registers.insert("tick".to_owned(), 0);
        registers.insert("halted".to_owned(), 0);
        Ok(MachineInspection {
            tick: Tick::ZERO,
            halted: false,
            frame_index: 0,
            cpu_registers: registers,
            device_registers: std::collections::BTreeMap::new(),
            memory: None,
        })
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        let limit = self.memory.len() as u64;
        let start = usize::try_from(address).map_err(|_| BackendError::OutOfBounds {
            address,
            length,
            limit,
        })?;
        let end = start.checked_add(length).ok_or(BackendError::OutOfBounds {
            address,
            length,
            limit,
        })?;
        if end > self.memory.len() {
            return Err(BackendError::OutOfBounds {
                address,
                length,
                limit,
            });
        }
        Ok(self.memory[start..end].to_vec())
    }

    fn write_memory(&mut self, _address: u64, _data: &[u8]) -> Result<(), BackendError> {
        Err(BackendError::Execution(
            "hex-memory test backend is read-only".to_owned(),
        ))
    }
}

/// Factory for the fixed-window test backend above.
struct HexMemoryFactory {
    descriptor: BackendDescriptor,
    memory: Arc<Vec<u8>>,
}

impl HexMemoryFactory {
    fn new(bytes: usize) -> Self {
        let capabilities = Capabilities {
            save_states: true,
            memory_read: true,
            register_inspection: true,
            deterministic_replay: true,
            ..Capabilities::none()
        };
        let descriptor = BackendDescriptor::new(
            BackendId::new("chronofork.test.hexmemory"),
            "ChronoFork hex-memory test backend",
            "1",
            capabilities,
            true,
        );
        let memory = (0..bytes).map(|index| (index % 251) as u8).collect();
        Self {
            descriptor,
            memory: Arc::new(memory),
        }
    }
}

impl BackendFactory for HexMemoryFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(HexMemoryBackend {
            descriptor: self.descriptor.clone(),
            memory: (*self.memory).clone(),
        }))
    }
}

struct TempSandbox {
    root: PathBuf,
}

impl TempSandbox {
    fn new(name: &str) -> Result<Self, String> {
        let base = std::env::temp_dir();
        let suffix = std::process::id();
        let root = base.join(format!("chronofork-t5-{name}-{suffix}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).map_err(|error| format!("create sandbox: {error}"))?;
        Ok(Self { root })
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), String> {
        fs::write(self.root.join(relative), bytes)
            .map_err(|error| format!("write fixture {relative}: {error}"))
    }
}

impl Drop for TempSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn session_inspection_builtins_match_the_session() -> Result<(), String> {
    let factory = Arc::new(SyntheticBackendFactory::default());
    let script = "let session = open()\n\
                  step(3)\n\
                  print session\n\
                  print tick()\n\
                  print state_hash()\n\
                  print register(\"pc\")\n\
                  print read(0, 4)\n";
    let run = run_source(script, ScriptOptions::new(Arc::clone(&factory) as Arc<_>));
    assert_eq!(run.status(), ScriptStatus::Passed);
    assert_eq!(run.stats().steps(), 3);

    let mut manager = SessionManager::new(factory);
    let session = manager
        .create_session()
        .map_err(|error| format!("create session: {error}"))?;
    for _ in 0..3 {
        manager
            .step(session, InputFrame::NEUTRAL)
            .map_err(|error| format!("step: {error}"))?;
    }
    let inspection = manager
        .inspect(session)
        .map_err(|error| format!("inspect: {error}"))?;
    let memory = manager
        .read_memory(session, 0, 4)
        .map_err(|error| format!("read memory: {error}"))?;
    let registers = inspection.machine.registers();
    let pc = registers
        .get("pc")
        .ok_or_else(|| "synthetic backend exposes pc".to_owned())?;

    let expected = vec![
        session.raw().to_string(),
        inspection.machine.tick.raw().to_string(),
        inspection.state_hash.to_hex(),
        pc.to_string(),
        lowercase_hex(&memory),
    ];
    assert_eq!(run.prints(), expected);

    // The state hash contract is 64 lowercase hexadecimal characters.
    let hash = &run.prints()[2];
    assert_eq!(hash.len(), 64);
    assert!(
        hash.bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );

    assert_failure(&execute("open()\nopen()\n"), "already_open", Some(2))?;
    assert_failure(&execute("print tick()\n"), "not_open", Some(1))?;
    assert_failure(
        &execute("open()\nprint step(-1)\n"),
        "invalid_step_count",
        Some(2),
    )?;
    assert_failure(
        &execute("open()\nprint register(\"nope\")\n"),
        "unknown_register",
        Some(2),
    )?;
    assert_failure(
        &execute("open()\nprint read(0, 0)\n"),
        "invalid_length",
        Some(2),
    )?;
    assert_failure(
        &execute("open()\nprint read(0, 4097)\n"),
        "invalid_length",
        Some(2),
    )?;
    assert_failure(
        &execute("open()\nprint read(300, 1)\n"),
        "memory_out_of_bounds",
        Some(2),
    )?;

    let zero_step = execute("open()\nprint step(0)\n");
    assert_eq!(zero_step.status(), ScriptStatus::Passed);
    assert_eq!(zero_step.prints(), ["0"]);

    let limited = run_source(
        "open()\nstep(3)\n",
        options().with_limits(ScriptLimits {
            steps: 2,
            ..ScriptLimits::default()
        }),
    );
    assert_failure(&limited, "limit.steps", Some(2))?;
    assert_eq!(limited.stats().steps(), 0);

    // reset() returns 0 and restores the session's creation state.
    let reset = execute("open()\nstep(3)\nprint reset()\nprint tick()\nprint state_hash()\n");
    assert_eq!(reset.status(), ScriptStatus::Passed);
    let mut fresh = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
    let fresh_session = fresh
        .create_session()
        .map_err(|error| format!("fresh session: {error}"))?;
    let fresh_view = fresh
        .inspect(fresh_session)
        .map_err(|error| format!("fresh inspection: {error}"))?;
    assert_eq!(
        reset.prints(),
        vec![
            "0".to_owned(),
            "0".to_owned(),
            fresh_view.state_hash.to_hex(),
        ]
    );
    assert_failure(&execute("print reset()\n"), "not_open", Some(1))?;
    Ok(())
}

#[test]
fn report_print_and_text_builtins() -> Result<(), String> {
    let run = execute(
        "print 1\n\
         print true\n\
         print \"txt\"\n\
         print str(2 + 3)\n\
         print len(\"héllo\")\n\
         print str(0 - 5)\n\
         print str(false)\n",
    );
    assert_eq!(run.status(), ScriptStatus::Passed);
    assert_eq!(run.prints(), ["1", "true", "txt", "5", "6", "-5", "false"]);

    let reported = execute(
        "report(\"i\", 1)\n\
         report(\"b\", true)\n\
         report(\"t\", \"hi\")\n\
         report(\"zero\", 0)\n",
    );
    assert_eq!(reported.status(), ScriptStatus::Passed);
    assert_eq!(reported.stats().report_bytes(), 15);
    assert_eq!(
        reported.report().get("i").map(ReportValue::render),
        Some("1".to_owned())
    );
    assert_eq!(
        reported.report().get("b").map(ReportValue::render),
        Some("true".to_owned())
    );
    assert_eq!(
        reported.report().get("t").map(ReportValue::render),
        Some("hi".to_owned())
    );
    let fields = reported.report_fields();
    let keys: Vec<&str> = fields
        .iter()
        .filter_map(|(key, _)| key.strip_prefix("report."))
        .collect();
    assert_eq!(keys, ["b", "i", "t", "zero"]);

    assert_failure(
        &execute("report(\"k\", 1)\nreport(\"k\", 2)\n"),
        "report_key_duplicate",
        Some(2),
    )?;
    assert_failure(&execute("report(\"\", 1)\n"), "report_key_invalid", Some(1))?;
    assert_failure(
        &execute("report(\"1bad\", 1)\n"),
        "report_key_invalid",
        Some(1),
    )?;
    assert_failure(
        &execute("report(\"a-b\", 1)\n"),
        "report_key_invalid",
        Some(1),
    )?;
    let oversized_key = "a".repeat(65);
    let oversized = execute(&format!("report(\"{oversized_key}\", 1)\n"));
    assert_failure(&oversized, "report_key_invalid", Some(1))?;
    assert_failure(&execute("report(1, 2)\n"), "type", Some(1))?;

    let failed = execute("print 1\nfail(\"stop here\")\n");
    assert_failure(&failed, "fail", Some(2))?;
    assert_eq!(failed.prints(), ["1"]);
    assert_eq!(require_failure(&failed)?.message(), "stop here");

    let asserted = execute("assert 1 == 2\n");
    assert_failure(&asserted, "assertion_failed", Some(1))?;
    assert_eq!(asserted.stats().assertions(), 1);
    Ok(())
}

#[test]
fn script_report_is_passed_failed_and_deterministic() -> Result<(), String> {
    let source = "let x = 2\n\
                  assert x == 2\n\
                  print x\n\
                  report(\"x\", x)\n";
    let first = execute(source);
    let second = execute(source);
    assert_eq!(first.status(), ScriptStatus::Passed);
    assert_eq!(first.failure(), None);
    assert_eq!(first.prints(), ["2"]);
    assert_eq!(first.stats().statements(), 4);
    assert_eq!(first.stats().assertions(), 1);
    assert_eq!(first.stats().variables(), 1);
    assert_eq!(first.stats().report_bytes(), 2);
    assert_eq!(first.backend(), "chronofork.test.synthetic");
    assert_eq!(
        first.source_sha256(),
        hash_bytes(source.as_bytes()).to_hex()
    );
    assert_eq!(first.source_sha256().len(), 64);
    assert_eq!(report_text(&first), report_text(&second));
    assert_eq!(first.prints(), second.prints());

    let fields = first.report_fields();
    assert_eq!(
        fields
            .first()
            .map(|(key, value)| (key.as_str(), value.as_str())),
        Some(("status", "passed"))
    );
    assert_eq!(
        fields
            .get(1)
            .map(|(key, value)| (key.as_str(), value.as_str())),
        Some(("failure", "-"))
    );
    assert!(
        fields
            .iter()
            .any(|(key, value)| key == "report.x" && value == "2")
    );
    for (key, value) in &fields {
        assert!(!key.contains("path"), "field {key}");
        assert!(!key.contains("time"), "field {key}");
        assert!(!key.contains("pid"), "field {key}");
        assert!(!value.contains('/'), "field {key} value {value}");
    }

    let wrong_assertion = "let x = 2\nassert x == 3\nprint x\n";
    let first_failure = execute(wrong_assertion);
    let second_failure = execute(wrong_assertion);
    assert_failure(&first_failure, "assertion_failed", Some(2))?;
    assert_eq!(first_failure.prints().len(), 0);
    assert_eq!(report_text(&first_failure), report_text(&second_failure));
    let failure = require_failure(&first_failure)?;
    assert_eq!(failure.message(), "assertion failed");
    let fields = first_failure.report_fields();
    assert_eq!(
        fields
            .get(1)
            .map(|(key, value)| (key.as_str(), value.as_str())),
        Some(("failure", "assertion_failed"))
    );
    assert_eq!(
        fields
            .get(2)
            .map(|(key, value)| (key.as_str(), value.as_str())),
        Some(("message", "assertion failed"))
    );

    let explicit = "fail(\"deliberate\")\n";
    let first_failure = execute(explicit);
    let second_failure = execute(explicit);
    assert_failure(&first_failure, "fail", Some(1))?;
    assert_eq!(require_failure(&first_failure)?.message(), "deliberate");
    assert_eq!(report_text(&first_failure), report_text(&second_failure));
    Ok(())
}

#[test]
fn fork_checkpoint_restore_and_select_preserve_isolation() -> Result<(), String> {
    let script = "let root = open()\n\
                  step(3)\n\
                  let child = fork()\n\
                  print root\n\
                  print child\n\
                  print tick()\n\
                  print state_hash()\n\
                  step(2)\n\
                  let snap = checkpoint()\n\
                  print snap\n\
                  step(1)\n\
                  print restore(snap)\n\
                  print tick()\n\
                  print state_hash()\n\
                  print select(root)\n\
                  print tick()\n\
                  print state_hash()\n\
                  step(1)\n\
                  print tick()\n\
                  print state_hash()\n\
                  print select(child)\n\
                  print tick()\n\
                  print state_hash()\n";
    let first = execute(script);
    assert_eq!(first.status(), ScriptStatus::Passed);

    let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
    let root = manager
        .create_session()
        .map_err(|error| format!("create root: {error}"))?;
    for _ in 0..3 {
        manager
            .step(root, InputFrame::NEUTRAL)
            .map_err(|error| format!("root step: {error}"))?;
    }
    let root_view = manager
        .inspect(root)
        .map_err(|error| format!("root inspection: {error}"))?;
    let child = manager
        .fork_current(root)
        .map_err(|error| format!("fork: {error}"))?;
    for _ in 0..2 {
        manager
            .step(child, InputFrame::NEUTRAL)
            .map_err(|error| format!("child step: {error}"))?;
    }
    let child_view = manager
        .inspect(child)
        .map_err(|error| format!("child inspection: {error}"))?;
    let snapshot = manager
        .snapshot(child)
        .map_err(|error| format!("child checkpoint: {error}"))?;
    manager
        .step(child, InputFrame::NEUTRAL)
        .map_err(|error| format!("child extra step: {error}"))?;
    manager
        .restore(child, snapshot)
        .map_err(|error| format!("child restore: {error}"))?;
    let restored = manager
        .inspect(child)
        .map_err(|error| format!("restored inspection: {error}"))?;
    manager
        .step(root, InputFrame::NEUTRAL)
        .map_err(|error| format!("root extra step: {error}"))?;
    let root_after = manager
        .inspect(root)
        .map_err(|error| format!("root after inspection: {error}"))?;

    let expected = vec![
        root.raw().to_string(),
        child.raw().to_string(),
        root_view.machine.tick.raw().to_string(),
        root_view.state_hash.to_hex(),
        snapshot.raw().to_string(),
        "0".to_owned(),
        child_view.machine.tick.raw().to_string(),
        child_view.state_hash.to_hex(),
        "0".to_owned(),
        root_view.machine.tick.raw().to_string(),
        root_view.state_hash.to_hex(),
        root_after.machine.tick.raw().to_string(),
        root_after.state_hash.to_hex(),
        "0".to_owned(),
        restored.machine.tick.raw().to_string(),
        restored.state_hash.to_hex(),
    ];
    assert_eq!(first.prints(), expected);
    assert_eq!(
        child_view.state_hash, restored.state_hash,
        "restore must reproduce the recorded child state"
    );
    assert_ne!(
        root_after.state_hash, child_view.state_hash,
        "source and child must be independent"
    );

    let second = execute(script);
    assert_eq!(report_text(&first), report_text(&second));
    assert_eq!(first.prints(), second.prints());

    assert_failure(
        &execute("open()\nselect(999)\n"),
        "unknown_session",
        Some(2),
    )?;
    assert_failure(
        &execute("open()\nrestore(999)\n"),
        "unknown_snapshot",
        Some(2),
    )?;
    assert_failure(&execute("open()\nselect(-1)\n"), "invalid_handle", Some(2))?;
    assert_failure(&execute("open()\nrestore(-1)\n"), "invalid_handle", Some(2))?;
    assert_failure(&execute("fork()\n"), "not_open", Some(1))?;
    assert_failure(&execute("checkpoint()\n"), "not_open", Some(1))?;
    assert_failure(&execute("select(1)\n"), "not_open", Some(1))?;
    assert_failure(&execute("restore(1)\n"), "not_open", Some(1))?;
    assert_failure(&execute("fork(1)\n"), "arity", Some(1))?;
    assert_failure(&execute("open()\nrestore(\"x\")\n"), "type", Some(2))?;
    assert_failure(&execute("open()\ncheckpoint(1)\n"), "arity", Some(2))?;

    let snapshots = limits_with(|limits| limits.snapshots = 1);
    let limited = run_source(
        "open()\ncheckpoint()\ncheckpoint()\n",
        options().with_limits(snapshots),
    );
    assert_failure(&limited, "limit.snapshots", Some(3))?;
    let forked = run_source(
        "open()\nfork()\nfork()\n",
        options().with_limits(limits_with(|limits| limits.snapshots = 1)),
    );
    assert_failure(&forked, "limit.snapshots", Some(3))?;
    Ok(())
}

fn limits_with(change: impl FnOnce(&mut ScriptLimits)) -> ScriptLimits {
    let mut limits = ScriptLimits::default();
    change(&mut limits);
    limits
}

#[test]
fn trace_builtins_capture_export_and_compare() -> Result<(), String> {
    let sandbox = TempSandbox::new("trace")?;
    let expected = expected_trace(4, 16)?;
    let expected_bytes = expected.to_bytes();
    let script = "open()\n\
                  let records = trace_capture(4, 16)\n\
                  print records\n\
                  print trace_records()\n\
                  print trace_export(\"run.cftrace\")\n\
                  print trace_compare(\"run.cftrace\")\n";
    let run = run_in(sandbox.root(), script)?;
    assert_eq!(run.status(), ScriptStatus::Passed);
    assert_eq!(
        run.prints(),
        vec![
            "4".to_owned(),
            "4".to_owned(),
            expected_bytes.len().to_string(),
            "-1".to_owned(),
        ]
    );
    assert_eq!(run.stats().steps(), 4);
    assert_eq!(run.stats().trace_records(), 4);
    let exported = fs::read(sandbox.root().join("run.cftrace"))
        .map_err(|error| format!("read export: {error}"))?;
    assert_eq!(exported, expected_bytes);

    let short = expected_trace(3, 16)?;
    sandbox.write("short.cftrace", &short.to_bytes())?;
    let changed = run_in(
        sandbox.root(),
        "open()\ntrace_capture(4, 16)\nprint trace_compare(\"short.cftrace\")\n",
    )?;
    assert_eq!(changed.status(), ScriptStatus::Passed);
    assert_eq!(changed.prints(), ["3"]);

    let empty = execute("print trace_records()\n");
    assert_eq!(empty.status(), ScriptStatus::Passed);
    assert_eq!(empty.prints(), ["0"]);

    let no_trace = run_in(sandbox.root(), "open()\ntrace_export(\"run.cftrace\")\n")?;
    assert_failure(&no_trace, "no_stored_trace", Some(2))?;

    let no_sandbox = execute("open()\ntrace_capture(1, 4)\ntrace_export(\"x\")\n");
    assert_failure(&no_sandbox, "sandbox_unavailable", Some(3))?;
    let no_sandbox_compare = execute("open()\ntrace_capture(1, 4)\ntrace_compare(\"x\")\n");
    assert_failure(&no_sandbox_compare, "sandbox_unavailable", Some(3))?;

    assert_failure(
        &run_in(sandbox.root(), "open()\ntrace_capture(0, 16)\n")?,
        "invalid_trace_steps",
        Some(2),
    )?;
    assert_failure(
        &run_in(sandbox.root(), "open()\ntrace_capture(1000001, 16)\n")?,
        "invalid_trace_steps",
        Some(2),
    )?;
    assert_failure(
        &run_in(sandbox.root(), "open()\ntrace_capture(1, 0)\n")?,
        "invalid_trace_capacity",
        Some(2),
    )?;
    assert_failure(
        &run_in(sandbox.root(), "open()\ntrace_capture(1, 1000001)\n")?,
        "invalid_trace_capacity",
        Some(2),
    )?;

    assert_failure(
        &run_in(sandbox.root(), "open()\ntrace_capture(1, \"x\")\n")?,
        "type",
        Some(2),
    )?;
    assert_failure(
        &run_in(sandbox.root(), "open()\ntrace_capture(1)\n")?,
        "arity",
        Some(2),
    )?;
    assert_failure(
        &run_in(sandbox.root(), "trace_export(1)\n")?,
        "type",
        Some(1),
    )?;
    assert_failure(
        &run_in(sandbox.root(), "trace_compare(1)\n")?,
        "type",
        Some(1),
    )?;

    let records_limited = run_source(
        "open()\ntrace_capture(4, 8)\n",
        options()
            .with_limits(limits_with(|limits| limits.trace_records = 3))
            .with_sandbox(sandbox.root().to_path_buf())
            .map_err(|error| format!("sandbox: {error}"))?,
    );
    assert_failure(&records_limited, "limit.trace_records", Some(2))?;
    assert_eq!(records_limited.stats().steps(), 0);

    let steps_limited = run_source(
        "open()\ntrace_capture(4, 8)\n",
        options()
            .with_limits(limits_with(|limits| limits.steps = 3))
            .with_sandbox(sandbox.root().to_path_buf())
            .map_err(|error| format!("sandbox: {error}"))?,
    );
    assert_failure(&steps_limited, "limit.steps", Some(2))?;
    assert_eq!(steps_limited.stats().steps(), 0);

    let memory_limited = run_source(
        "open()\ntrace_capture(4, 16)\n",
        options()
            .with_limits(limits_with(|limits| limits.string_bytes = 100))
            .with_sandbox(sandbox.root().to_path_buf())
            .map_err(|error| format!("sandbox: {error}"))?,
    );
    assert_failure(&memory_limited, "limit.string_bytes", Some(2))?;
    assert_eq!(memory_limited.stats().steps(), 0);

    sandbox.write("bad.cftrace", b"not a trace")?;
    let malformed = run_in(
        sandbox.root(),
        "open()\ntrace_capture(1, 4)\ntrace_compare(\"bad.cftrace\")\n",
    )?;
    assert_failure(&malformed, "trace_parse", Some(3))?;

    let missing = run_in(
        sandbox.root(),
        "open()\ntrace_capture(1, 4)\ntrace_compare(\"missing.cftrace\")\n",
    )?;
    assert_failure(&missing, "sandbox_io", Some(3))?;

    let other = TempSandbox::new("trace-repeat")?;
    let repeated = run_in(other.root(), script)?;
    assert_eq!(report_text(&run), report_text(&repeated));
    assert_eq!(run.prints(), repeated.prints());
    Ok(())
}

#[test]
fn tas_builtins_verify_and_compare_movies() -> Result<(), String> {
    let sandbox = TempSandbox::new("tas")?;
    let neutral = [InputFrame::NEUTRAL; 3];
    let good = movie_bytes(&neutral)?;
    let divergent = movie_bytes(&[InputFrame::new(1); 3])?;
    let corrupted = corrupted_movie_bytes()?;
    let long = movie_bytes(&[InputFrame::NEUTRAL; 4])?;
    let checkpoint_mismatch = movie_bytes_after_steps(1, &[InputFrame::NEUTRAL; 2])?;
    sandbox.write("good.cfmovie", &good)?;
    sandbox.write("divergent.cfmovie", &divergent)?;
    sandbox.write("corrupted.cfmovie", &corrupted)?;
    sandbox.write("long.cfmovie", &long)?;
    sandbox.write("checkpoint.cfmovie", &checkpoint_mismatch)?;

    let script = "print tas_verify(\"good.cfmovie\")\n\
                  print tas_verify(\"corrupted.cfmovie\")\n\
                  print tas_compare(\"good.cfmovie\", \"good.cfmovie\")\n\
                  print tas_compare(\"good.cfmovie\", \"divergent.cfmovie\")\n\
                  print tas_compare(\"good.cfmovie\", \"long.cfmovie\")\n";
    let run = run_in(sandbox.root(), script)?;
    assert_eq!(run.status(), ScriptStatus::Passed);
    assert_eq!(run.prints(), ["true", "false", "-1", "3", "4"]);
    assert_eq!(
        run.stats().steps(),
        6,
        "both tas_verify calls charge their recorded steps"
    );

    // The movie length is a conservative reservation: the corrupted movie diverges
    // on its first replay step but still charges all three recorded steps.
    let divergent_only = run_in(sandbox.root(), "print tas_verify(\"corrupted.cfmovie\")\n")?;
    assert_eq!(divergent_only.status(), ScriptStatus::Passed);
    assert_eq!(divergent_only.prints(), ["false"]);
    assert_eq!(
        divergent_only.stats().steps(),
        3,
        "the whole movie length is reserved conservatively"
    );

    // Replay and comparison are read-only: the fixture bytes never change.
    assert_eq!(
        fs::read(sandbox.root().join("good.cfmovie"))
            .map_err(|error| format!("read good: {error}"))?,
        good
    );
    assert_eq!(
        fs::read(sandbox.root().join("divergent.cfmovie"))
            .map_err(|error| format!("read divergent: {error}"))?,
        divergent
    );

    let isolated = run_in(
        sandbox.root(),
        "open()\nstep(2)\nprint tick()\n\
         print tas_verify(\"good.cfmovie\")\n\
         print tick()\nprint state_hash()\nprint state_hash()\n",
    )?;
    assert_eq!(isolated.status(), ScriptStatus::Passed);
    let prints = isolated.prints();
    assert_eq!(
        prints[0], prints[2],
        "verify must not advance the script session"
    );
    assert_eq!(
        prints[3], prints[4],
        "verify must not change the session hash"
    );

    let checkpoint = run_in(
        sandbox.root(),
        "print tas_compare(\"good.cfmovie\", \"checkpoint.cfmovie\")\n",
    )?;
    assert_failure(&checkpoint, "movie_checkpoint_mismatch", Some(1))?;

    sandbox.write("bad.cfmovie", b"garbage")?;
    sandbox.write("truncated.cfmovie", &good[..10])?;
    let mut version = good.clone();
    version[8] = 9;
    version[9] = 0;
    let body_len = version.len() - 32;
    let checksum = hash_bytes(&version[..body_len]);
    version[body_len..].copy_from_slice(checksum.as_bytes());
    sandbox.write("version.cfmovie", &version)?;
    for fixture in ["bad.cfmovie", "truncated.cfmovie", "version.cfmovie"] {
        let run = run_in(
            sandbox.root(),
            &format!("print tas_verify(\"{fixture}\")\n"),
        )?;
        assert_failure(&run, "movie_decode", Some(1))?;
        let compared = run_in(
            sandbox.root(),
            &format!("print tas_compare(\"good.cfmovie\", \"{fixture}\")\n"),
        )?;
        assert_failure(&compared, "movie_decode", Some(1))?;
    }

    let missing = run_in(sandbox.root(), "print tas_verify(\"missing.cfmovie\")\n")?;
    assert_failure(&missing, "sandbox_io", Some(1))?;

    let no_sandbox = execute("print tas_verify(\"good.cfmovie\")\n");
    assert_failure(&no_sandbox, "sandbox_unavailable", Some(1))?;
    let no_sandbox_compare = execute("print tas_compare(\"a\", \"b\")\n");
    assert_failure(&no_sandbox_compare, "sandbox_unavailable", Some(1))?;

    assert_failure(&run_in(sandbox.root(), "tas_verify(1)\n")?, "type", Some(1))?;
    assert_failure(
        &run_in(sandbox.root(), "tas_compare(1)\n")?,
        "arity",
        Some(1),
    )?;
    assert_failure(
        &run_in(sandbox.root(), "tas_compare(\"good.cfmovie\", 1)\n")?,
        "type",
        Some(1),
    )?;

    let limited = run_source(
        "print tas_verify(\"good.cfmovie\")\n",
        options()
            .with_limits(limits_with(|limits| limits.steps = 2))
            .with_sandbox(sandbox.root().to_path_buf())
            .map_err(|error| format!("sandbox: {error}"))?,
    );
    assert_failure(&limited, "limit.steps", Some(1))?;
    assert_eq!(limited.stats().steps(), 0);

    let other = TempSandbox::new("tas-repeat")?;
    other.write("good.cfmovie", &good)?;
    other.write("divergent.cfmovie", &divergent)?;
    other.write("corrupted.cfmovie", &corrupted)?;
    other.write("long.cfmovie", &long)?;
    let repeated = run_in(other.root(), script)?;
    assert_eq!(report_text(&run), report_text(&repeated));
    assert_eq!(run.prints(), repeated.prints());
    Ok(())
}

#[test]
fn read_returns_8192_hex_characters_within_the_documented_text_rule() -> Result<(), String> {
    // ADR-0011 §3.5: authored/concatenated text is bounded to 4 KiB per value,
    // while read(0, 4096) returns 8192 lowercase hex characters that are charged
    // to string_bytes when they become live.
    let factory: Arc<dyn BackendFactory> = Arc::new(HexMemoryFactory::new(4096));
    let source = "open()\n\
                  let raw = read(0, 4096)\n\
                  assert len(raw) == 8192\n\
                  report(\"hex_len\", len(raw))\n";
    let run = run_source(source, ScriptOptions::new(Arc::clone(&factory)));
    assert_eq!(run.status(), ScriptStatus::Passed);
    assert_eq!(
        run.report().get("hex_len").map(ReportValue::render),
        Some("8192".to_owned())
    );

    let limits = ScriptLimits {
        string_bytes: 8191,
        ..ScriptLimits::default()
    };
    let limited = run_source(source, ScriptOptions::new(factory).with_limits(limits));
    assert_failure(&limited, "limit.string_bytes", Some(2))?;

    // A 4097-byte string literal remains a typed parse rejection.
    let literal = format!("let text = \"{}\"\n", "a".repeat(4097));
    let literal_run = run_source(&literal, options());
    assert_failure(&literal_run, "parse.string_too_long", Some(1))?;
    Ok(())
}
