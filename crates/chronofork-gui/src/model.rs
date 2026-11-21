//! The presentation model: panes, focus, layout, panels, counters and the event log.
//!
//! [`GuiModel`] owns presentation state only. It holds **copies of decoded views**, never
//! a handle to a session, a backend or a snapshot, and it never derives a value the
//! service did not report. The one method that reaches outside is [`GuiModel::attach`],
//! the read-only attach sequence of `gui.md` §5.1: `session.inspect` plus one bounded
//! `session.read_memory` when the machine exposes memory, and nothing else. That is why
//! attach and detach cannot change emulation results — the sequence contains no mutating
//! operation — and why the property is testable at this layer with a scripted channel.
//!
//! Invariants the model maintains (each is asserted by a test):
//!
//! 1. A pane holds a view for exactly the session it names; a detach clears the slot.
//! 2. `focused` is either `None` or an attached pane; a detach moves focus to the lowest
//!    attached pane id, or to `None`.
//! 3. A pane slot's `generation` increments on every effective attach into it and never
//!    decreases; an idempotent re-attach of an already attached session only focuses and
//!    leaves the generation untouched.
//! 4. `frame_index` increments by exactly the number of presentation frames advanced.
//! 5. No model field is derived from a host clock: the only time value in the crate is the
//!    pacer's report, which never reaches the model.
//! 6. The event log length never exceeds its capacity, and the dropped counter accounts for
//!    every evicted entry.
//!
//! [`Rect`], [`SplitAxis`], [`split`] and [`Mosaic`] carry the deterministic frame geometry:
//! integer cell arithmetic with a typed result, no overflow, no panic, and row-major region
//! order so the same model always renders the same bytes.

use std::collections::{BTreeMap, VecDeque};

use chronofork_service::json::JsonValue;

use crate::channel::{OperationChannel, request_bytes, unsigned_input};
use crate::view::{
    CreateView, MemoryView, RunView, SessionView, decode_create, decode_memory, decode_session_view,
};
use crate::{EVENT_LOG_CAPACITY, GuiError, MAX_MEMORY_PANEL_BYTES, MAX_PANES};

/// Identifies one window: one rendered surface (one stdout, one HTML artifact).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WindowId(u64);

impl WindowId {
    /// Creates a window identifier from its raw value.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw identifier.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Identifies one pane inside a window (`1..=MAX_PANES`, allocated lowest-free-first).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PaneId(u32);

impl PaneId {
    /// Every valid pane identifier in ascending order.
    ///
    /// The length is [`MAX_PANES`] by construction, which a test asserts, so iteration
    /// over panes never needs a cast or an out-of-range check.
    pub const ALL: [Self; MAX_PANES] = [Self(1), Self(2), Self(3), Self(4)];

    /// Creates a pane identifier after validating its range.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::UnknownPane`] for `0` and for every value above
    /// [`MAX_PANES`]; a pane identifier outside that range names no pane, so it is not a
    /// value error but an unknown pane.
    pub const fn new(raw: u32) -> Result<Self, GuiError> {
        if raw == 0 || raw as usize > MAX_PANES {
            return Err(GuiError::UnknownPane { pane: raw });
        }
        Ok(Self(raw))
    }

    /// Returns the 1-based raw identifier.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Returns the zero-based slot index.
    const fn index(self) -> usize {
        (self.0 - 1) as usize
    }
}

/// The layout of a window: one focused pane or the documented 2x2 mosaic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layout {
    /// Render the focused pane only.
    Single,
    /// Render up to four panes in a 2x2 grid, row-major.
    Mosaic,
}

impl Layout {
    /// Returns the canonical command name of this layout.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Mosaic => "mosaic",
        }
    }

    /// Parses a layout argument.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::BadValue`] for any other name.
    pub fn parse(name: &str) -> Result<Self, GuiError> {
        match name {
            "single" => Ok(Self::Single),
            "mosaic" => Ok(Self::Mosaic),
            other => Err(GuiError::bad_value(
                "layout",
                &format!("`{other}` is neither single nor mosaic"),
            )),
        }
    }
}

/// One addressable panel.
///
/// The eight variants are exactly the documented panel names and their declaration order
/// is the render order, so a capabilities panel, a focus rule or a test can rely on one
/// source of truth. Video and audio are **not** panels in v1: their availability is
/// reported by the frame status line, and the process grammar rejects an unknown panel
/// name, so no unaddressable variant exists.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PanelKind {
    /// Session status line.
    Status,
    /// Session lineage.
    Lineage,
    /// Backend capability flags.
    Capabilities,
    /// Inspection registers.
    Registers,
    /// Bounded memory window.
    Memory,
    /// Debugger inspector sections, section by section.
    Inspector,
    /// TAS timeline, when the CLI supplied a movie.
    Timeline,
    /// Bounded presentation event log.
    EventLog,
}

