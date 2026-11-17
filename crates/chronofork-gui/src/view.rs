//! Immutable canonical views, their strict decoders and the presentation contracts.
//!
//! The model holds **copies of values**: integers, strings and vectors. No view contains
//! a handle, a lifetime or a reference into a service or a backend, so a view cannot
//! outlive or alias canonical state (ADR-0013 §3.2, `gui.md` §2). Decoding is strict and
//! field-exact: a missing, mistyped or unknown field is a typed [`GuiError::Protocol`],
//! never a default value, and a response that names a different operation is rejected
//! before any field is read.
//!
//! The operations consumed here are exactly the documented read set — `capabilities`,
//! `session.list`, `session.inspect`, `session.read_memory` — plus the two authorized
//! mutations `session.create` and `session.run`. Two consequences are normative:
//!
//! 1. **`session.create` is not a descriptor.** It returns the session, its state, tick,
//!    state hash, lineage and a `backend_id` ([`CreateView`]). The full descriptor and
//!    the capability flags come from `session.inspect` ([`SessionView`]) or
//!    `capabilities` ([`CapabilityView`]), so a `session.create` result carrying a
//!    `backend` object is rejected as an unknown field rather than read as a descriptor.
//! 2. **The per-session descriptor gates that session's presentation.** `rendering` and
//!    `audio` from [`BackendCapabilities`] are the gate; the pipeline layer resolves
//!    `Available`/`Unavailable` from them and never fabricates a picture or a sound.
//!
//! The bounded presentation value types ([`PresentationFrame`], [`AudioChunk`]) and the
//! [`VideoSource`]/[`AudioSink`] contracts live here because they are the vocabulary both
//! the presentation pipeline and its tests share; the capability gate itself is
//! [`crate::presentation`].

use std::collections::BTreeMap;

use chronofork_core::{StateHash, hash_bytes};
use chronofork_service::json::{JsonValue, MAX_INTEGER_U64};

use crate::channel::{ObjectView, parse_response, result_for};
use crate::{GuiError, MAX_AUDIO_SAMPLES, MAX_PRESENTATION_PIXELS, codec_limits};

/// The schema string a `capabilities` result must carry.
const CAPABILITIES_SCHEMA: &str = "chronofork.capabilities.v1";

/// Declares the published capability table and the names the strict decoder accepts.
///
/// One declaration is the **single source of truth** for every capability name: it expands
/// into the table the decoder reads (field set and value) and the table
/// `BackendCapabilities::flags` renders, so a published name, an accepted name and a
/// rendered name cannot drift apart.
macro_rules! capability_table {
    ($( $name:literal => $field:ident ),+ $(,)?) => {
        /// One published capability flag: its name, the field it reads and the field it writes.
        struct CapabilityFlag {
            /// The published name, which is also the descriptor field name.
            name: &'static str,
            /// Reads the flag from a decoded descriptor.
            read: fn(&BackendCapabilities) -> bool,
            /// Writes the flag while decoding a descriptor.
            write: fn(&mut BackendCapabilities, bool),
        }

        /// The capability flags of a backend descriptor, in published order.
        ///
        /// The table is generated from the one `capability_table!` declaration, and both
        /// the strict decoder and `BackendCapabilities::flags` consume it.
        const CAPABILITY_FLAGS: &[CapabilityFlag] = &[
            $(
                CapabilityFlag {
                    name: $name,
                    read: |capabilities| capabilities.$field,
                    write: |capabilities, value| capabilities.$field = value,
                },
            )+
        ];

        /// The capability names the strict decoder accepts, derived from the table.
        const CAPABILITY_FIELDS: [&str; CAPABILITY_FLAGS.len()] = {
            let mut names = [""; CAPABILITY_FLAGS.len()];
            let mut index = 0;
            while index < CAPABILITY_FLAGS.len() {
                names[index] = CAPABILITY_FLAGS[index].name;
                index += 1;
            }
            names
        };
    };
}

capability_table! {
    "execution" => execution,
    "save_states" => save_states,
    "memory_read" => memory_read,
    "memory_write" => memory_write,
    "register_inspection" => register_inspection,
    "instruction_stepping" => instruction_stepping,
    "breakpoints" => breakpoints,
    "watchpoints" => watchpoints,
    "debug_events" => debug_events,
    "tracing" => tracing,
    "state_cloning" => state_cloning,
    "deterministic_replay" => deterministic_replay,
    "rendering" => rendering,
    "audio" => audio,
}

/// The service limit fields of a `capabilities` result, in published order.
const LIMIT_FIELDS: [&str; 8] = [
    "max_request_bytes",
    "max_response_bytes",
    "max_steps_per_request",
    "max_memory_read",
    "max_sessions",
    "max_store_entries",
    "max_checkpoints",
    "json_max_depth",
];

/// The lifecycle state a session reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    /// The session accepts input and can advance.
    Ready,
    /// The session is paused by the session layer.
    Paused,
    /// The session is closed.
    Closed,
}

impl SessionState {
    /// Returns the canonical wire name of this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Paused => "paused",
            Self::Closed => "closed",
        }
    }

    /// Decodes the canonical wire name, rejecting anything else.
    fn from_wire(name: &str) -> Result<Self, GuiError> {
        match name {
            "ready" => Ok(Self::Ready),
            "paused" => Ok(Self::Paused),
            "closed" => Ok(Self::Closed),
            other => Err(GuiError::protocol(&format!(
                "session state `{other}` is not a documented state"
            ))),
        }
    }
}

/// The lineage of one session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lineage {
    parent_session: Option<u64>,
    origin_snapshot: Option<u64>,
    depth: u64,
}

impl Lineage {
    /// Returns the parent session id, when the session was forked.
    #[must_use]
    pub const fn parent_session(&self) -> Option<u64> {
        self.parent_session
    }

    /// Returns the snapshot the session originates from, when it was restored.
    #[must_use]
    pub const fn origin_snapshot(&self) -> Option<u64> {
        self.origin_snapshot
    }

    /// Returns the lineage depth (`0` for a root session).
    #[must_use]
    pub const fn depth(&self) -> u64 {
        self.depth
    }
}

/// The capability flags a backend descriptor publishes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendCapabilities {
    execution: bool,
    save_states: bool,
    memory_read: bool,
    memory_write: bool,
    register_inspection: bool,
    instruction_stepping: bool,
    breakpoints: bool,
    watchpoints: bool,
    debug_events: bool,
    tracing: bool,
    state_cloning: bool,
    deterministic_replay: bool,
    rendering: bool,
    audio: bool,
}

impl BackendCapabilities {
    /// Number of published capability flags.
    pub const FIELD_COUNT: usize = CAPABILITY_FLAGS.len();

