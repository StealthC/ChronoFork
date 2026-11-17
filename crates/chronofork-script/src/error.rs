//! Typed parse, evaluation and limit errors for the script runtime.
//!
//! Every failure carries a stable machine-readable code. Syntax errors also
//! carry a 1-based line and column; evaluation errors carry the 1-based line of
//! the statement that was executing. Messages are deterministic and never
//! contain host paths, wall-clock time, process identifiers or entropy.

use std::error::Error;
use std::fmt;

/// Kinds of syntax error reported by the lexer or parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseErrorKind {
    /// The source contains no statements (empty, comment-only or blank lines).
    EmptyProgram,
    /// A character that is not part of the script language.
    UnknownCharacter(char),
    /// A string literal is not terminated before the end of its line or file.
    UnterminatedString,
    /// A string escape other than `\\`, `\"`, `\n`, `\r` or `\t`.
    InvalidEscape(char),
    /// An integer literal does not fit `i64`.
    InvalidNumber,
    /// A string literal decodes to more than the per-value text bound.
    StringTooLong,
    /// A specific token was required at this position.
    ExpectedToken {
        /// Human-readable description of the required token.
        expected: &'static str,
    },
    /// The token at this position cannot start the construct being parsed.
    UnexpectedToken {
        /// Human-readable description of the found token.
        found: &'static str,
    },
    /// The source ended while a construct was still open.
    UnexpectedEndOfFile {
        /// Human-readable description of the token that was required.
        expected: &'static str,
    },
    /// `else` appeared without a matching `if`.
    StrayElse,
    /// A statement was not terminated by a newline.
    MissingNewline,
    /// Expression or block nesting exceeds the supported depth.
    NestingTooDeep,
}

impl ParseErrorKind {
    /// Returns the stable code for this kind of syntax error.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptyProgram => "parse.empty_program",
            Self::UnknownCharacter(_) => "parse.unknown_character",
            Self::UnterminatedString => "parse.unterminated_string",
            Self::InvalidEscape(_) => "parse.invalid_escape",
            Self::InvalidNumber => "parse.invalid_number",
            Self::StringTooLong => "parse.string_too_long",
            Self::ExpectedToken { .. } => "parse.expected_token",
            Self::UnexpectedToken { .. } => "parse.unexpected_token",
            Self::UnexpectedEndOfFile { .. } => "parse.unexpected_eof",
            Self::StrayElse => "parse.stray_else",
            Self::MissingNewline => "parse.missing_newline",
            Self::NestingTooDeep => "parse.nesting_too_deep",
        }
    }
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyProgram => f.write_str("script contains no statements"),
            Self::UnknownCharacter(character) => {
                write!(f, "unknown character {character:?}")
            }
            Self::UnterminatedString => f.write_str("unterminated string literal"),
            Self::InvalidEscape(character) => {
                write!(f, "unknown string escape '\\{character}'")
            }
            Self::InvalidNumber => f.write_str("integer literal does not fit i64"),
            Self::StringTooLong => {
                f.write_str("string literal exceeds the 4096-byte per-value text bound")
            }
            Self::ExpectedToken { expected } => write!(f, "expected {expected}"),
            Self::UnexpectedToken { found } => write!(f, "unexpected {found}"),
            Self::UnexpectedEndOfFile { expected } => {
                write!(f, "unexpected end of file; expected {expected}")
            }
            Self::StrayElse => f.write_str("'else' without a matching 'if'"),
            Self::MissingNewline => f.write_str("expected a newline after the statement"),
            Self::NestingTooDeep => f.write_str("nesting exceeds the supported depth"),
        }
    }
}

/// A typed syntax error with a 1-based source position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseError {
    kind: ParseErrorKind,
    line: usize,
    column: usize,
}

impl ParseError {
    pub(crate) const fn new(kind: ParseErrorKind, line: usize, column: usize) -> Self {
        Self { kind, line, column }
    }

    /// Returns the kind of syntax error.
    #[must_use]
    pub const fn kind(self) -> ParseErrorKind {
        self.kind
    }

