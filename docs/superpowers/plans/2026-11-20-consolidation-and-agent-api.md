# Consolidation and agent API — implementation plan (owner-reviewed draft)

- **Status:** Owner-reviewed plan, revised after the V2 review (`review-corrected-adrs.md`)
  and the V3 review (`review-corrected-adrs-v3.md`). The owner's decisions of 2026-11-20 are recorded and the
  reviewer/critic corrections (R1–R5, C1, V2-1…V2-5) are applied;
  `CORRECTIONS-APPLIED.md` and `CORRECTIONS-APPLIED-V2.md` carry the mapping. **No production
  code is changed by this document.** Acceptance of the ADRs is pending a further independent
  review (V3) of the corrected tree and the owner's act.
- **Date:** 2026-11-20.
- **Baseline:** `origin/main` `b8d63a17349e672ba6945eaba3379cec38cf9d1b`.
- **Related ADRs:** [ADR-0014](../../adr/0014-v1-scope-single-user-loopback.md) (v1 scope),
  [ADR-0015](../../adr/0015-verification-contract.md) (execution/verification),
  [ADR-0016](../../adr/0016-session-service-and-agent-api.md) (service/agent API).
- **Review evidence:** the independent review is durable in this tree at
  [`docs/reviews/2026-11-20-independent-review/REVIEW.md`](../../reviews/2026-11-20-independent-review/REVIEW.md)
  (with `areas/`, `critique-final.md`, `SYNTHESIS-DRAFT.md`, `baseline.md`, `benchmarks/`).
  Consolidation reviews: `.research/consolidation-2026-11-20/reviews/` (R1–R5),
  `.research/consolidation-2026-11-20/critique-consolidation.md` (C1) and
  `.research/consolidation-2026-11-20/review-corrected-adrs.md` (V2).
- **In-progress work that must be preserved:** `agent/8.4-z80-audio` (Z80/bus/audio, 14
  implementation commits plus uncommitted T4 work; branch tip `408cfd7`). This plan does not
  modify, build, rebase or merge that branch.

## 0. Owner decisions recorded (2026-11-20)

| # | Decision | Consequence in this plan |
|---|---|---|
| O-1 | **Non-loopback bind refuses to start** with a typed error and a test; `--unsafe-bind`/exposed deployment needs its own ADR | G-SEC asserts the refusal; ADR-0014 §1/§Verification |
| O-2 | **M8 stays open. Consolidation (8.6c) is a separate phase of the open M8; the M8 DoD is not amended.** 8.7/8.8 are executed afterwards | §5 sequencing; W5.6 does not close M8; G-DET stays a W2 requirement even though the capture surface is later |
| O-3 | **Finish the project's own YM2612/PSG/mixer from 8.4; do not adopt `ymfm` now.** A future synthesizer swap needs its own ADR and does not discard the Z80/bus work | W5.1/W5.3; §8 approvals |
| O-4 | **Unmapped I/O = deterministic open bus + typed error only where the hardware faults**, mapping `0xA10008–0xA1000F` first (port 3/EXT control = stub 0; port 3/EXT data = typed error) | W1.3; ADR-0014 §I/O |

**Owner decision O-5 (2026-11-21, W3g/R1d D2): the Mega Drive checkpoint payload bound is
deferred to W1.** The service layer decides `max_bytes_per_session` **before** a checkpoint is
captured, which requires a payload size it can bound; after `session.run` (or a trace/debug
write) that bound must come from `Backend::snapshot_payload_bound()`. The Mega Drive backend
does not declare one yet, so under a **finite** byte quota a post-run `session.snapshot` /
`session.fork` is refused with `limit_exceeded` **before allocating** (the unbounded in-process
paths — movie/player/rewind — are unaffected). That is a temporary, tracked limitation, not the
intended product behavior: **W1.8** declares the bound in the backend and adds the
service+Mega Drive regression. W3g must not edit `chronofork-megadrive` or the 8.4 work.

## 1. Goal

Turn the review's approved direction into small, verifiable changes that:

1. make at least one real ROM progress past the documented bootstrap marker and expose the
   next real blocker;
