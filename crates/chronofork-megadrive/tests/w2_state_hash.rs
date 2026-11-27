//! W2.1/W2.2 — the Mega Drive trait digest and the verified-step composition.
//!
//! The landing adds [`Backend::state_hash`] as the single canonical digest source and
//! [`Backend::step_verified`] as its only verified composition. These tests prove the
//! three invariants on the real backend:
//!
//! - [`hash_single_source`]: the explicit digest, the snapshot envelope digest and the
//!   verified path agree;
//! - [`state_hash_is_side_effect_free`]: calling it is idempotent and never advances
//!   emulated state;
//! - [`ordinary_step_does_not_claim_verification`]: ordinary `step` carries no digest
//!   (`state_hash == None`) and reports `verified == false`, while `step_verified`
//!   reports `true` with the digest and leaves byte-identical state.
//!
//! The digest is the existing Mega Drive serializer/view construction; nothing here
//! re-derives the payload layout.

use std::error::Error;

use chronofork_backend_api::{Backend, BackendError};
use chronofork_core::InputFrame;
use chronofork_megadrive::{MegadriveBackend, MegadriveFactory, demo_rom};

type TestResult = Result<(), Box<dyn Error>>;

fn machine() -> Result<MegadriveBackend, BackendError> {
    MegadriveFactory::new(demo_rom())?.create_machine()
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

    // Two identical machines: the ordinary and verified paths must report the same
    // digest, and each must agree with its own explicit `state_hash()`.
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
    assert_eq!(
        verified_outcome.state_hash,
        verified.state_hash()?,
        "the verified outcome's digest is the canonical source"
    );
    assert_eq!(
        ordinary.state_hash()?,
        verified.state_hash()?,
        "both machines are in the same canonical state"
    );
    assert_eq!(
        ordinary.snapshot()?.payload(),
        verified.snapshot()?.payload(),
        "the verified path changes no canonical bytes"
    );
    Ok(())
}

#[test]
fn state_hash_is_side_effect_free() -> TestResult {
    let mut backend = machine()?;
    let before = backend.inspect()?;
    let first = backend.state_hash()?;
    let second = backend.state_hash()?;
    assert_eq!(first, second, "the digest is idempotent");
    let after = backend.inspect()?;
    assert_eq!(after.tick, before.tick, "hashing does not advance the tick");
    assert_eq!(after.state_hash, before.state_hash);

    backend.step(InputFrame::NEUTRAL)?;
    assert_ne!(
        first,
        backend.state_hash()?,
        "the digest changes after the machine steps"
    );
    Ok(())
}

#[test]
fn ordinary_step_does_not_claim_verification() -> TestResult {
    let mut ordinary = machine()?;
    let mut verified = machine()?;
    let ordinary_outcome = ordinary.step(InputFrame::NEUTRAL)?;
    let verified_outcome = verified.step_verified(InputFrame::NEUTRAL)?;

    assert!(
        !ordinary_outcome.verified,
        "the ordinary path never claims verification"
    );
    assert!(
        verified_outcome.verified,
        "the verified path always claims verification"
    );
    assert_eq!(ordinary_outcome.tick, verified_outcome.tick);
    assert_eq!(ordinary_outcome.halted, verified_outcome.halted);
    assert!(
        ordinary_outcome.state_hash.is_none(),
        "the ordinary path computes no digest (W2-2)"
    );
    assert_eq!(verified_outcome.state_hash, verified.state_hash()?);
    Ok(())
}
