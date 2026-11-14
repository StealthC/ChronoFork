#![allow(clippy::expect_used)]

//! Adversarial property tests for the Mega Drive scheduler and VDP snapshot
//! parsers.
//!
//! Both parsers consume bytes restored from a checkpoint. Arbitrary bytes must
//! either be rejected with the parser's typed error or accepted as a canonical
//! snapshot that re-encodes to the same bytes; no input may panic.

use chronofork_megadrive::vdp::{SNAPSHOT_LEN as VDP_SNAPSHOT_LEN, SNAPSHOT_MAGIC as VDP_MAGIC};
use chronofork_megadrive::{InterruptScheduler, SchedulerConfig, Vdp};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::Index;

/// The scheduler snapshot header starts with an eight-byte magic value.
const SCHEDULER_MAGIC_LEN: usize = 8;

fn scheduler() -> InterruptScheduler {
    InterruptScheduler::new(SchedulerConfig::ntsc())
}

fn scheduler_snapshot(quanta: u64) -> Vec<u8> {
    let mut scheduler = scheduler();
    scheduler.advance(quanta).expect("advance scheduler");
    scheduler.snapshot_bytes()
}

fn vdp_snapshot(operations: Vec<(u8, u16)>) -> Vec<u8> {
    let mut vdp = Vdp::new();
    for (offset, value) in operations {
        // Invalid port writes are part of the adversarial state space.
        let _ = vdp.write16(offset, value);
    }
    vdp.snapshot_bytes()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn scheduler_arbitrary_bytes_never_panic(
        bytes in vec(any::<u8>(), 0..512),
    ) {
        let mut scheduler = scheduler();
        match scheduler.restore_bytes(&bytes) {
            Ok(()) => prop_assert_eq!(scheduler.snapshot_bytes(), bytes),
            Err(error) => prop_assert!(!error.to_string().is_empty()),
        }
    }

    #[test]
    fn scheduler_snapshots_round_trip_canonically(quanta in 0_u64..4096) {
        let bytes = scheduler_snapshot(quanta);
        let mut scheduler = scheduler();
        scheduler.restore_bytes(&bytes).expect("valid snapshot parses");
        prop_assert_eq!(scheduler.snapshot_bytes(), bytes);
    }

    #[test]
    fn scheduler_truncation_and_extension_are_typed_errors(
        quanta in 0_u64..1024,
        keep in any::<usize>(),
        extra in vec(any::<u8>(), 1..32),
    ) {
        let bytes = scheduler_snapshot(quanta);
        let cut = keep % bytes.len();
        let mut shorter = scheduler();
        prop_assert!(shorter.restore_bytes(&bytes[..cut]).is_err());

        let mut extended = bytes;
        extended.extend_from_slice(&extra);
        let mut longer = scheduler();
        prop_assert!(longer.restore_bytes(&extended).is_err());
    }

    #[test]
    fn scheduler_magic_corruption_is_typed(
        quanta in 0_u64..1024,
        position in any::<Index>(),
        delta in 1_u8..=u8::MAX,
    ) {
        let mut bytes = scheduler_snapshot(quanta);
        let index = position.index(SCHEDULER_MAGIC_LEN);
        bytes[index] ^= delta;
        let mut scheduler = scheduler();
        prop_assert!(scheduler.restore_bytes(&bytes).is_err());
    }

    #[test]
    fn vdp_arbitrary_bytes_never_panic(
        bytes in vec(any::<u8>(), 0..2048),
    ) {
        let mut vdp = Vdp::new();
        match vdp.restore_bytes(&bytes) {
            Ok(()) => prop_assert_eq!(vdp.snapshot_bytes(), bytes),
            Err(error) => prop_assert!(!error.to_string().is_empty()),
        }
    }

    #[test]
    fn vdp_exact_length_arbitrary_bytes_never_panic(
        bytes in vec(any::<u8>(), VDP_SNAPSHOT_LEN),
    ) {
        let mut vdp = Vdp::new();
        match vdp.restore_bytes(&bytes) {
            Ok(()) => prop_assert_eq!(vdp.snapshot_bytes(), bytes),
            Err(error) => prop_assert!(!error.to_string().is_empty()),
        }
    }

    #[test]
    fn vdp_snapshots_round_trip_canonically(
        operations in vec((0_u8..16, any::<u16>()), 0..32),
    ) {
        let bytes = vdp_snapshot(operations);
        let mut vdp = Vdp::new();
        vdp.restore_bytes(&bytes).expect("valid snapshot parses");
        prop_assert_eq!(vdp.snapshot_bytes(), bytes);
    }

    #[test]
    fn vdp_truncation_and_extension_are_typed_errors(
        operations in vec((0_u8..16, any::<u16>()), 0..16),
        delta in 1_usize..32,
        extra in vec(any::<u8>(), 1..32),
    ) {
        let bytes = vdp_snapshot(operations);
        let mut shorter = Vdp::new();
        prop_assert!(shorter.restore_bytes(&bytes[..bytes.len() - delta]).is_err());

        let mut extended = bytes;
        extended.extend_from_slice(&extra);
        let mut longer = Vdp::new();
        prop_assert!(longer.restore_bytes(&extended).is_err());
    }

    #[test]
    fn vdp_magic_corruption_is_typed(
        operations in vec((0_u8..16, any::<u16>()), 0..8),
        position in any::<Index>(),
        delta in 1_u8..=u8::MAX,
    ) {
        let mut bytes = vdp_snapshot(operations);
        let index = position.index(VDP_MAGIC.len());
        bytes[index] ^= delta;
        let mut vdp = Vdp::new();
        prop_assert!(vdp.restore_bytes(&bytes).is_err());
    }
}
