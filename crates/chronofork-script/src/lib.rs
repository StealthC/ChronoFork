//! Sandboxed deterministic scripting runtime for ChronoFork experiments.
//!
//! The crate implements a small, purpose-built DSL: integer, boolean and text
//! values; `let`/`set` variables; `print`/`assert`; `if`/`repeat`/`while`; and
//! calls such as `open`, `step`, `tick`, `state_hash`, `register`, `read`,
//! `reset`, `fork`, `checkpoint`, `restore`, `select`, `trace_capture`,
//! `trace_records`, `trace_export`, `trace_compare`, `tas_verify`,
//! `tas_compare`, `report`, `len`, `str` and `fail`. There are no floats,
//! containers, user functions, imports, host environment access, network,
//! clock, random or filesystem access beyond the explicit sandbox.
//!
//! # Sandbox
//!
//! File builtins require [`ScriptOptions::with_sandbox`]. Paths are relative,
//! resolved under the canonicalized root, may not contain `.`/`..`, and may not
//! follow a symlink or special file outside the root. Reads are bounded by the
//! trace and movie format limits. The sandbox is a deterministic capability
//! boundary for experiments, not an OS security boundary against hostile native
//! code.
//!
//! # Pipeline
//!
//! Source bytes are lexed and parsed into a typed AST, then evaluated against
//! [`ScriptLimits`] and an injected [`ScriptClock`]. The result is one
//! deterministic [`ScriptRun`] report. A script-level failure is a failed run
//! with a stable code, never a panic.
//!
//! # Determinism and limits
//!
//! Wall time is a safety abort only: it is read through the injected clock and
//! never enters a successful report. Source, operation, loop, variable, text,
//! report and backend-step budgets are checked before the bounded work.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use chronofork_script::{run_source, ScriptOptions};
//! use chronofork_test_backend::SyntheticBackendFactory;
//!
//! let options = ScriptOptions::new(Arc::new(SyntheticBackendFactory::default()));
//! let run = run_source("print 1 + 2\n", options);
//! assert_eq!(run.prints(), ["3"]);
//! assert_eq!(run.status().as_str(), "passed");
//! ```

mod clock;
mod error;
mod eval;
mod host;
mod lexer;
mod limits;
mod parser;
mod report;
mod runtime;
mod sandbox;

pub use clock::{ScriptClock, SystemClock};
pub use error::{LimitsError, ParseError, ParseErrorKind};
pub use limits::{
    DEFAULT_LOOP_ITERATIONS, DEFAULT_OPERATIONS, DEFAULT_REPORT_BYTES, DEFAULT_SNAPSHOTS,
    DEFAULT_SOURCE_BYTES, DEFAULT_STEPS, DEFAULT_STRING_BYTES, DEFAULT_TRACE_RECORDS,
    DEFAULT_VARIABLES, DEFAULT_WALL_TIME_MS, MAX_LOOP_ITERATIONS, MAX_OPERATIONS, MAX_REPORT_BYTES,
    MAX_SNAPSHOTS, MAX_SOURCE_BYTES, MAX_STEPS, MAX_STRING_BYTES, MAX_TEXT_BYTES,
    MAX_TRACE_RECORDS, MAX_VARIABLES, ScriptLimits, WALL_CHECK_INTERVAL_OPERATIONS,
};
pub use report::{ReportValue, ScriptFailure, ScriptRun, ScriptStats, ScriptStatus};
pub use runtime::{ScriptOptions, run_source};
pub use sandbox::SandboxError;
