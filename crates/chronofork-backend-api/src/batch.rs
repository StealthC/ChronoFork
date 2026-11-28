//! Deterministic batch execution and the stop-condition grammar (ADR-0015 §3–§4).
//!
//! [`Backend::run_batch`] is the **single** batch primitive: the default here is the
//! reference semantics and is composed from [`Backend::step`]/[`Backend::step_verified`].
//! A backend may override it only to avoid the per-step trait call; overrides must keep
//! this grammar, precedence and result shape.
//!
//! The loop reads emulated signals only: the tick and halt flag come from the step
//! outcome, memory is a side-effect-free debugger read, and the instruction pointer and
//! frame index come from [`Backend::batch_signals`]. Host time never enters a decision.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use chronofork_core::{InputFrame, StateHash, Tick};

use crate::debug::{DebugEventKind, DebugEventLog};
use crate::error::BackendError;
use crate::traits::Backend;

/// Maximum number of input frames one batch may carry.
pub const MAX_INPUT_FRAMES: usize = 4096;
/// Maximum number of stop conditions in one [`StopSet`].
pub const MAX_STOP_CONDITIONS: usize = 4;

/// A lock-free cooperative cancellation flag shared between a runner and its caller.
///
/// `Clone` shares the same atomic flag, so any thread holding a clone can cancel without
/// taking a session, manager or channel lock. The batch loop reads it with an atomic load
/// at step boundaries only.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// Creates a token that is not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. Lock-free and valid from any thread holding a clone.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Cooperative cancellation and an optional absolute host deadline for one batch run.
///
/// The deadline is host policy, never canonical state: it chooses how many steps run and
/// is never replayed. It is immutable after construction (`with_deadline` is a builder
/// step performed before sharing), so a clone cannot disagree with the original about
/// when the run must stop.
#[derive(Clone, Debug, Default)]
pub struct RunControl {
    token: CancelToken,
    deadline: Option<Instant>,
}

impl RunControl {
    /// Not cancelled, no deadline.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a control around an existing token (so the caller keeps a cancel handle).
    #[must_use]
    pub fn with_token(token: CancelToken) -> Self {
        Self {
            token,
            deadline: None,
        }
    }

    /// Sets the absolute host deadline **before** the control is shared.
    #[must_use]
    pub fn with_deadline(mut self, at: Instant) -> Self {
        self.deadline = Some(at);
        self
    }

    /// Returns the shared cancellation token.
    #[must_use]
    pub fn token(&self) -> &CancelToken {
        &self.token
    }

    /// Requests cancellation. Lock-free; valid from any thread holding a clone.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Returns whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Returns the absolute host deadline, when one was configured.
    #[must_use]
    pub const fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Returns the boundary stop reason when cancellation or the deadline already fired.
    ///
    /// Cancellation outranks an expired deadline at the same boundary. This is the only
    /// place host time is read, and it is read at step boundaries only.
    fn fired(&self) -> Option<StopReason> {
        if self.is_cancelled() {
            Some(StopReason::Cancelled)
        } else if self.deadline().is_some_and(|at| Instant::now() >= at) {
            Some(StopReason::Deadline)
        } else {
            None
        }
    }
}

/// Which observable a stop condition tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopKind {
    /// Instruction pointer.
    Pc,
    /// Big-endian 16-bit word at an address, compared through a mask.
    Memory,
    /// A debug event kind produced by the step.
    Event,
    /// Emulated tick.
    Tick,
    /// Frame index.
    Frame,
}

/// When a stop condition is tested.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StopPhase {
    /// Before the next step, without consuming it.
    PreStep,
    /// After a step.
    #[default]
    PostStep,
}

/// The value a stop condition compares against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopValue {
    /// Instruction pointer.
    Pc(u32),
    /// Big-endian word at `address`, compared as `(word & mask) == (expected & mask)`.
    Memory {
        /// Address of the word.
        address: u32,
        /// Bit mask applied to both sides.
        mask: u16,
        /// Expected masked word.
        expected: u16,
    },
    /// A typed debug event kind.
    Event(DebugEventKind),
    /// Absolute emulated tick.
    Tick(Tick),
    /// Frame index.
    Frame(u64),
}

