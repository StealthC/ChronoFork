#![allow(clippy::expect_used)]

//! Adversarial property tests for the Mega Drive scheduler and VDP snapshot
//! parsers.
//!
//! Both parsers consume bytes restored from a checkpoint. Arbitrary bytes must
//! either be rejected with the parser's typed error or accepted as a canonical
//! snapshot that re-encodes to the same bytes; no input may panic.

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::vdp::{SNAPSHOT_LEN as VDP_SNAPSHOT_LEN, SNAPSHOT_MAGIC as VDP_MAGIC};
use chronofork_megadrive::{InterruptScheduler, MegadriveFactory, SchedulerConfig, Vdp, demo_rom};
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

/// A step's `InputFrame` is not canonical state: a program that never reads the
/// controller serializes identically for every frame it is given, including
/// non-neutral ports, axes and keys.
#[test]
fn controller_input_never_enters_the_canonical_snapshot() {
    let factory = MegadriveFactory::new(demo_rom()).expect("valid demo ROM");
    let mut neutral = factory.create_machine().expect("machine");
    let mut non_neutral = factory.create_machine().expect("machine");
    let extreme = InputFrame::with_devices([u32::MAX; 4], [i16::MAX; 4], u64::MAX);
    // The demo ROM exercises the ALU, work RAM and the VDP port window; it never
    // reads the controller window.
    for _ in 0..23 {
        neutral.step(InputFrame::NEUTRAL).expect("neutral step");
        non_neutral.step(extreme).expect("input step");
    }

    let neutral_snapshot = neutral.snapshot().expect("neutral snapshot");
    let non_neutral_snapshot = non_neutral.snapshot().expect("input snapshot");
    assert_eq!(
        non_neutral_snapshot.payload(),
        neutral_snapshot.payload(),
        "input must not be serialized"
    );
    assert_eq!(
        non_neutral_snapshot.payload().len(),
        neutral_snapshot.payload().len()
    );
    assert_eq!(
        non_neutral_snapshot.state_hash(),
        neutral_snapshot.state_hash()
    );
    assert_eq!(
        non_neutral_snapshot.config_hash(),
        neutral_snapshot.config_hash()
    );
    assert_eq!(
        non_neutral.inspect().expect("inspect").memory,
        neutral.inspect().expect("inspect").memory
    );
}
