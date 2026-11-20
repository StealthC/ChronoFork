# Session State — 2026-11-20

**Date:** 2026-11-20

## Current Position

- **Milestone:** 8 — Functional Mega Drive emulator (**active**; phase 8.4 complete,
  verification/review/integration pending).
- **Phase 8.4 — Z80, bus arbitration and audio: COMPLETE** at the implementation and
  documentation level. Work is preserved as local commits on `agent/8.4-z80-audio`, based on
  `origin/main` `b8d63a1` after the phase merge of that commit (the branch was created from
  `origin/main` at `ab3b484`; the T4–T6 block HEAD was `4fe418d`): the T1–T6 phase commits,
  the `origin/main` merge and this documentation-closure commit sit on top of `b8d63a1`.
  **Nothing was pushed, no PR was opened, no merge to `main` happened**, and issue #17 remains
  open. This session's team (`chronofork-8-4-z80-audio-r3`) owns the closure.
- **Completed in this session (local commits):**
  - **T1–T6 (previous team, preserved commits):** Z80 core, Z80 interrupts/memory
    map/arbitration, YM2612, PSG + mixer/resampler, `MediaSource` audio + payload v6 + Z80
    advanced from `Backend::step`, and the audio-state oracle/evidence/capability docs. See the
    ignored `.agents/notes/8.4/handoff.md` for the per-task detail and honest limits.
  - **Merge of `origin/main`:** `b8d63a1` (`docs(agents): add engineering responsibility and
    technical judgment`) merged into the branch with the new `AGENTS.md` section preserved.
  - **Documentation closure:** `docs/planning/ROADMAP.md` (8.4 `[status: complete]`,
    `Completed: 2026-11-20`), `docs/planning/MILESTONE.md`, `AGENTS.md`, `docs/README.md`,
    `docs/roadmap/milestones.md` (M8 integration state; H3 `Planned` → `Partial`),
    `docs/requirements/functional.md` (FR-MD-008, FR-MD-011/012 status),
    `docs/architecture/megadrive-slice.md` (payload/state/config v6, Z80/audio section),
    `docs/quality/quality-gates.md` (phase 8.4 gate section) and the 15-task
    `docs/superpowers/plans/2026-11-17-8.4-z80-audio.md` checklist (75/75 checked).
  - **Repair round (t5, `8.4 R3`):** an independent review of the closure raised five
    findings and all five were repaired — YM2612 `0x28` key-on channel derivation (R1-F1),
    PSG noise shift periods to 32/64/128 internal ticks and the tone-2-clocked mode (R1-F2),
    `0xA11100` hardware `ZBR` request polarity per the owner's 2026-11-20 decision (R1-F3),
    `execution-model.md` `CFMDSTATE6` coverage (R1-F4) and the phase-8.4 inspection-surface
    record (R1-F5). R1-F2/R1-F3 are canonical-state behaviour fixes, so state hashes moved
    while payload lengths and the configuration hash did not; the affected goldens were
    re-pinned mechanically with the cause recorded (`backend.rs` golden hashes and six
    `chronofork-cli --test debug` `state_hash` expectations, RAM/VRAM dumps byte-identical)
    and four regression tests were added. The recorded internal PCM fixture re-probed
    unchanged. Demo baselines at the repair HEAD (three runs each, byte-identical, empty
    stderr): `megadrive demo` 1994 stdout bytes /
    `e5b9ecffacb2364c889b2ef969c5c2ee86e4da892c4c0e9b22baa8163ca99609` (the closure-head
    value at `8382255` was `794252b3…`, same length, moved by R1-F2/R1-F3), while `demo`
    (986 B, `9b4dd564…`) and `gui run` (2842 B, `d3abf57a…`) are unchanged. The phase-8.4
    differential harness was realigned to the repaired polarity (same bus-request byte on
    both sides) and re-run: exit 0, PASS on all 25 fields, and the request-asserted probe
    now agrees at Z80 `PC = 4` on both emulators instead of diverging.
- **Honest limits carried by phase 8.4:** Z80 `IXH`/`IXL`/`IYH`/`IYL` and `WZ`/`MEMPTR` are
  not modelled and undocumented `ED` opcodes are `NOP`s; bus arbitration follows the hardware
  `ZBR` request polarity with a functional fixed-stall model (60 master cycles per byte lane);
  `SSG-EG`, channel-3 special mode, CSM and the `0x21`/`0x2B` test bits are stored but not
  applied; the YM status busy bit is absent; envelope/amplitude/PSG attenuation/LFO are
  deterministic integer approximations (the PSG noise shift periods are the documented
  `N/512`, `N/1024` and `N/2048`, i.e. 32/64/128 internal ticks, ≈6991 Hz for selector 0,
  and the tone-2-clocked mode shifts once per full tone-2 period);
  timer IRQ delivery to the Z80 is phase 8.5; no
  cross-emulator audio **sample** equality is claimed, and audio capture to a file or device
  (WAV/streaming) is phase 8.7.
- **Evidence:** raw artifacts under the ignored `.research/8.4/` and
  `.research/evidence/8.4/`; the device-state differential (25 fields, pinned oracle
  `ec005615`, three negative controls) is summarized in
  [docs/research/audio-state-differential-evidence.md](../research/audio-state-differential-evidence.md).

## Open Decisions

- Intermediate per-plan commits are a stack (only each block HEAD builds in isolation);
  accepted for this phase and noted for review.
- Audio differential evidence is device-state only, never a sample comparison; the pinned
  oracle publishes no YM2612 register/channel page, so only the readable YM status byte is
  compared.
- The owner conditionally authorized push/PR/merge only after independent verification (V1)
  **and** independent review (R1) pass on the closure HEAD.

## Blockers

- None. No gate is red; `./scripts/check.sh` and `git diff --check` exit 0 at the closure HEAD.

## Recommended Next Step

1. Finish the phase-8.4 closure on `agent/8.4-z80-audio`: the review round 1 (R1) returned
   `needs_revision` with findings R1-F1–R1-F5, all repaired by t5; re-run independent
   verification (V1) and the review (R1 round 2) on the repaired HEAD, then captain
   integration (I1: push, PR with `Closes #17`, CI green, merge into `main`, cleanup).
2. Then start **phase 8.5 — Timers, controllers and interrupts** (issue #18): timer
   interrupts, the full TH latch and 3-/6-button protocol on ports 1–2, Z80 timer IRQ
   delivery, and interrupt delivery integrated with cycle timing, per
   [plan](../superpowers/plans/2026-11-17-8.5-timers-controllers.md).

Do not recreate T1–T6 and do not push or merge until V1 and R1 pass.
