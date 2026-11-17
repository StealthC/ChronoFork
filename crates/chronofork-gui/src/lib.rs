//! Human-facing presentation layer for ChronoFork (roadmap Stage 7 / milestone M7).
//!
//! `chronofork-gui` owns the presentation model, the panel and availability rules, the
//! frame renderers, the pacing and run loop and the cooperative controller lease. It is
//! a **consumer** of canonical views: it never owns, mutates or re-derives emulated
//! state, and it cannot name a session, snapshot or backend type because it does not
//! depend on those crates.
//!
//! The accepted decision is `docs/adr/0013-human-facing-gui.md`; the architecture
//! contract (module map, block field order, numeric limits, handoff table) is
//! `docs/architecture/gui.md`; the executable acceptance plan is
//! `docs/quality/m7-gui-acceptance.md`.
//!
//! # Layer boundary
//!
//! Direct dependencies are `chronofork-core` (SHA-256 digests) and
//! `chronofork-service` (canonical JSON value, codec, [`ServiceLimits`]). The crate
//! never depends on `chronofork-transport`, `chronofork-sessions`,
//! `chronofork-backend-api` or a concrete backend, and it opens no socket, file,
//! environment variable or host clock. The only way out is the injected
//! [`OperationChannel`]; the only way in for time is the injected host clock owned by
//! [`pacing`]. Every canonical byte is produced and read by the service codec, so this
//! crate never contains a second JSON implementation.
//!
//! # Modules
//!
//! | Module | Responsibility |
//! |---|---|
//! | `lib` (this module) | Crate documentation, module map, limits, [`GuiError`] and its stable protocol codes, re-exports |
//! | [`channel`] | [`OperationChannel`], [`ChannelFault`], canonical request encoding and strict response parsing helpers |
//! | [`view`] | Immutable view values and their strict, field-exact decoders, plus the bounded presentation value types and the video/audio contracts |
//! | [`model`] | [`GuiModel`]: panes, focus, layout, panels, counters and the bounded presentation event log, including the read-only attach sequence |
//! | [`render`] | [`Frame`], [`RenderOptions`], the plain/ANSI grid renderer and the self-contained HTML renderer |
//! | [`pacing`] | Injected host clock, pacer configuration, the explicit `pulse` path and the host-time run loop |
//! | [`presentation`] | The capability-gated video/audio pipeline (`Available`/`Unavailable`) |
//! | [`control`] | The window-scoped controller lease and the human/agent handoff state machine |
//! | [`command`] | The `chronofork gui run` line grammar and the typed command engine (`CommandOutcome`/`Block`/`BlockField`); the **CLI** serializes the blocks and separates them with one blank line |
//!
//! Every module above is implemented. This crate owns the operation channel, the strict
//! view decoders, the presentation model, the plain/ANSI and HTML renderers, the pacer and
//! its host-time run loop, the capability-gated presentation pipeline, the controller lease
//! and the command engine. The CLI stays the composition root: it supplies the operation
//! channel implementation, the flags and token file, the movie loader and the block
//! serialization.
//!
//! # Limits
//!
//! The constants below are the single source of the numeric bounds listed in
//! `docs/architecture/gui.md` §7. A bound is validated before the bounded work starts
//! and produces a typed [`GuiError`], never a partial allocation. The `step`/`run` bound
//! is deliberately absent: it is the service's own
//! `ServiceLimits::max_steps_per_request`, which the crate reads from the
//! `capabilities` view instead of duplicating.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod channel;
pub mod command;
pub mod control;
pub mod model;
pub mod pacing;
pub mod presentation;
pub mod render;
pub mod view;

use std::fmt;

use chronofork_service::ServiceLimits;

/// Maximum number of sessions attached to one window (`gui.md` §7).
pub const MAX_PANES: usize = 4;

/// Default frame width in cells.
pub const DEFAULT_WIDTH: u32 = 80;
/// Minimum accepted frame width in cells.
pub const MIN_WIDTH: u32 = 40;
/// Maximum accepted frame width in cells.
pub const MAX_WIDTH: u32 = 200;

/// Default frame height in lines.
pub const DEFAULT_HEIGHT: u32 = 24;
/// Minimum accepted frame height in lines.
pub const MIN_HEIGHT: u32 = 10;
/// Maximum accepted frame height in lines.
pub const MAX_HEIGHT: u32 = 100;

