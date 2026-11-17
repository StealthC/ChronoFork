# M7 GUI — Executable Acceptance Plan

- **Status:** **Executed and independently verified.** Every row M7-A1–M7-A30 is
  `executed - passed`. The promotion basis is the **final independent verification
  PASS4** (2026-11-17 13:50–13:54 America/Fortaleza) on the stable, fully repaired
  uncommitted worktree; the **final independent review REVIEW5** then returned **PASS**
  with no finding and no regression. The earlier rounds are preserved as history:
  independent verification FAIL (four documentation/surface-contract findings F1–F4),
  repair, verification FAIL3 (M7-A24 as written plus three low residuals), review rounds
  raising N1–N7, A24 and L1/L2, repair, PASS4. No row was promoted by the implementer;
  only the independent verifier execution produced `executed - passed` (§1.8).
- **Companion contract:** [ADR-0013: Human-facing GUI](../adr/0013-human-facing-gui.md) —
  **Accepted and implemented** (owner-approved 2026-11-17 by the explicit Stage 7
  implementation directive; implemented and verified 2026-11-17). §3.9 there is the
  normative `chronofork gui run` grammar; the [GUI architecture](../architecture/gui.md)
  holds the module map, block field order and numeric limits. This plan defines how that
  contract is proven.
- **Milestone:** [M7 — GUI](../roadmap/milestones.md#m7--gui) (`Complete with integration
  pending`); roadmap stage [Stage 7 — Human-facing
  GUI](../roadmap/roadmap.md#stage-7--human-facing-gui).
- **Related:** [architecture overview](../architecture/overview.md), [agent interfaces
  architecture](../architecture/agent-interfaces.md), [M6 acceptance
  plan](m6-agent-interfaces-acceptance.md), [quality gates](quality-gates.md),
  [testing strategy](testing-strategy.md), [anti-slop policy](anti-slop-policy.md),
  [functional requirements](../requirements/functional.md) (FR-GUI-001, FR-EXEC-007).

> **Final verification record (PASS4, 2026-11-17 13:50–13:54 America/Fortaleza).** The
> independent verifier re-executed all 30 rows on the then-uncommitted,
> implementation-complete worktree
> `/home/stealthc/projects/ChronoFork/.agents/worktrees/m7-gui`, branch `agent/m7-gui`,
> HEAD and base `93d47ed308874be0e8e7060641c238c7f6809e31` (= `origin/main`) plus the
> verified M6 snapshot; the tree stayed byte-stable across the run (51 files hashed
> identically at start and end). Verdict: **PASS**. The full gate `./scripts/check.sh`
> exited 0 with **76 targets / 988 passed / 0 failed / 0 ignored** against the M6
> baseline **67 / 800 / 0 / 0** (no target missing, none below baseline); both demos were
> 3/3 byte-identical to the baseline (`986 B 9b4dd564…`, `1582 B 912a5bb6…`, empty
> stderr); the M6 test counts were unchanged (11 / 67 / 60) and the frozen crates had no
> diff. No implementation, test or tracked documentation file was edited by the verifier;
> no commit, push or merge was made. Raw evidence is in the ignored
> `.agents/notes/m7/verify/final2/` tree (`PASS4.md`, `rows/`, `rows/process/`, `gate/`,
> `demos/`, `audit/`); the independent review is
> `.agents/notes/m7/review/REVIEW5.md`. The dated gate record is the
> [2026-11-17 M7 block](quality-gates.md#2026-11-17--m7-human-facing-gui-independent-final-verification-pass4).
> M7 is verified on that worktree and remains **integration-pending** (no commit or
> merge): the milestone is `Complete with integration pending`.

## 1. Status rules

1. **`planned - not yet executed`** means the row has a target, an exact command, an
   expected observation and an evidence path, but no executed result and no independent
   verifier record. Every row starts in that state.
2. A row becomes **`executed - passed`** only when the exact command, exit code, tree state
   (branch, base commit, uncommitted paths), test summary, observed values and ignored
   evidence artifact are recorded together **and** an independent verifier returns PASS.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact, changed baseline byte or unverified claim. A failed row is never
   converted into a skip, an `#[ignore]`, a relaxed assertion, a narrower claim or a
   deleted test after the fact.
4. A test that cannot run is not acceptance evidence. A filter that matches zero tests is a
   failed row.
5. A row may not be satisfied by an existing M6 capability. The green M0–M6 suite is a
   regression precondition, never M7 acceptance.
6. The M0–M6 canonical artifacts are a hard non-regression precondition: snapshot payload
   bytes, state hashes, configuration hashes, movie bytes, durable-store bytes, trace bytes,
   experiment artifact bytes, both demo outputs and the M6 parity evidence stay
   byte-identical to the captured baseline. Any unexplained difference stops the task and
   is reported.
7. Acceptance is limited to the **deterministic synthetic backend** on the Linux acceptance
   platform. The Mega Drive backend appears only as local-mode `--rom` composition plus its
   capability report (M7-A18), so no Mega Drive rendering, audio, transport or parity
   behavior is claimed. No native window, no picture, no sound, no browser/JS frontend, no
   TLS, no streaming, no performance and no cross-platform runtime claim is made.
8. Implementation is not verification. Only the independent verifier execution produces
   `executed - passed`; the documentation owner records the executed results afterwards.
9. A row whose behavior changes during repair is recorded with the change and the repair
   round, never with the original text.

## 2. Scope, baseline and evidence

### 2.1 Checkout, tree state and baseline

- Tree: `/home/stealthc/projects/ChronoFork/.agents/worktrees/m7-gui`, branch
  `agent/m7-gui`, base and HEAD `93d47ed308874be0e8e7060641c238c7f6809e31`
  (= freshly fetched `origin/main`) plus the verified-but-uncommitted **M6 snapshot**
  copied from `agent/m6-agent-interfaces` (same base). M6 is a frozen baseline: no M7 task
  may change `chronofork-service`, `chronofork-transport`, `chronofork-core`,
  `chronofork-sessions`, `chronofork-backend-api` or a backend crate.
- Expected tree state at verification time (uncommitted M7 work on top of the M6 snapshot):
  - modified: `AGENTS.md`, `Cargo.lock`, `Cargo.toml`, `README.md`,
    `crates/chronofork-cli/Cargo.toml`, `crates/chronofork-cli/src/main.rs`,
    `crates/chronofork-cli/tests/session_storage.rs`, `docs/README.md`,
    `docs/adr/README.md`, `docs/architecture/overview.md`, `docs/quality/quality-gates.md`,
    `docs/requirements/functional.md`, `docs/roadmap/milestones.md`,
    `docs/roadmap/roadmap.md`. `AGENTS.md`, `README.md`,
    `crates/chronofork-cli/tests/session_storage.rs` and the `docs/` entries other than
    `docs/adr/README.md` are the M6 snapshot and must not gain M7 edits;
    `Cargo.toml`, `Cargo.lock`, `crates/chronofork-cli/Cargo.toml` and
    `crates/chronofork-cli/src/main.rs` carry both the M6 changes and the M7 additions;
    `docs/adr/README.md` carries the M6 `0012` line and exactly one M7 `0013` line. The
    documentation promotion task owns the `docs/` entries beyond `docs/adr/README.md`;
  - untracked: the M6 paths (`crates/chronofork-cli/src/agent/`,
    `crates/chronofork-cli/src/session.rs`, `crates/chronofork-cli/tests/m6.rs`,
    `crates/chronofork-service/`, `crates/chronofork-transport/`,
    `docs/adr/0012-agent-native-interfaces.md`,
    `docs/architecture/agent-interfaces.md`,
    `docs/quality/m6-agent-interfaces-acceptance.md`) plus the M7 paths
    (`crates/chronofork-gui/`, `crates/chronofork-cli/src/gui.rs`,
    `crates/chronofork-cli/tests/gui.rs`, `docs/adr/0013-human-facing-gui.md`,
    `docs/architecture/gui.md`, `docs/quality/m7-gui-acceptance.md`).
  No local commit, merge or publication is authorized; M7 is verified on that worktree and
  remains integration-pending.
- The **documentation promotion** (2026-11-17, after PASS4 and REVIEW5) adds M7 edits to
  the listed `docs/` paths (`docs/README.md`, `docs/adr/README.md`,
  `docs/architecture/overview.md`, `docs/architecture/gui.md`,
  `docs/quality/quality-gates.md`, `docs/requirements/functional.md`,
  `docs/roadmap/milestones.md`, `docs/roadmap/roadmap.md`), to `README.md` and to
  `AGENTS.md`, plus ADR-0013's status line and three durable scope statements that
  contradicted the delivered milestone (`docs/vision/non-goals.md`,
  `docs/vision/product-vision.md`, `docs/vision/scope.md`). It changes no code, test,
  manifest or lockfile byte and no acceptance evidence; the verified code tree is
  unchanged. The promotion therefore raises the uncommitted status set from the verified
  28 entries to 31 (the verified 28-entry M6 snapshot + M7 delta, plus these three
  vision-document corrections), and the 30 acceptance rows stay `executed - passed`.
- Environment for every Cargo command: `CARGO_HOME=/tmp/t7-cargo` and the **worktree's own**
  target directory, `CARGO_TARGET_DIR=/home/stealthc/projects/ChronoFork/.agents/worktrees/m7-gui/target`
  (or omit the variable, which resolves to the same worktree `target/`). Never point
  `CARGO_TARGET_DIR` at the parent checkout's `/home/stealthc/projects/ChronoFork/target`:
  that is a different, older build tree and mixing it would invalidate the baseline
  comparison.
- Baseline captured by the captain on 2026-11-17 before M7 work, recorded in the ignored
  `.agents/notes/m7/captain/PROVENANCE.md` and not re-derived by this plan (per-target
  counts in `.research/m7/baseline/targets.log`):
  - `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` exit 0 on the untouched tree:
    **67 targets (57 test binaries + 10 doc-test targets) / 800 passed / 0 failed /
    0 ignored**;
  - `chronofork demo` — 986 bytes, SHA-256
    `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` (3/3 identical);
  - `chronofork megadrive demo` — 1582 bytes, SHA-256
    `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996` (3/3 identical);
  - M6 remote smoke: `serve` + `api health`/`session.create`/`session.run`/
    `session.inspect`/`session.read_memory` succeeded, with `rendering=false` and
    `audio=false` in the descriptor.

### 2.2 Evidence

- Raw row, gate, demo, remote and audit evidence is written under the ignored
  `.agents/notes/m7/verify/` tree: the final PASS4 run is under `final2/`
  (`PASS4.md`, `rows/M7-A<n>.log`, `rows/process/`, `gate/check-sh.log`,
  `gate/per-target.log` and `gate/per-target.json`, `demos/`, `audit/`); earlier rounds
  are preserved under the same tree (`FAIL.md`, `SUMMARY.md`, `final/FAIL3.md`,
  `pass2/`, `pass4/`, `remote/`, and the round-1 `rows/`, `gate/`, `demos/`, `audit/`).
  The independent review reports are `.agents/notes/m7/review/REVIEW*.md`, with REVIEW5
  as the final PASS.
- The authoritative dated gate result belongs in [quality gates](quality-gates.md); this
  plan links to it and records row-level observations. The M7 block there is written by the
  documentation promotion task, not by the implementer and not by this plan.
- The verifier edits no implementation, test or documentation file; only ignored evidence
  is written.
- Each row's evidence path below is the file that must exist before the row can be
  promoted; an empty or missing log fails the row.

### 2.3 Test targets

| Target | Command | Package |
|---|---|---|
| Crate unit tests (model, render, pacing, presentation, control, channel, command) | `cargo test -p chronofork-gui --lib` | `chronofork-gui` |
| Model/pane/layout integration | `cargo test -p chronofork-gui --test model` | `chronofork-gui` |
| Command grammar, operation set, attach/detach flow and control gate | `cargo test -p chronofork-gui --test command` | `chronofork-gui` |
| Renderer determinism, ANSI/HTML equality and escaping | `cargo test -p chronofork-gui --test render` | `chronofork-gui` |
| Pacing and run loop with `ManualClock` | `cargo test -p chronofork-gui --test pacing` | `chronofork-gui` |
| Video/audio capability gating | `cargo test -p chronofork-gui --test presentation` | `chronofork-gui` |
| Handoff lease and control gate | `cargo test -p chronofork-gui --test control` | `chronofork-gui` |
| CLI process surface (local and remote) | `cargo test -p chronofork-cli --test gui` | `chronofork-cli` |
| M6 CLI process regression | `cargo test -p chronofork-cli --test m6` | `chronofork-cli` |
| M6 transport/service regression | `cargo test -p chronofork-service`, `cargo test -p chronofork-transport` | service, transport |
| Full gate | `./scripts/check.sh` | workspace |

The crate's test files are exactly `tests/{model,command,render,pacing,presentation,control}.rs`
plus `tests/gui.rs` in the CLI; a new integration test file is a new target and must be added
to this table before a row can use it.

### 2.4 Verification history

| Round | Date | Result |
|---|---|---|
| Plan authored (architect) | 2026-11-17 | 30 rows `planned - not yet executed`; no execution claimed |
| Independent verification round 1 | 2026-11-17 12:40–13:12 | **FAIL** — 26 rows passed, A6/A8/A29 failed, A30 promotion half pending; findings F1–F4 (detach field order, title spacing, help on a closed stdout, empty `events` fallback claim). Gate exit 0, 76 targets / 980 passed, demos identical. See `.agents/notes/m7/verify/FAIL.md`. |
| Repair round | 2026-11-17 | F1–F4 fixed; review raised N1–N7, A24 and L1/L2, all repaired (SystemClock/PanelState/FieldName wording, A15 comment, lib.rs narrative, harness `BrokenPipe`, A6/A14 targets, targeted A24 grep, exception owner/expiry). |
| Independent verification round 2 | 2026-11-17 13:27–13:45 | **FAIL3** — M7-A24's expected grep was false as written; three low residuals. Gate exit 0, 76 targets / 987 passed, demos identical. See `.agents/notes/m7/verify/final/FAIL3.md`. |
| Repair round | 2026-11-17 | A24 command made targeted (`none`), N1–N7/L2 closed, harness regression test added (100 KB early-exit child). |
| Independent verification final (PASS4) | 2026-11-17 13:50–13:54 | **PASS** — 30/30 rows executed and passing; gate exit 0, 76 targets / 988 passed / 0 failed / 0 ignored; demos 3/3 byte-identical; M6 non-regression (11 / 67 / 60); claims/links audit 72 links / 15 anchors / 0 problems. See `.agents/notes/m7/verify/final2/PASS4.md`. |
| Independent review final (REVIEW5) | 2026-11-17 | **PASS** — N1–N7, A24 and L2 confirmed; R1–R9/A6/A8/A11 and F1–F4 no regression; no new finding. See `.agents/notes/m7/review/REVIEW5.md`. |
| Documentation promotion | 2026-11-17 | **Done** by this task (t8): plan status, milestone/roadmap, requirements, indexes, architecture overview, GUI page, dated gate block and ADR status updated from the executed evidence; rows stay `executed - passed`. |

## 3. Row map

| Area | Rows |
|---|---|
| Crate, layer boundary and canonical operation surface | M7-A1 – M7-A4 |
| Model, mosaic/layout, panels | M7-A5 – M7-A8 |
| Deterministic renderers | M7-A9 – M7-A11 |
| Capability-gated video/audio pipeline | M7-A12 – M7-A13 |
| Pacing, run loop and `pulse` | M7-A14 – M7-A15 |
| Human/agent handoff | M7-A16 – M7-A17, M7-A22 |
| Local mode, CLI grammar and process determinism | M7-A18 – M7-A20 |
| Remote observation and read-only inputs | M7-A21, M7-A23 |
| Security boundary | M7-A24 |
| Non-regression, gate and demos | M7-A25 – M7-A28 |
| Claim audit, verification and promotion | M7-A29 – M7-A30 |

## 4. Acceptance rows

Every row below is **`executed - passed`**, promoted by the 2026-11-17 PASS4
verification (§2.4). Commands are the exact commands the verifier executed in the
acceptance worktree with `CARGO_HOME=/tmp/t7-cargo`; a package or test filter that matched
zero tests would have failed the row.

### M7-A1 — Crate shape, dependencies and layer boundary

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/Cargo.toml`, `Cargo.toml`, `Cargo.lock`.
- **Command:**
  `cargo tree -p chronofork-gui --edges normal`;
  `diff ../m6-agent-interfaces/Cargo.lock Cargo.lock`;
  `diff ../m6-agent-interfaces/Cargo.lock Cargo.lock | grep -c '^> \[\[package\]\]'`;
  `diff ../m6-agent-interfaces/Cargo.lock Cargo.lock | grep -E '^<|^> *source = ' || echo none`;
  `grep -n "chronofork-gui" Cargo.toml crates/chronofork-gui/Cargo.toml`;
  `grep -n "workspace = true" crates/chronofork-gui/Cargo.toml`; and
  `grep -rnE "chronofork_transport|chronofork_sessions|chronofork_backend_api|std::net|TcpStream|std::fs|std::env" crates/chronofork-gui/src || echo none`.
- **Expected observation:** the crate's normal dependency tree roots at exactly
  `chronofork-core` and `chronofork-service`; every other node is either a workspace crate or
  the pre-existing `sha2` closure those two already had (M6 baseline, not an M7 addition).
  The M7 delta measured against the M6 snapshot worktree is exactly **one** new lockfile
  package (`grep -c` prints `1`) — `[[package]] name = "chronofork-gui"`,
  `version = "0.1.0"`, `dependencies = ["chronofork-core", "chronofork-service"]` — plus the
  `chronofork-gui` entry added to the CLI package's dependency list once the CLI composes the
  command surface; the M6 `chronofork-service`/`chronofork-transport` entries are part of the
  frozen snapshot and are not M7 additions. The combined filter prints `none`: nothing is
  removed or changed (`^<` is empty, so no existing package version or dependency line
  changes) and no `source = "registry+…"` line is added, so **no registry or third-party
  package is added**. The root `Cargo.toml` gains exactly the `crates/chronofork-gui` member
  and the `chronofork-gui` workspace dependency; `crates/chronofork-gui/Cargo.toml` uses
  `*.workspace = true` dependencies and carries `[lints] workspace = true`, so
  `unsafe_code = "forbid"` and the
  `unwrap_used`/`expect_used`/`panic`/`todo`/`unimplemented` denials apply. The layer grep
  prints `none`; the crate compiles under
  `cargo clippy --workspace --all-targets -- -D warnings` with **no `#[allow]` in
  production code**. The crate's test code carries the repository's existing test-only
  exception — `#![cfg_attr(test, allow(clippy::expect_used))]` in library tests and
  `#[allow(clippy::expect_used)]` in test binaries, inherited from the existing crates —
  which is registered here as the M7 lint exception and is not a production relaxation.
  **Owner: M7. Revisit when** the harness/test style permits fallible assertions without
  `expect` (or a global lint relaxation lands), whichever comes first.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A1.log` (cargo tree output, the
  lockfile diff against the M6 snapshot with the package count and the additive-only
  filter, the manifest grep and the layer grep).

### M7-A2 — Canonical operation surface is exactly the documented set

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/tests/command.rs` (grammar executor) and
  `crates/chronofork-gui/src/view.rs` unit tests, with a recording channel.
- **Command:** `cargo test -p chronofork-gui --test command` and
  `cargo test -p chronofork-gui --lib view`.
- **Expected observation:** a scripted channel records every `op` name; `CommandSession::open`
  issues `capabilities` exactly once; a script covering every command (`create`, `list`,
  `attach 1`, `focus 1`, `layout mosaic`, `panel memory on`, `step 1`, `run 2`, `pulse 2`,
  `pause`, `handoff agent`, `handoff human`, `frame`, `html`, `detach`, `quit`) issues
  exactly `capabilities`, `session.create`, `session.list`, `session.inspect`,
  `session.read_memory` and four `session.run` calls with `steps=[1, 2, 3, 3]` (the `step`,
  the `run`, and one run per pulsed frame), every run naming session 1 and sending no
  `buttons` field; no `session.fork`/`session.snapshot`/`session.restore`/`session.close`/
  `session.compare`/`store.*`/`experiment.run` call exists; `detach`, `focus`, `layout`,
  `panel`, `pause`, `handoff`, `frame` and `html` issue **zero** calls; the CLI process is
  not involved.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A2.log`.

### M7-A3 — Strict view decoding

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/view.rs` unit tests.
- **Command:** `cargo test -p chronofork-gui --lib view`.
- **Expected observation:** every view decoder accepts the documented operation result and
  rejects, with a typed `GuiError::Protocol` and no panic, a missing field, a wrong JSON
  type, an unknown extra field and a malformed hex digest; a 64-hex `state_hash` is
  validated; a `session.inspect` result without `machine.registers` is rejected rather than
  defaulted.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A3.log`.

### M7-A4 — `session.create` yields `backend_id`, not a descriptor

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/tests/command.rs` (the `create` block) and
  `crates/chronofork-gui/src/view.rs` unit tests.
- **Command:** `cargo test -p chronofork-gui --test command create` and
  `cargo test -p chronofork-gui --lib view`.
- **Expected observation:** the `create` block contains exactly `session`, `backend_id`,
  `state`, `tick`, `state_hash`, `lineage_parent`, `lineage_origin_snapshot`,
  `lineage_depth`; the model's descriptor and capability flags stay empty until an
  `attach` performs `session.inspect`; a `session.create` response that (incorrectly)
  carries a `backend` object is not treated as a descriptor (the extra field is rejected
  as unknown by the strict decoder, and the test asserts the typed error).
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A4.log`.

### M7-A5 — Pane, mosaic, layout, focus and limits

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/model.rs`, `tests/model.rs` and the process-level
  `crates/chronofork-cli/tests/gui.rs`.
- **Command:** `cargo test -p chronofork-gui --test model pane`,
  `cargo test -p chronofork-gui --lib model` and
  `cargo test -p chronofork-cli --test gui layout`.
- **Expected observation:** panes are allocated lowest-free-first (1..=4); a fifth attach is
  a typed `limit_exceeded` and leaves the model unchanged; `focus` on an empty or
  out-of-range pane is `unknown_pane`; `detach` with no attachment is `not_attached`;
  `layout single` renders only the focused pane and `layout mosaic` renders up to four
  regions in row-major order with empty regions blank; a `detach` moves focus to the lowest
  attached pane id or to `none`; `generation` increments per effective attach and is
  unchanged by an idempotent re-attach of an already attached session. At the process level
  the `layout` block reports `layout=mosaic panes=2 focused=2`, the mosaic grid draws
  `pane 1 session 1 …` and `pane 2 session 2 …`, detaching pane 1 reports `focused=2 panes=1`
  and detaching pane 2 reports `focused=none panes=0`, and a `focus` on the detached pane
  exits 1 with `unknown_pane`.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A5.log`.

### M7-A6 — Attach/detach is non-perturbative and issues only reads

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/tests/command.rs` (operation set, hidden-panel frame
  equality and the visible-panel single-line difference) and
  `crates/chronofork-gui/tests/model.rs` (frame bytes and event log).
- **Command:** `cargo test -p chronofork-gui --test command attach_detach` and
  `cargo test -p chronofork-gui --test model attach_detach`.
- **Expected observation:** with a scripted channel, the `attach` line executed by
  `command.rs` issues `session.inspect` and, when `machine.memory_length > 0`, exactly one
  `session.read_memory` whose `length` never exceeds `MAX_MEMORY_PANEL_BYTES`; the `detach`
  line issues no call at all; the test hides the presentation `events` panel
  (`panel events off`) before rendering the before/after frames, so the comparison is over
  the canonical panels and, for an unchanged session view, the two frames are
  byte-identical; a second command test leaves `events` visible and proves the difference is
  exactly one line — the `events` line, `events 2 last=attach sequence=1` before the detach
  and `events 4 last=attach sequence=3` after the re-attach — so that log is the only
  by-design difference; the model test's event log records one `attach`, one `detach` and one
  `attach` with monotonic sequences 0, 1, 2, while the command test's log also includes the
  earlier `create` and `panel` events (`create`, `attach`, `panel`, `detach`, `attach`,
  sequences 0–4); the `attach`/`detach` blocks carry the documented fields in order and a new
  `generation` on the re-attach. The filter `--test command attach_detach` matches both
  `attach_detach_issues_no_call_and_a_reattach_renders_the_same_frame` and
  `attach_detach_changes_only_the_events_panel_line`.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A6.log` (runs
  `--test command attach_detach`, which matches both the hidden-panel and the
  visible-panel test, and `--test model attach_detach`).

### M7-A7 — Attach/detach is non-perturbative against a real service

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs` (local mode, real in-process
  `AppService`) plus recorded process evidence.
- **Command:** `cargo test -p chronofork-cli --test gui attach_detach`.
- **Expected observation:** two local-mode runs — `create`, `attach 1`, `panel events off`,
  `frame`, `quit` and `create`, `attach 1`, `panel events off`, `frame`, `detach`,
  `attach 1`, `frame`, `quit` — produce three grids; the first run's grid equals the second
  run's first grid and the second run's second grid, byte for byte (the `events` panel is
  hidden so the comparison covers the canonical panels; without `panel events off` only the
  `events` log line differs, because it records the attach/detach sequence by design). The
  `create` and `attach` blocks report `tick=0` and the canonical fresh-session state hash
  (`c79d7afa6d7f9786b1eac6ebba652fc7dba5889f7398febf54e3c6ac99be79f8`, the M6 smoke value)
  before and after the detach; the frames carry the pane line, the tick and the status line
  `panes=1 focused=1 tick=0 state=ready rendering=unavailable audio=unavailable`, but **no**
  `state_hash` — no frame line carries a digest. The test therefore asserts the hash from
  the `attach` block and its absence from the grid (the cli-engineer adjusts
  `attach_detach_moves_only_the_presentation_log` accordingly).
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A7.log` (runs
  `--test gui attach_detach`, matching `attach_detach_moves_only_the_presentation_log`).

### M7-A8 — Panel availability matrix

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/presentation.rs` (`panel_availability`, the single
  owner of the matrix), `crates/chronofork-gui/src/model.rs` (panel names and `parse`),
  `crates/chronofork-gui/src/render/text.rs` (the panel formatter, including the empty-log
  fallback test) and `tests/render.rs` (the rendered panel lines).
- **Command:** `cargo test -p chronofork-gui --lib panel`,
  `cargo test -p chronofork-gui --lib text` and
  `cargo test -p chronofork-gui --test render panel`.
- **Expected observation:** the eight panel names are exactly `status`, `lineage`,
  `capabilities`, `registers`, `memory`, `inspectors`, `timeline`, `events`; an unknown name
  is `unknown_panel`; with no attached pane the canonical panels `status`, `lineage`,
  `capabilities`, `registers`, `memory` and `inspectors` report `available=false
  reason=no_session`; `timeline` reports `available=false reason=no_movie` without `--movie`,
  with or without an attached pane; `events` is the presentation event log, **not** canonical
  data, so it stays `available=true` with no attached pane; its empty form
  `events 0 last=none sequence=none` is the formatter's fallback, not a rendered state (a
  pane exists only after an attach, and an attach always records at least the `attach`
  event), and it is covered by a direct unit test in `render/text.rs` under
  `cargo test -p chronofork-gui --lib text`; the `inspectors` panel reports
  `cpu=available` and `vdp`/`interrupts`/`bus`/`disassembly` as
  `unavailable:not_exposed_by_transport`; the memory panel never requests more than
  `MAX_MEMORY_PANEL_BYTES`; no panel renders a value that is not present in a decoded view —
  a visible but unavailable panel renders `<name> unavailable reason=<reason>`, never a
  placeholder.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A8.log` (runs `--lib panel`,
  `--lib text` and `--test render panel`).

### M7-A9 — Deterministic text renderer

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/render/text.rs` and `tests/render.rs`.
- **Command:** `cargo test -p chronofork-gui --test render text` and
  `cargo test -p chronofork-gui --lib render`.
