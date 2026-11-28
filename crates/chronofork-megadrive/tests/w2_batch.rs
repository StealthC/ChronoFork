//! W2.3b — the real Mega Drive backend through the single batch primitive.
//!
//! The backend supplies cheap `batch_signals` (tick, PC, frame, halt), so `Tick`/`Pc`
//! stop conditions never fall back to an inspection. An ordinary batch computes no
//! digest; the verified policy asks the canonical source exactly where documented.

use std::error::Error;

use chronofork_backend_api::{
    Backend, InputSequence, RunControl, StopBudget, StopCondition, StopKind, StopPhase, StopReason,
    StopSet, StopValue, VerificationPolicy,
};
use chronofork_core::InputFrame;
use chronofork_megadrive::{MegadriveFactory, SchedulerConfig, interrupt_rom};

type TestResult = Result<(), Box<dyn Error>>;

fn factory() -> Result<MegadriveFactory, Box<dyn Error>> {
    Ok(MegadriveFactory::with_scheduler_config(
        interrupt_rom(3),
        SchedulerConfig::ntsc(),
    )?)
}

fn tick_stop(tick: chronofork_core::Tick) -> StopCondition {
    StopCondition {
        kind: StopKind::Tick,
        value: StopValue::Tick(tick),
        phase: StopPhase::PostStep,
    }
}

#[test]
fn batch_tick_stop_matches_sequential_step() -> TestResult {
    let factory = factory()?;
    let mut batch_machine = factory.create_machine()?;
    let mut sequential = factory.create_machine()?;

    // Discover the first step's tick with the ordinary path, then ask a fresh machine's
    // batch to stop at the same boundary.
    let first = sequential.step(InputFrame::NEUTRAL)?;
    let stop = StopSet::new(vec![tick_stop(first.tick)])?;
    let batch = batch_machine.run_batch(
        &InputSequence::neutral(),
        StopBudget::Steps(10),
        &stop,
        VerificationPolicy::None,
        &RunControl::new(),
    )?;
    assert_eq!(batch.performed, 1);
    assert_eq!(batch.stop_reason, StopReason::StopCondition);
    assert_eq!(batch.tick, first.tick);
    assert!(batch.state_hash.is_none(), "ordinary batch must not hash");
    assert_eq!(batch_machine.state_hash()?, sequential.state_hash()?);

    // The verified policy produces exactly the canonical digest for the same prefix.
    let mut verified_machine = factory.create_machine()?;
    let verified = verified_machine.run_batch(
        &InputSequence::neutral(),
        StopBudget::Steps(10),
        &stop,
        VerificationPolicy::EveryStep,
        &RunControl::new(),
    )?;
    assert_eq!(verified.performed, 1);
    assert_eq!(verified.state_hash, Some(verified_machine.state_hash()?));
    assert_eq!(
        verified.state_hash,
        Some(batch_machine.state_hash()?),
        "the verified batch and the ordinary batch end in the same canonical state"
    );
    Ok(())
}
