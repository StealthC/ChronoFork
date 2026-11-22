//! The typed operation catalog, its JSON-Schema subset and input validation.
//!
//! One declarative table is the single source of truth for the catalog order, the
//! mutating flag, the minimum scope, the published `inputSchema` and the runtime
//! argument validation, so a published schema and an accepted request cannot drift
//! apart. See architecture §3.

use std::collections::BTreeMap;

use crate::auth::Scope;
use crate::error::{ErrorCode, ServiceError};
use crate::json::{JsonValue, MAX_INTEGER};
use crate::limits::ServiceLimits;
use crate::tool::ToolSpec;

/// Maximum number of UTF-8 bytes in an advisory session or snapshot name.
pub(crate) const MAX_NAME_BYTES: usize = 256;

/// Largest `buttons` value the wire profile accepts.
const MAX_BUTTONS: u64 = 4_294_967_295;

/// The accepted shape of one input field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FieldKind {
    /// Required key: an integer at or above one.
    Key,
    /// Optional key: an integer at or above one, or `null`; absent means `null`.
    OptionalKey,
    /// Required step count in `min..=max_steps_per_request`.
    Steps {
        /// Smallest accepted step count.
        min: u64,
    },
    /// Optional step count in `0..=max_steps_per_request`; absent means zero.
    OptionalSteps,
    /// Optional input mask in `0..=4294967295`; absent means neutral input.
    OptionalButtons,
    /// Required address: an integer at or above zero.
    Address,
    /// Optional read length in `0..=max_memory_read`; absent means zero bytes.
    MemoryLength,
    /// Optional advisory name of `min_bytes..=256` UTF-8 bytes without control
    /// characters, or `null`.
    Name {
        /// Smallest accepted length in bytes.
        min_bytes: usize,
    },
    /// Optional strictly increasing experiment checkpoint list.
    Checkpoints,
    /// Optional operation identifier used for reconciliation (ADR-0016 §2).
    ///
    /// The key is known to the client **before** the mutation is sent, which is what
    /// makes a retry after a lost response safe: the server replays the retained
    /// result instead of re-applying the operation.
    OperationId,
    /// Optional per-request host wall-clock budget in `1..=max_run_wall_ms`.
    MaxWallMs,
    /// Optional process epoch a request key was minted under (ADR-0016 §2).
    ///
    /// A keyed mutation of a durable service must carry it, and a control query may use
    /// it to refuse a result produced by another process. An absent value is the current
    /// process (the in-process form) unless the service is durable.
    Epoch,
    /// Optional monotonic revision precondition.
    ExpectedRevision,
    /// Optional absolute tick precondition.
    ExpectedTick,
}

impl FieldKind {
    fn required(self) -> bool {
        matches!(self, Self::Key | Self::Steps { .. } | Self::Address)
    }

    fn schema(self, limits: &ServiceLimits) -> JsonValue {
        match self {
            Self::Key => limited_integer("integer", 1, MAX_INTEGER),
            Self::OptionalKey => nullable_integer(1, MAX_INTEGER),
            Self::Steps { min } => {
                limited_integer("integer", bound(min), bound(limits.max_steps_per_request))
            }
            Self::OptionalSteps => {
                limited_integer("integer", 0, bound(limits.max_steps_per_request))
            }
            Self::OptionalButtons => limited_integer("integer", 0, bound(MAX_BUTTONS)),
            Self::Address => limited_integer("integer", 0, MAX_INTEGER),
            Self::MemoryLength => limited_integer("integer", 0, bound(limits.max_memory_read)),
            Self::Name { min_bytes } => JsonValue::object([
                ("type", nullable_string_type()),
                ("minLength", JsonValue::int(bound(min_bytes))),
                ("maxLength", JsonValue::int(bound(MAX_NAME_BYTES))),
                // No C0 control character may appear in an advisory name.
                ("pattern", JsonValue::string("^[^\\u0000-\\u001F]*$")),
            ]),
            Self::Checkpoints => JsonValue::object([
                ("type", JsonValue::string("array")),
                ("items", limited_integer("integer", 0, MAX_INTEGER)),
                ("maxItems", JsonValue::int(bound(limits.max_checkpoints))),
            ]),
            Self::OperationId => limited_integer("integer", 1, MAX_INTEGER),
            Self::MaxWallMs => limited_integer("integer", 1, bound(limits.max_run_wall_ms)),
            Self::Epoch => limited_integer("integer", 0, MAX_INTEGER),
            Self::ExpectedRevision => limited_integer("integer", 0, MAX_INTEGER),
            Self::ExpectedTick => limited_integer("integer", 0, MAX_INTEGER),
        }
    }

