//! Typed tree-walking evaluator for the ChronoFork script language.
//!
//! Every executed statement, expression node, loop iteration and builtin call
//! is charged against the operation budget before the work. Variables live in
//! one flat program-global environment; blocks introduce no scope.

use std::collections::BTreeMap;
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_sessions::{ComparisonVerdict, MAX_MOVIE_BYTES, Movie, compare_movies};
use chronofork_trace::{
    MAX_CAPTURE_STEPS, MAX_TRACE_BYTES, MAX_TRACE_CAPACITY, Trace, TraceConfig,
};

use crate::clock::ScriptClock;
use crate::error::{EvalErrorKind, LimitKind, RunError};
use crate::host::Host;
use crate::limits::{MAX_TEXT_BYTES, ScriptLimits, WALL_CHECK_INTERVAL_OPERATIONS};
use crate::parser::{BinaryOp, Expr, Program, Stmt, StmtKind, UnaryOp};
use crate::report::{ReportValue, ScriptFailure, ScriptStats};
use crate::sandbox::{Sandbox, SandboxError, SandboxErrorKind};

/// Conservative upper bound for memory accounting of one stored trace record.
///
/// The v1 record body is at most 26 bytes; the in-memory `TraceRecord` value and
/// vector growth are larger, so the accounting estimate uses 64 bytes per
/// record plus a fixed header allowance. This is an accounting bound only, not
/// a second trace codec.
const TRACE_RECORD_MEMORY_BYTES: usize = 64;
/// Fixed header allowance for one stored trace.
const TRACE_HEADER_MEMORY_BYTES: usize = 512;

fn trace_memory_estimate(capacity: usize) -> usize {
    TRACE_HEADER_MEMORY_BYTES.saturating_add(capacity.saturating_mul(TRACE_RECORD_MEMORY_BYTES))
}

/// A runtime value. Scripts have no floats, containers, functions or `null`.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Value {
    Int(i64),
    Bool(bool),
    Text(String),
}

/// The non-short-circuiting binary operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StrictBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl StrictBinaryOp {
    const fn from_binary(operator: BinaryOp) -> Option<Self> {
        match operator {
            BinaryOp::Add => Some(Self::Add),
            BinaryOp::Sub => Some(Self::Sub),
            BinaryOp::Mul => Some(Self::Mul),
            BinaryOp::Div => Some(Self::Div),
            BinaryOp::Rem => Some(Self::Rem),
            BinaryOp::Eq => Some(Self::Eq),
            BinaryOp::Ne => Some(Self::Ne),
            BinaryOp::Lt => Some(Self::Lt),
            BinaryOp::Le => Some(Self::Le),
            BinaryOp::Gt => Some(Self::Gt),
            BinaryOp::Ge => Some(Self::Ge),
            BinaryOp::And | BinaryOp::Or => None,
        }
    }
}

/// Integer-only arithmetic operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IntegerOp {
    Sub,
    Mul,
}

/// Checked division operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DivisionOp {
    Divide,
    Remainder,
}

/// Ordering comparisons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompareOp {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl Value {
    const fn type_name(&self) -> &'static str {
        match self {
            Self::Int(_) => "Int",
            Self::Bool(_) => "Bool",
            Self::Text(_) => "Text",
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Text(text) => text.clone(),
        }
    }

    fn text_len(&self) -> Option<usize> {
        match self {
            Self::Text(text) => Some(text.len()),
            Self::Int(_) | Self::Bool(_) => None,
        }
    }
}

/// The evaluator output handed back to the runtime.
pub(crate) struct Execution {
    pub(crate) prints: Vec<String>,
    pub(crate) report: BTreeMap<String, ReportValue>,
    pub(crate) stats: ScriptStats,
    pub(crate) failure: Option<ScriptFailure>,
}

/// Executes a parsed program with the given limits, clock and backend factory.
pub(crate) fn execute(
    program: &Program,
    limits: &ScriptLimits,
    clock: &dyn ScriptClock,
    factory: Arc<dyn BackendFactory>,
    sandbox: Option<&Sandbox>,
) -> Execution {
    let mut evaluator = Evaluator {
        limits,
        clock,
        host: Host::new(factory),
        sandbox,
        stored_trace: None,
        stored_trace_bytes: 0,
        snapshots: 0,
        environment: BTreeMap::new(),
        prints: Vec::new(),
        report: BTreeMap::new(),
        stats: ScriptStats::default(),
        loop_iterations: 0,
        string_bytes: 0,
    };
    let mut failure = None;
    for statement in &program.statements {
        if let Err(error) = evaluator.execute_statement(statement) {
            failure = Some(ScriptFailure::new(
                error.code(),
                error.message(),
                Some(error.line()),
                None,
            ));
            break;
        }
    }
    let stats = ScriptStats {
        variables: evaluator.environment.len(),
        ..evaluator.stats
    };
    Execution {
        prints: evaluator.prints,
        report: evaluator.report,
        stats,
        failure,
    }
}

