//! One-shot script execution and caller configuration.
//!
//! [`run_source`] is the only entry point. It always returns a [`ScriptRun`]:
//! syntax, evaluation and limit failures are reported as a failed run with a
//! stable code, never as a panic or an untyped error. Callers that need
//! configuration errors before producing a report can call
//! [`ScriptLimits::validate`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_core::hash_bytes;

use crate::clock::{ScriptClock, SystemClock};
use crate::error::LimitKind;
use crate::eval::execute;
use crate::limits::ScriptLimits;
use crate::parser::parse_source;
use crate::report::{ScriptFailure, ScriptRun, ScriptStats};
use crate::sandbox::{Sandbox, SandboxError};

/// Caller-injected configuration for one script run.
pub struct ScriptOptions {
    limits: ScriptLimits,
    clock: Arc<dyn ScriptClock>,
    factory: Arc<dyn BackendFactory>,
    sandbox: Option<Sandbox>,
}

impl ScriptOptions {
    /// Creates options with documented default limits and the system clock.
    #[must_use]
    pub fn new(factory: Arc<dyn BackendFactory>) -> Self {
        Self {
            limits: ScriptLimits::default(),
            clock: Arc::new(SystemClock::new()),
            factory,
            sandbox: None,
        }
    }

    /// Replaces the script limits.
    #[must_use]
    pub fn with_limits(mut self, limits: ScriptLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Replaces the wall-time clock. Used by deterministic tests.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn ScriptClock>) -> Self {
        self.clock = clock;
        self
    }

    /// Enables the sandboxed file builtins under an existing directory root.
    ///
    /// # Errors
    ///
    /// Returns a typed [`SandboxError`] when the root cannot be canonicalized
    /// or is not a directory. The message never echoes the host path.
    pub fn with_sandbox(mut self, root: PathBuf) -> Result<Self, SandboxError> {
        self.sandbox = Some(Sandbox::new(root)?);
        Ok(self)
    }

    /// Returns the configured limits.
    #[must_use]
    pub const fn limits(&self) -> &ScriptLimits {
        &self.limits
    }
}

impl fmt::Debug for ScriptOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptOptions")
            .field("limits", &self.limits)
            .field("clock", &self.clock)
            .field("factory", &"<backend factory>")
            .field("sandbox", &self.sandbox.is_some())
            .finish()
    }
}

/// Executes one script source and returns its deterministic report.
///
/// The report always describes the outcome: a passing run, or a failed run with
/// a stable failure code, deterministic message and (where applicable) the
/// 1-based statement line or syntax column. Configuration errors, source-size
/// violations, syntax errors and evaluation failures are all represented in the
/// report so the caller can print it unchanged.
#[must_use]
pub fn run_source(source: &str, options: ScriptOptions) -> ScriptRun {
    let ScriptOptions {
        limits,
        clock,
        factory,
        sandbox,
    } = options;
    let backend = factory.descriptor().id().as_str().to_owned();
    let source_sha256 = hash_bytes(source.as_bytes()).to_hex();
    if let Err(error) = limits.validate() {
        return failed_run(
            source_sha256,
            backend,
            ScriptStats::default(),
            ScriptFailure::from(error),
        );
    }
    if source.len() > limits.source_bytes {
        let failure = ScriptFailure::new(
            LimitKind::SourceBytes.code(),
            format!(
                "source is {} bytes, limit is {} bytes",
                source.len(),
                limits.source_bytes
            ),
            None,
            None,
        );
        return failed_run(source_sha256, backend, ScriptStats::default(), failure);
    }
    let program = match parse_source(source) {
        Ok(program) => program,
        Err(error) => {
            return failed_run(
                source_sha256,
                backend,
                ScriptStats::default(),
                ScriptFailure::from(error),
            );
        }
    };
    let execution = execute(&program, &limits, clock.as_ref(), factory, sandbox.as_ref());
    ScriptRun::new(
        source_sha256,
        backend,
        execution.stats,
        execution.prints,
        execution.report,
        execution.failure,
    )
}

fn failed_run(
    source_sha256: String,
    backend: String,
    stats: ScriptStats,
    failure: ScriptFailure,
) -> ScriptRun {
    ScriptRun::new(
        source_sha256,
        backend,
        stats,
        Vec::new(),
        BTreeMap::new(),
        Some(failure),
    )
}
