# ChronoFork Documentation

This directory is the durable, human- and agent-readable knowledge base for
ChronoFork. Operational working memory lives in the ignored `.planning/` directory;
it is never a substitute for these documents.

## Status

Bootstrap foundation complete, with a partial real-hardware slice implemented. The
repository builds and tests a deterministic synthetic backend, the generic session
layer, and a minimal 68000 + ROM/RAM Mega Drive backend with a VDP
status/control/VRAM port window and a deterministic instruction-quantum interrupt
scheduler, including 68000 SR-mask interrupt gating (level 7 always admitted; other
levels only when strictly above the mask), `RTE` handler return and VDP VINT
pending/acknowledge. A bounded debug-only instruction trace and final oracle
differential evidence now exist: three synthetic ROMs match the pinned Exodus
oracle's projected address streams and selected final state, and all three negative
controls pass. `rom_alu` and `rom_mem_vdp` use the explicit 1,000,000-instruction
controlled window, while `rom_masked` uses the CLI default quantum 64 with the flag
omitted (the pre-repair `rom_mem_vdp` divergence and its E2-13 repair remain
recorded, and the earlier two-ROM run is preserved as pre-E2-15 controlled-condition
history). Remaining limits: privilege is not enforced; only HBlank level 4 / VBlank
level 6 sources are modelled, with level 7 a predicate rule only; no interrupt-timing
equivalence is claimed; and VDP interrupt-enable/acknowledge logic is absent.
Rendering, DMA, audio and Z80 are not implemented, and controller input covers only the
TH-high 3-button subset. The final M3 increment — real-hardware input TAS on the minimal
Mega Drive slice — is implemented and independently verified
([ADR-0009](adr/0009-real-hardware-input-tas.md); **Accepted**, owner-approved
2026-11-15T23:38:06-03:00; t5 PASS 2026-11-16), with the [M3 hardware acceptance
plan](quality/m3-hardware-acceptance.md) rows M3-A45–A53 `executed - passed`. **M3 is
complete for the documented deterministic minimal-slice scope**; ADR-0002 is accepted.
Claims below distinguish implemented facts from proposals and upstream research claims.

**M4 (debugging depth) is complete and independently verified for the documented
deterministic minimal-slice scope on Linux.** The contract
[ADR-0010](adr/0010-debugging-depth.md) is implemented: optional bounded non-canonical
debug observation (`DebugAccess` event log and byte-granular watchpoints), the
`chronofork-debug` host layer (PC/range/conditional breakpoints, read/write watchpoints,
event breakpoints and logpoints), register/memory/disassembly views and Mega Drive
CPU/VDP/interrupt/bus inspectors, exposed by `chronofork debug
run|inspect|memory|disasm|vdp|interrupts|bus`. Every M4-A1–A17 row in the [M4 acceptance
plan](quality/m4-debugging-depth-acceptance.md) is `executed - passed` (independent t5
PASS, 2026-11-16); the post-PASS capability-report/wording fix at `6b2a567` was re-gated
and audited. Stops are instruction-granular; no mid-instruction/cycle-exact stop, reverse
execution, persistent debugger state, UI/HTTP/MCP, DMA/Z80/audio/rendering inspection,
complete 68000 disassembly or cross-platform runtime evidence is claimed.

**M2 durable sessions are complete and independently verified (round 2, after the F1
repair).** Fase B persists canonical checkpoints under an explicit versioned root
and provides named metadata, deterministic listing, explicit non-cascading delete with
no automatic GC, a cooperative per-session writer lock/lease, storage-backed
rehydration and the `chronofork session save/load/list/delete` CLI. The M2-A1–A11
acceptance criteria and the full gate passed in two independent verification rounds;
round 1 found one medium CLI lock-scope finding (F1), which was repaired and
re-verified closed in round 2. See the executed [M2 acceptance
plan](quality/m2-durable-sessions-acceptance.md) and the dated [quality
gates](quality/quality-gates.md) result. Residual limits: durability is verified only
on local Linux filesystems (Windows/network filesystems unverified); the writer lock is
cooperative metadata, not authentication; a concurrent publish/read or delete/read
interleaving is not separately tested (`NFR-CONC-003` is Partial); holding the lock
across a long in-memory `--steps` advance lengthens contention; a crashed holder needs explicit
owner-checked recovery; retention is manual; there is no encryption, authentication or
format migration; and the public convenience `save_snapshot` remains publish-only for
third-party callers while the CLI uses the lock-aware workflow.

