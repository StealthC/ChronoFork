//! Typed clients used by the CLI and by parity tests.
//!
//! Both clients are thin: they frame one bounded HTTP/1.1 request, parse the status,
//! headers and `Content-Length`-delimited body, and hand the canonical bytes back
//! unchanged. A non-success status is [`TransportError::Http`] carrying the response body,
//! so the caller can print the typed envelope the daemon produced instead of a paraphrase.

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, parse, to_canonical_bytes};

use crate::error::TransportError;
use crate::mcp::{PROTOCOL_VERSION, ToolCallResult};

/// Connect timeout for one client request.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Read and write timeout for one client request.
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum accepted response head size in bytes.
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// Read chunk size used while assembling a response.
const CHUNK_BYTES: usize = 4096;

/// A bounded HTTP/1.1 client for one daemon.
///
/// `Debug` redacts the configured bearer token.
#[derive(Clone)]
pub struct HttpClient {
    base: String,
    token: String,
}

impl fmt::Debug for HttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpClient")
            .field("base", &self.base)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl HttpClient {
    /// Creates a client for `http://host:port` with one bearer token.
    #[must_use]
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        let base = base.into();
        Self {
            base: base.trim_end_matches('/').to_owned(),
            token: token.into(),
        }
    }

    /// Returns the configured base URL without a trailing slash.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Returns the configured bearer token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Fetches `GET /health` and returns the canonical response body.
    ///
    /// No bearer token is attached: `GET /health` is the one unauthenticated route.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::InvalidUrl`] for an unsupported base URL,
    /// [`TransportError::Io`] for a socket failure, [`TransportError::Protocol`] for a
    /// malformed or oversized response and [`TransportError::Http`] for a non-2xx status.
    pub fn health(&self) -> Result<Vec<u8>, TransportError> {
        self.request("GET", "/health", None, false)
    }

    /// Fetches `GET /v1/tools` and returns the canonical response body.
    ///
    /// # Errors
    ///
    /// As [`Self::health`]; a missing or unknown token surfaces as
    /// [`TransportError::Http`] with status `401`.
    pub fn tools(&self) -> Result<Vec<u8>, TransportError> {
        self.request("GET", "/v1/tools", None, true)
    }

    /// Calls one operation and returns the canonical response body.
    ///
    /// # Errors
    ///
    /// As [`Self::tools`]. A typed operation failure is
    /// [`TransportError::Http`] whose body is the `chronofork.error.v1` envelope.
    pub fn call(&self, op: &str, input: &JsonValue) -> Result<Vec<u8>, TransportError> {
        let document = JsonValue::object([("op", JsonValue::string(op)), ("input", input.clone())]);
        let body = to_canonical_bytes(&document, &ServiceLimits::default())?;
        self.call_raw(&body)
    }

    /// Sends one already-serialized canonical request document to `POST /v1/ops`.
    ///
    /// # Errors
    ///
    /// As [`Self::tools`].
    pub fn call_raw(&self, body: &[u8]) -> Result<Vec<u8>, TransportError> {
        self.request("POST", "/v1/ops", Some(body), true)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        authenticate: bool,
    ) -> Result<Vec<u8>, TransportError> {
        let authority = self.authority()?;
        let address = self.resolve(&authority)?;
        let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\n");
        if body.is_some() {
            request.push_str("Content-Type: application/json\r\n");
        }
        if let Some(body) = body {
            request.push_str("Content-Length: ");
            request.push_str(&body.len().to_string());
            request.push_str("\r\n");
        }
        if authenticate && !self.token.is_empty() {
            request.push_str("Authorization: Bearer ");
            request.push_str(&self.token);
            request.push_str("\r\n");
        }
        request.push_str("Connection: close\r\n\r\n");
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        let _ = stream.set_nodelay(true);
        stream.write_all(request.as_bytes())?;
        if let Some(body) = body {
            stream.write_all(body)?;
        }
        stream.flush()?;
        read_response(&mut stream)
    }

    fn authority(&self) -> Result<String, TransportError> {
        let Some(scheme) = self.base.get(..7) else {
            return Err(TransportError::InvalidUrl(self.base.clone()));
        };
        if !scheme.eq_ignore_ascii_case("http://") {
            return Err(TransportError::InvalidUrl(self.base.clone()));
        }
        let Some(authority) = self.base.get(7..) else {
            return Err(TransportError::InvalidUrl(self.base.clone()));
        };
        let unsupported = authority.is_empty()
            || authority.contains('/')
            || authority.contains('?')
            || authority.contains('#');
        if unsupported {
            return Err(TransportError::InvalidUrl(self.base.clone()));
        }
        Ok(authority.to_owned())
    }

    fn resolve(&self, authority: &str) -> Result<SocketAddr, TransportError> {
        let mut addresses = authority
            .to_socket_addrs()
            .map_err(|_| TransportError::InvalidUrl(self.base.clone()))?;
        addresses
            .next()
            .ok_or_else(|| TransportError::InvalidUrl(self.base.clone()))
    }
}

