// Anti-slop policy §10 exception: owner runtime-engineer; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It mirrors
// the existing headers in crates/chronofork-gui/tests/render.rs; no production code is
// affected.
#![allow(clippy::expect_used)]

//! Controller-lease acceptance tests for the M7 GUI (row M7-A16).
//!
//! The lease is exercised through [`CommandSession`] against a recording channel, so the
//! row's claims are assertions:
//!
//! - the initial state is `human` with generation 0 and sequence 0;
//! - `handoff agent` reports `changed=true` and increments both counters, and handing off to
//!   the current controller is a no-op with unchanged counters;
//! - `step`/`run`/`pulse` under `Agent` return the typed `control_denied` failure carrying
//!   the current generation and sequence, issue **zero** channel operations, and leave the
//!   model unchanged;
//! - observation commands never consult the lease, and `handoff` is always allowed;
//! - every effective transition and every denial is recorded in the presentation event log
//!   with monotonic sequences, and no host time is recorded anywhere.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chronofork_gui::command::{Block, BlockField, CommandConfig, CommandOutcome, CommandSession};
use chronofork_gui::control::{ControlError, Controller, Lease};
use chronofork_gui::pacing::{ManualClock, PacerConfig};
use chronofork_gui::render::RenderOptions;
use chronofork_gui::{ChannelFault, GuiError, GuiErrorCode, OperationChannel};
use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};

const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";
const BACKEND_ID: &str = "chronofork.test.synthetic";

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn envelope(op: &str, result: JsonValue) -> Vec<u8> {
    let document = JsonValue::object([("op", JsonValue::string(op)), ("result", result)]);
    to_canonical_bytes(&document, &limits()).expect("canonical bytes")
}

fn int(value: u64) -> JsonValue {
    JsonValue::int(i64::try_from(value).expect("canonical integer"))
}

fn capability_flags() -> BTreeMap<String, JsonValue> {
    [
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
    ]
    .into_iter()
    .map(|name| {
        (
            name.to_owned(),
            JsonValue::from_bool(name != "rendering" && name != "audio"),
        )
    })
    .collect()
}

fn capabilities_result() -> JsonValue {
    let flags = capability_flags();
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
                ("max_steps_per_request", int(100)),
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

fn inspect_result(session: u64, tick: u64) -> JsonValue {
    JsonValue::object([
        ("session", int(session)),
        ("state", JsonValue::string("ready")),
        (
            "lineage",
            JsonValue::object([
                ("parent_session", JsonValue::Null),
                ("origin_snapshot", JsonValue::Null),
                ("depth", int(0)),
            ]),
        ),
        (
            "backend",
            JsonValue::object([
                ("id", JsonValue::string(BACKEND_ID)),
                ("display_name", JsonValue::string("Synthetic")),
                ("version", JsonValue::string("0.1.0")),
                ("test_backend", JsonValue::from_bool(true)),
                ("capabilities", JsonValue::Object(capability_flags())),
            ]),
        ),
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
                ("memory_base", int(0)),
                ("memory_length", int(0)),
            ]),
        ),
    ])
}

/// A channel that records operation names and answers the four operations the lease needs.
struct RecordingChannel {
    state: Mutex<Recording>,
}

#[derive(Debug)]
struct Recording {
    calls: Vec<String>,
    tick: u64,
    next_session: u64,
}

impl RecordingChannel {
    fn new() -> Self {
        Self {
            state: Mutex::new(Recording {
                calls: Vec::new(),
                tick: 0,
                next_session: 1,
            }),
        }
    }

    fn ops(&self) -> Vec<String> {
        self.state.lock().expect("call log").calls.clone()
    }

