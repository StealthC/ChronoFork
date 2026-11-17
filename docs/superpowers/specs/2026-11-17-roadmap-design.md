# Roadmap Design — ChronoFork

> Template consumed by `plan-roadmap`. The brainstorm fills in every section;
> `plan-roadmap`'s VERIFY step grep-checks the headings, milestone count, and Surface tags
> before writing `docs/planning/ROADMAP.md`. Save the filled-in copy at
> `docs/superpowers/specs/YYYY-MM-DD-roadmap-design.md`.

**Date:** 2026-11-17
**Author:** DeepSeek Harness agent (captain), approved by the repository owner
**Stage:** roadmap (whole project)

## Goal

ChronoFork is a deterministic, forkable emulation and reverse-engineering platform: an
emulated machine is a session that can be checkpointed, branched and replayed exactly, and
humans, scripts and AI agents drive the same canonical services. The project has already
delivered its foundation and capability stages M0–M7 (bootstrap, minimal Mega Drive core,
durable sessions, TAS/replay, debugging, tracing/scripting, agent interfaces, presentation
GUI). Success at the project level means a second system runs behind the same contract with
honest capability differences, and the Mega Drive model deepens in parallel until it can be
trusted as a research tool — never by overstating what is implemented.

This design establishes the **durable lifecycle layer** those milestones are executed
through: `docs/planning/` as the current plan and session handoff, GitHub Issues/PRs as the
trackable projection, and `AGENTS.md` as the harness-independent index of rules.

## Target Users / Stakeholders

- Primary user / persona — the reverse engineer, TAS author and emulator developer who needs
  reproducible sessions, traces, movies and honest capability reporting.
- Secondary users / stakeholders — AI agents and scripts that drive the CLI, HTTP and MCP
  surfaces; maintainers who review PRs and need one source of truth for status.
- Non-users explicitly out of scope — players seeking a commercial-game emulator; hosted or
  cloud execution; anyone expecting a complete Mega Drive.

## Top-Level Success Criteria

- A deterministic minimal slice and its host layers (M0–M7) stay testable and honestly
  documented; the quality gate passes from a clean checkout.
- A second backend runs behind the unchanged backend/session/storage/transport contracts,
  with capability differences reported rather than normalized away (M8).
- Hardware depth H1–H5 is tracked honestly, and no accuracy or compatibility claim exists
  without executable evidence.
- Any new agent session can resume from `docs/planning/` and `AGENTS.md` alone, without the
  conversation that produced them.
- Every tracked activity maps to a GitHub Issue and lands through a reviewed PR.

## Approaches Considered

- **A — Thin lifecycle layer over the existing roadmap (chosen).**
  `docs/planning/` records the execution plan (active milestone, phases, session state,
  conventions) and links to `docs/roadmap/milestones.md`, which stays canonical for
  capability scope, evidence and follow-ups. Minimal churn, no evidence duplication.
- **B — Absorb status into `docs/planning/`.**
  Move milestone status/integration state into `docs/planning/ROADMAP.md` and reduce
  `docs/roadmap/` to scope intent. Removes duplication in one place but rewrites a large
  body of verified documentation and risks dropping evidence links; rejected for a
  migration that must not lose information.
- **C — Only `STATE.md` + `CONVENTIONS.md`.**
  The smallest possible footprint, but `project-orchestration` routes on `ROADMAP.md` and
  `MILESTONE.md`; without them `start-next-phase` has nothing to route and the workflow is
  not actually adopted. Rejected.

The owner chose **A**, with M8 as the current milestone (its `active` status means
"current milestone", not "merged") and the second-system choice deferred to phase 8.1.

## Milestones

The project already numbers its capability stages M0–M8, so this roadmap keeps that
numbering instead of renumbering from 1. Completed milestones are recorded as one line
each; their Definition of Done, work items and evidence remain in the canonical milestone
page.

### Milestone 0: Bootstrap foundation

**Goal:** Prove the architecture and establish durable project infrastructure.
**Status:** complete (2026-11-13).

### Milestone 1: Backend decision and minimal hardware slice

**Goal:** Select the real-backend strategy and implement the smallest credible Mega Drive
core behind the existing contracts.
**Status:** complete (2026-11-14).

### Milestone 2: Durable sessions

**Goal:** Make checkpoints, metadata, ownership and reloads durable and trustworthy across
process restarts.
**Status:** complete (2026-11-15).

### Milestone 3: TAS and replay