- **Expected observation:** a frame is exactly `height` lines of exactly `width` printable
  ASCII columns, space-padded, `\n`-terminated, with no trailing blank line; two renders of
  the same model are byte-identical; the title line, separator line, pane header lines and
  status line appear at the documented rows; long values are clipped (never wrapped) and a
  non-printable or non-ASCII byte in a view renders as `?`; frames at `MIN_WIDTH`/
  `MIN_HEIGHT` and `MAX_WIDTH`/`MAX_HEIGHT` obey the same rules.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A9.log`.

### M7-A10 — ANSI renderer is emphasis only

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/render/text.rs` unit tests.
- **Command:** `cargo test -p chronofork-gui --lib ansi`.
- **Expected observation:** an ANSI frame contains only `ESC[1m` and `ESC[0m` SGR
  sequences — no cursor movement, no clear, no alternate screen, no terminal query; removing
  SGR from the ANSI frame yields the plain frame byte for byte; repeated renders are
  byte-identical; a pane header is bold and reset.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A10.log`.

### M7-A11 — Self-contained HTML renderer with total escaping

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/render/html.rs` and `tests/render.rs`.
- **Command:** `cargo test -p chronofork-gui --test render html`.
- **Expected observation:** the document starts with `<!DOCTYPE html>`, declares
  `charset=utf-8`, contains inline `<style>` and `<pre>` blocks, and contains **no**
  `<script`, no `on` event-handler attribute, no `http://`/`https://` reference, no
  `<iframe>`, no CSS `url(` and no external stylesheet. The adversarial descriptor carries
  **distinct** payloads in `id`, `display_name` and `version`; only `backend.id()` has a
  rendering path by contract (the `capabilities` panel line), so only the identifier reaches
  the grid and it appears in the document escaped exactly once (`&lt;script&gt;` count `1`),
  while `display_name`/`version` never reach the frame at all; the second test
  (`html_escapes_any_grid_content_a_view_could_carry`) proves that any grid content carrying
  those strings is escaped (`&lt;script&gt;`, `&quot;`, `&#39;`, `&amp;`) and control bytes
  render as `?`. `display_name`/`version` have no frame rendering path: showing them would
  be a new T4/CLI surface and would need a new contract. The `<pre>` content equals the
  plain frames after entity decoding; a document larger than `MAX_HTML_BYTES` is rejected
  with a typed `limit_exceeded` rather than truncated silently; two renders are
  byte-identical.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A11.log`.

### M7-A12 — Video pipeline is capability-gated and never fabricated

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/presentation.rs` (the pipeline),
  `crates/chronofork-gui/src/view.rs` (the bounded value types and contracts) and
  `tests/presentation.rs`.