    fn validate(
        self,
        name: &'static str,
        value: Option<&JsonValue>,
        limits: &ServiceLimits,
        args: &mut Args,
    ) -> Result<(), ServiceError> {
        let Some(value) = value else {
            if self.required() {
                return Err(invalid_owned(format!("missing required field `{name}`")));
            }
            return Ok(());
        };
        match self {
            Self::Key => {
                let number = at_least(name, value, 1)?;
                args.integers.insert(name, number);
            }
            Self::OptionalKey => {
                if matches!(value, JsonValue::Null) {
                    return Ok(());
                }
                let number = at_least(name, value, 1)?;
                args.integers.insert(name, number);
            }
            Self::Steps { min } => {
                let number = at_least(name, value, bound(min))?;
                at_most(name, number, bound(limits.max_steps_per_request))?;
                args.integers.insert(name, number);
            }
            Self::OptionalSteps => {
                let number = at_least(name, value, 0)?;
                at_most(name, number, bound(limits.max_steps_per_request))?;
                args.integers.insert(name, number);
            }
            Self::OptionalButtons => {
                let number = at_least(name, value, 0)?;
                at_most(name, number, bound(MAX_BUTTONS))?;
                args.integers.insert(name, number);
            }
            Self::Address => {
                let number = at_least(name, value, 0)?;
                args.integers.insert(name, number);
            }
            Self::MemoryLength => {
                let number = at_least(name, value, 0)?;
                at_most(name, number, bound(limits.max_memory_read))?;
                args.integers.insert(name, number);
            }
            Self::Name { min_bytes } => match value {
                JsonValue::Null => {
                    args.names.insert(name, None);
                }
                JsonValue::String(text) => {
                    if text.len() < min_bytes {
                        return Err(invalid_owned(format!(
                            "`{name}` is shorter than {min_bytes} bytes"
                        )));
                    }
                    if text.len() > MAX_NAME_BYTES {
                        return Err(invalid_owned(format!(
                            "`{name}` is longer than {MAX_NAME_BYTES} bytes"
                        )));
                    }
                    if text.chars().any(char::is_control) {
                        return Err(invalid_owned(format!(
                            "`{name}` must not contain control characters"
                        )));
                    }
                    args.names.insert(name, Some(text.clone()));
                }
                _ => {
                    return Err(invalid_owned(format!("`{name}` must be a string or null")));
                }
            },
            Self::Checkpoints => {
                let values = value
                    .as_array()
                    .ok_or_else(|| invalid_owned(format!("`{name}` must be an array")))?;
                if values.len() > limits.max_checkpoints {
                    return Err(ServiceError::new(
                        ErrorCode::LimitExceeded,
                        format!("`{name}` exceeds the configured checkpoint limit"),
                    ));
                }
                let mut checkpoints = Vec::with_capacity(values.len());
                for item in values {
                    let number = at_least(name, item, 0)?;
                    checkpoints.push(
                        u64::try_from(number).map_err(|_| {
                            ServiceError::internal("validated checkpoint is negative")
                        })?,
                    );
                }
                args.checkpoints = checkpoints;
            }
            Self::OperationId => {
                if matches!(value, JsonValue::Null) {
                    return Ok(());
                }
                let number = at_least(name, value, 1)?;
                args.operations.insert(
                    name,
                    u64::try_from(number).map_err(|_| {
                        ServiceError::internal("validated operation id is negative")
                    })?,
                );
            }
            Self::MaxWallMs => {
                if matches!(value, JsonValue::Null) {
                    return Ok(());
                }
                let number = at_least(name, value, 1)?;
                at_most(name, number, bound(limits.max_run_wall_ms))?;
                args.integers.insert(name, number);
            }
            Self::Epoch | Self::ExpectedRevision | Self::ExpectedTick => {
                if matches!(value, JsonValue::Null) {
                    return Ok(());
                }
                let number = at_least(name, value, 0)?;
                args.integers.insert(name, number);
            }
        }
        Ok(())
    }
}

