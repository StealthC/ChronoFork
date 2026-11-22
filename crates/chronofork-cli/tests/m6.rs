// Anti-slop policy §10 exception: owner cli-dev; scope this integration test binary
// only; expiry when the repository retires the process-test `expect` convention. It
// mirrors the existing headers in tests/debug.rs, tests/demo.rs, tests/session_storage.rs
// and tests/trace.rs; no production code is affected.
#![allow(clippy::expect_used)]

//! Process-level acceptance tests for the Stage 6 CLI surfaces.
//!
//! Every test spawns the built `chronofork` binary as a real operating-system process,
//! so each invocation crosses the documented process boundary: stdout, stderr and the
//! exit status are the whole contract. The tests cover
//! [M6-A20](../docs/quality/m6-agent-interfaces-acceptance.md) through M6-A25:
//! `serve` + `api`, `serve` + `mcp`, `session --json` parity with the direct service,
//! `experiment run --out` parity and the documented rejection/exit-code cases.
//!
//! Expected bytes are produced by the direct `AppService` or by the typed transport
//! client — issued against the same daemon, on the same fresh state — and compared
//! exactly, with one documented exception: the operational `process_epoch` a v2 mutation
//! result carries necessarily differs between two processes, so `normalized_epoch`
//! replaces exactly that field before the comparison instead of dropping the equality
//! requirement (ADR-0016 §2). Servers use ephemeral loopback ports and stop after
//! a bounded request count; every wait is deadline-polled, never a fixed sleep.

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};
use chronofork_service::{
    ApiVersion, AppService, Principal, Scope, ServiceConfig, ServiceError, ServiceLimits,
};
use chronofork_sessions::SnapshotRepository;
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{HttpClient, McpClient};

use common::{path_str, run, temp_dir, write_rom};

/// A `chronofork.tokens.v1` table with the three documented scope kinds.
const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]},
 {"name":"other","token":"other-token-000001","scopes":["read","write"]},
 {"name":"admin","token":"admin-token-000001","scopes":["admin"]}]}"#;

/// Bearer token of the `operator` principal in [`TOKENS`].
const OPERATOR_TOKEN: &str = "operator-token-0001";

/// Bound on how long a `serve` child may take to print its records or stop.
const SERVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Synthetic backend identifier, as reported by the service capabilities operation.
const SYNTHETIC_BACKEND_ID: &str = "chronofork.test.synthetic";
/// Mega Drive backend identifier, as reported by the service capabilities operation.
const MEGADRIVE_BACKEND_ID: &str = "chronofork.megadrive.m68000";

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn empty() -> JsonValue {
    JsonValue::Object(Default::default())
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::object(entries)
}

fn parse_json(bytes: &[u8]) -> JsonValue {
    parse(bytes, &limits()).expect("canonical JSON")
}

fn with_newline(bytes: &[u8]) -> Vec<u8> {
    let mut output = bytes.to_vec();
    output.push(b'\n');
    output
}

fn direct_service(storage_root: Option<PathBuf>) -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root,
        artifacts_root: None,
        limits: limits(),
    })
    .expect("direct service")
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn other() -> Principal {
    Principal::new("other", [Scope::Read, Scope::Write])
}

fn admin() -> Principal {
    Principal::new("admin", [Scope::Admin])
}

fn request(op: &str, input: &JsonValue) -> Vec<u8> {
    let document = object([("op", JsonValue::string(op)), ("input", input.clone())]);
    to_canonical_bytes(&document, &limits()).expect("canonical request")
}

/// Dispatches one operation on the **v1** wire.
///
/// The local `session save|load|list|delete` commands dispatch in-process without a
/// version header (`dispatch_local`), and a v1 response is exactly the document the same
/// request returns from the HTTP surface without a version header. The expectations for
/// those commands are therefore built on v1; only the `api`/`mcp` surfaces are v2.
fn dispatch_v1(
    service: &AppService,
    principal: &Principal,
    op: &str,
    input: &JsonValue,
) -> Vec<u8> {
    service
        .dispatch(principal, &request(op, input))
        .expect("the service must accept this request")
}

fn dispatch(service: &AppService, principal: &Principal, op: &str, input: &JsonValue) -> Vec<u8> {
    // The CLI's `api`/`mcp` surfaces are v2 consumers (they probe the daemon and declare
    // v2), so the direct expectation must be built on the same wire version.
    service
        .dispatch_versioned(principal, &request(op, input), ApiVersion::V2)
        .expect("the direct service must accept this request")
}

fn dispatch_error(
    service: &AppService,
    principal: &Principal,
    op: &str,
    input: &JsonValue,
) -> Vec<u8> {
    let error: ServiceError = service
        .dispatch_versioned(principal, &request(op, input), ApiVersion::V2)
        .expect_err("dispatch must fail");
    error
        .to_canonical_bytes_versioned(&limits(), ApiVersion::V2)
        .expect("error envelope")
}

