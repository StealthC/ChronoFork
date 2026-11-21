#![allow(clippy::expect_used)]

//! Acceptance tests for the bounded typed clients.
//!
//! They run against a live in-process server (not a mock) and against hand-written raw
//! responses where the server must not be able to produce the failure under test.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;

use chronofork_service::json::JsonValue;
use chronofork_service::{
    AppService, Principal, Scope, ServiceConfig, ServiceLimits, TokenTable, parse_token_file,
};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{
    HttpClient, McpClient, ServerConfig, ServerHandle, TransportError, serve,
};

const OPERATOR_TOKEN: &str = "operator-token-0001";

const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
 {"name":"operator","token":"operator-token-0001","scopes":["read","write"]}]}"#;

fn tokens() -> TokenTable {
    parse_token_file(TOKENS.as_bytes()).expect("token table")
}

fn limits() -> ServiceLimits {
    ServiceLimits::default()
}

fn operator() -> Principal {
    Principal::new("operator", [Scope::Read, Scope::Write])
}

fn service() -> AppService {
    AppService::new(ServiceConfig {
        factory: Arc::new(SyntheticBackendFactory::default()),
        storage_root: None,
        artifacts_root: None,
        limits: limits(),
    })
    .expect("service")
}

fn start() -> (ServerHandle, String) {
    let server = serve(
        ServerConfig {
            tokens: tokens(),
            ..ServerConfig::default()
        },
        Arc::new(service()),
    )
    .expect("server");
    let base = format!("http://{}", server.local_addr());
    (server, base)
}

fn input(text: &str) -> JsonValue {
    chronofork_service::json::parse(text.as_bytes(), &limits()).expect("input JSON")
}

// ---------------------------------------------------------------------------
// A raw one-shot server for failures the daemon cannot produce
// ---------------------------------------------------------------------------