2. remove the per-step full-state verification from ordinary execution without losing
   determinism, TAS verification or first-divergence localization;
3. allow independent sessions to run without a global lock, with cancellation, deadlines and
   quotas that are actually enforced;
4. expose an agent-usable, batched reverse-engineering surface with live-session persistence;
5. make the quality evidence reproducible from a clean clone;
6. keep the Rust architecture, snapshot versioning, forks and deterministic execution.

## 2. Non-goals

- Rewriting the emulator, the session layer or the transport in async Rust.
- Multi-user, TLS, non-loopback deployment or durable principals in v1.
- **Amending the M8 Definition of Done to close a phase**: M8 stays open and 8.7/8.8 are
  executed; consolidation is a phase, not a milestone closure (O-2).
- **Adopting `ymfm` now** (O-3); the project's own audio core is completed first.
- 8.7 (capture modes) and 8.8 (native window) before the consolidation gates pass; M9 before
  M8 closes.
- Commercial ROMs, BIOS/firmware or GPL/non-commercial cores in the repository.
- Hash incremental/COW/dedup before the simple fixes are measured.
- Changing roadmap phases or closing milestones without owner approval.

## 3. Constraints

- One write task = one branch/worktree; the 8.4 checkout is owned by another session.
- Every behavior-bearing change follows the anti-slop workflow: baseline first, gates, no
  weakened tests, evidence.
- Snapshot/backend payloads are versioned; never reinterpret bytes.
- No `unsafe`; no new dependency without an ADR/license review.
- Emulated time is canonical; host time is only pacing/deadline.
- A capability is advertised only when tests back it.
- **An independent review of the corrected tree (V2) must pass before an ADR is marked
  `Accepted`, and an independent review of each behavior-bearing diff before its phase closes.
  Applying corrections is not acceptance.**

## 4. Workstreams and tasks

> Task IDs are planning identifiers, not AgentTeams task ids. Each task becomes an Issue/PR
> with objective, scope, acceptance, verification and independent review. The owner approves
> the ADRs, the scope reductions and the roadmap; the team owns implementation details.
> **Every task below names files/crates, a named test and a failure condition.**

### W0 — ADRs and governance (no code)

| ID | Task | Files | Acceptance | Verification | Depends on |
|---|---|---|---|---|---|
| W0.1 | Corrections applied to ADR-0014/15/16 and this plan (done: S2). Independent review of the corrected tree | `docs/adr/0014*`, `0015*`, `0016*`, `docs/adr/README.md`, this plan | V2 review returns PASS with every mandatory finding closed or explicitly deferred by the owner | Review report mapped finding→clause | S2 diff |
| W0.2 | Record O-1…O-4 and D-series decisions in the owning docs | ADR-0014 §1/§I/O, this plan §0, `docs/requirements/` where scope changes | Each decision traceable with date and effect; no contradiction with ADR-0014/16 | Reading + drift guard | V2 |
| W0.3 | Source-of-truth cleanup (done: t6): one status home, historical banners, `STATE.md`, drift guard | docs | Guard passes; no contradictory status | `check_docs_drift` (exit 0 fixed / 1 original) | — |
| W0.4 | Promote the review report as durable evidence — **done**: the report is already in the tree at `docs/reviews/2026-11-20-independent-review/REVIEW.md`; remaining work is only to keep the ADR citations pointing at it | `docs/reviews/2026-11-20-independent-review/REVIEW.md` (exists) | The paths cited by the ADRs resolve; no "does not exist" wording remains | Link check + drift guard | — |
| W0.5 | Update `docs/planning/STATE.md` with the unmerged 8.4 handoff (commit, owner, authorization, gates) | `docs/planning/STATE.md` | §5 below is recorded; a fresh session can resume safely | Reading | W0.1 |

### W1 — Boot/bus hardening (behavior-bearing; starts only after 8.4 is **integrated**)

**Prerequisite (executed, not just decided):** W5.1 must be complete — the 8.4 handoff commit
integrated and verified on the authorized base (H-1/H-2/H-3 below), with the 8.4 plan's gate
green. Editing `bus.rs`/`backend.rs` on the b8d63a1 baseline without the 8.4 base would
contradict the "preserve and land 8.4 first" requirement. If the owner chooses a different
path (a frozen handoff commit + sequential patches instead of an integrated merge), that
choice is recorded in H-3 and every W1 row depends on the recorded base commit.

