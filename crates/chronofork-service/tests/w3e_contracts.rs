#![allow(clippy::expect_used)]

//! W3e contract tests: the four contracts of the R1b re-review (F1–F4).
//!
//! Each test is one of the adversarial probes R1b ran against the W3d delivery, kept as a
//! permanent regression, plus the boundary case the probe does not reach:
//!
//! - **F1** — a key recognized after its outcome left the horizon stays recognized even
//!   when many later keys are used, and the identity memory refuses a new key instead of
//!   forgetting a used one;
//! - **F2** — the process identity is durable, published in v2 mutation results and
//!   validated before a keyed mutation, so a key from another epoch is an
//!   `unknown_outcome` and is never re-executed;
//! - **F3** — admission is decided before anything is built or captured, a growing
//!   payload is refused before the capture, and a refused concurrent fork leaves the
//!   parent exactly as it was;
//! - **F4** — a mutation marks its first irreversible effect, so a failure after it is
//!   recorded and a retry with the same key replays the recorded outcome instead of
//!   applying the effect twice, and `store.save` persists nothing before its fallible
//!   work is done.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, DebugAccess,
    MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash};
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

fn request(op: &str, input: &JsonValue) -> Vec<u8> {
    let document = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
    to_canonical_bytes(&document, &limits()).expect("request bytes")
}

fn input(text: &str) -> JsonValue {
    parse(text.as_bytes(), &limits()).expect("input JSON")
}

fn dispatch_version(
    service: &AppService,
    op: &str,
    text: &str,
    version: ApiVersion,
) -> Result<JsonValue, ServiceError> {
    let bytes = service.dispatch_versioned(&operator(), &request(op, &input(text)), version)?;
    let envelope = parse(&bytes, &limits()).expect("envelope");
    Ok(envelope.get("result").expect("result").clone())
}

fn dispatch_v2(service: &AppService, op: &str, text: &str) -> Result<JsonValue, ServiceError> {
    dispatch_version(service, op, text, ApiVersion::V2)
}

fn ok(service: &AppService, op: &str, text: &str) -> JsonValue {
    dispatch_v2(service, op, text).expect("operation must succeed")
}

fn number(value: &JsonValue, name: &str) -> u64 {
    value
        .get(name)
        .and_then(JsonValue::as_u64)
        .expect("unsigned field")
}

fn plain_service(limits: ServiceLimits) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits,
    })
    .expect("service")
}

/// A removed-on-drop temporary directory.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("chronofork-w3e-{}-{name}", std::process::id()));
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

// ---------------------------------------------------------------------------
// A counting backend: construction, capture and payload growth are observable
// ---------------------------------------------------------------------------

struct Counters {
    created: AtomicUsize,
    captured: AtomicUsize,
    growth: AtomicUsize,
}

impl Counters {
    fn new() -> Self {
        Self {
            created: AtomicUsize::new(0),
            captured: AtomicUsize::new(0),
            growth: AtomicUsize::new(0),
        }
    }
}

struct CountingBackend {
    inner: Box<dyn Backend>,
    counters: Arc<Counters>,
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
        let snapshot = self.inner.snapshot()?;
        let growth = if self.steps > 0 {
            self.counters.growth.load(Ordering::SeqCst)
        } else {
            0
        };
        if growth == 0 {
            return Ok(snapshot);
        }
        // A backend whose payload grows after execution: the byte quota must refuse the
        // capture before it happens, not after serializing it.
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

struct CountingFactory {
    inner: SyntheticBackendFactory,
    counters: Arc<Counters>,
    fail_after: Option<usize>,
    /// Construction index (1-based) that must fail, if any.
    fail_create_at: Option<usize>,
}

impl BackendFactory for CountingFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        let created = self.counters.created.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_create_at == Some(created) {
            return Err(BackendError::Halted);
        }
        Ok(Box::new(CountingBackend {
            inner: self.inner.create()?,
            counters: Arc::clone(&self.counters),
            fail_after: self.fail_after,
            steps: 0,
        }))
    }
}

fn counted_service(
    limits: ServiceLimits,
    counters: &Arc<Counters>,
    storage_root: Option<PathBuf>,
) -> AppService {
    counted_service_with(limits, counters, storage_root, None, None)
}

fn counted_service_with(
    limits: ServiceLimits,
    counters: &Arc<Counters>,
    storage_root: Option<PathBuf>,
    fail_after: Option<usize>,
    fail_create_at: Option<usize>,
) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(CountingFactory {
            inner: SyntheticBackendFactory::default(),
            counters: Arc::clone(counters),
            fail_after,
            fail_create_at,
        }),
        storage_root,
        artifacts_root: None,
        limits,
    })
    .expect("counting service")
}

