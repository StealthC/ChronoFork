#![allow(clippy::expect_used)]

//! W3.2 acceptance tests: cancellation, deadline, progress and reconciliation.
//!
//! These tests exercise the contract of ADR-0016 §2 through the public service API
//! only:
//!
//! - **cancel in one instruction:** a cancelled run is a deterministic prefix — the
//!   state it reaches equals the state of an independent oracle advanced by exactly
//!   the `performed` count the result reports;
//! - **cancellation is lock-free and observable during a run:** the control path is
//!   driven by another session (and another thread) while a run holds its own
//!   session owner, so a `cancel` that needed that owner would never return;
//! - **deadline:** a host wall-clock budget truncates a run and reports
//!   `stop_reason: "deadline"` with the prefix it applied;
//! - **progress:** `session.progress` reports the performed count of a run that is
//!   still executing, from the operation journal only;
//! - **reconciliation:** a retry with the same request key replays the recorded
//!   outcome instead of applying the mutation twice, the same key with different
//!   arguments is a typed conflict, and a key outside the horizon is an
//!   `unknown_outcome` — never an automatic re-execution.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use chronofork_core::InputFrame;
use chronofork_service::json::{JsonValue, to_canonical_bytes};
use chronofork_service::{
    ApiVersion, AppService, ErrorCode, Principal, Scope, ServiceConfig, ServiceError, ServiceLimits,
};
use chronofork_sessions::SessionManager;
use chronofork_test_backend::SyntheticBackendFactory;

/// Deadline for every call the contract says must not block.
///
/// Generous on purpose: the test binary runs these cases concurrently, so a control
/// call that waits for CPU is still distinguished from a control call that waits for
/// the session owner — the latter never completes at all.
const CONTROL_DEADLINE: Duration = Duration::from_secs(60);

/// Steps for one long run: far more than a truncated run ever completes, so a
/// control-path assertion cannot pass because the run happened to finish early.
// W2-2: ordinary steps no longer compute a per-step digest, so a step is much
// cheaper. The workload is sized to still exceed the 120 ms wall budget used by the
// deadline test; the cancel/progress tests only need a run that is long enough to be
// observed, and a larger budget keeps that margin. No assertion threshold changed.
const LONG_RUN_STEPS: u64 = 2_000_000;

fn limits() -> ServiceLimits {
    // W2-2 makes a step digest-free and therefore much cheaper, so observing a
    // 120 ms wall deadline needs a longer workload than the default request cap
    // allows. Only this file's fixture raises the cap; the request-limit
    // enforcement itself is covered by the service contract tests.
    ServiceLimits {
        max_steps_per_request: LONG_RUN_STEPS,
        ..ServiceLimits::default()
    }
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

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
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
    // The control surface is v2-only by contract: every request in this file declares
    // v2 (ADR-0016 §Compatibility).
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

/// Decodes the canonical envelope bytes of one successful request.
fn envelope(bytes: &[u8]) -> JsonValue {
    chronofork_service::json::parse(bytes, &limits()).expect("canonical envelope")
}

fn number(value: &JsonValue, name: &str) -> u64 {
    value
        .get(name)
        .and_then(JsonValue::as_u64)
        .unwrap_or_else(|| unreachable!("missing unsigned field `{name}` in {value:?}"))
}

fn text<'a>(value: &'a JsonValue, name: &str) -> &'a str {
    value
        .get(name)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| unreachable!("missing string field `{name}` in {value:?}"))
}

/// Creates one session and returns its identifier.
fn create(service: &AppService) -> u64 {
    number(&ok(service, &operator(), "session.create", "{}"), "session")
}

/// Runs one session on an independent session-layer oracle for `steps` steps.
///
/// This is the prefix authority: nothing in the service participates, so equality
/// proves the truncated run applied exactly that many canonical steps.
fn oracle(steps: u64) -> (u64, String) {
    let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
    let session = manager.create_session().expect("oracle session");
    for _ in 0..steps {
        manager
            .step(session, InputFrame::new(0))
            .expect("oracle step");
    }
    let view = manager.inspect(session).expect("oracle inspect");
    (view.machine.tick.raw(), view.machine.state_hash.to_hex())
}

