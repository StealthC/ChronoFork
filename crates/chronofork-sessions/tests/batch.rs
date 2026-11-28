//! W2.3b — deterministic batch execution, stop precedence and prefix replay.
//!
//! The tests use the synthetic backend through the session layer, so they exercise the
//! single backend batch primitive, the `Session::run_until`/`run_cycles` composition and
//! the recorded `RunRecord` prefix. No test here reads host time.

use std::error::Error;
use std::sync::Arc;

use chronofork_backend_api::{
    BatchOutcome, DebugEventKind, InputSequence, RunControl, StopCondition, StopKind, StopPhase,
    StopReason, StopSet, StopValue, VerificationPolicy,
};
use chronofork_core::{InputFrame, SessionId, Tick};
use chronofork_sessions::{Movie, MovieMeta, RunRecord, SessionError, SessionManager};
use chronofork_test_backend::SyntheticBackendFactory;

type TestResult = Result<(), Box<dyn Error>>;

fn manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn condition(kind: StopKind, value: StopValue, phase: StopPhase) -> StopCondition {
    StopCondition { kind, value, phase }
}

fn tick_stop(tick: Tick) -> StopCondition {
    condition(StopKind::Tick, StopValue::Tick(tick), StopPhase::PostStep)
}

fn pc_stop(pc: u32) -> StopCondition {
    condition(StopKind::Pc, StopValue::Pc(pc), StopPhase::PostStep)
}

fn memory_stop(address: u32) -> StopCondition {
    condition(
        StopKind::Memory,
        StopValue::Memory {
            address,
            mask: 0xFFFF,
            expected: 0,
        },
        StopPhase::PostStep,
    )
}

fn assert_same_prefix(left: &BatchOutcome, right: &BatchOutcome) {
    assert_eq!(left.performed, right.performed, "performed differs");
    assert_eq!(left.cycles, right.cycles, "cycles differ");
    assert_eq!(left.tick, right.tick, "tick differs");
    assert_eq!(left.halted, right.halted, "halted differs");
    assert_eq!(left.stop_reason, right.stop_reason, "stop reason differs");
    assert_eq!(
        left.stop_condition, right.stop_condition,
        "matched condition differs"
    );
    assert_eq!(left.state_hash, right.state_hash, "digest differs");
}

fn run(
    manager: &mut SessionManager,
    session: SessionId,
    stop: &StopSet,
    steps: u64,
    inputs: &InputSequence,
    policy: VerificationPolicy,
) -> Result<BatchOutcome, SessionError> {
    manager.run_until(session, stop, steps, inputs, policy, &RunControl::new())
}

