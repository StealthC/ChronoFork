# Architecture Overview

## Goals

ChronoFork separates four concerns that are usually entangled in emulator
frontends:

1. **Generic platform services** — identity, sessions, checkpoints, lineage,
   comparison, persistence and future transports.
2. **Backend contracts** — what an emulation implementation must provide and what
   guarantees it reports.
3. **Hardware implementation** — CPU, video, audio, buses and devices for a
   specific system.
4. **Presentation and automation** — CLI, HTTP, MCP, GUI, scripts and agents.

Only layer 3 is system-specific. Layers 1 and 2 must remain reusable by future
backends; layer 4 must not own canonical emulated state.

## Current layer diagram

```text
┌────────────────────────────────────────────────────────────────────┐
│ presentation / automation (never owns canonical state)             │
│ chronofork-cli  ·  chronofork-gui  ·  HTTP/MCP clients             │
├────────────────────────────────────────────────────────────────────┤
│ chronofork-transport                                               │
│ bounded HTTP/1.1 server · MCP JSON-RPC over HTTP · typed client    │
├────────────────────────────────────────────────────────────────────┤
│ chronofork-service                                                 │
│ operation catalog · canonical JSON · auth/ownership · leases       │
├────────────────────────────────────────────────────────────────────┤
│ chronofork-sessions  (lifecycle, snapshots, forks, durable store)  │
│ chronofork-debug     (host debugger over Backend)                  │
│ chronofork-trace     (host capture/query/compare, CFTRACE1)        │
│ chronofork-script    (sandboxed deterministic DSL)                 │
├────────────────────────────────────────────────────────────────────┤
│ chronofork-backend-api        (capabilities, contracts)            │
├───────────────────────────────┬────────────────────────────────────┤
│ chronofork-core               │ backends                           │
│ (IDs, ticks, input, hashes)   │                                    │
└───────────────────────────────┴────────────────────────────────────┘
        ▲
        │ implement backend-api
┌───────┴───────────────────────┬────────────────────────────────────┐
│ chronofork-test-backend       │ chronofork-megadrive               │
│ (synthetic validation)        │ (minimal 68000 + ROM/RAM)          │
└───────────────────────────────┴────────────────────────────────────┘
```

`chronofork-backend-api` depends only on `chronofork-core`.
`chronofork-sessions` depends on both, but never on a concrete backend.
Each backend depends on the contract, not on sessions.
`chronofork-debug` depends only on core and backend-api and borrows a backend for host
breakpoints, watchpoints, event breakpoints and logpoints; it owns no session, storage,
transport or UI state. `chronofork-cli` composes sessions, the debugger, the service and a
selected backend.

`chronofork-service` (M6) owns the application operations, validation, authentication,
ownership and the concurrency boundary. It depends on core, backend-api and sessions only,
injects concrete backends as `Arc<dyn BackendFactory>`, and contains no socket, HTTP header,
JSON-RPC or CLI type. `chronofork-transport` (M6) depends on `chronofork-service` and holds
the only HTTP/MCP code: a bounded blocking HTTP/1.1 server and MCP endpoint over
`std::net`/`std::thread`, plus the typed client the CLI and tests use. Both are thin: no
transport type may flow back into core, backend, sessions or service, and the session layer
stays the single owner of canonical emulated state
([ADR-0012](../adr/0012-agent-native-interfaces.md),
[agent interfaces](agent-interfaces.md)).

`chronofork-gui` (M7) is the presentation layer: strict decoders of the canonical service
views, a window/pane/mosaic model, eight panels, deterministic plain/ANSI and self-contained
HTML renderers, a `HostClock`-driven pacer/run loop, capability-gated video/audio pipelines
and a cooperative human/agent controller lease. It depends only on core and service and
never on transport, sessions or a backend, so it can reach canonical state only through the
catalog; the CLI composes the local or remote channel and is the only crate that touches
transport ([ADR-0013](../adr/0013-human-facing-gui.md), [GUI
architecture](gui.md), [M7 acceptance](../quality/m7-gui-acceptance.md)).

