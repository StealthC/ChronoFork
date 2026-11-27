#![allow(clippy::expect_used)]

//! Wire-level acceptance tests for the bounded HTTP/1.1 server.
//!
//! Every test drives the server over a real socket with hand-written request bytes, so
//! the framing rules of architecture §4 (status, headers, one request per connection,
//! bounds) are exercised as a peer would see them, not through the typed client.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{
    AppService, Principal, Scope, ServiceConfig, ServiceLimits, TokenTable, parse_token_file,
};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};
use chronofork_transport::{ServerConfig, ServerHandle, TransportError, serve};

// ---------------------------------------------------------------------------
// Token fixtures
// ---------------------------------------------------------------------------

const OPERATOR_TOKEN: &str = "operator-token-0001";
const ADMIN_TOKEN: &str = "admin-token-000001";
const READER_TOKEN: &str = "reader-token-00001";
const WRITER_TOKEN: &str = "writer-token-00001";
const OTHER_TOKEN: &str = "other-token-000001";

const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]},
 {"name":"admin","token":"admin-token-000001","scopes":["admin"]},
 {"name":"reader","token":"reader-token-00001","scopes":["read"]},
 {"name":"writer","token":"writer-token-00001","scopes":["write"]},
 {"name":"other","token":"other-token-000001","scopes":["read","write"]}]}"#;

fn tokens() -> TokenTable {
    parse_token_file(TOKENS.as_bytes()).expect("token table")
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

// ---------------------------------------------------------------------------
// Service and server fixtures
// ---------------------------------------------------------------------------

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn service_with(
    factory: SyntheticBackendFactory,
    storage_root: Option<&Path>,
    limits: ServiceLimits,
) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(factory),
        storage_root: storage_root.map(Path::to_path_buf),
        artifacts_root: None,
        limits,
    })
    .expect("service")
}

fn start(service: AppService) -> ServerHandle {
    serve(
        ServerConfig {
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service),
    )
    .expect("server")
}

fn start_with(service: AppService, config: ServerConfig) -> ServerHandle {
    serve(
        ServerConfig {
            tokens: tokens(),
            ..config
        },
        Arc::new(service),
    )
    .expect("server")
}

fn base(handle: &ServerHandle) -> String {
    format!("http://{}", handle.local_addr())
}

// ---------------------------------------------------------------------------
// Raw request/response helpers
// ---------------------------------------------------------------------------

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
        parse(&self.body, &limits()).expect("canonical response body")
    }

    fn error_code(&self) -> String {
        self.json()
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str)
            .expect("error envelope code")
            .to_owned()
    }
}

fn send_raw(addr: SocketAddr, request: &[u8]) -> HttpResponse {
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
    let status_line = lines.next().expect("status line");
    let status = status_line
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

fn request(
    method: &str,
    target: &str,
    token: Option<&str>,
    content_type: Option<&str>,
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
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
    if let Some(body) = body {
        head.push_str("Content-Length: ");
        head.push_str(&body.len().to_string());
        head.push_str("\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut bytes = head.into_bytes();
    if let Some(body) = body {
        bytes.extend_from_slice(body);
    }
    bytes
}

fn ops_body(op: &str, input: &JsonValue) -> Vec<u8> {
    to_canonical_bytes(
        &JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]),
        &limits(),
    )
    .expect("request body")
}

fn ops(op: &str, input: &JsonValue) -> Vec<u8> {
    let body = ops_body(op, input);
    request(
        "POST",
        "/v1/ops",
        Some(OPERATOR_TOKEN),
        Some("application/json"),
        Some(&body),
    )
}

fn empty_input() -> JsonValue {
    JsonValue::Object(Default::default())
}

fn call(addr: SocketAddr, op: &str, text: &str) -> HttpResponse {
    let input = parse(text.as_bytes(), &limits()).expect("input JSON");
    send_raw(addr, &ops(op, &input))
}

/// Waits until the server stops accepting; a bounded poll instead of a fixed sleep.
fn wait_until_closed(addr: SocketAddr) {
    let mut closed = false;
    for _ in 0..300 {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_err() {
            closed = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(closed, "server is still accepting connections at {addr}");
}

// ---------------------------------------------------------------------------
// Temporary directories
// ---------------------------------------------------------------------------

static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let sequence = DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chronofork-transport-http-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temporary directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Routes, headers and the documented envelope
// ---------------------------------------------------------------------------

#[test]
fn health_is_unauthenticated_and_carries_exactly_the_documented_headers() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    let response = send_raw(addr, &request("GET", "/health", None, None, None));
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body,
        br#"{"schema":"chronofork.health.v1","status":"ok"}"#
    );
    assert_eq!(
        response.headers,
        vec![
            (
                "content-type".to_owned(),
                "application/json; charset=utf-8".to_owned()
            ),
            ("content-length".to_owned(), response.body.len().to_string()),
            ("connection".to_owned(), "close".to_owned()),
            ("cache-control".to_owned(), "no-store".to_owned()),
            ("x-content-type-options".to_owned(), "nosniff".to_owned()),
        ],
        "the response carries exactly the headers of architecture §4"
    );
    assert_eq!(
        response.header("content-length"),
        Some(response.body.len().to_string().as_str())
    );
    assert!(response.header("server").is_none());
    assert!(response.header("set-cookie").is_none());
}

#[test]
fn health_is_served_even_with_an_empty_token_table() {
    let server = serve(
        ServerConfig::default(),
        Arc::new(service_with(
            SyntheticBackendFactory::default(),
            None,
            limits(),
        )),
    )
    .expect("server");
    let response = send_raw(
        server.local_addr(),
        &request("GET", "/health", None, None, None),
    );
    assert_eq!(response.status, 200);
    let unauthorized = send_raw(
        server.local_addr(),
        &request("GET", "/v1/tools", None, None, None),
    );
    assert_eq!(unauthorized.status, 401);
    assert_eq!(unauthorized.error_code(), "unauthorized");
}

#[test]
fn tools_publish_the_twenty_operation_catalog_in_order() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let response = send_raw(
        server.local_addr(),
        &request("GET", "/v1/tools", Some(OPERATOR_TOKEN), None, None),
    );
    assert_eq!(response.status, 200);
    let document = response.json();
    assert_eq!(
        document.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.tools.v1")
    );
    let tools = document
        .get("tools")
        .and_then(JsonValue::as_array)
        .expect("tools array");
    assert_eq!(tools.len(), 20);
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool.get("name").and_then(JsonValue::as_str).expect("name"))
        .collect();
    assert_eq!(
        names,
        vec![
            "capabilities",
            "session.create",
            "session.list",
            "session.inspect",
            "session.run",
            "session.fork",
            "session.snapshot",
            "session.restore",
            "session.compare",
            "session.close",
            "session.reap",
            "session.read_memory",
            "session.cancel",
            "session.progress",
            "session.operation",
            "store.list",
            "store.load",
            "store.save",
            "store.delete",
            "experiment.run",
        ]
    );
    for tool in tools {
        assert!(
            tool.get("description")
                .and_then(JsonValue::as_str)
                .is_some_and(|description| !description.is_empty())
        );
        assert!(tool.get("mutating").and_then(JsonValue::as_bool).is_some());
        assert!(tool.get("scope").and_then(JsonValue::as_str).is_some());
        assert_eq!(
            tool.get("inputSchema")
                .and_then(|schema| schema.get("type"))
                .and_then(JsonValue::as_str),
            Some("object")
        );
    }
}

