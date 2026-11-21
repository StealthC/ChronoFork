#![allow(clippy::expect_used)]

//! W3f contract tests: the five findings of the R1c re-review (C1–C5).
//!
//! Each test is one of the adversarial probes R1c ran against the W3e delivery, kept as a
//! permanent regression, plus the failure boundary the probe does not reach:
//!
//! - **C1** — the durable process identity is published with a crash-safe sequence
//!   (write, file sync, atomic rename, directory sync) and any persistence failure refuses
//!   the start instead of publishing an epoch;
//! - **C2** — a fork reserves its slot **before** it captures the parent's origin
//!   checkpoint, so a refused fork captures nothing, builds nothing and consumes no
//!   checkpoint identifier;
//! - **C3** — a create that fails after the backend was built removes that backend on
//!   every early return, so repeated failures leave no invisible live backend;
//! - **C4** — a byte quota is decided before any payload is allocated: a capture the
//!   backend does not bound is refused instead of attempted;
//! - **C5** — a stored checkpoint is journaled before the fallible metadata read that
//!   follows it, so a retry replays the recorded failure instead of capturing twice.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAccess,
    MachineInspection, StepOutcome,
};
use chronofork_core::InputFrame;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{
    ApiVersion, AppService, ErrorCode, Principal, Scope, ServiceConfig, ServiceError, ServiceLimits,
};
use chronofork_test_backend::SyntheticBackendFactory;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn request(op: &str, text: &str) -> Vec<u8> {
    let input = parse(text.as_bytes(), &limits()).expect("input");
    let document = JsonValue::object([("op", JsonValue::string(op)), ("input", input)]);
    to_canonical_bytes(&document, &limits()).expect("request bytes")
}

fn dispatch(service: &AppService, op: &str, text: &str) -> Result<JsonValue, ServiceError> {
    let bytes = service.dispatch_versioned(&operator(), &request(op, text), ApiVersion::V2)?;
    let envelope = parse(&bytes, &limits()).expect("envelope");
    Ok(envelope.get("result").expect("result").clone())
}

fn ok(service: &AppService, op: &str, text: &str) -> JsonValue {
    dispatch(service, op, text).expect("operation must succeed")
}

fn number(value: &JsonValue, name: &str) -> u64 {
    value
        .get(name)
        .and_then(JsonValue::as_u64)
        .expect("unsigned field")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("chronofork-w3f-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn durable_service(root: &std::path::Path) -> Result<AppService, ServiceError> {
    AppService::new_durable(
        ServiceConfig {
            factory: Arc::new(SyntheticBackendFactory::default()),
            storage_root: None,
            artifacts_root: None,
            limits: limits(),
        },
        root,
    )
}

// ---------------------------------------------------------------------------
// A counting backend: construction, capture, inspection failure and payload growth
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Counters {
    created: AtomicUsize,
    dropped: AtomicUsize,
    captured: AtomicUsize,
    growth: AtomicUsize,
    fail_inspect: AtomicBool,
}

struct CountedBackend {
    inner: Box<dyn Backend>,
    counters: Arc<Counters>,
    steps: usize,
}

impl Backend for CountedBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.steps += 1;
        self.inner.step(input)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.counters.captured.fetch_add(1, Ordering::SeqCst);
        let snapshot = self.inner.snapshot()?;
        let growth = if self.steps > 0 {
            self.counters.growth.load(Ordering::SeqCst)
        } else {
            0
        };
        if growth == 0 {
            return Ok(snapshot);
        }
        let mut payload = snapshot.payload().to_vec();
        payload.resize(payload.len() + growth, 0);
        Ok(BackendSnapshot::new(
            snapshot.backend_id().clone(),
            snapshot.backend_version(),
            snapshot.format_version(),
            snapshot.tick(),
            snapshot.config_hash(),
            snapshot.state_hash(),
            payload,
        ))
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        self.inner.restore(snapshot)
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        if self.counters.fail_inspect.load(Ordering::SeqCst) {
            return Err(BackendError::Halted);
        }
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

    // Deliberately no `snapshot_payload_bound`: this backend does not declare one, so the
    // session layer must not assume a bound for it (C4).
}