    /// Returns every capability flag in published order.
    ///
    /// The names and the order come from the one generated `CAPABILITY_FLAGS` table, so a
    /// capabilities panel renders the flags without holding a second list and adding or
    /// renaming a flag changes exactly one declaration.
    #[must_use]
    pub fn flags(&self) -> [(&'static str, bool); Self::FIELD_COUNT] {
        std::array::from_fn(|index| {
            let flag = &CAPABILITY_FLAGS[index];
            (flag.name, (flag.read)(self))
        })
    }

    /// Returns every capability disabled: the base the strict decoder fills in.
    ///
    /// The decoder writes each flag through the table, so no capability value is ever
    /// defaulted when a descriptor field is missing or mistyped.
    const fn all_disabled() -> Self {
        Self {
            execution: false,
            save_states: false,
            memory_read: false,
            memory_write: false,
            register_inspection: false,
            instruction_stepping: false,
            breakpoints: false,
            watchpoints: false,
            debug_events: false,
            tracing: false,
            state_cloning: false,
            deterministic_replay: false,
            rendering: false,
            audio: false,
        }
    }

    /// Returns whether the backend can produce presentation frames.
    ///
    /// This flag, taken from the canonical descriptor, is the video gate: `false` yields
    /// `Unavailable{CapabilityDisabled}` and no source is ever called.
    #[must_use]
    pub const fn rendering(&self) -> bool {
        self.rendering
    }

    /// Returns whether the backend can produce audio samples.
    ///
    /// This flag is the audio gate, exactly as [`Self::rendering`] is the video gate.
    #[must_use]
    pub const fn audio(&self) -> bool {
        self.audio
    }
}

/// The backend descriptor of one session or of the whole service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendView {
    id: String,
    display_name: String,
    version: String,
    test_backend: bool,
    capabilities: BackendCapabilities,
}

impl BackendView {
    /// Returns the stable backend identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the human-readable backend name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the backend version string.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns whether the backend is the deterministic synthetic test backend.
    #[must_use]
    pub const fn test_backend(&self) -> bool {
        self.test_backend
    }

    /// Returns the backend capability flags.
    #[must_use]
    pub const fn capabilities(&self) -> &BackendCapabilities {
        &self.capabilities
    }
}

/// The resource limits a service advertises.
///
/// The GUI reads `max_steps_per_request` from here instead of duplicating the bound, so a
/// `step`/`run` batch the service would reject is rejected before the call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsView {
    max_request_bytes: u64,
    max_response_bytes: u64,
    max_steps_per_request: u64,
    max_memory_read: u64,
    max_sessions: u64,
    max_store_entries: u64,
    max_checkpoints: u64,
    json_max_depth: u64,
}

impl LimitsView {
    /// Returns the maximum encoded request size in bytes.
    #[must_use]
    pub const fn max_request_bytes(&self) -> u64 {
        self.max_request_bytes
    }

    /// Returns the maximum canonical response size in bytes.
    #[must_use]
    pub const fn max_response_bytes(&self) -> u64 {
        self.max_response_bytes
    }

    /// Returns the maximum step count one `session.run` may advance.
    #[must_use]
    pub const fn max_steps_per_request(&self) -> u64 {
        self.max_steps_per_request
    }

    /// Returns the maximum byte count one memory read may return.
    #[must_use]
    pub const fn max_memory_read(&self) -> u64 {
        self.max_memory_read
    }

    /// Returns the maximum number of registered sessions.
    #[must_use]
    pub const fn max_sessions(&self) -> u64 {
        self.max_sessions
    }

    /// Returns the maximum number of durable entries one list may carry.
    #[must_use]
    pub const fn max_store_entries(&self) -> u64 {
        self.max_store_entries
    }

    /// Returns the maximum number of experiment checkpoints.
    #[must_use]
    pub const fn max_checkpoints(&self) -> u64 {
        self.max_checkpoints
    }

    /// Returns the maximum JSON nesting depth.
    #[must_use]
    pub const fn json_max_depth(&self) -> u64 {
        self.json_max_depth
    }
}

/// The service-level capabilities view fetched once at startup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityView {
    service_version: String,
    backend: BackendView,
    limits: LimitsView,
}

impl CapabilityView {
    /// Returns the service crate version.
    #[must_use]
    pub fn service_version(&self) -> &str {
        &self.service_version
    }

    /// Returns the service-level backend descriptor.
    #[must_use]
    pub const fn backend(&self) -> &BackendView {
        &self.backend
    }

    /// Returns the advertised resource limits.
    #[must_use]
    pub const fn limits(&self) -> &LimitsView {
        &self.limits
    }
}

/// One row of `session.list`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    session: u64,
    state: SessionState,
    tick: u64,
    state_hash: StateHash,
    lineage: Lineage,
}

impl SessionSummary {
    /// Returns the session id.
    #[must_use]
    pub const fn session(&self) -> u64 {
        self.session
    }

    /// Returns the lifecycle state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns the emulated tick.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Returns the canonical state hash.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.state_hash
    }

    /// Returns the session lineage.
    #[must_use]
    pub const fn lineage(&self) -> Lineage {
        self.lineage
    }
}

/// The `session.list` result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionList {
    sessions: Vec<SessionSummary>,
    truncated: bool,
}

impl SessionList {
    /// Returns the listed sessions in ascending session id order.
    #[must_use]
    pub fn sessions(&self) -> &[SessionSummary] {
        &self.sessions
    }

    /// Returns whether the service truncated the list at its own bound.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

/// The machine observation of one session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineView {
    tick: u64,
    state_hash: StateHash,
    halted: bool,
    registers: BTreeMap<String, u64>,
    memory_base: u64,
    memory_length: u64,
}

impl MachineView {
    /// Returns the emulated tick.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Returns the canonical state hash.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.state_hash
    }

    /// Returns whether the machine halted.
    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }

    /// Returns the inspection registers in the service's sorted order.
    ///
    /// Every value is a non-negative integer inside the portable JSON range (a mistyped
    /// or negative value is a typed rejection at decode time), and the map as a whole is
    /// bounded by the canonical response byte limit, so a panel can never hold a register
    /// outside the canonical range.
    #[must_use]
    pub fn registers(&self) -> &BTreeMap<String, u64> {
        &self.registers
    }

    /// Returns the base address of the memory window the machine exposes.
    #[must_use]
    pub const fn memory_base(&self) -> u64 {
        self.memory_base
    }

    /// Returns the length in bytes of the memory window the machine exposes.
    #[must_use]
    pub const fn memory_length(&self) -> u64 {
        self.memory_length
    }
}

/// The `session.inspect` result and the only source of a per-session descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionView {
    session: u64,
    state: SessionState,
    lineage: Lineage,
    backend: BackendView,
    machine: MachineView,
}

impl SessionView {
    /// Returns the session id.
    #[must_use]
    pub const fn session(&self) -> u64 {
        self.session
    }

    /// Returns the lifecycle state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns the session lineage.
    #[must_use]
    pub const fn lineage(&self) -> Lineage {
        self.lineage
    }

    /// Returns the per-session backend descriptor.
    #[must_use]
    pub const fn backend(&self) -> &BackendView {
        &self.backend
    }

    /// Returns the machine observation.
    #[must_use]
    pub const fn machine(&self) -> &MachineView {
        &self.machine
    }

    /// Applies the values a `session.run` result reports for this session.
    ///
    /// A run reports the performed step count, the new tick, the new state hash and the
    /// halt flag, and nothing else; the lifecycle state, lineage, descriptor, registers
    /// and memory window stay those of the last observation, so no value is invented for
    /// a field the operation does not return.
    pub(crate) fn apply_run(&mut self, run: &RunView) {
        self.machine.tick = run.tick;
        self.machine.state_hash = run.state_hash;
        self.machine.halted = run.halted;
    }
}

/// The `session.read_memory` result: one bounded memory window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryView {
    session: u64,
    address: u64,
    length: u64,
    bytes: Vec<u8>,
}

impl MemoryView {
    /// Returns the session id.
    #[must_use]
    pub const fn session(&self) -> u64 {
        self.session
    }