**M3's first through sixth increments (TAS, deterministic replay, partial replay/seek,
branch comparison, movie format v2 anchors and multi-device input, the in-process rewind
state cache with interactive stepping, rerecord/branch editing, and real-hardware input TAS
with `--rom` backend selection) are implemented and independently verified (M3-T4, M3-T5,
the t7 format-v2 verification PASS, the t5 rewind verification PASS, the t10
repair-round-2 PASS and the t5 hardware-TAS/M3 completion audit PASS,
2026-11-15/2026-11-16).** The first increment adds the canonical single-file v1 `.cfmovie`
format, record/playback of a per-tick input sequence on the deterministic synthetic
backend, exact first-divergence reporting, restart and M2-seeded replay from the embedded
checkpoint, and the `chronofork tas record/play/verify` CLI. The second increment adds
ranged replay/seek over an exact `(from, until]` window with typed range bounds, forward
single-tick advance and rewind by deterministic linear re-simulation from the embedded
checkpoint, and branch comparison of recorded `(tick, state hash)` records with
first-divergence, length and embedded-checkpoint identity reporting (no re-simulation).
The third increment adds the `CFMOVIE2` container with a canonical embedded checkpoint, an
ordered table of canonical checkpoint anchors bound to recorded steps, nearest-anchor
seek in the single ranged replay path, and schema-2 multi-device input (four digital
ports, four analog axes and a keyboard bitmap) recorded/replayed with deterministic
synthetic-backend consumption. Anchored ranged replay reports `anchored=1`,
`anchor_step`, `anchor_tick` and `prefix_verified=0`: it deliberately does not re-execute
the pre-anchor prefix, while full-range `verify` remains complete. The evidence is the
executed [M3 acceptance plan](quality/m3-tas-replay-acceptance.md), the [increment-2
acceptance plan](quality/m3-partial-replay-acceptance.md), the [format-v2 acceptance
plan](quality/m3-format-v2-acceptance.md),
[ADR-0004](adr/0004-tas-movie-format.md),
[ADR-0005](adr/0005-partial-replay-and-branch-comparison.md),
[ADR-0006](adr/0006-movie-format-v2-anchors-and-multi-device-input.md) and the dated
[quality gates](quality/quality-gates.md) results. A fourth increment — a bounded in-process
rewind state cache, deterministic rewind by restoring the nearest verified state and
replaying recorded inputs, and the interactive `chronofork tas studio` stepping surface —
is implemented and independently verified for the deterministic synthetic backend
([ADR-0007](adr/0007-in-process-rewind-and-interactive-stepping.md), M3-A28–A34, t5 PASS
2026-11-15; [M3 rewind acceptance plan](quality/m3-rewind-acceptance.md)). A fifth
increment — input editing, deterministic suffix re-simulation, branch artifacts and
anchor handling on edit — is implemented and independently verified for the deterministic
synthetic backend ([ADR-0008](adr/0008-rerecord-and-branch-editing.md); [M3 rerecord
acceptance plan](quality/m3-rerecord-acceptance.md) rows M3-A35–A44 `executed - passed`,
t10 repair-round-2 PASS 2026-11-16). It adds `chronofork tas edit`, verified-prefix plus
suffix re-simulation into a new branch artifact with `parent_movie_id` set to the source
id, `keep-prefix`/`drop`/`rebuild` anchor handling, and v1→v1 / v2→v2 preservation without
a new format version or stored rerecord counter. A sixth increment adds real-hardware input
TAS: stateless per-step Mega Drive controller data-register reads for ports 1–2 (TH-high
3-button subset; A/Start, TH/6-button multiplexing, timing, analog/keyboard hardware and
commercial ROMs deferred), no canonical snapshot/state-hash/demo/M1 change, and `--rom`
backend selection on `tas record/play/verify/studio/edit` (`compare` stays
backend-agnostic), verified end to end with the synthetic `controller_rom()`. The
[M3 hardware acceptance plan](quality/m3-hardware-acceptance.md) rows M3-A45–A53 are
`executed - passed` (t5 PASS 2026-11-16), and the M3 completion audit passed. **M3 is
complete for the documented deterministic minimal-slice scope** (t6 promotion; t7
promotion audit pending). Import/export adapters remain documented-only, and
checkpoint-reference movie support, HTTP/MCP/GUI and the Stage 6 daemon remain deferred;
none of the in-place/timeline/persisted-counter/mixed-anchor/O(1)-seek capabilities or
TH/6-button/analog/keyboard/cycle-accuracy claims is made. The hardware claim is the
synthetic 68000 controller-reading ROM and 3-button subset, not commercial ROMs or
controller timing. M2, M3 and M4 are the completed milestones.

