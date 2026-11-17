// Anti-slop policy §10 exception: owner runtime-engineer; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It mirrors
// the existing headers in crates/chronofork-gui/tests/render.rs; no production code is
// affected.
#![allow(clippy::expect_used)]

//! Command-engine acceptance tests for the M7 GUI (rows M7-A2, M7-A4 and M7-A6).
//!
//! The engine is driven through [`CommandSession`] against a **recording channel** that
//! answers the six documented operations and fails every other request, so the operation set
//! is an assertion rather than a promise:
//!
//! - M7-A2 — the canonical operation surface: `open` issues `capabilities` exactly once; the
//!   full script issues `session.create`, `session.list`, `session.inspect`,
//!   `session.read_memory` and `session.run` and nothing else; `detach`, `focus`, `layout`,
//!   `panel`, `pause`, `handoff`, `frame` and `html` issue zero calls.
//! - M7-A4 — `session.create` yields a `backend_id`, not a descriptor: the block carries the
//!   documented field order, the model gains no descriptor until an `attach` performs
//!   `session.inspect`, and a create response that carries a `backend` object is rejected by
//!   the strict decoder.
//! - M7-A6 — attach/detach is non-perturbative: `attach` issues `session.inspect` plus at most
//!   one bounded `session.read_memory`, `detach` issues nothing, a re-attach renders a
//!   byte-identical frame for an unchanged session, and the blocks carry the documented
//!   fields in order.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chronofork_core::hash_bytes;
use chronofork_gui::command::{
    Block, BlockField, CommandConfig, CommandOutcome, CommandSession, QuitReason, Record,
};
use chronofork_gui::pacing::{ManualClock, PacerConfig};
use chronofork_gui::render::RenderOptions;
use chronofork_gui::{
    ChannelFault, GuiErrorCode, MAX_MEMORY_PANEL_BYTES, OperationChannel, PanelKind,
};
use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};

const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";
const BACKEND_ID: &str = "chronofork.test.synthetic";
const MEMORY_BASE: u64 = 4096;

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn canonical(value: &JsonValue) -> Vec<u8> {
    to_canonical_bytes(value, &limits()).expect("canonical bytes")
}

fn envelope(op: &str, result: JsonValue) -> Vec<u8> {
    canonical(&JsonValue::object([
        ("op", JsonValue::string(op)),
        ("result", result),
    ]))
}

fn int(value: u64) -> JsonValue {
    JsonValue::int(i64::try_from(value).expect("value fits a canonical integer"))
}

fn capabilities_result(max_steps: u64) -> JsonValue {
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
        ("schema", JsonValue::string("chronofork.capabilities.v1")),
        ("service_version", JsonValue::string("0.1.0")),
        (
            "backend",
            JsonValue::object([
                ("id", JsonValue::string(BACKEND_ID)),
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
                ("max_request_bytes", int(1_048_576)),
                ("max_response_bytes", int(1_048_576)),
                ("max_steps_per_request", int(max_steps)),
                ("max_memory_read", int(4096)),
                ("max_sessions", int(1024)),
                ("max_store_entries", int(256)),
                ("max_checkpoints", int(32)),
                ("json_max_depth", int(32)),
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
    ])
}

fn lineage() -> JsonValue {
    JsonValue::object([
        ("parent_session", JsonValue::Null),
        ("origin_snapshot", JsonValue::Null),
        ("depth", int(0)),
    ])
}

fn backend_descriptor() -> JsonValue {
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
        ("id", JsonValue::string(BACKEND_ID)),
        ("display_name", JsonValue::string("Synthetic")),
        ("version", JsonValue::string("0.1.0")),
        ("test_backend", JsonValue::from_bool(true)),
        ("capabilities", JsonValue::Object(flags)),
    ])
}

fn create_result(session: u64) -> JsonValue {
    JsonValue::object([
        ("session", int(session)),
        ("backend_id", JsonValue::string(BACKEND_ID)),
        ("state", JsonValue::string("ready")),
        ("tick", int(0)),
        ("state_hash", JsonValue::string(HASH)),
        ("lineage", lineage()),
    ])
}

fn list_result(sessions: &[(u64, u64)]) -> JsonValue {
    let entries = sessions
        .iter()
        .map(|(session, tick)| {
            JsonValue::object([
                ("session", int(*session)),
                ("state", JsonValue::string("ready")),
                ("tick", int(*tick)),
                ("state_hash", JsonValue::string(HASH)),
                ("lineage", lineage()),
            ])
        })
        .collect();
    JsonValue::object([
        ("sessions", JsonValue::Array(entries)),
        ("truncated", JsonValue::from_bool(false)),
    ])
}

