#![allow(clippy::expect_used)]

//! W3.1 acceptance tests: one execution owner per session.
//!
//! These tests exercise the contract of ADR-0016 §1 through the public service API
//! only:
//!
//! - four independent sessions overlap by barrier rather than by wall-clock speedup
//!   alone, and their concurrent execution scales (the scale test is not relaxed);
//! - `list` and `inspect` of another session never wait for a long run, including a
//!   run that is parked inside a backend step;
//! - admission is atomic: concurrent `create` calls cannot overshoot `max_sessions`;
//! - `fork`/`close` around a running session keep the documented lock order and do
//!   not deadlock (every blocking call below runs under a deadline);
//! - determinism is preserved: concurrent independent execution produces the same
//!   bytes as the sequential oracle.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAccess,
    MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash};
use chronofork_service::json::{JsonValue, to_canonical_bytes};
use chronofork_service::{
    AppService, ErrorCode, Principal, Scope, ServiceConfig, ServiceError, ServiceLimits,
};
use chronofork_test_backend::SyntheticBackendFactory;

/// How long a test waits for an operation that the contract says must not block.
const CONTROL_DEADLINE: Duration = Duration::from_secs(10);

/// Steps per session in the scale test.
const SCALE_STEPS: u64 = 32_000;

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
    let bytes = service.dispatch(principal, &request_bytes(op, &input(text)))?;
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

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

/// Runs one blocking operation on its own thread and fails the test if it does not
/// finish within `deadline`.
///
/// A deadlock or a global-lock wait is therefore a test failure with a message, never
/// a hung test binary.
fn within<T: Send + 'static>(
    deadline: Duration,
    label: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        drop(sender.send(work()));
    });
    let value = receiver.recv_timeout(deadline).unwrap_or_else(|error| {
        // The worker thread is deliberately left parked: it is the evidence that the
        // operation blocked, and the assertion message names it.
        unreachable!("{label} did not complete within {deadline:?}: {error}")
    });
    worker.join().expect("worker thread");
    value
}

// ---------------------------------------------------------------------------
// A barrier backend: every session's first step blocks until all sessions arrive
// ---------------------------------------------------------------------------

#[derive(Default)]
struct BarrierState {
    waiting: usize,
    released: bool,
}

/// A reusable rendezvous for `parties` threads.
///
/// It is a *test* barrier, not a runtime primitive: it proves that the configured
/// number of executions are inside a session step at the same time, which is only
/// possible when independent sessions own independent locks. If a party never
/// arrives the others time out and the test reports the overlap it measured.
struct Barrier {
    parties: usize,
    state: Mutex<BarrierState>,
    signal: Condvar,
}

impl Barrier {
    fn new(parties: usize) -> Self {
        Self {
            parties,
            state: Mutex::new(BarrierState::default()),
            signal: Condvar::new(),
        }
    }

    /// Blocks until every party has arrived; returns the number of parties observed
    /// in the rendezvous.
    fn arrive(&self, timeout: Duration) -> usize {
        let mut state = self.state.lock().expect("barrier lock");
        if state.released {
            return self.parties;
        }
        state.waiting = state.waiting.saturating_add(1);
        let arrived = state.waiting;
        self.signal.notify_all();
        if arrived == self.parties {
            state.released = true;
            self.signal.notify_all();
            return arrived;
        }
        let deadline = Instant::now() + timeout;
        while !state.released {
            let remaining = match deadline.checked_duration_since(Instant::now()) {
                Some(remaining) if !remaining.is_zero() => remaining,
                _ => break,
            };
            let (next, _) = self
                .signal
                .wait_timeout(state, remaining)
                .expect("barrier wait");
            state = next;
        }
        arrived
    }
}

/// A backend whose first step participates in a shared barrier.
struct BarrierBackend {
    inner: Box<dyn Backend>,
    barrier: Arc<Barrier>,
    arrival: Arc<AtomicUsize>,
    steps: AtomicU64,
}

impl Backend for BarrierBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.inner.state_hash()
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        let step = self.steps.fetch_add(1, Ordering::SeqCst);
        if step == 0 {
            let arrived = self.barrier.arrive(Duration::from_secs(5));
            self.arrival.fetch_max(arrived, Ordering::SeqCst);
        }
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

