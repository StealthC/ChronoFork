# Quality Gates

A feature is not complete because it compiles. These gates must be executed and
recorded. Results below are from the bootstrap host; update them when the
toolchain or environment changes.

**Current status:** M8 phase 8.3 (complete VDP) is **implemented, independently verified
(rounds 1–3) and reviewed (rounds 2–3)**, and merged to `main` through the phase-8.3 PR
(issue #16); the phase gate below was executed and passed (evidence in the
[VDP state differential](../research/vdp-state-differential-evidence.md)). Phase 8.4
(Z80 core, arbitration, YM2612/PSG, 48 kHz mixer/resampler, payload v6) is
**complete on `agent/8.4-z80-audio` and gate-executed there, with independent
verification, review and integration pending**; the run is recorded on 2026-11-20
[below](#phase-84-z80-audio-gate-executed-pass--evidence-recorded). The latest dated
verification block remains the [2026-11-17 M8 phase 8.1 cartridge,
region, TMSS and boot](#2026-11-17--m8-phase-81-cartridge-region-tmss-and-boot-integration-gate)
(81 targets / 1,047 passed, 0 failed, 0 ignored; both demos byte-identical or explained only
by the documented v3 state-hash bump; GUI run deterministic), which verified the worktree on
branch `agent/8.1-cartridge-region-tmss` at base `a5de42a` after an independent review
returned PASS; the work is committed locally on that branch and **not** merged. Before it,
the [2026-11-17 M7 human-facing GUI
verification](#2026-11-17--m7-human-facing-gui-independent-final-verification-pass4)
(76 targets / 988 passed, 0 failed, 0 ignored, both demos byte-identical, M7 acceptance rows
`executed - passed`), which verified the worktree on branch `agent/m7-gui` at base
`93d47ed`; that work was merged to `main` at `cb05836` (PR #6), so M7 is `Complete`. Before
it, the [2026-11-17 M6 agent-native interfaces
verification](#2026-11-17--m6-agent-native-interfaces-independent-pass-2-verification-pass)
recorded 67 targets / 800 passed on the M6 worktree, also merged at `cb05836` (PR #6), and the
[2026-11-16 M5 tracing and scripting
verification](#2026-11-16--m5-tracing-and-scripting-independent-verification-pass)
recorded 57 targets / 662 passed on the M5 worktree. For the earlier anti-slop
refactor's implementation bytes, the authoritative recorded run is
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

## Phase 8.3 VDP gate (executed; PASS — evidence recorded)

```sh
cargo test --workspace
./scripts/check.sh
python3 scripts/oracle-vdp-diff.py --out-dir .research/evidence/8.3
```

Executed at the merged phase-8.3 HEAD: all three commands exit 0 (workspace tests,
`./scripts/check.sh` and the VDP differential); the run artifacts are under
`.research/evidence/8.3` and summarized in the
[VDP state differential](../research/vdp-state-differential-evidence.md).

The first two commands are the required local gate. The VDP differential drives the
pinned Exodus oracle (`exodus-mcpd`, revision `ec005615…`) and ChronoFork over
independently authored synthetic VDP programs at synchronized instruction boundaries. It
records per-program ROM/state/VRAM hashes, the first mismatch with register/address
context, a negative control and a skip-with-reason when the oracle or the built CLI is
absent (exit 3, never a pass). What it proves and what it does **not** claim:

- It compares the oracle-visible status and H/V counter readbacks, CPU state and work RAM
  at the stop marker, and ChronoFork's canonical VDP registers, full 64 KiB VRAM and CRAM
  readback against the values the synthetic program programmed and wrote.
- The pinned headless oracle exposes no VDP register or memory reflection and its
  data-port reads return the stale write latch, so VRAM/CRAM/VSRAM content is not
  byte-compared against Exodus. Pixels are pinned internally by `vdp_frame`; there is no
  cross-emulator framebuffer comparison.
- The absolute status bits and H/V counter values differ at reset because the two models
  start at different scanline phases and the H counter is approximate. These values are
  recorded as scoped divergences with their deltas, not treated as equal.
- VSRAM data-port reads are undecoded in ChronoFork; DMA state is not externally
  observable. Both are covered by backend unit tests, not by this oracle differential.
- A full workspace count is recorded with the run; do not copy a count from this
  definition into a verification claim.

See [VDP state differential evidence](../research/vdp-state-differential-evidence.md).

## Phase 8.4 Z80 audio gate (executed; PASS — evidence recorded)

```sh
python3 scripts/oracle-audio-state-diff.py --out-dir .research/evidence/8.4
./scripts/check.sh
cargo test -p chronofork-megadrive --test audio_snapshot
```

Executed at the phase-8.4 branch HEAD on 2026-11-20: the three commands exit 0, the
audio-state differential reports **PASS** for all 25 compared device-state fields and
`./scripts/check.sh` remains green (format, check, Clippy, workspace tests and rustdoc).
Run artifacts are under `.research/evidence/8.4` and summarized in the
[audio state differential](../research/audio-state-differential-evidence.md).

The gate was re-executed at the phase-closure HEAD on 2026-11-20, after merging
`origin/main` `b8d63a1` and applying the documentation closure: `./scripts/check.sh`
and `git diff --check` both exit 0. The closure commit is Conventional and scoped to
`docs/` + `AGENTS.md`; no `crates/` or `scripts/` file is touched. No test count is
copied into this record — run the gate yourself.

**Repair round (t5, 2026-11-20).** A review of the closed phase raised five findings and
all five were repaired: the YM2612 `0x28` key-on channel derivation (R1-F1), the PSG noise
shift periods and the tone-2-clocked mode (R1-F2), the `0xA11100` bus-request polarity, now
the hardware `ZBR` line per the owner's decision (R1-F3), the `execution-model.md`
`CFMDSTATE6` coverage (R1-F4) and the phase-8.4 inspection-surface record (R1-F5).
R1-F2 and R1-F3 are canonical-state behaviour fixes, so state hashes moved while payload
lengths and the configuration hash did not; the affected goldens were re-pinned
mechanically with the cause recorded (`backend::tests::snapshot_payload_and_hashes_are_golden`
and six `chronofork-cli --test debug` `state_hash` expectations, whose RAM/VRAM dumps stayed
byte-identical), four regression tests were added and none was removed, ignored or weakened.
The recorded internal PCM fixture was re-probed and is **unchanged**
(`73855433…`, 48 000 frames) because that fixture leaves the PSG noise channel silent; see the
attributed supersession note in the
[audio state differential](../research/audio-state-differential-evidence.md) and the ignored
`.research/8.4-r2/` artifacts. The gate was re-run green at the repair HEAD, and the repair
commit changed no `scripts/` file.

R1-F2 and R1-F3 also move the `megadrive demo` stdout, so the current demo baseline is
recorded here for the independent verification (three runs each, byte-identical, empty
stderr): `cargo run -q -p chronofork-cli -- megadrive demo` writes 1994 stdout bytes with
SHA-256 `e5b9ecffacb2364c889b2ef969c5c2ee86e4da892c4c0e9b22baa8163ca99609`. The closure-head
value at `8382255` was `794252b3fc3d1c54b95a0d54664fa34ef9916df29c67e321d0009a2f43c138b1`
(same length), so a verification must compare against the post-repair value. The `demo`
(986 bytes, `9b4dd564…`) and `gui run` (2842 bytes, `d3abf57a…`) stdout hashes are unchanged
from the closure head, and raw files are under the ignored `.research/8.4-r2/demos/`.

The phase-8.4 differential harness was also realigned to the repaired polarity and re-run:
with the **same** bus-request byte on both emulators (the former `1`/`0` workaround removed,
one byte-identical ROM, `sha256 2d7c469e…`) it exits **0** with **PASS** for all 25 compared
fields at the shared Z80 `HALT` boundary (HALT PC 186; both corrupted-field controls and the
missing-oracle exit-3 control pass). The request-asserted probe that previously demonstrated
the divergence (oracle Z80 `PC = 4`, ChronoFork `PC = 186`) now agrees on `PC = 4` on both
sides, which is the direct evidence that the `ZBR` polarity repair took effect; the note on
the [audio state differential](../research/audio-state-differential-evidence.md) records it.
Raw report: `.research/8.4-r2/evidence-r3/` (ignored).

Two harness executions are recorded, both exit 0 with **PASS** 25/25 at the shared Z80
`HALT` boundary, all three corrupted-field controls (`corrupt-z80-af`, `corrupt-psg-tone`,
`corrupt-ym-status`) detected plus the missing-oracle child exiting 3, one ROM shared by both
emulators (`sha256 2d7c469e…`, `oracle_bus_request` = `chronofork_bus_request` = 0) and the
request-asserted probe agreeing at Z80 `PC = 4` on both sides:

1. at commit `2001286` — artifacts `.research/8.4-t10/evidence/` (ignored). That commit is a
   **historical** HEAD, not the current one.
2. at the t12 repair tree, which is the **current HEAD** — this commit — with production code
   unchanged at `06f5e95`; artifacts `.research/evidence/8.4-final/` (ignored), the
   artifacts of record. The harness's own recorded `commit` field is `8500a4b` (this commit's
   parent) because the run predates the commit, and `crates/` is identical in both.

The exit-0 condition now includes the probe: `z80_pc_agrees` is part of the PASS gate (not
only a report field), so a probe disagreement is a divergence and exits 1. That gate was
exercised with an out-of-repo negative control (probe agreement forced false: exit 1, status
`divergence`, `arbitration_probe_agrees: false`).

The audio differential drives the pinned `exodus-mcpd` oracle (revision `ec005615…`)
and ChronoFork over one independently authored synthetic ROM and compares **device
state only** at a shared Z80 `HALT` boundary: the Z80 register file, the PSG tone/
volume/noise/source-control registers and the readable YM2612 status byte. It never
reads or compares audio samples. What it proves and what it does **not** claim:

- The recorded run predates the phase-8.4 repair round: the 68000 fixture differs in
  the Z80 bus-request byte because the model then had the inverse execution polarity of
  the pinned hardware model, and the literal same-ROM probe is recorded as a scoped
  arbitration divergence (oracle Z80 stops at `PC = 4`, ChronoFork reaches `PC = 186`).
  The repair implements the hardware `ZBR` polarity (asserting `0xA11100` bit 8 stops the
  Z80 and gives the 68000 the window; read bit 8 reports ownership), so that divergence is
  historical. No 68000 or arbitration equivalence is claimed.
- The pinned oracle publishes no YM2612 register/channel reflection (only Audio
  Logging), so YM2612 register and channel state is **not** compared; only the readable
  status byte is. PSG noise shift state is time-dependent and excluded; the unused
  PSG channel-4 tone register and Z80 alternate registers have no ChronoFork
  counterpart and are excluded with reasons.
- Negative controls corrupt a Z80 field, a PSG field and the YM status and require the
  exact first mismatch; a missing-oracle child invocation must exit **3** with a reason.
  The harness exits 0 only when every field matches and both controls pass, so it cannot
  report a silent PASS.
- Internal PCM determinism is pinned by hash in the backend tests, not by
  cross-emulator sample equality; no sample-equality claim is made anywhere.

See [audio state differential evidence](../research/audio-state-differential-evidence.md).

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

## 2026-11-16 — M3 rerecord and branch editing, increment 5 (independent verification — FAIL, not verified)

**Verdict: FAIL.** This is a dated gate-result record for the uncommitted increment-5
tree (`3f76d5f` plus the rerecord working-tree paths). It is **not** a verification PASS:
the increment must not be described as implemented-and-verified, the acceptance rows must
not be promoted to `executed - passed`, and M3 remains incomplete.

Tree under test: HEAD `3f76d5ffee680d2c7271c7c1d81d8c5a44055675` (branch
`agent/m3-complete`) + the uncommitted rerecord paths recorded in
`.research/m3-rerecord/verification/tree/` (git status, diff, changed-path list, manifest
hashes). The verifier's only tracked edit is this dated block; all other evidence is under
the ignored `.research/m3-rerecord/verification/` path. Every Cargo command used
`CARGO_HOME=/tmp/t7-cargo`. Pre-change baseline:
`.research/m3-rerecord/baseline/` (HEAD `3f76d5f`, clean tree).

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 37 targets, 497 passed, 0 failed, 0 ignored; no pre-existing target below the 35-target/458-passed baseline |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0; same 37/497/0/0 |
| Whitespace | `git diff --check` | exit 0 |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0; 3 cargo + 3 direct-binary runs, 0-byte program stderr, byte-identical to baseline (986 B, `9b4dd564…4652`) |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0; 3 cargo + 3 direct-binary runs, 0-byte program stderr, byte-identical to baseline (1582 B, `912a5bb6…8996`) |
| v1/v2/M2/M3 regression | baseline transcript replay, movie byte comparison, two-root session probe, target matrix | 134/134 regression checks PASS; movies/store/keys/checkpoint/state hash byte-identical; M1 oracle exit 0 (3/3 traces, final states, controls) |
| Edit acceptance rows | M3-A35–A44 executed per `rows/` | A35, A39, A43, A44 PASS; **A38, A40, A41, A42 FAIL**; A36 and A37 have unexecuted sub-criteria |

Blocking findings (details and commands in `SUMMARY.txt` and `rows/`):

- **F1 (blocker, A38.4/A40.1/A40.3):** an invalid-UTF-8 `--name` argument panics in
  `std::env::args()` with exit 101 and Rust panic text on stderr instead of the required
  typed rejection. Reproduced with a Python `execv` probe
  (`cli/edit/results/edit-name-invalid-utf8/`).
- **F2 (blocker, A41.4/A42):** rebuilt-anchor bytes embed the manager-allocated
  `origin_session`; two identical rebuild edits of the same source in one
  `SessionManager` produce different output bytes (first differing offset 547). This
  violates "a prior edit leaves no state in the manager that changes a later edit's
  bytes". Independent harness scenarios in `edit/determinism/`.
- **F3 (blocker, A40 planned targets/A42.1):** the accepted ADR-0008/plan module and test
  target `tas_edit` is implemented as `edit`
  (`crates/chronofork-cli/src/edit.rs`, `tests/edit.rs`); the documented command
  `cargo test -p chronofork-cli --test tas_edit` exits 101 "no test target named
  tas_edit", and no ADR/plan amendment records the spelling change.
- **F5 (low, A42.3):** `EditOutcome::into_movie` has no caller (speculative public API).
- **F6 (verification gap, A36.6/A37.5):** the output-size limit and the per-anchor /
  cumulative anchor byte budgets have no executing edit fixture with the fixed-size
  synthetic checkpoint; these sub-criteria remain unexecuted.

Non-blocking observation O1: the ADR, acceptance plan, execution model, requirements,
milestones, roadmap and `docs/README.md` still say the increment is "design only" / "no
implementation exists" while the tree contains the implementation and 39 new passing
tests. That wording understates the tree (it does not overclaim a capability) and must be
promoted by the documentation task only after the blocking findings and the unexecuted
sub-criteria are resolved and a verification re-run returns PASS.

No commit, push, tag or merge is authorized by this block. Full commands, exit codes,
artifact paths and SHA-256 manifests are in `.research/m3-rerecord/verification/`.

## 2026-11-16 — M3 rerecord and branch editing, increment 5, repair round 2 (independent verification PASS)

**Verdict: PASS** for the repaired uncommitted increment-5 tree under test (`3f76d5f` plus
the rerecord working-tree paths). This PASS supersedes the round-1 FAIL block above: the
blocking findings F1/F2/F3/F5 and the unexecuted A36.6/A37.5 sub-criteria were repaired
and re-executed from the corrected tree. It remains a verification record only: M3 is
**not** complete, real-hardware input TAS remains unimplemented, and the durable docs/plan
promotion is a separate task (t6) that must use this evidence.

Tree under test: HEAD `3f76d5ffee680d2c7271c7c1d81d8c5a44055675` (branch
`agent/m3-complete`) + the rerecord paths recorded in
`.research/m3-rerecord/verification-round-2/tree/` (status, diff, manifest hashes). The
verifier's only tracked edit is this dated block; every other artifact is under the
ignored evidence root. Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`. Pre-change
baseline: `.research/m3-rerecord/baseline/` (HEAD `3f76d5f`, clean tree).

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 37 targets, 502 passed, 0 failed, 0 ignored; no target below the 35-target/458-passed baseline |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0; same 37/502/0/0 |
| Whitespace | `git diff --check` | exit 0 |
| CLI demo | `cargo run -p chronofork-cli -- demo` | exit 0; 3 cargo + 3 direct-binary runs, 0-byte program stderr, byte-identical to baseline (986 B, `9b4dd564…4652`) |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | exit 0; 3 cargo + 3 direct-binary runs, 0-byte program stderr, byte-identical to baseline (1582 B, `912a5bb6…8996`) |
| v1/v2/M2/M3 regression | baseline transcript/movie replay, two-root session probe, target matrix | 134/134 checks PASS; movies/store/keys/checkpoint/state hash byte-identical; M1 oracle 3/3 traces, final states and controls |
| Edit acceptance rows | M3-A35–A44 per `rows/` | **all PASS**; A36.6/A37.5 now executed with bulk-payload fixtures; F1/F2/F3/F5 closed |

Repair closure evidence:

- **F1 closed:** `main.rs` collects `args_os` and rejects non-UTF-8 argv through the typed
  CLI error path. CLI probe: exit 1, empty stdout, `chronofork: command-line arguments
  must be valid UTF-8`, no output/temp file, valid argv still succeeds; targeted
  `invalid_utf8_arguments_are_typed_failures_not_panics` passes.
- **F2 closed:** rebuilt anchors carry the documented canonical fixed snapshot/origin
  identity (v2 recording keeps its session-assigned identity). Independent harness
  same-manager repeated-rebuild scenario now passes in debug and release; targeted
  `rebuild_anchors_do_not_depend_on_manager_history` asserts byte equality, identical
  reports, canonical ids, verification and anchored seek. v1/v2 recording movies remain
  byte-identical to the baseline.
- **F3 closed:** CLI adapter and test target are `tas_edit` as accepted; both
  `cargo test -p chronofork-cli --test tas_edit` (19 passed) and the release variant pass.
- **F5 closed:** unused `EditOutcome::into_movie` removed; structural audit finds no
  speculative public item.
- **F6 closed:** executable bulk-payload fixtures drive per-anchor
  `AnchorSnapshotTooLarge`, cumulative `AnchorTableTooLarge` and writer
  `FileTooLarge` (source at exactly `MAX_MOVIE_BYTES`, one-byte branch name) with typed
  errors and no returned movie.
- ADR-0008/acceptance-plan repair clarifications audited: no decision, limit, flag, block
  field, v1/v2 movie byte or recording byte changed; all 10 rows remained `planned`
  pending this verification.

Independent verification beyond the product tests: 13/13 harness scenarios in debug and
release (source non-mutation, prefix/suffix exactness, truncation without executing the
dropped suffix, anchors/checkpoint, branch metadata/chains/isolation, formats/device
boundary, incompatible-backend import, determinism), 301/301 CLI process-contract checks,
19/19 re-signed tamper probes, 134/134 baseline regression checks, 46/46 demo checks, and
structural/suppression audits. Full commands, exit codes and hashes are in
`.research/m3-rerecord/verification-round-2/` (`SUMMARY.txt`, `rows/`, `target-matrix.txt`,
`MANIFEST.sha256`).

Non-blocking observation O1: the ADR, acceptance plan, execution model, requirements,
milestones, roadmap and `docs/README.md` still carry pre-verification wording ("design
only", "no implementation exists", rows `planned - not yet executed`, milestone unchecked).
This understates the now-verified tree and must be promoted by the documentation task (t6)
using this evidence; this block is the gate result, not a capability/roadmap promotion.
Real-hardware input TAS remains unimplemented and M3 remains incomplete. No commit, push,
tag or merge is authorized by this block.

## 2026-11-16 — M3 hardware input TAS, final increment (independent verification PASS; M3 completion audited)

**Verdict: PASS** for the hardware-TAS increment and the M3 completion audit on the
recorded tree under test (`85af523` plus the increment worktree paths). Every acceptance
row M3-A45–A52 in the [M3 hardware acceptance plan](m3-hardware-acceptance.md) is
`executed - passed` with an executed command, exit code and artifact. M3-A53 passes its
scope-honesty/no-overclaim audit; its durable-doc half (ADR-0009 status wording,
executed-passed row statuses, milestone checkbox and acceptance-plan execution record)
is the separate documentation task t6, explicitly outside this verification's write
scope. **M3 may be marked complete for the documented deterministic minimal-slice scope
once t6 records that promotion and t7 audits it.** This block is a verification record
only: it does not commit, push, tag, merge or promote capability docs.

Tree under test: HEAD `85af5238fabf204bbb885df3823c604212edd948` (branch
`agent/m3-complete`) + 19 working-tree paths recorded in
`.research/m3-hardware/verification/tree/` (status, diff, manifest hashes). The verifier's
only tracked edit is this dated block; all other artifacts are under the ignored
`.research/m3-hardware/verification/` root (1053 files, `MANIFEST.sha256`).
Every Cargo command used `CARGO_HOME=/tmp/t7-cargo`. Pre-change baseline:
`.research/m3-hardware/baseline/` (HEAD `85af523`, clean tree; manifest verified before
and after this run).

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | exit 0 |
| Compilation | `cargo check --workspace` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | exit 0; 39 targets, 532 passed, 0 failed, 0 ignored (baseline 37/502/0/0) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| Full script | `./scripts/check.sh` | exit 0; same 39/532/0/0 |
| Whitespace | `git diff --check` | exit 0 |

Target matrix: every baseline target present with `passed >= baseline` and 0 failed;
two new targets (`tests/controller.rs` 6, `tests/tas_hardware.rs` 13) and +11 unit/
property tests explain the 30-test delta; no assertion deleted or relaxed.

| Regression surface | Result |
|---|---|
| CLI + Mega Drive demos | 3 cargo + 3 direct-binary runs each, exit 0, 0-byte program stderr, pairwise and baseline byte-identical (986 B `9b4dd564…4652`; 1582 B `912a5bb6…8996`); Mega Drive fork/source hashes equal baseline |
| v1/v2/edit/studio fixtures | 59/59 transcripts (exit/stdout/stderr) and 19/19 movies byte-identical to baseline; studio stdout `a6f23565…` |
| Durable two-root session probe | store/keys/checkpoint and `state_hash a4c253d2…` byte-identical; only `.cfmeta` wall-clock differs |
| Mega Drive state probes | all 3 oracle ROM state/trace files byte-identical to baseline (`dcbfcfdc/5237c672`, `9962a19a/6c278394`, `3bce3df7/bc61867b`) |
| M1 oracle | `oracle-diff.py --skip-build` exit 0, status=pass, 3/3 traces, 3/3 final states, 3/3 negative controls, 3/3 scheduler controls, masked gating 1/1; per-ROM JSON fields identical to baseline; ROMs contain no controller-window access |
| Differential serialized-state probe | pre-change (`git archive 85af523`) vs post-change payload/envelope/meta byte-identical (`db6e61db…` / `87c57711…` / `a8b7ffb0…`); pre-change snapshot restores post-change and vice versa; 23/23 per-step hashes equal for neutral vs extreme frames |
| Controller semantics | per-bit `0x40|(!mask&0x3F)`, neutral `0x7F`, A/Start/high bits inert, two independent ports, writes/debugger typed errors, step-scoped clearing on success and failure, input absent from snapshot/state/config |

Hardware-TAS acceptance highlights (raw evidence in
`.research/m3-hardware/verification/`): hardware record/play/verify with input-dependent
state (`6d1677d5…` neutral vs `2d97debc…` Up+B, first divergence index 4); deterministic
debug/release movie bytes (`3e11e762…`); scripted edit and studio (11 blocks, 2 cache
evictions, goto-2 state equal to ranged verify); two-controller v2 mapping with ports
2/3, axes and keys inert; 18 typed-error transcripts for missing/dir/short/4 MiB+/
unreadable/wrong ROM, wrong backend, `--root`/`--session`, `compare --rom` and
missing/duplicate `--rom`; ROM read-only and never embedded. Structural/suppression
audits: no added `#[allow]`/`#[ignore]`/`unsafe`/`todo!`, no deleted test or assertion,
`Cargo.toml`/`Cargo.lock` diff 0 bytes, `cargo tree -d` empty, one controller read path
and one manager-selection path.

M3 completion audit (executed): anchors/seek (movie_v2 8, replay 23), rewind/interactive
stepping (player 16, studio 11), rerecord/branch editing (edit 22, tas_edit 19),
multi-device input (tas 21, movie_v2 8) and real-hardware input TAS (controller 6, CLI
hardware 13, rows A45–A52) all pass; import/export adapters remain documented-only; no
O(1)/persistent-cache/real-controller-timing/cycle-accuracy/game-compatibility claim was
found. **M3 may be marked complete after t6 promotes the durable docs and acceptance
record; formal completion must wait for that promotion and the t7 audit.**

Non-blocking observation: the durable docs still carry pre-verification wording
("Proposed", "planned - not yet executed", "no controller code exists"). That understates
the now-verified tree (it is not an overclaim) and is exactly the t6 documentation
promotion this block requires; this verifier did not edit those paths. AS-14 note:
`backend.rs`/`bus.rs` already exceeded the 800-line review trigger at HEAD and the
controller additions are cohesive, tested and within the accepted ADR-0009 scope.

## 2026-11-16 — M4 debugging depth (independent verification PASS; promotion audit)

The independent verifier executed all M4-A1–A17 rows on the committed tree at HEAD
`df983214bb145a3e2e073949d81814cf808b69a0` (branch `agent/m4-debugging-depth`, clean
tree). Every row used the acceptance plan's exact command (22 row commands plus 2
supplementary row parts); all exited 0 with the expected counts, 0 failed and 0 ignored.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Exit 0 |
| Compilation | `cargo check --workspace` | Exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 |
| Tests | `cargo test --workspace` | 45 targets, 596 passed, 0 failed, 0 ignored |
| Full script | `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` | Exit 0, ends "all quality gates passed" |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Exit 0 |
| Demo | `cargo run -p chronofork-cli -- demo` | Exit 0; SHA-256 `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652`, empty stderr |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` | Exit 0; SHA-256 `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996`, empty stderr |

The pre-change baseline was 39 targets / 532 passed; no target is below baseline and the
M4 diff adds no `#[ignore]`, no `#[allow]` and deletes no test. Both demos were byte-identical
to the baseline in six runs each. The non-perturbation probe compared observation off/on
for 1,700 per-step states (state hash, config hash, payload bytes/version, inspection and
memory reads) with 0 mismatches; existing movie bytes matched between the baseline build
and HEAD (11/11 checks); the pinned-oracle differential was re-run at HEAD and remained
PASS 3/3 with 3/3 negative and 3/3 scheduler controls. The claim audit covered 44 statements
with no prohibited overclaim; its three low-severity precision notes (fixed 256-entry log
ring, adjacent instruction-fetch caveat, `backend describe` capability line) were fixed at
`6b2a567` and re-gated. Raw evidence, including per-row logs and the manifest, is in the
ignored `.research/m4/verification/` tree. This block is the M4 gate record; M4 is complete
for the documented deterministic minimal-slice scope on Linux.

## 2026-11-16 — M5 tracing and scripting (independent verification PASS)

The final independent verifier executed the [M5 acceptance
plan](m5-tracing-and-scripting-acceptance.md) on the worktree
`/home/stealthc/projects/ChronoFork/.agents/worktrees/m5-tracing-scripting`, branch
`agent/m5-tracing-scripting`, HEAD and base
`84f1476466c14e72f911689845c390173982fd81`. At verification time the implementation was
complete and **uncommitted**; this block verifies the worktree. It was subsequently merged
to `main` at `0c4e9f0` (PR #4) with the same tree. The first T7 verification
round returned `needs_revision` with findings F1–F10; T11 repaired every finding and T12
re-ran the full matrix and returned **PASS** (2026-11-16). T10 then reviewed the integrated
result and returned PASS. The T7 row executions remain round-1 evidence only; the final
verdict is T12's.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Exit 0 |
| Compilation | `cargo check --workspace` | Exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 |
| Tests | `cargo test --workspace` | 57 targets, 662 passed, 0 failed, 0 ignored |
| Full script | `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` | Exit 0, ends "all quality gates passed" |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Exit 0 |
| Contract: script CLI | `cargo test -p chronofork-cli --test script` | 3 passed, 0 failed, 0 ignored |
| Contract: CLI package | `cargo test -p chronofork-cli` | 117 passed across 10 targets, 0 failed, 0 ignored |
| Contract: fmt/Clippy | CLI-subtree fmt and `clippy -p chronofork-cli --all-targets -- -D warnings` | Exit 0 |
| Demo | `cargo run -p chronofork-cli -- demo` ×3 | Exit 0, 3/3 byte-identical, SHA-256 `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652`, empty stderr |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` ×3 | Exit 0, 3/3 byte-identical, SHA-256 `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996`, empty stderr |

The pre-M5 baseline was **45 targets / 596 passed / 0 failed / 0 ignored**; no target is
below baseline and the M5 diff adds no `#[ignore]`, no production `#[allow]`, no `unsafe`
and deletes no test. All 36 test-row commands (M5-A1–A34 plus the two A35 parts) were
re-executed with their exact plan commands (exit 0, every filter matched); A36–A39 were
re-verified by their component commands. Both demos were 3/3 byte-identical with empty
stderr; the baseline movie, Mega Drive state/trace and durable-store artifacts were
**37/37 OK**
(`demo.cfmovie` `160453c6…`, `md-state.bin` `e42c66ed…`, `md-trace.tsv` `b15d95bb…`,
store `24fc2f54…`/`2ab51705…`/`4de16840…`/`062cb9c4…`); the M1 state-hash anchors passed
(11 CLI debug tests), including reset `7dcab59e…` and five steps `d354cb24…`. Canonical
trace capture bytes were unchanged from round 1 (`e39cb42f…`, `5afe37…`), proving no trace
format change; the non-perturbation probe made **734 comparisons with 0 mismatches**
(40 ticks × both backends × nine observables plus snapshot and post-tracing movie bytes);
the pinned M1 oracle rerun matched 3/3 traces, 3/3 final states, 3/3 negative controls and
3/3 scheduler controls with schema `chronofork.oracle-diff.v2`. The T10 integrated review
covered the architecture boundaries, single-source-of-truth codec/capture/query/compare/
script/CLI logic, typed rejection matrix and bounds, and found no overclaim.

**Scoped test-only allow exception (F9).** `crates/chronofork-trace/src/lib.rs` carries
`#![cfg_attr(test, allow(clippy::expect_used))]` solely so its `#[cfg(test)]` unit tests
may use message-bearing `expect("…")`, which `AGENTS.md` allows for tests. It is
test-only, matches the pre-existing sessions-crate pattern, adds no production `allow`, and
`cargo clippy --workspace --all-targets -- -D warnings` passes. This is the only allow
attribute in the M5 code.

**Recorded precision limits (no T12 action required):** TSV export is bounded by the
retained record count (at most 1,000,000 rows) and has no separate byte cap, so it can
exceed the 64 MiB canonical `MAX_TRACE_BYTES`; the sandbox is a deterministic capability
boundary with metadata-then-operation checks, not OS/process isolation or a race-free or
hostile-code security boundary; wall-time checks occur at statement boundaries and every
256 operations, so one long but step/movie-bounded builtin is not preempted mid-call;
`string_bytes` also charges print text; and diagnostic stderr may name paths while reports
and stdout never do. These are recorded as M5-FU1–FU3 in the
[milestone](../roadmap/milestones.md#m5--tracing-and-scripting) rather than reopened.

Raw evidence is in the ignored `.agents/notes/m5/t12/` tree
(`gate/check.log`, `gate/target-matrix.txt`, `gate/contract-commands.log`,
`gate/demos-artifacts-trace.log`, `rows/`, `demos/`, `audit/`), with
`t12-verification.md`, `t10-review.md` and `t11-repair.md` beside it. This block is the M5
gate record; M5 is verified for the deterministic synthetic backend and the minimal Mega
Drive slice on Linux and is merged to `main` at `0c4e9f0` (PR #4); it is not released.

## 2026-11-17 — M6 agent-native interfaces (independent pass-2 verification PASS)

The independent verifier executed the [M6 acceptance
plan](m6-agent-interfaces-acceptance.md) on the **uncommitted** worktree
`/home/stealthc/projects/ChronoFork/.agents/worktrees/m6-agent-interfaces`, branch
`agent/m6-agent-interfaces`, HEAD and base `origin/main`
`93d47ed308874be0e8e7060641c238c7f6809e31`. At verification time the implementation was
complete and uncommitted (tracked diff: `Cargo.lock`, `Cargo.toml`, the CLI `Cargo.toml`,
`src/main.rs` and `tests/session_storage.rs`; untracked: the two new crates, the CLI
`src/agent/` module, `src/session.rs`, `tests/m6.rs` and three M6 documents). **This block
verifies that worktree only: no commit, merge, push or publication was authorized at
verification time; the work was subsequently merged to `main` at `cb05836` (PR #6).** The round-1 review
returned `needs_revision` with findings M6-R1–M6-R10; the repair closed the code-side
findings and added seven tests, and the **pass-2** re-execution returned **PASS**
(2026-11-17 02:27–02:40). A round-2 review then returned pass with no regression. The round-1
row executions (`ROWS.md`) remain round-1 evidence only; the promotion basis is pass 2.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Exit 0 |
| Compilation | `cargo check --workspace` | Exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 |
| Tests | `cargo test --workspace` | 67 targets (57 binaries + 10 doc-tests), 800 passed, 0 failed, 0 ignored |
| Full script | `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` | Exit 0 (`CHECK_EXIT=0`) |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Exit 0 |
| Thread safety | `cargo test -p chronofork-service --test service app_service_is_send_and_sync -- --exact` | 1 passed — `AppService: Send + Sync` |
| Demo | `cargo run -p chronofork-cli -- demo` ×3 | Exit 0, 3/3 byte-identical, SHA-256 `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652`, empty stderr |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` ×3 | Exit 0, 3/3 byte-identical, SHA-256 `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996`, empty stderr |

**Non-regression accounting.** The pre-M6 baseline was **57 targets / 662 passed / 0 failed /
0 ignored**; the M6 tree has 67 targets and every pre-existing target is still present. The 57
pre-existing targets contribute exactly **662 passed** (unchanged baseline) and the 10 new M6
targets contribute **138** (service lib 45, service `tests/service.rs` 22, transport lib 9,
client 10, http 21, mcp 17, parity 3, cli `tests/m6.rs` 11), so 662 + 138 = 800. The repair
added exactly seven tests over round 1 (793 → 800). The executed test set is a strict
superset (+7, −0); no pre-existing test was deleted and per-file `#[test]` counts match HEAD.
Two assertions in `crates/chronofork-cli/tests/session_storage.rs` were re-pointed as a
consequence of the documented CLI changes (`--json` became a valid flag, and the invalid
`--name` diagnostic now comes from the service catalog); that file stays at 18 green tests and
the change is commented in the test. Both demos are 3/3 byte-identical to the baseline with
empty stderr. No `#[ignore]` exists in the tree and no new third-party dependency was added
(`Cargo.lock` grows 40 → 42 packages, only the two new workspace crates); there is no
`unsafe`.

**Byte-level parity and canonical outputs.** Direct/HTTP/MCP parity is three tests in
`crates/chronofork-transport/tests/parity.rs`: the documented **23-step scenario** covering all
16 operations is byte-identical on the direct, HTTP and MCP surfaces
(`the_documented_scenario_is_byte_identical_on_direct_http_and_mcp`) and reproduces
identically on a fresh run, and `error_scenarios_agree_across_the_three_surfaces` compares
**seven error sections** across the applicable surfaces (unknown session, invalid input,
unknown operation versus MCP `-32602`, foreign session, unknown token, unknown route/wrong
content type, body too large). The **19-case rejection matrix** is a different test and
belongs to the HTTP wire contract (M6-A12, `crates/chronofork-transport/tests/http.rs`); it is
not a parity result and is not claimed as one. The live `capabilities` body is 1506 bytes with
SHA-256
`d5f8282e03c9eec75d0568a2829bb1790174f6e2762acfe0765d6676eee73a74` on both HTTP and MCP;
the experiment artifact `artifact_id`
`1da41a1f4f2b63be537bde55b8896fca510085ca15c7a5cf8d8f0387c6590de1` equals the SHA-256 of
the canonical artifact bytes and of the CLI `--out` file, with checkpoints
`[(0,0),(2,2),(4,4)]` and final step/tick `4`; the two-daemon restart scenario returns
`session_key=1`, `snapshot_key=1`, `tick=5`, `state_hash=a3b2e28f…b66fdb` from both the saving
and the reloading process, with `session.list` empty after restart as documented. All of these
values are identical to round 1.

**Intentional behavior changes recorded by the repair (pinned by tests).** `max_sessions`
counts all registered sessions including closed ones (closed sessions stay unlisted and
non-mutatable and keep occupying the bound, while `session_count()` reports live sessions
only); the human `session list` prints exactly
`chronofork: storage listing truncated at 256 entries` on stderr when the service reports
`truncated:true` (stdout record shape unchanged, `--json` keeps the envelope field); locally
produced and human-facing failures use the base `chronofork: <message>` prefix, with typed
codes only in canonical `chronofork.error.v1` envelopes. Contract-text corrections: the four
HTTP framing codes are documented as transport-owned and encoded through the single service
envelope builder, and `store.delete` is documented as mutating with no cascade.

**Anti-slop exception record (R10, broadened).** The new service and transport test binaries
follow the repository-wide test convention: unit-test modules use the documented
`#![cfg_attr(test, allow(clippy::expect_used))]` pattern and integration test binaries use the
process-test `expect` header, consistent with
[testing strategy](testing-strategy.md#8-initial-quality-gates) ("`expect_used` is allowed for
test diagnostics but must not appear in production library paths"). `crates/chronofork-cli/
tests/m6.rs` additionally carries the anti-slop policy §10 exception record naming its owner,
the single-binary scope and the expiry condition. No production `allow` was added, the only
pre-existing `#[allow]` remains `megadrive/interrupts.rs`, and `clippy -D warnings` exits 0.

**Recorded precision limits (no repair action required).** `io_timeout` is enforced and
verified at process level (an idle connection closed after 5.24 s with 0 bytes while the
daemon kept serving) but no automated test asserts the timeout expiry; `internal_error` is
covered by the status-table unit test but is never produced by an operation; the HTTP server
is HTTP/1.1 with no TLS, keep-alive, chunked bodies or SSE and `POST /mcp` is stateless
(no `Mcp-Session-Id`, resources/prompts/logging); tokens are coarse and static with no
rotation, expiry or audit log; ownership is process-local and lost on restart, durable
objects have no per-principal owner, and closed sessions are retained for the process
lifetime; debug/trace/script/TAS stay CLI-local; one backend configuration per daemon; no
performance claim is made. These are recorded as M6-FU1–M6-FU4 in the
[milestone](../roadmap/milestones.md#m6--agent-interfaces) rather than reopened.

Raw evidence is in the ignored `.agents/notes/m6/verify/` tree: `PASS2.md`,
`TREE-STATE-PASS2.md`, `rows2/` (every row command), `gate/check-sh-pass2.log`,
`gate/per-target-pass2.txt`, `demos2/`, `parity/*-pass2.log`, `audit/*-pass2*` and
`audit/a27-honesty.txt`, with the preserved round-1 `ROWS.md`, `rows/`, `gate/`, `demos/`,
`parity/` and `audit/`. This block is the M6 gate record. **Backend coverage:** the service,
parity, HTTP, MCP, client and CLI rows run against the **deterministic synthetic backend**;
the minimal Mega Drive backend is exercised only through `serve --rom` composition and its
capability report (M6-A20) — no Mega Drive transport behavior, and no Mega Drive
service/parity claim, is made. Verification ran on Linux/WSL; M6 is merged to `main` at
`cb05836` (PR #6).

## 2026-11-17 — M7 human-facing GUI (independent final verification PASS4)

The independent verifier executed the [M7 acceptance
plan](m7-gui-acceptance.md) on the **uncommitted** worktree
`/home/stealthc/projects/ChronoFork/.agents/worktrees/m7-gui`, branch `agent/m7-gui`, HEAD
and base `origin/main` `93d47ed308874be0e8e7060641c238c7f6809e31`, carrying the verified M6
snapshot plus the M7 delta (28 uncommitted entries: 14 modified, 14 untracked). The tree was
byte-stable across the run: 51 files hashed identically at 13:50:44 and 13:53:21, HEAD and
the status set unchanged. **This block verifies that worktree only: no commit, merge, push or
publication was authorized at verification time; the work was subsequently merged to `main`
at `cb05836` (PR #6).** Environment: `CARGO_HOME=/tmp/t7-cargo`, `CARGO_TERM_COLOR=never`, the worktree's
own `target/` (no `CARGO_TARGET_DIR` override), cargo 1.100.0 / rustc 1.100.0. The
promotion basis is the final **PASS4**; the [M7 acceptance
plan](m7-gui-acceptance.md) §2.4 records the earlier rounds (round-1 **FAIL** with findings
F1–F4, then **FAIL3** with M7-A24 false as written) and the repairs, and the final
independent review `REVIEW5` returned **PASS** with no new finding. The documentation
promotion that followed (2026-11-17) edited only tracked documentation — the promotion
targets listed in the acceptance plan plus three durable vision/scope statements that still
called the GUI undelivered — so the verified 28-entry code/doc set became 31 uncommitted
entries with no code, test, manifest, lockfile or acceptance-evidence change.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Exit 0 |
| Compilation | `cargo check --workspace` | Exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 |
| Tests | `cargo test --workspace` | 76 targets (65 test binaries + 11 doc-test targets), 988 passed, 0 failed, 0 ignored |
| Full script | `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` | Exit 0 |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Exit 0 |
| Demo | `cargo run -p chronofork-cli -- demo` ×3 | Exit 0, 3/3 byte-identical, 986 B, SHA-256 `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652`, empty stderr |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` ×3 | Exit 0, 3/3 byte-identical, 1582 B, SHA-256 `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996`, empty stderr |

**Non-regression accounting.** The M6 baseline was **67 targets / 800 passed / 0 failed / 0
ignored**; every pre-existing target is still present and no per-target count changed, so
the baseline contributes exactly 800. The **nine added targets** are the `chronofork-gui`
lib + six integration test binaries (`command` 21, `control` 8, `model` 10, `pacing` 9,
`presentation` 8, `render` 16), the CLI `tests/gui.rs` (15) and the gui doc-test target; the
crate's 101 library tests plus those 72 integration tests plus the CLI's 15 give **+188**
(800 + 188 = 988). The extra test over the previous M7 run (987) is
`harness_tolerates_a_child_that_stops_before_reading_the_whole_script`, the regression test
for the child-`BrokenPipe` harness race. No baseline target is missing or below baseline,
no test was deleted or ignored, and no new third-party dependency was added: the M7 delta
adds exactly the `chronofork-gui` workspace package to `Cargo.lock` and no registry package.
There is no `unsafe`; production code has no `#[allow]`, and the only added allowances are
the repository's test-only `expect_used` exception (registered in M7-A1 with owner **M7**
and a revisit condition) and the CLI GUI test harness pattern.

**Acceptance rows.** All 30 rows M7-A1–M7-A30 were re-executed with their exact commands and
pass; the non-zero exits inside row logs are the expected failure cases the rows assert
(A17 denial, A18 bad flag, A19's 16 documented failures, A23 truncated movie). The evidence
for each row is `.agents/notes/m7/verify/final2/rows/M7-A<n>.log`, with process-level
stdout/stderr separated under `rows/process/`. Notable executed observations: attach/detach
leaves the canonical tick/hash unchanged and the hidden-panel frames byte-identical (the
only by-design visible difference is the `events` line, A6/A7); the eight-panel matrix, the
pacer/pause gate, the capability-gated video/audio `Unavailable` paths and the lease
counters match the contract (A8–A16); `pulse` prints the documented deterministic cadence
(`frames=3 frame=3 tick=3 elapsed_nanos=60000000`, `frames=1 frame=4
elapsed_nanos=80000000`, A15); local and remote process runs are byte-identical (A20); the
real M6 daemon serves read-only observation and second-process handoff (A21/A22); the
`.cfmovie` is unchanged (A23); and the targeted token-material grep prints `none` while the
reader-token process test passes (A24).

**Claims and documents.** The audit resolved **72 links / 15 anchors with 0 problems** and
reviewed the 10 claim-grep hits: no rendering, audio, native-window, browser, streaming,
performance or compatibility claim is made; `rendering=unavailable`/`audio=unavailable` are
reported honestly and the `Available` pipeline path is exercised only with test doubles; the
controller lease is stated as cooperative, not a lock; `M6-FU1` is named where inspector
content is missing. The promotion (milestone/requirement/index/ADR statuses, this block) is
traceable to the executed rows.

**Recorded limits (no repair action required).** v1 presents text/ANSI and a static HTML
document only: no native toolkit window, no picture and no sound, no browser/JS frontend, no
streaming transport, no live audio output, no GUI input injection, no GUI state persistence
and no VDP/bus/disassembly/trace panel content (M6-FU1). Real video/audio sources and sinks
remain H2/H3. No v1 CLI process builds `SystemClock`: playback is the deterministic `pulse`
and live host-time playback is a follow-up. The controller lease is cooperative and
window-scoped. Verification ran on Linux/WSL only; no performance or cross-platform claim is
made.

Raw evidence is in the ignored `.agents/notes/m7/verify/final2/` tree — `PASS4.md`,
`rows/`, `rows/process/`, `gate/check-sh.log`, `gate/check-sh.exit`,
`gate/check-sh.time.log`, `gate/per-target.log`, `gate/per-target.json`, `demos/` and
`audit/` — with the earlier rounds preserved in the same tree and the review in
`.agents/notes/m7/review/REVIEW5.md`. This block is the M7 gate record. **Backend coverage:**
the GUI, renderer, pacing, presentation, attach/detach and process rows run against the
**deterministic synthetic backend**; the minimal Mega Drive backend appears only through
local `--rom` composition and its capability report (M7-A18), so no Mega Drive rendering,
audio or parity claim is made. M7 is merged to `main` at `cb05836` (PR #6).

## 2026-11-17 — M8 phase 8.1 cartridge, region, TMSS and boot (integration gate)

Phase 8.1 was implemented by an Agent Teams implementer in the worktree
`/home/stealthc/projects/ChronoFork/.agents/worktrees/8.1-cartridge-region-tmss`, branch
`agent/8.1-cartridge-region-tmss`, based on `origin/main` `a5de42a` (phase start commit
`70a4d0c`). A separate reviewer reviewed it independently: round 1 returned
**needs_revision** (five blocking findings F-01–F-05 plus lows: SRAM-spec offsets,
unserialized mapper registers, a new `dead_code` allowance, unbounded file read, a duplicated
header writer, TMSS unlock width and missing restore paths); the implementer repaired them and
round 2 returned **PASS** with every round-1 code finding verified resolved. This block is the
integrator's own re-run after applying the reviewer's advisory cleanups (three unreferenced
public accessors removed, two doc corrections) and the phase documentation; **no behavioural
change was made after the PASS**. The work was committed on the branch, pushed, opened as
PR #25 and merged to `main` at `f024b7b` after every CI job passed (format, check/clippy, test,
rustdoc, deterministic demos, release build/tests, MSRV 1.85.0 and the Windows/macOS build
checks; issue #14 closed by the merge). Environment: `CARGO_HOME=/tmp/t7-cargo`, the
worktree's own `target/`, cargo 1.100.0 / rustc 1.100.0.

| Gate | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Exit 0 |
| Compilation | `cargo check --workspace` | Exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 |
| Tests | `cargo test --workspace` | 81 targets, 1,047 passed, 0 failed, 0 ignored |
| Full script | `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` | Exit 0, `all quality gates passed` |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Exit 0 |
| Demo | `cargo run -p chronofork-cli -- demo` ×2 | Exit 0, byte-identical, 986 B, SHA-256 `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` (identical to baseline) |
| Mega Drive demo | `cargo run -p chronofork-cli -- megadrive demo` ×2 | Exit 0, byte-identical, 1,582 B, SHA-256 `132929190fd8a65576172dab6418f3a4a544f779e2a682d553fef6111a6da426`; the only line different from the pre-change baseline is the documented fork-isolation state hash |
| GUI run | `printf 'create\nattach 1\npulse 3\nframe\nquit\n' \| cargo run -p chronofork-cli -- gui run` ×2 | Exit 0, byte-identical, 2,842 B, SHA-256 `d3abf57abbd251c8eb40144691bd652937e9002bfd46cadb7434bfe5984e5fe6`, empty stderr |

**Non-regression accounting.** The pre-change baseline captured in the same worktree at
`a5de42a` is **76 targets / 988 passed / 0 failed / 0 ignored** (all five gates exit 0, both
demos byte-identical across three runs; collapsed in this page's previous M7 block and recorded
raw in `.research/anti-slop/8.1-baseline/`). The phase adds **five test binaries** (megadrive
`cartridge`, `cartridge_snapshot`, `cartridge_boot`, `bus_cartridge` and CLI `cartridge`) plus
cartridge unit tests, for **+59 tests (988 → 1,047)** and **+5 targets**. The reviewer's
per-target comparison found no target below baseline and no baseline test removed. No test was
deleted or `#[ignore]`d, no new `#[allow]` was added in production code (the only pre-existing
allowance is the unchanged `enum_variant_names` in `interrupts.rs` and the test-only
`expect_used`), no third-party dependency was added (`Cargo.toml`/`Cargo.lock` untouched), and
there is no `unsafe` and no `unwrap()` in production paths.

**Documented deviations from the plan** (recorded in the plan's Execution notes): the header
checksum is the hardware **word sum** over `0x200..` (the plan's byte sum matches neither
fixture; both fixtures match the word sum — Kid Chameleon `0x24F4`, Chakan `0x4BBD`); the SRAM
specification is the `RA` marker at `0x1B0` with start/end at `0x1B4`/`0x1B8`, so both fixtures
declare no SRAM; the mapper register file is write-only so all CPU-visible mapper state is
serialized and hashed; only a 32-bit write with bit 9 set releases TMSS. The independent
review re-checksummed both fixtures itself.

**Scoped claim.** The phase delivers the modelled cartridge device and payload/state/config
v3; it does **not** boot a commercial ROM. The capability report still states
`rendering=false` and `audio=false`, no compatibility is claimed, and no ROM, BIOS or game asset
is committed. Mapper variants beyond the Sega banking, images above the 64 MiB ceiling and SRAM
banking schemes other than the 64 KiB window remain phase 8.6 residual work.

Raw evidence is in the ignored worktree-local trees `.research/anti-slop/8.1-baseline/`
(pre-change) and `.research/anti-slop/8.1-integration/` (post-change): provenance, gate
outputs and exit codes, per-target test-result lines, demo/gui stdout, SHA-256 sums and the
`cmp` repeat results.

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
| Differential execution | Baseline-vs-refactor serialized-state probe implemented ([evidence §8.4](anti-slop-refactor-evidence.md#84-differential-serialized-state-probe-recreated)); the CPU oracle differential (11 synthetic ROMs) and the phase-8.3 VDP and phase-8.4 audio-state differentials are implemented as external harnesses; full emulator/pixel differential remains out of scope |
| Performance benchmarks | Implemented as `bench_step` example; evidence, not a CI gate ([evidence §8.5](anti-slop-refactor-evidence.md#85-benchmark-on-the-post-repair-tree)) |
| Concurrency tests | Implemented: M6 service manager lock and per-session writer lease (typed retryable `conflict`, reads allowed during a mutation) |
| Transport/wire tests | Implemented: M6 bounded HTTP/1.1 and MCP wire contracts, rejection matrices, server bounds, typed client and direct/HTTP/MCP byte parity |
| GUI/presentation tests | Implemented: M7 deterministic fixed-size renderers (plain/ANSI/HTML), panel/layout model, pacing and pause gate, capability-gated video/audio pipelines, attach/detach non-perturbation, handoff lease and the `chronofork gui run` process surface (synthetic backend; Linux/WSL) |
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
