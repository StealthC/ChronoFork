#![allow(clippy::expect_used)]

//! Operation-level acceptance tests for the transport-agnostic application service.
//!
//! They exercise the public API only: canonical envelopes, ownership, writer leases
//! with real threads, durable store semantics, experiment artifacts, limits and the
//! error taxonomy. Determinism is asserted by comparing canonical bytes against an
//! independent `SessionManager` oracle, never by inspecting private state.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAccess,
    MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash, hash_bytes};
use chronofork_service::json::{JsonValue, to_canonical_bytes};
use chronofork_service::{
    ApiVersion, AppService, ErrorCode, Principal, Scope, ServiceConfig, ServiceError,
    ServiceLimits, parse_token_file,
};
use chronofork_sessions::{SessionManager, SnapshotRepository};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

// ---------------------------------------------------------------------------
// Request helpers
// ---------------------------------------------------------------------------

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn service(limits: ServiceLimits) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits,
    })
    .expect("service")
}

fn service_with(
    limits: ServiceLimits,
    storage_root: Option<&Path>,
    artifacts_root: Option<&Path>,
) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: storage_root.map(Path::to_path_buf),
        artifacts_root: artifacts_root.map(Path::to_path_buf),
        limits,
    })
    .expect("service")
}

fn input(text: &str) -> JsonValue {
    chronofork_service::json::parse(text.as_bytes(), &limits()).expect("input JSON")
}

fn request_bytes(op: &str, input: &JsonValue) -> Vec<u8> {
    let request = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
    to_canonical_bytes(&request, &limits()).expect("request bytes")
}

/// Replaces the operational `process_epoch` value with `0` before a byte comparison.
///
/// Two services are two processes, so they always report different epochs; the epoch is
/// deliberately not emulated state (ADR-0016 §2), so the cross-run byte equality this
/// test asserts covers every field except that one.
fn normalize_epoch(bytes: &[u8]) -> Vec<u8> {
    let limits = limits();
    let document = chronofork_service::json::parse(bytes, &limits).expect("canonical document");
    let JsonValue::Object(mut root) = document else {
        return bytes.to_vec();
    };
    if let Some(JsonValue::Object(result)) = root.get_mut("result") {
        if result.contains_key("process_epoch") {
            result.insert("process_epoch".to_owned(), JsonValue::int(0));
        }
    }
    to_canonical_bytes(&JsonValue::Object(root), &limits).expect("canonical bytes")
}

fn dispatch_bytes(
    service: &AppService,
    principal: &Principal,
    op: &str,
    input: &JsonValue,
) -> Result<Vec<u8>, ServiceError> {
    // These acceptance tests exercise the v2 agent-API contract, so every request
    // declares v2. The v1 matrix has its own tests in the service unit suite and in
    // the wire test file.
    service.dispatch_versioned(principal, &request_bytes(op, input), ApiVersion::V2)
}

fn call(
    service: &AppService,
    principal: &Principal,
    op: &str,
    text: &str,
) -> Result<JsonValue, ServiceError> {
    let bytes = dispatch_bytes(service, principal, op, &input(text))?;
    let envelope = chronofork_service::json::parse(&bytes, &limits()).expect("envelope JSON");
    assert_eq!(
        envelope.get("op").and_then(JsonValue::as_str),
        Some(op),
        "envelope names the operation"
    );
    let result = envelope.get("result").expect("envelope result");
    assert!(result.as_object().is_some(), "envelope result is an object");
    Ok(result.clone())
}

fn ok(service: &AppService, principal: &Principal, op: &str, text: &str) -> JsonValue {
    call(service, principal, op, text).expect("operation must succeed")
}

fn err(service: &AppService, principal: &Principal, op: &str, text: &str) -> ServiceError {
    call(service, principal, op, text).expect_err("operation must be rejected")
}

fn field<'a>(value: &'a JsonValue, name: &str) -> &'a JsonValue {
    value.get(name).expect("result field")
}

fn number(value: &JsonValue, name: &str) -> u64 {
    field(value, name).as_u64().expect("unsigned field")
}

fn text_field<'a>(value: &'a JsonValue, name: &str) -> &'a str {
    field(value, name).as_str().expect("string field")
}

fn array<'a>(value: &'a JsonValue, name: &str) -> &'a [JsonValue] {
    field(value, name).as_array().expect("array field")
}

fn object<'a>(value: &'a JsonValue, name: &str) -> &'a BTreeMap<String, JsonValue> {
    field(value, name).as_object().expect("object field")
}

fn assert_hex64(value: &JsonValue, name: &str) {
    let text = text_field(value, name);
    assert_eq!(text.len(), 64, "{name} is 64 hex characters");
    assert!(
        text.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{name} is lowercase hex"
    );
}

// ---------------------------------------------------------------------------
// Principals
// ---------------------------------------------------------------------------

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn admin() -> Principal {
    Principal::new("admin", [Scope::Admin])
}

fn reader() -> Principal {
    Principal::new("reader", [Scope::Read])
}

fn other() -> Principal {
    Principal::new("other", [Scope::Read, Scope::Write])
}

// ---------------------------------------------------------------------------
// Independent oracle over the session layer
// ---------------------------------------------------------------------------

fn oracle_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn oracle_plan(plan: &[(u64, u32)]) -> (u64, String) {
    let mut manager = oracle_manager();
    let session = manager.create_session().expect("oracle session");
    for (steps, buttons) in plan {
        let input = InputFrame::new(*buttons);
        for _ in 0..*steps {
            manager.step(session, input).expect("oracle step");
        }
    }
    let view = manager.inspect(session).expect("oracle inspect");
    (view.machine.tick.raw(), view.state_hash.to_hex())
}

fn oracle_state(steps: u64, buttons: u32) -> (u64, String) {
    oracle_plan(&[(steps, buttons)])
}

fn oracle_snapshot_bytes(steps: u64, buttons: u32) -> Vec<u8> {
    let mut manager = oracle_manager();
    let session = manager.create_session().expect("oracle session");
    let input = InputFrame::new(buttons);
    for _ in 0..steps {
        manager.step(session, input).expect("oracle step");
    }
    let snapshot = manager.snapshot(session).expect("oracle snapshot");
    manager
        .snapshot_bytes(session, snapshot)
        .expect("oracle snapshot bytes")
}

fn oracle_memory(steps: u64, address: u64, length: usize) -> String {
    let mut manager = oracle_manager();
    let session = manager.create_session().expect("oracle session");
    for _ in 0..steps {
        manager
            .step(session, InputFrame::NEUTRAL)
            .expect("oracle step");
    }
    let bytes = manager
        .read_memory(session, address, length)
        .expect("oracle memory");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
            "chronofork-service-{label}-{}-{sequence}",
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
// A gated backend: `step` blocks until the test releases it
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct GateState {
    armed: bool,
    released: bool,
    waiting: bool,
}

#[derive(Debug, Default)]
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
}

struct GatedBackend {
    inner: Box<dyn Backend>,
    gate: Arc<Gate>,
}

impl Backend for GatedBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.inner.state_hash()
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
        limits: limits(),
    })
    .expect("gated service")
}

// ---------------------------------------------------------------------------
// Shape, catalog and capabilities
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn app_service_is_send_and_sync() {
    assert_send_sync::<AppService>();
    assert_send_sync::<Principal>();
    assert_send_sync::<ServiceError>();
}

