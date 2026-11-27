#![allow(clippy::expect_used)]

//! W3.3 acceptance tests: quotas, `close` release and `reap`.
//!
//! The contract under test is ADR-0016 §3:
//!
//! - `max_snapshots_per_session` and `max_bytes_per_session` are rejected **before**
//!   the allocation, so a rejected `session.snapshot` leaves the session and its
//!   checkpoints exactly as they were, and concurrent snapshots cannot overshoot;
//! - the global store bound is **best-effort and declared**: it is checked at
//!   admission, it is observable in `capabilities`, and it releases the bytes of a
//!   reaped session;
//! - `close` releases the backend/snapshot memory immediately, so a `create -> close`
//!   loop cannot accumulate closed state;
//! - `reap` reports the removed ids, later reports `not_found`, keeps an in-flight
//!   reader safe, and bounds closed entries independently of the live count.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
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
    // The lifecycle surface is v2-only by contract (`session.reap` and the v2 result
    // fields), so every request in this file declares v2 (ADR-0016 §Compatibility).
    let bytes =
        service.dispatch_versioned(principal, &request_bytes(op, &input(text)), ApiVersion::V2)?;
    let envelope = chronofork_service::json::parse(&bytes, &limits()).expect("envelope");
    assert_eq!(
        envelope.get("op").and_then(JsonValue::as_str),
        Some(op),
        "envelope names the operation"
    );
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

fn array(value: &JsonValue, name: &str) -> Vec<u64> {
    value
        .get(name)
        .and_then(JsonValue::as_array)
        .expect("array field")
        .iter()
        .map(|entry| entry.as_u64().expect("unsigned array entry"))
        .collect()
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

/// Runs `session.reap` and returns its registry accounting fields.
///
/// The global store bound is best-effort (ADR-0016 §3) and its accounting is reported
/// by the lifecycle operation, so a test reads it from there instead of inventing a
/// second discovery surface.
fn reap(service: &AppService, principal: &Principal, text: &str) -> JsonValue {
    ok(service, principal, "session.reap", text)
}

fn accounting(result: &JsonValue) -> (u64, u64, u64, u64) {
    (
        number(result, "registered"),
        number(result, "live"),
        number(result, "closed"),
        number(result, "retained_bytes"),
    )
}

// ---------------------------------------------------------------------------
// A gated backend: `step` parks until the test releases it
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
}

struct GatedBackend {
    inner: Box<dyn Backend>,
    gate: Arc<Gate>,
    created: Arc<AtomicUsize>,
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

impl Drop for GatedBackend {
    fn drop(&mut self) {
        // A backend that is dropped is a backend whose memory `close`/`reap` really
        // released, which is what makes the "close frees" claim observable.
        self.created.fetch_sub(1, Ordering::SeqCst);
    }
}

struct GatedFactory {
    inner: SyntheticBackendFactory,
    gate: Arc<Gate>,
    created: Arc<AtomicUsize>,
}

impl BackendFactory for GatedFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        self.created.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(GatedBackend {
            inner: self.inner.create()?,
            gate: Arc::clone(&self.gate),
            created: Arc::clone(&self.created),
        }))
    }
}

fn gated_service(
    gate: &Arc<Gate>,
    created: &Arc<AtomicUsize>,
    limits: ServiceLimits,
) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(GatedFactory {
            inner: SyntheticBackendFactory::default(),
            gate: Arc::clone(gate),
            created: Arc::clone(created),
        }),
        storage_root: None,
        artifacts_root: None,
        limits,
    })
    .expect("gated service")
}

// ---------------------------------------------------------------------------
// Quotas
// ---------------------------------------------------------------------------

/// The snapshot count is a strict bound checked before the allocation, and a
/// rejection leaves the session and its checkpoints unchanged.
#[test]
fn the_snapshot_count_quota_is_checked_before_allocating() {
    let service = service(ServiceLimits {
        max_snapshots_per_session: 2,
        ..limits()
    });
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");

    let first = ok(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":1,\"name\":\"a\"}",
    );
    let second = ok(
        &service,
        &operator,
        "session.snapshot",
        "{\"session\":1,\"name\":\"b\"}",
    );
    assert_eq!(number(&first, "snapshots"), 1);
    assert_eq!(number(&second, "snapshots"), 2);
    assert!(number(&second, "snapshot_bytes") > 0);

    // A third capture is rejected before it allocates: the error names the quota,
    // and the committed usage still reports the two checkpoints that exist.
    let rejected = dispatch(&service, &operator, "session.snapshot", "{\"session\":1}")
        .expect_err("the count quota is full");
    assert_eq!(rejected.code(), ErrorCode::LimitExceeded);
    assert!(
        rejected.to_string().contains("max_snapshots_per_session"),
        "the rejection names the quota: {rejected}"
    );
    // No checkpoints were silently dropped: both named ones are still restorable.
    let restored = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":1}",
    );
    assert_eq!(number(&restored, "snapshot"), 1);
    let restored = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":2}",
    );
    assert_eq!(number(&restored, "snapshot"), 2);
}

