# Milestone 8 Design — Functional Mega Drive emulator

> Template consumed by `new-milestone`. The brainstorm fills in every section; `new-milestone`'s
> VERIFY step grep-checks the headings, DoD criteria count, and per-phase Surface tags before
> writing `docs/planning/MILESTONE.md` and adding the milestone block to ROADMAP.md.

**Date:** 2026-11-17
**Milestone:** `8`
**Stage:** milestone (one milestone, multiple phases)

## Goal

Deliver a **functional Sega Mega Drive / Genesis emulator** under the existing backend
contract: full 68000 and Z80 instruction sets, a complete VDP (rendering, DMA, timing,
CRAM/VSRAM, H/V counters, interrupts), YM2612 FM and PSG audio, cartridge/region/TMSS/
mapper/SRAM support and the controller protocol, with deterministic headless capture of
video and audio and an optional native windowed frontend. The backend is only considered
complete — as the owner defined — when it can interpret the whole 68000 and Z80 and emulate
the VDP and all the rest, enough to boot and play a curated set of commercial ROMs with
honest, evidence-backed compatibility.

The milestone also fixes the execution model that makes this usable by both automation and
humans: **emulated time is a canonical concept measured in CPU cycles**, while host pacing
is a separate policy (`Unpaced` for headless/agent batch work, `Paced` for the window and
on request, `Manual`/step for debugging). Same inputs must produce the same frames, audio
and state hashes in every mode.

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

## Execution model (normative)

This section is the contract every phase and the frontend must satisfy.

1. **Canonical time is emulated cycles.** The machine advances in deterministic emulated
   cycles (68000, Z80 and VDP clocks with their documented ratios). Emulated time is part of
   snapshots and state hashes. No host clock, sleep, thread scheduling or wall-time value may
   influence canonical state.
2. **Pacing is host policy, never canon.** The host layer chooses how fast to *drive* the
   deterministic core. Supported modes:
   - `Unpaced` — run as fast as the host allows to a requested target (frame, scanline,
     cycle or input boundary). Default for headless agents, batch experiments and capture.
   - `Paced` — maintain the target field rate (59.94/60 NTSC, 50 PAL) using an injected host
     clock and audio buffering. Default for the native window; available in headless with an
     explicit `--pace` request.
   - `Manual`/step — advance a requested number of cycles/instructions/frames, used by tests,
     debugging and TAS stepping.
3. **Identical behavior across modes.** A frame produced unpaced and the same frame produced
   paced must be byte-identical, and the same holds for audio samples. Only *when* the host
   asks for work changes, never *what* the machine computes.
4. **Run-to-target API.** Headless automation requests explicit targets (`--frames`,
   `--to-cycle`, `--to-frame`) instead of sleeping. The existing deterministic `pulse`
   surface remains valid.
5. **Capture is a pure function of state and input.** Frame (PNG) and audio (WAV) artifacts
   carry canonical hashes; captures do not depend on window size, host load or audio device
   availability.
6. **Optimizations must preserve the contract.** Emulator-standard techniques are allowed and
   expected: decoded-instruction caches, per-scanline rendering, dirty-region/plane caches,
   batched device stepping, allocation-free hot paths, audio sample batching and a tuned
   release profile (e.g. LTO). Every optimization must pass the differential and cross-mode
   equality tests; none may change observable results or introduce host-dependent state.
   Skip-frames, run-ahead, savestate-rewind shortcuts and dynamic recompilation are **not**
   required and, if ever considered, need their own ADR and determinism analysis.
7. **Honest capability reporting.** `Capabilities::rendering` and `Capabilities::audio`
   become `true` only when the corresponding phase is verified; the modes and limits are
   documented in the canonical capability pages.

## Phases

1. **Phase 8.1: Cartridge, region, TMSS and boot** — `Surface: Backend`
   - **Goal:** read ROM header/region/TMSS/checksum, support >4 MiB images, a banking mapper and SRAM, and boot a cartridge deterministically to its main loop.
2. **Phase 8.2: Complete 68000 and the cycle time model** — `Surface: Backend`
   - **Goal:** implement the full 68000 instruction set, addressing modes, privilege and exception groups with cycle accounting, defining canonical emulated time for the machine.
3. **Phase 8.3: Complete VDP** — `Surface: Backend`
   - **Goal:** implement CRAM/VSRAM, planes/window/sprites/scrolling/priority/shadow-highlight, 256/320 modes, DMA, FIFO/access timing, H/V counters and VDP interrupt enable/acknowledge, with frame-level differential evidence.
4. **Phase 8.4: Z80, bus arbitration and audio** — `Surface: Backend`
   - **Goal:** implement the Z80 core and memory map, 68000/Z80/VDP bus arbitration, YM2612 six-channel FM and PSG, with deterministic mixing/resampling and audio differential evidence.
5. **Phase 8.5: Timers, controllers and interrupts** — `Surface: Backend`
   - **Goal:** implement timer interrupts, the full TH latch and 3-/6-button protocol on ports 1–2, and integrate interrupt delivery with cycle timing.