- **Command:** `cargo test -p chronofork-gui --test presentation video` and
  `cargo test -p chronofork-gui --lib presentation`.
- **Expected observation:** with `rendering=false`, the video pipeline is
  `Unavailable{CapabilityDisabled}`, the injected `VideoSource` records zero calls and the
  frame status line reports `rendering=unavailable`; with a descriptor reporting
  `rendering=true`, the deterministic test-pattern source produces frames of the declared
  dimensions, the per-frame SHA-256 digest is stable across runs, a frame larger than
  `MAX_PRESENTATION_PIXELS` is a typed error, and the pipeline is `Available` (the process
  surface still reports `rendering=unavailable` because v1 configures no sink — asserted in
  the same test); no code path constructs a frame without a source.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A12.log`.

### M7-A13 — Audio pipeline is capability-gated and never fabricated

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/presentation.rs` (the pipeline),
  `crates/chronofork-gui/src/view.rs` (the bounded value types and contracts) and
  `tests/presentation.rs`.
- **Command:** `cargo test -p chronofork-gui --test presentation audio` and
  `cargo test -p chronofork-gui --lib presentation`.
- **Expected observation:** with `audio=false`, the audio pipeline is
  `Unavailable{CapabilityDisabled}`, the injected `AudioSink` records zero writes and the
  frame status line reports `audio=unavailable`; with `audio=true` and a recording sink,
  chunks are deterministic (identical bytes and sequence numbers across two runs), a chunk
  larger than `MAX_AUDIO_SAMPLES` is a typed error, and there is no resampling, mixing or
  device semantic anywhere in the crate; the process surface reports `audio=unavailable` in
  every configuration (no sink in v1).
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A13.log`.

### M7-A14 — Pacing and run loop with an injected clock

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/pacing.rs`, `crates/chronofork-gui/tests/pacing.rs`
  and `crates/chronofork-gui/tests/command.rs` (the pause gate on the run loop).