#[test]
fn the_tools_route_requires_the_read_scope() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    let reader = send_raw(
        addr,
        &request("GET", "/v1/tools", Some(READER_TOKEN), None, None),
    );
    assert_eq!(reader.status, 200);
    // A write-only principal carries neither `read` nor `admin`.
    let writer = send_raw(
        addr,
        &request("GET", "/v1/tools", Some(WRITER_TOKEN), None, None),
    );
    assert_eq!(writer.status, 403);
    assert_eq!(writer.error_code(), "forbidden");
}

#[test]
fn post_ops_returns_the_service_envelope_bytes_verbatim() {
    let service = service_with(SyntheticBackendFactory::default(), None, limits());
    let direct = service
        .dispatch(&operator(), &ops_body("capabilities", &empty_input()))
        .expect("direct dispatch");
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let response = send_raw(server.local_addr(), &ops("capabilities", &empty_input()));
    assert_eq!(response.status, 200);
    assert_eq!(response.body, direct, "HTTP returns the dispatch bytes");

    // A mutating operation and a typed operation error keep the same byte identity.
    let created = send_raw(server.local_addr(), &ops("session.create", &empty_input()));
    assert_eq!(created.status, 200);
    assert_eq!(
        created.json().get("op").and_then(JsonValue::as_str),
        Some("session.create")
    );
    let session = created
        .json()
        .get("result")
        .and_then(|result| result.get("session"))
        .and_then(JsonValue::as_u64)
        .expect("session id");
    assert_eq!(session, 1);
    let missing = call(server.local_addr(), "session.inspect", r#"{"session":99}"#);
    assert_eq!(missing.status, 404);
    assert_eq!(missing.error_code(), "not_found");
    assert_eq!(
        missing.json().get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1")
    );
}

// ---------------------------------------------------------------------------
// Rejection matrix
// ---------------------------------------------------------------------------

#[test]
fn malformed_requests_are_rejected_with_typed_envelopes() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    let valid = ops_body("capabilities", &empty_input());

    struct Case {
        name: &'static str,
        request: Vec<u8>,
        status: u16,
        code: &'static str,
        allow: Option<&'static str>,
    }
    let post = |body: &[u8], content_type: &str| {
        request(
            "POST",
            "/v1/ops",
            Some(OPERATOR_TOKEN),
            Some(content_type),
            Some(body),
        )
    };
    let cases = vec![
        Case {
            name: "garbage request line",
            request: b"GARBAGE\r\n\r\n".to_vec(),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "missing version",
            request: b"GET /health\r\n\r\n".to_vec(),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "unsupported version",
            request: b"GET /health HTTP/1.0\r\n\r\n".to_vec(),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "absolute-form target",
            request: b"GET http://example/ HTTP/1.1\r\n\r\n".to_vec(),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "header without colon",
            request: b"GET /health HTTP/1.1\r\nbroken\r\n\r\n".to_vec(),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "missing Content-Length",
            request: request(
                "POST",
                "/v1/ops",
                Some(OPERATOR_TOKEN),
                Some("application/json"),
                None,
            ),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "chunked body",
            request: b"POST /v1/ops HTTP/1.1\r\nAuthorization: Bearer operator-token-0001\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "duplicate Content-Length",
            request: b"POST /v1/ops HTTP/1.1\r\nAuthorization: Bearer operator-token-0001\r\nContent-Type: application/json\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
            status: 400,
            code: "bad_request",
            allow: None,
        },
        Case {
            name: "wrong content type",
            request: post(&valid, "text/plain"),
            status: 415,
            code: "unsupported_media_type",
            allow: None,
        },
        Case {
            name: "missing content type",
            request: request(
                "POST",
                "/v1/ops",
                Some(OPERATOR_TOKEN),
                None,
                Some(&valid),
            ),
            status: 415,
            code: "unsupported_media_type",
            allow: None,
        },
        Case {
            name: "unknown route",
            request: request("GET", "/v2/ops", Some(OPERATOR_TOKEN), None, None),
            status: 404,
            code: "not_found",
            allow: None,
        },
        Case {
            name: "query string",
            request: request("GET", "/health?verbose=1", None, None, None),
            status: 400,
            code: "invalid_request",
            allow: None,
        },
        Case {
            name: "wrong method on health",
            request: request("POST", "/health", None, None, None),
            status: 405,
            code: "method_not_allowed",
            allow: Some("GET"),
        },
        Case {
            name: "wrong method on ops",
            request: request("GET", "/v1/ops", Some(OPERATOR_TOKEN), None, None),
            status: 405,
            code: "method_not_allowed",
            allow: Some("POST"),
        },
        Case {
            name: "wrong method on tools",
            request: request("POST", "/v1/tools", Some(OPERATOR_TOKEN), None, None),
            status: 405,
            code: "method_not_allowed",
            allow: Some("GET"),
        },
        Case {
            name: "wrong method on mcp",
            request: request("GET", "/mcp", Some(OPERATOR_TOKEN), None, None),
            status: 405,
            code: "method_not_allowed",
            allow: Some("POST"),
        },
        Case {
            name: "missing token",
            request: request(
                "POST",
                "/v1/ops",
                None,
                Some("application/json"),
                Some(&valid),
            ),
            status: 401,
            code: "unauthorized",
            allow: None,
        },
        Case {
            name: "unknown token",
            request: request(
                "POST",
                "/v1/ops",
                Some("not-a-known-token"),
                Some("application/json"),
                Some(&valid),
            ),
            status: 401,
            code: "unauthorized",
            allow: None,
        },
        Case {
            name: "non-bearer scheme",
            request: b"POST /v1/ops HTTP/1.1\r\nAuthorization: Basic b3BlcmF0b3I=\r\nContent-Type: application/json\r\nContent-Length: 39\r\n\r\n{\"op\":\"capabilities\",\"input\":{}}".to_vec(),
            status: 401,
            code: "unauthorized",
            allow: None,
        },
    ];
    for case in cases {
        let response = send_raw(addr, &case.request);
        assert_eq!(response.status, case.status, "{}", case.name);
        assert_eq!(response.error_code(), case.code, "{}", case.name);
        assert_eq!(response.header("allow"), case.allow, "{}", case.name);
        assert_eq!(
            response.header("content-type"),
            Some("application/json; charset=utf-8"),
            "{}",
            case.name
        );
        assert_eq!(
            response.header("cache-control"),
            Some("no-store"),
            "{}",
            case.name
        );
        assert_eq!(
            response.header("x-content-type-options"),
            Some("nosniff"),
            "{}",
            case.name
        );
        assert_eq!(
            response.header("connection"),
            Some("close"),
            "{}",
            case.name
        );
        assert!(
            response
                .header("content-length")
                .is_some_and(|length| length.parse::<usize>().ok() == Some(response.body.len())),
            "{} declares its body length",
            case.name
        );
    }
}

