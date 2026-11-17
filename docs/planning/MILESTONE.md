# Milestone 8: Functional Mega Drive emulator

**Status:** active
**Started:** 2026-11-17

## Goal

Deliver a functional Sega Mega Drive / Genesis emulator under the existing backend contract:
the complete 68000 and Z80 instruction sets, a complete VDP (rendering, DMA, timing,
CRAM/VSRAM, H/V counters, interrupts), YM2612 FM and PSG audio, cartridge/region/TMSS/
mapper/SRAM support, timers and the controller protocol — enough to boot and play a curated
set of commercial ROMs with honest, evidence-backed compatibility, deterministic headless
capture of video and audio, and an optional native windowed frontend. The backend is
considered complete, as the owner defined, only when it interprets the whole 68000 and Z80
and emulates the VDP and the rest, not when it runs the current synthetic fixtures. The
normative design is the
[milestone design](../superpowers/specs/2026-11-17-milestone-8-megadrive-functional-design.md).

## Definition of Done

- [ ] All planned phases complete.
- [ ] All tests passing and the full quality gate (`./scripts/check.sh`) green.
- [ ] The complete 68000 instruction set and addressing modes are implemented, including privilege (`USP`/`SSP`), `STOP`, `TRAP` and the documented exception groups, and are differential-tested against the pinned Exodus oracle.
- [ ] The complete Z80 instruction set, memory map and bus arbitration are implemented and tested.
- [ ] The VDP renders 256- and 320-pixel modes with planes, window, sprites, scrolling, priority and shadow/highlight, and implements CRAM/VSRAM, DMA, FIFO/access timing, H/V counters and VBlank/HBlank interrupt enable/acknowledge.
- [ ] YM2612 six-channel FM and PSG produce documented deterministic audio, mixed and resampled deterministically.
- [ ] Cartridge header/region/TMSS/checksum, ROM sizes above 4 MiB, a banking mapper and SRAM are implemented.
- [ ] Controllers on ports 1–2 (3-button and 6-button, full TH latch) and timer interrupts are implemented; port 3, analog and keyboard hardware are documented residual limits.
- [ ] A curated set of commercial ROMs boots and plays with video, audio and input at interactive speed; the compatibility list is documented and reflects executed evidence.
- [ ] Emulated time is cycle-based and independent of host time; unpaced, paced and stepped modes produce identical frame, audio and state hashes for the same input.
- [ ] Headless capture writes deterministic PNG frames and WAV audio, pinned by hash tests, with no display or audio device required.
- [ ] The native windowed frontend presents video and audio and accepts keyboard/controller input, is paced at 60 fps (NTSC) or 50 fps (PAL) by default, and preserves the presentation-only boundary.
- [ ] Save states serialize every device; payload versions are bumped, older snapshots are rejected with typed errors, and the break is documented.
- [ ] Documented measured performance: real-time 60/50 fps in paced mode and a recorded unpaced throughput, with the optimizations listed and their determinism implications stated.
- [ ] M0–M7 contracts that the new timing supersedes are re-evidenced or explicitly marked superseded; both deterministic demos still pass.
- [ ] No commercial ROM, BIOS, firmware or game asset is committed; validation artifacts stay under ignored paths.

## Execution Model

- Canonical time is **emulated cycles**; no host clock, sleep or thread scheduling may
  influence canonical state.
- Host pacing is a policy: `Unpaced` (run as fast as possible to a requested frame/cycle —
  default for headless agents and capture), `Paced` (60/50 fps with an injected host clock and
  audio buffering — default for the window, available headless with an explicit `--pace`), and
  `Manual`/step.
- The same inputs must produce byte-identical frames, audio and state hashes in every mode.
- Emulator-standard optimizations (decoded-instruction caches, per-scanline rendering, dirty
  caches, batching, allocation-free hot paths, tuned release profile) are expected but must
  preserve the equality tests; skip-frames, run-ahead and dynamic recompilation are not
  required and need their own ADR.

## Phases

1. Phase 8.1 — Cartridge, region, TMSS and boot [complete]
2. Phase 8.2 — Complete 68000 and the cycle time model [pending]
3. Phase 8.3 — Complete VDP [pending]
4. Phase 8.4 — Z80, bus arbitration and audio [pending]
5. Phase 8.5 — Timers, controllers and interrupts [pending]
6. Phase 8.6 — Commercial compatibility, accuracy evidence and performance [pending]
7. Phase 8.7 — Execution modes, pacing and headless capture [pending]
8. Phase 8.8 — Native windowed frontend [pending]

## Audit History

| Date | Verdict | Gaps |
|---|---|---|
| — | not audited | — |