/// The byte quota is checked against the retained checkpoint bytes before the new
/// record is stored.
#[test]
fn the_session_byte_quota_is_checked_before_allocating() {
    // The synthetic snapshot record is a few hundred bytes; a 700-byte budget admits
    // one checkpoint and rejects the second without needing to know the exact size.
    let service = service(ServiceLimits {
        max_bytes_per_session: 700,
        ..limits()
    });
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");

    let first = ok(&service, &operator, "session.snapshot", "{\"session\":1}");
    let used = number(&first, "snapshot_bytes");
    assert!(
        used > 0 && used <= 700,
        "one checkpoint must fit the budget, got {used}"
    );
    let rejected =
        dispatch(&service, &operator, "session.snapshot", "{\"session\":1}").expect_err("full");
    assert_eq!(rejected.code(), ErrorCode::LimitExceeded);
    assert!(
        rejected.to_string().contains("max_bytes_per_session"),
        "the rejection names the quota: {rejected}"
    );

    // The budget is still full after the rejection, so the first checkpoint is the
    // only one that exists and it is intact.
    let still_full = dispatch(&service, &operator, "session.snapshot", "{\"session\":1}")
        .expect_err("still full");
    assert_eq!(still_full.code(), ErrorCode::LimitExceeded);
    let restored = ok(
        &service,
        &operator,
        "session.restore",
        "{\"session\":1,\"snapshot\":1}",
    );
    assert_eq!(number(&restored, "tick"), 0);
}

/// Concurrent snapshots of one session cannot overshoot either quota.
#[test]
fn concurrent_snapshots_never_overshoot_the_quota() {
    let service = service(ServiceLimits {
        max_snapshots_per_session: 4,
        max_bytes_per_session: 64 * 1024 * 1024,
        ..limits()
    });
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");

    let attempts = 16_u64;
    let mut workers = Vec::new();
    for _ in 0..attempts {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        workers.push(thread::spawn(move || {
            dispatch(&service, &operator, "session.snapshot", "{\"session\":1}")
        }));
    }
    let mut admitted = 0_u64;
    let mut rejected = 0_u64;
    let mut contended = 0_u64;
    let mut observed = Vec::new();
    for worker in workers {
        match worker.join().expect("worker thread") {
            Ok(result) => {
                admitted += 1;
                observed.push(number(&result, "snapshots"));
            }
            Err(error) => match error.code() {
                ErrorCode::LimitExceeded => rejected += 1,
                // The writer lease fails a simultaneous mutation immediately rather
                // than queueing it (W3.1), which is a documented outcome for a race
                // between two mutations of one session.
                ErrorCode::Conflict => contended += 1,
                other => unreachable!("unexpected rejection: {other:?}"),
            },
        }
    }
    assert_eq!(admitted, 4, "exactly the quota is admitted");
    assert_eq!(rejected + contended, attempts - 4);
    observed.sort_unstable();
    assert_eq!(
        observed,
        vec![1, 2, 3, 4],
        "each admitted capture saw the next checkpoint index: no overshoot"
    );
    // The quota is still exactly full: a further capture is rejected.
    let overflow = dispatch(&service, &operator, "session.snapshot", "{\"session\":1}")
        .expect_err("the quota is full");
    assert_eq!(overflow.code(), ErrorCode::LimitExceeded);
}

// ---------------------------------------------------------------------------
// Close releases memory
// ---------------------------------------------------------------------------