#[test]
fn oversized_bodies_are_rejected_at_the_configured_bound() {
    let body = ops_body("capabilities", &empty_input());
    let bound = body.len();

    // Exactly at the bound the request is served.
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            max_request_bytes: bound,
            ..ServerConfig::default()
        },
    );
    let response = send_raw(
        server.local_addr(),
        &request(
            "POST",
            "/v1/ops",
            Some(OPERATOR_TOKEN),
            Some("application/json"),
            Some(&body),
        ),
    );
    assert_eq!(response.status, 200);
    drop(server);

    // One byte under the bound the same request is rejected as too large.
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            max_request_bytes: bound - 1,
            ..ServerConfig::default()
        },
    );
    let response = send_raw(
        server.local_addr(),
        &request(
            "POST",
            "/v1/ops",
            Some(OPERATOR_TOKEN),
            Some("application/json"),
            Some(&body),
        ),
    );
    assert_eq!(response.status, 413);
    assert_eq!(response.error_code(), "body_too_large");

    // A declared length beyond the bound is rejected before any body byte is read.
    let declared = format!(
        "POST /v1/ops HTTP/1.1\r\nAuthorization: Bearer {OPERATOR_TOKEN}\r\nContent-Type: application/json\r\nContent-Length: 1048576\r\n\r\n"
    );
    let response = send_raw(server.local_addr(), declared.as_bytes());
    assert_eq!(response.status, 413);
    assert_eq!(response.error_code(), "body_too_large");
}

