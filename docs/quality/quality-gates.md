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

## 2026-11-15 — M3 TAS/replay, first increment (M3-T4 independent verification)

Independent verifier PASS for the uncommitted M3 implementation tree. Tree under
test: `bcd6c6fae7edd8014aec27e89689933e182c246a` plus the M3 working-tree paths
(`crates/chronofork-sessions/src/movie.rs`, `crates/chronofork-sessions/tests/movie.rs`,
`crates/chronofork-cli/tests/tas.rs`, the CLI/sessions exports and the Fase A contract
docs). No commit or push. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

Authoritative raw evidence (ignored paths): `.research/m3-tas/verification/`
(`SUMMARY.txt`, `target-matrix.txt`, `A1-*.txt`–`A12-*.txt`, `logs/`, `targets/`,
`cli/`, `demos/`, `probes/`, `MANIFEST.sha256`); pre-change baseline:
`.research/m3-tas/baseline/`. The acceptance rows are executed and passed in that
evidence; the Fase A [M3 acceptance plan](m3-tas-replay-acceptance.md) still carries
`planned` status until the staged documentation-promotion task updates it. M3 is **not**
complete: frame advance, rewind, seek, rerecord, branch editing, multi-controller/ports/
analog input, real-hardware input TAS and HTTP/MCP/GUI/daemon remain deferred.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 31 binaries, 342 passed, 0 failed, 0 ignored |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0, ends "all quality gates passed" |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0; 3 runs, 0-byte stderr, byte-identical to the M3-T0 baseline |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0; 3 runs, 0-byte stderr, byte-identical to the M3-T0 baseline |
| Whitespace | `git diff --check` | exit 0 |
| M2 regression | the M2 target matrix plus `cargo test -p chronofork-cli --test session_storage` | every target exit 0; counts equal the M3-T0 baseline |
| State probe | the M3-T0 `session save/load` probe on two independent roots | checkpoint SHA-256 `120e83d7…ca1e` and `tick=7`/`state_hash=4666a724…9ed5` identical to baseline |

M3-A1–A12 executed with no failed criterion: movie round-trip/canonical bytes
(T0-identical across time, roots and debug/release), bounded decode and the typed
rejection matrix, every-tick record/replay, exact first divergence (tick, input index,
expected, observed), restart and M2-seeded replay after deleting the root, source
non-mutation/isolation, the `tas record|play|verify` process contract, no host time in
canonical bytes, M2 regression/demo/gate equality, scope honesty, branch metadata and
the single-codec/predicate/no-dependency structural audit. Non-blocking observations are
recorded in the evidence SUMMARY (truncation taxonomy under checksum-first decode and the
pending documentation promotion); no test, lint or assertion was weakened. Full details
and hashes are in the ignored artifacts above; this dated block is a gate-result
snapshot, and the per-target counts stay in the evidence.

## 2026-11-15 — M3 partial replay/seek and branch comparison, second increment (independent verification)

Independent verifier PASS for the uncommitted M3 increment-2 tree. Tree under test:
`c6c502de54932459a1d3bc0cd0ada10ceba97043` plus the increment-2 working-tree paths
(`crates/chronofork-sessions/src/replay.rs`, `crates/chronofork-sessions/src/{lib,movie}.rs`,
`crates/chronofork-sessions/tests/replay.rs`, `crates/chronofork-cli/src/{main,tas}.rs`,
`crates/chronofork-cli/tests/tas.rs`) and the owner-approved Fase A contract docs. The
pre-existing top-level `README.md` working-tree change (owner-authorized, sha256
`838398caac5547406f4f42078b9f66976bc985254e77c46ad61dfd6729a9d33b`) is excluded from
increment scope. No commit or push. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

