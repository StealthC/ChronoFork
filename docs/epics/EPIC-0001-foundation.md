# EPIC-0001 — Project Foundation, Research and Deterministic Session Architecture

- **Status:** Complete (bootstrap foundation); ADR-0002 accepted 2026-11-13
- **Milestone:** M0
- **Owner:** Lead architect
- **Created:** 2026-11-13

## Purpose

Establish a real, maintainable foundation for ChronoFork: validated environment,
durable planning, evidence-backed upstream research, licensing/provenance,
official documentation, a harness-independent `AGENTS.md`, a Rust workspace, a
deterministic synthetic backend, and executable proof of session isolation,
snapshots, restore and forking.

This epic does **not** implement a Mega Drive emulator. It builds the contracts a
real backend will implement.

## Scope

- Environment and toolchain validation.
- Reference acquisition and research.
- License/provenance assessment.
- Product vision, scope, requirements, architecture, ADRs, roadmap.
- Rust workspace and crate boundaries.
- Backend capability/snapshot/inspection contract.
- Deterministic synthetic backend.
- Session lifecycle, snapshots, restore, fork, lineage, comparison.
- CLI demonstration of implemented features only.
- Automated acceptance tests and quality gates.
- Final consolidated evidence report.

## Out of scope

- Real 68000/Z80/VDP/YM2612/PSG implementation.
- Game compatibility.
- GUI, HTTP API, MCP server, scripting runtime.
- TAS movie format and rerecording.
- Persistent database, cloud, multiplayer.
- Copy-on-write or process-isolated execution.
- CI provider selection or remote repository publication.
- Reuse of BizHawk code.
- Any commercial ROM/firmware.

## Workstreams and tasks

| ID | Task | Owner | Depends on | Status |
|---|---|---|---|---|
| E1-01 | Inspect WSL, toolchain, resources, network | Lead | — | Done |
| E1-02 | Read installed planning skill, initialize named plan | Lead | E1-01 | Done |
| E1-03 | Create `.gitignore` and Git repo before clones | Lead | — | Done |
| E1-04 | Clone Exodus, exodus-headless, BizHawk; record SHAs | Lead | E1-03 | Done |
| E1-05 | Research Exodus/headless architecture | Architecture researcher | E1-04 | Done |
| E1-06 | Research BizHawk TAS/debugging architecture | TAS researcher | E1-04 | Done |
| E1-07 | Licensing/provenance and testing strategy | Quality researcher | E1-04 | Done |
| E1-08 | Official docs, ADRs, roadmap, AGENTS.md | Lead | E1-05..07 | Done |
| E1-09 | Rust workspace and crate boundaries | Lead | E1-01 | Done |
| E1-10 | Backend contracts and synthetic backend | Lead | E1-09 | Done |
| E1-11 | Session lifecycle, snapshot, restore, fork | Lead | E1-10 | Done |
| E1-12 | Acceptance tests and quality gates | Lead | E1-11 | Done |
| E1-13 | Minimal CLI demo | Lead | E1-11 | Done |
| E1-14 | Backend strategy decision | Lead + owner | E1-05..07 | Done (Option E accepted) |
| E1-15 | Final review, evidence, handoff | Lead | all | Done |

## Acceptance criteria

1. `cargo fmt --all -- --check` succeeds.
2. `cargo check --workspace` succeeds.
3. `cargo clippy --workspace --all-targets -- -D warnings` succeeds.
4. `cargo test --workspace` passes all tests.
5. `chronofork demo` exits 0 and proves: checkpoint, three forks, divergent hashes,
   source isolation, memory isolation, restore/replay equality.
6. Snapshot serialization round-trips and rejects corruption/version mismatch.
7. Forked sessions start at the checkpoint tick/hash and do not mutate the source.
8. Reference repos are under `.research/` and ignored by Git.
9. No ROMs, firmware, secrets, build outputs or upstream clones are tracked.
10. `AGENTS.md` is harness-independent and links the docs index.
11. Licensing policy, notices and provenance exist; no upstream code is
    incorporated.
12. The real-backend decision is documented; ADR-0002 is **Accepted**, and EPIC-0002 tracks implementation.
13. The final report distinguishes implemented, unverified, blocked and next work.

## Testing expectations

- Unit tests for core values and snapshot codec.
- Backend unit tests for determinism, divergence, snapshot restore, bounds and
  invalid programs.
- Session acceptance tests for three-way fork, isolation, restore/replay, snapshot
  import and lifecycle guards.
- CLI integration tests for `demo` and `backend describe`.
- Quality gates executed locally and recorded in the final report.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| No real backend decision | Cannot run actual games | Explicit ADR-0002 proposal; do not pretend synthetic backend is hardware |
| Rust-native effort underestimated | Long delay | Incremental hardware slice; differential harness; decision gate |
| Upstream build unavailable (Ninja/clang/ROM) | Equivalence claim unverified | Document blocker; request approval before installing packages |
| BizHawk licensing heterogeneity | Legal exposure | No reuse; provenance policy; treat as research only |
| Snapshot format churn | Broken persistence | Versioned envelope; explicit rejection; compatibility policy |
| Overengineering | Slower foundation | Five crates; no HTTP/MCP/GUI yet |
| Hidden nondeterminism in future core | Invalid forks/replays | State hash tests, differential traces, documented limits |

## Deliverables

- Viable Git repository with ignored research/planning paths.
- Research reports (architecture, TAS/debugging, licensing).
- Official documentation tree and `AGENTS.md`.
- Rust workspace with contracts, sessions, synthetic backend, CLI.
- Automated tests and local quality gate.
- Backend strategy proposal and ADR-0002.
- Consolidated evidence report with blockers and next steps.

## Definition of Done

- All acceptance criteria are met or explicitly reported as unmet with a blocker.
- Quality gates have actually been executed, not predicted.
- Documentation links resolve.
- No prohibited assets are tracked.
- Licensing and provenance are explicit.
- The next milestone has a prioritized, dependency-aware plan.