impl PanelKind {
    /// Every addressable panel in render order.
    pub const ALL: [Self; 8] = [
        Self::Status,
        Self::Lineage,
        Self::Capabilities,
        Self::Registers,
        Self::Memory,
        Self::Inspector,
        Self::Timeline,
        Self::EventLog,
    ];

    /// Returns the canonical command and block name of this panel.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Lineage => "lineage",
            Self::Capabilities => "capabilities",
            Self::Registers => "registers",
            Self::Memory => "memory",
            Self::Inspector => "inspectors",
            Self::Timeline => "timeline",
            Self::EventLog => "events",
        }
    }

    /// Parses a panel argument.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::UnknownPanel`] for any other name.
    pub fn parse(name: &str) -> Result<Self, GuiError> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| GuiError::unknown_panel(name))
    }

    /// Returns whether the panel is visible when a window is created.
    ///
    /// The documented default set is `status`, `lineage`, `capabilities`, `registers` and
    /// `events`; `memory`, `inspectors` and `timeline` start hidden.
    #[must_use]
    pub const fn default_visible(self) -> bool {
        matches!(
            self,
            Self::Status | Self::Lineage | Self::Capabilities | Self::Registers | Self::EventLog
        )
    }
}

/// The presentation state of one panel.
///
/// Visibility is the whole state. The v1 renderers draw one clipped line per visible panel
/// and the process grammar has no scroll command, so a scroll offset would be state no
/// surface can set or read; it is deliberately absent rather than carried unused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanelState {
    visible: bool,
}

impl PanelState {
    /// Creates the state of a panel with the given visibility.
    #[must_use]
    pub const fn new(visible: bool) -> Self {
        Self { visible }
    }

    /// Returns whether the panel is visible.
    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// Sets the visibility.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
}

/// The kind of one presentation event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    /// A session was created through `session.create`.
    Create,
    /// A session was attached to a pane.
    Attach,
    /// A pane was detached.
    Detach,
    /// A pane was focused.
    Focus,
    /// The layout changed.
    Layout,
    /// A panel's visibility changed.
    Panel,
    /// A canonical step was performed.
    Step,
    /// A presentation frame advanced.
    Pulse,
    /// The presentation pause flag changed.
    Pause,
    /// The controller lease changed hands.
    Handoff,
}

impl EventKind {
    /// Returns the canonical block name of this event.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Attach => "attach",
            Self::Detach => "detach",
            Self::Focus => "focus",
            Self::Layout => "layout",
            Self::Panel => "panel",
            Self::Step => "step",
            Self::Pulse => "pulse",
            Self::Pause => "pause",
            Self::Handoff => "handoff",
        }
    }
}

/// One presentation event.
///
/// It records a presentation fact only: no host time, no path and no token can reach it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Event {
    sequence: u64,
    kind: EventKind,
    pane: Option<PaneId>,
    session: Option<u64>,
}

impl Event {
    /// Returns the monotonic sequence number.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the event kind.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        self.kind
    }

    /// Returns the pane the event concerns, when it concerns one.
    #[must_use]
    pub const fn pane(&self) -> Option<PaneId> {
        self.pane
    }

    /// Returns the session the event concerns, when it concerns one.
    #[must_use]
    pub const fn session(&self) -> Option<u64> {
        self.session
    }
}

/// A bounded event log with monotonic sequence numbers and oldest-first eviction.
#[derive(Debug)]
pub struct EventLog {
    entries: VecDeque<Event>,
    next: u64,
    dropped: u64,
    capacity: usize,
}

impl EventLog {
    /// Creates a log that keeps at most `capacity` entries.
    ///
    /// The capacity is normalized to `1..=EVENT_LOG_CAPACITY`: zero would keep no event
    /// at all, and a larger request would exceed the documented bound that keeps the log
    /// a bounded resource. The normalization is total, so building a model cannot fail
    /// for a capacity it does not own.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.clamp(1, EVENT_LOG_CAPACITY);
        Self {
            entries: VecDeque::with_capacity(capacity),
            next: 0,
            dropped: 0,
            capacity,
        }
    }

    /// Returns the applied capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of retained entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the log holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of entries evicted to stay within the capacity.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Returns the retained entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &Event> + '_ {
        self.entries.iter()
    }

    /// Returns the newest retained entry.
    #[must_use]
    pub fn last(&self) -> Option<&Event> {
        self.entries.back()
    }

    /// Appends one event, evicting the oldest entry when the log is full.
    fn push(&mut self, kind: EventKind, pane: Option<PaneId>, session: Option<u64>) -> Event {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        let event = Event {
            sequence: self.next,
            kind,
            pane,
            session,
        };
        self.next = self.next.saturating_add(1);
        self.entries.push_back(event);
        event
    }
}

