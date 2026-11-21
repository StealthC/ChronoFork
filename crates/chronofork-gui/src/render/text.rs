//! Plain and ANSI text renderers.
//!
//! This module renders a [`Frame`] from the window model and the active controller lease:
//! the title line, the separator, the pane regions (single or 2x2 mosaic, row-major) and
//! the status line of `gui.md` §4. Every line is exactly `width` printable ASCII columns,
//! space padded, clipped and never wrapped; a character a view reports outside printable
//! ASCII renders as `?` (the grid's write rule).
//!
//! ANSI mode inserts **SGR emphasis only** — `ESC[1m` around the title and the pane headers
//! with `ESC[0m` after — and never cursor movement, clearing, an alternate screen or a
//! terminal size query. Removing the SGR sequences from an ANSI frame yields the plain
//! frame byte for byte, which is an assertion (a test), not a promise.
//!
//! A frame is a pure function of a [`FrameSource`]: the model, the lease counters the
//! command layer passes in, the window configuration and the resolved video/audio
//! availability. Nothing in the grid is read from a host clock, a path, a token or the
//! environment, because none of those can reach a view or the model.

use crate::GuiError;
use crate::model::{GuiModel, Layout, Mosaic, Pane, PaneId, PanelKind, Rect};
use crate::presentation::{Availability, UnavailableReason, panel_availability};
use crate::render::{Frame, Grid, RenderOptions, check_frame_bounds};
use crate::view::{SessionView, TimelineView};

/// The `available` label of a status-line availability field.
const AVAILABLE: &str = "available";
/// The `unavailable` label of a status-line availability field.
const UNAVAILABLE: &str = "unavailable";
/// The placeholder a null lineage field renders as.
const NULL: &str = "null";
/// The placeholder an absent value renders as.
const NONE: &str = "none";

/// Everything one text frame is rendered from, besides the renderer options.
///
/// The command layer assembles this from the model, the controller lease, the window
/// configuration and the availability the presentation pipelines resolved; the renderer
/// owns no field of it, reads nothing else and stores no time value, so two renders of the
/// same source are byte-identical.
#[derive(Debug)]
pub struct FrameSource<'a> {
    /// The window model: panes, focus, layout, panels, counters and the event log.
    pub model: &'a GuiModel,
    /// The lease controller name, from `control::Controller::as_str` (`human` or `agent`).
    pub controller: &'a str,
    /// The lease generation the `status` panel reports.
    pub lease_generation: u64,
    /// The lease sequence the `status` panel reports.
    pub lease_sequence: u64,
    /// The configured presentation cadence, reported verbatim in the title line.
    pub fps: u32,
    /// The CLI-provided TAS timeline, when the window was opened with `--movie`.
    pub timeline: Option<&'a TimelineView>,
    /// The window-level video availability resolved by the presentation pipeline.
    pub rendering: Availability,
    /// The window-level audio availability resolved by the presentation pipeline.
    pub audio: Availability,
    /// Frame width in cells.
    pub width: u32,
    /// Frame height in lines.
    pub height: u32,
}

/// Renders the plain grid of one frame.
///
/// This is the value the HTML renderer embeds: the document is built from the very grids
/// the terminal receives, never from a second drawing pass.
///
/// # Errors
///
/// Returns [`GuiError::LimitExceeded`] when `width` or `height` is outside the documented
/// frame bounds, and [`GuiError::BadValue`] when the model could not produce a grid of
/// exactly that shape (which a validated model never does).
pub fn frame(source: &FrameSource<'_>) -> Result<Frame, GuiError> {
    build(source).map(|(frame, _)| frame)
}

/// Renders one frame as text: the plain grid, or the grid with SGR emphasis.
///
/// The result is exactly `height` lines of exactly `width` printable ASCII columns each,
/// every line `\n`-terminated and with no trailing blank line. With
/// [`RenderOptions::ansi`] only `ESC[1m` and `ESC[0m` are added, so stripping them yields
/// the plain rendering byte for byte.
///
/// # Errors
///
/// Returns the same typed failures as [`frame`].
pub fn render(source: &FrameSource<'_>, options: RenderOptions) -> Result<String, GuiError> {
    let (frame, emphasized) = build(source)?;
    if !options.ansi {
        return Ok(frame.to_text());
    }
    Ok(ansi_text(&frame, &emphasized))
}

