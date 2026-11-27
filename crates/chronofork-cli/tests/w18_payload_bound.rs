//! W1.8 — the Mega Drive checkpoint payload bound, end to end over the service.
//!
//! The composition root is the only place allowed to compose the service with a
//! concrete backend, so the service+Mega Drive regression for
//! [`SNAPSHOT_PAYLOAD_BOUND`] lives here. The tests drive the public canonical API
//! (`create` → `snapshot` → `run(1)` → `snapshot`/`fork`/`restore`) under a
//! **finite** `max_bytes_per_session` and prove that a capture which cannot fit is
//! refused with `limit_exceeded` **before** the backend is captured, leaving the
//! refused session's checkpoint set and identifier counter untouched.
//!
//! The serialized size of one checkpoint record and the service's preflight
//! predictions are measured from the session layer and the service itself, never
//! copied from a document, so a payload-format change fails these tests instead of
//! silently invalidating them.
//!
//! This binary carries **no lint suppression**: the workspace denies
//! `clippy::expect_used`/`clippy::unwrap_used` in tests too, so a failed
//! precondition here panics through `panic!`, which the workspace keeps allowed.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAccess,
    MachineInspection, MediaSource, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash};
use chronofork_core::{SessionId, SnapshotId};
use chronofork_megadrive::{MegadriveFactory, SNAPSHOT_PAYLOAD_BOUND, demo_rom};
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{
    ApiVersion, AppService, ErrorCode, Principal, Scope, ServiceConfig, ServiceError, ServiceLimits,
};
use chronofork_sessions::SessionManager;

// ---------------------------------------------------------------------------
// Failure helpers (no `expect`/`unwrap`: the workspace denies them in tests too)
// ---------------------------------------------------------------------------

/// Unwraps a `Result`, panicking with a contextual message on the error.
macro_rules! require_ok {
    ($value:expr, $message:expr) => {
        match $value {
            Ok(value) => value,
            Err(error) => panic!("{}: {error:?}", $message),
        }
    };
}

/// Unwraps an `Option`, panicking with a contextual message when it is absent.
macro_rules! require_some {
    ($value:expr, $message:expr) => {
        match $value {
            Some(value) => value,
            None => panic!("{}: value is absent", $message),
        }
    };
}

/// Extracts the error of a `Result`, panicking with a contextual message on `Ok`.
macro_rules! require_err {
    ($value:expr, $message:expr) => {
        match $value {
            Err(error) => error,
            Ok(value) => panic!("{}: call unexpectedly succeeded with {value:?}", $message),
        }
    };
}

// ---------------------------------------------------------------------------
// A counting backend: every `Backend::snapshot` call is observable
// ---------------------------------------------------------------------------

/// Counts the capture calls of every backend instance one factory produced.
#[derive(Default)]
struct Counters {
    captured: AtomicUsize,
}

/// Delegating wrapper that counts every [`Backend::snapshot`] call.
///
/// It is the proof that a refused capture never reaches the backend: the inner
/// Mega Drive backend only sees a call this wrapper passes through.
struct CountingBackend {
    inner: Box<dyn Backend>,
    counters: Arc<Counters>,
}

impl Backend for CountingBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.inner.state_hash()
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.inner.step(input)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.counters.captured.fetch_add(1, Ordering::SeqCst);
        self.inner.snapshot()
    }

    fn snapshot_payload_bound(&self) -> Option<usize> {
        // The wrapper must not change the declaration under test.
        self.inner.snapshot_payload_bound()
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

    fn media_mut(&mut self) -> Option<&mut dyn MediaSource> {
        self.inner.media_mut()
    }
}

/// Wraps a Mega Drive factory, counting captures through [`CountingBackend`].
struct CountingFactory {
    inner: MegadriveFactory,
    counters: Arc<Counters>,
}

