# Anti-Slop Policy

Status: Active
Scope: all tracked Rust code, tests, scripts and durable documentation
Baseline: commit `f8a0e2c` (2026-11-14), clean HEAD export
Owner: project maintainers

> During the audit the working tree contained uncommitted E2-15 work. Those changes are
> excluded from the counts below and are mentioned only where they demonstrate a drift
> risk. The committed tree is the baseline for this policy.

## 1. What "slop" means here

Slop is code, tests, documentation or process that adds maintenance, review, runtime or
cognitive cost without adding behavior, evidence or clarity. It is not "code I dislike",
and it is not a license to minimize everything.

Three questions decide whether something is slop:

1. If it is deleted, what behavior or evidence is lost?
2. If this rule, format or claim changes, how many places must be edited?
3. Does it make the next change easier or harder?

### Not slop

- Explicit hardware decode tables or byte layouts where each form is independent.
- Generated fixtures with provenance, even when they resemble other fixtures.
- Duplication between layers when the layers test different contracts (library semantics
  versus CLI wiring).
- Defensive validation at a trust boundary (untrusted snapshot bytes, debugger input).

## 2. Principles

| ID | Principle | Practical consequence |
|---|---|---|
| P1 | One source of truth per rule, format and claim | A second copy is a bug waiting for a divergent edit |
| P2 | Every abstraction pays rent | Two real callers, or a documented public seam, before a helper/trait |
| P3 | Delete before add | A smaller change is preferred when it preserves behavior and evidence |
| P4 | Explicit over clever | Typed errors, named constants, visible control flow |
| P5 | Resources are part of correctness | Bounded allocations and measured step cost, not assumptions |
| P6 | Public API is a promise | No `pub` item without a caller or a documented external contract |
| P7 | Tests are product code | Same review standard as production code, without duplicated scenarios |
| P8 | Documentation is code | Claims are versioned, linked and checked against behavior |

## 3. Rules

### 3.1 One source of truth

| ID | Rule |
|---|---|
| AS-01 | A binary format has exactly one reader and one writer, and repeated byte-read/write primitives have one shared implementation. If a second implementation is unavoidable, derive it from the first and add a differential test. |
| AS-02 | An invariant is encoded once. Backend compatibility, interrupt-mask delivery, video-standard codes and similar rules must live in one function. |
| AS-03 | A state hash is computed from the exact serialized state (or from the same single writer). Two hand-maintained serializers for one logical state are prohibited. |

### 3.2 No dead weight

| ID | Rule |
|---|---|
| AS-04 | Validate at construction; do not re-check impossible invariants. No `unwrap_or` fallback for an impossible index, no `try_from` after a range proof, no unreachable error branch. |
| AS-05 | No speculative public API. Every new `pub` item needs a caller in the same change or a documented external contract; otherwise keep it private. |
| AS-06 | No dead artifacts: commented-out code, unused private items, `TODO`/`FIXME` without owner and issue, or `#[allow]` without a reason on the same line/item. |

### 3.3 Duplication

| ID | Rule |
|---|---|
| AS-07 | Duplication is a decision. Three copies of non-trivial logic trigger extraction or a written exception in the change description. Copies that remain must be labeled as deliberate and covered by tests. |

### 3.4 Tests

| ID | Rule |
|---|---|
| AS-08 | One behavior, one test layer. Library semantics belong in library tests; CLI tests assert command wiring, formats and exit codes; demos are smoke tests, not a second oracle. |
| AS-09 | Tests are deterministic and bounded. No host clock, entropy, network or `sleep`; bounded runtime; decoders receive adversarial and arbitrary-byte tests. |

### 3.5 Errors and APIs

| ID | Rule |
|---|---|
| AS-10 | Errors keep their specificity at each boundary. Do not flatten a typed error into `String` when a variant exists; preserve the source chain. |
| AS-11 | Dependencies are liabilities. Justify each new dependency (license, maintenance, size), check for duplicate versions, and do not add a crate for trivial code. |

### 3.6 Resources

| ID | Rule |
|---|---|
| AS-12 | Bound resources. Never allocate from an external length before validating the range. Step-level work must be O(1) in machine size unless the cost is documented and benchmarked. |

### 3.7 Documentation

| ID | Rule |
|---|---|
| AS-13 | Documentation has one home. Keep one canonical capability/limitation table and link to it; do not repeat volatile claims, exact test counts or line counts in multiple documents. |

### 3.8 Structure and gates

| ID | Rule |
|---|---|
| AS-14 | A module has one responsibility. More than 800 non-test lines is a review trigger: split by responsibility, not by line count. |
| AS-15 | Gates are not negotiable. `./scripts/check.sh` runs before commit/merge; a lint or test is never weakened without documented review. |
| AS-16 | Refactor with a safety net. Keep or add a regression test before deduplicating; separate behavior-preserving refactors from behavior changes. |