fn inspect_result(session: u64, tick: u64, memory_length: u64) -> JsonValue {
    JsonValue::object([
        ("session", int(session)),
        ("state", JsonValue::string("ready")),
        ("lineage", lineage()),
        ("backend", backend_descriptor()),
        (
            "machine",
            JsonValue::object([
                ("tick", int(tick)),
                ("state_hash", JsonValue::string(HASH)),
                ("halted", JsonValue::from_bool(false)),
                (
                    "registers",
                    JsonValue::object([("d0", int(0)), ("pc", int(1024))]),
                ),
                ("memory_base", int(MEMORY_BASE)),
                ("memory_length", int(memory_length)),
            ]),
        ),
    ])
}

fn memory_result(session: u64, address: u64, length: u64) -> JsonValue {
    let bytes = usize::try_from(length).expect("length fits usize");
    JsonValue::object([
        ("session", int(session)),
        ("address", int(address)),
        ("length", int(length)),
        ("bytes", JsonValue::string("00".repeat(bytes))),
    ])
}

fn run_result(session: u64, steps: u64, tick: u64) -> JsonValue {
    JsonValue::object([
        ("session", int(session)),
        ("steps", int(steps)),
        ("tick", int(tick)),
        ("state_hash", JsonValue::string(HASH)),
        ("halted", JsonValue::from_bool(false)),
    ])
}

/// What one recording channel has observed.
#[derive(Debug)]
struct Recording {
    calls: Vec<(String, JsonValue)>,
    next_session: u64,
    tick: u64,
}

/// A channel that records every operation and answers only the documented six.
///
/// Any other operation is recorded and then refused, so a prohibited call cannot pass
/// unnoticed: it turns into a typed channel failure the test would see.
struct RecordingChannel {
    state: Mutex<Recording>,
    memory_length: u64,
    max_steps: u64,
}

impl RecordingChannel {
    fn new() -> Self {
        Self {
            state: Mutex::new(Recording {
                calls: Vec::new(),
                next_session: 1,
                tick: 0,
            }),
            memory_length: 4096,
            max_steps: 100,
        }
    }

    fn with_memory_length(memory_length: u64) -> Self {
        Self {
            memory_length,
            ..Self::new()
        }
    }

    fn ops(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("recording lock")
            .calls
            .iter()
            .map(|(op, _)| op.clone())
            .collect()
    }

    fn call_count(&self) -> usize {
        self.state.lock().expect("recording lock").calls.len()
    }

    fn inputs(&self, op: &str) -> Vec<JsonValue> {
        self.state
            .lock()
            .expect("recording lock")
            .calls
            .iter()
            .filter(|(name, _)| name == op)
            .map(|(_, input)| input.clone())
            .collect()
    }
}

impl OperationChannel for RecordingChannel {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault> {
        let document =
            parse(request, &limits()).map_err(|error| ChannelFault::new(&error.to_string()))?;
        let op = document
            .get("op")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| ChannelFault::new("the request carries no operation name"))?
            .to_owned();
        let input = document
            .get("input")
            .cloned()
            .ok_or_else(|| ChannelFault::new("the request carries no input object"))?;
        let mut state = self.state.lock().expect("recording lock");
        state.calls.push((op.clone(), input.clone()));
        match op.as_str() {
            "capabilities" => Ok(envelope(
                "capabilities",
                capabilities_result(self.max_steps),
            )),
            "session.create" => {
                let session = state.next_session;
                state.next_session = session.saturating_add(1);
                Ok(envelope("session.create", create_result(session)))
            }
            "session.list" => Ok(envelope("session.list", list_result(&[(2, 40), (1, 0)]))),
            "session.inspect" => {
                let session = input
                    .get("session")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("inspect without a session"))?;
                Ok(envelope(
                    "session.inspect",
                    inspect_result(session, state.tick, self.memory_length),
                ))
            }
            "session.read_memory" => {
                let session = input
                    .get("session")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("read_memory without a session"))?;
                let address = input
                    .get("address")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("read_memory without an address"))?;
                let length = input
                    .get("length")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("read_memory without a length"))?;
                Ok(envelope(
                    "session.read_memory",
                    memory_result(session, address, length),
                ))
            }
            "session.run" => {
                let session = input
                    .get("session")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("run without a session"))?;
                let steps = input
                    .get("steps")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("run without a step count"))?;
                state.tick = state.tick.saturating_add(steps);
                Ok(envelope(
                    "session.run",
                    run_result(session, steps, state.tick),
                ))
            }
            other => Err(ChannelFault::new(&format!(
                "the GUI requested an undocumented operation `{other}`"
            ))),
        }
    }
}

fn config() -> CommandConfig {
    CommandConfig {
        width: 80,
        height: 24,
        render: RenderOptions::plain(),
        pacer: PacerConfig::new(60, 1, 4).expect("cadence"),
        timeline: None,
    }
}