Files: `crates/chronofork-megadrive/src/{bus.rs,backend.rs,cartridge.rs}`, docs, tests.

| ID | Task | Acceptance | Named verification | Depends on |
|---|---|---|---|---|
| W1.1 | 24-bit address normalization in one bus choke point (`read8/16/32`, `write8/16/32`, `peek`, `write_memory`, exception frame; **not** the A7 register value) | `0xFFFFF7FE` maps to RAM; unit tests per path | `cargo test -p chronofork-megadrive`; `address_normalization` unit test fails on the old behavior | **W5.1 executed + verified** (H-1/H-2/H-3) |
| W1.2 | RAM mirror `0xE00000..0xFFFFFF` and documented aliases | Reads/writes mirror work RAM; no silent unmapped read | `ram_mirror` test | W1.1 |
| W1.3 | I/O controls `0xA10008–0xA1000F` (O-4): control 1/2/3/EXT readable/writable reset 0; port 3/EXT **data** typed error; deterministic open bus elsewhere | `TST.L $00A10008` returns a defined value; port 3 control reads 0 | `controller_bus` and `io_window` tests; the *Chakan* first-instruction case does not fault | O-4; W1.1 |
| W1.4 | DMA `memory→VRAM`: typed rejection until the 68000 side exists | No silent VRAM→VRAM; existing tests updated | `dma_rejects_memory_to_vram`; snapshot version note | — |
| W1.5 | Correct false technical statements in `docs/architecture/megadrive-slice.md` and related docs | Docs match code (A2-F11 closed) | `check_docs_drift`; manual read | — |
| W1.6 | Boot gates G-BOOT-0 and G-BOOT-1 | **G-BOOT-0:** 300k steps without a host error. **G-BOOT-1:** the fixture reaches its documented bootstrap marker (PC of init, expected VDP transitions, leaving the fatal handler) — **finding a blocker is a gate failure, and the investigation is the deliverable**; `BRA *` cannot pass | Boot harness per fixture with recorded markers; negative control (`BRA *` case must fail G-BOOT-1) | W1.1–W1.3 |
| W1.7 | Re-verify 8.4 after hardening; regenerate goldens only where semantics changed | Full gate green; diffs explained; no weakened tests | `./scripts/check.sh`; golden diff review | 8.4 integration |
| W1.8 | Declare `Backend::snapshot_payload_bound` on the Mega Drive backend (the documented payload size, validated against every supported payload) so a quota-checked capture can be decided before allocating (O-5, carry-over from W3g/D2) | A service+Mega Drive regression passes under a finite `max_bytes_per_session`: create → snapshot → run(1) → `snapshot`/`fork`/`restore` usable; a payload that genuinely cannot fit is still refused **before** the capture | `cargo test -p chronofork-service` (service+Mega Drive test) and `cargo test -p chronofork-megadrive`; the existing quota tests unchanged | W1 base (8.4 integrated); W3g/D2 carry-over |

### W2 — Execution/verification contract (ADR-0015)