/// Replaces the operational `process_epoch` value with `0` before a byte comparison.
///
/// Two processes always report different epochs, and the epoch is deliberately not
/// emulated state (ADR-0016 §2), so envelope-parity assertions normalize exactly that
/// one field instead of dropping the requirement that the rest is identical.
fn normalized_epoch(bytes: &[u8]) -> Vec<u8> {
    let limits = limits();
    let document = parse(bytes, &limits).expect("canonical envelope");
    let JsonValue::Object(mut root) = document else {
        return bytes.to_vec();
    };
    if let Some(JsonValue::Object(result)) = root.get_mut("result") {
        if result.contains_key("process_epoch") {
            result.insert("process_epoch".to_owned(), JsonValue::int(0));
        }
    }
    to_canonical_bytes(&JsonValue::Object(root), &limits).expect("canonical bytes")
}

fn write_tokens(dir: &Path) -> PathBuf {
    let path = dir.join("tokens.json");
    std::fs::write(&path, TOKENS).expect("write token file");
    path
}

fn cli(args: &[&str]) -> Output {
    run(args)
}

fn streams(output: &Output) -> (String, String) {
    (
        String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8"),
        String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8"),
    )
}

/// Requires exit 0, empty stderr and returns stdout.
fn expect_success(output: &Output) -> String {
    let (stdout, stderr) = streams(output);
    assert!(
        output.status.success(),
        "command failed\nexit: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    assert!(
        stderr.is_empty(),
        "successful command wrote to stderr:\n{stderr}"
    );
    stdout
}

/// Requires exit 1, empty stdout, one newline-terminated diagnostic and returns stderr.
fn expect_failure(output: &Output) -> String {
    let (stdout, stderr) = streams(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "typed failures exit 1\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.is_empty(),
        "typed failures keep stdout data-only: {stdout}"
    );
    assert!(!stderr.is_empty(), "a failure carries a diagnostic");
    assert!(
        stderr.ends_with('\n') && stderr.lines().count() == 1,
        "a failure is one newline-terminated stderr line:\n{stderr}"
    );
    stderr
}

/// Spawns `chronofork serve` on an ephemeral loopback port and reads its records.
fn spawn_serve(dir: &Path, extra: &[&str]) -> (Child, String, BTreeMap<String, String>) {
    let tokens = write_tokens(dir);
    let mut command = Command::new(env!("CARGO_BIN_EXE_chronofork"));
    command
        .args([
            "serve",
            "--token-file",
            path_str(&tokens),
            "--bind",
            "127.0.0.1:0",
        ])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn `chronofork serve`");
    let stdout = child.stdout.take().expect("serve stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut failure = String::new();
    let mut records = BTreeMap::new();
    for _ in 0..5 {
        match lines.next() {
            Some(Ok(line)) => {
                let Some((key, value)) = line.split_once('=') else {
                    failure = format!("serve record line is not `key=value`: {line}");
                    break;
                };
                records.insert(key.to_owned(), value.to_owned());
            }
            Some(Err(error)) => {
                failure = format!("failed to read a serve record: {error}");
                break;
            }
            None => {
                failure = "serve closed stdout before its records".to_owned();
                break;
            }
        }
    }
    if !failure.is_empty() {
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }
        let status = child.wait().expect("serve exits");
        assert!(
            failure.is_empty(),
            "serve did not publish its records ({failure}, status {status}): {stderr}"
        );
    }
    assert_eq!(records.get("record").map(String::as_str), Some("serve"));
    let bind = records.get("bind").expect("serve prints `bind=`").clone();
    let address: SocketAddr = bind.parse().expect("bind is a socket address");
    assert_eq!(address.ip().to_string(), "127.0.0.1");
    let base = format!("http://{bind}");
    (child, base, records)
}

/// Polls a child until it exits or the deadline expires.
fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait on the child") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Waits for the daemon to stop and returns its status and stderr.
fn finish_serve(mut child: Child) -> (ExitStatus, String) {
    let status =
        wait_for_exit(&mut child, SERVE_TIMEOUT).expect("serve stops after --max-requests");
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    (status, stderr)
}

// ---------------------------------------------------------------------------
// M6-A20: serve process flow, records and backend selection
// ---------------------------------------------------------------------------

