//! The capability-gated presentation surfaces: video, audio and panel availability.
//!
//! This module resolves presentation availability from canonical values and never
//! fabricates a picture or a sound. The vocabulary it resolves over lives in
//! [`crate::view`]: [`PresentationFrame`], [`AudioChunk`], [`VideoSource`] and [`AudioSink`]
//! are bounded value types and contracts, and [`Presentation<T>`] here is
//! `Available(T) | Unavailable(reason)` where the reason is one [`UnavailableReason`] of the
//! documented closed set:
//!
//! | Reason | Meaning | Reported by |
//! |---|---|---|
//! | `capability_disabled` | the canonical descriptor reports `rendering=false`, `audio=false` or `register_inspection=false` | the video pipeline, the audio pipeline, the `inspectors` panel's `cpu` section |
//! | `no_output_device` | v1 has no pixel or audio device, so the GUI cannot present data it could receive | the video pipeline, the audio pipeline |
//! | `not_exposed_by_transport` | the M6 operation surface does not carry the data at all (the `vdp`, `interrupts`, `bus` and `disassembly` sections of `inspectors`, blocked on M6-FU1) | the `inspectors` panel |
//! | `no_session` | no pane is attached, so no canonical view backs the panel | [`panel_availability`] |
//! | `no_movie` | the window was opened without `--movie`, so the TAS timeline has no data | [`panel_availability`] |
//!
//! The gate is the capability flag of the **canonical** descriptor
//! ([`BackendCapabilities`]): when `rendering` is `false` the video pipeline is
//! `Unavailable{CapabilityDisabled}` and the source is **never called**; when `audio` is
//! `false` the audio pipeline is `Unavailable{CapabilityDisabled}` and the sink is **never
//! written**. Both current backends report `false`, so `capability_disabled` is the honest
//! process result. A descriptor that advertises `true` still resolves to
//! `Unavailable{NoOutputDevice}` in v1, because the terminal and the HTML document present
//! no pixels and play no samples: the process surface never reports video or audio as
//! available (`docs/adr/0013-human-facing-gui.md` §3.6).
//!
//! The `Available` path is exercised by the crate's tests with a deterministic test-pattern
//! source and a recording sink. Those are **fixtures**, never production code: no shipped
//! path fabricates a picture or a sound, and this module adds no rendering, timing,
//! resampling, mixing or device semantic — H2 owns video depth and H3 owns audio depth.

use std::fmt;
use std::sync::Arc;

use crate::GuiError;
use crate::model::PanelKind;
use crate::view::{AudioChunk, AudioSink, BackendCapabilities, PresentationFrame, VideoSource};

/// Why a presentation surface is unavailable.
///
/// The set is closed and every variant is reported honestly: a surface either has data to
/// present or names one of these reasons, never a placeholder or a fabricated value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    /// The canonical descriptor does not advertise the capability this surface needs.
    CapabilityDisabled,
    /// No pixel or audio device is configured: v1 presents text and a static document.
    NoOutputDevice,
    /// The operation surface does not carry this data (M6-FU1 for the debugger sections).
    NotExposedByTransport,
    /// No pane is attached, so no canonical view backs the panel.
    NoSession,
    /// The window was opened without `--movie`, so the TAS timeline has no data.
    NoMovie,
}

impl UnavailableReason {
    /// Returns the stable wire string of this reason.
    ///
    /// These strings are what the frame renderer prints after `reason=` and inside
    /// `unavailable:<reason>`, and what the `panel` block reports as `reason`; there is no
    /// second spelling table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CapabilityDisabled => "capability_disabled",
            Self::NoOutputDevice => "no_output_device",
            Self::NotExposedByTransport => "not_exposed_by_transport",
            Self::NoSession => "no_session",
            Self::NoMovie => "no_movie",
        }
    }
}

/// A presentation surface that either carries a value or names why it cannot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Presentation<T> {
    /// The surface has data to present.
    Available(T),
    /// The surface has no data, for the documented reason.
    Unavailable(UnavailableReason),
}

impl<T> Presentation<T> {
    /// Wraps an available value.
    #[must_use]
    pub const fn available(value: T) -> Self {
        Self::Available(value)
    }

