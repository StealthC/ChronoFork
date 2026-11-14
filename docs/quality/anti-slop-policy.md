# Anti-Slop Policy — Agent Execution Playbook

- **Status:** Active.
- **Scope:** all tracked Rust code, tests, scripts and durable documentation.
- **Owner:** project maintainers. The refactor evidence document is verifier-owned.
- **Evidence of record:** [anti-slop refactor evidence](anti-slop-refactor-evidence.md).
  Its §8 is the final, authoritative PASS for the post-repair bytes; §7 is
  superseded history. Read §8 before citing any after-refactor number.
- **Pre-refactor baseline:** commit `c3e92d8`, captured in
  [evidence §2–§4](anti-slop-refactor-evidence.md#2-baseline-provenance) and
  `.research/anti-slop/baseline/` (git-ignored, SHA-256 manifest).
- **Related:** [quality gates](quality-gates.md),
  [testing strategy](testing-strategy.md),
  [backend model](../architecture/backend-model.md),
  [Mega Drive slice](../architecture/megadrive-slice.md),
  [AGENTS.md](../../AGENTS.md).

This document is an execution playbook, not an essay. Start at §1 for every task.
Use §8 as the review gate, §9 to recognize a known failure mode, and §10 before
taking an exception or changing behavior. Every rule below names a check an agent
can run; "I reviewed it" is not a check.

**Sandbox note.** In the current sandbox every Cargo invocation must use
`CARGO_HOME=/tmp/t7-cargo` because the default registry is read-only
([evidence §8.1](anti-slop-refactor-evidence.md#81-tree-under-test-and-changed-path-hashes)).
Record the variable in every evidence note. This changes the dependency cache
location only, not compiled behavior or emitted bytes.

## 0. Non-negotiable invariants

1. **Behavior preservation is proven, not asserted.** For a refactor, every gate,
   demo byte, state hash and serialized length must match the captured baseline
   (evidence §1 S1–S4).
2. **One source of truth.** A format, rule, compatibility predicate or canonical
   claim exists once; everything else links or calls it (AS-01–AS-03).
3. **No unverified claim.** A sentence that says "passes", "identical", "fixed",
   "N tests" or "implemented" is allowed only when a recorded command/artifact in
   the evidence document supports it. Timing figures are never equality gates.
4. **No weakening to pass.** A lint, test, assertion or gate is never removed or
   relaxed to obtain a green result; the only alternative is a documented exception
   under §10 (AS-15).

## 1. Pre-flight (mandatory, in order)

Do all eight steps before editing. Record the result of each in one short note
(task log or `.planning/`); the note becomes the evidence skeleton.

1. **Read `AGENTS.md`.** Check: `test -s AGENTS.md && grep -q "Anti-slop workflow"
   AGENTS.md`.
2. **Read this policy end to end.** Check: `test -f
   docs/quality/anti-slop-policy.md`.
3. **Read the task's acceptance contract, the relevant ADR and the relevant epic.**
   Start at [ADR index](../adr/README.md) and [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md).
   Check: you can name the task id, in-scope paths, out-of-scope paths, acceptance
   criteria and verify commands in your own words. If any is missing, stop (§10).
4. **Inspect the tree and recent history.** Run `git status --short --branch` and
   `git log --oneline -5`. Note HEAD, whether the tree is dirty, and whether the
   dirt belongs to another task.
5. **Locate the current baseline evidence.** Read
   [evidence §2–§4](anti-slop-refactor-evidence.md#2-baseline-provenance) and
   [§8](anti-slop-refactor-evidence.md#8-post-repair-re-verification-final-bytes).
   Check the raw artifacts exist:
   `test -f .research/anti-slop/baseline/MANIFEST.txt &&
   test -f .research/anti-slop/post-repair-verification/POST-REPAIR-SUMMARY.txt`
   (if missing, say so; do not invent a baseline).
6. **Classify the change with the table below.** If the answer is "baseline
   required", do §3 before touching the file.
7. **Write the task contract and confirm disjoint ownership** (§2). Check:
   `git diff --name-only` and `git status --short` contain no path outside your
   contract.
8. **Read the implementation and its existing tests before editing.** Run the
   narrowest relevant test first (for example
   `cargo test -p chronofork-megadrive debugger -- --nocapture`) so a pre-existing
   failure is not attributed to your change.

Stop conditions for pre-flight: missing/ambiguous contract, overlapping file
ownership, no baseline for a behavior-bearing change, or a tree already red for
unrelated reasons that you cannot isolate. Report; do not "fix forward".

### Change classification

| Change touches | Baseline required? | Because |
|---|---|---|
| Prose/docs with no behavior, capability or measurement claim | No; verify links and existing claims | A stale claim is still a defect |
| Capability/limitation table, error contract, public API description | Yes | Claim must match evidence |
| Ticks, CPU/register/PC/SR, VDP, interrupt delivery, fork/lineage, capability values | Yes (S1) | Observable semantics |
| Snapshot payload/envelope bytes, state hash, serialized length | Yes (S2) | Byte-level contract |
| CLI stdout/exit codes | Yes (S3) | User-visible contract |
| Tests only | Yes | Prove coverage was not lost |
| Dependency, toolchain, build script | Yes | Environment and notices |

## 2. Task contract and disjoint file ownership

A task is executable only if its contract states all of:

- task id, objective and the one question the result must answer;
- **in-scope paths** (exact files/directories) and **out-of-scope paths**;
- acceptance criteria, each mapped to a command or artifact;
- verify commands and their expected result shape;
- dependencies and who supplies their results;
- owner/attempt id and where raw evidence is written (ignored `.research/…`);
- whether the tracked evidence document must be refreshed, and by whom.

Ownership rules:

1. **One writer per file.** Never edit a file owned by another agent. The captain
   assigns ownership; if two tasks need one file, the integrator/lead owns it.
2. **Shared files are not worker files.** Workspace manifests, architecture docs,
   `AGENTS.md`, this policy and the evidence document belong to the lead,
   integrator or verifier.
3. **The evidence document is verifier-owned.** Implementers write raw artifacts
   under `.research/` and report; they do not edit the tracked evidence to make
   their result look verified.
4. **A cross-boundary change is a new task**, not a drive-by edit. Stop and
   escalate (§10).
5. **Do not start another member's task**; wait for the scheduler/captain.

Concrete check after every edit and before completion:

```sh
git status --short
git diff --name-only
# compare both lists with the contract's in-scope list; any extra path is a defect
```

If a commit is authorized, stage only the contract paths (`git add <path>…`,
never `git add -A`). If the tree must stay uncommitted, record the tree state and
changed-path hashes in the evidence note instead.

## 3. Baseline-first rule

Capture the baseline **before** the first behavior-bearing edit. "I know what it
printed" is not a baseline.

### 3.1 Capture (once per task, under an ignored path)

```sh
mkdir -p .research/<task>/baseline
export CARGO_HOME=/tmp/t7-cargo          # sandbox only; record it
git rev-parse HEAD > .research/<task>/baseline/head.txt
git status --short --branch > .research/<task>/baseline/git-status.txt
git diff | sha256sum > .research/<task>/baseline/worktree.diff.sha256

./scripts/check.sh > .research/<task>/baseline/check-sh.stdout.txt \
  2> .research/<task>/baseline/check-sh.stderr.txt
echo $? > .research/<task>/baseline/check-sh.exit.txt
cargo test --workspace > .research/<task>/baseline/test.stdout.txt 2>&1
# per-target listings; doc tests appear in the full run, not in --list
cargo test --workspace -- --list > .research/<task>/baseline/test-list.txt

for run in 1 2 3; do
  cargo run -p chronofork-cli -- demo \
    > .research/<task>/baseline/demo.run$run.stdout.txt \
    2> .research/<task>/baseline/demo.run$run.stderr.txt
  echo $? > .research/<task>/baseline/demo.run$run.exit.txt
  cargo run -p chronofork-cli -- megadrive demo \
    > .research/<task>/baseline/megadrive-demo.run$run.stdout.txt \
    2> .research/<task>/baseline/megadrive-demo.run$run.stderr.txt
  echo $? > .research/<task>/baseline/megadrive-demo.run$run.exit.txt
done
sha256sum .research/<task>/baseline/*.stdout.txt > .research/<task>/baseline/MANIFEST.txt
```

Also record environment versions (`rustc --version`, `cargo --version`) and the
changed-path hash set of the tree you started from. `git diff | sha256sum` covers
tracked modifications only; list untracked files separately with `git status --short`.

If serialized bytes may change, capture them directly with a **differential
probe**, not only the state hash: compile one small probe against a clean
`git archive <baseline-commit>` export and against the working tree, then
`cmp` the outputs ([evidence §8.4](anti-slop-refactor-evidence.md#84-differential-serialized-state-probe-recreated)).

### 3.2 Prove equality after

| Requirement | Minimum proof |
|---|---|
| Same behavior (S1) | Full gate exit codes equal; both demo stdouts `cmp`-identical to baseline; every hash/tick/register value in stdout identical |
| Same serialized bytes (S2) | Differential probe outputs byte-identical (`cmp`) plus state hash and length; or a golden-bytes test added in the same change |
| Same CLI contract (S3) | Exit 0 on every run; stdout SHA-256 equal; stderr Cargo-only and non-normative |
| No coverage loss (S4) | Per-target test table: every target ≥ baseline, every increase explained; no removed assertion without an equivalent elsewhere |
| Same structure | Exactly one codec/serializer/validator; no audited uncalled API; no new `#[allow(dead_code)]` (§4) |

Rules:

- A repaint of the same numbers is not proof. Re-run the commands yourself.
- If any byte differs, it is a **behavior change** until proven otherwise. Do not
  call it a cleanup. Go to §10.
- If the change legitimately alters behavior, capture the **new baseline first**,
  then implement, then prove the *new* contract (§10.3).

## 4. Design rules and their concrete checks

Keep the rule IDs: reviews and commit messages may cite them.

### 4.1 One source of truth

| ID | Rule | Concrete check (run it) |
|---|---|---|
| AS-01 | A binary format has one reader/writer built on the shared codec; repeated primitive parsing has one implementation. | `rg -n "struct .*Reader\|struct .*Writer" crates --glob '*.rs'` must find only `chronofork_core::codec::{Reader, Writer}` (plus non-codec types). Then for every `from_bytes`/`restore_bytes`/`decode`, confirm the body uses `Reader`. A documented exception must have (a) a comment explaining the error-contract reason, (b) a property test, (c) a residual-risk entry in the evidence. The VDP exact-length exception is the known example ([evidence §8.8](anti-slop-refactor-evidence.md#88-verdict-final-and-residual-risks) risk 1). |
| AS-02 | An invariant is encoded once: backend compatibility, interrupt delivery, video-standard codes. | Definition count: `rg -n "fn validate_envelope" crates` returns exactly one (`chronofork-backend-api/src/snapshot.rs`). `rg -n "backend_id\|config_hash" crates/chronofork-sessions/src` shows only translation/mapping, not a second comparison ([evidence §8.6](anti-slop-refactor-evidence.md#86-post-repair-contract-changes-verified)). New compatibility rule: one function, both backends call it. For interrupt-delivery/video-code rules, `rg -n "VideoStandard\|deliver" crates` and confirm one mapping function; a second `match` on the same enum elsewhere is a duplicate. |
| AS-03 | The state hash derives from the same serialized state body as the payload; no hand-maintained twin serializer. | `rg -n "fn state_body\|fn state_hash\|state_body_bytes"` shows one field-order writer per backend feeding both; the tests named `state_hash_hashes_the_payload_state_body*` are in the recorded passing suite; `snapshot()` serializes once. For the current design see [evidence §8.7](anti-slop-refactor-evidence.md#87-structural-recheck-on-the-post-repair-tree). |

### 4.2 No dead weight

| ID | Rule | Concrete check (run it) |
|---|---|---|
| AS-04 | Validate once at construction; no re-check of impossible invariants, no unreachable error branch. | `rg -n "unwrap_or\|unreachable!\|try_from" crates --glob '*.rs'`; for each hit, state the constructor/range proof that makes it unnecessary or keep the checked path. Clippy `-D warnings` does not prove reachability; a branch removed for unreachability needs a test covering the remaining paths. |
| AS-05 | No speculative public API. Every new `pub` item has a caller in the same change or a documented external contract. | For each new `pub` item name: `rg -n "\b<name>\b" crates docs` and count uses outside the definition. Zero internal callers + no written external contract = keep it private. Clippy cannot check this; the reference scan is the check. |
| AS-06 | No dead artifacts: commented-out code, unused private items, ownerless `TODO`/`FIXME`, unexplained `#[allow]`. | `rg -n "TODO\|FIXME\|allow\((dead_code\|unused)\|#\[ignore\]" crates --glob '*.rs'`; every hit needs owner + expiry or must be removed. `git diff -U0 \| rg "^\+.*(allow\(\|ignore)"` catches new suppressions. |

### 4.3 Duplication, errors, resources

| ID | Rule | Concrete check (run it) |
|---|---|---|
| AS-07 | Duplication is a decision: three copies of non-trivial logic trigger extraction or a written exception; kept copies are labeled and tested. | Search the duplicated symbol and count definitions; if 3+, extract or record "rule, reason, scope, owner, expiry" in the change/ADR (§10). |
| AS-08 | One behavior, one test layer. | For each scenario, name its owning crate: core codec → core tests; backend semantics → backend tests; session/fork → sessions tests; CLI → wiring/format/exit only; demo → smoke. `rg -n "acceptance\|fork\|replay" crates/chronofork-cli` should not re-assert session semantics. |
| AS-09 | Tests are deterministic and bounded. | `rg -n "sleep\|Instant::now\|SystemTime\|rand::" crates --glob '*.rs'` (proptest is allowed and seeded); property tests use bounded cases; every decoder has an arbitrary-byte/no-panic test. |
| AS-10 | Typed errors keep specificity at each boundary; no flattening to `String`. | Inspect every `map_*error` function and its match arms. Post-repair contract to preserve ([evidence §8.6](anti-slop-refactor-evidence.md#86-post-repair-contract-changes-verified)): `OutOfBounds` only for a request starting inside a mapped window whose validated end exceeds it; `WriteProtected`, `OddAddress`, VDP/side-effect and no-containing-window are `Execution`; session maps `BackendError::IncompatibleSnapshot` to `SessionError::IncompatibleSnapshot`. Any change to this classification is an intentional behavior change (§10). |
| AS-11 | Dependencies are liabilities. | `cargo tree -d` empty; license/notice updated (see `THIRD_PARTY_NOTICES.md`, evidence §8.2 for the proptest dev-only precedent); no crate for trivial code. |
| AS-12 | Bound resources. Never allocate from an external length before validating the range. | For each `with_capacity`/`vec![0; n]` on external input, find the validation before it. The debugger path must validate `u32::try_from(address)` before the `length == 0` shortcut and reject overruns before allocation ([evidence §8.6](anti-slop-refactor-evidence.md#86-post-repair-contract-changes-verified)); add a huge/overflow-length test. Step-level work is O(1) in machine size unless benchmarked and documented. |

### 4.4 Structure and gates

| ID | Rule | Concrete check (run it) |
|---|---|---|
| AS-13 | Documentation has one home: one canonical capability/limitation table, linked not copied; no volatile counts in durable docs. | Canonical table: [Mega Drive slice — Implemented / Not implemented](../architecture/megadrive-slice.md#implemented), with the repo-level status summary in [docs/README.md](../README.md). `rg -n "[0-9]{2,} (tests?\|passed)" docs AGENTS.md` and review each hit: only dated evidence/audit/historical blocks may contain a count; a current claim is a defect. Check links resolve. |
| AS-14 | One module, one responsibility. >800 non-test lines is a review trigger. | `wc -l crates/*/src/*.rs`; a large module must be split by responsibility or justified in review. |
| AS-15 | Gates are not negotiable and are never weakened to pass. | Run the §6 command block. In the diff: no new `#[allow]`, `#[ignore]`, deleted test, relaxed lint, `--no-verify`, or skipped gate. Any intentional change is an exception (§10). |
| AS-16 | Refactor with a safety net: a regression test before deduplication; behavior-preserving changes separated from behavior changes. | The diff contains a test that fails on the old duplication (or proves baseline bytes), and the task/commit states "no behavior change". If both are mixed, stop and split. |

## 5. Implementation loop

One small change at a time; the loop is:

1. **Pick the smallest change** that satisfies one contract item. No drive-by
   reformatting, renaming or "while I'm here" fixes.
2. **Regression test first.** For a refactor, capture/baseline first (§3), then
   make the structural change under a test that pins the old behavior. For a bug
   fix, add the failing test before the fix.
3. **Delete before add.** Prefer removing the duplicate/uncalled item. If adding an
   abstraction, name its two real callers now.
4. **No weakening.** Never add `#[allow]`, `#[ignore]`, `should_panic` (except the
   documented contract test), delete an assertion or lower a lint to get green.
   Check with `git diff -U0 | rg "^\+.*(allow\(|#\[ignore|cfg_attr)"`.
5. **Keep one writer per file and one logical change per commit/task.** If the
   change requires an owned-by-someone-else file, stop and escalate.
6. **Document at the single home** while the change is fresh: source comment for a
   non-obvious measured decision, canonical table/ADR for durable behavior, and a
   raw evidence note under `.research/`.
7. **Re-run the narrow test** after each edit, not only the full suite at the end.
8. **Record every deviation and exception as you go.** A deviation remembered at
   the end is usually a missing exception.

## 6. Verification loop

### 6.1 Exact commands

Run from the repository root. In the sandbox, prefix with
`CARGO_HOME=/tmp/t7-cargo` and record it.

```sh
./scripts/check.sh; echo "check.sh exit=$?"

cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

cargo run -p chronofork-cli -- demo \
  > /tmp/demo.stdout 2> /tmp/demo.stderr; echo "demo exit=$?"
sha256sum /tmp/demo.stdout
cmp /tmp/demo.stdout .research/<task>/baseline/demo.run1.stdout.txt \
  && echo "demo stdout identical"

cargo run -p chronofork-cli -- megadrive demo \
  > /tmp/megadrive-demo.stdout 2> /tmp/megadrive-demo.stderr; echo "megadrive exit=$?"
sha256sum /tmp/megadrive-demo.stdout
cmp /tmp/megadrive-demo.stdout .research/<task>/baseline/megadrive-demo.run1.stdout.txt \
  && echo "megadrive stdout identical"

cargo run -p chronofork-megadrive --example bench_step --release
git diff --check
```

Repeat each demo three times and `cmp` the repeats, as the baseline and final
verifications did ([evidence §8.3](anti-slop-refactor-evidence.md#83-demos-three-runs-each-compared-with-the-t1-baseline)).
A benchmark run is evidence that the fixed workload executes and round-trips; its
timing lines are not an equality gate ([evidence §8.5](anti-slop-refactor-evidence.md#85-benchmark-on-the-post-repair-tree),
[§8.8](anti-slop-refactor-evidence.md#88-verdict-final-and-residual-risks) risk 10).

### 6.2 Evidence-document requirements

Raw artifacts stay in the ignored `.research/<task>/` directory with a
`MANIFEST.txt` (`sha256sum`). The tracked
[evidence document](anti-slop-refactor-evidence.md) records, at minimum:

1. tree under test: HEAD, dirty/untracked paths, `git diff | sha256sum` before and
   after (proves the verifier changed no implementation file);
2. environment and the `CARGO_HOME` used;
3. every gate command with exit code and result; test count per target, compared
   with baseline and every delta explained;
4. demo commands, exit codes, stdout bytes, SHA-256 and `cmp` result for each run;
5. direct serialized-byte proof (differential probe or golden test) when bytes are
   claimed, including payload/envelope sizes and state hashes;
6. structural confirmation (one codec, one state writer per backend, one
   envelope validator, no audited uncalled API, no dead-code allows);
7. benchmark run with the functional lines (sizes/hashes/restore match) and an
   explicit "timing is not a gate" statement;
8. changed-path SHA-256 table binding the claimed bytes, plus residual risks and
   the exact scope of the claim;
9. verdict + limitations.

Rules for the document:

- **Do not edit it to attach your own result.** The verifier owns §7/§8 style
  sections; implementers supply raw artifacts and a report. A later edit that
  changes a hashed path invalidates the recorded hashes: stop and have the
  verifier re-run and refresh the document before completion is claimed.
- **The tree is uncommitted in the current refactor.** The final PASS binds the
  implementation bytes recorded in evidence §8.1 (`c3e92d8` + 21 modified tracked
  files / 7 untracked additions; [evidence §8.8](anti-slop-refactor-evidence.md#88-verdict-final-and-residual-risks)
  risk 9). Do not claim a commit exists. A later edit to a hashed implementation
  path invalidates §8.8 and requires the verifier to re-run and refresh the
  evidence. A documentation-only edit does not change the recorded implementation
  hashes or behavior, but it still changes the shipped tree, so the verifier/owner
  must bind the final tree before merge; never present a docs edit as re-verified.
- **No copied numbers.** Cite the evidence (for example "all gates passed, evidence
  §8.2") instead of pasting a count into a policy or README.

### 6.3 Stop conditions

Stop, report as failed/blocked, and do not mark the task complete when any of:

| # | Condition |
|---|---|
| 1 | Any gate exits non-zero, or a gate cannot run and the blocker is not reported exactly. |
| 2 | A demo's stdout differs from the baseline and the difference is not an approved behavior change with a new baseline. |
| 3 | A state hash, serialized length or probe byte differs and is not explained by an approved contract change. |
| 4 | A per-target test count decreases without an equivalent assertion mapped elsewhere. |
| 5 | A new `#[allow]`, `#[ignore]`, deleted test or relaxed lint appears without a written exception. |
| 6 | A public item is removed/renamed without an intentional API-change note (the refactor's dead-API removal is such a note). |
| 7 | The evidence hashes no longer match the tree, or the claim is wider than the artifacts (for example "all states" from one probe). |
| 8 | File ownership overlaps, or the change needs a file outside the contract. |
| 9 | The tree was already red for unrelated reasons and the failure cannot be isolated. |
| 10 | The only way forward is to guess at the contract or to weaken a check. |

Never "explain away" a difference. If it is intentional, it is a behavior change
and follows §10.3.

## 7. Documentation rules

1. **One canonical capability/limitation home.** The machine-level Implemented /
   Not implemented lists are in
   [Mega Drive slice](../architecture/megadrive-slice.md#implemented) and
   [Not implemented](../architecture/megadrive-slice.md#not-implemented); the
   repository-level status summary is [docs/README.md](../README.md). Link to them;
   do not restate their bullets in another document.
2. **No volatile counts in durable docs.** Test counts, line counts and timings
   belong in the dated evidence document only. Quality-gates' historical result
   blocks are snapshots, not current claims.
3. **Link the evidence.** A behavior or quality claim links
   [anti-slop refactor evidence](anti-slop-refactor-evidence.md) at the section that
   records the command. A claim without a section link is unverified.
4. **Versioned formats.** A serialized-format change updates the format version,
   the rejection tests, the capability table if user-visible, and the evidence
   (AGENTS.md architectural rule 7).
5. **Same-change docs.** Update the affected durable doc in the same change as the
   behavior. Do not leave a "TODO: docs" for a later agent.
6. **Checks before finishing:**
   `rg -n "TODO|FIXME" docs AGENTS.md` (ownerless hits are defects);
   `rg -n "[0-9]{2,} (tests?|passed)" docs AGENTS.md` (review each hit: only dated evidence/audit blocks);
   every relative Markdown link resolves (`test -f`/link check for touched files).

## 8. Agent review checklist (pass/fail)

Answer every question; a "no" is a failure with a required fix, not a discussion
point.

| # | Question | Pass condition | On fail |
|---|---|---|---|
| 1 | Is the task contract complete (scope, acceptance, verify, owner)? | Every field present in §2 | Stop; request contract |
| 2 | Is every edited path inside the contract and singly owned? | `git status`/`git diff --name-only` ⊆ in-scope | Revert; escalate |
| 3 | Was a baseline captured before the first behavior-bearing edit? | §3 artifacts exist with manifest | Stop; capture baseline (if old tree still reachable) or fail |
| 4 | Do all gates pass with recorded exit codes? | `./scripts/check.sh` exit 0 and individual gates exit 0 | Report failure; do not complete |
| 5 | Are both demos byte-identical to baseline across three runs? | `cmp` exit 0; equal SHA-256 | Behavior change → §10.3, or fail |
| 6 | Are serialized bytes/hashes/lengths equal (or the change intentional)? | Probe `cmp` exit 0 + equal hashes/lengths | Treat as behavior change |
| 7 | Is every test target ≥ baseline, with deltas explained? | Per-target table | Restore coverage or fail |
| 8 | Is there exactly one codec/serializer/envelope validator (or a recorded exception)? | §4.1 checks pass | Extract/derive, or write exception |
| 9 | Does every new `pub` item have a caller or documented contract? | Reference scan + note | Make it private/remove |
| 10 | Are all external-length allocations validated first? | §4.3 AS-12 check + test | Fix before completion |
| 11 | Are errors still typed/classified at every boundary? | AS-10 check; post-repair contract preserved | Restore classification or §10.3 |
| 12 | Any new dead branch, `TODO`, `#[allow]`, `#[ignore]`, deleted test? | `git diff -U0` scan clean | Remove or record exception |
| 13 | Is the canonical capability/limitation table still accurate, with links? | Table matches code; no copied counts | Update the one home |
| 14 | Does the evidence record commands, exits, counts, hashes, structure, risks and scope? | §6.2 fields present and attributable | Do not claim completion |
| 15 | Is the claim no wider than the evidence (states, platforms, timing)? | Wording matches artifacts/residual risks | Narrow the claim |
| 16 | Is the tree state (uncommitted vs committed) stated correctly? | Matches `git status`; hashes bind it | Correct the claim |

Reviewers do not approve an implementation they wrote, and do not block a blocker
on a minor issue.

## 9. Anti-pattern catalogue (real examples from this refactor)

Each row is a defect recorded by the 2026-11-14 anti-slop audit (audit HEAD
`f8a0e2c`), the refactor baseline (`c3e92d8`) it was measured against, and what the
refactor did about it. Use it to recognize the pattern; links go to the current
file or the evidence section that records closure. Closure evidence is what the
evidence document records, not a general guarantee.

| ID | Anti-pattern | Real example in this repository | Detect it | Required fix | Closure evidence |
|---|---|---|---|---|---|
| A1 | **Parallel binary readers** reimplementing the same primitives | Five readers: sessions `SnapshotReader`, synthetic `PayloadReader`, Mega Drive payload `Reader`, scheduler `SnapshotReader`, and manual VDP parsing — roughly 240 near-duplicate lines | `rg -n "struct .*Reader" crates`; count parse loops per format | One shared codec; every parser calls it (documented exception only) | One `chronofork_core::codec`; 4/5 readers use it, VDP is the documented index-based exception ([evidence §8.7](anti-slop-refactor-evidence.md#87-structural-recheck-on-the-post-repair-tree), [§8.8](anti-slop-refactor-evidence.md#88-verdict-final-and-residual-risks) risk 1) |
| A2 | **Two hand-maintained serializers for one state** | Mega Drive `payload` and `state_hash_for` wrote the same fields separately; a new field could silently escape the hash. The synthetic backend had the same split | `rg -n "fn state_body\|fn state_hash_for\|payload\(" crates/*/src`; ask "how many field lists?" | One field-order writer; payload and hash derive from it; test `state_hash == hash(state_body)` | `state_body`/`state_body_bytes` feeds both paths; named tests ([evidence §8.7](anti-slop-refactor-evidence.md#87-structural-recheck-on-the-post-repair-tree)) |
| A3 | **Duplicate compatibility predicates** | `Session::ensure_compatible` and `SessionManager::ensure_snapshot_backend` repeated backend-id checks; both backends repeated id/version/format/config checks; `VideoStandard::code` was re-encoded in the config hash | `rg -n "ensure_compatible\|ensure_snapshot_backend\|backend_id ==\|config_hash =="`; definition count for `validate_envelope` | One `validate_envelope`; boundary only translates the typed error | Helpers deleted; single predicate; `map_backend_compatibility` at the session boundary ([evidence §8.6](anti-slop-refactor-evidence.md#86-post-repair-contract-changes-verified)) |
| A4 | **Dead / unreachable branches and impossible-index fallbacks** | `u8::try_from(input.buttons() & 0xff)` cannot fail; `read_dn`/`write_dn` guarded indices already masked to `0..=7`; `u32::try_from` after an explicit range check; unreachable bus branches | `rg -n "try_from\|unwrap_or\|unreachable!\|panic!"`; ask for the proof that makes a check impossible | Validate once at construction; make access total; delete the branch and test the remaining paths | Dead checks removed; `peek8`/`write8` use a total `usize::from((address & 0xFFFF) as u16)` ([evidence §8.1](anti-slop-refactor-evidence.md#81-tree-under-test-and-changed-path-hashes)) |
| A5 | **Uncalled public API and misuse-prone helpers** | `Session::initial_tick`, `Session::snapshot_ids`, `BackendSnapshot::into_payload`, `InterruptScheduler::acknowledge_next` had no production caller; the backend comment said `acknowledge_next` was wrong for delivery | `rg -n "\b<name>\b" crates docs`; zero references outside the definition | Remove or privatize; if external contract, document and test it | All four absent from `crates/` and `docs/`; zero-reference scan of public functions; no dead-code allows ([evidence §8.7](anti-slop-refactor-evidence.md#87-structural-recheck-on-the-post-repair-tree)) |
| A6 | **Typed errors flattened to strings** | Mega Drive `map_memory_error` mapped every bus error to `Execution(String)`, so `OutOfBounds` was unusable, while the synthetic backend used it correctly | Inspect each `map_*error` match; `rg -n "Execution\(.*to_string\|format!\(" crates` | Preserve variants and their classification at every boundary | Post-repair: `OutOfBounds` only for true in-window overruns; protection/alignment/no-window → `Execution`; session boundary maps `IncompatibleSnapshot` ([evidence §8.6](anti-slop-refactor-evidence.md#86-post-repair-contract-changes-verified), risk 4) |
| A7 | **Unbounded allocation / double serialization** | `MegadriveBus::read_memory` allocated `Vec::with_capacity(length)` before validating the range (a huge length could abort); `snapshot()` serialized state twice | Find `with_capacity`/`vec![0;` on external input; check validation precedes; check how many times `snapshot()` builds the body | Validate range (and `u32::try_from(address)` before the zero-length shortcut) before allocating; serialize once | Range validated first; zero-length order restored with a regression test; `snapshot()` serializes once ([evidence §8.6](anti-slop-refactor-evidence.md#86-post-repair-contract-changes-verified)) |
| A8 | **Duplicated test scenarios and fixtures** | The CLI demo re-asserted the fork/isolation/replay scenario already owned by sessions `acceptance.rs`; synthetic ROM builders existed in CLI trace tests, CPU tests and `demo_rom` | `rg -n "fork\|isolation\|replay" crates/chronofork-cli/tests`; ask which layer owns the assertion | One scenario per layer; CLI tests wiring/format/exit only; share a fixture builder | CLI kept +1 wiring smoke, semantic assertions moved to owning layers; test accounting documented ([evidence §8.2](anti-slop-refactor-evidence.md#82-gates-and-the-199-test-accounting)); assessed by target ownership, not line-by-line (risk 7) |
| A9 | **Stale docs and volatile counts** | The audit found ~20 repeated limitation statements; `quality-gates.md` recorded 98 tests when HEAD ran 124; `testing-strategy.md` mentioned 40; CLI help still claimed interrupts were not gated (audit W10) | `rg -n "[0-9]{2,} (tests?\|passed)" docs AGENTS.md`; compare each claim with the canonical table and current evidence | One capability/limitation table; link it; counts only in dated evidence | Policy + gates link the canonical table and evidence; no current count is asserted outside the evidence ([§8.2](anti-slop-refactor-evidence.md#82-gates-and-the-199-test-accounting)) |
| A10 | **Manual gates that were not actually green** | At audit time the uncommitted tree failed `cargo fmt --check` and four tests; no CI provider was selected | Always run `./scripts/check.sh` yourself; never trust a summary | Run the full gate; record exit codes; block merge on red | Final recorded run: full script exit 0, every recorded test passed, rustdoc clean, `git diff --check` clean ([evidence §8.2](anti-slop-refactor-evidence.md#82-gates-and-the-199-test-accounting)) |
| A11 | **No adversarial decoder tests or benchmarks** | Five readers had no arbitrary-byte/property test; per-step cost was unmeasured | `rg -n "proptest\|proptest!\|bench" crates`; list decoder tests | Property tests for every decoder; a bench for step/snapshot cost | Four property-test binaries added ([evidence §8.2](anti-slop-refactor-evidence.md#82-gates-and-the-199-test-accounting)); `bench_step` exists ([evidence §8.5](anti-slop-refactor-evidence.md#85-benchmark-on-the-post-repair-tree)); the bench is evidence, not a CI gate |

## 10. Exceptions, escalation and legitimate behavior changes

### 10.1 Exception record

An exception is allowed only when written down and owned. Record:

- **rule** (for example AS-12) and **reason** (measured/contract constraint);
- **scope** (files/functions and whether it expires);
- **owner** and **expiry/removal condition**;
- **verification** (test/benchmark that covers the exception);
- **location** (ADR or change description; a code comment may explain, not hide).

An exception in a code comment alone is not an exception. "Temporary" without an
owner and expiry is slop. If the policy is wrong, amend it in a dedicated change
(§10.2), do not silently bypass it.

### 10.2 Escalation

| Situation | Escalate to | Include |
|---|---|---|
| Ambiguous contract or missing acceptance criterion | captain/lead | task id, exact ambiguity, options |
| File ownership conflict or need for a shared file | captain/lead/integrator | paths, why shared, proposed owner |
| Baseline missing or tree already red | captain/lead | command, exit code, whether failure predates the change |
| A rule cannot be met without weakening a gate | maintainer + reviewer | rule, constraint, proposed exception record |
| Policy itself is wrong or obsolete | maintainer | section, evidence, proposed amendment |

Workers do not create a replacement team, reassign tasks or edit the verifier's
document; use the team protocol.

### 10.3 When behavior legitimately changes

A bug fix, an intentional error-contract change or a new feature is not a
refactor. Do this, in order:

1. **Declare it** in the task contract as a behavior change with the old and new
   expected results; do not mix it into a "cleanup" commit.
2. **Capture a new baseline** for the changed surface (new expected stdout bytes,
   hashes, serialized length, error variants) and record why.
3. **Update the tests and the canonical documentation** (capability/limitation
   table, ADR/requirements, error contract) in the same change.
4. **Version any serialized format change** and add rejection/compatibility tests.
5. **Re-verify all four S1–S4 surfaces** against the *new* expectations; state
   explicitly which old values are intentionally different and why.
6. **Have the verifier record the new evidence**; cite it from the policy or docs.
   The old PASS does not apply to the new bytes.

The post-repair debugger-error change is the model: it intentionally reclassified
`OutOfBounds`/`Execution`, is covered by named tests, is recorded as a contract
change, and is explicitly excluded from the state-equality claim
([evidence §1](anti-slop-refactor-evidence.md#1-what-state-unchanged-means),
[§8.6](anti-slop-refactor-evidence.md#86-post-repair-contract-changes-verified)).
If a change is both a refactor and a behavior change, split it into two tasks with
two baselines.

## 11. References

- [Anti-slop refactor evidence](anti-slop-refactor-evidence.md) — verifier-owned
  baseline/after record; §8 is the final PASS.
- [Quality gates](quality-gates.md) — commands that must pass.
- [Testing strategy](testing-strategy.md) — test categories and fixture policy.
- [Architecture overview](../architecture/overview.md),
  [backend model](../architecture/backend-model.md),
  [Mega Drive slice](../architecture/megadrive-slice.md).
- [ADR index](../adr/README.md), [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md).
- [AGENTS.md](../../AGENTS.md) — mandatory repository instructions.
- [Licensing policy](../licensing/policy.md) — dependency/attribution rules.
