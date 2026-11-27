#![allow(clippy::expect_used)]

//! W3d regression tests: the six probes of review R1 and the findings behind them.
//!
//! Each test is one of the adversarial probes the independent review ran against the
//! W3.1–W3.3 implementation (`.research/implementation-2026-11-20/review-w3.md`), kept
//! as a permanent regression guard. The finding ids are cited in the test docs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAccess,
    MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash};
use chronofork_service::json::{JsonValue, to_canonical_bytes};
use chronofork_service::{
    ApiVersion, AppService, ErrorCode, Principal, Scope, ServiceConfig, ServiceError, ServiceLimits,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn service(limits: ServiceLimits) -> Arc<AppService> {
    Arc::new(
        AppService::new(ServiceConfig {
            factory: Arc::new(SyntheticBackendFactory::default()),
            storage_root: None,
            artifacts_root: None,
            limits,
        })
        .expect("service"),
    )
}

fn input(text: &str) -> JsonValue {
    chronofork_service::json::parse(text.as_bytes(), &limits()).expect("input JSON")
}

fn request_bytes(op: &str, input: &JsonValue) -> Vec<u8> {
    let request = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
    to_canonical_bytes(&request, &limits()).expect("request bytes")
}

fn dispatch(
    service: &AppService,
    principal: &Principal,
    op: &str,
    text: &str,
) -> Result<JsonValue, ServiceError> {
    dispatch_version(service, principal, op, text, ApiVersion::V2)
}

fn dispatch_version(
    service: &AppService,
    principal: &Principal,
    op: &str,
    text: &str,
    version: ApiVersion,
) -> Result<JsonValue, ServiceError> {
    let bytes = service.dispatch_versioned(principal, &request_bytes(op, &input(text)), version)?;
    let envelope = chronofork_service::json::parse(&bytes, &limits()).expect("envelope");
    Ok(envelope.get("result").expect("result").clone())
}

fn ok(service: &AppService, principal: &Principal, op: &str, text: &str) -> JsonValue {
    dispatch(service, principal, op, text).expect("operation must succeed")
}

fn number(value: &JsonValue, name: &str) -> u64 {
    value
        .get(name)
        .and_then(JsonValue::as_u64)
        .expect("unsigned field")
}

/// A result field that inspect reports under `machine`.
fn machine_number(value: &JsonValue, name: &str) -> u64 {
    value
        .get("machine")
        .and_then(|machine| machine.get(name))
        .and_then(JsonValue::as_u64)
        .expect("unsigned machine field")
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

// ---------------------------------------------------------------------------
// A counting backend: every capture and every construction is observable
// ---------------------------------------------------------------------------

/// Counters shared by every backend instance of one factory.
#[derive(Debug, Default)]
struct Counters {
    created: AtomicUsize,
    dropped: AtomicUsize,
    captured: AtomicUsize,
}

struct CountingBackend {
    inner: Box<dyn Backend>,
    counters: Arc<Counters>,
    /// Number of successful steps after which every later step fails.
    fail_after: Option<usize>,
    steps: usize,
}

impl Backend for CountingBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.inner.state_hash()
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        if let Some(limit) = self.fail_after {
            if self.steps >= limit {
                return Err(BackendError::Halted);
            }
        }
        self.steps += 1;
        self.inner.step(input)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.counters.captured.fetch_add(1, Ordering::SeqCst);
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

impl Drop for CountingBackend {
    fn drop(&mut self) {
        self.counters.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

struct CountingFactory {
    inner: SyntheticBackendFactory,
    counters: Arc<Counters>,
    fail_after: Option<usize>,
}

impl BackendFactory for CountingFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        self.counters.created.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountingBackend {
            inner: self.inner.create()?,
            counters: Arc::clone(&self.counters),
            fail_after: self.fail_after,
            steps: 0,
        }))
    }
}

fn counting_service(limits: ServiceLimits, counters: &Arc<Counters>) -> Arc<AppService> {
    counting_service_with(limits, counters, None)
}

fn counting_service_with(
    limits: ServiceLimits,
    counters: &Arc<Counters>,
    fail_after: Option<usize>,
) -> Arc<AppService> {
    Arc::new(
        AppService::new(ServiceConfig {
            factory: Arc::new(CountingFactory {
                inner: SyntheticBackendFactory::default(),
                counters: Arc::clone(counters),
                fail_after,
            }),
            storage_root: None,
            artifacts_root: None,
            limits,
        })
        .expect("counting service"),
    )
}