fn open(channel: &Arc<RecordingChannel>) -> CommandSession {
    CommandSession::open(
        Arc::clone(channel) as Arc<dyn OperationChannel>,
        Arc::new(ManualClock::new()),
        config(),
    )
    .expect("open the session")
}

fn as_block(outcome: Option<CommandOutcome>) -> Option<Block> {
    match outcome? {
        CommandOutcome::Block(block) => Some(block),
        CommandOutcome::Grid(_) | CommandOutcome::Html { .. } => None,
    }
}

fn as_grid(outcome: Option<CommandOutcome>) -> Option<String> {
    match outcome? {
        CommandOutcome::Grid(text) => Some(text),
        CommandOutcome::Block(_) | CommandOutcome::Html { .. } => None,
    }
}

fn as_html(outcome: Option<CommandOutcome>) -> Option<(String, Block)> {
    match outcome? {
        CommandOutcome::Html { document, block } => Some((document, block)),
        CommandOutcome::Block(_) | CommandOutcome::Grid(_) => None,
    }
}

fn block(outcome: Option<CommandOutcome>) -> Block {
    as_block(outcome).expect("a block outcome")
}

fn grid(outcome: Option<CommandOutcome>) -> String {
    as_grid(outcome).expect("a frame grid outcome")
}

fn html(outcome: Option<CommandOutcome>) -> (String, Block) {
    as_html(outcome).expect("an html outcome")
}

/// Renders one block field value the way the documented process surface spells it.
fn rendered(field: &BlockField) -> String {
    match field {
        BlockField::Uint(_, value) => value.to_string(),
        BlockField::Bool(_, value) => value.to_string(),
        BlockField::Text(_, value) => value.clone(),
        BlockField::OptionalUint(_, Some(value)) => value.to_string(),
        BlockField::OptionalUint(_, None) => "none".to_owned(),
    }
}

fn pairs(block: &Block) -> Vec<(String, String)> {
    block
        .fields
        .iter()
        .map(|field| (field.name().to_owned(), rendered(field)))
        .collect()
}

fn names(block: &Block) -> Vec<String> {
    block
        .fields
        .iter()
        .map(|field| field.name().to_owned())
        .collect()
}

fn execute(session: &mut CommandSession, line: &str) -> Block {
    block(session.execute(line).expect("the line executes"))
}

/// Returns the tick of the pane the window presents, so a loop assertion does not depend on
/// how the frame renders it.
fn focused_tick(session: &CommandSession) -> u64 {
    let pane = session.model().focused().expect("a focused pane");
    session
        .model()
        .pane(pane)
        .expect("an attached pane")
        .view()
        .machine()
        .tick()
}

#[test]
fn opening_the_window_issues_capabilities_exactly_once() {
    let channel = Arc::new(RecordingChannel::new());
    let session = open(&channel);
    assert_eq!(channel.ops(), ["capabilities"]);
    assert_eq!(session.lease().generation(), 0);
    assert_eq!(session.model().frame_index(), 0);
    assert_eq!(session.model().attached(), 0);
}

#[test]
fn the_documented_script_issues_exactly_the_documented_operations() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = CommandSession::open(
        Arc::clone(&channel) as Arc<dyn OperationChannel>,
        Arc::new(ManualClock::new()),
        CommandConfig {
            pacer: PacerConfig::new(60, 3, 4).expect("cadence"),
            ..config()
        },
    )
    .expect("open the session");

    for line in [
        "create",
        "list",
        "attach 1",
        "focus 1",
        "layout mosaic",
        "panel memory on",
        "step 1",
        "run 2",
        "pulse 2",
        "pause",
        "handoff agent",
        "handoff human",
        "frame",
        "html",
        "detach",
        "quit",
    ] {
        session.execute(line).expect("the line executes");
    }

    assert_eq!(
        channel.ops(),
        [
            "capabilities",
            "session.create",
            "session.list",
            "session.inspect",
            "session.read_memory",
            "session.run",
            "session.run",
            "session.run",
            "session.run",
        ],
        "the operation surface is exactly the documented set, in order"
    );

    let step_inputs = channel.inputs("session.run");
    let steps: Vec<u64> = step_inputs
        .iter()
        .map(|input| {
            input
                .get("steps")
                .and_then(JsonValue::as_u64)
                .expect("steps")
        })
        .collect();
    assert_eq!(
        steps,
        [1, 2, 3, 3],
        "step, run and one run per pulsed frame"
    );
    for input in &step_inputs {
        assert_eq!(
            input.get("session").and_then(JsonValue::as_u64),
            Some(1),
            "every run names the focused session"
        );
        assert_eq!(
            input.get("buttons"),
            None,
            "the engine sends neutral input and no button field"
        );
    }
}

