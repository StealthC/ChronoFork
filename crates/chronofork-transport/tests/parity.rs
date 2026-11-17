#![allow(clippy::expect_used)]

//! Direct/HTTP/MCP parity harness (architecture §11).
//!
//! One scenario — create, inspect, run, snapshot, run, restore, fork, compare,
//! read-memory, list, close, experiment, store save/list/load/continue/delete and the
//! error paths — runs on three independent surfaces: a direct [`AppService`], the HTTP
//! adapter and the MCP adapter. Every step's canonical bytes must be identical across the
//! three, with no normalization: a difference in one field, one key order or one message
//! is a failure. The MCP text content is compared after JSON-string decoding, which is
//! exactly what the adapter carries.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{
    AppService, Principal, Scope, ServiceConfig, ServiceLimits, TokenTable, parse_token_file,
};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{
    HttpClient, McpClient, ServerConfig, ServerHandle, TransportError, serve,
};

const OPERATOR_TOKEN: &str = "operator-token-0001";
const OTHER_TOKEN: &str = "other-token-000001";

const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]},
 {"name":"other","token":"other-token-000001","scopes":["read","write"]},
 {"name":"admin","token":"admin-token-000001","scopes":["admin"]}]}"#;

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn tokens() -> TokenTable {
    parse_token_file(TOKENS.as_bytes()).expect("token table")
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn other() -> Principal {
    Principal::new("other", [Scope::Read, Scope::Write])
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::object(entries)
}

fn empty() -> JsonValue {
    JsonValue::Object(Default::default())
}

fn number(value: &JsonValue, name: &str) -> u64 {
    value
        .get(name)
        .and_then(JsonValue::as_u64)
        .unwrap_or_else(|| unreachable!("missing unsigned field `{name}` in {value:?}"))
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
            "chronofork-transport-parity-{label}-{}-{sequence}",
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

fn configured_service(directory: &TempDir) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: Some(directory.path().join("store")),
        artifacts_root: Some(directory.path().join("artifacts")),
        limits: limits(),
    })
    .expect("service")
}

// ---------------------------------------------------------------------------
// Surfaces
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Outcome {
    bytes: Vec<u8>,
    is_error: bool,
}

impl Outcome {
    fn result(&self) -> JsonValue {
        assert!(!self.is_error, "expected a result, got an error envelope");
        parse(&self.bytes, &limits()).expect("result envelope")
    }

    fn envelope(&self) -> JsonValue {
        parse(&self.bytes, &limits()).expect("canonical envelope")
    }
}

trait Surface {
    fn invoke(&self, op: &str, input: &JsonValue) -> Outcome;
}

struct DirectSurface {
    service: Arc<AppService>,
    principal: Principal,
}

impl DirectSurface {
    fn new(service: Arc<AppService>, principal: Principal) -> Self {
        Self { service, principal }
    }
}

impl Surface for DirectSurface {
    fn invoke(&self, op: &str, input: &JsonValue) -> Outcome {
        let request = object([("op", JsonValue::string(op)), ("input", input.clone())]);
        let request = to_canonical_bytes(&request, &limits()).expect("request document");
        match self.service.dispatch(&self.principal, &request) {
            Ok(bytes) => Outcome {
                bytes,
                is_error: false,
            },
            Err(error) => Outcome {
                bytes: error.to_canonical_bytes(&limits()).expect("error envelope"),
                is_error: true,
            },
        }
    }
}

struct HttpSurface {
    client: HttpClient,
}

impl Surface for HttpSurface {
    fn invoke(&self, op: &str, input: &JsonValue) -> Outcome {
        match self.client.call(op, input) {
            Ok(bytes) => Outcome {
                bytes,
                is_error: false,
            },
            Err(TransportError::Http { body, .. }) => Outcome {
                bytes: body,
                is_error: true,
            },
            Err(error) => unreachable!("HTTP transport failure on a live server: {error:?}"),
        }
    }
}

struct McpSurface {
    client: McpClient,
}

impl Surface for McpSurface {
    fn invoke(&self, op: &str, input: &JsonValue) -> Outcome {
        match self.client.call_tool(op, input) {
            Ok(result) => Outcome {
                bytes: result.text.into_bytes(),
                is_error: result.is_error,
            },
            Err(error) => unreachable!("MCP transport failure on a live server: {error:?}"),
        }
    }
}

fn start_http(service: AppService) -> (ServerHandle, HttpClient) {
    let server = serve(
        ServerConfig {
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service),
    )
    .expect("http server");
    let client = HttpClient::new(format!("http://{}", server.local_addr()), OPERATOR_TOKEN);
    (server, client)
}