#[test]
fn request_line_header_and_head_bounds_are_enforced() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();

    // Request line over 8192 bytes.
    let mut long_line = b"GET /".to_vec();
    long_line.extend(std::iter::repeat_n(b'a', 8192));
    long_line.extend_from_slice(b" HTTP/1.1\r\n\r\n");
    let response = send_raw(addr, &long_line);
    assert_eq!(response.status, 400);
    assert_eq!(response.error_code(), "bad_request");

    // One header line over 8192 bytes.
    let mut long_header = b"GET /health HTTP/1.1\r\nX-Pad: ".to_vec();
    long_header.extend(std::iter::repeat_n(b'a', 8192));
    long_header.extend_from_slice(b"\r\n\r\n");
    let response = send_raw(addr, &long_header);
    assert_eq!(response.status, 400);
    assert_eq!(response.error_code(), "bad_request");

    // More than 64 headers.
    let mut many = b"GET /health HTTP/1.1\r\n".to_vec();
    for index in 0..65 {
        many.extend_from_slice(format!("X-{index}: 0\r\n").as_bytes());
    }
    many.extend_from_slice(b"\r\n");
    let response = send_raw(addr, &many);
    assert_eq!(response.status, 400);
    assert_eq!(response.error_code(), "bad_request");

    // Exactly 64 headers is accepted.
    let mut allowed = b"GET /health HTTP/1.1\r\n".to_vec();
    for index in 0..64 {
        allowed.extend_from_slice(format!("X-{index}: 0\r\n").as_bytes());
    }
    allowed.extend_from_slice(b"\r\n");
    let response = send_raw(addr, &allowed);
    assert_eq!(response.status, 200);

    // A head that never terminates and exceeds the whole head bound.
    let mut unterminated = b"GET /health HTTP/1.1\r\n".to_vec();
    for index in 0..70 {
        unterminated.extend_from_slice(format!("X-{index}: ").as_bytes());
        unterminated.extend(std::iter::repeat_n(b'a', 8000));
        unterminated.extend_from_slice(b"\r\n");
    }
    let response = send_raw(addr, &unterminated);
    assert_eq!(response.status, 400);
    assert_eq!(response.error_code(), "bad_request");
}

#[test]
fn one_request_is_served_per_connection() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    let mut pipelined = request("GET", "/health", None, None, None);
    pipelined.extend_from_slice(&request("GET", "/health", None, None, None));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream.write_all(&pipelined).expect("write");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read to end");
    let response = parse_response(&bytes);
    assert_eq!(response.status, 200);
    // The connection carries exactly one response and is then closed.
    assert_eq!(
        bytes
            .windows(12)
            .filter(|window| *window == b"HTTP/1.1 200")
            .count(),
        1
    );
    assert_eq!(
        response.header("content-length"),
        Some(response.body.len().to_string().as_str())
    );
}

// ---------------------------------------------------------------------------
// Authentication and ownership
// ---------------------------------------------------------------------------

#[test]
fn principal_scopes_and_ownership_are_enforced() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();

    // A read-only principal cannot create a session.
    let forbidden = send_raw(
        addr,
        &request(
            "POST",
            "/v1/ops",
            Some(READER_TOKEN),
            Some("application/json"),
            Some(&ops_body("session.create", &empty_input())),
        ),
    );
    assert_eq!(forbidden.status, 403);
    assert_eq!(forbidden.error_code(), "forbidden");

    // A write-only principal cannot list them either.
    let forbidden = send_raw(
        addr,
        &request(
            "POST",
            "/v1/ops",
            Some(WRITER_TOKEN),
            Some("application/json"),
            Some(&ops_body("session.list", &empty_input())),
        ),
    );
    assert_eq!(forbidden.status, 403);
    assert_eq!(forbidden.error_code(), "forbidden");

    // The operator creates a session; another principal may not observe it.
    let created = send_raw(
        addr,
        &request(
            "POST",
            "/v1/ops",
            Some(OPERATOR_TOKEN),
            Some("application/json"),
            Some(&ops_body("session.create", &empty_input())),
        ),
    );
    assert_eq!(created.status, 200);
    let session = created
        .json()
        .get("result")
        .and_then(|result| result.get("session"))
        .and_then(JsonValue::as_u64)
        .expect("session id");
    let inspect = to_canonical_bytes(
        &JsonValue::object([(
            "session",
            JsonValue::int(i64::try_from(session).expect("session id")),
        )]),
        &limits(),
    )
    .expect("inspect input");
    let input = parse(&inspect, &limits()).expect("parsed input");
    let body = ops_body("session.inspect", &input);

    let foreign = send_raw(
        addr,
        &request(
            "POST",
            "/v1/ops",
            Some(OTHER_TOKEN),
            Some("application/json"),
            Some(&body),
        ),
    );
    assert_eq!(foreign.status, 403);
    assert_eq!(foreign.error_code(), "forbidden");
    assert!(
        !foreign.body.windows(8).any(|window| window == b"operator"),
        "the error envelope never names another principal"
    );

    // The administrator may observe any session.
    let admin_body = body;
    let admin = send_raw(
        addr,
        &request(
            "POST",
            "/v1/ops",
            Some(ADMIN_TOKEN),
            Some("application/json"),
            Some(&admin_body),
        ),
    );
    assert_eq!(admin.status, 200);
    assert_eq!(
        admin.json().get("op").and_then(JsonValue::as_str),
        Some("session.inspect")
    );
}