- **Command:** `cargo test -p chronofork-gui --test pacing`,
  `cargo test -p chronofork-gui --test command pause` and
  `cargo test -p chronofork-gui --lib pacing`.
- **Expected observation:** `PacerConfig` rejects `fps`, `ticks_per_frame` and
  `max_catch_up_frames` outside their documented ranges with a typed error;
  `pulse(n)` advances exactly `n` frames, `n * ticks_per_frame` canonical steps, and moves
  the manual clock by exactly `n * (1_000_000_000 / fps)` nanos, with `dropped = 0` and no
  partial frame; with `ManualClock`, `advance_elapsed` at chosen instants advances `due`
  frames capped at `max_catch_up_frames` and reports the remainder as `dropped`; a clock
  that does not move advances zero frames; while the presentation pause flag is set,
  `CommandSession::advance_elapsed` pins the clock reading to `pacer.last_nanos()` and
  reports zero due/advanced/dropped/steps, advances no frame, issues no `session.run` and
  does not consume the elapsed time, while the explicit `pulse` path still advances the
  requested frames (the resumed loop then sees the accumulated time as due, capped by
  `max_catch_up_frames` with the remainder dropped) — covered by
  `pause_gates_the_host_time_loop_but_not_pulse` in `crates/chronofork-gui/tests/command.rs`,
  which the `--test command pause` filter matches; `SystemClock::now_nanos` is monotonic
  non-decreasing across two calls; no host time value is ever stored in the model.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A14.log` (runs `--test pacing`,
  `--test command pause` — matching `pause_gates_the_host_time_loop_but_not_pulse` — and
  `--lib pacing`).

### M7-A15 — `pulse` exposes the cadence deterministically

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs` and recorded process evidence.
- **Command:** `cargo test -p chronofork-cli --test gui pulse` and
  `printf 'create\nattach 1\npulse 3\npulse 1\nquit\n' | CARGO_HOME=/tmp/t7-cargo cargo run -q -p chronofork-cli -- gui run --fps 50`
