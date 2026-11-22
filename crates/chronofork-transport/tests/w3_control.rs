#![allow(clippy::expect_used)]

//! W3.2 wire-level test: cancel and deadline over a real socket.
//!
//! The service contract of ADR-0016 §2 has to survive the transport as well:
//!
//! - a run truncated by its **host deadline** returns its `performed`/`stop_reason`
//!   over HTTP instead of a failed request, which is what makes the client's
//!   long-operation timeout (raised above the 30 s server deadline) useful;
//! - a **cancel** issued from another connection while a run is in flight is
//!   answered immediately, and the run returns its partial result with a
//!   deterministic prefix;
//! - **progress** of the running operation is readable from the journal over the
//!   same socket while the run holds its session owner.
//!
//! Every request declares API wire v2: the control operations and the reconciliation
//! input/result fields are v2-only (ADR-0016 §Compatibility), so a request without the
//! header is served the v1 shape and cannot name them.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{AppService, ServiceConfig, ServiceLimits, TokenTable, parse_token_file};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{ServerConfig, ServerHandle, serve};

const OPERATOR_TOKEN: &str = "operator-token-0001";

const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]}]}"#;

/// Steps for a run that only ever stops because the control path stops it.
const LONG_RUN_STEPS: u64 = 40_000;

/// Deadline for one control request over the socket.
///
/// The worker pool runs these cases concurrently with the long run, so a control
/// request that waits for CPU is still distinguished from one that waits for the
/// session owner: the latter never completes at all.
const CONTROL_DEADLINE: Duration = Duration::from_secs(60);

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn tokens() -> TokenTable {
    parse_token_file(TOKENS.as_bytes()).expect("token table")
}

fn service() -> Arc<AppService> {
    Arc::new(
        AppService::new(ServiceConfig {
            factory: Arc::new(SyntheticBackendFactory::default()),
            storage_root: None,
            artifacts_root: None,
            limits: limits(),
        })
        .expect("service"),
    )
}

fn start() -> (ServerHandle, SocketAddr) {
    let config = ServerConfig {
        tokens: tokens(),
        ..ServerConfig::default()
    };
    let handle = serve(config, service()).expect("server");
    let address = handle.local_addr();
    (handle, address)
}

/// Sends one canonical request over a fresh connection and returns the status/body.
fn post(address: SocketAddr, request: &[u8]) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(CONTROL_DEADLINE))
        .expect("read timeout");
    let head = format!(
        "POST /v1/ops HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {OPERATOR_TOKEN}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\
         X-ChronoFork-API: 2\r\n\r\n",
        request.len()
    );
    stream.write_all(head.as_bytes()).expect("write head");
    stream.write_all(request).expect("write body");
    stream.flush().expect("flush");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read response");
    let text = String::from_utf8(raw).expect("utf-8 response");
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| unreachable!("no header terminator in {text}"));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| unreachable!("no status line in {head}"));
    (status, body.as_bytes().to_vec())
}

fn request(op: &str, input: &str) -> Vec<u8> {
    let limits = limits();
    let value = JsonValue::object([
        ("op", JsonValue::string(op)),
        (
            "input",
            parse(input.as_bytes(), &limits).expect("input object"),
        ),
    ]);
    to_canonical_bytes(&value, &limits).expect("request bytes")
}

/// Issues one request and returns the `result` object, requiring HTTP 200.
fn call(address: SocketAddr, op: &str, input: &str) -> JsonValue {
    let (status, body) = post(address, &request(op, input));
    assert_eq!(
        status,
        200,
        "{op} must succeed: {}",
        String::from_utf8_lossy(&body)
    );
    let envelope = parse(&body, &limits()).expect("canonical envelope");
    assert_eq!(
        envelope.get("op").and_then(JsonValue::as_str),
        Some(op),
        "envelope names the operation"
    );
    envelope.get("result").expect("result").clone()
}

fn number(value: &JsonValue, name: &str) -> u64 {
    value
        .get(name)
        .and_then(JsonValue::as_u64)
        .unwrap_or_else(|| unreachable!("missing unsigned field `{name}` in {value:?}"))
}

fn text(value: &JsonValue, name: &str) -> String {
    value
        .get(name)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| unreachable!("missing string field `{name}` in {value:?}"))
        .to_owned()
}

fn create(address: SocketAddr) -> u64 {
    number(&call(address, "session.create", "{}"), "session")
}

/// Returns the tick and state digest of an independent `steps`-step oracle.
///
/// The oracle is a **second service on its own socket**, so nothing in the daemon
/// under test participates: equality proves the truncated run applied exactly that
/// many canonical steps.
fn oracle(steps: u64) -> (ServerHandle, u64, String) {
    let (handle, address) = start();
    let session = create(address);
    let result = call(
        address,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":{steps}}}"),
    );
    (handle, number(&result, "tick"), text(&result, "state_hash"))
}

fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(1));
    }
    predicate()
}

#[test]
fn a_deadline_truncated_run_answers_with_its_prefix_before_the_client_gives_up() {
    let (handle, address) = start();
    let session = create(address);
    let started = Instant::now();
    let result = call(
        address,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":{LONG_RUN_STEPS},\"max_wall_ms\":150}}"),
    );
    let elapsed = started.elapsed();
    assert_eq!(text(&result, "stop_reason"), "deadline");
    assert_eq!(number(&result, "max_wall_ms"), 150);
    let performed = number(&result, "performed");
    assert!(
        performed < LONG_RUN_STEPS,
        "the deadline truncated the run: performed {performed}"
    );
    let (oracle_handle, tick, state_hash) = oracle(performed);
    assert_eq!(number(&result, "tick"), tick);
    assert_eq!(text(&result, "state_hash"), state_hash);
    oracle_handle.shutdown().expect("oracle shutdown");
    assert!(
        elapsed < CONTROL_DEADLINE,
        "a 150 ms deadline answered in {elapsed:?}"
    );
    handle.shutdown().expect("shutdown");
}

#[test]
fn cancel_and_progress_are_served_over_the_socket_while_a_run_is_in_flight() {
    let (handle, address) = start();
    let session = create(address);
    let runner = thread::spawn(move || {
        let (status, body) = post(
            address,
            &request(
                "session.run",
                &format!("{{\"session\":{session},\"steps\":{LONG_RUN_STEPS},\"operation_id\":6}}"),
            ),
        );
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
        parse(&body, &limits()).expect("canonical envelope")
    });

    // Another connection reads the running operation's progress: the control path is
    // served while the run holds its session owner.
    let observed = wait_until(CONTROL_DEADLINE, || {
        let progress = post(address, &request("session.progress", "{\"operation\":6}"));
        if progress.0 != 200 {
            return false;
        }
        let envelope = parse(&progress.1, &limits()).expect("envelope");
        let result = envelope.get("result").expect("result");
        result.get("state").and_then(JsonValue::as_str) == Some("active")
    });
    assert!(
        observed,
        "progress was not served while the run was in flight"
    );

    let cancelled = post(
        address,
        &request("session.cancel", &format!("{{\"session\":{session}}}")),
    );
    assert_eq!(
        cancelled.0,
        200,
        "cancel must be answered immediately: {}",
        String::from_utf8_lossy(&cancelled.1)
    );

    let envelope = runner.join().expect("run thread");
    let result = envelope.get("result").expect("result");
    assert_eq!(text(result, "stop_reason"), "cancelled");
    let performed = number(result, "performed");
    assert!(performed < LONG_RUN_STEPS, "performed {performed}");
    let (oracle_handle, tick, state_hash) = oracle(performed);
    assert_eq!(number(result, "tick"), tick);
    assert_eq!(text(result, "state_hash"), state_hash);
    oracle_handle.shutdown().expect("oracle shutdown");

    // The finished entry is readable afterwards, which is the reconciliation path of
    // a caller that lost the response.
    let recorded = call(address, "session.operation", "{\"operation\":6}");
    assert_eq!(text(&recorded, "state"), "finished");
    assert_eq!(text(&recorded, "stop_reason"), "cancelled");
    assert_eq!(number(&recorded, "performed"), performed);
    handle.shutdown().expect("shutdown");
}

#[test]
fn a_control_request_for_an_unknown_operation_is_a_typed_wire_error() {
    let (handle, address) = start();
    let (status, body) = post(address, &request("session.progress", "{\"operation\":404}"));
    assert_eq!(
        status, 409,
        "an unreconcilable outcome is a request-scoped refusal"
    );
    let document = parse(&body, &limits()).expect("canonical error envelope");
    assert_eq!(
        document.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1")
    );
    assert_eq!(
        document.get("wire_version").and_then(JsonValue::as_u64),
        Some(2)
    );
    assert_eq!(
        document
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("unknown_outcome")
    );
    handle.shutdown().expect("shutdown");
}

#[test]
fn the_advertised_limits_bounds_the_control_surface_over_the_wire() {
    let (handle, address) = start();
    let capabilities = call(address, "capabilities", "{}");
    assert_eq!(number(&capabilities, "api_version"), 2);
    let limits = capabilities.get("limits").expect("limits");
    assert_eq!(number(limits, "max_run_wall_ms"), 30_000);
    assert_eq!(number(limits, "max_operation_journal"), 1_024);
    assert_eq!(number(limits, "operation_ttl"), 600);
    handle.shutdown().expect("shutdown");
}
