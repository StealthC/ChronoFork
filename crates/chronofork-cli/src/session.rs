//! `chronofork session save|load|list|delete` as clients of the shared service.
//!
//! Each subcommand dispatches the matching `store.*` operation through an in-process
//! [`AppService`] configured with the requested root,
//! using the locally constructed administrator principal documented in
//! [`crate::agent::local_admin`]. The CLI owns no storage orchestration: serialization,
//! validation, metadata and locking stay in `chronofork-sessions`, and the operation
//! semantics, ownership and error taxonomy stay in `chronofork-service`.
//!
//! Two output forms share those results:
//!
//! - the default `key=value` record blocks are the documented human format and are
//!   unchanged;
//! - `--json` prints the exact canonical service envelope bytes plus one newline, the
//!   same bytes the HTTP surface would return for the matching operation.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use chronofork_service::AppService;
use chronofork_service::json::JsonValue;

use crate::agent::{
    dispatch_local, json_optional_string, json_unsigned, local_service, result_of,
    write_canonical_line,
};
use crate::{CliError, begin_record, option_value, parse_key, parse_u32, parse_u64, set_once};

/// Runs one `chronofork session` subcommand.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("save") => save(&args[1..]),
        Some("load") => load(&args[1..]),
        Some("list") => list(&args[1..]),
        Some("delete") => delete(&args[1..]),
        None => Err(CliError(
            "`session` requires a subcommand: save, load, list or delete".to_owned(),
        )
        .into()),
        Some(other) => Err(CliError(format!(
            "unknown `session` subcommand `{other}`; expected save, load, list or delete"
        ))
        .into()),
    }
}

/// Optional flags accepted by one `session` subcommand.
#[derive(Clone, Copy, Debug)]
struct SessionFlags {
    session: bool,
    snapshot: bool,
    name: bool,
    steps: bool,
    input: bool,
}

impl SessionFlags {
    const SAVE: Self = Self {
        session: true,
        snapshot: false,
        name: true,
        steps: true,
        input: true,
    };
    const LOAD: Self = Self {
        session: true,
        snapshot: true,
        name: false,
        steps: true,
        input: true,
    };
    const LIST: Self = Self {
        session: true,
        snapshot: false,
        name: false,
        steps: false,
        input: false,
    };
    const DELETE: Self = Self {
        session: true,
        snapshot: true,
        name: false,
        steps: false,
        input: false,
    };

    /// Flag list used in an unknown-option diagnostic.
    fn describe(self) -> String {
        let mut flags = vec!["--root <DIR>"];
        if self.session {
            flags.push("--session <KEY>");
        }
        if self.snapshot {
            flags.push("--snapshot <KEY>");
        }
        if self.name {
            flags.push("--name <NAME>");
        }
        if self.steps {
            flags.push("--steps <N>");
        }
        if self.input {
            flags.push("--input <U32>");
        }
        flags.push("--json");
        flags.join(" ")
    }
}

/// Parsed flags shared by the `session` subcommands.
#[derive(Debug, Default)]
struct SessionArgs {
    root: Option<PathBuf>,
    session: Option<u64>,
    snapshot: Option<u64>,
    name: Option<String>,
    steps: Option<u64>,
    input: Option<u32>,
    json: bool,
}

