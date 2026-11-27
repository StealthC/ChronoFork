# ADR-0015: Execution and verification are separate responsibilities

- **Status:** **Accepted** (owner direction recorded 2026-11-20: the recommended option —
  ADR now, hash opt-in, persistent buffers, cheap `status()`, render at the frame
  boundary — was selected; R1 C1–C10, R4 N1–N11/G1–G6 and critique C1 corrections applied
  2026-11-20; independent review V4 passed 2026-11-20). This accepts the execution/verification
  decision; it does not certify implementation or benchmark results.
- **Implementation status (2026-11-27):** plan **W2.1/W2.2** / design **W2-1/W2-2**
  landed. `Backend::state_hash()` is a required method with no default body,
  `VerifiedStepOutcome` and the default `Backend::step_verified()` compose `step` plus
  exactly one `state_hash()`, and `StepOutcome` is
  `{ tick, state_hash: Option<StateHash>, halted, verified }`. The ordinary `step` no
  longer computes a digest (`None`, `verified = false`); `step_verified` is the only
  per-step digest path and uses the same single canonical source, so snapshot bytes,
  state hashes, TAS/replay and demo goldens are unchanged. A before/after benchmark of the
  same fixed `interrupt_rom(3)` workload through
  `cargo run -p chronofork-megadrive --example bench_step --release` is recorded under
  the ignored `.research/consolidation-w2-2026-11-27/bench/` evidence (command, p50/p95,
  instructions/s and checkpoint bytes; no FPS or game-performance claim). **W2-3** (remove
  the `Option`, add the batch primitive) is not implemented. Evidence:
  `crates/chronofork-backend-api/tests/w2_verified.rs` (`verified_step_hashes_exactly_once_per_step`),
  `crates/chronofork-megadrive/tests/w2_state_hash.rs`,
  `crates/chronofork-megadrive/tests/w2_twin_machines.rs`
  (`twin_machines_normal_vs_verified`),
  `crates/chronofork-test-backend/tests/w2_state_hash.rs`.
- **Date:** 2026-11-20.
- **Milestone:** M8 (functional Mega Drive emulator) and the product-consolidation phase.
- **Related:** [ADR-0014](0014-v1-scope-single-user-loopback.md) (v1 scope),
  [ADR-0016](0016-session-service-and-agent-api.md) (service and agent API).
- **Review evidence:** independent review 2026-11-20, baseline `b8d63a1`. The durable report is
  in this tree at
  [`docs/reviews/2026-11-20-independent-review/REVIEW.md`](../reviews/2026-11-20-independent-review/REVIEW.md).
  Consolidation reviews: `.research/consolidation-2026-11-20/reviews/adr-0015-rust.md` (R1),
  `adr-0015-perf.md` (R4), `critique-consolidation.md` (C1) and the V2 review
  `review-corrected-adrs.md`.

## Context

Before W2-1, `Backend::step(&mut self, InputFrame) -> StepOutcome` returned
`StepOutcome { tick, state_hash, halted }` on every instruction
(`crates/chronofork-backend-api/src/traits.rs:12-33`). The Mega Drive implementation closes
each step with `state_hash()` (`crates/chronofork-megadrive/src/backend.rs:632-685`), which
serializes CPU, 64 KiB RAM, the complete VDP snapshot (VRAM/CRAM/VSRAM/registers/FIFO/DMA/
timing), mapper banks, SRAM, TMSS and the scheduler, then computes SHA-256.

The independent review measured, in release, on the `interrupt_rom(3)` workload:

| Component | Median | Notes |
|---|---:|---|
| Emulation path with render in the loop | **~0.18 µs/step** | Ablation build (`REVIEW_HASH=0`), measured total divided by steps; it **includes** amortized render (~1 frame per 10,256 step boundaries in the harness) |
| Pure CPU+VDP+scheduler (no render) | **~72 ns/step** | **Model-dependent estimate** obtained by subtracting the render share (4.873 fractional renders × 1.108 ms ≈ 5.40 ms over 50k steps). The render/frame ratio used incorporates the incomplete tail of the 400k-step harness, and render executes at discrete boundaries: this is not an isolated measurement, not a confidence interval and not a target |
| State body/copies | **~4.0 µs** | `state_body` construction |
| SHA-256 over 197,058 B | **83.2–93.0 µs** across the three probes (84.7 direct A2; 86.9 residual A3; 83.2–93.0 A1) | ~2.1–2.4 GB/s; the two probes in the table below must be labeled: 84.7 is a direct measurement, 86.9 is `hash_only − body_only` |
| Allocation/page-fault churn | **~116.6 µs** (bimodal) | 51.4 minor faults/step; 82 faults/50k = **0.0016 faults/step**; a difference between configurations, not an isolated component |
| Residual | **~2 µs** | closure of the sum |
| **Median step** | **209.8 µs** | p50 of the trials ≈ **4,766 steps/s**; the 5,185–5,269 steps/s figure is the aggregate rate over 5 bimodal trials (ratio 1.106–1.123) — the two statistics must not be mixed in one row |

The decomposition is not a disjoint sum: the medians come from different harnesses, the
page-fault cost is bimodal and the churn is a configuration difference. The robust statements
are:

- **SHA-256 alone is ~36.5–41.4% of the median step** (84.7/224 in the emulator probe;
  86.9/209.8 in the performance probe), not a single "38–41%" band.
- **The full observability path (body + SHA + churn) is ~98.95% of the median step**
  (`(91.0 + 116.6)/209.8`). The often-quoted 99.9% is `91.0/(91.0 + 0.173)` = **99.81%**, i.e.
  verification relative to verification + emulation, **not** a percentage of the step; the
  reciprocal quoted in the review is ~526×, not ~502×.
- **10,256 steps/frame** is the harness's **boundary count** for a 256×224 NTSC frame; the
  cycle-derived ratio is **10,038.20 steps per frame** (896,040 cycles / 89.263 cycles per step,
  ≈10,038), i.e. the boundary count is +2.17%. Every steps/s figure must state which convention
  it uses.
- Removing **only SHA-256** leaves, by illustrative subtraction, ~122.9 µs (~8.1k steps/s,
  ~0.79 fps at 10,256 steps/frame) because the body and churn remain. With a stable
  allocator, ~6.3 µs (~159k steps/s, ~15.5 fps) — still far from 60 fps on this workload.
- Removing the **entire observability path** is a **hypothesis to measure, not a result**.
  The ablation figure (~0.18 µs/step with render in the loop) is the observation-skipping
  build, which is **not** the target implementation: the target keeps normal, observable
  execution with buffer reuse, and its cost is only known after W2.1–W2.5 are implemented and
  measured. Persistent buffers are expected to reduce the body/allocation share, but that
  expectation is a hypothesis, not a measured outcome. Removing the full observability path is
  **necessary but not sufficient** for an interactive target on a real game: the measurement is
  synthetic, the ablation is not the target, and the projection does not include Z80, audio,
  scanline rendering or contention. **No fps projection is stated here**.
- The evidence A1 §4 ("~9.8k steps/s after removing the hash") answers a different question
  from the A3 ablation (5.55 M steps/s) and must not be mixed with it (R4-N10).

The contract also forces every consumer to pay the hash. `AppService::run` discards
`StepOutcome.state_hash` and only uses `halted` (`crates/chronofork-service/src/service.rs:
461-491`). `inspect()` recomputes the hash and copies 64 KiB of RAM
(`crates/chronofork-megadrive/src/backend.rs:883-977`), and tracing/debug call it per step.
`fork_current` materializes three copies of the same state (~773.5–822 KiB per fork). Render
runs on every frame boundary even when no consumer takes the frame.

Determinism does **not** depend on computing a hash every step: the hash detects divergence;
the input log plus deterministic replay localizes it. TAS verification and first-divergence
reporting do need a verified path, but not necessarily a hash in the normal execution path.

## Problem

How can ChronoFork preserve determinism, snapshot compatibility, TAS verification,
reproducible experiments and first-divergence localization while removing the full-state
verification path from ordinary execution?