| ID | Task | Acceptance | Named verification | Depends on |
|---|---|---|---|---|
| W2.1 | `Backend::state_hash()` explicit + `step_verified` calling the **single** hash source | Verified path reproduces current hashes; `step` unchanged during migration | `state_hash_matches_verified` invariant across deterministic tests (R1-C1) | ADR-0015 accepted |
| W2.2 | `StepOutcome` → `{tick, halted}`; migrate the 121 `.inspect()` and 93 `state_hash` reads with a declared wire version | Compiles; consumers migrated; service JSON/GUI decoders versioned | `cargo check`/`test`; census script for `.inspect()`/`state_hash` | W2.1 |
| W2.3 | `run_until`/`run_cycles` in `Session`; `Backend` exposes `step`/`step_verified` + at most one batch primitive; `run_frame` as a capability; lock-free cancel token; stop-condition grammar and precedence | Deterministic batch; deadline chooses `k`; `performed` + stop reason on cancel/deadline; replay records `k`+inputs; cancelled run == deterministic prefix | `cancel_equals_run_of_k`, `stop_condition_precedence`, `deadline_prefix_is_deterministic` | ADR-0015; R1-C2/C3/C10; R5-K2/K3 |
| W2.4 | `status()` cheap + register map; `MachineStatus`/per-device views; `list` stops inspecting every session | Trace/debug/service do not hash or copy 64 KiB per step; debugger receives the register map | `status_has_no_hash_or_copy`; `list_does_not_inspect`; latency measured | W2.1; R1-C4/C5; R5-K8 |
| W2.5 | Persistent buffers; `Arc` payload/initial_state (byte equality preserved); `reap()`; no COW | Allocations/step and fork memory measured; payload format unchanged | Bench before/after; `payload_bytes_equal` test | W2.1 |
| W2.6 | Render: preserve the frame index/window; skip only if the frame is reconstructible from boundary state | Frames byte-identical with and without a consumer; no future-state render | `frames_identical_without_consumer`, `frame_reconstructed_from_boundary` | W2.4 |
| W2.7 | Measure with the pinned workload and G1–G6 protocol (identified harness, interleaved A/B, p50+p95, minflt, allocator mode, verified path measured) | Methodology recorded; **no fps goal stated**; blocking only on a calibrated runner | `bench_step` + dated report; G-PERF gate | W2.1–W2.5 (W2.6 optional here) |
| W2.8 | Versioning: movie policy metadata; snapshot compatibility | Old movies replay via the verified path; new movies declare policy | Replay tests v1/v2 + goldens | W2.1 |

### W3 — Service/concurrency/agent API (ADR-0016)

| ID | Task | Acceptance | Named verification | Depends on |
|---|---|---|---|---|
| W3.1 | Per-session owner + registry with the phase-1/2/3 lock protocol (release the registry before waiting for an owner), atomic admission (global `N − 1`, per principal 1), and `list` returning owner-free metadata with the declared freshness contract (`running` marker) | Scale gate runs with **`workers = 5`** (four principals, `max_long_runs_global = 4`) so 4 sessions overlap by barrier and scale >2× on a calibrated runner; no deadlock; admission atomic; `list` bound expressed in `workers` and never waits on a running session; the **250 ms p95 acceptance target** for `list`/`cancel` under load is **measured by the test**, not asserted | `lock_order_no_deadlock`, `max_sessions_atomic_admission`, `list_metadata_no_owner`, `list_within_worker_bound` | ADR-0016 §1; **W2.4 or an equivalent metadata field cached at each commit**; R5-K1/K5/K6/K8; V2-2; V4-2 |
| W3.2 | Lock-free cancel + deadline + `expected_revision`/`expected_tick` + pre-known request key + bounded operation journal (never evicts active) | Cancel from another worker in ms; reconcile without double-apply; ABA/two-writes-at-one-tick/full-journal covered | `cancel_is_lock_free`, `reconcile_no_double_apply`, `journal_never_evicts_active`, `aba_restore_retry` | W3.1; R5-K2/K3/K4; R2-F3/F5 |
| W3.3 | Quotas with numeric defaults (session accounting is the source), atomic session admission, synced global byte counter or declared best-effort; `reap`/`close` semantics versioned; closed-entry bound (`max_closed_retained`=64 / `closed_retention_ttl`=300 s) | Limits reject before allocation; **`close` releases the backend/snapshot state immediately**, so repeated `create→close` does not accumulate; live count meaningful; `reap` with an in-flight reader safe; closed entries bounded independently of the live count | `quota_rejects_before_alloc`, `concurrent_quota`, `close_frees_state`, `closed_entries_bounded`, `reap_with_reader` | W3.1; R5-K7/K9; V3-2 |
| W3.4 | Minimal composed API **with the limits table**, crate boundary decided (sessions facade + injected provider), `capabilities` as single discovery source | Each operation declares input/limits/output/auth/cancelability; `memory_write` coherent; admin/hardware boundary preserved | `limits_reject_before_alloc` per group; `capabilities_single_source` | W3.1; ADR-0016 §4/§New limits; R2-F1/F2/F6 |
| W3.4a | **Frame capture as an explicit task:** format decided, response budget ≤128 KiB (or declared alternative), pagination/tiles, `frame_hash` always present, reconstruction test | Format + budget documented; a real 320×224 frame round-trips within budget | `frame_pagination_roundtrip`, `frame_hash_matches_bytes`, size test | W3.4; R2 correction 12; critique C1 §6 |
| W3.5 | Live branched `session.save`/`load` with a decided envelope | Round-trips tick/hash/state live and branched | `live_session_roundtrip`; version rejection test | W3.4 |
| W3.6 | Transport: `Origin` allowlist + 403, absolute pre-auth deadline, per-peer pre-auth cap, accept policy, worker-bound documentation, admission limit for long runs, and the **client-side v2 probe/refusal** (read-only `capabilities`, no sentinel mutation) | Slowloris cannot exhaust; `/health` responds; saturation test with the **default `workers = 4`** shows **three admitted + the fourth attempt rejected** + list/cancel executable, with the 250 ms p95 target measured; legacy-daemon test proves the client refuses v2 locally without dispatching the sentinel | `origin_allowlist_rejects`, `slowloris_bounded`, `saturation_documented`, `client_refuses_v2_on_legacy` | W3.1; R2-F4; critique C1 §5; V4-1/V4-2 |
| W3.7 | Docs/limits/capabilities honest; remove "reads remain available"; record supersession of ADR-0012 §5 | No unsupported promise remains | `check_docs_drift`; manual review | W3.1–W3.6; R5-K11 |