fn start_mcp(service: AppService) -> (ServerHandle, McpClient, SocketAddr) {
    let server = serve(
        ServerConfig {
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service),
    )
    .expect("mcp server");
    let addr = server.local_addr();
    let client = McpClient::new(format!("http://{addr}"), OPERATOR_TOKEN);
    (server, client, addr)
}

// ---------------------------------------------------------------------------
// The scenario
// ---------------------------------------------------------------------------

/// One comparable step: its label, canonical bytes and error flag.
#[derive(Debug)]
struct Step {
    label: String,
    bytes: Vec<u8>,
    is_error: bool,
}

struct Run<'a> {
    surface: &'a dyn Surface,
    steps: Vec<Step>,
}

impl Run<'_> {
    fn new(surface: &dyn Surface) -> Run<'_> {
        Run {
            surface,
            steps: Vec::new(),
        }
    }

    fn call(&mut self, label: &str, op: &str, input: JsonValue) -> Outcome {
        let outcome = self.surface.invoke(op, &input);
        self.steps.push(Step {
            label: label.to_owned(),
            bytes: outcome.bytes.clone(),
            is_error: outcome.is_error,
        });
        outcome
    }

    fn ok(&mut self, label: &str, op: &str, input: JsonValue) -> JsonValue {
        let outcome = self.call(label, op, input);
        outcome.result()
    }

    fn failed(&mut self, label: &str, op: &str, input: JsonValue) -> JsonValue {
        let outcome = self.call(label, op, input);
        assert!(outcome.is_error, "{label} must fail");
        parse(&outcome.bytes, &limits()).expect("error envelope")
    }
}

fn scenario(surface: &dyn Surface) -> Vec<Step> {
    let mut run = Run::new(surface);

    let capabilities = run.ok("capabilities", "capabilities", empty());
    assert_eq!(
        capabilities
            .get("result")
            .and_then(|result| result.get("schema"))
            .and_then(JsonValue::as_str),
        Some("chronofork.capabilities.v1")
    );

    let created = run.ok("session.create", "session.create", empty());
    let session = number(created.get("result").expect("result"), "session");

    run.ok(
        "session.inspect",
        "session.inspect",
        object([("session", JsonValue::int(cast(session)))]),
    );
    run.ok(
        "session.run",
        "session.run",
        object([
            ("session", JsonValue::int(cast(session))),
            ("steps", JsonValue::int(4)),
            ("buttons", JsonValue::int(1)),
        ]),
    );
    let snapshot = run.ok(
        "session.snapshot",
        "session.snapshot",
        object([
            ("session", JsonValue::int(cast(session))),
            ("name", JsonValue::string("checkpoint-a")),
        ]),
    );
    let snapshot = number(snapshot.get("result").expect("result"), "snapshot");
    run.ok(
        "session.run#2",
        "session.run",
        object([
            ("session", JsonValue::int(cast(session))),
            ("steps", JsonValue::int(3)),
        ]),
    );
    run.ok(
        "session.restore",
        "session.restore",
        object([
            ("session", JsonValue::int(cast(session))),
            ("snapshot", JsonValue::int(cast(snapshot))),
        ]),
    );
    let forked = run.ok(
        "session.fork",
        "session.fork",
        object([("session", JsonValue::int(cast(session)))]),
    );
    let fork = number(forked.get("result").expect("result"), "session");
    run.ok(
        "session.compare",
        "session.compare",
        object([
            ("left", JsonValue::int(cast(session))),
            ("right", JsonValue::int(cast(fork))),
        ]),
    );
    run.ok(
        "session.read_memory",
        "session.read_memory",
        object([
            ("session", JsonValue::int(cast(session))),
            ("address", JsonValue::int(0)),
            ("length", JsonValue::int(8)),
        ]),
    );
    run.ok("session.list", "session.list", empty());

    // Durable store: save a new session, list, load, continue it, then delete snapshot,
    // snapshot and session in that order (no cascade, so each step is a distinct proof).
    let saved = run.ok(
        "store.save",
        "store.save",
        object([("steps", JsonValue::int(5))]),
    );
    let key = number(saved.get("result").expect("result"), "session_key");
    run.ok("store.list", "store.list", empty());
    run.ok(
        "store.load",
        "store.load",
        object([
            ("session_key", JsonValue::int(cast(key))),
            ("steps", JsonValue::int(1)),
        ]),
    );
    let continued = run.ok(
        "store.save#2",
        "store.save",
        object([
            ("session_key", JsonValue::int(cast(key))),
            ("steps", JsonValue::int(2)),
        ]),
    );
    let second_snapshot = number(continued.get("result").expect("result"), "snapshot_key");
    run.ok(
        "store.delete#snapshot-1",
        "store.delete",
        object([
            ("session_key", JsonValue::int(cast(key))),
            ("snapshot_key", JsonValue::int(cast(second_snapshot - 1))),
        ]),
    );
    run.ok(
        "store.delete#snapshot-2",
        "store.delete",
        object([
            ("session_key", JsonValue::int(cast(key))),
            ("snapshot_key", JsonValue::int(cast(second_snapshot))),
        ]),
    );
    run.ok(
        "store.delete#session",
        "store.delete",
        object([("session_key", JsonValue::int(cast(key)))]),
    );
    let deleted = run.failed(
        "store.save#deleted",
        "store.save",
        object([
            ("session_key", JsonValue::int(cast(key))),
            ("steps", JsonValue::int(1)),
        ]),
    );
    assert_eq!(error_code(&deleted), "not_found");

    let experiment = run.ok(
        "experiment.run",
        "experiment.run",
        object([
            ("steps", JsonValue::int(6)),
            ("buttons", JsonValue::int(0)),
            (
                "checkpoints",
                JsonValue::array([JsonValue::int(0), JsonValue::int(3), JsonValue::int(6)]),
            ),
        ]),
    );
    let artifact_id = experiment
        .get("result")
        .and_then(|result| result.get("artifact_id"))
        .and_then(JsonValue::as_str)
        .expect("artifact id")
        .to_owned();
    assert_eq!(artifact_id.len(), 64);

    run.ok(
        "session.close",
        "session.close",
        object([("session", JsonValue::int(cast(session)))]),
    );
    let closed = run.failed(
        "session.inspect#closed",
        "session.inspect",
        object([("session", JsonValue::int(cast(session)))]),
    );
    assert_eq!(error_code(&closed), "closed");
    run.ok("session.list#closed", "session.list", empty());

    run.steps
}

