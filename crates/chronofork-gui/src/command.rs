//! The `chronofork gui run` line grammar and the typed command engine.
//!
//! This module owns the process surface of the crate: it tokenizes one command per stdin
//! line on ASCII whitespace, applies the documented grammar (ADR-0013 §3.9), dispatches
//! through the model, the controller lease, the pacer and the operation channel, and
//! returns a typed outcome — [`CommandOutcome::Block`] with a [`Record`] and the documented
//! [`BlockField`] order, [`CommandOutcome::Grid`] for `frame`, or
//! [`CommandOutcome::Html`] for `html`.
//!
//! **Block serialization belongs to the CLI, not to this crate.** The CLI writes
//! `record=<name>` and then one `key=value` line per field in the vector order, separates
//! the blocks with exactly one blank line, and decides whether `html` prints the document
//! or writes it to `--html` and prints its block, because the process surface is the
//! composition root. This module never renders `key=value` text and never joins blocks: it
//! returns values, so a test can assert field names, order and contents instead of parsing
//! output.
//!
//! # Execution rules
//!
//! - `create` and `attach` are the only commands whose channel calls happen inside
//!   [`GuiModel`] ([`GuiModel::create`], [`GuiModel::attach`]); this engine never
//!   re-implements `session.inspect` or `session.read_memory`. `open` issues `capabilities`
//!   exactly once, and `list`, `step`, `run` and `pulse` are the only other operations
//!   requested, so `detach`, `focus`, `layout`, `panel`, `pause`, `handoff`, `frame`,
//!   `html` and `quit` reach the channel zero times.
//! - A control command (`step`, `run`, `pulse`, and the host-time run loop of
//!   [`CommandSession::advance_elapsed`]) applies the controller lease **before** any
//!   channel call: a denied command performs no operation, records the attempt in the
//!   presentation event log and returns the typed `control_denied` failure carrying the
//!   current generation and sequence (ADR-0013 §3.8).
//! - The order inside one control command is: grammar (arity/type/`MAX_PULSE_FRAMES`),
//!   lease gate, attached-pane check, service `max_steps_per_request` bound, channel call.
//!   Every bound is checked before the bounded work starts, and a rejected line emits no
//!   block.
//! - Host time never reaches a block, a field or a frame: the pulse time is the pacer's
//!   cumulative own accounting, and the injected clock is read only by
//!   [`CommandSession::advance_elapsed`].
//! - A failure is the crate's single typed [`GuiError`]; the CLI maps it to exactly one
//!   stderr line (`chronofork: gui: <code>: <detail>`, or the canonical
//!   `chronofork.error.v1` envelope of an operation failure) and stops.
//!
//! # Field notes
//!
//! The block tables of `gui.md` §6.2 fix the field order. A few field shapes are worth
//! stating because they are easy to misread:
//!
//! - [`BlockField::Bool`] renders as `true`/`false` and [`BlockField::OptionalUint`] renders
//!   as the value or `none`, matching the documented `focused`/`reason` spellings of the
//!   process surface (a session lineage that has no parent reports `none`).
//! - In the `attach` block, `attached` reports whether the pane holds an attached session
//!   after the command (`true` for `attach`, `false` for `detach`) and `focused` is `true`
//!   after an attach; the `detach`, `focus` and `layout` blocks report `focused` as the
//!   focused pane id or `none`. The `detach` block's `generation` is the generation the
//!   detached attachment had, which is the last state the pane actually held.
//!
//! A [`BlockField::Text`] value is printable ASCII without spaces: the constructors
//! substitute `_` for any other character, so a peer-supplied identifier can never break the
//! line protocol. Field names are a [`FieldName`] (`Cow<'static, str>`): every fixed name is
//! a borrowed static and the `list` block's per-session names (`session.<id>.state`) are
//! owned, which is the one family of keys a `&'static str` could not represent.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use chronofork_core::hash_bytes;
use chronofork_service::json::JsonValue;

use crate::channel::{OperationChannel, request_bytes, unsigned_input};
use crate::control::{Controller, Lease};
use crate::model::{EventKind, GuiModel, Layout, PaneId, PanelKind, WindowId};
use crate::pacing::{ElapsedReport, HostClock, Pacer, PacerConfig};
use crate::presentation::{AudioPipeline, UnavailableReason, VideoPipeline, panel_availability};
use crate::render::text::{self, FrameSource};
use crate::render::{RenderOptions, html};
use crate::view::{
    CapabilityView, RunView, TimelineView, decode_capabilities, decode_run, decode_session_list,
};
use crate::{
    DEFAULT_HEIGHT, DEFAULT_WIDTH, GuiError, MAX_HEIGHT, MAX_PULSE_FRAMES, MAX_WIDTH, MIN_HEIGHT,
    MIN_WIDTH,
};

/// The name of one block field.
///
/// `gui.md` §6.1 documents `FieldName` as `Cow<'static, str>`: the `list` block's per-session
/// field names interpolate a canonical session identifier (`session.<id>.state`, `.tick`,
/// `.state_hash`, `.lineage_depth`), which a `&'static str` cannot express without leaking
/// memory, so fixed names stay borrowed statics and only that one family is owned.
pub type FieldName = Cow<'static, str>;

/// The presentation configuration of one window, assembled from CLI flags.
///
/// The values are plain: no file, no token and no environment value can reach this struct.
/// `width`/`height` are validated by [`CommandSession::open`] against the documented frame
/// bounds, `pacer` is already validated by [`PacerConfig::new`], and `timeline` is the plain
/// projection of a `.cfmovie` the composition root read read-only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandConfig {
    /// Frame width in cells.
    pub width: u32,
    /// Frame height in lines.
    pub height: u32,
    /// Renderer options (`--ansi`).
    pub render: RenderOptions,
    /// Presentation cadence.
    pub pacer: PacerConfig,
    /// The CLI-provided TAS timeline, when the window was opened with `--movie`.
    pub timeline: Option<TimelineView>,
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            render: RenderOptions::plain(),
            pacer: PacerConfig::default(),
            timeline: None,
        }
    }
}

