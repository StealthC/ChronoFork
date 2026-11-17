# Agent Interfaces Architecture

This page is the wire contract for [ADR-0012](../adr/0012-agent-native-interfaces.md):
the crate split, the canonical JSON profile, the typed operation catalog, the HTTP and
MCP mappings, authentication/ownership, concurrency, experiment artifacts and the limits.
It describes the M6 surface; the [M6 acceptance plan](../quality/m6-agent-interfaces-acceptance.md)
is how that surface is proven.

## 1. Layers

```text
                    chronofork-cli
          (serve / api / mcp / experiment / session)
                     │            │
        ┌────────────┘            └──────────────┐
        │                                       │
 chronofork-transport                    chronofork-sessions
 (HTTP/1.1, MCP JSON-RPC, client)         (canonical state, storage)
        │                                       ▲
        └──────────────► chronofork-service ────┘
                     (ops, JSON, auth, leases)
                               │
                 chronofork-core + chronofork-backend-api
```

| Crate | Responsibility | Must not contain |
|---|---|---|
| `chronofork-service` | Operation catalog, canonical JSON, token file, ownership, writer leases, experiment artifacts | Socket, HTTP/MCP/JSON-RPC types, CLI types, concrete backends |
| `chronofork-transport` | HTTP/1.1 server, MCP Streamable HTTP mapping, typed client, wire limits | Session state, operation semantics, backend selection |
| `chronofork-cli` | Composition of a backend factory, process surfaces, human formatting and exit codes | A second implementation of validation, auth or limits |

Dependency rule: `chronofork-service` depends on `chronofork-core`,
`chronofork-backend-api` and `chronofork-sessions` only; `chronofork-transport` depends on
`chronofork-service` only; the CLI composes backends and both crates. Transport types appear
in none of core, backend-api or sessions.

## 2. Canonical JSON profile

Every request and response body is UTF-8 JSON restricted to:

- `null`, `true`, `false`, integers, strings, arrays and objects;
- **no floating point** and no exponent notation;
- integers in `[-(2^53 - 1), 2^53 - 1]` (portable across JSON consumers);
- strings are UTF-8; the parser accepts `\uXXXX` escapes (including surrogate pairs) and
  rejects unpaired surrogates, raw control characters and invalid UTF-8;
- object keys are unique; duplicate keys are rejected at parse time;
- output sorts object keys bytewise and emits no insignificant whitespace;
- nesting depth is bounded by `limits.json_max_depth` (default 32);
- input size is bounded by `limits.max_request_bytes` (default 1 MiB) and output size by
  `limits.max_response_bytes` (default 1 MiB).

Two schema strings frame the envelope:

```json
{"op":"session.create","result":{"session":1}}
```

```json
{"schema":"chronofork.error.v1","error":{"code":"not_found","message":"unknown session","retryable":false}}
```

An operation error is always the error envelope. A successful call is always
`{"op":<name>,"result":<object>}` where `result` is described below. The transport never
rewrites result fields.

## 3. Operation catalog

Scope `read` requires the token's `read` scope; `write` requires `write`; `admin` implies
both and is required for cross-owner observation. Mutating operations hold a writer lease
for their session (see §7). All field names are exact; unknown fields are rejected.

| # | Operation | Scope | Mutating | Summary |
|---|---|---|---|---|
| 1 | `capabilities` | read | no | Service/backend capability report |
| 2 | `session.create` | write | yes | Create an in-memory session |
| 3 | `session.list` | read | no | List visible sessions |
| 4 | `session.inspect` | read | no | Inspect one session |
| 5 | `session.run` | write | yes | Advance a session by N steps |
| 6 | `session.fork` | write | yes | Fork a session |
| 7 | `session.snapshot` | write | yes | Capture and index a checkpoint |
| 8 | `session.restore` | write | yes | Restore an indexed checkpoint |
| 9 | `session.compare` | read | no | Compare two sessions |
| 10 | `session.close` | write | yes | Close a session |
| 11 | `session.read_memory` | read | no | Bounded memory read |
| 12 | `store.list` | read | no | List durable sessions/snapshots |
| 13 | `store.load` | read | no | Rehydrate and report one checkpoint |
| 14 | `store.save` | write | yes | Create or continue a durable session |
| 15 | `store.delete` | write | yes | Delete one durable session or snapshot |
| 16 | `experiment.run` | write | yes | Produce a content-addressed experiment artifact |

