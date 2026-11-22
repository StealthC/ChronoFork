//! The API wire version and the v1/v2 shape matrix (ADR-0016 §Compatibility/F10).
//!
//! The API wire is **v1** to an existing client and **v2** to one that asks for it. A
//! request without a version is v1, and a request that asks for a version this server
//! does not implement is refused **before dispatch**, so an incompatible call can never
//! apply a partial mutation.
//!
//! Two things are versioned, and only these two:
//!
//! - the **input surface**: the v2-only operations and the v2-only input fields are not
//!   part of v1, so a v1 request that names one is `unknown_op`/`invalid_request` rather
//!   than silently accepted;
//! - the **result/error shape**: the fields v2 added are removed from every v1 result
//!   ([`v1_result_fields`]), and a v1 error envelope carries no `wire_version`.

/// The API wire version one request is served with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiVersion {
    /// The original wire: the sixteen pre-ADR-0016 operations and their shapes.
    V1,
    /// The agent-API wire: adds the control operations, the reconciliation fields and
    /// the `wire_version` marker.
    V2,
}

impl ApiVersion {
    /// Returns the numeric wire version.
    #[must_use]
    pub const fn number(self) -> u64 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    /// Returns the stable string a client sends in its version header.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "1",
            Self::V2 => "2",
        }
    }

    /// Returns whether this is the v2 wire.
    #[must_use]
    pub const fn is_v2(self) -> bool {
        matches!(self, Self::V2)
    }

    /// Parses the version selector of one request.
    ///
    /// An absent selector is v1 (the compatibility rule); anything else that is not a
    /// version this server implements is rejected by the caller with a typed error.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "1" => Some(Self::V1),
            "2" => Some(Self::V2),
            _ => None,
        }
    }
}

/// Returns the v2-only **input** fields (ADR-0016 §2/§3).
///
/// A v1 request may not name one of these: the field is not part of the published v1
/// schema, and accepting it silently would make a v1 call depend on v2 semantics.
pub const V2_ONLY_FIELDS: [&str; 5] = [
    "operation_id",
    "operation_epoch",
    "max_wall_ms",
    "expected_revision",
    "expected_tick",
];

/// The v2-only **result** fields (ADR-0016 §2).
///
/// Unlike [`v1_result_fields`], these are added to every v2 mutation result, so a v1
/// response removes them from every operation instead of from a named list. They are
/// operational metadata: the process epoch a reconciliation result belongs to.
pub const V2_ONLY_RESULT_FIELDS: [&str; 1] = ["process_epoch"];

/// Returns the v2-only **operations** (ADR-0016 §2/§3).
///
/// A v1 request naming one is `unknown_op`: the operation does not exist on the v1
/// wire, so the shape it would produce is never promised to a v1 client.
pub const V2_ONLY_OPS: [&str; 4] = [
    "session.cancel",
    "session.progress",
    "session.operation",
    "session.reap",
];

/// Returns whether one operation belongs to the v2 wire only.
#[must_use]
pub fn is_v2_only_op(op: &str) -> bool {
    V2_ONLY_OPS.contains(&op)
}

/// Returns whether one input field belongs to the v2 wire only.
#[must_use]
pub fn is_v2_only_field(field: &str) -> bool {
    V2_ONLY_FIELDS.contains(&field)
}

/// Returns the result fields v2 added to one operation.
///
/// A v1 response removes exactly these, so the v1 shape is the documented shape an
/// existing client decodes. The list is the single table the projection uses, and it
/// names only fields v2 actually added: `session.create` publishes no revision (a new
/// session is revision zero by construction), so it needs no projection entry.
#[must_use]
pub fn v1_result_fields(op: &str) -> &'static [&'static str] {
    match op {
        "capabilities" => &["api_version", "process_epoch"],
        "session.fork" | "session.inspect" | "session.restore" => &["revision"],
        "session.run" => &["performed", "stop_reason", "revision", "max_wall_ms"],
        "session.snapshot" => &["snapshots", "snapshot_bytes"],
        "session.close" => &["released_checkpoint_bytes"],
        // `session.list` entries gained the `running` marker; the entry projection is
        // applied by the dispatcher, not by field removal here.
        _ => &[],
    }
}