/// The presentation state of one attached session.
#[derive(Clone, Debug)]
pub struct Pane {
    session: u64,
    generation: u64,
    view: SessionView,
    memory: Option<MemoryView>,
}

impl Pane {
    /// Returns the session this pane holds.
    #[must_use]
    pub const fn session(&self) -> u64 {
        self.session
    }

    /// Returns the attach generation of this pane slot.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the last decoded session view.
    #[must_use]
    pub const fn view(&self) -> &SessionView {
        &self.view
    }

    /// Returns the bounded memory window captured at attach time, when the machine
    /// exposes memory.
    #[must_use]
    pub const fn memory(&self) -> Option<&MemoryView> {
        self.memory.as_ref()
    }

    /// Applies the values a `session.run` result reports for this pane's session.
    fn apply_run(&mut self, run: &RunView) {
        self.view.apply_run(run);
    }
}

/// The result of one effective attach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttachReport {
    /// The pane the session is attached to.
    pub pane: PaneId,
    /// The pane slot's generation after the attach.
    pub generation: u64,
    /// Whether the session was already attached and only gained focus.
    pub reused: bool,
}

/// The result of one detach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetachReport {
    /// The pane that was cleared.
    pub pane: PaneId,
    /// The session that was detached.
    pub session: u64,
}

/// One cell rectangle inside a frame grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    /// Column of the left edge.
    pub x: u32,
    /// Row of the top edge.
    pub y: u32,
    /// Width in cells.
    pub width: u32,
    /// Height in cells.
    pub height: u32,
}

impl Rect {
    /// Creates a rectangle from its cell coordinates.
    #[must_use]
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns whether the rectangle covers no cell.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Returns the number of cells in the rectangle.
    #[must_use]
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// The axis of a mosaic split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitAxis {
    /// A horizontal cut, producing a top and a bottom rectangle.
    Horizontal,
    /// A vertical cut, producing a left and a right rectangle.
    Vertical,
}

/// Splits `area` into two rectangles along `axis`.
///
/// The first part receives `ratio_percent` of the axis length rounded down and at least
/// one cell; the second part receives the remainder. The arithmetic is integer-only and
/// saturating, so no input can overflow or panic.
///
/// # Errors
///
/// Returns [`GuiError::BadValue`] when `ratio_percent` is outside `1..=99` and
/// [`GuiError::LimitExceeded`] when the axis is shorter than the two cells a split needs.
pub fn split(area: Rect, axis: SplitAxis, ratio_percent: u32) -> Result<(Rect, Rect), GuiError> {
    if !(1..=99).contains(&ratio_percent) {
        return Err(GuiError::bad_value(
            "split ratio",
            "a split ratio must be between 1 and 99 percent",
        ));
    }
    let length = match axis {
        SplitAxis::Horizontal => area.height,
        SplitAxis::Vertical => area.width,
    };
    if length < 2 {
        return Err(GuiError::limit_exceeded("split minimum cells", 2));
    }
    let first = scaled(length, ratio_percent).max(1).min(length - 1);
    let second = length - first;
    Ok(match axis {
        SplitAxis::Horizontal => (
            Rect::new(area.x, area.y, area.width, first),
            Rect::new(area.x, area.y.saturating_add(first), area.width, second),
        ),
        SplitAxis::Vertical => (
            Rect::new(area.x, area.y, first, area.height),
            Rect::new(area.x.saturating_add(first), area.y, second, area.height),
        ),
    })
}

/// Returns `length * ratio_percent / 100` using integer-only saturating arithmetic.
fn scaled(length: u32, ratio_percent: u32) -> u32 {
    let scaled = u64::from(length) * u64::from(ratio_percent) / 100;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// The four regions of the mosaic layout, in row-major order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mosaic {
    regions: [Rect; 4],
}

impl Mosaic {
    /// Splits `area` into four regions with the documented 50/50 quadrants.
    ///
    /// # Errors
    ///
    /// Propagates the typed [`split`] errors, which a validated frame size never
    /// produces: a frame of at least `MIN_WIDTH` x `MIN_HEIGHT` always leaves at least two
    /// rows and two columns of pane area.
    pub fn quadrants(area: Rect) -> Result<Self, GuiError> {
        Self::new(area, 50, 50)
    }

    /// Splits `area` into four regions: one horizontal cut, then one vertical cut per row.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::BadValue`] for a ratio outside `1..=99` and
    /// [`GuiError::LimitExceeded`] when a region would be smaller than one cell.
    pub fn new(area: Rect, row_ratio: u32, column_ratio: u32) -> Result<Self, GuiError> {
        let (top, bottom) = split(area, SplitAxis::Horizontal, row_ratio)?;
        let (top_left, top_right) = split(top, SplitAxis::Vertical, column_ratio)?;
        let (bottom_left, bottom_right) = split(bottom, SplitAxis::Vertical, column_ratio)?;
        Ok(Self {
            regions: [top_left, top_right, bottom_left, bottom_right],
        })
    }