impl Drop for CountedBackend {
    fn drop(&mut self) {
        self.counters.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

struct CountedFactory {
    inner: SyntheticBackendFactory,
    counters: Arc<Counters>,
}

impl BackendFactory for CountedFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        self.counters.created.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountedBackend {
            inner: self.inner.create()?,
            counters: Arc::clone(&self.counters),
            steps: 0,
        }))
    }
}

fn counted_service(limits: ServiceLimits, counters: &Arc<Counters>) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(CountedFactory {
            inner: SyntheticBackendFactory::default(),
            counters: Arc::clone(counters),
        }),
        storage_root: None,
        artifacts_root: None,
        limits,
    })
    .expect("counting service")
}

fn live_backends(counters: &Counters) -> usize {
    counters
        .created
        .load(Ordering::SeqCst)
        .saturating_sub(counters.dropped.load(Ordering::SeqCst))
}

// ---------------------------------------------------------------------------
// C1 — the durable identity is published crash-safely
// ---------------------------------------------------------------------------

#[test]
fn c1_the_epoch_publish_is_durable_and_fail_closed() {
    let directory = TempDir::new("epoch");
    let root = directory.join("store");
    let identity = directory.join("store.chronofork-epoch");

    let first = durable_service(&root).expect("first start");
    assert_eq!(first.process_epoch(), 1);
    assert_eq!(
        std::fs::read_to_string(&identity).expect("identity file"),
        "1\n"
    );
    // The publish leaves no temporary or lock file behind on success.
    assert!(!directory.join("store.chronofork-epoch.tmp").exists());
    assert!(!directory.join("store.chronofork-epoch.lock").exists());

    // A leftover temporary from a crash between the write and the rename is never read as
    // the identity: the published value is the file, and the next start increments it.
    let temporary = directory.join("store.chronofork-epoch.tmp");
    std::fs::write(&temporary, "999\n").expect("leftover temporary");
    let second = durable_service(&root).expect("second start");
    assert_eq!(
        second.process_epoch(),
        2,
        "a crash between write and rename cannot publish the temporary value"
    );
    assert_eq!(
        std::fs::read_to_string(&identity).expect("identity file"),
        "2\n"
    );

    // Fail-closed: a location that cannot be written refuses the start instead of
    // publishing an epoch the process could not persist.
    let missing = directory.join("absent").join("store");
    let error = durable_service(&missing).expect_err("the identity cannot be written");
    assert_eq!(error.code(), ErrorCode::Storage);
}

// ---------------------------------------------------------------------------
// C2 — fork reserves the slot before it captures the parent
// ---------------------------------------------------------------------------

/// R1c probe: with one live session and `max_sessions = 1`, a fork was refused but still
/// constructed a child, captured the parent's checkpoint and consumed a checkpoint id.
#[test]
fn c2_a_refused_fork_constructs_captures_and_consumes_nothing() {
    let counters = Arc::new(Counters::default());
    let service = counted_service(
        ServiceLimits {
            max_sessions: 1,
            ..limits()
        },
        &counters,
    );
    ok(&service, "session.create", "{}");
    let created = counters.created.load(Ordering::SeqCst);
    let captured = counters.captured.load(Ordering::SeqCst);

    let error = dispatch(&service, "session.fork", "{\"session\":1}")
        .expect_err("the live bound is reached");
    assert_eq!(error.code(), ErrorCode::LimitExceeded);
    assert_eq!(
        counters.created.load(Ordering::SeqCst),
        created,
        "the refused fork constructed no child backend"
    );
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captured,
        "the refused fork captured no parent checkpoint"
    );

    // The parent's checkpoint identifier is untouched: the next capture is id 1.
    let next = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&next, "snapshot"), 1);
    assert_eq!(number(&next, "snapshots"), 1);
}

// ---------------------------------------------------------------------------
// C3 — a create that fails after building the backend leaves nothing alive
// ---------------------------------------------------------------------------

