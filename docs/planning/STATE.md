# Session State — 2026-11-17

**Date:** 2026-11-17

## Current Position

- **Milestone:** 8 — Functional Mega Drive emulator
- **Phase:** 8.1 — Cartridge, region, TMSS and boot
- **Last completed task:** Redefined M8 as the functional Mega Drive emulator (full 68000/Z80/
  VDP/audio/cartridge), moved the former multi-backend milestone to M9, and created the
  milestone design + planning state. No phase plan file exists yet; phase 8.1 has not started.
- **Next task:** Design and plan phase 8.1 (`brainstorming` → `writing-plans`), then build it.
  Phase 8.2 (cycle time model) is the foundational follow-up and should be planned immediately
  after 8.1.

## Open Decisions

- **Curated commercial ROM set** — which titles define M8's playability proof, and which region
  (NTSC-US, NTSC-J, PAL)? The owner selects this before phase 8.6. ROMs are never committed.
- **Audio differential tolerance** — exact sample equality against the Exodus oracle, or a
  documented tolerance with ChronoFork-side hashes? Decide in phase 8.4.
- **Port 3 / analog / keyboard** — inside M8 or recorded as H4 residual limits? Recommended:
  residual limits, since the curated set uses ports 1–2.
- **Native frontend dependencies** — `winit` + `softbuffer` + `cpal` in a separate crate, behind
  an ADR that supersedes ADR-0013's no-toolkit constraint. Required before phase 8.8.
- **M9 second system** — still undecided; it moves to phase 9.1 and is not blocking M8.

## Blockers

- None. Lifecycle state, GitHub projection (M8/M9 milestones and phase issues) and the
  consolidated skill lockfile are in place.

## Recommended Next Step

Resume with the lifecycle router (`start-next-phase`, or `groom-milestone` to design and plan
the phases in one sitting), starting at phase 8.1. Do not implement phase 8.1 before its plan is
approved; M8 stays `active` until `complete-milestone` runs after an audit. When the first phase
starts, update the canonical milestone page's integration state instead of leaving `Next` and
`active` to disagree silently.
