# Stage 1b Re-review (Round 2) — Repairs for Findings F1–F6

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** Complete (review only; no implementation or existing-document edits)
- **Reviewer:** `verifier` (independent reviewer; did not author the repairs)
- **Date:** 2026-11-14
- **Reviewed tree:** commit `73e0bfa` plus the uncommitted stage-1b working tree,
  after repairs t7 (F1), t8 (F5), t9 (F2–F4) and the F6 documentation correction
- **Supersedes:** [Round 1 review](stage1b-review.md) for acceptance purposes; the
  Round 1 document is retained as the historical record of the original findings
- **Verdict:** **Pass** — F1–F6 are closed, the workspace gate is green, the demo
  evidence is exact, and no snapshot regression or capability overclaim was found.

## 1. Scope and method

This is a read-only re-review of the three repair tasks against the Round 1
findings, plus the follow-up F6 count correction. I inspected the changed code
and documentation, re-ran the complete gate, compared the documented demo output
byte-for-byte with the actual demo output, checked the snapshot version constants
and round-trip/rejection tests for regression, swept all durable documentation for
stale test counts, verified that no new capability overclaim was introduced, and
audited Markdown relative links. I did not edit code, tests or any document other
than this review. A first round-2 pass raised one residual low finding (F6, a stale
count in the functional requirements); it was corrected to 71 crate tests with a
link to the authoritative gate record, and this pass re-verifies the whole tree.

## 2. Commands and results

| Command | Result |
|---|---|
| `./scripts/check.sh` | **Passed** (exit 0): fmt, check, clippy `-D warnings`, tests, rustdoc `-D warnings` |
| `cargo test --workspace` (inside the gate) | **98 passed, 0 failed**: core 8, sessions 4 + acceptance 5, test-backend 7, megadrive 71, CLI 3 |
| `cargo run -p chronofork-cli -- megadrive demo` | **Passed** (exit 0), final line `chronofork megadrive demo: PASS` |
| Documented demo output vs actual demo output | **Exact match** (12/12 lines, including both 64-character fork/source hashes) |
| Snapshot regression tests (`old_and_malformed_payloads_are_rejected`, `snapshots_round_trip_and_detect_configuration_mismatch`, `snapshots_include_vdp_and_scheduler_state`) | **3/3 passed** |
| `git status --short --ignored` | Only expected stage-1b modified files, the two untracked modules and the two untracked review files; only `target/`, `.agent-teams/`, `.agents/`, `.planning/`, `.research/` ignored |
| Markdown relative-link audit | **35 files, 145 relative links, 0 broken** |
| Stale-count/stale-claim sweep over durable docs | **No current-tree stale count or stale capability claim remains** |

## 3. Closure status of the review findings