#[test]
fn stop_condition_precedence() -> TestResult {
    // Pure selection: the documented order is pc > memory > event > frame > tick. A
    // stop set holds at most four conditions, so the five kinds are covered by two sets.
    let wide = StopSet::new(vec![
        tick_stop(Tick::from_raw(1)),
        condition(StopKind::Frame, StopValue::Frame(2), StopPhase::PostStep),
        condition(
            StopKind::Event,
            StopValue::Event(DebugEventKind::ControllerRead { port: 1, value: 0 }),
            StopPhase::PostStep,
        ),
        memory_stop(0x20),
    ])?;
    let kind = |indexes: &[usize]| wide.select(indexes).map(|matched| matched.kind);
    assert_eq!(kind(&[0, 1, 2, 3]), Some(StopKind::Memory));
    assert_eq!(kind(&[0, 1, 2]), Some(StopKind::Event));
    assert_eq!(kind(&[0, 1]), Some(StopKind::Frame));
    assert_eq!(kind(&[0]), Some(StopKind::Tick));

    let pc_over_memory = StopSet::new(vec![memory_stop(0x20), pc_stop(1)])?;
    assert_eq!(
        pc_over_memory.select(&[0, 1]).map(|matched| matched.kind),
        Some(StopKind::Pc)
    );

    // Same kind: the lowest value wins, and a true tie uses the request position.
    let memory = StopSet::new(vec![
        memory_stop(0x20),
        memory_stop(0x10),
        memory_stop(0x10),
    ])?;
    let first = memory.select(&[0, 1]).ok_or("a match must be selected")?;
    assert_eq!(
        first.value,
        StopValue::Memory {
            address: 0x10,
            mask: 0xFFFF,
            expected: 0
        }
    );
    assert_eq!(first.request_index, 1);
    let tie = memory.select(&[1, 2]).ok_or("a match must be selected")?;
    assert_eq!(tie.request_index, 1);

    // Real loop: after one synthetic step (pc = 1, tick = 1) Pc outranks Tick.
    let mut manager = manager();
    let session = manager.create_session()?;
    let stop = StopSet::new(vec![tick_stop(Tick::from_raw(1)), pc_stop(1)])?;
    let inputs = InputSequence::neutral();
    let outcome = run(
        &mut manager,
        session,
        &stop,
        10,
        &inputs,
        VerificationPolicy::None,
    )?;
    assert_eq!(outcome.performed, 1);
    assert_eq!(outcome.stop_reason, StopReason::StopCondition);
    assert_eq!(
        outcome.stop_condition.map(|matched| matched.kind),
        Some(StopKind::Pc)
    );

    // With only Tick left, a new session stops on it at the same boundary.
    let session = manager.create_session()?;
    let stop = StopSet::new(vec![tick_stop(Tick::from_raw(1))])?;
    let outcome = run(
        &mut manager,
        session,
        &stop,
        10,
        &inputs,
        VerificationPolicy::None,
    )?;
    assert_eq!(outcome.performed, 1);
    assert_eq!(
        outcome.stop_condition.map(|matched| matched.kind),
        Some(StopKind::Tick)
    );

    // A pre-step condition is tested on the current state and consumes no step.
    let session = manager.create_session()?;
    let stop = StopSet::new(vec![condition(
        StopKind::Tick,
        StopValue::Tick(Tick::ZERO),
        StopPhase::PreStep,
    )])?;
    let outcome = run(
        &mut manager,
        session,
        &stop,
        10,
        &inputs,
        VerificationPolicy::None,
    )?;
    assert_eq!(outcome.performed, 0);
    assert_eq!(outcome.tick, Tick::ZERO);
    assert_eq!(
        outcome.stop_condition.map(|matched| matched.kind),
        Some(StopKind::Tick)
    );
    assert_eq!(
        outcome.stop_condition.map(|matched| matched.request_index),
        Some(0)
    );

    // A matching pre-step condition outranks an exhausted zero-step budget at the same
    // boundary (`stop_condition > completed`).
    let session = manager.create_session()?;
    let stop = StopSet::new(vec![condition(
        StopKind::Tick,
        StopValue::Tick(Tick::ZERO),
        StopPhase::PreStep,
    )])?;
    let outcome = run(
        &mut manager,
        session,
        &stop,
        0,
        &inputs,
        VerificationPolicy::None,
    )?;
    assert_eq!(outcome.performed, 0);
    assert_eq!(outcome.stop_reason, StopReason::StopCondition);
    Ok(())
}