impl CountingFactory {
    fn new(counters: &Arc<Counters>) -> Result<Self, BackendError> {
        Ok(Self {
            inner: MegadriveFactory::new(demo_rom())?,
            counters: Arc::clone(counters),
        })
    }
}

impl BackendFactory for CountingFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(CountingBackend {
            inner: self.inner.create()?,
            counters: Arc::clone(&self.counters),
        }))
    }
}

// ---------------------------------------------------------------------------
// Service harness
// ---------------------------------------------------------------------------

fn service_with_quota(quota: usize, counters: &Arc<Counters>) -> AppService {
    let factory = require_ok!(CountingFactory::new(counters), "counting factory");
    require_ok!(
        AppService::new(ServiceConfig {
            factory: Arc::new(factory),
            storage_root: None,
            artifacts_root: None,
            limits: ServiceLimits {
                max_bytes_per_session: quota,
                ..ServiceLimits::default()
            },
        }),
        "service"
    )
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn request_bytes(op: &str, text: &str) -> Vec<u8> {
    let limits = ServiceLimits::default();
    let input = require_ok!(
        parse(text.as_bytes(), &limits),
        "request input is valid JSON"
    );
    let request = JsonValue::object([("op", JsonValue::string(op)), ("input", input)]);
    require_ok!(
        to_canonical_bytes(&request, &limits),
        "canonical request bytes"
    )
}

fn dispatch(service: &AppService, op: &str, text: &str) -> Result<JsonValue, ServiceError> {
    let bytes =
        service.dispatch_versioned(&operator(), &request_bytes(op, text), ApiVersion::V2)?;
    let limits = ServiceLimits::default();
    let envelope = require_ok!(parse(&bytes, &limits), "canonical response is valid JSON");
    assert_eq!(
        envelope.get("op").and_then(JsonValue::as_str),
        Some(op),
        "the envelope names the operation"
    );
    Ok(require_some!(envelope.get("result"), "result").clone())
}

fn ok(service: &AppService, op: &str, text: &str) -> JsonValue {
    require_ok!(dispatch(service, op, text), "operation must succeed")
}

fn number(value: &JsonValue, name: &str) -> u64 {
    require_some!(
        value.get(name).and_then(JsonValue::as_u64),
        "unsigned field in the result"
    )
}

fn record_len(value: &JsonValue) -> usize {
    usize::try_from(number(value, "bytes_length")).unwrap_or(usize::MAX)
}

// ---------------------------------------------------------------------------
// Measurement helpers
// ---------------------------------------------------------------------------

/// Exact serialized size of one checkpoint record for the demo ROM.
fn measured_record_bytes() -> usize {
    let manager = SessionManager::new(Arc::new(require_ok!(
        MegadriveFactory::new(demo_rom()),
        "valid demo ROM"
    )));
    let mut manager = manager;
    let session = require_ok!(manager.create_session(), "session");
    let snapshot = require_ok!(manager.snapshot(session), "snapshot");
    require_ok!(
        manager.snapshot_bytes(session, snapshot),
        "serialized checkpoint"
    )
    .len()
}

/// The byte count the service's preflight checks against the quota.
///
/// Read through the **public** rejection the service reports under a one-byte
/// quota, never copied from a document. `fresh` selects the prediction of a state
/// whose exact observed size is still valid; otherwise the prediction is derived
/// from [`SNAPSHOT_PAYLOAD_BOUND`] after a `run` cleared that observation.
fn preflight_bytes(fresh: bool) -> usize {
    let counters = Arc::new(Counters::default());
    let service = service_with_quota(1, &counters);
    ok(&service, "session.create", "{}");
    if !fresh {
        ok(&service, "session.run", "{\"session\":1,\"steps\":1}");
    }
    let rejected = require_err!(
        dispatch(&service, "session.snapshot", "{\"session\":1}"),
        "a one-byte quota cannot hold a record"
    );
    assert_eq!(rejected.code(), ErrorCode::LimitExceeded);
    let message = rejected.to_string();
    let problem = format!("the rejection names the predicted byte count: {message}");
    require_some!(
        message
            .split(' ')
            .find_map(|token| token.trim_end_matches(',').parse::<usize>().ok()),
        &problem
    )
}

/// Whether the exact `create → snapshot → run → snapshot …` sequence accepts
/// `captures` captures under the quota.
fn accepts_captures(quota: usize, captures: usize, counters: &Arc<Counters>) -> bool {
    let service = service_with_quota(quota, counters);
    if dispatch(&service, "session.create", "{}").is_err() {
        return false;
    }
    for index in 0..captures {
        if index == 1 && dispatch(&service, "session.run", "{\"session\":1,\"steps\":1}").is_err() {
            return false;
        }
        if dispatch(&service, "session.snapshot", "{\"session\":1}").is_err() {
            return false;
        }
    }
    true
}

/// Smallest quota under which the sequence accepts `captures` captures.
fn min_quota_for_captures(captures: usize, counters: &Arc<Counters>) -> usize {
    let mut low = 0usize;
    let mut high = 4 * preflight_bytes(false) + 1;
    assert!(
        accepts_captures(high, captures, counters),
        "the upper bound must accept the sequence"
    );
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if accepts_captures(mid, captures, counters) {
            high = mid;
        } else {
            low = mid;
        }
    }
    high
}