/// R1c probe: with a failing inspection and `max_sessions = 1`, three refused creates left
/// three live backends that the registry never knew about.
#[test]
fn c3_a_failed_initial_inspection_leaks_no_backend() {
    let counters = Arc::new(Counters::default());
    counters.fail_inspect.store(true, Ordering::SeqCst);
    let service = counted_service(
        ServiceLimits {
            max_sessions: 1,
            ..limits()
        },
        &counters,
    );
    for _ in 0..3 {
        assert!(dispatch(&service, "session.create", "{}").is_err());
    }
    assert_eq!(
        service.session_count().expect("count"),
        0,
        "no session was admitted"
    );
    assert_eq!(
        live_backends(&counters),
        0,
        "every refused create removed the backend it had built"
    );
}

// ---------------------------------------------------------------------------
// C4 — the byte quota is decided before the payload is allocated
// ---------------------------------------------------------------------------

/// R1c probe: a payload that grows 2 MiB was captured before the byte quota rejected it,
/// because the layer assumed a constant the backend never declared.
#[test]
fn c4_a_payload_above_any_declared_bound_is_refused_before_the_capture() {
    let counters = Arc::new(Counters::default());
    counters.growth.store(2 * 1024 * 1024, Ordering::SeqCst);
    let service = counted_service(
        ServiceLimits {
            max_bytes_per_session: 1024 * 1024 + 1024,
            ..limits()
        },
        &counters,
    );
    ok(&service, "session.create", "{}");
    ok(&service, "session.run", "{\"session\":1,\"steps\":1}");
    let before = counters.captured.load(Ordering::SeqCst);
    let error = dispatch(&service, "session.snapshot", "{\"session\":1}")
        .expect_err("the capture cannot be bounded");
    assert_eq!(error.code(), ErrorCode::LimitExceeded);
    assert!(
        error
            .message()
            .contains("declares no snapshot payload bound"),
        "the error names the missing backend declaration, not a reached limit: {error}"
    );
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        before,
        "an unbounded capture is refused before the backend allocates it"
    );
}

/// The same quota admits the capture once the backend declares a payload bound.
#[test]
fn c4_a_declared_bound_admits_the_capture_within_the_quota() {
    let service = AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits: ServiceLimits {
            max_bytes_per_session: 64 * 1024,
            ..limits()
        },
    })
    .expect("service");
    ok(&service, "session.create", "{}");
    ok(&service, "session.run", "{\"session\":1,\"steps\":1}");
    // The synthetic backend declares its fixed payload size, so a state changed by the
    // run is still preflighted exactly.
    let captured = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&captured, "snapshots"), 1);
    assert!(number(&captured, "snapshot_bytes") <= 64 * 1024);
}

// ---------------------------------------------------------------------------
// C5 — the stored checkpoint is journaled before the fallible metadata read
// ---------------------------------------------------------------------------

/// R1c probe: a snapshot stored its checkpoint, then failed on the inspection that follows,
/// and the retry captured a **second** checkpoint instead of replaying the failure.
#[test]
fn c5_a_snapshot_committed_before_a_failed_inspection_is_journaled() {
    let counters = Arc::new(Counters::default());
    let service = counted_service(limits(), &counters);
    ok(&service, "session.create", "{}");

    counters.fail_inspect.store(true, Ordering::SeqCst);
    let first = dispatch(
        &service,
        "session.snapshot",
        "{\"session\":1,\"operation_id\":9}",
    )
    .expect_err("the inspection after the capture fails");
    counters.fail_inspect.store(false, Ordering::SeqCst);

    // The retry replays the recorded failure: it must not capture a second checkpoint.
    let retry = dispatch(
        &service,
        "session.snapshot",
        "{\"session\":1,\"operation_id\":9}",
    )
    .expect_err("the recorded failure is replayed");
    assert_eq!(retry.code(), first.code());
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        2,
        "the create's initial capture plus the committed snapshot: the replay added none"
    );

    // The committed checkpoint is visible and no duplicate exists.
    let state = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&state, "snapshots"), 2);
}
