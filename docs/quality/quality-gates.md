# Quality Gates

A feature is not complete because it compiles. These gates must be executed and
recorded. Results below are from the bootstrap host; update them when the
toolchain or environment changes.

**Current status:** the authoritative recorded run for the anti-slop refactor's
implementation bytes is
[anti-slop refactor evidence §8.2](anti-slop-refactor-evidence.md#82-gates-and-the-199-test-accounting):
`./scripts/check.sh` exit 0 with all gates clean, both demos byte-identical to the
captured baseline ([§8.3](anti-slop-refactor-evidence.md#83-demos-three-runs-each-compared-with-the-t1-baseline)),
and the serialized-byte probe byte-identical
([§8.4](anti-slop-refactor-evidence.md#84-differential-serialized-state-probe-recreated)); the exact test
count and per-target accounting are recorded there, not copied here. The dated
result blocks below are historical snapshots; their test counts are not current
claims. Run the gates yourself rather than copying a number.

## Required local gate

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Or run the repository script:

```sh
./scripts/check.sh
```

The script sets `PATH` for a user-local rustup install and stops on the first
failure.

## Historical bootstrap results (2026-11-13; superseded, not a current claim)

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Passed after applying `cargo fmt --all` |
| Compilation | `cargo check --workspace` | Passed |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| Tests | `cargo test --workspace` | 40 passed, 0 failed |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Passed; broken intra-doc link fixed |
| Demo | `cargo run -p chronofork-cli -- demo` | Exit 0, `chronofork demo: PASS` |
| Capabilities | `cargo run -p chronofork-cli -- backend describe` | Exit 0, real capability values |

Test breakdown:

- `chronofork-core`: 8 unit tests.
- `chronofork-sessions`: 4 snapshot-codec tests + 5 acceptance tests.
- `chronofork-test-backend`: 7 unit tests.
- `chronofork-megadrive`: 13 unit/integration tests.
- `chronofork-cli`: 3 integration tests.
- Doc tests: 0.

## Historical Mega Drive VDP/interrupt integration results (2026-11-14, after F1/F5 repairs; superseded, not a current claim)

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Passed |
| Compilation | `cargo check --workspace` | Passed |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| Tests | `cargo test --workspace` | 98 passed, 0 failed |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Passed |
| Demo | `cargo run -p chronofork-cli -- megadrive demo` | Exit 0, `chronofork megadrive demo: PASS` |
| Full script | `./scripts/check.sh` | Passed end to end |

Test breakdown:

- `chronofork-core`: 8 unit tests.
- `chronofork-sessions`: 4 snapshot-codec tests + 5 acceptance tests.
- `chronofork-test-backend`: 7 unit tests.
- `chronofork-megadrive`: 71 unit/integration tests (CPU, bus, VDP, scheduler,
  backend, sessions).
- `chronofork-cli`: 3 integration tests.
- `chronofork-backend-api`: 0 unit tests.
- Doc tests: 0.

The exact `megadrive demo` output from the 2026-11-14 stage-1b gate run was:

```text
chronofork megadrive demo: 68000 + VDP port + interrupt scheduler slice
backend: ChronoFork Mega Drive 68000 (VDP port + interrupt scheduler, no rendering/audio) (0.1.0)
scope: 68000 subset, ROM/RAM, VDP status/control/VRAM window, deterministic interrupt scheduler
limitations: no rendering, audio, Z80, DMA, controllers or cycle-accurate timing
vdp: register 15 (auto-increment)=2 status=0x0100 address=0x0002
vdp: program wrote and read back VRAM word 0x1234 through the data port
interrupt: HBlank pending at tick 64 (level 4, vector 0x70, mask 7)
interrupt: HBlank delivered; handler pc=0x2C2 D7=1
interrupt: VBlank pending level 6 vector 0x78 at tick 896 (quantum=4); snapshot restored it
fork isolation: fork=2c3a89ee5ac070b763f0efe3e25f142ff99b8842820328e7102c60b7b76d3ebc source=e8cfa3a05b7efc606ec74c9eeab949ab984901e553037b1390f8b1aed6ff4ea0
snapshot: VDP, scheduler and CPU state reproduced from the checkpoint
chronofork megadrive demo: PASS
```

## 2026-11-15 — M2 durable sessions, Fase B (round 2, post-F1 repair; current M2 claim)

Authoritative gate run for the durable-session implementation, independently
re-verified after the F1 repair. Tree: `3d840634ed21d305762d57513e1da9ff4e22479d`
plus the uncommitted Fase B worktree. Raw evidence (ignored paths):
`.research/m2-durable-sessions/fase-b/verification/round2/` (`SUMMARY.txt`,
`MANIFEST.sha256`, `targets/`, demo and gate logs); round 1:
`.research/m2-durable-sessions/fase-b/verification/` (`SUMMARY.txt`,
`target-matrix.txt`, `MANIFEST.sha256`); F1 repair:
`.research/m2-durable-sessions/fase-b-repair-f1/SUMMARY.txt`. The durable acceptance
record is the [M2 durable-sessions acceptance plan](m2-durable-sessions-acceptance.md);
exact test counts and per-target accounting stay in the evidence artifacts and are not
copied here.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0, no failures (counts in the round-2 SUMMARY) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0, ends "all quality gates passed" |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0, 3 runs, stderr empty, byte-identical to the t1 baseline |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0, 3 runs, stderr empty, byte-identical to the t1 baseline |
| Whitespace | `git diff --check` | exit 0 |

Round 1 executed M2-A1–A11 and the CLI contract and reported one medium, non-blocking
finding: the CLI `save --session` read the latest snapshot before taking the writer
lock (F1). The repair added `SnapshotRepository::with_writer_lock` and
`save_snapshot_with_lock`, and the CLI now acquires the lock before the read and holds
it through rehydrate/advance/capture/publish. Round 2 re-verified F1 closed: typed
contention at acquisition, one winner in repeated real two-process races, no
stale-base double publish and no regression. Residual limits remain stated in the
[M2 acceptance plan §10](m2-durable-sessions-acceptance.md#10-out-of-scope-and-stated-limitations):
Linux-local-filesystem durability only (Windows/network filesystems unverified), a
cooperative writer lock, long lock holds with large `--steps`, explicit stale-lock
recovery, manual retention, no encryption/authentication or migration, and the
publish-only `save_snapshot` convenience caveat for third-party callers.

## Gate policy

1. Do not claim a gate passed without the actual command and result.
2. Do not weaken a lint or delete a test to make a gate pass without documenting
   why and obtaining review.
3. Do not mark a roadmap feature complete because a stub compiles.
4. Any new behavior needs at least one automated test at the lowest useful layer.
5. Any new external format needs round-trip, rejection and version tests.
6. Any determinism or snapshot claim needs an executable invariant test.
7. Any incorporated code needs provenance and license review before merge.

These gates cover correctness and formatting. Code structure, duplication and resource
budgets are governed by the [anti-slop policy](anti-slop-policy.md); the recorded
baseline/after proof for the current refactor is
[anti-slop refactor evidence](anti-slop-refactor-evidence.md). Do not claim a gate
passed without a command and an exit code recorded there or in your own run.

## Test categories and current coverage

| Category | Bootstrap status |
|---|---|
| Unit tests | Implemented |
| Integration/acceptance tests | Implemented |
| Snapshot round-trip | Implemented |
| Session isolation | Implemented |
| Deterministic replay | Implemented (synthetic) |
| Corruption/version rejection | Implemented |
| Property-based tests | Implemented: codec/snapshot property tests ([evidence §8.2](anti-slop-refactor-evidence.md#82-gates-and-the-199-test-accounting)) |
| Fuzz testing | Planned |
| Differential execution | Baseline-vs-refactor serialized-state probe implemented ([evidence §8.4](anti-slop-refactor-evidence.md#84-differential-serialized-state-probe-recreated)); full emulator differential remains planned |
| Performance benchmarks | Implemented as `bench_step` example; evidence, not a CI gate ([evidence §8.5](anti-slop-refactor-evidence.md#85-benchmark-on-the-post-repair-tree)) |
| Concurrency tests | Planned (no concurrent execution yet) |
| Serialization compatibility | Implemented for v1/v2 rejection (payload version tests) plus round-trip property tests |
| Cross-platform CI | Partial: Windows/macOS compile check; runtime tests are Linux-only |

## CI (GitHub Actions)

`.github/workflows/ci.yml` runs on every pushed commit (all branches) and on pull
requests. It mirrors the local gates and adds:

- `fmt`: `cargo fmt --all -- --check`.
- `check-clippy`: `cargo check --workspace` and Clippy with `-D warnings`.
- `test`: `cargo test --workspace --locked --no-fail-fast`, then the determinism
  suite single-threaded; the test log is uploaded as an artifact.
- `docs`: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.
- `demos`: both CLI demos run three times; stderr must be empty and stdout must be
  byte-identical across runs; outputs are uploaded as artifacts.
- `release`: release build and release-mode tests.
- `msrv`: `cargo check --workspace --all-targets --locked` with the declared MSRV
  1.85.0.
- `portability`: `cargo check --workspace --all-targets --locked` on Windows and
  macOS. These are build checks only; runtime tests and filesystem durability on
  those platforms remain unverified.

Actions are pinned to commit SHAs, the workflow requests only `contents: read`, and
it sets no concurrency cancellation, so every pushed commit runs the full workflow.

Not yet wired: JUnit XML reporting, `cargo deny`, fixture-manifest checks and
runtime tests on Windows/macOS. The recommended follow-ups remain a license/
provenance check when a dependency or incorporated file changes, and nightly
extended runs.

## Release gate (future)

A release must additionally have:

- Native Windows and Linux test evidence.
- Differential tests for the real backend.
- Snapshot compatibility matrix.
- Complete third-party notices and provenance.
- No ROMs/firmware/secrets in the artifact.
- Reproducible build instructions.