#[test]
fn serve_records_select_the_configured_backend_and_storage() {
    let dir = temp_dir("m6-serve-records");
    let rom = write_rom(&dir);
    let store = dir.join("store");
    let artifacts = dir.join("artifacts");
    let tokens = write_tokens(&dir);

    // Three requests: `/health`, the v2 capabilities probe, the capabilities operation
    // (a v2 client probes once before its first call, ADR-0016 §Compatibility).
    let (child, base, records) = spawn_serve(
        &dir,
        &[
            "--rom",
            path_str(&rom),
            "--store-root",
            path_str(&store),
            "--artifacts",
            path_str(&artifacts),
            "--max-requests",
            "3",
        ],
    );
    assert_eq!(
        records.get("backend").map(String::as_str),
        Some("megadrive")
    );
    assert_eq!(records.get("store").map(String::as_str), Some("enabled"));
    assert_eq!(
        records.get("artifacts").map(String::as_str),
        Some("enabled")
    );

    let health = expect_success(&cli(&[
        "api",
        "health",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
    ]));
    assert_eq!(
        health,
        "{\"schema\":\"chronofork.health.v1\",\"status\":\"ok\"}\n"
    );

    let capabilities = expect_success(&cli(&[
        "api",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
        "--op",
        "capabilities",
    ]));
    let envelope = parse_json(capabilities.trim_end().as_bytes());
    assert_eq!(
        envelope.get("op").and_then(JsonValue::as_str),
        Some("capabilities")
    );
    let result = envelope.get("result").expect("capabilities result");
    let backend = result.get("backend").expect("backend report");
    assert_eq!(
        backend.get("id").and_then(JsonValue::as_str),
        Some(MEGADRIVE_BACKEND_ID)
    );
    assert_eq!(
        result
            .get("store")
            .and_then(|store| store.get("configured"))
            .and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(
        result
            .get("artifacts")
            .and_then(|artifacts| artifacts.get("configured"))
            .and_then(JsonValue::as_bool),
        Some(true)
    );

    let (status, stderr) = finish_serve(child);
    assert!(
        status.success(),
        "serve exits 0 after --max-requests: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "loopback serve writes no diagnostics: {stderr}"
    );
}

#[test]
fn serve_stops_on_process_termination() {
    let dir = temp_dir("m6-serve-termination");
    let (mut child, _base, records) = spawn_serve(&dir, &[]);
    assert_eq!(
        records.get("backend").map(String::as_str),
        Some("synthetic")
    );
    assert_eq!(records.get("store").map(String::as_str), Some("disabled"));
    assert_eq!(
        records.get("artifacts").map(String::as_str),
        Some("disabled")
    );
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "serve keeps running without --max-requests"
    );
    child.kill().expect("terminate the daemon");
    let status = child.wait().expect("reap the daemon");
    assert!(
        !status.success(),
        "a terminated daemon was not reported as a clean exit"
    );
}

// ---------------------------------------------------------------------------
// M6-A21: api client parity
// ---------------------------------------------------------------------------