Authoritative raw evidence (ignored paths): `.research/m3-tas-inc2/verification/`
(`SUMMARY.txt`, `target-matrix.txt`, `A13-*.txt`–`A17-*.txt`,
`A13-A17-execution-record.txt`, `logs/`, `targets/`, `cli/`, `demos/`, `probes/`,
`MANIFEST.sha256`); pre-change baseline: `.research/m3-tas-inc2/baseline/`. The
acceptance rows are executed and passed in that evidence; the Fase A
[M3 partial-replay acceptance plan](m3-partial-replay-acceptance.md) remains `planned -
not yet executed` until the staged documentation-promotion task updates it. M3 is **not**
complete: checkpoint anchors and format v2, an in-process rewind buffer or state cache,
interactive stepping, rerecord/branch editing, multi-controller/ports/analog/keyboard
input, real-hardware input TAS, import/export adapters and HTTP/MCP/GUI/daemon remain
deferred.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 0 failed, 0 ignored; no target below the increment baseline (counts in `target-matrix.txt`) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0, ends "all quality gates passed" |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0; 3 runs, 0-byte program stderr, byte-identical to the increment baseline |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0; 3 runs, 0-byte program stderr, byte-identical to the increment baseline |
| Whitespace | `git diff --check` | exit 0 |
| M2 regression | the M2 target matrix plus `cargo test -p chronofork-cli --test session_storage` | every target exit 0; counts equal the increment baseline |
| State probe | the baseline `session save/load` probe on two independent roots | checkpoint SHA-256 `120e83d7…ca1e` and `tick=7`/`state_hash=4666a724…9ed5` identical to baseline |
| M3 v1 regression | increment-1 movies replayed with default and ranged paths; fresh recording byte-compared | v1 bytes and default CLI blocks identical to baseline; `MOVIE_FORMAT_VERSION`/`MOVIE_INPUT_SCHEMA_VERSION` still 1 |

M3-A13–A17 executed with no failed criterion: prefix-always-verified `(from, until]`
window replays equal to the recorded boundary hashes; seek, one-step forward advance and
rewind as separate invocations with no carried state; typed range rejection (before-start,
after-end, inverted, unaligned, empty and `u64::MAX` boundaries) before checkpoint import
with no session or step; recorded-expectation comparison with separate checkpoint,
exact-first-divergence and length verdicts where metadata and input masks never decide
equality and nothing re-simulates; non-mutation of source sessions and movie bytes;
v1/M2 non-regression; the CLI `play`/`verify --from/--until` and
`tas compare --movie/--against` process contract, help and corruption handling.
Non-blocking observations are recorded in the evidence SUMMARY (process-level message
spelling refinements and the pending documentation promotion); no test, lint or assertion
was weakened. Full details and hashes are in the ignored artifacts above; this dated block
is a gate-result snapshot, and the per-target counts stay in the evidence.

## 2026-11-15 — M3 increment 2 final verification after documentation promotion (t7)

Final independent verifier PASS for the increment-2 tree and its promoted documentation.
Tree under test: `c6c502de54932459a1d3bc0cd0ada10ceba97043` (branch `main`, equal to
`origin/main`; `git ls-remote origin refs/heads/main` reports the same commit) plus the
increment-2 working-tree implementation/test paths (sessions `replay.rs`, `lib.rs`,
`movie.rs`, `tests/replay.rs`; CLI `main.rs`, `tas.rs`, `tests/tas.rs`) and the promoted
docs. The pre-existing owner-authorized top-level `README.md` change (sha256
`838398caac5547406f4f42078b9f66976bc985254e77c46ad61dfd6729a9d33b`) remains excluded
from increment scope. No commit, push or tag was made. Every Cargo command used
`CARGO_HOME=/tmp/t7-cargo`.

Source/implementation drift check: all 70 tracked build/test input paths (`crates/`,
`tests/`, `scripts/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`,
`.github/`) are byte-identical to the t5 verification snapshot; only documentation files
were promoted after t5. The t5 evidence manifest re-verifies (665/665). Promoted claims
were checked against the executed M3-A13–A17 rows and t5 artifacts: every acceptance row
is still `executed - passed`, no capability is claimed beyond the deterministic synthetic
backend, and M3 remains explicitly **not complete** with the deferred scope unchanged
(checkpoint anchors/format v2, in-process rewind buffer/state cache and interactive
stepping, rerecord/branch editing, multi-controller/ports/analog/keyboard input,
real-hardware input TAS, import/export adapters, checkpoint-reference movies and
HTTP/MCP/GUI/daemon). Markdown links, anchors and 30 evidence-artifact references in the
promoted docs all resolve.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 0 failed, 0 ignored; no target below the increment baseline (counts in `target-matrix.txt`) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0, ends "all quality gates passed" |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0; 3 runs, 0-byte program stderr, byte-identical to the increment baseline |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0; 3 runs, 0-byte program stderr, byte-identical to the increment baseline |
| Whitespace | `git diff --check` | exit 0 |
| Source drift | build/test path SHA-256 set vs t5 evidence | identical; no drift (70/70 paths) |
| Repository hygiene | tracked-path audit; `git diff --cached`; `git ls-remote` | no `.research/`, `.planning/`, `.agent-teams/` or skill payload tracked; nothing staged; no commit/push |