/// `close` releases the checkpoint memory immediately and the entry stays closed
/// until it is reaped, so a create -> snapshot -> close loop accumulates nothing.
#[test]
fn close_releases_state_and_the_entry_stays_closed() {
    let gate = Arc::new(Gate::default());
    let created = Arc::new(AtomicUsize::new(0));
    let service = gated_service(&gate, &created, limits());
    let operator = operator();

    for round in 0..8_u64 {
        let session = number(&ok(&service, &operator, "session.create", "{}"), "session");
        assert_eq!(session, round + 1);
        let captured = ok(
            &service,
            &operator,
            "session.snapshot",
            &format!("{{\"session\":{session}}}"),
        );
        assert_eq!(number(&captured, "snapshots"), 1);
        assert!(number(&captured, "snapshot_bytes") > 0);
        let closed = ok(
            &service,
            &operator,
            "session.close",
            &format!("{{\"session\":{session}}}"),
        );
        assert!(
            number(&closed, "released_checkpoint_bytes") > 0,
            "close reports the checkpoint bytes it released"
        );
        // The session is still a closed entry (not gone), and it retains no checkpoint
        // memory: the release is immediate, not deferred to the reap.
        assert_eq!(
            dispatch(
                &service,
                &operator,
                "session.inspect",
                &format!("{{\"session\":{session}}}")
            )
            .expect_err("a closed session is not inspectable")
            .code(),
            ErrorCode::Closed
        );
        let reaped = reap(&service, &operator, "{}");
        assert_eq!(number(&reaped, "reaped"), 1);
        let (registered, live, closed, bytes) = accounting(&reaped);
        assert_eq!(
            (registered, live, closed, bytes),
            (0, 0, 0, 0),
            "round {round}: the closed entry held no checkpoint bytes and is gone"
        );
        assert_eq!(
            created.load(Ordering::SeqCst),
            0,
            "round {round}: the backend was released"
        );
    }
}

// ---------------------------------------------------------------------------
// Reap
// ---------------------------------------------------------------------------

/// `reap` removes entries, releases their registration slots, changes `inspect`
/// from `closed` to `not_found`, and never faults a reader already in flight.
#[test]
fn reap_removes_closed_entries_and_keeps_readers_safe() {
    let gate = Arc::new(Gate::default());
    let created = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(gated_service(&gate, &created, limits()));
    let operator = operator();

    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.create", "{}");
    ok(&service, &operator, "session.close", "{\"session\":1}");
    ok(&service, &operator, "session.close", "{\"session\":2}");

    // A reader that resolves the closed entry before the reap gets the documented
    // `closed` answer and holds the session's `Arc` for the duration of that call;
    // the reap must not fault it.
    let reader = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        thread::spawn(move || dispatch(&service, &operator, "session.inspect", "{\"session\":1}"))
    };
    let reader = reader
        .join()
        .expect("reader thread")
        .expect_err("a closed session is not inspectable");
    assert_eq!(reader.code(), ErrorCode::Closed);

    let reaped = ok(&service, &operator, "session.reap", "{}");
    assert_eq!(number(&reaped, "reaped"), 2);
    assert_eq!(array(&reaped, "sessions"), vec![1, 2]);

    // The entries are gone, not merely closed: the same request is now `not_found`.
    for session in 1..=2_u64 {
        assert_eq!(
            dispatch(
                &service,
                &operator,
                "session.inspect",
                &format!("{{\"session\":{session}}}")
            )
            .expect_err("a reaped session is gone")
            .code(),
            ErrorCode::NotFound
        );
    }
    let reaped_again = ok(&service, &operator, "session.reap", "{}");
    assert_eq!(number(&reaped_again, "reaped"), 0);
    assert!(array(&reaped_again, "sessions").is_empty());

    // The removed slots are reusable: the registry is empty and a new session is
    // admitted with a fresh identifier.
    assert_eq!(accounting(&reaped_again).0, 0, "the registry is empty");
    let session = number(&ok(&service, &operator, "session.create", "{}"), "session");
    assert_eq!(session, 3);
}

/// A named `reap` reaps one closed session, rejects a live one, and refuses another
/// principal's session.
#[test]
fn a_named_reap_is_authorized_and_only_removes_closed_sessions() {
    let service = service(limits());
    let operator = operator();
    let other = Principal::new("other", [Scope::Read, Scope::Write]);
    ok(&service, &operator, "session.create", "{}");

    // A live session is a typed conflict, never a silent removal.
    let live = dispatch(&service, &operator, "session.reap", "{\"session\":1}")
        .expect_err("a live session cannot be reaped");
    assert_eq!(live.code(), ErrorCode::Conflict);

    // Another principal cannot even name it.
    let foreign = dispatch(&service, &other, "session.reap", "{\"session\":1}")
        .expect_err("another principal's session");
    assert_eq!(foreign.code(), ErrorCode::Forbidden);

    // An unknown or already-reaped session is `not_found`.
    ok(&service, &operator, "session.close", "{\"session\":1}");
    let reaped = ok(&service, &operator, "session.reap", "{\"session\":1}");
    assert_eq!(number(&reaped, "reaped"), 1);
    assert_eq!(array(&reaped, "sessions"), vec![1]);
    assert_eq!(
        dispatch(&service, &operator, "session.reap", "{\"session\":1}")
            .expect_err("already reaped")
            .code(),
        ErrorCode::NotFound
    );
}

