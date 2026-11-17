//! Sandbox acceptance tests: path escapes, bounded reads, typed rejection and
//! the no-sandbox contract.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_script::{ScriptOptions, ScriptRun, ScriptStatus, run_source};
use chronofork_sessions::SessionManager;
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_trace::{MAX_TRACE_BYTES, Trace, TraceConfig};

fn options() -> ScriptOptions {
    ScriptOptions::new(Arc::new(SyntheticBackendFactory::default()))
}

fn sandbox_options(root: &Path) -> Result<ScriptOptions, String> {
    options()
        .with_sandbox(root.to_path_buf())
        .map_err(|error| format!("sandbox root: {error}"))
}

fn execute(source: &str) -> ScriptRun {
    run_source(source, options())
}

fn run_in(root: &Path, source: &str) -> Result<ScriptRun, String> {
    Ok(run_source(source, sandbox_options(root)?))
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

struct TempSandbox {
    root: PathBuf,
    outside: PathBuf,
}

impl TempSandbox {
    fn new(name: &str) -> Result<Self, String> {
        let base = std::env::temp_dir();
        let suffix = std::process::id();
        let root = base.join(format!("chronofork-t5-sandbox-{name}-{suffix}"));
        let outside = base.join(format!("chronofork-t5-sandbox-{name}-{suffix}-outside"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&root).map_err(|error| format!("create sandbox: {error}"))?;
        fs::create_dir_all(&outside).map_err(|error| format!("create outside: {error}"))?;
        Ok(Self { root, outside })
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn outside(&self) -> &Path {
        &self.outside
    }

    fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), String> {
        fs::write(self.root.join(relative), bytes)
            .map_err(|error| format!("write fixture {relative}: {error}"))
    }
}

impl Drop for TempSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.outside);
    }
}