    /// Wraps an unavailable surface with its reason.
    #[must_use]
    pub const fn unavailable(reason: UnavailableReason) -> Self {
        Self::Unavailable(reason)
    }

    /// Returns whether the surface is available.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// Returns the unavailability reason, when the surface is unavailable.
    #[must_use]
    pub const fn reason(&self) -> Option<UnavailableReason> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable(reason) => Some(*reason),
        }
    }

    /// Consumes the surface, returning its value when it is available.
    #[must_use]
    pub fn value(self) -> Option<T> {
        match self {
            Self::Available(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }
}

/// A presentation surface with no payload: exactly what an availability question needs.
pub type Availability = Presentation<()>;

/// Resolves whether a panel has canonical data to render.
///
/// This is the one owner of the panel availability matrix, so the frame renderer and the
/// `panel` command block cannot disagree about it:
///
/// | Panel | Available when |
/// |---|---|
/// | `status`, `lineage`, `capabilities`, `registers`, `memory`, `inspectors` | a pane is attached (`has_session`) |
/// | `timeline` | the CLI supplied a movie (`has_movie`), independent of the panes |
/// | `events` | always: the presentation event log exists with the window |
///
/// A panel that is visible but unavailable renders `<name> unavailable reason=<reason>` and
/// never an invented value.
#[must_use]
pub fn panel_availability(kind: PanelKind, has_session: bool, has_movie: bool) -> Availability {
    match kind {
        PanelKind::EventLog => Presentation::available(()),
        PanelKind::Timeline => {
            if has_movie {
                Presentation::available(())
            } else {
                Presentation::unavailable(UnavailableReason::NoMovie)
            }
        }
        _ => {
            if has_session {
                Presentation::available(())
            } else {
                Presentation::unavailable(UnavailableReason::NoSession)
            }
        }
    }
}

/// The capability-gated video pipeline.
///
/// The pipeline holds an optional [`VideoSource`] and nothing else: it adds no frame
/// timing, no scaling and no pixel semantic. A source is configured only by a frontend
/// that can actually present pixels, and v1 configures none.
pub struct VideoPipeline {
    source: Option<Arc<dyn VideoSource>>,
}

impl fmt::Debug for VideoPipeline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VideoPipeline")
            .field("source", &self.source.is_some())
            .finish()
    }
}

impl VideoPipeline {
    /// Creates a pipeline over an optional source.
    ///
    /// `None` is the v1 configuration in every mode: no pixel device exists, so the
    /// pipeline resolves to `Unavailable{NoOutputDevice}` even when the descriptor
    /// advertises `rendering=true`. Tests configure a deterministic source to exercise the
    /// `Available` path.
    #[must_use]
    pub fn new(source: Option<Arc<dyn VideoSource>>) -> Self {
        Self { source }
    }

    /// Resolves the availability of this pipeline for a descriptor's capability flags.
    ///
    /// The capability flag is checked first and the source is not consulted, so a disabled
    /// capability can never reach it.
    #[must_use]
    pub fn availability(&self, capabilities: &BackendCapabilities) -> Availability {
        if !capabilities.rendering() {
            return Presentation::unavailable(UnavailableReason::CapabilityDisabled);
        }
        match &self.source {
            None => Presentation::unavailable(UnavailableReason::NoOutputDevice),
            Some(_) => Presentation::available(()),
        }
    }

    /// Produces the frame carrying `sequence`, when the capability and a source allow it.
    ///
    /// This is the only path that can produce a [`PresentationFrame`], and it always asks
    /// a [`VideoSource`]: no code path constructs a frame from nothing.
    ///
    /// # Errors
    ///
    /// Propagates the source's typed [`GuiError`]. A disabled capability or a missing
    /// source is **not** an error — it is an honest `Unavailable`, which is what the
    /// frontends render.
    pub fn frame(
        &self,
        capabilities: &BackendCapabilities,
        sequence: u64,
    ) -> Result<Presentation<PresentationFrame>, GuiError> {
        if !capabilities.rendering() {
            return Ok(Presentation::unavailable(
                UnavailableReason::CapabilityDisabled,
            ));
        }
        match &self.source {
            None => Ok(Presentation::unavailable(UnavailableReason::NoOutputDevice)),
            Some(source) => source.frame(sequence).map(Presentation::Available),
        }
    }
}

