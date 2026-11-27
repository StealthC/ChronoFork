//! Bounded HTTP/1.1 server over `std::net` and `std::thread`.
//!
//! The server is deliberately a small, fixed protocol subset (architecture §4): one
//! request per connection (`Connection: close`), no keep-alive, no chunked request
//! bodies, a strict request line and bounded headers. A fixed worker pool is fed by a
//! bounded accept queue, so an idle but slow client cannot make the daemon spawn threads
//! without limit; the queue is bounded by leaving excess connections in the operating
//! system backlog instead of buffering them. Each queued connection carries the instant
//! it was accepted, and its absolute pre-auth deadline is measured from there, so queue
//! residence consumes the same budget.
//!
//! The server owns no session state and reimplements no service rule: `POST /v1/ops`
//! hands the body to [`AppService::dispatch`] and returns the service envelope bytes
//! unchanged. Order of checks, from cheapest to most expensive, is: query string,
//! `Origin` (v1 has no browser-facing surface, so any `Origin`-bearing request is
//! refused), route, method, framing (`Content-Length`, `Content-Type`, body bound),
//! bearer token, body read, dispatch. That order is observable and therefore documented
//! here.

use std::collections::VecDeque;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chronofork_service::ApiVersion;
use chronofork_service::json::JsonValue;
use chronofork_service::{
    AppService, ErrorCode, Principal, Scope, ServiceError, ServiceLimits, TokenTable,
};

use crate::admission::{AdmissionGate, AdmissionGuard, AdmissionLimits, is_long_operation};
use crate::error::TransportError;
use crate::mcp;
use crate::wire::{self, Response, WireCode};

/// Maximum accepted request-line length in bytes.
const MAX_REQUEST_LINE_BYTES: usize = 8192;
/// Maximum accepted number of header lines.
const MAX_HEADER_LINES: usize = 64;
/// Maximum accepted length of one header line in bytes.
const MAX_HEADER_LINE_BYTES: usize = 8192;
/// Maximum accepted size of the whole request head (request line, headers, terminator).
const MAX_HEAD_BYTES: usize =
    MAX_REQUEST_LINE_BYTES + 2 + MAX_HEADER_LINES * (MAX_HEADER_LINE_BYTES + 2) + 2;
/// Maximum number of connections waiting for a worker.
const ACCEPT_QUEUE_DEPTH: usize = 16;
/// How long the accept loop waits before it re-checks the shutdown flag.
const ACCEPT_POLL: Duration = Duration::from_millis(5);
/// How long a worker waits for a connection before it re-checks the shutdown flag.
const WORKER_POLL: Duration = Duration::from_millis(20);
/// Read chunk size used while assembling a request head or body.
const CHUNK_BYTES: usize = 1024;
/// Maximum number of unread request bytes discarded after a response is written.
const DRAIN_BYTES: usize = 64 * 1024;
/// Read timeout used while discarding those bytes.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(5);
/// Wall-clock bound on the discard phase, so a trickling peer cannot hold a worker.
const DRAIN_DEADLINE: Duration = Duration::from_millis(100);
/// Default absolute bound from `accept` to a complete request head (ADR-0014 §5).
const DEFAULT_PRE_AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Wire-level server configuration.
///
/// `Debug` redacts the bearer-token table so a configuration record can be logged
/// without exposing credentials.
#[derive(Clone)]
pub struct ServerConfig {
    /// Address to bind. Tests may pass port `0` for an ephemeral port.
    pub bind: SocketAddr,
    /// Fixed number of connection workers.
    pub workers: usize,
    /// Maximum requests served before shutdown, for bounded process tests.
    pub max_requests: Option<u64>,
    /// Per-connection read and write timeout.
    pub io_timeout: Duration,
    /// Absolute bound on the time from `accept` to a complete request head.
    ///
    /// Distinct from [`Self::io_timeout`], which bounds one read: a peer that trickles
    /// the head more slowly than `io_timeout` per byte would otherwise hold a worker
    /// indefinitely. The deadline is enforced in `read_head`, so pre-authentication
    /// slowloris cannot exhaust the fixed worker pool (ADR-0014 §5). It is stamped when
    /// `accept` returns and before the connection enters the bounded queue, so queue
    /// residence consumes the same budget instead of resetting it at dispatch. It ends
    /// once the head is complete; body reads are authenticated and use `io_timeout`.
    pub pre_auth_timeout: Duration,
    /// Maximum accepted request body size (`413` above it).
    ///
    /// This is the wire bound; the service codec independently enforces its own
    /// `ServiceLimits::max_request_bytes` on the JSON document. Keep the wire bound at or
    /// above the service bound so a document is never rejected by the transport first.
    pub max_request_bytes: usize,
    /// Bearer tokens accepted on the authenticated routes.
    ///
    /// An empty table authenticates nobody, so every authenticated route answers `401`;
    /// the CLI refuses to start without a validated token file.
    pub tokens: TokenTable,
    /// Maximum long runs admitted at once across all principals.
    ///
    /// Defaults to `workers - 1`, which is what reserves at least one worker for the
    /// control path (`cancel`/`progress`/`list`/`capabilities`) while `N - 1` long runs
    /// are in flight (ADR-0016 §1, F4/K10). A request above the bound is rejected with a
    /// typed `limit_exceeded` **before** its long work starts.
    pub max_long_runs_global: Option<usize>,
    /// Maximum long runs admitted at once for one principal.
    ///
    /// Defaults to 1, so a single client cannot consume the reserved control capacity.
    pub max_long_runs_per_principal: Option<usize>,
}