### W4 — Quality and reproducible evidence

**Independence boundary (V2-4):** only **W4.1–W4.5 and W4.8** are independent of 8.4 and of the
platform work and may start now. **W4.6 depends on W3.4/W3.5** (the agent surface must exist) and
**W4.7 depends on W4.1–W4.5**; neither is "independent of 8.4" in the scheduling sense and both
are sequenced in the DAG below.

| ID | Task | Acceptance | Named verification | Depends on |
|---|---|---|---|---|
| W4.1 | Commit the three synthetic legacy ROMs + a Rust fixture test that re-runs the generator | Clean clone validates fixtures; no commercial ROMs | `fixture_integrity` test on a clean clone | — (independent) |
| W4.2 | Differential oracle: vendored/pinned image **or** explicit skip with a count guard | Clean clone runs the differential or fails explicitly; CI never green by absence | Exit-code matrix; `zero-executed = failure` guard | — (independent) |
| W4.3 | New synthetic ROM for exceptions/`TRAP`/`RTE`/A7/frame; compare final state | The differential covers the highest-risk class | Harness + recorded evidence | W4.2 |
| W4.4 | Mutation control for the emulator (+1 cycle or inverted flag) | A deliberate regression makes the differential fail | Run the control; expected failure | W4.2 |
| W4.5 | Remove silent skips; "zero executed tests = failure" guard; demo golden cross-profile (167 s → ~8 s) | No test passes by absence; demo has a golden | Gate + measured duration | — (independent) |
| W4.6 | **Frozen KidEngine MCP script** on a redistributable synthetic fixture (round-trips/bytes/time measured, calibration policy declared) | The script runs end to end; thresholds justified **after** the flow is frozen | `kidengine_script` + measured report | **W3.4–W3.5** (not 8.4-independent) |
| W4.7 | CI layers: PR fast (fmt/check/clippy/core), nightly full + determinism + differential; release manual | Cost/duration measured; no duplicate runs | CI time comparison | W4.1–W4.5 |
| W4.8 | Anti-slop lite: one gate list, no noisy `allow(` scan, dated evidence separated from status | Policy still catches real defects; duplication removed | Policy review + drift guard | W0.3 (independent) |

### W5 — Backend roadmap and product consolidation

