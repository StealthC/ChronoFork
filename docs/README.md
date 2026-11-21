# ChronoFork Documentation

This directory is the durable, human- and agent-readable knowledge base for
ChronoFork. Operational working memory lives in the ignored `.planning/` directory;
it is never a substitute for these documents. Durable **lifecycle** state — the
current milestone, its phases, the session handoff and the project conventions — is
committed in [`planning/`](planning/ROADMAP.md), and design specs/plans live under
[`superpowers/`](superpowers/specs/2026-11-17-roadmap-design.md).

## Status

**Repository-level summary.** Status, integration state, evidence and follow-ups live
once in [Milestones](roadmap/milestones.md); phase execution in the
[planning lifecycle](planning/ROADMAP.md); machine-level Implemented / Not implemented
lists on the [Mega Drive slice](architecture/megadrive-slice.md#implemented) page. Link
those pages; do not copy their facts here.

- **M0–M7:** complete and merged to `main`.
- **M8 (functional Mega Drive):** in progress — phases 8.1–8.3 complete and merged
  (cartridge/TMSS, complete 68000/cycle model, complete VDP with rendering and DMA);
  phases 8.4–8.8 pending.
- **M9:** planned after M8.
- **Known gaps:** audio, Z80, timers, the full controller protocol, commercial
  compatibility and native-window/capture surfaces; import the lists from the two
  canonical pages rather than restating them.

Dated acceptance plans and evidence records are **historical snapshots**: they record what
was verified on their date and are not current status claims.

## Start here

| Document | Purpose |
|---|---|
| [Product vision](vision/product-vision.md) | Why ChronoFork exists and what it must become |
| [Scope](vision/scope.md) | Delivered scope, next capability scope and the hardware-depth tracks |
| [Non-goals](vision/non-goals.md) | Explicit boundaries and anti-goals |
| [Architecture overview](architecture/overview.md) | Layers, crates and dependency rules |
| [Backend model](architecture/backend-model.md) | Capability contract and snapshot compatibility |
| [Session model](architecture/session-model.md) | Identity, lifecycle, fork and isolation |
| [Execution model](architecture/execution-model.md) | Virtual time, determinism and replay |
| [Backend strategy](architecture/backend-strategy.md) | Accepted real-backend strategy |
| [Mega Drive 68000 slice](architecture/megadrive-slice.md) | Implemented 68000 + VDP port + interrupt-scheduler slice and debug trace |
| [Tracing and scripting architecture](architecture/tracing-and-scripting.md) | Host trace and session capture layers, canonical trace/query/compare, script runtime/limits/sandbox and CLI ownership |
| [Agent interfaces architecture](architecture/agent-interfaces.md) | Service/transport layers, canonical JSON profile, the 16-operation catalog, HTTP/MCP wire contract, authentication/ownership, concurrency, limits and the M6 non-goals |
| [GUI architecture](architecture/gui.md) | Presentation-only crate and command contract, view/model/render pipeline, attach/detach and run-loop sequences, panels/layouts, capability-gated video/audio, handoff state machine and limits |
| [Requirements](requirements/functional.md) | Functional requirements and traceability |
| [Non-functional requirements](requirements/non-functional.md) | Determinism, quality, portability |
| [Roadmap](roadmap/roadmap.md) | Long-term capability stages 0–8 |
| [Milestones](roadmap/milestones.md) | Canonical milestone status and integration state (M0–M9) plus the hardware-depth checkpoints H1–H5 |
| [Planning lifecycle](planning/ROADMAP.md) | Committed execution state: current milestone and phases, session handoff (`STATE.md`) and project conventions (`CONVENTIONS.md`) |
| [Roadmap design](superpowers/specs/2026-11-17-roadmap-design.md) | Approved whole-project design that established the lifecycle layer |
| [M8 milestone design](superpowers/specs/2026-11-17-milestone-8-megadrive-functional-design.md) | Functional Mega Drive milestone: full 68000/Z80/VDP/audio, execution modes and capture |
| [Codebase map](superpowers/specs/2026-11-17-codebase-map.md) | Evidence-checked map of the crate graph, entry points, tests, hotspots and change traps |
| [EPIC-0001](epics/EPIC-0001-foundation.md) | Foundation epic (complete) |
| [EPIC-0002](epics/EPIC-0002-megadrive-slice.md) | Mega Drive 68000 slice epic, its delivered increments and the hardware-depth follow-ups |
| [Testing strategy](quality/testing-strategy.md) | Test categories and fixtures |
| [ROM fixtures](quality/rom-fixtures.md) | Local-only commercial ROM fixtures: names, regions, hashes and acceptance scenarios (bytes never committed) |
| [Quality gates](quality/quality-gates.md) | Commands that must pass |
| [M2 durable-sessions acceptance](quality/m2-durable-sessions-acceptance.md) | Executed acceptance criteria, results and residual limits for durable sessions |
| [M3 TAS/replay acceptance](quality/m3-tas-replay-acceptance.md) | Executed acceptance criteria, results and residual limits for the v1 movie, record/replay, first divergence, restart and CLI |
| [M3 partial-replay acceptance](quality/m3-partial-replay-acceptance.md) | Executed acceptance criteria, results and residual limits for ranged replay/seek, single-tick advance, rewind and branch comparison |
| [M3 format-v2 acceptance](quality/m3-format-v2-acceptance.md) | Executed acceptance criteria, results and residual limits for the v2 movie container, anchors, anchor-based seek and multi-device input (t7 PASS, 2026-11-15) |
| [M3 rewind acceptance](quality/m3-rewind-acceptance.md) | Executed acceptance criteria, results and residual limits for the in-process rewind state cache and interactive `tas studio` stepping (t5 PASS, 2026-11-15) |
| [M3 rerecord acceptance](quality/m3-rerecord-acceptance.md) | Executed acceptance criteria, results and residual limits for input editing, suffix re-simulation, branch artifacts and anchor handling (M3-A35–A44; t10 repair-round-2 PASS, 2026-11-16) |
| [M3 hardware acceptance](quality/m3-hardware-acceptance.md) | Executed acceptance criteria, results and evidence for Mega Drive controller reads, hardware record/play/verify/edit/studio, snapshot/fork/rewind, two-controller input, `--rom` selection and M1/M2/M3 non-regression (M3-A45–A53; t5 PASS 2026-11-16) |
| [M4 debugging-depth acceptance](quality/m4-debugging-depth-acceptance.md) | Executed acceptance criteria, results and evidence for breakpoints, watchpoints, event breakpoints/logpoints, views/disassembly, Mega Drive inspectors and non-perturbation (M4-A1–A17; independent t5 PASS 2026-11-16) |
| [M5 tracing/scripting acceptance](quality/m5-tracing-and-scripting-acceptance.md) | Executed acceptance criteria, results and evidence for the canonical trace format/capture/query/compare/TSV, the sandboxed script DSL and both CLI surfaces (M5-A1–A39; final T12 PASS 2026-11-16) |
| [M6 agent-interfaces acceptance](quality/m6-agent-interfaces-acceptance.md) | Executed acceptance criteria, results and evidence for the service/transport split, canonical JSON/auth/ownership/concurrency, HTTP and MCP wire contracts, experiment artifacts, CLI parity and non-regression (M6-A1–A32; independent pass-2 PASS 2026-11-17) |
| [M7 GUI acceptance](quality/m7-gui-acceptance.md) | Executed acceptance criteria, results and evidence for the presentation crate, panels/layouts/renderers, pacing/run loop, capability-gated video/audio, attach/detach, handoff, remote observation and non-regression (M7-A1–A30; final PASS4 and REVIEW5 PASS 2026-11-17) |
| [Anti-slop policy](quality/anti-slop-policy.md) | Agent execution playbook: pre-flight, baseline-first, design rules, verification loop, review checklist |
| [Anti-slop refactor evidence](quality/anti-slop-refactor-evidence.md) | Verifier-owned baseline/after proof for the anti-slop refactor; §8 is the final PASS |
| [AGENTS.md](../AGENTS.md) | Mandatory repository instructions for coding agents, including the anti-slop workflow |
| [Licensing policy](licensing/policy.md) | Reuse and attribution rules |
| [Provenance](licensing/provenance.md) | Exact upstream revisions and incorporation status |
| [Upstream inventory](research/upstream-repositories.md) | Repositories, branches, commits, evidence |
| [Environment](development/environment.md) | Verified toolchain and limitations |
| [Getting started](development/getting-started.md) | Build, test and demo instructions |
| [ADR-0003: durable session storage](adr/0003-durable-session-storage.md) | Accepted durable-storage, metadata and ownership design; implemented and verified in Fase B |
| [ADR-0004: TAS movie format](adr/0004-tas-movie-format.md) | Accepted v1 canonical `.cfmovie`, embedded checkpoint, input schema and verification design; implemented and verified for the synthetic backend, deferred scope unchanged |
| [ADR-0005: partial replay and branch comparison](adr/0005-partial-replay-and-branch-comparison.md) | Accepted typed-range partial replay/seek by re-simulation and recorded-expectation branch comparison; implemented and verified for the synthetic backend, deferred scope unchanged |
| [ADR-0006: movie format v2](adr/0006-movie-format-v2-anchors-and-multi-device-input.md) | Accepted (owner-approved 2026-11-15) v2 container, checkpoint anchors, anchor-based seek and schema-2 multi-device input; implemented and independently verified for the synthetic backend (t7 PASS) |
| [ADR-0007: in-process rewind and interactive stepping](adr/0007-in-process-rewind-and-interactive-stepping.md) | Accepted (owner-approved 2026-11-15) bounded session-layer state cache, deterministic rewind and `tas studio` stdin surface; implemented and independently verified for the synthetic backend (t5 PASS) |
| [ADR-0008: rerecord and branch editing](adr/0008-rerecord-and-branch-editing.md) | **Accepted** (owner-approved 2026-11-15) functional input editing/truncation, deterministic suffix re-simulation, branch artifact identity/isolation and anchor policy; implemented and independently verified for the synthetic backend (t10 round-2 PASS, 2026-11-16) |
| [ADR-0009: real-hardware input TAS](adr/0009-real-hardware-input-tas.md) | **Accepted** (owner-approved 2026-11-15T23:38:06-03:00); implemented and independently verified for the deterministic minimal-slice scope (t5 PASS 2026-11-16): stateless per-step Mega Drive controller reads, canonical-state non-change, transient wiring and `--rom` TAS backend selection; M3 complete, t7 promotion audit PASS 2026-11-16 |
| [ADR-0011: tracing and scripting](adr/0011-tracing-and-scripting.md) | **Accepted** (owner-approved 2026-11-16 by the explicit Stage 5 directive); implemented and independently verified for the documented minimal-slice scope (final T12 PASS 2026-11-16; merged to `main` at `0c4e9f0`, PR #4): host trace surface, `CFTRACE1` v1, queries/comparison/TSV and the sandboxed deterministic DSL |
| [ADR-0012: agent-native interfaces](adr/0012-agent-native-interfaces.md) | **Accepted** (owner-approved 2026-11-17 by the explicit Stage 6 implementation directive); implemented and independently verified on `agent/m6-agent-interfaces` at base `93d47ed` (pass-2 PASS 2026-11-17; round-2 review pass): transport-agnostic service, bounded HTTP/1.1 + MCP adapters, ownership/concurrency boundary and experiment artifacts; merged to `main` at `cb05836` (PR #6) |
| [ADR-0013: human-facing GUI](adr/0013-human-facing-gui.md) | **Accepted and implemented** (owner-approved 2026-11-17 by the explicit Stage 7 implementation directive; implemented and independently verified 2026-11-17): presentation-only `chronofork-gui`, canonical views and renderers, pacing/run loop, capability-gated video/audio, attach/detach and human/agent handoff; verified on `agent/m7-gui` at base `93d47ed` (PASS4 + REVIEW5 PASS); merged to `main` at `cb05836` (PR #6) |
| [ADR index](adr/README.md) | Architectural decisions and status |

## Research reports

- [Emulator architecture](research/emulator-architecture.md)
- [TAS and debugging](research/tas-and-debugging.md)
- [Licensing assessment](research/licensing-assessment.md)
- [Oracle differential evidence](research/oracle-differential-evidence.md)

## Conventions

- Official technical documentation is written in English.
- Use Markdown links between related documents.
- Mark unverified upstream claims explicitly; do not present README claims as
  independently reproduced evidence.
- Record durable decisions in ADRs, not only in planning files or chat.
- Keep operational logs, scratch notes and temporary findings in `.planning/` or
  another ignored directory.
- `Proposed` ADRs have not been approved. Do not implement a major decision marked
  `Proposed` as if it were accepted.