impl StopValue {
    /// Returns the observable this value tests.
    ///
    /// A condition's `kind` must equal this value's kind; [`StopSet::new`] enforces the
    /// invariant so matching, precedence and reporting cannot disagree.
    #[must_use]
    pub const fn kind(self) -> StopKind {
        match self {
            Self::Pc(_) => StopKind::Pc,
            Self::Memory { .. } => StopKind::Memory,
            Self::Event(_) => StopKind::Event,
            Self::Tick(_) => StopKind::Tick,
            Self::Frame(_) => StopKind::Frame,
        }
    }
}

/// One typed stop condition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StopCondition {
    /// Observable tested.
    pub kind: StopKind,
    /// Value compared against.
    pub value: StopValue,
    /// Boundary at which the condition is tested.
    pub phase: StopPhase,
}

/// The matched stop condition, named for the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatchedStop {
    /// Observable that matched.
    pub kind: StopKind,
    /// Value that matched.
    pub value: StopValue,
    /// Position in the request order of the [`StopSet`].
    pub request_index: u8,
}

/// Why a batch ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// Cancellation was requested through the run's [`CancelToken`] (W2.3c).
    Cancelled,
    /// The absolute host deadline carried by [`RunControl`] expired (W2.3c).
    Deadline,
    /// The machine halted.
    Halted,
    /// A stop condition matched.
    StopCondition,
    /// The budget was consumed, or the batch was empty.
    Completed,
}

/// How much execution one batch may perform.
///
/// The budget is a first-class parameter of the one batch primitive: a step and a cycle
/// budget are two readings of the same question and are never converted into each other.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopBudget {
    /// No step or cycle ceiling; stop only on a signal or condition.
    Unbounded,
    /// Stop after at most this many steps.
    Steps(u64),
    /// Stop at the first step boundary at which consumed master cycles reach this value.
    Cycles(u64),
}

impl StopBudget {
    /// Returns true once `performed` steps and `cycles` consumed satisfy the budget.
    #[must_use]
    pub const fn is_exhausted(self, performed: u64, cycles: u64) -> bool {
        match self {
            Self::Unbounded => false,
            Self::Steps(steps) => performed >= steps,
            Self::Cycles(limit) => cycles >= limit,
        }
    }
}

/// A bounded ordered set of stop conditions (at most [`MAX_STOP_CONDITIONS`]).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StopSet {
    conditions: Vec<StopCondition>,
}

impl StopSet {
    /// Creates a stop set, rejecting more than [`MAX_STOP_CONDITIONS`] conditions and any
    /// condition whose `kind` disagrees with its value.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::InvalidConfiguration`] instead of silently dropping the
    /// tail of the request or accepting a contradictory kind/value pair.
    pub fn new(conditions: Vec<StopCondition>) -> Result<Self, BackendError> {
        if conditions.len() > MAX_STOP_CONDITIONS {
            return Err(BackendError::InvalidConfiguration(format!(
                "a stop set holds at most {MAX_STOP_CONDITIONS} conditions, got {}",
                conditions.len()
            )));
        }
        for condition in &conditions {
            if condition.kind != condition.value.kind() {
                return Err(BackendError::InvalidConfiguration(format!(
                    "stop condition kind {:?} does not match its {:?} value",
                    condition.kind,
                    condition.value.kind()
                )));
            }
        }
        Ok(Self { conditions })
    }

    /// Returns the conditions in request order.
    #[must_use]
    pub fn conditions(&self) -> &[StopCondition] {
        &self.conditions
    }

    /// Returns true when the set is empty (a batch then stops only on its budget/signals).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    /// Selects the winning condition among the request indexes that matched at one
    /// boundary, using the documented precedence.
    ///
    /// Precedence inside the condition set: `pc > memory > event > frame > tick`; for the
    /// same kind the lowest value wins (`pc` value, memory address, frame, tick; an event
    /// has no ordering value), and ties then use the request position. The caller passes
    /// the indexes of the conditions that matched, so this selection is pure and testable.
    #[must_use]
    pub fn select(&self, matched: &[usize]) -> Option<MatchedStop> {
        matched
            .iter()
            .filter_map(|&index| {
                let condition = self.conditions.get(index)?;
                Some((
                    condition_rank(condition.kind),
                    condition_order(*condition),
                    index,
                    condition,
                ))
            })
            .min_by_key(|&(rank, order, index, _)| (rank, order, index))
            .map(|(_, _, index, condition)| MatchedStop {
                kind: condition.kind,
                value: condition.value,
                request_index: u8::try_from(index).unwrap_or(u8::MAX),
            })
    }
}

