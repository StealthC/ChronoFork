//! W2.3c — lock-free cancellation and the absolute host deadline at step boundaries.
//!
//! The cancellation trigger is deterministic: the fixture backend cancels the shared
//! token synchronously inside its own step, so no test depends on a thread race. The
//! deadline tests only assert prefix soundness; a result that happens to complete is
//! accepted as the whole prefix.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendId, BackendSnapshot, BatchSignals,
    CancelToken, Capabilities, DebugEventKind, InputSequence, MachineInspection, RunControl,
    StepOutcome, StopBudget, StopCondition, StopKind, StopPhase, StopReason, StopSet, StopValue,
    VerificationPolicy,
};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};

type TestResult = Result<(), Box<dyn Error>>;

/// One synthetic control machine: one tick per step, optional synchronous cancel.
struct ControlBackend {
    descriptor: BackendDescriptor,
    tick: Tick,
    halted: bool,
    token: Option<CancelToken>,
    cancel_after: u64,
    steps: u64,
    first_step: Option<Arc<(Mutex<bool>, Condvar)>>,
}

impl ControlBackend {
    fn new(token: Option<CancelToken>, cancel_after: u64) -> Self {
        Self {
            descriptor: BackendDescriptor::new(
                BackendId::new("chronofork.test.control"),
                "Batch cancellation fixture (not an emulator)",
                "0",
                Capabilities::none(),
                true,
            ),
            tick: Tick::ZERO,
            halted: false,
            token,
            cancel_after,
            steps: 0,
            first_step: None,
        }
    }

    /// Installs a handshake signalled once the first step has been applied.
    fn with_first_step_handshake(mut self, handshake: Arc<(Mutex<bool>, Condvar)>) -> Self {
        self.first_step = Some(handshake);
        self
    }

    fn digest(&self) -> StateHash {
        hash_bytes(&self.tick.raw().to_le_bytes())
    }
}