#[test]
fn every_documented_error_status_is_produced_by_an_operation() {
    // 400 invalid_request.
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let response = call(server.local_addr(), "session.inspect", r#"{"session":"x"}"#);
    assert_eq!(response.status, 400);
    assert_eq!(response.error_code(), "invalid_request");
    drop(server);

    // 400 unknown_op.
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let response = call(server.local_addr(), "not.an.op", "{}");
    assert_eq!(response.status, 400);
    assert_eq!(response.error_code(), "unknown_op");
    drop(server);

    // 400 limit_exceeded.
    let mut restricted = limits();
    restricted.max_sessions = 1;
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        restricted,
    ));
    let first = call(server.local_addr(), "session.create", "{}");
    assert_eq!(first.status, 200);
    let second = call(server.local_addr(), "session.create", "{}");
    assert_eq!(second.status, 400);
    assert_eq!(second.error_code(), "limit_exceeded");
    drop(server);

    // 409 closed.
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    assert_eq!(
        call(server.local_addr(), "session.create", "{}").status,
        200
    );
    assert_eq!(
        call(server.local_addr(), "session.close", r#"{"session":1}"#).status,
        200
    );
    let closed = call(server.local_addr(), "session.inspect", r#"{"session":1}"#);
    assert_eq!(closed.status, 409);
    assert_eq!(closed.error_code(), "closed");
    drop(server);

    // 409 halted.
    let halting = SyntheticBackendFactory::new(SyntheticProgram::new(vec![0xff]).expect("program"));
    let server = start(service_with(halting, None, limits()));
    assert_eq!(
        call(server.local_addr(), "session.create", "{}").status,
        200
    );
    let stopped = call(
        server.local_addr(),
        "session.run",
        r#"{"session":1,"steps":1}"#,
    );
    assert_eq!(stopped.status, 200);
    let halted = call(
        server.local_addr(),
        "session.run",
        r#"{"session":1,"steps":1}"#,
    );
    assert_eq!(halted.status, 409);
    assert_eq!(halted.error_code(), "halted");
    drop(server);

    // 500 backend_error.
    let broken = SyntheticBackendFactory::new(SyntheticProgram::new(vec![0x42]).expect("program"));
    let server = start(service_with(broken, None, limits()));
    assert_eq!(
        call(server.local_addr(), "session.create", "{}").status,
        200
    );
    let rejected = call(
        server.local_addr(),
        "session.run",
        r#"{"session":1,"steps":1}"#,
    );
    assert_eq!(rejected.status, 500);
    assert_eq!(rejected.error_code(), "backend_error");
    drop(server);

    // 503 not_configured: no storage root.
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let unconfigured = call(server.local_addr(), "store.list", "{}");
    assert_eq!(unconfigured.status, 503);
    assert_eq!(unconfigured.error_code(), "not_configured");
    drop(server);

    // 409 conflict: deleting a durable session that still holds a snapshot.
    let directory = TempDir::new("conflict");
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        Some(directory.path()),
        limits(),
    ));
    let saved = call(server.local_addr(), "store.save", r#"{"steps":2}"#);
    assert_eq!(saved.status, 200);
    let conflicted = call(server.local_addr(), "store.delete", r#"{"session_key":1}"#);
    assert_eq!(conflicted.status, 409);
    assert_eq!(conflicted.error_code(), "conflict");
    drop(server);

    // 500 storage_error: a corrupt committed checkpoint is never loaded.
    let directory = TempDir::new("storage-error");
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        Some(directory.path()),
        limits(),
    ));
    assert_eq!(
        call(server.local_addr(), "store.save", r#"{"steps":1}"#).status,
        200
    );
    let checkpoint = directory
        .path()
        .join("v1/sessions/session-1/snapshots/snapshot-1/checkpoint.cfsnap");
    fs::write(&checkpoint, b"corrupt").expect("corrupt checkpoint");
    let corrupt = call(
        server.local_addr(),
        "store.load",
        r#"{"session_key":1,"snapshot_key":1}"#,
    );
    assert_eq!(corrupt.status, 500);
    assert_eq!(corrupt.error_code(), "storage_error");
}

// ---------------------------------------------------------------------------
// Bounds, workers and shutdown
// ---------------------------------------------------------------------------

#[test]
fn max_requests_stops_the_server_after_the_documented_count() {
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            max_requests: Some(2),
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();
    assert_eq!(
        send_raw(addr, &request("GET", "/health", None, None, None)).status,
        200
    );
    assert_eq!(
        send_raw(addr, &request("GET", "/health", None, None, None)).status,
        200
    );
    wait_until_closed(addr);
    server.shutdown().expect("shutdown");
}

#[test]
fn shutdown_closes_the_listener_and_joins_the_threads() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    assert_eq!(
        send_raw(addr, &request("GET", "/health", None, None, None)).status,
        200
    );
    server.shutdown().expect("shutdown");
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err(),
        "the listener is closed after shutdown"
    );
}

