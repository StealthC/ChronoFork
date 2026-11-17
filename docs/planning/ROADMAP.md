# Project Roadmap

> Lifecycle layer written and maintained by the `project-orchestration` skill (payload
> restored from `skills-lock.json`). This file records **execution**: which milestone is
> current, its phases, and their status.
>
> **Single-source rule.** Capability scope, milestone status/integration state, evidence and
> deferred follow-ups live once in
> [`docs/roadmap/milestones.md`](../roadmap/milestones.md) and stage intent in
> [`docs/roadmap/roadmap.md`](../roadmap/roadmap.md). Do not copy evidence, capability prose
> or requirement statuses here; link to them. When a phase closes, update this file and
> [`MILESTONE.md`](MILESTONE.md), then reconcile the canonical milestone page only if
> integration state changed.
>
> **Proportionality adaptations for this small repository.** The project already numbers its
> stages M0–M8, so that numbering is kept instead of restarting at 1. Completed milestones
> are one-line records; their Definition of Done, work items and evidence stay on the
> canonical milestone page. Hardware depth H1–H5 is a parallel track, never a milestone.
> M8 is marked active because it is the current milestone — its phases are still pending, so
> the status does not mean implementation or merge is in progress. The canonical milestone page
> calls M8 `Next` (not started) and remains authoritative for that; this file's `active` and
> its `**Started:**` date record when the lifecycle layer took M8 as the current milestone,
> not when implementation began. When phase 8.1 starts, update the milestone page's
> integration state instead of silently leaving the two vocabularies to disagree.

## Milestone 0: Bootstrap foundation [status: complete]

**Goal:** Prove the architecture and establish durable project infrastructure.
**Completed:** 2026-11-13
**Record:** [M0](../roadmap/milestones.md#m0--bootstrap-foundation)

## Milestone 1: Backend decision and minimal hardware slice [status: complete]

**Goal:** Select the real-backend strategy and implement the smallest credible Mega Drive
core behind the existing contracts.
**Completed:** 2026-11-14
**Record:** [M1](../roadmap/milestones.md#m1--backend-decision-and-minimal-hardware-slice)

## Milestone 2: Durable sessions [status: complete]

**Goal:** Make checkpoints, metadata, ownership and reloads durable and trustworthy across
process restarts.
**Completed:** 2026-11-15
**Record:** [M2](../roadmap/milestones.md#m2--durable-sessions)

## Milestone 3: TAS and replay [status: complete]

**Goal:** Record, replay, verify, rewind and edit deterministic input movies on the synthetic
backend and on the minimal Mega Drive slice.
**Completed:** 2026-11-16
**Record:** [M3](../roadmap/milestones.md#m3--tas-and-replay)

## Milestone 4: Debugging depth [status: complete]

**Goal:** Let reverse-engineering and automation workflows inspect and stop a session without
a GUI.
**Completed:** 2026-11-16
**Record:** [M4](../roadmap/milestones.md#m4--debugging-depth)

## Milestone 5: Tracing and scripting [status: complete]

**Goal:** Let scripts run controlled experiments and produce reproducible reports without
host-language plugins.
**Completed:** 2026-11-16
**Record:** [M5](../roadmap/milestones.md#m5--tracing-and-scripting)

## Milestone 6: Agent interfaces [status: complete]

**Goal:** Expose the same application services through CLI, HTTP and MCP with identical
semantics.
**Completed:** 2026-11-17
**Record:** [M6](../roadmap/milestones.md#m6--agent-interfaces)

## Milestone 7: GUI [status: complete]

**Goal:** Provide optional human-facing presentation surfaces that never own canonical state.
**Completed:** 2026-11-17
**Record:** [M7](../roadmap/milestones.md#m7--gui)

## Milestone 8: Multi-backend expansion [status: active]

**Goal:** Integrate a second system behind the same capability model without changing session,
storage or transport core semantics, and report capability differences honestly.
**Started:** 2026-11-17
**Design:** [roadmap design](../superpowers/specs/2026-11-17-roadmap-design.md)
**Definition of Done:**
- [ ] A second system is integrated without changing session, storage or transport core semantics.
- [ ] The capability matrix distinguishes what each backend really supports instead of normalizing differences away.
- [ ] Cross-backend differential tests run and are recorded as evidence.
- [ ] FR-API-004 (external SDK contract and stability policy) is implemented or explicitly deferred with a recorded decision.
- [ ] The full quality gate passes and the canonical docs/ADRs are updated.

### Phase 8.1: Second-system decision and capability contract [status: pending]

**Goal:** Choose the second system with the owner's approval and record the capability,
versioning and compatibility contract in an accepted ADR.
**Surface:** Backend
**HelpWanted:** no
**Plan:** _to be written_

### Phase 8.2: Second backend core behind the backend contract [status: pending]

**Goal:** Implement the minimal second backend — CPU subset, memory map, versioned snapshot
payload and an honest capability report — behind the unchanged backend contract.
**Surface:** Backend
**HelpWanted:** no
**Plan:** _to be written_

### Phase 8.3: Backend-neutral shared layers and capability matrix [status: pending]

**Goal:** Prove the shared session, TAS, trace, debug and service paths work for both
backends, with differences surfaced in a capability matrix rather than normalized away.
**Surface:** Refactor
**HelpWanted:** no
**Plan:** _to be written_

### Phase 8.4: Cross-backend differential tests [status: pending]

**Goal:** Add an executable cross-backend differential corpus and record the evidence.
**Surface:** Backend
**HelpWanted:** no
**Plan:** _to be written_

### Phase 8.5: External SDK contract and stability policy [status: pending]

**Goal:** Define the versioned external SDK and its stability/compatibility policy
(FR-API-004).
**Surface:** Docs
**HelpWanted:** no
**Plan:** _to be written_

## Parallel hardware-depth tracks (not milestones)

Hardware completeness never becomes a milestone and never reopens a closed one. Status,
work items and evidence are recorded once in the canonical milestone page.

- [H1 — CPU and instruction completeness](../roadmap/milestones.md#h1--cpu-and-instruction-completeness):
  broader 68000 coverage, privilege enforcement, user/supervisor stacks and the remaining
  exception groups.
- [H2 — Video, VDP and DMA](../roadmap/milestones.md#h2--video-vdp-and-dma): rendering,
  CRAM/VSRAM, DMA, raster and H/V timing, VDP interrupt-enable/acknowledge logic and FIFO.
- [H3 — Audio and Z80](../roadmap/milestones.md#h3--audio-and-z80): Z80, bus arbitration,
  YM2612 and PSG.
- [H4 — Cartridge, controllers and timers](../roadmap/milestones.md#h4--cartridge-controllers-and-timers):
  ROM identity/mapper/SRAM, the full controller protocol and timer interrupts.
- [H5 — Accuracy and evidence](../roadmap/milestones.md#h5--accuracy-and-evidence): broader
  differential corpus, interrupt-timing evidence, cycle accuracy, cross-platform determinism
  and performance after correctness.

## Deferred follow-ups (not scheduled phases)

M2-FU1, M3-FU1–FU3, M4-FU1/FU2, M6-FU1–FU4, FR-SNAPSHOT-007, FR-EXEC-006 and NFR-CONC-003
are recorded in the [canonical milestone page](../roadmap/milestones.md) and the
[requirements](../requirements/functional.md). A follow-up becomes a phase only when it is
groomed into the active milestone; until then it stays out of this execution plan.
