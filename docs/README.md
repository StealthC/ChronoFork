# ChronoFork Documentation

This directory is the durable, human- and agent-readable knowledge base for
ChronoFork. Operational working memory lives in the ignored `.planning/` directory;
it is never a substitute for these documents.

## Status

Bootstrap foundation complete, with a partial real-hardware slice implemented. The
repository builds and tests a deterministic synthetic backend, the generic session
layer, and a minimal 68000 + ROM/RAM Mega Drive backend with a VDP
status/control/VRAM port window and a deterministic instruction-quantum interrupt
scheduler. Rendering, DMA, audio, Z80 and controllers are not implemented. ADR-0002
is accepted. Claims below distinguish implemented facts from proposals and upstream
research claims.

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
| [Mega Drive 68000 slice](architecture/megadrive-slice.md) | Implemented 68000 + VDP port + interrupt-scheduler slice |
| [Requirements](requirements/functional.md) | Functional requirements and traceability |
| [Non-functional requirements](requirements/non-functional.md) | Determinism, quality, portability |
| [Roadmap](roadmap/roadmap.md) | Long-term stages |
| [Milestones](roadmap/milestones.md) | Deliverable checkpoints |
| [EPIC-0001](epics/EPIC-0001-foundation.md) | Foundation epic (complete) |
| [EPIC-0002](epics/EPIC-0002-megadrive-slice.md) | Mega Drive 68000 slice epic |
| [Testing strategy](quality/testing-strategy.md) | Test categories and fixtures |
| [Quality gates](quality/quality-gates.md) | Commands that must pass |
| [Licensing policy](licensing/policy.md) | Reuse and attribution rules |
| [Provenance](licensing/provenance.md) | Exact upstream revisions and incorporation status |
| [Upstream inventory](research/upstream-repositories.md) | Repositories, branches, commits, evidence |
| [Environment](development/environment.md) | Verified toolchain and limitations |
| [Getting started](development/getting-started.md) | Build, test and demo instructions |
| [ADR index](adr/README.md) | Architectural decisions and status |

## Research reports

- [Emulator architecture](research/emulator-architecture.md)
- [TAS and debugging](research/tas-and-debugging.md)
- [Licensing assessment](research/licensing-assessment.md)

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