    /// Returns the four regions in row-major order (top-left, top-right, bottom-left,
    /// bottom-right).
    #[must_use]
    pub const fn regions(&self) -> [Rect; 4] {
        self.regions
    }

    /// Returns one region by row-major index.
    #[must_use]
    pub fn region(&self, index: usize) -> Option<Rect> {
        self.regions.get(index).copied()
    }
}

/// The presentation model of one window.
///
/// The controller lease is deliberately **not** model state: [`crate::control`] owns the
/// lease and its counters, and the renderer receives it beside the model, so the handoff
/// generation and sequence have exactly one owner and a `handoff` can never be observed
/// half-applied through a stale copy.
#[derive(Debug)]
pub struct GuiModel {
    window: WindowId,
    panes: [Option<Pane>; MAX_PANES],
    slot_generations: [u64; MAX_PANES],
    focused: Option<PaneId>,
    layout: Layout,
    panels: BTreeMap<PanelKind, PanelState>,
    frame_index: u64,
    paused: bool,
    events: EventLog,
}

impl GuiModel {
    /// Creates an empty window with the documented panel defaults and the documented
    /// event-log capacity.
    #[must_use]
    pub fn new(window: WindowId) -> Self {
        Self::with_event_capacity(window, EVENT_LOG_CAPACITY)
    }

    /// Creates an empty window with an explicit event-log capacity.
    ///
    /// Used by tests that need to observe eviction; the capacity is normalized to
    /// `1..=EVENT_LOG_CAPACITY`.
    #[must_use]
    pub fn with_event_capacity(window: WindowId, capacity: usize) -> Self {
        let mut panels = BTreeMap::new();
        for kind in PanelKind::ALL {
            panels.insert(kind, PanelState::new(kind.default_visible()));
        }
        Self {
            window,
            panes: std::array::from_fn(|_| None),
            slot_generations: [0; MAX_PANES],
            focused: None,
            layout: Layout::Single,
            panels,
            frame_index: 0,
            paused: false,
            events: EventLog::new(capacity),
        }
    }

    /// Returns the window identifier.
    #[must_use]
    pub const fn window(&self) -> WindowId {
        self.window
    }

    /// Returns the current layout.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// Returns the focused pane, when one is attached.
    #[must_use]
    pub const fn focused(&self) -> Option<PaneId> {
        self.focused
    }