6. **Phase 8.6: Commercial compatibility, accuracy evidence and performance** — `Surface: Backend`
   - **Goal:** drive a curated commercial ROM set to playable video+audio+input, record the compatibility matrix and H5 evidence, and reach measured interactive speed with the documented optimizations.
7. **Phase 8.7: Execution modes, pacing and headless capture** — `Surface: Mixed`
   - **Goal:** implement `Unpaced`/`Paced`/`Manual` execution modes with run-to-target APIs, plus deterministic PNG frame and WAV audio capture that is identical across modes.
8. **Phase 8.8: Native windowed frontend** — `Surface: UI`
   - **Goal:** ship an optional native window (video, audio, keyboard/controller input, pause/step/save-state) paced at 60/50 fps, in a separate crate behind an accepted ADR that supersedes ADR-0013's no-toolkit constraint.

## Dependencies on Prior Milestones

- Depends on M1 (backend contract, bus, snapshots), M2 (durable snapshots), M3 (movies and
  deterministic replay), M4 (debug views), M5 (trace/script), M6 (service/transport) and M7
  (headless presentation seam: `PresentationFrame`, `AudioChunk`, `VideoSource`,
  `AudioSink`, `HostClock`).
- Depends on the accepted [ADR-0002](../../adr/0002-backend-boundaries.md) (backend boundaries)
  and the licensing policy; the native frontend depends on a new accepted ADR.
- The Exodus headless oracle (`exodus-for-ais`, MIT, pinned) is the differential reference
  for 68000/Z80/VDP behaviour and audio; it is used as an external oracle, never vendored.

## Reference Fixtures and Acceptance Scenarios

The first two commercial fixtures are recorded in [ROM fixtures](../../quality/rom-fixtures.md):
*Kid Chameleon (USA, Europe)* and *Chakan (USA, Europe)*, No-Intro-verified by CRC-32 and MD5.
The bytes stay only under the ignored `.research/roms/` directory of the primary checkout and
are shared with worktrees by absolute path; the tracked manifest records names, regions,
headers and hashes, never the ROM itself.

They drive three escalating scenarios:

1. **Boot** — reset vectors, TMSS, header/region parse and the main loop, reached headlessly.
2. **Intro** — the opening/attract sequence with **no input**; both titles present graphics and
   music (GEMS driver), exercising VDP rendering, DMA, YM2612 and PSG before gameplay. Frames
   and audio are captured deterministically and hash-pinned.
3. **TAS movie** — replay a full-game input movie and verify convergence and completion; the
   movie is an ignored local artifact, never the ROM.

More titles may be added over time; the list stays curated and documented, never a "runs
everything" claim.

## External Constraints

- Commercial ROMs, BIOS/firmware and game assets are never committed; compatibility
  validation runs locally and stores only non-redistributable evidence under ignored paths.
- The project is MIT; only permissively licensed references may be studied, and any code
  incorporation needs provenance, NOTICE updates and owner approval. GPL/NC sources are not
  usable.
- Accuracy and compatibility are empirical: the milestone promises a **documented curated
  list**, not universal compatibility.
- Real-time audio needs a native dependency (`cpal` or equivalent) and an ADR that supersedes
  ADR-0013's no-toolkit constraint; windowing (`winit` + `softbuffer`) is the intended stack.
- The workspace forbids `unsafe` in first-party code; dependencies are audited instead.

## Risk Areas

| Risk | Impact | Mitigation |
|---|---|---|
| Scope is very large (full CPU/VDP/audio) | Long milestone, risk of never closing | Eight small phases with executed evidence; `audit-milestone` per phase; honest deferrals |
| Cycle-accurate timing invalidates M1–M7 pinned evidence, demos, movies and state hashes | Contradictory claims, broken determinism tests | Explicit behavior/state version bump, re-evidence, supersession notes, old formats reject-only |
| Commercial compatibility is empirical and unbounded | Schedule and trust risk | Curated ROM list with documented status; differential oracle; no "runs everything" claim |
| Audio/video native dependencies conflict with ADR-0013 and minimal-dependency rule | Architecture violation or stalled GUI | Separate frontend crate, exact ADR before adding dependencies, headless-first delivery |
| Pacing leaks host time into canonical state | Non-deterministic results | Normative execution model above; cross-mode hash equality tests; injected clocks only |
| Performance work changes behavior | Silent divergence | Every optimization gated by differential tests and cross-mode equality; no skip/run-ahead |
| Legal contamination from upstream code | License violation | Reference-only by default; audited permissive incorporation only with provenance and approval |

## Open Questions

- Which additional commercial titles join the curated compatibility set (the first two are
  recorded in [ROM fixtures](../../quality/rom-fixtures.md)), and which region(s) must be
  supported beyond the current NTSC-U dumps (NTSC-J, PAL)?
- Acceptable audio differential tolerance: exact sample equality against the oracle, or a
  documented per-sample/per-frame tolerance with hashes on the ChronoFork side?
- Should port 3 / analog / keyboard hardware be inside this milestone or recorded as H4
  residual limits?
- Does the native frontend ship in the same milestone or as the first follow-up after
  headless capture proves the pipeline? (Recommended: same milestone, last phase.)