fn raw_server(response: Vec<u8>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("address");
    thread::spawn(move || {
        if let Ok((mut stream, _peer)) = listener.accept() {
            let mut sink = [0u8; 1024];
            let _ = stream.read(&mut sink);
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });
    addr
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

#[test]
fn the_client_reaches_every_route_of_a_live_server() {
    let (server, base) = start();
    let client = HttpClient::new(base.clone(), OPERATOR_TOKEN);

    assert_eq!(client.base(), base);
    assert_eq!(client.token(), OPERATOR_TOKEN);

    let health = client.health().expect("health");
    assert_eq!(
        health,
        br#"{"schema":"chronofork.health.v1","status":"ok"}"#.to_vec()
    );

    let tools = client.tools().expect("tools");
    let document = chronofork_service::json::parse(&tools, &limits()).expect("tools document");
    assert_eq!(
        document.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.tools.v1")
    );
    assert_eq!(
        document
            .get("tools")
            .and_then(JsonValue::as_array)
            .map(<[JsonValue]>::len),
        Some(20)
    );

    let direct = service()
        .dispatch(&operator(), br#"{"op":"capabilities","input":{}}"#)
        .expect("direct dispatch");
    let call = client
        .call("capabilities", &input("{}"))
        .expect("operation call");
    assert_eq!(call, direct, "the client returns the dispatch bytes");

    let raw = client
        .call_raw(br#"{"op":"capabilities","input":{}}"#)
        .expect("raw call");
    assert_eq!(raw, direct, "`call` and `call_raw` agree byte for byte");

    drop(server);
}

#[test]
fn a_trailing_slash_is_normalized_once() {
    let (server, base) = start();
    let client = HttpClient::new(format!("{base}/"), OPERATOR_TOKEN);
    assert_eq!(client.base(), base);
    assert_eq!(client.health().expect("health").len(), 47);
    drop(server);
}

#[test]
fn non_success_statuses_surface_as_typed_http_errors_with_the_body() {
    let (server, base) = start();
    let client = HttpClient::new(base.clone(), OPERATOR_TOKEN);

    let error = client
        .call("session.inspect", &input(r#"{"session":99}"#))
        .expect_err("unknown session");
    let body = match error {
        TransportError::Http { status, body } => {
            assert_eq!(status, 404);
            body
        }
        other => unreachable!("expected a transport error variant, got {other:?}"),
    };
    let envelope = chronofork_service::json::parse(&body, &limits()).expect("envelope");
    assert_eq!(
        envelope.get("schema").and_then(JsonValue::as_str),
        Some("chronofork.error.v1")
    );
    assert_eq!(
        envelope
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("not_found")
    );

    // Malformed request bytes are the daemon's 400, not a client-side rewrite.
    let error = client.call_raw(b"not json").expect_err("malformed body");
    match error {
        TransportError::Http { status, body } => {
            assert_eq!(status, 400);
            let envelope =
                chronofork_service::json::parse(&body, &limits()).expect("error envelope");
            assert_eq!(
                envelope
                    .get("error")
                    .and_then(|error| error.get("code"))
                    .and_then(JsonValue::as_str),
                Some("invalid_request")
            );
        }
        other => unreachable!("expected a transport error variant, got {other:?}"),
    }

    // An unknown token is 401 with the transport envelope.
    let unauthorized = HttpClient::new(base, "unknown-token-000001");
    let error = unauthorized.tools().expect_err("unknown token");
    match error {
        TransportError::Http { status, body } => {
            assert_eq!(status, 401);
            let envelope =
                chronofork_service::json::parse(&body, &limits()).expect("error envelope");
            assert_eq!(
                envelope
                    .get("error")
                    .and_then(|error| error.get("code"))
                    .and_then(JsonValue::as_str),
                Some("unauthorized")
            );
        }
        other => unreachable!("expected a transport error variant, got {other:?}"),
    }
    drop(server);
}

#[test]
fn unsupported_base_urls_are_rejected_before_connecting() {
    for base in [
        "ftp://127.0.0.1:8080",
        "http://",
        "http://127.0.0.1:8080/v1",
        "http://127.0.0.1:8080?x=1",
        "127.0.0.1:8080",
    ] {
        let error = HttpClient::new(base, OPERATOR_TOKEN)
            .health()
            .expect_err("unsupported base URL");
        assert!(
            matches!(error, TransportError::InvalidUrl(_)),
            "{base} is rejected as a URL"
        );
    }
}

#[test]
fn a_refused_connection_is_an_io_error() {
    // Bind and release a port so the address is almost certainly closed afterwards.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("address");
    drop(listener);
    let error = HttpClient::new(format!("http://{addr}"), OPERATOR_TOKEN)
        .health()
        .expect_err("connection refused");
    assert!(
        matches!(error, TransportError::Io(_)),
        "unexpected: {error:?}"
    );
}

#[test]
fn an_oversized_response_is_rejected_by_the_client_bound() {
    let limit = ServiceLimits::default().max_response_bytes;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        limit + 1
    );
    let addr = raw_server(response.into_bytes());
    let error = HttpClient::new(format!("http://{addr}"), OPERATOR_TOKEN)
        .health()
        .expect_err("oversized response");
    match error {
        TransportError::Protocol(message) => {
            assert!(message.contains("byte limit"), "{message}");
        }
        other => unreachable!("expected a transport error variant, got {other:?}"),
    }
}

#[test]
fn a_truncated_response_is_a_protocol_error() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc".to_vec();
    let addr = raw_server(response);
    let error = HttpClient::new(format!("http://{addr}"), OPERATOR_TOKEN)
        .health()
        .expect_err("truncated response");
    match error {
        TransportError::Protocol(message) => {
            assert!(message.contains("truncated"), "{message}");
        }
        other => unreachable!("expected a transport error variant, got {other:?}"),
    }
}

#[test]
fn a_response_head_beyond_the_client_bound_is_rejected() {
    let mut response = b"HTTP/1.1 200 OK\r\n".to_vec();
    response.extend(std::iter::repeat_n(b'X', 70_000));
    let addr = raw_server(response);
    let error = HttpClient::new(format!("http://{addr}"), OPERATOR_TOKEN)
        .health()
        .expect_err("oversized response head");
    match error {
        TransportError::Protocol(message) => {
            assert!(message.contains("head"), "{message}");
        }
        other => unreachable!("expected a transport error variant, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// MCP client
// ---------------------------------------------------------------------------

#[test]
fn the_mcp_client_initializes_lists_tools_and_calls_one() {
    let (server, base) = start();
    let client = McpClient::new(base.clone(), OPERATOR_TOKEN);
    assert_eq!(client.http().base(), base);

    let initialized = client.initialize().expect("initialize");
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

    let listed = client.list_tools().expect("tools/list");
    let tools = listed
        .get("tools")
        .and_then(JsonValue::as_array)
        .expect("tools");
    assert_eq!(tools.len(), 20);
    assert_eq!(
        tools
            .first()
            .and_then(|tool| tool.get("name"))
            .and_then(JsonValue::as_str),
        Some("capabilities")
    );

    let direct = service()
        .dispatch(&operator(), br#"{"op":"capabilities","input":{}}"#)
        .expect("direct dispatch");
    let result = client
        .call_tool("capabilities", &input("{}"))
        .expect("tools/call");
    assert!(!result.is_error);
    assert_eq!(
        result.text.as_bytes(),
        direct.as_slice(),
        "the client returns the exact envelope text"
    );

    // A typed error is a result, not a transport failure.
    let failed = client
        .call_tool("session.inspect", &input(r#"{"session":99}"#))
        .expect("typed error result");
    assert!(failed.is_error);
    let envelope =
        chronofork_service::json::parse(failed.text.as_bytes(), &limits()).expect("envelope");
    assert_eq!(
        envelope
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(JsonValue::as_str),
        Some("not_found")
    );
    drop(server);
}

#[test]
fn the_mcp_client_surfaces_a_json_rpc_error_as_a_protocol_error() {
    let (server, base) = start();
    let client = McpClient::new(base, OPERATOR_TOKEN);
    let error = client
        .call_tool("not.a.tool", &input("{}"))
        .expect_err("unknown tool");
    match error {
        TransportError::Protocol(message) => {
            assert!(message.contains("-32602"), "{message}");
        }
        other => unreachable!("expected a transport error variant, got {other:?}"),
    }
    drop(server);
}
