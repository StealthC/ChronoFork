// Anti-slop policy §10 exception: owner runtime-engineer; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It mirrors
// the existing headers in crates/chronofork-gui/tests/render.rs; no production code is
// affected.
#![allow(clippy::expect_used)]

//! Pacing and run-loop acceptance tests for the M7 GUI (row M7-A14).
//!
//! The cadence is exercised at both layers: directly through [`Pacer`] with a counting frame
//! hook, and through [`CommandSession`] with an injected [`ManualClock`], so the row's claims
//! are assertions:
//!
//! - `pulse(n)` advances exactly `n` frames and `n * ticks_per_frame` canonical steps, moves
//!   the cumulative time by exactly `n * (1_000_000_000 / fps)` nanos, reports `dropped=0`
//!   and never advances a partial frame;
//! - `advance_elapsed` at chosen instants advances `due` frames capped at
//!   `max_catch_up_frames` and reports the remainder as dropped, and a clock that does not
//!   move advances zero frames;
//! - `SystemClock::now_nanos` is monotonic non-decreasing;
//! - no host time value reaches the model, a block or a frame: two windows opened at
//!   different clock instants emit identical bytes for the same script.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chronofork_gui::command::{Block, BlockField, CommandConfig, CommandOutcome, CommandSession};
use chronofork_gui::pacing::{HostClock, ManualClock, Pacer, PacerConfig, SystemClock};
use chronofork_gui::render::RenderOptions;
use chronofork_gui::{ChannelFault, GuiErrorCode, OperationChannel};
use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};

const HASH: &str = "c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8";
const BACKEND_ID: &str = "chronofork.test.synthetic";
const NANOS_PER_SECOND: u64 = 1_000_000_000;

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