fn durable_service(root: &Path) -> AppService {
    AppService::new_durable(
        ServiceConfig {
            factory: Arc::new(SyntheticBackendFactory::default()),
            storage_root: None,
            artifacts_root: None,
            limits: limits(),
        },
        root,
    )
    .expect("durable service")
}

// ---------------------------------------------------------------------------
// F1 — the identity memory
// ---------------------------------------------------------------------------

/// R1b probe: with `max_operation_journal = 1` and `operation_ttl = 0`, keys 1, 2 and 3
/// evicted the first tombstone and a retry of key 1 created a **fourth** session.
#[test]
fn f1_an_expired_key_is_recognized_under_tombstone_pressure() {
    let service = plain_service(ServiceLimits {
        operation_ttl: 0,
        max_operation_journal: 1,
        ..limits()
    });
    for key in [1_u64, 2, 3] {
        let created = ok(
            &service,
            "session.create",
            &format!("{{\"operation_id\":{key}}}"),
        );
        assert_eq!(number(&created, "session"), key);
    }
    assert_eq!(service.session_count().expect("count"), 3);

    let retry = dispatch_v2(&service, "session.create", "{\"operation_id\":1}")
        .expect_err("a used key is never executed twice");
    assert_eq!(
        retry.code(),
        ErrorCode::UnknownOutcome,
        "the first key is still recognized after two later keys"
    );
    assert_eq!(
        service.session_count().expect("count"),
        3,
        "the refused retry created nothing"
    );
}

/// The identity memory is bounded and **fail-closed**: when it cannot remember one more
/// key the service refuses the new key before any work, instead of evicting an identity
/// and re-executing a mutation the client may already have applied.
#[test]
fn f1_a_full_identity_memory_refuses_a_new_key() {
    let service = plain_service(ServiceLimits {
        operation_ttl: 0,
        max_operation_journal: 1,
        ..limits()
    });
    ok(&service, "session.create", "{}");
    let identities = 1_024_u64;
    for key in 1..=identities {
        ok(
            &service,
            "session.run",
            &format!("{{\"session\":1,\"steps\":1,\"operation_id\":{key}}}"),
        );
    }
    // The identity memory is at its bound: a new key is refused, not remembered by
    // forgetting an older one.
    let refused = dispatch_v2(
        &service,
        "session.run",
        &format!(
            "{{\"session\":1,\"steps\":1,\"operation_id\":{}}}",
            identities + 1
        ),
    )
    .expect_err("the identity memory is full");
    assert_eq!(refused.code(), ErrorCode::LimitExceeded);
    // Every remembered key still answers `unknown_outcome`; none became fresh again.
    let retry = dispatch_v2(
        &service,
        "session.run",
        "{\"session\":1,\"steps\":1,\"operation_id\":1}",
    )
    .expect_err("the first key is still recognized");
    assert_eq!(retry.code(), ErrorCode::UnknownOutcome);
}

// ---------------------------------------------------------------------------
// F2 — the durable process identity
// ---------------------------------------------------------------------------

#[test]
fn f2_the_epoch_is_persisted_and_never_reused_across_starts() {
    let directory = TempDir::new("epoch");
    let root = directory.join("store");
    let first = durable_service(&root);
    let second = durable_service(&root);
    let first_epoch = first.process_epoch();
    let second_epoch = second.process_epoch();
    assert_eq!(
        second_epoch,
        first_epoch + 1,
        "a restart mints the next epoch, never the same one"
    );
    let path = directory.join("store.chronofork-epoch");
    let persisted = std::fs::read_to_string(&path).expect("the identity file is created");
    assert_eq!(
        persisted.trim(),
        second_epoch.to_string(),
        "the durable counter carries the last minted epoch"
    );
}