#[test]
fn the_catalog_is_exactly_the_twenty_documented_operations_in_order() {
    let service = service(limits());
    let tools = service.tools();
    // Sixteen pre-ADR-0016 operations plus the v2 control operations and the W3.3
    // lifecycle operation. The first sixteen keep their order and flags so an
    // existing client's decoder keeps working; the additions are additive
    // (ADR-0016 §Compatibility).
    let expected = [
        ("capabilities", false, Scope::Read),
        ("session.create", true, Scope::Write),
        ("session.list", false, Scope::Read),
        ("session.inspect", false, Scope::Read),
        ("session.run", true, Scope::Write),
        ("session.fork", true, Scope::Write),
        ("session.snapshot", true, Scope::Write),
        ("session.restore", true, Scope::Write),
        ("session.compare", false, Scope::Read),
        ("session.close", true, Scope::Write),
        ("session.reap", true, Scope::Write),
        ("session.read_memory", false, Scope::Read),
        ("session.cancel", true, Scope::Write),
        ("session.progress", false, Scope::Read),
        ("session.operation", false, Scope::Read),
        ("store.list", false, Scope::Read),
        ("store.load", false, Scope::Read),
        ("store.save", true, Scope::Write),
        ("store.delete", true, Scope::Write),
        ("experiment.run", true, Scope::Write),
    ];
    assert_eq!(tools.len(), expected.len());
    for (tool, (name, mutating, scope)) in tools.iter().zip(expected) {
        assert_eq!(tool.name, name);
        assert_eq!(tool.mutating, mutating, "{name}");
        assert_eq!(tool.scope, scope, "{name}");
        assert!(!tool.description.is_empty(), "{name}");
        assert_eq!(
            tool.input_schema.get("type").and_then(JsonValue::as_str),
            Some("object"),
            "{name} schema is an object"
        );
        assert_eq!(
            tool.input_schema
                .get("additionalProperties")
                .and_then(JsonValue::as_bool),
            Some(false),
            "{name} rejects unknown fields"
        );
        assert!(
            tool.input_schema
                .get("required")
                .and_then(JsonValue::as_array)
                .is_some(),
            "{name} publishes its required fields"
        );
    }
}

#[test]
fn capabilities_describe_service_backend_limits_and_configuration() {
    let service = service(limits());
    let capabilities = ok(&service, &reader(), "capabilities", "{}");
    assert_eq!(
        text_field(&capabilities, "schema"),
        "chronofork.capabilities.v1"
    );
    assert_eq!(text_field(&capabilities, "service_version"), "0.1.0");
    assert_eq!(
        object(&capabilities, "backend")
            .get("id")
            .and_then(JsonValue::as_str),
        Some("chronofork.test.synthetic")
    );
    assert_eq!(
        object(&capabilities, "backend")
            .get("capabilities")
            .and_then(JsonValue::as_object)
            .map(BTreeMap::len),
        Some(14),
        "every capability field is published"
    );
    assert_eq!(array(&capabilities, "ops").len(), 20);
    assert_eq!(
        number(&capabilities, "api_version"),
        chronofork_service::WIRE_VERSION,
        "the v2 wire version is declared"
    );
    // The process epoch is operational metadata, not emulated state: the v2
    // capabilities publishes it so a client can compare it before trusting a
    // reconciled result (ADR-0016 §2). It is the one field two processes may differ in.
    assert!(
        capabilities
            .get("process_epoch")
            .and_then(JsonValue::as_u64)
            .is_some_and(|epoch| epoch > 0),
        "the v2 capabilities publishes its process epoch"
    );
    let limit_fields = object(&capabilities, "limits");
    for name in [
        "max_request_bytes",
        "max_response_bytes",
        "max_steps_per_request",
        "max_memory_read",
        "max_sessions",
        "max_store_entries",
        "max_checkpoints",
        "json_max_depth",
        "max_run_wall_ms",
        "max_operation_journal",
        "operation_ttl",
        "max_closed_retained",
        "closed_retention_ttl",
    ] {
        assert!(limit_fields.contains_key(name), "{name}");
    }
    assert_eq!(
        limit_fields
            .get("max_run_wall_ms")
            .and_then(JsonValue::as_u64),
        Some(30_000)
    );
    assert_eq!(
        limit_fields
            .get("max_operation_journal")
            .and_then(JsonValue::as_u64),
        Some(1_024)
    );
    assert_eq!(
        limit_fields
            .get("operation_ttl")
            .and_then(JsonValue::as_u64),
        Some(600)
    );
    assert_eq!(
        limit_fields
            .get("max_steps_per_request")
            .and_then(JsonValue::as_u64),
        Some(100_000)
    );
    assert_eq!(
        object(&capabilities, "store")
            .get("configured")
            .and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(
        object(&capabilities, "artifacts")
            .get("configured")
            .and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(service.session_count().expect("count"), 0);

    let directory = TempDir::new("capabilities");
    let configured = service_with(limits(), Some(directory.path()), Some(directory.path()));
    let capabilities = ok(&configured, &admin(), "capabilities", "{}");
    assert_eq!(
        object(&capabilities, "store")
            .get("configured")
            .and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(
        object(&capabilities, "artifacts")
            .get("configured")
            .and_then(JsonValue::as_bool),
        Some(true)
    );
}

// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

#[test]
fn session_lifecycle_matches_an_independent_oracle() {
    let service = service(limits());
    let operator = operator();

    let created = ok(&service, &operator, "session.create", "{}");
    assert_eq!(number(&created, "session"), 1);
    assert_eq!(text_field(&created, "state"), "ready");
    assert_eq!(number(&created, "tick"), 0);
    assert_eq!(
        text_field(&created, "backend_id"),
        "chronofork.test.synthetic"
    );
    assert_eq!(text_field(&created, "state_hash"), oracle_state(0, 0).1);
    let lineage = object(&created, "lineage");
    assert_eq!(lineage.get("parent_session"), Some(&JsonValue::Null));
    assert_eq!(lineage.get("origin_snapshot"), Some(&JsonValue::Null));
    assert_eq!(lineage.get("depth").and_then(JsonValue::as_u64), Some(0));

    let listed = ok(&service, &operator, "session.list", "{}");
    assert_eq!(array(&listed, "sessions").len(), 1);
    assert!(!field(&listed, "truncated").as_bool().expect("truncated"));
    let first = &array(&listed, "sessions")[0];
    assert_eq!(number(first, "session"), 1);
    assert_eq!(number(first, "tick"), 0);

    let inspected = ok(&service, &operator, "session.inspect", "{\"session\":1}");
    assert_eq!(text_field(&inspected, "state"), "ready");
    let machine = object(&inspected, "machine");
    assert_eq!(machine.get("tick").and_then(JsonValue::as_u64), Some(0));
    assert_eq!(
        machine.get("halted").and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(
        machine.get("memory_base").and_then(JsonValue::as_u64),
        Some(0)
    );
    assert_eq!(
        machine.get("memory_length").and_then(JsonValue::as_u64),
        Some(256)
    );
    assert_eq!(
        machine
            .get("state_hash")
            .and_then(JsonValue::as_str)
            .map(str::len),
        Some(64)
    );
    assert!(
        machine
            .get("registers")
            .and_then(JsonValue::as_object)
            .is_some_and(|registers| registers.contains_key("pc"))
    );

    let run = ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":10}",
    );
    assert_eq!(number(&run, "steps"), 10);
    assert_eq!(number(&run, "tick"), 10);
    assert!(!field(&run, "halted").as_bool().expect("halted"));
    assert_eq!(text_field(&run, "state_hash"), oracle_state(10, 0).1);

    let snapshot = ok(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":1,\"name\":\"ten\"}",
    );
    assert_eq!(number(&snapshot, "snapshot"), 1);
    assert_eq!(text_field(&snapshot, "name"), "ten");
    assert_eq!(number(&snapshot, "tick"), 10);
    let expected_bytes = oracle_snapshot_bytes(10, 0);
    assert_eq!(
        number(&snapshot, "bytes_length"),
        u64::try_from(expected_bytes.len()).expect("length")
    );
    assert_eq!(
        text_field(&snapshot, "bytes_sha256"),
        hash_bytes(&expected_bytes).to_hex()
    );
    assert_hex64(&snapshot, "bytes_sha256");

    let continued = ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":5,\"buttons\":3}",
    );
    assert_eq!(number(&continued, "tick"), 15);
    assert_eq!(
        text_field(&continued, "state_hash"),
        oracle_plan(&[(10, 0), (5, 3)]).1
    );

    let restored = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":1}",
    );
    assert_eq!(number(&restored, "tick"), 10);
    assert_eq!(text_field(&restored, "state_hash"), oracle_state(10, 0).1);

    let forked = ok(&service, &operator, "session.fork", "{\"session\":1}");
    assert_eq!(number(&forked, "session"), 2);
    assert_eq!(number(&forked, "parent"), 1);
    assert_eq!(number(&forked, "tick"), 10);
    assert_eq!(text_field(&forked, "state_hash"), oracle_state(10, 0).1);
    let lineage = object(&forked, "lineage");
    assert_eq!(
        lineage.get("parent_session").and_then(JsonValue::as_u64),
        Some(1)
    );
    // Forking captures the current state as a new checkpoint; the parent already
    // owns snapshot 1, so the fork's origin is the freshly allocated snapshot 2.
    assert_eq!(
        lineage.get("origin_snapshot").and_then(JsonValue::as_u64),
        Some(2)
    );
    assert_eq!(lineage.get("depth").and_then(JsonValue::as_u64), Some(1));

    let compared = ok(
        &service,
        &operator,
        "session.compare",
        "{\"left\":1,\"right\":2}",
    );
    assert!(
        field(&compared, "equivalent")
            .as_bool()
            .expect("equivalent")
    );
    assert_eq!(number(&compared, "left_tick"), 10);
    assert_eq!(number(&compared, "right_tick"), 10);

    let diverged = ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":2,\"steps\":3}",
    );
    assert_eq!(number(&diverged, "tick"), 13);
    let compared = ok(
        &service,
        &operator,
        "session.compare",
        "{\"left\":1,\"right\":2}",
    );
    assert!(
        !field(&compared, "equivalent")
            .as_bool()
            .expect("equivalent")
    );
    assert_eq!(number(&compared, "left_tick"), 10);
    assert_eq!(number(&compared, "right_tick"), 13);

    let memory = ok(
        &service,
        &operator,
        "session.read_memory",
        "{\"session\":1,\"address\":32,\"length\":4}",
    );
    assert_eq!(number(&memory, "address"), 32);
    assert_eq!(number(&memory, "length"), 4);
    assert_eq!(text_field(&memory, "bytes"), oracle_memory(10, 32, 4));

    let closed = ok(&service, &operator, "session.close", "{\"session\":1}");
    assert_eq!(text_field(&closed, "state"), "closed");
    let closed_again = ok(&service, &operator, "session.close", "{\"session\":1}");
    assert_eq!(text_field(&closed_again, "state"), "closed");

    let listed = ok(&service, &operator, "session.list", "{}");
    assert_eq!(array(&listed, "sessions").len(), 1);
    assert_eq!(number(&array(&listed, "sessions")[0], "session"), 2);

    assert_eq!(
        err(
            &service,
            &operator,
            "session.run",
            "{\"session\":1,\"steps\":1}"
        )
        .code(),
        ErrorCode::Closed
    );
    assert_eq!(
        err(&service, &operator, "session.inspect", "{\"session\":1}").code(),
        ErrorCode::Closed
    );
    assert_eq!(
        err(
            &service,
            &operator,
            "session.restore",
            "{\"session\":1,\"snapshot\":1}"
        )
        .code(),
        ErrorCode::Closed
    );
    assert_eq!(
        err(&service, &operator, "session.snapshot", "{\"session\":1}").code(),
        ErrorCode::Closed
    );
}