    /// Returns the stable code for this syntax error.
    #[must_use]
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }

    /// Returns the 1-based source line.
    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    /// Returns the 1-based source column.
    #[must_use]
    pub const fn column(self) -> usize {
        self.column
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.kind
        )
    }
}

impl Error for ParseError {}

/// Kinds of runtime limit enforced by the evaluator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LimitKind {
    SourceBytes,
    Operations,
    LoopIterations,
    Variables,
    StringBytes,
    ReportBytes,
    Steps,
    TraceRecords,
    Snapshots,
    WallTime,
}

impl LimitKind {
    /// Returns the stable failure code for this limit.
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::SourceBytes => "limit.source_bytes",
            Self::Operations => "limit.operations",
            Self::LoopIterations => "limit.loop_iterations",
            Self::Variables => "limit.variables",
            Self::StringBytes => "limit.string_bytes",
            Self::ReportBytes => "limit.report_bytes",
            Self::Steps => "limit.steps",
            Self::TraceRecords => "limit.trace_records",
            Self::Snapshots => "limit.snapshots",
            Self::WallTime => "limit.wall_time",
        }
    }
}

/// A script limit that was exceeded or that aborted the run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LimitError {
    kind: LimitKind,
    limit: u64,
}

impl LimitError {
    pub(crate) const fn new(kind: LimitKind, limit: u64) -> Self {
        Self { kind, limit }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            LimitKind::WallTime => write!(f, "wall-time limit of {} ms elapsed", self.limit),
            _ => write!(f, "script limit of {} exceeded", self.limit),
        }
    }
}

/// An invalid [`ScriptLimits`](crate::ScriptLimits) configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsError {
    field: &'static str,
    value: u64,
    maximum: u64,
    zero: bool,
}

impl LimitsError {
    pub(crate) const fn zero(field: &'static str, maximum: u64) -> Self {
        Self {
            field,
            value: 0,
            maximum,
            zero: true,
        }
    }

    pub(crate) const fn above(field: &'static str, value: u64, maximum: u64) -> Self {
        Self {
            field,
            value,
            maximum,
            zero: false,
        }
    }

    /// Returns the stable failure code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "invalid_limit"
    }

    /// Returns the field that failed validation.
    #[must_use]
    pub const fn field(&self) -> &'static str {
        self.field
    }

    /// Returns the rejected value.
    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }

    /// Returns the documented maximum for the field.
    #[must_use]
    pub const fn maximum(&self) -> u64 {
        self.maximum
    }
}

impl fmt::Display for LimitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.zero {
            write!(
                f,
                "invalid script limit '{}': 0 must be at least 1",
                self.field
            )
        } else {
            write!(
                f,
                "invalid script limit '{}': {} exceeds the maximum {}",
                self.field, self.value, self.maximum
            )
        }
    }
}

impl Error for LimitsError {}

/// Kinds of runtime evaluation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EvalErrorKind {
    UnknownVariable(String),
    DuplicateVariable(String),
    UnknownBuiltin(String),
    Arity {
        name: String,
        expected: usize,
        found: usize,
    },
    Type {
        expected: &'static str,
        found: &'static str,
    },
    UnknownRegister(String),
    NotOpen,
    AlreadyOpen,
    Halted,
    UnknownSession,
    UnknownSnapshot,
    MemoryOutOfBounds {
        address: u64,
        length: usize,
    },
    Session(String),
    InvalidLoopCount(i64),
    InvalidStepCount(i64),
    InvalidAddress(i64),
    InvalidLength(i64),
    ReportKeyInvalid(String),
    ReportKeyDuplicate(String),
    TextTooLong,
    ArithmeticOverflow,
    DivisionByZero,
    AssertionFailed,
    Fail(String),
    IntegerOutOfRange,
    InvalidHandle(i64),
    InvalidTraceSteps(i64),
    InvalidTraceCapacity(i64),
    NoStoredTrace,
    SandboxUnavailable,
    SandboxRoot,
    SandboxPath,
    SandboxIo,
    SandboxTooLarge,
    TraceCapture(String),
    TraceParse(String),
    MovieDecode(String),
    MovieReplay(String),
    MovieCheckpointMismatch,
}

