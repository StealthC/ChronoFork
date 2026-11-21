// Anti-slop policy §10 exception: owner render-engineer; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It mirrors
// the existing headers in crates/chronofork-cli/tests/m6.rs and
// crates/chronofork-gui/tests/model.rs; no production code is affected.
#![allow(clippy::expect_used)]

//! Renderer acceptance tests for the M7 GUI.
//!
//! These tests exercise the two documented frontends and the panel availability rules
//! against a **scripted channel**, so every frame is a pure function of decoded views:
//!
//! - M7-A8 — panel availability: the matrix over the eight documented panels, `no_session`
//!   and `no_movie`, and panel lines that carry only decoded view values.
//! - M7-A9 — deterministic text renderer: exactly `height` lines of exactly `width`
//!   printable ASCII columns, space padded, `\n`-terminated, no trailing blank line, the
//!   documented rows, clipping (never wrapping), `?` for a non-printable or non-ASCII
//!   value, and the documented minimum and maximum frame sizes.
//! - M7-A10 — ANSI mode: emphasis only, and stripping SGR yields the plain frame.
//! - M7-A11 — the self-contained HTML document: doctype, charset, inline style, `<pre>`
//!   frames, total escaping of an adversarial descriptor, `<pre>` content equal to the
//!   plain grid after entity decoding, the `MAX_HTML_BYTES` bound and determinism.
//!
//! The `chronofork gui run` command engine and the process-level rows belong to the command
//! layer task; these tests render directly, the way that engine will.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use chronofork_gui::presentation::{Presentation, UnavailableReason, panel_availability};
use chronofork_gui::render::text::{self, FrameSource};
use chronofork_gui::render::{Frame, RenderOptions};
use chronofork_gui::{
    ChannelFault, GuiErrorCode, GuiModel, Layout, OperationChannel, PanelKind, TimelineAnchor,
    TimelineView, WindowId,
};
use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, to_canonical_bytes};

const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";

/// A channel that answers from a fixed script and never reaches the network.
struct ScriptedChannel {
    responses: Mutex<VecDeque<Vec<u8>>>,
}

impl ScriptedChannel {
    fn new(script: Vec<Vec<u8>>) -> Self {
        Self {
            responses: Mutex::new(script.into()),
        }
    }
}

impl OperationChannel for ScriptedChannel {
    fn call(&self, _request: &[u8]) -> Result<Vec<u8>, ChannelFault> {
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| ChannelFault::new("no scripted response left"))
    }
}

fn canonical(value: &JsonValue) -> Vec<u8> {
    to_canonical_bytes(value, &ServiceLimits::default()).expect("canonical bytes")
}

fn envelope(op: &str, result: JsonValue) -> Vec<u8> {
    canonical(&JsonValue::object([
        ("op", JsonValue::string(op)),
        ("result", result),
    ]))
}

fn descriptor(backend_id: &str) -> JsonValue {
    descriptor_with(backend_id, "Synthetic", "0.1.0")
}

/// A descriptor with explicit metadata, so a test can supply an adversarial identifier,
/// display name and version at once.
fn descriptor_with(backend_id: &str, display_name: &str, version: &str) -> JsonValue {
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
        let enabled = name != "rendering" && name != "audio";
        flags.insert(name.to_owned(), JsonValue::from_bool(enabled));
    }
    JsonValue::object([
        ("id", JsonValue::string(backend_id)),
        ("display_name", JsonValue::string(display_name)),
        ("version", JsonValue::string(version)),
        ("test_backend", JsonValue::from_bool(true)),
        ("capabilities", JsonValue::Object(flags)),
    ])
}

fn lineage() -> JsonValue {
    JsonValue::object([
        ("parent_session", JsonValue::Null),
        ("origin_snapshot", JsonValue::Null),
        ("depth", JsonValue::int(0)),
    ])
}