#[test]
fn identical_request_sequences_produce_identical_bytes() {
    fn scenario(service: &AppService, principal: &Principal) -> Vec<Vec<u8>> {
        let mut responses = Vec::new();
        let mut step = |op: &str, text: &str| {
            let input = input(text);
            let response = match dispatch_bytes(service, principal, op, &input) {
                Ok(bytes) => bytes,
                Err(error) => error
                    .to_canonical_bytes(&limits())
                    .expect("canonical error bytes"),
            };
            chronofork_service::json::parse(&response, &limits()).expect("canonical response");
            responses.push(response);
        };
        step("capabilities", "{}");
        step("session.create", "{}");
        step("session.inspect", "{\"session\":1}");
        step("session.run", "{\"session\":1,\"steps\":10}");
        step(
            "session.snapshot",
            "{\"session\":1,\"name\":\"checkpoint\"}",
        );
        step("session.run", "{\"session\":1,\"steps\":5,\"buttons\":3}");
        step("session.restore", "{\"session\":1,\"snapshot\":1}");
        step("session.fork", "{\"session\":1}");
        step("session.compare", "{\"left\":1,\"right\":2}");
        step(
            "session.read_memory",
            "{\"session\":1,\"address\":0,\"length\":8}",
        );
        step("session.list", "{}");
        step("session.close", "{\"session\":2}");
        step("experiment.run", "{\"steps\":4,\"checkpoints\":[0,2,4]}");
        step("session.nope", "{}");
        step("session.inspect", "{\"session\":99}");
        step("session.run", "{\"session\":1,\"steps\":0}");
        step("session.create", "{\"unexpected\":1}");
        responses
    }

    let first = service(limits());
    let second = service(limits());
    let left: Vec<Vec<u8>> = scenario(&first, &operator())
        .into_iter()
        .map(|bytes| normalize_epoch(&bytes))
        .collect();
    let right: Vec<Vec<u8>> = scenario(&second, &operator())
        .into_iter()
        .map(|bytes| normalize_epoch(&bytes))
        .collect();
    for (index, (a, b)) in left.iter().zip(&right).enumerate() {
        if a != b {
            eprintln!(
                "step {index}:\n  left  {}\n  right {}",
                String::from_utf8_lossy(a),
                String::from_utf8_lossy(b)
            );
        }
    }
    assert_eq!(left, right, "identical sequences produce identical bytes");
    assert_eq!(left.len(), 17);
}

#[test]
fn halted_machines_report_halted() {
    let factory = SyntheticBackendFactory::new(
        SyntheticProgram::new(vec![0x09, 0xff]).expect("halt program"),
    );
    let service = AppService::new(ServiceConfig {
        factory: Arc::new(factory),
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })
    .expect("service");
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    let run = ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":5}",
    );
    assert!(field(&run, "halted").as_bool().expect("halted"));
    assert_eq!(number(&run, "steps"), 2, "the halt stops the run");
    assert_eq!(number(&run, "tick"), 2);
    assert_eq!(
        err(
            &service,
            &operator,
            "session.run",
            "{\"session\":1,\"steps\":1}"
        )
        .code(),
        ErrorCode::Halted
    );
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

