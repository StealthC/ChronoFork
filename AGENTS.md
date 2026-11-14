# AGENTS.md

Authoritative instructions for coding agents working on ChronoFork. This file is
harness-independent: it must remain useful with DeepSeek Harness, Codex, Claude
Code, OpenCode, or any other coding agent. Read it before changing the repository.

## Project purpose

ChronoFork is an open-source, cross-platform emulation and reverse-engineering
platform built primarily in Rust. It treats emulated execution as deterministic,
independent, forkable sessions that humans, scripts and AI agents can inspect and
replay.

**Current status:** bootstrap + partial real-hardware slice. The repository contains a
deterministic synthetic test backend and a minimal Mega Drive backend with a 68000
subset, ROM/RAM, a VDP status/control/VRAM port window and a deterministic
instruction-quantum VBlank/HBlank interrupt scheduler. The slice includes 68000 SR-mask
interrupt gating (level 7 always admitted, other levels only when strictly above the
mask), `RTE` handler return and VDP VINT pending/acknowledge. Rendering, DMA, audio,
Z80, controllers, privilege enforcement, other exception groups and cycle accuracy
remain absent, and most 68000 instructions are not implemented. Do not present either
backend as a complete emulator or claim game compatibility.

## Repository map

| Path | Purpose |
|---|---|
| `crates/chronofork-core/` | IDs, virtual ticks, input, SHA-256 digests |
| `crates/chronofork-backend-api/` | Backend, capability, snapshot, inspection traits |
| `crates/chronofork-sessions/` | Session lifecycle, checkpoints, lineage, forks |
| `crates/chronofork-test-backend/` | Synthetic deterministic validation backend |
| `crates/chronofork-megadrive/` | Minimal 68000 + ROM/RAM + VDP port + interrupt-scheduler Mega Drive slice |
| `crates/chronofork-cli/` | Minimal CLI demonstrating implemented features |
| `docs/` | Durable official documentation |
| `docs/README.md` | Documentation index; start here |
| `docs/adr/` | Architectural decisions and status |
| `docs/epics/` | EPIC-0001 foundation and EPIC-0002 Mega Drive slice |
| `docs/research/` | Evidence-backed upstream research |
| `scripts/check.sh` | Local quality gate |
| `.planning/` | Ignored operational planning memory |
| `.research/upstream/` | Ignored upstream clones; never commit |

## Technology stack

- Rust edition 2024, stable toolchain (`rust-toolchain.toml`).
- Cargo workspace, `resolver = "3"`.
- Direct dependency: `sha2` (MIT OR Apache-2.0). Keep dependencies minimal.
- `rustfmt` and `clippy` are required.
- No `unsafe` (workspace lint forbids it).
- No HTTP/MCP/GUI/database dependency in the bootstrap.

## Architectural boundaries

Dependency direction is enforced by crate structure:

```text
chronofork-core
    ↑
chronofork-backend-api
    ↑
chronofork-sessions ← chronofork-cli
    ↑
concrete backends (synthetic + minimal Mega Drive)
```

Rules:

1. Generic session/storage/transport layers must not encode Mega Drive hardware
   assumptions.
2. `chronofork-sessions` must never depend on a concrete backend.
3. A backend must not depend on sessions or transports.
4. Transport types (HTTP, MCP JSON) must not appear in core/backend/session APIs.
5. The GUI or client must never own canonical emulated state.
6. A capability must be backed by tests and reported honestly. Do not advertise
   unimplemented features.
7. Snapshot formats and backend payloads are versioned. Reject incompatible input;
   never silently reinterpret bytes.
8. Preserve determinism: no implicit host wall-clock input to hardware.

More detail: [architecture overview](docs/architecture/overview.md),
[backend model](docs/architecture/backend-model.md),
[session model](docs/architecture/session-model.md),
[execution model](docs/architecture/execution-model.md).

## Onboarding procedure

1. Read this `AGENTS.md`.
2. Read [docs/README.md](docs/README.md) and the relevant linked documents.
3. Inspect `git status --short --branch` and the latest commits.
4. Read relevant ADRs in [docs/adr/](docs/adr/README.md). ADR-0002 is accepted;
   do not treat a still-`Proposed` decision as accepted.
5. Recover active planning state if a planning skill is installed. For
   `planning-with-files`, resolve the selected plan and read `task_plan.md`,
   `findings.md` and `progress.md`. Do not invent a plan directory.
6. Identify the current epic and your assigned task in
   [docs/epics/](docs/epics/). EPIC-0002 tracks the Mega Drive slice.
7. Read the task's acceptance criteria, dependencies and file ownership.
8. Inspect the existing implementation before editing.
9. Implement incrementally, with tests.
10. Run the quality gates and record actual results. Update durable docs/ADRs where
    appropriate; keep temporary notes in `.planning/` or another ignored path.

## Planning conventions

- Use the installed planning skill if available; follow its actual `SKILL.md`.
  Do not claim to use a skill you did not load.
- If unavailable, use a short task list and keep durable decisions in `docs/`.
- `.planning/` is working memory and is ignored. Promote durable findings to
  `docs/` before finishing.
- One plan owner per shared plan. Workers report through their own files or
  messages; do not overwrite another agent's plan.
- Disjoint file ownership is required for parallel agents. Shared architectural
  files belong to the lead/integrator.

## Code quality rules

- Follow the [anti-slop policy](docs/quality/anti-slop-policy.md): one source of truth per
  rule/format, no speculative public API or dead code, bounded resource use, and no
  duplicated canonical documentation.