struct Evaluator<'a> {
    limits: &'a ScriptLimits,
    clock: &'a dyn ScriptClock,
    host: Host,
    sandbox: Option<&'a Sandbox>,
    stored_trace: Option<Trace>,
    stored_trace_bytes: usize,
    snapshots: usize,
    environment: BTreeMap<String, Value>,
    prints: Vec<String>,
    report: BTreeMap<String, ReportValue>,
    stats: ScriptStats,
    loop_iterations: u64,
    string_bytes: usize,
}

impl Evaluator<'_> {
    fn execute_block(&mut self, block: &[Stmt]) -> Result<(), RunError> {
        for statement in block {
            self.execute_statement(statement)?;
        }
        Ok(())
    }

    fn execute_statement(&mut self, statement: &Stmt) -> Result<(), RunError> {
        let line = statement.line;
        self.check_wall_time(line)?;
        self.stats.statements = self.stats.statements.saturating_add(1);
        self.charge_operation(line)?;
        match &statement.kind {
            StmtKind::Let { name, value } => {
                let value = self.evaluate(value, line)?;
                self.declare(name, value, line)
            }
            StmtKind::Set { name, value } => {
                let value = self.evaluate(value, line)?;
                self.assign(name, value, line)
            }
            StmtKind::Print { value } => {
                let value = self.evaluate(value, line)?;
                self.push_print(&value.render(), line)
            }
            StmtKind::Assert { value } => {
                self.stats.assertions = self.stats.assertions.saturating_add(1);
                let value = self.evaluate(value, line)?;
                if !self.expect_bool(value, line)? {
                    return Err(RunError::eval(EvalErrorKind::AssertionFailed, line));
                }
                Ok(())
            }
            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                let value = self.evaluate(condition, line)?;
                if self.expect_bool(value, line)? {
                    self.execute_block(then_block)
                } else if let Some(else_block) = else_block {
                    self.execute_block(else_block)
                } else {
                    Ok(())
                }
            }
            StmtKind::Repeat { count, body } => {
                let value = self.evaluate(count, line)?;
                let count = self.expect_int(value, line)?;
                if count < 0 {
                    return Err(RunError::eval(EvalErrorKind::InvalidLoopCount(count), line));
                }
                let Ok(requested) = u64::try_from(count) else {
                    return Err(RunError::eval(EvalErrorKind::InvalidLoopCount(count), line));
                };
                let remaining = self
                    .limits
                    .loop_iterations
                    .saturating_sub(self.loop_iterations);
                if requested > remaining {
                    return Err(RunError::limit(
                        LimitKind::LoopIterations,
                        self.limits.loop_iterations,
                        line,
                    ));
                }
                for _ in 0..requested {
                    self.charge_loop_iteration(line)?;
                    self.execute_block(body)?;
                }
                Ok(())
            }
            StmtKind::While { condition, body } => loop {
                self.charge_loop_iteration(line)?;
                let value = self.evaluate(condition, line)?;
                if !self.expect_bool(value, line)? {
                    return Ok(());
                }
                self.execute_block(body)?;
            },
            StmtKind::Call(expression) => {
                self.evaluate(expression, line)?;
                Ok(())
            }
        }
    }

    fn evaluate(&mut self, expression: &Expr, line: usize) -> Result<Value, RunError> {
        self.charge_operation(line)?;
        match expression {
            Expr::Int(value) => Ok(Value::Int(*value)),
            Expr::Bool(value) => Ok(Value::Bool(*value)),
            Expr::Text(text) => Ok(Value::Text(text.clone())),
            Expr::Var(name) => {
                self.environment.get(name).cloned().ok_or_else(|| {
                    RunError::eval(EvalErrorKind::UnknownVariable(name.clone()), line)
                })
            }
            Expr::Unary { op, operand } => {
                let value = self.evaluate(operand, line)?;
                self.evaluate_unary(*op, value, line)
            }
            Expr::Binary { op, left, right } => self.evaluate_binary(*op, left, right, line),
            Expr::Call { name, args } => {
                let mut evaluated = Vec::with_capacity(args.len());
                for argument in args {
                    evaluated.push(self.evaluate(argument, line)?);
                }
                self.call_builtin(name, evaluated, line)
            }
        }
    }

    fn evaluate_unary(
        &self,
        operator: UnaryOp,
        value: Value,
        line: usize,
    ) -> Result<Value, RunError> {
        match operator {
            UnaryOp::Negate => {
                let value = self.expect_int(value, line)?;
                value
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or(RunError::eval(EvalErrorKind::ArithmeticOverflow, line))
            }
            UnaryOp::Not => {
                let value = self.expect_bool(value, line)?;
                Ok(Value::Bool(!value))
            }
        }
    }

    fn evaluate_binary(
        &mut self,
        operator: BinaryOp,
        left: &Expr,
        right: &Expr,
        line: usize,
    ) -> Result<Value, RunError> {
        match operator {
            BinaryOp::And => {
                let left_value = self.evaluate(left, line)?;
                if !self.expect_bool(left_value, line)? {
                    return Ok(Value::Bool(false));
                }
                let right_value = self.evaluate(right, line)?;
                Ok(Value::Bool(self.expect_bool(right_value, line)?))
            }
            BinaryOp::Or => {
                let left_value = self.evaluate(left, line)?;
                if self.expect_bool(left_value, line)? {
                    return Ok(Value::Bool(true));
                }
                let right_value = self.evaluate(right, line)?;
                Ok(Value::Bool(self.expect_bool(right_value, line)?))
            }
            _ => {
                let left_value = self.evaluate(left, line)?;
                let right_value = self.evaluate(right, line)?;
                let Some(operator) = StrictBinaryOp::from_binary(operator) else {
                    return Err(RunError::eval(EvalErrorKind::ArithmeticOverflow, line));
                };
                self.combine(operator, left_value, right_value, line)
            }
        }
    }

    fn combine(
        &self,
        operator: StrictBinaryOp,
        left: Value,
        right: Value,
        line: usize,
    ) -> Result<Value, RunError> {
        match operator {
            StrictBinaryOp::Add => match (left, right) {
                (Value::Int(left), Value::Int(right)) => left
                    .checked_add(right)
                    .map(Value::Int)
                    .ok_or(RunError::eval(EvalErrorKind::ArithmeticOverflow, line)),
                (Value::Text(left), Value::Text(right)) => {
                    let total = left.len().saturating_add(right.len());
                    if total > MAX_TEXT_BYTES {
                        return Err(RunError::eval(EvalErrorKind::TextTooLong, line));
                    }
                    let mut text = left;
                    text.push_str(&right);
                    Ok(Value::Text(text))
                }
                (left, right) => Err(binary_type_error("Int or Text", &left, &right, line)),
            },
            StrictBinaryOp::Sub => self.integer_combine(left, right, line, IntegerOp::Sub),
            StrictBinaryOp::Mul => self.integer_combine(left, right, line, IntegerOp::Mul),
            StrictBinaryOp::Div => self.checked_division(left, right, line, DivisionOp::Divide),
            StrictBinaryOp::Rem => self.checked_division(left, right, line, DivisionOp::Remainder),
            StrictBinaryOp::Eq | StrictBinaryOp::Ne => {
                if left.type_name() != right.type_name() {
                    return Err(binary_type_error(left.type_name(), &left, &right, line));
                }
                let equal = left == right;
                Ok(Value::Bool(if operator == StrictBinaryOp::Eq {
                    equal
                } else {
                    !equal
                }))
            }
            StrictBinaryOp::Lt => self.compare(left, right, line, CompareOp::Less),
            StrictBinaryOp::Le => self.compare(left, right, line, CompareOp::LessOrEqual),
            StrictBinaryOp::Gt => self.compare(left, right, line, CompareOp::Greater),
            StrictBinaryOp::Ge => self.compare(left, right, line, CompareOp::GreaterOrEqual),
        }
    }

    fn integer_combine(
        &self,
        left: Value,
        right: Value,
        line: usize,
        operation: IntegerOp,
    ) -> Result<Value, RunError> {
        let left = self.expect_int(left, line)?;
        let right = self.expect_int(right, line)?;
        let result = match operation {
            IntegerOp::Sub => left.checked_sub(right),
            IntegerOp::Mul => left.checked_mul(right),
        };
        result
            .map(Value::Int)
            .ok_or(RunError::eval(EvalErrorKind::ArithmeticOverflow, line))
    }

    fn checked_division(
        &self,
        left: Value,
        right: Value,
        line: usize,
        operation: DivisionOp,
    ) -> Result<Value, RunError> {
        let left = self.expect_int(left, line)?;
        let right = self.expect_int(right, line)?;
        if right == 0 {
            return Err(RunError::eval(EvalErrorKind::DivisionByZero, line));
        }
        let result = match operation {
            DivisionOp::Divide => left.checked_div(right),
            DivisionOp::Remainder => left.checked_rem(right),
        };
        result
            .map(Value::Int)
            .ok_or(RunError::eval(EvalErrorKind::ArithmeticOverflow, line))
    }

    fn compare(
        &self,
        left: Value,
        right: Value,
        line: usize,
        operation: CompareOp,
    ) -> Result<Value, RunError> {
        let result = match (&left, &right) {
            (Value::Int(left), Value::Int(right)) => compare_ints(*left, *right, operation),
            (Value::Text(left), Value::Text(right)) => compare_text(left, right, operation),
            (Value::Bool(_), Value::Bool(_)) => {
                return Err(RunError::eval(
                    EvalErrorKind::Type {
                        expected: "Int or Text",
                        found: "Bool",
                    },
                    line,
                ));
            }
            (left, right) => {
                return Err(binary_type_error(left.type_name(), left, right, line));
            }
        };
        Ok(Value::Bool(result))
    }

    fn expect_int(&self, value: Value, line: usize) -> Result<i64, RunError> {
        match value {
            Value::Int(value) => Ok(value),
            other => Err(RunError::eval(
                EvalErrorKind::Type {
                    expected: "Int",
                    found: other.type_name(),
                },
                line,
            )),
        }
    }

    fn expect_bool(&self, value: Value, line: usize) -> Result<bool, RunError> {
        match value {
            Value::Bool(value) => Ok(value),
            other => Err(RunError::eval(
                EvalErrorKind::Type {
                    expected: "Bool",
                    found: other.type_name(),
                },
                line,
            )),
        }
    }

    fn declare(&mut self, name: &str, value: Value, line: usize) -> Result<(), RunError> {
        if self.environment.contains_key(name) {
            return Err(RunError::eval(
                EvalErrorKind::DuplicateVariable(name.to_owned()),
                line,
            ));
        }
        if self.environment.len() >= self.limits.variables {
            return Err(RunError::limit(
                LimitKind::Variables,
                self.limits.variables as u64,
                line,
            ));
        }
        self.account_text_change(None, &value, line)?;
        self.environment.insert(name.to_owned(), value);
        Ok(())
    }

    fn assign(&mut self, name: &str, value: Value, line: usize) -> Result<(), RunError> {
        let Some(previous) = self.environment.get(name).cloned() else {
            return Err(RunError::eval(
                EvalErrorKind::UnknownVariable(name.to_owned()),
                line,
            ));
        };
        self.account_text_change(Some(&previous), &value, line)?;
        self.environment.insert(name.to_owned(), value);
        Ok(())
    }

    fn account_text_change(
        &mut self,
        previous: Option<&Value>,
        value: &Value,
        line: usize,
    ) -> Result<(), RunError> {
        let previous_len = previous.and_then(Value::text_len).unwrap_or(0);
        let next_len = value.text_len().unwrap_or(0);
        let total = self
            .string_bytes
            .saturating_sub(previous_len)
            .saturating_add(next_len);
        if total > self.limits.string_bytes {
            return Err(RunError::limit(
                LimitKind::StringBytes,
                self.limits.string_bytes as u64,
                line,
            ));
        }
        self.string_bytes = total;
        Ok(())
    }

    fn push_print(&mut self, text: &str, line: usize) -> Result<(), RunError> {
        let total = self.string_bytes.saturating_add(text.len());
        if total > self.limits.string_bytes {
            return Err(RunError::limit(
                LimitKind::StringBytes,
                self.limits.string_bytes as u64,
                line,
            ));
        }
        self.string_bytes = total;
        self.prints.push(text.to_owned());
        Ok(())
    }

    fn add_report(&mut self, key: &str, value: Value, line: usize) -> Result<(), RunError> {
        if !valid_report_key(key) {
            return Err(RunError::eval(
                EvalErrorKind::ReportKeyInvalid(key.to_owned()),
                line,
            ));
        }
        if self.report.contains_key(key) {
            return Err(RunError::eval(
                EvalErrorKind::ReportKeyDuplicate(key.to_owned()),
                line,
            ));
        }
        let rendered = value.render();
        let added = key.len().saturating_add(rendered.len());
        let report_total = self.stats.report_bytes.saturating_add(added);
        if report_total > self.limits.report_bytes {
            return Err(RunError::limit(
                LimitKind::ReportBytes,
                self.limits.report_bytes as u64,
                line,
            ));
        }
        let string_total = self.string_bytes.saturating_add(added);
        if string_total > self.limits.string_bytes {
            return Err(RunError::limit(
                LimitKind::StringBytes,
                self.limits.string_bytes as u64,
                line,
            ));
        }
        let report_value = match value {
            Value::Int(value) => ReportValue::Int(value),
            Value::Bool(value) => ReportValue::Bool(value),
            Value::Text(text) => ReportValue::Text(text),
        };
        self.report.insert(key.to_owned(), report_value);
        self.stats.report_bytes = report_total;
        self.string_bytes = string_total;
        Ok(())
    }

    fn call_builtin(
        &mut self,
        name: &str,
        args: Vec<Value>,
        line: usize,
    ) -> Result<Value, RunError> {
        match name {
            "open" => {
                expect_arity(name, args.len(), 0, line)?;
                let session = self
                    .host
                    .open()
                    .map_err(|kind| RunError::eval(kind, line))?;
                self.unsigned_to_int(session, line)
            }
            "step" => match args.as_slice() {
                [Value::Int(requested)] => self.step(*requested, line),
                [other] => Err(type_error("Int", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "tick" => {
                expect_arity(name, args.len(), 0, line)?;
                let tick = self
                    .host
                    .tick()
                    .map_err(|kind| RunError::eval(kind, line))?;
                self.unsigned_to_int(tick, line)
            }
            "state_hash" => {
                expect_arity(name, args.len(), 0, line)?;
                let hash = self
                    .host
                    .state_hash()
                    .map_err(|kind| RunError::eval(kind, line))?;
                Ok(Value::Text(hash))
            }
            "register" => match args.as_slice() {
                [Value::Text(register)] => {
                    let value = self
                        .host
                        .register(register)
                        .map_err(|kind| RunError::eval(kind, line))?;
                    self.unsigned_to_int(value, line)
                }
                [other] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "read" => match args.as_slice() {
                [Value::Int(address), Value::Int(length)] => self.read(*address, *length, line),
                [Value::Int(_), other] => Err(type_error("Int", other.type_name(), line)),
                [other, _] => Err(type_error("Int", other.type_name(), line)),
                _ => Err(arity_error(name, 2, args.len(), line)),
            },
            "report" => match args.as_slice() {
                [Value::Text(key), value] => {
                    let key = key.clone();
                    self.add_report(&key, value.clone(), line)?;
                    Ok(Value::Int(0))
                }
                [other, _] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 2, args.len(), line)),
            },
            "len" => match args.as_slice() {
                [Value::Text(text)] => {
                    let length = i64::try_from(text.len())
                        .map_err(|_| RunError::eval(EvalErrorKind::IntegerOutOfRange, line))?;
                    Ok(Value::Int(length))
                }
                [other] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "str" => match args.as_slice() {
                [value] => Ok(Value::Text(value.render())),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "fail" => match args.as_slice() {
                [Value::Text(message)] => {
                    Err(RunError::eval(EvalErrorKind::Fail(message.clone()), line))
                }
                [other] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "fork" => {
                expect_arity(name, args.len(), 0, line)?;
                self.reserve_snapshot(line)?;
                let child = self
                    .host
                    .fork()
                    .map_err(|kind| RunError::eval(kind, line))?;
                self.snapshots = self.snapshots.saturating_add(1);
                self.unsigned_to_int(child, line)
            }
            "checkpoint" => {
                expect_arity(name, args.len(), 0, line)?;
                self.reserve_snapshot(line)?;
                let snapshot = self
                    .host
                    .checkpoint()
                    .map_err(|kind| RunError::eval(kind, line))?;
                self.snapshots = self.snapshots.saturating_add(1);
                self.unsigned_to_int(snapshot, line)
            }
            "restore" => match args.as_slice() {
                [Value::Int(snapshot)] => {
                    let snapshot = validate_handle(*snapshot, line)?;
                    self.host
                        .restore(snapshot)
                        .map_err(|kind| RunError::eval(kind, line))?;
                    Ok(Value::Int(0))
                }
                [other] => Err(type_error("Int", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "select" => match args.as_slice() {
                [Value::Int(session)] => {
                    let session = validate_handle(*session, line)?;
                    self.host
                        .select(session)
                        .map_err(|kind| RunError::eval(kind, line))?;
                    Ok(Value::Int(0))
                }
                [other] => Err(type_error("Int", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "reset" => {
                expect_arity(name, args.len(), 0, line)?;
                self.host
                    .reset()
                    .map_err(|kind| RunError::eval(kind, line))?;
                Ok(Value::Int(0))
            }
            "trace_capture" => match args.as_slice() {
                [Value::Int(steps), Value::Int(capacity)] => {
                    self.trace_capture(*steps, *capacity, line)
                }
                [Value::Int(_), other] => Err(type_error("Int", other.type_name(), line)),
                [other, _] => Err(type_error("Int", other.type_name(), line)),
                _ => Err(arity_error(name, 2, args.len(), line)),
            },
            "trace_records" => {
                expect_arity(name, args.len(), 0, line)?;
                let records = i64::try_from(self.stats.trace_records)
                    .map_err(|_| RunError::eval(EvalErrorKind::IntegerOutOfRange, line))?;
                Ok(Value::Int(records))
            }
            "trace_export" => match args.as_slice() {
                [Value::Text(path)] => self.trace_export(path, line),
                [other] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "trace_compare" => match args.as_slice() {
                [Value::Text(path)] => self.trace_compare(path, line),
                [other] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "tas_verify" => match args.as_slice() {
                [Value::Text(path)] => self.tas_verify(path, line),
                [other] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 1, args.len(), line)),
            },
            "tas_compare" => match args.as_slice() {
                [Value::Text(left), Value::Text(right)] => self.tas_compare(left, right, line),
                [Value::Text(_), other] => Err(type_error("Text", other.type_name(), line)),
                [other, _] => Err(type_error("Text", other.type_name(), line)),
                _ => Err(arity_error(name, 2, args.len(), line)),
            },
            _ => Err(RunError::eval(
                EvalErrorKind::UnknownBuiltin(name.to_owned()),
                line,
            )),
        }
    }

    fn step(&mut self, requested: i64, line: usize) -> Result<Value, RunError> {
        if requested < 0 {
            return Err(RunError::eval(
                EvalErrorKind::InvalidStepCount(requested),
                line,
            ));
        }
        let Ok(requested) = u64::try_from(requested) else {
            return Err(RunError::eval(
                EvalErrorKind::InvalidStepCount(requested),
                line,
            ));
        };
        let remaining = self.limits.steps.saturating_sub(self.stats.steps);
        if requested > remaining {
            return Err(RunError::limit(LimitKind::Steps, self.limits.steps, line));
        }
        let mut tick = self
            .host
            .tick()
            .map_err(|kind| RunError::eval(kind, line))?;
        for _ in 0..requested {
            tick = self
                .host
                .step_once()
                .map_err(|kind| RunError::eval(kind, line))?;
            self.stats.steps = self.stats.steps.saturating_add(1);
        }
        self.unsigned_to_int(tick, line)
    }

    /// Reads `length` bytes and returns `2 * length` lowercase hex characters.
    ///
    /// This is the one runtime-produced `Text` value that may exceed the 4 KiB
    /// authored-text bound: ADR-0011 §3.5 bounds literals and concatenations,
    /// while a read is bounded by its `1..=4096` byte argument (at most 8192 hex
    /// characters). The result is charged to `string_bytes` whenever it becomes
    /// live variable, print or report text, like every other value.
    fn read(&mut self, address: i64, length: i64, line: usize) -> Result<Value, RunError> {
        if address < 0 {
            return Err(RunError::eval(EvalErrorKind::InvalidAddress(address), line));
        }
        if !(1..=4096).contains(&length) {
            return Err(RunError::eval(EvalErrorKind::InvalidLength(length), line));
        }
        let Ok(address) = u64::try_from(address) else {
            return Err(RunError::eval(EvalErrorKind::InvalidAddress(address), line));
        };
        let Ok(length) = usize::try_from(length) else {
            return Err(RunError::eval(EvalErrorKind::InvalidLength(length), line));
        };
        let bytes = self
            .host
            .read(address, length)
            .map_err(|kind| RunError::eval(kind, line))?;
        Ok(Value::Text(lowercase_hex(&bytes)))
    }

    fn reserve_snapshot(&self, line: usize) -> Result<(), RunError> {
        if self.snapshots >= self.limits.snapshots {
            return Err(RunError::limit(
                LimitKind::Snapshots,
                self.limits.snapshots as u64,
                line,
            ));
        }
        Ok(())
    }

    fn trace_capture(&mut self, steps: i64, capacity: i64, line: usize) -> Result<Value, RunError> {
        let steps = validate_trace_steps(steps, line)?;
        let capacity = validate_trace_capacity(capacity, line)?;
        let capacity_u64 = u64::try_from(capacity).unwrap_or(u64::MAX);
        if capacity_u64 > self.limits.trace_records {
            return Err(RunError::limit(
                LimitKind::TraceRecords,
                self.limits.trace_records,
                line,
            ));
        }
        let remaining_steps = self.limits.steps.saturating_sub(self.stats.steps);
        if steps > remaining_steps {
            return Err(RunError::limit(LimitKind::Steps, self.limits.steps, line));
        }
        let remaining_memory = self
            .limits
            .string_bytes
            .saturating_sub(self.string_bytes.saturating_sub(self.stored_trace_bytes));
        if trace_memory_estimate(capacity) > remaining_memory {
            return Err(RunError::limit(
                LimitKind::StringBytes,
                self.limits.string_bytes as u64,
                line,
            ));
        }
        let config = TraceConfig::new(capacity).map_err(|error| {
            RunError::eval(EvalErrorKind::TraceCapture(error.to_string()), line)
        })?;
        let trace = self
            .host
            .capture_trace(&config, steps)
            .map_err(|kind| RunError::eval(kind, line))?;
        let bytes = trace.to_bytes().len();
        let records = u64::try_from(trace.record_count()).unwrap_or(u64::MAX);
        if records > self.limits.trace_records {
            return Err(RunError::limit(
                LimitKind::TraceRecords,
                self.limits.trace_records,
                line,
            ));
        }
        let total = self
            .string_bytes
            .saturating_sub(self.stored_trace_bytes)
            .saturating_add(bytes);
        if total > self.limits.string_bytes {
            return Err(RunError::limit(
                LimitKind::StringBytes,
                self.limits.string_bytes as u64,
                line,
            ));
        }
        self.string_bytes = total;
        self.stored_trace_bytes = bytes;
        self.stored_trace = Some(trace);
        self.stats.trace_records = records;
        // Conservative reservation (ADR-0011 §3.5): the budget was checked
        // before executing and the whole requested count is charged even when
        // the backend halts early; no partial advance can exceed the budget.
        self.stats.steps = self.stats.steps.saturating_add(steps);
        self.unsigned_to_int(records, line)
    }

    fn trace_export(&self, path: &str, line: usize) -> Result<Value, RunError> {
        let Some(trace) = self.stored_trace.as_ref() else {
            return Err(RunError::eval(EvalErrorKind::NoStoredTrace, line));
        };
        let sandbox = self
            .sandbox
            .ok_or(RunError::eval(EvalErrorKind::SandboxUnavailable, line))?;
        let bytes = trace.to_bytes();
        sandbox
            .write(path, &bytes)
            .map_err(|error| sandbox_error(error, line))?;
        let length = i64::try_from(bytes.len())
            .map_err(|_| RunError::eval(EvalErrorKind::IntegerOutOfRange, line))?;
        Ok(Value::Int(length))
    }

    fn trace_compare(&self, path: &str, line: usize) -> Result<Value, RunError> {
        let Some(stored) = self.stored_trace.as_ref() else {
            return Err(RunError::eval(EvalErrorKind::NoStoredTrace, line));
        };
        let sandbox = self
            .sandbox
            .ok_or(RunError::eval(EvalErrorKind::SandboxUnavailable, line))?;
        let bytes = sandbox
            .read(path, MAX_TRACE_BYTES)
            .map_err(|error| sandbox_error(error, line))?;
        let other = Trace::from_bytes(&bytes)
            .map_err(|error| RunError::eval(EvalErrorKind::TraceParse(error.to_string()), line))?;
        match stored.compare(&other).divergence {
            Some(divergence) => {
                let index = i64::try_from(divergence.index)
                    .map_err(|_| RunError::eval(EvalErrorKind::IntegerOutOfRange, line))?;
                Ok(Value::Int(index))
            }
            None => Ok(Value::Int(-1)),
        }
    }

    fn tas_verify(&mut self, path: &str, line: usize) -> Result<Value, RunError> {
        let sandbox = self
            .sandbox
            .ok_or(RunError::eval(EvalErrorKind::SandboxUnavailable, line))?;
        let bytes = sandbox
            .read(path, MAX_MOVIE_BYTES)
            .map_err(|error| sandbox_error(error, line))?;
        let movie = Movie::from_bytes(&bytes)
            .map_err(|error| RunError::eval(EvalErrorKind::MovieDecode(error.to_string()), line))?;
        let steps = u64::try_from(movie.step_count())
            .map_err(|_| RunError::eval(EvalErrorKind::IntegerOutOfRange, line))?;
        let remaining_steps = self.limits.steps.saturating_sub(self.stats.steps);
        if steps > remaining_steps {
            return Err(RunError::limit(LimitKind::Steps, self.limits.steps, line));
        }
        let verified = self
            .host
            .verify_movie(&movie)
            .map_err(|kind| RunError::eval(kind, line))?;
        // Conservative reservation (ADR-0011 §3.5): the movie's recorded step
        // count was checked before replaying and is charged in full even when a
        // divergent movie stops the replay earlier.
        self.stats.steps = self.stats.steps.saturating_add(steps);
        Ok(Value::Bool(verified))
    }

    fn tas_compare(&self, left: &str, right: &str, line: usize) -> Result<Value, RunError> {
        let sandbox = self
            .sandbox
            .ok_or(RunError::eval(EvalErrorKind::SandboxUnavailable, line))?;
        let left_bytes = sandbox
            .read(left, MAX_MOVIE_BYTES)
            .map_err(|error| sandbox_error(error, line))?;
        let right_bytes = sandbox
            .read(right, MAX_MOVIE_BYTES)
            .map_err(|error| sandbox_error(error, line))?;
        let left_movie = Movie::from_bytes(&left_bytes)
            .map_err(|error| RunError::eval(EvalErrorKind::MovieDecode(error.to_string()), line))?;
        let right_movie = Movie::from_bytes(&right_bytes)
            .map_err(|error| RunError::eval(EvalErrorKind::MovieDecode(error.to_string()), line))?;
        let report = compare_movies(&left_movie, &right_movie);
        let tick = match report.verdict {
            ComparisonVerdict::Identical => return Ok(Value::Int(-1)),
            ComparisonVerdict::StepDivergence { tick_a, .. } => tick_a.raw(),
            ComparisonVerdict::LengthMismatch { common_steps } => {
                first_extra_tick(&left_movie, &right_movie, common_steps).ok_or_else(|| {
                    RunError::eval(
                        EvalErrorKind::MovieReplay(
                            "length mismatch without an extra recorded step".to_owned(),
                        ),
                        line,
                    )
                })?
            }
            ComparisonVerdict::CheckpointMismatch => {
                return Err(RunError::eval(EvalErrorKind::MovieCheckpointMismatch, line));
            }
        };
        let tick = i64::try_from(tick)
            .map_err(|_| RunError::eval(EvalErrorKind::IntegerOutOfRange, line))?;
        Ok(Value::Int(tick))
    }

    fn unsigned_to_int(&self, value: u64, line: usize) -> Result<Value, RunError> {
        i64::try_from(value)
            .map(Value::Int)
            .map_err(|_| RunError::eval(EvalErrorKind::IntegerOutOfRange, line))
    }

    fn charge_operation(&mut self, line: usize) -> Result<(), RunError> {
        if self.stats.operations >= self.limits.operations {
            return Err(RunError::limit(
                LimitKind::Operations,
                self.limits.operations,
                line,
            ));
        }
        self.stats.operations = self.stats.operations.saturating_add(1);
        if self.stats.operations % WALL_CHECK_INTERVAL_OPERATIONS == 0 {
            self.check_wall_time(line)?;
        }
        Ok(())
    }

    fn charge_loop_iteration(&mut self, line: usize) -> Result<(), RunError> {
        if self.loop_iterations >= self.limits.loop_iterations {
            return Err(RunError::limit(
                LimitKind::LoopIterations,
                self.limits.loop_iterations,
                line,
            ));
        }
        self.loop_iterations = self.loop_iterations.saturating_add(1);
        self.charge_operation(line)
    }

    fn check_wall_time(&self, line: usize) -> Result<(), RunError> {
        match self.limits.wall_time_ms {
            Some(deadline) if self.clock.now_millis() >= deadline => {
                Err(RunError::limit(LimitKind::WallTime, deadline, line))
            }
            _ => Ok(()),
        }
    }
}

fn validate_handle(value: i64, line: usize) -> Result<u64, RunError> {
    u64::try_from(value).map_err(|_| RunError::eval(EvalErrorKind::InvalidHandle(value), line))
}

fn validate_trace_steps(value: i64, line: usize) -> Result<u64, RunError> {
    let maximum = i64::try_from(MAX_CAPTURE_STEPS).unwrap_or(i64::MAX);
    if value < 1 || value > maximum {
        return Err(RunError::eval(
            EvalErrorKind::InvalidTraceSteps(value),
            line,
        ));
    }
    u64::try_from(value).map_err(|_| RunError::eval(EvalErrorKind::InvalidTraceSteps(value), line))
}

fn validate_trace_capacity(value: i64, line: usize) -> Result<usize, RunError> {
    let maximum = i64::try_from(MAX_TRACE_CAPACITY).unwrap_or(i64::MAX);
    if value < 1 || value > maximum {
        return Err(RunError::eval(
            EvalErrorKind::InvalidTraceCapacity(value),
            line,
        ));
    }
    usize::try_from(value)
        .map_err(|_| RunError::eval(EvalErrorKind::InvalidTraceCapacity(value), line))
}

fn sandbox_error(error: SandboxError, line: usize) -> RunError {
    let kind = match error.kind() {
        SandboxErrorKind::Root => EvalErrorKind::SandboxRoot,
        SandboxErrorKind::Path => EvalErrorKind::SandboxPath,
        SandboxErrorKind::Io => EvalErrorKind::SandboxIo,
        SandboxErrorKind::TooLarge => EvalErrorKind::SandboxTooLarge,
    };
    RunError::eval(kind, line)
}

fn first_extra_tick(left: &Movie, right: &Movie, index: usize) -> Option<u64> {
    left.steps()
        .get(index)
        .or_else(|| right.steps().get(index))
        .map(|step| step.outcome_tick().raw())
}

fn expect_arity(name: &str, found: usize, expected: usize, line: usize) -> Result<(), RunError> {
    if found == expected {
        Ok(())
    } else {
        Err(arity_error(name, expected, found, line))
    }
}

fn arity_error(name: &str, expected: usize, found: usize, line: usize) -> RunError {
    RunError::eval(
        EvalErrorKind::Arity {
            name: name.to_owned(),
            expected,
            found,
        },
        line,
    )
}

fn type_error(expected: &'static str, found: &'static str, line: usize) -> RunError {
    RunError::eval(EvalErrorKind::Type { expected, found }, line)
}

fn binary_type_error(expected: &'static str, left: &Value, right: &Value, line: usize) -> RunError {
    if left.type_name() == right.type_name() {
        RunError::eval(
            EvalErrorKind::Type {
                expected,
                found: left.type_name(),
            },
            line,
        )
    } else {
        RunError::eval(
            EvalErrorKind::Type {
                expected: left.type_name(),
                found: right.type_name(),
            },
            line,
        )
    }
}

fn compare_ints(left: i64, right: i64, operation: CompareOp) -> bool {
    match operation {
        CompareOp::Less => left < right,
        CompareOp::LessOrEqual => left <= right,
        CompareOp::Greater => left > right,
        CompareOp::GreaterOrEqual => left >= right,
    }
}

fn compare_text(left: &str, right: &str, operation: CompareOp) -> bool {
    match operation {
        CompareOp::Less => left < right,
        CompareOp::LessOrEqual => left <= right,
        CompareOp::Greater => left > right,
        CompareOp::GreaterOrEqual => left >= right,
    }
}

fn valid_report_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 64 {
        return false;
    }
    let mut bytes = key.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
