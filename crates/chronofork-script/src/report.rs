//! Deterministic script report and run statistics.
//!
//! A [`ScriptRun`] is the complete result of one script run. It contains no
//! wall-clock time, process identifier, hostname, path or unordered container,
//! so two identical runs with identical limits produce equal reports.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::error::{EvalError, LimitError, LimitsError, ParseError};

/// Final status of one script run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptStatus {
    /// The script completed every statement without a failure.
    Passed,
    /// The script stopped on a typed evaluation or limit failure.
    Failed,
}

impl ScriptStatus {
    /// Returns the stable lowercase status name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for ScriptStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a script run failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptFailure {
    code: String,
    message: String,
    line: Option<usize>,
    column: Option<usize>,
}

impl ScriptFailure {
    pub(crate) fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        line: Option<usize>,
        column: Option<usize>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            line,
            column,
        }
    }

    /// Returns the stable failure code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the deterministic failure message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the 1-based statement line, when the failure has one.
    #[must_use]
    pub const fn line(&self) -> Option<usize> {
        self.line
    }

    /// Returns the 1-based source column for syntax failures.
    #[must_use]
    pub const fn column(&self) -> Option<usize> {
        self.column
    }
}

impl fmt::Display for ScriptFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ScriptFailure {}

impl From<ParseError> for ScriptFailure {
    fn from(error: ParseError) -> Self {
        Self::new(
            error.code(),
            error.kind().to_string(),
            Some(error.line()),
            Some(error.column()),
        )
    }
}

impl From<EvalError> for ScriptFailure {
    fn from(error: EvalError) -> Self {
        Self::new(error.code(), error.message(), Some(error.line()), None)
    }
}

impl From<LimitError> for ScriptFailure {
    fn from(error: LimitError) -> Self {
        Self::new(error.code(), error.to_string(), None, None)
    }
}

impl From<LimitsError> for ScriptFailure {
    fn from(error: LimitsError) -> Self {
        Self::new(error.code(), error.to_string(), None, None)
    }
}

/// A report value produced by the `report(key, value)` builtin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportValue {
    /// Signed integer value.
    Int(i64),
    /// Boolean value.
    Bool(bool),
    /// UTF-8 text value.
    Text(String),
}

impl ReportValue {
    /// Renders the value deterministically.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

impl fmt::Display for ReportValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(value) => write!(f, "{value}"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Text(text) => f.write_str(text),
        }
    }
}

/// Deterministic counters reported for one script run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScriptStats {
    pub(crate) statements: u64,
    pub(crate) operations: u64,
    pub(crate) assertions: u64,
    pub(crate) steps: u64,
    pub(crate) variables: usize,
    pub(crate) report_bytes: usize,
    pub(crate) trace_records: u64,
}

impl ScriptStats {
    /// Returns the number of executed statements.
    #[must_use]
    pub const fn statements(&self) -> u64 {
        self.statements
    }

    /// Returns the number of charged operations.
    #[must_use]
    pub const fn operations(&self) -> u64 {
        self.operations
    }

    /// Returns the number of executed assertions.
    #[must_use]
    pub const fn assertions(&self) -> u64 {
        self.assertions
    }

    /// Returns the number of executed backend steps.
    #[must_use]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// Returns the number of live variables at the end of the run.
    #[must_use]
    pub const fn variables(&self) -> usize {
        self.variables
    }

    /// Returns the report key/value text size in bytes.
    #[must_use]
    pub const fn report_bytes(&self) -> usize {
        self.report_bytes
    }

    /// Returns the number of retained trace records in the stored trace.
    #[must_use]
    pub const fn trace_records(&self) -> u64 {
        self.trace_records
    }
}

/// The complete deterministic result of one script run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptRun {
    status: ScriptStatus,
    failure: Option<ScriptFailure>,
    source_sha256: String,
    stats: ScriptStats,
    prints: Vec<String>,
    report: BTreeMap<String, ReportValue>,
    backend: String,
}

impl ScriptRun {
    pub(crate) fn new(
        source_sha256: String,
        backend: String,
        stats: ScriptStats,
        prints: Vec<String>,
        report: BTreeMap<String, ReportValue>,
        failure: Option<ScriptFailure>,
    ) -> Self {
        let status = if failure.is_some() {
            ScriptStatus::Failed
        } else {
            ScriptStatus::Passed
        };
        Self {
            status,
            failure,
            source_sha256,
            stats,
            prints,
            report,
            backend,
        }
    }

    /// Returns the final status.
    #[must_use]
    pub const fn status(&self) -> ScriptStatus {
        self.status
    }

    /// Returns the typed failure, if the run failed.
    #[must_use]
    pub const fn failure(&self) -> Option<&ScriptFailure> {
        self.failure.as_ref()
    }

    /// Returns the SHA-256 of the source bytes as 64 lowercase hex characters.
    #[must_use]
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    /// Returns the deterministic run counters.
    #[must_use]
    pub const fn stats(&self) -> &ScriptStats {
        &self.stats
    }

    /// Returns the ordered `print` output lines.
    #[must_use]
    pub fn prints(&self) -> &[String] {
        &self.prints
    }

    /// Returns the custom report fields, ordered by key.
    #[must_use]
    pub const fn report(&self) -> &BTreeMap<String, ReportValue> {
        &self.report
    }

    /// Returns the stable backend identifier reported by the injected factory.
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// Returns the canonical `key=value` report fields in documented order.
    ///
    /// The order is `status`, `failure` (`-` when passed), `message` (only on
    /// failure), `source_sha256`, `statements`, `operations`, `assertions`,
    /// `steps`, `variables`, `report_bytes`, `trace_records`, `backend`, then
    /// one `report.<key>` field per custom report entry sorted by key.
    #[must_use]
    pub fn report_fields(&self) -> Vec<(String, String)> {
        let mut fields = Vec::new();
        fields.push(("status".to_owned(), self.status.as_str().to_owned()));
        fields.push((
            "failure".to_owned(),
            self.failure
                .as_ref()
                .map_or_else(|| "-".to_owned(), |failure| failure.code.clone()),
        ));
        if let Some(failure) = &self.failure {
            fields.push(("message".to_owned(), failure.message.clone()));
        }
        fields.push(("source_sha256".to_owned(), self.source_sha256.clone()));
        fields.push(("statements".to_owned(), self.stats.statements.to_string()));
        fields.push(("operations".to_owned(), self.stats.operations.to_string()));
        fields.push(("assertions".to_owned(), self.stats.assertions.to_string()));
        fields.push(("steps".to_owned(), self.stats.steps.to_string()));
        fields.push(("variables".to_owned(), self.stats.variables.to_string()));
        fields.push((
            "report_bytes".to_owned(),
            self.stats.report_bytes.to_string(),
        ));
        fields.push((
            "trace_records".to_owned(),
            self.stats.trace_records.to_string(),
        ));
        fields.push(("backend".to_owned(), self.backend.clone()));
        for (key, value) in &self.report {
            fields.push((format!("report.{key}"), value.render()));
        }
        fields
    }
}