    /// Returns the pane holding `id`, when one is attached there.
    #[must_use]
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes[id.index()].as_ref()
    }

    /// Returns the attached panes in ascending pane id order.
    pub fn panes(&self) -> impl Iterator<Item = (PaneId, &Pane)> + '_ {
        PaneId::ALL.into_iter().filter_map(move |pane| {
            self.panes[pane.index()]
                .as_ref()
                .map(|attached| (pane, attached))
        })
    }

    /// Returns the number of attached panes.
    #[must_use]
    pub fn attached(&self) -> usize {
        self.panes.iter().filter(|slot| slot.is_some()).count()
    }

    /// Returns the presentation state of one panel.
    ///
    /// An absent entry yields the documented default state, which is exactly what
    /// [`GuiModel::new`] seeds, so a renderer never has to handle a missing panel.
    #[must_use]
    pub fn panel(&self, kind: PanelKind) -> PanelState {
        self.panels
            .get(&kind)
            .copied()
            .unwrap_or(PanelState::new(kind.default_visible()))
    }

    /// Returns the visible panels in render order.
    pub fn visible_panels(&self) -> impl Iterator<Item = PanelKind> + '_ {
        PanelKind::ALL
            .into_iter()
            .filter(|kind| self.panel(*kind).visible())
    }

    /// Returns the presentation frame index.
    #[must_use]
    pub const fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// Returns whether presentation is paused.
    ///
    /// This is presentation state only: the M6 catalog has no pause operation and the
    /// model never claims to pause a canonical session.
    #[must_use]
    pub const fn paused(&self) -> bool {
        self.paused
    }

    /// Returns the presentation event log.
    #[must_use]
    pub const fn events(&self) -> &EventLog {
        &self.events
    }

    /// Appends one presentation event and returns it.
    ///
    /// The command and control layers record `step`, `pulse`, `pause` and `handoff`
    /// through this entry point, so the model stays the one owner of the log.
    pub fn record(&mut self, kind: EventKind, pane: Option<PaneId>, session: Option<u64>) -> Event {
        self.events.push(kind, pane, session)
    }

    /// Creates one session through `session.create` and records a `create` event.
    ///
    /// The returned view carries a `backend_id` only; the model gains no descriptor and no
    /// pane, because a descriptor comes from `session.inspect` or `capabilities`.
    ///
    /// # Errors
    ///
    /// Propagates the channel failure and the strict decoding of the operation result.
    pub fn create(&mut self, channel: &dyn OperationChannel) -> Result<CreateView, GuiError> {
        let request = JsonValue::Object(BTreeMap::new());
        let bytes = channel
            .call(&request_bytes("session.create", &request)?)
            .map_err(GuiError::from)?;
        let view = decode_create(&bytes)?;
        self.record(EventKind::Create, None, Some(view.session()));
        Ok(view)
    }

    /// Performs the read-only attach sequence and attaches the session to a pane.
    ///
    /// The sequence is exactly `gui.md` §5.1: one `session.inspect`, plus one
    /// `session.read_memory` bounded by [`MAX_MEMORY_PANEL_BYTES`] when the machine
    /// exposes memory. Nothing else is called, so attaching cannot change an emulation
    /// result. An already attached session is only focused and keeps its generation.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::LimitExceeded`] when four panes are already attached (the model
    /// is left unchanged, and no call is made), and propagates the channel failure and the
    /// strict decoding of both results.
    pub fn attach(
        &mut self,
        channel: &dyn OperationChannel,
        session: u64,
    ) -> Result<AttachReport, GuiError> {
        let already_attached = self
            .panes()
            .find(|(_, attached)| attached.session() == session)
            .map(|(pane, attached)| (pane, attached.generation()));
        if let Some((pane, generation)) = already_attached {
            self.focused = Some(pane);
            self.record(EventKind::Focus, Some(pane), Some(session));
            return Ok(AttachReport {
                pane,
                generation,
                reused: true,
            });
        }
        let pane = self.free_pane().ok_or_else(|| {
            GuiError::limit_exceeded(
                "attached panes",
                u64::try_from(MAX_PANES).unwrap_or(u64::MAX),
            )
        })?;
        let request = JsonValue::object([("session", unsigned_input(session)?)]);
        let bytes = channel
            .call(&request_bytes("session.inspect", &request)?)
            .map_err(GuiError::from)?;
        let view = decode_session_view(&bytes)?;
        let memory = if view.machine().memory_length() > 0 {
            let length = view
                .machine()
                .memory_length()
                .min(u64::try_from(MAX_MEMORY_PANEL_BYTES).unwrap_or(u64::MAX));
            let request = JsonValue::object([
                ("address", unsigned_input(view.machine().memory_base())?),
                ("length", unsigned_input(length)?),
                ("session", unsigned_input(session)?),
            ]);
            let bytes = channel
                .call(&request_bytes("session.read_memory", &request)?)
                .map_err(GuiError::from)?;
            Some(decode_memory(&bytes)?)
        } else {
            None
        };
        let index = pane.index();
        let generation = self.slot_generations[index].saturating_add(1);
        self.slot_generations[index] = generation;
        self.panes[index] = Some(Pane {
            session,
            generation,
            view,
            memory,
        });
        self.focused = Some(pane);
        self.record(EventKind::Attach, Some(pane), Some(session));
        Ok(AttachReport {
            pane,
            generation,
            reused: false,
        })
    }

    /// Detaches the focused pane and moves focus to the lowest attached pane.
    ///
    /// No operation is called: detaching is a purely presentational act, which is why it
    /// cannot perturb an emulation result.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::NotAttached`] when no pane is focused.
    pub fn detach(&mut self) -> Result<DetachReport, GuiError> {
        let pane = self.focused.ok_or(GuiError::NotAttached)?;
        let Some(detached) = self.panes[pane.index()].take() else {
            self.focused = None;
            return Err(GuiError::NotAttached);
        };
        self.record(EventKind::Detach, Some(pane), Some(detached.session()));
        let next = self.panes().next().map(|(pane, _)| pane);
        self.focused = next;
        Ok(DetachReport {
            pane,
            session: detached.session(),
        })
    }

    /// Focuses an attached pane and records a `focus` event.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::UnknownPane`] when the pane holds no session.
    pub fn focus(&mut self, pane: PaneId) -> Result<(), GuiError> {
        let session = match &self.panes[pane.index()] {
            Some(attached) => attached.session(),
            None => return Err(GuiError::unknown_pane(pane.raw())),
        };
        self.focused = Some(pane);
        self.record(EventKind::Focus, Some(pane), Some(session));
        Ok(())
    }

    /// Sets the layout and records a `layout` event.
    pub fn set_layout(&mut self, layout: Layout) {
        let focused = self.focused;
        let session = focused.and_then(|pane| self.panes[pane.index()].as_ref().map(Pane::session));
        self.layout = layout;
        self.record(EventKind::Layout, focused, session);
    }

    /// Sets a panel's visibility and records a `panel` event.
    pub fn set_panel(&mut self, kind: PanelKind, visible: bool) {
        self.panels
            .entry(kind)
            .or_insert(PanelState::new(kind.default_visible()))
            .set_visible(visible);
        let focused = self.focused;
        let session = focused.and_then(|pane| self.panes[pane.index()].as_ref().map(Pane::session));
        self.record(EventKind::Panel, focused, session);
    }

    /// Applies the values a `session.run` result reports to the pane holding that session.
    ///
    /// The operation returns the performed step count, the new tick, the new state hash and
    /// the halt flag; those four values replace the pane's copy and nothing else is
    /// derived.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::NotAttached`] when the session named by the result is not
    /// attached to any pane.
    pub fn apply_run(&mut self, run: &RunView) -> Result<(), GuiError> {
        for attached in self.panes.iter_mut().flatten() {
            if attached.session() == run.session() {
                attached.apply_run(run);
                return Ok(());
            }
        }
        Err(GuiError::NotAttached)
    }

    /// Advances the presentation frame index and records one `pulse` event.
    ///
    /// Call this once per presentation pulse: the index grows by exactly the number of
    /// frames advanced, and a zero-frame advance records nothing.
    pub fn advance_frames(&mut self, frames: u64) {
        if frames == 0 {
            return;
        }
        self.frame_index = self.frame_index.saturating_add(frames);
        let focused = self.focused;
        let session = focused.and_then(|pane| self.panes[pane.index()].as_ref().map(Pane::session));
        self.record(EventKind::Pulse, focused, session);
    }

    /// Toggles the presentation pause flag, records a `pause` event and returns the new
    /// state.
    pub fn toggle_pause(&mut self) -> bool {
        self.paused = !self.paused;
        let paused = self.paused;
        let focused = self.focused;
        let session = focused.and_then(|pane| self.panes[pane.index()].as_ref().map(Pane::session));
        self.record(EventKind::Pause, focused, session);
        paused
    }

    /// Returns the lowest pane slot that holds no session.
    fn free_pane(&self) -> Option<PaneId> {
        PaneId::ALL
            .into_iter()
            .find(|pane| self.panes[pane.index()].is_none())
    }
}