fn bounded_preflight_bytes() -> usize {
    preflight_bytes(false)
}

// ---------------------------------------------------------------------------
// The declared bound
// ---------------------------------------------------------------------------

#[test]
fn the_declared_payload_bound_covers_a_fresh_and_a_two_pending_state() {
    let factory = require_ok!(MegadriveFactory::new(demo_rom()), "valid demo ROM");
    let create = require_ok!(factory.create(), "machine");
    assert_eq!(
        create.snapshot_payload_bound(),
        Some(SNAPSHOT_PAYLOAD_BOUND),
        "the backend declares the format-derived bound"
    );
    let fresh = require_ok!(create.snapshot(), "snapshot").payload().len();
    assert!(
        fresh < SNAPSHOT_PAYLOAD_BOUND,
        "a fresh state is strictly below the bound"
    );

    // The bound is exact: a state with both scheduler requests pending reaches it,
    // and no sampled state exceeds it. The sweep measures the payload instead of
    // comparing against a copied number.
    let mut machine = require_ok!(factory.create(), "machine");
    let mut max = 0usize;
    let mut reached_bound = false;
    for _ in 0..40_000 {
        let length = require_ok!(machine.snapshot(), "snapshot").payload().len();
        assert!(
            length <= SNAPSHOT_PAYLOAD_BOUND,
            "a payload above the declared bound: {length}"
        );
        max = max.max(length);
        if length == SNAPSHOT_PAYLOAD_BOUND {
            reached_bound = true;
            break;
        }
        require_ok!(machine.step(InputFrame::NEUTRAL), "step");
    }
    assert!(
        reached_bound,
        "the sweep must reach the declared bound exactly (maximum {max})"
    );
}

// ---------------------------------------------------------------------------
// Session-layer invariants around a refusal (same object, before/after)
// ---------------------------------------------------------------------------

/// The checkpoint invariants a refusal must leave untouched.
///
/// The count and the retained bytes are read from the same session object before
/// and after the refusal; the identifier the refusal would have taken is proven
/// separately by the accepted capture that follows (`snapshot_within` refuses
/// **before** `allocate_snapshot_id`).
#[derive(Debug, Eq, PartialEq)]
struct Accounting {
    count: usize,
    retained_bytes: usize,
}

fn accounting(manager: &SessionManager, session: SessionId) -> Accounting {
    let handle = require_ok!(manager.session_handle(session), "session handle");
    let guarded = require_ok!(handle.lock(), "session owner");
    Accounting {
        count: guarded.snapshot_count(),
        retained_bytes: guarded.snapshot_bytes_retained(),
    }
}

