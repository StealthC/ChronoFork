# AGENTS.md

Authoritative instructions for coding agents working on ChronoFork. This file is
harness-independent: it must remain useful with DeepSeek Harness, Codex, Claude
Code, OpenCode, or any other coding agent. Read it before changing the repository.

For any write task: establish the correct checkout first, coordinate file ownership,
then plan, implement and verify. A task team normally shares one worktree; independent
concurrent write tasks use separate worktrees. Publication and integration have separate
authorization requirements (see [Git authorization](#git-authorization-and-integration)).

Work is tracked through the `project-orchestration` lifecycle: the committed
[`docs/planning/`](docs/planning/ROADMAP.md) directory holds the current plan and the session
handoff, GitHub Issues and PRs are the trackable projection, and
[Lifecycle and planning](#lifecycle-and-planning) is the index of rules. Read the project
state before starting or resuming any task.

## Project purpose

ChronoFork is an open-source, cross-platform emulation and reverse-engineering
platform built primarily in Rust. It treats emulated execution as deterministic,
independent, forkable sessions that humans, scripts and AI agents can inspect and
replay.

**Current status:** bootstrap + partial real-hardware slice. Milestones M0–M7 are
complete and merged to `main` (M4 merged at `4c595a2`, verified on
`agent/m4-debugging-depth` at `6d30243`; M5 merged at `0c4e9f0`, PR #4, verified on
`agent/m5-tracing-scripting` at base `84f1476`; M6 and M7 merged at `cb05836`, PR #6,
verified on `agent/m6-agent-interfaces` and `agent/m7-gui` at base `93d47ed`); and the
hardware-depth tracks H1–H5 are partial or planned. The repository contains a
deterministic synthetic test backend and a minimal Mega Drive backend with the
documented MC68000 instruction set, ROM/RAM, a complete VDP (CRAM/VSRAM, plane/window/sprite
composition, DMA, FIFO/CPU wait accounting, line/dot timing and VDP-owned VBlank/HBlank
requests) and a deterministic master-cycle VBlank/HBlank interrupt scheduler. The slice includes 68000 SR-mask
interrupt gating (level 7 always admitted, other levels only when strictly above the
mask), `RTE` handler return, VDP VINT pending/acknowledge and per-step controller reads
of `0x00A10003`/`0x00A10005` for the TH-high 3-button subset on ports 1–2. Pinned-oracle
differential evidence covers eleven synthetic ROMs at 11/11 address, per-instruction
cycle and selected-final-state comparisons. A host
debugger (`chronofork-debug`, exposed as `chronofork debug`, M4) adds breakpoints,
byte-granular watchpoints, event breakpoints/logpoints and bounded inspection views.
The M5 host layers add structured tracing (`chronofork-trace`, `chronofork trace`) and a
sandboxed deterministic script runtime (`chronofork-script`, `chronofork script`).
The M6 agent interfaces add a transport-agnostic service layer (`chronofork-service`) and
hand-rolled bounded HTTP/1.1 + MCP adapters with a typed client (`chronofork-transport`),
exposed by `chronofork serve`, `chronofork api`, `chronofork mcp` and
`chronofork experiment run`, with canonical machine-consumable experiment artifacts and
byte-level direct/HTTP/MCP parity. M6 was verified on
`agent/m6-agent-interfaces` at base `93d47ed` and is merged to `main` at `cb05836` (PR #6);
there is **no TLS**, no SSE/streaming, no keep-alive or chunked bodies,
and debug/trace/script/TAS operations are not exposed over transport (those remain CLI-local
and are tracked as M6-FU1–M6-FU4).
The M7 GUI adds a presentation-only `chronofork-gui` crate (strict view decoders, pane/
mosaic model, eight-panel matrix, deterministic plain/ANSI and self-contained HTML
renderers, `HostClock`/`ManualClock` pacing with a host-time run loop, capability-gated
video/audio pipelines and a cooperative human/agent controller lease) and the
`chronofork gui run` stdin-driven process surface in local and remote modes, with a
read-only `.cfmovie` timeline and attach/detach proven non-perturbative. M7 was verified on
`agent/m7-gui` at base `93d47ed` (PASS4 + review REVIEW5 PASS,
2026-11-17) and is merged to `main` at `cb05836` (PR #6). v1 presents
**text and a static HTML document only** — no native toolkit window, no picture, no sound;
real video/audio sources and sinks remain H2/H3, so the pipeline honestly reports both as
unavailable in every v1 process configuration, and no CLI process builds the host clock
(playback is the deterministic `pulse`).
Audio, Z80, timer interrupts, ROM header/region detection and
cartridge mapping, the rest of the controller protocol (A/Start, TH/6-button, port 3,
analog/keyboard), guest bus-error/trace delivery, other exception groups and cycle
accuracy remain absent. The documented MC68000 instruction set and privilege
enforcement for the implemented system instructions are present. Do not present either
backend as a complete emulator or claim game compatibility.

**M8 (functional Mega Drive emulator) is the active milestone.** Phases 8.2 (complete
68000 and cycle model) and 8.3 (complete VDP) are complete and merged; phases 8.4–8.8 are planned in
[docs/planning/ROADMAP.md](docs/planning/ROADMAP.md); M0–M7 are complete and merged. M8 completes the Mega Drive backend — the full 68000 and Z80, a complete VDP with
rendering/DMA/timing, YM2612/PSG audio, cartridge/TMSS/mapper/SRAM, timers and controllers —
with cycle-based canonical time, deterministic headless capture of frames and audio and an
optional native window. The former multi-backend milestone is M9. Do not treat "active" as
merged or implemented, and keep the limitations above until each phase is verified by executed
evidence. Lifecycle state and the next action live in
[docs/planning/](docs/planning/ROADMAP.md) and [docs/planning/STATE.md](docs/planning/STATE.md).

## Engineering responsibility and technical judgment

The project owner defines the product's goals, priorities, constraints and strategic direction but is not necessarily a domain expert in every technical area involved in the project.

Agents are expected to exercise independent engineering judgment rather than blindly implement instructions or follow existing plans.

When an existing decision, requested implementation or planned feature appears technically unsound, unnecessarily complex, inefficient, unsafe or inconsistent with the project's long-term objectives, agents must raise the concern and provide evidence-based alternatives.

Agents must not assume that previously approved ADRs, completed milestones, passing tests or existing implementations are inherently correct.

Prefer established engineering practices, mature solutions, primary technical references, reproducible experiments and measured evidence over speculative abstractions or implementation convenience.

For consequential decisions requiring owner approval, explain the problem, realistic alternatives, trade-offs, recommended approach, risks and validation criteria in accessible language. Do not delegate specialized technical choices to the owner without providing a justified recommendation.

Optimize for correctness, maintainability, practical usefulness and long-term project health, rather than merely completing tasks.

The purpose of agent autonomy is to improve engineering outcomes, not to bypass technical accountability or owner governance.

## Repository map

| Path | Purpose |
|---|---|
| `crates/chronofork-core/` | IDs, virtual ticks, input, SHA-256 digests |
| `crates/chronofork-backend-api/` | Backend, capability, snapshot, inspection traits |
| `crates/chronofork-sessions/` | Session lifecycle, checkpoints, lineage, forks |
| `crates/chronofork-test-backend/` | Synthetic deterministic validation backend |
| `crates/chronofork-megadrive/` | Mega Drive slice: 68000 + ROM/RAM + cartridge + complete VDP (rendering/DMA/timing/interrupts) + interrupt scheduler + controller reads; debug/disassembly/inspector views |
| `crates/chronofork-debug/` | Host debugger layer: breakpoints, watchpoints, event breakpoints/logpoints |
| `crates/chronofork-trace/` | Host trace layer: canonical `CFTRACE1` v1 format, capture/triggers/ring, queries, comparison and TSV export |
| `crates/chronofork-script/` | Sandboxed deterministic script DSL: lexer/parser/evaluator, limits, sandbox, session/trace/TAS/report bindings |
| `crates/chronofork-service/` | Transport-agnostic application service: typed 16-operation catalog, canonical JSON codec, token/scope authentication, session ownership, concurrency boundary (manager lock + per-session writer lease), experiment artifacts |
| `crates/chronofork-transport/` | Hand-rolled bounded HTTP/1.1 server, stateless MCP JSON-RPC endpoint over HTTP and a typed client; the only crate that knows HTTP/MCP framing |
| `crates/chronofork-gui/` | Presentation-only GUI layer: command grammar/executor, strict view decoders, pane/mosaic model, panels, plain/ANSI + HTML renderers, `HostClock` pacing/run loop, capability-gated video/audio pipelines and the human/agent controller lease |
| `crates/chronofork-cli/` | Minimal CLI demonstrating implemented features (`demo`, `megadrive`, `backend`, `session`, `tas`, `debug`, `trace`, `script`, `serve`, `api`, `mcp`, `experiment`, `gui`) |
| `docs/` | Durable official documentation |
| `docs/README.md` | Documentation index; start here |
| `docs/planning/` | Committed lifecycle state — `ROADMAP.md`, `MILESTONE.md`, `STATE.md`, `CONVENTIONS.md` — maintained by `project-orchestration` |
| `docs/superpowers/` | Approved design specs (`specs/`) and phase plans (`plans/`) produced by the lifecycle skills |
| `docs/adr/` | Architectural decisions and status |
| `docs/epics/` | EPIC-0001 foundation and EPIC-0002 Mega Drive slice |
| `docs/research/` | Evidence-backed upstream research |
| `scripts/check.sh` | Local quality gate |
| `skills-lock.json` | Pinned agent-skill lockfile; `.agents/skills/` is ignored and restored from it |
| `.planning/` | Ignored, worktree-local scratch/evidence for the optional `planning-with-files` skill; never the lifecycle source of truth |
| `.agents/notes/` | Ignored per-agent, per-task transient notes (when used) |
| `.agents/worktrees/` | Preferred ignored location for task worktrees (when used) |
| `.research/upstream/` | Ignored upstream clones; never commit |
| `.research/roms/` | Ignored local-only commercial ROM fixtures for M8; never committed, shared across worktrees by absolute path |

## Technology stack

- Rust edition 2024, stable toolchain (`rust-toolchain.toml`).
- Cargo workspace, `resolver = "3"`.
- Direct dependency: `sha2` (MIT OR Apache-2.0). Keep dependencies minimal.
- `rustfmt` and `clippy` are required.
- No `unsafe` (workspace lint forbids it).
- No third-party GUI or database dependency. HTTP and MCP are **hand-rolled** over
  `std::net`/`std::thread` in `chronofork-transport` (HTTP/1.1 and MCP JSON-RPC subsets) with
  no third-party HTTP, async-runtime or serialization dependency; M6 added no new
  third-party dependency. The M7 GUI renders to plain text/ANSI and a static HTML document
  in `chronofork-gui`, with no toolkit, browser, rendering or audio dependency (M7 also
  added no third-party dependency). Do not add `axum`/`hyper`/`tokio`/`serde` without an ADR
  that supersedes [ADR-0012](docs/adr/0012-agent-native-interfaces.md); do not add a GUI
  toolkit without an ADR that supersedes
  [ADR-0013](docs/adr/0013-human-facing-gui.md). M8 plans a **separate native frontend
  crate** (`winit` + `softbuffer` + `cpal`) behind such an ADR; no toolkit or audio-device
  dependency may enter `chronofork-gui` or the core, and the headless path must keep working
  without a display or an audio device.

## Architectural boundaries

Dependency direction is enforced by crate structure:

```text
crate                      direct workspace dependencies (read from each Cargo.toml)
chronofork-core            none
chronofork-backend-api     core
chronofork-debug           core, backend-api
chronofork-trace           core, backend-api
chronofork-sessions        core, backend-api, trace
chronofork-script          core, backend-api, sessions, trace
chronofork-service         core, backend-api, sessions
chronofork-transport       service
chronofork-gui             core, service
chronofork-cli             every layer above + chronofork-gui + test-backend and megadrive (selected backend)
concrete backends          core, backend-api only
```

An entry means "depends on", so the direction is `core ← backend-api ← sessions ← service ←
transport` with the host layers beside it, never a strict single stack.
`chronofork-debug` and `chronofork-trace` are siblings that depend only on core and
backend-api; neither depends on sessions. `chronofork-sessions` consumes `chronofork-trace`
and owns the single `capture_trace` entry point, and `chronofork-script` sits above both.
`chronofork-service` consumes core, backend-api and sessions (it does not depend on trace or
script), `chronofork-transport` consumes only the service, and the CLI is the composition root
that also selects a concrete backend. `chronofork-gui` consumes only core and service (never
transport, sessions or a backend), so presentation can reach canonical state only through
the service catalog; nothing depends on `chronofork-gui` except the CLI. No crate below the
CLI depends on `chronofork-transport` (only the CLI composes its typed client and supplies
the local/remote channel to the GUI), and the CLI never re-implements a codec, matcher or
limit.

Rules:

1. Generic session/storage/transport layers must not encode Mega Drive hardware
   assumptions.
2. `chronofork-sessions` must never depend on a concrete backend.
3. A backend must not depend on sessions or transports.
4. Transport types (HTTP, MCP JSON, sockets, JSON-RPC) must not appear in core,
   backend-api, sessions or service. `chronofork-service` has its own canonical JSON value
   type and knows nothing about wire framing.
5. `chronofork-service` must not depend on a concrete backend or on `chronofork-transport`;
   nothing below `chronofork-transport` may depend on it.
6. The GUI or client must never own canonical emulated state. `chronofork-gui` depends on
   core and service only; the CLI is the only crate that composes the transport, so a
   frontend cannot reach a backend or bypass the service boundary.
7. A capability must be backed by tests and reported honestly. Do not advertise
   unimplemented features.
8. Snapshot formats and backend payloads are versioned. Reject incompatible input;
   never silently reinterpret bytes.
9. Preserve determinism: no implicit host wall-clock input to hardware.
10. **Emulated time is canonical; host pacing is not.** The machine advances in deterministic
    emulated cycles. Unpaced headless execution (run as fast as possible to a requested
    frame/cycle), paced 60/50 fps presentation and manual stepping are host policies layered on
    top; they must never change canonical state, frames or audio, and the same inputs produce
    identical hashes in every mode.

More detail: [architecture overview](docs/architecture/overview.md),
[backend model](docs/architecture/backend-model.md),
[session model](docs/architecture/session-model.md),
[execution model](docs/architecture/execution-model.md).

## Onboarding procedure

1. Read this `AGENTS.md` and [docs/README.md](docs/README.md), then the relevant
   architecture and quality documentation.
2. Inspect `git status --short --branch`, recent commits and `git worktree list`.
   Identify the actual repository root and branch; do not assume the workspace points
   to the intended checkout.
3. Read relevant ADRs in [docs/adr/](docs/adr/README.md). ADR-0002 is accepted;
   do not treat a still-`Proposed` decision as accepted.
4. Identify the task, acceptance criteria, dependencies, write scope and required
   Git permissions. Fetch `origin`, fast-forward local `main`, and create or reuse the
   task worktree from the freshly fetched `origin/main` commit **before** loading
   lifecycle state or starting teammates.
5. Reconfirm repository root, branch, permissions and worktree after any directory
   change. Then read [docs/planning/ROADMAP.md](docs/planning/ROADMAP.md) and
   [STATE.md](docs/planning/STATE.md) and follow the lifecycle's resume routing
   (`progress` / `resume-work` / `start-next-phase`); do not invent a plan directory
   or silently reuse another task's plan.
6. Identify the current epic in [docs/epics/](docs/epics/). EPIC-0002 tracks the
   Mega Drive slice; check for newer accepted scope before making assumptions.
7. Inspect the existing implementation and evidence before editing; follow the
   anti-slop pre-flight below when the task affects behavior.
8. Implement incrementally within the assigned paths, coordinating with the team.
9. Run applicable gates, report actual results, and update canonical docs/ADRs.
   Keep temporary notes in ignored worktree-local paths.

## Concurrent work and Git worktrees

### When and where to isolate

- **One independent concurrent write task/team = one branch and one worktree.**
  Teammates in one coordinated team normally share that worktree; do not create one
  worktree per teammate by default. Read-only investigations and small sequential
  tasks may operate without a new worktree when there is no overlap risk.
- For a resumed task, reuse its existing worktree after checking that it is not owned
  by another active session. Never reuse a branch/worktree merely because its name
  looks similar. Check `git worktree list --porcelain`, its branch, status and owner.
- Prefer `<repo>/.agents/worktrees/<task-id>-<slug>/` if the environment tolerates
  nested checkouts. This directory **must** be ignored by the outer repository and
  excluded from recursive source indexing, broad searches and file watchers when
  possible. Use a sibling location outside the repository if nested worktrees cause
  watcher, performance, tooling or sandbox problems; record the actual path.
- Use the native Git CLI in the filesystem where the repo lives (Linux Git for WSL
  checkouts, Windows Git for Windows checkouts). Do not depend on Forge, GitHub, a
  particular harness or an external service to create local worktrees.
- Base a new branch on the approved base ref/commit; do not assume `main` or copy
  uncommitted changes from the parent checkout. If the parent has uncommitted work,
  identify whether it matters to this task; do not accidentally omit needed changes
  or disturb them. If the base ref or ownership is unclear, or a branch name is
  occupied, resolve the ambiguity before proceeding. Never use `--force` to claim
  a worktree.
- **A `main`-based task must start from freshly fetched `origin/main`, not this
  checkout's local `main`.** At session start and before branching, run `git fetch origin`
  and fast-forward local `main` to `origin/main` (never rewrite it); resolve and record the
  actual base commit with `git rev-parse origin/main`, and create the task branch/worktree
  from that commit. Local `main` can point at an older commit regardless of its name, and a
  stale local base silently starts the task from an older tree. An explicit non-`main`
  approved base ref/commit remains authoritative under the preceding rule.
- For a task that needs a new worktree, this is a Git pattern (shown in POSIX shell syntax; adapt it for PowerShell)
  after confirming the root, base ref, branch name and ignore rule (placeholders
  are not literal commands to copy unchanged):

  ```sh
  git fetch origin
  git rev-parse origin/main
  git check-ignore -q .agents/worktrees/ || exit 1
  git worktree add -b 'agent/<task-id>-<slug>' \
    '.agents/worktrees/<task-id>-<slug>' "$(git rev-parse origin/main)"
  git -C '.agents/worktrees/<task-id>-<slug>' rev-parse --show-toplevel
  ```

  If the path is not ignored, configure an appropriate ignore rule first; do not
  create the nested worktree and hope Git will ignore it. A sibling-directory
  worktree uses the same `git worktree add` operation with an external path.
  A worktree is always created from the freshly fetched `origin/main` commit (or the
  explicitly approved base commit), never from a stale local `main` or another branch's tip.

### Select and verify the effective checkout

- The coordinating agent/Team Lead creates or selects the worktree **before** team
  creation and planning initialization. When supported, change the session's
  effective working directory via the harness's workspace/working-directory API.
  `cd` in a one-off shell command does **not** necessarily change the session state.
- After a change, verify the effective checkout in the actual execution environment:

  ```sh
  pwd
  git rev-parse --show-toplevel
  git branch --show-current
  git status --short --branch
  git worktree list --porcelain
  ```

  Commands and file edits must target that same checkout. For shell commands whose
  working directory is not persistent, use explicit working-directory options or
  `git -C <worktree>`; for file tools, supply the correct worktree path.
- Changing a session working directory does not automatically grant new sandbox
  permissions or retarget already-running agents and processes. Validate read/write
  access with a harmless operation before implementation. If the harness cannot
  reliably retarget its file tools or teammates, open a new session at the worktree
  instead. Do not bypass security with `danger-full-access`, ACL rewriting, sudo or
  elevated privileges to make a convenience feature work without explicit approval.
- An agent operating in a task worktree must never edit, stage or run task-specific
  tests from the parent checkout by accident. Verify the target before each
  consequential Git action, and after context restoration or session resumption.

### Multi-agent coordination and ownership

- A **Team Lead** coordinates the team: chooses the checkout, assigns nonoverlapping
  write scopes, owns shared planning and integration state, and serializes Git
  mutations. A **teammate** implements, reviews or tests only the assigned work.
- Agent Teams (where available) is the task/communication layer; its write scopes
  are coordination conventions, **not enforced file locks**. When the harness has
  no Agent Teams, apply the same ownership rules to cooperating agents.
- Before creating teammates, the lead verifies that the worktree is selected and
  communicates its absolute path, branch, task ID, write scopes and dependencies.
  Each teammate verifies its own effective cwd/root on first use. If an agent
  cannot prove it is in the intended checkout, it stops and reports the mismatch.
- Teammates do not independently switch the team's worktree, create extra worktrees,
  update shared manifests/ADRs/`AGENTS.md` or modify another teammate's files without
  explicit coordination. Parallelize disjoint files; serialize shared files.
- Keep `git add`, `git commit`, `git reset`, checkout, rebase and merge operations
  under a single designated integrator/lead at a time. Avoid concurrent writes to
  the Git index, task plan or generated outputs. Coordinate build artifacts,
  container names, ports, databases and caches: Git worktrees isolate files, **not**
  external services or machine-wide state.

### Worktree lifecycle and handoff

1. **Prepare:** identify the task and approved base; inspect existing worktrees and
   current changes; create or reuse a uniquely named checkout if needed.
2. **Bind:** verify effective cwd, Git root, branch and permissions before spawning
   teammates or creating a plan.
3. **Execute:** partition file ownership, implement, review and test in the task
   checkout; save transient evidence and plan state locally.
4. **Handoff:** lead reviews diffs and gates, consolidates durable findings to
   versioned `docs/`, and reports commit/branch, pending work and integration status.
5. **Integrate:** follow [Git authorization](#git-authorization-and-integration)
   and repository review/CI policies. A finished implementation is not necessarily
   published or merged.
6. **Clean up:** only after checking active sessions, pending changes, ignored
   planning/notes and integration status. Preserve or archive untracked evidence
   deliberately; never `git worktree remove --force`, delete a branch or discard an
   active plan automatically. Request approval if any data might be lost.

## Lifecycle and planning

`project-orchestration` is the primary project lifecycle mechanism. It is an Agent Skill
(payload restored from `skills-lock.json`, see [Git hygiene](#git-hygiene)) and it is
harness-independent: the same files and procedures work under any coding agent.

- **Persistent lifecycle state lives in the committed
  [`docs/planning/`](docs/planning/ROADMAP.md) directory.** `ROADMAP.md` records the
  execution plan (active milestone and phases); `MILESTONE.md` defines the current milestone
  and its Definition of Done; `STATE.md` is the session handoff written by `pause-work`;
  `CONVENTIONS.md` records the invariant project conventions. Do not hand-edit the
  `CONVENTIONS.md` fields the Commit & Release Protocol reads — re-run `init-conventions`.
- **Before starting or resuming work, read the state**: this file, `docs/planning/`, the
  canonical milestone page and the relevant ADRs. Use the lifecycle's `progress` /
  `resume-work` / `start-next-phase` routing instead of asking where the project is or
  jumping straight to code. If `docs/planning/` is absent, that is a setup gap to report, not
  a licence to invent a plan directory.
- **The lifecycle is not the capability record.** Execute through `docs/planning/`; capability
  scope, milestone status/integration state, evidence and deferred follow-ups stay canonical
  in [docs/roadmap/milestones.md](docs/roadmap/milestones.md) and
  [docs/roadmap/roadmap.md](docs/roadmap/roadmap.md), and requirements status in
  [docs/requirements/](docs/requirements/functional.md). Link to them; do not copy their
  content (see the [anti-slop policy](docs/quality/anti-slop-policy.md)).
- **New features and durable decisions are reflected in the artifact that owns them**: a
  phase in `docs/planning/ROADMAP.md`, an approved design spec in `docs/superpowers/specs/`,
  a plan in `docs/superpowers/plans/`, an ADR in [docs/adr/](docs/adr/README.md), or the
  architecture/requirements pages. A decision that must survive the session never lives only
  in chat or in ignored scratch.
- **Design before build.** A phase gets an approved design spec and a plan before
  implementation. Dispatch through `brainstorming` → `writing-plans` → `executing-plans`
  (directly, by subagents or by Agent Teams); a small task may use a short design, but it is
  not skipped.
- **No parallel planning systems.** `docs/planning/` (tracked lifecycle) and `.planning/`
  (ignored scratch) have different jobs. Ignored scratch never overrides accepted scope,
  status or decisions.

### Ignored scratch plans (`planning-with-files`)

- Use an installed planning skill only after loading its actual `SKILL.md`; do not
  claim to use a skill you did not load. Otherwise use a concise task list and
  promote durable decisions to `docs/`.
- `.planning/` is ignored, **scratch/evidence local to a worktree**, not the
  source of truth for accepted architecture, capabilities, scope or status. Never
  share/copy its mutable state or `.attestation` between independent worktrees or
  sessions.
- One designated plan owner per team (normally the lead) may modify the shared
  `task_plan.md`, `findings.md`, `progress.md` and attestation state. Teammates
  report through task-board messages or their own task-specific ignored notes,
  e.g. `.agents/notes/<task-id>/<agent>.md` (if that path is ignored); they do not
  reinitialize, overwrite, reset or reattest the central plan.
- Keep daily execution details in `progress.md` and discoveries in `findings.md`;
  update `task_plan.md` when scope, milestones, criteria or phase structure changes,
  rather than on every task-status change. The task board tracks live assignment,
  dependencies and completion; planning files preserve longer-lived context.
- `PLAN TAMPERED` or any attestation mismatch is a **stop-and-investigate** event,
  not a prompt to disable a guard. Check the actual selected plan, current hash,
  changes and other active writers. Never delete `.mode`, clear the attestation or
  blindly reattest to suppress the warning. After reviewing an intended plan change,
  the authorized owner may reattest using the installed skill's documented process;
  verify the result. No automatic self-approval of unexpected changes.
- Resolve any `.active_plan`/named-plan selection **inside the active worktree**;
  verify that the canonical target path belongs to the intended checkout before
  reading or updating it. If selection is ambiguous or points elsewhere, stop.
- Before deleting/archiving a plan or worktree, extract durable findings into
  `docs/`, preserve any necessary evidence and verify no active session relies on it.

## GitHub integration

GitHub is the trackable projection of the lifecycle, not a second source of truth.

- **Issues are the trackable activities.** Prefer one Issue per feature or fix. Reference the
  Issue from the branch name and the commits, and open a PR that links it.
- **Implementations land through PRs.** Do not modify a protected branch directly. `main` is
  protected by project convention ([CONVENTIONS.md](docs/planning/CONVENTIONS.md)), and a
  change arrives through a reviewed PR with green checks — even when the host does not
  enforce the protection.
- **Milestones and Issues can be synchronized by `project-orchestration`**
  (`init-github-sync`, `sync-github`). Native sync is **one-way: local planning → GitHub**.
  Never copy a GitHub-side title, label, milestone or state back into `docs/planning/`;
  reconcile externally made changes consciously and re-run the sync afterwards.
- **Do not create duplicate Issues or Milestones.** Check what exists first. A closed Issue
  proves only that someone closed it: implementation is proven by code, executed tests and
  review evidence, and the phase is closed locally with `complete-phase`.
- **First-time publication needs explicit approval.** Prepare and dry-run the sync, present
  the resources that would be created or changed, then publish only after the owner approves.
- **GitHub Projects v2 is not part of the native integration.** Issues and Milestones are the
  supported projection; Projects v2 may be configured later only by an explicit decision.

## Development workflow

- **Isolate work.** One independent write task = one branch and one worktree (see
  [Concurrent work and Git worktrees](#concurrent-work-and-git-worktrees)). Read-only
  investigation and small sequential tasks do not need one.
- **Keep the base current.** Fetch `origin` and fast-forward local `main` to `origin/main`
  before branching and at session start; always create a worktree from the freshly fetched
  `origin/main` commit, never from a stale local `main`.
- **Never modify a protected branch directly.** `main` is the integration branch; changes
  arrive through a reviewed PR with green checks.
- **Verify before calling a task done.** Run the applicable gates and report the actual
  results ([Testing expectations](#testing-expectations)); a change is not complete because it
  compiles.
- **Record architectural changes in ADRs** and keep the technical documentation consistent
  with the code. Update the one canonical capability home instead of duplicating it.
- **Report delivery honestly.** Distinguish implemented, tested, committed locally, pushed,
  under review and merged (see [Git authorization](#git-authorization-and-integration)).

## Harness independence

This repository must remain executable by any coding agent (DeepSeek Harness, Codex, Claude
Code, OpenCode, others), by subagents and by humans.

- **Prefer portable mechanisms**: the Agent Skills standard (`SKILL.md` plus the
  `skills-lock.json` lockfile) and command-line tools (`git`, `gh`, `cargo`,
  `./scripts/check.sh`) over harness-specific APIs, editors or slash commands.
- **Do not write harness-specific instructions** (a particular harness's tools, commands or
  hooks) when a portable alternative exists. Harness configuration belongs in that harness's
  own settings, not in `AGENTS.md`.
- **Allow any orchestrator; require none.** A phase may be executed directly, by subagents or
  by Agent Teams; the plan and its evidence do not depend on which one ran.
- **Avoid competing systems.** One lifecycle (`project-orchestration`), one capability record
  (`docs/roadmap/`), one conventions file and one planning directory. Add tooling only when it
  removes duplication rather than creating it.

## jev-cli as a second opinion

The `jev-cli` skill wraps the `jev` command and returns a typed judgment about text you
submit. Load its `SKILL.md` for the command forms instead of restating them here. Use it
when the harness exposes it and the question is a judgment call about material we
already wrote, not a fact lookup:

- Claim honesty: does a doc or README sentence promise more than the evidence we have?
  Put the claim and the evidence (gate results, deferred-scope list, accepted ADR) in
  the same state; the evaluator cannot read the repository.
- Triage: which crate owns a change, whether a change needs a new ADR or which
  milestone (or hardware-depth track H1–H5) it belongs to, and how severe a review
  finding is. Prefer `choice` with described options; keep `noul` for a coarse yes/no.
- Commit and PR honesty: does the message claim something the diffstat does not show?

Constraints:

- Advisory only. Never in `./scripts/check.sh`, CI, acceptance plans or the anti-slop
  evidence; a merge still rests on executed tests and review.
- Not a hardware oracle. It follows rules placed in the state but cannot be trusted for
  emulation facts, cycle counts or compatibility.
- Treat empty state as a hard error before calling `jev`; an empty string or empty
  `@file` returns a confident answer with exit 0.
- Near the 0.5 boundary answers drift between runs, so require a margin or ask the
  question both ways. Exit codes 2/3/4 are technical failures, never a "no".
- The state leaves the machine: submit no secrets, credentials, ROM/BIOS bytes or
  private paths.

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
printf 'create\nattach 1\npulse 3\nframe\nquit\n' | cargo run -p chronofork-cli -- gui run
```

The `gui run` check must stay deterministic: stdout is byte-identical across runs, stderr is
empty on success, and no block or frame may carry a host path, token or host time. M8's
cross-mode contract adds: the same input must produce identical frame, audio and state hashes
in unpaced, paced and stepped execution. A change
is not complete because it compiles. Required behavior needs an automated
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

## Git authorization and integration

- The Team Lead/integrator may prepare local task branches and worktrees for an
  approved write task, subject to the workspace/sandbox rules above. This does not
  authorize remote publication or merging.
- **Local commits:** only when authorized by the task or an explicit standing
  repository policy; the lead/integrator stages reviewed, task-scoped paths and
  creates small logical commits. Teammates do not commit independently by default.
- **Push / remote branch / pull request:** only with explicit task approval or a
  previously documented standing policy naming the allowed remote and branches.
  A local commit is not permission to publish. Do not assume GitHub: support a
  configured Forgejo, GitLab or other remote without changing providers silently.
- **Merge into `main` or any protected/integration branch:** separate explicit
  authorization is required by default. Respect review, CI, branch protection and
  license/security checks. Passing tests or a teammate's review does not itself
  authorize merging. Prefer a reviewed change request where supported.
- **Never by default:** force push, rewrite shared history, delete shared branches,
  create/tag releases, change remotes, bypass CI/approvals, or auto-merge. These
  require a separate, specific authorization and the applicable repository policy.
- Before staging or committing, verify the active root/branch, inspect `git diff`
  and `git status`, and stage only intended paths; do not `git add -A` blindly.
  Serialized Git operations avoid races between teammates using the same checkout.
- A handoff report distinguishes **implemented**, **tested**, **committed locally**,
  **pushed**, **under review** and **merged**. Never describe an unmerged branch as
  integrated or an unverified change as done.

## Git hygiene

- `.research/upstream/` and `.planning/` must remain untracked/ignored.
- `.agents/skills/` and `.claude/skills/` are ignored; commit `skills-lock.json`, not the
  installed payload, and restore it with `npx skills install --frozen-lockfile`.
- Ensure `.agents/worktrees/` and any transient `.agents/notes/` directory are
  ignored before using them. Do not assume an ignore rule exists; check with
  `git check-ignore`. Worktree-local ignored files do not travel with commits.
- Do not add submodules or copy upstream repositories into the tree.
- Do not rewrite history, delete pre-existing user files or discard unknown changes.

## Security constraints

- Do not commit secrets, tokens, credentials or local environment files.
- Do not run untrusted scripts with host privileges.
- Scripting is sandboxed with explicit memory/time limits and a canonicalized sandbox root;
  it is a deterministic capability boundary, not process isolation or an audited
  hostile-code security boundary. Do not weaken it or run untrusted native code.
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
- For a write task, the team worked in the intended checkout; there are no
  unintended edits in the parent or another task's worktree.
- The lead reports the exact Git delivery status and outstanding review, push or
  merge approvals. A task can be implementation-complete but integration-pending.
- Operational plans/notes needed for recovery are preserved until safe handoff;
  durable decisions are consolidated in versioned documentation.
- The lifecycle state reflects the outcome: the phase is closed (or left `active` with the
  blocker in `STATE.md`), `docs/planning/ROADMAP.md` and `MILESTONE.md` agree, and the session
  handoff is enough for a fresh session to resume without this conversation.

## Understanding current project state

Use these sources, in order:

1. The effective checkout root, branch, `git status --short --branch`, linked
   worktrees and recent commits (never infer state from a different checkout).
2. [docs/planning/ROADMAP.md](docs/planning/ROADMAP.md) and
   [docs/planning/STATE.md](docs/planning/STATE.md): the lifecycle state, the active
   milestone and the last session handoff.
3. [docs/README.md](docs/README.md) and the relevant architecture/ADR pages.
4. Any worktree-local ignored scratch (`.planning/`) as context only — never as authority.
5. The current [epic](docs/epics/EPIC-0002-megadrive-slice.md) and
   [milestones](docs/roadmap/milestones.md).
6. `cargo test --workspace` output from the intended checkout, not assumptions.

If sources conflict, prefer code and executed tests for what *is*, and ADRs/docs
for what *should be*. Report the conflict rather than silently choosing one.
