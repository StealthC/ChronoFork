#![allow(clippy::expect_used)]

//! Wire-level acceptance tests for the stateless MCP Streamable HTTP endpoint.
//!
//! The endpoint is driven with hand-written JSON-RPC bytes so the protocol subset of
//! architecture §5 (initialize, notifications, ping, tools/list, tools/call, the error
//! codes and the absence of sessions/SSE) is proven as an MCP client sees it.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use chronofork_service::json::{JsonValue, parse};
use chronofork_service::{
    AppService, Principal, Scope, ServiceConfig, ServiceLimits, TokenTable, parse_token_file,
};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{ServerConfig, ServerHandle, serve};

const OPERATOR_TOKEN: &str = "operator-token-0001";

const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]},
 {"name":"reader","token":"reader-token-00001","scopes":["read"]}]}"#;

fn tokens() -> TokenTable {
    parse_token_file(TOKENS.as_bytes()).expect("token table")
}

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn service() -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })
    .expect("service")
}

fn start() -> (ServerHandle, SocketAddr) {
    let server = serve(
        ServerConfig {
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service()),
    )
    .expect("server");
    let addr = server.local_addr();
    (server, addr)
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> JsonValue {
        parse(&self.body, &limits()).expect("json-rpc response body")
    }

    fn rpc_code(&self) -> i64 {
        self.json()
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_i64)
            .expect("json-rpc error code")
    }

    fn rpc_message(&self) -> String {
        self.json()
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(JsonValue::as_str)
            .expect("json-rpc error message")
            .to_owned()
    }

    fn result(&self) -> JsonValue {
        self.json().get("result").expect("json-rpc result").clone()
    }
}

fn send(addr: SocketAddr, body: &[u8], token: Option<&str>) -> HttpResponse {
    send_with_content_type(addr, body, token, Some("application/json"))
}

fn send_with_content_type(
    addr: SocketAddr,
    body: &[u8],
    token: Option<&str>,
    content_type: Option<&str>,
) -> HttpResponse {
    let mut head = String::from("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(token) = token {
        head.push_str("Authorization: Bearer ");
        head.push_str(token);
        head.push_str("\r\n");
    }
    if let Some(content_type) = content_type {
        head.push_str("Content-Type: ");
        head.push_str(content_type);
        head.push_str("\r\n");
    }
    head.push_str("Content-Length: ");
    head.push_str(&body.len().to_string());
    head.push_str("\r\nConnection: close\r\n\r\n");
    let mut request = head.into_bytes();
    request.extend_from_slice(body);
    raw(addr, &request)
}

fn raw(addr: SocketAddr, request: &[u8]) -> HttpResponse {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");
    stream.write_all(request).expect("write request");
    stream.flush().expect("flush request");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read response");
    parse_response(&bytes)
}

fn parse_response(bytes: &[u8]) -> HttpResponse {
    let end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response head terminator")
        + 4;
    let head = std::str::from_utf8(&bytes[..end - 4]).expect("response head is UTF-8");
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .expect("status line")
        .split(' ')
        .nth(1)
        .expect("status code")
        .parse::<u16>()
        .expect("numeric status");
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').expect("response header");
            (name.to_ascii_lowercase(), value.trim().to_owned())
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: bytes.get(end..).unwrap_or_default().to_vec(),
    }
}

/// Sends one already-serialized request to `POST /v1/ops` and returns the body bytes.
fn http_call(addr: SocketAddr, body: &str) -> Vec<u8> {
    let request = format!(
        "POST /v1/ops HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {OPERATOR_TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    raw(addr, request.as_bytes()).body
}

fn rpc(body: &str) -> HttpResponse {
    let (server, addr) = start();
    let response = send(addr, body.as_bytes(), Some(OPERATOR_TOKEN));
    drop(server);
    response
}

fn call_tool(name: &str, arguments: &str) -> HttpResponse {
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#
    );
    rpc(&request)
}

fn text_of(result: &JsonValue) -> (&str, bool) {
    let content = result
        .get("content")
        .and_then(JsonValue::as_array)
        .and_then(|entries| entries.first())
        .expect("content block");
    assert_eq!(
        content.get("type").and_then(JsonValue::as_str),
        Some("text")
    );
    let text = content
        .get("text")
        .and_then(JsonValue::as_str)
        .expect("text content");
    let is_error = result
        .get("isError")
        .and_then(JsonValue::as_bool)
        .expect("isError flag");
    (text, is_error)
}

// ---------------------------------------------------------------------------
// Protocol subset
// ---------------------------------------------------------------------------