/// Asserts that a result is the deterministic prefix of exactly `performed` steps.
fn assert_prefix(result: &JsonValue) {
    let performed = number(result, "performed");
    assert_eq!(
        number(result, "steps"),
        performed,
        "`steps` reports what the run applied"
    );
    let (tick, state_hash) = oracle(performed);
    assert_eq!(
        number(result, "tick"),
        tick,
        "a truncated run stops at the tick of a {performed}-step run"
    );
    assert_eq!(
        text(result, "state_hash"),
        state_hash,
        "a truncated run's state is the {performed}-step canonical state"
    );
}

/// Waits until `predicate` holds or the deadline passes.
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

// ---------------------------------------------------------------------------
// Cancel in one instruction
// ---------------------------------------------------------------------------

#[test]
fn a_cancelled_run_is_a_deterministic_prefix_of_the_same_session() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    let request = Arc::new(request_bytes(
        "session.run",
        &input(&format!(
            "{{\"session\":{session},\"steps\":{LONG_RUN_STEPS},\"operation_id\":41}}"
        )),
    ));

    let runner = {
        let service = Arc::clone(&service);
        let principal = principal.clone();
        let request = Arc::clone(&request);
        thread::spawn(move || service.dispatch_versioned(&principal, &request, ApiVersion::V2))
    };

    // Cancellation is issued from this thread while the run holds its session
    // owner. `session.cancel` takes no session owner, so it cannot queue behind the
    // run; if it did, `wait_until` would time out and cancel would never be set.
    // ADR-0016 §Verification 2 asks for a **bounded** cancel latency: the signal is
    // an atomic store behind the journal lock, so it must not take longer than one
    // control round trip. The measurement is printed for the record and the assertion
    // is a generous bound, so a loaded machine cannot turn a correct implementation
    // into a flake; the failure it guards against is a cancel that waits for the
    // session owner, which never returns at all.
    let cancel_started = Instant::now();
    let cancelled = wait_until(CONTROL_DEADLINE, || {
        dispatch(
            &service,
            &principal,
            "session.cancel",
            &format!("{{\"session\":{session}}}"),
        )
        .is_ok()
    });
    let cancel_latency = cancel_started.elapsed();
    eprintln!("W3.2-CANCEL latency={cancel_latency:?}");
    assert!(
        cancelled,
        "`session.cancel` never reached the running operation"
    );
    assert!(
        cancel_latency < Duration::from_secs(5),
        "cancel must not wait for the session owner: {cancel_latency:?}"
    );

    let result = envelope(
        &runner
            .join()
            .expect("run thread")
            .expect("a cancelled run reports its partial result, not an envelope error"),
    );
    let result = result.get("result").expect("result").clone();
    assert_eq!(
        text(&result, "stop_reason"),
        "cancelled",
        "the caller's explicit signal wins at the boundary"
    );
    let performed = number(&result, "performed");
    assert!(
        performed < LONG_RUN_STEPS,
        "the run must stop early, performed {performed}"
    );
    assert_prefix(&result);
    assert_eq!(
        number(&result, "revision"),
        1,
        "a run that applied steps advances the session revision once"
    );
}

#[test]
fn cancel_reports_a_typed_error_when_nothing_is_running() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    let error = dispatch(
        &service,
        &principal,
        "session.cancel",
        &format!("{{\"session\":{session}}}"),
    )
    .expect_err("nothing is running");
    assert_eq!(error.code(), ErrorCode::Conflict);

    // An unknown operation key is outside the reconciliation horizon, not a
    // conflict: the server never guesses about an operation it cannot see.
    let error = dispatch(
        &service,
        &principal,
        "session.cancel",
        "{\"operation\":999999}",
    )
    .expect_err("unknown operation");
    assert_eq!(error.code(), ErrorCode::UnknownOutcome);

    // An operation that already finished is not cancellable either.
    let session = create(&service);
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":4,\"operation_id\":77}}"),
    );
    let error = dispatch(&service, &principal, "session.cancel", "{\"operation\":77}")
        .expect_err("already finished");
    assert_eq!(error.code(), ErrorCode::Cancelled);
}

