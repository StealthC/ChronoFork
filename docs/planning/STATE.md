# Session State — 2026-11-27

**Date:** 2026-11-27

## Current Position

- **Milestone:** M8 — Functional Mega Drive emulator (**active**; M8 stays open and its
  Definition of Done is not amended — owner decision O-2, 2026-11-20).
- **Phase 8.4 — Z80, bus arbitration and audio: complete, verified, reviewed and merged.**
  PR #31 (`Closes #17`) merged into `main` at `dc5b319`; payload/state/config v6.
- **W1 — boot/bus/DMA hardening: complete.** PR #33 (`994c3f2`) added the 24-bit bus
  choke point, RAM mirror, the `0xA10008–0xA1000F` control window, typed rejection of
  68000-memory DMA, the boot-gate harness and the re-verified 8.4 suite (the affected test
  goldens moved with the changed semantics). **G-BOOT-0 and
  G-BOOT-1 PASS at the W1 freeze** — both commercial fixtures complete the full step
  budget with no host error and every bootstrap marker holds; the result and its marker
  semantics are recorded in the committed
  [`boot_gate.rs`](../../crates/chronofork-megadrive/tests/boot_gate.rs) header, and the
  measured per-revision profile stays in the ignored `.research/w1-2026-11-21/` evidence.
  Follow-on landings: W1.8 payload bound (#38, `e6138b6`), VDP `0x0C` 8-bit VRAM read and
  payload v8 (#39, `a9fe4ae`, [ADR-0017](../adr/0017-vdp-read-cache-and-payload-v8.md)),
  hygiene/provenance (#40, `4993328`) and one global YM2612 selector with payload v9
  (#41, `1390841`, [ADR-0018](../adr/0018-ym2612-global-selector-and-payload-v9.md)).
  **There is no open W1.6/W1.7 work.**
- **W2 — execution/verification contract: W2.1/W2.2 INTEGRATED (PR #42, `cbba190`);
  W2.3a–c and W2.4a–b IMPLEMENTED AND PUBLISHED FOR EXTERNAL EVALUATION on the checkpoint
  branch `agent/w2-3-2026-11-27` (not merged); W2.5–W2.8 NOT started; W2 is not complete.**
  PR #42 merged into `main` at `cbba190` (merge commit; the owner
  merged it on 2026-11-27), and the post-merge CI run `46079267918` (`headSha cbba190`,
  workflow `CI`) succeeded. The branch `agent/consolidation-2026-11-27` carries the three
  commits `3da8ba4` (security), `9e56f65` (W2.1/W2.2) and `74ba70a` (docs/planning only); the
  reviewed product diff is
  `sha256 284c757550b4d1dfb678b4c3af5474f2b599f41a1963450359f066d6f8544735`. Independent
  verification (`t9`) and adversarial review (`t11`) **PASS** after the `t5`/`t6` failures
  were repaired by `t8`/`t10`. Durable summary:
  [docs/reviews/2026-11-27-w2-checkpoint/REPORT.md](../reviews/2026-11-27-w2-checkpoint/REPORT.md)
  (raw evidence in ignored `.research/consolidation-w2-2026-11-27/`).
  Plan W2.1 = design W2-1 (`Backend::state_hash` required with no default, `step_verified`
  routes through the single hash source, `StepOutcome` gains `state_hash: Option<_>` and
  `verified`); plan W2.2 = design W2-2 (`step` is digest-free: `state_hash = None`,
  `verified = false`; twin-machine invariant; digest consumers use the verified path).
  On top of that base, the checkpoint branch `agent/w2-3-2026-11-27` implements
  **W2.3a–c** (target `StepOutcome` shape, the single batch primitive/`run_until`/
  `run_cycles`, the typed stop grammar, lock-free `CancelToken`/`RunControl` cancellation
  and the absolute host deadline) and **W2.4a–b** (cheap `status()`/`inspection` split, and
  the server-side wire v2 with `performed`, `cycles`, `stop_reason`). The checkpoint is
  **published for external evaluation** (implementation commit `60e5207`, documentation
  commit on the same branch; PR open against `main`, **merge not authorized**), reviewed
  through the t18 verification and the t19→t25 review loop that ended with the **t25 final
  PASS**. It is **not merged and not yet externally evaluated**; **W2.5–W2.8 are not
  started** and begin only after that evaluation. W2 is not complete. Durable summary:
  [docs/reviews/2026-11-28-w2-3-4-checkpoint/REPORT.md](../reviews/2026-11-28-w2-3-4-checkpoint/REPORT.md).
  Governing ADR:
  [ADR-0015](../adr/0015-verification-contract.md). Detail:
  [plan §4 W2 and its execution ledger](../superpowers/plans/2026-11-20-consolidation-and-agent-api.md#12-execution-ledger-2026-11-27)
  and the [W2 design](../superpowers/specs/2026-11-20-w2-verification-migration-design.md).
- **W3 — service/concurrency/agent API: partial.** W3.1–W3.3 landed in PR #35
  (`a295834`): per-session owner, registry and the phased lock protocol; lock-free
  cancel/deadline plus expected revision and the bounded operation journal; quotas,
  `close`/`reap` semantics and the bounded closed-entry set. Executable coverage:
  `crates/chronofork-service/tests/w3_{concurrency,control,lifecycle}.rs`. **Evidence
  gap:** the W3.1 250 ms p95 `list`/`cancel` target is not yet measured by an executable
  test (barrier overlap, no-deadlock and atomic-admission tests exist; latency is not
  captured). W3.4 (composed API + limits table), W3.4a (frame capture) and W3.5 (live
  session save/load) are pending.
- **W4 — quality and reproducible evidence: partial.** PR #35 added committed synthetic
  fixtures with `fixture_integrity` (W4.1), the pinned differential with an explicit
  skip/exit matrix and `oracle_harness` (W4.2), the exception/`TRAP`/`RTE` synthetic ROM
  (W4.3), the fixture-required and cross-profile demo-golden guards (W4.5) and the CI
  layers in `.github/workflows/ci.yml` (W4.7). W4.8 is partial: dated gate evidence was
  separated and re-homed by #40. W4.4 (mutation control) and W4.6 (frozen KidEngine
  script) are pending.
- **W5 — backend and product consolidation: pending.** W5.1 is satisfied by the 8.4
  merge (`dc5b319`); W5.2–W5.6 (8.5 timers/controllers, audio verification, 8.6a
  compatibility, performance, 8.6c) are pending.
- **Deferred (not scheduled):** 8.7 execution modes/capture, 8.8 native window, M9, and
  the post-M8 MCP migration. M8 remains open for them.
- **Security posture:** the non-loopback bind refusal is **implemented** (owner decision
  2026-11-20; [ADR-0014](../adr/0014-v1-scope-single-user-loopback.md) §1/G-SEC). The CLI
  refuses at argument parsing, before the token file is read, the service is composed or a
  socket is bound, and `chronofork_transport::serve` refuses before `TcpListener::bind`;
  `crates/chronofork-cli/tests/serve_bind.rs` and
  `crates/chronofork-transport/tests/http.rs` prove it. This fix is outside the
  W2.1/W2.2 checkpoint. The dated M6 acceptance record
  (`docs/quality/m6-agent-interfaces-acceptance.md`) still describes the old
  accept-and-warn baseline; it is historical evidence superseded by ADR-0014's
  2026-11-27 implementation status.

## Open Decisions

- W2.3 and W2.4 are implemented and published for external evaluation on
  `agent/w2-3-2026-11-27` (PR open, not merged); W2.5–W2.8 are the next block only after
  that evaluation, and W2 is not complete until they land. Do not describe the checkpoint
  work as merged.
- The W3.1 p95 latency target needs a measured test on a calibrated runner before W3.1
  can be called closed; the scale and no-deadlock behaviour is already covered.
- Phase 8.5 must have its plan's port-3 rule corrected before execution (plan W5.2
  carry-over from the W1 review).

## Blockers

- **None.** W2.1/W2.2 are merged at `cbba190` with the post-merge CI run green. The
  W2.3a–c/W2.4a–b checkpoint is published on `agent/w2-3-2026-11-27` (implementation
  commit `60e5207`; PR open, merge not authorized) and awaits the owner's external
  evaluation; no W2.5+ implementation has started.
- **Tracked, not blocking:** W3.4/W3.4a/W3.5 and W5 remain pending as recorded below;
  W3.1's p95 measurement waits on a calibrated runner.

## Evidence

- Durable W2 checkpoint summaries:
  [docs/reviews/2026-11-27-w2-checkpoint/REPORT.md](../reviews/2026-11-27-w2-checkpoint/REPORT.md)
  (W2.1/W2.2, merged at `cbba190`) and
  [docs/reviews/2026-11-28-w2-3-4-checkpoint/REPORT.md](../reviews/2026-11-28-w2-3-4-checkpoint/REPORT.md)
  (W2.3/W2.4, published for external evaluation); the full checkpoint reports and raw logs
  stay in the ignored `.research/consolidation-w2-2026-11-27/` evidence tree.
- Quality gates and their historical records:
  [quality-gates.md](../quality/quality-gates.md); behaviour-preservation evidence:
  [anti-slop refactor evidence](../quality/anti-slop-refactor-evidence.md).
- Execution ledger (W0–W5 status, PR/commit/evidence, next step) and the W2 landing-order
  reconciliation:
  [consolidation plan §12](../superpowers/plans/2026-11-20-consolidation-and-agent-api.md#12-execution-ledger-2026-11-27).
- Accepted decisions: ADRs
  [0014](../adr/0014-v1-scope-single-user-loopback.md),
  [0015](../adr/0015-verification-contract.md),
  [0016](../adr/0016-session-service-and-agent-api.md),
  [0017](../adr/0017-vdp-read-cache-and-payload-v8.md),
  [0018](../adr/0018-ym2612-global-selector-and-payload-v9.md).
- Capability scope and integration evidence stay in the
  [canonical milestone page](../roadmap/milestones.md); this handoff does not restate
  volatile test counts.

## Recommended Next Step

Submit the published W2.3a–c/W2.4a–b checkpoint for **external evaluation** (branch
`agent/w2-3-2026-11-27`, base `cbba190`, implementation commit `60e5207`; the checkpoint PR
is open against `main` and is **not merged**), then start **W2.5–W2.8** only after that
evaluation approves the checkpoint. W2 is **not** complete and no merge may be claimed. Do not start 8.5. In parallel, close the W3.1 p95
evidence gap and continue with W3.4 → W3.4a/W3.5. Keep 8.7/8.8/M9 deferred and M8 open,
per O-2.