## Decision

### 1. `step` executes; it does not verify

`StepOutcome` becomes `{ tick, halted }` (or equivalent). `Backend::state_hash()` is an
explicit, side-effect-free method. Ordinary execution never serializes the full state or
computes SHA-256.

`StepOutcome.state_hash` may be retained as `Option<StateHash>` during a migration window,
but the target is a step without a mandatory hash. The exact shape is an implementation
detail; the contract is that **a normal step is not required to produce a hash**.

### 2. A verified path remains for TAS, replay and divergence

Add `step_verified` / `run_verified` (or a `VerificationPolicy` parameter) that preserves the
current per-step hash semantics. TAS record/play/verify, replay comparison and
first-divergence reporting use this path, so existing guarantees are not lost.

**Single hash source (R1-C1).** `step_verified` must not become a second implementation of
the hash. There is exactly one canonical computation: `state_hash()` over the same
`state_body` the snapshot derives, and the verified path calls it (or shares the already
built body). A dedicated test asserts `state_hash() == hash produced by the verified path`
across the workspace's deterministic tests, so the two can never drift silently. If the
verified path ever needs a different granularity, it is a *policy*, not a second serializer.

`VerificationPolicy` is explicit:

- `EveryStep` — TAS verification, precise divergence, debug;
- `EveryN(n)` / `Checkpoint` — experiments and CI;
- `OnDemand` — interactive inspection;
- the default for ordinary execution is **no per-step hash**.

### 3. Batch execution APIs and their ownership

Ownership is fixed (R1-C3/C10), because "add `run_until` to the backend/session" has two
readings and two PRs:

- `Backend` exposes `step`, `step_verified` and **at most one batch primitive** where the
  backend can be efficient (it owns the machine loop and can stop without a session round
  trip).
- `Session` composes `run_until(condition)` and `run_cycles(n)` on top of those.
- `run_frame()` is a **capability**, not a generic contract: it is offered only where the
  backend can identify a frame boundary deterministically (the Mega Drive backend can; a
  future backend may not). It must not be pushed into `Backend` as a required method.

The batch APIs accept:

- `stop_on`: PC, memory value, event, tick, frame — with an explicit grammar and
  **precedence** when several conditions match on the same boundary;
- input sequence/batch (bounded; the bound and the encoding are part of ADR-0016's limits
  table);
- `max_wall_ms` as a **host policy** (never canonical state);
- a cooperative cancellation token (see §3.1);
- a result carrying `performed`, `tick`, `halted`, the stop reason, and — when the policy
  requests it — a final `state_hash`.

Host time decides *where* to stop; it never changes the emulated evolution. Reproducibility
requires recording the **number of applied steps `k` and the inputs**, not replaying a
deadline. A cancelled or deadline-truncated run is a valid deterministic **prefix**: it must
report `performed = k` and a stop reason even when the envelope is a typed error (R5-K3).

#### 3.1 Cancellation, deadline and request keys (R1-C2, R5-K2/K3, critique C1 §4)

- **Cancellation must be lock-free.** The token lives where the stepping loop can observe it
  without acquiring the session owner or a transport worker that the run itself occupies
  (e.g. a shared atomic flag/`Arc` checked at instruction/batch boundaries). A `cancel`
  operation that queues behind the run it is meant to stop is not cancellation.
- **Granularity is deterministic and reported.** The token is checked at fixed boundaries
  (instruction/batch); the acknowledgment does not promise a step count, and the authority on
  what happened is the final `performed`/`tick`/stop reason.
- **`deadline`/`max_wall_ms` chooses `k`; replay records `k` + inputs.** The deadline is never
  replayed as a deadline.
