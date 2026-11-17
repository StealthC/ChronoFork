//! MCP over Streamable HTTP: the stateless JSON-RPC 2.0 tools subset.
//!
//! `POST /mcp` speaks MCP protocol `2025-03-26` (architecture §5). It supports exactly
//! `initialize`, `ping`, `tools/list` and `tools/call`; a notification (a JSON-RPC request
//! without an `id` member, which is how `notifications/initialized` is sent) answers
//! `202 Accepted` with an empty body. Nothing is remembered between requests: there is no
//! `Mcp-Session-Id`, no server-initiated message and no `text/event-stream` response.
//!
//! A `tools/call` result carries the service envelope bytes as the text content block and
//! `isError` reports whether that envelope is a typed error, so an MCP call, an HTTP call
//! and a direct [`chronofork_service::AppService`] call are comparable byte for byte after
//! the JSON string is decoded. Malformed params and unknown tool names are JSON-RPC
//! `-32602`; a batch request is `-32600`, invalid JSON is `-32700`, an unknown method is
//! `-32601`. JSON-RPC errors in a well-formed request answer `200 OK` with an `error`
//! member; transport-level framing errors (`-32700`, `-32600`) answer `400`. The single
//! additional code, `-32603`, is used only when an otherwise valid result cannot be
//! serialized inside `max_response_bytes`; no operation in the catalog can reach it with
//! the default limits.

use std::collections::BTreeMap;

use chronofork_service::json::JsonValue;
use chronofork_service::service::SERVICE_VERSION;
use chronofork_service::{AppService, Principal, ServiceError, ServiceLimits};

use crate::wire::{self, Response};

/// MCP protocol version implemented by this endpoint.
pub(crate) const PROTOCOL_VERSION: &str = "2025-03-26";

/// JSON-RPC parse error.
const PARSE_ERROR: i64 = -32700;
/// JSON-RPC invalid-request error (includes unsupported batches).
const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC method-not-found error.
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC invalid-params error (includes unknown tool names).
const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC internal error, used only when a result cannot fit the response bound.
const INTERNAL_ERROR: i64 = -32603;

/// The outcome of one MCP `tools/call`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallResult {
    /// Canonical JSON text placed in the MCP text content block.
    pub text: String,
    /// Whether the operation returned a typed error envelope.
    pub is_error: bool,
}

/// A JSON-RPC error that short-circuits one request.
#[derive(Clone, Copy, Debug)]
struct RpcError {
    code: i64,
    message: &'static str,
}

impl RpcError {
    const fn new(code: i64, message: &'static str) -> Self {
        Self { code, message }
    }
}

fn invalid_params(message: &'static str) -> RpcError {
    RpcError::new(INVALID_PARAMS, message)
}

/// Answers one `POST /mcp` body.
pub(crate) fn respond(
    body: &[u8],
    principal: &Principal,
    service: &AppService,
    limits: &ServiceLimits,
) -> Response {
    let document = match chronofork_service::json::parse(body, limits) {
        Ok(document) => document,
        Err(_) => return failure(JsonValue::Null, RpcError::new(PARSE_ERROR, "parse error")),
    };
    let request = match document {
        JsonValue::Object(request) => request,
        JsonValue::Array(_) => {
            return failure(
                JsonValue::Null,
                RpcError::new(INVALID_REQUEST, "batch requests are not supported"),
            );
        }
        _ => {
            return failure(
                JsonValue::Null,
                RpcError::new(INVALID_REQUEST, "invalid request"),
            );
        }
    };

    let id = request.get("id").cloned();
    if let Some(id) = &id {
        if !matches!(
            id,
            JsonValue::Int(_) | JsonValue::String(_) | JsonValue::Null
        ) {
            return failure(
                JsonValue::Null,
                RpcError::new(INVALID_REQUEST, "invalid request identifier"),
            );
        }
    }
    if request.get("jsonrpc").and_then(JsonValue::as_str) != Some("2.0") {
        return failure(
            id.unwrap_or(JsonValue::Null),
            RpcError::new(INVALID_REQUEST, "invalid request"),
        );
    }
    let Some(method) = request.get("method").and_then(JsonValue::as_str) else {
        return failure(
            id.unwrap_or(JsonValue::Null),
            RpcError::new(INVALID_REQUEST, "invalid request"),
        );
    };
    if id.is_none() {
        // A notification has no response body; `notifications/initialized` lands here.
        return Response::new(202, Vec::new());
    }
    let params = request.get("params").cloned();
    let outcome = match method {
        "initialize" => initialize(params),
        "ping" => ping(params),
        "tools/list" => list_tools(params, service),
        "tools/call" => call_tool(params, principal, service, limits),
        _ => Err(RpcError::new(METHOD_NOT_FOUND, "method not found")),
    };
    match outcome {
        Ok(result) => success(id.unwrap_or(JsonValue::Null), result, limits),
        Err(error) => failure(id.unwrap_or(JsonValue::Null), error),
    }
}

