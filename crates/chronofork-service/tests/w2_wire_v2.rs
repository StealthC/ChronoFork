//! W2.4b — server-side ChronoFork API v2 shape: cycles, inspect split, one hash.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, MachineInspection,
    MachineStatus, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash};
use chronofork_service::json::{JsonValue, parse};
use chronofork_service::{ApiVersion, AppService, Principal, Scope, ServiceConfig, ServiceLimits};
use chronofork_test_backend::SyntheticBackendFactory;

type TestResult = Result<(), Box<dyn Error>>;

struct CountingFactory {
    descriptor: BackendDescriptor,
    hashes: Arc<AtomicUsize>,
}

struct CountingBackend {
    inner: Box<dyn Backend>,
    hashes: Arc<AtomicUsize>,
}

impl Backend for CountingBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }
    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.hashes.fetch_add(1, Ordering::SeqCst);
        self.inner.state_hash()
    }
    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.inner.step(input)
    }
    fn batch_signals(&self) -> chronofork_backend_api::BatchSignals {
        self.inner.batch_signals()
    }
    fn status(&self) -> Result<MachineStatus, BackendError> {
        self.inner.status()
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
    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.inner.snapshot()
    }
    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        self.inner.restore(snapshot)
    }
}

impl BackendFactory for CountingFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        let inner = SyntheticBackendFactory::default().create()?;
        Ok(Box::new(CountingBackend {
            inner,
            hashes: Arc::clone(&self.hashes),
        }))
    }
}

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn principal() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn dispatch(
    service: &AppService,
    version: ApiVersion,
    op: &str,
    input: &str,
) -> Result<JsonValue, Box<dyn Error>> {
    let request = format!("{{\"op\":\"{op}\",\"input\":{input}}}");
    let bytes = service
        .dispatch_versioned(&principal(), request.as_bytes(), version)
        .map_err(|error| format!("{op}: {error}"))?;
    Ok(parse(&bytes, &limits())?)
}

fn result<'a>(document: &'a JsonValue, op: &str) -> Result<&'a JsonValue, Box<dyn Error>> {
    document
        .get("result")
        .ok_or_else(|| format!("{op} has a result").into())
}

fn number(value: &JsonValue, field: &str) -> Result<u64, Box<dyn Error>> {
    value
        .get(field)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| format!("{field} is an integer").into())
}

#[test]
fn v2_run_exposes_batch_fields_and_one_hash() -> TestResult {
    let factory = SyntheticBackendFactory::default();
    let descriptor = factory.descriptor().clone();
    let hashes = Arc::new(AtomicUsize::new(0));
    let counting = Arc::new(CountingFactory {
        descriptor,
        hashes: Arc::clone(&hashes),
    });
    let service = AppService::new(ServiceConfig {
        factory: counting,
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })?;
    dispatch(&service, ApiVersion::V2, "session.create", "{}")?;
    hashes.store(0, Ordering::SeqCst);

    let run = dispatch(
        &service,
        ApiVersion::V2,
        "session.run",
        "{\"session\":1,\"steps\":25}",
    )?;
    let run = result(&run, "session.run")?;
    assert_eq!(number(run, "performed")?, 25);
    assert_eq!(number(run, "steps")?, 25);
    assert!(
        number(run, "cycles")? > 0,
        "v2 run reports the batch cycle count"
    );
    assert_eq!(
        run.get("stop_reason").and_then(JsonValue::as_str),
        Some("completed")
    );
    assert!(run.get("state_hash").is_some(), "v1/v2 keep state_hash");
    assert_eq!(
        hashes.load(Ordering::SeqCst),
        1,
        "the run hashes once per operation, not per step"
    );

    // v2 inspect is served through the split with the documented machine shape and
    // also hashes exactly once.
    hashes.store(0, Ordering::SeqCst);
    let inspect = dispatch(
        &service,
        ApiVersion::V2,
        "session.inspect",
        "{\"session\":1}",
    )?;
    let inspect = result(&inspect, "session.inspect")?;
    let machine = inspect.get("machine").ok_or("machine object")?;
    assert!(machine.get("tick").is_some());
    assert!(machine.get("state_hash").is_some());
    assert!(machine.get("halted").is_some());
    assert!(machine.get("registers").is_some());
    assert!(machine.get("memory_base").is_some());
    assert!(machine.get("memory_length").is_some());
    assert_eq!(
        hashes.load(Ordering::SeqCst),
        1,
        "inspect hashes once for its v1/v2 state_hash field"
    );

    // The v1 projection still removes the batch-only fields.
    let v1 = dispatch(
        &service,
        ApiVersion::V1,
        "session.inspect",
        "{\"session\":1}",
    )?;
    let v1_machine = result(&v1, "session.inspect")?
        .get("machine")
        .ok_or("v1 machine")?
        .clone();
    assert_eq!(
        v1_machine.get("tick").and_then(JsonValue::as_u64),
        machine.get("tick").and_then(JsonValue::as_u64)
    );
    Ok(())
}