- **Expected observation:** the two `pulse` blocks print the documented fields in order
  (`pane`, `session`, `frames`, `frame`, `tick`, `state_hash`, `halted`, `ticks_per_frame`,
  `fps`, `elapsed_nanos`, `controller`) and contain, in order, `frames=3 frame=3
  tick=3 ticks_per_frame=1 fps=50 elapsed_nanos=60000000` and `frames=1 frame=4 tick=4
  ticks_per_frame=1 fps=50 elapsed_nanos=80000000`, with `halted=false` and
  `controller=human`; the process exits 0; running the same stdin twice yields
  byte-identical stdout; the `quit` block reports exactly `reason=line`, `controller=human`,
  `generation=0` and `sequence=0` (the `frame` index belongs to the `pulse` block, not to
  `quit`).
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A15.log` plus the two recorded
  runs `.agents/notes/m7/verify/final2/rows/process/A15-run1.out` and `A15-run2.out`
  (byte-identical stdout, empty stderr).

### M7-A16 — Handoff lease state machine

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-gui/src/control.rs` and `tests/control.rs`.
- **Command:** `cargo test -p chronofork-gui --test control` and
  `cargo test -p chronofork-gui --lib control`.
- **Expected observation:** the initial state is `Human` with `generation=0 sequence=0`;
  `handoff agent` yields `changed=true` and increments both counters; `handoff agent` again
  yields `changed=false` with unchanged counters; `handoff human` increments both and
  restores control; `step`/`run`/`pulse` under `Agent` return `GuiError::ControlDenied`
  carrying the current generation and sequence, issue **no** operation, and leave the
  rendered frame unchanged once the `events` panel is hidden — the denial attempt itself is
  recorded as its `step`/`pulse` event, which is the only model change; observation
  commands never consult the lease; the event log records every effective transition and
  every denial attempt with monotonic sequences; no host time is recorded.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A16.log`.

### M7-A17 — Handoff process flow H→A→H in local mode

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs`.
- **Command:** `cargo test -p chronofork-cli --test gui handoff`.
- **Expected observation:** one run with `create`, `attach 1`, `handoff agent`,
  `handoff human`, `step`, `quit` yields handoff blocks
  `controller=agent previous=human changed=true generation=1 sequence=1` then
  `controller=human previous=agent changed=true generation=2 sequence=2`, a `step` block
  with `requested=1 steps=1 tick=1 controller=human`, and a `quit` block with
  `controller=human generation=2 sequence=2`; a second run with `create`, `attach 1`,
  `handoff agent`, `step`, `quit` exits 1 with exactly one stderr line starting
  `chronofork: gui: control_denied:`, no block after `handoff`, and the process stops; a
  third run with `create`, `attach 1`, `handoff agent`, `frame`, `html`, `quit` succeeds,
  printing one grid and one document while `Agent` holds the lease (neither adds a block).
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A17.log` plus
  `.agents/notes/m7/verify/final2/rows/process/A17-hah.out` (the H→A→H run) and
  `A17-denied.out`/`A17-denied.err` (the denied-step run).

### M7-A18 — Local mode requires `create` and selects the backend honestly

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs`.
- **Command:** `cargo test -p chronofork-cli --test gui local` and the recorded
  `--rom` invocation in the same log.