    /// Returns the first address of the window.
    #[must_use]
    pub const fn address(&self) -> u64 {
        self.address
    }

    /// Returns the number of bytes in the window.
    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// Returns the window bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The `session.create` result.
///
/// It carries a `backend_id`, **not** a descriptor: the full descriptor and its
/// capability flags come from [`SessionView`] or [`CapabilityView`] (ADR-0013 §3.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateView {
    session: u64,
    backend_id: String,
    state: SessionState,
    tick: u64,
    state_hash: StateHash,
    lineage: Lineage,
}

impl CreateView {
    /// Returns the new session id.
    #[must_use]
    pub const fn session(&self) -> u64 {
        self.session
    }

    /// Returns the backend identifier the session was created on.
    #[must_use]
    pub fn backend_id(&self) -> &str {
        &self.backend_id
    }

    /// Returns the lifecycle state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns the emulated tick.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Returns the canonical state hash.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.state_hash
    }

    /// Returns the new session lineage.
    #[must_use]
    pub const fn lineage(&self) -> Lineage {
        self.lineage
    }
}

/// The `session.run` result.
///
/// `steps` is the count the service actually performed, which stops at a halt; the
/// requested count is a property of the request, not of the result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunView {
    session: u64,
    steps: u64,
    tick: u64,
    state_hash: StateHash,
    halted: bool,
}

impl RunView {
    /// Returns the session id.
    #[must_use]
    pub const fn session(&self) -> u64 {
        self.session
    }

    /// Returns the number of steps the service performed.
    #[must_use]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// Returns the emulated tick after the run.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Returns the canonical state hash after the run.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.state_hash
    }

    /// Returns whether the machine halted during the run.
    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }
}

/// One anchor of a TAS timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineAnchor {
    step: u64,
    tick: u64,
}

impl TimelineAnchor {
    /// Creates one anchor.
    #[must_use]
    pub const fn new(step: u64, tick: u64) -> Self {
        Self { step, tick }
    }

    /// Returns the movie step this anchor marks.
    #[must_use]
    pub const fn step(&self) -> u64 {
        self.step
    }

    /// Returns the emulated tick this anchor marks.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }
}

/// The TAS timeline the CLI reads read-only from a `.cfmovie` and passes in.
///
/// The GUI crate cannot open a file and must not depend on `chronofork-sessions`, so the
/// composition root owns the bounded movie loader and this value is its plain projection:
/// the format version, the step count, the end tick and the anchors (`gui.md` §2).
/// Sequence and value bounds are validated here, so a drifted producer cannot inject an
/// out-of-range or non-monotonic timeline into a panel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineView {
    format_version: u64,
    steps: u64,
    end_tick: u64,
    anchors: Vec<TimelineAnchor>,
}

impl TimelineView {
    /// Creates a timeline view from an already-loaded movie projection.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::LimitExceeded`] when a value is outside the portable JSON
    /// integer range and [`GuiError::BadValue`] when an anchor is not strictly increasing
    /// in movie step, decreases in tick, or points beyond the timeline's step count or end
    /// tick. The anchor count itself is bounded by the CLI's bounded movie loader, which
    /// is the one owner of that bound.
    pub fn new(
        format_version: u64,
        steps: u64,
        end_tick: u64,
        anchors: Vec<TimelineAnchor>,
    ) -> Result<Self, GuiError> {
        check_canonical(format_version, "timeline format version")?;
        check_canonical(steps, "timeline steps")?;
        check_canonical(end_tick, "timeline end tick")?;
        let mut previous: Option<TimelineAnchor> = None;
        for anchor in &anchors {
            check_canonical(anchor.step, "timeline anchor step")?;
            check_canonical(anchor.tick, "timeline anchor tick")?;
            if anchor.step > steps {
                return Err(GuiError::bad_value(
                    "timeline anchors",
                    "an anchor step is beyond the timeline step count",
                ));
            }
            if anchor.tick > end_tick {
                return Err(GuiError::bad_value(
                    "timeline anchors",
                    "an anchor tick is beyond the timeline end tick",
                ));
            }
            if let Some(previous) = previous {
                if anchor.step <= previous.step {
                    return Err(GuiError::bad_value(
                        "timeline anchors",
                        "anchor steps must be strictly increasing",
                    ));
                }
                if anchor.tick < previous.tick {
                    return Err(GuiError::bad_value(
                        "timeline anchors",
                        "anchor ticks must not decrease",
                    ));
                }
            }
            previous = Some(*anchor);
        }
        Ok(Self {
            format_version,
            steps,
            end_tick,
            anchors,
        })
    }

    /// Returns the movie format version.
    #[must_use]
    pub const fn format_version(&self) -> u64 {
        self.format_version
    }

    /// Returns the movie step count.
    #[must_use]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// Returns the movie end tick.
    #[must_use]
    pub const fn end_tick(&self) -> u64 {
        self.end_tick
    }

    /// Returns the anchors in movie order.
    #[must_use]
    pub fn anchors(&self) -> &[TimelineAnchor] {
        &self.anchors
    }
}

/// One bounded RGBA8 presentation frame.
///
/// The buffer is exactly `width * height * 4` bytes, the pixel count is bounded by
/// [`MAX_PRESENTATION_PIXELS`], and [`PresentationFrame::digest`] is the SHA-256 of the
/// canonical little-endian header plus the buffer, so two frames compare byte for byte
/// like every other canonical artifact (ADR-0013 §3.6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    sequence: u64,
}

impl PresentationFrame {
    /// Creates one frame after validating its declared size.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::BadValue`] for a zero dimension or a buffer length that does
    /// not match `width * height * 4`, and [`GuiError::LimitExceeded`] when the pixel count
    /// exceeds [`MAX_PRESENTATION_PIXELS`]. The pixel bound is checked before any
    /// arithmetic that depends on the buffer length.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>, sequence: u64) -> Result<Self, GuiError> {
        if width == 0 || height == 0 {
            return Err(GuiError::bad_value(
                "presentation frame",
                "a frame needs a non-zero width and height",
            ));
        }
        let pixels = u64::from(width) * u64::from(height);
        if pixels > u64::try_from(MAX_PRESENTATION_PIXELS).unwrap_or(u64::MAX) {
            return Err(GuiError::limit_exceeded(
                "presentation frame pixels",
                u64::try_from(MAX_PRESENTATION_PIXELS).unwrap_or(u64::MAX),
            ));
        }
        let expected = pixels * 4;
        if u64::try_from(rgba.len()).unwrap_or(u64::MAX) != expected {
            return Err(GuiError::bad_value(
                "presentation frame",
                "the RGBA buffer length is not width * height * 4",
            ));
        }
        Ok(Self {
            width,
            height,
            rgba,
            sequence,
        })
    }

    /// Returns the frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the RGBA8 buffer.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Returns the presentation sequence number.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the canonical SHA-256 digest of this frame.
    #[must_use]
    pub fn digest(&self) -> StateHash {
        let mut bytes = Vec::with_capacity(16 + self.rgba.len());
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.sequence.to_le_bytes());
        bytes.extend_from_slice(&self.rgba);
        hash_bytes(&bytes)
    }
}

/// One bounded audio chunk of signed 16-bit samples.
///
/// The crate adds no resampling, mixing or device semantic: a chunk is exactly the samples
/// a source produced, bounded by [`MAX_AUDIO_SAMPLES`], and its digest is the SHA-256 of
/// the canonical little-endian header plus the samples.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioChunk {
    sample_rate: u32,
    samples: Vec<i16>,
    sequence: u64,
}