impl ServerConfig {
    /// Returns the effective global long-run bound (`workers - 1` by default).
    fn long_run_limit(&self) -> usize {
        self.max_long_runs_global
            .unwrap_or_else(|| self.workers.saturating_sub(1))
    }

    /// Returns the effective per-principal long-run bound.
    fn per_principal_limit(&self) -> usize {
        self.max_long_runs_per_principal.unwrap_or(1)
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerConfig")
            .field("bind", &self.bind)
            .field("workers", &self.workers)
            .field("max_requests", &self.max_requests)
            .field("io_timeout", &self.io_timeout)
            .field("pre_auth_timeout", &self.pre_auth_timeout)
            .field("max_request_bytes", &self.max_request_bytes)
            .field("max_long_runs_global", &self.max_long_runs_global)
            .field(
                "max_long_runs_per_principal",
                &self.max_long_runs_per_principal,
            )
            .field("tokens", &"<redacted>")
            .finish()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            workers: 4,
            max_requests: None,
            // The server must tolerate a request whose handler runs to the service's
            // own deadline (ADR-0016 §1, V3-1): a 5 s socket timeout would kill a long
            // run mid-flight and turn a bounded operation into a dropped connection.
            // The service deadline, not this socket timeout, bounds the operation.
            io_timeout: Duration::from_secs(45),
            pre_auth_timeout: DEFAULT_PRE_AUTH_TIMEOUT,
            max_request_bytes: 1024 * 1024,
            tokens: TokenTable::default(),
            max_long_runs_global: None,
            max_long_runs_per_principal: None,
        }
    }
}

/// The routes of architecture §4.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Route {
    Health,
    Tools,
    Ops,
    Mcp,
}

impl Route {
    fn parse(path: &str) -> Option<Self> {
        match path {
            "/health" => Some(Self::Health),
            "/v1/tools" => Some(Self::Tools),
            "/v1/ops" => Some(Self::Ops),
            "/mcp" => Some(Self::Mcp),
            _ => None,
        }
    }

    const fn allowed_method(self) -> &'static str {
        match self {
            Self::Health | Self::Tools => "GET",
            Self::Ops | Self::Mcp => "POST",
        }
    }
}

/// A parsed request head plus any body bytes read together with it.
#[derive(Debug)]
struct Head {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    leftover: Vec<u8>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// The outcome of reading one request head.
enum HeadOutcome {
    /// The peer closed before sending any byte.
    Closed,
    /// The request was rejected before it could be routed.
    Rejected(Response),
    /// A well-formed head; the body (when the route has one) is still unread.
    Head(Head),
}

/// Connections waiting for a worker.
#[derive(Debug, Default)]
struct Queue {
    pending: VecDeque<QueuedConnection>,
    closed: bool,
}

/// A connection accepted from the listener, with the instant its pre-auth budget started.
///
/// The instant is recorded when `accept` returns, **before** the connection is queued, so
/// time spent waiting for a worker consumes the same absolute budget. A fresh deadline at
/// dispatch would let a queued peer wait one full budget per wave.
#[derive(Debug)]
struct QueuedConnection {
    stream: TcpStream,
    accepted_at: Instant,
}

/// State shared by the accept loop and the worker pool.
struct State {
    config: ServerConfig,
    service: Arc<AppService>,
    limits: ServiceLimits,
    tokens: TokenTable,
    shutdown: AtomicBool,
    served: AtomicU64,
    queue: Mutex<Queue>,
    ready: Condvar,
    /// In-flight long-run admission (ADR-0016 §1, F4/K10).
    admission: Arc<AdmissionGate>,
}

impl fmt::Debug for State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("State")
            .field("config", &self.config)
            .field("service", &self.service)
            .field("served", &self.served.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl State {
    fn lock_queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops accepting, wakes the workers and lets the request in flight finish.
    fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let mut queue = self.lock_queue();
        queue.closed = true;
        self.ready.notify_all();
    }
}

/// A running server.
///
/// Dropping the handle requests a shutdown and joins every thread, exactly like
/// [`ServerHandle::shutdown`]; [`ServerHandle::wait`] joins without requesting one.
#[derive(Debug)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    state: Arc<State>,
    accept: Option<JoinHandle<()>>,
    workers: Vec<JoinHandle<()>>,
}

