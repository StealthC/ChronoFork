#![allow(clippy::expect_used)]

//! W3d wire-version tests (review finding W3-R7).
//!
//! The ADR-0016 compatibility matrix is a contract about **shapes and negotiation**, so
//! these tests drive the real HTTP surface:
//!
//! - a request without a version is served as v1 and a request with `2` as v2, with the
//!   documented shape difference;
//! - an unimplementable version is refused before dispatch, so nothing is applied;
//! - the v2 surface (operations and input fields) does not exist on v1;
//! - the typed v2 client probes read-only and refuses locally against a daemon that does
//!   not advertise v2 — the sentinel mutation is never sent.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{
    ApiVersion, AppService, ServiceConfig, ServiceLimits, TokenTable, parse_token_file,
};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{HttpClient, ServerConfig, TransportError, serve};

const OPERATOR_TOKEN: &str = "operator-token-0001";

const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]}]}"#;

fn tokens() -> TokenTable {
    parse_token_file(TOKENS.as_bytes()).expect("token table")
}

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn empty() -> JsonValue {
    JsonValue::Object(Default::default())
}

fn start() -> chronofork_transport::ServerHandle {
    let service = AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })
    .expect("service");
    serve(
        ServerConfig {
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service),
    )
    .expect("server")
}

/// Sends one canonical request with an explicit version header (or none).
fn post(addr: SocketAddr, op: &str, input: &JsonValue, version: Option<&str>) -> (u32, Vec<u8>) {
    let document = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
    let body = to_canonical_bytes(&document, &limits()).expect("request bytes");
    let mut request = format!(
        "POST /v1/ops HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {OPERATOR_TOKEN}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(version) = version {
        request.push_str("X-ChronoFork-API: ");
        request.push_str(version);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    stream.write_all(request.as_bytes()).expect("write head");
    stream.write_all(&body).expect("write body");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read response");
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("header terminator");
    let head = String::from_utf8_lossy(&response[..split]).into_owned();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .expect("status");
    (status, response[split + 4..].to_vec())
}

fn envelope(bytes: &[u8]) -> JsonValue {
    parse(bytes, &limits()).expect("canonical envelope")
}

fn result_field<'a>(value: &'a JsonValue, name: &str) -> Option<&'a JsonValue> {
    value.get("result").and_then(|result| result.get(name))
}

/// The same `session.run` is served with the v1 or the v2 shape by declaration.
#[test]
fn the_version_header_selects_the_documented_result_shape() {
    let server = start();
    let addr = server.local_addr();
    let (status, created) = post(
        addr,
        "session.create",
        &JsonValue::object([("operation_id", JsonValue::int(9))]),
        Some("2"),
    );
    assert_eq!(status, 200);
    let created = envelope(&created);
    // The v2-only input field was accepted under the v2 header; `session.create` adds no
    // result field in v2 (a new session is revision zero), so this only proves the call
    // was served on the declared wire.
    assert!(
        result_field(&created, "session")
            .and_then(JsonValue::as_u64)
            .is_some(),
        "v2 create"
    );

    let input = JsonValue::object([("session", JsonValue::int(1)), ("steps", JsonValue::int(2))]);
    // No header: the request is v1, and the run result is the original shape.
    let (status, v1) = post(addr, "session.run", &input, None);
    assert_eq!(status, 200);
    let v1 = envelope(&v1);
    assert!(result_field(&v1, "revision").is_none());
    assert!(result_field(&v1, "performed").is_none());
    assert!(result_field(&v1, "stop_reason").is_none());
    assert!(result_field(&v1, "cycles").is_none());
    assert_eq!(
        result_field(&v1, "steps").and_then(JsonValue::as_u64),
        Some(2)
    );

    // Declared v2: the same call carries the reconciliation fields.
    let (status, v2) = post(addr, "session.run", &input, Some("2"));
    assert_eq!(status, 200);
    let v2 = envelope(&v2);
    assert_eq!(
        result_field(&v2, "performed").and_then(JsonValue::as_u64),
        Some(2)
    );
    assert_eq!(
        result_field(&v2, "stop_reason").and_then(JsonValue::as_str),
        Some("completed")
    );
    assert!(
        result_field(&v2, "cycles")
            .and_then(JsonValue::as_u64)
            .is_some_and(|cycles| cycles > 0),
        "v2 run reports the cycles consumed by the batch"
    );
    assert!(result_field(&v2, "revision").is_some());
    server.shutdown().expect("shutdown");
}