#[test]
fn ownership_is_enforced_for_every_session_operation() {
    let service = service(limits());
    let operator = operator();
    let admin = admin();

    ok(&service, &operator, "session.create", "{}");
    ok(&service, &admin, "session.create", "{}");

    let listed = ok(&service, &operator, "session.list", "{}");
    assert_eq!(array(&listed, "sessions").len(), 1);
    assert_eq!(number(&array(&listed, "sessions")[0], "session"), 1);
    let listed = ok(&service, &admin, "session.list", "{}");
    assert_eq!(array(&listed, "sessions").len(), 2);
    let listed = ok(&service, &other(), "session.list", "{}");
    assert!(array(&listed, "sessions").is_empty());

    for (op, text) in [
        ("session.inspect", "{\"session\":2}"),
        ("session.run", "{\"session\":2,\"steps\":1}"),
        ("session.fork", "{\"session\":2}"),
        ("session.snapshot", "{\"session\":2}"),
        ("session.restore", "{\"session\":2,\"snapshot\":1}"),
        ("session.close", "{\"session\":2}"),
        (
            "session.read_memory",
            "{\"session\":2,\"address\":0,\"length\":1}",
        ),
        ("session.compare", "{\"left\":1,\"right\":2}"),
    ] {
        assert_eq!(
            err(&service, &operator, op, text).code(),
            ErrorCode::Forbidden,
            "{op} must be forbidden for a foreign session"
        );
    }

    // An administrator may observe and mutate any session.
    ok(&service, &admin, "session.inspect", "{\"session\":1}");
    ok(
        &service,
        &admin,
        "session.run",
        "{\"session\":1,\"steps\":2}",
    );
    ok(&service, &admin, "session.snapshot", "{\"session\":1}");
    let fork = ok(&service, &admin, "session.fork", "{\"session\":1}");
    assert_eq!(number(&fork, "parent"), 1);
    let compared = ok(
        &service,
        &admin,
        "session.compare",
        "{\"left\":1,\"right\":2}",
    );
    assert!(field(&compared, "equivalent").as_bool().is_some());

    // The administrator's fork belongs to the administrator, not the owner.
    let fork_id = number(&fork, "session");
    assert_eq!(
        err(
            &service,
            &operator,
            "session.inspect",
            &format!("{{\"session\":{fork_id}}}")
        )
        .code(),
        ErrorCode::Forbidden
    );

    ok(&service, &admin, "session.close", "{\"session\":1}");
    let listed = ok(&service, &operator, "session.list", "{}");
    assert!(array(&listed, "sessions").is_empty());
    assert_eq!(
        err(&service, &operator, "session.inspect", "{\"session\":1}").code(),
        ErrorCode::Closed
    );
}

#[test]
fn unknown_sessions_are_not_found_for_every_principal() {
    let service = service(limits());
    for principal in [operator(), admin(), reader()] {
        assert_eq!(
            err(&service, &principal, "session.inspect", "{\"session\":7}").code(),
            ErrorCode::NotFound
        );
    }
    for principal in [operator(), admin()] {
        assert_eq!(
            err(
                &service,
                &principal,
                "session.run",
                "{\"session\":7,\"steps\":1}"
            )
            .code(),
            ErrorCode::NotFound
        );
    }
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        err(
            &service,
            &admin(),
            "session.restore",
            "{\"session\":1,\"snapshot\":9}"
        )
        .code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(
            &service,
            &operator,
            "session.read_memory",
            "{\"session\":1,\"address\":250,\"length\":10}"
        )
        .code(),
        ErrorCode::Backend
    );
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

#[test]
fn concurrent_mutations_of_one_session_conflict_immediately() {
    let gate = Arc::new(Gate::default());
    let service = Arc::new(gated_service(&gate));
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");

    gate.arm();
    let writer = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        thread::spawn(move || {
            dispatch_bytes(
                &service,
                &operator,
                "session.run",
                &input("{\"session\":1,\"steps\":1}"),
            )
        })
    };
    gate.wait_until_waiting();

    let contested = dispatch_bytes(
        &service,
        &operator,
        "session.run",
        &input("{\"session\":1,\"steps\":1}"),
    )
    .expect_err("a second mutation must fail while the lease is held");
    assert_eq!(contested.code(), ErrorCode::Conflict);
    assert!(contested.is_retryable());

    gate.release();
    let outcome = writer.join().expect("writer thread");
    assert!(
        outcome.is_ok(),
        "the first mutation keeps its lease and succeeds"
    );

    let after = ok(&service, &operator, "session.inspect", "{\"session\":1}");
    assert_eq!(
        object(&after, "machine")
            .get("tick")
            .and_then(JsonValue::as_u64),
        Some(1)
    );
}

#[test]
fn mutations_of_different_sessions_do_not_conflict() {
    let service = Arc::new(service(limits()));
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.create", "{}");

    let mut workers = Vec::new();
    for session in 1..=2_u64 {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        workers.push(thread::spawn(move || {
            dispatch_bytes(
                &service,
                &operator,
                "session.run",
                &input(&format!("{{\"session\":{session},\"steps\":4}}")),
            )
        }));
    }
    for worker in workers {
        let bytes = worker
            .join()
            .expect("worker thread")
            .expect("distinct sessions do not conflict");
        let envelope = chronofork_service::json::parse(&bytes, &limits()).expect("envelope");
        let result = envelope.get("result").expect("result");
        assert_eq!(number(result, "tick"), 4);
    }
}

#[test]
fn reads_are_not_rejected_while_a_mutation_is_in_flight() {
    let gate = Arc::new(Gate::default());
    let service = Arc::new(gated_service(&gate));
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");

    gate.arm();
    let writer = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        thread::spawn(move || {
            dispatch_bytes(
                &service,
                &operator,
                "session.run",
                &input("{\"session\":1,\"steps\":1}"),
            )
        })
    };
    gate.wait_until_waiting();

    // A read dispatched while the mutation holds the writer lease is not rejected
    // with `conflict`; it is serialized behind the manager lock and observes the
    // post-mutation state.
    let reader_handle = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        thread::spawn(move || {
            dispatch_bytes(
                &service,
                &operator,
                "session.inspect",
                &input("{\"session\":1}"),
            )
        })
    };
    thread::sleep(Duration::from_millis(50));
    gate.release();

    writer
        .join()
        .expect("writer thread")
        .expect("the mutation succeeds");
    let bytes = reader_handle
        .join()
        .expect("reader thread")
        .expect("a read is not rejected by a writer lease");
    let envelope = chronofork_service::json::parse(&bytes, &limits()).expect("envelope");
    let machine = object(envelope.get("result").expect("result"), "machine");
    assert_eq!(machine.get("tick").and_then(JsonValue::as_u64), Some(1));
}

// ---------------------------------------------------------------------------
// Durable store
// ---------------------------------------------------------------------------