#[test]
fn observation_commands_issue_no_operation() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    let before = channel.call_count();

    for line in [
        "focus 1",
        "layout mosaic",
        "panel memory on",
        "pause",
        "handoff agent",
        "frame",
        "html",
        "detach",
    ] {
        session.execute(line).expect("the line executes");
    }
    assert_eq!(
        channel.call_count(),
        before,
        "observation commands and handoff reach the channel zero times"
    );
}

#[test]
fn no_prohibited_operation_is_ever_requested() {
    let documented = [
        "capabilities",
        "session.create",
        "session.list",
        "session.inspect",
        "session.read_memory",
        "session.run",
    ];
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    for line in [
        "create", "list", "attach 1", "step", "run 1", "pulse 1", "frame", "html", "detach", "quit",
    ] {
        session.execute(line).expect("the line executes");
    }
    for op in channel.ops() {
        assert!(
            documented.contains(&op.as_str()),
            "the engine requested `{op}`, which is not part of the documented surface"
        );
        for prohibited in [
            "session.fork",
            "session.snapshot",
            "session.restore",
            "session.close",
            "session.compare",
            "store.list",
            "store.load",
            "store.save",
            "store.delete",
            "experiment.run",
        ] {
            assert_ne!(op, prohibited);
        }
    }
}

#[test]
fn create_yields_backend_id_and_no_descriptor() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    let created = execute(&mut session, "create");
    assert_eq!(created.record, Record::Create);
    assert_eq!(
        names(&created),
        [
            "session",
            "backend_id",
            "state",
            "tick",
            "state_hash",
            "lineage_parent",
            "lineage_origin_snapshot",
            "lineage_depth",
        ]
    );
    assert_eq!(
        pairs(&created)[1],
        ("backend_id".to_owned(), BACKEND_ID.to_owned())
    );
    assert_eq!(
        pairs(&created)[4],
        ("state_hash".to_owned(), HASH.to_owned())
    );
    assert_eq!(
        pairs(&created)[5],
        ("lineage_parent".to_owned(), "none".to_owned())
    );

    // No descriptor and no pane exist until an attach performs session.inspect.
    assert_eq!(session.model().attached(), 0);
    assert_eq!(session.model().focused(), None);
    let frame = grid(session.execute("frame").expect("frame"));
    assert!(
        frame.contains("panes=0 focused=none"),
        "status line: {frame}"
    );
    assert!(
        !frame.contains(BACKEND_ID),
        "the window holds no backend descriptor before an attach"
    );
    assert_eq!(channel.ops(), ["capabilities", "session.create"]);
}

#[test]
fn a_create_response_carrying_a_descriptor_is_rejected() {
    /// A channel whose `session.create` result carries an extra `backend` object.
    struct DescriptorChannel;

    impl OperationChannel for DescriptorChannel {
        fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault> {
            let document =
                parse(request, &limits()).map_err(|error| ChannelFault::new(&error.to_string()))?;
            let op = document
                .get("op")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| ChannelFault::new("no operation name"))?;
            match op {
                "capabilities" => Ok(envelope("capabilities", capabilities_result(100))),
                "session.create" => {
                    let mut result = create_result(1);
                    if let JsonValue::Object(fields) = &mut result {
                        fields.insert("backend".to_owned(), backend_descriptor());
                    }
                    Ok(envelope("session.create", result))
                }
                other => Err(ChannelFault::new(&format!("unexpected `{other}`"))),
            }
        }
    }

    let mut session = CommandSession::open(
        Arc::new(DescriptorChannel),
        Arc::new(ManualClock::new()),
        config(),
    )
    .expect("open the session");
    let error = session
        .execute("create")
        .expect_err("the descriptor is not a create field");
    assert_eq!(error.code(), GuiErrorCode::Channel);
    assert!(
        error.to_string().contains("backend"),
        "the strict decoder names the unknown field: {error}"
    );
    assert_eq!(session.model().events().len(), 0, "no event is recorded");
}

#[test]
fn attach_issues_inspect_and_one_bounded_memory_read() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");

    assert_eq!(
        channel.ops(),
        [
            "capabilities",
            "session.create",
            "session.inspect",
            "session.read_memory"
        ]
    );
    let inspect = channel.inputs("session.inspect");
    assert_eq!(inspect.len(), 1);
    assert_eq!(
        inspect[0].get("session").and_then(JsonValue::as_u64),
        Some(1)
    );

    let memory = channel.inputs("session.read_memory");
    assert_eq!(memory.len(), 1, "exactly one bounded read at attach");
    assert_eq!(
        memory[0].get("address").and_then(JsonValue::as_u64),
        Some(MEMORY_BASE)
    );
    let length = memory[0]
        .get("length")
        .and_then(JsonValue::as_u64)
        .expect("length");
    assert_eq!(
        length,
        u64::try_from(MAX_MEMORY_PANEL_BYTES).expect("bound fits")
    );
}