## 4. Severity

| Severity | Examples | Effect |
|---|---|---|
| Blocker | Duplicate canonical format/rule; behavior change without a test; capability overclaim; unbounded allocation from external input; public API that silently changes canonical behavior; weakened gate | Do not merge |
| Major | Three or more copies of non-trivial logic; missing test at the lowest layer; typed error flattened; stale capability document; unjustified dependency | Fix in the change or file a tracked follow-up with owner |
| Minor | Naming, local comment, formatting preference | Do not block; do not mix with a blocker |

Reviewers do not block a blocker on minor issues, and do not approve a blocker because the
rest of the change is good.

## 5. Review checklist

Before requesting review, the author answers:

- [ ] What did this change delete or simplify?
- [ ] Does every new `pub` item have a caller or a documented contract?
- [ ] Is every new rule/format encoded exactly once?
- [ ] Are impossible states unrepresentable, or validated once at construction?
- [ ] Are all allocations and loops bounded by validated input?
- [ ] Which test proves the behavior at the lowest useful layer?
- [ ] Does the canonical capability/limitation table still match the code?
- [ ] Is there any new `TODO`, `#[allow]` or dead branch? Who owns it and when does it expire?
- [ ] Did `./scripts/check.sh` pass, and is the actual result recorded?
- [ ] Is the net growth justified, and is the change reviewable in one sitting?

## 6. Audit: 2026-11-14 (HEAD `f8a0e2c`)

### 6.1 Strengths