#[test]
fn store_round_trip_matches_the_documented_semantics() {
    let directory = TempDir::new("store");
    let service = service_with(limits(), Some(directory.path()), None);
    let operator = operator();

    // A configured but uninitialized root is a storage failure, not a panic.
    assert_eq!(
        err(&service, &operator, "store.list", "{}").code(),
        ErrorCode::Storage
    );

    let saved = ok(
        &service,
        &operator,
        "store.save",
        "{\"steps\":10,\"name\":\"alpha\"}",
    );
    assert_eq!(number(&saved, "session_key"), 1);
    assert_eq!(number(&saved, "snapshot_key"), 1);
    assert_eq!(text_field(&saved, "name"), "alpha");
    assert_eq!(number(&saved, "creation_sequence"), 1);
    assert_eq!(number(&saved, "tick"), 10);
    assert_eq!(text_field(&saved, "state_hash"), oracle_state(10, 0).1);
    assert_eq!(field(&saved, "parent_session_key"), &JsonValue::Null);
    assert_eq!(field(&saved, "origin_snapshot_key"), &JsonValue::Null);
    assert_eq!(
        service.session_count().expect("count"),
        0,
        "durable operations never register live sessions"
    );

    let continued = ok(
        &service,
        &operator,
        "store.save",
        "{\"session_key\":1,\"steps\":5}",
    );
    assert_eq!(number(&continued, "session_key"), 1);
    assert_eq!(number(&continued, "snapshot_key"), 2);
    assert_eq!(number(&continued, "creation_sequence"), 2);
    assert_eq!(number(&continued, "tick"), 15);
    assert_eq!(text_field(&continued, "state_hash"), oracle_state(15, 0).1);
    assert_eq!(
        field(&continued, "name"),
        &JsonValue::Null,
        "a continuation without a name publishes an unnamed snapshot"
    );

    let listing = ok(&service, &operator, "store.list", "{}");
    assert_eq!(array(&listing, "sessions").len(), 1);
    let anomalies = object(&listing, "anomalies");
    assert_eq!(anomalies.get("count").and_then(JsonValue::as_u64), Some(0));
    assert_eq!(
        anomalies
            .get("labels")
            .and_then(JsonValue::as_array)
            .map(<[_]>::len),
        Some(0)
    );
    assert!(!field(&listing, "truncated").as_bool().expect("truncated"));
    let session = &array(&listing, "sessions")[0];
    assert_eq!(number(session, "session_key"), 1);
    assert_eq!(text_field(session, "name"), "alpha");
    assert_eq!(field(session, "parent_session_key"), &JsonValue::Null);
    assert_eq!(field(session, "origin_snapshot_key"), &JsonValue::Null);
    let snapshots = array(session, "snapshots");
    assert_eq!(snapshots.len(), 2);
    assert_eq!(number(&snapshots[0], "snapshot_key"), 1);
    assert_eq!(number(&snapshots[0], "tick"), 10);
    assert_eq!(number(&snapshots[0], "creation_sequence"), 1);
    assert_eq!(number(&snapshots[1], "snapshot_key"), 2);
    assert_eq!(number(&snapshots[1], "tick"), 15);
    assert_eq!(
        text_field(&snapshots[0], "state_hash"),
        oracle_state(10, 0).1
    );
    assert_eq!(
        number(&snapshots[0], "checkpoint_length"),
        u64::try_from(oracle_snapshot_bytes(10, 0).len()).expect("length")
    );
    assert_eq!(
        text_field(&snapshots[0], "checkpoint_sha256"),
        hash_bytes(&oracle_snapshot_bytes(10, 0)).to_hex()
    );
    assert_hex64(&snapshots[0], "checkpoint_sha256");
    assert!(
        !snapshots[0]
            .as_object()
            .expect("snapshot object")
            .contains_key("created_at_unix_ms")
    );

    // No host path and no advisory host timestamp travels.
    let listing_bytes = to_canonical_bytes(&listing, &limits()).expect("listing bytes");
    let listing_text = String::from_utf8_lossy(&listing_bytes);
    assert!(
        !listing_text.contains(&directory.path().display().to_string()),
        "no host path is transported"
    );
    assert!(!listing_text.contains("unix_ms"));

    let one = ok(&service, &operator, "store.list", "{\"session_key\":1}");
    assert_eq!(array(&one, "sessions").len(), 1);
    assert_eq!(array(&array(&one, "sessions")[0], "snapshots").len(), 2);

    // Loading the latest snapshot and replaying five steps equals the published
    // continuation, and nothing is written by a load.
    let loaded = ok(
        &service,
        &operator,
        "store.load",
        "{\"session_key\":1,\"steps\":5}",
    );
    assert_eq!(number(&loaded, "session_key"), 1);
    assert_eq!(number(&loaded, "snapshot_key"), 2);
    assert_eq!(number(&loaded, "creation_sequence"), 2);
    assert_eq!(number(&loaded, "tick"), 20);
    assert_eq!(
        text_field(&loaded, "state_hash"),
        oracle_plan(&[(10, 0), (10, 0)]).1
    );
    let loaded_from_first = ok(
        &service,
        &operator,
        "store.load",
        "{\"session_key\":1,\"snapshot_key\":1,\"steps\":5}",
    );
    assert_eq!(number(&loaded_from_first, "snapshot_key"), 1);
    assert_eq!(
        text_field(&loaded_from_first, "state_hash"),
        text_field(&continued, "state_hash")
    );
    let replay_only = ok(
        &service,
        &operator,
        "store.load",
        "{\"session_key\":1,\"snapshot_key\":1}",
    );
    assert_eq!(number(&replay_only, "tick"), 10);
    assert_eq!(number(&replay_only, "creation_sequence"), 1);

    // Unknown keys are typed and never create storage.
    assert_eq!(
        err(&service, &operator, "store.load", "{\"session_key\":9}").code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(
            &service,
            &operator,
            "store.load",
            "{\"session_key\":1,\"snapshot_key\":9}"
        )
        .code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(&service, &operator, "store.list", "{\"session_key\":9}").code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(
            &service,
            &operator,
            "store.delete",
            "{\"session_key\":1,\"snapshot_key\":9}"
        )
        .code(),
        ErrorCode::NotFound
    );

    // Deleting a session that still has snapshots is a conflict, not a cascade.
    assert_eq!(
        err(&service, &operator, "store.delete", "{\"session_key\":1}").code(),
        ErrorCode::Conflict
    );
    let deleted = ok(
        &service,
        &operator,
        "store.delete",
        "{\"session_key\":1,\"snapshot_key\":1}",
    );
    assert_eq!(number(&deleted, "session_key"), 1);
    assert_eq!(number(&deleted, "snapshot_key"), 1);
    assert_eq!(
        err(
            &service,
            &operator,
            "store.delete",
            "{\"session_key\":1,\"snapshot_key\":1}"
        )
        .code(),
        ErrorCode::NotFound
    );
    let listing = ok(&service, &operator, "store.list", "{}");
    assert_eq!(array(&array(&listing, "sessions")[0], "snapshots").len(), 1);
    assert_eq!(
        err(&service, &operator, "store.delete", "{\"session_key\":1}").code(),
        ErrorCode::Conflict,
        "the second snapshot still exists"
    );
    let deleted = ok(
        &service,
        &operator,
        "store.delete",
        "{\"session_key\":1,\"snapshot_key\":2}",
    );
    assert_eq!(number(&deleted, "snapshot_key"), 2);
    let deleted = ok(&service, &operator, "store.delete", "{\"session_key\":1}");
    assert_eq!(field(&deleted, "snapshot_key"), &JsonValue::Null);
    let listing = ok(&service, &operator, "store.list", "{}");
    assert!(array(&listing, "sessions").is_empty());
    assert_eq!(
        err(&service, &operator, "store.delete", "{\"session_key\":1}").code(),
        ErrorCode::NotFound
    );
}