- **Expected observation:** `attach 1` in a fresh local process exits 1 with exactly one
  stderr line that is the canonical `chronofork.error.v1` envelope carrying
  `"code":"not_found"` (an operation failure, not a GUI protocol code) and empty stdout;
  `create` then `attach 1` succeeds and reports the canonical descriptor id
  `backend=chronofork.test.synthetic` (`chronofork_test_backend::BACKEND_ID`); with
  `--rom <FILE>` built from the independently authored `demo_rom()` bytes, `create` +
  `attach` reports `backend=chronofork.megadrive.m68000`
  (`chronofork_megadrive::BACKEND_ID`), `test_backend=false`, `rendering=false` and
  `audio=false`; the block carries the descriptor id verbatim — there is no short-name
  table, so a second source of backend names never exists; local mode reads no token file —
  `--token-file` without `--remote` is a typed `bad_value`; the `--rom` run makes no
  hardware, rendering or audio claim.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A18.log`,
  `.agents/notes/m7/verify/final2/rows/M7-A18-rom.log` and
  `.agents/notes/m7/verify/final2/rows/process/A18-rom.out` /
  `A18-token-without-remote.out` (the `bad_value` local-token run).

### M7-A19 — CLI grammar, help and exit codes

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs` and recorded process evidence.
- **Command:** `cargo test -p chronofork-cli --test gui grammar` plus
  `CARGO_HOME=/tmp/t7-cargo cargo run -q -p chronofork-cli -- gui --help`,
  `... -- gui run --help`, `... -- gui run --width 39`, `... -- gui run --height 101`,
  `... -- gui run --fps 0`, `... -- gui run --fps 50 --fps 60`, `... -- gui run --bogus`,
  `... -- gui run --remote http://127.0.0.1:1` (without `--token-file`),
  `... -- gui run --remote http://127.0.0.1:1 --token-file <valid-file>` (closed port) and
  `printf 'bogus\n' | ... -- gui run`.
- **Expected observation:** both `--help` forms exit 0, print usage to stdout, list every
  documented flag and every command name, and print nothing to stderr; an out-of-range
  `--width`/`--height`/`--fps`, a repeated flag, an unknown flag, a missing `--token-file`
  with `--remote` and `--rom` together with `--remote` each exit 1 with empty stdout and
  exactly one `chronofork: gui: bad_value:` stderr line; a remote run against a closed port
  with a valid token file exits 1 with `chronofork: gui: channel:` and empty stdout; the
  line failures map to one documented code each — `bogus` → `unknown_command`, `step abc` →
  `bad_value`, `step 1 2` → `bad_arity`, `focus 9` → `unknown_pane`, `panel bogus on` →
  `unknown_panel`, `layout bogus` → `bad_value`, `detach` with no attachment →
  `not_attached`, `pulse 601` → `limit_exceeded` — each with exit 1, empty stdout and one
  stderr line; `quit` exits 0 with `reason=line`, and empty input exits 0 with exactly one
  `quit` block and `reason=eof`.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A19.log`.

### M7-A20 — Process-level determinism, local and remote

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs`.
- **Command:** `cargo test -p chronofork-cli --test gui deterministic`.
- **Expected observation:** for a fixed stdin script, two runs of `gui run` produce
  byte-identical stdout and empty stderr, in local mode and against a fresh daemon in remote
  mode (two fresh daemons produce byte-identical remote stdout, and the daemon's stderr is
  empty); the ANSI run differs from the plain run only by SGR sequences; the HTML output is
  byte-identical across runs, whether printed or written with `--html`; no block or frame
  contains a host path, a token, a URL/daemon address or a host time.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A20.log`.

### M7-A21 — Remote observation over the real M6 daemon

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs` (`spawn_serve` harness, the same pattern
  as `tests/m6.rs`).