| Finding | Severity | Status | Evidence |
|---|---|---|---|
| F1 — `LEA`/`PEA` decoded absolute-word forms as absolute-long | Medium | **Closed** | `LEA` matches `(opcode & 0xF1FF) == 0x41F9` and `PEA` matches `opcode == 0x4879` ([cpu.rs:329](../../crates/chronofork-megadrive/src/cpu.rs#L329), [:338](../../crates/chronofork-megadrive/src/cpu.rs#L338)); both fetch a 32-bit address. `absolute_word_and_pc_relative_forms_are_rejected_as_illegal` covers `0x41F8`, `0x4878`, `0x41FA`, `0x487A` and asserts `IllegalOpcode` with only the opcode word fetched, A7 and SR untouched ([cpu.rs:879](../../crates/chronofork-megadrive/src/cpu.rs#L879)). The positive tests use the real absolute-long encodings `0x47F9`/`0x4879` ([cpu.rs:854](../../crates/chronofork-megadrive/src/cpu.rs#L854), [:865](../../crates/chronofork-megadrive/src/cpu.rs#L865)) |
| F2 — authoritative docs still described VDP/interrupts as unimplemented | Medium | **Closed** | [AGENTS.md:14](../../AGENTS.md#L14) describes the partial slice and the remaining gaps; the repository-map row is updated ([:30](../../AGENTS.md#L30)); [overview.md:131](../architecture/overview.md#L131) splits implemented VDP-port/scheduler rows from remaining gaps; [backend-model.md:109](../architecture/backend-model.md#L109) lists the VDP window, scheduler, interrupt-entry delivery and device-inclusive snapshots, with production gaps explicit ([:124](../architecture/backend-model.md#L124)); [execution-model.md:105](../architecture/execution-model.md#L105) distinguishes the instruction-quantum scheduler and `CFMDSTATE2` hash from the synthetic baseline |
| F3 — stale "40 tests" counts | Low | **Closed** | [ADR-0001:70](../adr/0001-rust-workspace.md#L70) says 98 (71 in the crate); [testing-strategy.md:126](testing-strategy.md#L126) keeps the dated 40-test bootstrap baseline with an explicit current-tree note and [:556](testing-strategy.md#L556) records both runs; [licensing-assessment.md:48](../research/licensing-assessment.md#L48) is marked as a 2026-11-13 dated snapshot |
| F4 — demo evidence was abbreviated but presented as verbatim | Low | **Closed** | [quality-gates.md:70](quality-gates.md#L70) labels the block as the exact 2026-11-14 output and it matches the executed demo byte-for-byte, including both full hashes |
| F5 — data writes were accepted during a half-completed command | Low | **Closed** | `write_data_port` rejects any data-port write while `command_pending` is true, before touching VRAM, and leaves the half-latched command completable ([vdp.rs:463](../../crates/chronofork-megadrive/src/vdp.rs#L463)); `data_write_rejects_half_completed_command` covers both `write16` and `write8`, byte-identical VRAM, unchanged address, and successful completion afterwards ([vdp.rs:773](../../crates/chronofork-megadrive/src/vdp.rs#L773)) |
| F6 — functional requirements still claimed 69 crate tests | Low | **Closed** | [functional.md:71](../requirements/functional.md#L71) now says "71 crate tests" and links to [quality gates](quality-gates.md); the full count sweep finds no other current-tree stale count (the remaining 40-test references are explicitly dated bootstrap baseline snapshots) |

## 4. Findings

**No open findings.** No high, medium or low finding remains against the reviewed
tree. The only residual items are observations in §6 that do not require action for
this slice.

## 5. Regression and overclaim check

- **No snapshot regression.** The payload version/magic (`CFMD68K2`/2, `CFMDSTATE2`,
  `CFMDCONF2`), VDP snapshot (`CFVDP001`/1) and scheduler snapshot
  (`CFINTSCH`/1) constants are unchanged; the VDP half-command repair changes only
  the rejected-write path, not serialized state. Old/malformed payload rejection,
  VDP+scheduler payload inclusion and configuration-mismatch tests all still pass.
  `snapshot_preserves_half_completed_command` also still passes.
- **No new capability overclaim.** [AGENTS.md:14](../../AGENTS.md#L14),
  [overview.md:123](../architecture/overview.md#L123),
  [backend-model.md:106](../architecture/backend-model.md#L106) and
  [execution-model.md:103](../architecture/execution-model.md#L103) consistently
  describe the slice as partial, instruction-quantum, and without
  rendering/DMA/audio/Z80/controllers/`RTE`/SR-mask gating/VINT-pending wiring. No
  document claims rendering, tracing, cycle accuracy or a complete VDP.
- **No new test-count drift.** README 98, ADR-0001 98/71, testing-strategy 98,
  quality-gates 98/71, megadrive-slice 71/98, licensing note 98 and functional
  requirements 71 all agree with the executed gate; the 40-test figures are
  explicitly dated bootstrap records.
- **No upstream code or ROMs.** No tracked binaries or ROM assets were added; the
  repairs touched one CPU decoder, one VDP write path and documentation only.

## 6. Observations (not findings)

- The VDP read path still clears `command_pending` before rejecting a data read
  during a half-completed command, while the write path preserves the half-latched
  command. This is pre-existing behaviour, not a regression, and the public docs do
  not promise symmetry; if the team wants a single model for rejected data
  accesses, add a test that pins it, but it is not required for this slice.
- The Round 1 review remains in the tree as a historical document, so its
  statements about the pre-repair docs are intentionally no longer true of the
  current tree. No edit is needed.
- The two new modules and both review documents are untracked; the final commit
  must include them.

## 7. Verdict

**Pass.** All six review findings are closed and independently re-verified, the
workspace gate passes end to end with 98 tests (71 in `chronofork-megadrive`), the
documented demo evidence matches the executed demo exactly, snapshot versioning and
rejection behavior are unchanged, fork/restore and determinism coverage remain in
place, and the documentation reports the implemented slice and its limitations
honestly. The Stage 1B integration is ready for the next workflow step.