impl ServerHandle {
    /// Returns the effective bound address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops the server and waits for its threads to finish.
    ///
    /// Requests the shutdown first, so an idle server returns promptly instead of waiting
    /// for a request that will never come.
    pub fn shutdown(mut self) -> Result<(), TransportError> {
        self.state.stop();
        self.join_threads();
        Ok(())
    }

    /// Waits for the server to stop on its own, without requesting a shutdown.
    ///
    /// Joins the accept thread and every worker but never signals the shutdown flag, so a
    /// long-lived owner can wait for the server's own lifetime: with
    /// `max_requests = Some(n)` this returns once the `n`-th request has been served and
    /// the workers have drained, and with `max_requests = None` it blocks until the
    /// process is terminated. It also returns if the accept loop fails on its own.
    pub fn wait(mut self) -> Result<(), TransportError> {
        self.join_threads();
        Ok(())
    }

    fn join_threads(&mut self) {
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }

    fn joined(&self) -> bool {
        self.accept.is_none() && self.workers.is_empty()
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if self.joined() {
            return;
        }
        self.state.stop();
        self.join_threads();
    }
}

/// Starts the HTTP/MCP server on the configured address.
///
/// # Errors
///
/// Returns [`TransportError::Io`] when the address cannot be bound or a thread cannot be
/// started, and [`TransportError::Protocol`] when the address is not a loopback address,
/// the configuration asks for fewer than two workers, or the pre-auth deadline is zero.
pub fn serve(
    config: ServerConfig,
    service: Arc<AppService>,
) -> Result<ServerHandle, TransportError> {
    // ADR-0014 §1: v1 serves loopback only. Refusing here means a non-loopback address
    // never reaches `TcpListener::bind`, including when the transport is composed
    // directly rather than through `chronofork serve` (which refuses even earlier, at
    // argument parsing).
    if !config.bind.ip().is_loopback() {
        return Err(TransportError::Protocol(format!(
            "refusing the non-loopback bind address `{}`; v1 serves loopback only (ADR-0014)",
            config.bind
        )));
    }
    // A zero pre-auth budget would close every connection before its head could be
    // read, so it is a configuration error rather than a "disable the bound" switch.
    if config.pre_auth_timeout.is_zero() {
        return Err(TransportError::Protocol(
            "the pre-auth deadline must be greater than zero".to_owned(),
        ));
    }
    // ADR-0016 §1/F4: the bounded pool reserves at least one worker for the control
    // path, so a single-worker configuration cannot keep `cancel`/`list`/`progress`
    // executable while a long run is in flight. It is refused unless the operator
    // selects a separate control path, which this build does not implement.
    if config.workers < 2 {
        return Err(TransportError::Protocol(format!(
            "the HTTP server requires at least two workers (workers = {}), so one stays \
             reserved for the control path",
            config.workers
        )));
    }
    let listener = TcpListener::bind(config.bind)?;
    listener.set_nonblocking(true)?;
    let local_addr = listener.local_addr()?;
    let limits = *service.limits();
    let tokens = config.tokens.clone();
    let admission = AdmissionGate::new(AdmissionLimits {
        global: config.long_run_limit(),
        per_principal: config.per_principal_limit(),
    });
    let state = Arc::new(State {
        config,
        service,
        limits,
        tokens,
        shutdown: AtomicBool::new(false),
        served: AtomicU64::new(0),
        queue: Mutex::new(Queue::default()),
        ready: Condvar::new(),
        admission: Arc::new(admission),
    });

    let mut workers = Vec::with_capacity(state.config.workers);
    for _ in 0..state.config.workers {
        let worker_state = Arc::clone(&state);
        match thread::Builder::new()
            .name("chronofork-http-worker".to_owned())
            .spawn(move || worker_loop(worker_state))
        {
            Ok(handle) => workers.push(handle),
            Err(error) => {
                state.stop();
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(TransportError::Io(error));
            }
        }
    }

    let accept_state = Arc::clone(&state);
    match thread::Builder::new()
        .name("chronofork-http-accept".to_owned())
        .spawn(move || accept_loop(listener, accept_state))
    {
        Ok(accept) => Ok(ServerHandle {
            local_addr,
            state,
            accept: Some(accept),
            workers,
        }),
        Err(error) => {
            state.stop();
            for worker in workers {
                let _ = worker.join();
            }
            Err(TransportError::Io(error))
        }
    }
}