Common wire types:

- `session`, `snapshot`, `session_key`, `snapshot_key`, `parent_session`,
  `origin_snapshot`, `parent_session_key`, `origin_snapshot_key`: JSON integer `≥ 1`;
- `state_hash`, `config_hash`, `bytes_sha256`, `checkpoint_sha256`, `artifact_id`: 64
  lowercase hex characters;
- `tick`: integer `≥ 0`;
- `lineage`: `{"parent_session": <int|null>, "origin_snapshot": <int|null>, "depth": <int>}`;
- `backend`: `{"id": <string>, "display_name": <string>, "version": <string>,
  "test_backend": <bool>, "capabilities": {<14 boolean fields>}}` where the capability keys
  are exactly the `Capabilities` fields of `chronofork-backend-api`.
- `state`: `"ready" | "paused" | "closed"`.

### 3.1 `capabilities`

Input: `{}`.

```json
{
  "schema": "chronofork.capabilities.v1",
  "service_version": "0.1.0",
  "backend": {"id": "synthetic", "...": "..."},
  "ops": [{"name": "capabilities", "mutating": false}, "..."],
  "limits": {"max_request_bytes": 1048576, "...": 0},
  "store": {"configured": false},
  "artifacts": {"configured": false}
}
```

`ops` contains every catalog entry in the order of §3. `limits` contains every
`ServiceLimits` field. This operation never creates sessions.

### 3.2 `session.create`

Input: `{}`. Output:

```json
{"session":1,"state":"ready","tick":0,"state_hash":"<64 hex>",
 "lineage":{"parent_session":null,"origin_snapshot":null,"depth":0},
 "backend_id":"synthetic"}
```

The service fails with `limit_exceeded` when `max_sessions` registered sessions exist.
Closed sessions stay registered in the session layer (there is no removal API), so they keep
counting toward `max_sessions` for the process lifetime; this is a documented residual bound,
not a leak in the transport.

### 3.3 `session.list`

Input: `{}`. Output:

```json
{"sessions":[{"session":1,"state":"ready","tick":0,"state_hash":"<64 hex>",
  "lineage":{"parent_session":null,"origin_snapshot":null,"depth":0}}],
 "truncated":false}
```

Sorted by `session` ascending. A non-admin principal sees only sessions it created; an
admin sees every live session. At most `max_store_entries` entries are returned and
`truncated` reports whether more exist. Closed sessions are not listed.

### 3.4 `session.inspect`

Input: `{"session":1}`. Output:

```json
{"session":1,"state":"ready","lineage":{"...":"..."},"backend":{"...":"..."},
 "machine":{"tick":0,"state_hash":"<64 hex>","halted":false,
   "registers":{"d0":0},"memory_base":0,"memory_length":0}}
```

The memory window itself is not transported (`session.read_memory` is the bounded view).

### 3.5 `session.run`

Input: `{"session":1,"steps":10,"buttons":0}`. `steps` is an integer in
`1..=max_steps_per_request`; `buttons` is an integer in `0..=4294967295` and optional
(default `0`). The frame is `InputFrame::new(buttons)` applied identically to every step.
Output:

```json
{"session":1,"steps":10,"tick":10,"state_hash":"<64 hex>","halted":false}
```

A machine that halts during the requested steps succeeds with the `steps` actually
performed and `halted:true` (the loop stops at the halt). A request that starts on an
already-halted machine fails with code `halted`; a paused machine fails with code
`conflict` and `retryable:false`.