fn parse_session_args(
    subcommand: &str,
    args: &[String],
    flags: SessionFlags,
) -> Result<SessionArgs, CliError> {
    let mut parsed = SessionArgs::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--root" => set_once(
                &mut parsed.root,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--session" if flags.session => set_once(
                &mut parsed.session,
                parse_key(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--snapshot" if flags.snapshot => set_once(
                &mut parsed.snapshot,
                parse_key(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--name" if flags.name => set_once(
                &mut parsed.name,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--steps" if flags.steps => set_once(
                &mut parsed.steps,
                parse_u64(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--input" if flags.input => set_once(
                &mut parsed.input,
                parse_u32(option_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--json" => {
                if parsed.json {
                    return Err(CliError("`--json` was given more than once".to_owned()));
                }
                parsed.json = true;
            }
            other => {
                return Err(CliError(format!(
                    "unknown `session {subcommand}` option `{other}`; expected {}",
                    flags.describe()
                )));
            }
        }
        index += 1;
    }
    Ok(parsed)
}

/// Opens the in-process service over one durable root.
fn service(root: &Path) -> Result<AppService, Box<dyn Error>> {
    local_service(Some(root.to_owned()), None)
}

/// `session save` input: create a durable session, or continue one and append a snapshot.
fn save_input(options: &SessionArgs) -> Result<JsonValue, CliError> {
    let mut members = vec![
        (
            "steps",
            json_unsigned(options.steps.unwrap_or(0), "--steps")?,
        ),
        (
            "buttons",
            JsonValue::int(i64::from(options.input.unwrap_or(0))),
        ),
        ("name", json_optional_string(options.name.as_deref())),
    ];
    if let Some(session) = options.session {
        members.push(("session_key", json_unsigned(session, "--session")?));
    }
    Ok(JsonValue::object(members))
}

fn save(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("save", args, SessionFlags::SAVE)?;
    let root = options
        .root
        .as_deref()
        .ok_or_else(|| CliError("`session save` requires `--root <DIR>`".to_owned()))?;
    let service = service(root)?;
    let response = dispatch_local(&service, "store.save", &save_input(&options)?)?;
    if options.json {
        return write_canonical_line(&response);
    }

    let result = result_of(&response)?;
    let session_key = required_u64(&result, "session_key")?;
    // The operation result names the new snapshot, not the durable session it belongs
    // to; the documented human block shows both, so the session record is read back
    // through the same service (there is no storage access in the CLI).
    let session_name = durable_session_name(&service, session_key)?;
    let mut output = String::new();
    write_session_block(&mut output, session_key, &session_name);
    write_snapshot_result(&mut output, &result)?;
    print!("{output}");
    Ok(())
}

fn load(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("load", args, SessionFlags::LOAD)?;
    let root = options
        .root
        .as_deref()
        .ok_or_else(|| CliError("`session load` requires `--root <DIR>`".to_owned()))?;
    let session_key = options
        .session
        .ok_or_else(|| CliError("`session load` requires `--session <KEY>`".to_owned()))?;
    let input = JsonValue::object([
        ("session_key", json_unsigned(session_key, "--session")?),
        (
            "snapshot_key",
            optional_key_json(options.snapshot, "--snapshot")?,
        ),
        (
            "steps",
            json_unsigned(options.steps.unwrap_or(0), "--steps")?,
        ),
        (
            "buttons",
            JsonValue::int(i64::from(options.input.unwrap_or(0))),
        ),
    ]);
    let service = service(root)?;
    let response = dispatch_local(&service, "store.load", &input)?;
    if options.json {
        return write_canonical_line(&response);
    }
    let result = result_of(&response)?;
    let mut output = String::new();
    write_snapshot_result(&mut output, &result)?;
    print!("{output}");
    Ok(())
}

fn list(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("list", args, SessionFlags::LIST)?;
    let root = options
        .root
        .as_deref()
        .ok_or_else(|| CliError("`session list` requires `--root <DIR>`".to_owned()))?;
    let input = JsonValue::object([(
        "session_key",
        optional_key_json(options.session, "--session")?,
    )]);
    let service = service(root)?;
    let response = dispatch_local(&service, "store.list", &input)?;
    if options.json {
        return write_canonical_line(&response);
    }

    let result = result_of(&response)?;
    let sessions = result
        .get("sessions")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| CliError("service envelope has no `sessions`".to_owned()))?;
    let mut output = String::new();
    for session in sessions {
        let session_key = required_u64(session, "session_key")?;
        let name = nullable_str(session, "name")?;
        write_session_block(&mut output, session_key, name);
        let parent = optional_u64(session, "parent_session_key")?;
        let origin = optional_u64(session, "origin_snapshot_key")?;
        let snapshots = session
            .get("snapshots")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| CliError("service envelope has no `snapshots`".to_owned()))?;
        for snapshot in snapshots {
            write_snapshot_entry(&mut output, session_key, snapshot, parent, origin)?;
        }
    }
    print!("{output}");
    write_anomalies(&result)?;
    // A truncated listing still succeeds, but the human surface must not hide the
    // omission: one deterministic stderr notice names the entry budget. `--json`
    // already carries `truncated` in the canonical envelope.
    if result
        .get("truncated")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        eprintln!(
            "chronofork: storage listing truncated at {} entries",
            service.limits().max_store_entries
        );
    }
    Ok(())
}