fn accept_loop(listener: TcpListener, state: Arc<State>) {
    loop {
        if state.shutdown.load(Ordering::SeqCst) {
            break;
        }
        // Expire queued connections against their accept-relative budget before a
        // worker ever receives them. The queue is ordered by accept time, so only the
        // front can be expired; dropping it here releases the slot without spending a
        // worker's remaining budget on a peer that already ran out.
        let mut queue = state.lock_queue();
        let now = Instant::now();
        while let Some(front) = queue.pending.front() {
            if front.accepted_at + state.config.pre_auth_timeout > now {
                break;
            }
            queue.pending.pop_front();
        }
        // Backpressure: accept only when a worker could pick the connection up. Excess
        // clients wait in the operating system backlog, so the queue stays bounded.
        let has_room = queue.pending.len() < ACCEPT_QUEUE_DEPTH;
        drop(queue);
        if !has_room {
            thread::sleep(ACCEPT_POLL);
            continue;
        }
        match listener.accept() {
            Ok((stream, _peer)) => {
                // The absolute pre-auth budget starts at accept, not when a worker is
                // free: a queued connection must not receive a fresh budget.
                let accepted_at = Instant::now();
                let _ = stream.set_nodelay(true);
                let mut queue = state.lock_queue();
                if queue.closed {
                    drop(queue);
                    break;
                }
                queue.pending.push_back(QueuedConnection {
                    stream,
                    accepted_at,
                });
                drop(queue);
                state.ready.notify_one();
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    state.stop();
}

fn worker_loop(state: Arc<State>) {
    loop {
        let queued = {
            let mut queue = state.lock_queue();
            loop {
                if queue.closed {
                    return;
                }
                if let Some(queued) = queue.pending.pop_front() {
                    break queued;
                }
                let (guard, _timeout) = state
                    .ready
                    .wait_timeout(queue, WORKER_POLL)
                    .unwrap_or_else(PoisonError::into_inner);
                queue = guard;
            }
        };
        // Carry the accept instant through dispatch: a queued connection whose budget
        // already expired is closed by `read_head` before it can read or answer.
        let deadline = queued.accepted_at + state.config.pre_auth_timeout;
        handle_connection(queued.stream, deadline, &state);
        let served = state.served.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(max_requests) = state.config.max_requests {
            if served >= max_requests {
                state.stop();
            }
        }
    }
}

fn handle_connection(mut stream: TcpStream, deadline: Instant, state: &Arc<State>) {
    let _ = stream.set_write_timeout(Some(state.config.io_timeout));
    // `deadline` is the accept-relative absolute pre-auth budget; it ends when the head
    // is complete, and `read_head` recomputes the remaining budget before every read so
    // a trickling peer cannot extend it.
    let response = match read_head(&mut stream, deadline, state.config.io_timeout) {
        HeadOutcome::Closed => return,
        HeadOutcome::Rejected(response) => response,
        HeadOutcome::Head(head) => {
            // Authenticated body reads use the ordinary per-operation socket timeout.
            let _ = stream.set_read_timeout(Some(state.config.io_timeout));
            respond(&mut stream, state, &head)
        }
    };
    finish(&mut stream, &response);
}

/// Writes the response, ends the write direction and discards a bounded tail.
///
/// `Connection: close` means the peer may still be sending (a rejected body, a pipelined
/// second request). Closing with unread bytes makes the kernel answer with a reset, which
/// can destroy the typed rejection before the peer reads it, so the shutdown writes the
/// final `FIN` first and the tail is then read and discarded under a hard bound.
fn finish(stream: &mut TcpStream, response: &Response) {
    let _ = write_response(stream, response);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(DRAIN_TIMEOUT));
    let mut sink = [0u8; CHUNK_BYTES];
    let mut drained = 0_usize;
    let deadline = Instant::now() + DRAIN_DEADLINE;
    while drained < DRAIN_BYTES && Instant::now() < deadline {
        match stream.read(&mut sink) {
            Ok(0) => break,
            Ok(count) => drained = drained.saturating_add(count),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

/// Reads and parses one request head, leaving body bytes in `Head::leftover`.
///
/// Every read is bounded by what is left of the absolute `deadline` (and by
/// `io_timeout`), so a peer that trickles the head cannot hold the worker past the
/// pre-auth budget. A read that times out is reported like a closed peer: nothing is
/// answered before a complete head exists.
fn read_head(stream: &mut TcpStream, deadline: Instant, io_timeout: Duration) -> HeadOutcome {
    let mut buffer: Vec<u8> = Vec::with_capacity(CHUNK_BYTES);
    let mut chunk = [0u8; CHUNK_BYTES];
    let mut scanned = 0;
    loop {
        if let Some(end) = find_terminator(&buffer, scanned) {
            let head = buffer.get(..end).unwrap_or_default();
            let leftover = buffer.get(end + 4..).unwrap_or_default().to_vec();
            return match parse_head(head, leftover) {
                Ok(head) => HeadOutcome::Head(head),
                Err(response) => HeadOutcome::Rejected(response),
            };
        }
        if buffer.len() > MAX_HEAD_BYTES {
            return HeadOutcome::Rejected(wire::transport_failure(
                WireCode::BadRequest,
                "request head exceeds the wire limits",
            ));
        }
        let now = Instant::now();
        if now >= deadline {
            return HeadOutcome::Closed;
        }
        let remaining = deadline.saturating_duration_since(now).min(io_timeout);
        let _ = stream.set_read_timeout(Some(remaining));
        match stream.read(&mut chunk) {
            Ok(0) => {
                if buffer.is_empty() {
                    return HeadOutcome::Closed;
                }
                return HeadOutcome::Rejected(wire::transport_failure(
                    WireCode::BadRequest,
                    "incomplete request head",
                ));
            }
            Ok(count) => {
                scanned = buffer.len().saturating_sub(3);
                buffer.extend_from_slice(chunk.get(..count).unwrap_or_default());
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            // A read timeout or reset before a complete head leaves nothing to answer.
            Err(_) => return HeadOutcome::Closed,
        }
    }
}

fn find_terminator(buffer: &[u8], from: usize) -> Option<usize> {
    buffer
        .get(from..)?
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|offset| from + offset)
}

fn parse_head(head: &[u8], leftover: Vec<u8>) -> Result<Head, Response> {
    let text = std::str::from_utf8(head).map_err(|_| {
        wire::transport_failure(WireCode::BadRequest, "request head is not valid UTF-8")
    })?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    if request_line.len() > MAX_REQUEST_LINE_BYTES {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "request line exceeds the wire limit",
        ));
    }
    let parts: Vec<&str> = request_line.split(' ').collect();
    let [method, target, version] = parts.as_slice() else {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "malformed request line",
        ));
    };
    if method.is_empty() || !method.bytes().all(is_token_byte) {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "malformed request method",
        ));
    }
    if !target.starts_with('/') {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "unsupported request target",
        ));
    }
    if *version != "HTTP/1.1" {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "unsupported HTTP version",
        ));
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.len() > MAX_HEADER_LINE_BYTES {
            return Err(wire::transport_failure(
                WireCode::BadRequest,
                "header line exceeds the wire limit",
            ));
        }
        if headers.len() >= MAX_HEADER_LINES {
            return Err(wire::transport_failure(
                WireCode::BadRequest,
                "too many request headers",
            ));
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(wire::transport_failure(
                WireCode::BadRequest,
                "header continuation is not supported",
            ));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(wire::transport_failure(
                WireCode::BadRequest,
                "malformed request header",
            ));
        };
        if name.is_empty() || !name.bytes().all(is_token_byte) {
            return Err(wire::transport_failure(
                WireCode::BadRequest,
                "invalid request header name",
            ));
        }
        let value = value.trim_matches([' ', '\t']);
        let invalid = value
            .bytes()
            .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f);
        if invalid {
            return Err(wire::transport_failure(
                WireCode::BadRequest,
                "invalid request header value",
            ));
        }
        let name = name.to_ascii_lowercase();
        if name == "content-length" && headers.iter().any(|(key, _)| key == "content-length") {
            return Err(wire::transport_failure(
                WireCode::BadRequest,
                "duplicate Content-Length",
            ));
        }
        headers.push((name, value.to_owned()));
    }
    Ok(Head {
        method: (*method).to_owned(),
        target: (*target).to_owned(),
        headers,
        leftover,
    })
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn respond(stream: &mut TcpStream, state: &Arc<State>, head: &Head) -> Response {
    if head.target.contains('?') {
        return wire::service_failure(ErrorCode::InvalidRequest, "query strings are not supported");
    }
    // ADR-0014 §1.5: v1 has no browser-facing surface, so the permitted policy is that
    // no `Origin`-bearing request is accepted at all. This is checked before the route,
    // authentication or any body work, so the refusal is deterministic and cannot be
    // turned into a state mutation by a page in a local browser.
    if head.header("origin").is_some() {
        return wire::service_failure(
            ErrorCode::Forbidden,
            "an `Origin`-bearing request is refused: v1 has no browser-facing surface \
             (ADR-0014)",
        );
    }
    let Some(route) = Route::parse(&head.target) else {
        return wire::service_failure(ErrorCode::NotFound, "unknown route");
    };
    if head.method != route.allowed_method() {
        return wire::transport_failure(WireCode::MethodNotAllowed, "method not allowed")
            .with_allow(route.allowed_method());
    }
    match route {
        Route::Health => health_response(state),
        Route::Tools => {
            let principal = match authenticate(head, &state.tokens) {
                Ok(principal) => principal,
                Err(response) => return response,
            };
            if !principal.has_scope(Scope::Read) {
                return wire::service_failure(
                    ErrorCode::Forbidden,
                    "principal lacks the `read` scope",
                );
            }
            tools_response(state)
        }
        Route::Ops => {
            // The version is decided before anything runs: an incompatible request must
            // never reach dispatch (ADR-0016 §Compatibility/F10).
            let version = match requested_version(head, &state.limits) {
                Ok(version) => version,
                Err(response) => return response,
            };
            let (principal, body) = match authorized_body(stream, state, head) {
                Ok(parts) => parts,
                Err(response) => return response,
            };
            // Long-run admission happens **before** the work: a request above the global
            // or per-principal bound is answered with a typed limit instead of occupying
            // a worker (ADR-0016 §1, F4/K10).
            let _admission = match admit(state, &principal, &body) {
                Ok(guard) => guard,
                Err(response) => return response,
            };
            match state.service.dispatch_versioned(&principal, &body, version) {
                Ok(envelope) => Response::new(200, envelope),
                Err(error) => wire::service_error_versioned(&error, &state.limits, version),
            }
        }
        Route::Mcp => {
            let (principal, body) = match authorized_body(stream, state, head) {
                Ok(parts) => parts,
                Err(response) => return response,
            };
            mcp::respond(
                &body,
                &principal,
                &state.service,
                &state.limits,
                &state.admission,
            )
        }
    }
}