#[test]
fn batch_prefix_is_deterministic() -> TestResult {
    let inputs = InputSequence::new(vec![
        InputFrame::new(1),
        InputFrame::new(2),
        InputFrame::new(3),
    ])?;
    let stop = StopSet::default();

    let mut first = manager();
    let first_session = first.create_session()?;
    let first_outcome = run(
        &mut first,
        first_session,
        &stop,
        3,
        &inputs,
        VerificationPolicy::None,
    )?;
    assert_eq!(first_outcome.performed, 3);
    assert_eq!(first_outcome.stop_reason, StopReason::Completed);
    assert!(first_outcome.state_hash.is_none(), "ordinary batch hashes");

    let mut second = manager();
    let second_session = second.create_session()?;
    let second_outcome = run(
        &mut second,
        second_session,
        &stop,
        3,
        &inputs,
        VerificationPolicy::None,
    )?;
    assert_same_prefix(&first_outcome, &second_outcome);

    // The same prefix replayed one ordinary step at a time is the same state.
    let mut sequential = manager();
    let sequential_session = sequential.create_session()?;
    for index in 0..first_outcome.performed {
        sequential.step(sequential_session, inputs.frame(index))?;
    }
    assert_eq!(
        sequential.inspect(sequential_session)?.machine.tick,
        first_outcome.tick
    );
    assert_eq!(
        sequential.state_hash(sequential_session)?,
        first.state_hash(first_session)?
    );

    // A cycle budget uses the same one primitive and reports consumed cycles.
    let session = first.create_session()?;
    let cycles = first.run_cycles(
        session,
        1,
        &stop,
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    assert_eq!(cycles.performed, 1);
    assert_eq!(cycles.cycles, 1);
    assert_eq!(cycles.stop_reason, StopReason::Completed);
    Ok(())
}

#[test]
fn replay_batch_equivalence() -> TestResult {
    let frames: Vec<InputFrame> = (0..6).map(InputFrame::new).collect();
    let inputs = InputSequence::new(frames.clone())?;
    let stop = StopSet::default();

    // Record a v1 movie through the unchanged movie path, then replay/verify it.
    let mut manager = manager();
    let source = manager.create_session()?;
    let meta = MovieMeta::new(1, None, None)?;
    let movie = Movie::record(&mut manager, source, meta, &frames)?;
    let report = movie.verify(&mut manager)?;

    // The batch over the same inputs reaches the same final tick and digest.
    let batch_session = manager.create_session()?;
    let batch = manager.run_until(
        batch_session,
        &stop,
        frames.len() as u64,
        &inputs,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(batch.performed, frames.len() as u64);
    assert_eq!(batch.tick, report.final_tick);
    assert_eq!(batch.state_hash, Some(report.final_state_hash));

    // A recorded prefix replays to exactly the same outcome.
    let record = RunRecord::from_outcome(&batch, &inputs);
    assert_eq!(record.performed(), batch.performed);
    let replay_session = manager.create_session()?;
    let replayed = record.replay(
        &mut manager,
        replay_session,
        &stop,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(replayed, batch);

    // A shorter batch is the recorded movie prefix: step k-1 hash and tick agree.
    let prefix_inputs = inputs.prefix(3);
    let prefix_session = manager.create_session()?;
    let prefix = manager.run_until(
        prefix_session,
        &stop,
        3,
        &prefix_inputs,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(prefix.tick, movie.steps()[2].outcome_tick());
    assert_eq!(
        prefix.state_hash,
        Some(movie.steps()[2].state_hash()),
        "batch prefix digest matches the recorded movie step"
    );
    Ok(())
}

#[test]
fn long_neutral_prefix_records_without_materializing_padding() -> TestResult {
    let mut manager = manager();
    let neutral = InputSequence::neutral();
    let source = manager.create_session()?;
    let outcome = manager.run_until(
        source,
        &StopSet::default(),
        5000,
        &neutral,
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    assert_eq!(outcome.performed, 5000);
    assert_eq!(outcome.stop_reason, StopReason::Completed);

    // The record keeps the full performed count and stores no neutral frames: the cap is
    // on explicit input frames, not on execution work.
    let record = RunRecord::from_outcome(&outcome, &neutral);
    assert_eq!(record.performed(), 5000);
    assert!(record.inputs().is_empty(), "padding stays implicit");

    let replay_session = manager.create_session()?;
    let replayed = record.replay(
        &mut manager,
        replay_session,
        &StopSet::default(),
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    assert_eq!(replayed, outcome);

    // A short explicit sequence also records only its explicit frames, not `performed`
    // neutral copies, and still replays identically.
    let short = InputSequence::new(vec![
        InputFrame::new(3),
        InputFrame::new(1),
        InputFrame::new(4),
    ])?;
    let short_session = manager.create_session()?;
    let short_outcome = manager.run_until(
        short_session,
        &StopSet::default(),
        5000,
        &short,
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    let short_record = RunRecord::from_outcome(&short_outcome, &short);
    assert_eq!(short_record.performed(), 5000);
    assert_eq!(short_record.inputs().len(), 3);
    let short_replay = manager.create_session()?;
    let replayed_short = short_record.replay(
        &mut manager,
        short_replay,
        &StopSet::default(),
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    assert_eq!(replayed_short, short_outcome);

    // The explicit-frame bound itself is unchanged and still rejects before execution.
    let over_cap = vec![InputFrame::NEUTRAL; 4097];
    assert!(matches!(
        InputSequence::new(over_cap),
        Err(chronofork_backend_api::BackendError::Limit {
            field: "input_frames",
            ..
        })
    ));
    Ok(())
}