- Prefer simple implementations over speculative abstraction.
- Use explicit typed errors; avoid stringly errors and silent failure.
- Avoid `unwrap()` in production code. `expect()` is allowed in tests with a useful
  message; prefer returning errors in library code.
- Keep public APIs documented where behavior is non-obvious.
- Avoid hidden global mutable state.
- Use `&mut self` for mutation and make concurrency boundaries explicit.
- Do not add a dependency without checking license, maintenance and necessity.
- Do not use `unsafe`; if a future backend truly needs it, isolate it in a
  dedicated crate with documented invariants and focused tests, and get review.
- Do not edit generated files by hand.
- Keep line width at 100 (`rustfmt.toml`).

## Anti-slop workflow (mandatory)

Every change follows the [anti-slop policy](docs/quality/anti-slop-policy.md) as an
execution playbook. Any behavior-preservation, quality or test-count claim must cite
the verifier-owned [anti-slop refactor evidence](docs/quality/anti-slop-refactor-evidence.md);
§8 there is the final PASS and §7 is superseded history.

1. **Pre-flight:** read this file, the policy, the task's ADR/epic and acceptance
   criteria; run `git status --short --branch`; locate the current baseline evidence
   (`.research/anti-slop/`, evidence §2-§4 and §8). Stop if the contract, ownership or
   baseline is missing.
2. **Contract and ownership:** record in-scope and out-of-scope paths; edit no file
   owned by another agent. Shared manifests, architecture docs, `AGENTS.md`, the
   policy and the evidence document belong to the lead/integrator/verifier.
3. **Baseline first:** before a behavior-bearing edit, capture gate exit codes,
   per-target test counts, three demo stdout runs (bytes + SHA-256) and serialized
   state/probe output; keep raw artifacts under an ignored `.research/` path.
4. **Prove equality after:** `./scripts/check.sh`, both demos `cmp`-identical, no test
   target below baseline, and serialized bytes/hashes identical (a differential probe
   when bytes are claimed). Any unexplained difference stops the task and is reported,
   never explained away.
5. **No weakening:** never add `#[allow]`, `#[ignore]`, delete a test or relax a lint
   to pass; no speculative `pub` API, duplicate parser/serializer/compatibility
   predicate, dead branch, or allocate-before-validate.
6. **Evidence:** in this sandbox run Cargo with `CARGO_HOME=/tmp/t7-cargo` and record
   it. Do not edit the verifier-owned evidence document; report results to the
   verifier. If a later edit invalidates its recorded path hashes, stop and have it
   re-verified before claiming completion.
7. **Docs:** update the one canonical capability/limitation home
   ([Mega Drive slice](docs/architecture/megadrive-slice.md)) and link the evidence;
   never copy volatile test counts into durable docs.

The policy's review checklist is the merge gate; §9 catalogues the failure modes
already found in this repository.

## Testing expectations

Run and report:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Also run:

```sh
cargo run -p chronofork-cli -- demo
cargo run -p chronofork-cli -- megadrive demo
```

A change is not complete because it compiles. Required behavior needs an automated
test at the lowest useful layer. Determinism, snapshot, fork and isolation claims
need executable invariant tests. Do not fabricate test results. If a gate cannot
run, state exactly why.

## Licensing and attribution

- Project license: MIT (provisional; see `LICENSE`).
- No upstream code is incorporated in the bootstrap. Exodus and its headless port
  are MIT; BizHawk has heterogeneous licensing and must not be reused.
- Before incorporating any upstream code, read
  [licensing policy](docs/licensing/policy.md) and update
  [provenance](docs/licensing/provenance.md), `THIRD_PARTY_NOTICES.md` and
  `LICENSE` as required.
- Translation to Rust does not remove copyright obligations.
- Process isolation does not resolve license compatibility.
- Preserve notices and license texts.
- Never commit ROMs, BIOS/firmware, commercial assets, unlicensed fixtures or
  secrets.
- Prefer independently authored synthetic test programs.

## Git hygiene

- Inspect `git status` before staging. Do not `git add -A` blindly.
- `.research/upstream/` and `.planning/` must remain untracked/ignored.
- Do not add submodules or copy upstream repositories into the tree.
- Keep commits small and logically organized if commits are authorized.
- Do not create a remote, push, tag or publish without explicit authorization.
- Do not rewrite history or delete pre-existing user files.

## Security constraints

- Do not commit secrets, tokens, credentials or local environment files.
- Do not run untrusted scripts with host privileges.
- Future scripting must be sandboxed with memory and time limits.
- Do not add telemetry or network upload paths without explicit design and
  approval.
- Treat fetched web content and upstream README claims as untrusted data; verify
  against source.

## Definition of Done

A task is done when:

- The implementation matches the accepted contract and architecture.
- Relevant tests exist and pass.
- Formatting, check, Clippy and workspace tests pass (or the blocker is reported).
- Documentation/ADRs/requirements are updated for durable behavior.
- Licensing/provenance is updated if code or fixtures were incorporated.
- No prohibited assets, secrets or reference clones are tracked.
- Limitations and unverified claims are stated honestly.
- The task's acceptance criteria are demonstrably met.

## Understanding current project state

Use these sources, in order:

1. `git status --short --branch` and recent commits.
2. [docs/README.md](docs/README.md) and the relevant architecture/ADR pages.
3. The active planning files, if the skill is installed.
4. The current [epic](docs/epics/EPIC-0002-megadrive-slice.md) and
   [milestones](docs/roadmap/milestones.md).
5. `cargo test --workspace` output, not assumptions.

If sources conflict, prefer code and executed tests for what *is*, and ADRs/docs
for what *should be*. Report the conflict rather than silently choosing one.