#[cfg(test)]
mod tests {
    use chronofork_service::ServiceLimits;
    use chronofork_service::json::{JsonValue, to_canonical_bytes};

    use super::{
        EventKind, EventLog, GuiModel, Layout, MAX_PANES, Mosaic, PaneId, PanelKind, PanelState,
        Rect, SplitAxis, WindowId, split,
    };
    use crate::view::{RunView, decode_run};
    use crate::{EVENT_LOG_CAPACITY, GuiErrorCode};

    const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";

    fn window() -> WindowId {
        WindowId::from_raw(1)
    }

    fn run_view(session: u64, tick: u64) -> RunView {
        let result = JsonValue::object([
            (
                "session",
                JsonValue::int(i64::try_from(session).expect("session")),
            ),
            ("steps", JsonValue::int(4)),
            ("performed", JsonValue::int(4)),
            ("tick", JsonValue::int(i64::try_from(tick).expect("tick"))),
            ("state_hash", JsonValue::string(HASH)),
            ("halted", JsonValue::from_bool(false)),
            ("stop_reason", JsonValue::string("completed")),
            ("revision", JsonValue::int(1)),
            ("max_wall_ms", JsonValue::int(30_000)),
            // Operational metadata the v2 run result carries (ADR-0016 §2).
            ("process_epoch", JsonValue::int(7)),
        ]);
        let envelope =
            JsonValue::object([("op", JsonValue::string("session.run")), ("result", result)]);
        let bytes = to_canonical_bytes(&envelope, &ServiceLimits::default()).expect("canonical");
        decode_run(&bytes).expect("run view")
    }

    #[test]
    fn pane_ids_cover_the_documented_range_and_nothing_else() {
        assert_eq!(PaneId::ALL.len(), MAX_PANES);
        assert_eq!(PaneId::ALL[0].raw(), 1);
        assert_eq!(PaneId::ALL[MAX_PANES - 1].raw(), MAX_PANES as u32);
        for raw in [0, MAX_PANES as u32 + 1, u32::MAX] {
            let error = PaneId::new(raw).expect_err("out of range");
            assert_eq!(error.code(), GuiErrorCode::UnknownPane);
            assert!(matches!(error, crate::GuiError::UnknownPane { pane } if pane == raw));
        }
        for pane in PaneId::ALL {
            assert_eq!(PaneId::new(pane.raw()).expect("valid pane"), pane);
        }
    }