#[test]
fn store_reports_contention_corruption_and_configuration() {
    let directory = TempDir::new("store-contention");
    let operator = operator();

    let unconfigured = service(limits());
    for (op, text) in [
        ("store.list", "{}"),
        ("store.load", "{\"session_key\":1}"),
        ("store.save", "{}"),
        ("store.delete", "{\"session_key\":1}"),
    ] {
        assert_eq!(
            err(&unconfigured, &operator, op, text).code(),
            ErrorCode::NotConfigured,
            "{op} without a storage root"
        );
    }

    let service = service_with(limits(), Some(directory.path()), None);
    ok(&service, &operator, "store.save", "{\"steps\":4}");

    // A writer lock held elsewhere is a typed retryable conflict.
    let repository = SnapshotRepository::open(directory.path()).expect("repository");
    let lock = repository.acquire_writer_lock(1).expect("writer lock");
    let contended = err(&service, &operator, "store.save", "{\"session_key\":1}");
    assert_eq!(contended.code(), ErrorCode::Conflict);
    assert!(contended.is_retryable());
    repository.release_writer_lock(&lock).expect("release");
    let resumed = ok(
        &service,
        &operator,
        "store.save",
        "{\"session_key\":1,\"steps\":1}",
    );
    assert_eq!(number(&resumed, "snapshot_key"), 2);

    // A corrupt checkpoint is reported as an anomaly and never loaded.
    let checkpoint = directory
        .path()
        .join("v1/sessions/session-1/snapshots/snapshot-1/checkpoint.cfsnap");
    fs::write(&checkpoint, b"corrupt").expect("corrupt checkpoint");
    let listing = ok(&service, &operator, "store.list", "{}");
    let anomalies = object(&listing, "anomalies");
    assert_eq!(anomalies.get("count").and_then(JsonValue::as_u64), Some(1));
    assert_eq!(
        anomalies
            .get("labels")
            .and_then(JsonValue::as_array)
            .and_then(|labels| labels
                .iter()
                .map(JsonValue::as_str)
                .collect::<Option<Vec<_>>>()),
        Some(vec!["storage metadata mismatch"]),
        "a corrupt checkpoint is labelled without its path or reason"
    );
    let session = &array(&listing, "sessions")[0];
    assert_eq!(array(session, "snapshots").len(), 1);
    assert_eq!(
        err(
            &service,
            &operator,
            "store.load",
            "{\"session_key\":1,\"snapshot_key\":1}"
        )
        .code(),
        ErrorCode::Storage
    );
}

// ---------------------------------------------------------------------------
// Experiment artifacts
// ---------------------------------------------------------------------------

#[test]
fn experiment_artifacts_are_deterministic_and_content_addressed() {
    let service = service(limits());
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");

    let first = ok(
        &service,
        &operator,
        "experiment.run",
        "{\"steps\":10,\"checkpoints\":[0,5,10]}",
    );
    let second = ok(
        &service,
        &operator,
        "experiment.run",
        "{\"steps\":10,\"checkpoints\":[0,5,10]}",
    );
    assert_eq!(
        first, second,
        "identical requests produce identical results"
    );

    let artifact = field(&first, "artifact");
    assert_eq!(text_field(artifact, "schema"), "chronofork.experiment.v1");
    let backend = object(artifact, "backend");
    assert_eq!(
        backend.get("id").and_then(JsonValue::as_str),
        Some("chronofork.test.synthetic")
    );
    assert_eq!(
        backend.get("test_backend").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(
        backend
            .get("config_hash")
            .and_then(JsonValue::as_str)
            .map(str::len),
        Some(64)
    );
    assert_eq!(
        object(artifact, "input")
            .get("steps")
            .and_then(JsonValue::as_u64),
        Some(10)
    );
    assert_eq!(
        object(artifact, "input")
            .get("buttons")
            .and_then(JsonValue::as_u64),
        Some(0)
    );

    let checkpoints = array(artifact, "checkpoints");
    assert_eq!(checkpoints.len(), 3);
    for (index, checkpoint) in checkpoints.iter().enumerate() {
        let step = number(checkpoint, "step");
        assert_eq!(step, u64::try_from(index).expect("index") * 5);
        assert_eq!(number(checkpoint, "tick"), step);
        assert_eq!(
            text_field(checkpoint, "state_hash"),
            oracle_state(step, 0).1
        );
        assert_hex64(checkpoint, "state_hash");
    }
    let final_state = object(artifact, "final");
    assert_eq!(
        final_state.get("step").and_then(JsonValue::as_u64),
        Some(10)
    );
    assert_eq!(
        final_state.get("tick").and_then(JsonValue::as_u64),
        Some(10)
    );
    assert_eq!(
        final_state.get("halted").and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(
        final_state.get("state_hash").and_then(JsonValue::as_str),
        Some(oracle_state(10, 0).1.as_str())
    );

    let bytes = to_canonical_bytes(artifact, &limits()).expect("artifact bytes");
    let artifact_id = text_field(&first, "artifact_id");
    assert_eq!(artifact_id, hash_bytes(&bytes).to_hex());
    assert_eq!(
        text_field(&second, "artifact_id"),
        artifact_id,
        "the identifier is the digest of the canonical bytes"
    );

    // Invalid checkpoint plans are rejected before any run starts.
    assert_eq!(
        err(
            &service,
            &operator,
            "experiment.run",
            "{\"steps\":10,\"checkpoints\":[5,4]}"
        )
        .code(),
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(
            &service,
            &operator,
            "experiment.run",
            "{\"steps\":4,\"checkpoints\":[5]}"
        )
        .code(),
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&service, &operator, "experiment.run", "{\"steps\":0}").code(),
        ErrorCode::InvalidRequest
    );
}

#[test]
fn experiment_artifacts_persist_atomically() {
    let directory = TempDir::new("artifacts");
    let service = service_with(limits(), None, Some(directory.path()));
    let operator = operator();

    let result = ok(
        &service,
        &operator,
        "experiment.run",
        "{\"steps\":4,\"checkpoints\":[0,4]}",
    );
    let artifact_id = text_field(&result, "artifact_id").to_owned();
    let artifact = field(&result, "artifact");
    let bytes = to_canonical_bytes(artifact, &limits()).expect("artifact bytes");
    let path = directory.path().join(format!("{artifact_id}.cfexp.json"));
    assert_eq!(fs::read(&path).expect("persisted artifact"), bytes);
    assert_eq!(hash_bytes(&bytes).to_hex(), artifact_id);

    // Re-running the same plan leaves exactly one file and identical bytes.
    let again = ok(
        &service,
        &operator,
        "experiment.run",
        "{\"steps\":4,\"checkpoints\":[0,4]}",
    );
    assert_eq!(text_field(&again, "artifact_id"), artifact_id);
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("artifact directory")
            .count(),
        1,
        "no temporary file is left behind"
    );
    assert_eq!(fs::read(&path).expect("persisted artifact"), bytes);

    // A different plan is a different content address and a second file.
    let different = ok(&service, &operator, "experiment.run", "{\"steps\":3}");
    assert_ne!(text_field(&different, "artifact_id"), artifact_id);
    let entries = fs::read_dir(directory.path())
        .expect("artifact directory")
        .count();
    assert_eq!(entries, 2);

    // The response never contains the host artifact path.
    let response = to_canonical_bytes(&result, &limits()).expect("response bytes");
    assert!(!String::from_utf8_lossy(&response).contains(&directory.path().display().to_string()));
}

