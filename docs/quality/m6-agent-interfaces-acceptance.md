# M6 Agent Interfaces — Executable Acceptance Plan

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** **Executed and independently verified.** Every row M6-A1–M6-A32 is
  `executed - passed`. The promotion basis is the **pass-2** independent re-verification
  (2026-11-17) after the first review returned `needs_revision` with findings M6-R1–M6-R10
  and the repair round closed the code-side findings; the round-2 independent review then
  returned **pass**. The first pass (`ROWS.md`, 2026-11-17) is preserved as round-1 evidence
  and is **not** the promotion basis for the rows whose behavior changed. **Scope of this
  status:** the code-side acceptance is verified, and the *documentary* claim honesty of
  this plan and the documents it promotes (M6-A27) is confirmed by the **separate independent
  documentation audit** (t17 PASS, 2026-11-17) after its six findings M6-DOC-1–M6-DOC-6 were
  closed by the documentation repair (t16).
- **Companion contract:** [ADR-0012: Agent-native interfaces](../adr/0012-agent-native-interfaces.md)
  — **Accepted** (owner-approved 2026-11-17 by the explicit Stage 6 implementation
  directive). The [agent interfaces architecture](../architecture/agent-interfaces.md)
  is the exact wire contract; this plan defines how it is proven.
- **Milestone:** [M6 — Agent interfaces](../roadmap/milestones.md#m6--agent-interfaces)
  (`Complete`; merged to `main` at `cb05836`, PR #6); roadmap stage [Stage 6 — Agent-native
  interfaces](../roadmap/roadmap.md#stage-6--agent-native-interfaces).
- **Related:** [architecture overview](../architecture/overview.md), [session
  model](../architecture/session-model.md), [quality gates](quality-gates.md),
  [testing strategy](testing-strategy.md), [anti-slop policy](anti-slop-policy.md),
  [M5 acceptance plan](m5-tracing-and-scripting-acceptance.md).

> **Final verification record (pass 2, 2026-11-17 02:27–02:40 America/Fortaleza).** The
> independent verifier re-executed all 32 rows on the then-uncommitted,
> implementation-complete worktree `/home/stealthc/projects/ChronoFork/.agents/worktrees/
> m6-agent-interfaces`, branch `agent/m6-agent-interfaces`, HEAD
> `93d47ed308874be0e8e7060641c238c7f6809e31` (= `origin/main`), with uncommitted M6 work
> (`TREE-STATE-PASS2.md`). Verdict: **PASS**. The full gate
> `./scripts/check.sh` exited 0 with **67 targets (57 test binaries + 10 doc-test targets) /
> 800 passed / 0 failed / 0 ignored**; the target set is identical to pass 1 (none lost,
> none added); the **57 pre-existing targets** (49 test binaries + 8 doc-tests, the M0–M5
> baseline set) contribute exactly the recorded baseline **662 passed**, and the **10 new
> M6 targets** (8 binaries + 2 doc-tests) contribute 138; the repair added exactly seven
> tests over pass 1 (793 → 800). Both demos were 3/3 byte-identical to the captain baseline
> (`9b4dd564…b84652`, `912a5bb6…348996`, empty stderr). Every M6-R1–R10 finding is either
> confirmed fixed by execution (R1, R2, R3, R8, R9, R10) or a contract-text correction
> verified against code (R4, R5). The round-2 review (2026-11-17) returned **pass** and
> confirmed no regression. Verification history: pass 1 (t4) PASS, review (t5)
> `needs_revision` M6-R1–R10, repair (t14), pass 2 (t13) PASS, re-review (t15) pass.
> The authoritative dated gate block is the **2026-11-17 M6 block** in
> [quality gates](quality-gates.md). Raw evidence is under the ignored
> `.agents/notes/m6/verify/` tree: `PASS2.md`, `TREE-STATE-PASS2.md`, `rows2/`,
> `gate/check-sh-pass2.log`, `gate/per-target-pass2.txt`, `demos2/`, `parity/*-pass2.log`
> and the `audit/*-pass2*` files, with the round-1 record in `ROWS.md`, `rows/`, `gate/`,
> `demos/`, `parity/` and `audit/`.
> No local commit, merge or publication was authorized at verification time; the work was
> subsequently merged to `main` at `cb05836` (PR #6).

## 1. Status rules

1. **`planned - not yet executed`** means the row has target files, an exact command, an
   expected observation and an evidence path, but no executed result and no independent
   verifier record. At design time every row started in that state; the pass-2 re-execution
   and this promotion are what changed them to `executed - passed`.
2. A row becomes **`executed - passed`** only when the exact command, exit code, tree state
   (branch, base commit, uncommitted paths), test summary, observed values and ignored
   evidence artifact are recorded together and an independent verifier returns PASS.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact, changed baseline byte or unverified claim. A failed row is never
   converted into a skip, an `#[ignore]`, a relaxed assertion, a narrower claim or a
   deleted test after the fact.
4. A test that cannot run is not acceptance evidence. A filter that matches zero tests is a
   failed row.
5. A row may not be satisfied by an existing M1–M5 capability. A green pre-M6 suite is a
   regression precondition, never M6 acceptance.
6. The M1–M5 canonical artifacts are a hard non-regression precondition: snapshot payload
   bytes, state hashes, configuration hashes, movie bytes, durable-store bytes, trace bytes,
   both demo outputs and the M1 oracle evidence stay byte-identical to the captured
   baseline. Any unexplained difference stops the task and is reported.
7. Acceptance is limited to the **deterministic synthetic backend** on the Linux acceptance
   platform. No M6 row executes a workload on the minimal Mega Drive backend: that backend
   appears only as `serve --rom` composition plus its capability report and a `/health`
   round-trip (M6-A20), so no Mega Drive transport, service or parity behavior is claimed.
   No TLS, SSE, durable per-principal ownership, debug/trace/script-over-transport,
   commercial-ROM, game-compatibility, performance or cross-platform runtime claim is made.
8. Implementation is not verification. Only the independent verifier execution produces
   `executed - passed`; the documentation owner records the executed results afterwards.
9. A row whose *behavior* changed during repair must be recorded with the change, not with
   the round-1 text. Pass 2 is the promotion basis for M6-A23, M6-A25 and M6-A29 (§5.2).

## 2. Scope, baseline and evidence

### 2.1 Checkout, tree state and baseline

- Tree: `/home/stealthc/projects/ChronoFork/.agents/worktrees/m6-agent-interfaces`, branch
  `agent/m6-agent-interfaces`, HEAD and base `origin/main`
  `93d47ed308874be0e8e7060641c238c7f6809e31`. The verified tree is the **uncommitted**
  worktree on that base; no commit, merge or publication was authorized for this task.
- Verified tree state (`TREE-STATE-PASS2.md`, 2026-11-17T02:36:23-03:00): modified
  `Cargo.lock`, `Cargo.toml`, `crates/chronofork-cli/Cargo.toml`,
  `crates/chronofork-cli/src/main.rs`, `crates/chronofork-cli/tests/session_storage.rs`;
  untracked `crates/chronofork-cli/src/agent/`, `crates/chronofork-cli/src/session.rs`,
  `crates/chronofork-cli/tests/m6.rs`, `crates/chronofork-service/`,
  `crates/chronofork-transport/`, `docs/adr/0012-agent-native-interfaces.md`,
  `docs/architecture/agent-interfaces.md`,
  `docs/quality/m6-agent-interfaces-acceptance.md`. Tracked diffstat: 5 files, 85
  insertions, 424 deletions (the CLI session code moved into the new `src/session.rs`).
- Environment: `CARGO_HOME=/tmp/t7-cargo`,
  `CARGO_TARGET_DIR=/home/stealthc/projects/ChronoFork/target`, `rustc`/`cargo` 1.100.0.
- Baseline captured by the lead on 2026-11-17 before implementation:
  `./scripts/check.sh` exit 0, **57 test targets / 662 passed / 0 failed / 0 ignored**;
  `chronofork demo` SHA-256
  `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` (3/3 runs);
  `chronofork megadrive demo` SHA-256
  `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996` (3/3).
- Every command in the rows below runs with
  `CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/home/stealthc/projects/ChronoFork/target`.

### 2.2 Evidence

- Raw row, gate, demo, parity and audit evidence lives under the ignored
  `.agents/notes/m6/verify/` tree. Pass 2 added `PASS2.md`, `TREE-STATE-PASS2.md`, `rows2/`,
  `gate/check-sh-pass2.log`, `gate/per-target-pass2.txt`, `demos2/`, `parity/*-pass2.log`
  and the `audit/*-pass2*` files; the first-pass files (`ROWS.md`, `rows/`, `gate/`,
  `demos/`, `parity/`, `audit/`) are preserved unchanged.
- The authoritative dated gate result belongs in [quality gates](quality-gates.md); this
  plan links to it and records the row-level observations.
- The verifier edited no implementation, test or documentation file; only ignored evidence
  was written.

### 2.3 Test targets

| Target | Command | Package |
|---|---|---|
| Service unit tests (JSON, auth, catalog) | `cargo test -p chronofork-service --lib` | `chronofork-service` |
| Service operation tests | `cargo test -p chronofork-service --test service` | `chronofork-service` |
| HTTP wire tests | `cargo test -p chronofork-transport --test http` | `chronofork-transport` |
| MCP tests | `cargo test -p chronofork-transport --test mcp` | `chronofork-transport` |
| Client tests | `cargo test -p chronofork-transport --test client` | `chronofork-transport` |
| Direct/HTTP/MCP parity | `cargo test -p chronofork-transport --test parity` | `chronofork-transport` |
| Transport unit tests (envelope, status table) | `cargo test -p chronofork-transport --lib` | `chronofork-transport` |
| CLI process tests | `cargo test -p chronofork-cli --test m6` | `chronofork-cli` |
| CLI durable-session regression | `cargo test -p chronofork-cli --test session_storage` | `chronofork-cli` |
| Full gate | `./scripts/check.sh` | workspace |

Pass-2 per-target counts (`gate/per-target-pass2.txt`): service `--lib` 45, service
`--test service` 22, transport `--lib` 9, `--test http` 21, `--test mcp` 17, `--test client`
10, `--test parity` 3, cli `--test m6` 11, cli `--test session_storage` 18;
`cargo test -p chronofork-service` 67, `cargo test -p chronofork-transport` 60.

### 2.4 Verification history

| Round | Date | Result |
|---|---|---|
| Pass 1 (independent verifier) | 2026-11-17 | **PASS** on 32/32 rows; 67 targets / 793 passed. Preserved as round-1 evidence (`ROWS.md`, `rows/`, `gate/`, `demos/`). |
| Review round 1 | 2026-11-17 | **`needs_revision`** with findings M6-R1–M6-R10 (service limit accounting, CLI truncation notice, error prefix, contract-text mismatches, unbounded token read, `Debug` token rendering, anti-slop note, stale downstream docs). |
| Repair | 2026-11-17 | Closed the code-side findings; added exactly seven tests (793 → 800); no test removed, no lint weakened. |
| Pass 2 (independent re-verification) | 2026-11-17 | **PASS**; all 32 rows re-executed; R1/R2/R3/R8/R9/R10 confirmed by execution, R4/R5 confirmed as contract-text corrections matching code. **Promotion basis.** |
| Review round 2 (independent) | 2026-11-17 | **pass**; R1–R10 code closure confirmed, no regression. R6/R7 were documentary items owned by the documentation task; that task updated them, and their documentary correctness is confirmed by the separate documentation audit (findings M6-DOC-1–D6 closed by repair t16; re-audit t17 **PASS**, 2026-11-17). |

## 3. Row map

| Area | Rows |
|---|---|
| Canonical JSON and authentication | M6-A1 – M6-A5 |
| Operation catalog, lifecycle, concurrency | M6-A6 – M6-A8 |
| Durable store and experiment artifacts | M6-A9 – M6-A10 |
| HTTP wire contract | M6-A11 – M6-A13 |
| MCP wire contract | M6-A14 – M6-A15 |
| Server bounds, client, parity | M6-A16 – M6-A19 |
| CLI surfaces and parity | M6-A20 – M6-A25 |
| Boundary, claim and security audits | M6-A26 – M6-A27, M6-A31 |
| Non-regression and gate | M6-A28 – M6-A30, M6-A32 |

## 4. Acceptance rows

Every row below is **`executed - passed`** after the pass-2 independent re-verification
(2026-11-17). Commands ran in the acceptance worktree with `CARGO_HOME=/tmp/t7-cargo`; a
package/test filter matching zero tests would have failed the row. "Observed" values are
copied from the verifier record (`ROWS.md` round-1 detail where pass 2 records the row as
unchanged, plus the pass-2 change where one exists).

### M6-A1 — Canonical JSON round-trip and canonical bytes

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-service` unit tests.
- **Command:** `cargo test -p chronofork-service --lib`
- **Exit:** 0 — **45 passed / 0 failed / 0 ignored**.
- **Observed:** canonical object-key sorting and whitespace stripping; parse → serialize →
  parse stability; four property tests (round-trip, whitespace-insensitive parse, arbitrary
  bytes never panic, no insignificant whitespace). Pass 2 adds the R9 `Debug`-redaction unit
  test to the same target; the JSON assertions are unchanged from pass 1.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-lib.log` (round 1:
  `.agents/notes/m6/verify/rows/service-lib.log`).

### M6-A2 — Canonical JSON rejection matrix

- **Status:** `executed - passed`.
- **Target:** service unit tests.
- **Command:** `cargo test -p chronofork-service --lib`
- **Exit:** 0 — **45 passed**.
- **Observed:** typed `invalid_request`/`limit_exceeded` for floats and exponents, duplicate
  keys, depth (inside and outside the bound), oversized input, oversized output, raw control
  characters, invalid UTF-8 and unpaired surrogates; no panic path.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-lib.log`.

### M6-A3 — Token file parsing and validation matrix

- **Status:** `executed - passed`.
- **Target:** service unit tests.
- **Command:** `cargo test -p chronofork-service --lib`
- **Exit:** 0 — **45 passed**.
- **Observed:** a valid `chronofork.tokens.v1` document parses; empty, malformed, duplicate
  token, duplicate name, bad name, short and long token, unknown scope, empty scope, unknown
  field and non-object principal are typed `invalid_request`; oversized and non-UTF-8 input
  are typed. Pass 2 adds the R9 test that `TokenTable`'s `Debug` output names principals but
  never renders token material.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-lib.log`,
  `rows2/focus-R9a.log`.

### M6-A4 — Scope enforcement

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-service/tests/service.rs`.
- **Command:** `cargo test -p chronofork-service --test service`
- **Exit:** 0 — **22 passed**.
- **Observed:** a read-only principal on `session.create` → `forbidden`; an unknown token
  yields `authenticate() == None` (the transport then answers 401, exercised in M6-A12);
  `admin` implies read and write; no scope escalation through results.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-test.log`.

### M6-A5 — Session ownership

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-service/tests/service.rs`.
- **Command:** `cargo test -p chronofork-service --test service`
- **Exit:** 0 — **22 passed**.
- **Observed:** all eight session operations return `forbidden` for a foreign session; an
  admin may observe and mutate another principal's session; `session.list` filters by owner
  (operator 1, admin 2, other 0); closed sessions leave the list.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-test.log`.

### M6-A6 — Catalog and input validation

- **Status:** `executed - passed`.
- **Target:** service unit and operation tests.
- **Command:** `cargo test -p chronofork-service --lib` and
  `cargo test -p chronofork-service --test service`
- **Exit:** 0 / 0 — **45 + 22 passed**.
- **Observed:** exactly the 16 documented operations in catalog order with mutating flag,
  scope and input schema (`additionalProperties: false`, required list); unknown op →
  `unknown_op`; unknown field, wrong type, missing required field and out-of-range integer →
  `invalid_request`; the checkpoint list is bounded before work. The bound catalog is
  asserted by `the_catalog_is_exactly_the_sixteen_documented_operations_in_order`, which
  pass 2 re-checked against architecture §3.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-lib.log`,
  `rows2/service-test.log`, `rows2/r4-r5-doc-vs-code.log`.

### M6-A7 — Session lifecycle determinism

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-service/tests/service.rs`.
- **Command:** `cargo test -p chronofork-service --test service`
- **Exit:** 0 — **22 passed**.
- **Observed:** the lifecycle is compared against an independent `SessionManager` oracle:
  tick 0/10/15, restore at tick 10, fork at tick 10 with depth 1 and origin snapshot 2,
  compare equivalent → divergent at 10/13, `read_memory` hex equal to the oracle; a 17-step
  sequence is byte-identical across two fresh services; a halted run reports
  `steps=2, halted=true` and then `halted`.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-test.log`.

### M6-A8 — Writer-lease concurrency

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-service/tests/service.rs`.
- **Command:** `cargo test -p chronofork-service --test service`
- **Exit:** 0 — **22 passed**.
- **Observed:** with a gated backend the second same-session mutation fails immediately with
  `conflict` and `retryable:true` while the first succeeds; mutations of different sessions
  both succeed (`tick=4` each); a read succeeds while a writer lease is held and observes
  `tick=1`.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-test.log`.

### M6-A9 — Durable store operations

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-service/tests/service.rs`.
- **Command:** `cargo test -p chronofork-service --test service`
- **Exit:** 0 — **22 passed**.
- **Observed:** `store.save` new (key 1, tick 10) and continue (snapshot 2, tick 15),
  `store.list` (two snapshots with `creation_sequence`), `store.load` (tick 20; snapshot 1
  replays to tick 10) and `store.delete` with the documented non-cascading semantics:
  `conflict` while snapshots remain, `not_found` for an unknown key. A writer lock held
  elsewhere → retryable `conflict`; corrupt metadata → the anomaly label
  `storage metadata mismatch` and `storage_error` on load; the canonical bytes contain no
  host path and no advisory timestamp.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-test.log`.

### M6-A10 — Experiment artifacts

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-service/tests/service.rs` and recorded process evidence.
- **Command:** `cargo test -p chronofork-service --test service`, plus the recorded
  `experiment.run` process comparison.
- **Exit:** 0 — **22 passed**.
- **Observed:** repeated identical `experiment.run` yields identical artifact documents and
  ids; `artifact_id` equals the SHA-256 of the canonical artifact bytes
  (`1da41a1f4f2b63be537bde55b8896fca510085ca15c7a5cf8d8f0387c6590de1`); checkpoint
  ticks/hashes equal the oracle (`[(0,0),(2,2),(4,4)]`, final step/tick `4`);
  invalid checkpoint plans → `invalid_request`; persistence is atomic (one file, no temp
  left behind); the artifact contains no host time, path or principal; the operation
  allocates no live session id.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-test.log`,
  `.agents/notes/m6/verify/parity/experiment-parity-pass2.log`.

### M6-A11 — HTTP happy path and headers

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/http.rs`.
- **Command:** `cargo test -p chronofork-transport --test http`
- **Exit:** 0 — **21 passed**.
- **Observed:** `GET /health` returns 200 with exactly the five documented headers
  (unauthenticated); `GET /v1/tools` lists 16 tools in catalog order; `POST /v1/ops` returns
  the same bytes as a direct `dispatch`; one response per connection.
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-http.log`.

### M6-A12 — HTTP rejection matrix and authentication

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/http.rs`.
- **Command:** `cargo test -p chronofork-transport --test http`
- **Exit:** 0 — **21 passed**.
- **Observed:** 19-case rejection matrix — request line, version and target, header without a
  colon, missing and duplicate `Content-Length`, `Transfer-Encoding: chunked`, wrong and
  missing `Content-Type` (415), unknown route (404), query string (400), wrong method with
  `Allow`, missing and unknown bearer token and a non-bearer scheme (401).
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-http.log`.

### M6-A13 — Error code to HTTP status mapping

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport` unit tests and `tests/http.rs`.
- **Command:** `cargo test -p chronofork-transport --lib` and
  `cargo test -p chronofork-transport --test http`
- **Exit:** 0 / 0 — **9 + 21 passed**.
- **Observed:** the status table asserts all 13 `ErrorCode`s (including `internal_error` →
  500); the four framing codes are transport-owned and carry their documented status and
  envelope, encoded through the single service envelope builder; an operation produces each
  producible status (400 for `invalid_request`/`unknown_op`/`limit_exceeded`, 409 for
  `closed`/`halted`/`conflict`, 500 for `backend_error`/`storage_error`, 503 for
  `not_configured`, 404 for `not_found`, plus 401/403/405/415/413). Pass 2 adds three
  transport-lib tests proving `ServerConfig`'s `Debug` never renders bearer tokens (R9).
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-lib.log`,
  `rows2/transport-http.log`, `rows2/focus-R9a.log`.

### M6-A14 — MCP protocol subset

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/mcp.rs`.
- **Command:** `cargo test -p chronofork-transport --test mcp`
- **Exit:** 0 — **17 passed**.
- **Observed:** `initialize` returns protocol `2025-03-26` with serverInfo;
  `notifications/initialized` returns 202 with an empty body; `ping` returns `{}`;
  `tools/list` returns 16 tools with MCP field names; `tools/call` returns success and an
  `isError` result; unknown tool → -32602, batch → -32600 (400), parse error → -32700 (400),
  unknown method → -32601; `GET`/`DELETE`/`PUT /mcp` → 405 with `Allow: POST`; no
  `Mcp-Session-Id` header and never an `event:` stream.
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-mcp.log`.

### M6-A15 — MCP/HTTP envelope equality

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/mcp.rs` and recorded live-daemon parity.
- **Command:** `cargo test -p chronofork-transport --test mcp`, plus the recorded
  `api call` versus `mcp call` comparison.
- **Exit:** 0 — **17 passed**; both live requests exit 0.
- **Observed:** the `tools/call` text content is byte-identical to the direct `dispatch`
  bytes (after JSON-string decoding) and to the HTTP `POST /v1/ops` envelope; the live
  `capabilities` body is 1506 bytes with SHA-256
  `d5f8282e03c9eec75d0568a2829bb1790174f6e2762acfe0765d6676eee73a74` on both surfaces,
  identical to pass 1.
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-mcp.log`,
  `.agents/notes/m6/verify/parity/manual-http-vs-mcp-pass2.log`.

### M6-A16 — Server bounds and shutdown

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/http.rs` plus a live-daemon timeout probe.
- **Command:** `cargo test -p chronofork-transport --test http`, plus the recorded
  `io_timeout` process scenario.
- **Exit:** 0 — **21 passed**; the probe daemon exited 0.
- **Observed:** fixed workers (4 by default, `--workers 0` rejected), bounded accept queue
  (16; one worker served 8 clients), `max_requests` stops the server and `wait`/`shutdown`
  join, request line 8192, header line 8192, more than 64 headers rejected, bounded head and
  body (413 at bound−1, 200 at bound, declared length rejected before reading). `io_timeout`
  of 5 s closed an idle connection after **5.24 s** with 0 bytes while the daemon kept
  serving `/health` with 200 and exited 0 (pass 1: 5.13 s).
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-http.log`,
  `.agents/notes/m6/verify/rows2/a16-io-timeout.log`.

### M6-A17 — Typed client

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/client.rs`.
- **Command:** `cargo test -p chronofork-transport --test client`
- **Exit:** 0 — **10 passed**.
- **Observed:** `health`, `tools`, `call`, `call_raw` and the MCP `initialize`/`list_tools`/
  `call_tool` paths work against a live in-process server; 404/400/401 surface
  `TransportError::Http { status, body }` with the canonical envelope; URL rejection happens
  before connect; a refused connection is `Io`; the response cap, truncated body and
  oversized head are `Protocol`; unknown MCP tool is -32602. Pass 2 adds the R9 tests that
  the HTTP and MCP client `Debug` output never renders the bearer token.
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-client.log`,
  `rows2/focus-R9b.log`.

### M6-A18 — Direct/HTTP/MCP parity, all operations

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/parity.rs`.
- **Command:** `cargo test -p chronofork-transport --test parity`
- **Exit:** 0 — **3 passed**.
- **Observed:** the architecture §11 23-step scenario on three fresh direct/HTTP/MCP
  surfaces covers all 16 operations; every step is byte-identical (MCP after text decoding);
  a repeated fresh run is identical.
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-parity.log`.

### M6-A19 — Direct/HTTP/MCP parity, error scenarios

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-transport/tests/parity.rs`.
- **Command:** `cargo test -p chronofork-transport --test parity`
- **Exit:** 0 — **3 passed**.
- **Observed:** unknown session `not_found`, invalid input `invalid_request`, unknown op
  `unknown_op` on HTTP versus -32602 on MCP, foreign session `forbidden`, unknown token 401
  with an identical envelope, wrong content type 415, unknown route 404, body too large 413
  and batch -32600 all match across the applicable surfaces.
- **Evidence:** `.agents/notes/m6/verify/rows2/transport-parity.log`.

### M6-A20 — CLI serve process flow

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/m6.rs` plus a recorded two-daemon scenario.
- **Command:** `cargo test -p chronofork-cli --test m6`
- **Exit:** 0 — **11 passed** (pass 1: 9).
- **Observed:** `chronofork serve` validates the token file, prints `record=serve`,
  `bind=`, `backend=synthetic|megadrive`, `store=` and `artifacts=`, serves HTTP and MCP, and
  exits 0 after `--max-requests`; a malformed token file refuses to start; without a limit it
  keeps running until terminated. Recorded process run: two daemons on ephemeral loopback
  ports served their requests and exited 0. Pass 2 adds the R2 and R3 CLI tests to this
  target; the serve/api/mcp flows are unchanged.
- **Evidence:** `.agents/notes/m6/verify/rows2/cli-m6.log`,
  `.agents/notes/m6/verify/rows2/a30-daemon-restart.log`.

### M6-A21 — CLI api client parity

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/m6.rs`.
- **Command:** `cargo test -p chronofork-cli --test m6`
- **Exit:** 0 — **11 passed**.
- **Observed:** `api health|tools|call` bytes equal the typed client/direct service bytes
  plus one newline; `--principal operator|other|admin` selects the documented principal and
  ownership is visible; an input document can come from stdin; an unknown op prints one
  canonical `chronofork.error.v1` envelope line on stderr with empty stdout and exit 1.
- **Evidence:** `.agents/notes/m6/verify/rows2/cli-m6.log`.

### M6-A22 — CLI mcp client parity

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/m6.rs`.
- **Command:** `cargo test -p chronofork-cli --test m6`
- **Exit:** 0 — **11 passed**.
- **Observed:** `mcp tools` equals the canonical `tools/list`; `mcp call` text equals the
  HTTP envelope bytes; an `isError` result exits 1 with empty stdout and the canonical error
  envelope on stderr.
- **Evidence:** `.agents/notes/m6/verify/rows2/cli-m6.log`.

### M6-A23 — CLI session JSON parity

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/m6.rs` and `tests/session_storage.rs`.
- **Command:** `cargo test -p chronofork-cli --test m6` and
  `cargo test -p chronofork-cli --test session_storage`
- **Exit:** 0 / 0 — **11 + 18 passed**.
- **Observed:** `session save|load|list|delete --json` bytes equal the direct service
  `store.save/load/list/delete` envelopes on fresh roots, exactly one line; the `key=value`
  record shape of the human `session list` output is unchanged, and all 18 pre-existing
  `session_storage` tests stay green. **Intentional change recorded by the repair (R2):**
  when the service envelope reports `truncated:true`, the human `session list` now prints
  exactly one stderr line, `chronofork: storage listing truncated at 256 entries`; the
  stdout record shape is unchanged and `--json` remains silent with `truncated:true` in the
  envelope. The verifier's independent 129-session probe produced 256 `record=` blocks, that
  exact stderr line and `--json` `truncated:true` with empty stderr.
- **Evidence:** `.agents/notes/m6/verify/rows2/cli-m6.log`,
  `.agents/notes/m6/verify/rows2/cli-session-storage.log`, `rows2/focus-R2.log`,
  `rows2/independent-finding-checks.log`.

### M6-A24 — CLI experiment artifact parity

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/m6.rs` plus recorded process parity.
- **Command:** `cargo test -p chronofork-cli --test m6`
- **Exit:** 0 — **11 passed**.
- **Observed:** `chronofork experiment run --out <FILE>` writes exactly the service artifact
  bytes and prints nothing; without `--out` it prints the artifact plus one newline; a
  repeated run is identical. Recorded: the service `artifact_id`
  `1da41a1f4f2b63be537bde55b8896fca510085ca15c7a5cf8d8f0387c6590de1` equals
  `sha256(canonical artifact)` equals `sha256(--out file)`; the artifact contains no host
  path, time or principal.
- **Evidence:** `.agents/notes/m6/verify/rows2/cli-m6.log`,
  `.agents/notes/m6/verify/parity/experiment-parity-pass2.log`.

### M6-A25 — CLI rejection and exit codes

- **Status:** `executed - passed`.
- **Target:** `crates/chronofork-cli/tests/m6.rs`.
- **Command:** `cargo test -p chronofork-cli --test m6`
- **Exit:** 0 — **11 passed**.
- **Observed:** exit 1 with empty stdout and one stderr diagnostic for: missing token file,
  malformed token file, unknown principal, invalid URL, refused connection, invalid
  `--input`/`--input-file` JSON, unknown op (remote canonical envelope), server-side typed
  failures, unknown or missing flags and values, `--workers 0` and `--max-requests 0`.
  **Intentional changes recorded by the repair (R3, R8):** locally produced and human
  failures now use the base `chronofork: <message>` prefix — for example
  ``chronofork: token file `<path>` exceeds the 1048576-byte limit`` — and typed codes appear
  only inside the canonical `chronofork.error.v1` envelopes that `api`, `mcp` and `--json`
  emit; the token file is read with exactly the configured request bound, so an oversized
  file is one clear diagnostic rather than an unbounded read.
- **Evidence:** `.agents/notes/m6/verify/rows2/cli-m6.log`, `rows2/focus-R3.log`,
  `rows2/focus-R8.log`, `rows2/independent-finding-checks.log`.

### M6-A26 — Transport-boundary audit

- **Status:** `executed - passed`.
- **Target:** source and dependency audit.
- **Command:** greps for transport types in `crates/chronofork-core`,
  `crates/chronofork-backend-api`, `crates/chronofork-sessions` and
  `crates/chronofork-service`; `cargo tree`; `git diff Cargo.lock`; `rg unsafe`.
- **Exit:** 0.
- **Observed:** no `std::net`, HTTP, MCP or JSON-RPC type appears in core, backend-api,
  sessions or service (only doc-comment words); the dependency direction is
  `core ← backend-api ← sessions ← service ← transport` with the CLI composing all layers;
  no `unsafe`; the lockfile grows from 40 to 42 packages and adds only the two new workspace
  crates (no third-party change); there is no outbound connect except the client's explicit
  `--url`. Pass 2 re-ran the same audit with unchanged results.
- **Evidence:** `.agents/notes/m6/verify/audit/a26-boundary.txt`,
  `audit/a26-manifests.txt`, `audit/a26-cargo-lock-diff.txt`, `audit/a26-tree.txt`.

### M6-A27 — Capability and claim honesty

- **Status:** `executed - passed` (code-side, contract and documentary claim honesty;
  confirmed by the independent documentation audit t17 PASS, 2026-11-17).
- **Target:** requirements, ADR and architecture audit (code and contract), plus the
  documentation-claim audit owned by the documentation task.
- **Command:** recorded document/code audit.
- **Exit:** — (audit, not a test target).
- **Observed:** ADR-0012 and architecture §1–§12 match the code (catalog, routes/headers, MCP
  subset, limits, ownership, concurrency, artifacts); there is no TLS, SSE, streaming or
  trace-over-transport claim — all four are listed under "Not implemented in M6" and tracked
  as M6-FU1–FU4. The **code-side and contract claims are backed by the t13 pass-2 audit**
  (`.agents/notes/m6/verify/audit/a27-honesty.txt`, with the pass-2 spot checks in
  `audit/a28-r67-pass2.txt`). The **documentary claims are a separate matter**: at pass 2 the
  tracked documents under-claimed the delivered state (R6/R7); the documentation task
  updated them; the independent documentation audit then found **M6-DOC-1–M6-DOC-6**
  against that update. The documentation repair (**t16**) closed all six, and the
  independent documentation re-audit (**t17**) returned **PASS** on 2026-11-17 (0 missing
  links/anchors; claims consistent with the code and the pass-2 evidence), so documentary
  claim honesty is now independently confirmed.
- **Evidence:** `.agents/notes/m6/verify/audit/a27-honesty.txt`,
  `.agents/notes/m6/verify/audit/a28-r67-pass2.txt`, `audit/a27-a29-limits.txt`; independent
  documentation audit record t17 PASS (2026-11-17).

### M6-A28 — Full gate and non-regression

- **Status:** `executed - passed`.
- **Target:** repository gate.
- **Command:** `./scripts/check.sh`, plus both demos three times each, plus the
  weakening/deletion audit.
- **Exit:** 0.
- **Observed:** **67 targets / 800 passed / 0 failed / 0 ignored**; the 57 pre-existing
  targets are all present and contribute exactly 662 passed; no test was removed (the
  executed set is a strict superset: +7, −0); no `#[ignore]` anywhere; no new `#[allow]`
  beyond the pre-existing `megadrive/interrupts.rs` attribute and the repository's
  documented test-only header convention; per-file `#[test]` counts versus HEAD unchanged;
  both demos are 3/3 byte-identical to the baseline (`9b4dd564…b84652`,
  `912a5bb6…348996`) with empty stderr.
- **Evidence:** `.agents/notes/m6/verify/gate/check-sh-pass2.log`,
  `gate/per-target-pass2.txt`, `.agents/notes/m6/verify/demos2/`,
  `audit/a28-weakening-pass2.txt`, `audit/a28-tracked-diffs.txt`,
  `audit/executed-tests-pass2.txt`, `audit/a28-assertion-spotcheck-pass2.txt`.

### M6-A29 — Bounded-resource evidence

- **Status:** `executed - passed`.
- **Target:** service and transport tests.
- **Command:** `cargo test -p chronofork-service` and
  `cargo test -p chronofork-transport`
- **Exit:** 0 / 0 — **67 + 60 passed**.
- **Observed:** every `ServiceLimits` field is exercised and rejected before unbounded work
  (max_sessions, max_steps_per_request, max_memory_read, max_store_entries with `truncated`,
  max_checkpoints, max_response_bytes, max_request_bytes, json_max_depth) and every wire
  bound (request line, header line, header count, head, body, workers, accept queue) is
  rejected or queued as documented; `io_timeout` is exercised at process level. **Intentional
  change recorded by the repair (R1):** `max_sessions` counts **all registered sessions
  including closed ones**; closed sessions stay unlisted and non-mutatable
  (`inspect`/`run` → `closed`) and still count toward the bound, while
  `AppService::session_count()` continues to report live sessions only. The dedicated test
  drives `max_sessions = 2`: create twice, then create and fork → `limit_exceeded`; close
  both, create and fork still `limit_exceeded`; `session.list` empty; `session_count` 0.
  Architecture §3.2 and §7 state the same rule.
- **Evidence:** `.agents/notes/m6/verify/rows2/service-all.log`,
  `.agents/notes/m6/verify/rows2/transport-all.log`,
  `.agents/notes/m6/verify/rows2/focus-R1.log`,
  `.agents/notes/m6/verify/rows2/a16-io-timeout.log`.

### M6-A30 — Daemon restart and durability

- **Status:** `executed - passed`.
- **Target:** CLI/transport process evidence.
- **Command:** recorded two-daemon process scenario (same `--store-root`, two requests on
  the second daemon).
- **Exit:** 0 (both daemons).
- **Observed:** daemon #1 `store.save` → `session_key=1`, `snapshot_key=1`, `tick=5`,
  `state_hash=a3b2e28fa318b99e8230eaa9c4e5635c97d954d189f419b4919844c129b66fdb`; daemon #2
  in a fresh process with the same `--store-root` `store.load` returned the identical
  `session_key`/`snapshot_key`/`tick`/`state_hash`; `session.list` on daemon #2 returned
  `{"sessions":[],"truncated":false}`, so in-memory sessions are absent after a restart as
  documented. Identical to pass 1.
- **Evidence:** `.agents/notes/m6/verify/rows2/a30-daemon-restart.log`.

### M6-A31 — Security and no-egress audit

- **Status:** `executed - passed`.
- **Target:** source and process audit.
- **Command:** recorded audit.
- **Exit:** — (audit, not a test target).
- **Observed:** tokens come only from `--token-file` (no `--token` and no token through argv
  or environment; the only environment read in the tree is the pre-existing M2 lock
  `host_label()`); authentication is required on every route except `/health`, which is
  tested with an empty table; error envelopes are bounded, ASCII, path-free and token-free;
  no request field can name a path or ROM (the catalog has value fields only) and `--rom` is
  startup-only; there is no telemetry, upload or outbound code (`Command::new` appears only
  in tests); the default bind is `127.0.0.1:0` with a warning for a non-loopback address.
- **Evidence:** `.agents/notes/m6/verify/audit/a31-security.txt`,
  `audit/a26-boundary.txt`.
- **Superseded (2026-11-27):** this baseline warned on a non-loopback bind; the CLI and
  transport now refuse it before serving (see the 2026-11-27 implementation status in
  [ADR-0014](../adr/0014-v1-scope-single-user-loopback.md)). The observation above records
  the M6 baseline and is not a current claim.

### M6-A32 — Gate cleanliness and thread safety

- **Status:** `executed - passed`.
- **Target:** repository gate and documentation build.
- **Command:** `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, and
  `cargo test -p chronofork-service --test service app_service_is_send_and_sync -- --exact`.
- **Exit:** 0 / 0 / 0 / 0 / 0.
- **Observed:** all gates clean; the compile-time assertion proves `AppService`,
  `Principal` and `ServiceError` are `Send + Sync`, so the daemon may share the service
  across transport workers under its documented synchronization boundary.
- **Evidence:** `.agents/notes/m6/verify/rows2/a32-fmt.log`, `rows2/a32-check.log`,
  `rows2/a32-clippy.log`, `rows2/a32-doc.log`, `rows2/a32-send-sync.log`,
  `rows2/a32-commands.txt`.

## 5. Verification history and finding closure

### 5.1 Findings M6-R1–M6-R10 (round-1 review)

| Finding | Outcome | Executed evidence |
|---|---|---|
| **M6-R1** `max_sessions` counted only live sessions | **Fixed** — `Registry::registered_count()` gates create and fork; `live_count()` only feeds `session_count()` | `rows2/focus-R1.log`, M6-A29; architecture §3.2/§7 match |
| **M6-R2** human `session list` hid truncation | **Fixed** — exact stderr notice at the service `max_store_entries` bound, stdout shape unchanged, `--json` keeps `truncated:true` | `rows2/focus-R2.log`, `rows2/independent-finding-checks.log`, M6-A23 |
| **M6-R3** human/local errors carried a typed code prefix | **Fixed** — base `chronofork: <message>` prefix locally; typed codes only in canonical envelopes | `rows2/focus-R3.log`, `rows2/independent-finding-checks.log`, M6-A25 |
| **M6-R4** framing-code text implied service ownership | **Fixed as contract text** — architecture §4 now states the four framing codes are transport-owned and encoded through the single service envelope builder | `rows2/r4-r5-doc-vs-code.log` |
| **M6-R5** §3 table / `store.delete` text versus code | **Fixed as contract text** — the 16-row catalog table and the 13-code status table equal the code-bound tables; `store.delete` is mutating with the documented no-cascade semantics | `rows2/r4-r5-doc-vs-code.log` |
| **M6-R6** requirements/milestone text under-claimed M6 | **Updated by the documentation task** — FR-API-002/FR-API-003 promoted, FR-API-001 rewritten, M6 status updated; documentary correctness is subject to the independent documentation audit (M6-DOC-1–D6 repaired by t16; re-audit t17 PASS, 2026-11-17) | `audit/a28-r67-pass2.txt` (pre-promotion state), §6 below, M6-A27 |
| **M6-R7** no M6 block in the quality gates; no executed-row promotion | **Updated by the documentation task** — 2026-11-17 M6 gate block and the `executed - passed` rows above; documentary correctness is subject to the independent documentation audit (M6-DOC-1–D6 repaired by t16; re-audit t17 PASS, 2026-11-17) | [quality gates](quality-gates.md) 2026-11-17 block, §6 below, M6-A27 |
| **M6-R8** token file read was unbounded | **Fixed** — the read is bounded by the configured request limit and the parser keeps its own documented 1 MiB default | `rows2/focus-R8.log`, M6-A25 |
| **M6-R9** tokens rendered through `Debug` | **Fixed** — `TokenTable` names principals only, `ServerConfig` prints `tokens: "<redacted>"`, both clients print `token: "<redacted>"` | `rows2/focus-R9a.log`, `rows2/focus-R9b.log`, M6-A3/A13/A17 |
| **M6-R10** anti-slop exception record and lint weakening concern | **Fixed** — the CLI `tests/m6.rs` header carries the anti-slop exception record; no production `allow`, no `#[ignore]`, no test removed, per-file counts unchanged | `audit/a28-weakening-pass2.txt` |

### 5.2 Intentional behavior changes pinned by the repair

1. `max_sessions` counts all registered sessions including closed ones; closed sessions stay
   unlisted and non-mutatable and still occupy the bound. `session_count()` reports live
   sessions only.
2. The human `session list` prints exactly
   `chronofork: storage listing truncated at 256 entries` on stderr when the service reports
   `truncated:true`; stdout record shape is unchanged and `--json` keeps the envelope
   field.
3. Locally produced and human-facing failures print the base `chronofork: <message>`
   prefix; typed `code` fields appear only in canonical `chronofork.error.v1` envelopes
   (`api`, `mcp`, `--json`).
4. The invalid `--name` diagnostic wording now comes from the service catalog (for example
   `` `name` must not contain control characters ``) because validation moved into the
   shared service; this is the reason two assertions in
   `crates/chronofork-cli/tests/session_storage.rs` were re-pointed.
5. The four HTTP framing codes are documented as transport-owned and encoded through the
   single service envelope builder (R4), and `store.delete` is documented as mutating with
   no cascade (R5).

## 6. Promotion record

After the pass-2 independent PASS (2026-11-17) and the round-2 review `pass`, the
documentation owner promoted: the row statuses and verification record in this plan,
[ADR-0012](../adr/0012-agent-native-interfaces.md) indexing,
the [M6 milestone](../roadmap/milestones.md#m6--agent-interfaces) (`Complete`; merged to
`main` at `cb05836`, PR #6), the [roadmap Stage 6](../roadmap/roadmap.md#stage-6--agent-native-interfaces)
delivered paragraph, [functional requirements](../requirements/functional.md)
(`FR-API-001` updated, `FR-API-002`/`FR-API-003` **Implemented**, `FR-API-004` still
`Planned` for Stage 9), the [architecture overview](../architecture/overview.md) layer
diagram/dependency rules/implemented-versus-planned table, the [agent interfaces
architecture](../architecture/agent-interfaces.md) token-bound wording, the
[docs index](../README.md), the [ADR index](../adr/README.md), [README](../../README.md),
[AGENTS.md](../../AGENTS.md) and the dated [quality gates](quality-gates.md) block.

Promotion records evidence links and known limits; it never rewrites a failed row, converts
a planned test into a pass or claims a feature that was not executed. **The verified tree was
the worktree at branch `agent/m6-agent-interfaces` based on `origin/main` `93d47ed`; no
commit, merge, push or publication was authorized in this task at verification time. The
work was subsequently merged to `main` at `cb05836` (PR #6); M6 is `Complete`.**

**Documentation audit status.** The promotion above was produced by the documentation task,
and the independent documentation audit subsequently returned findings **M6-DOC-1–M6-DOC-6**
against it (attribution of the 19-case HTTP matrix, volatile counts in durable pages, an
inconsistent M7 status, an incorrect `AGENTS.md` dependency diagram, an over-eager M6-A27
closure and an over-broad Mega Drive coverage claim). The repair task (**t16**) closed all
six, and the independent documentation re-audit (**t17**) returned **PASS** on 2026-11-17
with 0 missing relative links and 0 bad anchors; the documentary claims of this promotion are
therefore independently verified.

## 7. Residual limits recorded by this plan

1. **No TLS, no SSE/streaming, no `Mcp-Session-Id`.** The HTTP server is HTTP/1.1 with one
   request per connection and no keep-alive or chunked bodies; `POST /mcp` is stateless
   (M6-FU3, M6-FU4).
2. **Ownership is process-local** and lost on daemon restart; durable store objects carry no
   per-principal owner, so they are governed by scope only (M6-FU2).
3. **Closed sessions are retained for the process lifetime** (the session layer has no
   removal API) and therefore keep counting toward `max_sessions`.
4. **Debug, trace, script and TAS operations are CLI-local**, not transport-exposed
   (M6-FU1). The catalog is deliberately curated, not a mirror of every library function.
5. **One backend configuration per daemon process** (synthetic, or Mega Drive with a startup
   `--rom`); there is no per-request backend selection or ROM path in a request.
6. **`io_timeout` is verified at process level** (5.24 s idle close) but has no automated
   timeout test; the other wire bounds are asserted by the automated suite.
7. **`internal_error` is table-tested but never produced by an operation**; it is a
   defect-only code.
8. **No performance claim.** The bounds are correctness/DoS limits, not throughput
   guarantees.
9. Verification ran on the Linux/WSL acceptance platform only; no cross-platform runtime
   claim is made.
10. **Documentation-scope note closed by the integration pass:** the doc comment on
    `ServiceLimits::max_sessions` in `crates/chronofork-service/src/limits.rs` now states the
    enforced rule (registered sessions including closed, with `session_count()` live-only),
    matching M6-A29 and architecture §3.2/§7. The change is comment-only with no behavior
    change; the service gates were re-run after it during integration.