fn live_backends(counters: &Counters) -> usize {
    counters
        .created
        .load(Ordering::SeqCst)
        .saturating_sub(counters.dropped.load(Ordering::SeqCst))
}

// ---------------------------------------------------------------------------
// Probe 1 (W3-R1): a request key on any mutation deduplicates
// ---------------------------------------------------------------------------

/// Review probe: `session.create {"operation_id":77}` twice created sessions 1 **and 2**.
///
/// Every journaled mutation marks the commit, so a repeat of the same key replays the
/// recorded outcome instead of applying the mutation again.
#[test]
fn every_journaled_mutation_deduplicates_its_request_key() {
    let service = service(limits());
    let operator = operator();

    // create: the same key must not create a second session.
    let first = ok(
        &service,
        &operator,
        "session.create",
        "{\"operation_id\":77}",
    );
    let again = ok(
        &service,
        &operator,
        "session.create",
        "{\"operation_id\":77}",
    );
    assert_eq!(
        number(&first, "session"),
        number(&again, "session"),
        "the replayed create returns the recorded result"
    );
    assert_eq!(service.session_count().expect("count"), 1);

    // fork: the same key must not fork twice.
    let fork = ok(
        &service,
        &operator,
        "session.fork",
        "{\"session\":1,\"operation_id\":88}",
    );
    let child = number(&fork, "session");
    let fork_again = ok(
        &service,
        &operator,
        "session.fork",
        "{\"session\":1,\"operation_id\":88}",
    );
    assert_eq!(number(&fork_again, "session"), child);
    assert_eq!(service.session_count().expect("count"), 2);

    // snapshot: the same key must not capture twice. The parent already owns one
    // checkpoint — the fork's origin — so the first explicit capture is the second.
    let captured = ok(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":1,\"operation_id\":5}",
    );
    assert_eq!(number(&captured, "snapshots"), 2);
    let captured_again = ok(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":1,\"operation_id\":5}",
    );
    assert_eq!(
        number(&captured_again, "snapshots"),
        2,
        "the replayed snapshot captured nothing"
    );
    assert_eq!(
        number(&captured_again, "snapshot"),
        number(&captured, "snapshot")
    );

    // restore: the same key must not move the state twice (the replay is the recorded
    // result, and the revision does not advance again).
    ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":3}",
    );
    let restored = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":1,\"operation_id\":31}",
    );
    let revision = number(&restored, "revision");
    ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":2}",
    );
    let replay = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":1,\"operation_id\":31}",
    );
    assert_eq!(number(&replay, "revision"), revision);
    let inspected = ok(&service, &operator, "session.inspect", "{\"session\":1}");
    assert_eq!(
        machine_number(&inspected, "tick"),
        2,
        "the replayed restore applied nothing"
    );
}

/// A **failed** journaled mutation is replayed as the same typed error.
#[test]
fn a_replayed_failure_keeps_its_typed_error() {
    let service = service(limits());
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    // First attempt: the session does not exist.
    let first = dispatch(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":9,\"operation_id\":4}",
    )
    .expect_err("unknown session");
    assert_eq!(first.code(), ErrorCode::NotFound);
    // A retry with the same key replays the recorded failure without touching anything,
    // and it is reported as an error, not as a 200 body.
    let retry = dispatch(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":9,\"operation_id\":4}",
    )
    .expect_err("replayed failure");
    assert_eq!(retry.code(), ErrorCode::NotFound);
    assert_eq!(retry.to_string(), first.to_string());
}

// ---------------------------------------------------------------------------
// Probe 2 (W3-R3): an expired key is never re-executed
// ---------------------------------------------------------------------------

/// Review probe: after the TTL the same key re-executed the mutation.
#[test]
fn an_expired_key_is_recognized_and_never_re_executed() {
    // Zero TTL: the result is retired the instant it completes, which makes the horizon
    // deterministic without waiting.
    let service = service(ServiceLimits {
        operation_ttl: 0,
        ..limits()
    });
    let operator = operator();
    let created = ok(
        &service,
        &operator,
        "session.create",
        "{\"operation_id\":21}",
    );
    assert_eq!(number(&created, "session"), 1);

    let retry = dispatch(
        &service,
        &operator,
        "session.create",
        "{\"operation_id\":21}",
    )
    .expect_err("outside the horizon");
    assert_eq!(retry.code(), ErrorCode::UnknownOutcome);
    assert!(
        retry.to_string().contains("did not re-execute"),
        "the refusal says the server did not re-execute: {retry}"
    );
    assert_eq!(
        service.session_count().expect("count"),
        1,
        "the refused retry created nothing"
    );
}