impl CommandConfig {
    /// Rejects a frame size outside the documented bounds.
    ///
    /// The frame size arrives from `--width`/`--height`, so an out-of-range value is a bad
    /// option value (`bad_value`, ADR-0013 §3.9 item 5) rather than the renderer's resource
    /// guard, which is never reachable once this check has run.
    fn check_frame_size(&self) -> Result<(), GuiError> {
        if !(MIN_WIDTH..=MAX_WIDTH).contains(&self.width) {
            return Err(GuiError::bad_value(
                "--width",
                &format!("{} is outside {MIN_WIDTH}..={MAX_WIDTH}", self.width),
            ));
        }
        if !(MIN_HEIGHT..=MAX_HEIGHT).contains(&self.height) {
            return Err(GuiError::bad_value(
                "--height",
                &format!("{} is outside {MIN_HEIGHT}..={MAX_HEIGHT}", self.height),
            ));
        }
        Ok(())
    }
}

/// Why the line loop stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuitReason {
    /// A `quit` line was read.
    Line,
    /// Standard input reached end of file.
    Eof,
}

impl QuitReason {
    /// Returns the canonical `reason` value of this stop.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Eof => "eof",
        }
    }
}

/// The record name of one block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Record {
    /// `create`
    Create,
    /// `list`
    List,
    /// `attach`
    Attach,
    /// `detach`
    Detach,
    /// `focus`
    Focus,
    /// `layout`
    Layout,
    /// `panel`
    Panel,
    /// `step` (a denied `run` is recorded as a step attempt, but a successful `run` has its
    /// own record name)
    Step,
    /// `run`
    Run,
    /// `pulse`
    Pulse,
    /// `pause`
    Pause,
    /// `handoff`
    Handoff,
    /// `html`
    Html,
    /// `quit`
    Quit,
}

impl Record {
    /// Returns the canonical `record=` name of this block.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::List => "list",
            Self::Attach => "attach",
            Self::Detach => "detach",
            Self::Focus => "focus",
            Self::Layout => "layout",
            Self::Panel => "panel",
            Self::Step => "step",
            Self::Run => "run",
            Self::Pulse => "pulse",
            Self::Pause => "pause",
            Self::Handoff => "handoff",
            Self::Html => "html",
            Self::Quit => "quit",
        }
    }
}

/// One typed field of a block, in the documented order.
///
/// The variants are public so the CLI formats them without a second mapping table; the
/// constructors keep the one text rule (printable ASCII, no spaces) in this crate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockField {
    /// A non-negative integer field.
    Uint(FieldName, u64),
    /// A boolean field, rendered as `true`/`false`.
    Bool(FieldName, bool),
    /// A text field: printable ASCII, no spaces.
    Text(FieldName, String),
    /// An optional non-negative integer, rendered as the value or `none`.
    OptionalUint(FieldName, Option<u64>),
}

impl BlockField {
    /// Creates an integer field.
    #[must_use]
    pub fn uint(name: impl Into<FieldName>, value: u64) -> Self {
        Self::Uint(name.into(), value)
    }

    /// Creates a boolean field.
    #[must_use]
    pub fn bool(name: impl Into<FieldName>, value: bool) -> Self {
        Self::Bool(name.into(), value)
    }

    /// Creates a text field, substituting `_` for any space, control or non-ASCII character.
    ///
    /// The substitution is the block counterpart of the frame renderer's `?` rule: a
    /// peer-supplied label can never break the `key=value` line protocol or move a
    /// terminal.
    #[must_use]
    pub fn text(name: impl Into<FieldName>, value: &str) -> Self {
        let sanitized: String = value
            .chars()
            .map(|character| {
                if ('!'..='~').contains(&character) {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        Self::Text(name.into(), sanitized)
    }

    /// Creates an optional integer field.
    #[must_use]
    pub fn optional_uint(name: impl Into<FieldName>, value: Option<u64>) -> Self {
        Self::OptionalUint(name.into(), value)
    }

    /// Returns the field name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Uint(name, _)
            | Self::Bool(name, _)
            | Self::Text(name, _)
            | Self::OptionalUint(name, _) => name,
        }
    }
}

/// One `record=` block: a name and its fields in the documented order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    /// The record name, written as `record=<name>` by the CLI.
    pub record: Record,
    /// The fields, in the documented order.
    pub fields: Vec<BlockField>,
}

impl Block {
    /// Creates one block.
    #[must_use]
    pub fn new(record: Record, fields: Vec<BlockField>) -> Self {
        Self { record, fields }
    }
}

/// What one executed line produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    /// A `record=` block for any command except `frame` and `html`.
    Block(Block),
    /// The serialized frame: `height` `\n`-terminated lines of `width` printable ASCII
    /// columns, already plain or ANSI per [`RenderOptions`].
    Grid(String),
    /// The `html` document and the `html` block that describes it.
    Html {
        /// The self-contained document.
        document: String,
        /// The `html` block (`bytes`, `sha256`).
        block: Block,
    },
}

/// One parsed command line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Parsed {
    Create,
    List,
    Attach(u64),
    Detach,
    Focus(PaneId),
    Layout(Layout),
    Panel(PanelKind, bool),
    Step(u64),
    Run(u64),
    Pulse(u64),
    Pause,
    Handoff(Controller),
    Frame,
    Html,
    Quit,
}

/// The window engine: model, pacer, lease, channel and renderer configuration.
///
/// One session is one window (`WindowId` 1). The session owns the presentation model, the
/// cadence, the cooperative lease and the startup capability view, and it holds no
/// canonical state: the only mutating operations it issues are `session.create` (through
/// [`GuiModel::create`]) and `session.run` under the human lease.
pub struct CommandSession {
    channel: Arc<dyn OperationChannel>,
    clock: Arc<dyn HostClock>,
    model: GuiModel,
    pacer: Pacer,
    lease: Lease,
    capabilities: CapabilityView,
    video: VideoPipeline,
    audio: AudioPipeline,
    config: CommandConfig,
}