/// Maximum number of bytes one memory panel refresh may read.
pub const MAX_MEMORY_PANEL_BYTES: usize = 256;

/// Maximum number of presentation events retained by the event log.
pub const EVENT_LOG_CAPACITY: usize = 256;

/// Maximum size in bytes of the self-contained HTML document.
pub const MAX_HTML_BYTES: usize = 1_048_576;

/// Maximum number of presentation frames one `pulse` command may advance.
pub const MAX_PULSE_FRAMES: u64 = 600;

/// Default presentation cadence in frames per second.
pub const DEFAULT_FPS: u32 = 60;
/// Minimum accepted presentation cadence.
pub const MIN_FPS: u32 = 1;
/// Maximum accepted presentation cadence.
pub const MAX_FPS: u32 = 240;

/// Default number of canonical steps advanced per presented frame.
pub const DEFAULT_TICKS_PER_FRAME: u64 = 1;
/// Maximum accepted number of canonical steps per presented frame.
pub const MAX_TICKS_PER_FRAME: u64 = 1000;

/// Default number of frames a late host may execute in one run-loop step.
pub const DEFAULT_MAX_CATCH_UP_FRAMES: u64 = 4;
/// Maximum accepted catch-up frame cap.
pub const MAX_MAX_CATCH_UP_FRAMES: u64 = 16;

/// Maximum number of pixels in one [`view::PresentationFrame`].
pub const MAX_PRESENTATION_PIXELS: usize = 1_048_576;

/// Maximum number of samples in one [`view::AudioChunk`].
pub const MAX_AUDIO_SAMPLES: usize = 65_536;

/// Longest diagnostic detail the crate keeps, in bytes.
///
/// Diagnostics are bounded and sanitized to printable ASCII before they reach a type,
/// so no view, command line, token or peer payload can flood stderr or move a terminal
/// (ADR-0013 §6).
pub(crate) const MAX_DETAIL_BYTES: usize = 120;

/// Stable protocol codes of the `chronofork gui run` surface.
///
/// The set is closed by ADR-0013 §3.9: a failure prints exactly one stderr line shaped
/// `chronofork: gui: <code>: <detail>` (or the canonical `chronofork.error.v1`
/// envelope when an operation failed). No other code exists, and none is invented by a
/// layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuiErrorCode {
    /// The line command or the option is not part of the documented grammar.
    UnknownCommand,
    /// A command carried the wrong number of arguments.
    BadArity,
    /// An option, its value or a command argument is invalid, missing or repeated.
    BadValue,
    /// The focused pane holds no attached session.
    NotAttached,
    /// The pane identifier is outside `1..=MAX_PANES` or holds no session.
    UnknownPane,
    /// The panel name is not one of the documented eight.
    UnknownPanel,
    /// A control command was issued while the agent holds the controller lease.
    ControlDenied,
    /// A documented resource bound would be exceeded.
    LimitExceeded,
    /// The `.cfmovie` could not be read read-only by the composition root.
    Movie,
    /// The operation channel failed, or a peer response was not the documented result.
    Channel,
}

impl GuiErrorCode {
    /// Returns the stable wire string of this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCommand => "unknown_command",
            Self::BadArity => "bad_arity",
            Self::BadValue => "bad_value",
            Self::NotAttached => "not_attached",
            Self::UnknownPane => "unknown_pane",
            Self::UnknownPanel => "unknown_panel",
            Self::ControlDenied => "control_denied",
            Self::LimitExceeded => "limit_exceeded",
            Self::Movie => "movie",
            Self::Channel => "channel",
        }
    }
}