- **Command:** `cargo test -p chronofork-cli --test gui remote_observe`.
- **Expected observation:** `chronofork serve --token-file <FILE> --bind 127.0.0.1:0`
  publishes `bind=127.0.0.1:<port>`; `gui run --remote <URL> --token-file <FILE>` with
  `list`, `attach 1`, `frame`, `html`, `quit` succeeds, and its `list`/`attach` values equal
  the values returned by a separate `chronofork api call --op session.inspect` for the same
  session (tick `0`, the same state hash, the same backend id, `rendering=false`,
  `audio=false`); the GUI issues no mutating operation in this scenario — the blocks are
  exactly `list`, `attach`, `quit` — and a second `session.inspect` after the run reports
  the same tick and state hash, which is the observable equivalent of "the daemon saw only
  reads" (the M6 transport keeps no per-request log, so no log claim is made).
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A21.log` (the test spawns the
  real daemon and records the observed values; no separate remote artifact was kept).

### M7-A22 — Remote handoff with a second process as the agent

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs` plus recorded process evidence.
- **Command:** `cargo test -p chronofork-cli --test gui remote_handoff`.
- **Expected observation:** with the daemon running and session 1 created, a first GUI run
  with `attach 1`, `handoff agent`, `step`, `quit` observes `attach.tick=0`, takes the lease
  as the agent (`generation=1 sequence=1`) and exits 1 with
  `chronofork: gui: control_denied:` when the denied `step` is reached, so the blocks are
  exactly `attach` and `handoff`; the **second process** (the agent) is
  `chronofork api call --url <URL> --token-file <FILE> --op session.run --input '{"session":1,"steps":5}'`,
  which returns `tick=5`; a **second GUI run** with `attach 1`, `frame`, `quit` issues a
  fresh `session.inspect` (a new process, so the attach is not the idempotent in-window
  re-attach) and therefore observes `attach.tick=5` and the post-run `session.inspect`
  state hash, with the frame showing `tick=5`; a third GUI run with `attach 1`,
  `handoff agent`, `handoff human`, `step`, `quit` hands control back in the same process and
  the human `step` executes with `tick=6`, confirmed by a following `api call` inspect. The
  model deliberately does not re-inspect on `frame`; the new process is the documented
  observation path.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A22.log` (the test spawns the
  daemon and runs the second process as the agent; no separate remote artifact was kept).

### M7-A23 — Read-only `.cfmovie` timeline

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs` and recorded process evidence.
- **Command:** `cargo test -p chronofork-cli --test gui timeline` and the recorded
  `--movie` invocation in the same log.
- **Expected observation:** with a `.cfmovie` produced by the existing recorder
  (`chronofork tas record --movie <FILE> --steps 5`), the script `create`, `attach 1`,
  `panel timeline on`, `frame`, `quit` under `gui run --movie <FILE>` reports the `panel`
  block as `panel=timeline visible=true available=true reason=none`, and the frame's
  timeline line is `timeline version=<format_version> steps=<step_count>
  end_tick=<end_tick> anchors=<anchor count>` with the movie's own values; the movie file's
  bytes and SHA-256 are unchanged after the run; without `--movie` the `panel` block is
  `available=false reason=no_movie` and the frame line is
  `timeline unavailable reason=no_movie`; a truncated movie exits 1 with a typed `movie`
  error and empty stdout.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A23.log`.

### M7-A24 — Token and security boundary

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/gui.rs` and a source audit.
- **Command:** `cargo test -p chronofork-cli --test gui token` and
  `grep -rnE "TokenTable|Authorization|Bearer[[:space:]]+[A-Za-z0-9]|token_file|read_token_table|chronofork_service::auth" crates/chronofork-gui/src || echo none`.
- **Expected observation:** the crate grep prints `none`: the crate contains no token
  table, no `Authorization`/`Bearer <credential>` material, no `token_file` path and no
  service auth import. Prose that uses the word "token" (`tokenizes`, "no token",
  `token-free`, `bearer-token`, "token file") is expected — it is documentation, not token
  material — so the check deliberately matches token *types and readers*, not the word; the
  `--test gui token` reader-token process test is the behavioural proof. In remote mode a token file whose
  first principal is read-only still lets the observation commands succeed — `list` (which
  sees only that principal's own sessions, here `sessions=0`), `frame` and `html` need no
  session at all — while the mutating and ownership-bound paths fail closed with the
  canonical `forbidden` envelope and empty stdout: `create` needs the `write` scope, and
  `attach 1` of a session owned by another principal is refused by the M6 ownership rule
  (not by a scope check), so no `step` ever runs. No token string or token-file path appears
  in stdout, stderr or the HTML document of any of those runs (checked by grepping the
  recorded outputs). The token file is read bounded: an oversized file is a typed
  `limit_exceeded` before buffering, a malformed file is a typed `bad_value`, and local mode
  reads no token file at all (`--token-file` without `--remote` is `bad_value`).
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A24.log` (records the targeted
  `TokenTable|Authorization|Bearer <cred>|token_file|read_token_table|chronofork_service::auth`
  grep returning `none` and the `--test gui token` reader-token test) plus
  `.agents/notes/m7/verify/final2/audit/probe-a24/` (`reader-tokens.json`,
  `gui-create.out/.err`, `gui-attach.out/.err`, `gui-reader.out/.err`).

### M7-A25 — Existing M6 surfaces still pass

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `crates/chronofork-cli/tests/m6.rs`, `crates/chronofork-service`,
  `crates/chronofork-transport`.
- **Command:** `cargo test -p chronofork-cli --test m6`,
  `cargo test -p chronofork-service`, `cargo test -p chronofork-transport`.
- **Expected observation:** all three commands exit 0 with the M6 baseline counts
  unchanged — cli `--test m6` 11 passed, `chronofork-service` 67 (45 lib + 22
  `--test service`), `chronofork-transport` 60 (9 lib + 10 client + 21 http + 17 mcp + 3
  parity), exactly as recorded per target in `.research/m7/baseline/targets.log`; no M6
  test is renamed, removed, ignored or relaxed; `chronofork serve`/`api`/`mcp`/`experiment`
  behavior is unchanged.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A25.log`.

### M7-A26 — Full workspace gate

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `scripts/check.sh`.
- **Command:** `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh`
- **Expected observation:** exit 0; `fmt`, `check`, `clippy -D warnings`, `test
  --workspace` and `doc -D warnings` all pass; the target count is **at least** the
  baseline 67 targets and the passed count **at least** 800, with the new
  `chronofork-gui` targets listed; no test target is missing relative to the baseline; no
  `#[allow]`, `#[ignore]` or lint relaxation was added to **production** code. The only
  added allowances are the test-only `expect_used` exception described in M7-A1, the pattern
  every existing crate already uses. **Registered duplication (owner M7):** the CLI's
  `gui.rs` reimplements the bounded-file/token-file reader (`read_bounded`,
  `read_token_table`) instead of sharing the agent helpers, keeping the GUI diagnostics
  path-free. Revisit when the frozen M6 snapshot helper can be shared or a common fallible
  reader is extracted; until then the bound and parser rule are duplicated by decision, not
  by accident.
- **Evidence:** `.agents/notes/m7/verify/final2/gate/check-sh.log`,
  `.agents/notes/m7/verify/final2/gate/per-target.log`,
  `.agents/notes/m7/verify/final2/gate/per-target.json` and
  `.agents/notes/m7/verify/final2/rows/M7-A26.log`.

### M7-A27 — Demos are byte-identical to the baseline

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `chronofork demo`, `chronofork megadrive demo`.
- **Command:** three runs each of
  `CARGO_HOME=/tmp/t7-cargo cargo run -q -p chronofork-cli -- demo` and
  `... -- megadrive demo`, with `sha256sum` per run.
- **Expected observation:** the synthetic demo is 986 bytes with SHA-256
  `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` on all three runs; the
  Mega Drive demo is 1582 bytes with SHA-256
  `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996` on all three runs;
  stderr is empty in every run.
- **Evidence:** `.agents/notes/m7/verify/final2/demos/synthetic-*.out`,
  `.agents/notes/m7/verify/final2/demos/megadrive-*.out`,
  `.agents/notes/m7/verify/final2/demos/sha256.txt` and the recorded
  `.agents/notes/m7/verify/final2/rows/process/A27-*.out` runs.

### M7-A28 — Frozen layers are untouched

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** the ten frozen M6 snapshot crates.
- **Command:** `git status --short`;
  `git diff --stat -- crates/chronofork-core crates/chronofork-backend-api crates/chronofork-sessions crates/chronofork-service crates/chronofork-transport crates/chronofork-debug crates/chronofork-trace crates/chronofork-script crates/chronofork-test-backend crates/chronofork-megadrive`;
  and, one crate at a time against the M6 snapshot worktree,
  `for c in core backend-api sessions service transport debug trace script test-backend megadrive; do diff -r --exclude=target "../m6-agent-interfaces/crates/chronofork-$c" "crates/chronofork-$c" || echo "DIFF chronofork-$c"; done`.
- **Expected observation:** the `git diff --stat` for the frozen crates prints nothing, and
  the per-crate loop prints **nothing at all** — no `diff` output and no `DIFF` line — for
  all ten crates (`core`, `backend-api`, `sessions`, `service`, `transport`, `debug`,
  `trace`, `script`, `test-backend`, `megadrive`). The new `crates/chronofork-gui/` is
  deliberately **not** in that loop: it is the M7 delta, not a frozen layer. The M7 delta is
  confined to `crates/chronofork-gui/`, `crates/chronofork-cli/src/gui.rs`,
  `crates/chronofork-cli/tests/gui.rs`, `crates/chronofork-cli/src/main.rs`,
  `crates/chronofork-cli/Cargo.toml`, `Cargo.toml`, `Cargo.lock` and the four M7
  documentation paths (`docs/adr/0013-human-facing-gui.md`, `docs/architecture/gui.md`,
  `docs/quality/m7-gui-acceptance.md`, `docs/adr/README.md`); no canonical artifact byte
  changed.
- **Evidence:** `.agents/notes/m7/verify/final2/rows/M7-A28.log`.

### M7-A29 — Claim and documentation audit

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17).
- **Target:** `docs/adr/0013-human-facing-gui.md`, `docs/architecture/gui.md`,
  `docs/quality/m7-gui-acceptance.md`, `docs/adr/README.md`.
