# ADR-0016: Session service — per-session execution and an agent-usable API

- **Status:** **Accepted** (owner direction recorded 2026-11-20: the recommended option —
  per-session lock/owner, cancel/deadline/quotas, a minimal composed agent API and live-session
  persistence — was selected; R2 F1–F10, R5 K1–K16 and critique C1 corrections applied
  2026-11-20; independent review V5 passed 2026-11-20). This accepts the service/API decision;
  it does not certify implementation or security.
- **Date:** 2026-11-20.
- **Milestone:** M8 (functional Mega Drive emulator) and the product-consolidation phase.
  Consolidation is a **phase of the open M8**; it does not close M8 (owner decision
  2026-11-20).
- **Related:** [ADR-0014](0014-v1-scope-single-user-loopback.md) (v1 scope),
  [ADR-0015](0015-verification-contract.md) (execution/verification split),
  [ADR-0012](0012-agent-native-interfaces.md) (existing service/transport boundary —
  **§5 concurrency is superseded here**: the manager lock leaves the step path).
- **Review evidence:** independent review 2026-11-20, baseline `b8d63a1`. The durable report is
  in this tree at
  [`docs/reviews/2026-11-20-independent-review/REVIEW.md`](../reviews/2026-11-20-independent-review/REVIEW.md).
  Consolidation reviews: `.research/consolidation-2026-11-20/reviews/adr-0016-interfaces.md` (R2),
  `adr-0016-concurrency.md` (R5), `critique-consolidation.md` (C1) and the V2 review
  `review-corrected-adrs.md`. The consolidation use cases (UC-1…UC-4) are normatively defined in
  [ADR-0014](0014-v1-scope-single-user-loopback.md#kidengine-flow-and-consolidation-use-cases).

## Context

`AppService` keeps `Mutex<SessionManager>`, `Mutex<Registry>` and `Mutex<LeaseTable>`
(`crates/chronofork-service/src/service.rs:114-122`). `session.run` holds the manager lock
for the whole step loop (`:461-491`), up to `max_steps_per_request = 100_000`
(`crates/chronofork-service/src/limits.rs:37`). The independent review measured:

- aggregate throughput ~5,066 / 4,514 / 4,593 / 4,629 steps/s for 1/2/4/8 sessions; efficiency
  **1.00 / 0.446 / 0.227 / 0.114** (by throughput, corrected from the synthesis's earlier
  numbers). CPU/wall ~0.96–0.98: serialization;
- a 100,000-step run takes ~9–24 s of global lock; the typed client's read timeout is 5 s
  (`client.rs:20-22`), so the client can fail while the server still applies state (unknown
  outcome);
- `close` during a run is rejected with `conflict`; there is no `cancel`/`pause` operation;
  a client disconnect does not cancel;
- the registry has no removal API; `max_sessions` is a lifetime limit; a closed session keeps
  its backend (~387.6 KiB each; ~397 MiB at 1,024);
- `experiment.run` uses a private manager and scales 5.97× on 8 threads — the backend is
  parallelizable; the bottleneck is the service design;
- the MCP catalog has 16 operations; debug/trace/script/TAS are not exposed (M6-FU1); a
  10,000-step observation over MCP costs ~20,000 calls, ~20.7 MB, ~5.2M tokens (bytes/4
  extrapolation) and ~103 s, versus ~1.04 s of emulation;
- there is no service operation to persist/export a **live, branched session**; `store.save`
  is a stateless durable workflow that rehydrates from a committed checkpoint and does not
  accept a live session.

## Problem

How should the service own sessions and expose them so that independent experiments can run
without global serialization, long operations can be cancelled or reconciled, and an agent can
perform the KidEngine workflow without thousands of round trips?

## Decision

### 1. One execution owner per session; no global manager lock on the step path

Replace the global `Mutex<SessionManager>` execution path with a registry that owns sessions
behind a per-session `Mutex`/owner (or a command queue/actor per session). The registry may
be sharded or use a concurrent map; the exact structure is an implementation detail, but the
**safety invariants below are contract** (critique C1 §5: they are preconditions, not
polish).

- `run`/`step`/`snapshot`/`restore` take only the target session's owner.
- **Lock protocol (K5/V2-2), with no global wait inside the registry:** the **registry lock is
  never held while waiting for a session owner**, and the session owner is never acquired while
  holding the registry.
  1. take the registry, look up/copy the session **handle** (an `Arc` keeps it alive), the
     session generation and the metadata, then **release the registry**;
  2. acquire the target session's owner with the registry free;
  3. after releasing the owner, re-take the registry if the operation must mutate the registry
     (`fork` insert, `close`/`reap` remove), re-validating the generation.
  This removes the "owner after registry" wait and with it the deadlock class K5: a run holds
  only its own session owner, so `list`, `inspect` of other sessions, `create` and `cancel` are
  never blocked by a global lock.
- **`fork`/`close` are multi-lock but never invert:** read the source handle (phase 1), acquire
  the source owner (phase 2) if the fork needs a consistent source snapshot, then release it and
  take the registry again for the insert/remove. A test exercises `fork`/`close` under a running
  session.
- **Registry admission is atomic and does not wait for a run (K6/V2-2):** `max_live_sessions`
  is checked-and-inserted in one critical section; the insert is of a handle, never of an owner
  wait. `create` therefore never queues behind a running session.
- **`list` is honest and does not verify (K1/K8/V2-2).** `list` returns **cheap metadata that
  requires no session owner**: id, state, generation, tick as of the last committed boundary,
  owner principal, backend id, live/dead flag. It does **not** hash and does not copy memory.
  - *Freshness contract:* the tick/metadata are as of the last completed mutating operation on
    that session; a `list` issued while a session runs reports the **pre-run** metadata, and the
    response marks the entry `running` (so a caller can distinguish "stale because running" from
    "stale because nothing happened"). Omission is possible only for sessions whose handle was
    reaped from the registry; `reap` is what makes them disappear, and that is observable through
    the generation.
  - *Dependency:* the cheap metadata (tick without `inspect`) is **W2.4 or an equivalent
    metadata field cached at each commit**; it is a declared dependency of this ADR, not an
    assumption.
  - `list` never waits for a running session: it reports metadata or the running marker, and it
    cannot exceed the transport's request deadline.
- **Admission and cancellation must not compete for the whole pool (F4/K10/V2-2/V3-1).** The
  transport has a **single synchronous worker pool of `N` workers** and a synchronous handler
  (`crates/chronofork-transport/src/http.rs:275-434`), so capacity is an explicit model, not an
  assumption:
  - **`N ≥ 2` is required.** The daemon **refuses to start** with `workers = 1` unless the
    operator selects the separate control path below; the refusal is a typed startup error and
    is tested.
  - `max_long_runs_global` defaults to **`N − 1`**: at most `N − 1` long runs may be admitted,
    reserving at least one worker for control/admission/classification. Admission happens
    **before** the long work is handed to the pool, so a request that would exceed the limit is
    rejected with a typed `limit` instead of occupying a worker while it waits.
  - `max_long_runs_per_principal` defaults to 1, so one principal cannot take the reserved
    capacity either.
  - The **control path** (`cancel`, `progress`, `list`, `capabilities`) is served by the
    reserved worker capacity and touches only the operation journal and the cancel flag — never
    a session owner or a stepping loop. This is what makes the reservation effective: the
    atomic cancel token makes `cancel` cheap, but **the token alone does not create a worker to
    serve the request**; the reservation (or the separate pool) does.
  - **Alternative (equivalent contract):** a **separate bounded control pool** for
    control/admission operations, in which case the long-run pool may be sized differently. If
    this option is chosen, the same numeric bounds and tests below apply.
  - **Numeric acceptance target (V4-2 — a goal to measure, not a current measurement):** with
    the reserved capacity, the acceptance target is that a `cancel` and a `list` issued while
    `N − 1` long runs are in flight complete within **250 ms p95** on a calibrated runner, and
    that the `N`-th long run is rejected within the same target. This number is a **target to
    be measured by the acceptance test**, not a measured result; if the measured value
    regresses, the admission limit is lowered rather than the target kept. A saturation test
    with **multiple principals** (four long-run **attempts** + one `list` + one `cancel`) must
    show control executable and the excess rejected; with the default `workers = 4` and
    `max_long_runs_global = 3`, **three are admitted and the fourth attempt is rejected**
    (the four attempts are not four admitted runs).
  - The accept policy (`accept` poll or blocking) is documented with a measured latency floor;
    it must not consume the reserved capacity.
- **Client timeout is decided (V3-1):** the typed client's 5 s read timeout
  (`crates/chronofork-transport/src/client.rs:19-22`) is replaced for long operations by a
  timeout **above the 30 s server deadline plus an explicit margin for the final
  response/hash** (v1 contract). Poll/asynchronous delivery of long runs is **out of scope for
  v1**; if it is ever chosen, it replaces this rule rather than coexisting as an ambiguity.
- `SessionManager` keeps its documented synchronous `&mut self` shape internally; the service
  is still the single synchronization boundary.
- No Tokio/async runtime is required. A per-session owner is the smallest correct step; an
  actor/worker per session is a later evolution if measurements demand it.
- **This supersedes ADR-0012 §5's manager-lock step path (K11).** The "reads remain
  available" wording in `docs/adr/0012-agent-native-interfaces.md:110`,
  `docs/architecture/overview.md:198-210` and
  `docs/architecture/agent-interfaces.md:444-445` is removed unless a measured bound backs it
  (W3.7).

### 2. Cancellation, deadline, progress and reconciliation

- **Cancellation is lock-free and observable during a run (K2/F3).** The token is a shared
  atomic signal read by the stepping loop at fixed instruction/batch boundaries **without
  taking the session owner**; `cancel` therefore never queues behind the run it interrupts.
  Authorization is the same principal/lease as the run; on cancellation the lease is released
  and the state transition is defined. The acknowledgment does not promise a step count: the
  authority is the final `performed`/`tick`/stop reason, carried even when the envelope is a
  typed error.
- **New wire error codes are versioned:** `cancelled` and `deadline` join the typed error set
  (`crates/chronofork-service/src/error.rs:17-44`) with a declared wire version.
- `run` accepts `max_wall_ms` as a host policy and returns `performed`, `tick`, `halted`, the
  stop reason and, when requested, a final `state_hash`. Host time never changes canonical
  evolution: **deadline chooses `k`; replay records `k` + inputs (K3).**
- **Reconciliation replaces "exactly once" (K3/C1 §4).** Tick is not a monotonic revision:
  `restore`, `reset` and `write_memory` can move state backwards or leave the tick unchanged
  (ABA; two writes at one tick). Therefore:
  - a **monotonic session revision/generation** is the precondition for retry
    (`expected_revision`, plus `expected_tick` where the tick is genuinely meant);
  - a client that must retry safely supplies a **request key known before the mutation**
    (bound to principal + session + argument digest). A key first revealed in a lost response
    is useless;
  - the **operation journal is bounded and never evicts an active operation** (K4/F5): the
    configured `max_operation_journal` (default 1,024) and `operation_ttl` (default 600 s),
    FIFO **only for finished** entries, a typed `limit` rejection when full, `running` distinct
    from `finished`. The journal lives in process memory.
  - an in-memory journal entry is recognized by `(key, digest)` only while it is retained; a
    duplicate request with the same key and a **different digest** is a typed conflict, not a
    silent re-apply.
- **Restart and expiry are unknown outcomes, not replayable ones (V2-3/V3-3).** The ADR does
  not promise reconciliation across a restart. Precisely:
  - **process epoch:** a monotonic process/generation identifier, reported in every result and
    in `capabilities`. It is **never reused across processes**: the counter is persisted
    (durably, e.g. in the store root or alongside the token file) and only increases, so a
    newly started process cannot mint an epoch that a client could confuse with an earlier one.
    A session retains its own monotonic revision across restarts only if it is durably persisted
    — live ownership and live sessions are process-local (ADR-0014);
  - **TTL starts at completion, not at submission:** an active (running) operation is **never**
    expired; `operation_ttl` (default 600 s) is counted from the moment the operation reaches a
    final state. A finished entry is recognized by `(key, digest)` while retained **and while
    the epoch matches**;
  - **FIFO pressure is explicit:** when the journal is full, the implementation either
    (a) rejects new journal insertions with a typed `limit` while guaranteeing a documented
    **minimum retention** for finished entries, or (b) shortens retention of the oldest
    **finished** entries only. It never evicts a running entry;
  - after a restart, or beyond the completed-entry TTL, a request key is **not** recognized: the
    outcome is `unknown_outcome`, and the server **must not re-execute automatically**. A retry
    is a caller decision, made safe by `expected_revision` (which detects staleness but does
    **not** recover a lost result) or by inspecting durable state;
  - the client-facing contract says exactly that: query the result while the entry is retained
    in the same epoch; outside that horizon the outcome is `unknown_outcome`, with no automatic
    re-execution. The previous unconditional reconciliation sentence in §6 is replaced by this
    horizon (V3-3);
  - Required tests: restart mid-operation (unknown outcome, no auto-retry), expiry of a key
    (counted from completion, active operations never expiring), and duplicate key with a
    different digest.
- `notifications/cancelled` and `progressToken` are implemented with real semantics or
  explicitly documented as unsupported; a 202 with no effect is not acceptable. `progress`
  is a **metadata read of the operation journal**, not a session read.

### 3. Quotas and lifecycle

- **Enforced now (strict, before allocation):** `max_snapshots_per_session`,
  `max_bytes_per_session`, `max_live_sessions` (atomic check-and-insert). These are the quotas
  the acceptance gate tests. The accounting's single source is `SessionManager`/`Session`
  (which know a snapshot's size), not `ServiceLimits` alone
  (`crates/chronofork-service/src/limits.rs:1-30`).
- **Global store-byte quota is explicitly deferred, not promised (K7/V2-2).** The store writer
  lock is per key, so a cross-key global byte check cannot be atomic without a shared counter.
  The ADR therefore states: v1 enforces the per-session byte quota and the snapshot count;
  `max_store_bytes`, if implemented, is **best-effort** (a synchronized counter is the
  follow-up W3.3 task) and **the acceptance gate does not claim it**. If the owner wants a strict
  global quota now, it becomes a hard requirement with an atomic reservation in the store — an
  explicit scope decision, not an implied one.
- **`reap()` trigger, response and handle retention (K9/V2-2/V3-2):**
  - `close` is **not** a retention loophole: closing a session releases its backend and snapshot
    memory **immediately** (the state is dropped; only the `Arc` handle keeps the session object
    alive for readers already in flight), so `create → close` repeated in a loop cannot
    accumulate closed state while the live count stays low. The verification row "`close` frees
    memory" is therefore literal, not aspirational;
  - trigger: `reap` is an **explicit operation** (authorized like `close`), and the daemon also
    runs it opportunistically when admitting a new session and when the live count is at the
    cap. In addition, **closed entries that were not reaped are bounded independently of the
    live count**: an internal retention limit of `max_closed_retained` = 64 entries or
    `closed_retention_ttl` = 300 s (whichever comes first) forces cleanup, so a low live count
    cannot hide an unbounded set of closed sessions;
  - response: the count of reaped sessions and their ids; removing a session means a later
    `inspect` of it reports `not_found` (changed from `closed`), and the generation makes the
    change observable;
  - retention: an `Arc` handle keeps a removed session alive for **in-flight readers** (no
    use-after-free); the entry disappears from the registry at reap time, and the memory of a
    removed session is released when the last in-flight handle drops. The ADR does not promise
    an exact release instant, only that no new operation can resolve the session and that an
    in-flight holder sees a consistent generation;
  - the registry counts only live sessions against `max_live_sessions`; the closed-entry bound
    above is what keeps closed state finite.
- Closed sessions no longer retain full state by default; an explicit archive/export is a
  separate operation.
- Quotas are enforced before allocation; a rejected request never partially applies.

### 4. Minimal composed agent API (roughly nine operation groups, not a CLI mirror)

The exact names/grouping are an implementation detail; the acceptance is the KidEngine flow.
**The surface is a contract, not a list of intentions: every operation declares its input, its
limits (`ServiceLimits` additions), its output shape, its authorization, whether it is
cancelable/deadline-bound, and its expected cost (F1).** The groups:

1. **`run`** — batch execution with `stop_on` (PC, memory value, event, tick, frame) with an
   explicit **grammar and precedence** (F8), bounded `inputs`, `max_wall_ms`, cancel/progress,
   and a result with `performed`/tick/halted/stop-reason/hash when requested. Single-tick
   `step` remains available for compatibility.
2. **`memory`** — read, write, diff and search, bounded and paged; a single region/window per
   call; `diff` semantics (number of ranges) and `search` (one window per call is
   insufficient: declared multi-window or repeated calls) are specified (F1).
3. **`disassemble`** — bounded address range, with the documented instruction set and an
   honest "unknown" encoding. **Seam (V2-5):** the backend contract exposes an **optional
   `disassemble` capability** (returns `None` where the backend cannot decode); the service
   reaches it through `chronofork-sessions` as a pass-through. There is **no concrete
   dependency** on `chronofork-debug` or a backend below the CLI, and no second canonical
   machine is created. Debugger-grade operations (breakpoints/watchpoints) use a
   CLI-injected provider trait object. If a design ever needs the dependency table to change,
   it requires its own ADR — it is not done silently here.
4. **`trace`** — capture with triggers/ring and query with filters; structured records, not
   raw text. Declares ring capacity, max records per query, trace identity and expiry.
5. **`debug`** — breakpoints/watchpoints/eventpoints/logpoints (set/clear/list) with stop
   reasons as a typed enum.
6. **`frame`** — capture with a **decided** format, budget and pagination, plus a
   reconstruction test. The raw 320×224 RGBA frame is 286,720 B (382,296 B base64 = 3.89× the
   previously floated 96 KiB budget); the real ceilings today are 1 MiB
   (`limits.rs:35-36`). The ADR fixes `max_frame_bytes` = 131,072 (128 KiB) with region/tile
   pagination (`max_frame_tiles` = 256) and `frame_hash` always present, and declares the MCP
   envelope amplification (1.17–1.67× measured in A6 §7). **Bounded frame queue (V3-2):**
  - capacity: `max_frame_queue` = **32 frames per session** (default), with the queue holding
    only frames a consumer has not taken; `max_frame_tiles` bounds one frame's size, not the
    queue;
  - retention: a queued frame is retained until taken or until **`frame_ttl` = 60 s** from
    enqueue;
  - drop rule: when the queue is full, the **oldest un-taken frame is dropped** and the drop is
    recorded (a counter per session plus the dropped frame index); no silent skip;
  - stale index: requesting a dropped or expired frame returns a typed
    `frame_not_retained` error carrying the index, never the next frame and never an empty
    success;
  - a frame request names a bounded window (region or tile range) and the maximum outstanding
    frames per consumer is the queue capacity. A slow consumer therefore cannot grow memory
    without bound. Pixel format stays W3.4a; the budget, pagination, capacity, TTL and drop
    rule are contract here.
7. **`session.save` / `session.load`** — persist/export and restore a **live, branched**
   session (not only a committed checkpoint); this closes the KidEngine gap. The envelope is
   decided and versioned in W3.5.
8. **`fork` / `compare`** — fork current/snapshot, compare ticks/hashes/regions.
9. **`capabilities` / `cancel` / `progress`** — discovery and control; `capabilities` is the
   **single discovery source** for operations and limits (F6). There is no second
   `server/discover` surface in v1; the post-M8 `2026-07-28` adapter translates it.

The API must be composable and batch-oriented. An agent should not need one call per
instruction; `run` with stop conditions and structured trace queries are the primary tools.
The nine groups are not a simplicity claim: the measurable acceptance is the frozen KidEngine
script on a redistributable synthetic fixture — **round trips, bytes and wall time measured
with a declared calibration policy, and the round-trip threshold justified after the flow is
frozen** (critique C1 §6).

### 5. Transport and security

- v1 is loopback, single-user (ADR-0014). No TLS, SSE/streaming, keep-alive or chunked bodies
  unless a later ADR adds them.
- **`Origin` allowlist, not "any localhost" (critique C1 §5, R2 correction 7):** the policy is
  an explicit list of trusted origins (or rejection of `Origin`-bearing requests when the
  surface does not need a browser), plus `Host`/loopback checks, returning a typed 403. A
  loopback port is not an authentication boundary.
- **Pre-auth availability is bounded:** an absolute pre-auth connection deadline, a per-peer
  pre-auth connection cap, and either a blocking `accept` or a justified/documented accept
  floor with a measured number. Slowloris pre-auth is a demonstrated availability defect
  (A6-F4); it must be tested, and skip-by-absence is not a pass.
- **The worker pool is part of the contract (F4/K10):** every documented bound (list latency,
  cancel latency, run admission) is expressed in terms of the configured worker count, and
  concurrent long operations are limited per principal with a typed rejection.
- **MCP version:** the wire today is `2025-03-26` (`crates/chronofork-transport/src/mcp.rs:29`);
  the `2026-07-28` migration is post-M8 and must not block the emulator. `rmcp` (or another
  SDK) is at most a dev-dependency used for conformance/interop tests.
- Static tokens/process-local ownership remain v1 limitations and must be documented (plus
  the ADR-0014 file-permission rule).

### 6. Error semantics and safety

- Typed errors for conflict, limit, cancelled, deadline, unknown session, invalid input; the
  `cancelled`/`deadline` codes are versioned on the wire.
- A cancelled or deadline-truncated run returns its `performed`/stop reason rather than losing
  the partial result in a generic error.
- No silent partial apply: a failed restore/snapshot leaves state untouched (existing
  invariant).
- Mutating operations validate before allocating; quotas are checked before work.
- A client that times out has a **bounded reconciliation horizon**, not an unconditional
  promise (V3-3): it can query the operation result **while the journal entry is retained and
  the process epoch matches**; beyond that, `operation_ttl` (measured **from completion**) or a
  restart makes the outcome `unknown_outcome`, and the server does not re-execute. A revision
  precondition (`expected_revision`) only rejects stale mutation; it does not recover a lost
  result.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Mirror ~30 CLI verbs in MCP | Larger surface, more tokens/maintenance, no batching/cancel/quotas; it does not close the KidEngine flow. |
| Keep CLI-only and defer MCP | Contradicts the agent-native thesis; agents cannot attach to a live session. |
| Tokio/async/actors now | Adds a runtime and concurrency complexity to a deterministic synchronous core; per-session locking is sufficient first. |
| Process per session now | Attractive isolation, but heavy; keep as a documented escape hatch, not the v1 default. |
| Global lock + smaller max_steps | Reduces the symptom, not the serialization; long operations still block. |
| Keep `store.save` as the only persistence | Cannot persist a live branched session; does not meet the KidEngine flow. |
| Accept any `localhost:<port>` `Origin` | A local port is not authentication; a browser on the same host can reach it. Use an explicit allowlist or refuse the routes (critique C1 §5). |
| Unbounded operation journal (no TTL/FIFO cap) | Reintroduces the F03-class growth defect in the service (K4/F5). |

## Limits table (contract, F1)

**This table is the contract, not a to-do list.** Every new operation gets a `ServiceLimits`
entry — the catalog's single source of limits
(`crates/chronofork-service/src/limits.rs:1-7`) — with a **default**, a **unit**, whether the
**scope is configurable** at runtime, and the **rejection** the operation returns when the
input exceeds it. Rejection happens BEFORE allocation, exactly as the existing 16 operations
do. The current `ServiceLimits` has eight entries; the additions below grow it.

| Limit | Default | Unit | Configurable scope | Rejection on exceed |
|---|---:|---|---|---|
| `max_steps_per_run` | 100,000 | steps | daemon (≤ existing `max_steps_per_request`) | typed `limit` error, before any step |
| `max_run_wall_ms` | 30,000 | ms (host) | per request + daemon cap | typed `deadline` |
| `max_input_frames` | 4,096 | input frames per `run` | daemon | typed `limit` |
| `max_memory_window` | existing | bytes | daemon | typed `limit` |
| `max_diff_ranges` | 256 | ranges | daemon | typed `limit` |
| `max_search_windows` | 8 | windows per call | daemon | typed `limit` |
| `max_search_pattern` | 64 | bytes | daemon | typed `limit` |
| `max_disasm_instructions` | 1,024 | instructions | daemon | typed `limit` |
| `max_trace_ring` | 65,536 | records | per capture + daemon cap | typed `limit` |
| `max_trace_records_per_query` | 4,096 | records | per query | typed `limit` |
| `trace_ttl` | 900 | s | daemon | expired trace → typed `not_found` |
| `max_breakpoints` / `max_watchpoints` / `max_logpoints` | 64 / 64 / 64 | entries | per session | typed `limit` |
| `max_frame_bytes` | 131,072 | bytes of response body | per request (≤ transport cap) | typed `limit` |
| `max_frame_tiles` | 256 | tiles per response | per request | typed `limit` |
| `max_frame_queue` | 32 | frames per session | daemon | oldest un-taken dropped (recorded); stale index → `frame_not_retained` |
| `frame_ttl` | 60 | s from enqueue | daemon | stale index → `frame_not_retained` |
| `max_live_snapshot_bytes` | 8,388,608 | bytes | per request (≤ store cap) | typed `limit` |
| `max_live_snapshot_age` | 86,400 | s | daemon | typed `expired` |
| `max_operation_journal` | 1,024 | entries | daemon | typed `limit` (never evicts an active entry) |
| `operation_ttl` | 600 | s | daemon | expired entry → unknown outcome (see §2) |
| `max_long_runs_global` | `workers − 1` (3 with the default 4) | admitted long runs | daemon, `N ≥ 2` required | typed `limit` before hand-off |
| `max_long_runs_per_principal` | 1 | admitted long runs | daemon | typed `limit` |
| `max_snapshots_per_session` | 64 | snapshots | per session | typed `limit` before allocation |
| `max_bytes_per_session` | 67,108,864 | bytes | per session | typed `limit` before allocation |
| `max_live_sessions` | existing `max_sessions` | live sessions | daemon | typed `limit`, atomic check-and-insert |
| `max_closed_retained` | 64 | closed entries | daemon | forced cleanup (independent of the live count) |
| `closed_retention_ttl` | 300 | s since close | daemon | forced cleanup |

**Scope of the numbers.** The defaults are conservative v1 values chosen so the contract can be
tested; per-session/store numbers are re-measured after the first real workload and may be
raised or lowered with a documented change. "Choose after measuring" is not an acceptable
state for a limit that gates acceptance.

**Caller-supplied values.** A request may lower a limit within its own scope where the table
marks the scope configurable; it can never raise it beyond the daemon cap. An unknown or
out-of-range value is a typed invalid-input error.

### `service.limits` (discovery)

The effective limits are readable through `capabilities` (§4.9) as `service.limits`, so an
agent can size its calls without guessing. `service.limits` returns the same table with the
effective (post-configuration) values.

## New limits and crate boundary (F1/F2)

**Stop conditions (`stop_on`) — grammar and precedence (F8/V3-2).** `run` accepts at most four
conditions, each `{ kind, value, phase }` (there is **no caller-supplied `tie_break`**: the
order is fixed by this contract):

- `kind` ∈ {`pc` (address), `memory` (address + mask/value), `event` (typed debug event),
  `tick` (absolute boundary), `frame` (frame index)};
- `phase` ∈ {`pre-step`, `post-step`} — `pre-step` tests before the next step and stops
  **without** consuming it; `post-step` tests after the step just executed. Default `post-step`;
- **tie-break (total order):** among conditions that match at the same boundary the order is
  `pc` > `memory` > `event` > `frame` > `tick`; among conditions of the same kind the lowest
  address/index wins (for `memory`, the lowest address); if the kind **and** the address/index
  are the same — including two `memory` conditions on the same address with **different masks
  or values** that both match — the **position in the request order** decides, regardless of
  the values. The result reports exactly one `stop_reason`;
- **same-boundary priority (strict, totally ordered):** `cancelled` > `deadline` > `halted` >
  `stop_condition` > `completed`. When both `cancel` and `deadline` are observed at the same
  boundary, **`cancelled` wins** (the caller's explicit signal beats the host deadline). The
  single interpretation is `stop_reason` ∈ {`cancelled`, `deadline`, `halted`, `stop_condition`,
  `completed`}; when it is `stop_condition`, the matched condition is named in
  `stop_condition_kind`/`value`/`request_index`. There is one reading of `stop_reason` in this
  ADR;
- **required tests (V4-3):** simultaneous cancel + deadline at the same boundary (expect
  `cancelled`), and two `memory` conditions on the same address with different masks that both
  match (expect the earlier request position).
- **Cancellation N (V3-2):** the cancel token is checked **once per instruction**, including
  inside a `run` batch — the backend's batch primitive must check it at every step boundary, so
  a cancelled run is a deterministic prefix whose length depends only on when the signal
  arrived, not on batch size. `performed` is the authority; the acknowledgment does not promise
  a step count.

**Inputs (F1).** `run` accepts `inputs` as a bounded sequence of input frames
(`max_input_frames`), one per emulated step, of the existing canonical `InputFrame` encoding —
no new wire type. A sequence shorter than the run's step budget is padded with the neutral
input; a longer sequence is a typed `limit` error.

**Crate boundary (F2, V2-5).** `chronofork-service` depends only on core + backend-api +
sessions today, and the backend-api trait has no disassembly seam. The contract is therefore a
**generic optional capability**, not a concrete dependency: the backend contract exposes an
optional `disassemble` capability (documented instruction set, honest "unknown" encoding, and
`None` when the backend cannot), and `chronofork-sessions` passes the capability through to the
service. Debugger-grade operations (breakpoints/watchpoints) are reached through a
CLI-injected provider trait object. Neither `sessions` nor `service` gains a dependency on
`chronofork-debug` or a concrete backend; the service never learns hardware details (AGENTS.md
rule 1). If a future design needs the dependency table itself to change, that is a separate ADR
— not done silently here.

## Consequences

### Positive

- Independent sessions can execute in parallel; a `list` no longer blocks on a running session
  beyond the documented bound (metadata is per-session).
- Agents get a composable, batched surface; token/call cost drops by orders of magnitude.
- Live-session persistence closes the KidEngine gap.
- Quotas and `reap` bound memory and make `max_sessions` meaningful.
- The concurrency invariants (lock order, atomic admission, locked-free cancel) are contract,
  so the design is implementable without inventing safety later.

### Negative / risks

- This is a service-boundary change: catalog, auth, ownership, leases, tests and the GUI
  channel must be updated, and the service JSON version is bumped.
- Per-session concurrency introduces new interleavings; the minimum test set is in
  §Verification 9, plus the race catalogue below.
- Reconciliation needs a bounded journal; it adds state, cleanup and a published
  deduplication horizon.
- Cancellation must be deterministic and not change canonical state; a cancel at a different
  boundary changes how many steps were applied, so the result must report `performed`.
- Frame format/pagination and live-session envelope are compatibility decisions taken in
  W3.4a/W3.5, not deferred silently.
- The client read timeout (5 s, `crates/chronofork-transport/src/client.rs:19-22`) and the
  default worker pool (4) become the visible bottleneck if the ADR's bounds are aspirational;
  K10 makes them part of the contract.

### Race catalogue (K5–K10, non-optional)

| # | Race | Mitigation required |
|---|---|---|
| R1 | `cancel` behind the run | lock-free token/flag (K2) |
| R2 | deadlock owner→registry | fixed lock order; release owner before registry (K5) |
| R3 | `max_sessions` overshoot | atomic check-and-insert (K6) |
| R4 | global store byte overshoot | synchronized counter or declared best-effort (K7) |
| R5 | use-after-free on `reap` | `Arc` handle keeps the session alive (K9) |
| R6 | stale read ratified as success | monotonic revision + pre-known request key (K3) |
| R7 | worker-pool starvation by long runs | admission limit per principal + documented bound in `workers` (K10/F4) |

### Compatibility and versioning

- Existing 16 operations remain valid unless a later ADR deprecates them; new operations are
  additive.
- **Wire version and negotiation (F10/V2-5/V3-5/V4-1):** the API wire is **v1 today**; the target
  of this ADR is **v2**. Selection and refusal are explicit, and the **refusal of a v2 call on a
  legacy v1 daemon is the client's job**, not the daemon's:
  - the client sends its version range on **every request** (`X-ChronoFork-API: 1` / `2` or the
    equivalent field in the MCP `initialize` + per-call metadata — HTTP is stateless here, so a
    single "session start" flag is not enough);
  - a request **without a version field is treated as v1** (old clients keep working);
  - **new client against a server:** before sending any v2 **mutation**, the client performs a
    **read-only probe** (`capabilities`/version discovery) and, if the server does not advertise
    v2, it fails locally with a typed `unsupported_version` and does **not** send the v2
    mutation. Detection never issues a mutating request, and a sentinel mutation must remain
    uncalled during detection (test: a legacy daemon that ignores the header and would dispatch
    `/ops` must not be reached by the sentinel);
  - **new server:** rejects incompatible versions **before dispatch** with the same typed error
    and no partial execution;
  - **legacy server:** the daemon installed before v2 is **not required to reject** anything —
    it ignores the header and dispatches as v1. A new ADR cannot retrofit behavior into a
    deployed daemon, so the contract places the guarantee in the client; no retroactive server
    behavior is promised;
  - this is the **API wire version**, distinct from the MCP protocol version (`2025-03-26`
    today), whose `2026-07-28` migration stays deferred.
- **Changed shapes in v2:** `run` result (adds `performed`, `stop_reason`, request/revision keys
  and the process epoch), the `cancelled`/`deadline` error codes, the service JSON's
  `state_hash` presence, and the `close`→`reap` semantics.
- **Client/decoder matrix (declared, with per-version parity):** old client + v2 server → serves
  the v1 shape for v1 calls; **new client + v1/legacy server → the client refuses v2 calls
  locally after the read-only probe** and uses the v1 shape for v1 calls (the server is not
  credited with a refusal it does not implement); new client + v2 server → v2; the GUI and the
  typed client are updated in the same change. Parity tests run **per version** (v1↔v1, v2↔v2)
  and for the cross cases that are declared supported; the matrix is part of the migration
  checklist.
- `store` operations remain; `session.save/load` is additive for live sessions.
- Ownership remains process-local in v1; durable principals are a future ADR.

## Verification

1. **Parallelism and overlap (V4-2):** the scale gate runs with **`workers = 5`**, four
   principals and `max_long_runs_global = 4`, so four long runs can be admitted and still leave
   one worker reserved for control. Four independent sessions must overlap by independent
   barriers (not only wall-clock speedup), and on a calibrated runner scale >2× versus 1.
   `list` of another session returns within the documented bound expressed in `workers`. The
   overlap test is not relaxed to make the scale gate pass.
2. **Cancellation:** a 100k-step run can be cancelled **from another worker** at a boundary;
   `cancel` latency is bounded in milliseconds; the result reports `performed`; the state is a
   deterministic prefix; no host time leaks into canonical state.
3. **Reconciliation, restart and expiry (V2-3):** a client timeout is reconciled via
   `expected_revision` and a request key known before the mutation; written tests cover timeout,
   **two writes at the same tick**, **run → restore → retry (ABA)**, **same key with different
   arguments**, **full journal**, **restart mid-operation** (result reported unknown, **no
   automatic re-execution**) and **key expiry** (unknown outcome beyond the horizon); retry does
   not double-apply; a cancelled/deadline run preserves `performed`.
4. **Quotas/reap (V2-2, honest scope):** the enforced quotas are
   `max_snapshots_per_session`, `max_bytes_per_session` and `max_live_sessions` with atomic
   admission; those reject before allocation and have a concurrent test. **The global store-byte
   quota is declared deferred and is not part of this gate.** `reap` reports the reaped count,
   makes later `inspect` report `not_found`, and an in-flight reader holding an `Arc` handle does
   not fault.
5. **Admission/cancellation (V2-2/V3-1/V4-2):** with the worker pool at its default (4) and
   `max_long_runs_global = 3`, the saturation test uses **multiple principals** with **four long
   run attempts** + `list` + `cancel`: **three are admitted, the fourth attempt is rejected**,
   and control stays executable. The acceptance target is that `cancel` and `list` complete
   within **250 ms p95** on a calibrated runner and the rejected attempt returns within the same
   target — **measured by this test**, not asserted as an existing result. `workers = 1` refuses
   to start.
6. **Agent flow:** the frozen KidEngine script runs on a redistributable synthetic fixture —
   configure a condition, run, query a bounded trace, locate and mutate a variable,
   fork/compare, save/restore, produce a reproducible report — with measured round trips,
   bytes and wall time (threshold justified after freezing; `10k steps < 2 s` is one row, not
   the acceptance).
7. **Live session:** save/load a branched live session round-trips tick/hash/state.
8. **Transport:** slowloris cannot exhaust the pool; `/health` succeeds; the `Origin` allowlist
   rejects an untrusted origin; 4 long runs + 1 `list` + 1 `cancel` saturation test documented;
   MCP interop with an official client in dev.
9. **Non-regression and version matrix (V4-1):** existing 16 operations and canonical parity
   tests still pass; the `capabilities` surface no longer advertises `memory_write` without the
   write operation; the v1/v2 parity matrix passes per version; and a **legacy-daemon test**
   proves the new client refuses v2 locally after a read-only probe — a sentinel mutation is
   never dispatched against a daemon that ignores the API header.
10. **Minimum test set (R5):** cancel == run of `k`; cancel in milliseconds; timeout reconcile;
    restart/expiry unknown-outcome; concurrent quota; reap/readers; close frees (repeated
    create→close does not accumulate); frame queue drop/expiry; deterministic isolation;
    relative G-PERF (shared with ADR-0015).

## Migration sketch

1. Introduce the registry/per-session owner behind the existing service API, with the lock
   order and atomic admission; keep the wire contract stable while tests migrate.
2. Land cheap `status()`/metadata (W2.4) or an independent cache, then make `list` honest.
3. Add lock-free cancel/deadline/progress/revision keys with versioned error codes; measure.
4. Add quotas/reap; update limits/docs.
5. Add the composed agent operations with the limits table; keep the old 16 for compatibility.
6. Add frame (W3.4a) and live-session save/load (W3.5); version the wire contract.
7. MCP protocol migration after M8; `Origin`/timeout now.

## Open questions

1. Per-session `Mutex` vs command queue/actor: start with `Mutex` and measure; actor only if a
   session needs concurrent readers/writers.
2. ~~Exact frame format?~~ **Moved to task W3.4a** with its own acceptance (format, budget,
   pagination, reconstruction test); the ADR fixes only the budget ceiling and the
   `frame_hash`-always rule.
3. Live-session serialization: reuse the snapshot payload + service metadata, or a new
   envelope? Recommendation: reuse the versioned snapshot payload and add live metadata
   (decided in W3.5).
4. ~~Quota defaults?~~ **Decided in the limits table** (contract): the defaults there are the
   v1 values and are re-measured after the first workload; a change is a documented limit
   change, not an open question.
5. Should `session.save` be a service operation or a transport-only convenience?
   Recommendation: service operation, so CLI/HTTP/MCP share semantics.