/// Reads the API wire version of one request.
///
/// An absent header is v1 (the compatibility rule); an unimplementable value is refused
/// with the typed `unsupported_version` envelope **before** the body is read or any work
/// starts.
fn requested_version(head: &Head, limits: &ServiceLimits) -> Result<ApiVersion, Response> {
    let Some(raw) = head.header(wire::API_VERSION_HEADER) else {
        return Ok(ApiVersion::V1);
    };
    ApiVersion::parse(raw).ok_or_else(|| {
        let error = ServiceError::new(
            ErrorCode::UnsupportedVersion,
            "this server does not implement the requested API wire version",
        );
        wire::service_error_versioned(&error, limits, ApiVersion::V1)
    })
}

/// Admits one request's long work, when it has any.
///
/// The operation is read from the canonical request document (never from a client
/// header), so the predicate cannot be dodged by a request that looks small. A request
/// that needs no admission, or whose document does not parse, returns `Ok(None)` and is
/// answered by dispatch with its own typed error.
fn admit(
    state: &Arc<State>,
    principal: &Principal,
    body: &[u8],
) -> Result<Option<AdmissionGuard>, Response> {
    let Ok(document) = chronofork_service::json::parse(body, &state.limits) else {
        return Ok(None);
    };
    let Some(op) = document.get("op").and_then(JsonValue::as_str) else {
        return Ok(None);
    };
    if !is_long_operation(op) {
        return Ok(None);
    }
    // SAFETY-free lifetime juggling: the guard borrows the gate out of the `Arc<State>`
    // that lives for this request, so the borrow is tied to the same allocation. The gate
    // is held in an `Arc` of its own to make that explicit.
    match AdmissionGate::acquire(&state.admission, principal.name()) {
        Some(guard) => Ok(Some(guard)),
        None => {
            let error = ServiceError::new(
                ErrorCode::LimitExceeded,
                "the long-run admission bound is reached; retry when a long run finishes",
            );
            Err(wire::service_error_versioned(
                &error,
                &state.limits,
                ApiVersion::V1,
            ))
        }
    }
}

