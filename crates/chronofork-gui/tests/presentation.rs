// Anti-slop policy §10 exception: owner render-engineer; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It mirrors
// the existing headers in crates/chronofork-cli/tests/m6.rs and
// crates/chronofork-gui/tests/model.rs; no production code is affected.
#![allow(clippy::expect_used)]

//! Capability-gated video/audio pipeline acceptance tests for the M7 GUI.
//!
//! - M7-A12 — with `rendering=false` the pipeline is `Unavailable{CapabilityDisabled}` and
//!   the injected source records **zero** calls; with `rendering=true` a deterministic
//!   test-pattern source (a fixture, never production code) produces frames of the declared
//!   dimensions with stable SHA-256 digests, an oversized frame is a typed error, and the
//!   process surface still reports `rendering=unavailable` because v1 configures no source.
//! - M7-A13 — with `audio=false` the pipeline is `Unavailable{CapabilityDisabled}` and the
//!   injected sink records **zero** writes; with `audio=true` and a recording sink the
//!   chunks are byte-identical and pass through untransformed (no resampling, no mixing, no
//!   device semantic), an oversized chunk is a typed error, and the process surface reports
//!   `audio=unavailable` in every configuration.
//!
//! The gate under test is the canonical descriptor: every capabilities value here is
//! decoded from a `chronofork.capabilities.v1` result, exactly as the command session
//! receives it at startup.

use std::sync::{Arc, Mutex};

use chronofork_gui::presentation::{AudioPipeline, Presentation, UnavailableReason, VideoPipeline};
use chronofork_gui::render::RenderOptions;
use chronofork_gui::render::text::{self, FrameSource};
use chronofork_gui::view::{
    AudioChunk, AudioSink, BackendCapabilities, PresentationFrame, VideoSource,
};
use chronofork_gui::{
    GuiError, GuiErrorCode, GuiModel, MAX_AUDIO_SAMPLES, MAX_PRESENTATION_PIXELS, WindowId,
};
use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, to_canonical_bytes};

/// Decodes a capabilities view carrying the given presentation flags, so the gate is always
/// the descriptor a real service would send.
fn capabilities(rendering: bool, audio: bool) -> BackendCapabilities {
    let mut flags = std::collections::BTreeMap::new();
    for name in [
        "execution",
        "save_states",
        "memory_read",
        "memory_write",
        "register_inspection",
        "instruction_stepping",
        "breakpoints",
        "watchpoints",
        "debug_events",
        "tracing",
        "state_cloning",
        "deterministic_replay",
        "rendering",
        "audio",
    ] {
        let enabled = match name {
            "rendering" => rendering,
            "audio" => audio,
            _ => true,
        };
        flags.insert(name.to_owned(), JsonValue::from_bool(enabled));
    }
    let result = JsonValue::object([
        ("schema", JsonValue::string("chronofork.capabilities.v1")),
        ("api_version", JsonValue::int(2)),
        ("service_version", JsonValue::string("0.1.0")),
        (
            "backend",
            JsonValue::object([
                ("id", JsonValue::string("synthetic")),
                ("display_name", JsonValue::string("Synthetic")),
                ("version", JsonValue::string("0.1.0")),
                ("test_backend", JsonValue::from_bool(true)),
                ("capabilities", JsonValue::Object(flags)),
            ]),
        ),
        ("ops", JsonValue::Array(Vec::new())),
        (
            "limits",
            JsonValue::object([
                ("max_request_bytes", JsonValue::int(1_048_576)),
                ("max_response_bytes", JsonValue::int(1_048_576)),
                ("max_steps_per_request", JsonValue::int(100_000)),
                ("max_memory_read", JsonValue::int(4096)),
                ("max_sessions", JsonValue::int(1024)),
                ("max_store_entries", JsonValue::int(256)),
                ("max_checkpoints", JsonValue::int(32)),
                ("json_max_depth", JsonValue::int(32)),
                ("max_run_wall_ms", JsonValue::int(30_000)),
                ("max_operation_journal", JsonValue::int(1_024)),
                ("operation_ttl", JsonValue::int(600)),
                ("max_closed_retained", JsonValue::int(64)),
                ("closed_retention_ttl", JsonValue::int(300)),
                ("max_snapshots_per_session", JsonValue::int(64)),
                ("max_bytes_per_session", JsonValue::int(67_108_864)),
                ("max_store_bytes", JsonValue::int(1_073_741_824)),
            ]),
        ),
        (
            "store",
            JsonValue::object([("configured", JsonValue::from_bool(false))]),
        ),
        (
            "artifacts",
            JsonValue::object([("configured", JsonValue::from_bool(false))]),
        ),
        ("process_epoch", JsonValue::int(7)),
    ]);
    let envelope = JsonValue::object([
        ("op", JsonValue::string("capabilities")),
        ("result", result),
    ]);
    let bytes = to_canonical_bytes(&envelope, &ServiceLimits::default()).expect("canonical bytes");
    let view = chronofork_gui::decode_capabilities(&bytes).expect("capabilities view");
    *view.backend().capabilities()
}

