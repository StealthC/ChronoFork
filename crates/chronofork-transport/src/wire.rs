//! Shared wire building blocks for the HTTP and MCP adapters.
//!
//! One place owns the transport-only failure codes, the documented status mapping and the
//! canonical encoding of transport-owned documents, so the two adapters cannot disagree
//! about a status, a header-bearing error body or a JSON document. See architecture §4.

use chronofork_service::json::JsonValue;
use chronofork_service::{ErrorCode, ServiceError, ServiceLimits};

/// The media type carried by every response (architecture §4).
pub(crate) const RESPONSE_MEDIA_TYPE: &str = "application/json; charset=utf-8";

/// A response ready to be written on the wire.
///
/// `allow` is set only for `405 Method Not Allowed`, where architecture §4 requires an
/// `Allow` header next to the typed error envelope.
#[derive(Debug)]
pub(crate) struct Response {
    /// HTTP status code.
    pub(crate) status: u16,
    /// Canonical response body; empty for `202 Accepted` notifications.
    pub(crate) body: Vec<u8>,
    /// Value of the `Allow` header, when one is required.
    pub(crate) allow: Option<&'static str>,
}

impl Response {
    pub(crate) fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            allow: None,
        }
    }

    pub(crate) fn with_allow(mut self, allow: &'static str) -> Self {
        self.allow = Some(allow);
        self
    }
}

/// A transport-level failure code that the service taxonomy does not carry.
///
/// Architecture §4 assigns `bad_request`, `method_not_allowed`, `unsupported_media_type`
/// and `body_too_large` to HTTP framing failures. Every other failure reuses the service
/// code so a client sees one taxonomy across surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WireCode {
    /// Malformed HTTP framing inside an otherwise accepted connection.
    BadRequest,
    /// The route exists but not for this method.
    MethodNotAllowed,
    /// A `POST` without `Content-Type: application/json`.
    UnsupportedMediaType,
    /// A `Content-Length` beyond the configured body limit.
    BodyTooLarge,
}

impl WireCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::BodyTooLarge => "body_too_large",
        }
    }

    pub(crate) const fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::MethodNotAllowed => 405,
            Self::UnsupportedMediaType => 415,
            Self::BodyTooLarge => 413,
        }
    }
}

/// Builds the typed envelope for a transport-only failure code.
pub(crate) fn transport_failure(code: WireCode, message: &'static str) -> Response {
    Response::new(code.status(), error_envelope(code.as_str(), message))
}

/// Builds the typed envelope for a service code used at the transport boundary.
///
/// `not_found`, `unauthorized`, `forbidden` and `invalid_request` are raised by the wire
/// layer as well (unknown route, missing token, missing scope, query string), and they
/// reuse the service envelope so the wire string and the message shape come from one
/// implementation.
pub(crate) fn service_failure(code: ErrorCode, message: &'static str) -> Response {
    let error = ServiceError::new(code, message);
    service_error(&error, &ServiceLimits::default())
}

/// Maps one service failure to its documented status and canonical envelope.
pub(crate) fn service_error(error: &ServiceError, limits: &ServiceLimits) -> Response {
    Response::new(status_for(error.code()), envelope_bytes(error, limits))
}

/// Returns the HTTP status of an [`ErrorCode`] (architecture §4).
pub(crate) const fn status_for(code: ErrorCode) -> u16 {
    match code {
        ErrorCode::InvalidRequest | ErrorCode::UnknownOp | ErrorCode::LimitExceeded => 400,
        ErrorCode::Unauthorized => 401,
        ErrorCode::Forbidden => 403,
        ErrorCode::NotFound => 404,
        ErrorCode::Conflict | ErrorCode::Closed | ErrorCode::Halted => 409,
        ErrorCode::NotConfigured => 503,
        ErrorCode::Backend | ErrorCode::Storage | ErrorCode::Internal => 500,
    }
}

/// Returns the reason phrase written in the status line.
pub(crate) const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// Serializes one transport-owned document under the service's canonical profile.
///
/// The documents built here are the health, catalog and MCP result envelopes. Their size
/// is bounded by the catalog and by the limits passed in; when the configured response
/// bound cannot hold one, the caller answers with the correct status and an empty body
/// instead of panicking or truncating a document.
pub(crate) fn encode(document: &JsonValue, limits: &ServiceLimits) -> Vec<u8> {
    chronofork_service::json::to_canonical_bytes(document, limits).unwrap_or_default()
}

/// Serializes a typed envelope; a message that cannot fit the response bound degrades to
/// an empty body with the correct status rather than a panic.
fn envelope_bytes(error: &ServiceError, limits: &ServiceLimits) -> Vec<u8> {
    error.to_canonical_bytes(limits).unwrap_or_default()
}

fn error_envelope(code: &'static str, message: &'static str) -> Vec<u8> {
    // The envelope shape (schema key, `error` object, `retryable` flag) stays owned by
    // `ServiceError::to_json`; only the code string is replaced, because the transport
    // taxonomy has codes the service taxonomy does not.
    let document = ServiceError::new(ErrorCode::InvalidRequest, message).to_json();
    let document = match document {
        JsonValue::Object(mut root) => {
            if let Some(JsonValue::Object(fields)) = root.get_mut("error") {
                fields.insert("code".to_owned(), JsonValue::string(code));
            }
            JsonValue::Object(root)
        }
        other => other,
    };
    encode(&document, &ServiceLimits::default())
}