#[test]
fn attach_without_memory_issues_only_inspect() {
    let channel = Arc::new(RecordingChannel::with_memory_length(0));
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    assert_eq!(
        channel.ops(),
        ["capabilities", "session.create", "session.inspect"]
    );
    assert!(channel.inputs("session.read_memory").is_empty());
}

#[test]
fn attach_detach_changes_only_the_events_panel_line() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    // The `events` panel stays visible: the presentation log is exactly the one place the
    // frame is allowed to differ across a detach and a re-attach.
    let before = grid(session.execute("frame").expect("frame"));
    execute(&mut session, "detach");
    execute(&mut session, "attach 1");
    let after = grid(session.execute("frame").expect("frame"));

    let first: Vec<&str> = before.lines().collect();
    let second: Vec<&str> = after.lines().collect();
    assert_eq!(
        first.len(),
        second.len(),
        "both frames have the same height"
    );
    let differing: Vec<usize> = first
        .iter()
        .zip(second.iter())
        .enumerate()
        .filter(|(_, (left, right))| left != right)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        differing.len(),
        1,
        "only the presentation log line differs: {differing:?}"
    );
    let index = differing[0];
    assert!(
        first[index].starts_with("events "),
        "the differing line is the events panel: {:?}",
        first[index]
    );
    assert_eq!(
        first[index].trim_end(),
        "events 2 last=attach sequence=1",
        "before the detach the log records create and attach"
    );
    assert_eq!(
        second[index].trim_end(),
        "events 4 last=attach sequence=3",
        "after the re-attach the log additionally records detach and attach"
    );

    // Replacing the log line makes the two grids byte-identical: the session content did not
    // change, only the presentation history did.
    let normalized = |lines: &[&str]| -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                if line.starts_with("events ") {
                    "events <log>".to_owned()
                } else {
                    (*line).to_owned()
                }
            })
            .collect()
    };
    assert_eq!(normalized(&first), normalized(&second));
}

#[test]
fn attach_detach_issues_no_call_and_a_reattach_renders_the_same_frame() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    // The presentation event log is the one panel whose content must change; hide it so the
    // comparison is about the session the panes hold.
    execute(&mut session, "panel events off");
    let before = grid(session.execute("frame").expect("frame"));
    let calls_before = channel.call_count();

    let detached = execute(&mut session, "detach");
    assert_eq!(
        channel.call_count(),
        calls_before,
        "detach is a presentation act"
    );
    assert_eq!(
        pairs(&detached)[3],
        ("attached".to_owned(), "false".to_owned())
    );

    let reattached = execute(&mut session, "attach 1");
    assert_eq!(
        channel.call_count(),
        calls_before + 2,
        "a re-attach repeats inspect and the bounded read"
    );
    let after = grid(session.execute("frame").expect("frame"));
    assert_eq!(
        before, after,
        "the rendered frame is byte-identical for an unchanged session"
    );
    assert_eq!(
        pairs(&reattached)[2],
        ("generation".to_owned(), "2".to_owned()),
        "a re-attach allocates a new generation for the pane slot"
    );
    assert_eq!(pairs(&reattached)[0], ("pane".to_owned(), "1".to_owned()));

    let kinds: Vec<&str> = session
        .model()
        .events()
        .entries()
        .map(|event| event.kind().name())
        .collect();
    assert_eq!(
        kinds,
        ["create", "attach", "panel", "detach", "attach"],
        "one attach, one detach and one re-attach with monotonic sequences"
    );
    let sequences: Vec<u64> = session
        .model()
        .events()
        .entries()
        .map(|event| event.sequence())
        .collect();
    assert_eq!(sequences, [0, 1, 2, 3, 4]);
}