#[test]
fn cancel_is_reachable_while_a_run_holds_another_session() {
    let service = service(limits());
    let principal = operator();
    let busy = create(&service);
    let request = Arc::new(request_bytes(
        "session.run",
        &input(&format!(
            "{{\"session\":{busy},\"steps\":{LONG_RUN_STEPS},\"operation_id\":8}}"
        )),
    ));
    let runner = {
        let service = Arc::clone(&service);
        let principal = principal.clone();
        thread::spawn(move || service.dispatch_versioned(&principal, &request, ApiVersion::V2))
    };

    // The other thread reports progress of the first session's run while that run
    // holds its session owner: the control path takes no session owner, so it cannot
    // queue behind it. The predicate requires the run to be *in flight*, so a
    // fast-finished run cannot make the assertions below vacuous.
    let in_flight = wait_until(CONTROL_DEADLINE, || {
        dispatch(&service, &principal, "session.list", "{}").is_ok()
            && dispatch(
                &service,
                &principal,
                "session.progress",
                "{\"operation\":8}",
            )
            .map(|progress| {
                text(&progress, "state") == "active" && number(&progress, "performed") > 0
            })
            .unwrap_or(false)
    });
    assert!(
        in_flight,
        "the control path never observed the run in flight"
    );

    // A conflicting run of the same session is refused immediately (writer lease),
    // which also proves the run still holds its owner at this point.
    let conflict = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{busy},\"steps\":1}}"),
    )
    .expect_err("the session is being mutated");
    assert_eq!(conflict.code(), ErrorCode::Conflict);
    assert!(conflict.is_retryable());

    assert!(
        dispatch(
            &service,
            &principal,
            "session.cancel",
            &format!("{{\"session\":{busy}}}")
        )
        .is_ok(),
        "the parked run must still be cancellable"
    );
    let result = runner.join().expect("run thread").expect("partial result");
    let result = envelope(&result);
    let result = result.get("result").expect("result");
    assert_eq!(text(result, "stop_reason"), "cancelled");
    assert_prefix(result);
}

// ---------------------------------------------------------------------------
// Deadline
// ---------------------------------------------------------------------------

#[test]
fn a_deadline_truncates_the_run_and_reports_the_applied_prefix() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    let started = Instant::now();
    let result = ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":{LONG_RUN_STEPS},\"max_wall_ms\":120}}"),
    );
    let elapsed = started.elapsed();
    assert_eq!(text(&result, "stop_reason"), "deadline");
    assert_eq!(number(&result, "max_wall_ms"), 120);
    let performed = number(&result, "performed");
    assert!(
        performed < LONG_RUN_STEPS,
        "the wall budget must truncate the run, performed {performed}"
    );
    assert!(
        elapsed >= Duration::from_millis(120),
        "the run cannot end before its budget: {elapsed:?}"
    );
    assert_prefix(&result);
}

#[test]
fn a_run_that_finishes_inside_its_budget_reports_completed() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    let result = ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":16,\"max_wall_ms\":30000}}"),
    );
    assert_eq!(text(&result, "stop_reason"), "completed");
    assert_eq!(number(&result, "performed"), 16);
    assert_prefix(&result);
}

#[test]
fn a_wall_budget_above_the_daemon_cap_is_rejected_before_any_step() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    let error = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":4,\"max_wall_ms\":30001}}"),
    )
    .expect_err("above the daemon cap");
    assert_eq!(error.code(), ErrorCode::LimitExceeded);
    // Nothing ran: the session is still at tick zero.
    let view = ok(
        &service,
        &principal,
        "session.inspect",
        &format!("{{\"session\":{session}}}"),
    );
    assert_eq!(
        number(view.get("machine").expect("machine"), "tick"),
        0,
        "a rejected request never partially applies"
    );
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