/// Builds the plain grid of one frame and the rows ANSI mode emphasizes.
fn build(source: &FrameSource<'_>) -> Result<(Frame, Vec<bool>), GuiError> {
    check_frame_bounds(source.width, source.height)?;
    let mut grid = Grid::new(source.width, source.height);

    let title = Rect::new(0, 0, source.width, 1);
    grid.put(title, 0, &title_line(source));
    grid.emphasize(title, 0);

    let separator = Rect::new(0, 1, source.width, 1);
    grid.put(
        separator,
        0,
        &"-".repeat(usize::try_from(source.width).unwrap_or(0)),
    );

    let pane_area = Rect::new(0, 2, source.width, source.height.saturating_sub(3));
    match source.model.layout() {
        Layout::Single => {
            let focused = source
                .model
                .focused()
                .and_then(|pane| source.model.pane(pane).map(|attached| (pane, attached)));
            render_region(&mut grid, pane_area, focused, source);
        }
        Layout::Mosaic => {
            let mosaic = Mosaic::quadrants(pane_area)?;
            for (index, region) in mosaic.regions().into_iter().enumerate() {
                let attached = source.model.panes().nth(index);
                render_region(&mut grid, region, attached, source);
            }
        }
    }

    let status = Rect::new(0, source.height.saturating_sub(1), source.width, 1);
    grid.put(status, 0, &status_line(source));

    let (rows, emphasized) = grid.into_rows();
    Ok((Frame::new(source.width, source.height, rows)?, emphasized))
}

/// Renders one pane region: its header line, then one line per visible panel.
///
/// An empty region stays blank, which is what the mosaic layout requires for the quadrants
/// no attached pane occupies. Panels beyond the region's last row are dropped, never
/// wrapped: a narrow frame shows less, not a reflowed layout.
fn render_region(
    grid: &mut Grid,
    region: Rect,
    attached: Option<(PaneId, &Pane)>,
    source: &FrameSource<'_>,
) {
    let Some((pane_id, pane)) = attached else {
        return;
    };
    grid.put(
        region,
        0,
        &pane_header(pane_id, pane, source.model.focused()),
    );
    grid.emphasize(region, 0);
    for (index, kind) in source.model.visible_panels().enumerate() {
        let offset = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
        if offset >= region.height {
            break;
        }
        grid.put(region, offset, &panel_text(kind, pane, source));
    }
}

/// Returns the documented title line of the frame.
fn title_line(source: &FrameSource<'_>) -> String {
    format!(
        "chronofork gui layout={} controller={} frame={} fps={}",
        source.model.layout().as_str(),
        source.controller,
        source.model.frame_index(),
        source.fps
    )
}

/// Returns the documented final status line of the frame.
///
/// The tick and lifecycle state are the focused pane's; with no attached pane no view
/// exists to report, so the line carries the honest `tick=0 state=none` placeholders
/// instead of an invented session state.
fn status_line(source: &FrameSource<'_>) -> String {
    let focused = source.model.focused();
    let (tick, state) = match focused.and_then(|pane| source.model.pane(pane)) {
        Some(pane) => (
            pane.view().machine().tick().to_string(),
            pane.view().state().as_str().to_owned(),
        ),
        None => ("0".to_owned(), NONE.to_owned()),
    };
    format!(
        "panes={} focused={} tick={tick} state={state} rendering={} audio={}",
        source.model.attached(),
        focused.map_or_else(|| NONE.to_owned(), |pane| pane.raw().to_string()),
        availability_label(&source.rendering),
        availability_label(&source.audio)
    )
}

/// Returns the documented header line of one pane region.
fn pane_header(pane_id: PaneId, pane: &Pane, focused: Option<PaneId>) -> String {
    let view = pane.view();
    let mut header = format!(
        "pane {} session {} tick {} state {}",
        pane_id.raw(),
        pane.session(),
        view.machine().tick(),
        view.state().as_str()
    );
    if focused == Some(pane_id) {
        header.push_str(" focused");
    }
    header
}

/// Returns the one line a panel renders, available or not.
fn panel_text(kind: PanelKind, pane: &Pane, source: &FrameSource<'_>) -> String {
    let availability = panel_availability(kind, true, source.timeline.is_some());
    match availability.reason() {
        Some(reason) => unavailable_panel(kind, reason),
        None => panel_content(kind, pane, source),
    }
}

/// Returns the documented line of a visible panel that has no canonical data.
fn unavailable_panel(kind: PanelKind, reason: UnavailableReason) -> String {
    format!("{} unavailable reason={}", kind.name(), reason.as_str())
}