- **Command:** a link-resolution check over the four files plus
  `grep -rnE "implemented|passes|identical|supported|renders|audio output|native window" docs/adr/0013-human-facing-gui.md docs/architecture/gui.md`
  reviewed line by line against the executed rows.
- **Expected observation:** every relative link resolves to an existing file and anchor;
  the documents claim no rendering, audio, native window, browser frontend, streaming or
  performance capability; the `Unavailable` paths are stated with their reasons; the
  cooperative (not enforcing) nature of the controller lease is stated; `M6-FU1` is named
  where the inspector content is missing; every "implemented/passes/identical" sentence is
  backed by an `executed - passed` row or is explicitly marked planned; the acceptance plan
  contained no invented result and every row still said `planned - not yet executed` when
  the audit ran — the later PASS4 promotion and this dated record are what changed that,
  not the row text; `docs/adr/README.md` contains exactly one new line for ADR-0013 (now
  carrying the implemented/verified status).
- **Evidence:** `.agents/notes/m7/verify/final2/audit/links.txt` (the PASS4 snapshot:
  72 links / 15 with anchor / 0 problems), `.agents/notes/m7/verify/final2/audit/claims4.md`
  and `.agents/notes/m7/verify/final2/rows/M7-A29.log`; the same checker re-runs clean on
  the post-promotion tree (0 problems — the link count grew because the promotion added
  links, none broken).

### M7-A30 — Independent verification and documentation promotion

- **Status:** `executed - passed` (independent verification PASS4, 2026-11-17). The
  verifier half is PASS4; the promotion half was executed by the documentation task
  (t8) on 2026-11-17 and is recorded in this page's status, §2.4, the milestone/milestone
  table, the requirement statuses and the dated M7 block in [quality gates](quality-gates.md).
- **Target:** the whole plan and the promotion set.
- **Command:** the independent verifier re-executes M7-A1–M7-A29 and records the verdict;
  the documentation task then runs the promotion greps:
  `grep -n "FR-GUI-001" docs/requirements/functional.md`,
  `grep -n "M7" docs/roadmap/milestones.md docs/roadmap/roadmap.md docs/README.md docs/architecture/overview.md docs/quality/quality-gates.md`,
  `grep -n "0013" docs/adr/README.md`.
- **Expected observation:** the verifier record states PASS with the tree state, gate
  counts, demo digests and per-row outcomes, or FAIL with findings; promotion updates the
  documentation index, overview, roadmap, milestone, requirement status (FR-GUI-001 and
  FR-EXEC-007), quality gates (dated M7 block) and the ADR index line, and every promoted
  sentence is traceable to an `executed - passed` row; the milestone and requirement
  promotion is consistent with what was actually verified and states the integration
  status (verified on the worktree, integration pending).
- **Evidence:** `.agents/notes/m7/verify/final2/PASS4.md` (verifier half),
  `.agents/notes/m7/review/REVIEW5.md` (independent review PASS), the dated
  [2026-11-17 M7 block](quality-gates.md#2026-11-17--m7-human-facing-gui-independent-final-verification-pass4)
  and this page's §2.4 verification history (promotion half); the pre-promotion greps
  are preserved in `.agents/notes/m7/verify/final2/audit/promotion.txt` and the earlier
  round-1 record in `.agents/notes/m7/verify/SUMMARY.md`.

## 5. Non-goals of this plan

No native toolkit window, no browser/JS frontend, no streaming transport, no live audio
output, no GUI input injection, no VDP/bus/disassembly/trace panel content, no rendering or
audio semantics, no Mega Drive hardware behavior, no performance claim and no cross-platform
runtime evidence. Those are follow-ups (native frontend after H2/H3; inspector content after
M6-FU1) and they do not reopen M7.