- **Idempotence and preconditions are separate concerns.** Tick is **not** a monotonic state
  revision: `restore`/`reset`/`write_memory` can move state backwards or leave the tick
  unchanged (two writes can land on the same tick; restore can produce an ABA sequence).
  Therefore:
  - a **session generation/revision counter**, monotonic across every mutating operation
    (step, run, restore, reset, write, fork/close), is the precondition for retry
    (`expected_revision`, and `expected_tick` where the caller genuinely means the tick);
  - a client that wants safe retry supplies a **request key known before the mutation**
    (bound to principal + session + a digest of the arguments). An `operation_id` that is
    first revealed in the lost response gives the client nothing to query;
  - the deduplication horizon (how long a key is remembered, what happens on restart and on
    expiry) is published, and an **active operation is never evicted to admit another**
    (bounded journal with explicit rejection; see ADR-0016 §2).

### 4. Cheap status; inspection on demand

`status()` returns tick, PC, halted and cheap register views without hashing or copying RAM.
`status()` cannot be "tick, PC, halted" only: the debugger and the trace layer need the
register map, so `status()` (or an equally cheap typed view) carries the register file, with
device pseudo-registers (VDP/cartridge/TMSS) separated from the 68000 registers (R1-C5).

`inspect()`/`read_memory` are on-demand and bounded. `MachineInspection` is split into a cheap
`MachineStatus` and typed, per-CPU/detail views (this also prepares the multi-backend contract
without implementing M9). The split is an **additional public break** on top of `StepOutcome`
and is declared together with it (same 0.x semver clause).

**Impact is ~4× larger than "tracing and the debugger" (R1-C4).** In the workspace today there
are **121 `.inspect()` calls and 93 `state_hash` reads**, and the service JSON plus the GUI's
strict decoders already depend on `state_hash` being present. Four paths derive from the
full-state body — `step`, `inspect`, `snapshot`, `fork_current` — so removing the hash from
`step` alone does not remove the cost. The ADR names all four; W2.4 must migrate them, and
the service/GUI contract must be versioned accordingly.

**Measured addendum (2026-11-20).** A clean `git archive b8d63a1` census (tree-manifest digest
`26246f89…`) reproduces the 121 empty-argument `.inspect()` calls exactly, but **not** the 93
`state_hash` reads: the nearest readings are **273** `StepOutcome.state_hash` field reads and
**597** total `state_hash` value reads. The count also omits
`SessionManager::inspect(<session>)` (112 keyed calls), which materializes the same full view
(register map + 64 KiB RAM + digest). Measured blast radius: **233 inspection
materializations (~1.9×) and 299 production sites (~1.3×)**. The decision is unchanged; only
the estimate of its blast radius is corrected. Counts are bound to the tree-manifest digest
and must be re-run per landing (the worktree is shared and moving).

### 5. Render at the frame boundary, not from future state

The current renderer composites at the frame boundary from a clone of canonical VDP state.
That boundary is preserved. If rendering is skipped because no consumer exists, the
implementation must keep enough boundary state to reconstruct that frame on demand; it must
**not** render the future state at `take`. Lazy rendering is an optimization, not a semantic
change: canonical state, latches, audio and frames must be identical with or without a
consumer.

**But today lazy rendering means "frames stop existing", not "frames get cheaper" (R1-C6):**
there is no frame consumer in production (the demos and the GUI read frames for
presentation/tests; no headless consumer asks for frames). Removing render from the loop
would therefore delete the observable artifact rather than economize it. Consequences:

- render is **not** part of this decision's core contract and is **not** a prerequisite for
  the interactive target (it is ~7.6% of a 16.67 ms frame budget — R1-C7): the contract
  change is about the observability path (body + SHA + churn);
- skipping render is allowed **only** when the frame can be reconstructed from preserved
  boundary state (frame index/window), and the byte-identical test with and without a
  consumer is the acceptance;
- the frame **format, budget, pagination and reconstruction test** are an explicit task
  (ADR-0016 W3.4a / plan), not an implied property of lazy rendering.

### 6. Persistent buffers; no speculative COW

Reuse the state body/VDP buffers across steps instead of allocating ~460 KiB per step.
Reuse `Arc` for snapshot payloads and `initial_state`; add `reap()` for closed sessions.
Copy-on-write, deduplication and persistent data structures are **not** required by this
decision; they need usage evidence and their own ADR.