#[test]
fn attach_and_detach_blocks_carry_the_documented_fields() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    let attached = execute(&mut session, "attach 1");
    assert_eq!(
        names(&attached),
        [
            "pane",
            "session",
            "generation",
            "attached",
            "focused",
            "layout",
            "panes",
            "backend",
            "backend_version",
            "test_backend",
            "rendering",
            "audio",
            "state",
            "tick",
            "state_hash",
            "halted",
            "lineage_parent",
            "lineage_origin_snapshot",
            "lineage_depth",
            "memory_base",
            "memory_length",
        ]
    );
    let values = pairs(&attached);
    assert_eq!(values[0], ("pane".to_owned(), "1".to_owned()));
    assert_eq!(values[1], ("session".to_owned(), "1".to_owned()));
    assert_eq!(values[2], ("generation".to_owned(), "1".to_owned()));
    assert_eq!(values[3], ("attached".to_owned(), "true".to_owned()));
    assert_eq!(values[4], ("focused".to_owned(), "true".to_owned()));
    assert_eq!(values[5], ("layout".to_owned(), "single".to_owned()));
    assert_eq!(values[6], ("panes".to_owned(), "1".to_owned()));
    assert_eq!(values[7], ("backend".to_owned(), BACKEND_ID.to_owned()));
    assert_eq!(values[9], ("test_backend".to_owned(), "true".to_owned()));
    assert_eq!(values[10], ("rendering".to_owned(), "false".to_owned()));
    assert_eq!(values[11], ("audio".to_owned(), "false".to_owned()));
    assert_eq!(values[12], ("state".to_owned(), "ready".to_owned()));
    assert_eq!(values[13], ("tick".to_owned(), "0".to_owned()));
    assert_eq!(values[14], ("state_hash".to_owned(), HASH.to_owned()));
    assert_eq!(values[15], ("halted".to_owned(), "false".to_owned()));
    assert_eq!(values[16], ("lineage_parent".to_owned(), "none".to_owned()));
    assert_eq!(values[18], ("lineage_depth".to_owned(), "0".to_owned()));
    assert_eq!(
        values[19],
        ("memory_base".to_owned(), MEMORY_BASE.to_string())
    );
    assert_eq!(values[20], ("memory_length".to_owned(), "4096".to_owned()));

    let detached = execute(&mut session, "detach");
    assert_eq!(
        names(&detached),
        [
            "pane",
            "session",
            "generation",
            "attached",
            "focused",
            "layout",
            "panes"
        ]
    );
    let values = pairs(&detached);
    assert_eq!(values[0], ("pane".to_owned(), "1".to_owned()));
    assert_eq!(values[1], ("session".to_owned(), "1".to_owned()));
    assert_eq!(values[2], ("generation".to_owned(), "1".to_owned()));
    assert_eq!(values[3], ("attached".to_owned(), "false".to_owned()));
    assert_eq!(values[4], ("focused".to_owned(), "none".to_owned()));
    assert_eq!(values[6], ("panes".to_owned(), "0".to_owned()));
}

#[test]
fn attach_is_idempotent_for_an_already_attached_session() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    let first = execute(&mut session, "attach 1");
    let calls = channel.call_count();
    let again = execute(&mut session, "attach 1");
    assert_eq!(
        channel.call_count(),
        calls,
        "an already attached session is only focused"
    );
    assert_eq!(pairs(&again)[0], pairs(&first)[0], "same pane");
    assert_eq!(
        pairs(&again)[2],
        pairs(&first)[2],
        "the generation is unchanged"
    );
    assert_eq!(session.model().attached(), 1);
}

#[test]
fn list_reports_the_sessions_in_ascending_order() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    let listed = execute(&mut session, "list");
    assert_eq!(
        names(&listed),
        [
            "sessions",
            "truncated",
            "session.1.state",
            "session.1.tick",
            "session.1.state_hash",
            "session.1.lineage_depth",
            "session.2.state",
            "session.2.tick",
            "session.2.state_hash",
            "session.2.lineage_depth",
        ]
    );
    let values = pairs(&listed);
    assert_eq!(values[0], ("sessions".to_owned(), "2".to_owned()));
    assert_eq!(values[1], ("truncated".to_owned(), "false".to_owned()));
    assert_eq!(
        values[2],
        ("session.1.state".to_owned(), "ready".to_owned())
    );
    assert_eq!(values[3], ("session.1.tick".to_owned(), "0".to_owned()));
    assert_eq!(values[7], ("session.2.tick".to_owned(), "40".to_owned()));
    assert_eq!(channel.ops(), ["capabilities", "session.list"]);
}

#[test]
fn step_and_run_blocks_carry_the_documented_fields() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");

    let stepped = execute(&mut session, "step");
    assert_eq!(stepped.record, Record::Step);
    assert_eq!(
        names(&stepped),
        [
            "pane",
            "session",
            "requested",
            "steps",
            "tick",
            "state_hash",
            "halted",
            "controller"
        ]
    );
    let values = pairs(&stepped);
    assert_eq!(values[2], ("requested".to_owned(), "1".to_owned()));
    assert_eq!(
        values[3],
        ("steps".to_owned(), "1".to_owned()),
        "default N=1"
    );
    assert_eq!(values[4], ("tick".to_owned(), "1".to_owned()));
    assert_eq!(values[7], ("controller".to_owned(), "human".to_owned()));

    let ran = execute(&mut session, "run 3");
    assert_eq!(ran.record, Record::Run);
    let values = pairs(&ran);
    assert_eq!(values[2], ("requested".to_owned(), "3".to_owned()));
    assert_eq!(values[3], ("steps".to_owned(), "3".to_owned()));
    assert_eq!(values[4], ("tick".to_owned(), "4".to_owned()));
    assert_eq!(
        channel.inputs("session.run").len(),
        2,
        "one run per control command"
    );
}