Evidence (ignored paths): `.research/m3-tas-inc2/final/` (`SUMMARY.txt`,
`target-matrix.txt`, `docs-claim-audit.txt`, `logs/`, `demos/`, `tree-audit.txt`,
`MANIFEST.sha256`); prior t5 evidence `.research/m3-tas-inc2/verification/`; baseline
`.research/m3-tas-inc2/baseline/`. One non-blocking documentation observation is recorded
in the final SUMMARY: two §6 compare example blocks retain illustrative numeric values
(they are labelled as observed shapes with exact values in `cli/`); the capability claim
and the M3-A15 row are unaffected. M3 is not complete and this block makes no claim about
any deferred item.

## 2026-11-15 — M3 format v2: checkpoint anchors and multi-device input, increment 3 (independent verification)

Independent verifier PASS for the uncommitted increment-3 tree. Tree under test:
`9643d74fd37097ebe487a398fe0a0259a3b3c7aa` (branch `agent/m3-complete`) plus the
increment-3 working-tree paths; the exact SHA-256 set is in
`.research/m3-v2/verification/tree/changed-paths.sha256.txt` and the implementation paths
are `crates/chronofork-core/src/input.rs`,
`crates/chronofork-test-backend/src/lib.rs`,
`crates/chronofork-sessions/src/{lib,movie,replay}.rs`,
`crates/chronofork-sessions/tests/{replay,movie_v2}.rs`,
`crates/chronofork-cli/src/{main,tas}.rs`, `crates/chronofork-cli/tests/tas.rs`, with the
increment-3 documentation paths. The verifier's only tracked edit is this dated block; no
commit, push or tag was made. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

Authoritative raw evidence (ignored paths): `.research/m3-v2/verification/`
(`SUMMARY.txt`, `target-matrix.txt`, `rows/A18-*.txt`–`rows/A27-*.txt`,
`rows/M3-A1-A17-regression.txt`, `audit/`, `targets/`, `cli/`, `demos/`, `probes/`,
`gate/`, `tree/`, `MANIFEST.sha256`); pre-change baseline: `.research/m3-v2/baseline/`
(captured at HEAD `9643d74` before any increment-3 edit). M3-A18–A27 were executed with no
failed criterion. The Fase A [M3 format-v2 acceptance plan](m3-format-v2-acceptance.md) and
ADR-0006 still carry their pre-verification status text (`planned - not yet executed` /
`not implemented`) until the staged documentation-promotion task updates them; this block
describes only what the artifacts prove. M3 is **not** complete: in-process rewind/state
cache and interactive stepping, rerecord/branch editing and real-hardware input TAS (plus
import/export adapters and HTTP/MCP/GUI/daemon) remain deferred and unchecked.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 33 targets, 422 passed, 0 failed, 0 ignored; no target below the 32-target baseline (counts in `target-matrix.txt`) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0, ends "all quality gates passed" |
| Whitespace | `git diff --check` | exit 0 |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0; 3 runs, 0-byte program stderr, byte-identical to the pre-change baseline |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0; 3 runs, 0-byte program stderr, byte-identical to the pre-change baseline |
| M2/M3-A1–A17 regression | the M2 target matrix, the M3-A1–A17 targets and `cargo test -p chronofork-cli --test session_storage` | every target exit 0 at or above the baseline counts |
| State probe | the baseline `session save/load` probe on two independent roots | `chronofork.store`, `session-keys` and `checkpoint.cfsnap` byte-identical to the baseline; `tick=7`, `state_hash=a4c253d2…c275` identical |
| v1 non-regression | fresh v1 recordings plus replayed pre-change fixture movies and transcripts | 45/45 comparisons PASS; movie SHA-256 `bb8b619d…1254`, `d89459a2…e185`, `eec8d021…766e` identical to the baseline |
| Determinism | two-process and debug/release v2 recordings and anchored replays | v2 bytes and replay blocks byte-identical; no host time in canonical bytes or reports |
| Suppression/dependency | suppression, structural and dependency scans | no added `#[allow]`/`#[ignore]`/`unsafe`/`todo!`; `Cargo.toml`/`Cargo.lock` diff 0 bytes; `cargo tree -d` clean; Mega Drive and backend-api diff 0 bytes |