/// A `session.inspect` result with an explicit backend id and register map.
fn inspect_result(
    session: u64,
    tick: u64,
    memory_base: u64,
    memory_length: u64,
    backend_id: &str,
    registers: &[(&str, u64)],
) -> Vec<u8> {
    inspect_result_with_descriptor(
        session,
        tick,
        memory_base,
        memory_length,
        descriptor(backend_id),
        registers,
    )
}

/// The same result with an explicit descriptor, so a test can supply adversarial metadata.
fn inspect_result_with_descriptor(
    session: u64,
    tick: u64,
    memory_base: u64,
    memory_length: u64,
    backend: JsonValue,
    registers: &[(&str, u64)],
) -> Vec<u8> {
    let registers = JsonValue::Object(
        registers
            .iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    JsonValue::int(i64::try_from(*value).expect("register value")),
                )
            })
            .collect(),
    );
    envelope(
        "session.inspect",
        JsonValue::object([
            (
                "session",
                JsonValue::int(i64::try_from(session).expect("session")),
            ),
            ("state", JsonValue::string("ready")),
            ("lineage", lineage()),
            ("revision", JsonValue::int(0)),
            ("backend", backend),
            (
                "machine",
                JsonValue::object([
                    ("tick", JsonValue::int(i64::try_from(tick).expect("tick"))),
                    ("state_hash", JsonValue::string(HASH)),
                    ("halted", JsonValue::from_bool(false)),
                    ("registers", registers),
                    (
                        "memory_base",
                        JsonValue::int(i64::try_from(memory_base).expect("base")),
                    ),
                    (
                        "memory_length",
                        JsonValue::int(i64::try_from(memory_length).expect("length")),
                    ),
                ]),
            ),
        ]),
    )
}

fn memory_result(session: u64, address: u64, bytes: &[u8]) -> Vec<u8> {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    envelope(
        "session.read_memory",
        JsonValue::object([
            (
                "session",
                JsonValue::int(i64::try_from(session).expect("session")),
            ),
            (
                "address",
                JsonValue::int(i64::try_from(address).expect("address")),
            ),
            (
                "length",
                JsonValue::int(i64::try_from(bytes.len()).expect("length")),
            ),
            ("bytes", JsonValue::string(hex)),
        ]),
    )
}

fn window() -> WindowId {
    WindowId::from_raw(1)
}

/// The plain default register map, so a frame is stable across tests.
fn default_registers() -> [(&'static str, u64); 2] {
    [("d0", 0), ("pc", 1024)]
}

/// Attaches one session with no memory window through a scripted channel.
fn attach(model: &mut GuiModel, session: u64) {
    let channel = ScriptedChannel::new(vec![inspect_result(
        session,
        0,
        0,
        0,
        "synthetic",
        &default_registers(),
    )]);
    model.attach(&channel, session).expect("attach");
}

fn source<'a>(model: &'a GuiModel, width: u32, height: u32) -> FrameSource<'a> {
    source_with(model, None, width, height)
}

fn source_with<'a>(
    model: &'a GuiModel,
    timeline: Option<&'a TimelineView>,
    width: u32,
    height: u32,
) -> FrameSource<'a> {
    FrameSource {
        model,
        controller: "human",
        lease_generation: 1,
        lease_sequence: 0,
        fps: 60,
        timeline,
        rendering: Presentation::unavailable(UnavailableReason::CapabilityDisabled),
        audio: Presentation::unavailable(UnavailableReason::CapabilityDisabled),
        width,
        height,
    }
}

fn plain(source: &FrameSource<'_>) -> String {
    text::render(source, RenderOptions::plain()).expect("plain frame")
}

fn ansi(source: &FrameSource<'_>) -> String {
    text::render(source, RenderOptions::with_ansi(true)).expect("ansi frame")
}

fn strip_sgr(text: &str) -> String {
    text.replace("\u{1b}[1m", "").replace("\u{1b}[0m", "")
}