const fn condition_rank(kind: StopKind) -> u8 {
    match kind {
        StopKind::Pc => 0,
        StopKind::Memory => 1,
        StopKind::Event => 2,
        StopKind::Frame => 3,
        StopKind::Tick => 4,
    }
}

fn condition_order(condition: StopCondition) -> u64 {
    match condition.value {
        StopValue::Pc(pc) => u64::from(pc),
        StopValue::Memory { address, .. } => u64::from(address),
        StopValue::Event(_) => 0,
        StopValue::Frame(frame) => frame,
        StopValue::Tick(tick) => tick.raw(),
    }
}

/// Cheap, hash-free observations the generic batch loop needs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BatchSignals {
    /// Current absolute tick, when the backend can report it cheaply.
    pub tick: Option<Tick>,
    /// Current instruction pointer, when the backend has one.
    pub pc: Option<u32>,
    /// Current frame index, when the backend has one.
    pub frame: Option<u64>,
    /// Whether the machine is halted.
    pub halted: bool,
}

/// A bounded sequence of canonical input frames for one batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputSequence {
    frames: Vec<InputFrame>,
}

impl InputSequence {
    /// Creates a sequence of at most [`MAX_INPUT_FRAMES`] frames.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Limit`] for a longer sequence, before any step runs.
    pub fn new(frames: Vec<InputFrame>) -> Result<Self, BackendError> {
        if frames.len() > MAX_INPUT_FRAMES {
            return Err(BackendError::Limit {
                field: "input_frames",
                limit: MAX_INPUT_FRAMES as u64,
                requested: frames.len() as u64,
            });
        }
        Ok(Self { frames })
    }

    /// Returns a neutral sequence (every step gets [`InputFrame::NEUTRAL`]).
    #[must_use]
    pub fn neutral() -> Self {
        Self { frames: Vec::new() }
    }

    /// Returns the number of explicit frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Returns true when every step uses the neutral frame.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Returns the frame for 0-based `step`, padding with neutral past the end.
    #[must_use]
    pub fn frame(&self, step: u64) -> InputFrame {
        usize::try_from(step)
            .ok()
            .and_then(|index| self.frames.get(index).copied())
            .unwrap_or(InputFrame::NEUTRAL)
    }

    /// Returns the first `performed` frames as a new sequence (used to record a run).
    ///
    /// Returns the stored prefix for a run of `performed` steps.
    ///
    /// The [`MAX_INPUT_FRAMES`] bound applies to **stored explicit frames**, never to the
    /// performed step count: a 5000-step run over an empty sequence records zero frames
    /// and the neutral padding stays implicit. The result therefore stores
    /// `min(performed, self.frames.len())` frames and cannot exceed the input cap.
    #[must_use]
    pub fn prefix(&self, performed: u64) -> Self {
        let count = usize::try_from(performed)
            .unwrap_or(usize::MAX)
            .min(self.frames.len());
        Self {
            frames: self.frames[..count].to_vec(),
        }
    }
}

/// Whether and when a batch computes canonical digests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VerificationPolicy {
    /// Ordinary steps; no digest is computed.
    #[default]
    None,
    /// Ordinary steps; the caller asks [`Backend::state_hash`] after the run.
    OnDemand,
    /// One digest per step (`step_verified`).
    EveryStep,
    /// One digest at 1-based steps `1, n+1, 2n+1, ...` (n must be nonzero).
    EveryN(u64),
    /// One digest at the step whose resulting tick equals the value.
    AtTick(Tick),
}