/// Validated arguments keyed by the catalog field name.
#[derive(Debug, Default)]
pub(crate) struct Args {
    integers: BTreeMap<&'static str, i64>,
    names: BTreeMap<&'static str, Option<String>>,
    operations: BTreeMap<&'static str, u64>,
    checkpoints: Vec<u64>,
}

impl Args {
    /// Returns a validated non-negative integer.
    pub(crate) fn unsigned(&self, name: &str) -> Result<u64, ServiceError> {
        let value = self
            .integers
            .get(name)
            .copied()
            .ok_or_else(|| ServiceError::internal("validated argument is missing"))?;
        u64::try_from(value).map_err(|_| ServiceError::internal("validated argument is negative"))
    }

    /// Returns a validated optional non-negative integer.
    pub(crate) fn optional_unsigned(&self, name: &str) -> Result<Option<u64>, ServiceError> {
        match self.integers.get(name) {
            None => Ok(None),
            Some(_) => self.unsigned(name).map(Some),
        }
    }

    /// Returns a validated non-negative integer, or zero when it was omitted.
    pub(crate) fn unsigned_or_zero(&self, name: &str) -> Result<u64, ServiceError> {
        match self.integers.get(name) {
            None => Ok(0),
            Some(_) => self.unsigned(name),
        }
    }

    /// Returns the validated button mask.
    pub(crate) fn buttons(&self) -> Result<u32, ServiceError> {
        let value = self.unsigned_or_zero("buttons")?;
        u32::try_from(value)
            .map_err(|_| ServiceError::internal("validated button mask is out of range"))
    }

    /// Returns the validated advisory name, when the request supplied one.
    pub(crate) fn name(&self, name: &str) -> Option<&str> {
        self.names.get(name).and_then(|value| value.as_deref())
    }

    /// Returns the validated operation key, when the request supplied one.
    ///
    /// A request key is a client-chosen positive integer, so it is bounded by the
    /// JSON integer rules and never carries caller-controlled text.
    pub(crate) fn operation(&self, name: &str) -> Option<u64> {
        self.operations.get(name).copied()
    }

    /// Returns the validated checkpoint list, empty when it was omitted.
    pub(crate) fn checkpoints(&self) -> &[u64] {
        &self.checkpoints
    }
}

/// Implementation selector for one catalog entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpKind {
    /// `capabilities`
    Capabilities,
    /// `session.create`
    Create,
    /// `session.list`
    List,
    /// `session.inspect`
    Inspect,
    /// `session.run`
    Run,
    /// `session.fork`
    Fork,
    /// `session.snapshot`
    Snapshot,
    /// `session.restore`
    Restore,
    /// `session.compare`
    Compare,
    /// `session.close`
    Close,
    /// `session.reap` (ADR-0016 §3 lifecycle)
    Reap,
    /// `session.read_memory`
    ReadMemory,
    /// `session.cancel` (ADR-0016 §2)
    Cancel,
    /// `session.progress` (ADR-0016 §2)
    Progress,
    /// `session.operation` (ADR-0016 §2 reconciliation horizon)
    Operation,
    /// `store.list`
    StoreList,
    /// `store.load`
    StoreLoad,
    /// `store.save`
    StoreSave,
    /// `store.delete`
    StoreDelete,
    /// `experiment.run`
    ExperimentRun,
}

/// One machine-callable operation definition.
pub(crate) struct OpSpec {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) mutating: bool,
    pub(crate) scope: Scope,
    pub(crate) kind: OpKind,
    pub(crate) fields: &'static [FieldSpec],
}

/// One accepted input field.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FieldSpec {
    pub(crate) name: &'static str,
    pub(crate) kind: FieldKind,
}

const fn field(name: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec { name, kind }
}