    #[test]
    fn layouts_and_panels_parse_exactly_the_documented_names() {
        assert_eq!(Layout::parse("single").expect("single"), Layout::Single);
        assert_eq!(Layout::parse("mosaic").expect("mosaic"), Layout::Mosaic);
        for name in ["", "Mosaic", "quad", "single "] {
            let error = Layout::parse(name).expect_err("unknown layout");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
        }
        assert_eq!(Layout::Single.as_str(), "single");
        assert_eq!(Layout::Mosaic.as_str(), "mosaic");

        let names: Vec<&str> = PanelKind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(
            names,
            [
                "status",
                "lineage",
                "capabilities",
                "registers",
                "memory",
                "inspectors",
                "timeline",
                "events"
            ]
        );
        for kind in PanelKind::ALL {
            assert_eq!(PanelKind::parse(kind.name()).expect("panel"), kind);
        }
        for name in ["", "video", "audio", "inspector", "event_log", "Status"] {
            let error = PanelKind::parse(name).expect_err("unknown panel");
            assert_eq!(error.code(), GuiErrorCode::UnknownPanel);
        }
    }

    #[test]
    fn the_documented_panel_defaults_are_seeded() {
        let model = GuiModel::new(window());
        for kind in PanelKind::ALL {
            assert_eq!(
                model.panel(kind).visible(),
                kind.default_visible(),
                "{}",
                kind.name()
            );
        }
        let visible: Vec<&str> = model.visible_panels().map(PanelKind::name).collect();
        assert_eq!(
            visible,
            ["status", "lineage", "capabilities", "registers", "events"]
        );
        assert!(!model.panel(PanelKind::Memory).visible());
        assert!(!model.panel(PanelKind::Inspector).visible());
        assert!(!model.panel(PanelKind::Timeline).visible());
    }

    #[test]
    fn a_new_window_attaches_nothing_and_holds_no_host_state() {
        let model = GuiModel::new(window());
        assert_eq!(model.window(), window());
        assert_eq!(model.attached(), 0);
        assert!(model.panes().next().is_none());
        assert_eq!(model.focused(), None);
        assert_eq!(model.layout(), Layout::Single);
        assert_eq!(model.frame_index(), 0);
        assert!(!model.paused());
        assert!(model.events().is_empty());
        assert_eq!(model.events().capacity(), EVENT_LOG_CAPACITY);
    }

    #[test]
    fn panel_visibility_is_recorded() {
        let mut state = PanelState::new(false);
        assert!(!state.visible());
        state.set_visible(true);
        assert!(state.visible());
        state.set_visible(false);
        assert!(!state.visible());

        // The seeded default of every documented panel is the model's value.
        let mut model = GuiModel::new(window());
        assert!(model.panel(PanelKind::Status).visible());
        model.set_panel(PanelKind::Status, false);
        assert!(!model.panel(PanelKind::Status).visible());
    }

    #[test]
    fn the_event_log_is_monotonic_bounded_and_counts_dropped_entries() {
        let mut log = EventLog::new(3);
        assert_eq!(log.capacity(), 3);
        for index in 0..5_u64 {
            let event = log.push(EventKind::Attach, Some(PaneId::ALL[0]), Some(index));
            assert_eq!(event.sequence(), index);
            assert_eq!(event.kind(), EventKind::Attach);
        }
        assert_eq!(log.len(), 3);
        assert_eq!(log.dropped(), 2);
        let sequences: Vec<u64> = log.entries().map(|event| event.sequence()).collect();
        assert_eq!(sequences, [2, 3, 4]);
        assert_eq!(log.last().map(super::Event::sequence), Some(4));
        assert_eq!(log.last().and_then(super::Event::session), Some(4));

        let mut tiny = EventLog::new(1);
        tiny.push(EventKind::Focus, Some(PaneId::ALL[1]), Some(1));
        assert_eq!(tiny.capacity(), 1);
        assert_eq!(tiny.len(), 1);
        assert_eq!(tiny.dropped(), 0);
        tiny.push(EventKind::Detach, Some(PaneId::ALL[1]), Some(1));
        assert_eq!(tiny.len(), 1);
        assert_eq!(tiny.dropped(), 1);
        assert_eq!(tiny.last().map(super::Event::kind), Some(EventKind::Detach));

        assert_eq!(EventLog::new(0).capacity(), 1);
        assert_eq!(EventLog::new(4096).capacity(), EVENT_LOG_CAPACITY);
    }