#[test]
fn serve_and_api_process_flow() {
    let dir = temp_dir("m6-serve-api");
    let tokens = write_tokens(&dir);
    // Thirteen requests: the v1 tools oracle, health, the tools read, then each v2
    // `api call` costs its one-time read-only capabilities probe plus the call itself
    // (create, list, other list, unknown op, capabilities) — ADR-0016 §Compatibility.
    let (child, base, records) = spawn_serve(&dir, &["--max-requests", "13"]);
    assert_eq!(
        records.get("backend").map(String::as_str),
        Some("synthetic")
    );
    assert_eq!(records.get("store").map(String::as_str), Some("disabled"));
    assert_eq!(
        records.get("artifacts").map(String::as_str),
        Some("disabled")
    );

    // Request 1: the typed client is the expected-bytes oracle for the tools route.
    let client = HttpClient::new(&base, OPERATOR_TOKEN);
    let expected_tools = client.tools().expect("the daemon serves /v1/tools");

    // Request 2: health is the unauthenticated route with a fixed document.
    let health = expect_success(&cli(&[
        "api",
        "health",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
    ]));
    assert_eq!(
        health,
        "{\"schema\":\"chronofork.health.v1\",\"status\":\"ok\"}\n"
    );

    // Request 3: `api tools` prints the daemon bytes unchanged plus one newline.
    let tools = expect_success(&cli(&[
        "api",
        "tools",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
    ]));
    assert_eq!(tools.as_bytes(), with_newline(&expected_tools).as_slice());

    let direct = direct_service(None);

    // Request 4: create a session as `operator`; the direct service produced the same
    // envelope for the same principal on a fresh instance.
    let expected_create = dispatch(&direct, &operator(), "session.create", &empty());
    let created = expect_success(&cli(&[
        "api",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
        "--op",
        "session.create",
    ]));
    // The mutation result carries the operational `process_epoch`, which necessarily
    // differs between the daemon and this direct service; the parity helper normalizes
    // exactly that field (ADR-0016 §2).
    assert_eq!(
        normalized_epoch(created.as_bytes()),
        normalized_epoch(&with_newline(&expected_create))
    );

    // Requests 5 and 6: ownership is visible through the same service envelopes.
    let expected_operator_list = dispatch(&direct, &operator(), "session.list", &empty());
    let expected_other_list = dispatch(&direct, &other(), "session.list", &empty());
    let listed = expect_success(&cli(&[
        "api",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
        "--op",
        "session.list",
    ]));
    assert_eq!(
        listed.as_bytes(),
        with_newline(&expected_operator_list).as_slice()
    );
    let other_list = expect_success(&cli(&[
        "api",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "other",
        "--op",
        "session.list",
    ]));
    assert_eq!(
        other_list.as_bytes(),
        with_newline(&expected_other_list).as_slice()
    );

    // Request 7: an unknown operation is a typed remote failure: the canonical envelope
    // is the single stderr line and stdout stays empty.
    let expected_error = dispatch_error(&direct, &operator(), "nope", &empty());
    let failed = cli(&[
        "api",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
        "--op",
        "nope",
    ]);
    let stderr = expect_failure(&failed);
    assert_eq!(stderr.as_bytes(), with_newline(&expected_error).as_slice());

    // Request 8: capabilities as `admin`; the direct service bytes are identical.
    let expected_capabilities = dispatch(&direct, &admin(), "capabilities", &empty());
    let capabilities = expect_success(&cli(&[
        "api",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "admin",
        "--op",
        "capabilities",
    ]));
    // The epoch differs by process; everything else must match byte for byte.
    assert_eq!(
        normalized_epoch(capabilities.as_bytes()),
        normalized_epoch(&with_newline(&expected_capabilities))
    );
    let report = parse_json(capabilities.trim_end().as_bytes());
    assert_eq!(
        report
            .get("result")
            .and_then(|result| result.get("backend"))
            .and_then(|backend| backend.get("id"))
            .and_then(JsonValue::as_str),
        Some(SYNTHETIC_BACKEND_ID)
    );

    let (status, stderr) = finish_serve(child);
    assert!(
        status.success(),
        "serve exits 0 after --max-requests: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "loopback serve writes no diagnostics: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// M6-A21: stdin input documents
// ---------------------------------------------------------------------------

#[test]
fn api_reads_the_input_document_from_stdin() {
    let dir = temp_dir("m6-api-stdin");
    let tokens = write_tokens(&dir);
    // Two requests: the v2 capabilities probe and the capabilities call below.
    let (child, base, _records) = spawn_serve(&dir, &["--max-requests", "2"]);

    let mut command = Command::new(env!("CARGO_BIN_EXE_chronofork"));
    command
        .args([
            "api",
            "call",
            "--url",
            &base,
            "--token-file",
            path_str(&tokens),
            "--principal",
            "operator",
            "--op",
            "capabilities",
            "--input-file",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut client = command.spawn().expect("spawn `api call` with stdin");
    {
        let mut stdin = client.stdin.take().expect("child stdin");
        stdin.write_all(b"{}").expect("write the input document");
    }
    let output = client.wait_with_output().expect("`api call` output");
    let stdout = expect_success(&output);

    let direct = direct_service(None);
    let expected = dispatch(&direct, &operator(), "capabilities", &empty());
    // `process_epoch` is operational metadata, not emulated state: the two documents
    // come from different processes, so the epoch is normalized before the byte
    // comparison — the rest of the document must still match byte for byte.
    assert_eq!(
        normalized_epoch(stdout.as_bytes()),
        normalized_epoch(&with_newline(&expected))
    );

    let (status, stderr) = finish_serve(child);
    assert!(
        status.success(),
        "serve exits 0 after --max-requests: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "loopback serve writes no diagnostics: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// M6-A22: mcp client parity
// ---------------------------------------------------------------------------

#[test]
fn serve_and_mcp_process_flow() {
    let dir = temp_dir("m6-serve-mcp");
    let tokens = write_tokens(&dir);
    // Nine requests: initialize and tools/list (v1 oracles), then every v2 CLI call
    // costs its one-time capabilities probe plus the call (mcp tools, mcp call, the
    // HTTP capabilities oracle and the failing mcp call) — ADR-0016 §Compatibility.
    let (child, base, _records) = spawn_serve(&dir, &["--max-requests", "9"]);

    // Request 1: protocol negotiation (MCP Streamable HTTP, stateless).
    let mcp = McpClient::new(&base, OPERATOR_TOKEN);
    let initialized = mcp.initialize().expect("initialize");
    assert_eq!(
        initialized
            .get("protocolVersion")
            .and_then(JsonValue::as_str),
        Some("2025-03-26")
    );
    assert_eq!(
        initialized
            .get("serverInfo")
            .and_then(|info| info.get("name"))
            .and_then(JsonValue::as_str),
        Some("chronofork")
    );

    // Request 2: the typed client is the expected-bytes oracle for tools/list.
    let tools = mcp.list_tools().expect("tools/list");
    let expected_tools = to_canonical_bytes(&tools, &limits()).expect("tools bytes");

    // Request 3: `mcp tools` prints the canonical result plus one newline.
    let listed = expect_success(&cli(&[
        "mcp",
        "tools",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
    ]));
    assert_eq!(listed.as_bytes(), with_newline(&expected_tools).as_slice());

    // Request 4: `mcp call` prints the tool result text, which is the service envelope.
    let called = expect_success(&cli(&[
        "mcp",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
        "--tool",
        "capabilities",
        "--arguments",
        "{}",
    ]));

    // Request 5: the same operation through the HTTP surface. The MCP text content and
    // the HTTP envelope compare byte for byte.
    // The MCP call above is a v2 call, so the HTTP oracle must ask for v2 too; the
    // epoch is normalized because the two processes report their own.
    let http = HttpClient::new(&base, OPERATOR_TOKEN).with_version(ApiVersion::V2);
    let expected_envelope = http
        .call("capabilities", &empty())
        .expect("HTTP capabilities");
    assert_eq!(
        normalized_epoch(called.as_bytes()),
        normalized_epoch(&with_newline(&expected_envelope))
    );

    // Request 6: an operation error is a `tools/call` result with `isError:true`; the
    // CLI reports it as one stderr line and exits 1 without stdout output.
    let failed = cli(&[
        "mcp",
        "call",
        "--url",
        &base,
        "--token-file",
        path_str(&tokens),
        "--principal",
        "operator",
        "--tool",
        "session.inspect",
        "--arguments",
        "{\"session\":99}",
    ]);
    let stderr = expect_failure(&failed);
    let error = parse_json(stderr.trim_end().as_bytes());
    assert_eq!(
        error.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1")
    );
    assert_eq!(
        error
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("not_found")
    );

    let (status, stderr) = finish_serve(child);
    assert!(
        status.success(),
        "serve exits 0 after --max-requests: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "loopback serve writes no diagnostics: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// M6-A23: session --json parity
// ---------------------------------------------------------------------------

#[test]
fn session_json_envelopes_match_the_service_operations() {
    let dir = temp_dir("m6-session-json");
    let cli_root = dir.join("cli-root");
    let service_root = dir.join("service-root");
    std::fs::create_dir_all(&cli_root).expect("create the CLI root");
    std::fs::create_dir_all(&service_root).expect("create the service root");
    let service = direct_service(Some(service_root));

    // store.save: a fresh durable session with one snapshot.
    let save_input = object([
        ("steps", JsonValue::int(4)),
        ("buttons", JsonValue::int(1)),
        ("name", JsonValue::string("m6")),
    ]);
    let expected_save = dispatch_v1(&service, &admin(), "store.save", &save_input);
    let saved = expect_success(&cli(&[
        "session",
        "save",
        "--root",
        path_str(&cli_root),
        "--name",
        "m6",
        "--steps",
        "4",
        "--input",
        "1",
        "--json",
    ]));
    assert_eq!(saved.as_bytes(), with_newline(&expected_save).as_slice());

    // store.load: rehydrate the appended snapshot and advance two steps in memory.
    let load_input = object([
        ("session_key", JsonValue::int(1)),
        ("steps", JsonValue::int(2)),
    ]);
    let expected_load = dispatch_v1(&service, &admin(), "store.load", &load_input);
    let loaded = expect_success(&cli(&[
        "session",
        "load",
        "--root",
        path_str(&cli_root),
        "--session",
        "1",
        "--steps",
        "2",
        "--json",
    ]));
    assert_eq!(loaded.as_bytes(), with_newline(&expected_load).as_slice());

    // store.list: every durable session and snapshot.
    let expected_list = dispatch_v1(&service, &admin(), "store.list", &empty());
    let listed = expect_success(&cli(&[
        "session",
        "list",
        "--root",
        path_str(&cli_root),
        "--json",
    ]));
    assert_eq!(listed.as_bytes(), with_newline(&expected_list).as_slice());

    // store.delete: one snapshot, then the now-empty session (no cascade).
    let delete_snapshot_input = object([
        ("session_key", JsonValue::int(1)),
        ("snapshot_key", JsonValue::int(1)),
    ]);
    let expected_delete_snapshot =
        dispatch_v1(&service, &admin(), "store.delete", &delete_snapshot_input);
    let deleted_snapshot = expect_success(&cli(&[
        "session",
        "delete",
        "--root",
        path_str(&cli_root),
        "--session",
        "1",
        "--snapshot",
        "1",
        "--json",
    ]));
    assert_eq!(
        deleted_snapshot.as_bytes(),
        with_newline(&expected_delete_snapshot).as_slice()
    );

    let delete_session_input = object([("session_key", JsonValue::int(1))]);
    let expected_delete_session =
        dispatch_v1(&service, &admin(), "store.delete", &delete_session_input);
    let deleted_session = expect_success(&cli(&[
        "session",
        "delete",
        "--root",
        path_str(&cli_root),
        "--session",
        "1",
        "--json",
    ]));
    assert_eq!(
        deleted_session.as_bytes(),
        with_newline(&expected_delete_session).as_slice()
    );

    // `--json` always prints exactly one canonical envelope line.
    for output in [saved, loaded, listed, deleted_snapshot, deleted_session] {
        assert!(
            output.ends_with('\n') && !output.trim_end().contains('\n'),
            "`--json` prints one envelope line plus its newline:\n{output}"
        );
        let envelope = parse_json(output.trim_end().as_bytes());
        assert!(
            envelope.get("op").and_then(JsonValue::as_str).is_some(),
            "`--json` prints a canonical service envelope:\n{output}"
        );
    }
}

// ---------------------------------------------------------------------------
// M6-A24: experiment artifact parity
// ---------------------------------------------------------------------------

#[test]
fn experiment_artifact_matches_the_service_document() {
    let dir = temp_dir("m6-experiment");
    let out = dir.join("experiment.cfexp.json");
    let arguments = object([
        ("steps", JsonValue::int(7)),
        ("buttons", JsonValue::int(3)),
        (
            "checkpoints",
            JsonValue::Array(vec![
                JsonValue::int(0),
                JsonValue::int(3),
                JsonValue::int(7),
            ]),
        ),
    ]);

    // The direct service artifact is the expected document.
    let service = direct_service(None);
    let envelope = dispatch(&service, &admin(), "experiment.run", &arguments);
    let result = parse_json(&envelope);
    let artifact = result
        .get("result")
        .and_then(|result| result.get("artifact"))
        .expect("experiment artifact");
    let expected = to_canonical_bytes(artifact, &limits()).expect("artifact bytes");

    // `--out` writes exactly the artifact document, atomically, and prints nothing.
    let written = expect_success(&cli(&[
        "experiment",
        "run",
        "--steps",
        "7",
        "--buttons",
        "3",
        "--checkpoints",
        "0,3,7",
        "--out",
        path_str(&out),
    ]));
    assert!(written.is_empty(), "`--out` leaves stdout empty: {written}");
    let bytes = std::fs::read(&out).expect("read the artifact");
    assert_eq!(bytes, expected);

    // Without `--out` the same document is printed plus one newline.
    let printed = expect_success(&cli(&[
        "experiment",
        "run",
        "--steps",
        "7",
        "--buttons",
        "3",
        "--checkpoints",
        "0,3,7",
    ]));
    assert_eq!(printed.as_bytes(), with_newline(&expected).as_slice());

    // The artifact is deterministic: repeating the request reproduces the bytes.
    let repeated = expect_success(&cli(&[
        "experiment",
        "run",
        "--steps",
        "7",
        "--buttons",
        "3",
        "--checkpoints",
        "0,3,7",
    ]));
    assert_eq!(repeated, printed);
}

// ---------------------------------------------------------------------------
// M6-A25: rejection and exit codes
// ---------------------------------------------------------------------------

#[test]
fn cli_rejections_keep_stdout_empty_and_exit_one() {
    let dir = temp_dir("m6-rejections");
    let tokens = write_tokens(&dir);
    let missing = dir.join("absent-tokens.json");
    let malformed = dir.join("malformed-tokens.json");
    std::fs::write(&malformed, b"{\"schema\":\"chronofork.tokens.v1\"}").expect("write malformed");

    // `serve` requires an explicit token file.
    let stderr = expect_failure(&cli(&["serve", "--bind", "127.0.0.1:0"]));
    assert!(
        stderr.starts_with("chronofork: ") && stderr.contains("requires `--token-file <FILE>`"),
        "{stderr}"
    );

    // A malformed token file refuses to start; it is a service-validated table.
    let stderr = expect_failure(&cli(&[
        "serve",
        "--token-file",
        path_str(&malformed),
        "--bind",
        "127.0.0.1:0",
    ]));
    assert!(stderr.contains("invalid token file"), "{stderr}");

    // An oversized token file is rejected by the bounded read with a clear message.
    let oversized = dir.join("oversized-tokens.json");
    std::fs::write(
        &oversized,
        vec![b'x'; ServiceLimits::default().max_request_bytes + 1],
    )
    .expect("write an oversized token file");
    let stderr = expect_failure(&cli(&[
        "serve",
        "--token-file",
        path_str(&oversized),
        "--bind",
        "127.0.0.1:0",
    ]));
    assert_eq!(
        stderr,
        format!(
            "chronofork: token file `{}` exceeds the {}-byte limit\n",
            oversized.display(),
            ServiceLimits::default().max_request_bytes
        )
    );

    // A missing token file is a local diagnostic for both surface families.
    let stderr = expect_failure(&cli(&[
        "api",
        "health",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&missing),
    ]));
    assert!(stderr.contains("failed to read token file"), "{stderr}");

    // An unknown principal is rejected before any connection is attempted.
    let stderr = expect_failure(&cli(&[
        "api",
        "health",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--principal",
        "nobody",
    ]));
    assert!(stderr.contains("has no principal `nobody`"), "{stderr}");

    // An unsupported base URL is rejected by the transport client.
    let stderr = expect_failure(&cli(&[
        "api",
        "health",
        "--url",
        "not-a-url",
        "--token-file",
        path_str(&tokens),
    ]));
    assert!(stderr.contains("invalid base URL"), "{stderr}");

    // A refused connection is a typed transport failure, not a panic.
    let refused_port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a probe port");
        listener.local_addr().expect("probe address").port()
    };
    let refused_url = format!("http://127.0.0.1:{refused_port}");
    let stderr = expect_failure(&cli(&[
        "api",
        "health",
        "--url",
        &refused_url,
        "--token-file",
        path_str(&tokens),
    ]));
    assert!(stderr.starts_with("chronofork: "), "{stderr}");

    // Invalid `--input` JSON is rejected before the request is sent.
    let stderr = expect_failure(&cli(&[
        "api",
        "call",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--op",
        "capabilities",
        "--input",
        "{\"steps\":}",
    ]));
    assert!(
        stderr.contains("`--input` is not canonical JSON"),
        "{stderr}"
    );

    // Unknown flags are rejected by their own command family.
    let stderr = expect_failure(&cli(&[
        "serve",
        "--token-file",
        path_str(&tokens),
        "--bogus",
    ]));
    assert!(
        stderr.contains("unknown `serve` option `--bogus`"),
        "{stderr}"
    );
    let stderr = expect_failure(&cli(&[
        "api",
        "health",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--op",
        "capabilities",
    ]));
    assert!(
        stderr.contains("unknown `api health` option `--op`"),
        "{stderr}"
    );
    let stderr = expect_failure(&cli(&[
        "mcp",
        "tools",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--tool",
        "capabilities",
    ]));
    assert!(
        stderr.contains("unknown `mcp tools` option `--tool`"),
        "{stderr}"
    );
    let stderr = expect_failure(&cli(&["experiment", "run", "--steps", "1", "--bogus"]));
    assert!(
        stderr.contains("unknown `experiment run` option `--bogus`"),
        "{stderr}"
    );

    // Missing required values keep the same one-line contract.
    let stderr = expect_failure(&cli(&["serve", "--token-file"]));
    assert!(stderr.contains("requires a value"), "{stderr}");
    let stderr = expect_failure(&cli(&[
        "serve",
        "--token-file",
        path_str(&tokens),
        "--workers",
        "0",
    ]));
    assert!(
        stderr.contains("`--workers` must be at least 2"),
        "{stderr}"
    );
    let stderr = expect_failure(&cli(&[
        "serve",
        "--token-file",
        path_str(&tokens),
        "--max-requests",
        "0",
    ]));
    assert!(
        stderr.contains("`--max-requests` must be at least 1"),
        "{stderr}"
    );
    let stderr = expect_failure(&cli(&["api", "health", "--token-file", path_str(&tokens)]));
    assert!(stderr.contains("requires `--url <URL>`"), "{stderr}");
    let stderr = expect_failure(&cli(&[
        "api",
        "call",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
    ]));
    assert!(stderr.contains("requires `--op <NAME>`"), "{stderr}");
    let stderr = expect_failure(&cli(&[
        "api",
        "call",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--op",
        "capabilities",
        "--input",
        "{}",
        "--input-file",
        "-",
    ]));
    assert!(
        stderr.contains("`--input` and `--input-file` are mutually exclusive"),
        "{stderr}"
    );
    let invalid_input_file = dir.join("invalid-input.json");
    std::fs::write(&invalid_input_file, b"{\"steps\":}").expect("write invalid input");
    let stderr = expect_failure(&cli(&[
        "api",
        "call",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--op",
        "capabilities",
        "--input-file",
        path_str(&invalid_input_file),
    ]));
    assert!(
        stderr.contains("`--input-file` is not canonical JSON"),
        "{stderr}"
    );
    let stderr = expect_failure(&cli(&[
        "mcp",
        "call",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
    ]));
    assert!(stderr.contains("requires `--tool <NAME>`"), "{stderr}");
    let stderr = expect_failure(&cli(&[
        "mcp",
        "call",
        "--url",
        "http://127.0.0.1:1",
        "--token-file",
        path_str(&tokens),
        "--tool",
        "capabilities",
        "--arguments",
        "{",
    ]));
    assert!(
        stderr.contains("`--arguments` is not canonical JSON"),
        "{stderr}"
    );
    let stderr = expect_failure(&cli(&["experiment", "run"]));
    assert!(stderr.contains("requires `--steps <N>`"), "{stderr}");

    // The local experiment command reports the service's typed validation with the
    // base human prefix: a local diagnostic never carries a typed error code.
    let stderr = expect_failure(&cli(&["experiment", "run", "--steps", "0"]));
    assert_eq!(stderr, "chronofork: `steps` must be at least 1\n");
    let stderr = expect_failure(&cli(&[
        "experiment",
        "run",
        "--steps",
        "2",
        "--checkpoints",
        "3",
    ]));
    assert_eq!(
        stderr,
        "chronofork: `checkpoints` entries must be at most `steps`\n"
    );
}

// ---------------------------------------------------------------------------
// M6-R2/R3: human diagnostics and truncation
// ---------------------------------------------------------------------------

#[test]
fn human_session_diagnostics_use_the_base_chronofork_prefix() {
    let dir = temp_dir("m6-human-diagnostics");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).expect("create the durable root");
    expect_success(&cli(&[
        "session",
        "save",
        "--root",
        path_str(&root),
        "--steps",
        "0",
    ]));

    // Unknown durable key: the pre-M6 wording, now without a typed-code prefix.
    let stderr = expect_failure(&cli(&[
        "session",
        "load",
        "--root",
        path_str(&root),
        "--session",
        "9999",
    ]));
    assert_eq!(stderr, "chronofork: unknown durable session key: 9999\n");

    // A missing root is never initialized by a read command.
    let missing = dir.join("missing");
    let stderr = expect_failure(&cli(&["session", "list", "--root", path_str(&missing)]));
    assert_eq!(stderr, "chronofork: storage root is not initialized\n");

    // A non-empty foreign directory is refused, not adopted.
    let foreign = dir.join("foreign");
    std::fs::create_dir_all(&foreign).expect("create the foreign directory");
    std::fs::write(foreign.join("data.bin"), b"not a storage root").expect("write foreign data");
    let stderr = expect_failure(&cli(&["session", "save", "--root", path_str(&foreign)]));
    assert_eq!(stderr, "chronofork: storage root is not empty\n");

    // Advisory names are validated once, by the service catalog.
    let stderr = expect_failure(&cli(&[
        "session",
        "save",
        "--root",
        path_str(&root),
        "--name",
        "bad\nname",
    ]));
    assert_eq!(
        stderr,
        "chronofork: `name` must not contain control characters\n"
    );

    // A contended writer names the durable session and keeps the base prefix.
    let repository = SnapshotRepository::open(&root).expect("open the storage root");
    let lock = repository
        .acquire_writer_lock(1)
        .expect("hold the real writer lock");
    let stderr = expect_failure(&cli(&[
        "session",
        "save",
        "--root",
        path_str(&root),
        "--session",
        "1",
        "--steps",
        "1",
    ]));
    assert_eq!(
        stderr,
        "chronofork: durable session 1 writer lock is held by owner\n"
    );
    drop(lock);

    // A corrupt record is reported as one path-free anomaly label on a successful list.
    let corrupt = dir.join("corrupt");
    std::fs::create_dir_all(&corrupt).expect("create the corrupt root");
    expect_success(&cli(&[
        "session",
        "save",
        "--root",
        path_str(&corrupt),
        "--steps",
        "0",
    ]));
    let metadata = corrupt
        .join("v1")
        .join("sessions")
        .join("session-1")
        .join("session.cfmeta");
    let mut bytes = std::fs::read(&metadata).expect("read the session metadata");
    bytes[0] ^= 0xff;
    std::fs::write(&metadata, bytes).expect("tamper with the session metadata");
    let output = cli(&["session", "list", "--root", path_str(&corrupt)]);
    let (stdout, stderr) = streams(&output);
    assert!(
        output.status.success(),
        "a list reports anomalies without failing: {stderr}"
    );
    assert!(stdout.is_empty(), "no committed entry remains: {stdout}");
    assert_eq!(
        stderr,
        "chronofork: storage anomaly: corrupt storage metadata\n"
    );
}

#[test]
fn human_session_list_reports_truncation_on_stderr() {
    let dir = temp_dir("m6-truncation");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).expect("create the durable root");
    let service = direct_service(Some(root.clone()));
    // 129 sessions with one snapshot each are 258 listing entries, above the
    // documented 256-entry budget of the default `ServiceLimits`.
    for _ in 0..129 {
        dispatch(
            &service,
            &admin(),
            "store.save",
            &object([("steps", JsonValue::int(0))]),
        );
    }

    let output = cli(&["session", "list", "--root", path_str(&root)]);
    let (stdout, stderr) = streams(&output);
    assert!(
        output.status.success(),
        "a truncated listing still succeeds: {stderr}"
    );
    assert_eq!(
        stderr,
        "chronofork: storage listing truncated at 256 entries\n"
    );
    assert_eq!(
        stdout.matches("record=").count(),
        256,
        "the documented budget still prints 256 records"
    );

    // `--json` carries `truncated` in the canonical envelope and stays quiet on stderr.
    let output = cli(&["session", "list", "--root", path_str(&root), "--json"]);
    let (stdout, stderr) = streams(&output);
    assert!(output.status.success());
    assert!(stderr.is_empty(), "--json is machine output only: {stderr}");
    let envelope = parse_json(stdout.trim_end().as_bytes());
    assert_eq!(
        envelope
            .get("result")
            .and_then(|result| result.get("truncated"))
            .and_then(JsonValue::as_bool),
        Some(true)
    );
}

// ---------------------------------------------------------------------------
// Usage and help coverage
// ---------------------------------------------------------------------------

#[test]
fn usage_and_help_cover_the_new_surfaces() {
    let help = expect_success(&cli(&["help"]));
    for needle in [
        "chronofork serve --token-file <FILE>",
        "chronofork api health|tools|call",
        "chronofork mcp tools|call",
        "chronofork experiment run --steps <N>",
        "[--json]",
    ] {
        assert!(
            help.contains(needle),
            "top-level help is missing `{needle}`:\n{help}"
        );
    }

    let serve_help = expect_success(&cli(&["serve", "--help"]));
    for needle in [
        "chronofork serve",
        "--token-file <FILE>",
        "--bind <ADDR>",
        "--store-root <DIR>",
        "--artifacts <DIR>",
        "--rom <FILE>",
        "--max-requests <N>",
        "record=serve",
    ] {
        assert!(
            serve_help.contains(needle),
            "serve help is missing `{needle}`:\n{serve_help}"
        );
    }

    let api_help = expect_success(&cli(&["api", "--help"]));
    for needle in [
        "chronofork api health",
        "chronofork api tools",
        "chronofork api call",
        "--principal <NAME>",
        "--input-file",
        "stdout is success-only",
    ] {
        assert!(
            api_help.contains(needle),
            "api help is missing `{needle}`:\n{api_help}"
        );
    }

    let mcp_help = expect_success(&cli(&["mcp", "--help"]));
    for needle in [
        "chronofork mcp tools",
        "chronofork mcp call",
        "--tool <NAME>",
    ] {
        assert!(
            mcp_help.contains(needle),
            "mcp help is missing `{needle}`:\n{mcp_help}"
        );
    }

    let experiment_help = expect_success(&cli(&["experiment", "--help"]));
    for needle in [
        "chronofork experiment run",
        "--steps <N>",
        "--buttons <U32>",
        "--checkpoints",
        "--out <FILE>",
    ] {
        assert!(
            experiment_help.contains(needle),
            "experiment help is missing `{needle}`:\n{experiment_help}"
        );
    }

    // Group commands without a subcommand are usage errors, not silent successes.
    for group in ["api", "mcp", "experiment"] {
        let stderr = expect_failure(&cli(&[group]));
        assert!(
            stderr.contains("requires a subcommand"),
            "`{group}` must require a subcommand:\n{stderr}"
        );
    }
}