fn delete(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_session_args("delete", args, SessionFlags::DELETE)?;
    let root = options
        .root
        .as_deref()
        .ok_or_else(|| CliError("`session delete` requires `--root <DIR>`".to_owned()))?;
    let session_key = options
        .session
        .ok_or_else(|| CliError("`session delete` requires `--session <KEY>`".to_owned()))?;
    let input = JsonValue::object([
        ("session_key", json_unsigned(session_key, "--session")?),
        (
            "snapshot_key",
            optional_key_json(options.snapshot, "--snapshot")?,
        ),
    ]);
    let service = service(root)?;
    let response = dispatch_local(&service, "store.delete", &input)?;
    if options.json {
        return write_canonical_line(&response);
    }
    let result = result_of(&response)?;
    let deleted_session = required_u64(&result, "session_key")?;
    let deleted_snapshot = optional_u64(&result, "snapshot_key")?;
    let mut output = String::new();
    write_deleted_block(&mut output, deleted_session, deleted_snapshot);
    print!("{output}");
    Ok(())
}

/// Reads the durable session record for its advisory name.
fn durable_session_name(service: &AppService, session_key: u64) -> Result<String, Box<dyn Error>> {
    let input = JsonValue::object([("session_key", json_unsigned(session_key, "--session")?)]);
    let response = dispatch_local(service, "store.list", &input)?;
    let result = result_of(&response)?;
    let session = result
        .get("sessions")
        .and_then(JsonValue::as_array)
        .and_then(|entries| entries.first())
        .ok_or_else(|| CliError("durable session listing is empty".to_owned()))?;
    Ok(nullable_str(session, "name")?.to_owned())
}

/// Renders the documented `key=value` records for one recorded snapshot.
struct SnapshotFields<'a> {
    session_key: u64,
    snapshot_key: u64,
    name: &'a str,
    tick: u64,
    state_hash: &'a str,
    creation_sequence: u64,
    parent_session_key: Option<u64>,
    origin_snapshot_key: Option<u64>,
}

fn write_snapshot_result(output: &mut String, result: &JsonValue) -> Result<(), Box<dyn Error>> {
    let fields = SnapshotFields {
        session_key: required_u64(result, "session_key")?,
        snapshot_key: required_u64(result, "snapshot_key")?,
        name: nullable_str(result, "name")?,
        tick: required_u64(result, "tick")?,
        state_hash: required_str(result, "state_hash")?,
        creation_sequence: required_u64(result, "creation_sequence")?,
        parent_session_key: optional_u64(result, "parent_session_key")?,
        origin_snapshot_key: optional_u64(result, "origin_snapshot_key")?,
    };
    write_snapshot_fields(output, &fields);
    Ok(())
}

/// Renders one snapshot from a listing, whose lineage is inherited from the session.
fn write_snapshot_entry(
    output: &mut String,
    session_key: u64,
    snapshot: &JsonValue,
    parent_session_key: Option<u64>,
    origin_snapshot_key: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    let fields = SnapshotFields {
        session_key,
        snapshot_key: required_u64(snapshot, "snapshot_key")?,
        name: nullable_str(snapshot, "name")?,
        tick: required_u64(snapshot, "tick")?,
        state_hash: required_str(snapshot, "state_hash")?,
        creation_sequence: required_u64(snapshot, "creation_sequence")?,
        parent_session_key,
        origin_snapshot_key,
    };
    write_snapshot_fields(output, &fields);
    Ok(())
}