#[test]
fn progress_reports_a_running_operation_from_the_journal_only() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    let request = Arc::new(request_bytes(
        "session.run",
        &input(&format!(
            "{{\"session\":{session},\"steps\":{LONG_RUN_STEPS},\"operation_id\":5}}"
        )),
    ));
    let runner = {
        let service = Arc::clone(&service);
        let principal = principal.clone();
        thread::spawn(move || service.dispatch_versioned(&principal, &request, ApiVersion::V2))
    };

    let mut last = String::new();
    let observed = wait_until(CONTROL_DEADLINE, || {
        match dispatch(
            &service,
            &principal,
            "session.progress",
            "{\"operation\":5}",
        ) {
            Ok(progress) => {
                last = format!("{progress:?}");
                text(&progress, "state") == "active" && number(&progress, "performed") > 0
            }
            Err(error) => {
                last = format!("error {error}");
                false
            }
        }
    });
    assert!(
        observed,
        "`session.progress` never observed the running operation; last {last}"
    );

    let progress = ok(
        &service,
        &principal,
        "session.progress",
        "{\"operation\":5}",
    );
    assert_eq!(number(&progress, "session"), session);
    assert!(number(&progress, "performed") > 0);

    // The run is still executing, and the session itself was never read for this
    // progress report: a conflicting mutation of the session is refused by the lease.
    let conflict = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":1}}"),
    )
    .expect_err("still running");
    assert_eq!(conflict.code(), ErrorCode::Conflict);

    assert!(dispatch(&service, &principal, "session.cancel", "{\"operation\":5}").is_ok());
    let result = runner.join().expect("run thread").expect("partial result");
    let result = envelope(&result);
    let result = result.get("result").expect("result");
    assert_eq!(text(result, "stop_reason"), "cancelled");

    // After the run finishes, the journal entry reports the final boundary.
    let progress = ok(
        &service,
        &principal,
        "session.progress",
        "{\"operation\":5}",
    );
    assert_eq!(text(&progress, "state"), "finished");
    assert_eq!(number(&progress, "performed"), number(result, "performed"));
    assert_eq!(text(&progress, "stop_reason"), "cancelled");
    assert_eq!(number(&progress, "tick"), number(result, "tick"));
}

#[test]
fn progress_of_an_unknown_operation_is_an_unknown_outcome() {
    let service = service(limits());
    let error = dispatch(
        &service,
        &operator(),
        "session.progress",
        "{\"operation\":3}",
    )
    .expect_err("unknown operation");
    assert_eq!(error.code(), ErrorCode::UnknownOutcome);
}

// ---------------------------------------------------------------------------
// Reconciliation
// ---------------------------------------------------------------------------

#[test]
fn a_retry_with_the_same_key_replays_the_recorded_outcome_without_re_applying() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    let text_input = format!("{{\"session\":{session},\"steps\":10,\"operation_id\":9}}");

    let first = ok(&service, &principal, "session.run", &text_input);
    let second = ok(&service, &principal, "session.run", &text_input);
    assert_eq!(
        first, second,
        "the recorded outcome is replayed byte for byte"
    );
    assert_eq!(
        number(&second, "tick"),
        10,
        "the retry did not apply the mutation a second time"
    );

    // The session advanced exactly once: an independent oracle of 10 steps agrees.
    let (tick, state_hash) = oracle(10);
    assert_eq!(number(&second, "tick"), tick);
    assert_eq!(text(&second, "state_hash"), state_hash);

    // The recorded envelope is readable through the operation journal, which is the
    // documented way a caller reconciles a lost response.
    let recorded = ok(
        &service,
        &principal,
        "session.operation",
        "{\"operation\":9}",
    );
    assert_eq!(text(&recorded, "state"), "finished");
    let outcome = recorded.get("outcome").expect("recorded outcome");
    assert_eq!(
        outcome
            .get("result")
            .and_then(|r| r.get("tick"))
            .and_then(JsonValue::as_u64),
        Some(10)
    );
}