/// Renders every line of a frame and asserts the documented grid invariants.
fn assert_grid_invariants(rendered: &str, width: u32, height: u32) {
    let columns = usize::try_from(width).expect("width");
    let lines = usize::try_from(height).expect("height");
    assert!(rendered.ends_with('\n'), "the grid is newline terminated");
    assert!(
        !rendered.ends_with("\n\n"),
        "a grid has no trailing blank line"
    );
    let rows: Vec<&str> = rendered.split('\n').collect();
    assert_eq!(
        rows.len(),
        lines + 1,
        "exactly height lines plus the terminator"
    );
    assert_eq!(
        rows[lines], "",
        "the last split part is the terminator only"
    );
    for (index, row) in rows[..lines].iter().enumerate() {
        assert_eq!(row.len(), columns, "row {index} is exactly width columns");
        assert!(
            row.is_ascii() && row.bytes().all(|byte| (0x20..=0x7e).contains(&byte)),
            "row {index} is printable ASCII only"
        );
    }
}

fn timeline() -> TimelineView {
    TimelineView::new(
        1,
        3,
        3,
        vec![TimelineAnchor::new(1, 1), TimelineAnchor::new(2, 2)],
    )
    .expect("timeline")
}

#[test]
fn text_frame_is_exactly_the_configured_grid() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 1);

    for (width, height) in [(40_u32, 10_u32), (60, 20), (200, 100)] {
        let rendered = plain(&source(&model, width, height));
        assert_grid_invariants(&rendered, width, height);
        let frame = text::frame(&source(&model, width, height)).expect("plain frame value");
        assert_eq!(frame.width(), width);
        assert_eq!(frame.height(), height);
        assert_eq!(frame.to_text(), rendered, "to_text is the plain grid");
    }
}

#[test]
fn text_frame_is_byte_identical_across_runs() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 4);
    model.set_layout(Layout::Mosaic);
    model.advance_frames(2);

    let first = plain(&source(&model, 80, 24));
    let second = plain(&source(&model, 80, 24));
    assert_eq!(first, second, "the same model renders the same bytes");
}

#[test]
fn text_frame_rows_carry_the_title_separator_header_and_status() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 1);
    let rendered = plain(&source(&model, 80, 24));
    let rows: Vec<&str> = rendered.lines().collect();

    assert!(
        rows[0].starts_with("chronofork gui layout=single controller=human frame=0 fps=60"),
        "title row: {:?}",
        rows[0]
    );
    assert_eq!(rows[1], "-".repeat(80), "separator row is all dashes");
    assert!(
        rows[2].starts_with("pane 1 session 1 tick 0 state ready focused"),
        "pane header row: {:?}",
        rows[2]
    );
    let status = rows[rows.len() - 1];
    assert!(
        status.starts_with(
            "panes=1 focused=1 tick=0 state=ready rendering=unavailable audio=unavailable"
        ),
        "status row: {status:?}"
    );
}

#[test]
fn text_frame_clips_long_values_and_marks_unprintable_values() {
    let hostile_id = "backend-with-an-identifier-long-enough-to-be-clipped-by-any-documented-width";
    let channel = ScriptedChannel::new(vec![inspect_result(
        1,
        0,
        0,
        0,
        hostile_id,
        &[("\u{1b}[31m", 7), ("d0", 0), ("pc", 1024)],
    )]);
    let mut model = GuiModel::new(window());
    model.attach(&channel, 1).expect("attach");

    let rendered = plain(&source(&model, 40, 12));
    assert_grid_invariants(&rendered, 40, 12);
    assert!(
        rendered.contains("registers ?[31m=7 d0=0 pc=1024"),
        "a non-printable register name renders as `?`:\n{rendered}"
    );
    assert!(
        !rendered.contains(hostile_id),
        "a long value is clipped, never wrapped or emitted in full"
    );
    assert!(
        rendered.contains("capabilities backend-with-an-identifier-"),
        "the clipped capabilities line keeps its documented prefix"
    );
    assert!(
        !rendered.contains('\u{1b}'),
        "no control byte reaches a grid"
    );
}

