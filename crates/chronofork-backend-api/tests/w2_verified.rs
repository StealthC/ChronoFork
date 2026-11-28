//! W2.1/W2.2 — the default `Backend::step_verified` composition.
//!
//! The trait's default must be `step` followed by exactly one `state_hash()` call on
//! the resulting state. A counting backend makes that composition observable without
//! depending on a concrete machine: after W2-2 the ordinary `step` computes no digest
//! at all and therefore never calls the trait method, while the default
//! `step_verified` calls it exactly once per step.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendId, BackendSnapshot, Capabilities,
    InputSequence, MachineInspection, RunControl, StepOutcome, StopBudget, StopSet,
    VerificationPolicy,
};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};

type TestResult = Result<(), Box<dyn Error>>;

/// A tiny backend that counts calls to the **trait** digest method.
struct CountingBackend {
    descriptor: BackendDescriptor,
    tick: Tick,
    halted: bool,
    hash_calls: Cell<usize>,
}

impl CountingBackend {
    fn new() -> Self {
        Self {
            descriptor: BackendDescriptor::new(
                BackendId::new("chronofork.test.counting"),
                "Default verified-step composition fixture (not an emulator)",
                "0",
                Capabilities::none(),
                true,
            ),
            tick: Tick::ZERO,
            halted: false,
            hash_calls: Cell::new(0),
        }
    }

    fn hash_calls(&self) -> usize {
        self.hash_calls.get()
    }

    /// The canonical digest body this fixture would compute if asked. The ordinary
    /// `step` no longer calls it (W2-2).
    fn digest(&self) -> StateHash {
        hash_bytes(&self.tick.raw().to_le_bytes())
    }
}

impl Backend for CountingBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.hash_calls.set(self.hash_calls.get() + 1);
        Ok(self.digest())
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        if self.halted {
            return Err(BackendError::Halted);
        }
        self.tick = self.tick.next()?;
        Ok(StepOutcome {
            tick: self.tick,
            // W2-3: ordinary execution carries no digest and makes no trait call.
            halted: self.halted,
            verified: false,
        })
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        Err(BackendError::Halted)
    }

    fn restore(&mut self, _snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        Err(BackendError::Halted)
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        Ok(MachineInspection {
            tick: self.tick,
            halted: self.halted,
            frame_index: 0,
            cpu_registers: BTreeMap::new(),
            device_registers: BTreeMap::new(),
            memory: None,
        })
    }

    fn read_memory(&self, _address: u64, _length: usize) -> Result<Vec<u8>, BackendError> {
        Ok(Vec::new())
    }

    fn write_memory(&mut self, _address: u64, _data: &[u8]) -> Result<(), BackendError> {
        Ok(())
    }
}

#[test]
fn verified_step_hashes_exactly_once_per_step() -> TestResult {
    let mut backend = CountingBackend::new();
    assert_eq!(backend.hash_calls(), 0);

    // W2-2: the ordinary path computes no digest and makes no trait call.
    let ordinary = backend.step(InputFrame::NEUTRAL)?;
    assert_eq!(
        backend.hash_calls(),
        0,
        "an ordinary step makes no trait digest call"
    );
    assert!(!ordinary.verified);

    // The default composition is `step` + exactly one `state_hash()`, and the result
    // claims verification.
    let verified = backend.step_verified(InputFrame::NEUTRAL)?;
    assert_eq!(
        backend.hash_calls(),
        1,
        "a verified step calls state_hash exactly once"
    );
    assert_eq!(verified.state_hash, backend.digest());

    // An explicit call adds exactly one more; another ordinary step adds none.
    assert_eq!(backend.state_hash()?, backend.digest());
    assert_eq!(backend.hash_calls(), 2);
    backend.step(InputFrame::NEUTRAL)?;
    assert_eq!(
        backend.hash_calls(),
        2,
        "an ordinary step after the verified one adds no digest call"
    );

    // A second verified step adds exactly one.
    backend.step_verified(InputFrame::NEUTRAL)?;
    assert_eq!(backend.hash_calls(), 3);
    Ok(())
}

#[test]
fn ordinary_batch_computes_no_digest() -> TestResult {
    let mut backend = CountingBackend::new();
    let inputs = InputSequence::neutral();
    let stop = StopSet::default();

    let ordinary = backend.run_batch(
        &inputs,
        StopBudget::Steps(3),
        &stop,
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    assert_eq!(ordinary.performed, 3);
    assert!(
        ordinary.state_hash.is_none(),
        "an ordinary batch must not compute a digest"
    );
    assert_eq!(
        backend.hash_calls(),
        0,
        "an ordinary batch makes no trait digest call"
    );

    // The same primitive serves the digest request, exactly once per verified step.
    let verified = backend.run_batch(
        &inputs,
        StopBudget::Steps(2),
        &stop,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(verified.performed, 2);
    assert_eq!(backend.hash_calls(), 2);
    assert_eq!(verified.state_hash, Some(backend.digest()));
    Ok(())
}

#[test]
fn over_budget_inputs_are_rejected_before_any_step() -> TestResult {
    let mut backend = CountingBackend::new();
    let before = backend.inspect()?.tick;
    let inputs = InputSequence::new(vec![InputFrame::NEUTRAL; 5])?;
    let result = backend.run_batch(
        &inputs,
        StopBudget::Steps(2),
        &StopSet::default(),
        VerificationPolicy::None,
        &RunControl::new(),
    );
    assert!(result.is_err(), "five inputs do not fit a two-step budget");
    let error = match result {
        Ok(_) => return Err("the over-budget batch must be rejected".into()),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            BackendError::Limit {
                field: "input_frames",
                limit: 2,
                requested: 5
            }
        ),
        "typed limit: {error:?}"
    );
    assert_eq!(backend.inspect()?.tick, before, "no step ran");
    assert_eq!(backend.hash_calls(), 0);
    Ok(())
}
