//! Limit acceptance tests: every documented bound, invalid construction and
//! deterministic wall-time enforcement through an injected fake clock.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chronofork_script::{
    DEFAULT_LOOP_ITERATIONS, DEFAULT_OPERATIONS, DEFAULT_REPORT_BYTES, DEFAULT_SNAPSHOTS,
    DEFAULT_SOURCE_BYTES, DEFAULT_STEPS, DEFAULT_STRING_BYTES, DEFAULT_TRACE_RECORDS,
    DEFAULT_VARIABLES, DEFAULT_WALL_TIME_MS, MAX_LOOP_ITERATIONS, MAX_OPERATIONS, MAX_REPORT_BYTES,
    MAX_SNAPSHOTS, MAX_SOURCE_BYTES, MAX_STEPS, MAX_STRING_BYTES, MAX_TRACE_RECORDS, MAX_VARIABLES,
    ReportValue, ScriptClock, ScriptLimits, ScriptOptions, ScriptRun, ScriptStatus, run_source,
};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

fn options() -> ScriptOptions {
    ScriptOptions::new(Arc::new(SyntheticBackendFactory::default()))
}

fn run_with(source: &str, limits: ScriptLimits) -> ScriptRun {
    run_source(source, options().with_limits(limits))
}

fn failure(run: &ScriptRun) -> Result<&chronofork_script::ScriptFailure, String> {
    run.failure()
        .ok_or_else(|| format!("expected a failed run, got status {}", run.status()))
}

fn assert_failure(run: &ScriptRun, code: &str, line: Option<usize>) -> Result<(), String> {
    assert_eq!(run.status(), ScriptStatus::Failed);
    let failure = failure(run)?;
    assert_eq!(failure.code(), code, "failure: {failure:?}");
    assert_eq!(failure.line(), line, "failure: {failure:?}");
    Ok(())
}