#[test]
fn wait_blocks_until_the_documented_request_count_is_served() {
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            max_requests: Some(2),
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();
    let (signalled, wait_outcome) = mpsc::channel();
    let waiting = thread::spawn(move || {
        let outcome = server.wait();
        // Signal after `wait` returned; the channel carries the result too.
        let _ = signalled.send(outcome.is_ok());
    });

    // The first request is served normally and must not end the wait.
    let first = send_raw(addr, &request("GET", "/health", None, None, None));
    assert_eq!(first.status, 200);
    assert!(
        wait_outcome
            .recv_timeout(Duration::from_millis(200))
            .is_err(),
        "wait returned before the request limit was reached"
    );

    // The second request is served by the still-running server, then waits ends.
    let second = send_raw(addr, &request("GET", "/health", None, None, None));
    assert_eq!(second.status, 200);
    assert_eq!(
        wait_outcome.recv_timeout(Duration::from_secs(5)),
        Ok(true),
        "wait returns Ok once the request limit is reached"
    );
    waiting.join().expect("wait thread");
    wait_until_closed(addr);
}

#[test]
fn requests_served_after_wait_started_are_handled() {
    // `wait` must not change the server's behaviour: requests that arrive after it began
    // blocking are served normally until the configured limit is reached.
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            max_requests: Some(3),
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();
    let (signalled, wait_outcome) = mpsc::channel();
    let waiting = thread::spawn(move || {
        let outcome = server.wait();
        let _ = signalled.send(outcome.is_ok());
    });

    for served in 1..=3 {
        let response = send_raw(addr, &request("GET", "/health", None, None, None));
        assert_eq!(
            response.status, 200,
            "request {served} is served while waiting"
        );
        if served < 3 {
            assert!(
                wait_outcome
                    .recv_timeout(Duration::from_millis(150))
                    .is_err(),
                "wait returned after request {served} although the limit is 3"
            );
        }
    }
    assert_eq!(
        wait_outcome.recv_timeout(Duration::from_secs(5)),
        Ok(true),
        "wait returns Ok after the third request"
    );
    waiting.join().expect("wait thread");
    wait_until_closed(addr);
}

#[test]
fn explicit_shutdown_still_stops_a_server_without_a_request_limit() {
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            max_requests: None,
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();
    assert_eq!(
        send_raw(addr, &request("GET", "/health", None, None, None)).status,
        200
    );
    server.shutdown().expect("shutdown");
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err(),
        "explicit shutdown closes the listener and joins the threads"
    );
}

#[test]
fn a_zero_worker_configuration_is_rejected() {
    let error = serve(
        ServerConfig {
            workers: 0,
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service_with(
            SyntheticBackendFactory::default(),
            None,
            limits(),
        )),
    )
    .expect_err("zero workers cannot serve");
    assert!(matches!(error, TransportError::Protocol(_)));
}

#[test]
fn a_single_worker_configuration_is_rejected() {
    // ADR-0016 §1/F4: the pool must reserve at least one worker for the control path, so
    // a single-worker daemon cannot keep `cancel`/`list`/`progress` executable while a
    // long run is in flight. The refusal is a typed startup error.
    let error = serve(
        ServerConfig {
            workers: 1,
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service_with(
            SyntheticBackendFactory::default(),
            None,
            limits(),
        )),
    )
    .expect_err("a single worker cannot reserve the control path");
    match error {
        TransportError::Protocol(message) => {
            assert!(
                message.contains("two workers"),
                "the refusal names the bound: {message}"
            );
        }
        other => unreachable!("expected a typed startup error, got {other:?}"),
    }
}

#[test]
fn the_worker_pool_serves_concurrent_requests_within_max_requests() {
    let total = 16_u64;
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            workers: 4,
            max_requests: Some(total),
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();
    let mut handles = Vec::new();
    for _ in 0..8 {
        handles.push(thread::spawn(move || {
            for _ in 0..2 {
                let response = send_raw(addr, &request("GET", "/health", None, None, None));
                assert_eq!(response.status, 200);
            }
        }));
    }
    for handle in handles {
        handle.join().expect("client thread");
    }
    wait_until_closed(addr);
    server.shutdown().expect("shutdown");
}

#[test]
fn a_small_pool_queues_concurrent_clients_and_serves_each_one() {
    // More clients than workers must queue in the bounded accept queue instead of being
    // dropped or served by an unbounded number of threads. Two is the smallest accepted
    // pool: one worker is reserved for the control path (ADR-0016 §1).
    let total = 8_u64;
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            workers: 2,
            max_requests: Some(total),
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();
    let mut handles = Vec::new();
    for _ in 0..total {
        handles.push(thread::spawn(move || {
            let response = send_raw(addr, &request("GET", "/health", None, None, None));
            assert_eq!(response.status, 200);
        }));
    }
    for handle in handles {
        handle.join().expect("client thread");
    }
    wait_until_closed(addr);
    server.shutdown().expect("shutdown");
}

