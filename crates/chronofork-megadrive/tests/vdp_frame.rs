#![allow(clippy::expect_used)]

//! Frame production through the media seam.

use chronofork_backend_api::{Backend, MediaSource};
use chronofork_core::{InputFrame, StateHash};
use chronofork_megadrive::{MegadriveFactory, demo_rom};

fn machine() -> chronofork_megadrive::MegadriveBackend {
    MegadriveFactory::new(demo_rom())
        .expect("factory")
        .create_machine()
        .expect("machine")
}

fn first_frame() -> (u16, u16, Vec<u8>, StateHash) {
    let mut machine = machine();
    assert!(machine.media_mut().is_some(), "media seam is advertised");
    let mut found = None;
    for _ in 0..200_000 {
        machine.step(InputFrame::NEUTRAL).expect("step");
        found = machine.take_video_frame();
        if found.is_some() {
            break;
        }
    }
    let frame = found.expect("a frame is produced");
    let checksum = frame.checksum();
    (frame.width, frame.height, frame.rgba, checksum)
}

#[test]
fn frame_is_produced_once_per_frame_with_canonical_rgba() {
    let (width, height, rgba, _) = first_frame();
    assert!(matches!(width, 256 | 320), "H32/H40 width");
    assert_eq!(height, 224, "NTSC active height");
    assert_eq!(rgba.len(), usize::from(width) * usize::from(height) * 4);
    assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 0xFF));
}

#[test]
fn a_second_take_before_a_new_frame_returns_none_and_audio_is_absent() {
    let mut machine = machine();
    let mut first = None;
    for _ in 0..200_000 {
        machine.step(InputFrame::NEUTRAL).expect("step");
        if let Some(frame) = machine.take_video_frame() {
            first = Some(frame.checksum());
            break;
        }
    }
    assert!(first.is_some(), "a frame was produced");
    assert!(machine.take_video_frame().is_none(), "one-frame queue");
    assert!(
        machine.take_audio_chunk().is_none(),
        "audio is not implemented"
    );
}

#[test]
fn frame_hashes_are_deterministic_across_identical_runs() {
    let (_, _, _, first) = first_frame();
    let (_, _, _, second) = first_frame();
    assert_eq!(first, second, "identical state produces the same frame");
}