#[test]
fn f2_a_key_from_another_epoch_is_an_unknown_outcome() {
    let directory = TempDir::new("binding");
    let root = directory.join("store");
    let first = durable_service(&root);
    let first_epoch = first.process_epoch();
    let created = ok(
        &first,
        "session.create",
        &format!("{{\"operation_id\":7,\"operation_epoch\":{first_epoch}}}"),
    );
    assert_eq!(number(&created, "session"), 1);
    assert_eq!(
        number(&created, "process_epoch"),
        first_epoch,
        "a v2 mutation result names its process"
    );

    // A second process (same durable identity file) mints the next epoch.
    let second = durable_service(&root);
    let second_epoch = second.process_epoch();
    assert!(second_epoch > first_epoch);

    // The retry of the first process's key is never re-executed.
    let retry = dispatch_v2(
        &second,
        "session.create",
        &format!("{{\"operation_id\":7,\"operation_epoch\":{first_epoch}}}"),
    )
    .expect_err("a key from another epoch cannot be reconciled");
    assert_eq!(retry.code(), ErrorCode::UnknownOutcome);
    assert_eq!(second.session_count().expect("count"), 0);

    // A key that does not name its epoch is refused by a durable service: without it the
    // server cannot tell a fresh intention from a retry of the previous process.
    let unnamed = dispatch_v2(&second, "session.create", "{\"operation_id\":8}")
        .expect_err("a durable service requires the epoch");
    assert_eq!(unnamed.code(), ErrorCode::UnknownOutcome);
    assert_eq!(second.session_count().expect("count"), 0);

    // A key minted under the current epoch is served, and a v1 mutation keeps the legacy
    // shape without the operational field.
    let current = ok(
        &second,
        "session.create",
        &format!("{{\"operation_id\":7,\"operation_epoch\":{second_epoch}}}"),
    );
    assert_eq!(number(&current, "session"), 1);
    let v1 = dispatch_version(&second, "session.create", "{}", ApiVersion::V1)
        .expect("a v1 create is still served");
    assert!(
        v1.get("process_epoch").is_none(),
        "the v1 shape never carries the operational epoch"
    );
}

// ---------------------------------------------------------------------------
// F3 — admission before any allocation
// ---------------------------------------------------------------------------

/// R1b probe: with `max_sessions = 1` a refused create still constructed a second backend.
#[test]
fn f3_a_rejected_create_constructs_nothing() {
    let counters = Arc::new(Counters::new());
    let service = counted_service(
        ServiceLimits {
            max_sessions: 1,
            ..limits()
        },
        &counters,
        None,
    );
    ok(&service, "session.create", "{}");
    assert_eq!(counters.created.load(Ordering::SeqCst), 1);
    for _ in 0..8 {
        assert_eq!(
            dispatch_v2(&service, "session.create", "{}")
                .expect_err("limit")
                .code(),
            ErrorCode::LimitExceeded
        );
    }
    assert_eq!(
        counters.created.load(Ordering::SeqCst),
        1,
        "strict admission precedes backend construction; a rollback after construction is \
         not a preflight"
    );
}

/// R1b probe: a backend whose payload grows 4 KiB after a step had its state captured
/// before the byte quota rejected the checkpoint.
#[test]
fn f3_a_growing_payload_is_rejected_before_the_capture() {
    let reference = plain_service(limits());
    ok(&reference, "session.create", "{}");
    let bytes = number(
        &ok(&reference, "session.snapshot", "{\"session\":1}"),
        "bytes_length",
    ) as usize;

    let counters = Arc::new(Counters::new());
    counters.growth.store(4096, Ordering::SeqCst);
    let service = counted_service(
        ServiceLimits {
            max_bytes_per_session: bytes + 1,
            ..limits()
        },
        &counters,
        None,
    );
    ok(&service, "session.create", "{}");
    ok(&service, "session.run", "{\"session\":1,\"steps\":1}");
    let before = counters.captured.load(Ordering::SeqCst);
    let rejected = dispatch_v2(&service, "session.snapshot", "{\"session\":1}")
        .expect_err("the grown payload cannot fit");
    assert_eq!(rejected.code(), ErrorCode::LimitExceeded);
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        before,
        "the quota was checked before the growing payload was captured"
    );
}