| ID | Task | Acceptance | Named verification | Depends on |
|---|---|---|---|---|
| W5.1 | Preserve and land 8.4 Z80/bus (and the existing audio core) through the ordinary flow | **H-1/H-2/H-3 resolved** (§5): handoff commit identified, owner named, integration authorized and gated. Nothing discarded; behavior is ported and reviewed, not preserved blindly | Integration review; 8.4 plan's own tests | H-1/H-2/H-3 |
| W5.2 | 8.5 timers/controllers + complete I/O window; **fix the 8.5 plan's port-3 rule first** | G-BOOT-1 stable on the fixture; controls tested | Boot gate + `controller_*` tests | W1, W5.1 |
| W5.3 | Audio: complete and verify the project's own YM2612/PSG/mixer (O-3) | Differential/tolerance review recorded; snapshot state deterministic; no new dependency | `audio_differential` + tolerance note | W5.1 |
| W5.4 | 8.6a compatibility (real DMA source, scanline render, contention, HBlank timing) | **G-BOOT-2 requires a content-level predicate** (frame reference or audio non-silence), not just a hash; commercial fixtures are local-only and never a CI pass | `g_boot_2` per fixture; recorded visual/audio evidence | W5.2–W5.3 |
| W5.5 | 8.6b performance (the ADR-0015 contract applied) | Per-frame budget measured on the pinned workload; no determinism regression | G-PERF with G1–G6 | W2, W5.4 |
| W5.6 | 8.6c consolidation: UC-1…UC-4 with the frozen definitions | UC definitions are **normative in ADR-0014** (§KidEngine flow); UC-1 uses a committed synthetic ROM (commercial = local attached evidence); UC-2/UC-3/UC-4 pass with versioned evidence. **This closes 8.6c, not M8** (O-2) | Repro artifact + re-execution; `check_docs_drift` | W3, W4.6, W5.5 |
| W5.7 | Keep 8.7/8.8/M9/TLS/async/COW explicitly deferred, without amending the DoD | Roadmap and ADR-0014 list the deferrals; M8 remains open | Reading + status guard | O-2 |

## 5. Sequencing (acyclic DAG)

```text
evidence: W0.4 (review report already durable in docs/reviews/)
   └─> corrections applied (S2, done) ──> independent review of the corrected tree (V2) ──> V3
          └─> ADRs Accepted (owner act)  ──> W2 design, W3 design
8.4 integration: H-1 commit + H-2 owner + H-3 authorization
   └─> W5.1 EXECUTED and verified on the authorized base   [prerequisite for W1]
          └─> W1.1–W1.7 (bus/boot)  ──> W5.2 (8.5)
W2.4 (cheap status + register map) ──> W3.1 read path   [or an independent metadata cache in W3.1]
W2.1–W2.5 (measured) ──┬──> W2.7 report (pinned workload, G1–G6)
                       └──> W2.6 (OPTIONAL render branch, measured) ──> feeds W2.7 only if pursued
W3.1 ──> W3.2 ──> W3.3 ──> W3.4 ──> W3.4a (frame) / W3.5 (live session) ──> W3.6 ──> W3.7
W4.1/W4.2/W4.5/W4.8 (independent: start now)
   └─> W4.3/W4.4 (need W4.2) ──> W4.7 (CI layers)
W4.6 (KidEngine script) needs W3.4/W3.5
W5.1 ──> W5.2 ──> W5.3 ──> W5.4 ──> W5.5 ──> W5.6 (8.6c; M8 stays open)
```

**Phases:**

| Phase | Work | Gate |
|---|---|---|
| **0 — Now (no backend code)** | V2/V3 review; W0.2/W0.4/W0.5; W4.1/W4.2/W4.5/W4.8 | V2/V3 PASS; review report in-tree (done) |
| **1 — After 8.4 is integrated (W5.1 executed)** | W1.1–W1.7; W2 design | H-1/H-2/H-3 **and W5.1 verified**; G-BOOT-0/1 |
| **2 — Platform** | W2.1–W2.8 (W2.6 optional); W3.1–W3.7; W4.3/W4.4/W4.7 (W4.6 after W3.4/W3.5) | ADR-0015/0016 accepted; W1 for behavior changes |
| **3 — Backend** | W5.2, W5.3, W5.4 | W1; 8.4 integrated |
| **4 — Consolidation** | W5.5, W5.6 | W5.4; frozen UC definitions; W4.6 |
| **Kept open (not deferred out of M8)** | 8.7, 8.8, then M9 | Owner act only if the product is redefined (O-2) |