| Strength | Evidence | Keep doing |
|---|---|---|
| Clear layer boundaries | `chronofork-sessions` depends only on the backend contract; concrete backends are dev-dependencies; `docs/architecture/overview.md` documents the direction | Enforce dependency direction in review and manifests |
| Determinism is designed, not assumed | `Tick` with checked overflow, no host clock/entropy, `BTreeMap` for stable order, SHA-256 canonical hashes | Reject implicit time/entropy in core state paths |
| Capability honesty | The concrete backend has a trace API, but `tracing` remains `false` in the descriptor because the generic contract does not expose it ([backend.rs:112](../../crates/chronofork-megadrive/src/backend.rs#L112)) | Report only tested capabilities |
| Versioned, validated snapshots | Magic, version, checksum, config hash and state hash are checked; restore builds replacement state before committing ([backend.rs:352](../../crates/chronofork-megadrive/src/backend.rs#L352)) | Keep atomic restore semantics |
| Strict, enforced lints | `unsafe_code = "forbid"`, `unwrap_used`/`expect_used`/`panic`/`todo` denied, Clippy clean, one localized `#[allow]` | Add lints with a reason, never remove them to pass |
| Minimal dependency surface | One direct external dependency (`sha2`) and no duplicate versions | Keep the dependency budget explicit |
| Tests target invariants | Executable tests across core, sessions, both backends and CLI, including determinism, fork isolation, corruption and version rejection | Add tests at the lowest layer, not only end to end |
| Honest documentation | Implemented, partial and not-implemented behavior are separated; ADR-0002 is accepted | Link claims to executable evidence |
| Bounded debug trace | Fixed-capacity ring with dropped/index accounting; excluded from canonical state ([trace.rs:95](../../crates/chronofork-megadrive/src/trace.rs#L95)) | Bound every diagnostic buffer |

### 6.2 Weaknesses

| ID | Weakness | Evidence | Priority |
|---|---|---|---|
| W1 | Five binary readers reimplement the same primitives | `Reader` in [snapshot.rs:204](../../crates/chronofork-sessions/src/snapshot.rs#L204), `PayloadReader` in [lib.rs:420](../../crates/chronofork-test-backend/src/lib.rs#L420), `Reader` in [backend.rs:603](../../crates/chronofork-megadrive/src/backend.rs#L603), `SnapshotReader` in [interrupts.rs:786](../../crates/chronofork-megadrive/src/interrupts.rs#L786), manual parsing in [vdp.rs:404](../../crates/chronofork-megadrive/src/vdp.rs#L404) — about 240 lines of near-duplicate code | P0 |
| W2 | One canonical state, two hand-maintained serializers | `payload` ([backend.rs:279](../../crates/chronofork-megadrive/src/backend.rs#L279)) and `state_hash_for` ([backend.rs:518](../../crates/chronofork-megadrive/src/backend.rs#L518)) serialize the same fields with different magic; a field added to one can silently escape the hash | P0 |
| W3 | Compatibility rules are duplicated | `Session::ensure_compatible` ([session.rs:260](../../crates/chronofork-sessions/src/session.rs#L260)) repeats the backend-id check in `SessionManager::ensure_snapshot_backend` ([manager.rs:286](../../crates/chronofork-sessions/src/manager.rs#L286)); both backends repeat id/version/format/config checks; `VideoStandard::code` ([interrupts.rs:120](../../crates/chronofork-megadrive/src/interrupts.rs#L120)) is re-encoded in `config_hash_for` ([backend.rs:493](../../crates/chronofork-megadrive/src/backend.rs#L493)) | P0 |
| W4 | Dead or unreachable branches | `u8::try_from(input.buttons() & 0xff)` cannot fail ([lib.rs:285](../../crates/chronofork-test-backend/src/lib.rs#L285)); `read_dn`/`write_dn` guard indices that are masked to `0..=7` ([cpu.rs:690](../../crates/chronofork-megadrive/src/cpu.rs#L690)); `u32::try_from` after an explicit range check ([cpu.rs:669](../../crates/chronofork-megadrive/src/cpu.rs#L669)) | P1 |
| W5 | Speculative public API and a misuse-prone helper | `initial_tick` ([session.rs:108](../../crates/chronofork-sessions/src/session.rs#L108)), `snapshot_ids` ([session.rs:126](../../crates/chronofork-sessions/src/session.rs#L126)) and `into_payload` ([snapshot.rs:92](../../crates/chronofork-backend-api/src/snapshot.rs#L92)) have no caller; `acknowledge_next` ([interrupts.rs:611](../../crates/chronofork-megadrive/src/interrupts.rs#L611)) is used only by tests and the backend comment says it is wrong for delivery; seven `STATUS_*` constants document bits with no internal consumer | P1 |
| W6 | Typed backend errors are flattened to strings | The contract defines `BackendError::OutOfBounds` ([error.rs:32](../../crates/chronofork-backend-api/src/error.rs#L32)), but the Mega Drive memory path maps every bus error to `Execution(String)` ([backend.rs:459](../../crates/chronofork-megadrive/src/backend.rs#L459)); `map_cpu` does the same for CPU errors. The synthetic backend uses `OutOfBounds` correctly, so callers cannot rely on one contract | P0 |
| W7 | Full-state hashing and allocation on every step | `step` computes `state_hash()` ([backend.rs:335](../../crates/chronofork-megadrive/src/backend.rs#L335)), which serializes and hashes 64 KiB RAM plus a 64 KiB VDP snapshot plus scheduler state — roughly 128 KiB and one allocation per instruction. `snapshot()` separately calls `state_hash()` and `payload()`, serializing twice | P0 |
| W8 | Unbounded debugger allocation | `MegadriveBus::read_memory` allocates `Vec::with_capacity(length)` before validating the range ([bus.rs:330](../../crates/chronofork-megadrive/src/bus.rs#L330)); the synthetic backend instead validates before slicing ([lib.rs:374](../../crates/chronofork-test-backend/src/lib.rs#L374)). A future transport can pass a huge `length` and trigger an abort/OOM | P0 |
| W9 | Duplicated test scenarios and fixtures | The CLI demo ([main.rs:617](../../crates/chronofork-cli/src/main.rs#L617)) re-asserts the fork/isolation/replay scenario already owned by [acceptance.rs:37](../../crates/chronofork-sessions/tests/acceptance.rs#L37); synthetic ROM builders exist in [trace.rs:14](../../crates/chronofork-cli/tests/trace.rs#L14), [cpu.rs:940](../../crates/chronofork-megadrive/src/cpu.rs#L940) and `demo_rom` | P1 |
| W10 | Documentation repeats volatile claims | About 20 limitation statements across README/docs/code; [quality-gates.md:54](quality-gates.md#L54) records 98 tests as its latest result while HEAD runs 124; [testing-strategy.md:15](testing-strategy.md#L15) still mentions 40. At audit time the CLI help still said the core does not gate interrupts ([main.rs:577](../../crates/chronofork-cli/src/main.rs#L577)) while the in-progress backend documentation described gating | P1 |
| W11 | Gates are manual and were not green in the worktree | HEAD passes `./scripts/check.sh`; at audit time the uncommitted E2-15 tree failed `cargo fmt --check` and four tests. No CI provider is selected ([quality-gates.md:116](quality-gates.md#L116)) | P1 |
| W12 | No adversarial decoder tests or benchmarks | Testing strategy lists property/fuzz tests and benchmarks as planned; the five readers have no arbitrary-byte test, and the step cost is unmeasured | P2 |

### 6.3 Mitigation plan

| Action | Closes | Effort | Done when |
|---|---|---|---|
| Introduce one bounded byte reader/writer (for example `chronofork-core::codec`) with a typed error mapping; migrate the five readers | W1 | M | One implementation; all formats still round-trip; arbitrary-byte tests never panic |
| Build one canonical state serializer and derive both payload and state hash from it | W2, W7 | M | Adding a state field requires one edit; a test proves `state_hash == hash(state_bytes)`; `snapshot()` serializes once |
| Add `validate_envelope` (id, version, format, config) to `chronofork-backend-api` and reuse it in both backends; expose `VideoStandard::code` to the config hash | W3 | S | No duplicate compatibility checks; error messages are consistent |
| Remove impossible branches and impossible-index fallbacks; make register access total | W4 | S | Clippy and review find no unreachable branch; tests cover the remaining paths |
| Remove or privatize zero-caller methods and the `acknowledge_next` footgun; keep only documented hardware constants | W5 | S | No `pub` item without a caller; no API that production code must avoid |
| Map debugger memory errors to `OutOfBounds` and preserve CPU/bus error classification | W6 | S | A caller can distinguish bounds, device and execution failures without parsing strings |
| Validate range before allocating in `read_memory`; cap debugger reads to the mapped window | W8 | S | Huge/overflowing lengths return a typed error without large allocation |
| Own the acceptance scenario in `chronofork-sessions`; reduce CLI tests to wiring/format/exit; share generated ROM builders through a dev-only support crate or feature | W9 | M | No scenario asserted in two layers; one ROM builder per fixture |
| Make the capability/limitation table canonical and link to it; delete repeated sentences and exact counts from durable docs | W10 | S | One table; a repo-wide search finds links, not copies |
| Run `./scripts/check.sh` before every commit; add an optional local hook; wire the same commands into CI when a provider is selected | W11 | S | A red tree cannot be committed unnoticed |
| Add property tests for every decoder and a criterion bench for step/snapshot cost | W12, W7 | M | Decoders accept arbitrary bytes without panic; a recorded benchmark justifies or changes the per-step hash |

Priority order: W2/W6/W8 (correctness and resource safety), then W1/W3 (single source of
truth), then W4/W5/W7/W9/W10, then W11/W12.

## 7. Metrics and ratchet

Metrics are diagnostics, not targets. Do not game LOC or test counts.

| Metric | Baseline (HEAD) | Ratchet |
|---|---|---|
| Tracked files | 81 | Explain large jumps |
| Rust LOC | 10,258 | Net growth is justified in review |
| Durable docs LOC | 6,935 | Prefer deleting duplication to adding prose |
| Workspace crates | 6 | A new crate needs a boundary justification |
| Direct external dependencies | 1 (`sha2`) | No new dependency without AS-11 |
| Lockfile entries | 16 | `cargo tree -d` stays empty |
| Tests | 124 | New behavior adds a test; no test is deleted to pass |
| `unsafe` blocks | 0 | Zero (workspace lint) |
| Production `unwrap`/`expect` | 0 | Zero (workspace lints) |
| Non-test `#[allow]` suppressions | 1 | New suppression requires reason and owner |
| Duplicate binary readers | 5 | 1 |
| Zero-caller `pub` methods | 3 | 0 |
| Modules over 900 lines | 5 (CPU, interrupts, backend, VDP, CLI) | Review trigger, not a hard cap |

Audit cadence:

- Every change: review checklist and `./scripts/check.sh`.
- Every milestone: recount the metrics above and record the result in this document.
- Before 1.0: freeze the public API and require an explicit compatibility decision for
  every remaining `pub` item.

Recommended automated checks (not yet installed; no CI provider is selected):

- `cargo deny check` for licenses, advisories and duplicate versions.
- `cargo machete` or `cargo udeps` for unused dependencies.
- A duplicate-block check (for example `jscpd` or a small token-based script) with a
  non-zero threshold, reviewed rather than blindly enforced.
- `proptest` for `from_bytes`/`restore_bytes` and a criterion bench for step and snapshot.
- A Markdown link check for `docs/`.

## 8. Exceptions

An exception is allowed only when it is written down and owned:

- State the rule, the reason, the scope, the owner and an expiry or removal condition.
- Add the exception to the change description or an ADR; do not hide it in a comment.
- Spikes and experiments live in `.planning/`, `.research/`, a branch or a feature flag,
  never merged as production code without the same review.
- If the policy itself is wrong, amend this document in a dedicated change rather than
  silently ignoring it.

## 9. References

- [Quality gates](quality-gates.md)
- [Testing strategy](testing-strategy.md)
- [Architecture overview](../architecture/overview.md)
- [Backend model](../architecture/backend-model.md)
- [AGENTS.md](../../AGENTS.md)
- [ADR index](../adr/README.md)
