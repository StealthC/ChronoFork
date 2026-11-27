//! W2.1/W2.2 — the synthetic backend's trait digest and verified-step composition.
//!
//! Same three invariants as the Mega Drive landing test, against the synthetic
//! backend's existing single `state_body` serializer:
//!
//! - [`hash_single_source`]: explicit digest == snapshot envelope digest == the
//!   verified path;
//! - [`state_hash_is_side_effect_free`]: idempotent, does not advance the tick;
//! - [`ordinary_step_does_not_claim_verification`]: `None` + `verified == false` on the
//!   ordinary path, `true` on the verified one, identical bytes.

use std::error::Error;

use chronofork_backend_api::{Backend, BackendError, BackendFactory};
use chronofork_core::InputFrame;
use chronofork_test_backend::SyntheticBackendFactory;

type TestResult = Result<(), Box<dyn Error>>;

fn machine() -> Result<Box<dyn Backend>, BackendError> {
    SyntheticBackendFactory::default().create()
}

#[test]
fn hash_single_source() -> TestResult {
    let mut backend = machine()?;
    backend.step(InputFrame::NEUTRAL)?;

    let explicit = backend.state_hash()?;
    assert_eq!(
        explicit,
        backend.snapshot()?.state_hash(),
        "the trait digest is the snapshot envelope digest for the same state"
    );

    let mut ordinary = machine()?;
    let mut verified = machine()?;
    let ordinary_outcome = ordinary.step(InputFrame::NEUTRAL)?;
    let verified_outcome = verified.step_verified(InputFrame::NEUTRAL)?;
    // W2-2: the ordinary path carries no digest; the verified path is the only
    // per-step source and it equals the explicit canonical `state_hash()`.
    assert!(
        ordinary_outcome.state_hash.is_none(),
        "the ordinary path carries no digest"
    );
    assert_eq!(verified_outcome.state_hash, verified.state_hash()?);
    assert_eq!(
        ordinary.state_hash()?,
        verified.state_hash()?,
        "both machines are in the same canonical state"
    );
    assert_eq!(
        ordinary.snapshot()?.payload(),
        verified.snapshot()?.payload()
    );
    Ok(())
}

#[test]
fn state_hash_is_side_effect_free() -> TestResult {
    let mut backend = machine()?;
    let before = backend.inspect()?;
    let first = backend.state_hash()?;
    let second = backend.state_hash()?;
    assert_eq!(first, second);
    let after = backend.inspect()?;
    assert_eq!(after.tick, before.tick);
    assert_eq!(after.state_hash, before.state_hash);

    backend.step(InputFrame::NEUTRAL)?;
    assert_ne!(first, backend.state_hash()?);
    Ok(())
}

#[test]
fn ordinary_step_does_not_claim_verification() -> TestResult {
    let mut ordinary = machine()?;
    let mut verified = machine()?;
    let ordinary_outcome = ordinary.step(InputFrame::NEUTRAL)?;
    let verified_outcome = verified.step_verified(InputFrame::NEUTRAL)?;

    assert!(!ordinary_outcome.verified);
    assert!(verified_outcome.verified);
    assert_eq!(ordinary_outcome.tick, verified_outcome.tick);
    assert_eq!(ordinary_outcome.halted, verified_outcome.halted);
    assert!(
        ordinary_outcome.state_hash.is_none(),
        "the ordinary path computes no digest (W2-2)"
    );
    assert_eq!(verified_outcome.state_hash, verified.state_hash()?);
    Ok(())
}