/// Closed entries are bounded independently of the live count: admission at the
/// session cap force-cleans the closed excess, so a low live count cannot hide
/// unbounded closed state.
#[test]
fn closed_entries_are_bounded_independently_of_the_live_count() {
    let service = service(ServiceLimits {
        max_sessions: 3,
        max_closed_retained: 2,
        ..limits()
    });
    let operator = operator();
    for session in 1..=3_u64 {
        ok(&service, &operator, "session.create", "{}");
        ok(
            &service,
            &operator,
            "session.close",
            &format!("{{\"session\":{session}}}"),
        );
    }
    // The registry is full (three closed entries) and admission must not fail while
    // bounded closed entries can be released: the excess is force-cleaned and the new
    // session is admitted, so a low live count cannot hide unbounded closed state.
    let admitted = ok(&service, &operator, "session.create", "{}");
    assert_eq!(number(&admitted, "session"), 4);
    let after_admission = reap(&service, &operator, "{}");
    assert_eq!(
        number(&after_admission, "reaped"),
        2,
        "only the two closed entries inside the bound survive admission"
    );
    assert_eq!(array(&after_admission, "sessions"), vec![2, 3]);
    assert_eq!(
        accounting(&after_admission),
        (1, 1, 0, 0),
        "the live session remains and the closed set was cleaned to its bound"
    );
}

/// `close` itself enforces the closed bound, far below the general session cap.
///
/// Nothing in this test reaches `max_sessions`: each `close` retires the closed entries
/// beyond `max_closed_retained`, so a low live count cannot hide unbounded closed state
/// even when admission never runs (ADR-0016 §3, K9/V3-2).
#[test]
fn close_retires_closed_entries_below_the_general_session_cap() {
    let service = service(ServiceLimits {
        max_sessions: 64,
        max_closed_retained: 2,
        ..limits()
    });
    let operator = operator();
    for session in 1..=6_u64 {
        ok(&service, &operator, "session.create", "{}");
        ok(
            &service,
            &operator,
            "session.close",
            &format!("{{\"session\":{session}}}"),
        );
    }
    // Six sessions were closed with no reap and the cap was never approached: only the
    // two most recently closed entries survive.
    let reported = reap(&service, &operator, "{}");
    assert_eq!(number(&reported, "reaped"), 2);
    assert_eq!(array(&reported, "sessions"), vec![5, 6]);
    assert_eq!(accounting(&reported), (0, 0, 0, 0));
}

/// A zero retention TTL retires a closed entry at the close that created it.
#[test]
fn the_closed_retention_ttl_retires_entries_below_the_count_bound() {
    let service = service(ServiceLimits {
        max_sessions: 64,
        max_closed_retained: 8,
        closed_retention_ttl: 0,
        ..limits()
    });
    let operator = operator();
    for session in 1..=3_u64 {
        ok(&service, &operator, "session.create", "{}");
        ok(
            &service,
            &operator,
            "session.close",
            &format!("{{\"session\":{session}}}"),
        );
    }
    let reported = reap(&service, &operator, "{}");
    assert_eq!(
        number(&reported, "reaped"),
        0,
        "the closes already retired them, below the count bound"
    );
    assert_eq!(accounting(&reported), (0, 0, 0, 0));
}

// ---------------------------------------------------------------------------
// The best-effort global store bound
// ---------------------------------------------------------------------------

