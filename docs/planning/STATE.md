# Session State — 2026-11-20

**Date:** 2026-11-20

## Current Position

- **Milestone:** 8 — Functional Mega Drive emulator (**active**; phase 8.4 complete, verified,
  reviewed and merged; integration record below).
- **Phase 8.4 — Z80, bus arbitration and audio: COMPLETE, VERIFIED and REVIEWED; merged into
  `main`.** Branch `agent/8.4-z80-audio`, based on `origin/main` `b8d63a1` (created from
  `ab3b484`; the T1–T6 block HEAD was `4fe418d`). Integration executed: PR #31 (`Closes #17`)
  merged into `main` at `dc5b31956a793c3d9d393e9d76a6ab73b86a9730`
  (`Merge pull request #31 from StealthC/agent/8.4-z80-audio`, 2026-11-20T21:57:53-03:00,
  issue #17 closed at 21:57:55); `origin/main` = `main` = that commit and the phase branch was
  deleted after the merge.
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
  (`./scripts/check.sh` exit 0 with every quality gate green; the pass/target totals were
  removed here and are **not** re-homed — this run's own record is the 8.4 branch's
  `VERIFICATION-T11.md`/`check-sh-t11.log` under the ignored `.research/8.4-r3-v2/`, named in
  the hygiene provenance note; the separate recovered W1 Step 0 log is linked from the Step 0
  entry below — CPU 11/11, VDP and
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
  `demo` 986 B / `9b4dd564…`, `gui run` 2842 B / `d3abf57a…`; `megadrive demo` was
  1994 B / `e5b9ecff…` at that commit tree and is now 1994 B / `64a95482…` (v7
  `state_hash` in its fork-isolation line — see the W1 entry below).
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

- **None for phase 8.4.** `./scripts/check.sh` and `git diff --check` exit 0 at the reviewed
  HEAD, and the reviewed work is integrated (`dc5b319`); push, PR and merge are done.
- **Boot/bus blocker (W1 prerequisite) — FIXED by W1.1–W1.3, in the
  `agent/w1-bus-boot-2026-11-21` worktree (uncommitted).** `TST.L $00A10008` and
  `TST.L $00A1000C` no longer take an address exception: the machine decodes 24
  address bits, work RAM is mirrored over `0xE00000..=0xFFFFFF`, and the port
  1/2/3/EXT control registers at `0xA10008..=0xA1000F` are a read/write stub in
  canonical state, so the 14-byte exception frame at `0xFFFFF7F0..0xFFFFF7FD` is
  written into the memory map instead of aborting the host. The open value is
  returned only inside the I/O window; the typed errors outside it, the
  step-scoped controller data registers and the Z80 arbitration remain unchanged.
  The controller/IO window follows ADR-0014 exactly: the port 1/2 control
  pairs latch, the port 3/EXT **control** pairs are the constant-zero stub
  (`0xA1000C..=0xA1000F`: reads return `0x00`, writes are discarded, so those
  lanes never change), port 3/EXT data stays a typed error, and the rest of the
  window is open bus. A debugger read or write that crosses `0xFFFFFF` is a
  typed rejection (never a wrap into ROM) and a debugger write overrun fails
  atomically.
  **W1.4 is done too:** the three 68000-memory DMA modes are rejected with
  `VdpError::DmaMemorySourceUnavailable { mode, words }` before any byte is read or
  written (the engine's source is a VRAM address, so the old path silently copied
  VRAM onto VRAM), while fill and copy keep working; a ROM that programs such a DMA
  now stops with a typed error, which is a known boot blocker for the G-BOOT-0/1
  gates, not a pass. The **W1.6 G-BOOT-0/1 gates and W1.7 re-verification are not
  done yet**. **Snapshot payload/state/config
  moved v6 → v7** (`CFMD68K7`/`CFMDSTATE7`/`CFMDCONF7`) because the control file
  is canonical state; v1–v6 are rejected with typed errors. Gate on this base:
  `./scripts/check.sh` exit 0 with every quality gate green and no target lost against
  the preceding baseline. The removed inline pass/target counts were **not** re-homed; their
  raw logs live only in ignored worktree evidence and are not reachable from this checkout, so
  the totals are recorded as unavailable in the hygiene provenance note rather than restated
  (anti-slop policy §7.2). Demos three runs each,
  byte-identical:
  `demo` 986 B / `9b4dd564…`, `gui run` 2842 B / `d3abf57a…` (both unchanged),
  `megadrive demo` 1994 B / `64a95482…` (was `e5b9ecff…`: its fork-isolation line
  prints the state hash, which the v7 bump moves; the 1994-byte length is
  unchanged).
- **Step 0 of W1 (gate on the base) — executed.** The full gate was re-executed on base
  `dc5b319` before any W1 edit; the raw log
  `.research/implementation-2026-11-20/step0-main-gate.log` lives in the named worktree
  `consolidation-adrs-2026-11-20`, had been missing through this checkout's `.research` root,
  and was recovered from that worktree. Its dated record — totals, SHA-256 and the preserved
  copy — is the
  [W1 Step 0 gate record](../quality/quality-gates.md#w1-step-0-gate-2026-11-20-recovered-historical-record)
  in `quality-gates.md`. This restates the
  recorded t11 PASS (`8500a4b`) as an execution on the W1 base, so W1 does not claim evidence it
  did not run.

## Recommended Next Step

Start **phase 8.5 — Timers, controllers and interrupts** (issue #18): timer interrupts, the
full TH latch and 3-/6-button protocol on ports 1–2, Z80 timer IRQ delivery, and interrupt
delivery integrated with cycle timing, per
[plan](../superpowers/plans/2026-11-17-8.5-timers-controllers.md). Do not recreate T1–T6.