fn write_snapshot_fields(output: &mut String, fields: &SnapshotFields<'_>) {
    begin_record(output, "snapshot");
    let _ = writeln!(output, "session_key={}", fields.session_key);
    let _ = writeln!(output, "snapshot_key={}", fields.snapshot_key);
    let _ = writeln!(output, "name={}", fields.name);
    let _ = writeln!(output, "tick={}", fields.tick);
    let _ = writeln!(output, "state_hash={}", fields.state_hash);
    let _ = writeln!(output, "creation_sequence={}", fields.creation_sequence);
    let _ = writeln!(
        output,
        "parent_session_key={}",
        optional_key(fields.parent_session_key)
    );
    let _ = writeln!(
        output,
        "origin_snapshot_key={}",
        optional_key(fields.origin_snapshot_key)
    );
}

fn write_session_block(output: &mut String, session_key: u64, name: &str) {
    begin_record(output, "session");
    let _ = writeln!(output, "session_key={session_key}");
    let _ = writeln!(output, "name={name}");
}

fn write_deleted_block(output: &mut String, session_key: u64, snapshot_key: Option<u64>) {
    begin_record(output, "deleted");
    let _ = writeln!(output, "session_key={session_key}");
    if let Some(snapshot_key) = snapshot_key {
        let _ = writeln!(output, "snapshot_key={snapshot_key}");
    }
}

/// Reports the service's anomaly labels as diagnostics; nothing is repaired.
///
/// A list that meets uncommitted or corrupt entries still succeeds. The service bounds
/// the number of labels it transports (with a total `count`), so the CLI prints every
/// transported label and a summary line when the list was truncated.
fn write_anomalies(result: &JsonValue) -> Result<(), Box<dyn Error>> {
    let anomalies = result
        .get("anomalies")
        .ok_or_else(|| CliError("service envelope has no `anomalies`".to_owned()))?;
    let count = anomalies
        .get("count")
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| CliError("service envelope has no anomaly count".to_owned()))?;
    let labels = anomalies
        .get("labels")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| CliError("service envelope has no anomaly labels".to_owned()))?;
    for label in labels {
        let label = label
            .as_str()
            .ok_or_else(|| CliError("anomaly label is not a string".to_owned()))?;
        eprintln!("chronofork: storage anomaly: {label}");
    }
    let shown = u64::try_from(labels.len()).unwrap_or(u64::MAX);
    if count > shown {
        eprintln!(
            "chronofork: storage anomaly: {} more not shown",
            count - shown
        );
    }
    Ok(())
}

/// Formats an optional durable key for the `-`-when-absent block convention.
fn optional_key(key: Option<u64>) -> String {
    match key {
        Some(key) => key.to_string(),
        None => "-".to_owned(),
    }
}

/// Renders an optional durable key as a canonical JSON nullable integer.
fn optional_key_json(key: Option<u64>, flag: &str) -> Result<JsonValue, CliError> {
    key.map_or(Ok(JsonValue::Null), |key| json_unsigned(key, flag))
}

fn required_u64(value: &JsonValue, key: &str) -> Result<u64, Box<dyn Error>> {
    value
        .get(key)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| CliError(format!("service envelope is missing `{key}`")).into())
}

fn optional_u64(value: &JsonValue, key: &str) -> Result<Option<u64>, Box<dyn Error>> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(entry) => entry
            .as_u64()
            .map(Some)
            .ok_or_else(|| CliError(format!("service envelope has a non-integer `{key}`")).into()),
    }
}

fn required_str<'a>(value: &'a JsonValue, key: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| CliError(format!("service envelope is missing `{key}`")).into())
}

fn nullable_str<'a>(value: &'a JsonValue, key: &str) -> Result<&'a str, Box<dyn Error>> {
    match value.get(key) {
        Some(JsonValue::String(text)) => Ok(text.as_str()),
        Some(JsonValue::Null) => Ok(""),
        _ => Err(CliError(format!("service envelope has no `{key}` label")).into()),
    }
}