### 3.6 `session.fork`

Input: `{"session":1}`. Forking captures a fresh checkpoint of the source as part of the
operation, so the child lineage names the newly allocated snapshot:

```json
{"session":2,"parent":1,"tick":0,"state_hash":"<64 hex>",
 "lineage":{"parent_session":1,"origin_snapshot":1,"depth":1}}
```

### 3.7 `session.snapshot`

Input: `{"session":1,"name":null}` (name optional, `0..=256` UTF-8 bytes, no control
characters). Output:

```json
{"session":1,"snapshot":1,"name":null,"tick":0,"state_hash":"<64 hex>",
 "bytes_length":0,"bytes_sha256":"<64 hex>"}
```

`bytes_length`/`bytes_sha256` describe the exact `Snapshot::to_bytes` output. Names are
advisory labels; they never enter a state hash.

### 3.8 `session.restore`

Input: `{"session":1,"snapshot":1}`. Output:

```json
{"session":1,"snapshot":1,"tick":0,"state_hash":"<64 hex>"}
```

Restoring a closed session fails with `closed`; an unknown snapshot with `not_found`.

### 3.9 `session.compare`

Input: `{"left":1,"right":2}`. Output:

```json
{"left":1,"right":2,"left_tick":0,"right_tick":0,
 "left_state_hash":"<64 hex>","right_state_hash":"<64 hex>","equivalent":true}
```

Both sessions must be visible to the principal.

### 3.10 `session.close`

Input: `{"session":1}`. Output: `{"session":1,"state":"closed"}`. Closing twice is
idempotent and returns `state:"closed"`.

### 3.11 `session.read_memory`

Input: `{"session":1,"address":0,"length":16}`. `length` is `0..=max_memory_read`. Output:

```json
{"session":1,"address":0,"length":16,"bytes":"<lowercase hex of the bytes actually read>"}
```

A backend read failure maps to `backend_error` with a stable message and no host path.

### 3.12 `store.list`

Requires a configured storage root; otherwise `not_configured`. A configured root that was
never initialized is `storage_error` (a load/list never initializes storage). Input:
`{"session_key":null}`. Output:

```json
{"sessions":[{"session_key":1,"name":null,"parent_session_key":null,
  "origin_snapshot_key":null,"snapshots":[{"snapshot_key":1,"name":null,
  "creation_sequence":1,"tick":0,"state_hash":"<64 hex>",
  "checkpoint_length":0,"checkpoint_sha256":"<64 hex>"}]}],
 "anomalies":{"count":0,"labels":[]},"truncated":false}
```

No host path and no advisory host timestamp is ever transported. At most
`max_store_entries` sessions and snapshots are returned; `truncated` reports any
omission. `anomalies.count` is the number of uncommitted or corrupt entries the storage layer
reported and `anomalies.labels` carries at most 16 deterministic, path-free category labels
(for example `corrupt storage metadata`) so a client can report the category without a host
path; anomalies are never repaired.

### 3.13 `store.load`

Requires a configured storage root. Input:
`{"session_key":1,"snapshot_key":null,"steps":0,"buttons":0}`. Loads the latest snapshot
when `snapshot_key` is null, rehydrates it into a transient private manager, applies
`steps`/`buttons` in memory only, and reports:

```json
{"session_key":1,"snapshot_key":1,"name":null,"creation_sequence":1,
 "tick":10,"state_hash":"<64 hex>","parent_session_key":null,
 "origin_snapshot_key":null}
```

Nothing is written and the transient session is not visible to `session.list`.

### 3.14 `store.save`

Requires a configured storage root. Input:
`{"session_key":null,"steps":0,"buttons":0,"name":null}`. Mirrors the durable
`chronofork session save` workflow:

- `session_key:null`: initialize the store when needed, create one fresh runtime session,
  advance `steps` with `buttons`, create the durable session, capture a checkpoint and
  publish it;
- `session_key:N`: open the existing store, take the per-session writer lock, load the
  latest committed snapshot, rehydrate, advance `steps` with `buttons`, capture and
  publish.

Output:

```json
{"session_key":1,"snapshot_key":1,"name":null,"creation_sequence":1,
 "tick":10,"state_hash":"<64 hex>","parent_session_key":null,"origin_snapshot_key":null}
```

A contended writer lock maps to `conflict` with `retryable:true`. On a continuation
(`session_key:N`) the optional `name` labels only the new snapshot; the durable session name
is unchanged. `store.save` is the one operation whose semantics deliberately match a
stateless CLI invocation rather than a live in-memory session.

### 3.15 `store.delete`

Requires a configured storage root. Input: `{"session_key":1,"snapshot_key":null}`
(deletes the session when `snapshot_key` is null, otherwise one snapshot). Output:

```json
{"session_key":1,"snapshot_key":null}
```

Deleting an unknown key fails with `not_found`. There is no cascade: deleting a session that
still contains snapshots fails with `conflict`, deleting one snapshot does not delete its
session, and deleting a session does not delete other sessions.

### 3.16 `experiment.run`

Input: `{"steps":10,"buttons":0,"checkpoints":[0,5,10]}`. `steps` is `1..=max_steps_per_request`;
`buttons` optional `0..=4294967295`; `checkpoints` is an optional array of strictly
increasing integers in `0..=steps` with at most `max_checkpoints` entries. The experiment
runs on a private session so it never changes live session ids. Output:

```json
{"artifact":{"schema":"chronofork.experiment.v1",
  "backend":{"id":"synthetic","display_name":"...","version":"0.1.0",
    "config_hash":"<64 hex>","test_backend":true},
  "input":{"steps":10,"buttons":0},
  "checkpoints":[{"step":0,"tick":0,"state_hash":"<64 hex>"}],
  "final":{"step":10,"tick":10,"state_hash":"<64 hex>","halted":false}},
 "artifact_id":"<64 hex>"}
```

`artifact_id` is the SHA-256 of the canonical JSON bytes of the `artifact` object. The
artifact contains no host time, path or principal, so identical requests produce identical
bytes on every surface and run. With `--artifacts <DIR>` configured, the service writes
`<artifact_id>.cfexp.json` atomically (temp file plus rename) unless it already exists.

## 4. HTTP transport

- HTTP/1.1, one request per connection (`Connection: close`), no keep-alive, no chunked
  request bodies. `Content-Length` is required on `POST`.
- `Content-Type: application/json` is required on `POST`; `Transfer-Encoding` is rejected.
- `Authorization: Bearer <token>` is required for every route except `GET /health`.
- Base error envelope: `chronofork.error.v1`. HTTP status mapping:

| Code | HTTP status |
|---|---|
| `invalid_request`, `unknown_op`, `limit_exceeded` | 400 |
| `unauthorized` | 401 |
| `forbidden` | 403 |
| `not_found` | 404 |
| `conflict`, `closed`, `halted` | 409 |
| `not_configured` | 503 |
| `backend_error`, `storage_error`, `internal_error` | 500 |

Transport-level failures use the same envelope with `code` =
`bad_request` (malformed HTTP), `method_not_allowed` (405, with `Allow`),
`unsupported_media_type` (415), `body_too_large` (413), `not_found` (404 unknown route) or
`unauthorized` (401). The four framing codes are owned by the transport adapter (the
service catalog is not an HTTP router); the transport encodes them through the service
envelope builder, so the envelope format still has exactly one implementation.

Routes:

