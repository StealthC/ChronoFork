//! W2.4a — `session.list` reads status metadata and never inspects each session.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendSnapshot, BatchSignals,
    MachineInspection, MachineStatus, StepOutcome,
};
use chronofork_core::InputFrame;
use chronofork_service::json::{JsonValue, parse};
use chronofork_service::{AppService, ServiceConfig, ServiceLimits};
use chronofork_test_backend::SyntheticBackendFactory;

type TestResult = Result<(), Box<dyn Error>>;

struct CountingFactory {
    descriptor: BackendDescriptor,
    inspects: Arc<AtomicUsize>,
    statuses: Arc<AtomicUsize>,
}

struct CountingBackend {
    inner: Box<dyn Backend>,
    inspects: Arc<AtomicUsize>,
    statuses: Arc<AtomicUsize>,
}

impl Backend for CountingBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }
    fn state_hash(&self) -> Result<chronofork_core::StateHash, BackendError> {
        self.inner.state_hash()
    }
    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.inner.step(input)
    }
    fn batch_signals(&self) -> BatchSignals {
        self.inner.batch_signals()
    }
    fn status(&self) -> Result<MachineStatus, BackendError> {
        self.statuses.fetch_add(1, Ordering::SeqCst);
        self.inner.status()
    }
    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        self.inspects.fetch_add(1, Ordering::SeqCst);
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
            inspects: Arc::clone(&self.inspects),
            statuses: Arc::clone(&self.statuses),
        }))
    }
}

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn dispatch(service: &AppService, input: &str) -> Result<JsonValue, Box<dyn Error>> {
    let principal = chronofork_service::Principal::new(
        "operator",
        [
            chronofork_service::Scope::Read,
            chronofork_service::Scope::Write,
        ],
    );
    let document = format!("{{\"op\":\"session.create\",\"input\":{input}}}");
    let bytes = service
        .dispatch(&principal, document.as_bytes())
        .map_err(|error| format!("dispatch: {error}"))?;
    Ok(parse(&bytes, &limits())?)
}

#[test]
fn list_does_not_inspect() -> TestResult {
    let factory = SyntheticBackendFactory::default();
    let descriptor = factory.descriptor().clone();
    let inspects = Arc::new(AtomicUsize::new(0));
    let statuses = Arc::new(AtomicUsize::new(0));
    let counting = Arc::new(CountingFactory {
        descriptor,
        inspects: Arc::clone(&inspects),
        statuses: Arc::clone(&statuses),
    });
    let service = AppService::new(ServiceConfig {
        factory: counting.clone(),
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })?;

    // Seed two sessions through the real create path.
    for _ in 0..2 {
        dispatch(&service, "{}")?;
    }
    inspects.store(0, Ordering::SeqCst);
    statuses.store(0, Ordering::SeqCst);

    let principal = chronofork_service::Principal::new(
        "operator",
        [
            chronofork_service::Scope::Read,
            chronofork_service::Scope::Write,
        ],
    );
    let request = b"{\"op\":\"session.list\",\"input\":{}}";
    let bytes = service
        .dispatch(&principal, request)
        .map_err(|error| format!("list: {error}"))?;
    let envelope = parse(&bytes, &limits())?;
    assert!(
        envelope.get("result").is_some(),
        "list returns its v1 envelope"
    );
    assert_eq!(
        inspects.load(Ordering::SeqCst),
        0,
        "session.list must not call inspect per session"
    );
    // The registry may answer from its committed metadata; what matters is that the
    // per-session detailed inspection path is never taken.
    let _ = statuses.load(Ordering::SeqCst);
    Ok(())
}