/// Every retained record serialized, in identifier order.
///
/// This is the byte-level half of the "unchanged" invariant: the accounting maps
/// could agree while a refusal had rewritten or renumbered a record, so the tests
/// that read it also compare these bytes before and after the refusal.
fn serialized_records(manager: &SessionManager, session: SessionId) -> Vec<(SnapshotId, Vec<u8>)> {
    let handle = require_ok!(manager.session_handle(session), "session handle");
    let guarded = require_ok!(handle.lock(), "session owner");
    let mut records = Vec::new();
    for id in 1..=guarded.snapshot_count() {
        let id = SnapshotId::from_raw(id as u64);
        let bytes = require_ok!(
            guarded.snapshot_bytes(id),
            "every identifier up to the count is a retained record"
        );
        records.push((id, bytes));
    }
    records
}

/// Captures one checkpoint on an already locked session owner under a quota.
fn capture_within(
    manager: &SessionManager,
    session: SessionId,
    max_snapshots: usize,
    max_bytes: usize,
) -> Result<u64, chronofork_sessions::SessionError> {
    let handle = require_ok!(manager.session_handle(session), "session handle");
    let mut guarded = require_ok!(handle.lock(), "session owner");
    guarded
        .snapshot_within(max_snapshots, max_bytes)
        .map(|id| id.raw())
}

fn megadrive_manager(counters: &Arc<Counters>) -> SessionManager {
    let factory = require_ok!(CountingFactory::new(counters), "counting factory");
    SessionManager::new(Arc::new(factory))
}

/// A refused capture leaves the same session's count, retained bytes, retained
/// records and identifier counter exactly as they were.
///
/// The accounting (count, retained bytes) and the serialized bytes of every
/// retained record are read from the same session object before and after the
/// refusal, and the accepted capture that follows reuses the identifier the
/// refusal would have taken (`snapshot_within` refuses **before**
/// `allocate_snapshot_id`).
///
/// The quota is derived from a measured record, so the second capture is refused
/// because it provably does not fit.
#[test]
fn a_refused_byte_quota_capture_leaves_the_same_session_untouched() {
    let counters = Arc::new(Counters::default());
    let record = measured_record_bytes();
    let mut manager = megadrive_manager(&counters);
    let session = require_ok!(manager.create_session(), "session");

    let first = require_ok!(manager.snapshot(session), "first capture");
    assert_eq!(first.raw(), 1, "the first capture is identifier 1");
    assert_eq!(
        require_ok!(manager.snapshot_bytes(session, first), "record bytes").len(),
        record,
        "the stored record is the measured one"
    );
    let before = accounting(&manager, session);
    assert_eq!(before.count, 1);
    assert_eq!(before.retained_bytes, record);
    let records_before = serialized_records(&manager, session);

    // A quota one byte below a second record: the capture is refused before the
    // backend is captured and leaves the session untouched.
    let captures_before = counters.captured.load(Ordering::SeqCst);
    let refused = capture_within(&manager, session, 64, 2 * record - 1);
    assert!(refused.is_err(), "the second record cannot fit the quota");
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captures_before,
        "the refusal happened before `Backend::snapshot()` was called"
    );
    assert_eq!(
        accounting(&manager, session),
        before,
        "the refused capture changed neither the count nor the retained bytes"
    );
    assert_eq!(
        serialized_records(&manager, session),
        records_before,
        "the refused capture changed no retained record"
    );

    // The next accepted capture reuses the identifier the refusal would have taken.
    let second = require_ok!(
        capture_within(&manager, session, 64, 2 * record),
        "the freed budget accepts the second capture"
    );
    assert_eq!(second, 2, "the refused capture consumed no identifier");
    let after = accounting(&manager, session);
    assert_eq!(after.count, 2);
    assert_eq!(after.retained_bytes, 2 * record);
}

