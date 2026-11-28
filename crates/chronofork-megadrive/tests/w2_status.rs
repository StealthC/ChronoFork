//! W2.4a — cheap status: no digest, no memory copy, bounded separated maps.
//!
//! The wrapper counts digest and memory-read calls; `status()` must touch neither, while
//! `inspect()` may return register detail but must stay digest-free and memory-free.

use std::cell::Cell;
use std::error::Error;
use std::time::Instant;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendSnapshot, BatchSignals, MAX_STATUS_REGISTERS,
    MachineInspection, MachineStatus, MediaSource, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash};
use chronofork_megadrive::{MegadriveBackend, MegadriveFactory, SchedulerConfig, interrupt_rom};

type TestResult = Result<(), Box<dyn Error>>;

struct CountingBackend {
    inner: MegadriveBackend,
    hashes: Cell<usize>,
    reads: Cell<usize>,
}

impl CountingBackend {
    fn new() -> Result<Self, Box<dyn Error>> {
        let factory =
            MegadriveFactory::with_scheduler_config(interrupt_rom(3), SchedulerConfig::ntsc())?;
        Ok(Self {
            inner: factory.create_machine()?,
            hashes: Cell::new(0),
            reads: Cell::new(0),
        })
    }
}

impl Backend for CountingBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        self.inner.descriptor()
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.hashes.set(self.hashes.get() + 1);
        self.inner.state_hash()
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.inner.step(input)
    }

    fn batch_signals(&self) -> BatchSignals {
        self.inner.batch_signals()
    }

    fn status(&self) -> Result<MachineStatus, BackendError> {
        self.inner.status()
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        self.inner.inspect()
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        self.reads.set(self.reads.get() + 1);
        self.inner.read_memory(address, length)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        self.inner.snapshot()
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        self.inner.restore(snapshot)
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        self.inner.write_memory(address, data)
    }

    fn media_mut(&mut self) -> Option<&mut dyn MediaSource> {
        self.inner.media_mut()
    }
}

#[test]
fn status_has_no_hash_or_copy() -> TestResult {
    let mut backend = CountingBackend::new()?;
    backend.step(InputFrame::NEUTRAL)?;

    let status = backend.status()?;
    assert_eq!(
        backend.hashes.get(),
        0,
        "status must not compute a canonical digest"
    );
    assert_eq!(
        backend.reads.get(),
        0,
        "status must not copy any memory window"
    );
    assert!(status.frame_index >= 1 || status.tick.raw() > 0);

    // Inspection is detail-on-demand: still no digest, no memory by default.
    let inspection = backend.inspect()?;
    assert_eq!(backend.hashes.get(), 0, "inspect must not compute a digest");
    assert_eq!(backend.reads.get(), 0, "inspect must not copy memory");
    assert!(
        inspection.memory.is_none(),
        "memory requires an explicit window"
    );

    // A window only appears when the caller asks for it.
    let windowed = backend.inspect_with_memory(0, 4)?;
    assert_eq!(backend.reads.get(), 1);
    assert!(windowed.memory.is_some());

    // Informational latency note: no hard threshold, because the host is not calibrated.
    let started = Instant::now();
    let trials = 10_000_u32;
    for _ in 0..trials {
        let _ = backend.status()?;
    }
    let per_call = started.elapsed() / trials;
    println!("status latency: {per_call:?} per call over {trials} calls (informational)");
    Ok(())
}

#[test]
fn status_registers_are_bounded_and_separated() -> TestResult {
    let backend = CountingBackend::new()?;
    let status = backend.status()?;
    assert!(
        status.cpu_registers.len() + status.device_registers.len() <= MAX_STATUS_REGISTERS,
        "status maps exceed MAX_STATUS_REGISTERS"
    );
    assert!(
        status
            .cpu_registers
            .keys()
            .all(|name| !status.device_registers.contains_key(name)),
        "a register name must not appear in both maps"
    );
    assert!(status.cpu_registers.contains_key("pc"));
    assert!(status.device_registers.contains_key("vdp_status"));
    assert_eq!(status.registers().len(), status.register_count());
    Ok(())
}
