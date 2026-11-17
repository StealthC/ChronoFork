//! The single canonical operation seam between the presentation layer and a service.
//!
//! There is exactly one way out of this crate: [`OperationChannel::call`] over an
//! already-encoded canonical request document. A request is
//! `{"op": <name>, "input": {<fields>}}` and a successful response is
//! `{"op": <name>, "result": {<fields>}}`, both produced and read by
//! `chronofork_service::json`, so the service codec stays the only JSON implementation
//! in the path (ADR-0013 §3.1, `gui.md` §2).
//!
//! The crate never owns the transport: the CLI implements this trait over the M6 typed
//! HTTP client (remote mode) and over an in-process `AppService` (local mode). A failure
//! is a [`ChannelFault`] carrying a bounded, path-free, token-free detail plus the
//! canonical `chronofork.error.v1` bytes when the peer produced them; the process
//! surface prints those bytes verbatim.
//!
//! `ObjectView` is the one strict field reader used by every decoder in [`crate::view`]:
//! it enforces the documented field set exactly — a missing, mistyped or unknown field is
//! a typed [`GuiError::Protocol`], never a default value.

use std::collections::BTreeMap;
use std::fmt;

use chronofork_service::ErrorCode;
use chronofork_service::ServiceError;
use chronofork_service::json::{JsonValue, MAX_INTEGER_U64, to_canonical_bytes};

use crate::{GuiError, bounded_detail, codec_limits};