#[test]
fn a_step_above_the_service_bound_is_rejected_before_the_call() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    let calls = channel.call_count();
    let error = session
        .execute("run 101")
        .expect_err("above the service bound");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    assert_eq!(
        channel.call_count(),
        calls,
        "a rejected batch issues no operation"
    );
}

#[test]
fn pulse_issues_one_run_per_frame_and_reports_the_cadence() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = CommandSession::open(
        Arc::clone(&channel) as Arc<dyn OperationChannel>,
        Arc::new(ManualClock::new()),
        CommandConfig {
            pacer: PacerConfig::new(50, 4, 4).expect("cadence"),
            ..config()
        },
    )
    .expect("open the session");
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    let calls = channel.call_count();

    let pulsed = execute(&mut session, "pulse 3");
    assert_eq!(
        names(&pulsed),
        [
            "pane",
            "session",
            "frames",
            "frame",
            "tick",
            "state_hash",
            "halted",
            "ticks_per_frame",
            "fps",
            "elapsed_nanos",
            "controller"
        ]
    );
    let values = pairs(&pulsed);
    assert_eq!(values[2], ("frames".to_owned(), "3".to_owned()));
    assert_eq!(values[3], ("frame".to_owned(), "3".to_owned()));
    assert_eq!(values[4], ("tick".to_owned(), "12".to_owned()));
    assert_eq!(values[7], ("ticks_per_frame".to_owned(), "4".to_owned()));
    assert_eq!(values[8], ("fps".to_owned(), "50".to_owned()));
    assert_eq!(
        values[9],
        ("elapsed_nanos".to_owned(), (3 * 20_000_000).to_string())
    );
    assert_eq!(values[10], ("controller".to_owned(), "human".to_owned()));
    assert_eq!(
        channel.call_count(),
        calls + 3,
        "one session.run per presented frame"
    );
    assert_eq!(session.model().frame_index(), 3);
    let events: Vec<&str> = session
        .model()
        .events()
        .entries()
        .map(|event| event.kind().name())
        .collect();
    assert_eq!(
        events,
        ["create", "attach", "pulse"],
        "one pulse event per pulse, not per frame"
    );
}

#[test]
fn pause_gates_the_host_time_loop_but_not_pulse() {
    // fps 1 and two canonical steps per frame keep every number exact.
    let channel = Arc::new(RecordingChannel::new());
    let clock = Arc::new(ManualClock::new());
    let mut session = CommandSession::open(
        Arc::clone(&channel) as Arc<dyn OperationChannel>,
        clock.clone(),
        CommandConfig {
            pacer: PacerConfig::new(1, 2, 4).expect("cadence"),
            ..config()
        },
    )
    .expect("open the session");
    execute(&mut session, "create");
    execute(&mut session, "attach 1");

    clock.advance_nanos(3_000_000_000);
    let first = session.advance_elapsed().expect("three frames are due");
    assert_eq!((first.due(), first.advanced(), first.dropped()), (3, 3, 0));
    assert_eq!(session.model().frame_index(), 3);
    assert_eq!(focused_tick(&session), 6, "two canonical steps per frame");
    let runs = channel.inputs("session.run").len();
    assert_eq!(runs, 3, "one session.run per advanced frame");

    let paused = execute(&mut session, "pause");
    assert_eq!(pairs(&paused)[0], ("paused".to_owned(), "true".to_owned()));
    let events = session.model().events().len();
    clock.advance_nanos(5_000_000_000);
    let loop_report = session
        .advance_elapsed()
        .expect("a paused loop reports nothing due");
    assert_eq!(
        (
            loop_report.due(),
            loop_report.advanced(),
            loop_report.dropped(),
            loop_report.steps()
        ),
        (0, 0, 0, 0),
        "a paused window advances no frame and drops nothing"
    );
    assert_eq!(loop_report.elapsed_nanos(), first.elapsed_nanos());
    assert_eq!(
        session.model().frame_index(),
        3,
        "the frame index does not move while paused"
    );
    assert_eq!(
        focused_tick(&session),
        6,
        "no canonical step runs while paused"
    );
    assert_eq!(
        channel.inputs("session.run").len(),
        runs,
        "the paused loop issues no session.run"
    );
    assert_eq!(
        session.model().events().len(),
        events,
        "the paused loop records no pulse event"
    );

    // `pulse` is the explicit path: the presentation pause flag does not gate it.
    let pulsed = execute(&mut session, "pulse 1");
    assert_eq!(pairs(&pulsed)[2], ("frames".to_owned(), "1".to_owned()));
    assert_eq!(session.model().frame_index(), 4);
    assert_eq!(focused_tick(&session), 8);

    // Still paused: the host time keeps accumulating but the loop does not consume it.
    clock.advance_nanos(3_000_000_000);
    let still_paused = session.advance_elapsed().expect("still paused");
    assert_eq!(still_paused.advanced(), 0);
    assert_eq!(session.model().frame_index(), 4);

    // Unpausing: the frames due since the pacer's last step are capped by
    // `max_catch_up_frames` and the remainder is reported as dropped.
    execute(&mut session, "pause");
    let resumed = session
        .advance_elapsed()
        .expect("the resumed loop catches up");
    assert_eq!(resumed.due(), 7, "11 s of clock minus the pacer's 4 s");
    assert_eq!(resumed.advanced(), 4, "capped at max_catch_up_frames");
    assert_eq!(resumed.dropped(), 3, "the remainder is dropped, not queued");
    assert_eq!(session.model().frame_index(), 8);
    assert_eq!(focused_tick(&session), 16);
    assert_eq!(
        channel.inputs("session.run").len(),
        runs + 1 + 4,
        "one run for the pulse and four for the resumed catch-up"
    );
}