impl EvalErrorKind {
    /// Returns the stable failure code for this evaluation error.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::UnknownVariable(_) => "unknown_variable",
            Self::DuplicateVariable(_) => "duplicate_variable",
            Self::UnknownBuiltin(_) => "unknown_builtin",
            Self::Arity { .. } => "arity",
            Self::Type { .. } => "type",
            Self::UnknownRegister(_) => "unknown_register",
            Self::NotOpen => "not_open",
            Self::AlreadyOpen => "already_open",
            Self::Halted => "halted",
            Self::UnknownSession => "unknown_session",
            Self::UnknownSnapshot => "unknown_snapshot",
            Self::MemoryOutOfBounds { .. } => "memory_out_of_bounds",
            Self::Session(_) => "session",
            Self::InvalidLoopCount(_) => "invalid_loop_count",
            Self::InvalidStepCount(_) => "invalid_step_count",
            Self::InvalidAddress(_) => "invalid_address",
            Self::InvalidLength(_) => "invalid_length",
            Self::ReportKeyInvalid(_) => "report_key_invalid",
            Self::ReportKeyDuplicate(_) => "report_key_duplicate",
            Self::TextTooLong => "text_too_long",
            Self::ArithmeticOverflow => "arithmetic_overflow",
            Self::DivisionByZero => "division_by_zero",
            Self::AssertionFailed => "assertion_failed",
            Self::Fail(_) => "fail",
            Self::IntegerOutOfRange => "integer_out_of_range",
            Self::InvalidHandle(_) => "invalid_handle",
            Self::InvalidTraceSteps(_) => "invalid_trace_steps",
            Self::InvalidTraceCapacity(_) => "invalid_trace_capacity",
            Self::NoStoredTrace => "no_stored_trace",
            Self::SandboxUnavailable => "sandbox_unavailable",
            Self::SandboxRoot => "sandbox_root",
            Self::SandboxPath => "sandbox_path",
            Self::SandboxIo => "sandbox_io",
            Self::SandboxTooLarge => "sandbox_too_large",
            Self::TraceCapture(_) => "trace_capture",
            Self::TraceParse(_) => "trace_parse",
            Self::MovieDecode(_) => "movie_decode",
            Self::MovieReplay(_) => "movie_replay",
            Self::MovieCheckpointMismatch => "movie_checkpoint_mismatch",
        }
    }
}