**`Arc` and equality (R1-S4 correction, critique C1 §3).** Moving `payload` to
`Arc<Vec<u8>>` does **not** change `PartialEq` semantics: `Arc<T>::PartialEq` compares the
pointed-to values, so byte equality is preserved. There is no silent semantic break to
declare. The distinction the contract does make is between the **serialized format** (which
does not change) and the **in-memory representation** (which becomes shared). If pointer
identity is ever needed, that is a new type, not a consequence of `Arc`. The state body is
shared between the payload and the hash (S3 confirmed), so there is no duplicated serializer
to refactor.

### 7. Formats and compatibility

- The snapshot payload format does not change because of this ADR. If a payload version bump
  is needed for a behavior change, it follows the existing versioning rules.
- TAS/movie compatibility: existing movies replay with the verified path. New movies record
  the verification policy and hashes at declared intervals. The movie format version is
  bumped only if the stored shape changes; old movies remain readable through the verified
  path.
- `state_hash` remains the canonical digest for artifacts, snapshots and comparisons; it is
  simply not computed on every ordinary step.
- A policy/version identifier must accompany any artifact whose verification semantics
  change.
- **Wire/service/GUI (R1-C4):** because the service JSON and the GUI's strict decoders
  currently require `state_hash`, the contract change is versioned at that boundary too, and
  the migration is part of W2.4 — not an afterthought.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep the per-step hash and micro-optimize | The measured ceiling is ~0.5–1 fps; reordering allocations does not remove the 197 KB serialization + SHA path. |
| Replace SHA-256 with BLAKE3/xxh3 | Still serializes 197 KB per step; smaller gain than the contract change; new dependency/ADR. |
| Incremental/dirty-page hash before changing the contract | Complex, unproven, and does not solve the body/allocation cost; defer until per-step hashing is a product requirement. |
| Copy-on-write snapshots now | Not needed to reach the performance goal; adds versioning complexity. `Arc` + `reap` is the immediate fix. |
| Keep `inspect()` hashing | `inspect()` is a read path; making it cheap is required for tracing/debug and service `list`. |
| Render lazily from current state at `take` | Would change which frame is observed (future state); forbidden by the boundary semantics. |

## Consequences

### Positive

- Ordinary execution **removes the mandatory verification cost** (the ~210 µs/step
  observability path); **production throughput will be measured** after W2.1–W2.5, and the
  ablation figure is **not a target or a claim**.
- TAS/replay keep a verified path; first-divergence reporting remains possible via checkpoint
  + replay.
- `inspect`, tracing and service metadata stop paying the full hash.
- Fork memory drops when the payload/initial state are shared; byte equality is preserved
  (`Arc` compares values).

### Negative / risks

- **Breaking public API**: `StepOutcome`, ~31 call sites, `MachineInspection` (the
  `MachineStatus`/views split), the 121 `.inspect()` calls and 93 `state_hash` reads, golden
  tests, the service JSON and the GUI's strict decoders, and `.cfmovie` semantics. An ADR and
  a migration plan are mandatory.
- **Loss of per-step resolution by default**: a divergence may be detected later. Mitigation:
  verification at checkpoints + replay from the last checkpoint to locate the exact step.
- **Render correctness risk**: skipping/lazy rendering must not change frames or latches, and
  must not delete the frame artifact. Mitigation: preserved boundary/index state + byte-identical
  frame tests with and without a consumer.
- **TAS compatibility**: old movies must keep replaying; new policy metadata must be
  versioned.
- **Benchmark claims**: the ablation is not the target implementation and the workload is
  synthetic; there is no fps promise in this ADR. Acceptance uses a pinned workload with a
  per-frame budget measured after implementation (G-PERF), and a real game frame is measured
  locally as attached evidence — never as a CI pass-by-absence.
- **Scope honesty**: removing the full observability path is **necessary but not sufficient**
  for interactive speed; the residual per-step work (Z80/audio/contention) is not measured by
  the ablation.

### Compatibility and versioning

- Snapshot payload: unchanged unless a separate behavior change requires a bump.
- Movie format: old files readable; new files may add policy metadata; a version bump is
  required if the stored shape changes.
