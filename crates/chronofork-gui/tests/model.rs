// Anti-slop policy §10 exception: owner ui-engineer; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It
// mirrors the existing headers in crates/chronofork-cli/tests/m6.rs and
// crates/chronofork-trace/tests/format.rs; no production code is affected.
#![allow(clippy::expect_used)]

//! Model-level acceptance tests for the M7 GUI foundation.
//!
//! These tests exercise the model with a **recording channel** and cover the model-side
//! halves of the M7 acceptance rows that do not need a spawned process:
//!
//! - M7-A2 — the canonical operation surface: `attach` issues exactly `session.inspect`
//!   plus one bounded `session.read_memory`, `create` issues exactly `session.create`, and
//!   `detach`, `focus`, `layout` and `panel` issue **zero** calls. No operation outside the
//!   documented set (`session.fork`, `session.snapshot`, `session.restore`,
//!   `session.close`, `session.compare`, `store.*`, `experiment.run`) is ever called.
//! - M7-A4 — `session.create` yields a `backend_id`, never a descriptor: the model gains no
//!   pane and no descriptor, and a create result that carries a `backend` object is
//!   rejected as an unknown field.
//! - M7-A5 — panes are allocated lowest-free-first, a fifth attach is a typed
//!   `limit_exceeded` that leaves the model unchanged, `focus`/`detach` report their typed
//!   failures, a detach moves focus to the lowest attached pane, and the generation
//!   increments per effective attach.
//! - M7-A6 — attach/detach is non-perturbative at the model level: `detach` issues no call
//!   at all, the memory read never exceeds `MAX_MEMORY_PANEL_BYTES`, and the session view
//!   (state, tick, hash, registers, memory window) is identical before a detach and after
//!   a re-attach. The rendered-frame comparison lands with the renderer that owns it; this
//!   test asserts the equality of the model values that frame would be rendered from.
//!
//! The `chronofork gui run` command engine belongs to the command layer task and is **not**
//! reimplemented here: these tests drive the model directly, the way that engine will.

use std::collections::VecDeque;
use std::sync::Mutex;

use chronofork_gui::{
    ChannelFault, GuiErrorCode, GuiModel, Layout, MAX_MEMORY_PANEL_BYTES, MAX_PANES,
    OperationChannel, PaneId, PanelKind, WindowId, decode_capabilities,
};
use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};

const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";

/// One recorded operation call: the operation name plus its decoded input object.
#[derive(Clone, Debug)]
struct Call {
    op: String,
    input: JsonValue,
}

/// A channel that records every call and answers from a fixed script.
struct RecordingChannel {
    calls: Mutex<Vec<Call>>,
    responses: Mutex<VecDeque<Result<Vec<u8>, ChannelFault>>>,
}