fn cast(value: u64) -> i64 {
    i64::try_from(value).expect("identifier fits the canonical profile")
}

fn error_code(envelope: &JsonValue) -> &str {
    assert_eq!(
        envelope.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1"),
        "every failure is the typed error envelope"
    );
    envelope
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(JsonValue::as_str)
        .expect("error code")
}

// ---------------------------------------------------------------------------
// Scenario parity
// ---------------------------------------------------------------------------

#[test]
fn the_documented_scenario_is_byte_identical_on_direct_http_and_mcp() {
    let direct_directory = TempDir::new("direct");
    let direct = DirectSurface::new(Arc::new(configured_service(&direct_directory)), operator());

    let http_directory = TempDir::new("http");
    let (_http_server, http_client) = start_http(configured_service(&http_directory));
    let http = HttpSurface {
        client: http_client,
    };

    let mcp_directory = TempDir::new("mcp");
    let (_mcp_server, mcp_client, _addr) = start_mcp(configured_service(&mcp_directory));
    let mcp = McpSurface { client: mcp_client };

    let direct_steps = scenario(&direct);
    let http_steps = scenario(&http);
    let mcp_steps = scenario(&mcp);

    assert_eq!(direct_steps.len(), 23, "the scenario runs every step");
    assert_eq!(http_steps.len(), direct_steps.len());
    assert_eq!(mcp_steps.len(), direct_steps.len());
    assert!(
        direct_steps.iter().any(|step| step.is_error),
        "the scenario includes error paths"
    );
    for ((expected, http_step), mcp_step) in direct_steps
        .iter()
        .zip(http_steps.iter())
        .zip(mcp_steps.iter())
    {
        assert_eq!(expected.label, http_step.label);
        assert_eq!(expected.label, mcp_step.label);
        assert_eq!(
            expected.is_error, http_step.is_error,
            "{} agrees on success and failure",
            expected.label
        );
        assert_eq!(
            expected.is_error, mcp_step.is_error,
            "{} agrees on success and failure",
            expected.label
        );
        assert_eq!(
            expected.bytes, http_step.bytes,
            "{} is byte-identical over HTTP",
            expected.label
        );
        assert_eq!(
            expected.bytes, mcp_step.bytes,
            "{} is byte-identical over MCP after decoding the text content",
            expected.label
        );
    }
}

#[test]
fn repeating_the_scenario_on_a_fresh_surface_is_byte_identical() {
    // Determinism is part of the contract: two independent runs on fresh storage must
    // agree with each other, not only across surfaces.
    let first_directory = TempDir::new("repeat-a");
    let first = DirectSurface::new(Arc::new(configured_service(&first_directory)), operator());
    let second_directory = TempDir::new("repeat-b");
    let second = DirectSurface::new(Arc::new(configured_service(&second_directory)), operator());
    let first_steps = scenario(&first);
    let second_steps = scenario(&second);
    for (left, right) in first_steps.iter().zip(second_steps.iter()) {
        assert_eq!(left.label, right.label);
        assert_eq!(left.bytes, right.bytes, "{} is deterministic", left.label);
    }
}