- Public Rust API: breaking change; 0.x semver allows it with an ADR and changelog.
- Hash policy: an artifact carries the policy/version that produced it.

## Verification

1. **Hash invariant, twin machines (R1-C1/C8, V2-5):** two backends are driven with the same
   inputs — one on the **ordinary execution path** (`step`) and one on the **verified path**
   (`step_verified`) — and after **every** step their ticks, hashes and comparable state must be
   equal. This is not the tautological "hash on demand equals the hash the same routine would
   produce on the same state": a shared serializer can make that pass even if the normal path
   executes differently. Additionally, `state_hash()` on demand equals the verified hash for the
   same state, asserted across the workspace's deterministic tests.
2. **Performance (G-PERF, R4-G1…G6):** the workload is **identified in the harness**
   (`interrupt_rom(3)`, NTSC, 3,420 master cycles/line, `89.263` cycles/step, 896,040
   cycles/frame, the step/frame convention used, the harness commit and the allocator mode);
   before/after use an **interleaved A/B in the same window** with the same CPU pinning,
   reporting p50 **and** p95, allocations, minor faults and memory. No absolute universal
   gate and no fps promise. `step_verified`/replay is measured too, to prove the cost was not
   displaced onto TAS. Blocking only on a calibrated runner; advisory elsewhere.
3. **Determinism (G-DET):** same inputs produce identical ticks, frames, audio and hashes at
   declared boundaries in unpaced/paced/stepped modes. This requirement is independent of the
   capture surface (8.7) and of this ADR: it stays in force for W2.
4. **TAS:** old movies replay and verify; new movies record policy; first divergence is
   localized by checkpoint + replay.
5. **Inspection:** `status()` does not hash/copy RAM and does expose the register map;
   `inspect()` is on demand; tracing/debug/service do not pay the full hash per step; the
   service JSON and GUI decoders are migrated with a declared version.
6. **Render:** frames are byte-identical with and without a consumer; no future-state render;
   when render is skipped, the frame is reconstructible from preserved boundary state.
7. **Fork/snapshot:** state hashes and payload bytes are unchanged by the optimization
   (`Arc` compares values, not pointers); memory is measured.
8. **Reconciliation (R5-K3):** a cancelled/deadline-truncated run reports `performed` and the
   stop reason; a retry with a pre-known request key does not double-apply; `expected_revision`
   (monotonic) and `expected_tick` behave as specified, including restore/ABA and two-writes-
   at-one-tick.

## Migration sketch

1. Add `state_hash()` and `step_verified` while keeping `step` behavior temporarily; the
   verified path calls the single hash source.
2. Introduce the new `StepOutcome` and the batch APIs (`run_until`/`run_cycles` in `Session`;
   `run_frame` as a capability); update call sites and tests.
3. Make `inspect()` cheap; add `status()` with the register map; update trace/debug/service
   and version the service JSON/GUI decoders.
4. Reuse buffers; measure (G-PERF); then make rendering lazy/opt-in **with boundary caching**,
   preserving the frame index.
5. Share payload/initial state via `Arc`; add `reap()`.
6. Re-run goldens; version artifacts; update docs.

## Open questions

1. `StepOutcome` without hash vs `Option<StateHash>` during migration? Recommendation: remove
   the field in the target API; a temporary `Option` only if it reduces the migration risk.
2. Default `VerificationPolicy` for the service: `OnDemand` or `Checkpoint`? Recommendation:
   `OnDemand` for ordinary runs, `EveryStep` only for explicit verify/TAS operations.
3. ~~Where does `run_until` live?~~ **Decided (§3):** `Session` composes `run_until`/`run_cycles`;
   `Backend` exposes `step`/`step_verified` plus at most one batch primitive; `run_frame` is a
   capability.
4. Does the movie format need a new version for policy metadata? Recommendation: yes if the
   stored shape changes; otherwise keep v2 and store policy in the artifact envelope.
5. ~~Exact stop-condition precedence grammar and the bounded input encoding?~~ **Decided in
   ADR-0016 §Limits table** (pre/post-step phase, tie-break order, bounded `inputs` sequence).