## Data flow

### Create

```text
CLI/service → SessionManager::create_session
            → BackendFactory::create
            → Backend::snapshot (initial state)
            → Session registered with root lineage
```

The initial backend snapshot is retained so `reset` can return to the creation
state without recreating the backend.

### Step

```text
SessionManager::step(id, InputFrame)
  → Session::step
  → Backend::step
  → StepOutcome { tick, state_hash, halted }
```

There is no background execution thread in the current implementation. The caller
owns the cadence, which makes tests and agent automation explicit.

### Snapshot

```text
SessionManager::snapshot(id)
  → Backend::snapshot (opaque payload + compatibility metadata)
  → Snapshot envelope (ID, origin session, format version, checksum)
  → stored in the session
```

Serialization is explicit and versioned. The host never interprets backend state.

### Fork

```text
SessionManager::fork_from_snapshot(source, snapshot)
  → clone snapshot metadata
  → BackendFactory::create (fresh backend)
  → Backend::restore(snapshot)
  → new Session with child lineage
  → child adopts the origin snapshot
```

A fork is a new mutable machine. It is not an OS process fork and does not share
mutable state with its source.

## Dependency rules

- `chronofork-core` must not depend on any other workspace crate.
- `chronofork-backend-api` may depend only on `chronofork-core`.
- `chronofork-sessions` may depend on core and backend-api, never on a concrete
  backend.
- A backend crate may depend on core and backend-api, never on sessions.
- `chronofork-service` may depend on core, backend-api and sessions, never on a concrete
  backend and never on `chronofork-transport`.
- `chronofork-transport` may depend on `chronofork-service` (and through it the lower
  layers); nothing below it may depend on `chronofork-transport`.
- `chronofork-gui` may depend only on `chronofork-core` and `chronofork-service`; it must not
  depend on transport, sessions, backend-api or a backend, and nothing depends on it except
  the CLI.
- CLI/HTTP/MCP adapters may depend on sessions and a concrete backend selected by
  composition, but the session model must not depend on them; the GUI reaches canonical
  state only through the service catalog.
- Transport types (HTTP, MCP JSON, sockets, JSON-RPC) must not appear in core,
  backend-api, sessions or service. `chronofork-service` has its own canonical JSON value
  type and knows nothing about the wire framing.
- Dependency direction is therefore `core ← backend-api ← sessions ← service ← transport`,
  with the CLI composing every layer. The M6 audit (M6-A26) checks it by grep, `cargo tree`
  and a lockfile diff.

## Cross-cutting decisions

- **Determinism:** no host wall-clock input to hardware; explicit virtual ticks;
  typed input injection; canonical state hashes.
- **Isolation:** one mutable backend per session; forks restore into fresh
  instances; memory writes are session-local.
- **Versioning:** outer snapshot envelope and backend payload have independent
  format versions; incompatible versions are rejected.
- **Errors:** typed errors at each boundary; backend errors are not silently
  converted into generic strings.
