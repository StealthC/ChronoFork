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
┌──────────────────────────────────────────────────────────────┐
│ chronofork-cli                 (presentation / demo)         │
│ chronofork-debug               (host debugger over Backend)  │
├──────────────────────────────────────────────────────────────┤
│ chronofork-sessions            (lifecycle, snapshots, forks) │
├──────────────────────────────────────────────────────────────┤
│ chronofork-backend-api         (capabilities, contracts)     │
├───────────────────────────────┬──────────────────────────────┤
│ chronofork-core               │ backends                     │
│ (IDs, ticks, input, hashes)   │                              │
└───────────────────────────────┴──────────────────────────────┘
        ▲
        │ implement backend-api
┌───────┴───────────────────────┬──────────────────────────────┐
│ chronofork-test-backend       │ chronofork-megadrive         │
│ (synthetic validation)        │ (minimal 68000 + ROM/RAM)    │
└───────────────────────────────┴──────────────────────────────┘
```

`chronofork-backend-api` depends only on `chronofork-core`.
`chronofork-sessions` depends on both, but never on a concrete backend.
Each backend depends on the contract, not on sessions.
`chronofork-debug` depends only on core and backend-api and borrows a backend for host
breakpoints, watchpoints, event breakpoints and logpoints; it owns no session, storage,
transport or UI state. `chronofork-cli` composes sessions, the debugger and a selected
backend.

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
- CLI/HTTP/MCP/GUI adapters may depend on sessions and a concrete backend selected
  by composition, but the session model must not depend on them.
- Transport types (HTTP, MCP JSON) must not appear in core, backend-api or
  sessions.

## Cross-cutting decisions

- **Determinism:** no host wall-clock input to hardware; explicit virtual ticks;
  typed input injection; canonical state hashes.
- **Isolation:** one mutable backend per session; forks restore into fresh
  instances; memory writes are session-local.
- **Versioning:** outer snapshot envelope and backend payload have independent
  format versions; incompatible versions are rejected.
- **Errors:** typed errors at each boundary; backend errors are not silently
  converted into generic strings.
- **Concurrency:** mutating session APIs take `&mut self`; no hidden global lock.
  Transport layers must add an explicit synchronization policy.
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
| VDP rendering/DMA, audio, Z80, controllers, mapper/SRAM | Not implemented |
| `RTE`, SR-mask interrupt gating and VDP VINT-pending status | Implemented for the modelled slice (privilege not enforced; HBlank level 4 / VBlank level 6 sources only, level 7 a predicate rule only; no interrupt-timing equivalence; VDP interrupt-enable/ack logic absent) |
| HTTP/MCP/GUI | Not implemented |
| Breakpoints/watchpoints/tracing | Contract sketches only |
| TAS movie format | Not implemented |
| Process-isolated backend | Proposed, not implemented |

## Open questions

- Which real backend strategy will be approved? See
  [backend strategy](backend-strategy.md) and
  [ADR-0002](../adr/0002-backend-boundaries.md).
- Should session execution eventually be driven by a scheduler thread, or remain
  caller-driven with explicit `step`/`run` services?
- What is the transport synchronization model for multiple agents sharing one
  session? The current answer is "one mutable owner at a time; no sharing without
  an explicit lock."