#[test]
fn v1_run_shape_has_no_v2_fields() -> TestResult {
    let service = AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })?;
    dispatch(&service, ApiVersion::V1, "session.create", "{}")?;
    let run = dispatch(
        &service,
        ApiVersion::V1,
        "session.run",
        "{\"session\":1,\"steps\":5}",
    )?;
    let run = result(&run, "session.run")?;
    assert_eq!(number(run, "steps")?, 5);
    for absent in [
        "performed",
        "cycles",
        "stop_reason",
        "revision",
        "max_wall_ms",
    ] {
        assert!(
            run.get(absent).is_none(),
            "the v1 run shape must not leak `{absent}`"
        );
    }
    assert!(run.get("state_hash").is_some());
    Ok(())
}

#[test]
fn expected_tick_uses_status_and_one_digest() -> TestResult {
    let factory = SyntheticBackendFactory::default();
    let descriptor = factory.descriptor().clone();
    let hashes = Arc::new(AtomicUsize::new(0));
    let counting = Arc::new(CountingFactory {
        descriptor,
        hashes: Arc::clone(&hashes),
    });
    let service = AppService::new(ServiceConfig {
        factory: counting,
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })?;
    dispatch(&service, ApiVersion::V2, "session.create", "{}")?;
    hashes.store(0, Ordering::SeqCst);

    // A matching expected_tick precondition is answered from the hash-free status, so the
    // successful run keeps exactly the one final response digest.
    let run = dispatch(
        &service,
        ApiVersion::V2,
        "session.run",
        "{\"session\":1,\"steps\":3,\"expected_tick\":0}",
    )?;
    let run = result(&run, "session.run")?;
    assert_eq!(number(run, "performed")?, 3);
    assert_eq!(
        hashes.load(Ordering::SeqCst),
        1,
        "expected_tick does not add a second digest"
    );

    // A rejected precondition applies zero steps and needs no digest at all.
    hashes.store(0, Ordering::SeqCst);
    let request =
        b"{\"op\":\"session.run\",\"input\":{\"session\":1,\"steps\":2,\"expected_tick\":999}}";
    let error = match service.dispatch_versioned(&principal(), request, ApiVersion::V2) {
        Ok(_) => return Err("the stale expected_tick must be rejected".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), chronofork_service::ErrorCode::Conflict);
    assert_eq!(
        hashes.load(Ordering::SeqCst),
        0,
        "a rejected precondition executes no step and needs no digest"
    );

    // Zero steps were applied: the tick is still the post-run tick.
    let inspect = dispatch(
        &service,
        ApiVersion::V2,
        "session.inspect",
        "{\"session\":1}",
    )?;
    let machine = result(&inspect, "session.inspect")?
        .get("machine")
        .ok_or("machine")?;
    assert_eq!(machine.get("tick").and_then(JsonValue::as_u64), Some(3));
    Ok(())
}