impl fmt::Display for EvalErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVariable(name) => write!(f, "unknown variable '{name}'"),
            Self::DuplicateVariable(name) => {
                write!(f, "variable '{name}' is already defined")
            }
            Self::UnknownBuiltin(name) => write!(f, "unknown builtin '{name}'"),
            Self::Arity {
                name,
                expected,
                found,
            } => write!(
                f,
                "builtin '{name}' expects {expected} arguments, found {found}"
            ),
            Self::Type { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            Self::UnknownRegister(name) => write!(f, "unknown register '{name}'"),
            Self::NotOpen => f.write_str("no session is open; call open() first"),
            Self::AlreadyOpen => f.write_str("a session is already open"),
            Self::Halted => f.write_str("the current session is halted"),
            Self::UnknownSession => f.write_str("the session identifier is unknown"),
            Self::UnknownSnapshot => f.write_str("the checkpoint identifier is unknown"),
            Self::MemoryOutOfBounds { address, length } => write!(
                f,
                "memory access out of bounds: address {address}, length {length}"
            ),
            Self::Session(message) => write!(f, "session error: {message}"),
            Self::InvalidLoopCount(count) => {
                write!(f, "repeat count {count} must not be negative")
            }
            Self::InvalidStepCount(count) => {
                write!(f, "step count {count} must not be negative")
            }
            Self::InvalidAddress(address) => {
                write!(f, "memory address {address} must not be negative")
            }
            Self::InvalidLength(length) => {
                write!(f, "read length {length} must be between 1 and 4096")
            }
            Self::ReportKeyInvalid(key) => write!(f, "report key '{key}' is invalid"),
            Self::ReportKeyDuplicate(key) => {
                write!(f, "report key '{key}' is already defined")
            }
            Self::TextTooLong => {
                f.write_str("text value exceeds the 4096-byte per-value text bound")
            }
            Self::ArithmeticOverflow => f.write_str("checked i64 arithmetic overflow"),
            Self::DivisionByZero => f.write_str("division or remainder by zero"),
            Self::AssertionFailed => f.write_str("assertion failed"),
            Self::Fail(message) => f.write_str(message),
            Self::IntegerOutOfRange => f.write_str("a backend integer does not fit in i64"),
            Self::InvalidHandle(value) => write!(
                f,
                "session or snapshot identifier {value} must not be negative"
            ),
            Self::InvalidTraceSteps(value) => write!(
                f,
                "trace capture steps {value} must be between 1 and 1000000"
            ),
            Self::InvalidTraceCapacity(value) => {
                write!(f, "trace capacity {value} must be between 1 and 1000000")
            }
            Self::NoStoredTrace => f.write_str("no trace is stored; call trace_capture() first"),
            Self::SandboxUnavailable => {
                f.write_str("file builtins require an explicit sandbox root")
            }
            Self::SandboxRoot => f.write_str("the sandbox root must be an existing directory"),
            Self::SandboxPath => {
                f.write_str("sandbox path is not a relative path inside the sandbox root")
            }
            Self::SandboxIo => f.write_str("sandbox file operation failed"),
            Self::SandboxTooLarge => f.write_str("sandbox file exceeds the permitted read bound"),
            Self::TraceCapture(message) => write!(f, "trace capture failed: {message}"),
            Self::TraceParse(message) => write!(f, "trace file is invalid: {message}"),
            Self::MovieDecode(message) => write!(f, "movie file is invalid: {message}"),
            Self::MovieReplay(message) => write!(f, "movie replay failed: {message}"),
            Self::MovieCheckpointMismatch => f.write_str("movie checkpoints differ"),
        }
    }
}

/// An evaluation failure with the 1-based line of the executing statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvalError {
    kind: EvalErrorKind,
    line: usize,
}

impl EvalError {
    pub(crate) const fn new(kind: EvalErrorKind, line: usize) -> Self {
        Self { kind, line }
    }

    pub(crate) fn code(&self) -> &'static str {
        self.kind.code()
    }

    pub(crate) fn message(&self) -> String {
        self.kind.to_string()
    }

    pub(crate) const fn line(&self) -> usize {
        self.line
    }
}

/// A limit failure with the 1-based line at which it was detected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LimitFailure {
    pub(crate) error: LimitError,
    pub(crate) line: usize,
}

/// The two runtime failure channels of the evaluator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunError {
    /// A typing, name, arithmetic, report or host error.
    Eval(EvalError),
    /// A configured resource limit was reached.
    Limit(LimitFailure),
}

impl RunError {
    pub(crate) const fn eval(kind: EvalErrorKind, line: usize) -> Self {
        Self::Eval(EvalError::new(kind, line))
    }

    pub(crate) const fn limit(kind: LimitKind, limit: u64, line: usize) -> Self {
        Self::Limit(LimitFailure {
            error: LimitError::new(kind, limit),
            line,
        })
    }

    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Eval(error) => error.code(),
            Self::Limit(failure) => failure.error.code(),
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Eval(error) => error.message(),
            Self::Limit(failure) => failure.error.to_string(),
        }
    }

    pub(crate) const fn line(&self) -> usize {
        match self {
            Self::Eval(error) => error.line(),
            Self::Limit(failure) => failure.line,
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eval(error) => write!(f, "line {}: {}", error.line(), error.message()),
            Self::Limit(failure) => {
                write!(f, "line {}: {}", failure.line, failure.error)
            }
        }
    }
}