// ---------------------------------------------------------------------------
// Probe 3 (W3-R2): restore moves the revision
// ---------------------------------------------------------------------------

/// Review probe: a restore changed the tick and left the revision unchanged.
#[test]
fn restore_advances_the_revision_so_a_stale_retry_is_detected() {
    let service = service(limits());
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    ok(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":1,\"name\":\"zero\"}",
    );
    let run = ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":4}",
    );
    let after_run = number(&run, "revision");
    assert_eq!(after_run, 1);

    let restored = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":1}",
    );
    assert_eq!(number(&restored, "tick"), 0, "the tick moved backwards");
    let after_restore = number(&restored, "revision");
    assert_eq!(
        after_restore,
        after_run + 1,
        "a restore advances the monotonic revision (ABA)"
    );

    // A retry that pinned the pre-restore revision is rejected before applying anything.
    let stale = dispatch(
        &service,
        &operator,
        "session.restore",
        &format!("{{\"session\":1,\"snapshot\":1,\"expected_revision\":{after_run}}}"),
    )
    .expect_err("stale revision");
    assert_eq!(stale.code(), ErrorCode::Conflict);
    let inspected = ok(&service, &operator, "session.inspect", "{\"session\":1}");
    assert_eq!(number(&inspected, "revision"), after_restore);
}

/// A run that fails after applying a prefix still publishes the prefix and the revision.
#[test]
fn a_run_that_fails_after_a_prefix_publishes_what_it_applied() {
    let counters = Arc::new(Counters::default());
    let service = counting_service_with(
        limits(),
        &counters,
        // The third step fails, so `steps` 5 applies exactly two.
        Some(2),
    );
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    let error = dispatch(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":5,\"operation_id\":404}",
    )
    .expect_err("the backend halts");
    assert_eq!(error.code(), ErrorCode::Halted);

    // The applied prefix is published: two steps, revision advanced, tick 2.
    let inspected = ok(&service, &operator, "session.inspect", "{\"session\":1}");
    assert_eq!(machine_number(&inspected, "tick"), 2);
    assert_eq!(number(&inspected, "revision"), 1);
    // And the journal retains the prefix the failed run reported.
    let progress = ok(
        &service,
        &operator,
        "session.progress",
        "{\"operation\":404}",
    );
    assert_eq!(number(&progress, "performed"), 2);
    assert_eq!(
        progress.get("failed").and_then(JsonValue::as_bool),
        Some(true)
    );
}

// ---------------------------------------------------------------------------
// Probe 4 (W3-R4): close releases the backend with no reader
// ---------------------------------------------------------------------------

/// Review probe: after `close` (without `reap`) the backend was still alive.
#[test]
fn close_releases_the_backend_without_waiting_for_reap() {
    let counters = Arc::new(Counters::default());
    let service = counting_service(limits(), &counters);
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.snapshot", "{\"session\":1}");
    assert_eq!(live_backends(&counters), 1);

    let closed = ok(&service, &operator, "session.close", "{\"session\":1}");
    assert!(number(&closed, "released_checkpoint_bytes") > 0);
    assert_eq!(
        live_backends(&counters),
        0,
        "close drops the registry's owner, so the backend is released immediately"
    );
    // The entry is still a tombstone: the lifecycle error is `closed`, not `not_found`.
    assert_eq!(
        dispatch(&service, &operator, "session.inspect", "{\"session\":1}")
            .expect_err("closed")
            .code(),
        ErrorCode::Closed
    );
    // Reaping the tombstone then reports `not_found`.
    ok(&service, &operator, "session.reap", "{}");
    assert_eq!(
        dispatch(&service, &operator, "session.inspect", "{\"session\":1}")
            .expect_err("reaped")
            .code(),
        ErrorCode::NotFound
    );
}