#[test]
fn text_frame_with_no_pane_is_blank_between_the_separator_and_the_status() {
    let model = GuiModel::new(window());
    let rendered = plain(&source(&model, 40, 12));
    assert_grid_invariants(&rendered, 40, 12);
    let rows: Vec<&str> = rendered.lines().collect();
    assert_eq!(rows[0].split(' ').next(), Some("chronofork"));
    assert!(
        rows.last()
            .expect("status")
            .starts_with("panes=0 focused=none tick=0 state=none")
    );
    for row in &rows[2..rows.len() - 1] {
        assert_eq!(*row, " ".repeat(40), "an empty pane area stays blank");
    }
}

#[test]
fn text_mosaic_places_panes_row_major_and_leaves_empty_regions_blank() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 1);
    attach(&mut model, 2);
    model.set_layout(Layout::Mosaic);

    let rendered = plain(&source(&model, 40, 10));
    assert_grid_invariants(&rendered, 40, 10);
    let rows: Vec<&str> = rendered.lines().collect();
    assert!(rows[0].contains("layout=mosaic"));
    assert_eq!(&rows[2][0..8], "pane 1 s", "top-left region holds pane 1");
    assert_eq!(
        &rows[2][20..28],
        "pane 2 s",
        "top-right region holds pane 2"
    );
    for row in rows.iter().take(9).skip(5) {
        assert_eq!(*row, " ".repeat(40), "the two unused quadrants stay blank");
    }
    // With four panes the quadrants are filled row-major: 1, 2 on the top row and 3, 4 on
    // the bottom row.
    attach(&mut model, 3);
    attach(&mut model, 4);
    let rendered = plain(&source(&model, 40, 10));
    assert_grid_invariants(&rendered, 40, 10);
    let rows: Vec<&str> = rendered.lines().collect();
    assert_eq!(&rows[2][0..8], "pane 1 s", "top-left region holds pane 1");
    assert_eq!(
        rows[2][20..28],
        *"pane 2 s",
        "top-right region holds pane 2"
    );
    assert_eq!(
        &rows[5][0..8],
        "pane 3 s",
        "bottom-left region holds pane 3"
    );
    assert_eq!(
        rows[5][20..28],
        *"pane 4 s",
        "bottom-right region holds pane 4"
    );

    // The single layout renders only the focused pane.
    model.set_layout(Layout::Single);
    let rendered = plain(&source(&model, 40, 10));
    assert!(
        !rendered.contains("pane 1 "),
        "single renders the focused pane"
    );
    assert!(
        rendered.contains("pane 4 session 4"),
        "pane 4 is the focused pane"
    );
}

#[test]
fn text_frame_obeys_the_documented_minimum_and_maximum_bounds() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 1);
    for (width, height) in [(40_u32, 10_u32), (200, 100)] {
        let rendered = plain(&source(&model, width, height));
        assert_grid_invariants(&rendered, width, height);
    }

    let error = text::render(&source(&model, 40 - 1, 10), RenderOptions::plain())
        .expect_err("width below the minimum");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    let error = text::render(&source(&model, 40, 100 + 1), RenderOptions::plain())
        .expect_err("height above the maximum");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
}

