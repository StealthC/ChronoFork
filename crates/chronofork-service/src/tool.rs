//! The typed operation catalog exposed through every transport.

use crate::auth::Scope;
use crate::json::JsonValue;

/// One machine-callable operation.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    /// Stable operation name, also the MCP tool name.
    pub name: &'static str,
    /// One-line description for `tools/list`.
    pub description: &'static str,
    /// Whether the operation mutates service or durable state.
    pub mutating: bool,
    /// Minimum scope required to call the operation.
    pub scope: Scope,
    /// JSON Schema subset describing the arguments object.
    pub input_schema: JsonValue,
}