/// An unimplementable version is a typed refusal **before** dispatch.
#[test]
fn an_unimplemented_version_is_refused_before_dispatch() {
    let server = start();
    let addr = server.local_addr();
    let (status, body) = post(addr, "session.create", &empty(), Some("3"));
    assert_eq!(
        status, 400,
        "the refusal is a client-side compatibility error"
    );
    let document = envelope(&body);
    assert_eq!(
        document
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("unsupported_version")
    );
    // Nothing was dispatched: a v2 list sees no session.
    let (_, listing) = post(addr, "session.list", &empty(), Some("2"));
    let listing = envelope(&listing);
    assert_eq!(
        result_field(&listing, "sessions")
            .and_then(JsonValue::as_array)
            .map(<[JsonValue]>::len),
        Some(0),
        "the refused request created nothing"
    );
    server.shutdown().expect("shutdown");
}

/// The v2-only operations do not exist on the v1 wire.
#[test]
fn a_v2_only_operation_is_unknown_on_v1() {
    let server = start();
    let addr = server.local_addr();
    let (status, body) = post(addr, "session.reap", &empty(), None);
    assert_eq!(status, 400);
    let document = envelope(&body);
    assert_eq!(
        document
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("unknown_op")
    );
    // On v2 the same request is accepted (there is nothing to reap).
    let (status, _) = post(addr, "session.reap", &empty(), Some("2"));
    assert_eq!(status, 200);
    server.shutdown().expect("shutdown");
}

/// A fake legacy daemon: it answers every request with a v1 capabilities document and
/// records what it was asked for.
fn spawn_legacy_daemon() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake daemon");
    let addr = listener.local_addr().expect("addr");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    thread::spawn(move || {
        for _ in 0..4 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 1024];
            // Read the head, then the body by Content-Length.
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                    Err(_) => break,
                }
                if let Some(split) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buffer[..split]).into_owned();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    if buffer.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            let split = buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap_or(0);
            let body = String::from_utf8_lossy(&buffer[split + 4..]).into_owned();
            recorder.lock().expect("recorder").push(body);
            // A legacy daemon: it ignores the version header and answers the v1 shape.
            let document = JsonValue::object([
                ("op", JsonValue::string("capabilities")),
                (
                    "result",
                    JsonValue::object([
                        ("schema", JsonValue::string("chronofork.capabilities.v1")),
                        ("service_version", JsonValue::string("0.0.0-legacy")),
                    ]),
                ),
            ]);
            let body = to_canonical_bytes(&document, &limits()).expect("fake response");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    (format!("http://{addr}"), seen)
}

/// The v2 client probes read-only and refuses locally against a legacy daemon; the
/// sentinel mutation is never dispatched.
#[test]
fn the_client_refuses_v2_locally_against_a_legacy_daemon() {
    let (base, seen) = spawn_legacy_daemon();
    let client = HttpClient::new(&base, OPERATOR_TOKEN).with_version(ApiVersion::V2);
    let error = client
        .call("session.create", &empty())
        .expect_err("the legacy daemon does not advertise v2");
    assert!(matches!(error, TransportError::UnsupportedVersion(_)));
    let requests = seen.lock().expect("recorder").clone();
    assert_eq!(
        requests.len(),
        1,
        "only the read-only probe reached the daemon; the sentinel mutation was not sent"
    );
    assert!(
        requests[0].contains("capabilities"),
        "the probe is the capabilities discovery call: {}",
        requests[0]
    );
    assert!(
        !requests[0].contains("session.create"),
        "the sentinel mutation must never be dispatched"
    );
}

/// A v1 client against a legacy daemon keeps working unchanged.
#[test]
fn a_v1_client_needs_no_probe_against_a_legacy_daemon() {
    let (base, seen) = spawn_legacy_daemon();
    let client = HttpClient::new(&base, OPERATOR_TOKEN);
    let response = client
        .call("capabilities", &empty())
        .expect("a v1 call is served");
    let document = envelope(&response);
    assert!(document.get("result").is_some());
    let requests = seen.lock().expect("recorder").clone();
    assert_eq!(
        requests.len(),
        1,
        "a v1 client sends exactly its own request"
    );
}