M3-A18–A27 executed with no failed criterion: v2 container round-trip, exact canonical
size accounting and deterministic anchors; bounded v2 decode with the typed rejection
matrix (library plus 22 re-signed CLI mutations); anchor capture, ordering and
tick/state-hash binding; nearest-anchor seek with boundary equality to linear replay,
exact first divergence, the documented pre-anchor prefix delta proven by a negative
control, and v1/anchor-less fallback; schema-2 multi-device input with the golden
pre-schema-2 state hash, v1-mask equivalence and unchanged payload layout; v1
non-regression with one reader/writer, one snapshot parser, one compatibility predicate
and one replay loop; the CLI process contract, help and typed rejections; determinism and
debug/release equality; the full gate, demos, M1/M2/v1 regression; and scope honesty
(deferred items still unimplemented, no M3-complete claim).

Non-blocking observations recorded in the evidence SUMMARY: (1) the format-v2 acceptance
plan and ADR-0006 status text still reads `planned - not yet executed` / `not implemented`
and must be promoted by a separate documentation task using this evidence — it understates
the tree and makes no false capability claim; (2) anchored ranged replay intentionally does
not re-execute the pre-anchor prefix and reports `prefix_verified=0` while full-range
`verify` remains complete; (3) the decode taxonomy stays checksum-first as in v1, so a
raw byte flip is reported as a checksum mismatch. Full details, commands, exit codes and
hashes are in the ignored artifacts above; this dated block is a gate-result snapshot and
the per-target counts stay in the evidence.

## 2026-11-15 — M3 rewind state cache and interactive stepping, increment 4 (independent verification)

Independent verifier PASS for the uncommitted increment-4 tree. Tree under test:
`b8b0f935e40e08854c2155f1b5208f6fc91424d8` (branch `agent/m3-complete`) plus the
increment-4 working-tree paths; the exact SHA-256 set and diff hash are in
`.research/m3-rewind/verification/tree/` (`changed-files.sha256.txt`,
`working-tree.diff`, `state-comparison.txt`). The implementation paths are
`crates/chronofork-sessions/src/{lib,manager,movie,replay,session}.rs`,
`crates/chronofork-sessions/src/player.rs`, `crates/chronofork-sessions/src/player/cache.rs`,
`crates/chronofork-sessions/tests/player.rs`,
`crates/chronofork-cli/src/{main,tas,studio}.rs`,
`crates/chronofork-cli/tests/studio.rs`, with the increment-4 documentation paths. The
verifier's only tracked edit is this dated block; no commit, push or tag was made. Every
Cargo command used `CARGO_HOME=/tmp/t7-cargo`.