impl AudioChunk {
    /// Creates one chunk after validating its declared size.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::BadValue`] for a zero sample rate and
    /// [`GuiError::LimitExceeded`] when the sample count exceeds [`MAX_AUDIO_SAMPLES`].
    pub fn new(sample_rate: u32, samples: Vec<i16>, sequence: u64) -> Result<Self, GuiError> {
        if sample_rate == 0 {
            return Err(GuiError::bad_value(
                "audio chunk",
                "a chunk needs a non-zero sample rate",
            ));
        }
        if samples.len() > MAX_AUDIO_SAMPLES {
            return Err(GuiError::limit_exceeded(
                "audio chunk samples",
                u64::try_from(MAX_AUDIO_SAMPLES).unwrap_or(u64::MAX),
            ));
        }
        Ok(Self {
            sample_rate,
            samples,
            sequence,
        })
    }

    /// Returns the sample rate in hertz.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Returns the samples.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    /// Returns the presentation sequence number.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the canonical SHA-256 digest of this chunk.
    #[must_use]
    pub fn digest(&self) -> StateHash {
        let mut bytes = Vec::with_capacity(12 + self.samples.len().saturating_mul(2));
        bytes.extend_from_slice(&self.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&self.sequence.to_le_bytes());
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        hash_bytes(&bytes)
    }
}

/// A source of presentation frames.
///
/// The pipeline calls a source only when the canonical descriptor advertises
/// `rendering=true` **and** a source is configured; v1 configures none, so the honest
/// process result is `Unavailable{NoOutputDevice}`. Implementations are shared, so a
/// recording source uses interior mutability to count calls in tests.
pub trait VideoSource: Send + Sync {
    /// Produces the frame carrying `sequence`.
    ///
    /// # Errors
    ///
    /// Returns a typed [`GuiError`] when the source cannot produce the frame.
    fn frame(&self, sequence: u64) -> Result<PresentationFrame, GuiError>;
}

/// A consumer of audio chunks.
///
/// The pipeline writes to a sink only when the canonical descriptor advertises
/// `audio=true` **and** a sink is configured; v1 configures none because there is no
/// output device.
pub trait AudioSink: Send + Sync {
    /// Consumes one bounded chunk.
    ///
    /// # Errors
    ///
    /// Returns a typed [`GuiError`] when the sink cannot accept the chunk.
    fn write(&self, chunk: &AudioChunk) -> Result<(), GuiError>;
}

/// Decodes the `capabilities` result into the startup capability view.
///
/// # Errors
///
/// Returns [`GuiError::Protocol`] for a response that is not the documented
/// `chronofork.capabilities.v1` result, including a missing, mistyped or unknown field in
/// the result, the descriptor, the flags, the limit table, the operation catalog, the
/// store block or the artifact block.
pub fn decode_capabilities(bytes: &[u8]) -> Result<CapabilityView, GuiError> {
    let document = parse_response(bytes)?;
    let result = result_for(&document, "capabilities")?;
    let view = ObjectView::exact(
        result,
        &[
            "schema",
            "service_version",
            "backend",
            "ops",
            "limits",
            "store",
            "artifacts",
        ],
        "capabilities result",
    )?;
    let schema = view.text("schema")?;
    if schema != CAPABILITIES_SCHEMA {
        return Err(GuiError::protocol(&format!(
            "capabilities result carries schema `{schema}`"
        )));
    }
    let backend = decode_backend(view.raw("backend")?)?;
    let limits = decode_limits(view.raw("limits")?)?;
    decode_catalog(view.array("ops")?)?;
    decode_configured(view.raw("store")?, "capabilities store")?;
    decode_configured(view.raw("artifacts")?, "capabilities artifacts")?;
    Ok(CapabilityView {
        service_version: view.text("service_version")?.to_owned(),
        backend,
        limits,
    })
}

/// Decodes the `session.list` result.
///
/// # Errors
///
/// Returns [`GuiError::Protocol`] for a response that is not the documented result: the
/// strict reader rejects a missing, mistyped or unknown field in the result or in any
/// listed session.
pub fn decode_session_list(bytes: &[u8]) -> Result<SessionList, GuiError> {
    let document = parse_response(bytes)?;
    let result = result_for(&document, "session.list")?;
    let view = ObjectView::exact(result, &["sessions", "truncated"], "session.list result")?;
    let entries = view.array("sessions")?;
    let mut sessions = Vec::with_capacity(entries.len());
    for entry in entries {
        sessions.push(decode_summary(entry)?);
    }
    Ok(SessionList {
        sessions,
        truncated: view.boolean("truncated")?,
    })
}

/// Decodes the `session.inspect` result into the per-session view.
///
/// # Errors
///
/// Returns [`GuiError::Protocol`] for a response that is not the documented result,
/// including a missing `machine.registers` map (rejected, never defaulted), a malformed
/// state hash and an unknown field anywhere in the result.
pub fn decode_session_view(bytes: &[u8]) -> Result<SessionView, GuiError> {
    let document = parse_response(bytes)?;
    let result = result_for(&document, "session.inspect")?;
    let view = ObjectView::exact(
        result,
        &["session", "state", "lineage", "backend", "machine"],
        "session.inspect result",
    )?;
    Ok(SessionView {
        session: view.unsigned("session")?,
        state: SessionState::from_wire(view.text("state")?)?,
        lineage: decode_lineage(view.raw("lineage")?)?,
        backend: decode_backend(view.raw("backend")?)?,
        machine: decode_machine(view.raw("machine")?)?,
    })
}

/// Decodes the `session.read_memory` result into the bounded memory view.
///
/// # Errors
///
/// Returns [`GuiError::Protocol`] for a response that is not the documented result, for a
/// `bytes` field that is not an even-length lowercase or uppercase hex string, for a
/// `length` that disagrees with the decoded byte count, or for a window larger than the
/// service's advertised `max_memory_read`.
pub fn decode_memory(bytes: &[u8]) -> Result<MemoryView, GuiError> {
    let document = parse_response(bytes)?;
    let result = result_for(&document, "session.read_memory")?;
    let view = ObjectView::exact(
        result,
        &["session", "address", "length", "bytes"],
        "session.read_memory result",
    )?;
    let address = view.unsigned("address")?;
    let declared = view.unsigned("length")?;
    let window = parse_hex(view.text("bytes")?, "memory bytes")?;
    let actual = u64::try_from(window.len()).unwrap_or(u64::MAX);
    if actual != declared {
        return Err(GuiError::protocol(
            "memory length does not match the decoded byte count",
        ));
    }
    let bound = u64::try_from(codec_limits().max_memory_read).unwrap_or(u64::MAX);
    if actual > bound {
        return Err(GuiError::limit_exceeded("memory window bytes", bound));
    }
    let session = view.unsigned("session")?;
    Ok(MemoryView {
        session,
        address,
        length: declared,
        bytes: window,
    })
}

