# Session State — 2026-11-19

**Date:** 2026-11-19

## Current Position

- **Milestone:** 8 — Functional Mega Drive emulator (**active**; phase 8.3 now complete).
- **Phase 8.3 — Complete VDP: complete and integrated.** Implemented on
  `agent/8.3-vdp` (24 implementation commits, HEAD `e1433ed`), independently verified
  (rounds 1–3 PASS) and independently reviewed (rounds 2–3 PASS). The integration commit
  closes the phase in `docs/planning/ROADMAP.md`/`MILESTONE.md`, pushes the branch and opens
  the phase-8.3 PR linked to issue #16; the PR is the integration record for the merge into
  `main`. If a fresh session reads this before the merge lands, check the PR/issue #16 state
  first.
- **Delivered:** the complete VDP — CRAM (64 words) and VSRAM (40 words) with word/byte
  addressing and auto-increment, plane A/B and window nametables with full/column/line
  scrolling, 80 link-ordered sprites with flips/duplication/collision/overflow latches,
  six-level priority, shadow/highlight, the H40 border, DMA (memory→VRAM/CRAM/VSRAM, fill,
  copy) with pre-validation and busy windows, the bounded 4-word FIFO with 68000 wait slots
  surfaced through the bus and applied as `wait*7` master cycles, VDP-owned VBlank/HBlank
  pending/acknowledge gated by register `0x00`, H/V counters and line/dot timing, and
  deterministic 256/320×224/240 RGBA frames through the backend-neutral `MediaSource` seam
  (one frame per VDP frame, second take `None`). Payload v5
  (`CFMD68K5`/`CFMDSTATE5`/`CFMDCONF5`) embeds the `CFVDP002` VDP state and frame index, and
  v1–v4 are rejected with typed errors; `rendering = true`, `audio = false`. Evidence: the
  pinned-oracle VDP state differential ([VDP state differential](../research/vdp-state-differential-evidence.md))
  plus the workspace gate and the H32/H40 frame fixtures.
- **Honest residual limits:** interlace, dot-exact H/V counters, the physical byte-swapped
  VRAM layout, exact H32 window geometry, line-mode vertical scrolling and column-scroll
  details are not modelled; DMA's source is a VRAM address (no 68000-memory view) and
  transfers are eager; VSRAM data-port reads are undecoded; the headless oracle exposes no
  VDP memory content, so pixels are pinned internally (no cross-emulator pixel comparison);
  the latched command CD5 is not cleared after an eager DMA, so a later data-port write with
  no new command could re-trigger a transfer (untested edge, hardware clears CD5 on
  completion); the `megadrive demo` renders a frame on every `inspect` and takes ~65–70 s.
  Z80, YM2612/PSG audio, timer interrupts, the full controller protocol, cartridge mapper
  variants/SRAM banking and headless PNG/WAV capture remain later M8 phases.
- **Next task:** phase 8.4 — Z80, bus arbitration and audio. Its plan is already
  groomed at [2026-11-17-8.4-z80-audio.md](../superpowers/plans/2026-11-17-8.4-z80-audio.md)
  (issue #17), so a fresh session routes `resume-work` → `start-next-phase` →
  `executing-plans`.
- **CI runtime follow-up (issue #28): complete and merged** (PR #30). The workflow now
  triggers only on `main` pushes and PRs (the duplicate branch-push run is gone), cancels
  superseded PR runs, runs the deterministic demos from the release binary (output
  byte-identical to debug) and splits the workspace tests into four parallel crate groups.
  Measured on the PR run: total workflow ~29.5 → **14m34s**; `Test` critical path 28m21s →
  **14m31s** (`cpu`), with `apps` 5m56s, `host` 1m13s and `core` 35s; demos 3m16s → **1m39s**.
  Non-blocking follow-ups recorded in the PR: a `cargo metadata` vs group-union guard when a
  crate is added, an optional per-group cache-key suffix, and listing all four `Test (…)`
  checks if branch protection is ever enabled.

## Open Decisions

- **None blocking.** The VDP limits above are documented, reviewed and deliberately
  deferred; they do not block closing 8.3 or starting 8.4.
- Audio differential tolerance (phase 8.4), port 3/analog/keyboard as H4 residual limits,
  native frontend dependencies (`winit`/`softbuffer`/`cpal` + ADR superseding ADR-0013,
  required before 8.8) and the M9 second system remain open as before.
- The curated commercial ROM set (Kid Chameleon and Chakan) remains phase 8.6 evidence.

## Blockers

- None. Phase 8.3 is complete; the branch is pushed and the PR is the merge record.

## Recommended Next Step

Resume with `resume-work` → `start-next-phase`: phase 8.4 (Z80, bus arbitration and audio)
already has an approved design and groomed plan and branches from the current `main`, which
now includes phases 8.1–8.3 and the CI runtime follow-up.