/// One typed GUI failure.
///
/// Every variant carries structured data, never a bare string, and every free-text
/// detail is sanitized to printable ASCII and bounded by `MAX_DETAIL_BYTES` through
/// the constructors. The process surface prints
/// `chronofork: gui: <code>: <detail>` from [`GuiError::code`] and [`fmt::Display`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuiError {
    /// A line command or an option name is not in the documented grammar.
    UnknownCommand {
        /// The rejected name, sanitized and bounded.
        name: String,
    },
    /// A command carried the wrong number of arguments.
    BadArity {
        /// The command name.
        command: &'static str,
        /// The accepted arity, as a short human phrase.
        expected: &'static str,
        /// The number of arguments that were supplied.
        got: usize,
    },
    /// An option, its value or a command argument is invalid.
    BadValue {
        /// The rejected field or flag.
        field: &'static str,
        /// A bounded explanation.
        detail: String,
    },
    /// The focused pane holds no attached session.
    NotAttached,
    /// The pane identifier is out of range or holds no session.
    UnknownPane {
        /// The rejected 1-based pane identifier.
        pane: u32,
    },
    /// The panel name is not one of the documented eight.
    UnknownPanel {
        /// The rejected name, sanitized and bounded.
        name: String,
    },
    /// A control command was issued while the agent holds the controller lease.
    ControlDenied {
        /// The lease generation at the moment of the rejection.
        generation: u64,
        /// The lease sequence at the moment of the rejection.
        sequence: u64,
    },
    /// A documented resource bound would be exceeded.
    LimitExceeded {
        /// The bounded resource.
        what: &'static str,
        /// The applied bound.
        limit: u64,
    },
    /// The read-only `.cfmovie` could not be used by the timeline panel.
    Movie {
        /// A bounded, path-free explanation.
        detail: String,
    },
    /// The operation channel failed.
    Channel {
        /// A bounded, path-free, token-free explanation.
        detail: String,
        /// The canonical `chronofork.error.v1` bytes when the peer produced them.
        envelope: Option<Vec<u8>>,
    },
    /// A peer response was not the documented canonical operation result.
    ///
    /// This is reported under the closed `channel` protocol code: the response did not
    /// satisfy the operation contract, so the failure is a channel-level one and no
    /// new code is invented for it.
    Protocol {
        /// A bounded explanation of the violated field rule.
        detail: String,
    },
}

impl GuiError {
    /// Creates an unknown-command failure.
    #[must_use]
    pub fn unknown_command(name: &str) -> Self {
        Self::UnknownCommand {
            name: bounded_detail(name),
        }
    }

    /// Creates a wrong-arity failure.
    #[must_use]
    pub const fn bad_arity(command: &'static str, expected: &'static str, got: usize) -> Self {
        Self::BadArity {
            command,
            expected,
            got,
        }
    }

    /// Creates an invalid-value failure.
    #[must_use]
    pub fn bad_value(field: &'static str, detail: &str) -> Self {
        Self::BadValue {
            field,
            detail: bounded_detail(detail),
        }
    }

    /// Creates a not-attached failure.
    #[must_use]
    pub const fn not_attached() -> Self {
        Self::NotAttached
    }

    /// Creates an unknown-pane failure.
    #[must_use]
    pub const fn unknown_pane(pane: u32) -> Self {
        Self::UnknownPane { pane }
    }

    /// Creates an unknown-panel failure.
    #[must_use]
    pub fn unknown_panel(name: &str) -> Self {
        Self::UnknownPanel {
            name: bounded_detail(name),
        }
    }

    /// Creates a control-denied failure holding the current lease counters.
    #[must_use]
    pub const fn control_denied(generation: u64, sequence: u64) -> Self {
        Self::ControlDenied {
            generation,
            sequence,
        }
    }

    /// Creates a limit-exceeded failure.
    #[must_use]
    pub const fn limit_exceeded(what: &'static str, limit: u64) -> Self {
        Self::LimitExceeded { what, limit }
    }

    /// Creates a movie failure.
    #[must_use]
    pub fn movie(detail: &str) -> Self {
        Self::Movie {
            detail: bounded_detail(detail),
        }
    }

    /// Creates a channel failure without a peer error envelope.
    #[must_use]
    pub fn channel(detail: &str) -> Self {
        Self::Channel {
            detail: bounded_detail(detail),
            envelope: None,
        }
    }

    /// Creates a protocol failure for a response that violated the operation contract.
    #[must_use]
    pub fn protocol(detail: &str) -> Self {
        Self::Protocol {
            detail: bounded_detail(detail),
        }
    }

