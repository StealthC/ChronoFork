//! Small shared output helpers for the service and its storage/experiment paths.
//!
//! Every integer that reaches the wire is checked against the canonical range and
//! every failure is a typed internal defect, never a panic or a silently clamped
//! value.

use chronofork_backend_api::BackendDescriptor;

use crate::error::ServiceError;
use crate::json::{JsonValue, MAX_INTEGER_U64};

/// Builds a JSON integer, rejecting a value outside the portable range.
pub(crate) fn unsigned(value: u64) -> Result<JsonValue, ServiceError> {
    if value > MAX_INTEGER_U64 {
        return Err(ServiceError::internal(
            "integer is outside the canonical range",
        ));
    }
    i64::try_from(value)
        .map(JsonValue::Int)
        .map_err(|_| ServiceError::internal("integer is outside the canonical range"))
}

/// Builds a JSON integer or `null` from an optional unsigned value.
pub(crate) fn optional_unsigned(value: Option<u64>) -> Result<JsonValue, ServiceError> {
    match value {
        Some(value) => unsigned(value),
        None => Ok(JsonValue::Null),
    }
}

/// Builds a JSON string or `null` from an optional advisory name.
pub(crate) fn optional_name(value: Option<&str>) -> JsonValue {
    match value {
        Some(value) => JsonValue::string(value),
        None => JsonValue::Null,
    }
}

/// Formats bytes as lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// Builds the documented backend object for a descriptor.
pub(crate) fn descriptor_json(descriptor: &BackendDescriptor) -> JsonValue {
    let capabilities = descriptor.capabilities();
    JsonValue::object([
        ("id", JsonValue::string(descriptor.id().as_str())),
        ("display_name", JsonValue::string(descriptor.display_name())),
        ("version", JsonValue::string(descriptor.version())),
        (
            "test_backend",
            JsonValue::from_bool(descriptor.is_test_backend()),
        ),
        (
            "capabilities",
            JsonValue::object([
                ("execution", JsonValue::from_bool(capabilities.execution)),
                (
                    "save_states",
                    JsonValue::from_bool(capabilities.save_states),
                ),
                (
                    "memory_read",
                    JsonValue::from_bool(capabilities.memory_read),
                ),
                (
                    "memory_write",
                    JsonValue::from_bool(capabilities.memory_write),
                ),
                (
                    "register_inspection",
                    JsonValue::from_bool(capabilities.register_inspection),
                ),
                (
                    "instruction_stepping",
                    JsonValue::from_bool(capabilities.instruction_stepping),
                ),
                (
                    "breakpoints",
                    JsonValue::from_bool(capabilities.breakpoints),
                ),
                (
                    "watchpoints",
                    JsonValue::from_bool(capabilities.watchpoints),
                ),
                (
                    "debug_events",
                    JsonValue::from_bool(capabilities.debug_events),
                ),
                ("tracing", JsonValue::from_bool(capabilities.tracing)),
                (
                    "state_cloning",
                    JsonValue::from_bool(capabilities.state_cloning),
                ),
                (
                    "deterministic_replay",
                    JsonValue::from_bool(capabilities.deterministic_replay),
                ),
                ("rendering", JsonValue::from_bool(capabilities.rendering)),
                ("audio", JsonValue::from_bool(capabilities.audio)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::{hex, optional_name, optional_unsigned, unsigned};
    use crate::json::JsonValue;

    #[test]
    fn integers_are_bounded() {
        assert_eq!(unsigned(0).expect("zero"), JsonValue::Int(0));
        assert_eq!(
            unsigned(9_007_199_254_740_991).expect("maximum"),
            JsonValue::Int(9_007_199_254_740_991)
        );
        assert_eq!(
            unsigned(9_007_199_254_740_992)
                .expect_err("out of range")
                .code(),
            crate::error::ErrorCode::Internal
        );
    }

    #[test]
    fn optional_values_render_null() {
        assert_eq!(optional_unsigned(None).expect("none"), JsonValue::Null);
        assert_eq!(optional_name(None), JsonValue::Null);
        assert_eq!(
            optional_name(Some("checkpoint")),
            JsonValue::string("checkpoint")
        );
    }

    #[test]
    fn bytes_render_as_lowercase_hex() {
        assert_eq!(hex(&[0x00, 0x0F, 0xA5, 0xFF]), "000fa5ff");
        assert_eq!(hex(&[]), "");
    }
}
