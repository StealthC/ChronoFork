#![allow(clippy::expect_used)]

//! Payload v6 identity, v5 rejection and round-trip coverage.

use chronofork_backend_api::{Backend, BackendError, BackendFactory, BackendSnapshot};
use chronofork_core::InputFrame;
use chronofork_megadrive::backend::{CONFIG_MAGIC, PAYLOAD_MAGIC, PAYLOAD_VERSION, STATE_MAGIC};
use chronofork_megadrive::{MegadriveFactory, demo_rom};

fn machine() -> chronofork_megadrive::MegadriveBackend {
    MegadriveFactory::new(demo_rom())
        .expect("factory")
        .create_machine()
        .expect("machine")
}

fn tampered(snapshot: &BackendSnapshot, magic: &[u8; 8], version: u16) -> BackendSnapshot {
    let mut payload = snapshot.payload().to_vec();
    payload[..8].copy_from_slice(magic);
    payload[8..10].copy_from_slice(&version.to_le_bytes());
    BackendSnapshot::new(
        snapshot.backend_id().clone(),
        snapshot.backend_version().to_owned(),
        snapshot.format_version(),
        snapshot.tick(),
        snapshot.config_hash(),
        snapshot.state_hash(),
        payload,
    )
}

#[test]
fn version_six_identity_is_published() {
    assert_eq!(PAYLOAD_VERSION, 6);
    assert_eq!(&PAYLOAD_MAGIC, b"CFMD68K6");
    assert_eq!(STATE_MAGIC, b"CFMDSTATE6");
    assert_eq!(CONFIG_MAGIC, b"CFMDCONF6");
}

#[test]
fn v6_round_trips_and_embeds_the_vdp_snapshot_with_its_frame_index() {
    let mut original = machine();
    for _ in 0..8 {
        original.step(InputFrame::NEUTRAL).expect("step");
    }
    let snapshot = original.snapshot().expect("snapshot");
    let mut restored = machine();
    restored.restore(&snapshot).expect("restore");
    assert_eq!(
        restored.snapshot().expect("resnapshot").state_hash(),
        snapshot.state_hash()
    );
    assert_eq!(
        restored.vdp().snapshot_bytes(),
        original.vdp().snapshot_bytes()
    );
    assert_eq!(restored.vdp().frame_index(), original.vdp().frame_index());
    let vdp_bytes = original.vdp().snapshot_bytes();
    assert!(
        snapshot
            .payload()
            .windows(vdp_bytes.len())
            .any(|window| window == vdp_bytes),
        "v5 embeds the VDP snapshot (which carries the frame index)"
    );
}

#[test]
fn v1_to_v5_payloads_are_rejected_with_typed_errors() {
    let mut machine = machine();
    machine.step(InputFrame::NEUTRAL).expect("step");
    let snapshot = machine.snapshot().expect("snapshot");

    for version in 1..=5_u16 {
        let candidate = tampered(&snapshot, &PAYLOAD_MAGIC, version);
        let error = machine
            .restore(&candidate)
            .expect_err("an old payload version is rejected");
        assert!(
            matches!(error, BackendError::InvalidSnapshot(_)),
            "typed rejection for v{version}: {error}"
        );
        assert!(
            error.to_string().contains(&format!("version {version}")),
            "reports the rejected version: {error}"
        );
    }

    // Old magics are rejected before any field is reinterpreted, even when the
    // version word claims v6.
    for magic in [
        *b"CFMD68K1",
        *b"CFMD68K2",
        *b"CFMD68K3",
        *b"CFMD68K4",
        *b"CFMD68K5",
    ] {
        let candidate = tampered(&snapshot, &magic, PAYLOAD_VERSION);
        assert!(matches!(
            machine.restore(&candidate),
            Err(BackendError::InvalidSnapshot(_))
        ));
    }
}

#[test]
fn rendering_and_audio_are_advertised() {
    let factory = MegadriveFactory::new(demo_rom()).expect("factory");
    let capabilities = factory.descriptor().capabilities();
    assert!(capabilities.rendering, "frame production is tested");
    assert!(capabilities.audio, "the 48 kHz audio seam is tested");
}