    /// Returns the stable protocol code of this failure.
    #[must_use]
    pub const fn code(&self) -> GuiErrorCode {
        match self {
            Self::UnknownCommand { .. } => GuiErrorCode::UnknownCommand,
            Self::BadArity { .. } => GuiErrorCode::BadArity,
            Self::BadValue { .. } => GuiErrorCode::BadValue,
            Self::NotAttached => GuiErrorCode::NotAttached,
            Self::UnknownPane { .. } => GuiErrorCode::UnknownPane,
            Self::UnknownPanel { .. } => GuiErrorCode::UnknownPanel,
            Self::ControlDenied { .. } => GuiErrorCode::ControlDenied,
            Self::LimitExceeded { .. } => GuiErrorCode::LimitExceeded,
            Self::Movie { .. } => GuiErrorCode::Movie,
            Self::Channel { .. } | Self::Protocol { .. } => GuiErrorCode::Channel,
        }
    }

    /// Returns the canonical `chronofork.error.v1` bytes carried by a channel failure.
    ///
    /// The process surface prints these bytes verbatim, and nothing else, when an
    /// operation failed on the peer.
    #[must_use]
    pub fn envelope(&self) -> Option<&[u8]> {
        match self {
            Self::Channel { envelope, .. } => envelope.as_deref(),
            _ => None,
        }
    }
}

impl fmt::Display for GuiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand { name } => write!(formatter, "unknown_command: `{name}`"),
            Self::BadArity {
                command,
                expected,
                got,
            } => write!(
                formatter,
                "bad_arity: {command} expects {expected}, got {got} argument(s)"
            ),
            Self::BadValue { field, detail } => write!(formatter, "bad_value: {field}: {detail}"),
            Self::NotAttached => {
                write!(formatter, "not_attached: the focused pane holds no session")
            }
            Self::UnknownPane { pane } => {
                write!(formatter, "unknown_pane: pane {pane} holds no session")
            }
            Self::UnknownPanel { name } => write!(formatter, "unknown_panel: `{name}`"),
            Self::ControlDenied {
                generation,
                sequence,
            } => write!(
                formatter,
                "control_denied: the agent holds control (generation {generation}, sequence {sequence})"
            ),
            Self::LimitExceeded { what, limit } => {
                write!(formatter, "limit_exceeded: {what} is limited to {limit}")
            }
            Self::Movie { detail } => write!(formatter, "movie: {detail}"),
            Self::Channel { detail, .. } | Self::Protocol { detail } => {
                write!(formatter, "channel: {detail}")
            }
        }
    }
}

impl std::error::Error for GuiError {}

/// Returns the canonical codec limits the crate encodes and decodes under.
///
/// Architecture §2 fixes one value for the whole path: the service's
/// [`ServiceLimits::default`]. The crate therefore holds no limit number of its own.
pub(crate) fn codec_limits() -> ServiceLimits {
    ServiceLimits::default()
}

/// Sanitizes one diagnostic detail to printable ASCII and bounds its length.
///
/// Non-printable and non-ASCII bytes become `?` because a detail can quote a
/// peer-supplied field name or a raw command line, and a diagnostic must never move a
/// terminal or carry a control sequence (ADR-0013 §6).
pub(crate) fn bounded_detail(text: &str) -> String {
    let mut output = String::with_capacity(MAX_DETAIL_BYTES);
    for character in text.chars() {
        if output.len() + 4 > MAX_DETAIL_BYTES {
            output.push_str("...");
            return output;
        }
        output.push(if (' '..='~').contains(&character) {
            character
        } else {
            '?'
        });
    }
    output
}

pub use channel::{ChannelFault, OperationChannel, parse_response, request_bytes, result_for};
pub use model::{
    AttachReport, DetachReport, Event, EventKind, EventLog, GuiModel, Layout, Mosaic, Pane, PaneId,
    PanelKind, PanelState, Rect, SplitAxis, WindowId,
};
pub use render::{Frame, RenderOptions};
pub use view::{
    AudioChunk, BackendCapabilities, BackendView, CapabilityView, CreateView, LimitsView, Lineage,
    MachineView, MemoryView, PresentationFrame, RunView, SessionList, SessionState, SessionSummary,
    SessionView, TimelineAnchor, TimelineView, VideoSource, decode_capabilities, decode_create,
    decode_memory, decode_run, decode_session_list, decode_session_view,
};

#[cfg(test)]
mod tests {
    use super::{GuiError, GuiErrorCode, MAX_DETAIL_BYTES, bounded_detail};