**Critical path:** V2/V3 → ADRs accepted → 8.4 integrated (W5.1) → W1 → W5.2 → W5.3 → W5.4 →
W5.5/W5.6. W2/W3 proceed in parallel where files do not overlap; W2 backend changes follow W1;
W2.6 is a measured branch that does not gate W2.7.

### 8.4 integration (H-1/H-2/H-3) — prerequisite of W1

Before any task touches `bus.rs`/`backend.rs`, all three must be **executed** (not merely
decided):

- **H-1 — handoff commit:** the exact branch/commit to integrate (`agent/8.4-z80-audio`, tip
  `408cfd7`) plus the uncommitted T4 changes, either committed by the 8.4 owner or explicitly
  excluded; the base commit for W1 is recorded.
- **H-2 — responsible/owner:** the 8.4 owner confirms what is complete and the next real
  blocker; the audio decision follows O-3.
- **H-3 — authorization and integration:** an authorized integration path (PR/review per the
  repository policy) and the 8.4 plan's gate green **on the W1 base**, with `bus.rs`/
  `backend.rs` owned sequentially. If the owner prefers a frozen handoff commit with sequential
  patches instead of an integrated merge, that variant is recorded here and W1 depends on the
  frozen commit; either way, "handoff decided" alone is not sufficient — W1 starts only after
  the recorded base exists and its gate has passed.

## 6. Gates

- **G-BOOT-0:** 300k steps on the fixtures without a host error. Necessary, not sufficient.
- **G-BOOT-1:** fixture-specific bootstrap markers reached (init PCs, VDP transitions, leaving
  the fatal handler). A fixture that stops at a blocker **fails the gate**; the investigation
  is the deliverable. A `BRA *` cannot pass it.
- **G-BOOT-2:** frame + audio per ROM with a **content-level predicate** (reference image or
  non-silence), local-only, skip without fixtures; never a CI pass by absence. A hash alone
  proves repeatability, not correctness.
- **G-PERF:** identified workload, interleaved A/B, p50+p95, allocator mode, verified path
  measured, relative regression, blocking only on a calibrated runner (G1–G6).
- **G-DET:** same inputs ⇒ identical ticks/frames/audio/hashes at declared boundaries in
  unpaced/paced/stepped modes. Independent of 8.7; stays a W2 requirement.
- **G-TAS:** old movies replay/verify; new movies declare policy; first divergence localized.
- **G-API:** the frozen KidEngine script on a synthetic fixture, with measured round trips,
  bytes and wall time and justified thresholds; frame/live-session work per W3.4a/W3.5.
- **G-SEC:** loopback only; non-loopback bind **refused** (typed error + test); `Origin`
  allowlist; slowloris bounded; no non-loopback claim.
- **G-QUALITY:** fmt/check/clippy/test/rustdoc; no silent skips; reproducible differential.

## 7. Risks and mitigations

| Risk | Mitigation |
|---|---|
| 8.4 branch conflicts with W1/W2 | H-1/H-2/H-3 before touching bus/backend; sequential ownership; never rebase/merge 8.4 from this plan |
| Corrections applied but not independently reviewed | V2 review of the corrected tree before `Accepted`; W0.1 |
| Behavior changes (bus/DMA) alter snapshots/hashes | Version payloads; regenerate goldens; separate behavior changes from optimizations; document breaks |
| API break (`StepOutcome`, service JSON, `MachineInspection`) destabilizes consumers | ADR + migration window; versioned wire; update the 121 `.inspect()`/93 `state_hash` reads |
| Concurrent service changes introduce races | The race catalogue in ADR-0016 (R1–R7) with tests; lock order and atomic admission are contract |
| Reconciliation double-applies or grows unbounded | Monotonic revision + pre-known request key; bounded journal that never evicts active (W3.2) |
| Render skipped to "save time" deletes the frame artifact | Preserve boundary/index state; byte-identical tests (W2.6); frame is an explicit task (W3.4a) |
| Performance numbers decided by noise | G1–G6 protocol; blocking only on a calibrated runner |
| Audio work discarded or duplicated | O-3: complete the project's own core; a future swap needs an ADR and does not discard Z80/bus |
| CI cost/cross-platform | Layered CI; pinned workloads; release manual; no absolute timing gate |
| License/provenance of audio/core | Primary license audit; pin commit; notices; ADR before adoption; no GPL/non-commercial embedding |