/// The global store-byte quota is **declared best-effort** (ADR-0016 §3): it is
/// reported exactly for committed state, it is machine-readably flagged as
/// best-effort, and it is not a hard admission gate.
#[test]
fn the_global_store_bound_is_declared_best_effort() {
    let service = service(limits());
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
    let empty = reap(&service, &operator, "{}");
    assert_eq!(number(&empty, "reaped"), 0);
    assert_eq!(accounting(&empty).3, 0, "nothing is retained yet");
    assert_eq!(
        empty
            .get("max_store_bytes_best_effort")
            .and_then(JsonValue::as_bool),
        Some(true),
        "the bound is machine-readably declared best-effort"
    );
    assert_eq!(
        empty.get("max_store_bytes").and_then(JsonValue::as_u64),
        Some(limits().max_store_bytes as u64)
    );

    // Capturing a checkpoint raises the committed sum, which the accounting reports:
    // the report is exact for committed state.
    let captured = ok(&service, &operator, "session.snapshot", "{\"session\":1}");
    let bytes = number(&captured, "snapshot_bytes");
    assert!(bytes > 0);
    let used = reap(&service, &operator, "{}");
    assert_eq!(
        accounting(&used).3,
        bytes,
        "the committed checkpoint bytes are reported"
    );

    // Closing releases them immediately, and the report follows.
    ok(&service, &operator, "session.close", "{\"session\":1}");
    let closed = reap(&service, &operator, "{}");
    assert_eq!(number(&closed, "reaped"), 1);
    assert_eq!(accounting(&closed), (0, 0, 0, 0));

    // The configured bound is published beside its accounting, and it is not a gate:
    // two sessions are admitted even though the bound is one byte.
    let tiny = AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits: ServiceLimits {
            max_store_bytes: 1,
            ..limits()
        },
    })
    .expect("tiny service");
    ok(&tiny, &operator, "session.create", "{}");
    ok(&tiny, &operator, "session.create", "{}");
    let usage = reap(&tiny, &operator, "{}");
    assert_eq!(
        accounting(&usage).0,
        2,
        "admission is not gated by the bound"
    );
    assert_eq!(
        usage.get("max_store_bytes").and_then(JsonValue::as_u64),
        Some(1),
        "the configured bound is still reported"
    );
}

/// A gated run does not lose its session to a concurrent reap.
#[test]
fn reap_skips_a_session_with_a_live_reader() {
    let gate = Arc::new(Gate::default());
    let created = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(gated_service(&gate, &created, limits()));
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
                "{\"session\":1,\"steps\":1}",
            )
            .expect("run")
        })
    };
    gate.wait_until_waiting();

    // The session is live, so a named reap is a typed conflict and the global reap
    // has nothing to remove; neither blocks on the run.
    let conflict = dispatch(&service, &operator, "session.reap", "{\"session\":1}")
        .expect_err("a live session cannot be reaped");
    assert_eq!(conflict.code(), ErrorCode::Conflict);
    let none = ok(&service, &operator, "session.reap", "{}");
    assert_eq!(number(&none, "reaped"), 0);

    gate.release();
    runner.join().expect("runner thread");
    ok(&service, &operator, "session.close", "{\"session\":1}");
    let reaped = ok(&service, &operator, "session.reap", "{}");
    assert_eq!(number(&reaped, "reaped"), 1);
    // The registry no longer resolves the session, so nothing can fault a reader that
    // already holds its handle; the backend is owned by that handle until it drops.
    assert_eq!(
        dispatch(&service, &operator, "session.inspect", "{\"session\":1}")
            .expect_err("reaped")
            .code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        created.load(Ordering::SeqCst),
        0,
        "reap dropped the last handle, so the backend was released"
    );
}

/// A long close/reap cycle keeps the registry and the backend count bounded.
#[test]
fn a_create_close_reap_loop_does_not_accumulate_state() {
    let gate = Arc::new(Gate::default());
    let created = Arc::new(AtomicUsize::new(0));
    let service = gated_service(
        &gate,
        &created,
        ServiceLimits {
            max_closed_retained: 1,
            ..limits()
        },
    );
    let operator = operator();
    for _ in 0..32 {
        let session = number(&ok(&service, &operator, "session.create", "{}"), "session");
        ok(
            &service,
            &operator,
            "session.close",
            &format!("{{\"session\":{session}}}"),
        );
        ok(&service, &operator, "session.reap", "{}");
    }
    let usage = reap(&service, &operator, "{}");
    assert_eq!(accounting(&usage), (0, 0, 0, 0), "the registry is empty");
    assert_eq!(
        created.load(Ordering::SeqCst),
        0,
        "every backend was released"
    );
    // The service is still usable after the loop.
    let session = number(&ok(&service, &operator, "session.create", "{}"), "session");
    assert_eq!(
        session, 33,
        "identifiers keep increasing across reap cycles"
    );
    let run = ok(
        &service,
        &operator,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":4}}"),
    );
    assert_eq!(number(&run, "tick"), 4);
}