#[test]
fn a_request_split_across_segments_is_reassembled() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    let body = ops_body("capabilities", &empty_input());
    let head = format!(
        "POST /v1/ops HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {OPERATOR_TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream.write_all(head.as_bytes()).expect("write head");
    stream.flush().expect("flush head");
    thread::sleep(Duration::from_millis(20));
    stream.write_all(&body).expect("write body");
    stream.flush().expect("flush body");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read response");
    let response = parse_response(&bytes);
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json().get("op").and_then(JsonValue::as_str),
        Some("capabilities")
    );
}

#[test]
fn the_effective_address_is_reported_and_reachable() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    assert_ne!(addr.port(), 0, "an ephemeral port is resolved on bind");
    assert!(base(&server).starts_with("http://127.0.0.1:"));
}

// ---------------------------------------------------------------------------
// ADR-0014 §1.5/§1.6: Origin policy and the absolute pre-auth deadline
// ---------------------------------------------------------------------------

/// Builds one request that carries an `Origin` header.
///
/// The server treats any `Origin`-bearing request as browser-originated and refuses it
/// (v1 has no browser-facing surface), so the body and token are deliberately valid
/// where present: only the `Origin` header decides the outcome.
fn request_with_origin(
    method: &str,
    target: &str,
    token: Option<&str>,
    content_type: Option<&str>,
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut head =
        format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://evil.example\r\n");
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
    if let Some(body) = body {
        head.push_str("Content-Length: ");
        head.push_str(&body.len().to_string());
        head.push_str("\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut bytes = head.into_bytes();
    if let Some(body) = body {
        bytes.extend_from_slice(body);
    }
    bytes
}

#[test]
fn an_origin_bearing_request_is_refused_before_authentication() {
    let server = start(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let addr = server.local_addr();
    let body = ops_body("capabilities", &empty_input());
    let mcp_body = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "health",
            request_with_origin("GET", "/health", None, None, None),
        ),
        (
            "tools",
            request_with_origin("GET", "/v1/tools", Some(OPERATOR_TOKEN), None, None),
        ),
        (
            "ops with a valid token",
            request_with_origin(
                "POST",
                "/v1/ops",
                Some(OPERATOR_TOKEN),
                Some("application/json"),
                Some(&body),
            ),
        ),
        (
            "ops with an invalid token",
            request_with_origin(
                "POST",
                "/v1/ops",
                Some("not-a-known-token"),
                Some("application/json"),
                Some(&body),
            ),
        ),
        (
            "mcp",
            request_with_origin(
                "POST",
                "/mcp",
                Some(OPERATOR_TOKEN),
                Some("application/json"),
                Some(mcp_body),
            ),
        ),
    ];
    for (name, request) in cases {
        let response = send_raw(addr, &request);
        assert_eq!(response.status, 403, "{name}");
        assert_eq!(response.error_code(), "forbidden", "{name}");
        // The refusal is the Origin rule itself, not a 401/200 that happens to look
        // like a rejection: it is emitted before the route and the bearer token.
        assert!(
            response
                .json()
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(JsonValue::as_str)
                .is_some_and(|message| message.contains("Origin")),
            "{name} names the Origin policy"
        );
    }

    // A request without `Origin` is unaffected: the ordinary surfaces stay reachable.
    let health = send_raw(addr, &request("GET", "/health", None, None, None));
    assert_eq!(health.status, 200);
    let capabilities = call(addr, "capabilities", "{}");
    assert_eq!(capabilities.status, 200);
}

#[test]
fn a_trickled_request_head_cannot_hold_a_worker_past_the_pre_auth_deadline() {
    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            io_timeout: Duration::from_secs(5),
            pre_auth_timeout: Duration::from_millis(300),
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();

    // More partial connections than workers (the default is four), so the workers would
    // stay occupied for the socket timeout without the absolute pre-auth deadline.
    let mut slow = Vec::new();
    for _ in 0..8 {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .expect("connect a slow client");
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("slow-client write timeout");
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n")
            .expect("write a partial head");
        stream.flush().expect("flush the partial head");
        slow.push(stream);
    }

    // Every trickled connection is closed once its accept-relative budget expires, which
    // releases the workers. A later request that queued past its own budget is refused by
    // the same rule, so recovery is checked after the release rather than concurrently.
    let deadline = Instant::now() + Duration::from_secs(5);
    for stream in &mut slow {
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("slow-client read timeout");
        let mut byte = [0_u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => break,
                // A rejection response would also end the connection; either way the
                // worker is released.
                Ok(_) => break,
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock
                        || error.kind() == ErrorKind::TimedOut =>
                {
                    assert!(
                        Instant::now() < deadline,
                        "a trickled connection is still open past the pre-auth deadline"
                    );
                }
                Err(_) => break,
            }
        }
    }

    // The pool recovered: a fresh legitimate request is served in the documented bound.
    let health = send_raw(addr, &request("GET", "/health", None, None, None));
    assert_eq!(health.status, 200);
}