#[test]
fn initialize_reports_the_documented_protocol_and_server_info() {
    let response = rpc(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0.1.0"}}}"#,
    );
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(response.header("connection"), Some("close"));
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    assert!(
        response.header("mcp-session-id").is_none(),
        "the endpoint is stateless and issues no session identifier"
    );
    let document = response.json();
    assert_eq!(
        document.get("jsonrpc").and_then(JsonValue::as_str),
        Some("2.0")
    );
    assert_eq!(
        document.get("id").and_then(JsonValue::as_i64),
        Some(1),
        "the request id is echoed exactly"
    );
    let result = response.result();
    assert_eq!(
        result.get("protocolVersion").and_then(JsonValue::as_str),
        Some("2025-03-26")
    );
    assert_eq!(
        result
            .get("capabilities")
            .and_then(|capabilities| capabilities.get("tools"))
            .and_then(|tools| tools.get("listChanged"))
            .and_then(JsonValue::as_bool),
        Some(false)
    );
    let server_info = result.get("serverInfo").expect("serverInfo");
    assert_eq!(
        server_info.get("name").and_then(JsonValue::as_str),
        Some("chronofork")
    );
    assert_eq!(
        server_info.get("version").and_then(JsonValue::as_str),
        Some(env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn notifications_answer_202_with_an_empty_body() {
    let (server, addr) = start();
    for request in [
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"ping"}"#,
    ] {
        let response = send(addr, request.as_bytes(), Some(OPERATOR_TOKEN));
        assert_eq!(response.status, 202, "{request}");
        assert!(response.body.is_empty(), "{request}");
        assert_eq!(response.header("content-length"), Some("0"), "{request}");
        assert_eq!(
            response.header("content-type"),
            Some("application/json; charset=utf-8")
        );
    }
    drop(server);
}

#[test]
fn ping_returns_an_empty_result_object() {
    let response = rpc(r#"{"jsonrpc":"2.0","id":"probe","method":"ping"}"#);
    assert_eq!(response.status, 200);
    let result = response.result();
    assert_eq!(
        result.as_object().map(std::collections::BTreeMap::len),
        Some(0)
    );
    assert_eq!(
        response.json().get("id").and_then(JsonValue::as_str),
        Some("probe"),
        "a string id is echoed exactly"
    );
}

#[test]
fn tools_list_publishes_the_catalog_with_mcp_field_names() {
    let response = rpc(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert_eq!(response.status, 200);
    let result = response.result();
    let tools = result
        .get("tools")
        .and_then(JsonValue::as_array)
        .expect("tools array");
    assert_eq!(tools.len(), 16);
    assert_eq!(
        tools
            .first()
            .and_then(|tool| tool.get("name"))
            .and_then(JsonValue::as_str),
        Some("capabilities")
    );
    assert_eq!(
        tools
            .last()
            .and_then(|tool| tool.get("name"))
            .and_then(JsonValue::as_str),
        Some("experiment.run")
    );
    let service = service();
    for (tool, spec) in tools.iter().zip(service.tools()) {
        assert_eq!(
            tool.get("description").and_then(JsonValue::as_str),
            Some(spec.description)
        );
        assert_eq!(
            tool.get("inputSchema"),
            Some(&spec.input_schema),
            "{} publishes the catalog schema",
            spec.name
        );
        assert_eq!(
            tool.as_object().map(std::collections::BTreeMap::len),
            Some(3),
            "MCP tools carry name, description and inputSchema"
        );
    }
}

#[test]
fn ids_are_echoed_exactly_for_numbers_strings_and_zero() {
    for id in ["0", "42", "-7", "\"abc\"", "\"\""] {
        let request = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#);
        let response = rpc(&request);
        assert_eq!(response.status, 200, "{id}");
        let documented = parse(id.as_bytes(), &limits()).expect("id document");
        assert_eq!(
            response.json().get("id"),
            Some(&documented),
            "the id {id} is echoed byte-exactly"
        );
    }
}

// ---------------------------------------------------------------------------
// tools/call
// ---------------------------------------------------------------------------

#[test]
fn tools_call_carries_the_service_envelope_bytes_verbatim() {
    // A fresh direct service and a fresh server both start from empty state, so the same
    // call produces the same canonical envelope.
    let direct_bytes = service()
        .dispatch(&operator(), br#"{"op":"capabilities","input":{}}"#)
        .expect("direct dispatch");
    let (server, addr) = start();
    let response = send(
        addr,
        br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"capabilities","arguments":{}}}"#,
        Some(OPERATOR_TOKEN),
    );
    drop(server);
    assert_eq!(response.status, 200);
    let result = response.result();
    let (text, is_error) = text_of(&result);
    assert!(!is_error, "a successful operation is not an error");
    assert_eq!(
        text.as_bytes(),
        direct_bytes,
        "the text is the exact envelope"
    );
    // The text is the canonical envelope, not a paraphrase.
    let envelope = parse(text.as_bytes(), &limits()).expect("envelope");
    assert_eq!(
        envelope.get("op").and_then(JsonValue::as_str),
        Some("capabilities")
    );
}

#[test]
fn tools_call_reports_typed_errors_with_is_error_and_keeps_the_envelope() {
    let (server, addr) = start();
    let response = send(
        addr,
        br#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"session.inspect","arguments":{"session":99}}}"#,
        Some(OPERATOR_TOKEN),
    );
    assert_eq!(
        response.status, 200,
        "a typed error is not a transport error"
    );
    let result = response.result();
    let (text, is_error) = text_of(&result);
    assert!(is_error, "the typed error sets isError");
    let envelope = parse(text.as_bytes(), &limits()).expect("error envelope");
    assert_eq!(
        envelope.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1")
    );
    assert_eq!(
        envelope
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("not_found")
    );

    // The same invalid input over HTTP produces the same envelope bytes.
    let http = http_call(addr, r#"{"op":"session.inspect","input":{"session":99}}"#);
    assert_eq!(http, text.as_bytes());
    drop(server);
}

#[test]
fn an_unknown_tool_is_invalid_params() {
    let response = call_tool("not.a.tool", "{}");
    assert_eq!(response.status, 200);
    assert_eq!(response.rpc_code(), -32602);
    assert!(response.rpc_message().contains("unknown tool"));
}

#[test]
fn malformed_tools_call_params_are_invalid_params() {
    for request in [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":7,"arguments":{}}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"seconds"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"capabilities","arguments":[]}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"capabilities","arguments":{},"extra":1}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":[]}"#,
    ] {
        let response = rpc(request);
        assert_eq!(response.status, 200, "{request}");
        assert_eq!(response.rpc_code(), -32602, "{request}");
    }
}

#[test]
fn invalid_operation_arguments_stay_a_typed_envelope_not_a_protocol_error() {
    let response = call_tool("session.run", r#"{"session":1,"steps":0}"#);
    assert_eq!(response.status, 200);
    let result = response.result();
    let (text, is_error) = text_of(&result);
    assert!(is_error);
    let envelope = parse(text.as_bytes(), &limits()).expect("envelope");
    assert_eq!(
        envelope
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("invalid_request")
    );
}

// ---------------------------------------------------------------------------
// Protocol rejections
// ---------------------------------------------------------------------------

#[test]
fn a_batch_request_is_invalid_request_with_status_400() {
    let response = rpc(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
    assert_eq!(response.status, 400);
    assert_eq!(response.rpc_code(), -32600);
    assert!(response.rpc_message().contains("batch"));
}

#[test]
fn invalid_json_is_a_parse_error() {
    for body in ["{", "not json", "", r#"{"jsonrpc":"2.0","id":1,"method":"#] {
        let response = rpc(body);
        assert_eq!(response.status, 400, "{body:?}");
        assert_eq!(response.rpc_code(), -32700, "{body:?}");
    }
}

#[test]
fn an_unknown_method_is_method_not_found() {
    let response = rpc(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#);
    assert_eq!(response.status, 200);
    assert_eq!(response.rpc_code(), -32601);
    assert_eq!(
        response.json().get("id").and_then(JsonValue::as_i64),
        Some(3)
    );
}

#[test]
fn malformed_json_rpc_envelopes_are_invalid_request() {
    for body in [
        r#"{"id":1,"method":"ping"}"#,
        r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":1}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":7}"#,
        r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
        r#""ping""#,
        r#"42"#,
    ] {
        let response = rpc(body);
        assert_eq!(response.status, 400, "{body}");
        assert_eq!(response.rpc_code(), -32600, "{body}");
    }
}

#[test]
fn the_route_requires_a_bearer_token_and_a_json_content_type() {
    let (server, addr) = start();
    let unauthorized = send(addr, br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, None);
    assert_eq!(unauthorized.status, 401);
    assert_eq!(
        unauthorized
            .json()
            .get("schema")
            .and_then(JsonValue::as_str),
        Some("chronofork.error.v1"),
        "authentication failures use the transport envelope, not JSON-RPC"
    );
    let wrong_type = send_with_content_type(
        addr,
        br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
        Some(OPERATOR_TOKEN),
        Some("text/event-stream"),
    );
    assert_eq!(wrong_type.status, 415);
    drop(server);
}

#[test]
fn get_and_delete_are_not_allowed() {
    let (server, addr) = start();
    for method in ["GET", "DELETE", "PUT"] {
        let request = format!(
            "{method} /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {OPERATOR_TOKEN}\r\n\r\n"
        );
        let response = raw(addr, request.as_bytes());
        assert_eq!(response.status, 405, "{method}");
        assert_eq!(response.header("allow"), Some("POST"), "{method}");
        assert_eq!(
            response
                .json()
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(JsonValue::as_str),
            Some("method_not_allowed")
        );
    }
    drop(server);
}

#[test]
fn responses_are_json_and_never_an_event_stream() {
    let (server, addr) = start();
    for request in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"capabilities","arguments":{}}}"#,
    ] {
        let response = send(addr, request.as_bytes(), Some(OPERATOR_TOKEN));
        assert_eq!(
            response.header("content-type"),
            Some("application/json; charset=utf-8")
        );
        assert!(
            response.header("mcp-session-id").is_none(),
            "no session identifier is issued"
        );
        assert!(!response.body.starts_with(b"event:"));
    }
    drop(server);
}