/// Returns the documented line of a visible panel that has canonical data.
///
/// Every value comes from a decoded view, the model's own event log or the CLI-provided
/// timeline; no panel invents a field the operation did not return.
fn panel_content(kind: PanelKind, pane: &Pane, source: &FrameSource<'_>) -> String {
    let view = pane.view();
    let machine = view.machine();
    match kind {
        PanelKind::Status => format!(
            "status tick={} state={} halted={} controller={} generation={} sequence={}",
            machine.tick(),
            view.state().as_str(),
            boolean(machine.halted()),
            source.controller,
            source.lease_generation,
            source.lease_sequence
        ),
        PanelKind::Lineage => {
            let lineage = view.lineage();
            format!(
                "lineage parent={} origin={} depth={}",
                optional_unsigned(lineage.parent_session()),
                optional_unsigned(lineage.origin_snapshot()),
                lineage.depth()
            )
        }
        PanelKind::Capabilities => capabilities_line(pane),
        PanelKind::Registers => registers_line(view),
        PanelKind::Memory => memory_line(pane),
        PanelKind::Inspector => inspectors_line(view),
        PanelKind::Timeline => timeline_line(source.timeline),
        PanelKind::EventLog => events_line(source),
    }
}

/// Returns `capabilities <backend-id> <flag>=<bool> ...` in descriptor order.
fn capabilities_line(pane: &Pane) -> String {
    let backend = pane.view().backend();
    let mut line = format!("capabilities {}", backend.id());
    for (name, enabled) in backend.capabilities().flags() {
        line.push(' ');
        line.push_str(name);
        line.push('=');
        line.push_str(boolean(enabled));
    }
    line
}

/// Returns `registers <name>=<value> ...` in the inspection's sorted order.
fn registers_line(view: &SessionView) -> String {
    let mut line = String::from("registers");
    for (name, value) in view.machine().registers() {
        line.push(' ');
        line.push_str(name);
        line.push('=');
        line.push_str(&value.to_string());
    }
    line
}

/// Returns `memory base=<hex> length=<n> <hex bytes>`.
///
/// The bytes are the bounded window captured at attach time. A machine that exposes no
/// memory window reports `length=0` and no bytes, which is the honest empty window rather
/// than an unavailable panel: the session answered.
fn memory_line(pane: &Pane) -> String {
    match pane.memory() {
        Some(memory) => format!(
            "memory base={:x} length={} {}",
            memory.address(),
            memory.length(),
            hex(memory.bytes())
        ),
        None => format!(
            "memory base={:x} length={}",
            pane.view().machine().memory_base(),
            pane.view().machine().memory_length()
        ),
    }
}

/// Returns the `inspectors` line: `cpu` from the inspection, the device sections honestly
/// unavailable because the M6 surface does not carry them.
fn inspectors_line(view: &SessionView) -> String {
    let registers_available = view
        .backend()
        .capabilities()
        .flags()
        .into_iter()
        .any(|(name, enabled)| name == "register_inspection" && enabled);
    let cpu = if registers_available {
        AVAILABLE.to_owned()
    } else {
        unavailable_label(UnavailableReason::CapabilityDisabled)
    };
    let transport = unavailable_label(UnavailableReason::NotExposedByTransport);
    format!(
        "inspectors cpu={cpu} vdp={transport} interrupts={transport} bus={transport} disassembly={transport}"
    )
}

/// Returns the `timeline` line, or the documented unavailable form without a movie.
fn timeline_line(timeline: Option<&TimelineView>) -> String {
    match timeline {
        Some(timeline) => format!(
            "timeline version={} steps={} end_tick={} anchors={}",
            timeline.format_version(),
            timeline.steps(),
            timeline.end_tick(),
            timeline.anchors().len()
        ),
        None => unavailable_panel(PanelKind::Timeline, UnavailableReason::NoMovie),
    }
}

/// Returns `events <n> last=<event> sequence=<n>` from the model's bounded event log.
fn events_line(source: &FrameSource<'_>) -> String {
    let events = source.model.events();
    match events.last() {
        Some(event) => format!(
            "events {} last={} sequence={}",
            events.len(),
            event.kind().name(),
            event.sequence()
        ),
        None => format!("events 0 last={NONE} sequence={NONE}"),
    }
}

/// Serializes a validated grid with SGR emphasis on the marked rows.
fn ansi_text(frame: &Frame, emphasized: &[bool]) -> String {
    let capacity = frame
        .lines()
        .iter()
        .map(|line| line.len().saturating_add(16))
        .sum();
    let mut text = String::with_capacity(capacity);
    for (index, line) in frame.lines().iter().enumerate() {
        if emphasized.get(index).copied().unwrap_or(false) {
            text.push_str(&ansi_line(line));
        } else {
            text.push_str(line);
        }
        text.push('\n');
    }
    text
}