/// The W1.8 state transition: an accepted capture, an advance, then a refusal
/// decided from the declared bound rather than from an observation.
///
/// The first capture leaves the session with `state_observed = true` and an exact
/// observed payload size. Advancing the machine (`run` on the service, `step`
/// here) invalidates that observation, so the preflight under test is the
/// declared-bound one: the values the refusal reports exceed the weight of a
/// record that starts from the stored observation, which is what
/// [`SNAPSHOT_PAYLOAD_BOUND`] predicts. The same session object is then asked for
/// a capture that fits, and the accepted identifier must be the one the refusals
/// would have taken — a conditional mutation that advances the counter only on
/// the declared-bound branch skips it.
#[test]
fn a_refused_changed_state_byte_quota_capture_leaves_the_same_session_untouched() {
    let counters = Arc::new(Counters::default());
    let record = measured_record_bytes();
    let mut manager = megadrive_manager(&counters);
    let session = require_ok!(manager.create_session(), "session");

    // A capture in an observed state, so the session holds exactly one record and
    // the next refusal can only be about the bytes of a second one.
    let first = require_ok!(manager.snapshot(session), "first capture");
    assert_eq!(first.raw(), 1, "the first capture is identifier 1");
    let before = accounting(&manager, session);
    assert_eq!(before.count, 1);
    assert_eq!(before.retained_bytes, record);
    let records_before = serialized_records(&manager, session);

    // Advance the machine: from here the observed payload size describes a state
    // that no longer exists, so the byte preflight must use the declared bound.
    require_ok!(manager.step(session, InputFrame::NEUTRAL), "step");

    // Refuse the capture at the exact quota boundary, then confirm the state really
    // is the unobserved one: with the stale observation this quota would fit, so
    // the refusal is the declared-bound refusal W1.8 exists to make safe.
    let captures_before = counters.captured.load(Ordering::SeqCst);
    let refused = require_err!(
        capture_within(&manager, session, 64, 2 * record),
        "a second record cannot fit a two-record quota after the state changed"
    );
    assert!(
        matches!(
            refused,
            chronofork_sessions::SessionError::LimitExceeded {
                quota: chronofork_sessions::SessionQuota::Bytes,
                ..
            }
        ),
        "the changed-state refusal is a byte-quota rejection: {refused:?}"
    );
    let current = require_some!(
        match refused {
            chronofork_sessions::SessionError::LimitExceeded { current, .. } => Some(current),
            _ => None,
        },
        "the refusal above is a byte-quota rejection"
    );
    assert!(
        current > 2 * record,
        "the declared bound is what made this capture unfittable ({current} vs {})",
        2 * record
    );
    let declared_quota = current - 1;
    let second_refusal = require_err!(
        capture_within(&manager, session, 64, declared_quota),
        "the byte preflight is independent of the previous refusal"
    );
    assert!(
        matches!(
            second_refusal,
            chronofork_sessions::SessionError::LimitExceeded {
                quota: chronofork_sessions::SessionQuota::Bytes,
                ..
            }
        ),
        "a repeated changed-state capture is refused by the byte quota: {second_refusal:?}"
    );

    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captures_before,
        "the refusal happened before `Backend::snapshot()` was called"
    );
    assert_eq!(
        accounting(&manager, session),
        before,
        "the refused changed-state capture changed neither count nor retained bytes"
    );
    assert_eq!(
        serialized_records(&manager, session),
        records_before,
        "the refused changed-state capture changed no retained record"
    );

    // The same session object accepts the capture as soon as the quota holds the
    // declared-bound record, and it reuses the identifier the refusals would have
    // taken: nothing advanced the counter behind a refusal.
    let second = require_ok!(
        capture_within(&manager, session, 64, current),
        "the declared-bound record fits exactly at the refusal's reported value"
    );
    assert_eq!(second, 2, "the refused captures consumed no identifier");
    let after = accounting(&manager, session);
    assert_eq!(after.count, 2);
    let accepted_record = after
        .retained_bytes
        .checked_sub(before.retained_bytes)
        .unwrap_or(usize::MAX);
    let second_id = SnapshotId::from_raw(second);
    assert_eq!(
        accepted_record,
        require_ok!(
            manager.snapshot_bytes(session, second_id),
            "the accepted capture is a retained record"
        )
        .len(),
        "the accepted changed-state record is what raised the retained bytes"
    );
}