#[test]
fn sandbox_rejects_escapes_and_bounds_reads() -> Result<(), String> {
    let sandbox = TempSandbox::new("main")?;
    let trace = expected_trace(2, 8)?.to_bytes();

    // Without a sandbox every file builtin is a typed failure, while non-file
    // builtins still run normally.
    let no_sandbox = execute("open()\ntrace_capture(1, 4)\ntrace_export(\"x\")\n");
    assert_failure(&no_sandbox, "sandbox_unavailable", Some(3))?;
    let no_sandbox_compare = execute("open()\ntrace_capture(1, 4)\ntrace_compare(\"x\")\n");
    assert_failure(&no_sandbox_compare, "sandbox_unavailable", Some(3))?;
    let no_sandbox_tas = execute("print tas_verify(\"x\")\n");
    assert_failure(&no_sandbox_tas, "sandbox_unavailable", Some(1))?;
    let no_sandbox_tas_compare = execute("print tas_compare(\"a\", \"b\")\n");
    assert_failure(&no_sandbox_tas_compare, "sandbox_unavailable", Some(1))?;
    let non_file = execute("open()\nprint step(1)\nprint tick()\n");
    assert_eq!(non_file.status(), ScriptStatus::Passed);
    assert_eq!(non_file.prints(), ["1", "1"]);

    // The root must exist and be a directory.
    let missing_root = sandbox.root().join("missing");
    match options().with_sandbox(missing_root) {
        Ok(_) => return Err("a missing sandbox root must be rejected".to_owned()),
        Err(error) => assert_eq!(error.code(), "sandbox_root"),
    }
    let file_root = sandbox.root().join("file-root");
    fs::write(&file_root, b"x").map_err(|error| format!("write file root: {error}"))?;
    match options().with_sandbox(file_root) {
        Ok(_) => return Err("a file sandbox root must be rejected".to_owned()),
        Err(error) => assert_eq!(error.code(), "sandbox_root"),
    }

    // A relative path inside the root is the accepted control.
    sandbox.write("run.cftrace", &trace)?;
    let accepted = run_in(
        sandbox.root(),
        "open()\ntrace_capture(2, 8)\n\
         print trace_export(\"out.cftrace\")\n\
         print trace_compare(\"run.cftrace\")\n",
    )?;
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(
        accepted.prints(),
        vec![trace.len().to_string(), "-1".to_owned()]
    );
    assert_eq!(
        fs::read(sandbox.root().join("out.cftrace"))
            .map_err(|error| format!("read export: {error}"))?,
        trace
    );
    assert!(
        !sandbox.outside().join("out.cftrace").exists(),
        "export must not write outside the root"
    );

    // Rejected paths: empty, absolute, dot components and NUL.
    let root_text = sandbox.root().to_string_lossy().into_owned();
    let rejected_paths = [
        String::new(),
        root_text.clone(),
        ".".to_owned(),
        "..".to_owned(),
        "./x".to_owned(),
        "a/../b".to_owned(),
        "bad\0name".to_owned(),
    ];
    for path in rejected_paths {
        let source = format!("open()\ntrace_capture(1, 4)\ntrace_export(\"{path}\")\n");
        let run = run_in(sandbox.root(), &source)?;
        assert_failure(&run, "sandbox_path", Some(3))?;
        assert!(
            !run.failure()
                .is_some_and(|failure| failure.message().contains(&root_text)),
            "failure message must not echo the host path"
        );
    }

    // A directory is not a regular file.
    fs::create_dir(sandbox.root().join("dir"))
        .map_err(|error| format!("create directory: {error}"))?;
    let directory = run_in(
        sandbox.root(),
        "open()\ntrace_capture(1, 4)\ntrace_compare(\"dir\")\n",
    )?;
    assert_failure(&directory, "sandbox_path", Some(3))?;

    // A rejected write never replaces a directory or creates a partial file.
    let rejected_write = run_in(
        sandbox.root(),
        "open()\ntrace_capture(1, 4)\ntrace_export(\"dir\")\n",
    )?;
    assert_failure(&rejected_write, "sandbox_path", Some(3))?;
    assert!(
        fs::metadata(sandbox.root().join("dir"))
            .map_err(|error| format!("dir metadata: {error}"))?
            .is_dir()
    );
    let missing_parent = run_in(
        sandbox.root(),
        "open()\ntrace_capture(1, 4)\ntrace_export(\"missing/out.cftrace\")\n",
    )?;
    assert_failure(&missing_parent, "sandbox_io", Some(3))?;
    assert!(!sandbox.root().join("missing").exists());

    // Missing and oversized reads are typed and bounded.
    let missing_read = run_in(
        sandbox.root(),
        "open()\ntrace_capture(1, 4)\ntrace_compare(\"missing.cftrace\")\n",
    )?;
    assert_failure(&missing_read, "sandbox_io", Some(3))?;
    let oversized = sandbox.root().join("big.cftrace");
    let file = fs::File::create(&oversized).map_err(|error| format!("create big: {error}"))?;
    file.set_len(
        u64::try_from(MAX_TRACE_BYTES)
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    )
    .map_err(|error| format!("size big: {error}"))?;
    let oversized_read = run_in(
        sandbox.root(),
        "open()\ntrace_capture(1, 4)\ntrace_compare(\"big.cftrace\")\n",
    )?;
    assert_failure(&oversized_read, "sandbox_too_large", Some(3))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        // A symlink that resolves outside the canonical root is rejected.
        let outside_trace = sandbox.outside().join("escape.cftrace");
        fs::write(&outside_trace, &trace)
            .map_err(|error| format!("write outside trace: {error}"))?;
        symlink(&outside_trace, sandbox.root().join("escape.cftrace"))
            .map_err(|error| format!("create symlink: {error}"))?;
        let escaped = run_in(
            sandbox.root(),
            "open()\ntrace_capture(1, 4)\ntrace_compare(\"escape.cftrace\")\n",
        )?;
        assert_failure(&escaped, "sandbox_path", Some(3))?;

        // A dangling symlink must not let a write create its outside target.
        let dangling_target = sandbox.outside().join("dangling-created.cftrace");
        let _ = fs::remove_file(&dangling_target);
        symlink(&dangling_target, sandbox.root().join("dangling.cftrace"))
            .map_err(|error| format!("create dangling symlink: {error}"))?;
        let dangling = run_in(
            sandbox.root(),
            "open()\ntrace_capture(1, 4)\ntrace_export(\"dangling.cftrace\")\n",
        )?;
        assert_failure(&dangling, "sandbox_path", Some(3))?;
        assert!(
            !dangling_target.exists(),
            "a rejected write must not create the symlink target"
        );

        // A unix socket is a special file, not a regular file.
        let listener = std::os::unix::net::UnixListener::bind(sandbox.root().join("socket"))
            .map_err(|error| format!("bind socket: {error}"))?;
        let special = run_in(
            sandbox.root(),
            "open()\ntrace_capture(1, 4)\ntrace_compare(\"socket\")\n",
        )?;
        assert_failure(&special, "sandbox_path", Some(3))?;
        drop(listener);
    }

    // A report never contains the sandbox root or a host path.
    for (key, value) in accepted.report_fields() {
        assert!(!key.contains("path"), "field {key}");
        assert!(!value.contains(&root_text), "field {key} value {value}");
    }
    Ok(())
}