// ---------------------------------------------------------------------------
// Probe 5 (W3-R5): a rejected create leaks no backend
// ---------------------------------------------------------------------------

/// Review probe: with `max_sessions = 1` a refused create left a second backend alive.
#[test]
fn a_rejected_create_leaks_no_backend() {
    let counters = Arc::new(Counters::default());
    let service = counting_service(
        ServiceLimits {
            max_sessions: 1,
            ..limits()
        },
        &counters,
    );
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    assert_eq!(live_backends(&counters), 1);

    for _ in 0..8 {
        assert_eq!(
            dispatch(&service, &operator, "session.create", "{}")
                .expect_err("limit")
                .code(),
            ErrorCode::LimitExceeded
        );
    }
    assert_eq!(
        live_backends(&counters),
        1,
        "every rejected create rolled its session back: no invisible backend accumulates"
    );
    assert_eq!(service.session_count().expect("count"), 1);
}

// ---------------------------------------------------------------------------
// Probe 6 (W3-R4): a closed entry does not occupy a live slot
// ---------------------------------------------------------------------------

/// Review probe: with `max_sessions = 1`, a closed entry blocked a new session.
#[test]
fn a_closed_entry_does_not_block_a_new_session() {
    let service = service(ServiceLimits {
        max_sessions: 1,
        ..limits()
    });
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.close", "{\"session\":1}");
    let admitted = ok(&service, &operator, "session.create", "{}");
    assert_eq!(
        admitted.get("state").and_then(JsonValue::as_str),
        Some("ready")
    );
    assert_eq!(number(&admitted, "tick"), 0);
    assert_eq!(service.session_count().expect("count"), 1);
}

// ---------------------------------------------------------------------------
// W3-R5: the byte quota rejects before capturing
// ---------------------------------------------------------------------------

/// The byte quota is a **preflight**: no capture and no serialization happen when the
/// record cannot fit.
#[test]
fn the_byte_quota_rejects_before_capturing() {
    let counters = Arc::new(Counters::default());
    let service = counting_service(
        ServiceLimits {
            max_bytes_per_session: 1,
            ..limits()
        },
        &counters,
    );
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    let before = counters.captured.load(Ordering::SeqCst);
    let rejected = dispatch(&service, &operator, "session.snapshot", "{\"session\":1}")
        .expect_err("the budget cannot fit any record");
    assert_eq!(rejected.code(), ErrorCode::LimitExceeded);
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        before,
        "the rejection happened before the backend was captured"
    );
}

/// A fork's origin checkpoint goes through the same per-session quota as a snapshot.
#[test]
fn a_fork_respects_the_parent_snapshot_quota() {
    let service = service(ServiceLimits {
        max_snapshots_per_session: 1,
        ..limits()
    });
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.snapshot", "{\"session\":1}");
    let rejected = dispatch(&service, &operator, "session.fork", "{\"session\":1}")
        .expect_err("the parent's checkpoint quota is full");
    assert_eq!(rejected.code(), ErrorCode::LimitExceeded);
    assert_eq!(
        service.session_count().expect("count"),
        1,
        "the rejected fork admitted no child"
    );
    // The parent still has exactly its own checkpoint.
    let restored = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":1}",
    );
    assert_eq!(number(&restored, "tick"), 0);
}

// ---------------------------------------------------------------------------
// W3-R3: the epoch is published and validated
// ---------------------------------------------------------------------------

/// The process epoch is discoverable and the control path validates a caller's epoch.
#[test]
fn the_process_epoch_is_published_and_validated() {
    let service = service(limits());
    let operator = operator();
    let capabilities = ok(&service, &operator, "capabilities", "{}");
    let epoch = number(&capabilities, "process_epoch");
    assert!(epoch > 0);

    ok(&service, &operator, "session.create", "{}");
    ok(
        &service,
        &operator,
        "session.run",
        "{\"session\":1,\"steps\":2,\"operation_id\":9}",
    );
    let progress = ok(&service, &operator, "session.progress", "{\"operation\":9}");
    assert_eq!(
        number(&progress, "epoch"),
        epoch,
        "the result carries its epoch"
    );
    // A caller naming the right epoch reads the result; a wrong epoch is an unknown
    // outcome, never a guess against another process's journal.
    assert!(
        dispatch(
            &service,
            &operator,
            "session.progress",
            &format!("{{\"operation\":9,\"epoch\":{epoch}}}"),
        )
        .is_ok()
    );
    assert_eq!(
        dispatch(
            &service,
            &operator,
            "session.progress",
            &format!("{{\"operation\":9,\"epoch\":{}}}", epoch + 999),
        )
        .expect_err("foreign epoch")
        .code(),
        ErrorCode::UnknownOutcome
    );
}

