//! Version 4 snapshot identity and extended register-file round trips.

#![allow(clippy::expect_used)]

use std::error::Error;

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::backend::{CONFIG_MAGIC, PAYLOAD_MAGIC, PAYLOAD_VERSION, STATE_MAGIC};
use chronofork_megadrive::{MegadriveBackend, MegadriveFactory, demo_rom};

fn new_machine() -> Result<MegadriveBackend, Box<dyn Error>> {
    Ok(MegadriveFactory::new(demo_rom())?.create_machine()?)
}

#[test]
fn version_four_identity_is_published() {
    assert_eq!(PAYLOAD_VERSION, 4);
    assert_eq!(&PAYLOAD_MAGIC, b"CFMD68K4");
    assert_eq!(STATE_MAGIC, b"CFMDSTATE4");
    assert_eq!(CONFIG_MAGIC, b"CFMDCONF4");
}

#[test]
fn v4_snapshot_round_trips_master_cycles_stacks_and_cpu_cycles() -> Result<(), Box<dyn Error>> {
    let mut machine = new_machine()?;
    machine.step(InputFrame::NEUTRAL)?;
    machine.step(InputFrame::NEUTRAL)?;
    let snapshot = machine.snapshot()?;
    let view = machine.inspect()?;
    let expected: Vec<(String, u64)> = [
        "master_cycles",
        "cpu_cycles",
        "usp",
        "ssp",
        "scheduler_frame",
        "scheduler_line",
        "scheduler_master_cycle_in_line",
    ]
    .iter()
    .map(|name| (name.to_string(), view.registers[*name]))
    .collect();

    let mut restored = new_machine()?;
    restored.restore(&snapshot)?;
    let restored_view = restored.inspect()?;
    for (name, value) in expected {
        assert_eq!(restored_view.registers[&name], value, "{name} round trip");
    }
    assert_eq!(restored.master_cycles(), machine.master_cycles());
    assert_eq!(restored.cpu().cycles(), machine.cpu().cycles());
    assert_eq!(restored.cpu().usp(), machine.cpu().usp());
    assert_eq!(restored.cpu().ssp(), machine.cpu().ssp());
    assert_eq!(
        restored.interrupts().snapshot_bytes(),
        machine.interrupts().snapshot_bytes()
    );
    Ok(())
}

#[test]
fn snapshot_state_hash_covers_the_master_counter() -> Result<(), Box<dyn Error>> {
    let mut machine = new_machine()?;
    machine.step(InputFrame::NEUTRAL)?;
    let first = machine.snapshot()?;
    machine.step(InputFrame::NEUTRAL)?;
    let second = machine.snapshot()?;
    assert_ne!(first.state_hash(), second.state_hash());

    let mut payload = first.payload().to_vec();
    let tick_offset = 8 + 2 + 1 + 8 + 32;
    payload[tick_offset] ^= 0x01;
    let mut tampered = chronofork_backend_api::BackendSnapshot::new(
        first.backend_id().clone(),
        first.backend_version(),
        first.format_version(),
        first.tick(),
        first.config_hash(),
        first.state_hash(),
        payload,
    );
    assert!(
        machine.restore(&tampered).is_err(),
        "payload hash must be checked"
    );
    let _ = &mut tampered;
    Ok(())
}

#[test]
fn v1_to_v3_payloads_are_rejected_with_typed_errors() -> Result<(), Box<dyn Error>> {
    let mut machine = new_machine()?;
    machine.step(InputFrame::NEUTRAL)?;
    let snapshot = machine.snapshot()?;
    for (magic, version) in [
        (b"CFMD68K1".as_slice(), 1_u16),
        (b"CFMD68K2".as_slice(), 2),
        (b"CFMD68K3".as_slice(), 3),
    ] {
        let mut payload = snapshot.payload().to_vec();
        payload[0..8].copy_from_slice(magic);
        payload[8..10].copy_from_slice(&version.to_le_bytes());
        let older = chronofork_backend_api::BackendSnapshot::new(
            snapshot.backend_id().clone(),
            snapshot.backend_version(),
            snapshot.format_version(),
            snapshot.tick(),
            snapshot.config_hash(),
            snapshot.state_hash(),
            payload,
        );
        let error = machine.restore(&older).expect_err("v1-v3 must be rejected");
        assert!(
            matches!(
                error,
                chronofork_backend_api::BackendError::InvalidSnapshot(_)
            ),
            "typed snapshot error, got {error:?}"
        );
    }
    Ok(())
}
