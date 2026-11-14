# ADR-0002: Backend Strategy and Boundaries

- **Status:** Proposed
- **Date:** 2026-11-13
- **Decision owner:** Project owner (approval required)
- **Supersedes:** none

## Context

ChronoFork's defining capability is forkable, deterministic execution. A real
Mega Drive / Genesis backend must eventually support complete snapshots, multiple
independent instances, headless operation and deep inspection. Three reference
projects were cloned and inspected:

- Exodus (`08f388f77040af28d16d44fdfbddb73252953161`, MIT).
- exodus-for-ais `linux-headless` (`ec0056151f3ff1c334d09ee375abb30a68fdfec5`,
  MIT).
- BizHawk (`d148601a4629c2c230e41647d7c94eee794011c7`, heterogeneous licensing).

## Problem

Which backend strategy should ChronoFork adopt for the first real system, and
where must the boundary sit so the decision can be reversed without rewriting the
platform?

## Alternatives

A. Rust-native Mega Drive backend.
B. C++ adapter around Exodus / exodus-headless.
C. Process-isolated worker per session.
D. External emulation service.
E. Staged hybrid: backend-neutral host + external Exodus reference harness for
   differential traces + a Rust-native production backend.

Full comparison: [backend strategy](../architecture/backend-strategy.md).

## Decision (proposed)

Adopt **Option E**:

1. Keep the backend-neutral contracts and synthetic backend already implemented.
2. Use exodus-headless only as an external differential/reference harness for real
   ROM traces; do not make its single-system MCP server the canonical session
   backend.
3. Build the Rust-native Mega Drive backend incrementally.
4. Revisit Options B/C only after proving reproducible builds, multi-instance
   support and versioned state export.

**This decision is not accepted.** No real backend work may begin under it until
the project owner approves.

## Rationale

- The headless MCP server explicitly holds one emulated system at a time, and its
  MCP surface has no save/load-state tool. That conflicts with forkable sessions.
- Exodus device classes do implement `SaveState`/`LoadState`, so the underlying
  core is a useful oracle, but exposing it requires C++ work.
- BizHawk's own LICENSE warns that the repository as-is mixes incompatible GPL
  licenses and includes components with unclear provenance. It is not a safe
  production reuse source.
- A Rust-native production backend keeps snapshot, determinism and multi-instance
  semantics under ChronoFork's control.
- A reference harness gives early real-hardware evidence without locking the
  product into a wrapper.

## Consequences

Positive:

- The host remains testable without hardware.
- The architecture can absorb a different backend later.
- License exposure is minimized.

Negative / risks:

- Rust-native emulation is a large, long-running effort.
- The reference harness still needs a C++ build (Ninja/clang) and a legal ROM for
  real tests; neither is available in the current environment.
- If the Rust-native path stalls, a C++ adapter may become attractive and could
  reintroduce ABI/build complexity.
- The recommendation remains unproven until the headless port is built and its
  multi-instance/state behavior is measured.

## Evidence

- `.research/upstream/exodus-headless/AGENTS.md`: "The server holds one emulated
  system at a time."
- `mcp/Tools*.cpp` exposes session/run/cpu/device/breakpoint/watchpoint/trace/
  disasm tools; no save/load-state tool.
- `Devices/YM2612/YM2612.h`, `Devices/MD1600IO/*` and
  `ExodusSDK/DeviceInterface/_Documentation/IDevice/Methods.SaveState.xml` show
  device-level state serialization in the underlying core.
- [Emulator architecture research](../research/emulator-architecture.md) documents
  no stable C ABI, a multi-threaded optimistic scheduler whose rollback is not
  user-facing time travel, XML/ZIP save states without an explicit version, and
  unproven multi-instance support.
- `tests/equivalence/README.md` documents a cross-platform trace-equivalence gate
  (upstream claim; not reproduced here).
- BizHawk `LICENSE` explicitly warns of incompatible GPL combinations and unclear
  third-party licenses.
- Exact revisions: [provenance](../licensing/provenance.md).