// ---------------------------------------------------------------------------
// W3-R7: the v1 wire is served and the v2 surface is refused on it
// ---------------------------------------------------------------------------

#[test]
fn the_v1_wire_refuses_v2_operations_and_fields_before_work() {
    let service = service(limits());
    let operator = operator();
    // A v2-only operation is unknown on v1.
    assert_eq!(
        dispatch_version(&service, &operator, "session.reap", "{}", ApiVersion::V1)
            .expect_err("v2-only")
            .code(),
        ErrorCode::UnknownOp
    );
    // A v2-only input field is rejected before the mutation.
    assert_eq!(
        dispatch_version(
            &service,
            &operator,
            "session.create",
            "{\"operation_id\":3}",
            ApiVersion::V1,
        )
        .expect_err("v2-only field")
        .code(),
        ErrorCode::InvalidRequest
    );
    assert_eq!(service.session_count().expect("count"), 0);
    // The same request on v2 is accepted.
    let created = dispatch_version(
        &service,
        &operator,
        "session.create",
        "{\"operation_id\":3}",
        ApiVersion::V2,
    )
    .expect("v2 create");
    assert_eq!(number(&created, "session"), 1);
}

// ---------------------------------------------------------------------------
// W3-R6 helper: the control path stays reachable while a long run is parked
// ---------------------------------------------------------------------------

/// A parked long run must not block the control read of its own operation.
#[test]
fn control_reads_are_served_while_a_run_is_parked() {
    let gate = Arc::new(ParkingGate::default());
    let service = Arc::new(
        AppService::new(ServiceConfig {
            factory: Arc::new(ParkingFactory {
                inner: SyntheticBackendFactory::default(),
                gate: Arc::clone(&gate),
            }),
            storage_root: None,
            artifacts_root: None,
            limits: limits(),
        })
        .expect("parking service"),
    );
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    gate.arm();
    let runner = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        thread::spawn(move || {
            dispatch(
                &service,
                &operator,
                "session.run",
                "{\"session\":1,\"steps\":1000,\"operation_id\":55}",
            )
        })
    };
    gate.wait_until_waiting();
    // The control read is journal-only, so it answers while the run holds its owner.
    let progress = ok(
        &service,
        &operator,
        "session.progress",
        "{\"operation\":55}",
    );
    assert!(
        progress.get("state").and_then(JsonValue::as_str).is_some(),
        "the control read reports the running operation"
    );
    gate.release();
    let outcome = runner.join().expect("runner");
    assert!(outcome.is_ok(), "the parked run completes once released");
}

#[derive(Default)]
struct ParkingState {
    armed: bool,
    released: bool,
    waiting: bool,
}

#[derive(Default)]
struct ParkingGate {
    state: Mutex<ParkingState>,
    signal: std::sync::Condvar,
}

impl ParkingGate {
    fn arm(&self) {
        let mut state = self.state.lock().expect("gate");
        state.armed = true;
        state.released = false;
        state.waiting = false;
    }

    fn wait(&self) {
        let mut state = self.state.lock().expect("gate");
        if !state.armed {
            return;
        }
        state.armed = false;
        state.waiting = true;
        self.signal.notify_all();
        while !state.released {
            state = self.signal.wait(state).expect("gate wait");
        }
    }

    fn wait_until_waiting(&self) {
        let mut state = self.state.lock().expect("gate");
        while !state.waiting {
            state = self.signal.wait(state).expect("gate wait");
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("gate");
        state.released = true;
        self.signal.notify_all();
    }
}

struct ParkingBackend {
    inner: Box<dyn Backend>,
    gate: Arc<ParkingGate>,
}

impl Backend for ParkingBackend {
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

struct ParkingFactory {
    inner: SyntheticBackendFactory,
    gate: Arc<ParkingGate>,
}

impl BackendFactory for ParkingFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(ParkingBackend {
            inner: self.inner.create()?,
            gate: Arc::clone(&self.gate),
        }))
    }
}
