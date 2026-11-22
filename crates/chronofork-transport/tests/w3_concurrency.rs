#![allow(clippy::expect_used)]

//! W3.1 wire-level test: a long run on one session does not block reads of another.
//!
//! The service owns one execution owner per session (ADR-0016 §1). Over a real
//! socket with a 4-worker pool, this test parks a 2,000-step run inside a backend
//! step on session 1 and proves that `session.list` and `session.inspect` of
//! session 2 are served while it is parked, within the typed client's read timeout.
//!
//! Every request declares API wire v2: the `running` marker and the control surface
//! are v2-only shapes (ADR-0016 §Compatibility), so a request without the header would
//! be answered with the v1 shape and could not observe them.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAccess,
    MachineInspection, StepOutcome,
};
use chronofork_core::InputFrame;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{AppService, ServiceConfig, ServiceLimits, TokenTable, parse_token_file};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{ServerConfig, ServerHandle, serve};

const OPERATOR_TOKEN: &str = "operator-token-0001";

/// A second principal, so the per-principal long-run bound is observable.
const WORKER_TOKEN: &str = "worker-token-0002";

const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]},
 {"name":"worker","token":"worker-token-0002","scopes":["read","write"]}]}"#;

/// The run parks in its first step; it only has to finish quickly after release.
const RUN_STEPS: u64 = 2_000;

/// Deadline for a control request that must not wait for the parked run.
const CONTROL_DEADLINE: Duration = Duration::from_secs(5);

/// Deadline for the parked run itself, which resumes after the gate is released.
const RUN_DEADLINE: Duration = Duration::from_secs(60);

fn tokens() -> TokenTable {
    parse_token_file(TOKENS.as_bytes()).expect("token table")
}

// ---------------------------------------------------------------------------
// A gate backend: the first `step` parks until the test releases it
// ---------------------------------------------------------------------------

#[derive(Default)]
struct GateState {
    armed: bool,
    released: bool,
    waiting: bool,
}

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    signal: Condvar,
}

impl Gate {
    fn arm(&self) {
        let mut state = self.state.lock().expect("gate lock");
        state.armed = true;
        state.released = false;
        state.waiting = false;
    }

    fn wait(&self) {
        let mut state = self.state.lock().expect("gate lock");
        if !state.armed {
            return;
        }
        // Only the first step parks: later steps must not re-enter the rendezvous.
        state.armed = false;
        state.waiting = true;
        self.signal.notify_all();
        while !state.released {
            state = self.signal.wait(state).expect("gate wait");
        }
    }

    fn wait_until_waiting(&self) {
        let mut state = self.state.lock().expect("gate lock");
        while !state.waiting {
            state = self.signal.wait(state).expect("gate wait");
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("gate lock");
        state.armed = false;
        state.released = true;
        self.signal.notify_all();
    }

    /// Returns a guard that releases the gate when it is dropped.
    ///
    /// A test that fails an assertion before its explicit release would otherwise leave
    /// the parked request inside a worker forever, and [`ServerHandle`]'s drop waits for
    /// that request: the guard turns an assertion failure into a failure instead of a
    /// hang. Releasing twice is harmless.
    fn release_on_drop(gate: &Arc<Self>) -> GateRelease {
        GateRelease(Arc::clone(gate))
    }
}

/// Releases its gate when dropped, including while unwinding an assertion failure.
struct GateRelease(Arc<Gate>);

impl Drop for GateRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct GatedBackend {
    inner: Box<dyn Backend>,
    gate: Arc<Gate>,
}

impl Backend for GatedBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.gate.wait();
        self.inner.step(input)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.inner.snapshot()
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        self.inner.restore(snapshot)
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        self.inner.inspect()
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        self.inner.read_memory(address, length)
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        self.inner.write_memory(address, data)
    }

    fn debug_access(&mut self) -> Option<&mut dyn DebugAccess> {
        self.inner.debug_access()
    }
}

struct GatedFactory {
    inner: SyntheticBackendFactory,
    gate: Arc<Gate>,
}

impl BackendFactory for GatedFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(GatedBackend {
            inner: self.inner.create()?,
            gate: Arc::clone(&self.gate),
        }))
    }
}

fn gated_service(gate: &Arc<Gate>) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(GatedFactory {
            inner: SyntheticBackendFactory::default(),
            gate: Arc::clone(gate),
        }),
        storage_root: None,
        artifacts_root: None,
        limits: ServiceLimits::default(),
    })
    .expect("service")
}

// ---------------------------------------------------------------------------
// Minimal wire client
// ---------------------------------------------------------------------------

fn post(addr: SocketAddr, body: &[u8], read_timeout: Duration) -> (u32, Vec<u8>) {
    post_as(addr, OPERATOR_TOKEN, body, read_timeout)
}