#[test]
fn experiments_do_not_disturb_live_sessions() {
    let service = service(limits());
    let operator = operator();
    let created = ok(&service, &operator, "session.create", "{}");
    assert_eq!(number(&created, "session"), 1);
    ok(
        &service,
        &operator,
        "experiment.run",
        "{\"steps\":7,\"checkpoints\":[0,7]}",
    );
    let created = ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        number(&created, "session"),
        2,
        "an experiment allocates no live session identifier"
    );
    let listed = ok(&service, &operator, "session.list", "{}");
    assert_eq!(array(&listed, "sessions").len(), 2);
}

#[test]
fn store_list_reports_bounded_path_free_anomaly_labels() {
    let directory = TempDir::new("anomaly-labels");
    let service = service_with(limits(), Some(directory.path()), None);
    let operator = operator();
    ok(&service, &operator, "store.save", "{\"steps\":2}");
    ok(
        &service,
        &operator,
        "store.save",
        "{\"session_key\":1,\"steps\":2}",
    );

    // Corrupt both snapshot metadata records. The listing still succeeds, reports the
    // true anomaly total and names only the category: no path, no reason, no lock
    // owner.
    for snapshot in 1..=2_u64 {
        let metadata = directory.path().join(format!(
            "v1/sessions/session-1/snapshots/snapshot-{snapshot}/snapshot.cfmeta"
        ));
        fs::write(&metadata, b"garbage").expect("corrupt snapshot metadata");
    }
    let listing = ok(&service, &operator, "store.list", "{}");
    let anomalies = object(&listing, "anomalies");
    assert_eq!(anomalies.get("count").and_then(JsonValue::as_u64), Some(2));
    assert_eq!(
        anomalies
            .get("labels")
            .and_then(JsonValue::as_array)
            .and_then(|labels| labels
                .iter()
                .map(JsonValue::as_str)
                .collect::<Option<Vec<_>>>()),
        Some(vec!["corrupt storage metadata"])
    );
    assert_eq!(
        array(&array(&listing, "sessions")[0], "snapshots").len(),
        0,
        "a corrupt snapshot is never listed as committed data"
    );

    let bytes = to_canonical_bytes(&listing, &limits()).expect("listing bytes");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains(&directory.path().display().to_string()),
        "no host path is transported: {text}"
    );
    assert!(
        !text.contains("snapshot.cfmeta"),
        "no storage file name: {text}"
    );
    assert!(!text.contains('/'), "no path separator: {text}");
}

#[test]
fn tokens_never_appear_in_service_outputs() {
    let document = concat!(
        "{\"schema\":\"chronofork.tokens.v1\",\"principals\":[",
        "{\"name\":\"reader\",\"token\":\"reader-secret-token-0001\",\"scopes\":[\"read\"]},",
        "{\"name\":\"writer\",\"token\":\"writer-secret-token-0002\",\"scopes\":[\"read\",\"write\"]},",
        "{\"name\":\"root\",\"token\":\"root-secret-token-0003\",\"scopes\":[\"admin\"]}",
        "]}"
    );
    let table = parse_token_file(document.as_bytes()).expect("token table");
    let credentials: Vec<(&str, &str)> = table.credentials().collect();
    assert_eq!(
        credentials,
        vec![
            ("reader", "reader-secret-token-0001"),
            ("writer", "writer-secret-token-0002"),
            ("root", "root-secret-token-0003"),
        ],
        "credentials are yielded once, in file order"
    );
    assert_eq!(credentials.len(), table.len());

    let directory = TempDir::new("token-leaks");
    let service = service_with(limits(), Some(directory.path()), Some(directory.path()));
    let reader = table
        .authenticate("reader-secret-token-0001")
        .expect("reader principal");
    let writer = table
        .authenticate("writer-secret-token-0002")
        .expect("writer principal");

    let mut responses = vec![
        dispatch_bytes(&service, &reader, "capabilities", &input("{}")).expect("capabilities"),
        dispatch_bytes(&service, &writer, "session.create", &input("{}")).expect("create"),
        dispatch_bytes(&service, &writer, "store.save", &input("{}")).expect("store save"),
        dispatch_bytes(&service, &reader, "store.list", &input("{}")).expect("store list"),
    ];
    for (principal, op, text) in [
        (&writer, "session.inspect", "{\"session\":99}"),
        (&reader, "session.create", "{}"),
        (&writer, "session.nope", "{}"),
    ] {
        responses.push(
            dispatch_bytes(&service, principal, op, &input(text))
                .expect_err("operation must be rejected")
                .to_canonical_bytes(&limits())
                .expect("canonical error bytes"),
        );
    }

    // Every published tool name, description, scope and input schema.
    let mut catalogue = Vec::new();
    for tool in service.tools() {
        let value = JsonValue::object([
            ("name", JsonValue::string(tool.name)),
            ("description", JsonValue::string(tool.description)),
            ("mutating", JsonValue::from_bool(tool.mutating)),
            ("scope", JsonValue::string(tool.scope.as_str())),
            ("input_schema", tool.input_schema),
        ]);
        catalogue
            .extend(to_canonical_bytes(&value, &limits()).expect("canonical tool description"));
    }
    responses.push(catalogue);

    for (name, token) in table.credentials() {
        for response in &responses {
            let text = String::from_utf8_lossy(response);
            assert!(
                !text.contains(token),
                "{name}'s token leaked into a service response: {text}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

#[test]
fn max_sessions_counts_live_sessions_and_closing_frees_a_slot() {
    let operator = operator();
    let service = self::service(ServiceLimits {
        max_sessions: 2,
        ..limits()
    });
    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        err(&service, &operator, "session.create", "{}").code(),
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        err(&service, &operator, "session.fork", "{\"session\":1}").code(),
        ErrorCode::LimitExceeded
    );

    // Closing a session frees its **live** slot: `max_sessions` is
    // `max_live_sessions` (ADR-0016 §3), so the closed entries stay as bounded
    // tombstones while a new session can be admitted.
    ok(&service, &operator, "session.close", "{\"session\":1}");
    ok(&service, &operator, "session.close", "{\"session\":2}");
    let admitted = ok(&service, &operator, "session.create", "{}");
    // The identifier is whatever the monotonic allocator hands out (a rejected create
    // consumes one without leaking a session); the slot count is what proves the point.
    assert!(number(&admitted, "session") >= 3);
    assert_eq!(text_field(&admitted, "state"), "ready");
    assert_eq!(service.session_count().expect("count"), 1);
    // A fork of a closed session is a lifecycle error, not a limit error.
    assert_eq!(
        err(&service, &operator, "session.fork", "{\"session\":1}").code(),
        ErrorCode::Closed
    );
    // The bound still applies to live sessions: with two admitted the next create is
    // refused until one is closed or reaped.
    ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        err(&service, &operator, "session.create", "{}").code(),
        ErrorCode::LimitExceeded
    );

    // Closed entries stay unlisted while the live ones are listed, so the retained
    // tombstones expose no state.
    let listed = ok(&service, &operator, "session.list", "{}");
    assert_eq!(
        array(&listed, "sessions").len(),
        2,
        "only live sessions are listed"
    );
    assert_eq!(
        err(&service, &operator, "session.inspect", "{\"session\":1}").code(),
        ErrorCode::Closed
    );
    assert_eq!(
        err(
            &service,
            &operator,
            "session.run",
            "{\"session\":1,\"steps\":1}"
        )
        .code(),
        ErrorCode::Closed
    );

    // `session_count` keeps its documented live-only meaning.
    assert_eq!(service.session_count().expect("count"), 2);
}

#[test]
fn every_documented_limit_is_enforced_before_bounded_work() {
    let operator = operator();

    let service = self::service(ServiceLimits {
        max_sessions: 1,
        ..limits()
    });
    ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        err(&service, &operator, "session.create", "{}").code(),
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        err(&service, &operator, "session.fork", "{\"session\":1}").code(),
        ErrorCode::LimitExceeded
    );
    // `max_sessions` bounds **live** sessions (ADR-0016 §3): closing the only session
    // frees its slot, so a create/close loop cannot be used to grow the registry but a
    // new session is admitted normally.
    ok(&service, &operator, "session.close", "{\"session\":1}");
    let admitted = ok(&service, &operator, "session.create", "{}");
    assert_eq!(text_field(&admitted, "state"), "ready");
    assert_eq!(service.session_count().expect("count"), 1);
    // A fork of the closed session is a lifecycle error, not a limit error.
    assert_eq!(
        err(&service, &operator, "session.fork", "{\"session\":1}").code(),
        ErrorCode::Closed
    );

    let service = self::service(ServiceLimits {
        max_steps_per_request: 2,
        ..limits()
    });
    ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        err(
            &service,
            &operator,
            "session.run",
            "{\"session\":1,\"steps\":3}"
        )
        .code(),
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        err(&service, &operator, "experiment.run", "{\"steps\":3}").code(),
        ErrorCode::LimitExceeded
    );
    let ran = ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":2}",
    );
    assert_eq!(number(&ran, "tick"), 2);

    let service = self::service(ServiceLimits {
        max_memory_read: 4,
        ..limits()
    });
    ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        err(
            &service,
            &operator,
            "session.read_memory",
            "{\"session\":1,\"address\":0,\"length\":5}"
        )
        .code(),
        ErrorCode::LimitExceeded
    );
    let memory = ok(
        &service,
        &operator,
        "session.read_memory",
        "{\"session\":1,\"address\":0,\"length\":4}",
    );
    assert_eq!(number(&memory, "length"), 4);

    let service = self::service(ServiceLimits {
        max_store_entries: 1,
        ..limits()
    });
    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.create", "{}");
    let listed = ok(&service, &operator, "session.list", "{}");
    assert_eq!(array(&listed, "sessions").len(), 1);
    assert!(field(&listed, "truncated").as_bool().expect("truncated"));

    let service = self::service(ServiceLimits {
        max_checkpoints: 1,
        ..limits()
    });
    assert_eq!(
        err(
            &service,
            &operator,
            "experiment.run",
            "{\"steps\":4,\"checkpoints\":[0,4]}"
        )
        .code(),
        ErrorCode::LimitExceeded
    );

    let service = self::service(ServiceLimits {
        max_response_bytes: 64,
        ..limits()
    });
    assert_eq!(
        err(&service, &reader(), "capabilities", "{}").code(),
        ErrorCode::LimitExceeded
    );

    let service = self::service(ServiceLimits {
        max_request_bytes: 16,
        ..limits()
    });
    let oversized = request_bytes("session.create", &input("{}"));
    assert_eq!(
        service
            .dispatch(&operator, &oversized)
            .expect_err("oversized request")
            .code(),
        ErrorCode::LimitExceeded
    );

    let service = self::service(ServiceLimits {
        json_max_depth: 1,
        ..limits()
    });
    let nested = request_bytes("session.create", &input("{}"));
    assert_eq!(
        service
            .dispatch(&operator, &nested)
            .expect_err("nested request")
            .code(),
        ErrorCode::LimitExceeded
    );
}