/// A deterministic test-pattern source. This is a **fixture**: no shipped path fabricates a
/// picture, and the pattern exists only to exercise the `Available` path.
struct TestPatternSource {
    width: u32,
    height: u32,
    calls: Mutex<Vec<u64>>,
}

impl TestPatternSource {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<u64> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn pattern(&self, sequence: u64) -> PresentationFrame {
        let pixels =
            usize::try_from(u64::from(self.width) * u64::from(self.height)).expect("pixel count");
        let mut rgba = Vec::with_capacity(pixels * 4);
        for index in 0..pixels {
            let index = u64::try_from(index).expect("pixel index");
            let red = u8::try_from((index + sequence) % 256).expect("red");
            let green = u8::try_from((index * 3 + sequence) % 256).expect("green");
            rgba.extend_from_slice(&[red, green, 0x40, 0xff]);
        }
        PresentationFrame::new(self.width, self.height, rgba, sequence).expect("pattern frame")
    }
}

impl VideoSource for TestPatternSource {
    fn frame(&self, sequence: u64) -> Result<PresentationFrame, GuiError> {
        self.calls.lock().expect("calls lock").push(sequence);
        Ok(self.pattern(sequence))
    }
}

/// A recording sink. This is a **fixture**: it stores what it receives and changes nothing.
struct RecordingSink {
    chunks: Mutex<Vec<AudioChunk>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            chunks: Mutex::new(Vec::new()),
        }
    }

    fn chunks(&self) -> Vec<AudioChunk> {
        self.chunks.lock().expect("chunks lock").clone()
    }
}

impl AudioSink for RecordingSink {
    fn write(&self, chunk: &AudioChunk) -> Result<(), GuiError> {
        self.chunks.lock().expect("chunks lock").push(chunk.clone());
        Ok(())
    }
}

fn chunk(sequence: u64) -> AudioChunk {
    AudioChunk::new(44_100, vec![1, -2, 3, -4, 5], sequence).expect("audio chunk")
}

/// The frame status line for one window, rendered through the documented text renderer.
fn status_line(
    rendering: &VideoPipeline,
    audio: &AudioPipeline,
    flags: &BackendCapabilities,
) -> String {
    let model = GuiModel::new(WindowId::from_raw(1));
    let source = FrameSource {
        model: &model,
        controller: "human",
        lease_generation: 0,
        lease_sequence: 0,
        fps: 60,
        timeline: None,
        rendering: rendering.availability(flags),
        audio: audio.availability(flags),
        width: 80,
        height: 24,
    };
    let text = text::render(&source, RenderOptions::plain()).expect("frame");
    text.lines().next_back().expect("status line").to_owned()
}

#[test]
fn video_pipeline_never_calls_the_source_when_the_capability_is_disabled() {
    let source = Arc::new(TestPatternSource::new(4, 4));
    let pipeline = VideoPipeline::new(Some(source.clone()));
    let flags = capabilities(false, false);

    assert_eq!(
        pipeline.availability(&flags),
        Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
    );
    assert_eq!(
        pipeline.frame(&flags, 0).expect("gate"),
        Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
    );
    assert_eq!(
        pipeline.frame(&flags, 99).expect("gate"),
        Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
    );
    assert!(
        source.calls().is_empty(),
        "a disabled capability must not call the source"
    );
}

#[test]
fn video_pipeline_reports_no_output_device_when_no_source_is_configured() {
    let pipeline = VideoPipeline::new(None);
    let flags = capabilities(true, true);

    assert_eq!(
        pipeline.availability(&flags),
        Presentation::Unavailable(UnavailableReason::NoOutputDevice)
    );
    assert_eq!(
        pipeline.frame(&flags, 7).expect("gate"),
        Presentation::Unavailable(UnavailableReason::NoOutputDevice)
    );
}

#[test]
fn video_test_pattern_frames_are_deterministic_and_bounded() {
    let flags = capabilities(true, false);
    let source = Arc::new(TestPatternSource::new(3, 2));
    let pipeline = VideoPipeline::new(Some(source.clone()));

    let frame = pipeline
        .frame(&flags, 11)
        .expect("gate")
        .value()
        .expect("available frame");
    assert_eq!(frame.width(), 3);
    assert_eq!(frame.height(), 2);
    assert_eq!(frame.sequence(), 11);
    assert_eq!(frame.rgba().len(), 3 * 2 * 4);

    // A second pipeline over an equivalent fixture produces the very same bytes: the
    // pattern and its digest are a pure function of the frame and its sequence.
    let second = VideoPipeline::new(Some(Arc::new(TestPatternSource::new(3, 2))))
        .frame(&flags, 11)
        .expect("gate")
        .value()
        .expect("available frame");
    assert_eq!(frame.rgba(), second.rgba());
    assert_eq!(frame.digest(), second.digest());
    assert_eq!(source.calls(), [11]);

    // A different sequence is a different frame, so the digest actually covers the bytes.
    let next = pipeline
        .frame(&flags, 12)
        .expect("gate")
        .value()
        .expect("available frame");
    assert_ne!(frame.digest(), next.digest());
    assert_eq!(source.calls(), [11, 12]);

    // The pixel bound is validated before any buffer is trusted.
    let error = PresentationFrame::new(MAX_PRESENTATION_PIXELS as u32 + 1, 1, Vec::new(), 0)
        .expect_err("over the pixel bound");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    assert!(matches!(
        error,
        GuiError::LimitExceeded {
            what: "presentation frame pixels",
            ..
        }
    ));

    // The process surface still reports `rendering=unavailable`: v1 configures no pixel
    // device, so a descriptor that advertises the capability changes nothing there.
    let status = status_line(&VideoPipeline::new(None), &AudioPipeline::new(None), &flags);
    assert!(status.contains("rendering=unavailable"), "{status:?}");
}

