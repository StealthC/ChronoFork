#![allow(clippy::expect_used)]

//! Adversarial property tests for the session checkpoint envelope parser.
//!
//! [`Snapshot::from_bytes`] consumes persisted, potentially untrusted bytes. The
//! parser must either return a typed [`chronofork_sessions::SessionError`] or a
//! snapshot that re-encodes canonically; it must never panic.

use std::sync::Arc;

use chronofork_core::InputFrame;
use chronofork_sessions::{SessionManager, Snapshot};
use chronofork_test_backend::SyntheticBackendFactory;
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::Index;

fn manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

/// Produces a valid serialized checkpoint for the fixed synthetic program.
fn serialized_checkpoint(steps: usize, inputs: &[u32]) -> Vec<u8> {
    let mut manager = manager();
    let session = manager.create_session().expect("session");
    for index in 0..steps {
        let input = inputs
            .get(index % inputs.len().max(1))
            .copied()
            .unwrap_or(0);
        manager
            .step(session, InputFrame::new(input))
            .expect("session step");
    }
    let snapshot = manager.snapshot(session).expect("checkpoint");
    manager
        .snapshot_bytes(session, snapshot)
        .expect("serialize checkpoint")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn arbitrary_bytes_are_rejected_or_round_trip_without_panicking(
        bytes in vec(any::<u8>(), 0..2048),
    ) {
        match Snapshot::from_bytes(&bytes) {
            Ok(snapshot) => {
                let reencoded = snapshot.to_bytes().expect("valid snapshot re-encodes");
                prop_assert_eq!(reencoded.as_slice(), bytes.as_slice());
                let reparsed = Snapshot::from_bytes(&reencoded).expect("re-encoded snapshot parses");
                prop_assert_eq!(reparsed, snapshot);
            }
            Err(error) => prop_assert!(!error.to_string().is_empty()),
        }
    }

    #[test]
    fn serialized_checkpoints_round_trip_exactly(
        steps in 0_usize..48,
        inputs in vec(any::<u32>(), 0..8),
    ) {
        let bytes = serialized_checkpoint(steps, &inputs);
        let snapshot = Snapshot::from_bytes(&bytes).expect("serialized checkpoint parses");
        prop_assert_eq!(snapshot.to_bytes().expect("re-encodes"), bytes);
        prop_assert!(snapshot.tick().raw() >= u64::try_from(steps).expect("small step count"));
    }

    #[test]
    fn truncation_is_a_typed_error(
        steps in 0_usize..48,
        keep in any::<usize>(),
    ) {
        let bytes = serialized_checkpoint(steps, &[1, 2, 3]);
        let cut = keep % bytes.len();
        let error = Snapshot::from_bytes(&bytes[..cut])
            .expect_err("a truncated checkpoint must be rejected");
        prop_assert!(!error.to_string().is_empty());
    }

    #[test]
    fn single_byte_corruption_is_rejected(
        steps in 0_usize..32,
        position in any::<Index>(),
        delta in 1_u8..=u8::MAX,
    ) {
        let mut bytes = serialized_checkpoint(steps, &[7]);
        let index = position.index(bytes.len());
        bytes[index] ^= delta;
        prop_assert!(Snapshot::from_bytes(&bytes).is_err());
    }

    #[test]
    fn appended_bytes_are_rejected(
        steps in 0_usize..32,
        extra in vec(any::<u8>(), 1..64),
    ) {
        let mut bytes = serialized_checkpoint(steps, &[3]);
        bytes.extend_from_slice(&extra);
        match Snapshot::from_bytes(&bytes) {
            Ok(snapshot) => prop_assert_eq!(snapshot.to_bytes().expect("re-encodes"), bytes),
            Err(error) => prop_assert!(!error.to_string().is_empty()),
        }
    }
}