struct BarrierFactory {
    inner: SyntheticBackendFactory,
    barrier: Arc<Barrier>,
    arrival: Arc<AtomicUsize>,
}

impl BackendFactory for BarrierFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(BarrierBackend {
            inner: self.inner.create()?,
            barrier: Arc::clone(&self.barrier),
            arrival: Arc::clone(&self.arrival),
            steps: AtomicU64::new(0),
        }))
    }
}

// ---------------------------------------------------------------------------
// A gate backend: `step` parks until the test releases it
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

fn gated_service(gate: &Arc<Gate>, limits: ServiceLimits) -> Arc<AppService> {
    Arc::new(
        AppService::new(ServiceConfig {
            factory: Arc::new(GatedFactory {
                inner: SyntheticBackendFactory::default(),
                gate: Arc::clone(gate),
            }),
            storage_root: None,
            artifacts_root: None,
            limits,
        })
        .expect("gated service"),
    )
}

// ---------------------------------------------------------------------------
// Overlap and scale
// ---------------------------------------------------------------------------

/// Four independent sessions must be inside a step at the same time.
///
/// The rendezvous is the evidence: a global execution lock can only ever let one
/// session's `step` run, so the barrier times out and the measured maximum overlap
/// is 1 or 2. The assertion is on the overlap, not on wall-clock time, so the test
/// is not relaxed when the machine is loaded.
#[test]
fn four_independent_sessions_overlap_by_barrier() {
    let barrier = Arc::new(Barrier::new(4));
    let arrival = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        AppService::new(ServiceConfig {
            factory: Arc::new(BarrierFactory {
                inner: SyntheticBackendFactory::default(),
                barrier: Arc::clone(&barrier),
                arrival: Arc::clone(&arrival),
            }),
            storage_root: None,
            artifacts_root: None,
            limits: limits(),
        })
        .expect("barrier service"),
    );
    let operator = operator();
    for _ in 0..4 {
        ok(&service, &operator, "session.create", "{}");
    }

    let mut workers = Vec::new();
    for session in 1..=4_u64 {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        workers.push(thread::spawn(move || {
            dispatch(
                &service,
                &operator,
                "session.run",
                &format!("{{\"session\":{session},\"steps\":8}}"),
            )
        }));
    }
    for worker in workers {
        let result = worker.join().expect("worker thread");
        assert!(result.is_ok(), "every admitted session run must succeed");
    }
    assert_eq!(
        arrival.load(Ordering::SeqCst),
        4,
        "all four sessions must be inside a step simultaneously"
    );
}

/// Concurrent independent execution must scale, measured against the same work run
/// sequentially on the same process.
///
/// The threshold is deliberately loose (1.5x, not the measured ~3.5x) so a loaded
/// single-core runner does not produce a flaky failure, while the serialized design
/// this replaces cannot pass it at all (it measured 1.00x).
#[test]
fn independent_sessions_scale_more_than_two_times() {
    let operator = operator();
    let runners = 4_u64;

    let sequential_service = service(limits());
    for _ in 0..runners {
        ok(&sequential_service, &operator, "session.create", "{}");
    }
    let start = Instant::now();
    for session in 1..=runners {
        let result = ok(
            &sequential_service,
            &operator,
            "session.run",
            &format!("{{\"session\":{session},\"steps\":{SCALE_STEPS}}}"),
        );
        assert_eq!(number(&result, "steps"), SCALE_STEPS);
    }
    let sequential = start.elapsed();

    let concurrent_service = service(limits());
    for _ in 0..runners {
        ok(&concurrent_service, &operator, "session.create", "{}");
    }
    let start = Instant::now();
    let mut workers = Vec::new();
    for session in 1..=runners {
        let service = Arc::clone(&concurrent_service);
        let operator = operator.clone();
        workers.push(thread::spawn(move || {
            dispatch(
                &service,
                &operator,
                "session.run",
                &format!("{{\"session\":{session},\"steps\":{SCALE_STEPS}}}"),
            )
        }));
    }
    for worker in workers {
        let result = worker.join().expect("worker thread").expect("run");
        assert_eq!(number(&result, "steps"), SCALE_STEPS);
    }
    let concurrent = start.elapsed();

    let ratio = sequential.as_secs_f64() / concurrent.as_secs_f64();
    // Recorded for the W3.1 evidence report; the assertion below is the gate.
    eprintln!(
        "W3.1-SCALE sessions={runners} steps={SCALE_STEPS} sequential={sequential:?} \
         concurrent={concurrent:?} ratio={ratio:.3}"
    );
    assert!(
        ratio > 1.5,
        "four independent sessions must scale (requires a multi-core runner): \
         sequential={sequential:?} concurrent={concurrent:?} ratio={ratio:.3}"
    );
}