**Goal:** Record, replay, verify, rewind and edit deterministic input movies on the
synthetic backend and on the minimal Mega Drive slice.
**Status:** complete (merged 2026-11-16, PR #1).

### Milestone 4: Debugging depth

**Goal:** Let reverse-engineering and automation workflows inspect and stop a session
without a GUI.
**Status:** complete (merged 2026-11-16, PR #2).

### Milestone 5: Tracing and scripting

**Goal:** Let scripts run controlled experiments and produce reproducible reports without
host-language plugins.
**Status:** complete (merged 2026-11-16, PR #4).

### Milestone 6: Agent interfaces

**Goal:** Expose the same application services through CLI, HTTP and MCP with identical
semantics.
**Status:** complete (merged 2026-11-17, PR #6).

### Milestone 7: GUI

**Goal:** Provide optional human-facing presentation surfaces that never own canonical
state.
**Status:** complete (merged 2026-11-17, PR #6).

### Milestone 8: Multi-backend expansion

**Goal:** Integrate a second system behind the same capability model without changing
session, storage or transport core semantics, with capability differences reported honestly.

**Phases:**

1. `Second-system decision and capability contract` — `Surface: Backend` — choose the system
   and record the capability, versioning and compatibility contract in an accepted ADR.
2. `Second backend core behind the backend contract` — `Surface: Backend` — the minimal
   second backend (CPU subset, memory map, versioned snapshot payload, honest capability
   report).
3. `Backend-neutral shared layers and capability matrix` — `Surface: Refactor` — the shared
   session/TAS/trace/debug/service paths work for both backends; differences surface in the
   capability matrix instead of being normalized away.
4. `Cross-backend differential tests` — `Surface: Backend` — an executable differential
   corpus and recorded evidence.
5. `External SDK contract and stability policy` — `Surface: Docs` — the versioned SDK and
   stability policy tracked as FR-API-004.

## Dependencies and Ordering Rationale

- M0 → M1: contracts and the synthetic validator had to exist before a real core.
- M1 → M2/M3/M4/M5: persistence, movies, debugging and tracing all observe a real core.
- M6 → M7: the GUI consumes the service/transport boundary; it must never reach a backend
  directly.
- M8 depends on the M2–M7 shared layers; it is the first unblocked, unstarted milestone.
- H1–H5 run in parallel and never gate a capability stage; a stage may be complete while the
  machine model stays narrow.
- External dependencies: none required — the project is self-contained, uses no third-party
  runtime dependency beyond `sha2`, and keeps upstream research under ignored paths.

## Risk Register (optional)

| Risk | Impact | Mitigation |
|---|---|---|
| Duplicated status between `docs/planning/` and `docs/roadmap/milestones.md` drifts | Two conflicting sources of truth | Single-source rule stated in `ROADMAP.md`; evidence/scope only in the milestone page |
| The second-system choice silently assumes a hardware decision | Roadmap phases planned against an unapproved decision | Phase 8.1 produces an ADR with alternatives; no choice is made in this migration |
| GitHub projection diverges from local files | Local status overwritten or contradictory issues | One-way sync (local → GitHub); external edits are advisory and reconciled consciously |
| Planning prose grows into a second documentation set | Maintenance cost, drift | `AGENTS.md` stays an index; `docs/planning/` holds state, not capability prose |

## Open Questions

- Which second system does M8 target (for example Master System, NES, Game Boy or another)?
  Decision and alternatives belong to phase 8.1; it needs owner approval before an ADR is
  accepted.
- Should `GitHub Projects v2` be adopted after native Issue/Milestone sync proves useful?
  Deferred; the native integration covers Issues and Milestones only.
- Should the local `planning-with-files` scratch (`/.planning/`) remain, or be retired in
  favor of `docs/planning/`? It is demoted to scratch for now.

## Lifecycle Layer Notes (migration record)

- `docs/planning/CONVENTIONS.md` records detected conventions plus the owner's confirmed
  choices: feature-branch model, `main` protected by convention, PR required, no release
  scheme or tags yet.
- `docs/planning/STATE.md` is written by `pause-work` and is the entry point for the next
  session.
- Sub-skill dependencies verified from their installed `SKILL.md` files: `brainstorming`
  (this document), `writing-plans` (phase plans under `docs/superpowers/plans/`),
  `executing-plans` (implementation).
- GitHub sync is initialized only after an explicit dry-run approval; the first publication
  is intentionally not part of this migration.
