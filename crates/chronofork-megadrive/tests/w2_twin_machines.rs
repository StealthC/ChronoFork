//! W2.2 — twin machines on the ordinary and verified paths agree.
//!
//! The ordinary path is digest-free from W2-2, so the digest comparison asks the
//! canonical `state_hash()` explicitly for the ordinary machine while the verified
//! machine supplies it from its one digest computation per step. The window runs
//! until the observable frame index advances: the loop stops when the machine tick
//! crosses one frame's master cycles (the frame identity the VDP owns), then checks
//! the visible `scheduler_frame` register, with a 200 000-step ceiling that fails if
//! the boundary is never reached. The two machines must agree on tick, halted and
//! digest at every step, and their snapshot bytes must be identical at the end.

use std::error::Error;

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::{MegadriveBackend, MegadriveFactory, SchedulerConfig, interrupt_rom};

type TestResult = Result<(), Box<dyn Error>>;

/// Ceiling for the twin window; reaching it without a frame boundary is a failure.
const MAX_TWIN_STEPS: u64 = 200_000;

fn machine(factory: &MegadriveFactory) -> Result<MegadriveBackend, Box<dyn Error>> {
    Ok(factory.create_machine()?)
}

#[test]
fn twin_machines_normal_vs_verified() -> TestResult {
    let config = SchedulerConfig::ntsc();
    // One full frame's master cycles. The VDP advances its frame index when this
    // boundary is crossed, so it is an observable window end, not an assumed step cost.
    let frame_cycles = u64::from(config.lines_per_frame()) * config.master_cycles_per_line();
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), config)?;
    let mut ordinary = machine(&factory)?;
    let mut verified = machine(&factory)?;

    let mut steps = 0_u64;
    loop {
        if steps == MAX_TWIN_STEPS {
            return Err(format!(
                "the twin window never reached a frame boundary within {MAX_TWIN_STEPS} steps"
            )
            .into());
        }
        let ordinary_outcome = ordinary.step(InputFrame::NEUTRAL)?;
        let verified_outcome = verified.step_verified(InputFrame::NEUTRAL)?;
        steps += 1;

        assert!(
            !ordinary_outcome.verified,
            "the ordinary outcome never claims verification"
        );
        assert!(
            verified_outcome.verified,
            "the verified outcome always claims verification"
        );
        assert!(
            ordinary_outcome.state_hash.is_none(),
            "ordinary step carries no digest at step {steps}"
        );
        assert_eq!(
            ordinary_outcome.tick, verified_outcome.tick,
            "tick diverged at step {steps}"
        );
        assert_eq!(
            ordinary_outcome.halted, verified_outcome.halted,
            "halt state diverged at step {steps}"
        );
        assert_eq!(
            ordinary.state_hash()?,
            verified_outcome.state_hash,
            "digest diverged at step {steps}"
        );
        if verified_outcome.tick.raw() >= frame_cycles {
            break;
        }
    }

    // The observable frame identity really advanced, and both machines are in the
    // same canonical state: snapshot bytes and envelope digest are equal.
    let frame = verified
        .inspect()?
        .registers
        .get("scheduler_frame")
        .copied()
        .unwrap_or_default();
    assert!(
        frame >= 1,
        "the twin window stopped before the visible frame index advanced"
    );
    let ordinary_snapshot = ordinary.snapshot()?;
    let verified_snapshot = verified.snapshot()?;
    assert_eq!(
        ordinary_snapshot.payload(),
        verified_snapshot.payload(),
        "the verified path changes no canonical bytes"
    );
    assert_eq!(
        ordinary_snapshot.state_hash(),
        verified_snapshot.state_hash()
    );
    Ok(())
}
