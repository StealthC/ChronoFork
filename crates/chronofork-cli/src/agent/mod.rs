//! Stage 6 command surfaces: `serve`, `api`, `mcp` and `experiment`.
//!
//! This module tree is the process-level adapter described by
//! [ADR-0012](../../../docs/adr/0012-agent-native-interfaces.md) and
//! [architecture §8](../../../docs/architecture/agent-interfaces.md#8-cli-as-a-client).
//! It composes a backend factory and a token file, reads process flags and writes
//! output and exit codes. Every operation is dispatched by `chronofork-service` and every
//! wire byte comes from `chronofork-transport`; nothing here validates an operation,
//! enforces a scope, re-encodes a response or re-implements a resource bound.
//!
//! Output policy, shared with the refactored `session` commands:
//!
//! - success writes the canonical response bytes unchanged followed by exactly one
//!   newline, so stdout is always machine-consumable;
//! - a typed remote failure (`non-2xx`, or an MCP `tools/call` with `isError:true`)
//!   writes the canonical `chronofork.error.v1` envelope to stderr as one line and
//!   leaves stdout empty;
//! - a local usage/transport failure writes one `chronofork: ...` line to stderr and
//!   nothing to stdout.
//!
//! Each submodule owns one command family; the plumbing below is the only code shared
//! between them and the `session` commands.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{
    AppService, Principal, Scope, ServiceConfig, ServiceLimits, TokenTable, parse_token_file,
};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::TransportError;

use crate::CliError;

mod api;
mod experiment;
mod mcp;
mod serve;

pub(crate) use api::run as run_api;
pub(crate) use experiment::run as run_experiment;
pub(crate) use mcp::run as run_mcp;
pub(crate) use serve::run as run_serve;

/// Marks a failure whose diagnostic was already written to stderr verbatim.
///
/// A typed remote failure is reported by the canonical error envelope itself. Printing
/// an additional `chronofork:` line would make stderr two lines and break the one-line
/// diagnostic contract, so [`crate::main`] suppresses its own line for this error.
#[derive(Debug)]
pub(crate) struct FailureReported;

impl fmt::Display for FailureReported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("failure diagnostic already reported")
    }
}

impl Error for FailureReported {}

/// The locally constructed administrator principal.
///
/// A local `session` or `experiment` command runs in the same process and trust domain
/// as its in-process service, which is the documented local boundary. Operation
/// semantics (validation, scope checks, limits) still run inside the service; only the
/// credential is implied by the process boundary. Remote surfaces always authenticate a
/// token from a file instead.
pub(crate) fn local_admin() -> Principal {
    Principal::new("local", [Scope::Admin])
}

/// Creates an in-process service over the synthetic test backend.
///
/// `storage_root` enables the durable `store.*` operations; `artifacts_root` enables
/// content-addressed artifact persistence. The command modules never construct a
/// service themselves.
pub(crate) fn local_service(
    storage_root: Option<PathBuf>,
    artifacts_root: Option<PathBuf>,
) -> Result<AppService, Box<dyn Error>> {
    let factory: Arc<dyn BackendFactory> = Arc::new(SyntheticBackendFactory::default());
    Ok(AppService::new(ServiceConfig {
        factory,
        storage_root,
        artifacts_root,
        limits: ServiceLimits::default(),
    })?)
}

/// Dispatches one operation through an in-process service and returns its envelope bytes.
///
/// A local command reports a service failure with the historical human shape,
/// `chronofork: <message>`. The typed error code stays inside canonical JSON error
/// envelopes (the `api`/`mcp` client surfaces) instead of leaking into a local
/// diagnostic, so the durable session commands keep their pre-M6 stderr wording.
pub(crate) fn dispatch_local(
    service: &AppService,
    op: &str,
    input: &JsonValue,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let limits = *service.limits();
    let request = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
    let body = to_canonical_bytes(&request, &limits)?;
    match service.dispatch(&local_admin(), &body) {
        Ok(envelope) => Ok(envelope),
        Err(error) => Err(Box::new(CliError(error.message().to_owned()))),
    }
}