/// One canonical operation call over an already-encoded request document.
///
/// The request is the canonical `{"op":..., "input":...}` document and the response is
/// the canonical envelope, so an implementor never parses or rewrites a field: it moves
/// bytes and reports a [`ChannelFault`]. Implementations are shared (`&self`), so one
/// channel can serve every pane and the model.
pub trait OperationChannel: Send + Sync {
    /// Performs one operation call.
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelFault`] when the transport failed or the peer answered with
    /// the canonical `chronofork.error.v1` envelope.
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault>;
}

/// One channel failure: a bounded diagnostic plus the peer error envelope, if any.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelFault {
    detail: String,
    envelope: Option<Vec<u8>>,
}

impl ChannelFault {
    /// Creates a fault without a peer error envelope.
    #[must_use]
    pub fn new(detail: &str) -> Self {
        Self {
            detail: bounded_detail(detail),
            envelope: None,
        }
    }

    /// Creates a fault carrying the canonical `chronofork.error.v1` bytes of the peer.
    ///
    /// The bytes are kept verbatim because the process surface prints them as the
    /// operation failure; only the diagnostic detail is bounded and sanitized.
    #[must_use]
    pub fn with_envelope(detail: &str, envelope: Vec<u8>) -> Self {
        Self {
            detail: bounded_detail(detail),
            envelope: Some(envelope),
        }
    }

    /// Returns the bounded, path-free and token-free diagnostic.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// Returns the canonical peer error envelope, when the peer produced one.
    #[must_use]
    pub fn envelope(&self) -> Option<&[u8]> {
        self.envelope.as_deref()
    }
}

impl fmt::Display for ChannelFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ChannelFault {}

impl From<ChannelFault> for GuiError {
    fn from(fault: ChannelFault) -> Self {
        Self::Channel {
            detail: fault.detail,
            envelope: fault.envelope,
        }
    }
}

/// Encodes one canonical request document for `op` with the given `input` object.
///
/// # Errors
///
/// Returns [`GuiError::LimitExceeded`] when the encoded request exceeds the codec byte
/// limit and [`GuiError::Protocol`] for an input the canonical profile cannot encode
/// (for example an integer outside the portable range).
pub fn request_bytes(op: &str, input: &JsonValue) -> Result<Vec<u8>, GuiError> {
    if input.as_object().is_none() {
        return Err(GuiError::bad_value(
            "operation input",
            "an operation input must be a JSON object",
        ));
    }
    let document = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
    let limits = codec_limits();
    to_canonical_bytes(&document, &limits).map_err(|error| {
        if error.code() == ErrorCode::LimitExceeded {
            GuiError::limit_exceeded(
                "canonical request bytes",
                u64::try_from(limits.max_request_bytes).unwrap_or(u64::MAX),
            )
        } else {
            GuiError::protocol(&error.to_string())
        }
    })
}

/// Parses one canonical response document under the service codec.
///
/// # Errors
///
/// Returns [`GuiError::LimitExceeded`] when the document exceeds the codec input byte
/// limit and [`GuiError::Protocol`] for any syntax, profile or duplicate-key violation.
pub fn parse_response(bytes: &[u8]) -> Result<JsonValue, GuiError> {
    let limits = codec_limits();
    chronofork_service::json::parse(bytes, &limits).map_err(|error| codec_failure(&error, &limits))
}

/// Returns the `result` of a canonical envelope whose operation name is `op`.
///
/// # Errors
///
/// Returns [`GuiError::Protocol`] when the document is not an `{"op","result"}` object,
/// when `op` is not a string, or when the response names a different operation.
pub fn result_for<'a>(document: &'a JsonValue, op: &str) -> Result<&'a JsonValue, GuiError> {
    let view = ObjectView::exact(document, &["op", "result"], "response")?;
    let actual = view.text("op")?;
    if actual != op {
        return Err(GuiError::protocol(&format!(
            "response carries operation `{actual}` instead of `{op}`"
        )));
    }
    view.raw("result")
}

/// Builds one canonical non-negative integer value for an operation input.
///
/// # Errors
///
/// Returns [`GuiError::LimitExceeded`] when `value` is outside the portable JSON
/// integer range, which is the bound the service enforces on every input.
pub fn unsigned_input(value: u64) -> Result<JsonValue, GuiError> {
    if value > MAX_INTEGER_U64 {
        return Err(GuiError::limit_exceeded(
            "canonical input integer",
            MAX_INTEGER_U64,
        ));
    }
    i64::try_from(value)
        .map(JsonValue::Int)
        .map_err(|_| GuiError::limit_exceeded("canonical input integer", MAX_INTEGER_U64))
}

/// Maps a codec failure onto the closed GUI protocol codes.
///
/// A byte-limit violation is reported as `limit_exceeded`; every other codec rejection
/// (syntax, profile, duplicate key, depth) means the peer response was not canonical and
/// is reported as a protocol failure under the `channel` code.
fn codec_failure(error: &ServiceError, limits: &chronofork_service::ServiceLimits) -> GuiError {
    if error.code() == ErrorCode::LimitExceeded {
        GuiError::limit_exceeded(
            "canonical response bytes",
            u64::try_from(limits.max_request_bytes).unwrap_or(u64::MAX),
        )
    } else {
        GuiError::protocol(&error.to_string())
    }
}

/// A strict, field-exact reader over one JSON object.
///
/// Construction verifies the documented field set in both directions: every allowed
/// field is present and no other field exists. The accessors then reject a mistyped
/// value. This is the one place where "strict decoding" is implemented, so every view
/// decoder inherits the same rule (ADR-0013 §3.2, `gui.md` §2).
#[derive(Debug)]
pub(crate) struct ObjectView<'a> {
    what: &'static str,
    fields: &'a BTreeMap<String, JsonValue>,
}

impl<'a> ObjectView<'a> {
    /// Asserts that `value` is an object with exactly the `allowed` fields.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::Protocol`] for a non-object, a missing field or an unknown
    /// field.
    pub(crate) fn exact(
        value: &'a JsonValue,
        allowed: &[&'static str],
        what: &'static str,
    ) -> Result<Self, GuiError> {
        let fields = value
            .as_object()
            .ok_or_else(|| GuiError::protocol(&format!("{what} is not a JSON object")))?;
        for key in fields.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(GuiError::protocol(&format!(
                    "{what} carries unknown field `{key}`"
                )));
            }
        }
        for name in allowed {
            if !fields.contains_key(*name) {
                return Err(GuiError::protocol(&format!(
                    "{what} is missing field `{name}`"
                )));
            }
        }
        Ok(Self { what, fields })
    }

    /// Returns the raw value of a field that construction already verified as present.
    pub(crate) fn raw(&self, field: &'static str) -> Result<&'a JsonValue, GuiError> {
        self.fields.get(field).ok_or_else(|| {
            GuiError::protocol(&format!("{} lost a field validated as present", self.what))
        })
    }

    /// Returns a field as a string.
    pub(crate) fn text(&self, field: &'static str) -> Result<&'a str, GuiError> {
        self.raw(field)?.as_str().ok_or_else(|| {
            GuiError::protocol(&format!("{} field `{field}` is not a string", self.what))
        })
    }

    /// Returns a field as a non-negative integer.
    pub(crate) fn unsigned(&self, field: &'static str) -> Result<u64, GuiError> {
        self.raw(field)?.as_u64().ok_or_else(|| {
            GuiError::protocol(&format!(
                "{} field `{field}` is not a non-negative integer",
                self.what
            ))
        })
    }

    /// Returns a field as an optional non-negative integer (`null` means absent).
    pub(crate) fn optional_unsigned(&self, field: &'static str) -> Result<Option<u64>, GuiError> {
        let value = self.raw(field)?;
        if matches!(value, JsonValue::Null) {
            return Ok(None);
        }
        value.as_u64().map(Some).ok_or_else(|| {
            GuiError::protocol(&format!(
                "{} field `{field}` is neither a non-negative integer nor null",
                self.what
            ))
        })
    }

    /// Returns a field as a boolean.
    pub(crate) fn boolean(&self, field: &'static str) -> Result<bool, GuiError> {
        self.raw(field)?.as_bool().ok_or_else(|| {
            GuiError::protocol(&format!("{} field `{field}` is not a boolean", self.what))
        })
    }

    /// Returns a field as an array.
    pub(crate) fn array(&self, field: &'static str) -> Result<&'a [JsonValue], GuiError> {
        self.raw(field)?.as_array().ok_or_else(|| {
            GuiError::protocol(&format!("{} field `{field}` is not an array", self.what))
        })
    }
}

#[cfg(test)]
mod tests {
    use chronofork_service::ServiceLimits;
    use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};

    use super::{
        ChannelFault, ObjectView, OperationChannel, parse_response, request_bytes, result_for,
        unsigned_input,
    };
    use crate::{GuiError, GuiErrorCode, MAX_DETAIL_BYTES};

    fn limits() -> ServiceLimits {
        ServiceLimits::default()
    }

    fn canonical(value: &JsonValue) -> Vec<u8> {
        to_canonical_bytes(value, &limits()).expect("canonical bytes")
    }

    fn empty_object() -> JsonValue {
        JsonValue::Object(std::collections::BTreeMap::new())
    }

    fn envelope(op: &str, result: JsonValue) -> Vec<u8> {
        canonical(&JsonValue::object([
            ("op", JsonValue::string(op)),
            ("result", result),
        ]))
    }

    #[test]
    fn requests_are_canonical_sorted_and_key_exact() {
        let input =
            JsonValue::object([("session", JsonValue::int(7)), ("steps", JsonValue::int(3))]);
        let bytes = request_bytes("session.run", &input).expect("request bytes");
        assert_eq!(
            bytes,
            b"{\"input\":{\"session\":7,\"steps\":3},\"op\":\"session.run\"}"
        );
        let reparsed = parse(&bytes, &limits()).expect("canonical request");
        assert_eq!(
            reparsed.get("op").and_then(JsonValue::as_str),
            Some("session.run")
        );
    }

    #[test]
    fn request_input_must_be_an_object() {
        for input in [
            JsonValue::Array(Vec::new()),
            JsonValue::Int(1),
            JsonValue::Null,
        ] {
            assert_eq!(
                request_bytes("capabilities", &input)
                    .expect_err("non-object input")
                    .code(),
                GuiErrorCode::BadValue
            );
        }
    }

    #[test]
    fn unsigned_inputs_are_bounded_by_the_portable_integer_range() {
        assert_eq!(unsigned_input(0).expect("zero"), JsonValue::Int(0));
        assert_eq!(
            unsigned_input(9_007_199_254_740_991).expect("maximum"),
            JsonValue::Int(9_007_199_254_740_991)
        );
        assert_eq!(
            unsigned_input(9_007_199_254_740_992)
                .expect_err("outside the canonical range")
                .code(),
            GuiErrorCode::LimitExceeded
        );
    }

    #[test]
    fn responses_are_parsed_canonically_and_anonymized_failures_stay_typed() {
        let bytes = envelope("capabilities", empty_object());
        let document = parse_response(&bytes).expect("canonical response");
        assert_eq!(
            document.get("op").and_then(JsonValue::as_str),
            Some("capabilities")
        );

        let error = parse_response(b"{\"op\":\"capabilities\",\"result\":{},\"op\":1}")
            .expect_err("duplicate key");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().starts_with("channel: "));

        let oversized = vec![b'a'; limits().max_request_bytes + 1];
        assert_eq!(
            parse_response(&oversized)
                .expect_err("oversized document")
                .code(),
            GuiErrorCode::LimitExceeded
        );
    }

    #[test]
    fn a_result_is_returned_only_for_the_documented_operation() {
        let bytes = envelope(
            "session.inspect",
            JsonValue::object([("session", JsonValue::int(1))]),
        );
        let document = parse_response(&bytes).expect("response");
        let result = result_for(&document, "session.inspect").expect("result");
        assert_eq!(result.get("session").and_then(JsonValue::as_u64), Some(1));

        let mismatch = result_for(&document, "session.list").expect_err("wrong op");
        assert_eq!(mismatch.code(), GuiErrorCode::Channel);
        assert!(mismatch.to_string().contains("session.list"));

        let missing = parse_response(b"{\"op\":\"capabilities\"}").expect("partial response");
        assert!(result_for(&missing, "capabilities").is_err());
    }

    #[test]
    fn channel_faults_are_bounded_and_carry_the_peer_envelope() {
        let long = "e".repeat(MAX_DETAIL_BYTES * 3);
        let fault = ChannelFault::new(&long);
        assert_eq!(fault.detail().len(), MAX_DETAIL_BYTES);
        assert_eq!(fault.envelope(), None);

        let bytes = b"{\"schema\":\"chronofork.error.v1\"}".to_vec();
        let fault = ChannelFault::with_envelope("forbidden", bytes.clone());
        assert_eq!(fault.envelope(), Some(bytes.as_slice()));
        let error: GuiError = fault.into();
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert_eq!(error.envelope(), Some(bytes.as_slice()));
        assert_eq!(error.to_string(), "channel: forbidden");
    }

    #[test]
    fn object_views_reject_missing_unknown_and_mistyped_fields() {
        let document = parse(
            b"{\"present\":1,\"text\":\"x\",\"flag\":true,\"list\":[],\"null\":null}",
            &limits(),
        )
        .expect("object");
        let view = ObjectView::exact(
            &document,
            &["present", "text", "flag", "list", "null"],
            "test result",
        )
        .expect("exact object");
        assert_eq!(view.unsigned("present").expect("present"), 1);
        assert_eq!(view.text("text").expect("text"), "x");
        assert!(view.boolean("flag").expect("flag"));
        assert!(view.array("list").expect("list").is_empty());
        assert_eq!(view.optional_unsigned("null").expect("null"), None);

        let unknown = parse(b"{\"present\":1,\"extra\":2}", &limits()).expect("object");
        let error =
            ObjectView::exact(&unknown, &["present"], "test result").expect_err("unknown field");
        assert!(error.to_string().contains("unknown field `extra`"));

        let missing = parse(b"{\"present\":1}", &limits()).expect("object");
        let error = ObjectView::exact(&missing, &["present", "absent"], "test result")
            .expect_err("missing");
        assert!(error.to_string().contains("missing field `absent`"));

        let mistyped = parse(b"{\"present\":\"1\"}", &limits()).expect("object");
        let view = ObjectView::exact(&mistyped, &["present"], "test result").expect("object view");
        assert_eq!(view.text("present").expect("present is a string"), "1");
        assert_eq!(
            view.unsigned("present").expect_err("mistyped").code(),
            GuiErrorCode::Channel
        );
        assert!(
            view.boolean("present")
                .expect_err("mistyped")
                .to_string()
                .contains("not a boolean")
        );

        let not_object = parse(b"[]", &limits()).expect("array");
        assert!(
            ObjectView::exact(&not_object, &[], "test result")
                .expect_err("not an object")
                .to_string()
                .contains("not a JSON object")
        );
    }

    /// A channel that fails every call, so a caller cannot silently ignore a fault.
    struct FailingChannel;

    impl OperationChannel for FailingChannel {
        fn call(&self, _request: &[u8]) -> Result<Vec<u8>, ChannelFault> {
            Err(ChannelFault::new("peer is unreachable"))
        }
    }

    #[test]
    fn an_operation_channel_is_object_safe_and_shareable() {
        let channel: &dyn OperationChannel = &FailingChannel;
        let request = request_bytes("capabilities", &empty_object()).expect("request");
        let fault = channel.call(&request).expect_err("failing channel");
        assert_eq!(fault.detail(), "peer is unreachable");
    }
}
