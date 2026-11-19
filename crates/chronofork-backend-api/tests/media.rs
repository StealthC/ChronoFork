#![allow(clippy::expect_used)]

//! Contract tests for the backend-neutral media seam.
//!
//! The seam exists so a capture layer can take video frames and audio chunks
//! from a backend through [`MediaSource`] without learning a backend type. A
//! backend that produces no media keeps the default
//! [`Backend::media_mut`] implementation and returns `None`.

use std::collections::BTreeMap;

use chronofork_backend_api::media::{AudioChunk, MediaError, MediaSource, VideoFrame};
use chronofork_backend_api::snapshot::BackendSnapshot;
use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendId, Capabilities, MachineInspection,
    StepOutcome,
};
use chronofork_core::{InputFrame, StateHash, Tick};

/// Minimal backend that keeps every optional surface at its default.
///
/// It exists so the default [`Backend::media_mut`] implementation is exercised
/// by a real trait implementor; the synthetic test backend is covered by its
/// own crate's tests.
struct NullBackend {
    descriptor: BackendDescriptor,
}

impl NullBackend {
    fn new() -> Self {
        Self {
            descriptor: BackendDescriptor::new(
                BackendId::new("chronofork.test.null"),
                "Null backend",
                "0",
                Capabilities::none(),
                true,
            ),
        }
    }
}

impl Backend for NullBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        Err(BackendError::Halted)
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        Err(BackendError::Halted)
    }

    fn restore(&mut self, _snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        Err(BackendError::Halted)
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        Ok(MachineInspection {
            tick: Tick::from_raw(0),
            state_hash: StateHash::from_bytes([0; 32]),
            halted: true,
            registers: BTreeMap::new(),
            memory_base: 0,
            memory: Vec::new(),
        })
    }

    fn read_memory(&self, _address: u64, _length: usize) -> Result<Vec<u8>, BackendError> {
        Ok(Vec::new())
    }

    fn write_memory(&mut self, _address: u64, _data: &[u8]) -> Result<(), BackendError> {
        Ok(())
    }
}

#[test]
fn default_backend_has_no_media_source() {
    let mut backend = NullBackend::new();
    assert!(
        backend.media_mut().is_none(),
        "a backend without media keeps the default None"
    );
}

#[test]
fn video_frame_checksum_is_stable() {
    let frame = VideoFrame::new(320, 224, vec![0x7F; 320 * 224 * 4]).expect("valid frame");
    assert_eq!(frame.checksum(), frame.clone().checksum());
    assert_ne!(frame.checksum().as_bytes(), &[0; 32]);
}

#[test]
fn video_frame_checksum_is_sha256_of_the_documented_canonical_bytes() {
    // Canonical bytes: width LE (2), height LE (1), then RGBA.
    // echo -n '020001000102030405060708' | xxd -r -p | sha256sum
    let frame = VideoFrame::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).expect("valid frame");
    assert_eq!(
        frame.checksum().to_hex(),
        "001120b27cca673d9401d0abaa65bcd56e1fd24825955dc00cff4cef2127c126"
    );
}

#[test]
fn video_frame_checksum_covers_dimensions_and_pixels() {
    // Both frames carry the same eight all-zero RGBA bytes.
    let frame = VideoFrame::new(2, 1, vec![0; 8]).expect("valid frame");
    let other_dimensions = VideoFrame::new(1, 2, vec![0; 8]).expect("valid frame");
    let mut pixels = vec![0; 8];
    pixels[7] = 1;
    let other_pixels = VideoFrame::new(2, 1, pixels).expect("valid frame");

    // The little-endian width/height header is part of the canonical bytes, so
    // two frames with the same all-zero payload but transposed dimensions hash
    // differently.
    assert_ne!(frame.checksum(), other_dimensions.checksum());
    assert_ne!(frame.checksum(), other_pixels.checksum());
}

#[test]
fn video_frame_rejects_a_length_that_does_not_match_dimensions() {
    assert_eq!(
        VideoFrame::new(4, 2, vec![0; 4 * 2 * 4 - 1]),
        Err(MediaError::FrameLengthMismatch {
            expected: 4 * 2 * 4,
            found: 4 * 2 * 4 - 1,
        })
    );
    assert_eq!(
        VideoFrame::new(4, 2, Vec::new()),
        Err(MediaError::FrameLengthMismatch {
            expected: 4 * 2 * 4,
            found: 0,
        })
    );
}

#[test]
fn audio_chunk_rejects_a_sample_count_that_is_not_channel_aligned() {
    assert_eq!(
        AudioChunk::new(vec![0; 7], 44_100, 2),
        Err(MediaError::AudioSampleCountNotAligned {
            samples: 7,
            channels: 2,
        })
    );
    assert_eq!(
        AudioChunk::new(vec![0; 6], 44_100, 2),
        Ok(AudioChunk {
            samples: vec![0; 6],
            sample_rate_hz: 44_100,
            channels: 2,
        })
    );
}

#[test]
fn audio_chunk_rejects_zero_channels_without_panicking() {
    assert_eq!(
        AudioChunk::new(vec![0; 4], 44_100, 0),
        Err(MediaError::AudioChannelCountZero)
    );
}

/// A media source holding one frame and one chunk, used to pin the take-once
/// contract.
struct OneShotSource {
    frame: Option<VideoFrame>,
    chunk: Option<AudioChunk>,
}

impl MediaSource for OneShotSource {
    fn take_video_frame(&mut self) -> Option<VideoFrame> {
        self.frame.take()
    }

    fn take_audio_chunk(&mut self) -> Option<AudioChunk> {
        self.chunk.take()
    }
}

#[test]
fn media_source_hands_out_each_item_once() {
    let mut source = OneShotSource {
        frame: Some(VideoFrame::new(1, 1, vec![0; 4]).expect("valid frame")),
        chunk: Some(AudioChunk {
            samples: vec![-1, 1],
            sample_rate_hz: 22_050,
            channels: 1,
        }),
    };

    assert!(source.take_video_frame().is_some());
    assert!(source.take_video_frame().is_none());
    assert!(source.take_audio_chunk().is_some());
    assert!(source.take_audio_chunk().is_none());
}