#[test]
fn text_ansi_is_emphasis_only_and_strips_back_to_plain() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 3);
    let source = source(&model, 80, 20);
    let plain_frame = plain(&source);
    let ansi_frame = ansi(&source);
    assert_eq!(strip_sgr(&ansi_frame), plain_frame);
    assert_grid_invariants(&strip_sgr(&ansi_frame), 80, 20);

    let mut escapes = Vec::new();
    let bytes = ansi_frame.as_bytes();
    let mut index = 0;
    while let Some(offset) = bytes[index..].iter().position(|byte| *byte == 0x1b) {
        let start = index + offset;
        let end = (start + 4).min(bytes.len());
        escapes.push(&ansi_frame[start..end]);
        index = end;
    }
    assert!(!escapes.is_empty());
    for sequence in escapes {
        assert!(
            sequence == "\u{1b}[1m" || sequence == "\u{1b}[0m",
            "only the two documented SGR sequences are emitted, found {sequence:?}"
        );
    }
    let rows: Vec<&str> = ansi_frame.lines().collect();
    assert!(
        rows[0].starts_with("\u{1b}[1mchronofork gui"),
        "the title is emphasized"
    );
    assert!(
        rows[2].starts_with("\u{1b}[1mpane 1 session 3"),
        "the pane header is emphasized"
    );
    assert!(
        !rows[rows.len() - 1].contains('\u{1b}'),
        "the status row is not emphasized"
    );
}

#[test]
fn panel_availability_matrix() {
    let no_session = |kind| panel_availability(kind, false, false);
    let with_session = |kind| panel_availability(kind, true, false);
    let with_movie = |kind| panel_availability(kind, true, true);

    for kind in [
        PanelKind::Status,
        PanelKind::Lineage,
        PanelKind::Capabilities,
        PanelKind::Registers,
        PanelKind::Memory,
        PanelKind::Inspector,
    ] {
        assert_eq!(
            no_session(kind).reason(),
            Some(UnavailableReason::NoSession),
            "{} without a pane",
            kind.name()
        );
        assert!(with_session(kind).is_available());
        assert!(with_movie(kind).is_available());
    }
    assert_eq!(
        no_session(PanelKind::Timeline).reason(),
        Some(UnavailableReason::NoMovie)
    );
    assert_eq!(
        with_session(PanelKind::Timeline).reason(),
        Some(UnavailableReason::NoMovie),
        "the timeline is backed by the movie, not by a session"
    );
    assert!(with_movie(PanelKind::Timeline).is_available());
    assert!(no_session(PanelKind::EventLog).is_available());

    // Panel lines live inside a pane region, so a window with no attached pane renders no
    // panel line at all: the `no_session` reason above is what the `panel` command reports
    // and what a visible panel would render if a pane existed.
    let model = GuiModel::new(window());
    let rendered = plain(&source(&model, 80, 24));
    assert!(!rendered.contains("unavailable reason="));
    assert!(!rendered.contains("status tick="));
}

#[test]
fn panel_lines_render_the_decoded_view_values() {
    let mut model = GuiModel::new(window());
    let channel = ScriptedChannel::new(vec![
        inspect_result(1, 4, 4096, 4, "synthetic", &default_registers()),
        memory_result(1, 4096, &[0xaa, 0xbb, 0xcc, 0xdd]),
    ]);
    model.attach(&channel, 1).expect("attach with memory");
    for kind in [PanelKind::Memory, PanelKind::Inspector, PanelKind::Timeline] {
        model.set_panel(kind, true);
    }
    let timeline = timeline();
    let rendered = plain(&source_with(&model, Some(&timeline), 200, 40));

    for expected in [
        "status tick=4 state=ready halted=false controller=human generation=1 sequence=0",
        "lineage parent=null origin=null depth=0",
        "capabilities synthetic execution=true save_states=true memory_read=true",
        "registers d0=0 pc=1024",
        "memory base=1000 length=4 aabbccdd",
        "inspectors cpu=available vdp=unavailable:not_exposed_by_transport",
        "interrupts=unavailable:not_exposed_by_transport",
        "timeline version=1 steps=3 end_tick=3 anchors=2",
        "events 4 last=panel sequence=3",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?}:\n{rendered}"
        );
    }

    // Without a movie the timeline panel reports the documented reason instead of a value.
    let rendered = plain(&source(&model, 200, 40));
    assert!(rendered.contains("timeline unavailable reason=no_movie"));
}