// ---------------------------------------------------------------------------
// Error-scenario parity
// ---------------------------------------------------------------------------

#[test]
fn error_scenarios_agree_across_the_three_surfaces() {
    let direct_directory = TempDir::new("errors-direct");
    let direct_service = Arc::new(configured_service(&direct_directory));
    let direct = DirectSurface::new(Arc::clone(&direct_service), operator());

    let http_directory = TempDir::new("errors-http");
    let (_http_server, http_client) = start_http(configured_service(&http_directory));
    let http = HttpSurface {
        client: http_client,
    };

    let mcp_directory = TempDir::new("errors-mcp");
    let (_mcp_server, mcp_client, mcp_addr) = start_mcp(configured_service(&mcp_directory));
    let mcp = McpSurface { client: mcp_client };

    // All three surfaces create their own session 1, so the identifiers line up.
    let surfaces: [&dyn Surface; 3] = [&direct, &http, &mcp];
    for surface in surfaces {
        let created = surface.invoke("session.create", &empty());
        assert!(!created.is_error);
    }

    // Unknown session.
    let input = object([("session", JsonValue::int(99))]);
    let expected = direct.invoke("session.inspect", &input);
    assert!(expected.is_error);
    let envelope = expected.envelope();
    assert_eq!(error_code(&envelope), "not_found");
    let over_http = http.invoke("session.inspect", &input);
    let over_mcp = mcp.invoke("session.inspect", &input);
    assert_eq!(expected.bytes, over_http.bytes);
    assert_eq!(expected.bytes, over_mcp.bytes);
    assert!(over_http.is_error && over_mcp.is_error);

    // Invalid input.
    let input = object([("session", JsonValue::int(1)), ("steps", JsonValue::int(0))]);
    let expected = direct.invoke("session.run", &input);
    let envelope = expected.envelope();
    assert_eq!(error_code(&envelope), "invalid_request");
    let over_http = http.invoke("session.run", &input);
    let over_mcp = mcp.invoke("session.run", &input);
    assert_eq!(expected.bytes, over_http.bytes);
    assert_eq!(expected.bytes, over_mcp.bytes);

    // Unknown operation: the service code is `unknown_op`; the MCP surface cannot see an
    // unknown operation because an unknown tool name is a JSON-RPC `-32602` instead.
    let input = empty();
    let expected = direct.invoke("not.an.op", &input);
    let envelope = expected.envelope();
    assert_eq!(error_code(&envelope), "unknown_op");
    let over_http = http.invoke("not.an.op", &input);
    assert_eq!(expected.bytes, over_http.bytes);
    let unknown_tool = mcp
        .client
        .call_tool("not.an.op", &input)
        .expect_err("an unknown tool is a JSON-RPC error");
    assert!(
        matches!(unknown_tool, TransportError::Protocol(ref message) if message.contains("-32602")),
        "unexpected MCP failure: {unknown_tool:?}"
    );

    // Foreign session: another principal is forbidden on the same service.
    let foreign = DirectSurface::new(Arc::clone(&direct_service), other());
    let input = object([("session", JsonValue::int(1))]);
    let expected = foreign.invoke("session.inspect", &input);
    let envelope = expected.envelope();
    assert_eq!(error_code(&envelope), "forbidden");
    let foreign_http = HttpClient::new(http.client.base().to_owned(), OTHER_TOKEN);
    let over_http = match foreign_http.call("session.inspect", &input) {
        Ok(_) => unreachable!("a foreign session must be rejected"),
        Err(TransportError::Http { body, .. }) => Outcome {
            bytes: body,
            is_error: true,
        },
        Err(error) => unreachable!("unexpected transport failure: {error:?}"),
    };
    let foreign_mcp = McpClient::new(mcp.client.http().base().to_owned(), OTHER_TOKEN);
    let over_mcp = match foreign_mcp.call_tool("session.inspect", &input) {
        Ok(result) => Outcome {
            bytes: result.text.into_bytes(),
            is_error: result.is_error,
        },
        Err(error) => unreachable!("unexpected MCP failure: {error:?}"),
    };
    assert_eq!(expected.bytes, over_http.bytes);
    assert_eq!(expected.bytes, over_mcp.bytes);

    // Unauthorized: identical transport envelope over HTTP and MCP.
    let anonymous_http = HttpClient::new(http.client.base().to_owned(), "bogus-token-000001");
    let http_body = match anonymous_http.call("capabilities", &input) {
        Ok(_) => unreachable!("an unknown token must be rejected"),
        Err(TransportError::Http { status, body }) => {
            assert_eq!(status, 401);
            body
        }
        Err(error) => unreachable!("unexpected transport failure: {error:?}"),
    };
    let anonymous_mcp = McpClient::new(mcp.client.http().base().to_owned(), "bogus-token-000001");
    let mcp_body = match anonymous_mcp.call_tool("capabilities", &input) {
        Ok(_) => unreachable!("an unknown token must be rejected"),
        Err(TransportError::Http { status, body }) => {
            assert_eq!(status, 401);
            body
        }
        Err(error) => unreachable!("unexpected transport failure: {error:?}"),
    };
    assert_eq!(
        http_body, mcp_body,
        "an unauthorized request produces the same envelope on both transports"
    );
    let envelope = parse(&http_body, &limits()).expect("envelope");
    assert_eq!(error_code(&envelope), "unauthorized");

    // Transport-level rejections (HTTP/MCP only) keep their documented typed codes.
    let valid = to_canonical_bytes(
        &object([
            ("op", JsonValue::string("capabilities")),
            ("input", empty()),
        ]),
        &limits(),
    )
    .expect("request body");
    let wrong_type = raw_post(
        mcp_addr,
        "/v1/ops",
        Some("text/plain"),
        &valid,
        Some(OPERATOR_TOKEN),
    );
    assert_eq!(wrong_type.status, 415);
    assert_eq!(error_code(&wrong_type.envelope()), "unsupported_media_type");

    let unknown_route = raw_post(
        mcp_addr,
        "/v2/ops",
        Some("application/json"),
        &valid,
        Some(OPERATOR_TOKEN),
    );
    assert_eq!(unknown_route.status, 404);
    assert_eq!(error_code(&unknown_route.envelope()), "not_found");

    // Body too large: the same canonical request is rejected against a server whose
    // configured body bound is smaller than it, and the bound is checked from the
    // declared length before the body is read.
    let restricted_directory = TempDir::new("errors-restricted");
    let restricted = serve(
        ServerConfig {
            max_request_bytes: 16,
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(configured_service(&restricted_directory)),
    )
    .expect("restricted server");
    let too_large = raw_post(
        restricted.local_addr(),
        "/v1/ops",
        Some("application/json"),
        &valid,
        Some(OPERATOR_TOKEN),
    );
    assert_eq!(too_large.status, 413);
    assert_eq!(error_code(&too_large.envelope()), "body_too_large");
    restricted.shutdown().expect("shutdown");

    let batch = raw_post(
        mcp_addr,
        "/mcp",
        Some("application/json"),
        b"[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]",
        Some(OPERATOR_TOKEN),
    );
    assert_eq!(batch.status, 400);
    let document = parse(&batch.body, &limits()).expect("json-rpc error");
    assert_eq!(
        document
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_i64),
        Some(-32600)
    );
}