fn post_as(addr: SocketAddr, token: &str, body: &[u8], read_timeout: Duration) -> (u32, Vec<u8>) {
    let mut stream = TcpStream::connect_timeout(&addr, CONTROL_DEADLINE).expect("connect");
    stream
        .set_read_timeout(Some(read_timeout))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(read_timeout))
        .expect("write timeout");
    let head = format!(
        "POST /v1/ops HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\
         X-ChronoFork-API: 2\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("write head");
    stream.write_all(body).expect("write body");
    stream.flush().expect("flush");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read response");
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("header terminator");
    let head_text = String::from_utf8_lossy(&response[..split]).into_owned();
    let status = head_text
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .expect("status code");
    (status, response[split + 4..].to_vec())
}

fn call(addr: SocketAddr, op: &str, text: &str) -> (u32, Vec<u8>) {
    call_as(addr, OPERATOR_TOKEN, op, text)
}

fn call_as(addr: SocketAddr, token: &str, op: &str, text: &str) -> (u32, Vec<u8>) {
    let input = parse(text.as_bytes(), &ServiceLimits::default()).expect("input");
    let request = JsonValue::object([("op", JsonValue::string(op)), ("input", input)]);
    let body = to_canonical_bytes(&request, &ServiceLimits::default()).expect("request bytes");
    post_as(addr, token, &body, CONTROL_DEADLINE)
}

/// Builds one `session.run` request body.
fn run_body(session: u64, steps: u64) -> Vec<u8> {
    let input = parse(
        format!("{{\"session\":{session},\"steps\":{steps}}}").as_bytes(),
        &ServiceLimits::default(),
    )
    .expect("input");
    let request = JsonValue::object([("op", JsonValue::string("session.run")), ("input", input)]);
    to_canonical_bytes(&request, &ServiceLimits::default()).expect("request bytes")
}

fn result_field(bytes: &[u8], name: &str) -> Option<JsonValue> {
    let envelope = parse(bytes, &ServiceLimits::default()).expect("envelope");
    envelope
        .get("result")
        .and_then(|result| result.get(name))
        .cloned()
}

fn result_number(bytes: &[u8], name: &str) -> u64 {
    result_field(bytes, name)
        .and_then(|value| value.as_u64())
        .expect("unsigned result field")
}

fn error_code(bytes: &[u8]) -> String {
    let envelope = parse(bytes, &ServiceLimits::default()).expect("error envelope");
    envelope
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(JsonValue::as_str)
        .expect("error envelope code")
        .to_owned()
}

fn create_session(addr: SocketAddr) -> u64 {
    create_session_as(addr, OPERATOR_TOKEN)
}

fn create_session_as(addr: SocketAddr, token: &str) -> u64 {
    let (status, body) = call_as(addr, token, "session.create", "{}");
    assert_eq!(status, 200, "session.create must succeed");
    result_number(&body, "session")
}

// ---------------------------------------------------------------------------

/// A parked long run over the wire leaves the control and read paths for another
/// session responsive.
#[test]
fn a_long_run_over_http_does_not_block_another_session() {
    let gate = Arc::new(Gate::default());
    let server: ServerHandle = serve(
        ServerConfig {
            tokens: tokens(),
            workers: 4,
            ..ServerConfig::default()
        },
        Arc::new(gated_service(&gate)),
    )
    .expect("server");
    let addr = server.local_addr();

    assert_eq!(create_session(addr), 1);
    assert_eq!(create_session(addr), 2);

    gate.arm();
    let _release = Gate::release_on_drop(&gate);
    let runner = thread::spawn(move || {
        // This request parks inside its first step until `gate.release()`, then
        // completes the remaining steps on its own worker thread.
        let input = parse(
            format!("{{\"session\":1,\"steps\":{RUN_STEPS}}}").as_bytes(),
            &ServiceLimits::default(),
        )
        .expect("input");
        let request =
            JsonValue::object([("op", JsonValue::string("session.run")), ("input", input)]);
        let body = to_canonical_bytes(&request, &ServiceLimits::default()).expect("request bytes");
        post(addr, &body, RUN_DEADLINE)
    });
    gate.wait_until_waiting();

    let started = Instant::now();
    let (list_status, list_body) = call(addr, "session.list", "{}");
    assert_eq!(list_status, 200, "list is served while a run is parked");
    let sessions = result_field(&list_body, "sessions")
        .and_then(|value| value.as_array().map(<[JsonValue]>::to_vec))
        .expect("sessions array");
    assert_eq!(sessions.len(), 2, "both sessions are listed");
    let running = sessions
        .iter()
        .find(|entry| {
            entry
                .get("session")
                .and_then(JsonValue::as_u64)
                .is_some_and(|session| session == 1)
        })
        .expect("the parked session is listed");
    assert_eq!(
        running.get("running").and_then(JsonValue::as_bool),
        Some(true),
        "the parked session is marked running"
    );
    assert!(
        started.elapsed() < CONTROL_DEADLINE,
        "list must not wait for the parked run"
    );

    let (inspect_status, inspect_body) = call(addr, "session.inspect", "{\"session\":2}");
    assert_eq!(inspect_status, 200, "inspect of another session is served");
    assert_eq!(result_number(&inspect_body, "session"), 2);

    gate.release();
    let (run_status, run_body) = runner.join().expect("runner thread");
    assert_eq!(run_status, 200, "the parked run completes");
    assert_eq!(result_number(&run_body, "steps"), RUN_STEPS);
    server.shutdown().expect("shutdown");
}