/// Every operation, in architecture §3 catalog order.
pub(crate) const CATALOG: &[OpSpec] = &[
    OpSpec {
        name: "capabilities",
        description: "Report service, backend, catalog and limit capabilities",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::Capabilities,
        fields: &[],
    },
    OpSpec {
        name: "session.create",
        description: "Create an in-memory session",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Create,
        fields: &[
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "session.list",
        description: "List the sessions visible to the caller",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::List,
        fields: &[],
    },
    OpSpec {
        name: "session.inspect",
        description: "Inspect one session state and machine",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::Inspect,
        fields: &[field("session", FieldKind::Key)],
    },
    OpSpec {
        name: "session.run",
        description: "Advance a session by steps with one input frame",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Run,
        fields: &[
            field("session", FieldKind::Key),
            field("steps", FieldKind::Steps { min: 1 }),
            field("buttons", FieldKind::OptionalButtons),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
            field("max_wall_ms", FieldKind::MaxWallMs),
            field("expected_revision", FieldKind::ExpectedRevision),
            field("expected_tick", FieldKind::ExpectedTick),
        ],
    },
    OpSpec {
        name: "session.fork",
        description: "Fork a session at its current state",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Fork,
        fields: &[
            field("session", FieldKind::Key),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "session.snapshot",
        description: "Capture and index a checkpoint",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Snapshot,
        fields: &[
            field("session", FieldKind::Key),
            field("name", FieldKind::Name { min_bytes: 0 }),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "session.restore",
        description: "Restore an indexed checkpoint",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Restore,
        fields: &[
            field("session", FieldKind::Key),
            field("snapshot", FieldKind::Key),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
            field("expected_revision", FieldKind::ExpectedRevision),
        ],
    },
    OpSpec {
        name: "session.compare",
        description: "Compare two sessions by tick and state hash",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::Compare,
        fields: &[
            field("left", FieldKind::Key),
            field("right", FieldKind::Key),
        ],
    },
    OpSpec {
        name: "session.close",
        description: "Close a session",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Close,
        fields: &[
            field("session", FieldKind::Key),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "session.reap",
        description: "Remove closed session entries and release their memory",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Reap,
        fields: &[
            field("session", FieldKind::OptionalKey),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "session.read_memory",
        description: "Read a bounded window of emulated memory",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::ReadMemory,
        fields: &[
            field("session", FieldKind::Key),
            field("address", FieldKind::Address),
            field("length", FieldKind::MemoryLength),
        ],
    },
    OpSpec {
        name: "session.cancel",
        description: "Request cancellation of a running operation (control path)",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::Cancel,
        fields: &[
            field("operation", FieldKind::OperationId),
            field("session", FieldKind::OptionalKey),
        ],
    },
    OpSpec {
        name: "session.progress",
        description: "Read the operation journal entry of one operation (control path)",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::Progress,
        fields: &[
            field("operation", FieldKind::OperationId),
            field("epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "session.operation",
        description: "Read a retained operation result inside the reconciliation horizon",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::Operation,
        fields: &[
            field("operation", FieldKind::OperationId),
            field("epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "store.list",
        description: "List durable sessions and their snapshots",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::StoreList,
        fields: &[field("session_key", FieldKind::OptionalKey)],
    },
    OpSpec {
        name: "store.load",
        description: "Rehydrate one durable checkpoint and report it",
        mutating: false,
        scope: Scope::Read,
        kind: OpKind::StoreLoad,
        fields: &[
            field("session_key", FieldKind::Key),
            field("snapshot_key", FieldKind::OptionalKey),
            field("steps", FieldKind::OptionalSteps),
            field("buttons", FieldKind::OptionalButtons),
        ],
    },
    OpSpec {
        name: "store.save",
        description: "Create or continue one durable session",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::StoreSave,
        fields: &[
            field("session_key", FieldKind::OptionalKey),
            field("steps", FieldKind::OptionalSteps),
            field("buttons", FieldKind::OptionalButtons),
            field("name", FieldKind::Name { min_bytes: 1 }),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "store.delete",
        description: "Delete one durable session or snapshot",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::StoreDelete,
        fields: &[
            field("session_key", FieldKind::Key),
            field("snapshot_key", FieldKind::OptionalKey),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
    OpSpec {
        name: "experiment.run",
        description: "Produce a content-addressed experiment artifact",
        mutating: true,
        scope: Scope::Write,
        kind: OpKind::ExperimentRun,
        fields: &[
            field("steps", FieldKind::Steps { min: 1 }),
            field("buttons", FieldKind::OptionalButtons),
            field("checkpoints", FieldKind::Checkpoints),
            field("operation_id", FieldKind::OperationId),
            field("operation_epoch", FieldKind::Epoch),
        ],
    },
];

/// Looks one operation up by its stable name.
pub(crate) fn find(name: &str) -> Option<&'static OpSpec> {
    CATALOG.iter().find(|spec| spec.name == name)
}

/// Returns the published catalog with one input schema per operation.
pub(crate) fn tools(limits: &ServiceLimits) -> Vec<ToolSpec> {
    CATALOG
        .iter()
        .map(|spec| ToolSpec {
            name: spec.name,
            description: spec.description,
            mutating: spec.mutating,
            scope: spec.scope,
            input_schema: schema(spec, limits),
        })
        .collect()
}

/// Validates one request input against the catalog definition of its operation.
pub(crate) fn validate(
    spec: &OpSpec,
    input: &JsonValue,
    limits: &ServiceLimits,
) -> Result<Args, ServiceError> {
    let object = input
        .as_object()
        .ok_or_else(|| invalid_owned("`input` must be a JSON object".to_owned()))?;
    for key in object.keys() {
        if !spec.fields.iter().any(|field| field.name == key.as_str()) {
            // The client-supplied key is never echoed: its length is client
            // controlled and would make the error envelope unbounded.
            return Err(invalid_owned("unknown field in `input`".to_owned()));
        }
    }
    let mut args = Args::default();
    for field in spec.fields {
        field
            .kind
            .validate(field.name, object.get(field.name), limits, &mut args)?;
    }
    Ok(args)
}

fn schema(spec: &OpSpec, limits: &ServiceLimits) -> JsonValue {
    let mut properties = BTreeMap::new();
    let mut required = Vec::new();
    for field in spec.fields {
        properties.insert(field.name.to_owned(), field.kind.schema(limits));
        if field.kind.required() {
            required.push(JsonValue::string(field.name));
        }
    }
    JsonValue::object([
        ("type", JsonValue::string("object")),
        ("properties", JsonValue::Object(properties)),
        ("required", JsonValue::Array(required)),
        ("additionalProperties", JsonValue::from_bool(false)),
    ])
}

fn limited_integer(kind: &'static str, minimum: i64, maximum: i64) -> JsonValue {
    JsonValue::object([
        ("type", JsonValue::string(kind)),
        ("minimum", JsonValue::int(minimum)),
        ("maximum", JsonValue::int(maximum)),
    ])
}

fn nullable_integer(minimum: i64, maximum: i64) -> JsonValue {
    JsonValue::object([
        ("type", nullable_type()),
        ("minimum", JsonValue::int(minimum)),
        ("maximum", JsonValue::int(maximum)),
    ])
}

fn nullable_type() -> JsonValue {
    JsonValue::array([JsonValue::string("integer"), JsonValue::string("null")])
}

fn nullable_string_type() -> JsonValue {
    JsonValue::array([JsonValue::string("string"), JsonValue::string("null")])
}

fn bound(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

fn integer(name: &'static str, value: &JsonValue) -> Result<i64, ServiceError> {
    value
        .as_i64()
        .ok_or_else(|| invalid_owned(format!("`{name}` must be an integer")))
}

fn at_least(name: &'static str, value: &JsonValue, minimum: i64) -> Result<i64, ServiceError> {
    let number = integer(name, value)?;
    if number < minimum {
        return Err(invalid_owned(format!(
            "`{name}` must be at least {minimum}"
        )));
    }
    Ok(number)
}

fn at_most(name: &'static str, number: i64, maximum: i64) -> Result<(), ServiceError> {
    if number > maximum {
        return Err(ServiceError::new(
            ErrorCode::LimitExceeded,
            format!("`{name}` must be at most {maximum}"),
        ));
    }
    Ok(())
}

fn invalid_owned(message: String) -> ServiceError {
    ServiceError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::{CATALOG, FieldKind, MAX_NAME_BYTES, find, tools, validate};
    use crate::auth::Scope;
    use crate::error::ErrorCode;
    use crate::json::{JsonValue, parse};
    use crate::limits::ServiceLimits;

    fn limits() -> ServiceLimits {
        ServiceLimits::default()
    }

    fn input(text: &str) -> JsonValue {
        parse(text.as_bytes(), &limits()).expect("valid JSON input")
    }

    fn validate_op(op: &str, text: &str) -> Result<super::Args, crate::error::ServiceError> {
        let spec = find(op).expect("catalog entry");
        validate(spec, &input(text), &limits())
    }

    fn code(op: &str, text: &str) -> ErrorCode {
        validate_op(op, text)
            .expect_err("input must be rejected")
            .code()
    }

    #[test]
    fn the_catalog_is_exactly_the_twenty_documented_operations_in_order() {
        let expected = [
            ("capabilities", false, Scope::Read),
            ("session.create", true, Scope::Write),
            ("session.list", false, Scope::Read),
            ("session.inspect", false, Scope::Read),
            ("session.run", true, Scope::Write),
            ("session.fork", true, Scope::Write),
            ("session.snapshot", true, Scope::Write),
            ("session.restore", true, Scope::Write),
            ("session.compare", false, Scope::Read),
            ("session.close", true, Scope::Write),
            ("session.reap", true, Scope::Write),
            ("session.read_memory", false, Scope::Read),
            ("session.cancel", true, Scope::Write),
            ("session.progress", false, Scope::Read),
            ("session.operation", false, Scope::Read),
            ("store.list", false, Scope::Read),
            ("store.load", false, Scope::Read),
            ("store.save", true, Scope::Write),
            ("store.delete", true, Scope::Write),
            ("experiment.run", true, Scope::Write),
        ];
        assert_eq!(CATALOG.len(), expected.len());
        for (spec, (name, mutating, scope)) in CATALOG.iter().zip(expected) {
            assert_eq!(spec.name, name);
            assert_eq!(spec.mutating, mutating, "{name}");
            assert_eq!(spec.scope, scope, "{name}");
            assert!(!spec.description.is_empty(), "{name}");
        }
        assert_eq!(tools(&limits()).len(), expected.len());
    }

    #[test]
    fn every_published_schema_is_a_bounded_object() {
        for spec in CATALOG {
            let schema = tools(&limits())
                .into_iter()
                .find(|tool| tool.name == spec.name)
                .expect("tool")
                .input_schema;
            assert_eq!(
                schema.get("type").and_then(JsonValue::as_str),
                Some("object")
            );
            assert_eq!(
                schema
                    .get("additionalProperties")
                    .and_then(JsonValue::as_bool),
                Some(false)
            );
            let properties = schema
                .get("properties")
                .and_then(JsonValue::as_object)
                .expect("properties");
            assert_eq!(properties.len(), spec.fields.len(), "{}", spec.name);
            let required = schema
                .get("required")
                .and_then(JsonValue::as_array)
                .expect("required list");
            let expected: Vec<&str> = spec
                .fields
                .iter()
                .filter(|field| field.kind.required())
                .map(|field| field.name)
                .collect();
            assert_eq!(
                required
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn the_field_shapes_are_the_documented_ones() {
        let tools = tools(&limits());
        let run = tools
            .iter()
            .find(|tool| tool.name == "session.run")
            .expect("run");
        let properties = run
            .input_schema
            .get("properties")
            .and_then(JsonValue::as_object)
            .expect("properties");
        assert_eq!(
            properties
                .get("steps")
                .and_then(|value| value.get("minimum"))
                .and_then(JsonValue::as_i64),
            Some(1)
        );

        let store_save = tools
            .iter()
            .find(|tool| tool.name == "store.save")
            .expect("save");
        let properties = store_save
            .input_schema
            .get("properties")
            .and_then(JsonValue::as_object)
            .expect("properties");
        assert_eq!(
            properties
                .get("session_key")
                .and_then(|value| value.get("minimum"))
                .and_then(JsonValue::as_i64),
            Some(1)
        );
        assert_eq!(
            properties
                .get("name")
                .and_then(|value| value.get("maxLength"))
                .and_then(JsonValue::as_i64),
            Some(i64::try_from(MAX_NAME_BYTES).expect("name bound"))
        );
    }

    #[test]
    fn valid_inputs_validate_with_defaults() {
        let args = validate_op("session.run", "{\"session\":1,\"steps\":10}").expect("run");
        assert_eq!(args.unsigned("steps").expect("steps"), 10);
        assert_eq!(args.buttons().expect("buttons"), 0);

        let args = validate_op("session.run", "{\"session\":1,\"steps\":10,\"buttons\":7}")
            .expect("run with buttons");
        assert_eq!(args.buttons().expect("buttons"), 7);

        let args = validate_op("store.list", "{}").expect("store list");
        assert_eq!(args.optional_unsigned("session_key").expect("key"), None);
        let args = validate_op("store.list", "{\"session_key\":null}").expect("store list null");
        assert_eq!(args.optional_unsigned("session_key").expect("key"), None);
        let args = validate_op("store.list", "{\"session_key\":3}").expect("store list key");
        assert_eq!(args.optional_unsigned("session_key").expect("key"), Some(3));

        let args = validate_op("session.snapshot", "{\"session\":1,\"name\":\"check\"}")
            .expect("snapshot name");
        assert_eq!(args.name("name"), Some("check"));
        let args = validate_op("session.snapshot", "{\"session\":1,\"name\":null}")
            .expect("snapshot null name");
        assert_eq!(args.name("name"), None);

        let args = validate_op("experiment.run", "{\"steps\":4,\"checkpoints\":[0,2,4]}")
            .expect("experiment");
        assert_eq!(args.checkpoints(), &[0, 2, 4]);
        let args = validate_op("experiment.run", "{\"steps\":4}").expect("experiment default");
        assert!(args.checkpoints().is_empty());
    }

    #[test]
    fn unknown_and_mistyped_fields_are_typed() {
        assert_eq!(code("capabilities", "{\"x\":1}"), ErrorCode::InvalidRequest);
        assert_eq!(
            code("session.inspect", "{\"session\":1,\"other\":1}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("session.inspect", "{\"session\":\"1\"}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(code("session.inspect", "{}"), ErrorCode::InvalidRequest);
        assert_eq!(
            code("session.inspect", "{\"session\":0}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("session.run", "{\"session\":1}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("session.run", "{\"session\":1,\"steps\":0}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("session.run", "{\"session\":1,\"steps\":100001}"),
            ErrorCode::LimitExceeded
        );
        assert_eq!(
            code(
                "session.run",
                "{\"session\":1,\"steps\":1,\"buttons\":4294967296}"
            ),
            ErrorCode::LimitExceeded
        );
        assert_eq!(
            code(
                "session.read_memory",
                "{\"session\":1,\"address\":0,\"length\":4097}"
            ),
            ErrorCode::LimitExceeded
        );
        assert_eq!(
            code("session.snapshot", "{\"session\":1,\"name\":3}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("session.snapshot", "{\"session\":1,\"name\":\"a\\u0001b\"}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("experiment.run", "{\"steps\":1,\"checkpoints\":\"x\"}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("experiment.run", "{\"steps\":1,\"checkpoints\":[-1]}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("store.save", "{\"session_key\":0}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("store.save", "{\"name\":\"\"}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            code("store.save", "{\"steps\":100001}"),
            ErrorCode::LimitExceeded
        );
        assert_eq!(code("session.inspect", "[]"), ErrorCode::InvalidRequest);
        assert_eq!(code("store.delete", "{}"), ErrorCode::InvalidRequest);
        assert_eq!(
            code(
                "session.read_memory",
                "{\"session\":1,\"address\":-1,\"length\":1}"
            ),
            ErrorCode::InvalidRequest
        );
        let long_name = "n".repeat(MAX_NAME_BYTES + 1);
        assert_eq!(
            code(
                "session.snapshot",
                &format!("{{\"session\":1,\"name\":\"{long_name}\"}}")
            ),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn checkpoint_lists_are_bounded_before_work_starts() {
        let limits = ServiceLimits {
            max_checkpoints: 2,
            ..ServiceLimits::default()
        };
        let spec = find("experiment.run").expect("experiment");
        let input = input("{\"steps\":4,\"checkpoints\":[0,1,2]}");
        assert_eq!(
            validate(spec, &input, &limits)
                .expect_err("too many checkpoints")
                .code(),
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn field_kinds_are_classified_once() {
        for kind in [
            FieldKind::Key,
            FieldKind::Steps { min: 1 },
            FieldKind::Address,
        ] {
            assert!(kind.required());
        }
        for kind in [
            FieldKind::OptionalKey,
            FieldKind::OptionalSteps,
            FieldKind::OptionalButtons,
            FieldKind::MemoryLength,
            FieldKind::Name { min_bytes: 0 },
            FieldKind::Checkpoints,
            FieldKind::OperationId,
            FieldKind::MaxWallMs,
            FieldKind::ExpectedRevision,
            FieldKind::ExpectedTick,
        ] {
            assert!(!kind.required());
        }
        // The control path is the only path served without a session owner
        // (ADR-0016 §1, K2/F3); the catalog test below pins its field name.
        assert!(!FieldKind::OperationId.required());
    }

    #[test]
    fn reconciliation_and_deadline_fields_are_bounded_before_work_starts() {
        let limits = ServiceLimits {
            max_run_wall_ms: 1_000,
            ..ServiceLimits::default()
        };
        let spec = find("session.run").expect("run");

        let value = input("{\"session\":1,\"steps\":1,\"operation_id\":7,\"max_wall_ms\":1000}");
        let args = validate(spec, &value, &limits).expect("bounded wall budget");
        assert_eq!(args.operation("operation_id"), Some(7));
        assert_eq!(
            args.optional_unsigned("max_wall_ms").expect("wall"),
            Some(1000)
        );

        // A caller may lower a limit inside its own scope but never raise it past the
        // daemon cap, and the rejection happens before any step runs.
        let over = input("{\"session\":1,\"steps\":1,\"max_wall_ms\":1001}");
        assert_eq!(
            validate(spec, &over, &limits).expect_err("over cap").code(),
            ErrorCode::LimitExceeded
        );
        let zero = input("{\"session\":1,\"steps\":1,\"max_wall_ms\":0}");
        assert_eq!(
            validate(spec, &zero, &limits).expect_err("zero").code(),
            ErrorCode::InvalidRequest
        );

        let revision = input("{\"session\":1,\"steps\":1,\"expected_revision\":0}");
        let args = validate(spec, &revision, &limits).expect("revision precondition");
        assert_eq!(
            args.optional_unsigned("expected_revision").expect("rev"),
            Some(0)
        );
        let negative = input("{\"session\":1,\"steps\":1,\"expected_revision\":-1}");
        assert_eq!(
            validate(spec, &negative, &limits)
                .expect_err("negative")
                .code(),
            ErrorCode::InvalidRequest
        );
        let absent = input("{\"session\":1,\"steps\":1}");
        let args = validate(spec, &absent, &limits).expect("defaults");
        assert_eq!(args.operation("operation_id"), None);
        assert_eq!(args.optional_unsigned("max_wall_ms").expect("wall"), None);
        assert_eq!(args.optional_unsigned("expected_tick").expect("tick"), None);
    }

    #[test]
    fn the_control_operations_declare_their_operation_key() {
        for name in ["session.cancel", "session.progress", "session.operation"] {
            let spec = find(name).expect("control operation");
            assert!(
                spec.fields.iter().any(|field| {
                    field.name == "operation" && matches!(field.kind, FieldKind::OperationId)
                }),
                "{name} must address an operation"
            );
        }
        // A read-only control operation must not require the write scope: `progress`
        // and `operation` are journal reads.
        assert_eq!(
            find("session.progress").expect("progress").scope,
            Scope::Read
        );
        assert_eq!(
            find("session.operation").expect("operation").scope,
            Scope::Read
        );
        assert_eq!(find("session.cancel").expect("cancel").scope, Scope::Write);
    }
}