// ---------------------------------------------------------------------------
// Errors and authentication
// ---------------------------------------------------------------------------

#[test]
fn errors_are_typed_canonical_and_leak_nothing() {
    let directory = TempDir::new("errors");
    let service = service_with(limits(), Some(directory.path()), None);
    let operator = operator();

    let error = err(&service, &operator, "session.inspect", "{\"session\":3}");
    let bytes = error
        .to_canonical_bytes(&limits())
        .expect("error envelope bytes");
    let envelope = chronofork_service::json::parse(&bytes, &limits()).expect("error envelope");
    assert_eq!(
        envelope.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1")
    );
    let body = envelope
        .get("error")
        .and_then(JsonValue::as_object)
        .expect("error object");
    assert_eq!(
        body.get("code").and_then(JsonValue::as_str),
        Some("not_found")
    );
    assert_eq!(
        body.get("retryable").and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(
        body.get("message").and_then(JsonValue::as_str),
        Some("unknown session")
    );

    let uninitialized = err(&service, &operator, "store.list", "{}");
    assert_eq!(uninitialized.code(), ErrorCode::Storage);
    let path_text = directory.path().display().to_string();
    for error in [
        error,
        uninitialized,
        err(
            &service,
            &operator,
            "session.run",
            "{\"session\":3,\"steps\":1}",
        ),
        err(&service, &operator, "experiment.run", "{\"steps\":0}"),
        err(&service, &reader(), "session.create", "{}"),
        err(&service, &operator, "session.nope", "{}"),
    ] {
        assert!(error.message().len() <= 120, "bounded message: {error}");
        assert!(
            !error.message().contains(&path_text),
            "no host path in {error}"
        );
        assert!(
            !error.message().contains('/'),
            "no path-like message in {error}"
        );
        assert!(
            error.message().is_ascii(),
            "messages stay printable ASCII: {error}"
        );
    }
}

#[test]
fn token_files_authenticate_and_gate_dispatch() {
    let document = concat!(
        "{\"schema\":\"chronofork.tokens.v1\",\"principals\":[",
        "{\"name\":\"reader\",\"token\":\"reader-token-000001\",\"scopes\":[\"read\"]},",
        "{\"name\":\"writer\",\"token\":\"writer-token-000002\",\"scopes\":[\"read\",\"write\"]},",
        "{\"name\":\"root\",\"token\":\"root-token-00000003\",\"scopes\":[\"admin\"]}",
        "]}"
    );
    let table = parse_token_file(document.as_bytes()).expect("token table");
    assert_eq!(table.len(), 3);
    let service = service(limits());

    let reader = table.authenticate("reader-token-000001").expect("reader");
    assert!(dispatch_bytes(&service, &reader, "capabilities", &input("{}")).is_ok());
    assert_eq!(
        dispatch_bytes(&service, &reader, "session.create", &input("{}"))
            .expect_err("read scope cannot mutate")
            .code(),
        ErrorCode::Forbidden
    );

    let writer = table
        .authenticate("writer-token-000002")
        .expect("writer principal");
    ok(&service, &writer, "session.create", "{}");
    let ran = ok(
        &service,
        &writer,
        "session.run",
        "{\"session\":1,\"steps\":1}",
    );
    assert_eq!(number(&ran, "tick"), 1);

    assert_eq!(
        dispatch_bytes(
            &service,
            &reader,
            "session.inspect",
            &input("{\"session\":1}")
        )
        .expect_err("a read principal cannot observe a foreign session")
        .code(),
        ErrorCode::Forbidden
    );

    let root = table
        .authenticate("root-token-00000003")
        .expect("administrator principal");
    ok(&service, &root, "session.inspect", "{\"session\":1}");
    assert!(table.authenticate("unknown-token-000000").is_none());
}