## Start here

| Document | Purpose |
|---|---|
| [Product vision](vision/product-vision.md) | Why ChronoFork exists and what it must become |
| [Scope](vision/scope.md) | What is in the current and next milestones |
| [Non-goals](vision/non-goals.md) | Explicit boundaries and anti-goals |
| [Architecture overview](architecture/overview.md) | Layers, crates and dependency rules |
| [Backend model](architecture/backend-model.md) | Capability contract and snapshot compatibility |
| [Session model](architecture/session-model.md) | Identity, lifecycle, fork and isolation |
| [Execution model](architecture/execution-model.md) | Virtual time, determinism and replay |
| [Backend strategy](architecture/backend-strategy.md) | Accepted real-backend strategy |
| [Mega Drive 68000 slice](architecture/megadrive-slice.md) | Implemented 68000 + VDP port + interrupt-scheduler slice and debug trace |
| [Requirements](requirements/functional.md) | Functional requirements and traceability |
| [Non-functional requirements](requirements/non-functional.md) | Determinism, quality, portability |
| [Roadmap](roadmap/roadmap.md) | Long-term stages |
| [Milestones](roadmap/milestones.md) | Deliverable checkpoints |
| [EPIC-0001](epics/EPIC-0001-foundation.md) | Foundation epic (complete) |
| [EPIC-0002](epics/EPIC-0002-megadrive-slice.md) | Mega Drive 68000 slice epic |
| [Testing strategy](quality/testing-strategy.md) | Test categories and fixtures |
| [Quality gates](quality/quality-gates.md) | Commands that must pass |
| [M2 durable-sessions acceptance](quality/m2-durable-sessions-acceptance.md) | Executed acceptance criteria, results and residual limits for durable sessions |
| [M3 TAS/replay acceptance](quality/m3-tas-replay-acceptance.md) | Executed acceptance criteria, results and residual limits for the v1 movie, record/replay, first divergence, restart and CLI |
| [M3 partial-replay acceptance](quality/m3-partial-replay-acceptance.md) | Executed acceptance criteria, results and residual limits for ranged replay/seek, single-tick advance, rewind and branch comparison |
| [M3 format-v2 acceptance](quality/m3-format-v2-acceptance.md) | Executed acceptance criteria, results and residual limits for the v2 movie container, anchors, anchor-based seek and multi-device input (t7 PASS, 2026-11-15) |
| [M3 rewind acceptance](quality/m3-rewind-acceptance.md) | Executed acceptance criteria, results and residual limits for the in-process rewind state cache and interactive `tas studio` stepping (t5 PASS, 2026-11-15) |
| [M3 rerecord acceptance](quality/m3-rerecord-acceptance.md) | Executed acceptance criteria, results and residual limits for input editing, suffix re-simulation, branch artifacts and anchor handling (M3-A35–A44; t10 repair-round-2 PASS, 2026-11-16) |
| [M3 hardware acceptance](quality/m3-hardware-acceptance.md) | Executed acceptance criteria, results and evidence for Mega Drive controller reads, hardware record/play/verify/edit/studio, snapshot/fork/rewind, two-controller input, `--rom` selection and M1/M2/M3 non-regression (M3-A45–A53; t5 PASS 2026-11-16) |
| [M4 debugging-depth acceptance](quality/m4-debugging-depth-acceptance.md) | Executed acceptance criteria, results and evidence for breakpoints, watchpoints, event breakpoints/logpoints, views/disassembly, Mega Drive inspectors and non-perturbation (M4-A1–A17; independent t5 PASS 2026-11-16) |
| [Anti-slop policy](quality/anti-slop-policy.md) | Agent execution playbook: pre-flight, baseline-first, design rules, verification loop, review checklist |
| [Anti-slop refactor evidence](quality/anti-slop-refactor-evidence.md) | Verifier-owned baseline/after proof for the anti-slop refactor; §8 is the final PASS |
| [AGENTS.md](../AGENTS.md) | Mandatory repository instructions for coding agents, including the anti-slop workflow |
| [Licensing policy](licensing/policy.md) | Reuse and attribution rules |
| [Provenance](licensing/provenance.md) | Exact upstream revisions and incorporation status |
| [Upstream inventory](research/upstream-repositories.md) | Repositories, branches, commits, evidence |
| [Environment](development/environment.md) | Verified toolchain and limitations |
| [Getting started](development/getting-started.md) | Build, test and demo instructions |
| [ADR-0003: durable session storage](adr/0003-durable-session-storage.md) | Accepted durable-storage, metadata and ownership design; implemented and verified in Fase B |
| [ADR-0004: TAS movie format](adr/0004-tas-movie-format.md) | Accepted v1 canonical `.cfmovie`, embedded checkpoint, input schema and verification design; implemented and verified for the synthetic backend, deferred scope unchanged |
| [ADR-0005: partial replay and branch comparison](adr/0005-partial-replay-and-branch-comparison.md) | Accepted typed-range partial replay/seek by re-simulation and recorded-expectation branch comparison; implemented and verified for the synthetic backend, deferred scope unchanged |
| [ADR-0006: movie format v2](adr/0006-movie-format-v2-anchors-and-multi-device-input.md) | Accepted (owner-approved 2026-11-15) v2 container, checkpoint anchors, anchor-based seek and schema-2 multi-device input; implemented and independently verified for the synthetic backend (t7 PASS) |
| [ADR-0007: in-process rewind and interactive stepping](adr/0007-in-process-rewind-and-interactive-stepping.md) | Accepted (owner-approved 2026-11-15) bounded session-layer state cache, deterministic rewind and `tas studio` stdin surface; implemented and independently verified for the synthetic backend (t5 PASS) |
| [ADR-0008: rerecord and branch editing](adr/0008-rerecord-and-branch-editing.md) | **Accepted** (owner-approved 2026-11-15) functional input editing/truncation, deterministic suffix re-simulation, branch artifact identity/isolation and anchor policy; implemented and independently verified for the synthetic backend (t10 round-2 PASS, 2026-11-16) |
| [ADR-0009: real-hardware input TAS](adr/0009-real-hardware-input-tas.md) | **Accepted** (owner-approved 2026-11-15T23:38:06-03:00); implemented and independently verified for the deterministic minimal-slice scope (t5 PASS 2026-11-16): stateless per-step Mega Drive controller reads, canonical-state non-change, transient wiring and `--rom` TAS backend selection; M3 complete, t7 audit pending |
| [ADR index](adr/README.md) | Architectural decisions and status |

## Research reports

- [Emulator architecture](research/emulator-architecture.md)
- [TAS and debugging](research/tas-and-debugging.md)
- [Licensing assessment](research/licensing-assessment.md)
- [Oracle differential evidence](research/oracle-differential-evidence.md)

## Conventions

- Official technical documentation is written in English.
- Use Markdown links between related documents.
- Mark unverified upstream claims explicitly; do not present README claims as
  independently reproduced evidence.
- Record durable decisions in ADRs, not only in planning files or chat.
- Keep operational logs, scratch notes and temporary findings in `.planning/` or
  another ignored directory.
- `Proposed` ADRs have not been approved. Do not implement a major decision marked
  `Proposed` as if it were accepted.