impl Backend for ControlBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        Ok(self.digest())
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        if self.halted {
            return Err(BackendError::Halted);
        }
        self.tick = self.tick.next()?;
        self.steps += 1;
        if let Some(handshake) = &self.first_step {
            let (lock, ready) = &**handshake;
            let mut entered = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            *entered = true;
            ready.notify_all();
        }
        // Deterministic trigger: the fixture itself requests cancellation at step k.
        if self.steps == self.cancel_after {
            if let Some(token) = &self.token {
                token.cancel();
            }
        }
        Ok(StepOutcome {
            tick: self.tick,
            halted: self.halted,
            verified: false,
        })
    }

    fn batch_signals(&self) -> BatchSignals {
        BatchSignals {
            tick: Some(self.tick),
            pc: None,
            frame: None,
            halted: self.halted,
        }
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

/// Minimal backend that deliberately relies on the provided `batch_signals()` default.
///
/// It starts at a nonzero tick and advances ten cycles per step, so a relative cycle
/// budget is only correct if the loop observes the real starting tick.
struct DefaultSignalsBackend {
    descriptor: BackendDescriptor,
    tick: Tick,
    halted: bool,
    hash_calls: Cell<usize>,
    steps: u64,
    halt_after: Option<u64>,
    cancel_on_halt: Option<CancelToken>,
    wait_until: Option<Instant>,
}

impl DefaultSignalsBackend {
    fn new(tick: Tick, halted: bool) -> Self {
        Self {
            descriptor: BackendDescriptor::new(
                BackendId::new("chronofork.test.default-signals"),
                "Default-hook batch fixture (not an emulator)",
                "0",
                Capabilities::none(),
                true,
            ),
            tick,
            halted,
            hash_calls: Cell::new(0),
            steps: 0,
            halt_after: None,
            cancel_on_halt: None,
            wait_until: None,
        }
    }

    /// Halts on step `n` (1-based), optionally cancelling the run in the same step.
    fn halting_after(mut self, n: u64, cancel_on_halt: Option<CancelToken>) -> Self {
        self.halt_after = Some(n);
        self.cancel_on_halt = cancel_on_halt;
        self
    }

    /// Makes the step wait until the host deadline, so the post-step boundary observes it.
    fn waiting_until(mut self, at: Instant) -> Self {
        self.wait_until = Some(at);
        self
    }
}

impl Backend for DefaultSignalsBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        self.hash_calls.set(self.hash_calls.get() + 1);
        Ok(hash_bytes(&self.tick.raw().to_le_bytes()))
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        if self.halted {
            return Err(BackendError::Halted);
        }
        let advanced = self
            .tick
            .raw()
            .checked_add(10)
            .ok_or(BackendError::TickOverflow)?;
        self.tick = Tick::from_raw(advanced);
        self.steps += 1;
        if self.halt_after == Some(self.steps) {
            self.halted = true;
            if let Some(token) = &self.cancel_on_halt {
                token.cancel();
            }
        }
        if let Some(at) = self.wait_until {
            while Instant::now() < at {
                thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(StepOutcome {
            tick: self.tick,
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

fn run_default(
    backend: &mut DefaultSignalsBackend,
    budget: StopBudget,
    control: &RunControl,
) -> Result<chronofork_backend_api::BatchOutcome, BackendError> {
    backend.run_batch(
        &InputSequence::neutral(),
        budget,
        &StopSet::default(),
        VerificationPolicy::None,
        control,
    )
}

#[test]
fn default_batch_cycle_budget_is_relative_to_the_real_start_tick() -> TestResult {
    let mut backend = DefaultSignalsBackend::new(Tick::from_raw(100), false);
    let outcome = run_default(&mut backend, StopBudget::Cycles(15), &RunControl::new())?;
    assert_eq!(outcome.performed, 2, "two ten-cycle steps fit in 15 cycles");
    assert_eq!(outcome.cycles, 20);
    assert_eq!(outcome.tick, Tick::from_raw(120));
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(
        backend.hash_calls.get(),
        0,
        "ordinary batch stepping stays hash-free"
    );
    Ok(())
}

#[test]
fn default_empty_batch_reports_the_real_current_tick() -> TestResult {
    let mut backend = DefaultSignalsBackend::new(Tick::from_raw(100), false);
    let outcome = run_default(&mut backend, StopBudget::Steps(0), &RunControl::new())?;
    assert_eq!(outcome.performed, 0);
    assert_eq!(outcome.cycles, 0);
    assert_eq!(outcome.tick, Tick::from_raw(100));
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    Ok(())
}

#[test]
fn default_pre_cancelled_and_deadline_runs_report_the_real_tick() -> TestResult {
    let mut cancelled = DefaultSignalsBackend::new(Tick::from_raw(100), false);
    let token = CancelToken::new();
    token.cancel();
    let outcome = run_default(
        &mut cancelled,
        StopBudget::Steps(10),
        &RunControl::with_token(token),
    )?;
    assert_eq!(outcome.performed, 0);
    assert_eq!(outcome.tick, Tick::from_raw(100));
    assert_eq!(outcome.stop_reason, StopReason::Cancelled);

    let mut expired = DefaultSignalsBackend::new(Tick::from_raw(100), false);
    let outcome = run_default(
        &mut expired,
        StopBudget::Steps(10),
        &RunControl::new().with_deadline(Instant::now()),
    )?;
    assert_eq!(outcome.performed, 0);
    assert_eq!(outcome.tick, Tick::from_raw(100));
    assert_eq!(outcome.stop_reason, StopReason::Deadline);
    Ok(())
}

#[test]
fn default_batch_reports_an_already_halted_machine() -> TestResult {
    let mut backend = DefaultSignalsBackend::new(Tick::from_raw(100), true);
    let outcome = run_default(&mut backend, StopBudget::Steps(5), &RunControl::new())?;
    assert_eq!(outcome.performed, 0);
    assert_eq!(outcome.tick, Tick::from_raw(100));
    assert!(outcome.halted);
    assert_eq!(outcome.stop_reason, StopReason::Halted);
    Ok(())
}

#[test]
fn poststep_cancel_preserves_the_step_halt_metadata() -> TestResult {
    // The backend halts and synchronously cancels inside step 1; the optional hook's
    // provided default would report `halted=false`. Cancellation still wins the reason,
    // but the outcome must carry the authoritative halt and step-derived tick/cycles.
    let token = CancelToken::new();
    let mut backend = DefaultSignalsBackend::new(Tick::from_raw(100), false)
        .halting_after(1, Some(token.clone()));
    let control = RunControl::with_token(token);
    let outcome = backend.run_batch(
        &InputSequence::neutral(),
        StopBudget::Steps(5),
        &StopSet::default(),
        VerificationPolicy::None,
        &control,
    )?;
    assert_eq!(outcome.stop_reason, StopReason::Cancelled);
    assert!(outcome.halted, "a winning cancel must not erase the halt");
    assert_eq!(outcome.performed, 1);
    assert_eq!(outcome.cycles, 10);
    assert_eq!(outcome.tick, Tick::from_raw(110));
    assert!(
        backend.halted,
        "the authoritative status also reports halted"
    );
    assert_eq!(backend.hash_calls.get(), 0, "stepping stays hash-free");
    Ok(())
}

#[test]
fn poststep_deadline_preserves_the_step_halt_metadata() -> TestResult {
    // The step waits past an absolute deadline while it halts; the post-step boundary sees
    // the deadline win, and must still report the authoritative halt and step tick/cycles.
    // The 200 ms margin keeps the initial pre-step check deterministically before the
    // deadline on any normal scheduler.
    let at = Instant::now() + Duration::from_millis(200);
    let mut backend = DefaultSignalsBackend::new(Tick::from_raw(100), false)
        .halting_after(1, None)
        .waiting_until(at);
    let control = RunControl::new().with_deadline(at);
    let outcome = backend.run_batch(
        &InputSequence::neutral(),
        StopBudget::Steps(5),
        &StopSet::default(),
        VerificationPolicy::None,
        &control,
    )?;
    assert_eq!(outcome.stop_reason, StopReason::Deadline);
    assert!(outcome.halted, "the deadline must not erase the halt");
    assert_eq!(outcome.performed, 1);
    assert_eq!(outcome.cycles, 10);
    assert_eq!(outcome.tick, Tick::from_raw(110));
    assert!(backend.halted);
    assert_eq!(backend.hash_calls.get(), 0, "stepping stays hash-free");
    Ok(())
}

#[test]
fn stop_set_rejects_mismatched_kind_and_value() -> TestResult {
    let values = [
        StopValue::Pc(4),
        StopValue::Memory {
            address: 0x20,
            mask: 0xFFFF,
            expected: 1,
        },
        StopValue::Event(DebugEventKind::BusWrite {
            address: 0,
            value: 0,
            width: 1,
        }),
        StopValue::Tick(Tick::from_raw(4)),
        StopValue::Frame(4),
    ];
    let kinds = [
        StopKind::Pc,
        StopKind::Memory,
        StopKind::Event,
        StopKind::Tick,
        StopKind::Frame,
    ];
    for kind in kinds {
        for value in values {
            let condition = StopCondition {
                kind,
                value,
                phase: StopPhase::PostStep,
            };
            let result = StopSet::new(vec![condition]);
            if kind == value.kind() {
                assert!(
                    result.is_ok(),
                    "matching {kind:?}/{value:?} must be accepted"
                );
            } else {
                assert!(
                    result.is_err(),
                    "mismatched {kind:?}/{value:?} must be rejected before execution"
                );
            }
        }
    }

    // A consistent set ranks and reports the same observable: PC outranks Tick, and the
    // reported kind is the PC condition's kind.
    let set = StopSet::new(vec![
        StopCondition {
            kind: StopKind::Tick,
            value: StopValue::Tick(Tick::from_raw(5)),
            phase: StopPhase::PostStep,
        },
        StopCondition {
            kind: StopKind::Pc,
            value: StopValue::Pc(3),
            phase: StopPhase::PostStep,
        },
    ])?;
    let matched = set.select(&[0, 1]).ok_or("a condition matched")?;
    assert_eq!(matched.kind, StopKind::Pc);
    assert_eq!(matched.value, StopValue::Pc(3));
    assert_eq!(matched.request_index, 1);
    Ok(())
}

fn frames(count: u64) -> Result<InputSequence, BackendError> {
    InputSequence::new(
        (0..count)
            .map(|index| InputFrame::new(u32::try_from(index % 4).unwrap_or(0)))
            .collect(),
    )
}

fn run(
    backend: &mut ControlBackend,
    inputs: &InputSequence,
    steps: u64,
    policy: VerificationPolicy,
    control: &RunControl,
) -> Result<chronofork_backend_api::BatchOutcome, BackendError> {
    backend.run_batch(
        inputs,
        StopBudget::Steps(steps),
        &StopSet::default(),
        policy,
        control,
    )
}

#[test]
fn cancel_equals_run_of_k() -> TestResult {
    let token = CancelToken::new();
    let inputs = frames(5)?;
    let mut cancelled = ControlBackend::new(Some(token.clone()), 3);
    let control = RunControl::with_token(token.clone());
    let outcome = run(
        &mut cancelled,
        &inputs,
        5,
        VerificationPolicy::EveryStep,
        &control,
    )?;
    assert_eq!(outcome.stop_reason, StopReason::Cancelled);
    assert_eq!(outcome.performed, 3);
    assert_eq!(outcome.tick, Tick::from_raw(3));

    // The identical prefix is a plain run of k steps: same tick and canonical digest.
    let prefix = inputs.prefix(3);
    let mut plain = ControlBackend::new(None, 0);
    let reference = run(
        &mut plain,
        &prefix,
        3,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(reference.stop_reason, StopReason::Completed);
    assert_eq!(reference.performed, outcome.performed);
    assert_eq!(reference.tick, outcome.tick);
    assert_eq!(reference.state_hash, outcome.state_hash);
    assert_eq!(prefix.len(), 3, "consumed input prefix is recorded");

    // Cancellation outranks an already-expired deadline at the same boundary.
    let expired =
        RunControl::with_token(token).with_deadline(Instant::now() - Duration::from_secs(1));
    let mut both = ControlBackend::new(None, 0);
    let outcome = run(&mut both, &inputs, 5, VerificationPolicy::None, &expired)?;
    assert_eq!(outcome.stop_reason, StopReason::Cancelled);
    assert_eq!(outcome.performed, 0);
    Ok(())
}

#[test]
fn deadline_prefix_is_deterministic() -> TestResult {
    let inputs = frames(8)?;

    // An already-expired deadline stops before the first step and is the empty prefix.
    let expired = RunControl::new().with_deadline(Instant::now() - Duration::from_secs(1));
    let mut stopped = ControlBackend::new(None, 0);
    let outcome = run(&mut stopped, &inputs, 8, VerificationPolicy::None, &expired)?;
    assert_eq!(outcome.stop_reason, StopReason::Deadline);
    assert_eq!(outcome.performed, 0);
    assert_eq!(outcome.tick, Tick::ZERO);
    let mut unreached = ControlBackend::new(None, 0);
    let reference = run(
        &mut unreached,
        &InputSequence::neutral(),
        0,
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    assert_eq!(reference.performed, outcome.performed);
    assert_eq!(reference.tick, outcome.tick);

    // A far deadline does not perturb a run that completes: identical to no deadline.
    let far = RunControl::new().with_deadline(Instant::now() + Duration::from_secs(3600));
    let mut with_deadline = ControlBackend::new(None, 0);
    let mut without = ControlBackend::new(None, 0);
    let a = run(
        &mut with_deadline,
        &inputs,
        8,
        VerificationPolicy::EveryStep,
        &far,
    )?;
    let b = run(
        &mut without,
        &inputs,
        8,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(a.stop_reason, StopReason::Completed);
    assert_eq!(a.performed, 8);
    assert_eq!(a.tick, b.tick);
    assert_eq!(a.state_hash, b.state_hash);

    // If a short deadline truncates the run, its result is still the exact prefix of the
    // unbounded run. (The assertion is sound whether it truncated or completed.)
    let short = RunControl::new().with_deadline(Instant::now() + Duration::from_micros(1));
    let mut truncated = ControlBackend::new(None, 0);
    let result = run(
        &mut truncated,
        &InputSequence::neutral(),
        u64::MAX,
        VerificationPolicy::EveryStep,
        &short,
    )?;
    assert!(
        matches!(
            result.stop_reason,
            StopReason::Deadline | StopReason::Completed
        ),
        "unexpected reason {:?}",
        result.stop_reason
    );
    let prefix = InputSequence::neutral();
    let mut replay = ControlBackend::new(None, 0);
    let expected = run(
        &mut replay,
        &prefix,
        result.performed,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(expected.tick, result.tick);
    assert_eq!(expected.state_hash, result.state_hash);
    Ok(())
}

#[test]
fn cancellation_is_observable_without_a_lock() -> TestResult {
    let token = CancelToken::new();
    let control = RunControl::with_token(token.clone());
    let inputs = InputSequence::neutral();
    // Explicit first-step handshake: the cancelling thread fires only after the backend
    // reports that its first step was applied. This removes the sleep/positive-progress
    // race without touching the atomic production path.
    let handshake = Arc::new((Mutex::new(false), Condvar::new()));
    let canceller = thread::spawn({
        let token = token.clone();
        let handshake = Arc::clone(&handshake);
        move || {
            let (lock, ready) = &*handshake;
            let mut entered = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            while !*entered {
                entered = ready
                    .wait(entered)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            drop(entered);
            token.cancel();
        }
    });
    let mut backend =
        ControlBackend::new(None, 0).with_first_step_handshake(Arc::clone(&handshake));
    let outcome = backend.run_batch(
        &inputs,
        StopBudget::Steps(u64::MAX),
        &StopSet::default(),
        VerificationPolicy::None,
        &control,
    )?;
    canceller.join().map_err(|_| "cancel thread panicked")?;

    assert!(control.is_cancelled());
    assert_eq!(outcome.stop_reason, StopReason::Cancelled);
    assert!(
        outcome.performed > 0,
        "the handshake guarantees the canceller fired after the first applied step"
    );
    assert!(
        ControlBackend::new(None, 0)
            .run_batch(
                &inputs,
                StopBudget::Steps(outcome.performed),
                &StopSet::default(),
                VerificationPolicy::None,
                &RunControl::new(),
            )?
            .tick
            == outcome.tick,
        "the reported performed count is the deterministic prefix"
    );
    Ok(())
}