## 8. Owner approvals

- **ADR-0014/0015/0016:** accept or request changes — **after** the V2 independent review of
  the corrected tree (the corrections themselves are not acceptance).
- **O-1…O-4:** recorded (2026-11-20); O-2 means no DoD amendment. If the owner later wants to
  redefine the product/milestone, that is a separate act listing every affected requirement.
- **Breaking API:** approve the `StepOutcome`/`MachineInspection`/service-JSON/wire-version
  break (0.x semver + ADRs).
- **M9 deferral** and the post-M8 `2026-07-28` MCP migration: acknowledged.
- **Publication/merge:** separate explicit authorization; this plan does not authorize push,
  PR, merge or release, and none of the 8.4 work may be integrated without H-3.

## 9. Definition of done (plan level)

- V2 review PASS; ADRs accepted or explicitly owner-deferred; decisions recorded.
- Each task has an Issue/PR with objective, scope, acceptance, verification and independent
  review; each names files, a test and a failure condition.
- G-BOOT/G-DET/G-TAS/G-API/G-SEC/G-QUALITY pass where applicable; no gate passes by absence.
- No silent skips; no weakened tests; no unreviewed dependency.
- The roadmap/status docs agree; **M8 remains open with 8.7/8.8 pending**; consolidation is
  closed as a phase.
- 8.4 is preserved and integrated through H-1/H-2/H-3.
- No commercial ROM/BIOS/firmware committed; no secrets or private paths.

## 10. Evidence and references

- Synthesis: `.research/consolidation-2026-11-20/SYNTHESIS-CONSOLIDATION.md`.
- Corrections applied: `.research/consolidation-2026-11-20/CORRECTIONS-APPLIED.md`.
- Reviews R1–R5: `.research/consolidation-2026-11-20/reviews/adr-001{4-roadmap,5-rust,5-perf,6-interfaces,6-concurrency}.md`.
- Critique: `.research/consolidation-2026-11-20/critique-consolidation.md`.
- Independent review: `docs/reviews/2026-11-20-independent-review/REVIEW.md` (promotion is
  W0.4; until then the review worktree copy).
- Code anchors: `crates/chronofork-backend-api/src/traits.rs:12-33`;
  `crates/chronofork-megadrive/src/backend.rs:632-685`, `:883-977`;
  `crates/chronofork-megadrive/src/bus.rs:51-53`, `:762-771`;
  `crates/chronofork-service/src/service.rs:114-122`, `:461-491`;
  `crates/chronofork-service/src/limits.rs:35-37`; `crates/chronofork-transport/src/mcp.rs:29`;
  `crates/chronofork-transport/src/http.rs:101`.

## 11. Open questions for the owner/team

1. ~~Should W2 start before W1 lands?~~ **Resolved:** design in parallel; code after W1 to
   avoid backend conflicts (W2.1–W2.5 measured before the optional render task W2.6).
2. ~~Audio inside 8.4 or paused?~~ **Resolved by O-3:** complete the project's own core.
3. **Exact frame format** (raw RGBA paginated, PNG, compact indexed) — W3.4a decides with a
   reconstruction and size test; the ADR fixes only the budget ceiling and `frame_hash`.
4. **Live-session envelope** — W3.5 decides (reuse snapshot payload + live metadata).
5. **Quota defaults** — the ADR-0016 table proposes conservative values; re-measure after the
   first workload.
6. **Round-trip threshold** for G-API — set after the KidEngine script is frozen (W4.6).
7. ~~Client timeout: raise or poll?~~ **Decided (V3-1):** the v1 client timeout is fixed above
   the 30 s server deadline plus an explicit margin for the final response/hash; asynchronous
   poll delivery is out of scope for v1 (if ever chosen, it replaces this rule).