/// A count-quota refusal is refused before the backend is captured too, and the
/// identifier it would have taken is still the next one.
#[test]
fn a_refused_count_quota_capture_leaves_the_same_session_untouched() {
    let counters = Arc::new(Counters::default());
    let record = measured_record_bytes();
    let mut manager = megadrive_manager(&counters);
    let session = require_ok!(manager.create_session(), "session");

    let first = require_ok!(manager.snapshot(session), "first capture");
    assert_eq!(first.raw(), 1);
    let before = accounting(&manager, session);
    assert_eq!(before.count, 1);
    let records_before = serialized_records(&manager, session);

    let captures_before = counters.captured.load(Ordering::SeqCst);
    let refused = capture_within(&manager, session, 1, usize::MAX);
    assert!(refused.is_err(), "the count quota holds one checkpoint");
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captures_before,
        "the count refusal happens before `Backend::snapshot()`"
    );
    assert_eq!(
        accounting(&manager, session),
        before,
        "the refused capture changed neither the count nor the retained bytes"
    );
    assert_eq!(
        serialized_records(&manager, session),
        records_before,
        "the refused count capture changed no retained record"
    );

    // With room for a second checkpoint the identifier is still the next one.
    let second = require_ok!(
        capture_within(&manager, session, 2, usize::MAX),
        "the raised count quota accepts the second capture"
    );
    assert_eq!(second, 2, "the refused capture consumed no identifier");
    let after = accounting(&manager, session);
    assert_eq!(after.count, 2);
    assert_eq!(after.retained_bytes, 2 * record);
}

// ---------------------------------------------------------------------------
// The service regression under a finite quota
// ---------------------------------------------------------------------------

#[test]
fn a_finite_quota_allows_snapshot_fork_and_restore_after_a_run() {
    let counters = Arc::new(Counters::default());
    let record = measured_record_bytes();
    let fresh = preflight_bytes(true);
    let bounded = bounded_preflight_bytes();
    assert!(
        bounded > fresh,
        "a capture decided from the declared bound must predict more than the exact one"
    );
    // Three preflight predictions of headroom: the parent keeps two checkpoints
    // and the child one, all under one finite quota.
    let service = service_with_quota(3 * bounded, &counters);

    let created = ok(&service, "session.create", "{}");
    assert_eq!(number(&created, "session"), 1);
    let first = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&first, "snapshot"), 1);
    assert_eq!(
        record_len(&first),
        record,
        "the stored record is the measured one"
    );

    // `run` clears the exact observed size, so the second capture's preflight must
    // use the declared bound; this is the behavior the task exists to unblock.
    ok(&service, "session.run", "{\"session\":1,\"steps\":1}");
    let second = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&second, "snapshot"), 2);

    // The child's creation checkpoint goes through the same finite quota.
    let fork = ok(&service, "session.fork", "{\"session\":1}");
    assert_eq!(number(&fork, "session"), 2);
    assert_eq!(
        require_ok!(service.session_count(), "session count"),
        2,
        "the accepted fork admitted one child"
    );

    // Restoring the post-run checkpoint is accepted under the same quota.
    let restored = ok(
        &service,
        "session.restore",
        "{\"session\":1,\"snapshot\":2}",
    );
    assert_eq!(number(&restored, "session"), 1);
    assert_eq!(
        second
            .get("state_hash")
            .and_then(JsonValue::as_str)
            .map(str::len),
        Some(64),
        "the checkpoint carries a state hash"
    );
}