/// The result of one batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatchOutcome {
    /// Steps performed (instructions, backend quanta).
    pub performed: u64,
    /// Consumed master cycles.
    pub cycles: u64,
    /// Tick after the last performed step (or the starting tick for an empty batch).
    pub tick: Tick,
    /// Whether the machine halted.
    pub halted: bool,
    /// Why the batch ended.
    pub stop_reason: StopReason,
    /// The matched condition when `stop_reason` is [`StopReason::StopCondition`].
    pub stop_condition: Option<MatchedStop>,
    /// Digest when the policy produced one; `None` for `None`/`OnDemand`.
    pub state_hash: Option<StateHash>,
}

/// Reference implementation of the single batch primitive.
///
/// Called by the provided [`Backend::run_batch`]; a backend override may replace it only
/// to avoid the per-step trait call while keeping this grammar and precedence.
pub(crate) fn run_batch_default<B: Backend + ?Sized>(
    backend: &mut B,
    inputs: &InputSequence,
    budget: StopBudget,
    stop: &StopSet,
    policy: VerificationPolicy,
    control: &RunControl,
) -> Result<BatchOutcome, BackendError> {
    if let StopBudget::Steps(steps) = budget {
        let requested = inputs.len() as u64;
        if requested > steps {
            return Err(BackendError::Limit {
                field: "input_frames",
                limit: steps,
                requested,
            });
        }
    }
    if let VerificationPolicy::EveryN(n) = policy {
        if n == 0 {
            return Err(BackendError::InvalidConfiguration(
                "`EveryN` verification requires a nonzero step interval".to_owned(),
            ));
        }
    }

    // Canonical time and the halt flag come from a real observation. `batch_signals()` is
    // an optional cheap hook whose provided default does not know them, so its missing
    // tick is filled from the hash-free `status()` fallback rather than invented as zero;
    // a default halt flag is corrected the same way. Stepping stays hash-free.
    let initial = backend.status()?;
    let mut signals = backend.batch_signals();
    signals.tick = Some(initial.tick);
    signals.halted = initial.halted;
    let mut tick = initial.tick;
    let mut performed = 0_u64;
    let mut cycles = 0_u64;
    let mut produced_hash: Option<StateHash> = None;

    // Same-boundary priority: cancelled > deadline > halted > stop_condition > completed.
    if let Some(reason) = control.fired() {
        return Ok(BatchOutcome {
            performed,
            cycles,
            tick,
            halted: signals.halted,
            stop_reason: reason,
            stop_condition: None,
            state_hash: produced_hash,
        });
    }

    if signals.halted {
        return Ok(BatchOutcome {
            performed,
            cycles,
            tick,
            halted: true,
            stop_reason: StopReason::Halted,
            stop_condition: None,
            state_hash: None,
        });
    }

    loop {
        // Cancellation and the deadline are checked at this step boundary, before any
        // pre-step condition, budget outcome or step. Host time is read only here.
        if let Some(reason) = control.fired() {
            return Ok(BatchOutcome {
                performed,
                cycles,
                tick,
                halted: signals.halted,
                stop_reason: reason,
                stop_condition: None,
                state_hash: produced_hash,
            });
        }

        // A pre-step condition outranks completion at the same boundary, so it is tested
        // before the exhausted-budget outcome. It never consumes the next step.
        if let Some(matched) =
            matching_stop(backend, stop, StopPhase::PreStep, tick, &signals, None)?
        {
            return Ok(BatchOutcome {
                performed,
                cycles,
                tick,
                halted: signals.halted,
                stop_reason: StopReason::StopCondition,
                stop_condition: Some(matched),
                state_hash: produced_hash,
            });
        }

        if budget.is_exhausted(performed, cycles) {
            return Ok(BatchOutcome {
                performed,
                cycles,
                tick,
                halted: signals.halted,
                stop_reason: StopReason::Completed,
                stop_condition: None,
                state_hash: produced_hash,
            });
        }

        let event_mark = event_mark(backend);
        let step_index = performed;
        let verify_every_step = matches!(policy, VerificationPolicy::EveryStep)
            || matches!(policy, VerificationPolicy::EveryN(n) if step_index % n == 0);
        let started_tick = tick;
        let (outcome_tick, outcome_halted) = if verify_every_step {
            let verified = backend.step_verified(inputs.frame(step_index))?;
            produced_hash = Some(verified.state_hash);
            (verified.tick, verified.halted)
        } else {
            let outcome = backend.step(inputs.frame(step_index))?;
            if let VerificationPolicy::AtTick(target) = policy {
                if outcome.tick == target {
                    produced_hash = Some(backend.state_hash()?);
                }
            }
            (outcome.tick, outcome.halted)
        };
        performed = performed.checked_add(1).ok_or(BackendError::TickOverflow)?;
        cycles = cycles
            .checked_add(outcome_tick.raw().saturating_sub(started_tick.raw()))
            .ok_or(BackendError::TickOverflow)?;
        tick = outcome_tick;
        // Refresh the optional hook, then restore the authoritative step-derived values:
        // a provided `halted=false`/missing tick must not erase the state metadata of a
        // step that just halted. The control *reason* priority below still applies
        // (cancelled > deadline > halted).
        signals = backend.batch_signals();
        signals.tick = Some(tick);
        signals.halted = outcome_halted;

        // Same-boundary priority again: cancellation/deadline outrank the halt signal,
        // which outranks a matching condition and completion.
        if let Some(reason) = control.fired() {
            return Ok(BatchOutcome {
                performed,
                cycles,
                tick,
                halted: signals.halted,
                stop_reason: reason,
                stop_condition: None,
                state_hash: produced_hash,
            });
        }

        if outcome_halted {
            return Ok(BatchOutcome {
                performed,
                cycles,
                tick,
                halted: true,
                stop_reason: StopReason::Halted,
                stop_condition: None,
                state_hash: produced_hash,
            });
        }

        if let Some(matched) = matching_stop(
            backend,
            stop,
            StopPhase::PostStep,
            tick,
            &signals,
            event_mark,
        )? {
            return Ok(BatchOutcome {
                performed,
                cycles,
                tick,
                halted: signals.halted,
                stop_reason: StopReason::StopCondition,
                stop_condition: Some(matched),
                state_hash: produced_hash,
            });
        }
    }
}