#[test]
fn serve_refuses_a_non_loopback_bind_before_binding() {
    // Reserve an ephemeral port and release it, so the same address can be probed after
    // the refused start.
    let probe = TcpListener::bind("127.0.0.1:0").expect("reserve a port");
    let port = probe.local_addr().expect("reserved address").port();
    drop(probe);

    let config = ServerConfig {
        bind: SocketAddr::from(([0, 0, 0, 0], port)),
        tokens: tokens(),
        ..ServerConfig::default()
    };
    let service = Arc::new(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let refused = serve(config, service);
    assert!(refused.is_err(), "a non-loopback bind must be refused");
    let message = match refused {
        Ok(_) => String::new(),
        Err(TransportError::Protocol(message)) => message,
        Err(other) => format!("unexpected error variant: {other:?}"),
    };
    assert!(
        message.contains("non-loopback") && message.contains("ADR-0014"),
        "the refusal is typed and names the policy: {message}"
    );

    // Nothing was left listening: the address is free to bind now.
    let listener = TcpListener::bind(("0.0.0.0", port))
        .expect("the refused server never bound the non-loopback address");
    drop(listener);
}

#[test]
fn the_pre_auth_deadline_must_be_positive() {
    let config = ServerConfig {
        pre_auth_timeout: Duration::ZERO,
        tokens: tokens(),
        ..ServerConfig::default()
    };
    let service = Arc::new(service_with(
        SyntheticBackendFactory::default(),
        None,
        limits(),
    ));
    let refused = serve(config, service);
    assert!(refused.is_err(), "a zero pre-auth deadline must be refused");
    let message = match refused {
        Ok(_) => String::new(),
        Err(TransportError::Protocol(message)) => message,
        Err(other) => format!("unexpected error variant: {other:?}"),
    };
    assert!(message.contains("pre-auth deadline"), "{message}");
}

#[test]
fn queued_connections_expire_against_the_accept_deadline() {
    // The advertised contract is one absolute pre-auth budget measured from `accept`,
    // including queue residence. A per-dispatch deadline would add one full budget per
    // wave (12 accepted sockets / 4 workers = 3 waves), which is far outside the
    // documented scheduling tolerance below.
    const PRE_AUTH: Duration = Duration::from_millis(400);
    /// Scheduling and thread-start tolerance around the configured budget. It is larger
    /// than the observed jitter (well under 100 ms) and far smaller than the ~800 ms a
    /// per-dispatch reset would add for the queued waves.
    const TOLERANCE: Duration = Duration::from_millis(500);
    /// Hard ceiling for one closure poll, so a regression fails instead of hanging.
    const CLOSE_DEADLINE: Duration = Duration::from_secs(5);

    let server = start_with(
        service_with(SyntheticBackendFactory::default(), None, limits()),
        ServerConfig {
            io_timeout: Duration::from_secs(5),
            pre_auth_timeout: PRE_AUTH,
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();

    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let mut readers = Vec::new();
    let mut tricklers = Vec::new();
    // More accepted connections than workers: the first four are dispatched, the rest
    // wait in the bounded userspace queue and must still expire against their accept
    // instant.
    for _ in 0..12 {
        let stream = TcpStream::connect_timeout(&addr, CLOSE_DEADLINE).expect("connect");
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .expect("write timeout");
        let mut partial = stream.try_clone().expect("clone partial writer");
        partial
            .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n")
            .expect("write a partial head");
        partial.flush().expect("flush the partial head");

        // Keep trickling one byte for longer than the budget: a per-read timeout or a
        // dispatch-relative budget would keep the connection alive.
        let mut trickle = stream.try_clone().expect("clone trickle writer");
        let stop = Arc::clone(&stop);
        tricklers.push(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if trickle.write_all(b"X").is_err() || trickle.flush().is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }));
        readers.push(stream);
    }

    let hard_deadline = started + PRE_AUTH + TOLERANCE + Duration::from_secs(1);
    let mut closed = 0;
    for stream in &mut readers {
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("read timeout");
        let mut byte = [0_u8; 1];
        loop {
            match stream.read(&mut byte) {
                // EOF: the deadline closed the connection without a response.
                Ok(0) => {
                    closed += 1;
                    break;
                }
                // A rejection response also ends the connection and releases the worker.
                Ok(_) => {
                    closed += 1;
                    break;
                }
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock
                        || error.kind() == ErrorKind::TimedOut =>
                {
                    assert!(
                        Instant::now() < hard_deadline,
                        "a queued trickling connection stayed open past \
                         {PRE_AUTH:?} + {TOLERANCE:?} tolerance"
                    );
                }
                Err(_) => {
                    closed += 1;
                    break;
                }
            }
        }
    }
    let elapsed = started.elapsed();
    stop.store(true, Ordering::SeqCst);
    for trickler in tricklers {
        let _ = trickler.join();
    }

    assert_eq!(closed, 12, "every queued connection must be closed");
    assert!(
        elapsed >= PRE_AUTH / 2,
        "connections closed before the configured budget could have elapsed: {elapsed:?}"
    );
    assert!(
        elapsed <= PRE_AUTH + TOLERANCE,
        "queued sockets must share the accept-relative {PRE_AUTH:?} budget; they closed \
         after {elapsed:?} (tolerance {TOLERANCE:?})"
    );

    // The pool recovers: /health is served after the pre-auth release, within the same
    // configured budget plus the documented tolerance.
    let health = send_raw(addr, &request("GET", "/health", None, None, None));
    assert_eq!(
        health.status, 200,
        "/health must be served once the pre-auth budget releases the workers"
    );
    assert!(
        started.elapsed() <= PRE_AUTH + TOLERANCE + Duration::from_secs(1),
        "/health was not served within the documented recovery bound"
    );
}