| Route | Method | Auth | Body/response |
|---|---|---|---|
| `/health` | GET | none | `{"schema":"chronofork.health.v1","status":"ok"}` |
| `/v1/tools` | GET | read | `{"schema":"chronofork.tools.v1","tools":[ToolSpec...]}` |
| `/v1/ops` | POST | per op | request `{"op":"...","input":{...}}`; response = service envelope bytes |
| `/mcp` | POST | per tool | JSON-RPC 2.0 request; response JSON |

Response headers: `Content-Type: application/json; charset=utf-8`, `Content-Length`,
`Connection: close`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`.
Query strings are rejected (`invalid_request`). Unknown routes are `not_found`.

Wire bounds: request line ≤ 8192 bytes; at most 64 headers, each ≤ 8192 bytes; body ≤
`max_request_bytes`; read/write timeout `io_timeout` (default 5 s); `workers` (default 4)
fixed worker threads with a bounded accept queue (16 connections) so the daemon never
spawns an unbounded number of threads; `max_requests` shuts the server down after N
requests for bounded process tests.

`GET /health` is the only unauthenticated response and contains no backend, path or token
information.

## 5. MCP over Streamable HTTP

`POST /mcp` implements a stateless JSON-RPC 2.0 endpoint with the tools subset at MCP
protocol version `2025-03-26`:

- `initialize` → `{"protocolVersion":"2025-03-26","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"chronofork","version":"0.1.0"}}`;
- `notifications/initialized` and any other notification (no `id`) → HTTP 202, empty body;
- `ping` → `{}`;
- `tools/list` → `{"tools":[{"name":..., "description":..., "inputSchema":...}]}`;
- `tools/call` → `{"content":[{"type":"text","text":"<canonical envelope JSON>"}],"isError":<bool>}`.

The `text` field carries exactly the service envelope bytes as a JSON string, so an MCP
call and an HTTP/direct call compare byte for byte after decoding. An operation error is a
`tools/call` result with `isError:true` and an error envelope in `text`; malformed
`tools/call` params and unknown tool names are JSON-RPC errors (`-32602`). Batches are
rejected with `-32600`; `GET`/`DELETE /mcp` return 405 with `Allow: POST`. The server never
sends server-initiated messages, does not issue `Mcp-Session-Id`, and returns
`application/json` rather than `text/event-stream`.

JSON-RPC error codes used: `-32700` parse error, `-32600` invalid request, `-32601` method
not found, `-32602` invalid params.

## 6. Authentication and ownership

Token file (`--token-file`), schema `chronofork.tokens.v1`:

```json
{"schema":"chronofork.tokens.v1",
 "principals":[{"name":"operator","token":"<opaque >=16 chars>","scopes":["read","write"]},
  {"name":"admin","token":"<opaque >=16 chars>","scopes":["admin"]}]}
```

Validation: at least one principal; unique names (`1..=64` bytes, ASCII
`[A-Za-z0-9._-]`); unique tokens (`16..=256` bytes, printable ASCII without whitespace);
`scopes` non-empty, a subset of `read`, `write`, `admin`; unknown fields rejected. The token
document is bounded in two distinct places, and they are not the same limit:
`chronofork_service::auth::parse_token_file` applies its **own** library default —
`ServiceLimits::default().max_request_bytes`, 1 MiB — while the CLI bounds the file *read*
by the request limit it will actually enforce: `chronofork serve` reads the file with its
effective server `limits.max_request_bytes` (and passes the same value into `ServerConfig`),
whereas `chronofork api` and `chronofork mcp` use the service default. The read therefore
stops at the configured bound (one byte past it is enough to reject), so an oversized file is
diagnosed before it is buffered rather than after. The service refuses to start with an
invalid or empty
table. Tokens are never accepted from
the command line or environment, are never echoed, and comparisons are not claimed to be
constant-time.

Ownership: every in-memory session records the creating principal. A non-admin principal
can list, inspect, mutate and compare only its own sessions; another principal's session
yields `forbidden` (existence is not hidden). An admin may operate on any session. Durable
store objects carry no per-principal owner in M6; they are governed by scope only and that
gap is recorded as **M6-FU2**.

## 7. Concurrency

- One `Mutex<SessionManager>` serializes all session access; the lock is never held across
  socket I/O.
- Every mutating session operation first takes that session's writer lease. A second
  concurrent mutation of the same session fails immediately with `conflict`
  (`retryable:true`) instead of interleaving; the first request keeps its lease until it
  returns.
- Reads do not take a writer lease, so `session.list`, `session.inspect`,
  `session.compare` and `session.read_memory` remain available while a mutation runs.
- `store.save` relies on the durable layer's per-session writer lock for cross-process
  exclusion, mapping contention to `conflict` with `retryable:true`.
- No background thread ever drives a session; the caller still owns the cadence.
- Closed sessions are retained for the process lifetime (no removal API in the session
  layer), so `max_sessions` is a bound on registered sessions, not on open ones.

## 8. CLI as a client

`chronofork serve --token-file <FILE> [--bind ADDR] [--store-root DIR] [--artifacts DIR]
[--rom FILE] [--workers N] [--max-requests N]` composes the backend factory, prints the
effective `bind=` address (and `backend=`, `store=`, `artifacts=` state) as `key=value`
records, then serves until the process is stopped or `--max-requests` is reached.

`chronofork api health|tools|call` and `chronofork mcp tools|call` are clients of that
daemon. They read a token file, select a principal (`--principal`, default the first) and
print the canonical response bytes unchanged, so the CLI adds no meaning.

`chronofork session save|load|list|delete` call the same service functions in-process and
gain `--json`, which prints the same canonical envelope the HTTP surface would return for
the matching operation. Human `key=value` output remains the default.

`chronofork experiment run --steps N [--buttons X] [--checkpoints a,b,c] [--out FILE]`
prints the canonical artifact document (or writes it with `--out`); it is the CLI form of
`experiment.run`.

Other M5 surfaces (`debug`, `trace`, `script`, `tas`) stay CLI-local in M6; exposing them
over transport is **M6-FU1**.

## 9. Experiment artifacts

Artifacts are content-addressed canonical documents, not emulated state: the session layer
remains the only canonical owner. They are reproducible (no host time, path or principal),
bounded (≤ `max_response_bytes`), and persisted only when `--artifacts` is configured.
A failed write is a `storage_error`; an id is never reused with different bytes.

## 10. Limits

`ServiceLimits` (defaults): `max_request_bytes` 1 MiB, `max_response_bytes` 1 MiB,
`max_steps_per_request` 100 000, `max_memory_read` 4096 B, `max_sessions` 1024,
`max_store_entries` 256, `max_checkpoints` 32, `json_max_depth` 32. Transport adds the
wire bounds in §4. A limit violation is a typed error before the bounded work starts.

## 11. Parity method

One scenario (create → inspect → run → snapshot → run → restore → fork → compare →
read-memory → list → close → experiment → store save/list/load/delete/save-continue) is
executed by four fresh surfaces: the direct `AppService`, an HTTP client, an MCP client and
the CLI process. Each step's canonical result bytes must be identical across surfaces; the
MCP text content is compared after JSON-string decoding. Error scenarios (unknown session,
invalid input, unauthorized, forbidden, wrong content type, body too large, unknown route,
unknown tool, batch request) compare their typed code and message too.

## 12. Not implemented in M6

- Debug/trace/script/TAS operations over transport (**M6-FU1**).
- Durable per-principal ownership and restart-persistent sessions (**M6-FU2**).
- TLS, token rotation/expiry, audit logs, multi-node deployment (**M6-FU3**).
- SSE/streaming, MCP resources/prompts/logging/completion, `Mcp-Session-Id` (**M6-FU4**).
- Per-request backend selection, cartridge upload, external SDKs (Stage 8).
- Closed in-memory sessions are retained for the process lifetime; `max_sessions` bounds
  registered sessions and there is no reclamation API yet.