/// Wraps the non-padding part of one grid line in the two documented SGR sequences.
///
/// The trailing padding stays outside the emphasis so the line keeps its exact width, and
/// an all-space line is emitted verbatim: removing `ESC[1m`/`ESC[0m` therefore yields the
/// plain line byte for byte.
fn ansi_line(line: &str) -> String {
    let content = line.trim_end_matches(' ');
    if content.is_empty() {
        return line.to_owned();
    }
    let padding = &line[content.len()..];
    let mut output = String::with_capacity(line.len().saturating_add(9));
    output.push_str("\u{1b}[1m");
    output.push_str(content);
    output.push_str("\u{1b}[0m");
    output.push_str(padding);
    output
}

/// Returns the status-line label of an availability value.
fn availability_label(availability: &Availability) -> &'static str {
    if availability.is_available() {
        AVAILABLE
    } else {
        UNAVAILABLE
    }
}

/// Returns the `unavailable:<reason>` form the `inspectors` section list uses.
fn unavailable_label(reason: UnavailableReason) -> String {
    format!("unavailable:{}", reason.as_str())
}

/// Returns the canonical text of a boolean field.
fn boolean(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Returns the canonical text of an optional unsigned field.
fn optional_unsigned(value: Option<u64>) -> String {
    value.map_or_else(|| NULL.to_owned(), |value| value.to_string())
}

/// Returns the lowercase hex encoding of a byte window.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            text.push(char::from_digit(u32::from(nibble), 16).unwrap_or('?'));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FrameSource, ansi_line, events_line, inspectors_line, render};
    use crate::model::{GuiModel, WindowId};
    use crate::presentation::{Presentation, UnavailableReason};
    use crate::render::RenderOptions;
    use crate::view::SessionView;
    use crate::{DEFAULT_FPS, MIN_HEIGHT, MIN_WIDTH};

    const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";

    fn window() -> GuiModel {
        GuiModel::new(WindowId::from_raw(1))
    }

    fn source<'a>(model: &'a GuiModel) -> FrameSource<'a> {
        FrameSource {
            model,
            controller: "human",
            lease_generation: 0,
            lease_sequence: 0,
            fps: DEFAULT_FPS,
            timeline: None,
            rendering: Presentation::unavailable(UnavailableReason::CapabilityDisabled),
            audio: Presentation::unavailable(UnavailableReason::CapabilityDisabled),
            width: MIN_WIDTH,
            height: MIN_HEIGHT,
        }
    }

    /// Removes every SGR sequence the ANSI renderer may emit.
    fn strip_sgr(text: &str) -> String {
        text.replace("\u{1b}[1m", "").replace("\u{1b}[0m", "")
    }

    /// Returns every escape sequence found in `text`, so a test can assert the closed set.
    fn escapes(text: &str) -> Vec<&str> {
        let mut found = Vec::new();
        let bytes = text.as_bytes();
        let mut index = 0;
        while let Some(offset) = bytes[index..].iter().position(|byte| *byte == 0x1b) {
            let start = index + offset;
            let end = (start + 4).min(bytes.len());
            found.push(&text[start..end]);
            index = end;
        }
        found
    }

    #[test]
    fn ansi_output_uses_only_the_two_documented_sgr_sequences() {
        let model = window();
        let text = render(&source(&model), RenderOptions::with_ansi(true)).expect("ansi frame");
        let sequences = escapes(&text);
        assert!(!sequences.is_empty(), "the title is emphasized");
        for sequence in sequences {
            assert!(
                sequence == "\u{1b}[1m" || sequence == "\u{1b}[0m",
                "an undocumented escape sequence reached the frame: {sequence:?}"
            );
        }
        assert!(text.starts_with("\u{1b}[1mchronofork gui layout=single"));
        assert!(
            !text.contains("\u{1b}[2J")
                && !text.contains("\u{1b}[H")
                && !text.contains("\u{1b}[6n"),
            "no cursor movement, clearing or terminal query is emitted"
        );
    }

    #[test]
    fn ansi_emphasis_strips_back_to_the_plain_frame() {
        let model = window();
        let source = source(&model);
        let plain = render(&source, RenderOptions::plain()).expect("plain frame");
        let ansi = render(&source, RenderOptions::with_ansi(true)).expect("ansi frame");
        assert_ne!(ansi, plain, "ANSI mode emphasizes the title");
        assert_eq!(
            strip_sgr(&ansi),
            plain,
            "stripping SGR yields the plain frame"
        );
        assert_eq!(
            ansi.lines().count(),
            plain.lines().count(),
            "emphasis never changes the shape"
        );
        for line in ansi.lines() {
            assert_eq!(
                strip_sgr(line).len(),
                usize::try_from(MIN_WIDTH).expect("width"),
                "a stripped line is exactly the configured width"
            );
        }
    }

    #[test]
    fn ansi_render_is_byte_identical_across_runs() {
        let model = window();
        let source = source(&model);
        let first = render(&source, RenderOptions::with_ansi(true)).expect("ansi frame");
        let second = render(&source, RenderOptions::with_ansi(true)).expect("ansi frame");
        assert_eq!(first, second);
    }

    /// Decodes one `session.inspect` result so a panel line can be exercised directly: the
    /// frame is narrower than some documented panel lines, and a unit test is where the
    /// unclipped line is asserted.
    fn session_view(register_inspection: bool) -> SessionView {
        use chronofork_service::ServiceLimits;
        use chronofork_service::json::{JsonValue, to_canonical_bytes};

        let mut flags = BTreeMap::new();
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
                "register_inspection" => register_inspection,
                "rendering" | "audio" => false,
                _ => true,
            };
            flags.insert(name.to_owned(), JsonValue::from_bool(enabled));
        }
        let result = JsonValue::object([
            ("session", JsonValue::int(1)),
            ("state", JsonValue::string("ready")),
            (
                "lineage",
                JsonValue::object([
                    ("parent_session", JsonValue::Null),
                    ("origin_snapshot", JsonValue::Null),
                    ("depth", JsonValue::int(0)),
                ]),
            ),
            ("revision", JsonValue::int(0)),
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
            (
                "machine",
                JsonValue::object([
                    ("tick", JsonValue::int(0)),
                    ("state_hash", JsonValue::string(HASH)),
                    ("halted", JsonValue::from_bool(false)),
                    ("registers", JsonValue::object([("d0", JsonValue::int(0))])),
                    ("memory_base", JsonValue::int(0)),
                    ("memory_length", JsonValue::int(0)),
                ]),
            ),
        ]);
        let envelope = JsonValue::object([
            ("op", JsonValue::string("session.inspect")),
            ("result", result),
        ]);
        let bytes =
            to_canonical_bytes(&envelope, &ServiceLimits::default()).expect("canonical bytes");
        crate::view::decode_session_view(&bytes).expect("session view")
    }

    #[test]
    fn render_panel_line_reports_every_inspector_section_honestly() {
        let line = inspectors_line(&session_view(true));
        assert_eq!(
            line,
            "inspectors cpu=available vdp=unavailable:not_exposed_by_transport interrupts=unavailable:not_exposed_by_transport bus=unavailable:not_exposed_by_transport disassembly=unavailable:not_exposed_by_transport"
        );
        assert_eq!(line.matches("not_exposed_by_transport").count(), 4);

        // A descriptor without register inspection makes the cpu section unavailable for
        // its own reason instead of a placeholder section header.
        let line = inspectors_line(&session_view(false));
        assert!(
            line.starts_with("inspectors cpu=unavailable:capability_disabled "),
            "{line}"
        );
        assert_eq!(line.matches("not_exposed_by_transport").count(), 4);
    }

    /// The `events` formatter's empty-log fallback cannot be reached through the public
    /// renderer — a frame renders panels only inside a pane region, and a pane exists only
    /// after an attach, which records an event — so the formatter is proven directly.
    #[test]
    fn events_line_falls_back_to_no_event_when_the_log_is_empty() {
        let model = GuiModel::new(WindowId::from_raw(1));
        assert!(model.events().is_empty(), "a fresh window holds no event");
        assert!(model.events().last().is_none());
        let source = FrameSource {
            model: &model,
            controller: "human",
            lease_generation: 0,
            lease_sequence: 0,
            fps: 60,
            timeline: None,
            rendering: Presentation::unavailable(UnavailableReason::NoOutputDevice),
            audio: Presentation::unavailable(UnavailableReason::NoOutputDevice),
            width: MIN_WIDTH,
            height: MIN_HEIGHT,
        };
        assert_eq!(events_line(&source), "events 0 last=none sequence=none");
    }

    #[test]
    fn ansi_emphasis_wraps_a_pane_header_and_keeps_the_padding() {
        let line = "pane 1 session 2 tick 3 state ready focused    ";
        let emphasized = ansi_line(line);
        assert_eq!(
            emphasized,
            "\u{1b}[1mpane 1 session 2 tick 3 state ready focused\u{1b}[0m    "
        );
        assert_eq!(strip_sgr(&emphasized), line);
        assert_eq!(
            ansi_line(&" ".repeat(8)),
            "        ",
            "a blank line is verbatim"
        );
    }
}