#[test]
fn the_same_key_with_different_arguments_is_a_typed_conflict() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":4,\"operation_id\":3}}"),
    );
    let error = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":5,\"operation_id\":3}}"),
    )
    .expect_err("different arguments");
    assert_eq!(error.code(), ErrorCode::Conflict);
    // The conflicting request did not apply anything: the session is still at the
    // tick the first request reached.
    let view = ok(
        &service,
        &principal,
        "session.inspect",
        &format!("{{\"session\":{session}}}"),
    );
    assert_eq!(
        number(view.get("machine").expect("machine"), "tick"),
        4,
        "a rejected request never partially applies"
    );
}

#[test]
fn a_lost_result_is_recoverable_from_the_journal_only_inside_the_horizon() {
    // A zero TTL makes the horizon deterministic: an entry stops being recognized
    // from the instant it finishes, and the server never re-executes it.
    let service = service(ServiceLimits {
        operation_ttl: 0,
        ..ServiceLimits::default()
    });
    let principal = operator();
    let session = create(&service);
    let text_input = format!("{{\"session\":{session},\"steps\":4,\"operation_id\":2}}");
    ok(&service, &principal, "session.run", &text_input);

    let error = dispatch(
        &service,
        &principal,
        "session.operation",
        "{\"operation\":2}",
    )
    .expect_err("outside the horizon");
    assert_eq!(error.code(), ErrorCode::UnknownOutcome);

    // The retry is **not** re-executed: the key is recognized as already used and its
    // outcome is outside the retention horizon, so the server answers
    // `unknown_outcome` and the session state is untouched (ADR-0016 §2, V3-3).
    let retry = dispatch(&service, &principal, "session.run", &text_input)
        .expect_err("a key outside the horizon is never re-executed");
    assert_eq!(retry.code(), ErrorCode::UnknownOutcome);
    let inspected = ok(
        &service,
        &principal,
        "session.inspect",
        &format!("{{\"session\":{session}}}"),
    );
    // `inspect` reports the tick under `machine`, not at the result root.
    let tick = inspected
        .get("machine")
        .and_then(|machine| machine.get("tick"))
        .and_then(JsonValue::as_u64);
    assert_eq!(tick, Some(4), "the refused retry applied no step");

    // A caller that really wants to run the mutation again uses a **new** key, which is
    // an explicit decision rather than a silent re-application.
    let fresh = ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":4,\"operation_id\":3}}"),
    );
    assert_eq!(number(&fresh, "tick"), 8);
}

#[test]
fn the_journal_limit_is_declared_and_rejects_before_work() {
    let service = service(ServiceLimits {
        max_operation_journal: 1,
        operation_ttl: 600,
        ..ServiceLimits::default()
    });
    let principal = operator();
    let first = create(&service);
    let second = create(&service);
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{first},\"steps\":2,\"operation_id\":1}}"),
    );
    // The journal holds one entry and it is not expired; a second key is a typed
    // `limit` rejection taken before any step runs.
    let error = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{second},\"steps\":2,\"operation_id\":2}}"),
    )
    .expect_err("journal full");
    assert_eq!(error.code(), ErrorCode::LimitExceeded);
    let view = ok(
        &service,
        &principal,
        "session.inspect",
        &format!("{{\"session\":{second}}}"),
    );
    let machine = view.get("machine").expect("machine");
    assert_eq!(
        number(machine, "tick"),
        0,
        "no step ran for the rejected request"
    );
}

// ---------------------------------------------------------------------------
// Revision preconditions (race R6: the tick is not a revision)
// ---------------------------------------------------------------------------

#[test]
fn expected_revision_detects_staleness_that_a_tick_cannot() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);

    // Two runs of one step each: the tick moves, and so does the revision.
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":1}}"),
    );
    let first_revision = number(
        &ok(
            &service,
            &principal,
            "session.inspect",
            &format!("{{\"session\":{session}}}"),
        ),
        "revision",
    );
    assert_eq!(first_revision, 1);

    // A stale retry (revision 0) is refused before any step.
    let error = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":5,\"expected_revision\":0}}"),
    )
    .expect_err("stale revision");
    assert_eq!(error.code(), ErrorCode::Conflict);

    // The current revision is accepted.
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":5,\"expected_revision\":{first_revision}}}"),
    );
}