/// Decodes the `session.create` result.
///
/// # Errors
///
/// Returns [`GuiError::Protocol`] for a response that is not the documented result. A
/// result carrying a `backend` descriptor object is rejected as an unknown field: the
/// operation does not return a descriptor, and the GUI must not read one from it.
pub fn decode_create(bytes: &[u8]) -> Result<CreateView, GuiError> {
    let document = parse_response(bytes)?;
    let result = result_for(&document, "session.create")?;
    let view = ObjectView::exact(
        result,
        &[
            "session",
            "state",
            "tick",
            "state_hash",
            "lineage",
            "backend_id",
        ],
        "session.create result",
    )?;
    Ok(CreateView {
        session: view.unsigned("session")?,
        backend_id: view.text("backend_id")?.to_owned(),
        state: SessionState::from_wire(view.text("state")?)?,
        tick: view.unsigned("tick")?,
        state_hash: parse_state_hash(view.text("state_hash")?, "session.create state_hash")?,
        lineage: decode_lineage(view.raw("lineage")?)?,
    })
}

/// Decodes the `session.run` result.
///
/// # Errors
///
/// Returns [`GuiError::Protocol`] for a response that is not the documented result,
/// including a malformed state hash or an unknown field.
pub fn decode_run(bytes: &[u8]) -> Result<RunView, GuiError> {
    let document = parse_response(bytes)?;
    let result = result_for(&document, "session.run")?;
    let view = ObjectView::exact(
        result,
        &["session", "steps", "tick", "state_hash", "halted"],
        "session.run result",
    )?;
    Ok(RunView {
        session: view.unsigned("session")?,
        steps: view.unsigned("steps")?,
        tick: view.unsigned("tick")?,
        state_hash: parse_state_hash(view.text("state_hash")?, "session.run state_hash")?,
        halted: view.boolean("halted")?,
    })
}

fn decode_backend(value: &JsonValue) -> Result<BackendView, GuiError> {
    let view = ObjectView::exact(
        value,
        &[
            "id",
            "display_name",
            "version",
            "test_backend",
            "capabilities",
        ],
        "backend descriptor",
    )?;
    Ok(BackendView {
        id: view.text("id")?.to_owned(),
        display_name: view.text("display_name")?.to_owned(),
        version: view.text("version")?.to_owned(),
        test_backend: view.boolean("test_backend")?,
        capabilities: decode_capability_flags(view.raw("capabilities")?)?,
    })
}

fn decode_capability_flags(value: &JsonValue) -> Result<BackendCapabilities, GuiError> {
    let view = ObjectView::exact(value, &CAPABILITY_FIELDS, "backend capabilities")?;
    let mut capabilities = BackendCapabilities::all_disabled();
    for flag in CAPABILITY_FLAGS {
        (flag.write)(&mut capabilities, view.boolean(flag.name)?);
    }
    Ok(capabilities)
}

fn decode_limits(value: &JsonValue) -> Result<LimitsView, GuiError> {
    let view = ObjectView::exact(value, &LIMIT_FIELDS, "capabilities limits")?;
    Ok(LimitsView {
        max_request_bytes: view.unsigned("max_request_bytes")?,
        max_response_bytes: view.unsigned("max_response_bytes")?,
        max_steps_per_request: view.unsigned("max_steps_per_request")?,
        max_memory_read: view.unsigned("max_memory_read")?,
        max_sessions: view.unsigned("max_sessions")?,
        max_store_entries: view.unsigned("max_store_entries")?,
        max_checkpoints: view.unsigned("max_checkpoints")?,
        json_max_depth: view.unsigned("json_max_depth")?,
    })
}

/// Validates the published operation catalog without retaining it.
///
/// No panel renders the catalog, so it is validated and dropped rather than stored: the
/// check still catches a drifted service, and the model holds no state no surface uses.
fn decode_catalog(entries: &[JsonValue]) -> Result<(), GuiError> {
    for entry in entries {
        let view = ObjectView::exact(entry, &["name", "mutating"], "capabilities op")?;
        view.text("name")?;
        view.boolean("mutating")?;
    }
    Ok(())
}

/// Validates one `{"configured": bool}` service block without retaining it.
fn decode_configured(value: &JsonValue, what: &'static str) -> Result<(), GuiError> {
    let view = ObjectView::exact(value, &["configured"], what)?;
    view.boolean("configured")?;
    Ok(())
}

fn decode_summary(value: &JsonValue) -> Result<SessionSummary, GuiError> {
    let view = ObjectView::exact(
        value,
        &["session", "state", "tick", "state_hash", "lineage"],
        "session.list entry",
    )?;
    Ok(SessionSummary {
        session: view.unsigned("session")?,
        state: SessionState::from_wire(view.text("state")?)?,
        tick: view.unsigned("tick")?,
        state_hash: parse_state_hash(view.text("state_hash")?, "session.list state_hash")?,
        lineage: decode_lineage(view.raw("lineage")?)?,
    })
}

fn decode_lineage(value: &JsonValue) -> Result<Lineage, GuiError> {
    let view = ObjectView::exact(
        value,
        &["parent_session", "origin_snapshot", "depth"],
        "session lineage",
    )?;
    Ok(Lineage {
        parent_session: view.optional_unsigned("parent_session")?,
        origin_snapshot: view.optional_unsigned("origin_snapshot")?,
        depth: view.unsigned("depth")?,
    })
}

fn decode_machine(value: &JsonValue) -> Result<MachineView, GuiError> {
    let view = ObjectView::exact(
        value,
        &[
            "tick",
            "state_hash",
            "halted",
            "registers",
            "memory_base",
            "memory_length",
        ],
        "session machine",
    )?;
    Ok(MachineView {
        tick: view.unsigned("tick")?,
        state_hash: parse_state_hash(view.text("state_hash")?, "session.inspect state_hash")?,
        halted: view.boolean("halted")?,
        registers: decode_registers(view.raw("registers")?)?,
        memory_base: view.unsigned("memory_base")?,
        memory_length: view.unsigned("memory_length")?,
    })
}

fn decode_registers(value: &JsonValue) -> Result<BTreeMap<String, u64>, GuiError> {
    let fields = value
        .as_object()
        .ok_or_else(|| GuiError::protocol("machine registers is not a JSON object"))?;
    let mut registers = BTreeMap::new();
    for (name, raw) in fields {
        let register = raw.as_u64().ok_or_else(|| {
            GuiError::protocol(&format!("register `{name}` is not a non-negative integer"))
        })?;
        registers.insert(name.clone(), register);
    }
    Ok(registers)
}

fn parse_state_hash(text: &str, what: &'static str) -> Result<StateHash, GuiError> {
    if text.len() != 64 {
        return Err(GuiError::protocol(&format!(
            "{what} is not a 64-character SHA-256 digest"
        )));
    }
    let bytes = parse_hex(text, what)?;
    let mut digest = [0_u8; 32];
    for (slot, byte) in digest.iter_mut().zip(bytes) {
        *slot = byte;
    }
    Ok(StateHash::from_bytes(digest))
}