/// Two concurrent forks race for the last slot (R1c C2).
///
/// The old probe parked **both captures** on a barrier; that rendezvous was a diagnostic
/// for the pre-reservation design, not a requirement: with admission reserved before the
/// capture, the refused fork must not capture at all. The barrier is therefore moved to
/// the start of the race, and the refusal has to leave the parent exactly as it was —
/// no checkpoint, no child construction and no consumed checkpoint identifier.
#[test]
fn f3_a_refused_concurrent_fork_preserves_its_parent() {
    let counters = Arc::new(Counters::new());
    let service = Arc::new(counted_service(
        ServiceLimits {
            max_sessions: 3,
            ..limits()
        },
        &counters,
        None,
    ));
    ok(&service, "session.create", "{}");
    ok(&service, "session.create", "{}");
    let created_before = counters.created.load(Ordering::SeqCst);
    let captured_before = counters.captured.load(Ordering::SeqCst);

    // Both forks enter the admission race together; only one can reserve the last slot.
    let start = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (1..=2_u64)
        .map(|id| {
            let service = Arc::clone(&service);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                let outcome =
                    dispatch_v2(&service, "session.fork", &format!("{{\"session\":{id}}}"));
                (id, outcome)
            })
        })
        .collect();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().expect("fork thread"))
        .collect();
    assert_eq!(
        results.iter().filter(|(_, result)| result.is_ok()).count(),
        1,
        "exactly one fork takes the last slot"
    );
    let (parent, error) = results
        .iter()
        .find(|(_, result)| result.is_err())
        .expect("one fork is refused");
    assert_eq!(
        error.as_ref().expect_err("refused").code(),
        ErrorCode::LimitExceeded
    );

    // The successful fork builds exactly one child; the refused one builds nothing.
    assert_eq!(
        counters.created.load(Ordering::SeqCst),
        created_before.saturating_add(1),
        "the refused fork constructed no child backend"
    );
    // The refused fork captured nothing: the only captures are the winner's origin
    // checkpoint and the child's initial state.
    assert_eq!(
        counters.captured.load(Ordering::SeqCst),
        captured_before.saturating_add(2),
        "the refused fork captured no parent checkpoint"
    );
    // And the refused parent is untouched: no checkpoint and no consumed identifier.
    let next = ok(
        &service,
        "session.snapshot",
        &format!("{{\"session\":{parent}}}"),
    );
    assert_eq!(
        number(&next, "snapshots"),
        1,
        "the refused fork left no origin checkpoint in parent {parent}"
    );
    assert_eq!(
        number(&next, "snapshot"),
        1,
        "the refused fork consumed no checkpoint identifier"
    );
}

// ---------------------------------------------------------------------------
// F4 — the journal records the first irreversible effect
// ---------------------------------------------------------------------------

/// A fork whose child backend cannot be built fails **after** the parent captured its
/// origin checkpoint. The retry with the same key replays the recorded failure instead of
/// capturing the parent's checkpoint a second time.
#[test]
fn f4_a_fork_failure_after_its_first_effect_is_replayed() {
    let counters = Arc::new(Counters::new());
    // The parent is construction 1; the child's construction (2) fails.
    let service = counted_service_with(limits(), &counters, None, None, Some(2));
    ok(&service, "session.create", "{}");

    let first = dispatch_v2(
        &service,
        "session.fork",
        "{\"session\":1,\"operation_id\":9}",
    )
    .expect_err("the child backend cannot be built");
    // The parent captured its origin checkpoint before the child failed: the mutation
    // applied a partial effect and must be recorded.
    let after_failure = ok(&service, "session.snapshot", "{\"session\":1}");
    assert_eq!(number(&after_failure, "snapshots"), 2);

    // The retry replays the recorded outcome: no second origin checkpoint is captured.
    let retry = dispatch_v2(
        &service,
        "session.fork",
        "{\"session\":1,\"operation_id\":9}",
    )
    .expect_err("the recorded failure is replayed");
    assert_eq!(retry.code(), first.code());
    ok(&service, "session.snapshot", "{\"session\":1}");
    let after_retry = dispatch_v2(&service, "session.snapshot", "{\"session\":1}")
        .expect("the parent still has room");
    assert_eq!(
        number(&after_retry, "snapshots"),
        4,
        "two explicit captures after one origin, and the replay added none"
    );
}

/// `store.save` must not publish a durable session before its fallible work is done. A
/// failing advance leaves the store empty, and because nothing was applied the retry is
/// free to execute again instead of replaying a failure.
#[test]
fn f4_store_save_persists_nothing_before_its_fallible_work() {
    let directory = TempDir::new("store-save");
    let root = directory.join("store");
    let counters = Arc::new(Counters::new());
    // Every step fails: the advance cannot complete.
    let service = counted_service_with(limits(), &counters, Some(root), Some(0), None);

    let first = dispatch_v2(&service, "store.save", "{\"steps\":2,\"operation_id\":7}")
        .expect_err("the advance fails");
    assert_eq!(first.code(), ErrorCode::Halted);
    let listing = ok(&service, "store.list", "{}");
    assert_eq!(
        listing
            .get("sessions")
            .and_then(JsonValue::as_array)
            .map(<[JsonValue]>::len),
        Some(0),
        "a failed save published no durable session"
    );

    // Nothing was applied, so the same key re-executes (a new backend is constructed)
    // instead of replaying the earlier failure.
    let constructed = counters.created.load(Ordering::SeqCst);
    let retry = dispatch_v2(&service, "store.save", "{\"steps\":2,\"operation_id\":7}")
        .expect_err("the retry fails the same way");
    assert_eq!(retry.code(), ErrorCode::Halted);
    assert!(
        counters.created.load(Ordering::SeqCst) > constructed,
        "an abandoned entry leaves the key fresh: the retry built a new backend"
    );
}