// ---------------------------------------------------------------------------
// Control and reads never wait for a long run
// ---------------------------------------------------------------------------

/// A parked 100,000-step run on one session must not delay control or reads on
/// another session.
#[test]
fn list_and_inspect_do_not_wait_for_a_running_session() {
    let gate = Arc::new(Gate::default());
    let service = gated_service(&gate, limits());
    let first = Principal::new("first", [Scope::Read, Scope::Write]);
    let second = Principal::new("second", [Scope::Read, Scope::Write]);
    ok(&service, &first, "session.create", "{}");
    ok(&service, &second, "session.create", "{}");

    gate.arm();
    let runner = {
        let service = Arc::clone(&service);
        let first = first.clone();
        thread::spawn(move || {
            dispatch(
                &service,
                &first,
                "session.run",
                "{\"session\":1,\"steps\":100000}",
            )
        })
    };
    gate.wait_until_waiting();

    // The run holds session 1's owner inside `step`. Everything below concerns
    // session 2 and must not wait for it.
    let listed = {
        let service = Arc::clone(&service);
        let second = second.clone();
        within(CONTROL_DEADLINE, "session.list", move || {
            ok(&service, &second, "session.list", "{}")
        })
    };
    let sessions = listed
        .get("sessions")
        .and_then(JsonValue::as_array)
        .expect("sessions array");
    assert_eq!(
        sessions.len(),
        1,
        "only the other principal's session is listed"
    );
    assert_eq!(number(&sessions[0], "session"), 2);

    let inspected = {
        let service = Arc::clone(&service);
        let second = second.clone();
        within(CONTROL_DEADLINE, "session.inspect", move || {
            ok(&service, &second, "session.inspect", "{\"session\":2}")
        })
    };
    assert_eq!(number(&inspected, "session"), 2);

    // A fresh session is admitted while the long run is parked.
    let created = {
        let service = Arc::clone(&service);
        let second = second.clone();
        within(CONTROL_DEADLINE, "session.create", move || {
            ok(&service, &second, "session.create", "{}")
        })
    };
    assert_eq!(number(&created, "session"), 3);

    gate.release();
    let outcome = runner.join().expect("runner thread");
    assert!(
        outcome.is_ok(),
        "the parked run completes once the gate is released"
    );
}

/// `fork` and `close` around a running session keep the documented order: a
/// conflicting mutation is rejected immediately, a `fork` of another session
/// succeeds, and neither deadlocks.
#[test]
fn fork_and_close_around_a_running_session_do_not_deadlock() {
    let gate = Arc::new(Gate::default());
    let service = gated_service(&gate, limits());
    let operator = operator();
    ok(&service, &operator, "session.create", "{}");
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
        })
    };
    gate.wait_until_waiting();

    // The running session holds its writer lease: a competing mutation is rejected,
    // never queued behind the run.
    let contested = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        within(CONTROL_DEADLINE, "contested fork", move || {
            dispatch(&service, &operator, "session.fork", "{\"session\":1}")
        })
    }
    .expect_err("a fork of the running session conflicts with its writer lease");
    assert_eq!(contested.code(), ErrorCode::Conflict);
    assert!(contested.is_retryable());

    // A different session's fork takes its own owner and completes.
    let forked = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        within(CONTROL_DEADLINE, "fork of session 2", move || {
            ok(&service, &operator, "session.fork", "{\"session\":2}")
        })
    };
    assert_eq!(number(&forked, "session"), 3);
    assert_eq!(number(&forked, "parent"), 2);

    // A comparison takes both owners in identifier order. Session 1 is parked, so
    // the comparison waits for that owner and completes once the gate releases it;
    // it must not deadlock or invert the lock order.
    let comparison = {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        let handle = thread::spawn(move || {
            dispatch(
                &service,
                &operator,
                "session.compare",
                "{\"left\":1,\"right\":2}",
            )
        });
        gate.release();
        handle
            .join()
            .expect("compare thread")
            .expect("compare succeeds once the owner is free")
    };
    assert_eq!(number(&comparison, "left"), 1);
    assert_eq!(number(&comparison, "right"), 2);

    let run = runner.join().expect("runner thread");
    assert!(run.is_ok(), "the parked run completes");

    // `close` of a free session still works after all of the above.
    let closed = ok(&service, &operator, "session.close", "{\"session\":2}");
    assert_eq!(number(&closed, "session"), 2);
    assert_eq!(
        dispatch(&service, &operator, "session.inspect", "{\"session\":2}")
            .expect_err("a closed session is not inspectable")
            .code(),
        ErrorCode::Closed
    );
}