fn capabilities_result() -> JsonValue {
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
                ("max_steps_per_request", int(1000)),
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

fn inspect_result(session: u64, tick: u64) -> JsonValue {
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
        ("session", int(session)),
        ("state", JsonValue::string("ready")),
        ("lineage", lineage()),
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

/// A channel that answers `capabilities`, `session.create`, `session.inspect` and
/// `session.run`, and counts the runs it performed.
struct CountingChannel {
    runs: Mutex<u64>,
    tick: Mutex<u64>,
}

impl CountingChannel {
    fn new() -> Self {
        Self {
            runs: Mutex::new(0),
            tick: Mutex::new(0),
        }
    }

    fn runs(&self) -> u64 {
        *self.runs.lock().expect("run counter")
    }

    fn tick(&self) -> u64 {
        *self.tick.lock().expect("tick")
    }
}

impl OperationChannel for CountingChannel {
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
        match op.as_str() {
            "capabilities" => Ok(envelope("capabilities", capabilities_result())),
            "session.create" => Ok(envelope(
                "session.create",
                JsonValue::object([
                    ("session", int(1)),
                    ("backend_id", JsonValue::string(BACKEND_ID)),
                    ("state", JsonValue::string("ready")),
                    ("tick", int(0)),
                    ("state_hash", JsonValue::string(HASH)),
                    ("lineage", lineage()),
                ]),
            )),
            "session.inspect" => {
                let session = input
                    .get("session")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| ChannelFault::new("inspect without a session"))?;
                Ok(envelope(
                    "session.inspect",
                    inspect_result(session, self.tick()),
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
                *self.runs.lock().expect("run counter") += 1;
                let mut tick = self.tick.lock().expect("tick");
                *tick = tick.saturating_add(steps);
                Ok(envelope(
                    "session.run",
                    JsonValue::object([
                        ("session", int(session)),
                        ("steps", int(steps)),
                        ("tick", int(*tick)),
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

fn config(ticks_per_frame: u64) -> CommandConfig {
    CommandConfig {
        width: 80,
        height: 24,
        render: RenderOptions::plain(),
        pacer: PacerConfig::new(60, ticks_per_frame, 4).expect("cadence"),
        timeline: None,
    }
}

fn open(channel: &Arc<CountingChannel>, clock: Arc<ManualClock>, ticks: u64) -> CommandSession {
    CommandSession::open(
        Arc::clone(channel) as Arc<dyn OperationChannel>,
        clock,
        config(ticks),
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
    as_block(session.execute(line).expect("the line executes")).expect("a block outcome")
}

fn frame(session: &mut CommandSession) -> String {
    as_grid(session.execute("frame").expect("frame")).expect("a frame grid")
}

#[test]
fn the_frame_length_follows_the_documented_formula() {
    for fps in [1_u32, 2, 30, 50, 60, 240] {
        let cadence = PacerConfig::new(fps, 1, 4).expect("cadence");
        assert_eq!(
            cadence.frame_ns(),
            NANOS_PER_SECOND / u64::from(fps),
            "fps {fps}"
        );
        assert_eq!(Pacer::new(cadence).frame_ns(), cadence.frame_ns());
    }
    assert_eq!(
        PacerConfig::new(60, 1, 4).expect("cadence").frame_ns(),
        16_666_666,
        "integer division, not rounding"
    );
}

#[test]
fn pulse_advances_exactly_the_requested_frames_and_steps() {
    let cadence = PacerConfig::new(60, 7, 4).expect("cadence");
    let mut pacer = Pacer::new(cadence);
    let mut calls = 0_u64;
    let mut steps = 0_u64;
    let report = pacer
        .pulse(5, |per_frame| {
            calls += 1;
            steps += per_frame;
            Ok(())
        })
        .expect("pulse");
    assert_eq!(calls, 5, "one frame hook per frame, never a partial frame");
    assert_eq!(steps, 35);
    assert_eq!(report.frames(), 5);
    assert_eq!(report.steps(), 35);
    assert_eq!(report.dropped(), 0, "the explicit path never drops");
    assert_eq!(
        report.elapsed_nanos(),
        5 * cadence.frame_ns(),
        "the manual time moves by exactly frames * frame_ns"
    );
    assert_eq!(pacer.last_nanos(), report.elapsed_nanos());
}

#[test]
fn advance_elapsed_caps_the_catch_up_and_reports_the_dropped_frames() {
    let clock = ManualClock::new();
    let cadence = PacerConfig::new(1, 1, 4).expect("cadence");
    let mut pacer = Pacer::new(cadence);
    let mut calls = 0_u64;

    clock.advance_nanos(3 * NANOS_PER_SECOND);
    let report = pacer
        .advance_elapsed(clock.now_nanos(), |_| {
            calls += 1;
            Ok(())
        })
        .expect("three frames are due");
    assert_eq!(
        (report.due(), report.advanced(), report.dropped()),
        (3, 3, 0)
    );
    assert_eq!(calls, 3);
    assert_eq!(report.elapsed_nanos(), 3 * NANOS_PER_SECOND);

    clock.advance_nanos(7 * NANOS_PER_SECOND);
    let report = pacer
        .advance_elapsed(clock.now_nanos(), |_| Ok(()))
        .expect("late step");
    assert_eq!(report.due(), 7);
    assert_eq!(report.advanced(), 4, "capped by max_catch_up_frames");
    assert_eq!(report.dropped(), 3, "the remainder is dropped");
    assert_eq!(report.elapsed_nanos(), 7 * NANOS_PER_SECOND);
    assert_eq!(report.steps(), 4);
}

#[test]
fn a_clock_that_does_not_move_advances_zero_frames() {
    let clock = ManualClock::new();
    let mut pacer = Pacer::new(PacerConfig::new(60, 1, 4).expect("cadence"));
    pacer.pulse(2, |_| Ok(())).expect("pulse");
    let frozen = pacer.last_nanos();
    for _ in 0..3 {
        let report = pacer
            .advance_elapsed(clock.now_nanos(), |_| Ok(()))
            .expect("no step");
        assert_eq!(report.due(), 0);
        assert_eq!(report.advanced(), 0);
        assert_eq!(report.elapsed_nanos(), frozen);
    }
}

#[test]
fn the_system_clock_is_monotonic_across_calls() {
    let clock = SystemClock::new();
    let mut previous = clock.now_nanos();
    for _ in 0..64 {
        let current = clock.now_nanos();
        assert!(current >= previous, "the host clock never goes backwards");
        previous = current;
    }
    assert!(
        HostClock::now_nanos(&SystemClock::default()) < NANOS_PER_SECOND * 3600,
        "the origin is construction, not an absolute epoch"
    );
}

#[test]
fn the_session_run_loop_advances_the_frames_the_clock_says_are_due() {
    // fps 1 and one canonical step per frame keep the arithmetic exact.
    let channel = Arc::new(CountingChannel::new());
    let clock = Arc::new(ManualClock::new());
    let mut session = CommandSession::open(
        Arc::clone(&channel) as Arc<dyn OperationChannel>,
        clock.clone(),
        CommandConfig {
            pacer: PacerConfig::new(1, 2, 4).expect("cadence"),
            ..config(1)
        },
    )
    .expect("open the session");
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    let runs = channel.runs();

    clock.advance_nanos(3 * NANOS_PER_SECOND);
    let report = session.advance_elapsed().expect("three frames are due");
    assert_eq!(
        (report.due(), report.advanced(), report.dropped()),
        (3, 3, 0)
    );
    assert_eq!(channel.runs(), runs + 3, "one session.run per due frame");
    assert_eq!(session.model().frame_index(), 3);
    assert_eq!(channel.tick(), 6, "two canonical steps per frame");
    let events: Vec<&str> = session
        .model()
        .events()
        .entries()
        .map(|event| event.kind().name())
        .collect();
    assert_eq!(
        events,
        ["create", "attach", "pulse"],
        "one pulse per loop step"
    );

    clock.advance_nanos(20 * NANOS_PER_SECOND);
    let report = session.advance_elapsed().expect("a late step");
    assert_eq!(report.due(), 20);
    assert_eq!(report.advanced(), 4, "a slow host cannot spiral");
    assert_eq!(report.dropped(), 16);
    assert_eq!(session.model().frame_index(), 7);
}

#[test]
fn no_host_time_value_reaches_the_model_a_block_or_a_frame() {
    let early = Arc::new(CountingChannel::new());
    let early_clock = Arc::new(ManualClock::new());
    let mut early_session = open(&early, early_clock.clone(), 2);

    let late = Arc::new(CountingChannel::new());
    let late_clock = Arc::new(ManualClock::at(786_240_000_000_000));
    let mut late_session = open(&late, late_clock.clone(), 2);

    let script = ["create", "attach 1", "step 2", "pulse 3", "pause", "frame"];
    let mut early_blocks = Vec::new();
    let mut late_blocks = Vec::new();
    let mut early_frame = String::new();
    let mut late_frame = String::new();
    for line in script {
        if line == "frame" {
            early_frame = frame(&mut early_session);
            late_frame = frame(&mut late_session);
            continue;
        }
        early_blocks.push(execute(&mut early_session, line));
        late_blocks.push(execute(&mut late_session, line));
    }
    assert_eq!(
        early_blocks, late_blocks,
        "the same script emits the same blocks at any clock instant"
    );
    assert_eq!(early_frame, late_frame, "the frame carries no host time");
    let pulsed = pairs(&early_blocks[3]);
    assert_eq!(
        pulsed[9],
        ("elapsed_nanos".to_owned(), (3 * 16_666_666).to_string()),
        "the pulse time is the pacer's own accounting, not the clock reading"
    );
    for clock in [&early_clock, &late_clock] {
        clock.advance_nanos(NANOS_PER_SECOND);
    }
    // Clock movement after the fact changes nothing that was emitted.
    assert_eq!(frame(&mut early_session), early_frame);
    assert_eq!(frame(&mut late_session), late_frame);
}

#[test]
fn a_pulse_of_zero_frames_reports_no_advance_and_issues_no_run() {
    let channel = Arc::new(CountingChannel::new());
    let clock = Arc::new(ManualClock::new());
    let mut session = open(&channel, clock, 1);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    let runs = channel.runs();
    let pulsed = execute(&mut session, "pulse 0");
    assert_eq!(channel.runs(), runs, "no frame is advanced");
    assert_eq!(pairs(&pulsed)[2], ("frames".to_owned(), "0".to_owned()));
    assert_eq!(pairs(&pulsed)[3], ("frame".to_owned(), "0".to_owned()));
    assert_eq!(
        pairs(&pulsed)[9],
        ("elapsed_nanos".to_owned(), "0".to_owned())
    );
}

#[test]
fn a_pulse_above_the_documented_bound_is_rejected_before_any_run() {
    let channel = Arc::new(CountingChannel::new());
    let clock = Arc::new(ManualClock::new());
    let mut session = open(&channel, clock, 1);
    execute(&mut session, "create");
    execute(&mut session, "attach 1");
    let runs = channel.runs();
    let error = session.execute("pulse 601").expect_err("above the bound");
    assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
    assert_eq!(channel.runs(), runs, "a rejected pulse advances nothing");
    assert_eq!(session.model().frame_index(), 0);
}