Authoritative raw evidence (ignored paths): `.research/m3-rewind/verification/`
(`SUMMARY.txt`, `target-matrix.txt`, `rows/A28-*.txt`–`rows/A34-*.txt`,
`rows/M3-A1-A27-regression.txt`, `audit/`, `targets/`, `cli/`, `studio/`, `demos/`,
`probes/`, `gate/`, `tree/`, `MANIFEST.sha256`); pre-change baseline:
`.research/m3-rewind/baseline/` (captured at the same HEAD with a clean tree). M3-A28–A34
were executed with no failed criterion (206/206 studio-transcript assertions PASS). The
durable docs and the CLI help still carry pre-verification wording (`planned - not yet
executed` / `not implemented` / "not independently verified") until the staged
documentation-promotion task updates them; this block describes only what the artifacts
prove. M3 is **not** complete: rerecord/branch editing and real-hardware input TAS remain
deferred and unchecked.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 35 targets, 458 passed, 0 failed, 0 ignored; no target below the 33-target/422-passed baseline (counts in `target-matrix.txt`) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0, ends "all quality gates passed"; same 35/458/0/0 |
| Whitespace | `git diff --check` | exit 0 |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0; 3 runs + 3 direct-binary runs, 0-byte program stderr, byte-identical to the pre-change baseline (986 B, `9b4dd564…4652`) |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0; 3 runs + 3 direct-binary runs, 0-byte program stderr, byte-identical to the baseline (1582 B, `912a5bb6…8996`) |
| M3-A28–A34 | `cargo test -p chronofork-sessions --test player`, `--lib`, `--release --test player`; `cargo test -p chronofork-cli --test studio`, `--release --test studio`; scripted `tas studio` transcripts | every target and row exit 0/passed; 74 sessions-lib, 16 player, 11 studio, 16 release-player, 11 release-studio passed |
| Isolation/determinism | two-process and debug/release studio scripts; external-manager and two-player library tests | byte-identical blocks, equal cache metrics, no pre-existing session mutation, no writes |
| M2/M3-A1–A27 regression | the M2 target matrix, M3-A1–A17/A18–A27 targets, CLI session/storage transcripts | every target at or above the baseline counts; 32/32 transcripts and 8/8 movie fixtures byte-identical |
| State probe | the baseline `session save/load` probe on two independent roots | `chronofork.store`, `session-keys` and `checkpoint.cfsnap` byte-identical to the baseline and across roots; `tick=7`, `state_hash=a4c253d2…c275` identical |
| M1 oracle differential | `python3 scripts/oracle-diff.py --skip-build` | exit 0; traces 3/3, final states 3/3, negative controls 3/3, scheduler/mask-gating controls 3/3 |
| Suppression/dependency | suppression, structural and dependency scans | no added `#[allow]`/`#[ignore]`/`unsafe`/`todo!`; no removed assertion; `Cargo.toml`/`Cargo.lock` diff 0 bytes; `cargo tree -d` clean; Mega Drive/core/backend-api/test-backend diff 0 bytes |

M3-A28–A34 executed with no failed criterion: cache capture keyed to verified steps with
no `SnapshotId` allocation or snapshot-map growth; cadence `N-1, 2N-1, …`; count/byte bounds,
deterministic evict-lowest-first and the exact `payload+256` metric; deterministic rewind
equal to fresh `Movie::verify`/`ReplayRange` replay at all 12 recorded ticks and at every
target class (cached, cached+replay, floor fallback, after eviction, caching disabled,
zero-step, current position); exact first-divergence attribution from floor and cache;
the fail-closed `tas studio` process contract, five block types, EOF/quit, read-only
non-mutation, 95 runs with only exit codes 0/1; isolation, cross-process and debug/release
equality; one player/cache implementation, one alignment rule, one codec/predicate/import
path, private cache and `pub(crate)` capture accessors; full gate, demos, v1/v2/M1/M2
regression; and scope honesty.

Non-blocking observations recorded in the evidence SUMMARY: (1) the acceptance plan,
ADR-0007 and the architecture/requirements/roadmap docs still say the increment is
planned/not implemented (and `tas --help` says "not independently verified"); these
understate the verified tree but make no false capability claim and must be promoted by the
separate documentation task using this evidence; (2) the top-level `chronofork help` output
adds two usage lines and three descriptive lines for `tas studio`; this additive help-only
change documents the new surface but is not enumerated in M3-A33's declared-exception list
and should be recorded as a declared addition (or the row amended) by that task; (3) the
recorded cache-path transcript shows deterministic evict-lowest-before-insert behaviour
(`goto 12` floor/replay 12, evictions 8; `goto 8` cache start step 1 because repeated
captures evict lower indices) — intended, documented and asserted. Full details, commands,
exit codes and hashes are in the ignored artifacts above; this dated block is a gate-result
snapshot and the per-target counts stay in the evidence.

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
