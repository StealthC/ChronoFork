# ADR-0012: Agent-native interfaces — one service, HTTP and MCP adapters

- **Status:** **Accepted** (owner-approved 2026-11-17 by the explicit Stage 6
  implementation directive, "implemente completamente o stage 6 do roadmap").
- **Date:** 2026-11-17.
- **Milestone:** [M6 — Agent interfaces](../roadmap/milestones.md#m6--agent-interfaces).
- **Roadmap stage:** [Stage 6 — Agent-native interfaces](../roadmap/roadmap.md#stage-6--agent-native-interfaces).
- **Companion contract:** [Agent interfaces architecture](../architecture/agent-interfaces.md)
  (exact wire schemas, error taxonomy, limits) and the executable
  [M6 acceptance plan](../quality/m6-agent-interfaces-acceptance.md).
- **Supersedes:** nothing. Extends [ADR-0001](0001-rust-workspace.md) (crate boundaries) and
  [ADR-0003](0003-durable-session-storage.md) (storage/ownership split).

## Context

Stages 0–5 delivered a deterministic session layer, durable storage, TAS movies, a host
debugger, structured tracing and a sandboxed script runtime. Every capability is reachable
only from the `chronofork` process that owns it: there is no way for an agent or a second
human process to observe or drive a session that another process created, and no machine
surface that is not a CLI parsing contract.

Stage 6 requires the same application services to be drivable through CLI, HTTP and MCP
with identical semantics, without creating a second source of truth and without letting
transport types leak into core, backend or session APIs. The repository's standing rules
narrow the acceptable design:

- Transport types never appear in `chronofork-core`, `chronofork-backend-api` or
  `chronofork-sessions` ([architecture overview](../architecture/overview.md#dependency-rules)).
- No `unsafe` (workspace lint forbids it), and dependencies stay minimal and justified
  ([AGENTS.md](../../AGENTS.md)).
- Every surface is bounded and deterministic; no implicit host wall-clock input to
  emulated state.
- A capability is advertised only when tests back it ([anti-slop
  policy](../quality/anti-slop-policy.md)).

## Problem

Where does the application surface live so that the CLI is one client among several rather
than the implementation, and what transport byte-level contracts follow from that?

## Decision

### 1. A transport-agnostic service crate owns the application operations

A new crate, `chronofork-service`, owns the typed operation catalog, input/output
validation, authentication/ownership policy and the concurrency boundary. It depends on
`chronofork-core`, `chronofork-backend-api` and `chronofork-sessions` only. It contains no
socket, HTTP header, JSON-RPC or CLI type; it consumes and produces its own canonical JSON
value type. Concrete backends are injected as `Arc<dyn BackendFactory>`, exactly as the
session layer requires.

`SessionManager` keeps its documented shape: synchronous, `&mut self`, no hidden lock. The
service adapter is the single place that adds a synchronization boundary and document it
([overview open question](../architecture/overview.md#open-questions)).

### 2. A transport crate owns HTTP and MCP

A second crate, `chronofork-transport`, depends on `chronofork-service` and implements:

- a bounded, blocking HTTP/1.1 server over `std::net` and `std::thread`;
- MCP over Streamable HTTP as a stateless JSON-RPC 2.0 endpoint;
- a typed client used by the CLI and by tests.

No third-party HTTP, async-runtime or serialization dependency is added. The reason is not
minimalism for its own sake: the repository already hand-rolls its canonical binary formats
(`CFSNAP`, `CFMOVIE2`, `CFTRACE1`), a full HTTP stack would drag a large transitive tree and
an async runtime into a synchronous deterministic codebase, and the required surface is a
small, fixed, auditable protocol subset. A hand-written bounded HTTP/1.1 server is ~600
lines and can be tested at the byte level with `std::net` clients.

### 3. One typed operation surface, not hundreds of endpoints

The service exposes a small catalog of named operations (`session.create`, `session.run`,
`store.save`, `experiment.run`, …). Both transports are thin mappings of the same catalog:

| Surface | Shape |
|---|---|
| HTTP | `GET /health`, `GET /v1/tools`, `POST /v1/ops` with `{"op": "...", "input": {...}}` |
| MCP | `POST /mcp` JSON-RPC `initialize`, `notifications/initialized`, `ping`, `tools/list`, `tools/call`; one MCP tool per operation |
| CLI | `chronofork serve`, `chronofork api tools|call|health`, `chronofork mcp tools|call`, and the existing `chronofork session *` commands refactored onto the same service functions |

The catalog is curated, not a mirror of every library function: it covers the session
lifecycle, durable checkpoints, bounded memory reads, deterministic experiment artifacts and
service capability discovery. Debug observation, trace capture and script execution stay
CLI-local in M6 and are recorded as **M6-FU1**; they are not hidden, and no transport claims
them.

### 4. Canonical JSON is a service-owned format

The service owns a strict JSON codec: integers only, duplicate keys rejected, object keys
sorted bytewise on output, no insignificant whitespace, bounded input depth/size and no
floats. This makes every response byte-deterministic and lets parity tests compare exact
bytes instead of field subsets. The codec is rejected-output oriented: unknown operation
fields are errors, never silently ignored.

### 5. Authentication, ownership and concurrency live at the service boundary

A token file (`chronofork.tokens.v1` JSON) maps opaque bearer tokens to principals with
coarse scopes (`read`, `write`, `admin`). The service records the creating principal for
every session; non-admin principals can only see and mutate their own sessions. Mutating
operations require the `write` scope (or `admin`); read operations require `read` (or
`admin`).

Concurrency uses two explicit layers:

- a service-wide manager lock serializes access to `SessionManager`;
- a per-session writer lease makes a second concurrent mutation of the same session fail
  immediately with a typed, retryable `conflict` instead of blocking or interleaving.

Reads take no writer lease, so listing and inspection remain available while a mutation
runs; they are still serialized by the manager lock and therefore observe a consistent
state. The daemon documents one writer per session at a time.

The transport boundary also enforces resource bounds (body size, header count, request
line, timeouts, worker count, response size) so a client cannot make the daemon allocate
without limit.

### 6. Machine-consumable experiment artifacts are first-class

`experiment.run` executes a bounded, fully specified input plan on a fresh session and
returns a canonical `chronofork.experiment.v1` document: backend descriptor and
configuration hash, the applied inputs, per-checkpoint ticks/state hashes, the final state
and a content-derived artifact id (SHA-256 of the canonical bytes). The document contains no
host time, no paths and no principal identity, so the same request produces byte-identical
artifact bytes on every run and transport. `chronofork experiment run --out <FILE>` writes
those exact bytes.

### 7. The CLI is a client, not a second implementation

`chronofork serve` hosts the service/transport. `chronofork api` and `chronofork mcp` are
typed clients of a running daemon. The existing `chronofork session save|load|list|delete`
commands are refactored to call the same service functions locally (in-process service) and
gain `--json` canonical output. Formatting and process exit codes stay in the CLI; every
semantic rule (validation, ownership, limits, error codes) stays in the service.

Parity is executable: one scenario harness runs each operation through the direct service,
the HTTP transport and the MCP transport against fresh instances and compares canonical
result bytes; a process-level test runs the same scenario through the CLI and compares the
canonical JSON.

## Alternatives considered and rejected

| Alternative | Why rejected |
|---|---|
| REST resources per operation (`POST /sessions/{id}/run`, …) | Hundreds of loose endpoints, duplicated validation, unstable URL taxonomy; contradicts the Stage 6 typed-tool requirement. |
| `axum`/`hyper`/`tokio` + `serde`/`serde_json` | Large transitive dependency tree, async runtime in a synchronous deterministic codebase, new MSRV/portability surface and licensing/provenance work for a bounded protocol subset. Revisit only if the surface needs streaming, TLS termination or WebSockets. |
| Server-Sent Events / long-lived MCP streams | The service has no server-initiated messages; MCP permits returning `application/json` to a POST. SSE would need streaming state and cancellation semantics with no current consumer. |
| MCP stdio transport | The Stage 6 contract is Streamable HTTP; a stdio server would be another transport with its own parity burden. |
| Persisting session ownership in durable storage metadata | Would change the storage format version and durability contract (M2). M6 keeps ownership in the service process and documents the restart limit; durable ownership is **M6-FU2**. |
| Exposing trace capture and script execution over HTTP/MCP immediately | They are large, sandbox-sensitive surfaces; the curated catalog ships first and the exposure is tracked as **M6-FU1** with its own limits. |
| A separate "experiment" daemon or database | The session layer is canonical; a second store would violate the no-second-source-of-truth exit criterion. Artifacts are content-addressed files, not state. |

## Consequences

### Positive

- One validation/auth/concurrency implementation for every transport; parity tests compare
  bytes rather than intentions.
- Transport types stay out of core/backend/session crates by construction.
- The daemon is dependency-free, bounded, `unsafe`-free and testable at the socket level.
- Canonical JSON and content-addressed artifacts give agents reproducible inputs.

### Negative and limits (stated honestly)

- The HTTP server is HTTP/1.1, one request per connection, no TLS, no keep-alive, no
  chunked bodies; it is suitable for loopback/sidecar use and must be fronted by a reverse
  proxy for anything else.
- `POST /mcp` is stateless: no `Mcp-Session-Id`, no SSE stream, no server-initiated requests,
  no resources/prompts/logging/completion. It supports the tools subset at protocol version
  `2025-03-26`.
- Tokens are coarse and static; there is no rotation, expiry, audit log or TLS. Ownership is
  process-local and is lost on daemon restart.
- No debug/trace/script operations over transport (M6-FU1); those remain CLI-local.
- The daemon drives one backend configuration chosen at startup (`synthetic` or
  `megadrive --rom <FILE>`); there is no per-request backend selection.
- No performance claim; bounds are correctness/DoS limits, not throughput guarantees.

## Security considerations

- Bind to loopback unless `--bind` explicitly names another address; the daemon prints the
  effective address and warns when it is not loopback.
- Tokens are read from a file and never from the command line or environment; the token file
  is not echoed. Comparison is not claimed to be constant-time.
- Error responses use stable codes and messages; they never include host paths, ROM bytes,
  backend payloads or token material.
- Request size, header count, timeouts, worker count and response size are bounded.
- ROM files are referenced by a server-side path at startup only; the transport never
  accepts file paths, bytes or URLs from a request.
- No telemetry, no outbound connections: the server only answers; the client only connects
  to an explicit `--url`.
- The script sandbox is not exposed by this ADR, so its capability boundary is unchanged.

## Verification and evidence

- [M6 acceptance plan](../quality/m6-agent-interfaces-acceptance.md) rows `M6-A1`–`M6-A32`
  with exact commands; rows become `executed - passed` only after independent verification.
- Wire contract: [agent interfaces architecture](../architecture/agent-interfaces.md).
- Parity harness: `crates/chronofork-transport/tests/parity.rs` (service/HTTP/MCP) and
  `crates/chronofork-cli/tests/m6.rs` (CLI process parity, auth/dispatch rejection).
- Repository gates and the dated M6 block in [quality gates](../quality/quality-gates.md).