impl RecordingChannel {
    fn new(script: Vec<Result<Vec<u8>, ChannelFault>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(script.into()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("calls lock")
            .iter()
            .map(|call| call.op.clone())
            .collect()
    }

    fn inputs(&self) -> Vec<JsonValue> {
        self.calls
            .lock()
            .expect("calls lock")
            .iter()
            .map(|call| call.input.clone())
            .collect()
    }

    fn clear(&self) {
        self.calls.lock().expect("calls lock").clear();
    }
}

impl OperationChannel for RecordingChannel {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault> {
        let limits = ServiceLimits::default();
        let document = parse(request, &limits).expect("canonical request");
        let op = document
            .get("op")
            .and_then(JsonValue::as_str)
            .expect("operation name")
            .to_owned();
        let input = document.get("input").cloned().expect("input object");
        self.calls
            .lock()
            .expect("calls lock")
            .push(Call { op, input });
        let mut responses = self.responses.lock().expect("responses lock");
        match responses.pop_front() {
            Some(response) => response,
            None => Err(ChannelFault::new("no scripted response left")),
        }
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

fn empty_object() -> JsonValue {
    JsonValue::Object(std::collections::BTreeMap::new())
}

fn descriptor(rendering: bool, audio: bool) -> JsonValue {
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
    JsonValue::object([
        ("id", JsonValue::string("synthetic")),
        ("display_name", JsonValue::string("Synthetic")),
        ("version", JsonValue::string("0.1.0")),
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

fn capability_result() -> Vec<u8> {
    envelope(
        "capabilities",
        JsonValue::object([
            ("schema", JsonValue::string("chronofork.capabilities.v1")),
            ("service_version", JsonValue::string("0.1.0")),
            ("backend", descriptor(false, false)),
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
        ]),
    )
}

fn create_result(session: u64, backend_id: &str) -> Vec<u8> {
    envelope(
        "session.create",
        JsonValue::object([
            (
                "session",
                JsonValue::int(i64::try_from(session).expect("session")),
            ),
            ("backend_id", JsonValue::string(backend_id)),
            ("state", JsonValue::string("ready")),
            ("tick", JsonValue::int(0)),
            ("state_hash", JsonValue::string(HASH)),
            ("lineage", lineage()),
        ]),
    )
}

fn create_result_with_a_descriptor(session: u64) -> Vec<u8> {
    envelope(
        "session.create",
        JsonValue::object([
            (
                "session",
                JsonValue::int(i64::try_from(session).expect("session")),
            ),
            ("backend_id", JsonValue::string("synthetic")),
            ("state", JsonValue::string("ready")),
            ("tick", JsonValue::int(0)),
            ("state_hash", JsonValue::string(HASH)),
            ("lineage", lineage()),
            ("backend", descriptor(false, false)),
        ]),
    )
}

fn inspect_result(session: u64, tick: u64, memory_base: u64, memory_length: u64) -> Vec<u8> {
    envelope(
        "session.inspect",
        JsonValue::object([
            (
                "session",
                JsonValue::int(i64::try_from(session).expect("session")),
            ),
            ("state", JsonValue::string("ready")),
            ("lineage", lineage()),
            ("backend", descriptor(false, false)),
            (
                "machine",
                JsonValue::object([
                    ("tick", JsonValue::int(i64::try_from(tick).expect("tick"))),
                    ("state_hash", JsonValue::string(HASH)),
                    ("halted", JsonValue::from_bool(false)),
                    (
                        "registers",
                        JsonValue::object([
                            ("d0", JsonValue::int(0)),
                            ("pc", JsonValue::int(1024)),
                        ]),
                    ),
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

fn input_unsigned(input: &JsonValue, field: &str) -> u64 {
    input
        .get(field)
        .and_then(JsonValue::as_u64)
        .expect("documented request field")
}

/// A scripted attach of one session with no memory window: one `session.inspect` response.
fn attach_script(session: u64) -> Vec<Result<Vec<u8>, ChannelFault>> {
    vec![Ok(inspect_result(session, 0, 0, 0))]
}

fn attach_with_memory_script(
    session: u64,
    memory_base: u64,
    memory_length: u64,
) -> Vec<Result<Vec<u8>, ChannelFault>> {
    let window = u64::try_from(MAX_MEMORY_PANEL_BYTES).expect("bound");
    let window = memory_length.min(window);
    let length = usize::try_from(window).expect("window");
    vec![
        Ok(inspect_result(session, 0, memory_base, memory_length)),
        Ok(memory_result(session, memory_base, &vec![0x5a; length])),
    ]
}

#[test]
fn create_decodes_a_backend_id_and_never_a_descriptor() {
    let channel = RecordingChannel::new(vec![Ok(create_result(1, "synthetic"))]);
    let mut model = GuiModel::new(window());

    let view = model.create(&channel).expect("create");
    assert_eq!(view.backend_id(), "synthetic");
    assert_eq!(view.session(), 1);
    assert_eq!(channel.calls(), ["session.create"]);
    assert_eq!(model.attached(), 0);
    assert_eq!(model.focused(), None);
    assert!(model.panes().next().is_none());
    assert_eq!(model.events().len(), 1);
    let event = model.events().last().expect("create event");
    assert_eq!(event.kind(), chronofork_gui::EventKind::Create);
    assert_eq!(event.session(), Some(1));
    assert_eq!(event.pane(), None);

    // A create result that (incorrectly) carries a descriptor is not a descriptor.
    let channel = RecordingChannel::new(vec![Ok(create_result_with_a_descriptor(2))]);
    let error = model.create(&channel).expect_err("unknown backend field");
    assert_eq!(error.code(), GuiErrorCode::Channel);
    assert!(error.to_string().contains("unknown field `backend`"));
    assert_eq!(model.attached(), 0, "a rejected create attaches nothing");
    assert_eq!(model.events().len(), 1, "a rejected create records nothing");
}

#[test]
fn pane_allocation_is_lowest_free_and_bounded() {
    let mut script = Vec::new();
    for session in 1..=4_u64 {
        script.extend(attach_script(session));
    }
    let channel = RecordingChannel::new(script);
    let mut model = GuiModel::new(window());

    for (index, report) in (1..=4_u64)
        .map(|session| model.attach(&channel, session).expect("attach"))
        .enumerate()
    {
        assert_eq!(report.pane, PaneId::ALL[index]);
        assert_eq!(report.generation, 1);
        assert!(!report.reused);
    }
    assert_eq!(model.attached(), MAX_PANES);
    assert_eq!(model.focused(), Some(PaneId::ALL[MAX_PANES - 1]));
    let calls_before = channel.calls().len();

    let error = model.attach(&channel, 5).expect_err("fifth attach");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    assert_eq!(
        model.attached(),
        MAX_PANES,
        "a rejected attach changes nothing"
    );
    assert_eq!(
        model.focused(),
        Some(PaneId::ALL[MAX_PANES - 1]),
        "a rejected attach does not move focus"
    );
    assert_eq!(
        channel.calls().len(),
        calls_before,
        "a rejected attach performs no call"
    );

    // The lowest free slot is reused after a detach, not the highest one.
    model.focus(PaneId::ALL[1]).expect("focus");
    model.detach().expect("detach");
    let channel = RecordingChannel::new(attach_script(5));
    let report = model.attach(&channel, 5).expect("reuse the freed slot");
    assert_eq!(report.pane, PaneId::ALL[1]);
    assert_eq!(report.generation, 2);
    assert_eq!(model.focused(), Some(PaneId::ALL[1]));
}

#[test]
fn pane_generation_increments_per_new_attachment_only() {
    let channel = RecordingChannel::new(attach_script(7));
    let mut model = GuiModel::new(window());

    let first = model.attach(&channel, 7).expect("attach");
    assert_eq!(first.generation, 1);
    assert_eq!(channel.calls(), ["session.inspect"]);

    // Attaching an already attached session only focuses it.
    channel.clear();
    let again = model.attach(&channel, 7).expect("idempotent attach");
    assert_eq!(again.pane, first.pane);
    assert_eq!(
        again.generation, 1,
        "an idempotent attach keeps the generation"
    );
    assert!(again.reused);
    assert!(
        channel.calls().is_empty(),
        "an idempotent attach calls nothing"
    );
    assert_eq!(model.attached(), 1);

    // A detach followed by a new attach increments the slot generation.
    model.detach().expect("detach");
    let channel = RecordingChannel::new(attach_script(7));
    let third = model.attach(&channel, 7).expect("re-attach");
    assert_eq!(third.pane, first.pane);
    assert_eq!(third.generation, 2);
    assert!(!third.reused);
}

#[test]
fn pane_focus_and_detach_report_typed_failures() {
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
    assert_eq!(
        PaneId::new(0).expect_err("zero is not a pane").code(),
        GuiErrorCode::UnknownPane
    );
    assert_eq!(
        PaneId::new(u32::try_from(MAX_PANES).expect("bound") + 1)
            .expect_err("out of range")
            .code(),
        GuiErrorCode::UnknownPane
    );
    assert_eq!(model.attached(), 0);
    assert_eq!(model.focused(), None);
}

#[test]
fn attach_detach_issues_only_the_canonical_reads() {
    let channel = RecordingChannel::new(attach_with_memory_script(3, 4096, 1024));
    let mut model = GuiModel::new(window());

    let report = model.attach(&channel, 3).expect("attach");
    assert_eq!(report.pane, PaneId::ALL[0]);
    assert_eq!(channel.calls(), ["session.inspect", "session.read_memory"]);

    // The memory window is bounded by MAX_MEMORY_PANEL_BYTES and starts at the base the
    // machine reports; nothing wider is ever requested.
    let inputs = channel.inputs();
    assert_eq!(input_unsigned(&inputs[1], "session"), 3);
    assert_eq!(input_unsigned(&inputs[1], "address"), 4096);
    assert_eq!(
        input_unsigned(&inputs[1], "length"),
        u64::try_from(MAX_MEMORY_PANEL_BYTES).expect("bound")
    );
    let pane = model.pane(report.pane).expect("pane");
    let memory = pane.memory().expect("memory window");
    assert_eq!(memory.address(), 4096);
    assert_eq!(
        memory.length(),
        u64::try_from(MAX_MEMORY_PANEL_BYTES).expect("bound")
    );
    assert_eq!(memory.bytes().len(), MAX_MEMORY_PANEL_BYTES);

    let attach_kinds: Vec<chronofork_gui::EventKind> = model
        .events()
        .entries()
        .map(chronofork_gui::Event::kind)
        .collect();
    assert_eq!(attach_kinds, [chronofork_gui::EventKind::Attach]);

    // Detaching performs no call at all.
    channel.clear();
    let detached = model.detach().expect("detach");
    assert_eq!(detached.pane, report.pane);
    assert_eq!(detached.session, 3);
    assert!(channel.calls().is_empty(), "detach calls nothing");
    assert_eq!(model.attached(), 0);
    assert_eq!(model.focused(), None);

    // Re-attaching after a detach repeats exactly the read sequence.
    let channel = RecordingChannel::new(attach_with_memory_script(3, 4096, 1024));
    model.attach(&channel, 3).expect("re-attach");
    assert_eq!(channel.calls(), ["session.inspect", "session.read_memory"]);

    let kinds: Vec<chronofork_gui::EventKind> = model
        .events()
        .entries()
        .map(chronofork_gui::Event::kind)
        .collect();
    assert_eq!(
        kinds,
        [
            chronofork_gui::EventKind::Attach,
            chronofork_gui::EventKind::Detach,
            chronofork_gui::EventKind::Attach
        ]
    );
    let sequences: Vec<u64> = model
        .events()
        .entries()
        .map(|event| event.sequence())
        .collect();
    assert_eq!(sequences, [0, 1, 2], "event sequences are monotonic");
}

#[test]
fn attach_detach_preserves_the_session_view() {
    let channel = RecordingChannel::new(attach_with_memory_script(9, 8192, 64));
    let mut model = GuiModel::new(window());
    let report = model.attach(&channel, 9).expect("attach");
    let before = {
        let pane = model.pane(report.pane).expect("pane");
        (
            pane.session(),
            pane.generation(),
            pane.view().state(),
            pane.view().machine().tick(),
            pane.view().machine().state_hash(),
            pane.view().machine().halted(),
            pane.view().machine().registers().clone(),
            pane.view().machine().memory_base(),
            pane.view().machine().memory_length(),
            pane.view().backend().id().to_owned(),
            pane.memory().map(|memory| memory.bytes().to_vec()),
        )
    };

    model.detach().expect("detach");
    assert!(
        model.pane(report.pane).is_none(),
        "a detach clears the slot"
    );

    let channel = RecordingChannel::new(attach_with_memory_script(9, 8192, 64));
    let report = model.attach(&channel, 9).expect("re-attach");
    let after = {
        let pane = model.pane(report.pane).expect("pane");
        (
            pane.session(),
            pane.generation(),
            pane.view().state(),
            pane.view().machine().tick(),
            pane.view().machine().state_hash(),
            pane.view().machine().halted(),
            pane.view().machine().registers().clone(),
            pane.view().machine().memory_base(),
            pane.view().machine().memory_length(),
            pane.view().backend().id().to_owned(),
            pane.memory().map(|memory| memory.bytes().to_vec()),
        )
    };

    // Every value a frame is rendered from is unchanged; only the generation moved.
    assert_eq!(before.0, after.0);
    assert_eq!(before.2, after.2);
    assert_eq!(before.3, after.3);
    assert_eq!(before.4, after.4);
    assert_eq!(before.5, after.5);
    assert_eq!(before.6, after.6);
    assert_eq!(before.7, after.7);
    assert_eq!(before.8, after.8);
    assert_eq!(before.9, after.9);
    assert_eq!(before.10, after.10);
    assert_eq!(after.1, before.1 + 1, "only the generation advanced");
}

#[test]
fn pane_layout_and_panel_changes_issue_no_operation() {
    let channel = RecordingChannel::new(attach_script(4));
    let mut model = GuiModel::new(window());
    let report = model.attach(&channel, 4).expect("attach");
    channel.clear();

    model.focus(report.pane).expect("focus");
    model.set_layout(Layout::Mosaic);
    model.set_panel(PanelKind::Memory, true);
    model.set_panel(PanelKind::Timeline, true);
    model.toggle_pause();
    model.advance_frames(4);
    model.focus(report.pane).expect("focus again");
    model.detach().expect("detach");

    assert!(
        channel.calls().is_empty(),
        "model-only commands perform no operation, got {:?}",
        channel.calls()
    );
    assert_eq!(model.layout(), Layout::Mosaic);
    assert!(model.panel(PanelKind::Memory).visible());
    assert!(model.panel(PanelKind::Timeline).visible());
    assert_eq!(model.frame_index(), 4);
}

#[test]
fn the_model_never_calls_an_operation_outside_the_documented_set() {
    let mut script = vec![Ok(create_result(3, "synthetic"))];
    script.extend(attach_with_memory_script(2, 4096, 512));
    script.push(Ok(inspect_result(3, 0, 0, 0)));
    let channel = RecordingChannel::new(script);
    let mut model = GuiModel::new(window());

    model.create(&channel).expect("create");
    model.attach(&channel, 2).expect("attach with memory");
    model.attach(&channel, 3).expect("attach without memory");
    model.focus(PaneId::ALL[0]).expect("focus");
    model.set_layout(Layout::Mosaic);
    model.detach().expect("detach");

    for op in channel.calls() {
        assert!(
            matches!(
                op.as_str(),
                "session.create" | "session.inspect" | "session.read_memory"
            ),
            "undocumented operation `{op}` was called"
        );
    }
}

#[test]
fn a_channel_fault_leaves_the_model_unchanged() {
    let envelope = b"{\"error\":{\"code\":\"forbidden\",\"message\":\"nope\",\"retryable\":false},\"schema\":\"chronofork.error.v1\"}".to_vec();
    let channel = RecordingChannel::new(vec![Err(ChannelFault::with_envelope(
        "session.inspect failed",
        envelope.clone(),
    ))]);
    let mut model = GuiModel::new(window());

    let error = model.attach(&channel, 1).expect_err("channel failure");
    assert_eq!(error.code(), GuiErrorCode::Channel);
    assert_eq!(error.envelope(), Some(envelope.as_slice()));
    assert_eq!(model.attached(), 0);
    assert_eq!(model.focused(), None);
    assert!(model.events().is_empty());
    assert_eq!(channel.calls(), ["session.inspect"]);
}

#[test]
fn create_and_attach_decode_the_documented_operation_results() {
    let channel = RecordingChannel::new(vec![
        Ok(capability_result()),
        Ok(create_result(1, "synthetic")),
        Ok(inspect_result(1, 0, 0, 0)),
    ]);

    let capabilities = decode_capabilities(
        &channel
            .call(&request_for("capabilities", empty_object()))
            .expect("capabilities response"),
    )
    .expect("capabilities view");
    assert_eq!(capabilities.backend().id(), "synthetic");
    assert!(!capabilities.backend().capabilities().rendering());
    assert!(!capabilities.backend().capabilities().audio());
    assert_eq!(capabilities.limits().max_steps_per_request(), 100_000);

    let mut model = GuiModel::new(window());
    let created = model.create(&channel).expect("create");
    assert_eq!(created.backend_id(), "synthetic");
    model.attach(&channel, 1).expect("attach");
    let pane = model.pane(PaneId::ALL[0]).expect("pane");
    assert_eq!(pane.view().backend().id(), "synthetic");
    assert_eq!(pane.view().machine().tick(), 0);
    assert_eq!(
        channel.calls(),
        ["capabilities", "session.create", "session.inspect"]
    );
}

/// Encodes one request the way the crate does, for the assertions above.
fn request_for(op: &str, input: JsonValue) -> Vec<u8> {
    chronofork_gui::request_bytes(op, &input).expect("request bytes")
}