/// An MCP client over Streamable HTTP.
#[derive(Clone, Debug)]
pub struct McpClient {
    http: HttpClient,
}

impl McpClient {
    /// Creates an MCP client for one daemon.
    #[must_use]
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            http: HttpClient::new(base, token),
        }
    }

    /// Returns the underlying HTTP client.
    #[must_use]
    pub const fn http(&self) -> &HttpClient {
        &self.http
    }

    /// Performs the `initialize` request and returns the `result` object.
    ///
    /// # Errors
    ///
    /// As [`HttpClient::tools`], plus [`TransportError::Protocol`] when the daemon answers
    /// with a JSON-RPC error or a malformed result.
    pub fn initialize(&self) -> Result<JsonValue, TransportError> {
        let params = JsonValue::object([
            ("protocolVersion", JsonValue::string(PROTOCOL_VERSION)),
            ("capabilities", JsonValue::Object(Default::default())),
            (
                "clientInfo",
                JsonValue::object([
                    ("name", JsonValue::string("chronofork")),
                    ("version", JsonValue::string(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ]);
        self.rpc("initialize", Some(params))
    }

    /// Fetches `tools/list` and returns the `result` object.
    ///
    /// # Errors
    ///
    /// As [`Self::initialize`].
    pub fn list_tools(&self) -> Result<JsonValue, TransportError> {
        self.rpc("tools/list", None)
    }

    /// Calls one tool and returns its text content and error flag.
    ///
    /// # Errors
    ///
    /// As [`Self::initialize`]. A typed operation error is *not* a transport failure: it
    /// is returned as [`ToolCallResult`] with `is_error` set and the error envelope in
    /// `text`.
    pub fn call_tool(
        &self,
        name: &str,
        arguments: &JsonValue,
    ) -> Result<ToolCallResult, TransportError> {
        let params = JsonValue::object([
            ("name", JsonValue::string(name)),
            ("arguments", arguments.clone()),
        ]);
        let result = self.rpc("tools/call", Some(params))?;
        let content = result
            .get("content")
            .and_then(JsonValue::as_array)
            .and_then(|entries| entries.first())
            .ok_or_else(|| {
                TransportError::Protocol("tools/call result has no content".to_owned())
            })?;
        let text = content
            .get("text")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| TransportError::Protocol("tools/call content is not text".to_owned()))?;
        let is_error = result
            .get("isError")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        Ok(ToolCallResult {
            text: text.to_owned(),
            is_error,
        })
    }

    fn rpc(&self, method: &str, params: Option<JsonValue>) -> Result<JsonValue, TransportError> {
        let mut members = vec![
            ("jsonrpc", JsonValue::string("2.0")),
            ("id", JsonValue::int(1)),
            ("method", JsonValue::string(method)),
        ];
        if let Some(params) = params {
            members.push(("params", params));
        }
        let body = to_canonical_bytes(&JsonValue::object(members), &ServiceLimits::default())?;
        let response = self.http.request("POST", "/mcp", Some(&body), true)?;
        let document = parse(&response, &ServiceLimits::default())?;
        let object = document.as_object().ok_or_else(|| {
            TransportError::Protocol("json-rpc response is not an object".to_owned())
        })?;
        if let Some(error) = object.get("error") {
            let code = error.get("code").and_then(JsonValue::as_i64).unwrap_or(0);
            let message = error
                .get("message")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown error");
            return Err(TransportError::Protocol(format!(
                "json-rpc error {code}: {message}"
            )));
        }
        object
            .get("result")
            .cloned()
            .ok_or_else(|| TransportError::Protocol("json-rpc response has no result".to_owned()))
    }
}

fn read_response(stream: &mut TcpStream) -> Result<Vec<u8>, TransportError> {
    let mut buffer: Vec<u8> = Vec::with_capacity(CHUNK_BYTES);
    let mut chunk = [0u8; CHUNK_BYTES];
    let head_end = loop {
        if let Some(end) = find_terminator(&buffer) {
            break end;
        }
        if buffer.len() > MAX_HEAD_BYTES {
            return Err(TransportError::Protocol(
                "response head exceeds the wire limits".to_owned(),
            ));
        }
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(TransportError::Protocol(
                    "connection closed before the response head".to_owned(),
                ));
            }
            Ok(count) => buffer.extend_from_slice(chunk.get(..count).unwrap_or_default()),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(TransportError::Io(error)),
        }
    };
    let head_bytes = buffer.get(..head_end).unwrap_or_default();
    let head = std::str::from_utf8(head_bytes)
        .map_err(|_| TransportError::Protocol("response head is not valid UTF-8".to_owned()))?;
    let mut lines = head.split("\r\n");
    let status = parse_status(lines.next().unwrap_or_default())?;
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(TransportError::Protocol(
                "malformed response header".to_owned(),
            ));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = parse_length(value.trim_matches([' ', '\t']));
        }
    }
    let Some(content_length) = content_length else {
        return Err(TransportError::Protocol(
            "response is missing Content-Length".to_owned(),
        ));
    };
    if content_length > response_limit() {
        return Err(TransportError::Protocol(
            "response exceeds the configured byte limit".to_owned(),
        ));
    }
    let mut body = buffer.get(head_end + 4..).unwrap_or_default().to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let want = remaining.min(chunk.len());
        let slice = match chunk.get_mut(..want) {
            Some(slice) => slice,
            None => &mut [],
        };
        match stream.read(slice) {
            Ok(0) => {
                return Err(TransportError::Protocol(
                    "response body is truncated".to_owned(),
                ));
            }
            Ok(count) => body.extend_from_slice(slice.get(..count).unwrap_or_default()),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(TransportError::Io(error)),
        }
    }
    body.truncate(content_length);
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(TransportError::Http { status, body })
    }
}