fn parse_hex(text: &str, what: &'static str) -> Result<Vec<u8>, GuiError> {
    let digits = text.as_bytes();
    if digits.len() % 2 != 0 {
        return Err(GuiError::protocol(&format!(
            "{what} has an odd number of hex digits"
        )));
    }
    let mut output = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks_exact(2) {
        let high = hex_digit(pair[0])
            .ok_or_else(|| GuiError::protocol(&format!("{what} contains a non-hex character")))?;
        let low = hex_digit(pair[1])
            .ok_or_else(|| GuiError::protocol(&format!("{what} contains a non-hex character")))?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn check_canonical(value: u64, what: &'static str) -> Result<(), GuiError> {
    if value > MAX_INTEGER_U64 {
        return Err(GuiError::limit_exceeded(what, MAX_INTEGER_U64));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chronofork_service::ServiceLimits;
    use chronofork_service::json::{JsonValue, to_canonical_bytes};

    use super::{
        AudioChunk, CAPABILITY_FIELDS, PresentationFrame, SessionState, TimelineAnchor,
        TimelineView, decode_capabilities, decode_create, decode_memory, decode_run,
        decode_session_list, decode_session_view,
    };
    use crate::{GuiError, GuiErrorCode, MAX_AUDIO_SAMPLES, MAX_PRESENTATION_PIXELS};

    const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";

    fn envelope(op: &str, result: JsonValue) -> Vec<u8> {
        to_canonical_bytes(
            &JsonValue::object([("op", JsonValue::string(op)), ("result", result)]),
            &ServiceLimits::default(),
        )
        .expect("canonical envelope")
    }

    fn empty_object() -> JsonValue {
        JsonValue::Object(BTreeMap::new())
    }

    fn protocol_code<T: std::fmt::Debug>(result: Result<T, GuiError>) -> GuiErrorCode {
        result.expect_err("document must be rejected").code()
    }

    fn flags(
        configure: impl FnOnce(&mut BTreeMap<String, JsonValue>),
    ) -> BTreeMap<String, JsonValue> {
        let mut flags = BTreeMap::new();
        for name in CAPABILITY_FIELDS {
            let enabled = name != "rendering" && name != "audio";
            flags.insert(name.to_owned(), JsonValue::from_bool(enabled));
        }
        configure(&mut flags);
        flags
    }

    fn backend(configure: impl FnOnce(&mut BTreeMap<String, JsonValue>)) -> JsonValue {
        JsonValue::object([
            ("id", JsonValue::string("synthetic")),
            ("display_name", JsonValue::string("Synthetic")),
            ("version", JsonValue::string("0.1.0")),
            ("test_backend", JsonValue::from_bool(true)),
            ("capabilities", JsonValue::Object(flags(configure))),
        ])
    }

    fn capabilities_document(
        top: impl FnOnce(&mut BTreeMap<String, JsonValue>),
        flag_config: impl FnOnce(&mut BTreeMap<String, JsonValue>),
    ) -> Vec<u8> {
        let mut fields = BTreeMap::new();
        fields.insert(
            "schema".to_owned(),
            JsonValue::string("chronofork.capabilities.v1"),
        );
        fields.insert("service_version".to_owned(), JsonValue::string("0.1.0"));
        fields.insert("backend".to_owned(), backend(flag_config));
        fields.insert(
            "ops".to_owned(),
            JsonValue::array([JsonValue::object([
                ("name", JsonValue::string("capabilities")),
                ("mutating", JsonValue::from_bool(false)),
            ])]),
        );
        fields.insert(
            "limits".to_owned(),
            JsonValue::object([
                ("max_request_bytes", JsonValue::int(1_048_576)),
                ("max_response_bytes", JsonValue::int(1_048_576)),
                ("max_steps_per_request", JsonValue::int(100_000)),
                ("max_memory_read", JsonValue::int(4096)),
                ("max_sessions", JsonValue::int(1024)),
                ("max_store_entries", JsonValue::int(256)),
                ("max_checkpoints", JsonValue::int(32)),
                ("json_max_depth", JsonValue::int(32)),
            ]),
        );
        fields.insert(
            "store".to_owned(),
            JsonValue::object([("configured", JsonValue::from_bool(false))]),
        );
        fields.insert(
            "artifacts".to_owned(),
            JsonValue::object([("configured", JsonValue::from_bool(false))]),
        );
        top(&mut fields);
        envelope("capabilities", JsonValue::Object(fields))
    }

    fn lineage() -> JsonValue {
        JsonValue::object([
            ("parent_session", JsonValue::Null),
            ("origin_snapshot", JsonValue::Null),
            ("depth", JsonValue::int(0)),
        ])
    }

    fn machine_document(configure: impl FnOnce(&mut BTreeMap<String, JsonValue>)) -> JsonValue {
        let mut fields = BTreeMap::new();
        fields.insert("tick".to_owned(), JsonValue::int(3));
        fields.insert("state_hash".to_owned(), JsonValue::string(HASH));
        fields.insert("halted".to_owned(), JsonValue::from_bool(false));
        fields.insert(
            "registers".to_owned(),
            JsonValue::object([("d0", JsonValue::int(0)), ("pc", JsonValue::int(1024))]),
        );
        fields.insert("memory_base".to_owned(), JsonValue::int(4096));
        fields.insert("memory_length".to_owned(), JsonValue::int(0));
        configure(&mut fields);
        JsonValue::Object(fields)
    }

    fn inspect_document(
        machine: impl FnOnce(&mut BTreeMap<String, JsonValue>),
        top: impl FnOnce(&mut BTreeMap<String, JsonValue>),
    ) -> Vec<u8> {
        let mut fields = BTreeMap::new();
        fields.insert("session".to_owned(), JsonValue::int(1));
        fields.insert("state".to_owned(), JsonValue::string("ready"));
        fields.insert("lineage".to_owned(), lineage());
        fields.insert("backend".to_owned(), backend(|_| ()));
        fields.insert("machine".to_owned(), machine_document(machine));
        top(&mut fields);
        envelope("session.inspect", JsonValue::Object(fields))
    }

    fn memory_document(length: u64, hex: &str) -> Vec<u8> {
        envelope(
            "session.read_memory",
            JsonValue::object([
                ("session", JsonValue::int(1)),
                ("address", JsonValue::int(4096)),
                (
                    "length",
                    JsonValue::int(i64::try_from(length).expect("length")),
                ),
                ("bytes", JsonValue::string(hex)),
            ]),
        )
    }

    fn summary_document(state: &str) -> JsonValue {
        JsonValue::object([
            (
                "sessions",
                JsonValue::array([JsonValue::object([
                    ("session", JsonValue::int(1)),
                    ("state", JsonValue::string(state)),
                    ("tick", JsonValue::int(3)),
                    ("state_hash", JsonValue::string(HASH)),
                    ("lineage", lineage()),
                ])]),
            ),
            ("truncated", JsonValue::from_bool(true)),
        ])
    }

    fn create_document(extra: Option<(&'static str, JsonValue)>) -> Vec<u8> {
        let mut fields = BTreeMap::new();
        fields.insert("session".to_owned(), JsonValue::int(1));
        fields.insert("backend_id".to_owned(), JsonValue::string("synthetic"));
        fields.insert("state".to_owned(), JsonValue::string("ready"));
        fields.insert("tick".to_owned(), JsonValue::int(0));
        fields.insert("state_hash".to_owned(), JsonValue::string(HASH));
        fields.insert("lineage".to_owned(), lineage());
        if let Some((name, value)) = extra {
            fields.insert(name.to_owned(), value);
        }
        envelope("session.create", JsonValue::Object(fields))
    }

    fn run_document(extra: Option<(&'static str, JsonValue)>) -> Vec<u8> {
        let mut fields = BTreeMap::new();
        fields.insert("session".to_owned(), JsonValue::int(1));
        fields.insert("steps".to_owned(), JsonValue::int(3));
        fields.insert("tick".to_owned(), JsonValue::int(3));
        fields.insert("state_hash".to_owned(), JsonValue::string(HASH));
        fields.insert("halted".to_owned(), JsonValue::from_bool(true));
        if let Some((name, value)) = extra {
            fields.insert(name.to_owned(), value);
        }
        envelope("session.run", JsonValue::Object(fields))
    }

    #[test]
    fn capabilities_view_decodes_every_documented_field() {
        let bytes = capabilities_document(|_| (), |_| ());
        let view = decode_capabilities(&bytes).expect("capabilities");
        assert_eq!(view.service_version(), "0.1.0");
        assert_eq!(view.backend().id(), "synthetic");
        assert_eq!(view.backend().display_name(), "Synthetic");
        assert_eq!(view.backend().version(), "0.1.0");
        assert!(view.backend().test_backend());
        assert!(!view.backend().capabilities().rendering());
        assert!(!view.backend().capabilities().audio());
        let capabilities = view.backend().capabilities();
        assert_eq!(
            capabilities.flags().len(),
            super::BackendCapabilities::FIELD_COUNT
        );
        let published: Vec<&str> = capabilities.flags().iter().map(|(name, _)| *name).collect();
        assert_eq!(
            published,
            CAPABILITY_FIELDS.to_vec(),
            "flags() must publish exactly the decoder's accepted field list"
        );
        assert!(!capabilities.flags()[12].1, "rendering is disabled");
        assert!(!capabilities.flags()[13].1, "audio is disabled");
        assert_eq!(view.limits().max_steps_per_request(), 100_000);
        assert_eq!(view.limits().max_memory_read(), 4096);
        assert_eq!(view.limits().max_request_bytes(), 1_048_576);
        assert_eq!(view.limits().json_max_depth(), 32);
    }

    #[test]
    fn capabilities_view_rejects_a_wrong_schema_a_missing_block_and_an_extra_field() {
        let wrong_schema = capabilities_document(
            |fields| {
                fields.insert(
                    "schema".to_owned(),
                    JsonValue::string("chronofork.capabilities.v2"),
                );
            },
            |_| (),
        );
        assert_eq!(
            protocol_code(decode_capabilities(&wrong_schema)),
            GuiErrorCode::Channel
        );

        let missing = capabilities_document(
            |fields| {
                fields.remove("store");
            },
            |_| (),
        );
        let error = decode_capabilities(&missing).expect_err("missing store block");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("missing field `store`"));

        let unknown = capabilities_document(
            |fields| {
                fields.insert("extra".to_owned(), JsonValue::int(1));
            },
            |_| (),
        );
        let error = decode_capabilities(&unknown).expect_err("extra field");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("unknown field `extra`"));

        assert_eq!(
            protocol_code(decode_capabilities(&envelope(
                "session.list",
                empty_object()
            ))),
            GuiErrorCode::Channel
        );
    }

    #[test]
    fn a_capability_name_outside_the_published_table_is_rejected() {
        let renamed = capabilities_document(
            |_| (),
            |flags| {
                flags.remove("rendering");
                flags.insert("rendering_enabled".to_owned(), JsonValue::from_bool(false));
            },
        );
        let error = decode_capabilities(&renamed).expect_err("unknown capability name");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(
            error
                .to_string()
                .contains("unknown field `rendering_enabled`"),
            "unexpected detail: {error}"
        );

        // The removed name is now missing, which is the other half of the same rule.
        let removed = capabilities_document(
            |_| (),
            |flags| {
                flags.remove("audio");
            },
        );
        let error = decode_capabilities(&removed).expect_err("missing capability name");
        assert!(error.to_string().contains("missing field `audio`"));
    }

    #[test]
    fn capabilities_view_rejects_a_mistyped_flag_and_a_missing_flag() {
        let mistyped = capabilities_document(
            |_| (),
            |flags| {
                flags.insert("rendering".to_owned(), JsonValue::int(0));
            },
        );
        let error = decode_capabilities(&mistyped).expect_err("mistyped flag");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("`rendering`"));

        let missing = capabilities_document(
            |_| (),
            |flags| {
                flags.remove("audio");
            },
        );
        let error = decode_capabilities(&missing).expect_err("missing flag");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("missing field `audio`"));

        let mistyped_limits = capabilities_document(
            |fields| {
                fields.insert("limits".to_owned(), JsonValue::int(1));
            },
            |_| (),
        );
        assert_eq!(
            protocol_code(decode_capabilities(&mistyped_limits)),
            GuiErrorCode::Channel
        );
    }

    #[test]
    fn capabilities_view_validates_the_published_catalog_and_configuration_blocks() {
        let bad_op = capabilities_document(
            |fields| {
                fields.insert(
                    "ops".to_owned(),
                    JsonValue::array([JsonValue::object([("name", JsonValue::string("x"))])]),
                );
            },
            |_| (),
        );
        assert_eq!(
            protocol_code(decode_capabilities(&bad_op)),
            GuiErrorCode::Channel
        );

        let bad_artifacts = capabilities_document(
            |fields| {
                fields.insert(
                    "artifacts".to_owned(),
                    JsonValue::object([("configured", JsonValue::int(1))]),
                );
            },
            |_| (),
        );
        assert_eq!(
            protocol_code(decode_capabilities(&bad_artifacts)),
            GuiErrorCode::Channel
        );
    }

    #[test]
    fn session_list_decodes_rows_and_keeps_the_truncated_flag() {
        let list = decode_session_list(&envelope("session.list", summary_document("ready")))
            .expect("list");
        assert!(list.truncated());
        assert_eq!(list.sessions().len(), 1);
        let summary = &list.sessions()[0];
        assert_eq!(summary.session(), 1);
        assert_eq!(summary.state(), SessionState::Ready);
        assert_eq!(summary.tick(), 3);
        assert_eq!(summary.state_hash().to_hex(), HASH);
        assert_eq!(summary.lineage().depth(), 0);
        assert_eq!(summary.lineage().parent_session(), None);
        assert_eq!(summary.lineage().origin_snapshot(), None);
    }

    #[test]
    fn a_state_name_outside_the_documented_set_is_rejected() {
        let bytes = envelope("session.list", summary_document("running"));
        let error = decode_session_list(&bytes).expect_err("unknown state");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("`running`"));
    }

    #[test]
    fn inspect_view_decodes_the_machine_descriptor_and_registers() {
        let view = decode_session_view(&inspect_document(|_| (), |_| ())).expect("session view");
        assert_eq!(view.session(), 1);
        assert_eq!(view.state(), SessionState::Ready);
        assert_eq!(view.machine().tick(), 3);
        assert_eq!(view.machine().state_hash().to_hex(), HASH);
        assert!(!view.machine().halted());
        assert_eq!(view.machine().memory_base(), 4096);
        assert_eq!(view.machine().memory_length(), 0);
        assert_eq!(view.machine().registers().len(), 2);
        assert_eq!(view.machine().registers().get("pc"), Some(&1024));
        assert!(!view.backend().capabilities().rendering());
    }

    #[test]
    fn inspect_view_rejects_a_missing_registers_map_and_a_mistyped_register() {
        let missing = inspect_document(
            |machine| {
                machine.remove("registers");
            },
            |_| (),
        );
        let error = decode_session_view(&missing).expect_err("missing registers");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("missing field `registers`"));

        let mistyped = inspect_document(
            |machine| {
                machine.insert(
                    "registers".to_owned(),
                    JsonValue::object([("pc", JsonValue::string("1024"))]),
                );
            },
            |_| (),
        );
        let error = decode_session_view(&mistyped).expect_err("mistyped register");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("`pc`"));
    }

    #[test]
    fn inspect_view_rejects_a_malformed_state_hash_and_an_unknown_field() {
        let malformed: Vec<String> = vec![
            "c79d7afa".to_owned(),
            "g".repeat(64),
            format!("{HASH}00"),
            String::new(),
        ];
        for hash in malformed {
            let bytes = inspect_document(
                |machine| {
                    machine.insert("state_hash".to_owned(), JsonValue::string(hash.clone()));
                },
                |_| (),
            );
            let error = decode_session_view(&bytes).expect_err("malformed state hash");
            assert_eq!(error.code(), GuiErrorCode::Channel);
            assert!(
                error.to_string().contains("state_hash"),
                "unexpected detail: {error}"
            );
        }

        let unknown = inspect_document(
            |_| (),
            |fields| {
                fields.insert("extra".to_owned(), JsonValue::int(1));
            },
        );
        let error = decode_session_view(&unknown).expect_err("unknown field");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("unknown field `extra`"));
    }

    #[test]
    fn state_hashes_are_canonical_lowercase_hex() {
        let uppercase = inspect_document(
            |machine| {
                machine.insert(
                    "state_hash".to_owned(),
                    JsonValue::string(HASH.to_uppercase()),
                );
            },
            |_| (),
        );
        let view = decode_session_view(&uppercase).expect("uppercase hex is the same digest");
        assert_eq!(view.machine().state_hash().to_hex(), HASH);
    }

    #[test]
    fn memory_view_decodes_hex_and_enforces_the_length_contract() {
        let view = decode_memory(&memory_document(4, "000fa5ff")).expect("memory");
        assert_eq!(view.session(), 1);
        assert_eq!(view.address(), 4096);
        assert_eq!(view.length(), 4);
        assert_eq!(view.bytes(), &[0x00, 0x0F, 0xA5, 0xFF]);

        for (length, hex) in [(2, "000fa5ff"), (4, "000fa5f"), (4, "000fa5fg")] {
            let error = decode_memory(&memory_document(length, hex)).expect_err("invalid window");
            assert_eq!(error.code(), GuiErrorCode::Channel);
        }
    }

    #[test]
    fn memory_view_is_bounded_by_the_service_read_limit() {
        let length = ServiceLimits::default().max_memory_read;
        let hex = "00".repeat(length);
        decode_memory(&memory_document(
            u64::try_from(length).expect("length"),
            &hex,
        ))
        .expect("window at the limit");

        let oversized = length + 2;
        let hex = "00".repeat(oversized);
        let error = decode_memory(&memory_document(
            u64::try_from(oversized).expect("length"),
            &hex,
        ))
        .expect_err("window above the limit");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    }

    #[test]
    fn create_view_carries_a_backend_id_and_never_a_descriptor() {
        let view = decode_create(&create_document(None)).expect("create");
        assert_eq!(view.session(), 1);
        assert_eq!(view.backend_id(), "synthetic");
        assert_eq!(view.state(), SessionState::Ready);
        assert_eq!(view.tick(), 0);
        assert_eq!(view.state_hash().to_hex(), HASH);
        assert_eq!(view.lineage().depth(), 0);

        let error = decode_create(&create_document(Some(("backend", backend(|_| ())))))
            .expect_err("a create result is not a descriptor");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("unknown field `backend`"));
    }

    #[test]
    fn run_view_reports_the_performed_steps_and_the_halt_flag() {
        let view = decode_run(&run_document(None)).expect("run");
        assert_eq!(view.session(), 1);
        assert_eq!(view.steps(), 3);
        assert_eq!(view.tick(), 3);
        assert!(view.halted());
        assert_eq!(view.state_hash().to_hex(), HASH);
    }

    #[test]
    fn a_view_never_reads_a_field_the_operation_does_not_return() {
        let error = decode_run(&run_document(Some(("state", JsonValue::string("ready")))))
            .expect_err("extra field");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert!(error.to_string().contains("unknown field `state`"));
    }

    #[test]
    fn timeline_view_validates_monotonic_anchors_and_bounds() {
        let view = TimelineView::new(
            1,
            10,
            99,
            vec![TimelineAnchor::new(0, 0), TimelineAnchor::new(5, 50)],
        )
        .expect("timeline");
        assert_eq!(view.format_version(), 1);
        assert_eq!(view.steps(), 10);
        assert_eq!(view.end_tick(), 99);
        assert_eq!(view.anchors().len(), 2);
        assert_eq!(view.anchors()[1].step(), 5);
        assert_eq!(view.anchors()[1].tick(), 50);

        let cases = [
            (
                vec![TimelineAnchor::new(5, 50), TimelineAnchor::new(5, 60)],
                "strictly increasing",
            ),
            (
                vec![TimelineAnchor::new(5, 50), TimelineAnchor::new(6, 40)],
                "must not decrease",
            ),
            (
                vec![TimelineAnchor::new(11, 50)],
                "beyond the timeline step count",
            ),
            (
                vec![TimelineAnchor::new(1, 100)],
                "beyond the timeline end tick",
            ),
        ];
        for (anchors, expected) in cases {
            let error = TimelineView::new(10, 10, 99, anchors).expect_err("invalid timeline");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains(expected), "{error}");
        }

        let error = TimelineView::new(u64::MAX, 10, 10, Vec::new()).expect_err("bounded value");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    }

    #[test]
    fn presentation_frames_are_bounded_before_the_buffer_is_trusted() {
        let frame = PresentationFrame::new(2, 2, vec![0x11; 16], 7).expect("frame");
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.height(), 2);
        assert_eq!(frame.rgba().len(), 16);
        assert_eq!(frame.sequence(), 7);
        assert_eq!(
            frame.digest(),
            PresentationFrame::new(2, 2, vec![0x11; 16], 7)
                .expect("again")
                .digest()
        );
        assert_ne!(
            frame.digest(),
            PresentationFrame::new(2, 2, vec![0x12; 16], 7)
                .expect("changed")
                .digest()
        );
        assert_ne!(
            frame.digest(),
            PresentationFrame::new(2, 2, vec![0x11; 16], 8)
                .expect("resequenced")
                .digest()
        );

        for (width, height, bytes) in [(0, 2, 16), (2, 0, 16), (2, 2, 15), (2, 2, 17)] {
            let error =
                PresentationFrame::new(width, height, vec![0; bytes], 0).expect_err("bad frame");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
        }

        let too_wide = u32::try_from(MAX_PRESENTATION_PIXELS).expect("pixels") + 1;
        let error = PresentationFrame::new(too_wide, 1, Vec::new(), 0)
            .expect_err("pixel count above the limit");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    }

    #[test]
    fn audio_chunks_are_bounded_and_deterministic() {
        let chunk = AudioChunk::new(48_000, vec![-32768, 0, 32767], 2).expect("chunk");
        assert_eq!(chunk.sample_rate(), 48_000);
        assert_eq!(chunk.samples(), &[-32768, 0, 32767]);
        assert_eq!(chunk.sequence(), 2);
        assert_eq!(
            chunk.digest(),
            AudioChunk::new(48_000, vec![-32768, 0, 32767], 2)
                .expect("again")
                .digest()
        );

        assert_eq!(
            AudioChunk::new(0, Vec::new(), 0)
                .expect_err("zero sample rate")
                .code(),
            GuiErrorCode::BadValue
        );
        let error = AudioChunk::new(48_000, vec![0; MAX_AUDIO_SAMPLES + 1], 0)
            .expect_err("oversized chunk");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
        AudioChunk::new(48_000, vec![0; MAX_AUDIO_SAMPLES], 0).expect("chunk at the limit");
    }
}