#[test]
fn frame_and_html_render_the_same_model_without_reaching_the_channel() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    let calls = channel.call_count();

    let frame = grid(session.execute("frame").expect("frame"));
    let rows: Vec<&str> = frame.lines().collect();
    assert_eq!(rows.len(), 24);
    assert!(rows.iter().all(|row| row.len() == 80));
    assert!(rows[0].starts_with("chronofork gui layout=single controller=human frame=0 fps=60"));

    let (document, block) = html(session.execute("html").expect("html"));
    assert!(document.starts_with("<!DOCTYPE html>\n"));
    assert!(document.contains("<pre class=\"frame\" id=\"frame-1\">"));
    assert_eq!(
        names(&block),
        ["bytes", "sha256"],
        "the html block describes the exact document"
    );
    let values = pairs(&block);
    assert_eq!(values[0], ("bytes".to_owned(), document.len().to_string()));
    assert_eq!(
        values[1],
        (
            "sha256".to_owned(),
            hash_bytes(document.as_bytes()).to_hex()
        )
    );
    assert_eq!(channel.call_count(), calls, "rendering issues no operation");

    // The same model renders the same document twice.
    let (again, block_again) = html(session.execute("html").expect("html"));
    assert_eq!(document, again);
    assert_eq!(block, block_again);
}

#[test]
fn an_empty_or_whitespace_line_is_not_a_command() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    for line in ["", "   ", "\t ", "\r"] {
        assert_eq!(
            session.execute(line).expect("ignored"),
            None,
            "line {line:?}"
        );
    }
    assert_eq!(channel.ops(), ["capabilities"]);
}

#[test]
fn the_quit_block_carries_the_reason_and_the_lease() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    let block = execute(&mut session, "quit");
    assert_eq!(block.record, Record::Quit);
    assert_eq!(
        names(&block),
        ["reason", "controller", "generation", "sequence"]
    );
    let values = pairs(&block);
    assert_eq!(values[0], ("reason".to_owned(), "line".to_owned()));
    assert_eq!(values[1], ("controller".to_owned(), "human".to_owned()));
    assert_eq!(values[2], ("generation".to_owned(), "0".to_owned()));
    assert_eq!(values[3], ("sequence".to_owned(), "0".to_owned()));

    let eof = session.quit(QuitReason::Eof);
    assert_eq!(eof.record, Record::Quit);
    assert_eq!(pairs(&eof)[0], ("reason".to_owned(), "eof".to_owned()));
}

#[test]
fn the_panel_block_reports_availability_and_its_reason() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = open(&channel);
    let memory = execute(&mut session, "panel memory on");
    assert_eq!(names(&memory), ["panel", "visible", "available", "reason"]);
    let values = pairs(&memory);
    assert_eq!(values[0], ("panel".to_owned(), "memory".to_owned()));
    assert_eq!(values[1], ("visible".to_owned(), "true".to_owned()));
    assert_eq!(
        values[2],
        ("available".to_owned(), "false".to_owned()),
        "no pane is attached yet"
    );
    assert_eq!(values[3], ("reason".to_owned(), "no_session".to_owned()));

    let timeline = execute(&mut session, "panel timeline on");
    assert_eq!(
        pairs(&timeline)[3],
        ("reason".to_owned(), "no_movie".to_owned())
    );

    let events = execute(&mut session, "panel events off");
    assert_eq!(
        pairs(&events)[2],
        ("available".to_owned(), "true".to_owned())
    );
    assert!(!session.model().panel(PanelKind::EventLog).visible());
}