#[test]
fn html_document_is_self_contained_and_script_free() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 2);
    let frame = text::frame(&source(&model, 80, 24)).expect("plain frame");
    let document = chronofork_gui::render::html::render(&[frame]).expect("document");

    assert!(document.starts_with("<!DOCTYPE html>\n"));
    assert!(document.contains("<meta charset=\"utf-8\">"));
    assert!(document.contains("<style>"));
    assert!(document.contains("<pre class=\"frame\" id=\"frame-1\">"));
    assert!(document.ends_with("</body>\n</html>\n"));

    let lowered = document.to_lowercase();
    for forbidden in [
        "<script",
        "</script",
        "<iframe",
        "<link",
        "<object",
        "<embed",
        "http://",
        "https://",
        "javascript:",
        "url(",
        " onload",
        " onerror",
        " onclick",
    ] {
        assert!(
            !lowered.contains(forbidden),
            "the document must not contain {forbidden:?}"
        );
    }
}

#[test]
fn html_escapes_an_adversarial_backend_descriptor() {
    // `id`, `display_name` and `version` are the untrusted strings a canonical descriptor
    // carries. They get distinct payloads so these assertions can tell which field reached
    // the document.
    let id = "<script>alert('id')&\u{22}</script>";
    let display_name = "<script>alert('name')&\u{22}</script>";
    let version = "<script>alert('version')&\u{22}</script>";
    let channel = ScriptedChannel::new(vec![inspect_result_with_descriptor(
        1,
        0,
        0,
        0,
        descriptor_with(id, display_name, version),
        &[("\u{1b}[31m", 7), ("d0", 0)],
    )]);
    let mut model = GuiModel::new(window());
    model.attach(&channel, 1).expect("attach");
    let frame = text::frame(&source(&model, 80, 24)).expect("plain frame");
    let frame_text = frame.to_text();
    let document = chronofork_gui::render::html::render(&[frame]).expect("document");

    // The capabilities panel renders the backend **identifier** — the only descriptor field
    // with a rendering path (`gui.md` §4 panel forms) — so the identifier reaches the grid.
    assert!(
        frame_text.contains("capabilities <script>alert('id')&\"</script>"),
        "the identifier reaches the grid: {frame_text}"
    );
    // `display_name` and `version` have no rendering path at all: the frame never carries
    // them, so they cannot smuggle anything into the document either. That absence is the
    // guarantee for those two fields; `html_escapes_any_grid_content_a_view_could_carry`
    // additionally proves that a grid carrying those exact strings escapes them.
    assert!(!frame_text.contains("alert('name')"));
    assert!(!frame_text.contains("alert('version')"));

    // The identifier appears in the document only in its escaped form, exactly once.
    assert!(document.contains("&lt;script&gt;alert(&#39;id&#39;)&amp;&quot;"));
    assert_eq!(
        document.matches("&lt;script&gt;").count(),
        1,
        "only the identifier reaches the document"
    );
    assert!(document.contains("&quot;"));
    assert!(document.contains("&#39;"));
    assert!(document.contains("&amp;"));
    for raw in [
        "<script>",
        "</script>",
        "alert('id')",
        "alert('name')",
        "alert('version')",
    ] {
        assert!(
            !document.contains(raw),
            "the document carries the raw form {raw:?}"
        );
    }
    for escaped in [
        "&lt;script&gt;alert(&#39;name",
        "&lt;script&gt;alert(&#39;version",
    ] {
        assert!(
            !document.contains(escaped),
            "an unrendered descriptor field reached the document: {escaped:?}"
        );
    }
    assert!(
        document.contains("?[31m=7"),
        "a control byte renders as `?`"
    );
    assert!(
        !document.contains('\u{1b}'),
        "no control byte reaches the document"
    );

    // The `<pre>` content only carries escaped text: every `<` in the grid became `&lt;`.
    let content = pre_content(&document);
    assert!(!content.contains("<script>"));
    assert!(!content.contains("</script>"));
    assert!(content.contains("&lt;script&gt;"));
}