/// The capability-gated audio pipeline.
///
/// The pipeline holds an optional [`AudioSink`] and nothing else: it adds no resampling, no
/// mixing, no volume and no device semantic. A sink is configured only by a frontend that
/// can actually play samples, and v1 configures none.
pub struct AudioPipeline {
    sink: Option<Arc<dyn AudioSink>>,
}

impl fmt::Debug for AudioPipeline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioPipeline")
            .field("sink", &self.sink.is_some())
            .finish()
    }
}

impl AudioPipeline {
    /// Creates a pipeline over an optional sink.
    ///
    /// `None` is the v1 configuration in every mode: there is no audio device, so the
    /// pipeline resolves to `Unavailable{NoOutputDevice}` even when the descriptor
    /// advertises `audio=true`. Tests configure a recording sink to exercise the
    /// `Available` path.
    #[must_use]
    pub fn new(sink: Option<Arc<dyn AudioSink>>) -> Self {
        Self { sink }
    }

    /// Resolves the availability of this pipeline for a descriptor's capability flags.
    ///
    /// The capability flag is checked first and the sink is not consulted, so a disabled
    /// capability can never reach it.
    #[must_use]
    pub fn availability(&self, capabilities: &BackendCapabilities) -> Availability {
        if !capabilities.audio() {
            return Presentation::unavailable(UnavailableReason::CapabilityDisabled);
        }
        match &self.sink {
            None => Presentation::unavailable(UnavailableReason::NoOutputDevice),
            Some(_) => Presentation::available(()),
        }
    }

