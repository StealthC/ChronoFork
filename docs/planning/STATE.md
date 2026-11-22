# Session State — 2026-11-20

**Date:** 2026-11-20

## Current Position

- **Milestone:** 8 — Functional Mega Drive emulator (**active**; phase 8.4 complete,
  independently verified and reviewed; integration in progress).
- **Phase 8.4 — Z80, bus arbitration and audio: COMPLETE, VERIFIED and REVIEWED.** Branch
  `agent/8.4-z80-audio`, based on `origin/main` `b8d63a1` (created from `ab3b484`; the T1–T6
  block HEAD was `4fe418d`). The captain is opening the phase PR (`Closes #17`); the PR is the
  integration record for the merge into `main`. If a fresh session reads this before the merge
  lands, check the PR/issue #17 state first.
- **Delivered:** the complete documented Z80 core (main/CB/ED/DD-FD tables, flags, DAA, block
  ops, IM 0/1/2, NMI, one-instruction `EI` delay) with the Z80 memory map, bank register and
  deterministic 68000/Z80 bus arbitration following the hardware `ZBR` request polarity;
  YM2612 six-channel FM (registers/timers/status, envelopes, operators/algorithms/LFO,
  channel-6 DAC); SN76489 PSG; an integer-only mixer and exact rational 48 kHz resampler; audio
  exposed through `MediaSource`; the Z80 advanced from `Backend::step` with a serialized
  master-cycle carry; payload/state/config v6 (`CFMD68K6`/`CFMDSTATE6`/`CFMDCONF6`) carrying
  the Z80, arbitration, YM, PSG and mixer state with typed rejection of v1–v5; and
  `z80.*`/`arbitration.*`/`ym.*`/`psg.*`/`audio.*` inspection pseudo-registers.
- **Verification and review:** independent verification (`t11`) **PASS** at `8500a4b`
  (`./scripts/check.sh` exit 0, 1454 passed / 0 failed / 122 targets, CPU 11/11, VDP and
  audio-state oracle green, demos byte-identical, payload v1–v5 rejected, no weakened tests);
  independent review round 4 (`t13`) **PASS** at `68d367b` (`crates/` byte-identical to
  `06f5e95`, harness PASS 25/25 with the request-asserted probe agreeing, R3-F1/F2/F3
  resolved, no findings). The review gate for phase 8.4 is closed.
- **Repair history (all closed):** R1-F1–R1-F5 (YM `0x28` channel derivation, PSG noise
  periods, hardware `ZBR` polarity per the owner's 2026-11-20 decision, `execution-model.md`
  v6 coverage, inspection-surface record); R2-F1–R2-F6 and R2V-F2 (residual claims, demo
  baseline, harness realignment, overview/EPIC scope, evidence-page goldens); R3-F1–R3-F3
  (E2-11 `Done` in EPIC-0002, quality-gates HEAD record, `z80_pc_agrees` gated in the
  harness). The canonical-state fixes (R1-F2/R1-F3) moved state hashes while payload lengths
  and the configuration hash stayed fixed; the affected goldens were re-pinned with the cause
  recorded and four regression tests added. The recorded internal PCM fixture re-probed
  unchanged. Demos at the repaired tree (three runs each, byte-identical, empty stderr):
  `demo` 986 B / `9b4dd564…`, `megadrive demo` 1994 B / `e5b9ecff…`, `gui run` 2842 B /
  `d3abf57a…`.
- **Honest limits carried by phase 8.4:** Z80 `IXH`/`IXL`/`IYH`/`IYL` and `WZ`/`MEMPTR` are
  not modelled and undocumented `ED` opcodes are `NOP`s; bus arbitration follows the hardware
  `ZBR` request polarity but the Z80 yield is functional (on the request write, not at an
  instruction boundary) with a fixed 60-master-cycle stall per byte lane; `SSG-EG`, channel-3
  special mode, CSM and the `0x21`/`0x2B` test bits are stored but not applied; the YM status
  busy bit is absent; envelope/amplitude/PSG attenuation/LFO are deterministic integer
  approximations; PSG noise shift periods are `N/512`, `N/1024` and `N/2048` (32/64/128
  internal ticks, ≈6991 Hz for selector 0) and the tone-2-clocked mode shifts once per full
  tone-2 period; timer IRQ delivery to the Z80 is phase 8.5; no cross-emulator audio
  **sample** equality is claimed and audio capture to a file or device (WAV/streaming) is
  phase 8.7.
- **Evidence:** raw artifacts under the ignored `.research/8.4/`, `.research/8.4-r2/`,
  `.research/8.4-r3-v2/`, `.research/8.4-t10/` and `.research/evidence/8.4*`; the device-state
  differential (25 fields, pinned oracle `ec005615`, three negative controls) is summarized in
  [docs/research/audio-state-differential-evidence.md](../research/audio-state-differential-evidence.md).

## Open Decisions

- Intermediate per-plan commits are a stack (only each block HEAD builds in isolation);
  accepted for this phase.
- Audio differential evidence is device-state only, never a sample comparison; the pinned
  oracle publishes no YM2612 register/channel page, so only the readable YM status byte is
  compared.
- The automatic three-round review loop escalated; the owner explicitly authorized one extra
  review round (round 4), which passed. The escalation is resolved by that authorization.

## Blockers

- None. `./scripts/check.sh` and `git diff --check` exit 0 at the reviewed HEAD; the PR, CI
  and merge are the remaining integration steps.

## Recommended Next Step

Start **phase 8.5 — Timers, controllers and interrupts** (issue #18): timer interrupts, the
full TH latch and 3-/6-button protocol on ports 1–2, Z80 timer IRQ delivery, and interrupt
delivery integrated with cycle timing, per
[plan](../superpowers/plans/2026-11-17-8.5-timers-controllers.md). Do not recreate T1–T6.