impl fmt::Debug for CommandSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandSession")
            .field("model", &self.model)
            .field("pacer", &self.pacer)
            .field("lease", &self.lease)
            .field("capabilities", &self.capabilities)
            .field("video", &self.video)
            .field("audio", &self.audio)
            .field("config", &self.config)
            .field("channel", &"<operation channel>")
            .field("clock", &"<host clock>")
            .finish()
    }
}

impl CommandSession {
    /// Opens one window: validates the frame size, issues `capabilities` once and decodes it.
    ///
    /// The startup `capabilities` call is the window-level descriptor gate and the source of
    /// the service step bound; both pipelines start with no source and no sink, which is the
    /// only v1 configuration (`gui.md` §6.2).
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::BadValue`] for a frame size outside the documented bounds, the
    /// channel failure of the `capabilities` call, and the strict decoding failure of a
    /// response that is not the documented `chronofork.capabilities.v1` result.
    pub fn open(
        channel: Arc<dyn OperationChannel>,
        clock: Arc<dyn HostClock>,
        config: CommandConfig,
    ) -> Result<Self, GuiError> {
        config.check_frame_size()?;
        let request = JsonValue::Object(BTreeMap::new());
        let bytes = channel
            .call(&request_bytes("capabilities", &request)?)
            .map_err(GuiError::from)?;
        let capabilities = decode_capabilities(&bytes)?;
        Ok(Self {
            channel,
            clock,
            model: GuiModel::new(WindowId::from_raw(1)),
            pacer: Pacer::new(config.pacer),
            lease: Lease::new(),
            capabilities,
            video: VideoPipeline::new(None),
            audio: AudioPipeline::new(None),
            config,
        })
    }

    /// Returns the presentation model, which owns no canonical state.
    ///
    /// This read-only accessor exists for the acceptance tests: A2/A4/A6/A16 assert the
    /// panes, counters and event log after a command instead of parsing process output. The
    /// command engine itself is the only writer, and the CLI formats blocks without ever
    /// inspecting the model.
    #[must_use]
    pub fn model(&self) -> &GuiModel {
        &self.model
    }

    /// Returns the current controller lease.
    ///
    /// Read-only inspection accessor: the handoff tests assert the generation and sequence
    /// the lease reached, the same values the `handoff` and `quit` blocks report. The lease
    /// is mutated only by `handoff` inside the engine.
    #[must_use]
    pub fn lease(&self) -> Lease {
        self.lease
    }

    /// Parses and executes one line.
    ///
    /// Leading and trailing whitespace is ignored and an empty line yields `Ok(None)`; every
    /// other line is parsed against the closed grammar and then executed. A rejected line
    /// produces a typed [`GuiError`] and no outcome, so the caller emits no block for it.
    ///
    /// # Errors
    ///
    /// Returns the grammar failures (`unknown_command`, `bad_arity`, `bad_value`,
    /// `unknown_pane`, `unknown_panel`, `limit_exceeded`), the lease refusal
    /// (`control_denied`), `not_attached`, the channel failure of an operation, and the
    /// protocol failure of a response that is not the documented operation result.
    pub fn execute(&mut self, line: &str) -> Result<Option<CommandOutcome>, GuiError> {
        let mut tokens = line.split_ascii_whitespace();
        let Some(name) = tokens.next() else {
            return Ok(None);
        };
        let arguments: Vec<&str> = tokens.collect();
        let command = parse(name, &arguments)?;
        Ok(Some(self.run(command)?))
    }

    /// Builds the `quit` block for a `quit` line or for end of input.
    pub fn quit(&mut self, reason: QuitReason) -> Block {
        Block::new(
            Record::Quit,
            vec![
                BlockField::text("reason", reason.as_str()),
                BlockField::text("controller", self.lease.controller().as_str()),
                BlockField::uint("generation", self.lease.generation()),
                BlockField::uint("sequence", self.lease.sequence()),
            ],
        )
    }

    /// Runs the host-time run loop for one step: advances the frames the clock says are due.
    ///
    /// This is the FR-EXEC-007 loop of `gui.md` §5.2 and the reason the injected clock is
    /// held by the session. It is a control action, so it is gated by the lease, and it
    /// needs a focused pane because every advanced frame is one `session.run` on that pane.
    /// The host reading appears only in the returned [`ElapsedReport`]; a reading that is
    /// equal to or behind the pacer's cumulative time advances nothing.
    ///
    /// The process surface does not expose it: a host-time auto-advancing loop would make
    /// stdout depend on scheduling, so the CLI uses the deterministic `pulse` command. This
    /// method is therefore the crate's library entry point for FR-EXEC-007 — it is what
    /// keeps the injected clock live — and `tests/pacing.rs` drives it with
    /// [`crate::pacing::ManualClock`] at chosen instants.
    ///
    /// # Errors
    ///
    /// Returns the typed control refusal, `not_attached` when no pane is focused, and the
    /// channel or protocol failure of a frame's `session.run`.
    pub fn advance_elapsed(&mut self) -> Result<ElapsedReport, GuiError> {
        self.require_control(EventKind::Pulse)?;
        let (_, session) = self.focused_session()?;
        // `pause` gates this host-time loop, and only this loop: a paused window presents no
        // new frame, so the reading is pinned to the pacer's own cumulative time. The pacer
        // then reports zero due, zero advanced and zero dropped, and no `session.run` is
        // issued. The time that elapses while paused is not consumed: the next unpaused step
        // sees it as due, advances at most `max_catch_up_frames` and reports the remainder as
        // dropped, exactly as `gui.md` §5.2 specifies for a late host. `pulse` is the explicit
        // path and is deliberately not gated by the flag (see `pulse`).
        let now_nanos = if self.model.paused() {
            self.pacer.last_nanos()
        } else {
            self.clock.now_nanos()
        };
        let report = {
            let channel: &Arc<dyn OperationChannel> = &self.channel;
            let pacer: &mut Pacer = &mut self.pacer;
            let model: &mut GuiModel = &mut self.model;
            pacer.advance_elapsed(now_nanos, |steps| {
                run_session(channel.as_ref(), model, session, steps).map(|_| ())
            })?
        };
        self.model.advance_frames(report.advanced());
        Ok(report)
    }

    /// Dispatches one parsed command.
    fn run(&mut self, command: Parsed) -> Result<CommandOutcome, GuiError> {
        match command {
            Parsed::Create => Ok(CommandOutcome::Block(self.create()?)),
            Parsed::List => Ok(CommandOutcome::Block(self.list()?)),
            Parsed::Attach(session) => Ok(CommandOutcome::Block(self.attach(session)?)),
            Parsed::Detach => Ok(CommandOutcome::Block(self.detach()?)),
            Parsed::Focus(pane) => Ok(CommandOutcome::Block(self.focus(pane)?)),
            Parsed::Layout(layout) => Ok(CommandOutcome::Block(self.set_layout(layout))),
            Parsed::Panel(kind, visible) => {
                Ok(CommandOutcome::Block(self.set_panel(kind, visible)))
            }
            Parsed::Step(steps) => Ok(CommandOutcome::Block(self.advance(Record::Step, steps)?)),
            Parsed::Run(steps) => Ok(CommandOutcome::Block(self.advance(Record::Run, steps)?)),
            Parsed::Pulse(frames) => Ok(CommandOutcome::Block(self.pulse(frames)?)),
            Parsed::Pause => Ok(CommandOutcome::Block(self.pause())),
            Parsed::Handoff(controller) => Ok(CommandOutcome::Block(self.handoff(controller))),
            Parsed::Frame => Ok(CommandOutcome::Grid(self.frame()?)),
            Parsed::Html => self.html(),
            Parsed::Quit => Ok(CommandOutcome::Block(self.quit(QuitReason::Line))),
        }
    }

    /// Executes `create` through the model and returns its block.
    fn create(&mut self) -> Result<Block, GuiError> {
        let view = self.model.create(self.channel.as_ref())?;
        let lineage = view.lineage();
        Ok(Block::new(
            Record::Create,
            vec![
                BlockField::uint("session", view.session()),
                BlockField::text("backend_id", view.backend_id()),
                BlockField::text("state", view.state().as_str()),
                BlockField::uint("tick", view.tick()),
                BlockField::text("state_hash", &view.state_hash().to_hex()),
                BlockField::optional_uint("lineage_parent", lineage.parent_session()),
                BlockField::optional_uint("lineage_origin_snapshot", lineage.origin_snapshot()),
                BlockField::uint("lineage_depth", lineage.depth()),
            ],
        ))
    }

    /// Executes `list`: one `session.list` call and the per-session fields, ascending.
    fn list(&mut self) -> Result<Block, GuiError> {
        let request = JsonValue::Object(BTreeMap::new());
        let bytes = self
            .channel
            .call(&request_bytes("session.list", &request)?)
            .map_err(GuiError::from)?;
        let view = decode_session_list(&bytes)?;
        let mut sessions: Vec<_> = view.sessions().iter().collect();
        sessions.sort_by_key(|summary| summary.session());
        let mut fields = vec![
            BlockField::uint(
                "sessions",
                u64::try_from(sessions.len()).unwrap_or(u64::MAX),
            ),
            BlockField::bool("truncated", view.truncated()),
        ];
        for summary in sessions {
            let id = summary.session();
            fields.push(BlockField::text(
                format!("session.{id}.state"),
                summary.state().as_str(),
            ));
            fields.push(BlockField::uint(
                format!("session.{id}.tick"),
                summary.tick(),
            ));
            fields.push(BlockField::text(
                format!("session.{id}.state_hash"),
                &summary.state_hash().to_hex(),
            ));
            fields.push(BlockField::uint(
                format!("session.{id}.lineage_depth"),
                summary.lineage().depth(),
            ));
        }
        Ok(Block::new(Record::List, fields))
    }

    /// Executes `attach` through the model's read-only sequence and returns its block.
    fn attach(&mut self, session: u64) -> Result<Block, GuiError> {
        let report = self.model.attach(self.channel.as_ref(), session)?;
        let layout = self.model.layout().as_str();
        let panes = self.model.attached();
        let attached = self.model.pane(report.pane).ok_or(GuiError::NotAttached)?;
        let view = attached.view();
        let machine = view.machine();
        let lineage = view.lineage();
        Ok(Block::new(
            Record::Attach,
            vec![
                BlockField::uint("pane", u64::from(report.pane.raw())),
                BlockField::uint("session", session),
                BlockField::uint("generation", report.generation),
                BlockField::bool("attached", true),
                BlockField::bool("focused", true),
                BlockField::text("layout", layout),
                BlockField::uint("panes", u64::try_from(panes).unwrap_or(u64::MAX)),
                BlockField::text("backend", view.backend().id()),
                BlockField::text("backend_version", view.backend().version()),
                BlockField::bool("test_backend", view.backend().test_backend()),
                BlockField::bool("rendering", view.backend().capabilities().rendering()),
                BlockField::bool("audio", view.backend().capabilities().audio()),
                BlockField::text("state", view.state().as_str()),
                BlockField::uint("tick", machine.tick()),
                BlockField::text("state_hash", &machine.state_hash().to_hex()),
                BlockField::bool("halted", machine.halted()),
                BlockField::optional_uint("lineage_parent", lineage.parent_session()),
                BlockField::optional_uint("lineage_origin_snapshot", lineage.origin_snapshot()),
                BlockField::uint("lineage_depth", lineage.depth()),
                BlockField::uint("memory_base", machine.memory_base()),
                BlockField::uint("memory_length", machine.memory_length()),
            ],
        ))
    }

    /// Executes `detach`: a presentation act that issues no operation.
    fn detach(&mut self) -> Result<Block, GuiError> {
        let focused = self.model.focused().ok_or(GuiError::NotAttached)?;
        let generation = self
            .model
            .pane(focused)
            .map(|attached| attached.generation())
            .ok_or(GuiError::NotAttached)?;
        let report = self.model.detach()?;
        let focused = self.model.focused();
        let layout = self.model.layout().as_str();
        let panes = self.model.attached();
        Ok(Block::new(
            Record::Detach,
            vec![
                BlockField::uint("pane", u64::from(report.pane.raw())),
                BlockField::uint("session", report.session),
                BlockField::uint("generation", generation),
                BlockField::bool("attached", false),
                BlockField::optional_uint("focused", focused.map(|pane| u64::from(pane.raw()))),
                BlockField::text("layout", layout),
                BlockField::uint("panes", u64::try_from(panes).unwrap_or(u64::MAX)),
            ],
        ))
    }

    /// Executes `focus` on an attached pane.
    fn focus(&mut self, pane: PaneId) -> Result<Block, GuiError> {
        self.model.focus(pane)?;
        let session = self
            .model
            .pane(pane)
            .map(|attached| attached.session())
            .ok_or(GuiError::NotAttached)?;
        let layout = self.model.layout().as_str();
        let panes = self.model.attached();
        Ok(Block::new(
            Record::Focus,
            vec![
                BlockField::uint("pane", u64::from(pane.raw())),
                BlockField::uint("session", session),
                BlockField::uint("focused", u64::from(pane.raw())),
                BlockField::text("layout", layout),
                BlockField::uint("panes", u64::try_from(panes).unwrap_or(u64::MAX)),
            ],
        ))
    }

    /// Executes `layout`.
    fn set_layout(&mut self, layout: Layout) -> Block {
        self.model.set_layout(layout);
        let focused = self.model.focused();
        let panes = self.model.attached();
        Block::new(
            Record::Layout,
            vec![
                BlockField::text("layout", layout.as_str()),
                BlockField::uint("panes", u64::try_from(panes).unwrap_or(u64::MAX)),
                BlockField::optional_uint("focused", focused.map(|pane| u64::from(pane.raw()))),
            ],
        )
    }

    /// Executes `panel` and reports the panel's availability with its reason.
    fn set_panel(&mut self, kind: PanelKind, visible: bool) -> Block {
        self.model.set_panel(kind, visible);
        let availability = panel_availability(
            kind,
            self.model.focused().is_some(),
            self.config.timeline.is_some(),
        );
        let reason = availability
            .reason()
            .map_or("none", UnavailableReason::as_str);
        Block::new(
            Record::Panel,
            vec![
                BlockField::text("panel", kind.name()),
                BlockField::bool("visible", visible),
                BlockField::bool("available", availability.is_available()),
                BlockField::text("reason", reason),
            ],
        )
    }

    /// Executes `step`/`run`: one gated `session.run` folded into the model.
    fn advance(&mut self, record: Record, requested: u64) -> Result<Block, GuiError> {
        self.require_control(EventKind::Step)?;
        let (pane, session) = self.focused_session()?;
        let limit = self.capabilities.limits().max_steps_per_request();
        if requested > limit {
            return Err(GuiError::limit_exceeded("steps per request", limit));
        }
        let run = run_session(self.channel.as_ref(), &mut self.model, session, requested)?;
        self.model
            .record(EventKind::Step, Some(pane), Some(session));
        Ok(Block::new(
            record,
            vec![
                BlockField::uint("pane", u64::from(pane.raw())),
                BlockField::uint("session", session),
                BlockField::uint("requested", requested),
                BlockField::uint("steps", run.steps()),
                BlockField::uint("tick", run.tick()),
                BlockField::text("state_hash", &run.state_hash().to_hex()),
                BlockField::bool("halted", run.halted()),
                BlockField::text("controller", self.lease.controller().as_str()),
            ],
        ))
    }

    /// Executes `pulse`: the explicit, clock-independent frame path of the pacer.
    ///
    /// One `session.run` is issued per frame, each with `ticks_per_frame` steps, and the
    /// model's frame index advances once by the requested count (one `pulse` event). The
    /// presentation pause flag (`pause`) gates the host-time run loop of
    /// [`CommandSession::advance_elapsed`] but **not** this explicit path: a `pulse` is a
    /// requested number of frames, which is what keeps the process surface deterministic.
    fn pulse(&mut self, frames: u64) -> Result<Block, GuiError> {
        self.require_control(EventKind::Pulse)?;
        let (pane, session) = self.focused_session()?;
        let report = {
            let channel: &Arc<dyn OperationChannel> = &self.channel;
            let pacer: &mut Pacer = &mut self.pacer;
            let model: &mut GuiModel = &mut self.model;
            pacer.pulse(frames, |steps| {
                run_session(channel.as_ref(), model, session, steps).map(|_| ())
            })?
        };
        self.model.advance_frames(report.frames());
        let attached = self.model.pane(pane).ok_or(GuiError::NotAttached)?;
        let machine = attached.view().machine();
        Ok(Block::new(
            Record::Pulse,
            vec![
                BlockField::uint("pane", u64::from(pane.raw())),
                BlockField::uint("session", session),
                BlockField::uint("frames", report.frames()),
                BlockField::uint("frame", self.model.frame_index()),
                BlockField::uint("tick", machine.tick()),
                BlockField::text("state_hash", &machine.state_hash().to_hex()),
                BlockField::bool("halted", machine.halted()),
                BlockField::uint("ticks_per_frame", report.ticks_per_frame()),
                BlockField::uint("fps", u64::from(report.fps())),
                BlockField::uint("elapsed_nanos", report.elapsed_nanos()),
                BlockField::text("controller", self.lease.controller().as_str()),
            ],
        ))
    }

    /// Executes `pause`: the presentation flag only, never a canonical pause.
    fn pause(&mut self) -> Block {
        let paused = self.model.toggle_pause();
        Block::new(Record::Pause, vec![BlockField::bool("paused", paused)])
    }

    /// Executes `handoff`, appending a `handoff` event for an effective transition.
    fn handoff(&mut self, requested: Controller) -> Block {
        let report = self.lease.handoff(requested);
        if report.changed() {
            let focused = self.model.focused();
            let session =
                focused.and_then(|pane| self.model.pane(pane).map(|attached| attached.session()));
            self.model.record(EventKind::Handoff, focused, session);
        }
        Block::new(
            Record::Handoff,
            vec![
                BlockField::text("controller", report.controller().as_str()),
                BlockField::text("previous", report.previous().as_str()),
                BlockField::bool("changed", report.changed()),
                BlockField::uint("generation", report.generation()),
                BlockField::uint("sequence", report.sequence()),
            ],
        )
    }

    /// Executes `frame`: the serialized grid of the current model and lease.
    fn frame(&self) -> Result<String, GuiError> {
        text::render(&self.frame_source(), self.config.render)
    }

    /// Executes `html`: the self-contained document plus its block.
    fn html(&self) -> Result<CommandOutcome, GuiError> {
        let grid = text::frame(&self.frame_source())?;
        let document = html::render(std::slice::from_ref(&grid))?;
        let digest = hash_bytes(document.as_bytes()).to_hex();
        let block = Block::new(
            Record::Html,
            vec![
                BlockField::uint("bytes", u64::try_from(document.len()).unwrap_or(u64::MAX)),
                BlockField::text("sha256", &digest),
            ],
        );
        Ok(CommandOutcome::Html { document, block })
    }

    /// Assembles the renderer input from the model, the lease and the configuration.
    fn frame_source(&self) -> FrameSource<'_> {
        let capabilities = self.capabilities.backend().capabilities();
        FrameSource {
            model: &self.model,
            controller: self.lease.controller().as_str(),
            lease_generation: self.lease.generation(),
            lease_sequence: self.lease.sequence(),
            fps: self.pacer.config().fps(),
            timeline: self.config.timeline.as_ref(),
            rendering: self.video.availability(capabilities),
            audio: self.audio.availability(capabilities),
            width: self.config.width,
            height: self.config.height,
        }
    }

    /// Returns the focused pane and its session, or `not_attached`.
    fn focused_session(&self) -> Result<(PaneId, u64), GuiError> {
        let pane = self.model.focused().ok_or(GuiError::NotAttached)?;
        let session = self
            .model
            .pane(pane)
            .map(|attached| attached.session())
            .ok_or(GuiError::NotAttached)?;
        Ok((pane, session))
    }

    /// Applies the controller lease before a control command performs any channel call.
    ///
    /// A refusal is recorded in the presentation event log under the attempted action's
    /// kind, so "an agent was refused control" is observable, and it is returned as the
    /// typed `control_denied` failure carrying the current generation and sequence.
    fn require_control(&mut self, kind: EventKind) -> Result<(), GuiError> {
        match self.lease.authorize_control() {
            Ok(()) => Ok(()),
            Err(denied) => {
                let focused = self.model.focused();
                let session = focused
                    .and_then(|pane| self.model.pane(pane).map(|attached| attached.session()));
                self.model.record(kind, focused, session);
                Err(denied.into())
            }
        }
    }
}

/// Issues one `session.run` for `steps` canonical steps and folds the result into the model.
///
/// This is the crate's only `session.run` request, shared by `step`, `run`, `pulse` and the
/// host-time loop: the input is the documented `{session, steps}` with neutral input, and
/// the decoded [`RunView`] replaces the pane's values through [`GuiModel::apply_run`].
fn run_session(
    channel: &dyn OperationChannel,
    model: &mut GuiModel,
    session: u64,
    steps: u64,
) -> Result<RunView, GuiError> {
    let request = JsonValue::object([
        ("session", unsigned_input(session)?),
        ("steps", unsigned_input(steps)?),
    ]);
    let bytes = channel
        .call(&request_bytes("session.run", &request)?)
        .map_err(GuiError::from)?;
    let run = decode_run(&bytes)?;
    model.apply_run(&run)?;
    Ok(run)
}

/// Parses one command line's name and arguments against the closed grammar.
///
/// Every arm passes its literal name to the arity helpers, so a typed [`GuiError::BadArity`]
/// always carries a `&'static str` command name rather than the borrowed line.
fn parse(name: &str, arguments: &[&str]) -> Result<Parsed, GuiError> {
    match name {
        "create" => {
            no_arguments("create", arguments)?;
            Ok(Parsed::Create)
        }
        "list" => {
            no_arguments("list", arguments)?;
            Ok(Parsed::List)
        }
        "attach" => Ok(Parsed::Attach(unsigned_argument(
            "session",
            one_argument("attach", arguments)?,
        )?)),
        "detach" => {
            no_arguments("detach", arguments)?;
            Ok(Parsed::Detach)
        }
        "focus" => Ok(Parsed::Focus(pane_argument(one_argument(
            "focus", arguments,
        )?)?)),
        "layout" => Ok(Parsed::Layout(Layout::parse(one_argument(
            "layout", arguments,
        )?)?)),
        "panel" => {
            let (panel, visibility) = two_arguments("panel", arguments)?;
            Ok(Parsed::Panel(
                PanelKind::parse(panel)?,
                visibility_argument(visibility)?,
            ))
        }
        "step" => {
            let steps = match arguments {
                [] => 1,
                [only] => unsigned_argument("steps", only)?,
                _ => {
                    return Err(GuiError::bad_arity(
                        "step",
                        "at most one argument",
                        arguments.len(),
                    ));
                }
            };
            Ok(Parsed::Step(steps))
        }
        "run" => Ok(Parsed::Run(unsigned_argument(
            "steps",
            one_argument("run", arguments)?,
        )?)),
        "pulse" => {
            let frames = unsigned_argument("frames", one_argument("pulse", arguments)?)?;
            if frames > MAX_PULSE_FRAMES {
                return Err(GuiError::limit_exceeded("pulse frames", MAX_PULSE_FRAMES));
            }
            Ok(Parsed::Pulse(frames))
        }
        "pause" => {
            no_arguments("pause", arguments)?;
            Ok(Parsed::Pause)
        }
        "handoff" => Ok(Parsed::Handoff(Controller::parse(one_argument(
            "handoff", arguments,
        )?)?)),
        "frame" => {
            no_arguments("frame", arguments)?;
            Ok(Parsed::Frame)
        }
        "html" => {
            no_arguments("html", arguments)?;
            Ok(Parsed::Html)
        }
        "quit" => {
            no_arguments("quit", arguments)?;
            Ok(Parsed::Quit)
        }
        other => Err(GuiError::unknown_command(other)),
    }
}

/// Returns the single argument of `command`, or its typed arity failure.
fn one_argument<'a>(command: &'static str, arguments: &[&'a str]) -> Result<&'a str, GuiError> {
    match arguments {
        [only] => Ok(*only),
        _ => Err(GuiError::bad_arity(
            command,
            "exactly one argument",
            arguments.len(),
        )),
    }
}

/// Returns the two arguments of `command`, or its typed arity failure.
fn two_arguments<'a>(
    command: &'static str,
    arguments: &[&'a str],
) -> Result<(&'a str, &'a str), GuiError> {
    match arguments {
        [first, second] => Ok((*first, *second)),
        _ => Err(GuiError::bad_arity(
            command,
            "exactly two arguments",
            arguments.len(),
        )),
    }
}

/// Rejects any argument for `command`.
fn no_arguments(command: &'static str, arguments: &[&str]) -> Result<(), GuiError> {
    if arguments.is_empty() {
        return Ok(());
    }
    Err(GuiError::bad_arity(
        command,
        "no arguments",
        arguments.len(),
    ))
}

/// Parses a non-negative integer argument.
fn unsigned_argument(field: &'static str, text: &str) -> Result<u64, GuiError> {
    text.parse::<u64>()
        .map_err(|_| GuiError::bad_value(field, &format!("`{text}` is not a non-negative integer")))
}

/// Parses a 1-based pane identifier and validates its documented range.
fn pane_argument(text: &str) -> Result<PaneId, GuiError> {
    let pane = text
        .parse::<u32>()
        .map_err(|_| GuiError::bad_value("pane", &format!("`{text}` is not a pane identifier")))?;
    PaneId::new(pane)
}

/// Parses the `on`/`off` visibility argument of `panel`.
fn visibility_argument(text: &str) -> Result<bool, GuiError> {
    match text {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(GuiError::bad_value(
            "visibility",
            &format!("`{other}` is neither on nor off"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Block, BlockField, CommandConfig, QuitReason, Record, no_arguments, one_argument,
        pane_argument, parse, two_arguments, unsigned_argument, visibility_argument,
    };
    use crate::model::{Layout, PaneId, PanelKind};
    use crate::pacing::PacerConfig;
    use crate::render::RenderOptions;
    use crate::{DEFAULT_HEIGHT, DEFAULT_WIDTH, GuiErrorCode, MAX_PULSE_FRAMES};

    #[test]
    fn the_default_configuration_carries_the_documented_defaults() {
        let config = CommandConfig::default();
        assert_eq!(config.width, DEFAULT_WIDTH);
        assert_eq!(config.height, DEFAULT_HEIGHT);
        assert_eq!(config.render, RenderOptions::plain());
        assert_eq!(config.pacer, PacerConfig::default());
        assert_eq!(config.timeline, None);
    }

    #[test]
    fn the_frame_size_is_rejected_as_a_bad_option_value() {
        for width in [0, 39, 201, u32::MAX] {
            let config = CommandConfig {
                width,
                ..CommandConfig::default()
            };
            let error = config.check_frame_size().expect_err("width out of range");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains("--width"));
        }
        for height in [0, 9, 101] {
            let config = CommandConfig {
                height,
                ..CommandConfig::default()
            };
            let error = config.check_frame_size().expect_err("height out of range");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains("--height"));
        }
        assert_eq!(CommandConfig::default().check_frame_size(), Ok(()));
    }

    #[test]
    fn every_documented_command_parses_with_its_arguments() {
        assert_eq!(parse("create", &[]).expect("create"), super::Parsed::Create);
        assert_eq!(parse("list", &[]).expect("list"), super::Parsed::List);
        assert_eq!(
            parse("attach", &["12"]).expect("attach"),
            super::Parsed::Attach(12)
        );
        assert_eq!(parse("detach", &[]).expect("detach"), super::Parsed::Detach);
        assert_eq!(
            parse("focus", &["3"]).expect("focus"),
            super::Parsed::Focus(PaneId::new(3).expect("pane"))
        );
        assert_eq!(
            parse("layout", &["mosaic"]).expect("layout"),
            super::Parsed::Layout(Layout::Mosaic)
        );
        assert_eq!(
            parse("panel", &["memory", "on"]).expect("panel"),
            super::Parsed::Panel(PanelKind::Memory, true)
        );
        assert_eq!(
            parse("panel", &["events", "off"]).expect("panel"),
            super::Parsed::Panel(PanelKind::EventLog, false)
        );
        assert_eq!(parse("step", &[]).expect("step"), super::Parsed::Step(1));
        assert_eq!(
            parse("step", &["7"]).expect("step n"),
            super::Parsed::Step(7)
        );
        assert_eq!(parse("run", &["9"]).expect("run"), super::Parsed::Run(9));
        let bound = MAX_PULSE_FRAMES.to_string();
        assert_eq!(
            parse("pulse", &[bound.as_str()]).expect("pulse"),
            super::Parsed::Pulse(MAX_PULSE_FRAMES)
        );
        assert_eq!(parse("pause", &[]).expect("pause"), super::Parsed::Pause);
        assert_eq!(
            parse("handoff", &["agent"]).expect("handoff"),
            super::Parsed::Handoff(crate::control::Controller::Agent)
        );
        assert_eq!(parse("frame", &[]).expect("frame"), super::Parsed::Frame);
        assert_eq!(parse("html", &[]).expect("html"), super::Parsed::Html);
        assert_eq!(parse("quit", &[]).expect("quit"), super::Parsed::Quit);
    }

    #[test]
    fn an_unknown_command_name_is_typed_and_sanitized() {
        for name in ["bogus", "Create", "session", "\u{1b}[31m"] {
            let error = parse(name, &[]).expect_err("unknown command");
            assert_eq!(error.code(), GuiErrorCode::UnknownCommand);
            assert!(error.to_string().starts_with("unknown_command: "));
            assert!(!error.to_string().contains('\u{1b}'));
        }
    }

    #[test]
    fn wrong_arity_is_typed_for_every_command_shape() {
        assert_eq!(
            no_arguments("list", &["extra"])
                .expect_err("list takes no argument")
                .code(),
            GuiErrorCode::BadArity
        );
        assert_eq!(
            one_argument("attach", &[])
                .expect_err("attach needs a session")
                .code(),
            GuiErrorCode::BadArity
        );
        assert_eq!(
            one_argument("attach", &["1", "2"])
                .expect_err("attach takes one argument")
                .code(),
            GuiErrorCode::BadArity
        );
        assert_eq!(
            two_arguments("panel", &["memory"])
                .expect_err("panel needs two")
                .code(),
            GuiErrorCode::BadArity
        );
        assert_eq!(
            parse("step", &["1", "2"])
                .expect_err("step takes at most one")
                .code(),
            GuiErrorCode::BadArity
        );
        assert_eq!(
            parse("run", &[]).expect_err("run needs a count").code(),
            GuiErrorCode::BadArity
        );
        assert_eq!(
            parse("frame", &["now"])
                .expect_err("frame takes no argument")
                .code(),
            GuiErrorCode::BadArity
        );
        assert_eq!(
            parse("quit", &["now"])
                .expect_err("quit takes no argument")
                .code(),
            GuiErrorCode::BadArity
        );
    }

    #[test]
    fn argument_values_are_validated_with_their_documented_codes() {
        for text in ["", "-1", "1.5", "abc", "0x10"] {
            let error = unsigned_argument("steps", text).expect_err("not an integer");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains("steps"));
        }
        assert_eq!(unsigned_argument("frames", "0").expect("zero"), 0);

        for pane in ["0", "5", "4294967295"] {
            let error = pane_argument(pane).expect_err("out of range");
            assert_eq!(error.code(), GuiErrorCode::UnknownPane);
        }
        assert_eq!(
            pane_argument("abc").expect_err("not a pane").code(),
            GuiErrorCode::BadValue
        );
        assert_eq!(
            pane_argument("4").expect("pane"),
            PaneId::new(4).expect("pane")
        );

        for text in ["", "yes", "ON", "1"] {
            let error = visibility_argument(text).expect_err("not on/off");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
        }
        assert!(visibility_argument("on").expect("on"));
        assert!(!visibility_argument("off").expect("off"));

        assert_eq!(
            parse("panel", &["bogus", "on"])
                .expect_err("unknown panel")
                .code(),
            GuiErrorCode::UnknownPanel
        );
        assert_eq!(
            parse("layout", &["quad"])
                .expect_err("unknown layout")
                .code(),
            GuiErrorCode::BadValue
        );
        assert_eq!(
            parse("handoff", &["daemon"])
                .expect_err("unknown controller")
                .code(),
            GuiErrorCode::BadValue
        );
        assert_eq!(
            parse("focus", &["9"]).expect_err("unknown pane").code(),
            GuiErrorCode::UnknownPane
        );
    }

    #[test]
    fn a_pulse_above_the_documented_bound_is_a_limit_failure() {
        let too_many = (MAX_PULSE_FRAMES + 1).to_string();
        let error = parse("pulse", &[&too_many]).expect_err("above the bound");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
        assert!(error.to_string().contains("pulse frames"));
    }

    #[test]
    fn block_fields_keep_their_name_and_sanitize_text_values() {
        let uint = BlockField::uint("pane", 2);
        assert_eq!(uint.name(), "pane");
        let boolean = BlockField::bool("halted", false);
        assert_eq!(boolean.name(), "halted");
        let optional = BlockField::optional_uint("lineage_parent", None);
        assert_eq!(optional.name(), "lineage_parent");
        assert_eq!(
            optional,
            BlockField::OptionalUint("lineage_parent".into(), None)
        );

        let text = BlockField::text("backend", "chronofork.test.synthetic");
        assert_eq!(text.name(), "backend");
        assert_eq!(
            text,
            BlockField::Text("backend".into(), "chronofork.test.synthetic".to_owned())
        );

        let hostile = BlockField::text("backend", "a b\tc\u{7}d\u{e9}\u{1b}");
        assert_eq!(hostile.name(), "backend");
        assert_eq!(
            hostile,
            BlockField::Text("backend".into(), "a_b_c_d__".to_owned())
        );
    }

    #[test]
    fn an_owned_field_name_is_representable() {
        let name = format!("session.{}.state", 42);
        let field = BlockField::text(name.clone(), "ready");
        assert_eq!(field.name(), "session.42.state");
        assert_eq!(field.name(), name.as_str());
    }

    #[test]
    fn a_block_carries_its_record_name_and_field_order() {
        let block = Block::new(Record::Pause, vec![BlockField::bool("paused", true)]);
        assert_eq!(block.record, Record::Pause);
        assert_eq!(Record::Pause.as_str(), "pause");
        assert_eq!(block.fields[0].name(), "paused");

        let names: Vec<&str> = [
            Record::Create,
            Record::List,
            Record::Attach,
            Record::Detach,
            Record::Focus,
            Record::Layout,
            Record::Panel,
            Record::Step,
            Record::Run,
            Record::Pulse,
            Record::Pause,
            Record::Handoff,
            Record::Html,
            Record::Quit,
        ]
        .iter()
        .map(|record| record.as_str())
        .collect();
        assert_eq!(
            names,
            [
                "create", "list", "attach", "detach", "focus", "layout", "panel", "step", "run",
                "pulse", "pause", "handoff", "html", "quit"
            ]
        );
    }

    #[test]
    fn quit_reasons_are_the_documented_names() {
        assert_eq!(QuitReason::Line.as_str(), "line");
        assert_eq!(QuitReason::Eof.as_str(), "eof");
    }
}