    #[test]
    fn model_events_are_recorded_for_every_presentation_change() {
        let mut model = GuiModel::new(window());
        model.set_layout(Layout::Mosaic);
        assert_eq!(model.layout(), Layout::Mosaic);
        model.set_panel(PanelKind::Timeline, true);
        assert!(model.panel(PanelKind::Timeline).visible());
        assert!(model.toggle_pause());
        assert!(!model.toggle_pause());
        model.advance_frames(0);
        model.advance_frames(2);

        assert_eq!(model.frame_index(), 2);
        assert!(!model.paused());
        let kinds: Vec<EventKind> = model.events().entries().map(super::Event::kind).collect();
        assert_eq!(
            kinds,
            [
                EventKind::Layout,
                EventKind::Panel,
                EventKind::Pause,
                EventKind::Pause,
                EventKind::Pulse
            ]
        );
        let sequences: Vec<u64> = model
            .events()
            .entries()
            .map(super::Event::sequence)
            .collect();
        assert_eq!(sequences, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn focus_and_detach_report_typed_failures_on_an_empty_window() {
        let mut model = GuiModel::new(window());
        assert_eq!(
            model.detach().expect_err("nothing attached").code(),
            GuiErrorCode::NotAttached
        );
        for pane in PaneId::ALL {
            assert_eq!(
                model.focus(pane).expect_err("empty pane").code(),
                GuiErrorCode::UnknownPane
            );
        }
        assert_eq!(model.attached(), 0);
    }

    #[test]
    fn applying_a_run_to_an_unattached_session_is_typed() {
        let mut model = GuiModel::new(window());
        assert_eq!(
            model
                .apply_run(&run_view(1, 4))
                .expect_err("not attached")
                .code(),
            GuiErrorCode::NotAttached
        );
        assert_eq!(model.frame_index(), 0);
    }

    #[test]
    fn a_split_is_total_and_keeps_every_cell() {
        let area = Rect::new(0, 2, 40, 7);
        let (top, bottom) = split(area, SplitAxis::Horizontal, 50).expect("split");
        assert_eq!(top, Rect::new(0, 2, 40, 3));
        assert_eq!(bottom, Rect::new(0, 5, 40, 4));
        assert_eq!(top.height + bottom.height, area.height);
        assert_eq!(top.area() + bottom.area(), area.area());

        let (left, right) = split(area, SplitAxis::Vertical, 25).expect("split");
        assert_eq!(left, Rect::new(0, 2, 10, 7));
        assert_eq!(right, Rect::new(10, 2, 30, 7));
        assert_eq!(left.width + right.width, area.width);

        // A ratio that would round to zero still reserves one cell for the first part.
        let (thin, rest) = split(area, SplitAxis::Vertical, 1).expect("minimum");
        assert_eq!(thin.width, 1);
        assert_eq!(rest.width, 39);

        // Extreme rectangles saturate instead of overflowing.
        let huge = Rect::new(u32::MAX - 1, u32::MAX - 1, u32::MAX, u32::MAX);
        let (first, second) = split(huge, SplitAxis::Horizontal, 99).expect("huge split");
        assert_eq!(second.y, u32::MAX);
        assert_eq!(first.height + second.height, huge.height);
    }

    #[test]
    fn a_split_rejects_a_bad_ratio_and_an_axis_shorter_than_two_cells() {
        let area = Rect::new(0, 0, 10, 10);
        for ratio in [0, 100, 101, u32::MAX] {
            let error = split(area, SplitAxis::Horizontal, ratio).expect_err("bad ratio");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
        }
        for area in [Rect::new(0, 0, 10, 1), Rect::new(0, 0, 1, 10)] {
            let axis = if area.height < 2 {
                SplitAxis::Horizontal
            } else {
                SplitAxis::Vertical
            };
            let error = split(area, axis, 50).expect_err("axis too short");
            assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
        }
    }

    #[test]
    fn mosaic_regions_are_row_major_and_cover_the_area() {
        let area = Rect::new(0, 2, 40, 7);
        let mosaic = Mosaic::quadrants(area).expect("mosaic");
        let regions = mosaic.regions();
        assert_eq!(regions[0], Rect::new(0, 2, 20, 3));
        assert_eq!(regions[1], Rect::new(20, 2, 20, 3));
        assert_eq!(regions[2], Rect::new(0, 5, 20, 4));
        assert_eq!(regions[3], Rect::new(20, 5, 20, 4));
        assert_eq!(mosaic.region(0), Some(regions[0]));
        assert_eq!(mosaic.region(4), None);
        let covered: u64 = regions.iter().map(|region| region.area()).sum();
        assert_eq!(covered, area.area());

        let asymmetric = Mosaic::new(area, 25, 75).expect("asymmetric mosaic");
        let regions = asymmetric.regions();
        assert_eq!(regions[0], Rect::new(0, 2, 30, 1));
        assert_eq!(regions[1], Rect::new(30, 2, 10, 1));
        let covered: u64 = regions.iter().map(|region| region.area()).sum();
        assert_eq!(covered, area.area());
    }

    #[test]
    fn a_mosaic_rejects_a_region_smaller_than_one_cell() {
        let error = Mosaic::quadrants(Rect::new(0, 0, 1, 10)).expect_err("too narrow");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
        let error = Mosaic::quadrants(Rect::new(0, 0, 40, 1)).expect_err("too short");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    }
}