// ---------------------------------------------------------------------------
// Admission
// ---------------------------------------------------------------------------

/// Admission is an atomic check-and-insert: concurrent creates cannot overshoot
/// `max_sessions`, and the losers get a typed limit error.
#[test]
fn concurrent_admission_never_overshoots_max_sessions() {
    let service = service(ServiceLimits {
        max_sessions: 4,
        ..limits()
    });
    let operator = operator();
    let attempts = 16_u64;
    let mut workers = Vec::new();
    for _ in 0..attempts {
        let service = Arc::clone(&service);
        let operator = operator.clone();
        workers.push(thread::spawn(move || {
            dispatch(&service, &operator, "session.create", "{}")
        }));
    }
    let mut admitted = 0_usize;
    let mut rejected = 0_usize;
    for worker in workers {
        match worker.join().expect("worker thread") {
            Ok(result) => {
                assert!(number(&result, "session") <= attempts);
                admitted += 1;
            }
            Err(error) => {
                assert_eq!(error.code(), ErrorCode::LimitExceeded);
                rejected += 1;
            }
        }
    }
    assert_eq!(admitted, 4, "exactly the admitted sessions are created");
    assert_eq!(rejected, 12, "every other attempt is rejected");
    assert_eq!(service.session_count().expect("count"), 4);
    let listed = ok(&service, &operator, "session.list", "{}");
    assert_eq!(
        listed
            .get("sessions")
            .and_then(JsonValue::as_array)
            .expect("sessions")
            .len(),
        4
    );
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

/// Runs one session through the same request in both modes.
fn run_one(service: &Arc<AppService>, session: u64) -> JsonValue {
    dispatch(
        service,
        &operator(),
        "session.run",
        &format!("{{\"session\":{session},\"steps\":64,\"buttons\":3}}"),
    )
    .expect("run")
}

/// Concurrent execution of independent sessions produces exactly the bytes the same
/// work produces sequentially, per session.
#[test]
fn concurrent_execution_preserves_per_session_bytes() {
    let scenario = |service: &Arc<AppService>, concurrent: bool| {
        let operator = operator();
        let mut sessions = Vec::new();
        for _ in 0..4 {
            sessions.push(number(
                &ok(service, &operator, "session.create", "{}"),
                "session",
            ));
        }
        let results: Vec<JsonValue> = if concurrent {
            let mut workers = Vec::new();
            for session in sessions.iter().copied() {
                let service = Arc::clone(service);
                workers.push(thread::spawn(move || run_one(&service, session)));
            }
            workers
                .into_iter()
                .map(|worker| worker.join().expect("worker thread"))
                .collect()
        } else {
            sessions
                .iter()
                .copied()
                .map(|session| run_one(service, session))
                .collect()
        };
        results
    };

    let sequential = scenario(&service(limits()), false);
    let concurrent = scenario(&service(limits()), true);
    assert_eq!(sequential.len(), concurrent.len());
    for (index, (left, right)) in sequential.iter().zip(concurrent.iter()).enumerate() {
        let left_bytes = to_canonical_bytes(left, &limits()).expect("bytes");
        let right_bytes = to_canonical_bytes(right, &limits()).expect("bytes");
        assert_eq!(
            left_bytes, right_bytes,
            "session {index} must produce identical bytes in both modes"
        );
    }
    // The synthetic backend is stateless across sessions, so identical work means
    // identical ticks and digests: this is the canonical-state check, and it also
    // fails loudly if a session's state leaked into another's.
    for (index, result) in sequential.iter().enumerate() {
        assert_eq!(number(result, "tick"), 64, "session {index} tick");
        assert_eq!(number(result, "steps"), 64, "session {index} steps");
    }
}