    /// Writes one chunk to the configured sink, when the capability and a sink allow it.
    ///
    /// The chunk is passed through unchanged: the pipeline adds no resampling, no mixing
    /// and no format conversion.
    ///
    /// # Errors
    ///
    /// Propagates the sink's typed [`GuiError`]. A disabled capability or a missing sink is
    /// **not** an error — it is an honest `Unavailable`.
    pub fn write(
        &self,
        capabilities: &BackendCapabilities,
        chunk: &AudioChunk,
    ) -> Result<Availability, GuiError> {
        if !capabilities.audio() {
            return Ok(Presentation::unavailable(
                UnavailableReason::CapabilityDisabled,
            ));
        }
        match &self.sink {
            None => Ok(Presentation::unavailable(UnavailableReason::NoOutputDevice)),
            Some(sink) => {
                sink.write(chunk)?;
                Ok(Presentation::available(()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::{
        AudioPipeline, Availability, Presentation, UnavailableReason, VideoPipeline,
        panel_availability,
    };
    use crate::model::PanelKind;
    use crate::view::{AudioChunk, AudioSink, BackendCapabilities, PresentationFrame, VideoSource};
    use crate::{GuiError, MAX_AUDIO_SAMPLES};

    /// A capabilities value built by decoding the canonical `capabilities` result, so the
    /// gate is always the descriptor a real service would send.
    fn capabilities(rendering: bool, audio: bool) -> BackendCapabilities {
        use chronofork_service::ServiceLimits;
        use chronofork_service::json::{JsonValue, to_canonical_bytes};

        let names = [
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
        ];
        let mut flags = std::collections::BTreeMap::new();
        for name in names {
            let enabled = match name {
                "rendering" => rendering,
                "audio" => audio,
                _ => true,
            };
            flags.insert(name.to_owned(), JsonValue::from_bool(enabled));
        }
        let result = JsonValue::object([
            ("schema", JsonValue::string("chronofork.capabilities.v1")),
            ("service_version", JsonValue::string("0.1.0")),
            ("api_version", JsonValue::int(2)),
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
        let bytes = to_canonical_bytes(&envelope, &ServiceLimits::default()).expect("canonical");
        let view = crate::view::decode_capabilities(&bytes).expect("capabilities view");
        *view.backend().capabilities()
    }

    /// A deterministic test-pattern source: a fixture, not production code.
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
    }

    impl VideoSource for TestPatternSource {
        fn frame(&self, sequence: u64) -> Result<PresentationFrame, GuiError> {
            self.calls.lock().expect("calls lock").push(sequence);
            let pixels = usize::try_from(u64::from(self.width) * u64::from(self.height))
                .expect("pixel count");
            let mut rgba = Vec::with_capacity(pixels * 4);
            for index in 0..pixels {
                let value = u8::try_from((u64::try_from(index).expect("index") + sequence) % 256)
                    .expect("byte");
                rgba.extend_from_slice(&[value, value, value, 0xff]);
            }
            PresentationFrame::new(self.width, self.height, rgba, sequence)
        }
    }

    /// A recording sink: a fixture, not production code.
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
        AudioChunk::new(44_100, vec![1, -2, 3, -4], sequence).expect("audio chunk")
    }

    #[test]
    fn presentation_reasons_are_the_documented_closed_set() {
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

        let available: Availability = Presentation::available(());
        assert!(available.is_available());
        assert_eq!(available.reason(), None);
        assert_eq!(available.value(), Some(()));

        let unavailable: Availability = Presentation::unavailable(UnavailableReason::NoMovie);
        assert!(!unavailable.is_available());
        assert_eq!(unavailable.reason(), Some(UnavailableReason::NoMovie));
        assert_eq!(unavailable.value(), None);
        assert_eq!(
            Presentation::<u8>::unavailable(UnavailableReason::NoSession),
            Presentation::Unavailable(UnavailableReason::NoSession)
        );
        assert_eq!(Presentation::available(7_u8), Presentation::Available(7));
    }

    #[test]
    fn the_video_pipeline_never_calls_a_source_when_the_capability_is_disabled() {
        let source = Arc::new(TestPatternSource::new(2, 2));
        let pipeline = VideoPipeline::new(Some(source.clone()));
        let disabled = capabilities(false, false);

        assert_eq!(
            pipeline.availability(&disabled),
            Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
        );
        assert_eq!(
            pipeline.frame(&disabled, 0).expect("gate"),
            Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
        );
        assert!(
            source.calls().is_empty(),
            "a disabled capability calls nothing"
        );
    }

    #[test]
    fn the_video_pipeline_reports_no_output_device_without_a_source() {
        let pipeline = VideoPipeline::new(None);
        let enabled = capabilities(true, true);

        assert_eq!(
            pipeline.availability(&enabled),
            Presentation::Unavailable(UnavailableReason::NoOutputDevice)
        );
        assert_eq!(
            pipeline.frame(&enabled, 3).expect("gate"),
            Presentation::Unavailable(UnavailableReason::NoOutputDevice)
        );
        // The v1 configuration: the descriptor advertises rendering and the process still
        // reports the surface as unavailable, because there is no pixel device.
        assert!(!pipeline.availability(&enabled).is_available());
    }

    #[test]
    fn the_video_pipeline_produces_deterministic_frames_from_its_source() {
        let source = Arc::new(TestPatternSource::new(2, 3));
        let pipeline = VideoPipeline::new(Some(source.clone()));
        let enabled = capabilities(true, false);

        assert_eq!(pipeline.availability(&enabled), Presentation::available(()));
        let first = pipeline
            .frame(&enabled, 5)
            .expect("gate")
            .value()
            .expect("available frame");
        assert_eq!(first.width(), 2);
        assert_eq!(first.height(), 3);
        assert_eq!(first.sequence(), 5);
        assert_eq!(first.rgba().len(), 2 * 3 * 4);

        let second = VideoPipeline::new(Some(source.clone()))
            .frame(&enabled, 5)
            .expect("gate")
            .value()
            .expect("available frame");
        assert_eq!(first.digest(), second.digest(), "frames are deterministic");
        assert_eq!(first.rgba(), second.rgba());
        assert_eq!(source.calls(), [5, 5]);
    }

    #[test]
    fn an_oversized_frame_is_a_typed_limit_not_an_allocation() {
        let error =
            PresentationFrame::new(1025, 1024, Vec::new(), 0).expect_err("over the pixel bound");
        assert_eq!(error.code(), crate::GuiErrorCode::LimitExceeded);
        assert!(matches!(
            error,
            GuiError::LimitExceeded {
                what: "presentation frame pixels",
                ..
            }
        ));
    }

    #[test]
    fn the_audio_pipeline_never_writes_when_the_capability_is_disabled() {
        let sink = Arc::new(RecordingSink::new());
        let pipeline = AudioPipeline::new(Some(sink.clone()));
        let disabled = capabilities(false, false);

        assert_eq!(
            pipeline.availability(&disabled),
            Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
        );
        assert_eq!(
            pipeline.write(&disabled, &chunk(0)).expect("gate"),
            Presentation::Unavailable(UnavailableReason::CapabilityDisabled)
        );
        assert!(
            sink.chunks().is_empty(),
            "a disabled capability writes nothing"
        );
    }

    #[test]
    fn the_audio_pipeline_reports_no_output_device_without_a_sink() {
        let pipeline = AudioPipeline::new(None);
        let enabled = capabilities(false, true);

        assert_eq!(
            pipeline.availability(&enabled),
            Presentation::Unavailable(UnavailableReason::NoOutputDevice)
        );
        assert_eq!(
            pipeline.write(&enabled, &chunk(1)).expect("gate"),
            Presentation::Unavailable(UnavailableReason::NoOutputDevice)
        );
    }

    #[test]
    fn the_audio_pipeline_passes_deterministic_chunks_through_unchanged() {
        let sink = Arc::new(RecordingSink::new());
        let pipeline = AudioPipeline::new(Some(sink.clone()));
        let enabled = capabilities(false, true);

        assert_eq!(pipeline.availability(&enabled), Presentation::available(()));
        assert_eq!(
            pipeline.write(&enabled, &chunk(9)).expect("write"),
            Presentation::available(())
        );
        assert_eq!(
            pipeline.write(&enabled, &chunk(9)).expect("write"),
            Presentation::available(())
        );

        let chunks = sink.chunks();
        assert_eq!(
            chunks.len(),
            2,
            "the sink receives exactly one write per chunk"
        );
        assert_eq!(chunks[0], chunks[1], "identical chunks stay identical");
        assert_eq!(chunks[0], chunk(9), "the chunk is not transformed");
        assert_eq!(chunks[0].samples(), [1, -2, 3, -4]);
        assert_eq!(chunks[0].sample_rate(), 44_100);
        assert_eq!(chunks[0].sequence(), 9);
        assert_eq!(chunks[0].digest(), chunks[1].digest());
    }

    #[test]
    fn an_oversized_audio_chunk_is_a_typed_limit() {
        let samples = vec![0_i16; MAX_AUDIO_SAMPLES + 1];
        let error = AudioChunk::new(44_100, samples, 0).expect_err("over the sample bound");
        assert_eq!(error.code(), crate::GuiErrorCode::LimitExceeded);
        assert!(matches!(
            error,
            GuiError::LimitExceeded {
                what: "audio chunk samples",
                ..
            }
        ));
    }

    #[test]
    fn panel_availability_is_the_documented_matrix() {
        for kind in PanelKind::ALL {
            let no_session = panel_availability(kind, false, false);
            let session_no_movie = panel_availability(kind, true, false);
            let session_movie = panel_availability(kind, true, true);
            match kind {
                PanelKind::EventLog => {
                    assert!(no_session.is_available(), "events need no session");
                    assert!(session_no_movie.is_available());
                    assert!(session_movie.is_available());
                }
                PanelKind::Timeline => {
                    assert_eq!(
                        no_session.reason(),
                        Some(UnavailableReason::NoMovie),
                        "timeline without --movie reports no_movie"
                    );
                    assert_eq!(session_no_movie.reason(), Some(UnavailableReason::NoMovie));
                    assert!(session_movie.is_available());
                }
                _ => {
                    assert_eq!(
                        no_session.reason(),
                        Some(UnavailableReason::NoSession),
                        "{} without a pane reports no_session",
                        kind.name()
                    );
                    assert!(session_no_movie.is_available());
                    assert!(session_movie.is_available());
                }
            }
        }
    }
}
