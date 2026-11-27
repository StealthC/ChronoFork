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
- **W2 — execution/verification contract: W2.1/W2.2 IMPLEMENTED, awaiting external
  checkpoint evaluation; W2.3+ NOT started; W2 is not complete.**
  Branch `agent/consolidation-2026-11-27` at base `1390841`; the reviewed checkpoint is
  published on that branch for review (PR against `main`, merge not authorized). Plan W2.1 =
  design W2-1 (`Backend::state_hash` required with no default, `step_verified` routes through
  the single hash source, `StepOutcome` gains `state_hash: Option<_>` and `verified`); plan
  W2.2 = design W2-2 (`step` is digest-free: `state_hash = None`, `verified = false`;
  twin-machine invariant; digest consumers use the verified path). Independent verification
  (`t9`, second round) and adversarial review (`t11`) **PASS**; the self-sufficient checkpoint
  report is `.research/consolidation-w2-2026-11-27/CHECKPOINT.md` (ignored evidence).
  Design W2-3 (field removal / `run_batch`) and W2-4a (`MachineStatus`/`status` + service JSON
  v2) are **not started** and wait for the owner's evaluation of this checkpoint. Governing
  ADR: [ADR-0015](../adr/0015-verification-contract.md). Detail:
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

- W2.3+ (StepOutcome field removal / `run_batch`, `MachineStatus`/`status`, service JSON
  v2) and W2-4a must not start before the owner evaluates the W2.1/W2.2 checkpoint.
- The W3.1 p95 latency target needs a measured test on a calibrated runner before W3.1
  can be called closed; the scale and no-deadlock behaviour is already covered.
- Phase 8.5 must have its plan's port-3 rule corrected before execution (plan W5.2
  carry-over from the W1 review).

## Blockers

- **None for this checkpoint.** The work on `agent/consolidation-2026-11-27` is the
  W2.1/W2.2 implementation plus this lifecycle reconciliation; it is not merged.
- **Tracked, not blocking:** W3.4/W3.4a/W3.5 wait on the W3.4 service-surface decision;
  W3.1's p95 measurement waits on a calibrated runner.

## Evidence

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

Evaluate the **W2.1/W2.2 checkpoint** (published for review on
`agent/consolidation-2026-11-27`; report in
`.research/consolidation-w2-2026-11-27/CHECKPOINT.md`) before any W2.3+/W2-4a work; do not
start 8.5 or claim W2 complete. W2.3+ must not start before that evaluation. In parallel,
close the W3.1 p95 evidence gap and continue with W3.4 → W3.4a/W3.5. Keep 8.7/8.8/M9
deferred and M8 open, per O-2.