fn initialize(params: Option<JsonValue>) -> Result<JsonValue, RpcError> {
    match params {
        None | Some(JsonValue::Object(_)) => {}
        Some(_) => return Err(invalid_params("`params` must be an object")),
    }
    Ok(JsonValue::object([
        ("protocolVersion", JsonValue::string(PROTOCOL_VERSION)),
        (
            "capabilities",
            JsonValue::object([(
                "tools",
                JsonValue::object([("listChanged", JsonValue::from_bool(false))]),
            )]),
        ),
        (
            "serverInfo",
            JsonValue::object([
                ("name", JsonValue::string("chronofork")),
                ("version", JsonValue::string(SERVICE_VERSION)),
            ]),
        ),
    ]))
}

fn ping(params: Option<JsonValue>) -> Result<JsonValue, RpcError> {
    match params {
        None | Some(JsonValue::Null) | Some(JsonValue::Object(_)) => {}
        Some(_) => return Err(invalid_params("`params` must be an object")),
    }
    Ok(JsonValue::Object(BTreeMap::new()))
}

fn list_tools(params: Option<JsonValue>, service: &AppService) -> Result<JsonValue, RpcError> {
    match params {
        None | Some(JsonValue::Null) | Some(JsonValue::Object(_)) => {}
        Some(_) => return Err(invalid_params("`params` must be an object")),
    }
    let tools: Vec<JsonValue> = service
        .tools()
        .into_iter()
        .map(|tool| {
            JsonValue::object([
                ("name", JsonValue::string(tool.name)),
                ("description", JsonValue::string(tool.description)),
                ("inputSchema", tool.input_schema),
            ])
        })
        .collect();
    Ok(JsonValue::object([("tools", JsonValue::Array(tools))]))
}

fn call_tool(
    params: Option<JsonValue>,
    principal: &Principal,
    service: &AppService,
    limits: &ServiceLimits,
) -> Result<JsonValue, RpcError> {
    let Some(JsonValue::Object(params)) = params else {
        return Err(invalid_params("`params` must be an object"));
    };
    for key in params.keys() {
        if key != "name" && key != "arguments" {
            return Err(invalid_params("unknown field in `params`"));
        }
    }
    let Some(name) = params.get("name").and_then(JsonValue::as_str) else {
        return Err(invalid_params("`name` is required"));
    };
    let arguments = match params.get("arguments") {
        None | Some(JsonValue::Null) => JsonValue::Object(BTreeMap::new()),
        Some(JsonValue::Object(fields)) => JsonValue::Object(fields.clone()),
        Some(_) => return Err(invalid_params("`arguments` must be an object")),
    };
    if !service.tools().iter().any(|tool| tool.name == name) {
        return Err(invalid_params("unknown tool"));
    }
    let request = JsonValue::object([("op", JsonValue::string(name)), ("input", arguments)]);
    let envelope = match chronofork_service::json::to_canonical_bytes(&request, limits) {
        Ok(bytes) => bytes,
        Err(error) => return Ok(text_result(envelope_text(&error, limits), true)),
    };
    match service.dispatch(principal, &envelope) {
        Ok(envelope) => Ok(text_result(as_text(&envelope), false)),
        Err(error) => Ok(text_result(envelope_text(&error, limits), true)),
    }
}

fn text_result(text: String, is_error: bool) -> JsonValue {
    JsonValue::object([
        (
            "content",
            JsonValue::array([JsonValue::object([
                ("type", JsonValue::string("text")),
                ("text", JsonValue::string(text)),
            ])]),
        ),
        ("isError", JsonValue::from_bool(is_error)),
    ])
}

fn as_text(bytes: &[u8]) -> String {
    // Canonical documents are valid UTF-8 by construction; the empty fallback keeps this
    // total instead of panicking on an invariant break.
    String::from_utf8(bytes.to_vec()).unwrap_or_default()
}

fn envelope_text(error: &ServiceError, limits: &ServiceLimits) -> String {
    as_text(&error.to_canonical_bytes(limits).unwrap_or_default())
}

fn success(id: JsonValue, result: JsonValue, limits: &ServiceLimits) -> Response {
    let document = JsonValue::object([
        ("jsonrpc", JsonValue::string("2.0")),
        ("id", id),
        ("result", result),
    ]);
    match chronofork_service::json::to_canonical_bytes(&document, limits) {
        Ok(body) => Response::new(200, body),
        Err(_) => failure(
            JsonValue::Null,
            RpcError::new(INTERNAL_ERROR, "response exceeds the configured byte limit"),
        ),
    }
}

fn failure(id: JsonValue, error: RpcError) -> Response {
    let document = JsonValue::object([
        ("jsonrpc", JsonValue::string("2.0")),
        ("id", id),
        (
            "error",
            JsonValue::object([
                ("code", JsonValue::int(error.code)),
                ("message", JsonValue::string(error.message)),
            ]),
        ),
    ]);
    let status = match error.code {
        PARSE_ERROR | INVALID_REQUEST => 400,
        _ => 200,
    };
    Response::new(status, wire::encode(&document, &ServiceLimits::default()))
}