// ---------------------------------------------------------------------------
// A raw HTTP request, for the rejections the typed client cannot express
// ---------------------------------------------------------------------------

struct RawResponse {
    status: u16,
    body: Vec<u8>,
}

impl RawResponse {
    fn envelope(&self) -> JsonValue {
        parse(&self.body, &limits()).expect("typed envelope")
    }
}

fn raw_post(
    addr: SocketAddr,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
    token: Option<&str>,
) -> RawResponse {
    let mut request = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    if let Some(content_type) = content_type {
        request.push_str("Content-Type: ");
        request.push_str(content_type);
        request.push_str("\r\n");
    }
    if let Some(token) = token {
        request.push_str("Authorization: Bearer ");
        request.push_str(token);
        request.push_str("\r\n");
    }
    request.push_str("Content-Length: ");
    request.push_str(&body.len().to_string());
    request.push_str("\r\nConnection: close\r\n\r\n");
    let mut request = request.into_bytes();
    request.extend_from_slice(body);
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");
    stream.write_all(&request).expect("write request");
    stream.flush().expect("flush request");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read response");
    let end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response head terminator")
        + 4;
    let head = std::str::from_utf8(&bytes[..end - 4]).expect("response head is UTF-8");
    let status = head
        .lines()
        .next()
        .expect("status line")
        .split(' ')
        .nth(1)
        .expect("status code")
        .parse::<u16>()
        .expect("numeric status");
    RawResponse {
        status,
        body: bytes.get(end..).unwrap_or_default().to_vec(),
    }
}