/// Enforces the `POST` framing rules, the bearer token and reads the bounded body.
fn authorized_body(
    stream: &mut TcpStream,
    state: &Arc<State>,
    head: &Head,
) -> Result<(Principal, Vec<u8>), Response> {
    let content_length = framed_length(head, state.config.max_request_bytes)?;
    let principal = authenticate(head, &state.tokens)?;
    let body = read_body(stream, head, content_length)?;
    Ok((principal, body))
}

fn framed_length(head: &Head, max_request_bytes: usize) -> Result<usize, Response> {
    if head.header("transfer-encoding").is_some() {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "request bodies must not be chunked",
        ));
    }
    let Some(raw_length) = head.header("content-length") else {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "Content-Length is required on POST",
        ));
    };
    let Some(content_length) = parse_content_length(raw_length) else {
        return Err(wire::transport_failure(
            WireCode::BadRequest,
            "invalid Content-Length",
        ));
    };
    if content_length > max_request_bytes {
        return Err(wire::transport_failure(
            WireCode::BodyTooLarge,
            "request body exceeds the configured byte limit",
        ));
    }
    if !is_json_media_type(head.header("content-type")) {
        return Err(wire::transport_failure(
            WireCode::UnsupportedMediaType,
            "Content-Type must be application/json",
        ));
    }
    Ok(content_length)
}

