# Session State — 2026-11-17

**Date:** 2026-11-17

## Current Position

- **Milestone:** 8 — Multi-backend expansion
- **Phase:** 8.1 — Second-system decision and capability contract
- **Last completed task:** Adopted the `project-orchestration` lifecycle (no phase plan file
  exists yet; phase 8.1 has not started). Added committed `docs/planning/` state, the
  approved roadmap design, the `skills-lock.json` pin for the skill, and the `AGENTS.md`
  lifecycle/GitHub/development/harness-independence rules.
- **Next task:** Design and plan phase 8.1 (`brainstorming` → `writing-plans`), then build it.

## Open Decisions

- **Second system for M8** — not decided. Candidate directions to evaluate in phase 8.1:
  a Z80-based Master System (low overlap with the existing 68000 slice, reuses no H3 work),
  a 6502-based NES (very different device model, large rendering surface), or a Game Boy-style
  LR35902 system (small CPU, simple device model). The owner must approve the ADR before
  phase 8.2 starts; do not assume one.
- **External SDK shape (FR-API-004)** — whether phase 8.5 publishes a versioned external SDK
  or records an explicit deferral.
- **GitHub Projects v2** — deferred; native Issues and Milestones are the projection.
- **`/.planning/` scratch (planning-with-files)** — retained as ignored scratch only; retire it
  later if it stops being used.

## Blockers

- **First GitHub publication awaits explicit owner approval.** `init-github-sync` was run in
  dry-run mode only on 2026-11-17; no label, milestone or issue was created. Local planning is
  the source of truth until that approval.

## Recommended Next Step

Resume with the lifecycle router (`start-next-phase`, or `groom-milestone` to design and plan
all five M8 phases in one sitting), starting at phase 8.1's design. Do not implement phase 8.1
before its plan is approved; the milestone stays `active` until `complete-milestone` runs
after an audit.