#[test]
fn the_preflight_prediction_does_not_over_refuse_a_capture_that_fits() {
    let counters = Arc::new(Counters::default());
    let record = measured_record_bytes();
    let fresh = preflight_bytes(true);
    let bounded = bounded_preflight_bytes();
    // Exactly the two preflight predictions: one accepted before the run, one
    // after it. The post-run preflight predicts
    // `record_bytes_for(SNAPSHOT_PAYLOAD_BOUND)`, which this test derives from the
    // service itself, so the capture fits exactly. A bound larger than the real
    // two-pending payload would over-estimate and refuse this capture.
    let service = service_with_quota(fresh + bounded, &counters);

    ok(&service, "session.create", "{}");
    let first = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(record_len(&first), record);
    ok(&service, "session.run", "{\"session\":1,\"steps\":1}");
    let second = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&second, "snapshot"), 2);
    assert_eq!(
        record_len(&second),
        record,
        "a capture decided from the declared bound stores the same exact record"
    );
}

/// A service refusal is preflighted: it publishes no new checkpoint and stops
/// before the backend is captured.
///
/// The measurement of "unchanged" is the service's own public surface: a refused
/// capture publishes no new checkpoint, the refused fork admits no child, and the
/// counting wrapper sees no `Backend::snapshot` call. The session-layer guard
/// above carries the identifier-reuse proof, which this test does not perform.
#[test]
fn an_over_quota_capture_is_refused_before_the_backend_is_captured() {
    let counters = Arc::new(Counters::default());
    // The smallest quota that accepts one capture, measured through the service.
    let one = min_quota_for_captures(1, &counters);

    let service = service_with_quota(one, &counters);
    let created = ok(&service, "session.create", "{}");
    assert_eq!(number(&created, "session"), 1);
    let first = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&first, "snapshot"), 1);
    assert_eq!(record_len(&first), measured_record_bytes());

    // The pre-run state is still exactly observed, so the next capture is refused
    // by the byte preflight without a backend capture.
    let captures_before = counters.captured.load(Ordering::SeqCst);
    let rejected = require_err!(
        dispatch(&service, "session.snapshot", "{\"session\":1}"),
        "the quota holds one record and no second"
    );
    assert_eq!(rejected.code(), ErrorCode::LimitExceeded);
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captures_before,
        "the rejection happened before `Backend::snapshot()` was called"
    );

    // A fork's parent checkpoint goes through the same quota and is refused the
    // same way, with the same before-capture effect and no admitted child.
    let fork_rejected = require_err!(
        dispatch(&service, "session.fork", "{\"session\":1}"),
        "the parent has no record budget for the child"
    );
    assert_eq!(fork_rejected.code(), ErrorCode::LimitExceeded);
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captures_before,
        "the refused fork also stopped before the backend was captured"
    );
    assert_eq!(require_ok!(service.session_count(), "session count"), 1);
}

/// A service count-quota refusal is a preflight too: no backend capture, no record.
#[test]
fn a_service_count_quota_refusal_is_preflighted() {
    let counters = Arc::new(Counters::default());
    let factory = require_ok!(CountingFactory::new(&counters), "counting factory");
    let service = require_ok!(
        AppService::new(ServiceConfig {
            factory: Arc::new(factory),
            storage_root: None,
            artifacts_root: None,
            limits: ServiceLimits {
                max_snapshots_per_session: 1,
                ..ServiceLimits::default()
            },
        }),
        "service"
    );
    ok(&service, "session.create", "{}");
    let first = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&first, "snapshot"), 1);

    let captures_before = counters.captured.load(Ordering::SeqCst);
    let refused = require_err!(
        dispatch(&service, "session.snapshot", "{\"session\":1}"),
        "the count quota holds one checkpoint"
    );
    assert_eq!(refused.code(), ErrorCode::LimitExceeded);
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captures_before,
        "the count refusal happened before `Backend::snapshot()` was called"
    );
}