fn find_terminator(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_status(line: &str) -> Result<u16, TransportError> {
    let mut parts = line.split(' ');
    let version = parts.next().unwrap_or_default();
    let status = parts.next().unwrap_or_default();
    if !version.eq_ignore_ascii_case("HTTP/1.1") {
        return Err(TransportError::Protocol(
            "response is not HTTP/1.1".to_owned(),
        ));
    }
    status
        .parse::<u16>()
        .map_err(|_| TransportError::Protocol("malformed response status".to_owned()))
}

fn parse_length(raw: &str) -> Option<usize> {
    if raw.is_empty() || raw.len() > 20 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = raw.bytes().try_fold(0_u64, |value, byte| {
        value
            .checked_mul(10)?
            .checked_add(u64::from(byte.wrapping_sub(b'0')))
    })?;
    usize::try_from(value).ok()
}

fn response_limit() -> usize {
    ServiceLimits::default().max_response_bytes
}

#[cfg(test)]
mod debug_redaction {
    use super::{HttpClient, McpClient};

    const TOKEN: &str = "operator-token-0001";

    #[test]
    fn http_client_debug_never_renders_the_bearer_token() {
        let client = HttpClient::new("http://127.0.0.1:9", TOKEN);
        let rendered = format!("{client:?}");
        assert!(rendered.contains("http://127.0.0.1:9"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(!rendered.contains(TOKEN), "{rendered}");
    }

    #[test]
    fn mcp_client_debug_never_renders_the_bearer_token() {
        let client = McpClient::new("http://127.0.0.1:9", TOKEN);
        let rendered = format!("{client:?}");
        assert!(!rendered.contains(TOKEN), "{rendered}");
    }
}