fn parse_content_length(raw: &str) -> Option<usize> {
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

fn is_json_media_type(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let media_type = value.split(';').next().unwrap_or_default().trim();
    media_type.eq_ignore_ascii_case("application/json")
}

fn authenticate(head: &Head, tokens: &TokenTable) -> Result<Principal, Response> {
    let Some(raw) = head.header("authorization") else {
        return Err(wire::service_failure(
            ErrorCode::Unauthorized,
            "missing bearer token",
        ));
    };
    let Some((scheme, token)) = raw.split_once(' ') else {
        return Err(wire::service_failure(
            ErrorCode::Unauthorized,
            "malformed authorization header",
        ));
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(wire::service_failure(
            ErrorCode::Unauthorized,
            "unsupported authorization scheme",
        ));
    }
    tokens
        .authenticate(token.trim_matches(' '))
        .ok_or_else(|| wire::service_failure(ErrorCode::Unauthorized, "unknown bearer token"))
}

fn read_body(
    stream: &mut TcpStream,
    head: &Head,
    content_length: usize,
) -> Result<Vec<u8>, Response> {
    let prefix = head.leftover.as_slice();
    if let Some(body) = prefix.get(..content_length) {
        return Ok(body.to_vec());
    }
    let mut body = Vec::with_capacity(content_length);
    body.extend_from_slice(prefix);
    let mut chunk = [0u8; CHUNK_BYTES];
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let want = remaining.min(chunk.len());
        let slice = match chunk.get_mut(..want) {
            Some(slice) => slice,
            None => &mut [],
        };
        match stream.read(slice) {
            Ok(0) => {
                return Err(wire::transport_failure(
                    WireCode::BadRequest,
                    "truncated request body",
                ));
            }
            Ok(count) => {
                body.extend_from_slice(slice.get(..count).unwrap_or_default());
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => {
                return Err(wire::transport_failure(
                    WireCode::BadRequest,
                    "truncated request body",
                ));
            }
        }
    }
    Ok(body)
}

fn health_response(state: &Arc<State>) -> Response {
    let document = JsonValue::object([
        ("schema", JsonValue::string("chronofork.health.v1")),
        ("status", JsonValue::string("ok")),
    ]);
    Response::new(200, wire::encode(&document, &state.limits))
}

fn tools_response(state: &Arc<State>) -> Response {
    let tools: Vec<JsonValue> = state
        .service
        .tools()
        .into_iter()
        .map(|tool| {
            JsonValue::object([
                ("name", JsonValue::string(tool.name)),
                ("description", JsonValue::string(tool.description)),
                ("mutating", JsonValue::from_bool(tool.mutating)),
                ("scope", JsonValue::string(tool.scope.as_str())),
                ("inputSchema", tool.input_schema),
            ])
        })
        .collect();
    let document = JsonValue::object([
        ("schema", JsonValue::string("chronofork.tools.v1")),
        ("tools", JsonValue::Array(tools)),
    ]);
    Response::new(200, wire::encode(&document, &state.limits))
}

fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        wire::reason(response.status)
    );
    head.push_str("Content-Type: ");
    head.push_str(wire::RESPONSE_MEDIA_TYPE);
    head.push_str("\r\nContent-Length: ");
    head.push_str(&response.body.len().to_string());
    head.push_str("\r\nConnection: close\r\nCache-Control: no-store\r\n");
    head.push_str("X-Content-Type-Options: nosniff\r\n");
    if let Some(allow) = response.allow {
        head.push_str("Allow: ");
        head.push_str(allow);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_status_table_covers_every_error_code() {
        // Architecture §4 fixes the mapping; this test is the executable copy of that
        // table so a new `ErrorCode` cannot silently miss a status.
        let table = [
            (ErrorCode::InvalidRequest, 400),
            (ErrorCode::UnknownOp, 400),
            (ErrorCode::LimitExceeded, 400),
            (ErrorCode::Unauthorized, 401),
            (ErrorCode::Forbidden, 403),
            (ErrorCode::NotFound, 404),
            (ErrorCode::Conflict, 409),
            (ErrorCode::Closed, 409),
            (ErrorCode::Halted, 409),
            (ErrorCode::NotConfigured, 503),
            (ErrorCode::Backend, 500),
            (ErrorCode::Storage, 500),
            (ErrorCode::Internal, 500),
        ];
        for (code, status) in table {
            assert_eq!(wire::status_for(code), status, "{}", code.as_str());
        }
    }

    #[test]
    fn transport_failure_codes_carry_the_documented_status_and_envelope() {
        let table = [
            (WireCode::BadRequest, 400, "bad_request"),
            (WireCode::MethodNotAllowed, 405, "method_not_allowed"),
            (
                WireCode::UnsupportedMediaType,
                415,
                "unsupported_media_type",
            ),
            (WireCode::BodyTooLarge, 413, "body_too_large"),
        ];
        for (code, status, wire) in table {
            let response = wire::transport_failure(code, "rejected");
            assert_eq!(response.status, status);
            let document =
                chronofork_service::json::parse(&response.body, &ServiceLimits::default())
                    .expect("envelope");
            assert_eq!(
                document
                    .get("schema")
                    .and_then(JsonValue::as_str)
                    .expect("schema"),
                "chronofork.error.v1"
            );
            assert_eq!(
                document
                    .get("error")
                    .and_then(|error| error.get("code"))
                    .and_then(JsonValue::as_str)
                    .expect("code"),
                wire
            );
        }
    }

    #[test]
    fn content_length_parsing_is_strict_and_bounded() {
        assert_eq!(parse_content_length("0"), Some(0));
        assert_eq!(parse_content_length("16"), Some(16));
        assert_eq!(parse_content_length("+1"), None);
        assert_eq!(parse_content_length("-1"), None);
        assert_eq!(parse_content_length(""), None);
        assert_eq!(parse_content_length(" 1"), None);
        assert_eq!(parse_content_length("1 "), None);
        assert_eq!(parse_content_length("18446744073709551616"), None);
        assert_eq!(parse_content_length("99999999999999999999999"), None);
    }

    #[test]
    fn json_media_type_accepts_parameters_but_not_other_types() {
        assert!(is_json_media_type(Some("application/json")));
        assert!(is_json_media_type(Some("application/json; charset=utf-8")));
        assert!(is_json_media_type(Some("Application/JSON;charset=utf-8")));
        assert!(!is_json_media_type(Some("text/plain")));
        assert!(!is_json_media_type(Some("application/json-seq")));
        assert!(!is_json_media_type(None));
    }

    #[test]
    fn a_head_parses_into_a_method_target_and_lowercase_headers() {
        let head = parse_head(
            b"POST /v1/ops HTTP/1.1\r\nContent-Length: 2\r\nAuthorization: Bearer t",
            Vec::new(),
        )
        .expect("head");
        assert_eq!(head.method, "POST");
        assert_eq!(head.target, "/v1/ops");
        assert_eq!(head.header("content-length"), Some("2"));
        assert_eq!(head.header("authorization"), Some("Bearer t"));
    }

    #[test]
    fn malformed_heads_are_rejected_with_the_documented_code() {
        for (head, expected) in [
            (&b"GET\r\n"[..], 400_u16),
            (&b"GET /health\r\n"[..], 400),
            (&b"GET /health HTTP/1.0"[..], 400),
            (&b"GET health HTTP/1.1"[..], 400),
            (&b"GET  /health HTTP/1.1"[..], 400),
            (&b"GET /health HTTP/1.1\r\nfolded\r\n"[..], 400),
            (&b"GET /health HTTP/1.1\r\n: x\r\n"[..], 400),
        ] {
            let response = parse_head(head, Vec::new()).expect_err("rejection");
            assert_eq!(response.status, expected);
        }
    }
}

#[cfg(test)]
mod debug_redaction {
    use super::ServerConfig;
    use chronofork_service::parse_token_file;

    const TOKENS: &str = r#"{"schema":"chronofork.tokens.v1","principals":[
     {"name":"operator","token":"operator-token-0001","scopes":["read","write"]}]}"#;

    #[test]
    fn server_config_debug_never_renders_bearer_tokens() {
        let tokens = parse_token_file(TOKENS.as_bytes()).expect("token table");
        let config = ServerConfig {
            tokens,
            ..ServerConfig::default()
        };
        let rendered = format!("{config:?}");
        assert!(rendered.contains("ServerConfig"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        for (_, token) in config.tokens.credentials() {
            assert!(
                !rendered.contains(token),
                "token material leaked into `Debug`: {rendered}"
            );
        }
    }
}