    #[test]
    fn protocol_codes_are_the_documented_closed_set() {
        let expected = [
            (GuiErrorCode::UnknownCommand, "unknown_command"),
            (GuiErrorCode::BadArity, "bad_arity"),
            (GuiErrorCode::BadValue, "bad_value"),
            (GuiErrorCode::NotAttached, "not_attached"),
            (GuiErrorCode::UnknownPane, "unknown_pane"),
            (GuiErrorCode::UnknownPanel, "unknown_panel"),
            (GuiErrorCode::ControlDenied, "control_denied"),
            (GuiErrorCode::LimitExceeded, "limit_exceeded"),
            (GuiErrorCode::Movie, "movie"),
            (GuiErrorCode::Channel, "channel"),
        ];
        for (code, name) in expected {
            assert_eq!(code.as_str(), name);
        }
    }

    #[test]
    fn every_failure_reports_its_code_and_a_prefixed_display() {
        let cases = [
            (
                GuiError::unknown_command("bogus"),
                GuiErrorCode::UnknownCommand,
            ),
            (
                GuiError::bad_arity("step", "at most one argument", 2),
                GuiErrorCode::BadArity,
            ),
            (
                GuiError::bad_value("--width", "not an integer"),
                GuiErrorCode::BadValue,
            ),
            (GuiError::not_attached(), GuiErrorCode::NotAttached),
            (GuiError::unknown_pane(9), GuiErrorCode::UnknownPane),
            (GuiError::unknown_panel("bogus"), GuiErrorCode::UnknownPanel),
            (GuiError::control_denied(2, 3), GuiErrorCode::ControlDenied),
            (
                GuiError::limit_exceeded("pulse frames", 600),
                GuiErrorCode::LimitExceeded,
            ),
            (GuiError::movie("truncated"), GuiErrorCode::Movie),
            (GuiError::channel("closed"), GuiErrorCode::Channel),
            (GuiError::protocol("missing field"), GuiErrorCode::Channel),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code, "{error}");
            assert!(
                error.to_string().starts_with(code.as_str()),
                "{error} must be prefixed with its code"
            );
        }
    }

    #[test]
    fn a_protocol_failure_reports_the_channel_code_without_inventing_one() {
        let error = GuiError::protocol("capabilities result is missing field `backend`");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert_eq!(
            error.to_string(),
            "channel: capabilities result is missing field `backend`"
        );
        assert_eq!(error.envelope(), None);
    }

    #[test]
    fn channel_failures_keep_the_canonical_envelope_bytes() {
        let envelope = b"{\"error\":{\"code\":\"forbidden\"},\"schema\":\"chronofork.error.v1\"}";
        let error = GuiError::Channel {
            detail: "operation failed".to_owned(),
            envelope: Some(envelope.to_vec()),
        };
        assert_eq!(error.envelope(), Some(envelope.as_slice()));
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert_eq!(GuiError::channel("no peer bytes").envelope(), None);
    }

    #[test]
    fn details_are_bounded_and_sanitized_to_printable_ascii() {
        let long = "x".repeat(MAX_DETAIL_BYTES * 4);
        let bounded = bounded_detail(&long);
        assert_eq!(bounded.len(), MAX_DETAIL_BYTES);
        assert!(bounded.ends_with("..."));

        let hostile = "a\u{1b}[31mb\u{7}c\u{7f}d\u{e9}";
        assert_eq!(bounded_detail(hostile), "a?[31mb?c?d?");

        let error = GuiError::bad_value("--token-file", &long);
        assert!(matches!(
            error,
            GuiError::BadValue { detail, .. } if detail.len() == MAX_DETAIL_BYTES
        ));
    }

    #[test]
    fn an_error_detail_never_exceeds_the_bound_through_its_constructors() {
        let name = "z".repeat(10_000);
        for error in [
            GuiError::unknown_command(&name),
            GuiError::unknown_panel(&name),
            GuiError::movie(&name),
            GuiError::channel(&name),
            GuiError::protocol(&name),
            GuiError::bad_value("--html", &name),
        ] {
            let rendered = error.to_string();
            assert!(
                rendered.len() <= MAX_DETAIL_BYTES + 40,
                "diagnostic is unbounded: {} bytes",
                rendered.len()
            );
        }
    }
}