#[test]
fn audio_pipeline_never_writes_when_the_capability_is_disabled() {
    let sink = Arc::new(RecordingSink::new());
    let pipeline = AudioPipeline::new(Some(sink.clone()));
    let flags = capabilities(false, false);

    assert_eq!(
        pipeline.availability(&flags),
        Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
    );
    assert_eq!(
        pipeline.write(&flags, &chunk(0)).expect("gate"),
        Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
    );
    assert!(
        sink.chunks().is_empty(),
        "a disabled capability must not write to the sink"
    );
}

#[test]
fn audio_pipeline_reports_no_output_device_when_no_sink_is_configured() {
    let pipeline = AudioPipeline::new(None);
    let flags = capabilities(false, true);

    assert_eq!(
        pipeline.availability(&flags),
        Presentation::Unavailable(UnavailableReason::NoOutputDevice)
    );
    assert_eq!(
        pipeline.write(&flags, &chunk(2)).expect("gate"),
        Presentation::Unavailable(UnavailableReason::NoOutputDevice)
    );
}

#[test]
fn audio_chunks_are_deterministic_bounded_and_pass_through_untransformed() {
    let flags = capabilities(false, true);
    let sink = Arc::new(RecordingSink::new());
    let pipeline = AudioPipeline::new(Some(sink.clone()));

    assert_eq!(pipeline.availability(&flags), Presentation::available(()));
    assert_eq!(
        pipeline.write(&flags, &chunk(3)).expect("write"),
        Presentation::available(())
    );
    assert_eq!(
        pipeline.write(&flags, &chunk(3)).expect("write"),
        Presentation::available(())
    );

    let chunks = sink.chunks();
    assert_eq!(chunks.len(), 2, "one write per chunk, nothing extra");
    assert_eq!(chunks[0], chunks[1], "identical chunks stay identical");
    assert_eq!(chunks[0], chunk(3), "the chunk is passed through unchanged");
    assert_eq!(chunks[0].samples(), [1, -2, 3, -4, 5]);
    assert_eq!(chunks[0].sample_rate(), 44_100);
    assert_eq!(chunks[0].sequence(), 3);
    assert_eq!(chunks[0].digest(), chunks[1].digest());

    // The process surface reports `audio=unavailable` in every configuration: v1 has no
    // audio device, so a descriptor that advertises `audio=true` changes nothing there.
    let status = status_line(&VideoPipeline::new(None), &AudioPipeline::new(None), &flags);
    assert!(status.contains("audio=unavailable"), "{status:?}");

    // The sample bound is validated before the chunk is trusted.
    let samples = vec![0_i16; MAX_AUDIO_SAMPLES + 1];
    let error = AudioChunk::new(44_100, samples, 0).expect_err("over the sample bound");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    assert!(matches!(
        error,
        GuiError::LimitExceeded {
            what: "audio chunk samples",
            ..
        }
    ));
}

#[test]
fn the_process_surface_never_reports_video_or_audio_as_available() {
    // v1 configures no source and no sink in any mode, even against a descriptor that
    // advertises both capabilities: the pipelines stay unavailable and the frame status
    // line says so.
    let flags = capabilities(true, true);
    let video = VideoPipeline::new(None);
    let audio = AudioPipeline::new(None);
    assert!(!video.availability(&flags).is_available());
    assert!(!audio.availability(&flags).is_available());

    let status = status_line(&video, &audio, &flags);
    assert!(
        status.contains("rendering=unavailable"),
        "the process never reports rendering: {status:?}"
    );
    assert!(
        status.contains("audio=unavailable"),
        "the process never reports audio: {status:?}"
    );

    // The disabled descriptor reports the same status-line labels, for its own reason.
    let flags = capabilities(false, false);
    let status = status_line(&video, &audio, &flags);
    assert!(status.contains("rendering=unavailable audio=unavailable"));
    assert_eq!(
        video.availability(&capabilities(false, true)),
        Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
    );
}

#[test]
fn presentation_unavailable_reasons_are_the_documented_closed_set() {
    let reasons = [
        (UnavailableReason::CapabilityDisabled, "capability_disabled"),
        (UnavailableReason::NoOutputDevice, "no_output_device"),
        (
            UnavailableReason::NotExposedByTransport,
            "not_exposed_by_transport",
        ),
        (UnavailableReason::NoSession, "no_session"),
        (UnavailableReason::NoMovie, "no_movie"),
    ];
    for (reason, name) in reasons {
        assert_eq!(reason.as_str(), name);
    }
}