/// Parses a canonical envelope and returns its `result` object.
pub(crate) fn result_of(envelope: &[u8]) -> Result<JsonValue, Box<dyn Error>> {
    let limits = ServiceLimits::default();
    let document = parse(envelope, &limits)?;
    document
        .get("result")
        .cloned()
        .ok_or_else(|| CliError("service envelope has no `result`".to_owned()).into())
}

/// Writes canonical bytes followed by exactly one newline and flushes stdout.
pub(crate) fn write_canonical_line(bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    output.write_all(bytes)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

/// Writes already-canonical diagnostic bytes as one stderr line.
fn write_stderr_line(bytes: &[u8]) {
    let stderr = io::stderr();
    let mut output = stderr.lock();
    let _ = output.write_all(bytes);
    let _ = output.write_all(b"\n");
    let _ = output.flush();
}

/// Converts a validated unsigned value into a canonical JSON integer.
pub(crate) fn json_unsigned(value: u64, flag: &str) -> Result<JsonValue, CliError> {
    let value = i64::try_from(value).map_err(|_| {
        CliError(format!(
            "`{flag}` is too large for the canonical JSON profile"
        ))
    })?;
    Ok(JsonValue::int(value))
}

/// Converts an optional advisory name into `null` or a JSON string.
pub(crate) fn json_optional_string(value: Option<&str>) -> JsonValue {
    value.map_or(JsonValue::Null, JsonValue::string)
}

/// Maps a transport failure onto the documented process behaviour.
///
/// A typed remote failure carries the canonical envelope, which becomes the single
/// stderr line. Every other failure is a local diagnostic.
fn transport_failure(error: TransportError) -> Box<dyn Error> {
    match error {
        TransportError::Http { body, .. } if !body.is_empty() => {
            write_stderr_line(&body);
            Box::new(FailureReported)
        }
        other => Box::new(CliError(other.to_string())),
    }
}

/// Reads one bounded file, rejecting it before it is larger than the service limit.
fn read_bounded_file(path: &Path, limit: usize, what: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let file = File::open(path).map_err(|error| {
        CliError(format!(
            "failed to read {what} `{}`: {error}",
            path.display()
        ))
    })?;
    read_bounded(file, limit, what, &path.display().to_string())
}

/// Reads at most `limit` bytes and rejects a larger reader.
fn read_bounded<R: Read>(
    reader: R,
    limit: usize,
    what: &str,
    display: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut buffer = Vec::new();
    let bound = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    reader.take(bound).read_to_end(&mut buffer)?;
    if buffer.len() > limit {
        return Err(CliError(format!("{what} `{display}` exceeds the {limit}-byte limit")).into());
    }
    Ok(buffer)
}

/// Reads the token file into a validated table.
///
/// The service owns every validation rule; the CLI only refuses to start (or to connect)
/// with an invalid or empty table. The read is capped at `limit` bytes before the
/// document is parsed, so an oversized file is rejected without buffering it; the
/// library parser keeps its own default bound.
fn read_token_table(path: &Path, limit: usize) -> Result<TokenTable, Box<dyn Error>> {
    let bytes = read_bounded_file(path, limit, "token file")?;
    parse_token_file(&bytes).map_err(|error| {
        CliError(format!(
            "invalid token file `{}`: {}",
            path.display(),
            error.message()
        ))
        .into()
    })
}

/// Selects the bearer token of one principal, defaulting to the first one in the file.
fn select_token(
    table: &TokenTable,
    requested: Option<&str>,
    path: &Path,
) -> Result<String, Box<dyn Error>> {
    let selected = match requested {
        Some(name) => table
            .credentials()
            .find(|(candidate, _)| *candidate == name),
        None => table.credentials().next(),
    };
    let (_, token) = selected.ok_or_else(|| match requested {
        Some(name) => CliError(format!(
            "token file `{}` has no principal `{name}`",
            path.display()
        )),
        None => CliError(format!("token file `{}` has no principal", path.display())),
    })?;
    Ok(token.to_owned())
}

/// Parses one canonical JSON document supplied on the command line or stdin.
fn parse_input_json(bytes: &[u8], what: &str) -> Result<JsonValue, Box<dyn Error>> {
    let limits = ServiceLimits::default();
    parse(bytes, &limits).map_err(|error| {
        CliError(format!("{what} is not canonical JSON: {}", error.message())).into()
    })
}