fn assert_limits_invalid(limits: &ScriptLimits, field: &str) -> Result<(), String> {
    match limits.validate() {
        Ok(()) => Err(format!("limits with field {field} should be invalid")),
        Err(error) => {
            assert_eq!(error.code(), "invalid_limit");
            assert_eq!(error.field(), field, "error: {error}");
            Ok(())
        }
    }
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

#[derive(Debug)]
struct FakeClock {
    calls: AtomicU64,
    advance_per_call: u64,
}

impl FakeClock {
    fn new(advance_per_call: u64) -> Self {
        Self {
            calls: AtomicU64::new(0),
            advance_per_call,
        }
    }

    fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ScriptClock for FakeClock {
    fn now_millis(&self) -> u64 {
        self.calls
            .fetch_add(1, Ordering::SeqCst)
            .saturating_mul(self.advance_per_call)
    }
}

#[test]
fn execution_and_memory_limits_are_enforced() -> Result<(), String> {
    // The documented defaults are the ones the constructor actually uses.
    let defaults = ScriptLimits::default();
    assert_eq!(defaults.source_bytes, DEFAULT_SOURCE_BYTES);
    assert_eq!(defaults.operations, DEFAULT_OPERATIONS);
    assert_eq!(defaults.loop_iterations, DEFAULT_LOOP_ITERATIONS);
    assert_eq!(defaults.variables, DEFAULT_VARIABLES);
    assert_eq!(defaults.string_bytes, DEFAULT_STRING_BYTES);
    assert_eq!(defaults.report_bytes, DEFAULT_REPORT_BYTES);
    assert_eq!(defaults.steps, DEFAULT_STEPS);
    assert_eq!(defaults.trace_records, DEFAULT_TRACE_RECORDS);
    assert_eq!(defaults.snapshots, DEFAULT_SNAPSHOTS);
    assert_eq!(defaults.wall_time_ms, Some(DEFAULT_WALL_TIME_MS));
    assert!(defaults.validate().is_ok());

    // source_bytes: accept the exact size, reject one byte less before lexing.
    let source = "print 1\n";
    assert_eq!(source.len(), 8);
    let accepted = run_with(
        source,
        ScriptLimits {
            source_bytes: source.len(),
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    let rejected = run_with(
        source,
        ScriptLimits {
            source_bytes: source.len() - 1,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.source_bytes", None)?;
    assert_eq!(rejected.stats().statements(), 0);

    // operations: `print 1` costs the statement plus one literal.
    let accepted = run_with(
        "print 1\n",
        ScriptLimits {
            operations: 2,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(accepted.stats().operations(), 2);
    let rejected = run_with(
        "print 1\n",
        ScriptLimits {
            operations: 1,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.operations", Some(1))?;

    // loop_iterations: accept the exact repeat count, reject one less.
    let accepted = run_with(
        "repeat 3 {\nprint 1\n}\n",
        ScriptLimits {
            loop_iterations: 3,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    let rejected = run_with(
        "repeat 3 {\nprint 1\n}\n",
        ScriptLimits {
            loop_iterations: 2,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.loop_iterations", Some(1))?;

    // variables: accept the exact count, reject the insertion of the next one.
    let declaration = "let a = 1\nlet b = 2\nlet c = 3\n";
    let accepted = run_with(
        declaration,
        ScriptLimits {
            variables: 3,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(accepted.stats().variables(), 3);
    let rejected = run_with(
        declaration,
        ScriptLimits {
            variables: 2,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.variables", Some(3))?;
    assert_eq!(rejected.stats().variables(), 2);

    // string_bytes: live variable text is charged before insertion.
    let accepted = run_with(
        "let text = \"12345\"\n",
        ScriptLimits {
            string_bytes: 5,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    let rejected = run_with(
        "let text = \"12345\"\n",
        ScriptLimits {
            string_bytes: 4,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.string_bytes", Some(1))?;
    assert_eq!(rejected.stats().variables(), 0);

    // string_bytes also covers report key/value text.
    let accepted = run_with(
        "report(\"key\", \"value\")\n",
        ScriptLimits {
            string_bytes: 8,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    let rejected = run_with(
        "report(\"key\", \"value\")\n",
        ScriptLimits {
            string_bytes: 7,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.string_bytes", Some(1))?;

    // report_bytes: key plus rendered value, checked before insertion.
    let accepted = run_with(
        "report(\"key\", \"value\")\n",
        ScriptLimits {
            report_bytes: 8,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(accepted.stats().report_bytes(), 8);
    let rejected = run_with(
        "report(\"key\", \"value\")\n",
        ScriptLimits {
            report_bytes: 7,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.report_bytes", Some(1))?;
    assert_eq!(rejected.stats().report_bytes(), 0);

    // steps: a request beyond the budget aborts before any partial advance.
    let rejected = run_with(
        "open()\nstep(3)\n",
        ScriptLimits {
            steps: 2,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.steps", Some(2))?;
    assert_eq!(rejected.stats().steps(), 0);
    let accepted = run_with(
        "open()\nprint step(3)\n",
        ScriptLimits {
            steps: 3,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(accepted.stats().steps(), 3);
    assert_eq!(accepted.prints(), ["3"]);

    // Documented maxima are accepted; one above is a typed construction error.
    let maximum = ScriptLimits {
        source_bytes: MAX_SOURCE_BYTES,
        operations: MAX_OPERATIONS,
        loop_iterations: MAX_LOOP_ITERATIONS,
        variables: MAX_VARIABLES,
        string_bytes: MAX_STRING_BYTES,
        report_bytes: MAX_REPORT_BYTES,
        steps: MAX_STEPS,
        trace_records: MAX_TRACE_RECORDS,
        snapshots: MAX_SNAPSHOTS,
        wall_time_ms: Some(0),
    };
    assert!(maximum.validate().is_ok());
    let unlimited = ScriptLimits {
        wall_time_ms: None,
        ..maximum
    };
    assert!(unlimited.validate().is_ok());

    let above = [
        (
            "source_bytes",
            ScriptLimits {
                source_bytes: MAX_SOURCE_BYTES + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "operations",
            ScriptLimits {
                operations: MAX_OPERATIONS + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "loop_iterations",
            ScriptLimits {
                loop_iterations: MAX_LOOP_ITERATIONS + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "variables",
            ScriptLimits {
                variables: MAX_VARIABLES + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "string_bytes",
            ScriptLimits {
                string_bytes: MAX_STRING_BYTES + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "report_bytes",
            ScriptLimits {
                report_bytes: MAX_REPORT_BYTES + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "steps",
            ScriptLimits {
                steps: MAX_STEPS + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "trace_records",
            ScriptLimits {
                trace_records: MAX_TRACE_RECORDS + 1,
                ..ScriptLimits::default()
            },
        ),
        (
            "snapshots",
            ScriptLimits {
                snapshots: MAX_SNAPSHOTS + 1,
                ..ScriptLimits::default()
            },
        ),
    ];
    for (field, limits) in above {
        assert_limits_invalid(&limits, field)?;
    }

    let zeroed = [
        (
            "source_bytes",
            ScriptLimits {
                source_bytes: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "operations",
            ScriptLimits {
                operations: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "loop_iterations",
            ScriptLimits {
                loop_iterations: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "variables",
            ScriptLimits {
                variables: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "string_bytes",
            ScriptLimits {
                string_bytes: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "report_bytes",
            ScriptLimits {
                report_bytes: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "steps",
            ScriptLimits {
                steps: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "trace_records",
            ScriptLimits {
                trace_records: 0,
                ..ScriptLimits::default()
            },
        ),
        (
            "snapshots",
            ScriptLimits {
                snapshots: 0,
                ..ScriptLimits::default()
            },
        ),
    ];
    for (field, limits) in zeroed {
        assert_limits_invalid(&limits, field)?;
    }

    // Invalid limits surface as a failed run, not a panic or partial execution.
    let invalid = run_with(
        "print 1\n",
        ScriptLimits {
            operations: 0,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&invalid, "invalid_limit", None)?;
    assert_eq!(invalid.stats().statements(), 0);
    assert_eq!(invalid.stats().operations(), 0);

    // trace_records: the capture capacity is checked before any step executes.
    let accepted = run_with(
        "open()\ntrace_capture(4, 4)\n",
        ScriptLimits {
            trace_records: 4,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(accepted.stats().trace_records(), 4);
    assert_eq!(accepted.stats().steps(), 4);
    let rejected = run_with(
        "open()\ntrace_capture(4, 4)\n",
        ScriptLimits {
            trace_records: 3,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.trace_records", Some(2))?;
    assert_eq!(rejected.stats().steps(), 0);

    // string_bytes: stored trace bytes count against the aggregate text budget.
    let rejected = run_with(
        "open()\ntrace_capture(4, 16)\n",
        ScriptLimits {
            string_bytes: 100,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.string_bytes", Some(2))?;
    assert_eq!(rejected.stats().steps(), 0);
    let accepted = run_with(
        "open()\ntrace_capture(4, 16)\n",
        ScriptLimits {
            string_bytes: 4096,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(accepted.stats().trace_records(), 4);

    // snapshots: checkpoint/fork creation is bounded before it happens.
    let accepted = run_with(
        "open()\ncheckpoint()\ncheckpoint()\n",
        ScriptLimits {
            snapshots: 2,
            ..ScriptLimits::default()
        },
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    let rejected = run_with(
        "open()\ncheckpoint()\ncheckpoint()\n",
        ScriptLimits {
            snapshots: 1,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&rejected, "limit.snapshots", Some(3))?;

    // No limit failure depends on host speed: the whole test uses no sleeps.
    Ok(())
}

#[test]
fn wall_time_limit_uses_injected_clock() -> Result<(), String> {
    let program = "let x = 1\nprint x\n";

    // A non-advancing fake clock passes and matches a run with the deadline off.
    let still = Arc::new(FakeClock::new(0));
    let limited = run_source(
        program,
        options()
            .with_limits(ScriptLimits {
                wall_time_ms: Some(1),
                ..ScriptLimits::default()
            })
            .with_clock(Arc::clone(&still) as Arc<dyn ScriptClock>),
    );
    let unlimited = run_source(
        program,
        options().with_limits(ScriptLimits {
            wall_time_ms: None,
            ..ScriptLimits::default()
        }),
    );
    assert_eq!(limited.status(), ScriptStatus::Passed);
    assert_eq!(unlimited.status(), ScriptStatus::Passed);
    assert_eq!(report_text(&limited), report_text(&unlimited));
    assert_eq!(limited.prints(), ["1"]);
    assert!(still.calls() > 0);

    // An advancing fake clock aborts at the second statement boundary.
    let advancing = Arc::new(FakeClock::new(1));
    let aborted = run_source(
        program,
        options()
            .with_limits(ScriptLimits {
                wall_time_ms: Some(1),
                ..ScriptLimits::default()
            })
            .with_clock(Arc::clone(&advancing) as Arc<dyn ScriptClock>),
    );
    assert_failure(&aborted, "limit.wall_time", Some(2))?;
    assert_eq!(aborted.stats().statements(), 1);
    assert!(aborted.prints().is_empty());
    assert_eq!(advancing.calls(), 2);

    // The 256-operation checkpoint is deterministic: a single long expression
    // aborts exactly when the operation counter reaches the interval.
    let mut expression = String::from("let x = 1");
    for _ in 0..200 {
        expression.push_str("+1");
    }
    expression.push('\n');
    let operations_clock = Arc::new(FakeClock::new(1));
    let aborted = run_source(
        &expression,
        options()
            .with_limits(ScriptLimits {
                wall_time_ms: Some(1),
                ..ScriptLimits::default()
            })
            .with_clock(Arc::clone(&operations_clock) as Arc<dyn ScriptClock>),
    );
    assert_failure(&aborted, "limit.wall_time", Some(1))?;
    assert_eq!(aborted.stats().operations(), 256);
    assert_eq!(operations_clock.calls(), 2);

    // `None` disables enforcement even when the clock advances every check.
    let advancing = Arc::new(FakeClock::new(1000));
    let unlimited = run_source(
        "repeat 5 {\nprint 1\n}\n",
        options()
            .with_limits(ScriptLimits {
                wall_time_ms: None,
                ..ScriptLimits::default()
            })
            .with_clock(Arc::clone(&advancing) as Arc<dyn ScriptClock>),
    );
    assert_eq!(unlimited.status(), ScriptStatus::Passed);
    assert_eq!(unlimited.prints().len(), 5);
    assert_eq!(advancing.calls(), 0);

    // A passing report never exposes elapsed time or a path.
    let fields = limited.report_fields();
    for (key, value) in &fields {
        assert!(!key.contains("time"), "field {key}");
        assert!(!key.contains("path"), "field {key}");
        assert!(!value.contains('/'), "field {key} value {value}");
    }
    Ok(())
}

#[test]
fn step_reservations_are_conservative_and_typed() -> Result<(), String> {
    // The program halts on its first step: trace_capture executes one step but
    // reserves the requested count, as ADR-0011 §3.5 documents.
    let program = SyntheticProgram::new(vec![0xff]).map_err(|error| format!("program: {error}"))?;
    let factory = Arc::new(SyntheticBackendFactory::new(program));
    let run = run_source(
        "open()\nlet records = trace_capture(5, 8)\nreport(\"records\", records)\n",
        ScriptOptions::new(factory),
    );
    assert_eq!(run.status(), ScriptStatus::Passed);
    assert_eq!(run.stats().steps(), 5, "the requested count is reserved");
    assert_eq!(
        run.report().get("records").map(ReportValue::render),
        Some("1".to_owned()),
        "the halted capture retained the halted instruction"
    );

    // A request above the remaining budget is a typed rejection before any step.
    let program = SyntheticProgram::new(vec![0xff]).map_err(|error| format!("program: {error}"))?;
    let factory = Arc::new(SyntheticBackendFactory::new(program));
    let limits = ScriptLimits {
        steps: 4,
        ..ScriptLimits::default()
    };
    let limited = run_source(
        "open()\ntrace_capture(5, 8)\n",
        ScriptOptions::new(factory).with_limits(limits),
    );
    assert_failure(&limited, "limit.steps", Some(2))?;
    assert_eq!(limited.stats().steps(), 0, "no partial step is executed");
    Ok(())
}