    fn call_count(&self) -> usize {
        self.state.lock().expect("call log").calls.len()
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
        let mut state = self.state.lock().expect("call log");
        state.calls.push(op.clone());
        match op.as_str() {
            "capabilities" => Ok(envelope("capabilities", capabilities_result())),
            "session.list" => Ok(envelope(
                "session.list",
                JsonValue::object([
                    ("sessions", JsonValue::Array(Vec::new())),
                    ("truncated", JsonValue::from_bool(false)),
                ]),
            )),
            "session.create" => {
                let session = state.next_session;
                state.next_session = session.saturating_add(1);
                Ok(envelope(
                    "session.create",
                    JsonValue::object([
                        ("session", int(session)),
                        ("backend_id", JsonValue::string(BACKEND_ID)),
                        ("state", JsonValue::string("ready")),
                        ("tick", int(0)),
                        ("state_hash", JsonValue::string(HASH)),
                        (
                            "lineage",
                            JsonValue::object([
                                ("parent_session", JsonValue::Null),
                                ("origin_snapshot", JsonValue::Null),
                                ("depth", int(0)),
                            ]),
                        ),
                    ]),
                ))
            }
            "session.inspect" => {
                let session = input
                    .get("session")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("inspect without a session"))?;
                Ok(envelope(
                    "session.inspect",
                    inspect_result(session, state.tick),
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
                let tick = state.tick;
                Ok(envelope(
                    "session.run",
                    JsonValue::object([
                        ("session", int(session)),
                        ("steps", int(steps)),
                        ("tick", int(tick)),
                        ("state_hash", JsonValue::string(HASH)),
                        ("halted", JsonValue::from_bool(false)),
                    ]),
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

fn block(outcome: Option<CommandOutcome>) -> Block {
    as_block(outcome).expect("a block outcome")
}

fn grid(outcome: Option<CommandOutcome>) -> String {
    as_grid(outcome).expect("a frame grid outcome")
}

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

fn execute(session: &mut CommandSession, line: &str) -> Block {
    block(session.execute(line).expect("the line executes"))
}

/// Opens a window with one created and attached session, ready for control commands.
fn attached(channel: &Arc<RecordingChannel>) -> CommandSession {
    let mut session = open(channel);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    session
}

#[test]
fn a_new_window_starts_under_human_control() {
    let channel = Arc::new(RecordingChannel::new());
    let session = open(&channel);
    assert_eq!(session.lease(), Lease::new());
    assert_eq!(session.lease().controller(), Controller::Human);
    assert_eq!(session.lease().generation(), 0);
    assert_eq!(session.lease().sequence(), 0);
    assert_eq!(session.model().events().len(), 0, "no transition yet");
}

#[test]
fn handoff_agent_transfers_the_lease_and_reports_the_transition() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = attached(&channel);
    let handoff = execute(&mut session, "handoff agent");
    assert_eq!(
        pairs(&handoff),
        [
            ("controller".to_owned(), "agent".to_owned()),
            ("previous".to_owned(), "human".to_owned()),
            ("changed".to_owned(), "true".to_owned()),
            ("generation".to_owned(), "1".to_owned()),
            ("sequence".to_owned(), "1".to_owned()),
        ]
    );
    assert_eq!(session.lease().controller(), Controller::Agent);
    assert_eq!(session.lease().generation(), 1);
    assert_eq!(session.lease().sequence(), 1);
    assert_eq!(
        channel.ops(),
        ["capabilities", "session.create", "session.inspect"],
        "handoff itself issues no operation"
    );

    let back = execute(&mut session, "handoff human");
    assert_eq!(
        pairs(&back)[0],
        ("controller".to_owned(), "human".to_owned())
    );
    assert_eq!(pairs(&back)[1], ("previous".to_owned(), "agent".to_owned()));
    assert_eq!(pairs(&back)[2], ("changed".to_owned(), "true".to_owned()));
    assert_eq!(pairs(&back)[3], ("generation".to_owned(), "2".to_owned()));
    assert_eq!(pairs(&back)[4], ("sequence".to_owned(), "2".to_owned()));
    assert_eq!(session.lease().controller(), Controller::Human);
}

#[test]
fn handing_off_to_the_current_controller_is_a_no_op() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = attached(&channel);
    let events = session.model().events().len();

    let repeated = execute(&mut session, "handoff human");
    let values = pairs(&repeated);
    assert_eq!(values[2], ("changed".to_owned(), "false".to_owned()));
    assert_eq!(values[3], ("generation".to_owned(), "0".to_owned()));
    assert_eq!(values[4], ("sequence".to_owned(), "0".to_owned()));
    assert_eq!(session.lease(), Lease::new());
    assert_eq!(
        session.model().events().len(),
        events,
        "an ineffective handoff records no transition event"
    );

    execute(&mut session, "handoff agent");
    let events = session.model().events().len();
    let repeated = execute(&mut session, "handoff agent");
    let values = pairs(&repeated);
    assert_eq!(values[2], ("changed".to_owned(), "false".to_owned()));
    assert_eq!(values[3], ("generation".to_owned(), "1".to_owned()));
    assert_eq!(values[4], ("sequence".to_owned(), "1".to_owned()));
    assert_eq!(session.model().events().len(), events);
}

#[test]
fn control_commands_are_denied_under_agent_with_the_lease_counters() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = attached(&channel);
    execute(&mut session, "handoff agent");
    // The presentation log must grow with the denials; hide it so the comparison is about
    // the session the panes hold.
    execute(&mut session, "panel events off");
    let frame_before = grid(session.execute("frame").expect("frame"));
    let calls = channel.call_count();
    let frame_index = session.model().frame_index();

    for line in ["step", "run 2", "pulse 1"] {
        let error = session.execute(line).expect_err("the agent holds control");
        assert_eq!(error.code(), GuiErrorCode::ControlDenied, "line {line}");
        assert!(
            matches!(
                &error,
                GuiError::ControlDenied {
                    generation: 1,
                    sequence: 1
                }
            ),
            "the denial carries the current lease counters: {error}"
        );
        assert!(error.to_string().starts_with("control_denied: "));
    }

    assert_eq!(
        channel.call_count(),
        calls,
        "a denied control command performs no operation"
    );
    assert_eq!(session.model().frame_index(), frame_index);
    let frame_after = grid(session.execute("frame").expect("frame"));
    assert_eq!(
        frame_before, frame_after,
        "the presented session state is unchanged"
    );

    let loop_error = session
        .advance_elapsed()
        .expect_err("the host-time loop is a control action");
    assert!(matches!(
        &loop_error,
        GuiError::ControlDenied {
            generation: 1,
            sequence: 1
        }
    ));
}

#[test]
fn handoff_back_to_human_restores_control() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = attached(&channel);
    execute(&mut session, "handoff agent");
    session.execute("step").expect_err("denied");
    execute(&mut session, "handoff human");
    let stepped = execute(&mut session, "step");
    assert_eq!(pairs(&stepped)[3], ("steps".to_owned(), "1".to_owned()));
    assert_eq!(pairs(&stepped)[4], ("tick".to_owned(), "1".to_owned()));
    assert_eq!(
        pairs(&stepped)[7],
        ("controller".to_owned(), "human".to_owned())
    );
}

#[test]
fn observation_commands_never_consult_the_lease() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = attached(&channel);
    execute(&mut session, "handoff agent");

    for line in [
        "frame",
        "html",
        "panel memory on",
        "layout mosaic",
        "focus 1",
        "pause",
        "create",
        "list",
        "attach 2",
        "detach",
        "quit",
    ] {
        session
            .execute(line)
            .expect("an observation command executes under the agent lease");
    }
}

#[test]
fn every_transition_and_denial_is_recorded_with_monotonic_sequences() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = attached(&channel);
    execute(&mut session, "handoff agent");
    session.execute("step").expect_err("denied");
    session.execute("run 1").expect_err("denied");
    session.execute("pulse 1").expect_err("denied");
    execute(&mut session, "handoff human");
    execute(&mut session, "step");

    let kinds: Vec<&str> = session
        .model()
        .events()
        .entries()
        .map(|event| event.kind().name())
        .collect();
    assert_eq!(
        kinds,
        [
            "create", "attach", "handoff", "step", "step", "pulse", "handoff", "step"
        ],
        "every transition and every denial is recorded"
    );
    let sequences: Vec<u64> = session
        .model()
        .events()
        .entries()
        .map(|event| event.sequence())
        .collect();
    assert_eq!(sequences, [0, 1, 2, 3, 4, 5, 6, 7], "monotonic sequences");
}

#[test]
fn the_denial_reports_the_generation_and_sequence_of_the_lease() {
    let channel = Arc::new(RecordingChannel::new());
    let mut session = attached(&channel);
    for line in ["handoff agent", "handoff human", "handoff agent"] {
        execute(&mut session, line);
    }
    assert_eq!(session.lease().generation(), 3);
    assert_eq!(session.lease().sequence(), 3);

    let error = session.execute("step").expect_err("denied");
    assert_eq!(error.code(), GuiErrorCode::ControlDenied);
    let denied = ControlError::Denied {
        generation: 3,
        sequence: 3,
    };
    assert_eq!(GuiError::from(denied), error);
    assert_eq!(denied.generation(), 3);
    assert_eq!(denied.sequence(), 3);
    assert_eq!(
        denied.to_string(),
        "the agent holds control (generation 3, sequence 3)"
    );
}