/// A typed error is produced for another session while a run is parked: the control
/// path is reachable, so the error is a `not_found` envelope rather than a timeout.
#[test]
fn control_errors_are_typed_while_a_run_is_parked() {
    let gate = Arc::new(Gate::default());
    let server = serve(
        ServerConfig {
            tokens: tokens(),
            // One worker runs the parked request; the pool default still reserves the
            // control path, and this test pins the smaller bound on purpose.
            workers: 2,
            ..ServerConfig::default()
        },
        Arc::new(gated_service(&gate)),
    )
    .expect("server");
    let addr = server.local_addr();
    assert_eq!(create_session(addr), 1);

    gate.arm();
    let _release = Gate::release_on_drop(&gate);
    let runner = thread::spawn(move || {
        let input = parse(
            format!("{{\"session\":1,\"steps\":{RUN_STEPS}}}").as_bytes(),
            &ServiceLimits::default(),
        )
        .expect("input");
        let request =
            JsonValue::object([("op", JsonValue::string("session.run")), ("input", input)]);
        let body = to_canonical_bytes(&request, &ServiceLimits::default()).expect("request bytes");
        post(addr, &body, RUN_DEADLINE)
    });
    gate.wait_until_waiting();

    let (status, body) = call(addr, "session.inspect", "{\"session\":99}");
    assert_eq!(status, 404, "an unknown session is a typed not_found");
    assert_eq!(error_code(&body), "not_found");

    // The parked session is still observable through `list`, which does not take its
    // owner: the control path is served while the run holds that owner.
    let (list_status, list_body) = call(addr, "session.list", "{}");
    assert_eq!(list_status, 200);
    let sessions = result_field(&list_body, "sessions")
        .and_then(|value| value.as_array().map(<[JsonValue]>::to_vec))
        .expect("sessions array");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].get("running").and_then(JsonValue::as_bool),
        Some(true)
    );

    gate.release();
    let (run_status, _body) = runner.join().expect("runner thread");
    assert_eq!(run_status, 200);
    server.shutdown().expect("shutdown");
}

/// The long-run bound is enforced **before** a worker is occupied (W3-R6).
///
/// One principal's parked run holds one long-run slot. A second run from the same
/// principal is a typed `limit_exceeded` answered immediately — it never queues for the
/// parked session, so the refusal itself proves the decision happened before the work —
/// while another principal (and the reserved worker serving control reads) still reach
/// the service.
#[test]
fn an_excess_long_run_is_rejected_before_it_occupies_a_worker() {
    let gate = Arc::new(Gate::default());
    let server = serve(
        ServerConfig {
            tokens: tokens(),
            // Two long runs may be admitted at once (workers - 1); the third worker
            // stays reserved for the control path.
            workers: 3,
            max_long_runs_per_principal: Some(1),
            ..ServerConfig::default()
        },
        Arc::new(gated_service(&gate)),
    )
    .expect("server");
    let addr = server.local_addr();

    assert_eq!(create_session(addr), 1, "operator's session");
    assert_eq!(create_session_as(addr, WORKER_TOKEN), 2, "worker's session");

    gate.arm();
    let _release = Gate::release_on_drop(&gate);
    let runner = thread::spawn(move || post(addr, &run_body(1, RUN_STEPS), RUN_DEADLINE));
    gate.wait_until_waiting();

    // The control path is served while the long run is parked.
    let (list_status, _) = call(addr, "session.list", "{}");
    assert_eq!(list_status, 200, "list is served while a run is parked");

    // The same principal is at its bound: the refusal is immediate and typed.
    let started = Instant::now();
    let (rejected_status, rejected_body) = post(addr, &run_body(1, 5), CONTROL_DEADLINE);
    assert_eq!(
        rejected_status,
        400,
        "the excess run is refused: {}",
        String::from_utf8_lossy(&rejected_body)
    );
    assert_eq!(error_code(&rejected_body), "limit_exceeded");
    assert!(
        started.elapsed() < CONTROL_DEADLINE,
        "the refusal did not wait for the parked run: {:?}",
        started.elapsed()
    );

    // Another principal is not affected by the first principal's bound.
    let (other_status, other_body) = post_as(addr, WORKER_TOKEN, &run_body(2, 3), CONTROL_DEADLINE);
    assert_eq!(
        other_status,
        200,
        "another principal still runs: {}",
        String::from_utf8_lossy(&other_body)
    );
    assert_eq!(result_number(&other_body, "steps"), 3);

    gate.release();
    let (run_status, run_body) = runner.join().expect("runner thread");
    assert_eq!(run_status, 200, "the parked run completes");
    assert_eq!(result_number(&run_body, "steps"), RUN_STEPS);
    server.shutdown().expect("shutdown");
}