#[test]
fn html_escapes_any_grid_content_a_view_could_carry() {
    // The HTML renderer's documented input is a validated [`Frame`], and a frame carries
    // printable ASCII only. Feed it the exact payloads `display_name` and `version` hold in
    // the adversarial descriptor above: if a future panel ever rendered those fields, the
    // document would still contain only escaped forms and no executable construct.
    let payloads = [
        "display_name=<script>alert('name')&\u{22}</script>",
        "version=<script>alert('version')&\u{22}</script>",
    ];
    let mut lines: Vec<String> = payloads
        .iter()
        .map(|payload| format!("{payload:<80}"))
        .collect();
    lines.resize(10, " ".repeat(80));
    let frame = Frame::new(80, 10, lines).expect("frame");
    let document =
        chronofork_gui::render::html::render(std::slice::from_ref(&frame)).expect("document");

    for escaped in [
        "&lt;script&gt;alert(&#39;name&#39;)&amp;&quot;&lt;/script&gt;",
        "&lt;script&gt;alert(&#39;version&#39;)&amp;&quot;&lt;/script&gt;",
    ] {
        assert!(
            document.contains(escaped),
            "the document must carry the escaped form {escaped:?}"
        );
    }
    assert_eq!(
        document.matches("&lt;script&gt;").count(),
        2,
        "both payloads are present, escaped"
    );
    for raw in [
        "<script",
        "</script>",
        "alert('name')",
        "alert('version')",
        "&\u{22}",
    ] {
        assert!(
            !document.contains(raw),
            "the raw form {raw:?} reached the document"
        );
    }
    assert_eq!(
        decode_entities(pre_content(&document)),
        frame.to_text(),
        "decoding the document returns the plain grid"
    );
}

/// Returns the content of the first `<pre>` element.
fn pre_content(document: &str) -> &str {
    let open = document.find("<pre").expect("a pre element");
    let content_start = open
        + document[open..]
            .find('>')
            .expect("the pre element has a closing bracket")
        + 1;
    let length = document[content_start..]
        .find("</pre>")
        .expect("the pre element is closed");
    &document[content_start..content_start + length]
}

/// Decodes exactly the five entities the HTML renderer emits.
fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[test]
fn html_pre_content_equals_the_plain_frame_after_entity_decoding() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 5);
    let frame = text::frame(&source(&model, 60, 16)).expect("plain frame");
    let plain = frame.to_text();
    let document = chronofork_gui::render::html::render(&[frame]).expect("document");
    assert_eq!(
        decode_entities(pre_content(&document)),
        plain,
        "the document embeds the very grid the terminal receives"
    );
}

#[test]
fn html_document_is_byte_identical_across_runs() {
    let mut model = GuiModel::new(window());
    attach(&mut model, 6);
    let frame = text::frame(&source(&model, 60, 16)).expect("plain frame");
    let first =
        chronofork_gui::render::html::render(std::slice::from_ref(&frame)).expect("document");
    let second =
        chronofork_gui::render::html::render(std::slice::from_ref(&frame)).expect("document");
    assert_eq!(first, second);
}

#[test]
fn html_rejects_a_document_over_the_maximum_size() {
    let line = "&".repeat(usize::try_from(chronofork_gui::MAX_WIDTH).expect("width"));
    let frame = Frame::new(
        chronofork_gui::MAX_WIDTH,
        chronofork_gui::MAX_HEIGHT,
        vec![line; usize::try_from(chronofork_gui::MAX_HEIGHT).expect("height")],
    )
    .expect("maximum frame");
    chronofork_gui::render::html::render(std::slice::from_ref(&frame))
        .expect("one maximum frame fits the document bound");

    let frames = vec![frame; 20];
    let error = chronofork_gui::render::html::render(&frames)
        .expect_err("twenty maximum frames exceed the document bound");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    assert!(matches!(
        error,
        chronofork_gui::GuiError::LimitExceeded {
            what: "html document bytes",
            ..
        }
    ));
}
