# ChronoFork Documentation

This directory is the durable, human- and agent-readable knowledge base for
ChronoFork. Operational working memory lives in the ignored `.planning/` directory;
it is never a substitute for these documents.

## Status

Bootstrap foundation complete, with a partial real-hardware slice implemented. The
repository builds and tests a deterministic synthetic backend, the generic session
layer, and a minimal 68000 + ROM/RAM Mega Drive backend with a VDP
status/control/VRAM port window and a deterministic instruction-quantum interrupt
scheduler, including 68000 SR-mask interrupt gating (level 7 always admitted; other
levels only when strictly above the mask), `RTE` handler return and VDP VINT
pending/acknowledge. A bounded debug-only instruction trace and final oracle
differential evidence now exist: three synthetic ROMs match the pinned Exodus
oracle's projected address streams and selected final state, and all three negative
controls pass. `rom_alu` and `rom_mem_vdp` use the explicit 1,000,000-instruction
controlled window, while `rom_masked` uses the CLI default quantum 64 with the flag
omitted (the pre-repair `rom_mem_vdp` divergence and its E2-13 repair remain
recorded, and the earlier two-ROM run is preserved as pre-E2-15 controlled-condition
history). Remaining limits: privilege is not enforced; only HBlank level 4 / VBlank
level 6 sources are modelled, with level 7 a predicate rule only; no interrupt-timing
equivalence is claimed; and VDP interrupt-enable/acknowledge logic is absent.
Rendering, DMA, audio, Z80 and controllers are not implemented. ADR-0002 is
accepted. Claims below distinguish implemented facts from proposals and upstream
research claims.

**M2 durable sessions are complete and independently verified (round 2, after the F1
repair).** Fase B persists canonical checkpoints under an explicit versioned root
and provides named metadata, deterministic listing, explicit non-cascading delete with
no automatic GC, a cooperative per-session writer lock/lease, storage-backed
rehydration and the `chronofork session save/load/list/delete` CLI. The M2-A1–A11
acceptance criteria and the full gate passed in two independent verification rounds;
round 1 found one medium CLI lock-scope finding (F1), which was repaired and
re-verified closed in round 2. See the executed [M2 acceptance
plan](quality/m2-durable-sessions-acceptance.md) and the dated [quality
gates](quality/quality-gates.md) result. Residual limits: durability is verified only
on local Linux filesystems (Windows/network filesystems unverified); the writer lock is
cooperative metadata, not authentication; a concurrent publish/read or delete/read
interleaving is not separately tested (`NFR-CONC-003` is Partial); holding the lock
across a long in-memory `--steps` advance lengthens contention; a crashed holder needs explicit
owner-checked recovery; retention is manual; there is no encryption, authentication or
format migration; and the public convenience `save_snapshot` remains publish-only for
third-party callers while the CLI uses the lock-aware workflow.

**M3's first and second increments (TAS, deterministic replay, partial replay/seek and
branch comparison) are implemented and independently verified (M3-T4 and M3-T5 PASS,
2026-11-15).** The first increment adds the canonical single-file v1 `.cfmovie` format,
record/playback of a per-tick input sequence on the deterministic synthetic backend,
exact first-divergence reporting, restart and M2-seeded replay from the embedded
checkpoint, and the `chronofork tas record/play/verify` CLI. The second increment adds
ranged replay/seek over an exact `(from, until]` window with typed range bounds, forward
single-tick advance and rewind by deterministic linear re-simulation from the embedded
checkpoint, and branch comparison of recorded `(tick, state hash)` records with
first-divergence, length and embedded-checkpoint identity reporting (no re-simulation).
M3 is **not complete**: periodic anchors/checkpoints and seek via checkpoints (format
v2), an in-process rewind buffer/state cache and interactive stepping, rerecord, branch
editing, multi-controller, ports, analog and keyboard input, real-hardware input TAS,
import/export adapters, checkpoint-reference movies, HTTP/MCP/GUI and the daemon remain
deferred. The executed [M3 acceptance plan](quality/m3-tas-replay-acceptance.md), the
[increment-2 acceptance plan](quality/m3-partial-replay-acceptance.md),
[ADR-0004](adr/0004-tas-movie-format.md),
[ADR-0005](adr/0005-partial-replay-and-branch-comparison.md) and the dated
[quality gates](quality/quality-gates.md) results record the contracts, evidence and
residual limits; M2 remains the previous completed milestone.

## Start here

| Document | Purpose |
|---|---|
| [Product vision](vision/product-vision.md) | Why ChronoFork exists and what it must become |
| [Scope](vision/scope.md) | What is in the current and next milestones |
| [Non-goals](vision/non-goals.md) | Explicit boundaries and anti-goals |
| [Architecture overview](architecture/overview.md) | Layers, crates and dependency rules |
| [Backend model](architecture/backend-model.md) | Capability contract and snapshot compatibility |
| [Session model](architecture/session-model.md) | Identity, lifecycle, fork and isolation |
| [Execution model](architecture/execution-model.md) | Virtual time, determinism and replay |
| [Backend strategy](architecture/backend-strategy.md) | Accepted real-backend strategy |
| [Mega Drive 68000 slice](architecture/megadrive-slice.md) | Implemented 68000 + VDP port + interrupt-scheduler slice and debug trace |
| [Requirements](requirements/functional.md) | Functional requirements and traceability |
| [Non-functional requirements](requirements/non-functional.md) | Determinism, quality, portability |
| [Roadmap](roadmap/roadmap.md) | Long-term stages |
| [Milestones](roadmap/milestones.md) | Deliverable checkpoints |
| [EPIC-0001](epics/EPIC-0001-foundation.md) | Foundation epic (complete) |
| [EPIC-0002](epics/EPIC-0002-megadrive-slice.md) | Mega Drive 68000 slice epic |
| [Testing strategy](quality/testing-strategy.md) | Test categories and fixtures |
| [Quality gates](quality/quality-gates.md) | Commands that must pass |
| [M2 durable-sessions acceptance](quality/m2-durable-sessions-acceptance.md) | Executed acceptance criteria, results and residual limits for durable sessions |
| [M3 TAS/replay acceptance](quality/m3-tas-replay-acceptance.md) | Executed acceptance criteria, results and residual limits for the v1 movie, record/replay, first divergence, restart and CLI |
| [M3 partial-replay acceptance](quality/m3-partial-replay-acceptance.md) | Executed acceptance criteria, results and residual limits for ranged replay/seek, single-tick advance, rewind and branch comparison |
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
