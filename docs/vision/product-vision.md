# Product Vision

## The problem

Emulators are usually built as interactive applications. Debugging, automation and
experimentation are added around a machine loop that was never designed to be
observed, cloned or replayed. Reverse-engineering workflows therefore become a
mixture of fragile scripts, GUI-only inspection and manual notes. An AI agent can
drive an emulator only through whatever narrow surface the frontend exposes.

ChronoFork starts from the opposite direction: the emulated machine is a
deterministic state machine behind a typed contract, and every interaction is an
operation on a session that can be checkpointed, forked, inspected and replayed.

## Vision

ChronoFork is a cross-platform platform for **deterministic, forkable emulation and
reverse engineering**, usable by humans, scripts and AI agents through the same
application services.

A user should be able to:

1. Start a session with an explicitly identified ROM and hardware configuration.
2. Advance virtual time and inject input without host-clock nondeterminism.
3. Inspect CPU, memory, devices and traces through side-effect-free interfaces.
4. Create a named checkpoint.
5. Fork that checkpoint into independent branches.
6. Run different inputs or memory mutations in each branch.
7. Compare resulting state and traces.
8. Restore a checkpoint and reproduce a branch exactly.
9. Share structured artifacts with humans, other agents and external tools.

## Principles

1. **Determinism before performance.** A fast wrong answer is worse than a
   reproducible one. Host time must not silently change emulated behavior.
2. **Isolation before sharing.** Mutable hardware state belongs to exactly one
   session. Read-only configuration may be shared only when demonstrably safe.
3. **Capabilities are guarantees.** A backend must report what it actually
   implements; the host must not expose unsupported features as if they worked.
4. **Evidence before claims.** Snapshot, replay and determinism claims require
   executable tests. Research claims require source references.
5. **Generic core, specific hardware.** Session, storage, transport and experiment
   layers must not encode Mega Drive assumptions.
6. **Harness independence.** ChronoFork must remain understandable and usable by
   humans and any coding agent, not only one vendor's tooling.
7. **Transparent provenance.** Reuse is allowed only with verified licenses and
   explicit attribution; independent implementation is preferred when obligations
   would compromise the project.

## Target users

- Reverse engineers studying Mega Drive / Genesis software.
- TAS authors who need branching, rerecording and verifiable replay.
- Emulator developers validating cores and save states.
- Automation engineers and AI agents running controlled experiments.
- Researchers who need structured traces and reproducible artifacts.

## What makes it different

ChronoFork is not merely a graphical emulator. Its defining capabilities are:

- Reproducible emulation with explicit virtual time.
- Independent, clonable execution sessions.
- Branching execution histories with lineage.
- Deep hardware introspection separated from emulated bus access.
- Programmatic experimentation through stable services.
- Deterministic playback and divergence detection.
- Human/agent collaboration over the same session model.
- Structured, exportable traces consumable by external tools.

## Current horizon

The [backend strategy](../architecture/backend-strategy.md) is decided: [ADR-0002](../adr/0002-backend-boundaries.md)
accepted the staged hybrid on 2026-11-13, and the Sega Mega Drive / Genesis is the
first real system. Milestones M0–M5 are complete and merged to `main` (M4 at
`4c595a2`, verified on `agent/m4-debugging-depth` at `6d30243`; M5 at `0c4e9f0`,
PR #4, verified on `agent/m5-tracing-scripting` at base `84f1476`, final T12 PASS
2026-11-16), and M6 (agent-native interfaces) is next.
The machine model itself is still narrow, and that gap is tracked in the parallel
hardware-depth tracks H1–H5 rather than in the capability stages. Multi-system
support, the GUI, HTTP/MCP transports and hardware trace sources are roadmap items, not
current features; the host trace surface and the sandboxed deterministic script DSL are
delivered for the documented minimal-slice scope. See the [milestone status
table](../roadmap/milestones.md) for the canonical status and integration state.
