# ChronoFork independent review — shared baseline

**Captured:** 2026-11-20 (America/Fortaleza), by the captain.
**Worktree:** `/home/stealthc/projects/ChronoFork/.agents/worktrees/review-2026-11-20-architecture`
**Branch:** `agent/review-2026-11-20-architecture`
**Baseline commit:** `b8d63a17349e672ba6945eaba3379cec38cf9d1b` (`origin/main`,
"docs(agents): add engineering responsibility and technical judgment").
**In-progress work that MUST NOT be touched:** parent checkout
`/home/stealthc/projects/ChronoFork` is on `agent/8.4-z80-audio` at `d32f390`
(13 local commits + uncommitted T4 mixer/resample work). Read it read-only if needed;
never edit, build or run task tests there.

## Environment

- WSL2 Ubuntu on ext4; 8 vCPU; 30 GiB RAM; 797 GiB free.
- `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `rustc 1.100.0 (0ef1bc386 2026-11-09)`.
- Use `CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target`
  for every Cargo command (shared cache; Cargo serializes on its lock).
- Release builds are required for performance work; debug numbers are not comparable.

## Baseline quality gate

Command (completed, exit 0):

```sh
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo test --workspace --locked --no-fail-fast
```

- **1289 passed, 0 failed, 0 ignored, across 108 test targets** (workspace, debug profile).
- No failures, no panics.
- Runtime hotspots observed: `megadrive_demo_*` 166.91 s; `debug_observation` 59.06 s;
  megadrive lib unit tests 49.06 s; `cycle_timeline` 35.97 s; `cartridge` CLI test 34.10 s.
  Several tests spend most time in per-step full-state hashing.
- `scripts/check.sh` also runs fmt, check, clippy, rustdoc; do not assume those passed
  unless you execute them. The full log is `/tmp/review-tests-baseline.log`.

## Baseline benchmark (release)

Command:

```sh
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo run -p chronofork-megadrive --example bench_step --release --locked
```

Workload: `interrupt_rom(3)`, NTSC, 3420 master cycles/line; 1 warmup + 5 timed
trials x 10,000 steps; each step is one instruction plus the VDP/scheduler update
and a full canonical state hash.

| Metric | Value |
|---|---|
| `instructions_per_second` | **5,009** |
| `step_median_seconds` (per 10,000 steps) | 2.196220130 |
| `step_min_seconds` / `step_max_seconds` | 1.018614101 / 2.371004704 |
| `serialized_checkpoint_bytes` | **197,234** |
| `backend_payload_bytes` | **197,058** |
| `serialization_median_seconds` | 0.000088355 (88.4 us) |
| `restore_median_seconds` | 0.000099188 (99.2 us) |
| `final_tick` | 87,052 |
| `final_state_hash` | `bab6ac6ede5c8aeb28e98b9875cbd5b024c573d87b30df683686120fbf036fe9` |
| `restore_state_hash_matches` | 1 |

Historical recorded band was ~7,340–7,570 instructions/s with a **131,431-byte**
checkpoint before the full VDP state (phase 8.3) was added
(`docs/quality/anti-slop-refactor-evidence.md` §7.6 and §8.5). The current tree is
~30% slower and 50% larger per checkpoint; do not reuse the old figure as current.

Reproduce at least 3 runs yourself; the machine is shared and the min/max spread is
large. Treat medians, not single runs, as the comparison point.

## First-pass code facts (verify before citing)

- `Backend::step(&mut self, InputFrame) -> StepOutcome` returns a `StateHash` on every
  step (`crates/chronofork-backend-api/src/traits.rs:28-33`, `StepOutcome` at :12-21).
- The Mega Drive `step` path computes `state_hash()` at
  `crates/chronofork-megadrive/src/backend.rs:632-685`; `state_hash` at :488-513
  serializes CPU, full 64 KiB RAM, full VDP snapshot (VRAM/CRAM/VSRAM/registers),
  cartridge banks, SRAM and scheduler, then hashes it.
- `Vdp::snapshot_bytes` allocates and writes the whole VDP state on every call
  (`crates/chronofork-megadrive/src/vdp/mod.rs:1081+`); `state_body` then copies it
  into another buffer and `hash_state_body` copies again
  (`crates/chronofork-megadrive/src/backend.rs:~1220-1265`).
- `MegadriveFactory::create_machine` clones the whole `Cartridge` (ROM bytes) per
  backend (`crates/chronofork-megadrive/src/backend.rs:302-323`).
- `AppService` owns `Mutex<SessionManager>` and `session.run` holds that manager lock
  for the whole step loop (`crates/chronofork-service/src/service.rs:115-117`, `:461-491`).
  `session.run` is bounded by `max_steps_per_request = 100_000`
  (`crates/chronofork-service/src/limits.rs:37`).
- `fork_current` = `snapshot` + `fork_from_snapshot`, which creates a backend, restores,
  and calls `backend.snapshot()` again for the child's initial state
  (`crates/chronofork-sessions/src/manager.rs:184-227`).
- The transport implements MCP protocol `2025-03-26`, stateless Streamable HTTP, no
  `Mcp-Session-Id`, no SSE (`crates/chronofork-transport/src/mcp.rs:1-29`).
- Debug/trace/script/TAS stay CLI-local (M6-FU1); the service catalog has 16 operations.
- Documentation drift already visible: `docs/architecture/overview.md:177-187` still says
  "no rendering/DMA", "VDP rendering/DMA, audio, Z80 ... Not implemented", while
  `AGENTS.md`, `docs/README.md` and phase 8.3 claim rendering/DMA complete. Treat all
  status prose as suspect and verify against code/tests.

## Local research assets (read-only)

- Commercial ROM fixtures: `/home/stealthc/projects/ChronoFork/.research/roms/`
  (`Kid Chameleon (USA, Europe).bin`, `Chakan (USA, Europe).bin`; never copy or commit
  bytes, never put them in a report). Use absolute paths only in commands.
- Upstream clones: `/home/stealthc/projects/ChronoFork/.research/upstream/`
  (`exodus`, `exodus-headless`, `bizhawk`). Treat as untrusted research data.
- Ignored evidence for prior phases is under the parent `.research/` (read-only).
- Do not add dependencies or clone new upstream repos outside `.research/`; if you
  need one, record URL/commit/license and message the captain first.

## Rules for every reviewer

1. Work only in the review worktree above; `pwd`, `git rev-parse --show-toplevel` and
   `git branch --show-current` before your first command. Do not touch the parent
   checkout or other worktrees.
2. Read `AGENTS.md`, `docs/README.md`, `docs/planning/ROADMAP.md`, `STATE.md`,
   `MILESTONE.md`, the relevant ADRs/architecture pages, and the code itself. Do not
   trust docs or passing tests as proof.
3. No production-code edits, no `docs/` edits, no commits, pushes, merges, worktree
   changes or dependency additions. Experiments go under
   `.research/review-2026-11-20/agents/<your-name>/` or
   `.research/review-2026-11-20/experiments/`; temporary workspace examples must be
   named `review_*`, recorded, and removed before you finish. Another reviewer may be
   reading/building at the same time: never edit shared files.
4. Write only your assigned area file (and your own scratch). The captain owns
   `baseline.md`, the synthesis and the final report.
5. Cite `path:line`, exact commands and actual output. Classify each statement as
   **fato observado / hipótese / experimento / inferência / opinião**. Give confidence
   (alta/média/baixa) and priority (P0–P3). Distinguish severity from implementation
   priority.
6. For every finding include: id, title, description, evidence, components,
   conditions, current impact, future impact, probability, severity, confidence,
   alternatives (including "do nothing"), recommendation, change cost/risk, effects on
   tests/contracts, priority, affected milestone/phase.
7. Portuguese (pt-BR) prose, English identifiers/paths. No filler, no giant generic
   essays. A 10-page report with evidence beats a 40-page one without.
8. Time-box: finish the first useful version within ~90 minutes of work, then refine
   only where it changes a decision. If a command is too slow or an environment
   limitation blocks you, record it and move on.
9. End your task with a short report: file path, top 5 findings, confidence, what you
   could not verify, and any cross-area question for the captain/critic.