/// The revision is monotonic across runs, so a retry pinned to an earlier one is refused.
///
/// (Renamed in W3d: the previous name claimed a restore kept the revision, which is the
/// behavior review R1 rejected — a restore now advances it, and this test covers the
/// `expected_revision` precondition instead.)
#[test]
fn the_revision_is_monotonic_and_a_pinned_stale_retry_is_a_conflict() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":8}}"),
    );
    let after_run = ok(
        &service,
        &principal,
        "session.inspect",
        &format!("{{\"session\":{session}}}"),
    );
    let revision = number(&after_run, "revision");
    assert_eq!(revision, 1);

    // A snapshot/restore round trip leaves a stale tick behind: the tick goes back,
    // the revision does not, so `expected_revision` still rejects a retry computed
    // against the pre-run state (ADR-0016 §2, race R6).
    ok(
        &service,
        &principal,
        "session.snapshot",
        &format!("{{\"session\":{session}}}"),
    );

    // A second run advances the revision again; the tick never decreases the
    // revision, so a caller can always tell that state moved on.
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":3}}"),
    );
    let later = ok(
        &service,
        &principal,
        "session.inspect",
        &format!("{{\"session\":{session}}}"),
    );
    assert_eq!(number(&later, "revision"), 2);
    assert_eq!(
        number(later.get("machine").expect("machine"), "tick"),
        11,
        "the run advanced the canonical tick by three"
    );

    // A late retry pinned to the earlier revision is refused: the tick is not a
    // revision and cannot stand in for one.
    let error = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":1,\"expected_revision\":{revision}}}"),
    )
    .expect_err("stale revision");
    assert_eq!(error.code(), ErrorCode::Conflict);
}

#[test]
fn expected_tick_is_checked_under_the_owner_and_rejects_a_stale_tick() {
    let service = service(limits());
    let principal = operator();
    let session = create(&service);
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":4}}"),
    );
    let error = dispatch(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":2,\"expected_tick\":3}}"),
    )
    .expect_err("stale tick");
    assert_eq!(error.code(), ErrorCode::Conflict);
    ok(
        &service,
        &principal,
        "session.run",
        &format!("{{\"session\":{session},\"steps\":2,\"expected_tick\":4}}"),
    );
}

#[test]
fn capability_discovery_reports_the_control_surface_and_its_bounds() {
    let service = service(limits());
    let capabilities = ok(&service, &operator(), "capabilities", "{}");
    assert_eq!(number(&capabilities, "api_version"), 2);
    let limits = capabilities.get("limits").expect("limits");
    assert_eq!(number(limits, "max_run_wall_ms"), 30_000);
    assert_eq!(number(limits, "max_operation_journal"), 1_024);
    assert_eq!(number(limits, "operation_ttl"), 600);
    assert_eq!(number(limits, "max_closed_retained"), 64);
    assert_eq!(number(limits, "closed_retention_ttl"), 300);
    // The v2 control operations are advertised, so discovery and the implementation
    // cannot drift apart.
    let names: Vec<&str> = capabilities
        .get("ops")
        .and_then(JsonValue::as_array)
        .expect("ops")
        .iter()
        .filter_map(|entry| entry.get("name").and_then(JsonValue::as_str))
        .collect();
    for expected in ["session.cancel", "session.progress", "session.operation"] {
        assert!(names.contains(&expected), "{expected} is advertised");
    }
}

#[test]
fn the_error_envelope_declares_the_version_that_introduces_the_control_codes() {
    let json = ServiceError::new(ErrorCode::Cancelled, "stopped").to_json();
    assert_eq!(
        json.get("wire_version").and_then(JsonValue::as_u64),
        Some(2)
    );
    assert_eq!(
        json.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1"),
        "the schema string is stable; the version is the declared change"
    );
    assert_eq!(
        json.get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("cancelled")
    );
}