- **Concurrency:** mutating session APIs take `&mut self`; no hidden global lock. Transport
  layers must add an explicit synchronization policy — M6 does it once, in
  `chronofork-service` (see [open questions](#open-questions)).
- **Unsafe:** forbidden by workspace lint. If a future backend needs it, it must
  live in a dedicated crate with documented invariants and focused tests.

## Implemented versus planned

| Area | Current status |
|---|---|
| Session lifecycle | Implemented |
| Snapshot/restore/import | Implemented |
| Fork + lineage | Implemented |
| Deterministic synthetic backend | Implemented |
| Minimal 68000 + ROM/RAM + VDP port + interrupt-scheduler Mega Drive slice | Implemented (partial hardware) |
| VDP status/control/VRAM port window | Implemented (no rendering/DMA) |
| Deterministic VBlank/HBlank interrupt scheduler | Implemented (instruction-quantum, not cycle-accurate) |
| TH-high 3-button controller-read subset (ports 1–2) | Implemented for the minimal slice — see [Mega Drive slice](megadrive-slice.md) |
| VDP rendering/DMA, audio, Z80, controller protocol beyond TH-high 3-button, mapper/SRAM | Not implemented |
| `RTE`, SR-mask interrupt gating and VDP VINT-pending status | Implemented for the modelled slice (privilege enforced for the implemented system instructions; HBlank level 4 / VBlank level 6 sources only, level 7 a predicate rule only; no interrupt-timing equivalence; VDP interrupt-enable/ack logic absent) |
| HTTP/MCP/GUI | **Implemented** for the documented scope: HTTP and MCP (M6) — see [agent interfaces](agent-interfaces.md), [ADR-0012](../adr/0012-agent-native-interfaces.md) and [M6 acceptance](../quality/m6-agent-interfaces-acceptance.md); GUI presentation (M7) — see [GUI architecture](gui.md), [ADR-0013](../adr/0013-human-facing-gui.md) and [M7 acceptance](../quality/m7-gui-acceptance.md). The GUI is text/HTML only (no native toolkit, picture or sound); M6/M7 are verified and merged to `main` at `cb05836` (PR #6) |
| Transport-agnostic service + bounded HTTP/1.1 and MCP adapters | Implemented (M6): 16-operation catalog, canonical JSON, token/scope/ownership, manager lock + writer leases, daemon and typed client; HTTP/1.1 loopback/sidecar scope with **no TLS, SSE, keep-alive or chunked bodies** |
| Debug/trace/script/TAS operations over transport | Not implemented (M6-FU1): those surfaces stay CLI-local |
| Durable per-principal ownership, restart-persistent live sessions, TLS, token rotation/expiry, audit log, multi-node, MCP resources/prompts/logging/`Mcp-Session-Id` | Not implemented (M6-FU2–M6-FU4) |
| Debugger triggers/views (M4) and structured tracing (M5) | Implemented for the minimal slice as host layers; backend `breakpoints`/`tracing` capabilities remain `false` — see [M4 debugging](../adr/0010-debugging-depth.md), [M5 architecture](tracing-and-scripting.md) and [M5 acceptance](../quality/m5-tracing-and-scripting-acceptance.md) |
| TAS movie format | Implemented (v1/v2; M3) — see [M3 TAS/replay acceptance](../quality/m3-tas-replay-acceptance.md) |
| Process-isolated backend | Proposed, not implemented |

## Open questions

- Which real backend strategy will be approved? See
  [backend strategy](backend-strategy.md) and
  [ADR-0002](../adr/0002-backend-boundaries.md).
- Should session execution eventually be driven by a scheduler thread, or remain
  caller-driven with explicit `step`/`run` services?
- ~~What is the transport synchronization model for multiple agents sharing one session?~~
  **Resolved by [ADR-0012](../adr/0012-agent-native-interfaces.md) §1 and §5.** The session
  layer keeps its documented shape (synchronous, `&mut self`, no hidden lock) and
  `chronofork-service` is the single place that adds a synchronization boundary, in two
  explicit layers: one `Mutex<SessionManager>` serializes all access and is never held
  across socket I/O, and a per-session writer lease makes a second concurrent mutation of
  the same session fail immediately with a typed, retryable `conflict` instead of blocking
  or interleaving. Reads take no writer lease, so listing, inspection, comparison and memory
  reads stay available during a mutation; they are still serialized by the manager lock and
  observe a consistent state. Cross-process exclusion for durable saves rests on the M2
  storage writer lock, which also maps contention to a retryable `conflict`. The daemon
  therefore documents **one writer per session at a time**, not shared mutation. This is a
  process-local policy: ownership and live sessions are lost on restart, and multi-node
  deployment is **M6-FU2**/**M6-FU3**. See the [agent interfaces](agent-interfaces.md) §7
  contract and the executed rows M6-A8, M6-A29 and M6-A30 in the [M6 acceptance
  plan](../quality/m6-agent-interfaces-acceptance.md).