/// Returns the conditions that match at `phase`, selected by documented precedence.
fn matching_stop<B: Backend + ?Sized>(
    backend: &mut B,
    stop: &StopSet,
    phase: StopPhase,
    tick: Tick,
    signals: &BatchSignals,
    event_mark: Option<u64>,
) -> Result<Option<MatchedStop>, BackendError> {
    if stop.is_empty() {
        return Ok(None);
    }
    let mut matched = Vec::new();
    for (index, condition) in stop.conditions().iter().enumerate() {
        if condition.phase != phase {
            continue;
        }
        let hit = match condition.value {
            StopValue::Pc(pc) => signals.pc == Some(pc),
            StopValue::Memory {
                address,
                mask,
                expected,
            } => {
                let bytes = backend.read_memory(u64::from(address), 2)?;
                let high = u16::from(bytes.first().copied().unwrap_or(0));
                let low = u16::from(bytes.get(1).copied().unwrap_or(0));
                let word = (high << 8) | low;
                word & mask == expected & mask
            }
            StopValue::Event(kind) => {
                // Events are produced by a step, so a pre-step event condition never
                // matches; a post-step one matches any new event of that kind.
                phase == StopPhase::PostStep
                    && event_mark.is_some_and(|mark| event_since(backend, mark, kind))
            }
            StopValue::Tick(value) => tick == value,
            StopValue::Frame(frame) => signals.frame == Some(frame),
        };
        if hit {
            matched.push(index);
        }
    }
    Ok(stop.select(&matched))
}

/// Returns true when a retained debug event after logical index `mark` has `kind`.
fn event_since<B: Backend + ?Sized>(backend: &mut B, mark: u64, kind: DebugEventKind) -> bool {
    let Some(access) = backend.debug_access() else {
        return false;
    };
    let Some(log) = access.debug_event_log() else {
        return false;
    };
    let dropped = log.dropped();
    log.entries()
        .enumerate()
        .any(|(position, event)| dropped + position as u64 >= mark && event.kind == kind)
}

/// Returns the next logical event index before a step, when the backend observes events.
fn event_mark<B: Backend + ?Sized>(backend: &mut B) -> Option<u64> {
    let access = backend.debug_access()?;
    let log: &DebugEventLog = access.debug_event_log()?;
    Some(log.dropped() + log.len() as u64)
}
